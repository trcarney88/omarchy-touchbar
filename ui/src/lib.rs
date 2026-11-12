//! Rune renders offscreen; the existing unprivileged host owns the DRM lease and input safety.
use rune::render::{Gpu, OFFSCREEN_FORMAT, Offscreen, Renderer};
use rune::text::Icon;
use rune::{Color, Cx, El, Env, Runtime, el, icon, text};
use rune_fx::{FireParams, Fx, FxFrame, FxOptions, FxRenderer, FxSlot, Shape};
use serde::Deserialize;
use std::{
    ffi::c_void,
    panic::{AssertUnwindSafe, catch_unwind},
};

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct UiModel {
    pub providers: Vec<Provider>,
    pub selected: usize,
    pub picker: bool,
    pub pressed: i32,
    pub testing: bool,
    pub app: String,
    pub apps: Vec<App>,
    pub volume: f32,
    pub volume_muted: bool,
    pub volume_available: bool,
    pub volume_status: String,
    pub limits: Vec<Limit>,
    pub status: String,
    pub stale: bool,
    pub shift: i32,
    pub background: String,
    pub foreground: String,
    pub accent: String,
    pub muted: String,
    pub surface: String,
    pub warning: String,
    pub danger: String,
    pub accent_foreground: String,
    pub burn_minutes: f32,
    pub fire: bool,
    pub time: f32,
    pub fire_trigger_minutes: f32,
}
pub type Model = UiModel;
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub enabled: bool,
}
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct Limit {
    pub label: String,
    pub fraction: f32,
    pub countdown: String,
}
#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct App {
    pub id: String,
    pub label: String,
    pub x: f32,
    pub width: f32,
}
struct State {
    model: Model,
    icons: Vec<&'static Icon>,
}

