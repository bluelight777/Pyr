//! Right-click menus, all alike (like Discord's): roomy rows that light up in the accent color
//! under the pointer, red ones for what can't be undone, checks on the right, and small
//! headings. Every menu calls `style` first; its rows are ordinary buttons after that.

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use crate::theme;

/// A menu's rows, from here on: their size, and lit in the accent color under the pointer.
pub fn style(ui: &mut egui::Ui) {
    let accent = theme::EMBER();
    let on_accent = theme::on_ember();
    let s = ui.style_mut();
    s.spacing.button_padding = vec2(10.0, 4.0);
    s.spacing.interact_size.y = 30.0;
    s.spacing.item_spacing = vec2(8.0, 2.0);
    s.spacing.slider_width = 180.0;
    s.spacing.menu_spacing = 4.0;
    let w = &mut s.visuals.widgets;
    for v in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        v.corner_radius = CornerRadius::same(6);
        v.bg_stroke = Stroke::NONE;
        v.expansion = 0.0;
    }
    w.hovered.weak_bg_fill = accent;
    w.hovered.bg_fill = accent;
    w.hovered.fg_stroke = Stroke::new(1.5, on_accent);
    w.active.weak_bg_fill = theme::mix(accent, Color32::BLACK, 0.15);
    w.active.bg_fill = w.active.weak_bg_fill;
    w.active.fg_stroke = Stroke::new(1.5, on_accent);
    // (A submenu that's open stays lit, a little less.)
    w.open.weak_bg_fill = theme::mix(accent, theme::SIDE(), 0.45);
    w.open.bg_fill = w.open.weak_bg_fill;
    w.open.fg_stroke = Stroke::new(1.5, theme::TEXT());
    s.visuals.selection.bg_fill = theme::mix(accent, theme::SIDE(), 0.55);
    s.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, theme::LINE());
    ui.set_min_width(210.0);
}

/// A row for what can't be undone (delete, kick, leave): red, and red all over under the
/// pointer.
pub fn danger(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    let text = text.into();
    ui.scope(|ui| {
        let w = &mut ui.style_mut().visuals.widgets;
        w.inactive.fg_stroke = Stroke::new(1.5, theme::LIVE());
        w.hovered.weak_bg_fill = theme::LIVE();
        w.hovered.bg_fill = theme::LIVE();
        w.hovered.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        w.active.weak_bg_fill = theme::mix(theme::LIVE(), Color32::BLACK, 0.15);
        w.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
        ui.add(egui::Button::new(text))
    })
    .inner
}

/// A row with a check box on its right: returns whether it was clicked (`on` is how it is).
/// `red` for one that's a moderator's power (Server Mute).
pub fn check(ui: &mut egui::Ui, on: bool, text: impl Into<String>, red: bool) -> egui::Response {
    let text = text.into();
    // As wide as the menu (as its other rows make it), or as this one needs.
    let need = ui.painter().layout_no_wrap(text.clone(), theme::font(14.0), theme::TEXT()).size().x + 56.0;
    let w = ui.min_rect().width().max(need).max(160.0);
    let (r, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
    let hovered = resp.hovered();
    let (fill, fg) = match (hovered, red) {
        (true, true) => (theme::LIVE(), Color32::WHITE),
        (true, false) => (theme::EMBER(), theme::on_ember()),
        (false, true) => (Color32::TRANSPARENT, theme::LIVE()),
        (false, false) => (Color32::TRANSPARENT, theme::TEXT()),
    };
    let p = ui.painter();
    if fill != Color32::TRANSPARENT {
        p.rect_filled(r, CornerRadius::same(6), fill);
    }
    let g = p.layout(text, theme::font(14.0), fg, w - 46.0);
    p.galley(pos2(r.left() + 10.0, r.center().y - g.size().y / 2.0), g, fg);
    // The box: filled with a tick when it's on.
    let b = Rect::from_center_size(pos2(r.right() - 18.0, r.center().y), vec2(18.0, 18.0));
    let mark = if red { theme::LIVE() } else { theme::EMBER() };
    if on {
        p.rect_filled(b, CornerRadius::same(4), if hovered { Color32::WHITE } else { mark });
        let tick = if hovered { if red { theme::LIVE() } else { theme::EMBER() } } else { theme::on_ember() };
        p.add(egui::Shape::line(
            vec![pos2(b.left() + 4.0, b.center().y), pos2(b.left() + 7.5, b.bottom() - 4.5), pos2(b.right() - 3.5, b.top() + 4.5)],
            Stroke::new(2.0, tick),
        ));
    } else {
        p.rect_stroke(b, CornerRadius::same(4), Stroke::new(1.5, if hovered { fg } else { theme::DIM() }), StrokeKind::Inside);
    }
    resp
}

/// A small heading over a few rows.
pub fn header(ui: &mut egui::Ui, text: &str) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.min_rect().width().max(120.0), 22.0), Sense::hover());
    ui.painter().text(pos2(r.left() + 10.0, r.center().y + 2.0), Align2::LEFT_CENTER, text.to_uppercase(), theme::bold_font(10.5), theme::DIM());
}
