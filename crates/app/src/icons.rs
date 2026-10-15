//! Small vector icons drawn with the painter (crisp at any DPI, no icon font needed).

use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::sync::OnceLock;

use eframe::egui::epaint::PathStroke;
use eframe::egui::{Color32, CornerRadius, Mesh, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};

fn arc(c: Pos2, r: f32, a0: f32, a1: f32, n: usize) -> Vec<Pos2> {
    (0..=n).map(|i| {
        let a = a0 + (a1 - a0) * i as f32 / n as f32;
        pos2(c.x + r * a.cos(), c.y + r * a.sin())
    })
    .collect()
}

fn slash(p: &Painter, r: Rect) {
    let s = r.size().x;
    let a = r.left_top() + vec2(s * 0.18, s * 0.18);
    let b = r.right_bottom() - vec2(s * 0.18, s * 0.18);
    p.line_segment([a, b], Stroke::new(s * 0.16, Color32::from_rgb(0x1e, 0x19, 0x16)));
    p.line_segment([a, b], Stroke::new(s * 0.09, crate::theme::LIVE));
}

pub fn mic(p: &Painter, r: Rect, c: Color32, off: bool) {
    let s = r.width();
    let w = s * 0.11;
    let cx = r.center().x;
    let body = Rect::from_center_size(pos2(cx, r.top() + s * 0.36), vec2(s * 0.30, s * 0.46));
    p.rect_filled(body, CornerRadius::same((s * 0.15) as u8), c);
    let cup = arc(pos2(cx, r.top() + s * 0.42), s * 0.27, 0.0, PI, 16);
    p.add(Shape::line(cup, Stroke::new(w * 0.8, c)));
    p.line_segment([pos2(cx, r.top() + s * 0.69), pos2(cx, r.top() + s * 0.86)], Stroke::new(w * 0.8, c));
    p.line_segment([pos2(cx - s * 0.16, r.top() + s * 0.87), pos2(cx + s * 0.16, r.top() + s * 0.87)], Stroke::new(w * 0.8, c));
    if off {
        slash(p, r);
    }
}

pub fn headphones(p: &Painter, r: Rect, c: Color32, off: bool) {
    let s = r.width();
    let w = s * 0.09;
    let band = arc(pos2(r.center().x, r.top() + s * 0.55), s * 0.34, PI, TAU, 20);
    p.add(Shape::line(band, Stroke::new(w, c)));
    for dx in [-1.0f32, 1.0] {
        let cup = Rect::from_center_size(pos2(r.center().x + dx * s * 0.30, r.top() + s * 0.66), vec2(s * 0.17, s * 0.30));
        p.rect_filled(cup, CornerRadius::same((s * 0.06) as u8), c);
    }
    if off {
        slash(p, r);
    }
}

pub fn camera(p: &Painter, r: Rect, c: Color32, off: bool) {
    let s = r.width();
    let body = Rect::from_min_size(r.left_top() + vec2(s * 0.1, s * 0.28), vec2(s * 0.56, s * 0.44));
    p.rect_filled(body, CornerRadius::same((s * 0.08) as u8), c);
    let tri = vec![
        pos2(r.left() + s * 0.70, r.top() + s * 0.44),
        pos2(r.left() + s * 0.90, r.top() + s * 0.30),
        pos2(r.left() + s * 0.90, r.top() + s * 0.70),
        pos2(r.left() + s * 0.70, r.top() + s * 0.56),
    ];
    p.add(Shape::convex_polygon(tri, c, Stroke::NONE));
    if off {
        slash(p, r);
    }
}

pub fn screen(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    let mon = Rect::from_min_size(r.left_top() + vec2(s * 0.1, s * 0.18), vec2(s * 0.8, s * 0.52));
    p.rect_stroke(mon, CornerRadius::same((s * 0.06) as u8), Stroke::new(s * 0.09, c), StrokeKind::Inside);
    p.line_segment([pos2(r.center().x, mon.bottom()), pos2(r.center().x, r.top() + s * 0.82)], Stroke::new(s * 0.09, c));
    p.line_segment([pos2(r.center().x - s * 0.2, r.top() + s * 0.84), pos2(r.center().x + s * 0.2, r.top() + s * 0.84)], Stroke::new(s * 0.09, c));
    // up arrow ("share")
    let ac = mon.center();
    p.line_segment([pos2(ac.x, ac.y + s * 0.12), pos2(ac.x, ac.y - s * 0.1)], Stroke::new(s * 0.07, c));
    p.add(Shape::line(
        vec![pos2(ac.x - s * 0.1, ac.y), pos2(ac.x, ac.y - s * 0.11), pos2(ac.x + s * 0.1, ac.y)],
        Stroke::new(s * 0.07, c),
    ));
}

