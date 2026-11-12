//! Reel Row Remastered on screen (0.13.2), as 0.13.28 plays a round: the spin lands, then its
//! screens one after another, each in its time (`timeline`): wins lit and counted, tumbles
//! bursting and dropping, respins, walking wilds, Hot Wilds and Espresso Shots filling their
//! reels, bombs adding up, collectors gathering, Hold & Win on its own board; then the bonus.
//! Every machine's own sounds (`pyr_engine::bands`, 0.13.28's), and its own bombs, coins and
//! wilds (`rr-bombs`, `rr-coins`, `rr-wilds`, 0.13.28's art).

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::bands;
use pyr_engine::poker_sfx::Cue;
use pyr_poker::casino::reelrow;
use pyr_poker::casino::reels::{BLANK, REELS, ROWS, SCATTER, WILD, share};
use pyr_poker::casino::remaster::{self as rm, BOMB, COIN, COLLECTOR, Family, HOARDER, MAJOR, MINI, MINOR, PAYER};
use pyr_poker::net::RoundView;

use super::reel_themes::{Theme, theme};
use super::row_styles::Style;
use super::{col, ease_back, ease_out, glow_text, pics, radial};
use crate::poker::cards::chips_text;
use crate::theme;

/// A moment in a round's show, after the spin has landed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ev {
    /// Screen `k` at rest.
    Screen(usize),
    /// Hot Wilds or Espresso Shots filling their reels on screen `k`.
    Fill(usize),
    /// Screen `k`'s wins lit, and counted.
    Lit(usize),
    /// Screen `k`'s wins bursting (tumbles).
    Burst(usize),
    /// What's left dropping and new cells falling in: screen `k` becoming `k + 1`.
    Drop(usize),
    /// Screen `k` spun in: the cells that didn't stay spin again.
    Respin(usize),
    /// Walking wilds stepping a reel left, into screen `k`.
    Walk(usize),
    /// The bombs adding up; the chain's win multiplied by them.
    Bombs,
    Multiply,
    /// A collector in the spin gathering every coin.
    Collect,
    /// Hold & Win: its splash, its board (`k`: 0 as it starts, then each respin), its end, the
    /// Grand.
    HoldSplash,
    Hold(usize),
    HoldEnd,
    Grand,
    /// Bomb Spins (a free spin's show): its bombs on a win adding to the multiplier; its win
    /// times the multiplier; three scatters, five spins more.
    FreeAdd,
    FreeWin,
    FreeMore,
    /// The reels' part over (the bonus, or the round's end, next).
    Done,
}

/// Bomb Spins, a free spin's: the multiplier before it and after it, its chain's win before
/// the multiplier and what it paid (chips), and whether it brought five spins more.
#[derive(Clone, Copy, Debug, Default)]
pub struct FreeCtx {
    pub before: u32,
    pub after: u32,
    pub base: u64,
    pub paid: u64,
    pub more: bool,
}

/// A round being shown.
pub struct RmShow {
    pub round: RoundView,
    pub o: rm::Outcome,
    /// A free spin of Bomb Spins (its own show, one after another).
    pub free: Option<FreeCtx>,
    /// When the spin's screen comes to rest; the moments after it (seconds from then).
    pub landed: Instant,
    pub events: Vec<(f32, Ev)>,
    pub turbo: bool,
    /// Its end handled (the bonus started, or the round finished).
    pub over: bool,
}

impl RmShow {
    pub fn new(round: RoundView, o: rm::Outcome, landed: Instant, turbo: bool) -> RmShow {
        let events = timeline(&o, turbo, None);
        RmShow { round, o, free: None, landed, events, turbo, over: false }
    }

    /// A free spin of Bomb Spins (`o`: its screens and what it paid).
    pub fn new_free(round: RoundView, o: rm::Outcome, landed: Instant, turbo: bool, free: FreeCtx) -> RmShow {
        let events = timeline(&o, turbo, Some(&free));
        RmShow { round, o, free: Some(free), landed, events, turbo, over: false }
    }

    /// The moment showing at `now` (its place in the list, how long it's been showing and how
    /// long it lasts), once the spin has landed.
    pub fn at(&self, now: Instant) -> Option<(usize, Ev, f32, f32)> {
        if now < self.landed {
            return None;
        }
        let t = (now - self.landed).as_secs_f32();
        let i = self.events.iter().rposition(|(at, _)| *at <= t).unwrap_or(0);
        let (at, ev) = self.events[i];
        let next = self.events.get(i + 1).map(|e| e.0).unwrap_or(at + 1.0);
        Some((i, ev, t - at, (next - at).max(0.01)))
    }

    /// Hold & Win's own board showing (its splash, its respins, what it comes to, the Grand):
    /// the page shows its jackpots in place of the top prize and the machine's name.
    pub fn holding(&self, now: Instant) -> bool {
        matches!(self.at(now), Some((_, Ev::HoldSplash | Ev::Hold(_) | Ev::HoldEnd | Ev::Grand, ..)))
    }

    /// When the reels' part is over (from the landing).
    pub fn done_at(&self) -> Instant {
        self.landed + Duration::from_secs_f32(self.events.last().map(|e| e.0).unwrap_or(0.0))
    }

    /// What's been counted on screen by `now`: the round's parts as their moments pass (the
    /// bonus apart).
    pub fn shown(&self, now: Instant) -> u64 {
        let o = &self.o;
        let Some((i, ..)) = self.at(now) else { return 0 };
        let past = &self.events[..=i];
        if let Some(f) = &self.free {
            if past.iter().any(|e| matches!(e.1, Ev::FreeWin | Ev::Done)) {
                return f.paid;
            }
            return past.iter().filter_map(|e| if let Ev::Lit(k) = e.1 { Some(o.steps[k].wins.iter().map(|w| w.pay).sum::<u64>()) } else { None }).sum();
        }
        if past.iter().any(|e| e.1 == Ev::Done) {
            return o.paid.saturating_sub(o.bonus.as_ref().map(|b| b.paid()).unwrap_or(0));
        }
        let mut ways: u64 = past.iter().filter_map(|e| if let Ev::Lit(k) = e.1 { Some(o.steps[k].wins.iter().map(|w| w.pay).sum::<u64>()) } else { None }).sum();
        if past.iter().any(|e| e.1 == Ev::Multiply) {
            ways = o.ways_paid;
        }
        let mut sum = ways;
        if past.iter().any(|e| e.1 == Ev::Collect) {
            sum += o.collected;
        }
        if past.iter().any(|e| e.1 == Ev::HoldEnd) {
            sum += o.hold_paid;
        }
        if past.iter().any(|e| e.1 == Ev::Grand) {
            sum += o.grand;
        }
        sum
    }
}

/// The reels a screen's Hot Wilds or Espresso Shots newly fill.
/// How long a Hot Wild's or an Espresso Shot's reel takes to fill, its words settling.
const FILL_SECS: f32 = 0.95;

fn filled(fam: Family, s: &rm::Step) -> Vec<usize> {
    match fam {
        Family::Hot => (0..REELS).filter(|&r| s.x[r] > 0).collect(),
        Family::Espresso => (1..REELS - 1).filter(|&r| s.grid[r][0] == WILD && s.held & (0b111 << (r * ROWS)) == 0).collect(),
        _ => Vec::new(),
    }
}

/// Collectors newly landed on Hold & Win's board `k`.
fn gathering(o: &rm::Outcome, k: usize) -> Vec<(usize, usize)> {
    if k == 0 {
        return Vec::new();
    }
    let (a, b) = (&o.hold[k - 1], &o.hold[k]);
    let new = (0..REELS * ROWS).map(|c| (c / ROWS, c % ROWS)).any(|(r, row)| a.grid[r][row] == BLANK && b.grid[r][row] != BLANK);
    (0..REELS * ROWS)
        .map(|c| (c / ROWS, c % ROWS))
        // (A collector, as it lands; a Hoarder, as it lands and on every respin bringing
        // something new; a Payer paying the coins, as it lands.)
        .filter(|&(r, row)| match b.grid[r][row] {
            COLLECTOR | PAYER => a.grid[r][row] == BLANK,
            HOARDER => new,
            _ => false,
        })
        .collect()
}

/// The round's moments, from the landing (Turbo: twice as quick).
pub fn timeline(o: &rm::Outcome, turbo: bool, free: Option<&FreeCtx>) -> Vec<(f32, Ev)> {
    let k = if turbo { 0.5 } else { 1.0 };
    let fam = rm::family(o.machine);
    let mut out = Vec::new();
    let mut t = 0.0f32;
    let mut push = |e: Ev, dur: f32| {
        out.push((t, e));
        t += dur * k;
    };
    if !o.steps.is_empty() {
        for (i, s) in o.steps.iter().enumerate() {
            if i == 0 {
                push(Ev::Screen(0), 0.12);
            } else {
                match fam {
                    Family::Tumble => {}
                    Family::Spotlight => {
                        push(Ev::Walk(i), 0.6);
                        push(Ev::Respin(i), 0.8);
                    }
                    _ => push(Ev::Respin(i), 0.8),
                }
            }
            if !filled(fam, s).is_empty() {
                push(Ev::Fill(i), FILL_SECS);
            }
            let tumbles = fam == Family::Tumble && i + 1 < o.steps.len();
            if !s.wins.is_empty() {
                push(Ev::Lit(i), if tumbles { 0.75 } else { 1.0 });
            }
            if tumbles {
                push(Ev::Burst(i), 0.5);
                push(Ev::Drop(i), 0.32);
            }
        }
        if let Some(f) = free {
            if f.after > f.before {
                push(Ev::FreeAdd, 1.3);
            }
            if f.paid > 0 && f.after >= 2 {
                push(Ev::FreeWin, 1.3);
            }
            if f.more {
                push(Ev::FreeMore, 1.4);
            }
        } else if fam == Family::Tumble && o.mult > 1 {
            let bombs = o.steps.last().map(|s| s.grid.iter().flatten().filter(|&&c| c == BOMB).count()).unwrap_or(0);
            push(Ev::Bombs, 0.5 + 0.28 * bombs as f32);
            push(Ev::Multiply, 1.1);
        }
        if o.collected > 0 {
            push(Ev::Collect, 1.4);
        }
    }
    if !o.hold.is_empty() {
        push(Ev::HoldSplash, 2.4);
        push(Ev::Hold(0), 0.8);
        for i in 1..o.hold.len() {
            push(Ev::Hold(i), if gathering(o, i).is_empty() { 0.95 } else { 1.8 });
        }
        push(Ev::HoldEnd, 1.5);
        if o.grand > 0 {
            push(Ev::Grand, 3.0);
        }
    }
    out.push((t, Ev::Done));
    out
}

