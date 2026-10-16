//! The updater, where people look for it. On Windows it's a chip in Ingle's own title strip,
//! right after the name: the version ("0.7.3", and a check-for-updates arrow beside it), and
//! when an update is out, a glowing green "Update to 0.7.4" (click it: Ingle downloads it,
//! installs it and comes back where it was). Without a title strip of Ingle's own (Linux) the
//! same states are a button at the bottom of the server rail.

use std::f32::consts::TAU;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, Sense, Stroke, pos2, vec2};
use ingle_proto::release::Manifest;

use crate::update::{Asker, Status, Updater, current_version};
use crate::{icons, theme};

/// How long "Up to date" shows after a check, and "Couldn't check" after a failed one.
const UP_TO_DATE_FOR: f32 = 4.0;
const FAILED_FOR: f32 = 12.0;
/// How long a newly found update's button pulses before it settles into a steady glow.
const PULSE_FOR: f32 = 10.0;
/// One pulse.
const PULSE: f32 = 1.6;

const GREEN: Color32 = Color32::from_rgb(0x23, 0xa5, 0x5a);
const GREEN_HOT: Color32 = Color32::from_rgb(0x2e, 0xc2, 0x6c);
const GREEN_DEEP: Color32 = Color32::from_rgb(0x16, 0x3a, 0x25);
const GREEN_TINT: Color32 = Color32::from_rgb(0x1c, 0x33, 0x23);
const AMBER: Color32 = Color32::from_rgb(0xf0, 0xb2, 0x32);
const AMBER_TINT: Color32 = Color32::from_rgb(0x3b, 0x2d, 0x14);
const AMBER_HOT: Color32 = Color32::from_rgb(0x4d, 0x3a, 0x18);

/// The chip's height in the title strip.
const H: f32 = 18.0;
const ICON: f32 = 12.0;

/// What the updater shows, besides what the updater itself knows.
#[derive(Default)]
pub struct Chip {
    /// When a check asked for here said this is the newest version.
    up_to_date: Option<Instant>,
    /// When a check asked for here couldn't be made, and why.
    failed: Option<(Instant, String)>,
    /// When someone asked for a check here, until its answer comes (the arrow turns
    /// meanwhile). The regular checks in the background don't show.
    asking: Option<Instant>,
    /// When someone last asked for a check here (what it finds needs no toast: they're
    /// looking right at the answer).
    asked: Option<Instant>,
    /// The update on offer, and when it was first shown (it pulses for a while then).
    offered: Option<(String, Instant)>,
}

/// What it's showing.
enum Look {
    /// The version, with an arrow to check for updates.
    Version,
    Checking,
    /// Up to date, for this many seconds now.
    UpToDate(f32),
    Failed(String),
    /// An update to get, and how many releases this one is behind.
    Available(Manifest, usize),
    /// Downloading it: bytes done, and in all.
    Downloading(u64, u64),
    /// Downloaded: being installed, and Ingle is about to restart.
    Restarting,
}

/// Where the updater is, for what Ingle says about it ("Click Update …").
pub fn where_it_is() -> &'static str {
    if crate::titlebar::enabled() { "at the top of the window" } else { "at the bottom of the server list" }
}

impl Chip {
    /// A check asked for here said this is the newest version.
    pub fn told_up_to_date(&mut self) {
        self.asking = None;
        self.up_to_date = Some(Instant::now());
        self.failed = None;
    }

    /// A check asked for here couldn't be made.
    pub fn told_failed(&mut self, why: String) {
        self.asking = None;
        self.failed = Some((Instant::now(), why));
        self.up_to_date = None;
    }

    /// A check asked for here found a newer version (the Update button shows it).
    pub fn told_found(&mut self) {
        self.asking = None;
    }

    /// Someone asked for a check here a moment ago (so what it finds is seen right here).
    pub fn just_asked(&self) -> bool {
        self.asked.is_some_and(|t| t.elapsed() < Duration::from_secs(60))
    }

    fn check(&mut self, u: &Updater) {
        self.asked = Some(Instant::now());
        self.asking = Some(Instant::now());
        self.up_to_date = None;
        self.failed = None;
        u.check_now(Asker::Window, || {});
    }

