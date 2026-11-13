//! Cards, chips and card backs. Corners, pips and suits are drawn (sharp at any size, and the
//! suits don't depend on a font having them); the court figures, the ace of spades and the
//! backs are Meshy's paintings (0.13.22).

use std::sync::Mutex;

use eframe::egui::{self, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_poker::cards::Card;

/// My deck: the back I picked and the four-color deck (set as the casino's drawn, from my
/// record and settings). Cards that are nobody else's come in it: blackjack's and baccarat's,
/// the shoes, and mine at a poker table.
static MINE: Mutex<(String, bool)> = Mutex::new((String::new(), false));

pub fn set_mine(back: &str, four: bool) {
    if let Ok(mut m) = MINE.lock() {
        if m.0 != back {
            m.0 = back.to_string();
        }
        m.1 = four;
    }
}

/// The back I picked (empty: Ember, everyone's first).
pub fn my_back() -> String {
    MINE.lock().map(|m| m.0.clone()).unwrap_or_default()
}

/// Whether I play with the four-color deck.
pub fn four() -> bool {
    MINE.lock().map(|m| m.1).unwrap_or(false)
}

/// Suit colors: the classic two (black and red), or four (clubs green, diamonds blue), which
/// are easier to tell apart at a glance.
pub fn suit_color(suit: u8, four: bool) -> Color32 {
    match (suit, four) {
        (0, true) => Color32::from_rgb(0x1f, 0x8a, 0x4c),
        (1, true) => Color32::from_rgb(0x1f, 0x6f, 0xd1),
        (1, false) | (2, _) => Color32::from_rgb(0xd3, 0x26, 0x34),
        _ => Color32::from_rgb(0x18, 0x1a, 0x1f),
    }
}

/// A suit's shape, filling `r` (a square).
pub fn suit(p: &Painter, r: Rect, suit: u8, color: Color32) {
    suit_at(p, r.center(), r.width().min(r.height()), suit, color, false);
}

/// A suit `s` across, centered on `c`, upside down if `flip` (the bottom half of a card). Each is
/// 0.13.28's own shape, traced off it at six times the size (a bot's face in Which bot?): one
/// smooth outline (the club's stem a second, tucked into its leaves), filled whole so nothing
/// overlaps.
fn suit_at(p: &Painter, c: Pos2, s: f32, suit: u8, color: Color32, flip: bool) {
    let f = if flip { -1.0 } else { 1.0 };
    let shapes = outlines();
    let parts = &shapes[match suit {
        2 => 0,
        1 => 1,
        3 => 2,
        _ => 3,
    }];
    let feather = 1.0 / p.ctx().pixels_per_point();
    for part in parts {
        let mut mesh = egui::epaint::Mesh::default();
        let n = part.points.len() as u32;
        for (q, nrm) in part.points.iter().zip(&part.normals) {
            let at = c + vec2(q[0] * s, q[1] * s * f);
            let out = vec2(nrm[0], nrm[1] * f) * (feather * 0.5);
            mesh.vertices.push(egui::epaint::Vertex { pos: at - out, uv: egui::epaint::WHITE_UV, color });
            mesh.vertices.push(egui::epaint::Vertex { pos: at + out, uv: egui::epaint::WHITE_UV, color: Color32::TRANSPARENT });
        }
        // The inside (each point's inner vertex), then the soft edge round it.
        mesh.indices.extend(part.triangles.iter().map(|i| i * 2));
        for i in 0..n {
            let j = (i + 1) % n;
            mesh.indices.extend([i * 2, i * 2 + 1, j * 2 + 1, i * 2, j * 2 + 1, j * 2]);
        }
        p.add(Shape::mesh(mesh));
    }
}

const HEART: &[&[[f32; 2]]] = &[
    &[[0.0000, -0.3129], [0.0184, -0.3567], [0.0485, -0.3989], [0.0749, -0.4227], [0.1101, -0.4415], [0.1366, -0.4494], [0.1982, -0.4529], [0.2423, -0.4478], [0.2687, -0.4393], [0.3391, -0.4047], [0.3950, -0.3567], [0.4399, -0.2951], [0.4571, -0.2598], [0.4736, -0.2070], [0.4830, -0.1277], [0.4745, -0.0661], [0.4655, -0.0308], [0.4448, 0.0212], [0.4031, 0.0925], [0.3479, 0.1594], [0.2920, 0.2158], [0.0661, 0.4064], [0.0000, 0.4656], [-0.0661, 0.4064], [-0.2920, 0.2158], [-0.3479, 0.1594], [-0.4031, 0.0925], [-0.4448, 0.0212], [-0.4655, -0.0308], [-0.4745, -0.0661], [-0.4830, -0.1277], [-0.4736, -0.2070], [-0.4571, -0.2598], [-0.4399, -0.2951], [-0.3950, -0.3567], [-0.3391, -0.4047], [-0.2687, -0.4393], [-0.2423, -0.4478], [-0.1982, -0.4529], [-0.1366, -0.4494], [-0.1101, -0.4415], [-0.0749, -0.4227], [-0.0485, -0.3989], [-0.0184, -0.3567]],
];
const DIAMOND: &[&[[f32; 2]]] = &[
    &[[0.0000, -0.4739], [0.0818, -0.3465], [0.1623, -0.2336], [0.2588, -0.1141], [0.3601, -0.0044], [0.3601, 0.0044], [0.2735, 0.0921], [0.1573, 0.2325], [0.0714, 0.3553], [0.0000, 0.4696], [-0.0714, 0.3553], [-0.1573, 0.2325], [-0.2735, 0.0921], [-0.3601, 0.0044], [-0.3601, -0.0044], [-0.2588, -0.1141], [-0.1623, -0.2336], [-0.0818, -0.3465]],
];
const SPADE: &[&[[f32; 2]]] = &[
    &[[0.0000, -0.4772], [0.0659, -0.4170], [0.2564, -0.2592], [0.3384, -0.1801], [0.3765, -0.1362], [0.4132, -0.0834], [0.4468, -0.0132], [0.4557, 0.0220], [0.4621, 0.0747], [0.4578, 0.1186], [0.4454, 0.1713], [0.4349, 0.1965], [0.4173, 0.2280], [0.3927, 0.2592], [0.3625, 0.2855], [0.3295, 0.3060], [0.2855, 0.3236], [0.2416, 0.3322], [0.1801, 0.3322], [0.1450, 0.3249], [0.1098, 0.3123], [0.0659, 0.2855], [0.0395, 0.2590], [0.0220, 0.2329], [0.0162, 0.2416], [0.0185, 0.2768], [0.0277, 0.3119], [0.0435, 0.3470], [0.0734, 0.3910], [0.0970, 0.4173], [0.1577, 0.4613], [0.1538, 0.4704], [0.0000, 0.4702], [-0.1538, 0.4704], [-0.1577, 0.4613], [-0.0970, 0.4173], [-0.0734, 0.3910], [-0.0435, 0.3470], [-0.0277, 0.3119], [-0.0185, 0.2768], [-0.0162, 0.2416], [-0.0220, 0.2329], [-0.0395, 0.2590], [-0.0659, 0.2855], [-0.1098, 0.3123], [-0.1450, 0.3249], [-0.1801, 0.3322], [-0.2416, 0.3322], [-0.2855, 0.3236], [-0.3295, 0.3060], [-0.3625, 0.2855], [-0.3927, 0.2592], [-0.4173, 0.2280], [-0.4349, 0.1965], [-0.4454, 0.1713], [-0.4578, 0.1186], [-0.4621, 0.0747], [-0.4557, 0.0220], [-0.4468, -0.0132], [-0.4132, -0.0834], [-0.3765, -0.1362], [-0.3384, -0.1801], [-0.2564, -0.2592], [-0.0659, -0.4170]],
];
const CLUB: &[&[[f32; 2]]] = &[
    &[[0.0000, -0.4494], [0.0131, -0.4501], [0.0394, -0.4483], [0.0745, -0.4386], [0.1166, -0.4166], [0.1447, -0.3942], [0.1717, -0.3640], [0.1970, -0.3202], [0.2061, -0.2905], [0.2116, -0.2324], [0.2012, -0.1886], [0.1838, -0.1447], [0.1886, -0.1379], [0.2412, -0.1460], [0.2938, -0.1377], [0.3201, -0.1293], [0.3640, -0.1038], [0.3903, -0.0796], [0.4212, -0.0395], [0.4354, -0.0131], [0.4473, 0.0307], [0.4468, 0.1009], [0.4342, 0.1436], [0.4087, 0.1886], [0.3652, 0.2325], [0.3201, 0.2589], [0.2850, 0.2697], [0.2412, 0.2734], [0.1973, 0.2702], [0.1622, 0.2600], [0.1268, 0.2412], [0.0921, 0.2135], [0.0658, 0.1821], [0.0482, 0.1503], [0.0415, 0.1272], [0.0307, 0.1115], [0.0293, 0.1009], [0.0219, 0.0940], [0.0000, 0.0940], [-0.0219, 0.0940], [-0.0293, 0.1009], [-0.0307, 0.1115], [-0.0415, 0.1272], [-0.0482, 0.1503], [-0.0658, 0.1821], [-0.0921, 0.2135], [-0.1268, 0.2412], [-0.1622, 0.2600], [-0.1973, 0.2702], [-0.2412, 0.2734], [-0.2850, 0.2697], [-0.3201, 0.2589], [-0.3652, 0.2325], [-0.4087, 0.1886], [-0.4342, 0.1436], [-0.4468, 0.1009], [-0.4473, 0.0307], [-0.4354, -0.0131], [-0.4212, -0.0395], [-0.3903, -0.0796], [-0.3640, -0.1038], [-0.3201, -0.1293], [-0.2938, -0.1377], [-0.2412, -0.1460], [-0.1886, -0.1379], [-0.1838, -0.1447], [-0.2012, -0.1886], [-0.2116, -0.2324], [-0.2061, -0.2905], [-0.1970, -0.3202], [-0.1717, -0.3640], [-0.1447, -0.3942], [-0.1166, -0.4166], [-0.0745, -0.4386], [-0.0394, -0.4483], [-0.0131, -0.4501]],
    &[[0.0000, 0.0800], [0.0300, 0.0800], [0.0244, 0.1272], [0.0276, 0.1798], [0.0547, 0.3114], [0.0733, 0.3553], [0.0998, 0.3991], [0.1317, 0.4342], [0.1666, 0.4605], [0.1622, 0.4697], [0.0745, 0.4666], [0.0000, 0.4674], [-0.0745, 0.4666], [-0.1622, 0.4697], [-0.1666, 0.4605], [-0.1317, 0.4342], [-0.0998, 0.3991], [-0.0733, 0.3553], [-0.0547, 0.3114], [-0.0276, 0.1798], [-0.0244, 0.1272], [-0.0300, 0.0800]],
];

/// One part of a suit, ready to draw: its outline, the triangles inside it, and each point's
/// outward normal (for the soft edge: as long as it needs to be at a corner, within reason).
struct Outline {
    points: Vec<[f32; 2]>,
    triangles: Vec<u32>,
    normals: Vec<[f32; 2]>,
}

/// The four suits' parts (hearts, diamonds, spades, clubs), worked out once.
fn outlines() -> &'static [Vec<Outline>; 4] {
    static ALL: std::sync::OnceLock<[Vec<Outline>; 4]> = std::sync::OnceLock::new();
    ALL.get_or_init(|| [HEART, DIAMOND, SPADE, CLUB].map(|parts| parts.iter().map(|pts| outline(pts)).collect()))
}

