//! Cards, chips and card backs. Corners, pips and suits are drawn (sharp at any size, and the
//! suits don't depend on a font having them); the court figures, the ace of spades and the
//! backs are Meshy's paintings (0.13.22).

use std::sync::Mutex;

use eframe::egui::{self, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};
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

/// Suit colors: the classic two (black and red, 0.13.28's), or four (clubs green, diamonds
/// blue), which are easier to tell apart at a glance.
pub fn suit_color(suit: u8, four: bool) -> Color32 {
    match (suit, four) {
        (0, true) => Color32::from_rgb(0x1f, 0x8a, 0x4c),
        (1, true) => Color32::from_rgb(0x1f, 0x6f, 0xd1),
        (1, false) | (2, _) => Color32::from_rgb(0xcc, 0x1f, 0x2e),
        _ => Color32::from_rgb(0x17, 0x19, 0x1f),
    }
}

/// A card's face (0.13.28's, measured): white at its top to a warm white at its foot, inside an
/// edge of its own.
const FACE_TOP: Color32 = Color32::from_rgb(0xff, 0xfe, 0xfa);
const FACE_FOOT: Color32 = Color32::from_rgb(0xf2, 0xee, 0xe4);
const EDGE: Color32 = Color32::from_rgb(0xc9, 0xc0, 0xae);

/// A suit's shape, filling `r` (a square).
pub fn suit(p: &Painter, r: Rect, suit: u8, color: Color32) {
    suit_at(p, r.center(), r.width().min(r.height()), suit, color, false, false);
}

/// `color` lighter or darker: each channel times `k`.
fn times(color: Color32, k: f32) -> Color32 {
    let m = |x: u8| (x as f32 * k).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(m(color.r()), m(color.g()), m(color.b()), color.a())
}

