//! Pyr Casino's pictures: the floor, the tables, every game's tile and the slots' symbols, made
//! from Meshy renders by `tools/make-casino-art.py` and kept in the app (WebP). Each is read
//! the first time it's wanted, off the screen's thread, and kept as a texture after that
//! (at the size asked for: one drawn small is made small first, so it stays smooth).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use eframe::egui::{self, Color32, ColorImage, Pos2, Rect, TextureHandle, TextureOptions, pos2};

macro_rules! pics {
    ($($name:literal),* $(,)?) => {
        &[$(($name, include_bytes!(concat!("../../assets/casino/", $name, ".webp")))),*]
    };
}

/// Every picture, by name.
const PICS: &[(&str, &[u8])] = pics![
    "floor",
    "dragon-lair",
    "bonfire",
    "emotes",
    "table-poker",
    "table-blackjack",
    "table-baccarat",
    "felt-green",
    "card-back",
    "chip",
    "tile-sevens",
    "tile-blackjack",
    "tile-dragon",
    "tile-roulette",
    "tile-baccarat",
    "tile-craps",
    "tile-dice",
    "tile-keno",
    "tile-phoenix",
    "tile-tables",
    "tile-timebombs",
    "tile-tourney",
    "tile-friday",
    "sevens-symbols",
    "dragon-symbols",
    "dragon-age1",
    "dragon-age2",
    "dragon-age3",
    "dragon-age4",
    "dragon-age5",
    "reels-scoville-tile",
    "reels-scoville-symbols",
    "reels-deepsea-tile",
    "reels-deepsea-symbols",
    "reels-cosmic-tile",
    "reels-cosmic-symbols",
    "reels-sugar-tile",
    "reels-sugar-symbols",
    "reels-goldrush-tile",
    "reels-goldrush-symbols",
    "reels-neon-tile",
    "reels-neon-symbols",
    "reels-tomb-tile",
    "reels-tomb-symbols",
    "reels-frost-tile",
    "reels-frost-symbols",
    "reels-cauldron-tile",
    "reels-cauldron-symbols",
    "reels-hoard-tile",
    "reels-hoard-symbols",
    "reels-grind-tile",
    "reels-grind-symbols",
    "reels-cabaret-tile",
    "reels-cabaret-symbols",
    "host-scoville",
    "host-sugar",
    "host-grind",
    "host-frost",
];

enum Slot {
    Reading,
    Read(ColorImage),
    Ready(TextureHandle),
    Failed,
}

/// (name, the size it was made for: 0 as it is) → where it's got to.
fn cache() -> &'static Mutex<HashMap<(&'static str, u32), Slot>> {
    static C: OnceLock<Mutex<HashMap<(&'static str, u32), Slot>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn bytes(name: &str) -> Option<(&'static str, &'static [u8])> {
    PICS.iter().find(|(n, _)| *n == name).map(|(n, b)| (*n, *b))
}

/// The picture as a texture, at most `max_px` across or down (0: as it is); None while it's
/// still being read (the screen is drawn again when it's ready).
pub fn tex(ctx: &egui::Context, name: &str, max_px: u32) -> Option<TextureHandle> {
    let (name, data) = bytes(name)?;
    // Sizes go in steps, so a picture drawn a little bigger or smaller keeps its texture (and
    // in the screen's own pixels: a scaled-up screen gets a sharper one).
    let max_px = (max_px as f32 * ctx.pixels_per_point()).round() as u32;
    let size = if max_px == 0 { 0 } else { max_px.next_power_of_two().max(32) };
    let key = (name, size);
    let mut c = cache().lock().ok()?;
    match c.get_mut(&key) {
        Some(Slot::Ready(t)) => return Some(t.clone()),
        Some(Slot::Reading) | Some(Slot::Failed) => return None,
        Some(slot @ Slot::Read(_)) => {
            let Slot::Read(img) = std::mem::replace(slot, Slot::Failed) else { return None };
            let t = ctx.load_texture(format!("casino-{name}-{size}"), img, TextureOptions::LINEAR);
            *slot = Slot::Ready(t.clone());
            return Some(t);
        }
        None => {}
    }
    c.insert(key, Slot::Reading);
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let read = image::load_from_memory_with_format(data, image::ImageFormat::WebP).map(|im| {
            let im = if size > 0 && (im.width() > size || im.height() > size) { im.resize(size, size, image::imageops::FilterType::Lanczos3) } else { im };
            let rgba = im.to_rgba8();
            ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
        });
        if let Ok(mut c) = cache().lock() {
            c.insert(
                key,
                match read {
                    Ok(img) => Slot::Read(img),
                    Err(e) => {
                        tracing::warn!("casino picture {name} couldn't be read: {e}");
                        Slot::Failed
                    }
                },
            );
        }
        ctx.request_repaint();
    });
    None
}

