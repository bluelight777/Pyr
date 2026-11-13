//! Emotes at the tables: eighteen stickers made with Meshy (the Pyr flame's nine faces, and nine
//! casino ones: GG, ALL IN, aces, a crying chip, laughing dice, a cool chip, a bad beat, a bag of
//! chips, the crown), sent as `e:<name>` and shown big over the sender's seat: popping in, a
//! bounce, a float up, and gone. And fourteen that move (0.13.23), from their own sheet of
//! Meshy's pictures (eight by eight): each has its little act over the seat, its pictures in
//! turn and its tears, steam, notes or Zs, as 0.13.28 plays them (filmed there and fitted
//! picture by picture: `rebuild/measured/emotes.md`). At poker, blackjack, craps, baccarat and
//! roulette alike.

use std::f32::consts::{PI, TAU};
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, emath::Rot2, pos2, vec2};
use pyr_poker::net::TableGame;

use super::{col, ease_back, pics, radial};
use crate::theme;

/// Each sticker: its name on the wire (after `e:`), and what it says (a tooltip).
pub const EMOTES: [(&str, &str); 18] = [
    ("laugh", "Crying laughing"),
    ("cry", "Crying"),
    ("cool", "Cool"),
    ("wow", "Wow"),
    ("angry", "Steaming"),
    ("think", "Hmm"),
    ("nice", "Nice!"),
    ("love", "Love it"),
    ("rich", "Rich"),
    ("gg", "GG"),
    ("allin", "All in"),
    ("aces", "Aces"),
    ("sob", "Bad chip day"),
    ("lol", "Dice LOL"),
    ("boss", "Boss chip"),
    ("badbeat", "Bad beat"),
    ("bag", "Bag of chips"),
    ("king", "King of the table"),
];

/// The ones that move: their names on the wire (after `e:`) and what they say, as 0.13.28's.
pub const MOVING: [(&str, &str); 14] = [
    ("rofl", "Rolling on the floor laughing"),
    ("river", "Crying a river"),
    ("mindblown", "Mind blown"),
    ("rage", "Rage"),
    ("shades", "Deal with it"),
    ("rain", "Making it rain"),
    ("fire", "On fire"),
    ("dance", "Victory dance"),
    ("clown", "Clown"),
    ("fish", "Fish!"),
    ("popcorn", "Popcorn"),
    ("salty", "Salty"),
    ("sleep", "Wake me up"),
    ("chicken", "Chicken"),
];

/// Each moving one's pictures on its sheet: the first, how many, and the one it shows standing
/// still (in the picker).
const MOVES: [(usize, usize, usize); 14] = [
    (0, 4, 3),
    (4, 4, 1),
    (8, 4, 2),
    (12, 6, 3),
    (18, 3, 2),
    (22, 4, 2),
    (26, 4, 1),
    (30, 4, 0),
    (34, 4, 1),
    (38, 4, 3),
    (42, 6, 5),
    (48, 4, 2),
    (52, 4, 3),
    (56, 4, 2),
];

/// The shades on their own (that come down for "Deal with it").
const SHADES: usize = 21;

/// "Wake me up"'s Zs, as 0.13.28's: when each comes (k), from where and how fast (in the emote's
/// size, about its middle; measured, the same in every film of it).
const ZS: [(f32, egui::Vec2, egui::Vec2); 5] = [
    (0.12, vec2(0.304, -0.367), vec2(0.278, -0.448)),
    (0.28, vec2(0.242, -0.403), vec2(0.17, -0.51)),
    (1.40, vec2(0.242, -0.394), vec2(0.269, -0.439)),
    (1.70, vec2(0.304, -0.358), vec2(0.295, -0.439)),
    (1.88, vec2(0.233, -0.385), vec2(0.179, -0.51)),
];

/// The ones on the bar (the rest are a click away), as 0.13.28's: three of them move.
pub const QUICK: [&str; 6] = ["gg", "nice", "rofl", "rage", "shades", "allin"];

/// How long one shows over a seat.
pub const SHOW_S: f32 = 3.2;

/// How much bigger one that moves is than a sticker.
const MOVING_SCALE: f32 = 1.1;

/// A sticker or one that moves, from what's sent (`e:gg`, `e:rofl`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Emote {
    Sticker(usize),
    Moving(usize),
}

