//! Voice meters: how each person in my call sounds right now, in the look picked in Settings
//! → Voice & Video. The engine reads everyone's level and bands every 20 ms
//! (`Engine::voice_meters`: the people I hear, and me while I'm sending); here they're eased
//! (quick to rise and slow to fall, like a real meter's) and painted: by my name in the voice
//! panel, by each person in my voice channel, and on call tiles.

use std::collections::HashMap;
use std::f32::consts::{PI, TAU};
use std::time::Duration;

use eframe::egui::{self, Align2, Color32, CornerRadius, Mesh, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::audio::Meter;
use pyr_engine::audio::spectrum::BANDS;
use pyr_proto::control::SessionId;

use crate::theme;

/// How the meters look.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Style {
    /// Bars growing out of a line through the middle: deep sounds in the middle, bright ones
    /// out to the sides (like the website's).
    Bars,
    /// An equalizer: a bar for each band, deep on the left and bright on the right, green to
    /// amber to red, with caps that fall back slowly.
    Spectrum,
    /// Glowing waves that swell and ripple with the voice.
    Wave,
    /// A row of lights, green to amber to red, like a mixing desk's.
    Dots,
    /// A light that brightens with the voice and sends out rings on each word (around
    /// pictures too).
    Glow,
    Off,
}

impl Style {
    #[cfg(test)]
    pub const ALL: [Style; 6] = [Style::Bars, Style::Spectrum, Style::Wave, Style::Dots, Style::Glow, Style::Off];
    /// The looks to pick from.
    pub const LOOKS: [Style; 5] = [Style::Bars, Style::Spectrum, Style::Wave, Style::Dots, Style::Glow];

    /// From `Settings::voice_meter`: Off when it's "off", or "off:" and the look they had
    /// (see `off_setting`); Bars when it's empty or unknown.
    pub fn from_setting(s: &str) -> Style {
        if s == "off" || s.starts_with("off:") { Style::Off } else { Style::look_of(s) }
    }

    /// The look `Settings::voice_meter` shows, or showed before they were turned off.
    pub fn look_of(s: &str) -> Style {
        match s.strip_prefix("off:").unwrap_or(s) {
            "spectrum" => Style::Spectrum,
            "wave" => Style::Wave,
            "dots" => Style::Dots,
            "glow" => Style::Glow,
            _ => Style::Bars,
        }
    }

    /// `Settings::voice_meter` with the meters turned off, keeping their look for when they're
    /// turned back on.
    pub fn off_setting(s: &str) -> String {
        format!("off:{}", Style::look_of(s).setting())
    }

    pub fn setting(self) -> &'static str {
        match self {
            Style::Bars => "bars",
            Style::Spectrum => "spectrum",
            Style::Wave => "wave",
            Style::Dots => "dots",
            Style::Glow => "glow",
            Style::Off => "off",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Style::Bars => "Bars",
            Style::Spectrum => "Spectrum",
            Style::Wave => "Wave",
            Style::Dots => "Dots",
            Style::Glow => "Glow",
            Style::Off => "Off",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Style::Bars => "Bars from the middle out, like a waveform: deep sounds in the middle, bright ones at the sides",
            Style::Spectrum => "An equalizer: deep sounds on the left, bright ones on the right, with caps that fall back slowly",
            Style::Wave => "Glowing waves that swell and ripple with the voice",
            Style::Dots => "Lights from green to amber to red, like a mixing desk's",
            Style::Glow => "A light that pulses with the voice and sends out rings on each word, around pictures too",
            Style::Off => "No voice meters (the speaking rings still show)",
        }
    }
}

/// How long a ring sent out on a word lasts (s).
const RIPPLE_LIFE: f32 = 1.1;

/// `cur` moved towards `target` over `dt` seconds: quickly (time constant `rise`) going up,
/// slowly (`fall`) coming down.
fn ease(cur: f32, target: f32, dt: f32, rise: f32, fall: f32) -> f32 {
    let tau = if target > cur { rise } else { fall };
    cur + (target - cur) * (1.0 - (-dt / tau).exp())
}

/// dBFS to 0..1, the way loudness is heard: -62 dB and quieter is nothing, -12 dB is full.
pub fn level_of(db: f32) -> f32 {
    ((db + 62.0) / 50.0).clamp(0.0, 1.0).powf(1.15)
}

