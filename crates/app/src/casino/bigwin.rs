//! A big win, as 0.13.28 shows it: the win counts up from nothing through the levels it reaches
//! (Big at ten times the bet, Mega at 25, Epic at 50, Legendary at 250, Mythic at a thousand),
//! each on its own plaque over turning rays. Crossing into a level stops the count dead for a
//! beat: a flash, the new plaque popping in, a ring going out and a fountain of coins. The last
//! level's count slows to the total and lands with another. Mythic's rays turn through every
//! colour, with fireworks and lightning. A jackpot ends on the crown, "JACKPOT!", whatever the
//! count reached. A click while it counts jumps to the total; another, once it's there, closes
//! it (it goes by itself a few seconds after).
//!
//! Its sounds, 0.13.28's (the slots' `s_all_` ones at a slot, the casino's `c_` ones elsewhere):
//! the tier-up as it opens, and again a whole tone higher at each level; from Legendary on a
//! slam with it, a semitone higher each level; ticks as it counts, fifty a second, rising half a
//! semitone at a time to an octave; and at the landing a slam two semitones under the level's
//! (a jackpot's crown comes with Mythic's tier-up and slam as well).
//!
//! (Measured on 0.13.28, filmed and recorded: Dice at 22 and 26.75 times, Keno at 22 and 110,
//! Pyr Sevens at 247.65, 250.10, 252.15 and 316.50 from chosen seeds on a test server, Ember
//! Dragon's Grand at 1,078 and 1,079 times and Minis at 20.20 and 21.20.)

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Shape, Stroke, pos2, vec2};
use pyr_engine::poker_sfx::Cue;

use super::{Celebration, chips_text, radial, times};
use crate::theme;

/// A level: its name, from how many times the bet (hundredths), its plaque (cell of
/// `rr-plaques`, three by two) and where that plaque's panel is, and its colours.
pub struct Level {
    pub name: &'static str,
    pub from_x100: u64,
    pub plaque: usize,
    /// The panel's middle in its cell (fractions across and down) and its width (fraction).
    panel: (f32, f32, f32),
    /// The name's colour and its dark edge (Mythic's and the jackpot's turn with the rays).
    ink: Color32,
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
        ray: Color32::from_rgb(0xd2, 0x7c, 0x38),
        glow: Color32::from_rgb(0x74, 0x30, 0x10),
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
    // (0.13.28 went into Legendary between 248.55 and 250.10 times: from 250.)
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
    Level {
        name: "MYTHIC WIN",
        from_x100: 100_000,
        plaque: 4,
        panel: (0.5, 0.6, 0.36),
        ink: Color32::from_rgb(0xff, 0xc8, 0xe8),
        edge: Color32::from_rgb(0x2c, 0x10, 0x20),
        ray: Color32::from_rgb(0xc8, 0xa0, 0x60),
        glow: Color32::from_rgb(0x6a, 0x40, 0x30),
    },
];

/// A jackpot's last word, on the crown once the count's there (whatever level it reached).
const JACKPOT: Level = Level { name: "JACKPOT!", ..LEVELS[4] };
const MYTHIC: usize = 4;
const CROWN: usize = 5;

fn level(i: usize) -> &'static Level {
    if i == CROWN { &JACKPOT } else { &LEVELS[i.min(MYTHIC)] }
}

/// The pace (seconds; 0.13.28's, filmed): fading in; the beat on reaching each level before the
/// count goes on; fading out.
const FADE_IN: f32 = 0.2;
const BEAT: f32 = 0.12;
const FADE: f32 = 0.45;
/// The longest count (five levels': 11.0 and 11.1 seconds), and every jackpot's (a Mini at 20
/// times took as long as a Grand at a thousand).
const LONGEST: f32 = 11.0;

/// The whole count through `levels` levels, shared evenly by them: 3.65 seconds for one, 2.3
/// more for each after (6, 7.9, 10.3 to 10.5 filmed), to the longest.
fn count_secs(levels: usize) -> f32 {
    (3.65 + 2.3 * levels.saturating_sub(1) as f32).min(LONGEST)
}

