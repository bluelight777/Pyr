//! Roulette at a table of five (0.13.0 servers): people and bots round one wheel, a spin every
//! few seconds. The wheel turns on the left, the ball going round, dropping and rattling into its
//! pocket as at the single-player wheel; beside it the number called, the last numbers and how
//! they've fallen; the seats down the right; and the layout along the bottom with everyone's
//! chips on it, each seat's ringed in its own colour (mine bigger). Every spin is spun again by
//! this Pyr from its seeds (see `pyr_engine::poker::SharedLive`).

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::poker_sfx::roulette as rt;
use pyr_engine::state::ServerView;
use pyr_poker::casino::roulette::{self as rl, Chip, Spot};
use pyr_poker::net::{RlReveal, RlSeatView, RlStage, RlView, TableGame, ToServer};
use pyr_poker::rlroom::{self, BET_MS, SEATS, SHOW_MS};

use super::roulette::{Board, ball, draw_wheel, hops_by, number_color, wheel_turn};
use super::{chip_stack, chips_now, col, ease_out, glow_text, radial, vgrad};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

const BAR_H: f32 = 96.0;
const STRIP_H: f32 = 44.0;
/// Each seat's colour: its ring round its chips on the layout, its mark in the list.
const SEAT_COLORS: [Color32; SEATS] = [
    Color32::from_rgb(0xff, 0x8a, 0x3d),
    Color32::from_rgb(0x4d, 0xb8, 0xff),
    Color32::from_rgb(0xb4, 0x7c, 0xff),
    Color32::from_rgb(0x4d, 0xe0, 0x9a),
    Color32::from_rgb(0xff, 0x6b, 0x9a),
];
/// How long the chips take to go once the number's called (losers swept, winners paid), before
/// the next spin's chips show.
const SETTLE_S: f32 = 1.4;

#[derive(Default)]
pub struct RlTableUi {
    /// The table I'm at (watching, or sitting): its server and id.
    pub at: Option<(String, u64)>,
    asked: Option<Instant>,
    watch_asked: Option<Instant>,
    /// My chips as I put them down, for spin `.0` (shown till the table has them).
    desired: Option<(u64, Vec<Chip>)>,
    /// The spin on the table, when the ball went, and where the wheel was then.
    spun: Option<(u64, Instant, f32)>,
    /// The wheel's last push (when, and where it was then): it keeps turning between spins.
    wheel_push: Option<(Instant, f32)>,
    /// My chips while they're in (the balance waits for the number), and the last spin whose
    /// result was called for me.
    playing_chips: Option<u64>,
    told: Option<u64>,
    /// My last spin's chips (for Rebet).
    rebet: Vec<Chip>,
}

/// A button on the controls; true when it's clicked.
fn button(ui: &mut egui::Ui, r: Rect, id: &str, text: &str, sub: Option<String>, fill: Color32, enabled: bool) -> bool {
    let resp = ui.interact(r, ui.id().with(("rlt", id)), Sense::click());
    let p = ui.painter();
    let bg = if !enabled { Color32::from_white_alpha(16) } else if resp.hovered() { fill.gamma_multiply(1.18) } else { fill };
    p.rect_filled(r, theme::r(12), bg);
    if enabled {
        p.rect_stroke(r, theme::r(12), Stroke::new(1.0f32, Color32::from_white_alpha(40)), StrokeKind::Inside);
    }
    let dark = enabled && (fill == col::GOLD || fill == col::GOLD2);
    let tc = if dark { Color32::from_rgb(0x2a, 0x10, 0x04) } else if enabled { Color32::WHITE } else { Color32::from_white_alpha(70) };
    match sub {
        Some(sub) => {
            p.text(r.center() - vec2(0.0, 8.0), Align2::CENTER_CENTER, text, theme::bold_font(15.0), tc);
            p.text(r.center() + vec2(0.0, 11.0), Align2::CENTER_CENTER, sub, theme::font(11.5), tc.gamma_multiply(0.8));
        }
        None => {
            p.text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(15.0), tc);
        }
    }
    enabled && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Where each seat's chips sit on a spot shared with others: mine in the middle, the others
/// round it, so every stack shows.
fn seat_offset(slot: usize, r: f32) -> egui::Vec2 {
    const AT: [(f32, f32); 5] = [(0.0, 0.0), (0.75, -0.45), (-0.75, -0.45), (0.75, 0.55), (-0.75, 0.55)];
    let (x, y) = AT[slot.min(4)];
    vec2(x, y) * r
}

impl PyrApp {
    /// Leaves the roulette table I'm at (standing up first: my chips come back).
    pub fn rl_table_leave(&mut self) {
        if let Some((key, id)) = self.poker.casino.rlt.at.take() {
            self.engine.poker(&key, ToServer::RlLeave { table: id });
            self.engine.poker(&key, ToServer::RlUnwatch { table: id });
            self.engine.rl_close(&key);
        }
        let t = &mut self.poker.casino.rlt;
        t.desired = None;
        t.spun = None;
        t.playing_chips = None;
    }

