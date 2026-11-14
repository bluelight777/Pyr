//! Dominó Cubano at its tables (0.13.11 servers): four seats round a square table in a Havana
//! bar, partners facing in Pairs (each for themselves in Solo), the house's players in the seats
//! nobody takes. As 0.13.28 draws it, measured off its screen: the tables along the top, the
//! room (`domino-room.webp`) and the table (`table-domino.webp`), whose felt the line snakes
//! across from the first tile laid, each end's number marked where the next tile goes; a plate
//! a seat (who, how many tiles, whose turn), their tiles face down beside them; my tiles big
//! along the bottom, and the controls under them. Every hand is played again by this Pyr from
//! its seeds once it's over (see `pyr_engine::poker::PokerState::dom_view`).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::casino::domino::{End, Kind, MAX_MULT, Move, Over, Tile};
use pyr_poker::domroom::TURN_MS;
use pyr_poker::net::{DomHandView, DomSeatView, DomStage, DomView, TableGame, ToServer};

use super::{chips_now, col, ease_out, hgrad, vgrad};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

use super::picker::STRIP_H;

/// Seats round a table.
const SEATS: usize = 4;
/// The controls along the bottom (68 tall, 10 off the bottom), and my tiles above them.
const BAR_H: f32 = 78.0;
const RACK_H: f32 = 126.0;
/// The table's art is square; the table's drawn this big when there's room (the 1280 × 800
/// window's), everything on it to scale.
const TABLE: f32 = 446.0;
/// The felt on the art (a fraction of it each way): the line stays on it.
const FELT: Rect = Rect { min: pos2(0.1570, 0.1794), max: pos2(0.8430, 0.8229) };
/// A tile on the line, long side (its short side half that), at the table's full size.
const TILE_L: f32 = 27.0;
/// How far the line runs either side of the first tile before it turns (in short sides): 0.13.28
/// turned a tile that would have reached 10.5 and kept one reaching 9.5.
const ROW_HALF: f32 = 9.5;
/// A seat's plate.
const PLATE: Vec2 = vec2(175.0, 58.0);
/// Each seat's own colour (before the first hand), and the two sides' in a hand.
const SEAT_COLORS: [Color32; SEATS] = [col::GOLD, Color32::from_rgb(0x4d, 0xb8, 0xff), Color32::from_rgb(0xff, 0x6b, 0x9a), Color32::from_rgb(0x4d, 0xe0, 0x9a)];
const TEAL: Color32 = Color32::from_rgb(0x3d, 0xd6, 0xc6);
/// ¡Doblo!'s orange (its button, as 0.13.28's: ember, (255,144,31) when hovered).
const DOBLO: Color32 = col::EMBER;
/// The covers I can put up: the ante times these.
const COVERS: [u32; 4] = [1, 2, 4, 8];

/// The pips' colours, 1 to 9 (a blank has none), as 0.13.28 paints them.
const PIP: [Color32; 10] = [
    Color32::BLACK,
    Color32::from_rgb(192, 28, 40),
    Color32::from_rgb(31, 122, 60),
    Color32::from_rgb(29, 63, 156),
    Color32::from_rgb(184, 90, 16),
    Color32::from_rgb(107, 42, 156),
    Color32::from_rgb(14, 14, 16),
    Color32::from_rgb(14, 126, 134),
    Color32::from_rgb(138, 22, 74),
    Color32::from_rgb(154, 116, 14),
];

#[derive(Default)]
pub struct DomTableUi {
    /// The table I'm at (watching, or sitting): its server and id.
    pub at: Option<(String, u64)>,
    asked: Option<Instant>,
    watch_asked: Option<Instant>,
    /// A table I opened: the newest table (above this id) is the one to go to.
    follow_new: Option<u64>,
    /// "+ New table"'s menu, open.
    new_menu: bool,
    /// How much I put up, in antes (×2 to begin with).
    cover: Option<u32>,
    /// A tile picked that goes on either end: the end's to be picked.
    picked: Option<Tile>,
    /// The move last sent, and where the hand had got to (its number, its moves, a double on
    /// the table), not to send it twice. (Another move there goes: one the table refused, a
    /// ¡Doblo! past what everyone covers, mustn't keep me from playing.)
    sent: Option<(u64, usize, bool, Move)>,
    /// When each tile in the line came (by hand and tile), for its slide in; the hand they're of.
    came: HashMap<Tile, Instant>,
    hand_of: Option<u64>,
    /// The last hand whose result was called out here.
    told: Option<u64>,
    /// My chips while I'm in a hand (the balance waits for its result).
    playing_chips: Option<u64>,
    /// Where the table had got to when its sounds were last made (the table, hand and moves), so
    /// each move sounds once, and nothing that happened before I came does.
    heard: Option<(u64, u64, DomStage, usize)>,
}

/// Where a sound's placed, by the place it comes from (mine or seat 0's at the bottom, then
/// right, top, left): dominó's own sounds half way to the side, the table's (its knock, chips and
/// double) 0.4, as 0.13.28 places them (measured: 1/√3 and √3 between the two sides).
const PAN_TILE: [f32; SEATS] = [0.0, 0.5, 0.0, -0.5];
const PAN_TABLE: [f32; SEATS] = [0.0, 0.4, 0.0, -0.4];
/// How long a tile takes to slide in.
const SLIDE: f32 = 0.35;
/// When its sound is asked for: as it lands, less the 0.05 s a sound takes to be heard here
/// (side by side with 0.13.28 at one table: each tile 0.32-0.40 s after its view came there).
const LANDS: f32 = SLIDE - 0.05;
/// The table's sounds' volumes here, as loud as 0.13.28's at a dominó table (measured against
/// what it played: `tests::table_sound_levels`; my turn's bells, the same every time, to the
/// sample's peak; the others, made fresh each time, by their energy and peaks).
const V_PASS: f32 = 0.57;
const V_DOUBLE: f32 = 0.8;
const V_ACCEPT: f32 = 0.7;
const V_TURN: f32 = 0.5;
const V_DEAL_IN: f32 = 0.6;
const V_DRAW: f32 = 0.62;

/// Where a tile of the line lies: its rectangle (in short sides, the first tile's middle the
/// origin), and which way it's read: across (the first number on the left) or down (the first
/// number on top).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Laid {
    pub rect: Rect,
    pub across: bool,
    pub first: u8,
    pub second: u8,
}

/// The line laid out: each tile where it lies (in the line's order), and where the next tile
/// goes at each end (its middle; the end's number is marked there).
#[derive(Debug, PartialEq)]
pub struct Layout {
    pub tiles: Vec<Laid>,
    pub next: [Pos2; 2],
}

/// One arm of the line, from the first tile out: the way it's going along its row (-1 left, +1
/// right), the way it turns (-1 up, +1 down), where the next tile starts, the row's middle, and
/// the last tile's rectangle.
struct Arm {
    h: f32,
    v: f32,
    at: f32,
    row: f32,
    last: Rect,
}

impl Arm {
    /// Where a tile goes next (and whether it turns): its rectangle.
    fn place(&self, double: bool) -> (Rect, bool) {
        let (w, ht) = if double { (1.0, 2.0) } else { (2.0, 1.0) };
        let far = self.at + self.h * w;
        if far.abs() <= ROW_HALF + 1e-3 {
            let (x0, x1) = if self.h > 0.0 { (self.at, far) } else { (far, self.at) };
            return (Rect::from_min_max(pos2(x0, self.row - ht / 2.0), pos2(x1, self.row + ht / 2.0)), false);
        }
        // It turns: down the side, beside the last tile's outer end.
        let (x0, x1) = if self.h > 0.0 { (self.at - 1.0, self.at) } else { (self.at, self.at + 1.0) };
        let (y0, y1) = if self.v < 0.0 { (self.last.top() - 2.0, self.last.top()) } else { (self.last.bottom(), self.last.bottom() + 2.0) };
        (Rect::from_min_max(pos2(x0, y0), pos2(x1, y1)), true)
    }

    fn lay(&mut self, r: Rect, turned: bool) {
        if turned {
            // The new row runs back the other way, level with the turn's far half.
            self.row = if self.v < 0.0 { r.top() + 0.5 } else { r.bottom() - 0.5 };
            self.h = -self.h;
            self.at = if self.h > 0.0 { r.right() } else { r.left() };
        } else {
            self.at = if self.h > 0.0 { r.right() } else { r.left() };
        }
        self.last = r;
    }
}

/// Lays the line out as 0.13.28 does: the first tile in the middle (a double crosswise), each
/// end running out along the row to `ROW_HALF` either side, then turning (the left end up, the
/// right end down) and coming back the other way a row over, and so on; doubles crosswise in a
/// row. (Up and down it's then put in the felt's middle by its tiles' middles: see `draw_line`.)
pub fn layout(chain: &[Tile], lead: usize) -> Layout {
    let mut tiles: Vec<Option<Laid>> = vec![None; chain.len()];
    let Some(&first) = chain.get(lead) else { return Layout { tiles: Vec::new(), next: [pos2(-1.5, 0.0), pos2(1.5, 0.0)] } };
    let double = first[0] == first[1];
    let lead_rect = if double { Rect::from_center_size(Pos2::ZERO, vec2(1.0, 2.0)) } else { Rect::from_center_size(Pos2::ZERO, vec2(2.0, 1.0)) };
    tiles[lead] = Some(Laid { rect: lead_rect, across: !double, first: first[0], second: first[1] });
    let mut next = [Pos2::ZERO; 2];
    // The left arm: the tiles before the first one, nearest first; its open number at each step
    // is the near number of the tile laid.
    for (k, dir) in [(0usize, -1.0f32), (1, 1.0)] {
        let mut arm = Arm { h: dir, v: dir, at: dir * lead_rect.width() / 2.0, row: 0.0, last: lead_rect };
        let idx: Vec<usize> = if k == 0 { (0..lead).rev().collect() } else { (lead + 1..chain.len()).collect() };
        for i in idx {
            let t = chain[i];
            // (The line's tiles are in order: [a, b] with b against the next one along.)
            let (near, far) = if k == 0 { (t[1], t[0]) } else { (t[0], t[1]) };
            let d = t[0] == t[1];
            let (r, turned) = arm.place(d);
            let laid = if turned {
                // Down the side: read from the row it leaves.
                let (top, bottom) = if arm.v < 0.0 { (far, near) } else { (near, far) };
                Laid { rect: r, across: false, first: top, second: bottom }
            } else if d {
                Laid { rect: r, across: false, first: t[0], second: t[1] }
            } else {
                let (left, right) = if arm.h < 0.0 { (far, near) } else { (near, far) };
                Laid { rect: r, across: true, first: left, second: right }
            };
            tiles[i] = Some(laid);
            arm.lay(r, turned);
        }
        let (r, _) = arm.place(false);
        next[k] = r.center();
    }
    Layout { tiles: tiles.into_iter().map(|t| t.unwrap()).collect(), next }
}