pub fn gear(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    let ctr = r.center();
    for i in 0..8 {
        let a = i as f32 / 8.0 * TAU;
        let d = vec2(a.cos(), a.sin());
        p.line_segment([ctr + d * s * 0.26, ctr + d * s * 0.42], Stroke::new(s * 0.13, c));
    }
    p.circle_stroke(ctr, s * 0.25, Stroke::new(s * 0.12, c));
}

pub fn hangup(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    let ctr = r.center() + vec2(0.0, s * 0.08);
    let pts = arc(ctr + vec2(0.0, s * 0.18), s * 0.42, PI * 1.18, PI * 1.82, 16);
    p.add(Shape::line(pts, Stroke::new(s * 0.16, c)));
    for dx in [-1.0f32, 1.0] {
        let e = Rect::from_center_size(ctr + vec2(dx * s * 0.33, s * 0.06), vec2(s * 0.18, s * 0.14));
        p.rect_filled(e, CornerRadius::same((s * 0.05) as u8), c);
    }
}

pub fn speaker(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    let poly = vec![
        pos2(r.left() + s * 0.12, r.top() + s * 0.38),
        pos2(r.left() + s * 0.30, r.top() + s * 0.38),
        pos2(r.left() + s * 0.52, r.top() + s * 0.18),
        pos2(r.left() + s * 0.52, r.top() + s * 0.82),
        pos2(r.left() + s * 0.30, r.top() + s * 0.62),
        pos2(r.left() + s * 0.12, r.top() + s * 0.62),
    ];
    p.add(Shape::convex_polygon(poly, c, Stroke::NONE));
    let ctr = pos2(r.left() + s * 0.52, r.center().y);
    p.add(Shape::line(arc(ctr, s * 0.18, -0.9, 0.9, 10), Stroke::new(s * 0.08, c)));
    p.add(Shape::line(arc(ctr, s * 0.32, -0.9, 0.9, 12), Stroke::new(s * 0.08, c)));
}

pub fn hash(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    let st = Stroke::new(s * 0.09, c);
    let (l, t) = (r.left(), r.top());
    p.line_segment([pos2(l + s * 0.42, t + s * 0.15), pos2(l + s * 0.32, t + s * 0.85)], st);
    p.line_segment([pos2(l + s * 0.70, t + s * 0.15), pos2(l + s * 0.60, t + s * 0.85)], st);
    p.line_segment([pos2(l + s * 0.18, t + s * 0.37), pos2(l + s * 0.86, t + s * 0.37)], st);
    p.line_segment([pos2(l + s * 0.14, t + s * 0.63), pos2(l + s * 0.82, t + s * 0.63)], st);
}

pub fn plus(p: &Painter, r: Rect, c: Color32, w: f32) {
    let ctr = r.center();
    let h = r.width() * 0.32;
    p.line_segment([ctr - vec2(h, 0.0), ctr + vec2(h, 0.0)], Stroke::new(w, c));
    p.line_segment([ctr - vec2(0.0, h), ctr + vec2(0.0, h)], Stroke::new(w, c));
}

pub fn close(p: &Painter, r: Rect, c: Color32) {
    let ctr = r.center();
    let h = r.width() * 0.26;
    p.line_segment([ctr - vec2(h, h), ctr + vec2(h, h)], Stroke::new(r.width() * 0.1, c));
    p.line_segment([ctr + vec2(-h, h), ctr + vec2(h, -h)], Stroke::new(r.width() * 0.1, c));
}

pub fn lock(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    let body = Rect::from_min_size(r.left_top() + vec2(s * 0.2, s * 0.45), vec2(s * 0.6, s * 0.42));
    p.rect_filled(body, CornerRadius::same((s * 0.08) as u8), c);
    p.add(Shape::line(arc(pos2(r.center().x, r.top() + s * 0.45), s * 0.2, PI, TAU, 12), Stroke::new(s * 0.1, c)));
}

pub fn chat(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    let b = Rect::from_min_size(r.left_top() + vec2(s * 0.12, s * 0.16), vec2(s * 0.76, s * 0.54));
    p.rect_filled(b, CornerRadius::same((s * 0.16) as u8), c);
    let tail = vec![pos2(b.left() + s * 0.18, b.bottom() - 1.0), pos2(b.left() + s * 0.12, b.bottom() + s * 0.18), pos2(b.left() + s * 0.36, b.bottom() - 1.0)];
    p.add(Shape::convex_polygon(tail, c, Stroke::NONE));
}