fn outline(pts: &[[f32; 2]]) -> Outline {
    let n = pts.len();
    let area: f32 = (0..n).map(|i| pts[i][0] * pts[(i + 1) % n][1] - pts[(i + 1) % n][0] * pts[i][1]).sum::<f32>() / 2.0;
    let sign = area.signum();
    let cross = |o: [f32; 2], a: [f32; 2], b: [f32; 2]| (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0]);
    // Ear clipping: a corner that turns the shape's way, with nothing of the shape inside it.
    let mut left: Vec<usize> = (0..n).collect();
    let mut triangles = Vec::with_capacity(3 * n);
    while left.len() > 3 {
        let m = left.len();
        let ear = (0..m).find(|&k| {
            let (i0, i1, i2) = (left[(k + m - 1) % m], left[k], left[(k + 1) % m]);
            let (a, b, c) = (pts[i0], pts[i1], pts[i2]);
            if cross(a, b, c) * sign <= 1e-9 {
                return false;
            }
            !left.iter().any(|&j| {
                j != i0 && j != i1 && j != i2 && {
                    let q = pts[j];
                    cross(a, b, q) * sign >= 0.0 && cross(b, c, q) * sign >= 0.0 && cross(c, a, q) * sign >= 0.0
                }
            })
        });
        let Some(k) = ear else { break };
        triangles.extend([left[(k + m - 1) % m] as u32, left[k] as u32, left[(k + 1) % m] as u32]);
        left.remove(k);
    }
    if left.len() == 3 {
        triangles.extend(left.iter().map(|&i| i as u32));
    }
    let normals = (0..n)
        .map(|i| {
            let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
            let edge = |p: [f32; 2], q: [f32; 2]| {
                let (dx, dy) = (q[0] - p[0], q[1] - p[1]);
                let len = (dx * dx + dy * dy).sqrt().max(1e-6);
                [sign * dy / len, -sign * dx / len]
            };
            let (n0, n1) = (edge(a, b), edge(b, c));
            let mid = [(n0[0] + n1[0]) * 0.5, (n0[1] + n1[1]) * 0.5];
            let len2 = (mid[0] * mid[0] + mid[1] * mid[1]).max(1.0 / 9.0);
            [mid[0] / len2, mid[1] / len2]
        })
        .collect();
    Outline { points: pts.to_vec(), triangles, normals }
}

