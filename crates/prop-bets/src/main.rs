//! Pyr Prop Bets, in the Pyr Library: every prop bet on the Pyr server showing in Pyr, in a window of
//! its own (in Pyr's, or popped out), with your Pyr Casino chips. Pyr starts it and the two talk a
//! JSON object a line (see `crates/app/src/game_bridge.rs`): Pyr tells it the props as they
//! change, my chips and the server's text channels, and says what happened (a win, or what the
//! server said no to); it asks Pyr to bet, cash out, open, settle or call off a prop (`@pyr {...}`
//! on its output), and Pyr does that on the server just as a prop's card in chat does. The server
//! checks everything: this window only shows, and asks. Started by itself, it says to open it
//! from Pyr.
//!
//! 0.2 (Pyr 0.15): props come in categories, and the casino has props of its own that open and
//! settle by themselves (games, prices, the weather: `docs/PROPS.md`). So the window is a
//! place to look around in: a front page, a page a category with each game's props together,
//! a page a prop with how its odds went, what I hold and what it's worth, and the server's best.
//!
//! 0.3 (Pyr 0.16): parlays (`parlay.rs`): a slip that picks go on from anywhere, its page, and
//! my parlays as tickets.
//!
//! 0.3.1: a frame that can't be shown no longer leaves what it drew waiting on the graphics card
//! (see `surface_error`). 0.3.2: frames that keep failing (its size changed without it hearing)
//! put themselves right (see `unshown`).

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod asks;
mod look;
mod model;
mod pages;
mod parlay;
mod views;
mod words;

use std::io::BufRead;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, Sense, vec2};
use serde_json::{Value, json};

use look::*;
use model::Model;

/// How long something Pyr said shows at the top.
const SAID_FOR: Duration = Duration::from_secs(8);
/// Nothing from Pyr by then: it's a Pyr from before Pyr Prop Bets (one that knows it tells it at
/// once).
const TOO_OLD_AFTER: Duration = Duration::from_secs(6);
/// The window is wide enough for the categories down its side from this width.
const WIDE: f32 = 820.0;

fn main() -> eframe::Result<()> {
    let bridge = std::env::var_os("PYR_BRIDGE").is_some();
    // (Inside Pyr's window, Pyr shows it once it's in its place.)
    let hidden = std::env::var_os("PYR_EMBED").is_some();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Pyr Prop Bets")
        .with_app_id("prop-bets")
        .with_inner_size([1100.0, 780.0])
        .with_min_inner_size([340.0, 320.0])
        .with_visible(!hidden);
    if let Ok(img) = image::load_from_memory(EMBLEM) {
        let img = img.to_rgba8();
        viewport = viewport.with_icon(egui::IconData { width: img.width(), height: img.height(), rgba: img.into_raw() });
    }
    let mut options = eframe::NativeOptions { viewport, ..Default::default() };
    options.wgpu_options.on_surface_error = std::sync::Arc::new(surface_error);
    eframe::run_native(
        "Pyr Prop Bets",
        options,
        Box::new(move |cc| {
            // (For `surface_error`: the graphics card's queue.)
            if let Some(rs) = &cc.wgpu_render_state {
                *GPU_QUEUE.lock().unwrap_or_else(|e| e.into_inner()) = Some(rs.queue.clone());
            }
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
                // (Only what changed each time, from a Pyr that knows how.)
                asks::ask(asks::hello());
            }
            // (`PROP_BETS_TRY_UNSHOWN`, for trying `unshown` on a test screen: as if the last
            // three frames, over two seconds, couldn't be shown.)
            if std::env::var_os("PROP_BETS_TRY_UNSHOWN").is_some() {
                let now = Instant::now();
                *UNSHOWN.lock().unwrap_or_else(|e| e.into_inner()) = Some((now - Duration::from_secs(2), 3, now, "pretend"));
            }
            Ok(Box::new(App::new(&cc.egui_ctx, rx, bridge)))
        }),
    )
}

/// The graphics card's queue (wgpu), for `surface_error`.
static GPU_QUEUE: std::sync::Mutex<Option<eframe::wgpu::Queue>> = std::sync::Mutex::new(None);

