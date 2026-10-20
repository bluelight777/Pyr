//! Baccarat at the app: Player and Banker, the cards dealt in turn (third cards by the
//! tableau), the totals, and the bead road of past coups.

use std::time::Instant;

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::cards::Card;
use pyr_poker::casino::baccarat::{self as bc, Bets, Side};
use pyr_poker::casino::{Bet, Outcome};
use pyr_poker::net::RoundView;

use super::{chip_stack, col, glow_text, oval, vgrad};
use crate::app::PyrApp;
use crate::poker::cards::{self as cards, chips_text};
use crate::theme;

#[derive(Default)]
pub struct BacUi {
    pub bets: Bets,
    last: Bets,
    /// The coup being shown and when its cards started coming out.
    show: Option<(RoundView, Instant, bool)>,
    /// Past coups, oldest first.
    pub road: Vec<Side>,
}

/// When each card of a coup comes out (seconds from the deal): player, banker, player, banker,
/// then third cards.
fn card_times(o: &bc::Outcome) -> (Vec<f32>, Vec<f32>) {
    let mut p = vec![0.0, 0.8];
    let mut b = vec![0.4, 1.2];
    let mut t = 2.0;
    if o.player.len() > 2 {
        p.push(t);
        t += 0.9;
    }
    if o.banker.len() > 2 {
        b.push(t);
    }
    (p, b)
}

fn done_at(o: &bc::Outcome) -> f32 {
    let (p, b) = card_times(o);
    p.iter().chain(b.iter()).fold(0.0f32, |a, x| a.max(*x)) + 0.9
}

impl PyrApp {
    pub(super) fn baccarat_round(&mut self, r: &RoundView) {
        let Outcome::Baccarat(o) = &r.outcome else { return };
        self.poker.casino.baccarat.show = Some((r.clone(), Instant::now(), false));
        let (pt, bt) = card_times(o);
        for t in pt.iter().chain(bt.iter()) {
            self.poker.casino.sound_in(*t, Cue::Deal, 0.0, 0.7);
            self.poker.casino.sound_in(*t + 0.25, Cue::Flip, 0.0, 0.5);
        }
    }

