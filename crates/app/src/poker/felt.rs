//! The table itself, drawn properly: the room's light, a padded leather rail lit from above, a
//! brass trim, and cloth with a weave (a texture made once per look, stretched over the oval).

use eframe::egui::{self, Color32, ColorImage, Mesh, Painter, Pos2, Rect, Shape, Stroke, TextureHandle, TextureOptions, pos2, vec2};
use egui::epaint::Vertex;

const TAU: f32 = std::f32::consts::TAU;

/// The cloth's colors: (under the lamp, at the rail).
pub fn cloth(id: &str) -> (Color32, Color32) {
    match id {
        "felt_campfire" => (Color32::from_rgb(0xb2, 0x48, 0x22), Color32::from_rgb(0x3c, 0x0f, 0x08)),
        "felt_royal" => (Color32::from_rgb(0x2c, 0x6a, 0xc8), Color32::from_rgb(0x0a, 0x1d, 0x48)),
        "felt_crimson" => (Color32::from_rgb(0xb0, 0x1e, 0x36), Color32::from_rgb(0x3a, 0x05, 0x10)),
        "felt_obsidian" => (Color32::from_rgb(0x3c, 0x3e, 0x48), Color32::from_rgb(0x0b, 0x0b, 0x10)),
        // Classic green (everyone starts with it).
        _ => (Color32::from_rgb(0x1f, 0x9a, 0x5c), Color32::from_rgb(0x06, 0x36, 0x20)),
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// A small hash, for the cloth's grain.
fn hash(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    ((h ^ (h >> 16)) & 0xffff) as f32 / 65535.0
}

/// The cloth as a picture: lit in the middle, darker toward the rail, with a fine weave and
/// a printed spade.
pub fn cloth_image(id: &str) -> ColorImage {
    let (w, h) = (768usize, 400usize);
    let (lit, dark) = cloth(id);
    let mut px = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let u = x as f32 / (w - 1) as f32 * 2.0 - 1.0;
            let v = y as f32 / (h - 1) as f32 * 2.0 - 1.0;
            // The lamp is a little above the middle.
            let d = (u * u * 0.9 + (v + 0.18) * (v + 0.18) * 1.1).sqrt();
            let mut c = mix(lit, dark, smooth(d * 0.95));
            // Weave: fine diagonal threads, and grain.
            let thread = (((x + y) % 3) as f32 - 1.0) * 0.012 + (((x + 2 * h - y) % 4) as f32 - 1.5) * 0.008;
            let grain = (hash(x as u32, y as u32) - 0.5) * 0.07;
            let k = 1.0 + thread + grain;
            c = Color32::from_rgb((c.r() as f32 * k).clamp(0.0, 255.0) as u8, (c.g() as f32 * k).clamp(0.0, 255.0) as u8, (c.b() as f32 * k).clamp(0.0, 255.0) as u8);
            px.push(c);
        }
    }
    ColorImage::new([w, h], px)
}

pub fn cloth_texture(ctx: &egui::Context, id: &str) -> TextureHandle {
    ctx.load_texture(format!("poker-cloth-{id}"), cloth_image(id), TextureOptions::LINEAR)
}

/// A point on a rounded oval (a superellipse) at angle `t`.
pub fn on_oval(c: Pos2, a: f32, b: f32, t: f32) -> Pos2 {
    let (cs, sn) = (t.cos(), t.sin());
    let e = 2.0 / 3.2;
    c + vec2(a * cs.signum() * cs.abs().powf(e), b * sn.signum() * sn.abs().powf(e))
}

const STEPS: usize = 160;