fn color(value: &str) -> Color {
    rune::color::hex(value).unwrap_or(Color::BLACK)
}
fn label(value: impl Into<String>, size: f32, ink: Color) -> El {
    text(value.into())
        .font(rune::tok::TextStyle::new(size, 500.0))
        .color(ink)
}
fn provider_icon(s: &State, index: usize) -> El {
    let enabled = s.model.providers.get(index).is_some_and(|p| p.enabled);
    let ink = color(if enabled {
        &s.model.foreground
    } else {
        &s.model.muted
    });
    s.icons.get(index).map_or_else(
        || label("?", 30.0, ink),
        |i| {
            let logo = icon(i, 34.0).color(ink);
            if enabled { logo } else { logo.icon_one_color() }
        },
    )
}
static SPEAKER: Icon = Icon {
    name: "volume",
    view: 24.0,
    body: r#"<path d="M11 5 6 9H3v6h3l5 4V5Z" fill="currentColor"/><path d="M15 8a6 6 0 0 1 0 8M18 5a10 10 0 0 1 0 14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>"#,
};
static MUTED: Icon = Icon {
    name: "muted",
    view: 24.0,
    body: r#"<path d="M11 5 6 9H3v6h3l5 4V5Z" fill="currentColor"/><path d="m16 9 5 6m0-6-5 6" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>"#,
};
static BACK: Icon = Icon {
    name: "back",
    view: 24.0,
    body: r#"<path d="m12 5-7 7 7 7M5 12h15" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>"#,
};
fn view(s: &State, _: &mut Cx<State>) -> El {
    let m = &s.model;
    let (bg, fg, accent, muted, surface) = (
        color(&m.background),
        color(&m.foreground),
        color(&m.accent),
        color(&m.muted),
        color(&m.surface),
    );
    let shift = (m.shift % 5 - 2) as f32;
    let mut root = el().size(2008.0, 60.0).fill(bg).key("touchbar");
    if m.fire && m.app == "ai" && !m.picker {
        root = root.child(
            el().key("quota-fire")
                .effect(1)
                .effect_bleed(60.0)
                .size(1880.0, 4.0)
                .absolute(0.0, 56.0),
        );
        // The fire dances around the information, not over its legibility or brand colors.
        root = root.child(
            el().key("fire-scrim")
                .size(1752.0, 40.0)
                .absolute(120.0, 10.0)
                .radius(8.0)
                .fill(Color::BLACK.alpha(0.30)),
        );
    }
    if m.app == "menu" {
        for a in &m.apps {
            root = root.child(
                el().key(format!("launch-{}", a.id))
                    .role("button")
                    .label(format!("Open {}", a.id))
                    .size(a.width, 48.0)
                    .absolute(a.x, 6.0)
                    .radius(8.0)
                    .fill(surface)
                    .border(1.0, accent)
                    .center()
                    .child(
                        if a.id == "volume" {
                            icon(&SPEAKER, 32.0).color(accent)
                        } else {
                            label(&a.label, 28.0, accent)
                        }
                        .offset(shift, 0.0),
                    ),
            );
        }
    } else if m.app == "volume" {
        let available = m.volume_available;
        let level = m.volume.clamp(0.0, 1.0);
        let ink = if !available {
            muted
        } else if m.volume_muted {
            color(&m.danger)
        } else {
            accent
        };
        root = root.child(
            el().key("mute-button")
                .role("button")
                .label(if m.volume_muted { "Unmute" } else { "Mute" })
                .disabled(!available)
                .size(108.0, 48.0)
                .absolute(12.0, 6.0)
                .radius(8.0)
                .fill(surface)
                .center()
                .child(
                    icon(if m.volume_muted { &MUTED } else { &SPEAKER }, 34.0)
                        .color(ink)
                        .offset(shift, 0.0),
                ),
        );
        root = root.child(
            el().key("volume-slider")
                .role("slider")
                .label("Output volume")
                .disabled(!available)
                .size(1500.0, 48.0)
                .absolute(200.0, 6.0)
                .child(
                    el().key("volume-track")
                        .size(1460.0, 10.0)
                        .absolute(20.0, 19.0)
                        .radius(5.0)
                        .fill(surface),
                )
                .child(
                    el().size(1460.0 * level, 10.0)
                        .absolute(20.0, 19.0)
                        .radius(5.0)
                        .fill(if m.volume_muted { muted } else { ink }),
                )
                .child(
                    el().key("volume-knob")
                        .size(28.0, 28.0)
                        .absolute(6.0 + 1460.0 * level, 10.0)
                        .radius(14.0)
                        .fill(if available { fg } else { muted }),
                ),
        );
        root = root.child(
            label(
                if available {
                    format!("{:.0}%", level * 100.0)
                } else {
                    "—".into()
                },
                27.0,
                fg,
            )
            .absolute(1725.0 + shift, 13.0),
        );
        if !available {
            root = root.child(
                el().size(400.0, 42.0)
                    .absolute(740.0, 9.0)
                    .fill(bg)
                    .center()
                    .child(label(&m.volume_status, 22.0, muted)),
            );
        }
    } else if m.app == "functions" {
        for n in 1..=12 {
            let pressed = m.pressed == n;
            let w = 1864.0 / 12.0;
            root = root.child(
                el().key(format!("function-{n}"))
                    .role("button")
                    .label(format!("F{n}"))
                    .size(w - 6.0, 48.0)
                    .absolute(15.0 + (n - 1) as f32 * w, 6.0)
                    .radius(8.0)
                    .fill(if pressed { accent } else { surface })
                    .center()
                    .child(
                        label(
                            format!("F{n}"),
                            26.0,
                            if pressed {
                                color(&m.accent_foreground)
                            } else {
                                fg
                            },
                        )
                        .offset(shift, 0.0),
                    ),
            );
        }
    } else if m.app == "ai" {
        root = root.child(
            el().key("provider-toggle")
                .role("button")
                .label("Choose AI provider")
                .size(96.0, 48.0)
                .absolute(12.0, 6.0)
                .radius(8.0)
                .fill(surface)
                .center()
                .child(provider_icon(s, m.selected).offset(shift, 0.0)),
        );
        if m.picker {
            // One stable, generous hit target per provider. No vertical popup on a 60px panel.
            for (i, p) in m.providers.iter().enumerate() {
                root = root.child(
                    el().key(format!("provider-{}", p.id))
                        .role("button")
                        .label(&p.name)
                        .disabled(!p.enabled)
                        .size(100.0, 48.0)
                        .absolute(132.0 + i as f32 * 140.0, 6.0)
                        .radius(8.0)
                        .fill(if i == m.selected { surface } else { bg })
                        .center()
                        .child(provider_icon(s, i)),
                );
            }
        } else {
            for (i, l) in m.limits.iter().take(2).enumerate() {
                let x = 150.0 + i as f32 * 740.0 + shift;
                let ink = if l.fraction >= 0.9 {
                    color(&m.danger)
                } else {
                    accent
                };
                root = root.child(
                    label(&l.label, 21.0, fg)
                        .w(120.0)
                        .truncate()
                        .absolute(x, 17.0),
                );
                root = root.child(
                    el().size(220.0, 10.0)
                        .fill(surface)
                        .absolute(x + 128.0, 25.0)
                        .child(
                            el().size(220.0 * l.fraction.clamp(0.0, 1.0), 10.0)
                                .fill(ink),
                        ),
                );
                root = root.child(
                    label(format!("{:.0}%", l.fraction * 100.0), 25.0, fg)
                        .absolute(x + 370.0, 14.0),
                );
                root = root
                    .child(label(format!("↻ {}", l.countdown), 21.0, fg).absolute(x + 460.0, 17.0));
            }
            let x = if m.limits.is_empty() { 150.0 } else { 1640.0 };
            root = root.child(
                label(
                    &m.status,
                    18.0,
                    if m.stale { color(&m.warning) } else { muted },
                )
                .w(if m.limits.is_empty() { 1680.0 } else { 230.0 })
                .truncate()
                .absolute(x + shift, 19.0),
            );
        }
    }
    if m.app != "menu" {
        root = root.child(
            el().key("back-button")
                .role("button")
                .label("Back to app menu")
                .size(108.0, 48.0)
                .absolute(1888.0, 6.0)
                .radius(8.0)
                .fill(bg)
                .border(2.0, accent)
                .center()
                .child(icon(&BACK, 32.0).color(accent).offset(shift, 0.0)),
        );
    } else if m.testing {
        root = root.child(
            el().size(108.0, 48.0)
                .absolute(1888.0, 6.0)
                .radius(8.0)
                .border(1.0, color(&m.warning)),
        );
    }
    if m.testing {
        root = root.child(
            label("TEST", 10.0, color(&m.warning))
                .key("test-badge")
                .absolute(1894.0, 8.0),
        );
    }
    root
}

