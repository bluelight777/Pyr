//! Plinko at the app (0.13.5), as 0.13.28 drew it: on the left, Manual or Auto, the amount (typed,
//! halved or doubled), the difficulty, the rows (8 to 16), what it can pay and Drop a ball; on the
//! right, Meshy's temple behind a triangle of gold pegs that ring and light up as the balls strike
//! them, and the buckets, which are pressed down and lit when a ball lands, its multiple floating
//! up; the last ten buckets down the right. Balls can be dropped one after another while others
//! are still falling. (Sizes, colors and timings measured off 0.13.28's screen and filmed.)

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::casino::plinko::{self as pk, Risk};
use pyr_poker::casino::{Bet, Outcome};
use pyr_poker::net::RoundView;

use super::col;
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

/// From the ring to the first peg, then each row, then into the bucket (seconds: 0.13.28's, as
/// filmed: a tenth of a second a row).
const DROP_S: f32 = 0.22;
const ROW_S: f32 = 0.1;
const LAND_S: f32 = 0.1;

/// The column on the board's right for the last results, and how many it shows.
const RESULTS_W: f32 = 70.0;
const RESULTS_SHOWN: usize = 10;

const PANEL: Color32 = Color32::from_rgb(0x13, 0x0c, 0x1c);
const PURPLE: Color32 = Color32::from_rgb(0x4a, 0x2a, 0x5a);
const DARK: Color32 = Color32::from_rgb(0x0f, 0x0a, 0x14);
/// The amount's field, and the small buttons, the rows' box and slider (0.13.28's browns).
const FIELD: Color32 = Color32::from_rgb(0x1a, 0x15, 0x12);
const KNOB: Color32 = Color32::from_rgb(0x2f, 0x28, 0x23);
const GOLD: Color32 = col::GOLD;
/// Words on gold, and the running Auto play's button (0.13.28's).
const ON_GOLD: Color32 = Color32::from_rgb(0x2a, 0x10, 0x04);
const STOP_RED: Color32 = Color32::from_rgb(0xb0, 0x20, 0x2a);
const PEG: Color32 = Color32::from_rgb(0xc8, 0x92, 0x2a);

struct Ball {
    o: pk::Outcome,
    start: Instant,
    /// Pegs struck so far (each rings once), and whether it's in its bucket.
    struck: usize,
    landed: bool,
}

pub struct PlinkoUi {
    pub chips: u64,
    amount: String,
    pub risk: Risk,
    pub rows: u8,
    auto_tab: bool,
    /// Playing by itself: games left (`auto::FOREVER`: until stopped), and whether a big win
    /// stops it (off unless ticked: wins never stop it otherwise).
    pub auto: super::auto::Auto,
    games: String,
    balls: Vec<Ball>,
    /// Pegs lit (row, peg) and buckets a ball landed in, since when.
    lit: HashMap<(u8, u8), Instant>,
    bounced: HashMap<u8, Instant>,
    /// The last balls' buckets (the bucket, of how many rows, its multiple, when), newest first.
    results: Vec<(u8, u8, u64, Instant)>,
    next_at: Option<Instant>,
}

impl Default for PlinkoUi {
    fn default() -> Self {
        PlinkoUi {
            chips: 100,
            amount: "100".into(),
            risk: Risk::Medium,
            rows: 16,
            auto_tab: false,
            auto: super::auto::Auto { left: 0, big_stops: false },
            games: "0".into(),
            balls: Vec::new(),
            lit: HashMap::new(),
            bounced: HashMap::new(),
            results: Vec::new(),
            next_at: None,
        }
    }
}

/// A multiple as the buckets show it: "110×", "7.5×", "4.0×", "2.79×", "0.29×".
pub fn label(x100: u64) -> String {
    if x100 >= 1000 {
        format!("{}×", chips_text(x100 / 100))
    } else if x100 % 100 == 0 {
        format!("{}.0×", x100 / 100)
    } else if x100 % 10 == 0 {
        format!("{}.{}×", x100 / 100, (x100 % 100) / 10)
    } else {
        format!("{}.{:02}×", x100 / 100, x100 % 100)
    }
}

