//! Poker in Pyr: a server's poker room (its tables, a leaderboard, my record and looks), and
//! the table itself (see `table`). The server deals and keeps the chips; this draws it all and
//! sends what the player does.

pub mod cards;
pub mod felt;
pub mod table;

use std::time::Instant;

use eframe::egui::{self, Align, Color32, Layout, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::state::{AppState, ServerView};
use pyr_poker::bots::Level;
use pyr_poker::game::{Limit, Rules, Variant};
use pyr_poker::net::{BuyIn, TableId, TableSummary, ToServer};
use pyr_poker::progress::{self, ACHIEVEMENTS, COSMETICS, Kind};

use crate::app::{Act, PyrApp};
use crate::theme;
use cards::{chips_short, chips_text};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Tables,
    Leaderboard,
    Me,
}

/// What a new table will be.
#[derive(Clone)]
pub struct NewTable {
    pub name: String,
    pub variant: Variant,
    pub limit: Limit,
    /// Index into [`STAKES`].
    pub stakes: usize,
    pub seats: usize,
    /// Bots: how many, and how good.
    pub bots: usize,
    pub level: Level,
}

/// Blinds to pick from.
pub const STAKES: [(u64, u64); 6] = [(5, 10), (10, 20), (25, 50), (50, 100), (100, 200), (250, 500)];

impl Default for NewTable {
    fn default() -> NewTable {
        NewTable { name: String::new(), variant: Variant::Holdem, limit: Limit::NoLimit, stakes: 1, seats: 6, bots: 3, level: Level::Regular }
    }
}

/// Moves to make when it's my turn (ticked while waiting).
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum PreAction {
    #[default]
    None,
    CheckFold,
    Check,
    CallAny,
}

#[derive(Default)]
pub struct PokerUi {
    /// The poker room open, on this server (and the table, when one's open).
    pub open: Option<String>,
    pub table: Option<TableId>,
    pub tab: Tab,
    pub new_table: Option<NewTable>,
    /// Sitting down: (table, seat, chips chosen).
    pub buy_in: Option<(TableId, usize, u64)>,
    /// Adding chips: (table, chips).
    pub top_up: Option<(TableId, u64)>,
    pub pre: PreAction,
    /// The hand the pre-action was ticked in (it's forgotten at the next).
    pub pre_hand: Option<u64>,
    /// The raise amount in the box, and the hand/turn it's for.
    pub raise: u64,
    pub raise_for: Option<(u64, usize, u64)>,
    pub raise_text: String,
    pub four_color: bool,
    pub show_odds: bool,
    pub log_open: bool,
    /// The last time the lobby was asked for.
    pub lobby_asked: Option<Instant>,
    /// The chance of winning shown for my hand: (what it was worked out for, the chance).
    pub odds: Option<(String, f64)>,
    /// A celebration showing: (text, since).
    pub banner: Option<(String, Instant)>,
    /// A table was just asked for: open it when it comes.
    pub awaiting_new: bool,
    /// The cloth's picture, for the felt it's made for.
    pub cloth: Option<(String, egui::TextureHandle)>,
    /// How many of a table's events have been heard: (table, hand, count).
    pub heard: Option<(TableId, Option<u64>, usize)>,
    /// The hand and turn the hurry-up chime played for.
    pub hurried: Option<(u64, usize)>,
    pub mute: bool,
    /// The engine's celebrations already shown, and whether its error has been.
    pub gains_seen: usize,
    pub error_shown: bool,
}

impl PyrApp {
    /// Opens a server's poker room.
    pub fn open_poker(&mut self, key: &str) {
        self.poker.open = Some(key.to_string());
        self.poker.table = None;
        self.poker.lobby_asked = None;
        self.engine.poker(key, ToServer::Lobby);
    }

    pub fn close_poker(&mut self) {
        if let (Some(key), Some(t)) = (self.poker.open.clone(), self.poker.table) {
            self.engine.poker(&key, ToServer::Unwatch { table: t });
        }
        self.poker.open = None;
        self.poker.table = None;
    }

    pub fn open_table(&mut self, key: &str, id: TableId) {
        self.poker.table = Some(id);
        self.poker.pre = PreAction::None;
        self.engine.poker(key, ToServer::Watch { table: id });
    }

