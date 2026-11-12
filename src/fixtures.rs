//! Strict, bounded decoding and projection of simulator fixture data.
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use touchmeter_ui::{Limit, Model};

pub const MAX_FIXTURE_BYTES: usize = 65_536;
pub const PROVIDER_IDS: [&str; 12] = [
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

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub version: u8,
    pub selected: String,
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub now: Option<String>,
    pub providers: BTreeMap<String, ProviderFixture>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<VolumeFixture>,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProviderFixture {
    pub enabled: bool,
    pub ready: bool,
    pub limits: Vec<LimitFixture>,
    #[serde(default, rename = "limitsStale")]
    pub limits_stale: bool,
    #[serde(
        default,
        rename = "limitsFetchedAt",
        skip_serializing_if = "Option::is_none"
    )]
    pub limits_fetched_at: Option<f64>,
    #[serde(default, rename = "usageStatusText")]
    pub usage_status_text: String,
    #[serde(
        default,
        rename = "burnMinutes",
        skip_serializing_if = "Option::is_none"
    )]
    pub burn_minutes: Option<f64>,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LimitFixture {
    pub label: String,
    pub percent: f64,
    #[serde(rename = "resetsAt")]
    pub resets_at: String,
}
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct VolumeFixture {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub percent: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available: Option<bool>,
}

