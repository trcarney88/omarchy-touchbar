use std::time::Instant;

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::Duration,
};

/// Exclusive per-user runtime ownership, removed only if the pathname still
/// names the inode created by this process.
#[derive(Debug)]
pub struct RuntimeLock {
    path: PathBuf,
    dev: u64,
    ino: u64,
}

impl RuntimeLock {
    pub fn acquire() -> Result<Self, String> {
        let dir = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is unset")?;
        Self::acquire_in(Path::new(&dir), std::process::id(), hostname())
    }

    pub fn acquire_in(dir: &Path, pid: u32, host: String) -> Result<Self, String> {
        let meta = fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
        if !meta.is_dir()
            || meta.file_type().is_symlink()
            || meta.uid() != unsafe { libc::getuid() }
            || meta.mode() & 0o022 != 0
        {
            return Err("Unsafe XDG_RUNTIME_DIR".into());
        }
        let path = dir.join("touchmeter.lock");
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                inspect_existing(&path)?;
                return Err("AlreadyRunning".into());
            }
            Err(e) => return Err(e.to_string()),
        };
        let meta = file.metadata().map_err(|e| e.to_string())?;
        let lock = Self {
            path: path.clone(),
            dev: meta.dev(),
            ino: meta.ino(),
        };
        let result = writeln!(file, "{pid}\ntouchmeter\n{host}").and_then(|_| file.sync_all());
        if let Err(e) = result {
            drop(file);
            drop(lock);
            return Err(e.to_string());
        }
        Ok(lock)
    }
}

impl Drop for RuntimeLock {
    fn drop(&mut self) {
        if let Ok(m) = fs::symlink_metadata(&self.path) {
            if m.dev() == self.dev && m.ino() == self.ino && !m.file_type().is_symlink() {
                let _ = fs::remove_file(&self.path);
            }
        }
    }
}

fn inspect_existing(path: &Path) -> Result<(), String> {
    let md = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if md.file_type().is_symlink()
        || !md.is_file()
        || md.uid() != unsafe { libc::getuid() }
        || md.mode() & 0o022 != 0
    {
        return Err("Unsafe existing touchmeter lock".into());
    }
    let mut text = String::new();
    File::open(path)
        .and_then(|mut f| f.take(4097).read_to_string(&mut text))
        .map_err(|e| e.to_string())?;
    let lines: Vec<_> = text.lines().collect();
    // Accept both the legacy three-line lock and the five-line Qt lock
    // (pid/app/host/machine/boot), while bounding all metadata.
    if !matches!(lines.len(), 3 | 5)
        || lines[1] != "touchmeter"
        || lines[2].is_empty()
        || (lines.len() == 5 && (lines[3].is_empty() || lines[4].is_empty()))
        || text.len() > 4096
    {
        return Err("Malformed touchmeter lock".into());
    }
    let pid: u32 = lines[0].parse().map_err(|_| "Malformed touchmeter lock")?;
    if pid == 0 {
        return Err("Malformed touchmeter lock".into());
    }
    match unsafe { libc::kill(pid as i32, 0) } {
        0 => Err("AlreadyRunning".into()),
        _ if std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM) => {
            Err("AlreadyRunning".into())
        }
        _ => Err("Stale lock is not automatically removed".into()),
    }
}

