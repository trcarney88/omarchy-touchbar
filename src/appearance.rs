use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub const MAX_CONFIG_BYTES: usize = 65_536;
pub const POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppearanceSnapshot {
    pub colors: BTreeMap<String, String>,
    pub accent: String,
    pub accent_foreground: String,
    pub language: String,
    pub format_locale: String,
    pub text_direction: TextDirection,
    pub translations: BTreeMap<String, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextDirection {
    LeftToRight,
    RightToLeft,
}

impl AppearanceSnapshot {
    pub fn color(&self, name: &str) -> &str {
        self.colors
            .get(name)
            .map(String::as_str)
            .unwrap_or("#101315")
    }
    pub fn translate(&self, source: &str) -> String {
        self.translations
            .get(source)
            .cloned()
            .unwrap_or_else(|| source.to_owned())
    }
    pub fn status_label(&self, source: &str) -> String {
        let (prefix, body) = source
            .strip_prefix("TEST · ")
            .map_or((None, source), |s| (Some("TEST"), s));
        let translated = if let Some(n) = body
            .strip_prefix('~')
            .and_then(|s| s.strip_suffix("m to limit"))
            .and_then(|s| s.trim().parse::<u64>().ok())
        {
            substitute(self, "%1m to limit", &[n.to_string()])
        } else {
            self.translate(body)
        };
        match prefix {
            Some(p) => format!("{} · {translated}", self.translate(p)),
            None => translated,
        }
    }
    pub fn countdown_label(&self, source: &str) -> String {
        if let Some((a, b)) = source.split_once("h ") {
            if let (Ok(a), Some(b)) = (
                a.parse::<u64>(),
                b.strip_suffix('m').and_then(|x| x.parse::<u64>().ok()),
            ) {
                return substitute(self, "%1h %2m", &[a.to_string(), b.to_string()]);
            }
        }
        if let Some((a, b)) = source.split_once("d ") {
            if let (Ok(a), Some(b)) = (
                a.parse::<u64>(),
                b.strip_suffix('h').and_then(|x| x.parse::<u64>().ok()),
            ) {
                return substitute(self, "%1d %2h", &[a.to_string(), b.to_string()]);
            }
        }
        self.translate(source)
    }
    pub fn basic_label(&self, source: &str) -> String {
        self.translate(source)
    }
    pub fn apply_color(&self, name: &str) -> &str {
        self.color(name)
    }
}

fn substitute(snapshot: &AppearanceSnapshot, key: &str, values: &[String]) -> String {
    let mut text = snapshot.translate(key);
    for (i, value) in values.iter().enumerate() {
        text = text.replace(&format!("%{}", i + 1), value);
    }
    text
}

#[derive(Clone, Debug)]
pub struct AppearanceOptions {
    pub demo: bool,
    pub language_override: Option<String>,
    pub home: PathBuf,
    pub omarchy_path: PathBuf,
    pub catalog_dir: PathBuf,
}

impl Default for AppearanceOptions {
    fn default() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        Self {
            demo: false,
            language_override: None,
            omarchy_path: PathBuf::from(
                std::env::var_os("OMARCHY_PATH").unwrap_or_else(|| "/usr/share/omarchy".into()),
            ),
            catalog_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("i18n"),
            home,
        }
    }
}

pub struct Appearance {
    options: AppearanceOptions,
    snapshot: AppearanceSnapshot,
    last_poll: Option<Instant>,
}

impl Appearance {
    pub fn new(options: AppearanceOptions) -> Self {
        let snapshot = load_snapshot(&options);
        Self {
            options,
            snapshot,
            last_poll: None,
        }
    }
    pub fn snapshot(&self) -> &AppearanceSnapshot {
        &self.snapshot
    }
    pub fn poll(&mut self, now: Instant) -> bool {
        if self
            .last_poll
            .is_some_and(|last| now.duration_since(last) < POLL_INTERVAL)
        {
            return false;
        }
        self.last_poll = Some(now);
        let next = load_snapshot(&self.options);
        if next == self.snapshot {
            false
        } else {
            self.snapshot = next;
            true
        }
    }
    pub fn reload(&mut self) -> bool {
        let next = load_snapshot(&self.options);
        if next == self.snapshot {
            false
        } else {
            self.snapshot = next;
            true
        }
    }
}

