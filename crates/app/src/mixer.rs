//! The call's mixer: a mixing desk for everyone's volume, for me alone (nobody else hears what
//! I change here). Everyone in the call has a strip in their own colors (their picture's, like
//! their tile) with two faders, their voice and their stream's sound, each with a live meter
//! of what reaches me (after its fader); on the left are MY MIC (how loud I'm sent, for
//! everyone) and ALL (everything I hear). A fader is heard as it moves and kept once it's let
//! go; double-click one for 100%, or turn the mouse wheel over it.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI};
use std::time::Duration;

use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::state::{AppState, CallView, ServerView};
use pyr_proto::media::track;

use crate::app::{Act, PyrApp, avatar};
use crate::{icons, theme};

/// The panel's height, open.
pub const HEIGHT: f32 = 342.0;
/// Faders go up to 200%.
const MAX: f32 = 2.0;
/// Lights in a meter.
const LEDS: usize = 24;
/// A strip's parts, top to bottom.
const HEAD_H: f32 = 66.0;
const LABEL_H: f32 = 18.0;
const FADER_H: f32 = 132.0;
const READ_H: f32 = 18.0;
const BUTTONS_H: f32 = 30.0;
const STRIP_H: f32 = HEAD_H + LABEL_H + FADER_H + READ_H + BUTTONS_H + 10.0;
/// A fader's column (its scale, groove and meter).
const COL_W: f32 = 62.0;
const GAP: f32 = 10.0;

/// What the mixer keeps between frames: its meters (eased), and the volumes people had
/// before they were muted here.
#[derive(Default)]
pub struct MixerUi {
    leds: HashMap<String, Led>,
    /// A person's voice volume before Mute (to go back to).
    before_mute: HashMap<String, f32>,
    /// When the meters last moved (seconds).
    last: Option<f64>,
    /// Faders gliding to where Even out (or its Undo) put them.
    glide: Option<Glide>,
    /// What the faders were before Even out (for Undo), and when it was done.
    undo: Option<(f64, Vec<(String, bool, f32)>)>,
    /// The loudness everyone was evened out to (dBFS), and when: a line across the meters.
    target: Option<(f64, f32)>,
    /// What Even out did, said for a while (since when).
    note: Option<(f64, String)>,
}

/// Faders moving by themselves: each person's voice (or stream) from one volume to another.
struct Glide {
    start: f64,
    moves: Vec<(String, bool, f32, f32)>,
}

/// How long the faders take to glide into place.
const GLIDE_S: f64 = 0.9;
/// How long Undo, the target line and the note stay.
const UNDO_S: f64 = 15.0;
/// Streams sit this much under voices, so people can still be heard over a game or music.
pub const STREAM_UNDER_DB: f32 = 5.0;

/// Even out: for everyone heard enough to tell how loud they usually are (`loud`: who, their
/// stream or not, dBFS as sent), the volume that makes them as loud as everyone else. The
/// loudness aimed for is the middle of everyone's voices (kept between -28 and -18 dBFS, so it
/// never ends up deafening or faint), streams a little under it. Someone I turned all the way
/// down stays down. Returns the voices' target and the volumes, rounded to whole percents.
pub fn even_out(loud: &[(String, bool, f32)], now: &dyn Fn(&str, bool) -> f32) -> (f32, Vec<(String, bool, f32)>) {
    let mut voices: Vec<f32> = loud.iter().filter(|x| !x.1).map(|x| x.2).collect();
    voices.sort_by(f32::total_cmp);
    let target = match voices.len() {
        0 => -22.0,
        n if n % 2 == 1 => voices[n / 2],
        n => (voices[n / 2 - 1] + voices[n / 2]) / 2.0,
    }
    .clamp(-28.0, -18.0);
    let plan = loud
        .iter()
        .filter(|(uid, stream, _)| now(uid, *stream) > 0.0001)
        .map(|(uid, stream, db)| {
            let aim = if *stream { target - STREAM_UNDER_DB } else { target };
            let (lo, hi) = if *stream { (0.1, MAX) } else { (0.25, MAX) };
            let v = 10f32.powf((aim - db) / 20.0).clamp(lo, hi);
            (uid.clone(), *stream, (v * 100.0).round() / 100.0)
        })
        .collect();
    (target, plan)
}

/// Eased: quick to start, gentle to land.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// A meter's reading as shown: quick up, slower down, with a peak light that waits, then
/// falls.
#[derive(Default, Clone, Copy, Debug)]
pub struct Led {
    pub level: f32,
    pub peak: f32,
    peak_at: f64,
}

impl Led {
    pub fn update(&mut self, target: f32, now: f64, dt: f32) {
        let tau = if target > self.level { 0.025 } else { 0.3 };
        self.level += (target - self.level) * (1.0 - (-dt / tau).exp());
        if self.level >= self.peak {
            self.peak = self.level;
            self.peak_at = now;
        } else if now - self.peak_at > 1.0 {
            self.peak = (self.peak - dt * 0.5).max(self.level);
        }
    }
}

/// dBFS to how far up a meter (0 to 1): -60 dB at the bottom, 0 at the top.
pub fn meter_level(db: f32) -> f32 {
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

/// A level (dBFS) after a volume of `gain` (0 to 2).
pub fn after(db: f32, gain: f32) -> f32 {
    if gain <= 0.0001 { -120.0 } else { db + 20.0 * gain.log10() }
}

/// A fader's value as shown.
pub fn percent(v: f32) -> String {
    format!("{:.0}%", (v * 100.0).round())
}

/// What a fader did this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Moved {
    No,
    /// Being dragged: heard now.
    Live(f32),
    /// Let go, reset or scrolled: heard and kept.
    Done(f32),
}

/// A fader's value where the pointer is, between `top` and `bottom` of its travel; it clicks
/// into place at 100% (within a few percent of it), like a real desk's unity mark.
pub fn value_at(y: f32, top: f32, bottom: f32) -> f32 {
    let v = ((bottom - y) / (bottom - top).max(1.0) * MAX).clamp(0.0, MAX);
    if (v - 1.0).abs() < 0.035 { 1.0 } else { (v * 100.0).round() / 100.0 }
}

/// The color a fader and its strip glow in, from someone's tile colors: bright, in their hue.
fn accent_of(tint: (Color32, Color32)) -> Color32 {
    let (h, s, _) = theme::hsl(tint.1);
    if s < 0.1 { theme::EMBER2() } else { theme::from_hsl(h, s.max(0.55), 0.62) }
}