    fn look(&mut self, u: &Updater) -> Look {
        match u.status() {
            Status::Available(m) => {
                if self.offered.as_ref().map(|(v, _)| *v != m.version).unwrap_or(true) {
                    self.offered = Some((m.version.clone(), Instant::now()));
                }
                let missed = m.missed(current_version());
                return Look::Available(m, missed);
            }
            Status::Downloading(done, total) => return Look::Downloading(done, total),
            Status::Ready(..) => return Look::Restarting,
            Status::Idle | Status::Failed(_) | Status::Stale(_) => {}
        }
        // (Its answer comes within the check's time limits; past that, stop turning anyway.)
        if self.asking.is_some_and(|t| t.elapsed() < Duration::from_secs(100)) {
            return Look::Checking;
        }
        if let Some(t) = self.up_to_date {
            let s = t.elapsed().as_secs_f32();
            if s < UP_TO_DATE_FOR {
                return Look::UpToDate(s);
            }
            self.up_to_date = None;
        }
        if let Some((t, why)) = &self.failed {
            if t.elapsed().as_secs_f32() < FAILED_FOR {
                return Look::Failed(why.clone());
            }
            self.failed = None;
        }
        Look::Version
    }

    /// How strongly a newly found update pulses now (0 to 1; 0 once it has settled).
    fn pulse(&self, now: f32) -> f32 {
        let Some((_, since)) = &self.offered else { return 0.0 };
        let age = since.elapsed().as_secs_f32();
        if age >= PULSE_FOR {
            return 0.0;
        }
        // (Fades out over the last pulse rather than stopping dead.)
        let fade = ((PULSE_FOR - age) / PULSE).min(1.0);
        let _ = now;
        (0.5 - 0.5 * (age * TAU / PULSE).cos()) * fade
    }

