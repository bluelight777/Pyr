//! Settings (the gear by my name), like Discord's: a big sheet with its pages listed down the
//! left (with a search that narrows them) and the page on the right, its title at the top and
//! a way out (or Escape). Settings are switches that slide, choices shown whole, and sliders
//! that show what they're at.
//!
//! Nothing starts again, and nothing is saved over and over, while a slider moves: volumes
//! change as they're dragged and are kept when let go of; the others are taken when let go of
//! (and the microphone takes those without stopping, see `Controls::retune`).

use std::f32::consts::{FRAC_PI_2, PI};
use std::hash::Hash;
use std::ops::RangeInclusive;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, Key, Layout, Modifiers, Rect, RichText, Sense, Stroke, StrokeKind, UiBuilder, pos2, vec2,
};
use pyr_engine::config::{CameraQuality, InputMode, NoiseSuppression, ScreenPreset, Settings};
use pyr_engine::hotkey;
use pyr_engine::state::AppState;
use pyr_proto::control::FILES_FEATURE;

use crate::app::{Act, Dialog, PyrApp, Presence, avatar};
use crate::dialogs::field_label;
use crate::update::Status;
use crate::{icons, theme};

/// A page of Settings. Its id stays the same everywhere (`Dialog::Settings`'s tab: About is
/// 2); `words` are what the search finds it by, as well as its name.
struct Page {
    id: u8,
    name: &'static str,
    group: &'static str,
    words: &'static str,
}

const PAGES: &[Page] = &[
    Page {
        id: 0,
        name: "My Account",
        group: "User settings",
        words: "account profile picture avatar photo display name nickname identity key file backup safety code vouch make sure verify shield",
    },
    Page {
        id: 1,
        name: "Voice & Video",
        group: "App settings",
        words: "voice video audio microphone mic input output device speakers headphones headset volume loudness test check push talk ptt key \
                shortcut activity sensitivity threshold automatic noise suppression isolation echo cancellation gain quality bitrate kbps qos \
                priority meter meters visualizer soundboard sounds join camera webcam screen share stream quality devices",
    },
    Page {
        id: 4,
        name: "Notifications",
        group: "App settings",
        words: "notifications sounds ping chime ring ringtone calls volume test delete messages confirm mute muting streamer mode stream streaming live twitch obs hide privacy camera webcam overlays stage layout",
    },
    Page {
        id: 5,
        name: "Appearance",
        group: "App settings",
        words: "appearance skin skins theme colors colours fonts picture background gallery moving pictures gif gifs animated animation play",
    },
    Page { id: 6, name: "Claude", group: "App settings", words: "claude connector mcp plugins add-ons addons skins make ai code" },
    Page { id: 3, name: "Windows", group: "App settings", words: "windows tray clock icon start startup boot login quit exit minimize close" },
    Page {
        id: 2,
        name: "About Pyr",
        group: "About",
        words: "about version update updates upgrade bug report error reports crash licenses credits encryption privacy",
    },
];

/// The pages there are here (Windows only on Windows).
fn pages() -> impl Iterator<Item = &'static Page> {
    PAGES.iter().filter(|p| p.id != 3 || cfg!(windows))
}

/// Whether `page` is found by what's typed in the search: each word typed starts one of its
/// words (or its name's).
fn found(page: &Page, search: &str) -> bool {
    let words: Vec<String> =
        format!("{} {}", page.name, page.words).to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_string).collect();
    search.to_lowercase().split_whitespace().all(|q| {
        let q: String = q.chars().filter(|c| c.is_alphanumeric()).collect();
        q.is_empty() || words.iter().any(|w| w.starts_with(&q))
    })
}

/// The width of the list of pages down the sheet's left.
const SIDEBAR_W: f32 = 232.0;
/// How round the sheet's corners are.
const ROUND: u8 = 14;
/// The widest a page's settings get (they read best in a column, like Discord's).
const PAGE_W: f32 = 720.0;

/// The sheet's size in a window this big: most of it, never more than fits.
pub(crate) fn sheet_size(window: egui::Vec2) -> egui::Vec2 {
    vec2((window.x - 96.0).clamp(720.0, 1100.0).min(window.x - 24.0), (window.y - 80.0).clamp(460.0, 820.0).min(window.y - 24.0))
}

/// Settings' sheet: no margin of its own (its list of pages and the page fill it).
pub(crate) fn sheet_frame() -> egui::Frame {
    egui::Frame::new().fill(theme::MAIN()).stroke(Stroke::new(1.0, theme::LINE())).corner_radius(ROUND).shadow(egui::epaint::Shadow {
        offset: [0, 18],
        blur: 60,
        spread: 0,
        color: Color32::from_black_alpha(150),
    })
}

// ------------------------------------------------------------------ the parts of a page

/// A heading over a group of settings: small capitals, with room above.
pub(crate) fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(16.0);
    ui.label(RichText::new(text.to_uppercase()).font(theme::bold_font(12.0)).color(theme::MUTED()));
    ui.add_space(4.0);
}

/// A line between groups of settings.
pub(crate) fn divider(ui: &mut egui::Ui) {
    ui.add_space(18.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(r.x_range(), r.center().y, Stroke::new(1.0, theme::LINE()));
    ui.add_space(4.0);
}

/// What a setting does, under it.
pub(crate) fn note(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(13.0).color(theme::MUTED())).wrap());
}

/// Small print, under a setting.
pub(crate) fn fine(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(theme::DIM())).wrap());
}

/// An on/off switch in `r`: the knob slides across when it changes, with a tick in it when
/// it's on and a cross when it's off.
fn paint_switch(ui: &egui::Ui, r: Rect, on: bool, hovered: bool, id: egui::Id) {
    let t = ui.ctx().animate_bool_with_time(id, on, 0.14);
    let p = ui.painter();
    let off = theme::mix(theme::ELEV2(), theme::DIM(), 0.4);
    let mut track = theme::mix(off, theme::EMBER(), t);
    if hovered {
        track = theme::mix(track, Color32::WHITE, 0.08);
    }
    p.rect_filled(r, CornerRadius::same((r.height() / 2.0) as u8), track);
    let rad = r.height() / 2.0 - 3.0;
    let x = egui::lerp(r.left() + 3.0 + rad..=r.right() - 3.0 - rad, t);
    let c = pos2(x, r.center().y);
    p.circle_filled(c + vec2(0.0, 1.0), rad, Color32::from_black_alpha(45));
    p.circle_filled(c, rad, Color32::WHITE);
    let k = rad * 0.5;
    if t > 0.5 {
        p.add(egui::Shape::line(vec![c + vec2(-k, 0.0), c + vec2(-k * 0.25, k * 0.75), c + vec2(k, -k * 0.7)], Stroke::new(2.0, track)));
    } else {
        let k = k * 0.8;
        p.line_segment([c + vec2(-k, -k), c + vec2(k, k)], Stroke::new(2.0, track));
        p.line_segment([c + vec2(-k, k), c + vec2(k, -k)], Stroke::new(2.0, track));
    }
}

