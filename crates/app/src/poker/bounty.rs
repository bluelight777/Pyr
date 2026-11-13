//! Bounty Poker (0.13.6) at the app: its card in the sit & gos (play one for 1,000 to 100,000
//! chips), the crosshair drawn on it and in the Games menu, and the price on every head at the
//! table.
//!
//! The server keeps the bounties (in the tournament's view, by player); the table shows them.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use eframe::emath::GuiRounding;
use pyr_engine::state::ServerView;
use pyr_poker::net::{ToServer, TourneyId, TourneyPhase, TourneyView};
use pyr_poker::tourney::{BOUNTY_BUY_INS, BOUNTY_STACK, BOUNTY_WAIT_MS};

use super::cards::chips_text;
use crate::app::PyrApp;
use crate::casino::{col, ease_out, glow_text, radial};
use crate::theme;

const TAU: f32 = std::f32::consts::TAU;
/// The Bounty Poker card's colors (0.13.28's): the card, its edge, the line over its name, the
/// glow behind its name, and the words on a gold button.
const CARD_BASE: Color32 = Color32::from_rgb(0x0c, 0x06, 0x08);
const CARD_EDGE: Color32 = Color32::from_rgb(0xa8, 0x2a, 0x22);
const KICKER: Color32 = Color32::from_rgb(0xff, 0x8a, 0x7a);
const ON_GOLD: Color32 = Color32::from_rgb(0x2a, 0x10, 0x04);
const PLAY_INK: Color32 = Color32::from_rgb(0x4a, 0x1a, 0x06);
/// The crosshair's red, the chip's marks and the star on it, and the ring inside its edge.
const RED: Color32 = Color32::from_rgb(0xe8, 0x3a, 0x2c);
const CREAM: Color32 = Color32::from_rgb(0xff, 0xf2, 0xc8);
const STAR: Color32 = Color32::from_rgb(0x9a, 0x4a, 0x08);
const CHIP_RING: Color32 = Color32::from_rgb(0x8a, 0x5a, 0x10);
/// The price on a head at the table: its pill, the pill's edge.
const PILL: Color32 = Color32::from_rgb(0x8a, 0x16, 0x12);
const PILL_EDGE: Color32 = Color32::from_rgb(0xff, 0x9a, 0x7a);
/// The wide glow round the crosshair on the card (premultiplied: it darkens what it's over),
/// and the Games menu picture's dark red.
const WIDE_GLOW: Color32 = Color32::from_rgba_premultiplied(50, 8, 6, 117);
const THUMB: Color32 = Color32::from_rgb(0x2a, 0x08, 0x08);

/// My knockouts at Bounty Poker: what I'd won from bounties when each tournament was last seen,
/// and the one being shown ("BOUNTY +250" over the table, 0.13.28's: only to whoever took it).
#[derive(Default)]
pub struct Knockouts {
    seen: HashMap<TourneyId, u64>,
    shown: Option<(TourneyId, Instant, u64)>,
}

/// How long a knockout shows: the crosshair pops in over its first half second, and it all
/// goes over its last (filmed on 0.13.28: three seconds).
const KO_S: f32 = 3.0;
const KO_IN_S: f32 = 0.5;
const KO_OUT_S: f32 = 0.5;

fn wall() -> u64 {
    pyr_proto::now_ms().max(0) as u64
}

/// The crosshair on a gold chip (0.13.28's, measured off its screen to the pixel but for the
/// dashes' turn): `u` is its size (1 on the card, where the ring is 112 across the middle of its
/// stroke), `t` the time (its outer ring of dashes turns clockwise).
pub fn crosshair(p: &egui::Painter, c: Pos2, u: f32, t: f32) {
    // A red glow round it.
    radial(p, c, 121.0 * u, RED.gamma_multiply(0.2), Color32::TRANSPARENT);
    // The dashes, turning.
    let n = 12;
    let dash_r = 66.95 * u;
    let len = TAU / 30.0;
    let spin = t * 0.35;
    let dash = Stroke::new(3.15 * u, RED.gamma_multiply(0.7));
    for k in 0..n {
        let a0 = spin + k as f32 * TAU / n as f32 - len / 2.0;
        let pts: Vec<Pos2> = (0..=6).map(|j| a0 + len * j as f32 / 6.0).map(|a| c + vec2(a.cos(), a.sin()) * dash_r).collect();
        p.add(Shape::line(pts, dash));
    }
    // The ring (egui strokes a circle outside its radius), and its four ticks.
    p.circle_stroke(c, 53.33 * u, Stroke::new(5.57 * u, RED));
    let (t0, t1) = (38.25 * u, 63.2 * u);
    let tick = Stroke::new(5.6 * u, RED);
    for (dx, dy) in [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)] {
        let d = vec2(dx, dy);
        p.line_segment([c + d * t0, c + d * t1], tick);
    }
    // The chip: its shadow, the gold, eight marks across its edge, a ring inside them, and a
    // star.
    let r = 29.75 * u;
    p.circle_filled(c + vec2(0.0, 2.5 * u), 31.0 * u, Color32::from_black_alpha(90));
    p.circle_filled(c, r, col::GOLD);
    let (m0, m1, half) = (21.8 * u, 29.65 * u, 2.15 * u);
    for k in 0..8 {
        let a = (k as f32 + 0.5) * TAU / 8.0;
        let (d, across) = (vec2(a.cos(), a.sin()), vec2(-a.sin(), a.cos()) * half);
        p.add(Shape::convex_polygon(vec![c + d * m0 - across, c + d * m1 - across, c + d * m1 + across, c + d * m0 + across], CREAM, Stroke::NONE));
    }
    p.circle_stroke(c, 21.7 * u, Stroke::new(1.85 * u, CHIP_RING));
    star(p, c, 14.875 * u, 6.2 * u, STAR);
}

