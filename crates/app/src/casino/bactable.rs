//! Baccarat at a table of five (0.12.0 servers): people and bots around one shoe, a coup every
//! few seconds. The table is the Meshy art (`table-baccarat.webp`, burgundy felt in black leather),
//! with the layout drawn on its felt: the Player and Banker boxes under the dealer, the shoe, the
//! scoreboard of the coups so far, and round the rail each seat's own Player, Banker, Tie and pair
//! circles. The cards slide out of the shoe in dealing order and turn over a side at a time, the
//! third cards after; the winner's called and the chips go where they're owed. Every coup is
//! dealt again by this Pyr from its seeds (see `pyr_engine::poker::SharedLive`).

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::bacroom::{BET_MS, SEATS, SHOW_MS};
use pyr_poker::cards::Card;
use pyr_poker::casino::baccarat::{self as bc, Bets, Side};
use pyr_poker::net::{BacMark, BacReveal, BacSeatView, BacStage, BacView, TableGame, ToServer};

use super::{chip_stack, chips_now, col, ease_back, ease_out, glow_text, hgrad, radial, vgrad};
use crate::app::PyrApp;
use crate::poker::cards::{self as cards, chips_text};
use crate::theme;

/// The art's size, and the part of it that always shows (the table, rail to rail).
const ART: Vec2 = vec2(1376.0, 774.0);
const SHOW: Rect = Rect { min: pos2(150.0, 70.0), max: pos2(1226.0, 712.0) };
/// Each seat's betting circles' middle on the felt: seat 0 on the dealer's left (the right as
/// you look), round to seat 4.
const SEAT_AT: [(f32, f32); SEATS] = [(1100.0, 404.0), (944.0, 514.0), (688.0, 540.0), (432.0, 514.0), (276.0, 404.0)];
/// The Player and Banker boxes under the dealer, the shoe, the scoreboard.
const PLAYER_AT: (f32, f32) = (520.0, 252.0);
const BANKER_AT: (f32, f32) = (856.0, 252.0);
const SHOE_AT: (f32, f32) = (1066.0, 150.0);
const BOARD: Rect = Rect { min: pos2(508.0, 356.0), max: pos2(868.0, 470.0) };
/// The winner's called over the middle of the felt.
const CALL_AT: (f32, f32) = (688.0, 404.0);
const CARD: Vec2 = vec2(66.0, 92.0);
/// A card's slide out of the shoe.
const SLIDE: f32 = 0.34;
const BAR_H: f32 = 96.0;
const STRIP_H: f32 = 44.0;
const PLAYER_BLUE: Color32 = Color32::from_rgb(0x3d, 0x8b, 0xff);
const BANKER_RED: Color32 = Color32::from_rgb(0xff, 0x4a, 0x4a);
const TIE_GREEN: Color32 = Color32::from_rgb(0x3a, 0xd0, 0x7a);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Spot {
    Player,
    Banker,
    Tie,
    PlayerPair,
    BankerPair,
}

const SPOTS: [Spot; 5] = [Spot::Player, Spot::Banker, Spot::Tie, Spot::PlayerPair, Spot::BankerPair];

impl Spot {
    /// Where it is in a seat's cluster (art units from the cluster's middle), and its size.
    fn place(self) -> ((f32, f32), f32) {
        match self {
            Spot::Player => ((-37.0, 14.0), 28.0),
            Spot::Banker => ((37.0, 14.0), 28.0),
            Spot::Tie => ((0.0, -30.0), 18.0),
            Spot::PlayerPair => ((-46.0, -33.0), 13.5),
            Spot::BankerPair => ((46.0, -33.0), 13.5),
        }
    }

    fn color(self) -> Color32 {
        match self {
            Spot::Player | Spot::PlayerPair => PLAYER_BLUE,
            Spot::Banker | Spot::BankerPair => BANKER_RED,
            Spot::Tie => TIE_GREEN,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Spot::Player => "PLAYER",
            Spot::Banker => "BANKER",
            Spot::Tie => "TIE",
            Spot::PlayerPair => "P·P",
            Spot::BankerPair => "B·P",
        }
    }

    fn tip(self) -> &'static str {
        match self {
            Spot::Player => "Player: pays 1 to 1",
            Spot::Banker => "Banker: pays 19 to 20 (a 5% commission)",
            Spot::Tie => "Tie: pays 8 to 1 (Player and Banker bets come back)",
            Spot::PlayerPair => "Player pair: the Player's first two cards a pair, pays 11 to 1",
            Spot::BankerPair => "Banker pair: the Banker's first two cards a pair, pays 11 to 1",
        }
    }

    fn of(self, b: &Bets) -> u64 {
        match self {
            Spot::Player => b.player,
            Spot::Banker => b.banker,
            Spot::Tie => b.tie,
            Spot::PlayerPair => b.player_pair,
            Spot::BankerPair => b.banker_pair,
        }
    }

    fn slot(self, b: &mut Bets) -> &mut u64 {
        match self {
            Spot::Player => &mut b.player,
            Spot::Banker => &mut b.banker,
            Spot::Tie => &mut b.tie,
            Spot::PlayerPair => &mut b.player_pair,
            Spot::BankerPair => &mut b.banker_pair,
        }
    }

    /// What it paid back on a coup (its stake with its win, or the stake back on a push).
    fn paid(self, b: &Bets, r: &BacReveal) -> u64 {
        let stake = self.of(b);
        match self {
            Spot::Player => match r.winner {
                Side::Player => stake * 2,
                Side::Tie => stake,
                Side::Banker => 0,
            },
            Spot::Banker => match r.winner {
                Side::Banker => stake + stake * 95 / 100,
                Side::Tie => stake,
                Side::Player => 0,
            },
            Spot::Tie => if r.winner == Side::Tie { stake * 9 } else { 0 },
            Spot::PlayerPair => if r.player_pair { stake * 12 } else { 0 },
            Spot::BankerPair => if r.banker_pair { stake * 12 } else { 0 },
        }
    }
}

