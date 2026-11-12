use std::{
    env, fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_CONFIG: u64 = 65_536;
const KEY: &str = "fire.trigger-minutes";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub fire_trigger_minutes: u16,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            fire_trigger_minutes: 30,
        }
    }
}

pub fn path() -> PathBuf {
    config_path_from(
        env::var_os("XDG_CONFIG_HOME").as_deref(),
        env::var_os("HOME").as_deref(),
    )
}
pub fn config_path_from(xdg: Option<&std::ffi::OsStr>, home: Option<&std::ffi::OsStr>) -> PathBuf {
    let base = xdg
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.map(PathBuf::from).map(|p| p.join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    base.join("touchmeter/config.json")
}

pub fn read(path: impl AsRef<Path>) -> Result<Config, String> {
    load(path.as_ref())
}
fn load(path: &Path) -> Result<Config, String> {
    #[cfg(unix)]
    let opened = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
    };
    #[cfg(not(unix))]
    let opened = fs::File::open(path);
    let f = match opened {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => return Err(format!("Cannot read config: {e}")),
    };
    if !f
        .metadata()
        .map_err(|e| format!("Cannot read config: {e}"))?
        .is_file()
    {
        return Err("Config is not a regular file".into());
    }
    let mut bytes = Vec::new();
    f.take(MAX_CONFIG + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read config: {e}"))?;
    if bytes.len() as u64 > MAX_CONFIG {
        return Err("Config exceeds 64 KiB".into());
    }
    let v: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "Config must be a JSON object".to_string())?;
    if !v.is_object() {
        return Err("Config must be a JSON object".into());
    }
    let object = v.as_object().unwrap();
    if object.keys().any(|k| k != "version" && k != "fire") {
        return Err("Unknown config section".into());
    }
    let version = v
        .get("version")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(f64::NAN);
    if !version.is_finite() || version != 1.0 {
        return Err("Config version must be 1".into());
    }
    let fire = match v.get("fire") {
        None => None,
        Some(x) => Some(x.as_object().ok_or("fire must be an object")?),
    };
    let minutes = if let Some(fire) = fire {
        if fire.keys().any(|k| k != "triggerMinutes") {
            return Err("Unknown fire setting".into());
        }
        match fire.get("triggerMinutes") {
            None => 30,
            Some(x) => x
                .as_f64()
                .filter(|n| n.is_finite() && n.fract() == 0.0 && (1.0..=1440.0).contains(n))
                .map(|n| n as u16)
                .ok_or_else(|| format!("{KEY} must be an integer from 1 to 1440"))?,
        }
    } else {
        30
    };
    Ok(Config {
        fire_trigger_minutes: minutes,
    })
}

