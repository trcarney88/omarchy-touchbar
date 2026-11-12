//! Resolves launch fixtures and runtime simulator overrides without touching live state.
use crate::{
    fixtures::{Fixture, scenario},
    testcli,
};
use chrono::{DateTime, Utc};
use std::{path::PathBuf, time::Duration};
use touchmeter_ui::Model;

pub struct SimData {
    base: Fixture,
    override_path: PathBuf,
    override_data: Option<Fixture>,
    selected: String,
    applied: bool,
    base_path: Option<PathBuf>,
    last_visible: Option<Fixture>,
    fallback: Option<Fixture>,
    last_signature: Option<String>,
    explicit_mode: Option<String>,
}
impl SimData {
    pub fn new(base: Fixture, override_path: PathBuf) -> Self {
        let selected = base.selected.clone();
        Self {
            base,
            override_path,
            override_data: None,
            selected,
            applied: false,
            base_path: None,
            last_visible: None,
            fallback: None,
            last_signature: None,
            explicit_mode: None,
        }
    }
    pub fn with_base_path(mut self, path: Option<PathBuf>) -> Self {
        self.base_path = path;
        self
    }
    pub fn with_explicit_mode(mut self, mode: Option<String>) -> Self {
        self.explicit_mode = mode;
        self
    }
    pub fn poll(&mut self, model: &mut Model, now: DateTime<Utc>) -> bool {
        let previous = self.override_data.clone();
        let old_base = self.base.clone();
        if let Some(path) = &self.base_path {
            if let Ok(Some(f)) = testcli::read_fixture(path) {
                self.base = f;
                if let Some(mode) = &self.explicit_mode {
                    self.base.mode = mode.clone();
                }
            } else if testcli::read_fixture(path).is_err() || !path.exists() {
                if self.fallback.is_none() {
                    self.fallback = Some(scenario("menu", now).expect("builtin menu fixture"));
                }
                self.base = self.fallback.clone().unwrap();
            }
        }
        self.override_data = match testcli::read_fixture(&self.override_path) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("touchmeter: ignoring simulator override: {e}");
                None
            }
        };
        let effective_changed = previous != self.override_data
            || (self.override_data.is_none() && old_base != self.base);
        let changed = effective_changed || !self.applied;
        let effective = self.override_data.as_ref().unwrap_or(&self.base);
        if changed {
            if previous != self.override_data || self.override_data.is_none() {
                self.selected = effective.selected.clone();
            }
            let (app, picker) = match effective.mode.as_str() {
                "menu" => ("menu", false),
                "providers" => ("ai", true),
                "functions" => ("functions", false),
                "volume" => ("volume", false),
                _ => ("ai", false),
            };
            model.app = app.into();
            model.picker = picker;
            if previous != self.override_data {
                model.volume = 0.7;
                model.volume_muted = false;
                model.volume_available = true;
            }
            self.applied = true;
        }
        let app = model.app.clone();
        let picker = model.picker;
        let volume = model.volume;
        let muted = model.volume_muted;
        let available = model.volume_available;
        effective.project(model, &self.selected, now);
        if !changed {
            model.app = app;
            model.picker = picker;
            model.volume = volume;
            model.volume_muted = muted;
            model.volume_available = available;
        }
        let after = visible_signature(model);
        let did_change = changed
            || self.last_visible.as_ref() != Some(effective)
            || self.last_signature.as_ref() != Some(&after);
        self.last_signature = Some(after);
        self.last_visible = Some(effective.clone());
        did_change
    }
    pub fn select(&mut self, model: &mut Model, index: usize, now: DateTime<Utc>) {
        if let Some(p) = model.providers.get(index).filter(|p| p.enabled) {
            self.selected = p.id.clone();
            let app = model.app.clone();
            let volume = model.volume;
            let muted = model.volume_muted;
            let available = model.volume_available;
            self.override_data
                .as_ref()
                .unwrap_or(&self.base)
                .project(model, &self.selected, now);
            model.app = app;
            model.picker = false;
            model.volume = volume;
            model.volume_muted = muted;
            model.volume_available = available;
        }
    }
}
pub fn launch(
    path: Option<&std::path::Path>,
    scenario_name: Option<&str>,
    now: DateTime<Utc>,
) -> Result<Fixture, String> {
    if let Some(p) = path {
        testcli::read_fixture(p)?.ok_or_else(|| format!("Fixture not found: {}", p.display()))
    } else if let Some(n) = scenario_name {
        scenario(n, now)
    } else {
        scenario("menu", now)
    }
}
pub const POLL_INTERVAL: Duration = Duration::from_millis(500);

