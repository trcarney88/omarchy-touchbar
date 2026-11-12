//! Fail-closed session policy. Blocking logind work belongs on a worker thread.
use serde_json::Value;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub fn permits(
    p: &Value,
    preparing_sleep: bool,
    preparing_shutdown: bool,
    hyprland_unlocked: Option<bool>,
    displays_on: Option<bool>,
    inhibitor_valid: bool,
) -> bool {
    let uid = unsafe { libc::getuid() as u64 };
    !preparing_sleep
        && !preparing_shutdown
        && hyprland_unlocked == Some(true)
        && displays_on == Some(true)
        && inhibitor_valid
        && p.get("Active").and_then(Value::as_bool) == Some(true)
        && p.get("LockedHint").and_then(Value::as_bool) == Some(false)
        && p.get("State").and_then(Value::as_str) == Some("active")
        && p.get("Class").and_then(Value::as_str) == Some("user")
        && p.get("Type").and_then(Value::as_str) == Some("wayland")
        && p.get("User").and_then(Value::as_array).is_some_and(|a| {
            a.len() == 2
                && a[0].as_u64() == Some(uid)
                && a[1].as_str().is_some_and(|s| s.starts_with('/'))
        })
}

pub fn permits_with_manager(
    p: &Value,
    manager: &Value,
    hypr: Option<bool>,
    displays: Option<bool>,
    inhibitor_valid: bool,
) -> bool {
    permits(
        p,
        manager
            .get("PreparingForSleep")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        manager
            .get("PreparingForShutdown")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        hypr,
        displays,
        inhibitor_valid,
    )
}

pub fn decode_hypr_locked(bytes: &[u8]) -> Option<bool> {
    let v: Value = serde_json::from_slice(bytes).ok()?;
    let locked = match &v {
        Value::Bool(value) => *value,
        Value::Object(object) => object.get("locked")?.as_bool()?,
        _ => return None,
    };
    Some(!locked)
}
pub fn decode_monitors(bytes: &[u8]) -> Option<bool> {
    let v: Value = serde_json::from_slice(bytes).ok()?;
    let monitors = v.as_array()?;
    if monitors.is_empty() {
        return None;
    }
    let mut any_enabled = false;
    let mut all_enabled_on = true;
    for m in monitors {
        let enabled = match (m.get("enabled"), m.get("disabled")) {
            (Some(enabled), Some(disabled)) => {
                let enabled = enabled.as_bool()?;
                let disabled = disabled.as_bool()?;
                if enabled == disabled {
                    return None;
                }
                enabled
            }
            (Some(enabled), None) => enabled.as_bool()?,
            (None, Some(disabled)) => !disabled.as_bool()?,
            (None, None) => return None,
        };
        let dpms = m.get("dpmsStatus")?.as_bool()?;
        if enabled {
            any_enabled = true;
            all_enabled_on &= dpms;
        }
    }
    Some(any_enabled && all_enabled_on)
}

#[derive(Clone, Debug)]
pub struct SessionSnapshot {
    pub allowed: bool,
    pub preparing_sleep: bool,
    pub preparing_shutdown: bool,
    pub valid_until: Instant,
    pub requires_panel_off: bool,
}

