//! Flash Pots at the app: the card in the tournaments (play one for 100, 1,000 or 10,000 chips),
//! the prize wheel spinning at the table, the fuse burning down, and the bang.
//!
//! The server keeps the time (Unix ms): when the wheel was spun and when the bomb goes off. The
//! app draws from those, so everyone at the table sees the same wheel and the same fuse.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::net::{ToServer, TourneyId, TourneyPhase, TourneyView};
use pyr_poker::tourney::{BOMB_BUY_INS, BOMB_FUSE_MS, BOMB_SEATS, BOMB_WAIT_MS, WHEEL_MS};

use super::cards::chips_text;
use crate::app::PyrApp;
use crate::casino::{col, ease_out, glow_text, radial, vgrad_round};
use crate::theme;

const TAU: f32 = std::f32::consts::TAU;
/// The wheel's segments, clockwise from the top: mostly 3s, the big ones few (the odds are the
/// server's; this is the picture).
const WHEEL: [u32; 16] = [3, 5, 3, 8, 3, 15, 3, 5, 3, 40, 3, 8, 3, 150, 5, 1000];
/// How long the wheel turns at the least (an older server leaves five seconds for the wheel);
/// it comes in for this long first.
const SPIN_S: f32 = 3.2;
const INTRO_S: f32 = 0.55;
/// The bang, then the warning stays.
const BOOM_S: f32 = 2.6;
/// The winner's moment, after the last hand.
const FINALE_S: f32 = 10.0;
/// A moment before it, to see the last hand's board and who took it.
const FINALE_BEAT_S: f32 = 2.4;

/// What's been played for each Flash Pot: the wheel (its start, the peg under the pointer and
/// when a click was last heard, the landing), the fuse's hiss, the last second ticked, the bang
/// (and when it was first seen), and the winner's moment (seen it being played; when it began;
/// dismissed).
#[derive(Default, Clone, Copy)]
pub struct BombFx {
    wheel: bool,
    seg: Option<usize>,
    clicked: Option<Instant>,
    landed: bool,
    counted: bool,
    fuse: bool,
    tick: u64,
    boom: Option<Instant>,
    running: bool,
    finale: Option<Instant>,
    finale_done: bool,
    finale_sounded: bool,
    finale_counted: bool,
}

pub type BombSounds = HashMap<TourneyId, BombFx>;

fn wall() -> u64 {
    pyr_proto::now_ms().max(0) as u64
}

/// A multiplier's color on the wheel.
fn seg_color(m: u32) -> Color32 {
    match m {
        3 => Color32::from_rgb(0x24, 0x3a, 0x7a),
        5 => Color32::from_rgb(0x1c, 0x6a, 0x7a),
        8 => Color32::from_rgb(0x1f, 0x7a, 0x48),
        15 => Color32::from_rgb(0x6a, 0x2a, 0x9a),
        40 => Color32::from_rgb(0xd0, 0x6a, 0x14),
        150 => Color32::from_rgb(0xc0, 0x1c, 0x2e),
        _ => Color32::from_rgb(0xe0, 0xa8, 0x20),
    }
}

/// A cartoon bomb: a black ball with a shine, its cap, and a fuse (lit: sparking, with
/// `left` of it to burn, from 1 to 0).
pub fn bomb_icon(p: &egui::Painter, c: Pos2, r: f32, t: f32, lit: bool, left: f32) {
    p.circle_filled(c + vec2(r * 0.1, r * 0.16), r, Color32::from_black_alpha(90));
    // The ball, lit from the top left: light at that point, dark round the edge.
    let light = c - vec2(r * 0.35, r * 0.35);
    let mut m = egui::Mesh::default();
    m.colored_vertex(light, Color32::from_rgb(0x6a, 0x6a, 0x78));
    let n = 48;
    for i in 0..=n {
        let a = i as f32 / n as f32 * TAU;
        m.colored_vertex(c + vec2(a.cos(), a.sin()) * r, Color32::from_rgb(0x0e, 0x0e, 0x12));
    }
    for i in 1..=n as u32 {
        m.add_triangle(0, i, i + 1);
    }
    p.add(Shape::mesh(m));
    p.circle_stroke(c, r, Stroke::new(1.0f32, Color32::from_rgb(0x08, 0x08, 0x0a)));
    p.circle_filled(c + vec2(-r * 0.36, -r * 0.38), r * 0.2, Color32::from_white_alpha(70));
    p.circle_filled(c + vec2(-r * 0.2, -r * 0.52), r * 0.08, Color32::from_white_alpha(110));
    // The cap.
    let a = -0.85f32;
    let neck = c + vec2(a.cos(), a.sin()) * r * 0.92;
    let (ux, uy) = (vec2(a.cos(), a.sin()), vec2(-a.sin(), a.cos()));
    let cap = [neck - uy * r * 0.26, neck + uy * r * 0.26, neck + uy * r * 0.26 + ux * r * 0.3, neck - uy * r * 0.26 + ux * r * 0.3];
    p.add(Shape::convex_polygon(cap.to_vec(), Color32::from_rgb(0x70, 0x70, 0x7a), Stroke::new(1.0f32, Color32::from_rgb(0x30, 0x30, 0x36))));
    // The fuse: a curl from the cap, as much of it as is left.
    let base = neck + ux * r * 0.3;
    let n = 16;
    let shown = ((n as f32) * left.clamp(0.06, 1.0)).ceil() as usize;
    let pts: Vec<Pos2> = (0..=shown)
        .map(|i| {
            let k = i as f32 / n as f32;
            base + vec2(k * r * 0.75, -k * r * 0.55) + vec2(0.0, -(k * std::f32::consts::PI).sin() * r * 0.28)
        })
        .collect();
    p.add(Shape::line(pts.clone(), Stroke::new((r * 0.1).max(1.5), Color32::from_rgb(0xc8, 0xa8, 0x70))));
    if lit && let Some(tip) = pts.last() {
        let flick = 0.75 + 0.25 * (t * 31.0).sin().abs();
        radial(p, *tip, r * 0.55 * flick, Color32::from_rgba_unmultiplied(0xff, 0xc0, 0x40, 200), Color32::TRANSPARENT);
        for k in 0..7 {
            let ang = k as f32 / 7.0 * TAU + t * 9.0;
            let len = r * (0.22 + 0.18 * ((t * 23.0 + k as f32 * 1.7).sin().abs()));
            p.line_segment([*tip, *tip + vec2(ang.cos(), ang.sin()) * len], Stroke::new(1.5f32, Color32::from_rgb(0xff, 0xe0, 0x80)));
        }
        p.circle_filled(*tip, r * 0.1, Color32::WHITE);
    }
}

