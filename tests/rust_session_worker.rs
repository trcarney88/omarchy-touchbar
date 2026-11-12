#![cfg(all(target_os = "linux", feature = "linux-hardware"))]

#[path = "../src/jobs.rs"]
mod jobs;
#[path = "../src/linux/quiescence.rs"]
mod quiescence;
#[path = "../src/linux/session.rs"]
mod session;
#[path = "../src/linux/session_worker.rs"]
mod session_worker;
use session::SessionSnapshot;
use session_worker::{SessionEvent, SessionSource, SessionWorker};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

struct FakeActuator(Arc<AtomicBool>);
impl quiescence::Actuator for FakeActuator {
    fn revoke_authority(&mut self) {}
    fn cancel_effects_nonblocking(&mut self) {}
    fn release_keys_and_sync(&mut self, _: Duration) -> Result<(), String> {
        Ok(())
    }
    fn fresh_panel_off(&mut self) -> Result<(), String> {
        self.0.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn blackout_existing_framebuffer(&mut self) -> Result<(), String> {
        Ok(())
    }
}
fn token(transaction: u64, off: Arc<AtomicBool>) -> quiescence::QuiescenceToken {
    quiescence::attempt(&mut FakeActuator(off), transaction).unwrap()
}
fn wait_event(worker: &SessionWorker, predicate: impl Fn(&SessionEvent) -> bool) -> SessionEvent {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let event = worker
            .recv_timeout(Duration::from_millis(50))
            .expect("worker event within deadline");
        if predicate(&event) {
            return event;
        }
        assert!(Instant::now() < deadline, "expected event not received");
    }
}
fn start_drop_counter(drops: Arc<AtomicUsize>) -> SessionWorker {
    SessionWorker::start_with(move || {
        struct Counted(Arc<AtomicUsize>);
        impl Drop for Counted {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::SeqCst);
            }
        }
        impl SessionSource for Counted {
            fn begin_refresh(&mut self) -> Result<(), ()> {
                Ok(())
            }
            fn poll(&mut self, now: Instant) -> SessionSnapshot {
                SessionSnapshot {
                    allowed: true,
                    preparing_sleep: false,
                    preparing_shutdown: false,
                    valid_until: now + Duration::from_secs(1),
                    requires_panel_off: false,
                }
            }
            fn acquire_inhibitor(&mut self) -> Result<(), ()> {
                Ok(())
            }
            fn acknowledge_panel_off(&mut self) -> bool {
                false
            }
        }
        Ok(Counted(drops))
    })
    .unwrap()
}

#[test]
fn successful_refreshes_republish_before_previous_grant_expires() {
    struct TimedSource(Arc<std::sync::Mutex<Vec<Instant>>>);
    impl SessionSource for TimedSource {
        fn begin_refresh(&mut self) -> Result<(), ()> {
            self.0.lock().unwrap().push(Instant::now());
            Ok(())
        }
        fn poll(&mut self, now: Instant) -> SessionSnapshot {
            let start = *self.0.lock().unwrap().last().unwrap();
            SessionSnapshot {
                allowed: true,
                preparing_sleep: false,
                preparing_shutdown: false,
                valid_until: start + Duration::from_secs(1),
                requires_panel_off: false,
            }
        }
        fn acquire_inhibitor(&mut self) -> Result<(), ()> {
            Ok(())
        }
        fn acknowledge_panel_off(&mut self) -> bool {
            false
        }
    }
    let starts = Arc::new(std::sync::Mutex::new(Vec::new()));
    let source_starts = starts.clone();
    let mut worker = SessionWorker::start_with(move || Ok(TimedSource(source_starts))).unwrap();
    let mut snapshots = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(2);
    while snapshots.len() < 4 && Instant::now() < deadline {
        if let Ok(event) = worker.recv_timeout(Duration::from_millis(50)) {
            if let SessionEvent::Snapshot { generation, .. } = event {
                snapshots.push((generation, Instant::now()));
            }
        }
    }
    worker.force_stop();
    assert!(worker.stop_and_join(Duration::from_secs(2)));
    assert_eq!(snapshots.len(), 4, "four snapshots within bounded wait");
    let begins = starts.lock().unwrap();
    for (index, (generation, arrived)) in snapshots.iter().enumerate() {
        assert_eq!(*generation, index as u64 + 1);
        if index > 0 {
            assert!(*arrived < begins[index - 1] + Duration::from_secs(1));
        }
    }
}

