//! A big win, as 0.13.28 shows it: the win counts up from nothing through the levels it reaches
//! (Big at ten times the bet, Mega at 25, Epic at 50, Legendary, and Mythic at a thousand), each
//! on its own plaque over turning rays. Crossing into a level stops the count dead for a beat:
//! a flash, the new plaque popping in, a ring going out and a burst of coins. The last level's
//! count slows to the total and lands with another. Mythic sets off fireworks and lightning. A
//! click while it counts jumps to the total; another, once it's there, closes it (it goes by
//! itself three seconds after).
//!
//! (Measured on 0.13.28 by filming it: Dice at 26.75 times, Keno at 22 and 110 times, Pyr Sevens
//! from chosen seeds. Legendary's threshold: see `LEVELS`.)

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Shape, Stroke, pos2, vec2};
use pyr_engine::poker_sfx::Cue;

use super::{Celebration, chips_text, ease_back, radial, times};
use crate::theme;

/// A level: its name, from how many times the bet (hundredths), its plaque (cell of
/// `rr-plaques`, three by two) and where that plaque's panel is, and its colours.
pub struct Level {
    pub name: &'static str,
    pub from_x100: u64,
    pub plaque: usize,
    /// The panel's middle in its cell (fractions across and down) and its width (fraction).
    panel: (f32, f32, f32),
    /// The name's colour and its dark edge.
    pub ink: Color32,
    edge: Color32,
    /// The rays' colour and the glow behind the plaque.
    ray: Color32,
    glow: Color32,
}

pub const LEVELS: [Level; 5] = [
    Level {
        name: "BIG WIN",
        from_x100: 1_000,
        plaque: 0,
        panel: (0.5, 0.5, 0.63),
        ink: Color32::from_rgb(0xff, 0xc8, 0x3c),
        edge: Color32::from_rgb(0x30, 0x18, 0x06),
        ray: Color32::from_rgb(0xb4, 0x96, 0x2c),
        glow: Color32::from_rgb(0x6e, 0x50, 0x14),
    },
    Level {
        name: "MEGA WIN",
        from_x100: 2_500,
        plaque: 1,
        panel: (0.5, 0.58, 0.53),
        ink: Color32::from_rgb(0xfe, 0x70, 0x29),
        edge: Color32::from_rgb(0x22, 0x12, 0x0e),
        ray: Color32::from_rgb(0xc4, 0x6e, 0x30),
        glow: Color32::from_rgb(0x6e, 0x2c, 0x0e),
    },
    Level {
        name: "EPIC WIN",
        from_x100: 5_000,
        plaque: 2,
        panel: (0.51, 0.49, 0.61),
        ink: Color32::from_rgb(0xbe, 0x5a, 0xff),
        edge: Color32::from_rgb(0x1e, 0x0c, 0x34),
        ray: Color32::from_rgb(0x8c, 0x40, 0xa0),
        glow: Color32::from_rgb(0x4a, 0x1e, 0x5a),
    },
    // (Legendary from 250 times: 0.13.28 went into it between 242 and 263 times.)
    Level {
        name: "LEGENDARY WIN",
        from_x100: 25_000,
        plaque: 3,
        panel: (0.495, 0.56, 0.6),
        ink: Color32::from_rgb(0x42, 0xdc, 0xfe),
        edge: Color32::from_rgb(0x08, 0x1c, 0x2c),
        ray: Color32::from_rgb(0x6c, 0xae, 0xb0),
        glow: Color32::from_rgb(0x14, 0x4a, 0x54),
    },
    // (Mythic: not yet seen on 0.13.28; its colours are the rebuild's guess.)
    Level {
        name: "MYTHIC WIN",
        from_x100: 100_000,
        plaque: 4,
        panel: (0.5, 0.62, 0.37),
        ink: Color32::from_rgb(0xff, 0xe2, 0x6a),
        edge: Color32::from_rgb(0x3c, 0x22, 0x04),
        ray: Color32::from_rgb(0xd8, 0xa4, 0x40),
        glow: Color32::from_rgb(0x7a, 0x4a, 0x14),
    },
];