    /// The chip in Ingle's title strip, from `x` along and centred on `y`. Returns where it ends.
    pub fn title_chip(&mut self, ui: &mut egui::Ui, u: &Updater, x: f32, y: f32) -> f32 {
        let look = self.look(u);
        let now = ui.input(|i| i.time) as f32;
        let id = ui.id().with("update-chip");
        let top = y - H / 2.0;
        let ver = ui.painter().layout_no_wrap(current_version().to_string(), theme::font(11.0), theme::DIM);
        let vw = ver.size().x;
        const PAD: f32 = 6.0;
        const GAP: f32 = 5.0;

        match look {
            // The version, then the update's own button.
            Look::Available(..) | Look::Downloading(..) | Look::Restarting => {
                ui.painter().galley(pos2(x + PAD, y - ver.size().y / 2.0), ver, theme::DIM);
                let bx = x + PAD + vw + 8.0;
                self.update_button(ui, u, look, id, bx, top, now)
            }
            // The version (click it to check for updates), and how a check went.
            look => {
                let (text, icon_col, text_col, fill, tip): (Option<&str>, Color32, Color32, Color32, String) = match &look {
                    Look::Checking => (Some("Checking…"), theme::MUTED, theme::MUTED, theme::ELEV, "Checking for updates…".into()),
                    Look::UpToDate(_) => (
                        Some("Up to date"),
                        theme::ONLINE,
                        theme::ONLINE,
                        GREEN_TINT,
                        format!("Ingle {} is the newest version.", current_version()),
                    ),
                    Look::Failed(why) => (
                        Some("Couldn't check"),
                        AMBER,
                        AMBER,
                        AMBER_TINT,
                        format!("Couldn't check for updates: {why}\nClick to try again."),
                    ),
                    _ => (None, theme::DIM, theme::DIM, Color32::TRANSPARENT, format!("Ingle {}. Click to check for updates.", current_version())),
                };
                let text_galley = text.map(|t| ui.painter().layout_no_wrap(t.to_string(), theme::font(11.0), text_col));
                let target = PAD + vw + GAP + ICON + text_galley.as_ref().map(|g| GAP + g.size().x).unwrap_or(0.0) + PAD - 1.0;
                let w = ui.ctx().animate_value_with_time(id.with("w"), target, 0.18);
                let rect = Rect::from_min_size(pos2(x, top), vec2(w, H));
                let checking = matches!(look, Look::Checking);
                let resp = ui.interact(rect, id, if checking { Sense::hover() } else { Sense::click() });
                let hot = ui.ctx().animate_bool_with_time(id.with("hot"), resp.hovered() && !checking, 0.12);
                // (Up to date fades away over its last half second.)
                let fade = match look {
                    Look::UpToDate(s) => ((UP_TO_DATE_FOR - s) / 0.5).clamp(0.0, 1.0),
                    _ => 1.0,
                };
                let fill = match look {
                    Look::Version => theme::HOVER.gamma_multiply(hot),
                    Look::Failed(_) => lerp(AMBER_TINT, AMBER_HOT, hot),
                    _ => fill.gamma_multiply(fade),
                };
                let p = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
                p.rect_filled(rect, CornerRadius::same((H / 2.0) as u8), fill);
                let ver_col = lerp(if matches!(look, Look::Version) { theme::DIM } else { theme::MUTED }, theme::TEXT, hot);
                p.text(pos2(x + PAD, y), Align2::LEFT_CENTER, current_version(), theme::font(11.0), ver_col);
                let icon = Rect::from_min_size(pos2(x + PAD + vw + GAP, y - ICON / 2.0), vec2(ICON, ICON));
                match look {
                    Look::Checking => {
                        icons::refresh(&p, icon, icon_col, now * 6.0);
                        ui.ctx().request_repaint_after(Duration::from_millis(16));
                    }
                    Look::UpToDate(_) => {
                        icons::tick(&p, icon, icon_col.gamma_multiply(fade));
                        ui.ctx().request_repaint_after(Duration::from_millis(50));
                    }
                    Look::Failed(_) => {
                        icons::alert(&p, icon, lerp(icon_col, Color32::from_rgb(0xff, 0xcc, 0x66), hot));
                        ui.ctx().request_repaint_after(Duration::from_millis(500));
                    }
                    // A quarter turn as the pointer comes onto it: "check again".
                    _ => icons::refresh(&p, icon, lerp(icon_col.gamma_multiply(0.8), theme::TEXT, hot), hot * 0.25),
                }
                if let Some(g) = text_galley {
                    let col = match look {
                        Look::UpToDate(_) => text_col.gamma_multiply(fade),
                        Look::Failed(_) => lerp(text_col, Color32::from_rgb(0xff, 0xcc, 0x66), hot),
                        _ => text_col,
                    };
                    p.galley(pos2(icon.right() + GAP, y - g.size().y / 2.0), g, col);
                }
                let resp = if checking { resp } else { resp.on_hover_cursor(egui::CursorIcon::PointingHand) };
                if resp.on_hover_text(tip).clicked() && !checking {
                    self.check(u);
                }
                rect.right()
            }
        }
    }

