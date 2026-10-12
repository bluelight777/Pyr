//! Warm "campfire" dark theme: colours, fonts and egui style.

use std::sync::Arc;

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals,
};

pub const RAIL: Color32 = Color32::from_rgb(0x15, 0x11, 0x0f);
pub const SIDE: Color32 = Color32::from_rgb(0x1e, 0x19, 0x16);
pub const SIDE_DARK: Color32 = Color32::from_rgb(0x1a, 0x15, 0x12);
pub const MAIN: Color32 = Color32::from_rgb(0x26, 0x20, 0x1c);
pub const ELEV: Color32 = Color32::from_rgb(0x2f, 0x28, 0x23);
pub const ELEV2: Color32 = Color32::from_rgb(0x3a, 0x31, 0x2b);
pub const HOVER: Color32 = Color32::from_rgb(0x2a, 0x23, 0x1f);
pub const LINE: Color32 = Color32::from_rgb(0x3b, 0x32, 0x2c);
pub const SHADOW_LINE: Color32 = Color32::from_rgb(0x14, 0x10, 0x0e);
pub const TEXT: Color32 = Color32::from_rgb(0xee, 0xe6, 0xdf);
pub const BODY: Color32 = Color32::from_rgb(0xe6, 0xdd, 0xd5);
pub const MUTED: Color32 = Color32::from_rgb(0xa8, 0x99, 0x8c);
pub const DIM: Color32 = Color32::from_rgb(0x7d, 0x70, 0x67);
pub const EMBER: Color32 = Color32::from_rgb(0xff, 0x7a, 0x3d);
pub const EMBER2: Color32 = Color32::from_rgb(0xff, 0xb3, 0x47);
pub const GLOW: Color32 = Color32::from_rgb(0xff, 0xa9, 0x4d);
pub const LIVE: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);
pub const ONLINE: Color32 = Color32::from_rgb(0x6c, 0xc0, 0x70);
pub const OWNER: Color32 = Color32::from_rgb(0xf5, 0xb0, 0x41);
pub const ADMIN: Color32 = Color32::from_rgb(0xec, 0x8f, 0x63);
pub const LINK: Color32 = Color32::from_rgb(0xff, 0xad, 0x7a);
pub const STAGE: Color32 = Color32::from_rgb(0x10, 0x0d, 0x0b);
pub const TILE: Color32 = Color32::from_rgb(0x1d, 0x18, 0x15);
pub const CODE_BG: Color32 = Color32::from_rgb(0x1a, 0x15, 0x12);

const AVATARS: [Color32; 8] = [
    Color32::from_rgb(0xff, 0xb3, 0x47),
    Color32::from_rgb(0x7f, 0xc8, 0xa9),
    Color32::from_rgb(0xf0, 0x8a, 0x8a),
    Color32::from_rgb(0x9f, 0xb4, 0xff),
    Color32::from_rgb(0xd7, 0xb2, 0xff),
    Color32::from_rgb(0xe6, 0xd3, 0x8a),
    Color32::from_rgb(0x8a, 0xd0, 0xe6),
    Color32::from_rgb(0xf2, 0xa9, 0xc7),
];

/// Stable colour for a person (from their public key).
pub fn avatar_color(user_id: &str) -> Color32 {
    let h = user_id.bytes().fold(0u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    AVATARS[(h % AVATARS.len() as u32) as usize]
}

pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn bold_font(size: f32) -> FontId {
    FontId::new(size, bold())
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("inter".into(), Arc::new(FontData::from_static(include_bytes!("../assets/fonts/Inter-Regular.otf"))));
    fonts
        .font_data
        .insert("inter-semibold".into(), Arc::new(FontData::from_static(include_bytes!("../assets/fonts/Inter-SemiBold.otf"))));
    let fallbacks: Vec<String> =
        fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut prop = vec!["inter".to_string()];
    prop.extend(fallbacks.iter().cloned());
    fonts.families.insert(FontFamily::Proportional, prop);
    let mut b = vec!["inter-semibold".to_string()];
    b.extend(fallbacks);
    fonts.families.insert(bold(), b);
    ctx.set_fonts(fonts);

    // Always the warm dark theme, even when Windows is set to light mode (egui would
    // otherwise switch to its light style on the first frame).
    ctx.set_theme(egui::ThemePreference::Dark);
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, font(11.5)),
            (TextStyle::Body, font(14.5)),
            (TextStyle::Button, font(14.0)),
            (TextStyle::Heading, bold_font(19.0)),
            (TextStyle::Monospace, mono(13.0)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.slider_width = 220.0;
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.scroll.floating = true;
        let mut v = Visuals::dark();
        v.panel_fill = MAIN;
        v.window_fill = SIDE;
        v.window_stroke = Stroke::new(1.0f32, LINE);
        v.window_corner_radius = CornerRadius::same(10);
        v.extreme_bg_color = CODE_BG;
        v.faint_bg_color = ELEV;
        v.code_bg_color = CODE_BG;
        v.hyperlink_color = LINK;
        v.selection.bg_fill = Color32::from_rgb(0x8a, 0x40, 0x1c);
        v.selection.stroke = Stroke::new(1.0f32, EMBER2);
        v.override_text_color = None;
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = SIDE;
        w.noninteractive.weak_bg_fill = SIDE;
        w.noninteractive.bg_stroke = Stroke::new(1.0f32, LINE);
        w.noninteractive.fg_stroke = Stroke::new(1.0f32, BODY);
        w.inactive.bg_fill = ELEV;
        w.inactive.weak_bg_fill = ELEV;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0f32, TEXT);
        w.inactive.corner_radius = CornerRadius::same(6);
        w.hovered.bg_fill = ELEV2;
        w.hovered.weak_bg_fill = ELEV2;
        w.hovered.bg_stroke = Stroke::new(1.0f32, Color32::from_rgb(0x55, 0x48, 0x3f));
        w.hovered.fg_stroke = Stroke::new(1.5f32, TEXT);
        w.hovered.corner_radius = CornerRadius::same(6);
        w.active.bg_fill = Color32::from_rgb(0x4a, 0x3a, 0x30);
        w.active.weak_bg_fill = Color32::from_rgb(0x4a, 0x3a, 0x30);
        w.active.fg_stroke = Stroke::new(1.5f32, EMBER2);
        w.active.corner_radius = CornerRadius::same(6);
        w.open.bg_fill = ELEV2;
        w.open.weak_bg_fill = ELEV2;
        s.visuals = v;
    });
}

/// Primary (ember) button.
pub fn primary_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_string()).color(Color32::from_rgb(0x1b, 0x14, 0x11)).font(bold_font(14.0)))
        .fill(EMBER)
        .corner_radius(CornerRadius::same(6))
}

pub fn danger_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_string()).color(Color32::WHITE).font(bold_font(14.0)))
        .fill(LIVE)
        .corner_radius(CornerRadius::same(6))
}

/// Window icon: the campfire from `assets/icon` (made by `tools/make-icons.py`).
pub fn app_icon() -> egui::IconData {
    let rgba = include_bytes!("../assets/icon/ingle-128.rgba").to_vec();
    egui::IconData { rgba, width: 128, height: 128 }
}