impl Fixture {
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_FIXTURE_BYTES {
            return Err("Fixture exceeds 64 KiB".into());
        }
        let mut raw: serde_json::Value =
            serde_json::from_slice(bytes).map_err(|e| format!("Invalid fixture: {e}"))?;
        let root = raw.as_object_mut().ok_or("Fixture must be an object")?;
        if let Some(v) = root.get("version") {
            if v.as_f64() == Some(1.0) {
                root.insert("version".into(), serde_json::Value::Number(1.into()));
            }
        }
        let providers = root
            .get("providers")
            .and_then(serde_json::Value::as_object)
            .ok_or("providers must be an object")?;
        for (id, p) in providers {
            if !p.is_object() {
                return Err(format!("Provider {id} must be an object"));
            }
            let fields = p.as_object().unwrap();
            if !fields
                .get("limits")
                .is_some_and(serde_json::Value::is_array)
            {
                return Err("limits must be an array".into());
            }
            if !fields["limits"]
                .as_array()
                .unwrap()
                .iter()
                .all(serde_json::Value::is_object)
            {
                return Err("limits must contain objects".into());
            }
            for key in ["limitsFetchedAt", "burnMinutes"] {
                if fields.get(key).is_some_and(serde_json::Value::is_null) {
                    return Err(format!("{key} must not be null"));
                }
            }
        }
        if root.get("now").is_some_and(serde_json::Value::is_null) {
            return Err("now must not be null".into());
        }
        if let Some(v) = root.get("volume") {
            let o = v.as_object().ok_or("volume must be an object")?;
            for k in ["percent", "muted", "available"] {
                if o.get(k).is_some_and(serde_json::Value::is_null) {
                    return Err(format!("volume.{k} must not be null"));
                }
            }
        }
        let f: Self = serde_json::from_value(raw).map_err(|e| format!("Invalid fixture: {e}"))?;
        f.validate()?;
        Ok(f)
    }
    pub fn validate(&self) -> Result<(), String> {
        let err = |s: &str| Err(s.to_owned());
        if self.version != 1 {
            return err("Fixture version must be 1");
        }
        if !["menu", "usage", "providers", "functions", "volume"].contains(&self.mode.as_str()) {
            return err("Invalid fixture mode");
        }
        if !PROVIDER_IDS.contains(&self.selected.as_str())
            || !self.providers.contains_key(&self.selected)
        {
            return err("Unknown or missing selected provider");
        }
        let now = self
            .now
            .as_deref()
            .map(parse_time)
            .transpose()?
            .unwrap_or_else(Utc::now);
        for (id, p) in &self.providers {
            if !PROVIDER_IDS.contains(&id.as_str()) {
                return err("Unknown provider");
            }
            if p.limits.len() > 8
                || p.usage_status_text.chars().count() > 120
                || p.burn_minutes
                    .is_some_and(|v| !v.is_finite() || !(0.0..=1440.0).contains(&v))
                || p.limits_fetched_at
                    .is_some_and(|v| !v.is_finite() || v < 0.)
            {
                return err("Invalid provider values");
            }
            for l in &p.limits {
                if l.label.is_empty()
                    || l.label.chars().count() > 48
                    || l.label.contains(['<', '>'])
                    || !l.percent.is_finite()
                    || !(0.0..=1.0).contains(&l.percent)
                {
                    return err("Invalid limit");
                }
                if !l.resets_at.is_empty() {
                    let t = parse_time(&l.resets_at)?;
                    if t > now + Duration::days(366) {
                        return err("Limit reset too far in future");
                    }
                }
            }
        }
        if let Some(v) = &self.volume {
            if v.percent
                .is_some_and(|n| !n.is_finite() || !(0.0..=100.0).contains(&n))
            {
                return err("Invalid volume percent");
            }
        }
        Ok(())
    }
    /// Projects provider-dependent display state while preserving catalog and appearance from `model`.
    pub fn project(&self, model: &mut Model, selected: &str, now: DateTime<Utc>) {
        let clock = self
            .now
            .as_deref()
            .and_then(|s| parse_time(s).ok())
            .unwrap_or(now);
        for p in &mut model.providers {
            p.enabled = self.providers.get(&p.id).is_some_and(|d| d.enabled);
        }
        model.fire = false;
        model.limits.clear();
        model.burn_minutes = -1.;
        model.stale = false;
        if let Some((i, _p)) = model
            .providers
            .iter_mut()
            .enumerate()
            .find(|(_, p)| p.id == selected)
        {
            model.selected = i;
            if let Some(data) = self.providers.get(selected) {
                model.app = match self.mode.as_str() {
                    "menu" => "menu",
                    "providers" => "ai",
                    "functions" => "functions",
                    "volume" => "volume",
                    _ => "ai",
                }
                .into();
                model.picker = self.mode == "providers";
                let fetched = data.limits_fetched_at.unwrap_or(0.);
                let stale = data.limits_stale
                    || fetched <= 0.
                    || fetched > clock.timestamp_millis() as f64 + 60_000.
                    || clock.timestamp_millis() as f64 - fetched > 20. * 60_000.
                    || data.limits.iter().any(|l| {
                        !l.resets_at.is_empty()
                            && parse_time(&l.resets_at).is_ok_and(|t| t <= clock)
                    });
                model.stale = stale;
                let usable = data.enabled && data.ready;
                model.limits = if usable {
                    data.limits
                        .iter()
                        .filter_map(|l| {
                            let countdown = if l.resets_at.is_empty() {
                                "reset unknown".into()
                            } else {
                                parse_time(&l.resets_at)
                                    .ok()
                                    .map(|t| human_countdown(t.signed_duration_since(clock)))
                                    .unwrap_or_else(|| "reset unknown".into())
                            };
                            let label = match l.label.as_str() {
                                "5h window" => "5 HOUR".into(),
                                "Weekly (7-day)" => "WEEKLY".into(),
                                _ => l.label.clone(),
                            };
                            Some(Limit {
                                label,
                                fraction: l.percent as f32,
                                countdown,
                            })
                        })
                        .collect()
                } else {
                    vec![]
                };
                model.status = if !data.enabled {
                    "Sign-in required".into()
                } else if !data.ready || data.limits.is_empty() {
                    if data.usage_status_text.is_empty() {
                        "Usage unavailable".into()
                    } else {
                        data.usage_status_text.clone()
                    }
                } else {
                    data.usage_status_text.clone()
                };
                if !stale && usable && !data.limits.is_empty() {
                    model.burn_minutes = data.burn_minutes.unwrap_or(-1.) as f32;
                }
                let burning =
                    model.burn_minutes >= 0. && model.burn_minutes <= model.fire_trigger_minutes;
                model.fire = burning && !stale && usable && !data.limits.is_empty();
                if burning {
                    model.status = format!("~{}m to limit", model.burn_minutes.ceil() as i32);
                }
                model.status = format!("TEST · {}", model.status);
            }
        }
        if let Some(v) = &self.volume {
            model.volume = (v.percent.unwrap_or(65.) / 100.) as f32;
            model.volume_muted = v.muted.unwrap_or(false);
            model.volume_available = v.available.unwrap_or(true);
        }
    }
}
fn parse_time(s: &str) -> Result<DateTime<Utc>, String> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| "Invalid ISO-8601 timestamp".into())
}
fn human_countdown(d: Duration) -> String {
    if d <= Duration::zero() {
        return "reset due".into();
    }
    let n = (d.num_seconds() + 59).max(0) / 60;
    if n < 1440 {
        format!("{}h {}m", n / 60, n % 60)
    } else {
        format!("{}d {}h", n / 1440, (n % 1440) / 60)
    }
}