fn rank_text(rank: u8) -> &'static str {
    ["2", "3", "4", "5", "6", "7", "8", "9", "10", "J", "Q", "K", "A"][rank as usize]
}

/// Where a number card's pips go, as on a real deck: (across, down), fractions of the card.
/// Those below the middle are drawn upside down.
fn pip_spots(rank: u8) -> &'static [(f32, f32)] {
    const L: f32 = 0.34;
    const C: f32 = 0.5;
    const R: f32 = 0.66;
    const T: f32 = 0.2;
    const B: f32 = 0.8;
    const M: f32 = 0.5;
    const U: f32 = 0.4;
    const D: f32 = 0.6;
    match rank + 2 {
        2 => &[(C, T), (C, B)],
        3 => &[(C, T), (C, M), (C, B)],
        4 => &[(L, T), (R, T), (L, B), (R, B)],
        5 => &[(L, T), (R, T), (C, M), (L, B), (R, B)],
        6 => &[(L, T), (R, T), (L, M), (R, M), (L, B), (R, B)],
        7 => &[(L, T), (R, T), (C, 0.35), (L, M), (R, M), (L, B), (R, B)],
        8 => &[(L, T), (R, T), (C, 0.35), (L, M), (R, M), (C, 0.65), (L, B), (R, B)],
        9 => &[(L, T), (R, T), (L, U), (R, U), (C, M), (L, D), (R, D), (L, B), (R, B)],
        _ => &[(L, T), (R, T), (C, 0.3), (L, U), (R, U), (L, D), (R, D), (C, 0.7), (L, B), (R, B)],
    }
}

