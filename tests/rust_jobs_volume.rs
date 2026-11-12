#[path = "../src/jobs.rs"]
mod jobs;
#[path = "../src/linux/volume.rs"]
mod volume;

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    thread,
    time::{Duration, Instant},
};

fn script(body: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let p = std::env::temp_dir().join(format!(
        "touchmeter-job-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o700)).unwrap();
    p.to_string_lossy().into_owned()
}
fn wait(j: &mut jobs::Job) -> jobs::JobResult {
    let end = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(r) = j.poll() {
            return r;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(10));
    }
}
fn poll_volume(v: &mut volume::VolumeAdapter, from: Duration, until: Duration) {
    let end = Instant::now() + Duration::from_secs(2);
    let mut t = from;
    while t <= until {
        v.tick(t);
        if v.state.available {
            return;
        }
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(5));
        t += Duration::from_millis(10);
    }
}

#[test]
fn wpctl_parser_and_argv_are_strict() {
    assert_eq!(
        volume::parse_volume(b"Volume: 1.25 [MUTED]\n"),
        Some((1.0, true))
    );
    assert_eq!(volume::parse_volume(b"Volume: 0.45"), Some((0.45, false)));
    assert_eq!(volume::parse_volume(b"Volume: nope"), None);
    assert_eq!(volume::parse_volume(&vec![b'x'; 257]), None);
    let (p, a) = volume::command("fake", volume::Operation::SetVolume(0.45678));
    assert_eq!(p, "fake");
    assert_eq!(
        a,
        [
            "set-volume",
            "--limit",
            "1.0",
            "@DEFAULT_AUDIO_SINK@",
            "0.457"
        ]
    );
}
#[test]
fn jobs_bound_output_timeout_status_and_cancel() {
    let p = script("printf ok; printf bad >&2");
    let mut j = jobs::Job::start(
        &p,
        std::iter::empty::<&str>(),
        std::iter::empty::<(&str, &str)>(),
        Duration::from_secs(1),
        32,
        7,
    )
    .unwrap();
    let r = wait(&mut j);
    assert_eq!(r.stdout, b"ok");
    assert_eq!(r.stderr, b"bad");
    assert_eq!(r.status, Some(0));
    fs::remove_file(p).unwrap();
    let p = script("head -c 13000 /dev/zero");
    let mut j = jobs::Job::start(
        &p,
        std::iter::empty::<&str>(),
        std::iter::empty::<(&str, &str)>(),
        Duration::from_secs(1),
        14000,
        70,
    )
    .unwrap();
    let r = wait(&mut j);
    assert_eq!(r.stdout.len(), 13000);
    assert!(r.error.is_none());
    fs::remove_file(p).unwrap();
    let p = script("head -c 3000 /dev/zero; head -c 3000 /dev/zero >&2");
    let mut j = jobs::Job::start(
        &p,
        std::iter::empty::<&str>(),
        std::iter::empty::<(&str, &str)>(),
        Duration::from_secs(1),
        5000,
        71,
    )
    .unwrap();
    assert_eq!(wait(&mut j).error, Some(jobs::JobError::OutputLimit));
    fs::remove_file(p).unwrap();
    let mut j = jobs::Job::start(
        "/definitely/missing/touchmeter-program",
        std::iter::empty::<&str>(),
        std::iter::empty::<(&str, &str)>(),
        Duration::from_secs(1),
        10,
        72,
    )
    .unwrap();
    assert_eq!(wait(&mut j).error, Some(jobs::JobError::Spawn));
    let p = script("sleep 5");
    let mut j = jobs::Job::start(
        &p,
        std::iter::empty::<&str>(),
        std::iter::empty::<(&str, &str)>(),
        Duration::from_millis(40),
        10,
        8,
    )
    .unwrap();
    assert_eq!(wait(&mut j).error, Some(jobs::JobError::Timeout));
    fs::remove_file(p).unwrap();
    let p = script("printf '01234567890123456789'");
    let mut j = jobs::Job::start(
        &p,
        std::iter::empty::<&str>(),
        std::iter::empty::<(&str, &str)>(),
        Duration::from_secs(1),
        4,
        9,
    )
    .unwrap();
    assert_eq!(wait(&mut j).error, Some(jobs::JobError::OutputLimit));
    fs::remove_file(p).unwrap();
    let p = script("sleep 5");
    let mut j = jobs::Job::start(
        &p,
        std::iter::empty::<&str>(),
        std::iter::empty::<(&str, &str)>(),
        Duration::from_secs(10),
        10,
        10,
    )
    .unwrap();
    thread::sleep(Duration::from_millis(30));
    j.cancel();
    assert_eq!(wait(&mut j).error, Some(jobs::JobError::Cancelled));
    fs::remove_file(p).unwrap();
}