#[derive(Default)]
pub struct BacTableUi {
    /// The table I'm at (watching, or sitting): its server and id.
    pub at: Option<(String, u64)>,
    asked: Option<Instant>,
    watch_asked: Option<Instant>,
    /// My bets as I put them down, for coup `.0` (shown till the table has them).
    desired: Option<(u64, Bets)>,
    /// The coup on the table and when it was dealt (everything about it times from then).
    dealt: Option<(u64, Instant)>,
    /// My chips while my bets are in (the balance waits for the coup's result), and the last coup
    /// whose result was called for me.
    playing_chips: Option<u64>,
    told: Option<u64>,
    /// My last coup's bets (for Rebet).
    rebet: Bets,
}

/// When each card of a coup comes out and turns over (seconds after the deal), and when the
/// result's called: the four cards out in turn, the Player's turned over, then the Banker's,
/// then the third cards one at a time.
struct Times {
    player: Vec<(f32, f32)>,
    banker: Vec<(f32, f32)>,
    result: f32,
}

fn times(r: &BacReveal) -> Times {
    let mut player = vec![(0.25, 2.2), (1.05, 2.2)];
    let mut banker = vec![(0.65, 3.0), (1.45, 3.0)];
    let mut t = 3.0;
    if r.player.len() > 2 {
        player.push((t + 0.8, t + 1.25));
        t += 1.25;
    }
    if r.banker.len() > 2 {
        banker.push((t + 0.8, t + 1.25));
        t += 1.25;
    }
    Times { player, banker, result: t + 0.7 }
}

/// Where the art is on the screen.
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

/// A card sliding out of the shoe to `at`, face down, turning over at `flip` (seconds since the
/// deal, as `since` is).
fn card(p: &egui::Painter, shoe: Pos2, at: Pos2, size: Vec2, face: Card, appear: f32, flip: f32, since: f32) {
    if since < appear {
        return;
    }
    let k = ((since - appear) / SLIDE).min(1.0);
    let c = shoe + (at - shoe) * ease_out(k);
    p.rect_filled(Rect::from_center_size(c + vec2(2.0, 4.0), size), theme::r(6), Color32::from_black_alpha((80.0 * k) as u8));
    let turn = ((since - flip) / 0.32).clamp(0.0, 1.0);
    if turn > 0.5 {
        let w = size.x * (turn - 0.5) * 2.0;
        cards::face(p, Rect::from_center_size(c, vec2(w.max(2.0), size.y)), face, false, false);
    } else {
        let w = size.x * (1.0 - turn * 2.0).max(0.04);
        cards::back(p, Rect::from_center_size(c, vec2(w, size.y)), "");
    }
}