/// A machine's own sound (its band, `s_<machine>_<what>`), or one they share (`s_all_<what>`).
pub fn band(machine: u8, what: &str) -> Option<Cue> {
    bands::id(&format!("s_{}_{what}", theme(machine).id)).or_else(|| bands::id(&format!("s_all_{what}"))).map(Cue::Band)
}

/// The sounds of a round's moments, with when each plays (seconds from the landing).
pub fn sounds(show: &RmShow) -> Vec<(f32, Cue, f32)> {
    let o = &show.o;
    let m = o.machine;
    let mut out = Vec::new();
    let mut add = |t: f32, what: &str, vol: f32| {
        if let Some(c) = band(m, what) {
            out.push((t, c, vol));
        }
    };
    for (i, &(t, ev)) in show.events.iter().enumerate() {
        let next = show.events.get(i + 1).map(|e| e.0).unwrap_or(t);
        match ev {
            Ev::Fill(_) => add(t, if rm::family(m) == Family::Frozen { "freeze" } else { "grow" }, 0.8),
            Ev::Lit(k) => {
                let won: u64 = o.steps[k].wins.iter().map(|w| w.pay).sum();
                let x = won as f64 / o.bet.max(1) as f64;
                add(t, if x >= 5.0 { "win2" } else if x >= 1.0 { "win1" } else { "win0" }, 0.75);
            }
            Ev::Burst(_) => add(t, "pop", 0.8),
            Ev::Drop(_) => add(t, "drop", 0.7),
            Ev::Respin(k) => {
                add(t, "spin", 0.5);
                // Frozen wilds landing; a walking wild's step.
                let s = &o.steps[k];
                let new_wilds = (0..REELS * ROWS).any(|c| s.grid[c / ROWS][c % ROWS] == WILD && s.held & (1 << c) == 0);
                if new_wilds && rm::family(m) == Family::Frozen {
                    add(next - 0.05, "freeze", 0.8);
                }
            }
            Ev::Walk(_) => add(t, "step", 0.8),
            Ev::Bombs => {
                let n = o.steps.last().map(|s| s.grid.iter().flatten().filter(|&&c| c == BOMB).count()).unwrap_or(0);
                for j in 0..n {
                    add(t + 0.25 + j as f32 * 0.28 * if show.turbo { 0.5 } else { 1.0 }, "bombadd", 0.7);
                }
            }
            Ev::Multiply => add(t, "multiply", 0.9),
            Ev::FreeAdd => {
                let n = o.steps.last().map(|s| s.grid.iter().flatten().filter(|&&c| c == BOMB).count()).unwrap_or(0);
                for j in 0..n {
                    add(t + 0.25 + j as f32 * 0.28 * if show.turbo { 0.5 } else { 1.0 }, "bombadd", 0.7);
                }
            }
            Ev::FreeWin => add(t, "multiply", 0.9),
            Ev::FreeMore => add(t, "trigger", 0.8),
            Ev::Collect => add(t, "collect", 0.85),
            Ev::HoldSplash => add(t, "trigger", 0.9),
            Ev::Hold(k) if k > 0 => {
                let (a, b) = (&o.hold[k - 1], &o.hold[k]);
                let new: Vec<(usize, usize)> = (0..REELS * ROWS).map(|c| (c / ROWS, c % ROWS)).filter(|&(r, row)| a.grid[r][row] == BLANK && b.grid[r][row] != BLANK).collect();
                let land = t + 0.45 * if show.turbo { 0.5 } else { 1.0 };
                if !new.is_empty() {
                    add(land, "coin", 0.8);
                    add(land + 0.15, "reset", 0.6);
                    if new.iter().any(|&(r, row)| b.grid[r][row] == COIN && b.values[r][row] >= MINI) {
                        add(land + 0.1, "jackpot", 0.8);
                    }
                }
                if !gathering(o, k).is_empty() {
                    add(land + 0.3, "collect", 0.85);
                }
            }
            Ev::HoldEnd => add(t, "win1", 0.8),
            Ev::Grand => add(t, "grand", 1.0),
            _ => {}
        }
    }
    out
}

// What's where on 0.13.28's sheets.

fn bomb_cell(machine: u8) -> usize {
    match theme(machine).id {
        "cosmic" => 0,
        "sugar" => 1,
        "neon" => 2,
        _ => 3,
    }
}

fn coin_cells(machine: u8) -> (usize, usize) {
    match theme(machine).id {
        "deepsea" => (0, 1),
        "goldrush" => (2, 3),
        "tomb" => (4, 5),
        _ => (6, 7),
    }
}

/// A wild machine's own wild (`rr-wilds`): Scoville's pepper, Frostbite's frozen snowman, the
/// Daily Grind's cup, Starlight's feather.
fn wild_cell(machine: u8) -> Option<usize> {
    match theme(machine).id {
        "scoville" => Some(0),
        "frost" => Some(1),
        "grind" => Some(2),
        "cabaret" => Some(3),
        _ => None,
    }
}

/// Cell `i` of a sheet `cols` by `rows`, drawn in `r`.
pub(super) fn sheet(p: &egui::Painter, ctx: &egui::Context, name: &str, cols: usize, rows: usize, i: usize, r: Rect, tint: Color32) -> bool {
    let (cx, cy) = ((i % cols) as f32, (i / cols) as f32);
    let uv = Rect::from_min_max(pos2(cx / cols as f32, cy / rows as f32), pos2((cx + 1.0) / cols as f32, (cy + 1.0) / rows as f32));
    pics::part(p, ctx, name, uv, r, (r.width() * cols as f32 * 1.4) as u32 + 1, tint)
}

/// A machine's round that builds' picture (Play Feature): three of its bombs, or its coin
/// glowing with a Payer's ring.
pub fn builds_icon(p: &egui::Painter, ctx: &egui::Context, machine: u8, r: Rect, tint: Color32) {
    let s = r.width() * 0.62;
    match rm::family(machine) {
        Family::Tumble => {
            for (dx, dy) in [(-0.2f32, 0.12f32), (0.2, 0.12), (0.0, -0.1)] {
                let c = r.center() + vec2(dx * r.width(), dy * r.height());
                sheet(p, ctx, "rr-bombs", 2, 2, bomb_cell(machine), Rect::from_center_size(c, vec2(s, s)), tint);
            }
        }
        _ => {
            radial(p, r.center(), r.width() * 0.5, Color32::from_rgba_unmultiplied(0xb0, 0x60, 0xff, 90), Color32::TRANSPARENT);
            for (dx, dy) in [(-0.18f32, 0.1f32), (0.18, 0.1), (0.0, -0.1)] {
                let c = r.center() + vec2(dx * r.width(), dy * r.height());
                sheet(p, ctx, "rr-coins", 4, 2, coin_cells(machine).0, Rect::from_center_size(c, vec2(s, s)), tint);
            }
        }
    }
}

/// A machine's feature spin's picture (Play Feature): its bomb, its coin, its wild.
pub fn feature_icon(p: &egui::Painter, ctx: &egui::Context, machine: u8, r: Rect, tint: Color32) {
    match rm::family(machine) {
        Family::Tumble => _ = sheet(p, ctx, "rr-bombs", 2, 2, bomb_cell(machine), r.shrink(r.width() * 0.1), tint),
        Family::Hold => _ = sheet(p, ctx, "rr-coins", 4, 2, coin_cells(machine).0, r.shrink(r.width() * 0.1), tint),
        _ => _ = sheet(p, ctx, "rr-wilds", 2, 2, wild_cell(machine).unwrap_or(0), r.shrink(r.width() * 0.08), tint),
    }
}

/// Text with a dark edge, for numbers on symbols.
fn label(p: &egui::Painter, at: Pos2, text: &str, font: egui::FontId, color: Color32) {
    let edge = Color32::from_black_alpha((color.a() as f32 * 0.85) as u8);
    for (dx, dy) in [(-1.5, 0.0), (1.5, 0.0), (0.0, -1.5), (0.0, 1.5), (-1.0, -1.0), (1.0, 1.0), (-1.0, 1.0), (1.0, -1.0)] {
        p.text(at + vec2(dx, dy), Align2::CENTER_CENTER, text, font.clone(), edge);
    }
    p.text(at, Align2::CENTER_CENTER, text, font, color);
}

/// What a coin says: the jackpot's name, or its worth in chips.
/// A word on a ribbon of `color` (a jackpot coin's name, a Payer's, a Hoarder's).
fn ribbon(p: &egui::Painter, at: Pos2, text: &str, size: f32, color: Color32, a: f32) {
    let g = p.layout_no_wrap(text.to_string(), theme::bold_font(size), faded(Color32::WHITE, a));
    let r = Rect::from_center_size(at, g.size() + vec2(size * 0.9, size * 0.12));
    p.rect_filled(r, theme::r(3), faded(color, a));
    p.rect_stroke(r, theme::r(3), Stroke::new(1.0f32, faded(Color32::from_white_alpha(120), a)), StrokeKind::Inside);
    p.galley(r.center() - g.size() / 2.0, g, faded(Color32::WHITE, a));
}

/// A coin's worth on its dark pill.
fn worth_pill(p: &egui::Painter, at: Pos2, text: &str, size: f32, color: Color32, a: f32) {
    let g = p.layout_no_wrap(text.to_string(), theme::bold_font(size), faded(color, a));
    let r = Rect::from_center_size(at, g.size() + vec2(size * 0.7, size * 0.1));
    p.rect_filled(r, theme::r((r.height() / 2.0) as u8), Color32::from_black_alpha((215.0 * a) as u8));
    p.galley(r.center() - g.size() / 2.0, g, faded(color, a));
}

fn coin_text(value: u32, bet: u64) -> String {
    match value {
        MINI => "MINI".into(),
        MINOR => "MINOR".into(),
        MAJOR => "MAJOR".into(),
        v => chips_text(share(bet, v as u64, 100)),
    }
}

fn faded(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}

