use crate::{base, fixtures};
use chrono::Utc;
use std::{path::Path, process::ExitCode};
use touchmeter_ui::Ui;

const HELP: &str = "Usage: touchmeter preview OUTPUT.png [--demo|--fixture FILE|--scenario NAME] [--menu|--ai|--functions|--providers|--volume] [--language CODE]\n";

pub fn run(args: &[String]) -> ExitCode {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{HELP}");
        return ExitCode::SUCCESS;
    }
    match render(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("touchmeter preview: {e}");
            ExitCode::FAILURE
        }
    }
}

fn render(args: &[String]) -> Result<(), String> {
    let output = args.first().ok_or_else(|| HELP.trim().to_string())?;
    let (mut demo, mut fixture, mut scenario, mut page, mut language) =
        (false, None, None, None, None);
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--demo" => demo = true,
            "--fixture" | "--scenario" | "--language" => {
                let key = args[i].as_str();
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| format!("{key} requires a value"))?
                    .clone();
                match key {
                    "--fixture" => fixture = Some(value),
                    "--scenario" => scenario = Some(value),
                    _ => language = Some(value),
                }
                i += 1;
            }
            "--menu" | "--ai" | "--functions" | "--providers" | "--volume" => {
                if page.replace(args[i][2..].to_string()).is_some() {
                    return Err("conflicting page overrides".into());
                }
            }
            x => return Err(format!("unknown option: {x}")),
        }
        i += 1;
    }
    if fixture.is_some() && scenario.is_some() {
        return Err("--fixture and --scenario are mutually exclusive".into());
    }
    if demo && fixture.is_some() {
        return Err("--demo and --fixture are mutually exclusive".into());
    }
    if language
        .as_deref()
        .is_some_and(|s| !crate::valid_language(s))
    {
        return Err("invalid language code".into());
    }
    // Preview is deliberately detached: only explicit fixture/scenario data or fictional demo data is used.
    let now = Utc::now();
    let mut model = base();
    let data = if let Some(path) = fixture {
        crate::testcli::read_fixture(Path::new(&path))?.ok_or("fixture not found")?
    } else if let Some(name) = scenario {
        fixtures::scenario(&name, now)?
    } else {
        fixtures::scenario("menu", now)?
    };
    data.project(&mut model, &data.selected, now);
    let appearance = crate::appearance::Appearance::new(crate::appearance::AppearanceOptions {
        demo,
        language_override: language,
        ..Default::default()
    });
    crate::apply_appearance(&mut model, appearance.snapshot());
    if let Some(p) = page {
        model.app = if p == "providers" { "ai" } else { &p }.into();
        model.picker = p == "providers";
    }
    model.testing = true;
    model.time = 0.0;
    let mut ui =
        Ui::new(model.clone()).map_err(|e| format!("Rune UI initialization failed: {e}"))?;
    let pixels = ui.render(model);
    if pixels.len() != 2008 * 60 * 4 {
        return Err(format!("invalid rendered frame length: {}", pixels.len()));
    }
    let file =
        std::fs::File::create(output).map_err(|e| format!("cannot create {}: {e}", output))?;
    let mut encoder = png::Encoder::new(file, 2008, 60);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(&pixels))
        .map_err(|e| format!("PNG write failed: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).into()).collect()
    }
    #[test]
    fn help_is_headless() {
        assert_eq!(run(&args(&["--help"])), ExitCode::SUCCESS);
    }
    #[test]
    fn rejects_conflicts_and_invalid_language_before_rendering() {
        assert!(
            render(&args(&["x.png", "--fixture", "x", "--demo"]))
                .unwrap_err()
                .contains("mutually exclusive")
        );
        assert!(
            render(&args(&["x.png", "--fixture", "x", "--scenario", "menu"]))
                .unwrap_err()
                .contains("mutually exclusive")
        );
        assert!(
            render(&args(&["x.png", "--language"]))
                .unwrap_err()
                .contains("requires a value")
        );
        assert!(
            render(&args(&["x.png", "--language", "bad_code"]))
                .unwrap_err()
                .contains("invalid language")
        );
        assert!(
            render(&args(&["x.png", "--wat"]))
                .unwrap_err()
                .contains("unknown option")
        );
    }
}
