#[path = "../src/controls.rs"]
mod controls;
use controls::{Controller, Effect, InputEvent};
use touchmeter_ui::{Model, Provider};
fn model(app: &str) -> Model {
    let mut m = Model {
        app: app.into(),
        ..Default::default()
    };
    m.apps = controls::catalog();
    m.providers = vec![
        Provider {
            id: "a".into(),
            enabled: true,
            ..Default::default()
        },
        Provider {
            id: "b".into(),
            enabled: false,
            ..Default::default()
        },
    ];
    m.volume_available = true;
    m
}
#[test]
fn launches_back_and_cancel_keys() {
    let (mut c, mut m) = (Controller::default(), model("menu"));
    c.handle(InputEvent::Begin(700., 20.), &mut m);
    c.handle(InputEvent::End, &mut m);
    assert_eq!(m.app, "functions");
    let e = c.handle(InputEvent::Begin(30., 20.), &mut m);
    assert!(e.contains(&Effect::FunctionKey {
        number: 1,
        pressed: true
    }));
    let e = c.handle(InputEvent::Cancel, &mut m);
    assert!(e.contains(&Effect::FunctionKey {
        number: 1,
        pressed: false
    }));
    assert_eq!(m.pressed, 0);
}
#[test]
fn volume_capture_and_sleep_wake() {
    let (mut c, mut m) = (Controller::default(), model("volume"));
    c.handle(InputEvent::Begin(200., 20.), &mut m);
    assert!(matches!(
        c.handle(InputEvent::Move(800., 30.), &mut m).last(),
        Some(Effect::SetVolume(_))
    ));
    c.handle(InputEvent::Cancel, &mut m);
    c.set_sleeping(true, &mut m);
    let e = c.handle(InputEvent::Begin(50., 20.), &mut m);
    assert_eq!(e, vec![Effect::Activity]);
}
#[test]
fn provider_picker_disabled_and_actions() {
    let (mut c, mut m) = (Controller::default(), model("ai"));
    c.handle(InputEvent::Begin(30., 20.), &mut m);
    c.handle(InputEvent::End, &mut m);
    assert!(m.picker);
    c.handle(InputEvent::Begin(280., 20.), &mut m);
    c.handle(InputEvent::End, &mut m);
    assert!(m.picker);
    c.handle(InputEvent::Begin(150., 20.), &mut m);
    c.handle(InputEvent::End, &mut m);
    assert_eq!(m.selected, 0);
}

#[test]
fn release_revalidates_captured_provider_against_current_view() {
    let (mut c, mut m) = (Controller::default(), model("ai"));
    m.picker = true;
    c.handle(InputEvent::Begin(150., 20.), &mut m);
    m.picker = false;
    assert!(
        !c.handle(InputEvent::End, &mut m)
            .contains(&Effect::SelectProvider(0))
    );
    assert_eq!(m.selected, 0);

    m.picker = true;
    c.handle(InputEvent::Begin(150., 20.), &mut m);
    m.app = "volume".into();
    assert!(
        !c.handle(InputEvent::End, &mut m)
            .contains(&Effect::SelectProvider(0))
    );
    assert_eq!(m.app, "volume");

    m.app = "ai".into();
    c.handle(InputEvent::Begin(150., 20.), &mut m);
    m.providers[0].enabled = false;
    assert!(
        !c.handle(InputEvent::End, &mut m)
            .contains(&Effect::SelectProvider(0))
    );
    assert_eq!(m.selected, 0);
}

#[test]
fn release_does_not_apply_app_action_after_external_page_change() {
    let (mut c, mut m) = (Controller::default(), model("menu"));
    c.handle(InputEvent::Begin(700., 20.), &mut m);
    m.app = "volume".into();
    assert!(c.handle(InputEvent::End, &mut m).is_empty());
    assert_eq!(m.app, "volume");
}

#[test]
fn view_change_during_contact_cancels_and_releases_function_key() {
    let (mut c, mut m) = (Controller::default(), model("functions"));
    let down = c.handle(InputEvent::Begin(12. + 1864. / 24., 20.), &mut m);
    assert!(down.contains(&Effect::FunctionKey {
        number: 1,
        pressed: true
    }));
    m.app = "volume".into();
    let effects = c.cancel_if_view_changed(&mut m).unwrap();
    assert!(effects.contains(&Effect::FunctionKey {
        number: 1,
        pressed: false
    }));
    assert_eq!(m.pressed, 0);
    assert!(c.handle(InputEvent::End, &mut m).is_empty());
}
