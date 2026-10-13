//! Colour emoji, like Discord: Twemoji pictures (graphics CC-BY 4.0 by Twitter/X and
//! contributors) drawn over the space text layout leaves for them, plus each server's own
//! uploaded emoji (written `<:name:id>` in messages, `<a:name:id>` when animated).
//!
//! The text keeps the real characters (so copy and paste work); they're set in one of two
//! placeholder fonts — "blank" for the first character of an emoji (1em wide, invisible) and
//! "zero" for the rest (no width) — and the picture is painted where the blank one landed.
//! See `scripts/make-emoji.py` for how the assets are made.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

use eframe::egui::{self, Color32, FontFamily, FontId, Pos2, Rect, pos2};
use serde::Deserialize;

static INDEX: &[u8] = include_bytes!("../assets/emoji/index.json");
static ATLAS_PNG: &[u8] = include_bytes!("../assets/emoji/atlas.png");
pub static BLANK_TTF: &[u8] = include_bytes!("../assets/emoji/blank.ttf");
pub static ZERO_TTF: &[u8] = include_bytes!("../assets/emoji/zero.ttf");

/// Emoji are this much bigger than the text around them (22px on 16px text in Discord).
pub const SCALE: f32 = 1.375;
/// A message that's only emoji (up to this many) shows them big.
pub const JUMBO_MAX: usize = 27;
pub const JUMBO_SIZE: f32 = 48.0;

pub fn blank_family() -> FontFamily {
    FontFamily::Name("emoji-blank".into())
}
pub fn zero_family() -> FontFamily {
    FontFamily::Name("emoji-zero".into())
}

#[derive(Deserialize)]
struct RawIndex {
    cell: u32,
    cols: u32,
    files: Vec<String>,
    text_default: Vec<String>,
    groups: Vec<Group>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Group {
    pub name: String,
    pub items: Vec<Item>,
}

#[derive(Deserialize, Clone, Debug)]
pub struct Item {
    /// The emoji itself.
    pub e: String,
    /// Its name ("grinning face").
    pub n: String,
    /// Twemoji key (atlas cell lookup).
    pub k: String,
    /// Shortcodes, without colons ("grinning").
    #[serde(default)]
    pub s: Vec<String>,
    /// The five skin tones: (emoji, key).
    #[serde(default)]
    pub t: Option<Vec<(String, String)>>,
}

impl Item {
    /// This emoji in a skin tone (0 = default, 1–5), if it has them.
    pub fn toned(&self, tone: u8) -> (&str, &str) {
        match (&self.t, tone) {
            (Some(t), 1..=5) => (&t[tone as usize - 1].0, &t[tone as usize - 1].1),
            _ => (&self.e, &self.k),
        }
    }
}

pub struct Data {
    pub cell: u32,
    pub cols: u32,
    pub rows: u32,
    cells: HashMap<String, u16>,
    text_default: HashSet<String>,
    first: HashSet<char>,
    pub groups: Vec<Group>,
    /// Shortcode → (group, item).
    pub by_short: HashMap<String, (usize, usize)>,
}

pub fn data() -> &'static Data {
    static D: OnceLock<Data> = OnceLock::new();
    D.get_or_init(|| {
        let raw: RawIndex = serde_json::from_slice(INDEX).expect("emoji index is valid");
        let mut cells = HashMap::new();
        let mut first = HashSet::new();
        for (i, f) in raw.files.iter().enumerate() {
            cells.insert(f.clone(), i as u16);
            // Twemoji keeps the emoji selector (fe0f) in some names; text is matched without.
            let bare: Vec<&str> = f.split('-').filter(|p| *p != "fe0f").collect();
            cells.entry(bare.join("-")).or_insert(i as u16);
            if let Some(c) = f.split('-').next().and_then(|h| u32::from_str_radix(h, 16).ok()).and_then(char::from_u32) {
                first.insert(c);
            }
        }
        let mut by_short = HashMap::new();
        for (g, grp) in raw.groups.iter().enumerate() {
            for (i, it) in grp.items.iter().enumerate() {
                for s in &it.s {
                    by_short.entry(s.clone()).or_insert((g, i));
                }
            }
        }
        let rows = (raw.files.len() as u32).div_ceil(raw.cols);
        Data {
            cell: raw.cell,
            cols: raw.cols,
            rows,
            cells,
            text_default: raw.text_default.into_iter().collect(),
            first,
            groups: raw.groups,
            by_short,
        }
    })
}

/// One emoji found in text.
#[derive(Clone, Debug, PartialEq)]
pub enum EmojiRef {
    /// A Twemoji atlas cell.
    Unicode(u16),
    /// A server's own emoji: (name, file id, animated).
    Custom { name: String, id: String, animated: bool },
}

