// The staff titles' emblems over a seat's picture (0.13.4/0.13.6), as 0.13.28 draws them: the
// Owner of the Game's winged shield with a crown, a Game Master's with a gem, an Ambassador's
// small silver shield. Shapes only (epaint), so `work/shapelab` can draw them too: they were
// traced off 0.13.28's screen drawn three times over (WINIT_X11_SCALE_FACTOR=3), on the seat
// with the title and without it.
//
// Every length is in the table's units (a picture is 44 across... its radius is 44 k): `c` is
// the middle of the seat's picture, `z` the table's scale k, `t` seconds (for the glow's beat,
// the turning rays and the sparkles).

use epaint::{Color32, Mesh, Pos2, Shape, Stroke, pos2, vec2};

/// Which emblem.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EmblemKind {
    Owner,
    GameMaster,
    Ambassador,
}

const OUTLINE: Color32 = Color32::from_rgb(90, 52, 8);
const RIM: Color32 = Color32::from_rgb(240, 199, 102);
const BEVEL: Color32 = Color32::from_rgb(222, 177, 82);
/// The wings' five feathers, top to bottom: each lighter than the one under it.
const FEATHERS: [Color32; 5] = [
    Color32::from_rgb(240, 198, 102),
    Color32::from_rgb(238, 194, 93),
    Color32::from_rgb(237, 189, 83),
    Color32::from_rgb(235, 184, 74),
    Color32::from_rgb(233, 179, 64),
];

fn mul(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a.clamp(0.0, 1.0))
}

/// A closed outline as a filled convex shape with its edge, or (for a shape that isn't convex)
/// its fill as a fan from `mid`.
fn poly(out: &mut Vec<Shape>, pts: Vec<Pos2>, fill: Color32, edge: Stroke) {
    out.push(Shape::convex_polygon(pts, fill, edge));
}

/// A fan from `mid` (a shape that's star-shaped round it, like a feather or a star).
fn fan(out: &mut Vec<Shape>, mid: Pos2, pts: &[Pos2], fill: Color32, edge: Stroke) {
    let mut m = Mesh::default();
    m.colored_vertex(mid, fill);
    for p in pts {
        m.colored_vertex(*p, fill);
    }
    let n = pts.len() as u32;
    for i in 0..n {
        m.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    out.push(Shape::mesh(m));
    if edge.width > 0.0 {
        out.push(Shape::closed_line(pts.to_vec(), edge));
    }
}

/// The quadratic curve from `a` to `b` bent toward `ctl`, as `n` points (without `a`).
fn quad(a: Pos2, ctl: Pos2, b: Pos2, n: usize) -> Vec<Pos2> {
    (1..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            pos2(u * u * a.x + 2.0 * u * t * ctl.x + t * t * b.x, u * u * a.y + 2.0 * u * t * ctl.y + t * t * b.y)
        })
        .collect()
}

/// A shield's outline: its top from corner to corner `hw` out at `top`, arched `arch` higher in
/// the middle; its sides straight down to `shoulder` (`sw` out), then each one curve to the point
/// at `tip`, bent toward (`bend.0` out, `bend.1` down). Clockwise from the top left.
fn shield(at: &dyn Fn(f32, f32) -> Pos2, hw: f32, top: f32, arch: f32, sw: f32, shoulder: f32, bend: (f32, f32), tip: f32) -> Vec<Pos2> {
    let mut pts = vec![at(-hw, top)];
    pts.extend(quad(at(-hw, top), at(0.0, top - 2.0 * arch), at(hw, top), 10));
    pts.push(at(sw, shoulder));
    pts.extend(quad(at(sw, shoulder), at(bend.0, bend.1), at(0.0, tip), 12));
    pts.extend(quad(at(0.0, tip), at(-bend.0, bend.1), at(-sw, shoulder), 12));
    pts
}

/// A star of `n` points round `mid`, the first straight up.
fn star_points(mid: Pos2, n: usize, outer: f32, inner: f32) -> Vec<Pos2> {
    (0..n * 2)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / n as f32;
            let r = if i % 2 == 0 { outer } else { inner };
            mid + vec2(a.cos(), a.sin()) * r
        })
        .collect()
}

/// A sparkle: four thin points round `mid`.
fn sparkle(out: &mut Vec<Shape>, mid: Pos2, up: f32, down: f32, side: f32, waist: f32, col: Color32) {
    let pts = vec![
        mid + vec2(0.0, -up),
        mid + vec2(waist, -waist),
        mid + vec2(side, 0.0),
        mid + vec2(waist, waist),
        mid + vec2(0.0, down),
        mid + vec2(-waist, waist),
        mid + vec2(-side, 0.0),
        mid + vec2(-waist, -waist),
    ];
    fan(out, mid, &pts, col, Stroke::NONE);
}

