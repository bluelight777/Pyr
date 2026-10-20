//! Blackjack at the app: the felt, the shoe, cards sliding out and turning over, the dealer's
//! hole card, split hands side by side, and the moves (with basic strategy's hint, if wanted).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::cards::Card;
use pyr_poker::casino::blackjack::{self as bj, Action, Hand, Phase};
use pyr_poker::casino::{Bet, Outcome};
use pyr_poker::net::RoundView;

use super::{chip_stack, col, oval, vgrad};
use crate::app::PyrApp;
use crate::poker::cards::{self as cards, chips_text};
use crate::theme;

#[derive(Default)]
pub struct BjUi {
    pub bet: u64,
    last_bet: u64,
    /// When each card came out (dealer: hand -1), to slide it in.
    appear: HashMap<(i32, usize), Instant>,
    /// The hand that's over, shown until the next deal; when its results show, and when the
    /// dealer's hole card turns over.
    over: Option<(bj::Outcome, Instant, Instant)>,
    pub hint: bool,
}

impl BjUi {
    /// Notes cards not seen before, each coming out `gap` after the last.
    fn note(&mut self, dealer: &[Card], hands: &[Hand], start: Instant, gap: f32) -> Instant {
        let mut t = start;
        let mut keys: Vec<(i32, usize)> = Vec::new();
        // In dealing order for a fresh hand: player, dealer, player, dealer; then as they come.
        if self.appear.is_empty() && hands.len() == 1 && hands[0].cards.len() == 2 {
            keys.extend([(0, 0), (-1, 0), (0, 1), (-1, 1)]);
        }
        for (h, hand) in hands.iter().enumerate() {
            for i in 0..hand.cards.len() {
                keys.push((h as i32, i));
            }
        }
        for i in 0..dealer.len().max(2) {
            keys.push((-1, i));
        }
        for k in keys {
            if let std::collections::hash_map::Entry::Vacant(e) = self.appear.entry(k) {
                e.insert(t);
                t += Duration::from_secs_f32(gap);
            }
        }
        t
    }
}

impl PyrApp {
    pub(super) fn blackjack_round(&mut self, r: &RoundView) {
        let Outcome::Blackjack(o) = &r.outcome else { return };
        let now = Instant::now();
        let ui = &mut self.poker.casino.bj;
        if ui.over.is_some() {
            ui.appear.clear();
        }
        // The hole card turns over, then the dealer's draws come out one by one.
        let hole_at = ui.note(&[], &o.hands, now, 0.35).max(now + Duration::from_secs_f32(0.3));
        let dealer_keys: Vec<usize> = (2..o.dealer.len()).collect();
        let mut t = hole_at + Duration::from_secs_f32(0.5);
        for i in dealer_keys {
            ui.appear.entry((-1, i)).or_insert_with(|| {
                let at = t;
                t += Duration::from_secs_f32(0.6);
                at
            });
        }
        let results_at = t;
        ui.over = Some((o.clone(), results_at, hole_at));
        let n = o.dealer.len().saturating_sub(2);
        self.poker.casino.sound_in(hole_at.saturating_duration_since(now).as_secs_f32(), Cue::Flip, 0.0, 0.7);
        for k in 0..n {
            self.poker.casino.sound_in(hole_at.saturating_duration_since(now).as_secs_f32() + 0.5 + k as f32 * 0.6, Cue::Deal, 0.0, 0.7);
        }
        let back = results_at.saturating_duration_since(now).as_secs_f32();
        if o.paid > o.wagered {
            self.poker.casino.sound_in(back, Cue::WinSmall, 0.0, 0.8);
            self.poker.casino.sound_in(back + 0.2, Cue::Chips, 0.0, 0.7);
        }
        self.poker.casino.settle_at(results_at);
        self.poker.casino.celebrate(o.paid, o.wagered, false, back);
    }