fn key_of(chars: &[char]) -> String {
    chars.iter().filter(|c| **c != '\u{fe0f}').map(|c| format!("{:x}", *c as u32)).collect::<Vec<_>>().join("-")
}

/// The atlas cell for an emoji written as text (if it's one).
pub fn cell_of(s: &str) -> Option<u16> {
    let chars: Vec<char> = s.chars().collect();
    match match_at(&chars, 0) {
        Some((n, EmojiRef::Unicode(c))) if n == chars.len() => Some(c),
        _ => None,
    }
}

/// The emoji starting at `chars[i]`, if any: how many chars it spans and what it is.
pub fn match_at(chars: &[char], i: usize) -> Option<(usize, EmojiRef)> {
    let c = *chars.get(i)?;
    if c == '<' {
        return custom_at(chars, i);
    }
    let d = data();
    if !d.first.contains(&c) {
        return None;
    }
    let max = (chars.len() - i).min(14);
    for len in (1..=max).rev() {
        let seq = &chars[i..i + len];
        if seq.last() == Some(&'\u{fe0f}') && len > 1 {
            // Counted below as trailing.
            continue;
        }
        let key = key_of(seq);
        let Some(&cell) = d.cells.get(&key) else { continue };
        let mut end = i + len;
        let has_fe0f = chars.get(end) == Some(&'\u{fe0f}') || seq.contains(&'\u{fe0f}');
        // Characters like ❤ or © are plain text unless followed by the emoji selector.
        if !key.contains('-') && d.text_default.contains(&key) && !has_fe0f {
            return None;
        }
        // Digits, # and * are only emoji as keycaps.
        if len == 1 && (c.is_ascii()) {
            return None;
        }
        while chars.get(end) == Some(&'\u{fe0f}') {
            end += 1;
        }
        return Some((end - i, EmojiRef::Unicode(cell)));
    }
    None
}

/// `<:name:id>` or `<a:name:id>`.
fn custom_at(chars: &[char], i: usize) -> Option<(usize, EmojiRef)> {
    let rest: String = chars[i..chars.len().min(i + 110)].iter().collect();
    let close = rest.find('>')?;
    let inner = &rest[1..close];
    let (animated, inner) = match inner.strip_prefix("a:") {
        Some(r) => (true, r),
        None => (false, inner.strip_prefix(':')?),
    };
    let (name, id) = inner.split_once(':')?;
    let ok_name = (2..=32).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    let ok_id = (8..=80).contains(&id.len()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !(ok_name && ok_id) {
        return None;
    }
    Some((rest[..=close].chars().count(), EmojiRef::Custom { name: name.into(), id: id.into(), animated }))
}

/// Every emoji in `text`: (char index, char count, emoji).
pub fn find_all(text: &str) -> Vec<(usize, usize, EmojiRef)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if let Some((n, e)) = match_at(&chars, i) {
            out.push((i, n, e));
            i += n;
        } else {
            i += 1;
        }
    }
    out
}

/// A message that's nothing but (a few) emoji and spaces: shown big.
pub fn is_jumbo(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let found = find_all(text);
    if found.is_empty() || found.len() > JUMBO_MAX {
        return false;
    }
    let covered: usize = found.iter().map(|f| f.1).sum();
    let spaces = chars.iter().filter(|c| c.is_whitespace()).count();
    covered + spaces == chars.len()
}

/// Text formats holding the space for an emoji at text size `size`.
pub fn formats(size: f32) -> (egui::text::TextFormat, egui::text::TextFormat) {
    let s = size;
    let mut lead = egui::text::TextFormat::simple(FontId::new(s, blank_family()), Color32::TRANSPARENT);
    lead.valign = egui::Align::Center;
    let mut rest = egui::text::TextFormat::simple(FontId::new(s, zero_family()), Color32::TRANSPARENT);
    rest.valign = egui::Align::Center;
    (lead, rest)
}

/// Where each emoji (by its first char index) ended up in a laid-out galley.
pub fn placed(galley: &egui::Galley, starts: &[usize]) -> Vec<Rect> {
    let mut want: Vec<(usize, usize)> = starts.iter().copied().enumerate().map(|(n, s)| (s, n)).collect();
    want.sort();
    let mut out = vec![Rect::NOTHING; starts.len()];
    let mut idx = 0usize;
    let mut w = 0usize;
    for row in &galley.rows {
        for g in &row.glyphs {
            while w < want.len() && want[w].0 < idx {
                w += 1;
            }
            if w < want.len() && want[w].0 == idx {
                let size = g.advance_width;
                let cy = row.pos.y + row.size.y / 2.0;
                let x = row.pos.x + g.pos.x;
                // A hair of space around each, like Discord.
                let r = Rect::from_min_max(pos2(x, cy - size / 2.0), pos2(x + size, cy + size / 2.0));
                out[want[w].1] = r.shrink(size * 0.045);
                w += 1;
            }
            idx += 1;
        }
        if row.ends_with_newline {
            idx += 1;
        }
    }
    out
}