/// A setting that's on or off: its name, what it does under it, and a switch on the right
/// (the whole row can be clicked). True when it was changed.
pub(crate) fn toggle_row(ui: &mut egui::Ui, on: &mut bool, title: &str, about: &str) -> bool {
    let w = ui.available_width();
    let text_w = (w - 64.0).max(120.0);
    let t = ui.painter().layout(title.to_string(), theme::font(15.0), theme::TEXT(), text_w);
    let a = (!about.is_empty()).then(|| ui.painter().layout(about.to_string(), theme::font(13.0), theme::MUTED(), text_w));
    let line = t.size().y.max(24.0);
    let h = line + a.as_ref().map(|g| 3.0 + g.size().y).unwrap_or(0.0);
    let (r, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let enabled = ui.is_enabled();
    let resp = if enabled { resp.on_hover_cursor(CursorIcon::PointingHand) } else { resp };
    let changed = enabled && resp.clicked();
    if changed {
        *on = !*on;
    }
    let p = ui.painter();
    p.galley(pos2(r.left(), r.top() + (line - t.size().y) / 2.0), t, theme::TEXT());
    if let Some(g) = a {
        p.galley(pos2(r.left(), r.top() + line + 3.0), g, theme::MUTED());
    }
    let sw = Rect::from_min_size(pos2(r.right() - 44.0, r.top() + (line - 24.0) / 2.0), vec2(44.0, 24.0));
    paint_switch(ui, sw, *on, enabled && resp.hovered(), resp.id.with("switch"));
    ui.add_space(10.0);
    changed
}

/// One of a few choices, as a row (like Discord's Input Mode): a ring that fills when it's
/// the one chosen, its name and what it means. True when it's clicked (and wasn't chosen).
pub(crate) fn choice_row(ui: &mut egui::Ui, chosen: bool, title: &str, about: &str) -> bool {
    let w = ui.available_width();
    let text_w = (w - 64.0).max(120.0);
    let t = ui.painter().layout(title.to_string(), theme::font(15.0), theme::TEXT(), text_w);
    let a = (!about.is_empty()).then(|| ui.painter().layout(about.to_string(), theme::font(13.0), theme::MUTED(), text_w));
    let inner = t.size().y + a.as_ref().map(|g| 2.0 + g.size().y).unwrap_or(0.0);
    let h = (inner + 20.0).max(44.0);
    let (r, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
    let enabled = ui.is_enabled();
    let hovered = enabled && resp.hovered();
    let p = ui.painter();
    let fill = if chosen {
        theme::mix(theme::ELEV(), theme::EMBER(), 0.10)
    } else if hovered {
        theme::ELEV()
    } else {
        theme::mix(theme::ELEV(), theme::MAIN(), 0.45)
    };
    p.rect_filled(r, crate::theme::r(8), fill);
    if chosen {
        p.rect_stroke(r, crate::theme::r(8), Stroke::new(1.0, theme::mix(theme::EMBER(), theme::ELEV(), 0.45)), StrokeKind::Inside);
    }
    let ring = pos2(r.left() + 22.0, r.center().y);
    if chosen {
        p.circle_stroke(ring, 8.0, Stroke::new(2.0, theme::EMBER()));
        p.circle_filled(ring, 4.5, theme::EMBER());
    } else {
        p.circle_stroke(ring, 8.0, Stroke::new(2.0, if hovered { theme::TEXT() } else { theme::MUTED() }));
    }
    let top = r.center().y - inner / 2.0;
    let th = t.size().y;
    p.galley(pos2(r.left() + 44.0, top), t, theme::TEXT());
    if let Some(g) = a {
        p.galley(pos2(r.left() + 44.0, top + th + 2.0), g, theme::MUTED());
    }
    ui.add_space(2.0);
    enabled && !chosen && resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

/// A few choices side by side, the one chosen lit (like a switch with more than two ways).
/// The one clicked, if one was.
pub(crate) fn segmented(ui: &mut egui::Ui, id_salt: impl Hash, chosen: usize, labels: &[&str]) -> Option<usize> {
    let id = ui.make_persistent_id(id_salt);
    let w = ui.available_width().min(600.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 38.0), Sense::hover());
    ui.painter().rect_filled(r, crate::theme::r(9), theme::SIDE());
    let seg = (w - 6.0) / labels.len() as f32;
    let mut clicked = None;
    for (i, label) in labels.iter().enumerate() {
        let s = Rect::from_min_size(pos2(r.left() + 3.0 + i as f32 * seg, r.top() + 3.0), vec2(seg, 32.0));
        let resp = ui.interact(s, id.with(i), Sense::click());
        let on = i == chosen;
        let hovered = resp.hovered() && ui.is_enabled();
        let p = ui.painter();
        if on {
            p.rect_filled(s, crate::theme::r(7), theme::EMBER());
        } else if hovered {
            p.rect_filled(s, crate::theme::r(7), theme::ELEV());
        }
        let (font, color) = if on {
            (theme::bold_font(13.5), theme::on_ember())
        } else {
            (theme::font(13.5), if hovered { theme::TEXT() } else { theme::MUTED() })
        };
        p.text(s.center(), Align2::CENTER_CENTER, *label, font, color);
        if ui.is_enabled() && !on && resp.on_hover_cursor(CursorIcon::PointingHand).clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// How a slider was moved this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Slid {
    No,
    /// Held, at this (not taken yet).
    Live(f32),
    /// Let go of (or clicked, or moved with the keyboard), at this.
    Done(f32),
}

/// The pointer on a slider (`resp`): pressed or dragged, its value is `at` where it is
/// (`Live`, kept in `id`'s temporary data); let go of, it's `Done` (also if it was let go of
/// off the window). A double click puts it back to `mark`.
fn hold(ui: &egui::Ui, resp: &egui::Response, id: egui::Id, mark: Option<f32>, at: impl Fn(f32) -> f32) -> Slid {
    let held: Option<f32> = ui.data(|d| d.get_temp(id));
    if resp.double_clicked() {
        if let Some(m) = mark {
            ui.data_mut(|d| d.remove::<f32>(id));
            return Slid::Done(m);
        }
    }
    if resp.is_pointer_button_down_on() {
        if let Some(p) = resp.interact_pointer_pos() {
            let v = at(p.x);
            if held != Some(v) {
                ui.data_mut(|d| d.insert_temp(id, v));
                return Slid::Live(v);
            }
        }
        return Slid::No;
    }
    if let Some(v) = held {
        ui.data_mut(|d| d.remove::<f32>(id));
        return Slid::Done(v);
    }
    Slid::No
}

/// The value tag over a slider's knob at `at` while it's held or pointed at.
fn value_tag(ui: &egui::Ui, id: egui::Id, at: egui::Pos2, text: &str) {
    let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, id.with("tag")));
    let g = p.layout_no_wrap(text.to_string(), theme::bold_font(12.0), theme::TEXT());
    let r = Rect::from_center_size(at - vec2(0.0, 26.0), g.size() + vec2(14.0, 8.0));
    p.rect_filled(r.translate(vec2(0.0, 1.0)), crate::theme::r(6), Color32::from_black_alpha(80));
    p.rect_filled(r, crate::theme::r(6), theme::RAIL());
    p.galley(r.center() - g.size() / 2.0, g, theme::TEXT());
}

/// A slider: the part up to its knob lit, the value beside it (and over the knob while it's
/// held), and a notch at `mark` (100% on a volume): it clicks into it when near, and a double
/// click puts it back there. The arrow keys move it a `step` at a time while it's pointed at.
/// What it says comes from `text`. While it's held its value is its own (`Live`): only when
/// it's let go of is it `Done`.
pub(crate) fn slide(
    ui: &mut egui::Ui,
    id_salt: impl Hash,
    value: f32,
    range: RangeInclusive<f32>,
    step: f32,
    mark: Option<f32>,
    text: impl Fn(f32) -> String,
) -> Slid {
    let id = ui.make_persistent_id(id_salt);
    let (lo, hi) = (*range.start(), *range.end());
    let total = ui.available_width().min(460.0);
    let mut out = Slid::No;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let (r, resp) = ui.allocate_exact_size(vec2((total - 66.0).max(80.0), 30.0), Sense::click_and_drag());
        let track = Rect::from_center_size(r.center(), vec2(r.width() - 20.0, 6.0));
        let x_of = |v: f32| track.left() + ((v - lo) / (hi - lo)).clamp(0.0, 1.0) * track.width();
        let at = |x: f32| {
            let mut v = lo + ((x - track.left()) / track.width()).clamp(0.0, 1.0) * (hi - lo);
            if step > 0.0 {
                v = lo + ((v - lo) / step).round() * step;
            }
            if let Some(m) = mark {
                if (v - m).abs() < (hi - lo) * 0.025 {
                    v = m;
                }
            }
            v.clamp(lo, hi)
        };
        let enabled = ui.is_enabled();
        if enabled {
            out = hold(ui, &resp, id, mark, at);
            // The keyboard, while it's pointed at (and nothing's being typed in).
            if out == Slid::No && resp.hovered() && ui.memory(|m| m.focused().is_none()) {
                let k = ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::ArrowRight) as i32 - i.consume_key(Modifiers::NONE, Key::ArrowLeft) as i32);
                if k != 0 {
                    out = Slid::Done((value + k as f32 * step.max((hi - lo) / 100.0)).clamp(lo, hi));
                }
            }
        }
        let held: Option<f32> = ui.data(|d| d.get_temp(id));
        let v = match out {
            Slid::Live(v) | Slid::Done(v) => v,
            Slid::No => held.unwrap_or(value),
        };
        let p = ui.painter();
        p.rect_filled(track, crate::theme::r(3), theme::mix(theme::ELEV2(), theme::MAIN(), 0.2));
        let x = x_of(v);
        p.rect_filled(Rect::from_min_max(track.min, pos2(x, track.bottom())), crate::theme::r(3), theme::EMBER());
        if let Some(m) = mark {
            let mx = x_of(m);
            p.line_segment([pos2(mx, track.top() - 5.0), pos2(mx, track.bottom() + 5.0)], Stroke::new(2.0, theme::DIM()));
        }
        let hot = enabled && (resp.hovered() || held.is_some() || resp.is_pointer_button_down_on());
        let knob = pos2(x, track.center().y);
        let rad = if hot { 10.0 } else { 9.0 };
        p.circle_filled(knob + vec2(0.0, 1.0), rad, Color32::from_black_alpha(60));
        p.circle_filled(knob, rad, Color32::WHITE);
        if hot {
            p.circle_stroke(knob, rad + 3.0, Stroke::new(2.0, theme::EMBER().gamma_multiply(0.45)));
        }
        if held.is_some() || matches!(out, Slid::Live(_)) {
            value_tag(ui, id, knob, &text(v));
        }
        let resp = if enabled { resp.on_hover_cursor(CursorIcon::PointingHand) } else { resp };
        if let (Some(m), false) = (mark, held.is_some() || matches!(out, Slid::Live(_))) {
            resp.on_hover_text(format!("Double-click for {}", text(m)));
        }
        let (vr, _) = ui.allocate_exact_size(vec2(56.0, 30.0), Sense::hover());
        ui.painter().text(vr.left_center(), Align2::LEFT_CENTER, text(v), theme::bold_font(13.5), if hot { theme::TEXT() } else { theme::BODY() });
    });
    out
}

