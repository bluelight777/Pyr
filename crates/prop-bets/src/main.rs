//! Prop Bets, in the Pyr Library: every prop bet on the Pyr server showing in Pyr, in a window of
//! its own (in Pyr's, or popped out), with your Pyr Casino chips. Pyr starts it and the two talk a
//! JSON object a line (see `crates/app/src/game_bridge.rs`): Pyr tells it the props as they
//! change, my chips and the server's text channels, and says what happened (a win, or what the
//! server said no to); it asks Pyr to bet, cash out, open, settle or call off a prop (`@pyr {...}`
//! on its output), and Pyr does that on the server just as a prop's card in chat does. The server
//! checks everything: this window only shows, and asks. Started by itself, it says to open it
//! from Pyr.
//!
//! Its cards are the ones in Pyr's chat (`crates/app/src/props.rs`): the same art, words and
//! odds, so a prop looks the same wherever it's bet on.

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use std::io::{BufRead, Write};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Pos2, Rect, RichText, Sense, Stroke, TextStyle, Visuals, pos2, vec2};
use pyr_poker::props::{self as p, PropState, PropView};
use serde_json::{Value, json};

// ---- Pyr's look (Campfire's colors, Inter)

const MAIN: Color32 = Color32::from_rgb(0x26, 0x20, 0x1c);
const SIDE: Color32 = Color32::from_rgb(0x1e, 0x19, 0x16);
const ELEV: Color32 = Color32::from_rgb(0x2f, 0x28, 0x23);
const ELEV2: Color32 = Color32::from_rgb(0x3a, 0x31, 0x2b);
const LINE: Color32 = Color32::from_rgb(0x3b, 0x32, 0x2c);
const TEXT: Color32 = Color32::from_rgb(0xee, 0xe6, 0xdf);
const BODY: Color32 = Color32::from_rgb(0xe6, 0xdd, 0xd5);
const MUTED: Color32 = Color32::from_rgb(0xa8, 0x99, 0x8c);
const DIM: Color32 = Color32::from_rgb(0x7d, 0x70, 0x67);
const EMBER: Color32 = Color32::from_rgb(0xff, 0x7a, 0x3d);
const EMBER2: Color32 = Color32::from_rgb(0xff, 0xb3, 0x47);
const LIVE: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);
const CODE_BG: Color32 = Color32::from_rgb(0x1a, 0x15, 0x12);
const SELECTION: Color32 = Color32::from_rgb(0x8a, 0x40, 0x1c);
const ON_EMBER: Color32 = Color32::from_rgb(0x1b, 0x14, 0x11);
const GOLD: Color32 = Color32::from_rgb(0xff, 0xd3, 0x7a);

/// The outcomes' jeweled chips, in order, and their colors (as Pyr's cards have them).
const CHIPS: [&[u8]; 6] = [
    include_bytes!("../../app/assets/props/chip-emerald.webp"),
    include_bytes!("../../app/assets/props/chip-ruby.webp"),
    include_bytes!("../../app/assets/props/chip-sapphire.webp"),
    include_bytes!("../../app/assets/props/chip-amber.webp"),
    include_bytes!("../../app/assets/props/chip-amethyst.webp"),
    include_bytes!("../../app/assets/props/chip-pearl.webp"),
];
const COLORS: [Color32; 6] = [
    Color32::from_rgb(0x2e, 0xc2, 0x6e),
    Color32::from_rgb(0xe5, 0x48, 0x4d),
    Color32::from_rgb(0x4a, 0x7d, 0xf0),
    Color32::from_rgb(0xf0, 0xa0, 0x30),
    Color32::from_rgb(0xa8, 0x5c, 0xf0),
    Color32::from_rgb(0xe8, 0xe2, 0xd6),
];
const BANNER: &[u8] = include_bytes!("../../app/assets/props/banner.webp");
const CARD_BG: &[u8] = include_bytes!("../../app/assets/props/card-bg.webp");
const EMBLEM: &[u8] = include_bytes!("../../app/assets/props/emblem.webp");
const WON: &[u8] = include_bytes!("../../app/assets/props/won.webp");
const LOCK: &[u8] = include_bytes!("../../app/assets/props/lock.webp");

/// How long something Pyr said shows at the top.
const SAID_FOR: Duration = Duration::from_secs(8);

fn main() -> eframe::Result<()> {
    let bridge = std::env::var_os("PYR_BRIDGE").is_some();
    // (Inside Pyr's window, Pyr shows it once it's in its place.)
    let hidden = std::env::var_os("PYR_EMBED").is_some();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Prop Bets")
        .with_app_id("prop-bets")
        .with_inner_size([560.0, 780.0])
        .with_min_inner_size([320.0, 300.0])
        .with_visible(!hidden);
    if let Ok(img) = image::load_from_memory(EMBLEM) {
        let img = img.to_rgba8();
        viewport = viewport.with_icon(egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() });
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        "Prop Bets",
        options,
        Box::new(move |cc| {
            let (tx, rx) = mpsc::channel();
            if bridge {
                let ctx = cc.egui_ctx.clone();
                let _ = std::thread::Builder::new().name("from-pyr".into()).spawn(move || {
                    for line in std::io::stdin().lock().lines() {
                        let Ok(line) = line else { break };
                        if let Ok(v) = serde_json::from_str::<Value>(&line) {
                            if tx.send(v).is_err() {
                                return;
                            }
                            ctx.request_repaint();
                        }
                    }
                    // Pyr closed: so does this.
                    let _ = tx.send(json!({"op": "pyr_gone"}));
                    ctx.request_repaint();
                });
            }
            Ok(Box::new(App::new(&cc.egui_ctx, rx, bridge)))
        }),
    )
}