// ------------------------------------------------------------------ the atlas texture

struct Atlas {
    image: Mutex<Option<egui::ColorImage>>,
    tex: OnceLock<egui::TextureHandle>,
}

fn atlas_state() -> &'static Arc<Atlas> {
    static A: OnceLock<Arc<Atlas>> = OnceLock::new();
    A.get_or_init(|| {
        let a = Arc::new(Atlas { image: Mutex::new(None), tex: OnceLock::new() });
        let a2 = a.clone();
        let _ = std::thread::Builder::new().name("ingle-emoji".into()).spawn(move || {
            if let Ok(img) = image::load_from_memory(ATLAS_PNG) {
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                *a2.image.lock().unwrap() = Some(egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()));
            }
        });
        a
    })
}

/// Start decoding the emoji pictures (call early so they're ready when first needed).
pub fn preload() {
    let _ = atlas_state();
    let _ = data();
}

/// The atlas texture, once decoded.
pub fn atlas(ctx: &egui::Context) -> Option<egui::TextureId> {
    let a = atlas_state();
    if let Some(t) = a.tex.get() {
        return Some(t.id());
    }
    let img = a.image.lock().unwrap().take();
    match img {
        Some(img) => {
            let t = ctx.load_texture("emoji-atlas", img, egui::TextureOptions::LINEAR);
            let id = t.id();
            let _ = a.tex.set(t);
            Some(id)
        }
        None => {
            ctx.request_repaint_after(std::time::Duration::from_millis(60));
            None
        }
    }
}

/// Where an emoji's picture is in the atlas, half a pixel in from each side of its cell. The
/// emoji sit edge to edge in the atlas and most touch their cell's edge, so smooth sampling
/// right at the edge blends in the neighbouring emoji: thin lines around the picture.
pub fn uv(cell: u16) -> Rect {
    let d = data();
    let (c, r) = ((cell as u32 % d.cols) as f32, (cell as u32 / d.cols) as f32);
    let (w, h) = ((d.cols * d.cell) as f32, (d.rows * d.cell) as f32);
    let cs = d.cell as f32;
    Rect::from_min_max(
        pos2((c * cs + 0.5) / w, (r * cs + 0.5) / h),
        pos2(((c + 1.0) * cs - 0.5) / w, ((r + 1.0) * cs - 0.5) / h),
    )
}

/// Draws one emoji filling `rect`. `custom` turns a server emoji's file id into a texture.
pub fn paint(
    painter: &egui::Painter,
    rect: Rect,
    e: &EmojiRef,
    custom: &mut dyn FnMut(&str) -> Option<egui::TextureId>,
) {
    paint_faded(painter, rect, e, custom, 1.0);
}

/// `paint`, see-through by `alpha` (0 to 1), for emoji that fade away.
pub fn paint_faded(
    painter: &egui::Painter,
    rect: Rect,
    e: &EmojiRef,
    custom: &mut dyn FnMut(&str) -> Option<egui::TextureId>,
    alpha: f32,
) {
    let tint = Color32::WHITE.gamma_multiply(alpha.clamp(0.0, 1.0));
    match e {
        EmojiRef::Unicode(cell) => {
            if let Some(t) = atlas(painter.ctx()) {
                // A plain textured quad: egui's image shape adds a soft half-pixel rim whose
                // texture coordinates reach past this cell into the next emoji in the atlas.
                let mut mesh = egui::Mesh::with_texture(t);
                mesh.add_rect_with_uv(rect, uv(*cell), tint);
                painter.add(egui::Shape::mesh(mesh));
            }
        }
        EmojiRef::Custom { id, .. } => {
            if let Some(t) = custom(id) {
                painter.image(t, rect, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), tint);
            } else {
                painter.rect_filled(rect.shrink(rect.width() * 0.15), 4.0, Color32::from_white_alpha((14.0 * alpha.clamp(0.0, 1.0)) as u8));
            }
        }
    }
}

/// An emoji as a widget (picker cells, reactions).
pub fn widget(
    ui: &mut egui::Ui,
    size: f32,
    e: &EmojiRef,
    custom: &mut dyn FnMut(&str) -> Option<egui::TextureId>,
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    paint(ui.painter(), rect, e, custom);
    resp
}