/// A frame that couldn't be shown (Blue, Nov 19, on Pyr 0.16.1: "Pyr Prop bets is just frozen"):
/// the window covered by a game full screen, hidden or minimized with Pyr, the screen off or
/// locked, the graphics card busy. egui has handed the frame's shapes and pictures to the
/// graphics card's queue by then, and there they waited for the next frame that could be shown:
/// this window draws every second and whenever Pyr tells it something, so in an hour of that
/// they'd come to gigabytes, and the next frame shown had all of it to get through first (Pyr's
/// own window had the same, bug report 35, and has been sending them on since 0.11.0). Sent on
/// now, they're done with at once. Said on its errors (Pyr keeps the last of them), at most
/// every 10 s.
fn surface_error(err: eframe::wgpu::SurfaceError) -> eframe::egui_wgpu::SurfaceErrorAction {
    if let Some(q) = GPU_QUEUE.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        q.submit(std::iter::empty());
    }
    // (Each kind, "outdated" too: see `unshown`.)
    static LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if last.is_none_or(|t| t.elapsed() >= Duration::from_secs(10)) {
        *last = Some(Instant::now());
        eprintln!("a frame couldn't be shown: {err}");
    }
    use eframe::wgpu::SurfaceError as E;
    let kind = match err {
        E::Outdated => "outdated",
        E::Timeout => "timed out",
        E::Lost => "lost",
        E::OutOfMemory => "out of memory",
        _ => "other",
    };
    let mut u = UNSHOWN.lock().unwrap_or_else(|e| e.into_inner());
    let now = Instant::now();
    *u = match *u {
        Some((first, n, at, _)) if now - at < UNSHOWN_GAP => Some((first, n + 1, now, kind)),
        _ => Some((now, 1, now, kind)),
    };
    eframe::egui_wgpu::SurfaceErrorAction::SkipFrame
}

/// Frames that couldn't be shown, one after another (none more than `UNSHOWN_GAP` apart): since
/// when, how many, the last, and why the last couldn't.
static UNSHOWN: std::sync::Mutex<Option<(Instant, u32, Instant, &'static str)>> = std::sync::Mutex::new(None);
const UNSHOWN_GAP: Duration = Duration::from_secs(3);

/// Frames that keep failing ("outdated": the window's size changed without this hearing of it,
/// which egui only puts right on the next change of size): Blue, Nov 20, inside Pyr 0.16.2, "it's
/// just frozen on this screen, buttons do not work". Pyr took the frame off this window just
/// after it showed itself, and it drew on at the size it had with its frame (a black strip along
/// its bottom and right as wide as a frame), then not at all. So after three in a row, a second
/// or more apart in all, its size is changed and changed back (once every 5 s at most, never
/// minimized), and it hears its size again. Says so on its errors (Pyr keeps them in pyr.log).
fn unshown(ctx: &egui::Context, nudged: &mut Option<Instant>, back: &mut Option<egui::Vec2>) {
    if let Some(size) = back.take() {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
        return;
    }
    let Some((first, n, at, kind)) = *UNSHOWN.lock().unwrap_or_else(|e| e.into_inner()) else { return };
    if at.elapsed() >= UNSHOWN_GAP {
        return;
    }
    // (While frames fail, the next comes soon: one a second otherwise.)
    ctx.request_repaint_after(Duration::from_millis(250));
    let minimized = ctx.input(|i| i.viewport().minimized) == Some(true);
    if n < 3 || first.elapsed() < Duration::from_secs(1) || minimized || nudged.is_some_and(|t| t.elapsed() < Duration::from_secs(5)) {
        return;
    }
    let Some(size) = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size())) else { return };
    eprintln!("{n} frames in a row couldn't be shown ({kind}), over {:.1} s: changing the window's size and back", first.elapsed().as_secs_f32());
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size + vec2(0.0, 1.0)));
    *nudged = Some(Instant::now());
    *back = Some(size);
}

/// The page showing.
#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Home,
    Category(String),
    Prop(u64),
    Mine,
    /// The parlay slip, what there is to pick from, and my parlays.
    Parlay,
    Leaders,
    Results,
    /// What the search box found for these words.
    Search(String),
}

