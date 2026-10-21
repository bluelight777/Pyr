//! Pyr Casino in the app: the floor (every game, the Phoenix Jackpot, big wins as they happen,
//! Pyr Friday Night) and a table for each game against the house. Poker (tables and
//! tournaments) is in `crate::poker`; this is everything else.
//!
//! The server plays every round; this draws it, animates it, and checks it: the engine plays
//! each round again from its seeds (see `pyr_engine::casino`), and the count of rounds checked
//! is on every table's top bar.

pub mod art;
pub mod baccarat;
pub mod blackjack;
pub mod craps;
pub mod dice;
pub mod floor;
pub mod keno;
pub mod roulette;
pub mod slots;

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::casino::Game;
use pyr_poker::net::RoundView;

use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

/// The casino's own colors (it looks like a casino whatever skin Pyr wears).
pub mod col {
    use eframe::egui::Color32;
    pub const NIGHT: Color32 = Color32::from_rgb(0x0b, 0x0c, 0x14);
    pub const GOLD: Color32 = Color32::from_rgb(0xf5, 0xc4, 0x51);
    pub const GOLD2: Color32 = Color32::from_rgb(0xff, 0xe2, 0x9a);
    pub const GOLD_DARK: Color32 = Color32::from_rgb(0x8a, 0x64, 0x1c);
    pub const EMBER: Color32 = Color32::from_rgb(0xff, 0x7a, 0x1a);
    pub const RED: Color32 = Color32::from_rgb(0xd2, 0x2f, 0x3c);
    pub const BLUE: Color32 = Color32::from_rgb(0x2f, 0x6f, 0xe0);
    pub const GREEN: Color32 = Color32::from_rgb(0x2f, 0xb8, 0x6a);
    pub const FELT: Color32 = Color32::from_rgb(0x0f, 0x5a, 0x3a);
    pub const FELT_DARK: Color32 = Color32::from_rgb(0x05, 0x26, 0x18);
    pub const TEXT: Color32 = Color32::from_rgb(0xf4, 0xee, 0xe2);
    pub const MUTED: Color32 = Color32::from_rgb(0xb8, 0xab, 0x96);
    pub const WIN: Color32 = Color32::from_rgb(0x6a, 0xe8, 0x8a);
    pub const LOSE: Color32 = Color32::from_rgb(0xf0, 0x7a, 0x6a);
}

/// Chip values on the rack.
pub const CHIPS: [u64; 8] = [1, 5, 25, 100, 500, 1_000, 5_000, 25_000];

/// A win shown big over the table.
pub struct Celebration {
    pub title: &'static str,
    pub amount: u64,
    pub multiple_x100: u64,
    pub since: Instant,
    pub tier: u8,
}

/// A round's result called out big over its table (the size a stream's viewers can read):
/// what, a line under it, its color, where (a fraction of the table's area across and down),
/// and from when.
pub struct Announcement {
    pub title: String,
    pub sub: String,
    pub color: Color32,
    pub at: (f32, f32),
    pub since: Instant,
}

/// The casino's screens' state.
pub struct CasinoUi {
    /// The game open (none: the floor, in the lobby).
    pub game: Option<Game>,
    /// The server the casino was opened on (its state asked for).
    pub opened: Option<String>,
    /// The chip picked on the rack.
    pub chip: u64,
    pub fullscreen: bool,
    pub fullscreen_on: bool,
    /// Streamer mode's stage put away (Esc) until the game or table is left.
    pub stage_off: bool,
    /// The stream layout's menu, open under this button.
    pub stream_menu: Option<Rect>,
    /// Streamer mode: my chips when it came on (the session's up or down from there).
    pub session_start: Option<u64>,
    pub mute: bool,
    /// Rounds and big wins already shown, and answers already had.
    pub seen_round: u64,
    pub seen_answers: u64,
    pub seen_big_win: u64,
    /// A request on its way, since when.
    pub pending: Option<Instant>,
    pub celebration: Option<Celebration>,
    pub announcement: Option<Announcement>,
    /// Sounds to play later (animations land on them).
    pub sounds: Vec<(Instant, Cue, f32, f32)>,
    pub fairness_open: bool,
    /// The balance while a round plays out on screen: the chips with the bet taken but not yet
    /// the win, until the round settles there (or the time given, at the latest).
    pub hold: Option<(u64, Instant)>,
    /// The balance as shown (it counts up to a win), what it last counted to, and the last
    /// gain, since when (it floats up from the balance).
    pub shown_chips: Option<f64>,
    pub last_target: u64,
    pub gain: Option<(u64, Instant)>,
    pub slots: slots::SlotsUi,
    pub bj: blackjack::BjUi,
    pub roulette: roulette::RouletteUi,
    pub baccarat: baccarat::BacUi,
    pub craps: craps::CrapsUi,
    pub dice: dice::DiceUi,
    pub keno: keno::KenoUi,
}

impl Default for CasinoUi {
    fn default() -> Self {
        CasinoUi {
            game: None,
            opened: None,
            chip: 25,
            fullscreen: false,
            fullscreen_on: false,
            stage_off: false,
            stream_menu: None,
            session_start: None,
            mute: false,
            seen_round: 0,
            seen_answers: 0,
            seen_big_win: 0,
            pending: None,
            celebration: None,
            announcement: None,
            sounds: Vec::new(),
            fairness_open: false,
            hold: None,
            shown_chips: None,
            last_target: 0,
            gain: None,
            slots: Default::default(),
            bj: Default::default(),
            roulette: Default::default(),
            baccarat: Default::default(),
            craps: Default::default(),
            dice: Default::default(),
            keno: Default::default(),
        }
    }
}

impl CasinoUi {
    /// A sound `after` from now.
    pub fn sound_in(&mut self, after: f32, cue: Cue, pan: f32, vol: f32) {
        self.sounds.push((Instant::now() + Duration::from_secs_f32(after.max(0.0)), cue, pan, vol));
    }

    /// The round's on the table: the win goes on the balance now.
    pub fn settle_balance(&mut self) {
        self.hold = None;
    }

    /// The round settles on screen by itself at `at` (the balance waits till then).
    pub fn settle_at(&mut self, at: Instant) {
        if let Some(h) = &mut self.hold {
            h.1 = at;
        }
    }

    pub fn busy(&self) -> bool {
        self.pending.is_some_and(|t| t.elapsed() < Duration::from_secs(10))
    }