/// The count's pace (seconds; 0.13.28's, filmed: Dice at 22 and 26.75 times, Pyr Sevens at
/// 316.5): fading in; the whole count (`count_secs`), shared evenly by the levels it goes
/// through; the beat on reaching each level; how long the total stays (`hold_secs`); fading out.
const FADE_IN: f32 = 0.2;
const BEAT: f32 = 0.15;
const FADE: f32 = 0.5;

/// The count through `levels` levels: 3.65 seconds for one, 2.35 more for each after.
fn count_secs(levels: usize) -> f32 {
    3.65 + 2.35 * levels.saturating_sub(1) as f32
}

/// The total stays a little longer the higher it went.
fn hold_secs(top: usize) -> f32 {
    2.55 + 0.42 * top as f32
}

/// When the count reaches each level and lands on the total.
#[derive(Clone, Debug)]
pub struct Plan {
    m_x100: u64,
    top: usize,
    /// When each level's plaque comes (level 0: the start), and when its count starts (after
    /// the beat).
    reach: Vec<f32>,
    starts: Vec<f32>,
    end: f32,
}

impl Plan {
    pub fn new(m_x100: u64) -> Plan {
        let top = LEVELS.iter().rposition(|l| m_x100 >= l.from_x100).unwrap_or(0);
        let slot = count_secs(top + 1) / (top + 1) as f32;
        let reach: Vec<f32> = (0..=top).map(|l| l as f32 * slot).collect();
        let starts: Vec<f32> = reach.iter().enumerate().map(|(l, &r)| if l == 0 { 0.0 } else { r + BEAT }).collect();
        Plan { m_x100, top, reach, starts, end: (top + 1) as f32 * slot }
    }

    /// When the count's on the total (a click at `skipped` jumps there), and when it's gone.
    pub fn times(&self, skipped: Option<f32>) -> (f32, f32) {
        let end = skipped.map(|k| k.min(self.end)).unwrap_or(self.end);
        (end, end + hold_secs(self.top) + FADE)
    }

    /// The level showing at `t`.
    pub fn level_at(&self, t: f32, skipped: Option<f32>) -> u8 {
        if skipped.is_some_and(|k| t >= k) {
            return self.top as u8;
        }
        (1..=self.top).rfind(|&l| t >= self.reach[l]).unwrap_or(0) as u8
    }

    /// When the level showing at `t` came.
    fn level_came(&self, t: f32, skipped: Option<f32>) -> f32 {
        let l = self.level_at(t, skipped) as usize;
        match skipped {
            Some(k) if t >= k && self.level_at(k - 0.001, None) as usize != l => k,
            _ => self.reach[l],
        }
    }

    /// How many times the bet the count shows at `t` (hundredths).
    pub fn shown_x100(&self, t: f32, skipped: Option<f32>) -> u64 {
        if skipped.is_some_and(|k| t >= k) || t >= self.end {
            return self.m_x100;
        }
        let l = self.level_at(t, None) as usize;
        let lo = if l == 0 { 0 } else { LEVELS[l].from_x100 };
        if t < self.starts[l] {
            return lo;
        }
        if l < self.top {
            // Climbing past: a little faster as it nears the next level.
            let x = ((t - self.starts[l]) / (self.reach[l + 1] - self.starts[l])).clamp(0.0, 1.0);
            lo + ((LEVELS[l + 1].from_x100 - lo) as f64 * x.powf(1.25) as f64) as u64
        } else {
            // The last: slowing to the total.
            let x = ((t - self.starts[l]) / (self.end - self.starts[l])).clamp(0.0, 1.0);
            lo + ((self.m_x100 - lo) as f64 * (1.0 - (1.0 - x).powi(3)) as f64) as u64
        }
    }