pub fn which(emote: &str) -> Option<Emote> {
    let name = emote.strip_prefix("e:")?;
    EMOTES.iter().position(|(n, _)| *n == name).map(Emote::Sticker).or_else(|| MOVING.iter().position(|(n, _)| *n == name).map(Emote::Moving))
}

pub fn wire(e: Emote) -> String {
    match e {
        Emote::Sticker(i) => format!("e:{}", EMOTES[i].0),
        Emote::Moving(j) => format!("e:{}", MOVING[j].0),
    }
}

fn words(e: Emote) -> &'static str {
    match e {
        Emote::Sticker(i) => EMOTES[i].1,
        Emote::Moving(j) => MOVING[j].1,
    }
}

/// Sticker `i` drawn in `r`.
pub fn draw(p: &egui::Painter, ctx: &egui::Context, i: usize, r: Rect, tint: Color32) {
    let (cx, cy) = ((i % 6) as f32, (i / 6) as f32);
    let uv = Rect::from_min_max(pos2(cx / 6.0, cy / 3.0), pos2((cx + 1.0) / 6.0, (cy + 1.0) / 3.0));
    let px = ((r.width() * 6.0).max(384.0) as u32).min(1152);
    pics::part(p, ctx, "emotes", uv, r, px, tint);
}

/// The moving ones' sheet, made for pictures about `base` across (one texture for the whole
/// act, whatever it grows to: from the size it settles at). None while it's still being read.
fn sheet(ctx: &egui::Context, base: f32) -> Option<egui::TextureHandle> {
    pics::tex(ctx, "emotes-anim", ((base * 8.0).max(512.0) as u32).min(1792))
}

/// Picture `i` of the moving ones' sheet, `size` across about `c`, turned `turn` (radians,
/// clockwise), from the sheet made for `base`. False while the sheet's still being read.
fn picture(p: &egui::Painter, ctx: &egui::Context, i: usize, c: Pos2, size: f32, base: f32, turn: f32, tint: Color32) -> bool {
    let (x, y) = ((i % 8) as f32, (i / 8) as f32);
    let uv = Rect::from_min_max(pos2(x / 8.0, y / 8.0), pos2((x + 1.0) / 8.0, (y + 1.0) / 8.0));
    let Some(t) = sheet(ctx, base) else { return false };
    let mut m = egui::Mesh::with_texture(t.id());
    m.add_rect_with_uv(Rect::from_center_size(c, vec2(size, size)), uv, tint);
    if turn != 0.0 {
        m.rotate(Rot2::from_angle(turn), c);
    }
    p.add(Shape::mesh(m));
    true
}

/// An emote over a seat at `at`, `k` seconds after it came: a pop, a bounce, a float up, gone
/// (one that moves: its act, after the same pop and before the same fade). (Anything else is
/// drawn as it came: an emoji or a word, as before.)
pub fn pop(p: &egui::Painter, ctx: &egui::Context, emote: &str, at: Pos2, k: f32, size: f32) {
    if !(0.0..SHOW_S).contains(&k) {
        return;
    }
    let fade = (1.0 - (k - (SHOW_S - 0.6)).max(0.0) / 0.6).clamp(0.0, 1.0);
    let lift = 26.0 * (k / SHOW_S);
    let c = at - vec2(0.0, lift);
    let grow = ease_back((k / 0.38).min(1.0));
    match which(emote) {
        Some(Emote::Sticker(i)) => {
            // (A little bounce after landing, then it breathes.)
            let bob = if k > 0.38 { 1.0 + 0.05 * ((k - 0.38) * 9.0).sin() * (-(k - 0.38) * 2.2).exp() } else { 1.0 };
            let s = size * grow.max(0.05) * bob;
            radial(p, c, s * 0.75, col::GOLD.gamma_multiply(0.28 * fade), Color32::TRANSPARENT);
            draw(p, ctx, i, Rect::from_center_size(c, vec2(s, s)), Color32::WHITE.gamma_multiply(fade));
        }
        Some(Emote::Moving(j)) => {
            // (A tenth bigger than a sticker, glow and all: their pictures leave more room round
            // them. Measured: 111.7 px where a sticker's 101.6.)
            let s = size * MOVING_SCALE * grow.max(0.05);
            radial(p, c, s * 0.75, col::GOLD.gamma_multiply(0.28 * fade), Color32::TRANSPARENT);
            act(p, ctx, j, k, c, s, size * MOVING_SCALE, fade);
        }
        None => {
            p.circle_filled(c, size * 0.3, Color32::from_black_alpha((160.0 * fade) as u8));
            crate::emoji::paint_text(p, c, Align2::CENTER_CENTER, emote, theme::bold_font(size * 0.26), Color32::WHITE.gamma_multiply(fade), None);
        }
    }
}

