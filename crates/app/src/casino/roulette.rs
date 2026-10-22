//! Roulette at the app: the wheel turning and the ball circling, slowing, dropping and
//! rattling into its pocket; and the layout, where a click puts the chip picked on a number,
//! on the line between two (a split), at a corner, at the end of a row (a street or six), or
//! on the outside bets.

use std::time::Instant;

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::casino::roulette::{self as rl, Chip, Spot, WHEEL};
use pyr_poker::casino::{Bet, Outcome};
use pyr_poker::net::RoundView;

use super::{chip_stack, col, glow_text, radial, vgrad};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

use pyr_engine::poker_sfx::roulette as rt;

const TAU: f32 = std::f32::consts::TAU;

#[derive(Default)]
pub struct RouletteUi {
    /// The bets on the layout, in the order placed (Undo takes the last off).
    pub chips: Vec<Chip>,
    last: Vec<Chip>,
    /// The spin being shown: the round, when it started, where the wheel was, and whether it's
    /// been settled (the number in the history, the win counted).
    spin: Option<(RoundView, Instant, f32, bool)>,
    /// The wheel's last push (when, and where it was then): it keeps turning between spins.
    wheel_push: Option<(Instant, f32)>,
    /// The last numbers.
    pub history: Vec<u8>,
}

impl RouletteUi {
    fn total(&self) -> u64 {
        self.chips.iter().map(|c| c.chips).sum()
    }

    fn on(&self, spot: &Spot) -> u64 {
        self.chips.iter().filter(|c| c.spot == *spot).map(|c| c.chips).sum()
    }

    /// The layout's bets merged by spot (what's sent).
    fn merged(&self) -> Vec<Chip> {
        let mut out: Vec<Chip> = Vec::new();
        for c in &self.chips {
            match out.iter_mut().find(|o| o.spot == c.spot) {
                Some(o) => o.chips += c.chips,
                None => out.push(c.clone()),
            }
        }
        out
    }
}

/// The wheel's turning `t` seconds after it was last pushed (radians): a push at each spin
/// sets it going at 2.4 rad/s (about 23 turns a minute), and it slows toward the 1.2 rad/s it
/// keeps between spins, as a real wheel's rotor does; it never stops dead.
pub(super) fn wheel_turn(t: f32) -> f32 {
    1.2 * t + 36.0 * (1.0 - (-t / 30.0).exp())
}

fn pocket_angle(pocket: i32) -> f32 {
    (pocket as f32 + 0.5) / 37.0 * TAU
}

/// The ball's hops once it's down, in pockets (mostly on the way it was going, against the
/// wheel's turning), picked by the round so spins don't all look alike. The last lands in its
/// pocket.
fn hops(r: &RoundView) -> [i32; 4] {
    hops_by(r.server_seed[1])
}

/// The ball's hops picked by a byte of the spin's seed.
pub(super) fn hops_by(b: u8) -> [i32; 4] {
    const WAYS: [[i32; 4]; 6] = [[-5, -3, -1, 1], [-6, -2, 1, -1], [-4, -3, -2, 0], [-7, -3, -1, 0], [-3, -4, 1, -1], [-5, -2, -2, 1]];
    WAYS[b as usize % WAYS.len()]
}

/// The ball at `t` seconds into a spin (the wheel at `a0` when it was pushed): its angle, and
/// how far out it is (1 on the track, 0 down in a pocket, above 0 in the air between pockets).
/// See `roulette` in the sounds for the timing it keeps.
pub(super) fn ball(a0: f32, pocket: usize, hops: [i32; 4], t: f32) -> (f32, f32) {
    let wheel = |t: f32| a0 + wheel_turn(t);
    // The pockets it lands in: the first, then after each hop; the last is its own.
    let mut at = [0i32; 5];
    at[4] = pocket as i32;
    for i in (0..4).rev() {
        at[i] = at[i + 1] - hops[i];
    }
    let lands = rt::landings();
    if t < rt::POCKETS_S {
        // Round the track (against the wheel), slowing, then down the cone: it reaches the
        // pockets where its first one is just then.
        let a = wheel(rt::POCKETS_S) + pocket_angle(at[0]) + rt::ball_run(rt::POCKETS_S) - rt::ball_run(t);
        let out = if t < rt::LEAVE_S {
            1.0
        } else if t < rt::DIAMOND_S {
            let k = (t - rt::LEAVE_S) / (rt::DIAMOND_S - rt::LEAVE_S);
            1.0 - 0.45 * k * k
        } else {
            // Knocked up off the diamond, then down into the pockets.
            let k = (t - rt::DIAMOND_S) / (rt::POCKETS_S - rt::DIAMOND_S);
            0.55 * (1.0 - k * k) + 0.16 * (std::f32::consts::PI * k).sin() * (1.0 - k)
        };
        return (a, out);
    }
    // Hopping from pocket to pocket (carried round by the wheel all the while).
    let mut from = rt::POCKETS_S;
    for (i, (d, h)) in rt::HOPS.iter().enumerate() {
        if t < lands[i] {
            let k = ((t - from) / d).clamp(0.0, 1.0);
            let rel = pocket_angle(at[i]) + (pocket_angle(at[i + 1]) - pocket_angle(at[i])) * k;
            return (wheel(t) + rel, h / 0.26 * 4.0 * k * (1.0 - k));
        }
        from = lands[i];
    }
    // In its pocket: a last little rattle, then still.
    let since = t - lands[3];
    let rattle = if t < rt::SETTLED_S { 0.012 * (since * 70.0).sin().abs() * (1.0 - since / (rt::SETTLED_S - lands[3])) } else { 0.0 };
    (wheel(t) + pocket_angle(pocket as i32), rattle)
}