    /// The poker room page (the main area).
    pub fn poker_page(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, acts: &mut Vec<Act>) {
        // Celebrations: experience, achievements, level-ups (taken from the engine's list).
        // (Forgetting them happens a moment later, so each is taken once by counting.)
        if s.poker.gains.len() < self.poker.gains_seen {
            self.poker.gains_seen = 0;
        }
        if s.poker.gains.len() > self.poker.gains_seen {
            self.engine.take_poker_gains(&s.key);
            let from = self.poker.gains_seen;
            self.poker.gains_seen = s.poker.gains.len();
            for g in &s.poker.gains[from..] {
                for a in &g.achievements {
                    if let Some(a) = progress::achievement(a) {
                        acts.push(Act::Toast(format!("Achievement: {} (+{} XP). {}", a.name, a.xp, a.about), false));
                    }
                }
                if let Some(l) = g.level_up {
                    self.poker.banner = Some((format!("Level {l}!"), Instant::now()));
                    acts.push(Act::Toast(format!("You reached level {l}. New looks may be waiting in your profile."), false));
                }
            }
        }
        match &s.poker.error {
            Some(e) if !self.poker.error_shown => {
                acts.push(Act::Toast(e.clone(), true));
                self.poker.error_shown = true;
                self.engine.clear_poker_error(&s.key);
            }
            Some(_) => {}
            None => self.poker.error_shown = false,
        }
        if self.poker.awaiting_new
            && let Some(mine) = s.me.as_ref()
            && let Some((id, _)) = s.poker.tables.iter().rev().find(|(_, t)| t.view.owner == mine.user_id)
        {
            self.poker.awaiting_new = false;
            self.poker.table = Some(*id);
        }
        if let Some(id) = self.poker.table {
            match s.poker.table(id) {
                Some(t) => {
                    let t = t.clone();
                    self.poker_table(ui, st, s, &t);
                }
                None => {
                    // It closed (or the connection dropped): back to the room.
                    if s.poker.lobby.is_some() {
                        self.poker.table = None;
                    }
                    ui.centered_and_justified(|ui| ui.label(RichText::new("Opening the table…").color(theme::MUTED())));
                }
            }
            return;
        }
        // The lobby refreshes every few seconds while it's open.
        if self.poker.lobby_asked.is_none_or(|t| t.elapsed().as_secs() >= 5) {
            self.poker.lobby_asked = Some(Instant::now());
            self.engine.poker(&s.key, ToServer::Lobby);
            if self.poker.tab == Tab::Leaderboard {
                self.engine.poker(&s.key, ToServer::Leaderboard);
            }
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
        self.lobby(ui, s);
        self.new_table_dialog(ui.ctx(), s);
    }

    fn lobby(&mut self, ui: &mut egui::Ui, s: &ServerView) {
        let full = ui.max_rect();
        // A warm glow behind the header.
        let p = ui.painter().clone();
        let head = Rect::from_min_size(full.min, vec2(full.width(), 190.0));
        for i in 0..14 {
            let t = i as f32 / 14.0;
            let r = Rect::from_min_max(head.min, pos2(head.max.x, head.min.y + head.height() * (1.0 - t)));
            p.rect_filled(r, 0, theme::EMBER().gamma_multiply(0.012));
        }
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(22.0);
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                let (r, _) = ui.allocate_exact_size(vec2(46.0, 46.0), Sense::hover());
                let c = r.center();
                ui.painter().circle_filled(c, 23.0, theme::ELEV());
                cards::suit(ui.painter(), Rect::from_center_size(c, vec2(26.0, 26.0)), 3, theme::TEXT());
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("Poker Room").font(theme::bold_font(26.0)).color(theme::TEXT()));
                    ui.label(RichText::new(format!("{}: Hold'em and Omaha with play chips, against friends and bots. Every shuffle is checked by your own Pyr.", s.name)).color(theme::MUTED()));
                });
            });
            ui.add_space(16.0);
            if let Some(me) = &s.poker.me {
                self.me_card(ui, s, me.clone());
            }
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                for (tab, name) in [(Tab::Tables, "Tables"), (Tab::Leaderboard, "Leaderboard"), (Tab::Me, "Profile & Looks")] {
                    let on = self.poker.tab == tab;
                    let b = egui::Button::new(RichText::new(name).font(theme::bold_font(14.0)).color(if on { theme::on_ember() } else { theme::TEXT() }))
                        .fill(if on { theme::EMBER() } else { theme::ELEV() })
                        .corner_radius(theme::r(8))
                        .min_size(vec2(0.0, 34.0));
                    if ui.add(b).clicked() {
                        self.poker.tab = tab;
                        if tab == Tab::Leaderboard {
                            self.engine.poker(&s.key, ToServer::Leaderboard);
                        }
                    }
                }
                if self.poker.tab == Tab::Tables {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(28.0);
                        if ui.add(theme::primary_button("New table").min_size(vec2(130.0, 34.0))).clicked() {
                            let mut n = NewTable::default();
                            n.name = format!("{}'s table", s.me.as_ref().map(|m| m.name.as_str()).unwrap_or("My"));
                            self.poker.new_table = Some(n);
                        }
                    });
                }
            });
            ui.add_space(12.0);
            match self.poker.tab {
                Tab::Tables => self.tables_list(ui, s),
                Tab::Leaderboard => self.leaderboard(ui, s),
                Tab::Me => self.profile(ui, s),
            }
            ui.add_space(30.0);
        });
    }

    fn me_card(&mut self, ui: &mut egui::Ui, s: &ServerView, me: pyr_poker::net::RecordView) {
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            let w = (ui.available_width() - 28.0).max(300.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 92.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(r, theme::r(14), theme::ELEV());
            p.rect_stroke(r, theme::r(14), Stroke::new(1.0f32, theme::LINE()), StrokeKind::Inside);
            // Level badge.
            let lc = r.left_center() + vec2(48.0, 0.0);
            p.circle_filled(lc, 30.0, theme::ELEV2());
            p.circle_stroke(lc, 30.0, Stroke::new(3.0f32, theme::EMBER()));
            // Experience around the badge.
            let span = (me.level_to - me.level_from).max(1) as f32;
            let frac = ((me.xp - me.level_from) as f32 / span).clamp(0.0, 1.0);
            arc(p, lc, 30.0, frac, Stroke::new(4.0f32, theme::GLOW()));
            p.text(lc + vec2(0.0, -4.0), egui::Align2::CENTER_CENTER, me.level.to_string(), theme::bold_font(22.0), theme::TEXT());
            p.text(lc + vec2(0.0, 15.0), egui::Align2::CENTER_CENTER, "LEVEL", theme::bold_font(8.5), theme::MUTED());
            let x = r.left() + 96.0;
            p.text(pos2(x, r.top() + 22.0), egui::Align2::LEFT_CENTER, &me.name, theme::bold_font(17.0), theme::TEXT());
            p.text(pos2(x, r.top() + 46.0), egui::Align2::LEFT_CENTER, format!("{} XP to level {}", chips_text(me.level_to.saturating_sub(me.xp)), me.level + 1), theme::font(12.5), theme::MUTED());
            let stats = format!(
                "{} hands · won {} · showdowns won {}%",
                chips_text(me.hands),
                chips_text(me.hands_won),
                if me.showdowns > 0 { me.showdowns_won * 100 / me.showdowns } else { 0 }
            );
            p.text(pos2(x, r.top() + 68.0), egui::Align2::LEFT_CENTER, stats, theme::font(12.5), theme::DIM());
            // Chips, and the daily chips.
            let cx = r.right() - 330.0;
            cards::pile(p, pos2(cx, r.center().y + 22.0), me.chips.max(1), 10.0, &me.chip_set);
            p.text(pos2(cx + 58.0, r.top() + 30.0), egui::Align2::LEFT_CENTER, chips_text(me.chips), theme::bold_font(22.0), theme::OWNER());
            let at = if me.at_tables > 0 { format!("chips · {} at tables", chips_text(me.at_tables)) } else { "chips".to_string() };
            p.text(pos2(cx + 58.0, r.top() + 56.0), egui::Align2::LEFT_CENTER, at, theme::font(12.5), theme::MUTED());
            let b = Rect::from_min_size(pos2(r.right() - 150.0, r.center().y - 20.0), vec2(132.0, 40.0));
            let label = if me.daily_ready { "Daily chips".to_string() } else { format!("Back tomorrow · {}🔥", me.daily_streak) };
            let resp = ui.put(b, egui::Button::new(RichText::new(label).font(theme::bold_font(13.5)).color(if me.daily_ready { theme::on_ember() } else { theme::MUTED() })).fill(if me.daily_ready { theme::EMBER() } else { theme::ELEV2() }).corner_radius(theme::r(10)));
            if resp
                .on_hover_text("Free chips every day, more for each day in a row (up to a week). Chips are only for play: they can't be bought or cashed out.")
                .clicked()
                && me.daily_ready
            {
                self.engine.poker(&s.key, ToServer::Daily);
            }
        });
    }

    fn tables_list(&mut self, ui: &mut egui::Ui, s: &ServerView) {
        let Some(tables) = s.poker.lobby.clone() else {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                ui.label(RichText::new("Finding the tables…").color(theme::MUTED()));
            });
            return;
        };
        if tables.is_empty() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No tables yet").font(theme::bold_font(18.0)).color(theme::TEXT()));
                ui.label(RichText::new("Start one with New table: bots keep the seats warm until your friends sit down.").color(theme::MUTED()));
            });
            return;
        }
        for t in tables {
            self.table_row(ui, s, &t);
            ui.add_space(8.0);
        }
    }

    fn table_row(&mut self, ui: &mut egui::Ui, s: &ServerView, t: &TableSummary) {
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            let w = (ui.available_width() - 28.0).max(300.0);
            let (r, resp) = ui.allocate_exact_size(vec2(w, 72.0), Sense::click());
            let p = ui.painter();
            let hot = resp.hovered();
            p.rect_filled(r, theme::r(12), theme::row(if hot { theme::ELEV2() } else { theme::ELEV() }));
            // A small felt with the seats taken.
            let felt = Rect::from_center_size(r.left_center() + vec2(46.0, 0.0), vec2(64.0, 40.0));
            p.rect_filled(felt, theme::r(20), Color32::from_rgb(0x17, 0x6b, 0x45));
            p.rect_stroke(felt, theme::r(20), Stroke::new(3.0f32, Color32::from_rgb(0x3a, 0x26, 0x1a)), StrokeKind::Outside);
            let taken = t.players + t.bots;
            for i in 0..t.seats {
                let a = i as f32 / t.seats as f32 * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2;
                let c = felt.center() + vec2(a.cos() * 38.0, a.sin() * 25.0);
                p.circle_filled(c, 4.0, if i < taken { theme::OWNER() } else { theme::DIM().gamma_multiply(0.6) });
            }
            let x = r.left() + 94.0;
            p.text(pos2(x, r.top() + 22.0), egui::Align2::LEFT_CENTER, &t.name, theme::bold_font(16.0), theme::TEXT());
            p.text(pos2(x, r.top() + 46.0), egui::Align2::LEFT_CENTER, t.rules.name(), theme::font(13.0), theme::MUTED());
            let who = if t.names.is_empty() { "Nobody sitting yet".to_string() } else { t.names.join(", ") };
            let mid = r.left() + w * 0.45;
            p.text(pos2(mid, r.top() + 22.0), egui::Align2::LEFT_CENTER, format!("{} / {} seats · {} people, {} bots", taken, t.seats, t.players, t.bots), theme::font(13.0), theme::BODY());
            let who: String = who.chars().take(60).collect();
            p.text(pos2(mid, r.top() + 46.0), egui::Align2::LEFT_CENTER, who, theme::font(12.5), theme::DIM());
            let right = r.right() - 140.0;
            p.text(pos2(right - 120.0, r.top() + 22.0), egui::Align2::LEFT_CENTER, format!("Avg pot {}", chips_short(t.average_pot)), theme::font(12.5), theme::MUTED());
            p.text(pos2(right - 120.0, r.top() + 46.0), egui::Align2::LEFT_CENTER, format!("{} hands/hr", t.hands_per_hour), theme::font(12.5), theme::DIM());
            let b = Rect::from_min_size(pos2(r.right() - 120.0, r.center().y - 18.0), vec2(104.0, 36.0));
            let open = ui.put(b, theme::primary_button(if taken < t.seats { "Join" } else { "Watch" }));
            if open.clicked() || resp.double_clicked() {
                self.open_table(&s.key, t.id);
            }
        });
    }

    fn leaderboard(&mut self, ui: &mut egui::Ui, s: &ServerView) {
        let Some(rows) = s.poker.leaderboard.clone() else {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                ui.label(RichText::new("Loading…").color(theme::MUTED()));
            });
            return;
        };
        for (i, row) in rows.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                let w = (ui.available_width() - 28.0).max(300.0);
                let (r, _) = ui.allocate_exact_size(vec2(w, 44.0), Sense::hover());
                let p = ui.painter();
                let mine = s.me.as_ref().is_some_and(|m| m.user_id == row.uid);
                p.rect_filled(r, theme::r(8), if mine { theme::row(theme::ELEV2()) } else if i % 2 == 0 { theme::row(theme::ELEV()) } else { Color32::TRANSPARENT });
                let medal = match i {
                    0 => Some(Color32::from_rgb(0xff, 0xd2, 0x4a)),
                    1 => Some(Color32::from_rgb(0xd6, 0xdc, 0xe4)),
                    2 => Some(Color32::from_rgb(0xd0, 0x8a, 0x4c)),
                    _ => None,
                };
                let c = r.left_center() + vec2(22.0, 0.0);
                match medal {
                    Some(m) => {
                        p.circle_filled(c, 12.0, m);
                        p.text(c, egui::Align2::CENTER_CENTER, (i + 1).to_string(), theme::bold_font(12.0), Color32::from_rgb(0x22, 0x1a, 0x10));
                    }
                    None => {
                        p.text(c, egui::Align2::CENTER_CENTER, (i + 1).to_string(), theme::bold_font(13.0), theme::MUTED());
                    }
                }
                p.text(r.left_center() + vec2(48.0, 0.0), egui::Align2::LEFT_CENTER, &row.name, theme::bold_font(14.5), theme::TEXT());
                p.text(pos2(r.left() + w * 0.36, r.center().y), egui::Align2::LEFT_CENTER, format!("Level {}", row.level), theme::font(13.0), theme::MUTED());
                p.text(pos2(r.left() + w * 0.5, r.center().y), egui::Align2::LEFT_CENTER, format!("{} hands", chips_text(row.hands)), theme::font(13.0), theme::DIM());
                p.text(pos2(r.left() + w * 0.66, r.center().y), egui::Align2::LEFT_CENTER, if row.best_hand.is_empty() { "—".into() } else { row.best_hand.clone() }, theme::font(13.0), theme::DIM());
                p.text(r.right_center() - vec2(16.0, 0.0), egui::Align2::RIGHT_CENTER, chips_text(row.chips), theme::bold_font(15.0), theme::OWNER());
            });
        }
    }

    fn profile(&mut self, ui: &mut egui::Ui, s: &ServerView) {
        let Some(me) = s.poker.me.clone() else { return };
        let pct = |a: u64, b: u64| if b > 0 { format!("{}%", a * 100 / b) } else { "—".into() };
        let stats = [
            ("Hands played", chips_text(me.hands)),
            ("Hands won", format!("{} ({})", chips_text(me.hands_won), pct(me.hands_won, me.hands))),
            ("Showdowns won", format!("{} of {}", me.showdowns_won, me.showdowns)),
            ("Plays before the flop (VPIP)", pct(me.vpip, me.hands)),
            ("Raises before the flop (PFR)", pct(me.pfr, me.hands)),
            ("Biggest pot", chips_text(me.biggest_pot)),
            ("Best hand", if me.best_hand.is_empty() { "—".into() } else { me.best_hand.clone() }),
        ];
        section(ui, "Your numbers");
        egui::Grid::new("poker-stats").num_columns(2).spacing(vec2(40.0, 8.0)).show(ui, |ui| {
            for (k, v) in stats {
                ui.horizontal(|ui| {
                    ui.add_space(28.0);
                    ui.label(RichText::new(k).color(theme::MUTED()));
                });
                ui.label(RichText::new(v).font(theme::bold_font(14.0)).color(theme::TEXT()));
                ui.end_row();
            }
        });
        section(ui, &format!("Achievements · {} of {}", me.achievements.len(), ACHIEVEMENTS.len()));
        let cols = ((ui.available_width() - 56.0) / 250.0).floor().max(1.0) as usize;
        for row in ACHIEVEMENTS.chunks(cols) {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                for a in row {
                    let got = me.achievements.iter().any(|x| x == a.id);
                    let (r, resp) = ui.allocate_exact_size(vec2(240.0, 58.0), Sense::hover());
                    let p = ui.painter();
                    p.rect_filled(r, theme::r(10), if got { theme::ELEV2() } else { theme::ELEV() });
                    if got {
                        p.rect_stroke(r, theme::r(10), Stroke::new(1.5f32, theme::OWNER()), StrokeKind::Inside);
                    }
                    let ic = Rect::from_center_size(r.left_center() + vec2(24.0, 0.0), vec2(22.0, 22.0));
                    crate::icons::star(p, ic, if got { theme::OWNER() } else { theme::DIM() }, got);
                    p.text(r.left_top() + vec2(46.0, 14.0), egui::Align2::LEFT_TOP, a.name, theme::bold_font(13.5), if got { theme::TEXT() } else { theme::MUTED() });
                    p.text(r.left_top() + vec2(46.0, 34.0), egui::Align2::LEFT_TOP, format!("+{} XP", a.xp), theme::font(11.5), theme::DIM());
                    resp.on_hover_text(a.about);
                    ui.add_space(8.0);
                }
            });
            ui.add_space(8.0);
        }
        section(ui, "Looks · unlocked by playing, never bought");
        for (kind, title) in [(Kind::Felt, "Table felt"), (Kind::CardBack, "Card back"), (Kind::Chips, "Chips"), (Kind::Frame, "Picture frame")] {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                ui.label(RichText::new(title).font(theme::bold_font(13.0)).color(theme::MUTED()));
            });
            ui.horizontal_wrapped(|ui| {
                ui.add_space(28.0);
                for c in COSMETICS.iter().filter(|c| c.kind == kind) {
                    let have = me.unlocked.iter().any(|u| u == c.id);
                    let on = [&me.felt, &me.card_back, &me.chip_set, &me.frame].iter().any(|x| x.as_str() == c.id);
                    let (r, resp) = ui.allocate_exact_size(vec2(118.0, 96.0), Sense::click());
                    let p = ui.painter();
                    p.rect_filled(r, theme::r(10), theme::ELEV());
                    if on {
                        p.rect_stroke(r, theme::r(10), Stroke::new(2.0f32, theme::EMBER()), StrokeKind::Inside);
                    }
                    let sw = Rect::from_center_size(r.center() + vec2(0.0, -12.0), vec2(56.0, 48.0));
                    match kind {
                        Kind::Felt => {
                            let (a, b) = table::felt_colors(c.id);
                            p.rect_filled(sw, theme::r(22), b);
                            p.rect_filled(sw.shrink(8.0), theme::r(16), a);
                        }
                        Kind::CardBack => cards::back(p, Rect::from_center_size(sw.center(), vec2(34.0, 48.0)), c.id),
                        Kind::Chips => cards::pile(p, sw.center() + vec2(0.0, 18.0), 1630, 9.0, c.id),
                        Kind::Frame => {
                            p.circle_filled(sw.center(), 20.0, theme::ELEV2());
                            p.circle_stroke(sw.center(), 22.0, Stroke::new(4.0f32, table::frame_color(c.id).unwrap_or(theme::LINE())));
                        }
                    }
                    p.text(pos2(r.center().x, r.bottom() - 18.0), egui::Align2::CENTER_CENTER, c.name, theme::font(12.0), if have { theme::TEXT() } else { theme::DIM() });
                    if !have {
                        p.rect_filled(r, theme::r(10), Color32::from_black_alpha(110));
                        crate::icons::lock(p, Rect::from_center_size(sw.center(), vec2(18.0, 18.0)), theme::MUTED());
                    }
                    let tip = if have { format!("{}: click to use it", c.name) } else { format!("{}: {}", c.name, c.unlock.describe()) };
                    if resp.on_hover_text(tip).clicked() && have {
                        self.engine.poker(&s.key, ToServer::Equip { id: c.id.into() });
                    }
                }
            });
            ui.add_space(10.0);
        }
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            ui.checkbox(&mut self.poker.four_color, "Four-color deck (clubs green, diamonds blue)");
            ui.add_space(20.0);
            ui.checkbox(&mut self.poker.show_odds, "Show my chance of winning at the table");
        });
    }

    fn new_table_dialog(&mut self, ctx: &egui::Context, s: &ServerView) {
        let Some(mut n) = self.poker.new_table.clone() else { return };
        let mut open = true;
        let mut create = false;
        egui::Window::new("New table").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).open(&mut open).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(RichText::new("Name").color(theme::MUTED()));
            ui.add(egui::TextEdit::singleline(&mut n.name).char_limit(40).desired_width(f32::INFINITY));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Game").color(theme::MUTED()));
                ui.selectable_value(&mut n.variant, Variant::Holdem, "Texas Hold'em");
                ui.selectable_value(&mut n.variant, Variant::Omaha, "Omaha");
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Betting").color(theme::MUTED()));
                ui.selectable_value(&mut n.limit, Limit::NoLimit, "No Limit");
                ui.selectable_value(&mut n.limit, Limit::PotLimit, "Pot Limit");
                ui.selectable_value(&mut n.limit, Limit::FixedLimit, "Limit");
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Blinds").color(theme::MUTED()));
                for (i, (sb, bb)) in STAKES.iter().enumerate() {
                    ui.selectable_value(&mut n.stakes, i, format!("{sb}/{bb}"));
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Seats").color(theme::MUTED()));
                for k in [2, 4, 6, 9] {
                    ui.selectable_value(&mut n.seats, k, k.to_string());
                }
            });
            n.bots = n.bots.min(n.seats - 1);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Bots").color(theme::MUTED()));
                ui.add(egui::Slider::new(&mut n.bots, 0..=n.seats - 1));
                ui.selectable_value(&mut n.level, Level::Rookie, "Rookie");
                ui.selectable_value(&mut n.level, Level::Regular, "Regular");
                ui.selectable_value(&mut n.level, Level::Shark, "Shark");
            });
            let (sb, bb) = STAKES[n.stakes];
            let b = BuyIn::default();
            ui.label(RichText::new(format!("Sit down with {} to {} chips ({} to {} big blinds). Bots leave their seats to people who join; you can add or take them away at the table.", chips_text(b.min_bb * bb), chips_text(b.max_bb * bb), b.min_bb, b.max_bb)).size(12.5).color(theme::DIM()));
            let _ = sb;
            ui.add_space(10.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(theme::primary_button("Create table")).clicked() {
                    create = true;
                }
            });
        });
        if create {
            let (sb, bb) = STAKES[n.stakes];
            let rules = Rules { variant: n.variant, limit: n.limit, small_blind: sb, big_blind: bb, ante: 0, seats: n.seats };
            // Bots in the seats after the first (seat 0 is kept for whoever made it).
            let bots = (1..=n.bots).map(|i| (i * n.seats / (n.bots + 1).max(1) % n.seats).max(1)).collect::<std::collections::BTreeSet<_>>();
            let mut seats: Vec<usize> = bots.into_iter().collect();
            let mut k = 1;
            while seats.len() < n.bots && k < n.seats {
                if !seats.contains(&k) {
                    seats.push(k);
                }
                k += 1;
            }
            let bots = seats.into_iter().map(|s| (s, n.level)).collect();
            self.engine.poker(&s.key, ToServer::Create { name: n.name.clone(), rules, bots, buy_in: BuyIn::default() });
            self.poker.new_table = None;
            self.poker.awaiting_new = true;
            return;
        }
        self.poker.new_table = if open { Some(n) } else { None };
    }
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        ui.add_space(28.0);
        ui.label(RichText::new(title).font(theme::bold_font(16.0)).color(theme::TEXT()));
    });
    ui.add_space(6.0);
}

/// An arc around `c`, `frac` of the way round from the top, clockwise.
pub fn arc(p: &egui::Painter, c: egui::Pos2, r: f32, frac: f32, stroke: Stroke) {
    if frac <= 0.0 {
        return;
    }
    let n = (64.0 * frac).ceil().max(2.0) as usize;
    let pts: Vec<egui::Pos2> = (0..=n)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + frac * std::f32::consts::TAU * i as f32 / n as f32;
            c + vec2(a.cos() * r, a.sin() * r)
        })
        .collect();
    p.add(egui::Shape::line(pts, stroke));
}