/// Text with colour emoji, laid out and drawn (one line or wrapped).
pub fn label(
    ui: &mut egui::Ui,
    text: &str,
    font: FontId,
    color: Color32,
    wrap: Option<f32>,
    custom: &mut dyn FnMut(&str) -> Option<egui::TextureId>,
) -> egui::Response {
    let mut job = egui::text::LayoutJob::default();
    if let Some(w) = wrap {
        job.wrap.max_width = w;
    }
    let found = find_all(text);
    let chars: Vec<char> = text.chars().collect();
    let plain = egui::text::TextFormat::simple(font.clone(), color);
    let (lead, rest) = formats(font.size * SCALE);
    let mut i = 0;
    for (start, n, _) in &found {
        if *start > i {
            job.append(&chars[i..*start].iter().collect::<String>(), 0.0, plain.clone());
        }
        job.append(&chars[*start].to_string(), 0.0, lead.clone());
        if *n > 1 {
            job.append(&chars[start + 1..start + n].iter().collect::<String>(), 0.0, rest.clone());
        }
        i = start + n;
    }
    if i < chars.len() {
        job.append(&chars[i..].iter().collect::<String>(), 0.0, plain);
    }
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let resp = ui.add(egui::Label::new(galley.clone()).sense(egui::Sense::click()));
    let starts: Vec<usize> = found.iter().map(|f| f.0).collect();
    for (r, f) in placed(&galley, &starts).into_iter().zip(&found) {
        if r.is_positive() {
            paint(ui.painter(), r.translate(resp.rect.min.to_vec2()), &f.2, custom);
        }
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_emoji_in_text() {
        let f = find_all("hi 😀 there 👍🏽 and 🇺🇸 ok");
        assert_eq!(f.len(), 3, "{f:?}");
        assert_eq!(f[0].0, 3);
        assert_eq!(f[1].1, 2); // thumbs up + skin tone
        assert_eq!(f[2].1, 2); // flag
        // Family (ZWJ sequence) is one emoji.
        let fam = "👨‍👩‍👧";
        assert_eq!(find_all(fam).len(), 1);
        assert_eq!(find_all(fam)[0].1, fam.chars().count());
        // Keycap, heart with and without the emoji selector, plain text.
        assert_eq!(find_all("1\u{fe0f}\u{20e3}").len(), 1);
        assert_eq!(find_all("❤\u{fe0f}").len(), 1);
        assert_eq!(find_all("© 2026 ❤").len(), 0);
        assert!(find_all("abc 123 #tag *x*").is_empty());
    }

    #[test]
    fn custom_emoji_and_jumbo() {
        let f = find_all("nice <:party_cat:abcdEFGH1234> <a:dance:xyz-12345678>");
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].2, EmojiRef::Custom { name: "party_cat".into(), id: "abcdEFGH1234".into(), animated: false });
        assert!(matches!(&f[1].2, EmojiRef::Custom { animated: true, .. }));
        assert!(find_all("<b>bold</b> <:x:short>").is_empty());
        assert!(is_jumbo("😀 😀"));
        assert!(is_jumbo("<:party_cat:abcdEFGH1234>"));
        assert!(!is_jumbo("😀 hi"));
        assert!(!is_jumbo(""));
    }

    #[test]
    fn picker_data_is_complete() {
        let d = data();
        let n: usize = d.groups.iter().map(|g| g.items.len()).sum();
        assert!(n > 1500, "{n}");
        assert!(d.by_short.contains_key("joy") || d.by_short.contains_key("grinning"));
        for g in &d.groups {
            for it in &g.items {
                assert!(cell_of(&it.e).is_some() || d.cells.contains_key(&it.k), "{} {}", it.n, it.e);
            }
        }
        // Each picture stays inside its own cell, half a pixel in from every side.
        let (w, h) = ((d.cols * d.cell) as f32, (d.rows * d.cell) as f32);
        for cell in [0u16, 1, (d.cols + 1) as u16, (d.cols * (d.rows - 1)) as u16] {
            let (c, r) = ((cell as u32 % d.cols) as f32, (cell as u32 / d.cols) as f32);
            let u = uv(cell);
            let cs = d.cell as f32;
            assert!((u.min.x * w - (c * cs + 0.5)).abs() < 1e-3 && (u.min.y * h - (r * cs + 0.5)).abs() < 1e-3);
            assert!((u.max.x * w - ((c + 1.0) * cs - 0.5)).abs() < 1e-3 && (u.max.y * h - ((r + 1.0) * cs - 0.5)).abs() < 1e-3);
        }
    }
}
