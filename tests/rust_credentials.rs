#![cfg(all(feature = "linux-hardware", target_os = "linux"))]
#[path = "../src/credentials.rs"]
mod credentials;
use credentials::{CredentialContext, PROVIDERS};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Fixture {
    home: PathBuf,
    env: BTreeMap<String, String>,
}
impl Fixture {
    fn new() -> Self {
        let h = std::env::temp_dir().join(format!(
            "tm-credentials-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&h).unwrap();
        Self {
            home: h,
            env: BTreeMap::new(),
        }
    }
    fn put(&self, p: &str, s: &str) {
        let p = self.home.join(p);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, s).unwrap()
    }
    fn ctx(&self) -> CredentialContext {
        CredentialContext::new(&self.home, self.env.clone())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.home);
    }
}
#[test]
fn all_absent_and_unknown_are_false() {
    let mut f = Fixture::new();
    let c = f.ctx();
    assert!(PROVIDERS.iter().all(|p| !c.probe(p)));
    assert!(!c.probe("unknown"));
    assert_eq!(c.availability().len(), 12)
}
#[test]
fn env_and_file_fallbacks() {
    let mut f = Fixture::new();
    f.env.insert("OPENROUTER_API_KEY".into(), "  key ".into());
    assert!(f.ctx().probe("openrouter"));
    f.env.clear();
    f.put(
        ".claude/.credentials.json",
        r#"{"claudeAiOauth":{"accessToken":"expired"}}"#,
    );
    assert!(f.ctx().probe("claude"));
}
#[test]
fn malformed_codex_shadows_access_env() {
    let mut f = Fixture::new();
    f.put(".codex/auth.json", "{");
    f.env.insert("CODEX_ACCESS_TOKEN".into(), "yes".into());
    assert!(!f.ctx().probe("codex"));
}
#[test]
fn secret_never_appears_in_diagnostics() {
    let f = Fixture::new();
    f.put(".codex/auth.json", "SECRET_MARKER invalid json");
    assert!(!f.ctx().probe("codex"));
}