/// Between two colors, `t` of the way (0 to 1).
fn between(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

/// `r` with its corners rounded by `rad`, filled with a soft edge a pixel wide, each point the
/// color `at` its height gives.
fn soft_round_rect(p: &Painter, r: Rect, rad: f32, at: impl Fn(f32) -> Color32) {
    let rad = rad.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    let seg = 8;
    let mut pts = Vec::with_capacity(4 * (seg + 1));
    for (c, a0) in [(pos2(r.right() - rad, r.bottom() - rad), 0.0f32), (pos2(r.left() + rad, r.bottom() - rad), 90.0), (pos2(r.left() + rad, r.top() + rad), 180.0), (pos2(r.right() - rad, r.top() + rad), 270.0)] {
        for i in 0..=seg {
            let a = (a0 + 90.0 * i as f32 / seg as f32).to_radians();
            pts.push(c + vec2(a.cos(), a.sin()) * rad);
        }
    }
    let n = pts.len();
    let half = 0.5 / p.ctx().pixels_per_point();
    let mut m = egui::epaint::Mesh::default();
    for i in 0..n {
        let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        // (Clockwise on the screen: the outside is to the left of each edge.)
        let out = |p: Pos2, q: Pos2| {
            let d = (q - p).normalized();
            vec2(d.y, -d.x)
        };
        let mid = (out(a, b) + out(b, c)) * 0.5;
        let nrm = if mid.length_sq() < 1e-6 { out(a, b) } else { mid / mid.length_sq().max(1.0 / 9.0) };
        let inner = b - nrm * half;
        m.colored_vertex(inner, at(inner.y));
        m.colored_vertex(b + nrm * half, Color32::TRANSPARENT);
    }
    for i in 1..n as u32 - 1 {
        m.add_triangle(0, 2 * i, 2 * (i + 1));
    }
    for i in 0..n as u32 {
        let j = (i + 1) % n as u32;
        m.add_triangle(2 * i, 2 * i + 1, 2 * j + 1);
        m.add_triangle(2 * i, 2 * j + 1, 2 * j);
    }
    p.add(Shape::mesh(m));
}

/// A suit `s` across, centered on `c`, upside down if `flip` (the bottom half of a card). Each is
/// 0.13.28's own shape, traced off it at six times the size (a bot's face in Which bot?): one
/// smooth outline (the club's stem a second, tucked into its leaves), filled whole so nothing
/// overlaps. `shade`: lit from above as a card's pips are (0.13.28's: 1.15 times the color at
/// the top of its square to 0.875 at the foot, whichever way up it is).
fn suit_at(p: &Painter, c: Pos2, s: f32, suit: u8, color: Color32, flip: bool, shade: bool) {
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
            let color = if shade { times(color, 1.15 - 0.275 * ((at.y - out.y - (c.y - s / 2.0)) / s).clamp(0.0, 1.0)) } else { color };
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

/// Where a number card's pips go, as 0.13.28 lays them out (measured off its board): (across,
/// down), fractions of the card. Two columns and the middle; the rows a fifth of the way in from
/// top and foot, the middle, a 7's and an 8's halfway between, a 9's and a 10's four rows evenly
/// apart. Those below the middle are drawn upside down.
fn pip_spots(rank: u8) -> &'static [(f32, f32)] {
    const L: f32 = 0.345;
    const C: f32 = 0.5;
    const R: f32 = 0.655;
    const T: f32 = 0.2155;
    const B: f32 = 1.0 - T;
    const M: f32 = 0.5;
    const U: f32 = T + (B - T) / 3.0;
    const D: f32 = 1.0 - U;
    const UP: f32 = (T + M) / 2.0;
    const DOWN: f32 = 1.0 - UP;
    const TEN_UP: f32 = (T + U) / 2.0;
    const TEN_DOWN: f32 = 1.0 - TEN_UP;
    match rank + 2 {
        2 => &[(C, T), (C, B)],
        3 => &[(C, T), (C, M), (C, B)],
        4 => &[(L, T), (R, T), (L, B), (R, B)],
        5 => &[(L, T), (R, T), (C, M), (L, B), (R, B)],
        6 => &[(L, T), (R, T), (L, M), (R, M), (L, B), (R, B)],
        7 => &[(L, T), (R, T), (C, UP), (L, M), (R, M), (L, B), (R, B)],
        8 => &[(L, T), (R, T), (C, UP), (L, M), (R, M), (C, DOWN), (L, B), (R, B)],
        9 => &[(L, T), (R, T), (L, U), (R, U), (C, M), (L, D), (R, D), (L, B), (R, B)],
        _ => &[(L, T), (R, T), (C, TEN_UP), (L, U), (R, U), (L, D), (R, D), (C, TEN_DOWN), (L, B), (R, B)],
    }
}

/// A card face up in `r` (about 5:7). `dim`: not part of the winning hand, or folded.
pub fn face(p: &Painter, r: Rect, card: Card, four: bool, dim: bool) {
    let (w, h) = (r.width(), r.height());
    let round = CornerRadius::same((w * 0.09).round() as u8);
    // A soft shadow; the card itself.
    p.rect_filled(r.translate(vec2(0.0, h * 0.03)), round, Color32::from_black_alpha(70));
    body(p, r, round);
    // (Edge on, turning over: nothing to read.)
    if w >= 6.0 {
        let col = suit_color(card.suit(), four);
        let (rank, s) = (card.rank(), card.suit());
        // (Whole at any size, as 0.13.28's: a seat's shown cards, a hand in someone's history.)
        corners(p, r, rank, s, col);
        match rank {
            12 => ace(p, r, s, col),
            9..=11 => court(p, r, rank, s, col),
            _ => {
                for &(x, y) in pip_spots(rank) {
                    pip(p, pos2(r.left() + x * w, r.top() + y * h), w * 0.161, s, col, y > 0.5);
                }
            }
        }
    }
    if dim {
        p.rect_filled(r, round, Color32::from_black_alpha(120));
    }
}

/// A card's body (0.13.28's, measured): its edge, and its face inside it (the edge a 79th of the
/// card across) shading from white at the top to a warm white at the foot.
fn body(p: &Painter, r: Rect, round: CornerRadius) {
    let edge = r.width() * 0.0127;
    p.rect_filled(r, round, EDGE);
    soft_round_rect(p, r.shrink(edge), (round.nw as f32 - edge).max(0.0), |y| between(FACE_TOP, FACE_FOOT, (y - r.top()) / r.height()));
}

/// The rank with its suit under it, top left, and the same turned round bottom right. 0.13.28's:
/// the rank in Barlow Condensed Black, both an eighth of the way in.
fn corners(p: &Painter, r: Rect, rank: u8, suit: u8, col: Color32) {
    let (w, h) = (r.width(), r.height());
    let g = p.layout_no_wrap(rank_text(rank).to_string(), crate::theme::display_font("barlow-black", h * RANK_SIZE), col);
    let gs = g.size();
    let (cx, top) = (r.left() + w * RANK_ACROSS, r.top() + h * RANK_TOP);
    p.galley(pos2(cx - gs.x / 2.0, top), g.clone(), col);
    let cx = r.right() - w * RANK_ACROSS;
    p.add(egui::epaint::TextShape::new(pos2(cx + gs.x / 2.0, r.bottom() - h * RANK_TOP), g, col).with_angle(std::f32::consts::PI));
    pip(p, pos2(r.left() + w * 0.125, r.top() + h * CORNER_PIP), w * 0.1454, suit, col, false);
    pip(p, pos2(r.right() - w * 0.125, r.bottom() - h * CORNER_PIP), w * 0.1454, suit, col, true);
}

/// A pip on a card: its suit `s` across, lit from above. (0.13.28's diamonds come out a little
/// bigger there than on a bot's face: measured on its board, 2% across.)
fn pip(p: &Painter, c: Pos2, s: f32, suit: u8, col: Color32, flip: bool) {
    let s = if suit == 1 { s * 1.019 } else { s };
    suit_at(p, c, s, suit, col, flip, true);
}

/// A card's corner (0.13.28's, measured): the rank's lettering (its size, a fifth of the card's
/// height), the middle of its line, where its line's top is (above the card's own: the letters'
/// tops come 0.054 of the way down), and the suit under it (an eighth of the way in).
const RANK_SIZE: f32 = 0.2;
const RANK_ACROSS: f32 = 0.1265;
const RANK_TOP: f32 = -0.0055;
const CORNER_PIP: f32 = 0.2826;

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
    // (0.13.28's: three eighths of the card across, a hair below its middle.)
    suit_at(p, c + vec2(0.0, r.height() * 0.001), w * 0.375, suit, col, false, true);
}

