#[path = "../src/config.rs"]
#[allow(dead_code)]
mod config;

use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static N: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "touchmeter-config-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
    fn file(&self) -> PathBuf {
        self.0.join("config.json")
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run(p: &PathBuf, a: &[&str]) -> Result<String, String> {
    config::run_at(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>(), p)
}

#[test]
fn paths() {
    assert_eq!(
        config::config_path_from(Some(std::ffi::OsStr::new("/tmp/cfg")), None),
        PathBuf::from("/tmp/cfg/touchmeter/config.json")
    );
    assert_eq!(
        config::config_path_from(
            Some(std::ffi::OsStr::new("relative")),
            Some(std::ffi::OsStr::new("/home/u"))
        ),
        PathBuf::from("/home/u/.config/touchmeter/config.json")
    );
}
#[test]
fn defaults_and_cli_mutations() {
    let t = Temp::new();
    let p = t.file();
    assert_eq!(config::read(&p).unwrap().fire_trigger_minutes, 30);
    assert_eq!(
        run(&p, &["set", "fire.trigger-minutes", "60"])
            .unwrap()
            .contains("= 60"),
        true
    );
    assert_eq!(run(&p, &["get", "fire.trigger-minutes"]).unwrap(), "60\n");
    assert_eq!(run(&p, &["reset"]).unwrap().contains("= 30"), true);
}
#[test]
fn strict_validation_and_preservation() {
    let t = Temp::new();
    let p = t.file();
    for json in [
        "[]",
        "{}",
        "{\"version\":1,\"x\":2}",
        "{\"version\":1,\"fire\":null}",
        "{\"version\":1,\"fire\":{\"triggerMinutes\":30.5}}",
        "{\"version\":1,\"fire\":{\"triggerMinutes\":1441}}",
        "{\"version\":1,\"fire\":null}",
        "{\"version\":1,\"fire\":{\"triggerMinutes\":null}}",
        "{\"version\":1,\"fire\":[45]}",
        "{\"version\":1,\"fire\":[]}",
    ] {
        fs::write(&p, json).unwrap();
        assert!(config::read(&p).is_err(), "{json}");
        assert!(run(&p, &["set", "fire.trigger-minutes", "31"]).is_err());
        assert_eq!(fs::read_to_string(&p).unwrap(), json);
    }
}

#[test]
fn omitted_fire_and_integral_decimal_numbers_are_accepted() {
    let t = Temp::new();
    let p = t.file();
    fs::write(&p, "{\"version\":1}").unwrap();
    assert_eq!(config::read(&p).unwrap().fire_trigger_minutes, 30);
    fs::write(&p, "{\"version\":1.0,\"fire\":{\"triggerMinutes\":30.0}}").unwrap();
    assert_eq!(config::read(&p).unwrap().fire_trigger_minutes, 30);
    assert!(run(&p, &["set", "fire.trigger-minutes", "3e1"]).is_ok());
    assert!(run(&p, &["set", "fire.trigger-minutes", "30.5"]).is_err());
}

#[test]
fn binary_config_cli_is_headless_and_uses_xdg_directory() {
    let t = Temp::new();
    let binary = env!("CARGO_BIN_EXE_touchmeter");
    let invoke = |args: &[&str]| {
        std::process::Command::new(binary)
            .args(["config"])
            .args(args)
            .env("XDG_CONFIG_HOME", &t.0)
            .output()
            .unwrap()
    };
    let help = invoke(&["--help"]);
    assert!(help.status.success());
    let path = invoke(&["path"]);
    assert!(String::from_utf8_lossy(&path.stdout).contains("touchmeter/config.json"));
    assert!(invoke(&["show"]).status.success());
    assert!(invoke(&["get", "fire.trigger-minutes"]).status.success());
    assert!(
        invoke(&["set", "fire.trigger-minutes", "45"])
            .status
            .success()
    );
    assert!(invoke(&["reset"]).status.success());
}

#[test]
fn binary_defaults_and_test_state_are_independent() {
    let t = Temp::new();
    let config_home = t.0.join("config-home");
    let runtime = t.0.join("runtime");
    fs::create_dir(&runtime).unwrap();
    let binary = env!("CARGO_BIN_EXE_touchmeter");
    let invoke = |args: &[&str]| {
        std::process::Command::new(binary)
            .args(args)
            .env("XDG_CONFIG_HOME", &config_home)
            .env("XDG_RUNTIME_DIR", &runtime)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap()
    };
    let default = invoke(&["config", "get", "fire.trigger-minutes"]);
    assert!(default.status.success());
    assert_eq!(String::from_utf8(default.stdout).unwrap().trim(), "30");
    assert!(invoke(&["config", "show"]).status.success());
    assert!(!config_home.exists());
    assert!(
        invoke(&["config", "set", "fire.trigger-minutes", "60"])
            .status
            .success()
    );
    let original = fs::read(config_home.join("touchmeter/config.json")).unwrap();
    assert!(
        !invoke(&["config", "set", "fire.trigger-minutes", "0"])
            .status
            .success()
    );
    assert_eq!(
        fs::read(config_home.join("touchmeter/config.json")).unwrap(),
        original
    );
    assert!(invoke(&["test", "scenario", "fire"]).status.success());
    assert!(invoke(&["test", "clear"]).status.success());
    assert_eq!(
        fs::read(config_home.join("touchmeter/config.json")).unwrap(),
        original
    );
}

#[test]
fn backup_symlink_and_existing_lock() {
    let t = Temp::new();
    let p = t.file();
    let original = b"{\"version\":1,\"fire\":{\"triggerMinutes\":45}}";
    fs::write(&p, original).unwrap();
    run(&p, &["set", "fire.trigger-minutes", "46"]).unwrap();
    let backup = fs::read_dir(&t.0)
        .unwrap()
        .map(Result::unwrap)
        .find(|e| e.file_name().to_string_lossy().contains(".bak."))
        .unwrap();
    assert_eq!(fs::read(backup.path()).unwrap(), original);
    let lock = PathBuf::from(format!("{}.lock", p.display()));
    fs::write(&lock, "other process\n").unwrap();
    let before = fs::read(&p).unwrap();
    assert!(
        run(&p, &["set", "fire.trigger-minutes", "47"])
            .unwrap_err()
            .contains("being changed")
    );
    assert_eq!(fs::read(&p).unwrap(), before);
    fs::remove_file(lock).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let target = t.0.join("target");
        fs::write(&target, b"untouched").unwrap();
        fs::remove_file(&p).unwrap();
        symlink(&target, &p).unwrap();
        assert!(
            run(&p, &["set", "fire.trigger-minutes", "50"])
                .unwrap_err()
                .contains("symlink")
        );
        assert_eq!(fs::read(target).unwrap(), b"untouched");
    }
}
#[test]
fn size_and_private_write() {
    let t = Temp::new();
    let p = t.file();
    fs::write(&p, vec![b' '; 65537]).unwrap();
    assert!(config::read(&p).is_err());
    fs::remove_file(&p).unwrap();
    run(&p, &["set", "fire.trigger-minutes", "31"]).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&p).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
