//! Reel Row's machines from 0.13.0 (on servers that have it): each machine its own room, its
//! own lettering, cabinet, spin button and win (`row_styles`), its own mini game (`minis`),
//! and Play Feature and Fairness (`stake`) as Stake's slots have them. The rounds are the
//! house's (`pyr_poker::casino::reelrow`), played from my seed for the machine and checked by
//! this Pyr like every other.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::casino::feature::Feature;
use pyr_poker::casino::reels::{self as rr, BETS, REELS, ROWS};
use pyr_poker::casino::reelrow;
use pyr_poker::casino::Outcome;
use pyr_poker::net::RoundView;

use super::humor::Fun;
use super::minis::MiniPlay;
use super::reel_themes::{Theme, theme};
use super::row_styles::{Button, Frame, Fx, Style, style};
use super::spin::{Reel, SPEED};
use super::stake::{Pick, SlotKind, StakeUi};
use super::{col, glow_text, mix, pics, radial, vgrad, vgrad_round};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

/// Filler under the reels while they spin: this many symbols round each strip.
const STRIP: usize = 24;

/// Something flying up out of a win.
#[derive(Clone, Copy)]
struct Particle {
    at: Pos2,
    vel: egui::Vec2,
    born: Instant,
    life: f32,
    seed: u32,
}

pub struct RowUi {
    pub machine: u8,
    pub bet: u64,
    reels: [Reel; REELS],
    strips: [[u8; STRIP]; REELS],
    /// The round on the reels, when they all stop, and whether its wins have been counted.
    showing: Option<RoundView>,
    stopped_at: Option<Instant>,
    counted: Option<Instant>,
    waiting: Option<Instant>,
    /// The mini game being played on screen.
    pub(super) mini: Option<MiniPlay>,
    /// All or nothing being revealed: since when.
    all_in: Option<(RoundView, Instant, bool)>,
    /// The round's win, as counted so far.
    pub win: u64,
    pub auto: super::auto::Auto,
    pub turbo: bool,
    next_at: Option<Instant>,
    pub fun: Fun,
    pub stake: StakeUi,
    fx: Vec<Particle>,
}

impl Default for RowUi {
    fn default() -> Self {
        RowUi {
            machine: 0,
            bet: 100,
            reels: std::array::from_fn(|r| Reel::still((r * 5) as f32)),
            strips: std::array::from_fn(|r| filler(r as u64 * 7_919 + 3)),
            showing: None,
            stopped_at: None,
            counted: None,
            waiting: None,
            mini: None,
            all_in: None,
            win: 0,
            auto: Default::default(),
            turbo: false,
            next_at: None,
            fun: Fun::default(),
            stake: StakeUi::default(),
            fx: Vec::new(),
        }
    }
}

/// Symbols to fill a strip with (roughly the machine's own odds).
fn filler(seed: u64) -> [u8; STRIP] {
    let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    std::array::from_fn(|_| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        match (x >> 11) % 1000 {
            0..200 => 0,
            200..370 => 1,
            370..510 => 2,
            510..610 => 3,
            610..670 => 4,
            670..700 => 5,
            700..725 => rr::WILD,
            725..750 => rr::SCATTER,
            _ => rr::BLANK,
        }
    })
}

impl RowUi {
    fn running(&self, now: Instant) -> bool {
        self.waiting.is_some() || self.stopped_at.is_some_and(|e| now < e) || self.mini.as_ref().is_some_and(|b| !b.collected) || self.all_in.is_some()
    }

    /// What a spin (with the armed feature) costs.
    fn spin_cost(&self) -> u64 {
        pyr_poker::casino::feature::cost(self.bet, reelrow::price(self.stake.armed.unwrap_or(Feature::Spin)))
    }
}

/// Where everything goes on the page.
struct Layout {
    /// The room's art (it may run off the page), and the part of the page it covers.
    art: Rect,
    /// The reels, their cells, and the cabinet round them.
    reels: Rect,
    cell: f32,
    gap: f32,
    cabinet: Rect,
    /// The machine's name above the reels.
    marquee: Pos2,
    /// The controls along the bottom.
    bar: Rect,
}

fn layout(area: Rect, st: &Style, funny: bool) -> Layout {
    let bar_h = 104.0;
    let bar = Rect::from_min_max(pos2(area.left() + 16.0, area.bottom() - bar_h), pos2(area.right() - 16.0, area.bottom() - 10.0));
    let side = if funny { (area.width() * 0.17).clamp(130.0, 220.0) } else { 0.0 };
    let room = Rect::from_min_max(pos2(area.left() + side, area.top()), pos2(area.right(), bar.top() - 8.0));
    const ART: egui::Vec2 = vec2(1376.0, 774.0);
    match st.window {
        Some(win) => {
            // The art as big as fits with the room's place for the reels at its fullest, then
            // the reels in that place (a little in from its edges).
            let ww = win.width() * ART.x;
            let wh = win.height() * ART.y;
            // (No bigger than a little past covering the page: the room should show round them.)
            let cover = (area.width() / ART.x).max(area.height() / ART.y);
            let k = ((room.width() - 40.0) / ww).min((room.height() - 70.0) / wh).min(cover * 1.12);
            let art_size = ART * k;
            let win_c = pos2(room.center().x, room.top() + 52.0 + (room.height() - 52.0) / 2.0);
            let art_min = win_c - vec2((win.min.x + win.width() / 2.0) * art_size.x, (win.min.y + win.height() / 2.0) * art_size.y);
            let art = Rect::from_min_size(art_min, art_size);
            let place = Rect::from_min_max(art.min + vec2(win.min.x * art_size.x, win.min.y * art_size.y), art.min + vec2(win.max.x * art_size.x, win.max.y * art_size.y)).shrink2(vec2(ww * k * 0.03, wh * k * 0.04));
            let cell = (place.width() / 5.2).min(place.height() / 3.05);
            let gap = (cell * 0.05).max(3.0);
            let rw = cell * 5.0 + gap * 4.0;
            let reels = Rect::from_center_size(place.center(), vec2(rw, cell * 3.0));
            Layout { art, reels, cell, gap, cabinet: reels.expand(cell * 0.12), marquee: pos2(reels.center().x, (reels.top() - cell * 0.42).max(area.top() + 30.0)), bar }
        }
        None => {
            // The art covering the page; the reels in a cabinet in the middle.
            let k = (area.width() / ART.x).max(area.height() / ART.y);
            let art = Rect::from_center_size(area.center(), ART * k);
            let top_h = 96.0;
            let max_w = (room.width() - 80.0).min(1060.0);
            let cell = ((room.height() - top_h - 50.0) / 3.0).min((max_w - 80.0) / 5.0).clamp(56.0, 168.0);
            let gap = (cell * 0.06).max(4.0);
            let rw = cell * 5.0 + gap * 4.0;
            let cab_h = top_h + cell * 3.0 + 34.0;
            let top = room.top() + ((room.height() - cab_h) / 2.0).max(8.0);
            let cabinet = Rect::from_center_size(pos2(room.center().x, top + cab_h / 2.0), vec2(rw + 70.0, cab_h));
            let reels = Rect::from_min_size(pos2(cabinet.center().x - rw / 2.0, cabinet.top() + top_h), vec2(rw, cell * 3.0));
            Layout { art, reels, cell, gap, cabinet, marquee: pos2(cabinet.center().x, cabinet.top() + 44.0), bar }
        }
    }
}

impl PyrApp {
    /// Opens a machine.
    pub fn row_open(&mut self, machine: u8) {
        let ui = &mut self.poker.casino.row;
        if ui.machine != machine {
            ui.machine = machine;
            ui.showing = None;
            ui.mini = None;
            ui.all_in = None;
            ui.auto.stop();
            ui.counted = None;
            ui.win = 0;
            ui.stake.armed = None;
            ui.stake.menu = false;
        }
        // (The Info panel and the top bar go by the old machine's number too.)
        self.poker.casino.reels.machine = machine;
        self.poker.casino.game = Some(pyr_poker::casino::Game::Reels);
    }