/// Something that jumps up to a new high, stays there a moment (`hold_for`), then falls
/// faster and faster (a meter's peak light, an equalizer's caps).
fn hold_and_fall(v: &mut f32, speed: &mut f32, hold: &mut f32, now: f32, dt: f32, hold_for: f32) {
    if now >= *v {
        *v = now;
        *speed = 0.0;
        *hold = hold_for;
    } else if *hold > 0.0 {
        *hold -= dt;
    } else {
        *speed += 2.4 * dt;
        *v = (*v - *speed * dt).max(now);
    }
}

/// One person's meter, eased for drawing.
#[derive(Clone, Debug, Default)]
pub struct Track {
    /// How loud, 0 to 1.
    pub level: f32,
    /// The loudest lately (held a moment, then falling).
    pub peak: f32,
    peak_speed: f32,
    peak_hold: f32,
    /// Each band, deep to bright, 0 to 1, and its cap (the highest lately, falling).
    pub bands: [f32; BANDS],
    pub caps: [f32; BANDS],
    caps_speed: [f32; BANDS],
    caps_hold: [f32; BANDS],
    /// Where the waves are (they move faster when it's louder).
    pub phase: f32,
    /// Rings sent out on words: (seconds old, how loud the word was).
    pub ripples: Vec<(f32, f32)>,
    /// The level between words, to tell when a new one starts, and whether this word has had
    /// its ring.
    floor: f32,
    rang: bool,
}

impl Track {
    fn step(&mut self, m: Option<&Meter>, dt: f32) {
        let (target, bands) = m.map(|m| (level_of(m.db), m.bands)).unwrap_or((0.0, [0.0; BANDS]));
        self.level = ease(self.level, target, dt, 0.025, 0.16);
        for i in 0..BANDS {
            self.bands[i] = ease(self.bands[i], bands[i], dt, 0.02, 0.24);
            hold_and_fall(&mut self.caps[i], &mut self.caps_speed[i], &mut self.caps_hold[i], self.bands[i], dt, 0.35);
        }
        hold_and_fall(&mut self.peak, &mut self.peak_speed, &mut self.peak_hold, self.level, dt, 0.6);
        self.phase = (self.phase + dt * (1.4 + 7.5 * self.level)) % (TAU * 64.0);
        // A word starting (the level jumping up from where it was between words): a ring.
        self.floor = if target < self.floor { target } else { self.floor + (target - self.floor) * (1.0 - (-dt / 0.4).exp()) };
        if target < self.floor + 0.06 {
            self.rang = false;
        }
        if !self.rang && target > self.floor + 0.2 && target > 0.3 && self.ripples.last().is_none_or(|r| r.0 > 0.16) {
            self.ripples.push((0.0, target));
            self.rang = true;
        }
        for r in self.ripples.iter_mut() {
            r.0 += dt;
        }
        self.ripples.retain(|r| r.0 < RIPPLE_LIFE);
        if self.ripples.len() > 5 {
            self.ripples.remove(0);
        }
    }

    /// Still showing something.
    pub fn busy(&self) -> bool {
        const E: f32 = 0.004;
        self.level > E || self.peak > E || self.bands.iter().chain(&self.caps).any(|v| *v > E) || !self.ripples.is_empty()
    }
}

/// Everyone's meters.
#[derive(Default)]
pub struct Meters {
    tracks: HashMap<SessionId, Track>,
    last: Option<f64>,
    /// The made-up voice the previews in Settings show.
    demo: Track,
    demo_last: Option<f64>,
}

impl Meters {
    /// The latest readings (`Engine::voice_meters`) at `t` (seconds). True while a meter is
    /// still moving, so the window keeps drawing.
    pub fn update(&mut self, readings: Vec<(SessionId, Meter)>, t: f64) -> bool {
        let dt = self.last.map(|l| (t - l) as f32).unwrap_or(0.0).clamp(0.0, 0.1);
        self.last = Some(t);
        let mut now: HashMap<SessionId, Meter> = HashMap::new();
        for (s, m) in readings {
            now.entry(s).and_modify(|e| e.merge(&m)).or_insert(m);
        }
        for s in now.keys() {
            self.tracks.entry(*s).or_default();
        }
        let mut busy = false;
        self.tracks.retain(|s, tr| {
            tr.step(now.get(s), dt);
            let b = tr.busy();
            busy |= b;
            b || now.contains_key(s)
        });
        busy
    }