pub struct Ui {
    rt: Runtime<State>,
    gpu: Gpu,
    renderer: Renderer,
    target: Offscreen,
    fx: Option<FxRenderer>,
}
fn load_icon(assets: &str, id: &str) -> &'static Icon {
    let bundled = match id {
        "codex" => include_str!("../assets/codex.svg"),
        "claude" => include_str!("../assets/claude.svg"),
        "grok" => include_str!("../assets/grok.svg"),
        "kimi" => include_str!("../assets/kimi.svg"),
        "deepseek" => include_str!("../assets/deepseek.svg"),
        "minimax" => include_str!("../assets/minimax.svg"),
        "zai" => include_str!("../assets/zai.svg"),
        "openrouter" => include_str!("../assets/openrouter.svg"),
        "synthetic" => include_str!("../assets/synthetic.svg"),
        "ollama" => include_str!("../assets/ollama.svg"),
        "opencode" => include_str!("../assets/opencode.svg"),
        "cursor" => include_str!("../assets/cursor.svg"),
        _ => "",
    };
    let mut svg = std::fs::read_to_string(format!("{assets}/shell/plugins/agents/assets/{id}.svg"))
        .unwrap_or_else(|_| bundled.into())
        .replace('\'', "\"");
    let bounds = svg
        .split("viewBox=")
        .nth(1)
        .map(str::trim_start)
        .and_then(|v| {
            let quote = v.chars().next()?;
            v.get(1..)?.split(quote).next()
        })
        .unwrap_or("0 0 24 24")
        .split_whitespace()
        .filter_map(|v| v.parse::<f32>().ok())
        .collect::<Vec<_>>();
    let view = bounds
        .get(2)
        .copied()
        .unwrap_or(24.0)
        .max(bounds.get(3).copied().unwrap_or(24.0));
    // Rune's monochrome SVG subset does not skip text content in <title>.
    // Keep the root's inherited fill/stroke attributes, but omit metadata.
    for tag in ["title", "desc"] {
        while let Some(start) = svg.find(&format!("<{tag}")) {
            let Some(end) = svg[start..].find(&format!("</{tag}>")) else {
                break;
            };
            svg.replace_range(start..start + end + tag.len() + 3, "");
        }
    }
    // Icons are SVG bodies, not nested SVG viewports. In particular, Omarchy's
    // width="1em" must not shrink a logo inside Rune's generated SVG document.
    svg = svg.replacen("<svg", "<g", 1).replace("</svg>", "</g>");
    let dx = (view - bounds.get(2).copied().unwrap_or(view)) / 2.0;
    let dy = (view - bounds.get(3).copied().unwrap_or(view)) / 2.0;
    svg = format!("<g transform=\"translate({dx} {dy})\">{svg}</g>");
    Box::leak(Box::new(Icon {
        name: Box::leak(id.to_owned().into_boxed_str()),
        view,
        body: Box::leak(svg.into_boxed_str()),
    }))
}
impl Ui {
    pub fn new(model: Model) -> Result<Self, String> {
        let assets = std::env::var("OMARCHY_PATH").unwrap_or_else(|_| "/usr/share/omarchy".into());
        let icons = model
            .providers
            .iter()
            .map(|p| load_icon(&assets, &p.id))
            .collect();
        let mut rt = Runtime::new(
            State { model, icons },
            view,
            (2008.0, 60.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.set_srgb_blend(true);
        let gpu = Gpu::try_new(Gpu::instance(), None)?;
        let format = OFFSCREEN_FORMAT.remove_srgb_suffix();
        let mut renderer = Renderer::new(&gpu.device, format);
        renderer.set_srgb_blend(true);
        renderer.attach_text(&mut rt.text.atlas);
        let target = Offscreen::with_format(&gpu.device, 2008, 60, format);
        Ok(Self {
            rt,
            gpu,
            renderer,
            target,
            fx: None,
        })
    }
    pub fn render(&mut self, model: Model) -> Vec<u8> {
        self.rt.update(|s, tx| {
            s.model = model;
            tx.settle();
        });
        self.rt.frame(0.0);
        self.renderer
            .upload_images(&self.gpu.device, &self.gpu.queue, &mut self.rt.images);
        let mut encoder = self.gpu.device.create_command_encoder(&Default::default());
        self.renderer.prepare(
            &self.gpu.device,
            &self.gpu.queue,
            &mut self.rt.text.atlas,
            &self.rt.instances,
            (2008, 60),
        );
        if let Some((index, slot)) = self
            .rt
            .instances
            .iter()
            .enumerate()
            .find(|(_, i)| i.params[1] == rune::render::KIND_EFFECT)
        {
            let m = &self.rt.state.model;
            let urgency = (1.0 - m.burn_minutes / m.fire_trigger_minutes.max(1.0)).clamp(0.0, 1.0);
            let layers = [Fx::fire(FireParams {
                height: 48.0 + urgency * 12.0,
                scale: 22.0,
                speed: 1.2,
                intensity: 0.85 + urgency * 0.55,
                embers: 24,
                base_glow: 0.35,
                ..FireParams::default()
            })
            .on(Shape::rounded(
                slot.rect[0],
                slot.rect[1],
                slot.rect[2],
                slot.rect[3],
                0.0,
            ))
            .at(m.time)
            .seed(42)];
            let fx = self.fx.get_or_insert_with(|| {
                FxRenderer::new(
                    &self.gpu.device,
                    OFFSCREEN_FORMAT.remove_srgb_suffix(),
                    FxOptions { dither: true },
                )
            });
            fx.set_srgb_blend(true);
            fx.prepare_with_slots(
                &self.gpu.device,
                &self.gpu.queue,
                &FxFrame {
                    viewport: [2008, 60],
                    scale: 1.0,
                    layers: &layers,
                    backdrop: None,
                    sources: &[],
                },
                &[FxSlot::default().clip([0.0, 0.0, 1880.0, 60.0], [0; 4])],
            );
            self.renderer.draw_with(
                &mut encoder,
                &self.target.view,
                self.rt.clear_color(),
                &mut |pass, site| {
                    if site.index == index as u32 {
                        fx.draw_layer_in(pass, site.format, site.origin, 0);
                    }
                },
            );
        } else {
            self.renderer
                .draw(&mut encoder, &self.target.view, self.rt.clear_color());
        }
        self.target.copy(&mut encoder);
        self.gpu.queue.submit(Some(encoder.finish()));
        self.target.read(&self.gpu.device)
    }
}

// All calls run on the host's main thread. No panic is allowed across the C ABI.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn touchmeter_ui_new(json: *const u8, len: usize) -> *mut c_void {
    catch_unwind(AssertUnwindSafe(|| {
        let model =
            serde_json::from_slice(unsafe { std::slice::from_raw_parts(json, len) }).ok()?;
        match Ui::new(model) {
            Ok(ui) => Some(Box::into_raw(Box::new(ui)).cast()),
            Err(e) => {
                eprintln!("touchmeter: Rune initialization failed: {e}");
                None
            }
        }
    }))
    .ok()
    .flatten()
    .unwrap_or(std::ptr::null_mut())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn touchmeter_ui_render(
    ui: *mut c_void,
    json: *const u8,
    len: usize,
    out: *mut u8,
    out_len: usize,
) -> bool {
    if ui.is_null() || out_len != 2008 * 60 * 4 {
        return false;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let Ok(model) = serde_json::from_slice(unsafe { std::slice::from_raw_parts(json, len) })
        else {
            return false;
        };
        let pixels = unsafe { &mut *ui.cast::<Ui>() }.render(model);
        unsafe {
            std::ptr::copy_nonoverlapping(pixels.as_ptr(), out, pixels.len());
        }
        true
    }))
    .unwrap_or(false)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn touchmeter_ui_free(ui: *mut c_void) {
    if !ui.is_null() {
        drop(unsafe { Box::from_raw(ui.cast::<Ui>()) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_model_can_be_constructed_and_updated_without_json() {
        let mut model = Model {
            app: "functions".into(),
            ..Model::default()
        };
        model.pressed = 4;
        model.providers.push(Provider {
            id: "codex".into(),
            name: "Codex".into(),
            enabled: true,
        });
        assert_eq!(model.app, "functions");
        assert_eq!(model.pressed, 4);
        assert!(model.providers[0].enabled);
    }
    fn runtime(functions: bool, picker: bool) -> Runtime<State> {
        let model: Model = serde_json::from_value(serde_json::json!({
            "background": "#101315", "foreground": "#cacccc", "accent": "#ffd60a", "muted": "#707880",
            "surface": "#272727", "warning": "#ffc163", "danger": "#ff5f56", "accent_foreground": "#101315",
            "app": if functions { "functions" } else { "ai" }, "picker": picker,
            "providers": [{"id": "codex", "name": "OpenAI", "enabled": true}, {"id": "claude", "name": "Anthropic", "enabled": false}]
        })).unwrap();
        let mut rt = Runtime::new(
            State {
                model,
                icons: vec![],
            },
            view,
            (2008.0, 60.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        rt.frame(0.0);
        rt
    }
    #[test]
    fn function_hit_targets_match_the_host() {
        let rt = runtime(true, false);
        for n in 1..=12 {
            let rect = rt.presented(&format!("function-{n}")).unwrap().0;
            assert!((rect[0] - (15.0 + (n - 1) as f32 * 1864.0 / 12.0)).abs() < 1.0);
            assert_eq!(rect[1], 6.0);
            assert_eq!(rect[3], 48.0);
            let e = rt
                .elements()
                .into_iter()
                .find(|e| e.key.as_deref() == Some(&format!("function-{n}")))
                .unwrap();
            assert_eq!(e.radius, 8.0);
        }
        assert_eq!(
            rt.presented("back-button").unwrap().0,
            [1888.0, 6.0, 108.0, 48.0]
        );
    }
    #[test]
    fn provider_picker_is_icon_only_and_fits_the_panel() {
        let rt = runtime(false, true);
        assert_eq!(
            rt.presented("provider-toggle").unwrap().0,
            [12.0, 6.0, 96.0, 48.0]
        );
        assert_eq!(
            rt.presented("provider-codex").unwrap().0,
            [132.0, 6.0, 100.0, 48.0]
        );
        assert_eq!(
            rt.presented("provider-claude").unwrap().0,
            [272.0, 6.0, 100.0, 48.0]
        );
        for e in rt.elements() {
            assert!(e.rect[1] >= 0.0 && e.rect[1] + e.rect[3] <= 60.0);
            assert!(!matches!(
                e.text.as_deref(),
                Some("CODEX" | "OpenAI" | "Anthropic")
            ));
        }
    }
    #[test]
    fn installed_provider_logos_are_not_blank() {
        let assets = std::env::var("OMARCHY_PATH").unwrap_or_else(|_| "/usr/share/omarchy".into());
        for id in [
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
        ] {
            let icon = load_icon(&assets, id);
            assert!(
                rune::text::icon_coverage(icon, 34).iter().any(|&v| v > 0),
                "{id} must have a visible logo"
            );
        }
    }
    #[test]
    fn bundled_provider_logos_work_without_omarchy() {
        for id in [
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
        ] {
            let icon = load_icon("/nonexistent/touchmeter-no-omarchy", id);
            assert!(
                rune::text::icon_coverage(icon, 34).iter().any(|&v| v > 0),
                "{id} needs a bundled logo"
            );
        }
    }
    #[test]
    fn test_badge_is_visible_in_every_mode() {
        for (functions, picker, app) in [
            (false, false, "ai"),
            (false, true, "ai"),
            (true, false, "functions"),
            (false, false, "menu"),
            (false, false, "volume"),
        ] {
            let mut rt = runtime(functions, picker);
            assert!(rt.presented("test-badge").is_none());
            rt.update(|s, tx| {
                s.model.app = app.into();
                s.model.testing = true;
                tx.settle();
            });
            rt.frame(0.0);
            let badge = rt.presented("test-badge").unwrap().0;
            assert!(badge[0] >= 1888.0 && badge[0] + badge[2] <= 1996.0);
            assert!(badge[1] >= 6.0 && badge[1] + badge[3] <= 54.0);
        }
    }
    #[test]
    fn launcher_and_volume_hit_regions_match_the_host() {
        let mut rt = runtime(false, false);
        rt.update(|s, tx| {
            s.model.app = "menu".into();
            s.model.fire = true;
            s.model.apps = vec![
                App {
                    id: "functions".into(),
                    label: "F".into(),
                    x: 620.0,
                    width: 240.0,
                },
                App {
                    id: "ai".into(),
                    label: "AI".into(),
                    x: 884.0,
                    width: 240.0,
                },
                App {
                    id: "volume".into(),
                    x: 1148.0,
                    width: 240.0,
                    ..Default::default()
                },
            ];
            tx.settle();
        });
        rt.frame(0.0);
        assert_eq!(
            rt.presented("launch-volume").unwrap().0,
            [1148.0, 6.0, 240.0, 48.0]
        );
        assert!(rt.presented("back-button").is_none());
        assert!(rt.presented("quota-fire").is_none());
        rt.update(|s, tx| {
            s.model.app = "volume".into();
            s.model.volume = 0.5;
            s.model.volume_available = true;
            tx.settle();
        });
        rt.frame(0.0);
        assert_eq!(
            rt.presented("mute-button").unwrap().0,
            [12.0, 6.0, 108.0, 48.0]
        );
        assert_eq!(
            rt.presented("volume-slider").unwrap().0,
            [200.0, 6.0, 1500.0, 48.0]
        );
        assert_eq!(
            rt.presented("volume-track").unwrap().0,
            [220.0, 25.0, 1460.0, 10.0]
        );
        assert_eq!(
            rt.presented("volume-knob").unwrap().0,
            [936.0, 16.0, 28.0, 28.0]
        );
        assert!(rt.presented("quota-fire").is_none());
        assert!(rt.presented("back-button").is_some());
    }
}
