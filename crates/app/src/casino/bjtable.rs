//! Blackjack at a table of seven (0.12.0 servers): people and bots in the seats against one
//! dealer, a round every few seconds. The table is the Meshy art (`table-blackjack.webp`), each
//! seat on one of its betting circles. Cards slide out of the shoe in dealing order, the
//! dealer's hole card turns over once the seats are done, and every round is checked by this
//! Pyr (the engine plays it again from its seeds: see `pyr_engine::poker::BjLive`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::cards::Card;
use pyr_poker::casino::bjtable::{RoundView, SEATS, SeatPlay};
use pyr_poker::casino::blackjack::{self as bj, Action, Phase};
use pyr_poker::net::{BjSeatView, BjStage, BjView, TableGame, ToServer};

use super::{chip_stack, chips_now, col, ease_out, hgrad, vgrad};
use crate::app::PyrApp;
use crate::poker::cards::{self as cards, chips_text};
use crate::theme;

/// The art's size, and the part of it that always shows (the table, rail to rail).
const ART: Vec2 = vec2(1376.0, 774.0);
const SHOW: Rect = Rect { min: pos2(166.0, 64.0), max: pos2(1214.0, 702.0) };
/// Each seat's betting circle on the art: first base (seat 0, on the dealer's left) first.
const CIRCLES: [(f32, f32); SEATS] = [(1052.0, 340.0), (970.0, 452.0), (842.0, 530.0), (688.0, 556.0), (535.0, 530.0), (407.0, 452.0), (324.0, 340.0)];
const CIRCLE_R: f32 = 31.0;
/// The dealer's cards (under the chip tray), and the shoe they come out of.
const DEALER_AT: (f32, f32) = (688.0, 292.0);
const SHOE_AT: (f32, f32) = (978.0, 178.0);
/// The dealer among the seats (in the cards' keys).
const DEALER: u8 = u8::MAX;
/// Between cards as they're dealt; the dealer's draws come slower; a card's slide.
const DEAL_GAP: f32 = 0.2;
const DRAW_GAP: f32 = 0.55;
const SLIDE: f32 = 0.3;
/// The controls along the bottom, and the tables along the top.
const BAR_H: f32 = 96.0;
const STRIP_H: f32 = 44.0;

#[derive(Default)]
pub struct BjTableUi {
    /// The table I'm at (watching, or sitting): its server and id.
    pub at: Option<(String, u64)>,
    /// When the tables were last asked for, and the table to watch.
    asked: Option<Instant>,
    watch_asked: Option<Instant>,
    /// A table I opened: the newest table (above this id) is the one to go to.
    follow_new: Option<u64>,
    /// My bet for the next round as I put it together, my last one, and whether it's placed
    /// again by itself every round; a bet made while the last round was still on the table.
    pub bet: u64,
    pub last_bet: u64,
    pub auto: bool,
    queued: bool,
    /// The round I've bet in (a bet goes once a round).
    bet_round: Option<u64>,
    /// When each card came out, and the card it was: by seat (`DEALER` for the dealer), hand
    /// and card.
    appear: HashMap<(u8, usize, usize), (Instant, Card)>,
    /// The round on the table (its cards are forgotten when the next one's dealt), whether its
    /// hole card was seen face down here (so its turning over is shown), and the last round
    /// whose betting was watched here (its deal is shown coming out).
    deal: Option<u64>,
    hole_seen: bool,
    betting_seen: Option<u64>,
    /// A round over: when the hole card turns over, and when the results show.
    reveal: Option<(Instant, Instant)>,
    /// My chips while the round played (the balance waits on them until the results show).
    playing_chips: Option<u64>,
    pub hint: bool,
}

/// Where the art is on the screen: `origin` + a point on the art × `k`.
#[derive(Clone, Copy)]
struct Fit {
    k: f32,
    origin: Pos2,
}

impl Fit {
    fn new(room: Rect) -> Fit {
        let k = (room.width() / SHOW.width()).min(room.height() / SHOW.height()).max(0.2);
        Fit { k, origin: room.center() - SHOW.center().to_vec2() * k }
    }

    fn at(&self, (x, y): (f32, f32)) -> Pos2 {
        self.origin + vec2(x, y) * self.k
    }
}

/// What noting a round found: when new cards come out (for their sound), when the hole card
/// turns over, and when the results show (if they're to be shown coming).
#[derive(Default)]
struct Noted {
    dealt: Vec<Instant>,
    hole: Option<Instant>,
    results: Option<Instant>,
}

impl BjTableUi {
    /// Notes the round on the table: each card not seen before gets a time to come out (a fresh
    /// deal in dealing order; a card drawn as it comes); a round over gets its reveal (the hole
    /// card, then the dealer's draws, then the results).
    fn note(&mut self, v: &BjView, rv: &RoundView, now: Instant) -> Noted {
        let mut out = Noted::default();
        let deal = v.round_no;
        if self.deal != Some(deal) {
            self.deal = Some(deal);
            self.appear.clear();
            self.reveal = None;
            self.hole_seen = false;
        }
        // Seen being dealt (from its betting, or while it's played), the cards come out in
        // order; opened in the middle of a round, or after one, they're all there at once.
        let fresh = self.appear.is_empty();
        let animate = !fresh || v.stage == BjStage::Playing || self.betting_seen == Some(deal);
        if fresh && animate {
            // (Dealt face down here: it turns over at the end, even if the round's over at once.)
            self.hole_seen = true;
        }
        let past = now.checked_sub(Duration::from_secs(3)).unwrap_or(now);
        let mut t = now;
        // In dealing order for a fresh deal: a card to each seat from first base, the dealer's
        // up card, a second card to each, the hole card. Then whatever's new, as it came.
        let mut order: Vec<((u8, usize, usize), Option<Card>)> = Vec::new();
        let mut seats: Vec<&SeatPlay> = rv.seats.iter().collect();
        seats.sort_by_key(|s| s.seat);
        if self.appear.is_empty() {
            for k in 0..2 {
                for sp in &seats {
                    if let Some(c) = sp.hands.first().and_then(|h| h.cards.get(k)) {
                        order.push(((sp.seat, 0, k), Some(*c)));
                    }
                }
                order.push(((DEALER, 0, k), rv.dealer.get(k).copied()));
            }
        }
        for sp in &seats {
            for (h, hand) in sp.hands.iter().enumerate() {
                for (i, c) in hand.cards.iter().enumerate() {
                    order.push(((sp.seat, h, i), Some(*c)));
                }
            }
        }
        order.push(((DEALER, 0, 0), rv.dealer.first().copied()));
        if rv.hole_hidden {
            self.hole_seen = true;
            order.push(((DEALER, 0, 1), None));
        }
        for (key, card) in order {
            let card = card.unwrap_or(Card::from_index(0));
            let changed = match self.appear.get(&key) {
                None => true,
                // (A card moved by a split: it comes out again. Not the hole card: it turns over.)
                Some((_, was)) => *was != card && key.0 != DEALER,
            };
            if changed {
                let at = if animate { t } else { past };
                self.appear.insert(key, (at, card));
                if animate {
                    out.dealt.push(at);
                    t += Duration::from_secs_f32(DEAL_GAP);
                }
            }
        }
        // The round's over: the hole card turns over once the last card's landed, then the
        // dealer draws, then the results.
        if !rv.hole_hidden && rv.phase == Phase::Over && self.reveal.is_none() {
            let shown = self.hole_seen;
            let last = self.appear.values().map(|(at, _)| *at).max().unwrap_or(now).max(now);
            let hole_at = if shown { last + Duration::from_secs_f32(SLIDE + 0.25) } else { past };
            let mut d = hole_at + Duration::from_secs_f32(0.55);
            for i in 1..rv.dealer.len() {
                if i == 1 {
                    // (The hole card stays where it lies, and turns over at `hole_at`.)
                    let e = self.appear.entry((DEALER, 0, 1)).or_insert((past, rv.dealer[1]));
                    e.1 = rv.dealer[1];
                    continue;
                }
                let at = if shown { d } else { past };
                self.appear.insert((DEALER, 0, i), (at, rv.dealer[i]));
                if shown {
                    out.dealt.push(at);
                    d += Duration::from_secs_f32(DRAW_GAP);
                }
            }
            let results_at = if shown { d.max(hole_at + Duration::from_secs_f32(0.6)) } else { now };
            self.reveal = Some((hole_at, results_at));
            if shown {
                out.hole = Some(hole_at);
                out.results = Some(results_at);
            }
        }
        out
    }
}