/// A number from 0 to 1 for little thing `i`'s `k`th choice (the same every frame, everywhere).
fn rnd(i: u32, k: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9e37_79b1) ^ k.wrapping_mul(0x85eb_ca77) ^ 0x2545_f491;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297a_2d39);
    x ^= x >> 15;
    x as f32 / u32::MAX as f32
}

/// Little things given out one every `every` seconds from `from` on (until `until`), each living
/// `life`: for each one about at `k`, its number and how far through its life it is (0 to 1).
fn spray(k: f32, from: f32, until: f32, every: f32, life: f32) -> impl Iterator<Item = (u32, f32)> {
    let k0 = k - from;
    let last = if k0 < 0.0 { -1 } else { ((k0.min(until - from)) / every).floor() as i64 };
    let first = ((k0 - life) / every).ceil().max(0.0) as i64;
    (first..=last).filter_map(move |n| {
        let age = (k0 - n as f32 * every) / life;
        (0.0..1.0).contains(&age).then_some((n as u32, age))
    })
}

/// How much of a little thing shows at `age` (0 to 1 through its life): in quickly, out slowly.
fn shows(age: f32) -> f32 {
    (age / 0.12).min(1.0) * (1.0 - ((age - 0.55) / 0.45).max(0.0))
}

fn with_alpha(c: Color32, a: f32) -> Color32 {
    let [r, g, b, _] = c.to_array();
    Color32::from_rgba_unmultiplied(r, g, b, (a.clamp(0.0, 1.0) * 255.0) as u8)
}

const TEAR: Color32 = Color32::from_rgb(0x74, 0xcb, 0xf6);
const STEAM: Color32 = Color32::from_rgb(0xf0, 0xf0, 0xf0);
const SMOKE: Color32 = Color32::from_rgb(0x96, 0x96, 0x96);
const BILL: Color32 = Color32::from_rgb(0x4c, 0xa6, 0x5a);
const COIN: Color32 = Color32::from_rgb(0xf2, 0xc2, 0x3e);

/// A little thing thrown at `v` (face sizes a second) from `from`, falling at `g`, `t` seconds on.
fn thrown(from: Pos2, v: egui::Vec2, g: f32, t: f32, s: f32) -> Pos2 {
    from + (v * t + vec2(0.0, 0.5 * g * t * t)) * s
}

