//! The casino's pictures, drawn here (crisp at any size, nothing to ship): Pyr Sevens' symbols
//! (painted once with tiny-skia, gradients and all, then kept as textures), dice, and the small
//! pictures on the floor's game tiles.

use eframe::egui::{self, Align2, Color32, ColorImage, Pos2, Rect, Shape, Stroke, TextureHandle, TextureOptions, pos2, vec2};
use pyr_poker::casino::slots;
use tiny_skia::{Color, FillRule, GradientStop, LinearGradient, Paint, PathBuilder, Pixmap, Point, RadialGradient, SpreadMode, Stroke as SkStroke, Transform};

use super::col;
use crate::theme;

const PX: u32 = 192;

fn paint_solid(r: u8, g: u8, b: u8, a: u8) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(r, g, b, a);
    p.anti_alias = true;
    p
}

fn radial_paint(cx: f32, cy: f32, rad: f32, stops: &[(f32, [u8; 4])]) -> Paint<'static> {
    let mut p = Paint::default();
    p.anti_alias = true;
    let st = stops.iter().map(|(t, c)| GradientStop::new(*t, Color::from_rgba8(c[0], c[1], c[2], c[3]))).collect();
    if let Some(sh) = RadialGradient::new(Point::from_xy(cx - rad * 0.3, cy - rad * 0.35), Point::from_xy(cx, cy), rad, st, SpreadMode::Pad, Transform::identity()) {
        p.shader = sh;
    }
    p
}

fn linear_paint(x0: f32, y0: f32, x1: f32, y1: f32, stops: &[(f32, [u8; 4])]) -> Paint<'static> {
    let mut p = Paint::default();
    p.anti_alias = true;
    let st = stops.iter().map(|(t, c)| GradientStop::new(*t, Color::from_rgba8(c[0], c[1], c[2], c[3]))).collect();
    if let Some(sh) = LinearGradient::new(Point::from_xy(x0, y0), Point::from_xy(x1, y1), st, SpreadMode::Pad, Transform::identity()) {
        p.shader = sh;
    }
    p
}

fn circle(pm: &mut Pixmap, x: f32, y: f32, r: f32, paint: &Paint, t: Transform) {
    let mut pb = PathBuilder::new();
    pb.push_circle(x, y, r);
    if let Some(path) = pb.finish() {
        pm.fill_path(&path, paint, FillRule::Winding, t, None);
    }
}

fn oval(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, paint: &Paint, t: Transform) {
    let mut pb = PathBuilder::new();
    if let Some(r) = tiny_skia::Rect::from_xywh(x - w / 2.0, y - h / 2.0, w, h) {
        pb.push_oval(r);
    }
    if let Some(path) = pb.finish() {
        pm.fill_path(&path, paint, FillRule::Winding, t, None);
    }
}

fn poly(pm: &mut Pixmap, pts: &[(f32, f32)], paint: &Paint, t: Transform) {
    let mut pb = PathBuilder::new();
    for (i, (x, y)) in pts.iter().enumerate() {
        if i == 0 {
            pb.move_to(*x, *y);
        } else {
            pb.line_to(*x, *y);
        }
    }
    pb.close();
    if let Some(path) = pb.finish() {
        pm.fill_path(&path, paint, FillRule::Winding, t, None);
    }
}

