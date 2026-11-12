//! State-file implementation for the legacy `touchmeter test` command.
use crate::fixtures::{scenario, Fixture, MAX_FIXTURE_BYTES, PROVIDER_IDS};
use chrono::{SecondsFormat, Utc};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
};

const HELP: &str = "Usage:\n  touchmeter test scenario <all-providers|high-usage|stale|missing|signed-out|fire|menu|volume|volume-muted|volume-unavailable>\n  touchmeter test load <fixture.json>\n  touchmeter test set <provider> [--enabled|--disabled] [--short 0..100] [--weekly 0..100] [--burn-minutes 0..1440|--no-burn] [--stale|--fresh] [--mode menu|usage|providers|functions|volume]\n  touchmeter test status\n  touchmeter test clear\nAdd --simulator anywhere to target simulator state.\n";
fn paths(sim: bool) -> Result<(PathBuf, PathBuf), String> {
    let fallback = sim && std::env::var_os("XDG_RUNTIME_DIR").is_none();
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            if sim {
                Some(std::env::temp_dir().join(format!("touchmeter-{}", unsafe { libc::getuid() })))
            } else {
                None
            }
        })
        .ok_or("A user-owned XDG_RUNTIME_DIR is required")?;
    if fallback && !dir.exists() {
        use std::os::unix::fs::DirBuilderExt;
        let mut b = fs::DirBuilder::new();
        b.mode(0o700);
        b.create(&dir).map_err(|e| e.to_string())?;
    }
    let m = fs::symlink_metadata(&dir).map_err(|e| e.to_string())?;
    if !m.is_dir()
        || m.file_type().is_symlink()
        || unsafe { libc::getuid() } != m.uid()
        || m.mode() & if fallback { 0o077 } else { 0o022 } != 0
    {
        return Err("Unsafe test state directory".into());
    }
    let file = dir.join(if sim {
        "touchmeter-simulator-test.json"
    } else {
        "touchmeter-test.json"
    });
    Ok((
        file.clone(),
        PathBuf::from(format!("{}.lock", file.display())),
    ))
}
use std::os::unix::fs::{MetadataExt, PermissionsExt};
fn read(path: &Path) -> Result<Fixture, String> {
    read_fixture(path)?.ok_or_else(|| "No test fixture".into())
}
pub(crate) fn runtime_path(sim: bool) -> Result<PathBuf, String> {
    paths(sim).map(|x| x.0)
}
pub(crate) fn read_fixture(path: &Path) -> Result<Option<Fixture>, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let f = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err("Refusing symlink at test state path".into());
            }
            return Ok(None);
        }
        Err(e) => return Err(e.to_string()),
    };
    let md = f.metadata().map_err(|e| e.to_string())?;
    if !md.is_file() {
        return Err("Refusing non-regular fixture".into());
    }
    let mut b = Vec::new();
    f.take((MAX_FIXTURE_BYTES + 1) as u64)
        .read_to_end(&mut b)
        .map_err(|e| e.to_string())?;
    if b.len() > MAX_FIXTURE_BYTES {
        return Err("Refusing overlarge fixture".into());
    }
    Fixture::decode(&b).map(Some)
}
fn acquire_lock(lock: &Path) -> Result<Lock, String> {
    let mut l = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock)
        .map_err(|_| "Test state is being changed by another process")?;
    let guard = Lock(lock.to_path_buf());
    use std::os::unix::fs::PermissionsExt;
    l.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())?;
    writeln!(l, "{}\ntouchmeter\n{}", std::process::id(), hostname()).map_err(|e| e.to_string())?;
    Ok(guard)
}
fn write(path: &Path, f: &Fixture) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(f).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FIXTURE_BYTES {
        return Err("Fixture exceeds 64 KiB".into());
    }
    let parent = path.parent().unwrap();
    let tmp = parent.join(format!(
        ".touchmeter-{}-{}.tmp",
        std::process::id(),
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    let mut o = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| e.to_string())?;
    let _tmp_guard = Lock(tmp.clone());
    o.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|e| e.to_string())?;
    o.write_all(&bytes)
        .and_then(|_| o.sync_all())
        .map_err(|e| e.to_string())?;
    drop(o);
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Refusing symlink at test state path".into());
    }
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(())
}
fn hostname() -> String {
    let mut b = [0 as libc::c_char; 256];
    let result = unsafe { libc::gethostname(b.as_mut_ptr(), b.len()) };
    if result != 0 {
        return "unknown".into();
    }
    bounded_hostname(&b)
}
fn utc_iso(time: chrono::DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, true)
}
fn bounded_hostname(bytes: &[libc::c_char]) -> String {
    let end = bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(bytes.as_ptr().cast(), end) })
        .into_owned()
}
#[cfg(test)]
mod tests {
    #[test]
    fn full_hostname_buffer_is_bounded() {
        let bytes = [b'x' as libc::c_char; 256];
        assert_eq!(super::bounded_hostname(&bytes), "x".repeat(256));
    }
}
struct Lock(PathBuf);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
pub fn run(mut a: Vec<String>) -> ExitCode {
    let n = a.iter().filter(|x| *x == "--simulator").count();
    if n > 1 {
        eprintln!("Use --simulator only once");
        return ExitCode::from(1);
    }
    a.retain(|x| x != "--simulator");
    let cmd = a.first().map(String::as_str).unwrap_or("");
    if ["", "--help", "help"].contains(&cmd) {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let sim = n == 1;
    let (path, lock) = match paths(sim) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    if cmd == "status" && a.len() == 1 {
        if fs::symlink_metadata(&path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
            println!("{{\"active\":false}}");
            return ExitCode::SUCCESS;
        }
        return match read(&path) {
            Ok(f) => {
                println!("{}", serde_json::to_string_pretty(&f).unwrap());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        };
    }
    if cmd == "clear" && a.len() == 1 {
        let _g = match acquire_lock(&lock) {
            Ok(g) => g,
            Err(_) => {
                eprintln!("Test state is being changed by another process");
                return ExitCode::from(1);
            }
        };
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            eprintln!("Refusing symlink at test state path");
            return ExitCode::from(1);
        }
        if let Err(e) = fs::remove_file(&path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("{e}");
                return ExitCode::from(1);
            }
        }
        println!(
            "{}",
            if sim {
                "Simulator overrides cleared"
            } else {
                "Live mode restored"
            }
        );
        return ExitCode::SUCCESS;
    }
    let _guard = match acquire_lock(&lock) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let result = (|| -> Result<Fixture, String> {
        let mut f = match cmd {
            "scenario" if a.len() == 2 => {
                let now = Utc::now();
                scenario(&a[1], now).map(|mut f| {
                    if sim {
                        f.now = Some(utc_iso(now));
                    } else {
                        f.now = None;
                    }
                    f
                })
            }
            "load" if a.len() == 2 => read(Path::new(&a[1])),
            "set" if a.len() >= 2 => {
                let id = &a[1];
                if !PROVIDER_IDS.contains(&id.as_str()) {
                    return Err("Unknown provider".into());
                }
                let mut f = if let Some(f) = read_fixture(&path)? {
                    f
                } else {
                    let mut f = scenario("signed-out", Utc::now())?;
                    if !sim {
                        f.now = None;
                    }
                    f
                };
                if !f.providers.contains_key(id) {
                    let seed = scenario(
                        "signed-out",
                        f.now
                            .as_deref()
                            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                            .map(|t| t.with_timezone(&Utc))
                            .unwrap_or_else(Utc::now),
                    )?;
                    f.providers.insert(id.clone(), seed.providers[id].clone());
                }
                let now = f
                    .now
                    .as_deref()
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                    .map(|t| t.with_timezone(&Utc))
                    .unwrap_or_else(Utc::now);
                if sim && f.now.is_none() {
                    f.now = Some(utc_iso(now));
                }
                let p = f.providers.get_mut(id).ok_or("Unknown provider")?;
                let mut i = 2;
                while i < a.len() {
                    match a[i].as_str() {
                        "--enabled" | "--disabled" => {
                            p.enabled = a[i] == "--enabled";
                            p.ready = p.enabled
                        }
                        "--stale" | "--fresh" => p.limits_stale = a[i] == "--stale",
                        "--no-burn" => p.burn_minutes = None,
                        "--burn-minutes" | "--short" | "--weekly" => {
                            let key = a[i].as_str();
                            i += 1;
                            let v: f64 = a
                                .get(i)
                                .ok_or("Missing value")?
                                .parse()
                                .map_err(|_| "Invalid numeric value")?;
                            if !v.is_finite()
                                || v < 0.
                                || v > if key == "--burn-minutes" { 1440. } else { 100. }
                            {
                                return Err("Value out of range".into());
                            }
                            if key == "--burn-minutes" {
                                p.burn_minutes = Some(v)
                            } else {
                                let label = if key == "--short" {
                                    "5h window"
                                } else {
                                    "Weekly (7-day)"
                                };
                                let limit = crate::fixtures::LimitFixture {
                                    label: label.into(),
                                    percent: v / 100.,
                                    resets_at: utc_iso(
                                        now + chrono::Duration::hours(if key == "--short" {
                                            2
                                        } else {
                                            96
                                        }),
                                    ),
                                };
                                if let Some(existing) =
                                    p.limits.iter_mut().find(|l| l.label == label)
                                {
                                    *existing = limit;
                                } else {
                                    p.limits.push(limit);
                                }
                            }
                        }
                        "--mode" => {
                            i += 1;
                            f.mode = a.get(i).ok_or("Missing mode")?.clone()
                        }
                        x => return Err(format!("Unknown/incomplete option: {x}")),
                    }
                    i += 1
                }
                p.limits_fetched_at = Some(now.timestamp_millis() as f64);
                f.selected = id.clone();
                if !a.contains(&"--mode".into()) {
                    f.mode = "usage".into()
                }
                Ok(f)
            }
            _ => Err("Unknown command/arguments. Use touchmeter test --help".into()),
        }?;
        if sim && cmd != "load" && f.now.is_none() {
            f.now = Some(utc_iso(Utc::now()))
        }
        f.validate()?;
        Ok(f)
    })();
    match result {
        Ok(f) => match write(&path, &f) {
            Ok(()) => {
                println!("Test state updated");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::from(1)
            }
        },
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(1)
        }
    }
}