/// A cell's symbol in `r`: Reel Row's off the machine's sheet, or Remastered's bombs, coins,
/// collectors and wilds with their numbers. `a`: how bright (dimmed while others win).
#[allow(clippy::too_many_arguments)]
pub fn draw_cell(p: &egui::Painter, ctx: &egui::Context, machine: u8, sym: u8, value: u32, bet: u64, r: Rect, a: f32, accent: Color32) {
    let th = theme(machine);
    let tint = Color32::from_gray((255.0 * a.clamp(0.0, 1.0)) as u8);
    let num = (r.height() * 0.26).clamp(10.0, 40.0);
    match sym {
        BLANK => {
            p.circle_filled(r.center(), (r.width() * 0.03).max(1.5), faded(accent, 0.18 * a));
        }
        BOMB => {
            if sheet(p, ctx, "rr-bombs", 2, 2, bomb_cell(machine), r.shrink(r.width() * 0.04), tint) && value > 0 {
                label(p, r.center() + vec2(0.0, r.height() * 0.02), &format!("{value}×"), theme::bold_font(num), faded(Color32::WHITE, a));
            }
        }
        COIN => {
            if !sheet(p, ctx, "rr-coins", 4, 2, coin_cells(machine).0, r.expand(r.width() * 0.02), tint) {
                return;
            }
            let jackpot = matches!(value, MINI | MINOR | MAJOR);
            let t = coin_text(value, bet);
            if jackpot {
                ribbon(p, pos2(r.center().x, r.bottom() - r.height() * 0.17), &t, num * 0.6, Color32::from_rgb(0xa8, 0x14, 0x24), a);
            } else {
                let size = num * if t.len() > 5 { 0.66 } else { 0.8 };
                worth_pill(p, pos2(r.center().x, r.bottom() - r.height() * 0.17), &t, size, Color32::WHITE, a);
            }
        }
        COLLECTOR => {
            if !sheet(p, ctx, "rr-coins", 4, 2, coin_cells(machine).1, r.shrink(r.width() * 0.02), tint) {
                return;
            }
            // (Its worth on a dark pill, as a coin's: 0.13.28's.)
            if value > 0 {
                worth_pill(p, pos2(r.center().x, r.bottom() - r.height() * 0.17), &coin_text(value, bet), num * 0.8, Color32::WHITE, a);
            }
        }
        // Super Hold & Win's: a Payer, the coin glowing purple with its ribbon; a Hoarder, the
        // collector glowing gold with its.
        PAYER => {
            radial(p, r.center(), r.width() * 0.62, Color32::from_rgba_unmultiplied(0xb0, 0x5a, 0xff, (150.0 * a) as u8), Color32::TRANSPARENT);
            if !sheet(p, ctx, "rr-coins", 4, 2, coin_cells(machine).0, r.expand(r.width() * 0.02), tint) {
                return;
            }
            p.circle_stroke(r.center(), r.width() * 0.47, Stroke::new(2.5f32, faded(Color32::from_rgb(0xc0, 0x7a, 0xff), a)));
            ribbon(p, pos2(r.center().x, r.top() + r.height() * 0.1), "PAYER", num * 0.5, Color32::from_rgb(0x8a, 0x3a, 0xe0), a);
            if value > 0 {
                worth_pill(p, pos2(r.center().x, r.bottom() - r.height() * 0.17), &coin_text(value, bet), num * 0.8, Color32::WHITE, a);
            }
        }
        // (0.13.28's: the collector in an emerald ring, "HOARDER" on an emerald ribbon, its worth
        // on a dark pill.)
        HOARDER => {
            let green = Color32::from_rgb(0x2e, 0xd8, 0x96);
            radial(p, r.center(), r.width() * 0.62, Color32::from_rgba_unmultiplied(0x2e, 0xd8, 0x96, (90.0 * a) as u8), Color32::TRANSPARENT);
            if !sheet(p, ctx, "rr-coins", 4, 2, coin_cells(machine).1, r.shrink(r.width() * 0.02), tint) {
                return;
            }
            p.circle_stroke(r.center(), r.width() * 0.47, Stroke::new(2.5f32, faded(green, a)));
            ribbon(p, pos2(r.center().x, r.top() + r.height() * 0.1), "HOARDER", num * 0.46, Color32::from_rgb(0x1c, 0xb0, 0x78), a);
            if value > 0 {
                worth_pill(p, pos2(r.center().x, r.bottom() - r.height() * 0.17), &coin_text(value, bet), num * 0.8, Color32::WHITE, a);
            }
        }
        WILD if wild_cell(machine).is_some() => {
            if !sheet(p, ctx, "rr-wilds", 2, 2, wild_cell(machine).unwrap(), r.shrink(r.width() * 0.06), tint) {
                return;
            }
            label(p, pos2(r.center().x, r.bottom() - r.height() * 0.12), "WILD", theme::bold_font(num * 0.5), faded(Color32::WHITE, a));
        }
        _ => {
            pics::cell_inset(p, ctx, &format!("reels-{}-symbols", th.id), sym as usize, r, (r.width() * 3.0) as u32, tint, 0.04);
        }
    }
}

/// Where the reels are.
#[derive(Clone, Copy)]
pub struct Board {
    pub reels: Rect,
    pub cell: f32,
    pub gap: f32,
    /// The machine's page (for what covers it: splashes).
    pub area: Rect,
}

impl Board {
    pub fn reel(&self, r: usize) -> Rect {
        Rect::from_min_size(pos2(self.reels.left() + r as f32 * (self.cell + self.gap), self.reels.top()), vec2(self.cell, self.cell * ROWS as f32))
    }

    /// Cell (`r`, `row`): `row` may be a fraction (falling) or above the top.
    pub fn cell_at(&self, r: f32, row: f32) -> Rect {
        Rect::from_min_size(pos2(self.reels.left() + r * (self.cell + self.gap), self.reels.top() + row * self.cell), vec2(self.cell, self.cell)).shrink(self.cell * 0.07)
    }
}

/// A Hot Wild's reel (Scoville's pepper in flames, 0.13.28's), `grow` of the way filled from
/// A Hot Wild's reel (Scoville's pepper in flames, 0.13.28's): the picture rising from the
/// reel's foot over what landed (`t` seconds into its filling; infinite once filled), "HOT
/// WILDS ×N" at its foot while it fills, then its multiplier on top and "HOT WILD" at its foot.
#[allow(clippy::too_many_arguments)]
fn hot_reel(p: &egui::Painter, ctx: &egui::Context, b: &Board, r: usize, x: u8, t: f32, font: &str, time: f32) {
    let full = b.reel(r);
    let rise = ease_out((t / 0.2).clamp(0.0, 1.0));
    let shown = Rect::from_min_max(pos2(full.left(), full.bottom() - full.height() * rise), full.max);
    let clip = p.with_clip_rect(shown);
    pics::cover(&clip, ctx, "rr-reel-hot", full, 640, Color32::WHITE);
    radial(&clip, full.center(), full.width() * (0.9 + 0.08 * (time * 5.0).sin()), Color32::from_rgba_unmultiplied(0xff, 0x80, 0x20, 40), Color32::TRANSPARENT);
    p.rect_stroke(shown, theme::r(6), Stroke::new(2.5f32, col::GOLD), StrokeKind::Inside);
    if t >= 0.5 {
        let pop = ease_back(((t - 0.5) / 0.15).clamp(0.0, 1.0));
        glow_text(p, pos2(full.center().x, full.top() + b.cell * 0.32), Align2::CENTER_CENTER, &format!("×{x}"), theme::display_font(font, (b.cell * 0.3).clamp(16.0, 44.0) * (0.6 + 0.4 * pop)), col::GOLD2, Color32::from_rgba_unmultiplied(0xff, 0x60, 0x10, 160));
    }
    let foot = pos2(full.center().x, full.bottom() - b.cell * 0.16);
    if t < 0.85 {
        let fs = (b.cell * 0.13).clamp(10.0, 18.0);
        let g = p.layout_no_wrap(format!("HOT WILDS ×{x}"), theme::display_font(font, fs), col::GOLD2);
        let pill = Rect::from_center_size(foot, g.size() + vec2(fs * 1.2, fs * 0.45));
        p.rect_filled(pill, theme::r((pill.height() / 2.0) as u8), Color32::from_black_alpha(220));
        p.rect_stroke(pill, theme::r((pill.height() / 2.0) as u8), Stroke::new(1.5f32, col::GOLD), StrokeKind::Inside);
        p.galley(pill.center() - g.size() / 2.0, g, col::GOLD2);
    } else {
        label(p, foot, "HOT WILD", theme::bold_font((b.cell * 0.13).clamp(10.0, 18.0)), Color32::WHITE);
    }
}

/// An Espresso Shot's reel (the Daily Grind's pour, 0.13.28's): the picture rising from the
/// reel's foot over what landed (`t` as `hot_reel`'s), "ESPRESSO" at its foot once filled, and
/// "+1 SPIN" on top on the spin it stays for.
fn shot_reel(p: &egui::Painter, ctx: &egui::Context, b: &Board, r: usize, t: f32, held: bool) {
    let full = b.reel(r);
    let rise = ease_out((t / 0.2).clamp(0.0, 1.0));
    let shown = Rect::from_min_max(pos2(full.left(), full.bottom() - full.height() * rise), full.max);
    let clip = p.with_clip_rect(shown);
    pics::cover(&clip, ctx, "rr-reel-shot", full, 640, Color32::WHITE);
    p.rect_stroke(shown, theme::r(6), Stroke::new(2.0f32, col::GOLD), StrokeKind::Inside);
    let fs = (b.cell * 0.12).clamp(10.0, 17.0);
    let tag = |at: Pos2, t: &str| {
        let gal = p.layout_no_wrap(t.to_string(), theme::bold_font(fs), Color32::BLACK);
        let pill = Rect::from_center_size(at, gal.size() + vec2(14.0, 6.0));
        p.rect_filled(pill, theme::r(10), Color32::from_rgb(0xf0, 0xd8, 0xa0));
        p.galley(pill.center() - gal.size() / 2.0, gal, Color32::BLACK);
    };
    // (While it fills, the note across the reels' foot says it all.)
    if t >= FILL_SECS {
        tag(pos2(full.center().x, full.bottom() - b.cell * 0.14), "ESPRESSO");
    }
    if held {
        tag(pos2(full.center().x, full.top() + b.cell * 0.14), "+1 SPIN");
    }
}

/// A spotlight wild's beam from the top of its reel down to it.
fn spotlight(p: &egui::Painter, b: &Board, at: Rect, a: f32) {
    let top = b.reels.top();
    let w0 = at.width() * 0.16;
    let w1 = at.width() * 0.5;
    let pts = vec![pos2(at.center().x - w0, top), pos2(at.center().x + w0, top), pos2(at.center().x + w1, at.center().y + at.height() * 0.2), pos2(at.center().x - w1, at.center().y + at.height() * 0.2)];
    p.add(Shape::convex_polygon(pts, Color32::from_rgba_unmultiplied(0xff, 0xf4, 0xd8, (46.0 * a) as u8), Stroke::NONE));
    radial(p, at.center(), at.width() * 0.7, Color32::from_rgba_unmultiplied(0xff, 0xf0, 0xc0, (60.0 * a) as u8), Color32::TRANSPARENT);
}