    /// A round came back: the reels land on it, or a bought bonus (or all or nothing) starts.
    pub(super) fn row_round(&mut self, round: &RoundView) {
        let Outcome::Row(o) = &round.outcome else { return };
        let now = Instant::now();
        let ui = &mut self.poker.casino.row;
        ui.waiting = None;
        ui.win = 0;
        ui.counted = None;
        ui.showing = Some(round.clone());
        if o.feature == Feature::AllIn {
            ui.stopped_at = Some(now);
            ui.all_in = Some((round.clone(), now, false));
            self.poker.casino.sound_in(0.0, Cue::Fuse, 0.0, 0.7);
            return;
        }
        let Some(grid) = o.grid else {
            // A bonus bought: its intro, then the game.
            ui.stopped_at = Some(now);
            ui.mini = Some(MiniPlay::new(round.clone(), now + Duration::from_millis(1_500)));
            let th = theme(o.machine);
            let (bonus, sup) = SlotKind::Row(o.machine).bonus_name();
            let title = if o.bonus.as_ref().is_some_and(|b| b.times > 1) { sup } else { bonus };
            let sub = match o.feature {
                Feature::Mystery if o.lucky => "the Mystery Bonus turned out SUPER: every prize ×3".to_string(),
                Feature::Mystery => "the Mystery Bonus".to_string(),
                Feature::Super => "every prize ×3".to_string(),
                _ => th.tagline.to_string(),
            };
            self.poker.casino.announce(title.to_uppercase(), sub, th.accent, (0.5, 0.42), 0.0);
            self.poker.casino.sound_in(0.0, Cue::Jackpot, 0.0, 0.6);
            return;
        };
        for (r, strip) in ui.strips.iter_mut().enumerate() {
            let mut s = filler(round.nonce.wrapping_mul(31).wrapping_add(r as u64));
            s[..ROWS].copy_from_slice(&grid[r]);
            *strip = s;
        }
        let speed = SPEED * ui.fun.reel_speed(theme(o.machine).humor);
        for reel in &mut ui.reels {
            if reel.speed <= 0.0 || reel.landed(now) {
                reel.start(now, speed);
            }
        }
        // (A Chance spin with the bonus coming: the last reels take their time, for the
        // suspense.)
        let slow = o.scatters >= 2 && !ui.turbo;
        let min = if ui.turbo { 0.25 } else { 0.7 };
        let ends = super::spin::land(&mut ui.reels, &[STRIP; REELS], &[0; REELS], now + Duration::from_secs_f32(min), ui.turbo);
        let mut ends = ends;
        if slow {
            for (i, reel) in ui.reels.iter_mut().enumerate().skip(3) {
                if let Some((st, from, to, dur)) = reel.land {
                    let extra = Duration::from_millis(500 * (i as u64 - 2));
                    reel.land = Some((st + extra, from - speed * extra.as_secs_f32(), to - speed * extra.as_secs_f32(), dur));
                    ends[i] += extra;
                }
            }
        }
        let last = ends.iter().max().copied().unwrap_or(now) + Duration::from_secs_f32(super::spin::land_secs(ui.turbo) * 0.45);
        ui.stopped_at = Some(last);
        for (i, e) in ends.iter().enumerate() {
            self.poker.casino.sound_in(e.saturating_duration_since(now).as_secs_f32(), Cue::ReelStop, (i as f32 - 2.0) * 0.3, 0.7);
        }
    }

    /// A round: a spin (with the armed feature), or a feature bought.
    fn row_play(&mut self, s: &ServerView, feature: Feature) {
        let now = Instant::now();
        let ui = &self.poker.casino.row;
        let cost = pyr_poker::casino::feature::cost(ui.bet, reelrow::price(feature));
        if cost > super::chips_now(s) {
            self.poker.casino.row.auto.stop();
            return;
        }
        if self.poker.casino.busy() || ui.running(now) {
            return;
        }
        let (machine, bet) = (ui.machine, ui.bet);
        let spins = feature.spins();
        if spins {
            self.poker.casino.sound_in(0.0, Cue::ReelSpin, 0.0, 0.6);
        }
        let ui = &mut self.poker.casino.row;
        if spins {
            let speed = SPEED * ui.fun.reel_speed(theme(machine).humor);
            for r in &mut ui.reels {
                r.start(now, speed);
            }
        }
        ui.waiting = Some(now);
        ui.showing = None;
        ui.mini = None;
        ui.all_in = None;
        ui.counted = None;
        ui.stopped_at = None;
        ui.win = 0;
        self.poker.casino.celebration = None;
        self.poker.casino.pending = Some(now);
        self.play_seeded(s, SlotKind::Row(machine), pyr_poker::casino::Bet::Row { machine, bet, feature });
    }