    /// The moments that hit: each level reached after Big, and the landing (or the click that
    /// jumped there).
    fn hits(&self, skipped: Option<f32>) -> Vec<f32> {
        let (end, _) = self.times(skipped);
        let mut h: Vec<f32> = self.reach[1..].iter().copied().filter(|&r| r < end).collect();
        h.push(end);
        h
    }
}

/// A sound of 0.13.28's casino, by its name.
pub fn sound(name: &str) -> Option<Cue> {
    pyr_engine::bands::id(name).map(Cue::Band)
}

/// A number from 0 to 1 for thing `i`'s `k`th choice (the same every frame).
fn rnd(i: u32, k: u32) -> f32 {
    let mut x = i.wrapping_mul(0x9e37_79b1) ^ k.wrapping_mul(0x85eb_ca77) ^ 0x2545_f491;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    x ^= x >> 12;
    x = x.wrapping_mul(0x297a_2d39);
    x ^= x >> 15;
    x as f32 / u32::MAX as f32
}

fn faded(c: Color32, a: f32) -> Color32 {
    let a = a.clamp(0.0, 1.0);
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (c.a() as f32 * a) as u8)
}

/// The big win over `area`, `t` seconds in (`time`: the clock, for what turns and twinkles).
pub fn draw(p: &egui::Painter, ctx: &egui::Context, area: Rect, cel: &Celebration, t: f32, time: f32) {
    let skipped = cel.skipped.map(|k| (k - cel.since).as_secs_f32());
    let plan = &cel.plan;
    let (end, life) = plan.times(skipped);
    let a = (t / FADE_IN).clamp(0.0, 1.0) * ((life - t) / FADE).clamp(0.0, 1.0);
    let level = plan.level_at(t, skipped) as usize;
    let lv = &LEVELS[level];
    let came = t - plan.level_came(t, skipped);
    // (0.13.28's sizes, at its 968 by 744 casino.)
    let s = (area.width() / 968.0).min(area.height() / 744.0);
    let w = 968.0 * s;
    let c = pos2(area.center().x, area.top() + area.height() * 0.42);
    let p = p.with_clip_rect(area);

    // The table darkened, a glow behind the plaque and the rays turning round it.
    p.rect_filled(area, 0.0, Color32::from_black_alpha((222.0 * a) as u8));
    radial(&p, c, w * 0.55, faded(lv.glow, 0.55 * a), Color32::TRANSPARENT);
    rays(&p, c, area, w, lv.ray, a, time);

    // A ring going out from each level's start and the landing.
    let hits = plan.hits(skipped);
    for h in std::iter::once(0.0).chain(hits.iter().copied()) {
        let u = (t - h) / 0.9;
        if (0.0..1.0).contains(&u) {
            let r = w * (0.3 + 0.16 * u);
            p.circle_stroke(c, r, Stroke::new(2.0 * s, faded(Color32::from_rgb(0xff, 0xe8, 0xb0), 0.75 * (1.0 - u) * a)));
        }
    }

    // Coins: raining from the top, and a burst from the plaque at the start and each hit.
    rain(&p, area, s, t, a);
    for (k, h) in std::iter::once(0.0).chain(hits.iter().copied()).enumerate() {
        if t >= h {
            burst(&p, area, c, s, t - h, k as u32, a);
        }
    }

    // Mythic: fireworks and lightning across the screen.
    if level == 4 {
        fireworks(&p, area, s, came, a);
        lightning(&p, area, s, came, a, time);
    }

    // The plaque, popping in as each level comes, a little bigger once the count's landed,
    // breathing a little.
    let pop = 0.55 + 0.45 * ease_back((came / 0.25).min(1.0));
    let landed = 1.0 + 0.05 * ease_back(((t - end) / 0.3).clamp(0.0, 1.0));
    let k = pop * landed * (1.0 + 0.008 * (time * 2.6).sin());
    let cell = vec2(w * 0.61, w * 0.61 * 340.0 / 600.0) * k;
    let panel = pos2(c.x, c.y);
    let cell_rect = Rect::from_min_size(panel - vec2(cell.x * lv.panel.0, cell.y * lv.panel.1), cell);
    let tint = Color32::from_white_alpha((255.0 * a) as u8);
    let shown = super::rmshow::sheet(&p, ctx, "rr-plaques", 3, 2, lv.plaque, cell_rect, tint);
    if shown {
        let pw = cell.x * lv.panel.2;
        name(&p, panel, lv, pw, a);
    }

    // Sparkles round it.
    sparkles(&p, c, w, s, time, a);

    // The amount counting, how many times the bet, and what a click does.
    let x100 = plan.shown_x100(t, skipped);
    let amount = if x100 >= plan.m_x100 { cel.amount } else { (cel.amount as u128 * x100 as u128 / plan.m_x100.max(1) as u128) as u64 };
    let ay = c.y + area.height() * 0.2735;
    outlined(&p, pos2(c.x, ay), &chips_text(amount), theme::bold_font(66.0 * s), faded(Color32::WHITE, a), faded(Color32::from_rgb(0x1a, 0x10, 0x08), 0.85 * a), 2.2 * s);
    let line = format!("{} your bet", times(x100));
    outlined(&p, pos2(c.x, c.y + area.height() * 0.342), &line, theme::bold_font(20.5 * s), faded(lv.ink, a), faded(Color32::from_rgb(0x10, 0x08, 0x04), 0.7 * a), 1.2 * s);
    let hint = if t < end { "click to skip" } else { "click to carry on" };
    p.text(pos2(c.x, c.y + area.height() * 0.382), Align2::CENTER_CENTER, hint, theme::font(13.5 * s), faded(Color32::from_gray(0xc8), 0.75 * a));

    // Pyr Sevens' Phoenix Jackpot: its name over the plaque.
    if cel.jackpot {
        outlined(&p, pos2(c.x, c.y - cell.y * 0.62), "PHOENIX JACKPOT", theme::bold_font(34.0 * s), faded(Color32::from_rgb(0xff, 0xd0, 0x50), a), faded(Color32::from_rgb(0x40, 0x14, 0x04), a), 2.0 * s);
    }

    // A white flash as each level comes and as the count lands (a softer one as it opens).
    for (h, k) in std::iter::once((0.0, 0.45)).chain(hits.iter().map(|&h| (h, 1.0))) {
        let u = (t - h) / 0.32;
        if (0.0..1.0).contains(&u) {
            p.rect_filled(area, 0.0, Color32::from_white_alpha((115.0 * k * (1.0 - u) * a) as u8));
        }
    }
}