/// The prize wheel at `angle`, centered on `c`: the segment `lit` glowing (it's landed), the
/// lights round its rim chasing (`spinning`) or flashing, and its pointer kicked aside by the
/// pegs as they pass (`flap`, 0 to 1).
fn prize_wheel(p: &egui::Painter, c: Pos2, r: f32, angle: f32, lit: Option<usize>, t: f32, spinning: bool, flap: f32) {
    p.circle_filled(c + vec2(0.0, r * 0.05), r * 1.16, Color32::from_black_alpha(150));
    // The rim: dark wood with lights all round.
    p.circle_filled(c, r * 1.13, Color32::from_rgb(0x2a, 0x16, 0x0a));
    p.circle_stroke(c, r * 1.13, Stroke::new(r * 0.02, col::GOLD_DARK));
    let bulbs = 32;
    for b in 0..bulbs {
        let a = b as f32 / bulbs as f32 * TAU + t * 0.15;
        let at = c + vec2(a.cos(), a.sin()) * r * 1.095;
        let on = if spinning { (b + (t * 14.0) as usize) % 4 < 2 } else if lit.is_some() { (t * 6.0) as usize % 2 == b % 2 } else { true };
        let glow = if lit.is_some() && !spinning { seg_color(lit.map(|i| WHEEL[i]).unwrap_or(3)) } else { col::GOLD2 };
        if on {
            radial(p, at, r * 0.05, glow.gamma_multiply(0.7), Color32::TRANSPARENT);
            p.circle_filled(at, r * 0.018, Color32::from_rgb(0xff, 0xf6, 0xd8));
        } else {
            p.circle_filled(at, r * 0.016, Color32::from_rgb(0x5a, 0x40, 0x20));
        }
    }
    p.circle_filled(c, r * 1.06, Color32::from_rgb(0x3a, 0x22, 0x10));
    let seg = TAU / WHEEL.len() as f32;
    for (i, m) in WHEEL.iter().enumerate() {
        let a0 = angle + i as f32 * seg - TAU / 4.0;
        let steps = 6;
        let mut pts = vec![c];
        for k in 0..=steps {
            let a = a0 + seg * k as f32 / steps as f32;
            pts.push(c + vec2(a.cos(), a.sin()) * r);
        }
        let mut fill = seg_color(*m);
        if lit == Some(i) {
            fill = crate::casino::mix(fill, Color32::WHITE, 0.25 + 0.2 * (t * 7.0).sin().abs());
        } else if lit.is_some() {
            fill = crate::casino::mix(fill, Color32::BLACK, 0.35);
        }
        p.add(Shape::convex_polygon(pts, fill, Stroke::new(1.0f32, Color32::from_black_alpha(80))));
        // Its multiplier, out by the rim, reading outwards.
        let mid = a0 + seg / 2.0;
        let at = c + vec2(mid.cos(), mid.sin()) * r * 0.72;
        let text = format!("{m}×");
        let size = if *m >= 100 { r * 0.1 } else { r * 0.12 };
        let galley = p.layout_no_wrap(text, theme::bold_font(size), Color32::WHITE);
        let pos = at - galley.size() / 2.0;
        p.add(egui::epaint::TextShape::new(pos, galley, Color32::WHITE).with_angle_and_anchor(mid, Align2::CENTER_CENTER));
    }
    // The rim, its pegs, and the hub.
    p.circle_stroke(c, r, Stroke::new(r * 0.05, col::GOLD));
    for i in 0..WHEEL.len() {
        let a = angle + i as f32 * seg - TAU / 4.0;
        p.circle_filled(c + vec2(a.cos(), a.sin()) * r * 0.97, r * 0.025, col::GOLD2);
    }
    radial(p, c, r * 0.24, Color32::from_rgb(0x4a, 0x4a, 0x54), Color32::from_rgb(0x10, 0x10, 0x14));
    p.circle_stroke(c, r * 0.24, Stroke::new(2.0f32, col::GOLD));
    bomb_icon(p, c + vec2(-r * 0.02, r * 0.03), r * 0.12, t, true, 0.7);
    // The pointer, at the top, on its pivot: kicked aside by each peg as it passes.
    let pivot = c - vec2(0.0, r * 1.17);
    let tilt = -flap * 0.42;
    let rot = |v: egui::Vec2| pivot + vec2(v.x * tilt.cos() - v.y * tilt.sin(), v.x * tilt.sin() + v.y * tilt.cos());
    let pts = vec![rot(vec2(-r * 0.1, -r * 0.02)), rot(vec2(r * 0.1, -r * 0.02)), rot(vec2(0.0, r * 0.27))];
    p.add(Shape::convex_polygon(pts.iter().map(|q| *q + vec2(2.0, 3.0)).collect(), Color32::from_black_alpha(120), Stroke::NONE));
    p.add(Shape::convex_polygon(pts, col::GOLD2, Stroke::new(2.0f32, Color32::from_rgb(0x5a, 0x34, 0x10))));
    p.circle_filled(pivot, r * 0.045, col::GOLD);
    p.circle_stroke(pivot, r * 0.045, Stroke::new(1.5f32, Color32::from_rgb(0x5a, 0x34, 0x10)));
}

/// A little hash for where things go (the same each frame, different for each `i`).
fn h01(i: u32, salt: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9e37_79b9) ^ salt.wrapping_mul(0x85eb_ca6b);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    x ^= x >> 12;
    (x % 10_000) as f32 / 10_000.0
}

/// Light rays turning slowly behind something, from `c` out to `len`.
fn rays(p: &egui::Painter, c: Pos2, len: f32, t: f32, color: Color32, n: usize) {
    for k in 0..n {
        let a = k as f32 / n as f32 * TAU + t * 0.25;
        let w = TAU / n as f32 * 0.3;
        let tip = |d: f32| c + vec2((a + d).cos(), (a + d).sin()) * len;
        p.add(Shape::convex_polygon(vec![c, tip(-w), tip(w)], color, Stroke::NONE));
    }
}