impl PyrApp {
    pub(super) fn roulette_round(&mut self, r: &RoundView) {
        let Outcome::Roulette(_) = &r.outcome else { return };
        let ui = &mut self.poker.casino.roulette;
        // The croupier pushes the wheel and launches the ball.
        let now = Instant::now();
        let at = ui.wheel_push.map(|(t, a)| a + wheel_turn((now - t).as_secs_f32())).unwrap_or(0.0) % TAU;
        ui.wheel_push = Some((now, at));
        ui.spin = Some((r.clone(), now, at, false));
        self.poker.casino.sound_in(0.0, Cue::BallRoll, 0.0, 0.7);
        self.poker.casino.sound_in(rt::DIAMOND_S, Cue::BallDrop, 0.0, 0.8);
    }

    pub(super) fn roulette_page(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let now = Instant::now();
        let p = ui.painter().clone();
        vgrad(&p, area, Color32::from_rgb(0x0c, 0x24, 0x18), Color32::from_rgb(0x04, 0x10, 0x0a));
        radial(&p, area.center(), area.width() * 0.6, Color32::from_rgba_unmultiplied(0x30, 0x90, 0x60, 40), Color32::TRANSPARENT);
        let bar_h = 86.0;
        let play = Rect::from_min_max(area.min + vec2(20.0, 16.0), area.max - vec2(20.0, bar_h + 10.0));
        // The layout along the bottom, as big as fits; the wheel above it on the left; the
        // result, the last numbers and the bet under the pointer beside the wheel.
        let cw = ((play.width() - 12.0) / 14.0).min(play.height() * 0.52 / 3.68).min(92.0);
        let lay = Rect::from_min_size(pos2(play.center().x - cw * 7.0, play.bottom() - cw * 3.68 - 6.0), vec2(cw * 14.0, cw * 3.68));
        let above = Rect::from_min_max(pos2(lay.left(), play.top()), pos2(lay.right(), lay.top() - 22.0));
        let wheel_d = above.height().min(above.width() * 0.46);
        let wc = pos2(above.left() + wheel_d / 2.0 + 6.0, above.center().y);
        let side = Rect::from_min_max(pos2(wc.x + wheel_d / 2.0 + 40.0, above.top() + 6.0), above.max);
        // Where the spin is.
        let (spin_t, spin, settled) = match &self.poker.casino.roulette.spin {
            Some((r, start, a0, settled)) => ((now - *start).as_secs_f32(), Some((r.clone(), *a0)), *settled),
            None => (0.0, None, true),
        };
        let done = spin.as_ref().is_none_or(|_| spin_t >= rt::RESULT_S);
        // The wheel turns all the time (slowly between spins).
        let push = *self.poker.casino.roulette.wheel_push.get_or_insert((now, 0.0));
        let wheel_a = push.1 + wheel_turn((now - push.0).as_secs_f32());
        draw_wheel(&p, wc, wheel_d / 2.0, wheel_a);
        if done {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
        } else {
            ui.ctx().request_repaint();
        }
        // The ball.
        let mut result: Option<(u8, u64)> = None;
        if let Some((r, a0)) = &spin
            && let Outcome::Roulette(o) = &r.outcome
        {
            let (a, out) = ball(*a0, o.pocket as usize, hops(r), spin_t);
            let rad = wheel_d / 2.0;
            let br = rad * (0.62 + 0.26 * out);
            p.circle_filled(wc + vec2(a.cos(), a.sin()) * br + vec2(1.5, 2.5), rad * 0.035, Color32::from_black_alpha(120));
            p.circle_filled(wc + vec2(a.cos(), a.sin()) * br, rad * 0.035, Color32::WHITE);
            if done && !settled {
                let ui_s = &mut self.poker.casino.roulette;
                if let Some(sp) = ui_s.spin.as_mut() {
                    sp.3 = true;
                }
                ui_s.history.insert(0, o.number);
                ui_s.history.truncate(40);
                let (paid, wagered) = (o.paid, o.chips.iter().map(|c| c.chips).sum::<u64>());
                if paid > 0 {
                    self.poker.casino.sound_in(0.0, Cue::WinSmall, 0.0, 0.8);
                    self.poker.casino.sound_in(0.15, Cue::Chips, 0.0, 0.7);
                }
                self.poker.casino.settle_balance();
                self.poker.casino.celebrate(paid, wagered, false, 0.1);
            }
            if done {
                // The number, big, in the middle of the wheel.
                let color = number_color(o.number);
                p.circle_filled(wc, 48.0, color);
                p.circle_stroke(wc, 48.0, Stroke::new(3.0f32, col::GOLD));
                glow_text(&p, wc, Align2::CENTER_CENTER, &o.number.to_string(), theme::bold_font(40.0), Color32::WHITE, Color32::from_black_alpha(120));
                result = Some((o.number, o.paid));
            }
        }
        // Beside the wheel, big enough to read on a stream: the result (or what's going on),
        // the last numbers, which have been hot and cold, and how red and black have fallen.
        let big = (side.width() / 9.0).clamp(30.0, 50.0);
        let mut y = side.top() + 6.0;
        match result {
            Some((n, won)) => {
                let r_c = (big * 0.95).round();
                let cc = pos2(side.left() + r_c, y + r_c);
                p.circle_filled(cc + vec2(2.0, 4.0), r_c, Color32::from_black_alpha(120));
                p.circle_filled(cc, r_c, number_color(n));
                p.circle_stroke(cc, r_c, Stroke::new(3.0f32, col::GOLD));
                glow_text(&p, cc, Align2::CENTER_CENTER, &n.to_string(), theme::bold_font(big * 0.95), Color32::WHITE, Color32::from_black_alpha(120));
                let x = cc.x + r_c + 18.0;
                let name = if n == 0 { "ZERO".to_string() } else { format!("{n} {}", rl::color_name(n).to_uppercase()) };
                glow_text(&p, pos2(x, cc.y - big * 0.42), Align2::LEFT_CENTER, &name, theme::bold_font(big * 0.85), Color32::WHITE, number_color(n).gamma_multiply(0.8));
                let traits = if n == 0 {
                    "Only straight up, splits and the like win".to_string()
                } else {
                    format!("{}  ·  {}  ·  {} dozen", if n % 2 == 0 { "Even" } else { "Odd" }, if n <= 18 { "1 to 18" } else { "19 to 36" }, ["1st", "2nd", "3rd"][((n - 1) / 12) as usize])
                };
                p.text(pos2(x, cc.y + big * 0.12), Align2::LEFT_CENTER, traits, theme::font(big * 0.34), col::MUTED);
                let (text, color) = if won > 0 { (format!("You won {}", chips_text(won)), col::WIN) } else { ("No win this spin".to_string(), col::MUTED) };
                p.text(pos2(x, cc.y + big * 0.62), Align2::LEFT_CENTER, text, theme::bold_font(big * if won > 0 { 0.56 } else { 0.42 }), color);
                y = cc.y + r_c + 18.0;
            }
            None => {
                let text = if spin.is_some() { "No more bets…" } else { "Place your bets" };
                glow_text(&p, pos2(side.left(), y + big * 0.6), Align2::LEFT_CENTER, text, theme::bold_font(big * 0.85), col::GOLD2, Color32::from_black_alpha(140));
                p.text(pos2(side.left(), y + big * 1.35), Align2::LEFT_CENTER, "Single zero: the house keeps 2.7%", theme::font(big * 0.36), col::MUTED);
                y += big * 1.95 + 18.0;
            }
        }
        let hist = self.poker.casino.roulette.history.clone();
        p.text(pos2(side.left(), y), Align2::LEFT_CENTER, "LAST NUMBERS", theme::bold_font(13.0), col::MUTED);
        y += 14.0;
        let ball = (side.width() / 14.0).clamp(17.0, 24.0);
        let step = ball * 2.0 + 6.0;
        let per_row = (((side.width() + 6.0) / step).floor() as usize).max(4);
        for (i, n) in hist.iter().take(per_row * 2).enumerate() {
            let c = pos2(side.left() + ball + (i % per_row) as f32 * step, y + ball + 2.0 + (i / per_row) as f32 * step);
            p.circle_filled(c, if i == 0 { ball + 1.0 } else { ball }, number_color(*n));
            if i == 0 {
                p.circle_stroke(c, ball + 1.0, Stroke::new(2.5f32, col::GOLD));
            }
            p.text(c, Align2::CENTER_CENTER, n.to_string(), theme::bold_font(ball * 0.82), Color32::WHITE);
        }
        if hist.is_empty() {
            p.text(pos2(side.left(), y + 16.0), Align2::LEFT_CENTER, "None yet", theme::font(14.0), theme::DIM());
            y += 40.0;
        } else {
            y += if hist.len() > per_row { step * 2.0 } else { step } + 10.0;
        }
        // Hot and cold: what's come up most in the last numbers, and what's been away longest.
        if hist.len() >= 10 && y + 70.0 < side.bottom() {
            let mut count = [0u32; 37];
            for n in &hist {
                count[*n as usize] += 1;
            }
            let mut hot: Vec<u8> = (0..37u8).filter(|n| count[*n as usize] >= 2).collect();
            hot.sort_by_key(|n| std::cmp::Reverse((count[*n as usize], hist.iter().position(|h| h == n).map(|i| usize::MAX - i))));
            let mut cold: Vec<u8> = (0..37u8).filter(|n| count[*n as usize] == 0).collect();
            cold.sort_by_key(|n| (*n as u32 * 2654435761u32) % 37);
            let small = ball * 0.82;
            for (k, (label, nums, tint)) in [("HOT", hot.into_iter().take(4).collect::<Vec<_>>(), col::EMBER), ("COLD", cold.into_iter().take(4).collect::<Vec<_>>(), col::BLUE)].into_iter().enumerate() {
                let x0 = side.left() + k as f32 * (side.width() / 2.0).max(200.0);
                if x0 + 120.0 > side.right() {
                    break;
                }
                p.text(pos2(x0, y + small), Align2::LEFT_CENTER, label, theme::bold_font(13.0), tint);
                for (i, n) in nums.iter().enumerate() {
                    let c = pos2(x0 + 52.0 + small + i as f32 * (small * 2.0 + 6.0), y + small);
                    p.circle_filled(c, small, number_color(*n));
                    p.circle_stroke(c, small, Stroke::new(1.5f32, tint.gamma_multiply(0.8)));
                    p.text(c, Align2::CENTER_CENTER, n.to_string(), theme::bold_font(small * 0.8), Color32::WHITE);
                }
            }
            y += small * 2.0 + 18.0;
        }
        // How the last numbers fell: red, black and zero.
        if hist.len() >= 5 && y + 30.0 < side.bottom() {
            let reds = hist.iter().filter(|n| **n != 0 && rl::is_red(**n)).count() as f32;
            let zeros = hist.iter().filter(|n| **n == 0).count() as f32;
            let total = hist.len() as f32;
            let blacks = total - reds - zeros;
            let bar = Rect::from_min_size(pos2(side.left(), y), vec2((side.width() - 10.0).min(380.0), 14.0));
            let mut x = bar.left();
            for (k, c) in [(reds, col::RED), (zeros, col::GREEN), (blacks, Color32::from_rgb(0x30, 0x30, 0x38))] {
                let w = bar.width() * k / total;
                if w > 0.0 {
                    p.rect_filled(Rect::from_min_size(pos2(x, bar.top()), vec2(w, bar.height())), 0, c);
                }
                x += w;
            }
            p.text(pos2(bar.left(), bar.bottom() + 14.0), Align2::LEFT_CENTER, format!("Red {:.0}%  ·  Zero {:.0}%  ·  Black {:.0}%   (last {})", reds / total * 100.0, zeros / total * 100.0, blacks / total * 100.0, hist.len()), theme::font(14.0), col::MUTED);
        }
        // The layout.
        let hits = spin.as_ref().filter(|_| done).and_then(|(r, _)| if let Outcome::Roulette(o) = &r.outcome { Some(o.number) } else { None });
        self.roulette_layout(ui, lay, hits, done);
        // The controls.
        let bar = Rect::from_min_max(pos2(area.left() + 20.0, area.bottom() - bar_h), pos2(area.right() - 20.0, area.bottom() - 14.0));
        self.roulette_controls(ui, s, bar, done);
    }

