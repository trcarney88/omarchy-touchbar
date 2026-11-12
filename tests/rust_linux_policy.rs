#[path = "../src/linux/input.rs"]
mod input;
#[path = "../src/jobs.rs"]
mod jobs;
#[path = "../src/linux/session.rs"]
mod session;
use input::*;
use session::*;
use std::time::{Duration, Instant};

#[test]
fn discovery_coordinates_and_multitouch_gating() {
    assert!(is_touchbar_device("MacBookPro18,3 Touch Bar", "apple_z2"));
    assert!(!is_touchbar_device("Other", "apple_z2"));
    let xr = AxisRange { min: 0, max: 100 };
    let yr = xr;
    assert_eq!(panel_point(0, 100, xr, yr), Some((0.0, 59.0)));
    assert_eq!(panel_point(0, 0, AxisRange { min: 1, max: 1 }, yr), None);
    let mut p = InputPolicy::new(xr, yr);
    p.event(RawEvent::Slot(0));
    p.event(RawEvent::Tracking(4));
    p.event(RawEvent::PositionX(50));
    p.event(RawEvent::PositionY(50));
    assert!(p.event(RawEvent::SyncReport).is_empty()); // initial all-up gate
    p.event(RawEvent::Tracking(-1));
    p.event(RawEvent::SyncReport);
    p.event(RawEvent::Tracking(5));
    assert!(matches!(
        p.event(RawEvent::SyncReport)[0],
        TouchEvent::Begin { .. }
    ));
    p.event(RawEvent::Slot(1));
    p.event(RawEvent::Tracking(6));
    p.event(RawEvent::Slot(0));
    p.event(RawEvent::Tracking(-1));
    assert_eq!(p.event(RawEvent::SyncReport), vec![TouchEvent::End]);
    p.event(RawEvent::Slot(1));
    p.event(RawEvent::Tracking(-1));
    p.event(RawEvent::SyncReport);
    assert_eq!(p.event(RawEvent::Resync), vec![TouchEvent::Cancel]);
    assert!(p.event(RawEvent::Disconnected).is_empty());
    p.event(RawEvent::Slot(0));
    p.event(RawEvent::Tracking(7));
    assert!(p.event(RawEvent::SyncReport).is_empty());
    p.event(RawEvent::Tracking(-1));
    p.event(RawEvent::SyncReport);
    p.event(RawEvent::Tracking(8));
    assert!(matches!(
        p.event(RawEvent::SyncReport)[0],
        TouchEvent::Begin { .. }
    ));
    p.event(RawEvent::Slot(1));
    p.event(RawEvent::Tracking(9));
    p.event(RawEvent::Slot(0));
    p.event(RawEvent::Tracking(-1));
    assert_eq!(p.event(RawEvent::SyncReport), vec![TouchEvent::End]);
    p.event(RawEvent::Slot(0));
    p.event(RawEvent::Tracking(10));
    assert!(p.event(RawEvent::SyncReport).is_empty());
    p.event(RawEvent::Slot(1));
    p.event(RawEvent::Tracking(-1));
    p.event(RawEvent::SyncReport);
    p.event(RawEvent::Slot(0));
    p.event(RawEvent::Tracking(-1));
    assert!(p.event(RawEvent::SyncReport).is_empty());
}

