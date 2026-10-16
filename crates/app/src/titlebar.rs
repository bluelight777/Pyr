//! Windows: Ingle draws its own dark title strip (like Discord) instead of the system's white
//! one. The window has no system frame, so this also handles dragging, double-click to
//! maximize, the minimize / maximize / close buttons and resizing from the edges.

use eframe::egui::{self, Align2, Color32, CursorIcon, Pos2, Rect, ResizeDirection, Sense, Stroke, ViewportCommand, pos2, vec2};

use crate::theme;

/// Height of the strip.
pub const HEIGHT: f32 = 24.0;
const BUTTON_W: f32 = 44.0;
/// How close to the window's edge the pointer must be to resize it.
const GRIP: f32 = 5.0;

/// True when Ingle draws its own title strip: on Windows (and, to see it elsewhere, in
/// development builds run with `INGLE_DEV_TITLE_STRIP` set).
pub fn enabled() -> bool {
    cfg!(windows) || (cfg!(debug_assertions) && std::env::var_os("INGLE_DEV_TITLE_STRIP").is_some())
}

/// Draws the strip and handles resizing. Call first thing in `update`. `after_name(ui, x, y)`
/// draws what goes right after Ingle's name (the version and the updater), from `x` along and
/// centred on `y`.
pub fn show(ctx: &egui::Context, title: &str, after_name: impl FnOnce(&mut egui::Ui, f32, f32) -> f32) {
    if !enabled() {
        return;
    }
    let (fullscreen, maximized) = ctx.input(|i| (i.viewport().fullscreen.unwrap_or(false), i.viewport().maximized.unwrap_or(false)));
    if fullscreen {
        return;
    }
    if !maximized {
        resize_edges(ctx);
    }
    egui::TopBottomPanel::top("ingle-title-bar")
        .exact_height(HEIGHT)
        .resizable(false)
        .show_separator_line(false)
        .frame(egui::Frame::new().fill(theme::panel(theme::RAIL(), false)))
        .show(ctx, |ui| {
            let full = ui.max_rect();
            let buttons_w = BUTTON_W * 3.0;
            let drag_rect = Rect::from_min_max(full.min, pos2(full.max.x - buttons_w, full.max.y));
            // Dragging moves the window; double-click maximizes / restores.
            let drag = ui.interact(drag_rect, ui.id().with("drag"), Sense::click_and_drag());
            if drag.double_clicked() {
                ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
            } else if drag.drag_started_by(egui::PointerButton::Primary) {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            let p = ui.painter().clone();
            // Ingle's flame and name, then the version and the updater (see `update_chip`).
            crate::icons::flame(&p, Rect::from_center_size(pos2(full.left() + 17.0, full.center().y), vec2(13.0, 13.0)));
            let name = p.text(pos2(full.left() + 28.0, full.center().y), Align2::LEFT_CENTER, title, theme::bold_font(12.0), theme::MUTED());
            after_name(ui, name.right() + 3.0, full.center().y);

            let mut x = full.max.x - buttons_w;
            for kind in [Btn::Min, Btn::Max, Btn::Close] {
                let r = Rect::from_min_size(pos2(x, full.min.y), vec2(BUTTON_W, HEIGHT));
                x += BUTTON_W;
                let resp = ui.interact(r, ui.id().with(kind as u8), Sense::click());
                let hover = resp.hovered();
                if hover {
                    let bg = if kind == Btn::Close { Color32::from_rgb(0xd8, 0x3b, 0x3b) } else { theme::HOVER() };
                    p.rect_filled(r, 0.0, bg);
                }
                let col = if hover { theme::TEXT() } else { theme::MUTED() };
                draw_glyph(&p, kind, r.center(), maximized, col);
                if resp.clicked() {
                    match kind {
                        Btn::Min => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                        Btn::Max => ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized)),
                        // The same as the system close button: hides to the tray if that's on.
                        Btn::Close => ctx.send_viewport_cmd(ViewportCommand::Close),
                    }
                }
            }
        });
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Btn {
    Min,
    Max,
    Close,
}

fn draw_glyph(p: &egui::Painter, kind: Btn, c: Pos2, maximized: bool, col: Color32) {
    let s = Stroke::new(1.0_f32, col);
    match kind {
        Btn::Min => {
            p.line_segment([pos2(c.x - 5.0, c.y), pos2(c.x + 5.0, c.y)], s);
        }
        Btn::Max if maximized => {
            // Two overlapping squares: "restore".
            p.rect_stroke(Rect::from_center_size(c + vec2(-1.5, 1.5), vec2(8.0, 8.0)), 1.0, s, egui::StrokeKind::Middle);
            p.line_segment([pos2(c.x - 3.0, c.y - 5.0), pos2(c.x + 5.0, c.y - 5.0)], s);
            p.line_segment([pos2(c.x + 5.0, c.y - 5.0), pos2(c.x + 5.0, c.y + 3.0)], s);
        }
        Btn::Max => {
            p.rect_stroke(Rect::from_center_size(c, vec2(10.0, 10.0)), 1.0, s, egui::StrokeKind::Middle);
        }
        Btn::Close => {
            p.line_segment([pos2(c.x - 5.0, c.y - 5.0), pos2(c.x + 5.0, c.y + 5.0)], s);
            p.line_segment([pos2(c.x + 5.0, c.y - 5.0), pos2(c.x - 5.0, c.y + 5.0)], s);
        }
    }
}

/// With no system frame, resizing is done by pressing near the window's edges.
fn resize_edges(ctx: &egui::Context) {
    let screen = ctx.viewport_rect();
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
    let (l, r) = (pos.x - screen.left() < GRIP, screen.right() - pos.x < GRIP);
    let (t, b) = (pos.y - screen.top() < GRIP, screen.bottom() - pos.y < GRIP);
    let dir = match (l, r, t, b) {
        (true, _, true, _) => ResizeDirection::NorthWest,
        (_, true, true, _) => ResizeDirection::NorthEast,
        (true, _, _, true) => ResizeDirection::SouthWest,
        (_, true, _, true) => ResizeDirection::SouthEast,
        (true, ..) => ResizeDirection::West,
        (_, true, ..) => ResizeDirection::East,
        (_, _, true, _) => ResizeDirection::North,
        (_, _, _, true) => ResizeDirection::South,
        _ => return,
    };
    ctx.set_cursor_icon(match dir {
        ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
        ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
        ResizeDirection::West | ResizeDirection::East => CursorIcon::ResizeHorizontal,
        _ => CursorIcon::ResizeVertical,
    });
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
    }
}
