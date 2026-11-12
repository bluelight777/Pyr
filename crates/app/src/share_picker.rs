//! Share your screen (0.13.14), as 0.13.28 has it (its code read out of its program): "Pick a
//! whole screen or one window." under the title; Screens, Windows and Other as headings, each
//! with an icon, how many there are and a line to the right; screens as tiles with a monitor
//! shaped like each one (its number on it, its name and size under it); a window or a test
//! picture a line each (pointed at, the whole title), what's picked outlined with a tick. Then
//! the preview, the quality, the sound and Go Live.
//!
//! The title and the buttons always show; what's between fits what's left: the preview gets
//! smaller in a short window (210 at most, 64 at least), and the screens, windows and the rest
//! scroll in what's left.
//!
//! Secondary text is brightened where a skin makes it too faint on its background
//! (`theme::readable`, 0.13.14).

use eframe::egui::{self, Align, Align2, Color32, CornerRadius, Layout, Painter, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::config::ScreenPreset;
use pyr_engine::state::AppState;
use pyr_engine::video::{ScreenSource, SourceKind};

use crate::app::{Act, Dialog, PyrApp};
use crate::{icons, theme};

/// The dialog's width (its contents).
const WIDTH: f32 = 600.0;
/// Screen tiles: as many across as fit 160 each (three at most), 10 apart, 114 tall; one screen
/// alone takes half the width (270 at most).
const TILE_GAP: f32 = 10.0;
const TILE_H: f32 = 114.0;

/// "3840 × 2160".
fn size_text(s: &ScreenSource) -> String {
    format!("{} \u{d7} {}", s.width, s.height)
}

/// `text` in one line `width` wide at most, cut short with "…" if it doesn't fit (and whether it
/// was).
fn one_line(ui: &egui::Ui, text: &str, font: egui::FontId, color: Color32, width: f32) -> (std::sync::Arc<egui::Galley>, bool) {
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), font, color, width.max(10.0));
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let cut = galley.elided;
    (galley, cut)
}

/// A heading: 10 above it, a line 22 tall (its icon in an 18 square at the left, its name, how
/// many in a pill, and a line to the right edge), 6 under it.
fn heading(ui: &mut egui::Ui, icon: &dyn Fn(&Painter, Rect, Color32), label: &str, count: Option<usize>) {
    ui.add_space(10.0);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    let p = ui.painter();
    let mid = rect.center().y;
    let icon_rect = Rect::from_center_size(pos2(rect.left() + 9.0, mid), vec2(18.0, 18.0));
    icon(p, icon_rect, theme::EMBER2());
    let name = p.text(pos2(rect.left() + 27.0, mid), Align2::LEFT_CENTER, label, theme::bold_font(14.5), theme::TEXT());
    let mut end = name.right();
    if let Some(n) = count {
        let g = p.layout_no_wrap(n.to_string(), theme::bold_font(11.0), theme::TEXT());
        let pill = Rect::from_min_max(pos2(end + 8.0, icon_rect.top()), pos2(g.rect.width() + 14.0 + (end + 8.0), icon_rect.top() + 18.0));
        p.rect_filled(pill, CornerRadius::same(9), theme::ELEV2());
        p.galley(pos2((pill.min.x + pill.max.x) * 0.5, (pill.min.y + pill.max.y) * 0.5) - g.rect.size() * 0.5, g, theme::TEXT());
        end = pill.right();
    }
    if rect.right() > end + 12.0 {
        p.line_segment([pos2(end + 12.0, mid), pos2(rect.right(), mid)], Stroke::new(1.0f32, theme::LINE()));
    }
    ui.add_space(6.0);
}

/// What stands for a kind of source: a monitor, a window, or a test picture's bars.
fn source_icon(p: &Painter, r: Rect, c: Color32, kind: &SourceKind) {
    match kind {
        SourceKind::Monitor => icons::monitor(p, r, c),
        SourceKind::Window => icons::window(p, r, c),
        SourceKind::Test => icons::test_pattern(p, r),
    }
}