/// A rounded rectangle filled `top` to `bottom`, straight down (`radius` on all corners).
fn gradient_rect(p: &egui::Painter, r: Rect, radius: f32, top: Color32, bottom: Color32) {
    let at = |y: f32| theme::mix(top, bottom, ((y - r.top()) / r.height().max(1.0)).clamp(0.0, 1.0));
    let rad = radius.min(r.width() / 2.0).min(r.height() / 2.0);
    let corners = [
        (pos2(r.right() - rad, r.top() + rad), -FRAC_PI_2, 0.0),
        (pos2(r.right() - rad, r.bottom() - rad), 0.0, FRAC_PI_2),
        (pos2(r.left() + rad, r.bottom() - rad), FRAC_PI_2, PI),
        (pos2(r.left() + rad, r.top() + rad), PI, PI + FRAC_PI_2),
    ];
    let mut mesh = egui::Mesh::default();
    let c = r.center();
    mesh.colored_vertex(c, at(c.y));
    for (o, a0, a1) in corners {
        for i in 0..=7 {
            let a = a0 + (a1 - a0) * i as f32 / 7.0;
            let q = o + rad * vec2(a.cos(), a.sin());
            mesh.colored_vertex(q, at(q.y));
        }
    }
    let n = mesh.vertices.len() as u32 - 1;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    p.add(egui::Shape::mesh(mesh));
}

/// A meter: `LEDS` lights from green to amber to red, lit up to `led`'s level, with its peak.
/// `faint`: what's measured isn't going anywhere (my mic while it's not sending).
fn meter(p: &egui::Painter, r: Rect, led: Led, faint: bool) {
    let gap = 1.5;
    let h = (r.height() - gap * (LEDS - 1) as f32) / LEDS as f32;
    let lit_n = (led.level * LEDS as f32).round() as usize;
    let peak_i = ((led.peak * LEDS as f32).ceil() as usize).min(LEDS);
    for i in 0..LEDS {
        let f = (i as f32 + 0.5) / LEDS as f32;
        let c = if f < 0.7 {
            Color32::from_rgb(0x5f, 0xd0, 0x6c)
        } else if f < 0.9 {
            Color32::from_rgb(0xff, 0xb3, 0x47)
        } else {
            Color32::from_rgb(0xff, 0x5a, 0x4f)
        };
        let y1 = r.bottom() - i as f32 * (h + gap);
        let seg = Rect::from_min_max(pos2(r.left(), y1 - h), pos2(r.right(), y1));
        let on = i < lit_n || (peak_i > 0 && i + 1 == peak_i && led.peak > 0.02);
        if on {
            let a = if faint { 0.45 } else { 1.0 };
            p.rect_filled(seg.expand2(vec2(2.0, 1.0)), crate::theme::r(2), c.gamma_multiply(0.22 * a));
            p.rect_filled(seg, crate::theme::r(1), c.gamma_multiply(a));
        } else {
            p.rect_filled(seg, crate::theme::r(1), c.gamma_multiply(0.09));
        }
    }
}

/// A fader and its meter, in column `col`: `value` (0 to 2), glowing in `accent`; `dim` for
/// one with nothing coming through it right now (it still moves).
#[allow(clippy::too_many_arguments)]
fn fader(ui: &mut egui::Ui, id: egui::Id, col: Rect, value: f32, accent: Color32, led: Led, faint: bool, dim: bool, mark: Option<f32>, tip: &str) -> Moved {
    let p = ui.painter().clone();
    let gx = col.left() + 30.0;
    let (top, bottom) = (col.top() + 10.0, col.bottom() - 10.0);
    let y_of = |v: f32| bottom - (v / MAX).clamp(0.0, 1.0) * (bottom - top);
    // What can be grabbed: the groove and around it.
    let grab = Rect::from_min_max(pos2(gx - 18.0, col.top()), pos2(gx + 18.0, col.bottom()));
    let resp = ui.interact(grab, id, Sense::click_and_drag());
    let mut moved = Moved::No;
    let pointer_v = || resp.interact_pointer_pos().map(|q| value_at(q.y, top, bottom));
    if resp.double_clicked() {
        moved = Moved::Done(1.0);
    } else if resp.drag_stopped() {
        moved = Moved::Done(pointer_v().unwrap_or(value));
    } else if resp.dragged() || resp.drag_started() {
        moved = pointer_v().map(Moved::Live).unwrap_or(Moved::No);
    } else if resp.clicked() {
        moved = pointer_v().map(Moved::Done).unwrap_or(Moved::No);
    }
    // The mouse wheel, a notch at a time.
    if resp.hovered() {
        let wheel = ui.input(|i| i.raw_scroll_delta.y);
        if wheel != 0.0 {
            let step = if wheel > 0.0 { 0.02 } else { -0.02 };
            moved = Moved::Done(((value + step) * 100.0).round().clamp(0.0, MAX * 100.0) / 100.0);
            ui.input_mut(|i| {
                i.raw_scroll_delta = egui::Vec2::ZERO;
                i.smooth_scroll_delta = egui::Vec2::ZERO;
            });
        }
    }
    let shown = match moved {
        Moved::Live(v) | Moved::Done(v) => v,
        Moved::No => value,
    };
    let active = resp.dragged() || resp.hovered();
    let fade = if dim { 0.5 } else { 1.0 };
    // The scale: a mark every 25%, longer at 0, 100% (the desk's unity mark) and 200%.
    for k in 0..=8 {
        let v = k as f32 * 0.25;
        let y = y_of(v);
        let major = k % 4 == 0;
        let unity = k == 4;
        let len = if unity { 12.0 } else if major { 9.0 } else { 5.0 };
        let c = if unity { theme::EMBER2() } else { theme::DIM().gamma_multiply(if major { 0.9 } else { 0.55 }) };
        p.line_segment([pos2(gx - 13.0 - len * 0.5, y), pos2(gx - 13.0 + len * 0.5, y)], Stroke::new(if unity { 1.6 } else { 1.0 }, c.gamma_multiply(fade)));
    }
    // The groove, sunk in.
    let groove = Rect::from_min_max(pos2(gx - 3.0, top - 2.0), pos2(gx + 3.0, bottom + 2.0));
    p.rect_filled(groove, crate::theme::r(3), Color32::from_black_alpha(170));
    p.rect_stroke(groove, crate::theme::r(3), Stroke::new(1.0, Color32::from_white_alpha(10)), StrokeKind::Outside);
    // Lit from the bottom up to the cap, in their color.
    let y = y_of(shown);
    if shown > 0.0 {
        let fill = Rect::from_min_max(pos2(gx - 1.5, y), pos2(gx + 1.5, bottom));
        p.rect_filled(fill.expand2(vec2(3.5, 0.0)), crate::theme::r(4), accent.gamma_multiply(0.18 * fade));
        p.rect_filled(fill, crate::theme::r(2), accent.gamma_multiply(fade));
    }
    // The meter beside it.
    let m = Rect::from_min_max(pos2(gx + 15.0, top), pos2(gx + 22.0, bottom));
    meter(&p, m, led, faint);
    // Where they usually are (after this fader): a little pointer beside the meter.
    if let Some(level) = mark.filter(|l| *l > 0.0) {
        let y = bottom - level * (bottom - top);
        let tip = pos2(m.right() + 2.0, y);
        p.add(egui::Shape::convex_polygon(
            vec![tip, pos2(tip.x + 6.0, y - 4.0), pos2(tip.x + 6.0, y + 4.0)],
            Color32::WHITE.gamma_multiply(0.85 * if dim { 0.5 } else { 1.0 }),
            Stroke::NONE,
        ));
    }
    // The cap: a little metal block with a line across it, lit while it's held.
    let cap = Rect::from_center_size(pos2(gx, y), vec2(32.0, 15.0));
    p.rect_filled(cap.translate(vec2(0.0, 3.0)).expand(1.0), crate::theme::r(5), Color32::from_black_alpha(110));
    gradient_rect(&p, cap, 4.0, Color32::from_rgb(0xf4, 0xee, 0xe8), Color32::from_rgb(0x9a, 0x91, 0x89));
    p.rect_stroke(cap, crate::theme::r(4), Stroke::new(1.0, Color32::from_black_alpha(140)), StrokeKind::Inside);
    let line = if active { accent } else { Color32::from_rgb(0x3a, 0x33, 0x2e) };
    p.line_segment([pos2(cap.left() + 4.0, y), pos2(cap.right() - 4.0, y)], Stroke::new(2.0, line));
    if resp.dragged() {
        p.rect_stroke(cap.expand(2.5), crate::theme::r(6), Stroke::new(1.5, accent.gamma_multiply(0.7)), StrokeKind::Outside);
    }
    if !resp.dragged() {
        resp.on_hover_text(tip);
    }
    moved
}

