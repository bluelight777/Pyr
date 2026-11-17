//! Prop Bets, in the Pyr Library: every prop bet on the Pyr server showing in Pyr, in a window of
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

#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod asks;
mod look;
mod model;
mod pages;
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
/// Nothing from Pyr by then: it's a Pyr from before Prop Bets (one that knows it tells it at
/// once).
const TOO_OLD_AFTER: Duration = Duration::from_secs(6);
/// The window is wide enough for the categories down its side from this width.
const WIDE: f32 = 820.0;

fn main() -> eframe::Result<()> {
    let bridge = std::env::var_os("PYR_BRIDGE").is_some();
    // (Inside Pyr's window, Pyr shows it once it's in its place.)
    let hidden = std::env::var_os("PYR_EMBED").is_some();
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Prop Bets")
        .with_app_id("prop-bets")
        .with_inner_size([1100.0, 780.0])
        .with_min_inner_size([340.0, 320.0])
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
                // (Only what changed each time, from a Pyr that knows how.)
                asks::ask(asks::hello());
            }
            Ok(Box::new(App::new(&cc.egui_ctx, rx, bridge)))
        }),
    )
}

/// The page showing.
#[derive(Clone, Debug, PartialEq)]
pub enum View {
    Home,
    Category(String),
    Prop(u64),
    Mine,
    Leaders,
    Results,
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
}

impl App {
    fn new(ctx: &egui::Context, rx: mpsc::Receiver<Value>, bridge: bool) -> App {
        style(ctx);
        App { rx, bridge, started: Instant::now(), m: Model::default(), art: Art::load(ctx), view: View::Home, back: Vec::new(), betting: None, new_prop: None, setting: None, days: 7, stats_asked: None, looked: None, shown_chips: 0.0, came: Instant::now() }
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
            self.came = Instant::now();
        }
    }

    pub fn go_back(&mut self) {
        self.view = self.back.pop().unwrap_or(View::Home);
        self.betting = None;
        self.came = Instant::now();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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
            self.view = View::Home;
        }
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
        // prop's own page. The mouse's back button goes back.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.new_prop.take().is_none() && self.setting.take().is_none() && self.betting.take().is_none() && matches!(self.view, View::Prop(_)) {
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
            return note(ui, "Open Prop Bets from Pyr", "It bets your Pyr Casino chips on your Pyr server's props, so it opens from Pyr: in Plugins, Prop Bets, then Open.");
        }
        if !self.m.heard {
            // (A Pyr from before Prop Bets opens it as any program, and never tells it the props.)
            if self.started.elapsed() > TOO_OLD_AFTER {
                return note(ui, "Update Pyr for Prop Bets", "This Pyr is older than Prop Bets, so it doesn't tell it the props: update Pyr, then open Prop Bets again.");
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
            return note(ui, "Go to a server in Pyr", "Prop Bets shows the props of the server you were last on in Pyr: go to one, then come back here.");
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
                    ui.label(RichText::new("Prop Bets").font(bold_font(18.0)).color(GOLD));
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