/// Moving one `j`'s act `k` seconds in, about `c`, `s` across (settling at `base`), faded to
/// `fade`. Each as 0.13.28 plays it (filmed there at 60 frames a second and fitted picture by
/// picture: `rebuild/measured/emotes.md`); its little things (tears, steam, sparks...) as many,
/// as big, where and as fast as 0.13.28's, though not the very same ones.
fn act(p: &egui::Painter, ctx: &egui::Context, j: usize, k: f32, c: Pos2, s: f32, base: f32, fade: f32) {
    let (first, count, _) = MOVES[j];
    let tint = Color32::WHITE.gamma_multiply(fade);
    let loop_at = |fps: f32| first + (k * fps) as usize % count;
    let face = |i: usize, at: Pos2, size: f32, turn: f32| {
        picture(p, ctx, i, at, size, base, turn, tint);
    };
    let dot = |at: Pos2, r: f32, color: Color32, a: f32| p.circle_filled(at, r, with_alpha(color, a * fade));
    match MOVING[j].0 {
        "rofl" => {
            // Rolling: tipping side to side and back, hopping as it goes, tears coming off its
            // eyes (over its face, a few falling away).
            let a = TAU * 1.3 * k;
            let at = c + vec2(0.092 * s * a.sin(), -0.0386 * s * (2.0 * a).sin().abs());
            let turn = 0.417 * a.sin();
            face(loop_at(6.0), at, s, turn);
            let rot = Rot2::from_angle(turn);
            for (n, age) in spray(k, 0.12, SHOW_S, 0.085, 0.7) {
                let side = if n % 2 == 0 { -1.0 } else { 1.0 };
                let eye = at + rot * vec2(side * (0.12 + 0.08 * rnd(n, 3)), -0.09) * s;
                let v = vec2(side * (0.15 + 0.25 * rnd(n, 1)), -(0.1 + 0.25 * rnd(n, 2)));
                dot(thrown(eye, v, 1.2, age * 0.7, s), s * 0.018, TEAR, shows(age));
            }
        }
        "river" => {
            // Sobbing (a little shake), tears running from its eyes down its face and off it.
            let at = c + vec2(0.012 * s * (TAU * 7.0 * k).sin(), 0.0);
            face(loop_at(4.0), at, s, 0.0);
            for (n, age) in spray(k, 0.08, SHOW_S, 0.075, 0.85) {
                let side = if n % 2 == 0 { -1.0 } else { 1.0 };
                let eye = at + vec2(side * (0.1 + 0.05 * rnd(n, 1)), -0.03) * s;
                let v = vec2(side * 0.03 * rnd(n, 2), 0.05 + 0.1 * rnd(n, 3));
                dot(thrown(eye, v, 1.4, age * 0.85, s), s * 0.016, TEAR, shows(age));
            }
        }
        "mindblown" => {
            // Shocked, cracking, the blast (it shakes and jumps, a gold ring goes out), burnt out
            // and smoking.
            let i = first
                + match k {
                    k if k < 0.55 => 0,
                    k if k < 0.92 => 1,
                    k if k < 1.5 => 2,
                    _ => 3,
                };
            let mut at = c;
            if (0.92..1.58).contains(&k) {
                at.x += 0.031 * s * (TAU * 6.74 * k).sin();
            }
            if (0.92..1.44).contains(&k) {
                at.y -= 0.036 * s * (PI * (k - 0.92) / 0.52).sin();
            } else if (1.44..1.61).contains(&k) {
                at.y += 0.012 * s * (PI * (k - 1.44) / 0.17).sin();
            }
            face(i, at, s, 0.0);
            if (0.92..1.53).contains(&k) {
                let u = (k - 0.92) / 0.61;
                p.circle_stroke(at, s * (0.48 + 0.985 * (k - 0.92)), Stroke::new((0.025 * s).max(1.5), with_alpha(col::GOLD, 0.75 * (1.0 - u) * fade)));
            }
            for (n, age) in spray(k, 1.85, SHOW_S, 0.45, 0.8) {
                let t = age * 0.8;
                let from = at + vec2((rnd(n, 1) - 0.55) * 0.26 * s, -0.45 * s);
                let to = from + vec2((rnd(n, 2) - 0.5) * 0.28 * s * t, -0.28 * s * t);
                dot(to, s * (0.055 + 0.06 * t), SMOKE, 0.55 * shows(age));
            }
        }
        "rage" => {
            // Fuming: shaking, swelling a little, steam pouring off the top of its head.
            let grow = 1.0 + (0.05 * k).min(0.06);
            let drift = if k < 0.9 { 0.0224 } else { 0.0224 - 0.0493 * ((k - 0.9) / 1.5).min(1.0) };
            let at = c + vec2(0.0245 * s * (TAU * 6.2 * k).sin(), drift * s);
            for (n, age) in spray(k, 0.1, SHOW_S, 0.1, 0.55) {
                let t = age * 0.55;
                let from = at + vec2((rnd(n, 1) - 0.5) * 0.56 * s, -0.4 * s);
                let to = from + vec2((rnd(n, 2) - 0.5) * 0.28 * s * t, -0.63 * s * t);
                dot(to, s * (0.035 + 0.03 * rnd(n, 3)), STEAM, 0.62 * shows(age));
            }
            face(loop_at(7.0), at, s * grow, 0.0);
        }
        "shades" => {
            // Deal with it: a smirk, then the shades come down and stay on.
            let landed = 1.133;
            if k < landed {
                face(first + (k * 3.0) as usize % 2, c, s, 0.0);
                if k >= 0.455 {
                    let u = ((k - 0.455) / (landed - 0.455)).clamp(0.0, 1.0);
                    let a = ((k - 0.455) / 0.21).min(1.0);
                    picture(p, ctx, SHADES, c - vec2(0.0, 0.902 * s * (1.0 - u * u)), s, base, 0.0, Color32::WHITE.gamma_multiply(fade * a));
                }
            } else {
                let bump = if k < landed + 0.12 { 0.008 * s * (PI * (k - landed) / 0.12).sin() } else { 0.0 };
                face(first + 2, c + vec2(0.0, bump), s, 0.0);
            }
        }
        "rain" => {
            // Making it rain: bills and coins coming down round its head.
            face(loop_at(5.0), c, s, 0.0);
            for (n, age) in spray(k, 0.05, SHOW_S, 0.085, 0.6) {
                let t = age * 0.6;
                let x = (rnd(n, 1) - 0.5) * 1.0;
                let from = c + vec2(x, -0.55 - 0.12 * rnd(n, 2)) * s;
                let at = from + vec2(x * 0.35 * t, 0.5 * t) * s;
                let a = shows(age);
                if n % 3 == 0 {
                    dot(at, s * 0.026, COIN, a);
                } else {
                    let turn = Rot2::from_angle((rnd(n, 3) - 0.5) * 3.0 + t * (rnd(n, 4) - 0.5) * 8.0);
                    let (w, h) = (0.062 * s, 0.03 * s);
                    let pts = [vec2(-w, -h), vec2(w, -h), vec2(w, h), vec2(-w, h)].map(|d| at + turn * (d * 0.5)).to_vec();
                    p.add(Shape::convex_polygon(pts, with_alpha(BILL, a * fade), Stroke::NONE));
                }
            }
        }
        "fire" => {
            // On fire: an orange blaze round it, flickering bigger and smaller, sparks going up.
            let grow = 1.0 + 0.03 * (TAU * 3.0 * k).sin();
            radial(p, c, s * 0.6, Color32::from_rgb(0xff, 0x8c, 0x26).gamma_multiply(0.33 * fade), Color32::TRANSPARENT);
            face(loop_at(4.0), c, s * grow, 0.0);
            for (n, age) in spray(k, 0.05, SHOW_S, 0.035, 0.4) {
                let t = age * 0.4;
                let from = c + vec2((rnd(n, 1) - 0.5) * 0.9, -0.3 - 0.25 * rnd(n, 2)) * s;
                let to = from + vec2((rnd(n, 3) - 0.5) * 0.2 * t, -(0.7 + 0.45 * rnd(n, 4)) * t) * s;
                let color = if rnd(n, 5) < 0.5 { Color32::from_rgb(0xff, 0x8c, 0x2a) } else { Color32::from_rgb(0xff, 0xd2, 0x5c) };
                dot(to, s * (0.012 + 0.008 * rnd(n, 6)), color, shows(age));
            }
        }
        "dance" => {
            // The victory dance: swaying and hopping to the beat, notes going up over its head.
            let a = TAU * 1.1 * k;
            let at = c + vec2(0.08 * s * a.sin(), -0.07 * s * (2.0 * a).sin().abs());
            face(first + (k * 2.2) as usize % count, at, s, 0.12 * a.sin());
            for (n, age) in spray(k, 0.13, SHOW_S, 0.28, 0.7) {
                let t = age * 0.7;
                let from = c + vec2(-0.17 + 0.3 * rnd(n, 1), -0.41) * s;
                let to = from + vec2((rnd(n, 2) - 0.5) * 0.15 * t + 0.015 * (t * 7.0 + rnd(n, 3) * 6.0).sin(), -0.5 * t) * s;
                p.text(to, Align2::CENTER_CENTER, "♪", theme::bold_font(s * 0.16), with_alpha(col::GOLD2, shows(age) * fade));
            }
        }
        "clown" => {
            // A clown: honking its nose (rocking a little as it does).
            let i = loop_at(3.0);
            let turn = if i == first + 1 { 0.054 * (TAU * 2.0 * k + 0.19).sin() } else { 0.0 };
            face(i, c, s, turn);
        }
        "fish" => {
            // A fish out of water: flopping from side to side, splashing.
            let a = TAU * 0.9 * k;
            let at = c + vec2(0.0, -0.1207 * s * (2.0 * a).sin().abs());
            face(loop_at(5.0), at, s, 0.35 * a.sin());
            for (n, age) in spray(k, 0.15, SHOW_S, 0.07, 0.3) {
                let side = if rnd(n, 0) < 0.4 { -1.0 } else { 1.0 };
                let from = at + vec2(-0.2 + 0.6 * rnd(n, 1), 0.35) * s;
                let v = vec2(side * (0.3 + 0.3 * rnd(n, 2)), -(0.2 + 0.3 * rnd(n, 3)));
                dot(thrown(from, v, 2.0, age * 0.3, s), s * 0.016, TEAR, shows(age));
            }
        }
        "popcorn" => {
            // Watching the show: eating popcorn, a piece hopping out of the box now and then.
            face(loop_at(3.0), c, s, 0.0);
            for (n, age) in spray(k, 0.3, SHOW_S, 0.4, 0.35) {
                let from = c + vec2(-0.28 + 0.12 * (rnd(n, 1) - 0.5), -0.03) * s;
                let v = vec2((rnd(n, 2) - 0.5) * 0.3, -(0.3 + 0.2 * rnd(n, 3)));
                let at = thrown(from, v, 1.5, age * 0.35, s);
                dot(at, s * 0.021, Color32::from_rgb(0xff, 0xf4, 0xd8), shows(age));
                dot(at + vec2(0.006, 0.006) * s, s * 0.011, Color32::from_rgb(0xf6, 0xc8, 0x6a), shows(age));
            }
        }
        "salty" => {
            // Salty: shaking salt out (its spout low on the left), then arms crossed.
            let i = loop_at(1.6);
            let pouring = i - first < 2;
            let turn = if pouring { 0.125 + 0.125 * (TAU * 8.0 * k).sin() } else { 0.0 };
            face(i, c, s, turn);
            if pouring {
                for (n, age) in spray(k, 0.0, SHOW_S, 0.045, 0.4) {
                    let from = c + vec2(-0.33 + 0.08 * (rnd(n, 1) - 0.5), 0.28) * s;
                    let v = vec2(-(0.05 + 0.25 * rnd(n, 2)), 0.1 + 0.25 * rnd(n, 3));
                    dot(thrown(from, v, 1.0, age * 0.4, s), s * (0.009 + 0.004 * rnd(n, 4)), Color32::WHITE, shows(age));
                }
            }
        }
        "sleep" => {
            // Wake me up: breathing slowly, Zs drifting up off the top of its head, two and then
            // three of them, as 0.13.28's (the same ones every time: born when, where, how fast).
            let grow = 1.0 + 0.0245 * (TAU * 0.6 * k).sin();
            face(loop_at(1.5), c, s * grow, 0.0);
            for (born, from, v) in ZS {
                let age = (k - born) / 1.15;
                if !(0.0..1.0).contains(&age) {
                    continue;
                }
                let at = c + (from + v * (k - born)) * s;
                let a = (age / 0.07).min(1.0) * ((1.0 - age) / 0.25).min(1.0);
                let font = theme::bold_font(s * 0.15 * (0.55 + 0.45 * (age / 0.18).min(1.0)));
                p.text(at, Align2::CENTER_CENTER, "Z", font, with_alpha(Color32::from_rgb(0xb0, 0xc4, 0xde), a * fade));
            }
        }
        "chicken" => {
            // Chicken: running from side to side, a hop at every step, bits of feather flying.
            let run = TAU * 0.8 * k;
            let at = c + vec2(0.1786 * s * run.sin(), -0.042 * s * (TAU * 3.0 * k).sin().abs());
            face(loop_at(6.0), at, s, 0.0);
            let back = if run.cos() > 0.0 { -1.0 } else { 1.0 };
            for (n, age) in spray(k, 0.2, SHOW_S, 0.19, 0.3) {
                let from = at + vec2((rnd(n, 1) - 0.5) * 0.3, 0.1 + 0.2 * rnd(n, 2)) * s;
                let v = vec2(back * (0.8 + 0.4 * rnd(n, 3)), -0.2 * rnd(n, 4));
                let to = thrown(from, v, 0.5, age * 0.3, s);
                let color = if rnd(n, 5) < 0.6 { Color32::from_rgb(0xd6, 0x99, 0x4d) } else { Color32::from_rgb(0xb8, 0x3a, 0x26) };
                let turn = Rot2::from_angle(rnd(n, 6) * 6.0 + age * 5.0);
                let (w, h) = (0.04 * s, 0.016 * s);
                let pts: Vec<Pos2> = (0..8)
                    .map(|q| {
                        let ang = q as f32 / 8.0 * TAU;
                        to + turn * vec2(ang.cos() * w * 0.5, ang.sin() * h * 0.5)
                    })
                    .collect();
                p.add(Shape::convex_polygon(pts, with_alpha(color, shows(age) * fade), Stroke::NONE));
            }
        }
        _ => face(first, c, s, 0.0),
    }
}

