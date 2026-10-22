//! Tournaments in the poker room: the list (what's coming, what's on, what's just over), a
//! tournament's own lobby (blinds, prizes, standings, my table), and setting one up.

use std::time::Instant;

use eframe::egui::{self, Align, Color32, Layout, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::state::ServerView;
use pyr_poker::bots::Level;
use pyr_poker::game::{Limit, Variant};
use pyr_poker::net::{ToServer, TourneyId, TourneyPhase, TourneyRules, TourneySummary, TourneyView};
use pyr_poker::tourney::{payouts, standard_levels};
use pyr_proto::control::perm;

use super::cards::{chips_short, chips_text};
use super::section;
use crate::app::PyrApp;
use crate::theme;

/// What a new tournament will be.
#[derive(Clone)]
pub struct NewTourney {
    pub name: String,
    pub variant: Variant,
    pub limit: Limit,
    pub seats: usize,
    pub stack: u64,
    pub buy_in: u64,
    /// Minutes from now.
    pub start_in: u64,
    pub level_min: u64,
    pub max_players: usize,
    pub late_levels: u32,
    pub bots: usize,
    pub bot_level: Level,
}

impl Default for NewTourney {
    fn default() -> Self {
        NewTourney {
            name: String::new(),
            variant: Variant::Holdem,
            limit: Limit::NoLimit,
            seats: 9,
            stack: 3000,
            buy_in: 1000,
            start_in: 15,
            level_min: 6,
            max_players: 90,
            late_levels: 3,
            bots: 0,
            bot_level: Level::Regular,
        }
    }
}

fn wall() -> u64 {
    pyr_proto::now_ms().max(0) as u64
}

/// "in 12 min", "in 1 h 5 min", "now".
pub fn until(ms: u64) -> String {
    let now = wall();
    if ms <= now {
        return "now".into();
    }
    let s = (ms - now) / 1000;
    if s < 60 {
        format!("in {s} s")
    } else if s < 3600 {
        format!("in {} min", s.div_ceil(60))
    } else if s < 86_400 {
        format!("in {} h {} min", s / 3600, (s % 3600) / 60)
    } else {
        format!("in {} days", s / 86_400)
    }
}

/// "3:07" left on a clock.
fn clock(ms: u64) -> String {
    let s = ms.saturating_sub(wall()) / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn ordinal(n: usize) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn state_line(t: &TourneySummary) -> (String, Color32) {
    match t.phase {
        TourneyPhase::Registering => (format!("Starts {}", until(t.rules.start_ms)), theme::OWNER()),
        TourneyPhase::Running => (format!("Level {} · {} of {} left", t.level + 1, t.players_left, t.entrants), theme::ONLINE()),
        TourneyPhase::Finished => ("Finished".into(), theme::MUTED()),
        TourneyPhase::Cancelled => ("Called off".into(), theme::DIM()),
    }
}

impl PyrApp {
    pub fn tourneys_list(&mut self, ui: &mut egui::Ui, s: &ServerView) {
        let Some(list) = s.poker.tourneys.clone().map(|l| l.into_iter().filter(|t| t.rules.bomb.is_none()).collect::<Vec<_>>()) else {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                ui.label(RichText::new("Finding tournaments…").color(theme::MUTED()));
            });
            return;
        };
        if list.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No tournaments yet").font(theme::bold_font(18.0)).color(theme::TEXT()));
                let tip = if s.can(perm::MANAGE_SERVER) {
                    "Set one up with New tournament: a start time, a buy-in, blinds that climb, and prizes for the top places."
                } else {
                    "When this server sets one up, it shows here: register, and your Pyr takes you to your table when it starts."
                };
                ui.label(RichText::new(tip).color(theme::MUTED()));
            });
            return;
        }
        // Coming up and on first, then the finished.
        let mut list = list;
        list.sort_by_key(|t| (matches!(t.phase, TourneyPhase::Finished | TourneyPhase::Cancelled), t.rules.start_ms));
        for t in list {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                let w = (ui.available_width() - 28.0).max(300.0);
                let (r, resp) = ui.allocate_exact_size(vec2(w, 78.0), Sense::click());
                let p = ui.painter();
                p.rect_filled(r, theme::r(12), theme::row(if resp.hovered() { theme::ELEV2() } else { theme::ELEV() }));
                if t.registered && matches!(t.phase, TourneyPhase::Registering | TourneyPhase::Running) {
                    p.rect_stroke(r, theme::r(12), Stroke::new(1.5f32, theme::OWNER().gamma_multiply(0.7)), StrokeKind::Inside);
                }
                // A trophy-ish badge: the prize pool.
                let badge = Rect::from_center_size(r.left_center() + vec2(44.0, 0.0), vec2(60.0, 54.0));
                p.rect_filled(badge, theme::r(10), Color32::from_rgb(0x3a, 0x2a, 0x10));
                p.text(badge.center() + vec2(0.0, -9.0), egui::Align2::CENTER_CENTER, "🏆", theme::font(16.0), Color32::WHITE);
                p.text(badge.center() + vec2(0.0, 13.0), egui::Align2::CENTER_CENTER, chips_short(t.prize_pool), theme::bold_font(12.0), Color32::from_rgb(0xff, 0xd2, 0x5e));
                let x = r.left() + 90.0;
                p.text(pos2(x, r.top() + 22.0), egui::Align2::LEFT_CENTER, &t.rules.name, theme::bold_font(16.0), theme::TEXT());
                let game = format!("{} · {} chips · {} a table", game_name(&t.rules), chips_text(t.rules.stack), t.rules.seats);
                p.text(pos2(x, r.top() + 46.0), egui::Align2::LEFT_CENTER, game, theme::font(13.0), theme::MUTED());
                let (line, col) = state_line(&t);
                p.text(pos2(x, r.top() + 64.0), egui::Align2::LEFT_CENTER, line, theme::bold_font(12.0), col);
                let mid = r.left() + w * 0.5;
                let fee = if t.rules.buy_in == 0 { "Free".to_string() } else { format!("Buy-in {}", chips_text(t.rules.buy_in)) };
                p.text(pos2(mid, r.top() + 26.0), egui::Align2::LEFT_CENTER, fee, theme::bold_font(14.0), theme::BODY());
                p.text(pos2(mid, r.top() + 50.0), egui::Align2::LEFT_CENTER, format!("{} / {} players", t.entrants, t.rules.max_players), theme::font(13.0), theme::DIM());
                let b = Rect::from_min_size(pos2(r.right() - 130.0, r.center().y - 18.0), vec2(114.0, 36.0));
                let label = match (t.phase, t.registered) {
                    (TourneyPhase::Registering, false) => "Register",
                    (TourneyPhase::Running, true) => "Play",
                    _ => "Open",
                };
                if ui.put(b, theme::primary_button(label)).clicked() || resp.clicked() {
                    self.open_tourney(&s.key, t.id);
                }
            });
            ui.add_space(8.0);
        }
    }

    pub fn open_tourney(&mut self, key: &str, id: TourneyId) {
        self.poker.tourney = Some(id);
        self.engine.poker(key, ToServer::TourneyWatch { id });
    }

    /// A tournament's lobby.
    pub fn tourney_page(&mut self, ui: &mut egui::Ui, s: &ServerView, id: TourneyId) {
        let Some(v) = s.poker.tourney.get(&id).cloned() else {
            ui.centered_and_justified(|ui| ui.label(RichText::new("Opening the tournament…").color(theme::MUTED())));
            return;
        };
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        let me = s.me.as_ref().map(|m| m.user_id.clone()).unwrap_or_default();
        let mine = v.entrants.iter().find(|e| e.uid == me).cloned();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                if ui.add(egui::Button::new(RichText::new("‹ Pyr Casino").font(theme::bold_font(13.5))).corner_radius(theme::r(8))).clicked() {
                    self.engine.poker(&s.key, ToServer::TourneyUnwatch { id });
                    self.poker.tourney = None;
                }
            });
            ui.add_space(10.0);
            // The header: name, what it is, and where it stands.
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&v.rules.name).font(theme::bold_font(26.0)).color(theme::TEXT()));
                    let fee = if v.rules.buy_in == 0 { "free to enter".to_string() } else { format!("{} chips to enter", chips_text(v.rules.buy_in)) };
                    ui.label(RichText::new(format!("{} · {} · starting stack {} · set up by {}", game_name(&v.rules), fee, chips_text(v.rules.stack), v.host)).color(theme::MUTED()));
                });
            });
            // A Flash Pot that's over: another, at the same buy-in.
            if v.bomb.is_some() && matches!(v.phase, TourneyPhase::Finished | TourneyPhase::Cancelled) {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add_space(28.0);
                    let label = format!("Play another Flash Pot · {}", chips_text(v.rules.buy_in));
                    if ui.add(theme::primary_button(&label)).clicked() {
                        self.engine.poker(&s.key, ToServer::TimeBomb { buy_in: v.rules.buy_in });
                        self.engine.poker(&s.key, ToServer::TourneyUnwatch { id });
                        self.poker.tourney = None;
                        self.poker.tab = super::Tab::Tables;
                    }
                });
            }
            ui.add_space(14.0);
            self.tourney_status(ui, s, &v, mine.as_ref());
            ui.add_space(16.0);
            // Two columns: prizes and blinds | standings.
            ui.horizontal_top(|ui| {
                ui.add_space(28.0);
                let w = ((ui.available_width() - 28.0 - 20.0) * 0.36).max(240.0);
                ui.allocate_ui(vec2(w, 10.0), |ui| {
                    ui.set_width(w);
                    ui.vertical(|ui| {
                        self.prizes(ui, &v);
                        ui.add_space(16.0);
                        self.blinds_sheet(ui, &v);
                    });
                });
                ui.add_space(20.0);
                ui.vertical(|ui| {
                    ui.set_width((ui.available_width() - 28.0).max(260.0));
                    self.standings(ui, s, &v, &me);
                });
            });
            ui.add_space(30.0);
        });
        if v.phase == TourneyPhase::Registering && s.can(perm::MANAGE_SERVER) {
            // (Calling off is in the status card.)
        }
    }

    fn tourney_status(&mut self, ui: &mut egui::Ui, s: &ServerView, v: &TourneyView, mine: Option<&pyr_poker::net::EntrantView>) {
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            let w = (ui.available_width() - 28.0).max(300.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 96.0), Sense::hover());
            let p = ui.painter().clone();
            p.rect_filled(r, theme::r(14), theme::ELEV());
            p.rect_stroke(r, theme::r(14), Stroke::new(1.0f32, theme::LINE()), StrokeKind::Inside);
            let big = |x: f32, top: &str, val: String, col: Color32| {
                p.text(pos2(x, r.top() + 28.0), egui::Align2::LEFT_CENTER, top, theme::font(12.5), theme::MUTED());
                p.text(pos2(x, r.top() + 58.0), egui::Align2::LEFT_CENTER, val, theme::bold_font(22.0), col);
            };
            let left = v.entrants.iter().filter(|e| e.place.is_none()).count();
            match v.phase {
                TourneyPhase::Registering => {
                    big(r.left() + 24.0, "Starts", until(v.rules.start_ms), theme::OWNER());
                    big(r.left() + 24.0 + w * 0.22, "Players", format!("{} / {}", v.entrants.len(), v.rules.max_players), theme::TEXT());
                }
                TourneyPhase::Running => {
                    let b = v.rules.levels[v.level.min(v.rules.levels.len() - 1)];
                    let blinds = if b.ante > 0 { format!("{}/{} ({})", chips_short(b.small), chips_short(b.big), chips_short(b.ante)) } else { format!("{}/{}", chips_short(b.small), chips_short(b.big)) };
                    big(r.left() + 24.0, &format!("Level {}", v.level + 1), blinds, theme::TEXT());
                    big(r.left() + 24.0 + w * 0.22, "Next level", v.next_level_ms.map(clock).unwrap_or_else(|| "—".into()), theme::OWNER());
                    big(r.left() + 24.0 + w * 0.40, "Players left", format!("{left} / {}", v.entrants.len()), theme::TEXT());
                }
                TourneyPhase::Finished => {
                    let winner = v.entrants.iter().find(|e| e.place == Some(1)).map(|e| e.name.clone()).unwrap_or_default();
                    big(r.left() + 24.0, "Winner", winner, Color32::from_rgb(0xff, 0xd2, 0x4a));
                    big(r.left() + 24.0 + w * 0.40, "Players", v.entrants.len().to_string(), theme::TEXT());
                }
                TourneyPhase::Cancelled => big(r.left() + 24.0, "Called off", "Buy-ins went back".into(), theme::MUTED()),
            }
            if v.phase != TourneyPhase::Cancelled {
                big(r.left() + 24.0 + w * 0.58, "Prize pool", chips_text(v.prize_pool), Color32::from_rgb(0xff, 0xd2, 0x5e));
            }
            // What I can do.
            let b = Rect::from_min_size(pos2(r.right() - 170.0, r.center().y - 20.0), vec2(150.0, 40.0));
            match (v.phase, mine) {
                (TourneyPhase::Registering, None) | (TourneyPhase::Running, None) => {
                    let late = v.phase == TourneyPhase::Running;
                    let open = (!late || v.level < v.rules.late_levels as usize) && v.entrants.len() < v.rules.max_players;
                    if open {
                        let label = if v.rules.buy_in == 0 { "Register".to_string() } else { format!("Register · {}", chips_short(v.rules.buy_in)) };
                        if ui.put(b, theme::primary_button(&label)).on_hover_text("The buy-in comes off your chips now; it's yours back if you unregister before the start, or if it's called off.").clicked() {
                            self.engine.poker(&s.key, ToServer::TourneyRegister { id: v.id });
                        }
                    } else {
                        p.text(b.center(), egui::Align2::CENTER_CENTER, if late { "Registration closed" } else { "Full" }, theme::bold_font(14.0), theme::MUTED());
                    }
                }
                (TourneyPhase::Registering, Some(_)) => {
                    p.text(pos2(b.left(), r.top() + 18.0), egui::Align2::LEFT_CENTER, "✓ You're in", theme::bold_font(13.0), theme::ONLINE());
                    if ui.put(b, egui::Button::new("Unregister").corner_radius(theme::r(10))).clicked() {
                        self.engine.poker(&s.key, ToServer::TourneyUnregister { id: v.id });
                    }
                }
                (TourneyPhase::Running, Some(e)) if e.place.is_none() => {
                    if let Some(t) = v.my_table
                        && ui.put(b, theme::primary_button("Go to my table")).clicked()
                    {
                        self.open_table(&s.key, t);
                    }
                }
                (_, Some(e)) => {
                    if let Some(place) = e.place {
                        let text = if e.prize > 0 { format!("You finished {} · won {}", ordinal(place), chips_text(e.prize)) } else { format!("You finished {}", ordinal(place)) };
                        p.text(r.right_center() - vec2(20.0, 0.0), egui::Align2::RIGHT_CENTER, text, theme::bold_font(15.0), if e.prize > 0 { Color32::from_rgb(0xff, 0xd2, 0x4a) } else { theme::TEXT() });
                    }
                }
                _ => {}
            }
            let host = s.me.as_ref().is_some_and(|m| m.name == v.host) || s.can(perm::MANAGE_SERVER);
            if v.phase == TourneyPhase::Registering && host {
                let c = Rect::from_min_size(pos2(r.right() - 170.0, r.bottom() - 26.0), vec2(150.0, 20.0));
                if ui.put(c, egui::Button::new(RichText::new("Call it off").size(12.0).color(theme::LIVE())).frame(false)).on_hover_text("Everyone's buy-in goes back.").clicked() {
                    self.engine.poker(&s.key, ToServer::TourneyCancel { id: v.id });
                }
            }
        });
    }

    fn prizes(&self, ui: &mut egui::Ui, v: &TourneyView) {
        section(ui, "Prizes");
        let pay = if v.payouts.is_empty() { payouts(v.prize_pool, v.entrants.len()) } else { v.payouts.clone() };
        if v.prize_pool == 0 {
            ui.label(RichText::new("A free tournament: experience and the glory.").color(theme::MUTED()));
            return;
        }
        for (i, amt) in pay.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(ordinal(i + 1)).font(theme::bold_font(14.0)).color(if i == 0 { Color32::from_rgb(0xff, 0xd2, 0x4a) } else { theme::TEXT() }));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(chips_text(*amt)).font(theme::bold_font(14.0)).color(theme::OWNER()));
                });
            });
        }
        let note = match v.bomb {
            Some(b) => match b.multiplier {
                Some(m) => format!("A Flash Pot: the wheel picked {m}× the buy-in, and the winner takes it all."),
                None => "A Flash Pot: the wheel picks the prize (2 to 1,000 times the buy-in) when the table's full, and the winner takes it all.".to_string(),
            },
            None => "The prize pool is the buy-ins; it grows as people register.".to_string(),
        };
        ui.label(RichText::new(note).size(12.0).color(theme::DIM()));
    }

    fn blinds_sheet(&self, ui: &mut egui::Ui, v: &TourneyView) {
        section(ui, &format!("Blinds · {} min levels", v.rules.level_ms / 60_000));
        let from = v.level.saturating_sub(1);
        for (i, b) in v.rules.levels.iter().enumerate().skip(from).take(8) {
            let now = i == v.level && v.phase == TourneyPhase::Running;
            ui.horizontal(|ui| {
                let col = if now { theme::OWNER() } else if i < v.level { theme::DIM() } else { theme::BODY() };
                ui.label(RichText::new(format!("{:>2}", i + 1)).monospace().color(theme::DIM()));
                ui.label(RichText::new(format!("{} / {}", chips_text(b.small), chips_text(b.big))).font(theme::bold_font(13.5)).color(col));
                if b.ante > 0 {
                    ui.label(RichText::new(format!("ante {}", chips_text(b.ante))).size(12.5).color(theme::DIM()));
                }
                if now {
                    ui.label(RichText::new("◀ now").size(12.0).color(theme::OWNER()));
                }
            });
        }
    }

    fn standings(&mut self, ui: &mut egui::Ui, s: &ServerView, v: &TourneyView, me: &str) {
        section(ui, if v.phase == TourneyPhase::Registering { "Registered" } else { "Standings" });
        let pay = &v.payouts;
        for (i, e) in v.entrants.iter().enumerate().take(200) {
            let w = ui.available_width();
            let (r, resp) = ui.allocate_exact_size(vec2(w, 32.0), Sense::click());
            let p = ui.painter();
            let mine = e.uid == me;
            p.rect_filled(r, theme::r(6), if mine { theme::row(theme::ELEV2()) } else if i % 2 == 0 { theme::row(theme::ELEV()) } else { Color32::TRANSPARENT });
            let rank = e.place.unwrap_or(i + 1);
            let in_money = rank <= pay.len() && v.phase != TourneyPhase::Registering;
            p.text(r.left_center() + vec2(12.0, 0.0), egui::Align2::LEFT_CENTER, if v.phase == TourneyPhase::Registering { (i + 1).to_string() } else { ordinal(rank) }, theme::bold_font(12.5), if in_money { Color32::from_rgb(0xff, 0xd2, 0x5e) } else { theme::MUTED() });
            let name = if e.bot.is_some() { format!("{} (bot)", e.name) } else { e.name.clone() };
            p.text(r.left_center() + vec2(60.0, 0.0), egui::Align2::LEFT_CENTER, name, theme::bold_font(13.5), if e.place.is_some() && e.place != Some(1) { theme::DIM() } else { theme::TEXT() });
            let right = match (e.place, e.prize) {
                (Some(_), p) if p > 0 => format!("won {}", chips_text(p)),
                (Some(_), _) => "out".into(),
                (None, _) if v.phase == TourneyPhase::Registering => String::new(),
                (None, _) => chips_text(e.chips),
            };
            p.text(r.right_center() - vec2(12.0, 0.0), egui::Align2::RIGHT_CENTER, right, theme::bold_font(13.0), theme::OWNER());
            if let Some(t) = e.table
                && v.phase == TourneyPhase::Running
            {
                let n = v.tables.iter().position(|x| *x == t).map(|k| k + 1).unwrap_or(0);
                p.text(pos2(r.left() + w * 0.6, r.center().y), egui::Align2::LEFT_CENTER, format!("Table {n}"), theme::font(12.5), theme::DIM());
                if resp.on_hover_text("Watch this table").clicked() {
                    self.open_table(&s.key, t);
                }
            }
        }
    }

    pub fn new_tourney_dialog(&mut self, ctx: &egui::Context, s: &ServerView) {
        let Some(mut n) = self.poker.new_tourney.clone() else { return };
        let mut open = true;
        let mut create = false;
        egui::Window::new("New tournament").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).open(&mut open).show(ctx, |ui| {
            ui.set_width(460.0);
            ui.label(RichText::new("Name").color(theme::MUTED()));
            ui.add(egui::TextEdit::singleline(&mut n.name).char_limit(40).desired_width(f32::INFINITY));
            ui.add_space(8.0);
            egui::Grid::new("new-tourney").num_columns(2).spacing(vec2(14.0, 8.0)).show(ui, |ui| {
                ui.label(RichText::new("Game").color(theme::MUTED()));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut n.variant, Variant::Holdem, "Texas Hold'em");
                    ui.selectable_value(&mut n.variant, Variant::Omaha, "Omaha");
                });
                ui.end_row();
                ui.label(RichText::new("Betting").color(theme::MUTED()));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut n.limit, Limit::NoLimit, "No Limit");
                    ui.selectable_value(&mut n.limit, Limit::PotLimit, "Pot Limit");
                    ui.selectable_value(&mut n.limit, Limit::FixedLimit, "Limit");
                });
                ui.end_row();
                ui.label(RichText::new("Buy-in").color(theme::MUTED()));
                ui.horizontal(|ui| {
                    for b in [0u64, 100, 500, 1000, 5000, 25_000] {
                        ui.selectable_value(&mut n.buy_in, b, if b == 0 { "Free".to_string() } else { chips_short(b) });
                    }
                });
                ui.end_row();
                ui.label(RichText::new("Starting chips").color(theme::MUTED()));
                ui.horizontal(|ui| {
                    for k in [1500u64, 3000, 5000, 10_000] {
                        ui.selectable_value(&mut n.stack, k, chips_short(k));
                    }
                });
                ui.end_row();
                ui.label(RichText::new("Starts in").color(theme::MUTED()));
                ui.horizontal(|ui| {
                    for (m, label) in [(1u64, "1 min"), (5, "5 min"), (15, "15 min"), (30, "30 min"), (60, "1 h"), (120, "2 h"), (24 * 60, "1 day")] {
                        ui.selectable_value(&mut n.start_in, m, label);
                    }
                });
                ui.end_row();
                ui.label(RichText::new("Levels").color(theme::MUTED()));
                ui.horizontal(|ui| {
                    for m in [3u64, 5, 6, 8, 10, 15] {
                        ui.selectable_value(&mut n.level_min, m, format!("{m} min"));
                    }
                });
                ui.end_row();
                ui.label(RichText::new("At a table").color(theme::MUTED()));
                ui.horizontal(|ui| {
                    for k in [6usize, 9] {
                        ui.selectable_value(&mut n.seats, k, format!("{k} players"));
                    }
                });
                ui.end_row();
                ui.label(RichText::new("Most players").color(theme::MUTED()));
                ui.add(egui::Slider::new(&mut n.max_players, 6..=pyr_poker::tourney::MAX_ENTRANTS));
                ui.end_row();
                ui.label(RichText::new("Late entry").color(theme::MUTED()));
                ui.add(egui::Slider::new(&mut n.late_levels, 0..=6).suffix(" levels"));
                ui.end_row();
                ui.label(RichText::new("Bots").color(theme::MUTED()));
                ui.horizontal(|ui| {
                    ui.add(egui::Slider::new(&mut n.bots, 0..=n.max_players.min(60)));
                    ui.selectable_value(&mut n.bot_level, Level::Rookie, "Rookie");
                    ui.selectable_value(&mut n.bot_level, Level::Regular, "Regular");
                    ui.selectable_value(&mut n.bot_level, Level::Shark, "Shark");
                    if s.has(pyr_proto::control::CASINO_012_FEATURE) {
                        ui.selectable_value(&mut n.bot_level, Level::Genius, "Genius");
                    }
                });
                ui.end_row();
            });
            ui.add_space(6.0);
            let pool = n.buy_in * 10;
            let first = payouts(pool, 10).first().copied().unwrap_or(0);
            ui.label(RichText::new(format!("Buy-ins make the prize pool (bots enter free and add nothing): with 10 players it's {}, {} to the winner. It's called off, buy-ins back, if fewer than 2 are in at the start.", chips_text(pool), chips_text(first))).size(12.5).color(theme::DIM()));
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(theme::primary_button("Set it up")).clicked() && !n.name.trim().is_empty() {
                    create = true;
                }
            });
        });
        if create {
            let rules = TourneyRules {
                name: n.name.trim().to_string(),
                variant: n.variant,
                limit: n.limit,
                seats: n.seats,
                stack: n.stack,
                buy_in: n.buy_in,
                min_players: 2,
                max_players: n.max_players,
                start_ms: wall() + n.start_in * 60_000,
                level_ms: n.level_min * 60_000,
                levels: standard_levels(n.stack),
                late_levels: n.late_levels,
                added: 0,
                bomb: None,
            };
            let bots = vec![n.bot_level; n.bots];
            self.engine.poker(&s.key, ToServer::TourneyCreate { rules, bots });
            self.poker.new_tourney = None;
            self.poker.tourneys_asked = None;
            return;
        }
        self.poker.new_tourney = if open { Some(n) } else { None };
    }

    /// Follows me round a tournament: when it seats me (it started, or I was moved to
    /// another table), that table opens.
    pub fn follow_tourneys(&mut self, s: &ServerView) {
        // I'm out of one I was playing, or I won it: where I finished, big on the table (it
        // stays up a moment for its last hand).
        let me = s.me.as_ref().map(|m| m.user_id.clone()).unwrap_or_default();
        for (id, v) in &s.poker.tourney {
            let Some(e) = v.entrants.iter().find(|e| e.uid == me) else { continue };
            let was = self.poker.my_place.insert(*id, e.place);
            if was == Some(None)
                && let Some(place) = e.place
            {
                let text = if place == 1 && e.prize > 0 {
                    format!("You won {}!", chips_text(e.prize))
                } else if place == 1 {
                    "You won!".to_string()
                } else {
                    format!("You finished {}", ordinal(place))
                };
                self.poker.banner = Some((text, Instant::now()));
            }
        }
        for (id, v) in &s.poker.tourney {
            let seen = self.poker.my_tables.get(id).copied().flatten();
            if v.my_table != seen {
                self.poker.my_tables.insert(*id, v.my_table);
                if let Some(t) = v.my_table {
                    // Opened if I was at the old one, in this tournament's lobby, or anywhere in
                    // the room (not if I've wandered off to a cash table).
                    let at_cash = self.poker.table.is_some_and(|x| s.poker.table(x).is_some_and(|l| l.view.tourney.is_none()));
                    if !at_cash {
                        self.poker.table = Some(t);
                        self.poker.tourney = None;
                        self.poker.banner = (seen.is_some()).then(|| ("Moved to a new table".to_string(), Instant::now()));
                    }
                }
            }
        }
    }
}

pub fn game_name(r: &TourneyRules) -> String {
    let limit = match r.limit {
        Limit::NoLimit => "No Limit",
        Limit::PotLimit => "Pot Limit",
        Limit::FixedLimit => "Limit",
    };
    let v = match r.variant {
        Variant::Holdem => "Hold'em",
        Variant::Omaha => "Omaha",
    };
    format!("{limit} {v}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn places_read_well() {
        let got: Vec<String> = [1, 2, 3, 4, 11, 12, 13, 21, 22, 101, 111].iter().map(|n| ordinal(*n)).collect();
        assert_eq!(got, ["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "101st", "111th"]);
    }
}