pub fn people(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    p.circle_filled(pos2(r.left() + s * 0.38, r.top() + s * 0.32), s * 0.14, c);
    p.add(Shape::convex_polygon(arc(pos2(r.left() + s * 0.38, r.top() + s * 0.82), s * 0.26, PI, TAU, 12), c, Stroke::NONE));
    p.circle_filled(pos2(r.left() + s * 0.70, r.top() + s * 0.36), s * 0.11, c);
    p.add(Shape::convex_polygon(arc(pos2(r.left() + s * 0.72, r.top() + s * 0.80), s * 0.19, PI, TAU, 10), c, Stroke::NONE));
}

pub fn signal(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    for (i, h) in [0.3f32, 0.55, 0.8].iter().enumerate() {
        let x = r.left() + s * (0.12 + i as f32 * 0.3);
        let bar = Rect::from_min_max(pos2(x, r.bottom() - s * h), pos2(x + s * 0.18, r.bottom()));
        p.rect_filled(bar, CornerRadius::same(1), c);
    }
}

pub fn reply(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    let pts = vec![
        pos2(r.left() + s * 0.85, r.top() + s * 0.78),
        pos2(r.left() + s * 0.80, r.top() + s * 0.55),
        pos2(r.left() + s * 0.62, r.top() + s * 0.42),
        pos2(r.left() + s * 0.22, r.top() + s * 0.42),
    ];
    p.add(Shape::line(pts, Stroke::new(s * 0.1, c)));
    p.add(Shape::line(
        vec![pos2(r.left() + s * 0.42, r.top() + s * 0.22), pos2(r.left() + s * 0.2, r.top() + s * 0.42), pos2(r.left() + s * 0.42, r.top() + s * 0.62)],
        Stroke::new(s * 0.1, c),
    ));
}

pub fn pencil(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    let a = pos2(r.left() + s * 0.25, r.top() + s * 0.75);
    let b = pos2(r.left() + s * 0.75, r.top() + s * 0.25);
    p.line_segment([a, b], Stroke::new(s * 0.16, c));
    p.circle_filled(a + vec2(-s * 0.04, s * 0.04), s * 0.05, c);
}

pub fn trash(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    let body = Rect::from_min_max(pos2(r.left() + s * 0.28, r.top() + s * 0.34), pos2(r.right() - s * 0.28, r.bottom() - s * 0.14));
    p.rect_filled(body, CornerRadius::same(2), c);
    p.line_segment([pos2(r.left() + s * 0.2, r.top() + s * 0.26), pos2(r.right() - s * 0.2, r.top() + s * 0.26)], Stroke::new(s * 0.08, c));
    p.line_segment([pos2(r.center().x - s * 0.08, r.top() + s * 0.18), pos2(r.center().x + s * 0.08, r.top() + s * 0.18)], Stroke::new(s * 0.08, c));
}

pub fn smile(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    let ctr = r.center();
    p.circle_stroke(ctr, s * 0.36, Stroke::new(s * 0.08, c));
    p.circle_filled(ctr + vec2(-s * 0.12, -s * 0.08), s * 0.05, c);
    p.circle_filled(ctr + vec2(s * 0.12, -s * 0.08), s * 0.05, c);
    p.add(Shape::line(arc(ctr + vec2(0.0, s * 0.02), s * 0.18, 0.4, PI - 0.4, 10), Stroke::new(s * 0.07, c)));
}

/// Ingle's campfire: the app icon's fire without its tile (the home button, the welcome screen).
/// Drawn from the same shapes as the icon (tools/make-icons.py), on the icon's 512 grid: two
/// logs crossed in an X seen from a little above, lit by the fire.
pub fn flame(p: &Painter, r: Rect) {
    let fire = fire();
    // The fire spans y 66-455 of the grid, flame tip to log ends: fit that to `r`, centred.
    let u = r.height() * 0.96 / 389.0;
    let at = |q: Pos2| r.center() + (q - pos2(256.0, 260.5)) * u;
    let hair = 1.0 / p.ctx().pixels_per_point();
    let cross = at(pos2(256.0, 384.0));
    let (half, radius) = (124.0 * u, 26.0 * u);
    let (sin, cos) = 17.0f32.to_radians().sin_cos();
    // B (near end at the lower left) lies under A.
    let b = (vec2(-cos, sin), vec2(sin, cos));
    log(p, cross, b, half, radius, None, hair);
    log(p, cross, (vec2(cos, sin), vec2(-sin, cos)), half, radius, None, hair);
    // The fire sits on top of the logs. egui only smooths the edges of convex shapes, and a flame is anything but; so each part
    // is filled as triangles, then traced with a one-pixel line of the same colour.
    for (outline, tris, colour) in [(&fire.flame, &fire.flame_tris, crate::theme::EMBER), (&fire.core, &fire.core_tris, GOLD)] {
        let pts: Vec<Pos2> = outline.iter().map(|&q| at(q)).collect();
        let mut mesh = Mesh::default();
        for &q in &pts {
            mesh.colored_vertex(q, colour);
        }
        for t in tris {
            mesh.add_triangle(t[0], t[1], t[2]);
        }
        p.add(Shape::mesh(mesh));
        p.add(Shape::closed_line(pts, Stroke::new(hair, colour)));
    }
}

