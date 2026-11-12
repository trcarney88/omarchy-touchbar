//! Physical-runtime data coordinator. Construction is deliberately inert: filesystem
//! polling and collector work begins only after `tick`.
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};

pub use crate::live::{LiveBackend, PROVIDERS};
use crate::live_worker::LiveWorker;

const CONFIG_POLL: Duration = Duration::from_millis(500);
const FIXTURE_POLL: Duration = Duration::from_millis(500);

#[derive(Clone, Debug)]
pub struct DataPaths {
    pub state_home: PathBuf,
    pub config: PathBuf,
    /// Caller supplies testcli::runtime_path(false); simulator path is never consulted.
    pub fixture: PathBuf,
    pub collector: PathBuf,
}

/// Owns live-data/config/physical-fixture state. Call only after hardware runtime lock.
pub struct PhysicalData {
    paths: DataPaths,
    backend: LiveBackend,
    worker: LiveWorker,
    fixture_active: bool,
    fixture_data: Option<crate::fixtures::Fixture>,
    fixture_selected: Option<String>,
    fixture_last_check: Option<Instant>,
    config_last_check: Option<Instant>,
    config_minutes: u32,
    shutdown: bool,
    credential_context: Option<crate::credentials::CredentialContext>,
    probe_rx: Option<mpsc::Receiver<(u64, HashMap<String, bool>)>>,
    probe_generation: u64,
    probe_started: bool,
    appearance: Option<crate::appearance::Appearance>,
    volume_fraction: f32,
    volume_muted: bool,
    authorized: bool,
    fixture_dirty: bool,
}

impl PhysicalData {
    pub fn new(paths: DataPaths, enabled: HashMap<String, bool>) -> Self {
        let backend = LiveBackend::new(paths.state_home.clone(), enabled);
        let worker = LiveWorker::new(&backend, paths.collector.clone());
        Self {
            paths,
            backend,
            worker,
            fixture_active: false,
            fixture_data: None,
            fixture_selected: None,
            fixture_last_check: None,
            config_last_check: None,
            config_minutes: 30,
            shutdown: false,
            credential_context: None,
            probe_rx: None,
            probe_generation: 0,
            probe_started: false,
            appearance: None,
            volume_fraction: 0.65,
            volume_muted: false,
            authorized: false,
            fixture_dirty: false,
        }
    }

    pub fn with_credential_context(
        mut self,
        context: crate::credentials::CredentialContext,
    ) -> Self {
        self.credential_context = Some(context);
        self
    }
    pub fn with_appearance(mut self, options: crate::appearance::AppearanceOptions) -> Self {
        self.appearance = Some(crate::appearance::Appearance::new(options));
        self
    }
    /// Apply only palette/translations to a cloned model; stored runtime data is unchanged.
    pub fn render_model(&self, model: &touchmeter_ui::Model) -> touchmeter_ui::Model {
        let mut rendered = model.clone();
        if let Some(appearance) = &self.appearance {
            let s = appearance.snapshot();
            rendered.background = s.color("background").to_owned();
            rendered.foreground = s.color("foreground").to_owned();
            rendered.accent = s.accent.clone();
            rendered.muted = s.color("muted").to_owned();
            rendered.surface = s.color("lighter_background").to_owned();
            rendered.warning = s.color("yellow").to_owned();
            rendered.danger = s.color("red").to_owned();
            rendered.accent_foreground = s.accent_foreground.clone();
            rendered.status = s.status_label(&rendered.status);
            for limit in &mut rendered.limits {
                limit.countdown = s.countdown_label(&limit.countdown);
            }
        }
        rendered
    }
    /// Returns true when handled in test mode; never contacts wpctl.
    pub fn set_fixture_volume(&mut self, fraction: f32) -> bool {
        if !self.fixture_active {
            return false;
        }
        if !fraction.is_finite() {
            return true;
        }
        self.volume_fraction = fraction.clamp(0.0, 1.0);
        self.fixture_dirty = true;
        true
    }
    pub fn toggle_fixture_mute(&mut self) -> bool {
        if !self.fixture_active {
            return false;
        }
        self.volume_muted = !self.volume_muted;
        self.fixture_dirty = true;
        true
    }
    pub fn fixture_volume(&self) -> Option<(f32, bool)> {
        self.fixture_active
            .then_some((self.volume_fraction, self.volume_muted))
    }