/// A five-pointed star, point up: `big` to its points, `small` to the corners between them.
fn star(p: &egui::Painter, c: Pos2, big: f32, small: f32, color: Color32) {
    let at = |k: usize, rr: f32| {
        let a = -TAU / 4.0 + k as f32 * TAU / 10.0;
        c + vec2(a.cos(), a.sin()) * rr
    };
    // (Ten triangles from the middle, as 0.13.28 draws it: a star isn't a convex shape.)
    for k in 0..10 {
        let (a, b) = if k % 2 == 0 { (at(k, big), at(k + 1, small)) } else { (at(k, small), at(k + 1, big)) };
        p.add(Shape::convex_polygon(vec![c, a, b], color, Stroke::NONE));
    }
}

/// Bounty Poker's picture in the Games menu (0.13.28's): the crosshair, small, on its dark red
/// (`radius`: the menu's pictures' corners). Not dimmed like the others when not pointed at.
pub fn thumb(p: &egui::Painter, r: Rect, radius: u8, t: f32) {
    let p = p.with_clip_rect(r.intersect(p.clip_rect()));
    p.rect_filled(r, egui::CornerRadius::same(radius), THUMB);
    crosshair(&p, r.center(), 0.2214, t);
}

/// The price on a player's head, on a pill at the foot of their name plate (0.13.28's: its
/// middle 2 under the plate's foot), `k` the table's size (1: the plate 164 by 50; 0.13.28
/// draws it at 0.84 in a window 1280 by 800, at 1.1 in 1600 by 1000, and so the pill: 74 by 14
/// there, 98 by 17 here).
pub fn bounty_pill(p: &egui::Painter, plate_foot: Pos2, bounty: u64, k: f32, fade: f32) {
    let text = p.layout_no_wrap(format!("BOUNTY {}", chips_text(bounty)), theme::bold_font(9.5 * k), Color32::WHITE);
    let w = text.size().x + 27.0 * k;
    // (On whole pixels, as egui fills it: the ring's and the words' places come from it.)
    let r = Rect::from_center_size(plate_foot + vec2(0.0, 2.0 * k), vec2(w, 16.0 * k)).round_to_pixels(p.pixels_per_point());
    let round = egui::CornerRadius::same((r.height() / 2.0).round() as u8);
    p.rect_filled(r, round, PILL.gamma_multiply(fade));
    p.rect_stroke(r, round, Stroke::new(1.0f32, PILL_EDGE.gamma_multiply(fade)), StrokeKind::Inside);
    let ic = pos2(r.left() + 9.0 * k, r.center().y);
    p.circle_stroke(ic, 4.2 * k, Stroke::new(1.2 * k, Color32::WHITE.gamma_multiply(fade)));
    p.circle_filled(ic, 1.4 * k, col::GOLD2.gamma_multiply(fade));
    p.galley(pos2(r.left() + 17.0 * k, r.center().y - text.size().y / 2.0), text, Color32::WHITE.gamma_multiply(fade));
}