const GOLD: Color32 = Color32::from_rgb(0xff, 0xd2, 0x7a);
const BARK: Color32 = Color32::from_rgb(0x5a, 0x34, 0x20);
const WOOD: Color32 = Color32::from_rgb(0xc9, 0x98, 0x68);
/// Across a log, from its top edge (0) to its underside (1): where the fire catches it, the
/// side we see, the underside. LOG_SHADING in tools/make-icons.py.
const LOG_SHADING: [(f32, [u8; 3]); 4] =
    [(0.0, [0xe7, 0xa0, 0x61]), (0.22, [0xf0, 0xaa, 0x68]), (0.58, [0xb4, 0x74, 0x47]), (1.0, [0x6a, 0x3e, 0x26])];

fn log_shade(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let i = LOG_SHADING.iter().position(|s| t <= s.0).unwrap_or(LOG_SHADING.len() - 1).max(1);
    let ((t0, c0), (t1, c1)) = (LOG_SHADING[i - 1], LOG_SHADING[i]);
    let k = (t - t0) / (t1 - t0);
    let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k).round() as u8;
    Color32::from_rgb(mix(c0[0], c1[0]), mix(c0[1], c1[1]), mix(c0[2], c1[2]))
}

/// One log: centred on `c`, `dir.0` pointing to its near end and `dir.1` to its underside,
/// `half` from its centre to each end. Its ends are circles seen at an angle, so ellipses
/// squashed along the log; the near one shows its cut face. Drawn from `from` along the log
/// (negative: toward the far end) to the near end; None draws all of it.
fn log(p: &Painter, c: Pos2, dir: (Vec2, Vec2), half: f32, r: f32, from: Option<f32>, hair: f32) {
    let (along, down) = dir;
    let kr = 0.53 * r;
    const STEPS: usize = 16;
    let mut shape: Vec<Vec2> = (0..=STEPS)
        .map(|i| -FRAC_PI_2 + PI * i as f32 / STEPS as f32)
        .map(|a| vec2(half + kr * a.cos(), r * a.sin()))
        .chain((0..=STEPS).map(|i| FRAC_PI_2 + PI * i as f32 / STEPS as f32).map(|a| vec2(-half + kr * a.cos(), r * a.sin())))
        .collect();
    if let Some(x0) = from {
        shape = clip(&shape, |q| q.x - x0);
    }
    let place = move |q: Vec2| c + along * q.x + down * q.y;
    // Lit top fading to the dark underside: a strip per stretch of LOG_SHADING, each coloured
    // at its corners so the colours blend across it.
    let mut mesh = Mesh::default();
    for s in LOG_SHADING.windows(2) {
        let (y0, y1) = (r * (2.0 * s[0].0 - 1.0), r * (2.0 * s[1].0 - 1.0));
        let strip = clip(&clip(&shape, |q| q.y - y0), |q| y1 - q.y);
        if strip.len() < 3 {
            continue;
        }
        let first = mesh.vertices.len() as u32;
        for &q in &strip {
            mesh.colored_vertex(place(q), log_shade((q.y / r + 1.0) / 2.0));
        }
        for i in 1..strip.len() as u32 - 1 {
            mesh.add_triangle(first, first + i, first + i + 1);
        }
    }
    p.add(Shape::mesh(mesh));
    // a hairline round it, shaded the same way, for a smooth edge
    let outline: Vec<Pos2> = shape.iter().map(|&q| place(q)).collect();
    p.add(Shape::closed_line(outline, PathStroke::new_uv(hair, move |_, q: Pos2| log_shade(((q - c).dot(down) / r + 1.0) / 2.0))));
    // the cut face at the near end: a bark rim round pale wood
    let ellipse = |rx: f32, ry: f32| -> Vec<Pos2> {
        (0..32).map(|i| i as f32 / 32.0 * TAU).map(|a| place(vec2(half + rx * a.cos(), ry * a.sin()))).collect()
    };
    p.add(Shape::convex_polygon(ellipse(kr, r), BARK, Stroke::NONE));
    p.add(Shape::convex_polygon(ellipse(kr * 0.8, r * 0.84), WOOD, Stroke::NONE));
}

