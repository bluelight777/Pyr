//! Images in the UI: attachments (still and animated GIFs) and profile pictures.
//!
//! Decoding happens on a background thread so a big GIF never stalls the window; the UI
//! uploads the finished frames as textures and plays them back by time.

use std::collections::HashMap;
use std::sync::mpsc;
use std::time::Duration;

use eframe::egui;
use image::{AnimationDecoder, ImageDecoder};

/// Largest side kept for chat images (the viewer shows them up to this size too).
const MAX_SIDE: u32 = 1600;
/// Animated GIFs: frames are kept smaller, and there's a cap on how many.
const MAX_ANIM_SIDE: u32 = 640;
const MAX_FRAMES: usize = 600;

pub struct Decoded {
    pub frames: Vec<(egui::ColorImage, Duration)>,
}

pub enum Tex {
    Loading,
    Ready { frames: Vec<(egui::TextureHandle, Duration)>, size: egui::Vec2, total: Duration },
    Failed(String),
}

impl Tex {
    /// The frame to show now (animated images loop).
    pub fn frame(&self, t: f64) -> Option<(egui::TextureId, egui::Vec2, Option<Duration>)> {
        let Tex::Ready { frames, size, total } = self else { return None };
        if frames.len() == 1 || total.is_zero() {
            return Some((frames[0].0.id(), *size, None));
        }
        let mut at = Duration::from_secs_f64(t % total.as_secs_f64());
        for (tex, d) in frames {
            if at < *d {
                return Some((tex.id(), *size, Some(*d - at)));
            }
            at -= *d;
        }
        Some((frames[0].0.id(), *size, Some(frames[0].1)))
    }
}

/// Decodes images on a worker thread and keeps the textures, keyed by an id (a file id, or
/// "avatar:" + file id for the small round version).
pub struct Images {
    tex: HashMap<String, Tex>,
    tx: mpsc::Sender<(String, std::sync::Arc<Vec<u8>>, u32)>,
    rx: mpsc::Receiver<(String, Result<Decoded, String>)>,
}

impl Default for Images {
    fn default() -> Self {
        let (tx, jobs) = mpsc::channel::<(String, std::sync::Arc<Vec<u8>>, u32)>();
        let (done, rx) = mpsc::channel();
        let _ = std::thread::Builder::new().name("ingle-images".into()).spawn(move || {
            while let Ok((key, bytes, max_side)) = jobs.recv() {
                let r = decode(&bytes, max_side);
                if done.send((key, r)).is_err() {
                    break;
                }
            }
        });
        Images { tex: HashMap::new(), tx, rx }
    }
}

impl Images {
    /// The texture for `key`, decoding `bytes` (once) if needed. `max_side` limits the size.
    pub fn get(&mut self, ctx: &egui::Context, key: &str, bytes: Option<std::sync::Arc<Vec<u8>>>, max_side: u32) -> &Tex {
        self.collect(ctx);
        if !self.tex.contains_key(key) {
            if let Some(b) = bytes {
                self.tex.insert(key.to_string(), Tex::Loading);
                let _ = self.tx.send((key.to_string(), b, max_side));
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }
        self.tex.get(key).unwrap_or(&Tex::Loading)
    }

    pub fn forget(&mut self, key: &str) {
        self.tex.remove(key);
    }

    /// Upload any finished decodes.
    fn collect(&mut self, ctx: &egui::Context) {
        while let Ok((key, r)) = self.rx.try_recv() {
            let t = match r {
                Ok(d) => {
                    let size = d.frames.first().map(|f| egui::vec2(f.0.size[0] as f32, f.0.size[1] as f32)).unwrap_or_default();
                    let total = d.frames.iter().map(|f| f.1).sum();
                    let frames = d
                        .frames
                        .into_iter()
                        .enumerate()
                        .map(|(i, (img, delay))| (ctx.load_texture(format!("{key}#{i}"), img, egui::TextureOptions::LINEAR), delay))
                        .collect::<Vec<_>>();
                    if frames.is_empty() { Tex::Failed("empty image".into()) } else { Tex::Ready { frames, size, total } }
                }
                Err(e) => Tex::Failed(e),
            };
            self.tex.insert(key, t);
            ctx.request_repaint();
        }
    }
}

fn to_color_image(img: &image::RgbaImage) -> egui::ColorImage {
    egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw())
}