    pub fn get(&self, s: SessionId) -> Option<&Track> {
        self.tracks.get(&s)
    }

    /// The previews' made-up voice, moved on to `t`.
    pub fn demo(&mut self, t: f64) -> Track {
        let dt = self.demo_last.map(|l| (t - l) as f32).unwrap_or(0.0).clamp(0.0, 0.1);
        self.demo_last = Some(t);
        self.demo.step(demo_voice(t).as_ref(), dt);
        self.demo.clone()
    }
}

/// A made-up voice for the previews in Settings: words and a pause, with sounds deep and bright
/// moving about.
fn demo_voice(t: f64) -> Option<Meter> {
    let phrase = (t % 3.4) as f32;
    if phrase > 2.6 {
        return None;
    }
    let t = t as f32;
    let syllables = 0.5 + 0.5 * (TAU * 3.3 * t).sin();
    let swell = 0.6 + 0.4 * (0.5 + 0.5 * (TAU * 1.3 * t + 1.0).sin());
    let edges = (phrase / 0.12).min(1.0) * ((2.6 - phrase) / 0.2).min(1.0);
    let env = (syllables.powf(0.8) * swell * edges).clamp(0.0, 1.0);
    let f1 = 2.5 + 1.5 * (1.7 * t).sin();
    let f2 = 8.0 + 2.0 * (1.1 * t + 2.0).sin();
    let mut bands = [0f32; BANDS];
    for (i, b) in bands.iter_mut().enumerate() {
        let x = i as f32;
        let shape = 0.95 * (-(x - f1).powi(2) / 3.0).exp() + 0.6 * (-(x - f2).powi(2) / 4.0).exp() + 0.12;
        *b = (shape * (0.3 + 0.7 * env) * env.sqrt()).clamp(0.0, 1.0);
    }
    Some(Meter { db: -54.0 + 42.0 * env, bands })
}

/// A band's value at a place between bands (0 = the deepest, BANDS - 1 the brightest).
fn band_at(bands: &[f32; BANDS], at: f32) -> f32 {
    let at = at.clamp(0.0, (BANDS - 1) as f32);
    let i = at.floor() as usize;
    let j = (i + 1).min(BANDS - 1);
    let f = at - i as f32;
    bands[i] * (1.0 - f) + bands[j] * f
}

/// The skin's colors the meters use.
struct Colors {
    core: Color32,
    tip: Color32,
    glow: Color32,
    ok: Color32,
    hot: Color32,
    dim: Color32,
    text: Color32,
}

fn colors() -> Colors {
    Colors { core: theme::EMBER(), tip: theme::EMBER2(), glow: theme::GLOW(), ok: theme::ONLINE(), hot: theme::LIVE(), dim: theme::DIM(), text: theme::TEXT() }
}

/// The meter for `track` (none: quiet) in `rect`, in `style`. With `idle`, a quiet meter still
/// shows faintly (so you can see where it is); without, nothing shows until there's a sound.
pub fn paint(p: &egui::Painter, rect: Rect, style: Style, track: Option<&Track>, idle: bool) {
    let quiet = Track::default();
    let t = track.unwrap_or(&quiet);
    if style == Style::Off || (!idle && !t.busy()) {
        return;
    }
    let p = p.with_clip_rect(rect.expand(4.0).intersect(p.clip_rect()));
    let c = colors();
    match style {
        Style::Bars => bars(&p, rect, t, &c),
        Style::Spectrum => spectrum(&p, rect, t, &c),
        Style::Wave => wave(&p, rect, t, &c),
        Style::Dots => dots(&p, rect, t, &c),
        Style::Glow => glow(&p, rect, t, &c),
        Style::Off => {}
    }
}

/// Around a picture (Glow): a soft light that swells with the voice, and a ring going out on
/// each word. Paint it before the picture, so it's behind.
pub fn halo(p: &egui::Painter, center: Pos2, radius: f32, t: &Track) {
    let c = colors();
    if t.level > 0.01 {
        let reach = radius * (0.3 + 0.75 * t.level) + 4.0;
        glow_ring(p, center, radius * 0.92, radius + reach, c.glow.gamma_multiply(0.55 * t.level.sqrt()));
    }
    for (age, loud) in &t.ripples {
        let k = age / RIPPLE_LIFE;
        let r = radius + 2.0 + k * radius * 0.9;
        let a = (1.0 - k).powi(2) * loud * 0.8;
        p.circle_stroke(center, r, Stroke::new(0.6 + 1.6 * (1.0 - k), c.glow.gamma_multiply(a)));
    }
}

