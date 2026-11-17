//! Pyr's look (Campfire's colors, Inter), the art, and the pieces every page is drawn with.

use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Pos2, Rect, RichText, Shape, Stroke, TextStyle, Visuals, pos2, vec2};

pub const MAIN: Color32 = Color32::from_rgb(0x26, 0x20, 0x1c);
pub const SIDE: Color32 = Color32::from_rgb(0x1e, 0x19, 0x16);
pub const ELEV: Color32 = Color32::from_rgb(0x2f, 0x28, 0x23);
pub const ELEV2: Color32 = Color32::from_rgb(0x3a, 0x31, 0x2b);
pub const LINE: Color32 = Color32::from_rgb(0x3b, 0x32, 0x2c);
pub const TEXT: Color32 = Color32::from_rgb(0xee, 0xe6, 0xdf);
pub const BODY: Color32 = Color32::from_rgb(0xe6, 0xdd, 0xd5);
pub const MUTED: Color32 = Color32::from_rgb(0xa8, 0x99, 0x8c);
pub const DIM: Color32 = Color32::from_rgb(0x7d, 0x70, 0x67);
pub const EMBER: Color32 = Color32::from_rgb(0xff, 0x7a, 0x3d);
pub const EMBER2: Color32 = Color32::from_rgb(0xff, 0xb3, 0x47);
pub const LIVE: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);
pub const CODE_BG: Color32 = Color32::from_rgb(0x1a, 0x15, 0x12);
pub const SELECTION: Color32 = Color32::from_rgb(0x8a, 0x40, 0x1c);
pub const ON_EMBER: Color32 = Color32::from_rgb(0x1b, 0x14, 0x11);
pub const GOLD: Color32 = Color32::from_rgb(0xff, 0xd3, 0x7a);
pub const UP: Color32 = Color32::from_rgb(0x3d, 0xd6, 0x8c);
pub const DOWN: Color32 = Color32::from_rgb(0xf0, 0x6a, 0x6a);
/// A card's own dark, and its edge.
pub const CARD: Color32 = Color32::from_rgb(0x1b, 0x15, 0x12);
pub const CARD_EDGE: Color32 = Color32::from_rgb(0x4a, 0x38, 0x24);

/// The outcomes' colors, in order (as the jeweled chips: emerald, ruby, sapphire, amber,
/// amethyst, pearl).
pub const COLORS: [Color32; 6] = [
    Color32::from_rgb(0x2e, 0xc2, 0x6e),
    Color32::from_rgb(0xe5, 0x48, 0x4d),
    Color32::from_rgb(0x5a, 0x8f, 0xf5),
    Color32::from_rgb(0xf0, 0xa0, 0x30),
    Color32::from_rgb(0xb0, 0x70, 0xf5),
    Color32::from_rgb(0xe8, 0xe2, 0xd6),
];

/// The leagues: their props come a game at a time, and close as it starts.
pub const SPORTS: [&str; 6] = ["nfl", "nba", "nhl", "mlb", "cfb", "cbb"];

/// Which of the six colors (and jeweled chips) an outcome wears. A yes and a no (an up and a
/// down, an over and an under) wear green and red; anything else (teams, ranges) wears colors
/// that don't say good or bad, so no team is "the red one".
pub fn hue(outcomes: &[String], i: usize) -> usize {
    let means = |s: &str| match s.split_whitespace().next().unwrap_or("").to_lowercase().as_str() {
        "yes" | "up" | "over" => Some(0),
        "no" | "down" | "under" => Some(1),
        _ => None,
    };
    if let [a, b] = outcomes {
        if let (Some(x), Some(y)) = (means(a), means(b)) {
            if x != y {
                return if i == 0 { x } else { y };
            }
        }
    }
    [2, 3, 4, 5, 0, 1][i % 6]
}

/// An outcome's color.
pub fn color_of(outcomes: &[String], i: usize) -> Color32 {
    COLORS[hue(outcomes, i)]
}