/// A moving one standing still (its picture in the picker), or acting (hovered: its act from
/// where the pop ends, round and round).
fn still_or_acting(p: &egui::Painter, ctx: &egui::Context, j: usize, r: Rect, acting: bool, tint: Color32) {
    if acting {
        let k = 0.38 + (ctx.input(|i| i.time) as f32) % (SHOW_S - 0.6);
        act(p, ctx, j, k, r.center(), r.width(), r.width(), 1.0);
        ctx.request_repaint();
    } else {
        picture(p, ctx, MOVES[j].0 + MOVES[j].2, r.center(), r.width(), r.width(), 0.0, tint);
    }
}

/// Any emote in `r` in the picker or on the bar.
fn draw_any(p: &egui::Painter, ctx: &egui::Context, e: Emote, r: Rect, hot: bool, tint: Color32) {
    match e {
        Emote::Sticker(i) => draw(p, ctx, i, r, tint),
        Emote::Moving(j) => still_or_acting(p, ctx, j, r, hot, tint),
    }
}

/// The emote bar: the quick ones, and More (all of them). What was picked, if anything (at
/// most one every so often, so nobody floods the table).
pub fn bar(ui: &mut egui::Ui, at: Pos2, size: f32, id: &str, last: &mut Option<Instant>) -> Option<String> {
    let ctx = ui.ctx().clone();
    let p = ui.painter().clone();
    let ready = ready(last);
    let mut picked = None;
    let mut x = at.x;
    for name in QUICK {
        let Some(e) = which(&format!("e:{name}")) else { continue };
        let r = Rect::from_min_size(pos2(x, at.y), vec2(size, size));
        x += size + 6.0;
        let resp = ui.interact(r, ui.id().with((id, name)), Sense::click());
        let hot = resp.hovered() && ready;
        p.rect_filled(r, theme::r(12), if hot { Color32::from_white_alpha(34) } else { Color32::from_white_alpha(12) });
        let grow = if hot { 1.08 } else { 0.92 };
        draw_any(&p, &ctx, e, Rect::from_center_size(r.center(), r.size() * grow), hot, if ready { Color32::WHITE } else { Color32::from_white_alpha(110) });
        if resp.on_hover_text(words(e)).clicked() && ready {
            picked = Some(wire(e));
        }
    }
    // More: all of them.
    let r = Rect::from_min_size(pos2(x, at.y), vec2(size, size));
    let resp = ui.interact(r, ui.id().with((id, "more")), Sense::click());
    p.rect_filled(r, theme::r(12), if resp.hovered() { Color32::from_white_alpha(34) } else { Color32::from_white_alpha(12) });
    p.rect_stroke(r, theme::r(12), Stroke::new(1.0f32, col::GOLD_DARK), StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, "+", theme::bold_font(size * 0.5), col::GOLD2);
    let resp = resp.on_hover_text("All the emotes");
    if let Some(e) = grid(&resp, id, ready, egui::RectAlign::TOP_START) {
        picked = Some(wire(e));
    }
    if picked.is_some() {
        *last = Some(Instant::now());
    }
    picked
}

