//! Small vector icons drawn with the painter (crisp at any DPI, no icon font needed).

use std::f32::consts::{PI, TAU};

use eframe::egui::{Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};

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

pub fn flame(p: &Painter, r: Rect) {
    let s = r.width();
    let ctr = r.center();
    let mut outer = Vec::new();
    for i in 0..=40 {
        let t = i as f32 / 40.0 * TAU;
        let x = (t).sin() * s * 0.30 * (1.0 - 0.35 * (t.cos() * 0.5 + 0.5));
        let y = -(t.cos()) * s * 0.36 + s * 0.06;
        let tip = if t.cos() > 0.0 { t.cos().powf(3.0) * s * 0.12 } else { 0.0 };
        outer.push(ctr + vec2(x, y - tip));
    }
    p.add(Shape::convex_polygon(outer, crate::theme::EMBER, Stroke::NONE));
    p.circle_filled(ctr + vec2(0.0, s * 0.16), s * 0.13, Color32::from_rgb(0xff, 0xd2, 0x7a));
    p.line_segment([ctr + vec2(-s * 0.3, s * 0.40), ctr + vec2(s * 0.3, s * 0.40)], Stroke::new(s * 0.08, Color32::from_rgb(0x8a, 0x5a, 0x3c)));
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
        let k = if exit { corner + eframe::egui::vec2(dx * l, dy * l) } else { corner };
        let (sx, sy) = if exit { (-dx, -dy) } else { (dx, dy) };
        p.line_segment([k, k + eframe::egui::vec2(sx * l, 0.0)], s);
        p.line_segment([k, k + eframe::egui::vec2(0.0, sy * l)], s);
    }
}