    pub(super) fn baccarat_page(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let now = Instant::now();
        let p = ui.painter().clone();
        vgrad(&p, area, Color32::from_rgb(0x10, 0x08, 0x10), Color32::from_rgb(0x06, 0x04, 0x08));
        let table = Rect::from_center_size(pos2(area.center().x, area.top() + (area.height() - 100.0) / 2.0), vec2(area.width() * 0.9, area.height() - 118.0));
        oval(&p, table.expand(16.0), Color32::from_rgb(0x4a, 0x2a, 0x14), Color32::from_rgb(0x24, 0x12, 0x08));
        oval(&p, table, Color32::from_rgb(0x1c, 0x24, 0x5e), Color32::from_rgb(0x08, 0x0a, 0x24));
        let cx = table.center().x;
        let cw = (area.height() * 0.12).clamp(58.0, 100.0);
        let ch = cw * 1.4;
        let show = self.poker.casino.baccarat.show.clone();
        let t = show.as_ref().map(|(_, at, _)| (now - *at).as_secs_f32()).unwrap_or(0.0);
        // The hands.
        let py = table.top() + table.height() * 0.3;
        let left_c = pos2(cx - table.width() * 0.2, py);
        let right_c = pos2(cx + table.width() * 0.2, py);
        p.text(left_c - vec2(0.0, ch * 0.75 + 10.0), Align2::CENTER_CENTER, "PLAYER", theme::bold_font(18.0), Color32::from_rgb(0x8a, 0xb8, 0xff));
        p.text(right_c - vec2(0.0, ch * 0.75 + 10.0), Align2::CENTER_CENTER, "BANKER", theme::bold_font(18.0), Color32::from_rgb(0xff, 0x8a, 0x8a));
        let shoe = pos2(cx, table.top() + 20.0);
        if let Some((r, _, settled)) = &show
            && let Outcome::Baccarat(o) = &r.outcome
        {
            let (ptimes, btimes) = card_times(o);
            // (`out`: which way is away from the middle, where a third card goes.)
            let draw_hand = |cs: &[Card], times: &[f32], c: Pos2, out: f32| -> Vec<Card> {
                let mut shown = Vec::new();
                for (i, card) in cs.iter().enumerate() {
                    let at = times.get(i).copied().unwrap_or(0.0);
                    if t < at {
                        continue;
                    }
                    let k = ((t - at) / 0.3).min(1.0);
                    let pos = if i < 2 { c + vec2((i as f32 - 0.5) * cw * 1.1, 0.0) } else { c + vec2(out * cw * 1.68, ch * 0.1) };
                    let q = shoe + (pos - shoe) * super::ease_out(k);
                    let flip = ((t - at - 0.2) / 0.2).clamp(0.0, 1.0);
                    if flip > 0.5 {
                        cards::face(&p, Rect::from_center_size(q, vec2(cw * (flip - 0.5) * 2.0, ch)), *card, false, false);
                    } else {
                        cards::back(&p, Rect::from_center_size(q, vec2(cw * (1.0 - flip * 2.0).max(0.05), ch)), "");
                    }
                    if flip >= 1.0 {
                        shown.push(*card);
                    }
                }
                shown
            };
            let ps = draw_hand(&o.player, &ptimes, left_c, -1.0);
            let bs = draw_hand(&o.banker, &btimes, right_c, 1.0);
            for (cs, c) in [(&ps, left_c), (&bs, right_c)] {
                if !cs.is_empty() {
                    let v = bc::total(cs);
                    let r = Rect::from_center_size(c + vec2(0.0, ch * 0.5 + 26.0), vec2(48.0, 32.0));
                    p.rect_filled(r, theme::r(16), Color32::from_black_alpha(190));
                    p.text(r.center(), Align2::CENTER_CENTER, v.to_string(), theme::bold_font(20.0), Color32::WHITE);
                }
            }
            let end = done_at(o);
            if t >= end {
                let (word, color) = match o.winner {
                    Side::Player => ("PLAYER WINS", Color32::from_rgb(0x8a, 0xb8, 0xff)),
                    Side::Banker => ("BANKER WINS", Color32::from_rgb(0xff, 0x8a, 0x8a)),
                    Side::Tie => ("TIE", col::GREEN),
                };
                let natural = (o.player.len() == 2 && o.player_total >= 8) || (o.banker.len() == 2 && o.banker_total >= 8);
                let line = format!("{word}  {} to {}{}", o.player_total.max(o.banker_total), o.player_total.min(o.banker_total), if natural { "  ·  natural" } else { "" });
                glow_text(&p, pos2(cx, py + ch * 0.95), Align2::CENTER_CENTER, &line, theme::bold_font(26.0), color, Color32::from_black_alpha(160));
                if o.paid > 0 {
                    p.text(pos2(cx, py + ch * 0.95 + 30.0), Align2::CENTER_CENTER, format!("You won {}", chips_text(o.paid)), theme::bold_font(17.0), col::WIN);
                }
                if !*settled {
                    let ui_s = &mut self.poker.casino.baccarat;
                    ui_s.road.push(o.winner);
                    if ui_s.road.len() > 72 {
                        ui_s.road.remove(0);
                    }
                    if let Some(sh) = ui_s.show.as_mut() {
                        sh.2 = true;
                    }
                    if o.paid > 0 {
                        self.poker.casino.sound_in(0.0, Cue::WinSmall, 0.0, 0.8);
                    }
                    self.poker.casino.settle_balance();
                    self.poker.casino.celebrate(o.paid, o.bets.total(), false, 0.1);
                }
            }
        }
        // The betting spots: Player, Tie and Banker in a row, the pairs under Player and Banker.
        let row_y = table.center().y + table.height() * 0.2;
        let unit = (table.width() * 0.62 / 660.0).clamp(0.75, 1.25);
        let (main_w, tie_w, gap) = (230.0 * unit, 170.0 * unit, 16.0 * unit);
        let side_x = tie_w / 2.0 + gap + main_w / 2.0;
        let pair_y = row_y + 37.0 * unit + 10.0 + 25.0 * unit;
        let spots: [(&str, &str, Pos2, Vec2Size, Color32, u8); 5] = [
            ("PLAYER", "pays 1 to 1", pos2(cx - side_x, row_y), Vec2Size(main_w, 74.0 * unit), Color32::from_rgb(0x2a, 0x5a, 0xd0), 0),
            ("TIE", "pays 8 to 1", pos2(cx, row_y), Vec2Size(tie_w, 74.0 * unit), col::GREEN, 2),
            ("BANKER", "pays 0.95 to 1", pos2(cx + side_x, row_y), Vec2Size(main_w, 74.0 * unit), col::RED, 1),
            ("P PAIR", "11 to 1", pos2(cx - side_x, pair_y), Vec2Size(main_w * 0.72, 50.0 * unit), Color32::from_rgb(0x2a, 0x5a, 0xd0), 3),
            ("B PAIR", "11 to 1", pos2(cx + side_x, pair_y), Vec2Size(main_w * 0.72, 50.0 * unit), col::RED, 4),
        ];
        let can_bet = show.as_ref().is_none_or(|(r, _, _)| matches!(&r.outcome, Outcome::Baccarat(o) if t >= done_at(o)));
        // (The side that won glows, once the coup is shown.)
        let winner = show.as_ref().and_then(|(r, _, _)| match &r.outcome {
            Outcome::Baccarat(o) if t >= done_at(o) => Some(o.winner),
            _ => None,
        });
        for (label, pays, c, size, color, which) in spots {
            let r = Rect::from_center_size(c, vec2(size.0, size.1));
            let resp = ui.interact(r, ui.id().with(("bac", which)), Sense::click());
            let won = matches!((winner, which), (Some(Side::Player), 0) | (Some(Side::Banker), 1) | (Some(Side::Tie), 2));
            if won {
                p.rect_filled(r.expand(5.0), theme::r(18), color.gamma_multiply(0.35));
            }
            p.rect_filled(r, theme::r(14), color.gamma_multiply(if resp.hovered() && can_bet { 0.5 } else { 0.3 }));
            p.rect_stroke(r, theme::r(14), Stroke::new(if won { 3.0 } else { 2.0 }, if won { col::GOLD2 } else { color }), StrokeKind::Inside);
            let big = size.1 > 60.0;
            p.text(r.left_center() + vec2(16.0, if big { -10.0 } else { -7.0 }), Align2::LEFT_CENTER, label, theme::bold_font(if big { 18.0 } else { 13.0 }), Color32::WHITE);
            p.text(r.left_center() + vec2(16.0, if big { 13.0 } else { 9.0 }), Align2::LEFT_CENTER, pays, theme::font(if big { 12.5 } else { 11.0 }), Color32::from_white_alpha(190));
            let b = &mut self.poker.casino.baccarat.bets;
            let slot = match which {
                0 => &mut b.player,
                1 => &mut b.banker,
                2 => &mut b.tie,
                3 => &mut b.player_pair,
                _ => &mut b.banker_pair,
            };
            let chip = self.poker.casino.chip;
            let mut placed = false;
            if can_bet && resp.clicked() {
                let max = if which >= 2 { 5_000 } else { 50_000 };
                *slot = (*slot + chip).min(max);
                placed = true;
            }
            if can_bet && resp.secondary_clicked() {
                *slot = 0;
            }
            let amount = *slot;
            if placed {
                self.poker.casino.sound_in(0.0, Cue::Chips, 0.0, 0.4);
            }
            if amount > 0 {
                let cr = if big { 17.0 } else { 12.0 };
                chip_stack(&p, pos2(r.right() - cr - 14.0, r.center().y + cr * 0.5), cr, amount);
            }
        }
        // The bead road.
        let road = self.poker.casino.baccarat.road.clone();
        let cell = 18.0;
        let rows = 6;
        let cols_n = 12;
        let rr = Rect::from_min_size(pos2(area.right() - 24.0 - cols_n as f32 * cell, area.top() + 16.0), vec2(cols_n as f32 * cell, rows as f32 * cell));
        p.rect_filled(rr.expand(6.0), theme::r(8), Color32::from_black_alpha(160));
        for k in 0..rows * cols_n {
            let c = rr.min + vec2((k / rows) as f32 * cell + cell / 2.0, (k % rows) as f32 * cell + cell / 2.0);
            p.circle_stroke(c, cell * 0.38, Stroke::new(1.0f32, Color32::from_white_alpha(22)));
        }
        let start = road.len().saturating_sub(rows * cols_n);
        for (k, side) in road[start..].iter().enumerate() {
            let c = rr.min + vec2((k / rows) as f32 * cell + cell / 2.0, (k % rows) as f32 * cell + cell / 2.0);
            let (color, letter) = match side {
                Side::Player => (Color32::from_rgb(0x2a, 0x5a, 0xd0), "P"),
                Side::Banker => (col::RED, "B"),
                Side::Tie => (col::GREEN, "T"),
            };
            p.circle_filled(c, cell * 0.42, color);
            p.text(c, Align2::CENTER_CENTER, letter, theme::bold_font(10.0), Color32::WHITE);
        }
        let count = |w: Side| road.iter().filter(|x| **x == w).count();
        p.text(rr.left_bottom() + vec2(0.0, 16.0), Align2::LEFT_CENTER, "BEAD ROAD", theme::bold_font(10.5), col::MUTED);
        p.text(rr.right_bottom() + vec2(0.0, 16.0), Align2::RIGHT_CENTER, format!("P {}  ·  B {}  ·  T {}", count(Side::Player), count(Side::Banker), count(Side::Tie)), theme::bold_font(11.0), col::TEXT);
        // The controls.
        let bar = Rect::from_min_max(pos2(area.left() + 20.0, area.bottom() - 84.0), pos2(area.right() - 20.0, area.bottom() - 14.0));
        self.baccarat_controls(ui, s, bar, can_bet);
    }