/// The level's name across its plaque's panel, as wide as the panel lets it be.
fn name(p: &egui::Painter, at: Pos2, lv: &Level, panel_w: f32, a: f32) {
    let probe = p.layout_no_wrap(lv.name.to_string(), theme::bold_font(100.0), Color32::WHITE);
    let size = (100.0 * panel_w * 0.88 / probe.size().x.max(1.0)).min(panel_w * 0.25);
    let font = theme::bold_font(size);
    // A shadow under it, then the dark edge, then the colour.
    p.text(at + vec2(0.0, size * 0.07), Align2::CENTER_CENTER, lv.name, font.clone(), Color32::from_black_alpha((150.0 * a) as u8));
    outlined(p, at, lv.name, font, faded(lv.ink, a), faded(lv.edge, a), (size * 0.045).max(1.2));
}

/// Text with a dark edge round it.
fn outlined(p: &egui::Painter, at: Pos2, text: &str, font: egui::FontId, color: Color32, edge: Color32, d: f32) {
    for k in 0..8 {
        let ang = k as f32 / 8.0 * std::f32::consts::TAU;
        p.text(at + vec2(ang.cos(), ang.sin()) * d, Align2::CENTER_CENTER, text, font.clone(), edge);
    }
    p.text(at, Align2::CENTER_CENTER, text, font, color);
}