/// Asks Pyr for something: a line on the output Pyr reads.
fn ask(v: Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "@pyr {v}");
    let _ = out.flush();
}

/// What's asked of Pyr (`op` and all), one function each so the tests can hold them to what
/// Pyr reads (`crates/app/src/plugins/programs.rs`).
mod asks {
    use serde_json::{Value, json};

    pub fn bet(id: u64, outcome: usize, chips: u64, min_shares: f64) -> Value {
        json!({"op": "prop_bet", "id": id, "outcome": outcome, "chips": chips, "min_shares": min_shares})
    }
    pub fn cash_out(id: u64, outcome: usize) -> Value {
        json!({"op": "prop_cash_out", "id": id, "outcome": outcome})
    }
    pub fn open(channel: i64, question: &str, outcomes: &[String], depth: u64, closes_in: u64) -> Value {
        json!({"op": "prop_open", "channel": channel, "question": question, "outcomes": outcomes, "depth": depth, "closes_in": closes_in})
    }
    pub fn settle(id: u64, outcome: usize) -> Value {
        json!({"op": "prop_settle", "id": id, "outcome": outcome})
    }
    pub fn void(id: u64) -> Value {
        json!({"op": "prop_void", "id": id})
    }
    pub fn refresh() -> Value {
        json!({"op": "props_refresh"})
    }
}

// ---- words

/// "12,345"
fn chips_text(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// "1 bet", "1,204 bets".
fn count(n: u64, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{} {many}", chips_text(n)) }
}

/// "62%"
fn pct(x: f64) -> String {
    format!("{}%", (x * 100.0).round() as i64)
}

/// "in 2 h", "in 5 min".
fn time_left(ms: i64) -> String {
    let m = (ms / 60_000).max(1);
    if m < 60 {
        format!("in {m} min")
    } else if m < 48 * 60 {
        format!("in {} h", m / 60)
    } else {
        format!("in {} days", m / 1440)
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

// ---- what Pyr said

/// Which props show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    /// Taking bets, or waiting to be settled.
    Open,
    /// The ones I hold shares in.
    Mine,
    /// Settled or called off.
    Done,
}

/// Everything Pyr told it, as it stands.
#[derive(Default)]
struct Model {
    /// Heard from Pyr yet, and Pyr's gone (closed).
    heard: bool,
    gone: bool,
    /// The server showing in Pyr (its name), and whether it has prop bets.
    server: Option<String>,
    props_here: bool,
    /// Me (my id), my chips (once the casino said), the server's text channels.
    me: String,
    chips: Option<u64>,
    channels: Vec<(i64, String)>,
    /// Every prop, newest first.
    props: Vec<PropView>,
    /// What Pyr said lately: text, whether it went wrong, when.
    said: Option<(String, bool, Instant)>,
}

impl Model {
    fn hear(&mut self, v: &Value) {
        match v["op"].as_str() {
            Some("props") => {
                self.heard = true;
                self.server = v["server"]["name"].as_str().map(String::from);
                self.props_here = v["props_here"].as_bool().unwrap_or(false);
                self.me = v["me"].as_str().unwrap_or_default().to_string();
                self.chips = v["chips"].as_u64();
                self.channels = v["channels"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|c| Some((c["id"].as_i64()?, c["name"].as_str()?.to_string()))).collect())
                    .unwrap_or_default();
                self.props = v["list"].as_array().map(|a| a.iter().filter_map(|x| serde_json::from_value(x.clone()).ok()).collect()).unwrap_or_default();
                self.props.sort_by(|a, b| b.id.cmp(&a.id));
            }
            Some("said") => {
                if let Some(text) = v["text"].as_str().filter(|t| !t.trim().is_empty()) {
                    self.said = Some((text.to_string(), v["error"].as_bool().unwrap_or(false), Instant::now()));
                }
            }
            Some("pyr_gone") => self.gone = true,
            _ => {}
        }
    }

    fn shows(&self, tab: Tab, prop: &PropView) -> bool {
        match tab {
            Tab::Open => matches!(prop.state, PropState::Open | PropState::Closed),
            Tab::Mine => prop.mine.iter().any(|h| h.shares > 0.0),
            Tab::Done => matches!(prop.state, PropState::Settled | PropState::Void),
        }
    }

    fn shown(&self, tab: Tab) -> Vec<&PropView> {
        self.props.iter().filter(|prop| self.shows(tab, prop)).collect()
    }

    fn channel_name(&self, id: i64) -> Option<&str> {
        self.channels.iter().find(|c| c.0 == id).map(|c| c.1.as_str())
    }
}

// ---- the art

struct Art {
    chips: Vec<egui::TextureHandle>,
    banner: Option<egui::TextureHandle>,
    card_bg: Option<egui::TextureHandle>,
    emblem: Option<egui::TextureHandle>,
    won: Option<egui::TextureHandle>,
    lock: Option<egui::TextureHandle>,
}

impl Art {
    fn load(ctx: &egui::Context) -> Art {
        let t = |name: &str, bytes: &[u8]| -> Option<egui::TextureHandle> {
            let img = image::load_from_memory(bytes).ok()?.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            Some(ctx.load_texture(name, egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), egui::TextureOptions::LINEAR))
        };
        Art {
            chips: CHIPS.iter().enumerate().filter_map(|(i, b)| t(&format!("chip-{i}"), b)).collect(),
            banner: t("banner", BANNER),
            card_bg: t("card-bg", CARD_BG),
            emblem: t("emblem", EMBLEM),
            won: t("won", WON),
            lock: t("lock", LOCK),
        }
    }
}