const CHIPS: [&[u8]; 6] = [
    include_bytes!("../../app/assets/props/chip-emerald.webp"),
    include_bytes!("../../app/assets/props/chip-ruby.webp"),
    include_bytes!("../../app/assets/props/chip-sapphire.webp"),
    include_bytes!("../../app/assets/props/chip-amber.webp"),
    include_bytes!("../../app/assets/props/chip-amethyst.webp"),
    include_bytes!("../../app/assets/props/chip-pearl.webp"),
];
const BANNER: &[u8] = include_bytes!("../../app/assets/props/banner.webp");
const CARD_BG: &[u8] = include_bytes!("../../app/assets/props/card-bg.webp");
pub const EMBLEM: &[u8] = include_bytes!("../../app/assets/props/emblem.webp");
const WON: &[u8] = include_bytes!("../../app/assets/props/won.webp");
const LOCK: &[u8] = include_bytes!("../../app/assets/props/lock.webp");

pub struct Art {
    pub chips: Vec<egui::TextureHandle>,
    pub banner: Option<egui::TextureHandle>,
    pub card_bg: Option<egui::TextureHandle>,
    pub emblem: Option<egui::TextureHandle>,
    pub won: Option<egui::TextureHandle>,
    pub lock: Option<egui::TextureHandle>,
}

impl Art {
    pub fn load(ctx: &egui::Context) -> Art {
        let t = |name: &str, bytes: &[u8]| -> Option<egui::TextureHandle> {
            let img = image::load_from_memory(bytes).ok()?.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            Some(ctx.load_texture(name, egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), egui::TextureOptions::LINEAR))
        };
        Art {
            chips: CHIPS.iter().enumerate().filter_map(|(i, b)| t(&format!("chip-{i}"), b)).collect(),
            banner: t("banner", BANNER),
            card_bg: t("card-bg", CARD_BG),
            emblem: t("emblem", EMBLEM),
            won: t("won", WON),
            lock: t("lock", LOCK),
        }
    }
}

/// The part of a picture that fills `r` (cut to `r`'s shape about `focus`, like a cover).
fn cover_uv(size: [usize; 2], r: Rect, focus: (f32, f32)) -> Rect {
    let (iw, ih) = (size[0] as f32, size[1] as f32);
    let (want, have) = (r.width() / r.height().max(1.0), iw / ih.max(1.0));
    if have > want {
        let k = want / have;
        let x = ((focus.0 - k / 2.0).max(0.0)).min(1.0 - k);
        Rect::from_min_max(pos2(x, 0.0), pos2(x + k, 1.0))
    } else {
        let k = have / want;
        let y = ((focus.1 - k / 2.0).max(0.0)).min(1.0 - k);
        Rect::from_min_max(pos2(0.0, y), pos2(1.0, y + k))
    }
}

/// A picture over `r` as a cover, tinted.
pub fn cover(p: &egui::Painter, t: Option<&egui::TextureHandle>, r: Rect, tint: Color32) {
    if let Some(t) = t {
        p.image(t.id(), r, cover_uv(t.size(), r, (0.5, 0.5)), tint);
    }
}

/// A picture over `r` as a cover about `focus`, its corners rounded.
pub fn cover_rounded(t: &egui::TextureHandle, r: Rect, radius: f32, focus: (f32, f32), tint: Color32) -> Shape {
    let uv = cover_uv(t.size(), r, focus);
    let rad = radius.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    let to_uv = |q: Pos2| pos2(uv.left() + (q.x - r.left()) / r.width().max(1.0) * uv.width(), uv.top() + (q.y - r.top()) / r.height().max(1.0) * uv.height());
    let mut m = egui::Mesh::with_texture(t.id());
    let c = r.center();
    m.vertices.push(egui::epaint::Vertex { pos: c, uv: to_uv(c), color: tint });
    let seg = 8;
    let corners = [(pos2(r.right() - rad, r.bottom() - rad), 0.0f32), (pos2(r.left() + rad, r.bottom() - rad), 90.0), (pos2(r.left() + rad, r.top() + rad), 180.0), (pos2(r.right() - rad, r.top() + rad), 270.0)];
    for (cc, a0) in corners {
        for i in 0..=seg {
            let a = (a0 + 90.0 * i as f32 / seg as f32).to_radians();
            let q = cc + vec2(a.cos(), a.sin()) * rad;
            m.vertices.push(egui::epaint::Vertex { pos: q, uv: to_uv(q), color: tint });
        }
    }
    let n = 4 * (seg + 1) as u32;
    for i in 1..=n {
        m.add_triangle(0, i, if i == n { 1 } else { i + 1 });
    }
    Shape::mesh(m)
}