fn shrink(img: image::RgbaImage, max_side: u32) -> image::RgbaImage {
    let (w, h) = img.dimensions();
    if w.max(h) <= max_side {
        return img;
    }
    let s = max_side as f32 / w.max(h) as f32;
    let (nw, nh) = (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1));
    image::imageops::resize(&img, nw, nh, image::imageops::FilterType::Triangle)
}

/// Decode a PNG, JPEG, WebP or GIF (all its frames) into textures-to-be.
pub fn decode(bytes: &[u8], max_side: u32) -> Result<Decoded, String> {
    let fmt = image::guess_format(bytes).map_err(|e| e.to_string())?;
    if fmt == image::ImageFormat::Gif {
        let dec = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
        let mut frames = Vec::new();
        for f in dec.into_frames().take(MAX_FRAMES) {
            let f = f.map_err(|e| e.to_string())?;
            let (n, d) = f.delay().numer_denom_ms();
            let ms = if d == 0 { 100 } else { n / d };
            // Browsers treat very short delays as 100 ms; so do we.
            let delay = Duration::from_millis(if ms < 20 { 100 } else { ms as u64 });
            let img = shrink(f.into_buffer(), max_side.min(MAX_ANIM_SIDE));
            frames.push((to_color_image(&img), delay));
        }
        if frames.len() == 1 {
            // A still GIF can be as sharp as any other image.
            let img = image::load_from_memory_with_format(bytes, fmt).map_err(|e| e.to_string())?.to_rgba8();
            frames = vec![(to_color_image(&shrink(img, max_side)), Duration::ZERO)];
        }
        return Ok(Decoded { frames });
    }
    let img = image::load_from_memory_with_format(bytes, fmt).map_err(|e| e.to_string())?.to_rgba8();
    Ok(Decoded { frames: vec![(to_color_image(&shrink(img, max_side)), Duration::ZERO)] })
}

pub fn max_side_for_chat() -> u32 {
    MAX_SIDE
}

/// A profile picture from any image: the middle square, 256×256, as PNG.
pub fn make_avatar(bytes: &[u8]) -> Result<Vec<u8>, String> {
    make_avatar_crop(bytes, 0.5, 0.5, 1.0)
}

/// A profile picture from part of an image: the square centred at (`cx`, `cy`) (0–1 across
/// and down the image) whose side is `frac` of the image's shorter side.
pub fn make_avatar_crop(bytes: &[u8], cx: f32, cy: f32, frac: f32) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(bytes).map_err(|_| "That file isn't an image Ingle can read.".to_string())?.to_rgba8();
    let (w, h) = img.dimensions();
    let side = ((w.min(h) as f32 * frac.clamp(0.05, 1.0)).round() as u32).clamp(1, w.min(h));
    let x = ((cx * w as f32 - side as f32 / 2.0).round().max(0.0) as u32).min(w - side);
    let y = ((cy * h as f32 - side as f32 / 2.0).round().max(0.0) as u32).min(h - side);
    let square = image::imageops::crop_imm(&img, x, y, side, side).to_image();
    let small = image::imageops::resize(&square, 256, 256, image::imageops::FilterType::Lanczos3);
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(small)
        .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out)
}

/// An image from the clipboard (a screenshot, say), as PNG.
pub fn clipboard_png() -> Option<Vec<u8>> {
    let mut cb = arboard::Clipboard::new().ok()?;
    let img = cb.get_image().ok()?;
    let rgba = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned())?;
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(rgba).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
    Some(out)
}

/// Image files people can attach (checked by content, not by name).
pub fn check_attachment(name: &str, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() as u64 > ingle_proto::control::MAX_ATTACHMENT_BYTES {
        return Err(format!("{name} is over 10 MB."));
    }
    if ingle_proto::control::sniff_image(bytes).is_none() {
        return Err(format!("{name} isn't a PNG, JPEG, GIF or WebP image."));
    }
    Ok(())
}