/// The total stays a little longer the higher it went (2.5 s at Big to 4.5 at the crown).
fn hold_secs(top: usize) -> f32 {
    2.55 + 0.47 * top as f32
}

/// When the count reaches each level and lands on the total.
#[derive(Clone, Debug)]
pub struct Plan {
    m_x100: u64,
    /// The level the count ends on (0: Big).
    top: usize,
    jackpot: bool,
    /// When each level's plaque comes (level 0: the start), and when its count starts (after
    /// the beat).
    reach: Vec<f32>,
    starts: Vec<f32>,
    end: f32,
}

impl Plan {
    pub fn new(m_x100: u64, jackpot: bool) -> Plan {
        let top = LEVELS.iter().rposition(|l| m_x100 >= l.from_x100).unwrap_or(0);
        let count = if jackpot { LONGEST } else { count_secs(top + 1) };
        let slot = count / (top + 1) as f32;
        let reach: Vec<f32> = (0..=top).map(|l| l as f32 * slot).collect();
        let starts: Vec<f32> = reach.iter().enumerate().map(|(l, &r)| if l == 0 { 0.0 } else { r + BEAT }).collect();
        Plan { m_x100, top, jackpot, reach, starts, end: count }
    }

    /// The level it ends on: the count's, or a jackpot's crown.
    fn last(&self) -> usize {
        if self.jackpot { CROWN } else { self.top }
    }

    /// When the count's on the total (a click at `skipped` jumps there), and when it's gone.
    pub fn times(&self, skipped: Option<f32>) -> (f32, f32) {
        let end = skipped.map(|k| k.min(self.end)).unwrap_or(self.end);
        let hold = hold_secs(if self.jackpot { MYTHIC } else { self.top });
        (end, end + hold + FADE)
    }

    /// The level showing at `t` (`CROWN`: a jackpot's, once the count's there).
    pub fn level_at(&self, t: f32, skipped: Option<f32>) -> usize {
        let (end, _) = self.times(skipped);
        if t >= end {
            return self.last();
        }
        self.counting_level(t)
    }

    fn counting_level(&self, t: f32) -> usize {
        (1..=self.top).rfind(|&l| t >= self.reach[l]).unwrap_or(0)
    }

    /// When the level showing at `t` came.
    fn level_came(&self, t: f32, skipped: Option<f32>) -> f32 {
        let (end, _) = self.times(skipped);
        if t >= end {
            let before = self.counting_level(end - 1e-4);
            return if self.last() != before { end } else { self.reach[before] };
        }
        self.reach[self.counting_level(t)]
    }