    /// Calls a round's result out big over the table, `delay` from now: `at` is where (a
    /// fraction of the table's area, across and down).
    pub fn announce(&mut self, title: impl Into<String>, sub: impl Into<String>, color: Color32, at: (f32, f32), delay: f32) {
        self.announcement = Some(Announcement { title: title.into(), sub: sub.into(), color, at, since: Instant::now() + Duration::from_secs_f32(delay.max(0.0)) });
    }

    /// Shows a win big, if it's big enough.
    pub fn celebrate(&mut self, paid: u64, wagered: u64, jackpot: bool, delay: f32) {
        let m = pyr_poker::casino::multiple_x100(paid, wagered);
        let (title, tier) = if jackpot {
            ("PHOENIX JACKPOT", 4)
        } else if m >= 5_000 {
            ("EPIC WIN", 3)
        } else if m >= 2_500 {
            ("MEGA WIN", 2)
        } else if m >= 1_000 {
            ("BIG WIN", 1)
        } else {
            return;
        };
        self.celebration = Some(Celebration { title, amount: paid, multiple_x100: m, since: Instant::now() + Duration::from_secs_f32(delay), tier });
        self.sound_in(delay, if tier >= 4 { Cue::Jackpot } else { Cue::WinBig }, 0.0, 0.9);
    }
}

/// My chips now: my record's (kept up to date by every round, buy-in and gift), or the casino's
/// own count until my record has come.
pub fn chips_now(s: &ServerView) -> u64 {
    s.poker.me.as_ref().map(|m| m.chips).unwrap_or(s.poker.casino.chips)
}

/// "x12.50" from hundredths.
pub fn times(x100: u64) -> String {
    if x100 % 100 == 0 { format!("{}×", x100 / 100) } else { format!("{}.{:02}×", x100 / 100, x100 % 100) }
}

/// Slows down to a stop, overshooting a little and settling back.
pub fn ease_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.2;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// A vertical gradient filling `r`.
pub fn vgrad(p: &egui::Painter, r: Rect, top: Color32, bottom: Color32) {
    let mut m = egui::Mesh::default();
    m.colored_vertex(r.left_top(), top);
    m.colored_vertex(r.right_top(), top);
    m.colored_vertex(r.right_bottom(), bottom);
    m.colored_vertex(r.left_bottom(), bottom);
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    p.add(Shape::mesh(m));
}

/// Between two colors.
pub fn mix(a: Color32, b: Color32, k: f32) -> Color32 {
    let k = k.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// A vertical gradient filling `r`, its corners rounded by `radius`.
pub fn vgrad_round(p: &egui::Painter, r: Rect, radius: f32, top: Color32, bottom: Color32) {
    let rad = radius.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    let at = |y: f32| mix(top, bottom, (y - r.top()) / r.height().max(1.0));
    let mut m = egui::Mesh::default();
    let c = r.center();
    m.colored_vertex(c, at(c.y));
    let seg = 10;
    // Clockwise from the bottom right (y grows downwards).
    let corners = [(pos2(r.right() - rad, r.bottom() - rad), 0.0f32), (pos2(r.left() + rad, r.bottom() - rad), 90.0), (pos2(r.left() + rad, r.top() + rad), 180.0), (pos2(r.right() - rad, r.top() + rad), 270.0)];
    for (cc, a0) in corners {
        for i in 0..=seg {
            let a = (a0 + 90.0 * i as f32 / seg as f32).to_radians();
            let q = cc + vec2(a.cos(), a.sin()) * rad;
            m.colored_vertex(q, at(q.y));
        }
    }
    let n = 4 * (seg + 1) as u32;
    for i in 1..=n {
        m.add_triangle(0, i, if i == n { 1 } else { i + 1 });
    }
    p.add(Shape::mesh(m));
}

/// A disc shading from `inner` at the middle to `outer` at the edge.
pub fn radial(p: &egui::Painter, c: Pos2, r: f32, inner: Color32, outer: Color32) {
    let mut m = egui::Mesh::default();
    m.colored_vertex(c, inner);
    let n = 64;
    for i in 0..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        m.colored_vertex(c + vec2(a.cos(), a.sin()) * r, outer);
    }
    for i in 1..=n as u32 {
        m.add_triangle(0, i, i + 1);
    }
    p.add(Shape::mesh(m));
}

/// An oval shading from `inner` to `outer` (a felt).
pub fn oval(p: &egui::Painter, r: Rect, inner: Color32, outer: Color32) {
    let mut m = egui::Mesh::default();
    let c = r.center();
    m.colored_vertex(c, inner);
    let n = 72;
    for i in 0..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        m.colored_vertex(c + vec2(a.cos() * r.width() / 2.0, a.sin() * r.height() / 2.0), outer);
    }
    for i in 1..=n as u32 {
        m.add_triangle(0, i, i + 1);
    }
    p.add(Shape::mesh(m));
}

/// Text with a soft glow behind it.
pub fn glow_text(p: &egui::Painter, at: Pos2, align: Align2, text: &str, font: egui::FontId, color: Color32, glow: Color32) {
    for (dx, dy) in [(-2.0, 0.0), (2.0, 0.0), (0.0, -2.0), (0.0, 2.0), (-1.5, -1.5), (1.5, 1.5), (1.5, -1.5), (-1.5, 1.5)] {
        p.text(at + vec2(dx, dy), align, text, font.clone(), glow);
    }
    p.text(at, align, text, font, color);
}

/// A chip seen from above, with its value on it.
pub fn flat_chip(p: &egui::Painter, c: Pos2, r: f32, value: u64, lift: bool) {
    let (base, edge) = chip_colors(value);
    if lift {
        p.circle_filled(c + vec2(0.0, 3.0), r, Color32::from_black_alpha(110));
    }
    p.circle_filled(c, r, base);
    for k in 0..8 {
        let a = k as f32 / 8.0 * std::f32::consts::TAU;
        let q = c + vec2(a.cos(), a.sin()) * r * 0.82;
        p.add(Shape::convex_polygon(
            {
                let w = 0.16;
                let (a1, a2) = (a - w, a + w);
                vec![c + vec2(a1.cos(), a1.sin()) * r * 0.98, c + vec2(a2.cos(), a2.sin()) * r * 0.98, c + vec2(a2.cos(), a2.sin()) * r * 0.66, c + vec2(a1.cos(), a1.sin()) * r * 0.66]
            },
            edge,
            Stroke::NONE,
        ));
        let _ = q;
    }
    p.circle_filled(c, r * 0.62, base);
    p.circle_stroke(c, r * 0.62, Stroke::new(1.0f32, edge.gamma_multiply(0.8)));
    let label = crate::poker::cards::chips_short(value);
    let text = if base.r() as u32 + base.g() as u32 + base.b() as u32 > 500 { Color32::from_rgb(0x20, 0x1a, 0x14) } else { Color32::WHITE };
    p.text(c, Align2::CENTER_CENTER, label, theme::bold_font((r * 0.62).max(8.0)), text);
}