/// The glow behind an emblem: rings of the same faint colour, so it's brightest in the middle.
fn glow(out: &mut Vec<Shape>, mid: Pos2, r: f32, col: Color32, each: f32, rings: usize) {
    for i in 0..rings {
        let rr = r * (1.0 - i as f32 / rings as f32);
        out.push(Shape::circle_filled(mid, rr, mul(col, each)));
    }
}

/// Light turning slowly behind the emblem: nine thin rays from its middle, brightest on the
/// beat.
fn rays(out: &mut Vec<Shape>, mid: Pos2, z: f32, t: f32, strength: f32) {
    if strength <= 0.01 {
        return;
    }
    let turn = t * 20f32.to_radians();
    for i in 0..9 {
        let a = turn + i as f32 * std::f32::consts::TAU / 9.0;
        let d = vec2(a.cos(), a.sin());
        let n = vec2(-d.y, d.x);
        let from = mid + d * 20.0 * z;
        let to = mid + d * 44.0 * z;
        let mut m = Mesh::default();
        let lit = mul(Color32::from_rgb(255, 250, 235), 0.28 * strength);
        m.colored_vertex(from + n * 0.7 * z, lit);
        m.colored_vertex(from - n * 0.7 * z, lit);
        m.colored_vertex(to, Color32::TRANSPARENT);
        m.add_triangle(0, 1, 2);
        out.push(Shape::mesh(m));
    }
}

/// Sparkles round the emblem, a few at a time: each comes and goes in a second, somewhere on a
/// ring 19 to 27 out, the next ones elsewhere.
fn sparkles(out: &mut Vec<Shape>, mid: Pos2, z: f32, t: f32) {
    const EVERY: f32 = 1.5;
    for slot in 0..3u32 {
        let phase = t / EVERY + slot as f32 * 0.17;
        let n = phase.floor() as u32;
        let age = (phase - phase.floor()) * EVERY;
        if age > 1.0 {
            continue;
        }
        // (Where: a hash of which one it is, so it's the same on every frame of its second.)
        let h = n.wrapping_mul(2_654_435_761).wrapping_add(slot.wrapping_mul(40_503)).rotate_left(13).wrapping_mul(2_246_822_519);
        let side = if (n + slot) % 2 == 0 { -1.0 } else { 1.0 };
        let a = (-70.0 + (h % 1000) as f32 / 1000.0 * 120.0).to_radians();
        let r = (19.0 + ((h >> 10) % 1000) as f32 / 1000.0 * 8.0) * z;
        let at = mid + vec2(side * a.cos() * r, a.sin() * r);
        let size = (age * std::f32::consts::PI).sin() * 3.4 * z;
        sparkle(out, at, size, size, size, size * 0.22, Color32::from_rgb(255, 252, 240));
    }
}