/// Sparks and confetti flying out of `c` and falling, `k` seconds after the burst.
fn burst(p: &egui::Painter, c: Pos2, size: f32, k: f32, colors: &[Color32], n: u32, salt: u32) {
    if k < 0.0 || k > 2.4 {
        return;
    }
    let fade = (1.0 - k / 2.4).clamp(0.0, 1.0);
    for i in 0..n {
        let a = h01(i, salt) * TAU;
        let speed = size * (0.45 + h01(i, salt + 1) * 0.9);
        let d = speed * ease_out((k / 1.1).min(1.0));
        let q = c + vec2(a.cos(), a.sin()) * d + vec2(0.0, k * k * size * 0.35);
        let color = colors[(i as usize) % colors.len()];
        if i % 3 == 0 {
            // (A scrap of confetti, tumbling.)
            let w = 5.0 * (k * 9.0 + i as f32).sin().abs() + 1.0;
            p.rect_filled(Rect::from_center_size(q, vec2(w, 7.0)), 1.0, color.gamma_multiply(fade));
        } else {
            p.circle_filled(q, 2.2 + 1.8 * fade, color.gamma_multiply(fade));
        }
    }
}

/// Which of the wheel's segments the spin lands on (one showing the multiplier). Where it can,
/// one just past a bigger prize: the wheel creeps by the big one on its way to a stop. (Picked
/// by when it was spun, so it isn't always the same one.)
fn landing(multiplier: u32, spun_ms: u64) -> usize {
    let n = WHEEL.len();
    let spots: Vec<usize> = WHEEL.iter().enumerate().filter(|(_, m)| **m == multiplier).map(|(i, _)| i).collect();
    if spots.is_empty() {
        return 0;
    }
    // (The segment that passes the pointer just before segment i is i + 1.)
    let teasing: Vec<usize> = spots.iter().copied().filter(|i| WHEEL[(i + 1) % n] > multiplier).collect();
    let pick = if teasing.is_empty() { &spots } else { &teasing };
    pick[(spun_ms / 7 % pick.len() as u64) as usize]
}

/// The wheel's angle `s` seconds into a spin of `spin` seconds, landing on segment `i`: eight
/// turns, slowing to a long creep over the last few pegs, a little rock as it stops.
fn wheel_angle(i: usize, s: f32, spin: f32) -> f32 {
    let seg = TAU / WHEEL.len() as f32;
    // Segment i's middle at the top: angle = -(i + 1/2) segments; and eight turns to get there.
    let end = -(i as f32 + 0.5) * seg + 8.0 * TAU;
    let k = (s / spin).clamp(0.0, 1.0);
    let e = 1.0 - (1.0 - k).powf(4.6);
    let after = s - spin;
    let rock = if after > 0.0 && after < 0.9 { seg * 0.07 * (-after * 5.0).exp() * (after * 17.0).sin() * (1.0 - after / 0.9) } else { 0.0 };
    end * e + rock
}

/// Where the pointer is on the wheel at `angle`: the segment under it, and how far it's come
/// from the last peg (0 just past one, 1 at the next).
fn under_pointer(angle: f32) -> (usize, f32) {
    let n = WHEEL.len() as f32;
    let seg = TAU / n;
    let q = (-angle / seg).rem_euclid(n);
    let i = q.floor() as usize % WHEEL.len();
    (i, 1.0 - q.fract())
}

