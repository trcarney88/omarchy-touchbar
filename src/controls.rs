use touchmeter_ui::{App, Model};

#[derive(Debug, Clone, Copy)]
pub enum InputEvent {
    Begin(f64, f64),
    Move(f64, f64),
    End,
    Cancel,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Activity,
    FunctionKey { number: i32, pressed: bool },
    SelectProvider(usize),
    SetVolume(f64),
    ToggleMute,
    PageChanged,
}

pub struct Controller {
    contact: bool,
    sleeping: bool,
    key: i32,
    action: Option<Action>,
    drag: bool,
    view: Option<ViewToken>,
    point: Option<(f64, f64)>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct ViewToken {
    app: String,
    picker: bool,
    apps: Vec<(String, u32, u32)>,
    providers: Vec<(String, bool)>,
    volume_available: bool,
}
fn view_token(m: &Model) -> ViewToken {
    ViewToken {
        app: m.app.clone(),
        picker: m.picker,
        apps: m
            .apps
            .iter()
            .map(|a| (a.id.clone(), a.x.to_bits(), a.width.to_bits()))
            .collect(),
        providers: m
            .providers
            .iter()
            .map(|p| (p.id.clone(), p.enabled))
            .collect(),
        volume_available: m.volume_available,
    }
}
#[derive(Clone, PartialEq)]
enum Action {
    Back,
    App(String),
    Mute,
    Providers,
    Provider(usize),
}
impl Default for Controller {
    fn default() -> Self {
        Self {
            contact: false,
            sleeping: false,
            key: 0,
            action: None,
            drag: false,
            view: None,
            point: None,
        }
    }
}

impl Controller {
    pub fn cancel(&mut self, model: &mut Model) -> Vec<Effect> {
        self.contact = false;
        self.action = None;
        self.drag = false;
        self.view = None;
        self.point = None;
        if self.key != 0 {
            let n = self.key;
            self.key = 0;
            model.pressed = 0;
            vec![
                Effect::FunctionKey {
                    number: n,
                    pressed: false,
                },
                Effect::PageChanged,
            ]
        } else {
            vec![]
        }
    }
    pub fn open_page(&mut self, page: &str, model: &mut Model) -> Vec<Effect> {
        let mut out = self.cancel(model);
        if page == "menu" || ["functions", "ai", "volume"].contains(&page) {
            if model.app != page || model.picker {
                model.app = page.into();
                model.picker = false;
                out.push(Effect::PageChanged);
            }
        }
        out
    }
    pub fn set_sleeping(&mut self, yes: bool, model: &mut Model) -> Vec<Effect> {
        let out = if yes { self.cancel(model) } else { vec![] };
        self.sleeping = yes;
        out
    }
    /// Cancel an armed contact when a poll changes the controls visible beneath it.
    pub fn cancel_if_view_changed(&mut self, model: &mut Model) -> Option<Vec<Effect>> {
        if self.contact
            && self
                .view
                .as_ref()
                .is_some_and(|view| *view != view_token(model))
        {
            Some(self.cancel(model))
        } else {
            None
        }
    }
    fn key_at(x: f64, y: f64) -> i32 {
        if !x.is_finite() || !y.is_finite() || x < 12. || x >= 1876. || y < 6. || y >= 54. {
            return 0;
        }
        let w = 1864. / 12.;
        let i = ((x - 12.) / w) as i32;
        let l = x - 12. - i as f64 * w;
        if l >= 3. && l < w - 3. {
            i + 1
        } else {
            0
        }
    }
    fn action(x: f64, y: f64, m: &Model) -> Option<Action> {
        if !x.is_finite() || !y.is_finite() || y < 6. || y >= 54. {
            return None;
        }
        if m.app == "menu" {
            return m
                .apps
                .iter()
                .find(|a| x >= a.x as f64 && x < (a.x + a.width) as f64)
                .map(|a| Action::App(a.id.clone()));
        }
        if (1888. ..1996.).contains(&x) {
            return Some(Action::Back);
        }
        if m.app == "volume" && m.volume_available && (12. ..120.).contains(&x) {
            return Some(Action::Mute);
        }
        if m.app == "ai" {
            if (12. ..108.).contains(&x) {
                return Some(Action::Providers);
            }
            if m.picker && x >= 132. && x < 1812. {
                let i = ((x - 132.) / 140.) as usize;
                if x - 132. - i as f64 * 140. < 100.
                    && m.providers.get(i).is_some_and(|p| p.enabled)
                {
                    return Some(Action::Provider(i));
                }
            }
        }
        None
    }
    pub fn handle(&mut self, e: InputEvent, m: &mut Model) -> Vec<Effect> {
        match e {
            InputEvent::Begin(x, y) => {
                if self.contact {
                    return vec![];
                }
                self.contact = true;
                self.view = Some(view_token(m));
                self.point = Some((x, y));
                let wake = self.sleeping;
                self.sleeping = false;
                let mut o = vec![Effect::Activity];
                if wake {
                    return o;
                }
                if m.app == "volume"
                    && m.volume_available
                    && x >= 200.
                    && x < 1700.
                    && y >= 6.
                    && y < 54.
                {
                    self.drag = true;
                    o.push(Effect::SetVolume(((x - 220.) / 1460.).clamp(0., 1.)));
                    return o;
                }
                self.action = Self::action(x, y, m);
                if self.action.is_none() && m.app == "functions" {
                    self.key = Self::key_at(x, y);
                    if self.key > 0 {
                        m.pressed = self.key;
                        o.push(Effect::FunctionKey {
                            number: self.key,
                            pressed: true,
                        });
                        o.push(Effect::PageChanged)
                    }
                }
                o
            }
            InputEvent::Move(x, y) => {
                if !self.contact {
                    return vec![];
                }
                if self
                    .view
                    .as_ref()
                    .is_some_and(|view| *view != view_token(m))
                {
                    return self.cancel(m);
                }
                self.point = Some((x, y));
                let mut o = vec![Effect::Activity];
                if self.drag {
                    if !x.is_finite() || !y.is_finite() || y < 0. || y >= 60. {
                        self.drag = false
                    } else {
                        o.push(Effect::SetVolume(((x - 220.) / 1460.).clamp(0., 1.)))
                    }
                } else {
                    if self.action.is_some() && Self::action(x, y, m) != self.action {
                        self.action = None
                    }
                    if self.key > 0 && Self::key_at(x, y) != self.key {
                        let n = self.key;
                        self.key = 0;
                        m.pressed = 0;
                        o.push(Effect::FunctionKey {
                            number: n,
                            pressed: false,
                        });
                        o.push(Effect::PageChanged)
                    }
                }
                o
            }
            InputEvent::Cancel => self.cancel(m),
            InputEvent::End => {
                if !self.contact {
                    return vec![];
                }
                let a = self.action.clone().filter(|action| {
                    self.view.as_ref() == Some(&view_token(m))
                        && self
                            .point
                            .is_some_and(|(x, y)| Self::action(x, y, m).as_ref() == Some(action))
                });
                let mut o = self.cancel(m);
                match a {
                    Some(Action::Back) => o.extend(self.open_page("menu", m)),
                    Some(Action::App(p)) => o.extend(self.open_page(&p, m)),
                    Some(Action::Mute) => o.push(Effect::ToggleMute),
                    Some(Action::Providers) => {
                        m.picker = !m.picker;
                        o.push(Effect::PageChanged)
                    }
                    Some(Action::Provider(i)) => {
                        m.selected = i;
                        m.picker = false;
                        o.push(Effect::SelectProvider(i));
                        o.push(Effect::PageChanged)
                    }
                    None => {}
                }
                o
            }
        }
    }
}

pub fn catalog() -> Vec<App> {
    [
        ("functions", "F", 620.),
        ("ai", "AI", 884.),
        ("volume", "", 1148.),
    ]
    .into_iter()
    .map(|(id, label, x)| App {
        id: id.into(),
        label: label.into(),
        x,
        width: 240.,
    })
    .collect()
}
