//! Read-only, fail-closed probes for locally saved agent credentials.
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

pub const PROVIDERS: [&str; 12] = [
    "codex",
    "claude",
    "grok",
    "kimi",
    "deepseek",
    "minimax",
    "zai",
    "openrouter",
    "synthetic",
    "ollama",
    "opencode",
    "cursor",
];
const LIMIT: u64 = 1_048_576;

/// All inputs are injected, making probes deterministic and avoiding process-global environment reads.
#[derive(Clone)]
pub struct CredentialContext {
    pub home: PathBuf,
    pub env: BTreeMap<String, String>,
}
impl CredentialContext {
    pub fn new(home: impl Into<PathBuf>, env: BTreeMap<String, String>) -> Self {
        Self {
            home: home.into(),
            env,
        }
    }
    fn get(&self, k: &str) -> Option<&str> {
        self.env.get(k).map(String::as_str)
    }
    fn xdg(&self, k: &str, default: PathBuf) -> PathBuf {
        self.get(k)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or(default)
    }
    fn data(&self) -> PathBuf {
        self.xdg("XDG_DATA_HOME", self.home.join(".local/share"))
    }
    fn config(&self) -> PathBuf {
        self.xdg("XDG_CONFIG_HOME", self.home.join(".config"))
    }
    fn pi(&self) -> PathBuf {
        self.get("PI_CODING_AGENT_DIR")
            .filter(|s| !s.trim().is_empty())
            .map(|s| expand(s.trim(), &self.home))
            .unwrap_or(self.home.join(".pi/agent"))
    }
    fn omp(&self) -> PathBuf {
        self.home.join(".omp/agent")
    }
    fn path(&self, key: &str, default: PathBuf) -> PathBuf {
        self.get(key)
            .filter(|s| !s.is_empty())
            .map(|s| expand(s, &self.home))
            .unwrap_or(default)
    }
    pub fn availability(&self) -> BTreeMap<String, bool> {
        PROVIDERS
            .into_iter()
            .map(|p| (p.into(), self.probe(p)))
            .collect()
    }
    pub fn probe(&self, id: &str) -> bool {
        match id {
            "codex" => self.codex(),
            "claude" => self.claude(),
            "grok" => self.grok(),
            "kimi" => self.kimi(),
            "deepseek" => self.deepseek(),
            "minimax" => self.minimax(),
            "zai" => self.zai(),
            "openrouter" => self.openrouter(),
            "synthetic" => self.synthetic(),
            "ollama" => self.ollama(),
            "opencode" => self.opencode(),
            "cursor" => self.cursor(),
            _ => false,
        }
    }
    fn json(&self, p: &Path) -> Option<Value> {
        let b = read(p)?;
        serde_json::from_slice(&b).ok()
    }
    fn codex(&self) -> bool {
        let h = self
            .get("CODEX_HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or(self.home.join(".codex"));
        let auth = if h.join("auth.json").is_file() {
            match self.json(&h.join("auth.json")) {
                Some(v) => v,
                None => return false,
            }
        } else {
            Value::Null
        };
        truth(self.get("CODEX_ACCESS_TOKEN"))
            || field(&auth, &["OPENAI_API_KEY"])
            || field(&auth, &["tokens", "access_token"])
            || field(&auth, &["tokens", "refresh_token"])
    }
    fn claude(&self) -> bool {
        let d = self.path("CLAUDE_CONFIG_DIR", self.home.join(".claude"));
        self.get("ANTHROPIC_API_KEY").is_some_and(|s| !s.is_empty())
            || self.json(&d.join(".credentials.json")).is_some_and(|v| {
                field_trim(&v, &["claudeAiOauth", "accessToken"])
                    || v.pointer("/claudeAiOauth/accessToken")
                        .and_then(Value::as_str)
                        .and_then(|s| substitute(s, &self.env))
                        .is_some_and(|s| !s.trim().is_empty())
            })
    }
    fn grok(&self) -> bool {
        let p = self
            .get("GROK_HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .unwrap_or(self.home.join(".grok"))
            .join("auth.json");
        self.json(&p).is_some_and(|v| {
            v.as_object()
                .and_then(|m| m.values().find(|v| v.is_object()))
                .and_then(|v| v.get("key"))
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty())
        })
    }
    fn kimi(&self) -> bool {
        if truth(self.get("KIMI_API_KEY").map(str::trim)) {
            return true;
        }
        if let Some(raw) = self.get("KIMI_CODE_HOME").filter(|s| !s.is_empty()) {
            let p = expand(raw, &self.home).join("credentials/kimi-code.json");
            return self.json(&p).is_some_and(|v| field(&v, &["access_token"]));
        }
        let mut p = vec![self.home.join(".kimi-code/credentials/kimi-code.json")];
        let legacy = self
            .get("KIMI_SHARE_DIR")
            .filter(|s| !s.trim().is_empty())
            .map(|s| PathBuf::from(s.trim()))
            .unwrap_or(self.home.join(".kimi"));
        p.push(legacy.join("credentials/kimi-code.json"));
        p.into_iter()
            .find(|p| p.is_file())
            .and_then(|p| self.json(&p))
            .is_some_and(|v| field(&v, &["access_token"]))
    }
    fn deepseek(&self) -> bool {
        if truth(self.get("DEEPSEEK_API_KEY").map(str::trim)) {
            return true;
        }
        let mut paths = vec![
            self.pi().join("auth.json"),
            self.home.join(".config/omarchy/agents/provider.json"),
        ];
        let d = self
            .get("DSH_HOME")
            .filter(|s| !s.trim().is_empty())
            .map(|s| expand(s.trim(), &self.home))
            .unwrap_or(self.home.join(".dsh"));
        paths.extend([d.join(".credentials.yaml"), d.join(".env")]);
        paths.into_iter().any(|p| {
            self.json(&p).is_some_and(|v| deepkey(&v))
                || read(&p).is_some_and(|b| deep_text(&String::from_utf8_lossy(&b)))
        })
    }
    fn minimax(&self) -> bool {
        if truth(self.get("MINIMAX_API_KEY").map(str::trim)) {
            return true;
        }
        let dir = self
            .get("MMX_CONFIG_DIR")
            .map(|s| {
                if s.is_empty() {
                    PathBuf::from(".")
                } else {
                    expand(s, &self.home)
                }
            })
            .unwrap_or(self.home.join(".mmx"));
        let cp = dir.join("config.json");
        if let Some(v) = self
            .json(&cp)
            .filter(|v| v.as_object().is_some_and(|m| !m.is_empty()))
        {
            return first_config_key(&v);
        }
        if read(&dir.join("config.yaml"))
            .is_some_and(|b| yaml_named(&String::from_utf8_lossy(&b), "api_key"))
        {
            return true;
        }
        let p = self.pi().join("auth.json");
        self.json(&p).is_some_and(|v| {
            storekey(
                &v,
                &[
                    "minimax",
                    "minimax-cn",
                    "minimax-coding-plan",
                    "minimax-cn-coding-plan",
                ],
                true,
            )
        })
    }
    fn zai(&self) -> bool {
        if let Some(s) = ["ZAI_API_KEY", "ZHIPU_API_KEY", "ZHIPUAI_API_KEY"]
            .iter()
            .find_map(|k| self.get(k).filter(|s| truth(Some(s))))
        {
            return truth(Some(s.trim()));
        }
        let g = self.config().join("omarchy/agents/provider.json");
        if self
            .json(&g)
            .and_then(|v| v.get("zai").cloned())
            .and_then(|v| v.get("apiKey").cloned())
            .and_then(|v| v.as_str().map(str::to_owned))
            .is_some_and(|s| !s.trim().is_empty())
        {
            return true;
        }
        if dotenv(&self.home.join(".omp/agent/.env"), "ZAI_API_KEY")
            .or_else(|| dotenv(&self.pi().join(".env"), "ZAI_API_KEY"))
            .is_some_and(|s| !s.trim().is_empty())
        {
            return true;
        }
        self.json(&self.pi().join("auth.json"))
            .is_some_and(|v| storekey(&v, &["zai", "zhipu"], true))
    }
    fn openrouter(&self) -> bool {
        self.get("OPENROUTER_API_KEY")
            .is_some_and(|s| !s.trim().is_empty())
            || self
                .json(&self.config().join("openrouter.json"))
                .is_some_and(|v| field(&v, &["apiKey"]))
            || self
                .json(&self.pi().join("auth.json"))
                .is_some_and(|v| storekey(&v, &["openrouter"], false))
    }
    fn synthetic(&self) -> bool {
        if truth(self.get("SYNTHETIC_API_KEY").map(str::trim)) {
            return true;
        }
        let pi = self
            .get("PI_CODING_AGENT_DIR")
            .map(|s| expand(s, &self.home))
            .unwrap_or(self.home.join(".pi/agent"));
        let mut paths = vec![pi.join("auth.json"), self.home.join(".omp/agent/auth.json")];
        for base in [
            self.home.join(".pi/profiles"),
            self.home.join(".omp/profiles"),
        ] {
            if let Ok(rd) = fs::read_dir(base) {
                let mut ps: Vec<_> = rd
                    .flatten()
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                    .map(|e| e.path().join("agent/auth.json"))
                    .collect();
                ps.sort();
                paths.extend(ps)
            }
        }
        paths.into_iter().any(|p| {
            self.json(&p).is_some_and(|v| {
                v.pointer("/synthetic/key")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().starts_with('!'))
                    .and_then(|s| substitute(s, &self.env))
                    .is_some_and(|s| !s.trim().is_empty())
            })
        })
    }
    fn ollama(&self) -> bool {
        truth(self.get("OLLAMA_API_KEY").map(str::trim))
            || self
                .json(&self.config().join("ollama.json"))
                .is_some_and(|v| field(&v, &["apiKey"]))
            || self
                .json(&self.pi().join("auth.json"))
                .is_some_and(|v| storekey(&v, &["ollama-cloud"], true))
    }
    fn opencode(&self) -> bool {
        if ["OPENCODE_API_KEY", "OPENCODE_GO_API_KEY"]
            .iter()
            .any(|k| truth(self.get(k).map(str::trim)))
        {
            return true;
        }
        let root = self.data().join("opencode");
        let configured = self
            .get("OPENCODE_DB")
            .filter(|s| !s.trim().is_empty())
            .map(|s| {
                let p = expand(s, &self.home);
                if p.is_absolute() { p } else { root.join(p) }
            });
        let db = configured.unwrap_or_else(|| {
            let a = root.join("opencode.db");
            if a.is_file() {
                a
            } else {
                let b = root.join("opencode-v2.db");
                if b.is_file() { b } else { a }
            }
        });
        if opencode_db(&db) {
            return true;
        }
        self.json(&self.data().join("opencode/auth.json"))
            .is_some_and(|v| opencode_storekey(&v))
            || self
                .json(&self.pi().join("auth.json"))
                .is_some_and(|v| opencode_storekey(&v))
            || self
                .json(&self.omp().join("auth.json"))
                .is_some_and(|v| opencode_storekey(&v))
    }
    fn cursor(&self) -> bool {
        let p = self
            .get("CURSOR_AUTH_PATH")
            .filter(|s| !s.trim().is_empty())
            .map(|s| expand(s.trim(), &self.home))
            .unwrap_or(self.config().join("cursor/auth.json"));
        let v = self.json(&p);
        if let Some(k) = self.get("CURSOR_API_KEY").filter(|s| !s.trim().is_empty()) {
            return v.as_ref().is_some_and(|v| {
                v.get("apiKey")
                    .and_then(Value::as_str)
                    .is_some_and(|s| s.trim() == k.trim())
                    && field_trim(v, &["accessToken"])
            });
        }
        v.is_some_and(|v| field_trim(&v, &["accessToken"]))
            || self
                .config()
                .join("Cursor/User/globalStorage/state.vscdb")
                .exists()
                && sqlite_cursor(&self.config().join("Cursor/User/globalStorage/state.vscdb"))
    }
}
fn expand(s: &str, h: &Path) -> PathBuf {
    let s = s
        .strip_prefix("~/")
        .map(|x| h.join(x))
        .unwrap_or_else(|| PathBuf::from(s));
    s
}
fn read(p: &Path) -> Option<Vec<u8>> {
    let m = fs::symlink_metadata(p).ok()?;
    if !m.file_type().is_file() || m.len() > LIMIT {
        return None;
    }
    let mut f = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(p)
        .ok()?;
    if !f.metadata().ok()?.is_file() {
        return None;
    }
    let mut b = Vec::new();
    f.take(LIMIT + 1).read_to_end(&mut b).ok()?;
    (b.len() as u64 <= LIMIT).then_some(b)
}
fn truth(s: Option<&str>) -> bool {
    s.is_some_and(|s| !s.is_empty())
}
fn field(v: &Value, path: &[&str]) -> bool {
    let mut x = v;
    for k in path {
        let Some(y) = x.get(*k) else { return false };
        x = y
    }
    x.as_str().is_some_and(|s| !s.is_empty())
}
fn keyok(s: &str, reject: bool) -> bool {
    !s.trim().is_empty() && (!reject || (!s.trim().starts_with('!') && !s.trim().starts_with('$')))
}
fn storekey(v: &Value, ids: &[&str], reject: bool) -> bool {
    ids.iter().any(|id| {
        let x = v.get(*id);
        x.is_some_and(|x| {
            if let Some(s) = x.as_str() {
                keyok(s, reject)
            } else {
                x.get("key")
                    .or_else(|| x.get("apiKey"))
                    .or_else(|| x.get("token"))
                    .and_then(Value::as_str)
                    .is_some_and(|s| keyok(s, reject))
                    || x.pointer("/oauth/access_token")
                        .and_then(Value::as_str)
                        .is_some_and(|s| keyok(s, reject))
            }
        })
    })
}
fn deepkey(v: &Value) -> bool {
    let Some(v) = v.get("deepseek") else {
        return false;
    };
    if let Some(s) = v.as_str() {
        return keyok(s, true);
    }
    ["key", "apiKey", "api_key"].iter().any(|k| {
        v.get(*k)
            .and_then(Value::as_str)
            .is_some_and(|s| keyok(s, true))
    })
}
fn first_config_key(v: &Value) -> bool {
    for k in ["api_key", "apiKey"] {
        if let Some(s) = v
            .get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            return keyok(s, true);
        }
    }
    v.pointer("/oauth/access_token")
        .and_then(Value::as_str)
        .is_some_and(|s| keyok(s, true))
}
fn clean_value(s: &str) -> String {
    let s = s.trim();
    let s = s.split('#').next().unwrap_or("").trim();
    s.trim_matches(|c| c == '\'' || c == '"').to_owned()
}
fn dotenv(p: &Path, key: &str) -> Option<String> {
    let b = read(p)?;
    let text = String::from_utf8_lossy(&b);
    let mut found = None;
    for line in text.lines() {
        let l = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        if let Some((k, v)) = l.split_once('=') {
            if k.trim() == key {
                found = Some(clean_value(v))
            }
        }
    }
    found
}
fn yaml_named(text: &str, key: &str) -> bool {
    text.lines().any(|l| {
        l.trim_start()
            .strip_prefix(&format!("{key}:"))
            .is_some_and(|v| keyok(&clean_value(v), true))
    })
}
fn deep_text(text: &str) -> bool {
    yaml_named(text, "DEEPSEEK_API_KEY")
        || dotenv_text(text, "DEEPSEEK_API_KEY").is_some_and(|s| keyok(&s, true))
}
fn dotenv_text(text: &str, key: &str) -> Option<String> {
    let mut found = None;
    for line in text.lines() {
        let l = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        if let Some((k, v)) = l.split_once('=') {
            if k.trim() == key {
                found = Some(clean_value(v))
            }
        }
    }
    found
}
fn substitute(s: &str, env: &BTreeMap<String, String>) -> Option<String> {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        let braced = chars.peek() == Some(&'{');
        if braced {
            chars.next();
        }
        let mut name = String::new();
        while let Some(&c) = chars.peek() {
            if c == '_' || c.is_ascii_alphanumeric() {
                name.push(c);
                chars.next();
            } else {
                break;
            }
        }
        if braced && chars.next() != Some('}') {
            return None;
        }
        let val = env.get(&name)?;
        if name.is_empty() || val.is_empty() {
            return None;
        }
        out.push_str(val)
    }
    Some(out)
}
fn field_trim(v: &Value, path: &[&str]) -> bool {
    let mut x = v;
    for k in path {
        let Some(y) = x.get(*k) else { return false };
        x = y
    }
    x.as_str().is_some_and(|s| !s.trim().is_empty())
}
fn opencode_storekey(v: &Value) -> bool {
    let Some(entry) = v.get("opencode") else {
        return false;
    };
    let key = entry
        .as_str()
        .or_else(|| entry.get("key").and_then(Value::as_str));
    key.is_some_and(|s| keyok(s, true))
}
fn opencode_db(p: &Path) -> bool {
    if !p.is_file() {
        return false;
    }
    // Bounded fail-closed WAL compatibility: WAL-mode databases are skipped because
    // a read-only SQLite open may still need shared-memory sidecars; do not risk writes.
    let wal = p.with_file_name(format!(
        "{}-wal",
        p.file_name().unwrap_or_default().to_string_lossy()
    ));
    if fs::metadata(wal).is_ok_and(|m| m.len() > 0) {
        return false;
    }
    let Ok(c) = Connection::open_with_flags(
        p,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return false;
    };
    let _ = c.busy_timeout(Duration::from_secs(2));
    let _ = c.pragma_update(None, "query_only", true);
    let Ok(mut q)=c.prepare("SELECT integration_id,value FROM credential WHERE active=1 AND integration_id IN ('opencode-go','opencode') ORDER BY integration_id='opencode-go' DESC,time_updated DESC") else{return false};
    let Ok(rows) = q.query_map([], |r| r.get::<_, String>(1)) else {
        return false;
    };
    rows.flatten().any(|s| {
        serde_json::from_str::<Value>(&s).ok().is_some_and(|v| {
            (v.get("type").and_then(Value::as_str) == Some("key")
                && v.get("key")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.trim().is_empty()))
                || (v.get("type").and_then(Value::as_str) == Some("oauth")
                    && v.get("access")
                        .and_then(Value::as_str)
                        .is_some_and(|s| !s.trim().is_empty()))
        })
    })
}
fn sqlite_cursor(p: &Path) -> bool {
    let uri = format!("file:{}?mode=ro", p.display());
    let Ok(c) = Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    ) else {
        return false;
    };
    let _ = c.busy_timeout(Duration::from_secs(2));
    let _ = c.pragma_update(None, "query_only", true);
    c.query_row(
        "SELECT value FROM ItemTable WHERE key='cursorAuth/accessToken' LIMIT 1",
        [],
        |r| r.get::<_, String>(0),
    )
    .is_ok_and(|s| !s.trim().is_empty())
}