impl PyrApp {
    /// The Flash Pot card at the top of the tournaments: what it is, a Play button for each
    /// buy-in, and the one I'm waiting in.
    pub fn time_bomb_card(&mut self, ui: &mut egui::Ui, s: &ServerView) {
        let me = s.me.as_ref().map(|m| m.user_id.clone()).unwrap_or_default();
        let t = ui.ctx().input(|i| i.time) as f32;
        // Mine: waiting or playing (the views the server sends me as I'm in it).
        let mine: Option<TourneyView> = s
            .poker
            .tourney
            .values()
            .filter(|v| v.bomb.is_some() && v.entrants.iter().any(|e| e.uid == me && e.place.is_none()) && matches!(v.phase, TourneyPhase::Registering | TourneyPhase::Running))
            .max_by_key(|v| v.id)
            .cloned();
        let playing_now = s.poker.tourneys.as_ref().map(|l| l.iter().filter(|x| x.rules.bomb.is_some() && x.phase == TourneyPhase::Running).count()).unwrap_or(0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(28.0);
            let w = (ui.available_width() - 28.0).max(320.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 150.0), Sense::hover());
            let p = ui.painter().clone();
            vgrad_round(&p, r, 16.0, Color32::from_rgb(0x2a, 0x10, 0x14), Color32::from_rgb(0x12, 0x08, 0x0c));
            radial(&p, r.left_center() + vec2(90.0, 0.0), 160.0, Color32::from_rgba_unmultiplied(0xff, 0x6a, 0x1a, 60), Color32::TRANSPARENT);
            p.rect_stroke(r, theme::r(16), Stroke::new(1.5f32, Color32::from_rgb(0x9a, 0x3a, 0x2a)), StrokeKind::Inside);
            bomb_icon(&p, r.left_center() + vec2(68.0, 18.0), 40.0, t, true, 0.55 + 0.45 * (t * 0.25).sin().abs());
            let x = r.left() + 170.0;
            glow_text(&p, pos2(x, r.top() + 30.0), Align2::LEFT_CENTER, "FLASH POT", theme::bold_font(24.0), col::GOLD2, Color32::from_rgba_unmultiplied(0xff, 0x50, 0x10, 90));
            p.text(pos2(x + 168.0, r.top() + 31.0), Align2::LEFT_CENTER, if playing_now > 0 { format!("{playing_now} going off now") } else { format!("Sit & go · {BOMB_SEATS} players") }, theme::bold_font(12.0), col::MUTED);
            // (What it says ends before the right side's buttons and words.)
            let hint = mine.is_none().then(|| p.layout_no_wrap("Starts with five in; bots fill empty seats after 30 s".to_string(), theme::font(12.5), col::MUTED));
            let right_w = hint.as_ref().map(|g| g.size().x).unwrap_or(0.0).max(356.0) + 24.0;
            let text_w = (r.right() - 20.0 - right_w - x).max(240.0);
            let galley = p.layout(
                format!(
                    "Five players, 500 chips each, the blinds up every minute. The wheel picks the prize, up to 1,000 times the buy-in, and the winner takes it all. Get the chip lead before the fuse burns down: after {} minutes the Flash Pot goes off and every hand is all in.",
                    BOMB_FUSE_MS / 60_000
                ),
                theme::font(13.0),
                col::TEXT,
                text_w,
            );
            p.galley(pos2(x, r.top() + 50.0), galley, col::TEXT);
            // What I can do.
            let right = r.right() - 20.0;
            match &mine {
                Some(v) if v.phase == TourneyPhase::Registering => {
                    let n = v.entrants.len();
                    let bots_in = (v.rules.start_ms.saturating_sub(wall()) / 1000).min(BOMB_WAIT_MS / 1000);
                    p.text(pos2(right, r.top() + 40.0), Align2::RIGHT_CENTER, format!("Waiting for players · {n} of {}", v.rules.max_players), theme::bold_font(16.0), col::GOLD2);
                    p.text(pos2(right, r.top() + 64.0), Align2::RIGHT_CENTER, format!("Bots take any empty seats in 0:{bots_in:02}"), theme::font(13.0), col::MUTED);
                    // The seats: who's in (left of the Leave button).
                    let seats = v.rules.max_players;
                    for k in 0..seats {
                        let c = pos2(right - 112.0 - (seats - 1 - k) as f32 * 38.0, r.top() + 98.0);
                        let filled = k < n;
                        p.circle_filled(c, 15.0, if filled { col::EMBER } else { Color32::from_white_alpha(14) });
                        p.circle_stroke(c, 15.0, Stroke::new(1.5f32, if filled { col::GOLD2 } else { Color32::from_white_alpha(50) }));
                        if let Some(e) = v.entrants.get(k) {
                            let initial: String = e.name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                            p.text(c, Align2::CENTER_CENTER, initial, theme::bold_font(13.0), Color32::BLACK);
                        }
                    }
                    let b = Rect::from_min_size(pos2(right - 80.0, r.top() + 82.0), vec2(80.0, 32.0));
                    if ui.put(b, egui::Button::new("Leave").corner_radius(theme::r(10))).on_hover_text("Your buy-in back").clicked() {
                        self.engine.poker(&s.key, ToServer::TourneyUnregister { id: v.id });
                    }
                    ui.ctx().request_repaint_after(Duration::from_millis(250));
                }
                Some(v) => {
                    let prize = v.bomb.and_then(|b| b.multiplier).map(|m| format!("{m}× · prize {}", chips_text(v.prize_pool))).unwrap_or_default();
                    p.text(pos2(right, r.top() + 44.0), Align2::RIGHT_CENTER, "You're in a Flash Pot", theme::bold_font(16.0), col::GOLD2);
                    p.text(pos2(right, r.top() + 68.0), Align2::RIGHT_CENTER, prize, theme::font(13.0), col::MUTED);
                    let b = Rect::from_min_size(pos2(right - 160.0, r.top() + 88.0), vec2(160.0, 38.0));
                    if let Some(t) = v.my_table
                        && ui.put(b, theme::primary_button("Go to my table")).clicked()
                    {
                        self.open_table(&s.key, t);
                    }
                }
                None => {
                    let chips = s.poker.me.as_ref().map(|m| m.chips).unwrap_or(0);
                    let mut bx = right;
                    for buy_in in BOMB_BUY_INS.iter().rev() {
                        let bw = 112.0;
                        let b = Rect::from_min_size(pos2(bx - bw, r.top() + 84.0), vec2(bw, 44.0));
                        bx -= bw + 10.0;
                        let can = chips >= *buy_in;
                        let resp = ui.interact(b, ui.id().with(("bomb", *buy_in)), Sense::click());
                        let fill = if !can { Color32::from_white_alpha(14) } else if resp.hovered() { col::GOLD2 } else { col::GOLD };
                        p.rect_filled(b, theme::r(12), fill);
                        p.text(b.center() - vec2(0.0, 7.0), Align2::CENTER_CENTER, "PLAY", theme::bold_font(11.0), if can { Color32::from_rgb(0x4a, 0x1a, 0x06) } else { col::MUTED });
                        p.text(b.center() + vec2(0.0, 9.0), Align2::CENTER_CENTER, chips_text(*buy_in), theme::bold_font(17.0), if can { Color32::from_rgb(0x2a, 0x10, 0x04) } else { col::MUTED });
                        let tip = if can { format!("{} chips to play; win up to {}", chips_text(*buy_in), chips_text(*buy_in * 1000)) } else { format!("You need {} chips", chips_text(*buy_in)) };
                        if resp.on_hover_text(tip).clicked() && can {
                            self.engine.poker(&s.key, ToServer::TimeBomb { buy_in: *buy_in });
                        }
                    }
                    if let Some(g) = hint {
                        p.galley(pos2(right - g.size().x, r.top() + 64.0 - g.size().y / 2.0), g, col::MUTED);
                    }
                }
            }
        });
    }

    /// At a Flash Pot's table: the wheel while it spins, then the fuse and the prize over the
    /// felt; the bang when it goes off; and their sounds.
    pub fn bomb_overlay(&mut self, ui: &mut egui::Ui, s: &ServerView, tid: TourneyId, felt_c: Pos2, felt_a: f32, felt_b: f32) {
        let Some(v) = s.poker.tourney.get(&tid) else { return };
        let Some(b) = v.bomb else { return };
        let now = wall();
        let t = ui.ctx().input(|i| i.time) as f32;
        let p = ui.painter().clone();
        let mut fx = self.poker.bomb_fx.get(&tid).copied().unwrap_or_default();
        let mut cues: Vec<Cue> = Vec::new();
        // The wheel: from the spin until the first hand's dealt.
        if self.bomb_wheel(ui, v, &b, &mut fx, felt_c, felt_b, t, &mut cues) {
            self.poker.bomb_fx.insert(tid, fx);
            self.play_bomb(cues);
            return;
        }
        // The fuse (or, gone off, the warning), over the felt's top edge.
        let at = felt_c - vec2(0.0, felt_b * 0.62 + 6.0);
        if b.exploded {
            let first = *fx.boom.get_or_insert_with(|| {
                cues.push(Cue::Boom);
                Instant::now()
            });
            let k = first.elapsed().as_secs_f32();
            if k < BOOM_S {
                boom(&p, ui.max_rect(), felt_c, felt_a, k);
            }
            let plate = Rect::from_center_size(at, vec2(300.0, 40.0));
            let pulse = 0.8 + 0.2 * (t * 5.0).sin().abs();
            p.rect_filled(plate, theme::r(20), Color32::from_rgba_unmultiplied(0x8a, 0x10, 0x14, 230));
            p.rect_stroke(plate, theme::r(20), Stroke::new(2.0f32, Color32::from_rgb(0xff, 0x8a, 0x5a).gamma_multiply(pulse)), StrokeKind::Inside);
            bomb_icon(&p, plate.left_center() + vec2(24.0, 2.0), 13.0, t, false, 0.05);
            p.text(plate.center() + vec2(12.0, 0.0), Align2::CENTER_CENTER, "ALL IN EVERY HAND", theme::bold_font(17.0), Color32::WHITE);
            p.text(at + vec2(0.0, 32.0), Align2::CENTER_CENTER, format!("Winner takes {}", chips_text(v.prize_pool)), theme::bold_font(13.0), col::GOLD2);
        } else if let Some(off) = b.goes_off_ms {
            let left_ms = off.saturating_sub(now);
            let secs = left_ms.div_ceil(1000);
            let frac = left_ms as f32 / b.fuse_ms.max(1) as f32;
            let hot = secs <= 30;
            let urgent = secs <= 10;
            let plate = Rect::from_center_size(at, vec2(236.0, 44.0));
            let pulse = if urgent { 0.5 + 0.5 * (t * 10.0).sin().abs() } else if hot { 0.7 + 0.3 * (t * 4.0).sin().abs() } else { 1.0 };
            p.rect_filled(plate, theme::r(22), Color32::from_black_alpha(200));
            p.rect_stroke(plate, theme::r(22), Stroke::new(if hot { 2.0 } else { 1.0 }, if hot { Color32::from_rgb(0xff, 0x5a, 0x3a).gamma_multiply(pulse) } else { col::GOLD_DARK }), StrokeKind::Inside);
            bomb_icon(&p, plate.left_center() + vec2(26.0, 4.0), 15.0, t, true, frac);
            let clock = format!("{}:{:02}", secs / 60, secs % 60);
            p.text(plate.left_center() + vec2(56.0, 0.0), Align2::LEFT_CENTER, clock, theme::bold_font(22.0), if hot { Color32::from_rgb(0xff, 0x7a, 0x5a) } else { Color32::WHITE });
            p.text(plate.right_center() - vec2(16.0, 7.0), Align2::RIGHT_CENTER, format!("PRIZE {}", chips_text(v.prize_pool)), theme::bold_font(12.5), col::GOLD2);
            p.text(plate.right_center() - vec2(16.0, -9.0), Align2::RIGHT_CENTER, b.multiplier.map(|m| format!("{m}× the buy-in")).unwrap_or_default(), theme::font(11.0), col::MUTED);
            if hot {
                p.text(at + vec2(0.0, 34.0), Align2::CENTER_CENTER, "When it goes off, every hand is all in", theme::bold_font(12.5), Color32::from_rgb(0xff, 0x9a, 0x7a).gamma_multiply(pulse));
            }
            // A hiss at ten seconds, a tick for each of the last five.
            if secs <= 10 && secs > 5 && !fx.fuse {
                fx.fuse = true;
                cues.push(Cue::Fuse);
            }
            if (1..=5).contains(&secs) && fx.tick != secs {
                fx.tick = secs;
                cues.push(Cue::Tick);
            }
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }
        self.poker.bomb_fx.insert(tid, fx);
        self.play_bomb(cues);
    }

    fn play_bomb(&mut self, cues: Vec<Cue>) {
        if self.poker.mute {
            return;
        }
        for c in cues {
            let vol = match c {
                Cue::Boom | Cue::Jackpot => 1.0,
                Cue::ReelStop => 0.55,
                _ => 0.8,
            };
            self.engine.play_poker(c, 0.0, vol);
        }
    }

    /// The prize wheel, from its spin until the first hand's dealt (the server says when: the
    /// window it leaves before the first hand). It comes in big over the table, spins, slows
    /// to a creep over the last pegs (clicking on each, its pointer kicked aside), rocks to a
    /// stop, and the prize it landed on bursts out, counting up. True while it's showing.
    #[allow(clippy::too_many_arguments)]
    fn bomb_wheel(&mut self, ui: &mut egui::Ui, v: &TourneyView, b: &pyr_poker::net::BombView, fx: &mut BombFx, felt_c: Pos2, felt_b: f32, t: f32, cues: &mut Vec<Cue>) -> bool {
        let (Some(m), Some(spun)) = (b.multiplier, b.spun_ms) else { return false };
        let now = wall();
        let since = now.saturating_sub(spun) as f32 / 1000.0;
        // (The first hand comes when the wheel's window is over.)
        let window = b.goes_off_ms.map(|g| g.saturating_sub(b.fuse_ms).saturating_sub(spun) as f32 / 1000.0).unwrap_or(WHEEL_MS as f32 / 1000.0).clamp(4.0, 12.0);
        let spin = (window - INTRO_S - 3.2).max(SPIN_S);
        let show_until = window + 0.6;
        if since >= show_until {
            return false;
        }
        let p = ui.painter().clone();
        let area = ui.max_rect();
        let fade = ((show_until - since) / 0.9).clamp(0.0, 1.0) * (since / 0.3).clamp(0.0, 1.0);
        let s_spin = since - INTRO_S;
        let landed = s_spin >= spin;
        let after = s_spin - spin;
        let big = m >= 40;
        // The room goes dark, a light on the wheel.
        p.rect_filled(area, 0, Color32::from_black_alpha((165.0 * fade) as u8));
        let r = (felt_b * 1.12).clamp(110.0, 250.0);
        // (A big prize shakes the table as it lands.)
        let shake = if landed && big && after < 0.7 { vec2((after * 70.0).sin(), (after * 53.0).cos()) * 9.0 * (1.0 - after / 0.7) } else { vec2(0.0, 0.0) };
        let c = felt_c - vec2(0.0, felt_b * 0.12) + shake;
        let grow = crate::casino::ease_back((since / INTRO_S).min(1.0));
        let r = r * (0.55 + 0.45 * grow);
        let lit = landed.then(|| landing(m, spun));
        let glow_c = if landed { seg_color(m) } else { col::GOLD };
        rays(&p, c, r * 2.4, t, glow_c.gamma_multiply(if landed { 0.16 } else { 0.07 } * fade), 18);
        radial(&p, c, r * 1.9, Color32::from_rgba_unmultiplied(0xff, 0xc0, 0x50, (50.0 * fade) as u8), Color32::TRANSPARENT);
        let i = landing(m, spun);
        let angle = wheel_angle(i, s_spin.max(0.0), spin);
        let (under, past) = under_pointer(angle);
        let flap = if s_spin > 0.0 && past < 0.22 { 1.0 - past / 0.22 } else { 0.0 };
        prize_wheel(&p, c, r, angle, lit, t, s_spin > 0.0 && !landed, flap * if landed { 0.0 } else { 1.0 });
        // Sounds: the launch, a click for each peg (as many as can be heard), the landing.
        if !fx.wheel && since < 1.5 {
            cues.push(Cue::ReelSpin);
        }
        fx.wheel = true;
        if s_spin > 0.0 && !landed && fx.seg != Some(under) {
            fx.seg = Some(under);
            if fx.clicked.is_none_or(|at| at.elapsed() >= Duration::from_millis(70)) {
                fx.clicked = Some(Instant::now());
                cues.push(Cue::ReelStop);
            }
        }
        // The title, over it all.
        let title_at = c - vec2(0.0, r * 1.5);
        if !landed {
            let pulse = 0.85 + 0.15 * (t * 5.0).sin().abs();
            glow_text(&p, title_at, Align2::CENTER_CENTER, "SPINNING FOR THE PRIZE", theme::bold_font((r * 0.16).clamp(18.0, 34.0)), Color32::WHITE.gamma_multiply(fade), col::EMBER.gamma_multiply(0.6 * fade * pulse));
            p.text(c + vec2(0.0, r * 1.3), Align2::CENTER_CENTER, format!("{} buy-in · the winner takes it all", chips_text(v.rules.buy_in)), theme::bold_font(15.0), col::GOLD2.gamma_multiply(fade));
        } else {
            if !fx.landed {
                fx.landed = true;
                cues.push(if m >= 150 {
                    Cue::Jackpot
                } else if m >= 15 {
                    Cue::WinBig
                } else {
                    Cue::WinSmall
                });
            }
            // The prize bursts out: sparks and confetti in its colors, the multiplier big over the
            // wheel, and the prize counting up from the buy-in.
            let colors = [seg_color(m), col::GOLD2, Color32::WHITE, col::EMBER];
            burst(&p, c - vec2(0.0, r * 1.05), r * if big { 2.2 } else { 1.3 }, after, &colors, if big { 90 } else { 50 }, 7);
            if big {
                burst(&p, c + vec2(-r * 1.2, -r * 0.3), r * 1.4, after - 0.35, &colors, 50, 19);
                burst(&p, c + vec2(r * 1.2, -r * 0.3), r * 1.4, after - 0.55, &colors, 50, 23);
            }
            let pop = crate::casino::ease_back((after / 0.45).min(1.0));
            let label = format!("{m}×");
            let size = r * if big { 0.62 } else { 0.5 } * pop.max(0.05);
            p.circle_filled(c, r * 0.5 * pop.max(0.05), Color32::from_black_alpha((190.0 * fade) as u8));
            glow_text(&p, c, Align2::CENTER_CENTER, &label, theme::bold_font(size), col::GOLD2.gamma_multiply(fade), seg_color(m).gamma_multiply(0.9 * fade));
            let word = match m {
                1000 => "JACKPOT!",
                150 => "MEGA PRIZE!",
                40 => "HUGE PRIZE!",
                15 => "BIG PRIZE!",
                _ => "THE PRIZE",
            };
            glow_text(&p, title_at, Align2::CENTER_CENTER, word, theme::bold_font((r * 0.2).clamp(22.0, 44.0) * pop.max(0.3)), Color32::WHITE.gamma_multiply(fade), seg_color(m).gamma_multiply(0.8 * fade));
            // The prize, counting up from the buy-in.
            let k = ((after - 0.3) / 1.4).clamp(0.0, 1.0);
            let shown = v.rules.buy_in as f64 + (v.prize_pool.saturating_sub(v.rules.buy_in)) as f64 * ease_out(k) as f64;
            if k >= 1.0 && !fx.counted {
                fx.counted = true;
                cues.push(Cue::Chips);
            }
            let words = format!("PRIZE {}", chips_text(shown.round() as u64));
            let font = theme::bold_font((r * 0.17).clamp(20.0, 36.0));
            let galley = p.layout_no_wrap(words.clone(), font.clone(), col::GOLD2);
            let plate = Rect::from_center_size(c + vec2(0.0, r * 1.38), galley.size() + vec2(48.0, 18.0));
            p.rect_filled(plate, theme::r(18), Color32::from_black_alpha((225.0 * fade) as u8));
            p.rect_stroke(plate, theme::r(18), Stroke::new(2.0f32, seg_color(m).gamma_multiply(fade)), StrokeKind::Inside);
            glow_text(&p, plate.center(), Align2::CENTER_CENTER, &words, font, col::GOLD2.gamma_multiply(fade), col::EMBER.gamma_multiply(0.5 * fade));
            p.text(plate.center_bottom() + vec2(0.0, 16.0), Align2::CENTER_CENTER, "Winner takes it all", theme::bold_font(13.0), Color32::WHITE.gamma_multiply(0.8 * fade));
        }
        ui.ctx().request_repaint();
        true
    }

    /// The end of a Flash Pot I played: the winner's moment, big over everything for a few
    /// seconds (click to go on). Their picture crowned in a burst of light, WINNER, the prize
    /// counting up, coins pouring down and fireworks going off; and if it's me, YOU WIN.
    pub(super) fn bomb_finale(&mut self, ctx: &egui::Context, s: &ServerView, area: Rect) {
        let me = s.me.as_ref().map(|m| m.user_id.clone()).unwrap_or_default();
        // A Flash Pot I'm in, seen being played, now over.
        let mut show: Option<TourneyView> = None;
        for (id, v) in &s.poker.tourney {
            if v.bomb.is_none() || !v.entrants.iter().any(|e| e.uid == me) {
                continue;
            }
            let fx = self.poker.bomb_fx.entry(*id).or_default();
            if v.phase == TourneyPhase::Running {
                fx.running = true;
            }
            if v.phase == TourneyPhase::Finished && fx.running && !fx.finale_done {
                if fx.finale.is_none() {
                    fx.finale = Some(Instant::now());
                }
                if show.is_none() {
                    show = Some(v.clone());
                }
            }
        }
        let Some(v) = show else { return };
        let mut fx = self.poker.bomb_fx.get(&v.id).copied().unwrap_or_default();
        let since = fx.finale.map(|t| t.elapsed().as_secs_f32()).unwrap_or(0.0) - FINALE_BEAT_S;
        if since < 0.0 {
            ctx.request_repaint_after(std::time::Duration::from_secs_f32(-since));
            return;
        }
        if since >= FINALE_S {
            fx.finale_done = true;
            self.poker.bomb_fx.insert(v.id, fx);
            return;
        }
        let Some(winner) = v.entrants.iter().find(|e| e.place == Some(1)).cloned() else { return };
        let mine = winner.uid == me;
        let my_place = v.entrants.iter().find(|e| e.uid == me).and_then(|e| e.place);
        let prize = v.payouts.first().copied().unwrap_or(v.prize_pool).max(winner.prize);
        let mut cues = Vec::new();
        if !fx.finale_sounded {
            fx.finale_sounded = true;
            cues.push(if mine { Cue::Jackpot } else { Cue::WinBig });
        }
        let t = ctx.input(|i| i.time) as f32;
        let face = if winner.bot.is_none() { self.face(ctx, s, &winner.uid) } else { None };
        let mut dismiss = false;
        egui::Area::new(egui::Id::new(("bomb-finale", v.id))).order(egui::Order::Foreground).fixed_pos(area.min).show(ctx, |ui| {
            let resp = ui.allocate_rect(Rect::from_min_size(area.min, area.size()), Sense::click());
            if resp.clicked() && since > 1.2 {
                dismiss = true;
            }
            let p = ui.painter_at(area);
            let fade = (since / 0.35).clamp(0.0, 1.0) * ((FINALE_S - since) / 0.8).clamp(0.0, 1.0);
            p.rect_filled(area, 0, Color32::from_black_alpha((215.0 * fade) as u8));
            let c = area.center() - vec2(0.0, area.height() * 0.08);
            let unit = (area.height() * 0.11).clamp(48.0, 96.0);
            rays(&p, c, area.width().max(area.height()), t, Color32::from_rgba_unmultiplied(0xff, 0xc8, 0x50, (26.0 * fade) as u8), 24);
            radial(&p, c, unit * 4.2, Color32::from_rgba_unmultiplied(0xff, 0xb0, 0x40, (90.0 * fade) as u8), Color32::TRANSPARENT);
            // Fireworks going off round the edges, one after another.
            let colors = [col::GOLD2, col::EMBER, Color32::from_rgb(0x6a, 0xe8, 0x8a), Color32::from_rgb(0x6a, 0xa8, 0xff), Color32::from_rgb(0xff, 0x6a, 0xc8), Color32::WHITE];
            for n in 0..14u32 {
                let at = 0.35 + n as f32 * 0.62;
                let k = since - at;
                if (0.0..2.4).contains(&k) {
                    let x = area.left() + area.width() * (0.12 + 0.76 * h01(n, 3));
                    let y = area.top() + area.height() * (0.12 + 0.38 * h01(n, 5));
                    let set = [colors[n as usize % colors.len()], colors[(n as usize + 2) % colors.len()], Color32::WHITE];
                    burst(&p, pos2(x, y), unit * (1.6 + h01(n, 9)), k, &set, 46, n * 31 + 1);
                }
            }
            // Coins pouring down.
            for n in 0..46u32 {
                let delay = h01(n, 11) * 3.0;
                let k = since - delay;
                if k < 0.0 {
                    continue;
                }
                let fall = (area.height() + 80.0) * ((k * (0.32 + h01(n, 13) * 0.3)) % 1.0);
                let x = area.left() + area.width() * h01(n, 17);
                let q = pos2(x + (k * 2.0 + n as f32).sin() * 14.0, area.top() - 40.0 + fall);
                let turn = (k * (4.0 + h01(n, 19) * 5.0)).cos();
                let rr = 9.0 + h01(n, 23) * 7.0;
                let coin = Rect::from_center_size(q, vec2(rr * 2.0 * turn.abs().max(0.12), rr * 2.0));
                p.add(egui::epaint::EllipseShape::filled(coin.center(), coin.size() / 2.0, Color32::from_rgb(0xd8, 0x98, 0x20).gamma_multiply(fade)));
                p.add(egui::epaint::EllipseShape::filled(coin.center(), coin.size() / 2.0 * 0.78, Color32::from_rgb(0xff, 0xd8, 0x5a).gamma_multiply(fade)));
            }
            // The winner: crowned, in a ring of light.
            let pop = crate::casino::ease_back(((since - 0.15) / 0.5).clamp(0.0, 1.0));
            let fr = unit * 0.9 * pop.max(0.01);
            for g in 0..6 {
                p.circle_filled(c, fr + 6.0 + g as f32 * 5.0, Color32::from_rgba_unmultiplied(0xff, 0xd0, 0x60, (22.0 * fade) as u8));
            }
            p.circle_filled(c, fr + 5.0, col::GOLD.gamma_multiply(fade));
            match winner.bot {
                Some(level) => super::table::bot_face(&p, c, fr, &winner.name, level),
                None => crate::app::avatar(&p, c, fr, &winner.uid, &winner.name, false, Color32::BLACK, face),
            }
            let drop = ease_out(((since - 0.55) / 0.4).clamp(0.0, 1.0));
            if drop > 0.0 {
                let cr = Rect::from_center_size(c - vec2(0.0, fr + unit * 0.35 + (1.0 - drop) * unit * 1.5), vec2(unit * 1.1, unit * 0.72));
                crate::icons::crown(&p, cr, Color32::from_rgb(0xff, 0xd7, 0x6a).gamma_multiply(drop * fade));
            }
            // WINNER (or YOU WIN), the prize counting up, and what it was.
            let tpop = crate::casino::ease_back(((since - 0.35) / 0.5).clamp(0.0, 1.0));
            let title = if mine { "YOU WIN!".to_string() } else { format!("{} WINS!", winner.name.to_uppercase()) };
            let ty = c.y + fr + unit * 0.95;
            glow_text(&p, pos2(c.x, ty), Align2::CENTER_CENTER, &title, theme::bold_font(unit * 0.82 * tpop.max(0.05)), col::GOLD2.gamma_multiply(fade), col::EMBER.gamma_multiply(0.8 * fade));
            let k = ((since - 0.8) / 2.2).clamp(0.0, 1.0);
            let shown = (prize as f64 * ease_out(k) as f64).round() as u64;
            if k >= 1.0 && !fx.finale_counted {
                fx.finale_counted = true;
                cues.push(Cue::Chips);
            }
            let count = format!("{} chips", chips_text(shown));
            let cy = ty + unit * 0.95;
            let swell = if k >= 1.0 { 1.0 + 0.06 * ((since - 3.0) * 6.0).sin().max(0.0) } else { 1.0 };
            glow_text(&p, pos2(c.x, cy), Align2::CENTER_CENTER, &count, theme::bold_font(unit * 0.62 * swell), Color32::WHITE.gamma_multiply(fade), col::GOLD.gamma_multiply(0.7 * fade));
            let m = v.bomb.and_then(|b| b.multiplier).unwrap_or(0);
            let sub = format!("Flash Pot · {} buy-in · {}× the buy-in", chips_text(v.rules.buy_in), m);
            p.text(pos2(c.x, cy + unit * 0.6), Align2::CENTER_CENTER, sub, theme::bold_font(15.0), col::GOLD2.gamma_multiply(0.85 * fade));
            if !mine && let Some(place) = my_place {
                p.text(pos2(c.x, cy + unit * 0.6 + 26.0), Align2::CENTER_CENTER, format!("You finished {}", super::tourneys::ordinal(place)), theme::bold_font(14.0), Color32::WHITE.gamma_multiply(0.75 * fade));
            }
            if since > 2.5 {
                p.text(area.center_bottom() - vec2(0.0, 28.0), Align2::CENTER_CENTER, "Click to go on", theme::font(13.0), Color32::from_white_alpha((150.0 * fade) as u8));
            }
        });
        if dismiss {
            fx.finale_done = true;
        }
        self.poker.bomb_fx.insert(v.id, fx);
        self.play_bomb(cues);
        ctx.request_repaint();
    }
}