/// A screen's tile: the monitor (shaped like it), its number on it, its name and size under it.
/// `number`: its place in the list, unless its name says it (a made-up screen's "Screen 2 —
/// XB273K GP").
fn tile(ui: &mut egui::Ui, w: f32, s: &ScreenSource, number: usize, picked: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(w, TILE_H), Sense::click());
    let p = ui.painter();
    if picked {
        theme::selected(p, rect, 10, theme::ELEV2());
        p.rect_stroke(rect.shrink(0.75), theme::r(10), Stroke::new(1.5f32, theme::EMBER()), StrokeKind::Inside);
    } else {
        p.rect_filled(rect, theme::r(10), if resp.hovered() { theme::ELEV2() } else { theme::ELEV() });
        if theme::outlines() {
            p.rect_stroke(rect.shrink(0.5), theme::r(10), Stroke::new(1.0f32, theme::LINE()), StrokeKind::Inside);
        }
    }
    let (number, name) = match s.name.split_once(" \u{2014} ") {
        Some((first, rest)) if first.starts_with("Screen ") => (first["Screen ".len()..].trim().to_string(), rest),
        _ => (number.to_string(), s.name.as_str()),
    };
    // The monitor: 43 tall inside (less if it's so wide it would be wider than the tile allows),
    // its frame 2 outside that.
    let aspect = if s.width != 0 && s.height != 0 { s.width as f32 / s.height as f32 } else { 16.0 / 9.0 };
    let mon_w = (rect.width() - 40.0).min((TILE_H - 14.0 - 48.0 - 9.0) * aspect);
    let mon_h = mon_w / aspect;
    let cx = (rect.min.x + rect.max.x) * 0.5;
    let top = rect.min.y + 14.0;
    let screen = Rect::from_center_size(pos2(cx, top + mon_h * 0.5), vec2(mon_w, mon_h));
    let accent = if picked { theme::EMBER() } else { theme::readable(theme::MUTED(), theme::ELEV(), 4.5) };
    p.rect_filled(screen, theme::r(4), theme::STAGE());
    p.rect_stroke(screen, theme::r(4), Stroke::new(2.0f32, accent), StrokeKind::Outside);
    // (Its stand: a neck 5 long under the frame, and a foot 22 wide.)
    let sx = (screen.max.x + screen.min.x) * 0.5;
    let neck = screen.bottom() + 2.0;
    p.line_segment([pos2(sx, neck), pos2(sx, neck + 5.0)], Stroke::new(2.0f32, accent));
    p.line_segment([pos2(sx - 11.0, neck + 6.0), pos2(sx + 11.0, neck + 6.0)], Stroke::new(2.0f32, accent));
    let number_color = if picked { theme::EMBER2() } else { theme::TEXT() };
    p.text(pos2(sx, (screen.top() + screen.bottom()) * 0.5), Align2::CENTER_CENTER, number, theme::bold_font(17.0), number_color);
    let (name, cut) = one_line(ui, name, theme::bold_font(13.0), theme::TEXT(), rect.width() - 20.0);
    let p = ui.painter();
    p.galley(pos2((name.rect.max.x - name.rect.min.x) * -0.5 + cx, rect.bottom() - 40.0), name, theme::TEXT());
    if s.width != 0 {
        let color = theme::readable(theme::MUTED(), theme::ELEV(), 4.5);
        p.text(pos2(cx, rect.bottom() - 13.0), Align2::CENTER_CENTER, size_text(s), theme::font(12.0), color);
    }
    if picked {
        icons::done(p, Rect::from_center_size(pos2(rect.right() - 15.0, rect.top() + 15.0), vec2(18.0, 18.0)), theme::EMBER());
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if cut { resp.on_hover_text(&s.name) } else { resp }
}

/// A window's (or a test picture's) line: its icon, its title (cut short with "…" if it doesn't
/// fit: pointed at, the whole of it) and its size at the right.
fn row(ui: &mut egui::Ui, s: &ScreenSource, picked: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    let p = ui.painter();
    if picked {
        theme::selected(p, rect, 8, theme::ELEV2());
        p.rect_stroke(rect.shrink(0.75), theme::r(8), Stroke::new(1.5f32, theme::EMBER()), StrokeKind::Inside);
    } else if resp.hovered() {
        p.rect_filled(rect, theme::r(8), theme::HOVER());
    }
    let size_color = theme::readable(theme::MUTED(), theme::SIDE(), 4.5);
    let mid = (rect.min.y + rect.max.y) * 0.5;
    let icon = Rect::from_min_max(pos2(rect.min.x + 20.0 - 8.5, mid - 8.5), pos2(rect.min.x + 20.0 + 8.5, mid + 8.5));
    source_icon(p, icon, if picked { theme::EMBER2() } else { size_color }, &s.kind);
    let mut x = rect.max.x - 12.0;
    if s.width != 0 {
        let g = p.layout_no_wrap(size_text(s), theme::font(12.0), size_color);
        x += g.rect.min.x - g.rect.max.x;
        p.galley(pos2(x, (g.rect.max.y - g.rect.min.y) * -0.5 + mid), g, size_color);
        x -= 12.0;
    }
    if picked {
        icons::done(p, Rect::from_min_max(pos2(x - 8.0 - 8.0, mid - 8.0), pos2(x - 8.0 + 8.0, mid + 8.0)), theme::EMBER());
        x -= 24.0;
    }
    let left = icon.max.x + 11.0;
    let (title, cut) = one_line(ui, &s.name, theme::font(13.5), theme::TEXT(), x - left);
    ui.painter().galley(pos2(left, (title.rect.max.y - title.rect.min.y) * -0.5 + mid), title, theme::TEXT());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if cut { resp.on_hover_text(&s.name) } else { resp }
}

