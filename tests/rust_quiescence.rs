#[path = "../src/linux/quiescence.rs"]
mod quiescence;

use quiescence::{Actuator, KEY_RELEASE_TIMEOUT, attempt};
use std::collections::VecDeque;
use std::time::Duration;

struct Fake {
    trace: Vec<&'static str>,
    keys: Result<(), &'static str>,
    offs: VecDeque<Result<(), &'static str>>,
    blackout: Result<(), &'static str>,
}
impl Fake {
    fn ok() -> Self {
        Self {
            trace: vec![],
            keys: Ok(()),
            offs: [Ok(())].into(),
            blackout: Ok(()),
        }
    }
}
impl Actuator for Fake {
    fn revoke_authority(&mut self) {
        self.trace.push("revoke");
    }
    fn cancel_effects_nonblocking(&mut self) {
        self.trace.push("cancel");
    }
    fn release_keys_and_sync(&mut self, timeout: Duration) -> Result<(), String> {
        assert_eq!(timeout, KEY_RELEASE_TIMEOUT);
        self.trace.push("keys");
        self.keys.map_err(str::to_owned)
    }
    fn fresh_panel_off(&mut self) -> Result<(), String> {
        self.trace.push("off");
        self.offs
            .pop_front()
            .unwrap_or(Ok(()))
            .map_err(str::to_owned)
    }
    fn blackout_existing_framebuffer(&mut self) -> Result<(), String> {
        self.trace.push("blackout");
        self.blackout.map_err(str::to_owned)
    }
}

#[test]
fn success_order_and_token_transaction() {
    let mut a = Fake::ok();
    let token = attempt(&mut a, 42).unwrap();
    assert_eq!(token.transaction(), 42);
    assert_eq!(a.trace, ["revoke", "cancel", "keys", "off"]);
}

#[test]
fn key_failure_and_off_success_never_yields_token() {
    let mut a = Fake::ok();
    a.keys = Err("keys");
    let e = attempt(&mut a, 1).unwrap_err();
    assert_eq!(e.key_release_error.as_deref(), Some("keys"));
    assert_eq!(e.initial_panel_off_error, None);
    assert_eq!(a.trace, ["revoke", "cancel", "keys", "off"]);
}

#[test]
fn failed_off_blackouts_and_retries_once_but_requires_key_barrier() {
    let mut a = Fake::ok();
    a.keys = Err("keys");
    a.offs = [Err("first"), Ok(())].into();
    let e = attempt(&mut a, 2).unwrap_err();
    assert_eq!(e.attempts, 1);
    assert_eq!(e.initial_panel_off_error.as_deref(), Some("first"));
    assert_eq!(
        a.trace,
        ["revoke", "cancel", "keys", "off", "blackout", "off"]
    );
}

#[test]
fn both_failures_are_recorded_and_all_required_steps_attempted() {
    let mut a = Fake::ok();
    a.keys = Err("keys");
    a.offs = [Err("first"), Err("retry")].into();
    let e = attempt(&mut a, 3).unwrap_err();
    assert_eq!(e.key_release_error.as_deref(), Some("keys"));
    assert_eq!(e.initial_panel_off_error.as_deref(), Some("first"));
    assert_eq!(e.retry_panel_off_error.as_deref(), Some("retry"));
    assert_eq!(e.blackout_error, None);
    assert_eq!(
        a.trace,
        ["revoke", "cancel", "keys", "off", "blackout", "off"]
    );
}

#[test]
fn no_resource_is_owned_or_dropped_by_helper() {
    use std::cell::Cell;
    struct Spy<'a>(&'a Cell<usize>);
    impl Drop for Spy<'_> {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    let drops = Cell::new(0);
    let held = Spy(&drops);
    let mut a = Fake::ok();
    let _ = attempt(&mut a, 4).unwrap();
    assert_eq!(drops.get(), 0);
    drop(held);
    assert_eq!(drops.get(), 1);
}
