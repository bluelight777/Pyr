//! Warm "campfire" dark theme: colors, fonts and egui style. A skin (see `crate::looks`) can
//! change the colors, the fonts and put a picture behind the chat: the colors are read
//! through the functions below, so everything Pyr draws follows the skin in use.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle, Visuals,
};

/// The palette in use, in `pyr_proto::skin::COLORS`' order (0: Campfire's color).
static PALETTE: [AtomicU32; 25] = [const { AtomicU32::new(0) }; 25];
/// A skin's colors are in use (not Campfire's).
static SKINNED: AtomicBool = AtomicBool::new(false);
/// How much of a skin's picture shows through the panels (thousandths; 0: no picture), and
/// whether through the side panels too (not just the chat).
static PICTURE_SHOW: AtomicU32 = AtomicU32::new(0);
static PICTURE_WHOLE_WINDOW: AtomicBool = AtomicBool::new(false);

/// A skin's shapes (see `pyr_proto::skin::Look`): how round corners are (thousandths of
/// Pyr's own), people's pictures (0 circle, 1 rounded, 2 square), how what's selected is
/// marked (0 fill, 1 bar, 2 outline), and outlines on buttons and boxes.
static CORNERS: AtomicU32 = AtomicU32::new(1000);
static FACES: AtomicU8 = AtomicU8::new(0);
static SELECTED: AtomicU8 = AtomicU8::new(0);
static OUTLINES: AtomicBool = AtomicBool::new(false);

/// Use a skin's shapes (`None`: Campfire's). Call `apply_style` after.
pub fn set_look(l: Option<&pyr_proto::skin::Look>) {
    let d = pyr_proto::skin::Look::default();
    let l = l.unwrap_or(&d);
    CORNERS.store((l.corners.clamp(0.0, 2.0) * 1000.0).round() as u32, Ordering::Relaxed);
    FACES.store(
        match l.avatars.as_str() {
            "rounded" => 1,
            "square" => 2,
            _ => 0,
        },
        Ordering::Relaxed,
    );
    SELECTED.store(
        match l.selected.as_str() {
            "bar" => 1,
            "outline" => 2,
            _ => 0,
        },
        Ordering::Relaxed,
    );
    OUTLINES.store(l.outlines, Ordering::Relaxed);
}

/// Corners `n` round in Campfire, as round as the skin in use has them.
pub fn r(n: u8) -> CornerRadius {
    CornerRadius::same(rf(n as f32).round().min(255.0) as u8)
}

/// The same, for a radius worked out (not a fixed one).
pub fn rf(n: f32) -> f32 {
    n * CORNERS.load(Ordering::Relaxed) as f32 / 1000.0
}

/// How round the corners of someone's picture `radius` across are: a circle, or the skin's
/// rounded square or square.
pub fn face_round(radius: f32) -> f32 {
    match FACES.load(Ordering::Relaxed) {
        0 => radius,
        1 => radius * 0.42,
        _ => radius * 0.14,
    }
}

/// Whether people's pictures are circles (the skin in use hasn't made them squares).
pub fn round_faces() -> bool {
    FACES.load(Ordering::Relaxed) == 0
}

/// Whether what's selected is marked by filling it (Campfire's way).
pub fn selected_fills() -> bool {
    SELECTED.load(Ordering::Relaxed) == 0
}

/// Whether buttons, boxes and cards have outlines.
pub fn outlines() -> bool {
    OUTLINES.load(Ordering::Relaxed)
}

/// Marks a row as the one selected (a channel, a conversation, a page), the skin's way: filled
/// (`fill`, see-through over a skin's picture), a bar of the accent at its left, or an outline
/// of the accent. `round`: its corners, in Campfire.
pub fn selected(p: &egui::Painter, rect: egui::Rect, round: u8, fill: Color32) {
    let corners = r(round);
    match SELECTED.load(Ordering::Relaxed) {
        1 => {
            p.rect_filled(rect, corners, row(fill).gamma_multiply(0.6));
            let bar = egui::Rect::from_min_max(egui::pos2(rect.left(), rect.top() + 5.0), egui::pos2(rect.left() + 3.0, rect.bottom() - 5.0));
            p.rect_filled(bar, r(2), EMBER());
        }
        2 => {
            p.rect_filled(rect, corners, row(fill).gamma_multiply(0.4));
            p.rect_stroke(rect.shrink(0.5), corners, Stroke::new(1.0f32, EMBER()), egui::StrokeKind::Inside);
        }
        _ => {
            p.rect_filled(rect, corners, row(fill));
        }
    }
}