/// A pseudo-random symbol for a cell spinning (the machine's kinds, roughly as often).
fn spinning_sym(machine: u8, seed: u64) -> (u8, u32) {
    let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0x5bd1_e995;
    x ^= x >> 29;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 32;
    let k = (x % 1000) as u32;
    let fam = rm::family(machine);
    let sym = match k {
        0..200 => 0,
        200..370 => 1,
        370..510 => 2,
        510..610 => 3,
        610..670 => 4,
        670..700 => 5,
        700..725 => SCATTER,
        _ => BLANK,
    };
    match fam {
        Family::Tumble if k % 37 == 0 => (BOMB, [2, 3, 5, 10][(x as usize >> 3) % 4]),
        Family::Hold if k % 7 == 0 => (COIN, [50, 100, 200, 500][(x as usize >> 3) % 4]),
        _ => (sym, 0),
    }
}

/// A "WIN 25" tag at the foot of a win.
fn win_tag(p: &egui::Painter, at: Pos2, text: &str, font: &str, size: f32, accent: Color32) {
    let g = p.layout_no_wrap(text.to_string(), theme::display_font(font, size), col::GOLD2);
    let r = Rect::from_center_size(at, g.size() + vec2(30.0, 8.0));
    p.rect_filled(r, theme::r((r.height() / 2.0) as u8), Color32::from_black_alpha(215));
    p.rect_stroke(r, theme::r((r.height() / 2.0) as u8), Stroke::new(1.0f32, faded(accent, 0.8)), StrokeKind::Inside);
    p.galley(r.center() - g.size() / 2.0, g, col::GOLD2);
}

/// A message across the foot of the reels ("ENCORE! A STEP TO THE LEFT").
fn reel_note(p: &egui::Painter, b: &Board, text: &str, font: &str, accent: Color32, a: f32) {
    let g = p.layout_no_wrap(text.to_string(), theme::display_font(font, (b.cell * 0.13).clamp(12.0, 20.0)), faded(col::GOLD2, a));
    let r = Rect::from_center_size(pos2(b.reels.center().x, b.reels.bottom() - b.cell * 0.16), g.size() + vec2(28.0, 10.0));
    p.rect_filled(r, theme::r((r.height() / 2.0) as u8), Color32::from_black_alpha((215.0 * a) as u8));
    p.rect_stroke(r, theme::r((r.height() / 2.0) as u8), Stroke::new(1.0f32, faded(accent, 0.8 * a)), StrokeKind::Inside);
    p.galley(r.center() - g.size() / 2.0, g, faded(col::GOLD2, a));
}

/// The feature's banner at the top of the reels while it's played ("BOMB SPIN / A spin with
/// three bombs on the reels from the start."), as 0.13.28 shows it.
pub fn banner(p: &egui::Painter, b: &Board, title: &str, line: &str, accent: Color32, a: f32) {
    if a <= 0.0 {
        return;
    }
    // (0.13.28's, measured: the feature's name in its colour, its line under it, on a dark box
    // edged in its colour, over the top of the reels, in the middle of the page.)
    let c = b.cell;
    let tg = p.layout_no_wrap(title.to_uppercase(), theme::bold_font((c * 0.303).clamp(18.0, 44.0)), faded(accent, a));
    let lg = p.layout_no_wrap(line.to_string(), theme::bold_font((c * 0.136).clamp(11.0, 19.0)), faded(Color32::WHITE, a));
    let w = tg.size().x.max(lg.size().x) + c * 0.45;
    let r = Rect::from_center_size(pos2(b.area.center().x, b.reels.top() - c * 0.143), vec2(w, c * 0.76));
    p.rect_filled(r, theme::r(14), Color32::from_rgba_unmultiplied(0x18, 0x11, 0x12, (235.0 * a) as u8));
    p.rect_stroke(r, theme::r(14), Stroke::new(2.5f32, faded(accent, a)), StrokeKind::Inside);
    p.galley(pos2(r.center().x - tg.size().x / 2.0, r.top() + c * 0.139), tg.clone(), accent);
    p.galley(pos2(r.center().x - lg.size().x / 2.0, r.top() + c * 0.5), lg, Color32::WHITE);
}

/// How a cell drops in a tumble: (from row, to row) for each cell of reel `r` going from screen
/// `a` (its burst cells gone) to `b`.
fn drops(a: &rm::Step, r: usize) -> ([f32; ROWS], usize) {
    let gone = rm::burst(&a.grid, &a.wins);
    let kept: Vec<usize> = (0..ROWS).rev().filter(|&row| gone & (1 << (r * ROWS + row)) == 0).collect();
    let n_new = ROWS - kept.len();
    let mut from = [0.0f32; ROWS];
    for (i, row) in (0..ROWS).rev().enumerate() {
        from[row] = if i < kept.len() { kept[i] as f32 } else { row as f32 - n_new as f32 };
    }
    (from, n_new)
}

/// Gravity with a little bounce at the end.
fn fall(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    if u < 0.78 { (u / 0.78).powi(2) } else { 1.0 - 0.07 * ((u - 0.78) / 0.22 * std::f32::consts::PI).sin() }
}

/// Every bomb on a screen, with what it's worth.
fn bombs(s: &rm::Step) -> Vec<(usize, usize, u32)> {
    (0..REELS).flat_map(|r| (0..ROWS).map(move |row| (r, row))).filter(|&(r, row)| s.grid[r][row] == BOMB).map(|(r, row)| (r, row, s.values[r][row])).collect()
}

/// The multiplier's badge (top right of the reels): a bomb with the total on it.
fn mult_badge(p: &egui::Painter, ctx: &egui::Context, b: &Board, machine: u8, n: u32, pop: f32) {
    let s = b.cell * 0.62 * (1.0 + 0.25 * (1.0 - ease_back(pop.clamp(0.0, 1.0))));
    let c = pos2(b.reels.right() - b.cell * 0.15, b.reels.top() - b.cell * 0.28);
    let r = Rect::from_center_size(c, vec2(s, s));
    radial(p, c, s * 0.9, Color32::from_rgba_unmultiplied(0xc0, 0x80, 0xff, 70), Color32::TRANSPARENT);
    sheet(p, ctx, "rr-bombs", 2, 2, bomb_cell(machine), r, Color32::WHITE);
    label(p, c, &format!("×{n}"), theme::bold_font((s * 0.34).clamp(12.0, 30.0)), Color32::WHITE);
}

/// The lit cells of a screen's wins, and where its tag goes.
fn lit_cells(s: &rm::Step) -> (u16, Option<(usize, usize)>) {
    let m = rm::burst(&s.grid, &s.wins);
    if m == 0 {
        return (0, None);
    }
    let cells: Vec<(usize, usize)> = (0..REELS * ROWS).filter(|c| m & (1 << c) != 0).map(|c| (c / ROWS, c % ROWS)).collect();
    let longest = s.wins.iter().map(|w| w.count as usize).max().unwrap_or(3);
    let mid = (longest - 1) / 2 + if longest >= 4 { 1 } else { 0 };
    let at = cells.iter().filter(|c| c.0 == mid).max_by_key(|c| c.1).copied().or_else(|| cells.iter().max_by_key(|c| c.1).copied());
    (m, at)
}

/// Draws a screen's cells: `skip` the ones not drawn, `lit` the ones winning (the rest dimmed
/// when any are), `y` each cell's row (falling), `a` each one's brightness.
#[allow(clippy::too_many_arguments)]
fn draw_cells(p: &egui::Painter, ctx: &egui::Context, b: &Board, machine: u8, bet: u64, s: &rm::Step, skip: u16, lit: u16, y: &[[f32; ROWS]; REELS], accent: Color32, time: f32) {
    let fam = rm::family(machine);
    for r in 0..REELS {
        // A whole reel of Hot Wilds or Espresso Shots: drawn as one.
        let full = s.grid[r].iter().all(|&c| c == WILD) && matches!(fam, Family::Hot | Family::Espresso);
        if full && skip & (0b111 << (r * ROWS)) == 0 {
            continue;
        }
        for row in 0..ROWS {
            let bit = 1 << (r * ROWS + row);
            if skip & bit != 0 {
                continue;
            }
            let rect = b.cell_at(r as f32, y[r][row]);
            let on = lit & bit != 0;
            let a = if lit == 0 || on { 1.0 } else { 0.22 };
            if on {
                let pulse = 0.5 + 0.5 * (time * 6.0).sin();
                radial(p, rect.center(), b.cell * 0.62, faded(accent, 0.38 + 0.18 * pulse), Color32::TRANSPARENT);
            }
            let grow = if on { 1.0 + 0.05 * (time * 6.0).sin().abs() } else { 1.0 };
            let rect = Rect::from_center_size(rect.center(), rect.size() * grow);
            if s.grid[r][row] == WILD && fam == Family::Spotlight {
                spotlight(p, b, rect, a);
            }
            draw_cell(p, ctx, machine, s.grid[r][row], s.values[r][row], bet, rect, a, accent);
        }
    }
}

const STILL: [[f32; ROWS]; REELS] = [[0.0, 1.0, 2.0]; REELS];