    pub(super) fn blackjack_page(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let now = Instant::now();
        let p = ui.painter().clone();
        let c = s.poker.casino.clone();
        // The felt: a half-moon table, the dealer along its straight side at the top and its
        // curved rail just above the controls.
        vgrad(&p, area, Color32::from_rgb(0x0c, 0x0a, 0x10), Color32::from_rgb(0x06, 0x05, 0x08));
        let rail_y = area.bottom() - 104.0;
        let center_y = area.top() - area.height() * 0.4;
        let table = Rect::from_center_size(pos2(area.center().x, center_y), vec2(area.width() * 1.16, (rail_y - center_y) * 2.0));
        oval(&p, table.expand(20.0), Color32::from_rgb(0x5a, 0x34, 0x18), Color32::from_rgb(0x2a, 0x14, 0x08));
        p.add(egui::epaint::EllipseShape::stroke(table.center(), table.size() / 2.0 + vec2(20.0, 20.0), Stroke::new(2.0f32, Color32::from_rgb(0x7a, 0x4a, 0x22))));
        oval(&p, table, Color32::from_rgb(0x18, 0x7a, 0x4e), col::FELT_DARK);
        p.add(egui::epaint::EllipseShape::stroke(table.center(), table.size() / 2.0, Stroke::new(2.0f32, col::GOLD_DARK)));
        p.add(egui::epaint::EllipseShape::stroke(table.center(), table.size() / 2.0 - vec2(46.0, 46.0), Stroke::new(1.0f32, col::GOLD.gamma_multiply(0.25))));
        let cx = area.center().x;
        let top = area.top();
        p.text(pos2(cx, top + area.height() * 0.37), Align2::CENTER_CENTER, "BLACKJACK PAYS 3 TO 2", theme::bold_font(22.0), col::GOLD.gamma_multiply(0.55));
        p.text(pos2(cx, top + area.height() * 0.37 + 26.0), Align2::CENTER_CENTER, "Dealer stands on soft 17  ·  Insurance pays 2 to 1", theme::bold_font(13.0), Color32::from_white_alpha(70));
        // The shoe.
        let shoe = Rect::from_center_size(pos2(area.right() - 110.0, top + 70.0), vec2(90.0, 64.0));
        p.rect_filled(shoe.translate(vec2(0.0, 6.0)), theme::r(8), Color32::from_black_alpha(100));
        p.rect_filled(shoe, theme::r(8), Color32::from_rgb(0x3a, 0x22, 0x14));
        cards::back(&p, Rect::from_center_size(shoe.center() + vec2(-10.0, -4.0), vec2(44.0, 60.0)), "");

        let cw = (area.height() * 0.13).clamp(64.0, 110.0);
        let ch = cw * 1.4;
        let live = c.blackjack.clone();
        let over = self.poker.casino.bj.over.clone();
        // Cards in play come from the view; a finished hand from its outcome.
        let (dealer, hole_hidden, hands, active, phase) = match (&live, &over) {
            (Some(v), _) => (v.dealer.clone(), v.hole_hidden, v.hands.clone(), v.active, Some(v.phase)),
            (None, Some((o, _, _))) => (o.dealer.clone(), false, o.hands.clone(), usize::MAX, None),
            _ => (vec![], false, vec![], 0, None),
        };
        if let Some(v) = &live {
            let gap = 0.32;
            let ui_s = &mut self.poker.casino.bj;
            let before = ui_s.appear.len();
            let _ = ui_s.note(&v.dealer, &v.hands, now, gap);
            let added = ui_s.appear.len() - before;
            for k in 0..added {
                self.poker.casino.sound_in(k as f32 * gap, Cue::Deal, 0.3, 0.7);
            }
        }
        let shoe_at = shoe.center();
        // The dealer's cards.
        let dy = top + 40.0 + ch / 2.0;
        let dn = dealer.len().max(if hole_hidden { 2 } else { 0 });
        for i in 0..dn {
            let at = pos2(cx + (i as f32 - (dn as f32 - 1.0) / 2.0) * cw * 0.62, dy);
            let appear = self.poker.casino.bj.appear.get(&(-1, i)).copied().unwrap_or(now);
            let face = dealer.get(i).copied();
            let hidden = i == 1 && (hole_hidden || over.as_ref().is_some_and(|(_, _, hole)| now < *hole));
            draw_card(&p, shoe_at, at, cw, ch, face, hidden, appear, now);
        }
        if !dealer.is_empty() {
            // (The total of the cards face up so far: it climbs as the dealer draws.)
            let hole_down = hole_hidden || over.as_ref().is_some_and(|(_, _, hole)| now < *hole + Duration::from_millis(250));
            let seen: Vec<Card> = dealer
                .iter()
                .enumerate()
                .filter(|(i, _)| !(*i == 1 && hole_down) && self.poker.casino.bj.appear.get(&(-1, *i)).is_none_or(|t| now >= *t + Duration::from_millis(250)))
                .map(|(_, c)| *c)
                .collect();
            if !seen.is_empty() {
                let (v, soft) = bj::value(&seen);
                total_pill(&p, pos2(cx, dy + ch / 2.0 + 22.0), v, soft && v < 21, bj::is_blackjack(&seen), v > 21);
            }
            p.text(pos2(cx, top + 22.0), Align2::CENTER_CENTER, "DEALER", theme::bold_font(12.0), Color32::from_white_alpha(120));
        }
        // My hands.
        let py = top + area.height() * 0.6;
        let n = hands.len().max(1) as f32;
        let span = (cw * 2.6).min((area.width() - 120.0) / n);
        let results_shown = over.as_ref().is_some_and(|(_, t, _)| now >= *t);
        for (h, hand) in hands.iter().enumerate() {
            let hx = cx + (h as f32 - (n - 1.0) / 2.0) * span;
            let k = hand.cards.len() as f32;
            if h == active && phase == Some(Phase::Playing) {
                p.add(egui::epaint::EllipseShape::filled(pos2(hx, py + ch * 0.55), vec2(cw * 1.4, 22.0), col::GOLD.gamma_multiply(0.18)));
            }
            for (i, card) in hand.cards.iter().enumerate() {
                let at = pos2(hx + (i as f32 - (k - 1.0) / 2.0) * cw * 0.34, py - i as f32 * 14.0);
                let appear = self.poker.casino.bj.appear.get(&(h as i32, i)).copied().unwrap_or(now);
                draw_card(&p, shoe_at, at, cw, ch, Some(*card), false, appear, now);
            }
            let seen: Vec<Card> = hand
                .cards
                .iter()
                .enumerate()
                .filter(|(i, _)| self.poker.casino.bj.appear.get(&(h as i32, *i)).is_none_or(|t| now >= *t + Duration::from_millis(250)))
                .map(|(_, c)| *c)
                .collect();
            if !seen.is_empty() {
                let (v, soft) = bj::value(&seen);
                total_pill(&p, pos2(hx, py + ch / 2.0 + 22.0), v, soft && v < 21, seen.len() == 2 && hand.is_blackjack(), v > 21);
            }
            chip_stack(&p, pos2(hx, py + ch / 2.0 + 70.0), 18.0, hand.bet);
            if hand.doubled {
                p.text(pos2(hx + 46.0, py + ch / 2.0 + 64.0), Align2::LEFT_CENTER, "DOUBLED", theme::bold_font(10.5), col::GOLD);
            }
            if results_shown && let Some(res) = hand.result {
                let (text, color) = match res {
                    bj::Result::Blackjack => (format!("Blackjack!  +{}", chips_text(hand.paid - hand.bet)), col::GOLD2),
                    bj::Result::Win => (format!("Win  +{}", chips_text(hand.paid - hand.bet)), col::WIN),
                    bj::Result::Push => ("Push".to_string(), Color32::from_rgb(0xc8, 0xc8, 0xd0)),
                    bj::Result::Lose => ("Lose".to_string(), col::LOSE),
                    bj::Result::Bust => ("Bust".to_string(), col::LOSE),
                };
                let w = 30.0 + text.len() as f32 * 8.5;
                let r = Rect::from_center_size(pos2(hx, py - ch / 2.0 - 32.0 - (k - 1.0).max(0.0) * 14.0), vec2(w, 34.0));
                p.rect_filled(r, theme::r(17), Color32::from_black_alpha(200));
                p.rect_stroke(r, theme::r(17), Stroke::new(1.5f32, color), StrokeKind::Inside);
                p.text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(16.0), color);
            }
        }
        if let Some((o, t, _)) = &over
            && now >= *t
            && o.insurance > 0
        {
            let text = if o.dealer_blackjack { format!("Insurance pays {}", chips_text(o.insurance * 3)) } else { "Insurance lost".into() };
            p.text(pos2(cx, dy + ch / 2.0 + 52.0), Align2::CENTER_CENTER, text, theme::bold_font(13.0), col::GOLD);
        }
        // Betting circle (no hand in play).
        let bet_c = pos2(cx, py + 10.0);
        if live.is_none() && (over.is_none() || results_shown) {
            let bet = self.poker.casino.bj.bet;
            let ring = Rect::from_center_size(bet_c + if over.is_some() { vec2(0.0, ch * 0.9) } else { vec2(0.0, 0.0) }, vec2(120.0, 120.0));
            let resp = ui.interact(ring, ui.id().with("bj-circle"), Sense::click());
            if over.is_none() {
                p.circle_stroke(ring.center(), 56.0, Stroke::new(2.0f32, col::GOLD.gamma_multiply(if resp.hovered() { 0.9 } else { 0.5 })));
                p.text(ring.center() + vec2(0.0, 40.0), Align2::CENTER_CENTER, "BET", theme::bold_font(11.0), col::GOLD.gamma_multiply(0.6));
                chip_stack(&p, ring.center() + vec2(0.0, 8.0), 22.0, bet);
            }
            if resp.clicked() {
                let chip = self.poker.casino.chip;
                self.poker.casino.bj.bet = (bet + chip).min(50_000);
                self.poker.casino.bj.over = None;
                self.poker.casino.bj.appear.clear();
                self.poker.casino.sound_in(0.0, Cue::Chips, 0.0, 0.5);
            }
            if resp.secondary_clicked() {
                self.poker.casino.bj.bet = 0;
            }
        }
        // The controls.
        let bar = Rect::from_min_max(pos2(area.left() + 20.0, area.bottom() - 84.0), pos2(area.right() - 20.0, area.bottom() - 14.0));
        self.blackjack_controls(ui, s, bar, live.as_ref());
    }

    fn blackjack_controls(&mut self, ui: &mut egui::Ui, s: &ServerView, bar: Rect, live: Option<&bj::View>) {
        let p = ui.painter().clone();
        p.rect_filled(bar, theme::r(18), Color32::from_black_alpha(170));
        let busy = self.poker.casino.busy();
        let c = &s.poker.casino;
        let keys = ui.ctx().input(|i| (i.key_pressed(egui::Key::H), i.key_pressed(egui::Key::S), i.key_pressed(egui::Key::D), i.key_pressed(egui::Key::P), i.key_pressed(egui::Key::Enter)));
        let typing = ui.ctx().wants_keyboard_input();
        match live {
            Some(v) => {
                // The moves.
                let hint = if self.poker.casino.bj.hint && v.phase == Phase::Playing {
                    v.hands.get(v.active).map(|h| bj::basic(&h.cards, v.dealer[0], v.allowed.contains(&Action::Double), v.allowed.contains(&Action::Split)))
                } else {
                    None
                };
                let moves: Vec<(Action, &str, bool)> = if v.phase == Phase::Insurance {
                    vec![(Action::Insure, "Insurance", false), (Action::NoInsurance, "No insurance", false)]
                } else {
                    vec![(Action::Hit, "Hit  (H)", keys.0), (Action::Stand, "Stand  (S)", keys.1), (Action::Double, "Double  (D)", keys.2), (Action::Split, "Split  (P)", keys.3)]
                };
                let w = 150.0;
                let total = moves.len() as f32 * (w + 12.0) - 12.0;
                let mut x = bar.center().x - total / 2.0;
                for (a, label, key) in moves {
                    let r = Rect::from_min_size(pos2(x, bar.center().y - 24.0), vec2(w, 48.0));
                    x += w + 12.0;
                    let ok = v.allowed.contains(&a) && !busy;
                    let resp = ui.interact(r, ui.id().with(("bj", label)), Sense::click());
                    let fill = match a {
                        Action::Hit => Color32::from_rgb(0x2a, 0x8a, 0x4a),
                        Action::Stand => Color32::from_rgb(0xb8, 0x2a, 0x30),
                        Action::Double => Color32::from_rgb(0xc8, 0x8a, 0x1a),
                        Action::Split => Color32::from_rgb(0x2a, 0x5a, 0xc0),
                        _ => Color32::from_rgb(0x55, 0x48, 0x60),
                    };
                    p.rect_filled(r, theme::r(12), if ok { if resp.hovered() { fill.gamma_multiply(1.2) } else { fill } } else { Color32::from_white_alpha(18) });
                    if hint == Some(a) {
                        p.rect_stroke(r.expand(3.0), theme::r(14), Stroke::new(2.5f32, col::GOLD2), StrokeKind::Outside);
                    }
                    let cost = if matches!(a, Action::Double | Action::Split) { v.hands.get(v.active).map(|h| h.bet).unwrap_or(0) } else if a == Action::Insure { v.base / 2 } else { 0 };
                    p.text(r.center() - vec2(0.0, if cost > 0 { 7.0 } else { 0.0 }), Align2::CENTER_CENTER, label, theme::bold_font(15.0), if ok { Color32::WHITE } else { Color32::from_white_alpha(70) });
                    if cost > 0 {
                        p.text(r.center() + vec2(0.0, 12.0), Align2::CENTER_CENTER, format!("+{}", chips_text(cost)), theme::font(11.5), if ok { Color32::from_white_alpha(200) } else { Color32::from_white_alpha(60) });
                    }
                    if ok && (resp.clicked() || (key && !typing)) {
                        self.poker.casino.pending = Some(Instant::now());
                        self.engine.casino_blackjack(&s.key, a);
                        if matches!(a, Action::Double | Action::Split | Action::Insure) {
                            self.poker.casino.sound_in(0.0, Cue::Chips, 0.0, 0.6);
                        }
                    }
                }
            }
            None => {
                self.chip_rack(ui, pos2(bar.left() + 16.0, bar.center().y - 26.0), 25_000);
                let bet = self.poker.casino.bj.bet;
                let last = self.poker.casino.bj.last_bet;
                let mut x = bar.right() - 16.0;
                let mut button = |ui: &mut egui::Ui, text: &str, w: f32, primary: bool, enabled: bool| -> bool {
                    let r = Rect::from_min_size(pos2(x - w, bar.center().y - 22.0), vec2(w, 44.0));
                    x -= w + 10.0;
                    let resp = ui.interact(r, ui.id().with(("bj-b", text)), Sense::click());
                    let fill = if !enabled { Color32::from_white_alpha(18) } else if primary { if resp.hovered() { col::GOLD2 } else { col::GOLD } } else if resp.hovered() { Color32::from_white_alpha(46) } else { Color32::from_white_alpha(24) };
                    ui.painter().rect_filled(r, theme::r(12), fill);
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(15.0), if primary && enabled { Color32::from_rgb(0x2a, 0x10, 0x04) } else if enabled { Color32::WHITE } else { Color32::from_white_alpha(70) });
                    enabled && resp.clicked()
                };
                let can = bet >= 10 && bet <= super::chips_now(s) && !busy;
                if button(ui, "Deal  (Enter)", 170.0, true, can) || (can && keys.4 && !typing) {
                    if let Some(commit) = c.commit {
                        self.poker.casino.bj.last_bet = bet;
                        self.poker.casino.bj.over = None;
                        self.poker.casino.bj.appear.clear();
                        self.poker.casino.celebration = None;
                        self.poker.casino.pending = Some(Instant::now());
                        self.engine.casino_play(&s.key, commit, Bet::Blackjack { chips: bet });
                    }
                }
                let rebet = bet == 0 && last > 0;
                if button(ui, if rebet { "Rebet" } else { "Clear" }, 110.0, false, rebet || bet > 0) {
                    self.poker.casino.bj.bet = if rebet { last } else { 0 };
                    self.poker.casino.bj.over = None;
                    self.poker.casino.bj.appear.clear();
                }
                let hint = self.poker.casino.bj.hint;
                if button(ui, if hint { "Hints on" } else { "Hints off" }, 110.0, false, true) {
                    self.poker.casino.bj.hint = !hint;
                }
                p.text(pos2(x - 10.0, bar.center().y), Align2::RIGHT_CENTER, format!("Bet {}", chips_text(bet)), theme::bold_font(16.0), if bet > super::chips_now(s) { col::LOSE } else { col::GOLD2 });
                if bet > 0 && bet < 10 {
                    p.text(pos2(bar.center().x, bar.top() - 14.0), Align2::CENTER_CENTER, "The least bet is 10", theme::font(12.5), col::MUTED);
                } else if bet == 0 {
                    p.text(pos2(bar.center().x, bar.top() - 14.0), Align2::CENTER_CENTER, "Pick a chip and click the circle to bet (right-click it to clear)", theme::font(12.5), col::MUTED);
                }
            }
        }
    }
}