/// The part of a picture `size` big that covers `r` (cut to `r`'s shape about `focus`, a
/// fraction across and down: the middle is (0.5, 0.5)).
fn cover_uv(size: [usize; 2], r: Rect, focus: (f32, f32)) -> Rect {
    let (iw, ih) = (size[0] as f32, size[1] as f32);
    let (want, have) = (r.width() / r.height().max(1.0), iw / ih.max(1.0));
    if have > want {
        let k = want / have;
        let x = ((focus.0 - k / 2.0).max(0.0)).min(1.0 - k);
        Rect::from_min_max(pos2(x, 0.0), pos2(x + k, 1.0))
    } else {
        let k = have / want;
        let y = ((focus.1 - k / 2.0).max(0.0)).min(1.0 - k);
        Rect::from_min_max(pos2(0.0, y), pos2(1.0, y + k))
    }
}

/// Draws a picture to fill `r` (cut to `r`'s shape about its middle, like a cover), tinted.
pub fn cover(p: &egui::Painter, ctx: &egui::Context, name: &str, r: Rect, max_px: u32, tint: Color32) -> bool {
    let Some(t) = tex(ctx, name, max_px) else { return false };
    p.image(t.id(), r, cover_uv(t.size(), r, (0.5, 0.5)), tint);
    true
}

/// Like [`cover`], its corners rounded by `radius`, cut about `focus` (a fraction of the
/// picture across and down).
#[allow(clippy::too_many_arguments)]
pub fn cover_rounded(p: &egui::Painter, ctx: &egui::Context, name: &str, r: Rect, radius: f32, focus: (f32, f32), max_px: u32, tint: Color32) -> bool {
    let Some(t) = tex(ctx, name, max_px) else { return false };
    let uv = cover_uv(t.size(), r, focus);
    let rad = radius.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    let to_uv = |q: Pos2| pos2(uv.left() + (q.x - r.left()) / r.width().max(1.0) * uv.width(), uv.top() + (q.y - r.top()) / r.height().max(1.0) * uv.height());
    let mut m = egui::Mesh::with_texture(t.id());
    let c = r.center();
    m.vertices.push(egui::epaint::Vertex { pos: c, uv: to_uv(c), color: tint });
    let seg = 8;
    // Clockwise from the bottom right (y grows downwards).
    let corners = [(pos2(r.right() - rad, r.bottom() - rad), 0.0f32), (pos2(r.left() + rad, r.bottom() - rad), 90.0), (pos2(r.left() + rad, r.top() + rad), 180.0), (pos2(r.right() - rad, r.top() + rad), 270.0)];
    for (cc, a0) in corners {
        for i in 0..=seg {
            let a = (a0 + 90.0 * i as f32 / seg as f32).to_radians();
            let q = cc + egui::vec2(a.cos(), a.sin()) * rad;
            m.vertices.push(egui::epaint::Vertex { pos: q, uv: to_uv(q), color: tint });
        }
    }
    let n = 4 * (seg + 1) as u32;
    for i in 1..=n {
        m.add_triangle(0, i, if i == n { 1 } else { i + 1 });
    }
    p.add(egui::Shape::mesh(m));
    true
}

/// One cell of a 3 x 3 sheet of symbols (0 to 8, left to right, top to bottom) drawn in `r`.
/// (`max_px` is for the whole sheet: three cells across.)
pub fn cell(p: &egui::Painter, ctx: &egui::Context, sheet: &str, i: usize, r: Rect, max_px: u32, tint: Color32) -> bool {
    let Some(t) = tex(ctx, sheet, max_px) else { return false };
    let (cx, cy) = ((i % 3) as f32, (i / 3) as f32);
    let uv = Rect::from_min_max(pos2(cx / 3.0, cy / 3.0), pos2((cx + 1.0) / 3.0, (cy + 1.0) / 3.0));
    p.image(t.id(), r, uv, tint);
    true
}

/// The part of a picture (a fraction of it, from 0 to 1 each way) drawn in `r`.
pub fn part(p: &egui::Painter, ctx: &egui::Context, name: &str, uv: Rect, r: Rect, max_px: u32, tint: Color32) -> bool {
    let Some(t) = tex(ctx, name, max_px) else { return false };
    p.image(t.id(), r, uv, tint);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_picture_reads() {
        for (name, data) in PICS {
            let im = image::load_from_memory_with_format(data, image::ImageFormat::WebP).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(im.width() >= 64 && im.height() >= 64, "{name}");
        }
    }
}