/// The reels after the spin has landed: the moment showing, drawn.
#[allow(clippy::too_many_arguments)]
pub fn draw(p: &egui::Painter, ctx: &egui::Context, th: &Theme, st: &Style, b: &Board, show: &RmShow, now: Instant, time: f32) {
    let Some((_, ev, since, dur)) = show.at(now) else { return };
    let o = &show.o;
    let m = o.machine;
    let fam = rm::family(m);
    let u = (since / dur).clamp(0.0, 1.0);
    let clip = p.with_clip_rect(b.reels.expand(b.cell * 0.04));
    let accent = th.accent;
    // Hold & Win's own board.
    if matches!(ev, Ev::HoldSplash | Ev::Hold(_) | Ev::HoldEnd | Ev::Grand) || (ev == Ev::Done && !o.hold.is_empty()) {
        draw_hold(p, ctx, th, st, b, show, ev, since, dur, time);
        return;
    }
    let last = o.steps.len().saturating_sub(1);
    // The screen showing, and what's done to it.
    let (k, mut skip, mut lit, mut ys) = (
        match ev {
            Ev::Screen(k) | Ev::Fill(k) | Ev::Lit(k) | Ev::Burst(k) | Ev::Respin(k) | Ev::Walk(k) => k,
            Ev::Drop(k) => k + 1,
            _ => last,
        },
        0u16,
        0u16,
        STILL,
    );
    if o.steps.is_empty() {
        return;
    }
    let s = &o.steps[k];
    // Full reels of Hot Wilds and Espresso Shots, under the cells: a reel newly filled shows
    // what landed (spinning in with the rest on a respin), then fills from its foot.
    let filling = if let Ev::Fill(f) = ev { f == k } else { false };
    let new_full = filled(fam, s);
    let landed = s.landed.unwrap_or(s.grid);
    for r in 0..REELS {
        let full = s.grid[r].iter().all(|&c| c == WILD) && matches!(fam, Family::Hot | Family::Espresso);
        if !full {
            continue;
        }
        let fresh = new_full.contains(&r);
        if fresh && ev == Ev::Respin(k) {
            continue;
        }
        let t = if fresh && filling {
            since
        } else if fresh && matches!(ev, Ev::Screen(_)) {
            -1.0
        } else {
            f32::INFINITY
        };
        if t < 0.2 {
            for row in 0..ROWS {
                draw_cell(&clip, ctx, m, landed[r][row], 0, o.bet, b.cell_at(r as f32, row as f32), 1.0, accent);
            }
        }
        if t < 0.0 {
            continue;
        }
        match fam {
            Family::Hot => hot_reel(&clip, ctx, b, r, s.x[r], t, st.font, time),
            _ => shot_reel(&clip, ctx, b, r, t, s.held & (0b111 << (r * ROWS)) != 0),
        }
    }
    // (New shots: their note across the foot while they fill, 0.13.28's.)
    if filling && fam == Family::Espresso {
        reel_note(p, b, "ESPRESSO SHOT! IT STAYS A SPIN", st.font, accent, (since * 6.0).min(1.0));
    }
    match ev {
        Ev::Lit(_) => {
            let (l, at) = lit_cells(s);
            lit = l;
            draw_cells(&clip, ctx, b, m, o.bet, s, 0, lit, &ys, accent, time);
            if let Some((r, row)) = at {
                let won: u64 = s.wins.iter().map(|w| w.pay).sum();
                let c = b.cell_at(r as f32, row as f32);
                // (Its foot on the winner's.)
                let size = (b.cell * 0.147).clamp(12.0, 26.0);
                win_tag(p, pos2(c.center().x, c.bottom() - size), &format!("WIN {}", chips_text(won)), st.font, size, accent);
            }
            return;
        }
        Ev::Burst(_) => {
            // (0.13.28's: the board bright again with the winners aglow; a ring of lights round
            // each, in the machine's two colours; the winners swell and fade as the lights fly
            // off; then they're gone, the last lights drifting.)
            let (l, _) = lit_cells(s);
            skip = l;
            draw_cells(&clip, ctx, b, m, o.bet, s, skip, 0, &ys, accent, time);
            for c in 0..REELS * ROWS {
                if l & (1 << c) == 0 {
                    continue;
                }
                let (r, row) = (c / ROWS, c % ROWS);
                let rect = b.cell_at(r as f32, row as f32);
                let ctr = rect.center();
                // The winner: aglow, swelling, fading.
                let fade = 1.0 - ((u - 0.5) / 0.2).clamp(0.0, 1.0);
                if fade > 0.0 {
                    let swell = 1.0 + 0.12 * ((u - 0.16) / 0.2).clamp(0.0, 1.0);
                    if u < 0.5 {
                        radial(&clip, ctr, b.cell * 0.62, faded(accent, 0.45), Color32::TRANSPARENT);
                    }
                    draw_cell(&clip, ctx, m, s.grid[r][row], s.values[r][row], o.bet, Rect::from_center_size(ctr, rect.size() * swell), fade, accent);
                }
                // The lights.
                if u >= 0.16 {
                    let v = ((u - 0.5) / 0.5).clamp(0.0, 1.0);
                    for j in 0..12 {
                        let ang = j as f32 / 12.0 * std::f32::consts::TAU + c as f32 * 0.7;
                        // (Off they fly, each its own way a little.)
                        let wob = ((j * 7 + c * 3) % 5) as f32 * 0.06;
                        let d = b.cell * (0.5 + (0.45 + wob) * ease_out(v));
                        let col = if j % 2 == 0 { st.second } else { accent };
                        let rad = b.cell * 0.035 * (1.0 - 0.5 * v);
                        clip.circle_filled(ctr + vec2(ang.cos(), ang.sin()) * d, rad, faded(col, 0.9 * (1.0 - v * v)));
                    }
                }
            }
            return;
        }
        Ev::Drop(j) => {
            let a = &o.steps[j];
            for r in 0..REELS {
                let (from, _) = drops(a, r);
                for row in 0..ROWS {
                    ys[r][row] = from[row] + (row as f32 - from[row]) * fall(u);
                }
            }
            draw_cells(&clip, ctx, b, m, o.bet, s, 0, 0, &ys, accent, time);
            return;
        }
        Ev::Respin(_) => {
            // What stayed, still; the rest spinning, then landing a reel after another.
            let held = s.held;
            let stop = |r: usize| (0.28 + r as f32 * 0.09) / 0.8;
            for r in 0..REELS {
                let full = s.grid[r].iter().all(|&c| c == WILD) && matches!(fam, Family::Hot | Family::Espresso);
                let reel_held = held & (0b111 << (r * ROWS)) == 0b111 << (r * ROWS);
                if full && reel_held {
                    continue;
                }
                for row in 0..ROWS {
                    let bit = 1 << (r * ROWS + row);
                    if held & bit != 0 {
                        continue;
                    }
                    skip |= bit;
                    let rect = b.cell_at(r as f32, row as f32);
                    // (A new shot: landing as it landed, to fill the reel after.)
                    if full && u >= stop(r) {
                        let k2 = ((u - stop(r)) / 0.12).min(1.0);
                        let sym = s.landed.map(|l| l[r][row]).unwrap_or(WILD);
                        draw_cell(&clip, ctx, m, sym, 0, o.bet, rect.translate(vec2(0.0, -(1.0 - ease_back(k2)) * b.cell * 0.12)), 1.0, accent);
                        continue;
                    }
                    if u < stop(r) {
                        // Spinning: symbols streaming down the cell.
                        let cc = p.with_clip_rect(rect.expand(b.cell * 0.07));
                        let pos = since * 14.0 + row as f32 * 0.37 + r as f32 * 0.61;
                        for d in 0..2 {
                            let n = pos.floor() as u64 + d;
                            let (sym, val) = spinning_sym(m, n.wrapping_mul(31).wrapping_add((r * 7 + row) as u64 + show.round.nonce));
                            let off = (pos.fract() - d as f32) * b.cell;
                            draw_cell(&cc, ctx, m, sym, val, o.bet, rect.translate(vec2(0.0, off)), 0.55, accent);
                        }
                    } else {
                        let k2 = ((u - stop(r)) / 0.12).min(1.0);
                        let bounce = (1.0 - ease_back(k2)) * b.cell * 0.12;
                        draw_cell(&clip, ctx, m, s.grid[r][row], s.values[r][row], o.bet, rect.translate(vec2(0.0, -bounce)), 1.0, accent);
                    }
                }
            }
            draw_cells(&clip, ctx, b, m, o.bet, s, skip, 0, &ys, accent, time);
            return;
        }
        Ev::Walk(_) => {
            // The stage cleared; each walking wild a reel to the left.
            let prev = &o.steps[k - 1];
            for c in 0..REELS * ROWS {
                let (r, row) = (c / ROWS, c % ROWS);
                if prev.grid[r][row] == WILD {
                    let x = r as f32 - ease_out(u);
                    let rect = b.cell_at(x, row as f32);
                    let a = if r == 0 { 1.0 - u } else { 1.0 };
                    let cc = p.with_clip_rect(b.reels.expand(b.cell * 0.02));
                    spotlight(&cc, b, rect, a);
                    draw_cell(&cc, ctx, m, WILD, 0, o.bet, rect, a, accent);
                } else {
                    let rect = b.cell_at(r as f32, row as f32);
                    draw_cell(&clip, ctx, m, prev.grid[r][row], prev.values[r][row], o.bet, rect, (1.0 - u * 1.6).max(0.0), accent);
                }
            }
            reel_note(p, b, "ENCORE! A STEP TO THE LEFT", st.font, accent, (u * 4.0).min(1.0));
            return;
        }
        Ev::FreeAdd | Ev::FreeWin | Ev::FreeMore if show.free.is_some() => {
            let f = show.free.unwrap();
            draw_cells(&clip, ctx, b, m, o.bet, s, 0, 0, &ys, accent, time);
            let badge_at = pos2(b.reels.right() - b.cell * 0.15, b.reels.top() - b.cell * 0.28);
            // (0.13.28's notes during Bomb Spins: a little higher and bigger than a win's tag.)
            let fs = (b.cell * 0.147).clamp(12.0, 26.0);
            let foot = pos2(b.reels.center().x, b.reels.bottom() - b.cell * 0.23);
            let note = |text: &str, a: f32| {
                let g = p.layout_no_wrap(text.to_string(), theme::display_font(st.font, (b.cell * 0.127).clamp(12.0, 26.0)), faded(col::GOLD2, a));
                let r = Rect::from_center_size(foot, g.size() + vec2(32.0, 12.0));
                p.rect_filled(r, theme::r((r.height() / 2.0) as u8), Color32::from_black_alpha((215.0 * a) as u8));
                p.rect_stroke(r, theme::r((r.height() / 2.0) as u8), Stroke::new(1.0f32, faded(accent, 0.8 * a)), StrokeKind::Inside);
                p.galley(r.center() - g.size() / 2.0, g, faded(col::GOLD2, a));
            };
            match ev {
                Ev::FreeAdd => {
                    // This spin's bombs flying up into the multiplier, which counts up.
                    let per = 0.28 * if show.turbo { 0.5 } else { 1.0 };
                    let mut sum = f.before;
                    for (j, (r, row, v)) in bombs(s).iter().enumerate() {
                        let t0 = 0.25 * if show.turbo { 0.5 } else { 1.0 } + j as f32 * per;
                        let rect = b.cell_at(*r as f32, *row as f32);
                        if since >= t0 + per {
                            sum += v;
                            continue;
                        }
                        if since >= t0 {
                            let q = ((since - t0) / per).clamp(0.0, 1.0);
                            let at = rect.center() + (badge_at - rect.center()) * ease_out(q);
                            radial(p, rect.center(), b.cell * 0.6, faded(accent, 0.5 * (1.0 - q)), Color32::TRANSPARENT);
                            label(p, at, &format!("{v}×"), theme::bold_font((b.cell * 0.2).clamp(12.0, 30.0)), Color32::WHITE);
                        }
                    }
                    if sum >= 2 {
                        mult_badge(p, ctx, b, m, sum.min(f.after), 1.0);
                    }
                    if since > 0.25 {
                        note(&format!("MULTIPLIER ×{} AND BUILDING", f.after), ((since - 0.25) * 5.0).min(1.0));
                    }
                }
                Ev::FreeWin => {
                    mult_badge(p, ctx, b, m, f.after, (since / 0.35).min(1.0));
                    win_tag(p, foot, &format!("{} × {} = {}", chips_text(f.base), f.after, chips_text(f.paid)), st.font, fs, accent);
                }
                _ => {
                    if f.after >= 2 {
                        mult_badge(p, ctx, b, m, f.after, 1.0);
                    }
                    note("3 SCATTERS: 5 MORE SPINS", (since * 5.0).min(1.0));
                }
            }
            return;
        }
        Ev::Bombs | Ev::Multiply => {
            draw_cells(&clip, ctx, b, m, o.bet, s, 0, 0, &ys, accent, time);
            let bs = bombs(s);
            let total: u32 = bs.iter().map(|x| x.2).sum();
            let per = 0.28 * if show.turbo { 0.5 } else { 1.0 };
            let badge_at = pos2(b.reels.right() - b.cell * 0.15, b.reels.top() - b.cell * 0.28);
            if ev == Ev::Bombs {
                let mut sum = 0;
                for (j, (r, row, v)) in bs.iter().enumerate() {
                    let t0 = 0.25 * if show.turbo { 0.5 } else { 1.0 } + j as f32 * per;
                    let rect = b.cell_at(*r as f32, *row as f32);
                    if since >= t0 + per {
                        sum += v;
                        continue;
                    }
                    if since >= t0 {
                        // The bomb's worth flying up to the badge.
                        let q = ((since - t0) / per).clamp(0.0, 1.0);
                        let at = rect.center() + (badge_at - rect.center()) * ease_out(q);
                        radial(p, rect.center(), b.cell * 0.6, faded(accent, 0.5 * (1.0 - q)), Color32::TRANSPARENT);
                        label(p, at, &format!("{v}×"), theme::bold_font((b.cell * 0.2).clamp(12.0, 30.0)), Color32::WHITE);
                    }
                }
                if sum > 0 {
                    mult_badge(p, ctx, b, m, sum, 1.0);
                }
            } else {
                mult_badge(p, ctx, b, m, total, (since / 0.35).min(1.0));
                let won = o.ways_paid;
                win_tag(p, pos2(b.reels.center().x, b.reels.bottom() - b.cell * 0.2), &format!("×{total}  WIN {}", chips_text(won)), st.font, (b.cell * 0.147).clamp(12.0, 26.0), accent);
            }
            return;
        }
        Ev::Collect => {
            let s0 = &o.steps[0];
            draw_cells(&clip, ctx, b, m, o.bet, s0, 0, 0, &ys, accent, time);
            // Every coin's worth flying to the collector(s).
            let coins: Vec<(usize, usize, u32)> = (0..REELS * ROWS).map(|c| (c / ROWS, c % ROWS)).filter(|&(r, row)| s0.grid[r][row] == COIN).map(|(r, row)| (r, row, s0.values[r][row])).collect();
            // (0.13.28's: thin lines of light from every coin to the collector, fading.)
            let a = (u * 5.0).min(1.0) * (1.0 - ((u - 0.7) * 4.0).clamp(0.0, 1.0));
            for row in (0..ROWS).filter(|&row| s0.grid[REELS - 1][row] == COLLECTOR) {
                let to = b.cell_at((REELS - 1) as f32, row as f32).center();
                radial(p, to, b.cell * 0.7, faded(col::GOLD, 0.4 + 0.2 * (time * 6.0).sin()), Color32::TRANSPARENT);
                for (r, rr, _) in &coins {
                    let from = b.cell_at(*r as f32, *rr as f32).center();
                    p.line_segment([from, to], Stroke::new(1.4f32, Color32::from_rgba_unmultiplied(0xe8, 0xf0, 0xb0, (170.0 * a) as u8)));
                }
            }
            if u > 0.7 {
                win_tag(p, pos2(b.reels.center().x, b.reels.bottom() - b.cell * 0.2), &format!("COLLECTED {}", chips_text(o.collected)), st.font, (b.cell * 0.147).clamp(12.0, 26.0), accent);
            }
            return;
        }
        _ => {}
    }
    // At rest (and as the reels' part ends).
    let fill_skip = 0;
    draw_cells(&clip, ctx, b, m, o.bet, s, fill_skip, lit, &ys, accent, time);
    if matches!(ev, Ev::Done) && o.mult > 1 {
        mult_badge(p, ctx, b, m, o.mult, 1.0);
    }
    if let Some(f) = &show.free {
        let x = if matches!(ev, Ev::Done) { f.after } else { f.before };
        if x >= 2 {
            mult_badge(p, ctx, b, m, x, 1.0);
        }
    }
}

