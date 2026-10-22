//! Pyr Sevens at the app: the cabinet, the reels spinning and stopping one by one, the winning
//! lines traced, free spins, autoplay and turbo.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::casino::slots::{self as sl, LINES, PAYLINES, REELS, ROWS, STRIPS};
use pyr_poker::casino::Bet;
use pyr_poker::net::RoundView;

use super::{art, col, glow_text, radial, vgrad, vgrad_round};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

const LINE_BETS: [u64; 10] = [1, 2, 5, 10, 25, 50, 100, 200, 250, 500];
use super::spin::{Reel, SPEED};

pub struct SlotsUi {
    pub line_bet: u64,
    reels: [Reel; REELS],
    /// The round being shown, which of its spins, and when that spin's reels all stop.
    showing: Option<RoundView>,
    spin: usize,
    stopped_at: Option<Instant>,
    /// The spin last counted; the spin whose wins are on show, since when; and when the next
    /// free spin sets off (the wins show until then).
    counted: Option<usize>,
    display: Option<(usize, Instant)>,
    next_start: Option<Instant>,
    /// Spinning, waiting for the house.
    waiting: Option<Instant>,
    /// Wins counted so far in the round (free spins add up).
    round_win: u64,
    /// Autoplay: spins left (or until stopped), and whether a big win stops it.
    pub auto: super::auto::Auto,
    pub turbo: bool,
    pub paytable: bool,
    jackpot: f64,
    /// Free spins: the banner shown since.
    banner: Option<(Instant, String)>,
    next_at: Option<Instant>,
}

impl Default for SlotsUi {
    fn default() -> Self {
        SlotsUi {
            line_bet: 5,
            reels: std::array::from_fn(|r| Reel::still((r * 7 % STRIPS[r].len()) as f32)),
            showing: None,
            spin: 0,
            stopped_at: None,
            counted: None,
            display: None,
            next_start: None,
            waiting: None,
            round_win: 0,
            auto: Default::default(),
            turbo: false,
            paytable: false,
            jackpot: 0.0,
            banner: None,
            next_at: None,
        }
    }
}

impl SlotsUi {
    /// Spinning, landing, or still counting a round's spins.
    fn running(&self, now: Instant) -> bool {
        if self.waiting.is_some() || self.stopped_at.is_some_and(|e| now < e) {
            return true;
        }
        match &self.showing {
            Some(r) => match &r.outcome {
                pyr_poker::casino::Outcome::Slots(o) => self.counted != Some(o.spins.len() - 1),
                _ => false,
            },
            None => false,
        }
    }

    /// The jackpot as shown: counting up toward the real pool.
    pub fn shown_jackpot(&mut self, pool: u64) -> u64 {
        if self.jackpot <= 0.0 || (pool as f64) < self.jackpot {
            self.jackpot = pool as f64;
        }
        self.jackpot += (pool as f64 - self.jackpot) * 0.06;
        self.jackpot.round() as u64
    }

    fn start_spinning(&mut self, now: Instant) {
        for r in &mut self.reels {
            r.start(now, SPEED);
        }
    }

    /// Lands each reel on `stops`, one after another from `from`: when each comes to rest.
    fn land(&mut self, stops: [u8; REELS], from: Instant) -> [Instant; REELS] {
        let lens: Vec<usize> = STRIPS.iter().map(|s| s.len()).collect();
        let stops: Vec<usize> = stops.iter().map(|s| *s as usize).collect();
        let ends = super::spin::land(&mut self.reels, &lens, &stops, from, self.turbo);
        std::array::from_fn(|i| ends[i])
    }
}

impl PyrApp {
    pub(super) fn slots_round(&mut self, round: &RoundView) {
        let pyr_poker::casino::Outcome::Slots(o) = &round.outcome else { return };
        let now = Instant::now();
        let ui = &mut self.poker.casino.slots;
        // Spin at least a moment before stopping.
        let min = if ui.turbo { 0.25 } else { 0.7 };
        let from = ui.waiting.map(|t| (t + Duration::from_secs_f32(min)).max(now)).unwrap_or(now);
        if ui.waiting.is_none() {
            ui.start_spinning(now);
        }
        ui.waiting = None;
        ui.spin = 0;
        ui.round_win = 0;
        ui.counted = None;
        ui.display = None;
        ui.next_start = None;
        ui.showing = Some(round.clone());
        let stops = o.spins[0].stops;
        let ends = ui.land(stops, from);
        self.slots_landing(ends, now);
    }