/// A card face up in `r` (about 5:7). `dim`: not part of the winning hand, or folded.
pub fn face(p: &Painter, r: Rect, card: Card, four: bool, dim: bool) {
    let w = r.width();
    let round = CornerRadius::same((w * 0.09).round() as u8);
    // A soft shadow, the card, a fine edge.
    p.rect_filled(r.translate(vec2(0.0, r.height() * 0.03)), round, Color32::from_black_alpha(70));
    p.rect_filled(r, round, Color32::from_rgb(0xfb, 0xfa, 0xf6));
    p.rect_stroke(r, round, Stroke::new(1.0f32, Color32::from_rgb(0xd9, 0xd5, 0xcc)), StrokeKind::Inside);
    // (Edge on, turning over: nothing to read.)
    if w >= 6.0 {
        let col = suit_color(card.suit(), four);
        let (rank, s) = (card.rank(), card.suit());
        // Small cards (a hand's history, the board in a list) keep a big corner to read by.
        let small = r.height() < 60.0;
        corners(p, r, rank, s, col, small);
        if small {
            suit_at(p, r.center() + vec2(w * 0.12, r.height() * 0.16), w * 0.5, s, col, false);
        } else {
            match rank {
                12 => ace(p, r, s, col),
                9..=11 => court(p, r, rank, s, col),
                _ => {
                    let size = if rank >= 7 { w * 0.17 } else { w * 0.2 };
                    for &(x, y) in pip_spots(rank) {
                        suit_at(p, pos2(r.left() + x * w, r.top() + y * r.height()), size, s, col, y > 0.5);
                    }
                }
            }
        }
    }
    if dim {
        p.rect_filled(r, round, Color32::from_black_alpha(120));
    }
}

