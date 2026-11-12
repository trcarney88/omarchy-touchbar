use crate::burnrate::{BurnRate, BurnWindow};
use chrono::{DateTime, Duration, Utc};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

pub const PROVIDERS: [(&str, &str); 12] = [
    ("codex", "OpenAI / Codex"),
    ("claude", "Anthropic / Claude"),
    ("grok", "xAI / Grok"),
    ("kimi", "Kimi"),
    ("deepseek", "DeepSeek"),
    ("minimax", "MiniMax"),
    ("zai", "Z.ai"),
    ("openrouter", "OpenRouter"),
    ("synthetic", "Synthetic"),
    ("ollama", "Ollama Cloud"),
    ("opencode", "OpenCode Zen"),
    ("cursor", "Cursor"),
];
#[derive(Clone, Debug)]
pub struct LiveWindow {
    pub window: String,
    pub label: String,
    pub fraction: f64,
    pub resets_at: Option<DateTime<Utc>>,
    pub countdown: String,
    pub expired: bool,
}
#[derive(Clone, Debug)]
pub struct LiveSnapshot {
    pub provider: String,
    pub account: String,
    pub enabled: bool,
    pub windows: Vec<LiveWindow>,
    pub status: String,
    pub stale: bool,
    pub burn_minutes: Option<f64>,
    pub limits_fetched_at_ms: i64,
}
#[derive(Clone, Debug)]
pub struct ProviderState {
    pub enabled: bool,
    pub account: String,
    pub windows: Vec<LiveWindow>,
    pub status: String,
    pub stale: bool,
    pub burn_minutes: Option<f64>,
    pub limits_fetched_at_ms: i64,
}
#[derive(Clone, Debug)]
pub struct RefreshRequest {
    pub generation: u64,
    pub provider: String,
    pub argv: Vec<String>,
}

pub fn usage_path(state_home: &Path, provider: &str) -> PathBuf {
    state_home
        .join("omarchy/agents/usage")
        .join(format!("{provider}.json"))
}
pub fn parse_record(
    bytes: &[u8],
    now: DateTime<Utc>,
) -> Result<(Vec<LiveWindow>, String, i64, bool, String), String> {
    if bytes.len() > 1024 * 1024 {
        return Err("Usage data is too large".into());
    }
    let v: Value =
        serde_json::from_slice(bytes).map_err(|_| "Usage data unavailable".to_string())?;
    if !v.is_object() || v["ready"] != true {
        return Err("Usage data unavailable".into());
    }
    let a = v["limits"].as_array().ok_or("Limits unavailable")?;
    if a.is_empty() || a.len() > 8 {
        return Err("Limits unavailable".into());
    }
    let mut out = Vec::new();
    for x in a {
        let (Some(label), Some(frac), Some(rs)) = (
            x["label"].as_str(),
            x["percent"].as_f64(),
            x["resetsAt"].as_str(),
        ) else {
            continue;
        };
        if label.is_empty()
            || label.chars().count() > 48
            || label.contains(['<', '>'])
            || !frac.is_finite()
            || !(0.0..=1.0).contains(&frac)
        {
            continue;
        }
        let reset = if rs.is_empty() {
            None
        } else {
            match DateTime::parse_from_rfc3339(rs) {
                Ok(t) => Some(t.with_timezone(&Utc)),
                Err(_) => continue,
            }
        };
        if reset.is_some_and(|r| r > now + Duration::days(366)) {
            continue;
        }
        let mins = reset.map(|r| ((r - now).num_seconds().max(0) + 59) / 60);
        let countdown = match mins {
            None => "reset unknown".into(),
            Some(0) => "reset due".into(),
            Some(m) if m >= 1440 => format!("{}d {}h", m / 1440, (m % 1440) / 60),
            Some(m) => format!("{}h {}m", m / 60, m % 60),
        };
        out.push(LiveWindow {
            window: label.into(),
            label: if label == "5h window" || label == "Session (5-hour)" {
                "5 HOUR".into()
            } else if label == "Weekly (7-day)" {
                "WEEKLY".into()
            } else {
                label.into()
            },
            fraction: frac,
            resets_at: reset,
            countdown,
            expired: mins == Some(0),
        });
    }
    let status = v["usageStatusText"]
        .as_str()
        .unwrap_or("")
        .chars()
        .take(120)
        .collect();
    let fetched = v["limitsFetchedAt"]
        .as_f64()
        .filter(|n| n.is_finite())
        .unwrap_or(0.0) as i64;
    let stale = v["limitsStale"] == true
        || fetched <= 0
        || fetched > now.timestamp_millis() + 60_000
        || now.timestamp_millis() - fetched > 20 * 60_000
        || out.iter().any(|w| w.expired);
    let account = v["accounts"]
        .as_array()
        .and_then(|a| a.iter().find(|x| x["active"] == true))
        .and_then(|x| x["id"].as_str())
        .unwrap_or("")
        .to_string();
    Ok((out, account, fetched, stale, status))
}