/// The part of a convex polygon where `side` is zero or more.
fn clip(poly: &[Vec2], side: impl Fn(Vec2) -> f32) -> Vec<Vec2> {
    let mut out = Vec::with_capacity(poly.len() + 2);
    for (i, &a) in poly.iter().enumerate() {
        let b = poly[(i + 1) % poly.len()];
        let (sa, sb) = (side(a), side(b));
        if sa >= 0.0 {
            out.push(a);
        }
        if (sa >= 0.0) != (sb >= 0.0) {
            out.push(a + (b - a) * (sa / (sa - sb)));
        }
    }
    out
}

/// The flame and its gold heart in a 100 x 140 box, base centre at (50, 140): a start point,
/// then (control, control, end) per cubic curve. FLAME and CORE in tools/make-icons.py.
const FLAME: (Pos2, [[f32; 6]; 7]) = (
    pos2(50.0, 140.0),
    [
        [76.0, 140.0, 96.0, 121.0, 96.0, 96.0],
        [96.0, 74.0, 87.0, 60.0, 77.0, 46.0],
        [68.0, 33.0, 61.0, 19.0, 60.0, 0.0],
        [47.0, 13.0, 37.0, 31.0, 39.0, 58.0],
        [31.0, 53.0, 26.0, 45.0, 25.0, 33.0],
        [14.0, 46.0, 4.0, 65.0, 4.0, 96.0],
        [4.0, 121.0, 24.0, 140.0, 50.0, 140.0],
    ],
);
const CORE: (Pos2, [[f32; 6]; 5]) = (
    pos2(50.0, 135.0),
    [
        [63.0, 135.0, 72.0, 125.0, 72.0, 112.0],
        [72.0, 99.0, 63.0, 91.0, 56.0, 84.0],
        [51.0, 79.0, 48.0, 73.0, 47.0, 66.0],
        [40.0, 79.0, 28.0, 93.0, 28.0, 112.0],
        [28.0, 125.0, 37.0, 135.0, 50.0, 135.0],
    ],
);

/// The flame's outlines on the icon's grid (300 tall, base at 256,366) and their triangles,
/// worked out once.
struct Fire {
    flame: Vec<Pos2>,
    flame_tris: Vec<[u32; 3]>,
    core: Vec<Pos2>,
    core_tris: Vec<[u32; 3]>,
}

fn fire() -> &'static Fire {
    static FIRE: OnceLock<Fire> = OnceLock::new();
    FIRE.get_or_init(|| {
        let k = 300.0 / 140.0;
        let place = |start: Pos2, curves: &[[f32; 6]]| -> Vec<Pos2> {
            outline(start, curves).into_iter().map(|q| pos2(256.0 + (q.x - 50.0) * k, 66.0 + q.y * k)).collect()
        };
        let flame = place(FLAME.0, &FLAME.1);
        let core = place(CORE.0, &CORE.1);
        Fire { flame_tris: triangulate(&flame), flame, core_tris: triangulate(&core), core }
    })
}

/// Points along a closed run of cubic curves (the last one ends where the first starts).
fn outline(start: Pos2, curves: &[[f32; 6]]) -> Vec<Pos2> {
    const STEPS: usize = 12;
    let mut pts = vec![start];
    let mut p0 = start;
    for c in curves {
        let (p1, p2, p3) = (pos2(c[0], c[1]), pos2(c[2], c[3]), pos2(c[4], c[5]));
        for i in 1..=STEPS {
            let t = i as f32 / STEPS as f32;
            let s = 1.0 - t;
            pts.push((p0.to_vec2() * (s * s * s) + p1.to_vec2() * (3.0 * s * s * t) + p2.to_vec2() * (3.0 * s * t * t) + p3.to_vec2() * (t * t * t)).to_pos2());
        }
        p0 = p3;
    }
    pts.pop(); // back at the start
    pts
}