    /// The roulette tables' page: the tables along the top, the wheel, the seats, the layout,
    /// the controls.
    pub(super) fn rl_table_page(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let now = Instant::now();
        let key = s.key.clone();
        if self.poker.casino.rlt.asked.is_none_or(|t| t.elapsed() > Duration::from_secs(8)) {
            self.poker.casino.rlt.asked = Some(now);
            self.engine.poker(&key, ToServer::RlTables);
        }
        let tables = s.poker.rl_tables.clone().unwrap_or_default();
        let mine = self.poker.casino.rlt.at.clone().filter(|(k, _)| *k == key).map(|(_, id)| id);
        let at = match mine {
            Some(id) if tables.is_empty() || tables.iter().any(|t| t.id == id) => Some(id),
            _ => tables.first().map(|t| t.id),
        };
        let live = s.poker.rl.clone().filter(|b| Some(b.view.id) == at);
        if let Some(id) = at {
            let moved = self.poker.casino.rlt.at != Some((key.clone(), id));
            if moved || (live.is_none() && self.poker.casino.rlt.watch_asked.is_none_or(|t| t.elapsed() > Duration::from_secs(3))) {
                self.poker.casino.rlt.at = Some((key.clone(), id));
                self.poker.casino.rlt.watch_asked = Some(now);
                self.engine.poker(&key, ToServer::RlWatch { table: id });
            }
        }
        let p = ui.painter().clone();
        vgrad(&p, area, Color32::from_rgb(0x0c, 0x24, 0x18), Color32::from_rgb(0x04, 0x10, 0x0a));
        radial(&p, area.center(), area.width() * 0.6, Color32::from_rgba_unmultiplied(0x30, 0x90, 0x60, 40), Color32::TRANSPARENT);
        let strip = Rect::from_min_size(area.min, vec2(area.width(), STRIP_H));
        let bar = Rect::from_min_max(pos2(area.left() + 16.0, area.bottom() - BAR_H + 6.0), pos2(area.right() - 16.0, area.bottom() - 10.0));
        let room = Rect::from_min_max(pos2(area.left() + 20.0, strip.bottom() + 12.0), pos2(area.right() - 20.0, bar.top() - 26.0));
        self.rl_tables_strip(ui, s, strip, &tables, at);
        // The layout along the bottom; above it the wheel on the left, the seats on the right,
        // and the number and the last ones between.
        let cw = ((room.width() - 12.0) / 14.0).min(room.height() * 0.5 / 3.68).min(86.0);
        let lay = Rect::from_min_size(pos2(room.center().x - cw * 7.0, room.bottom() - cw * 3.68), vec2(cw * 14.0, cw * 3.68));
        let board = Board::new(lay);
        let above = Rect::from_min_max(room.min, pos2(room.right(), board.felt.top() - 30.0));
        let seats_w = (above.width() * 0.3).clamp(240.0, 340.0);
        let seats_r = Rect::from_min_max(pos2(above.right() - seats_w, above.top()), above.max);
        let wheel_d = above.height().min((above.width() - seats_w) * 0.5);
        let wc = pos2(above.left() + wheel_d / 2.0 + 6.0, above.center().y);
        let side = Rect::from_min_max(pos2(wc.x + wheel_d / 2.0 + 34.0, above.top() + 4.0), pos2(seats_r.left() - 20.0, above.bottom()));
        // The wheel turns all the time (slowly between spins).
        let push = *self.poker.casino.rlt.wheel_push.get_or_insert((now, 0.0));
        let wheel_a = push.1 + wheel_turn((now - push.0).as_secs_f32());
        draw_wheel(&p, wc, wheel_d / 2.0, wheel_a);
        let Some(live) = live else {
            board.draw(&p, None);
            p.text(side.center(), Align2::CENTER_CENTER, if s.poker.rl_tables.is_none() { "Setting out the tables…" } else { "Sitting you at the table…" }, theme::bold_font(16.0), col::MUTED);
            ui.ctx().request_repaint_after(Duration::from_millis(33));
            return;
        };
        let v = &live.view;
        // The spin on the table (once the ball's gone, until the next), on its own clock.
        let shown = matches!(v.stage, RlStage::Shown).then(|| v.last.clone()).flatten();
        if let Some(r) = &shown
            && self.poker.casino.rlt.spun.is_none_or(|(n, _, _)| n != r.spin)
        {
            // (Joined late: part way through already.)
            let ms_left = v.ms_left.saturating_sub(live.at.elapsed().as_millis() as u64);
            let since = Duration::from_millis(SHOW_MS.saturating_sub(ms_left));
            let went = now.checked_sub(since).unwrap_or(now);
            // The croupier pushes the wheel and launches the ball.
            let a = wheel_a % std::f32::consts::TAU;
            let a0 = a - wheel_turn(since.as_secs_f32());
            let t = &mut self.poker.casino.rlt;
            t.wheel_push = Some((went, a0));
            t.spun = Some((r.spin, went, a0));
            if since.as_secs_f32() < 0.5 {
                self.poker.casino.sound_in(0.0, Cue::BallRoll, 0.0, 0.7);
            }
            if since.as_secs_f32() < rt::DIAMOND_S {
                self.poker.casino.sound_in(rt::DIAMOND_S - since.as_secs_f32(), Cue::BallDrop, 0.0, 0.8);
            }
            self.rl_result_for_me(v, r, went, now);
        }
        let spin = self.poker.casino.rlt.spun.filter(|(n, _, _)| shown.as_ref().is_some_and(|r| r.spin == *n)).map(|(_, went, a0)| ((now - went).as_secs_f32(), a0));
        let since = spin.map(|(t, _)| t);
        let called = since.is_some_and(|t| t >= rt::RESULT_S);
        if spin.is_some_and(|(t, _)| t < rt::RESULT_S + 0.5) {
            ui.ctx().request_repaint();
        } else {
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
        // The ball.
        if let (Some(r), Some((t, a0))) = (&shown, spin) {
            let (a, out) = ball(a0, r.pocket as usize, hops_by(r.commit[1]), t);
            let rad = wheel_d / 2.0;
            let br = rad * (0.62 + 0.26 * out);
            p.circle_filled(wc + vec2(a.cos(), a.sin()) * br + vec2(1.5, 2.5), rad * 0.035, Color32::from_black_alpha(120));
            p.circle_filled(wc + vec2(a.cos(), a.sin()) * br, rad * 0.035, Color32::WHITE);
            if called {
                p.circle_filled(wc, 48.0, number_color(r.number));
                p.circle_stroke(wc, 48.0, Stroke::new(3.0f32, col::GOLD));
                glow_text(&p, wc, Align2::CENTER_CENTER, &r.number.to_string(), theme::bold_font(40.0), Color32::WHITE, Color32::from_black_alpha(120));
            }
        }
        // My chips while they're in.
        let mine_in = v.my_seat.and_then(|m| v.seats.get(m as usize).cloned().flatten()).is_some_and(|x| rlroom::total(&x.chips) > 0);
        if mine_in && v.stage == RlStage::Betting {
            self.poker.casino.rlt.playing_chips = Some(chips_now(s));
        }
        // The history shows the number once it's called (not while the ball's still going).
        let mut hist: Vec<u8> = v.history.iter().rev().copied().collect();
        if shown.is_some() && !called && !hist.is_empty() {
            hist.remove(0);
        }
        self.rl_side(&p, side, shown.as_ref().filter(|_| called), spin.is_some() && !called, v, &hist);
        // The seats, down the right.
        let row_h = (seats_r.height() / SEATS as f32).min(58.0);
        for i in 0..SEATS {
            let r = Rect::from_min_size(pos2(seats_r.left(), seats_r.top() + i as f32 * row_h), vec2(seats_r.width(), row_h - 6.0));
            let seat = v.seats.get(i).cloned().flatten();
            self.rl_seat(ui, s, v, i as u8, seat.as_ref(), shown.as_ref(), since, r);
        }
        // The layout and everyone's chips on it.
        let settling = since.is_some_and(|t| t < rt::RESULT_S + SETTLE_S);
        let hit = shown.as_ref().filter(|_| called).map(|r| r.number);
        board.draw(&p, hit);
        self.rl_chips(ui, s, v, &board, shown.as_ref(), since, settling);
        self.rl_controls(ui, s, &live, bar, settling);
        // Emotes, big over the seat of whoever sent them.
        let size = 96.0;
        super::emotes::pops(&p, ui.ctx(), &s.poker.game_emotes, TableGame::Roulette, v.id, size, |seat| pos2(seats_r.left() - 40.0, seats_r.top() + (seat as f32 + 0.5) * row_h));
    }

    /// Beside the wheel: the number called (or what's going on), the last numbers, hot and cold,
    /// and how red and black have fallen.
    fn rl_side(&self, p: &egui::Painter, side: Rect, called: Option<&RlReveal>, rolling: bool, v: &RlView, hist: &[u8]) {
        let big = (side.width() / 9.0).clamp(26.0, 46.0);
        let mut y = side.top() + 4.0;
        match called {
            Some(r) => {
                let n = r.number;
                let r_c = (big * 0.95).round();
                let cc = pos2(side.left() + r_c, y + r_c);
                p.circle_filled(cc + vec2(2.0, 4.0), r_c, Color32::from_black_alpha(120));
                p.circle_filled(cc, r_c, number_color(n));
                p.circle_stroke(cc, r_c, Stroke::new(3.0f32, col::GOLD));
                glow_text(p, cc, Align2::CENTER_CENTER, &n.to_string(), theme::bold_font(big * 0.95), Color32::WHITE, Color32::from_black_alpha(120));
                let x = cc.x + r_c + 16.0;
                let name = if n == 0 { "ZERO".to_string() } else { format!("{n} {}", rl::color_name(n).to_uppercase()) };
                glow_text(p, pos2(x, cc.y - big * 0.3), Align2::LEFT_CENTER, &name, theme::bold_font(big * 0.8), Color32::WHITE, number_color(n).gamma_multiply(0.8));
                let traits = if n == 0 {
                    "Only straight up, splits and the like win".to_string()
                } else {
                    format!("{}  ·  {}  ·  {} dozen", if n % 2 == 0 { "Even" } else { "Odd" }, if n <= 18 { "1 to 18" } else { "19 to 36" }, ["1st", "2nd", "3rd"][((n - 1) / 12) as usize])
                };
                p.text(pos2(x, cc.y + big * 0.32), Align2::LEFT_CENTER, traits, theme::font(big * 0.34), col::MUTED);
                y = cc.y + r_c + 16.0;
            }
            None => {
                let text = if rolling {
                    "No more bets…"
                } else if v.stage == RlStage::Waiting {
                    "Sit down to play"
                } else {
                    "Place your bets"
                };
                glow_text(p, pos2(side.left(), y + big * 0.6), Align2::LEFT_CENTER, text, theme::bold_font(big * 0.8), col::GOLD2, Color32::from_black_alpha(140));
                p.text(pos2(side.left(), y + big * 1.35), Align2::LEFT_CENTER, format!("{}  ·  {} to {} a place  ·  single zero", v.name, chips_text(v.min_bet), chips_text(v.max_bet)), theme::font(big * 0.34), col::MUTED);
                y += big * 1.95 + 14.0;
            }
        }
        p.text(pos2(side.left(), y), Align2::LEFT_CENTER, "LAST NUMBERS", theme::bold_font(12.5), col::MUTED);
        y += 12.0;
        let ball = (side.width() / 16.0).clamp(14.0, 22.0);
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
            y += if hist.len() > per_row { step * 2.0 } else { step } + 8.0;
        }
        if hist.len() >= 5 && y + 30.0 < side.bottom() {
            let reds = hist.iter().filter(|n| **n != 0 && rl::is_red(**n)).count() as f32;
            let zeros = hist.iter().filter(|n| **n == 0).count() as f32;
            let total = hist.len() as f32;
            let blacks = total - reds - zeros;
            let bar = Rect::from_min_size(pos2(side.left(), y), vec2((side.width() - 10.0).min(360.0), 12.0));
            let mut x = bar.left();
            for (k, c) in [(reds, col::RED), (zeros, col::GREEN), (blacks, Color32::from_rgb(0x30, 0x30, 0x38))] {
                let w = bar.width() * k / total;
                if w > 0.0 {
                    p.rect_filled(Rect::from_min_size(pos2(x, bar.top()), vec2(w, bar.height())), 0, c);
                }
                x += w;
            }
            p.text(pos2(bar.left(), bar.bottom() + 13.0), Align2::LEFT_CENTER, format!("Red {:.0}%  ·  Zero {:.0}%  ·  Black {:.0}%   (last {})", reds / total * 100.0, zeros / total * 100.0, blacks / total * 100.0, hist.len()), theme::font(13.0), col::MUTED);
        }
    }