/// A fader's value now: where it's being dragged, else as it was.
fn now_at(m: Moved, was: f32) -> f32 {
    match m {
        Moved::Live(v) | Moved::Done(v) => v,
        Moved::No => was,
    }
}

/// A small switch under a strip: lit (in `on_color`) while it's on.
fn pill(ui: &mut egui::Ui, id: egui::Id, r: Rect, label: &str, on: bool, on_color: Color32, tip: &str) -> bool {
    let resp = ui.interact(r, id, Sense::click());
    let p = ui.painter();
    let (fill, text) = if on {
        (on_color.gamma_multiply(0.85), Color32::WHITE)
    } else if resp.hovered() {
        (theme::ELEV2(), theme::TEXT())
    } else {
        (Color32::from_black_alpha(90), theme::MUTED())
    };
    p.rect_filled(r, crate::theme::r(7), fill);
    if on {
        p.rect_stroke(r.expand(1.5), crate::theme::r(8), Stroke::new(1.0, on_color.gamma_multiply(0.45)), StrokeKind::Outside);
    }
    p.text(r.center(), Align2::CENTER_CENTER, label, theme::bold_font(11.0), text);
    resp.on_hover_text(tip).clicked()
}

/// Who a strip is for.
enum Strip {
    Mic,
    All,
    Person { uid: String, session: u32, streaming: bool, sound: bool, muted: bool, deaf: bool, speaking: bool },
}