/// A card sliding out of the shoe to `at`, turning over as it lands (or staying face down);
/// `turn`: a face-down card turning over then.
#[allow(clippy::too_many_arguments)]
fn card(p: &egui::Painter, shoe: Pos2, at: Pos2, size: Vec2, face: Option<Card>, turn: Option<Instant>, appear: Instant, now: Instant) {
    if now < appear {
        return;
    }
    let k = ((now - appear).as_secs_f32() / SLIDE).min(1.0);
    let c = shoe + (at - shoe) * ease_out(k);
    // Turning over in the last part of its slide; or, the hole card, where it lies.
    let flip = match turn {
        Some(t) => (now.saturating_duration_since(t).as_secs_f32() / 0.3).clamp(0.0, 1.0),
        None => ((k - 0.55) / 0.45).clamp(0.0, 1.0),
    };
    match face {
        Some(f) if flip > 0.5 => {
            let w = size.x * (flip - 0.5) * 2.0;
            cards::face(p, Rect::from_center_size(c, vec2(w.max(2.0), size.y)), f, false, false);
        }
        _ => {
            let w = if face.is_none() { size.x } else { size.x * (1.0 - flip * 2.0).abs().max(0.04) };
            cards::back(p, Rect::from_center_size(c, vec2(w, size.y)), "");
        }
    }
}

/// A hand's total in a pill.
fn total_pill(p: &egui::Painter, at: Pos2, k: f32, cards: &[Card]) {
    let (v, soft) = bj::value(cards);
    let blackjack = bj::is_blackjack(cards);
    let text = if blackjack {
        "BLACKJACK".to_string()
    } else if v > 21 {
        format!("{v} BUST")
    } else if soft && v < 21 {
        format!("{} / {v}", v - 10)
    } else {
        v.to_string()
    };
    let f = (13.0 * k).clamp(10.5, 16.0);
    let w = f * 1.4 + text.len() as f32 * f * 0.62;
    let r = Rect::from_center_size(at, vec2(w, f * 1.75));
    let color = if blackjack { col::GOLD } else if v > 21 { col::RED } else { Color32::from_rgba_unmultiplied(0x16, 0x16, 0x1c, 235) };
    p.rect_filled(r, theme::r(((f * 0.9) as u8).max(6)), color);
    p.text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(f), if blackjack { Color32::BLACK } else { Color32::WHITE });
}

/// A hand's result in a pill.
fn result_pill(p: &egui::Painter, at: Pos2, k: f32, hand: &bj::Hand) {
    let Some(res) = hand.result else { return };
    let (text, color) = match res {
        bj::Result::Blackjack => (format!("BLACKJACK +{}", chips_text(hand.paid.saturating_sub(hand.bet))), col::GOLD2),
        bj::Result::Win => (format!("WIN +{}", chips_text(hand.paid.saturating_sub(hand.bet))), col::WIN),
        bj::Result::Push => ("PUSH".to_string(), Color32::from_rgb(0xd0, 0xd0, 0xd8)),
        bj::Result::Lose => ("LOSE".to_string(), col::LOSE),
        bj::Result::Bust => ("BUST".to_string(), col::LOSE),
    };
    let f = (13.5 * k).clamp(11.0, 17.0);
    let w = f * 1.6 + text.len() as f32 * f * 0.64;
    let r = Rect::from_center_size(at, vec2(w, f * 1.9));
    p.rect_filled(r, theme::r(((f * 0.95) as u8).max(6)), Color32::from_black_alpha(215));
    p.rect_stroke(r, theme::r(((f * 0.95) as u8).max(6)), Stroke::new(1.5f32, color), StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(f), color);
}