    /// The green button for an update that's out (and how its download is going).
    #[allow(clippy::too_many_arguments)]
    fn update_button(&mut self, ui: &mut egui::Ui, u: &Updater, look: Look, id: egui::Id, x: f32, top: f32, now: f32) -> f32 {
        let y = top + H / 2.0;
        let version = self.offered.as_ref().map(|(v, _)| v.clone()).unwrap_or_default();
        let (label, font) = match &look {
            Look::Available(m, _) => (format!("Update to {}", m.version), theme::bold_font(11.0)),
            Look::Downloading(done, total) => {
                let pct = (*done as f32 / (*total).max(1) as f32 * 100.0).floor() as u32;
                (format!("Updating… {pct}%"), theme::bold_font(11.0))
            }
            _ => ("Restarting…".to_string(), theme::bold_font(11.0)),
        };
        let galley = ui.painter().layout_no_wrap(label, font.clone(), Color32::WHITE);
        // (While downloading, as wide as it'll ever be: it doesn't twitch as the numbers change.)
        let text_w = match &look {
            Look::Downloading(..) => ui.painter().layout_no_wrap("Updating… 100%".into(), font, Color32::WHITE).size().x,
            _ => galley.size().x,
        };
        let target = 9.0 + ICON + 5.0 + text_w + 10.0;
        let w = ui.ctx().animate_value_with_time(id.with("bw"), target, 0.2);
        let rect = Rect::from_min_size(pos2(x, top), vec2(w, H));
        let clickable = matches!(look, Look::Available(..));
        let resp = ui.interact(rect, id.with("update"), if clickable { Sense::click() } else { Sense::hover() });
        let hot = ui.ctx().animate_bool_with_time(id.with("bhot"), resp.hovered() && clickable, 0.15);
        let pulse = if clickable { self.pulse(now) } else { 0.0 };
        let radius = CornerRadius::same((H / 2.0) as u8);
        let p = ui.painter().with_clip_rect(ui.clip_rect());

        // The glow around it: a breath while it's new, brighter under the pointer.
        let glow = 0.30 + 0.45 * pulse.max(hot);
        p.add(egui::epaint::Shadow { offset: [0, 0], blur: 12, spread: 1, color: GREEN.gamma_multiply(glow) }.as_shape(rect, radius));

        match look {
            Look::Downloading(done, total) => {
                let f = (done as f32 / total.max(1) as f32).clamp(0.0, 1.0);
                p.rect_filled(rect, radius, GREEN_DEEP);
                // How far along, filling from the left (its end soft, not a hard edge).
                let filled = Rect::from_min_max(rect.min, pos2(rect.left() + rect.width() * f, rect.bottom()));
                let done_part = p.with_clip_rect(filled.intersect(p.clip_rect()));
                done_part.rect_filled(rect, radius, GREEN);
                shine(&done_part, rect, (now / 1.4).fract());
                ui.ctx().request_repaint_after(Duration::from_millis(33));
            }
            _ => {
                p.rect_filled(rect, radius, lerp(GREEN, GREEN_HOT, hot));
                if pulse > 0.0 {
                    // A glint across it now and then while it's new.
                    let age = self.offered.as_ref().map(|(_, t)| t.elapsed().as_secs_f32()).unwrap_or(0.0);
                    let phase = (age % (PULSE * 2.0)) / PULSE;
                    if phase < 1.0 {
                        shine(&p, rect, phase);
                    }
                    ui.ctx().request_repaint_after(Duration::from_millis(33));
                }
            }
        }
        // A little gloss on its top half.
        let gloss = Rect::from_min_max(rect.min, pos2(rect.right(), rect.center().y));
        let r = (H / 2.0) as u8;
        p.rect_filled(gloss, CornerRadius { nw: r, ne: r, sw: 0, se: 0 }, Color32::from_white_alpha(18));
        p.rect_stroke(rect, radius, Stroke::new(1.0_f32, Color32::from_white_alpha(22)), egui::StrokeKind::Inside);

        let icon = Rect::from_min_size(pos2(rect.left() + 9.0, y - ICON / 2.0), vec2(ICON, ICON));
        let clip = p.with_clip_rect(rect.intersect(p.clip_rect()));
        match look {
            Look::Restarting => {
                icons::refresh(&clip, icon, Color32::WHITE, now * 6.0);
                ui.ctx().request_repaint_after(Duration::from_millis(16));
            }
            Look::Downloading(..) => icons::download(&clip, icon, Color32::WHITE, 0.5 - 0.5 * (now * TAU / 0.9).cos()),
            _ => {
                // The arrow nods as the pointer comes onto it (and with each pulse while it's new).
                icons::download(&clip, icon, Color32::WHITE, hot.max(pulse));
            }
        }
        clip.galley(pos2(icon.right() + 5.0, y - galley.size().y / 2.0), galley, Color32::WHITE);

        let tip = match &look {
            Look::Available(m, missed) => {
                let behind = if *missed > 1 { format!(" You're {missed} updates behind.") } else { String::new() };
                format!("Ingle {} is out.{behind}\nClick to update: Ingle restarts and puts you back where you were.", m.version)
            }
            Look::Downloading(done, total) => {
                format!("Downloading Ingle {version}: {:.1} of {:.1} MB", *done as f64 / 1e6, *total as f64 / 1e6)
            }
            _ => format!("Installing Ingle {version}. Ingle restarts and puts you back where you were."),
        };
        let resp = if clickable { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
        if resp.on_hover_text(tip).clicked() {
            if let Look::Available(m, _) = look {
                u.download(m);
            }
        }
        rect.right()
    }

    /// The updater at the bottom of the server rail, where there's no title strip of Ingle's own.
    pub fn rail_button(&mut self, ui: &mut egui::Ui, u: &Updater) {
        let look = self.look(u);
        let now = ui.input(|i| i.time) as f32;
        let id = ui.id().with("update-rail");
        match look {
            Look::Available(..) | Look::Downloading(..) | Look::Restarting => {
                let (rect, resp) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::click());
                let clickable = matches!(look, Look::Available(..));
                let hot = ui.ctx().animate_bool_with_time(id.with("hot"), resp.hovered() && clickable, 0.15);
                let pulse = if clickable { self.pulse(now) } else { 0.0 };
                let radius = CornerRadius::same(if hot > 0.5 { 16 } else { 24 });
                let p = ui.painter();
                p.add(egui::epaint::Shadow { offset: [0, 0], blur: 14, spread: 1, color: GREEN.gamma_multiply(0.30 + 0.45 * pulse.max(hot)) }.as_shape(rect, radius));
                match &look {
                    Look::Downloading(done, total) => {
                        let f = (*done as f32 / (*total).max(1) as f32).clamp(0.0, 1.0);
                        p.rect_filled(rect, radius, GREEN_DEEP);
                        let filled = Rect::from_min_max(pos2(rect.left(), rect.bottom() - rect.height() * f), rect.max);
                        p.with_clip_rect(filled).rect_filled(rect, radius, GREEN);
                        let pct = format!("{}%", (f * 100.0).floor() as u32);
                        p.text(rect.center(), Align2::CENTER_CENTER, pct, theme::bold_font(13.0), Color32::WHITE);
                        ui.ctx().request_repaint_after(Duration::from_millis(100));
                    }
                    Look::Restarting => {
                        p.rect_filled(rect, radius, GREEN);
                        icons::refresh(p, rect.shrink(13.0), Color32::WHITE, now * 6.0);
                        ui.ctx().request_repaint_after(Duration::from_millis(16));
                    }
                    Look::Available(_, missed) => {
                        p.rect_filled(rect, radius, lerp(GREEN, GREEN_HOT, hot));
                        icons::download(p, rect.shrink(12.0), Color32::WHITE, hot.max(pulse));
                        if pulse > 0.0 {
                            ui.ctx().request_repaint_after(Duration::from_millis(33));
                        }
                        // Several updates missed: how many, in a red badge (like Discord's counts).
                        if *missed > 1 {
                            let t = if *missed > 9 { "9+".to_string() } else { missed.to_string() };
                            let b = Rect::from_center_size(rect.right_bottom() + vec2(-6.0, -6.0), vec2(if *missed > 9 { 24.0 } else { 20.0 }, 20.0));
                            p.rect_filled(b.expand(3.0), CornerRadius::same(13), theme::RAIL);
                            p.rect_filled(b, CornerRadius::same(10), theme::LIVE);
                            p.text(b.center(), Align2::CENTER_CENTER, t, theme::bold_font(12.0), Color32::WHITE);
                        }
                    }
                    _ => {}
                }
                let version = self.offered.as_ref().map(|(v, _)| v.clone()).unwrap_or_default();
                let tip = match &look {
                    Look::Available(m, missed) => {
                        let behind = if *missed > 1 { format!(" You're {missed} updates behind.") } else { String::new() };
                        format!("Ingle {} is out.{behind}\nClick to update: Ingle restarts and puts you back where you were.", m.version)
                    }
                    Look::Downloading(done, total) => format!("Downloading Ingle {version}: {:.1} of {:.1} MB", *done as f64 / 1e6, *total as f64 / 1e6),
                    _ => format!("Installing Ingle {version}…"),
                };
                let resp = if clickable { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp };
                if resp.on_hover_text(tip).clicked() {
                    if let Look::Available(m, _) = look {
                        u.download(m);
                    }
                }
            }
            look => {
                // The version, small, and the check-for-updates arrow (or how a check went).
                let (rect, resp) = ui.allocate_exact_size(vec2(56.0, 22.0), Sense::click());
                let checking = matches!(look, Look::Checking);
                let hot = ui.ctx().animate_bool_with_time(id.with("hot"), resp.hovered() && !checking, 0.12);
                let (icon_col, fill, tip) = match &look {
                    Look::Checking => (theme::MUTED, theme::ELEV, "Checking for updates…".to_string()),
                    Look::UpToDate(_) => (theme::ONLINE, GREEN_TINT, format!("Ingle {} is the newest version.", current_version())),
                    Look::Failed(why) => (AMBER, lerp(AMBER_TINT, AMBER_HOT, hot), format!("Couldn't check for updates: {why}\nClick to try again.")),
                    _ => (lerp(theme::DIM, theme::TEXT, hot), theme::HOVER.gamma_multiply(hot), format!("Ingle {}. Click to check for updates.", current_version())),
                };
                let p = ui.painter();
                p.rect_filled(rect, CornerRadius::same(11), fill);
                let g = p.layout_no_wrap(current_version().to_string(), theme::font(10.5), lerp(theme::DIM, theme::TEXT, hot));
                let total = g.size().x + 4.0 + 11.0;
                let left = rect.center().x - total / 2.0;
                let icon = Rect::from_min_size(pos2(left + g.size().x + 4.0, rect.center().y - 5.5), vec2(11.0, 11.0));
                p.galley(pos2(left, rect.center().y - g.size().y / 2.0), g, theme::DIM);
                match look {
                    Look::Checking => {
                        icons::refresh(p, icon, icon_col, now * 6.0);
                        ui.ctx().request_repaint_after(Duration::from_millis(16));
                    }
                    Look::UpToDate(_) => {
                        icons::tick(p, icon, icon_col);
                        ui.ctx().request_repaint_after(Duration::from_millis(100));
                    }
                    Look::Failed(_) => {
                        icons::alert(p, icon, icon_col);
                        ui.ctx().request_repaint_after(Duration::from_millis(500));
                    }
                    _ => icons::refresh(p, icon, icon_col, hot * 0.25),
                }
                let resp = if checking { resp } else { resp.on_hover_cursor(egui::CursorIcon::PointingHand) };
                if resp.on_hover_text(tip).clicked() && !checking {
                    self.check(u);
                }
            }
        }
    }
}

