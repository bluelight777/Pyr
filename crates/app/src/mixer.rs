//! The call's mixer: a mixing desk for everyone's volume, for me alone (nobody else hears what
//! I change here). Everyone in the call has a strip in their own colors (their picture's, like
//! their tile) with two faders, their voice and their stream's sound, each with a live meter
//! of what reaches me (after its fader); on the left are MY MIC (how loud I'm sent, for
//! everyone) and ALL (everything I hear). A fader is heard as it moves and kept once it's let
//! go; double-click one for 100%, or turn the mouse wheel over it.

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_2, PI};
use std::time::Duration;

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use ingle_engine::state::{AppState, CallView, ServerView};
use ingle_proto::media::track;

use crate::app::{Act, IngleApp, avatar};
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
            p.rect_filled(seg.expand2(vec2(2.0, 1.0)), CornerRadius::same(2), c.gamma_multiply(0.22 * a));
            p.rect_filled(seg, CornerRadius::same(1), c.gamma_multiply(a));
        } else {
            p.rect_filled(seg, CornerRadius::same(1), c.gamma_multiply(0.09));
        }
    }
}

/// A fader and its meter, in column `col`: `value` (0 to 2), glowing in `accent`; `dim` for
/// one with nothing coming through it right now (it still moves).
#[allow(clippy::too_many_arguments)]
fn fader(ui: &mut egui::Ui, id: egui::Id, col: Rect, value: f32, accent: Color32, led: Led, faint: bool, dim: bool, tip: &str) -> Moved {
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
    p.rect_filled(groove, CornerRadius::same(3), Color32::from_black_alpha(170));
    p.rect_stroke(groove, CornerRadius::same(3), Stroke::new(1.0, Color32::from_white_alpha(10)), StrokeKind::Outside);
    // Lit from the bottom up to the cap, in their color.
    let y = y_of(shown);
    if shown > 0.0 {
        let fill = Rect::from_min_max(pos2(gx - 1.5, y), pos2(gx + 1.5, bottom));
        p.rect_filled(fill.expand2(vec2(3.5, 0.0)), CornerRadius::same(4), accent.gamma_multiply(0.18 * fade));
        p.rect_filled(fill, CornerRadius::same(2), accent.gamma_multiply(fade));
    }
    // The meter beside it.
    let m = Rect::from_min_max(pos2(gx + 15.0, top), pos2(gx + 22.0, bottom));
    meter(&p, m, led, faint);
    // The cap: a little metal block with a line across it, lit while it's held.
    let cap = Rect::from_center_size(pos2(gx, y), vec2(32.0, 15.0));
    p.rect_filled(cap.translate(vec2(0.0, 3.0)).expand(1.0), CornerRadius::same(5), Color32::from_black_alpha(110));
    gradient_rect(&p, cap, 4.0, Color32::from_rgb(0xf4, 0xee, 0xe8), Color32::from_rgb(0x9a, 0x91, 0x89));
    p.rect_stroke(cap, CornerRadius::same(4), Stroke::new(1.0, Color32::from_black_alpha(140)), StrokeKind::Inside);
    let line = if active { accent } else { Color32::from_rgb(0x3a, 0x33, 0x2e) };
    p.line_segment([pos2(cap.left() + 4.0, y), pos2(cap.right() - 4.0, y)], Stroke::new(2.0, line));
    if resp.dragged() {
        p.rect_stroke(cap.expand(2.5), CornerRadius::same(6), Stroke::new(1.5, accent.gamma_multiply(0.7)), StrokeKind::Outside);
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
    p.rect_filled(r, CornerRadius::same(7), fill);
    if on {
        p.rect_stroke(r.expand(1.5), CornerRadius::same(8), Stroke::new(1.0, on_color.gamma_multiply(0.45)), StrokeKind::Outside);
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

impl IngleApp {
    /// The mixer panel (opened from the call's buttons).
    pub(crate) fn mixer_panel(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, call: &CallView, acts: &mut Vec<Act>) {
        let area = ui.max_rect();
        let card = Rect::from_min_max(pos2(area.left() + 16.0, area.top() + 4.0), pos2(area.right() - 16.0, area.bottom() - 6.0));
        if card.height() < 40.0 {
            return;
        }
        let p = ui.painter_at(area);
        // The desk: a dark slab with a soft light along its top.
        p.rect_filled(card.translate(vec2(0.0, 4.0)).expand(2.0), CornerRadius::same(18), Color32::from_black_alpha(70));
        gradient_rect(&p, card, 16.0, theme::mix(theme::ELEV(), theme::STAGE(), 0.15), theme::mix(theme::SIDE_DARK(), theme::STAGE(), 0.4));
        p.rect_stroke(card, CornerRadius::same(16), Stroke::new(1.0, theme::LINE()), StrokeKind::Inside);
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
        p.text(
            pos2(title.right() + 12.0, head.center().y),
            Align2::LEFT_CENTER,
            "Only you hear what you change here · double-click a fader for 100%",
            theme::font(12.0),
            theme::DIM(),
        );
        let close = Rect::from_center_size(pos2(head.right() - 10.0, head.center().y), vec2(24.0, 24.0));
        let cr = ui.interact(close, ui.id().with("mixer-close"), Sense::click());
        if cr.hovered() {
            p.rect_filled(close, CornerRadius::same(6), theme::ELEV2());
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
        let reset_w = 132.0;
        let reset = Rect::from_min_size(pos2(close.left() - 10.0 - reset_w, head.center().y - 12.0), vec2(reset_w, 24.0));
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

        // The meters: what's coming in now, eased.
        let now = ui.input(|i| i.time);
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
                        let moved = fader(ui, ui.id().with("f-mic"), fader_rect(col), v, acc, led("mic"), !transmitting, false, tip);
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
                        let moved = fader(ui, ui.id().with("f-all"), fader_rect(col), v, theme::EMBER2(), led("all"), false, false, "Everything you hear: everyone, streams and sounds");
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
                        let moved = fader(ui, ui.id().with(("f-v", session)), fader_rect(col), voice, acc, led(&format!("v:{uid}")), false, false, &format!("{name}'s voice, for you"));
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
                        let moved = fader(ui, ui.id().with(("f-s", session)), fader_rect(col), stream, acc, led(&format!("s:{uid}")), false, !live, &tip);
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

    /// A strip's top: in their colors, their picture (lit while they talk), their name, and
    /// whether they're muted or deafened.
    #[allow(clippy::too_many_arguments)]
    fn strip_head(&mut self, ui: &mut egui::Ui, r: Rect, tint: (Color32, Color32), who: Option<(&str, &str, Option<egui::TextureId>)>, speaking: bool, muted: bool, deaf: bool) {
        let p = ui.painter();
        // The strip, and its top in their colors.
        gradient_rect(p, r, 12.0, theme::mix(tint.1, Color32::BLACK, 0.55).gamma_multiply(0.55), Color32::from_black_alpha(110));
        let top = Rect::from_min_size(r.min, vec2(r.width(), HEAD_H));
        gradient_rect(p, top, 12.0, tint.0, theme::mix(tint.1, tint.0, 0.15));
        p.rect_stroke(r, CornerRadius::same(12), Stroke::new(1.0, if speaking { theme::GLOW().gamma_multiply(0.8) } else { Color32::from_white_alpha(14) }), StrokeKind::Inside);
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