/// The first frame's size without decoding everything (for layout before the image loads).
#[allow(dead_code)]
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let r = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let d = r.into_decoder().ok()?;
    Some(d.dimensions())
}

/// Pick image files with the system's file dialog (multiple).
pub fn pick_images() -> Vec<(String, Vec<u8>)> {
    let Some(paths) = rfd::FileDialog::new()
        .set_title("Choose images")
        .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
        .pick_files()
    else {
        return Vec::new();
    };
    read_files(&paths)
}

pub fn pick_image() -> Option<(String, Vec<u8>)> {
    let path = rfd::FileDialog::new()
        .set_title("Choose a picture")
        .add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp"])
        .pick_file()?;
    read_files(&[path]).into_iter().next()
}

pub fn read_files(paths: &[std::path::PathBuf]) -> Vec<(String, Vec<u8>)> {
    paths
        .iter()
        .filter_map(|p| {
            let name = p.file_name()?.to_string_lossy().to_string();
            Some((name, std::fs::read(p).ok()?))
        })
        .collect()
}

/// Save an image where the person chooses.
pub fn save_as(name: &str, bytes: &[u8]) -> Result<bool, String> {
    let Some(path) = rfd::FileDialog::new().set_title("Save image").set_file_name(name).save_file() else { return Ok(false) };
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 100, 50, 255]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn avatars_are_square_256_pngs() {
        let a = make_avatar(&png(400, 300)).unwrap();
        let img = image::load_from_memory(&a).unwrap();
        assert_eq!((img.width(), img.height()), (256, 256));
        assert!(make_avatar(b"not an image").is_err());
    }

    #[test]
    fn decodes_and_shrinks() {
        let d = decode(&png(3200, 100), 1600).unwrap();
        assert_eq!(d.frames.len(), 1);
        assert_eq!(d.frames[0].0.size, [1600, 50]);
    }

    #[test]
    fn decodes_animated_gifs() {
        use image::codecs::gif::GifEncoder;
        let mut out = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut out);
            for i in 0..3u8 {
                let img = image::RgbaImage::from_pixel(20, 10, image::Rgba([i * 80, 0, 0, 255]));
                let f = image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(70, 1));
                enc.encode_frame(f).unwrap();
            }
        }
        let d = decode(&out, 1600).unwrap();
        assert_eq!(d.frames.len(), 3);
        assert_eq!(d.frames[1].1, Duration::from_millis(70));
        assert_eq!(ingle_proto::control::image_size(&out), Some((20, 10)));
    }

    #[test]
    fn jpeg_and_webp_sizes_match_the_decoder() {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(37, 21, image::Rgb([1, 2, 3])));
        let mut jpg = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut jpg), image::ImageFormat::Jpeg).unwrap();
        assert_eq!(ingle_proto::control::image_size(&jpg), Some((37, 21)));
        let mut webp = Vec::new();
        img.to_rgba8();
        image::DynamicImage::ImageRgba8(img.to_rgba8()).write_to(&mut std::io::Cursor::new(&mut webp), image::ImageFormat::WebP).unwrap();
        assert_eq!(ingle_proto::control::image_size(&webp), Some((37, 21)));
    }

    #[test]
    fn attachments_must_be_images() {
        assert!(check_attachment("a.png", &png(2, 2)).is_ok());
        assert!(check_attachment("a.exe", b"MZ\x90\x00").is_err());
    }

    #[test]
    fn avatar_crop_takes_the_chosen_square() {
        // 40x20: left half red, right half blue. Zoomed on the right, the picture is blue.
        let mut img = image::RgbaImage::new(40, 20);
        for (x, _, p) in img.enumerate_pixels_mut() {
            *p = if x < 20 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 255, 255]) };
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let out = super::make_avatar_crop(&png, 0.75, 0.5, 0.5).unwrap();
        let got = image::load_from_memory(&out).unwrap().to_rgba8();
        assert_eq!(got.dimensions(), (256, 256));
        assert_eq!(got.get_pixel(128, 128).0, [0, 0, 255, 255]);
        // Out-of-range centres are pulled back inside the picture.
        assert!(super::make_avatar_crop(&png, 5.0, -3.0, 1.0).is_ok());
    }
}