/// Triangles covering a simple polygon, by ear clipping (a few dozen points, done once).
fn triangulate(pts: &[Pos2]) -> Vec<[u32; 3]> {
    let cross = |a: Pos2, b: Pos2, c: Pos2| (b - a).x * (c - a).y - (b - a).y * (c - a).x;
    let mut ring: Vec<usize> = (0..pts.len()).collect();
    // Wind it so that corners pointing out turn the positive way.
    let area: f32 = (0..pts.len()).map(|i| cross(Pos2::ZERO, pts[i], pts[(i + 1) % pts.len()])).sum();
    if area < 0.0 {
        ring.reverse();
    }
    let mut tris = Vec::with_capacity(pts.len());
    while ring.len() > 3 {
        let m = ring.len();
        let ear = (0..m).find(|&i| {
            let (a, b, c) = (pts[ring[(i + m - 1) % m]], pts[ring[i]], pts[ring[(i + 1) % m]]);
            cross(a, b, c) > 0.0
                && !ring.iter().any(|&j| {
                    let q = pts[j];
                    q != a && q != b && q != c && cross(a, b, q) >= 0.0 && cross(b, c, q) >= 0.0 && cross(c, a, q) >= 0.0
                })
        });
        let Some(i) = ear else { break }; // not a simple polygon: draw what we have
        tris.push([ring[(i + m - 1) % m] as u32, ring[i] as u32, ring[(i + 1) % m] as u32]);
        ring.remove(i);
    }
    if ring.len() == 3 {
        tris.push([ring[0] as u32, ring[1] as u32, ring[2] as u32]);
    }
    tris
}

pub fn crown(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    let base = Rect::from_min_max(pos2(r.left() + s * 0.15, r.top() + s * 0.55), pos2(r.right() - s * 0.15, r.bottom() - s * 0.18));
    p.rect_filled(base, CornerRadius::same(1), c);
    for (x0, x1, tip) in [(0.15f32, 0.45f32, 0.30f32), (0.35, 0.65, 0.50), (0.55, 0.85, 0.70)] {
        let tri = vec![
            pos2(r.left() + s * x0, r.top() + s * 0.58),
            pos2(r.left() + s * tip, r.top() + s * 0.22),
            pos2(r.left() + s * x1, r.top() + s * 0.58),
        ];
        p.add(Shape::convex_polygon(tri, c, Stroke::NONE));
    }
}

/// A cloud (renting a server).
pub fn cloud(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    let base = r.top() + s * 0.68;
    p.circle_filled(pos2(r.left() + s * 0.34, base - s * 0.12), s * 0.17, c);
    p.circle_filled(pos2(r.left() + s * 0.55, base - s * 0.22), s * 0.23, c);
    p.circle_filled(pos2(r.left() + s * 0.74, base - s * 0.10), s * 0.15, c);
    p.rect_filled(Rect::from_min_max(pos2(r.left() + s * 0.20, base - s * 0.14), pos2(r.left() + s * 0.86, base + s * 0.05)), CornerRadius::same((s * 0.09) as u8), c);
}

/// A rack server (a machine you already have).
pub fn server(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    for (i, y) in [0.20f32, 0.52].iter().enumerate() {
        let b = Rect::from_min_size(pos2(r.left() + s * 0.14, r.top() + s * y), vec2(s * 0.72, s * 0.26));
        p.rect_stroke(b, CornerRadius::same((s * 0.05) as u8), Stroke::new(s * 0.07, c), StrokeKind::Inside);
        p.circle_filled(pos2(b.right() - s * 0.14, b.center().y), s * 0.045, if i == 0 { crate::theme::ONLINE } else { c });
        p.line_segment([pos2(b.left() + s * 0.12, b.center().y), pos2(b.left() + s * 0.34, b.center().y)], Stroke::new(s * 0.06, c));
    }
}

/// Two chain links (joining with an invite link).
pub fn link(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    let stroke = Stroke::new(s * 0.10, c);
    // A capsule along the diagonal, as a closed outline.
    let capsule = |center: Pos2| {
        let (len, rad) = (s * 0.17, s * 0.15);
        let dir = vec2(1.0, -1.0).normalized();
        let n = vec2(dir.y, -dir.x);
        let a = center - dir * len;
        let b = center + dir * len;
        let base = n.y.atan2(n.x);
        let mut pts = Vec::new();
        for i in 0..=12 {
            let t = base + PI * i as f32 / 12.0;
            pts.push(b + vec2(t.cos(), t.sin()) * rad);
        }
        for i in 0..=12 {
            let t = base + PI + PI * i as f32 / 12.0;
            pts.push(a + vec2(t.cos(), t.sin()) * rad);
        }
        Shape::closed_line(pts, stroke)
    };
    p.add(capsule(r.center() + vec2(-s * 0.13, s * 0.13)));
    p.add(capsule(r.center() + vec2(s * 0.13, -s * 0.13)));
}