/// A bet being picked: the prop, the outcome, how much (as typed).
pub struct Betting {
    pub id: u64,
    pub outcome: usize,
    pub amount: String,
}

/// The New prop bet form.
#[derive(Clone)]
pub struct NewProp {
    pub channel: Option<i64>,
    pub question: String,
    pub outcomes: Vec<String>,
    pub depth: u64,
    pub closes_in: u64,
    pub category: String,
}

/// The casino's own props, being set (whoever manages the server).
#[derive(Clone)]
pub struct Setting {
    pub on: bool,
    pub off: Vec<String>,
    pub depth: u64,
}

pub struct App {
    rx: mpsc::Receiver<Value>,
    pub bridge: bool,
    started: Instant,
    pub m: Model,
    pub art: Art,
    pub view: View,
    /// The pages before this one (Back).
    pub back: Vec<View>,
    pub betting: Option<Betting>,
    pub new_prop: Option<NewProp>,
    pub setting: Option<Setting>,
    /// The best at props: over how many days, and when they were last asked for.
    pub days: u32,
    pub stats_asked: Option<(u32, Instant)>,
    /// The prop whose page was last looked up (with how its odds went), and when.
    pub looked: Option<(u64, Instant)>,
    /// My chips as shown (they run up or down to what they are).
    pub shown_chips: f64,
    /// When the page showing came (it fades in).
    pub came: Instant,
    /// What's typed in the search box.
    pub search: String,
    /// The parlay being put together, the category its page shows to pick from, and whether
    /// the page showing is that one (there a click on an outcome is a pick).
    pub slip: parlay::Slip,
    pub slip_cat: Option<String>,
    pub picking: bool,
    /// The Parlays page shows its other side: my parlays as tickets (not the slip and what
    /// there is to pick from).
    pub parlay_mine: bool,
    /// When the window's size was last changed and back (see `unshown`), and the size it goes
    /// back to on the next frame.
    nudged: Option<Instant>,
    nudge_back: Option<egui::Vec2>,
}

impl App {
    fn new(ctx: &egui::Context, rx: mpsc::Receiver<Value>, bridge: bool) -> App {
        style(ctx);
        App { rx, bridge, started: Instant::now(), m: Model::default(), art: Art::load(ctx), view: View::Home, back: Vec::new(), betting: None, new_prop: None, setting: None, days: 7, stats_asked: None, looked: None, shown_chips: 0.0, came: Instant::now(), search: String::new(), slip: parlay::Slip::default(), slip_cat: None, picking: false, parlay_mine: false, nudged: None, nudge_back: None }
    }

    /// Goes to a page (the one showing can be come back to).
    pub fn go(&mut self, view: View) {
        if view != self.view {
            let from = std::mem::replace(&mut self.view, view);
            // (A page from the side isn't a step further in: Back goes to where a prop was opened from.)
            if matches!(self.view, View::Prop(_)) {
                self.back.push(from);
            } else {
                self.back.clear();
            }
            self.betting = None;
            self.search.clear();
            self.came = Instant::now();
        }
    }

    pub fn go_back(&mut self) {
        self.view = self.back.pop().unwrap_or(View::Home);
        self.betting = None;
        // (Back to what was found: the words are in the box again.)
        self.search = match &self.view {
            View::Search(q) => q.clone(),
            _ => String::new(),
        };
        self.came = Instant::now();
    }

    /// The search box changed: what's typed is looked for (a page of its own, which Back
    /// leaves), and with nothing typed it's back to where the looking started.
    pub fn searched(&mut self) {
        let q = self.search.trim().to_string();
        let searching = matches!(self.view, View::Search(_));
        match (searching, q.is_empty()) {
            (true, true) => self.view = self.back.pop().unwrap_or(View::Home),
            (true, false) => self.view = View::Search(q),
            (false, false) => {
                let from = std::mem::replace(&mut self.view, View::Search(q));
                self.back.push(from);
                self.betting = None;
            }
            (false, true) => {}
        }
    }