/// A soft ring of light around `center`: `color` at radius `r0`, fading out by `r1`.
fn glow_ring(p: &egui::Painter, center: Pos2, r0: f32, r1: f32, color: Color32) {
    let n = (r1 as usize).clamp(24, 96);
    let mut m = Mesh::default();
    for i in 0..=n {
        let (s, c) = (i as f32 / n as f32 * TAU).sin_cos();
        m.colored_vertex(center + vec2(c, s) * r0, color);
        m.colored_vertex(center + vec2(c, s) * r1, Color32::TRANSPARENT);
        if i > 0 {
            let k = (2 * i) as u32;
            m.add_triangle(k - 2, k - 1, k);
            m.add_triangle(k - 1, k + 1, k);
        }
    }
    p.add(Shape::mesh(m));
}

/// Bars growing out of a line through the middle, like the website's: deep sounds in the
/// middle, bright ones out to the sides.
fn bars(p: &egui::Painter, r: Rect, t: &Track, c: &Colors) {
    let bw = (r.height() * 0.16).clamp(2.0, 4.5);
    let gap = (bw * 0.8).max(1.6);
    let mut n = ((r.width() + gap) / (bw + gap)).floor().max(3.0) as usize;
    if n % 2 == 0 {
        n -= 1;
    }
    let n = n.min(25);
    let total = n as f32 * bw + (n - 1) as f32 * gap;
    let x0 = r.center().x - total / 2.0;
    let mid = r.center().y;
    let half = r.height() / 2.0 - 0.5;
    let c0 = (n - 1) as f32 / 2.0;
    for i in 0..n {
        let d = (i as f32 - c0).abs() / c0.max(1.0);
        let v = (band_at(&t.bands, 1.0 + d * 10.5) * (1.08 - 0.3 * d * d)).clamp(0.0, 1.0);
        let h = (v * half).max(bw / 2.0);
        let x = x0 + i as f32 * (bw + gap);
        let bar = Rect::from_min_max(pos2(x, mid - h), pos2(x + bw, mid + h));
        let round = CornerRadius::same((bw / 2.0).round() as u8);
        if v > 0.04 {
            p.rect_filled(bar.expand(1.4), CornerRadius::same((bw / 2.0 + 1.4).round() as u8), c.glow.gamma_multiply(0.24 * v));
        }
        // Quiet: faint dots on the line. Sounding: ember, brighter at the tips the louder.
        let lit = (v * 5.0).min(1.0);
        let col = theme::mix(c.core, c.tip, 0.25 + 0.75 * v);
        p.rect_filled(bar, round, if lit < 1.0 { c.dim.gamma_multiply(0.5 * (1.0 - lit)).blend(col.gamma_multiply(lit)) } else { col });
    }
}

/// An equalizer: a bar for each band (or pairs of them, when it's small), deep on the left,
/// rising from the bottom, green to amber to red, with caps that fall back slowly.
fn spectrum(p: &egui::Painter, r: Rect, t: &Track, c: &Colors) {
    let n = ((r.width() + 1.0) / 4.0).floor().clamp(6.0, BANDS as f32) as usize;
    let gap = if r.width() / n as f32 >= 6.0 { 2.0 } else { 1.0 };
    let bw = (r.width() - gap * (n - 1) as f32) / n as f32;
    let floor = r.bottom() - 0.5;
    let full = r.height() - 1.5;
    for i in 0..n {
        let at = i as f32 * (BANDS - 1) as f32 / (n - 1) as f32;
        let v = band_at(&t.bands, at);
        let cap = band_at(&t.caps, at);
        let x = r.left() + i as f32 * (bw + gap);
        // Its place, faintly, when it's quiet.
        p.rect_filled(Rect::from_min_max(pos2(x, floor - 1.0), pos2(x + bw, floor)), CornerRadius::ZERO, c.dim.gamma_multiply(0.4));
        if v > 0.01 {
            gradient_bar(p, x, x + bw, floor, floor - v * full, full, c);
        }
        if cap > 0.03 {
            let y = floor - cap * full;
            p.rect_filled(Rect::from_min_max(pos2(x, y - 1.3), pos2(x + bw, y)), CornerRadius::ZERO, c.text.gamma_multiply(0.85));
        }
    }
}