pub fn scenario(name: &str, now: DateTime<Utc>) -> Result<Fixture, String> {
    if ![
        "all-providers",
        "high-usage",
        "stale",
        "missing",
        "signed-out",
        "fire",
        "menu",
        "volume",
        "volume-muted",
        "volume-unavailable",
    ]
    .contains(&name)
    {
        return Err("Unknown scenario".into());
    }
    let mut providers = BTreeMap::new();
    for (i, id) in PROVIDER_IDS.iter().enumerate() {
        let disabled = name == "signed-out";
        let limits = if name == "missing" {
            vec![]
        } else {
            vec![
                LimitFixture {
                    label: "5h window".into(),
                    percent: if name == "high-usage" {
                        0.96
                    } else {
                        0.12 + i as f64 * 0.06
                    },
                    resets_at: (now + Duration::hours(2)).to_rfc3339(),
                },
                LimitFixture {
                    label: "Weekly (7-day)".into(),
                    percent: 0.61,
                    resets_at: (now + Duration::days(4)).to_rfc3339(),
                },
            ]
        };
        providers.insert(
            (*id).into(),
            ProviderFixture {
                enabled: !disabled,
                ready: !disabled,
                limits,
                limits_stale: name == "stale",
                limits_fetched_at: Some(now.timestamp_millis() as f64),
                usage_status_text: if name == "missing" {
                    "Simulated unavailable limits".into()
                } else {
                    String::new()
                },
                burn_minutes: (name == "fire").then_some(20.),
            },
        );
    }
    let f = Fixture {
        version: 1,
        selected: "claude".into(),
        mode: if name == "all-providers" {
            "providers"
        } else if name == "menu" {
            "menu"
        } else if name.starts_with("volume") {
            "volume"
        } else {
            "usage"
        }
        .into(),
        now: Some(now.to_rfc3339()),
        providers,
        volume: name.starts_with("volume").then_some(VolumeFixture {
            percent: Some(65.),
            muted: Some(name == "volume-muted"),
            available: Some(name != "volume-unavailable"),
        }),
    };
    f.validate()?;
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn clock() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }
    #[test]
    fn samples_decode() {
        for p in [
            "docs/fixtures/claude-high.json",
            "docs/fixtures/fire.json",
            "docs/fixtures/volume.json",
        ] {
            Fixture::decode(&std::fs::read(p).unwrap()).unwrap();
        }
    }
    #[test]
    fn rejects_bad_shapes_and_bounds() {
        for s in [
            r#"{"version":1,"selected":"claude","mode":"usage","providers":{"claude":{"enabled":true,"ready":true,"limits":[],"x":1}}}"#,
            r#"{"version":"1","selected":"claude","mode":"usage","providers":{}}"#,
        ] {
            assert!(Fixture::decode(s.as_bytes()).is_err());
        }
        assert!(Fixture::decode(&vec![b' '; MAX_FIXTURE_BYTES + 1]).is_err());
        let base = r#"{"version":1.0,"selected":"claude","mode":"usage","providers":{"claude":{"enabled":true,"ready":true,"limits":[{"label":"ok","percent":0.2,"resetsAt":""}]}}}"#;
        assert!(Fixture::decode(base.as_bytes()).is_ok());
        for bad in [
            base.replace("\"limits\":[", "\"limits\":null,"),
            base.replace("\"resetsAt\":\"\"", "\"resetsAt\":null"),
            base.replace("\"limits\":[", "\"limits\":[[] ,"),
            base.replace("\"version\":1.0", "\"version\":true"),
        ] {
            assert!(Fixture::decode(bad.as_bytes()).is_err());
        }
        let null_now = base.replace("\"mode\":\"usage\"", "\"mode\":\"usage\",\"now\":null");
        assert!(Fixture::decode(null_now.as_bytes()).is_err());
        let volume = base.replace("\"mode\":\"usage\"", "\"mode\":\"usage\",\"volume\":[]");
        assert!(Fixture::decode(volume.as_bytes()).is_err());
    }
    #[test]
    fn scenarios_and_projection() {
        for n in [
            "all-providers",
            "high-usage",
            "stale",
            "missing",
            "signed-out",
            "fire",
            "menu",
            "volume",
            "volume-muted",
            "volume-unavailable",
        ] {
            let f = scenario(n, clock()).unwrap();
            let mut m = crate::base();
            f.project(&mut m, "claude", clock());
            assert_eq!(m.selected, 1);
            if n == "fire" {
                assert!(m.fire)
            }
            if n == "stale" || n == "missing" || n == "signed-out" {
                assert!(!m.fire)
            }
        }
    }
    #[test]
    fn fire_threshold_direction_and_suppression() {
        let mut f = scenario("fire", clock()).unwrap();
        let mut m = crate::base();
        f.project(&mut m, "claude", clock());
        assert!(m.fire);
        f.providers.get_mut("claude").unwrap().burn_minutes = Some(40.);
        f.project(&mut m, "claude", clock());
        assert!(!m.fire);
        f.providers.get_mut("claude").unwrap().burn_minutes = None;
        f.project(&mut m, "claude", clock());
        assert!(!m.fire);
        let p = f.providers.get_mut("claude").unwrap();
        p.burn_minutes = Some(20.);
        p.enabled = false;
        f.project(&mut m, "claude", clock());
        assert!(!m.fire);
    }
    #[test]
    fn fixture_clock_and_catalog_selection_are_projected() {
        let f = scenario("fire", clock()).unwrap();
        let later = clock() + Duration::hours(1);
        let mut m = crate::base();
        f.project(&mut m, "claude", later);
        assert!(m.limits[0].countdown.starts_with("2h"));
        let mut moving = f.clone();
        moving.now = None;
        moving.project(&mut m, "claude", later);
        assert!(m.limits[0].countdown.starts_with("1h"));
        moving.project(&mut m, "codex", later);
        assert_eq!(m.selected, 0);
        assert!(m.providers.iter().all(|p| p.enabled));
    }
    #[test]
    fn projection_formats_resets_labels_and_clears_missing_selection() {
        let mut f = scenario("fire", clock()).unwrap();
        {
            let p = f.providers.get_mut("claude").unwrap();
            p.burn_minutes = None;
            p.limits[0].resets_at = (clock() + Duration::seconds(49)).to_rfc3339();
            p.limits[1].resets_at = clock().to_rfc3339();
        }
        let mut m = crate::base();
        f.project(&mut m, "claude", clock());
        assert_eq!(m.limits[0].countdown, "0h 1m");
        assert_eq!(m.limits[0].label, "5 HOUR");
        assert_eq!(m.limits[1].countdown, "reset due");
        f.providers.get_mut("claude").unwrap().limits[0]
            .resets_at
            .clear();
        f.project(&mut m, "claude", clock());
        assert_eq!(m.limits[0].countdown, "reset unknown");
        f.providers.remove("codex");
        f.project(&mut m, "codex", clock());
        assert!(m.limits.is_empty());
        assert!(!m.fire);
    }
}