#[test]
fn cancellation_before_spawn_is_authoritative_and_reap_is_bounded() {
    let marker = std::env::temp_dir().join(format!("touchmeter-marker-{}", std::process::id()));
    let _ = fs::remove_file(&marker);
    let p = script(&format!("touch '{}' ; sleep 5", marker.display()));
    let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
    let release = std::sync::Arc::new(std::sync::Barrier::new(2));
    let (a, b) = (entered.clone(), release.clone());
    let mut j = jobs::Job::start_with_pre_spawn_hook(
        &p,
        std::iter::empty::<&str>(),
        std::iter::empty::<(&str, &str)>(),
        Duration::from_secs(2),
        32,
        88,
        move || {
            a.wait();
            b.wait();
        },
    )
    .unwrap();
    entered.wait();
    j.cancel();
    release.wait();
    assert_eq!(wait(&mut j).error, Some(jobs::JobError::Cancelled));
    assert!(!marker.exists());
    fs::remove_file(p).unwrap();
    let p = script(&format!("touch '{}' ; sleep 5", marker.display()));
    let entered = std::sync::Arc::new(std::sync::Barrier::new(2));
    let release = std::sync::Arc::new(std::sync::Barrier::new(2));
    let (a, b) = (entered.clone(), release.clone());
    let mut j = jobs::Job::start_with_hooks(
        &p,
        std::iter::empty::<&str>(),
        std::iter::empty::<(&str, &str)>(),
        Duration::from_secs(2),
        32,
        90,
        || {},
        move || {
            a.wait();
            b.wait();
        },
    )
    .unwrap();
    entered.wait();
    let begin = Instant::now();
    j.cancel();
    assert!(begin.elapsed() < Duration::from_millis(100));
    release.wait();
    assert_eq!(wait(&mut j).error, Some(jobs::JobError::Cancelled));
    fs::remove_file(p).unwrap();
    let p = script("sleep 5");
    let mut j = jobs::Job::start(
        &p,
        std::iter::empty::<&str>(),
        std::iter::empty::<(&str, &str)>(),
        Duration::from_secs(5),
        20,
        89,
    )
    .unwrap();
    let end = Instant::now() + Duration::from_secs(1);
    while Instant::now() < end {
        if j.cancel_and_wait(Duration::from_millis(100)).is_some() {
            break;
        }
    }
    assert!(Instant::now() < end);
    fs::remove_file(p).unwrap();
}

#[test]
fn deactivation_cancels_running_fake_volume_job_and_drops_queued_write() {
    let log =
        std::env::temp_dir().join(format!("touchmeter-deactivate-log-{}", std::process::id()));
    let _ = fs::remove_file(&log);
    let p = script(&format!("echo \"$*\" >> '{}' ; sleep 5", log.display()));
    let mut v = volume::VolumeAdapter::new(&p);
    v.set_active(true);
    v.tick(Duration::from_secs(1));
    let end = Instant::now() + Duration::from_secs(1);
    while !log.exists() {
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(5));
    }
    let start = Instant::now();
    v.set_active(false);
    assert!(start.elapsed() < Duration::from_millis(500));
    v.tick(Duration::from_secs(2));
    thread::sleep(Duration::from_millis(30));
    assert_eq!(fs::read_to_string(&log).unwrap().lines().count(), 1);
    fs::remove_file(log).unwrap();
    fs::remove_file(p).unwrap();
}