/// One button for them all (where a table's controls leave no room for the bar): a sticker that
/// opens them all.
pub fn button(ui: &mut egui::Ui, r: Rect, id: &str, last: &mut Option<Instant>) -> Option<String> {
    let ctx = ui.ctx().clone();
    let p = ui.painter().clone();
    let ready = ready(last);
    let resp = ui.interact(r, ui.id().with((id, "emotes")), Sense::click());
    let hot = resp.hovered();
    p.rect_filled(r, theme::r(14), if hot { Color32::from_white_alpha(36) } else { Color32::from_white_alpha(16) });
    p.rect_stroke(r, theme::r(14), Stroke::new(1.0f32, if hot { col::GOLD } else { col::GOLD_DARK }), StrokeKind::Inside);
    let s = r.height().min(r.width()) * if hot { 0.98 } else { 0.86 };
    draw(&p, &ctx, 0, Rect::from_center_size(r.center(), vec2(s, s)), if ready { Color32::WHITE } else { Color32::from_white_alpha(110) });
    let resp = resp.on_hover_text("Emotes: send one to the table");
    let picked = grid(&resp, id, ready, egui::RectAlign::TOP_END).map(wire);
    if picked.is_some() {
        *last = Some(Instant::now());
    }
    picked
}

/// Whether one can be sent yet.
fn ready(last: &Option<Instant>) -> bool {
    last.is_none_or(|t| t.elapsed() > Duration::from_millis(1_200))
}