/// The rank with its suit under it, top left, and the same upside down bottom right (only the
/// top left on a small card, bigger).
fn corners(p: &Painter, r: Rect, rank: u8, suit: u8, col: Color32, small: bool) {
    let w = r.width();
    let size = match (small, rank) {
        (true, 8) => w * 0.31,
        (true, _) => w * 0.38,
        (false, 8) => w * 0.21,
        (false, _) => w * 0.25,
    };
    let g = p.layout_no_wrap(rank_text(rank).to_string(), egui::FontId::new(size, crate::theme::bold()), col);
    let gs = g.size();
    let cx = r.left() + if small { w * 0.2 } else { w * 0.135 };
    let top = r.top() + w * 0.04;
    // (A line of text is taller than its letters: the pip goes under the letters.)
    let pip = if small { w * 0.24 } else { w * 0.15 };
    let pip_y = top + gs.y * 0.86 + pip * 0.55;
    p.galley(pos2(cx - gs.x / 2.0, top), g.clone(), col);
    suit_at(p, pos2(cx, pip_y), pip, suit, col, false);
    if !small {
        let cx = r.right() - (cx - r.left());
        let bottom = r.bottom() - w * 0.04;
        p.add(egui::epaint::TextShape::new(pos2(cx + gs.x / 2.0, bottom), g, col).with_angle(std::f32::consts::PI));
        suit_at(p, pos2(cx, r.bottom() - (pip_y - r.top())), pip, suit, col, true);
    }
}

/// An ace: one big suit, and for the ace of spades, Meshy's spade inlaid with gold in a gold
/// ring.
fn ace(p: &Painter, r: Rect, suit: u8, col: Color32) {
    let w = r.width();
    let c = r.center();
    if suit == 3 {
        let gold = Color32::from_rgb(0xc9, 0xa2, 0x4a);
        p.circle_stroke(c, w * 0.34, Stroke::new((w * 0.022).max(1.0), gold));
        p.circle_stroke(c, w * 0.305, Stroke::new((w * 0.008).max(0.6), gold.gamma_multiply(0.7)));
        let pic = Rect::from_center_size(c + vec2(0.0, -w * 0.01), vec2(w * 0.46, w * 0.46 * 463.0 / 400.0));
        if crate::casino::pics::cover(p, p.ctx(), "card-ace-spades", pic, texture_px(pic), Color32::WHITE) {
            return;
        }
    }
    suit_at(p, c, w * 0.42, suit, col, false);
}