    /// Everyone's chips on the layout: the spin's (riding, then swept or paid once the number's
    /// called), or the next spin's (mine as I put them down, till the table has them). My own
    /// go down with a click (right-click takes a place off).
    #[allow(clippy::too_many_arguments)]
    fn rl_chips(&mut self, ui: &mut egui::Ui, s: &ServerView, v: &RlView, board: &Board, shown: Option<&RlReveal>, since: Option<f32>, settling: bool) {
        let p = ui.painter().clone();
        let me = v.my_seat;
        let desired = self.poker.casino.rlt.desired.clone().filter(|(n, _)| *n == v.spin_no).map(|(_, c)| c);
        // Each seat's chips to show, and whether they're the spin's (riding) or the next's.
        let riding = shown.is_some() && settling;
        let mut by_seat: Vec<(u8, Vec<Chip>, bool)> = Vec::new();
        for (i, seat) in v.seats.iter().enumerate() {
            let Some(x) = seat else { continue };
            let i = i as u8;
            let chips = if riding {
                x.last_chips.clone()
            } else if me == Some(i) {
                desired.clone().unwrap_or_else(|| x.chips.clone())
            } else {
                x.chips.clone()
            };
            let unsent = !riding && me == Some(i) && desired.as_ref().is_some_and(|d| *d != x.chips);
            by_seat.push((i, chips, unsent));
        }
        // Where the pointer is (sitting): which bet, its numbers lit.
        let mine_seat = me.and_then(|m| v.seats.get(m as usize).cloned().flatten());
        let hovered = if mine_seat.is_some() { ui.ctx().pointer_hover_pos().and_then(|q| board.spot_at(q)) } else { None };
        let next = desired.clone().or_else(|| mine_seat.as_ref().map(|x| x.chips.clone())).unwrap_or_default();
        if let Some(spot) = &hovered {
            board.light(&p, spot);
            let on = next.iter().find(|c| c.spot == *spot).map(|c| c.chips).unwrap_or(0);
            board.tip(&p, spot, on);
        }
        // On each spot, mine in the middle and the others round it.
        let r = (board.cw * 0.22).clamp(8.0, 15.0);
        let small = r * 0.72;
        let called = shown.zip(since).filter(|(_, t)| *t >= rt::RESULT_S);
        let mut spots: Vec<Spot> = Vec::new();
        for (_, chips, _) in &by_seat {
            for c in chips {
                if !spots.contains(&c.spot) {
                    spots.push(c.spot);
                }
            }
        }
        for spot in spots {
            let at = board.spot_pos(&spot);
            let mut here: Vec<(u8, u64, bool)> = by_seat.iter().filter_map(|(i, chips, unsent)| chips.iter().find(|c| c.spot == spot).map(|c| (*i, c.chips, *unsent))).collect();
            here.sort_by_key(|(i, _, _)| (Some(*i) != me, *i));
            let mine_here = here.first().is_some_and(|(i, _, _)| Some(*i) == me);
            for (k, (i, amount, unsent)) in here.iter().enumerate() {
                let slot = if mine_here || me.is_none() { k } else { k + 1 };
                let mine = Some(*i) == me;
                let rr = if mine { r } else { small };
                let mut q = at + seat_offset(slot, r * 1.3);
                let mut painter = p.clone();
                if *unsent {
                    painter.set_opacity(0.6);
                }
                if let Some((rv, t)) = called {
                    let u = ((t - rt::RESULT_S) / 0.7).clamp(0.0, 1.0);
                    if spot.covers(rv.number) {
                        radial(&p, q, rr * 2.6, col::GOLD.gamma_multiply(0.55 * (1.0 - u * 0.3)), Color32::TRANSPARENT);
                    } else {
                        // Swept off to the croupier.
                        let to = pos2(board.felt.center().x, board.felt.top() - 40.0);
                        q = q + (to - q) * ease_out(u);
                        painter.set_opacity(1.0 - u);
                    }
                }
                chip_stack(&painter, q, rr, *amount);
                painter.circle_stroke(q, rr + 1.5, Stroke::new(if mine { 2.5f32 } else { 2.0 }, SEAT_COLORS[*i as usize % SEATS]));
            }
        }
        // My clicks: the chip picked goes down (right-click takes the place off).
        let Some(mine) = mine_seat else { return };
        let open = !settling && !mine.ready;
        let resp = ui.interact(board.felt, ui.id().with("rlt-layout"), Sense::click());
        if open && let Some(spot) = hovered {
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let top = v.max_bet.min(rl::max_on(&spot));
            let mut want = next.clone();
            if resp.clicked() {
                let have = want.iter().find(|c| c.spot == spot).map(|c| c.chips).unwrap_or(0);
                let add = self.poker.casino.chip.max(if have == 0 { v.min_bet } else { 0 }).min(top.saturating_sub(have));
                if add > 0 && want.len() < rlroom::MAX_SPOTS {
                    want.push(Chip { spot, chips: add });
                    want = rlroom::merged(&want);
                }
            }
            if resp.secondary_clicked() {
                want.retain(|c| c.spot != spot);
            }
            if want != next {
                self.rl_send(s, v, want, false);
            }
        }
    }