/// All of them, big, in a menu opened from `resp` (the stickers, then the ones that move); the
/// one clicked.
fn grid(resp: &egui::Response, id: &str, ready: bool, align: egui::RectAlign) -> Option<Emote> {
    let ctx = resp.ctx.clone();
    let mut picked = None;
    egui::Popup::menu(resp).align(align).show(|ui| {
        crate::menu::style(ui);
        let mut section = |ui: &mut egui::Ui, title: &str, list: Vec<Emote>, grid_id: &str| {
            ui.label(egui::RichText::new(title).strong().color(col::GOLD2));
            egui::Grid::new((id, grid_id)).spacing(vec2(6.0, 6.0)).show(ui, |ui| {
                for (n, e) in list.into_iter().enumerate() {
                    let (cell, resp) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::click());
                    let hot = resp.hovered();
                    ui.painter().rect_filled(cell, theme::r(10), if hot { Color32::from_white_alpha(30) } else { Color32::from_white_alpha(8) });
                    draw_any(ui.painter(), &ctx, e, cell.shrink(if hot { 1.0 } else { 5.0 }), hot, if ready { Color32::WHITE } else { Color32::from_white_alpha(120) });
                    if resp.on_hover_text(words(e)).clicked() && ready {
                        picked = Some(e);
                        ui.close();
                    }
                    if n % 6 == 5 {
                        ui.end_row();
                    }
                }
            });
        };
        section(ui, "Emotes", (0..EMOTES.len()).map(Emote::Sticker).collect(), "emote-grid");
        section(ui, "Moving", (0..MOVING.len()).map(Emote::Moving).collect(), "moving-grid");
    });
    picked
}