/// A jack, queen or king, as 0.13.28's (measured off its board): the figure in a gold frame,
/// double-ended like a real deck (its top above a gold line slanting up through the middle, the
/// same turned round below it, a small gold diamond where they meet), the suit small in the
/// frame's corners.
fn court(p: &Painter, r: Rect, rank: u8, suit: u8, col: Color32) {
    let (w, h) = (r.width(), r.height());
    let c = r.center();
    let frame = Rect::from_min_max(pos2(r.left() + w * 0.2074, r.top() + h * 0.0246), pos2(r.right() - w * 0.2074, r.bottom() - h * 0.0246));
    let band = w * 0.0156;
    let inside = frame.shrink(band);
    // (The gold: light at the top, deep in the middle, dark at the foot.)
    let gold = |y: f32| {
        let t = ((y - frame.top()) / frame.height()).clamp(0.0, 1.0);
        if t < 0.5 { between(GOLD_TOP, GOLD_MID, t * 2.0) } else { between(GOLD_MID, GOLD_FOOT, t * 2.0 - 1.0) }
    };
    soft_round_rect(p, inside, 1.0, |_| CREAM);
    // The slant: up to the right through the middle.
    let rise = inside.width() * 0.0875;
    let (a, b) = (pos2(inside.left(), c.y + rise), pos2(inside.right(), c.y - rise));
    match crate::casino::pics::tex(p.ctx(), court_art(rank, suit), texture_px(inside)) {
        Some(t) => {
            // The picture a square 0.707 of the card across, its top just under the frame's;
            // above the slant, and the same turned round the card's middle below it.
            let pic = Rect::from_min_size(pos2(c.x - w * 0.3535, r.top() + h * 0.0323), vec2(w * 0.707, w * 0.707));
            let uv = |q: Pos2| pos2((q.x - pic.left()) / pic.width(), (q.y - pic.top()) / pic.height());
            let top = [inside.left_top(), inside.right_top(), b, a];
            let mut m = egui::epaint::Mesh::with_texture(t.id());
            for q in top {
                m.vertices.push(egui::epaint::Vertex { pos: q, uv: uv(q), color: Color32::WHITE });
            }
            for q in top {
                m.vertices.push(egui::epaint::Vertex { pos: c + (c - q), uv: uv(q), color: Color32::WHITE });
            }
            m.indices.extend([0, 1, 2, 0, 2, 3, 4, 5, 6, 4, 6, 7]);
            p.add(Shape::mesh(m));
        }
        None => {
            suit_at(p, c, w * 0.3, suit, col, false, true);
        }
    }
    soft_round_ring(p, frame, 3.0, band, gold);
    // A fine line inside the gold, the slant, and the diamond on it.
    let fine = frame.shrink(w * 0.0296);
    let corner = |q: Pos2, d: Vec2| q + d;
    let k = 0.7f32;
    let line: Vec<Pos2> = [
        corner(fine.left_top(), vec2(k, 0.0)),
        corner(fine.right_top(), vec2(-k, 0.0)),
        corner(fine.right_top(), vec2(0.0, k)),
        corner(fine.right_bottom(), vec2(0.0, -k)),
        corner(fine.right_bottom(), vec2(-k, 0.0)),
        corner(fine.left_bottom(), vec2(k, 0.0)),
        corner(fine.left_bottom(), vec2(0.0, -k)),
        corner(fine.left_top(), vec2(0.0, k)),
    ]
    .to_vec();
    p.add(Shape::closed_line(line, Stroke::new(w * 0.006, FINE)));
    p.line_segment([a, b], Stroke::new(w * 0.0092, GOLD_MID));
    let (dx, dy) = (w * 0.021, w * 0.027);
    p.add(Shape::convex_polygon(vec![c + vec2(0.0, -dy), c + vec2(dx, 0.0), c + vec2(0.0, dy), c + vec2(-dx, 0.0)], GOLD_TOP, Stroke::new(w * 0.006, GOLD_FOOT)));
    pip(p, pos2(r.left() + w * 0.3007, r.top() + h * 0.0915), w * 0.0866, suit, col, false);
    pip(p, pos2(r.right() - w * 0.3007, r.bottom() - h * 0.0915), w * 0.0866, suit, col, true);
}