/// The screens, windows and test pictures, under their headings.
fn sources(ui: &mut egui::Ui, screens: &[ScreenSource], chosen: &mut Option<String>) {
    ui.spacing_mut().item_spacing.y = 3.0;
    for (label, kind) in [("Screens", SourceKind::Monitor), ("Windows", SourceKind::Window), ("Other", SourceKind::Test)] {
        let items: Vec<&ScreenSource> = screens.iter().filter(|s| s.kind == kind).collect();
        if items.is_empty() {
            continue;
        }
        heading(ui, &|p, r, c| source_icon(p, r, c, &kind), label, Some(items.len()));
        if kind == SourceKind::Monitor {
            let n = items.len();
            let w = ui.available_width();
            let across = (((w + 10.0) / 160.0).floor() as usize).max(1).min(n).min(3);
            let tile_w = if n == 1 { ((w - 10.0) * 0.5).min(270.0) } else { ((across - 1) as f32 * -10.0 + w) / across as f32 };
            for (r, line) in items.chunks(across).enumerate() {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = TILE_GAP;
                    for (i, s) in line.iter().enumerate() {
                        let picked = chosen.as_deref() == Some(s.id.as_str());
                        if tile(ui, tile_w, s, r * across + i + 1, picked).clicked() {
                            *chosen = Some(s.id.clone());
                        }
                    }
                });
                ui.add_space(4.0);
            }
        } else {
            for s in items {
                let picked = chosen.as_deref() == Some(s.id.as_str());
                if row(ui, s, picked).clicked() {
                    *chosen = Some(s.id.clone());
                }
            }
        }
    }
}

