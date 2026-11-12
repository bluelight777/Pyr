//! Craps at a table of five (0.12.0 servers): people and bots round one layout, the dice going
//! from shooter to shooter. The layout's the single-player table's (pass line and odds, don't
//! pass and its lay odds, the field, the place numbers, the hardways and the one-roll props) with
//! everyone's chips on it, each player's in their own place on each bet and ringed in their
//! seat's color; the seats along the rail; the dice thrown from the shooter's seat, the
//! stickman's call, and what the roll did for each. Every roll is rolled again by this Pyr from
//! its seeds (see `pyr_engine::poker::SharedLive`).
//!
//! As 0.13.28 has it, measured off its screen: a LAY strip over each number's box and a BUY
//! strip under it (0.13.23), LAY ODDS beside don't pass once there's a point (0.13.6), the OFF
//! puck at the field's far end (it covered the word FIELD), my chips piled as I put them down
//! with what they come to on a tag (0.13.24), what a buy or a lay pays worked out in its words
//! (0.13.26), and the rack offering only the chips I have left to put down.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::casino::craps::{self as cr, Bets, Decided, HARD_NUMBERS, PLACE_NUMBERS, Table};
use pyr_poker::crapsroom::{BET_MS, SEATS, SHOW_MS};
use pyr_poker::net::{CrapsReveal, CrapsSeatView, CrapsStage, CrapsView, TableGame, ToServer};

use super::{art, chip_pile, chip_stack, chips_for, chips_now, col, ease_out, glow_text, radial, vgrad};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

