//! Reel Row's machines at the app: twelve themed cabinets (their art from Meshy), five reels of
//! three, 243 ways, wins lit up as they're counted, and the bonus each machine plays:
//! pick-and-dodge on a board of fifteen tiles, or a prize wheel. Four of them are funny, each
//! its own way (`humor`). The rounds are the house's (`pyr_poker::casino::reels`), checked by
//! this Pyr like every other.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::casino::reels::{self as rr, Bonus, Pick, BETS, REELS, ROWS};
use pyr_poker::casino::{Bet, Outcome};
use pyr_poker::net::RoundView;

use super::humor::Fun;
use super::reel_themes::{Theme, theme};
use super::spin::{Reel, SPEED};
use super::{col, ease_out, glow_text, pics, radial, vgrad, vgrad_round};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

/// Filler under the reels while they spin: this many symbols round each strip.
const STRIP: usize = 24;

/// The bonus as it's played on screen.
struct BonusPlay {
    round: RoundView,
    /// Pick-and-dodge: the tile opened for each pick so far, in order.
    opened: Vec<usize>,
    /// The wheel: when it set off (and the inner one, on an Upgrade).
    wheel_at: Option<Instant>,
    inner_at: Option<Instant>,
    /// When it was over on screen, and whether its win has gone on the balance.
    over_at: Option<Instant>,
    collected: bool,
    started: Instant,
}

pub struct ReelsUi {
    pub machine: u8,
    pub bet: u64,
    reels: [Reel; REELS],
    strips: [[u8; STRIP]; REELS],
    /// The round on the reels, when they all stop, and whether its wins have been counted.
    showing: Option<RoundView>,
    stopped_at: Option<Instant>,
    counted: Option<Instant>,
    /// Spinning, waiting for the house.
    waiting: Option<Instant>,
    bonus: Option<BonusPlay>,
    /// The round's win (base and bonus), as counted so far.
    pub win: u64,
    pub auto: super::auto::Auto,
    pub turbo: bool,
    next_at: Option<Instant>,
    pub fun: Fun,
}

impl Default for ReelsUi {
    fn default() -> Self {
        ReelsUi {
            machine: 0,
            bet: 100,
            reels: std::array::from_fn(|r| Reel::still((r * 5) as f32)),
            strips: std::array::from_fn(|r| filler(r as u64 * 7_919 + 3)),
            showing: None,
            stopped_at: None,
            counted: None,
            waiting: None,
            bonus: None,
            win: 0,
            auto: Default::default(),
            turbo: false,
            next_at: None,
            fun: Fun::default(),
        }
    }
}

