use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
static N: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "touchmeter-preview-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
    fn run(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_touchmeter"))
            .args(args)
            .env("XDG_RUNTIME_DIR", &self.0)
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .output()
            .unwrap()
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn fixture_preview_emits_expected_png_without_touching_cli_state() {
    let t = Temp::new();
    assert!(t.run(&["test", "scenario", "signed-out"]).status.success());
    let before = fs::read(t.0.join("touchmeter-test.json")).unwrap();
    let output = t.0.join("preview.png");
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/docs/fixtures/claude-high.json"
    );
    let result = t.run(&["preview", output.to_str().unwrap(), "--fixture", fixture]);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let png = fs::read(output).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(
        (
            u32::from_be_bytes(png[16..20].try_into().unwrap()),
            u32::from_be_bytes(png[20..24].try_into().unwrap())
        ),
        (2008, 60)
    );
    assert_eq!(fs::read(t.0.join("touchmeter-test.json")).unwrap(), before);
    assert!(
        !t.run(&[
            "preview",
            t.0.join("bad.png").to_str().unwrap(),
            "--fixture",
            fixture,
            "--demo"
        ])
        .status
        .success()
    );
    assert_eq!(fs::read(t.0.join("touchmeter-test.json")).unwrap(), before);
}
