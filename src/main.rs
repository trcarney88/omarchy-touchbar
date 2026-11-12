use controls::{Controller, Effect, InputEvent};
use std::{
    process::ExitCode,
    rc::Rc,
    time::{Duration, Instant},
};
use touchmeter_ui::{App, Limit, Model, Provider, Ui};
mod appearance;
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
mod burnrate;
mod config;
mod controls;
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
mod credentials;
mod fixtures;
mod i18n;
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
mod jobs;
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
mod linux;
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
mod live;
#[cfg(all(target_os = "linux", feature = "linux-hardware"))]
mod live_worker;
mod preview;
mod simdata;
mod testcli;
use winit::{
    application::ApplicationHandler,
    dpi::LogicalSize,
    event::{ElementState, MouseButton, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy},
    keyboard::{KeyCode, PhysicalKey},
    window::{Fullscreen, Window, WindowId},
};

const W: f64 = 2008.0;
const H: f64 = 60.0;
const FRAME_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 30);
const FRAME_LEN: usize = 2008 * 60 * 4;

fn valid_frame_len(len: usize) -> bool {
    len == FRAME_LEN
}
fn error_exit_code(error: Option<&str>) -> ExitCode {
    if error.is_some() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
fn valid_monitor_value(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.is_empty() && !value.starts_with('-'))
}
fn panel_point(px: f64, py: f64, width: u32, height: u32) -> Option<(f64, f64)> {
    if width == 0 || height == 0 {
        return None;
    }
    let scale = (width as f64 / W).min(height as f64 / H);
    let x = (px - (width as f64 - W * scale) / 2.) / scale;
    let y = (py - (height as f64 - H * scale) / 2.) / scale;
    ((0.0..W).contains(&x) && (0.0..H).contains(&y)).then_some((x, y))
}
fn animation_deadline(
    model: &Model,
    occluded: bool,
    width: u32,
    height: u32,
    now: Instant,
    last_frame: Option<Instant>,
) -> Option<Instant> {
    if !model.fire || model.app != "ai" || model.picker || occluded || width == 0 || height == 0 {
        return None;
    }
    Some(
        last_frame
            .map_or(now, |last| last + FRAME_INTERVAL)
            .max(now),
    )
}
fn base() -> Model {
    Model {
        providers: [
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
        ]
        .iter()
        .map(|id| Provider {
            id: (*id).into(),
            name: id.to_string(),
            enabled: true,
        })
        .collect(),
        selected: 0,
        app: "menu".into(),
        apps: vec![
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
                label: "".into(),
                x: 1148.0,
                width: 240.0,
            },
        ],
        volume: 0.7,
        volume_available: true,
        volume_status: "Simulation".into(),
        limits: vec![
            Limit {
                label: "Session".into(),
                fraction: 0.38,
                countdown: "2h 14m".into(),
            },
            Limit {
                label: "Weekly".into(),
                fraction: 0.62,
                countdown: "3d 8h".into(),
            },
        ],
        status: "SIMULATED · No hardware or services connected".into(),
        background: "#101318".into(),
        foreground: "#f2f4f8".into(),
        accent: "#8ab4f8".into(),
        muted: "#777f8c".into(),
        surface: "#252b34".into(),
        warning: "#f0b35a".into(),
        danger: "#f07878".into(),
        accent_foreground: "#101318".into(),
        testing: true,
        fire_trigger_minutes: 30.0,
        burn_minutes: 20.0,
        ..Model::default()
    }
}
fn physical_model() -> Model {
    let mut model = base();
    for provider in &mut model.providers {
        provider.enabled = false;
    }
    model.limits.clear();
    model.status = "Sign in required".into();
    model.testing = false;
    model.fire = false;
    model.burn_minutes = 0.0;
    model.fire_trigger_minutes = 0.0;
    model.volume_available = false;
    model.volume = 0.0;
    model.pressed = 0;
    model.picker = false;
    model.app = "menu".into();
    model
}
fn version_string() -> String {
    format!("touchmeter {} (Rust)", env!("CARGO_PKG_VERSION"))
}
fn apply_appearance(model: &mut Model, snapshot: &appearance::AppearanceSnapshot) {
    model.background = snapshot.apply_color("background").into();
    model.foreground = snapshot.apply_color("foreground").into();
    model.accent = snapshot.accent.clone();
    model.muted = snapshot.apply_color("muted").into();
    model.surface = snapshot.apply_color("lighter_background").into();
    model.warning = snapshot.apply_color("yellow").into();
    model.danger = snapshot.apply_color("red").into();
    model.accent_foreground = snapshot.accent_foreground.clone();
    model.status = snapshot.status_label(&model.status);
    model.volume_status = snapshot.basic_label(&model.volume_status);
    for limit in &mut model.limits {
        limit.label = snapshot.basic_label(&limit.label);
        limit.countdown = snapshot.countdown_label(&limit.countdown);
    }
}
fn help() {
    println!(
        "touchmeter — experimental Rust Rune simulator\nUsage: touchmeter [simulate] [--scenario all-providers|high-usage|stale|missing|signed-out|fire|menu|volume|volume-muted|volume-unavailable | --fixture FILE] [--page menu|ai|functions|providers|volume] [--monitor INDEX|NAME] [--fullscreen] [--demo] [--language CODE]\n       touchmeter open (Linux hardware build only)\n       touchmeter preview OUTPUT.png [--demo|--fixture FILE|--scenario NAME] [--menu|--ai|--functions|--providers|--volume] [--language CODE]\n       touchmeter test ... | touchmeter config ..."
    );
}
struct AppState {
    model: Model,
    ui: Option<Ui>,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    pressed: Option<KeyCode>,
    controller: Controller,
    pointer_contact: bool,
    pointer: (f64, f64),
    fullscreen: bool,
    monitor: Option<String>,
    start: Instant,
    occluded: bool,
    last_frame: Option<Instant>,
    terminal_error: Option<String>,
    sim_data: simdata::SimData,
    next_sim_poll: Instant,
    config_path: std::path::PathBuf,
    last_config_error: Option<String>,
    list_monitors: bool,
    appearance: appearance::Appearance,
}
#[derive(Debug)]
enum UserEvent {
    Shutdown,
}
impl AppState {
    fn fail(&mut self, e: &ActiveEventLoop, message: String) {
        eprintln!("touchmeter: {message}");
        self.terminal_error = Some(message);
        e.exit();
    }
    fn draw(&mut self) -> Result<(), String> {
        if self.ui.is_none() {
            match Ui::new(self.model.clone()) {
                Ok(ui) => self.ui = Some(ui),
                Err(err) => {
                    return Err(format!("Rune UI initialization failed: {err}"));
                }
            }
        }
        let (Some(ui), Some(window), Some(surface)) =
            (&mut self.ui, &self.window, &mut self.surface)
        else {
            return Ok(());
        };
        let mut rendered = self.model.clone();
        apply_appearance(&mut rendered, self.appearance.snapshot());
        let pixels = ui.render(rendered);
        if !valid_frame_len(pixels.len()) {
            return Err(format!(
                "invalid rendered frame length: {} (expected {FRAME_LEN})",
                pixels.len()
            ));
        }
        let size = window.inner_size();
        let scale = (size.width as f64 / W).min(size.height as f64 / H);
        if size.width == 0 || size.height == 0 || scale <= 0. {
            return Ok(());
        }
        let dw = (W * scale) as u32;
        let dh = (H * scale) as u32;
        let ox = (size.width - dw) / 2;
        let oy = (size.height - dh) / 2;
        let mut b = surface
            .buffer_mut()
            .map_err(|err| format!("softbuffer buffer acquisition failed: {err}"))?;
        b.fill(0x101318);
        for y in 0..dh {
            let sy = (y as usize * 60 / dh as usize).min(59);
            for x in 0..dw {
                let sx = (x as usize * 2008 / dw as usize).min(2007);
                let i = (sy * 2008 + sx) * 4;
                let (r, g, bl) = (pixels[i], pixels[i + 1], pixels[i + 2]);
                b[((y + oy) * size.width + x + ox) as usize] =
                    ((r as u32) << 16) | ((g as u32) << 8) | bl as u32;
            }
        }
        b.present()
            .map_err(|err| format!("softbuffer presentation failed: {err}"))?;
        Ok(())
    }
    fn redraw(&mut self, e: &ActiveEventLoop) {
        if let Err(err) = self.draw() {
            self.fail(e, err);
        }
    }
    fn page(&mut self, p: &str) {
        let page = if p == "providers" { "ai" } else { p };
        let effects = self.controller.open_page(page, &mut self.model);
        self.apply_effects(effects);
        self.pressed = None;
        self.model.pressed = 0;
        if p == "providers" {
            self.model.picker = true;
        }
        self.pointer_contact = false;
    }
    fn apply_effects(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::SetVolume(value) => self.model.volume = value as f32,
                Effect::ToggleMute => self.model.volume_muted = !self.model.volume_muted,
                Effect::SelectProvider(index) => {
                    self.sim_data
                        .select(&mut self.model, index, chrono::Utc::now())
                }
                _ => {}
            }
        }
    }
    fn pointer_event(&mut self, event: InputEvent) {
        let effects = self.controller.handle(event, &mut self.model);
        self.apply_effects(effects);
    }
    #[cfg(test)]
    fn pointer_move_panel(&mut self, point: Option<(f64, f64)>) {
        if !self.pointer_contact {
            return;
        }
        self.pointer_event(match point {
            Some((x, y)) => InputEvent::Move(x, y),
            None => InputEvent::Move(-1., -1.),
        });
    }
    fn cancel_pointer(&mut self) {
        self.pointer_contact = false;
        self.pointer_event(InputEvent::Cancel);
        self.pressed = None;
        self.model.pressed = 0;
    }
    fn key(&mut self, k: KeyCode, down: bool) {
        if !down {
            if self.pressed == Some(k) {
                self.pressed = None;
                self.model.pressed = 0;
            }
            return;
        }
        match k {
            KeyCode::Escape => {
                if self.fullscreen {
                    if let Some(w) = &self.window {
                        w.set_fullscreen(None)
                    }
                    self.fullscreen = false
                } else {
                    // The event handler exits the loop for non-fullscreen Escape.
                }
            }
            KeyCode::KeyM => self.page("menu"),
            KeyCode::F11 if self.model.app != "functions" => {
                if let Some(w) = &self.window {
                    w.set_fullscreen(if self.fullscreen {
                        None
                    } else {
                        Some(Fullscreen::Borderless(None))
                    });
                    self.fullscreen = !self.fullscreen
                }
            }
            KeyCode::ArrowLeft => self.model.shift -= 1,
            KeyCode::ArrowRight => self.model.shift += 1,
            KeyCode::Enter => {
                if self.model.app == "menu" {
                    self.page("ai")
                } else if self.model.app == "ai" {
                    self.cancel_pointer();
                    self.model.picker = !self.model.picker
                }
            }
            KeyCode::Digit1 if self.model.app == "menu" => self.page("functions"),
            KeyCode::Digit2 if self.model.app == "menu" => self.page("ai"),
            KeyCode::Digit3 if self.model.app == "menu" => self.page("volume"),
            _ => {
                let n = match k {
                    KeyCode::F1 => 1,
                    KeyCode::F2 => 2,
                    KeyCode::F3 => 3,
                    KeyCode::F4 => 4,
                    KeyCode::F5 => 5,
                    KeyCode::F6 => 6,
                    KeyCode::F7 => 7,
                    KeyCode::F8 => 8,
                    KeyCode::F9 => 9,
                    KeyCode::F10 => 10,
                    KeyCode::F11 => 11,
                    KeyCode::F12 => 12,
                    _ => 0,
                };
                if n > 0 && self.model.app == "functions" {
                    self.model.pressed = n;
                    self.pressed = Some(k)
                }
            }
        }
    }
    fn poll_config(&mut self, now: chrono::DateTime<chrono::Utc>) -> bool {
        match config::read(&self.config_path) {
            Ok(c) => {
                self.last_config_error = None;
                if self.model.fire_trigger_minutes != c.fire_trigger_minutes as f32 {
                    self.model.fire_trigger_minutes = c.fire_trigger_minutes as f32;
                    self.sim_data.poll(&mut self.model, now);
                    return true;
                }
            }
            Err(error) => {
                if self.last_config_error.as_ref() != Some(&error) {
                    eprintln!("touchmeter: {error} — keeping last valid configuration");
                }
                self.last_config_error = Some(error);
            }
        }
        false
    }
}
impl ApplicationHandler<UserEvent> for AppState {
    fn user_event(&mut self, e: &ActiveEventLoop, _event: UserEvent) {
        self.cancel_pointer();
        e.exit();
    }
    fn resumed(&mut self, e: &ActiveEventLoop) {
        if self.list_monitors {
            let primary = e.primary_monitor().and_then(|m| m.name());
            for (index, monitor) in e.available_monitors().enumerate() {
                let name = monitor.name().unwrap_or_else(|| "(unknown)".into());
                let size = monitor.size();
                println!(
                    "{index}\t{name}\t{}x{}{}",
                    size.width,
                    size.height,
                    if Some(name.clone()) == primary {
                        "\tprimary"
                    } else {
                        ""
                    }
                );
            }
            e.exit();
            return;
        }
        if self.window.is_some() {
            return;
        }
        let mut a = winit::window::WindowAttributes::default()
            .with_title("Touchmeter · Rust Simulator")
            .with_inner_size(LogicalSize::new(1200., 220.));
        if self.fullscreen {
            a = a.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        if let Some(m) = &self.monitor {
            let mons = e.available_monitors();
            let selected = if let Ok(n) = m.parse::<usize>() {
                mons.into_iter().nth(n)
            } else {
                mons.into_iter().find(|x| x.name().as_deref() == Some(m))
            };
            if let Some(mon) = selected {
                if self.fullscreen {
                    a = a.with_fullscreen(Some(Fullscreen::Borderless(Some(mon))))
                } else {
                    a = a.with_position(mon.position())
                }
            } else {
                eprintln!("touchmeter: requested monitor not found: {m}");
                self.terminal_error = Some(format!("requested monitor not found: {m}"));
                e.exit();
                return;
            }
        }
        let w = match e.create_window(a) {
            Ok(w) => Rc::new(w),
            Err(err) => {
                self.fail(e, format!("window creation failed: {err}"));
                return;
            }
        };
        let c = match softbuffer::Context::new(w.clone()) {
            Ok(c) => c,
            Err(err) => {
                self.fail(e, format!("softbuffer context creation failed: {err}"));
                return;
            }
        };
        let mut s = match softbuffer::Surface::new(&c, w.clone()) {
            Ok(s) => s,
            Err(err) => {
                self.fail(e, format!("softbuffer surface creation failed: {err}"));
                return;
            }
        };
        let z = w.inner_size();
        s.resize(
            std::num::NonZeroU32::new(z.width.max(1)).unwrap(),
            std::num::NonZeroU32::new(z.height.max(1)).unwrap(),
        )
        .unwrap_or_else(|err| {
            self.terminal_error = Some(format!("softbuffer resize failed: {err}"));
            eprintln!("touchmeter: softbuffer resize failed: {err}");
            e.exit();
        });
        if self.terminal_error.is_some() {
            return;
        }
        self.surface = Some(s);
        self.window = Some(w);
        self.redraw(e);
    }
    fn window_event(&mut self, e: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => e.exit(),
            WindowEvent::Focused(false) => {
                self.cancel_pointer();
                self.redraw(e);
            }
            WindowEvent::Resized(s) => {
                self.cancel_pointer();
                if let Some(b) = &mut self.surface {
                    if let Err(err) = b.resize(
                        std::num::NonZeroU32::new(s.width.max(1)).unwrap(),
                        std::num::NonZeroU32::new(s.height.max(1)).unwrap(),
                    ) {
                        self.fail(e, format!("softbuffer resize failed: {err}"));
                        return;
                    }
                }
                self.redraw(e)
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let PhysicalKey::Code(k) = event.physical_key {
                    if !event.repeat {
                        if k == KeyCode::Escape
                            && !self.fullscreen
                            && event.state == ElementState::Pressed
                        {
                            e.exit();
                        }
                        self.key(k, event.state == ElementState::Pressed);
                        self.redraw(e)
                    }
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.pointer = (position.x, position.y);
                let point = self.window.as_ref().and_then(|w| {
                    let sz = w.inner_size();
                    panel_point(position.x, position.y, sz.width, sz.height)
                });
                let active = self.pointer_contact;
                if let Some((x, y)) = point.filter(|_| self.pointer_contact) {
                    self.pointer_event(InputEvent::Move(x, y));
                } else if self.pointer_contact {
                    self.pointer_event(InputEvent::Move(-1.0, -1.0));
                }
                if active {
                    self.redraw(e)
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                let size = self.window.as_ref().unwrap().inner_size();
                let (x, y) = panel_point(self.pointer.0, self.pointer.1, size.width, size.height)
                    .unwrap_or((-1., -1.));
                if state == ElementState::Pressed {
                    self.pointer_contact = true;
                    self.pointer_event(InputEvent::Begin(x, y));
                } else {
                    self.pointer_event(InputEvent::End);
                    self.pointer_contact = false;
                }
                self.redraw(e)
            }
            WindowEvent::RedrawRequested => self.redraw(e),
            WindowEvent::Occluded(occluded) => self.occluded = occluded,
            _ => {}
        }
    }
    fn about_to_wait(&mut self, e: &ActiveEventLoop) {
        let now = Instant::now();
        if now >= self.next_sim_poll {
            self.next_sim_poll = now + simdata::POLL_INTERVAL.min(appearance::POLL_INTERVAL);
            let appearance_changed = self.appearance.poll(now);
            let utc_now = chrono::Utc::now();
            let config_changed = self.poll_config(utc_now);
            let sim_changed = self.sim_data.poll(&mut self.model, utc_now);
            if let Some(effects) = self.controller.cancel_if_view_changed(&mut self.model) {
                self.pointer_contact = false;
                self.apply_effects(effects);
            }
            if sim_changed || config_changed || appearance_changed {
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
        }
        let size = self.window.as_ref().map(|w| w.inner_size());
        let deadline = size.and_then(|s| {
            animation_deadline(
                &self.model,
                self.occluded,
                s.width,
                s.height,
                Instant::now(),
                self.last_frame,
            )
        });
        if let Some(deadline) = deadline {
            let deadline = deadline.min(self.next_sim_poll);
            e.set_control_flow(ControlFlow::WaitUntil(deadline));
            if deadline <= Instant::now() {
                self.model.time = self.start.elapsed().as_secs_f32();
                self.last_frame = Some(Instant::now());
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
        } else {
            e.set_control_flow(ControlFlow::WaitUntil(self.next_sim_poll));
        }
    }
}
fn main() -> ExitCode {
    let mut a = std::env::args().skip(1).collect::<Vec<_>>();
    if a.len() == 1 && (a[0] == "--version" || a[0] == "-V") {
        println!("{}", version_string());
        return ExitCode::SUCCESS;
    }
    if a.first().is_some_and(|x| x == "open") {
        if a.len() != 1 {
            eprintln!("usage: touchmeter open");
            return ExitCode::from(2);
        }
        #[cfg(all(target_os = "linux", feature = "linux-hardware"))]
        return match linux::runtime::run(physical_model()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) if error == "AlreadyRunning" => ExitCode::SUCCESS,
            Err(error) if error.starts_with("FORCED_RELEASE:") => {
                eprintln!("touchmeter: {error}");
                ExitCode::from(78)
            }
            Err(error) => {
                eprintln!("touchmeter: {error}");
                ExitCode::FAILURE
            }
        };
        #[cfg(not(all(target_os = "linux", feature = "linux-hardware")))]
        {
            eprintln!(
                "touchmeter: physical opening is unavailable in this build; no device or service initialized"
            );
            return ExitCode::from(2);
        }
    }
    if a.first().is_some_and(|x| x == "test") {
        a.remove(0);
        return testcli::run(a);
    }
    if a.first().is_some_and(|x| x == "config") {
        return match config::run_cli(&a[1..]) {
            Ok(out) => {
                print!("{out}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("{err}");
                ExitCode::FAILURE
            }
        };
    }
    if a.first().is_some_and(|x| x == "preview") {
        return preview::run(&a[1..]);
    }
    if a.iter().any(|x| x == "--help" || x == "-h") {
        help();
        return ExitCode::SUCCESS;
    }
    if a.first().is_some_and(|x| x == "simulate") {
        a.remove(0);
    }
    let mut m = base();
    let config_path = config::path();
    match config::read(&config_path) {
        Ok(c) => m.fire_trigger_minutes = c.fire_trigger_minutes as f32,
        Err(error) => eprintln!("touchmeter: {error} — using default configuration"),
    }
    let (mut full, mut mon, mut list_monitors) = (false, None, false);
    let (mut demo, mut language_override) = (false, None);
    let (mut scenario_arg, mut fixture_arg, mut page_arg) = (None, None, None);
    let mut i = 0;
    while i < a.len() {
        match a[i].as_str() {
            "--fullscreen" => full = true,
            "--demo" => demo = true,
            "--language" => {
                let Some(language) = a.get(i + 1) else {
                    eprintln!("--language requires a code");
                    return ExitCode::from(2);
                };
                if !valid_language(language) {
                    eprintln!("invalid language code");
                    return ExitCode::from(2);
                }
                language_override = Some(language.clone());
                i += 1;
            }
            "--list-monitors" => {
                list_monitors = true;
            }
            "--scenario" => {
                scenario_arg = a.get(i + 1).cloned();
                if scenario_arg.is_none() {
                    eprintln!("--scenario requires a name");
                    return ExitCode::from(2);
                }
                i += 1
            }
            "--fixture" => {
                fixture_arg = a.get(i + 1).cloned();
                if fixture_arg.is_none() {
                    eprintln!("--fixture requires a path");
                    return ExitCode::from(2);
                }
                i += 1
            }
            "--monitor" => {
                if !valid_monitor_value(a.get(i + 1).map(String::as_str)) {
                    eprintln!("--monitor requires index/name");
                    return ExitCode::from(2);
                }
                mon = a.get(i + 1).cloned();
                i += 1
            }
            "--page" | "--menu" | "--ai" | "--functions" | "--providers" | "--volume" => {
                let p = if a[i] == "--page" {
                    i += 1;
                    a.get(i).map(String::as_str).unwrap_or("")
                } else {
                    &a[i][2..]
                };
                if !["menu", "ai", "functions", "providers", "volume"].contains(&p) {
                    eprintln!("invalid page");
                    return ExitCode::from(2);
                }
                if page_arg.replace(p.to_string()).is_some() {
                    eprintln!("conflicting page overrides");
                    return ExitCode::from(2);
                }
            }
            x => {
                eprintln!("unknown option: {x}");
                return ExitCode::from(2);
            }
        }
        i += 1;
    }
    if fixture_arg.is_some() && scenario_arg.is_some() {
        eprintln!("--fixture and --scenario are mutually exclusive");
        return ExitCode::from(2);
    }
    let mut launch = match simdata::launch(
        fixture_arg.as_deref().map(std::path::Path::new),
        scenario_arg.as_deref(),
        chrono::Utc::now(),
    ) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("touchmeter: {e}");
            return ExitCode::from(2);
        }
    };
    if let Some(p) = page_arg.as_ref() {
        launch.mode = match p.as_str() {
            "menu" => "menu",
            "providers" => "providers",
            "functions" => "functions",
            "volume" => "volume",
            _ => "usage",
        }
        .into();
    }
    let sim_path = match testcli::runtime_path(true) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("touchmeter: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut sim_data = simdata::SimData::new(launch.clone(), sim_path)
        .with_base_path(fixture_arg.as_deref().map(std::path::PathBuf::from))
        .with_explicit_mode(page_arg.as_ref().map(|p| {
            match p.as_str() {
                "menu" => "menu",
                "providers" => "providers",
                "functions" => "functions",
                "volume" => "volume",
                _ => "usage",
            }
            .to_string()
        }));
    sim_data.poll(&mut m, chrono::Utc::now());
    let appearance = appearance::Appearance::new(appearance::AppearanceOptions {
        demo,
        language_override,
        ..Default::default()
    });
    let ev = match EventLoop::<UserEvent>::with_user_event().build() {
        Ok(ev) => ev,
        Err(err) => {
            eprintln!("touchmeter: event loop creation failed: {err}");
            return ExitCode::FAILURE;
        }
    };
    ev.set_control_flow(ControlFlow::Wait);
    let mut s = AppState {
        model: m,
        ui: None,
        window: None,
        surface: None,
        pressed: None,
        controller: Controller::default(),
        pointer_contact: false,
        pointer: (0., 0.),
        fullscreen: full,
        monitor: mon,
        start: Instant::now(),
        occluded: false,
        last_frame: None,
        terminal_error: None,
        sim_data,
        next_sim_poll: Instant::now() + simdata::POLL_INTERVAL,
        config_path,
        last_config_error: None,
        list_monitors,
        appearance,
    };
    install_shutdown_handler(ev.create_proxy());
    if let Err(err) = ev.run_app(&mut s) {
        eprintln!("touchmeter: event loop failed: {err}");
        return ExitCode::FAILURE;
    }
    error_exit_code(s.terminal_error.as_deref())
}

fn valid_language(value: &str) -> bool {
    let mut parts = value.split('-');
    let language = parts.next().unwrap_or("");
    (2..=3).contains(&language.len())
        && language.bytes().all(|b| b.is_ascii_lowercase())
        && parts
            .next()
            .is_none_or(|r| r.len() == 2 && r.bytes().all(|b| b.is_ascii_alphabetic()))
        && parts.next().is_none()
}

fn install_shutdown_handler(proxy: EventLoopProxy<UserEvent>) {
    if let Ok(mut signals) = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
    ]) {
        std::thread::spawn(move || {
            if signals.forever().next().is_some() {
                let _ = proxy.send_event(UserEvent::Shutdown);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_model_is_safe_and_has_no_simulated_or_authenticated_data() {
        let model = physical_model();
        assert!(model.providers.iter().all(|provider| !provider.enabled));
        assert!(model.limits.is_empty());
        assert_eq!(model.status, "Sign in required");
        assert!(!model.testing && !model.fire);
        assert_eq!(model.burn_minutes, 0.0);
        assert!(!model.volume_available);
        assert_eq!(model.volume, 0.0);
        assert_eq!(model.pressed, 0);
        assert!(!model.picker);
        assert_eq!(model.app, "menu");
    }

    #[test]
    fn version_uses_package_metadata() {
        assert_eq!(
            version_string(),
            format!("touchmeter {} (Rust)", env!("CARGO_PKG_VERSION"))
        );
        assert_eq!(env!("CARGO_PKG_VERSION"), "0.8.0");
    }

    fn state(app: &str) -> AppState {
        let mut model = base();
        model.app = app.into();
        AppState {
            model,
            ui: None,
            window: None,
            surface: None,
            pressed: None,
            controller: Controller::default(),
            pointer_contact: false,
            pointer: (0., 0.),
            fullscreen: false,
            monitor: None,
            start: Instant::now(),
            occluded: false,
            last_frame: None,
            terminal_error: None,
            sim_data: simdata::SimData::new(
                fixtures::scenario("menu", chrono::Utc::now()).unwrap(),
                std::env::temp_dir().join("touchmeter-no-test-override"),
            ),
            next_sim_poll: Instant::now() + simdata::POLL_INTERVAL,
            config_path: std::env::temp_dir().join("touchmeter-no-config"),
            last_config_error: None,
            list_monitors: false,
            appearance: appearance::Appearance::new(appearance::AppearanceOptions::default()),
        }
    }

    #[test]
    fn monitor_argument_must_be_a_nonempty_value() {
        assert!(valid_monitor_value(Some("0")));
        assert!(valid_monitor_value(Some("DisplayPort-1")));
        assert!(!valid_monitor_value(None));
        assert!(!valid_monitor_value(Some("")));
        assert!(!valid_monitor_value(Some("--fullscreen")));
    }

    #[test]
    fn language_override_validation_and_appearance_projection() {
        assert!(valid_language("da"));
        assert!(valid_language("en-US"));
        assert!(!valid_language("bad_code"));
        assert!(!valid_language("e"));
        let mut model = base();
        let appearance = appearance::Appearance::new(appearance::AppearanceOptions {
            demo: true,
            language_override: Some("da".into()),
            ..Default::default()
        });
        apply_appearance(&mut model, appearance.snapshot());
        assert_eq!(model.accent, appearance.snapshot().accent);
        assert_eq!(appearance.snapshot().language, "da");
    }

    #[test]
    fn pointer_action_cancels_permanently_after_leaving_target() {
        let mut s = state("menu");
        s.pointer_contact = true;
        s.pointer_event(InputEvent::Begin(900., 20.));
        s.pointer_move_panel(Some((700., 20.)));
        s.pointer_move_panel(Some((900., 20.)));
        s.pointer_event(InputEvent::End);
        assert_eq!(s.model.app, "menu");
    }

    #[test]
    fn function_key_highlight_clears_on_different_key() {
        let mut s = state("functions");
        s.pointer_contact = true;
        s.pointer_event(InputEvent::Begin(12. + 3.5 * (1864. / 12.), 20.));
        assert!(s.model.pressed > 0);
        s.pointer_move_panel(Some((12. + 4.5 * (1864. / 12.), 20.)));
        assert_eq!(s.model.pressed, 0);
    }

    #[test]
    fn shared_function_hit_targets_leave_three_pixel_gaps() {
        let mut s = state("functions");
        let key_width = 1864.0 / 12.0;
        s.pointer_event(InputEvent::Begin(12.0 + key_width - 1.0, 20.0));
        assert_eq!(s.model.pressed, 0);
        s.pointer_event(InputEvent::End);
        s.pointer_event(InputEvent::Begin(12.0 + key_width / 2.0, 20.0));
        assert_eq!(s.model.pressed, 1);
    }

    #[test]
    fn pointer_motion_preserves_keyboard_function_key_highlight() {
        let mut s = state("functions");
        s.key(KeyCode::F4, true);

        s.pointer_move_panel(Some((12. + 4.5 * (1864. / 12.), 20.)));
        assert_eq!(s.model.pressed, 4);
        assert_eq!(s.pressed, Some(KeyCode::F4));

        s.pointer_move_panel(None);
        assert_eq!(s.model.pressed, 4);
        assert_eq!(s.pressed, Some(KeyCode::F4));

        s.key(KeyCode::F4, false);
        assert_eq!(s.model.pressed, 0);
        assert_eq!(s.pressed, None);
    }

    #[test]
    fn enter_cancels_armed_picker_contact_before_hiding_picker() {
        let mut s = state("ai");
        s.model.picker = true;
        s.pointer_contact = true;
        s.pointer_event(InputEvent::Begin(150., 20.));
        s.key(KeyCode::Enter, true);
        assert!(!s.model.picker);
        assert!(!s.pointer_contact);
        s.pointer_event(InputEvent::End);
        assert_eq!(s.model.selected, 0);
    }

    #[test]
    fn slider_drag_cancels_after_vertical_exit() {
        let mut s = state("volume");
        s.pointer_contact = true;
        s.pointer_event(InputEvent::Begin(800., 20.));
        let before = s.model.volume;
        s.pointer_move_panel(Some((1000., 55.)));
        s.pointer_move_panel(Some((1000., 60.)));
        let before_after_exit = s.model.volume;
        s.pointer_move_panel(Some((900., 20.)));
        assert_eq!(s.model.volume, before_after_exit);
        assert_ne!(before, before_after_exit);
    }

    #[test]
    fn animation_scheduler_gates_and_caps_frames() {
        let now = Instant::now();
        let mut m = base();
        assert_eq!(animation_deadline(&m, false, 10, 10, now, None), None);
        m.fire = true;
        assert_eq!(animation_deadline(&m, false, 10, 10, now, None), None);
        m.app = "ai".into();
        assert_eq!(animation_deadline(&m, false, 10, 10, now, None), Some(now));
        m.picker = true;
        assert_eq!(animation_deadline(&m, false, 10, 10, now, None), None);
        m.picker = false;
        assert_eq!(animation_deadline(&m, true, 10, 10, now, None), None);
        assert_eq!(animation_deadline(&m, false, 0, 10, now, None), None);
        assert_eq!(animation_deadline(&m, false, 10, 0, now, None), None);
        let last = now - Duration::from_millis(10);
        assert_eq!(
            animation_deadline(&m, false, 10, 10, now, Some(last)),
            Some(last + FRAME_INTERVAL)
        );
        let last = now - FRAME_INTERVAL - Duration::from_millis(1);
        assert_eq!(
            animation_deadline(&m, false, 10, 10, now, Some(last)),
            Some(now)
        );
    }

    #[test]
    fn rendered_frame_length_must_match_rgba_surface() {
        assert!(valid_frame_len(FRAME_LEN));
        assert!(!valid_frame_len(FRAME_LEN - 1));
        assert!(!valid_frame_len(FRAME_LEN + 1));
    }

    fn launcher_hit(x: f64, y: f64, model: &Model) -> Option<&str> {
        if !(6.0..54.0).contains(&y) {
            return None;
        }
        model
            .apps
            .iter()
            .find(|a| (a.x as f64..(a.x + a.width) as f64).contains(&x))
            .map(|a| a.id.as_str())
    }

    #[test]
    fn launcher_uses_centered_controls_catalog() {
        let m = base();
        assert_eq!(
            m.apps.iter().map(|a| a.x as i32).collect::<Vec<_>>(),
            [620, 884, 1148]
        );
        assert_eq!(launcher_hit(700., 20., &m), Some("functions"));
        assert_eq!(launcher_hit(900., 20., &m), Some("ai"));
        assert_eq!(launcher_hit(1200., 20., &m), Some("volume"));
        assert_eq!(launcher_hit(860., 20., &m), None);
        assert_eq!(launcher_hit(900., 55., &m), None);
    }

    #[test]
    fn model_fire_preset_is_bounded_and_badges_simulated_controls() {
        let m = base();
        assert!(m.testing);
        assert_eq!(m.fire_trigger_minutes, 30.0);
        assert_eq!(m.burn_minutes, 20.0);
        assert!(m.providers.iter().all(|p| p.enabled));
    }

    #[test]
    fn config_reload_updates_fire_without_resetting_navigation_or_volume() {
        let dir =
            std::env::temp_dir().join(format!("touchmeter-config-reload-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");
        let mut s = state("functions");
        s.config_path = path.clone();
        s.model.fire_trigger_minutes = 45.0;
        let mut fire_fixture = fixtures::scenario("fire", chrono::Utc::now()).unwrap();
        if let Some(provider) = fire_fixture.providers.get_mut("claude") {
            provider.burn_minutes = Some(40.0);
        }
        fire_fixture.selected = "claude".into();
        s.sim_data = simdata::SimData::new(fire_fixture, dir.join("override"));
        s.sim_data.poll(&mut s.model, chrono::Utc::now());
        s.model.app = "functions".into();
        s.model.volume = 0.31;
        assert!(s.model.fire);
        std::fs::write(&path, r#"{"version":1,"fire":{"triggerMinutes":30}}"#).unwrap();
        s.poll_config(chrono::Utc::now());
        assert!(!s.model.fire);
        assert_eq!(s.model.fire_trigger_minutes, 30.0);
        assert_eq!(s.model.app, "functions");
        assert_eq!(s.model.volume, 0.31);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn closed_letterbox_and_zero_size_are_not_coordinates() {
        fn transform(px: f64, py: f64, width: u32, height: u32) -> Option<(f64, f64)> {
            if width == 0 || height == 0 {
                return None;
            }
            let s = (width as f64 / W).min(height as f64 / H);
            let (ox, oy) = ((width as f64 - W * s) / 2., (height as f64 - H * s) / 2.);
            let (x, y) = ((px - ox) / s, (py - oy) / s);
            ((0.0..W).contains(&x) && (0.0..H).contains(&y)).then_some((x, y))
        }
        assert_eq!(transform(10., 10., 0, 0), None);
        assert_eq!(transform(0., 0., 1000, 500), None);
        let (x, y) = transform(500., 250., 1000, 500).unwrap();
        assert!((x - 1004.).abs() < 0.01);
        assert!((y - 30.).abs() < 0.01);
    }
}
