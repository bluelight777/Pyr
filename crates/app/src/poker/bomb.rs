//! Time Bombs at the app: the card in the tournaments (play one for 100, 1,000 or 10,000 chips),
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
/// How long the wheel turns (its sound's ding lands on the stop).
const SPIN_S: f32 = 4.05;
/// The bang, then the warning stays.
const BOOM_S: f32 = 2.6;

/// What's been played for each Time Bomb: the wheel, the fuse's hiss, the last second ticked,
/// the bang (and when it was first seen).
#[derive(Default, Clone, Copy)]
pub struct BombFx {
    wheel: bool,
    fuse: bool,
    tick: u64,
    boom: Option<Instant>,
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

/// The prize wheel at `angle`, centered on `c`.
fn prize_wheel(p: &egui::Painter, c: Pos2, r: f32, angle: f32, lit: Option<usize>, t: f32) {
    p.circle_filled(c + vec2(0.0, r * 0.05), r * 1.07, Color32::from_black_alpha(140));
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
            fill = crate::casino::mix(fill, Color32::WHITE, 0.35);
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
    // The pointer, at the top.
    let top = c - vec2(0.0, r * 1.02);
    p.add(Shape::convex_polygon(vec![top + vec2(-r * 0.09, -r * 0.12), top + vec2(r * 0.09, -r * 0.12), top + vec2(0.0, r * 0.1)], col::GOLD2, Stroke::new(2.0f32, Color32::from_rgb(0x5a, 0x34, 0x10))));
}

/// Which of the wheel's segments the spin lands on (one showing the multiplier, picked by when
/// it was spun so it isn't always the same one).
fn landing(multiplier: u32, spun_ms: u64) -> usize {
    let spots: Vec<usize> = WHEEL.iter().enumerate().filter(|(_, m)| **m == multiplier).map(|(i, _)| i).collect();
    if spots.is_empty() {
        return 0;
    }
    spots[(spun_ms / 7 % spots.len() as u64) as usize]
}

/// The wheel's angle `s` seconds into its spin, landing on segment `i`.
fn wheel_angle(i: usize, s: f32) -> f32 {
    let seg = TAU / WHEEL.len() as f32;
    // Segment i's middle at the top: angle = -(i + 1/2) segments; and six turns to get there.
    let end = -(i as f32 + 0.5) * seg + 6.0 * TAU;
    let k = (s / SPIN_S).clamp(0.0, 1.0);
    let e = 1.0 - (1.0 - k).powi(4);
    end * e
}

impl PyrApp {
    /// The Time Bomb card at the top of the tournaments: what it is, a Play button for each
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
            glow_text(&p, pos2(x, r.top() + 30.0), Align2::LEFT_CENTER, "TIME BOMB", theme::bold_font(24.0), col::GOLD2, Color32::from_rgba_unmultiplied(0xff, 0x50, 0x10, 90));
            p.text(pos2(x + 168.0, r.top() + 31.0), Align2::LEFT_CENTER, if playing_now > 0 { format!("{playing_now} going off now") } else { format!("Sit & go · {BOMB_SEATS} players") }, theme::bold_font(12.0), col::MUTED);
            // (What it says ends before the right side's buttons and words.)
            let hint = mine.is_none().then(|| p.layout_no_wrap("Starts with five in; bots fill empty seats after 30 s".to_string(), theme::font(12.5), col::MUTED));
            let right_w = hint.as_ref().map(|g| g.size().x).unwrap_or(0.0).max(356.0) + 24.0;
            let text_w = (r.right() - 20.0 - right_w - x).max(240.0);
            let galley = p.layout(
                format!(
                    "Five players, 500 chips each, the blinds up every minute. The wheel picks the prize, up to 1,000 times the buy-in, and the winner takes it all. Get the chip lead before the fuse burns down: after {} minutes the bomb goes off and every hand is all in.",
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
                    p.text(pos2(right, r.top() + 44.0), Align2::RIGHT_CENTER, "You're in a Time Bomb", theme::bold_font(16.0), col::GOLD2);
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

    /// At a Time Bomb's table: the wheel while it spins, then the fuse and the prize over the
    /// felt; the bang when it goes off; and their sounds.
    pub fn bomb_overlay(&mut self, ui: &mut egui::Ui, s: &ServerView, tid: TourneyId, felt_c: Pos2, felt_a: f32, felt_b: f32) {
        let Some(v) = s.poker.tourney.get(&tid) else { return };
        let Some(b) = v.bomb else { return };
        let now = wall();
        let t = ui.ctx().input(|i| i.time) as f32;
        let p = ui.painter().clone();
        let mut fx = self.poker.bomb_fx.get(&tid).copied().unwrap_or_default();
        let mut cues: Vec<Cue> = Vec::new();
        // The wheel: from the spin until a little after the first hand's due.
        if let (Some(m), Some(spun)) = (b.multiplier, b.spun_ms) {
            let since = now.saturating_sub(spun) as f32 / 1000.0;
            let show_until = WHEEL_MS as f32 / 1000.0 + 2.2;
            if since < show_until {
                if !fx.wheel && since < 1.0 {
                    cues.push(Cue::Wheel);
                }
                fx.wheel = true;
                let i = landing(m, spun);
                let r = (felt_b * 0.95).clamp(90.0, 190.0);
                let c = felt_c - vec2(0.0, felt_b * 0.05);
                let fade = ((show_until - since) / 0.5).clamp(0.0, 1.0);
                p.circle_filled(c, r * 1.6, Color32::from_black_alpha((120.0 * fade) as u8));
                let landed = since >= SPIN_S;
                prize_wheel(&p, c, r, wheel_angle(i, since), landed.then_some(i), t);
                if landed {
                    let k = ((since - SPIN_S) / 0.4).min(1.0);
                    let pop = crate::casino::ease_back(k);
                    let words = format!("{m}×  ·  PRIZE {}", chips_text(v.prize_pool));
                    let galley = p.layout_no_wrap(words.clone(), theme::bold_font(26.0 * pop.max(0.3)), col::GOLD2);
                    let plate = Rect::from_center_size(c + vec2(0.0, r * 1.25), galley.size() + vec2(40.0, 16.0));
                    p.rect_filled(plate, theme::r(16), Color32::from_black_alpha((220.0 * fade) as u8));
                    p.rect_stroke(plate, theme::r(16), Stroke::new(1.5f32, col::GOLD.gamma_multiply(fade)), StrokeKind::Inside);
                    glow_text(&p, plate.center(), Align2::CENTER_CENTER, &words, theme::bold_font(26.0 * pop.max(0.3)), col::GOLD2.gamma_multiply(fade), col::EMBER.gamma_multiply(0.5 * fade));
                } else {
                    p.text(c + vec2(0.0, r * 1.22), Align2::CENTER_CENTER, "Spinning for the prize…", theme::bold_font(16.0), Color32::WHITE);
                }
                self.poker.bomb_fx.insert(tid, fx);
                self.play_bomb(cues);
                return;
            }
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
            let vol = if c == Cue::Boom { 1.0 } else { 0.8 };
            self.engine.play_poker(c, 0.0, vol);
        }
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
                let a = wheel_angle(i, SPIN_S + 1.0);
                let mid = (a + i as f32 * seg + seg / 2.0).rem_euclid(TAU);
                assert!(mid < 1e-3 || (TAU - mid) < 1e-3, "{m}× segment {i}: {mid}");
            }
        }
        // Every multiplier the server can pick is on the wheel.
        for (m, _) in pyr_poker::tourney::MULTIPLIERS {
            assert!(WHEEL.contains(&m), "{m}");
        }
    }
}