/// A bucket's color: yellow in the middle to red at the edges (0.13.28's five, in between
/// for the rows that have more buckets).
fn bucket_color(k: usize, n: usize) -> Color32 {
    let c = n as f32 / 2.0;
    let d = ((k as f32 - c).abs() / c).clamp(0.0, 1.0);
    let stops = [(0.0, (0xff, 0xc8, 0x3a)), (0.25, (0xff, 0xa1, 0x2a)), (0.5, (0xff, 0x7a, 0x1a)), (0.75, (0xf3, 0x4a, 0x2a)), (1.0, (0xe8, 0x1a, 0x3a))];
    for w in stops.windows(2) {
        let ((a, ca), (b, cb)) = (w[0], w[1]);
        if d <= b {
            let t = (d - a) / (b - a);
            let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
            return Color32::from_rgb(m(ca.0, cb.0), m(ca.1, cb.1), m(ca.2, cb.2));
        }
    }
    Color32::from_rgb(0xe8, 0x1a, 0x3a)
}

/// `c` with `white` of white mixed in.
fn whiter(c: Color32, white: f32) -> Color32 {
    let m = |x: u8| (x as f32 + (255.0 - x as f32) * white.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgb(m(c.r()), m(c.g()), m(c.b()))
}

/// `a` toward `b` by `t`.
fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// The board's geometry for `rows` in the board's panel: the top peg row's center, the spacing
/// across and down, and the pegs', the ball's and the ring's radii.
struct Board {
    top: Pos2,
    dx: f32,
    dy: f32,
    peg_r: f32,
    ball_r: f32,
    ring_r: f32,
    rows: usize,
}

impl Board {
    /// 0.13.28's layout, measured at three sizes: the results take a column on the right; the
    /// pegs are as wide as the rest allows (or as tall as the panel does), a little below the
    /// panel's middle.
    fn new(panel: Rect, rows: u8) -> Board {
        let n = rows as f32;
        let room = panel.width() - RESULTS_W;
        let dx = (room * 0.92 / (n + 2.0)).min((panel.height() - 77.5) / (2.28 + 0.88 * (n - 1.0))).max(4.0);
        let dy = dx * 0.88;
        let mid = panel.top() + 5.0 + panel.height() / 2.0 - 0.36 * dx;
        let top = pos2(panel.left() + room / 2.0, mid - dy * (n - 1.0) / 2.0);
        Board { top, dx, dy, peg_r: (dx * 0.125).clamp(2.0, 6.0), ball_r: (dx * 0.27).clamp(4.0, 11.0), ring_r: (dx * 0.47).clamp(8.0, 15.0), rows: rows as usize }
    }

    /// Peg `j` (0 to `row + 2`) of row `row`.
    fn peg(&self, row: usize, j: usize) -> Pos2 {
        pos2(self.top.x + (j as f32 - (row as f32 + 2.0) / 2.0) * self.dx, self.top.y + row as f32 * self.dy)
    }

    /// Bucket `k`'s rectangle.
    fn bucket(&self, k: usize) -> Rect {
        let c = pos2(self.top.x + (k as f32 - self.rows as f32 / 2.0) * self.dx, self.top.y + (self.rows as f32 - 1.0) * self.dy + self.dx * 0.85);
        Rect::from_center_size(c, vec2(self.dx * 0.9, self.dx * 0.82))
    }

    fn ring(&self) -> Pos2 {
        self.top - vec2(0.0, self.dx * 1.43)
    }

    /// Where a ball is `t` seconds after it was dropped, and how many pegs it has struck.
    fn ball_at(&self, path: &[bool], t: f32) -> (Pos2, usize) {
        let contact = self.peg_r + self.ball_r;
        let at_row = |row: usize, rights: usize| -> Pos2 { pos2(self.top.x + (2.0 * rights as f32 - row as f32) * self.dx / 2.0, self.top.y + row as f32 * self.dy - contact) };
        if t < DROP_S {
            let u = t / DROP_S;
            let a = self.ring();
            let b = at_row(0, 0);
            return (pos2(a.x, a.y + (b.y - a.y) * u * u), 0);
        }
        let n = path.len();
        let t = t - DROP_S;
        let i = (t / ROW_S) as usize;
        if i < n {
            let u = (t - i as f32 * ROW_S) / ROW_S;
            let rights = path[..i].iter().filter(|r| **r).count();
            let from = at_row(i, rights);
            let to_rights = rights + path[i] as usize;
            let to = if i + 1 < n {
                at_row(i + 1, to_rights)
            } else {
                let b = self.bucket(to_rights);
                pos2(b.center().x, b.top() - self.ball_r * 0.3)
            };
            // (A little hop off the peg, then down.)
            let hop = self.dy * 0.35;
            let y = from.y + (to.y - from.y) * u * u - hop * 4.0 * u * (1.0 - u) * 0.6;
            return (pos2(from.x + (to.x - from.x) * u, y), i + 1);
        }
        let rights = path.iter().filter(|r| **r).count();
        let b = self.bucket(rights);
        let u = ((t - n as f32 * ROW_S) / LAND_S).min(1.0);
        (pos2(b.center().x, b.top() - self.ball_r * 0.3 + (b.height() * 0.45) * u), n)
    }
}

/// How long a ball takes from the ring into its bucket.
fn fall_s(rows: usize) -> f32 {
    DROP_S + rows as f32 * ROW_S + LAND_S
}

impl PyrApp {
    pub(super) fn plinko_round(&mut self, r: &RoundView) {
        let Outcome::Plinko(o) = &r.outcome else { return };
        self.poker.casino.plinko.balls.push(Ball { o: o.clone(), start: Instant::now(), struck: 0, landed: false });
    }

    fn plinko_drop(&mut self, s: &ServerView) {
        let c = &s.poker.casino;
        let ui = &self.poker.casino.plinko;
        let held: u64 = ui.balls.iter().filter(|b| !b.landed).map(|b| b.o.paid).sum();
        if ui.chips == 0 || ui.chips + held > super::chips_now(s) || self.poker.casino.busy() {
            self.poker.casino.plinko.auto.stop();
            return;
        }
        let Some(commit) = c.commit else { return };
        self.poker.casino.pending = Some(Instant::now());
        self.engine.casino_play(&s.key, commit, Bet::Plinko { chips: ui.chips, risk: ui.risk, rows: ui.rows });
    }

    pub(super) fn plinko_page(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let now = Instant::now();
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        // The temple behind everything (as bright as 0.13.28 showed it: three quarters, and
        // darker behind the board).
        super::pics::cover(&p, &ctx, "plinko-back", area, 1376, Color32::from_gray(190));
        let left = Rect::from_min_size(area.min + vec2(18.0, 18.0), vec2(300.0, area.height() - 36.0));
        let board_r = Rect::from_min_max(pos2(left.right() + 20.0, area.top() + 18.0), area.max - vec2(18.0, 18.0));
        self.plinko_controls(ui, s, left, now);
        // The board.
        p.rect_filled(board_r, theme::r(18), Color32::from_rgba_unmultiplied(0x09, 0x07, 0x12, 118));
        p.rect_stroke(board_r, theme::r(18), Stroke::new(1.2_f32, Color32::from_rgb(0x9a, 0x76, 0x3c)), StrokeKind::Inside);
        let pk_ui = &self.poker.casino.plinko;
        let (risk, rows) = (pk_ui.risk, pk_ui.rows);
        // (Balls still falling keep the board they were dropped on.)
        let shown_rows = pk_ui.balls.last().map(|b| b.o.rows).unwrap_or(rows);
        let shown_risk = pk_ui.balls.last().map(|b| b.o.risk).unwrap_or(risk);
        p.text(board_r.left_top() + vec2(26.0, 26.0), Align2::LEFT_CENTER, format!("PLINKO · {} · {} ROWS", shown_risk.name().to_uppercase(), shown_rows), theme::bold_font(14.0), Color32::from_rgb(0xff, 0xec, 0xc8));
        let b = Board::new(board_r, shown_rows);
        let n = shown_rows as usize;
        // The ring the balls drop from, in a warm glow.
        let ring = b.ring();
        super::radial(&p, ring, b.dx * 1.1, Color32::from_rgba_unmultiplied(0xff, 0xac, 0x37, 80), Color32::TRANSPARENT);
        p.circle_filled(ring, b.ring_r, Color32::from_rgba_unmultiplied(0xff, 0xb0, 0x50, 18));
        p.circle_stroke(ring, b.ring_r, Stroke::new(1.5_f32, Color32::from_rgb(0xd0, 0xa2, 0x4a)));
        // Pegs: gold, lit pale with a soft disc around them for a moment when a ball strikes.
        let lit_peg = Color32::from_rgb(0xf2, 0xda, 0x91);
        for row in 0..n {
            for j in 0..row + 3 {
                let c = b.peg(row, j);
                let glow = self.poker.casino.plinko.lit.get(&(row as u8, j as u8)).map(|t| (1.0 - (now - *t).as_secs_f32() / 0.45).max(0.0)).unwrap_or(0.0);
                if glow > 0.0 {
                    p.circle_filled(c, b.dx * 0.45, Color32::from_rgba_unmultiplied(0xff, 0xd2, 0x78, (66.0 * glow) as u8));
                }
                p.circle_filled(c + vec2(0.0, b.peg_r * 0.25), b.peg_r * 1.05, Color32::from_black_alpha(90));
                p.circle_filled(c, b.peg_r, mix(PEG, lit_peg, glow));
                p.circle_filled(c - vec2(b.peg_r * 0.33, b.peg_r * 0.4), b.peg_r * 0.38, mix(Color32::from_rgb(0xe7, 0xd0, 0xa4), Color32::from_rgb(0xff, 0xf6, 0xe0), glow));
            }
        }
        // Buckets: pressed down and lit when a ball lands, their multiple floating up above.
        let table = pk::table(shown_risk, shown_rows);
        let mut floats = Vec::new();
        for (k, m) in table.iter().enumerate() {
            let mut r = b.bucket(k);
            let since = self.poker.casino.plinko.bounced.get(&(k as u8)).map(|t| (now - *t).as_secs_f32());
            let mut flash = 0.0;
            if let Some(t) = since {
                if t < 0.42 {
                    r = r.translate(vec2(0.0, (std::f32::consts::PI * t / 0.42).sin() * b.dx * 0.2));
                }
                flash = (1.0 - t / 0.65).max(0.0) * 0.34;
                if t < 1.3 {
                    floats.push((b.bucket(k), *m, t));
                }
            }
            let c = bucket_color(k, n);
            let face = theme::r(4);
            p.rect_filled(r, face, whiter(c, flash));
            // (The top two fifths a little lighter.)
            let upper = Rect::from_min_max(r.min, pos2(r.right(), r.top() + r.height() * 0.4));
            p.rect_filled(upper, egui::CornerRadius { nw: face.nw, ne: face.ne, sw: 0, se: 0 }, whiter(c, 0.16 + flash));
            let size = (b.dx * 0.27).clamp(7.5, 12.0);
            let text = label(*m);
            let size = if text.len() > 5 { size * 0.85 } else { size };
            p.text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(size), Color32::from_rgb(0x2f, 0x0c, 0x05));
        }
        for (r, m, t) in floats {
            let a = if t < 0.35 { 1.0 } else { (1.0 - (t - 0.35) / 0.95).max(0.0) };
            let rise = 1.0 - (1.0 - (t / 1.3).min(1.0)).powi(3);
            let at = r.center_top() - vec2(0.0, b.dx * 0.45 + b.dx * 0.7 * rise);
            let font = theme::bold_font((b.dx * 0.32).clamp(12.0, 16.0));
            p.text(at + vec2(1.0, 1.5), Align2::CENTER_BOTTOM, label(m), font.clone(), Color32::from_rgba_unmultiplied(0x1a, 0x08, 0x08, (170.0 * a) as u8));
            p.text(at, Align2::CENTER_BOTTOM, label(m), font, Color32::from_rgb(0xf0, 0x94, 0x7c).gamma_multiply(a));
        }
        // The balls, falling.
        let mut struck: Vec<(u8, u8)> = Vec::new();
        let mut landed: Vec<(u8, u8, u64, u64, u64)> = Vec::new();
        let mut sounds = 0;
        for ball in self.poker.casino.plinko.balls.iter_mut() {
            let t = (now - ball.start).as_secs_f32();
            let (pos, hits) = b.ball_at(&ball.o.path, t);
            while ball.struck < hits.min(ball.o.path.len()) {
                let row = ball.struck;
                let rights = ball.o.path[..row].iter().filter(|r| **r).count();
                struck.push((row as u8, (rights + 1) as u8));
                ball.struck += 1;
                sounds += 1;
            }
            if !ball.landed && t >= fall_s(ball.o.path.len()) {
                ball.landed = true;
                landed.push((ball.o.bucket, ball.o.rows, ball.o.multiplier_x100, ball.o.paid, ball.o.chips));
            }
            if !ball.landed {
                p.circle_filled(pos, b.ball_r * 1.6, Color32::from_rgba_unmultiplied(0xff, 0x8a, 0x3a, 30));
                p.circle_filled(pos, b.ball_r * 1.25, Color32::from_rgba_unmultiplied(0xff, 0x9a, 0x3a, 44));
                p.circle_filled(pos, b.ball_r, Color32::from_rgb(0xff, 0xb0, 0x4a));
                p.circle_filled(pos - vec2(b.ball_r * 0.36, b.ball_r * 0.4), b.ball_r * 0.36, Color32::from_rgb(0xff, 0xf6, 0xd8));
            }
        }
        let rings: Vec<u8> = struck.iter().take(sounds.min(2)).map(|s| s.0).collect();
        let pu = &mut self.poker.casino.plinko;
        for k in struck {
            pu.lit.insert(k, now);
        }
        pu.lit.retain(|_, t| (now - *t).as_secs_f32() < 0.5);
        pu.bounced.retain(|_, t| (now - *t).as_secs_f32() < 1.4);
        pu.balls.retain(|b| !b.landed || (now - b.start).as_secs_f32() < fall_s(b.o.path.len()) + 0.2);
        // Each peg its row's note; a bucket its own, louder when it pays more than the ball cost
        // (and no other win sound), as 0.13.28's.
        for row in rings {
            self.poker.casino.sound_in(0.0, Cue::Peg(row), 0.0, 0.5);
        }
        for (bucket, rows, m, paid, chips) in landed {
            let pu = &mut self.poker.casino.plinko;
            pu.bounced.insert(bucket, now);
            pu.results.insert(0, (bucket, rows, m, now));
            pu.results.truncate(RESULTS_SHOWN);
            let out = (bucket as i32 - rows as i32 / 2).unsigned_abs() as u8;
            self.poker.casino.sound_in(0.0, Cue::Bucket(out), 0.0, if paid > chips { 0.8 } else { 0.5 });
            // (Auto play: the next ball after this one, unless a big win stops it when ticked.)
            let big = m >= 1000;
            // A big win's shown once the bucket's lit and the win's floated up: 1.25 s after the
            // ball lands, as 0.13.28's (filmed: 1.15 and 1.35 s, at 11 times); balls keep
            // dropping under it.
            self.poker.casino.celebrate(paid, chips, false, 1.25);
            if self.poker.casino.plinko.auto.on() && !self.poker.casino.plinko.auto.spun(big) {
                self.poker.casino.plinko.next_at = None;
            }
        }
        // The balance: the chips with the balls still falling not yet won.
        let falling: u64 = self.poker.casino.plinko.balls.iter().filter(|b| !b.landed).map(|b| b.o.paid).sum();
        if falling > 0 {
            self.poker.casino.hold = Some((super::chips_now(s).saturating_sub(falling), now + Duration::from_secs(10)));
        } else if self.poker.casino.hold.is_some() && self.poker.casino.plinko.balls.iter().all(|b| b.landed) {
            self.poker.casino.settle_balance();
        }
        // The last buckets, newest at the top of the column on the right, in their buckets'
        // colors (the older ones dimmer).
        let mut y = board_r.top() + 35.0;
        for (i, (bucket, rows, m, at)) in self.poker.casino.plinko.results.iter().enumerate() {
            let fresh = (now - *at).as_secs_f32();
            let a = (fresh / 0.15).min(1.0) * if i == 0 { 1.0 } else { 0.77 };
            let r = Rect::from_min_size(pos2(board_r.right() - 15.0 - 56.0, y), vec2(56.0, 30.0));
            p.rect_filled(r, theme::r(7), bucket_color(*bucket as usize, *rows as usize).gamma_multiply(a));
            p.text(r.center(), Align2::CENTER_CENTER, label(*m), theme::bold_font(12.5), Color32::from_rgb(0x2f, 0x0c, 0x05).gamma_multiply(a));
            y += 36.0;
        }
        // Auto play's next ball.
        if self.poker.casino.plinko.auto.on() {
            let due = self.poker.casino.plinko.next_at.is_none_or(|t| now >= t);
            if due && !self.poker.casino.busy() {
                self.poker.casino.plinko.next_at = Some(now + Duration::from_millis(420));
                self.plinko_drop(s);
            }
        }
    }

    /// The panel on the left, as 0.13.28's (measured off its screen at 1280 by 800: every row's
    /// place, the sizes of its words and its colours; the panel's always 300 wide).
    fn plinko_controls(&mut self, ui: &mut egui::Ui, s: &ServerView, r: Rect, _now: Instant) {
        let p = ui.painter().clone();
        p.rect_filled(r, theme::r(18), PANEL.gamma_multiply(0.94));
        p.rect_stroke(r, theme::r(18), Stroke::new(1.0_f32, Color32::from_rgb(0x3a, 0x2a, 0x4e)), StrokeKind::Inside);
        let x = r.left() + 18.0;
        // (The rows run to three from the right edge: 0.13.28's.)
        let w = r.width() - 21.0;
        let mut y = r.top() + 18.0;
        // (Boxes a pixel lower than the words' rows would put them: 0.13.28's, measured.)
        let at = |x: f32, y: f32| pos2(x, y + 1.0);
        let busy = self.poker.casino.busy();
        let pill = |ui: &mut egui::Ui, id: &str, rect: Rect, text: &str, on: bool, font: egui::FontId, corner: u8, on_fill: Color32, on_text: Color32, off_text: Color32| -> egui::Response {
            let resp = ui.interact(rect, ui.id().with(("pk", id)), Sense::click());
            let fill = if on { on_fill } else if resp.hovered() { Color32::from_rgb(0x2a, 0x20, 0x36) } else { DARK };
            ui.painter().rect_filled(rect, theme::r(corner), fill);
            ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, font, if on { on_text } else { off_text });
            resp
        };
        // Manual or Auto.
        let tab = (w - 31.0) / 2.0;
        let auto_tab = self.poker.casino.plinko.auto_tab;
        if pill(ui, "manual", Rect::from_min_size(pos2(x, y), vec2(tab, 36.0)), "Manual", !auto_tab, theme::bold_font(14.0), 18, PURPLE, Color32::WHITE, theme::MUTED()).clicked() {
            self.poker.casino.plinko.auto_tab = false;
        }
        if pill(ui, "auto", Rect::from_min_size(pos2(x + tab + 8.0, y), vec2(tab, 36.0)), "Auto", auto_tab, theme::bold_font(14.0), 18, PURPLE, Color32::WHITE, theme::MUTED()).clicked() {
            self.poker.casino.plinko.auto_tab = true;
        }
        y += 58.0;
        // The amount: its field, then halve and double.
        p.text(pos2(x, y), Align2::LEFT_CENTER, "Amount", theme::font(14.5), theme::MUTED());
        y += 17.0;
        let field = Rect::from_min_size(at(x, y + 3.0), vec2(148.0, 24.0));
        p.rect_filled(field, theme::r(6), FIELD);
        let pu = &mut self.poker.casino.plinko;
        let resp = ui.put(
            field.translate(vec2(0.0, -1.0)),
            egui::TextEdit::singleline(&mut pu.amount).frame(false).font(theme::bold_font(15.5)).text_color(theme::TEXT()).vertical_align(egui::Align::Center),
        );
        if resp.changed() {
            let digits: String = pu.amount.chars().filter(|c| c.is_ascii_digit()).collect();
            pu.amount = digits.clone();
            pu.chips = digits.parse::<u64>().unwrap_or(0).min(pk::MAX_BET);
        }
        if resp.lost_focus() {
            pu.chips = pu.chips.clamp(pk::MIN_BET, pk::MAX_BET);
            pu.amount = pu.chips.to_string();
        }
        let small = |ui: &mut egui::Ui, id: &str, rect: Rect, text: &str| -> bool {
            let resp = ui.interact(rect, ui.id().with(("pk", id)), Sense::click());
            ui.painter().rect_filled(rect, theme::r(6), if resp.hovered() { Color32::from_rgb(0x3a, 0x30, 0x2a) } else { KNOB });
            // (Its word a pixel above the middle: 0.13.28's.)
            ui.painter().text(rect.center() - vec2(0.0, 1.0), Align2::CENTER_CENTER, text, theme::bold_font(14.5), Color32::WHITE);
            resp.clicked()
        };
        if small(ui, "half", Rect::from_min_size(at(x + 156.0, y), vec2(40.0, 30.0)), "½") {
            let pu = &mut self.poker.casino.plinko;
            pu.chips = (pu.chips / 2).max(pk::MIN_BET);
            pu.amount = pu.chips.to_string();
        }
        if small(ui, "double", Rect::from_min_size(at(x + 204.0, y), vec2(43.0, 30.0)), "2×") {
            let pu = &mut self.poker.casino.plinko;
            pu.chips = (pu.chips * 2).clamp(pk::MIN_BET, pk::MAX_BET);
            pu.amount = pu.chips.to_string();
        }
        y += 53.0;
        // The difficulty.
        p.text(pos2(x, y), Align2::LEFT_CENTER, "Difficulty", theme::font(14.5), theme::MUTED());
        y += 17.0;
        let widths = [61.0, 75.0, 61.0, 64.0];
        let gap = (w - widths.iter().sum::<f32>()) / 3.0;
        let mut bx = x;
        for (risk, bw) in Risk::ALL.into_iter().zip(widths) {
            let on = self.poker.casino.plinko.risk == risk;
            if pill(ui, risk.name(), Rect::from_min_size(at(bx, y), vec2(bw, 32.0)), risk.name(), on, theme::bold_font(12.75), 11, GOLD, ON_GOLD, Color32::WHITE).clicked() {
                self.poker.casino.plinko.risk = risk;
            }
            bx += bw + gap;
        }
        y += 55.0;
        // The rows.
        p.text(pos2(x, y), Align2::LEFT_CENTER, "Rows", theme::font(14.5), theme::MUTED());
        y += 17.0;
        let track = Rect::from_min_size(at(x, y + 11.0), vec2(w - 59.0, 8.0));
        let rows = self.poker.casino.plinko.rows;
        let f = (rows - pk::MIN_ROWS) as f32 / (pk::MAX_ROWS - pk::MIN_ROWS) as f32;
        p.rect_filled(track, theme::r(4), KNOB);
        // (The handle stays inside the track, end to end.)
        let hx = track.left() + 9.0 + (track.width() - 18.0) * f;
        let handle = Rect::from_center_size(pos2(hx, track.center().y), vec2(18.0, 24.0));
        let resp = ui.interact(track.expand2(vec2(10.0, 12.0)), ui.id().with("pk-rows"), Sense::click_and_drag());
        p.rect_filled(handle, theme::r(5), KNOB);
        p.rect_stroke(handle, theme::r(5), Stroke::new(1.2_f32, Color32::from_rgb(0xee, 0xe6, 0xdf)), StrokeKind::Inside);
        if (resp.dragged() || resp.clicked())
            && let Some(q) = resp.interact_pointer_pos()
        {
            let f = ((q.x - track.left()) / track.width()).clamp(0.0, 1.0);
            self.poker.casino.plinko.rows = pk::MIN_ROWS + (f * (pk::MAX_ROWS - pk::MIN_ROWS) as f32).round() as u8;
        }
        let vb = Rect::from_min_size(at(track.right() + 8.0, y), vec2(40.0, 30.0));
        p.rect_filled(vb, theme::r(6), KNOB);
        p.text(vb.center(), Align2::CENTER_CENTER, rows.to_string(), theme::font(14.5), theme::TEXT());
        y += 46.0;
        let (risk, rows, chips) = (self.poker.casino.plinko.risk, self.poker.casino.plinko.rows, self.poker.casino.plinko.chips);
        let top = pk::top_x100(risk, rows);
        p.text(pos2(x, y), Align2::LEFT_CENTER, format!("Up to {} · pays back 99%", label(top)), theme::font(12.0), theme::DIM());
        let have = super::chips_now(s);
        let ok = chips >= pk::MIN_BET && chips <= have && !busy;
        if self.poker.casino.plinko.auto_tab {
            // Auto play: how many games (0: until stopped), and whether a big win stops it (both
            // dimmed while it runs).
            let running = self.poker.casino.plinko.auto.on();
            y += 32.0;
            p.text(pos2(x, y), Align2::LEFT_CENTER, "Number of games (0: until you stop it)", theme::font(14.5), theme::MUTED());
            y += 17.0;
            let field = Rect::from_min_size(at(x, y), vec2(w, 22.0));
            p.rect_filled(field, theme::r(6), FIELD);
            let pu = &mut self.poker.casino.plinko;
            let resp = ui.put(field, |ui: &mut egui::Ui| ui.add_enabled(!running, egui::TextEdit::singleline(&mut pu.games).frame(false).font(theme::font(14.5)).text_color(theme::TEXT()).vertical_align(egui::Align::Center)));
            if resp.changed() {
                pu.games = pu.games.chars().filter(|c| c.is_ascii_digit()).take(6).collect();
            }
            y += 45.0;
            let mut stop = pu.auto.big_stops;
            let row = Rect::from_center_size(pos2(x + w / 2.0, y - 3.0), vec2(w, 22.0));
            let changed = ui.scope_builder(egui::UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)), |ui| ui.add_enabled(!running, egui::Checkbox::new(&mut stop, egui::RichText::new("Stop on a big win (10× or more)").font(theme::font(14.5)).color(theme::TEXT()))).changed()).inner;
            if changed {
                self.poker.casino.plinko.auto.big_stops = stop;
            }
            y += 23.0;
            let text = if running { format!("Stop autoplay ({} left)", self.poker.casino.plinko.auto.left_text()) } else { "Start autoplay".to_string() };
            if big_button(ui, Rect::from_min_size(at(x, y), vec2(w, 48.0)), &text, running || ok, running) {
                if running {
                    self.poker.casino.plinko.auto.stop();
                } else {
                    let n: u32 = self.poker.casino.plinko.games.parse().unwrap_or(0);
                    self.poker.casino.plinko.auto.left = if n == 0 { super::auto::FOREVER } else { n };
                    self.poker.casino.plinko.next_at = None;
                }
            }
            y += 74.0;
        } else {
            y += 23.0;
            let space = super::keys::pressed(ui.ctx(), super::keys::Move::PlinkoDrop);
            if big_button(ui, Rect::from_min_size(at(x, y), vec2(w, 48.0)), "Drop a ball", ok, false) || (space && ok) {
                self.plinko_drop(s);
            }
            y += 74.0;
        }
        p.text(pos2(x, y), Align2::LEFT_CENTER, format!("Ball {} · the most it can win: {}", chips_text(chips), chips_text(chips.saturating_mul(top) / 100)), theme::font(12.0), theme::MUTED());
    }
}