pub fn chip_colors(value: u64) -> (Color32, Color32) {
    match value {
        0..=4 => (Color32::from_rgb(0xee, 0xee, 0xea), Color32::from_rgb(0x2a, 0x6c, 0xe0)),
        5..=24 => (Color32::from_rgb(0xd0, 0x2b, 0x3a), Color32::from_rgb(0xf5, 0xf2, 0xea)),
        25..=99 => (Color32::from_rgb(0x1e, 0x9a, 0x55), Color32::from_rgb(0xf5, 0xf2, 0xea)),
        100..=499 => (Color32::from_rgb(0x24, 0x24, 0x28), Color32::from_rgb(0xf5, 0xf2, 0xea)),
        500..=999 => (Color32::from_rgb(0x7b, 0x3f, 0xc6), Color32::from_rgb(0xf5, 0xf2, 0xea)),
        1000..=4999 => (Color32::from_rgb(0xf0, 0xc0, 0x2e), Color32::from_rgb(0x8a, 0x24, 0x1c)),
        5000..=24_999 => (Color32::from_rgb(0xf0, 0x7a, 0x1e), Color32::from_rgb(0xf5, 0xf2, 0xea)),
        _ => (Color32::from_rgb(0x2a, 0x6c, 0xe0), Color32::from_rgb(0xff, 0xd7, 0x6a)),
    }
}

/// Chips for an amount, stacked (biggest at the bottom), drawn from above at `c`.
pub fn chip_stack(p: &egui::Painter, c: Pos2, r: f32, amount: u64) {
    if amount == 0 {
        return;
    }
    let mut left = amount;
    let mut chips = Vec::new();
    for d in CHIPS.iter().rev() {
        while left >= *d && chips.len() < 6 {
            chips.push(*d);
            left -= d;
        }
    }
    if chips.is_empty() {
        chips.push(1);
    }
    for (i, v) in chips.iter().enumerate() {
        flat_chip(p, c + vec2(0.0, -(i as f32) * r * 0.22), r, *v, i == 0);
    }
    if chips.len() > 1 || amount != chips[0] {
        let top = c + vec2(0.0, -(chips.len() as f32) * r * 0.22 - r - 9.0);
        let t = chips_text(amount);
        let w = 12.0 + t.len() as f32 * 7.0;
        let tag = Rect::from_center_size(top, vec2(w, 18.0));
        p.rect_filled(tag, CornerRadius::same(9), Color32::from_black_alpha(190));
        p.text(tag.center(), Align2::CENTER_CENTER, t, theme::bold_font(11.5), col::GOLD2);
    }
}

impl PyrApp {
    /// Opens the casino on a server (its state comes; big wins while it's open).
    pub fn casino_open(&mut self, s: &ServerView) {
        if self.poker.casino.opened.as_deref() != Some(s.key.as_str()) {
            if let Some(old) = self.poker.casino.opened.take() {
                self.engine.casino_open(&old, false);
            }
            self.poker.casino.opened = Some(s.key.clone());
            self.engine.casino_open(&s.key, true);
        }
    }

    pub fn casino_close(&mut self) {
        if let Some(old) = self.poker.casino.opened.take() {
            self.engine.casino_open(&old, false);
        }
        self.poker.casino.game = None;
        self.poker.casino.hold = None;
        self.poker.casino.shown_chips = None;
        if self.poker.casino.fullscreen {
            self.poker.casino.fullscreen = false;
        }
    }

    /// Plays any sounds whose time has come.
    pub fn casino_sounds(&mut self) {
        let now = Instant::now();
        let due: Vec<_> = self.poker.casino.sounds.iter().filter(|s| s.0 <= now).cloned().collect();
        self.poker.casino.sounds.retain(|s| s.0 > now);
        if self.poker.casino.mute {
            return;
        }
        for (_, cue, pan, vol) in due.into_iter().take(4) {
            self.engine.play_poker(cue, pan, vol);
        }
    }