impl PyrApp {
    /// The mixer panel (opened from the call's buttons).
    pub(crate) fn mixer_panel(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, call: &CallView, acts: &mut Vec<Act>) {
        let area = ui.max_rect();
        let card = Rect::from_min_max(pos2(area.left() + 16.0, area.top() + 4.0), pos2(area.right() - 16.0, area.bottom() - 6.0));
        if card.height() < 40.0 {
            return;
        }
        let p = ui.painter_at(area);
        // The desk: a dark slab with a soft light along its top.
        p.rect_filled(card.translate(vec2(0.0, 4.0)).expand(2.0), crate::theme::r(18), Color32::from_black_alpha(70));
        gradient_rect(&p, card, 16.0, theme::mix(theme::ELEV(), theme::STAGE(), 0.15), theme::mix(theme::SIDE_DARK(), theme::STAGE(), 0.4));
        p.rect_stroke(card, crate::theme::r(16), Stroke::new(1.0, theme::LINE()), StrokeKind::Inside);
        let lamp = Rect::from_center_size(pos2(card.center().x, card.top() + 1.0), vec2(card.width() * 0.5, 2.0));
        let mut glow = egui::Mesh::default();
        for (x, a) in [(lamp.left(), 0.0), (lamp.center().x, 0.55), (lamp.right(), 0.0)] {
            let c = theme::EMBER().gamma_multiply(a);
            glow.colored_vertex(pos2(x, lamp.top()), c);
            glow.colored_vertex(pos2(x, lamp.bottom()), c);
        }
        for i in 0..2u32 {
            glow.add_triangle(i * 2, i * 2 + 1, i * 2 + 2);
            glow.add_triangle(i * 2 + 1, i * 2 + 3, i * 2 + 2);
        }
        p.add(egui::Shape::mesh(glow));

        // Its name, what it's for, Reset and Close along the top.
        let head = Rect::from_min_size(card.min + vec2(18.0, 10.0), vec2(card.width() - 36.0, 26.0));
        icons::mixer(&p, Rect::from_min_size(pos2(head.left(), head.center().y - 8.0), vec2(16.0, 16.0)), theme::EMBER2());
        let title = p.text(pos2(head.left() + 24.0, head.center().y), Align2::LEFT_CENTER, "MIXER", theme::bold_font(12.5), theme::TEXT());
        // (Cut to fit left of the buttons.)
        let room = (head.right() - 300.0) - (title.right() + 12.0);
        if room > 40.0 {
            let mut job = egui::text::LayoutJob::simple(
                "Only you hear what you change here · double-click a fader for 100%".into(),
                theme::font(12.0),
                theme::DIM(),
                room,
            );
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            job.wrap.overflow_character = Some('…');
            let g = ui.fonts_mut(|f| f.layout_job(job));
            p.galley(pos2(title.right() + 12.0, head.center().y - g.size().y / 2.0), g, theme::DIM());
        }
        let close = Rect::from_center_size(pos2(head.right() - 10.0, head.center().y), vec2(24.0, 24.0));
        let cr = ui.interact(close, ui.id().with("mixer-close"), Sense::click());
        if cr.hovered() {
            p.rect_filled(close, crate::theme::r(6), theme::ELEV2());
        }
        icons::close(&p, close.shrink(6.0), if cr.hovered() { theme::TEXT() } else { theme::MUTED() });
        if cr.on_hover_text("Close the mixer").clicked() {
            self.ui.mixer_open = false;
        }

        // Who's here, as strips: me and everything first, then everyone else in the call.
        let parts = s.voice_in(call.channel_id);
        let mut strips = vec![Strip::Mic, Strip::All];
        for v in parts.iter().filter(|v| v.session != call.session) {
            strips.push(Strip::Person {
                uid: v.user_id.clone(),
                session: v.session,
                streaming: v.tracks.iter().any(|t| t.id == track::SCREEN),
                sound: v.tracks.iter().any(|t| t.id == track::SCREEN_AUDIO),
                muted: v.self_mute || v.server_mute,
                deaf: v.self_deaf || v.server_deaf,
                speaking: call.speaking.contains(&v.session),
            });
        }
        let now = ui.input(|i| i.time);
        // Even out: how loud everyone usually is, and the faders gliding there.
        let loud = self.engine.loudness();
        let loud_of = |session: u32, t: u8| loud.iter().find(|(s, tr, _)| *s == session && *tr == t).map(|x| x.2);
        self.glide_faders(now, acts);
        let even = Rect::from_min_size(pos2(close.left() - 10.0 - 104.0, head.center().y - 12.0), vec2(104.0, 24.0));
        let gliding = self.ui.mixer.glide.is_some();
        let er = ui.interact(even, ui.id().with("mixer-even"), Sense::click());
        let lit = er.hovered() || gliding;
        gradient_rect(&p, even, 8.0, if lit { theme::EMBER2() } else { theme::EMBER() }, theme::mix(theme::EMBER(), Color32::BLACK, 0.25));
        if lit {
            p.rect_stroke(even.expand(1.5), crate::theme::r(9), Stroke::new(1.0, theme::EMBER2().gamma_multiply(0.6)), StrokeKind::Outside);
        }
        // (Three bars coming level.)
        let ic = Rect::from_center_size(pos2(even.left() + 16.0, even.center().y), vec2(12.0, 12.0));
        for (i, h) in [0.55f32, 0.9, 0.7].into_iter().enumerate() {
            let k = if gliding { 0.8 } else { h };
            let x = ic.left() + 1.5 + i as f32 * 4.5;
            p.line_segment([pos2(x, ic.bottom()), pos2(x, ic.bottom() - ic.height() * k)], Stroke::new(2.4, theme::on_ember()));
        }
        p.text(pos2(even.left() + 28.0, even.center().y), Align2::LEFT_CENTER, "Even out", theme::bold_font(12.0), theme::on_ember());
        let tip = "Listens to how loud each person really talks (only while they talk) and how loud their stream is, then moves \
                   every fader so everyone sounds just as loud: streams a little under voices, so people can still be heard over \
                   them. Faders you turned all the way down stay down.";
        if er.on_hover_text(tip).clicked() && !gliding {
            self.start_even_out(st, s, call, &loud, now, acts);
        }
        let reset_w = 132.0;
        let reset = Rect::from_min_size(pos2(even.left() - 8.0 - reset_w, head.center().y - 12.0), vec2(reset_w, 24.0));
        let any_changed = strips.iter().any(|x| match x {
            Strip::Person { uid, .. } => {
                st.user_volumes.get(uid).is_some_and(|v| (v - 1.0).abs() > 0.001) || st.stream_volumes.get(uid).is_some_and(|v| (v - 1.0).abs() > 0.001)
            }
            _ => false,
        });
        if any_changed && pill(ui, ui.id().with("mixer-reset"), reset, "Everyone to 100%", false, theme::EMBER(), "Set everyone's voice and stream back to 100%") {
            for x in &strips {
                if let Strip::Person { uid, .. } = x {
                    acts.push(Act::PersonVolume { uid: uid.clone(), volume: 1.0, stream: false, save: true });
                    acts.push(Act::PersonVolume { uid: uid.clone(), volume: 1.0, stream: true, save: true });
                }
            }
        }

        // What Even out did (and Undo), for a while.
        if let Some((at, text)) = self.ui.mixer.note.clone().filter(|(at, _)| now - at < UNDO_S) {
            let fade = (1.0 - ((now - at - (UNDO_S - 1.0)) as f32).max(0.0)).clamp(0.0, 1.0);
            let g = ui.painter().layout_no_wrap(text, theme::font(12.0), theme::TEXT().gamma_multiply(fade));
            let x = (reset.left() - 14.0 - g.size().x - 52.0).max(title.right() + 12.0);
            let undo_r = Rect::from_min_size(pos2(x + g.size().x + 8.0, head.center().y - 11.0), vec2(44.0, 22.0));
            // (Over the line saying what the mixer is for.)
            p.rect_filled(Rect::from_min_max(pos2(x - 6.0, head.top()), pos2(undo_r.right() + 4.0, head.bottom())), crate::theme::r(6), theme::mix(theme::ELEV(), theme::STAGE(), 0.15));
            p.galley(pos2(x, head.center().y - g.size().y / 2.0), g, theme::TEXT());
            if self.ui.mixer.undo.is_some() && pill(ui, ui.id().with("mixer-undo"), undo_r, "Undo", false, theme::EMBER(), "Put the faders back where they were") {
                if let Some((_, before)) = self.ui.mixer.undo.take() {
                    let moves = before.into_iter().map(|(uid, stream, v)| {
                        let from = if stream { st.stream_volumes.get(&uid) } else { st.user_volumes.get(&uid) }.copied().unwrap_or(1.0);
                        (uid, stream, from, v)
                    });
                    self.ui.mixer.glide = Some(Glide { start: now, moves: moves.collect() });
                    self.ui.mixer.note = Some((now, "Put back.".into()));
                    self.ui.mixer.target = None;
                }
            }
        }

        // The meters: what's coming in now, eased.
        let dt = self.ui.mixer.last.map(|l| (now - l) as f32).unwrap_or(0.016).clamp(0.0, 0.1);
        self.ui.mixer.last = Some(now);
        let levels = self.engine.track_levels();
        let level = |session: u32, t: u8| levels.iter().find(|(s, tr, _)| *s == session && *tr == t).map(|x| x.2);
        let master = st.settings.output_volume;
        let mut loudest = -120.0f32;
        let mut targets: Vec<(String, f32)> = Vec::new();
        for x in &strips {
            if let Strip::Person { uid, session, .. } = x {
                let voice = level(*session, track::MIC).map(|db| after(db, st.user_volumes.get(uid).copied().unwrap_or(1.0))).unwrap_or(-120.0);
                let stream = level(*session, track::SCREEN_AUDIO).map(|db| after(db, st.stream_volumes.get(uid).copied().unwrap_or(1.0))).unwrap_or(-120.0);
                loudest = loudest.max(voice).max(stream);
                targets.push((format!("v:{uid}"), meter_level(voice)));
                targets.push((format!("s:{uid}"), meter_level(stream)));
            }
        }
        let mic = self.engine.my_mic_level();
        targets.push(("mic".into(), mic.map(|m| meter_level(m.0)).unwrap_or(0.0)));
        targets.push(("all".into(), meter_level(after(loudest, master))));
        for (k, t) in targets {
            self.ui.mixer.leds.entry(k).or_default().update(t, now, dt);
        }
        let leds = self.ui.mixer.leds.clone();
        let led = |k: &str| leds.get(k).copied().unwrap_or_default();
        let (my_muted, my_deaf) = self.engine.voice_prefs();

        // The strips, side by side (scrolling sideways when there are many).
        let body = Rect::from_min_max(pos2(card.left() + 14.0, head.bottom() + 10.0), pos2(card.right() - 14.0, card.bottom() - 10.0));
        let widths: Vec<f32> = strips.iter().map(|x| if matches!(x, Strip::Person { .. }) { 2.0 * COL_W + 20.0 } else { COL_W + 30.0 }).collect();
        let total: f32 = widths.iter().sum::<f32>() + GAP * (widths.len() as f32 - 1.0) + 14.0;
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::left_to_right(egui::Align::Min)));
        egui::ScrollArea::horizontal().id_salt("mixer-strips").auto_shrink([false, false])
            .scroll_source(egui::containers::scroll_area::ScrollSource { drag: false, ..Default::default() })
            .show(&mut child, |ui| {
            let (row, _) = ui.allocate_exact_size(vec2(total.max(body.width()), STRIP_H.min(body.height())), Sense::hover());
            let mut x = row.left();
            for (i, strip) in strips.iter().enumerate() {
                let w = widths[i];
                let r = Rect::from_min_size(pos2(x, row.top()), vec2(w, STRIP_H.min(row.height())));
                x += w + GAP;
                // A line between me and everyone else.
                if i == 2 {
                    let lx = r.left() - GAP / 2.0 - 2.0;
                    ui.painter().line_segment([pos2(lx, r.top() + 12.0), pos2(lx, r.bottom() - 12.0)], Stroke::new(1.0, theme::LINE()));
                    x += 4.0;
                }
                let r = if i >= 2 { r.translate(vec2(4.0, 0.0)) } else { r };
                if !ui.is_rect_visible(r) {
                    continue;
                }
                match strip {
                    Strip::Mic => {
                        let tint = self.tile_tint(s, &st.my_uid).unwrap_or((theme::TILE(), theme::TILE()));
                        let face = self.face(ui.ctx(), s, &st.my_uid);
                        let transmitting = mic.is_some_and(|m| m.1);
                        self.strip_head(ui, r, tint, Some((&st.my_uid, "You", face)), transmitting, my_muted, false);
                        let col = column(r, 0);
                        label(ui, col, "MY MIC", false, theme::MUTED());
                        let acc = accent_of(tint);
                        let v = st.settings.input_gain;
                        let tip = "How loud your microphone is sent (everyone hears this one)";
                        let moved = fader(ui, ui.id().with("f-mic"), fader_rect(col), v, acc, led("mic"), !transmitting, false, None, tip);
                        match moved {
                            Moved::Live(g) => acts.push(Act::InputGain { gain: g, save: false }),
                            Moved::Done(g) => acts.push(Act::InputGain { gain: g, save: true }),
                            Moved::No => {}
                        }
                        readout(ui, col, now_at(moved, v));
                        let b = buttons_rect(r).shrink2(vec2(10.0, 0.0));
                        if pill(ui, ui.id().with("p-mic"), b, if my_muted { "Muted" } else { "Mute" }, my_muted, theme::LIVE(), "Mute or unmute your microphone") {
                            acts.push(Act::Mute(!my_muted));
                        }
                    }
                    Strip::All => {
                        let tint = (theme::mix(theme::EMBER(), Color32::BLACK, 0.86), theme::mix(theme::EMBER(), Color32::BLACK, 0.62));
                        self.strip_head(ui, r, tint, None, false, false, my_deaf);
                        let col = column(r, 0);
                        label(ui, col, "ALL", false, theme::MUTED());
                        let v = master;
                        let moved = fader(ui, ui.id().with("f-all"), fader_rect(col), v, theme::EMBER2(), led("all"), false, false, None, "Everything you hear: everyone, streams and sounds");
                        match moved {
                            Moved::Live(g) => acts.push(Act::OutputVolume { volume: g, save: false }),
                            Moved::Done(g) => acts.push(Act::OutputVolume { volume: g, save: true }),
                            Moved::No => {}
                        }
                        readout(ui, col, now_at(moved, v));
                        let b = buttons_rect(r).shrink2(vec2(10.0, 0.0));
                        if pill(ui, ui.id().with("p-all"), b, if my_deaf { "Deafened" } else { "Deafen" }, my_deaf, theme::LIVE(), "Hear nobody (and mute yourself), like Discord's") {
                            acts.push(Act::Deafen(!my_deaf));
                        }
                    }
                    Strip::Person { uid, session, streaming, sound, muted, deaf, speaking } => {
                        let name = s.member_name(uid);
                        let tint = self.tile_tint(s, uid).unwrap_or((theme::TILE(), theme::TILE()));
                        let face = self.face_moving(ui.ctx(), s, uid, *speaking);
                        self.strip_head(ui, r, tint, Some((uid, &name, face)), *speaking, *muted, *deaf);
                        let acc = accent_of(tint);
                        let voice = st.user_volumes.get(uid).copied().unwrap_or(1.0);
                        let stream = st.stream_volumes.get(uid).copied().unwrap_or(1.0);
                        // Their voice.
                        let col = column(r, 0);
                        label(ui, col, "VOICE", false, theme::MUTED());
                        let mark = loud_of(*session, track::MIC).map(|db| meter_level(after(db, voice)));
                        let tip = match loud_of(*session, track::MIC) {
                            Some(db) => format!("{name}'s voice, for you. They usually talk at {db:.0} dB (the white pointer, after this fader)."),
                            None => format!("{name}'s voice, for you"),
                        };
                        let moved = fader(ui, ui.id().with(("f-v", session)), fader_rect(col), voice, acc, led(&format!("v:{uid}")), false, false, mark, &tip);
                        match moved {
                            Moved::Live(v) => acts.push(Act::PersonVolume { uid: uid.clone(), volume: v, stream: false, save: false }),
                            Moved::Done(v) => acts.push(Act::PersonVolume { uid: uid.clone(), volume: v, stream: false, save: true }),
                            Moved::No => {}
                        }
                        readout(ui, col, now_at(moved, voice));
                        // Their stream's sound (it can be set before they share).
                        let col = column(r, 1);
                        let live = *streaming && *sound;
                        label(ui, col, "STREAM", live, if live { theme::TEXT() } else { theme::DIM() });
                        let tip = if live {
                            format!("The sound of {name}'s screen share, for you")
                        } else {
                            format!("The sound of {name}'s screen share, for you (they aren't sharing any right now)")
                        };
                        let mark = loud_of(*session, track::SCREEN_AUDIO).map(|db| meter_level(after(db, stream)));
                        let moved = fader(ui, ui.id().with(("f-s", session)), fader_rect(col), stream, acc, led(&format!("s:{uid}")), false, !live, mark, &tip);
                        match moved {
                            Moved::Live(v) => acts.push(Act::PersonVolume { uid: uid.clone(), volume: v, stream: true, save: false }),
                            Moved::Done(v) => acts.push(Act::PersonVolume { uid: uid.clone(), volume: v, stream: true, save: true }),
                            Moved::No => {}
                        }
                        readout(ui, col, now_at(moved, stream));
                        // Mute them (for me), and their soundboard sounds.
                        let b = buttons_rect(r);
                        let half = (b.width() - 8.0) / 2.0;
                        let (m, sb) = (Rect::from_min_size(b.min, vec2(half, b.height())), Rect::from_min_size(b.min + vec2(half + 8.0, 0.0), vec2(half, b.height())));
                        let silenced = voice <= 0.0001;
                        if pill(ui, ui.id().with(("p-m", session)), m, if silenced { "Muted" } else { "Mute" }, silenced, theme::LIVE(), &format!("Mute {name} for you")) {
                            let back = if silenced { self.ui.mixer.before_mute.remove(uid).unwrap_or(1.0).max(0.05) } else { 0.0 };
                            if !silenced {
                                self.ui.mixer.before_mute.insert(uid.clone(), voice);
                            }
                            acts.push(Act::PersonVolume { uid: uid.clone(), volume: back, stream: false, save: true });
                        }
                        let sb_off = st.settings.soundboard_muted.contains(uid);
                        let tip = if sb_off { format!("Hear {name}'s soundboard sounds again") } else { format!("Mute {name}'s soundboard sounds, for you") };
                        if pill(ui, ui.id().with(("p-sb", session)), sb, "Sounds", !sb_off, theme::mix(theme::EMBER(), Color32::BLACK, 0.25), &tip) {
                            acts.push(Act::MuteSoundboard { uid: uid.clone(), muted: !sb_off });
                        }
                    }
                }
            }
        });
        // Where everyone was evened out to: a line across the meters, fading away.
        if let Some((at, target)) = self.ui.mixer.target.filter(|(at, _)| now - at < UNDO_S) {
            let fade = (1.0 - ((now - at) as f32 - 4.0).max(0.0) / 3.0).clamp(0.0, 1.0);
            if fade > 0.0 && strips.len() > 2 {
                let fr = fader_rect(column(Rect::from_min_size(body.min, vec2(10.0, STRIP_H)), 0));
                let (top, bottom) = (fr.top() + 10.0, fr.bottom() - 10.0);
                let y = bottom - meter_level(target) * (bottom - top);
                let x0 = body.left() + 2.0 * (COL_W + 30.0) + 2.0 * GAP + 8.0;
                let x1 = body.right().min(body.left() + total);
                // It sweeps across when it comes, glows, and a spark runs along it now and then.
                let age = (now - at) as f32;
                let reach = ease((age / 0.45).min(1.0));
                let xe = x0 + (x1 - x0) * reach;
                let hot = theme::EMBER2();
                let core = theme::mix(hot, Color32::WHITE, 0.35);
                // Soft light either side of it, and a bright core: one smooth band (a mesh, so no
                // seams), fading in at both ends.
                let glow_c = theme::EMBER();
                let ends = |t: f32| (t.min(1.0 - t) * 7.0).clamp(0.0, 1.0);
                let band = |half: f32, col: Color32, alpha: f32| {
                    let mut m = egui::Mesh::default();
                    let n = 40;
                    for k in 0..=n {
                        let t = k as f32 / n as f32;
                        let x = x0 + (xe - x0) * t;
                        let a = alpha * fade * ends(t);
                        m.colored_vertex(pos2(x, y - half), Color32::TRANSPARENT);
                        m.colored_vertex(pos2(x, y), col.gamma_multiply(a));
                        m.colored_vertex(pos2(x, y + half), Color32::TRANSPARENT);
                    }
                    for k in 0..n as u32 {
                        let (a, b) = (3 * k, 3 * (k + 1));
                        m.add_triangle(a, a + 1, b + 1);
                        m.add_triangle(a, b, b + 1);
                        m.add_triangle(a + 1, a + 2, b + 2);
                        m.add_triangle(a + 1, b + 1, b + 2);
                    }
                    p.add(egui::Shape::mesh(m));
                };
                band(14.0, glow_c, 0.22);
                band(6.0, hot, 0.55);
                band(1.6, core, 1.0);
                // The spark: every couple of seconds a bright comet runs left to right.
                let run = ((age - 0.45).max(0.0) % 2.2) / 0.9;
                if run < 1.0 && reach >= 1.0 {
                    let sx = x0 + (x1 - x0) * ease(run);
                    let tail = 70.0;
                    let mut m = egui::Mesh::default();
                    let n = 16;
                    for k in 0..=n {
                        let t = k as f32 / n as f32;
                        let x = sx - tail * (1.0 - t);
                        let h = 0.6 + 2.4 * t;
                        let c = theme::mix(hot, Color32::WHITE, 0.6).gamma_multiply(t * t * fade);
                        m.colored_vertex(pos2(x, y - h), c);
                        m.colored_vertex(pos2(x, y + h), c);
                    }
                    for k in 0..n as u32 {
                        m.add_triangle(2 * k, 2 * k + 1, 2 * k + 3);
                        m.add_triangle(2 * k, 2 * k + 2, 2 * k + 3);
                    }
                    p.add(egui::Shape::mesh(m));
                    for (r, alpha) in [(12.0, 0.08), (6.0, 0.25), (2.6, 1.0)] {
                        p.circle_filled(pos2(sx, y), r, theme::mix(hot, Color32::WHITE, 0.7).gamma_multiply(alpha * fade));
                    }
                }
                // Notches where it crosses each person's faders: the level they were set to.
                let mut sx0 = body.left();
                let mut spots = Vec::new();
                for (i, strip) in strips.iter().enumerate() {
                    let w = if matches!(strip, Strip::Person { .. }) { 2.0 * COL_W + 20.0 } else { COL_W + 30.0 };
                    if let Strip::Person { streaming, .. } = strip {
                        let r = Rect::from_min_size(pos2(sx0, body.top()), vec2(w, STRIP_H));
                        spots.push(column(r, 0).center().x);
                        if *streaming {
                            spots.push(column(r, 1).center().x);
                        }
                    }
                    let _ = i;
                    sx0 += w + GAP;
                }
                for cx in spots {
                    if cx > xe {
                        continue;
                    }
                    let d = 4.0;
                    let pts = vec![pos2(cx, y - d), pos2(cx + d, y), pos2(cx, y + d), pos2(cx - d, y)];
                    p.add(egui::Shape::convex_polygon(pts, core.gamma_multiply(fade), Stroke::new(1.0, hot.gamma_multiply(0.6 * fade))));
                }
                // Its label: a little glowing tag at the end.
                if reach >= 1.0 {
                    let text = format!("evened out \u{2248} {target:.0} dB");
                    let galley = p.layout_no_wrap(text, theme::bold_font(10.5), Color32::WHITE);
                    let tag = Rect::from_min_size(pos2(x1 - galley.size().x - 18.0, y - galley.size().y - 14.0), galley.size() + vec2(14.0, 6.0));
                    p.rect_filled(tag.expand(2.0), theme::r(8), hot.gamma_multiply(0.15 * fade));
                    p.rect_filled(tag, theme::r(7), theme::mix(Color32::BLACK, hot, 0.22).gamma_multiply(0.92 * fade));
                    p.rect_stroke(tag, theme::r(7), Stroke::new(1.0, hot.gamma_multiply(0.8 * fade)), egui::StrokeKind::Inside);
                    p.galley(tag.min + vec2(7.0, 3.0), galley, core.gamma_multiply(fade));
                }
            }
        }
        if strips.len() == 2 {
            p.text(
                pos2(body.left() + 2.0 * (COL_W + 30.0) + GAP + 40.0, body.top() + STRIP_H / 2.0),
                Align2::LEFT_CENTER,
                "When others join the call, each gets a strip here: their voice and their stream, each with its own fader.",
                theme::font(13.0),
                theme::DIM(),
            );
        }
        ui.ctx().request_repaint_after(Duration::from_millis(33));
    }

    /// Moves the gliding faders a step (heard as they go), and keeps them where they land.
    fn glide_faders(&mut self, now: f64, acts: &mut Vec<Act>) {
        let Some(g) = &self.ui.mixer.glide else { return };
        let t = ((now - g.start) / GLIDE_S) as f32;
        let done = t >= 1.0;
        let k = ease(t);
        for (uid, stream, from, to) in &g.moves {
            let v = ((from + (to - from) * k) * 100.0).round() / 100.0;
            acts.push(Act::PersonVolume { uid: uid.clone(), volume: if done { *to } else { v }, stream: *stream, save: done });
        }
        if done {
            self.ui.mixer.glide = None;
        }
    }

    /// Even out, started: works out where every fader goes (see `even_out`) and sets them
    /// gliding there, keeping where they were for Undo.
    fn start_even_out(&mut self, st: &AppState, s: &ServerView, call: &CallView, loud: &[(u32, u8, f32)], now: f64, acts: &mut Vec<Act>) {
        let others: Vec<_> = s.voice_in(call.channel_id).into_iter().filter(|v| v.session != call.session).collect();
        let mut measured: Vec<(String, bool, f32)> = Vec::new();
        let mut unheard: Vec<String> = Vec::new();
        let mut muted: Vec<String> = Vec::new();
        for v in &others {
            // Someone muted can't be heard now: their voice fader stays where it is (what was
            // measured of them may be from long ago, or from before something changed; it
            // turned a muted friend up to 188%).
            if v.self_mute || v.server_mute {
                muted.push(s.member_name(&v.user_id));
            } else {
                match loud.iter().find(|(se, t, _)| *se == v.session && *t == track::MIC) {
                    Some(x) => measured.push((v.user_id.clone(), false, x.2)),
                    None => unheard.push(s.member_name(&v.user_id)),
                }
            }
            if let Some(x) = loud.iter().find(|(se, t, _)| *se == v.session && *t == track::SCREEN_AUDIO) {
                measured.push((v.user_id.clone(), true, x.2));
            }
        }
        let vol = |uid: &str, stream: bool| if stream { st.stream_volumes.get(uid) } else { st.user_volumes.get(uid) }.copied().unwrap_or(1.0);
        let (target, plan) = even_out(&measured, &vol);
        if plan.is_empty() {
            let text = if others.is_empty() {
                "Nobody else is here to even out yet.".to_string()
            } else {
                "Nobody has talked long enough to tell how loud they are yet: give it a few seconds of talking.".to_string()
            };
            acts.push(Act::Toast(text, false));
            return;
        }
        let voices = plan.iter().filter(|x| !x.1).count();
        let streams = plan.len() - voices;
        let text = match (voices, streams) {
            (v, 0) => format!("Evened out {v} {}.", if v == 1 { "voice" } else { "voices" }),
            (0, n) => format!("Evened out {n} {}.", if n == 1 { "stream" } else { "streams" }),
            (v, n) => format!("Evened out {v} {} and {n} {}.", if v == 1 { "voice" } else { "voices" }, if n == 1 { "stream" } else { "streams" }),
        };
        // Someone a fader can't bring all the way: said, so it isn't a mystery.
        let short = plan.iter().filter(|(_, stream, _)| !stream).find_map(|(uid, _, v)| {
            let db = measured.iter().find(|m| m.0 == *uid && !m.1)?.2;
            let off = after(db, *v) - target;
            (off.abs() > 2.0).then(|| (s.member_name(uid), off < 0.0))
        });
        // (The rest, said in a note of its own: the mixer's top only has room for a line.)
        let mut more = String::new();
        match short {
            Some((name, true)) => more.push_str(&format!("{name} is still quieter than the rest: 200% is as far as a fader goes (their microphone is turned down low). ")),
            Some((name, false)) => more.push_str(&format!("{name} is still louder than the rest: Even out doesn't go below 25%. ")),
            None => {}
        }
        match unheard.as_slice() {
            [] => {}
            [one] => more.push_str(&format!("{one} hasn't talked enough to tell how loud they are yet.")),
            many => more.push_str(&format!("{} people haven't talked enough to tell how loud they are yet.", many.len())),
        }
        match muted.as_slice() {
            [] => {}
            [one] => more.push_str(&format!(" {one} is muted, so their voice was left as it is.")),
            many => more.push_str(&format!(" {} people are muted, so their voices were left as they are.", many.len())),
        }
        if !more.is_empty() {
            acts.push(Act::Toast(more.trim().to_string(), false));
        }
        self.ui.mixer.undo = Some((now, plan.iter().map(|(u, stream, _)| (u.clone(), *stream, vol(u, *stream))).collect()));
        self.ui.mixer.glide = Some(Glide { start: now, moves: plan.into_iter().map(|(u, stream, to)| (u.clone(), stream, vol(&u, stream), to)).collect() });
        self.ui.mixer.target = Some((now, target));
        self.ui.mixer.note = Some((now, text));
        self.glide_faders(now, acts);
    }

    /// A strip's top: in their colors, their picture (lit while they talk), their name, and
    /// whether they're muted or deafened.
    #[allow(clippy::too_many_arguments)]
    fn strip_head(&mut self, ui: &mut egui::Ui, r: Rect, tint: (Color32, Color32), who: Option<(&str, &str, Option<egui::TextureId>)>, speaking: bool, muted: bool, deaf: bool) {
        let p = ui.painter();
        // The strip, and its top in their colors.
        gradient_rect(p, r, 12.0, theme::mix(tint.1, Color32::BLACK, 0.55).gamma_multiply(0.55), Color32::from_black_alpha(110));
        let top = Rect::from_min_size(r.min, vec2(r.width(), HEAD_H));
        gradient_rect(p, top, 12.0, tint.0, theme::mix(tint.1, tint.0, 0.15));
        p.rect_stroke(r, crate::theme::r(12), Stroke::new(1.0, if speaking { theme::GLOW().gamma_multiply(0.8) } else { Color32::from_white_alpha(14) }), StrokeKind::Inside);
        let c = pos2(r.center().x, top.top() + 25.0);
        match who {
            Some((uid, name, face)) => {
                avatar(p, c, 18.0, uid, name, speaking, tint.1, face);
                let w = r.width() - 14.0;
                let mut job = egui::text::LayoutJob::simple(name.to_string(), theme::bold_font(12.0), theme::TEXT(), w);
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                job.wrap.overflow_character = Some('…');
                job.halign = egui::Align::Center;
                let g = ui.fonts_mut(|f| f.layout_job(job));
                p.galley(pos2(r.center().x, top.top() + 46.0), g, theme::TEXT());
            }
            None => {
                p.circle_filled(c, 18.0, Color32::from_black_alpha(80));
                p.circle_stroke(c, 18.0, Stroke::new(1.5, theme::EMBER2().gamma_multiply(0.7)));
                icons::speaker(p, Rect::from_center_size(c, vec2(18.0, 18.0)), theme::EMBER2());
                p.text(pos2(r.center().x, top.top() + 53.0), Align2::CENTER_CENTER, "Everyone", theme::bold_font(12.0), theme::TEXT());
            }
        }
        // Muted or deafened (themselves, or by the server): in the corner.
        let corner = Rect::from_min_size(pos2(r.right() - 22.0, r.top() + 6.0), vec2(16.0, 16.0));
        if deaf {
            p.circle_filled(corner.center(), 10.0, Color32::from_black_alpha(140));
            icons::headphones(p, corner.shrink(1.0), theme::LIVE(), true);
        } else if muted {
            p.circle_filled(corner.center(), 10.0, Color32::from_black_alpha(140));
            icons::mic(p, corner.shrink(1.0), theme::LIVE(), true);
        }
    }
}