#[test]
fn session_is_fail_closed_fresh_and_sleep_ack_ordered() {
    let good = serde_json::json!({"Active":true,"LockedHint":false,"State":"active","Class":"user","Type":"wayland","User":[unsafe { libc::getuid() },"/user"]});
    assert!(permits(&good, false, false, Some(true), Some(true), true));
    for (k, v) in [
        ("Active", serde_json::json!(false)),
        ("LockedHint", serde_json::json!(true)),
        ("Type", serde_json::json!("x")),
        ("User", serde_json::Value::Null),
    ] {
        let mut p = good.clone();
        p[k] = v;
        assert!(!permits(&p, false, false, Some(true), Some(true), true));
    }
    assert!(!permits(&good, false, false, None, Some(true), true));
    assert!(!permits(&good, true, false, Some(true), Some(true), true));
    assert!(!permits(&good, false, true, Some(true), Some(true), true));
    assert!(!permits(&good, false, false, Some(true), Some(true), false));
    assert!(!permits_with_manager(
        &good,
        &serde_json::json!({"PreparingForSleep":true,"PreparingForShutdown":false}),
        Some(true),
        Some(true),
        true
    ));
    assert!(!permits_with_manager(
        &good,
        &serde_json::json!({"PreparingForSleep":false}),
        Some(true),
        Some(true),
        true
    ));
    assert_eq!(decode_hypr_locked(br#"false"#), Some(true));
    assert_eq!(decode_hypr_locked(br#"true"#), Some(false));
    assert_eq!(decode_hypr_locked(br#"{"locked":false}"#), Some(true));
    assert_eq!(decode_hypr_locked(br#"{"locked":true}"#), Some(false));
    assert!(!permits(
        &good,
        false,
        false,
        decode_hypr_locked(br#"true"#),
        Some(true),
        true
    ));
    assert!(permits(
        &good,
        false,
        false,
        decode_hypr_locked(br#"false"#),
        Some(true),
        true
    ));
    assert_eq!(decode_hypr_locked(br#"{}"#), None);
    assert_eq!(decode_hypr_locked(br#"{"locked":"false"}"#), None);
    assert_eq!(decode_hypr_locked(br#"null"#), None);
    assert_eq!(
        decode_monitors(br#"[{"enabled":true,"dpmsStatus":true}]"#),
        Some(true)
    );
    assert_eq!(decode_monitors(br#"[{"enabled":true}]"#), None);
    assert_eq!(
        decode_monitors(br#"[{"disabled":false,"dpmsStatus":true}]"#),
        Some(true)
    );
    assert_eq!(
        decode_monitors(br#"[{"enabled":true,"disabled":false,"dpmsStatus":true}]"#),
        Some(true)
    );
    assert_eq!(
        decode_monitors(br#"[{"enabled":true,"disabled":true,"dpmsStatus":true}]"#),
        None
    );
    assert_eq!(
        decode_monitors(br#"[{"disabled":false,"dpmsStatus":false}]"#),
        Some(false)
    );
    assert_eq!(
        decode_monitors(br#"[{"enabled":false,"dpmsStatus":false}]"#),
        Some(false)
    );
    assert_eq!(
        decode_monitors(
            br#"[{"disabled":true,"dpmsStatus":false},{"disabled":false,"dpmsStatus":true}]"#
        ),
        Some(true)
    );
    assert_eq!(
        decode_monitors(
            br#"[{"enabled":true,"dpmsStatus":true},{"enabled":true,"dpmsStatus":false}]"#
        ),
        Some(false)
    );
    assert_eq!(
        decode_monitors(br#"[{"disabled":"false","dpmsStatus":true}]"#),
        None
    );
    assert_eq!(
        decode_monitors(
            br#"[{"enabled":false,"dpmsStatus":false},{"enabled":"true","dpmsStatus":true}]"#
        ),
        None
    );
    assert_eq!(decode_monitors(br#"[{"enabled":true}]"#), None);
    assert_eq!(decode_monitors(br#"[]"#), None);
    assert!(!permits(
        &good,
        false,
        false,
        Some(true),
        Some(false),
        false
    ));
    let now = Instant::now();
    let c = Check::fresh(true, now);
    assert_eq!(c.get(now, Duration::from_secs(1)), Some(&true));
    assert_eq!(
        c.get(now + Duration::from_secs(2), Duration::from_secs(1)),
        None
    );
    let mut s = SleepCoordinator::default();
    assert_eq!(
        s.prepare(true),
        vec![
            SleepAction::ClearPermission,
            SleepAction::ReleaseKeys,
            SleepAction::StopAudio,
            SleepAction::PanelOff
        ]
    );
    assert!(s.prepare(true).is_empty());
    assert_eq!(
        s.acknowledge_panel_off(),
        vec![SleepAction::AcknowledgeSleep, SleepAction::CloseInhibitor]
    );
    assert!(s.acknowledge_panel_off().is_empty());
    assert_eq!(
        s.prepare(false),
        vec![SleepAction::ReacquireInhibitor, SleepAction::RefreshChecks]
    );
}