    pub(super) fn row_page(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let now = Instant::now();
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        let time = ctx.input(|i| i.time) as f32;
        let m = self.poker.casino.row.machine;
        let th = theme(m);
        let st = style(m);
        let kind = SlotKind::Row(m);
        let funny = th.humor.is_some();
        let lay = layout(area, st, funny);
        // The room: under the art (where it doesn't reach), the art dimmed; over it, the art
        // itself, bright where the reels are.
        p.rect_filled(area, 0, th.deep);
        let clip = p.with_clip_rect(area);
        pics::cover(&clip, &ctx, &format!("rr-{}-back", th.id), area, 1376, Color32::from_gray(70));
        let shade = (255.0 * (1.0 - st.dim * 0.9)) as u8;
        pics::part(&clip, &ctx, &format!("rr-{}-back", th.id), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), lay.art, 1376, Color32::from_gray(shade));
        radial(&clip, lay.reels.center(), area.width().max(area.height()) * 0.75, Color32::TRANSPARENT, Color32::from_black_alpha(110));
        vgrad(&clip, Rect::from_min_max(pos2(area.left(), lay.bar.top() - 60.0), area.max), Color32::TRANSPARENT, Color32::from_black_alpha(200));
        // A refused spin: the reels come to rest.
        if let Some(since) = self.poker.casino.row.waiting
            && self.poker.casino.pending.is_none()
            && since.elapsed() > Duration::from_millis(300)
        {
            let r = &mut self.poker.casino.row;
            r.waiting = None;
            for reel in &mut r.reels {
                *reel = Reel::still(reel.pos(now).round());
            }
            r.auto.stop();
        }
        // The cabinet (or the room's own trim), the marquee, the reels.
        self.row_cabinet(&p, th, st, &lay, time);
        let in_mini = self.poker.casino.row.mini.as_ref().is_some_and(|b| now >= b.started);
        let in_all_in = self.poker.casino.row.all_in.is_some();
        if !in_mini && !in_all_in {
            self.row_reels(&p, &ctx, th, st, &lay, now, time);
        }
        self.row_after_stop(s, now);
        // Wins flying up.
        self.row_fx(&p, st, th, now);
        if in_all_in {
            self.row_all_in(ui, s, th, st, lay.reels.expand(lay.cell * 0.3), now, time);
        }
        if self.poker.casino.row.mini.as_ref().is_some_and(|b| now >= b.started) {
            let r = Rect::from_min_max(pos2(area.left() + 12.0, area.top() + 8.0), pos2(area.right() - 12.0, lay.bar.top() - 10.0));
            self.row_mini(ui, s, th, st, r, now, time);
        }
        // A funny machine's host.
        if funny && !in_mini {
            let side = Rect::from_min_max(pos2(area.left() + 10.0, lay.reels.top() - 40.0), pos2(area.left() + (area.width() * 0.17).clamp(130.0, 220.0) - 6.0, lay.bar.top() - 10.0));
            self.poker.casino.row.fun.draw(&p, &ctx, th, side, lay.reels, now, time);
        }
        self.row_controls(ui, s, th, st, lay.bar, now, time);
        // Its top prize, top middle: a progressive jackpot's pool, counting, or how far it goes.
        if !in_mini && !in_all_in {
            use pyr_poker::casino::reelrow::{POOL_SEED, TOPS, Top};
            let t = &TOPS[m as usize];
            let bet = self.poker.casino.row.bet;
            let (line, sub) = match t.top {
                Top::Progressive => {
                    let pool = s.poker.casino.row_pools.get(&m).copied().unwrap_or(POOL_SEED);
                    let mine = pyr_poker::casino::reelrow::top_win(m, bet, pool);
                    (format!("{}  {}", t.name.to_uppercase(), chips_text(pool)), format!("PROGRESSIVE JACKPOT · {} at your bet", chips_text(mine)))
                }
                Top::Fixed(x) => (format!("{}  {}×", t.name.to_uppercase(), chips_text(x)), "TOP PRIZE".to_string()),
            };
            let g = p.layout_no_wrap(line, theme::display_font(st.font, 20.0), col::GOLD2);
            let w = g.size().x + 40.0;
            let pill = Rect::from_center_size(pos2(area.center().x, area.top() + 30.0), vec2(w.max(240.0), 48.0));
            p.rect_filled(pill, theme::r(24), Color32::from_black_alpha(190));
            let glow = 0.5 + 0.3 * (time * 2.2).sin();
            p.rect_stroke(pill, theme::r(24), Stroke::new(1.5f32, th.accent.gamma_multiply(glow)), StrokeKind::Inside);
            p.galley(pos2(pill.center().x - g.size().x / 2.0, pill.top() + 5.0), g, col::GOLD2);
            p.text(pos2(pill.center().x, pill.bottom() - 10.0), Align2::CENTER_CENTER, sub, theme::bold_font(10.5), col::MUTED);
        }
        // Play Feature and Fairness, over everything.
        let (bet, spin_cost) = (self.poker.casino.row.bet, self.poker.casino.row.bet);
        let chips = super::chips_now(s);
        if let Some(pick) = self.play_feature_menu(ui, kind, area, bet, spin_cost, th.accent, chips) {
            match pick {
                Pick::Arm(f) => self.poker.casino.row.stake.armed = f,
                Pick::Buy(f) => self.row_play(s, f),
            }
        }
        self.fairness_panel(&ctx, s, kind);
        // Spinning by itself.
        let r = &self.poker.casino.row;
        if r.auto.on() && r.next_at.is_some_and(|t| now >= t) && !self.poker.casino.busy() && !r.running(now) {
            self.poker.casino.row.next_at = None;
            let f = self.poker.casino.row.stake.armed.unwrap_or(Feature::Spin);
            self.row_play(s, f);
        }
        if self.poker.casino.row.running(now) || !self.poker.casino.row.fx.is_empty() {
            ctx.request_repaint();
        }
    }

    /// The cabinet round the reels, in the machine's own make, and its name over them.
    fn row_cabinet(&self, p: &egui::Painter, th: &Theme, st: &Style, lay: &Layout, time: f32) {
        let cab = lay.cabinet;
        let fr = lay.reels.expand(lay.cell * 0.09);
        let a = th.accent;
        let b = st.second;
        match st.frame {
            Frame::Holo => {
                // Light, not wood: a pane of glass, scan lines, corner brackets.
                p.rect_filled(cab, theme::r(18), Color32::from_rgba_unmultiplied(0x08, 0x10, 0x30, 150));
                for i in 0..((cab.height() / 4.0) as i32) {
                    let y = cab.top() + i as f32 * 4.0 + (time * 20.0) % 4.0;
                    p.line_segment([pos2(cab.left() + 4.0, y), pos2(cab.right() - 4.0, y)], Stroke::new(1.0f32, Color32::from_rgba_unmultiplied(0x4a, 0xe8, 0xff, 10)));
                }
                p.rect_stroke(cab, theme::r(18), Stroke::new(1.5f32, b.gamma_multiply(0.7)), StrokeKind::Inside);
                let l = 34.0;
                for (c, dx, dy) in [(cab.left_top(), 1.0, 1.0), (cab.right_top(), -1.0, 1.0), (cab.left_bottom(), 1.0, -1.0), (cab.right_bottom(), -1.0, -1.0)] {
                    p.line_segment([c, c + vec2(l * dx, 0.0)], Stroke::new(3.5f32, b));
                    p.line_segment([c, c + vec2(0.0, l * dy)], Stroke::new(3.5f32, b));
                }
            }
            Frame::Timber => {
                p.rect_filled(cab.translate(vec2(0.0, 10.0)), theme::r(10), Color32::from_black_alpha(160));
                vgrad_round(p, cab, 10.0, Color32::from_rgb(0x6a, 0x42, 0x20), Color32::from_rgb(0x34, 0x1e, 0x0c));
                // Planks.
                for i in 1..6 {
                    let y = cab.top() + cab.height() * i as f32 / 6.0;
                    p.line_segment([pos2(cab.left() + 6.0, y), pos2(cab.right() - 6.0, y)], Stroke::new(1.5f32, Color32::from_black_alpha(90)));
                }
                // Iron straps with rivets.
                for x in [cab.left() + 22.0, cab.right() - 22.0] {
                    p.rect_filled(Rect::from_center_size(pos2(x, cab.center().y), vec2(14.0, cab.height())), 0, Color32::from_rgb(0x3a, 0x3a, 0x40));
                    for k in 0..8 {
                        p.circle_filled(pos2(x, cab.top() + 12.0 + k as f32 * (cab.height() - 24.0) / 7.0), 3.0, Color32::from_rgb(0x9a, 0x9a, 0xa4));
                    }
                }
                p.rect_stroke(cab, theme::r(10), Stroke::new(3.0f32, Color32::from_rgb(0x22, 0x12, 0x06)), StrokeKind::Inside);
            }
            Frame::Stone => {
                p.rect_filled(cab.translate(vec2(0.0, 12.0)), theme::r(6), Color32::from_black_alpha(170));
                vgrad_round(p, cab, 6.0, Color32::from_rgb(0xb0, 0x8a, 0x52), Color32::from_rgb(0x6a, 0x4e, 0x28));
                // Carved bands of glyph-like marks, and gold edges.
                for (y0, y1) in [(cab.top() + 8.0, cab.top() + 22.0), (cab.bottom() - 22.0, cab.bottom() - 8.0)] {
                    let band = Rect::from_min_max(pos2(cab.left() + 8.0, y0), pos2(cab.right() - 8.0, y1));
                    p.rect_filled(band, 0, Color32::from_rgb(0x8a, 0x66, 0x30));
                    let n = (band.width() / 18.0) as i32;
                    for i in 0..n {
                        let x = band.left() + 9.0 + i as f32 * 18.0;
                        let k = i % 4;
                        let c = band.center().y;
                        match k {
                            0 => {
                                p.circle_stroke(pos2(x, c), 4.0, Stroke::new(1.5f32, Color32::from_rgb(0x3a, 0x26, 0x0c)));
                            }
                            1 => {
                                p.line_segment([pos2(x, c - 5.0), pos2(x, c + 5.0)], Stroke::new(2.0f32, Color32::from_rgb(0x3a, 0x26, 0x0c)));
                            }
                            2 => {
                                p.add(Shape::convex_polygon(vec![pos2(x, c - 5.0), pos2(x + 5.0, c + 4.0), pos2(x - 5.0, c + 4.0)], Color32::TRANSPARENT, Stroke::new(1.5f32, Color32::from_rgb(0x3a, 0x26, 0x0c))));
                            }
                            _ => {
                                p.line_segment([pos2(x - 5.0, c), pos2(x + 5.0, c)], Stroke::new(2.0f32, Color32::from_rgb(0x3a, 0x26, 0x0c)));
                            }
                        }
                    }
                }
                p.rect_stroke(cab, theme::r(6), Stroke::new(3.0f32, col::GOLD), StrokeKind::Inside);
                p.rect_stroke(cab.shrink(5.0), theme::r(4), Stroke::new(1.0f32, b.gamma_multiply(0.6)), StrokeKind::Inside);
            }
            Frame::Arcane => {
                p.rect_filled(cab.translate(vec2(0.0, 12.0)), theme::r(40), Color32::from_black_alpha(170));
                vgrad_round(p, cab, 40.0, Color32::from_rgb(0x2a, 0x1a, 0x30), Color32::from_rgb(0x10, 0x08, 0x14));
                p.rect_stroke(cab, theme::r(40), Stroke::new(3.0f32, Color32::from_rgb(0x5a, 0x40, 0x2a)), StrokeKind::Inside);
                // Runes round the arch, glowing one after another.
                let n = 26;
                for i in 0..n {
                    let t = i as f32 / n as f32;
                    let x = cab.left() + 18.0 + t * (cab.width() - 36.0);
                    let glow = 0.25 + 0.75 * (((time * 1.6 - t * 6.0).sin() + 1.0) / 2.0).powi(3);
                    for y in [cab.top() + 14.0, cab.bottom() - 14.0] {
                        let c = pos2(x, y);
                        let rune = ["ᚠ", "ᚢ", "ᚦ", "ᚨ", "ᚱ", "ᚲ", "ᚷ", "ᚹ"][i % 8];
                        p.text(c, Align2::CENTER_CENTER, rune, theme::font(13.0), a.gamma_multiply(glow));
                    }
                }
            }
            Frame::Jade => {
                p.rect_filled(cab.translate(vec2(0.0, 12.0)), theme::r(16), Color32::from_black_alpha(170));
                vgrad_round(p, cab, 16.0, Color32::from_rgb(0x0e, 0x5a, 0x3a), Color32::from_rgb(0x04, 0x26, 0x16));
                // Scales along the edges.
                let mut x = cab.left() + 12.0;
                while x < cab.right() - 12.0 {
                    for y in [cab.top() + 10.0, cab.bottom() - 10.0] {
                        p.circle_stroke(pos2(x, y), 7.0, Stroke::new(1.5f32, col::GOLD.gamma_multiply(0.7)));
                    }
                    x += 12.0;
                }
                p.rect_stroke(cab, theme::r(16), Stroke::new(3.5f32, col::GOLD), StrokeKind::Inside);
                p.rect_stroke(cab.shrink(6.0), theme::r(12), Stroke::new(1.5f32, b.gamma_multiply(0.8)), StrokeKind::Inside);
            }
            Frame::Chalk => {
                p.rect_filled(cab.translate(vec2(0.0, 10.0)), theme::r(8), Color32::from_black_alpha(160));
                p.rect_filled(cab, theme::r(8), Color32::from_rgb(0x6a, 0x46, 0x22));
                let board = cab.shrink(14.0);
                p.rect_filled(board, theme::r(4), Color32::from_rgb(0x22, 0x2a, 0x24));
                // Chalk doodles in the corners: steam curls and a heart.
                for (c, k) in [(board.left_top() + vec2(30.0, 30.0), 1.0f32), (board.right_top() + vec2(-30.0, 30.0), -1.0)] {
                    for j in 0..3 {
                        let x = c.x + k * j as f32 * 10.0;
                        let pts: Vec<Pos2> = (0..12).map(|t| pos2(x + (t as f32 * 0.8 + time).sin() * 3.0, c.y - t as f32 * 2.0)).collect();
                        p.add(Shape::line(pts, Stroke::new(1.2f32, Color32::from_white_alpha(70))));
                    }
                }
                p.rect_stroke(cab, theme::r(8), Stroke::new(2.0f32, Color32::from_rgb(0x3a, 0x22, 0x0c)), StrokeKind::Inside);
            }
            // The rooms with a place for the reels: trim round it.
            Frame::Brass => {
                p.rect_filled(fr.expand(10.0), theme::r(24), Color32::from_rgb(0x6a, 0x4a, 0x1e));
                p.rect_stroke(fr.expand(10.0), theme::r(24), Stroke::new(3.0f32, Color32::from_rgb(0xd8, 0xa8, 0x5a)), StrokeKind::Inside);
                let e = fr.expand(4.0);
                let n = 18;
                for i in 0..n {
                    let t = i as f32 / (n - 1) as f32;
                    for (x, y) in [(e.left() + t * e.width(), e.top()), (e.left() + t * e.width(), e.bottom())] {
                        p.circle_filled(pos2(x, y), 3.2, Color32::from_rgb(0xe8, 0xc8, 0x8a));
                    }
                }
            }
            Frame::Neon => {
                let flick = if ((time * 7.3).sin() * 43.0).fract() > 0.93 { 0.4 } else { 1.0 };
                for (k, c, w) in [(14.0, a, 6.0), (6.0, b, 3.0)] {
                    let e = fr.expand(k);
                    p.rect_stroke(e, theme::r(14), Stroke::new(w + 8.0, c.gamma_multiply(0.15 * flick)), StrokeKind::Middle);
                    p.rect_stroke(e, theme::r(14), Stroke::new(w, c.gamma_multiply(flick)), StrokeKind::Middle);
                }
            }
            Frame::Ice => {
                p.rect_filled(fr.expand(12.0), theme::r(8), Color32::from_rgb(0x4a, 0x30, 0x1a));
                p.rect_stroke(fr.expand(12.0), theme::r(8), Stroke::new(3.0f32, Color32::from_rgb(0xd8, 0xf0, 0xff)), StrokeKind::Inside);
                // Icicles along the top.
                let mut x = fr.left() - 6.0;
                let mut i = 0;
                while x < fr.right() + 6.0 {
                    let len = 8.0 + ((i * 37) % 11) as f32 * 1.6;
                    p.add(Shape::convex_polygon(vec![pos2(x - 4.0, fr.top() - 12.0), pos2(x + 4.0, fr.top() - 12.0), pos2(x, fr.top() - 12.0 + len)], Color32::from_rgba_unmultiplied(0xe0, 0xf4, 0xff, 220), Stroke::NONE));
                    x += 14.0;
                    i += 1;
                }
            }
            Frame::Marquee => {
                p.rect_filled(fr.expand(12.0), theme::r(10), Color32::from_rgb(0x5a, 0x06, 0x10));
                p.rect_stroke(fr.expand(12.0), theme::r(10), Stroke::new(3.0f32, col::GOLD), StrokeKind::Inside);
                // Bulbs chasing round.
                let e = fr.expand(6.0);
                let per = 26;
                for side in 0..4 {
                    for i in 0..per {
                        let t = i as f32 / per as f32;
                        let q = match side {
                            0 => pos2(e.left() + t * e.width(), e.top()),
                            1 => pos2(e.right(), e.top() + t * e.height()),
                            2 => pos2(e.right() - t * e.width(), e.bottom()),
                            _ => pos2(e.left(), e.bottom() - t * e.height()),
                        };
                        let on = ((time * 8.0) as i32 + i + side * per) % 3 == 0;
                        p.circle_filled(q, 3.0, if on { Color32::from_rgb(0xff, 0xf0, 0xc8) } else { Color32::from_rgb(0x8a, 0x6a, 0x30) });
                    }
                }
            }
            Frame::Candy => {
                let e = fr.expand(12.0);
                p.rect_filled(e, theme::r(20), Color32::WHITE);
                let clip = p.with_clip_rect(e);
                let mut x = e.left() - e.height();
                while x < e.right() {
                    clip.add(Shape::convex_polygon(vec![pos2(x, e.bottom()), pos2(x + 10.0, e.bottom()), pos2(x + 10.0 + e.height(), e.top()), pos2(x + e.height(), e.top())], Color32::from_rgb(0xff, 0x6a, 0xb4), Stroke::NONE));
                    x += 22.0;
                }
            }
            Frame::Cantina => {
                p.rect_filled(fr.expand(12.0), theme::r(8), Color32::from_rgb(0x4a, 0x22, 0x0e));
                p.rect_stroke(fr.expand(12.0), theme::r(8), Stroke::new(2.5f32, Color32::from_rgb(0x2a, 0x10, 0x04)), StrokeKind::Inside);
                // A string of chili lights along the top, swinging a little.
                let n = 16;
                for i in 0..n {
                    let t = i as f32 / (n - 1) as f32;
                    let x = fr.left() + t * fr.width();
                    let sag = (t * std::f32::consts::PI).sin() * 10.0;
                    let y = fr.top() - 22.0 + sag;
                    let on = ((time * 3.0) as i32 + i) % 4 != 0;
                    let c = if i % 2 == 0 { b } else { col::GOLD };
                    let pos = pos2(x, y + 7.0);
                    p.add(egui::epaint::EllipseShape::filled(pos, vec2(3.5, 8.0), if on { c } else { c.gamma_multiply(0.35) }));
                    if on {
                        radial(p, pos, 14.0, c.gamma_multiply(0.25), Color32::TRANSPARENT);
                    }
                }
            }
        }
        // The machine's name, in its own lettering.
        let size = (lay.cell * 0.3).clamp(24.0, 46.0) * if st.font == "monoton" { 0.82 } else { 1.0 };
        let font = theme::display_font(st.font, size);
        let at = lay.marquee;
        let g = p.layout_no_wrap(th.name.to_string(), font.clone(), Color32::WHITE);
        let plate = Rect::from_center_size(at, g.size() + vec2(40.0, 14.0));
        if st.window.is_some() {
            p.rect_filled(plate, theme::r(16), Color32::from_black_alpha(150));
        }
        glow_text(p, at + vec2(0.0, 3.0), Align2::CENTER_CENTER, th.name, font.clone(), Color32::from_black_alpha(180), Color32::TRANSPARENT);
        glow_text(p, at, Align2::CENTER_CENTER, th.name, font, mix(col::GOLD2, a, 0.25), a.gamma_multiply(0.55 + 0.15 * (time * 2.0).sin()));
    }

    /// The reels: each cell's symbol, wins lit as they're counted.
    #[allow(clippy::too_many_arguments)]
    fn row_reels(&mut self, p: &egui::Painter, ctx: &egui::Context, th: &Theme, st: &Style, lay: &Layout, now: Instant, time: f32) {
        let r = &self.poker.casino.row;
        let sheet = format!("reels-{}-symbols", th.id);
        let cell = lay.cell;
        let px = (cell * 3.0) as u32;
        let frame = lay.reels.expand(cell * 0.06);
        p.rect_filled(frame, theme::r(12), Color32::from_black_alpha(225));
        let wild_spin = r.showing.as_ref().is_some_and(|rv| matches!(&rv.outcome, Outcome::Row(o) if o.feature == Feature::Wild));
        let (lit, msg) = self.row_lit(now);
        let r = &self.poker.casino.row;
        for i in 0..REELS {
            let rr_ = Rect::from_min_size(pos2(lay.reels.left() + i as f32 * (cell + lay.gap), lay.reels.top()), vec2(cell, cell * 3.0));
            vgrad(p, rr_, st.reel_top, st.reel_bottom);
            let clip = p.with_clip_rect(rr_);
            let reel = r.reels[i];
            let pos = reel.pos(now);
            let base = pos.floor() as i32;
            let frac = pos - pos.floor();
            let blurred = reel.blurred(now);
            // A Wild spin's middle reel: alight while it spins.
            if wild_spin && i == reelrow::WILD_REEL {
                radial(&clip, rr_.center(), cell * 1.6, th.accent.gamma_multiply(0.35 + 0.15 * (time * 5.0).sin()), Color32::TRANSPARENT);
            }
            for row in -1..=3 {
                let idx = (base + row).rem_euclid(STRIP as i32) as usize;
                let sym = r.strips[i][idx];
                let y = rr_.top() + (row as f32 - frac) * cell;
                let sr = Rect::from_min_size(pos2(rr_.left(), y), vec2(cell, cell)).shrink(cell * 0.07);
                if sym == rr::BLANK {
                    clip.circle_filled(sr.center(), cell * 0.03, th.accent.gamma_multiply(0.18));
                    continue;
                }
                let on = lit.iter().any(|(rl, rw)| *rl == i && row == *rw as i32) && !blurred;
                if on {
                    let pulse = 0.5 + 0.5 * (time * 6.0).sin();
                    radial(&clip, sr.center(), cell * 0.6, th.accent.gamma_multiply(0.4 + 0.2 * pulse), Color32::TRANSPARENT);
                }
                let grow = if on { 1.0 + 0.06 * (time * 6.0).sin().abs() } else { 1.0 };
                let sr = Rect::from_center_size(sr.center(), sr.size() * grow);
                if blurred {
                    pics::cell_inset(&clip, ctx, &sheet, sym as usize, sr.translate(vec2(0.0, -cell * 0.14)), px, Color32::from_white_alpha(70), 0.04);
                    pics::cell_inset(&clip, ctx, &sheet, sym as usize, sr, px, Color32::from_white_alpha(190), 0.04);
                } else {
                    let dim = if lit.is_empty() || on { Color32::WHITE } else { Color32::from_gray(140) };
                    pics::cell_inset(&clip, ctx, &sheet, sym as usize, sr, px, dim, 0.04);
                }
                // Scatters landed: they glow and turn, the bonus close.
                if sym == rr::SCATTER && !blurred && reel.landed(now) {
                    clip.circle_stroke(sr.center(), cell * 0.44, Stroke::new(2.0f32, st.second.gamma_multiply(0.5 + 0.5 * (time * 4.0).sin().abs())));
                }
            }
            vgrad(&clip, Rect::from_min_size(rr_.min, vec2(cell, cell * 0.3)), Color32::from_black_alpha(150), Color32::TRANSPARENT);
            vgrad(&clip, Rect::from_min_max(pos2(rr_.left(), rr_.bottom() - cell * 0.3), rr_.max), Color32::TRANSPARENT, Color32::from_black_alpha(150));
        }
        if let Some(m) = msg {
            let g = p.layout_no_wrap(m, theme::display_font(st.font, (cell * 0.16).clamp(15.0, 24.0)), col::GOLD2);
            let mr = Rect::from_center_size(pos2(lay.reels.center().x, lay.reels.bottom() - cell * 0.22), g.size() + vec2(30.0, 12.0));
            p.rect_filled(mr, theme::r(17), Color32::from_black_alpha(220));
            p.rect_stroke(mr, theme::r(17), Stroke::new(1.0f32, th.accent.gamma_multiply(0.7)), StrokeKind::Inside);
            p.galley(mr.center() - g.size() / 2.0, g, col::GOLD2);
        }
        // A Chance spin's mark, while it's armed.
        if let Some(f) = self.poker.casino.row.stake.armed {
            let (name, _) = SlotKind::Row(self.poker.casino.row.machine).describe(f);
            let t = format!("{} ON", name.to_uppercase());
            let g = p.layout_no_wrap(t, theme::bold_font(12.0), Color32::BLACK);
            let tag = Rect::from_min_size(pos2(lay.reels.right() - g.size().x - 22.0, lay.reels.top() - g.size().y - 14.0), g.size() + vec2(16.0, 6.0));
            p.rect_filled(tag, theme::r(10), th.accent);
            p.galley(tag.min + vec2(8.0, 3.0), g, Color32::BLACK);
        }
    }

    /// The cells lit (reel, row) and the line about the win shown now.
    fn row_lit(&self, now: Instant) -> (Vec<(usize, usize)>, Option<String>) {
        let r = &self.poker.casino.row;
        let (Some(round), Some(since)) = (&r.showing, r.counted) else { return (Vec::new(), None) };
        let Outcome::Row(o) = &round.outcome else { return (Vec::new(), None) };
        let Some(grid) = o.grid else { return (Vec::new(), None) };
        if o.wins.is_empty() {
            return (Vec::new(), None);
        }
        let k = now.duration_since(since).as_secs_f32();
        let th = theme(o.machine);
        let which = if k < 1.2 { None } else { Some(((k - 1.2) / 1.4) as usize % o.wins.len()) };
        let mut lit = Vec::new();
        for (i, w) in o.wins.iter().enumerate() {
            if which.is_some_and(|x| x != i) {
                continue;
            }
            for (reel, col_) in grid.iter().enumerate().take(w.count as usize) {
                for (row, s) in col_.iter().enumerate() {
                    if *s == w.symbol || *s == rr::WILD {
                        lit.push((reel, row));
                    }
                }
            }
        }
        let msg = match which {
            Some(i) => {
                let w = &o.wins[i];
                Some(format!("{} × {} · {} way{} · {}", th.symbols[w.symbol as usize], w.count, w.ways, if w.ways == 1 { "" } else { "s" }, chips_text(w.pay)))
            }
            None => Some(format!("WIN {}", chips_text(o.wins.iter().map(|w| w.pay).sum()))),
        };
        (lit, msg)
    }

    /// Once the reels are still: the wins counted (and shown off), then the mini game, or the
    /// round's end.
    fn row_after_stop(&mut self, s: &ServerView, now: Instant) {
        let r = &self.poker.casino.row;
        let Some(end) = r.stopped_at else { return };
        if now < end || r.counted.is_some() || r.all_in.is_some() {
            return;
        }
        let Some(round) = r.showing.clone() else { return };
        let Outcome::Row(o) = &round.outcome else { return };
        if o.grid.is_none() {
            // (A bonus bought: the game's on already.)
            self.poker.casino.row.counted = Some(now);
            return;
        }
        let th = theme(o.machine);
        let base: u64 = o.wins.iter().map(|w| w.pay).sum();
        let ui = &mut self.poker.casino.row;
        ui.counted = Some(now);
        ui.win = base;
        if base > 0 {
            let big = base >= o.bet * 10;
            self.poker.casino.sound_in(0.0, if big { Cue::WinBig } else { Cue::WinSmall }, 0.0, 0.7);
            self.row_burst(base, o.bet, now);
        }
        if o.bonus.is_none() {
            let v1 = to_v1(o);
            self.poker.casino.row.fun.round(th.humor, &v1, round.nonce);
        }
        match &o.bonus {
            Some(_) => {
                let (bonus, _) = SlotKind::Row(o.machine).bonus_name();
                self.poker.casino.announce(format!("{}!", bonus.to_uppercase()), format!("{} {}", o.scatters, th.scatters), th.accent, (0.5, 0.42), 0.3);
                self.poker.casino.sound_in(0.4, Cue::Jackpot, 0.0, 0.6);
                self.poker.casino.row.mini = Some(MiniPlay::new(round.clone(), now + Duration::from_millis(1_900)));
            }
            None => self.row_finish(s, &round, now),
        }
    }

    /// The round's over on screen: its win on the balance, a celebration if it's big, and the
    /// next spin if it's spinning by itself.
    pub(super) fn row_finish(&mut self, _s: &ServerView, round: &RoundView, now: Instant) {
        let Outcome::Row(o) = &round.outcome else { return };
        self.poker.casino.settle_balance();
        self.poker.casino.celebrate(o.paid, o.wagered(), false, 0.2);
        // The machine's top prize: its own moment, over everything.
        if let Some(b) = o.bonus.as_ref().filter(|b| b.top > 0) {
            let t = &pyr_poker::casino::reelrow::TOPS[o.machine as usize];
            let x = b.top / o.bet.max(1);
            self.poker.casino.announce(format!("{} · {}×", t.name.to_uppercase(), chips_text(x)), format!("You won {} chips", chips_text(b.top)), col::GOLD2, (0.5, 0.4), 0.0);
            self.poker.casino.sound_in(0.0, Cue::Jackpot, 0.0, 1.0);
            self.row_burst(b.top, o.bet, now);
        }
        let big = o.paid >= o.wagered() * 10;
        let ui = &mut self.poker.casino.row;
        if ui.auto.spun(big) {
            let pause = if o.paid > 0 { 1.3 } else { 0.5 };
            ui.next_at = Some(now + Duration::from_secs_f32(pause * if ui.turbo { 0.5 } else { 1.0 }));
        }
    }

    /// A win's burst: how much flies up out of the reels by how big it is.
    pub(super) fn row_burst(&mut self, won: u64, bet: u64, now: Instant) {
        let n = (8.0 + (won as f32 / bet.max(1) as f32).sqrt() * 10.0).min(140.0) as u32;
        let ui = &mut self.poker.casino.row;
        let seed0 = (now.elapsed().as_nanos() as u32) ^ (won as u32).wrapping_mul(2654435761);
        for i in 0..n {
            let h = seed0.wrapping_add(i.wrapping_mul(0x9e37_79b9)).wrapping_mul(0x85eb_ca6b);
            let fx = (h % 1000) as f32 / 1000.0;
            let fy = ((h >> 10) % 1000) as f32 / 1000.0;
            ui.fx.push(Particle { at: pos2(fx, 0.5 + fy * 0.5), vel: vec2(fx - 0.5, -0.6 - fy * 0.8), born: now + Duration::from_millis((i * 9) as u64), life: 1.4 + fy * 1.2, seed: h });
        }
    }

    /// The win's particles, the machine's own: embers, bubbles, stars and so on.
    fn row_fx(&mut self, p: &egui::Painter, st: &Style, th: &Theme, now: Instant) {
        let area = p.clip_rect();
        let ui = &mut self.poker.casino.row;
        ui.fx.retain(|q| now < q.born + Duration::from_secs_f32(q.life));
        for q in &ui.fx {
            if now < q.born {
                continue;
            }
            let t = (now - q.born).as_secs_f32();
            let k = t / q.life;
            let fade = (1.0 - k).clamp(0.0, 1.0);
            let x = area.left() + area.width() * (0.2 + q.at.x * 0.6) + q.vel.x * t * 260.0;
            let gravity = match st.fx {
                Fx::Bubbles | Fx::Wisps | Fx::Steam | Fx::Embers | Fx::Stars | Fx::Sparks => -40.0,
                _ => 260.0,
            };
            let y = area.top() + area.height() * (0.3 + q.at.y * 0.35) + q.vel.y * t * 300.0 + gravity * t * t * 0.5;
            let c = pos2(x, y);
            let s = 3.0 + (q.seed % 7) as f32;
            let spin = t * 6.0 + (q.seed % 100) as f32;
            match st.fx {
                Fx::Embers => {
                    radial(p, c, s * 2.4, Color32::from_rgb(0xff, 0x8a, 0x2a).gamma_multiply(fade), Color32::TRANSPARENT);
                    p.circle_filled(c, s * 0.5, Color32::from_rgb(0xff, 0xe0, 0x8a).gamma_multiply(fade));
                }
                Fx::Bubbles => {
                    p.circle_stroke(c, s * 1.4, Stroke::new(1.5f32, Color32::from_rgb(0xbf, 0xf0, 0xff).gamma_multiply(fade)));
                    p.circle_filled(c + vec2(-s * 0.4, -s * 0.4), s * 0.3, Color32::from_white_alpha((160.0 * fade) as u8));
                }
                Fx::Stars | Fx::Sparks => {
                    let cc = if st.fx == Fx::Stars { Color32::WHITE } else if q.seed % 2 == 0 { th.accent } else { st.second };
                    for a in 0..4 {
                        let ang = spin * 0.2 + a as f32 * std::f32::consts::FRAC_PI_2;
                        p.line_segment([c, c + vec2(ang.cos(), ang.sin()) * s * 1.6], Stroke::new(1.5f32, cc.gamma_multiply(fade)));
                    }
                    p.circle_filled(c, s * 0.4, cc.gamma_multiply(fade));
                }
                Fx::Sprinkles | Fx::Confetti => {
                    let cols = [Color32::from_rgb(0xff, 0x6a, 0xb4), Color32::from_rgb(0x8a, 0xff, 0xd8), Color32::from_rgb(0xff, 0xe0, 0x6a), Color32::from_rgb(0x8a, 0xb4, 0xff), Color32::WHITE];
                    let cc = cols[(q.seed % 5) as usize].gamma_multiply(fade);
                    let d = vec2(spin.cos(), spin.sin()) * s * if st.fx == Fx::Confetti { 1.2 } else { 0.9 };
                    p.line_segment([c - d, c + d], Stroke::new(if st.fx == Fx::Confetti { 4.0f32 } else { 2.5 }, cc));
                }
                Fx::Nuggets | Fx::Coins => {
                    let w = s * 1.2 * spin.cos().abs().max(0.2);
                    p.add(egui::epaint::EllipseShape::filled(c, vec2(w, s * 1.2), col::GOLD.gamma_multiply(fade)));
                    p.add(egui::epaint::EllipseShape::stroke(c, vec2(w, s * 1.2), Stroke::new(1.0f32, Color32::from_rgb(0x8a, 0x5a, 0x10).gamma_multiply(fade))));
                }
                Fx::Sand => {
                    p.circle_filled(c, s * 0.45, Color32::from_rgb(0xe8, 0xc8, 0x8a).gamma_multiply(fade));
                }
                Fx::Snow => {
                    for a in 0..3 {
                        let ang = spin * 0.1 + a as f32 * std::f32::consts::PI / 3.0;
                        let d = vec2(ang.cos(), ang.sin()) * s;
                        p.line_segment([c - d, c + d], Stroke::new(1.3f32, Color32::WHITE.gamma_multiply(fade)));
                    }
                }
                Fx::Wisps => {
                    radial(p, c, s * 3.0, st.second.gamma_multiply(0.6 * fade), Color32::TRANSPARENT);
                    p.circle_filled(c, s * 0.5, th.accent.gamma_multiply(fade));
                }
                Fx::Steam => {
                    radial(p, c, s * 4.0, Color32::from_white_alpha((70.0 * fade) as u8), Color32::TRANSPARENT);
                }
            }
        }
    }

    /// All or nothing: a vault (the machine's colours) with the odds on it, spun open; 2,500×
    /// the bet behind it, or nothing.
    #[allow(clippy::too_many_arguments)]
    fn row_all_in(&mut self, ui: &mut egui::Ui, s: &ServerView, th: &Theme, st: &Style, r: Rect, now: Instant, time: f32) {
        let Some((round, since, done)) = self.poker.casino.row.all_in.clone() else { return };
        let Outcome::Row(o) = &round.outcome else { return };
        let p = ui.painter().clone();
        let t = (now - since).as_secs_f32();
        let spin_t = 3.2;
        let _ = time;
        let landed = super::stake::all_in_dial(&p, r, th.accent, st.font, o.lucky, o.paid, &format!("1 in 50.5 for {}", chips_text(o.bet * reelrow::ALL_IN_X)), t);
        if landed {
            if !done {
                self.poker.casino.row.all_in.as_mut().unwrap().2 = true;
                if o.lucky {
                    self.poker.casino.sound_in(0.0, Cue::Jackpot, 0.0, 0.9);
                    self.row_burst(o.paid, o.bet, now);
                } else {
                    self.poker.casino.sound_in(0.0, Cue::Fold, 0.0, 0.6);
                }
                self.poker.casino.row.win = o.paid;
            }
            if t > spin_t + 2.2 {
                self.poker.casino.row.all_in = None;
                self.row_finish(s, &round, now);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn row_controls(&mut self, ui: &mut egui::Ui, s: &ServerView, th: &Theme, st: &Style, bar: Rect, now: Instant, time: f32) {
        let p = ui.painter().clone();
        let m = self.poker.casino.row.machine;
        let kind = SlotKind::Row(m);
        vgrad_round(&p, bar, 18.0, mix(th.deep, Color32::from_rgb(0x16, 0x10, 0x16), 0.5), Color32::from_rgb(0x08, 0x06, 0x0a));
        p.rect_stroke(bar, theme::r(18), Stroke::new(1.0f32, th.accent.gamma_multiply(0.45)), StrokeKind::Inside);
        let cy = bar.center().y;
        let busy = self.poker.casino.busy() || self.poker.casino.row.running(now);
        let bet = self.poker.casino.row.bet;
        // The bet.
        p.text(pos2(bar.left() + 22.0, bar.top() + 18.0), Align2::LEFT_CENTER, "BET", theme::bold_font(11.5), col::MUTED);
        let minus = Rect::from_center_size(pos2(bar.left() + 36.0, cy + 10.0), vec2(32.0, 32.0));
        let plus = Rect::from_center_size(pos2(bar.left() + 150.0, cy + 10.0), vec2(32.0, 32.0));
        for (r, label, dir) in [(minus, "−", -1i32), (plus, "+", 1)] {
            let resp = ui.interact(r, ui.id().with(("rowbet", label)), Sense::click());
            p.circle_filled(r.center(), 16.0, if resp.hovered() && !busy { th.accent } else { mix(th.accent, Color32::BLACK, 0.5) });
            p.text(r.center(), Align2::CENTER_CENTER, label, theme::bold_font(19.0), Color32::BLACK);
            if resp.clicked() && !busy {
                let i = BETS.iter().position(|x| *x == bet).unwrap_or(2) as i32;
                self.poker.casino.row.bet = BETS[(i + dir).clamp(0, BETS.len() as i32 - 1) as usize];
            }
        }
        let amount = Rect::from_center_size(pos2(bar.left() + 93.0, cy + 10.0), vec2(76.0, 30.0));
        let aresp = ui.interact(amount, ui.id().with("rowbet-pick"), Sense::click()).on_hover_text("Choose the bet");
        p.text(amount.center(), Align2::CENTER_CENTER, chips_text(bet), theme::bold_font(20.0), col::GOLD2);
        if !busy {
            egui::Popup::menu(&aresp).align(egui::RectAlign::TOP_START).show(|ui| {
                crate::menu::style(ui);
                for b in BETS {
                    if ui.button(format!("{} chips", chips_text(b))).clicked() {
                        self.poker.casino.row.bet = b;
                        ui.close();
                    }
                }
            });
        }
        // Win.
        let win = self.poker.casino.row.win;
        p.text(pos2(bar.left() + 188.0, bar.top() + 18.0), Align2::LEFT_CENTER, "WIN", theme::bold_font(11.5), col::MUTED);
        p.text(pos2(bar.left() + 188.0, cy + 10.0), Align2::LEFT_CENTER, if win > 0 { chips_text(win) } else { "–".into() }, theme::bold_font(22.0), if win > 0 { col::WIN } else { col::MUTED });
        // Play Feature, beside the win.
        let pf = Rect::from_min_size(pos2(bar.left() + 290.0, cy - 22.0), vec2(200.0, 44.0));
        self.play_feature_button(ui, kind, pf, th.accent, !busy && !self.poker.casino.row.auto.on());
        // Spin: the machine's own button, between Play Feature and the small buttons.
        let auto = self.poker.casino.row.auto;
        let small_w = 4.0 * 8.0 + [100.0, 66.0, 80.0, 56.0].iter().sum::<f32>();
        let spin_c = pos2(((pf.right() + bar.right() - 16.0 - small_w) / 2.0).max(pf.right() + 60.0), cy);
        let rad = 42.0;
        let spin_r = Rect::from_center_size(spin_c, vec2(rad * 2.4, rad * 2.0));
        let resp = ui.interact(spin_r, ui.id().with("rowspin"), Sense::click());
        let cost = self.poker.casino.row.spin_cost();
        spin_button(&p, st.button, spin_c, rad, th, st, resp.hovered() && !busy, busy && !auto.on(), time);
        let label = if auto.on() { "STOP" } else { "SPIN" };
        p.text(spin_c - vec2(0.0, 7.0), Align2::CENTER_CENTER, label, theme::display_font(st.font, 21.0), Color32::from_rgb(0x1a, 0x08, 0x04));
        let sub = if auto.on() { format!("auto {}", auto.left_text()) } else if cost != bet { chips_text(cost) } else { String::new() };
        if !sub.is_empty() {
            p.text(spin_c + vec2(0.0, 14.0), Align2::CENTER_CENTER, sub, theme::bold_font(11.0), Color32::from_rgb(0x1a, 0x08, 0x04));
        }
        let space = ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) && !ui.ctx().wants_keyboard_input() && self.poker.casino.info.is_none() && !self.poker.casino.row.stake.menu;
        if resp.on_hover_text("Spin (Space)").clicked() || space {
            if auto.on() {
                self.poker.casino.row.auto.stop();
            } else if !busy {
                let f = self.poker.casino.row.stake.armed.unwrap_or(Feature::Spin);
                self.row_play(s, f);
            }
        }
        // Fairness, Auto, Turbo, Info (right to left).
        let mut bx = bar.right() - 16.0;
        let mut place = |w: f32| {
            let r = Rect::from_min_size(pos2(bx - w, cy - 15.0), vec2(w, 30.0));
            bx -= w + 8.0;
            r
        };
        let small = |ui: &mut egui::Ui, r: Rect, id: &str, text: &str, on: bool, tip: &str| -> egui::Response {
            let resp = ui.interact(r, ui.id().with(("rowbtn", id)), Sense::click());
            ui.painter().rect_filled(r, theme::r(15), if on { th.accent } else if resp.hovered() { Color32::from_white_alpha(40) } else { Color32::from_white_alpha(18) });
            ui.painter().text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(12.0), if on { Color32::BLACK } else { col::TEXT });
            resp.on_hover_text(tip)
        };
        let fr = place(100.0);
        self.fairness_button(ui, kind, fr);
        let ar = place(80.0);
        let auto_label = if auto.on() { format!("Auto {}", auto.left_text()) } else { "Auto ⌄".into() };
        let resp = small(ui, ar, "auto", &auto_label, auto.on(), if auto.on() { "Stop spinning by itself" } else { "Spin by itself: 10 to 1,000 times, or until you stop it" });
        if auto.on() {
            if resp.clicked() {
                self.poker.casino.row.auto.stop();
            }
        } else if let Some(n) = super::auto::menu(&resp, &mut self.poker.casino.row.auto) {
            self.poker.casino.row.auto.left = n;
            if !busy {
                self.poker.casino.row.next_at = Some(Instant::now());
            }
        }
        let tr = place(66.0);
        if small(ui, tr, "turbo", "Turbo", self.poker.casino.row.turbo, "Faster spins").clicked() {
            self.poker.casino.row.turbo = !self.poker.casino.row.turbo;
        }
        let ir = place(56.0);
        if small(ui, ir, "info", "Info", self.poker.casino.info.is_some(), "What it pays, its statistics and its story").clicked() {
            self.info_toggle();
        }
        if super::chips_now(s) < cost {
            p.text(pos2(bar.center().x, bar.bottom() - 8.0), Align2::CENTER_CENTER, "Not enough chips for this bet: lower it, or take the daily chips in the lobby.", theme::font(11.0), col::LOSE);
        }
    }
}