impl PyrApp {
    /// At a Bounty Poker table: a knockout of mine, as 0.13.28 shows it to whoever took the
    /// head: the crosshair pops up over the top seat and "BOUNTY +250" under it, for three
    /// seconds (`top`: the top seat's middle; `k`: the table's size).
    pub fn bounty_overlay(&mut self, ui: &mut egui::Ui, s: &ServerView, tid: TourneyId, top: Pos2, k: f32) {
        let me = s.me.as_ref().map(|m| m.user_id.as_str()).unwrap_or_default();
        if let Some(v) = s.poker.tourney.get(&tid)
            && let Some(e) = v.entrants.iter().find(|e| e.uid == me)
        {
            let before = self.poker.knockouts.seen.insert(tid, e.bounties_won);
            // (Not the winner's own bounty at the end: that comes with the prize.)
            if let Some(before) = before
                && e.bounties_won > before
                && v.phase == TourneyPhase::Running
                && e.place.is_none()
            {
                self.poker.knockouts.shown = Some((tid, Instant::now(), e.bounties_won - before));
            }
        }
        let Some((shown_tid, at, amount)) = self.poker.knockouts.shown else { return };
        let age = at.elapsed().as_secs_f32();
        if shown_tid != tid || age >= KO_S {
            self.poker.knockouts.shown = None;
            return;
        }
        ui.ctx().request_repaint();
        let p = ui.painter();
        let t = ui.ctx().input(|i| i.time) as f32;
        let pop = crate::casino::ease_back((age / KO_IN_S).min(1.0));
        let out = ((age - (KO_S - KO_OUT_S)) / KO_OUT_S).clamp(0.0, 1.0);
        let size = pop * (1.0 - ease_out(out));
        if size > 0.01 {
            crosshair(p, top, 0.57 * k * size, t);
        }
        let alpha = (age / 0.1).min(1.0) * (1.0 - out);
        let glow = Color32::from_rgba_unmultiplied(0xe8, 0x20, 0x10, 110).gamma_multiply(alpha);
        glow_text(p, top + vec2(0.0, 70.0 * k), Align2::CENTER_CENTER, &format!("BOUNTY +{}", chips_text(amount)), theme::bold_font(45.0 * k), col::GOLD2.gamma_multiply(alpha), glow);
    }