/// A bar from `bottom` up to `top`, colored by how high each part of it is (of `full`):
/// green, then amber, then red near the top.
fn gradient_bar(p: &egui::Painter, x0: f32, x1: f32, bottom: f32, top: f32, full: f32, c: &Colors) {
    let stops = [(0.0, c.ok), (0.55, c.tip), (0.85, c.hot), (1.0, c.hot)];
    let color_at = |f: f32| {
        let f = f.clamp(0.0, 1.0);
        let k = stops.windows(2).position(|w| f <= w[1].0).unwrap_or(stops.len() - 2);
        let (a, b) = (stops[k], stops[k + 1]);
        theme::mix(a.1, b.1, ((f - a.0) / (b.0 - a.0).max(1e-3)).clamp(0.0, 1.0))
    };
    let height = bottom - top;
    let mut m = Mesh::default();
    let mut ys: Vec<f32> = vec![0.0];
    for (s, _) in &stops[1..] {
        if s * full < height {
            ys.push(s * full);
        }
    }
    ys.push(height);
    for w in ys.windows(2) {
        let (y0, y1) = (bottom - w[0], bottom - w[1]);
        let i = m.vertices.len() as u32;
        m.colored_vertex(pos2(x0, y0), color_at(w[0] / full));
        m.colored_vertex(pos2(x1, y0), color_at(w[0] / full));
        m.colored_vertex(pos2(x1, y1), color_at(w[1] / full));
        m.colored_vertex(pos2(x0, y1), color_at(w[1] / full));
        m.add_triangle(i, i + 1, i + 2);
        m.add_triangle(i, i + 2, i + 3);
    }
    p.add(Shape::mesh(m));
}

/// Glowing waves: three lines that swell with the voice, the deep sounds making the big slow
/// one, the bright sounds the small quick ones.
fn wave(p: &egui::Painter, r: Rect, t: &Track, c: &Colors) {
    let mid = r.center().y;
    p.line_segment([pos2(r.left(), mid), pos2(r.right(), mid)], Stroke::new(1.0f32, c.dim.gamma_multiply(0.45)));
    let amp = (r.height() / 2.0 - 1.0) * t.level.powf(0.8);
    if amp < 0.25 {
        return;
    }
    let avg = |b: &[f32]| b.iter().sum::<f32>() / b.len() as f32;
    let (low, middle, high) = (avg(&t.bands[0..5]), avg(&t.bands[5..10]), avg(&t.bands[10..]));
    // (How many times it waves across, how fast it moves, how big, its color, how thick.)
    let layers = [
        (1.3 + 0.6 * middle, 1.0, 0.55 + 0.45 * low, c.tip, 1.8f32),
        (2.2 + 0.8 * high, -1.4, 0.75 * (0.45 + 0.55 * middle), c.core, 1.3),
        (3.4 + 1.0 * high, 1.9, 0.55 * (0.35 + 0.65 * high), c.glow, 1.0),
    ];
    let n = (r.width() / 2.0).clamp(24.0, 160.0) as usize;
    for (k, (waves, speed, size, col, width)) in layers.iter().enumerate().rev() {
        let phase = t.phase * speed + k as f32 * 1.7;
        let pts: Vec<Pos2> = (0..=n)
            .map(|j| {
                let u = j as f32 / n as f32;
                let taper = (PI * u).sin().powf(1.6);
                pos2(r.left() + u * r.width(), mid + amp * size * taper * (TAU * waves * u + phase).sin())
            })
            .collect();
        // A soft light between the wave and the middle line.
        let mut m = Mesh::default();
        for (j, q) in pts.iter().enumerate() {
            m.colored_vertex(*q, col.gamma_multiply(0.26));
            m.colored_vertex(pos2(q.x, mid), col.gamma_multiply(0.03));
            if j > 0 {
                let i = (2 * j) as u32;
                m.add_triangle(i - 2, i - 1, i);
                m.add_triangle(i - 1, i + 1, i);
            }
        }
        p.add(Shape::mesh(m));
        p.add(Shape::line(pts, Stroke::new(*width, *col)));
    }
}