impl PyrApp {
    /// The share picker's dialog (`room`: the most its contents can take up).
    pub(crate) fn share_picker(&mut self, ui: &mut egui::Ui, st: &AppState, selected: Option<String>, room: f32, acts: &mut Vec<Act>, close: &mut bool) {
        ui.set_width(WIDTH);
        crate::dialogs::title(ui, "Share your screen");
        ui.label(RichText::new("Pick a whole screen or one window.").size(13.0).color(theme::readable(theme::MUTED(), theme::SIDE(), 5.0)));
        // What's between the title and the buttons, and of that, the preview as big as fits
        // (64 to 210) and the screens, windows and test pictures in what's left.
        let body_h = (room - 44.0 - 20.0 - 56.0).max(120.0);
        let preview_h = ((body_h - 215.0 - 230.0) * 0.5).max(64.0).min(210.0);
        let sources_h = (body_h - 215.0 - preview_h).max(100.0);
        let preset = st.settings.screen_preset;
        let mut chosen = selected.clone();
        let screens = &st.devices.screens;
        egui::ScrollArea::vertical().id_salt("share-body").max_height(body_h).auto_shrink([false, true]).show(ui, |ui| {
            if screens.is_empty() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Looking for screens and windows…").color(theme::MUTED()));
                });
            }
            egui::ScrollArea::vertical().id_salt("share-sources").max_height(sources_h).auto_shrink([false, true]).show(ui, |ui| {
                sources(ui, screens, &mut chosen);
            });
            if chosen.is_none() {
                chosen = screens.iter().find(|s| s.kind == SourceKind::Monitor).map(|s| s.id.clone());
            }
            self.share_preview(ui, chosen.as_deref(), preview_h);
            heading(ui, &|p, r, c| icons::gear(p, r.shrink(1.0), c), "Quality", None);
            let mut p = preset;
            egui::ComboBox::from_id_salt("screen-preset").selected_text(p.label()).width(260.0).show_ui(ui, |ui| {
                for q in ScreenPreset::ALL {
                    ui.selectable_value(&mut p, q, q.label());
                }
            });
            if p != preset {
                let mut s = st.settings.clone();
                s.screen_preset = p;
                acts.push(Act::Settings(s));
            }
            ui.add_space(8.0);
            let sound_ok = chosen.as_deref().map(pyr_engine::can_share_sound).unwrap_or(false);
            let what = match chosen.as_deref() {
                Some(id) if id.starts_with("win:") => "Share this app's sound",
                Some(id) if id.starts_with("test:") => "Share sound (a test tone)",
                _ => "Share sound (everything you hear, except this call)",
            };
            // Shown unticked when this source can't carry sound, so it never looks like it will.
            let mut share = st.settings.share_audio && sound_ok;
            let cb = ui.add_enabled(sound_ok, egui::Checkbox::new(&mut share, if sound_ok { what } else { "Share sound" }));
            if !sound_ok {
                cb.on_disabled_hover_text("Sharing sound needs Windows 10 (build 20348+) or Windows 11 for now.");
            } else if share != st.settings.share_audio {
                let mut s = st.settings.clone();
                s.share_audio = share;
                acts.push(Act::Settings(s));
            }
            ui.add_space(6.0);
            ui.label(
                RichText::new("Free, no subscription. Viewers choose to watch, so it only uses bandwidth for people who are watching.")
                    .size(12.0)
                    .color(theme::readable(theme::DIM(), theme::SIDE(), 4.5)),
            );
        });
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            let ok = chosen.is_some();
            if ui.add_enabled(ok, theme::primary_button("Go Live").min_size(vec2(110.0, 34.0))).clicked() {
                if let Some(id) = chosen.clone() {
                    acts.push(Act::StartScreen(id));
                    *close = true;
                }
            }
            if ui.button("Cancel").clicked() {
                *close = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                // (A refresh icon and the word; a box behind them under the pointer.)
                let (r, resp) = ui.allocate_exact_size(vec2(96.0, 30.0), Sense::click());
                let p = ui.painter();
                let c = if resp.hovered() {
                    p.rect_filled(r, theme::r(8), theme::ELEV2());
                    theme::TEXT()
                } else {
                    theme::readable(theme::MUTED(), theme::SIDE(), 5.0)
                };
                let mid = (r.min.y + r.max.y) * 0.5;
                icons::refresh(p, Rect::from_min_max(pos2(r.min.x + 18.0 - 8.0, mid - 8.0), pos2(r.min.x + 18.0 + 8.0, mid + 8.0)), c, 0.0);
                p.text(pos2(r.min.x + 32.0, mid), Align2::LEFT_CENTER, "Refresh", theme::font(13.0), c);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Look again for screens and windows").clicked() {
                    acts.push(Act::RefreshDevices);
                }
            });
        });
        if !*close {
            self.ui.dialog = Some(Dialog::ScreenPicker { selected: chosen });
        }
    }

    /// The preview of what's picked, under its heading: as big as `height`, in a black box with
    /// a thin frame round it.
    pub(crate) fn share_preview(&mut self, ui: &mut egui::Ui, source: Option<&str>, height: f32) {
        if self.ui.preview_source.as_deref() != source {
            self.ui.preview_source = source.map(String::from);
            self.ui.preview_tex = None;
            self.engine.preview_screen(source);
        }
        if let Some((w, h, rgba)) = self.engine.preview_frame(640) {
            let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
            match self.ui.preview_tex.as_mut() {
                Some((t, at)) => {
                    t.set(img, egui::TextureOptions::LINEAR);
                    *at = std::time::Instant::now();
                }
                None => {
                    let t = ui.ctx().load_texture("share-preview", img, egui::TextureOptions::LINEAR);
                    self.ui.preview_tex = Some((t, std::time::Instant::now()));
                }
            }
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        heading(ui, &|p, r, c| icons::eye(p, r.shrink(2.0), c), "Preview", None);
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        let p = ui.painter();
        p.rect_filled(r, theme::r(10), Color32::BLACK);
        p.rect_stroke(r, theme::r(10), Stroke::new(1.0f32, theme::LINE()), StrokeKind::Outside);
        match &self.ui.preview_tex {
            Some((t, _)) => {
                let size = t.size_vec2();
                let scale = (r.width() / size.x).min(r.height() / size.y);
                let half = size * scale * 0.5;
                let c = pos2((r.min.x + r.max.x) * 0.5, (r.min.y + r.max.y) * 0.5);
                let mut mesh = egui::Mesh::with_texture(t.id());
                mesh.add_rect_with_uv(Rect::from_min_max(c - half, c + half), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                p.add(egui::Shape::mesh(mesh));
            }
            None => {
                let text = if source.is_some() { "Getting a preview…" } else { "Pick a screen or window" };
                p.text(pos2((r.min.x + r.max.x) * 0.5, (r.min.y + r.max.y) * 0.5), Align2::CENTER_CENTER, text, theme::font(13.0), theme::DIM());
            }
        }
        ui.add_space(4.0);
    }
}
