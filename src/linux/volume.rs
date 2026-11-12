//! Pure volume policy plus an explicitly started wpctl adapter.
use crate::jobs::{Job, JobResult};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeState {
    pub available: bool,
    pub fraction: f64,
    pub muted: bool,
}
impl Default for VolumeState {
    fn default() -> Self {
        Self {
            available: false,
            fraction: 0.0,
            muted: false,
        }
    }
}

pub fn parse_volume(bytes: &[u8]) -> Option<(f64, bool)> {
    if bytes.len() > 256 || !bytes.is_ascii() {
        return None;
    }
    let s = std::str::from_utf8(bytes).ok()?.trim();
    let rest = s.strip_prefix("Volume:")?.trim_start();
    let (number, muted) = if let Some(n) = rest.strip_suffix(" [MUTED]") {
        (n.trim_end(), true)
    } else {
        (rest, false)
    };
    if number.is_empty()
        || !number.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        || number.matches('.').count() > 1
    {
        return None;
    }
    let v: number::Number = number.parse().ok()?;
    if !v.0.is_finite() {
        return None;
    }
    Some((v.0.clamp(0.0, 1.0), muted))
}
mod number {
    pub struct Number(pub f64);
    impl std::str::FromStr for Number {
        type Err = ();
        fn from_str(s: &str) -> Result<Self, ()> {
            if s.ends_with('.') || s.starts_with('.') {
                return Err(());
            }
            s.parse::<f64>().map(Self).map_err(|_| ())
        }
    }
}

pub fn command(program: &str, operation: Operation) -> (&str, Vec<String>) {
    let args = match operation {
        Operation::Read => vec!["get-volume".into(), "@DEFAULT_AUDIO_SINK@".into()],
        Operation::SetVolume(v) => vec![
            "set-volume".into(),
            "--limit".into(),
            "1.0".into(),
            "@DEFAULT_AUDIO_SINK@".into(),
            format!("{:.3}", v.clamp(0.0, 1.0)),
        ],
        Operation::SetMute(m) => vec![
            "set-mute".into(),
            "@DEFAULT_AUDIO_SINK@".into(),
            if m { "1" } else { "0" }.into(),
        ],
    };
    (program, args)
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Operation {
    Read,
    SetVolume(f64),
    SetMute(bool),
}

/// In-memory caller policy. `tick` advances work; no process is started until explicit `start`.
pub struct VolumeAdapter {
    pub state: VolumeState,
    active: bool,
    program: String,
    job: Option<(Job, Operation)>,
    volume: Option<f64>,
    mute: Option<bool>,
    read_pending: bool,
    flush_at: Option<Duration>,
    last_poll: Duration,
    clock: Duration,
    generation: u64,
}
impl VolumeAdapter {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            state: VolumeState::default(),
            active: false,
            program: program.into(),
            job: None,
            volume: None,
            mute: None,
            read_pending: false,
            flush_at: None,
            last_poll: Duration::ZERO,
            clock: Duration::ZERO,
            generation: 0,
        }
    }
    pub fn set_active(&mut self, active: bool) {
        if !active {
            self.deactivate_now();
            self.finish_cancel(Duration::from_millis(250));
            return;
        }
        if self.active == active {
            return;
        }
        self.active = active;
        self.generation += 1;
        self.state.available = false;
        self.read_pending = true;
        self.last_poll = Duration::ZERO;
    }
    /// Immediately make the adapter inert and request cancellation without waiting.
    /// Any unfinished job remains owned here until it is reaped by `tick` or
    /// `finish_cancel`.
    pub fn deactivate_now(&mut self) {
        if self.active {
            self.active = false;
            self.generation += 1;
        }
        self.volume = None;
        self.mute = None;
        self.read_pending = false;
        self.flush_at = None;
        if let Some((job, _)) = self.job.as_mut() {
            job.cancel();
        }
    }
    /// Wait up to `timeout` for a cancelled job to finish and be reaped.
    pub fn finish_cancel(&mut self, timeout: Duration) -> bool {
        let Some((job, _)) = self.job.as_mut() else {
            return true;
        };
        if job.cancel_and_wait(timeout).is_some() {
            self.job = None;
            true
        } else {
            false
        }
    }
    pub fn set_volume(&mut self, v: f64) {
        if self.active && self.state.available && v.is_finite() {
            self.state.fraction = v.clamp(0.0, 1.0);
            self.volume = Some(self.state.fraction);
            self.flush_at
                .get_or_insert(self.clock + Duration::from_millis(40));
        }
    }
    pub fn toggle_mute(&mut self) {
        if self.active && self.state.available {
            self.state.muted = !self.state.muted;
            self.mute = Some(self.state.muted);
            self.flush_at
                .get_or_insert(self.clock + Duration::from_millis(40));
        }
    }
    pub fn tick(&mut self, now: Duration) {
        self.clock = now;
        if !self.active {
            let stopped = self
                .job
                .as_mut()
                .is_some_and(|(job, _)| job.poll().is_some());
            if stopped {
                self.job = None;
            }
            return;
        }
        if !self.active {
            return;
        }
        if self.job.is_some() {
            let result = self.job.as_mut().and_then(|(j, _)| j.poll());
            if let Some(r) = result {
                let (_, op) = self.job.take().unwrap();
                if r.generation != self.generation || !self.active {
                    // Reaped stale/cancelled job; do not apply its result.
                } else {
                    self.finish(op, r);
                }
            } else {
                return;
            }
        }
        if now.saturating_sub(self.last_poll) >= Duration::from_secs(1) {
            self.last_poll = now;
            self.read_pending = true;
        }
        if self.flush_at.is_some_and(|at| now >= at) {
            self.flush_at = None;
        } else if self.flush_at.is_some() {
            return;
        }
        let op = if let Some(v) = self.volume.take() {
            Some(Operation::SetVolume(v))
        } else if let Some(m) = self.mute.take() {
            Some(Operation::SetMute(m))
        } else if self.read_pending {
            self.read_pending = false;
            Some(Operation::Read)
        } else {
            None
        };
        if let Some(op) = op {
            let (p, a) = command(&self.program, op);
            let args: Vec<_> = a.iter().map(String::as_str).collect();
            self.job = Job::start(
                p,
                args,
                [("LC_ALL", "C")],
                Duration::from_secs(2),
                256,
                self.generation,
            )
            .ok()
            .map(|j| (j, op));
            if self.job.is_none() {
                self.state.available = false;
            }
        }
    }
    fn finish(&mut self, op: Operation, r: JobResult) {
        if r.generation != self.generation || !self.active {
            return;
        }
        if r.error.is_some() || r.status != Some(0) {
            self.state.available = false;
            self.volume = None;
            self.mute = None;
            return;
        }
        match op {
            Operation::Read => {
                if let Some((v, m)) = parse_volume(&r.stdout) {
                    if self.volume.is_none() {
                        self.state.fraction = v
                    }
                    if self.mute.is_none() {
                        self.state.muted = m
                    }
                    self.state.available = true
                } else {
                    self.state.available = false
                }
            }
            _ => self.read_pending = true,
        }
    }
}