/// A card sliding from the shoe to `at` from `appear`, turning over as it lands (or staying
/// face down).
#[allow(clippy::too_many_arguments)]
fn draw_card(p: &egui::Painter, from: Pos2, at: Pos2, cw: f32, ch: f32, face: Option<Card>, hidden: bool, appear: Instant, now: Instant) {
    if now < appear {
        return;
    }
    let k = ((now - appear).as_secs_f32() / 0.32).min(1.0);
    let e = super::ease_out(k);
    let c = from + (at - from) * e;
    // Turning over in the last part of the slide.
    let flip = ((k - 0.55) / 0.45).clamp(0.0, 1.0);
    match face {
        Some(card) if !hidden && flip > 0.5 => {
            let w = cw * (flip - 0.5) * 2.0;
            cards::face(p, Rect::from_center_size(c, vec2(w.max(2.0), ch)), card, false, false);
        }
        _ => {
            let w = if hidden { cw } else { cw * (1.0 - flip * 2.0).abs().max(0.04) };
            cards::back(p, Rect::from_center_size(c, vec2(w, ch)), "");
        }
    }
}

/// A hand's total in a pill under it.
fn total_pill(p: &egui::Painter, at: Pos2, v: u8, soft: bool, blackjack: bool, bust: bool) {
    let text = if blackjack {
        "BLACKJACK".to_string()
    } else if bust {
        format!("{v} BUST")
    } else if soft {
        format!("{} / {v}", v - 10)
    } else {
        v.to_string()
    };
    let w = 24.0 + text.len() as f32 * 8.5;
    let r = Rect::from_center_size(at, vec2(w, 26.0));
    let color = if blackjack { col::GOLD } else if bust { col::RED } else { Color32::from_rgb(0x20, 0x20, 0x26) };
    p.rect_filled(r, theme::r(13), color);
    p.text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(14.0), if blackjack { Color32::BLACK } else { Color32::WHITE });
}