fn hostname() -> String {
    let mut b = [0u8; 256];
    if unsafe { libc::gethostname(b.as_mut_ptr().cast(), b.len()) } == 0 {
        String::from_utf8_lossy(&b[..b.iter().position(|x| *x == 0).unwrap_or(b.len())])
            .into_owned()
    } else {
        "unknown".into()
    }
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
fn make_physical_data() -> Result<super::data::PhysicalData, String> {
    use std::{collections::HashMap, path::PathBuf};
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is unset")?;
    let state_home = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/state"));
    let fixture = crate::testcli::runtime_path(false)?;
    let credentials = crate::credentials::CredentialContext::new(home, std::env::vars().collect());
    Ok(super::data::PhysicalData::new(
        super::data::DataPaths {
            state_home,
            config: crate::config::path(),
            fixture,
            collector: PathBuf::from("omarchy"),
        },
        HashMap::new(),
    )
    .with_credential_context(credentials)
    .with_appearance(crate::appearance::AppearanceOptions::default()))
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
pub struct HardwareRuntime {
    input: Option<super::input::DeviceAdapter>,
    display: Option<super::display::DrmDisplay>,
    keyboard: Option<super::keyboard::Adapter>,
    lease: Option<super::lease::Adapter>,
    lock: Option<RuntimeLock>,
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl HardwareRuntime {
    pub fn start(timeout: Duration) -> Result<Self, String> {
        Self::start_with(RuntimeLock::acquire, |lock| {
            Self::start_adapters(lock, timeout)
        })
    }

    fn start_with<L, F>(lock: L, adapters: F) -> Result<Self, String>
    where
        L: FnOnce() -> Result<RuntimeLock, String>,
        F: FnOnce(RuntimeLock) -> Result<Self, String>,
    {
        adapters(lock()?)
    }

    fn start_adapters(lock: RuntimeLock, timeout: Duration) -> Result<Self, String> {
        let mut runtime = Self {
            input: None,
            display: None,
            keyboard: None,
            lease: None,
            lock: Some(lock),
        };
        let result = (|| {
            runtime.lease = Some(super::lease::Adapter::start(timeout)?);
            runtime.keyboard = Some(super::keyboard::Adapter::start(timeout)?);
            let until = std::time::Instant::now() + timeout;
            while !runtime.lease.as_ref().unwrap().is_acquired()
                || !runtime.keyboard.as_ref().unwrap().ready()
            {
                if std::time::Instant::now() >= until {
                    return Err("Timed out waiting for lease and keyboard".into());
                }
                let slice = Duration::from_millis(20)
                    .min(until.saturating_duration_since(std::time::Instant::now()));
                runtime.lease.as_mut().unwrap().pump(slice)?;
                runtime.keyboard.as_mut().unwrap().pump(slice)?;
            }
            let fd = runtime
                .lease
                .as_ref()
                .unwrap()
                .leased_fd()
                .ok_or("DRM lease disappeared")?
                .try_clone_to_owned()
                .map_err(|e| e.to_string())?;
            runtime.display =
                Some(super::display::DrmDisplay::from_lease_fd(fd).map_err(|e| e.to_string())?);
            runtime
                .display
                .as_mut()
                .unwrap()
                .set_power(false)
                .map_err(|e| e.to_string())?;
            runtime.input = Some(super::input::DeviceAdapter::start().map_err(|e| e.to_string())?);
            Ok(())
        })();
        if let Err(e) = result {
            runtime.shutdown();
            return Err(e);
        }
        Ok(runtime)
    }

    pub fn shutdown(&mut self) {
        if let Some(k) = self.keyboard.as_mut() {
            let _ = k.release_all();
        }
        if let Some(d) = self.display.as_mut() {
            let _ = d.set_power(false);
        }
        self.input.take();
        self.display.take();
        self.keyboard.take();
        self.lease.take();
        self.lock.take();
    }
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl Drop for HardwareRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Hardware-backed actuator for one bounded quiescence transaction.
/// The runtime and its other owners remain borrowed for the entire barrier.
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
pub struct RuntimeHardwareActuator<'a> {
    hardware: &'a mut HardwareRuntime,
    controller: &'a mut crate::controls::Controller,
    model: &'a mut touchmeter_ui::Model,
    volume: &'a mut super::volume::VolumeAdapter,
    core: &'a mut RuntimeCore,
    recovery: bool,
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl<'a> RuntimeHardwareActuator<'a> {
    pub fn new(
        hardware: &'a mut HardwareRuntime,
        controller: &'a mut crate::controls::Controller,
        model: &'a mut touchmeter_ui::Model,
        volume: &'a mut super::volume::VolumeAdapter,
        core: &'a mut RuntimeCore,
        recovery: bool,
    ) -> Self {
        Self {
            hardware,
            controller,
            model,
            volume,
            core,
            recovery,
        }
    }
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl super::quiescence::Actuator for RuntimeHardwareActuator<'_> {
    fn revoke_authority(&mut self) {
        self.core.revoke();
        let _ = self.controller.set_sleeping(true, self.model);
        self.model.pressed = 0;
    }

    fn cancel_effects_nonblocking(&mut self) {
        self.volume.deactivate_now();
    }

    fn release_keys_and_sync(&mut self, timeout: Duration) -> Result<(), String> {
        let keyboard = self
            .hardware
            .keyboard
            .as_mut()
            .ok_or("Keyboard unavailable after startup")?;
        let result = if self.recovery {
            keyboard.recover_release_and_sync(timeout)
        } else {
            keyboard.release_all_and_sync(timeout)
        };
        self.recovery = true;
        result
    }

    fn fresh_panel_off(&mut self) -> Result<(), String> {
        self.hardware
            .display
            .as_mut()
            .ok_or("Display unavailable after startup")?
            .fresh_power_off()
            .map_err(|e| e.to_string())
    }

    fn blackout_existing_framebuffer(&mut self) -> Result<(), String> {
        self.hardware
            .display
            .as_mut()
            .ok_or("Display unavailable after startup")?
            .blackout()
            .map_err(|e| e.to_string())
    }
}

pub fn ensure_fresh_or_quiesce<F>(
    core: &mut RuntimeCore,
    now: Instant,
    proof_current: bool,
    quiesce: F,
) -> Result<bool, String>
where
    F: FnOnce(&mut RuntimeCore) -> Result<(), String>,
{
    if core.check_fresh(now) {
        return Ok(true);
    }
    if !proof_current {
        quiesce(core)?;
    }
    Ok(false)
}

/// Advance the OLED text-label offset every fifteen minutes.
pub fn oled_shift(unix_seconds: i64) -> i32 {
    (unix_seconds.div_euclid(15 * 60) % 5) as i32
}

/// Keep the animated AI clock tied to monotonic runtime time on every render
/// path, including dirty frames that are paced independently of animation.
pub fn update_model_clock(model: &mut touchmeter_ui::Model, origin: Instant, now: Instant) {
    if model.fire && model.app == "ai" && !model.picker {
        model.time = now.saturating_duration_since(origin).as_secs_f32();
    }
}

#[derive(Default, Debug, PartialEq, Eq)]
struct FrameDiagnostics {
    render_started: u64,
    render_completed: u64,
    present_started: u64,
    present_completed: u64,
    auth_rejected: u64,
}

impl FrameDiagnostics {
    fn sample_due(&self, now: Instant, last: Instant) -> bool {
        now.duration_since(last) >= Duration::from_secs(2)
    }
}

#[cfg(test)]
mod frame_diagnostics_tests {
    use super::FrameDiagnostics;
    use std::time::{Duration, Instant};

    #[test]
    fn samples_every_two_seconds_and_counters_stage_at_operation_boundaries() {
        let start = Instant::now();
        let mut d = FrameDiagnostics::default();
        assert!(!d.sample_due(start + Duration::from_millis(1999), start));
        assert!(d.sample_due(start + Duration::from_secs(2), start));
        d.render_started += 1;
        assert_eq!((d.render_started, d.render_completed), (1, 0));
        d.render_completed += 1;
        d.present_started += 1;
        assert_eq!((d.present_started, d.present_completed), (1, 0));
        d.present_completed += 1;
        assert_eq!((d.render_started, d.render_completed), (1, 1));
        assert_eq!((d.present_started, d.present_completed), (1, 1));
    }
}

#[cfg(test)]
mod oled_shift_tests {
    use super::oled_shift;

    #[test]
    fn shifts_at_quarter_hour_boundaries() {
        assert_eq!(oled_shift(0), 0);
        assert_eq!(oled_shift(59), 0);
        assert_eq!(oled_shift(60), 0);
        assert_eq!(oled_shift(14 * 60), 0);
        assert_eq!(oled_shift(15 * 60), 1);
        assert_eq!(oled_shift(-1), -1);
    }
}

pub fn route_fixture_audio_effect<F, L>(fixture_active: bool, fixture: F, live: L)
where
    F: FnOnce(),
    L: FnOnce(),
{
    if fixture_active { fixture() } else { live() }
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
fn quiesce_permission_loss(
    hardware: &mut HardwareRuntime,
    controller: &mut crate::controls::Controller,
    model: &mut touchmeter_ui::Model,
    volume: &mut super::volume::VolumeAdapter,
    core: &mut RuntimeCore,
    transaction: u64,
) -> Result<super::quiescence::QuiescenceToken, String> {
    let mut actuator =
        RuntimeHardwareActuator::new(hardware, controller, model, volume, core, false);
    super::quiescence::attempt(&mut actuator, transaction.max(1))
        .map_err(|e| format!("quiescence failed: {e:?}"))
}

/// Bounded bridge between the shutdown transaction and the session worker.
/// It deliberately retains the worker handle when an acknowledgement or join
/// times out; dropping the bridge never drops either owner.
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
pub struct RuntimeShutdownWorker<'a> {
    worker: &'a mut super::session_worker::SessionWorker,
    ready_transaction: Option<u64>,
    stopped_transaction: Option<u64>,
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl<'a> RuntimeShutdownWorker<'a> {
    pub fn new(worker: &'a mut super::session_worker::SessionWorker) -> Self {
        Self {
            worker,
            ready_transaction: None,
            stopped_transaction: None,
        }
    }

    fn wait_for(&mut self, transaction: u64, ready: bool, timeout: Duration) -> Result<(), String> {
        use super::session_worker::SessionEvent;
        let deadline = Instant::now() + timeout;
        loop {
            if (ready && self.ready_transaction == Some(transaction))
                || (!ready && self.stopped_transaction == Some(transaction))
            {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(if ready {
                    "Timed out waiting for worker park"
                } else {
                    "Timed out waiting for worker stop"
                }
                .into());
            }
            match self.worker.recv_timeout(remaining) {
                Ok(SessionEvent::QuiescenceReady { transaction: txn })
                    if ready && txn == transaction =>
                {
                    self.ready_transaction = Some(txn);
                }
                Ok(SessionEvent::QuiescenceStopped { transaction: txn })
                    if !ready && txn == transaction =>
                {
                    self.stopped_transaction = Some(txn);
                }
                Ok(SessionEvent::Failed { generation }) => {
                    return Err(format!("Session worker failed (generation {generation})"));
                }
                Ok(_) => {} // Discard unrelated session publications during shutdown.
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    return Err(if ready {
                        "Timed out waiting for worker park"
                    } else {
                        "Timed out waiting for worker stop"
                    }
                    .into());
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err("Session worker event channel disconnected".into());
                }
            }
        }
    }
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl super::shutdown::Worker for RuntimeShutdownWorker<'_> {
    fn request_park(&mut self, transaction: u64) -> Result<(), String> {
        if self.ready_transaction != Some(transaction) {
            self.worker.begin_quiescence(transaction);
        }
        Ok(())
    }

    fn wait_ready(&mut self, transaction: u64, timeout: Duration) -> Result<(), String> {
        self.wait_for(transaction, true, timeout)
    }

    fn complete_and_join(
        &mut self,
        token: &super::quiescence::QuiescenceToken,
        timeout: Duration,
    ) -> Result<(), String> {
        let transaction = token.transaction();
        let deadline = Instant::now() + timeout;
        if self.stopped_transaction != Some(transaction) {
            self.worker.complete_quiescence(token);
            self.wait_for(
                transaction,
                false,
                deadline.saturating_duration_since(Instant::now()),
            )?;
        }
        if self
            .worker
            .stop_and_join(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(())
        } else {
            Err("Session worker join timed out".into())
        }
    }

    fn force_stop_and_join(&mut self, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        self.worker.force_stop();
        if self
            .worker
            .stop_and_join(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(())
        } else {
            Err("Forced session worker join timed out".into())
        }
    }
}

/// Run shutdown while retaining all caller-owned resources on a failed
/// quiescence attempt. `next_event` is the blocking operator/event source used
/// while the transaction is held; no event other than Retry or Force advances
/// the shutdown state.
pub fn drive_retained_shutdown<A, W, N>(
    transaction: u64,
    actuator: &mut A,
    worker: &mut W,
    mut next_event: N,
) -> super::shutdown::Outcome
where
    A: super::quiescence::Actuator,
    W: super::shutdown::Worker,
    N: FnMut() -> super::shutdown::Event,
{
    drive_retained_shutdown_with_fault(transaction, actuator, worker, None, next_event)
}

pub fn drive_retained_shutdown_with_fault<A, W, N>(
    transaction: u64,
    actuator: &mut A,
    worker: &mut W,
    initial_fault: Option<String>,
    mut next_event: N,
) -> super::shutdown::Outcome
where
    A: super::quiescence::Actuator,
    W: super::shutdown::Worker,
    N: FnMut() -> super::shutdown::Event,
{
    use super::shutdown::{Event, Outcome, ShutdownTransaction, State};
    let mut shutdown = if let Some(report) = initial_fault {
        ShutdownTransaction::fault_held(transaction, report)
    } else {
        ShutdownTransaction::new(transaction)
    };
    let mut outcome = if shutdown.state() == State::FaultHeld {
        let park = worker.request_park(transaction);
        if let Err(error) = park {
            shutdown.last_failure = Some(format!(
                "{}; worker park request failed: {error}",
                shutdown
                    .last_failure
                    .as_deref()
                    .unwrap_or("quiescence failed")
            ));
        }
        Outcome::Held
    } else {
        shutdown.handle(Event::OrdinaryStop, actuator, worker)
    };
    loop {
        match outcome {
            Outcome::SafeStopped | Outcome::ForcedReleased => return outcome,
            Outcome::Held | Outcome::Noop(State::FaultHeld) => {
                eprintln!(
                    "FAULT_HELD: {}",
                    shutdown
                        .last_failure
                        .as_deref()
                        .unwrap_or("shutdown proof unavailable")
                );
                outcome = match next_event() {
                    Event::Retry => shutdown.handle(Event::Retry, actuator, worker),
                    Event::Force => shutdown.handle(Event::Force, actuator, worker),
                    _ => Outcome::Noop(State::FaultHeld),
                };
            }
            Outcome::Noop(_) => return outcome,
        }
    }
}

/// Enter the physical interaction loop after the caller has constructed a
/// non-simulated base model. Session authorization is obtained asynchronously;
/// input is ignored until a fresh allow snapshot arrives.
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
pub fn run(mut model: touchmeter_ui::Model) -> Result<(), String> {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::Instant,
    };
    let mut recovery_signals = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGUSR1,
        signal_hook::consts::SIGUSR2,
    ])
    .map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    signal_hook::flag::register(signal_hook::consts::SIGTERM, flag.clone())
        .map_err(|e| e.to_string())?;
    signal_hook::flag::register(signal_hook::consts::SIGINT, flag).map_err(|e| e.to_string())?;
    let mut hardware = HardwareRuntime::start(Duration::from_secs(5))?;
    eprintln!(
        "Touchmeter {}: DSI-1 60x2008 leased, initial panel off",
        env!("CARGO_PKG_VERSION")
    );
    let mut data = make_physical_data()?;
    model.providers.iter_mut().for_each(|p| p.enabled = false);
    model.testing = false;
    model.fire = false;
    model.volume_available = false;
    model.status = "Checking session authorization".into();
    let startup_now = Instant::now();
    data.set_authorized(false);
    data.tick(startup_now, chrono::Utc::now(), &mut model);
    let mut ui = touchmeter_ui::Ui::new(model.clone())?;
    let mut worker = super::session_worker::SessionWorker::start()?;
    let mut controller = crate::controls::Controller::default();
    let mut volume = super::volume::VolumeAdapter::new("wpctl");
    let mut core = RuntimeCore::default();
    let mut generation = 0;
    let origin = Instant::now();
    let mut last_activity = Instant::now();
    let mut last_frame = Instant::now();
    let mut on = false;
    let mut dirty = true;
    let mut last_presented_app = model.app.clone();
    let mut idle = false;
    let mut wake_grace_until = Instant::now();
    let mut failure = None;
    let mut permission_quiesced = false;
    let mut held_fault = None;
    let mut last_logged_authorized = false;
    let mut first_frame_logged = false;
    let mut diagnostic_at = Instant::now();
    let mut iterations = 0u64;
    let mut drained_touches = 0u64;
    let mut rejected_begins = 0u64;
    let mut rendered = 0u64;
    let mut presented = 0u64;
    let mut diag = FrameDiagnostics::default();
    let mut render_total = Duration::ZERO;
    let mut present_total = Duration::ZERO;
    let mut sampled_render_marker = false;
    while !stop.load(Ordering::Relaxed) {
        iterations += 1;
        // Pace every pass, including early continues on stale authorization.
        thread::sleep(Duration::from_millis(8));
        let now = Instant::now();
        if model.fire && diag.sample_due(now, diagnostic_at) {
            eprintln!(
                "Touchmeter runtime diag: iterations={iterations} render={}/{} render_ms={} present={}/{} present_ms={} drained_touches={drained_touches} rejected_begin_app_mismatch={rejected_begins} rejected_begin_auth={} dirty={dirty} on={on} idle={idle} authorized={} fire={} picker={} app={} last_presented_app={} time={:.3}",
                diag.render_completed,
                diag.render_started,
                render_total.as_millis(),
                diag.present_completed,
                diag.present_started,
                present_total.as_millis(),
                diag.auth_rejected,
                core.allowed(now),
                model.fire,
                model.picker,
                model.app,
                last_presented_app,
                model.time
            );
            diagnostic_at = now;
            sampled_render_marker = true;
        }
        let shift = oled_shift(chrono::Utc::now().timestamp());
        if model.shift != shift {
            model.shift = shift;
            dirty = true;
        }
        if hardware
            .lease
            .as_mut()
            .unwrap()
            .pump(Duration::ZERO)
            .is_err()
            || hardware
                .keyboard
                .as_mut()
                .unwrap()
                .pump(Duration::ZERO)
                .is_err()
        {
            failure = Some("Wayland adapter failed".to_string());
            break;
        }
        for event in worker.drain() {
            match event {
                super::session_worker::SessionEvent::RefreshStarted { generation: g } => {
                    generation = g;
                    core.refresh_started(Instant::now());
                }
                super::session_worker::SessionEvent::Snapshot {
                    generation: g,
                    snapshot,
                } if g == generation => {
                    if snapshot.requires_panel_off {
                        let proof = if permission_quiesced {
                            Ok(())
                        } else {
                            quiesce_permission_loss(
                                &mut hardware,
                                &mut controller,
                                &mut model,
                                &mut volume,
                                &mut core,
                                g.wrapping_add(1).max(1),
                            )
                            .map(|_| ())
                        };
                        match acknowledge_after_quiescence(proof, || {
                            worker.acknowledge_panel_off(g)
                        }) {
                            Ok(()) => {
                                permission_quiesced = true;
                                on = false;
                            }
                            Err(e) => {
                                failure = Some(e.clone());
                                held_fault = Some(e);
                                break;
                            }
                        }
                    }
                    if snapshot.allowed && Instant::now() < snapshot.valid_until {
                        core.authorize_until(snapshot.valid_until);
                        permission_quiesced = false;
                        if !idle && !on {
                            dirty = true;
                        }
                    } else {
                        core.revoke();
                    }
                }
                super::session_worker::SessionEvent::Failed { .. } => {
                    core.revoke();
                }
                _ => {}
            }
        }
        if failure.is_some() {
            break;
        }
        let authorized = core.allowed(Instant::now())
            && hardware.lease.as_ref().unwrap().status().is_ok()
            && hardware.keyboard.as_ref().unwrap().ready();
        data.set_authorized(authorized);
        if authorized != last_logged_authorized {
            eprintln!("Touchmeter session authorization: {authorized}");
            last_logged_authorized = authorized;
        }
        if !authorized {
            core.revoke();
            let _ = controller.set_sleeping(true, &mut model);
            dirty = true;
            if !permission_quiesced {
                match quiesce_permission_loss(
                    &mut hardware,
                    &mut controller,
                    &mut model,
                    &mut volume,
                    &mut core,
                    generation.wrapping_add(1).max(1),
                ) {
                    Ok(_) => {
                        permission_quiesced = true;
                        on = false;
                    }
                    Err(e) => {
                        failure = Some(e.clone());
                        held_fault = Some(e);
                        break;
                    }
                }
            }
        }
        let was_fixture = data.fixture_active();
        dirty |= data.tick(Instant::now(), chrono::Utc::now(), &mut model);
        let fixture_active = data.fixture_active();
        if fixture_active && !was_fixture {
            volume.deactivate_now();
        }
        if let Some(effects) = controller.cancel_if_view_changed(&mut model) {
            for effect in effects {
                if let crate::controls::Effect::FunctionKey {
                    number,
                    pressed: false,
                } = effect
                {
                    if let Err(e) = hardware
                        .keyboard
                        .as_mut()
                        .unwrap()
                        .function_key(number as u8, false)
                    {
                        failure = Some(e);
                        break;
                    }
                }
            }
            if failure.is_some() {
                break;
            }
        }
        if let Some(idle_changed) = hardware.keyboard.as_mut().unwrap().take_idle_changed() {
            if idle_changed {
                idle = true;
                let _ = controller.set_sleeping(true, &mut model);
                let _ = hardware.keyboard.as_mut().unwrap().release_all();
                volume.deactivate_now();
                dirty = true;
            } else {
                idle = false;
                wake_grace_until = now + Duration::from_secs(120);
                dirty = true;
            }
        }
        let touches = match hardware.input.as_mut().unwrap().drain() {
            Ok(v) => v,
            Err(e) => {
                failure = Some(e.to_string());
                break;
            }
        };
        drained_touches += touches.len() as u64;
        for touch in touches {
            let event = match touch {
                super::input::TouchEvent::Begin { x, y } => {
                    crate::controls::InputEvent::Begin(x, y)
                }
                super::input::TouchEvent::Move { x, y } => crate::controls::InputEvent::Move(x, y),
                super::input::TouchEvent::End => crate::controls::InputEvent::End,
                super::input::TouchEvent::Cancel => crate::controls::InputEvent::Cancel,
            };
            if matches!(event, crate::controls::InputEvent::Begin(_, _))
                && model.app != last_presented_app
            {
                rejected_begins += 1;
                continue;
            }
            if !authorized && !matches!(event, crate::controls::InputEvent::Cancel) {
                if matches!(event, crate::controls::InputEvent::Begin(_, _)) {
                    rejected_begins += 1;
                    diag.auth_rejected += 1;
                }
                continue;
            }
            let before = (
                model.app.clone(),
                model.picker,
                model.pressed,
                model.selected,
                model.volume,
                model.volume_muted,
            );
            for effect in controller.handle(event, &mut model) {
                let fresh = ensure_fresh_or_quiesce(
                    &mut core,
                    Instant::now(),
                    permission_quiesced,
                    |core| {
                        quiesce_permission_loss(
                            &mut hardware,
                            &mut controller,
                            &mut model,
                            &mut volume,
                            core,
                            generation.wrapping_add(1).max(1),
                        )
                        .map(|_| ())
                    },
                );
                match fresh {
                    Ok(true) => {}
                    Ok(false) => {
                        permission_quiesced = true;
                        on = false;
                        let _ = controller.set_sleeping(true, &mut model);
                        break;
                    }
                    Err(e) => {
                        failure = Some(e.clone());
                        held_fault = Some(e);
                        break;
                    }
                }
                match effect {
                    crate::controls::Effect::FunctionKey { number, pressed }
                        if authorized
                            && core.check_fresh(Instant::now())
                            && (1..=12).contains(&number) =>
                    {
                        if let Err(e) = hardware
                            .keyboard
                            .as_mut()
                            .unwrap()
                            .function_key(number as u8, pressed)
                        {
                            failure = Some(e);
                            break;
                        }
                    }
                    crate::controls::Effect::PageChanged => {
                        volume.set_active(
                            authorized && !data.fixture_active() && model.app == "volume",
                        );
                    }
                    crate::controls::Effect::SelectProvider(index) => {
                        if let Some(provider_id) = model.providers.get(index).map(|p| p.id.clone())
                        {
                            data.select(&provider_id);
                            let was_fixture = data.fixture_active();
                            dirty |= data.tick(Instant::now(), chrono::Utc::now(), &mut model);
                            if data.fixture_active() && !was_fixture {
                                volume.deactivate_now();
                            }
                            if let Some(effects) = controller.cancel_if_view_changed(&mut model) {
                                for effect in effects {
                                    if let crate::controls::Effect::FunctionKey {
                                        number,
                                        pressed: false,
                                    } = effect
                                    {
                                        if let Err(e) = hardware
                                            .keyboard
                                            .as_mut()
                                            .unwrap()
                                            .function_key(number as u8, false)
                                        {
                                            failure = Some(e);
                                            break;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    crate::controls::Effect::SetVolume(v)
                        if authorized
                            && core.check_fresh(Instant::now())
                            && model.app == "volume" =>
                    {
                        route_fixture_audio_effect(
                            data.fixture_active(),
                            || {
                                data.set_fixture_volume(v as f32);
                                if let Some((fraction, muted)) = data.fixture_volume() {
                                    model.volume = fraction;
                                    model.volume_muted = muted;
                                }
                            },
                            || volume.set_volume(v),
                        );
                    }
                    crate::controls::Effect::ToggleMute
                        if authorized
                            && core.check_fresh(Instant::now())
                            && model.app == "volume" =>
                    {
                        route_fixture_audio_effect(
                            data.fixture_active(),
                            || {
                                data.toggle_fixture_mute();
                                if let Some((fraction, muted)) = data.fixture_volume() {
                                    model.volume = fraction;
                                    model.volume_muted = muted;
                                }
                            },
                            || volume.toggle_mute(),
                        );
                    }
                    crate::controls::Effect::Activity => {
                        last_activity = now;
                        if idle {
                            idle = false;
                            wake_grace_until = now + Duration::from_secs(120);
                        }
                    }
                    _ => {}
                }
            }
            if failure.is_some() {
                break;
            }
            if before
                != (
                    model.app.clone(),
                    model.picker,
                    model.pressed,
                    model.selected,
                    model.volume,
                    model.volume_muted,
                )
            {
                dirty = true;
            }
        }
        match ensure_fresh_or_quiesce(&mut core, Instant::now(), permission_quiesced, |core| {
            quiesce_permission_loss(
                &mut hardware,
                &mut controller,
                &mut model,
                &mut volume,
                core,
                generation.wrapping_add(1).max(1),
            )
            .map(|_| ())
        }) {
            Ok(true) => {}
            Ok(false) => {
                permission_quiesced = true;
                on = false;
                volume.deactivate_now();
                let _ = controller.set_sleeping(true, &mut model);
                continue;
            }
            Err(e) => {
                failure = Some(e.clone());
                held_fault = Some(e);
                break;
            }
        }
        let volume_active = authorized
            && core.allowed(Instant::now())
            && !idle
            && model.app == "volume"
            && !data.fixture_active();
        if !data.fixture_active() {
            volume.set_active(volume_active);
        }
        if volume_active && core.allowed(Instant::now()) {
            volume.tick(origin.elapsed());
        }
        let vs = volume.state;
        if !data.fixture_active()
            && (model.volume_available != (volume_active && vs.available)
                || model.volume != vs.fraction as f32
                || model.volume_muted != vs.muted)
        {
            dirty = true;
        }
        if !data.fixture_active() {
            model.volume_available = volume_active && vs.available;
        }
        if !data.fixture_active() && volume_active && vs.available {
            model.volume = vs.fraction as f32;
            model.volume_muted = vs.muted;
        }
        if !authorized || idle {
            if on {
                if let Err(e) = hardware.display.as_mut().unwrap().set_power(false) {
                    failure = Some(e.to_string());
                    break;
                }
                on = false;
            }
        } else if authorized
            && core.allowed(Instant::now())
            && !idle
            && !on
            && (dirty || now.duration_since(last_frame) >= Duration::from_millis(16))
        {
            update_model_clock(&mut model, origin, now);
            rendered += 1;
            diag.render_started += 1;
            let render_at = Instant::now();
            if sampled_render_marker {
                eprintln!(
                    "Touchmeter runtime diag: render_start seq={} app={} time={:.3}",
                    diag.render_started, model.app, model.time
                );
                sampled_render_marker = false;
            }
            let frame = ui.render(data.render_model(&model));
            render_total += render_at.elapsed();
            diag.render_completed += 1;
            if frame.len() != 2008 * 60 * 4 {
                failure = Some("Invalid renderer frame length".into());
                break;
            }
            if !match ensure_fresh_or_quiesce(
                &mut core,
                Instant::now(),
                permission_quiesced,
                |core| {
                    quiesce_permission_loss(
                        &mut hardware,
                        &mut controller,
                        &mut model,
                        &mut volume,
                        core,
                        generation.wrapping_add(1).max(1),
                    )
                    .map(|_| ())
                },
            ) {
                Ok(fresh) => fresh,
                Err(e) => {
                    failure = Some(e.clone());
                    held_fault = Some(e);
                    break;
                }
            } {
                permission_quiesced = true;
                on = false;
                continue;
            }
            diag.present_started += 1;
            let present_at = Instant::now();
            let present_result = hardware.display.as_mut().unwrap().present(&frame);
            present_total += present_at.elapsed();
            if present_result.is_ok() {
                diag.present_completed += 1;
            }
            if let Err(e) = present_result {
                failure = Some(e.to_string());
                break;
            }
            presented += 1;
            if !match ensure_fresh_or_quiesce(
                &mut core,
                Instant::now(),
                permission_quiesced,
                |core| {
                    quiesce_permission_loss(
                        &mut hardware,
                        &mut controller,
                        &mut model,
                        &mut volume,
                        core,
                        generation.wrapping_add(1).max(1),
                    )
                    .map(|_| ())
                },
            ) {
                Ok(fresh) => fresh,
                Err(e) => {
                    failure = Some(e.clone());
                    held_fault = Some(e);
                    break;
                }
            } {
                permission_quiesced = true;
                on = false;
                continue;
            }
            if let Err(e) = hardware.display.as_mut().unwrap().set_power(true) {
                failure = Some(e.to_string());
                break;
            }
            on = true;
            if !first_frame_logged {
                eprintln!("Touchmeter: first frame presented; panel active");
                first_frame_logged = true;
            }
            last_frame = now;
            dirty = false;
            last_presented_app = model.app.clone();
        } else if on
            && core.allowed(Instant::now())
            && dirty
            && now.duration_since(last_frame) >= Duration::from_millis(16)
        {
            update_model_clock(&mut model, origin, now);
            rendered += 1;
            diag.render_started += 1;
            let render_at = Instant::now();
            if sampled_render_marker {
                eprintln!(
                    "Touchmeter runtime diag: render_start seq={} app={} time={:.3}",
                    diag.render_started, model.app, model.time
                );
                sampled_render_marker = false;
            }
            let frame = ui.render(data.render_model(&model));
            render_total += render_at.elapsed();
            diag.render_completed += 1;
            if frame.len() != 2008 * 60 * 4 {
                failure = Some("Invalid renderer frame length".into());
                break;
            }
            if !match ensure_fresh_or_quiesce(
                &mut core,
                Instant::now(),
                permission_quiesced,
                |core| {
                    quiesce_permission_loss(
                        &mut hardware,
                        &mut controller,
                        &mut model,
                        &mut volume,
                        core,
                        generation.wrapping_add(1).max(1),
                    )
                    .map(|_| ())
                },
            ) {
                Ok(fresh) => fresh,
                Err(e) => {
                    failure = Some(e.clone());
                    held_fault = Some(e);
                    break;
                }
            } {
                permission_quiesced = true;
                on = false;
                continue;
            }
            diag.present_started += 1;
            let present_at = Instant::now();
            let present_result = hardware.display.as_mut().unwrap().present(&frame);
            present_total += present_at.elapsed();
            if present_result.is_ok() {
                diag.present_completed += 1;
            }
            if let Err(e) = present_result {
                failure = Some(e.to_string());
                break;
            }
            presented += 1;
            dirty = false;
            last_frame = now;
            last_presented_app = model.app.clone();
        } else if on
            && authorized
            && !idle
            && model.fire
            && model.app == "ai"
            && !model.picker
            && now.duration_since(last_frame) >= Duration::from_millis(33)
        {
            update_model_clock(&mut model, origin, now);
            rendered += 1;
            diag.render_started += 1;
            let render_at = Instant::now();
            if sampled_render_marker {
                eprintln!(
                    "Touchmeter runtime diag: render_start seq={} app={} time={:.3}",
                    diag.render_started, model.app, model.time
                );
                sampled_render_marker = false;
            }
            let frame = ui.render(data.render_model(&model));
            render_total += render_at.elapsed();
            diag.render_completed += 1;
            if frame.len() != 2008 * 60 * 4 {
                failure = Some("Invalid renderer frame length".into());
                break;
            }
            if !match ensure_fresh_or_quiesce(
                &mut core,
                Instant::now(),
                permission_quiesced,
                |core| {
                    quiesce_permission_loss(
                        &mut hardware,
                        &mut controller,
                        &mut model,
                        &mut volume,
                        core,
                        generation.wrapping_add(1).max(1),
                    )
                    .map(|_| ())
                },
            ) {
                Ok(fresh) => fresh,
                Err(e) => {
                    failure = Some(e.clone());
                    held_fault = Some(e);
                    break;
                }
            } {
                permission_quiesced = true;
                on = false;
                continue;
            }
            diag.present_started += 1;
            let present_at = Instant::now();
            let present_result = hardware.display.as_mut().unwrap().present(&frame);
            present_total += present_at.elapsed();
            if present_result.is_ok() {
                diag.present_completed += 1;
            }
            if let Err(e) = present_result {
                failure = Some(e.to_string());
                break;
            }
            presented += 1;
            last_frame = now;
            last_presented_app = model.app.clone();
        }
        if model.fire && now.duration_since(diagnostic_at) >= Duration::from_secs(2) {
            eprintln!(
                "Touchmeter runtime diag: iterations={iterations} renders={rendered} presents={presented} drained_touches={drained_touches} rejected_begins={rejected_begins} dirty={dirty} fresh={} fire={} picker={} app={} last_presented_app={} time={:.3}",
                core.allowed(now),
                model.fire,
                model.picker,
                model.app,
                last_presented_app,
                model.time
            );
            diagnostic_at = now;
        }
        if authorized
            && !idle
            && now >= wake_grace_until
            && now.duration_since(last_activity) >= Duration::from_secs(120)
        {
            idle = true;
            let _ = enter_dark(&mut controller, &mut model);
            dirty = true;
            volume.deactivate_now();
        }
    }
    core.revoke();
    data.set_authorized(false);
    volume.set_active(false);
    let _ = controller.cancel(&mut model);
    let outcome = {
        let mut actuator = RuntimeHardwareActuator::new(
            &mut hardware,
            &mut controller,
            &mut model,
            &mut volume,
            &mut core,
            false,
        );
        let mut shutdown_worker = RuntimeShutdownWorker::new(&mut worker);
        drive_retained_shutdown_with_fault(
            generation.wrapping_add(1).max(1),
            &mut actuator,
            &mut shutdown_worker,
            held_fault,
            || loop {
                match recovery_signals.forever().next() {
                    Some(signal_hook::consts::SIGUSR1) => {
                        eprintln!("FAULT_HELD: retrying shutdown quiescence (SIGUSR1)");
                        break super::shutdown::Event::Retry;
                    }
                    Some(signal_hook::consts::SIGUSR2) => {
                        eprintln!(
                            "UNCONFIRMED_RELEASE: operator requested unsafe force release (SIGUSR2)"
                        );
                        break super::shutdown::Event::Force;
                    }
                    Some(signal_hook::consts::SIGTERM | signal_hook::consts::SIGINT) => {
                        eprintln!(
                            "FAULT_HELD: still held; send SIGUSR1 to retry or SIGUSR2 to force"
                        );
                        break super::shutdown::Event::OrdinaryStop;
                    }
                    Some(_) => continue,
                    None => {
                        eprintln!("FAULT_HELD: signal stream ended; retaining hardware owners");
                        thread::park();
                    }
                }
            },
        )
    };
    match outcome {
        super::shutdown::Outcome::SafeStopped => {
            data.shutdown();
            hardware.shutdown();
            failure.map_or(Ok(()), Err)
        }
        super::shutdown::Outcome::ForcedReleased => {
            data.shutdown();
            hardware.shutdown();
            Err("FORCED_RELEASE: operator explicitly requested unverified release".into())
        }
        _ => unreachable!("retained shutdown returns only terminal outcomes"),
    }
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
struct RuntimeShutdown<'a> {
    hardware: &'a mut HardwareRuntime,
    worker: &'a mut super::session_worker::SessionWorker,
    generation: u64,
    off: bool,
}
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl ShutdownActuator for RuntimeShutdown<'_> {
    fn release_keys(&mut self) -> Result<(), String> {
        self.hardware
            .keyboard
            .as_mut()
            .map_or(Ok(()), |k| k.release_all())
    }
    fn deactivate_volume(&mut self) {}
    fn power_off(&mut self) -> Result<(), String> {
        if let Some(d) = self.hardware.display.as_mut() {
            d.set_power(false).map_err(|e| e.to_string())?;
        }
        self.off = true;
        Ok(())
    }
    fn acknowledge_off(&mut self) {
        if self.generation != 0 {
            self.worker.acknowledge_panel_off(self.generation);
        }
    }
    fn stop_worker(&mut self) -> Result<(), String> {
        if self.generation != 0 {
            self.worker.stop_after_panel_off(self.generation);
        }
        if self.worker.stop_and_join(Duration::from_secs(2)) {
            Ok(())
        } else {
            Err("Session worker did not stop within timeout".into())
        }
    }
    fn drop_input_display(&mut self) {
        self.hardware.input.take();
        self.hardware.display.take();
    }
    fn drop_keyboard_lease_lock(&mut self) {
        self.hardware.keyboard.take();
        self.hardware.lease.take();
        self.hardware.lock.take();
    }
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
pub fn diagnostics() -> String {
    "Linux hardware runtime available; no hardware or services queried".into()
}

/// Ordered fail-safe teardown shared by production orchestration and tests.
/// `off` must return success before `ack` is ever called; failures do not skip
/// later resource cleanup.
pub trait ShutdownActuator {
    fn release_keys(&mut self) -> Result<(), String>;
    fn deactivate_volume(&mut self);
    fn power_off(&mut self) -> Result<(), String>;
    fn acknowledge_off(&mut self);
    fn stop_worker(&mut self) -> Result<(), String>;
    fn drop_input_display(&mut self);
    fn drop_keyboard_lease_lock(&mut self);
}
pub fn ordered_shutdown<A: ShutdownActuator>(a: &mut A) -> Result<(), String> {
    let mut errors = Vec::new();
    if let Err(e) = a.release_keys() {
        errors.push(e);
    }
    a.deactivate_volume();
    let off_ok = match a.power_off() {
        Ok(()) => true,
        Err(e) => {
            errors.push(e);
            false
        }
    };
    if off_ok {
        a.acknowledge_off();
    }
    if let Err(e) = a.stop_worker() {
        errors.push(e);
    }
    a.drop_input_display();
    a.drop_keyboard_lease_lock();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Shared revoke gate used by the physical loop and unit tests. Authorization
/// is consumed before any caller can dispatch another input event.
#[derive(Default)]
pub struct RuntimeCore {
    authorized_until: Option<Instant>,
    sleeping: bool,
}
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
fn enter_dark(
    controller: &mut crate::controls::Controller,
    model: &mut touchmeter_ui::Model,
) -> Vec<crate::controls::Effect> {
    controller.set_sleeping(true, model)
}
impl RuntimeCore {
    pub fn refresh_started(&mut self, now: Instant) {
        if self
            .authorized_until
            .is_some_and(|deadline| now >= deadline)
        {
            self.revoke();
        }
    }
    pub fn authorize_until(&mut self, deadline: Instant) {
        self.authorized_until = Some(deadline);
    }
    pub fn revoke(&mut self) {
        self.authorized_until = None;
    }
    pub fn allowed(&self, now: Instant) -> bool {
        !self.sleeping && self.authorized_until.is_some_and(|t| now < t)
    }
    pub fn check_fresh(&mut self, now: Instant) -> bool {
        if self.allowed(now) {
            true
        } else {
            self.revoke();
            false
        }
    }
    pub fn idle(&mut self) {
        self.authorized_until = None;
        self.sleeping = true;
    }
    pub fn wake(&mut self) {
        self.sleeping = false;
        self.authorized_until = None;
    }
}

pub fn acknowledge_after_quiescence<A, E, F>(proof: Result<A, E>, acknowledge: F) -> Result<(), E>
where
    F: FnOnce(),
{
    match proof {
        Ok(_) => {
            acknowledge();
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// Policy state shared by the physical runtime and its deterministic tests.
/// Hardware operations are represented as actions so callers can perform them
/// in the required order and only report successful operations back to policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    PowerOff,
    PowerOn,
    AcknowledgeSleep,
    ReleaseKeys,
    DeactivateVolume,
    CancelCollectors,
    StopWorkers,
    DropInput,
    DropDisplay,
    DropKeyboard,
    DropLease,
    DropLock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    SessionDenied,
    SessionAllowed,
    SleepPreparing,
    SleepResumed,
    Touch,
    LeaseRevoked,
    Shutdown,
    PowerOffSucceeded,
    PowerOffFailed,
}

/// Pure, clock-injected safety policy. It never performs I/O.
#[derive(Debug)]
pub struct Engine {
    allowed_until: Option<Instant>,
    panel_on: bool,
    sleeping: bool,
    shutting_down: bool,
    acknowledged: bool,
    lease_revoked: bool,
    volume_active: bool,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            allowed_until: None,
            panel_on: false,
            sleeping: false,
            shutting_down: false,
            acknowledged: false,
            lease_revoked: false,
            volume_active: false,
        }
    }
}

impl Engine {
    pub fn allowed(&self, now: Instant) -> bool {
        !self.sleeping
            && !self.shutting_down
            && !self.lease_revoked
            && self.allowed_until.is_some_and(|until| now < until)
    }

    pub fn transition(&mut self, event: Event, now: Instant) -> Vec<Action> {
        match event {
            Event::SessionAllowed => {
                if !self.sleeping && !self.shutting_down && !self.lease_revoked {
                    self.allowed_until = Some(now + std::time::Duration::from_secs(1));
                }
                vec![]
            }
            Event::SessionDenied | Event::SleepPreparing | Event::LeaseRevoked => {
                self.allowed_until = None;
                if event == Event::SleepPreparing {
                    self.sleeping = true;
                    self.acknowledged = false;
                }
                if event == Event::LeaseRevoked {
                    self.lease_revoked = true;
                }
                self.volume_active = false;
                let mut a = vec![Action::DeactivateVolume, Action::ReleaseKeys];
                if self.panel_on {
                    a.push(Action::PowerOff);
                }
                a
            }
            Event::SleepResumed => {
                self.sleeping = false;
                self.allowed_until = None;
                vec![]
            }
            Event::Touch if !self.panel_on && self.allowed(now) => vec![Action::PowerOn],
            Event::Touch => vec![],
            Event::PowerOffSucceeded => {
                self.panel_on = false;
                if self.sleeping && !self.acknowledged {
                    self.acknowledged = true;
                    vec![Action::AcknowledgeSleep]
                } else {
                    vec![]
                }
            }
            Event::PowerOffFailed => vec![],
            Event::Shutdown => {
                if self.shutting_down {
                    return vec![];
                }
                self.shutting_down = true;
                self.allowed_until = None;
                self.volume_active = false;
                let mut a = vec![
                    Action::DeactivateVolume,
                    Action::CancelCollectors,
                    Action::ReleaseKeys,
                ];
                if self.panel_on {
                    a.push(Action::PowerOff);
                }
                a.extend([
                    Action::StopWorkers,
                    Action::DropInput,
                    Action::DropDisplay,
                    Action::DropKeyboard,
                    Action::DropLease,
                    Action::DropLock,
                ]);
                a
            }
        }
    }

    pub fn power_on_succeeded(&mut self) {
        self.panel_on = true;
    }
    pub fn set_volume_active(&mut self, on: bool, now: Instant) {
        self.volume_active = on && self.allowed(now);
    }
    pub fn volume_active(&self) -> bool {
        self.volume_active
    }
}