/// The moving ones' pictures read before they're wanted (at a table, for its emotes `size`
/// across), so the first one sent shows whole at once. (Better than 0.13.28, whose first one
/// at a table showed its tears and glow without its face for a second.)
pub fn get_ready(ctx: &egui::Context, size: f32) {
    let _ = sheet(ctx, size);
}

/// The emotes lately at a blackjack, craps or baccarat table, each over the seat it came from
/// (`seat_at`).
pub fn pops(p: &egui::Painter, ctx: &egui::Context, list: &[(TableGame, u64, u8, String, Instant)], game: TableGame, table: u64, size: f32, seat_at: impl Fn(u8) -> Pos2) {
    get_ready(ctx, size);
    for (g, t, seat, e, at) in list {
        if *g == game && *t == table {
            pop(p, ctx, e, seat_at(*seat), at.elapsed().as_secs_f32(), size);
        }
    }
    if list.iter().any(|(g, t, .., at)| *g == game && *t == table && at.elapsed().as_secs_f32() < SHOW_S) {
        ctx.request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_is_its_own() {
        let mut names: Vec<&str> = EMOTES.iter().map(|e| e.0).chain(MOVING.iter().map(|e| e.0)).collect();
        let n = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), n);
        for e in (0..EMOTES.len()).map(Emote::Sticker).chain((0..MOVING.len()).map(Emote::Moving)) {
            assert_eq!(which(&wire(e)), Some(e));
            // (the server takes at most sixteen characters)
            assert!(wire(e).chars().count() <= 16);
        }
        for q in QUICK {
            assert!(which(&format!("e:{q}")).is_some(), "{q}");
        }
    }

    #[test]
    fn the_pictures_fit_the_sheet() {
        let mut next = 0;
        for (first, count, still) in MOVES {
            assert_eq!(first, next, "in order, none missed");
            assert!(still < count);
            next = first + count + usize::from(first == 18);
        }
        assert_eq!(next, 60);
        assert_eq!(SHADES, 21);
    }

    #[test]
    fn little_things_come_and_go() {
        // nothing before it starts; one born every `every`, each gone after `life`
        assert_eq!(spray(0.05, 0.1, 9.0, 0.1, 0.5).count(), 0);
        let alive: Vec<_> = spray(1.0, 0.0, 9.0, 0.1, 0.5).collect();
        assert!((4..=6).contains(&alive.len()), "{alive:?}");
        assert!(alive.iter().all(|(_, a)| (0.0..1.0).contains(a)));
        // none born after `until`
        assert!(spray(3.0, 0.0, 1.0, 0.1, 0.5).next().is_none());
    }
}