/// "100%".
pub(crate) fn percent(v: f32) -> String {
    format!("{v:.0}%")
}

/// The sensitivity's range (dBFS), quietest to loudest.
const SENS: RangeInclusive<f32> = -80.0..=-10.0;

/// Input sensitivity, like Discord's: a bar showing how loud the microphone hears me now
/// (`level`, dBFS; None while nothing's listening), green from the knob on (what gets
/// through) and amber up to it (what doesn't), and the knob (`threshold`) to move. With `auto`
/// there's no knob: Pyr decides by itself, and the bar just shows the level.
pub(crate) fn sensitivity(ui: &mut egui::Ui, id_salt: impl Hash, threshold: f32, level: Option<f32>, auto: bool) -> Slid {
    let id = ui.make_persistent_id(id_salt);
    let (lo, hi) = (*SENS.start(), *SENS.end());
    let total = ui.available_width().min(560.0);
    let mut out = Slid::No;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        let sense = if auto || !ui.is_enabled() { Sense::hover() } else { Sense::click_and_drag() };
        let (r, resp) = ui.allocate_exact_size(vec2((total - 76.0).max(100.0), 34.0), sense);
        let bar = Rect::from_center_size(r.center(), vec2(r.width() - 12.0, 10.0));
        let x_of = |db: f32| bar.left() + ((db - lo) / (hi - lo)).clamp(0.0, 1.0) * bar.width();
        let at = |x: f32| (lo + ((x - bar.left()) / bar.width()).clamp(0.0, 1.0) * (hi - lo)).round();
        if !auto && ui.is_enabled() {
            out = hold(ui, &resp, id, None, at);
        }
        let held: Option<f32> = ui.data(|d| d.get_temp(id));
        let th = match out {
            Slid::Live(v) | Slid::Done(v) => v,
            Slid::No => held.unwrap_or(threshold),
        };
        let p = ui.painter();
        let amber = crate::app::IDLE;
        let round = crate::theme::r(5);
        if auto {
            p.rect_filled(bar, round, theme::mix(theme::ELEV2(), theme::MAIN(), 0.2));
            if let Some(l) = level {
                p.rect_filled(Rect::from_min_max(bar.min, pos2(x_of(l), bar.bottom())), round, theme::ONLINE());
            }
        } else {
            let tx = x_of(th);
            // What doesn't get through, and what does.
            p.rect_filled(Rect::from_min_max(bar.min, pos2(tx, bar.bottom())), round, theme::mix(amber, theme::MAIN(), 0.72));
            p.rect_filled(Rect::from_min_max(pos2(tx, bar.top()), bar.max), round, theme::mix(theme::ONLINE(), theme::MAIN(), 0.72));
            if let Some(l) = level {
                let lx = x_of(l);
                p.rect_filled(Rect::from_min_max(bar.min, pos2(lx.min(tx), bar.bottom())), round, amber);
                if lx > tx {
                    p.rect_filled(Rect::from_min_max(pos2(tx, bar.top()), pos2(lx, bar.bottom())), round, theme::ONLINE());
                }
            }
            // The knob: a white bar standing across it.
            let hot = resp.hovered() || held.is_some();
            let knob = Rect::from_center_size(pos2(tx, bar.center().y), vec2(if hot { 10.0 } else { 8.0 }, 26.0));
            p.rect_filled(knob.translate(vec2(0.0, 1.0)), crate::theme::r(4), Color32::from_black_alpha(70));
            p.rect_filled(knob, crate::theme::r(4), Color32::WHITE);
            if held.is_some() {
                value_tag(ui, id, pos2(tx, bar.center().y - 2.0), &format!("{th:.0} dB"));
            }
            if ui.is_enabled() {
                resp.on_hover_cursor(CursorIcon::ResizeHorizontal);
            }
        }
        let (vr, _) = ui.allocate_exact_size(vec2(64.0, 34.0), Sense::hover());
        let label = if auto { "Auto".to_string() } else { format!("{th:.0} dB") };
        ui.painter().text(vr.left_center(), Align2::LEFT_CENTER, label, theme::bold_font(13.5), theme::BODY());
    });
    out
}

/// How loud the microphone is now, as a row of little bars lighting up (like Discord's mic
/// test): green, then amber, then red for the loudest. Unlit while nothing's listening.
fn level_pills(ui: &mut egui::Ui, level: Option<f32>) {
    let w = ui.available_width().min(420.0);
    let (r, _) = ui.allocate_exact_size(vec2(w, 36.0), Sense::hover());
    let n = ((w + 3.0) / 8.0).floor().max(1.0) as usize;
    let lit = level.map(|db| (((db + 66.0) / 60.0).clamp(0.0, 1.0) * n as f32).round() as usize).unwrap_or(0);
    let p = ui.painter();
    for i in 0..n {
        let pill = Rect::from_min_size(pos2(r.left() + i as f32 * 8.0, r.center().y - 10.0), vec2(5.0, 20.0));
        let f = i as f32 / n as f32;
        let c = if i >= lit {
            theme::mix(theme::ELEV2(), theme::MAIN(), 0.2)
        } else if f < 0.7 {
            theme::ONLINE()
        } else if f < 0.88 {
            crate::app::IDLE
        } else {
            theme::LIVE()
        };
        p.rect_filled(pill, crate::theme::r(2), c);
    }
}

/// A list of devices to choose from ("Default" first), as wide as there's room for.
fn device_combo(ui: &mut egui::Ui, id: &str, value: &mut Option<String>, devices: &[(String, String)]) {
    let current = value.as_ref().and_then(|v| devices.iter().find(|d| &d.0 == v).map(|d| d.1.clone())).unwrap_or_else(|| "Default".into());
    egui::ComboBox::from_id_salt(id).selected_text(current).width(ui.available_width().min(340.0)).height(320.0).show_ui(ui, |ui| {
        crate::menu::style(ui);
        ui.selectable_value(value, None, "Default");
        for (dev_id, name) in devices {
            ui.selectable_value(value, Some(dev_id.clone()), name);
        }
    });
}