    /// How many times the bet the count shows at `t` (hundredths).
    pub fn shown_x100(&self, t: f32, skipped: Option<f32>) -> u64 {
        if skipped.is_some_and(|k| t >= k) || t >= self.end {
            return self.m_x100;
        }
        let l = self.counting_level(t);
        let lo = if l == 0 { 0 } else { LEVELS[l].from_x100 };
        if t < self.starts[l] {
            return lo;
        }
        if l < self.top {
            // Climbing past: a little faster as it nears the next level.
            let x = ((t - self.starts[l]) / (self.reach[l + 1] - self.starts[l])).clamp(0.0, 1.0);
            lo + ((LEVELS[l + 1].from_x100 - lo) as f64 * x.powf(1.3) as f64) as u64
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

    /// Every sound, when (seconds in), how loud, and whether it's a tick (`slot`: the slots'
    /// sounds, not the casino's). As 0.13.28: the tier-up as it opens and a whole tone higher at
    /// each level; from Legendary on a slam with it, a semitone higher each level; the ticks,
    /// fifty a second, half a semitone higher every twenty-fourth of the count; the landing's
    /// slam, two semitones under the level's (a jackpot's crown has Mythic's tier-up and slam
    /// too). How loud: as 0.13.28's, recorded at the same settings (the tier-up at 0.85, the
    /// slams at 0.7 and 1).
    pub fn sounds(&self, skipped: Option<f32>, slot: bool) -> Vec<(f32, Cue, f32, bool)> {
        let (tierup, slam, tick, tick_vol) = if slot { ("s_all_tierup", "s_all_winslam", "s_all_count", 0.1) } else { ("c_tierup", "c_winslam", "c_coin", 0.19) };
        let mut out = Vec::new();
        // (`up`: hundredths of a semitone.)
        let mut add = |at: f32, name: &str, up: i32, vol: f32| {
            if let Some(c) = cue(name, up) {
                out.push((at, c, vol, name == tick));
            }
        };
        let (end, _) = self.times(skipped);
        let hit = |add: &mut dyn FnMut(f32, &str, i32, f32), at: f32, l: usize| {
            add(at, tierup, 200 * l as i32, 0.85);
            if l >= 3 {
                add(at, slam, 100 * (l as i32 - 2), 0.7);
            }
        };
        add(0.0, tierup, 0, 0.85);
        for l in 1..=self.top {
            if self.reach[l] < end {
                hit(&mut add, self.reach[l], l);
            }
        }
        // The ticks: fifty a second all through the count, rising half a semitone at a time, an
        // octave in all.
        let mut at = 0.0;
        while at < end {
            let step = (at / self.end * 24.0).floor() as i32;
            add(at, tick, 50 * step, tick_vol);
            at += 0.02;
        }
        let last = if self.jackpot {
            hit(&mut add, end, MYTHIC);
            MYTHIC
        } else {
            self.top
        };
        add(end, slam, 100 * (last as i32 - 2), 1.0);
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }
}

/// A sound of 0.13.28's casino by its name, `cents` hundredths of a semitone higher.
pub fn cue(name: &str, cents: i32) -> Option<Cue> {
    pyr_engine::bands::id(name).map(|i| if cents == 0 { Cue::Band(i) } else { Cue::BandUp(i, cents.clamp(-2400, 2400) as i16) })
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

/// A colour from its hue (0 to 1 round the wheel), saturation and value.
fn hsv(h: f32, s: f32, v: f32) -> Color32 {
    let h = h.rem_euclid(1.0) * 6.0;
    let i = h.floor();
    let f = h - i;
    let (p, q, u) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let (r, g, b) = match i as u32 {
        0 => (v, u, p),
        1 => (q, v, p),
        2 => (p, v, u),
        3 => (p, q, v),
        4 => (u, p, v),
        _ => (v, p, q),
    };
    Color32::from_rgb((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

/// The big win over `area`, `t` seconds in (`time`: the clock, for what turns and twinkles).
pub fn draw(p: &egui::Painter, ctx: &egui::Context, area: Rect, cel: &Celebration, t: f32, time: f32) {
    let skipped = cel.skipped.map(|k| (k - cel.since).as_secs_f32());
    let plan = &cel.plan;
    let (end, life) = plan.times(skipped);
    let a = (t / FADE_IN).clamp(0.0, 1.0) * ((life - t) / FADE).clamp(0.0, 1.0);
    let li = plan.level_at(t, skipped);
    let lv = level(li);
    let came = t - plan.level_came(t, skipped);
    // Mythic and the crown: everything turns through the colours (once round in four seconds).
    let wheel = li >= MYTHIC;
    let hue = (time * 0.25).rem_euclid(1.0);
    let (ray, glow, ink, edge, line) = if wheel {
        (hsv(hue, 0.5, 0.8), hsv(hue, 0.6, 0.42), hsv(hue, 0.32, 1.0), hsv(hue, 0.7, 0.14), hsv(hue, 0.42, 1.0))
    } else {
        (lv.ray, lv.glow, lv.ink, lv.edge, lv.ink)
    };
    // (0.13.28's sizes, at its 968 by 744 casino.)
    let s = (area.width() / 968.0).min(area.height() / 744.0);
    let w = 968.0 * s;
    let c = pos2(area.center().x, area.top() + area.height() * 0.42);
    let p = p.with_clip_rect(area);

    // The table darkened, a glow behind the plaque and the rays turning round it.
    p.rect_filled(area, 0.0, Color32::from_black_alpha((222.0 * a) as u8));
    radial(&p, c, w * 0.55, faded(glow, 0.55 * a), Color32::TRANSPARENT);
    rays(&p, c, area, w, ray, a, time);

    // A ring going out round the plaque as it opens, at each level and at the landing.
    let hits = plan.hits(skipped);
    for h in std::iter::once(0.0).chain(hits.iter().copied()) {
        let d = t - h;
        if (0.0..0.55).contains(&d) {
            let r = w * (0.16 + 0.62 * d);
            p.circle_stroke(c, r, Stroke::new(2.0 * s, faded(Color32::from_rgb(0xff, 0xe8, 0xb0), 0.8 * (1.0 - d / 0.55) * a)));
        }
    }

    // Mythic and the crown: sparks flying out, fireworks bursting round the screen.
    if wheel {
        sparks(&p, area, c, s, came, a);
        fireworks(&p, area, c, s, came, a);
    }

    // Coins: a fountain of them from the bottom as it opens, at each level and at the landing.
    for (k, h) in std::iter::once(0.0).chain(hits.iter().copied()).enumerate() {
        if t >= h {
            burst(&p, area, s, t - h, k as u32, a);
        }
    }

    // The plaque, popping in as each level comes, a little bigger once the count's landed,
    // breathing a little.
    let x = (came / 0.22).clamp(0.0, 1.0);
    let pop = 0.35 + 0.65 * (1.0 - (1.0 - x) * (1.0 - x));
    let landed = 1.0 + 0.05 * ((t - end) / 0.3).clamp(0.0, 1.0);
    let k = pop * landed * (1.0 + 0.008 * (time * 2.6).sin());
    let cell = vec2(w * 0.61, w * 0.61 * 340.0 / 600.0) * k;
    let cell_rect = Rect::from_min_size(c - vec2(cell.x * lv.panel.0, cell.y * lv.panel.1), cell);
    let tint = Color32::from_white_alpha((255.0 * a) as u8);
    if plaque(&p, ctx, lv.plaque, cell_rect, area, tint) {
        name(&p, c, lv.name, ink, edge, cell.x * lv.panel.2, s, a);
    }

    // Sparkles round it.
    sparkles(&p, c, w, s, time, a);

    // The amount counting, how many times the bet, and what a click does.
    let x100 = plan.shown_x100(t, skipped);
    let amount = if x100 >= plan.m_x100 { cel.amount } else { (cel.amount as u128 * x100 as u128 / plan.m_x100.max(1) as u128) as u64 };
    let ay = c.y + area.height() * 0.2735;
    outlined(&p, pos2(c.x, ay), &chips_text(amount), theme::bold_font(66.0 * s), faded(Color32::WHITE, a), faded(Color32::from_rgb(0x1a, 0x10, 0x08), 0.85 * a), 2.2 * s);
    let bet = format!("{} your bet", times(x100));
    outlined(&p, pos2(c.x, c.y + area.height() * 0.342), &bet, theme::bold_font(20.5 * s), faded(line, a), faded(Color32::from_rgb(0x10, 0x08, 0x04), 0.7 * a), 1.2 * s);
    let hint = if t < end { "click to skip" } else { "click to carry on" };
    p.text(pos2(c.x, c.y + area.height() * 0.382), Align2::CENTER_CENTER, hint, theme::font(13.5 * s), faded(Color32::from_gray(0xc8), 0.75 * a));

    // Lightning as Mythic comes and as a Mythic or a jackpot lands.
    if wheel {
        for &h in &hits {
            if plan.level_at(h, skipped) >= MYTHIC && (0.0..0.45).contains(&(t - h)) {
                lightning(&p, area, s, t - h, (h * 10.0) as u32, a, time);
            }
        }
    }

    // A white flash as each level comes and as the count lands (a softer one as it opens).
    for (h, k) in std::iter::once((0.0, 0.45)).chain(hits.iter().map(|&h| (h, 1.0))) {
        let u = (t - h) / 0.32;
        if (0.0..1.0).contains(&u) {
            p.rect_filled(area, 0.0, Color32::from_white_alpha((140.0 * k * (1.0 - u) * a) as u8));
        }
    }
}

/// The plaques' sheet's size on the card: the biggest it's drawn at, whatever the pop (a size
/// that changed as it popped would have the card read it again, and nothing drawn till then).
fn plaques_px(area: Rect) -> u32 {
    let s = (area.width() / 968.0).min(area.height() / 744.0);
    (968.0 * s * 0.61 * 1.07 * 3.0 * 1.4) as u32 + 1
}

/// Plaque `i` (of `rr-plaques`, three by two) in `r`.
fn plaque(p: &egui::Painter, ctx: &egui::Context, i: usize, r: Rect, area: Rect, tint: Color32) -> bool {
    let (cx, cy) = ((i % 3) as f32, (i / 3) as f32);
    let uv = Rect::from_min_max(pos2(cx / 3.0, cy / 2.0), pos2((cx + 1.0) / 3.0, (cy + 1.0) / 2.0));
    super::pics::part(p, ctx, "rr-plaques", uv, r, plaques_px(area), tint)
}

/// The plaques read before they're wanted (a game's page asks every frame; nothing's done once
/// they're there), so a big win's first never comes without its plaque.
pub fn warm(ctx: &egui::Context, area: Rect) {
    let _ = super::pics::tex(ctx, "rr-plaques", plaques_px(area));
}

/// The level's name across its plaque's panel, as wide as the panel lets it be, edged darkly
/// (as thick on Mythic's small panel as on Big's).
fn name(p: &egui::Painter, at: Pos2, text: &str, ink: Color32, edge: Color32, panel_w: f32, s: f32, a: f32) {
    let probe = p.layout_no_wrap(text.to_string(), theme::bold_font(100.0), Color32::WHITE);
    let size = (100.0 * panel_w * 0.88 / probe.size().x.max(1.0)).min(panel_w * 0.25);
    let font = theme::bold_font(size);
    // A shadow under it, then the dark edge, then the colour.
    p.text(at + vec2(0.0, size * 0.07), Align2::CENTER_CENTER, text, font.clone(), Color32::from_black_alpha((150.0 * a) as u8));
    outlined(p, at, text, font, faded(ink, a), faded(edge, a), (size * 0.055).max(2.4 * s));
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

/// A fountain of coins thrown up from the bottom of the table, `u` seconds ago (`k`: which),
/// spreading across it and falling back.
fn burst(p: &egui::Painter, area: Rect, s: f32, u: f32, k: u32, a: f32) {
    if u > 3.0 {
        return;
    }
    let from = pos2(area.center().x, area.bottom() + 12.0 * s);
    for i in 0..64u32 {
        let id = k * 101 + i;
        let ang = (rnd(id, 7) - 0.5) * 1.2;
        let speed = (850.0 + rnd(id, 8) * 450.0) * s;
        let v = vec2(ang.sin() * speed, -ang.cos() * speed);
        let q = from + vec2((rnd(id, 11) - 0.5) * 60.0 * s, 0.0) + v * u + vec2(0.0, 0.5 * 1100.0 * s * u * u);
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
        star(p, q, size, faded(Color32::WHITE, a));
    }
}

fn star(p: &egui::Painter, q: Pos2, size: f32, col: Color32) {
    let thin = size * 0.22;
    p.add(Shape::convex_polygon(vec![q - vec2(size, 0.0), q - vec2(0.0, thin), q + vec2(size, 0.0), q + vec2(0.0, thin)], col, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![q - vec2(0.0, size), q + vec2(thin, 0.0), q + vec2(0.0, size), q - vec2(thin, 0.0)], col, Stroke::NONE));
    p.circle_filled(q, thin * 1.2, faded(col, 0.6));
}

/// The fireworks' colours: pink, mint, sky, lilac, peach, white.
const SPARK: [Color32; 6] = [
    Color32::from_rgb(0xff, 0x8c, 0xc8),
    Color32::from_rgb(0x8c, 0xf0, 0xb4),
    Color32::from_rgb(0x8c, 0xdc, 0xff),
    Color32::from_rgb(0xc4, 0x9c, 0xff),
    Color32::from_rgb(0xff, 0xc8, 0x8c),
    Color32::from_rgb(0xf4, 0xf4, 0xff),
];

/// Mythic's sparks: thin lines of colour with a white head, flying out from the plaque across
/// the screen all the while (`since`: since Mythic or the crown came).
fn sparks(p: &egui::Painter, area: Rect, c: Pos2, s: f32, since: f32, a: f32) {
    let far = area.width().max(area.height());
    for i in 0..150u32 {
        let period = 0.9 + rnd(i, 40) * 0.8;
        let ph = since + rnd(i, 41) * period;
        let n = (ph / period).floor() as u32;
        let u = ph / period - n as f32;
        let id = i * 977 + n;
        let ang = rnd(id, 42) * std::f32::consts::TAU;
        let dir = vec2(ang.cos(), ang.sin());
        // (Flying out, each a little askew.)
        let tilt = (rnd(id, 45) - 0.5) * 0.7;
        let along = vec2(dir.x * tilt.cos() - dir.y * tilt.sin(), dir.x * tilt.sin() + dir.y * tilt.cos());
        let r = far * (0.06 + 0.7 * u);
        let len = (28.0 + rnd(id, 43) * 34.0) * s;
        let head = c + dir * r;
        let tail = head - along * len;
        let col = SPARK[(rnd(id, 44) * SPARK.len() as f32) as usize % SPARK.len()];
        let al = a * (u / 0.08).min(1.0) * 0.85;
        p.line_segment([tail, head], Stroke::new(1.6 * s, faded(col, al)));
        p.circle_filled(head, 1.8 * s, faded(Color32::WHITE, al));
    }
}

/// Mythic's fireworks: a ring of short dashes opening out somewhere new every third of a
/// second, each in one colour.
fn fireworks(p: &egui::Painter, area: Rect, c: Pos2, s: f32, since: f32, a: f32) {
    const EVERY: f32 = 0.24;
    const LIFE: f32 = 1.0;
    let latest = (since / EVERY).floor() as i32;
    for n in (latest - 5).max(0)..=latest {
        let u = (since - n as f32 * EVERY) / LIFE;
        if !(0.0..1.0).contains(&u) {
            continue;
        }
        let n = n as u32;
        // (Anywhere but over the plaque and the count.)
        let mut at = pos2(area.left() + area.width() * (0.08 + rnd(n, 20) * 0.84), area.top() + area.height() * (0.08 + rnd(n, 21) * 0.62));
        if (at.x - c.x).abs() < area.width() * 0.2 {
            at.x += if at.x < c.x { -area.width() * 0.2 } else { area.width() * 0.2 };
        }
        let col = SPARK[(rnd(n, 22) * 5.0) as usize % 5];
        let open = 1.0 - (1.0 - u).powi(3);
        let r0 = (14.0 + 50.0 * open) * s;
        let dash = (22.0 * (1.0 - u) + 6.0) * s;
        let al = a * (1.0 - u).powf(1.5);
        for i in 0..22 {
            let ang = i as f32 / 22.0 * std::f32::consts::TAU + rnd(n, 23);
            let d = vec2(ang.cos(), ang.sin());
            p.line_segment([at + d * r0, at + d * (r0 + dash)], Stroke::new(1.6 * s, faded(col, al)));
        }
    }
}

/// Lightning: two jagged white bolts with a lilac glow from the top to the bottom, flickering,
/// `u` seconds after it struck (`n`: which strike).
fn lightning(p: &egui::Painter, area: Rect, s: f32, u: f32, n: u32, a: f32, time: f32) {
    let flicker = if (time * 38.0).sin() > -0.35 { 1.0 } else { 0.4 };
    let al = (1.0 - u / 0.45).clamp(0.0, 1.0) * flicker * a;
    let x0 = area.left() + area.width() * (0.62 + rnd(n, 30) * 0.2);
    for b in 0..2u32 {
        let mut pts = vec![pos2(x0 + b as f32 * 26.0 * s, area.top())];
        let steps = 12;
        for k in 1..=steps {
            let y = area.top() + area.height() * k as f32 / steps as f32;
            let x = pts[k - 1].x + (rnd(n * 17 + b * 101 + k as u32, 32) - 0.5) * 64.0 * s - 3.0 * s;
            pts.push(pos2(x, y));
        }
        p.add(Shape::line(pts.clone(), Stroke::new(8.0 * s, faded(Color32::from_rgb(0xc8, 0xa8, 0xff), 0.35 * al))));
        p.add(Shape::line(pts, Stroke::new(2.2 * s, faded(Color32::WHITE, al))));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_climbs_through_the_levels_and_lands_on_the_total() {
        // 26.75 times: Big, then Mega (as 0.13.28's Dice at 3.70%).
        let p = Plan::new(2_675, false);
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
        let top = |x100| Plan::new(x100, false).top;
        assert_eq!(top(1_000), 0);
        assert_eq!(top(2_499), 0);
        assert_eq!(top(2_500), 1);
        assert_eq!(top(4_950), 1);
        assert_eq!(top(11_000), 2);
        assert_eq!(top(24_855), 2);
        assert_eq!(top(25_010), 3);
        assert_eq!(top(100_000), 4);
    }

    /// The counts' lengths as 0.13.28's, filmed.
    #[test]
    fn the_counts_take_as_long_as_0_13_28s() {
        let end = |x100, jackpot| Plan::new(x100, jackpot).times(None).0;
        assert!((end(2_200, false) - 3.6).abs() < 0.1);
        assert!((end(24_765, false) - 7.9).abs() < 0.35);
        assert!((end(31_650, false) - 10.45).abs() < 0.25);
        assert!((end(107_900, true) - 11.0).abs() < 0.15);
        // A jackpot counts as long whatever it pays, and ends on the crown.
        let mini = Plan::new(2_020, true);
        assert!((mini.times(None).0 - 11.0).abs() < 0.01);
        assert_eq!(mini.level_at(10.9, None), 0);
        assert_eq!(mini.level_at(11.0, None), CROWN);
        assert!((mini.times(None).1 - mini.times(None).0 - 4.9).abs() < 0.1);
    }

    /// Its sounds: the tier-up a tone higher at each level, the slams, an octave of ticks, the
    /// landing.
    #[test]
    fn its_sounds_are_0_13_28s() {
        let id = |n| pyr_engine::bands::id(n).unwrap();
        let p = Plan::new(31_650, false);
        let snd = p.sounds(None, true);
        let tierups: Vec<(f32, Cue)> = snd.iter().filter(|s| matches!(s.1, Cue::Band(i) | Cue::BandUp(i, _) if i == id("s_all_tierup"))).map(|s| (s.0, s.1)).collect();
        assert_eq!(tierups.iter().map(|s| s.1).collect::<Vec<_>>(), vec![Cue::Band(id("s_all_tierup")), Cue::BandUp(id("s_all_tierup"), 200), Cue::BandUp(id("s_all_tierup"), 400), Cue::BandUp(id("s_all_tierup"), 600)]);
        let slams: Vec<Cue> = snd.iter().filter(|s| matches!(s.1, Cue::Band(i) | Cue::BandUp(i, _) if i == id("s_all_winslam"))).map(|s| s.1).collect();
        // Legendary's, and the landing's at Legendary.
        assert_eq!(slams, vec![Cue::BandUp(id("s_all_winslam"), 100), Cue::BandUp(id("s_all_winslam"), 100)]);
        let ticks: Vec<&(f32, Cue, f32, bool)> = snd.iter().filter(|s| matches!(s.1, Cue::Band(i) | Cue::BandUp(i, _) if i == id("s_all_count"))).collect();
        assert!(ticks.len() > 450 && ticks.iter().all(|t| t.3));
        assert_eq!(ticks.last().unwrap().1, Cue::BandUp(id("s_all_count"), 1150));
        // Dice's big win, the casino's sounds: the landing's slam two semitones down.
        let d = Plan::new(2_200, false).sounds(None, false);
        assert_eq!(d.last().unwrap().1, Cue::BandUp(id("c_winslam"), -200));
        assert!(d.iter().any(|s| s.1 == Cue::Band(id("c_tierup")) && s.0 == 0.0));
        // A jackpot's crown: Mythic's tier-up and slam, and the landing's.
        let j = Plan::new(2_020, true).sounds(None, true);
        let at_end: Vec<Cue> = j.iter().filter(|s| s.0 == 11.0).map(|s| s.1).collect();
        assert!(at_end.contains(&Cue::BandUp(id("s_all_tierup"), 800)));
        assert_eq!(at_end.iter().filter(|c| **c == Cue::BandUp(id("s_all_winslam"), 200)).count(), 2);
    }
}
