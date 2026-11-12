use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Env(PathBuf);
impl Env {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "touchmeter-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_touchmeter"))
            .args(args)
            .env("XDG_RUNTIME_DIR", &self.0)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap()
    }
    fn state(&self, sim: bool) -> PathBuf {
        self.0.join(if sim {
            "touchmeter-simulator-test.json"
        } else {
            "touchmeter-test.json"
        })
    }
    fn run_env(&self, args: &[&str], runtime: &std::path::Path) -> Output {
        Command::new(env!("CARGO_BIN_EXE_touchmeter"))
            .args(args)
            .env("XDG_RUNTIME_DIR", runtime)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap()
    }
    fn run_fallback(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_touchmeter"))
            .args(args)
            .env_remove("XDG_RUNTIME_DIR")
            .env("TMPDIR", &self.0)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap()
    }
}

#[test]
fn simulator_fallback_directory_requires_private_mode() {
    use std::os::unix::fs::PermissionsExt;
    let e = Env::new();
    let fallback =
        e.0.join(format!("touchmeter-{}", unsafe { libc::getuid() }));
    fs::create_dir(&fallback).unwrap();
    fs::set_permissions(&fallback, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        !e.run_fallback(&["test", "--simulator", "status"])
            .status
            .success()
    );
    fs::set_permissions(&fallback, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(
        e.run_fallback(&["test", "--simulator", "scenario", "fire"])
            .status
            .success()
    );
    assert_eq!(
        fs::metadata(fallback).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[test]
fn rejects_unsafe_runtime_symlinks_and_locked_updates() {
    use std::os::unix::fs::symlink;
    let e = Env::new();
    let unsafe_dir = e.0.join("unsafe");
    fs::create_dir(&unsafe_dir).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&unsafe_dir, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(
        !e.run_env(&["test", "scenario", "fire"], &unsafe_dir)
            .status
            .success()
    );
    assert!(e.run(&["test", "scenario", "fire"]).status.success());
    let state = e.state(false);
    let before = fs::read(&state).unwrap();
    let lock = PathBuf::from(format!("{}.lock", state.display()));
    fs::write(&lock, b"locked").unwrap();
    assert!(
        !e.run(&["test", "set", "claude", "--enabled"])
            .status
            .success()
    );
    assert_eq!(fs::read(&state).unwrap(), before);
    fs::remove_file(lock).unwrap();
    fs::remove_file(&state).unwrap();
    symlink("missing-target", &state).unwrap();
    assert!(!e.run(&["test", "status"]).status.success());
    assert!(!e.run(&["test", "clear"]).status.success());
    fs::remove_file(&state).unwrap();
    fs::create_dir(&state).unwrap();
    assert!(!e.run(&["test", "clear"]).status.success());
}

#[test]
fn state_permissions_and_bounded_load() {
    use std::os::unix::fs::PermissionsExt;
    let e = Env::new();
    assert!(e.run(&["test", "scenario", "fire"]).status.success());
    assert_eq!(
        fs::metadata(e.state(false)).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let big = e.0.join("big.json");
    fs::write(&big, vec![b' '; 65537]).unwrap();
    assert!(
        !e.run(&["test", "load", big.to_str().unwrap()])
            .status
            .success()
    );
}
impl Drop for Env {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn physical_and_simulator_state_are_independent_and_clearable() {
    let e = Env::new();
    assert!(e.run(&["test", "scenario", "fire"]).status.success());
    assert!(
        e.run(&["test", "--simulator", "scenario", "menu"])
            .status
            .success()
    );
    let physical = fs::read(&e.state(false)).unwrap();
    let sim = fs::read(&e.state(true)).unwrap();
    assert_ne!(physical, sim);
    let status = e.run(&["test", "status"]);
    assert!(status.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&status.stdout).unwrap()["mode"],
        "usage"
    );
    assert!(e.run(&["test", "--simulator", "clear"]).status.success());
    assert!(!e.state(true).exists());
    assert_eq!(fs::read(&e.state(false)).unwrap(), physical);
    assert!(e.run(&["test", "clear"]).status.success());
    assert!(!e.state(false).exists());
}

#[test]
fn invalid_set_preserves_bytes_and_emitted_fixture_roundtrips_without_nulls() {
    let e = Env::new();
    assert!(e.run(&["test", "scenario", "signed-out"]).status.success());
    let before = fs::read(e.state(false)).unwrap();
    assert!(
        !e.run(&["test", "set", "claude", "--short", "101"])
            .status
            .success()
    );
    assert_eq!(fs::read(e.state(false)).unwrap(), before);
    assert!(
        e.run(&["test", "set", "claude", "--enabled", "--short", "25"])
            .status
            .success()
    );
    let bytes = fs::read(e.state(false)).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    fn no_null(v: &serde_json::Value) {
        match v {
            serde_json::Value::Object(o) => {
                for x in o.values() {
                    assert!(!x.is_null());
                    no_null(x)
                }
            }
            serde_json::Value::Array(a) => {
                for x in a {
                    no_null(x)
                }
            }
            _ => (),
        }
    }
    no_null(&value);
    assert!(
        e.run(&["test", "load", e.state(false).to_str().unwrap()])
            .status
            .success()
    );
}

#[test]
fn clocks_and_updates_preserve_fixture_semantics() {
    let e = Env::new();
    assert!(e.run(&["test", "scenario", "fire"]).status.success());
    let physical: serde_json::Value =
        serde_json::from_slice(&fs::read(e.state(false)).unwrap()).unwrap();
    assert!(physical.get("now").is_none());
    assert!(
        e.run(&["test", "--simulator", "scenario", "fire"])
            .status
            .success()
    );
    let frozen: serde_json::Value =
        serde_json::from_slice(&fs::read(e.state(true)).unwrap()).unwrap();
    assert!(frozen.get("now").is_some());

    let fixture = e.0.join("moving.json");
    let mut moving = physical.clone();
    moving.as_object_mut().unwrap().remove("now");
    fs::write(&fixture, serde_json::to_vec(&moving).unwrap()).unwrap();
    assert!(
        e.run(&["test", "load", fixture.to_str().unwrap()])
            .status
            .success()
    );
    assert!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(e.state(false)).unwrap())
            .unwrap()
            .get("now")
            .is_none()
    );
    let fixed = "2026-11-10T12:00:00Z";
    let mut frozen_fixture = moving.clone();
    frozen_fixture["now"] = serde_json::Value::String(fixed.into());
    fs::write(&fixture, serde_json::to_vec(&frozen_fixture).unwrap()).unwrap();
    assert!(
        e.run(&["test", "load", fixture.to_str().unwrap()])
            .status
            .success()
    );
    assert!(
        e.run(&["test", "set", "claude", "--short", "77"])
            .status
            .success()
    );
    let changed: serde_json::Value =
        serde_json::from_slice(&fs::read(e.state(false)).unwrap()).unwrap();
    assert_eq!(changed["now"], fixed);
    let limits = changed["providers"]["claude"]["limits"].as_array().unwrap();
    assert_eq!(limits[0]["label"], "5h window");
    assert_eq!(limits[0]["percent"], 0.77);
    assert_eq!(limits[1]["label"], "Weekly (7-day)");
    assert_eq!(limits[0]["resetsAt"], "2026-11-10T14:00:00Z");
    // User-supplied fixture timestamps retain their original RFC3339 spelling.
    assert!(limits[1]["resetsAt"].as_str().unwrap().ends_with("+00:00"));
    assert_eq!(
        changed["providers"]["claude"]["limitsFetchedAt"],
        1794312000000.0
    );

    let mut with_other = changed.clone();
    with_other["providers"]
        .as_object_mut()
        .unwrap()
        .remove("codex");
    fs::write(&fixture, serde_json::to_vec(&with_other).unwrap()).unwrap();
    assert!(
        e.run(&["test", "load", fixture.to_str().unwrap()])
            .status
            .success()
    );
    assert!(
        e.run(&["test", "set", "codex", "--enabled"])
            .status
            .success()
    );
    let seeded: serde_json::Value =
        serde_json::from_slice(&fs::read(e.state(false)).unwrap()).unwrap();
    assert_eq!(
        seeded["providers"]["claude"],
        changed["providers"]["claude"]
    );
    assert_eq!(
        seeded["providers"]["codex"]["limits"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn fresh_set_keeps_physical_clock_moving_and_simulator_freezes_it() {
    let e = Env::new();
    assert!(
        e.run(&["test", "set", "claude", "--enabled"])
            .status
            .success()
    );
    assert!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(e.state(false)).unwrap())
            .unwrap()
            .get("now")
            .is_none()
    );
    assert!(
        e.run(&["test", "--simulator", "set", "claude", "--enabled"])
            .status
            .success()
    );
    assert!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(e.state(true)).unwrap())
            .unwrap()
            .get("now")
            .is_some()
    );
}