    fn roulette_layout(&mut self, ui: &mut egui::Ui, lay: Rect, hit: Option<u8>, done: bool) {
        let p = ui.painter().clone();
        let board = Board::new(lay);
        board.draw(&p, hit);
        // Where the pointer is: which bet, its numbers lit.
        let pointer = ui.ctx().pointer_hover_pos();
        let hovered = pointer.and_then(|q| board.spot_at(q));
        if let Some(spot) = &hovered {
            board.light(&p, spot);
            let on = self.poker.casino.roulette.on(spot);
            board.tip(&p, spot, on);
        }
        // Clicks: put a chip down; right-click takes that bet off.
        let resp = ui.interact(board.felt, ui.id().with("roulette-layout"), Sense::click());
        if done && let Some(spot) = hovered {
            if resp.clicked() {
                let max = rl::max_on(&spot);
                let have = self.poker.casino.roulette.on(&spot);
                let add = self.poker.casino.chip.min(max.saturating_sub(have));
                if add > 0 {
                    self.poker.casino.roulette.chips.push(Chip { spot, chips: add });
                    self.poker.casino.sound_in(0.0, Cue::Chips, 0.0, 0.4);
                }
            }
            if resp.secondary_clicked() {
                self.poker.casino.roulette.chips.retain(|c| c.spot != spot);
            }
        }
        // The chips on the layout.
        let won: Vec<Spot> = hit.map(|n| self.poker.casino.roulette.merged().into_iter().filter(|c| c.spot.covers(n)).map(|c| c.spot).collect()).unwrap_or_default();
        for c in self.poker.casino.roulette.merged() {
            let at = board.spot_pos(&c.spot);
            if won.contains(&c.spot) {
                p.circle_filled(at, board.cw * 0.36, col::GOLD.gamma_multiply(0.45));
            }
            chip_stack(&p, at, (board.cw * 0.24).clamp(9.0, 16.0), c.chips);
        }
    }