/// A row of lights, green to amber to red, lit up to how loud it is, the loudest lately held a
/// moment.
fn dots(p: &egui::Painter, r: Rect, t: &Track, c: &Colors) {
    let d = (r.height() * 0.42).clamp(3.0, 7.0);
    let gap = (d * 0.45).max(1.5);
    let n = ((r.width() + gap) / (d + gap)).floor().clamp(4.0, 14.0) as usize;
    let total = n as f32 * d + (n - 1) as f32 * gap;
    let x0 = r.center().x - total / 2.0 + d / 2.0;
    let peak_at = (t.peak * n as f32).ceil() as usize;
    for i in 0..n {
        let at = pos2(x0 + i as f32 * (d + gap), r.center().y);
        let top = (i as f32 + 1.0) / n as f32;
        let col = if top <= 0.6 { c.ok } else if top <= 0.85 { c.tip } else { c.hot };
        let lit = ((t.level - i as f32 / n as f32) * n as f32).clamp(0.0, 1.0);
        let lit = if i + 1 == peak_at && t.peak > 0.02 { lit.max(0.85) } else { lit };
        p.circle_filled(at, d / 2.0, c.dim.gamma_multiply(0.28));
        if lit > 0.0 {
            p.circle_filled(at, d * 0.95, col.gamma_multiply(0.22 * lit));
            p.circle_filled(at, d / 2.0, col.gamma_multiply(lit));
            p.circle_filled(at - vec2(d * 0.13, d * 0.13), d * 0.16, Color32::WHITE.gamma_multiply(0.4 * lit));
        }
    }
}

/// A light like a neon tube, longer and brighter the louder, sending out a ring on each word.
fn glow(p: &egui::Painter, r: Rect, t: &Track, c: &Colors) {
    let mid = r.center();
    let round = |h: f32| CornerRadius::same((h / 2.0).clamp(0.0, 255.0) as u8);
    p.rect_filled(Rect::from_center_size(mid, vec2(r.width() * 0.9, 1.5)), crate::theme::r(1), c.dim.gamma_multiply(0.4));
    for (age, loud) in &t.ripples {
        let k = age / RIPPLE_LIFE;
        let ring = Rect::from_center_size(mid, vec2(r.width() * (0.2 + 0.85 * k), r.height() * (0.3 + 0.75 * k)));
        let a = (1.0 - k).powi(2) * loud * 0.75;
        p.rect_stroke(ring, round(ring.height()), Stroke::new(1.2f32, c.glow.gamma_multiply(a)), StrokeKind::Middle);
    }
    let lv = t.level;
    if lv < 0.01 {
        return;
    }
    let len = r.width() * (0.15 + 0.85 * lv);
    let th = (r.height() * 0.3).max(3.0);
    for (grow, alpha) in [(3.6f32, 0.05f32), (2.8, 0.07), (2.1, 0.11), (1.5, 0.18)] {
        let g = Rect::from_center_size(mid, vec2(len + th * (grow - 1.0), th * grow));
        p.rect_filled(g, round(g.height()), c.glow.gamma_multiply(alpha * (0.4 + 0.6 * lv)));
    }
    let core = Rect::from_center_size(mid, vec2(len, th));
    p.rect_filled(core, round(th), theme::mix(c.core, c.tip, lv));
    let hot = Rect::from_center_size(mid, vec2(len * 0.85, (th * 0.35).max(1.0)));
    p.rect_filled(hot, round(hot.height()), theme::mix(c.tip, Color32::WHITE, 0.55 * lv));
}