/// A jack, queen or king: its figure in a frame, double-ended like a real deck (the top half,
/// and the same turned round below).
fn court(p: &Painter, r: Rect, rank: u8, suit: u8, col: Color32) {
    let w = r.width();
    let frame = Rect::from_min_max(pos2(r.left() + w * 0.24, r.top() + w * 0.12), pos2(r.right() - w * 0.24, r.bottom() - w * 0.12));
    let name = court_art(rank, suit);
    let half = Rect::from_min_max(frame.min, pos2(frame.right(), frame.center().y));
    let line = Stroke::new((w * 0.012).max(0.8), col.gamma_multiply(0.8));
    match crate::casino::pics::tex(p.ctx(), name, texture_px(half)) {
        Some(t) => {
            // The figure from the top down, cut to the half's shape; turned round below.
            let [iw, ih] = t.size();
            // (Its top part when the half is wider than the picture; its middle when narrower.)
            let k = (half.height() / half.width()) * (iw as f32 / ih.max(1) as f32);
            let uv = if k <= 1.0 { Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, k)) } else { Rect::from_min_max(pos2(0.5 - 0.5 / k, 0.0), pos2(0.5 + 0.5 / k, 1.0)) };
            p.image(t.id(), half, uv, Color32::WHITE);
            let below = Rect::from_min_max(pos2(frame.left(), frame.center().y), frame.max);
            p.image(t.id(), below, Rect::from_min_max(uv.max, uv.min), Color32::WHITE);
        }
        None => {
            p.rect_filled(frame, CornerRadius::ZERO, Color32::from_rgb(0xf0, 0xe3, 0xc3));
            suit_at(p, frame.center(), w * 0.3, suit, col, false);
        }
    }
    p.rect_stroke(frame, CornerRadius::ZERO, line, StrokeKind::Outside);
    p.line_segment([pos2(frame.left(), frame.center().y), pos2(frame.right(), frame.center().y)], line);
}

/// The court figures' pictures: jacks, queens and kings of clubs, diamonds, hearts, spades.
fn court_art(rank: u8, suit: u8) -> &'static str {
    const ART: [[&str; 4]; 3] = [["card-jc", "card-jd", "card-jh", "card-js"], ["card-qc", "card-qd", "card-qh", "card-qs"], ["card-kc", "card-kd", "card-kh", "card-ks"]];
    ART[(rank as usize).clamp(9, 11) - 9][(suit as usize).min(3)]
}

/// How big a picture's texture is wanted for `r` (the longer side, in points).
fn texture_px(r: Rect) -> u32 {
    r.width().max(r.height()).max(16.0).round() as u32
}

/// The picture of a card back, by its look's id ("back_jade"...). Anything else is Ember's,
/// the one everyone has from the start.
pub fn back_art(design: &str) -> &'static str {
    match design {
        "back_midnight" => "card-back-midnight",
        "back_jade" => "card-back-jade",
        "back_gold" => "card-back-gold",
        "back_inferno" => "card-back-inferno",
        "back_aurora" => "card-back-aurora",
        _ => "card-back-ember",
    }
}

/// A card face down, in the back `design` (a look's id; empty: mine). Every face-down card goes
/// through here, the ones being dealt too.
pub fn back(p: &Painter, r: Rect, design: &str) {
    let mine;
    let design = if design.is_empty() {
        mine = my_back();
        mine.as_str()
    } else {
        design
    };
    let rad = r.width().min(r.height() * 0.7) * 0.09;
    let round = CornerRadius::same(rad.round() as u8);
    p.rect_filled(r.translate(vec2(0.0, r.height() * 0.03)), round, Color32::from_black_alpha(70));
    // The painting, the whole of it (squeezed while it turns over).
    if !crate::casino::pics::stretch_rounded(p, p.ctx(), back_art(design), r, rad, texture_px(r), Color32::WHITE) {
        drawn_back(p, r, design);
    }
    p.rect_stroke(r, round, Stroke::new(1.0f32, Color32::from_black_alpha(120)), StrokeKind::Inside);
}