/// A page in the list down the sheet's left: its icon and name, lit when it's the one open.
fn nav_row(ui: &mut egui::Ui, open: bool, page: &Page) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let hovered = resp.hovered();
    let bg = if open {
        theme::ELEV2()
    } else if hovered {
        theme::ELEV()
    } else {
        theme::SIDE()
    };
    let p = ui.painter();
    if open {
        theme::selected(p, r, 6, bg);
        // (Campfire's way here: filled, with the accent's bar too.)
        if theme::selected_fills() {
            p.rect_filled(Rect::from_min_size(pos2(r.left(), r.center().y - 8.0), vec2(3.0, 16.0)), crate::theme::r(2), theme::EMBER());
        }
    } else if hovered {
        p.rect_filled(r, crate::theme::r(6), bg);
    }
    let fg = if open {
        theme::TEXT()
    } else if hovered {
        theme::BODY()
    } else {
        theme::MUTED()
    };
    let ir = Rect::from_center_size(pos2(r.left() + 21.0, r.center().y), vec2(18.0, 18.0));
    match page.id {
        0 => icons::person(p, ir, fg),
        1 => icons::mic(p, ir, fg, false),
        4 => icons::bell(p, ir, fg),
        5 => icons::palette_mono(p, ir.shrink(1.0), fg, bg),
        6 => icons::link(p, ir, fg),
        3 => icons::screen(p, ir, fg),
        _ => icons::info(p, ir, fg),
    }
    p.text(pos2(r.left() + 40.0, r.center().y), Align2::LEFT_CENTER, page.name, if open { theme::bold_font(14.5) } else { theme::font(14.5) }, fg);
    resp.on_hover_cursor(CursorIcon::PointingHand).clicked()
}

/// The way out of Settings (like Discord's): a ring with a cross in it, and ESC under it.
fn close_button(ui: &mut egui::Ui, c: egui::Pos2) -> bool {
    let ring = Rect::from_center_size(c, vec2(36.0, 36.0));
    let resp = ui.interact(ring, ui.id().with("settings-close"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    let hot = resp.hovered();
    let p = ui.painter();
    if hot {
        p.circle_filled(c, 18.0, theme::ELEV());
    }
    p.circle_stroke(c, 17.0, Stroke::new(2.0, if hot { theme::TEXT() } else { theme::DIM() }));
    icons::close(p, Rect::from_center_size(c, vec2(22.0, 22.0)), if hot { theme::TEXT() } else { theme::MUTED() });
    p.text(c + vec2(0.0, 30.0), Align2::CENTER_CENTER, "ESC", theme::bold_font(11.0), theme::DIM());
    resp.clicked()
}

/// A band from `top` to `bottom` in color, its top corners rounded by `round` (my profile
/// card's banner).
fn gradient_band(p: &egui::Painter, r: Rect, round: f32, top: Color32, bottom: Color32) {
    let mut pts = Vec::new();
    let n = 10;
    for i in 0..=n {
        let a = PI + FRAC_PI_2 * i as f32 / n as f32;
        pts.push(pos2(r.left() + round + round * a.cos(), r.top() + round + round * a.sin()));
    }
    for i in 0..=n {
        let a = PI + FRAC_PI_2 + FRAC_PI_2 * i as f32 / n as f32;
        pts.push(pos2(r.right() - round + round * a.cos(), r.top() + round + round * a.sin()));
    }
    pts.push(r.right_bottom());
    pts.push(r.left_bottom());
    let color = |y: f32| theme::mix(top, bottom, ((y - r.top()) / r.height()).clamp(0.0, 1.0));
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.center(), color(r.center().y));
    for q in &pts {
        mesh.colored_vertex(*q, color(q.y));
    }
    let k = pts.len() as u32;
    for i in 0..k {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % k);
    }
    p.add(egui::Shape::mesh(mesh));
}

/// My profile card's banner, in my picture's own color (like Discord's for people without a
/// banner of their own): a little brighter at the top. Greys stay grey.
fn banner(accent: Color32) -> (Color32, Color32) {
    let (h, s, _) = theme::hsl(accent);
    let s = if s < 0.08 { 0.04 } else { s.clamp(0.3, 0.75) };
    (theme::from_hsl(h, s, 0.40), theme::from_hsl(h, s * 0.9, 0.27))
}

/// My presence on my picture on the profile card: bigger than in the lists.
fn big_status(p: &egui::Painter, c: egui::Pos2, presence: Presence, bg: Color32) {
    p.circle_filled(c, 13.0, bg);
    match presence {
        Presence::Online => {
            p.circle_filled(c, 9.0, theme::ONLINE());
        }
        Presence::Idle => crate::app::moon(p, c, 9.5, bg),
        Presence::Dnd => {
            p.circle_filled(c, 9.5, crate::app::DND);
            p.line_segment([c - vec2(5.0, 0.0), c + vec2(5.0, 0.0)], Stroke::new(3.4, bg));
        }
        Presence::Offline => {
            p.circle_stroke(c, 6.5, Stroke::new(3.0, theme::DIM()));
        }
    }
}

/// Shows a file in its folder (selected, on Windows).
fn show_in_folder(path: &std::path::Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn();
    }
    #[cfg(not(windows))]
    if let Some(dir) = path.parent() {
        crate::create::open_folder(dir);
    }
}

// ------------------------------------------------------------------ the sheet

impl PyrApp {
    pub(crate) fn settings_dialog(&mut self, ui: &mut egui::Ui, st: &AppState, tab: u8, acts: &mut Vec<Act>, close: &mut bool) {
        let size = sheet_size(ui.ctx().content_rect().size());
        let (sheet, _) = ui.allocate_exact_size(size, Sense::hover());
        // Settings as they are, changed by the page below (and sent when they were).
        let mut draft = st.settings.clone();
        let before = draft.clone();
        let mut t = self.ui.settings_tab_next.take().unwrap_or(tab);
        if !pages().any(|p| p.id == t) {
            t = 1;
        }
        // Report a bug (1) or Suggest a feature (2), clicked on the About page.
        let mut about_click = 0u8;

        // The pages, down the left.
        let side = Rect::from_min_max(sheet.min + vec2(1.0, 1.0), pos2(sheet.left() + SIDEBAR_W, sheet.bottom() - 1.0));
        let r = ROUND - 1;
        ui.painter().rect_filled(side, CornerRadius { nw: r, sw: r, ne: 0, se: 0 }, theme::SIDE());
        let mut side_ui = ui.new_child(UiBuilder::new().max_rect(side.shrink2(vec2(12.0, 16.0))).layout(Layout::top_down(Align::Min)));
        self.settings_sidebar(&mut side_ui, &mut t, acts);

        // The page: its title and the way out, and what's on it (scrolling when it's long).
        let body = Rect::from_min_max(pos2(side.right(), sheet.top()), sheet.max);
        let title_y = body.top() + 44.0;
        let name = pages().find(|p| p.id == t).map(|p| p.name).unwrap_or("Settings");
        ui.painter().text(pos2(body.left() + 40.0, title_y), Align2::LEFT_CENTER, name, theme::bold_font(22.0), theme::TEXT());
        if close_button(ui, pos2(body.right() - 46.0, title_y)) {
            *close = true;
        }
        let scroll = Rect::from_min_max(pos2(body.left(), title_y + 10.0), pos2(body.right() - 4.0, body.bottom() - 4.0));
        let mut page_ui = ui.new_child(UiBuilder::new().max_rect(scroll).layout(Layout::top_down(Align::Min)));
        egui::ScrollArea::vertical().id_salt(("settings-page", t)).auto_shrink([false, false]).show(&mut page_ui, |ui| {
            egui::Frame::new().inner_margin(egui::Margin { left: 40, right: 28, top: 18, bottom: 28 }).show(ui, |ui| {
                ui.set_width((ui.available_width() - 40.0).min(PAGE_W));
                let s = ui.spacing_mut();
                s.interact_size.y = 34.0;
                s.button_padding = vec2(14.0, 7.0);
                s.item_spacing = vec2(10.0, 8.0);
                match t {
                    0 => self.account_page(ui, st, &mut draft, acts),
                    1 => self.voice_page(ui, st, &mut draft, acts),
                    4 => self.notifications_page(ui, st, &mut draft, acts),
                    5 => {
                        self.appearance_tab(ui, st, acts, close);
                        divider(ui);
                        moving_pictures(ui, &mut draft);
                    }
                    6 => self.claude_tab(ui, st, acts),
                    3 => windows_page(ui, &mut draft, acts),
                    _ => about_click = self.about_page(ui, &mut draft, acts),
                }
            });
        });
        // What's scrolled up fades under the title, rather than being cut off.
        let fade = Rect::from_min_max(scroll.min, pos2(scroll.right() - 12.0, scroll.top() + 22.0));
        let mut mesh = egui::Mesh::default();
        let solid = theme::MAIN();
        mesh.colored_vertex(fade.left_top(), solid);
        mesh.colored_vertex(fade.right_top(), solid);
        mesh.colored_vertex(fade.right_bottom(), Color32::TRANSPARENT);
        mesh.colored_vertex(fade.left_bottom(), Color32::TRANSPARENT);
        mesh.add_triangle(0, 1, 2);
        mesh.add_triangle(0, 2, 3);
        ui.painter().add(egui::Shape::mesh(mesh));
        // The sheet's edge, crisp over the list's color.
        ui.painter().rect_stroke(sheet, CornerRadius::same(ROUND), Stroke::new(1.0, theme::LINE()), StrokeKind::Inside);

        if draft != before {
            acts.push(Act::Settings(draft));
        }
        if about_click > 0 {
            if st.mic_test {
                acts.push(Act::MicTest(false));
            }
            // Replaces this dialog (not "close", which would clear it).
            self.ui.dialog = Some(if about_click == 1 { self.report_bug_dialog() } else { Dialog::SuggestIdea { title: String::new(), body: String::new() } });
            return;
        }
        if !*close {
            self.ui.dialog = Some(Dialog::Settings { tab: t });
        }
    }