    fn roulette_controls(&mut self, ui: &mut egui::Ui, s: &ServerView, bar: Rect, done: bool) {
        let p = ui.painter().clone();
        p.rect_filled(bar, theme::r(18), Color32::from_black_alpha(170));
        let total = self.poker.casino.roulette.total();
        let c = &s.poker.casino;
        let busy = self.poker.casino.busy() || !done;
        let mut x = bar.right() - 16.0;
        let mut button = |ui: &mut egui::Ui, text: &str, w: f32, primary: bool, enabled: bool| -> bool {
            let r = Rect::from_min_size(pos2(x - w, bar.center().y - 22.0), vec2(w, 44.0));
            x -= w + 10.0;
            let resp = ui.interact(r, ui.id().with(("rl", text)), Sense::click());
            let fill = if !enabled { Color32::from_white_alpha(18) } else if primary { if resp.hovered() { col::GOLD2 } else { col::GOLD } } else if resp.hovered() { Color32::from_white_alpha(46) } else { Color32::from_white_alpha(24) };
            ui.painter().rect_filled(r, theme::r(12), fill);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(15.0), if primary && enabled { Color32::from_rgb(0x2a, 0x10, 0x04) } else if enabled { Color32::WHITE } else { Color32::from_white_alpha(70) });
            enabled && resp.clicked()
        };
        let space = ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) && !ui.ctx().wants_keyboard_input();
        let can = total > 0 && total <= super::chips_now(s) && !busy;
        if (button(ui, "Spin", 130.0, true, can) || (space && can))
            && let Some(commit) = c.commit
        {
            let chips = self.poker.casino.roulette.merged();
            self.poker.casino.roulette.last = self.poker.casino.roulette.chips.clone();
            self.poker.casino.pending = Some(Instant::now());
            self.poker.casino.celebration = None;
            self.engine.casino_play(&s.key, commit, Bet::Roulette { chips });
        }
        if button(ui, "Double", 96.0, false, !busy && total > 0) {
            // (Each bet up to its limit.)
            let more: Vec<Chip> = self
                .poker
                .casino
                .roulette
                .merged()
                .into_iter()
                .map(|c| Chip { chips: c.chips.min(rl::max_on(&c.spot).saturating_sub(c.chips)), spot: c.spot })
                .filter(|c| c.chips > 0)
                .collect();
            self.poker.casino.roulette.chips.extend(more);
        }
        let last = !self.poker.casino.roulette.last.is_empty();
        if button(ui, "Rebet", 90.0, false, !busy && last && total == 0) {
            self.poker.casino.roulette.chips = self.poker.casino.roulette.last.clone();
        }
        if button(ui, "Undo", 80.0, false, !busy && total > 0) {
            self.poker.casino.roulette.chips.pop();
        }
        if button(ui, "Clear", 80.0, false, !busy && total > 0) {
            self.poker.casino.roulette.chips.clear();
        }
        let color = if total > super::chips_now(s) { col::LOSE } else { col::GOLD2 };
        self.rack_and_bet(ui, bar, x, 5_000, format!("Bet {}", chips_text(total)), color);
    }
}