/// The panel's big button: gold (red with white words to stop Auto play, `stop`).
fn big_button(ui: &mut egui::Ui, r: Rect, text: &str, enabled: bool, stop: bool) -> bool {
    let resp = ui.interact(r, ui.id().with(("pk-big", stop)), if enabled { Sense::click() } else { Sense::hover() });
    let fill = if stop {
        if resp.hovered() { Color32::from_rgb(0xc4, 0x2a, 0x34) } else { STOP_RED }
    } else if !enabled {
        Color32::from_rgb(0x5a, 0x4a, 0x2c)
    } else if resp.hovered() {
        Color32::from_rgb(0xff, 0xd6, 0x66)
    } else {
        GOLD
    };
    ui.painter().rect_filled(r, theme::r(12), fill);
    ui.painter().text(r.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, text, theme::bold_font(17.0), if stop { Color32::WHITE } else { ON_GOLD });
    enabled && resp.clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The buckets read as 0.13.28's did.
    #[test]
    fn multiples_read_as_on_0_13_28s_buckets() {
        let labels: Vec<String> = [11000, 5700, 2600, 1300, 400, 120, 40, 30, 29, 750, 279, 54, 1_000_000, 50].iter().map(|m| label(*m)).collect();
        assert_eq!(labels, ["110×", "57×", "26×", "13×", "4.0×", "1.2×", "0.4×", "0.3×", "0.29×", "7.5×", "2.79×", "0.54×", "10,000×", "0.5×"]);
    }
}