/// A back drawn in its colors, while its painting's still being read.
fn drawn_back(p: &Painter, r: Rect, design: &str) {
    let rad = r.width().min(r.height() * 0.7) * 0.09;
    let (a, b) = match design {
        "back_midnight" => (Color32::from_rgb(0x15, 0x1d, 0x3b), Color32::from_rgb(0x2a, 0x3a, 0x7a)),
        "back_jade" => (Color32::from_rgb(0x0c, 0x3b, 0x2e), Color32::from_rgb(0x17, 0x6b, 0x52)),
        "back_gold" => (Color32::from_rgb(0x16, 0x13, 0x0c), Color32::from_rgb(0x8a, 0x66, 0x18)),
        "back_inferno" => (Color32::from_rgb(0x1a, 0x08, 0x04), Color32::from_rgb(0xd0, 0x4a, 0x10)),
        "back_aurora" => (Color32::from_rgb(0x10, 0x16, 0x2c), Color32::from_rgb(0x2a, 0x8a, 0x8a)),
        _ => (Color32::from_rgb(0x5a, 0x0c, 0x10), Color32::from_rgb(0xa8, 0x7a, 0x30)),
    };
    p.rect_filled(r, CornerRadius::same(rad.round() as u8), a);
    p.rect_stroke(r.shrink(r.width() * 0.07), CornerRadius::same((rad * 0.6).round() as u8), Stroke::new(1.0f32, b), StrokeKind::Inside);
    if r.width() > 10.0 {
        let mid = r.width().min(r.height()) * 0.22;
        p.circle_stroke(r.center(), mid, Stroke::new(1.0f32, b));
    }
}

/// Chip colors by value: white 1, red 5, green 25, black 100, purple 500, yellow 1,000,
/// orange 5,000, blue 25,000 (the set's look tints them).
fn chip_color(value: u64, set: &str) -> (Color32, Color32) {
    let base = match value {
        0..=4 => Color32::from_rgb(0xee, 0xee, 0xea),
        5..=24 => Color32::from_rgb(0xd0, 0x2b, 0x3a),
        25..=99 => Color32::from_rgb(0x1e, 0x9a, 0x55),
        100..=499 => Color32::from_rgb(0x24, 0x24, 0x28),
        500..=999 => Color32::from_rgb(0x7b, 0x3f, 0xc6),
        1000..=4999 => Color32::from_rgb(0xf0, 0xc0, 0x2e),
        5000..=24_999 => Color32::from_rgb(0xf0, 0x7a, 0x1e),
        _ => Color32::from_rgb(0x2a, 0x6c, 0xe0),
    };
    let edge = match set {
        "chips_gold" => Color32::from_rgb(0xff, 0xd7, 0x6a),
        "chips_neon" => Color32::from_rgb(0x5f, 0xf4, 0xff),
        _ => Color32::from_rgb(0xf5, 0xf2, 0xea),
    };
    (base, edge)
}

const DENOMS: [u64; 8] = [25_000, 5_000, 1_000, 500, 100, 25, 5, 1];

/// A pile of chips for `amount` (a few stacks side by side, bottom center at `at`), sized `r`.
pub fn pile(p: &Painter, at: Pos2, amount: u64, r: f32, set: &str) {
    let mut left = amount;
    let mut stacks: Vec<(u64, u32)> = Vec::new();
    for d in DENOMS {
        let n = (left / d) as u32;
        if n > 0 {
            stacks.push((d, n.min(12)));
            left -= d * n as u64;
        }
        if stacks.len() == 4 {
            break;
        }
    }
    let w = stacks.len() as f32 * r * 2.1;
    let mut x = at.x - w / 2.0 + r;
    for (d, n) in stacks {
        for i in 0..n {
            chip(p, pos2(x, at.y - r * 0.45 - i as f32 * r * 0.28), r, d, set);
        }
        x += r * 2.1;
    }
}

/// One chip seen from a little above.
pub fn chip(p: &Painter, c: Pos2, r: f32, value: u64, set: &str) {
    let (base, edge) = chip_color(value, set);
    let squash = 0.55;
    // The side, then the top: an ellipse with stripes on its edge.
    let side = Rect::from_center_size(c + vec2(0.0, r * 0.14), vec2(r * 2.0, r * 2.0 * squash));
    p.add(egui::epaint::EllipseShape::filled(side.center(), side.size() / 2.0, darker(base)));
    let top = Rect::from_center_size(c, vec2(r * 2.0, r * 2.0 * squash));
    p.add(egui::epaint::EllipseShape::filled(top.center(), top.size() / 2.0, base));
    for k in 0..6 {
        let a = k as f32 * std::f32::consts::TAU / 6.0;
        let pt = c + vec2(a.cos() * r * 0.85, a.sin() * r * 0.85 * squash);
        p.add(egui::epaint::EllipseShape::filled(pt, vec2(r * 0.16, r * 0.16 * squash), edge));
    }
    p.add(egui::epaint::EllipseShape::stroke(c, vec2(r * 0.55, r * 0.55 * squash), Stroke::new(1.0f32, edge.gamma_multiply(0.7))));
}