    /// When the reels come to rest: each one's thunk, and the spin counted after the last.
    fn slots_landing(&mut self, ends: [Instant; REELS], now: Instant) {
        let dur = if self.poker.casino.slots.turbo { 0.32 } else { 0.5 };
        let last = ends.iter().max().copied().unwrap_or(now) + Duration::from_secs_f32(dur * 0.45);
        self.poker.casino.slots.stopped_at = Some(last);
        for (i, e) in ends.iter().enumerate() {
            self.poker.casino.sound_in(e.saturating_duration_since(now).as_secs_f32(), Cue::ReelStop, (i as f32 - 2.0) * 0.3, 0.7);
        }
    }

    pub(super) fn slots_page(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let now = Instant::now();
        let p = ui.painter().clone();
        let t = ui.ctx().input(|i| i.time) as f32;
        let c = s.poker.casino.clone();
        let free_round = self.poker.casino.slots.showing.as_ref().is_some_and(|r| self.poker.casino.slots.spin > 0 && matches!(&r.outcome, pyr_poker::casino::Outcome::Slots(o) if o.spins.len() > 1));
        // The room: its picture (the sevens in flames, Meshy's), dimmed, with a warm (or, in
        // free spins, violet) glow.
        p.rect_filled(area, 0, Color32::from_rgb(0x0c, 0x06, 0x08));
        super::pics::cover(&p, ui.ctx(), "tile-sevens", area, 1024, Color32::from_gray(80));
        vgrad(&p, area, Color32::from_black_alpha(110), Color32::from_black_alpha(210));
        radial(&p, area.center() - vec2(0.0, 60.0), area.width() * 0.6, if free_round { Color32::from_rgba_unmultiplied(0x9a, 0x4a, 0xff, 50) } else { Color32::from_rgba_unmultiplied(0xff, 0x6a, 0x1a, 36) }, Color32::TRANSPARENT);

        // The cabinet's size: the reels as big as fit.
        let controls_h = 118.0;
        let top_h = 96.0;
        let max_w = (area.width() - 60.0).min(1040.0);
        let cell = ((area.height() - controls_h - top_h - 70.0) / 3.0).min((max_w - 120.0) / 5.0).clamp(64.0, 170.0);
        let reels_w = cell * 5.0 + 4.0 * 10.0;
        let cab_w = reels_w + 120.0;
        let cab_h = top_h + cell * 3.0 + 40.0;
        let block = cab_h + controls_h;
        let top = area.top() + ((area.height() - block) / 2.0).max(20.0);
        let cab = Rect::from_center_size(pos2(area.center().x, top + cab_h / 2.0), vec2(cab_w, cab_h));
        p.rect_filled(cab.translate(vec2(0.0, 10.0)), theme::r(28), Color32::from_black_alpha(140));
        vgrad_round(&p, cab, 28.0, Color32::from_rgb(0x5a, 0x10, 0x16), Color32::from_rgb(0x24, 0x06, 0x0a));
        p.rect_stroke(cab, theme::r(28), Stroke::new(3.0f32, col::GOLD), StrokeKind::Inside);
        p.rect_stroke(cab.shrink(7.0), theme::r(22), Stroke::new(1.0f32, col::GOLD_DARK), StrokeKind::Inside);
        // Lights round the top.
        for i in 0..24 {
            let x = cab.left() + 30.0 + i as f32 * (cab.width() - 60.0) / 23.0;
            let on = ((t * 6.0) as i32 + i) % 3 == 0;
            p.circle_filled(pos2(x, cab.top() + 14.0), 4.0, if on { col::GOLD2 } else { col::GOLD_DARK });
        }
        // The marquee and the jackpot.
        let title = if free_round { "FREE SPINS".to_string() } else { "PYR SEVENS".to_string() };
        glow_text(&p, pos2(cab.center().x, cab.top() + 42.0), Align2::CENTER_CENTER, &title, theme::bold_font(30.0), col::GOLD2, col::EMBER.gamma_multiply(0.5));
        let jp = self.poker.casino.slots.shown_jackpot(c.jackpot);
        let jr = Rect::from_center_size(pos2(cab.center().x, cab.top() + 76.0), vec2(360.0, 30.0));
        p.rect_filled(jr, theme::r(15), Color32::from_black_alpha(170));
        p.rect_stroke(jr, theme::r(15), Stroke::new(1.0f32, col::GOLD_DARK), StrokeKind::Inside);
        p.text(jr.center(), Align2::CENTER_CENTER, format!("PHOENIX JACKPOT  {}", chips_text(jp)), theme::bold_font(15.0), col::GOLD);

        // The reels.
        let reels = Rect::from_min_size(pos2(cab.center().x - reels_w / 2.0, cab.top() + top_h), vec2(reels_w, cell * 3.0));
        let frame = reels.expand(10.0);
        p.rect_filled(frame, theme::r(14), Color32::from_rgb(0x1a, 0x10, 0x0c));
        let ui_s = &self.poker.casino.slots;
        let showing = ui_s.showing.clone();
        let reel_rects: Vec<Rect> = (0..REELS).map(|i| Rect::from_min_size(pos2(reels.left() + i as f32 * (cell + 10.0), reels.top()), vec2(cell, cell * 3.0))).collect();
        for (i, rr) in reel_rects.iter().enumerate() {
            vgrad(&p, *rr, Color32::from_rgb(0x1c, 0x0c, 0x0c), Color32::from_rgb(0x34, 0x16, 0x10));
            let clip = p.with_clip_rect(*rr);
            let reel = self.poker.casino.slots.reels[i];
            let pos = reel.pos(now);
            let len = STRIPS[i].len() as i32;
            let base = pos.floor() as i32;
            let frac = pos - pos.floor();
            for row in -1..=3 {
                let idx = (base + row).rem_euclid(len) as usize;
                let y = rr.top() + (row as f32 - frac) * cell;
                let sr = Rect::from_min_size(pos2(rr.left(), y), vec2(cell, cell)).shrink(cell * 0.08);
                if reel.blurred(now) {
                    // A blur while it spins fast.
                    art::paint_symbol(&clip, sr.translate(vec2(0.0, -cell * 0.12)), STRIPS[i][idx], Color32::from_white_alpha(70));
                    art::paint_symbol(&clip, sr, STRIPS[i][idx], Color32::from_white_alpha(170));
                } else {
                    art::paint_symbol(&clip, sr, STRIPS[i][idx], Color32::WHITE);
                }
            }
            // Shading top and bottom, like a drum.
            vgrad(&clip, Rect::from_min_size(rr.min, vec2(cell, cell * 0.35)), Color32::from_black_alpha(110), Color32::TRANSPARENT);
            vgrad(&clip, Rect::from_min_max(pos2(rr.left(), rr.bottom() - cell * 0.35), rr.max), Color32::TRANSPARENT, Color32::from_black_alpha(110));
        }
        p.rect_stroke(frame, theme::r(14), Stroke::new(3.0f32, col::GOLD_DARK), StrokeKind::Inside);

        // What landed (its wins are traced below), and the lines that won.
        let display = self.poker.casino.slots.display.filter(|_| self.poker.casino.slots.next_start.is_none_or(|t| now < t));
        let won: Vec<usize> = match (&showing, display) {
            (Some(r), Some((i, _))) => match &r.outcome {
                pyr_poker::casino::Outcome::Slots(o) => o.spins.get(i).map(|sp| sp.wins.iter().map(|w| w.line as usize).collect()).unwrap_or_default(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        };
        // Line numbers at the sides, 1 to 10 on the left and 11 to 20 on the right, each by the
        // row its line starts (or ends) on, spread out within the row. A line that won lights up.
        for side in 0..2usize {
            for row in 0..ROWS {
                let ls: Vec<usize> = (side * 10..side * 10 + 10).filter(|l| PAYLINES[*l][if side == 0 { 0 } else { REELS - 1 }] as usize == row).collect();
                let n = ls.len().max(1) as f32;
                let gap = (cell / n).min(24.0);
                let rad = (gap / 2.0 - 1.0).min(9.5);
                for (k, l) in ls.iter().enumerate() {
                    let y = reels.top() + (row as f32 + 0.5) * cell + (k as f32 - (n - 1.0) / 2.0) * gap;
                    let x = if side == 0 { reels.left() - 33.0 } else { reels.right() + 33.0 };
                    let lc = line_color(*l);
                    let lit = won.contains(l);
                    if lit {
                        p.circle_filled(pos2(x, y), rad + 4.0, lc.gamma_multiply(0.35));
                    }
                    p.circle_filled(pos2(x, y), rad, if lit { lc } else { lc.gamma_multiply(0.55) });
                    p.text(pos2(x, y), Align2::CENTER_CENTER, format!("{}", l + 1), theme::bold_font((rad * 1.05).max(8.0)), Color32::BLACK);
                }
            }
        }

        // Wins traced one by one.
        if let (Some(r), Some((shown_i, shown_since))) = (&showing, display)
            && let pyr_poker::casino::Outcome::Slots(o) = &r.outcome
            && let Some(sp) = o.spins.get(shown_i)
        {
            let since = (now - shown_since).as_secs_f32();
            if !sp.wins.is_empty() {
                let which = if since < 0.9 { None } else { Some(((since - 0.9) / 1.1) as usize % sp.wins.len()) };
                for (k, w) in sp.wins.iter().enumerate() {
                    if which.is_some_and(|x| x != k) {
                        continue;
                    }
                    let lc = line_color(w.line as usize);
                    let pts: Vec<Pos2> = (0..REELS).map(|i| pos2(reel_rects[i].center().x, reels.top() + (PAYLINES[w.line as usize][i] as f32 + 0.5) * cell)).collect();
                    p.add(Shape::line(pts.clone(), Stroke::new(7.0f32, Color32::from_black_alpha(120))));
                    p.add(Shape::line(pts.clone(), Stroke::new(4.0f32, lc)));
                    for i in 0..w.count as usize {
                        let cr = Rect::from_center_size(pts[i], vec2(cell * 0.94, cell * 0.94));
                        p.rect_stroke(cr, theme::r(10), Stroke::new(3.0f32, lc), StrokeKind::Inside);
                    }
                    if which.is_some() {
                        let msg = format!("Line {}: {} × {} pays {}", w.line + 1, w.count, sl::symbol_name(w.symbol), chips_text(w.pay));
                        let mr = Rect::from_center_size(pos2(reels.center().x, reels.bottom() + 24.0), vec2(26.0 + msg.len() as f32 * 9.6, 34.0));
                        p.rect_filled(mr, theme::r(17), Color32::from_black_alpha(210));
                        p.text(mr.center(), Align2::CENTER_CENTER, msg, theme::bold_font(17.0), col::GOLD2);
                    }
                }
            }
            if sp.stars >= 3 {
                let g = sp.grid();
                for (i, reel) in g.iter().enumerate() {
                    for (row, sym) in reel.iter().enumerate() {
                        if *sym == sl::STAR {
                            let cr = Rect::from_min_size(pos2(reel_rects[i].left(), reels.top() + row as f32 * cell), vec2(cell, cell));
                            p.rect_stroke(cr.shrink(3.0), theme::r(10), Stroke::new(3.0 + (t * 8.0).sin().abs() * 2.0, col::GOLD2), StrokeKind::Inside);
                        }
                    }
                }
            }
        }

        // After a spin stops: count its win, then free spins or the end of the round.
        self.slots_after_stop(now, s);
        // A spin the house refused (not enough chips, say): the reels come to rest.
        if let Some(since) = self.poker.casino.slots.waiting
            && self.poker.casino.pending.is_none()
            && since.elapsed() > Duration::from_millis(300)
        {
            let ui_s = &mut self.poker.casino.slots;
            ui_s.waiting = None;
            for r in &mut ui_s.reels {
                let pos = r.pos(now).round();
                *r = Reel::still(pos);
            }
            ui_s.auto.stop();
        }

        // Free spins' banner.
        if let Some((since, text)) = self.poker.casino.slots.banner.clone() {
            let k = (now - since).as_secs_f32();
            if k < 2.2 {
                let a = (k / 0.25).min(1.0) * ((2.2 - k) / 0.4).clamp(0.0, 1.0);
                let br = Rect::from_center_size(reels.center(), vec2(reels.width() * 0.8, 110.0));
                p.rect_filled(br, theme::r(20), Color32::from_rgba_unmultiplied(0x30, 0x0a, 0x50, (225.0 * a) as u8));
                p.rect_stroke(br, theme::r(20), Stroke::new(2.0f32, col::GOLD.gamma_multiply(a)), StrokeKind::Inside);
                glow_text(&p, br.center() - vec2(0.0, 12.0), Align2::CENTER_CENTER, &text, theme::bold_font(40.0), col::GOLD2.gamma_multiply(a), Color32::from_rgba_unmultiplied(0xc0, 0x60, 0xff, (120.0 * a) as u8));
                p.text(br.center() + vec2(0.0, 30.0), Align2::CENTER_CENTER, "every win in them pays triple", theme::bold_font(15.0), Color32::WHITE.gamma_multiply(a));
            } else {
                self.poker.casino.slots.banner = None;
            }
        }

        // The controls.
        let bar = Rect::from_min_size(pos2(cab.left(), cab.bottom() + 16.0), vec2(cab.width(), controls_h - 16.0));
        self.slots_controls(ui, s, bar);
        self.slots_paytable(ui.ctx());
    }

    fn slots_after_stop(&mut self, now: Instant, s: &ServerView) {
        // Autoplay spins again.
        if self.poker.casino.slots.auto.on() && self.poker.casino.slots.next_at.is_some_and(|t| now >= t) && !self.poker.casino.busy() {
            self.poker.casino.slots.next_at = None;
            self.slots_spin(s);
            return;
        }
        let Some(end) = self.poker.casino.slots.stopped_at else { return };
        if now < end || self.poker.casino.slots.counted == Some(self.poker.casino.slots.spin) {
            return;
        }
        let Some(r) = self.poker.casino.slots.showing.clone() else { return };
        let pyr_poker::casino::Outcome::Slots(o) = &r.outcome else { return };
        let i = self.poker.casino.slots.spin;
        let Some(sp) = o.spins.get(i) else { return };
        // Count it once, as it stops, and show its wins.
        self.poker.casino.slots.counted = Some(i);
        self.poker.casino.slots.display = Some((i, now));
        self.poker.casino.slots.next_start = None;
        self.poker.casino.slots.round_win += sp.pay;
        if sp.pay > 0 {
            let big = sp.pay >= o.line_bet * LINES as u64 * 5;
            self.poker.casino.sound_in(0.0, if big { Cue::WinBig } else { Cue::WinSmall }, 0.0, 0.7);
        }
        let next = i + 1;
        if let Some(fs) = o.spins.get(next) {
            // On to the next free spin, after the wins have had a moment (and the banner, if
            // these stars started them).
            let pause = if sp.free_spins > 0 { 2.4 } else if sp.pay > 0 { 1.4 } else { 0.6 };
            if sp.free_spins > 0 {
                let left = o.spins.len() - next;
                self.poker.casino.slots.banner = Some((now, if i == 0 { format!("{left} FREE SPINS") } else { format!("+{} FREE SPINS", sp.free_spins) }));
                self.poker.casino.sound_in(0.0, Cue::Jackpot, 0.0, 0.5);
            }
            let ui = &mut self.poker.casino.slots;
            ui.spin = next;
            let start = now + Duration::from_secs_f32(if ui.turbo { pause * 0.5 } else { pause });
            ui.next_start = Some(start);
            for r in &mut ui.reels {
                r.start(start, SPEED);
            }
            let spin_for = if ui.turbo { 0.3 } else { 0.7 };
            let stops = fs.stops;
            let ends = ui.land(stops, start + Duration::from_secs_f32(spin_for));
            self.slots_landing(ends, now);
            self.poker.casino.sound_in(start.saturating_duration_since(now).as_secs_f32(), Cue::ReelSpin, 0.0, 0.5);
            return;
        }
        // The round's over: the win goes on the balance.
        self.poker.casino.settle_balance();
        let total = o.paid;
        let jackpot = o.jackpot_won > 0;
        self.poker.casino.celebrate(total, o.line_bet * LINES as u64, jackpot, 0.2);
        if total > 0 {
            let lines = o.spins.iter().map(|s| s.wins.len()).sum::<usize>();
            self.poker.casino.announce(format!("WIN {}", chips_text(total)), if lines > 1 { format!("{lines} winning lines") } else { String::new() }, col::GOLD2, (0.5, 0.16), 0.15);
        }
        let big = total >= o.line_bet * LINES as u64 * 10 || jackpot;
        let ui = &mut self.poker.casino.slots;
        // Autoplay: one spin fewer to go (a big win stops it, unless that's switched off).
        if ui.auto.spun(big) {
            ui.next_at = Some(now + Duration::from_secs_f32(if total > 0 { 1.2 } else { 0.45 } * if ui.turbo { 0.5 } else { 1.0 }));
        }
    }

    fn slots_spin(&mut self, s: &ServerView) {
        let c = &s.poker.casino;
        let Some(commit) = c.commit else { return };
        let ui = &self.poker.casino.slots;
        let total = ui.line_bet * LINES as u64;
        if total > super::chips_now(s) {
            self.poker.casino.slots.auto.stop();
            return;
        }
        if self.poker.casino.busy() || ui.running(Instant::now()) {
            return;
        }
        let now = Instant::now();
        let ui = &mut self.poker.casino.slots;
        ui.start_spinning(now);
        ui.waiting = Some(now);
        ui.showing = None;
        ui.spin = 0;
        ui.round_win = 0;
        ui.counted = None;
        ui.display = None;
        ui.next_start = None;
        ui.stopped_at = None;
        self.poker.casino.celebration = None;
        self.poker.casino.pending = Some(now);
        self.poker.casino.sound_in(0.0, Cue::ReelSpin, 0.0, 0.6);
        self.engine.casino_play(&s.key, commit, Bet::Slots { line_bet: self.poker.casino.slots.line_bet });
    }

    fn slots_controls(&mut self, ui: &mut egui::Ui, s: &ServerView, bar: Rect) {
        let p = ui.painter().clone();
        p.rect_filled(bar, theme::r(18), Color32::from_rgb(0x14, 0x0c, 0x10));
        p.rect_stroke(bar, theme::r(18), Stroke::new(1.0f32, col::GOLD_DARK), StrokeKind::Inside);
        let cy = bar.center().y;
        // Line bet.
        let lb = self.poker.casino.slots.line_bet;
        p.text(pos2(bar.left() + 24.0, bar.top() + 22.0), Align2::LEFT_CENTER, "LINE BET", theme::bold_font(11.5), col::MUTED);
        let minus = Rect::from_center_size(pos2(bar.left() + 40.0, cy + 12.0), vec2(34.0, 34.0));
        let plus = Rect::from_center_size(pos2(bar.left() + 150.0, cy + 12.0), vec2(34.0, 34.0));
        for (r, label, dir) in [(minus, "−", -1i32), (plus, "+", 1)] {
            let resp = ui.interact(r, ui.id().with(("lb", label)), Sense::click());
            p.circle_filled(r.center(), 17.0, if resp.hovered() { col::GOLD } else { col::GOLD_DARK });
            p.text(r.center(), Align2::CENTER_CENTER, label, theme::bold_font(20.0), Color32::BLACK);
            if resp.clicked() {
                let i = LINE_BETS.iter().position(|x| *x == lb).unwrap_or(2) as i32;
                let j = (i + dir).clamp(0, LINE_BETS.len() as i32 - 1) as usize;
                self.poker.casino.slots.line_bet = LINE_BETS[j];
            }
        }
        p.text(pos2(bar.left() + 95.0, cy + 12.0), Align2::CENTER_CENTER, chips_text(lb), theme::bold_font(22.0), col::GOLD2);
        p.text(pos2(bar.left() + 196.0, bar.top() + 22.0), Align2::LEFT_CENTER, "TOTAL BET", theme::bold_font(11.5), col::MUTED);
        p.text(pos2(bar.left() + 196.0, cy + 12.0), Align2::LEFT_CENTER, chips_text(lb * LINES as u64), theme::bold_font(22.0), col::TEXT);
        // The spin button.
        let spin_c = pos2(bar.center().x, cy);
        let spin_r = Rect::from_center_size(spin_c, vec2(92.0, 92.0));
        let resp = ui.interact(spin_r, ui.id().with("spin"), Sense::click());
        let busy = self.poker.casino.busy() || self.poker.casino.slots.running(Instant::now());
        let auto = self.poker.casino.slots.auto;
        p.circle_filled(spin_c + vec2(0.0, 5.0), 46.0, Color32::from_black_alpha(140));
        radial(&p, spin_c, 46.0, if resp.hovered() && !busy { Color32::from_rgb(0xff, 0xd0, 0x60) } else { Color32::from_rgb(0xff, 0xb0, 0x30) }, Color32::from_rgb(0xc0, 0x40, 0x10));
        p.circle_stroke(spin_c, 46.0, Stroke::new(3.0f32, col::GOLD2));
        let label = if auto.on() { "STOP" } else { "SPIN" };
        p.text(spin_c - vec2(0.0, if auto.on() { 6.0 } else { 0.0 }), Align2::CENTER_CENTER, label, theme::bold_font(22.0), Color32::from_rgb(0x3a, 0x10, 0x04));
        if auto.on() {
            p.text(spin_c + vec2(0.0, 16.0), Align2::CENTER_CENTER, format!("auto {}", auto.left_text()), theme::bold_font(11.0), Color32::from_rgb(0x3a, 0x10, 0x04));
        }
        let space = ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) && !ui.ctx().wants_keyboard_input() && self.poker.casino.info.is_none();
        if resp.on_hover_text("Spin (Space)").clicked() || space {
            if auto.on() {
                self.poker.casino.slots.auto.stop();
            } else if !busy {
                self.slots_spin(s);
            }
        }
        // Win, autoplay, turbo, paytable.
        let win = self.poker.casino.slots.round_win;
        let wx = bar.center().x + 80.0;
        p.text(pos2(wx, bar.top() + 22.0), Align2::LEFT_CENTER, "WIN", theme::bold_font(11.5), col::MUTED);
        p.text(pos2(wx, cy + 12.0), Align2::LEFT_CENTER, if win > 0 { chips_text(win) } else { "–".into() }, theme::bold_font(24.0), if win > 0 { col::WIN } else { col::MUTED });
        let mut bx = bar.right() - 20.0;
        let mut small = |ui: &mut egui::Ui, id: &str, text: &str, on: bool, tip: &str| -> egui::Response {
            let w = 18.0 + text.chars().count() as f32 * 8.0;
            let r = Rect::from_min_size(pos2(bx - w, cy - 15.0), vec2(w, 30.0));
            bx -= w + 8.0;
            let resp = ui.interact(r, ui.id().with(("slot-btn", id)), Sense::click());
            ui.painter().rect_filled(r, theme::r(15), if on { col::GOLD } else if resp.hovered() { Color32::from_white_alpha(40) } else { Color32::from_white_alpha(18) });
            ui.painter().text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(12.5), if on { Color32::BLACK } else { col::TEXT });
            resp.on_hover_text(tip)
        };
        if small(ui, "info", "Info", self.poker.casino.info.is_some(), "What it pays back, its statistics, and how it plays").clicked() {
            self.info_toggle();
        }
        if small(ui, "paytable", "Paytable", self.poker.casino.slots.paytable, "What each symbol pays, and the lines").clicked() {
            self.poker.casino.slots.paytable = !self.poker.casino.slots.paytable;
        }
        if small(ui, "turbo", "Turbo", self.poker.casino.slots.turbo, "Faster spins").clicked() {
            self.poker.casino.slots.turbo = !self.poker.casino.slots.turbo;
        }
        // Autoplay: its menu (how many spins, or until stopped); running, a click stops it.
        let auto_label = if auto.on() { format!("Auto {}", auto.left_text()) } else { "Auto ⌄".into() };
        let tip = if auto.on() { "Stop spinning by itself" } else { "Spin by itself: 10 to 1,000 times, or until you stop it" };
        let resp = small(ui, "auto", &auto_label, auto.on(), tip);
        if auto.on() {
            if resp.clicked() {
                self.poker.casino.slots.auto.stop();
            }
        } else if let Some(n) = super::auto::menu(&resp, &mut self.poker.casino.slots.auto) {
            self.poker.casino.slots.auto.left = n;
            if !busy {
                self.poker.casino.slots.next_at = Some(Instant::now());
            }
        }
        if super::chips_now(s) < lb * LINES as u64 {
            p.text(pos2(bar.center().x, bar.bottom() - 10.0), Align2::CENTER_CENTER, "Not enough chips for this bet: lower it, or take the daily chips in the lobby.", theme::font(11.5), col::LOSE);
        }
    }