/// The bang: a flash, a fireball, smoke rings and sparks, and BOOM shaking over the table.
fn boom(p: &egui::Painter, area: Rect, c: Pos2, a: f32, k: f32) {
    let fade = (1.0 - k / BOOM_S).clamp(0.0, 1.0);
    // The flash.
    if k < 0.25 {
        p.rect_filled(area, 0, Color32::from_white_alpha((200.0 * (1.0 - k / 0.25)) as u8));
    }
    let grow = ease_out((k / 0.6).min(1.0));
    radial(p, c, a * (0.3 + 0.9 * grow), Color32::from_rgba_unmultiplied(0xff, 0xb0, 0x30, (230.0 * fade) as u8), Color32::TRANSPARENT);
    radial(p, c, a * (0.15 + 0.5 * grow), Color32::from_rgba_unmultiplied(0xff, 0xf0, 0xb0, (250.0 * fade * fade) as u8), Color32::TRANSPARENT);
    for ring in 0..3 {
        let rr = a * (0.2 + grow * (0.6 + ring as f32 * 0.25));
        p.circle_stroke(c, rr, Stroke::new(6.0 - ring as f32 * 1.5, Color32::from_rgba_unmultiplied(0x60, 0x50, 0x48, (140.0 * fade) as u8)));
    }
    for i in 0..36 {
        let ang = i as f32 * 2.399 + 0.3;
        let speed = 0.55 + ((i * 37 % 11) as f32 / 11.0) * 0.8;
        let d = a * speed * ease_out((k / 1.4).min(1.0));
        let q = c + vec2(ang.cos(), ang.sin()) * d + vec2(0.0, k * k * 60.0);
        p.circle_filled(q, 3.5 * fade + 1.0, Color32::from_rgba_unmultiplied(0xff, 0xd0, 0x60, (255.0 * fade) as u8));
    }
    // BOOM, shaking.
    let shake = vec2((k * 63.0).sin(), (k * 47.0).cos()) * 10.0 * (1.0 - (k / 0.9).min(1.0));
    let size = 70.0 + 50.0 * crate::casino::ease_back((k / 0.4).min(1.0));
    glow_text(p, c + shake, Align2::CENTER_CENTER, "BOOM!", theme::bold_font(size), Color32::from_rgba_unmultiplied(0xff, 0xf2, 0xc0, (255.0 * fade) as u8), Color32::from_rgba_unmultiplied(0xc0, 0x30, 0x10, (200.0 * fade) as u8));
    p.text(c + vec2(0.0, size * 0.75) + shake * 0.5, Align2::CENTER_CENTER, "Everyone's all in, every hand, till one is left", theme::bold_font(16.0), Color32::WHITE.gamma_multiply(fade));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wheel_lands_on_the_multiplier_under_the_pointer() {
        let seg = TAU / WHEEL.len() as f32;
        for (m, _) in pyr_poker::tourney::MULTIPLIERS {
            for spun in [0u64, 1, 7, 12_345, 99_999] {
                let i = landing(m, spun);
                assert_eq!(WHEEL[i], m);
                // At the end of the spin, segment i's middle is at the top (the pointer).
                let a = wheel_angle(i, SPIN_S + 1.0, SPIN_S);
                let mid = (a + i as f32 * seg + seg / 2.0).rem_euclid(TAU);
                assert!(mid < 1e-3 || (TAU - mid) < 1e-3, "{m}× segment {i}: {mid}");
            }
        }
        // Every multiplier the server can pick is on the wheel.
        for (m, _) in pyr_poker::tourney::MULTIPLIERS {
            assert!(WHEEL.contains(&m), "{m}");
        }
        // A small prize lands just past a bigger one (the wheel creeps by it), whenever the
        // wheel has one there; and the pointer's segment is the one landed on.
        for spun in 0..40u64 {
            let i = landing(3, spun);
            assert!(WHEEL[(i + 1) % WHEEL.len()] > 3, "3× after something bigger: {i}");
            let a = wheel_angle(i, SPIN_S + 1.0, SPIN_S);
            assert_eq!(under_pointer(a).0, i);
        }
    }
}