pub struct LiveBackend {
    pub state_home: PathBuf,
    pub selected: String,
    pub enabled: HashMap<String, bool>,
    pub states: HashMap<String, ProviderState>,
    pub test_mode: bool,
    pub fire_trigger_minutes: u32,
    generation: u64,
    burn: BurnRate,
    refresh_failed: bool,
    live_selected: Option<String>,
}
impl LiveBackend {
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn new(state_home: PathBuf, enabled: HashMap<String, bool>) -> Self {
        Self {
            state_home,
            selected: "codex".into(),
            enabled,
            states: HashMap::new(),
            test_mode: false,
            fire_trigger_minutes: 30,
            generation: 0,
            burn: BurnRate::default(),
            refresh_failed: false,
            live_selected: None,
        }
    }
    pub fn select(&mut self, id: &str) -> bool {
        if !PROVIDERS.iter().any(|x| x.0 == id) || !self.enabled.get(id).copied().unwrap_or(false) {
            return false;
        }
        self.selected = id.into();
        self.generation += 1;
        self.refresh_failed = false;
        true
    }
    pub fn refresh_request(&mut self) -> Option<RefreshRequest> {
        if self.test_mode {
            return None;
        }
        Some(RefreshRequest {
            generation: self.generation,
            provider: self.selected.clone(),
            argv: vec![
                "omarchy".into(),
                "agent".into(),
                "usage".into(),
                "update".into(),
                "--limits-only".into(),
                self.selected.clone(),
            ],
        })
    }
    pub fn complete(&mut self, generation: u64, failed: bool) {
        if generation == self.generation {
            self.refresh_failed = failed
        }
    }
    pub fn poll(&mut self, now: DateTime<Utc>) -> LiveSnapshot {
        let id = self.selected.clone();
        let enabled = self.enabled.get(&id).copied().unwrap_or(false);
        let path = usage_path(&self.state_home, &id);
        let parsed = read_bounded(&path).and_then(|b| parse_record(&b, now));
        let (mut wins, account, fetched, mut stale, mut status) = match parsed {
            Ok(x) => x,
            Err(e) => (vec![], String::new(), 0, true, e),
        };
        if !enabled {
            wins.clear();
            status = "Sign-in required".into()
        }
        stale |= self.refresh_failed;
        let usable = enabled && !stale && !wins.is_empty();
        let burn = if self.test_mode {
            None
        } else {
            self.burn.observe(
                &id,
                &account,
                &wins
                    .iter()
                    .map(|w| BurnWindow {
                        window: w.window.clone(),
                        fraction: w.fraction,
                        resets_at: w.resets_at,
                    })
                    .collect::<Vec<_>>(),
                fetched,
                now.timestamp_millis(),
                usable,
            )
        };
        if status.is_empty() {
            status = if stale {
                "STALE · retrying".into()
            } else {
                "LIVE · used".into()
            }
        }
        if let Some(m) = burn {
            if m <= self.fire_trigger_minutes as f64 {
                status = format!("~{}m to limit", m.ceil() as u64)
            }
        }
        LiveSnapshot {
            provider: id,
            account,
            enabled,
            windows: wins,
            status,
            stale,
            burn_minutes: burn,
            limits_fetched_at_ms: fetched,
        }
    }
    pub fn set_test_mode(&mut self, on: bool) {
        if self.test_mode != on {
            self.burn.clear();
            if on {
                self.live_selected = Some(self.selected.clone());
            } else if let Some(id) = self.live_selected.take() {
                self.selected = id;
            }
            self.test_mode = on;
            self.generation += 1
        }
    }
    pub fn update_availability(&mut self, availability: HashMap<String, bool>) {
        self.enabled = availability;
        if !self.enabled.get(&self.selected).copied().unwrap_or(false) {
            self.burn.clear();
        }
    }
}
fn read_bounded(path: &Path) -> Result<Vec<u8>, String> {
    let m = fs::metadata(path).map_err(|_| "Usage data unavailable".to_string())?;
    if !m.is_file() || m.len() > 1024 * 1024 {
        return Err("Usage data is too large".into());
    }
    use std::io::Read;
    let f = fs::File::open(path).map_err(|_| "Usage data unavailable".to_string())?;
    let mut b = Vec::with_capacity(m.len() as usize);
    f.take(1024 * 1024 + 1)
        .read_to_end(&mut b)
        .map_err(|_| "Usage data unavailable".to_string())?;
    if b.len() > 1024 * 1024 {
        Err("Usage data is too large".into())
    } else {
        Ok(b)
    }
}

impl LiveSnapshot {
    pub fn apply_to_model(&self, model: &mut touchmeter_ui::Model, fire_trigger_minutes: u32) {
        model.providers = PROVIDERS
            .iter()
            .map(|(id, name)| touchmeter_ui::Provider {
                id: (*id).into(),
                name: (*name).into(),
                enabled: if *id == self.provider {
                    self.enabled
                } else {
                    model
                        .providers
                        .iter()
                        .find(|p| p.id == *id)
                        .is_some_and(|p| p.enabled)
                },
            })
            .collect();
        if let Some(i) = model.providers.iter().position(|p| p.id == self.provider) {
            model.selected = i;
        }
        model.limits = self
            .windows
            .iter()
            .map(|w| touchmeter_ui::Limit {
                label: w.label.clone(),
                fraction: w.fraction as f32,
                countdown: w.countdown.clone(),
            })
            .collect();
        model.status = self.status.clone();
        model.stale = self.stale;
        model.burn_minutes = self.burn_minutes.unwrap_or(-1.0) as f32;
        model.fire_trigger_minutes = fire_trigger_minutes as f32;
        model.fire = self
            .burn_minutes
            .is_some_and(|m| m <= fire_trigger_minutes as f64);
    }
}