/// A strip's `i`th fader column.
fn column(r: Rect, i: usize) -> Rect {
    let x = r.left() + 10.0 + i as f32 * COL_W;
    Rect::from_min_max(pos2(x, r.top() + HEAD_H + 4.0), pos2(x + COL_W, r.top() + HEAD_H + 4.0 + LABEL_H + FADER_H + READ_H))
}

/// Where a column's fader goes.
fn fader_rect(col: Rect) -> Rect {
    Rect::from_min_size(col.min + vec2(0.0, LABEL_H), vec2(col.width(), FADER_H))
}

/// Where a strip's switches go.
fn buttons_rect(r: Rect) -> Rect {
    let y = r.top() + HEAD_H + 4.0 + LABEL_H + FADER_H + READ_H + 4.0;
    Rect::from_min_size(pos2(r.left() + 10.0, y), vec2(r.width() - 20.0, BUTTONS_H - 6.0))
}

/// A column's name, over its fader (with a red dot when it's live).
fn label(ui: &egui::Ui, col: Rect, text: &str, live: bool, color: Color32) {
    let at = pos2(col.left() + 34.0, col.top() + LABEL_H / 2.0);
    let g = ui.painter().layout_no_wrap(text.to_string(), theme::bold_font(10.0), color);
    let w = g.size().x + if live { 10.0 } else { 0.0 };
    let x0 = at.x - w / 2.0;
    if live {
        ui.painter().circle_filled(pos2(x0 + 3.0, at.y), 3.0, theme::LIVE());
    }
    ui.painter().galley(pos2(x0 + if live { 10.0 } else { 0.0 }, at.y - g.size().y / 2.0), g, color);
}