/// A rounded box whose color goes from one to another: `how_far` says how far along (0 to 1)
/// a point of it is.
fn gradient_by(r: Rect, radius: f32, from: Color32, to: Color32, how_far: impl Fn(Pos2) -> f32) -> Shape {
    let rad = radius.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    let mix = |q: Pos2| {
        let t = how_far(q).clamp(0.0, 1.0);
        let f = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
        Color32::from_rgba_premultiplied(f(from.r(), to.r()), f(from.g(), to.g()), f(from.b(), to.b()), f(from.a(), to.a()))
    };
    let mut m = egui::Mesh::default();
    let c = r.center();
    m.vertices.push(egui::epaint::Vertex { pos: c, uv: egui::epaint::WHITE_UV, color: mix(c) });
    let seg = 6;
    let corners = [(pos2(r.right() - rad, r.bottom() - rad), 0.0f32), (pos2(r.left() + rad, r.bottom() - rad), 90.0), (pos2(r.left() + rad, r.top() + rad), 180.0), (pos2(r.right() - rad, r.top() + rad), 270.0)];
    for (cc, a0) in corners {
        for i in 0..=seg {
            let a = (a0 + 90.0 * i as f32 / seg as f32).to_radians();
            let q = cc + vec2(a.cos(), a.sin()) * rad;
            m.vertices.push(egui::epaint::Vertex { pos: q, uv: egui::epaint::WHITE_UV, color: mix(q) });
        }
    }
    let n = 4 * (seg + 1) as u32;
    for i in 1..=n {
        m.add_triangle(0, i, if i == n { 1 } else { i + 1 });
    }
    Shape::mesh(m)
}

/// A rounded box that goes from one color at the left to another at the right.
pub fn gradient(r: Rect, radius: f32, left: Color32, right: Color32) -> Shape {
    gradient_by(r, radius, left, right, |q| (q.x - r.left()) / r.width().max(1.0))
}

/// A rounded box that goes from one color at its top-left corner to another by its middle (and
/// stays that to the bottom-right): a card lit from its corner.
pub fn gradient_corner(r: Rect, radius: f32, corner: Color32, rest: Color32) -> Shape {
    gradient_by(r, radius, corner, rest, |q| ((q.x - r.left()) / r.width().max(1.0) + (q.y - r.top()) / r.height().max(1.0)) * 1.1)
}

pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn bold_font(size: f32) -> FontId {
    FontId::new(size, bold())
}

pub fn primary_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text.to_string()).color(ON_EMBER).font(bold_font(14.0))).fill(EMBER).corner_radius(6)
}