    fn baccarat_controls(&mut self, ui: &mut egui::Ui, s: &ServerView, bar: Rect, can_bet: bool) {
        let p = ui.painter().clone();
        p.rect_filled(bar, theme::r(18), Color32::from_black_alpha(170));
        self.chip_rack(ui, pos2(bar.left() + 16.0, bar.center().y - 26.0), 25_000);
        let bets = self.poker.casino.baccarat.bets.clone();
        let total = bets.total();
        let c = &s.poker.casino;
        let busy = self.poker.casino.busy() || !can_bet;
        let mut x = bar.right() - 16.0;
        let mut button = |ui: &mut egui::Ui, text: &str, w: f32, primary: bool, enabled: bool| -> bool {
            let r = Rect::from_min_size(pos2(x - w, bar.center().y - 22.0), vec2(w, 44.0));
            x -= w + 10.0;
            let resp = ui.interact(r, ui.id().with(("bc", text)), Sense::click());
            let fill = if !enabled { Color32::from_white_alpha(18) } else if primary { if resp.hovered() { col::GOLD2 } else { col::GOLD } } else if resp.hovered() { Color32::from_white_alpha(46) } else { Color32::from_white_alpha(24) };
            ui.painter().rect_filled(r, theme::r(12), fill);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(15.0), if primary && enabled { Color32::from_rgb(0x2a, 0x10, 0x04) } else if enabled { Color32::WHITE } else { Color32::from_white_alpha(70) });
            enabled && resp.clicked()
        };
        let enter = ui.ctx().input(|i| i.key_pressed(egui::Key::Enter)) && !ui.ctx().wants_keyboard_input();
        let ok = total >= 10 && total <= super::chips_now(s) && bets.check().is_ok() && !busy;
        if (button(ui, "Deal", 130.0, true, ok) || (enter && ok))
            && let Some(commit) = c.commit
        {
            self.poker.casino.baccarat.last = bets.clone();
            self.poker.casino.pending = Some(Instant::now());
            self.poker.casino.celebration = None;
            self.engine.casino_play(&s.key, commit, Bet::Baccarat(bets.clone()));
        }
        let last = self.poker.casino.baccarat.last.clone();
        if button(ui, "Rebet", 90.0, false, !busy && last.total() > 0 && total == 0) {
            self.poker.casino.baccarat.bets = last;
        }
        if button(ui, "Clear", 80.0, false, !busy && total > 0) {
            self.poker.casino.baccarat.bets = Bets::default();
        }
        p.text(pos2(x - 10.0, bar.center().y), Align2::RIGHT_CENTER, format!("Bet {}", chips_text(total)), theme::bold_font(16.0), if total > super::chips_now(s) { col::LOSE } else { col::GOLD2 });
        if total == 0 && can_bet {
            p.text(pos2(bar.center().x, bar.top() - 14.0), Align2::CENTER_CENTER, "Pick a chip, then click Player, Banker or Tie (right-click a bet to take it off)", theme::font(12.5), col::MUTED);
        } else if total > 0 && total < 10 {
            p.text(pos2(bar.center().x, bar.top() - 14.0), Align2::CENTER_CENTER, "The least bet is 10", theme::font(12.5), col::MUTED);
        }
    }
}

/// A spot's size.
#[derive(Clone, Copy)]
struct Vec2Size(f32, f32);