/// The emblem's shapes for a seat whose picture's middle is `c`.
pub fn emblem(kind: EmblemKind, c: Pos2, z: f32, t: f32) -> Vec<Shape> {
    let at = |x: f32, y: f32| c + vec2(x, y) * z;
    let mut out = Vec::new();
    // (Everything breathes on a three-second beat.)
    let swing = (t * std::f32::consts::TAU / 3.0).sin();
    let beat = 1.0 + 0.12 * swing;
    let ray_strength = (0.5 + 0.5 * swing).powi(2);
    let edge = Stroke::new(0.8 * z, OUTLINE);
    match kind {
        EmblemKind::Ambassador => {
            glow(&mut out, at(0.0, -49.9), 30.5 * z, Color32::from_rgb(100, 180, 255), 0.072 * beat, 10);
            rays(&mut out, at(0.0, -49.9), z, t, ray_strength);
            let navy_edge = Stroke::new(0.8 * z, Color32::from_rgb(40, 58, 92));
            let outer = shield(&at, 10.7, -58.6, 1.6, 10.2, -52.0, (8.0, -42.0), -35.6);
            poly(&mut out, outer, Color32::from_rgb(204, 216, 235), navy_edge);
            let inner = shield(&at, 8.2, -56.6, 1.2, 7.9, -51.5, (6.2, -43.5), -38.8);
            poly(&mut out, inner, Color32::from_rgb(150, 166, 196), Stroke::NONE);
            let field = shield(&at, 7.1, -55.9, 1.0, 6.8, -51.0, (5.4, -44.5), -41.2);
            poly(&mut out, field, Color32::from_rgb(18, 58, 122), Stroke::NONE);
            // Its star: a broad point up, a thin one down.
            let star = [at(0.0, -52.7), at(6.34, -47.9), at(1.58, -46.3), at(0.0, -41.6), at(-1.58, -46.3), at(-6.34, -47.9)];
            fan(&mut out, at(0.0, -47.0), &star, Color32::from_rgb(240, 246, 255), Stroke::NONE);
        }
        EmblemKind::Owner | EmblemKind::GameMaster => {
            let owner = kind == EmblemKind::Owner;
            let (halo, each) = if owner { (Color32::from_rgb(255, 88, 60), 0.054) } else { (Color32::from_rgb(154, 106, 255), 0.045) };
            glow(&mut out, at(0.0, -49.9), 38.0 * z, halo, each * beat, 10);
            rays(&mut out, at(0.0, -49.9), z, t, ray_strength);
            // The wings: five feathers each side, the lowest first (each upper one over it).
            let feathers: [[(f32, f32); 4]; 5] = [
                [(-32.36, -67.17), (-13.47, -61.78), (-13.47, -56.63), (-24.95, -59.52)],
                [(-33.23, -59.64), (-13.47, -58.61), (-12.83, -53.58), (-23.88, -54.42)],
                [(-31.68, -52.91), (-12.67, -55.45), (-12.00, -50.18), (-22.73, -49.35)],
                [(-28.40, -47.37), (-12.55, -51.60), (-11.49, -47.13), (-21.90, -45.94)],
                [(-24.44, -43.48), (-11.72, -48.75), (-10.85, -45.39), (-15.45, -43.29)],
            ];
            for (i, f) in feathers.iter().enumerate().rev() {
                for side in [-1.0f32, 1.0] {
                    let p: Vec<Pos2> = f.iter().map(|(x, y)| at(side * x.abs(), *y)).collect();
                    let mid = pos2((p[0].x + p[1].x + p[2].x + p[3].x) / 4.0, (p[0].y + p[1].y + p[2].y + p[3].y) / 4.0);
                    fan(&mut out, mid, &p, FEATHERS[i], edge);
                }
            }
            // The shield: its rim, a darker bevel, the field.
            let outer = shield(&at, 13.2, -62.4, 1.4, 12.87, -55.8, (9.41, -43.6), -31.3);
            poly(&mut out, outer, RIM, edge);
            let bevel = shield(&at, 10.2, -58.9, 1.1, 10.0, -54.5, (7.9, -44.0), -34.85);
            poly(&mut out, bevel, BEVEL, Stroke::NONE);
            let field_col = if owner { Color32::from_rgb(112, 14, 26) } else { Color32::from_rgb(58, 26, 110) };
            let field = shield(&at, 8.4, -58.0, 0.9, 8.2, -54.0, (6.6, -45.0), -36.4);
            poly(&mut out, field, field_col, Stroke::NONE);
            // The star, its points faceted.
            let (light, dark) = if owner { (Color32::from_rgb(255, 224, 148), Color32::from_rgb(208, 172, 116)) } else { (Color32::from_rgb(246, 214, 132), Color32::from_rgb(196, 168, 114)) };
            let mid = at(0.0, -46.7);
            let pts = star_points(mid, 5, 7.4 * z, 3.6 * z);
            fan(&mut out, mid, &pts, light, Stroke::new(0.5 * z, Color32::from_rgb(183, 134, 44)));
            for p in &pts {
                out.push(Shape::line_segment([mid, *p], Stroke::new(0.35 * z, mul(dark, 0.8))));
            }
            if owner {
                // The crown: three points with pearls, a red jewel, a darker band at its foot.
                let crown = vec![at(-11.3, -63.1), at(-10.9, -71.0), at(-5.4, -67.5), at(0.0, -74.2), at(5.4, -67.5), at(10.9, -71.0), at(11.3, -63.1)];
                fan(&mut out, at(0.0, -65.5), &crown, Color32::from_rgb(255, 196, 74), Stroke::new(0.8 * z, Color32::from_rgb(130, 66, 14)));
                poly(&mut out, vec![at(-11.2, -65.9), at(11.2, -65.9), at(11.3, -63.4), at(-11.3, -63.4)], Color32::from_rgb(235, 176, 69), Stroke::NONE);
                for (x, y) in [(-12.5, -72.2), (0.0, -75.4), (12.5, -72.2)] {
                    out.push(Shape::circle_filled(at(x, y), 1.45 * z, Color32::from_rgb(255, 246, 208)));
                }
                out.push(Shape::circle_filled(at(0.0, -66.7), 1.8 * z, Color32::from_rgb(255, 74, 58)));
            } else {
                // The gem, in a faint glow of its own.
                out.push(Shape::circle_filled(at(0.0, -65.3), 6.9 * z, mul(Color32::from_rgb(154, 106, 255), 0.35)));
                let g = |x: f32, y: f32| at(x, y - 65.3);
                poly(&mut out, vec![g(0.0, -4.4), g(3.6, 0.0), g(0.0, 4.4), g(-3.6, 0.0)], Color32::from_rgb(255, 243, 195), Stroke::NONE);
                poly(&mut out, vec![g(0.0, -3.3), g(2.7, 0.0), g(0.0, 3.3), g(-2.7, 0.0)], Color32::from_rgb(165, 122, 255), Stroke::NONE);
                poly(&mut out, vec![g(0.0, -3.3), g(2.7, 0.0), g(-2.7, 0.0)], Color32::from_rgb(210, 188, 255), Stroke::NONE);
            }
        }
    }
    sparkles(&mut out, at(0.0, -49.9), z, t);
    out
}
