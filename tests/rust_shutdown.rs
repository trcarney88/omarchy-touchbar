#[path = "../src/linux/quiescence.rs"]
mod quiescence;
#[path = "../src/linux/shutdown.rs"]
mod shutdown;

use quiescence::{Actuator, QuiescenceToken};
use shutdown::{Event, Outcome, ShutdownTransaction, State, Worker};
use std::{cell::Cell, collections::VecDeque, time::Duration};

struct A {
    key: bool,
    offs: VecDeque<bool>,
    trace: Vec<&'static str>,
}
impl A {
    fn good() -> Self {
        Self {
            key: true,
            offs: [true].into(),
            trace: vec![],
        }
    }
}
impl Actuator for A {
    fn revoke_authority(&mut self) {
        self.trace.push("revoke")
    }
    fn cancel_effects_nonblocking(&mut self) {
        self.trace.push("cancel")
    }
    fn release_keys_and_sync(&mut self, _: Duration) -> Result<(), String> {
        self.trace.push("keys");
        if self.key { Ok(()) } else { Err("key".into()) }
    }
    fn fresh_panel_off(&mut self) -> Result<(), String> {
        self.trace.push("off");
        if self.offs.pop_front().unwrap_or(true) {
            Ok(())
        } else {
            Err("off".into())
        }
    }
    fn blackout_existing_framebuffer(&mut self) -> Result<(), String> {
        self.trace.push("blackout");
        Ok(())
    }
}
struct W {
    ready: bool,
    join: bool,
    calls: Vec<&'static str>,
    drops: Option<Cell<usize>>,
}
impl Worker for W {
    fn request_park(&mut self, t: u64) -> Result<(), String> {
        assert_eq!(t, 9);
        self.calls.push("park");
        Ok(())
    }
    fn wait_ready(&mut self, t: u64, _: Duration) -> Result<(), String> {
        assert_eq!(t, 9);
        self.calls.push("ready");
        if self.ready {
            Ok(())
        } else {
            Err("timeout".into())
        }
    }
    fn complete_and_join(&mut self, t: &QuiescenceToken, _: Duration) -> Result<(), String> {
        assert_eq!(t.transaction(), 9);
        self.calls.push("complete");
        if self.join {
            Ok(())
        } else {
            Err("join timeout".into())
        }
    }
    fn force_stop_and_join(&mut self, _: Duration) -> Result<(), String> {
        self.calls.push("force");
        Ok(())
    }
}
impl Drop for W {
    fn drop(&mut self) {
        if let Some(n) = &self.drops {
            n.set(n.get() + 1)
        }
    }
}
fn worker() -> W {
    W {
        ready: true,
        join: true,
        calls: vec![],
        drops: None,
    }
}

#[test]
fn quiescence_failure_holds_without_ack_or_drop() {
    let n = Cell::new(0);
    let mut a = A::good();
    a.key = false;
    let mut w = worker();
    w.drops = Some(Cell::new(0));
    let mut d = ShutdownTransaction::new(9);
    assert_eq!(d.handle(Event::OrdinaryStop, &mut a, &mut w), Outcome::Held);
    assert!(!w.calls.contains(&"complete"));
    assert_eq!(n.get(), 0);
    assert_eq!(d.state(), State::FaultHeld);
    assert_eq!(a.trace, ["revoke", "cancel", "keys", "off"]);
}
#[test]
fn failed_off_fallback_retry_holds() {
    let mut a = A {
        key: true,
        offs: [false, false].into(),
        trace: vec![],
    };
    let mut w = worker();
    let mut d = ShutdownTransaction::new(9);
    assert_eq!(d.handle(Event::OrdinaryStop, &mut a, &mut w), Outcome::Held);
    assert!(!w.calls.contains(&"complete"));
    assert_eq!(
        a.trace,
        ["revoke", "cancel", "keys", "off", "blackout", "off"]
    );
}
#[test]
fn ready_worker_completes_only_after_token() {
    let mut a = A::good();
    let mut w = worker();
    let mut d = ShutdownTransaction::new(9);
    assert_eq!(
        d.handle(Event::OrdinaryStop, &mut a, &mut w),
        Outcome::SafeStopped
    );
    assert_eq!(w.calls, ["park", "ready", "complete"]);
}
#[test]
fn readiness_and_join_failures_are_held_not_forced() {
    for ready in [false, true] {
        let mut a = A::good();
        let mut w = worker();
        w.ready = ready;
        w.join = false;
        let mut d = ShutdownTransaction::new(9);
        assert_eq!(d.handle(Event::OrdinaryStop, &mut a, &mut w), Outcome::Held);
        assert_eq!(d.state(), State::FaultHeld);
        assert!(!w.calls.contains(&"force"));
    }
}
#[test]
fn retry_is_explicit_once_and_time_allow_do_nothing() {
    let mut a = A::good();
    a.key = false;
    let mut w = worker();
    let mut d = ShutdownTransaction::new(9);
    assert_eq!(d.handle(Event::OrdinaryStop, &mut a, &mut w), Outcome::Held);
    let before = a.trace.len();
    assert_eq!(
        d.handle(Event::TimeAdvanced, &mut a, &mut w),
        Outcome::Noop(State::FaultHeld)
    );
    assert_eq!(
        d.handle(Event::Allow, &mut a, &mut w),
        Outcome::Noop(State::FaultHeld)
    );
    assert_eq!(a.trace.len(), before);
    a.key = true;
    assert_eq!(d.handle(Event::Retry, &mut a, &mut w), Outcome::SafeStopped);
    assert_eq!(d.attempt_count, 2);
}
#[test]
fn force_is_distinct_and_does_not_complete_safely() {
    let mut a = A::good();
    a.key = false;
    let mut w = worker();
    let mut d = ShutdownTransaction::new(9);
    assert_eq!(d.handle(Event::OrdinaryStop, &mut a, &mut w), Outcome::Held);
    assert_eq!(
        d.handle(Event::Force, &mut a, &mut w),
        Outcome::ForcedReleased
    );
    assert_eq!(d.state(), State::ForcedReleased);
    assert!(!w.calls.contains(&"complete"));
    assert!(w.calls.contains(&"force"));
}