/// Rays turning slowly round the plaque, bright near it and fading out.
fn rays(p: &egui::Painter, c: Pos2, area: Rect, w: f32, color: Color32, a: f32, time: f32) {
    const N: usize = 18;
    let reach = area.width().max(area.height()) * 1.2;
    let turn = time * 0.12;
    let mut m = egui::Mesh::default();
    for i in 0..N {
        let mid = turn + i as f32 / N as f32 * std::f32::consts::TAU;
        let half = std::f32::consts::PI / N as f32 * 0.5;
        let base = m.vertices.len() as u32;
        m.colored_vertex(c, faded(color, 0.55 * a));
        for (ang, r, al) in [(mid - half * 0.55, w * 0.3, 0.42), (mid + half * 0.55, w * 0.3, 0.42), (mid - half, w * 0.62, 0.0), (mid + half, w * 0.62, 0.0), (mid - half * 1.2, reach, 0.0), (mid + half * 1.2, reach, 0.0)] {
            m.colored_vertex(c + vec2(ang.cos(), ang.sin()) * r, faded(color, al * a));
        }
        m.add_triangle(base, base + 1, base + 2);
        m.add_triangle(base + 1, base + 2, base + 4);
        m.add_triangle(base + 1, base + 3, base + 4);
        m.add_triangle(base + 3, base + 4, base + 6);
        m.add_triangle(base + 3, base + 5, base + 6);
    }
    p.add(Shape::mesh(m));
}

/// A gold coin, turning (`spin`) as it flies: a bright rim, a ring inside it, a glint.
fn coin(p: &egui::Painter, c: Pos2, r: f32, spin: f32, a: f32) {
    let k = spin.cos().abs().max(0.14);
    let e = |c, rx: f32, ry: f32, col| egui::epaint::EllipseShape::filled(c, vec2(rx, ry), col);
    p.add(e(c + vec2(0.0, r * 0.08), r * k, r, faded(Color32::from_rgb(0xa8, 0x6c, 0x0c), a)));
    p.add(e(c, r * k, r, faded(Color32::from_rgb(0xff, 0xd6, 0x4e), a)));
    p.add(e(c, r * k * 0.86, r * 0.86, faded(Color32::from_rgb(0xe0, 0xa2, 0x26), a)));
    p.add(egui::epaint::EllipseShape::stroke(c, vec2(r * k * 0.74, r * 0.74), Stroke::new((r * 0.09).max(0.8), faded(Color32::from_rgb(0xf6, 0xc6, 0x4a), a))));
    if k > 0.35 {
        p.add(e(c + vec2(-r * k * 0.32, -r * 0.46), r * k * 0.28, r * 0.13, faded(Color32::from_rgb(0xff, 0xf2, 0xc8), 0.9 * a)));
    }
}

/// Coins falling from the top all the while.
fn rain(p: &egui::Painter, area: Rect, s: f32, t: f32, a: f32) {
    for i in 0..40u32 {
        let period = 2.4 + rnd(i, 1) * 1.6;
        let first = rnd(i, 2) * 1.4;
        if t < first {
            continue;
        }
        let u = (t - first) % period;
        let x = area.left() + rnd(i, 3) * area.width() + (u * 2.0 + i as f32).sin() * 10.0 * s;
        let y = area.top() - 20.0 * s + (180.0 + rnd(i, 4) * 160.0) * s * u + 0.5 * 260.0 * s * u * u;
        if y > area.bottom() + 20.0 * s {
            continue;
        }
        coin(p, pos2(x, y), (12.0 + rnd(i, 5) * 4.0) * s, u * (4.0 + rnd(i, 6) * 5.0) + i as f32, a);
    }
}

