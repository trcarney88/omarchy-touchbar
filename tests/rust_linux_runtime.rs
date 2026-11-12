#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
#[path = "../src/appearance.rs"]
mod appearance;
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
#[path = "../src/burnrate.rs"]
mod burnrate;
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
#[path = "../src/config.rs"]
mod config;
#[path = "../src/controls.rs"]
mod controls;
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
#[path = "../src/credentials.rs"]
mod credentials;
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
#[path = "../src/fixtures.rs"]
mod fixtures;
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
#[path = "../src/i18n.rs"]
mod i18n;
#[path = "../src/jobs.rs"]
mod jobs;
#[path = "../src/linux/mod.rs"]
mod linux;
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
#[path = "../src/live.rs"]
mod live;
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
#[path = "../src/live_worker.rs"]
mod live_worker;
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
#[path = "../src/testcli.rs"]
mod testcli;

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
fn base() -> touchmeter_ui::Model {
    let mut model = touchmeter_ui::Model::default();
    model.providers = fixtures::PROVIDER_IDS
        .iter()
        .map(|id| touchmeter_ui::Provider {
            id: (*id).into(),
            name: (*id).into(),
            enabled: false,
        })
        .collect();
    model.fire_trigger_minutes = 30.;
    model
}
use linux::runtime;

use runtime::{Action, Engine, Event};
use std::time::{Duration, Instant};

#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
#[test]
fn dirty_ai_fire_frames_advance_monotonic_clock_and_other_views_do_not() {
    let origin = Instant::now();
    let mut model = base();
    model.fire = true;
    model.app = "ai".into();
    for elapsed in [Duration::from_millis(40), Duration::from_millis(80)] {
        runtime::update_model_clock(&mut model, origin, origin + elapsed);
        assert_eq!(model.time, elapsed.as_secs_f32());
    }
    model.picker = true;
    model.time = 7.0;
    runtime::update_model_clock(&mut model, origin, origin + Duration::from_secs(2));
    assert_eq!(model.time, 7.0);
    model.picker = false;
    model.fire = false;
    runtime::update_model_clock(&mut model, origin, origin + Duration::from_secs(3));
    assert_eq!(model.time, 7.0);
}