#[test]
fn quiescence_token_completes_parked_worker_and_drops_source_on_join() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut worker = start_drop_counter(drops.clone());
    worker.begin_quiescence(77);
    assert!(matches!(
        wait_event(&worker, |e| matches!(
            e,
            SessionEvent::QuiescenceReady { transaction: 77 }
        )),
        SessionEvent::QuiescenceReady { transaction: 77 }
    ));
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    let off = Arc::new(AtomicBool::new(false));
    worker.complete_quiescence(&token(77, off.clone()));
    assert!(off.load(Ordering::SeqCst));
    assert!(matches!(
        wait_event(&worker, |e| matches!(
            e,
            SessionEvent::QuiescenceStopped { transaction: 77 }
        )),
        SessionEvent::QuiescenceStopped { transaction: 77 }
    ));
    assert!(worker.stop_and_join(Duration::from_secs(2)));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn wrong_quiescence_token_does_not_stop_worker_force_stop_needs_no_ack() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut worker = start_drop_counter(drops.clone());
    worker.begin_quiescence(77);
    wait_event(&worker, |e| {
        matches!(e, SessionEvent::QuiescenceReady { transaction: 77 })
    });
    worker.complete_quiescence(&token(78, Arc::new(AtomicBool::new(false))));
    assert!(!worker.stop_and_join(Duration::from_millis(50)));
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    worker.force_stop();
    assert!(worker.stop_and_join(Duration::from_secs(2)));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn generation_zero_park_and_complete_survive_timed_join_forced_cleanup() {
    let drops = Arc::new(AtomicUsize::new(0));
    let mut worker = start_drop_counter(drops.clone());
    worker.begin_quiescence(91);
    wait_event(&worker, |e| {
        matches!(e, SessionEvent::QuiescenceReady { transaction: 91 })
    });
    assert!(!worker.stop_and_join(Duration::from_millis(30)));
    let off = Arc::new(AtomicBool::new(false));
    worker.complete_quiescence(&token(91, off.clone()));
    assert!(matches!(
        wait_event(&worker, |e| matches!(
            e,
            SessionEvent::QuiescenceStopped { transaction: 91 }
        )),
        SessionEvent::QuiescenceStopped { transaction: 91 }
    ));
    assert!(worker.stop_and_join(Duration::from_secs(2)));
    assert!(off.load(Ordering::SeqCst));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

struct Fake {
    began: Arc<AtomicBool>,
    ack: Arc<AtomicBool>,
}
impl SessionSource for Fake {
    fn begin_refresh(&mut self) -> Result<(), ()> {
        self.began.store(true, Ordering::SeqCst);
        Ok(())
    }
    fn poll(&mut self, now: Instant) -> SessionSnapshot {
        SessionSnapshot {
            allowed: false,
            preparing_sleep: true,
            preparing_shutdown: false,
            valid_until: now + Duration::from_millis(80),
            requires_panel_off: true,
        }
    }
    fn acquire_inhibitor(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn acknowledge_panel_off(&mut self) -> bool {
        self.ack.store(true, Ordering::SeqCst);
        true
    }
}
#[test]
fn announces_denial_before_blocking_refresh_and_requires_matching_ack_for_stop() {
    let began = Arc::new(AtomicBool::new(false));
    let ack = Arc::new(AtomicBool::new(false));
    let b = began.clone();
    let a = ack.clone();
    let mut worker = SessionWorker::start_with(move || Ok(Fake { began: b, ack: a })).unwrap();
    let ev = worker.recv_timeout(Duration::from_secs(1)).unwrap();
    let generation = match ev {
        SessionEvent::RefreshStarted { generation } => generation,
        _ => panic!("expected denial event first"),
    };
    assert!(began.load(Ordering::SeqCst));
    worker.acknowledge_panel_off(generation.wrapping_add(1));
    worker.stop_after_panel_off(generation.wrapping_add(1));
    let snapshot = worker.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(
        matches!(snapshot,SessionEvent::Snapshot{generation:g,snapshot} if g==generation && !snapshot.allowed && snapshot.requires_panel_off)
    );
    worker.acknowledge_panel_off(generation);
    std::thread::sleep(Duration::from_millis(30));
    assert!(ack.load(Ordering::SeqCst));
    worker.stop_after_panel_off(generation);
    assert!(worker.stop_and_join(Duration::from_secs(1)));
}

#[test]
fn sleep_and_shutdown_both_require_panel_off_and_ack() {
    assert!(session::requires_panel_off(Some(true), Some(false)));
    assert!(session::requires_panel_off(Some(false), Some(true)));
    assert!(session::requires_panel_off(None, Some(false)));
    assert!(!session::requires_panel_off(Some(false), Some(false)));
}

struct Awake(Arc<AtomicBool>);
impl SessionSource for Awake {
    fn begin_refresh(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn poll(&mut self, now: Instant) -> SessionSnapshot {
        SessionSnapshot {
            allowed: true,
            preparing_sleep: false,
            preparing_shutdown: false,
            valid_until: now + Duration::from_secs(1),
            requires_panel_off: false,
        }
    }
    fn acquire_inhibitor(&mut self) -> Result<(), ()> {
        Ok(())
    }
    fn acknowledge_panel_off(&mut self) -> bool {
        false
    }
}
#[test]
fn awake_stop_after_confirmed_panel_off_does_not_need_logind_ack() {
    let mut w = SessionWorker::start_with(|| Ok(Awake(Arc::new(AtomicBool::new(false))))).unwrap();
    let generation = match w.recv_timeout(Duration::from_secs(1)).unwrap() {
        SessionEvent::RefreshStarted { generation } => generation,
        _ => panic!(),
    };
    let _ = w.recv_timeout(Duration::from_secs(1)).unwrap();
    w.stop_after_panel_off(generation.wrapping_add(1));
    assert!(!w.stop_and_join(Duration::from_millis(20)));
    w.stop_after_panel_off(generation);
    assert!(w.stop_and_join(Duration::from_secs(1)));
}

struct SleepResume {
    begins: Arc<std::sync::atomic::AtomicUsize>,
    acquires: Arc<std::sync::atomic::AtomicUsize>,
    acks: Arc<std::sync::atomic::AtomicUsize>,
}
impl SessionSource for SleepResume {
    fn begin_refresh(&mut self) -> Result<(), ()> {
        self.begins.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn poll(&mut self, now: Instant) -> SessionSnapshot {
        let preparing = self.begins.load(Ordering::SeqCst) <= 2;
        SessionSnapshot {
            allowed: !preparing,
            preparing_sleep: preparing,
            preparing_shutdown: false,
            valid_until: now + Duration::from_secs(1),
            requires_panel_off: preparing,
        }
    }
    fn acquire_inhibitor(&mut self) -> Result<(), ()> {
        self.acquires.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
    fn acknowledge_panel_off(&mut self) -> bool {
        self.acks.fetch_add(1, Ordering::SeqCst);
        true
    }
}
#[test]
fn repeated_preparation_does_not_reacquire_until_resume_refreshes() {
    let begins = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let acquires = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let acks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let (b, a, k) = (begins.clone(), acquires.clone(), acks.clone());
    let mut w = SessionWorker::start_with(move || {
        Ok(SleepResume {
            begins: b,
            acquires: a,
            acks: k,
        })
    })
    .unwrap();
    let mut generation = 0;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(e) = w.recv_timeout(Duration::from_millis(100)) {
            match e {
                SessionEvent::RefreshStarted { generation: g } => generation = g,
                SessionEvent::Snapshot {
                    generation: g,
                    snapshot,
                } if snapshot.requires_panel_off => {
                    generation = g;
                    if g == 1 {
                        w.acknowledge_panel_off(g);
                    }
                }
                SessionEvent::Snapshot {
                    generation: g,
                    snapshot,
                } if snapshot.allowed => {
                    generation = g;
                    break;
                }
                _ => {}
            }
        }
    }
    assert!(begins.load(Ordering::SeqCst) >= 4);
    assert_eq!(acquires.load(Ordering::SeqCst), 2);
    assert_eq!(acks.load(Ordering::SeqCst), 1);
    w.stop_after_panel_off(generation);
    assert!(w.stop_and_join(Duration::from_secs(1)));
}