/// A burst of coins thrown out from the plaque, `u` seconds ago (`k`: which burst).
fn burst(p: &egui::Painter, area: Rect, c: Pos2, s: f32, u: f32, k: u32, a: f32) {
    if u > 3.0 {
        return;
    }
    for i in 0..34u32 {
        let id = k * 101 + i;
        let ang = std::f32::consts::PI * (1.05 + rnd(id, 7) * 0.9);
        let speed = (380.0 + rnd(id, 8) * 520.0) * s;
        let v = vec2(ang.cos() * speed, ang.sin() * speed);
        let q = c + v * u + vec2(0.0, 0.5 * 1300.0 * s * u * u);
        if !area.expand(30.0 * s).contains(q) {
            continue;
        }
        coin(p, q, (12.0 + rnd(id, 9) * 4.0) * s, u * (6.0 + rnd(id, 10) * 6.0) + id as f32, a);
    }
}

/// White four-pointed stars twinkling round the plaque.
fn sparkles(p: &egui::Painter, c: Pos2, w: f32, s: f32, time: f32, a: f32) {
    for i in 0..16u32 {
        let period = 1.1 + rnd(i, 11) * 0.9;
        let ph = ((time + rnd(i, 12) * period) / period).fract();
        // (Each comes somewhere new every time it twinkles.)
        let n = ((time + rnd(i, 12) * period) / period).floor() as u32;
        let ang = rnd(i * 31 + n, 13) * std::f32::consts::TAU;
        let r = w * (0.2 + rnd(i * 31 + n, 14) * 0.32);
        let q = c + vec2(ang.cos() * r, ang.sin() * r * 0.72);
        let size = (std::f32::consts::PI * ph).sin() * (6.0 + rnd(i, 15) * 8.0) * s;
        if size < 0.5 {
            continue;
        }
        let col = faded(Color32::WHITE, a);
        let thin = size * 0.22;
        p.add(Shape::convex_polygon(vec![q - vec2(size, 0.0), q - vec2(0.0, thin), q + vec2(size, 0.0), q + vec2(0.0, thin)], col, Stroke::NONE));
        p.add(Shape::convex_polygon(vec![q - vec2(0.0, size), q + vec2(thin, 0.0), q + vec2(0.0, size), q - vec2(thin, 0.0)], col, Stroke::NONE));
        p.circle_filled(q, thin * 1.2, faded(Color32::WHITE, 0.6 * a));
    }
}

/// Mythic's fireworks: a burst of coloured sparks somewhere new every half second.
fn fireworks(p: &egui::Painter, area: Rect, s: f32, since: f32, a: f32) {
    const EVERY: f32 = 0.45;
    const LIFE: f32 = 1.4;
    let colours = [
        Color32::from_rgb(0xff, 0xd2, 0x4a),
        Color32::from_rgb(0xff, 0x4a, 0x3a),
        Color32::from_rgb(0x5a, 0xe0, 0xff),
        Color32::from_rgb(0xff, 0x6a, 0xe0),
        Color32::from_rgb(0x7a, 0xff, 0x8a),
    ];
    let latest = (since / EVERY).floor() as i32;
    for n in (latest - 4).max(0)..=latest {
        let born = n as f32 * EVERY;
        let u = since - born;
        if !(0.0..LIFE).contains(&u) {
            continue;
        }
        let n = n as u32;
        let at = pos2(area.left() + area.width() * (0.12 + rnd(n, 20) * 0.76), area.top() + area.height() * (0.1 + rnd(n, 21) * 0.45));
        let colour = colours[(rnd(n, 22) * colours.len() as f32) as usize % colours.len()];
        let fade = (1.0 - u / LIFE).powi(2);
        for i in 0..30u32 {
            let ang = i as f32 / 30.0 * std::f32::consts::TAU + rnd(n, 23);
            let speed = (150.0 + rnd(n * 37 + i, 24) * 90.0) * s;
            let travel = speed * (1.0 - (-u * 2.2).exp()) / 2.2;
            let q = at + vec2(ang.cos(), ang.sin()) * travel + vec2(0.0, 40.0 * s * u * u);
            let tail = at + vec2(ang.cos(), ang.sin()) * travel * 0.82 + vec2(0.0, 40.0 * s * u * u * 0.8);
            p.line_segment([tail, q], Stroke::new(2.0 * s, faded(colour, 0.7 * fade * a)));
            p.circle_filled(q, 2.2 * s, faded(Color32::WHITE, fade * a));
        }
        radial(p, at, 70.0 * s, faded(colour, 0.35 * fade * a), Color32::TRANSPARENT);
    }
}