/// Bomb Spins' multiplier up top while the reels spin between free spins.
pub fn free_badge(p: &egui::Painter, ctx: &egui::Context, b: &Board, machine: u8, n: u32) {
    if n >= 2 {
        mult_badge(p, ctx, b, machine, n, 1.0);
    }
}

/// Bomb Spins' count, over the reels' top left (0.13.28's: "SPIN 6 / 10").
pub fn free_counter(p: &egui::Painter, b: &Board, index: usize, total: usize, font: &str, accent: Color32) {
    let fs = (b.cell * 0.15).clamp(12.0, 24.0);
    let g = p.layout_no_wrap(format!("SPIN {} / {}", index + 1, total), theme::display_font(font, fs), col::GOLD2);
    let size = g.size() + vec2(fs * 1.4, fs * 0.5);
    let r = Rect::from_min_size(pos2(b.reels.left(), b.reels.top() - b.cell * 0.36 - size.y / 2.0), size);
    p.rect_filled(r, theme::r((r.height() / 2.0) as u8), Color32::from_black_alpha(225));
    p.rect_stroke(r, theme::r((r.height() / 2.0) as u8), Stroke::new(1.8f32, accent), StrokeKind::Inside);
    p.galley(r.center() - g.size() / 2.0, g, col::GOLD2);
}

/// Bomb Spins over: its tag in the middle of the reels (0.13.28's: "BOMB SPINS / 10 spins · ×6
/// built · 142 chips").
pub fn free_summary(p: &egui::Painter, b: &Board, spins: usize, built: u32, chips: u64, font: &str, accent: Color32, a: f32) {
    if a <= 0.0 {
        return;
    }
    // (Measured off 0.13.28's: the name in the machine's colour and lettering, the line in white
    // under it, on a dark tag edged in the colour, between the first and second rows.)
    let fs = (b.cell * 0.133).clamp(14.0, 30.0);
    let tg = p.layout_no_wrap("BOMB SPINS".into(), theme::display_font(font, fs), faded(accent, a));
    let line = format!("{spins} spins · ×{built} built · {} chips", chips_text(chips));
    let lg = p.layout_no_wrap(line, theme::bold_font((b.cell * 0.062).clamp(9.5, 14.0)), faded(Color32::WHITE, a));
    let w = tg.size().x.max(lg.size().x) + fs * 0.9;
    let h = tg.size().y + lg.size().y + fs * 0.72;
    let r = Rect::from_center_size(pos2(b.reels.center().x, b.reels.top() + b.cell), vec2(w, h));
    p.rect_filled(r, theme::r((r.height() * 0.3) as u8), Color32::from_black_alpha((230.0 * a) as u8));
    p.rect_stroke(r, theme::r((r.height() * 0.3) as u8), Stroke::new(2.0f32, faded(accent, a)), StrokeKind::Inside);
    p.galley(pos2(r.center().x - tg.size().x / 2.0, r.top() + fs * 0.36), tg.clone(), accent);
    p.galley(pos2(r.center().x - lg.size().x / 2.0, r.top() + fs * 0.36 + tg.size().y), lg, Color32::WHITE);
}

/// The jackpots' plaques across the top of the page while Hold & Win's board shows (0.13.28's
/// place for the top prize and the machine's name): the Mini, the Minor, the Major and the
/// Grand, each with what it pays at this bet (a progressive machine's Grand: its pool's share).
fn plaques(p: &egui::Painter, ctx: &egui::Context, b: &Board, o: &rm::Outcome, pool_grand: u64) {
    let w = (b.cell * 1.58).min(b.area.width() / 4.4);
    let h = w * 0.458;
    let y = b.area.top() + 6.0 + h / 2.0;
    let names = ["MINI", "MINOR", "MAJOR", "GRAND"];
    // (Each plaque's word in its metal's colour.)
    let inks = [Color32::from_rgb(0xe8, 0xa8, 0x70), Color32::from_rgb(0xdd, 0xe4, 0xee), col::GOLD2, Color32::WHITE];
    let worth = [share(o.bet, MINI as u64, 100), share(o.bet, MINOR as u64, 100), share(o.bet, MAJOR as u64, 100), pool_grand];
    for i in 0..4 {
        let c = pos2(b.reels.center().x + (i as f32 - 1.5) * (w + 11.0), y);
        let r = Rect::from_center_size(c, vec2(w, h));
        sheet(p, ctx, "rr-jackpots", 2, 2, i, r, Color32::WHITE);
        p.text(pos2(c.x, r.top() + h * 0.43), Align2::CENTER_CENTER, names[i], theme::bold_font((h * 0.16).clamp(8.0, 13.0)), inks[i]);
        p.text(pos2(c.x, r.top() + h * 0.66), Align2::CENTER_CENTER, chips_text(worth[i]), theme::bold_font((h * 0.2).clamp(9.0, 16.0)), Color32::WHITE);
    }
}

/// A pill over or under Hold & Win's board: the machine's lettering in gold, its colour round it.
fn board_pill(p: &egui::Painter, r: Rect, accent: Color32) {
    p.rect_filled(r, theme::r((r.height() / 2.0) as u8), Color32::from_black_alpha(225));
    p.rect_stroke(r, theme::r((r.height() / 2.0) as u8), Stroke::new(1.8f32, accent), StrokeKind::Inside);
}

/// Respins left after board `k` (three, back to three whenever anything new lands).
fn respins_left(o: &rm::Outcome, k: usize) -> u8 {
    let mut left = rm::RESPINS;
    for i in 1..=k.min(o.hold.len().saturating_sub(1)) {
        let new = (0..REELS * ROWS).any(|c| o.hold[i - 1].grid[c / ROWS][c % ROWS] == BLANK && o.hold[i].grid[c / ROWS][c % ROWS] != BLANK);
        left = if new { rm::RESPINS } else { left.saturating_sub(1) };
    }
    left
}