fn stroke_path(pm: &mut Pixmap, pb: PathBuilder, paint: &Paint, width: f32, t: Transform) {
    if let Some(path) = pb.finish() {
        let s = SkStroke { width, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
        pm.stroke_path(&path, paint, &s, t, None);
    }
}

/// A symbol on a 200-unit square.
fn draw_symbol(sym: u8) -> Option<ColorImage> {
    let mut pm = Pixmap::new(PX, PX)?;
    let k = PX as f32 / 200.0;
    let t = Transform::from_scale(k, k);
    let white = paint_solid(255, 255, 255, 200);
    match sym {
        slots::CHERRY => {
            let stem = paint_solid(0x2e, 0x7d, 0x32, 255);
            let mut pb = PathBuilder::new();
            pb.move_to(72.0, 118.0);
            pb.quad_to(80.0, 70.0, 118.0, 38.0);
            pb.move_to(132.0, 124.0);
            pb.quad_to(128.0, 80.0, 118.0, 38.0);
            stroke_path(&mut pm, pb, &stem, 7.0, t);
            let leaf = linear_paint(118.0, 30.0, 165.0, 55.0, &[(0.0, [0x5c, 0xc0, 0x4a, 255]), (1.0, [0x1d, 0x6b, 0x2a, 255])]);
            let mut pb = PathBuilder::new();
            pb.move_to(118.0, 40.0);
            pb.quad_to(140.0, 18.0, 170.0, 40.0);
            pb.quad_to(145.0, 62.0, 118.0, 40.0);
            pb.close();
            if let Some(path) = pb.finish() {
                pm.fill_path(&path, &leaf, FillRule::Winding, t, None);
            }
            for (x, y) in [(68.0, 138.0), (134.0, 144.0)] {
                circle(&mut pm, x, y + 4.0, 38.0, &paint_solid(0, 0, 0, 70), t);
                circle(&mut pm, x, y, 38.0, &radial_paint(x, y, 46.0, &[(0.0, [0xff, 0x6b, 0x6b, 255]), (0.55, [0xd4, 0x14, 0x2a, 255]), (1.0, [0x6e, 0x05, 0x12, 255])]), t);
                oval(&mut pm, x - 12.0, y - 14.0, 16.0, 10.0, &white, t);
            }
        }
        slots::LEMON => {
            let body = radial_paint(100.0, 100.0, 85.0, &[(0.0, [0xff, 0xf6, 0x9a, 255]), (0.6, [0xf7, 0xd2, 0x1e, 255]), (1.0, [0xb8, 0x8a, 0x00, 255])]);
            let mut pb = PathBuilder::new();
            pb.move_to(22.0, 104.0);
            pb.cubic_to(30.0, 40.0, 170.0, 40.0, 178.0, 96.0);
            pb.cubic_to(170.0, 160.0, 30.0, 165.0, 22.0, 104.0);
            pb.close();
            if let Some(path) = pb.finish() {
                pm.fill_path(&path, &paint_solid(0, 0, 0, 60), FillRule::Winding, Transform::from_translate(0.0, 5.0).post_scale(k, k), None);
                pm.fill_path(&path, &body, FillRule::Winding, t, None);
            }
            oval(&mut pm, 72.0, 78.0, 40.0, 14.0, &white, t);
            circle(&mut pm, 20.0, 104.0, 7.0, &paint_solid(0xc9, 0x9a, 0x00, 255), t);
            circle(&mut pm, 180.0, 96.0, 7.0, &paint_solid(0xc9, 0x9a, 0x00, 255), t);
        }
        slots::PLUM => {
            circle(&mut pm, 100.0, 116.0, 64.0, &paint_solid(0, 0, 0, 70), t);
            circle(&mut pm, 100.0, 110.0, 64.0, &radial_paint(100.0, 110.0, 76.0, &[(0.0, [0xd6, 0x7a, 0xff, 255]), (0.55, [0x7b, 0x24, 0xb8, 255]), (1.0, [0x33, 0x08, 0x5a, 255])]), t);
            let mut pb = PathBuilder::new();
            pb.move_to(100.0, 52.0);
            pb.quad_to(118.0, 110.0, 100.0, 168.0);
            stroke_path(&mut pm, pb, &paint_solid(0x3d, 0x0c, 0x66, 140), 4.0, t);
            let leaf = linear_paint(100.0, 30.0, 150.0, 55.0, &[(0.0, [0x6c, 0xd0, 0x5a, 255]), (1.0, [0x1d, 0x6b, 0x2a, 255])]);
            let mut pb = PathBuilder::new();
            pb.move_to(100.0, 50.0);
            pb.quad_to(118.0, 18.0, 150.0, 28.0);
            pb.quad_to(132.0, 58.0, 100.0, 50.0);
            pb.close();
            if let Some(path) = pb.finish() {
                pm.fill_path(&path, &leaf, FillRule::Winding, t, None);
            }
            oval(&mut pm, 74.0, 84.0, 26.0, 14.0, &white, t);
        }
        slots::GRAPES => {
            let berries = [(80.0, 70.0), (120.0, 70.0), (60.0, 102.0), (100.0, 102.0), (140.0, 102.0), (80.0, 134.0), (120.0, 134.0), (100.0, 164.0)];
            for (x, y) in berries {
                circle(&mut pm, x, y + 4.0, 23.0, &paint_solid(0, 0, 0, 60), t);
            }
            for (x, y) in berries {
                circle(&mut pm, x, y, 23.0, &radial_paint(x, y, 28.0, &[(0.0, [0xd8, 0xff, 0x9a, 255]), (0.6, [0x6c, 0xbf, 0x2e, 255]), (1.0, [0x2a, 0x5e, 0x0e, 255])]), t);
                oval(&mut pm, x - 7.0, y - 8.0, 9.0, 6.0, &white, t);
            }
            let mut pb = PathBuilder::new();
            pb.move_to(100.0, 48.0);
            pb.quad_to(104.0, 30.0, 118.0, 18.0);
            stroke_path(&mut pm, pb, &paint_solid(0x6b, 0x45, 0x1e, 255), 7.0, t);
        }
        slots::BELL => {
            let gold = radial_paint(100.0, 95.0, 95.0, &[(0.0, [0xff, 0xf3, 0xb0, 255]), (0.5, [0xf2, 0xbf, 0x2a, 255]), (1.0, [0x8a, 0x5a, 0x00, 255])]);
            let mut pb = PathBuilder::new();
            pb.move_to(100.0, 22.0);
            pb.cubic_to(62.0, 24.0, 52.0, 70.0, 52.0, 110.0);
            pb.cubic_to(52.0, 136.0, 34.0, 146.0, 28.0, 156.0);
            pb.line_to(172.0, 156.0);
            pb.cubic_to(166.0, 146.0, 148.0, 136.0, 148.0, 110.0);
            pb.cubic_to(148.0, 70.0, 138.0, 24.0, 100.0, 22.0);
            pb.close();
            if let Some(path) = pb.finish() {
                pm.fill_path(&path, &paint_solid(0, 0, 0, 70), FillRule::Winding, Transform::from_translate(0.0, 5.0).post_scale(k, k), None);
                pm.fill_path(&path, &gold, FillRule::Winding, t, None);
            }
            oval(&mut pm, 100.0, 156.0, 150.0, 16.0, &linear_paint(25.0, 156.0, 175.0, 156.0, &[(0.0, [0x8a, 0x5a, 0x00, 255]), (0.5, [0xff, 0xe0, 0x6a, 255]), (1.0, [0x8a, 0x5a, 0x00, 255])]), t);
            circle(&mut pm, 100.0, 172.0, 15.0, &radial_paint(100.0, 172.0, 18.0, &[(0.0, [0xff, 0xf0, 0xa0, 255]), (1.0, [0x9a, 0x6a, 0x08, 255])]), t);
            oval(&mut pm, 80.0, 70.0, 14.0, 44.0, &paint_solid(255, 255, 255, 120), t);
        }
        slots::BAR => {
            let mut pb = PathBuilder::new();
            if let Some(r) = tiny_skia::Rect::from_xywh(14.0, 60.0, 172.0, 80.0) {
                pb.push_rect(r);
            }
            if let Some(path) = pb.finish() {
                pm.fill_path(&path, &paint_solid(0, 0, 0, 80), FillRule::Winding, Transform::from_translate(0.0, 5.0).post_scale(k, k), None);
                pm.fill_path(&path, &linear_paint(0.0, 60.0, 0.0, 140.0, &[(0.0, [0x3a, 0x3a, 0x44, 255]), (1.0, [0x0a, 0x0a, 0x10, 255])]), FillRule::Winding, t, None);
                let s = SkStroke { width: 7.0, ..Default::default() };
                pm.stroke_path(&path, &linear_paint(0.0, 60.0, 200.0, 140.0, &[(0.0, [0xff, 0xe3, 0x8a, 255]), (0.5, [0xc8, 0x8a, 0x1a, 255]), (1.0, [0xff, 0xe3, 0x8a, 255])]), &s, t, None);
            }
        }
        slots::DIAMOND => {
            let facets: [(&[(f32, f32)], [u8; 4]); 7] = [
                (&[(40.0, 70.0), (70.0, 38.0), (85.0, 70.0)], [0x9a, 0xe6, 0xff, 255]),
                (&[(70.0, 38.0), (130.0, 38.0), (115.0, 70.0), (85.0, 70.0)], [0xd0, 0xf6, 0xff, 255]),
                (&[(130.0, 38.0), (160.0, 70.0), (115.0, 70.0)], [0x6a, 0xc6, 0xf6, 255]),
                (&[(40.0, 70.0), (85.0, 70.0), (100.0, 172.0)], [0x2a, 0x86, 0xd8, 255]),
                (&[(85.0, 70.0), (115.0, 70.0), (100.0, 172.0)], [0x58, 0xb4, 0xf4, 255]),
                (&[(115.0, 70.0), (160.0, 70.0), (100.0, 172.0)], [0x18, 0x5c, 0xb0, 255]),
                (&[(40.0, 70.0), (160.0, 70.0), (160.0, 72.0), (40.0, 72.0)], [0xff, 0xff, 0xff, 200]),
            ];
            poly(&mut pm, &[(40.0, 75.0), (70.0, 43.0), (130.0, 43.0), (160.0, 75.0), (100.0, 177.0)], &paint_solid(0, 0, 0, 70), t);
            for (pts, c) in facets {
                poly(&mut pm, pts, &paint_solid(c[0], c[1], c[2], c[3]), t);
            }
            // A sparkle.
            poly(&mut pm, &[(132.0, 30.0), (136.0, 46.0), (152.0, 50.0), (136.0, 54.0), (132.0, 70.0), (128.0, 54.0), (112.0, 50.0), (128.0, 46.0)], &paint_solid(255, 255, 255, 230), t);
        }
        slots::SEVEN => {
            let seven: [(f32, f32); 8] = [(38.0, 30.0), (164.0, 30.0), (164.0, 56.0), (104.0, 172.0), (66.0, 172.0), (124.0, 64.0), (64.0, 64.0), (58.0, 80.0)];
            let shadow = Transform::from_translate(3.0, 6.0).post_scale(k, k);
            poly(&mut pm, &seven, &paint_solid(0, 0, 0, 90), shadow);
            let mut pb = PathBuilder::new();
            for (i, (x, y)) in seven.iter().enumerate() {
                if i == 0 {
                    pb.move_to(*x, *y);
                } else {
                    pb.line_to(*x, *y);
                }
            }
            pb.close();
            if let Some(path) = pb.finish() {
                pm.fill_path(&path, &linear_paint(0.0, 30.0, 0.0, 172.0, &[(0.0, [0xff, 0x6a, 0x5a, 255]), (0.5, [0xe0, 0x12, 0x24, 255]), (1.0, [0x8a, 0x02, 0x10, 255])]), FillRule::Winding, t, None);
                let s = SkStroke { width: 8.0, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
                pm.stroke_path(&path, &linear_paint(0.0, 30.0, 200.0, 172.0, &[(0.0, [0xff, 0xf0, 0xa8, 255]), (0.5, [0xd9, 0x9a, 0x1a, 255]), (1.0, [0xff, 0xe3, 0x8a, 255])]), &s, t, None);
            }
            poly(&mut pm, &[(46.0, 36.0), (156.0, 36.0), (156.0, 42.0), (46.0, 42.0)], &paint_solid(255, 255, 255, 140), t);
        }
        slots::WILD => {
            // Pyr's flame, glowing.
            circle(&mut pm, 100.0, 104.0, 92.0, &radial_paint(100.0, 110.0, 96.0, &[(0.0, [0xff, 0xb0, 0x40, 150]), (1.0, [0xff, 0x60, 0x10, 0])]), t);
            let mut pb = PathBuilder::new();
            pb.move_to(100.0, 14.0);
            pb.cubic_to(126.0, 52.0, 162.0, 76.0, 150.0, 128.0);
            pb.cubic_to(142.0, 162.0, 120.0, 178.0, 100.0, 178.0);
            pb.cubic_to(72.0, 178.0, 48.0, 158.0, 50.0, 124.0);
            pb.cubic_to(52.0, 98.0, 70.0, 84.0, 76.0, 62.0);
            pb.cubic_to(86.0, 80.0, 92.0, 90.0, 94.0, 104.0);
            pb.cubic_to(110.0, 82.0, 110.0, 48.0, 100.0, 14.0);
            pb.close();
            if let Some(path) = pb.finish() {
                pm.fill_path(&path, &linear_paint(0.0, 14.0, 0.0, 178.0, &[(0.0, [0xff, 0xf3, 0x9a, 255]), (0.45, [0xff, 0x9a, 0x1a, 255]), (1.0, [0xd4, 0x2a, 0x0a, 255])]), FillRule::Winding, t, None);
            }
            let mut pb = PathBuilder::new();
            pb.move_to(100.0, 92.0);
            pb.cubic_to(116.0, 112.0, 128.0, 130.0, 118.0, 154.0);
            pb.cubic_to(110.0, 170.0, 88.0, 170.0, 82.0, 152.0);
            pb.cubic_to(78.0, 136.0, 92.0, 124.0, 100.0, 92.0);
            pb.close();
            if let Some(path) = pb.finish() {
                pm.fill_path(&path, &paint_solid(0xff, 0xf8, 0xd0, 235), FillRule::Winding, t, None);
            }
        }
        _ => {
            // The star.
            circle(&mut pm, 100.0, 100.0, 94.0, &radial_paint(100.0, 100.0, 96.0, &[(0.0, [0xff, 0xe6, 0x7a, 140]), (1.0, [0xff, 0xb0, 0x20, 0])]), t);
            let mut pts = Vec::new();
            for i in 0..10 {
                let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
                let r = if i % 2 == 0 { 82.0 } else { 36.0 };
                pts.push((100.0 + a.cos() * r, 104.0 + a.sin() * r));
            }
            poly(&mut pm, &pts.iter().map(|(x, y)| (x + 3.0, y + 5.0)).collect::<Vec<_>>(), &paint_solid(0, 0, 0, 80), t);
            let mut pb = PathBuilder::new();
            for (i, (x, y)) in pts.iter().enumerate() {
                if i == 0 {
                    pb.move_to(*x, *y);
                } else {
                    pb.line_to(*x, *y);
                }
            }
            pb.close();
            if let Some(path) = pb.finish() {
                pm.fill_path(&path, &radial_paint(100.0, 100.0, 90.0, &[(0.0, [0xff, 0xfa, 0xd0, 255]), (0.5, [0xff, 0xc8, 0x2a, 255]), (1.0, [0xb0, 0x6a, 0x00, 255])]), FillRule::Winding, t, None);
                let s = SkStroke { width: 4.0, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
                pm.stroke_path(&path, &paint_solid(0xff, 0xf6, 0xc8, 255), &s, t, None);
            }
        }
    }
    Some(ColorImage::from_rgba_premultiplied([PX as usize, PX as usize], pm.data()))
}

/// A symbol's texture (drawn the first time it's needed).
pub fn symbol(ctx: &egui::Context, sym: u8) -> Option<TextureHandle> {
    let id = egui::Id::new(("pyr-slot-symbol", sym));
    if let Some(t) = ctx.data(|d| d.get_temp::<TextureHandle>(id)) {
        return Some(t);
    }
    let img = draw_symbol(sym)?;
    let t = ctx.load_texture(format!("slot-{sym}"), img, TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    Some(t)
}

/// A symbol in `r`, with the words the picture doesn't carry (BAR, WILD).
pub fn paint_symbol(p: &egui::Painter, r: Rect, sym: u8, tint: Color32) {
    if let Some(t) = symbol(p.ctx(), sym) {
        p.image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
    }
    let s = r.width();
    match sym {
        slots::BAR => {
            p.text(r.center() + vec2(0.0, 1.0), Align2::CENTER_CENTER, "BAR", theme::bold_font(s * 0.27), tint_mul(Color32::WHITE, tint));
        }
        slots::WILD => {
            let band = Rect::from_center_size(r.center() + vec2(0.0, s * 0.33), vec2(s * 0.7, s * 0.2));
            p.rect_filled(band, egui::CornerRadius::same((s * 0.06) as u8), tint_mul(Color32::from_rgb(0x6a, 0x0a, 0x10), tint));
            p.text(band.center(), Align2::CENTER_CENTER, "WILD", theme::bold_font(s * 0.15), tint_mul(col::GOLD2, tint));
        }
        _ => {}
    }
}

fn tint_mul(c: Color32, tint: Color32) -> Color32 {
    let f = |a: u8, b: u8| ((a as u16 * b as u16) / 255) as u8;
    Color32::from_rgba_unmultiplied(f(c.r(), tint.r()), f(c.g(), tint.g()), f(c.b(), tint.b()), f(c.a(), tint.a()))
}

/// A die showing `n` (1 to 6), turned `angle` radians, `size` across.
pub fn die(p: &egui::Painter, c: Pos2, size: f32, n: u8, angle: f32, color: Color32) {
    let h = size / 2.0;
    let rot = |v: egui::Vec2| {
        let (s, co) = angle.sin_cos();
        vec2(v.x * co - v.y * s, v.x * s + v.y * co)
    };
    // A rounded square, as a polygon.
    let mut pts = Vec::new();
    let rr = size * 0.18;
    for (cx, cy, a0) in [(h - rr, h - rr, 0.0f32), (-(h - rr), h - rr, 90.0), (-(h - rr), -(h - rr), 180.0), (h - rr, -(h - rr), 270.0)] {
        for k in 0..=6 {
            let a = (a0 + k as f32 * 15.0).to_radians();
            pts.push(c + rot(vec2(cx + a.cos() * rr, cy + a.sin() * rr)));
        }
    }
    let shadow: Vec<Pos2> = pts.iter().map(|q| *q + vec2(3.0, 5.0)).collect();
    p.add(Shape::convex_polygon(shadow, Color32::from_black_alpha(90), Stroke::NONE));
    p.add(Shape::convex_polygon(pts, color, Stroke::new(1.0f32, Color32::from_black_alpha(60))));
    let light = color.r() as u32 * 299 + color.g() as u32 * 587 + color.b() as u32 * 114 > 150_000;
    let ink = if light { Color32::from_rgb(0x1c, 0x18, 0x18) } else { Color32::from_rgb(0xfa, 0xf6, 0xee) };
    let pip = |x: f32, y: f32| p.circle_filled(c + rot(vec2(x, y) * size * 0.27), size * 0.085, ink);
    match n {
        1 => {
            p.circle_filled(c, size * 0.13, if light { col::RED } else { ink });
        }
        2 => {
            pip(-1.0, -1.0);
            pip(1.0, 1.0);
        }
        3 => {
            pip(-1.0, -1.0);
            pip(0.0, 0.0);
            pip(1.0, 1.0);
        }
        4 => {
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                pip(x, y);
            }
        }
        5 => {
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (0.0, 0.0), (-1.0, 1.0), (1.0, 1.0)] {
                pip(x, y);
            }
        }
        _ => {
            for (x, y) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 0.0), (1.0, 0.0), (-1.0, 1.0), (1.0, 1.0)] {
                pip(x, y);
            }
        }
    }
}