    /// Settings' pages down its left (narrowed by the search at the top), and Pyr's version
    /// at the bottom (like Discord's, for a bug report: click it to copy it).
    fn settings_sidebar(&mut self, ui: &mut egui::Ui, t: &mut u8, acts: &mut Vec<Act>) {
        let full = ui.max_rect();
        let search_id = ui.make_persistent_id("settings-search");
        let (sr, _) = ui.allocate_exact_size(vec2(full.width(), 32.0), Sense::hover());
        ui.painter().rect_filled(sr, crate::theme::r(6), theme::RAIL());
        let mut box_ui = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_max(sr.min + vec2(10.0, 0.0), sr.max - vec2(30.0, 0.0))).layout(Layout::left_to_right(Align::Center)));
        let resp = box_ui.add(
            egui::TextEdit::singleline(&mut self.ui.settings_search)
                .id(search_id)
                .hint_text("Search")
                .frame(false)
                .desired_width(f32::INFINITY)
                .font(theme::font(14.0))
                .margin(egui::Margin::ZERO),
        );
        crate::textbox::menu(&mut box_ui, &resp, &mut self.ui.settings_search, true);
        let ir = Rect::from_center_size(pos2(sr.right() - 16.0, sr.center().y), vec2(16.0, 16.0));
        if self.ui.settings_search.is_empty() {
            icons::search(ui.painter(), ir, theme::DIM());
        } else {
            let x = ui.interact(ir.expand(4.0), search_id.with("clear"), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Clear");
            icons::close(ui.painter(), ir, if x.hovered() { theme::TEXT() } else { theme::MUTED() });
            if x.clicked() {
                self.ui.settings_search.clear();
            }
        }
        let search = self.ui.settings_search.trim().to_string();
        let shown: Vec<&Page> = pages().filter(|p| found(p, &search)).collect();
        // Enter: the first page found.
        if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
            if let Some(p) = shown.first() {
                *t = p.id;
            }
        }
        ui.add_space(6.0);
        let mut group = "";
        for p in &shown {
            if p.group != group {
                group = p.group;
                ui.add_space(8.0);
                let (hr, _) = ui.allocate_exact_size(vec2(full.width(), 22.0), Sense::hover());
                ui.painter().text(pos2(hr.left() + 10.0, hr.center().y), Align2::LEFT_CENTER, group.to_uppercase(), theme::bold_font(11.0), theme::DIM());
            }
            if nav_row(ui, *t == p.id, p) {
                *t = p.id;
            }
        }
        if shown.is_empty() {
            ui.add_space(14.0);
            ui.label(RichText::new("No settings match that.").size(13.0).color(theme::DIM()));
        }
        // The version, at the bottom.
        let v = format!("Pyr {}", env!("CARGO_PKG_VERSION"));
        let os = format!("{} {}", std::env::consts::OS, std::env::consts::ARCH);
        let vr = Rect::from_min_max(pos2(full.left(), full.bottom() - 40.0), full.max);
        let resp = ui.interact(vr, search_id.with("version"), Sense::click()).on_hover_text("Copy the version (for a bug report)").on_hover_cursor(CursorIcon::PointingHand);
        let copied_id = search_id.with("copied");
        let now = ui.input(|i| i.time);
        if resp.clicked() {
            acts.push(Act::Copy(format!("{v} · {os}")));
            ui.data_mut(|d| d.insert_temp(copied_id, now));
        }
        let copied = ui.data(|d| d.get_temp::<f64>(copied_id)).is_some_and(|at| now - at < 1.6);
        if copied {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        }
        let c = if resp.hovered() { theme::MUTED() } else { theme::DIM() };
        let p = ui.painter();
        p.text(pos2(vr.left() + 10.0, vr.top() + 10.0), Align2::LEFT_CENTER, &v, theme::font(12.5), c);
        if copied {
            p.text(pos2(vr.left() + 10.0, vr.top() + 28.0), Align2::LEFT_CENTER, "Copied", theme::bold_font(12.5), theme::ONLINE());
        } else {
            p.text(pos2(vr.left() + 10.0, vr.top() + 28.0), Align2::LEFT_CENTER, &os, theme::font(12.5), c);
        }
    }

    /// My Account: my profile card (my picture over a band in its colors, my name and how
    /// I show), my name, my identity, and making sure of people.
    fn account_page(&mut self, ui: &mut egui::Ui, st: &AppState, draft: &mut Settings, acts: &mut Vec<Act>) {
        ui.add_space(4.0);
        let w = ui.available_width();
        let face = self.my_face_moving(ui.ctx(), true);
        let accent = self
            .my_avatar()
            .and_then(|(k, _)| self.images.accent(&k).or_else(|| self.images.accent(&format!("{k}#moving"))))
            .unwrap_or_else(|| theme::avatar_color(&st.my_uid));
        let (top, bottom) = banner(accent);
        let presence = Presence::mine(st);
        egui::Frame::new().fill(theme::SIDE()).corner_radius(12).show(ui, |ui| {
            ui.set_width(w);
            let (band, _) = ui.allocate_exact_size(vec2(w, 96.0), Sense::hover());
            gradient_band(ui.painter(), band, 12.0, top, bottom);
            let c = pos2(band.left() + 24.0 + 46.0, band.bottom() + 4.0);
            ui.painter().circle_filled(c, 46.0 + 6.0, theme::SIDE());
            avatar(ui.painter(), c, 46.0, &st.my_uid, &st.my_name, false, theme::SIDE(), face);
            big_status(ui.painter(), c + vec2(33.0, 33.0), presence, theme::SIDE());
            // My name and how I show, beside my picture; changing the picture on the right.
            let (row, _) = ui.allocate_exact_size(vec2(w, 66.0), Sense::hover());
            let x = band.left() + 24.0 + 92.0 + 18.0;
            let mut name_ui = ui.new_child(UiBuilder::new().max_rect(Rect::from_min_max(pos2(x, row.top() + 8.0), pos2(row.right() - 290.0, row.top() + 34.0))));
            crate::emoji::label(&mut name_ui, &st.my_name, theme::bold_font(20.0), theme::TEXT(), Some(row.width() - 290.0 - x + row.left()), &mut |_| None);
            ui.painter().text(pos2(x, row.top() + 46.0), Align2::LEFT_CENTER, presence.label(), theme::font(13.0), theme::MUTED());
            let mut buttons = ui.new_child(
                UiBuilder::new().max_rect(Rect::from_min_max(pos2(row.right() - 300.0, row.top() + 6.0), pos2(row.right() - 16.0, row.top() + 44.0))).layout(Layout::right_to_left(Align::Center)),
            );
            buttons.spacing_mut().button_padding = vec2(14.0, 7.0);
            if face.is_some() && buttons.button("Remove").clicked() {
                acts.push(Act::SetAvatar(None));
            }
            let pick = buttons.add(theme::primary_button("Change picture")).on_hover_text(
                "PNG, JPEG, GIF or WebP: you can zoom in and move it before it's saved. A moving one moves while you talk. It shows on every server you're in.",
            );
            if pick.clicked() {
                crate::images::pick_async(crate::images::PickFor::Avatar, false, ui.ctx());
            }
            // The details, in a box of their own.
            egui::Frame::new().fill(theme::MAIN()).corner_radius(10).inner_margin(16).outer_margin(egui::Margin { left: 16, right: 16, top: 0, bottom: 16 }).show(
                ui,
                |ui| {
                    ui.set_width(ui.available_width());
                    let s = ui.spacing_mut();
                    s.interact_size.y = 34.0;
                    s.button_padding = vec2(14.0, 7.0);
                    field_label(ui, "Display name");
                    ui.horizontal(|ui| {
                        let typed = self.ui.name_draft.get_or_insert_with(|| st.my_name.clone());
                        let resp = ui.add(
                            egui::TextEdit::singleline(typed).hint_text("Your name").char_limit(32).desired_width(280.0).margin(egui::Margin::symmetric(10, 8)),
                        );
                        crate::textbox::menu(ui, &resp, typed, true);
                        let name = pyr_proto::control::clean_name(typed, 32).unwrap_or_default();
                        let ok = !name.is_empty() && name != st.my_name;
                        let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        if (ui.add_enabled(ok, theme::primary_button("Save")).clicked() || (enter && ok)) && ok {
                            acts.push(Act::SetName(name.clone()));
                            *typed = name;
                        }
                    });
                    fine(ui, "How you show to everyone, on every server you're in.");
                    ui.add_space(6.0);
                    field_label(ui, "Your identity");
                    note(ui, "There's no account or password. Your identity is a key stored on this computer; servers recognize you by it.");
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Safety code").color(theme::MUTED()));
                        ui.label(RichText::new(&st.safety_code).font(theme::mono(14.0)).color(theme::TEXT()));
                        if ui.small_button("Copy").clicked() {
                            acts.push(Act::Copy(st.safety_code.clone()));
                        }
                    });
                    fine(ui, "Friends can compare this code with you to be sure they're talking to the real you.");
                    ui.add_space(6.0);
                    let key = self.engine.identity_path();
                    let path = key.display().to_string();
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Key file").color(theme::MUTED()));
                        ui.add(egui::Label::new(RichText::new(&path).font(theme::mono(11.5)).color(theme::DIM())).truncate());
                    });
                    ui.horizontal(|ui| {
                        if ui.small_button("Copy path").clicked() {
                            acts.push(Act::Copy(path.clone()));
                        }
                        if ui.small_button("Show in folder").clicked() {
                            show_in_folder(&key);
                        }
                    });
                    fine(ui, "Back this file up to keep your identity if you reinstall. Don't share it.");
                },
            );
        });
        if st.servers.iter().any(|s| s.status.is_connected() && !s.has(FILES_FEATURE)) {
            ui.add_space(6.0);
            fine(ui, "Some of your servers run an older Pyr Server and won't show your picture until they're updated.");
        }
        divider(ui);
        heading(ui, "Making sure of people");
        let n = st.verified.len();
        note(
            ui,
            &match n {
                0 => "Right-click someone (or click their name in chat) and choose Make sure it's them: compare six words, or ask something only they'd know. A shield then shows by their name.".to_string(),
                1 => "You've made sure of 1 person: a shield shows by their name.".to_string(),
                n => format!("You've made sure of {n} people: a shield shows by their names."),
            },
        );
        ui.add_space(8.0);
        toggle_row(
            ui,
            &mut draft.vouch,
            "Vouch for the people I make sure of",
            "On the servers you share with them. People who made sure of you then see them as vouched for (an outlined shield): it's how a whole group gets covered without everyone checking everyone.",
        );
    }

    /// Voice & Video: devices and volumes, the mic test, input mode and sensitivity, voice
    /// processing, voice meters, the soundboard, the camera and screen sharing.
    fn voice_page(&mut self, ui: &mut egui::Ui, st: &AppState, draft: &mut Settings, acts: &mut Vec<Act>) {
        let devices = &st.devices;
        // How loud I am now (only while something's listening: the mic test, or a call).
        let level = (st.mic_test || st.call.is_some()).then_some(st.mic_level_db);
        heading(ui, "Voice settings");
        ui.columns(2, |cols| {
            field_label(&mut cols[0], "Input device");
            device_combo(&mut cols[0], "in-dev", &mut draft.input_device, &devices.inputs);
            field_label(&mut cols[1], "Output device");
            device_combo(&mut cols[1], "out-dev", &mut draft.output_device, &devices.outputs);
            field_label(&mut cols[0], "Input volume");
            // (Heard as it's dragged, kept when let go of.)
            match slide(&mut cols[0], "in-vol", st.settings.input_gain * 100.0, 0.0..=200.0, 1.0, Some(100.0), percent) {
                Slid::Live(v) => acts.push(Act::InputGain { gain: v / 100.0, save: false }),
                Slid::Done(v) => acts.push(Act::InputGain { gain: v / 100.0, save: true }),
                Slid::No => {}
            }
            field_label(&mut cols[1], "Output volume");
            match slide(&mut cols[1], "out-vol", st.settings.output_volume * 100.0, 0.0..=200.0, 1.0, Some(100.0), percent) {
                Slid::Live(v) => acts.push(Act::OutputVolume { volume: v / 100.0, save: false }),
                Slid::Done(v) => acts.push(Act::OutputVolume { volume: v / 100.0, save: true }),
                Slid::No => {}
            }
        });
        heading(ui, "Mic test");
        note(ui, "Having mic trouble? Start a test and say something: you'll hear yourself the way others hear you, with everything below applied.");
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let label = if st.mic_test { "Stop testing" } else { "Let's check" };
            if ui.add(theme::primary_button(label).min_size(vec2(124.0, 36.0))).clicked() {
                acts.push(Act::MicTest(!st.mic_test));
            }
            ui.add_space(8.0);
            level_pills(ui, level);
        });
        if st.mic_test {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }

        divider(ui);
        heading(ui, "Input mode");
        if choice_row(ui, draft.input_mode == InputMode::VoiceActivity, "Voice Activity", "Your microphone opens by itself while you talk.") {
            draft.input_mode = InputMode::VoiceActivity;
        }
        if choice_row(ui, draft.input_mode == InputMode::PushToTalk, "Push to Talk", "Your microphone opens only while you hold a key (even with a game in front).") {
            draft.input_mode = InputMode::PushToTalk;
        }
        match draft.input_mode {
            InputMode::VoiceActivity => {
                heading(ui, "Input sensitivity");
                toggle_row(
                    ui,
                    &mut draft.vad_auto,
                    "Automatically determine input sensitivity",
                    "Pyr listens for speech (and with voice isolation, for your voice) and opens your microphone for it.",
                );
                if let Slid::Done(v) = sensitivity(ui, "vad", draft.vad_threshold_db, level, draft.vad_auto) {
                    draft.vad_threshold_db = v;
                }
                if level.is_none() {
                    fine(ui, "Your level shows here while you test your mic (Let's check) or are in voice.");
                } else if !draft.vad_auto {
                    fine(ui, "Talk normally: when you speak the bar should go past the knob, into the green, and stay short of it when you don't.");
                }
            }
            InputMode::PushToTalk => {
                heading(ui, "Shortcut");
                egui::ComboBox::from_id_salt("ptt-key").selected_text(hotkey::key_label(&draft.ptt_key)).width(240.0).height(320.0).show_ui(ui, |ui| {
                    crate::menu::style(ui);
                    for k in hotkey::KEY_CHOICES {
                        ui.selectable_value(&mut draft.ptt_key, k.to_string(), hotkey::key_label(k));
                    }
                });
                fine(ui, "Works while other apps are focused, and doesn't block the key in your game.");
                heading(ui, "Release delay");
                if let Slid::Done(v) = slide(ui, "ptt-delay", draft.ptt_release_ms as f32, 0.0..=1000.0, 10.0, None, |v| format!("{v:.0} ms")) {
                    draft.ptt_release_ms = v as u32;
                }
                fine(ui, "Keeps your microphone open a moment after you let go of the key, so the end of your last word isn't cut off.");
            }
        }

        divider(ui);
        heading(ui, "Noise suppression");
        let modes = [NoiseSuppression::Off, NoiseSuppression::Standard, NoiseSuppression::Strong, NoiseSuppression::Isolation];
        let now = modes.iter().position(|m| *m == draft.noise_suppression).unwrap_or(0);
        if let Some(i) = segmented(ui, "noise", now, &["Off", "Standard", "Strong (AI)", "Voice isolation"]) {
            draft.noise_suppression = modes[i];
        }
        note(
            ui,
            match draft.noise_suppression {
                NoiseSuppression::Off => "Your microphone as it is, nothing taken out.",
                NoiseSuppression::Standard => "Takes out steady noise: fans, hum and hiss.",
                NoiseSuppression::Strong => "An AI model (RNNoise) takes out most of the noise around your voice, typing and clicks too.",
                NoiseSuppression::Isolation => {
                    "Only your voice gets through: typing, mouse clicks, fans, machines, TVs, people talking in the room and sounds between \
                     your words are cut. Someone talking right next to you as loudly as you can still be heard. Try it with Let's check."
                }
            },
        );
        heading(ui, "Voice processing");
        toggle_row(
            ui,
            &mut draft.echo_cancellation,
            "Echo cancellation",
            "Needed if you use speakers instead of headphones: it keeps what they play from going back out through your microphone. \
             With headphones it stands aside by itself.",
        );
        // What it's doing right now (in a call, or while Let's check runs).
        if draft.echo_cancellation
            && let Some(now) = self.engine.mic_echo()
        {
            fine(ui, &format!("Right now: {now}"));
        }
        toggle_row(ui, &mut draft.auto_gain, "Automatic gain control", "Keeps how loud you come across even, whether you're close to your microphone or further away.");
        toggle_row(
            ui,
            &mut draft.voice_priority,
            "Prioritize voice packets (Quality of Service)",
            "Tells your router and Wi-Fi to send Pyr's voice ahead of downloads and other traffic. Turn it off if your voice cuts out or won't connect (a few routers don't handle it).",
        );
        heading(ui, "Voice quality");
        if let Slid::Done(v) = slide(ui, "bitrate", draft.voice_bitrate_kbps as f32, 24.0..=192.0, 8.0, Some(64.0), |v| format!("{v:.0} kbps")) {
            draft.voice_bitrate_kbps = v as u32;
        }
        fine(ui, "Opus with in-band error correction. Discord's default is 64 kbps; anything above ~64 kbps is overkill for voice but great for music.");

        divider(ui);
        heading(ui, "Voice meter");
        crate::meter::picker(ui, &mut self.meters, &mut draft.voice_meter);
        fine(
            ui,
            "Shows across the voice panel (yours), by each person in your voice channel and on call tiles. It moves with any sound someone \
             sends: their voice, their screen's sound and their soundboard sounds. Right-click yours in the voice panel to change its look \
             or turn voice meters off.",
        );

        divider(ui);
        self.soundboard_settings(ui, st, draft);

        divider(ui);
        heading(ui, "Video settings");
        ui.columns(2, |cols| {
            field_label(&mut cols[0], "Camera");
            device_combo(&mut cols[0], "cam-dev", &mut draft.camera_device, &devices.cameras);
            field_label(&mut cols[1], "Camera quality");
            egui::ComboBox::from_id_salt("cam-q").selected_text(draft.camera_quality.label()).width(cols[1].available_width().min(340.0)).show_ui(&mut cols[1], |ui| {
                crate::menu::style(ui);
                for q in [CameraQuality::Low, CameraQuality::Standard, CameraQuality::High] {
                    ui.selectable_value(&mut draft.camera_quality, q, q.label());
                }
            });
        });
        field_label(ui, "Screen share quality");
        egui::ComboBox::from_id_salt("scr-q").selected_text(draft.screen_preset.label()).width(ui.available_width().min(340.0)).show_ui(ui, |ui| {
            crate::menu::style(ui);
            for q in ScreenPreset::ALL {
                ui.selectable_value(&mut draft.screen_preset, q, q.label());
            }
        });
        ui.add_space(10.0);
        if ui.button("Look for devices again").on_hover_text("For a microphone, speakers or a camera plugged in just now").clicked() {
            acts.push(Act::RefreshDevices);
        }
    }

    /// Notifications: sounds, ringing, their volume, and messages.
    fn notifications_page(&mut self, ui: &mut egui::Ui, st: &AppState, draft: &mut Settings, acts: &mut Vec<Act>) {
        heading(ui, "Sounds");
        toggle_row(
            ui,
            &mut draft.sounds,
            "Play sounds",
            "A ping for new messages (not in muted channels, or the one you're looking at), and chimes when people join or leave your voice \
             channel, start or stop sharing their screen, and when you tune in to a stream.",
        );
        toggle_row(
            ui,
            &mut draft.ring,
            "Ring when someone calls me",
            "Calls in direct messages ring (and you hear it ringing when you call), even with the other sounds off.",
        );
        ui.add_enabled_ui(draft.ring, |ui| self.ringtone_setting(ui, st, acts));
        heading(ui, "Volume");
        ui.add_enabled_ui(draft.sounds || draft.ring, |ui| {
            ui.horizontal(|ui| {
                let w = ui.available_width();
                ui.allocate_ui(vec2((w - 90.0).min(460.0), 30.0), |ui| {
                    if let Slid::Done(v) = slide(ui, "sound-vol", draft.sound_volume * 100.0, 0.0..=150.0, 1.0, Some(100.0), percent) {
                        draft.sound_volume = v / 100.0;
                    }
                });
                if ui.button("Test").on_hover_text("Hear a sound at this volume").clicked() {
                    acts.push(Act::TestSound(draft.sound_volume));
                }
            });
        });
        fine(ui, "How loud Pyr's own sounds and ringing are (not people's voices: those are under Voice & Video).");
        divider(ui);
        heading(ui, "Messages");
        toggle_row(ui, &mut draft.confirm_delete, "Ask before deleting a message", "Holding Shift when you click Delete skips the question either way.");
        heading(ui, "Muting");
        note(ui, "Right-click a channel, a server or a direct message conversation to mute it.");
        divider(ui);
        heading(ui, "Streamer mode");
        // What's in effect: on by choice, or by itself while I share my screen (the saved choice
        // alone showed "off" then, with my cards still face down).
        let auto = self.streamer_auto();
        let mut on = self.streamer_mode();
        let was = on;
        toggle_row(
            ui,
            &mut on,
            "Streamer mode",
            if auto {
                "On by itself: you're sharing your screen in a call. Turn it off here and it stays off until your next share. At a poker table \
                 your cards stay face down until you peek (hold Space, or point at them); invite links and codes are hidden."
            } else {
                "For when you're live: at a poker table your cards stay face down until you peek (hold Space, or point at them), and so does the \
                 hand strength meter; invite links and codes are hidden (Copy still copies them). It turns on by itself while you share your \
                 screen."
            },
        );
        if on != was {
            self.set_streamer_mode(on);
            draft.streamer_mode = on;
        }
        toggle_row(
            ui,
            &mut draft.stream_stage,
            "Just the game in Pyr's window",
            "In streamer mode, while you play at Pyr Casino or a poker table, Pyr's server and channel lists are put away: a window \
             capture shows the game and nothing of your servers. Esc brings them back.",
        );
        ui.horizontal(|ui| {
            ui.label(RichText::new("Room for your camera and overlays").color(theme::TEXT()));
            for (key, name) in [("", "None"), ("left", "Left side"), ("right", "Right side")] {
                if ui.selectable_label(draft.stream_room == key, name).clicked() {
                    draft.stream_room = key.to_string();
                }
            }
        });
        note(ui, "In streamer mode, a quarter of the window down that side stays clear at a game or a poker table, for your camera and overlays; the game fits itself into the rest.");
    }

    /// About Pyr: what it is and which version, updates, reporting bugs and ideas, and
    /// credits. 1 when Report a bug was clicked, 2 for Suggest a feature.
    fn about_page(&mut self, ui: &mut egui::Ui, draft: &mut Settings, acts: &mut Vec<Act>) -> u8 {
        let mut report = 0u8;
        ui.add_space(4.0);
        egui::Frame::new().fill(theme::SIDE()).corner_radius(12).inner_margin(20).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 32.0, theme::ELEV());
                icons::flame(ui.painter(), r.shrink(13.0));
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(pyr_proto::APP_NAME).font(theme::bold_font(24.0)).color(theme::TEXT()));
                        let v = env!("CARGO_PKG_VERSION");
                        let g = ui.painter().layout_no_wrap(v.to_string(), theme::bold_font(12.0), theme::MUTED());
                        let (pr, _) = ui.allocate_exact_size(g.size() + vec2(14.0, 8.0), Sense::hover());
                        ui.painter().rect_filled(pr, crate::theme::r(8), theme::ELEV2());
                        ui.painter().galley(pr.center() - g.size() / 2.0, g, theme::MUTED());
                    });
                    ui.label(RichText::new("Lean voice, video, screen share and chat. Written in Rust.").color(theme::MUTED()));
                });
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let s = ui.spacing_mut();
                s.interact_size.y = 34.0;
                s.button_padding = vec2(14.0, 7.0);
                match self.updater.status() {
                    Status::Available(m) => {
                        if ui.add(theme::primary_button(&format!("Update to {}", m.version))).on_hover_text("Pyr restarts and puts you back where you were").clicked() {
                            self.updater.download(m);
                        }
                    }
                    Status::Downloading(done, total) => {
                        ui.spinner();
                        let pct = if total > 0 { done * 100 / total } else { 0 };
                        ui.label(RichText::new(format!("Downloading the update… {pct}%")).color(theme::MUTED()));
                    }
                    _ => {
                        let checking = self.updater.checking();
                        if ui.add_enabled(!checking, egui::Button::new(if checking { "Checking…" } else { "Check for updates" })).clicked() {
                            acts.push(Act::CheckUpdates);
                        }
                        if let Some((said, at)) = &self.ui.update_answer {
                            if at.elapsed() < std::time::Duration::from_secs(120) && !checking {
                                let (text, color) = match said {
                                    crate::update::Checked::UpToDate => ("You have the newest Pyr.".to_string(), theme::ONLINE()),
                                    crate::update::Checked::Found(v) => (format!("Pyr {v} is out."), theme::EMBER2()),
                                    crate::update::Checked::Failed(e) => (format!("Couldn't check: {e}"), theme::LIVE()),
                                };
                                ui.label(RichText::new(text).size(13.0).color(color));
                            }
                        }
                    }
                }
                if pyr_engine::bugs::can_report() {
                    if ui.button("Suggest a feature").on_hover_text("An idea for Pyr. Asked for already? Yours counts as one more for it.").clicked() {
                        report = 2;
                    }
                    if ui.button("Report a bug").clicked() {
                        report = 1;
                    }
                }
            });
        });
        if pyr_engine::bugs::can_report() {
            ui.add_space(14.0);
            toggle_row(
                ui,
                &mut draft.auto_reports,
                "Send error reports automatically",
                "When something goes wrong (a screen share or camera that won't start or stops working, or Pyr crashed or froze), Pyr sends \
                 Firelit Games what happened and its log (never your messages), so it gets fixed.",
            );
        }
        divider(ui);
        heading(ui, "What makes Pyr different");
        let points = [
            ("End-to-end encrypted calls", "Voice, cameras and screen shares are encrypted on your PC with keys the server never sees (X25519 + ChaCha20-Poly1305, new keys whenever someone joins or leaves)."),
            ("No accounts", "Your identity is a key on this computer. No email, phone number or ID."),
            ("Self-hosted", "Communities run their own small server. Text history lives on that server."),
            ("Free quality", "1080p60 screen sharing and high-bitrate audio without a subscription."),
        ];
        for pair in points.chunks(2) {
            ui.columns(2, |cols| {
                for (col, (h, b)) in cols.iter_mut().zip(pair) {
                    egui::Frame::new().fill(theme::SIDE()).corner_radius(10).inner_margin(14).show(col, |ui| {
                        ui.set_width(ui.available_width());
                        ui.set_min_height(92.0);
                        // (Not justified, as a column's text would be.)
                        ui.with_layout(Layout::top_down(Align::Min), |ui| {
                            ui.label(RichText::new(*h).font(theme::bold_font(14.5)).color(theme::TEXT()));
                            ui.add(egui::Label::new(RichText::new(*b).size(13.0).color(theme::MUTED())).wrap());
                        });
                    });
                }
            });
            ui.add_space(2.0);
        }
        ui.add_space(4.0);
        fine(ui, "Text messages are stored on the server you're talking on (like Discord), not end-to-end encrypted.");
        heading(ui, "Credits");
        fine(
            ui,
            "Emoji graphics: Twemoji by Twitter, Inc. and other contributors, licensed CC-BY 4.0. Emoji names: Emojibase (MIT). GIF search \
             powered by GIPHY and KLIPY. Voice isolation: DeepFilterNet (MIT/Apache-2.0) and RNNoise (BSD). Spellcheck: spellbook (MPL-2.0) \
             with dictionaries from SCOWL (MIT/BSD), RLA-ES (MPL-1.1) and Grammalecte (MPL-2.0). Reading sound files for the soundboard: \
             Symphonia (MPL-2.0). Pyr's own soundboard sounds are synthesized by Pyr (no recordings).",
        );
        report
    }
}