fn color(i: usize) -> Color32 {
    let v = PALETTE[i].load(Ordering::Relaxed);
    let [r, g, b] = if v == 0 { pyr_proto::skin::CAMPFIRE[i] } else { [(v >> 16) as u8, (v >> 8) as u8, v as u8] };
    Color32::from_rgb(r, g, b)
}

/// Use a skin's palette (`None`: Campfire's). Call `apply_style` after, for egui's own widgets.
pub fn set_palette(p: Option<[[u8; 3]; 25]>) {
    for (i, slot) in PALETTE.iter().enumerate() {
        let v = p.map(|p| {
            let [r, g, b] = p[i];
            0xff00_0000 | (r as u32) << 16 | (g as u32) << 8 | b as u32
        });
        slot.store(v.unwrap_or(0), Ordering::Relaxed);
    }
    SKINNED.store(p.is_some(), Ordering::Relaxed);
}

/// A skin's picture showing through the panels: how much (0: none), and through the side
/// panels too or just the chat.
pub fn set_picture(show: f32, whole_window: bool) {
    PICTURE_SHOW.store((show.clamp(0.0, 1.0) * 1000.0) as u32, Ordering::Relaxed);
    PICTURE_WHOLE_WINDOW.store(whole_window, Ordering::Relaxed);
}

/// Whether a skin's picture shows through here (`chat`: the chat and pages like it; the side
/// panels only when the picture is under the whole window).
fn picture_here(chat: bool) -> bool {
    PICTURE_SHOW.load(Ordering::Relaxed) > 0 && (chat || PICTURE_WHOLE_WINDOW.load(Ordering::Relaxed))
}

/// A panel's fill: its color, see-through where a skin's picture shows (see `picture_here`).
/// For the outermost panels; the ones inside them take `inner`.
pub fn panel(c: Color32, chat: bool) -> Color32 {
    let show = PICTURE_SHOW.load(Ordering::Relaxed) as f32 / 1000.0;
    if picture_here(chat) { c.gamma_multiply(1.0 - show) } else { c }
}

/// The fill of a panel inside one filled with `panel` (the same color): none where the picture
/// shows, so it isn't covered twice.
pub fn inner(c: Color32, chat: bool) -> Color32 {
    if picture_here(chat) { Color32::TRANSPARENT } else { c }
}

macro_rules! palette {
    ($($name:ident = $i:expr, $what:literal;)*) => {
        $(
            #[doc = $what]
            #[allow(non_snake_case)]
            #[inline]
            pub fn $name() -> Color32 {
                color($i)
            }
        )*
        #[cfg(test)]
        const NAMES: &[(&str, usize)] = &[$((stringify!($name), $i)),*];
    };
}

palette! {
    RAIL = 0, "The strip of server icons on the far left.";
    SIDE = 1, "The channel and member lists.";
    SIDE_DARK = 2, "The bars under the channel list.";
    MAIN = 3, "The chat.";
    ELEV = 4, "Cards, boxes to type in, buttons.";
    ELEV2 = 5, "A button or card under the pointer.";
    HOVER_SOLID = 6, "A row under the pointer (see `HOVER`).";
    LINE = 7, "Lines between things.";
    SHADOW_LINE = 8, "The darker line under headers.";
    TEXT = 9, "Headings and names.";
    BODY = 10, "Message text.";
    MUTED = 11, "Secondary text.";
    DIM = 12, "The faintest text.";
    EMBER = 13, "The accent.";
    EMBER2 = 14, "The accent's lighter shade.";
    GLOW = 15, "Speaking rings and glows.";
    LIVE = 16, "LIVE, alerts, unread badges.";
    ONLINE = 17, "Online dots.";
    OWNER = 18, "The owner's crown.";
    ADMIN = 19, "Admins' names.";
    LINK = 20, "Links.";
    STAGE = 21, "Behind video and screen shares.";
    TILE = 22, "Video tiles.";
    CODE_BG = 23, "Code blocks.";
    SELECTION = 24, "Selected text.";
}

/// A row under the pointer: the skin's hover color, see-through where a skin's picture shows
/// (see `row`).
#[allow(non_snake_case)]
#[inline]
pub fn HOVER() -> Color32 {
    row(HOVER_SOLID())
}

/// A row's highlight (under the pointer, or the one chosen): `c` as it is, or, while a skin's
/// picture shows, a little lighter and see-through, so the picture still shows behind the row
/// instead of being hidden by a solid block.
pub fn row(c: Color32) -> Color32 {
    if PICTURE_SHOW.load(Ordering::Relaxed) == 0 {
        return c;
    }
    let lit = mix(c, TEXT(), 0.10);
    Color32::from_rgba_unmultiplied(lit.r(), lit.g(), lit.b(), ROW_ALPHA)
}

/// How solid a row's highlight is over a skin's picture (of 255).
const ROW_ALPHA: u8 = 120;