/// The room round the table: dark, with warm lamplight falling on the table's middle.
pub fn room(p: &Painter, r: Rect, center: Pos2, warm: Color32) {
    p.rect_filled(r, 0, Color32::from_rgb(0x0b, 0x0a, 0x0c));
    let mut m = Mesh::default();
    let rings = [(0.0, warm), (0.45, mix(warm, Color32::from_rgb(0x0b, 0x0a, 0x0c), 0.55)), (1.0, Color32::from_rgb(0x0b, 0x0a, 0x0c))];
    let ra = r.width() * 0.62;
    let rb = r.height() * 0.66;
    m.colored_vertex(center, rings[0].1);
    for (k, col) in &rings[1..] {
        for i in 0..STEPS {
            let t = i as f32 / STEPS as f32 * TAU;
            m.colored_vertex(center + vec2(t.cos() * ra * k, t.sin() * rb * k), *col);
        }
    }
    let ring = |n: usize, i: usize| (1 + n * STEPS + i % STEPS) as u32;
    for i in 0..STEPS {
        m.add_triangle(0, ring(0, i), ring(0, i + 1));
        m.add_triangle(ring(0, i), ring(1, i), ring(1, i + 1));
        m.add_triangle(ring(0, i), ring(1, i + 1), ring(0, i + 1));
    }
    p.with_clip_rect(r).add(Shape::mesh(m));
}

/// A ring of the oval between two sizes, colored per point.
fn band(m: &mut Mesh, c: Pos2, (a0, b0): (f32, f32), (a1, b1): (f32, f32), col0: impl Fn(f32) -> Color32, col1: impl Fn(f32) -> Color32) {
    let base = m.vertices.len() as u32;
    for i in 0..STEPS {
        let t = i as f32 / STEPS as f32 * TAU;
        m.colored_vertex(on_oval(c, a0, b0, t), col0(t));
        m.colored_vertex(on_oval(c, a1, b1, t), col1(t));
    }
    for i in 0..STEPS as u32 {
        let j = (i + 1) % STEPS as u32;
        let (o0, i0, o1, i1) = (base + 2 * i, base + 2 * i + 1, base + 2 * j, base + 2 * j + 1);
        m.add_triangle(o0, i0, o1);
        m.add_triangle(i0, i1, o1);
    }
}

