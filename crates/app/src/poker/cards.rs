//! Cards, chips and card backs, drawn (no pictures: crisp at any size, and the suits don't
//! depend on a font having them).

use eframe::egui::{self, Color32, CornerRadius, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_poker::cards::Card;

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
    let c = r.center();
    let s = r.width().min(r.height());
    match suit {
        // Hearts: two circles and a point.
        2 => {
            let rad = s * 0.25;
            p.circle_filled(c + vec2(-s * 0.22, -s * 0.12), rad, color);
            p.circle_filled(c + vec2(s * 0.22, -s * 0.12), rad, color);
            p.add(Shape::convex_polygon(
                vec![c + vec2(-s * 0.46, -s * 0.04), c + vec2(s * 0.46, -s * 0.04), c + vec2(0.0, s * 0.46)],
                color,
                Stroke::NONE,
            ));
        }
        // Diamonds.
        1 => {
            p.add(Shape::convex_polygon(vec![c + vec2(0.0, -s * 0.5), c + vec2(s * 0.38, 0.0), c + vec2(0.0, s * 0.5), c + vec2(-s * 0.38, 0.0)], color, Stroke::NONE));
        }
        // Spades: an upside-down heart and a stem.
        3 => {
            let rad = s * 0.24;
            p.circle_filled(c + vec2(-s * 0.21, s * 0.08), rad, color);
            p.circle_filled(c + vec2(s * 0.21, s * 0.08), rad, color);
            p.add(Shape::convex_polygon(vec![c + vec2(0.0, -s * 0.48), c + vec2(s * 0.44, s * 0.02), c + vec2(-s * 0.44, s * 0.02)], color, Stroke::NONE));
            stem(p, c, s, color);
        }
        // Clubs: three circles and a stem.
        _ => {
            let rad = s * 0.2;
            p.circle_filled(c + vec2(0.0, -s * 0.22), rad, color);
            p.circle_filled(c + vec2(-s * 0.22, s * 0.08), rad, color);
            p.circle_filled(c + vec2(s * 0.22, s * 0.08), rad, color);
            p.circle_filled(c + vec2(0.0, s * 0.02), rad * 0.6, color);
            stem(p, c, s, color);
        }
    }
}

fn stem(p: &Painter, c: Pos2, s: f32, color: Color32) {
    p.add(Shape::convex_polygon(vec![c + vec2(-s * 0.06, s * 0.1), c + vec2(s * 0.06, s * 0.1), c + vec2(s * 0.16, s * 0.48), c + vec2(-s * 0.16, s * 0.48)], color, Stroke::NONE));
}

fn rank_text(rank: u8) -> &'static str {
    ["2", "3", "4", "5", "6", "7", "8", "9", "10", "J", "Q", "K", "A"][rank as usize]
}

/// A card face up in `r` (about 5:7). `dim`: not part of the winning hand, or folded.
pub fn face(p: &Painter, r: Rect, card: Card, four: bool, dim: bool) {
    let round = CornerRadius::same((r.width() * 0.09).round() as u8);
    // A soft shadow, the card, a fine edge.
    p.rect_filled(r.translate(vec2(0.0, r.height() * 0.03)), round, Color32::from_black_alpha(70));
    p.rect_filled(r, round, Color32::from_rgb(0xfb, 0xfa, 0xf6));
    p.rect_stroke(r, round, Stroke::new(1.0f32, Color32::from_rgb(0xd9, 0xd5, 0xcc)), StrokeKind::Inside);
    let col = suit_color(card.suit(), four);
    let w = r.width();
    // The rank top left, its suit under it, and a big suit in the middle.
    let font = egui::FontId::new(w * 0.36, crate::theme::bold());
    p.text(r.left_top() + vec2(w * 0.1, w * 0.04), egui::Align2::LEFT_TOP, rank_text(card.rank()), font, col);
    let small = Rect::from_min_size(r.left_top() + vec2(w * 0.11, w * 0.44), vec2(w * 0.24, w * 0.24));
    suit(p, small, card.suit(), col);
    let big = Rect::from_center_size(r.center() + vec2(w * 0.1, w * 0.18), vec2(w * 0.52, w * 0.52));
    suit(p, big, card.suit(), col);
    if dim {
        p.rect_filled(r, round, Color32::from_black_alpha(120));
    }
}

/// A card face down, in the back the player chose.
pub fn back(p: &Painter, r: Rect, design: &str) {
    let round = CornerRadius::same((r.width() * 0.09).round() as u8);
    let (a, b, line) = match design {
        "back_midnight" => (Color32::from_rgb(0x15, 0x1d, 0x3b), Color32::from_rgb(0x2a, 0x3a, 0x7a), Color32::from_rgb(0x9f, 0xb4, 0xff)),
        "back_jade" => (Color32::from_rgb(0x0c, 0x3b, 0x2e), Color32::from_rgb(0x17, 0x6b, 0x52), Color32::from_rgb(0x9b, 0xf0, 0xc8)),
        "back_gold" => (Color32::from_rgb(0x3a, 0x2a, 0x0a), Color32::from_rgb(0x8a, 0x66, 0x18), Color32::from_rgb(0xff, 0xdb, 0x7a)),
        _ => (Color32::from_rgb(0x5a, 0x1c, 0x10), Color32::from_rgb(0xb4, 0x45, 0x1f), Color32::from_rgb(0xff, 0xb3, 0x6b)),
    };
    p.rect_filled(r.translate(vec2(0.0, r.height() * 0.03)), round, Color32::from_black_alpha(70));
    p.rect_filled(r, round, Color32::from_rgb(0xf4, 0xf1, 0xea));
    let inner = r.shrink(r.width() * 0.07);
    p.rect_filled(inner, CornerRadius::same((r.width() * 0.06) as u8), a);
    // A lattice of diamonds, and a flame in the middle.
    let step = inner.width() / 4.0;
    let clip = p.with_clip_rect(inner);
    let mut y = inner.top() - step;
    let mut row = 0;
    while y < inner.bottom() + step {
        let mut x = inner.left() - step + if row % 2 == 0 { 0.0 } else { step / 2.0 };
        while x < inner.right() + step {
            let c = pos2(x, y);
            clip.add(Shape::convex_polygon(vec![c + vec2(0.0, -step * 0.35), c + vec2(step * 0.25, 0.0), c + vec2(0.0, step * 0.35), c + vec2(-step * 0.25, 0.0)], b, Stroke::NONE));
            x += step;
        }
        y += step * 0.5;
        row += 1;
    }
    let mid = Rect::from_center_size(inner.center(), vec2(inner.width() * 0.5, inner.width() * 0.5));
    p.circle_filled(mid.center(), mid.width() * 0.5, a);
    p.circle_stroke(mid.center(), mid.width() * 0.5, Stroke::new(1.5f32, line));
    crate::icons::flame(p, mid.shrink(mid.width() * 0.2));
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