/// Hold & Win: its splash; its board with the jackpots over it and the respins left; coins
/// landing and collectors gathering respin by respin; what it all comes to; the Grand.
#[allow(clippy::too_many_arguments)]
fn draw_hold(p: &egui::Painter, ctx: &egui::Context, th: &Theme, st: &Style, b: &Board, show: &RmShow, ev: Ev, since: f32, dur: f32, time: f32) {
    let o = &show.o;
    let m = o.machine;
    let u = (since / dur).clamp(0.0, 1.0);
    let accent = th.accent;
    if ev == Ev::HoldSplash {
        // (The splash itself goes over the whole page: `splash`. Behind it, the board waiting.)
        if since < 0.5 {
            plaques(p, ctx, b, o, grand_pays(o));
        }
        return;
    }
    let k = match ev {
        Ev::Hold(k) => k,
        _ => o.hold.len() - 1,
    };
    let board = &o.hold[k];
    let prev = if k > 0 { Some(&o.hold[k - 1]) } else { None };
    let land_at = 0.45 * if show.turbo { 0.5 } else { 1.0 } / dur.max(0.01);
    let landed = !matches!(ev, Ev::Hold(_)) || k == 0 || u >= land_at;
    // The jackpots, and the respins left (on the machine's name's place), while it plays; once
    // it's over, the page as it was, with what Hold & Win left on the reels.
    let over = ev == Ev::Done;
    let fs = (b.cell * 0.175).clamp(12.0, 22.0);
    if !over {
        plaques(p, ctx, b, o, grand_pays(o));
        let left = if landed { respins_left(o, k) } else { respins_left(o, k.saturating_sub(1)).saturating_sub(1) };
        let g = p.layout_no_wrap("RESPINS".into(), theme::display_font(st.font, fs), col::GOLD2);
        let d = fs * 0.9;
        let w = g.size().x + 12.0 + 3.0 * d + 2.0 * d * 0.53 + fs * 1.9;
        let pr = Rect::from_center_size(pos2(b.reels.center().x, b.reels.top() - b.cell * 0.31), vec2(w, g.size().y + fs * 0.5));
        board_pill(p, pr, accent);
        let tx = pr.left() + fs * 0.85;
        p.galley(pos2(tx, pr.center().y - g.size().y / 2.0), g.clone(), col::GOLD2);
        for i in 0..rm::RESPINS {
            let c = pos2(tx + g.size().x + 12.0 + d / 2.0 + i as f32 * d * 1.53, pr.center().y);
            if i < left {
                radial(p, c, d * 0.85, faded(col::GOLD, 0.55), Color32::TRANSPARENT);
                p.circle_filled(c, d / 2.0, col::GOLD2);
            } else {
                p.circle_stroke(c, d / 2.0 - 1.0, Stroke::new(1.5f32, faded(col::GOLD2, 0.45)));
            }
        }
    }
    // The board.
    for r in 0..REELS {
        for row in 0..ROWS {
            let cell = b.cell_at(r as f32, row as f32).expand(b.cell * 0.025);
            p.rect_filled(cell, theme::r(8), Color32::from_rgba_unmultiplied(0x02, 0x06, 0x0c, 235));
            p.rect_stroke(cell, theme::r(8), Stroke::new(1.2f32, faded(accent, 0.42)), StrokeKind::Inside);
        }
    }
    // A respin's moments once its places land: Payers paying every coin, then collectors and
    // Hoarders gathering (Super Hold & Win; Hold & Win's collectors gather in the second part).
    let is_hold_k = matches!(ev, Ev::Hold(_)) && k > 0;
    let gatherers: Vec<(usize, usize)> = if is_hold_k { gathering(o, k).into_iter().filter(|&(r, row)| board.grid[r][row] != PAYER).collect() } else { Vec::new() };
    let new_payers: Vec<(usize, usize)> = if is_hold_k { gathering(o, k).into_iter().filter(|&(r, row)| board.grid[r][row] == PAYER).collect() } else { Vec::new() };
    let paid_now: u32 = new_payers.iter().map(|&(r, row)| board.values[r][row]).sum();
    let phase = 0.35 * if show.turbo { 0.5 } else { 1.0 } / dur.max(0.01);
    let pay_q = if new_payers.is_empty() { 1.0 } else { ((u - land_at) / phase).clamp(0.0, 1.0) };
    let gat_from = if new_payers.is_empty() { land_at } else { land_at + phase };
    let gat_q = if gatherers.is_empty() { 1.0 } else { ((u - gat_from) / phase).clamp(0.0, 1.0) };
    for c in 0..REELS * ROWS {
        let (r, row) = (c / ROWS, c % ROWS);
        let rect = b.cell_at(r as f32, row as f32);
        let was_empty = prev.is_some_and(|pv| pv.grid[r][row] == BLANK);
        if !landed && was_empty {
            // Spinning: lines rushing down through the place.
            let cc = p.with_clip_rect(rect.expand(b.cell * 0.02));
            for j in 0..3 {
                let y = rect.top() + ((since * 5.0 + j as f32 / 3.0 + c as f32 * 0.11).fract()) * rect.height();
                cc.line_segment([pos2(rect.left() + 4.0, y), pos2(rect.right() - 4.0, y)], Stroke::new(1.2f32, faded(accent, 0.38)));
            }
            continue;
        }
        let sym = board.grid[r][row];
        if sym == BLANK {
            continue;
        }
        let fresh = was_empty && landed;
        let pop = if fresh { ease_back(((u - land_at) / 0.15).clamp(0.0, 1.0)) } else { 1.0 };
        let rr = Rect::from_center_size(rect.center(), rect.size() * (0.6 + 0.4 * pop));
        // What it shows: a coin, its worth before the Payers' till they've paid; a collector
        // or a Hoarder gathering, what it had till the coins have flown to it.
        let gathering_here = gatherers.contains(&(r, row));
        let val = if sym == COIN && pay_q < 1.0 {
            board.values[r][row].saturating_sub(paid_now)
        } else if gathering_here && gat_q < 1.0 {
            if was_empty { 0 } else { prev.map(|pv| pv.values[r][row]).unwrap_or(0) }
        } else {
            board.values[r][row]
        };
        if fresh {
            radial(p, rect.center(), b.cell * 0.7, faded(col::GOLD, 0.45 * (1.0 - (u - land_at).max(0.0) * 2.0)), Color32::TRANSPARENT);
        }
        if gathering_here && gat_q > 0.0 && gat_q < 1.0 {
            radial(p, rect.center(), b.cell * 0.75, faded(col::GOLD, 0.5), Color32::TRANSPARENT);
        }
        draw_cell(p, ctx, m, sym, val, o.bet, rr, 1.0, accent);
    }
    // The Payers' worth flying to every coin; the coins' worth flying to what gathers them.
    let fly = theme::bold_font((b.cell * 0.12).clamp(10.0, 18.0));
    if pay_q > 0.0 && pay_q < 1.0 {
        for &(pr, prow) in &new_payers {
            let from = b.cell_at(pr as f32, prow as f32).center();
            for c2 in 0..REELS * ROWS {
                let (r2, row2) = (c2 / ROWS, c2 % ROWS);
                if board.grid[r2][row2] != COIN {
                    continue;
                }
                let to = b.cell_at(r2 as f32, row2 as f32).center();
                label(p, from + (to - from) * ease_out(pay_q), &format!("+{}", coin_text(board.values[pr][prow], o.bet)), fly.clone(), Color32::from_rgb(0xd8, 0xa8, 0xff));
            }
        }
    }
    // (0.13.28's: thin lines of light from every piece gathered to what gathers it, fading.)
    if gat_q > 0.0 && gat_q < 1.0 {
        let a = (gat_q * 4.0).min(1.0) * (1.0 - ((gat_q - 0.75) * 4.0).clamp(0.0, 1.0));
        for &(gr, grow) in &gatherers {
            let to = b.cell_at(gr as f32, grow as f32).center();
            let of: &[u8] = if board.grid[gr][grow] == HOARDER { &[COIN, PAYER, COLLECTOR] } else { &[COIN, PAYER] };
            for c2 in 0..REELS * ROWS {
                let (r2, row2) = (c2 / ROWS, c2 % ROWS);
                if !of.contains(&board.grid[r2][row2]) || (r2, row2) == (gr, grow) {
                    continue;
                }
                let from = b.cell_at(r2 as f32, row2 as f32).center();
                p.line_segment([from, to], Stroke::new(1.4f32, Color32::from_rgba_unmultiplied(0xe8, 0xf0, 0xb0, (170.0 * a) as u8)));
            }
        }
    }
    // What it's come to so far.
    let total: u64 = board.values.iter().flatten().map(|&v| v as u64).sum();
    let shown = if landed { total } else { prev.map(|pv| pv.values.iter().flatten().map(|&v| v as u64).sum()).unwrap_or(0) };
    let name = if o.feature == pyr_poker::casino::feature::Feature::Builds { "SUPER HOLD & WIN" } else { "HOLD & WIN" };
    let text = format!("{name} {}", chips_text(share(o.bet, shown, 100)));
    let g = p.layout_no_wrap(text, theme::display_font(st.font, fs), col::GOLD2);
    let pr = Rect::from_center_size(pos2(b.reels.center().x, b.reels.bottom() + b.cell * 0.19), g.size() + vec2(fs * 1.75, fs * 0.5));
    board_pill(p, pr, if ev == Ev::HoldEnd { faded(accent, 0.6 + 0.4 * (time * 6.0).sin().abs()) } else { accent });
    p.galley(pr.center() - g.size() / 2.0, g, col::GOLD2);
    if ev == Ev::Grand {
        let a = (since / 0.3).min(1.0);
        let r = Rect::from_center_size(b.reels.center(), vec2(b.cell * 3.2, b.cell * 1.5) * (0.8 + 0.2 * ease_back(a)));
        p.rect_filled(b.area, 0, Color32::from_black_alpha((120.0 * a) as u8));
        sheet(p, ctx, "rr-jackpots", 2, 2, 3, r, Color32::WHITE);
        let name = if reelrow::progressive(m) { reelrow::TOPS[m as usize].name.to_uppercase() } else { "GRAND".to_string() };
        glow_text(p, pos2(r.center().x, r.top() + r.height() * 0.36), Align2::CENTER_CENTER, &name, theme::display_font(st.font, (r.height() * 0.17).clamp(16.0, 40.0)), col::GOLD2, faded(accent, 0.7));
        glow_text(p, pos2(r.center().x, r.top() + r.height() * 0.62), Align2::CENTER_CENTER, &chips_text(o.grand), theme::display_font(st.font, (r.height() * 0.2).clamp(18.0, 48.0)), Color32::WHITE, faded(col::GOLD, 0.6));
    }
}