/// A court card's frame (0.13.28's): its gold top, middle and foot, the figure's ground, the fine
/// line inside the gold.
const GOLD_TOP: Color32 = Color32::from_rgb(0xfb, 0xe3, 0x93);
const GOLD_MID: Color32 = Color32::from_rgb(0xd9, 0xa8, 0x3c);
const GOLD_FOOT: Color32 = Color32::from_rgb(0xa8, 0x76, 0x1c);
const CREAM: Color32 = Color32::from_rgb(0xf1, 0xe2, 0xc1);
const FINE: Color32 = Color32::from_rgb(0xe0, 0xb8, 0x61);

/// A ring `width` wide just inside `r` (its outer corners rounded by `rad`), with soft edges a
/// pixel wide, each point the color `at` its height gives.
fn soft_round_ring(p: &Painter, r: Rect, rad: f32, width: f32, at: impl Fn(f32) -> Color32) {
    let (seg, side) = (8, 12);
    // (The sides cut in pieces too, so the color follows the height all the way down.)
    let outline = |r: Rect, rad: f32| {
        let rad = rad.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
        let corners = [(pos2(r.right() - rad, r.bottom() - rad), 0.0f32), (pos2(r.left() + rad, r.bottom() - rad), 90.0), (pos2(r.left() + rad, r.top() + rad), 180.0), (pos2(r.right() - rad, r.top() + rad), 270.0)];
        let mut pts = Vec::with_capacity(4 * (seg + 1 + side));
        for (k, (c, a0)) in corners.iter().enumerate() {
            for i in 0..=seg {
                let a = (a0 + 90.0 * i as f32 / seg as f32).to_radians();
                pts.push((*c + vec2(a.cos(), a.sin()) * rad, vec2(a.cos(), a.sin())));
            }
            let a = (a0 + 90.0).to_radians();
            let nrm = vec2(a.cos(), a.sin());
            let (from, to) = (*c + nrm * rad, corners[(k + 1) % 4].0 + nrm * rad);
            for j in 1..=side {
                pts.push((from + (to - from) * (j as f32 / (side + 1) as f32), nrm));
            }
        }
        pts
    };
    let (outer, inner) = (outline(r, rad), outline(r.shrink(width), (rad - width).max(0.0)));
    let half = 0.5 / p.ctx().pixels_per_point();
    let mut m = egui::epaint::Mesh::default();
    for ((o, no), (i, ni)) in outer.iter().zip(&inner) {
        // (Each point's way out: along its corner's radius, a straight side's own normal.)
        let no = if no.length_sq() > 0.0 { *no } else { *ni };
        m.colored_vertex(*o + no * half, Color32::TRANSPARENT);
        m.colored_vertex(*o - no * half, at(o.y));
        m.colored_vertex(*i + *ni * half, at(i.y));
        m.colored_vertex(*i - *ni * half, Color32::TRANSPARENT);
    }
    let n = outer.len() as u32;
    for k in 0..n {
        let j = (k + 1) % n;
        for band in 0..3 {
            let (a0, a1, b0, b1) = (4 * k + band, 4 * k + band + 1, 4 * j + band, 4 * j + band + 1);
            m.add_triangle(a0, a1, b1);
            m.add_triangle(a0, b1, b0);
        }
    }
    p.add(Shape::mesh(m));
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
    // The card's body, and the painting in it a little in from its edge, the whole of it
    // (squeezed while it turns over). (0.13.28's: a white margin round the painting.)
    body(p, r, round);
    let art = r.shrink2(vec2(r.width() * 0.036, r.height() * 0.038));
    if !crate::casino::pics::stretch_rounded(p, p.ctx(), back_art(design), art, rad * 0.6, texture_px(art), Color32::WHITE) {
        drawn_back(p, art, design);
    }
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
