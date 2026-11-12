//! Touch Bar multitouch policy and explicit Linux evdev adapter.
use std::collections::BTreeMap;

pub const PANEL_WIDTH: f64 = 2007.0;
pub const PANEL_HEIGHT: f64 = 59.0;

#[derive(Clone, Debug, PartialEq)]
pub enum TouchEvent {
    Begin { x: f64, y: f64 },
    Move { x: f64, y: f64 },
    End,
    Cancel,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisRange {
    pub min: i32,
    pub max: i32,
}

pub fn panel_point(x: i32, y: i32, xr: AxisRange, yr: AxisRange) -> Option<(f64, f64)> {
    if xr.max <= xr.min || yr.max <= yr.min {
        return None;
    }
    let scale = |v: i32, r: AxisRange, extent: f64| {
        ((v as f64 - r.min as f64) / (r.max as f64 - r.min as f64)).clamp(0.0, 1.0) * extent
    };
    Some((scale(x, xr, PANEL_WIDTH), scale(y, yr, PANEL_HEIGHT)))
}

/// Data-only discovery validation; never accesses sysfs or /dev/input.
pub fn is_touchbar_device(name: &str, phys: &str) -> bool {
    let Some(model) = name.strip_prefix("MacBookPro") else {
        return false;
    };
    let Some((major, rest)) = model.split_once(',') else {
        return false;
    };
    let Some(minor) = rest.strip_suffix(" Touch Bar") else {
        return false;
    };
    !major.is_empty()
        && major.bytes().all(|b| b.is_ascii_digit())
        && !minor.is_empty()
        && minor.bytes().all(|b| b.is_ascii_digit())
        && phys.trim() == "apple_z2"
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawEvent {
    Slot(usize),
    Tracking(i32),
    PositionX(i32),
    PositionY(i32),
    SyncReport,
    Resync,
    Disconnected,
}
#[derive(Clone, Copy, Debug)]
struct Contact {
    tracking: i32,
    x: i32,
    y: i32,
}
impl Default for Contact {
    fn default() -> Self {
        Self {
            tracking: -1,
            x: 0,
            y: 0,
        }
    }
}

/// Stateful frame processor. Call `event` for each decoded ABS/SYN event.
pub struct InputPolicy {
    slots: BTreeMap<usize, Contact>,
    slot: usize,
    selected_slot: Option<usize>,
    selected: Option<i32>,
    xr: AxisRange,
    yr: AxisRange,
    gate_until_all_up: bool,
}
impl InputPolicy {
    pub fn new(xr: AxisRange, yr: AxisRange) -> Self {
        Self {
            slots: BTreeMap::new(),
            slot: 0,
            selected_slot: None,
            selected: None,
            xr,
            yr,
            gate_until_all_up: true,
        }
    }
    pub fn event(&mut self, e: RawEvent) -> Vec<TouchEvent> {
        match e {
            RawEvent::Slot(s) => self.slot = s,
            RawEvent::Tracking(id) => {
                self.slots.entry(self.slot).or_default().tracking = id;
            }
            RawEvent::PositionX(x) => self.slots.entry(self.slot).or_default().x = x,
            RawEvent::PositionY(y) => self.slots.entry(self.slot).or_default().y = y,
            RawEvent::Resync => {
                self.selected = None;
                self.selected_slot = None;
                self.gate_until_all_up = true;
                return vec![TouchEvent::Cancel];
            }
            RawEvent::Disconnected => {
                let was = self.selected.take().is_some();
                self.selected_slot = None;
                self.gate_until_all_up = true;
                self.slots.clear();
                return if was {
                    vec![TouchEvent::Cancel]
                } else {
                    vec![]
                };
            }
            RawEvent::SyncReport => return self.frame(),
        }
        vec![]
    }
    pub fn frame(&mut self) -> Vec<TouchEvent> {
        let active: Vec<_> = self
            .slots
            .iter()
            .filter(|(_, c)| c.tracking >= 0)
            .map(|(s, c)| (*s, *c))
            .collect();
        if self.gate_until_all_up {
            if active.is_empty() {
                self.gate_until_all_up = false;
            }
            return vec![];
        }
        if let Some(id) = self.selected {
            let selected_slot = self.selected_slot.unwrap_or(self.slot);
            if !self
                .slots
                .get(&selected_slot)
                .is_some_and(|c| c.tracking == id)
            {
                self.selected = None;
                self.selected_slot = None;
                self.gate_until_all_up = !active.is_empty();
                return vec![TouchEvent::End];
            }
            if let Some(c) = self.slots.get(&selected_slot) {
                if let Some((x, y)) = panel_point(c.x, c.y, self.xr, self.yr) {
                    return vec![TouchEvent::Move { x, y }];
                }
            }
            return vec![TouchEvent::Cancel];
        }
        if active.is_empty() {
            return vec![];
        }
        let (slot, c) = active[0];
        self.slot = slot;
        self.selected_slot = Some(slot);
        self.selected = Some(c.tracking);
        panel_point(c.x, c.y, self.xr, self.yr)
            .map(|(x, y)| vec![TouchEvent::Begin { x, y }])
            .unwrap_or_default()
    }
}

/// Open and exclusively grab the validated Apple Touch Bar. Calling this is an
/// explicit hardware operation; importing this module never accesses devices.
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
pub struct DeviceAdapter {
    device: evdev::Device,
    policy: InputPolicy,
    slot: usize,
}

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl DeviceAdapter {
    pub fn start() -> std::io::Result<Self> {
        use evdev::AbsoluteAxisCode as A;
        use std::{fs, io};
        for entry in fs::read_dir("/dev/input")? {
            let path = entry?.path();
            if !path
                .file_name()
                .and_then(|x| x.to_str())
                .is_some_and(|x| x.starts_with("event"))
            {
                continue;
            }
            let Ok(mut d) = evdev::Device::open(&path) else {
                continue;
            };
            let name = d.name().unwrap_or("");
            let phys = d.physical_path().unwrap_or("");
            if !is_touchbar_device(name, phys) {
                continue;
            }
            let Some(axes) = d.supported_absolute_axes() else {
                continue;
            };
            if !axes.contains(A::ABS_MT_SLOT)
                || !axes.contains(A::ABS_MT_TRACKING_ID)
                || !axes.contains(A::ABS_MT_POSITION_X)
                || !axes.contains(A::ABS_MT_POSITION_Y)
            {
                continue;
            }
            let abs = d
                .get_absinfo()?
                .collect::<std::collections::HashMap<_, _>>();
            let Some(xr) = abs.get(&A::ABS_MT_POSITION_X) else {
                continue;
            };
            let Some(yr) = abs.get(&A::ABS_MT_POSITION_Y) else {
                continue;
            };
            let Some(slots) = abs.get(&A::ABS_MT_SLOT) else {
                continue;
            };
            if xr.maximum() <= xr.minimum()
                || yr.maximum() <= yr.minimum()
                || slots.maximum() < slots.minimum()
            {
                continue;
            }
            d.set_nonblocking(true)?;
            d.grab()?;
            return Ok(Self {
                device: d,
                policy: InputPolicy::new(
                    AxisRange {
                        min: xr.minimum(),
                        max: xr.maximum(),
                    },
                    AxisRange {
                        min: yr.minimum(),
                        max: yr.maximum(),
                    },
                ),
                slot: 0,
            });
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "verified MacBook Touch Bar not found",
        ))
    }
    pub fn fd(&self) -> std::os::fd::RawFd {
        use std::os::fd::AsRawFd;
        self.device.as_raw_fd()
    }
    /// Drain available evdev records. SYN_DROPPED/synchronization emits Cancel;
    /// the policy then requires an observed all-up frame before selecting again.
    pub fn drain(&mut self) -> std::io::Result<Vec<TouchEvent>> {
        use evdev::{AbsoluteAxisCode as A, EventSummary, SynchronizationCode};
        let mut out = Vec::new();
        let events = match self.device.fetch_events() {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(out),
            Err(e) => return Err(e),
        };
        for e in events {
            match e.destructure() {
                EventSummary::AbsoluteAxis(_, axis, value) => match axis {
                    A::ABS_MT_SLOT => {
                        self.slot = value.max(0) as usize;
                        out.extend(self.policy.event(RawEvent::Slot(self.slot)));
                    }
                    A::ABS_MT_TRACKING_ID => {
                        out.extend(self.policy.event(RawEvent::Tracking(value)))
                    }
                    A::ABS_MT_POSITION_X => {
                        out.extend(self.policy.event(RawEvent::PositionX(value)))
                    }
                    A::ABS_MT_POSITION_Y => {
                        out.extend(self.policy.event(RawEvent::PositionY(value)))
                    }
                    _ => (),
                },
                EventSummary::Synchronization(_, SynchronizationCode::SYN_REPORT, _) => {
                    out.extend(self.policy.event(RawEvent::SyncReport))
                }
                EventSummary::Synchronization(_, SynchronizationCode::SYN_DROPPED, _) => {
                    out.extend(self.policy.event(RawEvent::Resync))
                }
                _ => (),
            }
        }
        Ok(out)
    }
}
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
impl Drop for DeviceAdapter {
    fn drop(&mut self) {
        let _ = self.device.ungrab();
    }
}