/// The part of a picture that fills `r` (cut to `r`'s shape about `focus`, like a cover).
fn cover_uv(size: [usize; 2], r: Rect, focus: (f32, f32)) -> Rect {
    let (iw, ih) = (size[0] as f32, size[1] as f32);
    let (want, have) = (r.width() / r.height().max(1.0), iw / ih.max(1.0));
    if have > want {
        let k = want / have;
        let x = ((focus.0 - k / 2.0).max(0.0)).min(1.0 - k);
        Rect::from_min_max(pos2(x, 0.0), pos2(x + k, 1.0))
    } else {
        let k = have / want;
        let y = ((focus.1 - k / 2.0).max(0.0)).min(1.0 - k);
        Rect::from_min_max(pos2(0.0, y), pos2(1.0, y + k))
    }
}

/// A picture over `r` as a cover, tinted.
fn cover(p: &egui::Painter, t: Option<&egui::TextureHandle>, r: Rect, tint: Color32) {
    if let Some(t) = t {
        p.image(t.id(), r, cover_uv(t.size(), r, (0.5, 0.5)), tint);
    }
}

/// A picture over `r` as a cover about `focus`, its corners rounded.
fn cover_rounded(t: &egui::TextureHandle, r: Rect, radius: f32, focus: (f32, f32), tint: Color32) -> egui::Shape {
    let uv = cover_uv(t.size(), r, focus);
    let rad = radius.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    let to_uv = |q: Pos2| pos2(uv.left() + (q.x - r.left()) / r.width().max(1.0) * uv.width(), uv.top() + (q.y - r.top()) / r.height().max(1.0) * uv.height());
    let mut m = egui::Mesh::with_texture(t.id());
    let c = r.center();
    m.vertices.push(egui::epaint::Vertex { pos: c, uv: to_uv(c), color: tint });
    let seg = 8;
    let corners = [(pos2(r.right() - rad, r.bottom() - rad), 0.0f32), (pos2(r.left() + rad, r.bottom() - rad), 90.0), (pos2(r.left() + rad, r.top() + rad), 180.0), (pos2(r.right() - rad, r.top() + rad), 270.0)];
    for (cc, a0) in corners {
        for i in 0..=seg {
            let a = (a0 + 90.0 * i as f32 / seg as f32).to_radians();
            let q = cc + vec2(a.cos(), a.sin()) * rad;
            m.vertices.push(egui::epaint::Vertex { pos: q, uv: to_uv(q), color: tint });
        }
    }
    let n = 4 * (seg + 1) as u32;
    for i in 1..=n {
        m.add_triangle(0, i, if i == n { 1 } else { i + 1 });
    }
    egui::Shape::mesh(m)
}

fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

fn bold_font(size: f32) -> FontId {
    FontId::new(size, bold())
}

fn primary_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(ON_EMBER).font(bold_font(14.0))).fill(EMBER).corner_radius(6)
}

/// Pyr's fonts and widgets.
fn style(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("inter".into(), Arc::new(FontData::from_static(include_bytes!("../../app/assets/fonts/Inter-Regular.otf"))));
    fonts.font_data.insert("inter-semibold".into(), Arc::new(FontData::from_static(include_bytes!("../../app/assets/fonts/Inter-SemiBold.otf"))));
    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut body = vec!["inter".to_string()];
    body.extend(fallbacks.iter().cloned());
    let mut b = vec!["inter-semibold".to_string()];
    b.extend(fallbacks);
    fonts.families.insert(FontFamily::Proportional, body);
    fonts.families.insert(bold(), b);
    ctx.set_fonts(fonts);
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Body, FontId::proportional(14.5)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Heading, bold_font(19.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.menu_margin = egui::Margin::same(6);
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.scroll.floating = true;
        let mut v = Visuals::dark();
        v.panel_fill = MAIN;
        v.window_fill = SIDE;
        v.window_stroke = Stroke::new(1.0f32, LINE);
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(10);
        v.popup_shadow = egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(150) };
        v.extreme_bg_color = CODE_BG;
        v.faint_bg_color = ELEV;
        v.selection.bg_fill = SELECTION;
        v.selection.stroke = Stroke::new(1.0f32, EMBER2);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = SIDE;
        w.noninteractive.weak_bg_fill = SIDE;
        w.noninteractive.bg_stroke = Stroke::new(1.0f32, LINE);
        w.noninteractive.fg_stroke = Stroke::new(1.0f32, BODY);
        w.inactive.bg_fill = ELEV;
        w.inactive.weak_bg_fill = ELEV;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0f32, TEXT);
        w.inactive.corner_radius = CornerRadius::same(6);
        w.hovered.bg_fill = ELEV2;
        w.hovered.weak_bg_fill = ELEV2;
        w.hovered.bg_stroke = Stroke::new(1.0f32, Color32::from_rgb(0x55, 0x48, 0x3f));
        w.hovered.fg_stroke = Stroke::new(1.5f32, TEXT);
        w.hovered.corner_radius = CornerRadius::same(6);
        let active = Color32::from_rgb(0x4a, 0x3a, 0x30);
        w.active.bg_fill = active;
        w.active.weak_bg_fill = active;
        w.active.fg_stroke = Stroke::new(1.5f32, EMBER2);
        w.active.corner_radius = CornerRadius::same(6);
        w.open.bg_fill = ELEV2;
        w.open.weak_bg_fill = ELEV2;
        s.visuals = v;
    });
}

// ---- the window

/// A bet being picked: the prop, the outcome, how much (as typed).
struct Betting {
    id: u64,
    outcome: usize,
    amount: String,
}