fn visible_signature(model: &Model) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}",
        model.selected,
        model.status,
        model.fire,
        model.stale,
        model.burn_minutes,
        model
            .limits
            .iter()
            .map(|l| format!("{}:{}:{}", l.label, l.fraction, l.countdown))
            .collect::<Vec<_>>()
            .join(","),
        model
            .providers
            .iter()
            .map(|p| format!("{}:{}", p.id, p.enabled))
            .collect::<Vec<_>>()
            .join(","),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::{fs, os::unix::fs::PermissionsExt};
    fn setup() -> (PathBuf, PathBuf) {
        let d = std::env::temp_dir().join(format!(
            "touchmeter-simdata-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap()
        ));
        fs::create_dir(&d).unwrap();
        fs::set_permissions(&d, fs::Permissions::from_mode(0o700)).unwrap();
        (d.join("override.json"), d)
    }
    fn write(p: &PathBuf, f: &Fixture) {
        fs::write(p, serde_json::to_vec(f).unwrap()).unwrap()
    }
    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 11, 12, 0, 0).unwrap()
    }
    #[test]
    fn startup_override_change_and_clear_restore_base() {
        let (p, d) = setup();
        let base = scenario("menu", now()).unwrap();
        let override_f = scenario("fire", now()).unwrap();
        write(&p, &override_f);
        let mut s = SimData::new(base.clone(), p.clone());
        let mut m = crate::base();
        assert!(s.poll(&mut m, now()));
        assert_eq!(m.app, "ai");
        let newer = scenario("volume-muted", now()).unwrap();
        write(&p, &newer);
        assert!(s.poll(&mut m, now()));
        assert_eq!(m.app, "volume");
        fs::remove_file(&p).unwrap();
        s.poll(&mut m, now());
        assert_eq!(m.app, "menu");
        fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn unchanged_poll_preserves_navigation_volume_and_selection() {
        let (p, d) = setup();
        let f = scenario("fire", now()).unwrap();
        write(&p, &f);
        let mut s = SimData::new(scenario("menu", now()).unwrap(), p.clone());
        let mut m = crate::base();
        s.poll(&mut m, now());
        m.app = "functions".into();
        m.volume = 0.31;
        m.volume_muted = true;
        s.poll(&mut m, now());
        assert_eq!(m.app, "functions");
        assert_eq!(m.volume, 0.31);
        assert!(m.volume_muted);
        fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn provider_selection_projects_provider_specific_data() {
        let (p, d) = setup();
        let mut f = scenario("fire", now()).unwrap();
        f.providers.get_mut("codex").unwrap().limits[0].percent = 0.22;
        f.providers.get_mut("claude").unwrap().limits[0].percent = 0.88;
        write(&p, &f);
        let mut s = SimData::new(f.clone(), p.clone());
        let mut m = crate::base();
        s.poll(&mut m, now());
        s.select(&mut m, 0, now());
        assert_eq!(m.limits[0].fraction, 0.22);
        s.select(&mut m, 1, now());
        assert_eq!(m.limits[0].fraction, 0.88);
        fs::remove_dir_all(d).unwrap();
    }
    #[test]
    fn projection_clock_advances_without_resetting_controls() {
        let (p, d) = setup();
        let mut f = scenario("fire", now()).unwrap();
        f.now = None;
        write(&p, &f);
        let mut s = SimData::new(f, p.clone());
        let mut m = crate::base();
        s.poll(&mut m, now());
        let before = m.limits[0].countdown.clone();
        m.app = "functions".into();
        m.volume = 0.42;
        assert!(s.poll(&mut m, now() + chrono::Duration::minutes(30)));
        assert!(!s.poll(&mut m, now() + chrono::Duration::minutes(30)));
        assert_eq!(m.app, "functions");
        assert_eq!(m.volume, 0.42);
        assert_ne!(m.limits[0].countdown, before);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn replacement_base_survives_override_until_clear() {
        let (p, d) = setup();
        let b = d.join("base.json");
        let first = scenario("menu", now()).unwrap();
        let mut second = scenario("menu", now()).unwrap();
        second.mode = "functions".into();
        write(&b, &first);
        write(&p, &scenario("fire", now()).unwrap());
        let mut s = SimData::new(first, p.clone()).with_base_path(Some(b.clone()));
        let mut m = crate::base();
        s.poll(&mut m, now());
        write(&b, &second);
        s.poll(&mut m, now());
        assert_eq!(m.app, "ai");
        fs::remove_file(&p).unwrap();
        s.poll(&mut m, now());
        assert_eq!(m.app, "functions");
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn invalid_or_deleted_base_falls_back_to_menu_but_override_wins() {
        let (p, d) = setup();
        let b = d.join("base.json");
        let initial = scenario("volume", now()).unwrap();
        write(&b, &initial);
        write(&p, &scenario("fire", now()).unwrap());
        let mut s = SimData::new(initial, p.clone()).with_base_path(Some(b.clone()));
        let mut m = crate::base();
        s.poll(&mut m, now());
        fs::write(&b, b"bad").unwrap();
        s.poll(&mut m, now());
        assert_eq!(m.app, "ai");
        fs::remove_file(&p).unwrap();
        s.poll(&mut m, now());
        assert_eq!(m.app, "menu");
        write(&b, &scenario("volume", now()).unwrap());
        s.poll(&mut m, now());
        fs::remove_file(&b).unwrap();
        s.poll(&mut m, now());
        assert_eq!(m.app, "menu");
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn hidden_base_change_does_not_reset_override_interaction() {
        let (p, d) = setup();
        let b = d.join("base.json");
        let first = scenario("menu", now()).unwrap();
        let mut second = first.clone();
        second.mode = "volume".into();
        write(&b, &first);
        write(&p, &scenario("fire", now()).unwrap());
        let mut s = SimData::new(first, p.clone()).with_base_path(Some(b.clone()));
        let mut m = crate::base();
        s.poll(&mut m, now());
        m.app = "functions".into();
        m.volume = 0.28;
        write(&b, &second);
        assert!(!s.poll(&mut m, now()));
        assert_eq!(m.app, "functions");
        assert_eq!(m.volume, 0.28);
        fs::remove_file(&p).unwrap();
        s.poll(&mut m, now());
        assert_eq!(m.app, "volume");
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn explicit_launch_page_is_base_page_through_startup_override_and_clear() {
        let (p, d) = setup();
        let b = d.join("base.json");
        let base = scenario("fire", now()).unwrap();
        write(&b, &base);
        write(&p, &scenario("volume-muted", now()).unwrap());
        let mut s = SimData::new(base.clone(), p.clone())
            .with_base_path(Some(b.clone()))
            .with_explicit_mode(Some("functions".into()));
        let mut m = crate::base();
        assert!(s.poll(&mut m, now()));
        assert_eq!(m.app, "volume");
        fs::remove_file(&p).unwrap();
        s.poll(&mut m, now());
        assert_eq!(m.app, "functions");
        let mut replacement = scenario("high-usage", now()).unwrap();
        replacement.mode = "usage".into();
        replacement.providers.get_mut("claude").unwrap().limits[0].percent = 0.97;
        write(&b, &replacement);
        s.poll(&mut m, now());
        assert_eq!(m.app, "functions");
        assert_eq!(m.limits[0].fraction, 0.97);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn repeated_invalid_base_keeps_fallback_stable_and_user_navigation() {
        let (p, d) = setup();
        let b = d.join("base.json");
        write(&b, &scenario("volume", now()).unwrap());
        let mut s =
            SimData::new(scenario("volume", now()).unwrap(), p).with_base_path(Some(b.clone()));
        let mut m = crate::base();
        s.poll(&mut m, now());
        fs::remove_file(&b).unwrap();
        assert!(s.poll(&mut m, now()));
        m.app = "functions".into();
        m.volume = 0.24;
        assert!(!s.poll(&mut m, now()));
        assert_eq!(m.app, "functions");
        assert_eq!(m.volume, 0.24);
        fs::remove_dir_all(d).unwrap();
    }
}