/// A soft band of light across a pill (`phase` 0 to 1: from just off its left end to just off
/// its right). Kept off the round ends, so it never shows outside them.
fn shine(p: &egui::Painter, rect: Rect, phase: f32) {
    let inner = rect.shrink2(vec2(rect.height() * 0.4, 0.0));
    let half = 10.0;
    let cx = rect.left() - half * 2.0 + (rect.width() + half * 4.0) * phase;
    let slant = rect.height() * 0.45;
    let (t, b) = (rect.top(), rect.bottom());
    let clear = Color32::TRANSPARENT;
    let light = Color32::from_white_alpha(70);
    let mut mesh = egui::Mesh::default();
    // Three columns of two points each (left edge, the middle, right edge), slanted.
    for (dx, col) in [(-half, clear), (0.0, light), (half, clear)] {
        mesh.colored_vertex(pos2(cx + dx + slant, t), col);
        mesh.colored_vertex(pos2(cx + dx - slant, b), col);
    }
    for i in [0u32, 2] {
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i + 1, i + 3, i + 2);
    }
    p.with_clip_rect(inner.intersect(p.clip_rect())).add(egui::Shape::mesh(mesh));
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_blend_end_to_end() {
        assert_eq!(lerp(GREEN, GREEN_HOT, 0.0), GREEN);
        assert_eq!(lerp(GREEN, GREEN_HOT, 1.0), GREEN_HOT);
        assert_eq!(lerp(Color32::TRANSPARENT, Color32::WHITE, 2.0), Color32::WHITE);
    }

    #[test]
    fn a_new_update_pulses_then_settles() {
        let mut c = Chip::default();
        assert_eq!(c.pulse(0.0), 0.0, "nothing on offer");
        c.offered = Some(("9.9.9".into(), Instant::now() - Duration::from_secs_f32(PULSE / 2.0)));
        assert!(c.pulse(0.0) > 0.9, "half a pulse in, it's at its brightest");
        c.offered = Some(("9.9.9".into(), Instant::now() - Duration::from_secs_f32(PULSE_FOR + 1.0)));
        assert_eq!(c.pulse(0.0), 0.0, "settled");
    }

    #[test]
    fn says_where_it_is() {
        let w = where_it_is();
        assert!(w.starts_with("at the "));
    }
}