/// A paper plane pointing right: send the message.
pub fn send(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    let ctr = r.center();
    let nose = ctr + vec2(s * 0.44, 0.0);
    let notch = ctr + vec2(-s * 0.16, 0.0);
    // Two wings meeting along the fold, clockwise (egui's preferred winding).
    p.add(Shape::convex_polygon(vec![nose, ctr + vec2(-s * 0.40, s * 0.38), notch], c, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![nose, notch, ctr + vec2(-s * 0.40, -s * 0.38)], c, Stroke::NONE));
}

/// Two arrows chasing each other round a circle (Check for updates). `turn` spins it, in
/// radians.
pub fn refresh(p: &Painter, r: Rect, c: Color32, turn: f32) {
    let s = r.width().min(r.height());
    let ctr = r.center();
    let rad = s * 0.32;
    let w = (s * 0.09).max(1.6);
    let at = |a: f32, k: f32| pos2(ctr.x + (rad + k) * a.cos(), ctr.y + (rad + k) * a.sin());
    for half in [0.0, PI] {
        let a0 = turn + half + 0.35;
        let a1 = turn + half + PI - 0.55;
        p.add(Shape::line(arc(ctr, rad, a0, a1, 12), Stroke::new(w, c)));
        // The arrowhead, pointing on round (clockwise on screen), wound clockwise.
        let h = s * 0.14;
        p.add(Shape::convex_polygon(vec![at(a1 + 0.42, 0.0), at(a1, -h), at(a1, h)], c, Stroke::NONE));
    }
}

/// A jigsaw piece (Plugins).
pub fn puzzle(p: &Painter, r: Rect, c: Color32) {
    let s = r.width().min(r.height());
    let o = r.center() - vec2(s / 2.0, s / 2.0);
    let at = |x: f32, y: f32| o + vec2(x * s, y * s);
    // The body, a little rounded, with a round knob on a neck on top and on the right.
    let body = Rect::from_min_max(at(0.08, 0.32), at(0.70, 0.94));
    p.rect_filled(body, CornerRadius::same((s * 0.09) as u8), c);
    p.rect_filled(Rect::from_min_max(at(0.32, 0.18), at(0.46, 0.34)), CornerRadius::ZERO, c);
    p.circle_filled(at(0.39, 0.16), s * 0.13, c);
    p.rect_filled(Rect::from_min_max(at(0.68, 0.56), at(0.84, 0.70)), CornerRadius::ZERO, c);
    p.circle_filled(at(0.84, 0.63), s * 0.13, c);
}

/// A tick (in a checkbox).
pub fn tick(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    let pts = vec![r.left_top() + vec2(s * 0.22, s * 0.52), r.left_top() + vec2(s * 0.42, s * 0.72), r.left_top() + vec2(s * 0.78, s * 0.30)];
    p.add(Shape::line(pts, Stroke::new(s * 0.14, c)));
}

/// A tick in a circle (a finished step).
pub fn done(p: &Painter, r: Rect, c: Color32) {
    let s = r.width();
    p.circle_filled(r.center(), s * 0.45, c);
    let pts = vec![r.left_top() + vec2(s * 0.28, s * 0.52), r.left_top() + vec2(s * 0.44, s * 0.67), r.left_top() + vec2(s * 0.72, s * 0.36)];
    p.add(Shape::line(pts, Stroke::new(s * 0.11, Color32::from_rgb(0x1b, 0x14, 0x11))));
}

/// A cross in a circle (a failed step).
pub fn failed(p: &Painter, r: Rect) {
    let s = r.width();
    p.circle_filled(r.center(), s * 0.45, crate::theme::LIVE);
    let k = s * 0.17;
    let st = Stroke::new(s * 0.1, Color32::WHITE);
    p.line_segment([r.center() + vec2(-k, -k), r.center() + vec2(k, k)], st);
    p.line_segment([r.center() + vec2(-k, k), r.center() + vec2(k, -k)], st);
}

/// Four corner brackets: full screen (or, with `exit`, pointing inward to leave it).
pub fn fullscreen(p: &Painter, r: Rect, c: Color32, exit: bool) {
    let s = Stroke::new(2.0f32, c);
    let l = r.width() * 0.32;
    for (corner, dx, dy) in [
        (r.left_top(), 1.0, 1.0),
        (r.right_top(), -1.0, 1.0),
        (r.left_bottom(), 1.0, -1.0),
        (r.right_bottom(), -1.0, -1.0),
    ] {
        // The bend sits on the corner (enter) or a bracket's length in from it (exit).
        let k = if exit { corner + vec2(dx * l, dy * l) } else { corner };
        let (sx, sy) = if exit { (-dx, -dy) } else { (dx, dy) };
        p.line_segment([k, k + vec2(sx * l, 0.0)], s);
        p.line_segment([k, k + vec2(0.0, sy * l)], s);
    }
}