pub fn default_palette() -> BTreeMap<String, String> {
    [
        ("accent", "#FFD60A"),
        ("background", "#101315"),
        ("foreground", "#cacccc"),
        ("muted", "#707880"),
        ("lighter_background", "#272727"),
        ("red", "#ff5f56"),
        ("yellow", "#ffc163"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect()
}

fn valid_language(s: &str) -> bool {
    let mut p = s.split('-');
    let lang = p.next().unwrap_or("");
    (2..=3).contains(&lang.len())
        && lang.bytes().all(|b| b.is_ascii_lowercase())
        && p.next()
            .is_none_or(|r| r.len() == 2 && r.bytes().all(|b| b.is_ascii_alphabetic()))
        && p.next().is_none()
}
fn read_bounded(path: &Path) -> Option<Vec<u8>> {
    let m = fs::metadata(path).ok()?;
    if m.len() as usize > MAX_CONFIG_BYTES {
        return None;
    }
    let b = fs::read(path).ok()?;
    (b.len() <= MAX_CONFIG_BYTES).then_some(b)
}
fn parse_colors(bytes: &[u8], colors: &mut BTreeMap<String, String>) {
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        if !colors.contains_key(key) {
            continue;
        }
        let value = v.trim().trim_matches(|c| c == '\'' || c == '"');
        if value.len() == 7
            && value.starts_with('#')
            && value[1..].bytes().all(|b| b.is_ascii_hexdigit())
        {
            colors.insert(key.into(), value.into());
        }
    }
}
fn load_snapshot(o: &AppearanceOptions) -> AppearanceSnapshot {
    let mut colors = default_palette();
    if !o.demo {
        if let Some(b) = read_bounded(
            &o.home
                .join(".local/state/omarchy/current/theme/colors.toml"),
        ) {
            parse_colors(&b, &mut colors)
        }
    }
    let from_file = read_bounded(&o.home.join(".config/omarchy/language"))
        .and_then(|b| String::from_utf8(b).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| valid_language(s))
        .unwrap_or_else(|| "en".into());
    let language = o
        .language_override
        .as_deref()
        .filter(|s| valid_language(s))
        .unwrap_or(&from_file)
        .to_owned();
    let base = language.split('-').next().unwrap_or("en");
    let mut translations = BTreeMap::new();
    for code in ["en", base, language.as_str()] {
        let embedded = match code {
            "en" => Some(crate::i18n::ENGLISH),
            "da" => Some(crate::i18n::DANISH),
            _ => None,
        };
        if let Some(json) = embedded {
            merge_translations(&mut translations, json);
        }
        if let Some(bytes) = read_bounded(&o.catalog_dir.join(format!("{code}.json"))) {
            merge_translations(&mut translations, std::str::from_utf8(&bytes).unwrap_or(""));
        }
    }
    let registry = read_bounded(&o.omarchy_path.join("default/i18n/locales.json"))
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    let format_locale = registry
        .as_ref()
        .and_then(|r| r.get(&language))
        .and_then(|r| r.get("formatLocale"))
        .and_then(|v| v.as_str())
        .unwrap_or(&language)
        .to_owned();
    let accent = colors["accent"].clone();
    let accent_foreground = if luminance(&colors["background"]).abs_diff(luminance(&accent))
        >= luminance(&colors["foreground"]).abs_diff(luminance(&accent))
    {
        colors["background"].clone()
    } else {
        colors["foreground"].clone()
    };
    let text_direction = if matches!(
        language.split('-').next().unwrap_or(""),
        "ar" | "he" | "fa" | "ur" | "ps" | "sd" | "ug" | "yi"
    ) {
        TextDirection::RightToLeft
    } else {
        TextDirection::LeftToRight
    };
    AppearanceSnapshot {
        colors,
        accent,
        accent_foreground,
        language,
        format_locale,
        text_direction,
        translations,
    }
}
fn merge_translations(out: &mut BTreeMap<String, String>, json: &str) {
    if let Ok(serde_json::Value::Object(obj)) = serde_json::from_str(json) {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                out.insert(k, s.to_owned());
            }
        }
    }
}
fn luminance(s: &str) -> u32 {
    u32::from_str_radix(&s[1..3], 16).unwrap_or(0) * 299
        + u32::from_str_radix(&s[3..5], 16).unwrap_or(0) * 587
        + u32::from_str_radix(&s[5..7], 16).unwrap_or(0) * 114
}