/// The table: shadow, rail, trim, cloth (`tex`), the line where bets go, and the room's mark.
pub fn table(p: &Painter, c: Pos2, a: f32, b: f32, tex: &TextureHandle, mark: &str, rules: &str) {
    let rail = (a * 0.075).clamp(22.0, 40.0);
    // A soft shadow on the floor.
    let mut m = Mesh::default();
    band(&mut m, c + vec2(0.0, 18.0), (a + rail + 34.0, b + rail + 38.0), (a + rail - 4.0, b + rail - 2.0), |_| Color32::TRANSPARENT, |_| Color32::from_black_alpha(170));
    p.add(Shape::mesh(m));
    let mut fill = Mesh::default();
    fill.colored_vertex(c + vec2(0.0, 18.0), Color32::from_black_alpha(170));
    let base = fill.vertices.len() as u32;
    for i in 0..STEPS {
        let t = i as f32 / STEPS as f32 * TAU;
        fill.colored_vertex(on_oval(c + vec2(0.0, 18.0), a + rail - 4.0, b + rail - 2.0, t), Color32::from_black_alpha(170));
    }
    for i in 0..STEPS as u32 {
        fill.add_triangle(0, base + i, base + (i + 1) % STEPS as u32);
    }
    p.add(Shape::mesh(fill));

    // The rail: padded leather, rounded in section (dark at both edges, a sheen along the
    // top), lit from above (the far side catches more light).
    let light = |t: f32| 0.55 + 0.45 * (-t.sin()).max(0.0) - 0.25 * t.sin().max(0.0);
    let leather = Color32::from_rgb(0x2a, 0x1c, 0x16);
    let sheen = Color32::from_rgb(0x7a, 0x5a, 0x48);
    let edge_dark = Color32::from_rgb(0x0c, 0x08, 0x07);
    let lit = |k: f32, t: f32| mix(edge_dark, mix(leather, sheen, k), light(t));
    let mut m = Mesh::default();
    let ring = |f: f32| (a + rail * f, b + rail * f);
    band(&mut m, c, ring(1.0), ring(0.82), |_| edge_dark, move |t| lit(0.25, t));
    band(&mut m, c, ring(0.82), ring(0.55), move |t| lit(0.25, t), move |t| lit(0.95, t));
    band(&mut m, c, ring(0.55), ring(0.2), move |t| lit(0.95, t), move |t| lit(0.35, t));
    band(&mut m, c, ring(0.2), ring(0.0), move |t| lit(0.35, t), |_| edge_dark);
    p.add(Shape::mesh(m));
    // Stitches along the rail.
    let stitch = Color32::from_rgba_unmultiplied(0xe8, 0xd2, 0xb0, 34);
    for i in 0..STEPS {
        if i % 2 == 0 {
            let t0 = i as f32 / STEPS as f32 * TAU;
            let t1 = (i as f32 + 0.6) / STEPS as f32 * TAU;
            p.line_segment([on_oval(c, a + rail * 0.86, b + rail * 0.86, t0), on_oval(c, a + rail * 0.86, b + rail * 0.86, t1)], Stroke::new(1.0f32, stitch));
        }
    }

    // Brass trim between the rail and the cloth.
    let mut m = Mesh::default();
    let brass_lo = Color32::from_rgb(0x6a, 0x4a, 0x1e);
    let brass_hi = Color32::from_rgb(0xe8, 0xc0, 0x6a);
    band(&mut m, c, (a + 1.0, b + 1.0), (a - 4.0, b - 4.0), move |t| mix(brass_lo, brass_hi, light(t) * 0.9), move |t| mix(brass_lo, brass_hi, light(t) * 0.5));
    p.add(Shape::mesh(m));

    // The cloth.
    let ca = a - 4.0;
    let cb = b - 4.0;
    let mut m = Mesh::with_texture(tex.id());
    m.vertices.push(Vertex { pos: c, uv: pos2(0.5, 0.5), color: Color32::WHITE });
    for i in 0..STEPS {
        let t = i as f32 / STEPS as f32 * TAU;
        let q = on_oval(c, ca, cb, t);
        let uv = pos2(0.5 + (q.x - c.x) / ca * 0.5, 0.5 + (q.y - c.y) / cb * 0.5);
        m.vertices.push(Vertex { pos: q, uv, color: Color32::WHITE });
    }
    for i in 0..STEPS as u32 {
        m.add_triangle(0, 1 + i, 1 + (i + 1) % STEPS as u32);
    }
    p.add(Shape::mesh(m));
    // The rail's shadow on the cloth.
    let mut m = Mesh::default();
    band(&mut m, c, (ca, cb), (ca - 26.0, cb - 22.0), |_| Color32::from_black_alpha(110), |_| Color32::TRANSPARENT);
    p.add(Shape::mesh(m));

    // The line where bets go, and the room's mark, printed on the cloth.
    let ink = Color32::from_white_alpha(30);
    let mut pts: Vec<Pos2> = (0..=STEPS).map(|i| on_oval(c, ca * 0.74, cb * 0.68, i as f32 / STEPS as f32 * TAU)).collect();
    pts.dedup();
    p.add(Shape::line(pts, Stroke::new(1.5f32, ink)));
    let spade = Rect::from_center_size(c + vec2(0.0, cb * 0.36), vec2(cb * 0.16, cb * 0.16));
    super::cards::suit(p, spade, 3, Color32::from_white_alpha(18));
    p.text(c + vec2(0.0, cb * 0.52), egui::Align2::CENTER_CENTER, mark, crate::theme::bold_font((cb * 0.085).clamp(12.0, 22.0)), Color32::from_white_alpha(26));
    p.text(c + vec2(0.0, cb * 0.62), egui::Align2::CENTER_CENTER, rules, crate::theme::font(12.0), Color32::from_white_alpha(40));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloth_is_lit_in_the_middle() {
        let img = cloth_image("");
        let at = |x: usize, y: usize| {
            let c = img.pixels[y * img.size[0] + x];
            c.r() as u32 + c.g() as u32 + c.b() as u32
        };
        let mid: u32 = (0..20).map(|i| at(380 + i, 160)).sum();
        let edge: u32 = (0..20).map(|i| at(5 + i, 200)).sum();
        assert!(mid > edge * 2, "{mid} {edge}");
    }
}