/// The New prop bet form.
#[derive(Clone)]
struct NewProp {
    channel: Option<i64>,
    question: String,
    outcomes: Vec<String>,
    depth: u64,
    closes_in: u64,
}

struct App {
    rx: mpsc::Receiver<Value>,
    bridge: bool,
    m: Model,
    tab: Tab,
    betting: Option<Betting>,
    new_prop: Option<NewProp>,
    art: Art,
}

impl App {
    fn new(ctx: &egui::Context, rx: mpsc::Receiver<Value>, bridge: bool) -> App {
        style(ctx);
        App { rx, bridge, m: Model::default(), tab: Tab::Open, betting: None, new_prop: None, art: Art::load(ctx) }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(v) = self.rx.try_recv() {
            self.m.hear(&v);
        }
        if self.m.gone {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        // (Betting closes by the clock, and what Pyr said goes: the window keeps up.)
        ctx.request_repaint_after(Duration::from_secs(15));
        egui::TopBottomPanel::top("head").frame(egui::Frame::new().fill(SIDE).inner_margin(egui::Margin::symmetric(14, 10))).show(ctx, |ui| self.head(ui));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(MAIN).inner_margin(egui::Margin::symmetric(14, 12))).show(ctx, |ui| self.body(ui));
        self.new_prop_window(ctx);
    }
}

/// A word in the middle of the window.
fn note(ui: &mut egui::Ui, title: &str, words: &str) {
    ui.add_space(48.0);
    ui.vertical_centered(|ui| {
        ui.set_max_width(400.0);
        ui.label(RichText::new(title).font(bold_font(18.0)).color(TEXT));
        ui.add_space(4.0);
        ui.label(RichText::new(words).color(MUTED));
    });
}

