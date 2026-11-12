#![cfg(all(target_os = "linux", feature = "linux-hardware"))]

#[path = "../src/controls.rs"]
mod controls;

#[path = "../src/appearance.rs"]
mod appearance;
#[path = "../src/burnrate.rs"]
mod burnrate;
#[path = "../src/config.rs"]
mod config;
#[path = "../src/credentials.rs"]
mod credentials;
#[path = "../src/linux/data.rs"]
mod data;
#[path = "../src/fixtures.rs"]
mod fixtures;
#[path = "../src/i18n.rs"]
mod i18n;
#[path = "../src/jobs.rs"]
mod jobs;
#[path = "../src/live.rs"]
mod live;
#[path = "../src/live_worker.rs"]
mod live_worker;
#[path = "../src/testcli.rs"]
mod testcli;

fn base() -> touchmeter_ui::Model {
    let mut model = touchmeter_ui::Model::default();
    model.providers = fixtures::PROVIDER_IDS
        .iter()
        .map(|id| touchmeter_ui::Provider {
            id: (*id).into(),
            name: (*id).into(),
            enabled: false,
        })
        .collect();
    model.fire_trigger_minutes = 30.;
    model
}

#[test]
fn construction_is_inert_and_fixture_is_physical_path_only() {
    use std::{collections::HashMap, fs, path::PathBuf, time::Instant};
    let root =
        std::env::temp_dir().join(format!("touchmeter-physical-data-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let physical = root.join("physical.json");
    fs::write(&physical, fixture("claude", 0.5)).unwrap();
    let mut svc = data::PhysicalData::new(
        data::DataPaths {
            state_home: root.clone(),
            config: root.join("config.json"),
            fixture: physical,
            collector: PathBuf::from("fake-collector"),
        },
        HashMap::new(),
    );
    assert!(!svc.fixture_active());
    let mut model = base();
    let now = Instant::now();
    svc.tick(now, chrono::Utc::now(), &mut model);
    assert!(svc.fixture_active());
    svc.shutdown();
    fs::remove_dir_all(root).unwrap();
}

fn fixture(selected: &str, percent: f64) -> String {
    let mut f = fixtures::scenario("fire", chrono::Utc::now()).unwrap();
    f.selected = selected.into();
    let p = f.providers.get_mut(selected).unwrap();
    p.limits[0].percent = percent;
    serde_json::to_string(&f).unwrap()
}

#[test]
fn physical_fixture_projects_selection_and_clear_restores_live_mode() {
    use std::{
        collections::HashMap,
        fs,
        path::PathBuf,
        time::{Duration, Instant},
    };
    let root = std::env::temp_dir().join(format!("touchmeter-fixture-data-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("physical.json");
    fs::write(&path, fixture("claude", 00.72)).unwrap();
    let mut svc = data::PhysicalData::new(
        data::DataPaths {
            state_home: root.clone(),
            config: root.join("config"),
            fixture: path.clone(),
            collector: PathBuf::from("fake-collector"),
        },
        HashMap::new(),
    );
    let mut model = base();
    let now = Instant::now();
    svc.tick(now, chrono::Utc::now(), &mut model);
    assert!(svc.fixture_active());
    assert!(model.testing);
    assert_eq!(model.providers[model.selected].id, "claude");
    assert_eq!(model.limits[0].fraction, 00.72);
    fs::remove_file(path).unwrap();
    svc.tick(
        now + Duration::from_millis(501),
        chrono::Utc::now(),
        &mut model,
    );
    assert!(!svc.fixture_active());
    assert!(!model.testing);
    svc.shutdown();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fixture_corruption_retains_last_valid_data_and_volume_mutations_are_in_memory() {
    use std::{
        collections::HashMap,
        fs,
        path::PathBuf,
        time::{Duration, Instant},
    };
    let root =
        std::env::temp_dir().join(format!("touchmeter-fixture-retain-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("physical.json");
    fs::write(&path, fixture("claude", 00.72)).unwrap();
    let mut svc = data::PhysicalData::new(
        data::DataPaths {
            state_home: root.clone(),
            config: root.join("cfg"),
            fixture: path.clone(),
            collector: PathBuf::from("does-not-exist"),
        },
        HashMap::new(),
    );
    let mut model = base();
    let now = Instant::now();
    svc.tick(now, chrono::Utc::now(), &mut model);
    assert!(svc.set_fixture_volume(0.8));
    assert!(svc.toggle_fixture_mute());
    assert_eq!(svc.fixture_volume(), Some((0.8, true)));
    fs::write(path, b"{").unwrap();
    svc.tick(
        now + Duration::from_millis(501),
        chrono::Utc::now(),
        &mut model,
    );
    assert!(svc.fixture_active());
    assert_eq!(model.providers[model.selected].id, "claude");
    assert_eq!(model.limits[0].fraction, 0.72);
    svc.shutdown();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn configuration_poll_preserves_navigation_and_live_worker_uses_fake_program() {
    use std::{
        collections::HashMap,
        fs,
        path::PathBuf,
        time::{Duration, Instant},
    };
    let root = std::env::temp_dir().join(format!("touchmeter-config-data-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let cfg = root.join("config.json");
    fs::write(&cfg, r#"{"version":1,"fire":{"triggerMinutes":47}}"#).unwrap();
    let script = root.join("collector");
    fs::write(&script, "#!/bin/sh\nexit 0\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut svc = data::PhysicalData::new(
        data::DataPaths {
            state_home: root.clone(),
            config: cfg,
            fixture: root.join("missing-physical"),
            collector: script,
        },
        HashMap::new(),
    );
    let mut model = touchmeter_ui::Model::default();
    model.app = "volume".into();
    model.volume = 0.33;
    let now = Instant::now();
    svc.tick(now, chrono::Utc::now(), &mut model);
    svc.tick(
        now + Duration::from_millis(501),
        chrono::Utc::now(),
        &mut model,
    );
    assert_eq!(svc.fire_trigger_minutes(), 47);
    assert_eq!(model.app, "volume");
    assert_eq!(model.volume, 0.33);
    svc.shutdown();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_snapshot_projects_the_complete_constructor_availability_map() {
    use std::{collections::HashMap, fs, path::PathBuf, time::Instant};
    let root = std::env::temp_dir().join(format!("touchmeter-availability-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let record = root.join("omarchy/agents/usage/codex.json");
    fs::create_dir_all(record.parent().unwrap()).unwrap();
    fs::write(
        &record,
        r#"{"ready":true,"limits":[{"label":"5h window","percent":0.31,"resetsAt":""}]}"#,
    )
    .unwrap();
    let mut available = HashMap::new();
    available.insert("codex".to_owned(), true);
    available.insert("claude".to_owned(), true);
    let mut service = data::PhysicalData::new(
        data::DataPaths {
            state_home: root.clone(),
            config: root.join("missing-config"),
            fixture: root.join("no-fixture"),
            collector: PathBuf::from("/bin/true"),
        },
        available,
    );
    service.set_authorized(true);
    let mut model = base();
    service.tick(Instant::now(), chrono::Utc::now(), &mut model);
    assert!(
        model
            .providers
            .iter()
            .find(|p| p.id == "codex")
            .unwrap()
            .enabled
    );
    assert!(
        model
            .providers
            .iter()
            .find(|p| p.id == "claude")
            .unwrap()
            .enabled
    );
    assert!(
        model
            .providers
            .iter()
            .filter(|p| p.id != "codex" && p.id != "claude")
            .all(|p| !p.enabled)
    );

    service.shutdown();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn fixture_poll_preserves_interactive_navigation_and_reports_page_changes() {
    use std::{
        collections::HashMap,
        fs,
        path::PathBuf,
        time::{Duration, Instant},
    };
    let root = std::env::temp_dir().join(format!("touchmeter-navigation-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("physical.json");
    let mut f = fixtures::scenario("fire", chrono::Utc::now()).unwrap();
    f.mode = "providers".into();
    f.providers.values_mut().for_each(|p| p.enabled = true);
    fs::write(&path, serde_json::to_vec(&f).unwrap()).unwrap();
    let mut svc = data::PhysicalData::new(
        data::DataPaths {
            state_home: root.clone(),
            config: root.join("config"),
            fixture: path.clone(),
            collector: PathBuf::from("inert-collector"),
        },
        HashMap::new(),
    );
    let mut model = base();
    let now = Instant::now();
    assert!(svc.tick(now, chrono::Utc::now(), &mut model));
    assert_eq!(model.app, "ai");
    assert!(model.picker);
    model.picker = false;
    model.app = "menu".into();
    assert!(!svc.tick(
        now + Duration::from_millis(501),
        chrono::Utc::now(),
        &mut model
    ));
    assert_eq!(model.app, "menu");
    // A fixture source change reapplies its navigation, even with identical provider data.
    f.mode = "usage".into();
    fs::write(&path, serde_json::to_vec(&f).unwrap()).unwrap();
    assert!(svc.tick(
        now + Duration::from_millis(1002),
        chrono::Utc::now(),
        &mut model
    ));
    assert_eq!(model.app, "ai");
    f.mode = "providers".into();
    fs::write(&path, serde_json::to_vec(&f).unwrap()).unwrap();
    assert!(svc.tick(
        now + Duration::from_millis(1503),
        chrono::Utc::now(),
        &mut model
    ));
    assert_eq!(model.app, "ai");
    assert!(model.picker);
    svc.shutdown();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn live_fixture_picker_selection_and_armed_menu_contact_survive_polls() {
    use controls::{Controller, Effect, InputEvent};
    use std::{
        collections::HashMap,
        fs,
        path::PathBuf,
        time::{Duration, Instant},
    };
    let root = std::env::temp_dir().join(format!(
        "touchmeter-picker-interaction-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let path = root.join("fixture.json");
    let mut f = fixtures::scenario("all-providers", chrono::Utc::now()).unwrap();
    fs::write(&path, serde_json::to_vec(&f).unwrap()).unwrap();
    let mut data = data::PhysicalData::new(
        data::DataPaths {
            state_home: root.clone(),
            config: root.join("config"),
            fixture: path.clone(),
            collector: PathBuf::from("inert-collector"),
        },
        HashMap::new(),
    );
    let mut model = base();
    model.apps = controls::catalog();
    let mut controller = Controller::default();
    let now = Instant::now();
    assert!(data.tick(now, chrono::Utc::now(), &mut model));
    assert!(model.picker);

    controller.handle(InputEvent::Begin(150., 20.), &mut model);
    let effects = controller.handle(InputEvent::End, &mut model);
    assert!(effects.contains(&Effect::SelectProvider(0)));
    assert!(data.select(&model.providers[model.selected].id));
    assert!(data.tick(
        now + Duration::from_millis(150),
        chrono::Utc::now(),
        &mut model
    ));
    assert_eq!(model.providers[model.selected].id, "codex");
    assert!(!model.picker);
    for ms in [501, 1002] {
        data.tick(
            now + Duration::from_millis(ms),
            chrono::Utc::now(),
            &mut model,
        );
        assert_eq!(model.providers[model.selected].id, "codex");
        assert!(!model.picker);
    }

    controller.handle(InputEvent::Begin(1900., 20.), &mut model);
    controller.handle(InputEvent::End, &mut model);
    assert_eq!(model.app, "menu");
    data.tick(
        now + Duration::from_millis(1503),
        chrono::Utc::now(),
        &mut model,
    );
    assert_eq!(model.app, "menu");
    controller.handle(InputEvent::Begin(700., 20.), &mut model);
    data.tick(
        now + Duration::from_millis(2004),
        chrono::Utc::now(),
        &mut model,
    );
    assert!(controller.cancel_if_view_changed(&mut model).is_none());
    assert!(
        controller
            .handle(InputEvent::End, &mut model)
            .contains(&Effect::PageChanged)
    );
    assert_eq!(model.app, "functions");
    data.shutdown();
    fs::remove_dir_all(root).unwrap();
}