/// The outside bets along the bottom of the layout, left to right.
const OUTS: [(&str, Spot); 6] = [("1-18", Spot::Low), ("EVEN", Spot::Even), ("", Spot::Red), ("", Spot::Black), ("ODD", Spot::Odd), ("19-36", Spot::High)];

/// The layout's shape on the screen: the zero, twelve columns of three numbers, the "2 to 1"
/// column, then the dozens and the outside bets under them. Shared by the single-player wheel
/// and the tables.
#[derive(Clone, Copy)]
pub(super) struct Board {
    pub felt: Rect,
    grid: Rect,
    zero: Rect,
    cols: Rect,
    dozens: Rect,
    outside: Rect,
    pub cw: f32,
    chh: f32,
}

impl Board {
    pub fn new(lay: Rect) -> Board {
        let cw = (lay.width() / 14.0).min(lay.height() / 3.68);
        let chh = cw * 0.8;
        let grid = Rect::from_min_size(pos2(lay.left() + cw, lay.top()), vec2(cw * 12.0, chh * 3.0));
        let zero = Rect::from_min_size(lay.left_top(), vec2(cw, chh * 3.0));
        let cols = Rect::from_min_size(pos2(grid.right(), grid.top()), vec2(cw, chh * 3.0));
        let dozens = Rect::from_min_size(pos2(grid.left(), grid.bottom()), vec2(cw * 12.0, chh * 0.8));
        let outside = Rect::from_min_size(pos2(grid.left(), dozens.bottom()), vec2(cw * 12.0, chh * 0.8));
        let felt = Rect::from_min_max(zero.min - vec2(6.0, 6.0), pos2(cols.right() + 6.0, outside.bottom() + 6.0));
        Board { felt, grid, zero, cols, dozens, outside, cw, chh }
    }

    /// The number in column `i` (0 to 11), row `j` (0 at the top: 3, 6, 9…).
    fn num_at(i: usize, j: usize) -> u8 {
        (3 * i + (3 - j)) as u8
    }

    /// A number's box.
    fn cell(&self, n: u8) -> Rect {
        if n == 0 {
            return self.zero;
        }
        let i = ((n - 1) / 3) as f32;
        let j = (2 - (n - 1) % 3) as f32;
        Rect::from_min_size(pos2(self.grid.left() + i * self.cw, self.grid.top() + j * self.chh), vec2(self.cw, self.chh))
    }