    pub fn backend(&self) -> &LiveBackend {
        &self.backend
    }
    pub fn set_authorized(&mut self, enabled: bool) {
        if self.shutdown || self.authorized == enabled {
            return;
        }
        self.authorized = enabled;
        if enabled {
            self.worker.resume();
        } else {
            self.probe_generation = self.probe_generation.wrapping_add(1);
            self.worker.pause();
        }
    }
    pub fn fixture_active(&self) -> bool {
        self.fixture_active
    }
    pub fn fire_trigger_minutes(&self) -> u32 {
        self.config_minutes
    }

    /// Selects an enabled catalog provider immediately. Fixture mode uses fixture selection only.
    pub fn select(&mut self, id: &str) -> bool {
        if self.fixture_active {
            let Some(value) = self.fixture_data.as_ref() else {
                return false;
            };
            if !PROVIDERS.iter().any(|(key, _)| *key == id) {
                return false;
            }
            if !value.providers.get(id).is_some_and(|p| p.enabled) {
                return false;
            }
            self.fixture_selected = Some(id.to_owned());
            self.fixture_dirty = true;
            return true;
        }
        self.worker.select(&mut self.backend, id)
    }

    pub fn tick(
        &mut self,
        instant: Instant,
        wall: DateTime<Utc>,
        model: &mut touchmeter_ui::Model,
    ) -> bool {
        if self.shutdown {
            return false;
        }
        let before = (
            model.status.clone(),
            model.selected,
            model.fire,
            model.testing,
            model.app.clone(),
            model.picker,
            model.fire_trigger_minutes,
            model
                .providers
                .iter()
                .map(|p| (p.id.clone(), p.name.clone(), p.enabled))
                .collect::<Vec<_>>(),
            model
                .limits
                .iter()
                .map(|l| (l.label.clone(), l.fraction, l.countdown.clone()))
                .collect::<Vec<_>>(),
        );
        if self
            .fixture_last_check
            .is_none_or(|t| instant.saturating_duration_since(t) >= FIXTURE_POLL)
        {
            self.fixture_last_check = Some(instant);
            self.poll_fixture(model, wall);
        }
        let appearance_changed = self.appearance.as_mut().is_some_and(|a| a.poll(instant));
        if self
            .config_last_check
            .is_none_or(|t| instant.saturating_duration_since(t) >= CONFIG_POLL)
        {
            self.config_last_check = Some(instant);
            if let Ok(c) = crate::config::read(&self.paths.config) {
                self.config_minutes = c.fire_trigger_minutes as u32;
                self.fixture_dirty = true;
            }
        }
        if !self.fixture_active {
            if self.authorized {
                self.service_credential_probe(instant);
                if let Some(snapshot) = self.worker.tick(&mut self.backend, instant, wall).cloned()
                {
                    for provider in &mut model.providers {
                        provider.enabled = self
                            .backend
                            .enabled
                            .get(&provider.id)
                            .copied()
                            .unwrap_or(false);
                    }
                    snapshot.apply_to_model(model, self.config_minutes);
                }
            }
            model.fire_trigger_minutes = self.config_minutes as f32;
        }
        if self.fixture_active && self.fixture_dirty {
            self.apply_fixture(model, wall, false);
            self.fixture_dirty = false;
        }
        appearance_changed
            || before
                != (
                    model.status.clone(),
                    model.selected,
                    model.fire,
                    model.testing,
                    model.app.clone(),
                    model.picker,
                    model.fire_trigger_minutes,
                    model
                        .providers
                        .iter()
                        .map(|p| (p.id.clone(), p.name.clone(), p.enabled))
                        .collect::<Vec<_>>(),
                    model
                        .limits
                        .iter()
                        .map(|l| (l.label.clone(), l.fraction, l.countdown.clone()))
                        .collect::<Vec<_>>(),
                )
    }