    fn slots_paytable(&mut self, ctx: &egui::Context) {
        if !self.poker.casino.slots.paytable {
            return;
        }
        let mut open = true;
        let lb = self.poker.casino.slots.line_bet;
        egui::Window::new("Pyr Sevens: what pays").collapsible(false).resizable(false).default_width(560.0).open(&mut open).show(ctx, |ui| {
            ui.label(RichText::new(format!("At {} a line ({} a spin). Wins pay left to right on any of the 20 lines.", chips_text(lb), chips_text(lb * LINES as u64))).color(theme::MUTED()));
            ui.add_space(6.0);
            egui::Grid::new("paytable").num_columns(4).spacing([22.0, 6.0]).show(ui, |ui| {
                ui.label(RichText::new("Symbol").strong());
                ui.label(RichText::new("3 in a row").strong());
                ui.label(RichText::new("4").strong());
                ui.label(RichText::new("5").strong());
                ui.end_row();
                for sym in (0..=sl::WILD).rev() {
                    let (r, _) = ui.allocate_exact_size(vec2(150.0, 46.0), Sense::hover());
                    art::paint_symbol(ui.painter(), Rect::from_min_size(r.min, vec2(46.0, 46.0)), sym, Color32::WHITE);
                    ui.painter().text(r.min + vec2(54.0, 23.0), Align2::LEFT_CENTER, sl::symbol_name(sym), theme::font(14.0), theme::TEXT());
                    for k in 0..3 {
                        let pay = sl::PAYS[sym as usize][k] * lb;
                        let extra = if sym == sl::WILD && k == 2 { " or the jackpot" } else { "" };
                        ui.label(RichText::new(format!("{}{extra}", chips_text(pay))).size(14.0).strong());
                    }
                    ui.end_row();
                }
            });
            ui.add_space(8.0);
            let (r, _) = ui.allocate_exact_size(vec2(46.0, 46.0), Sense::hover());
            art::paint_symbol(ui.painter(), r, sl::STAR, Color32::WHITE);
            ui.label(RichText::new(format!(
                "Stars pay anywhere: 3 pay {} and start {} free spins, 4 pay {} and start {}, 5 pay {} and start {}. In free spins every win pays triple, and stars add more.",
                chips_text(sl::STAR_PAYS[0] * lb * LINES as u64),
                sl::STAR_SPINS[0],
                chips_text(sl::STAR_PAYS[1] * lb * LINES as u64),
                sl::STAR_SPINS[1],
                chips_text(sl::STAR_PAYS[2] * lb * LINES as u64),
                sl::STAR_SPINS[2]
            )));
            ui.label(RichText::new("The flame is wild: it stands in for any symbol but the star. Five flames on a line win the Phoenix Jackpot: all of it at the top line bet (500), a share at less."));
            ui.add_space(6.0);
            let (r, _) = ui.allocate_exact_size(vec2(520.0, 120.0), Sense::hover());
            let p = ui.painter();
            for l in 0..LINES {
                let col_x = r.left() + (l % 10) as f32 * 52.0;
                let row_y = r.top() + (l / 10) as f32 * 60.0;
                let cellw = 8.0;
                for reel in 0..REELS {
                    for row in 0..ROWS {
                        let on = PAYLINES[l][reel] as usize == row;
                        let cr = Rect::from_min_size(pos2(col_x + reel as f32 * (cellw + 1.0), row_y + 14.0 + row as f32 * (cellw + 1.0)), vec2(cellw, cellw));
                        p.rect_filled(cr, 1, if on { line_color(l) } else { Color32::from_white_alpha(25) });
                    }
                }
                p.text(pos2(col_x, row_y), Align2::LEFT_TOP, format!("{}", l + 1), theme::font(10.0), theme::MUTED());
            }
        });
        if !open {
            self.poker.casino.slots.paytable = false;
        }
    }
}

/// Each line's color.
pub fn line_color(l: usize) -> Color32 {
    const C: [Color32; 10] = [
        Color32::from_rgb(0xff, 0xd2, 0x4a),
        Color32::from_rgb(0x4a, 0xd8, 0xff),
        Color32::from_rgb(0xff, 0x6a, 0x8a),
        Color32::from_rgb(0x7a, 0xff, 0x8a),
        Color32::from_rgb(0xc0, 0x8a, 0xff),
        Color32::from_rgb(0xff, 0x9a, 0x3a),
        Color32::from_rgb(0x3a, 0xff, 0xd8),
        Color32::from_rgb(0xff, 0x5a, 0xd8),
        Color32::from_rgb(0xd8, 0xff, 0x4a),
        Color32::from_rgb(0x6a, 0x9a, 0xff),
    ];
    C[l % C.len()]
}