/// A button on the controls; true when it's clicked.
fn button(ui: &mut egui::Ui, r: Rect, id: &str, text: &str, sub: Option<String>, fill: Color32, enabled: bool) -> bool {
    let resp = ui.interact(r, ui.id().with(("bjt", id)), Sense::click());
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

impl PyrApp {
    /// Leaves the blackjack table I'm at (standing up first: a bet placed comes back; hands in
    /// play stand, and are paid as they come out).
    pub fn bj_table_leave(&mut self) {
        if let Some((key, id)) = self.poker.casino.bjt.at.take() {
            self.engine.poker(&key, ToServer::BjLeave { table: id });
            self.engine.poker(&key, ToServer::BjUnwatch { table: id });
            self.engine.bj_close(&key);
        }
        let t = &mut self.poker.casino.bjt;
        t.deal = None;
        t.appear.clear();
        t.reveal = None;
        t.queued = false;
        t.playing_chips = None;
    }

    /// The blackjack tables' page: the tables along the top, the table, the controls.
    pub(super) fn bj_table_page(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let now = Instant::now();
        let key = s.key.clone();
        // The tables (asked for now and then), and the one I'm at: the first, to begin with.
        if self.poker.casino.bjt.asked.is_none_or(|t| t.elapsed() > Duration::from_secs(8)) {
            self.poker.casino.bjt.asked = Some(now);
            self.engine.poker(&key, ToServer::BjTables);
        }
        let tables = s.poker.bj_tables.clone().unwrap_or_default();
        // (A table I opened: go to it once it's here.)
        if let Some(above) = self.poker.casino.bjt.follow_new
            && let Some(b) = s.poker.bj.as_ref().filter(|b| b.view.id > above)
        {
            self.poker.casino.bjt.follow_new = None;
            self.poker.casino.bjt.at = Some((key.clone(), b.view.id));
            self.poker.casino.bjt.asked = None;
        }
        let mine = self.poker.casino.bjt.at.clone().filter(|(k, _)| *k == key).map(|(_, id)| id);
        let at = match mine {
            Some(id) if tables.is_empty() || tables.iter().any(|t| t.id == id) => Some(id),
            _ => tables.first().map(|t| t.id),
        };
        let live = s.poker.bj.clone().filter(|b| Some(b.view.id) == at);
        if let Some(id) = at {
            let moved = self.poker.casino.bjt.at != Some((key.clone(), id));
            if moved || (live.is_none() && self.poker.casino.bjt.watch_asked.is_none_or(|t| t.elapsed() > Duration::from_secs(3))) {
                self.poker.casino.bjt.at = Some((key.clone(), id));
                self.poker.casino.bjt.watch_asked = Some(now);
                self.engine.poker(&key, ToServer::BjWatch { table: id });
            }
        }
        let p = ui.painter().clone();
        p.rect_filled(area, 0, Color32::from_rgb(2, 2, 4));
        let strip = Rect::from_min_size(area.min, vec2(area.width(), STRIP_H));
        let bar = Rect::from_min_max(pos2(area.left() + 16.0, area.bottom() - BAR_H + 6.0), pos2(area.right() - 16.0, area.bottom() - 10.0));
        let room = Rect::from_min_max(pos2(area.left(), strip.bottom()), pos2(area.right(), bar.top() - 4.0));
        let fit = Fit::new(room);
        // The table, its dark edges running off into the room's.
        let art = Rect::from_min_size(fit.origin, ART * fit.k);
        let clip = p.with_clip_rect(room.union(strip));
        super::pics::part(&clip, ui.ctx(), "table-blackjack", Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), art, art.width() as u32, Color32::WHITE);
        let dark = Color32::from_rgb(2, 2, 4);
        let clear = Color32::from_rgba_unmultiplied(2, 2, 4, 0);
        let fade = 90.0 * fit.k;
        hgrad(&clip, Rect::from_min_max(art.left_top(), pos2(art.left() + fade, art.bottom())), dark, clear);
        hgrad(&clip, Rect::from_min_max(pos2(art.right() - fade, art.top()), art.right_bottom()), clear, dark);
        vgrad(&clip, Rect::from_min_max(pos2(art.left(), art.bottom() - fade), art.right_bottom()), clear, dark);
        // The table's rules, on the felt.
        let k = fit.k;
        p.text(fit.at((688.0, 398.0)), Align2::CENTER_CENTER, "BLACKJACK PAYS 3 TO 2", theme::bold_font((24.0 * k).max(11.0)), col::GOLD.gamma_multiply(0.5));
        p.text(fit.at((688.0, 428.0)), Align2::CENTER_CENTER, "Dealer stands on soft 17  ·  Insurance pays 2 to 1", theme::bold_font((12.5 * k).max(9.0)), Color32::from_white_alpha(70));
        // The shoe.
        let shoe = fit.at(SHOE_AT);
        let shoe_r = Rect::from_center_size(shoe, vec2(84.0, 58.0) * k);
        p.rect_filled(shoe_r.translate(vec2(0.0, 5.0 * k)), theme::r(8), Color32::from_black_alpha(120));
        vgrad(&p, shoe_r, Color32::from_rgb(0x4a, 0x2c, 0x18), Color32::from_rgb(0x22, 0x12, 0x08));
        p.rect_stroke(shoe_r, theme::r(8), Stroke::new(1.0f32, col::GOLD_DARK), StrokeKind::Inside);
        cards::back(&p, Rect::from_center_size(shoe + vec2(-9.0, -3.0) * k, vec2(40.0, 56.0) * k), "");

        self.bj_tables_strip(ui, s, strip, &tables, at);
        let Some(live) = live else {
            p.text(fit.at((688.0, 470.0)), Align2::CENTER_CENTER, if s.poker.bj_tables.is_none() { "Setting out the tables…" } else { "Sitting you at the table…" }, theme::bold_font(16.0), col::MUTED);
            return;
        };
        let v = &live.view;
        // The round being played, or just over (between rounds the table's swept).
        let rv = if matches!(v.stage, BjStage::Playing | BjStage::Settled) { v.round.clone() } else { None };
        if matches!(v.stage, BjStage::Waiting | BjStage::Betting) {
            self.poker.casino.bjt.betting_seen = Some(v.round_no);
        }
        // New cards, the hole card, the results: their times and sounds.
        if let Some(rv) = &rv {
            let n = self.poker.casino.bjt.note(v, rv, now);
            for at in n.dealt {
                self.poker.casino.sound_in(at.saturating_duration_since(now).as_secs_f32(), Cue::Deal, 0.0, 0.6);
            }
            if let Some(h) = n.hole {
                self.poker.casino.sound_in(h.saturating_duration_since(now).as_secs_f32(), Cue::Flip, 0.0, 0.7);
            }
            if let Some(res) = n.results {
                self.bj_results_for_me(v, rv, now, res);
            }
        }
        // My chips while my bet's in or I'm in a round (the balance waits on them till the
        // results show).
        let in_round = v.my_seat.is_some_and(|m| rv.as_ref().is_some_and(|r| r.seats.iter().any(|sp| sp.seat == m)));
        let bet_in = v.my_seat.and_then(|m| v.seats.get(m as usize).cloned().flatten()).is_some_and(|x| x.decided && x.bet > 0);
        if (v.stage == BjStage::Playing && in_round) || (v.stage == BjStage::Betting && bet_in) {
            self.poker.casino.bjt.playing_chips = Some(chips_now(s));
        }
        let reveal = self.poker.casino.bjt.reveal;
        let results_shown = reveal.is_some_and(|(_, r)| now >= r);
        let cw = 58.0 * k;
        let size = vec2(cw, cw * 1.4);
        // The dealer's cards, and their total as they turn up.
        if let Some(rv) = &rv {
            let n = rv.dealer.len().max(if rv.hole_hidden { 2 } else { 0 });
            let c = fit.at(DEALER_AT);
            let mut seen = Vec::new();
            for i in 0..n {
                let pos = c + vec2((i as f32 - (n as f32 - 1.0) / 2.0) * cw * 0.72, 0.0);
                let Some((appear, _)) = self.poker.casino.bjt.appear.get(&(DEALER, 0, i)).copied() else { continue };
                let face = rv.dealer.get(i).copied().filter(|_| !(i == 1 && rv.hole_hidden));
                let turn = if i == 1 { reveal.map(|(h, _)| h).or(Some(now + Duration::from_secs(3600))) } else { None };
                card(&p, shoe, pos, size, face, turn, appear, now);
                let up = match turn {
                    Some(t) => now >= t + Duration::from_millis(200),
                    None => now >= appear + Duration::from_secs_f32(SLIDE),
                };
                if let (Some(f), true) = (face, up) {
                    seen.push(f);
                }
            }
            if !seen.is_empty() {
                total_pill(&p, c + vec2(0.0, size.y / 2.0 + 16.0 * k), k, &seen);
            }
        }
        // The seats: each person or bot, their bet, their cards.
        let to_act = rv.as_ref().filter(|r| v.stage == BjStage::Playing && r.phase != Phase::Over).and_then(|r| r.to_act);
        let clock = live.view.ms_left.saturating_sub(live.at.elapsed().as_millis() as u64);
        for i in 0..SEATS {
            let seat = v.seats.get(i).cloned().flatten();
            let c = fit.at(CIRCLES[i]);
            let r = CIRCLE_R * k;
            let sp = rv.as_ref().and_then(|rv| rv.seats.iter().find(|sp| sp.seat as usize == i)).cloned();
            self.bj_seat(ui, s, v, i as u8, seat.as_ref(), sp.as_ref(), c, r, k, shoe, size, to_act == Some(i as u8), clock, results_shown);
        }
        if let Some(rv) = &rv
            && results_shown
            && let Some((_, at)) = reveal
            && now < at + Duration::from_secs(1)
        {
            // (The insurance bets settled, said once over the dealer's cards.)
            let insured: u64 = rv.seats.iter().map(|sp| sp.insurance).sum();
            if insured > 0 {
                let text = if rv.dealer_blackjack { "Insurance pays 2 to 1" } else { "Insurance lost" };
                p.text(fit.at(DEALER_AT) + vec2(0.0, size.y / 2.0 + 42.0 * k), Align2::CENTER_CENTER, text, theme::bold_font(13.0), col::GOLD);
            }
        }
        self.bj_controls(ui, s, &live, bar, now);
        // Emotes, big over the circle of whoever sent them.
        let size = (112.0 * k).clamp(84.0, 132.0);
        super::emotes::pops(&p, ui.ctx(), &s.poker.game_emotes, TableGame::Blackjack, v.id, size, |seat| fit.at(CIRCLES[(seat as usize).min(SEATS - 1)]) - vec2(0.0, CIRCLE_R * k * 0.5));
    }

    /// The tables, along the top: each one's name, its bets and who's at it; click one to go to
    /// it (standing up from this one), or open another.
    fn bj_tables_strip(&mut self, ui: &mut egui::Ui, s: &ServerView, strip: Rect, tables: &[pyr_poker::net::BjSummary], at: Option<u64>) {
        let p = ui.painter().clone();
        vgrad(&p, strip, Color32::from_rgb(0x12, 0x10, 0x18), Color32::from_rgb(0x08, 0x07, 0x0c));
        let mut x = strip.left() + 14.0;
        for t in tables {
            let on = Some(t.id) == at;
            let text = format!("{}  ·  {}–{}", t.name, chips_text(t.min_bet), chips_text(t.max_bet));
            let who = format!("{} / 7", t.people + t.bots);
            let w = 34.0 + (text.chars().count() + who.len()) as f32 * 7.2;
            let r = Rect::from_min_size(pos2(x, strip.top() + 7.0), vec2(w, strip.height() - 14.0));
            x += w + 8.0;
            if r.right() > strip.right() - 150.0 {
                break;
            }
            let resp = ui.interact(r, ui.id().with(("bj-table", t.id)), Sense::click());
            p.rect_filled(r, theme::r(15), if on { col::GOLD.gamma_multiply(0.22) } else if resp.hovered() { Color32::from_white_alpha(22) } else { Color32::from_white_alpha(10) });
            p.rect_stroke(r, theme::r(15), Stroke::new(1.0f32, if on { col::GOLD } else { Color32::from_white_alpha(30) }), StrokeKind::Inside);
            p.text(r.left_center() + vec2(14.0, 0.0), Align2::LEFT_CENTER, text, theme::bold_font(13.0), if on { col::GOLD2 } else { col::TEXT });
            p.text(r.right_center() - vec2(12.0, 0.0), Align2::RIGHT_CENTER, who, theme::font(12.0), col::MUTED);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                self.bj_table_leave();
                self.poker.casino.bjt.at = Some((s.key.clone(), t.id));
                self.engine.poker(&s.key, ToServer::BjWatch { table: t.id });
                self.poker.casino.bjt.watch_asked = Some(Instant::now());
            }
        }
        // Another table (when they're all busy).
        let r = Rect::from_min_size(pos2(strip.right() - 144.0, strip.top() + 7.0), vec2(130.0, strip.height() - 14.0));
        let resp = ui.interact(r, ui.id().with("bj-new"), Sense::click());
        p.rect_filled(r, theme::r(15), if resp.hovered() { Color32::from_white_alpha(26) } else { Color32::from_white_alpha(12) });
        p.text(r.center(), Align2::CENTER_CENTER, "+ Another table", theme::bold_font(13.0), col::TEXT);
        if resp.on_hover_text("Open a table of your own (10 to 1,000 a hand)").clicked() {
            self.bj_table_leave();
            self.poker.casino.bjt.follow_new = Some(tables.iter().map(|t| t.id).max().unwrap_or(0));
            self.engine.poker(&s.key, ToServer::BjNew);
        }
    }

    /// The results of a round over for me: called out big, and the balance counting up as they
    /// show.
    fn bj_results_for_me(&mut self, v: &BjView, rv: &RoundView, now: Instant, at: Instant) {
        let Some(sp) = v.my_seat.and_then(|m| rv.seats.iter().find(|sp| sp.seat == m)) else { return };
        let back = at.saturating_duration_since(now).as_secs_f32();
        let wagered = sp.wagered();
        let insurance = if rv.dealer_blackjack { sp.insurance * 3 } else { 0 };
        let paid = sp.hands.iter().map(|h| h.paid).sum::<u64>() + insurance;
        if let Some(chips) = self.poker.casino.bjt.playing_chips.take() {
            self.poker.casino.hold = Some((chips, at));
        }
        let (title, color) = if sp.hands.iter().any(|h| h.result == Some(bj::Result::Blackjack)) {
            ("BLACKJACK!", col::GOLD2)
        } else if paid > wagered {
            ("YOU WIN", col::WIN)
        } else if paid == wagered && paid > 0 {
            ("PUSH", Color32::from_rgb(0xd8, 0xd8, 0xe0))
        } else if sp.hands.iter().all(|h| h.result == Some(bj::Result::Bust)) {
            ("BUST", col::LOSE)
        } else {
            ("DEALER WINS", col::LOSE)
        };
        let sub = if paid > wagered { format!("+{}", chips_text(paid - wagered)) } else { String::new() };
        self.poker.casino.announce(title, sub, color, (0.5, 0.3), back);
        if paid > wagered {
            self.poker.casino.sound_in(back, Cue::WinSmall, 0.0, 0.8);
            self.poker.casino.sound_in(back + 0.2, Cue::Chips, 0.0, 0.7);
        }
        self.poker.casino.celebrate(paid, wagered, false, back);
    }

    /// One seat: its betting circle (and the bet in it), who's in it, their hands.
    #[allow(clippy::too_many_arguments)]
    fn bj_seat(&mut self, ui: &mut egui::Ui, s: &ServerView, v: &BjView, i: u8, seat: Option<&BjSeatView>, sp: Option<&SeatPlay>, c: Pos2, r: f32, k: f32, shoe: Pos2, size: Vec2, turn: bool, clock_ms: u64, results: bool) {
        let p = ui.painter().clone();
        let now = Instant::now();
        let me = v.my_seat == Some(i);
        let hit = Rect::from_center_size(c, vec2(r * 2.4, r * 2.4));
        let resp = ui.interact(hit, ui.id().with(("bj-seat", i)), Sense::click());
        let betting = matches!(v.stage, BjStage::Waiting | BjStage::Betting | BjStage::Settled);
        // Whose turn: a glow round the circle, and their time running out.
        if turn {
            for g in 0..5 {
                p.circle_filled(c, r + 4.0 + g as f32 * 3.0 * k, col::GOLD.gamma_multiply(0.06));
            }
            let total = if seat.is_some_and(|x| x.bot) { 2_000.0 } else { pyr_poker::bjroom::TURN_MS as f32 };
            let frac = (clock_ms as f32 / total).clamp(0.0, 1.0);
            let ring = if frac > 0.5 { col::WIN } else if frac > 0.2 { col::GOLD } else { col::RED };
            p.circle_stroke(c, r + 3.0, Stroke::new(4.0f32, Color32::from_black_alpha(150)));
            arc(&p, c, r + 3.0, frac, Stroke::new(4.0f32, ring));
        } else if me {
            p.circle_stroke(c, r + 2.0, Stroke::new(2.0f32, col::GOLD.gamma_multiply(0.8)));
        }
        // The chips in the circle: the round's bets, or the next round's.
        let chip_r = (r * 0.5).max(8.0);
        match (sp, seat) {
            (Some(sp), _) if v.stage == BjStage::Playing || (v.stage == BjStage::Settled && !seat.is_some_and(|x| x.decided)) => {
                chip_stack(&p, c + vec2(0.0, r * 0.25), chip_r, sp.base);
            }
            (_, Some(x)) if x.decided && x.bet > 0 => chip_stack(&p, c + vec2(0.0, r * 0.25), chip_r, x.bet),
            (_, Some(x)) if me && !x.decided && self.poker.casino.bjt.bet > 0 && betting => {
                // (Mine, still being put together: see-through till it's placed.)
                let mut ghost = p.clone();
                ghost.set_opacity(0.6);
                chip_stack(&ghost, c + vec2(0.0, r * 0.25), chip_r, self.poker.casino.bjt.bet);
            }
            (_, Some(x)) if x.decided && x.bet == 0 && betting => {
                p.text(c, Align2::CENTER_CENTER, "OUT", theme::bold_font((11.0 * k).max(9.0)), Color32::from_white_alpha(110));
            }
            _ => {}
        }
        // Click my circle to bet the chip picked (right-click to take it back).
        if me && betting && seat.is_some_and(|x| !x.decided) {
            if resp.clicked() {
                let chip = self.poker.casino.chip;
                let t = &mut self.poker.casino.bjt;
                t.bet = (t.bet + chip).min(v.max_bet);
                self.poker.casino.sound_in(0.0, Cue::Chips, 0.0, 0.5);
            }
            if resp.secondary_clicked() {
                self.poker.casino.bjt.bet = 0;
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
        }
        // Who's sitting: their picture and name under the circle (or an open seat).
        let plate_c = c + vec2(0.0, r + 15.0 * k.max(0.8));
        let sitting = v.my_seat.is_some();
        match seat {
            Some(x) => {
                let name = if x.name.chars().count() > 12 { format!("{}…", x.name.chars().take(11).collect::<String>()) } else { x.name.clone() };
                let f = (12.5 * k).clamp(10.5, 14.0);
                let w = f * 2.6 + name.chars().count() as f32 * f * 0.6 + if x.bot { f * 2.6 } else { 0.0 };
                let plate = Rect::from_center_size(plate_c, vec2(w, f * 2.0));
                p.rect_filled(plate, theme::r(((f) as u8).max(6)), Color32::from_black_alpha(if me { 230 } else { 200 }));
                p.rect_stroke(plate, theme::r((f as u8).max(6)), Stroke::new(1.0f32, if me { col::GOLD } else if turn { col::GOLD.gamma_multiply(0.7) } else { Color32::from_white_alpha(36) }), StrokeKind::Inside);
                let face_c = plate.left_center() + vec2(f * 0.95, 0.0);
                let fr = f * 0.72;
                if x.bot {
                    crate::poker::table::bot_face(&p, face_c, fr, &x.name, pyr_poker::bots::Level::Regular);
                } else {
                    let face = self.face(ui.ctx(), s, &x.player);
                    crate::app::avatar(&p, face_c, fr, &x.player, &x.name, false, Color32::BLACK, face);
                }
                p.text(face_c + vec2(fr + 5.0, 0.0), Align2::LEFT_CENTER, &name, theme::bold_font(f), if me { col::GOLD2 } else { col::TEXT });
                if x.bot {
                    let tag = Rect::from_min_size(pos2(plate.right() - f * 2.6, plate.center().y - f * 0.62), vec2(f * 2.2, f * 1.24));
                    p.rect_filled(tag, theme::r(5), Color32::from_rgb(0x3d, 0x5f, 0xc4));
                    p.text(tag.center(), Align2::CENTER_CENTER, "BOT", theme::bold_font(f * 0.72), Color32::WHITE);
                    // Who it is (hover), and it can be asked to leave between rounds (right-click).
                    let who = pyr_poker::bots::persona::of(&x.name);
                    let bot_resp = ui.interact(plate, ui.id().with(("bj-bot", i)), Sense::click()).on_hover_text(format!("{}: a bot that {}. It plays with the house's chips.", x.name, who.bj_style()));
                    if v.stage != BjStage::Playing {
                        bot_resp.context_menu(|ui| {
                            crate::menu::style(ui);
                            if ui.button(format!("Ask {} to leave", x.name)).clicked() {
                                self.engine.poker(&s.key, ToServer::BjBot { table: v.id, seat: i, add: false });
                                ui.close();
                            }
                        });
                    }
                }
            }
            None => {
                // An open seat: sit in it (if I'm not sitting), or put a bot in it.
                let hover = resp.hovered();
                if hover {
                    p.circle_filled(c, r * 0.96, Color32::from_white_alpha(24));
                }
                if !sitting {
                    p.text(c, Align2::CENTER_CENTER, "SIT", theme::bold_font((12.0 * k).max(9.5)), col::GOLD.gamma_multiply(if hover { 1.0 } else { 0.55 }));
                    if resp.on_hover_text("Sit here").clicked() {
                        self.poker.casino.bjt.bet = 0;
                        self.poker.casino.bjt.bet_round = None;
                        self.engine.poker(&s.key, ToServer::BjSit { table: v.id, seat: i });
                    }
                }
                let f = (11.0 * k).clamp(9.5, 12.5);
                let br = Rect::from_center_size(plate_c, vec2(f * 5.4, f * 1.9));
                let bresp = ui.interact(br, ui.id().with(("bj-addbot", i)), Sense::click());
                p.rect_filled(br, theme::r((f as u8).max(6)), if bresp.hovered() { Color32::from_rgb(0x3d, 0x5f, 0xc4) } else { Color32::from_black_alpha(150) });
                p.text(br.center(), Align2::CENTER_CENTER, "+ BOT", theme::bold_font(f * 0.9), if bresp.hovered() { Color32::WHITE } else { Color32::from_white_alpha(140) });
                if bresp.on_hover_text("Seat a bot here: each plays its own way, with the house's chips").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.engine.poker(&s.key, ToServer::BjBot { table: v.id, seat: i, add: true });
                }
            }
        }
        // Their hands, above the circle (side by side when split), and each one's total; the
        // results once the dealer's done.
        let Some(sp) = sp else { return };
        let n = sp.hands.len().max(1) as f32;
        let scale = if n > 2.0 { 0.72 } else if n > 1.0 { 0.86 } else { 1.0 };
        let hs = size * scale;
        // Each card steps far enough right that the one under it shows its rank and suit (a
        // "10" too), closing up a little past four cards.
        let fan = |m: usize| if m > 1 { (1.6 / (m - 1) as f32).min(0.5) } else { 0.5 };
        let widest = sp.hands.iter().map(|h| 1.0 + fan(h.cards.len()) * h.cards.len().saturating_sub(1) as f32).fold(1.0f32, f32::max);
        let spread = hs.x * (widest + 0.22);
        let playing_hand = turn && v.stage == BjStage::Playing;
        for (h, hand) in sp.hands.iter().enumerate() {
            let base = c + vec2((h as f32 - (n - 1.0) / 2.0) * spread, -(r + hs.y * 0.55 + 4.0 * k));
            let step = vec2(hs.x * fan(hand.cards.len()), -hs.y * 0.07);
            let m = hand.cards.len() as f32;
            let first = base - step * ((m - 1.0) / 2.0).max(0.0);
            if playing_hand && h == sp.active {
                p.add(egui::epaint::EllipseShape::filled(base + vec2(0.0, hs.y * 0.55), vec2(hs.x * 0.9, 6.0 * k), col::GOLD.gamma_multiply(0.45)));
            }
            let mut seen = Vec::new();
            for (ci, cd) in hand.cards.iter().enumerate() {
                let Some((appear, _)) = self.poker.casino.bjt.appear.get(&(sp.seat, h, ci)).copied() else { continue };
                card(&p, shoe, first + step * ci as f32, hs, Some(*cd), None, appear, now);
                if now >= appear + Duration::from_secs_f32(SLIDE) {
                    seen.push(*cd);
                }
            }
            let top = first + step * (m - 1.0).max(0.0) - vec2(0.0, hs.y / 2.0);
            if results && hand.result.is_some() {
                result_pill(&p, top - vec2(0.0, 14.0 * k), k, hand);
            } else if !seen.is_empty() {
                total_pill(&p, top - vec2(0.0, 12.0 * k), k, &seen);
            }
            // A double, or a split hand's own bet: its chips beside the cards.
            if hand.doubled || sp.hands.len() > 1 {
                chip_stack(&p, base + vec2(hs.x * 0.5 + chip_r * 0.9, hs.y * 0.3), chip_r * 0.75, hand.bet);
            }
        }
        if sp.insurance > 0 {
            let f = (10.0 * k).max(9.0);
            p.text(c + vec2(r + 6.0, -r * 0.6), Align2::LEFT_CENTER, format!("INS {}", chips_text(sp.insurance)), theme::bold_font(f), col::GOLD);
        }
    }

    /// The controls along the bottom: betting (the rack, my bet, Bet, Rebet), my moves when
    /// it's my turn, or what's happening.
    fn bj_controls(&mut self, ui: &mut egui::Ui, s: &ServerView, live: &pyr_engine::poker::BjLive, bar: Rect, now: Instant) {
        let p = ui.painter().clone();
        let v = &live.view;
        p.rect_filled(bar, theme::r(18), Color32::from_black_alpha(185));
        p.rect_stroke(bar, theme::r(18), Stroke::new(1.0f32, Color32::from_white_alpha(22)), StrokeKind::Inside);
        let left = v.ms_left.saturating_sub(live.at.elapsed().as_millis() as u64);
        let secs = left.div_ceil(1000);
        let typing = ui.ctx().wants_keyboard_input();
        let keys = ui.ctx().input(|i| (i.key_pressed(egui::Key::H), i.key_pressed(egui::Key::S), i.key_pressed(egui::Key::D), i.key_pressed(egui::Key::P), i.key_pressed(egui::Key::Enter)));
        let say = |text: String, color: Color32| {
            p.text(pos2(bar.left() + 22.0, bar.center().y), Align2::LEFT_CENTER, text, theme::bold_font(15.0), color);
        };
        let Some(me) = v.my_seat else {
            // Watching: sit in an open seat.
            let free = v.seats.iter().position(|x| x.is_none());
            say(
                match free {
                    Some(_) => format!("{}: bets {} to {}. Click an open seat to sit down.", v.name, chips_text(v.min_bet), chips_text(v.max_bet)),
                    None => format!("{} is full: watch, or try another table.", v.name),
                },
                col::MUTED,
            );
            if let Some(seat) = free {
                let r = Rect::from_min_size(pos2(bar.right() - 176.0, bar.center().y - 24.0), vec2(160.0, 48.0));
                if button(ui, r, "sit", "Sit down", None, col::GOLD, true) {
                    self.poker.casino.bjt.bet = 0;
                    self.poker.casino.bjt.bet_round = None;
                    self.engine.poker(&s.key, ToServer::BjSit { table: v.id, seat: seat as u8 });
                }
            }
            return;
        };
        let mine = v.seats.get(me as usize).cloned().flatten();
        let decided = mine.as_ref().is_some_and(|x| x.decided);
        let rv = v.round.as_ref();
        let my_turn = v.stage == BjStage::Playing && rv.is_some_and(|r| r.to_act == Some(me) && r.phase != Phase::Over);
        // Stand up, at the far right always.
        let up = Rect::from_min_size(pos2(bar.right() - 126.0, bar.center().y - 22.0), vec2(112.0, 44.0));
        if button(ui, up, "stand", "Stand up", None, Color32::from_white_alpha(30), true) {
            self.engine.poker(&s.key, ToServer::BjLeave { table: v.id });
            self.poker.casino.bjt.bet = 0;
            self.poker.casino.bjt.queued = false;
            self.poker.casino.bjt.auto = false;
            return;
        }
        // Emotes, beside it.
        let em = Rect::from_min_size(pos2(up.left() - 12.0 - 56.0, bar.center().y - 24.0), vec2(56.0, 48.0));
        if let Some(e) = super::emotes::button(ui, em, "bj-emotes", &mut self.poker.casino.emote_last) {
            self.engine.poker(&s.key, ToServer::GameEmote { game: TableGame::Blackjack, table: v.id, emote: e });
        }
        let right = em.left() - 12.0;
        if my_turn && let Some(rv) = rv {
            // My moves (with basic strategy's hint, if wanted), and my time.
            let sp = rv.seats.iter().find(|sp| sp.seat == me);
            let hint = match (sp, self.poker.casino.bjt.hint && rv.phase == Phase::Playing) {
                (Some(sp), true) => sp.hands.get(sp.active).map(|h| bj::basic(&h.cards, rv.dealer[0], rv.allowed.contains(&Action::Double), rv.allowed.contains(&Action::Split))),
                _ => None,
            };
            let moves: Vec<(Action, &str, bool, Color32)> = if rv.phase == Phase::Insurance {
                vec![(Action::Insure, "Insurance", false, Color32::from_rgb(0x55, 0x48, 0x60)), (Action::NoInsurance, "No insurance", false, Color32::from_rgb(0x55, 0x48, 0x60))]
            } else {
                vec![
                    (Action::Hit, "Hit  (H)", keys.0, Color32::from_rgb(0x2a, 0x8a, 0x4a)),
                    (Action::Stand, "Stand  (S)", keys.1, Color32::from_rgb(0xb8, 0x2a, 0x30)),
                    (Action::Double, "Double  (D)", keys.2, Color32::from_rgb(0xc8, 0x8a, 0x1a)),
                    (Action::Split, "Split  (P)", keys.3, Color32::from_rgb(0x2a, 0x5a, 0xc0)),
                ]
            };
            let w = 138.0;
            let total = moves.len() as f32 * (w + 10.0) - 10.0;
            let mut x = (bar.center().x - total / 2.0).max(bar.left() + 210.0).min(right - total);
            for (a, label, key, fill) in moves {
                let r = Rect::from_min_size(pos2(x, bar.center().y - 26.0), vec2(w, 52.0));
                x += w + 10.0;
                let ok = rv.allowed.contains(&a);
                if hint == Some(a) {
                    p.rect_stroke(r.expand(3.0), theme::r(14), Stroke::new(2.5f32, col::GOLD2), StrokeKind::Outside);
                }
                let cost = sp.map(|sp| match a {
                    Action::Double | Action::Split => sp.hands.get(sp.active).map(|h| h.bet).unwrap_or(0),
                    Action::Insure => sp.base / 2,
                    _ => 0,
                });
                let sub = cost.filter(|c| *c > 0).map(|c| format!("+{}", chips_text(c)));
                if button(ui, r, label, label, sub, fill, ok) || (ok && key && !typing) {
                    self.engine.poker(&s.key, ToServer::BjAct { table: v.id, action: a });
                    if matches!(a, Action::Double | Action::Split | Action::Insure) {
                        self.poker.casino.sound_in(0.0, Cue::Chips, 0.0, 0.6);
                    }
                }
            }
            p.text(pos2(bar.left() + 22.0, bar.center().y - 10.0), Align2::LEFT_CENTER, "Your turn", theme::bold_font(16.0), col::GOLD2);
            p.text(pos2(bar.left() + 22.0, bar.center().y + 12.0), Align2::LEFT_CENTER, format!("{secs}s left"), theme::font(13.0), if secs <= 5 { col::LOSE } else { col::MUTED });
            let hr = Rect::from_min_size(pos2(bar.left() + 110.0, bar.center().y - 4.0), vec2(80.0, 26.0));
            let on = self.poker.casino.bjt.hint;
            if button(ui, hr, "hint", if on { "Hints on" } else { "Hints off" }, None, if on { Color32::from_rgb(0x4a, 0x3a, 0x10) } else { Color32::from_white_alpha(24) }, true) {
                self.poker.casino.bjt.hint = !on;
            }
            return;
        }
        if v.stage == BjStage::Playing {
            // Someone else's turn (or the dealer's).
            let who = rv.and_then(|r| r.to_act).and_then(|seat| v.seats.get(seat as usize).cloned().flatten()).map(|x| format!("{}'s turn…", x.name)).unwrap_or_else(|| "The dealer's turn…".into());
            let mine = rv.and_then(|r| r.seats.iter().find(|sp| sp.seat == me)).map(|sp| format!("  ·  your bet {}", chips_text(sp.wagered()))).unwrap_or_else(|| "  ·  you're sitting this one out".into());
            say(format!("{who}{mine}"), col::MUTED);
            return;
        }
        // Betting: the rack, my bet, and Bet / Rebet.
        let t = &mut self.poker.casino.bjt;
        let round = v.round_no;
        let open = matches!(v.stage, BjStage::Waiting | BjStage::Betting);
        if t.bet_round.is_some_and(|r| r != round) {
            t.bet_round = None;
        }
        let chips = chips_now(s);
        // A bet made while the last round was on the table, or the same again by itself.
        if open && !decided && t.bet_round.is_none() {
            let amount = if t.queued { t.bet } else if t.auto { t.last_bet } else { 0 };
            if amount >= v.min_bet && amount <= v.max_bet && amount <= chips {
                t.queued = false;
                t.bet_round = Some(round);
                t.last_bet = amount;
                t.bet = 0;
                self.engine.bj_bet(&s.key, v.id, amount);
                self.poker.casino.sound_in(0.0, Cue::Chips, 0.0, 0.6);
                return;
            }
        }
        if decided {
            let what = match mine.as_ref().map(|x| x.bet).unwrap_or(0) {
                0 => "You're sitting this round out.".to_string(),
                b => format!("Your bet's in: {}. ", chips_text(b)) + if v.stage == BjStage::Betting { "The deal's coming." } else { "" },
            };
            let clock = if v.stage == BjStage::Betting { format!("  Cards in {secs}s") } else { String::new() };
            say(format!("{what}{clock}"), col::GOLD2);
            let ar = Rect::from_min_size(pos2(right - 150.0, bar.center().y - 22.0), vec2(150.0, 44.0));
            let auto = self.poker.casino.bjt.auto;
            if button(ui, ar, "auto", if auto { "Auto rebet: on" } else { "Auto rebet: off" }, None, if auto { Color32::from_rgb(0x4a, 0x3a, 0x10) } else { Color32::from_white_alpha(24) }, true) {
                self.poker.casino.bjt.auto = !auto;
            }
            return;
        }
        let bet = self.poker.casino.bjt.bet;
        let last = self.poker.casino.bjt.last_bet;
        let queued = self.poker.casino.bjt.queued;
        let mut x = right;
        let mut place = |w: f32| {
            let r = Rect::from_min_size(pos2(x - w, bar.center().y - 24.0), vec2(w, 48.0));
            x -= w + 10.0;
            r
        };
        let ok = bet >= v.min_bet && bet <= v.max_bet && bet <= chips;
        let (label, sub) = match v.stage {
            BjStage::Settled if queued => ("Bet placed".to_string(), Some("for the next round".to_string())),
            BjStage::Settled => (format!("Bet {}", chips_text(bet)), Some(format!("next round in {secs}s"))),
            BjStage::Betting => (format!("Bet {}  (Enter)", chips_text(bet)), Some(format!("{secs}s to bet"))),
            _ => (format!("Bet {}  (Enter)", chips_text(bet)), Some("starts the round".to_string())),
        };
        let r = place(190.0);
        if (button(ui, r, "bet", &label, sub, col::GOLD, ok && !queued) || (ok && !queued && keys.4 && !typing)) && bet > 0 {
            let t = &mut self.poker.casino.bjt;
            if open {
                t.bet_round = Some(round);
                t.last_bet = bet;
                t.bet = 0;
                self.engine.bj_bet(&s.key, v.id, bet);
                self.poker.casino.sound_in(0.0, Cue::Chips, 0.0, 0.6);
            } else {
                t.queued = true;
            }
        }
        let r = place(118.0);
        let rebet = bet == 0 && last > 0;
        if button(ui, r, "rebet", if rebet { "Rebet" } else { "Clear" }, rebet.then(|| chips_text(last)), Color32::from_white_alpha(30), rebet || bet > 0) {
            let t = &mut self.poker.casino.bjt;
            t.queued = false;
            t.bet = if rebet { last } else { 0 };
        }
        let r = place(128.0);
        let auto = self.poker.casino.bjt.auto;
        if button(ui, r, "auto", "Auto rebet", Some(if auto { "on".into() } else { "off".into() }), if auto { Color32::from_rgb(0x4a, 0x3a, 0x10) } else { Color32::from_white_alpha(24) }, true) {
            self.poker.casino.bjt.auto = !auto;
            if !auto && self.poker.casino.bjt.last_bet == 0 {
                self.poker.casino.bjt.last_bet = bet;
            }
        }
        let color = if bet > chips { col::LOSE } else { col::GOLD2 };
        let max_chip = super::CHIPS.iter().copied().filter(|c| *c <= v.max_bet).last().unwrap_or(1);
        self.rack_and_bet(ui, bar, x, max_chip, format!("Bet {}", chips_text(bet)), color);
        let hint = if bet == 0 {
            format!("Pick a chip, click your circle to bet ({} to {})", chips_text(v.min_bet), chips_text(v.max_bet))
        } else if bet < v.min_bet {
            format!("The least bet here is {}", chips_text(v.min_bet))
        } else if v.stage == BjStage::Settled {
            "Betting opens as soon as this round's cleared".to_string()
        } else {
            String::new()
        };
        if !hint.is_empty() {
            p.text(pos2(bar.center().x, bar.top() - 12.0), Align2::CENTER_CENTER, hint, theme::font(12.5), col::MUTED);
        }
        // How long's left to bet: a line along the top of the controls.
        if v.stage == BjStage::Betting {
            let frac = (left as f32 / pyr_poker::bjroom::BET_MS as f32).clamp(0.0, 1.0);
            let line = Rect::from_min_size(bar.left_top() + vec2(18.0, 0.0), vec2((bar.width() - 36.0) * frac, 3.0));
            p.rect_filled(line, theme::r(2), if frac > 0.3 { col::GOLD } else { col::LOSE });
        }
        let _ = now;
    }
}

/// An arc round `c` from the top, clockwise, `frac` of the way round.
fn arc(p: &egui::Painter, c: Pos2, r: f32, frac: f32, stroke: Stroke) {
    if frac <= 0.0 {
        return;
    }
    let n = (64.0 * frac).ceil().max(2.0) as usize;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * frac * i as f32 / n as f32;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    p.add(egui::Shape::line(pts, stroke));
}