/// A fader's value under it.
fn readout(ui: &egui::Ui, col: Rect, v: f32) {
    let c = if (v - 1.0).abs() < 0.001 { theme::MUTED() } else { theme::EMBER2() };
    ui.painter().text(pos2(col.left() + 34.0, col.bottom() - READ_H / 2.0), Align2::CENTER_CENTER, percent(v), theme::mono(11.5), c);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn faders_go_by_where_theyre_held_and_click_into_100() {
        let (top, bottom) = (0.0, 200.0);
        assert_eq!(value_at(200.0, top, bottom), 0.0);
        assert_eq!(value_at(0.0, top, bottom), 2.0);
        assert_eq!(value_at(-50.0, top, bottom), 2.0, "past the top: the most");
        assert_eq!(value_at(100.0, top, bottom), 1.0);
        assert_eq!(value_at(103.0, top, bottom), 1.0, "close to 100%: 100%");
        assert_eq!(value_at(150.0, top, bottom), 0.5);
        assert_eq!(percent(0.5), "50%");
        assert_eq!(percent(1.234), "123%");
    }

    #[test]
    fn even_out_makes_everyone_as_loud_as_the_middle_voice() {
        let loud = vec![
            ("quiet".to_string(), false, -32.0),
            ("middle".to_string(), false, -22.0),
            ("loud".to_string(), false, -12.0),
            ("loud".to_string(), true, -10.0),
            ("muted".to_string(), false, -20.0),
        ];
        let muted = |uid: &str, _: bool| if uid == "muted" { 0.0 } else { 1.0 };
        let (target, plan) = even_out(&loud, &muted);
        // Four voices: the middle of -32, -22, -20 and -12.
        assert!((target + 21.0).abs() < 0.01, "{target}");
        let get = |uid: &str, stream: bool| plan.iter().find(|x| x.0 == uid && x.1 == stream).map(|x| x.2);
        // 11 dB up is more than 200%: as far as it goes.
        assert_eq!(get("quiet", false), Some(2.0));
        // 1 dB up and 9 dB down.
        assert_eq!(get("middle", false), Some(1.12));
        assert_eq!(get("loud", false), Some(0.35));
        // Their stream: 5 dB under the voices (-26 dB from -10: 16 dB down).
        assert_eq!(get("loud", true), Some(0.16));
        // Someone turned all the way down stays down.
        assert_eq!(get("muted", false), None);
        // After it, everyone's meters point at about the same place.
        for (uid, stream, db) in &loud {
            if let Some(v) = get(uid, *stream).filter(|_| *uid != "quiet") {
                let aim = if *stream { target - STREAM_UNDER_DB } else { target };
                assert!((after(*db, v) - aim).abs() < 0.3, "{uid}: {}", after(*db, v));
            }
        }
        // Everyone very loud: brought down to -18 dB, not to where they all are.
        let (t2, _) = even_out(&[("a".into(), false, -6.0), ("b".into(), false, -8.0)], &|_, _| 1.0);
        assert_eq!(t2, -18.0);
        // Nobody measured: nothing to do.
        assert!(even_out(&[], &|_, _| 1.0).1.is_empty());
    }

    #[test]
    fn meters_show_what_reaches_me() {
        assert_eq!(meter_level(-60.0), 0.0);
        assert_eq!(meter_level(0.0), 1.0);
        // Half the volume: 6 dB quieter; double: 6 dB louder; off: nothing.
        assert!((after(-20.0, 0.5) + 26.02).abs() < 0.01);
        assert!((after(-20.0, 2.0) + 13.98).abs() < 0.01);
        assert!(meter_level(after(-10.0, 0.0)) == 0.0);
        // Up at once, down slowly, the peak waiting a moment.
        let mut l = Led::default();
        l.update(0.8, 0.0, 0.1);
        assert!(l.level > 0.75, "{l:?}");
        l.update(0.0, 0.1, 0.1);
        assert!(l.level > 0.4 && l.peak > 0.75, "{l:?}");
        l.update(0.0, 2.0, 0.1);
        assert!(l.peak < 0.8, "the peak falls after a while: {l:?}");
    }
}