    /// The felt, the numbers and the outside bets; `hit` outlined.
    pub fn draw(&self, p: &egui::Painter, hit: Option<u8>) {
        let (cw, chh) = (self.cw, self.chh);
        p.rect_filled(self.felt, theme::r(10), Color32::from_rgb(0x0e, 0x5e, 0x38));
        p.rect_stroke(self.felt, theme::r(10), Stroke::new(2.0f32, col::GOLD_DARK), StrokeKind::Inside);
        let line = Stroke::new(1.5f32, Color32::from_white_alpha(170));
        for i in 0..12 {
            for j in 0..3 {
                let n = Self::num_at(i, j);
                let r = self.cell(n);
                p.rect_stroke(r, 0, line, StrokeKind::Inside);
                let color = if rl::is_red(n) { col::RED } else { Color32::from_rgb(0x16, 0x16, 0x1a) };
                p.add(egui::epaint::EllipseShape::filled(r.center(), vec2(cw * 0.32, chh * 0.34), color));
                p.text(r.center(), Align2::CENTER_CENTER, n.to_string(), theme::bold_font(cw * 0.3), Color32::WHITE);
                if hit == Some(n) {
                    p.rect_stroke(r.shrink(2.0), 0, Stroke::new(3.0f32, col::GOLD2), StrokeKind::Inside);
                }
            }
        }
        p.rect_stroke(self.zero, 0, line, StrokeKind::Inside);
        p.add(egui::epaint::EllipseShape::filled(self.zero.center(), vec2(cw * 0.32, chh * 0.5), col::GREEN));
        p.text(self.zero.center(), Align2::CENTER_CENTER, "0", theme::bold_font(cw * 0.34), Color32::WHITE);
        if hit == Some(0) {
            p.rect_stroke(self.zero.shrink(2.0), 0, Stroke::new(3.0f32, col::GOLD2), StrokeKind::Inside);
        }
        for j in 0..3 {
            let r = Rect::from_min_size(pos2(self.cols.left(), self.cols.top() + j as f32 * chh), vec2(cw, chh));
            p.rect_stroke(r, 0, line, StrokeKind::Inside);
            p.text(r.center(), Align2::CENTER_CENTER, "2 to 1", theme::bold_font(cw * 0.2), Color32::WHITE);
        }
        for d in 0..3 {
            let r = Rect::from_min_size(pos2(self.dozens.left() + d as f32 * cw * 4.0, self.dozens.top()), vec2(cw * 4.0, self.dozens.height()));
            p.rect_stroke(r, 0, line, StrokeKind::Inside);
            p.text(r.center(), Align2::CENTER_CENTER, ["1st 12", "2nd 12", "3rd 12"][d], theme::bold_font(cw * 0.26), Color32::WHITE);
        }
        for (k, (label, spot)) in OUTS.iter().enumerate() {
            let r = Rect::from_min_size(pos2(self.outside.left() + k as f32 * cw * 2.0, self.outside.top()), vec2(cw * 2.0, self.outside.height()));
            p.rect_stroke(r, 0, line, StrokeKind::Inside);
            match spot {
                Spot::Red | Spot::Black => {
                    let c = r.center();
                    let (w, h) = (cw * 0.5, self.outside.height() * 0.32);
                    p.add(Shape::convex_polygon(vec![c + vec2(0.0, -h), c + vec2(w, 0.0), c + vec2(0.0, h), c + vec2(-w, 0.0)], if *spot == Spot::Red { col::RED } else { Color32::from_rgb(0x16, 0x16, 0x1a) }, Stroke::new(1.0f32, Color32::WHITE)));
                }
                _ => {
                    p.text(r.center(), Align2::CENTER_CENTER, *label, theme::bold_font(cw * 0.26), Color32::WHITE);
                }
            }
        }
    }

    /// The bet under a point on the felt: a number, the line between two (a split), a corner,
    /// the bottom edge of a column (a street) or between two (a six line), or an outside bet.
    pub fn spot_at(&self, q: Pos2) -> Option<Spot> {
        if !self.felt.contains(q) {
            return None;
        }
        let (grid, cw, chh) = (self.grid, self.cw, self.chh);
        let num_at = Self::num_at;
        let edge = 0.2;
        if grid.expand2(vec2(cw * 0.1, chh * 0.25)).contains(q) && !self.zero.contains(q) && !self.cols.contains(q) {
            let fx = ((q.x - grid.left()) / cw).clamp(0.0, 11.999);
            let fy = (q.y - grid.top()) / chh;
            let i = fx.floor() as usize;
            let j = (fy.floor().max(0.0) as usize).min(2);
            let near_v = fx.fract() < edge && i > 0;
            let near_v2 = fx.fract() > 1.0 - edge && i < 11;
            let near_h = fy.fract() < edge && fy > 0.5 && fy < 3.0;
            let near_h2 = fy.fract() > 1.0 - edge && fy < 2.5;
            let bottom = fy > 3.0 - edge;
            // A street or a six line, at the bottom edge.
            if bottom {
                if near_v {
                    return Some(Spot::Line { row: (i - 1) as u8 });
                }
                if near_v2 {
                    return Some(Spot::Line { row: i as u8 });
                }
                return Some(Spot::Street { row: i as u8 });
            }
            let vi = if near_v { Some(i - 1) } else if near_v2 { Some(i) } else { None };
            let hj = if near_h { Some(j - 1) } else if near_h2 { Some(j) } else { None };
            match (vi, hj) {
                (Some(a), Some(b)) => {
                    let ns = [num_at(a, b), num_at(a, b + 1), num_at(a + 1, b), num_at(a + 1, b + 1)];
                    return Some(Spot::Corner { a: *ns.iter().min().unwrap() });
                }
                (Some(a), None) => {
                    let (x, y) = (num_at(a, j), num_at(a + 1, j));
                    return Some(Spot::Split { a: x.min(y), b: x.max(y) });
                }
                (None, Some(b)) => {
                    let (x, y) = (num_at(i, b), num_at(i, b + 1));
                    return Some(Spot::Split { a: x.min(y), b: x.max(y) });
                }
                _ => {}
            }
            if fx.fract() < edge && i == 0 {
                return Some(Spot::Split { a: 0, b: num_at(0, j) });
            }
            return Some(Spot::Straight { n: num_at(i, j) });
        }
        if self.zero.contains(q) {
            return Some(Spot::Straight { n: 0 });
        }
        if self.cols.contains(q) {
            let j = (((q.y - self.cols.top()) / chh).floor() as usize).min(2);
            return Some(Spot::Column { c: (2 - j) as u8 });
        }
        if self.dozens.contains(q) {
            let d = (((q.x - self.dozens.left()) / (cw * 4.0)).floor() as usize).min(2);
            return Some(Spot::Dozen { d: d as u8 });
        }
        if self.outside.contains(q) {
            let k = (((q.x - self.outside.left()) / (cw * 2.0)).floor() as usize).min(5);
            return Some(OUTS[k].1);
        }
        None
    }