#[test]
fn deactivate_now_returns_immediately_and_retains_unfinished_job() {
    let log = std::env::temp_dir().join(format!("touchmeter-now-log-{}", std::process::id()));
    let _ = fs::remove_file(&log);
    let p = script(&format!("echo \"$*\" >> '{}' ; sleep 5", log.display()));
    let mut v = volume::VolumeAdapter::new(&p);
    v.set_active(true);
    v.tick(Duration::from_secs(1));
    let end = Instant::now() + Duration::from_secs(1);
    while !log.exists() {
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(5));
    }
    let start = Instant::now();
    v.deactivate_now();
    assert!(start.elapsed() < Duration::from_millis(100));
    assert!(!v.finish_cancel(Duration::ZERO));
    v.tick(Duration::from_secs(2));
    thread::sleep(Duration::from_millis(30));
    assert_eq!(fs::read_to_string(&log).unwrap().lines().count(), 1);
    assert!(v.finish_cancel(Duration::from_secs(1)));
    fs::remove_file(log).unwrap();
    fs::remove_file(p).unwrap();
}
#[test]
fn volume_stays_inert_until_active_and_cancels_on_deactivation() {
    let mut v = volume::VolumeAdapter::new("/no/such/wpctl");
    v.tick(Duration::from_secs(3));
    assert!(!v.state.available);
    v.set_active(true);
    v.tick(Duration::from_secs(3));
    assert!(!v.state.available);
    v.set_active(false);
    assert!(!v.state.available);
}

#[test]
fn fake_wpctl_coalesces_and_orders_volume_before_mute() {
    let log = std::env::temp_dir().join(format!("touchmeter-wpctl-log-{}", std::process::id()));
    let _ = fs::remove_file(&log);
    let p = script(&format!(
        "echo \"$*\" >> '{}' ; if [ \"$1\" = get-volume ]; then echo 'Volume: 0.4'; fi; sleep 0.03",
        log.display()
    ));
    let mut v = volume::VolumeAdapter::new(&p);
    v.set_active(true);
    poll_volume(&mut v, Duration::from_secs(1), Duration::from_secs(2));
    assert!(v.state.available);
    let t = Duration::from_secs(3);
    v.tick(t);
    v.set_volume(0.2);
    v.set_volume(0.7);
    v.toggle_mute();
    v.tick(t);
    v.tick(t + Duration::from_millis(39));
    assert_eq!(fs::read_to_string(&log).unwrap().lines().count(), 1);
    v.tick(t + Duration::from_millis(40));
    let end = Instant::now() + Duration::from_secs(2);
    while Instant::now() < end {
        v.tick(t + Duration::from_millis(50));
        if fs::read_to_string(&log).unwrap().contains("set-mute") {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    let lines = fs::read_to_string(&log).unwrap();
    let ops: Vec<_> = lines.lines().collect();
    let volume_at = ops
        .iter()
        .position(|s| s.starts_with("set-volume"))
        .unwrap();
    let mute_at = ops.iter().position(|s| s.starts_with("set-mute")).unwrap();
    assert_eq!(
        ops[volume_at],
        "set-volume --limit 1.0 @DEFAULT_AUDIO_SINK@ 0.700"
    );
    assert_eq!(ops[mute_at], "set-mute @DEFAULT_AUDIO_SINK@ 1");
    assert!(volume_at < mute_at);
    v.set_active(false);
    let n = ops.len();
    v.tick(t + Duration::from_secs(1));
    assert_eq!(fs::read_to_string(&log).unwrap().lines().count(), n);
    fs::remove_file(log).unwrap();
    fs::remove_file(p).unwrap();
}