pub fn run_cli(args: &[String]) -> Result<String, String> {
    run_at(args, path())
}
pub fn run_at(args: &[String], path: impl AsRef<Path>) -> Result<String, String> {
    let p = path.as_ref();
    let usage = "Usage:\n  touchmeter config show\n  touchmeter config get <key>\n  touchmeter config set <key> <value>\n  touchmeter config reset [key]\n  touchmeter config path\n\nSettings:\n  fire.trigger-minutes   Projected minutes to quota exhaustion that trigger flames\n                         Integer 1–1440; default 30. Larger = earlier warning.\nChanges persist and apply live, including in test mode. No restart or administrator access needed.\n";
    let cmd = args.first().map(String::as_str).unwrap_or("");
    if cmd.is_empty() || cmd == "help" || cmd == "--help" {
        return Ok(usage.into());
    }
    if cmd == "path" && args.len() == 1 {
        return Ok(format!("{}\n", p.display()));
    }
    let (mode, key) = match (cmd, args.len()) {
        ("show", 1) => ("show", None),
        ("get", 2) => ("get", Some(args[1].as_str())),
        ("set", 3) => ("set", Some(args[1].as_str())),
        ("reset", 1) => ("reset", None),
        ("reset", 2) => ("reset", Some(args[1].as_str())),
        _ => return Err("Unknown command/arguments. Use touchmeter config --help".into()),
    };
    if let Some(k) = key {
        if k != KEY {
            return Err(format!("Unknown setting: {k}"));
        }
    }
    let requested = if mode == "set" {
        parse_minutes(&args[2])?
    } else {
        30
    };
    if mode == "show" || mode == "get" {
        let c = load(p)?;
        return Ok(if mode == "get" {
            format!("{}\n", c.fire_trigger_minutes)
        } else {
            format!("{{\n  \"{KEY}\": {}\n}}\n", c.fire_trigger_minutes)
        });
    }
    save(p, requested)?;
    Ok(format!(
        "{KEY} = {requested} (saved; running app updates within 0.5s)\n"
    ))
}
fn parse_minutes(s: &str) -> Result<u16, String> {
    let n = s
        .parse::<f64>()
        .ok()
        .filter(|n| n.is_finite() && n.fract() == 0.0 && (1.0..=1440.0).contains(n));
    n.map(|n| n as u16)
        .ok_or_else(|| format!("{KEY} must be an integer from 1 to 1440"))
}
fn save(path: &Path, minutes: u16) -> Result<(), String> {
    let dir = path.parent().ok_or("Cannot create config directory")?;
    fs::create_dir_all(dir).map_err(|_| "Cannot create config directory".to_string())?;
    let lock_path = PathBuf::from(format!("{}.lock", path.display()));
    let mut lock = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
    {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            if !remove_proven_stale_lock(&lock_path) {
                return Err("Config is being changed by another process".into());
            }
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
                .map_err(|_| "Config is being changed by another process".to_string())?
        }
        Err(_) => return Err("Config is being changed by another process".into()),
    };
    let _guard = LockGuard { path: lock_path };
    writeln!(lock, "{}\ntouchmeter\n{}", std::process::id(), hostname())
        .and_then(|_| lock.sync_all())
        .map_err(|_| "Cannot create config lock".to_string())?;
    let meta = fs::symlink_metadata(path);
    if meta.as_ref().is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Refusing to overwrite a symlinked config".into());
    }
    let old = load(path).map_err(|e| format!("{e}; config left unchanged"))?;
    let existing = path.exists();
    if existing {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let backup = PathBuf::from(format!(
            "{}.bak.{}.{}",
            path.display(),
            ts,
            std::process::id()
        ));
        fs::copy(path, backup)
            .map_err(|_| "Cannot back up existing config; unchanged".to_string())?;
    }
    let _ = old;
    let data = format!(
        "{{\n  \"version\": 1,\n  \"fire\": {{\n    \"triggerMinutes\": {minutes}\n  }}\n}}\n"
    );
    let tmp = PathBuf::from(format!(
        "{}.tmp.{}.{}",
        path.display(),
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(&tmp)
        .map_err(|_| "Cannot atomically save config".to_string())?;
    let mut cleanup = TempGuard {
        path: tmp.clone(),
        active: true,
    };
    f.write_all(data.as_bytes())
        .and_then(|_| f.sync_all())
        .map_err(|_| "Cannot atomically save config".to_string())?;
    fs::rename(&tmp, path).map_err(|_| "Cannot atomically save config".to_string())?;
    cleanup.active = false;
    Ok(())
}
#[cfg(unix)]
fn remove_proven_stale_lock(path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.uid() != unsafe { libc::geteuid() } {
        return false;
    }
    let Ok(data) = fs::read(path) else {
        return false;
    };
    let Ok(text) = std::str::from_utf8(&data) else {
        return false;
    };
    let mut lines = text.lines();
    let Some(pid) = lines
        .next()
        .and_then(|s| s.parse::<libc::pid_t>().ok())
        .filter(|p| *p > 0)
    else {
        return false;
    };
    if lines.next() != Some("touchmeter")
        || lines.next() != Some(hostname().as_str())
        || lines.next().is_some()
    {
        return false;
    }
    if unsafe { libc::kill(pid, 0) } == 0
        || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    {
        return false;
    }
    let Ok(meta2) = fs::symlink_metadata(path) else {
        return false;
    };
    let Ok(data2) = fs::read(path) else {
        return false;
    };
    if meta.dev() != meta2.dev()
        || meta.ino() != meta2.ino()
        || meta.uid() != meta2.uid()
        || data != data2
    {
        return false;
    }
    fs::remove_file(path).is_ok()
}
#[cfg(not(unix))]
fn remove_proven_stale_lock(_: &Path) -> bool {
    false
}
fn hostname() -> String {
    let mut bytes = [0u8; 256];
    let result = unsafe { libc::gethostname(bytes.as_mut_ptr().cast(), bytes.len()) };
    if result != 0 {
        return "unknown".into();
    }
    let len = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..len]).replace(['\n', '\r'], "")
}
struct TempGuard {
    path: PathBuf,
    active: bool,
}
impl Drop for TempGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = fs::remove_file(&self.path);
        }
    }
}
struct LockGuard {
    path: PathBuf,
}
impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