/// The line's up-and-down middle: halfway between its highest and lowest tile middles.
fn line_middle(l: &Layout) -> f32 {
    let (lo, hi) = l.tiles.iter().fold((f32::MAX, f32::MIN), |(lo, hi), t| (lo.min(t.rect.center().y), hi.max(t.rect.center().y)));
    if l.tiles.is_empty() { 0.0 } else { (lo + hi) / 2.0 }
}

/// Where the pips go on half a tile (fractions of the half, across and down), 1 to 9.
fn pips(n: u8) -> &'static [(f32, f32)] {
    const L: f32 = 0.25;
    const C: f32 = 0.5;
    const R: f32 = 0.75;
    const T: f32 = 0.25;
    const B: f32 = 0.75;
    match n {
        1 => &[(C, C)],
        2 => &[(L, T), (R, B)],
        3 => &[(L, T), (C, C), (R, B)],
        4 => &[(L, T), (R, T), (L, B), (R, B)],
        5 => &[(L, T), (R, T), (C, C), (L, B), (R, B)],
        6 => &[(L, T), (C, T), (R, T), (L, B), (C, B), (R, B)],
        7 => &[(L, T), (C, T), (R, T), (C, C), (L, B), (C, B), (R, B)],
        8 => &[(L, T), (C, T), (R, T), (L, C), (R, C), (L, B), (C, B), (R, B)],
        9 => &[(L, T), (C, T), (R, T), (L, C), (C, C), (R, C), (L, B), (C, B), (R, B)],
        _ => &[],
    }
}

/// A tile face up in `r` (down: `first` on top; across: `first` on the left), dimmed (`dim`:
/// 0 to 1), as 0.13.28 draws its tiles (measured off its rack, 46 × 92): an ivory face in a
/// two-tone edge, its depth under it, a dark bar across the middle with a brass pin, the pips on
/// a grid of three (across, the grid's turned: a 6 runs down in two columns).
pub fn tile(p: &egui::Painter, r: Rect, across: bool, first: u8, second: u8, dim: f32) {
    let short = r.width().min(r.height());
    let round = short * 0.11;
    // (Under it a shadow down and to the right, then its depth: 3 across and 4 down, and 2, under
    // one in my hand; about a pixel and less on the line, where they show by turns as the line
    // falls on the pixels. 0.13.28's, side by side: its rack, and its line over 40 hands.)
    let depth = short * 0.045;
    p.rect_filled(r.translate(vec2(short * 0.065, short * 0.093)), round, Color32::from_black_alpha(97));
    p.rect_filled(r.translate(vec2(0.0, depth)), round, Color32::from_rgb(215, 202, 173));
    // (A big one's edge is two-toned; a small one's, on the line, a single lighter line.)
    let face = if short >= 24.0 {
        p.rect_filled(r, round, Color32::from_rgb(185, 169, 134));
        p.rect_filled(r.shrink(1.0), (round - 1.0).max(0.0), Color32::from_rgb(223, 213, 187));
        r.shrink(2.0)
    } else {
        p.rect_filled(r, round, Color32::from_rgb(201, 188, 157));
        r.shrink(1.0)
    };
    super::vgrad_round(p, face, (round - 2.0).max(0.0), Color32::from_rgb(248, 242, 226), Color32::from_rgb(246, 239, 220));
    // The bar across the middle, and its brass pin.
    let (halves, bar) = if across {
        let w = r.width() / 2.0;
        let a = Rect::from_min_size(r.min, vec2(w, r.height()));
        let b = Rect::from_min_size(r.min + vec2(w, 0.0), vec2(w, r.height()));
        ([a, b], [pos2(r.center().x, r.top() + r.height() * 0.09), pos2(r.center().x, r.bottom() - r.height() * 0.09)])
    } else {
        let h = r.height() / 2.0;
        let a = Rect::from_min_size(r.min, vec2(r.width(), h));
        let b = Rect::from_min_size(r.min + vec2(0.0, h), vec2(r.width(), h));
        ([a, b], [pos2(r.left() + r.width() * 0.09, r.center().y), pos2(r.right() - r.width() * 0.09, r.center().y)])
    };
    p.line_segment(bar, Stroke::new((short * 0.045).max(1.0), Color32::from_rgb(109, 92, 64)));
    p.circle_filled(r.center(), short * 0.04 + 0.7, Color32::from_rgb(201, 154, 58));
    let pr = short * 0.065 + 0.45;
    for (half, n) in halves.iter().zip([first, second]) {
        for &(fx, fy) in pips(n) {
            // (Across, the grid's turned over its diagonal.)
            let (fx, fy) = if across { (fy, fx) } else { (fx, fy) };
            p.circle_filled(half.min + vec2(half.width() * fx, half.height() * fy), pr, PIP[n as usize]);
        }
    }
    if dim > 0.0 {
        p.rect_filled(r.expand2(vec2(0.0, depth / 2.0)).translate(vec2(0.0, depth / 2.0)), round, Color32::from_rgba_unmultiplied(88, 69, 24, (dim * 115.0) as u8));
    }
}

/// An end's number on the line, in its pips' colour made brighter (0.13.28's: half again and a
/// little more; a blank's in the 9's gold, a 6's as dark as its pips, hardly to be seen).
fn marker_ink(n: u8) -> Color32 {
    let c = PIP[if n == 0 { 9 } else { n.min(9) } as usize];
    let up = |v: u8| (v as f32 * 1.585).round().min(255.0) as u8;
    Color32::from_rgb(up(c.r()), up(c.g()), up(c.b()))
}

/// A tile face down, across or down: 0.13.28's dark red back, a gold line round it just inside
/// its edge, a gold dot in the middle.
fn back(p: &egui::Painter, r: Rect) {
    let short = r.width().min(r.height());
    p.rect_filled(r, 1.0, Color32::from_rgb(142, 22, 34));
    p.rect_stroke(r, 1.0, Stroke::new(1.0f32, Color32::from_rgb(150, 34, 37)), StrokeKind::Inside);
    p.rect_stroke(r.shrink(1.0), 0.5, Stroke::new(1.0f32, Color32::from_rgb(194, 110, 58)), StrokeKind::Inside);
    p.circle_filled(r.center(), short * 0.15, Color32::from_rgb(229, 170, 74));
}