/// The BOT tag on bots' names (Discord's blurple; the same in every skin).
pub const BOT: Color32 = Color32::from_rgb(0x58, 0x65, 0xf2);

/// `a` moved towards `b` by `t`.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// A color's hue (0 to 1), saturation and lightness.
pub fn hsl(c: Color32) -> (f32, f32, f32) {
    let (r, g, b) = (c.r() as f32 / 255.0, c.g() as f32 / 255.0, c.b() as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    if d <= f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    (h / 6.0, s.clamp(0.0, 1.0), l)
}

/// The color of hue `h` (0 to 1), saturation `s` and lightness `l`.
pub fn from_hsl(h: f32, s: f32, l: f32) -> Color32 {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h.rem_euclid(1.0) * 6.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    let to = |v: f32| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(to(r), to(g), to(b))
}

/// What's behind someone in a call, in their picture's own color (see `images::accent_of`):
/// deep and rich, darker at the top, like Discord's. (Top, bottom.)
pub fn tile_tint(accent: Color32) -> (Color32, Color32) {
    let (h, s, _) = hsl(accent);
    // (A grey picture keeps a grey tile; any color gets a rich, deep version of itself.)
    let s = if s < 0.1 { s } else { s.clamp(0.45, 0.85) };
    (from_hsl(h, s, 0.085), from_hsl(h, s, 0.215))
}

/// Text on the accent color: dark, or white on a dark accent.
pub fn on_ember() -> Color32 {
    let dark = [0x1b, 0x14, 0x11];
    let e = EMBER();
    if pyr_proto::skin::contrast(dark, [e.r(), e.g(), e.b()]) >= 4.5 { Color32::from_rgb(dark[0], dark[1], dark[2]) } else { Color32::WHITE }
}

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

/// Stable color for a person (from their public key).
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
    // Clicks as forgiving as Windows' own: a slow press, or a hand that moves a few pixels while
    // clicking, still counts. (egui's defaults, 0.8 s and 6 points, turn those into drags, so a
    // reaction or button sometimes didn't respond.)
    ctx.options_mut(|o| {
        o.input_options.max_click_dist = 10.0;
        o.input_options.max_click_duration = 5.0;
    });
    install_fonts(ctx, None);
    crate::emoji::preload();
    // Always the warm dark theme, even when Windows is set to light mode (egui would
    // otherwise switch to its light style on the first frame).
    ctx.set_theme(egui::ThemePreference::Dark);
    apply_style(ctx);
}

/// A skin's fonts: body and bold (each checked to parse first), and how much to scale them.
pub struct SkinFonts {
    pub body: Option<Vec<u8>>,
    pub bold: Option<Vec<u8>>,
    pub scale: f32,
}

/// Pyr's fonts, with a skin's first (Inter stays behind them for anything they don't have).
pub fn install_fonts(ctx: &egui::Context, skin: Option<SkinFonts>) {
    let mut fonts = FontDefinitions::default();
    fonts
        .font_data
        .insert("inter".into(), Arc::new(FontData::from_static(include_bytes!("../assets/fonts/Inter-Regular.otf"))));
    fonts
        .font_data
        .insert("inter-semibold".into(), Arc::new(FontData::from_static(include_bytes!("../assets/fonts/Inter-SemiBold.otf"))));
    let fallbacks: Vec<String> =
        fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let (mut prop, mut b) = (vec!["inter".to_string()], vec!["inter-semibold".to_string()]);
    if let Some(s) = skin {
        let tweak = egui::FontTweak { scale: s.scale.clamp(0.8, 1.25), ..Default::default() };
        // (Each was parsed already: egui stops the app on a font it can't read.)
        let body = s.body.filter(|f| pyr_engine::skins::font_ok(f));
        let bold = s.bold.filter(|f| pyr_engine::skins::font_ok(f)).or_else(|| body.clone());
        if let Some(f) = body {
            fonts.font_data.insert("skin-body".into(), Arc::new(FontData::from_owned(f).tweak(tweak)));
            prop.insert(0, "skin-body".into());
        }
        if let Some(f) = bold {
            fonts.font_data.insert("skin-bold".into(), Arc::new(FontData::from_owned(f).tweak(tweak)));
            b.insert(0, "skin-bold".into());
        }
    }
    prop.extend(fallbacks.iter().cloned());
    // Last: the parts of emoji that take no room (the emoji-style selector, joiners, a flag's
    // tag letters) take none in plain text either, instead of showing as empty boxes (a
    // channel name being typed, say).
    prop.push("emoji-zero".into());
    fonts.families.insert(FontFamily::Proportional, prop);
    b.extend(fallbacks);
    b.push("emoji-zero".into());
    fonts.families.insert(bold(), b);
    fonts.font_data.insert("emoji-blank".into(), Arc::new(FontData::from_static(crate::emoji::BLANK_TTF)));
    fonts.font_data.insert("emoji-zero".into(), Arc::new(FontData::from_static(crate::emoji::ZERO_TTF)));
    fonts.families.insert(crate::emoji::blank_family(), vec!["emoji-blank".into()]);
    fonts.families.insert(crate::emoji::zero_family(), vec!["emoji-zero".into()]);
    ctx.set_fonts(fonts);
}