/// A color between two.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// Pyr's fonts and widgets.
pub fn style(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert("inter".into(), Arc::new(FontData::from_static(include_bytes!("../../app/assets/fonts/Inter-Regular.otf"))));
    fonts.font_data.insert("inter-semibold".into(), Arc::new(FontData::from_static(include_bytes!("../../app/assets/fonts/Inter-SemiBold.otf"))));
    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut body = vec!["inter".to_string()];
    body.extend(fallbacks.iter().cloned());
    let mut b = vec!["inter-semibold".to_string()];
    b.extend(fallbacks);
    fonts.families.insert(FontFamily::Proportional, body);
    fonts.families.insert(bold(), b);
    ctx.set_fonts(fonts);
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, FontId::proportional(11.5)),
            (TextStyle::Body, FontId::proportional(14.5)),
            (TextStyle::Button, FontId::proportional(14.0)),
            (TextStyle::Heading, bold_font(19.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        s.spacing.item_spacing = vec2(8.0, 6.0);
        s.spacing.button_padding = vec2(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.menu_margin = egui::Margin::same(6);
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.scroll.floating = true;
        let mut v = Visuals::dark();
        v.panel_fill = MAIN;
        v.window_fill = SIDE;
        v.window_stroke = Stroke::new(1.0f32, LINE);
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(10);
        v.popup_shadow = egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(150) };
        v.extreme_bg_color = CODE_BG;
        v.faint_bg_color = ELEV;
        v.selection.bg_fill = SELECTION;
        v.selection.stroke = Stroke::new(1.0f32, EMBER2);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = SIDE;
        w.noninteractive.weak_bg_fill = SIDE;
        w.noninteractive.bg_stroke = Stroke::new(1.0f32, LINE);
        w.noninteractive.fg_stroke = Stroke::new(1.0f32, BODY);
        w.inactive.bg_fill = ELEV;
        w.inactive.weak_bg_fill = ELEV;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0f32, TEXT);
        w.inactive.corner_radius = CornerRadius::same(6);
        w.hovered.bg_fill = ELEV2;
        w.hovered.weak_bg_fill = ELEV2;
        w.hovered.bg_stroke = Stroke::new(1.0f32, Color32::from_rgb(0x55, 0x48, 0x3f));
        w.hovered.fg_stroke = Stroke::new(1.5f32, TEXT);
        w.hovered.corner_radius = CornerRadius::same(6);
        let active = Color32::from_rgb(0x4a, 0x3a, 0x30);
        w.active.bg_fill = active;
        w.active.weak_bg_fill = active;
        w.active.fg_stroke = Stroke::new(1.5f32, EMBER2);
        w.active.corner_radius = CornerRadius::same(6);
        w.open.bg_fill = ELEV2;
        w.open.weak_bg_fill = ELEV2;
        s.visuals = v;
    });
}

// ---- the categories' looks

/// A category's color and a word about it.
pub fn category_look(id: &str) -> (Color32, &'static str) {
    match id {
        "nfl" => (Color32::from_rgb(0x4c, 0xa3, 0x55), "Every game: the winner, the spread, the total"),
        "nba" => (Color32::from_rgb(0xf0, 0x86, 0x3a), "Tonight's games and tomorrow's"),
        "nhl" => (Color32::from_rgb(0x58, 0xb4, 0xe8), "Winners, totals, and overtime"),
        "mlb" => (Color32::from_rgb(0xe0, 0x56, 0x4f), "Winners, totals, and the first inning"),
        "cfb" => (Color32::from_rgb(0xc9, 0x9a, 0x45), "The Top 25's games"),
        "cbb" => (Color32::from_rgb(0xe8, 0x6f, 0x51), "The Top 25's games"),
        "topical" => (Color32::from_rgb(0x4f, 0xc4, 0xb8), "The world's numbers: rates, money, the debt"),
        "casino" => (Color32::from_rgb(0xf2, 0xc1, 0x4e), "This server's own casino"),
        "crypto" => (Color32::from_rgb(0xf7, 0x93, 0x1a), "Bitcoin, Ether and the rest, at midnight UTC"),
        "weather" => (Color32::from_rgb(0x6c, 0xb8, 0xf0), "Tomorrow's high, and rain, in ten cities"),
        "entertainment" => (Color32::from_rgb(0xb9, 0x7c, 0xe8), "What everyone's playing"),
        "science" => (Color32::from_rgb(0x5a, 0xd6, 0xa0), "Earthquakes, the Sun, rockets, asteroids"),
        "art" => (Color32::from_rgb(0xf0, 0x6a, 0xa8), "The Gallery Draw, and the sky's picture of the day"),
        "community" => (EMBER, "Props your server's members opened"),
        _ => (MUTED, "New here"),
    }
}