impl App {
    /// Its emblem and name, the server's name and my chips; New prop and Refresh.
    fn head(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
            cover(ui.painter(), self.art.emblem.as_ref(), r, Color32::WHITE);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new("Prop Bets").font(bold_font(18.0)).color(GOLD));
                let under = match (&self.m.server, self.m.chips) {
                    (Some(s), Some(c)) if self.m.props_here => format!("{s} · {} chips", chips_text(c)),
                    (Some(s), _) => s.clone(),
                    _ => "Bets with your Pyr Casino chips".into(),
                };
                ui.label(RichText::new(under).size(12.5).color(MUTED));
            });
            if self.bridge && self.m.server.is_some() && self.m.props_here {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(primary_button("New prop")).on_hover_text("Open a prop bet in one of this server's channels").clicked() && self.new_prop.is_none() {
                        self.new_prop = Some(NewProp { channel: self.m.channels.first().map(|c| c.0), question: String::new(), outcomes: vec!["Yes".into(), "No".into()], depth: p::DEPTHS[0].0, closes_in: 0 });
                    }
                    if ui.button("Refresh").on_hover_text("Every prop again, from the server").clicked() {
                        ask(asks::refresh());
                    }
                });
            }
        });
    }

    fn body(&mut self, ui: &mut egui::Ui) {
        if !self.bridge {
            return note(ui, "Open Prop Bets from Pyr", "It bets your Pyr Casino chips on your Pyr server's props, so it opens from Pyr: in Plugins, Prop Bets, then Open.");
        }
        if !self.m.heard {
            ui.add_space(48.0);
            ui.vertical_centered(|ui| {
                ui.spinner();
                ui.label(RichText::new("Getting the props from Pyr…").color(MUTED));
            });
            return;
        }
        if self.m.server.is_none() {
            return note(ui, "Go to a server in Pyr", "Prop Bets shows the props of the server you were last on in Pyr: go to one, then come back here.");
        }
        if !self.m.props_here {
            return note(ui, "No prop bets on this server", "Its owner's Pyr server needs Pyr 0.14.0 or newer for prop bets.");
        }
        self.said(ui);
        self.tabs(ui);
        ui.add_space(10.0);
        let now = now_ms();
        let shown: Vec<PropView> = self.m.shown(self.tab).into_iter().cloned().collect();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                if shown.is_empty() {
                    let (title, words) = match self.tab {
                        Tab::Open => ("No props open", "Open one with New prop, or with /prop in a channel."),
                        Tab::Mine => ("No bets of yours", "Bet on a prop and it shows here, with what it's worth now."),
                        Tab::Done => ("Nothing settled yet", "Props show here once they're settled or called off."),
                    };
                    return note(ui, title, words);
                }
                for prop in &shown {
                    self.card(ui, prop, now);
                    ui.add_space(14.0);
                }
            });
        });
    }

    /// What Pyr said lately (a win, or what the server said no to), for a few seconds.
    fn said(&mut self, ui: &mut egui::Ui) {
        let Some((text, error, at)) = self.m.said.clone() else { return };
        let age = at.elapsed();
        if age > SAID_FOR {
            self.m.said = None;
            return;
        }
        ui.ctx().request_repaint_after(SAID_FOR - age);
        let (fill, col) = if error { (Color32::from_rgb(0x3a, 0x1c, 0x1c), Color32::from_rgb(0xff, 0xb4, 0xb4)) } else { (Color32::from_rgb(0x33, 0x2a, 0x16), GOLD) };
        egui::Frame::new().fill(fill).corner_radius(8).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).color(col)).wrap());
        });
        ui.add_space(8.0);
    }

    fn tabs(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            for (tab, name) in [(Tab::Open, "Open"), (Tab::Mine, "My bets"), (Tab::Done, "Settled")] {
                let n = self.m.shown(tab).len();
                let on = self.tab == tab;
                let label = RichText::new(format!("{name}  {n}")).font(bold_font(13.5)).color(if on { ON_EMBER } else { TEXT });
                if ui.add(egui::Button::new(label).fill(if on { EMBER2 } else { ELEV }).corner_radius(14)).clicked() && !on {
                    self.tab = tab;
                    self.betting = None;
                }
            }
        });
    }

    /// A prop's card, as in Pyr's chat: the question on its lit board (with its channel), each
    /// outcome on its chip with its odds and Bet, what I hold with Cash out, how much is in and
    /// the latest bets, and Settle and Call off for whoever may.
    fn card(&mut self, ui: &mut egui::Ui, prop: &PropView, now: i64) {
        let w = 460.0_f32.min(ui.available_width());
        egui::Frame::new().fill(Color32::from_rgb(0x14, 0x0f, 0x0d)).stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x5a, 0x40, 0x22))).corner_radius(12).show(ui, |ui| {
            ui.set_width(w);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                // The board: its art behind the question.
                let (top, _) = ui.allocate_exact_size(vec2(w, 96.0), Sense::hover());
                let pnt = ui.painter();
                match &self.art.banner {
                    Some(t) => {
                        pnt.add(cover_rounded(t, top, 12.0, (0.42, 0.38), Color32::from_gray(150)));
                    }
                    None => {
                        pnt.rect_filled(top, CornerRadius::same(12), Color32::from_rgb(0x2a, 0x18, 0x10));
                    }
                }
                pnt.rect_filled(Rect::from_min_max(top.left_top() + vec2(14.0, 12.0), top.right_bottom() - vec2(14.0, 10.0)), CornerRadius::same(8), Color32::from_black_alpha(150));
                let emblem = Rect::from_min_size(top.left_top() + vec2(22.0, 20.0), vec2(30.0, 30.0));
                cover(pnt, self.art.emblem.as_ref(), emblem, Color32::WHITE);
                let state = match prop.state {
                    PropState::Open if prop.open_at(now) => match prop.closes_at {
                        0 => "OPEN".to_string(),
                        c => format!("OPEN · CLOSES {}", time_left(c - now).to_uppercase()),
                    },
                    PropState::Open | PropState::Closed => "BETTING CLOSED".into(),
                    PropState::Settled => "SETTLED".into(),
                    PropState::Void => "CALLED OFF".into(),
                };
                pnt.text(emblem.right_center() + vec2(8.0, -7.0), Align2::LEFT_CENTER, format!("PROP BET #{}", prop.id), bold_font(11.5), GOLD);
                pnt.text(emblem.right_center() + vec2(8.0, 8.0), Align2::LEFT_CENTER, state, bold_font(10.5), Color32::from_rgb(0xd8, 0xc8, 0xb0));
                if let Some(c) = self.m.channel_name(prop.channel_id) {
                    pnt.text(pos2(top.right() - 26.0, emblem.center().y), Align2::RIGHT_CENTER, format!("#{c}"), bold_font(11.5), Color32::from_rgb(0xd8, 0xc8, 0xb0));
                }
                let q = ui.fonts_mut(|f| f.layout(prop.question.clone(), bold_font(16.0), Color32::WHITE, w - 56.0));
                let qh = q.size().y;
                ui.painter().galley(pos2(top.left() + 26.0, top.bottom() - 14.0 - qh), q, Color32::WHITE);
                // The rest, on the card's velvet (its place kept before what's drawn over it).
                let body_top = ui.cursor().top();
                let slot = ui.painter().add(egui::Shape::Noop);
                let inner = egui::Frame::new().inner_margin(egui::Margin { left: 30, right: 30, top: 12, bottom: 30 }).show(ui, |ui| {
                    ui.set_width(w - 60.0);
                    self.outcomes(ui, prop, now);
                    self.mine(ui, prop, now);
                    footer(ui, prop);
                    self.admin(ui, prop);
                });
                let body = Rect::from_min_max(pos2(top.left() + 1.0, body_top), pos2(top.right() - 1.0, inner.response.rect.bottom() - 1.0));
                if let Some(t) = &self.art.card_bg {
                    ui.painter().set(slot, cover_rounded(t, body, 11.0, (0.5, 0.75), Color32::from_gray(200)));
                }
            });
        });
    }

    /// Each outcome: its chip, name and odds, a bar, and Bet (or, settled, who won).
    fn outcomes(&mut self, ui: &mut egui::Ui, prop: &PropView, now: i64) {
        let prices = prop.prices();
        let open = prop.open_at(now);
        // (Whoever opened it settles it, so doesn't bet on it.)
        let mine = prop.author == self.m.me;
        if prop.state == PropState::Settled {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(54.0, 54.0), Sense::hover());
                cover(ui.painter(), self.art.won.as_ref(), r, Color32::WHITE);
                ui.vertical(|ui| {
                    let won = prop.winner.and_then(|w| prop.outcomes.get(w)).cloned().unwrap_or_default();
                    ui.label(RichText::new(format!("Settled: {won}")).font(bold_font(17.0)).color(GOLD));
                    let who = if prop.winners.is_empty() {
                        "Nobody had chips on it.".to_string()
                    } else {
                        prop.winners.iter().map(|(n, c)| format!("{n} +{}", chips_text(*c))).collect::<Vec<_>>().join(" · ")
                    };
                    ui.label(RichText::new(who).size(12.5).color(MUTED));
                });
            });
            ui.add_space(6.0);
        } else if prop.state == PropState::Void {
            ui.label(RichText::new("Called off: everyone's chips went back to them.").italics().color(MUTED));
            ui.add_space(6.0);
        } else if !open {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
                cover(ui.painter(), self.art.lock.as_ref(), r, Color32::WHITE);
                ui.label(RichText::new("Betting has closed: it pays out when it's settled.").color(MUTED));
            });
            ui.add_space(4.0);
        }
        for (i, name) in prop.outcomes.iter().enumerate() {
            let price = prices.get(i).copied().unwrap_or(0.0);
            let won = prop.winner == Some(i);
            let faded = prop.state == PropState::Settled && !won;
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
            let pnt = ui.painter();
            let chip = Rect::from_center_size(pos2(row.left() + 17.0, row.center().y), vec2(32.0, 32.0));
            match self.art.chips.get(i) {
                Some(t) => {
                    pnt.image(t.id(), chip, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), if faded { Color32::from_gray(90) } else { Color32::WHITE });
                }
                None => {
                    pnt.circle_filled(chip.center(), 15.0, COLORS[i % 6]);
                }
            }
            let col = COLORS[i % 6];
            let x = row.left() + 42.0;
            let bar_w = row.width() - 42.0 - 132.0;
            pnt.text(pos2(x, row.top() + 12.0), Align2::LEFT_CENTER, name, bold_font(14.0), if faded { DIM } else { TEXT });
            pnt.text(pos2(x + bar_w, row.top() + 12.0), Align2::RIGHT_CENTER, pct(price), bold_font(14.0), if faded { DIM } else { col });
            let bar = Rect::from_min_size(pos2(x, row.top() + 24.0), vec2(bar_w, 6.0));
            pnt.rect_filled(bar, CornerRadius::same(3), Color32::from_white_alpha(18));
            pnt.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * price as f32, bar.height())), CornerRadius::same(3), if faded { Color32::from_gray(70) } else { col });
            if open && !mine {
                let b = Rect::from_min_size(pos2(row.right() - 76.0, row.center().y - 15.0), vec2(76.0, 30.0));
                let picked = self.betting.as_ref().is_some_and(|bt| bt.id == prop.id && bt.outcome == i);
                let button = egui::Button::new(RichText::new("Bet").font(bold_font(13.0)).color(ON_EMBER)).fill(col).corner_radius(15);
                let button = if picked { button.stroke(Stroke::new(2.0_f32, Color32::WHITE)) } else { button };
                if ui.put(b, button).on_hover_text(format!("Bet chips on {name}: each share pays a chip if it wins")).clicked() {
                    self.betting = if picked { None } else { Some(Betting { id: prop.id, outcome: i, amount: self.betting.as_ref().map(|b| b.amount.clone()).unwrap_or_else(|| "1000".into()) }) };
                }
            }
            if self.betting.as_ref().is_some_and(|bt| bt.id == prop.id && bt.outcome == i) {
                self.bet_panel(ui, prop, i);
            }
        }
    }

    /// Picking how much to bet on an outcome: chips buttons and a box, what it pays, and Place.
    fn bet_panel(&mut self, ui: &mut egui::Ui, prop: &PropView, outcome: usize) {
        let have = self.m.chips;
        let Some(bt) = self.betting.as_mut() else { return };
        let mut done = false;
        egui::Frame::new().fill(Color32::from_black_alpha(90)).corner_radius(8).inner_margin(10).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                for (label, n) in [("100", 100u64), ("1K", 1_000), ("10K", 10_000), ("100K", 100_000), ("1M", 1_000_000)] {
                    if ui.small_button(label).clicked() {
                        bt.amount = n.to_string();
                    }
                }
                if let Some(h) = have {
                    if ui.small_button("All in").on_hover_text(format!("All {} of your chips", chips_text(h))).clicked() {
                        bt.amount = h.min(p::MAX_BET).to_string();
                    }
                }
                ui.add(egui::TextEdit::singleline(&mut bt.amount).desired_width(110.0).hint_text("chips"));
            });
            let chips: u64 = bt.amount.replace([',', ' '], "").parse().unwrap_or(0);
            let shares = if chips > 0 { p::shares_for(&prop.q, prop.b, outcome, chips as f64) } else { 0.0 };
            let name = prop.outcomes.get(outcome).cloned().unwrap_or_default();
            let line = if chips < p::MIN_BET {
                format!("At least {} chips.", p::MIN_BET)
            } else if have.is_some_and(|h| chips > h) {
                format!("You have {} chips.", chips_text(have.unwrap_or(0)))
            } else {
                let profit = shares.floor() as i64 - chips as i64;
                format!("Pays {} chips if {name} wins ({}{}).", chips_text(shares.floor() as u64), if profit >= 0 { "+" } else { "-" }, chips_text(profit.unsigned_abs()))
            };
            ui.label(RichText::new(line).size(12.5).color(MUTED));
            ui.horizontal(|ui| {
                let ok = (p::MIN_BET..=p::MAX_BET).contains(&chips) && have.is_none_or(|h| chips <= h);
                if ui.add_enabled(ok, primary_button("Place bet")).clicked() {
                    // (What it was shown to pay, less a little: if the odds moved more than
                    // that, the server says so instead.)
                    ask(asks::bet(prop.id, outcome, chips, shares * 0.98));
                    done = true;
                }
                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
        });
        if done {
            self.betting = None;
        }
    }

    /// What I hold, worth now, with Cash out.
    fn mine(&mut self, ui: &mut egui::Ui, prop: &PropView, now: i64) {
        if prop.mine.is_empty() || matches!(prop.state, PropState::Settled | PropState::Void) {
            return;
        }
        ui.add_space(4.0);
        for h in &prop.mine {
            let name = prop.outcomes.get(h.outcome).cloned().unwrap_or_default();
            let worth = prop.worth(h).max(0.0).floor() as u64;
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("You: {name}, pays {} · paid {} · worth {} now", chips_text(h.shares.floor() as u64), chips_text(h.cost), chips_text(worth))).size(12.5).color(TEXT));
                if prop.open_at(now) && ui.small_button("Cash out").on_hover_text(format!("Sell your {name} back for {} chips", chips_text(worth))).clicked() {
                    ask(asks::cash_out(prop.id, h.outcome));
                }
            });
        }
    }

    /// Settle (pick what happened) and Call off, for whoever opened it and moderators.
    fn admin(&mut self, ui: &mut egui::Ui, prop: &PropView) {
        if !prop.can_settle || matches!(prop.state, PropState::Settled | PropState::Void) {
            return;
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.menu_button(RichText::new("Settle…").font(bold_font(13.0)), |ui| {
                ui.label(RichText::new("What happened? Winners are paid a chip a share.").size(12.0).color(DIM));
                for (i, o) in prop.outcomes.iter().enumerate() {
                    if ui.button(o).clicked() {
                        ask(asks::settle(prop.id, i));
                        ui.close();
                    }
                }
            });
            if ui.small_button("Call off").on_hover_text("Everyone gets back what they put in (once others have bet, only a moderator can)").clicked() {
                ask(asks::void(prop.id));
            }
        });
    }

    /// The New prop bet window: the channel it goes in, then as `/prop`'s in Pyr.
    fn new_prop_window(&mut self, ctx: &egui::Context) {
        let Some(mut np) = self.new_prop.clone() else { return };
        let mut open = true;
        let mut done = false;
        egui::Window::new(RichText::new("New prop bet").font(bold_font(17.0)))
            .id(egui::Id::new("new-prop"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .max_width(460.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(ui.available_width().min(460.0));
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                    cover(ui.painter(), self.art.emblem.as_ref(), r, Color32::WHITE);
                    ui.add(
                        egui::Label::new(
                            RichText::new("Everyone in the channel bets Pyr Casino chips on it. The odds move with every bet; winners get a chip for every share. You back it and settle it (so you don't bet on it).")
                                .size(12.5)
                                .color(MUTED),
                        )
                        .wrap(),
                    );
                });
                ui.add_space(8.0);
                ui.label(RichText::new("CHANNEL").size(11.0).strong().color(DIM));
                let picked = np.channel.and_then(|c| self.m.channel_name(c)).map(|n| format!("#{n}")).unwrap_or_else(|| "Pick one".into());
                egui::ComboBox::from_id_salt("new-prop-channel").selected_text(picked).width(220.0).show_ui(ui, |ui| {
                    for (id, name) in &self.m.channels {
                        ui.selectable_value(&mut np.channel, Some(*id), format!("#{name}"));
                    }
                });
                ui.add_space(6.0);
                ui.label(RichText::new("QUESTION").size(11.0).strong().color(DIM));
                ui.add(egui::TextEdit::singleline(&mut np.question).hint_text("Will it rain on Saturday?").desired_width(f32::INFINITY));
                ui.add_space(6.0);
                ui.label(RichText::new("OUTCOMES").size(11.0).strong().color(DIM));
                let mut remove = None;
                for (i, o) in np.outcomes.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                        cover(ui.painter(), self.art.chips.get(i), r, Color32::WHITE);
                        ui.add(egui::TextEdit::singleline(o).desired_width(300.0).char_limit(30));
                        if i >= 2 && ui.small_button("×").on_hover_text("Take this outcome out").clicked() {
                            remove = Some(i);
                        }
                    });
                }
                if let Some(i) = remove {
                    np.outcomes.remove(i);
                }
                if np.outcomes.len() < p::MAX_OUTCOMES && ui.small_button("+ Outcome").clicked() {
                    np.outcomes.push(String::new());
                }
                ui.add_space(6.0);
                ui.label(RichText::new("HOW MUCH THE ODDS MOVE").size(11.0).strong().color(DIM));
                let n = np.outcomes.iter().filter(|o| !o.trim().is_empty()).count().max(2);
                ui.horizontal(|ui| {
                    for (d, name) in p::DEPTHS {
                        ui.selectable_value(&mut np.depth, d, name).on_hover_text(format!("Deeper moves less with each bet, and takes more to back: {} chips here.", chips_text(p::backing(n, d as f64))));
                    }
                });
                // What it takes from me, and why.
                let backing = p::backing(n, np.depth as f64);
                let short = self.m.chips.is_some_and(|h| h < backing);
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "You put up {} chips to back its odds, so every winning share gets paid. What's left of them comes back to you when you settle it.{}",
                            chips_text(backing),
                            match self.m.chips {
                                Some(h) if short => format!(" You have {}.", chips_text(h)),
                                _ => String::new(),
                            }
                        ))
                        .size(12.0)
                        .color(if short { LIVE } else { MUTED }),
                    )
                    .wrap(),
                );
                ui.add_space(6.0);
                ui.label(RichText::new("BETTING CLOSES").size(11.0).strong().color(DIM));
                ui.horizontal_wrapped(|ui| {
                    for (secs, name) in p::CLOSES_IN.into_iter().zip(["When I settle it", "In an hour", "In a day", "In a week"]) {
                        ui.selectable_value(&mut np.closes_in, secs, name);
                    }
                });
                ui.add_space(10.0);
                let check = p::check_new(&np.question, &np.outcomes);
                if let Err(e) = &check {
                    if !np.question.trim().is_empty() {
                        ui.label(RichText::new(e).size(12.0).color(LIVE));
                    }
                }
                ui.horizontal(|ui| {
                    let ok = check.is_ok() && !short && np.channel.is_some();
                    if ui.add_enabled(ok, primary_button("Open the prop")).clicked() {
                        if let (Ok((q, outcomes)), Some(channel)) = (&check, np.channel) {
                            ask(asks::open(channel, q, outcomes, np.depth, np.closes_in));
                            self.tab = Tab::Open;
                            done = true;
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        done = true;
                    }
                });
            });
        self.new_prop = if open && !done { Some(np) } else { None };
    }
}