/// A button on the controls; true when it's clicked.
fn button(ui: &mut egui::Ui, r: Rect, id: &str, text: &str, sub: Option<String>, fill: Color32, enabled: bool) -> bool {
    let resp = ui.interact(r, ui.id().with(("domt", id)), Sense::click());
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

/// Solo's pozo, what's left to draw from (0.13.28's, measured): five tiles face down, fanned up
/// to the right from the felt's lower left corner, and "POZO 31" under them.
fn dom_pozo(p: &egui::Painter, pool: usize, table: Rect, k: f32) {
    if pool == 0 {
        return;
    }
    let felt = Rect::from_min_max(table.min + FELT.min.to_vec2() * table.width(), table.min + FELT.max.to_vec2() * table.height());
    let corner = pos2(felt.left(), felt.bottom());
    for i in 0..pool.min(5) {
        let min = corner + vec2(14.0 + 3.25 * i as f32, -22.0 - 2.0 * i as f32) * k;
        back(p, Rect::from_min_size(min, vec2(18.0, 9.0) * k));
    }
    p.text(corner + vec2(36.0, -5.5) * k, Align2::LEFT_CENTER, format!("POZO {pool}"), theme::bold_font(11.0 * k), col::GOLD2);
}

/// ¡Doblo!'s words: bold 18 gold, lit round their edges 1.5 px (0.13.28's, measured: thinner
/// than the felt's outlined words).
fn lit_text(p: &egui::Painter, at: Pos2, text: &str, glow: Color32) {
    for (dx, dy) in [(-1.5, 0.0), (1.5, 0.0), (0.0, -1.5), (0.0, 1.5), (-1.06, -1.06), (1.06, 1.06), (1.06, -1.06), (-1.06, 1.06)] {
        p.text(at + vec2(dx, dy), Align2::CENTER_CENTER, text, theme::bold_font(18.0), glow);
    }
    p.text(at, Align2::CENTER_CENTER, text, theme::bold_font(18.0), col::GOLD2);
}

/// Where each place at the table is on the screen (the bottom mine, or seat 0's when I'm
/// watching; then round the way play goes: right, top, left).
fn plate_at(table: Rect, k: f32, place: usize) -> Pos2 {
    match place {
        0 => pos2(table.center().x, table.bottom() - 31.5 * k),
        1 => pos2(table.right() + 95.5 * k, table.center().y),
        2 => pos2(table.center().x, table.top() + 31.0 * k),
        _ => pos2(table.left() - 96.5 * k, table.center().y),
    }
}

/// The names of everyone on one side ("Olive & Yoandri").
fn side_names(v: &DomView, seats: &[u8]) -> String {
    let names: Vec<String> = seats.iter().filter_map(|s| v.seats.get(*s as usize).cloned().flatten()).map(|x| x.name).collect();
    names.join(" & ")
}

/// The line under how a hand ended: who won, the pot, a Capicúa's or Chiva's bonus, a ¡Doblo!'s
/// stake ("Olive & Chucho · pot 800 · +150% bonus · ×2", 0.13.28's words).
fn result_line(names: &str, pot: u64, bonus_pct: u32, mult: u32) -> String {
    let bonus = if bonus_pct > 0 { format!(" · +{bonus_pct}% bonus") } else { String::new() };
    let times = if mult > 1 { format!(" · ×{mult}") } else { String::new() };
    format!("{names} · pot {}{bonus}{times}", chips_text(pot))
}

/// The pips on some tiles.
fn pip_count(tiles: &[Tile]) -> u32 {
    tiles.iter().map(|t| (t[0] + t[1]) as u32).sum()
}

impl PyrApp {
    /// Leaves the dominó table I'm at (standing up: what I put up comes back; in a hand, my tiles
    /// are played for me till it's over).
    pub fn dom_table_leave(&mut self) {
        if let Some((key, id)) = self.poker.casino.domt.at.take() {
            self.engine.poker(&key, ToServer::DomLeave { table: id });
            self.engine.poker(&key, ToServer::DomUnwatch { table: id });
            self.engine.dom_close(&key);
        }
        let t = &mut self.poker.casino.domt;
        t.picked = None;
        t.sent = None;
        t.came.clear();
        t.hand_of = None;
        t.new_menu = false;
        t.playing_chips = None;
        t.heard = None;
    }

    /// The dominó tables' page: the tables along the top, the room and the table, my tiles, the
    /// controls.
    pub(super) fn dom_table_page(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let now = Instant::now();
        let key = s.key.clone();
        if self.poker.casino.domt.asked.is_none_or(|t| t.elapsed() > Duration::from_secs(8)) {
            self.poker.casino.domt.asked = Some(now);
            self.engine.poker(&key, ToServer::DomTables);
        }
        let tables = s.poker.dom_tables.clone().unwrap_or_default();
        if let Some(above) = self.poker.casino.domt.follow_new
            && let Some(d) = s.poker.dom.as_ref().filter(|d| d.view.id > above)
        {
            self.poker.casino.domt.follow_new = None;
            self.poker.casino.domt.at = Some((key.clone(), d.view.id));
            self.poker.casino.domt.asked = None;
        }
        let mine = self.poker.casino.domt.at.clone().filter(|(k, _)| *k == key).map(|(_, id)| id);
        let at = match mine {
            Some(id) if tables.is_empty() || tables.iter().any(|t| t.id == id) => Some(id),
            _ => tables.first().map(|t| t.id),
        };
        let live = s.poker.dom.clone().filter(|d| Some(d.view.id) == at);
        if let Some(id) = at {
            let moved = self.poker.casino.domt.at != Some((key.clone(), id));
            if moved || (live.is_none() && self.poker.casino.domt.watch_asked.is_none_or(|t| t.elapsed() > Duration::from_secs(3))) {
                self.poker.casino.domt.at = Some((key.clone(), id));
                self.poker.casino.domt.watch_asked = Some(now);
                self.engine.poker(&key, ToServer::DomWatch { table: id });
            }
        }
        let p = ui.painter().clone();
        let strip = Rect::from_min_size(area.min, vec2(area.width(), STRIP_H));
        let bar = Rect::from_min_max(pos2(area.left() + 16.0, area.bottom() - BAR_H), pos2(area.right() - 16.0, area.bottom() - 10.0));
        let rack = Rect::from_min_max(pos2(bar.left(), bar.top() - 6.0 - RACK_H), pos2(bar.right(), bar.top() - 6.0));
        // The room, darkened, and darker still to the sides and under the tables (0.13.28's, fitted
        // to its screen: the picture at 44%, half again darker at the sides fading over 296, a
        // quarter under the tables fading over 186).
        p.rect_filled(area, 0, Color32::from_rgb(4, 3, 2));
        super::pics::cover(&p, ui.ctx(), "domino-room", area, area.width().max(area.height()) as u32, Color32::from_gray(113));
        let side = (area.width() * 0.33).min(296.0);
        let dark = Color32::from_black_alpha(131);
        hgrad(&p, Rect::from_min_max(area.min, pos2(area.left() + side, area.bottom())), dark, Color32::TRANSPARENT);
        hgrad(&p, Rect::from_min_max(pos2(area.right() - side, area.top()), area.max), Color32::TRANSPARENT, dark);
        vgrad(&p, Rect::from_min_max(pos2(area.left(), strip.bottom()), pos2(area.right(), strip.bottom() + 186.0)), Color32::from_black_alpha(62), Color32::TRANSPARENT);
        vgrad(&p, Rect::from_min_max(pos2(area.left(), area.bottom() - 344.0), area.max), Color32::TRANSPARENT, Color32::from_black_alpha(79));
        // The table, as big as fits between the tables and my tiles.
        let room = Rect::from_min_max(pos2(area.left(), strip.bottom() + 10.0), pos2(area.right(), rack.top() - 6.0));
        let side_px = room.height().min(area.width() - 2.0 * 190.0).min(TABLE * 1.6).max(160.0);
        let table = Rect::from_center_size(pos2(area.center().x, room.top() + side_px / 2.0), vec2(side_px, side_px));
        let k = side_px / TABLE;
        super::pics::part(&p, ui.ctx(), "table-domino", Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), table, side_px as u32, Color32::WHITE);
        // My tiles' place, along the bottom.
        p.rect_filled(rack, theme::r(18), Color32::from_black_alpha(128));
        self.dom_strip(ui, s, strip, &tables, at);
        let Some(live) = live else {
            p.text(table.center(), Align2::CENTER_CENTER, if s.poker.dom_tables.is_none() { "Setting out the tables…" } else { "Sitting you at the table…" }, theme::bold_font(16.0), col::MUTED);
            return;
        };
        let v = live.view.clone();
        let clock = v.ms_left.saturating_sub(live.at.elapsed().as_millis() as u64);
        // A new hand: its tiles come as they're laid.
        let hand = v.hand.clone();
        if self.poker.casino.domt.hand_of != Some(v.round_no) && v.stage == DomStage::Playing {
            self.poker.casino.domt.hand_of = Some(v.round_no);
            self.poker.casino.domt.came.clear();
            self.poker.casino.domt.picked = None;
        }
        if let Some(h) = &hand {
            let fresh = self.poker.casino.domt.came.is_empty();
            let past = now.checked_sub(Duration::from_secs(5)).unwrap_or(now);
            for t in &h.chain {
                self.poker.casino.domt.came.entry(*t).or_insert(if fresh && h.chain.len() > 1 { past } else { now });
            }
        }
        self.dom_sounds(&v);
        // My chips while I'm in the hand: the balance waits for its result.
        let me = v.my_seat;
        let in_hand = me.is_some_and(|m| hand.as_ref().is_some_and(|h| h.seats.contains(&m)));
        if v.stage == DomStage::Playing && in_hand {
            self.poker.casino.domt.playing_chips = Some(chips_now(s));
        }
        self.dom_info(&p, &v, table, k);
        let base = me.unwrap_or(0) as usize;
        let place_of = |seat: usize| (seat + SEATS - base) % SEATS;
        // The line.
        if let Some(h) = &hand {
            let faded = v.stage != DomStage::Playing && v.stage != DomStage::Settled;
            self.dom_line(ui, s, &v, h, table, k, now, faded);
            // (Between hands too, the last hand's: 0.13.28 leaves it there, bright.)
            if h.draws {
                dom_pozo(&p, h.pool, table, k);
            }
        }
        // The seats, and the tiles in their hands.
        for seat in 0..SEATS {
            let c = plate_at(table, k, place_of(seat));
            self.dom_seat(ui, s, &v, seat as u8, place_of(seat), c, table, k, clock);
        }
        // A ¡Doblo! against my side: take it, or fold. (Anyone else sees who doubled, on the
        // felt where the answer's asked.)
        if let Some(h) = hand.as_ref().filter(|h| v.stage == DomStage::Playing && h.offer.is_some()) {
            self.dom_offer(ui, s, &v, h, table, k);
        }
        // Between hands: what to do.
        let between = hand.is_none() || matches!(v.stage, DomStage::Waiting | DomStage::Gathering);
        if between && (hand.is_none() || v.stage != DomStage::Settled) {
            self.dom_welcome(&p, &v, table, k);
        }
        // The result.
        if v.stage == DomStage::Settled {
            self.dom_result(ui, &v, table, k, now);
        }
        self.dom_rack(ui, s, &v, rack, now);
        self.dom_controls(ui, s, &live, bar, clock, now);
        // Emotes, big over the plate of whoever sent them.
        let size = (112.0 * k).clamp(84.0, 132.0);
        super::emotes::pops(&p, ui.ctx(), &s.poker.game_emotes, TableGame::Domino, v.id, size, |seat| plate_at(table, k, place_of((seat as usize).min(SEATS - 1))) - vec2(0.0, PLATE.y * 0.5 * k));
    }

    /// The tables, along the top (as 0.13.28 has them: the cards narrower than the other games',
    /// a table's kind and ante under its name), and "+ New table".
    fn dom_strip(&mut self, ui: &mut egui::Ui, s: &ServerView, strip: Rect, tables: &[pyr_poker::net::DomSummary], at: Option<u64>) {
        let p = ui.painter().clone();
        vgrad(&p, strip, Color32::from_rgb(0x19, 0x10, 0x0a), Color32::from_rgb(0x0a, 0x07, 0x04));
        p.text(pos2(strip.left() + 16.0, strip.top() + 21.0), Align2::LEFT_TOP, "TABLES", theme::bold_font(12.0), col::GOLD2);
        p.text(pos2(strip.left() + 16.0, strip.top() + 39.0), Align2::LEFT_TOP, "pick one", theme::font(11.0), col::MUTED);
        const CARD: Vec2 = vec2(170.0, 56.0);
        let new_w = 120.0;
        let new_r = Rect::from_min_size(pos2(strip.right() - 12.0 - new_w, strip.top() + 14.0), vec2(new_w, 44.0));
        let mut x = strip.left() + 86.0;
        let mut go = None;
        for t in tables {
            let r = Rect::from_min_size(pos2(x, strip.top() + 8.0), CARD);
            x += CARD.x + 10.0;
            if r.right() > new_r.left() - 6.0 {
                break;
            }
            let on = Some(t.id) == at;
            let resp = ui.interact(r, ui.id().with(("dom-table", t.id)), Sense::click());
            if on {
                super::vgrad_round(&p, r, 14.0, col::GOLD.gamma_multiply(0.40), col::GOLD.gamma_multiply(0.17));
                p.rect_stroke(r, theme::r(14), Stroke::new(2.0f32, col::GOLD2), StrokeKind::Inside);
            } else {
                p.rect_filled(r, theme::r(14), Color32::from_white_alpha(if resp.hovered() { 22 } else { 11 }));
                p.rect_stroke(r, theme::r(14), Stroke::new(1.0f32, Color32::from_white_alpha(50)), StrokeKind::Inside);
            }
            // YOU'RE HERE, or the seats open.
            let seated = t.people + t.bots;
            let open = SEATS.saturating_sub(seated);
            let (words, fill, ink) = if on {
                ("YOU'RE HERE".to_string(), col::GOLD2, Color32::from_rgb(0x2a, 0x10, 0x04))
            } else if open == 0 {
                ("FULL".to_string(), Color32::from_rgb(0x8a, 0x24, 0x1c), Color32::WHITE)
            } else {
                (format!("{open} OPEN"), Color32::from_rgb(0x1e, 0x6a, 0x3e), Color32::WHITE)
            };
            let g = p.layout_no_wrap(words, theme::bold_font(10.0), ink);
            let pill = Rect::from_min_size(pos2(r.right() - 12.0 - g.size().x - 14.0, r.top() + 9.0), vec2(g.size().x + 14.0, 17.0));
            // The name, as far as the pill (a long one goes under it, as 0.13.28's).
            let name_color = if on { col::GOLD2 } else { Color32::WHITE };
            let cut = p.with_clip_rect(Rect::from_min_max(r.min, pos2(pill.left() - 1.0, r.bottom())).intersect(p.clip_rect()));
            cut.text(r.left_top() + vec2(14.0, 9.0), Align2::LEFT_TOP, &t.name, theme::bold_font(16.0), name_color);
            p.rect_filled(pill, egui::CornerRadius::same(9), fill);
            p.galley(pos2(pill.left() + 7.0, r.top() + 12.0), g, ink);
            let dot = |i: usize| pos2(r.right() - 14.0 - (SEATS - 1 - i) as f32 * 13.0, r.top() + 40.0);
            for i in 0..SEATS {
                if i < seated {
                    p.circle_filled(dot(i), 4.5, Color32::from_rgb(0x3a, 0xe0, 0x7a));
                } else {
                    p.circle_stroke(dot(i), 4.0, Stroke::new(1.0f32, Color32::from_white_alpha(110)));
                }
            }
            // Its kind and ante (shorter where they'd run into the dots).
            let room = dot(0).x - 4.5 - 6.0 - (r.left() + 14.0);
            let color = if on { Color32::from_rgb(0xff, 0xe6, 0xa8) } else { Color32::from_rgb(0xdf, 0xb2, 0x4c) };
            let kind = if t.kind == Kind::Pairs { "Pairs" } else { "Solo" };
            let says = [format!("{kind} · ante {}", chips_text(t.ante)), format!("{kind} · {}", chips_text(t.ante)), format!("{kind} · {}", crate::poker::cards::chips_short(t.ante))];
            let g = says.iter().map(|w| p.layout_no_wrap(w.clone(), theme::bold_font(12.5), color)).find(|g| g.size().x <= room).unwrap_or_else(|| p.layout_no_wrap(says[2].clone(), theme::bold_font(12.5), color));
            p.galley(r.left_top() + vec2(14.0, 33.0), g, color);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                go = Some(t.id);
            }
        }
        if let Some(id) = go {
            self.dom_table_leave();
            self.poker.casino.domt.at = Some((s.key.clone(), id));
            self.engine.poker(&s.key, ToServer::DomWatch { table: id });
            self.poker.casino.domt.watch_asked = Some(Instant::now());
        }
        // A new table: Pairs or Solo, at an ante I can put up.
        let resp = ui.interact(new_r, ui.id().with("dom-new"), Sense::click());
        let open = self.poker.casino.domt.new_menu;
        p.rect_filled(new_r, theme::r(12), Color32::from_white_alpha(if resp.hovered() || open { 40 } else { 26 }));
        p.rect_stroke(new_r, theme::r(12), Stroke::new(1.0f32, Color32::from_white_alpha(40)), StrokeKind::Inside);
        p.text(new_r.center(), Align2::CENTER_CENTER, "+ New table", theme::bold_font(15.0), col::TEXT);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.poker.casino.domt.new_menu = !open;
        }
        if self.poker.casino.domt.new_menu {
            let chips = chips_now(s);
            let mut picked = None;
            let menu = egui::Area::new(ui.id().with("dom-new-menu")).order(egui::Order::Foreground).fixed_pos(pos2(new_r.right() - 300.0, new_r.bottom() + 9.0)).show(ui.ctx(), |ui| {
                egui::Frame::new().fill(Color32::from_rgb(0x1c, 0x18, 0x14)).stroke(Stroke::new(1.0f32, Color32::from_white_alpha(30))).corner_radius(theme::r(10)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
                    ui.set_width(276.0);
                    ui.label(egui::RichText::new("A new dominó table").font(theme::bold_font(14.5)).color(col::GOLD2));
                    ui.label(egui::RichText::new("Pairs: two against two, partners facing. Solo: each for themselves.").font(theme::font(11.5)).color(col::MUTED));
                    for kind in [Kind::Pairs, Kind::Solo] {
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(if kind == Kind::Pairs { "Pairs" } else { "Solo" }).font(theme::bold_font(12.5)).color(col::TEXT));
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
                            for ante in pyr_poker::domroom::ANTES {
                                let ok = ante <= chips;
                                let b = egui::Button::new(egui::RichText::new(chips_text(ante)).font(theme::font(13.0)).color(if ok { col::TEXT } else { Color32::from_white_alpha(70) })).fill(Color32::from_white_alpha(if ok { 18 } else { 8 })).corner_radius(theme::r(6)).min_size(vec2(40.0, 28.0));
                                if ui.add_enabled(ok, b).clicked() {
                                    picked = Some((kind, ante));
                                }
                            }
                        });
                    }
                });
            });
            if let Some((kind, ante)) = picked {
                self.poker.casino.domt.new_menu = false;
                self.dom_table_leave();
                self.poker.casino.domt.follow_new = Some(tables.iter().map(|t| t.id).max().unwrap_or(0));
                self.engine.poker(&s.key, ToServer::DomNew { kind, ante });
            } else if menu.response.clicked_elsewhere() && !ui.rect_contains_pointer(new_r) {
                self.poker.casino.domt.new_menu = false;
            }
        }
    }

    /// The plate by the table: the kind and the table's name, the ante (the pot in a hand); in a
    /// hand, the doubling cube under it.
    fn dom_info(&self, p: &egui::Painter, v: &DomView, table: Rect, k: f32) {
        let r = Rect::from_min_size(pos2(table.left() - 22.0 * k - 150.0, table.top() + 53.0 * k), vec2(150.0, 54.0));
        p.rect_filled(r, theme::r(10), Color32::from_black_alpha(190));
        let kind = if v.kind == Kind::Pairs { "PAIRS" } else { "SOLO" };
        p.text(r.left_top() + vec2(10.0, 8.0), Align2::LEFT_TOP, format!("{kind} · {}", v.name), theme::bold_font(11.0), col::MUTED);
        let playing = v.hand.as_ref().is_some_and(|h| h.over.is_none()) || v.stage == DomStage::Settled;
        let big = match (&v.hand, playing) {
            (Some(h), true) => format!("Pot {}", chips_text(v.ante * h.seats.len() as u64 * h.mult as u64)),
            _ => format!("Ante {}", chips_text(v.ante)),
        };
        p.text(r.left_top() + vec2(10.0, 24.0), Align2::LEFT_TOP, big, theme::bold_font(17.0), col::GOLD2);
        // The cube: what a ¡Doblo! would take the stake to, and whose it is to turn.
        let Some(h) = &v.hand else { return };
        if h.kind == Kind::Solo && h.seats.len() > 2 {
            return;
        }
        let c = pos2(r.center().x, table.top() + 141.0 * k);
        let cube = Rect::from_center_size(c, vec2(34.0, 34.0));
        let can = h.mult < MAX_MULT;
        p.rect_filled(cube, theme::r(6), if can { Color32::from_rgb(0xf6, 0xf0, 0xe2) } else { Color32::from_rgb(0xc8, 0xb8, 0x98) });
        // (Taken: ringed gold when it's my side's, teal when it's theirs: watching, always teal,
        // as 0.13.28 has it whichever side took it.)
        let mine = v.my_seat.and_then(|m| h.seats.iter().position(|s| *s == m)).map(|p| (p % 2) as u8);
        if let Some(side) = h.cube {
            p.rect_stroke(cube, theme::r(6), Stroke::new(3.0f32, if mine == Some(side) { col::GOLD } else { TEAL }), StrokeKind::Inside);
        }
        let shown = if h.mult == 1 { 2 } else { h.mult };
        p.text(c, Align2::CENTER_CENTER, shown.to_string(), theme::bold_font(17.0), Color32::from_rgb(0x1a, 0x14, 0x10));
        let who = match h.cube {
            None => "¡DOBLO! open".to_string(),
            Some(side) => {
                if mine == Some(side) { format!("×{} · yours to double", h.mult) } else { format!("×{} · theirs", h.mult) }
            }
        };
        p.text(c + vec2(0.0, 29.0), Align2::CENTER_CENTER, who, theme::font(10.5), col::MUTED);
    }

    /// The line on the felt: each tile where it lies (the newest sliding in from whoever laid it,
    /// glowing), each end's number where the next tile goes; my picked tile's ends to choose.
    #[allow(clippy::too_many_arguments)]
    fn dom_line(&mut self, ui: &mut egui::Ui, s: &ServerView, v: &DomView, h: &DomHandView, table: Rect, k: f32, now: Instant, faded: bool) {
        let p = ui.painter().clone();
        let felt = Rect::from_min_max(table.min + FELT.min.to_vec2() * table.width(), table.min + FELT.max.to_vec2() * table.height());
        let l = layout(&h.chain, h.lead);
        let unit = TILE_L * k / 2.0;
        // (Too tall for the felt, a very long line: smaller.)
        let (lo, hi) = l.tiles.iter().fold((f32::MAX, f32::MIN), |(lo, hi), t| (lo.min(t.rect.top()), hi.max(t.rect.bottom())));
        let tall = (hi - lo + 2.0) * unit;
        let fit = (felt.height() / tall.max(1.0)).min(1.0);
        let unit = unit * fit;
        let mid = line_middle(&l);
        // (0.13.28's sits 0.6 above the felt's middle and 0.2 left of it: matched to it over 40
        // hands, side by side, by where its tiles' edges and shadows fall on the pixels.)
        let origin = pos2(felt.center().x - 0.2 * k, felt.center().y - 0.6 * k - mid * unit);
        let to_screen = |r: Rect| Rect::from_min_max(origin + r.min.to_vec2() * unit, origin + r.max.to_vec2() * unit);
        let painter = p.clone();
        // (The tile as it was played, either way round: the line has it turned to fit.)
        let same = |a: Tile, b: Tile| a == b || a == [b[1], b[0]];
        let last_tile = h.last.and_then(|(_, m)| if let Move::Play { tile, .. } = m { Some(tile) } else { None });
        let laid_by = h.last.map(|(seat, _)| seat);
        let base = v.my_seat.unwrap_or(0) as usize;
        for (i, t) in l.tiles.iter().enumerate() {
            let tile_here = h.chain[i];
            let r = to_screen(t.rect);
            let came = self.poker.casino.domt.came.get(&tile_here).copied().unwrap_or(now);
            let a = ((now.saturating_duration_since(came)).as_secs_f32() / SLIDE).min(1.0);
            let r = if a < 1.0 && !faded {
                // Sliding in from whoever laid it.
                let from = laid_by.map(|seat| plate_at(table, k, (seat as usize + SEATS - base) % SEATS)).unwrap_or(r.center());
                ui.ctx().request_repaint();
                r.translate((from - r.center()) * (1.0 - ease_out(a)))
            } else {
                r
            };
            // (The newest one glows on after the hand's over, as 0.13.28's: the dominó that won it,
            // under the result, and under the felt darkened till the next hand's dealt.)
            let newest = last_tile.is_some_and(|t| same(t, tile_here));
            if newest {
                super::radial(&painter, r.center(), TILE_L * k * 1.15, col::GOLD.gamma_multiply(0.32), Color32::TRANSPARENT);
            }
            tile(&painter, r, t.across, t.first, t.second, 0.0);
            if newest {
                painter.rect_stroke(r, 1.5, Stroke::new(1.5f32, col::GOLD.gamma_multiply(0.9)), StrokeKind::Inside);
            }
        }
        if faded {
            // (The last hand's line, under the felt darkened, between hands.)
            let dim = Rect::from_min_max(table.min + vec2(74.0, 83.0) * k, table.min + vec2(372.0, 363.0) * k);
            p.rect_filled(dim, theme::r(4), Color32::from_black_alpha(150));
        }
        if faded || h.over.is_some() || h.chain.is_empty() {
            return;
        }
        // Each end's number, where the next tile goes; a picked tile's ends, to click (0.13.28's:
        // a tile's room, ringed in gold, glowing orange, the number orange in it).
        let ends = [h.chain[0][0], h.chain[h.chain.len() - 1][1]];
        let picked = self.poker.casino.domt.picked;
        for (e, at) in l.next.iter().enumerate() {
            let c = origin + at.to_vec2() * unit;
            let n = ends[e];
            let end = if e == 0 { End::Left } else { End::Right };
            if let Some(t) = picked.filter(|t| h.allowed.contains(&Move::Play { tile: *t, end })) {
                let spot = Rect::from_center_size(c, vec2(28.0, 28.0) * k);
                let resp = ui.interact(spot, ui.id().with(("dom-end", e)), Sense::click());
                super::radial(&p, c, 30.0 * k, col::EMBER.gamma_multiply(0.30), Color32::TRANSPARENT);
                p.rect_filled(spot, theme::r(8), Color32::from_black_alpha(if resp.hovered() { 90 } else { 130 }));
                super::radial(&p, c, 10.0 * k, col::EMBER.gamma_multiply(0.35), Color32::TRANSPARENT);
                p.rect_stroke(spot, theme::r(8), Stroke::new(3.0f32, col::GOLD2), StrokeKind::Middle);
                p.text(c, Align2::CENTER_CENTER, n.to_string(), theme::bold_font(9.0 * k.max(0.8)), Color32::from_rgb(255, 112, 40));
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.dom_send(s, v, Move::Play { tile: t, end });
                }
                continue;
            }
            p.circle_filled(c, 5.5 * k, Color32::from_black_alpha(115));
            p.text(c, Align2::CENTER_CENTER, n.to_string(), theme::bold_font(8.5 * k.max(0.8)), marker_ink(n));
        }
    }

    /// A seat: its plate (who; how many tiles, or ready, or how it ended for them; whose turn),
    /// an open seat's SIT and + BOT; their tiles face down beside them (shown at the end).
    #[allow(clippy::too_many_arguments)]
    fn dom_seat(&mut self, ui: &mut egui::Ui, s: &ServerView, v: &DomView, seat: u8, place: usize, c: Pos2, table: Rect, k: f32, clock: u64) {
        let p = ui.painter().clone();
        let plate = Rect::from_center_size(c, PLATE);
        let x = v.seats.get(seat as usize).cloned().flatten();
        let hand = v.hand.as_ref();
        let pos = hand.and_then(|h| h.seats.iter().position(|s| *s == seat));
        let playing = v.stage == DomStage::Playing;
        let turn = playing && hand.is_some_and(|h| h.to_act == Some(seat) && h.over.is_none());
        let me = v.my_seat == Some(seat);
        // Its colour: its own before the first hand; in one, its side's (mine gold, as 0.13.28's: the
        // bottom and top places, whoever's watching).
        let color = match (hand, pos) {
            (Some(h), Some(_)) if h.kind == Kind::Pairs => if place % 2 == 0 { col::GOLD } else { TEAL },
            _ => SEAT_COLORS[seat as usize],
        };
        p.rect_filled(plate, theme::r(10), Color32::from_black_alpha(200));
        let border = if turn { Stroke::new(2.5f32, col::GOLD2) } else { Stroke::new(1.0f32, Color32::from_white_alpha(28)) };
        p.rect_stroke(plate, theme::r(10), border, StrokeKind::Inside);
        p.rect_filled(Rect::from_min_size(pos2(plate.left() + 6.5, c.y - 20.0), vec2(4.0, 40.0)), theme::r(2), color);
        let face_c = plate.left_center() + vec2(39.0, 0.0);
        let fr = 20.0;
        let Some(x) = x else {
            // An open seat.
            p.text(pos2(plate.left() + 19.0, c.y), Align2::LEFT_CENTER, "Open seat", theme::font(13.5), Color32::from_white_alpha(110));
            let sitting = v.my_seat.is_some();
            let bot = Rect::from_min_size(pos2(plate.left() + 109.0, c.y - 12.0), vec2(57.0, 24.0));
            if !sitting {
                let sit = Rect::from_min_size(pos2(plate.left() + 56.0, c.y - 12.0), vec2(46.0, 24.0));
                let resp = ui.interact(sit, ui.id().with(("dom-sit", seat)), Sense::click());
                p.rect_filled(sit, theme::r(8), Color32::from_black_alpha(if resp.hovered() { 120 } else { 200 }));
                p.rect_stroke(sit, theme::r(8), Stroke::new(1.0f32, col::GOLD.gamma_multiply(0.8)), StrokeKind::Inside);
                p.text(sit.center(), Align2::CENTER_CENTER, "SIT", theme::bold_font(11.5), col::GOLD2);
                if resp.on_hover_text("Sit here").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.engine.poker(&s.key, ToServer::DomSit { table: v.id, seat });
                }
            }
            let resp = ui.interact(bot, ui.id().with(("dom-bot", seat)), Sense::click());
            p.rect_filled(bot, theme::r(8), if resp.hovered() { Color32::from_rgb(0x3d, 0x5f, 0xc4) } else { Color32::from_white_alpha(22) });
            p.text(bot.center(), Align2::CENTER_CENTER, "+ BOT", theme::bold_font(11.5), Color32::WHITE);
            if resp.on_hover_text("Seat a bot here: it plays with the house's chips").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.engine.poker(&s.key, ToServer::DomBot { table: v.id, seat, add: true });
            }
            return;
        };
        if x.bot {
            crate::poker::table::bot_face(&p, face_c, fr, &x.name, pyr_poker::bots::Level::Regular);
            let resp = ui.interact(plate, ui.id().with(("dom-botplate", seat)), Sense::click()).on_hover_text(format!("{}: a bot. It plays with the house's chips.", x.name));
            if v.stage != DomStage::Playing {
                resp.context_menu(|ui| {
                    crate::menu::style(ui);
                    if ui.button(format!("Ask {} to leave", x.name)).clicked() {
                        self.engine.poker(&s.key, ToServer::DomBot { table: v.id, seat, add: false });
                        ui.close();
                    }
                });
            }
        } else {
            let face = self.face(ui.ctx(), s, &x.player);
            crate::app::avatar(&p, face_c, fr, &x.player, &x.name, false, Color32::BLACK, face);
        }
        // Their time running out: a ring round their picture (a bot's turn's short, as 0.13.28's:
        // it's counted against a person's time).
        if turn {
            let frac = (clock as f32 / TURN_MS as f32).clamp(0.0, 1.0);
            let ring = if frac > 0.3 { col::GOLD2 } else { col::LOSE };
            arc(&p, face_c, fr + 4.0, frac, Stroke::new(2.5f32, ring));
        }
        let name = if x.name.chars().count() > 13 { format!("{}…", x.name.chars().take(12).collect::<String>()) } else { x.name.clone() };
        p.text(face_c + vec2(29.0, -9.0), Align2::LEFT_CENTER, name, theme::bold_font(14.0), if me { col::GOLD2 } else { col::TEXT });
        let (status, tint) = self.dom_status(v, seat, &x, pos);
        p.text(face_c + vec2(29.0, 11.0), Align2::LEFT_CENTER, status, theme::font(12.0), tint);
        // Their tiles, face down beside them (mine are along the bottom); everyone's at the end.
        let (Some(h), Some(i)) = (hand, pos) else { return };
        // (Mine are along the bottom, and not shown again at the end: 0.13.28's; everyone else's
        // beside them once it's over, till the next is dealt.)
        if me {
            return;
        }
        let shown = h.shown.get(i).cloned().unwrap_or_default();
        let n = if h.over.is_some() { shown.len() } else { h.counts.get(i).copied().unwrap_or(0) as usize };
        if n == 0 || (h.over.is_none() && v.stage != DomStage::Playing) {
            return;
        }
        let (long, short) = (23.0 * k, 11.0 * k);
        let pitch = 13.4 * k;
        let total = (n as f32 - 1.0) * pitch + short;
        for j in 0..n {
            let r = match place {
                0 | 2 => Rect::from_min_size(pos2(plate.right() + 11.5 * k + j as f32 * pitch, c.y - long / 2.0), vec2(short, long)),
                1 => Rect::from_min_size(pos2(table.right() - 41.5 * k, table.center().y - total / 2.0 + j as f32 * pitch), vec2(long, short)),
                _ => Rect::from_min_size(pos2(table.left() + 18.0 * k, table.center().y - total / 2.0 + j as f32 * pitch), vec2(long, short)),
            };
            match shown.get(j) {
                Some(t) if h.over.is_some() => {
                    let across = r.width() > r.height();
                    tile(&p, r, across, t[0], t[1], 0.0);
                }
                _ => back(&p, r),
            }
        }
    }

    /// What a seat's plate says under the name, and in what colour.
    fn dom_status(&self, v: &DomView, seat: u8, x: &DomSeatView, pos: Option<usize>) -> (String, Color32) {
        let h = v.hand.as_ref();
        match (v.stage, h, pos) {
            (DomStage::Playing, Some(h), Some(i)) => {
                let n = h.counts.get(i).copied().unwrap_or(0);
                let mut words = if n == 1 { "1 tile".to_string() } else { format!("{n} tiles") };
                if h.last.is_some_and(|(s, m)| s == seat && m == Move::Pass) {
                    words += " · passed";
                }
                // (Solo: a tile taken from the pozo, as 0.13.28 says it.)
                if h.last.is_some_and(|(s, m)| s == seat && m == Move::Draw) {
                    words += " · drew";
                }
                // (Whoever's ¡Doblo! is on the table.)
                if h.offer.is_some() && h.last.is_some_and(|(s, m)| s == seat && m == Move::Double) {
                    words += " · ¡DOBLO!";
                }
                // (Orange only for two tiles or fewer: the doubler's "· ¡DOBLO!" stays muted, as
                // 0.13.28's does, side by side.)
                (words, if n <= 2 { col::EMBER } else { col::MUTED })
            }
            (DomStage::Settled, Some(h), Some(i)) => {
                let r = v.result.as_ref();
                if r.is_some_and(|r| r.winners.contains(&seat)) {
                    // (A person's share; a bot's is the house's.)
                    let paid = r.and_then(|r| r.paid.iter().find(|(s, _)| *s == seat)).map(|(_, c)| *c).unwrap_or(0);
                    if !x.bot && paid > 0 { (format!("won {}", chips_text(paid)), col::WIN) } else { ("won".to_string(), col::WIN) }
                } else {
                    let left = pip_count(h.shown.get(i).map(|t| t.as_slice()).unwrap_or(&[]));
                    (format!("{left} pips left"), col::MUTED)
                }
            }
            // (Sitting while a hand goes on without them: 0.13.28's words.)
            (DomStage::Playing, Some(_), None) => ("sitting this one out".to_string(), col::MUTED),
            _ if x.ready && !x.bot && x.escrow > 0 => (format!("in · {} up", chips_text(x.escrow)), col::WIN),
            _ if x.ready => ("ready".to_string(), col::MUTED),
            _ => ("not in yet".to_string(), col::MUTED),
        }
    }

    /// A ¡Doblo! against my side, on the felt (as 0.13.28's): who doubled, what accepting and
    /// folding come to, and the two buttons.
    fn dom_offer(&mut self, ui: &mut egui::Ui, s: &ServerView, v: &DomView, h: &DomHandView, table: Rect, k: f32) {
        let p = ui.painter().clone();
        let answering = h.allowed.contains(&Move::Accept);
        let who = h.last.filter(|(_, m)| *m == Move::Double).and_then(|(seat, _)| v.seats.get(seat as usize).cloned().flatten()).map(|x| x.name).unwrap_or_default();
        let title = format!("¡DOBLO!  {who} doubles to ×{}", h.mult * 2);
        // (The words lit orange round their edges, as 0.13.28's are.)
        let glow = col::EMBER.gamma_multiply(0.9);
        let c = pos2(table.center().x, table.center().y + 79.0 * k);
        if !answering {
            // The doubling side's (0.13.28's, measured: 278 by 54, on the answer's middle).
            let r = Rect::from_center_size(c, vec2(278.0, 54.0));
            p.rect_filled(r, theme::r(14), Color32::from_black_alpha(242));
            p.rect_stroke(r, theme::r(14), Stroke::new(2.0f32, col::EMBER), StrokeKind::Inside);
            lit_text(&p, r.center() - vec2(0.0, 3.0), &title, glow);
            return;
        }
        let r = Rect::from_center_size(c, vec2(278.0, 118.0));
        p.rect_filled(r, theme::r(14), Color32::from_black_alpha(242));
        p.rect_stroke(r, theme::r(14), Stroke::new(2.0f32, col::EMBER), StrokeKind::Inside);
        lit_text(&p, pos2(r.center().x, r.top() + 24.0), &title, glow);
        let stake = v.ante * h.mult as u64;
        p.text(
            pos2(r.center().x, r.top() + 49.0),
            Align2::CENTER_CENTER,
            format!("Accept: play on for {} each (and the cube is yours). Fold: lose {} now.", chips_text(stake * 2), chips_text(stake)),
            theme::font(12.5),
            col::MUTED,
        );
        let accept = Rect::from_min_size(pos2(r.left() + 13.0, r.top() + 66.0), vec2(120.0, 40.0));
        let fold = Rect::from_min_size(pos2(r.left() + 145.0, r.top() + 66.0), vec2(120.0, 40.0));
        if button(ui, accept, "accept", "Accept", None, col::GOLD, true) {
            self.dom_send(s, v, Move::Accept);
        }
        if button(ui, fold, "fold", "Fold", None, Color32::from_rgb(35, 42, 40), true) {
            self.dom_send(s, v, Move::Fold);
        }
    }

    /// The words on the felt between hands.
    fn dom_welcome(&self, p: &egui::Painter, v: &DomView, table: Rect, k: f32) {
        let c = pos2(table.center().x, table.center().y + 7.0 * k);
        let box_r = Rect::from_center_size(c, vec2(274.0, 95.0));
        p.rect_filled(box_r, theme::r(12), Color32::from_black_alpha(130));
        // (A hand's coming: 0.13.28 says so while it gathers.)
        let title = if v.stage == DomStage::Gathering { "Next hand: put up your chips" } else { "Sit down and put up the ante to play" };
        super::glow_text(p, c - vec2(0.0, 21.0), Align2::CENTER_CENTER, title, theme::bold_font(20.0), col::GOLD2, Color32::from_black_alpha(200));
        let rules = if v.kind == Kind::Pairs { "Two against two, partners facing. Double-nine, ten tiles each." } else { "Each for themselves. Double-nine, ten tiles each; three or fewer draw from the pozo." };
        p.text(c + vec2(0.0, 9.0), Align2::CENTER_CENTER, rules, theme::font(13.0), col::MUTED);
        p.text(c + vec2(0.0, 29.0), Align2::CENTER_CENTER, "Capicúa pays half again · Chiva pays double · ¡Doblo! up to ×8", theme::font(12.5), Color32::from_rgb(0xe8, 0xb7, 0x52));
    }

    /// The hand over: how it ended and who won, on the felt; mine called out big.
    fn dom_result(&mut self, ui: &mut egui::Ui, v: &DomView, table: Rect, k: f32, now: Instant) {
        let (Some(r), Some(h)) = (&v.result, &v.hand) else { return };
        let p = ui.painter().clone();
        let side: Vec<u8> = r.winners.clone();
        let names = side_names(v, &side);
        let r_box = Rect::from_center_size(pos2(table.center().x, table.top() + 139.0 * k), vec2(240.0, 74.0));
        // (How it ended, 12.5 above the middle: bold 24 gold2 edged dark; a Capicúa or a Chiva bold
        // 28 lit ember round in a box edged ember, the bonus on the line under it. 0.13.28's,
        // measured side by side to the pixel.)
        let bonus = r.bonus_pct > 0;
        p.rect_filled(r_box, theme::r(14), Color32::from_black_alpha(200));
        p.rect_stroke(r_box, theme::r(14), Stroke::new(2.0f32, if bonus { col::EMBER } else { col::GOLD }), StrokeKind::Inside);
        let (size, edge) = if bonus { (28.0, col::EMBER) } else { (24.0, Color32::from_black_alpha(120)) };
        super::glow_text(&p, r_box.center() - vec2(0.0, 12.5), Align2::CENTER_CENTER, &r.how, theme::bold_font(size), col::GOLD2, edge);
        p.text(r_box.center() + vec2(0.0, 18.0), Align2::CENTER_CENTER, result_line(&names, r.pot, r.bonus_pct, r.mult), theme::font(13.0), col::TEXT);
        // A tranque: who had the fewest pips.
        if let Some(Over::Blocked { pips, .. }) = &h.over {
            // (Each side, or in Solo each player by their place: "P1: 52  ·  P2: 15", 0.13.28's.)
            let each: Vec<String> = pips.iter().enumerate().map(|(i, n)| format!("{}{}: {n}", if h.kind == Kind::Pairs { "Side " } else { "P" }, i + 1)).collect();
            p.text(pos2(r_box.center().x, r_box.bottom() + 14.0), Align2::CENTER_CENTER, format!("Tranque: fewest pips wins  ({})", each.join("  ·  ")), theme::font(12.0), col::MUTED);
        }
        // Mine, once.
        if self.poker.casino.domt.told == Some(v.round_no) {
            return;
        }
        self.poker.casino.domt.told = Some(v.round_no);
        let Some(m) = v.my_seat.filter(|m| h.seats.contains(m)) else { return };
        let paid = r.paid.iter().find(|(s, _)| *s == m).map(|(_, c)| *c).unwrap_or(0);
        let staked = v.ante * r.mult as u64;
        if let Some(chips) = self.poker.casino.domt.playing_chips.take() {
            self.poker.casino.hold = Some((chips, now + Duration::from_millis(600)));
        }
        // (How it ended, big: what I won, or the stake I lost.)
        let title = r.how.to_uppercase();
        if r.winners.contains(&m) {
            self.poker.casino.announce(title, format!("+{}", chips_text(paid.saturating_sub(staked))), col::WIN, (0.5, 0.42), 0.3);
            // (0.13.28's: its small win 0.3 s after the last tile's slammed down, the chips coming
            // in 0.35 s after that.)
            self.poker.casino.sound_in(LANDS + 0.3, Cue::WinSmall, 0.0, 0.8);
            self.poker.casino.sound_in(LANDS + 0.65, Cue::Chips, 0.0, V_ACCEPT);
            self.poker.casino.celebrate(paid, staked, false, 0.6);
        } else {
            self.poker.casino.announce(title, format!("−{}", chips_text(staked)), col::LOSE, (0.5, 0.42), 0.3);
        }
    }

    /// My tiles, big along the bottom: on my turn the ones that go bright and the rest dimmed;
    /// click one to lay it (on the end it goes on; one that goes on either, then the end).
    fn dom_rack(&mut self, ui: &mut egui::Ui, s: &ServerView, v: &DomView, rack: Rect, now: Instant) {
        let p = ui.painter().clone();
        let h = v.hand.as_ref().filter(|_| v.stage == DomStage::Playing);
        let mine = h.map(|h| h.mine.clone()).unwrap_or_default();
        if v.my_seat.is_none() {
            p.text(rack.center(), Align2::CENTER_CENTER, "Watching: sit in an open seat to play", theme::font(14.0), col::MUTED);
            return;
        }
        let Some(h) = h.filter(|_| !mine.is_empty()) else {
            p.text(rack.center(), Align2::CENTER_CENTER, "Your tiles come here when the hand's dealt", theme::font(14.0), col::MUTED);
            return;
        };
        p.text(rack.left_top() + vec2(16.0, 9.0), Align2::LEFT_TOP, format!("YOUR TILES · {} pips", pip_count(&mine)), theme::bold_font(12.0), col::GOLD2);
        // (Asked to answer a ¡Doblo! isn't playing a tile: my tiles stay as they are, as
        // 0.13.28's do, side by side.)
        let my_turn = h.to_act == v.my_seat && h.over.is_none() && h.offer.is_none();
        let plays: Vec<Move> = h.allowed.iter().copied().filter(|m| matches!(m, Move::Play { .. })).collect();
        if my_turn && h.offer.is_none() {
            let words = if plays.is_empty() {
                "Nothing fits"
            } else if self.poker.casino.domt.picked.is_some() {
                "Now pick its end, on the line"
            } else {
                "Your turn: pick a tile"
            };
            p.text(pos2(rack.right() - 16.0, rack.top() + 9.0), Align2::RIGHT_TOP, words, theme::bold_font(12.5), col::GOLD2);
        }
        let (w, ht) = (46.0, 92.0);
        let pitch = 57.5;
        let total = mine.len() as f32 * pitch - (pitch - w);
        let x0 = rack.center().x - total / 2.0;
        let y = rack.top() + 19.0;
        for (i, t) in mine.iter().enumerate() {
            let r = Rect::from_min_size(pos2(x0 + i as f32 * pitch, y), vec2(w, ht));
            let ends: Vec<End> = plays.iter().filter_map(|m| if let Move::Play { tile, end } = m { (tile == t).then_some(*end) } else { None }).collect();
            let ok = my_turn && !ends.is_empty();
            let picked = self.poker.casino.domt.picked == Some(*t);
            let resp = ui.interact(r, ui.id().with(("dom-rack", i)), Sense::click());
            // (One that goes: up out of the rack, ringed in gold, the rest dimmed; the one picked
            // for an end higher, ringed orange.)
            let r = r.translate(vec2(0.0, if picked { -14.0 } else if ok { -8.0 } else { 0.0 }));
            if ok {
                super::radial(&p, r.center(), ht * 0.75, col::GOLD.gamma_multiply(0.16), Color32::TRANSPARENT);
            }
            tile(&p, r, false, t[0], t[1], if my_turn && !ok { 1.0 } else { 0.0 });
            if ok {
                p.rect_stroke(r.expand(3.5), theme::r(8), Stroke::new(3.0f32, if picked { col::EMBER } else if resp.hovered() { col::GOLD2 } else { col::GOLD }), StrokeKind::Middle);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    // One that goes on either end: the end's picked next, on the line. (With both
                    // ends the same, 0.13.28's server offers only the left.)
                    if ends.len() > 1 {
                        self.poker.casino.domt.picked = if picked { None } else { Some(*t) };
                    } else {
                        self.dom_send(s, v, Move::Play { tile: *t, end: ends[0] });
                    }
                }
            }
        }
        let _ = now;
    }

    /// The table's sounds, as 0.13.28 makes them (heard off it at Café Habana, watching and
    /// sitting; dominó's own made sample for sample): the tiles shuffled as a hand starts; each
    /// tile as it lands, placed toward whoever laid it (the first one harder, the one that ends
    /// the hand slammed down); a pass the table's knock, a ¡Doblo! its chips shoved in, an accept
    /// its chips; my turn come round, its bells. Nothing for a fold, or for anything that happened
    /// before I came. (My own moves sound as they come back, as everyone's do.)
    fn dom_sounds(&mut self, v: &DomView) {
        let moves = v.hand.as_ref().map_or(0, |h| h.moves);
        let Some((id, round, stage, seen)) = self.poker.casino.domt.heard.replace((v.id, v.round_no, v.stage, moves)) else { return };
        if id != v.id {
            return;
        }
        let casino = &mut self.poker.casino;
        if v.stage == DomStage::Playing && (round != v.round_no || stage != DomStage::Playing) {
            // A hand starting (one already under way when it's first seen: nothing).
            if moves == 0 {
                casino.sound_in(0.0, Cue::DomShuffle, 0.0, 0.4);
            }
            return;
        }
        let Some(h) = &v.hand else { return };
        if round != v.round_no || stage != DomStage::Playing || moves <= seen {
            return;
        }
        // (Moves missed between two looks: the last one sounds.)
        let Some((seat, mv)) = h.last else { return };
        let place = (seat as usize + SEATS - v.my_seat.unwrap_or(0) as usize) % SEATS;
        match mv {
            Move::Play { .. } => {
                let (cue, vol) = if h.over.is_some() {
                    (Cue::DomSlam, 1.0)
                } else if h.chain.len() == 1 {
                    (Cue::DomOpen, 0.75)
                } else {
                    (Cue::DomTile, 0.75)
                };
                casino.sound_in(LANDS, cue, PAN_TILE[place], vol);
            }
            Move::Pass => casino.sound_in(0.0, Cue::Check, PAN_TABLE[place], V_PASS),
            Move::Double => casino.sound_in(0.0, Cue::AllIn, PAN_TABLE[place], V_DOUBLE),
            Move::Accept => casino.sound_in(0.0, Cue::Chips, PAN_TABLE[place], V_ACCEPT),
            // (Solo's draw from the pozo: a quick bright slide, heard off 0.13.28 at once, made
            // fresh each time; as loud as its, 25 draws side by side, a touch darker.)
            Move::Draw => casino.sound_in(0.0, Cue::Deal, PAN_TABLE[place], V_DRAW),
            Move::Fold => {}
        }
        // My turn come round (not as a hand starts, nor while a double's being answered).
        if h.over.is_none() && h.offer.is_none() && v.my_seat.is_some_and(|m| h.to_act == Some(m) && h.seats.contains(&m)) {
            // (0.13.28's: 0.51-0.63 s after the move's view, side by side.)
            casino.sound_in(0.5, Cue::Turn, 0.0, V_TURN);
        }
    }

    /// Sends a move (once for where the hand's got to).
    fn dom_send(&mut self, s: &ServerView, v: &DomView, m: Move) {
        let Some(h) = &v.hand else { return };
        let at = (v.round_no, h.moves, h.offer.is_some(), m);
        if self.poker.casino.domt.sent == Some(at) {
            return;
        }
        self.poker.casino.domt.sent = Some(at);
        self.poker.casino.domt.picked = None;
        self.engine.poker(&s.key, ToServer::DomMove { table: v.id, mv: m });
    }

    /// The controls along the bottom: Sit down (watching); what I put up, and Deal me in
    /// (between hands); my moves on my turn (Pass or Draw, ¡Doblo!, an offer's Accept or Fold);
    /// emotes and Stand up always.
    fn dom_controls(&mut self, ui: &mut egui::Ui, s: &ServerView, live: &pyr_engine::poker::SharedLive<DomView>, bar: Rect, clock: u64, now: Instant) {
        let p = ui.painter().clone();
        let v = &live.view;
        p.rect_filled(bar, theme::r(18), Color32::from_black_alpha(216));
        p.rect_stroke(bar, theme::r(18), Stroke::new(1.0f32, Color32::from_white_alpha(22)), StrokeKind::Inside);
        let typing = ui.ctx().wants_keyboard_input();
        let go = super::keys::pressed(ui.ctx(), super::keys::Move::DomGo) && !typing;
        let say = |text: String, color: Color32| {
            p.text(pos2(bar.left() + 22.0, bar.center().y), Align2::LEFT_CENTER, text, theme::bold_font(15.0), color);
        };
        let kind = if v.kind == Kind::Pairs { "Pairs" } else { "Solo" };
        let Some(me) = v.my_seat else {
            let free = v.seats.iter().position(|x| x.is_none());
            say(
                match free {
                    Some(_) => format!("{}: {kind} · ante {}. Sit in an open seat to play.", v.name, chips_text(v.ante)),
                    None => format!("{} is full: watch, or try another table.", v.name),
                },
                col::MUTED,
            );
            if let Some(seat) = free {
                let r = Rect::from_min_size(pos2(bar.right() - 176.0, bar.center().y - 23.0), vec2(160.0, 46.0));
                if button(ui, r, "sit", "Sit down", None, col::GOLD, true) {
                    self.engine.poker(&s.key, ToServer::DomSit { table: v.id, seat: seat as u8 });
                }
            }
            return;
        };
        let h = v.hand.as_ref();
        let in_hand = h.is_some_and(|h| h.seats.contains(&me) && h.over.is_none()) && v.stage == DomStage::Playing;
        // Stand up, at the far right always (after this hand, in one).
        let up = Rect::from_min_size(pos2(bar.right() - 14.0 - 112.0, bar.center().y - 23.0), vec2(112.0, 46.0));
        if button(ui, up, "stand", "Stand up", in_hand.then(|| "after this hand".to_string()), Color32::from_white_alpha(30), true) {
            self.engine.poker(&s.key, ToServer::DomLeave { table: v.id });
            self.poker.casino.domt.picked = None;
            if !in_hand {
                return;
            }
        }
        let em = Rect::from_min_size(pos2(up.left() - 12.0 - 56.0, bar.center().y - 23.0), vec2(56.0, 46.0));
        if let Some(e) = super::emotes::button(ui, em, "dom-emotes", &mut self.poker.casino.emote_last) {
            self.engine.poker(&s.key, ToServer::GameEmote { game: TableGame::Domino, table: v.id, emote: e });
        }
        let right = em.left() - 10.0;
        if in_hand && let Some(h) = h {
            // A ¡Doblo! on the table (answered on the felt, by whoever's on its side). Asked to
            // answer it myself, my time runs along the top as on my turn (0.13.28's).
            if h.offer.is_some() {
                say("A double is on the table".to_string(), col::MUTED);
                if h.to_act == Some(me) {
                    let frac = (clock as f32 / TURN_MS as f32).clamp(0.0, 1.0);
                    let line = Rect::from_min_size(bar.left_top() + vec2(18.0, -1.0), vec2((bar.width() - 36.0) * frac, 3.0));
                    p.rect_filled(line, theme::r(2), if frac > 0.3 { col::GOLD } else { col::LOSE });
                    ui.ctx().request_repaint_after(Duration::from_millis(200));
                }
                return;
            }
            let my_turn = h.to_act == Some(me);
            if !my_turn {
                let who = h.to_act.and_then(|seat| v.seats.get(seat as usize).cloned().flatten()).map(|x| x.name).unwrap_or_default();
                let words = if h.offer.is_some() { format!("¡Doblo!: {who} to answer") } else { format!("{who} to play") };
                say(words, col::MUTED);
                return;
            }
            let secs = clock.div_ceil(1000);
            say(format!("Your turn · {secs}s"), col::GOLD2);
            // My time: a line along the top of the controls.
            let frac = (clock as f32 / TURN_MS as f32).clamp(0.0, 1.0);
            let line = Rect::from_min_size(bar.left_top() + vec2(18.0, -1.0), vec2((bar.width() - 36.0) * frac, 3.0));
            p.rect_filled(line, theme::r(2), if frac > 0.3 { col::GOLD } else { col::LOSE });
            ui.ctx().request_repaint_after(Duration::from_millis(200));
            let mut x = right;
            let mut place = |w: f32| {
                let r = Rect::from_min_size(pos2(x - w, bar.center().y - 23.0), vec2(w, 46.0));
                x -= w + 10.0;
                r
            };
            // Nothing fits: pass (or draw, in Solo), gold beside ¡Doblo!.
            for (m, words) in [(Move::Draw, "Draw"), (Move::Pass, "Pass")] {
                let sub = if m == Move::Draw { format!("from the pozo ({})", h.pool) } else { "nothing fits".to_string() };
                if h.allowed.contains(&m) && (button(ui, place(200.0), words, words, Some(super::keys::with_key_dot(&sub, super::keys::Move::DomGo)), col::GOLD, true) || go) {
                    self.dom_send(s, v, m);
                }
            }
            // (The table lists a ¡Doblo! past what everyone in the hand covers, and refuses it:
            // 0.13.28 shows it only as far as they cover.)
            if h.allowed.contains(&Move::Double) && h.mult * 2 <= v.cap {
                let stake = v.ante * h.mult as u64 * 2;
                if button(ui, place(170.0), "double", &format!("¡Doblo! ×{}", h.mult * 2), Some(format!("stake to {}", chips_text(stake))), DOBLO, true) {
                    self.dom_send(s, v, Move::Double);
                }
            }
            return;
        }
        // Between hands: what I put up (covering my doubles), and Deal me in.
        let mine = v.seats.get(me as usize).cloned().flatten();
        let ready = mine.as_ref().is_some_and(|x| x.ready);
        let chips = chips_now(s);
        let cover = self.poker.casino.domt.cover.unwrap_or(2);
        let x0 = right;
        let deal = Rect::from_min_size(pos2(x0 - 220.0, bar.center().y - 23.0), vec2(220.0, 46.0));
        let settled = v.stage == DomStage::Settled || (v.stage == DomStage::Playing && h.is_some());
        // What's said at the left (shorter where the covers would run over it: 0.13.28's did).
        let room = x0 - 220.0 - 10.0 - 4.0 * 76.0 + 10.0 - (bar.left() + 22.0) - 8.0;
        let words = if settled {
            "Next hand in a moment".to_string()
        } else {
            [format!("Ante {} · cover doubles up to:", chips_text(v.ante)), "Cover doubles up to:".to_string(), "Cover:".to_string()]
                .into_iter()
                .find(|w| p.layout_no_wrap(w.clone(), theme::bold_font(15.0), col::TEXT).size().x <= room)
                .unwrap_or_default()
        };
        say(words, if settled { col::MUTED } else { col::TEXT });
        if ready {
            // In: what I put up, and the deal's coming (its time a line along the top).
            let up = mine.as_ref().map(|x| x.escrow).unwrap_or(0);
            let when = if v.stage == DomStage::Gathering { format!(" · deal in {}s", clock.div_ceil(1000)) } else { String::new() };
            button(ui, deal, "deal", "You're in", Some(format!("{} up{when}", chips_text(up))), col::GOLD, false);
            if v.stage == DomStage::Gathering {
                let frac = (clock as f32 / pyr_poker::domroom::WAIT_MS as f32).clamp(0.0, 1.0);
                p.rect_filled(Rect::from_min_size(bar.left_top() + vec2(18.0, -1.0), vec2((bar.width() - 36.0) * frac, 3.0)), theme::r(2), col::GOLD);
            }
            ui.ctx().request_repaint_after(Duration::from_millis(250));
            return;
        }
        let put = v.ante * cover as u64;
        let ok = put <= chips && !settled;
        let sub = if v.stage == DomStage::Gathering { format!("{} up · deal in {}s", chips_text(put), clock.div_ceil(1000)) } else { format!("put up {}", chips_text(put)) };
        if v.stage == DomStage::Gathering {
            let frac = (clock as f32 / pyr_poker::domroom::WAIT_MS as f32).clamp(0.0, 1.0);
            p.rect_filled(Rect::from_min_size(bar.left_top() + vec2(18.0, -1.0), vec2((bar.width() - 36.0) * frac, 3.0)), theme::r(2), col::GOLD);
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
        if button(ui, deal, "deal", "Deal me in", Some(super::keys::with_key_dot(&sub, super::keys::Move::DomGo)), col::GOLD, ok) || (ok && go) {
            self.engine.dom_ready(&s.key, v.id, put);
            self.poker.casino.sound_in(0.0, Cue::Chips, 0.0, V_DEAL_IN);
        }
        let mut x = deal.left() - 10.0;
        for c in COVERS.iter().rev() {
            let r = Rect::from_min_size(pos2(x - 66.0, bar.center().y - 23.0), vec2(66.0, 46.0));
            x -= 76.0;
            let on = *c == cover;
            let afford = v.ante * *c as u64 <= chips;
            if button(ui, r, &format!("cover{c}"), &format!("×{c}"), Some(chips_text(v.ante * *c as u64)), if on { col::GOLD } else { Color32::from_white_alpha(24) }, afford) {
                self.poker.casino.domt.cover = Some(*c);
            }
        }
        p.text(pos2(bar.center().x, bar.top() - 12.0), Align2::CENTER_CENTER, "What you put up covers your doubles: what the hand doesn't use comes back. Bots play with the house's chips.", theme::font(12.0), col::MUTED);
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
    p.add(Shape::line(pts, stroke));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The line under a hand's end, as 0.13.28's read on its screen (a Capicúa, a Chiva, both
    /// at ×2, and plain ones).
    #[test]
    fn a_hands_end_line_is_0_13_28s() {
        assert_eq!(result_line("Yuneisy & Caridad", 400, 0, 1), "Yuneisy & Caridad · pot 400");
        assert_eq!(result_line("Tía Cuca & Rigo", 400, 50, 1), "Tía Cuca & Rigo · pot 400 · +50% bonus");
        assert_eq!(result_line("Friend & Tata", 400, 100, 1), "Friend & Tata · pot 400 · +100% bonus");
        assert_eq!(result_line("Yoandri & Marlenis", 800, 150, 2), "Yoandri & Marlenis · pot 800 · +150% bonus · ×2");
        assert_eq!(result_line("Olive & Chucho", 1600, 0, 4), "Olive & Chucho · pot 1,600 · ×4");
    }

    /// Dominó's sounds are 0.13.28's, sample for sample: fitted to what it played (every tile, the
    /// opening one, a hand won by a tile, a hand's shuffle), the model's values here.
    #[test]
    fn dominos_sounds_are_0_13_28s() {
        let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let sum = |s: &[f32]| s.iter().map(|x| x.abs() as f64).sum::<f64>();
        for (c, len, pk, at, total) in [
            (Cue::DomTile, 4320, 0.60834, [0.14889, -0.09366, -0.00851], 176.19),
            (Cue::DomOpen, 4320, 0.82473, [0.54977, -0.16938, -0.01265], 305.506),
            (Cue::DomSlam, 10800, 0.9, [0.19889, -0.16344, -0.01384], 438.321),
            (Cue::DomShuffle, 72000, 0.39587, [-0.02087, 0.00286, 0.01345], 1887.604),
        ] {
            let s = pyr_engine::poker_sfx::render_mono(c);
            assert_eq!(s.len(), len, "{c:?}");
            assert!((peak(&s) - pk).abs() < 2e-4, "{c:?} {}", peak(&s));
            for (i, v) in [100, 1000, 3000].into_iter().zip(at) {
                assert!((s[i] - v).abs() < 2e-4, "{c:?} at {i}: {} not {v}", s[i]);
            }
            assert!((sum(&s) - total).abs() < 0.05, "{c:?} {}", sum(&s));
            assert_eq!(s, pyr_engine::poker_sfx::render_mono(c), "{c:?}: the same every time");
        }
    }

    /// The table's sounds' loudness here against 0.13.28's at a dominó table (what it played,
    /// recorded: a pass's knock, a ¡Doblo!'s shove of chips, an accept's chips, my turn's bells;
    /// the energy over each one's first moments, both sides, sounds at 70%): the volume that
    /// matches, by the median of 200 of ours. Run by hand to set the `V_` volumes.
    #[test]
    #[ignore]
    fn table_sound_levels() {
        use pyr_engine::poker_sfx::render_mono;
        let energy = |s: &[f32], n: usize| s.iter().take(n).map(|x| (*x as f64).powi(2)).sum::<f64>();
        for (c, secs, theirs) in [(Cue::Check, 0.3, 12.4), (Cue::AllIn, 0.85, 115.0), (Cue::Chips, 0.25, 11.0), (Cue::Turn, 0.3, 25.3)] {
            let n = (secs * 48000.0) as usize;
            let mut es: Vec<f64> = (0..200).map(|_| energy(&render_mono(c), n)).collect();
            es.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let peak = render_mono(c).iter().fold(0.0f32, |m, x| m.max(x.abs()));
            println!("{c:?}: energy median {:.3} (10% {:.3}, 90% {:.3}), a peak {peak:.3}; volume for 0.13.28's: {:.3}", es[100], es[20], es[180], (theirs / (0.49 * es[100])).sqrt());
        }
    }

    /// A hand 0.13.28 played (Café Habana, round 1), at its last tile but one: every tile where
    /// 0.13.28 had it, measured off its screen (in short sides from the first tile's middle).
    #[test]
    fn the_line_lies_as_0_13_28_lays_it() {
        let chain: Vec<Tile> = vec![
            [4, 8], [8, 9], [9, 5], [5, 5], [5, 6], [6, 8], [8, 8], [8, 0], [0, 7], [7, 6], [6, 9], [9, 9], [9, 3], [3, 8], [8, 1], [1, 7], [7, 3], [3, 6], [6, 4], [4, 7], [7, 7], [7, 2], [2, 4], [4, 0], [0, 1], [1, 3], [3, 0],
            [0, 0], [0, 6], [6, 1], [1, 5], [5, 0], [0, 2], [2, 8],
        ];
        let lead = chain.iter().position(|t| *t == [7, 7]).unwrap();
        let l = layout(&chain, lead);
        let at = |t: Tile| l.tiles[chain.iter().position(|x| *x == t).unwrap()].rect;
        let r = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
        // The first row: the lead crosswise, four tiles each side.
        assert_eq!(at([7, 7]), r(-0.5, -1.0, 0.5, 1.0));
        assert_eq!(at([7, 3]), r(-8.5, -0.5, -6.5, 0.5));
        assert_eq!(at([0, 1]), r(6.5, -0.5, 8.5, 0.5));
        // The left end turns up over its last tile, and comes back along a row above.
        assert_eq!(at([1, 7]), r(-8.5, -2.5, -7.5, -0.5));
        assert!(!l.tiles[chain.iter().position(|x| *x == [1, 7]).unwrap()].across);
        assert_eq!(at([8, 1]), r(-7.5, -2.5, -5.5, -1.5));
        assert_eq!(at([9, 9]), r(-1.5, -3.0, -0.5, -1.0));
        assert_eq!(at([8, 8]), r(7.5, -3.0, 8.5, -1.0));
        // ...and up again over a double, back along a third.
        assert_eq!(at([6, 8]), r(7.5, -5.0, 8.5, -3.0));
        assert_eq!(at([5, 6]), r(5.5, -5.0, 7.5, -4.0));
        assert_eq!(at([4, 8]), r(-1.5, -5.0, 0.5, -4.0));
        // The right end turns down, and comes back below.
        assert_eq!(at([1, 3]), r(7.5, 0.5, 8.5, 2.5));
        assert_eq!(at([0, 0]), r(4.5, 1.0, 5.5, 3.0));
        assert_eq!(at([2, 8]), r(-7.5, 1.5, -5.5, 2.5));
        // The numbers read the way the line runs: the 7 of the 3-7 against the row it leaves.
        let t = l.tiles[chain.iter().position(|x| *x == [7, 3]).unwrap()];
        assert_eq!((t.first, t.second), (7, 3));
        let t = l.tiles[chain.iter().position(|x| *x == [1, 7]).unwrap()];
        assert_eq!((t.first, t.second), (1, 7));
        let t = l.tiles[chain.iter().position(|x| *x == [1, 3]).unwrap()];
        assert_eq!((t.first, t.second), (1, 3));
        let t = l.tiles[chain.iter().position(|x| *x == [2, 8]).unwrap()];
        assert_eq!((t.first, t.second), (8, 2));
        // Where the next tiles go: past the 4-8 on the top row, past the 8-2 on the bottom (0.13.28
        // marked the 8 there, not round the corner: a tile reaching 9.5 doesn't turn).
        assert_eq!(l.next[0], pos2(-2.5, -4.5));
        assert_eq!(l.next[1], pos2(-8.5, 2.0));
    }

    /// The line's middle, up and down, follows 0.13.28's (from its film of the same hand: the
    /// lead's row 0, 0.6, 0.9, 0.3, 0, 1.0, 1.3 short sides down as the line grew).
    #[test]
    fn the_line_sits_in_the_middle_of_the_felt() {
        let l = layout(&[[7, 7]], 0);
        assert_eq!(line_middle(&l), 0.0);
        let l = layout(&[[1, 7], [7, 3], [3, 6], [6, 4], [4, 7], [7, 7]], 5);
        assert!((line_middle(&l) + 0.75).abs() < 0.01);
    }
}