    fn service_credential_probe(&mut self, instant: Instant) {
        if let Some(rx) = &self.probe_rx {
            if let Ok((generation, result)) = rx.try_recv() {
                self.probe_rx = None;
                self.probe_started = false;
                if !self.fixture_active && generation == self.backend.generation() {
                    self.worker.update_availability(&mut self.backend, result);
                }
            }
        }
        if !self.authorized || self.fixture_active || self.probe_rx.is_some() {
            return;
        }
        let Some(context) = self.credential_context.clone() else {
            return;
        };
        if !self.worker.credential_request_due(instant, false) {
            return;
        }
        let generation = self.backend.generation();
        let (tx, rx) = mpsc::channel();
        self.probe_rx = Some(rx);
        self.probe_generation = generation;
        self.probe_started = true;
        thread::spawn(move || {
            let result = context.availability().into_iter().collect();
            let _ = tx.send((generation, result));
        });
    }

    fn poll_fixture(&mut self, model: &mut touchmeter_ui::Model, wall: DateTime<Utc>) {
        match crate::testcli::read_fixture(&self.paths.fixture) {
            Ok(Some(fixture)) => {
                let changed = self.fixture_data.as_ref() != Some(&fixture);
                if !self.fixture_active || changed {
                    self.worker.set_test_mode(&mut self.backend, true);
                    // Credential probes are read-only but not cooperatively cancellable. Their
                    // result is tagged and discarded on mode/generation changes.
                    self.fixture_active = true;
                    self.probe_generation = self.backend.generation();
                }
                if changed {
                    self.fixture_data = Some(fixture);
                    self.fixture_selected = None;
                    self.volume_fraction = 0.65;
                    self.volume_muted = false;
                    model.volume_available = true;
                }
                self.fixture_active = true;
                self.apply_fixture(model, wall, changed);
            }
            Ok(None) if self.fixture_active => {
                self.fixture_active = false;
                self.fixture_data = None;
                self.fixture_selected = None;
                self.worker.set_test_mode(&mut self.backend, false);
                model.testing = false;
                if let Some(i) = model
                    .providers
                    .iter()
                    .position(|p| p.id == self.backend.selected)
                {
                    model.selected = i;
                }
                model.app = "menu".into();
            }
            Err(_) => {} // Keep last good fixture and pause live work.
            _ => {}
        }
    }
    fn apply_fixture(
        &mut self,
        model: &mut touchmeter_ui::Model,
        wall: DateTime<Utc>,
        apply_navigation: bool,
    ) {
        let Some(fixture) = self.fixture_data.as_ref() else {
            return;
        };
        let selected = self
            .fixture_selected
            .as_deref()
            .unwrap_or(&fixture.selected)
            .to_owned();
        let controls = (
            model.volume,
            model.volume_muted,
            model.volume_available,
            model.app.clone(),
            model.picker,
            model.selected,
        );
        model.fire_trigger_minutes = self.config_minutes as f32;
        fixture.project(model, &selected, wall);
        model.testing = true;
        if let Some(v) = &fixture.volume {
            model.volume = (v.percent.unwrap_or(65.) / 100.) as f32;
            model.volume_muted = v.muted.unwrap_or(false);
            model.volume_available = v.available.unwrap_or(true);
        } else {
            model.volume = self.volume_fraction;
            model.volume_muted = self.volume_muted;
        }
        if !apply_navigation {
            model.app = controls.3;
            model.picker = controls.4;
        }
    }
    pub fn shutdown(&mut self) {
        if !self.shutdown {
            self.shutdown = true;
            self.probe_generation = self.probe_generation.wrapping_add(1);
            self.probe_rx = None;
            self.worker.shutdown();
        }
    }
}