    /// The result of a spin for me, called out big as the ball settles, and the balance
    /// counting up.
    fn rl_result_for_me(&mut self, v: &RlView, r: &RlReveal, went: Instant, now: Instant) {
        if self.poker.casino.rlt.told == Some(r.spin) {
            return;
        }
        self.poker.casino.rlt.told = Some(r.spin);
        let Some(me) = v.my_seat.and_then(|m| v.seats.get(m as usize).cloned().flatten()) else { return };
        let chips = me.last_chips.clone();
        let wagered = rlroom::total(&chips);
        if wagered == 0 {
            return;
        }
        self.poker.casino.rlt.rebet = chips;
        let at = went + Duration::from_secs_f32(rt::RESULT_S);
        let back = at.saturating_duration_since(now).as_secs_f32();
        if let Some(c) = self.poker.casino.rlt.playing_chips.take() {
            self.poker.casino.hold = Some((c, at));
        }
        let paid = me.paid;
        let n = r.number;
        let (title, color) = if paid > wagered {
            ("YOU WIN".to_string(), col::WIN)
        } else if paid > 0 {
            ("SOME BACK".to_string(), Color32::from_rgb(0xd8, 0xd8, 0xe0))
        } else {
            (if n == 0 { "ZERO".to_string() } else { format!("{n} {}", rl::color_name(n).to_uppercase()) }, number_color(n).gamma_multiply(1.4))
        };
        let sub = if paid > wagered { format!("+{}", chips_text(paid - wagered)) } else { String::new() };
        self.poker.casino.announce(title, sub, color, (0.5, 0.42), back + 0.2);
        if paid > 0 {
            self.poker.casino.sound_in(back + 0.15, Cue::WinSmall, 0.0, 0.8);
            self.poker.casino.sound_in(back + 0.4, Cue::Chips, 0.0, 0.7);
        }
        self.poker.casino.celebrate(paid, wagered, false, back + 0.2);
    }