/// How much is in, how many bets, who opened it, and the latest bets.
fn footer(ui: &mut egui::Ui, prop: &PropView) {
    ui.add_space(6.0);
    ui.add(
        egui::Label::new(
            RichText::new(format!("{} chips in · {} · opened and backed by {} ({} chips)", chips_text(prop.pool), count(prop.bets as u64, "bet", "bets"), prop.author_name, chips_text(prop.backing)))
                .size(12.0)
                .color(DIM),
        )
        .wrap(),
    );
    if !prop.latest.is_empty() {
        let lines: Vec<String> = prop
            .latest
            .iter()
            .take(4)
            .map(|b| {
                let o = prop.outcomes.get(b.outcome).cloned().unwrap_or_default();
                if b.chips >= 0 { format!("{} {} on {o}", b.name, chips_text(b.chips as u64)) } else { format!("{} cashed out {o}", b.name) }
            })
            .collect();
        ui.add(egui::Label::new(RichText::new(lines.join(" · ")).size(11.5).color(DIM)).wrap());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyr_poker::props::Holding;

    fn prop(id: u64, state: PropState, mine: bool) -> PropView {
        PropView {
            id,
            channel_id: 3,
            question: format!("Question {id}?"),
            outcomes: vec!["Yes".into(), "No".into()],
            b: 10_000.0,
            q: vec![0.0, 0.0],
            state,
            mine: if mine { vec![Holding { outcome: 0, shares: 120.0, cost: 100 }] } else { Vec::new() },
            ..Default::default()
        }
    }

    #[test]
    fn what_pyr_tells_it_is_what_it_shows() {
        let mut m = Model::default();
        assert!(!m.heard);
        let line = json!({
            "op": "props",
            "server": {"key": "k", "name": "Brethren of Blood"},
            "props_here": true,
            "me": "u1",
            "chips": 25_000,
            "channels": [{"id": 3, "name": "general"}, {"id": 4, "name": "casino"}],
            "list": [prop(4, PropState::Settled, true), prop(9, PropState::Open, false), prop(7, PropState::Closed, true), prop(2, PropState::Void, false)],
        });
        m.hear(&serde_json::from_str(&line.to_string()).unwrap());
        assert!(m.heard && m.props_here);
        assert_eq!(m.server.as_deref(), Some("Brethren of Blood"));
        assert_eq!(m.chips, Some(25_000));
        assert_eq!(m.channel_name(4), Some("casino"));
        assert_eq!(m.props.iter().map(|p| p.id).collect::<Vec<_>>(), vec![9, 7, 4, 2], "newest first");
        let ids = |tab| m.shown(tab).iter().map(|p| p.id).collect::<Vec<_>>();
        assert_eq!(ids(Tab::Open), vec![9, 7], "taking bets, or waiting to be settled");
        assert_eq!(ids(Tab::Mine), vec![7, 4]);
        assert_eq!(ids(Tab::Done), vec![4, 2]);
        // No server showing in Pyr; then what Pyr said; then Pyr closing.
        m.hear(&json!({"op": "props", "server": null, "props_here": false, "list": []}));
        assert!(m.server.is_none() && m.props.is_empty() && m.chips.is_none());
        m.hear(&json!({"op": "said", "text": "The odds moved: try again.", "error": true}));
        assert!(m.said.as_ref().is_some_and(|s| s.1 && s.0.starts_with("The odds")));
        m.hear(&json!({"op": "something new"}));
        assert!(!m.gone);
        m.hear(&json!({"op": "pyr_gone"}));
        assert!(m.gone);
    }

    #[test]
    fn what_it_asks_is_what_pyr_reads() {
        // (Pyr's side reads these very lines: `prop_bets_window_asks_for_bets_and_props` in
        // crates/app/src/plugins/programs.rs.)
        assert_eq!(asks::bet(12, 1, 500, 980.5).to_string(), r#"{"chips":500,"id":12,"min_shares":980.5,"op":"prop_bet","outcome":1}"#);
        assert_eq!(asks::cash_out(12, 0), json!({"op": "prop_cash_out", "id": 12, "outcome": 0}));
        assert_eq!(asks::open(3, "Rain?", &["Yes".into(), "No".into()], 10_000, 3600), json!({"op": "prop_open", "channel": 3, "question": "Rain?", "outcomes": ["Yes", "No"], "depth": 10_000, "closes_in": 3600}));
        assert_eq!(asks::settle(12, 0), json!({"op": "prop_settle", "id": 12, "outcome": 0}));
        assert_eq!(asks::void(12), json!({"op": "prop_void", "id": 12}));
        assert_eq!(asks::refresh(), json!({"op": "props_refresh"}));
    }

    #[test]
    fn words_read_as_in_pyr() {
        assert_eq!(chips_text(1_234_567), "1,234,567");
        assert_eq!(count(1, "bet", "bets"), "1 bet");
        assert_eq!(count(1204, "bet", "bets"), "1,204 bets");
        assert_eq!(pct(0.625), "63%");
        assert_eq!(time_left(30_000), "in 1 min");
        assert_eq!(time_left(2 * 3_600_000 + 5_000), "in 2 h");
        assert_eq!(time_left(3 * 86_400_000), "in 3 days");
    }
}