#[cfg(unix)]
mod lock_tests {
    use super::runtime::RuntimeLock;
    use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};

    fn temp_dir() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let p = std::env::temp_dir().join(format!(
            "touchmeter-lock-test-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
        p
    }

    #[test]
    fn lock_creation_and_inode_guarded_removal() {
        let dir = temp_dir();
        let lock = RuntimeLock::acquire_in(&dir, std::process::id(), "test-host".into()).unwrap();
        assert!(dir.join("touchmeter.lock").exists());
        drop(lock);
        assert!(!dir.join("touchmeter.lock").exists());
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn symlink_runtime_directory_is_rejected() {
        let dir = temp_dir();
        let link = dir.with_file_name(format!(
            "{}-link",
            dir.file_name().unwrap().to_string_lossy()
        ));
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(RuntimeLock::acquire_in(&link, 44, "host".into()).is_err());
        fs::remove_file(link).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn active_pid_lock_is_not_stolen() {
        let dir = temp_dir();
        fs::write(
            dir.join("touchmeter.lock"),
            format!("{}\ntouchmeter\nhost\n", std::process::id()),
        )
        .unwrap();
        fs::set_permissions(
            dir.join("touchmeter.lock"),
            fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert!(
            RuntimeLock::acquire_in(&dir, 45, "host".into())
                .unwrap_err()
                .contains("AlreadyRunning")
        );
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn runtime_core_expires_and_idle_requires_fresh_authorization() {
    use runtime::RuntimeCore;
    let now = Instant::now();
    let mut c = RuntimeCore::default();
    c.authorize_until(now + Duration::from_secs(1));
    assert!(c.allowed(now));
    assert!(!c.allowed(now + Duration::from_secs(1)));
    c.idle();
    c.authorize_until(now + Duration::from_secs(4));
    assert!(!c.allowed(now));
    c.wake();
    assert!(!c.allowed(now));
}

#[test]
fn refresh_keeps_unexpired_grant_and_panel_ack_requires_proof() {
    use runtime::{RuntimeCore, acknowledge_after_quiescence};
    let now = Instant::now();
    let mut core = RuntimeCore::default();
    core.authorize_until(now + Duration::from_secs(1));
    core.refresh_started(now + Duration::from_millis(500));
    assert!(core.allowed(now + Duration::from_millis(500)));
    core.refresh_started(now + Duration::from_secs(1));
    assert!(!core.allowed(now + Duration::from_secs(1)));

    let acknowledgements = std::cell::Cell::new(0);
    assert!(acknowledge_after_quiescence(Ok::<_, &str>(()), || acknowledgements.set(1)).is_ok());
    assert_eq!(acknowledgements.get(), 1);
    assert!(
        acknowledge_after_quiescence(Err::<(), _>("failed"), || acknowledgements.set(2)).is_err()
    );
    assert_eq!(acknowledgements.get(), 1);
}

#[test]
fn freshness_gate_quiesces_before_privileged_dispatch_or_present() {
    use runtime::{RuntimeCore, ensure_fresh_or_quiesce};
    let now = Instant::now();
    let deadline = now + Duration::from_millis(20);
    let mut core = RuntimeCore::default();
    core.authorize_until(deadline);
    let privileged = std::cell::Cell::new(0);
    let barriers = std::cell::Cell::new(0);

    assert!(
        ensure_fresh_or_quiesce(&mut core, now, false, |_| {
            barriers.set(barriers.get() + 1);
            Ok(())
        })
        .unwrap()
    );
    privileged.set(privileged.get() + 1);
    assert_eq!(privileged.get(), 1);

    // A render callback may consume the remaining authorization window; the
    // post-render check must quiesce before presentation.
    let after_render = deadline;
    let presented = std::cell::Cell::new(false);
    let fresh = ensure_fresh_or_quiesce(&mut core, after_render, false, |_| {
        barriers.set(barriers.get() + 1);
        Ok(())
    })
    .unwrap();
    if fresh {
        presented.set(true);
    }
    assert!(!presented.get());
    assert_eq!(barriers.get(), 1);
    assert!(!core.allowed(after_render));
}

#[test]
fn fixture_audio_routing_never_invokes_live_audio_operation() {
    let fixture_writes = std::cell::Cell::new(0);
    let live_writes = std::cell::Cell::new(0);
    runtime::route_fixture_audio_effect(
        true,
        || fixture_writes.set(fixture_writes.get() + 1),
        || live_writes.set(live_writes.get() + 1),
    );
    assert_eq!(fixture_writes.get(), 1);
    assert_eq!(live_writes.get(), 0);
    runtime::route_fixture_audio_effect(
        false,
        || fixture_writes.set(fixture_writes.get() + 1),
        || live_writes.set(live_writes.get() + 1),
    );
    assert_eq!(fixture_writes.get(), 1);
    assert_eq!(live_writes.get(), 1);
}

#[test]
fn retained_shutdown_retries_only_on_retry_and_force_is_explicit() {
    use linux::{quiescence, shutdown};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Actuator {
        fail: std::sync::Arc<std::sync::atomic::AtomicBool>,
        attempts: usize,
    }
    impl quiescence::Actuator for Actuator {
        fn revoke_authority(&mut self) {}
        fn cancel_effects_nonblocking(&mut self) {}
        fn release_keys_and_sync(&mut self, _: Duration) -> Result<(), String> {
            self.attempts += 1;
            if self.fail.load(Ordering::SeqCst) {
                Err("key barrier failed".into())
            } else {
                Ok(())
            }
        }
        fn fresh_panel_off(&mut self) -> Result<(), String> {
            if self.fail.load(Ordering::SeqCst) {
                Err("off failed".into())
            } else {
                Ok(())
            }
        }
        fn blackout_existing_framebuffer(&mut self) -> Result<(), String> {
            Ok(())
        }
    }
    struct Worker {
        completed: usize,
        forced: usize,
    }
    impl shutdown::Worker for Worker {
        fn request_park(&mut self, _: u64) -> Result<(), String> {
            Ok(())
        }
        fn wait_ready(&mut self, _: u64, _: Duration) -> Result<(), String> {
            Ok(())
        }
        fn complete_and_join(
            &mut self,
            _: &quiescence::QuiescenceToken,
            _: Duration,
        ) -> Result<(), String> {
            self.completed += 1;
            Ok(())
        }
        fn force_stop_and_join(&mut self, _: Duration) -> Result<(), String> {
            self.forced += 1;
            Ok(())
        }
    }

    let events = AtomicUsize::new(0);
    let fail = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let release_failure = fail.clone();
    let mut a = Actuator { fail, attempts: 0 };
    let mut w = Worker {
        completed: 0,
        forced: 0,
    };
    let result = runtime::drive_retained_shutdown(44, &mut a, &mut w, || {
        match events.fetch_add(1, Ordering::SeqCst) {
            0 => shutdown::Event::OrdinaryStop,
            1 => shutdown::Event::TimeAdvanced,
            _ => {
                release_failure.store(false, Ordering::SeqCst);
                shutdown::Event::Retry
            }
        }
    });
    assert_eq!(result, shutdown::Outcome::SafeStopped);
    assert_eq!(a.attempts, 2);
    assert_eq!(w.completed, 1);

    let mut a = Actuator {
        fail: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
        attempts: 0,
    };
    let mut w = Worker {
        completed: 0,
        forced: 0,
    };
    let result = runtime::drive_retained_shutdown(45, &mut a, &mut w, || shutdown::Event::Force);
    assert_eq!(result, shutdown::Outcome::ForcedReleased);
    assert_eq!(w.completed, 0);
    assert_eq!(w.forced, 1);
}

#[test]
fn shared_shutdown_releases_before_wait_and_never_acks_failed_off() {
    use runtime::{ShutdownActuator, ordered_shutdown};
    struct Fake {
        trace: Vec<&'static str>,
        fail: bool,
    }
    impl ShutdownActuator for Fake {
        fn release_keys(&mut self) -> Result<(), String> {
            self.trace.push("keys");
            Ok(())
        }
        fn deactivate_volume(&mut self) {
            self.trace.push("volume")
        }
        fn power_off(&mut self) -> Result<(), String> {
            self.trace.push("off");
            if self.fail {
                Err("off failed".into())
            } else {
                Ok(())
            }
        }
        fn acknowledge_off(&mut self) {
            self.trace.push("ack")
        }
        fn stop_worker(&mut self) -> Result<(), String> {
            self.trace.push("worker");
            Ok(())
        }
        fn drop_input_display(&mut self) {
            self.trace.push("input/display")
        }
        fn drop_keyboard_lease_lock(&mut self) {
            self.trace.push("keyboard/lease/lock")
        }
    }
    let mut bad = Fake {
        trace: vec![],
        fail: true,
    };
    assert!(ordered_shutdown(&mut bad).is_err());
    assert_eq!(
        bad.trace,
        [
            "keys",
            "volume",
            "off",
            "worker",
            "input/display",
            "keyboard/lease/lock"
        ]
    );
    let mut ok = Fake {
        trace: vec![],
        fail: false,
    };
    ordered_shutdown(&mut ok).unwrap();
    assert_eq!(
        ok.trace,
        [
            "keys",
            "volume",
            "off",
            "ack",
            "worker",
            "input/display",
            "keyboard/lease/lock"
        ]
    );
}

#[test]
fn controller_idle_wake_consumes_first_contact() {
    let mut model = touchmeter_ui::Model::default();
    model.app = "functions".into();
    let mut controller = controls::Controller::default();
    controller.set_sleeping(true, &mut model);
    let effects = controller.handle(controls::InputEvent::Begin(80., 20.), &mut model);
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, controls::Effect::Activity))
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, controls::Effect::FunctionKey { pressed: true, .. }))
    );
    controller.handle(controls::InputEvent::End, &mut model);
    let next = controller.handle(controls::InputEvent::Begin(80., 20.), &mut model);
    assert!(
        next.iter()
            .any(|e| matches!(e, controls::Effect::FunctionKey { pressed: true, .. }))
    );
}