    /// Where a bet's chips sit.
    pub fn spot_pos(&self, spot: &Spot) -> Pos2 {
        let (grid, cw, chh) = (self.grid, self.cw, self.chh);
        let at = |x: f32, y: f32| pos2(grid.left() + x * cw, grid.top() + y * chh);
        let cell = |n: u8| -> (f32, f32) { (((n - 1) / 3) as f32, (2 - (n - 1) % 3) as f32) };
        match *spot {
            Spot::Straight { n: 0 } => self.zero.center(),
            Spot::Straight { n } => {
                let (i, j) = cell(n);
                at(i + 0.5, j + 0.5)
            }
            Spot::Split { a: 0, b } => {
                let (_, j) = cell(b);
                at(0.0, j + 0.5)
            }
            Spot::Split { a, b } => {
                let (i1, j1) = cell(a);
                let (i2, j2) = cell(b);
                at((i1 + i2) / 2.0 + 0.5, (j1 + j2) / 2.0 + 0.5)
            }
            Spot::Street { row } => at(row as f32 + 0.5, 3.0),
            Spot::Line { row } => at(row as f32 + 1.0, 3.0),
            Spot::Corner { a } => {
                let (i, j) = cell(a);
                at(i + 1.0, j)
            }
            Spot::Column { c } => pos2(self.cols.center().x, self.cols.top() + ((2 - c) as f32 + 0.5) * chh),
            Spot::Dozen { d } => pos2(self.dozens.left() + (d as f32 + 0.5) * cw * 4.0, self.dozens.center().y),
            other => {
                let k = OUTS.iter().position(|(_, s)| *s == other).unwrap_or(0);
                pos2(self.outside.left() + (k as f32 + 0.5) * cw * 2.0, self.outside.center().y)
            }
        }
    }

    /// The numbers a bet covers, lit.
    pub fn light(&self, p: &egui::Painter, spot: &Spot) {
        for n in 0..=36u8 {
            if spot.covers(n) {
                p.rect_filled(self.cell(n), 0, Color32::from_white_alpha(46));
            }
        }
    }

    /// What a bet is and pays (and what's on it), over the layout's top right corner.
    pub fn tip(&self, p: &egui::Painter, spot: &Spot, on: u64) {
        let tip = if on > 0 { format!("{}  ·  pays {} to 1  ·  on it {}", spot.name(), spot.pays(), chips_text(on)) } else { format!("{}  ·  pays {} to 1", spot.name(), spot.pays()) };
        let galley = p.layout_no_wrap(tip, theme::bold_font(13.0), col::GOLD2);
        let size = galley.size() + vec2(20.0, 8.0);
        let tr = Rect::from_min_size(pos2(self.felt.right() - size.x, self.felt.top() - size.y - 10.0), size);
        p.rect_filled(tr, theme::r(8), Color32::from_black_alpha(190));
        p.galley(tr.min + vec2(10.0, 4.0), galley, col::GOLD2);
    }
}

/// A number's color on the wheel and the layout.
pub(super) fn number_color(n: u8) -> Color32 {
    if n == 0 {
        col::GREEN
    } else if rl::is_red(n) {
        col::RED
    } else {
        Color32::from_rgb(0x1a, 0x1a, 0x20)
    }
}