    /// The Bounty Poker card in the sit & gos (0.13.6), as 0.13.28 draws it: what it is, a Play
    /// button for each buy-in, the one I'm waiting in (Leave, its six seats, the bots'
    /// countdown), or the one I'm playing; the crosshair turning on the right.
    pub fn bounty_card(&mut self, ui: &mut egui::Ui, s: &ServerView) {
        let me = s.me.as_ref().map(|m| m.user_id.clone()).unwrap_or_default();
        let mine: Option<TourneyView> = s
            .poker
            .tourney
            .values()
            .filter(|v| v.rules.bounty.is_some() && v.entrants.iter().any(|e| e.uid == me && e.place.is_none()) && matches!(v.phase, TourneyPhase::Registering | TourneyPhase::Running))
            .max_by_key(|v| v.id)
            .cloned();
        let hunting = s.poker.tourneys.as_ref().map(|l| l.iter().filter(|x| x.rules.bounty.is_some() && x.phase == TourneyPhase::Running).count()).unwrap_or(0);
        let ctx = ui.ctx().clone();
        let t = ctx.input(|i| i.time) as f32;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(28.0);
            let w = (ui.available_width() - 28.0).max(320.0);
            let desc = ui.painter().layout(
                format!(
                    "Six players, {} chips each, a turbo. Half of every buy-in is the price on that player's head: knock someone out and half their bounty is yours on the spot, the other half goes onto your head. The top two share the prize pool, and the last one standing collects their own bounty too.",
                    chips_text(BOUNTY_STACK)
                ),
                theme::font(13.0),
                col::TEXT,
                w * 0.5,
            );
            let by = (76.0 + desc.size().y + 16.0).max(146.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, by + 50.0), Sense::hover());
            let p = ui.painter().clone();
            // Its shadow; the card; the crosshair's glow (over the card's edges, and past them);
            // its edge.
            p.rect_filled(r.translate(vec2(0.0, 8.0)), theme::r(16), Color32::from_black_alpha(115));
            p.rect_filled(r, theme::r(16), CARD_BASE);
            let c = pos2(r.right() - 150.0, r.center().y);
            // (A wide dark red glow round the crosshair, over the card's edge and past it: it
            // darkens the Bomb Pot card's corner above, as 0.13.28's does.)
            radial(&p, c, 260.0, WIDE_GLOW, Color32::TRANSPARENT);
            crosshair(&p, c, 1.0, t);
            p.rect_stroke(r, theme::r(16), Stroke::new(1.5f32, CARD_EDGE), StrokeKind::Inside);
            ctx.request_repaint();
            let x = r.left() + 26.0;
            let kicker = if hunting > 0 { format!("SIT & GO · {hunting} HUNTING NOW") } else { "SIT & GO · 6 PLAYERS".to_string() };
            p.text(pos2(x, r.top() + 17.0), Align2::LEFT_TOP, kicker, theme::bold_font(12.5), KICKER);
            let glow = Color32::from_rgba_unmultiplied(0xe8, 0x20, 0x10, 110);
            glow_text(&p, pos2(x, r.top() + 54.0), Align2::LEFT_CENTER, "BOUNTY POKER", theme::bold_font(32.0), col::GOLD2, glow);
            p.galley(pos2(x, r.top() + 76.0), desc, col::TEXT);
            let row = Rect::from_min_size(pos2(x, r.top() + by), vec2(r.right() - 26.0 - x, 44.0));
            let cy = row.center().y;
            match &mine {
                Some(v) if v.phase == TourneyPhase::Registering => {
                    let b = Rect::from_min_size(pos2(x, cy - 18.0), vec2(90.0, 36.0));
                    if ui.put(b, egui::Button::new("Leave").corner_radius(theme::r(10))).on_hover_text("Your buy-in back").clicked() {
                        self.engine.poker(&s.key, ToServer::TourneyUnregister { id: v.id });
                    }
                    for k in 0..v.rules.max_players {
                        let at = pos2(x + 120.0 + 38.0 * k as f32, cy);
                        let e = v.entrants.get(k);
                        // (Who's in: their picture in a red ring.)
                        p.circle_filled(at, 15.0, if e.is_some() { RED } else { Color32::from_white_alpha(14) });
                        if e.is_some() {
                            p.circle_stroke(at, 14.8, Stroke::new(1.85f32, col::GOLD2));
                        } else {
                            p.circle_stroke(at, 15.0, Stroke::new(1.5f32, Color32::from_white_alpha(50)));
                        }
                        if let Some(e) = e {
                            match e.bot {
                                Some(level) => super::table::bot_face(&p, at, 14.0, &e.name, level),
                                None => {
                                    let face = self.face(&ctx, s, &e.uid);
                                    crate::app::avatar(&p, at, 14.0, &e.uid, &e.name, false, theme::SIDE(), face);
                                }
                            }
                        }
                    }
                    let n = v.entrants.len();
                    let bots_in = (v.rules.start_ms.saturating_sub(wall()) / 1000).min(BOUNTY_WAIT_MS / 1000);
                    let tx = x + 120.0 + 38.0 * v.rules.max_players as f32;
                    p.text(pos2(tx, cy - 9.0), Align2::LEFT_CENTER, format!("Waiting for players · {n} of {}", v.rules.max_players), theme::bold_font(15.0), col::GOLD2);
                    p.text(pos2(tx, cy + 11.0), Align2::LEFT_CENTER, format!("Bots take any empty seats in 0:{bots_in:02}"), theme::font(12.5), col::MUTED);
                    ui.ctx().request_repaint_after(Duration::from_millis(250));
                }
                Some(v) => {
                    let b = Rect::from_min_size(pos2(x, row.top() + 4.0), vec2(170.0, 38.0));
                    if let Some(t) = v.my_table
                        && ui.put(b, theme::primary_button("Go to my table")).clicked()
                    {
                        self.open_table(&s.key, t);
                    }
                    let mine = v.entrants.iter().find(|e| e.uid == me);
                    let line = mine.map(|e| format!("The price on your head: {} · bounties won: {}", chips_text(e.bounty), chips_text(e.bounties_won))).unwrap_or_default();
                    p.text(pos2(b.right() + 16.0, cy - 8.0), Align2::LEFT_CENTER, "You're in Bounty Poker", theme::bold_font(15.0), col::GOLD2);
                    p.text(pos2(b.right() + 16.0, cy + 12.0), Align2::LEFT_CENTER, line, theme::font(12.5), col::MUTED);
                }
                None => {
                    let chips = s.poker.me.as_ref().map(|m| m.chips).unwrap_or(0);
                    for (k, buy_in) in BOUNTY_BUY_INS.iter().enumerate() {
                        let b = Rect::from_min_size(pos2(x + 128.0 * k as f32, row.top()), vec2(118.0, 44.0));
                        let can = chips >= *buy_in;
                        let resp = ui.interact(b, ui.id().with(("bounty", *buy_in)), Sense::click());
                        p.rect_filled(b.translate(vec2(0.0, 3.0)), theme::r(12), Color32::from_black_alpha(128));
                        let fill = if !can { Color32::from_white_alpha(14) } else if resp.hovered() { col::GOLD2 } else { col::GOLD };
                        p.rect_filled(b, theme::r(12), fill);
                        let ink = if can { ON_GOLD } else { col::MUTED };
                        p.text(b.center() - vec2(0.0, 8.0), Align2::CENTER_CENTER, "PLAY", theme::bold_font(10.5), if can { PLAY_INK } else { col::MUTED });
                        p.text(b.center() + vec2(0.0, 8.0), Align2::CENTER_CENTER, chips_text(*buy_in), theme::bold_font(17.0), ink);
                        let tip = if can { format!("{} chips to play; {} on every head", chips_text(*buy_in), chips_text(*buy_in / 2)) } else { format!("You need {} chips", chips_text(*buy_in)) };
                        if resp.on_hover_text(tip).clicked() && can {
                            self.engine.poker(&s.key, ToServer::Bounty { buy_in: *buy_in });
                        }
                    }
                }
            }
        });
    }
}
