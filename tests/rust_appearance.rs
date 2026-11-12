#[path = "../src/appearance.rs"]
mod appearance;
#[path = "../src/i18n.rs"]
mod i18n;

use appearance::*;

#[test]
fn palette_demo_and_translation_fallback() {
    let colors = default_palette();
    assert_eq!(colors.len(), 7);
    assert_eq!(
        i18n::merge_catalogs(&[r#"{"Hello":"Hello"}"#, r#"{"Hello":"Hej"}"#])["Hello"],
        "Hej"
    );
    let dir = std::env::temp_dir().join(format!("touchmeter-appearance-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("en.json"), r#"{"Hello":"Hello","Count":"Count"}"#).unwrap();
    std::fs::write(dir.join("da.json"), r#"{"Hello":"Hej"}"#).unwrap();
    let opts = AppearanceOptions {
        demo: true,
        language_override: Some("da".into()),
        home: dir.join("home"),
        omarchy_path: dir.join("omarchy"),
        catalog_dir: dir.clone(),
    };
    let s = Appearance::new(opts).snapshot().clone();
    assert_eq!(s.language, "da");
    assert_eq!(s.translate("Hello"), "Hej");
    assert_eq!(s.translate("Count"), "Count");
    assert_eq!(s.colors, default_palette());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn atomic_theme_replacement_is_seen_on_poll() {
    let dir = std::env::temp_dir().join(format!("touchmeter-poll-{}", std::process::id()));
    let theme = dir.join("home/.local/state/omarchy/current/theme");
    std::fs::create_dir_all(&theme).unwrap();
    let opts = AppearanceOptions {
        demo: false,
        language_override: None,
        home: dir.join("home"),
        omarchy_path: dir.join("none"),
        catalog_dir: dir.join("catalog"),
    };
    let mut a = Appearance::new(opts);
    std::fs::write(theme.join("new"), "accent = '#123456'\n").unwrap();
    std::fs::rename(theme.join("new"), theme.join("colors.toml")).unwrap();
    let now = std::time::Instant::now();
    assert!(a.poll(now));
    assert_eq!(a.snapshot().accent, "#123456");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn language_file_and_rtl_metadata_are_resolved() {
    let dir = std::env::temp_dir().join(format!("touchmeter-language-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("home/.config/omarchy")).unwrap();
    std::fs::write(dir.join("home/.config/omarchy/language"), "ar-EG\n").unwrap();
    let opts = AppearanceOptions {
        demo: true,
        language_override: None,
        home: dir.join("home"),
        omarchy_path: dir.join("none"),
        catalog_dir: dir.join("catalog"),
    };
    let snapshot = Appearance::new(opts).snapshot().clone();
    assert_eq!(snapshot.language, "ar-EG");
    assert_eq!(snapshot.text_direction, TextDirection::RightToLeft);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn embedded_danish_translates_projected_labels_and_dynamic_text() {
    let dir = std::env::temp_dir().join(format!("touchmeter-da-{}", std::process::id()));
    let opts = AppearanceOptions {
        demo: true,
        language_override: Some("da".into()),
        home: dir.join("home"),
        omarchy_path: dir.join("none"),
        catalog_dir: dir.join("missing-catalog"),
    };
    let s = Appearance::new(opts).snapshot().clone();
    assert_eq!(s.basic_label("5 HOUR"), "5 TIMER");
    assert_eq!(s.basic_label("WEEKLY"), "UGENTLIG");
    assert_eq!(s.countdown_label("2h 14m"), "2 t 14 min");
    assert_eq!(s.countdown_label("3d 8h"), "3 d 8 t");
    assert_eq!(
        s.status_label("TEST · Sign-in required"),
        "TEST · Log ind påkrævet"
    );
    assert_eq!(
        s.status_label("TEST · ~20m to limit"),
        "TEST · 20 min. til grænse"
    );
    assert_eq!(
        s.status_label("TEST · Simulated unavailable limits"),
        "TEST · Simulerede grænser ikke tilgængelige"
    );
    assert_eq!(s.countdown_label("reset due"), "nulstilling nu");
    assert_eq!(s.basic_label("unrecognized"), "unrecognized");
    assert_eq!(s.basic_label("UGENTLIG"), "UGENTLIG");
}