/// Settings → Voice & Video: the looks to pick from, each showing a made-up voice. Sets
/// `chosen` (`Settings::voice_meter`) when one's clicked.
pub fn picker(ui: &mut egui::Ui, meters: &mut Meters, chosen: &mut String) {
    let demo = meters.demo(ui.input(|i| i.time));
    let shown = Style::from_setting(chosen) != Style::Off;
    let look = Style::look_of(chosen);
    let mut show = shown;
    let about = "Off: no voice meters anywhere, and nothing's measured for them. The green speaking rings still show.";
    if crate::settings::toggle_row(ui, &mut show, "Show voice meters", about) {
        *chosen = if show { look.setting().to_string() } else { Style::off_setting(chosen) };
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 8.0);
        for s in Style::LOOKS {
            let (r, resp) = ui.allocate_exact_size(vec2(106.0, 76.0), Sense::click());
            let on = s == look;
            // (Turned off: the looks are faded; picking one turns them back on.)
            let mut p = ui.painter().clone();
            if !shown {
                p.set_opacity(if resp.hovered() { 0.75 } else { 0.4 });
            }
            let p = &p;
            p.rect_filled(r, crate::theme::r(8), if resp.hovered() { theme::ELEV2() } else { theme::ELEV() });
            if on {
                p.rect_stroke(r, crate::theme::r(8), Stroke::new(2.0f32, theme::EMBER()), StrokeKind::Inside);
            }
            let screen = Rect::from_min_size(r.min + vec2(9.0, 9.0), vec2(r.width() - 18.0, 36.0));
            p.rect_filled(screen, crate::theme::r(5), theme::SIDE_DARK());
            match s {
                Style::Off => {}
                Style::Glow => {
                    // A picture with its light, and the light by a name.
                    let face = pos2(screen.left() + 20.0, screen.center().y);
                    let p2 = p.with_clip_rect(screen);
                    halo(&p2, face, 9.0, &demo);
                    p2.circle_filled(face, 9.0, theme::mix(theme::EMBER(), theme::SIDE_DARK(), 0.35));
                    let tube = Rect::from_min_max(pos2(screen.left() + 38.0, screen.top() + 9.0), pos2(screen.right() - 6.0, screen.bottom() - 9.0));
                    paint(&p2, tube, s, Some(&demo), true);
                }
                _ => paint(p, screen.shrink2(vec2(7.0, 6.0)), s, Some(&demo), true),
            }
            p.text(pos2(r.center().x, r.bottom() - 15.0), Align2::CENTER_CENTER, s.name(), theme::bold_font(12.5), if on { theme::TEXT() } else { theme::MUTED() });
            let about = if shown { s.about().to_string() } else { format!("{} (click to turn voice meters back on with this look)", s.about()) };
            if resp.on_hover_text(about).clicked() {
                *chosen = s.setting().to_string();
            }
        }
    });
    ui.ctx().request_repaint_after(Duration::from_millis(30));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_meter_rises_quickly_falls_slowly_and_rings_on_words() {
        let mut m = Meters::default();
        let loud = Meter { db: -14.0, bands: [0.8; BANDS] };
        let mut t = 0.0;
        assert!(!m.update(vec![], t), "nothing to show");
        for _ in 0..6 {
            t += 0.016;
            m.update(vec![(7, loud)], t);
        }
        let tr = m.get(7).unwrap().clone();
        assert!(tr.level > 0.9, "up within 0.1 s: {}", tr.level);
        assert_eq!(tr.ripples.len(), 1, "a word started");
        // Quiet now: still coming down a moment later, gone after a while.
        t += 0.1;
        assert!(m.update(vec![], t));
        let tr = m.get(7).unwrap();
        assert!(tr.level > 0.2 && tr.level < 0.9, "{}", tr.level);
        assert!(tr.caps[0] > 0.7, "the caps hold a moment");
        for _ in 0..200 {
            t += 0.016;
            m.update(vec![], t);
        }
        assert!(m.get(7).is_none(), "forgotten once it's still");
    }

    #[test]
    fn settings_names_go_both_ways() {
        for s in Style::ALL {
            assert_eq!(Style::from_setting(s.setting()), s);
        }
        assert_eq!(Style::from_setting(""), Style::Bars);
        // Turned off, they keep their look for when they're turned back on.
        let off = Style::off_setting("wave");
        assert_eq!((Style::from_setting(&off), Style::look_of(&off)), (Style::Off, Style::Wave));
        assert_eq!(Style::off_setting(&off), off, "off twice is still off, with the same look");
        assert_eq!(Style::look_of("off"), Style::Bars);
        for (setting, on) in [("", true), ("glow", true), ("off", false), ("off:dots", false)] {
            let s = pyr_engine::config::Settings { voice_meter: setting.into(), ..Default::default() };
            assert_eq!(s.voice_meters_on(), on, "{setting}");
            assert_eq!(Style::from_setting(setting) != Style::Off, on, "{setting}");
        }
    }

    #[test]
    fn levels_are_heard_not_measured() {
        assert_eq!(level_of(-90.0), 0.0);
        assert_eq!(level_of(0.0), 1.0);
        assert!(level_of(-40.0) > 0.3 && level_of(-40.0) < 0.5);
    }
}