    /// A casino game's page: the top bar, then the game.
    pub fn casino_game(&mut self, ui: &mut egui::Ui, s: &ServerView, game: Game) {
        self.casino_open(s);
        let c = &s.poker.casino;
        // A request answered (or refused, or given up on).
        if c.answers != self.poker.casino.seen_answers {
            self.poker.casino.seen_answers = c.answers;
            self.poker.casino.pending = None;
        }
        if self.poker.casino.pending.is_some_and(|t| t.elapsed() > Duration::from_secs(10)) {
            self.poker.casino.pending = None;
        }
        // A round came back: the game animates it.
        if c.round_seq != self.poker.casino.seen_round {
            self.poker.casino.seen_round = c.round_seq;
            self.poker.casino.pending = None;
            if let Some(r) = c.rounds.back().cloned() {
                self.casino_round(&r, game);
            }
        }
        let full = ui.max_rect();
        let p = ui.painter().clone();
        p.rect_filled(full, 0, col::NIGHT);
        let bar = Rect::from_min_size(full.min, vec2(full.width(), 56.0));
        let area = Rect::from_min_max(pos2(full.left(), bar.bottom()), full.max);
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(area));
        match game {
            Game::Slots => self.slots_page(&mut child, s, area),
            Game::Blackjack => self.blackjack_page(&mut child, s, area),
            Game::Roulette => self.roulette_page(&mut child, s, area),
            Game::Baccarat => self.baccarat_page(&mut child, s, area),
            Game::Craps => self.craps_page(&mut child, s, area),
            Game::Dice => self.dice_page(&mut child, s, area),
            Game::Keno => self.keno_page(&mut child, s, area),
        }
        self.casino_top_bar(ui, s, game, bar);
        self.big_win_banner(ui, s, area);
        self.announcement(ui, area);
        self.celebration(ui, area);
        self.fairness_window(ui.ctx(), s);
        self.casino_sounds();
        ui.ctx().request_repaint_after(Duration::from_millis(16));
    }

    /// Hands a finished round to its game's table to animate. The balance shows the bet taken
    /// but not the win until the round settles on the table (each game says when).
    fn casino_round(&mut self, r: &RoundView, page: Game) {
        if r.outcome.game() == page {
            let before_win = r.chips.saturating_sub(r.outcome.paid());
            self.poker.casino.hold = Some((before_win, Instant::now() + Duration::from_secs(40)));
        }
        match r.outcome.game() {
            Game::Slots => self.slots_round(r),
            Game::Blackjack => self.blackjack_round(r),
            Game::Roulette => self.roulette_round(r),
            Game::Baccarat => self.baccarat_round(r),
            Game::Craps => self.craps_round(r),
            Game::Dice => self.dice_round(r),
            Game::Keno => self.keno_round(r),
        }
    }

    fn casino_top_bar(&mut self, ui: &mut egui::Ui, s: &ServerView, game: Game, bar: Rect) {
        let p = ui.painter().clone();
        vgrad(&p, bar, Color32::from_rgb(0x1b, 0x16, 0x24), Color32::from_rgb(0x0e, 0x0c, 0x14));
        p.line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0f32, col::GOLD_DARK));
        let back = Rect::from_min_size(bar.left_top() + vec2(12.0, 11.0), vec2(104.0, 34.0));
        if ui.put(back, egui::Button::new(RichText::new("‹ Casino").font(theme::bold_font(13.5)).color(col::TEXT)).fill(Color32::from_white_alpha(14)).corner_radius(theme::r(8))).clicked() {
            self.poker.casino.game = None;
            self.poker.casino.hold = None;
            self.poker.tab = crate::poker::Tab::Floor;
        }
        let c = &s.poker.casino;
        let mut x = bar.right() - 12.0;
        let mut button = |ui: &mut egui::Ui, w: f32, on: bool, tip: &str| -> (egui::Response, Rect) {
            let r = Rect::from_min_size(pos2(x - w, bar.top() + 11.0), vec2(w, 34.0));
            x -= w + 8.0;
            let resp = ui.interact(r, ui.id().with(("casino-bar", tip)), Sense::click());
            let fill = if on { col::GOLD.gamma_multiply(0.85) } else if resp.hovered() { Color32::from_white_alpha(30) } else { Color32::from_white_alpha(14) };
            ui.painter().rect_filled(r, theme::r(8), fill);
            (resp.on_hover_text(tip), r)
        };
        // Sound, fullscreen, streamer mode.
        let mute = self.poker.casino.mute;
        let (resp, r) = button(ui, 38.0, false, if mute { "Sound off: click for the casino's sounds" } else { "Sound on" });
        if mute {
            crate::icons::speaker_off(&p, r.shrink(9.0), col::MUTED);
        } else {
            crate::icons::speaker(&p, r.shrink(9.0), col::TEXT);
        }
        if resp.clicked() {
            self.poker.casino.mute = !mute;
        }
        let fs = self.poker.casino.fullscreen;
        let (resp, r) = button(ui, 38.0, fs, if fs { "Leave fullscreen (Esc)" } else { "Fullscreen: just the casino, the whole screen (F11)" });
        crate::icons::fullscreen(&p, r.shrink(10.0), if fs { col::NIGHT } else { col::TEXT }, fs);
        if resp.clicked() {
            self.poker.casino.fullscreen = !fs;
        }
        let streamer = self.streamer_mode();
        let (resp, r) = button(ui, 38.0, streamer, if streamer { "Streamer mode is on: your cards at poker stay face down until you peek (hold Space), invite links are hidden, and the game is laid out for a stream. Click for its layout." } else { "Streamer mode, for when you're live: click to set it up" });
        crate::icons::eye(&p, r.shrink(10.0), if streamer { col::NIGHT } else { col::TEXT });
        if resp.clicked() {
            self.poker.casino.stream_menu = if self.poker.casino.stream_menu.is_some() { None } else { Some(r) };
        }
        // How many rounds this Pyr checked.
        let problems = !c.problems.is_empty();
        let text = if problems { format!("⚠ {} round{} didn't check out", c.problems.len(), if c.problems.len() == 1 { "" } else { "s" }) } else { format!("✓ {} checked", chips_text(c.checked)) };
        let w = 26.0 + text.chars().count() as f32 * 7.4;
        let (resp, r) = button(ui, w, false, "Every round is played from the house's secret seed and one from your Pyr. Afterwards the house shows its seed, and your Pyr plays the round again to be sure. Click to see how.");
        p.text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(12.5), if problems { col::LOSE } else { col::WIN });
        if resp.clicked() {
            self.poker.casino.fairness_open = true;
        }
        // My chips: a bet comes off at once; a win goes on as the round settles, counting up.
        let real = chips_now(s);
        let now = Instant::now();
        let cu = &mut self.poker.casino;
        let target = match cu.hold {
            Some((held, until)) if now < until => held,
            _ => {
                cu.hold = None;
                real
            }
        };
        if target > cu.last_target && cu.shown_chips.is_some() {
            cu.gain = Some((target - cu.last_target, now));
        }
        cu.last_target = target;
        let shown = match cu.shown_chips {
            Some(v) if (target as f64) > v => {
                // (Up by a fraction each frame, at least a little, so a win counts up quickly.)
                let step = ((target as f64 - v) * 0.11).max(1.0);
                (v + step).min(target as f64)
            }
            _ => target as f64,
        };
        cu.shown_chips = Some(shown);
        let t = chips_text(shown.round() as u64);
        // (Bigger in streamer mode: the bankroll is what a stream's viewers watch.)
        let streaming = self.streamer_mode();
        let (fs, ph) = if streaming { (22.0, 42.0) } else { (17.0, 38.0) };
        let w = 52.0 + chips_text(target.max(real)).len() as f32 * fs * 0.56;
        let r = Rect::from_min_size(pos2(x - w, bar.center().y - ph / 2.0), vec2(w, ph));
        let counting = (shown.round() as u64) < target;
        p.rect_filled(r, theme::r(19), Color32::from_black_alpha(140));
        p.rect_stroke(r, theme::r(19), Stroke::new(if counting { 1.5f32 } else { 1.0 }, if counting { col::GOLD } else { col::GOLD_DARK }), StrokeKind::Inside);
        flat_chip(&p, r.left_center() + vec2(19.0, 0.0), 12.0, 1_000, false);
        p.text(r.left_center() + vec2(38.0, 0.0), Align2::LEFT_CENTER, t, theme::bold_font(fs), if counting { col::WIN } else { col::GOLD2 });
        let bal = r;
        // Streamer mode: how the session's going, since streamer mode came on (chips up or
        // down), beside the balance.
        let settled = target;
        let cu = &mut self.poker.casino;
        if !streaming {
            cu.session_start = None;
        } else if cu.session_start.is_none() && cu.hold.is_none() {
            cu.session_start = Some(settled);
        }
        let r = match cu.session_start {
            Some(start) if streaming => {
                let d = settled as i64 - start as i64;
                let text = format!("SESSION {}{}", if d > 0 { "+" } else if d < 0 { "−" } else { "" }, chips_text(d.unsigned_abs()));
                let sw = 30.0 + text.chars().count() as f32 * 9.0;
                let sr = Rect::from_min_size(pos2(r.left() - 10.0 - sw, bar.center().y - 17.0), vec2(sw, 34.0));
                let color = if d > 0 { col::WIN } else if d < 0 { col::LOSE } else { col::MUTED };
                p.rect_filled(sr, theme::r(17), Color32::from_black_alpha(140));
                p.rect_stroke(sr, theme::r(17), Stroke::new(1.0f32, color.gamma_multiply(0.6)), StrokeKind::Inside);
                p.text(sr.center(), Align2::CENTER_CENTER, text, theme::bold_font(15.0), color);
                Rect::from_min_max(sr.min, r.max)
            }
            _ => r,
        };
        // The game's name and what it is, in the room left of the balance (cut short in a
        // narrow window rather than running under it).
        let words = p.with_clip_rect(Rect::from_min_max(bar.min, pos2(r.left() - 14.0, bar.max.y)));
        words.text(pos2(back.right() + 16.0, bar.center().y - 8.0), Align2::LEFT_CENTER, game.name(), theme::bold_font(17.0), col::GOLD2);
        words.text(pos2(back.right() + 16.0, bar.center().y + 11.0), Align2::LEFT_CENTER, game.pitch(), theme::font(12.0), col::MUTED);
        // The gain, floating up off the balance.
        if let Some((g, since)) = self.poker.casino.gain {
            let k = since.elapsed().as_secs_f32() / 1.8;
            if k >= 1.0 {
                self.poker.casino.gain = None;
            } else {
                let a = (1.0 - k).powf(0.6) * (k / 0.08).min(1.0);
                let text = format!("+{}", chips_text(g));
                let at = pos2(bal.center().x, bal.bottom() + 18.0 + ease_out(k) * 22.0);
                let pill = Rect::from_center_size(at, vec2(24.0 + text.len() as f32 * 9.0, 26.0));
                p.rect_filled(pill, theme::r(13), Color32::from_rgba_unmultiplied(0x0c, 0x1a, 0x10, (230.0 * a) as u8));
                p.rect_stroke(pill, theme::r(13), Stroke::new(1.0f32, col::WIN.gamma_multiply(0.7 * a)), StrokeKind::Inside);
                p.text(pill.center(), Align2::CENTER_CENTER, text, theme::bold_font(15.0), col::WIN.gamma_multiply(a));
            }
        }
        // Esc and F11.
        let (esc, f11) = ui.ctx().input(|i| (i.key_pressed(egui::Key::Escape), i.key_pressed(egui::Key::F11)));
        if esc && self.poker.casino.fullscreen && self.poker.casino.stream_menu.is_none() {
            self.poker.casino.fullscreen = false;
        }
        if f11 {
            self.poker.casino.fullscreen = !self.poker.casino.fullscreen;
        }
    }

    /// A game's rack of chips and its bet in words, in the room its controls leave left of
    /// their buttons (`right`: the leftmost button's edge, less its gap). In a narrow window
    /// (streamer mode's room for a camera, say) the biggest chips make way first (the bet's for
    /// a stream's viewers too), then the words: never one over the other. Returns where the
    /// words went.
    pub fn rack_and_bet(&mut self, ui: &mut egui::Ui, bar: Rect, right: f32, max_chip: u64, text: String, color: Color32) -> Option<Rect> {
        let at = pos2(bar.left() + 16.0, bar.center().y - 26.0);
        let room = right - 10.0 - at.x;
        let all = CHIPS.iter().filter(|v| **v <= max_chip).count();
        let galley = ui.painter().layout_no_wrap(text, theme::bold_font(16.0), color);
        let words = galley.size().x;
        let fits = |w: f32| (((w + 8.0) / 52.0).floor().max(0.0) as usize).min(all);
        let beside_words = fits(room - 18.0 - words);
        let (n, with_words) = if beside_words >= all.min(3) { (beside_words, true) } else { (fits(room).max(1), false) };
        let biggest = CHIPS.iter().copied().filter(|v| *v <= max_chip).take(n).last().unwrap_or(1);
        self.chip_rack(ui, at, biggest);
        if !with_words {
            return None;
        }
        let r = Rect::from_min_size(pos2(right - 10.0 - words, bar.center().y - galley.size().y / 2.0), galley.size());
        ui.painter().galley(r.min, galley, color);
        Some(r)
    }

    /// The rack of chips to bet with; returns the rack's height.
    pub fn chip_rack(&mut self, ui: &mut egui::Ui, at: Pos2, max_chip: u64) -> f32 {
        let r = 22.0;
        let mut x = at.x;
        for v in CHIPS {
            if v > max_chip {
                break;
            }
            let c = pos2(x + r, at.y + r);
            let hit = Rect::from_center_size(c, vec2(r * 2.0, r * 2.0));
            let resp = ui.interact(hit, ui.id().with(("chip", v)), Sense::click());
            let on = self.poker.casino.chip == v;
            let lift = if on { -6.0 } else if resp.hovered() { -3.0 } else { 0.0 };
            if on {
                ui.painter().circle_filled(c + vec2(0.0, lift), r + 4.0, col::GOLD.gamma_multiply(0.35));
            }
            flat_chip(ui.painter(), c + vec2(0.0, lift), r, v, true);
            if resp.on_hover_text(format!("Bet {} at a click (right-click a bet to take chips off)", chips_text(v))).clicked() {
                self.poker.casino.chip = v;
            }
            x += r * 2.0 + 8.0;
        }
        r * 2.0 + 8.0
    }

    /// A big win over the table: its name, the chips counting up, coins falling.
    fn celebration(&mut self, ui: &mut egui::Ui, area: Rect) {
        let Some(cel) = &self.poker.casino.celebration else { return };
        let now = Instant::now();
        if now < cel.since {
            return;
        }
        let t = (now - cel.since).as_secs_f32();
        let life = if cel.tier >= 4 { 7.0 } else { 4.5 };
        let resp = ui.interact(area, ui.id().with("celebration"), Sense::click());
        if t > life || resp.clicked() {
            self.poker.casino.celebration = None;
            return;
        }
        let p = ui.painter().clone();
        let fade = (t / 0.3).min(1.0) * ((life - t) / 0.6).clamp(0.0, 1.0);
        p.rect_filled(area, 0, Color32::from_black_alpha((150.0 * fade) as u8));
        // Coins.
        let n = 40 + cel.tier as usize * 25;
        for i in 0..n {
            let seed = (i as f32 * 12.9898).sin() * 43758.547;
            let fx = seed.fract().abs();
            let start = ((i as f32 * 0.37).fract()) * 1.6;
            let tt = t - start;
            if tt < 0.0 {
                continue;
            }
            let y = area.top() - 20.0 + tt * tt * 260.0 + tt * 120.0;
            if y > area.bottom() + 20.0 {
                continue;
            }
            let x = area.left() + fx * area.width() + (tt * 3.0 + i as f32).sin() * 18.0;
            let spin = (tt * 8.0 + i as f32).cos().abs().max(0.15);
            let c = pos2(x, y);
            p.add(egui::epaint::EllipseShape::filled(c, vec2(9.0 * spin, 9.0), col::GOLD.gamma_multiply(fade)));
            p.add(egui::epaint::EllipseShape::stroke(c, vec2(9.0 * spin, 9.0), Stroke::new(1.5f32, col::GOLD_DARK.gamma_multiply(fade))));
        }
        let c = area.center() - vec2(0.0, 40.0);
        let pop = ease_back((t / 0.5).min(1.0));
        let size = (if cel.tier >= 4 { 64.0 } else { 56.0 }) * pop.max(0.2);
        radial(&p, c, 260.0 * pop.max(0.1), col::EMBER.gamma_multiply(0.35 * fade), Color32::TRANSPARENT);
        glow_text(&p, c, Align2::CENTER_CENTER, cel.title, theme::bold_font(size), col::GOLD2.gamma_multiply(fade), col::EMBER.gamma_multiply(0.5 * fade));
        let shown = (cel.amount as f64 * ease_out((t / 2.2).min(1.0)) as f64) as u64;
        glow_text(&p, c + vec2(0.0, size * 0.95), Align2::CENTER_CENTER, &chips_text(shown), theme::bold_font(size * 0.75), Color32::WHITE.gamma_multiply(fade), col::GOLD_DARK.gamma_multiply(0.6 * fade));
        p.text(c + vec2(0.0, size * 1.65), Align2::CENTER_CENTER, format!("{} your bet", times(cel.multiple_x100)), theme::bold_font(18.0), col::GOLD.gamma_multiply(fade));
        p.text(c + vec2(0.0, size * 1.65 + 28.0), Align2::CENTER_CENTER, "click to carry on", theme::font(12.5), col::MUTED.gamma_multiply(fade));
    }

    /// A round's result, called out big (see `CasinoUi::announce`): it pops up, stays a
    /// couple of seconds, and fades. A big win's celebration takes its place.
    fn announcement(&mut self, ui: &mut egui::Ui, area: Rect) {
        let Some(a) = &self.poker.casino.announcement else { return };
        let now = Instant::now();
        if now < a.since {
            return;
        }
        let t = (now - a.since).as_secs_f32();
        const LIFE: f32 = 2.9;
        let celebrating = self.poker.casino.celebration.as_ref().is_some_and(|c| now >= c.since);
        if t > LIFE || celebrating {
            self.poker.casino.announcement = None;
            return;
        }
        let p = ui.painter().clone();
        let pop = ease_back((t / 0.32).min(1.0));
        let fade = ((LIFE - t) / 0.45).clamp(0.0, 1.0);
        let title_size = (area.width() / 26.0).clamp(30.0, 52.0);
        let sub_size = title_size * 0.45;
        let title = p.layout_no_wrap(a.title.clone(), theme::bold_font(title_size), a.color);
        let sub = (!a.sub.is_empty()).then(|| p.layout_no_wrap(a.sub.clone(), theme::bold_font(sub_size), col::TEXT));
        let w = title.size().x.max(sub.as_ref().map(|g| g.size().x).unwrap_or(0.0)) + title_size * 1.4;
        let h = title.size().y + sub.as_ref().map(|g| g.size().y + 6.0).unwrap_or(0.0) + title_size * 0.6;
        let c = pos2(area.left() + area.width() * a.at.0, area.top() + area.height() * a.at.1);
        let scale = 0.6 + 0.4 * pop;
        let r = Rect::from_center_size(c, vec2(w, h) * scale);
        p.rect_filled(r.translate(vec2(0.0, 6.0)), theme::r(22), Color32::from_black_alpha((110.0 * fade) as u8));
        p.rect_filled(r, theme::r(22), Color32::from_rgba_unmultiplied(0x0c, 0x0a, 0x12, (232.0 * fade) as u8));
        p.rect_stroke(r, theme::r(22), Stroke::new(2.5f32, a.color.gamma_multiply(fade)), StrokeKind::Inside);
        radial(&p, c, w * 0.6 * scale, a.color.gamma_multiply(0.18 * fade), Color32::TRANSPARENT);
        let ty = if sub.is_some() { r.top() + h * scale * 0.42 } else { c.y };
        glow_text(&p, pos2(c.x, ty), Align2::CENTER_CENTER, &a.title, theme::bold_font(title_size * scale), a.color.gamma_multiply(fade), Color32::from_black_alpha((160.0 * fade) as u8));
        if sub.is_some() {
            p.text(pos2(c.x, r.bottom() - h * scale * 0.24), Align2::CENTER_CENTER, &a.sub, theme::bold_font(sub_size * scale), col::TEXT.gamma_multiply(fade));
        }
    }

    /// Someone's big win, just now: a banner sliding in at the top for a few seconds (my own
    /// wins have the celebration instead).
    fn big_win_banner(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let c = &s.poker.casino;
        let id = egui::Id::new("casino-big-win-shown");
        if c.big_win_seq != self.poker.casino.seen_big_win {
            self.poker.casino.seen_big_win = c.big_win_seq;
            let mine = s.me.as_ref().map(|m| m.name.clone());
            if let Some(w) = c.big_wins.first()
                && Some(&w.name) != mine.as_ref()
            {
                ui.ctx().data_mut(|d| d.insert_temp(id, (Instant::now(), format!("{} just won {} on {} ({})", w.name, chips_text(w.paid), w.game.name(), w.what))));
            }
        }
        let Some((since, text)) = ui.ctx().data(|d| d.get_temp::<(Instant, String)>(id)) else { return };
        let t = since.elapsed().as_secs_f32();
        if t > 5.0 {
            ui.ctx().data_mut(|d| d.remove::<(Instant, String)>(id));
            return;
        }
        let slide = ease_out((t / 0.35).min(1.0)) * ((5.0 - t) / 0.35).clamp(0.0, 1.0);
        let w = 56.0 + text.chars().count() as f32 * 7.8;
        let r = Rect::from_center_size(pos2(area.center().x, area.top() - 24.0 + 48.0 * slide), vec2(w, 36.0));
        let p = ui.painter();
        p.rect_filled(r, theme::r(18), Color32::from_rgba_unmultiplied(0x2a, 0x10, 0x04, 235));
        p.rect_stroke(r, theme::r(18), Stroke::new(1.5f32, col::GOLD), StrokeKind::Inside);
        crate::icons::flame(p, Rect::from_center_size(r.left_center() + vec2(20.0, 0.0), vec2(18.0, 18.0)));
        p.text(r.center() + vec2(10.0, 0.0), Align2::CENTER_CENTER, text, theme::bold_font(13.5), col::GOLD2);
    }

    /// How the rounds are checked, and any that didn't check out.
    fn fairness_window(&mut self, ctx: &egui::Context, s: &ServerView) {
        if !self.poker.casino.fairness_open {
            return;
        }
        let mut open = true;
        let c = &s.poker.casino;
        egui::Window::new("Every round, checked").collapsible(false).resizable(false).default_width(520.0).open(&mut open).show(ctx, |ui| {
            ui.label(RichText::new("Pyr Casino can't choose how a round comes out, and neither can you.").strong().size(15.0));
            ui.add_space(6.0);
            for line in [
                "1. Before each round the house picks a secret seed and shows you only its fingerprint (a SHA-256 hash).",
                "2. Your Pyr makes a seed of its own and sends it with your bet. The house can't know it in advance.",
                "3. The reels, cards, ball, dice or numbers come from both seeds together and the round's number.",
                "4. After the round the house shows its seed. Your Pyr checks it matches the fingerprint, then plays the whole round again itself, to be sure every reel, card, ball and die came out exactly as the house said.",
            ] {
                ui.label(RichText::new(line).size(13.5));
            }
            ui.add_space(8.0);
            ui.label(RichText::new(format!("This Pyr has checked {} round{} on {} so far.", chips_text(c.checked), if c.checked == 1 { "" } else { "s" }, s.name)).color(col::WIN).strong());
            for pr in &c.problems {
                ui.label(RichText::new(format!("⚠ {pr}")).color(col::LOSE));
            }
            if let Some(r) = c.rounds.back() {
                ui.add_space(8.0);
                ui.collapsing("The last round's seeds", |ui| {
                    let hex = |b: &[u8; 32]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
                    ui.label(RichText::new(format!("Round {} ({})", r.nonce, r.outcome.game().name())).strong());
                    ui.label(RichText::new(format!("Fingerprint shown first: {}", hex(&r.commit))).monospace().size(11.0));
                    ui.label(RichText::new(format!("House seed shown after: {}", hex(&r.server_seed))).monospace().size(11.0));
                    ui.label(RichText::new(format!("Your Pyr's seed: {}", hex(&r.client_seed))).monospace().size(11.0));
                    ui.label(RichText::new("The round's seed is SHA-256 of \"pyr-casino round v1\", the house seed, your seed, the game's id and the round's number; its random numbers are SHA-256 in counter mode, each draw by rejection sampling so every result is equally likely.").size(11.5).color(theme::DIM()));
                });
            }
            ui.add_space(6.0);
            ui.label(RichText::new("What the house keeps over time: slots 5.5% (1% of it goes back through the Phoenix Jackpot), blackjack about 0.5% played well, roulette 2.7%, baccarat 1.06% on Banker, craps 1.4% on the pass line (0% on the odds), dice 1%, keno about 5%.").size(12.5).color(theme::MUTED()));
        });
        if !open {
            self.poker.casino.fairness_open = false;
        }
    }

    /// Streamer mode: on by choice, or by itself while I'm sharing my screen.
    pub fn streamer_mode(&self) -> bool {
        self.ui.streamer_mode || self.ui.streamer_auto_live
    }

    /// Streamer mode is on only by itself, because I'm sharing my screen in a call (not by my
    /// choice): every switch for it says so, or it looks off while my cards stay hidden.
    pub fn streamer_auto(&self) -> bool {
        self.ui.streamer_auto_live && !self.ui.streamer_mode
    }

    pub fn set_streamer_mode(&mut self, on: bool) {
        self.ui.streamer_set = Some(on);
        self.ui.streamer_mode = on;
        if !on && self.ui.streamer_auto_live {
            // (Turned off while sharing: it stays off until the next share.)
            self.ui.streamer_auto_live = false;
            self.ui.streamer_auto_off = true;
        }
    }

    /// Pyr Casino fullscreen: only the casino, on the whole screen (Esc or the button leaves).
    /// Or streamer mode's stage: only the game, in Pyr's window (see `staged`).
    pub fn casino_fullscreen_ui(&mut self, ctx: &egui::Context, st: &pyr_engine::state::AppState, acts: &mut Vec<crate::app::Act>) -> bool {
        if self.poker.casino.game.is_none() && self.poker.table.is_none() {
            self.poker.casino.stage_off = false;
        }
        let server = self.poker.open.clone().and_then(|k| st.server(&k).cloned());
        if self.poker.casino.fullscreen && server.is_none() {
            self.poker.casino.fullscreen = false;
        }
        let fullscreen = self.poker.casino.fullscreen;
        if fullscreen != self.poker.casino.fullscreen_on {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(fullscreen));
            self.poker.casino.fullscreen_on = fullscreen;
        }
        let staged = self.staged();
        if !(fullscreen || staged) || server.is_none() {
            return false;
        }
        let server = server.expect("checked");
        let menu_open = self.poker.casino.stream_menu.is_some();
        egui::CentralPanel::default().frame(egui::Frame::new().fill(col::NIGHT)).show(ctx, |ui| self.poker_page(ui, st, &server, acts));
        // (Esc closes the streaming menu first.)
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) && !menu_open {
            if self.poker.casino.fullscreen {
                self.poker.casino.fullscreen = false;
            } else if staged {
                self.poker.casino.stage_off = true;
            }
        }
        true
    }

    /// Streamer mode's stage: while a casino game or a poker table is open, it fills Pyr's
    /// window, Pyr's server and channel lists put away (a window capture shows the game, and
    /// nothing of which servers and channels are mine). Esc brings them back until the game's
    /// left.
    pub fn staged(&self) -> bool {
        self.streamer_mode() && self.ui.stream_stage && !self.poker.casino.stage_off && self.poker.open.is_some() && (self.poker.casino.game.is_some() || self.poker.table.is_some())
    }

    /// At a casino game or a poker table in streamer mode: room kept clear down one side for a
    /// camera and overlays (a full-height column, the stream's own background), and the rest for
    /// the game, which lays itself out in it.
    pub fn stream_split(&self, full: Rect) -> (Rect, Option<Rect>) {
        if !self.streamer_mode() {
            return (full, None);
        }
        let w = (full.width() * 0.25).clamp(260.0, 520.0);
        if full.width() - w < 700.0 {
            return (full, None);
        }
        match self.ui.stream_room.as_str() {
            "left" => (Rect::from_min_max(pos2(full.left() + w, full.top()), full.max), Some(Rect::from_min_max(full.min, pos2(full.left() + w, full.bottom())))),
            "right" => (Rect::from_min_max(full.min, pos2(full.right() - w, full.bottom())), Some(Rect::from_min_max(pos2(full.right() - w, full.top()), full.max))),
            _ => (full, None),
        }
    }

    /// The room kept clear: plain (it's under the stream's overlays), saying what it is only
    /// while the streaming menu's open (never on the stream by itself).
    pub fn stream_room_ui(&mut self, ui: &mut egui::Ui, room: Rect) {
        let p = ui.painter();
        p.rect_filled(room, 0, Color32::from_rgb(0x08, 0x07, 0x0b));
        let edge = if room.left() <= ui.max_rect().left() + 1.0 { room.right() } else { room.left() };
        p.line_segment([pos2(edge, room.top()), pos2(edge, room.bottom())], Stroke::new(1.0f32, col::GOLD_DARK.gamma_multiply(0.5)));
        if self.poker.casino.stream_menu.is_some() {
            p.text(room.center(), Align2::CENTER_CENTER, "Kept clear for your camera\nand overlays (streamer mode)", theme::font(13.0), col::MUTED);
        }
    }

    /// The stream layout's menu, under the button that opened it.
    pub fn stream_menu_ui(&mut self, ctx: &egui::Context, acts: &mut Vec<crate::app::Act>) {
        let Some(anchor) = self.poker.casino.stream_menu else { return };
        let streamer = self.streamer_mode();
        let (room, stage) = (self.ui.stream_room.clone(), self.ui.stream_stage);
        let area = egui::Area::new(egui::Id::new("stream-menu"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos2(anchor.right() - 340.0, anchor.bottom() + 8.0))
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).fill(Color32::from_rgb(0x16, 0x12, 0x1e)).stroke(Stroke::new(1.0f32, col::GOLD_DARK)).inner_margin(14.0).show(ui, |ui| {
                    ui.set_width(312.0);
                    ui.label(RichText::new("Streaming").font(theme::bold_font(16.0)).color(col::GOLD2));
                    ui.add_space(6.0);
                    let mut on = streamer;
                    let auto = self.streamer_auto();
                    if ui.checkbox(&mut on, RichText::new("Streamer mode").strong()).changed() {
                        self.set_streamer_mode(on);
                    }
                    if auto {
                        ui.label(RichText::new("On by itself: you're sharing your screen in a call. Turn it off here and it stays off until your next share.").size(12.0).color(col::GOLD2));
                    }
                    ui.label(RichText::new("Your cards at poker stay face down until you peek (hold Space), invite links are hidden, and games are laid out for a stream. It turns on by itself while you share your screen.").size(12.0).color(col::MUTED));
                    ui.add_space(8.0);
                    ui.add_enabled_ui(on, |ui| {
                        let mut st = stage;
                        if ui.checkbox(&mut st, "Just the game in Pyr's window").changed() {
                            acts.push(crate::app::Act::StreamLayout { room: None, stage: Some(st) });
                        }
                        ui.label(RichText::new("While you play, Pyr's server and channel lists are put away: a window capture shows the game, and nothing of your servers. Esc brings them back.").size(12.0).color(col::MUTED));
                        ui.add_space(8.0);
                        ui.label(RichText::new("Room for your camera and overlays").strong());
                        ui.horizontal(|ui| {
                            for (key, name) in [("", "None"), ("left", "Left side"), ("right", "Right side")] {
                                if ui.selectable_label(room == key, name).clicked() && room != key {
                                    acts.push(crate::app::Act::StreamLayout { room: Some(key.to_string()), stage: None });
                                }
                            }
                        });
                        ui.label(RichText::new("A quarter of the window, top to bottom, kept clear: the game fits itself into the rest.").size(12.0).color(col::MUTED));
                    });
                });
            });
        // Closed by a click anywhere else.
        let clicked_away = ctx.input(|i| i.pointer.any_pressed()) && ctx.pointer_interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !anchor.contains(p));
        if clicked_away || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.poker.casino.stream_menu = None;
        }
    }
}