/// What Hold & Win's Grand pays at a round's bet (a progressive machine's: its pool's share).
fn grand_pays(o: &rm::Outcome) -> u64 {
    if reelrow::progressive(o.machine) { reelrow::top_win(o.machine, o.bet, o.pool) } else { o.bet * rm::GRAND_X }
}

/// Hold & Win's splash (0.13.28's), over the whole page, the bar too: the machine's splash
/// picture under turning rays, "HOLD & WIN", the machine's line for it, "click to start".
pub fn splash(p: &egui::Painter, ctx: &egui::Context, th: &Theme, st: &Style, area: Rect, show: &RmShow, now: Instant, time: f32) {
    let Some((_, Ev::HoldSplash, since, dur)) = show.at(now) else { return };
    let (title, line) = if show.o.feature == pyr_poker::casino::feature::Feature::Builds {
        let (name, what) = super::stake::SlotKind::Rm(show.o.machine).describe(pyr_poker::casino::feature::Feature::Builds);
        (name.to_uppercase(), what)
    } else {
        ("HOLD & WIN".to_string(), feature_words(show.o.machine).0)
    };
    splash_with(p, ctx, th, st, area, &title, &line, since, dur, time);
}

/// A feature's splash over the page: the machine's splash picture under turning rays, the
/// feature's name in the machine's lettering, its line, "click to start".
#[allow(clippy::too_many_arguments)]
pub fn splash_with(p: &egui::Painter, ctx: &egui::Context, th: &Theme, st: &Style, area: Rect, title: &str, line: &str, since: f32, dur: f32, time: f32) {
    let r = area;
    let a = (since / 0.4).min(1.0) * (1.0 - ((since - dur + 0.4) / 0.4).clamp(0.0, 1.0));
    if a <= 0.0 {
        return;
    }
    let p = p.with_clip_rect(r);
    p.rect_filled(r, 0, Color32::from_black_alpha((235.0 * a) as u8));
    pics::cover(&p, ctx, &format!("rr-{}-splash", th.id), r, 1376, Color32::from_gray((255.0 * a) as u8));
    // Rays, turning slowly, from behind the title.
    let title_c = pos2(r.center().x, r.top() + r.height() * 0.217);
    let reach = r.width().max(r.height()) * 1.2;
    for i in 0..24 {
        if i % 2 == 1 {
            continue;
        }
        let a0 = (i as f32 / 24.0) * std::f32::consts::TAU + time * 0.08;
        let a1 = a0 + std::f32::consts::TAU / 24.0;
        let pts = vec![title_c, title_c + vec2(a0.cos(), a0.sin()) * reach, title_c + vec2(a1.cos(), a1.sin()) * reach];
        p.add(Shape::convex_polygon(pts, Color32::from_white_alpha((16.0 * a) as u8), Stroke::NONE));
    }
    let size = (r.height() * 0.118).clamp(32.0, 96.0);
    glow_text(&p, title_c + vec2(0.0, size * 0.04), Align2::CENTER_CENTER, title, theme::display_font(st.font, size), faded(Color32::BLACK, 0.7 * a), Color32::TRANSPARENT);
    glow_text(&p, title_c, Align2::CENTER_CENTER, title, theme::display_font(st.font, size), faded(col::GOLD2, a), faded(th.accent, 0.6 * a));
    let line = line.to_string();
    let fs = (r.height() * 0.0242).clamp(12.0, 18.0);
    let g = p.layout(line, theme::bold_font(fs), faded(Color32::WHITE, a), (r.width() - 120.0).min(fs * 45.0));
    let box_c = pos2(r.center().x, r.top() + r.height() * 0.323);
    let at = box_c - g.size() / 2.0;
    p.rect_filled(Rect::from_min_size(at, g.size()).expand2(vec2(fs, fs * 0.6)), theme::r(10), Color32::from_black_alpha((175.0 * a) as u8));
    p.galley(at, g, Color32::WHITE);
    p.text(pos2(r.center().x, r.bottom() - 28.0), Align2::CENTER_CENTER, "click to start", theme::font(13.0), faded(col::MUTED, a));
}

/// What a filler cell spinning past shows for a machine (bombs and coins with a made-up worth).
pub fn filler_value(sym: u8, seed: u64) -> u32 {
    match sym {
        BOMB => [2, 3, 4, 5, 8, 10][(seed % 6) as usize],
        COIN => [50, 100, 200, 300, 500][(seed % 5) as usize],
        _ => 0,
    }
}

/// What a price is, said: "2", "1.75" (hundredths of the bet).
fn times(h: u64) -> String {
    if h % 100 == 0 { format!("{}", h / 100) } else { format!("{:.2}", h as f64 / 100.0) }
}

/// A machine's feature in a few words (its floor tile's tooltip and the start of its Info
/// paragraph), and the rest of its rules, in 0.13.28's words.
pub fn feature_words(machine: u8) -> (String, String) {
    let m = &rm::MACHINES[(machine as usize).min(11)];
    let th = theme(machine);
    match m.family {
        Family::Tumble => {
            let (fall, bombs, most) = match th.id {
                "cosmic" => ("new symbols fall in", "Star orbs", 500),
                "sugar" => ("new candy drops in", "Candy bombs", 100),
                "neon" => ("new symbols fall in", "Neon multipliers", 100),
                _ => ("new symbols fall in", "Potions", 100),
            };
            (
                format!("{}: Wins burst and {fall}. {bombs} of 2× to {most}× add up and multiply the whole chain.", m.feature),
                format!("A win's symbols burst and the rest drop, new ones falling in from the top, again and again while there are wins. Bombs land now and then (about one cell in seventy), worth 2× to {most}×: when the tumbling's over, every bomb on the reels adds up, and the whole chain's win is multiplied by the total. Three scatters after the tumbles start the bonus."),
            )
        }
        Family::Hold => {
            let (coins, collector, grand) = match th.id {
                "deepsea" => ("pearls", "The clam", "the Grand"),
                "goldrush" => ("nuggets", "The minecart", "the Mother Lode"),
                "tomb" => ("scarabs", "The Eye of Ra", "the Grand"),
                _ => ("coins", "The dragon", "the Hoard"),
            };
            let end = if reelrow::progressive(machine) { format!("Fill all fifteen places for the progressive jackpot, {}.", reelrow::TOPS[machine as usize].name) } else { "Fill all fifteen places for the Grand, 1000× the bet.".to_string() };
            (
                format!("{}: Six {coins} or more hold for three respins. {collector} on the last reel collects them all. Fill the reels for {grand}.", m.feature),
                format!("Coins land with what they're worth on them (the Mini, Minor and Major among them: 20×, 50× and 200× the bet), and now and then a collector on the last reel, which pays every coin on the reels. Six coins or more start Hold & Win: three respins, every new coin held and the count back to three, a collector landing gathering everything held. {end}"),
            )
        }
        Family::Hot => (
            format!("{}: A wild fills its whole reel with a ×2, ×3 or ×5 on it, multiplying every win through it.", m.feature),
            "A wild landing on reels 2, 3 or 4 fills its whole reel and puts a ×2 (six times in ten), a ×3 or a ×5 on it; wins through two or three Hot Wild reels multiply by all of them.".into(),
        ),
        Family::Frozen => (
            format!("{}: Wilds freeze where they land and the rest of the reels spin again, as long as new wilds keep landing.", m.feature),
            "Wilds freeze where they land and every other cell spins again, again for every new wild, up to twenty respins, paying each time.".into(),
        ),
        Family::Spotlight => (
            format!("{}: A wild takes a step left every respin, paying all the way, till it walks off the stage.", m.feature),
            "A spotlight wild steps one reel to the left each respin while the rest spins again, paying every time, until it walks off the first reel; new wilds join the walk.".into(),
        ),
        Family::Espresso => (
            format!("{}: A wild fills its whole reel and stays for one more spin.", m.feature),
            "A wild on reels 2, 3 or 4 fills its reel and stays for one more spin while the rest spins again; new shots keep it going.".into(),
        ),
    }
}

/// A machine's floor tile's feature badge (0.13.28's words and colours).
pub fn feature_badge(machine: u8) -> (&'static str, Color32) {
    match theme(machine).id {
        "cosmic" => ("TUMBLES · 500× BOMBS", Color32::from_rgb(0x9a, 0x5a, 0xff)),
        "scoville" => ("HOT WILDS ×5", Color32::from_rgb(0xff, 0x5a, 0x2a)),
        "frost" => ("FROZEN WILDS", Color32::from_rgb(0x4a, 0xb8, 0xe8)),
        "grind" => ("ESPRESSO SHOTS", Color32::from_rgb(0x75, 0x36, 0x12)),
        "cabaret" => ("SPOTLIGHT WILDS", Color32::from_rgb(0xe8, 0x3a, 0x6a)),
        _ if rm::family(machine) == Family::Hold => ("HOLD & WIN", Color32::from_rgb(0xd8, 0xa0, 0x20)),
        _ => ("TUMBLES · BOMBS", Color32::from_rgb(0xe0, 0x4a, 0xc8)),
    }
}

/// The Info page's paragraphs on a machine's feature, Ante, its feature spin and its most, in
/// 0.13.28's words.
pub fn info_paragraphs(machine: u8) -> Vec<String> {
    let m = &rm::MACHINES[(machine as usize).min(11)];
    let (short, rest) = feature_words(machine);
    let mut out = vec![format!("{short} {rest}")];
    out.push(format!("Ante, beside the bet: each spin a quarter more, and the bonus about {} times as likely.", if m.family == Family::Tumble { "1.5" } else { "1.7" }));
    let (_, what) = super::stake::SlotKind::Rm(machine).describe(pyr_poker::casino::feature::Feature::Wild);
    out.push(format!("Or buy the feature straight away with Play Feature: the {}, {}× the bet. {what}", m.special_name, times(m.special_price)));
    if m.builds_price > 0 {
        let (name, what) = super::stake::SlotKind::Rm(machine).describe(pyr_poker::casino::feature::Feature::Builds);
        out.push(format!("{name}, from Play Feature at {}× the bet: {what}", times(m.builds_price)));
    }
    if m.cap_x > 0 {
        out.push(format!("A round pays up to {}× the bet.", chips_text(m.cap_x)));
    }
    out
}