    /// The search box: every prop here with those words in it.
    pub fn search_box(&mut self, ui: &mut egui::Ui, width: f32) {
        let field = egui::TextEdit::singleline(&mut self.search).desired_width(width).hint_text("Search props").margin(egui::Margin::symmetric(10, 6));
        if ui.add(field).changed() {
            self.searched();
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        unshown(ctx, &mut self.nudged, &mut self.nudge_back);
        let was = self.m.server_key.clone();
        while let Ok(v) = self.rx.try_recv() {
            self.m.hear(&v);
        }
        // Another server showing in Pyr: what was being done was for the one before.
        if self.m.server_key != was {
            self.betting = None;
            self.new_prop = None;
            self.setting = None;
            self.stats_asked = None;
            self.looked = None;
            self.back.clear();
            self.search.clear();
            self.view = View::Home;
            self.slip = parlay::Slip::default();
            self.slip_cat = None;
            self.parlay_mine = false;
        }
        // The parlay on the slip was placed: the slip is done with, and its ticket is there
        // to see. And what's no longer a pick goes from the slip.
        if std::mem::take(&mut self.m.parlay_on) {
            self.slip.legs.clear();
            self.slip.placing = None;
            self.parlay_mine = true;
        }
        self.slip_tidy();
        self.picking = self.view == View::Parlay && !self.parlay_mine;
        if self.m.gone {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        // A prop that's gone (forgotten by the server): back to where it was opened from.
        if let View::Prop(id) = self.view {
            if self.m.heard && !self.m.props.contains_key(&id) {
                self.go_back();
            }
        }
        // Escape closes what's open over the page: a window, the bet being picked, then a
        // prop's own page or what the search found. The mouse's back button goes back.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.new_prop.take().is_none() && self.setting.take().is_none() && self.betting.take().is_none() && matches!(self.view, View::Prop(_) | View::Search(_)) {
            self.go_back();
        }
        if ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1)) && matches!(self.view, View::Prop(_)) {
            self.go_back();
        }
        // (Betting closes by the clock, countdowns tick, and what Pyr said goes.)
        ctx.request_repaint_after(Duration::from_secs(1));
        // My chips run to what they are.
        let chips = self.m.chips.unwrap_or(0) as f64;
        if (self.shown_chips - chips).abs() > 0.5 {
            let step = (chips - self.shown_chips) * (ctx.input(|i| i.stable_dt).min(0.1) as f64 * 9.0).min(1.0);
            self.shown_chips = if step.abs() < 1.0 { chips } else { self.shown_chips + step };
            ctx.request_repaint();
        }
        egui::TopBottomPanel::top("head").frame(egui::Frame::new().fill(SIDE).inner_margin(egui::Margin::symmetric(14, 9))).show(ctx, |ui| self.head(ui));
        let ready = self.bridge && self.m.heard && self.m.server.is_some() && self.m.props_here;
        let wide = ctx.content_rect().width() >= WIDE;
        if ready && wide {
            egui::SidePanel::left("rail").exact_width(214.0).resizable(false).frame(egui::Frame::new().fill(SIDE).inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 8 })).show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| self.rail(ui));
            });
        }
        egui::CentralPanel::default().frame(egui::Frame::new().fill(MAIN).inner_margin(egui::Margin::symmetric(16, 12))).show(ctx, |ui| {
            if !ready {
                return self.not_ready(ui);
            }
            self.said(ui);
            if !wide {
                self.strip(ui);
                ui.add_space(6.0);
            }
            self.page(ui);
        });
        self.new_prop_window(ctx);
        self.setting_window(ctx);
        self.celebrate(ctx);
    }
}

/// A word in the middle of the window.
pub fn note(ui: &mut egui::Ui, title: &str, words: &str) {
    ui.add_space(48.0);
    ui.vertical_centered(|ui| {
        ui.set_max_width(420.0);
        ui.label(RichText::new(title).font(bold_font(18.0)).color(TEXT));
        ui.add_space(4.0);
        ui.label(RichText::new(words).color(MUTED));
    });
}