/// A category's emblem: a tile in its color with its mark (the leagues' letters; a little
/// drawing for the rest), `r` square.
pub fn emblem(p: &egui::Painter, id: &str, r: Rect, art: &Art) {
    let (col, _) = category_look(id);
    let rad = r.width() * 0.24;
    p.add(gradient(r, rad, mix(col, Color32::BLACK, 0.55), mix(col, Color32::BLACK, 0.25)));
    p.rect_stroke(r, CornerRadius::same(rad as u8), Stroke::new(1.0_f32, mix(col, Color32::WHITE, 0.15).gamma_multiply(0.7)), egui::StrokeKind::Inside);
    let c = r.center();
    let u = r.width() / 40.0;
    let ink = Color32::from_rgb(0xff, 0xf4, 0xe6);
    let line = Stroke::new(1.7 * u, ink);
    let letters = |t: &str| {
        p.text(c, Align2::CENTER_CENTER, t, bold_font(13.5 * u), ink);
    };
    // An ellipse's outline about `c`, `a` wide and `b` high (halves), turned by `turn` degrees.
    let ellipse = |a: f32, b: f32, turn: f32, stroke: Stroke| {
        let (s, co) = turn.to_radians().sin_cos();
        let pts: Vec<Pos2> = (0..=36).map(|i| {
            let t = i as f32 / 36.0 * std::f32::consts::TAU;
            let (x, y) = (a * t.cos(), b * t.sin());
            c + vec2(x * co - y * s, x * s + y * co)
        }).collect();
        p.add(Shape::line(pts, stroke));
    };
    match id {
        "nfl" => letters("NFL"),
        "nba" => letters("NBA"),
        "nhl" => letters("NHL"),
        "mlb" => letters("MLB"),
        "cfb" => letters("CFB"),
        "cbb" => letters("CBB"),
        "topical" => {
            // A globe.
            p.circle_stroke(c, 11.0 * u, line);
            ellipse(5.0 * u, 11.0 * u, 0.0, Stroke::new(1.3 * u, ink));
            p.line_segment([c - vec2(11.0 * u, 0.0), c + vec2(11.0 * u, 0.0)], Stroke::new(1.3 * u, ink));
            p.line_segment([c + vec2(-9.3 * u, -5.5 * u), c + vec2(9.3 * u, -5.5 * u)], Stroke::new(1.0 * u, ink));
            p.line_segment([c + vec2(-9.3 * u, 5.5 * u), c + vec2(9.3 * u, 5.5 * u)], Stroke::new(1.0 * u, ink));
        }
        "casino" => {
            // One of the jeweled chips.
            match art.chips.get(3) {
                Some(t) => {
                    p.image(t.id(), Rect::from_center_size(c, vec2(27.0 * u, 27.0 * u)), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    p.circle_stroke(c, 11.0 * u, line);
                    p.circle_stroke(c, 6.0 * u, Stroke::new(1.2 * u, ink));
                }
            }
        }
        "crypto" => {
            // A coin with a B struck through.
            p.circle_stroke(c, 11.5 * u, line);
            p.text(c + vec2(0.3 * u, 0.2 * u), Align2::CENTER_CENTER, "B", bold_font(15.0 * u), ink);
            for dx in [-1.6, 1.8] {
                p.line_segment([c + vec2(dx * u, -8.5 * u), c + vec2(dx * u, -6.0 * u)], Stroke::new(1.4 * u, ink));
                p.line_segment([c + vec2(dx * u, 6.2 * u), c + vec2(dx * u, 8.7 * u)], Stroke::new(1.4 * u, ink));
            }
        }
        "weather" => {
            // The sun behind a cloud.
            let s = c + vec2(5.0 * u, -5.0 * u);
            p.circle_stroke(s, 4.6 * u, Stroke::new(1.5 * u, ink));
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([s + d * 6.6 * u, s + d * 8.8 * u], Stroke::new(1.3 * u, ink));
            }
            let fill = mix(col, Color32::BLACK, 0.42);
            for (dx, dy, rr) in [(-6.0, 4.5, 4.6), (-0.5, 2.0, 6.2), (5.0, 5.0, 4.2)] {
                p.circle_filled(c + vec2(dx * u, dy * u), rr * u, ink);
            }
            p.rect_filled(Rect::from_min_max(c + vec2(-6.0 * u, 4.5 * u), c + vec2(5.0 * u, 9.2 * u)), CornerRadius::ZERO, ink);
            for (dx, dy, rr) in [(-6.0, 4.5, 3.0), (-0.5, 2.6, 4.6), (5.0, 5.0, 2.6)] {
                p.circle_filled(c + vec2(dx * u, dy * u), rr * u, fill);
            }
            p.rect_filled(Rect::from_min_max(c + vec2(-6.0 * u, 3.2 * u), c + vec2(5.0 * u, 7.6 * u)), CornerRadius::ZERO, fill);
        }
        "entertainment" => {
            // Play.
            p.circle_stroke(c, 11.0 * u, line);
            p.add(Shape::convex_polygon(vec![c + vec2(-3.6 * u, -6.0 * u), c + vec2(6.6 * u, 0.0), c + vec2(-3.6 * u, 6.0 * u)], ink, Stroke::NONE));
        }
        "science" => {
            // An atom.
            for turn in [0.0, 60.0, 120.0] {
                ellipse(12.0 * u, 4.6 * u, turn, Stroke::new(1.3 * u, ink));
            }
            p.circle_filled(c, 2.4 * u, ink);
        }
        "art" => {
            // A picture in its frame.
            let f = Rect::from_center_size(c, vec2(24.0 * u, 19.0 * u));
            p.rect_stroke(f, CornerRadius::same((2.0 * u) as u8), line, egui::StrokeKind::Middle);
            p.add(Shape::convex_polygon(vec![f.left_bottom() + vec2(3.0 * u, -3.0 * u), f.left_bottom() + vec2(9.5 * u, -11.0 * u), f.left_bottom() + vec2(14.5 * u, -3.0 * u)], ink, Stroke::NONE));
            p.add(Shape::convex_polygon(vec![f.left_bottom() + vec2(11.5 * u, -3.0 * u), f.left_bottom() + vec2(16.5 * u, -8.0 * u), f.left_bottom() + vec2(21.0 * u, -3.0 * u)], ink, Stroke::NONE));
            p.circle_filled(f.right_top() + vec2(-5.5 * u, 5.0 * u), 2.0 * u, ink);
        }
        "community" => {
            // People.
            for (dx, s) in [(-5.5f32, 0.85f32), (5.5, 1.0)] {
                let h = c + vec2(dx * u, -4.0 * u * s);
                p.circle_filled(h, 3.6 * u * s, ink);
                p.add(Shape::convex_polygon(vec![h + vec2(-6.0 * u * s, 12.5 * u * s), h + vec2(-4.6 * u * s, 6.0 * u * s), h + vec2(0.0, 4.6 * u * s), h + vec2(4.6 * u * s, 6.0 * u * s), h + vec2(6.0 * u * s, 12.5 * u * s)], ink, Stroke::NONE));
            }
        }
        other => {
            let first: String = other.chars().take(2).collect::<String>().to_uppercase();
            letters(&first);
        }
    }
}