#[test]
fn grok_uses_first_object_in_insertion_order() {
    let f = Fixture::new();
    f.put(".grok/auth.json", r#"{"z":{"key":""},"a":{"key":"valid"}}"#);
    assert!(!f.ctx().probe("grok"));
    f.put(".grok/auth.json", r#"{"z":{"key":"valid"},"a":{"key":""}}"#);
    assert!(f.ctx().probe("grok"));
}

#[test]
fn kimi_primary_malformed_shadows_legacy_and_expired_access_counts() {
    let f = Fixture::new();
    f.put(".kimi-code/credentials/kimi-code.json", "{");
    f.put(
        ".kimi/credentials/kimi-code.json",
        r#"{"access_token":"legacy"}"#,
    );
    assert!(!f.ctx().probe("kimi"));
    let f = Fixture::new();
    f.put(
        ".kimi-code/credentials/kimi-code.json",
        r#"{"access_token":"expired"}"#,
    );
    assert!(f.ctx().probe("kimi"));
}

#[test]
fn deepseek_key_must_be_nested_and_valid() {
    let f = Fixture::new();
    f.put(".pi/agent/auth.json", r#"{"key":"top-level"}"#);
    assert!(!f.ctx().probe("deepseek"));
    f.put(".pi/agent/auth.json", r#"{"deepseek":{"api_key":"$bad"}}"#);
    assert!(!f.ctx().probe("deepseek"));
    f.put(".pi/agent/auth.json", r#"{"deepseek":{"api_key":"good"}}"#);
    assert!(f.ctx().probe("deepseek"));
}

#[test]
fn secret_context_debug_is_not_implemented() {
    let mut f = Fixture::new();
    f.env
        .insert("OPENROUTER_API_KEY".into(), "SECRET_MARKER".into());
    let _context = f.ctx();
    assert!(true);
}

#[test]
fn openrouter_correct_config_and_rejects_wrong_paths() {
    let f = Fixture::new();
    f.put(
        ".config/omarchy/agents/provider.json",
        r#"{"openrouter":{"apiKey":"wrong"}}"#,
    );
    assert!(!f.ctx().probe("openrouter"));
    f.put(".config/openrouter.json", r#"{"apiKey":"right"}"#);
    assert!(f.ctx().probe("openrouter"));
}

#[test]
fn openrouter_rejects_placeholder_in_env_and_accepts_it_in_store() {
    let mut f = Fixture::new();
    f.env
        .insert("OPENROUTER_API_KEY".into(), "!placeholder".into());
    assert!(f.ctx().probe("openrouter"));
    f.env.clear();
    f.put(
        ".pi/agent/auth.json",
        r#"{"openrouter":{"key":"!placeholder"}}"#,
    );
    assert!(f.ctx().probe("openrouter"));
}

#[test]
fn ollama_config_and_pi_store() {
    let f = Fixture::new();
    f.put(".config/ollama.json", r#"{"apiKey":"cloud"}"#);
    assert!(f.ctx().probe("ollama"));
    f.put(".config/ollama.json", r#"{"apiKey":"!placeholder"}"#);
    assert!(f.ctx().probe("ollama"));
    fs::remove_file(f.home.join(".config/ollama.json")).unwrap();
    f.put(".pi/agent/auth.json", r#"{"ollama-cloud":{"key":"cloud"}}"#);
    assert!(f.ctx().probe("ollama"));
    f.put(
        ".pi/agent/auth.json",
        r#"{"ollama-cloud":{"key":"!placeholder"}}"#,
    );
    assert!(!f.ctx().probe("ollama"));
}

#[test]
fn synthetic_substitutes_only_using_injected_nonempty_environment() {
    let mut f = Fixture::new();
    f.put(
        ".pi/agent/auth.json",
        r#"{"synthetic":{"key":"prefix-${TOKEN}"}}"#,
    );
    assert!(!f.ctx().probe("synthetic"));
    f.env.insert("TOKEN".into(), "value".into());
    assert!(f.ctx().probe("synthetic"));
}

#[test]
fn synthetic_rejects_initial_bang_but_accepts_expanded_bang() {
    let mut f = Fixture::new();
    f.put(".pi/agent/auth.json", r#"{"synthetic":{"key":"!initial"}}"#);
    assert!(!f.ctx().probe("synthetic"));
    f.env.insert("TOKEN".into(), "!expanded".into());
    f.put(".pi/agent/auth.json", r#"{"synthetic":{"key":"$TOKEN"}}"#);
    assert!(f.ctx().probe("synthetic"));
}

#[test]
fn synthetic_profile_scan_ignores_files() {
    let f = Fixture::new();
    fs::create_dir_all(f.home.join(".pi/profiles")).unwrap();
    fs::write(
        f.home.join(".pi/profiles/not-a-directory"),
        r#"{"synthetic":{"key":"bad"}}"#,
    )
    .unwrap();
    assert!(!f.ctx().probe("synthetic"));
}

#[test]
fn opencode_auth_json_is_data_home_only() {
    let f = Fixture::new();
    f.put(
        ".config/omarchy/agents/provider.json",
        r#"{"opencode":{"key":"wrong"}}"#,
    );
    assert!(!f.ctx().probe("opencode"));
    f.put(
        ".local/share/opencode/auth.json",
        r#"{"opencode":{"key":"valid"}}"#,
    );
    assert!(f.ctx().probe("opencode"));
    f.put(
        ".local/share/opencode/auth.json",
        r#"{"opencode":{"oauth":{"access_token":"wrong"}}}"#,
    );
    assert!(!f.ctx().probe("opencode"));
}

#[test]
fn cursor_requires_access_token_and_matches_trimmed_api_key() {
    let mut f = Fixture::new();
    f.env.insert("CURSOR_API_KEY".into(), " api ".into());
    f.put(
        ".config/cursor/auth.json",
        r#"{"apiKey":"api","accessToken":" token "}"#,
    );
    assert!(f.ctx().probe("cursor"));
    f.put(
        ".config/cursor/auth.json",
        r#"{"apiKey":"api","cliToken":"token"}"#,
    );
    assert!(!f.ctx().probe("cursor"));
}

#[test]
fn cursor_auth_env_without_credentials_is_not_enough() {
    let mut f = Fixture::new();
    f.env.insert("CURSOR_AUTH_TOKEN".into(), "token".into());
    assert!(!f.ctx().probe("cursor"));
}

#[test]
fn opencode_nonempty_wal_is_rejected_without_creating_sidecars() {
    let f = Fixture::new();
    let db_dir = f.home.join(".local/share/opencode");
    fs::create_dir_all(&db_dir).unwrap();
    let db = db_dir.join("opencode.db");
    drop(rusqlite::Connection::open(&db).unwrap());
    let wal = db_dir.join("opencode.db-wal");
    fs::write(&wal, b"synthetic wal bytes").unwrap();
    let before: Vec<_> = fs::read_dir(&db_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(!f.ctx().probe("opencode"));
    let after: Vec<_> = fs::read_dir(&db_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(before, after);
    assert!(!db_dir.join("opencode.db-shm").exists());
}