    /// The tables, along the top: each one's name, its bets and who's at it.
    fn rl_tables_strip(&mut self, ui: &mut egui::Ui, s: &ServerView, strip: Rect, tables: &[pyr_poker::net::RlSummary], at: Option<u64>) {
        let p = ui.painter().clone();
        vgrad(&p, strip, Color32::from_rgb(0x0a, 0x1c, 0x12), Color32::from_rgb(0x04, 0x0c, 0x08));
        let mut x = strip.left() + 14.0;
        for t in tables {
            let on = Some(t.id) == at;
            let text = format!("{}  ·  {}–{}", t.name, chips_text(t.min_bet), chips_text(t.max_bet));
            let who = format!("{} / {SEATS}", t.people + t.bots);
            let w = 34.0 + (text.chars().count() + who.len()) as f32 * 7.2;
            let r = Rect::from_min_size(pos2(x, strip.top() + 7.0), vec2(w, strip.height() - 14.0));
            x += w + 8.0;
            if r.right() > strip.right() - 12.0 {
                break;
            }
            let resp = ui.interact(r, ui.id().with(("rl-table", t.id)), Sense::click());
            p.rect_filled(r, theme::r(15), if on { col::GOLD.gamma_multiply(0.22) } else if resp.hovered() { Color32::from_white_alpha(22) } else { Color32::from_white_alpha(10) });
            p.rect_stroke(r, theme::r(15), Stroke::new(1.0f32, if on { col::GOLD } else { Color32::from_white_alpha(30) }), StrokeKind::Inside);
            p.text(r.left_center() + vec2(14.0, 0.0), Align2::LEFT_CENTER, text, theme::bold_font(13.0), if on { col::GOLD2 } else { col::TEXT });
            p.text(r.right_center() - vec2(12.0, 0.0), Align2::RIGHT_CENTER, who, theme::font(12.0), col::MUTED);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                self.rl_table_leave();
                self.poker.casino.rlt.at = Some((s.key.clone(), t.id));
                self.engine.poker(&s.key, ToServer::RlWatch { table: t.id });
                self.poker.casino.rlt.watch_asked = Some(Instant::now());
            }
        }
    }

    /// One seat in the list: its colour, who's in it, what they have down (or how the last spin
    /// went for them); or an open seat to sit in or put a bot in.
    #[allow(clippy::too_many_arguments)]
    fn rl_seat(&mut self, ui: &mut egui::Ui, s: &ServerView, v: &RlView, i: u8, seat: Option<&RlSeatView>, shown: Option<&RlReveal>, since: Option<f32>, r: Rect) {
        let p = ui.painter().clone();
        let me = v.my_seat == Some(i);
        let color = SEAT_COLORS[i as usize % SEATS];
        p.rect_filled(r, theme::r(12), Color32::from_black_alpha(if me { 200 } else { 150 }));
        p.rect_stroke(r, theme::r(12), Stroke::new(1.0f32, if me { col::GOLD } else { Color32::from_white_alpha(26) }), StrokeKind::Inside);
        let mark = Rect::from_min_size(r.left_top() + vec2(8.0, 8.0), vec2(5.0, r.height() - 16.0));
        p.rect_filled(mark, theme::r(3), color);
        let f = (r.height() * 0.3).clamp(11.0, 14.5);
        match seat {
            Some(x) => {
                let face_c = pos2(mark.right() + 8.0 + r.height() * 0.32, r.center().y);
                let fr = r.height() * 0.32;
                if x.bot {
                    crate::poker::table::bot_face(&p, face_c, fr, &x.name, pyr_poker::bots::Level::Regular);
                } else {
                    let face = self.face(ui.ctx(), s, &x.player);
                    crate::app::avatar(&p, face_c, fr, &x.player, &x.name, false, Color32::BLACK, face);
                }
                let name = if x.name.chars().count() > 14 { format!("{}…", x.name.chars().take(13).collect::<String>()) } else { x.name.clone() };
                let nx = face_c.x + fr + 9.0;
                p.text(pos2(nx, r.center().y - f * 0.55), Align2::LEFT_CENTER, &name, theme::bold_font(f), if me { col::GOLD2 } else { col::TEXT });
                // What they have down, or how the spin went for them.
                let called = shown.zip(since).is_some_and(|(_, t)| t >= rt::RESULT_S);
                let (line, lc) = if called && rlroom::total(&x.last_chips) > 0 {
                    let w = rlroom::total(&x.last_chips);
                    if x.paid > w { (format!("won {}", chips_text(x.paid - w)), col::GOLD2) } else if x.paid > 0 { (format!("{} back", chips_text(x.paid)), col::MUTED) } else { (format!("lost {}", chips_text(w)), Color32::from_white_alpha(120)) }
                } else if shown.is_some() && !called && rlroom::total(&x.last_chips) > 0 {
                    (format!("{} riding", chips_text(rlroom::total(&x.last_chips))), col::MUTED)
                } else if rlroom::total(&x.chips) > 0 {
                    (format!("{} on {} place{}", chips_text(rlroom::total(&x.chips)), x.chips.len(), if x.chips.len() == 1 { "" } else { "s" }), col::MUTED)
                } else if x.bot {
                    ("thinking…".to_string(), Color32::from_white_alpha(110))
                } else {
                    ("no chips down".to_string(), Color32::from_white_alpha(110))
                };
                p.text(pos2(nx, r.center().y + f * 0.62), Align2::LEFT_CENTER, line, theme::font(f * 0.82), lc);
                let mut right = r.right() - 10.0;
                if x.ready && v.stage == RlStage::Betting {
                    let g = p.layout_no_wrap("READY".into(), theme::bold_font(f * 0.72), col::WIN);
                    right -= g.size().x;
                    p.galley(pos2(right, r.center().y - g.size().y / 2.0), g, col::WIN);
                    right -= 8.0;
                }
                if x.bot {
                    let tag = Rect::from_min_size(pos2(right - f * 2.3, r.center().y - f * 0.6), vec2(f * 2.3, f * 1.2));
                    p.rect_filled(tag, theme::r(5), Color32::from_rgb(0x3d, 0x5f, 0xc4));
                    p.text(tag.center(), Align2::CENTER_CENTER, "BOT", theme::bold_font(f * 0.68), Color32::WHITE);
                    let resp = ui.interact(r, ui.id().with(("rl-bot", i)), Sense::click()).on_hover_text(format!("{}: a bot that bets {}. It plays with the house's chips.", x.name, x.style));
                    resp.context_menu(|ui| {
                        crate::menu::style(ui);
                        if ui.button(format!("Ask {} to leave", x.name)).clicked() {
                            self.engine.poker(&s.key, ToServer::RlBot { table: v.id, seat: i, add: false });
                            ui.close();
                        }
                    });
                }
            }
            None => {
                p.text(pos2(mark.right() + 14.0, r.center().y), Align2::LEFT_CENTER, "Open seat", theme::font(f * 0.9), Color32::from_white_alpha(110));
                let sitting = v.my_seat.is_some();
                let h = (r.height() - 16.0).min(30.0);
                let br = Rect::from_min_size(pos2(r.right() - 10.0 - 76.0, r.center().y - h / 2.0), vec2(76.0, h));
                let bresp = ui.interact(br, ui.id().with(("rl-addbot", i)), Sense::click());
                p.rect_filled(br, theme::r(8), if bresp.hovered() { Color32::from_rgb(0x3d, 0x5f, 0xc4) } else { Color32::from_white_alpha(18) });
                p.text(br.center(), Align2::CENTER_CENTER, "+ BOT", theme::bold_font(12.0), if bresp.hovered() { Color32::WHITE } else { Color32::from_white_alpha(150) });
                if bresp.on_hover_text("Seat a bot here: each bets its own way, with the house's chips").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.engine.poker(&s.key, ToServer::RlBot { table: v.id, seat: i, add: true });
                }
                if !sitting {
                    let sr = Rect::from_min_size(pos2(br.left() - 8.0 - 64.0, br.top()), vec2(64.0, h));
                    let resp = ui.interact(sr, ui.id().with(("rl-sit", i)), Sense::click());
                    p.rect_filled(sr, theme::r(8), if resp.hovered() { col::GOLD } else { Color32::from_black_alpha(170) });
                    p.rect_stroke(sr, theme::r(8), Stroke::new(1.0f32, col::GOLD.gamma_multiply(0.6)), StrokeKind::Inside);
                    p.text(sr.center(), Align2::CENTER_CENTER, "SIT", theme::bold_font(12.5), if resp.hovered() { Color32::BLACK } else { col::GOLD });
                    if resp.on_hover_text("Sit here").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.poker.casino.rlt.desired = None;
                        self.engine.poker(&s.key, ToServer::RlSit { table: v.id, seat: i });
                    }
                }
            }
        }
    }

    /// My chips to the table (shown at once, the table's own once it has them).
    fn rl_send(&mut self, s: &ServerView, v: &RlView, mut chips: Vec<Chip>, ready: bool) {
        // Each place brought into what this table takes first.
        if rlroom::fit(&mut chips, v.min_bet, v.max_bet) {
            self.toast(format!("Bets here are {} to {} a place: yours set to fit", chips_text(v.min_bet), chips_text(v.max_bet)), false);
        }
        self.poker.casino.rlt.desired = Some((v.spin_no, chips.clone()));
        self.poker.casino.sound_in(0.0, Cue::Chips, 0.0, 0.45);
        self.engine.rl_bets(&s.key, v.id, v.spin_no, chips, ready);
    }

    /// The controls along the bottom: the rack, Rebet or Clear, Double, Spin (I'm ready), and
    /// the clock; or, watching, a seat to sit in.
    fn rl_controls(&mut self, ui: &mut egui::Ui, s: &ServerView, live: &pyr_engine::poker::SharedLive<RlView>, bar: Rect, settling: bool) {
        let p = ui.painter().clone();
        let v = &live.view;
        p.rect_filled(bar, theme::r(18), Color32::from_black_alpha(185));
        p.rect_stroke(bar, theme::r(18), Stroke::new(1.0f32, Color32::from_white_alpha(22)), StrokeKind::Inside);
        let left = v.ms_left.saturating_sub(live.at.elapsed().as_millis() as u64);
        let secs = left.div_ceil(1000);
        let say = |text: String, color: Color32| {
            p.text(pos2(bar.left() + 22.0, bar.center().y), Align2::LEFT_CENTER, text, theme::bold_font(15.0), color);
        };
        let Some(me) = v.my_seat else {
            let free = v.seats.iter().position(|x| x.is_none());
            say(
                match free {
                    Some(_) => format!("{}: {} to {} a place. Sit in an open seat to play.", v.name, chips_text(v.min_bet), chips_text(v.max_bet)),
                    None => format!("{} is full: watch, or try another table.", v.name),
                },
                col::MUTED,
            );
            if let Some(seat) = free {
                let r = Rect::from_min_size(pos2(bar.right() - 176.0, bar.center().y - 24.0), vec2(160.0, 48.0));
                if button(ui, r, "sit", "Sit down", None, col::GOLD, true) {
                    self.poker.casino.rlt.desired = None;
                    self.engine.poker(&s.key, ToServer::RlSit { table: v.id, seat: seat as u8 });
                }
            }
            return;
        };
        let mine = v.seats.get(me as usize).cloned().flatten();
        let on = self.poker.casino.rlt.desired.clone().filter(|(c, _)| *c == v.spin_no).map(|(_, b)| b).or_else(|| mine.as_ref().map(|x| x.chips.clone())).unwrap_or_default();
        let total = rlroom::total(&on);
        let ready = mine.as_ref().is_some_and(|x| x.ready);
        // Stand up, at the far right always.
        let up = Rect::from_min_size(pos2(bar.right() - 126.0, bar.center().y - 22.0), vec2(112.0, 44.0));
        if button(ui, up, "stand", "Stand up", None, Color32::from_white_alpha(30), true) {
            self.engine.poker(&s.key, ToServer::RlLeave { table: v.id });
            self.poker.casino.rlt.desired = None;
            return;
        }
        let mut x = up.left() - 12.0;
        let mut place = |w: f32| {
            let r = Rect::from_min_size(pos2(x - w, bar.center().y - 24.0), vec2(w, 48.0));
            x -= w + 10.0;
            r
        };
        let em = place(56.0);
        if let Some(e) = super::emotes::button(ui, em, "rl-emotes", &mut self.poker.casino.emote_last) {
            self.engine.poker(&s.key, ToServer::GameEmote { game: TableGame::Roulette, table: v.id, emote: e });
        }
        let chips = chips_now(s);
        let open = !settling && !ready;
        let enter = ui.ctx().input(|i| i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Space)) && !ui.ctx().wants_keyboard_input();
        let (label, sub) = match v.stage {
            RlStage::Betting if ready => ("Ready".to_string(), Some(format!("ball in {secs}s"))),
            RlStage::Betting => (format!("Spin  ({})", chips_text(total)), Some(format!("{secs}s to bet · Space"))),
            RlStage::Shown => ("Spin".to_string(), Some(if settling { "the ball's going".to_string() } else { "next spin".to_string() })),
            RlStage::Waiting => ("Spin".to_string(), Some("starts the spin".to_string())),
        };
        let r = place(180.0);
        let can_spin = total > 0 && !ready && v.stage != RlStage::Shown;
        if button(ui, r, "spin", &label, sub, col::GOLD, can_spin) || (can_spin && enter) {
            self.rl_send(s, v, on.clone(), true);
        }
        let r = place(96.0);
        let doubled: Vec<Chip> = on.iter().map(|c| Chip { spot: c.spot, chips: (c.chips * 2).min(v.max_bet.min(rl::max_on(&c.spot))) }).collect();
        let more = rlroom::total(&doubled).saturating_sub(total);
        if button(ui, r, "double", "Double", None, Color32::from_white_alpha(30), open && total > 0 && more > 0 && more <= chips) {
            self.rl_send(s, v, doubled, false);
        }
        let r = place(112.0);
        let rebet = self.poker.casino.rlt.rebet.clone();
        let can_rebet = total == 0 && rlroom::total(&rebet) > 0 && rlroom::total(&rebet) <= chips && open;
        if total > 0 {
            if button(ui, r, "clear", "Clear", None, Color32::from_white_alpha(30), open) {
                self.rl_send(s, v, Vec::new(), false);
            }
        } else if button(ui, r, "rebet", "Rebet", Some(chips_text(rlroom::total(&rebet))), Color32::from_white_alpha(30), can_rebet) {
            self.rl_send(s, v, rebet, false);
        }
        let color = if total > chips + mine.as_ref().map(|x| rlroom::total(&x.chips)).unwrap_or(0) { col::LOSE } else { col::GOLD2 };
        let max_chip = super::CHIPS.iter().copied().filter(|c| *c <= v.max_bet).last().unwrap_or(1);
        self.rack_and_bet(ui, bar, x, max_chip, format!("Bets {}", chips_text(total)), color);
        let hint = if settling {
            "The ball's going round".to_string()
        } else if total == 0 {
            format!("Pick a chip, then click the layout: a number, a line between two, a corner, a row's end, or the outside bets ({} to {} a place)", chips_text(v.min_bet), chips_text(v.max_bet))
        } else {
            String::new()
        };
        if !hint.is_empty() {
            p.text(pos2(bar.center().x, bar.top() - 12.0), Align2::CENTER_CENTER, hint, theme::font(12.5), col::MUTED);
        }
        // How long's left to bet: a line along the top of the controls.
        if v.stage == RlStage::Betting {
            let frac = (left as f32 / BET_MS as f32).clamp(0.0, 1.0);
            let line = Rect::from_min_size(bar.left_top() + vec2(18.0, 0.0), vec2((bar.width() - 36.0) * frac, 3.0));
            p.rect_filled(line, theme::r(2), if frac > 0.3 { col::GOLD } else { col::LOSE });
        }
        // My chips are on the table now (or it said no, or the spin's gone by): the table's own
        // show from here.
        if let Some((spin, d)) = &self.poker.casino.rlt.desired
            && (*spin != v.spin_no || mine.as_ref().is_some_and(|x| x.chips == *d) || s.poker.error.is_some())
        {
            self.poker.casino.rlt.desired = None;
        }
    }
}
