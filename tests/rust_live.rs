#[path = "../src/burnrate.rs"]
mod burnrate;
#[path = "../src/jobs.rs"]
mod jobs;
#[path = "../src/live.rs"]
mod live;
#[path = "../src/live_worker.rs"]
mod live_worker;
use chrono::{DateTime, Utc};

#[test]
fn parser_bounds_and_preserves_multiple_windows() {
    let now = DateTime::parse_from_rfc3339("2026-11-11T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let bytes=br#"{"ready":true,"limitsFetchedAt":1794408000000,"limits":[{"label":"5h window","percent":0.5,"resetsAt":"2026-11-11T14:00:00Z"},{"label":"Weekly (7-day)","percent":0.2,"resetsAt":""},{"label":"<bad>","percent":0.1,"resetsAt":""}]}"#;
    let (w, _, _, _, _) = live::parse_record(bytes, now).unwrap();
    assert_eq!(w.len(), 2);
    assert_eq!(w[0].label, "5 HOUR");
    assert_eq!(w[1].countdown, "reset unknown");
    assert!(live::parse_record(&vec![b'x'; 1024 * 1024 + 1], now).is_err());
}

#[test]
fn burn_rate_requires_distinct_observations_and_isolated_accounts() {
    let mut b = burnrate::BurnRate::default();
    let t = 1_800_000_000_000i64;
    let reset = DateTime::from_timestamp_millis(t + 86_400_000).unwrap();
    let w = |f| burnrate::BurnWindow {
        window: "5h".into(),
        fraction: f,
        resets_at: Some(reset),
    };
    assert!(b.observe("codex", "a", &[w(0.2)], t, t, true).is_none());
    assert!(
        b.observe("codex", "a", &[w(0.3)], t, t + 60_000, true)
            .is_none()
    ); // duplicate fetched timestamp
    assert!(
        b.observe("codex", "b", &[w(0.4)], t + 120_000, t + 120_000, true)
            .is_none()
    );
    assert!(
        b.observe("codex", "b", &[w(0.5)], t + 180_000, t + 180_000, true)
            .is_some()
    );
}

#[test]
fn selection_changes_generation_and_test_mode_clears_history() {
    let mut x = live::LiveBackend::new(
        "/tmp/no-live-data".into(),
        [("codex".into(), true), ("claude".into(), true)].into(),
    );
    let r = x.refresh_request().unwrap();
    assert!(!r.argv.is_empty());
    assert!(x.select("claude"));
    x.complete(r.generation, true);
    assert!(!x.select("unknown"));
    x.set_test_mode(true);
    assert!(x.poll(Utc::now()).burn_minutes.is_none());
}

#[test]
fn rate_math_refill_suppression_and_provider_isolation() {
    let mut b = burnrate::BurnRate::default();
    let t = 1_800_000_000_000i64;
    let reset = DateTime::from_timestamp_millis(t + 86_400_000).unwrap();
    let w = |fraction| burnrate::BurnWindow {
        window: "daily".into(),
        fraction,
        resets_at: Some(reset),
    };
    assert_eq!(b.observe("codex", "a", &[w(0.1)], t, t, true), None);
    let estimate = b
        .observe("codex", "a", &[w(0.2)], t + 60_000, t + 60_000, true)
        .unwrap();
    assert!((estimate - 8.0).abs() < 0.001); // Remaining .8 at .1/minute.
    assert_eq!(
        b.observe("claude", "a", &[w(0.4)], t + 120_000, t + 120_000, true),
        None
    );

    let soon_reset = DateTime::from_timestamp_millis(t + 300_000).unwrap();
    let short = burnrate::BurnWindow {
        window: "session".into(),
        fraction: 0.5,
        resets_at: Some(soon_reset),
    };
    assert_eq!(b.observe("grok", "a", &[short.clone()], t, t, true), None);
    let short = burnrate::BurnWindow {
        fraction: 0.6,
        ..short
    };
    assert_eq!(
        b.observe("grok", "a", &[short], t + 60_000, t + 60_000, true),
        None
    ); // Refill precedes depletion.
}

#[test]
fn parser_rejects_stale_and_invalid_records_without_losing_other_valid_windows() {
    let now = DateTime::parse_from_rfc3339("2026-11-11T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc);
    let stale = br#"{"ready":true,"limitsFetchedAt":1794400000000,"limitsStale":true,"limits":[{"label":"A","percent":2,"resetsAt":""},{"label":"B","percent":0.4,"resetsAt":"2026-11-11T13:00:00Z"}]}"#;
    let (w, _, _, is_stale, _) = live::parse_record(stale, now).unwrap();
    assert!(is_stale);
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].window, "B");
    assert!(live::parse_record(br#"{"ready":true,"limits":[]}"#, now).is_err());
}

#[test]
fn disabled_selection_and_old_generation_completion_are_inert() {
    let mut x = live::LiveBackend::new(
        "/tmp/no-live-data".into(),
        [("codex".into(), true), ("claude".into(), true)].into(),
    );
    let old = x.refresh_request().unwrap();
    assert!(x.select("claude"));
    let current = x.refresh_request().unwrap();
    x.complete(old.generation, true);
    let snap = x.poll(Utc::now());
    assert_eq!(snap.provider, "claude");
    assert_eq!(snap.status, "Usage data unavailable");
    x.complete(current.generation, true);
    assert!(x.poll(Utc::now()).stale);
    assert!(!x.select("cursor"));
}

#[test]
fn test_mode_suppresses_requests_and_restores_selection() {
    let mut b = live::LiveBackend::new(
        "/tmp/no-live-data".into(),
        [("codex".into(), true), ("claude".into(), true)].into(),
    );
    assert!(b.select("claude"));
    b.set_test_mode(true);
    assert!(b.refresh_request().is_none());
    b.set_test_mode(false);
    assert_eq!(b.selected, "claude");
}

#[test]
fn worker_is_explicit_and_executes_only_the_fake_program() {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        thread,
        time::{Duration, Instant},
    };
    let dir = std::env::temp_dir().join(format!("touchmeter-live-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("invoked");
    let script = dir.join("collector");
    fs::write(
        &script,
        format!("#!/bin/sh\nprintf '%s' \"$*\" > '{}'\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let mut b = live::LiveBackend::new(dir.clone(), [("codex".into(), true)].into());
    let mut worker = live_worker::LiveWorker::new(&b, &script);
    assert!(!marker.exists());
    worker.resume();
    worker.tick(&mut b, Instant::now(), Utc::now());
    for _ in 0..100 {
        if marker.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(
        fs::read_to_string(&marker).unwrap(),
        "agent usage update --limits-only codex"
    );
    assert_eq!(
        live_worker::LiveWorker::intervals(),
        (Duration::from_secs(300), Duration::from_secs(15))
    );
    worker.set_test_mode(&mut b, true);
    assert!(b.refresh_request().is_none());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn model_projection_preserves_unrelated_controls_and_applies_fire_threshold() {
    let mut model = touchmeter_ui::Model::default();
    model.volume = 0.7;
    model.selected = 0;
    model.fire_trigger_minutes = 99.0;
    let snap = live::LiveSnapshot {
        provider: "codex".into(),
        account: "acct".into(),
        enabled: true,
        windows: vec![live::LiveWindow {
            window: "5h window".into(),
            label: "5 HOUR".into(),
            fraction: 0.8,
            resets_at: None,
            countdown: "reset unknown".into(),
            expired: false,
        }],
        status: "LIVE".into(),
        stale: false,
        burn_minutes: Some(20.0),
        limits_fetched_at_ms: 1,
    };
    snap.apply_to_model(&mut model, 30);
    assert_eq!(model.volume, 0.7);
    assert!(model.fire);
    assert_eq!(model.fire_trigger_minutes, 30.0);
    assert_eq!(model.limits[0].label, "5 HOUR");
}

#[test]
fn credential_probe_cadence_is_a_request_only_and_pauses_in_test_mode() {
    use std::time::{Duration, Instant};
    let b = live::LiveBackend::new("/tmp/no-live-data".into(), std::collections::HashMap::new());
    let mut w = live_worker::LiveWorker::new(&b, "/unused");
    let t = Instant::now();
    assert!(w.credential_request_due(t, false));
    assert!(!w.credential_request_due(t + Duration::from_secs(14), false));
    assert!(!w.credential_request_due(t + Duration::from_secs(15), true));
    assert!(w.credential_request_due(t + Duration::from_secs(15), false));
}