pub const PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";
const CHECK_MAX_AGE: Duration = Duration::from_secs(1);
pub fn requires_panel_off(preparing_sleep: Option<bool>, preparing_shutdown: Option<bool>) -> bool {
    preparing_sleep != Some(false) || preparing_shutdown != Some(false)
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
pub struct SessionAdapter {
    connection: zbus::blocking::Connection,
    session: zbus::zvariant::OwnedObjectPath,
    pub inhibitor: Option<zbus::zvariant::OwnedFd>,
    locked_job: Option<crate::jobs::Job>,
    monitors_job: Option<crate::jobs::Job>,
    generation: u64,
    started_at: Option<Instant>,
    session_props: Option<Value>,
    manager_props: Option<Value>,
    sleeping: bool,
    off_acknowledged: bool,
    hypr_check: Option<bool>,
    display_check: Option<bool>,
}
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl SessionAdapter {
    pub fn start() -> zbus::Result<Self> {
        use zbus::{
            blocking::{connection::Builder, Proxy},
            zvariant::OwnedObjectPath,
        };
        let connection = Builder::system()?
            .method_timeout(Duration::from_secs(1))
            .build()?;
        let manager = Proxy::new(
            &connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )?;
        let session: OwnedObjectPath = if let Ok(id) = std::env::var("XDG_SESSION_ID") {
            if id.is_empty() {
                return Err(zbus::Error::Failure("empty XDG_SESSION_ID".into()));
            }
            manager.call("GetSession", &(id,))?
        } else {
            manager.call("GetSessionByPID", &(0u32,))?
        };
        Ok(Self {
            connection,
            session,
            inhibitor: None,
            locked_job: None,
            monitors_job: None,
            generation: 0,
            started_at: None,
            session_props: None,
            manager_props: None,
            sleeping: false,
            off_acknowledged: false,
            hypr_check: None,
            display_check: None,
        })
    }
    pub fn refresh(&self) -> Option<(Value, Value)> {
        use zbus::blocking::Proxy;
        let manager = Proxy::new(
            &self.connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            PROPERTIES_INTERFACE,
        )
        .ok()?;
        let m: HashMap<String, zbus::zvariant::OwnedValue> = manager
            .call("GetAll", &("org.freedesktop.login1.Manager",))
            .ok()?;
        let mp = properties_json(&m, &["PreparingForSleep", "PreparingForShutdown"])?;
        let session = Proxy::new(
            &self.connection,
            "org.freedesktop.login1",
            self.session.as_str(),
            PROPERTIES_INTERFACE,
        )
        .ok()?;
        let p: HashMap<String, zbus::zvariant::OwnedValue> = session
            .call("GetAll", &("org.freedesktop.login1.Session",))
            .ok()?;
        Some((
            properties_json(
                &p,
                &["Active", "LockedHint", "State", "Class", "Type", "User"],
            )?,
            mp,
        ))
    }
    pub fn acquire_inhibitor(&mut self) -> zbus::Result<()> {
        use zbus::blocking::Proxy;
        let p = Proxy::new(
            &self.connection,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )?;
        self.inhibitor = Some(p.call(
            "Inhibit",
            &(
                "sleep:shutdown",
                "touchmeter",
                "Disable input before sleep",
                "delay",
            ),
        )?);
        Ok(())
    }

    /// Begins a new worker refresh. The synchronous D-Bus calls are bounded individually;
    /// invoke only from the policy worker, never from a UI callback.
    pub fn begin_refresh(&mut self) -> bool {
        self.allowed_reset();
        self.started_at = Some(Instant::now());
        let Some((session, manager)) = self.refresh() else {
            return false;
        };
        self.session_props = Some(session);
        self.manager_props = Some(manager);
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.hypr_check = None;
        self.display_check = None;
        self.locked_job = crate::jobs::Job::start(
            "hyprctl",
            ["-j", "locked"],
            std::iter::empty::<(&str, &str)>(),
            Duration::from_secs(1),
            64 * 1024,
            generation,
        )
        .ok();
        self.monitors_job = crate::jobs::Job::start(
            "hyprctl",
            ["-j", "monitors"],
            std::iter::empty::<(&str, &str)>(),
            Duration::from_secs(1),
            64 * 1024,
            generation,
        )
        .ok();
        self.locked_job.is_some() && self.monitors_job.is_some()
    }

    fn allowed_reset(&mut self) {
        self.locked_job = None;
        self.monitors_job = None;
        self.session_props = None;
        self.manager_props = None;
        self.started_at = None;
    }

    pub fn poll(&mut self, now: Instant) -> SessionSnapshot {
        let started = self.started_at.unwrap_or(now);
        if let Some(job) = self.locked_job.as_mut() {
            if let Some(r) = job.poll() {
                if r.generation == self.generation && r.error.is_none() && r.status == Some(0) {
                    self.hypr_check = decode_hypr_locked(&r.stdout);
                }
            }
        }
        if let Some(job) = self.monitors_job.as_mut() {
            if let Some(r) = job.poll() {
                if r.generation == self.generation && r.error.is_none() && r.status == Some(0) {
                    self.display_check = decode_monitors(&r.stdout);
                }
            }
        }
        let pending = self.hypr_check.is_none() || self.display_check.is_none();
        let (sleep, shutdown) = self
            .manager_props
            .as_ref()
            .map(|m| {
                (
                    m.get("PreparingForSleep").and_then(Value::as_bool),
                    m.get("PreparingForShutdown").and_then(Value::as_bool),
                )
            })
            .unwrap_or((None, None));
        let preparing = sleep == Some(true) || shutdown == Some(true);
        if preparing && !self.sleeping {
            self.sleeping = true;
            self.off_acknowledged = false;
        }
        if sleep == Some(false) && shutdown == Some(false) {
            self.sleeping = false;
        }
        let flags_ok = sleep.is_some() && shutdown.is_some();
        let allowed = !pending
            && now.saturating_duration_since(started) <= CHECK_MAX_AGE
            && flags_ok
            && !self.sleeping
            && self
                .session_props
                .as_ref()
                .zip(self.manager_props.as_ref())
                .is_some_and(|(s, m)| {
                    permits_with_manager(
                        s,
                        m,
                        self.hypr_check,
                        self.display_check,
                        self.inhibitor.is_some(),
                    )
                });
        SessionSnapshot {
            allowed,
            preparing_sleep: sleep.unwrap_or(true),
            preparing_shutdown: shutdown.unwrap_or(true),
            valid_until: started + CHECK_MAX_AGE,
            requires_panel_off: requires_panel_off(sleep, shutdown) && !self.off_acknowledged,
        }
    }

    pub fn acknowledge_panel_off(&mut self) -> bool {
        if !self.sleeping || self.off_acknowledged {
            return false;
        }
        self.off_acknowledged = true;
        self.inhibitor.take();
        true
    }
}
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
fn properties_json(
    m: &HashMap<String, zbus::zvariant::OwnedValue>,
    required: &[&str],
) -> Option<Value> {
    let mut out = serde_json::Map::new();
    for key in required {
        let v = m.get(*key)?;
        let val = match *key {
            "Active" | "LockedHint" | "PreparingForSleep" | "PreparingForShutdown" => {
                Value::Bool(bool::try_from(v.clone()).ok()?)
            }
            "State" | "Class" | "Type" => Value::String(String::try_from(v.clone()).ok()?),
            "User" => {
                let (uid, path): (u32, zbus::zvariant::OwnedObjectPath) =
                    <(u32, zbus::zvariant::OwnedObjectPath)>::try_from(v.clone()).ok()?;
                serde_json::json!([uid, path.as_str()])
            }
            _ => return None,
        };
        out.insert((*key).into(), val);
    }
    Some(Value::Object(out))
}

#[derive(Clone, Debug)]
pub struct Check<T> {
    pub value: Option<T>,
    pub checked_at: Instant,
}
impl<T> Check<T> {
    pub fn fresh(value: T, now: Instant) -> Self {
        Self {
            value: Some(value),
            checked_at: now,
        }
    }
    pub fn get(&self, now: Instant, max_age: Duration) -> Option<&T> {
        if now.saturating_duration_since(self.checked_at) <= max_age {
            self.value.as_ref()
        } else {
            None
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SleepAction {
    ClearPermission,
    ReleaseKeys,
    StopAudio,
    PanelOff,
    AcknowledgeSleep,
    CloseInhibitor,
    ReacquireInhibitor,
    RefreshChecks,
}
#[derive(Default)]
pub struct SleepCoordinator {
    preparing: bool,
    off_acknowledged: bool,
}
impl SleepCoordinator {
    pub fn prepare(&mut self, sleeping: bool) -> Vec<SleepAction> {
        if sleeping {
            if self.preparing {
                return vec![];
            }
            self.preparing = true;
            self.off_acknowledged = false;
            vec![
                SleepAction::ClearPermission,
                SleepAction::ReleaseKeys,
                SleepAction::StopAudio,
                SleepAction::PanelOff,
            ]
        } else {
            self.preparing = false;
            self.off_acknowledged = false;
            vec![SleepAction::ReacquireInhibitor, SleepAction::RefreshChecks]
        }
    }
    pub fn acknowledge_panel_off(&mut self) -> Vec<SleepAction> {
        if !self.preparing || self.off_acknowledged {
            return vec![];
        }
        self.off_acknowledged = true;
        vec![SleepAction::AcknowledgeSleep, SleepAction::CloseInhibitor]
    }
}