/// Symbols to fill a strip with (by the machine's own odds, roughly; never the bonus hazard,
/// which only the bonus shows).
fn filler(seed: u64) -> [u8; STRIP] {
    let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    std::array::from_fn(|_| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let r = (x >> 11) % 1000;
        // (Pay symbols 0 to 5 by their weights, a few wilds and scatters, blanks.)
        match r {
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

impl ReelsUi {
    /// Spinning, landing, counting, or in the bonus.
    fn running(&self, now: Instant) -> bool {
        self.waiting.is_some() || self.stopped_at.is_some_and(|e| now < e) || self.bonus.as_ref().is_some_and(|b| !b.collected)
    }

    fn cost(&self, buy: bool) -> u64 {
        if buy { self.bet * rr::BUY_X } else { self.bet }
    }
}

impl PyrApp {
    /// Opens a machine.
    pub fn reels_open(&mut self, machine: u8) {
        let ui = &mut self.poker.casino.reels;
        if ui.machine != machine {
            ui.machine = machine;
            ui.showing = None;
            ui.bonus = None;
            ui.auto.stop();
            ui.counted = None;
            ui.win = 0;
        }
        self.poker.casino.game = Some(pyr_poker::casino::Game::Reels);
    }

    /// A round came back: the reels land on it (or a bought bonus starts).
    pub(super) fn reels_round(&mut self, round: &RoundView) {
        let Outcome::Reels(o) = &round.outcome else { return };
        let now = Instant::now();
        let ui = &mut self.poker.casino.reels;
        ui.waiting = None;
        ui.win = 0;
        ui.counted = None;
        ui.showing = Some(round.clone());
        if o.bought {
            ui.stopped_at = Some(now);
            ui.bonus = Some(BonusPlay { round: round.clone(), opened: Vec::new(), wheel_at: None, inner_at: None, over_at: None, collected: false, started: now });
            self.poker.casino.sound_in(0.0, Cue::Jackpot, 0.0, 0.5);
            return;
        }
        let Some(grid) = o.grid else { return };
        // Each strip: the round's three symbols at its top, filler after them.
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
        let min = if ui.turbo { 0.25 } else { 0.7 };
        let lens = [STRIP; REELS];
        let ends = super::spin::land(&mut ui.reels, &lens, &[0; REELS], now + Duration::from_secs_f32(min), ui.turbo);
        let last = ends.iter().max().copied().unwrap_or(now) + Duration::from_secs_f32(super::spin::land_secs(ui.turbo) * 0.45);
        ui.stopped_at = Some(last);
        for (i, e) in ends.iter().enumerate() {
            self.poker.casino.sound_in(e.saturating_duration_since(now).as_secs_f32(), Cue::ReelStop, (i as f32 - 2.0) * 0.3, 0.7);
        }
    }

    fn reels_spin(&mut self, s: &ServerView, buy: bool) {
        let Some(commit) = s.poker.casino.commit else { return };
        let now = Instant::now();
        let ui = &self.poker.casino.reels;
        if ui.cost(buy) > super::chips_now(s) {
            self.poker.casino.reels.auto.stop();
            return;
        }
        if self.poker.casino.busy() || ui.running(now) {
            return;
        }
        let (machine, bet) = (ui.machine, ui.bet);
        if !buy {
            self.poker.casino.sound_in(0.0, Cue::ReelSpin, 0.0, 0.6);
        }
        let ui = &mut self.poker.casino.reels;
        if !buy {
            let speed = SPEED * ui.fun.reel_speed(theme(machine).humor);
            for r in &mut ui.reels {
                r.start(now, speed);
            }
        }
        ui.waiting = Some(now);
        ui.showing = None;
        ui.bonus = None;
        ui.counted = None;
        ui.stopped_at = None;
        ui.win = 0;
        self.poker.casino.celebration = None;
        self.poker.casino.pending = Some(now);
        self.engine.casino_play(&s.key, commit, Bet::Reels { machine, bet, buy });
    }

    pub(super) fn reels_page(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let now = Instant::now();
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        let time = ctx.input(|i| i.time) as f32;
        let th = theme(self.poker.casino.reels.machine);
        // The room: the machine's own picture, dimmed, its colors over it.
        p.rect_filled(area, 0, th.deep);
        pics::cover(&p, &ctx, &format!("reels-{}-tile", th.id), area, 1024, Color32::from_gray(96));
        vgrad(&p, area, Color32::from_black_alpha(90), Color32::from_black_alpha(200));
        radial(&p, area.center(), area.width().max(area.height()) * 0.7, Color32::TRANSPARENT, Color32::from_black_alpha(120));

        // A refused spin (not enough chips, say): the reels come to rest.
        if let Some(since) = self.poker.casino.reels.waiting
            && self.poker.casino.pending.is_none()
            && since.elapsed() > Duration::from_millis(300)
        {
            let r = &mut self.poker.casino.reels;
            r.waiting = None;
            for reel in &mut r.reels {
                *reel = Reel::still(reel.pos(now).round());
            }
            r.auto.stop();
        }

        // The cabinet: the reels as big as fit, a side for a funny machine's host.
        let funny = th.humor.is_some();
        let side_w = if funny { (area.width() * 0.2).clamp(150.0, 250.0) } else { 0.0 };
        let controls_h = 104.0;
        let top_h = 92.0;
        let max_w = (area.width() - 60.0 - side_w).min(1060.0);
        let cell = ((area.height() - controls_h - top_h - 60.0) / 3.0).min((max_w - 80.0) / 5.0).clamp(60.0, 168.0);
        let gap = (cell * 0.06).max(4.0);
        let reels_w = cell * 5.0 + gap * 4.0;
        let cab_w = reels_w + 70.0;
        let cab_h = top_h + cell * 3.0 + 34.0;
        let top = area.top() + ((area.height() - cab_h - controls_h) / 2.0).max(14.0);
        let cx = area.center().x + side_w / 2.0;
        let cab = Rect::from_center_size(pos2(cx, top + cab_h / 2.0), vec2(cab_w, cab_h));
        p.rect_filled(cab.translate(vec2(0.0, 12.0)), theme::r(30), Color32::from_black_alpha(150));
        vgrad_round(&p, cab, 28.0, super::mix(th.mid, Color32::BLACK, 0.15), th.deep);
        p.rect_stroke(cab, theme::r(28), Stroke::new(3.0f32, col::GOLD), StrokeKind::Inside);
        p.rect_stroke(cab.shrink(7.0), theme::r(22), Stroke::new(1.0f32, th.accent.gamma_multiply(0.6)), StrokeKind::Inside);
        // Lights along the top.
        for i in 0..22 {
            let x = cab.left() + 30.0 + i as f32 * (cab.width() - 60.0) / 21.0;
            let on = ((time * 5.0) as i32 + i) % 3 == 0;
            p.circle_filled(pos2(x, cab.top() + 13.0), 3.5, if on { th.accent } else { th.accent.gamma_multiply(0.3) });
        }
        // The marquee: its name, and what it pays back.
        glow_text(&p, pos2(cab.center().x, cab.top() + 42.0), Align2::CENTER_CENTER, th.name, theme::bold_font((cell * 0.24).clamp(22.0, 34.0)), col::GOLD2, th.accent.gamma_multiply(0.45));
        p.text(pos2(cab.center().x, cab.top() + 72.0), Align2::CENTER_CENTER, format!("{} · 243 ways · RTP 99%", th.tagline), theme::font(12.5), col::MUTED);

        let reels = Rect::from_min_size(pos2(cab.center().x - reels_w / 2.0, cab.top() + top_h), vec2(reels_w, cell * 3.0));
        let reel_rects: Vec<Rect> = (0..REELS).map(|i| Rect::from_min_size(pos2(reels.left() + i as f32 * (cell + gap), reels.top()), vec2(cell, cell * 3.0))).collect();
        let in_bonus = self.poker.casino.reels.bonus.is_some();
        if !in_bonus {
            self.reels_draw(&p, &ctx, th, &reel_rects, reels, cell, now, time);
        }
        // Counting the spin once the reels are still: its wins, then the bonus or the end.
        self.reels_after_stop(s, now);
        if in_bonus {
            self.reels_bonus(ui, s, th, reels.expand2(vec2(14.0, 10.0)), now, time);
        }
        // A funny machine's host and gimmick.
        if funny {
            let side = Rect::from_min_max(pos2((cab.left() - side_w - 16.0).max(area.left() + 12.0), cab.top()), pos2(cab.left() - 16.0, cab.bottom() + controls_h - 10.0));
            self.poker.casino.reels.fun.draw(&p, &ctx, th, side, reels, now, time);
        }
        // The controls.
        let bar = Rect::from_min_size(pos2(cab.left(), cab.bottom() + 12.0), vec2(cab.width(), controls_h - 18.0));
        self.reels_controls(ui, s, th, bar, now);
        // Autoplay spins again.
        let r = &self.poker.casino.reels;
        if r.auto.on() && r.next_at.is_some_and(|t| now >= t) && !self.poker.casino.busy() && !r.running(now) {
            self.poker.casino.reels.next_at = None;
            self.reels_spin(s, false);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn reels_draw(&mut self, p: &egui::Painter, ctx: &egui::Context, th: &Theme, reel_rects: &[Rect], reels: Rect, cell: f32, now: Instant, time: f32) {
        let r = &self.poker.casino.reels;
        let sheet = format!("reels-{}-symbols", th.id);
        let px = (cell * 3.0) as u32;
        // The glass behind the reels.
        let frame = reels.expand(10.0);
        p.rect_filled(frame, theme::r(16), Color32::from_black_alpha(220));
        for rr_ in reel_rects {
            vgrad(p, *rr_, super::mix(th.deep, Color32::BLACK, 0.4), super::mix(th.deep, th.mid, 0.25));
        }
        // What's lit: the wins being shown, one after another once the round's counted.
        let (lit, msg) = self.reels_lit(now);
        for (i, rr_) in reel_rects.iter().enumerate() {
            let clip = p.with_clip_rect(*rr_);
            let reel = r.reels[i];
            let pos = reel.pos(now);
            let base = pos.floor() as i32;
            let frac = pos - pos.floor();
            let blurred = reel.blurred(now);
            for row in -1..=3 {
                let idx = (base + row).rem_euclid(STRIP as i32) as usize;
                let sym = r.strips[i][idx];
                let y = rr_.top() + (row as f32 - frac) * cell;
                let sr = Rect::from_min_size(pos2(rr_.left(), y), vec2(cell, cell)).shrink(cell * 0.07);
                if sym == rr::BLANK {
                    // (Nothing landed here: a faint mark, so the reel doesn't look torn.)
                    clip.circle_filled(sr.center(), cell * 0.03, th.accent.gamma_multiply(0.18));
                    continue;
                }
                let on = lit.iter().any(|(rl, rw)| *rl == i && (row == *rw as i32)) && !blurred;
                if on {
                    let pulse = 0.5 + 0.5 * (time * 6.0).sin();
                    radial(&clip, sr.center(), cell * 0.55, th.accent.gamma_multiply(0.35 + 0.2 * pulse), Color32::TRANSPARENT);
                }
                let grow = if on { 1.0 + 0.05 * (time * 6.0).sin().abs() } else { 1.0 };
                let sr = Rect::from_center_size(sr.center(), sr.size() * grow);
                if blurred {
                    pics::cell(&clip, ctx, &sheet, sym as usize, sr.translate(vec2(0.0, -cell * 0.14)), px, Color32::from_white_alpha(70));
                    pics::cell(&clip, ctx, &sheet, sym as usize, sr, px, Color32::from_white_alpha(190));
                } else {
                    let dim = if lit.is_empty() || on { Color32::WHITE } else { Color32::from_gray(150) };
                    pics::cell(&clip, ctx, &sheet, sym as usize, sr, px, dim);
                }
                if on {
                    clip.rect_stroke(sr.expand(cell * 0.04), theme::r(12), Stroke::new(2.5f32, th.accent), StrokeKind::Inside);
                }
            }
            // Shading top and bottom, like a drum.
            vgrad(&clip, Rect::from_min_size(rr_.min, vec2(cell, cell * 0.3)), Color32::from_black_alpha(140), Color32::TRANSPARENT);
            vgrad(&clip, Rect::from_min_max(pos2(rr_.left(), rr_.bottom() - cell * 0.3), rr_.max), Color32::TRANSPARENT, Color32::from_black_alpha(140));
        }
        p.rect_stroke(frame, theme::r(16), Stroke::new(3.0f32, col::GOLD_DARK), StrokeKind::Inside);
        // The win being shown, under the reels.
        if let Some(m) = msg {
            let g = p.layout_no_wrap(m, theme::bold_font(17.0), col::GOLD2);
            let mr = Rect::from_center_size(pos2(reels.center().x, reels.bottom() - 26.0), g.size() + vec2(30.0, 14.0));
            p.rect_filled(mr, theme::r(17), Color32::from_black_alpha(215));
            p.rect_stroke(mr, theme::r(17), Stroke::new(1.0f32, th.accent.gamma_multiply(0.7)), StrokeKind::Inside);
            p.galley(mr.center() - g.size() / 2.0, g, col::GOLD2);
        }
    }

    /// The cells lit (reel, row) and the line about the win shown now.
    fn reels_lit(&self, now: Instant) -> (Vec<(usize, usize)>, Option<String>) {
        let r = &self.poker.casino.reels;
        let (Some(round), Some(since)) = (&r.showing, r.counted) else { return (Vec::new(), None) };
        let Outcome::Reels(o) = &round.outcome else { return (Vec::new(), None) };
        let Some(grid) = o.grid else { return (Vec::new(), None) };
        if o.wins.is_empty() {
            return (Vec::new(), None);
        }
        let k = now.duration_since(since).as_secs_f32();
        let th = theme(o.machine);
        // First every win at once, then each in turn.
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
            None => {
                let base: u64 = o.wins.iter().map(|w| w.pay).sum();
                Some(format!("WIN {}", chips_text(base)))
            }
        };
        (lit, msg)
    }

    /// Once the reels are still: the wins counted (and said), then the bonus, or the round's end.
    fn reels_after_stop(&mut self, s: &ServerView, now: Instant) {
        let r = &self.poker.casino.reels;
        let Some(end) = r.stopped_at else { return };
        if now < end || r.counted.is_some() {
            return;
        }
        let Some(round) = r.showing.clone() else { return };
        let Outcome::Reels(o) = &round.outcome else { return };
        let th = theme(o.machine);
        let base: u64 = o.wins.iter().map(|w| w.pay).sum();
        let ui = &mut self.poker.casino.reels;
        ui.counted = Some(now);
        ui.win = base;
        if base > 0 {
            let big = base >= o.bet * 10;
            self.poker.casino.sound_in(0.0, if big { Cue::WinBig } else { Cue::WinSmall }, 0.0, 0.7);
        }
        // The funny machines take it in (a bonus is told when it's over).
        if o.bonus.is_none() {
            self.poker.casino.reels.fun.round(th.humor, o, round.nonce);
        }
        match &o.bonus {
            Some(_) if !o.bought => {
                // Scatters: the bonus, after a moment to see them.
                self.poker.casino.announce(format!("{}!", th.bonus.to_uppercase()), format!("{} {}", o.scatters, th.scatters), th.accent, (0.5, 0.42), 0.3);
                self.poker.casino.sound_in(0.4, Cue::Jackpot, 0.0, 0.6);
                self.poker.casino.reels.bonus = Some(BonusPlay { round: round.clone(), opened: Vec::new(), wheel_at: None, inner_at: None, over_at: None, collected: false, started: now + Duration::from_millis(1_800) });
            }
            // (A bought bonus: its board's up already, and the round ends when it's collected.)
            Some(_) => {}
            None => self.reels_finish(s, &round, now),
        }
    }

    /// The round's over on screen: its win on the balance, a celebration if it's big, and the
    /// next spin if it's spinning by itself.
    fn reels_finish(&mut self, _s: &ServerView, round: &RoundView, now: Instant) {
        let Outcome::Reels(o) = &round.outcome else { return };
        self.poker.casino.settle_balance();
        self.poker.casino.celebrate(o.paid, o.wagered(), false, 0.2);
        let big = o.paid >= o.wagered() * 10;
        let ui = &mut self.poker.casino.reels;
        if ui.auto.spun(big) {
            let pause = if o.paid > 0 { 1.3 } else { 0.5 };
            ui.next_at = Some(now + Duration::from_secs_f32(pause * if ui.turbo { 0.5 } else { 1.0 }));
        }
    }

    /// The bonus board: pick-and-dodge's tiles, or the wheel.
    fn reels_bonus(&mut self, ui: &mut egui::Ui, s: &ServerView, th: &Theme, r: Rect, now: Instant, time: f32) {
        let Some(b) = self.poker.casino.reels.bonus.as_ref() else { return };
        if now < b.started {
            // (The reels with the scatters still show until it starts.)
            return;
        }
        let Outcome::Reels(o) = b.round.outcome.clone() else { return };
        let Some(bonus) = o.bonus.clone() else { return };
        let p = ui.painter().clone();
        p.rect_filled(r, theme::r(18), Color32::from_black_alpha(235));
        p.rect_stroke(r, theme::r(18), Stroke::new(2.0f32, th.accent.gamma_multiply(0.8)), StrokeKind::Inside);
        match &bonus {
            Bonus::Pick { picks, finale, won } => self.reels_pick(ui, s, th, &o, picks, *finale, *won, r, now, time),
            Bonus::Wheel { slice, inner, won } => self.reels_wheel(ui, s, th, &o, *slice, *inner, *won, r, now, time),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn reels_pick(&mut self, ui: &mut egui::Ui, s: &ServerView, th: &Theme, o: &rr::Outcome, picks: &[Pick], finale: bool, won: u64, r: Rect, now: Instant, _time: f32) {
        let p = ui.painter().clone();
        let ctx = ui.ctx().clone();
        let opened = self.poker.casino.reels.bonus.as_ref().map(|b| b.opened.clone()).unwrap_or_default();
        let shown = &picks[..opened.len().min(picks.len())];
        let lives = 1 + shown.iter().filter(|x| matches!(x, Pick::Life)).count() as i32 - shown.iter().filter(|x| matches!(x, Pick::Saved)).count() as i32;
        let so_far: u64 = shown.iter().map(|x| match x {
            Pick::Chips { chips } | Pick::Gold { chips } => *chips,
            _ => 0,
        }).sum();
        let done = opened.len() >= picks.len();
        // The heading.
        glow_text(&p, pos2(r.center().x, r.top() + 26.0), Align2::CENTER_CENTER, &th.bonus.to_uppercase(), theme::bold_font(24.0), col::GOLD2, th.accent.gamma_multiply(0.5));
        let line = format!("{} {} hide in here: each uses up a life, and with none left it's over. Find every safe tile for {}.", rr::HAZARDS, th.hazards, th.finale);
        let g = p.layout(line, theme::font(12.5), col::MUTED, r.width() - 40.0);
        p.galley(pos2(r.center().x - g.size().x / 2.0, r.top() + 46.0), g, col::MUTED);
        let lives = lives.max(0);
        let status = format!("Won {} · {} li{} left", chips_text(so_far + if done && finale { o.bet * rr::FINALE_X } else { 0 }), lives, if lives == 1 { "fe" } else { "ves" });
        p.text(pos2(r.center().x, r.top() + 84.0), Align2::CENTER_CENTER, status, theme::bold_font(15.0), col::TEXT);
        // The board: five across, three down.
        let board = Rect::from_min_max(pos2(r.left() + 18.0, r.top() + 100.0), pos2(r.right() - 18.0, r.bottom() - 54.0));
        let tw = (board.width() - 4.0 * 10.0) / 5.0;
        let thh = (board.height() - 2.0 * 10.0) / 3.0;
        let sheet = format!("reels-{}-symbols", th.id);
        let mut clicked = None;
        for i in 0..rr::TILES {
            let (cx, cy) = ((i % 5) as f32, (i / 5) as f32);
            let tr = Rect::from_min_size(pos2(board.left() + cx * (tw + 10.0), board.top() + cy * (thh + 10.0)), vec2(tw, thh));
            let at = opened.iter().position(|t| *t == i);
            match at.and_then(|k| picks.get(k).map(|x| (k, x))) {
                None => {
                    let resp = ui.interact(tr, ui.id().with(("pick", i)), if done { Sense::hover() } else { Sense::click() });
                    let hover = resp.hovered() && !done;
                    vgrad_round(&p, tr, 12.0, if hover { th.accent.gamma_multiply(0.8) } else { th.mid }, th.deep);
                    p.rect_stroke(tr, theme::r(12), Stroke::new(1.5f32, col::GOLD.gamma_multiply(if hover { 1.0 } else { 0.6 })), StrokeKind::Inside);
                    p.text(tr.center(), Align2::CENTER_CENTER, "?", theme::bold_font((thh * 0.4).min(40.0)), col::GOLD2.gamma_multiply(if done { 0.4 } else { 0.9 }));
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !done {
                        clicked = Some(i);
                    }
                }
                Some((k, pick)) => {
                    let fresh = k + 1 == opened.len();
                    let pop = if fresh { 1.0 + 0.08 * (1.0 - ((now.duration_since(self.poker.casino.reels.bonus.as_ref().map(|b| b.started).unwrap_or(now)).as_secs_f32() * 3.0).min(1.0))) } else { 1.0 };
                    let tr = Rect::from_center_size(tr.center(), tr.size() * pop);
                    let (fill, text, sub): (Color32, String, &str) = match pick {
                        Pick::Chips { chips } => (Color32::from_rgb(0x1e, 0x3a, 0x22), format!("+{}", chips_text(*chips)), "chips"),
                        Pick::Gold { chips } => (Color32::from_rgb(0x5a, 0x42, 0x0a), format!("+{}", chips_text(*chips)), th.gold),
                        Pick::Life => (Color32::from_rgb(0x1a, 0x3a, 0x5a), "+1 LIFE".into(), th.life),
                        Pick::Saved => (Color32::from_rgb(0x4a, 0x2a, 0x10), "SAVED".into(), th.hazard),
                        Pick::Busted => (Color32::from_rgb(0x5a, 0x0e, 0x0e), th.busted.into(), th.hazard),
                    };
                    vgrad_round(&p, tr, 12.0, fill, super::mix(fill, Color32::BLACK, 0.5));
                    p.rect_stroke(tr, theme::r(12), Stroke::new(1.5f32, col::GOLD.gamma_multiply(0.7)), StrokeKind::Inside);
                    if matches!(pick, Pick::Saved | Pick::Busted) {
                        let ir = Rect::from_center_size(tr.center() - vec2(0.0, thh * 0.08), vec2(thh * 0.6, thh * 0.6));
                        pics::cell(&p, &ctx, &sheet, 8, ir, (ir.width() * 3.0) as u32, Color32::WHITE);
                        p.text(pos2(tr.center().x, tr.bottom() - 14.0), Align2::CENTER_CENTER, text, theme::bold_font(13.0), if matches!(pick, Pick::Busted) { col::LOSE } else { col::GOLD2 });
                    } else {
                        p.text(tr.center() - vec2(0.0, 8.0), Align2::CENTER_CENTER, text, theme::bold_font((thh * 0.22).clamp(13.0, 22.0)), col::GOLD2);
                        p.text(tr.center() + vec2(0.0, 14.0), Align2::CENTER_CENTER, sub, theme::font(11.5), col::MUTED);
                    }
                }
            }
        }
        // Spinning by itself: the tiles open themselves, left to right.
        let auto = self.poker.casino.reels.auto.on();
        let b = self.poker.casino.reels.bonus.as_mut().unwrap();
        if clicked.is_none() && auto && !done {
            let last = b.over_at.unwrap_or(b.started);
            if now.duration_since(last) > Duration::from_millis(550) {
                clicked = (0..rr::TILES).find(|t| !b.opened.contains(t));
            }
        }
        if let Some(i) = clicked {
            b.opened.push(i);
            b.over_at = Some(now);
            let k = b.opened.len() - 1;
            let cue = match picks.get(k) {
                Some(Pick::Chips { .. }) => Cue::Coin,
                Some(Pick::Gold { .. }) => Cue::WinSmall,
                Some(Pick::Busted) => Cue::Fold,
                _ => Cue::Flip,
            };
            self.poker.casino.sound_in(0.0, cue, 0.0, 0.6);
            if k + 1 == picks.len() && finale {
                self.poker.casino.sound_in(0.2, Cue::Jackpot, 0.0, 0.7);
                self.poker.casino.announce(format!("{}!", th.finale), format!("+{}", chips_text(o.bet * rr::FINALE_X)), col::GOLD2, (0.5, 0.45), 0.2);
            }
        }
        // Over: collect (by itself when spinning by itself).
        if done {
            let br = Rect::from_center_size(pos2(r.center().x, r.bottom() - 30.0), vec2(260.0, 44.0));
            self.reels_collect(ui, s, th, o, won, br, now);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn reels_wheel(&mut self, ui: &mut egui::Ui, s: &ServerView, th: &Theme, o: &rr::Outcome, slice: u8, inner: Option<u8>, won: u64, r: Rect, now: Instant, time: f32) {
        let p = ui.painter().clone();
        let b = self.poker.casino.reels.bonus.as_ref().unwrap();
        let (wheel_at, inner_at) = (b.wheel_at, b.inner_at);
        let spin_secs = if self.poker.casino.reels.turbo { 3.0 } else { 5.0 };
        // The wheel on the left and what it's about on the right; in a narrow room, the wheel
        // under its name and the button under it.
        let wide = r.width() > r.height() * 1.25;
        let (c, radius, side) = if wide {
            let radius = ((r.height() - 64.0) / 2.0).min(r.width() * 0.27).max(70.0);
            let c = pos2(r.left() + 44.0 + radius, r.center().y + 12.0);
            (c, radius, Rect::from_min_max(pos2(c.x + radius + 44.0, r.top() + 28.0), pos2(r.right() - 26.0, r.bottom() - 24.0)))
        } else {
            let radius = ((r.height() - 150.0) / 2.0).min(r.width() * 0.4).max(60.0);
            let c = pos2(r.center().x, r.top() + 78.0 + radius);
            (c, radius, Rect::from_min_max(pos2(r.left() + 20.0, c.y + radius + 10.0), pos2(r.right() - 20.0, r.bottom() - 8.0)))
        };
        // Its name, and (when there's room) what it holds.
        let lo = rr::OUTER.iter().filter(|x| **x != rr::UPGRADE).min().copied().unwrap_or(0);
        let hi = rr::OUTER.iter().max().copied().unwrap_or(0);
        if wide {
            glow_text(&p, side.left_top(), Align2::LEFT_TOP, &th.bonus.to_uppercase(), theme::bold_font(28.0), col::GOLD2, th.accent.gamma_multiply(0.5));
            let g = p.layout(
                format!("{} slices of {lo}× to {hi}× the bet, and one UPGRADE: a second wheel of {}× to {}×.", rr::OUTER.len() - 1, rr::INNER[0], rr::INNER[rr::INNER.len() - 1]),
                theme::font(14.0),
                col::MUTED,
                side.width(),
            );
            p.galley(side.left_top() + vec2(0.0, 44.0), g, col::MUTED);
        } else {
            glow_text(&p, pos2(r.center().x, r.top() + 26.0), Align2::CENTER_CENTER, &th.bonus.to_uppercase(), theme::bold_font(24.0), col::GOLD2, th.accent.gamma_multiply(0.5));
        }
        // The outer wheel's angle: turning slowly until it's spun, then easing to its slice
        // under the pointer (at the top).
        let n = rr::OUTER.len() as f32;
        let seg = std::f32::consts::TAU / n;
        let rest = -std::f32::consts::FRAC_PI_2 - (slice as f32 + 0.5) * seg;
        let turns = 5.0 * std::f32::consts::TAU;
        let ang = match wheel_at {
            None => -std::f32::consts::FRAC_PI_2 + time * 0.15,
            Some(t) => {
                let k = (now.duration_since(t).as_secs_f32() / spin_secs).min(1.0);
                rest - turns * (1.0 - ease_out(k))
            }
        };
        let landed = wheel_at.is_some_and(|t| now.duration_since(t).as_secs_f32() >= spin_secs);
        draw_wheel(&p, c, radius, ang, &rr::OUTER.map(|x| if x == rr::UPGRADE { "UPGRADE".to_string() } else { format!("{x}×") }), th, Some(slice as usize).filter(|_| landed), time);
        // The Upgrade: the inner wheel, its own spin.
        let mut inner_landed = inner.is_none();
        if let (Some(i), true) = (inner, landed) {
            let ir = radius * 0.55;
            let n2 = rr::INNER.len() as f32;
            let seg2 = std::f32::consts::TAU / n2;
            let rest2 = -std::f32::consts::FRAC_PI_2 - (i as f32 + 0.5) * seg2;
            let ang2 = match inner_at {
                None => -std::f32::consts::FRAC_PI_2,
                Some(t) => {
                    let k = (now.duration_since(t).as_secs_f32() / spin_secs).min(1.0);
                    rest2 - 4.0 * std::f32::consts::TAU * (1.0 - ease_out(k))
                }
            };
            inner_landed = inner_at.is_some_and(|t| now.duration_since(t).as_secs_f32() >= spin_secs);
            p.circle_filled(c, ir + 8.0, Color32::from_black_alpha(200));
            draw_wheel(&p, c, ir, ang2, &rr::INNER.map(|x| format!("{x}×")), th, Some(i as usize).filter(|_| inner_landed), time);
            if inner_at.is_none() {
                self.poker.casino.reels.bonus.as_mut().unwrap().inner_at = Some(now + Duration::from_millis(900));
                self.poker.casino.sound_in(0.9, Cue::Wheel, 0.0, 0.8);
                self.poker.casino.announce("UPGRADE!", "the jackpot wheel", col::GOLD2, (0.5, 0.2), 0.0);
            }
        }
        // The pointer, at the top.
        let tip = pos2(c.x, c.y - radius - 4.0);
        p.add(Shape::convex_polygon(vec![tip + vec2(-14.0, -26.0), tip + vec2(14.0, -26.0), tip + vec2(0.0, 6.0)], col::GOLD2, Stroke::new(2.0f32, Color32::from_rgb(0x5a, 0x3a, 0x08))));
        // What it landed on.
        if wide && landed {
            let x = super::times(pyr_poker::casino::multiple_x100(won, o.bet));
            let what = if inner.is_some() { if inner_landed { format!("UPGRADED · {x}") } else { "UPGRADE!".into() } } else { x };
            let y = side.top() + 120.0;
            p.text(pos2(side.left(), y), Align2::LEFT_TOP, "LANDED ON", theme::bold_font(12.0), col::MUTED);
            glow_text(&p, pos2(side.left(), y + 18.0), Align2::LEFT_TOP, &what, theme::bold_font(40.0), th.accent, Color32::from_black_alpha(140));
            if inner_landed {
                p.text(pos2(side.left(), y + 70.0), Align2::LEFT_TOP, format!("{} chips", chips_text(won)), theme::bold_font(20.0), col::WIN);
            }
        }
        let button = if wide { Rect::from_min_size(pos2(side.left(), side.bottom() - 46.0), vec2(side.width().min(260.0), 46.0)) } else { Rect::from_center_size(pos2(r.center().x, r.bottom() - 34.0), vec2(240.0, 44.0)) };
        // Spin it (by itself after a moment, when spinning by itself).
        if wheel_at.is_none() {
            let resp = ui.interact(button, ui.id().with("wheel-spin"), Sense::click());
            p.rect_filled(button.translate(vec2(0.0, 3.0)), theme::r(23), Color32::from_black_alpha(120));
            p.rect_filled(button, theme::r(23), if resp.hovered() { col::GOLD2 } else { col::GOLD });
            p.text(button.center(), Align2::CENTER_CENTER, "SPIN THE WHEEL", theme::bold_font(17.0), Color32::from_rgb(0x2a, 0x10, 0x04));
            let auto = self.poker.casino.reels.auto.on();
            let started = self.poker.casino.reels.bonus.as_ref().map(|b| b.started).unwrap_or(now);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() || (auto && now.duration_since(started) > Duration::from_millis(800)) {
                self.poker.casino.reels.bonus.as_mut().unwrap().wheel_at = Some(now);
                self.poker.casino.sound_in(0.0, Cue::Wheel, 0.0, 0.8);
            }
        }
        if landed && inner_landed {
            self.reels_collect(ui, s, th, o, won, button, now);
        }
    }

    /// The bonus is over: its win, and Collect (by itself after a moment when spinning by
    /// itself).
    #[allow(clippy::too_many_arguments)]
    fn reels_collect(&mut self, ui: &mut egui::Ui, s: &ServerView, th: &Theme, o: &rr::Outcome, won: u64, br: Rect, now: Instant) {
        let p = ui.painter().clone();
        let b = self.poker.casino.reels.bonus.as_mut().unwrap();
        let over_at = *b.over_at.get_or_insert(now);
        if b.collected {
            return;
        }
        let resp = ui.interact(br, ui.id().with("collect"), Sense::click());
        p.rect_filled(br.translate(vec2(0.0, 3.0)), theme::r(23), Color32::from_black_alpha(120));
        p.rect_filled(br, theme::r(23), if resp.hovered() { col::GOLD2 } else { col::GOLD });
        p.text(br.center(), Align2::CENTER_CENTER, format!("COLLECT {}", chips_text(won)), theme::bold_font(17.0), Color32::from_rgb(0x2a, 0x10, 0x04));
        let auto = self.poker.casino.reels.auto.on();
        if resp.clicked() || (auto && now.duration_since(over_at) > Duration::from_millis(1_800)) {
            let round = self.poker.casino.reels.bonus.as_ref().unwrap().round.clone();
            let ui_s = &mut self.poker.casino.reels;
            ui_s.bonus.as_mut().unwrap().collected = true;
            ui_s.bonus = None;
            ui_s.win = o.paid;
            ui_s.fun.round(th.humor, o, round.nonce);
            self.reels_finish(s, &round, now);
        }
    }

    fn reels_controls(&mut self, ui: &mut egui::Ui, s: &ServerView, th: &Theme, bar: Rect, now: Instant) {
        let p = ui.painter().clone();
        vgrad_round(&p, bar, 18.0, Color32::from_rgb(0x16, 0x10, 0x16), Color32::from_rgb(0x0c, 0x08, 0x0e));
        p.rect_stroke(bar, theme::r(18), Stroke::new(1.0f32, col::GOLD_DARK), StrokeKind::Inside);
        let cy = bar.center().y;
        let busy = self.poker.casino.busy() || self.poker.casino.reels.running(now);
        let bet = self.poker.casino.reels.bet;
        // The bet: − and +, and the amount (a click lists them all).
        p.text(pos2(bar.left() + 22.0, bar.top() + 18.0), Align2::LEFT_CENTER, "BET", theme::bold_font(11.5), col::MUTED);
        let minus = Rect::from_center_size(pos2(bar.left() + 36.0, cy + 10.0), vec2(32.0, 32.0));
        let plus = Rect::from_center_size(pos2(bar.left() + 150.0, cy + 10.0), vec2(32.0, 32.0));
        for (r, label, dir) in [(minus, "−", -1i32), (plus, "+", 1)] {
            let resp = ui.interact(r, ui.id().with(("rbet", label)), Sense::click());
            p.circle_filled(r.center(), 16.0, if resp.hovered() && !busy { col::GOLD } else { col::GOLD_DARK });
            p.text(r.center(), Align2::CENTER_CENTER, label, theme::bold_font(19.0), Color32::BLACK);
            if resp.clicked() && !busy {
                let i = BETS.iter().position(|x| *x == bet).unwrap_or(2) as i32;
                self.poker.casino.reels.bet = BETS[(i + dir).clamp(0, BETS.len() as i32 - 1) as usize];
            }
        }
        let amount = Rect::from_center_size(pos2(bar.left() + 93.0, cy + 10.0), vec2(76.0, 30.0));
        let aresp = ui.interact(amount, ui.id().with("rbet-pick"), Sense::click()).on_hover_text("Choose the bet");
        p.text(amount.center(), Align2::CENTER_CENTER, chips_text(bet), theme::bold_font(20.0), col::GOLD2);
        if !busy {
            egui::Popup::menu(&aresp).align(egui::RectAlign::TOP_START).show(|ui| {
                crate::menu::style(ui);
                for b in BETS {
                    if ui.button(format!("{} chips", chips_text(b))).clicked() {
                        self.poker.casino.reels.bet = b;
                        ui.close();
                    }
                }
            });
        }
        // Win.
        let win = self.poker.casino.reels.win;
        p.text(pos2(bar.left() + 190.0, bar.top() + 18.0), Align2::LEFT_CENTER, "WIN", theme::bold_font(11.5), col::MUTED);
        p.text(pos2(bar.left() + 190.0, cy + 10.0), Align2::LEFT_CENTER, if win > 0 { chips_text(win) } else { "–".into() }, theme::bold_font(22.0), if win > 0 { col::WIN } else { col::MUTED });
        // Spin: between the win and the buttons on the right (their widths as they're drawn
        // below).
        let auto_now = self.poker.casino.reels.auto;
        let auto_text = if auto_now.on() { format!("Auto {}", auto_now.left_text()) } else { "Auto ⌄".into() };
        let buttons_w: f32 = [format!("Buy bonus {}", chips_text(bet * rr::BUY_X)), "Turbo".into(), auto_text, "Info".into()].iter().map(|t| 20.0 + t.chars().count() as f32 * 7.6 + 8.0).sum();
        let left_of_buttons = bar.right() - 16.0 - buttons_w;
        let spin_c = pos2(((bar.left() + 310.0 + left_of_buttons) / 2.0).max(bar.left() + 350.0), cy);
        let spin_r = Rect::from_center_size(spin_c, vec2(80.0, 80.0));
        let resp = ui.interact(spin_r, ui.id().with("rspin"), Sense::click());
        let auto = self.poker.casino.reels.auto;
        p.circle_filled(spin_c + vec2(0.0, 4.0), 40.0, Color32::from_black_alpha(140));
        radial(&p, spin_c, 40.0, if resp.hovered() && !busy { th.accent } else { super::mix(th.accent, col::GOLD, 0.5) }, super::mix(th.mid, Color32::BLACK, 0.2));
        p.circle_stroke(spin_c, 40.0, Stroke::new(3.0f32, col::GOLD2));
        p.text(spin_c - vec2(0.0, if auto.on() { 6.0 } else { 0.0 }), Align2::CENTER_CENTER, if auto.on() { "STOP" } else { "SPIN" }, theme::bold_font(20.0), Color32::from_rgb(0x1a, 0x08, 0x04));
        if auto.on() {
            p.text(spin_c + vec2(0.0, 15.0), Align2::CENTER_CENTER, format!("auto {}", auto.left_text()), theme::bold_font(10.5), Color32::from_rgb(0x1a, 0x08, 0x04));
        }
        let space = ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) && !ui.ctx().wants_keyboard_input() && self.poker.casino.info.is_none();
        if resp.on_hover_text("Spin (Space)").clicked() || space {
            if auto.on() {
                self.poker.casino.reels.auto.stop();
            } else if !busy {
                self.reels_spin(s, false);
            }
        }
        // Buy the bonus, Auto, Turbo, Info (right to left).
        let mut bx = bar.right() - 16.0;
        let mut small = |ui: &mut egui::Ui, id: &str, text: &str, on: bool, tip: &str| -> egui::Response {
            let w = 20.0 + text.chars().count() as f32 * 7.6;
            let r = Rect::from_min_size(pos2(bx - w, cy - 15.0), vec2(w, 30.0));
            bx -= w + 8.0;
            let resp = ui.interact(r, ui.id().with(("rbtn", id)), Sense::click());
            ui.painter().rect_filled(r, theme::r(15), if on { col::GOLD } else if resp.hovered() { Color32::from_white_alpha(40) } else { Color32::from_white_alpha(18) });
            ui.painter().text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(12.0), if on { Color32::BLACK } else { col::TEXT });
            resp.on_hover_text(tip)
        };
        let cost = bet * rr::BUY_X;
        if small(ui, "buy", &format!("Buy bonus {}", chips_text(cost)), false, &format!("Start {} now, for {} times the bet", th.bonus, rr::BUY_X)).clicked() && !busy && !auto.on() {
            self.reels_spin(s, true);
        }
        if small(ui, "turbo", "Turbo", self.poker.casino.reels.turbo, "Faster spins").clicked() {
            self.poker.casino.reels.turbo = !self.poker.casino.reels.turbo;
        }
        let auto_label = if auto.on() { format!("Auto {}", auto.left_text()) } else { "Auto ⌄".into() };
        let resp = small(ui, "auto", &auto_label, auto.on(), if auto.on() { "Stop spinning by itself" } else { "Spin by itself: 10 to 1,000 times, or until you stop it" });
        if auto.on() {
            if resp.clicked() {
                self.poker.casino.reels.auto.stop();
            }
        } else if let Some(n) = super::auto::menu(&resp, &mut self.poker.casino.reels.auto) {
            self.poker.casino.reels.auto.left = n;
            if !busy {
                self.poker.casino.reels.next_at = Some(Instant::now());
            }
        }
        if small(ui, "info", "Info", self.poker.casino.info.is_some(), "What it pays, its statistics and its story").clicked() {
            self.info_toggle();
        }
        if super::chips_now(s) < bet {
            p.text(pos2(bar.center().x, bar.bottom() - 8.0), Align2::CENTER_CENTER, "Not enough chips for this bet: lower it, or take the daily chips in the lobby.", theme::font(11.0), col::LOSE);
        }
    }
}

/// A prize wheel: its slices round `c`, turned to `ang` (radians, the first slice's start),
/// `hit` lit when it's landed.
fn draw_wheel(p: &egui::Painter, c: Pos2, radius: f32, ang: f32, labels: &[String], th: &Theme, hit: Option<usize>, time: f32) {
    let n = labels.len();
    let seg = std::f32::consts::TAU / n as f32;
    p.circle_filled(c + vec2(0.0, 6.0), radius + 10.0, Color32::from_black_alpha(140));
    p.circle_filled(c, radius + 9.0, col::GOLD_DARK);
    for i in 0..n {
        let a0 = ang + i as f32 * seg;
        let upgrade = labels[i] == "UPGRADE";
        let fill = if upgrade {
            col::GOLD
        } else if i % 2 == 0 {
            th.mid
        } else {
            super::mix(th.deep, th.mid, 0.4)
        };
        let lit = hit == Some(i);
        let fill = if lit { super::mix(fill, Color32::WHITE, 0.25 + 0.15 * (time * 8.0).sin().abs()) } else { fill };
        let mut pts = vec![c];
        for k in 0..=8 {
            let a = a0 + seg * k as f32 / 8.0;
            pts.push(c + vec2(a.cos(), a.sin()) * radius);
        }
        p.add(Shape::convex_polygon(pts, fill, Stroke::new(1.0f32, Color32::from_black_alpha(120))));
        // Its label, along the slice from the rim inwards.
        let mid = a0 + seg / 2.0;
        let at = c + vec2(mid.cos(), mid.sin()) * radius * 0.7;
        let size = if upgrade { (radius * 0.07).clamp(9.0, 14.0) } else { (radius * 0.1).clamp(11.0, 20.0) };
        p.text(at, Align2::CENTER_CENTER, &labels[i], theme::bold_font(size), if upgrade { Color32::from_rgb(0x2a, 0x10, 0x04) } else { col::TEXT });
    }
    // Lights round the rim, and the hub.
    for k in 0..n * 2 {
        let a = ang + k as f32 * seg / 2.0;
        let on = ((time * 6.0) as usize + k) % 2 == 0;
        p.circle_filled(c + vec2(a.cos(), a.sin()) * (radius + 4.5), 2.6, if on { col::GOLD2 } else { th.accent.gamma_multiply(0.5) });
    }
    p.circle_filled(c, radius * 0.16, col::GOLD);
    p.circle_stroke(c, radius * 0.16, Stroke::new(2.0f32, Color32::from_rgb(0x5a, 0x3a, 0x08)));
}