/// The wheel: its wooden bowl, the ball track, 37 pockets in order, and the turret.
pub(super) fn draw_wheel(p: &egui::Painter, c: Pos2, r: f32, angle: f32) {
    p.circle_filled(c + vec2(0.0, 8.0), r * 1.02, Color32::from_black_alpha(120));
    radial(p, c, r, Color32::from_rgb(0x8a, 0x52, 0x26), Color32::from_rgb(0x3a, 0x1e, 0x0c));
    p.circle_stroke(c, r, Stroke::new(3.0f32, col::GOLD_DARK));
    // The ball track.
    p.circle_filled(c, r * 0.92, Color32::from_rgb(0x2a, 0x1a, 0x10));
    radial(p, c, r * 0.9, Color32::from_rgb(0x5a, 0x3a, 0x22), Color32::from_rgb(0x1a, 0x0e, 0x08));
    // The pockets.
    let (outer, inner) = (r * 0.8, r * 0.56);
    for (i, n) in WHEEL.iter().enumerate() {
        let a0 = angle + i as f32 / 37.0 * TAU;
        let a1 = angle + (i + 1) as f32 / 37.0 * TAU;
        let fill = if *n == 0 { col::GREEN } else if rl::is_red(*n) { Color32::from_rgb(0xb8, 0x1c, 0x2a) } else { Color32::from_rgb(0x14, 0x14, 0x18) };
        let steps = 3;
        let mut pts = Vec::new();
        for k in 0..=steps {
            let a = a0 + (a1 - a0) * k as f32 / steps as f32;
            pts.push(c + vec2(a.cos(), a.sin()) * outer);
        }
        for k in (0..=steps).rev() {
            let a = a0 + (a1 - a0) * k as f32 / steps as f32;
            pts.push(c + vec2(a.cos(), a.sin()) * inner);
        }
        p.add(Shape::convex_polygon(pts, fill, Stroke::new(1.0f32, col::GOLD_DARK)));
        // The number, upright to the rim.
        let mid = (a0 + a1) / 2.0;
        let at = c + vec2(mid.cos(), mid.sin()) * (outer - r * 0.075);
        let galley = p.layout_no_wrap(n.to_string(), theme::bold_font(r * 0.062), Color32::WHITE);
        let pos = at - galley.size() / 2.0;
        p.add(egui::epaint::TextShape::new(pos, galley, Color32::WHITE).with_angle_and_anchor(mid + TAU / 4.0, Align2::CENTER_CENTER));
    }
    p.circle_stroke(c, outer, Stroke::new(2.0f32, col::GOLD));
    // The cone and the turret.
    radial(p, c, inner, Color32::from_rgb(0xb0, 0x74, 0x36), Color32::from_rgb(0x5a, 0x32, 0x14));
    p.circle_stroke(c, inner, Stroke::new(2.0f32, col::GOLD_DARK));
    for k in 0..4 {
        let a = angle * 1.0 + k as f32 * TAU / 4.0;
        p.line_segment([c, c + vec2(a.cos(), a.sin()) * inner * 0.42], Stroke::new(r * 0.035, col::GOLD));
        p.circle_filled(c + vec2(a.cos(), a.sin()) * inner * 0.42, r * 0.03, col::GOLD2);
    }
    radial(p, c, r * 0.1, col::GOLD2, col::GOLD_DARK);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The angle between two directions, -π to π.
    fn diff(a: f32, b: f32) -> f32 {
        (a - b + std::f32::consts::PI).rem_euclid(TAU) - std::f32::consts::PI
    }

    #[test]
    fn the_ball_goes_round_drops_hops_and_settles_in_its_pocket() {
        for (pocket, ways) in [(0usize, [-5, -3, -1, 1]), (17, [-6, -2, 1, -1]), (36, [-7, -3, -1, 0])] {
            let a0 = 1.3;
            // Settled: in its pocket, carried round by the wheel, for good.
            for t in [rt::SETTLED_S, rt::RESULT_S, rt::RESULT_S + 5.0] {
                let (a, out) = ball(a0, pocket, ways, t);
                assert!(diff(a, a0 + wheel_turn(t) + pocket_angle(pocket as i32)).abs() < 1e-3, "pocket {pocket} at {t}");
                assert_eq!(out, 0.0);
            }
            // No jumps: from one 120th of a second to the next it moves only a little (round
            // the track at its fastest, 13.5 rad/s, is 0.11), and only down, never jumps out.
            let mut last = ball(a0, pocket, ways, 0.0);
            let mut t = 1.0 / 120.0;
            while t < rt::RESULT_S {
                let now = ball(a0, pocket, ways, t);
                assert!(diff(now.0, last.0).abs() < 0.13, "pocket {pocket}: the ball jumped at {t:.3} s");
                assert!((now.1 - last.1).abs() < 0.08, "pocket {pocket}: the ball jumped out at {t:.3} s");
                last = now;
                t += 1.0 / 120.0;
            }
            // On the track until it leaves, then down, then hopping no higher than a pocket.
            assert_eq!(ball(a0, pocket, ways, rt::LEAVE_S - 0.01).1, 1.0);
            assert!(ball(a0, pocket, ways, rt::POCKETS_S + 0.01).1 < 0.1);
            assert!((rt::POCKETS_S..rt::SETTLED_S).all_steps(|t| ball(a0, pocket, ways, t).1 <= 0.5));
        }
        // The ball slows all the way round the track; the wheel never stops.
        assert!((0.0..rt::LEAVE_S).all_steps(|t| rt::ball_speed(t + 0.01) < rt::ball_speed(t)));
        assert!((rt::ball_speed(0.0) - 13.5).abs() < 0.01 && (rt::ball_speed(rt::LEAVE_S) - 3.4).abs() < 0.05);
        assert!(rt::ball_run(rt::LEAVE_S) / TAU > 6.0, "a good six laps");
        assert!((0.0..600.0).all_steps(|t| wheel_turn(t + 0.01) - wheel_turn(t) > 0.01 * 1.19));
        // The number's called once the ball's still, about 8 seconds in.
        assert!((7.5..9.0).contains(&rt::RESULT_S), "{}", rt::RESULT_S);
    }

    trait Steps {
        fn all_steps(self, f: impl Fn(f32) -> bool) -> bool;
    }

    impl Steps for std::ops::Range<f32> {
        fn all_steps(self, f: impl Fn(f32) -> bool) -> bool {
            let mut t = self.start;
            while t < self.end {
                if !f(t) {
                    return false;
                }
                t += 0.01;
            }
            true
        }
    }
}