/// egui's own widgets in the palette in use (again after a skin changes it).
pub fn apply_style(ctx: &egui::Context) {
    let skinned = SKINNED.load(Ordering::Relaxed);
    ctx.all_styles_mut(|s| {
        let active = if skinned { mix(ELEV2(), EMBER(), 0.1) } else { Color32::from_rgb(0x4a, 0x3a, 0x30) };
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
        s.spacing.menu_margin = egui::Margin::same(6);
        s.spacing.scroll.bar_width = 8.0;
        s.spacing.scroll.floating = true;
        let mut v = Visuals::dark();
        v.panel_fill = MAIN();
        v.window_fill = SIDE();
        v.window_stroke = Stroke::new(1.0f32, LINE());
        v.window_corner_radius = r(10);
        // Menus and tooltips: rounded, roomy, floating over things on a soft shadow.
        v.menu_corner_radius = r(10);
        v.popup_shadow = egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(150) };
        v.extreme_bg_color = CODE_BG();
        v.faint_bg_color = ELEV();
        v.code_bg_color = CODE_BG();
        v.hyperlink_color = LINK();
        v.selection.bg_fill = SELECTION();
        v.selection.stroke = Stroke::new(1.0f32, EMBER2());
        v.override_text_color = None;
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = SIDE();
        w.noninteractive.weak_bg_fill = SIDE();
        w.noninteractive.bg_stroke = Stroke::new(1.0f32, LINE());
        w.noninteractive.fg_stroke = Stroke::new(1.0f32, BODY());
        w.inactive.bg_fill = ELEV();
        w.inactive.weak_bg_fill = ELEV();
        // (A skin can outline buttons and boxes: crisper, like an older program's.)
        w.inactive.bg_stroke = if outlines() { Stroke::new(1.0f32, mix(LINE(), TEXT(), 0.12)) } else { Stroke::NONE };
        w.inactive.fg_stroke = Stroke::new(1.0f32, TEXT());
        w.inactive.corner_radius = r(6);
        w.hovered.bg_fill = ELEV2();
        w.hovered.weak_bg_fill = ELEV2();
        w.hovered.bg_stroke = Stroke::new(1.0f32, if skinned { mix(ELEV2(), TEXT(), 0.12) } else { Color32::from_rgb(0x55, 0x48, 0x3f) });
        w.hovered.fg_stroke = Stroke::new(1.5f32, TEXT());
        w.hovered.corner_radius = r(6);
        w.active.bg_fill = active;
        w.active.weak_bg_fill = active;
        w.active.fg_stroke = Stroke::new(1.5f32, EMBER2());
        w.active.corner_radius = r(6);
        w.open.bg_fill = ELEV2();
        w.open.weak_bg_fill = ELEV2();
        s.visuals = v;
    });
}

/// Primary (ember) button.
pub fn primary_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_string()).color(on_ember()).font(bold_font(14.0)))
        .fill(EMBER())
        .corner_radius(r(6))
}

pub fn danger_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_string()).color(Color32::WHITE).font(bold_font(14.0)))
        .fill(LIVE())
        .corner_radius(r(6))
}

/// Window icon: the campfire from `assets/icon` (made by `tools/make-icons.py`).
pub fn app_icon() -> egui::IconData {
    let rgba = include_bytes!("../assets/icon/pyr-128.rgba").to_vec();
    egui::IconData { rgba, width: 128, height: 128 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_is_the_skins_list_in_its_order() {
        assert_eq!(NAMES.len(), pyr_proto::skin::COLORS.len());
        for (name, i) in NAMES {
            assert_eq!(pyr_proto::skin::color_index(&name.trim_end_matches("_SOLID").to_ascii_lowercase()), Some(*i), "{name}");
        }
        // Campfire's colors as before skins.
        assert_eq!(MAIN(), Color32::from_rgb(0x26, 0x20, 0x1c));
        assert_eq!(SELECTION(), Color32::from_rgb(0x8a, 0x40, 0x1c));
        assert_eq!(on_ember(), Color32::from_rgb(0x1b, 0x14, 0x11));
        assert_eq!(panel(MAIN(), true), MAIN(), "no picture: as it is");
    }
}
