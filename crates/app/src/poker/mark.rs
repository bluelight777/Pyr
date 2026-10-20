//! Pyr Poker's mark, printed on the cloth: a poker chip with Pyr's flame in its middle, inside a
//! casino's seal. (Never a spade, or any card suit.) Drawn once with tiny-skia (curves
//! and a cut-out egui's shapes can't fill), white, then tinted onto the table.

use eframe::egui::{self, Color32, ColorImage, Rect, TextureHandle, TextureOptions};
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, Stroke, Transform};

/// The mark (square, `SIZE` pixels), in white on nothing: drawn the first time it's needed.
pub fn texture(ctx: &egui::Context, seal: bool) -> Option<TextureHandle> {
    let name = if seal { "pyr-poker-mark" } else { "pyr-poker-chip" };
    let id = egui::Id::new(name);
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return Some(t);
    }
    let img = if seal { draw(SIZE)? } else { draw_chip(256)? };
    let t = ctx.load_texture(name, img, TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    Some(t)
}

const SIZE: u32 = 512;

/// The mark at `px` pixels square (its design is on a 200-unit square).
pub fn draw(px: u32) -> Option<ColorImage> {
    draw_with(px, px as f32 / 200.0, 0.0, true)
}

/// Just the chip, filling the square (an icon: Pyr Poker in the channel list and its header).
pub fn draw_chip(px: u32) -> Option<ColorImage> {
    let k = px as f32 / 126.0;
    draw_with(px, k, -37.0 * k, false)
}

fn draw_with(px: u32, k: f32, shift: f32, seal: bool) -> Option<ColorImage> {
    let mut pm = Pixmap::new(px, px)?;
    let t = Transform::from_scale(k, k).post_translate(shift, shift);
    let mut white = Paint::default();
    white.set_color_rgba8(255, 255, 255, 255);
    white.anti_alias = true;

    // A poker chip (never a card suit): its disc, the inlays round its edge and a ring inside,
    // cut out; and in its middle Pyr's flame, cut out too.
    let mut pb = PathBuilder::new();
    pb.push_circle(100.0, 100.0, 62.0);
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::TAU / 8.0 + std::f32::consts::FRAC_PI_8;
        let (w, r0, r1) = (0.2f32, 51.0f32, 62.5f32);
        let pt = |r: f32, a: f32| (100.0 + r * a.cos(), 100.0 + r * a.sin());
        let (x, y) = pt(r0, a - w * 1.15);
        pb.move_to(x, y);
        let (x, y) = pt(r1, a - w);
        pb.line_to(x, y);
        let (x, y) = pt(r1, a + w);
        pb.line_to(x, y);
        let (x, y) = pt(r0, a + w * 1.15);
        pb.line_to(x, y);
        pb.close();
    }
    pb.push_circle(100.0, 100.0, 45.0);
    pb.push_circle(100.0, 100.0, 41.5);
    // The flame (drawn on its own 200-unit sketch, made bigger and set in the middle).
    let f = |x: f32, y: f32| ((x - 100.0) * 1.45 + 100.0, (y - 90.0) * 1.45 + 100.0);
    let m = |pb: &mut PathBuilder, pts: [(f32, f32); 3]| {
        let [(a, b), (c, d), (e, g)] = pts.map(|(x, y)| f(x, y));
        pb.cubic_to(a, b, c, d, e, g);
    };
    let (x, y) = f(100.0, 66.0);
    pb.move_to(x, y);
    m(&mut pb, [(109.0, 78.0), (117.0, 88.0), (113.0, 101.0)]);
    m(&mut pb, [(111.0, 109.0), (105.0, 114.0), (100.0, 114.0)]);
    m(&mut pb, [(92.0, 114.0), (86.0, 107.0), (87.0, 99.0)]);
    m(&mut pb, [(88.0, 92.0), (93.0, 88.0), (95.0, 81.0)]);
    m(&mut pb, [(98.0, 87.0), (101.0, 90.0), (102.0, 94.0)]);
    m(&mut pb, [(105.0, 86.0), (104.0, 76.0), (100.0, 66.0)]);
    pb.close();
    pm.fill_path(&pb.finish()?, &white, FillRule::EvenOdd, t, None);

    // The seal: two rings, broken where a diamond sits at each side.
    if !seal {
        return Some(ColorImage::from_rgba_premultiplied([px as usize, px as usize], pm.data()));
    }
    for (r, w) in [(78.0f32, 2.4f32), (71.0, 0.9)] {
        for (from, to) in [(0.09f32, std::f32::consts::PI - 0.09), (std::f32::consts::PI + 0.09, std::f32::consts::TAU - 0.09)] {
            let mut pb = PathBuilder::new();
            let n = 64;
            for i in 0..=n {
                let a = from + (to - from) * i as f32 / n as f32;
                let (x, y) = (100.0 + r * a.cos(), 100.0 + r * a.sin());
                if i == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) }
            }
            let stroke = Stroke { width: w, line_cap: tiny_skia::LineCap::Round, ..Default::default() };
            pm.stroke_path(&pb.finish()?, &white, &stroke, t, None);
        }
    }
    for cx in [25.5f32, 174.5] {
        let mut pb = PathBuilder::new();
        pb.move_to(cx - 6.0, 100.0);
        pb.line_to(cx, 94.0);
        pb.line_to(cx + 6.0, 100.0);
        pb.line_to(cx, 106.0);
        pb.close();
        pm.fill_path(&pb.finish()?, &white, FillRule::Winding, t, None);
    }
    // (tiny-skia's pixels are premultiplied, as egui's are.)
    Some(ColorImage::from_rgba_premultiplied([px as usize, px as usize], pm.data()))
}

/// The mark on the cloth: centered at `c`, `side` across, in `tint`.
pub fn paint(p: &egui::Painter, c: egui::Pos2, side: f32, tint: Color32) {
    paint_as(p, c, side, tint, true);
}

/// Pyr Poker's icon (the chip and its flame, no seal): centered at `c`, `side` across.
pub fn icon(p: &egui::Painter, c: egui::Pos2, side: f32, tint: Color32) {
    paint_as(p, c, side, tint, false);
}

fn paint_as(p: &egui::Painter, c: egui::Pos2, side: f32, tint: Color32, seal: bool) {
    if let Some(tex) = texture(p.ctx(), seal) {
        let r = Rect::from_center_size(c, egui::vec2(side, side));
        p.image(tex.id(), r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), tint);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_mark_draws_with_its_flame_cut_out() {
        let img = super::draw(200).unwrap();
        let at = |x: usize, y: usize| img.pixels[y * 200 + x].a();
        assert!(at(100, 138) > 200, "the chip");
        assert!(at(100, 143) < 30, "the ring inside it is cut out");
        assert!(at(100, 122) < 30, "the flame in its middle is cut out");
        assert!(at(100, 155) > 200 && at(157, 100) > 200, "the chip's edge");
        assert!(at(100, 22) > 200, "the seal's ring");
        assert!(at(5, 5) == 0, "nothing outside");
    }
}