impl App {
    /// Why there's nothing to show yet.
    fn not_ready(&mut self, ui: &mut egui::Ui) {
        if !self.bridge {
            return note(ui, "Open Pyr Prop Bets from Pyr", "It bets your Pyr Casino chips on your Pyr server's props, so it opens from Pyr: in Plugins, Pyr Prop Bets, then Open.");
        }
        if !self.m.heard {
            // (A Pyr from before Pyr Prop Bets opens it as any program, and never tells it the props.)
            if self.started.elapsed() > TOO_OLD_AFTER {
                return note(ui, "Update Pyr to use Pyr Prop Bets", "This Pyr is older than Pyr Prop Bets, so it doesn't tell it the props: update Pyr, then open Pyr Prop Bets again.");
            }
            ui.ctx().request_repaint_after(Duration::from_millis(500));
            ui.add_space(48.0);
            ui.vertical_centered(|ui| {
                ui.spinner();
                ui.label(RichText::new("Getting the props from Pyr…").color(MUTED));
            });
            return;
        }
        if self.m.server.is_none() {
            return note(ui, "Go to a server in Pyr", "Pyr Prop Bets shows the props of the server you were last on in Pyr: go to one, then come back here.");
        }
        note(ui, "No prop bets on this server", "Its owner's Pyr server needs Pyr 0.14.0 or newer for prop bets.")
    }

    /// Its emblem and name, the server's name, my chips; New prop, and the casino's own props
    /// (for whoever manages the server).
    fn head(&mut self, ui: &mut egui::Ui) {
        let ready = self.bridge && self.m.server.is_some() && self.m.props_here;
        // (A narrow window keeps its room for my chips and the buttons.)
        let narrow = ready && ui.available_width() < 540.0;
        ui.horizontal(|ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::click());
            cover(ui.painter(), self.art.emblem.as_ref(), r, Color32::WHITE);
            if resp.on_hover_text("The front page").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.go(View::Home);
            }
            if !narrow {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(RichText::new("Pyr Prop Bets").font(bold_font(18.0)).color(GOLD));
                    let under = match &self.m.server {
                        Some(s) => s.clone(),
                        None => "Bets with your Pyr Casino chips".into(),
                    };
                    ui.label(RichText::new(under).size(12.5).color(MUTED));
                });
            }
            if !ready {
                return;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.m.auto.as_ref().is_some_and(|a| a.can_set) {
                    if ui.add(egui::Button::new(RichText::new("⚙").size(16.0))).on_hover_text("The casino's own props on this server: on or off, and which").clicked() && self.setting.is_none() {
                        let a = self.m.auto.clone().unwrap_or_default();
                        self.setting = Some(Setting { on: a.on, off: a.off, depth: a.depth });
                    }
                }
                if ui.add(egui::Button::new(RichText::new("↻").size(16.0))).on_hover_text("Every prop again, from the server").clicked() {
                    asks::ask(asks::refresh());
                }
                if ui.add(primary_button("New prop")).on_hover_text("Open a prop bet of your own in one of this server's channels").clicked() && self.new_prop.is_none() {
                    let category = match &self.view {
                        View::Category(c) => c.clone(),
                        _ => pyr_poker::props::COMMUNITY.to_string(),
                    };
                    self.new_prop = Some(NewProp { channel: self.m.channel.filter(|c| self.m.channel_name(*c).is_some()).or(self.m.channels.first().map(|c| c.0)), question: String::new(), outcomes: vec!["Yes".into(), "No".into()], depth: pyr_poker::props::DEPTHS[0].0, closes_in: 0, category });
                }
                // My chips, on one of the jeweled chips' gold.
                if self.m.chips.is_some() {
                    let text = words::chips_text(self.shown_chips.round().max(0.0) as u64);
                    let g = ui.painter().layout_no_wrap(text, bold_font(15.0), GOLD);
                    let (r, _) = ui.allocate_exact_size(vec2(g.size().x + 44.0, 30.0), Sense::hover());
                    ui.painter().rect_filled(r, egui::CornerRadius::same(15), Color32::from_rgb(0x33, 0x2a, 0x16));
                    if let Some(t) = self.art.chips.get(3) {
                        ui.painter().image(t.id(), egui::Rect::from_center_size(r.left_center() + vec2(17.0, 0.0), vec2(22.0, 22.0)), egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    ui.painter().galley(egui::pos2(r.left() + 32.0, r.center().y - g.size().y / 2.0), g, GOLD);
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
}