/// A keno ball with its number.
pub fn ball(p: &egui::Painter, c: Pos2, r: f32, n: u8, hit: bool) {
    let base = if hit { col::GOLD } else { Color32::from_rgb(0xe8, 0xe4, 0xdc) };
    p.circle_filled(c + vec2(0.0, r * 0.12), r, Color32::from_black_alpha(90));
    super::radial(p, c, r, Color32::WHITE, base);
    p.circle_filled(c, r * 0.58, Color32::WHITE);
    p.circle_stroke(c, r * 0.58, Stroke::new(1.5f32, if hit { col::RED } else { col::BLUE }));
    p.text(c, Align2::CENTER_CENTER, n.to_string(), theme::bold_font(r * 0.66), Color32::from_rgb(0x1a, 0x16, 0x14));
}

/// The small picture on a floor tile, for each game (and poker).
pub fn tile(p: &egui::Painter, r: Rect, which: &str, t: f32) {
    let c = r.center();
    let s = r.height();
    match which {
        "slots" => {
            for (i, sym) in [slots::SEVEN, slots::WILD, slots::SEVEN].iter().enumerate() {
                let cell = Rect::from_center_size(c + vec2((i as f32 - 1.0) * s * 0.36, 0.0), vec2(s * 0.34, s * 0.5));
                p.rect_filled(cell, egui::CornerRadius::same(6), Color32::from_rgb(0xfb, 0xf6, 0xea));
                let bob = if i == 1 { (t * 3.0).sin() * 2.0 } else { 0.0 };
                paint_symbol(p, Rect::from_center_size(cell.center() + vec2(0.0, bob), vec2(s * 0.3, s * 0.3)), *sym, Color32::WHITE);
            }
        }
        "blackjack" => {
            let ace = pyr_poker::cards::Card::new(12, 3);
            let king = pyr_poker::cards::Card::new(11, 2);
            let cw = s * 0.42;
            crate::poker::cards::face(p, Rect::from_center_size(c + vec2(-cw * 0.32, 4.0), vec2(cw, cw * 1.4)), king, false, false);
            crate::poker::cards::face(p, Rect::from_center_size(c + vec2(cw * 0.32, -4.0), vec2(cw, cw * 1.4)), ace, false, false);
        }
        "roulette" => {
            let rad = s * 0.36;
            p.circle_filled(c, rad + 6.0, Color32::from_rgb(0x5a, 0x34, 0x1a));
            let n = 37;
            let spin = t * 0.6;
            for i in 0..n {
                let a0 = spin + i as f32 / n as f32 * std::f32::consts::TAU;
                let a1 = spin + (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
                let num = pyr_poker::casino::roulette::WHEEL[i];
                let fill = if num == 0 { col::GREEN } else if pyr_poker::casino::roulette::is_red(num) { col::RED } else { Color32::from_rgb(0x16, 0x16, 0x1a) };
                p.add(Shape::convex_polygon(vec![c + vec2(a0.cos(), a0.sin()) * rad * 0.55, c + vec2(a0.cos(), a0.sin()) * rad, c + vec2(a1.cos(), a1.sin()) * rad, c + vec2(a1.cos(), a1.sin()) * rad * 0.55], fill, Stroke::NONE));
            }
            p.circle_filled(c, rad * 0.55, Color32::from_rgb(0x7a, 0x4a, 0x22));
            p.circle_filled(c, rad * 0.18, col::GOLD);
            let ba = -t * 1.4;
            p.circle_filled(c + vec2(ba.cos(), ba.sin()) * rad * 0.78, s * 0.035, Color32::WHITE);
        }
        "baccarat" => {
            let nine = pyr_poker::cards::Card::new(7, 1);
            let cw = s * 0.36;
            crate::poker::cards::face(p, Rect::from_center_size(c + vec2(-cw * 0.62, 0.0), vec2(cw, cw * 1.4)), nine, false, false);
            crate::poker::cards::back(p, Rect::from_center_size(c + vec2(cw * 0.62, 0.0), vec2(cw, cw * 1.4)), "");
            p.text(c + vec2(0.0, s * 0.38), Align2::CENTER_CENTER, "PLAYER  ·  BANKER", theme::bold_font(10.0), col::GOLD2);
        }
        "craps" => {
            die(p, c + vec2(-s * 0.18, 2.0), s * 0.36, 5, -0.3 + (t * 0.7).sin() * 0.05, Color32::from_rgb(0xd8, 0x1f, 0x2e));
            die(p, c + vec2(s * 0.2, -4.0), s * 0.36, 2, 0.4, Color32::from_rgb(0xd8, 0x1f, 0x2e));
        }
        "dice" => {
            let bar = Rect::from_center_size(c + vec2(0.0, s * 0.12), vec2(r.width() * 0.62, 10.0));
            p.rect_filled(Rect::from_min_max(bar.min, pos2(bar.left() + bar.width() * 0.495, bar.bottom())), egui::CornerRadius::same(5), col::GREEN);
            p.rect_filled(Rect::from_min_max(pos2(bar.left() + bar.width() * 0.495, bar.top()), bar.max), egui::CornerRadius::same(5), col::RED);
            let x = bar.left() + bar.width() * (0.5 + 0.35 * (t * 0.8).sin());
            p.circle_filled(pos2(x, bar.center().y), 9.0, Color32::WHITE);
            p.text(c - vec2(0.0, s * 0.16), Align2::CENTER_CENTER, format!("{:.2}", 50.0 + 35.0 * (t * 0.8).sin()), theme::bold_font(s * 0.22), Color32::WHITE);
        }
        "timebomb" => {
            crate::poker::bomb::bomb_icon(p, c + vec2(-s * 0.06, s * 0.06), s * 0.3, t, true, 0.5 + 0.5 * (t * 0.3).sin().abs());
            p.text(c + vec2(s * 0.56, s * 0.22), Align2::CENTER_CENTER, "1000×", theme::bold_font(s * 0.16), col::GOLD2);
        }
        "keno" => {
            for (i, n) in [7u8, 21, 42, 63, 80].iter().enumerate() {
                let bob = ((t * 2.0 + i as f32).sin()) * 2.0;
                ball(p, c + vec2((i as f32 - 2.0) * s * 0.2, bob + if i % 2 == 0 { -6.0 } else { 6.0 }), s * 0.11, *n, i == 2);
            }
        }
        _ => {
            // Poker: two cards and a stack of chips.
            let a = pyr_poker::cards::Card::new(12, 2);
            let k = pyr_poker::cards::Card::new(12, 1);
            let cw = s * 0.36;
            crate::poker::cards::face(p, Rect::from_center_size(c + vec2(-cw * 0.5, -2.0), vec2(cw, cw * 1.4)), a, false, false);
            crate::poker::cards::face(p, Rect::from_center_size(c + vec2(cw * 0.1, 4.0), vec2(cw, cw * 1.4)), k, false, false);
            super::chip_stack(p, c + vec2(cw * 0.95, s * 0.22), s * 0.1, 1_500);
        }
    }
}