/// The dice in the air.
const ROLL_SECS: f32 = 1.15;
const BAR_H: f32 = 96.0;
use super::picker::STRIP_H;
/// The seats along the rail, and their colors (each player's chips are ringed in theirs).
const PLATES_H: f32 = 74.0;
const SEAT_COLORS: [Color32; SEATS] = [
    Color32::from_rgb(0xf5, 0xc4, 0x51),
    Color32::from_rgb(0x5a, 0xb8, 0xff),
    Color32::from_rgb(0xff, 0x6a, 0x9a),
    Color32::from_rgb(0x5a, 0xe0, 0xa0),
    Color32::from_rgb(0xb0, 0x8a, 0xff),
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Spot {
    Pass,
    Odds,
    DontPass,
    DontOdds,
    Field,
    Place(usize),
    Buy(usize),
    Lay(usize),
    Hard(usize),
    AnySeven,
    AnyCraps,
}

fn slot(b: &mut Bets, s: Spot) -> &mut u64 {
    match s {
        Spot::Pass => &mut b.pass,
        Spot::Odds => &mut b.pass_odds,
        Spot::DontPass => &mut b.dont_pass,
        Spot::DontOdds => &mut b.dont_odds,
        Spot::Field => &mut b.field,
        Spot::Place(i) => &mut b.place[i],
        Spot::Buy(i) => &mut b.buy[i],
        Spot::Lay(i) => &mut b.lay[i],
        Spot::Hard(i) => &mut b.hard[i],
        Spot::AnySeven => &mut b.any_seven,
        Spot::AnyCraps => &mut b.any_craps,
    }
}

fn amount(b: &Bets, s: Spot) -> u64 {
    match s {
        Spot::Pass => b.pass,
        Spot::Odds => b.pass_odds,
        Spot::DontPass => b.dont_pass,
        Spot::DontOdds => b.dont_odds,
        Spot::Field => b.field,
        Spot::Place(i) => b.place[i],
        Spot::Buy(i) => b.buy[i],
        Spot::Lay(i) => b.lay[i],
        Spot::Hard(i) => b.hard[i],
        Spot::AnySeven => b.any_seven,
        Spot::AnyCraps => b.any_craps,
    }
}

/// Whether a bet's spot is wide (chips along it) or a box (chips in a little cluster).
fn wide(s: Spot) -> bool {
    matches!(s, Spot::Pass | Spot::DontPass | Spot::Field | Spot::AnySeven | Spot::AnyCraps)
}

/// A chip's size on a spot: 12 along the wide ones, 10 in a box, 9 on a BUY or LAY strip.
fn chip_r(s: Spot) -> f32 {
    match s {
        Spot::Buy(_) | Spot::Lay(_) => 9.0,
        _ if wide(s) => 12.0,
        _ => 10.0,
    }
}

/// Where seat `i`'s chips go on a spot (seat 0's on the right, as its seat is).
fn chip_at(r: Rect, s: Spot, i: usize) -> Pos2 {
    let i = SEATS - 1 - i.min(SEATS - 1);
    match s {
        // (Along the strip, right of its word, 1 under its middle: 0.13.28's, its chips fitted
        // on its screen at three sizes.)
        Spot::Buy(_) | Spot::Lay(_) => pos2(r.left() + r.width() * (0.12 + 0.76 * (i as f32 + 0.5) / SEATS as f32), r.center().y + 1.0),
        _ if wide(s) => {
            let x0 = if matches!(s, Spot::Field) { 0.30 } else { 0.10 };
            let span = if matches!(s, Spot::Field) { 0.66 } else { 0.80 };
            pos2(r.left() + r.width() * (x0 + span * (i as f32 + 0.5) / SEATS as f32), r.top() + r.height() * 0.66)
        }
        _ => {
            let (x, y) = [(0.22, 0.58), (0.5, 0.58), (0.78, 0.58), (0.36, 0.86), (0.64, 0.86)][i % SEATS];
            pos2(r.left() + r.width() * x, r.top() + r.height() * y)
        }
    }
}

/// The server feature that says its craps tables take lay odds behind don't pass (0.13.6).
const LAY_ODDS_FEATURE: &str = pyr_proto::control::HIGH_TABLES_FEATURE;

/// What a place bet pays, a buy's true odds, and a lay's, by number (4, 5, 6, 8, 9, 10).
const PLACE_PAYS: [&str; 6] = ["9 to 5", "7 to 5", "7 to 6", "7 to 6", "7 to 5", "9 to 5"];
const BUY_ODDS: [&str; 6] = ["2 to 1", "3 to 2", "6 to 5", "6 to 5", "3 to 2", "2 to 1"];
const LAY_ODDS: [&str; 6] = ["1 to 2", "2 to 3", "5 to 6", "5 to 6", "2 to 3", "1 to 2"];

/// A buy's words (0.13.26's), worked out for `a` on it: what it pays less the vig, and what
/// placing the same would win instead.
fn buy_words(i: usize, a: u64) -> String {
    let n = PLACE_NUMBERS[i];
    let (win, vig, comes) = (cr::odds_win(n, a), cr::vig(a), cr::buy_pays(n, a));
    let placed = cr::place_win(n, a);
    let beside = match comes.cmp(&placed) {
        std::cmp::Ordering::Greater => format!("buying pays {} more", chips_text(comes - placed)),
        std::cmp::Ordering::Less => format!("placing pays {} more", chips_text(placed - comes)),
        std::cmp::Ordering::Equal => "the same".into(),
    };
    format!(
        "Buy {n}: the {n} before a 7 pays true odds ({}), less a vig of 5% of the bet (rounded down, 1 at least), taken only when it wins. {a} on it: the {n} pays {} less {} vig, so {} comes to you and the bet stays up; a 7 takes the bet. Off on the come-out unless your buys are working. Placed ({}), {a} on the {n} would win {}: {beside}.",
        BUY_ODDS[i],
        chips_text(win),
        chips_text(vig),
        chips_text(comes),
        PLACE_PAYS[i],
        chips_text(placed),
        a = chips_text(a),
    )
}

/// A lay's words, worked out for `a` on it.
fn lay_words(i: usize, a: u64) -> String {
    let n = PLACE_NUMBERS[i];
    let win = cr::lay_win(n, a);
    format!(
        "Lay {n}: a 7 before the {n} pays true odds ({}), less a 5% vig on the win (rounded down, 1 at least), taken only when it wins. {} on it: a 7 pays {} less {} vig, so {} comes to you and the bet stays up; the {n} takes the bet. Working on every roll, the come-out too.",
        LAY_ODDS[i],
        chips_text(a),
        chips_text(win),
        chips_text(cr::vig(win)),
        chips_text(cr::lay_pays(n, a)),
    )
}

/// A spot's words for a hover: a buy's and a lay's for what's on it (`on`), or the chip picked.
fn words(spot: Spot, point: Option<u8>, on: u64, chip: u64) -> String {
    let a = if on > 0 { on } else { chip };
    match spot {
        Spot::Pass => "Pass line: wins on 7 or 11 at the come-out, then on the point before a 7".into(),
        Spot::Odds => format!("Odds behind the pass line: up to {}× it, paid at true odds (no edge)", point.map(cr::max_odds).unwrap_or(3)),
        Spot::DontPass => "Don't pass: wins on 2 or 3 at the come-out, then on a 7 before the point".into(),
        Spot::DontOdds => format!("Lay odds behind don't pass: up to {}× it, paid at true odds on a 7 (no edge)", cr::LAY_ODDS),
        Spot::Field => "Field: one roll; 2, 3, 4, 9, 10, 11 or 12 wins".into(),
        Spot::Place(i) => format!("Place {}: pays {}", PLACE_NUMBERS[i], PLACE_PAYS[i]),
        Spot::Buy(i) => buy_words(i, a),
        Spot::Lay(i) => lay_words(i, a),
        Spot::Hard(i) => {
            let n = HARD_NUMBERS[i];
            format!("Hard {n}: {h} and {h} before a 7 or an easy {n}", h = n / 2)
        }
        Spot::AnySeven => "Any seven: one roll, pays 4 to 1".into(),
        Spot::AnyCraps => "Any craps (2, 3 or 12): one roll, pays 7 to 1".into(),
    }
}

/// The stickman's call for a roll.
fn call(r: &CrapsReveal) -> String {
    let sum = r.dice.0 + r.dice.1;
    let hard = r.dice.0 == r.dice.1;
    match (r.point_before, r.point_after) {
        (None, Some(p)) => format!("THE POINT IS {p}"),
        (None, None) => match sum {
            7 => "WINNER SEVEN!".into(),
            11 => "YO-LEVEN! WINNER".into(),
            2 => "CRAPS, ACES".into(),
            3 => "CRAPS, ACE-DEUCE".into(),
            _ => "CRAPS, TWELVE".into(),
        },
        (Some(p), None) if sum == p => {
            if hard {
                format!("{p} THE HARD WAY! WINNER")
            } else {
                format!("{p}, WINNER! MARK IT")
            }
        }
        (Some(_), None) => "SEVEN OUT".into(),
        (Some(_), Some(_)) => {
            if hard && HARD_NUMBERS.contains(&sum) {
                format!("{sum} THE HARD WAY")
            } else if matches!(sum, 2 | 3 | 4 | 9 | 10 | 11 | 12) {
                format!("{sum}, FIELD WINS")
            } else {
                format!("{sum}, ROLL AGAIN")
            }
        }
    }
}

/// Whether a bet stays up when it wins (what comes back is its win alone): place bets,
/// hardways, buys and lays. (Lay odds come down with don't pass: 0.13.25, "a win with lay odds
/// was called out with its stake counted in".)
fn stays_up(bet: &str) -> bool {
    bet.starts_with("Place") || bet.starts_with("Hard") || bet.starts_with("Buy") || (bet.starts_with("Lay") && bet != "Lay odds")
}

/// What a roll's decided bets came to: chips won (a winner that stays up wins only its win; one
/// that comes down, its win past its stake) less stakes lost.
fn net(d: &[Decided]) -> i64 {
    d.iter()
        .map(|d| {
            if d.won {
                if stays_up(&d.bet) { d.returned as i64 } else { d.returned as i64 - d.stake as i64 }
            } else {
                -(d.stake as i64)
            }
        })
        .sum()
}

#[derive(Default)]
pub struct CrapsTableUi {
    /// The table I'm at (watching, or sitting): its server and id.
    pub at: Option<(String, u64)>,
    asked: Option<Instant>,
    watch_asked: Option<Instant>,
    /// My bets as I put them down, for roll `.0` (shown till the table has them).
    desired: Option<(u64, Bets)>,
    /// The roll on the table and when the dice flew; everyone's bets as the dice flew (shown
    /// till they land: nothing gives the roll away early), with the shooter then.
    rolled: Option<(u64, Instant)>,
    flying: Option<(u64, Vec<Option<Bets>>, Option<u8>)>,
    /// My chips while my bets are riding (the balance waits for the dice), and the last roll
    /// whose result was called for me.
    playing_chips: Option<u64>,
    told: Option<u64>,
    /// My chips on each bet as I put them down, chip by chip (drawn so while they still come to
    /// what's on it).
    piles: Vec<(Spot, Vec<u64>)>,
}

impl CrapsTableUi {
    /// My pile on a bet, if its chips come to what's on it.
    fn pile(&self, spot: Spot, amount: u64) -> Option<&Vec<u64>> {
        self.piles.iter().find(|(s, _)| *s == spot).map(|(_, c)| c).filter(|c| amount > 0 && c.iter().sum::<u64>() == amount)
    }

    /// A chip onto my pile on a bet that had `before` on it (the chips that come to, when the
    /// pile isn't as I put it down any more: set to fit the table, say).
    fn pile_add(&mut self, spot: Spot, before: u64, chip: u64) {
        let mut chips = self.pile(spot, before).cloned().unwrap_or_else(|| chips_for(before));
        chips.push(chip);
        self.piles.retain(|(s, _)| *s != spot);
        self.piles.push((spot, chips));
    }
}

/// A button on the controls; true when it's clicked.
fn button(ui: &mut egui::Ui, r: Rect, id: &str, text: &str, sub: Option<String>, fill: Color32, enabled: bool) -> bool {
    let resp = ui.interact(r, ui.id().with(("crt", id)), Sense::click());
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
    /// Leaves the craps table I'm at (standing up: what may come off comes back; a pass line with
    /// a point on keeps working, and is paid when it's decided).
    pub fn craps_table_leave(&mut self) {
        if let Some((key, id)) = self.poker.casino.crt.at.take() {
            self.engine.poker(&key, ToServer::CrapsLeave { table: id });
            self.engine.poker(&key, ToServer::CrapsUnwatch { table: id });
            self.engine.craps_close(&key);
        }
        let t = &mut self.poker.casino.crt;
        t.desired = None;
        t.rolled = None;
        t.playing_chips = None;
        t.piles.clear();
    }

    /// The craps tables' page: the tables along the top, the table, the seats, the controls.
    pub(super) fn craps_table_page(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let now = Instant::now();
        let key = s.key.clone();
        if self.poker.casino.crt.asked.is_none_or(|t| t.elapsed() > Duration::from_secs(8)) {
            self.poker.casino.crt.asked = Some(now);
            self.engine.poker(&key, ToServer::CrapsTables);
        }
        let tables = s.poker.craps_tables.clone().unwrap_or_default();
        let mine = self.poker.casino.crt.at.clone().filter(|(k, _)| *k == key).map(|(_, id)| id);
        let at = match mine {
            Some(id) if tables.is_empty() || tables.iter().any(|t| t.id == id) => Some(id),
            _ => tables.first().map(|t| t.id),
        };
        let live = s.poker.craps.clone().filter(|c| Some(c.view.id) == at);
        if let Some(id) = at {
            let moved = self.poker.casino.crt.at != Some((key.clone(), id));
            if moved || (live.is_none() && self.poker.casino.crt.watch_asked.is_none_or(|t| t.elapsed() > Duration::from_secs(3))) {
                self.poker.casino.crt.at = Some((key.clone(), id));
                self.poker.casino.crt.watch_asked = Some(now);
                self.engine.poker(&key, ToServer::CrapsWatch { table: id });
            }
        }
        let p = ui.painter().clone();
        let time = ui.ctx().input(|i| i.time) as f32;
        vgrad(&p, area, Color32::from_rgb(0x07, 0x14, 0x0e), Color32::from_rgb(0x02, 0x06, 0x04));
        let strip = Rect::from_min_size(area.min, vec2(area.width(), STRIP_H));
        let bar = Rect::from_min_max(pos2(area.left() + 16.0, area.bottom() - BAR_H + 6.0), pos2(area.right() - 16.0, area.bottom() - 10.0));
        self.crt_tables_strip(ui, s, strip, &tables, at);
        // The table: a wooden rail round the felt, the seats along the near side.
        let room = Rect::from_min_max(pos2(area.left() + 22.0, strip.bottom() + 16.0), pos2(area.right() - 22.0, bar.top() - 10.0));
        let plates = Rect::from_min_max(pos2(room.left(), room.bottom() - PLATES_H), room.max);
        let felt = Rect::from_min_max(room.min + vec2(16.0, 14.0), pos2(room.right() - 16.0, plates.top() - 18.0));
        let rail = felt.expand(14.0);
        p.rect_filled(rail.translate(vec2(0.0, 10.0)), theme::r(34), Color32::from_black_alpha(140));
        vgrad(&p, rail, Color32::from_rgb(0x5a, 0x34, 0x1a), Color32::from_rgb(0x2a, 0x16, 0x0a));
        p.rect_stroke(rail, theme::r(30), Stroke::new(1.5f32, col::GOLD_DARK), StrokeKind::Inside);
        vgrad(&p, felt, Color32::from_rgb(0x0f, 0x60, 0x3e), Color32::from_rgb(0x08, 0x42, 0x2a));
        radial(&p, felt.center(), felt.width() * 0.55, Color32::from_rgba_unmultiplied(0xff, 0xf0, 0xc8, 18), Color32::TRANSPARENT);
        p.rect_stroke(felt, theme::r(22), Stroke::new(2.0f32, col::GOLD_DARK), StrokeKind::Inside);
        let Some(live) = live else {
            p.text(felt.center(), Align2::CENTER_CENTER, if s.poker.craps_tables.is_none() { "Setting out the tables…" } else { "Sitting you at the table…" }, theme::bold_font(16.0), col::MUTED);
            return;
        };
        let v = live.view.clone();
        // (What this server's tables take: buys and lays since 0.13.23, lay odds since 0.13.6.
        // An older server's table keeps the layout it knows, as 0.13.28 shows it there.)
        let strips = s.has(pyr_proto::control::CRAPS_BUY_LAY_FEATURE);
        let lay_odds = s.has(LAY_ODDS_FEATURE);
        let spots = self.crt_layout(&p, felt, &v, strips, lay_odds);
        // The roll on the table, on its own clock (joined late: part way through already).
        let shown = (v.stage == CrapsStage::Shown).then(|| v.last.clone()).flatten();
        if let Some(r) = &shown
            && self.poker.casino.crt.rolled.is_none_or(|(n, _)| n != r.roll)
        {
            let ms_left = v.ms_left.saturating_sub(live.at.elapsed().as_millis() as u64);
            let since = Duration::from_millis(SHOW_MS.saturating_sub(ms_left));
            let rolled = now.checked_sub(since).unwrap_or(now);
            self.poker.casino.crt.rolled = Some((r.roll, rolled));
            let k = since.as_secs_f32();
            if k < 0.3 {
                self.poker.casino.sound_in(0.0, Cue::DiceRoll, 0.3, 0.8);
            }
            self.crt_result_for_me(&v, r, rolled, now);
        }
        let since = self.poker.casino.crt.rolled.filter(|(n, _)| shown.as_ref().is_some_and(|r| r.roll == *n)).map(|(_, d)| now.duration_since(d).as_secs_f32());
        let landed = since.is_some_and(|k| k >= ROLL_SECS);
        if v.stage == CrapsStage::Betting {
            self.poker.casino.crt.flying = Some((v.roll_no, v.seats.iter().map(|x| x.as_ref().map(|x| x.bets.clone())).collect(), v.shooter));
        }
        // (While the dice fly: the bets and the shooter as they were when they were thrown.)
        let before = self.poker.casino.crt.flying.clone().filter(|(n, ..)| !landed && shown.as_ref().is_some_and(|r| r.roll == *n));
        // My chips while my bets ride.
        let mine_in = v.my_seat.and_then(|m| v.seats.get(m as usize).cloned().flatten()).is_some_and(|x| x.bets.total() > 0);
        if mine_in && v.stage == CrapsStage::Betting {
            self.poker.casino.crt.playing_chips = Some(chips_now(s));
        }
        // Everyone's chips on the layout (mine as I put them down, till the table has them).
        let desired = self.poker.casino.crt.desired.clone().filter(|(n, _)| *n == v.roll_no).map(|(_, b)| b);
        for (i, seat) in v.seats.iter().enumerate() {
            let Some(x) = seat else { continue };
            let me = v.my_seat == Some(i as u8);
            let mut bets = if me { desired.clone().unwrap_or_else(|| x.bets.clone()) } else { x.bets.clone() };
            if let Some((_, was, _)) = &before {
                bets = was.get(i).cloned().flatten().unwrap_or_default();
            }
            let ghost = me && before.is_none() && desired.as_ref().is_some_and(|d| *d != x.bets);
            for (spot, r) in &spots {
                let a = amount(&bets, *spot);
                if a == 0 {
                    continue;
                }
                let c = chip_at(*r, *spot, i);
                let cr_ = chip_r(*spot);
                let ring = SEAT_COLORS[i];
                p.circle_stroke(c + vec2(0.0, 1.0), cr_ + 2.5, Stroke::new(if me { 2.5f32 } else { 1.6 }, ring.gamma_multiply(if me { 1.0 } else { 0.75 })));
                let mut g = p.clone();
                if ghost {
                    g.set_opacity(0.6);
                }
                // (Mine as I piled them, while they still come to what's on the bet.)
                match self.poker.casino.crt.pile(*spot, a).filter(|_| me) {
                    Some(chips) => chip_pile(&g, c, cr_, chips),
                    None => chip_stack(&g, c, cr_, a),
                }
            }
        }
        // Clicks on the layout: my bets.
        self.crt_clicks(ui, s, &v, &spots, desired.clone(), landed || v.stage != CrapsStage::Shown);
        // The dice, thrown from the shooter's seat; the call; each seat's result.
        if let (Some(r), Some(k)) = (&shown, since) {
            let land = pos2(felt.center().x - 20.0, felt.top() + felt.height() * 0.62);
            let from = self.crt_plate_at(plates, r.shooter.unwrap_or(2) as usize);
            for (i, face) in [r.dice.0, r.dice.1].iter().enumerate() {
                let u = (k / ROLL_SECS).min(1.0);
                let end = land + vec2(i as f32 * 66.0, (i as f32 - 0.5) * 16.0);
                let e = ease_out(u);
                let bounce = ((u * 9.0).sin() * (1.0 - u) * 50.0).abs();
                let q = from + (end - from) * e - vec2(0.0, bounce);
                let shown_face = if u < 0.85 { ((k * 23.0 + i as f32 * 3.0) as u8 % 6) + 1 } else { *face };
                let angle = (1.0 - u) * 9.0 + i as f32;
                art::die(&p, q, 52.0, shown_face, angle, Color32::from_rgb(0xd8, 0x1f, 0x2e));
            }
            if landed {
                let a = ((k - ROLL_SECS) / 0.25).min(1.0);
                let text = call(r);
                let g = p.layout_no_wrap(text.clone(), theme::bold_font(34.0), col::GOLD2);
                let plate = Rect::from_center_size(pos2(land.x + 33.0, land.y - 70.0), g.size() + vec2(36.0, 14.0));
                p.rect_filled(plate, theme::r(14), Color32::from_black_alpha((205.0 * a) as u8));
                p.rect_stroke(plate, theme::r(14), Stroke::new(1.0f32, col::GOLD.gamma_multiply(0.6 * a)), StrokeKind::Inside);
                glow_text(&p, plate.center(), Align2::CENTER_CENTER, &text, theme::bold_font(34.0), col::GOLD2.gamma_multiply(a), Color32::from_black_alpha((120.0 * a) as u8));
            }
        }
        // The seats (the shooter's the one who threw, till the dice land).
        let mut seats_view = v.clone();
        if let Some((_, _, shooter)) = &before {
            seats_view.shooter = *shooter;
        }
        for i in 0..SEATS {
            let seat = v.seats.get(i).cloned().flatten();
            let result = if landed { seat.as_ref().filter(|x| !x.decided.is_empty()).map(|x| net(&x.decided)) } else { None };
            self.crt_seat(ui, s, &seats_view, i as u8, seat.as_ref(), plates, result, time);
        }
        // The rolls lately, along the top of the felt.
        if let Some(c) = &s.poker.craps {
            let mut x = felt.right() - 10.0;
            for (a, b) in c.rolls.iter().take(12).skip(if since.is_some_and(|k| k < ROLL_SECS) { 1 } else { 0 }) {
                let t = format!("{}", a + b);
                let r = Rect::from_min_size(pos2(x - 28.0, felt.top() - 30.0), vec2(28.0, 20.0));
                p.rect_filled(r, theme::r(6), if a + b == 7 { col::RED } else { Color32::from_black_alpha(140) });
                p.text(r.center(), Align2::CENTER_CENTER, t, theme::bold_font(11.5), Color32::WHITE);
                x -= 32.0;
            }
            if v.streak > 0 {
                p.text(pos2(x - 6.0, felt.top() - 20.0), Align2::RIGHT_CENTER, format!("Shooter's points: {}", v.streak), theme::bold_font(12.0), col::GOLD);
            }
        }
        self.crt_controls(ui, s, &live, bar, now);
        // Emotes, big over the plate of whoever sent them.
        super::emotes::pops(&p, ui.ctx(), &s.poker.game_emotes, TableGame::Craps, v.id, 112.0, |seat| self.crt_plate_at(plates, (seat as usize).min(SEATS - 1)) - vec2(0.0, 58.0));
    }

    /// The layout, with the puck on the point: each bet's spot. `strips`: with LAY and BUY
    /// along each number's box; `lay_odds`: with LAY ODDS beside don't pass on a point.
    fn crt_layout(&self, p: &egui::Painter, felt: Rect, v: &CrapsView, strips: bool, lay_odds: bool) -> Vec<(Spot, Rect)> {
        let w = felt.width();
        let line = Stroke::new(2.0f32, Color32::from_white_alpha(200));
        let thin = Stroke::new(1.0f32, Color32::from_white_alpha(110));
        let mut spots: Vec<(Spot, Rect)> = Vec::new();
        let main = Rect::from_min_max(felt.min + vec2(18.0, 16.0), pos2(felt.left() + w * 0.68, felt.bottom() - 16.0));
        let bw = main.width() / 6.0;
        let place_h = main.height() * if strips { 0.42 } else { 0.34 };
        for (i, n) in PLACE_NUMBERS.iter().enumerate() {
            // A number's box: LAY along its top, the number (its place bet), BUY along its bottom.
            let r = Rect::from_min_size(pos2(main.left() + i as f32 * bw, main.top()), vec2(bw, place_h));
            p.rect_stroke(r, 0, line, StrokeKind::Inside);
            let label = if *n == 6 { "SIX".to_string() } else if *n == 9 { "NINE".to_string() } else { n.to_string() };
            let size = (place_h * 0.24).min(bw * 0.22);
            let ink = if *n == 6 || *n == 8 { col::RED } else { col::GOLD2 };
            let pays = ["9:5", "7:5", "7:6", "7:6", "7:5", "9:5"][i];
            let pays_font = theme::bold_font((bw * 0.1).clamp(9.0, 12.0));
            let num = if strips {
                let lay = Rect::from_min_max(r.min, pos2(r.right(), r.top() + place_h * 0.2));
                let buy = Rect::from_min_max(pos2(r.left(), r.top() + place_h * 0.78), r.max);
                let num = Rect::from_min_max(lay.left_bottom(), buy.right_top());
                // (The strips before the number: 0.13.28 puts a seat's place chips down over
                // its buy's tag.)
                spots.push((Spot::Lay(i), lay));
                spots.push((Spot::Buy(i), buy));
                p.hline(r.x_range(), lay.bottom(), thin);
                p.hline(r.x_range(), buy.top(), thin);
                p.text(lay.left_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, "LAY", theme::bold_font(11.0), Color32::from_white_alpha(150));
                p.text(buy.left_center() + vec2(6.0, 0.0), Align2::LEFT_CENTER, "BUY", theme::bold_font(11.0), col::GOLD2);
                p.text(num.center_top() + vec2(0.0, num.height() * 0.24), Align2::CENTER_CENTER, label, theme::bold_font(size), ink);
                p.text(num.center_top() + vec2(0.0, num.height() * 0.44), Align2::CENTER_CENTER, pays, pays_font, Color32::from_white_alpha(140));
                num
            } else {
                // (A table without buys and lays: the box is all the number's, as 0.13.0's.)
                p.text(r.center_top() + vec2(0.0, place_h * 0.2), Align2::CENTER_CENTER, label, theme::bold_font(size), ink);
                p.text(r.center_top() + vec2(0.0, place_h * 0.38), Align2::CENTER_CENTER, pays, pays_font, Color32::from_white_alpha(140));
                r
            };
            spots.push((Spot::Place(i), num));
            if v.point == Some(*n) {
                let pc = num.right_top() + vec2(-18.0, 18.0);
                p.circle_filled(pc + vec2(1.0, 2.0), 15.0, Color32::from_black_alpha(120));
                p.circle_filled(pc, 15.0, Color32::WHITE);
                p.circle_stroke(pc, 15.0, Stroke::new(2.0f32, Color32::BLACK));
                p.text(pc, Align2::CENTER_CENTER, "ON", theme::bold_font(10.5), Color32::BLACK);
            }
        }
        let rest = Rect::from_min_max(pos2(main.left(), main.top() + place_h), main.max);
        let band = rest.height() / 3.0;
        let field = Rect::from_min_size(rest.min, vec2(rest.width(), band));
        let dont = Rect::from_min_size(pos2(rest.left(), rest.top() + band), vec2(rest.width(), band * 0.8));
        let pass = Rect::from_min_max(pos2(rest.left(), rest.top() + band * 1.8), rest.max);
        p.rect_stroke(field, 0, line, StrokeKind::Inside);
        p.text(field.left_top() + vec2(16.0, band * 0.3), Align2::LEFT_CENTER, "FIELD", theme::bold_font(20.0), col::GOLD2);
        p.text(field.left_top() + vec2(16.0, band * 0.62), Align2::LEFT_CENTER, "2 double · 12 triple", theme::font(11.0), Color32::from_white_alpha(190));
        p.text(field.center_top() + vec2(field.width() * 0.12, band * 0.26), Align2::CENTER_CENTER, "2 · 3 · 4 · 9 · 10 · 11 · 12", theme::bold_font(15.0), Color32::from_white_alpha(230));
        spots.push((Spot::Field, field));
        if v.point.is_none() {
            // (At the field's far end since 0.13.23: it covered the word FIELD.)
            let pc = field.right_top() + vec2(-24.0, 24.0);
            p.circle_filled(pc, 15.0, Color32::from_rgb(0x18, 0x18, 0x1c));
            p.circle_stroke(pc, 15.0, Stroke::new(2.0f32, Color32::WHITE));
            p.text(pc, Align2::CENTER_CENTER, "OFF", theme::bold_font(9.5), Color32::WHITE);
        }
        // Don't pass, and its lay odds beside it once there's a point.
        p.rect_stroke(dont, 0, line, StrokeKind::Inside);
        let lay_odds = lay_odds && v.point.is_some();
        let dont_label = if lay_odds { Rect::from_min_size(dont.min, vec2(dont.width() * 0.74, dont.height())) } else { dont };
        p.text(dont_label.center_top() + vec2(0.0, dont.height() * 0.26), Align2::CENTER_CENTER, "DON'T PASS BAR", theme::bold_font(15.0), Color32::from_white_alpha(230));
        spots.push((Spot::DontPass, dont_label));
        if lay_odds {
            let behind = Rect::from_min_max(pos2(dont_label.right(), dont.top()), dont.max);
            p.rect_stroke(behind.shrink(5.0), theme::r(10), Stroke::new(1.5f32, col::GOLD), StrokeKind::Inside);
            p.text(behind.center_top() + vec2(0.0, dont.height() * 0.26), Align2::CENTER_CENTER, "LAY ODDS", theme::bold_font(13.0), col::GOLD2);
            spots.push((Spot::DontOdds, behind));
        }
        p.rect_stroke(pass, 0, line, StrokeKind::Inside);
        let pass_label = Rect::from_min_size(pass.min, vec2(pass.width() * 0.74, pass.height()));
        p.text(pass_label.center_top() + vec2(0.0, pass.height() * 0.26), Align2::CENTER_CENTER, "PASS LINE", theme::bold_font(20.0), Color32::WHITE);
        spots.push((Spot::Pass, pass_label));
        let odds = Rect::from_min_max(pos2(pass_label.right(), pass.top()), pass.max);
        if v.point.is_some() {
            p.rect_stroke(odds.shrink(5.0), theme::r(10), Stroke::new(1.5f32, col::GOLD), StrokeKind::Inside);
            p.text(odds.center_top() + vec2(0.0, odds.height() * 0.24), Align2::CENTER_CENTER, "ODDS", theme::bold_font(13.0), col::GOLD2);
            spots.push((Spot::Odds, odds));
        }
        let props = Rect::from_min_max(pos2(main.right() + 18.0, main.top()), pos2(felt.right() - 18.0, felt.bottom() - 16.0));
        p.rect_stroke(props, theme::r(10), line, StrokeKind::Inside);
        let ph = props.height() / 4.0;
        let seven = Rect::from_min_size(props.min, vec2(props.width(), ph * 0.9));
        p.text(seven.center_top() + vec2(0.0, ph * 0.26), Align2::CENTER_CENTER, "ANY SEVEN  ·  4 to 1", theme::bold_font(14.0), col::RED);
        spots.push((Spot::AnySeven, seven));
        for (i, n) in HARD_NUMBERS.iter().enumerate() {
            let (cx, cy) = (i % 2, i / 2);
            let r = Rect::from_min_size(pos2(props.left() + cx as f32 * props.width() / 2.0, props.top() + ph * 0.9 + cy as f32 * ph * 1.05), vec2(props.width() / 2.0, ph * 1.05));
            p.rect_stroke(r, 0, Stroke::new(1.0f32, Color32::from_white_alpha(120)), StrokeKind::Inside);
            let half = n / 2;
            let d = (r.height() * 0.24).min(r.width() * 0.2);
            art::die(p, r.center_top() + vec2(-d * 0.62, d * 0.9), d, half, 0.0, Color32::from_rgb(0xf4, 0xf0, 0xe8));
            art::die(p, r.center_top() + vec2(d * 0.62, d * 0.9), d, half, 0.0, Color32::from_rgb(0xf4, 0xf0, 0xe8));
            p.text(r.center_top() + vec2(0.0, d * 1.9), Align2::CENTER_CENTER, format!("{} to 1", if *n == 4 || *n == 10 { 7 } else { 9 }), theme::bold_font(11.0), col::GOLD2);
            spots.push((Spot::Hard(i), r));
        }
        let crapsr = Rect::from_min_max(pos2(props.left(), props.top() + ph * 3.0), props.max);
        p.text(crapsr.center_top() + vec2(0.0, crapsr.height() * 0.26), Align2::CENTER_CENTER, "ANY CRAPS  ·  7 to 1", theme::bold_font(14.0), col::RED);
        spots.push((Spot::AnyCraps, crapsr));
        spots
    }

    /// Clicks on the layout: the chip picked onto a bet (right-click takes it down), where the
    /// game allows it now; a bet's words by the pointer.
    fn crt_clicks(&mut self, ui: &mut egui::Ui, s: &ServerView, v: &CrapsView, spots: &[(Spot, Rect)], desired: Option<Bets>, open: bool) {
        let Some(me) = v.my_seat else { return };
        let Some(seat) = v.seats.get(me as usize).cloned().flatten() else { return };
        let p = ui.painter().clone();
        let have = desired.unwrap_or_else(|| seat.bets.clone());
        let table = Table { point: v.point, bets: seat.bets.clone(), streak: v.streak };
        let chip = self.poker.casino.chip;
        // The chips I have left to put down (what's on the table is taken already).
        let left = (chips_now(s) + seat.bets.total()).saturating_sub(have.total());
        let mut tip = None;
        for (spot, r) in spots {
            let resp = ui.interact(*r, ui.id().with(("crt-spot", format!("{spot:?}"))), Sense::click());
            if resp.hovered() {
                p.rect_filled(*r, 0, Color32::from_white_alpha(16));
                tip = Some(words(*spot, v.point, amount(&have, *spot), chip));
                if open {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
            }
            if !open {
                continue;
            }
            if resp.clicked() {
                let before = amount(&have, *spot);
                let mut want = have.clone();
                *slot(&mut want, *spot) += chip;
                if *spot == Spot::Odds
                    && let Some(pt) = v.point
                {
                    want.pass_odds = want.pass_odds.min(want.pass * cr::max_odds(pt));
                }
                // Each bet brought into what this table takes first.
                let fitted = pyr_poker::crapsroom::fit(&mut want, v.min_bet, v.max_bet);
                let fit_words = format!("Bets here are {} to {}: yours set to fit", chips_text(v.min_bet), chips_text(v.max_bet));
                let more = want.total().saturating_sub(have.total());
                if want == have {
                    // (At the table's most already.)
                    if fitted {
                        self.toast(fit_words, false);
                    }
                    continue;
                }
                if let Err(e) = cr::check_change(&table, &want) {
                    self.toast(format!("Can't: {e}"), false);
                } else if more > left {
                    // (0.13.24: a bet that's more than I have says so and stays off the table.
                    // Better than 0.13.28: it said to pick a smaller chip when the rack had none.)
                    let smaller = super::table_chips(v.min_bet, v.max_bet).0 < chip;
                    self.toast(format!("You have {} chips left to put down{}", chips_text(left), if chip > left && smaller { ": pick a smaller chip" } else { "" }), false);
                } else {
                    if fitted {
                        self.toast(fit_words, false);
                    }
                    self.poker.casino.crt.pile_add(*spot, before, chip);
                    self.crt_send(s, v, want, false);
                }
            } else if resp.secondary_clicked() && amount(&have, *spot) > 0 {
                let mut want = have.clone();
                *slot(&mut want, *spot) = 0;
                match cr::check_change(&table, &want) {
                    Ok(()) => self.crt_send(s, v, want, false),
                    Err(e) => self.toast(format!("Can't: {e}"), false),
                }
            }
        }
        if let Some(t) = tip {
            // By the pointer, wrapped, and kept on the screen (0.13.23).
            let g = p.layout(t, theme::bold_font(13.0), col::GOLD2, 420.0);
            let size = g.size() + vec2(16.0, 10.0);
            let screen = ui.ctx().content_rect().shrink(6.0);
            let at = ui.ctx().pointer_hover_pos().unwrap_or_default() + vec2(14.0, 18.0);
            let at = pos2(at.x.min(screen.right() - size.x).max(screen.left()), at.y.min(screen.bottom() - size.y).max(screen.top()));
            let r = Rect::from_min_size(at, size);
            // (Over the chat button and whatever else is on the page.)
            let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, ui.id().with("crt-tip")));
            p.rect_filled(r, theme::r(8), Color32::from_black_alpha(220));
            p.galley(r.min + vec2(8.0, 5.0), g, col::GOLD2);
        }
    }

    /// My bets to the table (shown at once; the table's own once it has them).
    fn crt_send(&mut self, s: &ServerView, v: &CrapsView, bets: Bets, ready: bool) {
        self.poker.casino.crt.desired = Some((v.roll_no, bets.clone()));
        self.poker.casino.sound_in(0.0, Cue::Chips, 0.0, 0.45);
        self.engine.craps_bets(&s.key, v.id, v.roll_no, bets, ready);
    }

    /// Where seat `i`'s plate is along the rail.
    fn crt_plate_at(&self, plates: Rect, i: usize) -> Pos2 {
        // (Seat 0 on the right, as at the other tables: the dealer's left.)
        let w = plates.width() / SEATS as f32;
        pos2(plates.right() - w * (i as f32 + 0.5), plates.center().y)
    }

    /// One seat along the rail: who's in it (the shooter with the dice), or an open seat; what
    /// the roll did for them.
    #[allow(clippy::too_many_arguments)]
    fn crt_seat(&mut self, ui: &mut egui::Ui, s: &ServerView, v: &CrapsView, i: u8, seat: Option<&CrapsSeatView>, plates: Rect, result: Option<i64>, time: f32) {
        let p = ui.painter().clone();
        let c = self.crt_plate_at(plates, i as usize);
        let me = v.my_seat == Some(i);
        let color = SEAT_COLORS[i as usize];
        let w = (plates.width() / SEATS as f32 - 14.0).min(230.0);
        let plate = Rect::from_center_size(c, vec2(w, 52.0));
        match seat {
            Some(x) => {
                let shooter = v.shooter == Some(i);
                if me {
                    radial(&p, c, w * 0.7, col::GOLD.gamma_multiply(0.12 + 0.04 * (time * 2.0).sin()), Color32::TRANSPARENT);
                }
                p.rect_filled(plate.translate(vec2(0.0, 3.0)), theme::r(16), Color32::from_black_alpha(120));
                p.rect_filled(plate, theme::r(16), Color32::from_rgba_unmultiplied(0x10, 0x0c, 0x08, 235));
                p.rect_stroke(plate, theme::r(16), Stroke::new(if me { 2.0f32 } else { 1.2 }, color.gamma_multiply(if me { 1.0 } else { 0.7 })), StrokeKind::Inside);
                let face_c = plate.left_center() + vec2(26.0, 0.0);
                if x.bot {
                    crate::poker::table::bot_face(&p, face_c, 17.0, &x.name, pyr_poker::bots::Level::Regular);
                } else {
                    let face = self.face(ui.ctx(), s, &x.player);
                    crate::app::avatar(&p, face_c, 17.0, &x.player, &x.name, false, Color32::BLACK, face);
                }
                let name: String = if x.name.chars().count() > 13 { format!("{}…", x.name.chars().take(12).collect::<String>()) } else { x.name.clone() };
                p.text(face_c + vec2(24.0, -9.0), Align2::LEFT_CENTER, &name, theme::bold_font(13.5), if me { col::GOLD2 } else { col::TEXT });
                let on = x.bets.total();
                let sub = if on > 0 { format!("{} on the layout", chips_text(on)) } else { "no bets".into() };
                p.text(face_c + vec2(24.0, 10.0), Align2::LEFT_CENTER, sub, theme::font(11.0), col::MUTED);
                if x.bot {
                    let tag = Rect::from_min_size(pos2(plate.right() - 40.0, plate.top() + 6.0), vec2(32.0, 15.0));
                    p.rect_filled(tag, theme::r(5), Color32::from_rgb(0x3d, 0x5f, 0xc4));
                    p.text(tag.center(), Align2::CENTER_CENTER, "BOT", theme::bold_font(9.0), Color32::WHITE);
                    let resp = ui.interact(plate, ui.id().with(("crt-bot", i)), Sense::click()).on_hover_text(format!("{}: a bot that bets {}. It plays with the house's chips.", x.name, x.style));
                    resp.context_menu(|ui| {
                        crate::menu::style(ui);
                        if ui.button(format!("Ask {} to leave", x.name)).clicked() {
                            self.engine.poker(&s.key, ToServer::CrapsBot { table: v.id, seat: i, add: false });
                            ui.close();
                        }
                    });
                }
                if shooter {
                    // The dice, by the shooter.
                    let d = pos2(plate.right() - 22.0, plate.bottom() - 16.0);
                    art::die(&p, d - vec2(9.0, 0.0), 15.0, 5, -0.2, Color32::from_rgb(0xd8, 0x1f, 0x2e));
                    art::die(&p, d + vec2(8.0, 2.0), 15.0, 2, 0.3, Color32::from_rgb(0xd8, 0x1f, 0x2e));
                    let tag = Rect::from_center_size(plate.center_top() - vec2(0.0, 2.0), vec2(74.0, 18.0));
                    p.rect_filled(tag, theme::r(9), col::RED);
                    p.text(tag.center(), Align2::CENTER_CENTER, "SHOOTER", theme::bold_font(10.5), Color32::WHITE);
                } else if x.ready && v.stage == CrapsStage::Betting {
                    let tag = Rect::from_center_size(plate.center_top() - vec2(0.0, 2.0), vec2(56.0, 16.0));
                    p.rect_filled(tag, theme::r(8), Color32::from_rgb(0x1e, 0x5a, 0x34));
                    p.text(tag.center(), Align2::CENTER_CENTER, "READY", theme::bold_font(9.5), col::WIN);
                }
                if let Some(n) = result.filter(|n| *n != 0) {
                    let (text, fill) = if n > 0 { (format!("+{}", chips_text(n as u64)), col::WIN) } else { (format!("−{}", chips_text(n.unsigned_abs())), col::LOSE) };
                    let r = Rect::from_center_size(plate.center_top() - vec2(0.0, 24.0), vec2(16.0 + text.len() as f32 * 9.0, 24.0));
                    p.rect_filled(r, theme::r(12), Color32::from_black_alpha(215));
                    p.rect_stroke(r, theme::r(12), Stroke::new(1.5f32, fill), StrokeKind::Inside);
                    p.text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(14.0), fill);
                }
            }
            None => {
                p.rect_stroke(plate, theme::r(16), Stroke::new(1.0f32, color.gamma_multiply(0.3)), StrokeKind::Inside);
                let sitting = v.my_seat.is_some();
                if !sitting {
                    let sr = Rect::from_center_size(plate.center() - vec2(w * 0.22, 0.0), vec2(w * 0.38, 30.0));
                    let resp = ui.interact(sr, ui.id().with(("crt-sit", i)), Sense::click());
                    p.rect_filled(sr, theme::r(10), if resp.hovered() { col::GOLD } else { Color32::from_black_alpha(170) });
                    p.text(sr.center(), Align2::CENTER_CENTER, "SIT", theme::bold_font(12.0), if resp.hovered() { Color32::BLACK } else { col::GOLD });
                    if resp.on_hover_text("Sit here").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.poker.casino.crt.desired = None;
                        self.engine.poker(&s.key, ToServer::CrapsSit { table: v.id, seat: i });
                    }
                }
                let br = Rect::from_center_size(plate.center() + vec2(if sitting { 0.0 } else { w * 0.22 }, 0.0), vec2(w * 0.38, 30.0));
                let bresp = ui.interact(br, ui.id().with(("crt-addbot", i)), Sense::click());
                p.rect_filled(br, theme::r(10), if bresp.hovered() { Color32::from_rgb(0x3d, 0x5f, 0xc4) } else { Color32::from_black_alpha(150) });
                p.text(br.center(), Align2::CENTER_CENTER, "+ BOT", theme::bold_font(11.5), if bresp.hovered() { Color32::WHITE } else { Color32::from_white_alpha(150) });
                if bresp.on_hover_text("Seat a bot here: each bets its own way, with the house's chips").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.engine.poker(&s.key, ToServer::CrapsBot { table: v.id, seat: i, add: true });
                }
            }
        }
    }

    /// A roll's result for me, called out big once the dice land, the balance with it.
    fn crt_result_for_me(&mut self, v: &CrapsView, r: &CrapsReveal, rolled: Instant, now: Instant) {
        if self.poker.casino.crt.told == Some(r.roll) {
            return;
        }
        self.poker.casino.crt.told = Some(r.roll);
        let Some(me) = v.my_seat.and_then(|m| v.seats.get(m as usize).cloned().flatten()) else { return };
        let at = rolled + Duration::from_secs_f32(ROLL_SECS + 0.2);
        let back = at.saturating_duration_since(now).as_secs_f32();
        if let Some(chips) = self.poker.casino.crt.playing_chips.take() {
            self.poker.casino.hold = Some((chips, at));
        }
        if me.decided.is_empty() {
            return;
        }
        let n = net(&me.decided);
        let wagered: u64 = me.decided.iter().map(|d| d.stake).sum();
        if n > 0 {
            self.poker.casino.announce("YOU WIN", format!("+{}", chips_text(n as u64)), col::WIN, (0.5, 0.24), back);
            self.poker.casino.sound_in(back, Cue::WinSmall, 0.0, 0.8);
            self.poker.casino.sound_in(back + 0.2, Cue::Chips, 0.0, 0.7);
        }
        self.poker.casino.celebrate(me.paid, wagered.max(1), false, back);
    }

    /// The tables, along the top: a card each (as 0.13.28's, see `picker`); click one to go to it
    /// (standing up from this one).
    fn crt_tables_strip(&mut self, ui: &mut egui::Ui, s: &ServerView, strip: Rect, tables: &[pyr_poker::net::CrapsSummary], at: Option<u64>) {
        let p = ui.painter().clone();
        vgrad(&p, strip, Color32::from_rgb(0x0c, 0x14, 0x10), Color32::from_rgb(0x05, 0x08, 0x06));
        let picks: Vec<super::picker::Pick> = tables.iter().map(|t| super::picker::Pick { id: t.id, name: t.name.clone(), min: t.min_bet, max: t.max_bet, seated: t.people + t.bots, note: t.point.map(|n| format!("point {n}")) }).collect();
        if let Some(id) = super::picker::cards(ui, strip, &picks, SEATS, at, "crt-table", 12.0) {
            self.craps_table_leave();
            self.poker.casino.crt.at = Some((s.key.clone(), id));
            self.engine.poker(&s.key, ToServer::CrapsWatch { table: id });
            self.poker.casino.crt.watch_asked = Some(Instant::now());
        }
    }

    /// The controls along the bottom: the rack, Clear, Ready, and the shooter's Throw; or,
    /// watching, a seat to sit in.
    fn crt_controls(&mut self, ui: &mut egui::Ui, s: &ServerView, live: &pyr_engine::poker::SharedLive<CrapsView>, bar: Rect, now: Instant) {
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
            let mut words = match free {
                Some(_) => format!("{}: bets {} to {}. Sit in an open seat to play.", v.name, chips_text(v.min_bet), chips_text(v.max_bet)),
                None => format!("{} is full: watch, or try another table.", v.name),
            };
            if v.left.total() > 0 {
                words = format!("Your pass line's still working: {}. {words}", chips_text(v.left.total()));
            }
            say(words, col::MUTED);
            if let Some(seat) = free {
                let r = Rect::from_min_size(pos2(bar.right() - 176.0, bar.center().y - 24.0), vec2(160.0, 48.0));
                if button(ui, r, "sit", "Sit down", None, col::GOLD, true) {
                    self.poker.casino.crt.desired = None;
                    self.engine.poker(&s.key, ToServer::CrapsSit { table: v.id, seat: seat as u8 });
                }
            }
            return;
        };
        let mine = v.seats.get(me as usize).cloned().flatten();
        let on = self.poker.casino.crt.desired.clone().filter(|(n, _)| *n == v.roll_no).map(|(_, b)| b).or_else(|| mine.as_ref().map(|x| x.bets.clone())).unwrap_or_default();
        let ready = mine.as_ref().is_some_and(|x| x.ready);
        let shooter = v.shooter == Some(me);
        let up = Rect::from_min_size(pos2(bar.right() - 126.0, bar.center().y - 22.0), vec2(112.0, 44.0));
        if button(ui, up, "stand", "Stand up", None, Color32::from_white_alpha(30), true) {
            self.engine.poker(&s.key, ToServer::CrapsLeave { table: v.id });
            self.poker.casino.crt.desired = None;
            return;
        }
        let mut x = up.left() - 12.0;
        let mut place = |w: f32| {
            let r = Rect::from_min_size(pos2(x - w, bar.center().y - 24.0), vec2(w, 48.0));
            x -= w + 10.0;
            r
        };
        // Emotes, beside it.
        let em = place(56.0);
        if let Some(e) = super::emotes::button(ui, em, "craps-emotes", &mut self.poker.casino.emote_last) {
            self.engine.poker(&s.key, ToServer::GameEmote { game: TableGame::Craps, table: v.id, emote: e });
        }
        let betting = v.stage == CrapsStage::Betting;
        let space = super::keys::pressed(ui.ctx(), super::keys::Move::CrapsRoll);
        let clear_key = super::keys::pressed(ui.ctx(), super::keys::Move::CrapsClear);
        // (" · Space": the roll's key, as it is now.)
        let roll_key = super::keys::first_key(super::keys::Move::CrapsRoll).map(|k| format!(" · {k}")).unwrap_or_default();
        if shooter {
            // The shooter throws (once the bets are in), or the dice go by themselves at the end.
            let r = place(190.0);
            let sub = if betting { Some(format!("or they go in {secs}s{roll_key}")) } else { Some("the dice are coming back".into()) };
            if button(ui, r, "throw", "Throw the dice", sub, col::RED, betting) || (betting && space) {
                if let Some((_, d)) = self.poker.casino.crt.desired.clone() {
                    self.crt_send(s, v, d, true);
                }
                self.engine.poker(&s.key, ToServer::CrapsThrow { table: v.id });
            }
        } else {
            let r = place(170.0);
            let (label, sub) = if ready { ("Ready".to_string(), format!("dice in {secs}s")) } else { ("I'm ready".to_string(), if betting { format!("{secs}s to bet{roll_key}") } else { "next roll".into() }) };
            if (button(ui, r, "ready", &label, Some(sub), col::GOLD, betting && !ready) || (betting && !ready && space)) && betting {
                self.crt_send(s, v, on.clone(), true);
            }
        }
        // Buys are off on the come-out roll unless they're set working (0.13.23): the button
        // for it, while there's no point and I've a buy up.
        if v.point.is_none() && on.buy.iter().any(|b| *b > 0) {
            let r = place(150.0);
            let (label, sub) = if on.buys_working { ("Buys working", "on the come-out too") } else { ("Buys off", "on the come-out") };
            if button(ui, r, "buys-working", label, Some(sub.into()), Color32::from_white_alpha(30), betting) {
                let mut want = on.clone();
                want.buys_working = !want.buys_working;
                self.crt_send(s, v, want, false);
            }
        }
        let r = place(104.0);
        if button(ui, r, "clear", "Clear", None, Color32::from_white_alpha(30), on.total() > 0) || (clear_key && on.total() > 0) {
            // Down comes what may (the pass line stays once there's a point).
            let mut want = Bets::default();
            if v.point.is_some() {
                want.pass = mine.as_ref().map(|x| x.bets.pass).unwrap_or(0);
            }
            self.crt_send(s, v, want, false);
        }
        let chips = chips_now(s);
        let color = if on.total() > chips + mine.as_ref().map(|x| x.bets.total()).unwrap_or(0) { col::LOSE } else { col::GOLD2 };
        // The rack: the table's chips I can put down (0.13.24: a chip bigger than the chips I
        // had left could be picked, and every bet with it was refused), its smallest always.
        let (lo, hi) = super::table_chips(v.min_bet, v.max_bet);
        let chips_left = (chips + mine.as_ref().map(|x| x.bets.total()).unwrap_or(0)).saturating_sub(on.total());
        let hi = super::CHIPS.iter().copied().filter(|c| *c >= lo && *c <= chips_left).last().unwrap_or(lo).min(hi);
        self.rack_and_bet_from(ui, bar, x, (lo, hi), format!("On the layout {}", chips_text(on.total())), color);
        let shooter_name = v.shooter.and_then(|i| v.seats.get(i as usize).cloned().flatten()).map(|x| x.name);
        let hint = if on.total() == 0 {
            format!("Pick a chip, then click a bet on the layout ({} to {} each)", chips_text(v.min_bet), chips_text(v.max_bet))
        } else if !shooter && let Some(n) = shooter_name {
            format!("{n} has the dice")
        } else {
            String::new()
        };
        if !hint.is_empty() {
            p.text(pos2(bar.center().x, bar.top() - 12.0), Align2::CENTER_CENTER, hint, theme::font(12.5), col::MUTED);
        }
        if betting {
            let frac = (left as f32 / BET_MS as f32).clamp(0.0, 1.0);
            let line = Rect::from_min_size(bar.left_top() + vec2(18.0, 0.0), vec2((bar.width() - 36.0) * frac, 3.0));
            p.rect_filled(line, theme::r(2), if frac > 0.3 { col::GOLD } else { col::LOSE });
        }
        // My bets are on the table now (or it said no, or the roll's gone by).
        if let Some((n, d)) = &self.poker.casino.crt.desired
            && (*n != v.roll_no || mine.as_ref().is_some_and(|x| x.bets == *d) || s.poker.error.is_some())
        {
            self.poker.casino.crt.desired = None;
        }
        let _ = now;
    }
}