fn darker(c: Color32) -> Color32 {
    Color32::from_rgb((c.r() as f32 * 0.62) as u8, (c.g() as f32 * 0.62) as u8, (c.b() as f32 * 0.62) as u8)
}

/// "12,500"
pub fn chips_text(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// "1 hand", "1,204 hands": a count and its word, singular for one.
pub fn count(n: u64, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{} {many}", chips_text(n)) }
}

/// "12.5K", "1.2M": short, for tight spots.
pub fn chips_short(n: u64) -> String {
    match n {
        0..=9_999 => chips_text(n),
        10_000..=999_999 => {
            let k = n as f64 / 1000.0;
            if k < 100.0 { format!("{:.1}K", k).replace(".0K", "K") } else { format!("{:.0}K", k) }
        }
        _ => format!("{:.2}M", n as f64 / 1_000_000.0).replace(".00M", "M"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every card back in the casino has a painting of its own (Blue picked red, Ember, and was
    /// dealt green); an empty one is mine.
    #[test]
    fn every_back_has_its_own_picture() {
        use pyr_poker::progress::{COSMETICS, Kind};
        let backs: Vec<&str> = COSMETICS.iter().filter(|c| c.kind == Kind::CardBack).map(|c| c.id).collect();
        assert_eq!(backs, ["back_ember", "back_midnight", "back_jade", "back_gold", "back_inferno", "back_aurora"]);
        let arts: std::collections::HashSet<&str> = backs.iter().map(|b| back_art(b)).collect();
        assert_eq!(arts.len(), backs.len(), "two backs share a picture");
        for b in &backs {
            assert!(crate::casino::pics::has(back_art(b)), "{b} has no picture");
        }
        assert_eq!(back_art("back_ember"), "card-back-ember");
        assert_eq!(back_art("back_jade"), "card-back-jade");
        set_mine("back_midnight", true);
        assert_eq!((my_back(), four()), ("back_midnight".to_string(), true));
        // Every court card has its figure.
        for rank in 9..=11u8 {
            for suit in 0..4u8 {
                assert!(crate::casino::pics::has(court_art(rank, suit)));
            }
        }
        assert_eq!(court_art(11, 2), "card-kh");
        assert_eq!(court_art(9, 1), "card-jd");
    }

    /// The pips come as on a real deck: as many as the card says, the bottom half's upside down.
    #[test]
    fn pips_as_on_a_real_deck() {
        for rank in 0..9u8 {
            let spots = pip_spots(rank);
            assert_eq!(spots.len(), rank as usize + 2);
            // Turned round, a card's pips land on themselves (all but the seven's: it has one
            // more pip above the middle than below, as on a real deck).
            for &(x, y) in spots {
                let turned = spots.iter().any(|&(x2, y2)| (x2 - (1.0 - x)).abs() < 1e-6 && (y2 - (1.0 - y)).abs() < 1e-6);
                assert!(turned || rank + 2 == 7, "{}: {x},{y}", rank + 2);
            }
        }
    }

    #[test]
    fn chip_numbers_read_well() {
        assert_eq!(chips_text(0), "0");
        assert_eq!(chips_text(1250), "1,250");
        assert_eq!(chips_text(1_000_000), "1,000,000");
        assert_eq!(chips_short(12_500), "12.5K");
        assert_eq!(chips_short(20_000), "20K");
        assert_eq!(chips_short(250_000), "250K");
        assert_eq!(chips_short(1_500_000), "1.50M");
        assert_eq!(chips_short(2_000_000), "2M");
    }
}