/// The spin button, in the machine's own shape.
#[allow(clippy::too_many_arguments)]
fn spin_button(p: &egui::Painter, shape: Button, c: Pos2, r: f32, th: &Theme, st: &Style, hover: bool, waiting: bool, time: f32) {
    let face = if hover { mix(th.accent, Color32::WHITE, 0.2) } else { mix(th.accent, col::GOLD, 0.45) };
    let rim = col::GOLD2;
    let shadow = Color32::from_black_alpha(150);
    let poly = |pts: Vec<Pos2>, fill: Color32, stroke: Stroke| {
        p.add(Shape::convex_polygon(pts, fill, stroke));
    };
    match shape {
        Button::Round => {
            p.circle_filled(c + vec2(0.0, 4.0), r, shadow);
            radial(p, c, r, face, mix(th.mid, Color32::BLACK, 0.2));
            p.circle_stroke(c, r, Stroke::new(3.0f32, rim));
            for i in 0..16 {
                let a = i as f32 / 16.0 * std::f32::consts::TAU;
                let on = ((time * 6.0) as usize + i) % 4 == 0;
                p.circle_filled(c + vec2(a.cos(), a.sin()) * (r + 7.0), 2.6, if on { st.second } else { st.second.gamma_multiply(0.3) });
            }
        }
        Button::Hex => {
            let pts: Vec<Pos2> = (0..6).map(|i| c + vec2((i as f32 * std::f32::consts::FRAC_PI_3).cos(), (i as f32 * std::f32::consts::FRAC_PI_3).sin()) * r * 1.08).collect();
            poly(pts.iter().map(|q| *q + vec2(0.0, 4.0)).collect(), shadow, Stroke::NONE);
            poly(pts.clone(), face, Stroke::new(2.5f32, st.second));
            let inner: Vec<Pos2> = pts.iter().map(|q| c + (*q - c) * 1.18).collect();
            for i in 0..6 {
                let on = ((time * 5.0) as usize + i) % 3 == 0;
                p.line_segment([inner[i], inner[(i + 1) % 6]], Stroke::new(2.0f32, if on { st.second } else { st.second.gamma_multiply(0.25) }));
            }
        }
        Button::Diamond => {
            let pts = vec![c + vec2(0.0, -r * 1.15), c + vec2(r * 1.2, 0.0), c + vec2(0.0, r * 1.15), c + vec2(-r * 1.2, 0.0)];
            poly(pts.iter().map(|q| *q + vec2(0.0, 4.0)).collect(), shadow, Stroke::NONE);
            poly(pts, face, Stroke::new(3.0f32, rim));
            let facet = vec![c + vec2(0.0, -r * 1.15), c + vec2(r * 0.5, -r * 0.2), c + vec2(-r * 0.5, -r * 0.2)];
            poly(facet, Color32::from_white_alpha(50), Stroke::NONE);
        }
        Button::Pill => {
            let pr = Rect::from_center_size(c, vec2(r * 2.6, r * 1.5));
            p.rect_filled(pr.translate(vec2(0.0, 4.0)), theme::r((r * 0.75) as u8), shadow);
            vgrad_round(p, pr, r * 0.75, mix(face, Color32::WHITE, 0.15), face);
            p.rect_stroke(pr, theme::r((r * 0.75) as u8), Stroke::new(3.0f32, st.second), StrokeKind::Inside);
            p.rect_filled(Rect::from_min_size(pr.min + vec2(r * 0.4, 5.0), vec2(pr.width() - r * 0.8, 5.0)), theme::r(3), Color32::from_white_alpha(90));
        }
        Button::Shield => {
            let pts = vec![c + vec2(-r, -r * 0.95), c + vec2(r, -r * 0.95), c + vec2(r * 0.95, r * 0.15), c + vec2(0.0, r * 1.15), c + vec2(-r * 0.95, r * 0.15)];
            poly(pts.iter().map(|q| *q + vec2(0.0, 4.0)).collect(), shadow, Stroke::NONE);
            poly(pts, face, Stroke::new(3.0f32, rim));
            p.circle_stroke(c, r * 0.72, Stroke::new(1.5f32, st.second.gamma_multiply(0.6)));
        }
        Button::Star => {
            let mut pts = Vec::new();
            for i in 0..10 {
                let a = i as f32 / 10.0 * std::f32::consts::TAU - std::f32::consts::FRAC_PI_2;
                let rr_ = if i % 2 == 0 { r * 1.25 } else { r * 0.78 };
                pts.push(c + vec2(a.cos(), a.sin()) * rr_);
            }
            // (A star isn't convex: the middle as a disc, the five points as triangles.)
            p.circle_filled(c + vec2(0.0, 4.0), r * 0.8, shadow);
            for i in 0..5 {
                let tip = pts[i * 2];
                let l = pts[(i * 2 + 9) % 10];
                let rgt = pts[i * 2 + 1];
                poly(vec![tip, rgt, c, l], face, Stroke::NONE);
            }
            p.circle_filled(c, r * 0.8, face);
            let mut outline = pts.clone();
            outline.push(pts[0]);
            p.add(Shape::line(outline, Stroke::new(2.5f32, rim)));
        }
        Button::Wheel => {
            // A ship's wheel: spokes and handles, turning slowly.
            let ang = time * 0.6;
            for i in 0..8 {
                let a = ang + i as f32 / 8.0 * std::f32::consts::TAU;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([c, c + d * (r + 12.0)], Stroke::new(5.0f32, Color32::from_rgb(0x8a, 0x5a, 0x22)));
                p.circle_filled(c + d * (r + 13.0), 5.0, Color32::from_rgb(0xb0, 0x7a, 0x32));
            }
            p.circle_stroke(c, r, Stroke::new(8.0f32, Color32::from_rgb(0x8a, 0x5a, 0x22)));
            radial(p, c, r * 0.8, face, mix(th.mid, Color32::BLACK, 0.3));
            p.circle_stroke(c, r * 0.8, Stroke::new(2.5f32, rim));
        }
    }
    if waiting {
        let a = time * 6.0;
        p.circle_stroke(c, r * 0.95, Stroke::new(1.0f32, Color32::from_white_alpha(40)));
        p.line_segment([c + vec2(a.cos(), a.sin()) * r * 0.7, c + vec2(a.cos(), a.sin()) * r * 0.95], Stroke::new(3.0f32, Color32::WHITE));
    }
}

/// The round as the funny machines' hosts take it in (they were made for the reels' rounds of
/// 0.12.0).
fn to_v1(o: &reelrow::Outcome) -> rr::Outcome {
    rr::Outcome { machine: o.machine, bet: o.bet, bought: false, grid: o.grid, wins: o.wins.clone(), scatters: o.scatters, bonus: None, paid: o.paid }
}