/// A button on the controls; true when it's clicked.
fn button(ui: &mut egui::Ui, r: Rect, id: &str, text: &str, sub: Option<String>, fill: Color32, enabled: bool) -> bool {
    let resp = ui.interact(r, ui.id().with(("bac", id)), Sense::click());
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

/// The winner, called: "BANKER WINS", "PLAYER WINS", "TIE".
fn call(r: &BacReveal) -> (String, Color32) {
    let (p, b) = (bc::total(&r.player), bc::total(&r.banker));
    let natural = r.player.len() == 2 && r.banker.len() == 2 && (p >= 8 || b >= 8);
    match r.winner {
        Side::Player => (format!("PLAYER WINS {p} TO {b}{}", if natural { " · NATURAL" } else { "" }), PLAYER_BLUE),
        Side::Banker => (format!("BANKER WINS {b} TO {p}{}", if natural { " · NATURAL" } else { "" }), BANKER_RED),
        Side::Tie => (format!("TIE · {p} ALL"), TIE_GREEN),
    }
}

impl PyrApp {
    /// Leaves the baccarat table I'm at (standing up first: my bets come back).
    pub fn bac_table_leave(&mut self) {
        if let Some((key, id)) = self.poker.casino.bact.at.take() {
            self.engine.poker(&key, ToServer::BacLeave { table: id });
            self.engine.poker(&key, ToServer::BacUnwatch { table: id });
            self.engine.bac_close(&key);
        }
        let t = &mut self.poker.casino.bact;
        t.desired = None;
        t.dealt = None;
        t.playing_chips = None;
    }

    /// The baccarat tables' page: the tables along the top, the table, the controls.
    pub(super) fn bac_table_page(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let now = Instant::now();
        let key = s.key.clone();
        if self.poker.casino.bact.asked.is_none_or(|t| t.elapsed() > Duration::from_secs(8)) {
            self.poker.casino.bact.asked = Some(now);
            self.engine.poker(&key, ToServer::BacTables);
        }
        let tables = s.poker.bac_tables.clone().unwrap_or_default();
        let mine = self.poker.casino.bact.at.clone().filter(|(k, _)| *k == key).map(|(_, id)| id);
        let at = match mine {
            Some(id) if tables.is_empty() || tables.iter().any(|t| t.id == id) => Some(id),
            _ => tables.first().map(|t| t.id),
        };
        let live = s.poker.bac.clone().filter(|b| Some(b.view.id) == at);
        if let Some(id) = at {
            let moved = self.poker.casino.bact.at != Some((key.clone(), id));
            if moved || (live.is_none() && self.poker.casino.bact.watch_asked.is_none_or(|t| t.elapsed() > Duration::from_secs(3))) {
                self.poker.casino.bact.at = Some((key.clone(), id));
                self.poker.casino.bact.watch_asked = Some(now);
                self.engine.poker(&key, ToServer::BacWatch { table: id });
            }
        }
        let p = ui.painter().clone();
        let time = ui.ctx().input(|i| i.time) as f32;
        p.rect_filled(area, 0, Color32::from_rgb(3, 2, 3));
        let strip = Rect::from_min_size(area.min, vec2(area.width(), STRIP_H));
        let bar = Rect::from_min_max(pos2(area.left() + 16.0, area.bottom() - BAR_H + 6.0), pos2(area.right() - 16.0, area.bottom() - 10.0));
        let room = Rect::from_min_max(pos2(area.left(), strip.bottom()), pos2(area.right(), bar.top() - 4.0));
        let fit = Fit::new(room);
        let k = fit.k;
        // The table, its dark edges running off into the room's, a warm light over the felt.
        let art = Rect::from_min_size(fit.origin, ART * k);
        let clip = p.with_clip_rect(room.union(strip));
        super::pics::part(&clip, ui.ctx(), "table-baccarat", Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), art, art.width() as u32, Color32::WHITE);
        let dark = Color32::from_rgb(3, 2, 3);
        let clear = Color32::from_rgba_unmultiplied(3, 2, 3, 0);
        let fade = 90.0 * k;
        hgrad(&clip, Rect::from_min_max(art.left_top(), pos2(art.left() + fade, art.bottom())), dark, clear);
        hgrad(&clip, Rect::from_min_max(pos2(art.right() - fade, art.top()), art.right_bottom()), clear, dark);
        vgrad(&clip, Rect::from_min_max(pos2(art.left(), art.bottom() - fade), art.right_bottom()), clear, dark);
        radial(&clip, fit.at((688.0, 300.0)), 470.0 * k, Color32::from_rgba_unmultiplied(0xff, 0xc8, 0x8a, 22), Color32::TRANSPARENT);
        self.bac_layout(&p, fit, time);
        self.bac_tables_strip(ui, s, strip, &tables, at);
        let Some(live) = live else {
            p.text(fit.at((688.0, 420.0)), Align2::CENTER_CENTER, if s.poker.bac_tables.is_none() { "Setting out the tables…" } else { "Sitting you at the table…" }, theme::bold_font(16.0), col::MUTED);
            return;
        };
        let v = &live.view;
        self.bac_board(&p, fit, &v.history);
        // The coup on the table (once dealt, until the next is), on its own clock.
        let shown = matches!(v.stage, BacStage::Shown).then(|| v.last.clone()).flatten();
        if let Some(r) = &shown
            && self.poker.casino.bact.dealt.is_none_or(|(c, _)| c != r.coup)
        {
            // (Joined late: part way through already.)
            let ms_left = v.ms_left.saturating_sub(live.at.elapsed().as_millis() as u64);
            let since = Duration::from_millis(SHOW_MS.saturating_sub(ms_left));
            let dealt = now.checked_sub(since).unwrap_or(now);
            self.poker.casino.bact.dealt = Some((r.coup, dealt));
            self.bac_sounds(r, dealt, now);
            self.bac_result_for_me(v, r, dealt, now);
        }
        let since = self.poker.casino.bact.dealt.filter(|(c, _)| shown.as_ref().is_some_and(|r| r.coup == *c)).map(|(_, d)| now.duration_since(d).as_secs_f32());
        // My chips while my bets are in.
        let mine_in = v.my_seat.and_then(|m| v.seats.get(m as usize).cloned().flatten()).is_some_and(|x| x.bets.total() > 0);
        if mine_in && v.stage == BacStage::Betting {
            self.poker.casino.bact.playing_chips = Some(chips_now(s));
        }
        // The cards.
        let shoe = fit.at(SHOE_AT);
        let size = CARD * k;
        if let (Some(r), Some(since)) = (&shown, since) {
            let t = times(r);
            for (side, cards_, at, ts) in [(Side::Player, &r.player, PLAYER_AT, &t.player), (Side::Banker, &r.banker, BANKER_AT, &t.banker)] {
                let c = fit.at(at);
                let mut seen = Vec::new();
                for (i, cd) in cards_.iter().enumerate() {
                    let (appear, flip) = ts[i];
                    // The two side by side; the third out on the outside, a little lower.
                    let off = match (i, side) {
                        (0, _) => vec2(-38.0, 0.0),
                        (1, _) => vec2(38.0, 0.0),
                        (_, Side::Player) => vec2(-108.0, 8.0),
                        _ => vec2(108.0, 8.0),
                    };
                    card(&p, shoe, c + off * k, size, *cd, appear, flip, since);
                    if since >= flip + 0.2 {
                        seen.push(*cd);
                    }
                }
                // The total so far, under the cards.
                if !seen.is_empty() {
                    let total = bc::total(&seen);
                    let won = since >= t.result && (r.winner == side || r.winner == Side::Tie);
                    let tc = fit.at((at.0, at.1 + 82.0));
                    let rr = 19.0 * k.max(0.7);
                    let color = if side == Side::Player { PLAYER_BLUE } else { BANKER_RED };
                    if won {
                        radial(&p, tc, rr * 2.4, color.gamma_multiply(0.45 + 0.15 * (time * 4.0).sin()), Color32::TRANSPARENT);
                    }
                    p.circle_filled(tc, rr, Color32::from_rgb(0x14, 0x08, 0x0a));
                    p.circle_stroke(tc, rr, Stroke::new(2.5f32, color));
                    p.text(tc, Align2::CENTER_CENTER, total.to_string(), theme::bold_font(rr * 1.15), Color32::WHITE);
                }
            }
            // The winner, called; the winning box lit.
            if since >= t.result {
                let a = ((since - t.result) / 0.3).clamp(0.0, 1.0);
                let (text, color) = call(r);
                let pop = ease_back(a);
                let at = fit.at(CALL_AT);
                let f = (30.0 * k).clamp(16.0, 34.0) * (0.6 + 0.4 * pop);
                let g = p.layout_no_wrap(text.clone(), theme::bold_font(f), color);
                let plate = Rect::from_center_size(at, g.size() + vec2(40.0, 16.0) * k.max(0.7));
                p.rect_filled(plate, theme::r(16), Color32::from_black_alpha((215.0 * a) as u8));
                p.rect_stroke(plate, theme::r(16), Stroke::new(1.5f32, color.gamma_multiply(a)), StrokeKind::Inside);
                glow_text(&p, at, Align2::CENTER_CENTER, &text, theme::bold_font(f), Color32::WHITE.gamma_multiply(a), color.gamma_multiply(0.7 * a));
                let pairs: Vec<&str> = [(r.player_pair, "PLAYER PAIR"), (r.banker_pair, "BANKER PAIR")].iter().filter(|x| x.0).map(|x| x.1).collect();
                if !pairs.is_empty() {
                    p.text(plate.center_bottom() + vec2(0.0, 13.0 * k.max(0.8)), Align2::CENTER_CENTER, pairs.join("  ·  "), theme::bold_font((13.0 * k).max(10.0)), col::GOLD2.gamma_multiply(a));
                }
            }
        }
        // The seats.
        for i in 0..SEATS {
            let seat = v.seats.get(i).cloned().flatten();
            self.bac_seat(ui, s, v, i as u8, seat.as_ref(), shown.as_ref(), since, fit, time);
        }
        self.bac_controls(ui, s, &live, bar, now);
        // Emotes, big over the circles of whoever sent them.
        let size = (112.0 * fit.k).clamp(84.0, 132.0);
        super::emotes::pops(&p, ui.ctx(), &s.poker.game_emotes, TableGame::Baccarat, v.id, size, |seat| fit.at(SEAT_AT[(seat as usize).min(SEATS - 1)]) - vec2(0.0, 8.0 * fit.k));
    }

    /// The felt's markings: the Player and Banker boxes, the shoe, the rules, the scoreboard.
    fn bac_layout(&self, p: &egui::Painter, fit: Fit, time: f32) {
        let k = fit.k;
        for (at, name, color) in [(PLAYER_AT, "PLAYER", PLAYER_BLUE), (BANKER_AT, "BANKER", BANKER_RED)] {
            let c = fit.at(at);
            let r = Rect::from_center_size(c + vec2(0.0, 6.0) * k, vec2(290.0, 132.0) * k);
            p.rect_filled(r, theme::r(18), Color32::from_black_alpha(46));
            p.rect_stroke(r, theme::r(18), Stroke::new(1.5f32, col::GOLD.gamma_multiply(0.55)), StrokeKind::Inside);
            p.rect_stroke(r.shrink(5.0 * k), theme::r(14), Stroke::new(1.0f32, color.gamma_multiply(0.35)), StrokeKind::Inside);
            glow_text(p, c - vec2(0.0, 84.0 * k), Align2::CENTER_CENTER, name, theme::bold_font((22.0 * k).max(11.0)), color, color.gamma_multiply(0.35 + 0.1 * (time * 1.3).sin()));
        }
        p.text(fit.at((688.0, 168.0)), Align2::CENTER_CENTER, "PUNTO BANCO", theme::bold_font((13.0 * k).max(9.0)), col::GOLD.gamma_multiply(0.5));
        // The shoe.
        let shoe = fit.at(SHOE_AT);
        let shoe_r = Rect::from_center_size(shoe, vec2(92.0, 62.0) * k);
        p.rect_filled(shoe_r.translate(vec2(0.0, 5.0 * k)), theme::r(8), Color32::from_black_alpha(120));
        vgrad(p, shoe_r, Color32::from_rgb(0x2a, 0x10, 0x14), Color32::from_rgb(0x10, 0x04, 0x06));
        p.rect_stroke(shoe_r, theme::r(8), Stroke::new(1.0f32, col::GOLD_DARK), StrokeKind::Inside);
        cards::back(p, Rect::from_center_size(shoe + vec2(-10.0, -3.0) * k, vec2(42.0, 58.0) * k), "");
    }

    /// The scoreboard: every coup so far, a bead each (red Banker, blue Player, green Tie, its
    /// winning total on it, a dot for each pair), and the count of each.
    fn bac_board(&self, p: &egui::Painter, fit: Fit, history: &[BacMark]) {
        let k = fit.k;
        let r = Rect::from_min_max(fit.at((BOARD.min.x, BOARD.min.y)), fit.at((BOARD.max.x, BOARD.max.y)));
        p.rect_filled(r, theme::r(12), Color32::from_black_alpha(110));
        p.rect_stroke(r, theme::r(12), Stroke::new(1.0f32, col::GOLD.gamma_multiply(0.45)), StrokeKind::Inside);
        let (rows, cols) = (5usize, 14usize);
        let head = 22.0 * k;
        let grid = Rect::from_min_max(r.min + vec2(8.0 * k, head), r.max - vec2(8.0 * k, 6.0 * k));
        let cell = (grid.width() / cols as f32).min(grid.height() / rows as f32);
        let count = |s: Side| history.iter().filter(|m| m.winner == s).count();
        let words = [(format!("BANKER {}", count(Side::Banker)), BANKER_RED), (format!("PLAYER {}", count(Side::Player)), PLAYER_BLUE), (format!("TIE {}", count(Side::Tie)), TIE_GREEN)];
        let f = (11.0 * k).max(8.5);
        let mut x = r.left() + 12.0 * k;
        for (w, c) in words {
            let g = p.layout_no_wrap(w, theme::bold_font(f), c);
            let wd = g.size().x;
            p.galley(pos2(x, r.top() + head / 2.0 - g.size().y / 2.0), g, c);
            x += wd + 14.0 * k;
        }
        p.text(pos2(r.right() - 10.0 * k, r.top() + head / 2.0), Align2::RIGHT_CENTER, format!("{} coups", history.len()), theme::font(f), Color32::from_white_alpha(110));
        // The last ones that fit, column by column.
        let fits = rows * cols;
        let from = history.len().saturating_sub(fits).div_ceil(rows) * rows;
        let from = from.min(history.len());
        for (n, m) in history[from..].iter().enumerate() {
            let (cx, cy) = (n / rows, n % rows);
            let c = pos2(grid.left() + (cx as f32 + 0.5) * cell, grid.top() + (cy as f32 + 0.5) * cell);
            let color = match m.winner {
                Side::Banker => BANKER_RED,
                Side::Player => PLAYER_BLUE,
                Side::Tie => TIE_GREEN,
            };
            let rr = cell * 0.42;
            p.circle_filled(c, rr, color.gamma_multiply(0.9));
            if m.natural {
                p.circle_stroke(c, rr + 1.5, Stroke::new(1.5f32, col::GOLD2));
            }
            p.text(c, Align2::CENTER_CENTER, m.total.to_string(), theme::bold_font(rr * 1.1), Color32::WHITE);
            if m.banker_pair {
                p.circle_filled(c + vec2(-rr * 0.7, -rr * 0.7), rr * 0.28, BANKER_RED);
                p.circle_stroke(c + vec2(-rr * 0.7, -rr * 0.7), rr * 0.28, Stroke::new(1.0f32, Color32::WHITE));
            }
            if m.player_pair {
                p.circle_filled(c + vec2(rr * 0.7, rr * 0.7), rr * 0.28, PLAYER_BLUE);
                p.circle_stroke(c + vec2(rr * 0.7, rr * 0.7), rr * 0.28, Stroke::new(1.0f32, Color32::WHITE));
            }
        }
        if history.is_empty() {
            p.text(grid.center(), Align2::CENTER_CENTER, "A new shoe: the coups go here", theme::font(f), Color32::from_white_alpha(90));
        }
    }

    /// The sounds of a coup being dealt: each card out, each side turning over, the result.
    fn bac_sounds(&mut self, r: &BacReveal, dealt: Instant, now: Instant) {
        let since = now.duration_since(dealt).as_secs_f32();
        let t = times(r);
        let mut at = |s: f32, cue: Cue, vol: f32| {
            if s >= since {
                self.poker.casino.sound_in(s - since, cue, 0.0, vol);
            }
        };
        for (appear, _) in t.player.iter().chain(t.banker.iter()) {
            at(*appear, Cue::Deal, 0.55);
        }
        at(2.2, Cue::Flip, 0.7);
        at(3.0, Cue::Flip, 0.7);
        if let Some((_, f)) = t.player.get(2) {
            at(*f, Cue::Flip, 0.7);
        }
        if let Some((_, f)) = t.banker.get(2) {
            at(*f, Cue::Flip, 0.7);
        }
    }

    /// The result of a coup for me, called out big as it's shown, and the balance counting up.
    fn bac_result_for_me(&mut self, v: &BacView, r: &BacReveal, dealt: Instant, now: Instant) {
        if self.poker.casino.bact.told == Some(r.coup) {
            return;
        }
        self.poker.casino.bact.told = Some(r.coup);
        let Some(me) = v.my_seat.and_then(|m| v.seats.get(m as usize).cloned().flatten()) else { return };
        let bets = me.last_bets.clone();
        if bets.total() == 0 {
            return;
        }
        self.poker.casino.bact.rebet = bets.clone();
        let at = dealt + Duration::from_secs_f32(times(r).result);
        let back = at.saturating_duration_since(now).as_secs_f32();
        if let Some(chips) = self.poker.casino.bact.playing_chips.take() {
            self.poker.casino.hold = Some((chips, at));
        }
        let wagered = bets.total();
        let paid = me.paid;
        let (title, color) = if paid > wagered {
            ("YOU WIN", col::WIN)
        } else if paid == wagered {
            ("PUSH", Color32::from_rgb(0xd8, 0xd8, 0xe0))
        } else {
            match r.winner {
                Side::Banker => ("BANKER WINS", BANKER_RED),
                Side::Player => ("PLAYER WINS", PLAYER_BLUE),
                Side::Tie => ("TIE", TIE_GREEN),
            }
        };
        let sub = if paid > wagered { format!("+{}", chips_text(paid - wagered)) } else { String::new() };
        self.poker.casino.announce(title, sub, color, (0.5, 0.26), back + 0.25);
        if paid > wagered {
            self.poker.casino.sound_in(back + 0.2, Cue::WinSmall, 0.0, 0.8);
            self.poker.casino.sound_in(back + 0.45, Cue::Chips, 0.0, 0.7);
        }
        self.poker.casino.celebrate(paid, wagered, false, back + 0.25);
    }

    /// The tables, along the top: each one's name, its bets and who's at it.
    fn bac_tables_strip(&mut self, ui: &mut egui::Ui, s: &ServerView, strip: Rect, tables: &[pyr_poker::net::BacSummary], at: Option<u64>) {
        let p = ui.painter().clone();
        vgrad(&p, strip, Color32::from_rgb(0x16, 0x0c, 0x10), Color32::from_rgb(0x08, 0x05, 0x07));
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
            let resp = ui.interact(r, ui.id().with(("bac-table", t.id)), Sense::click());
            p.rect_filled(r, theme::r(15), if on { col::GOLD.gamma_multiply(0.22) } else if resp.hovered() { Color32::from_white_alpha(22) } else { Color32::from_white_alpha(10) });
            p.rect_stroke(r, theme::r(15), Stroke::new(1.0f32, if on { col::GOLD } else { Color32::from_white_alpha(30) }), StrokeKind::Inside);
            p.text(r.left_center() + vec2(14.0, 0.0), Align2::LEFT_CENTER, text, theme::bold_font(13.0), if on { col::GOLD2 } else { col::TEXT });
            p.text(r.right_center() - vec2(12.0, 0.0), Align2::RIGHT_CENTER, who, theme::font(12.0), col::MUTED);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                self.bac_table_leave();
                self.poker.casino.bact.at = Some((s.key.clone(), t.id));
                self.engine.poker(&s.key, ToServer::BacWatch { table: t.id });
                self.poker.casino.bact.watch_asked = Some(Instant::now());
            }
        }
    }

    /// One seat: its circles (and the bets in them), who's in it, what the coup did for them.
    #[allow(clippy::too_many_arguments)]
    fn bac_seat(&mut self, ui: &mut egui::Ui, s: &ServerView, v: &BacView, i: u8, seat: Option<&BacSeatView>, shown: Option<&BacReveal>, since: Option<f32>, fit: Fit, time: f32) {
        let p = ui.painter().clone();
        let k = fit.k;
        let c = fit.at(SEAT_AT[i as usize]);
        let me = v.my_seat == Some(i);
        let betting = matches!(v.stage, BacStage::Waiting | BacStage::Betting) || (v.stage == BacStage::Shown && shown.is_some_and(|r| since.is_some_and(|t| t >= times(r).result + 0.9)));
        if me {
            radial(&p, c, 120.0 * k, col::GOLD.gamma_multiply(0.10 + 0.03 * (time * 2.0).sin()), Color32::TRANSPARENT);
        }
        // What's on each circle: the coup being turned over (its bets, then what came of them),
        // or the bets for the next one (mine as I put them down, till the table has them).
        let result_t = shown.zip(since).map(|(r, t)| (r, t, times(r).result));
        let revealing = result_t.is_some_and(|(_, t, res)| t < res + 1.4);
        let desired = self.poker.casino.bact.desired.clone().filter(|(coup, _)| *coup == v.coup_no && me).map(|(_, b)| b);
        let next = desired.clone().or_else(|| seat.map(|x| x.bets.clone())).unwrap_or_default();
        let last = seat.map(|x| x.last_bets.clone()).unwrap_or_default();
        for spot in SPOTS {
            let ((dx, dy), rad) = spot.place();
            let sc = c + vec2(dx, dy) * k;
            let sr = rad * k;
            let color = spot.color();
            // The circle: a ring in its color, its name inside.
            p.circle_filled(sc, sr, Color32::from_rgba_unmultiplied(0x0c, 0x02, 0x05, 110));
            p.circle_stroke(sc, sr, Stroke::new(if spot == Spot::Player || spot == Spot::Banker { 2.0f32 } else { 1.4 }, color.gamma_multiply(0.85)));
            p.circle_stroke(sc, sr - 3.0 * k, Stroke::new(0.8f32, col::GOLD.gamma_multiply(0.35)));
            let hit = ui.interact(Rect::from_center_size(sc, vec2(sr, sr) * 2.0), ui.id().with(("bac-spot", i, spot as u8)), Sense::click());
            if me && hit.hovered() {
                p.circle_filled(sc, sr, Color32::from_white_alpha(22));
            }
            let hit = hit.on_hover_text(spot.tip());
            let chips_r = (sr * if rad > 20.0 { 0.62 } else { 0.8 }).max(7.0);
            let label_f = (if rad > 20.0 { 9.5 } else { 8.0 } * k).max(7.0);
            match (revealing, result_t) {
                (true, Some((r, t, res))) => {
                    let stake = spot.of(&last);
                    let paid = spot.paid(&last, r);
                    if stake == 0 {
                        p.text(sc, Align2::CENTER_CENTER, spot.label(), theme::bold_font(label_f), color.gamma_multiply(0.75));
                    } else if t < res {
                        chip_stack(&p, sc + vec2(0.0, sr * 0.2), chips_r, stake);
                    } else {
                        let u = ((t - res) / 0.6).clamp(0.0, 1.0);
                        if paid == 0 {
                            // Lost: swept to the dealer.
                            let to = fit.at((688.0, 120.0));
                            let q = sc + (to - sc) * ease_out(u);
                            let mut faded = p.clone();
                            faded.set_opacity(1.0 - u);
                            chip_stack(&faded, q, chips_r, stake);
                        } else {
                            // Won (or back on a push): the stake, the win beside it, a glow.
                            let won = paid > stake;
                            if won {
                                radial(&p, sc, sr * 2.2, col::GOLD.gamma_multiply(0.5 * (1.0 - u * 0.4)), Color32::TRANSPARENT);
                            }
                            chip_stack(&p, sc + vec2(-sr * 0.25, sr * 0.2), chips_r, stake);
                            if won {
                                let from = fit.at((688.0, 120.0));
                                let to = sc + vec2(sr * 0.45, sr * 0.2);
                                chip_stack(&p, from + (to - from) * ease_out(u), chips_r, paid - stake);
                                let lift = 10.0 * k * ease_out(u);
                                glow_text(&p, sc - vec2(0.0, sr + 14.0 * k + lift), Align2::CENTER_CENTER, &format!("+{}", chips_text(paid - stake)), theme::bold_font((13.0 * k).max(10.0)), col::GOLD2, Color32::from_black_alpha(160));
                            } else {
                                p.text(sc - vec2(0.0, sr + 10.0 * k), Align2::CENTER_CENTER, "PUSH", theme::bold_font((10.0 * k).max(8.0)), Color32::from_white_alpha(200));
                            }
                        }
                    }
                }
                _ => {
                    let amount = spot.of(&next);
                    if amount > 0 {
                        let mine_unsent = me && desired.as_ref().is_some_and(|d| seat.is_none_or(|x| x.bets != *d));
                        if mine_unsent {
                            let mut ghost = p.clone();
                            ghost.set_opacity(0.65);
                            chip_stack(&ghost, sc + vec2(0.0, sr * 0.2), chips_r, amount);
                        } else {
                            chip_stack(&p, sc + vec2(0.0, sr * 0.2), chips_r, amount);
                        }
                    } else {
                        p.text(sc, Align2::CENTER_CENTER, spot.label(), theme::bold_font(label_f), color.gamma_multiply(0.75));
                    }
                }
            }
            // My circles take the chip picked (right-click clears one).
            if me && betting && seat.is_some() {
                let mut want = next.clone();
                if hit.clicked() {
                    *spot.slot(&mut want) += self.poker.casino.chip;
                }
                if hit.secondary_clicked() {
                    *spot.slot(&mut want) = 0;
                }
                if hit.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if want != next {
                    self.bac_send(s, v, want, false);
                }
            }
        }
        // Who's sitting: a plate under the circles (or an open seat).
        let plate_c = c + vec2(0.0, 62.0 * k);
        let sitting = v.my_seat.is_some();
        match seat {
            Some(x) => {
                let name = if x.name.chars().count() > 12 { format!("{}…", x.name.chars().take(11).collect::<String>()) } else { x.name.clone() };
                let f = (12.5 * k).clamp(10.5, 14.0);
                let w = f * 2.6 + name.chars().count() as f32 * f * 0.6 + if x.bot { f * 2.6 } else { 0.0 };
                let plate = Rect::from_center_size(plate_c, vec2(w, f * 2.0));
                p.rect_filled(plate, theme::r((f as u8).max(6)), Color32::from_black_alpha(if me { 230 } else { 200 }));
                let ready = x.ready && v.stage == BacStage::Betting;
                p.rect_stroke(plate, theme::r((f as u8).max(6)), Stroke::new(1.0f32, if me { col::GOLD } else if ready { col::WIN.gamma_multiply(0.7) } else { Color32::from_white_alpha(36) }), StrokeKind::Inside);
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
                    let bot_resp = ui.interact(plate, ui.id().with(("bac-bot", i)), Sense::click()).on_hover_text(format!("{}: a bot that bets {}. It plays with the house's chips.", x.name, x.style));
                    bot_resp.context_menu(|ui| {
                        crate::menu::style(ui);
                        if ui.button(format!("Ask {} to leave", x.name)).clicked() {
                            self.engine.poker(&s.key, ToServer::BacBot { table: v.id, seat: i, add: false });
                            ui.close();
                        }
                    });
                }
                if ready {
                    p.text(plate.center_bottom() + vec2(0.0, 10.0 * k.max(0.8)), Align2::CENTER_CENTER, "READY", theme::bold_font((9.5 * k).max(8.0)), col::WIN.gamma_multiply(0.85));
                }
            }
            None => {
                let f = (11.0 * k).clamp(9.5, 12.5);
                if !sitting {
                    let sr = Rect::from_center_size(plate_c - vec2(f * 3.3, 0.0), vec2(f * 5.2, f * 1.9));
                    let resp = ui.interact(sr, ui.id().with(("bac-sit", i)), Sense::click());
                    p.rect_filled(sr, theme::r((f as u8).max(6)), if resp.hovered() { col::GOLD } else { Color32::from_black_alpha(170) });
                    p.text(sr.center(), Align2::CENTER_CENTER, "SIT", theme::bold_font(f * 0.95), if resp.hovered() { Color32::BLACK } else { col::GOLD });
                    if resp.on_hover_text("Sit here").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.poker.casino.bact.desired = None;
                        self.engine.poker(&s.key, ToServer::BacSit { table: v.id, seat: i });
                    }
                }
                let br = Rect::from_center_size(plate_c + vec2(if sitting { 0.0 } else { f * 3.3 }, 0.0), vec2(f * 5.4, f * 1.9));
                let bresp = ui.interact(br, ui.id().with(("bac-addbot", i)), Sense::click());
                p.rect_filled(br, theme::r((f as u8).max(6)), if bresp.hovered() { Color32::from_rgb(0x3d, 0x5f, 0xc4) } else { Color32::from_black_alpha(150) });
                p.text(br.center(), Align2::CENTER_CENTER, "+ BOT", theme::bold_font(f * 0.9), if bresp.hovered() { Color32::WHITE } else { Color32::from_white_alpha(140) });
                if bresp.on_hover_text("Seat a bot here: each bets its own way, with the house's chips").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.engine.poker(&s.key, ToServer::BacBot { table: v.id, seat: i, add: true });
                }
            }
        }
    }

    /// My bets to the table (shown at once, the table's own once it has them).
    fn bac_send(&mut self, s: &ServerView, v: &BacView, bets: Bets, ready: bool) {
        self.poker.casino.bact.desired = Some((v.coup_no, bets.clone()));
        self.poker.casino.sound_in(0.0, Cue::Chips, 0.0, 0.45);
        self.engine.bac_bets(&s.key, v.id, v.coup_no, bets, ready);
    }

    /// The controls along the bottom: the rack, Rebet and Clear, Deal (I'm ready), and the
    /// clock; or, watching, a seat to sit in.
    fn bac_controls(&mut self, ui: &mut egui::Ui, s: &ServerView, live: &pyr_engine::poker::SharedLive<BacView>, bar: Rect, now: Instant) {
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
                    Some(_) => format!("{}: bets {} to {}. Sit in an open seat to play.", v.name, chips_text(v.min_bet), chips_text(v.max_bet)),
                    None => format!("{} is full: watch, or try another table.", v.name),
                },
                col::MUTED,
            );
            if let Some(seat) = free {
                let r = Rect::from_min_size(pos2(bar.right() - 176.0, bar.center().y - 24.0), vec2(160.0, 48.0));
                if button(ui, r, "sit", "Sit down", None, col::GOLD, true) {
                    self.poker.casino.bact.desired = None;
                    self.engine.poker(&s.key, ToServer::BacSit { table: v.id, seat: seat as u8 });
                }
            }
            return;
        };
        let mine = v.seats.get(me as usize).cloned().flatten();
        let on = self.poker.casino.bact.desired.clone().filter(|(c, _)| *c == v.coup_no).map(|(_, b)| b).or_else(|| mine.as_ref().map(|x| x.bets.clone())).unwrap_or_default();
        let ready = mine.as_ref().is_some_and(|x| x.ready);
        // Stand up, at the far right always.
        let up = Rect::from_min_size(pos2(bar.right() - 126.0, bar.center().y - 22.0), vec2(112.0, 44.0));
        if button(ui, up, "stand", "Stand up", None, Color32::from_white_alpha(30), true) {
            self.engine.poker(&s.key, ToServer::BacLeave { table: v.id });
            self.poker.casino.bact.desired = None;
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
        if let Some(e) = super::emotes::button(ui, em, "bac-emotes", &mut self.poker.casino.emote_last) {
            self.engine.poker(&s.key, ToServer::GameEmote { game: TableGame::Baccarat, table: v.id, emote: e });
        }
        let chips = chips_now(s);
        let seen = v.last.as_ref().zip(self.poker.casino.bact.dealt).is_some_and(|(r, (c, d))| r.coup == c && now.duration_since(d).as_secs_f32() >= times(r).result + 0.9);
        let open = matches!(v.stage, BacStage::Waiting | BacStage::Betting) || (v.stage == BacStage::Shown && seen);
        let enter = ui.ctx().input(|i| i.key_pressed(egui::Key::Enter)) && !ui.ctx().wants_keyboard_input();
        let (label, sub) = match v.stage {
            BacStage::Betting if ready => ("Ready".to_string(), Some(format!("cards in {secs}s"))),
            BacStage::Betting => (format!("Deal  ({})", chips_text(on.total())), Some(format!("{secs}s to bet · Enter"))),
            BacStage::Shown => ("Deal".to_string(), Some("next coup".to_string())),
            BacStage::Waiting => ("Deal".to_string(), Some("starts the coup".to_string())),
        };
        let r = place(180.0);
        let can_deal = on.total() > 0 && !ready && v.stage != BacStage::Shown;
        if button(ui, r, "deal", &label, sub, col::GOLD, can_deal) || (can_deal && enter) {
            self.bac_send(s, v, on.clone(), true);
        }
        let r = place(112.0);
        let rebet = self.poker.casino.bact.rebet.clone();
        let can_rebet = on.total() == 0 && rebet.total() > 0 && rebet.total() <= chips && open;
        if on.total() > 0 {
            if button(ui, r, "clear", "Clear", None, Color32::from_white_alpha(30), open && !ready) {
                self.bac_send(s, v, Bets::default(), false);
            }
        } else if button(ui, r, "rebet", "Rebet", Some(chips_text(rebet.total())), Color32::from_white_alpha(30), can_rebet) {
            self.bac_send(s, v, rebet, false);
        }
        let color = if on.total() > chips + mine.as_ref().map(|x| x.bets.total()).unwrap_or(0) { col::LOSE } else { col::GOLD2 };
        let max_chip = super::CHIPS.iter().copied().filter(|c| *c <= v.max_bet).last().unwrap_or(1);
        self.rack_and_bet(ui, bar, x, max_chip, format!("Bets {}", chips_text(on.total())), color);
        let hint = if v.stage == BacStage::Shown && !open {
            "The cards are being turned over".to_string()
        } else if on.total() == 0 {
            format!("Pick a chip, then click Player, Banker, Tie or a pair at your seat ({} to {})", chips_text(v.min_bet), chips_text(v.max_bet))
        } else {
            String::new()
        };
        if !hint.is_empty() {
            p.text(pos2(bar.center().x, bar.top() - 12.0), Align2::CENTER_CENTER, hint, theme::font(12.5), col::MUTED);
        }
        // How long's left to bet: a line along the top of the controls.
        if v.stage == BacStage::Betting {
            let frac = (left as f32 / BET_MS as f32).clamp(0.0, 1.0);
            let line = Rect::from_min_size(bar.left_top() + vec2(18.0, 0.0), vec2((bar.width() - 36.0) * frac, 3.0));
            p.rect_filled(line, theme::r(2), if frac > 0.3 { col::GOLD } else { col::LOSE });
        }
        // My bets are on the table now (or it said no, or the coup's gone by): the table's own
        // show from here.
        if let Some((coup, d)) = &self.poker.casino.bact.desired
            && (*coup != v.coup_no || mine.as_ref().is_some_and(|x| x.bets == *d) || s.poker.error.is_some())
        {
            self.poker.casino.bact.desired = None;
        }
    }
}