/// A five-pointed star, filled (a favorite) or outlined.
pub fn star(p: &Painter, r: Rect, c: Color32, filled: bool) {
    let ctr = r.center() + vec2(0.0, r.height() * 0.04);
    let outer = r.width().min(r.height()) * 0.5;
    let inner = outer * 0.42;
    let pts: Vec<Pos2> = (0..10)
        .map(|i| {
            let a = -FRAC_PI_2 + i as f32 * PI / 5.0;
            let rad = if i % 2 == 0 { outer } else { inner };
            pos2(ctr.x + rad * a.cos(), ctr.y + rad * a.sin())
        })
        .collect();
    if filled {
        // A fan from the middle (a star isn't convex, so one polygon would fill it wrong).
        let mut mesh = Mesh::default();
        mesh.colored_vertex(ctr, c);
        for q in &pts {
            mesh.colored_vertex(*q, c);
        }
        for i in 0..10u32 {
            mesh.add_triangle(0, 1 + i, 1 + (i + 1) % 10);
        }
        p.add(Shape::mesh(mesh));
    } else {
        p.add(Shape::closed_line(pts, Stroke::new((outer * 0.16).max(1.2), c)));
    }
}

/// A rounded square: stop.
pub fn stop(p: &Painter, r: Rect, c: Color32) {
    let s = r.width().min(r.height());
    p.rect_filled(Rect::from_center_size(r.center(), vec2(s * 0.62, s * 0.62)), CornerRadius::same((s * 0.12) as u8), c);
}

/// The soundboard: a 2×2 board of pads, one lit, with a sound coming off it.
pub fn soundboard(p: &Painter, r: Rect, c: Color32) {
    let s = r.width().min(r.height());
    let pad = s * 0.3;
    let gap = s * 0.08;
    // The board at the bottom left, the waves rising off it to the top right.
    let o = Rect::from_center_size(r.center(), vec2(s, s)).min + vec2(s * 0.02, s * 0.3);
    let round = CornerRadius::same((s * 0.07) as u8);
    for (i, j) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let q = Rect::from_min_size(o + vec2(i as f32 * (pad + gap), j as f32 * (pad + gap)), vec2(pad, pad));
        if (i, j) == (1, 0) {
            p.rect_filled(q, round, c);
        } else {
            p.rect_stroke(q, round, Stroke::new((s * 0.075).max(1.2), c), StrokeKind::Inside);
        }
    }
    // Two little waves off the lit pad's corner.
    let k = o + vec2(2.0 * pad + gap, 0.0);
    for rad in [s * 0.14, s * 0.26] {
        p.add(Shape::line(arc(k, rad, -FRAC_PI_2 + 0.25, -0.25, 8), Stroke::new((s * 0.07).max(1.1), c)));
    }
}

pub fn clock(p: &Painter, r: Rect, c: Color32) {
    let s = Stroke::new(1.8f32, c);
    let rad = r.width().min(r.height()) / 2.0;
    p.circle_stroke(r.center(), rad - 1.0, s);
    p.line_segment([r.center(), r.center() + vec2(0.0, -rad * 0.55)], s);
    p.line_segment([r.center(), r.center() + vec2(rad * 0.45, rad * 0.2)], s);
}

pub fn gif(p: &Painter, r: Rect, c: Color32) {
    p.rect_stroke(r.shrink(1.0), 3.0, Stroke::new(1.6f32, c), StrokeKind::Inside);
    p.text(r.center(), eframe::egui::Align2::CENTER_CENTER, "GIF", crate::theme::bold_font(r.height() * 0.42), c);
}

pub fn eye(p: &Painter, r: Rect, c: Color32) {
    let s = Stroke::new(1.6f32, c);
    let (cx, cy, w, h) = (r.center().x, r.center().y, r.width() / 2.0, r.height() / 2.0);
    let top: Vec<Pos2> = (0..=12).map(|i| {
        let t = i as f32 / 12.0 * PI;
        pos2(cx - w * t.cos(), cy - h * 0.85 * t.sin())
    }).collect();
    let bottom: Vec<Pos2> = (0..=12).map(|i| {
        let t = i as f32 / 12.0 * PI;
        pos2(cx - w * t.cos(), cy + h * 0.85 * t.sin())
    }).collect();
    p.add(Shape::line(top, s));
    p.add(Shape::line(bottom, s));
    p.circle_filled(r.center(), h * 0.45, c);
}