/// Appearance → Moving pictures: play them, play them when pointed at, or don't.
fn moving_pictures(ui: &mut egui::Ui, draft: &mut Settings) {
    heading(ui, "Moving pictures");
    for (value, title, about) in [
        ("", "Play them", "GIFs and other moving pictures in chat play as they show."),
        ("hover", "Play when I point at them", "They stay still until you point at one."),
        ("never", "Don't play them", "They stay still (click one to open it and play it)."),
    ] {
        if choice_row(ui, draft.play_gifs == value, title, about) {
            draft.play_gifs = value.to_string();
        }
    }
    ui.add_space(4.0);
    fine(
        ui,
        "Either way they wait while Pyr's in the background, and clicking one opens it to play, pause or step through. Profile pictures \
         move while their person talks, and server icons when you point at them.",
    );
}

/// Windows: the icon by the clock, starting with Windows, and quitting.
fn windows_page(ui: &mut egui::Ui, draft: &mut Settings, acts: &mut Vec<Act>) {
    heading(ui, "Icon by the clock");
    note(
        ui,
        "Closing the window keeps Pyr running by the clock, so you stay in voice and friends can still reach you. Click the campfire icon \
         to open Pyr; right-click it to mute, leave voice or quit.",
    );
    ui.add_space(2.0);
    fine(ui, "Windows may tuck the icon under the ^ arrow. To keep it in view, drag it from there to the taskbar next to the clock.");
    if !crate::tray::is_active() {
        ui.add_space(4.0);
        ui.label(RichText::new("Windows isn't showing the icon right now, so closing the window minimizes Pyr instead.").size(12.5).color(theme::LIVE()));
    }
    divider(ui);
    heading(ui, "When Windows starts");
    toggle_row(
        ui,
        &mut draft.start_with_windows,
        "Open Pyr when Windows starts",
        "Like Discord: you're online and friends can reach you as soon as your PC is on.",
    );
    ui.add_enabled_ui(draft.start_with_windows, |ui| {
        toggle_row(ui, &mut draft.start_hidden, "Start in the tray", "Just the icon by the clock, no window, until you open it.");
    });
    divider(ui);
    ui.add_space(8.0);
    if ui.add(theme::danger_button("Quit Pyr").min_size(vec2(120.0, 36.0))).clicked() {
        acts.push(Act::Quit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_search_finds_pages_by_the_start_of_their_words() {
        let find = |q: &str| pages().filter(|p| found(p, q)).map(|p| p.id).collect::<Vec<_>>();
        assert_eq!(find("push"), vec![1]);
        assert_eq!(find("mic"), vec![1]);
        assert_eq!(find("ring"), vec![4]);
        assert_eq!(find("skin"), vec![5, 6]);
        assert_eq!(find("safety code"), vec![0]);
        assert_eq!(find("UPDATE"), vec![2]);
        assert_eq!(find("noise isol"), vec![1]);
        assert!(find("zebra").is_empty());
        assert_eq!(find("").len(), pages().count(), "nothing typed: every page");
    }

    #[test]
    fn the_sheet_fits_any_window() {
        for (w, h) in [(1280.0, 900.0), (1920.0, 1080.0), (940.0, 600.0), (700.0, 480.0)] {
            let s = sheet_size(vec2(w, h));
            assert!(s.x <= w - 24.0 && s.y <= h - 24.0, "{w}×{h}: {s:?}");
        }
        assert_eq!(sheet_size(vec2(1920.0, 1080.0)), vec2(1100.0, 820.0), "never more than this");
    }

    #[test]
    fn a_banner_is_in_the_pictures_color_and_greys_stay_grey() {
        let (top, bottom) = banner(Color32::from_rgb(0xe0, 0x30, 0x30));
        assert!(top.r() > top.g() * 2 && top.r() > bottom.r(), "red, brighter at the top: {top:?} {bottom:?}");
        let (top, _) = banner(Color32::from_rgb(0x80, 0x80, 0x80));
        let (r, g, b) = (top.r() as i32, top.g() as i32, top.b() as i32);
        assert!((r - g).abs() < 12 && (g - b).abs() < 12, "grey: {top:?}");
    }
}