/// Mythic's lightning: a jagged bolt down from the top, flickering, every so often.
fn lightning(p: &egui::Painter, area: Rect, s: f32, since: f32, a: f32, time: f32) {
    const EVERY: f32 = 0.7;
    const SHOW: f32 = 0.22;
    let n = (since / EVERY).floor() as u32;
    let u = since - n as f32 * EVERY;
    if u > SHOW {
        return;
    }
    let flicker = if (time * 40.0).sin() > -0.3 { 1.0 } else { 0.35 };
    let x0 = area.left() + area.width() * (0.1 + rnd(n, 30) * 0.8);
    let mut pts = vec![pos2(x0, area.top())];
    let steps = 9;
    let bottom = area.top() + area.height() * (0.45 + rnd(n, 31) * 0.3);
    for k in 1..=steps {
        let y = area.top() + (bottom - area.top()) * k as f32 / steps as f32;
        let x = pts[k - 1].x + (rnd(n * 17 + k as u32, 32) - 0.5) * 70.0 * s;
        pts.push(pos2(x, y));
    }
    let al = (1.0 - u / SHOW) * flicker * a;
    p.add(Shape::line(pts.clone(), Stroke::new(9.0 * s, faded(Color32::from_rgb(0x9a, 0xc8, 0xff), 0.3 * al))));
    p.add(Shape::line(pts, Stroke::new(2.5 * s, faded(Color32::WHITE, al))));
    p.rect_filled(area, 0.0, Color32::from_white_alpha((28.0 * al) as u8));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_climbs_through_the_levels_and_lands_on_the_total() {
        // 26.75 times: Big, then Mega (as 0.13.28's Dice at 3.70%).
        let p = Plan::new(2_675);
        assert_eq!(p.top, 1);
        assert_eq!(p.level_at(0.5, None), 0);
        assert_eq!(p.level_at(p.reach[1] + 0.01, None), 1);
        // The beat: the count stops dead on Mega's 25 times.
        assert_eq!(p.shown_x100(p.reach[1] + 0.1, None), 2_500);
        let (end, life) = p.times(None);
        assert_eq!(p.shown_x100(end, None), 2_675);
        assert!(life > end + 3.0);
        // Never down, never past the total.
        let mut last = 0;
        for i in 0..400 {
            let v = p.shown_x100(i as f32 * 0.02, None);
            assert!(v >= last && v <= 2_675);
            last = v;
        }
        // A click jumps to the total.
        assert_eq!(p.shown_x100(1.0, Some(0.9)), 2_675);
        assert_eq!(p.level_at(1.0, Some(0.9)), 1);
        assert_eq!(p.times(Some(0.9)).0, 0.9);
    }

    #[test]
    fn each_win_lands_on_its_level() {
        let top = |x100| Plan::new(x100).top;
        assert_eq!(top(1_000), 0);
        assert_eq!(top(2_499), 0);
        assert_eq!(top(2_500), 1);
        assert_eq!(top(4_950), 1);
        assert_eq!(top(11_000), 2);
        assert_eq!(top(100_000), 4);
    }
}