/// A little pill of text: `fill` behind `text` in `col`. Where it ended.
pub fn pill(p: &egui::Painter, at: Pos2, text: &str, col: Color32, fill: Color32, size: f32) -> Rect {
    let g = p.layout_no_wrap(text.to_string(), bold_font(size), col);
    let r = Rect::from_min_size(at, g.size() + vec2(size * 1.2, size * 0.5));
    p.rect_filled(r, CornerRadius::same((r.height() / 2.0) as u8), fill);
    p.galley(r.min + vec2(size * 0.6, size * 0.25), g, col);
    r
}

/// How much of a moment that started at `at` and lasts `secs` is left: 1 at its start, 0 once
/// it's over.
pub fn fading(at: std::time::Instant, secs: f32) -> f32 {
    (1.0 - at.elapsed().as_secs_f32() / secs).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_yes_is_green_a_no_red_and_a_team_neither() {
        let of = |names: &[&str]| -> Vec<usize> {
            let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
            (0..names.len()).map(|i| hue(&names, i)).collect()
        };
        assert_eq!(of(&["Yes", "No"]), vec![0, 1]);
        assert_eq!(of(&["No", "Yes"]), vec![1, 0]);
        assert_eq!(of(&["Up", "Down"]), vec![0, 1]);
        assert_eq!(of(&["Over 39.5", "Under 39.5"]), vec![0, 1]);
        assert_eq!(of(&["Ravens", "Chargers"]), vec![2, 3]);
        // (Three ways it can go: none of them is "the good one".)
        assert_eq!(of(&["Yes", "No", "Maybe"]), vec![2, 3, 4]);
        assert_eq!(of(&["a", "b", "c", "d", "e", "f", "g"]), vec![2, 3, 4, 5, 0, 1, 2]);
    }
}