#[test]
fn fresh_clock_gate_rejects_action_at_expiry_and_revokes_cached_allow() {
    use runtime::RuntimeCore;
    let now = Instant::now();
    let mut core = RuntimeCore::default();
    core.authorize_until(now + Duration::from_secs(1));
    assert!(!core.check_fresh(now + Duration::from_secs(1)));
    assert!(!core.allowed(now));
}

#[test]
fn deny_wins_over_touch_and_session_expiry_is_fail_closed() {
    let now = Instant::now();
    let mut engine = Engine::default();
    engine.transition(Event::SessionAllowed, now);
    assert!(engine.allowed(now));
    engine.transition(Event::SessionDenied, now);
    assert!(!engine.allowed(now));
    assert!(engine.transition(Event::Touch, now).is_empty());
    engine.transition(Event::SessionAllowed, now);
    assert!(!engine.allowed(now + Duration::from_secs(1)));
}

#[test]
fn sleep_ack_requires_successful_panel_off_and_resume_requires_fresh_check() {
    let now = Instant::now();
    let mut engine = Engine::default();
    engine.power_on_succeeded();
    engine.transition(Event::SleepPreparing, now);
    assert!(engine.transition(Event::PowerOffFailed, now).is_empty());
    assert_eq!(
        engine.transition(Event::PowerOffSucceeded, now),
        vec![Action::AcknowledgeSleep]
    );
    engine.transition(Event::SleepResumed, now);
    assert!(!engine.allowed(now));
}

#[test]
fn revocation_fails_closed_and_shutdown_is_idempotent_in_order() {
    let now = Instant::now();
    let mut engine = Engine::default();
    engine.transition(Event::SessionAllowed, now);
    engine.transition(Event::LeaseRevoked, now);
    engine.transition(Event::SessionAllowed, now);
    assert!(!engine.allowed(now));
    let actions = engine.transition(Event::Shutdown, now);
    assert!(
        actions.iter().position(|a| *a == Action::DropDisplay)
            < actions.iter().position(|a| *a == Action::DropLease)
    );
    assert!(engine.transition(Event::Shutdown, now).is_empty());
}
