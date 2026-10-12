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
    /// When each was last shown (egui pass number), to let go of ones not seen in a while.
    used: HashMap<String, u64>,
    /// Bytes of decoded pictures held as textures.
    bytes: usize,
    tx: mpsc::Sender<(String, std::sync::Arc<Vec<u8>>, u32)>,
    rx: mpsc::Receiver<(String, Result<Decoded, String>)>,
}

/// Pictures kept decoded at most (beyond this, the least recently shown are let go and
/// decoded again if they come back into view).
const TEX_BUDGET: usize = 512 << 20;
const TEX_MAX: usize = 1500;

fn tex_bytes(t: &Tex) -> usize {
    match t {
        Tex::Ready { frames, size, .. } => frames.len() * (size.x as usize).max(1) * (size.y as usize).max(1) * 4,
        _ => 0,
    }
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
        Images { tex: HashMap::new(), used: HashMap::new(), bytes: 0, tx, rx }
    }
}

impl Images {
    /// The texture for `key`, decoding `bytes` (once) if needed. `max_side` limits the size.
    pub fn get(&mut self, ctx: &egui::Context, key: &str, bytes: Option<std::sync::Arc<Vec<u8>>>, max_side: u32) -> &Tex {
        self.collect(ctx);
        let pass = ctx.cumulative_pass_nr();
        self.used.insert(key.to_string(), pass);
        if self.bytes > TEX_BUDGET || self.tex.len() > TEX_MAX {
            self.evict(pass);
        }
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
        if let Some(t) = self.tex.remove(key) {
            self.bytes = self.bytes.saturating_sub(tex_bytes(&t));
        }
        self.used.remove(key);
    }

    /// Lets go of the pictures shown longest ago (never ones on screen right now) until well
    /// under the budget.
    fn evict(&mut self, pass: u64) {
        let mut old: Vec<(u64, String)> = self
            .tex
            .keys()
            .map(|k| (self.used.get(k).copied().unwrap_or(0), k.clone()))
            .filter(|(at, _)| pass.saturating_sub(*at) > 2)
            .collect();
        old.sort();
        for (_, k) in old {
            if self.bytes <= TEX_BUDGET * 3 / 4 && self.tex.len() <= TEX_MAX * 3 / 4 {
                break;
            }
            self.forget(&k);
        }
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
            self.bytes += tex_bytes(&t);
            if let Some(old) = self.tex.insert(key, t) {
                self.bytes = self.bytes.saturating_sub(tex_bytes(&old));
            }
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

/// Decoding limits for pictures from anywhere: a small file can claim to be enormous (a
/// "decompression bomb"), so the declared size is checked first and memory is capped.
fn limits() -> image::Limits {
    let mut l = image::Limits::default();
    l.max_image_width = Some(ingle_proto::control::MAX_IMAGE_SIDE);
    l.max_image_height = Some(ingle_proto::control::MAX_IMAGE_SIDE);
    l.max_alloc = Some(320 << 20);
    l
}

/// A still picture, decoded within [`limits`].
pub fn load_limited(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    if !ingle_proto::control::image_dimensions_ok(bytes) {
        return Err("That image is too large, or isn't one Ingle can read.".into());
    }
    let mut r = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| e.to_string())?;
    r.limits(limits());
    r.decode().map_err(|e| e.to_string())
}

/// An animated GIF's decoder, within [`limits`].
fn gif_decoder(bytes: &[u8]) -> Result<image::codecs::gif::GifDecoder<std::io::Cursor<&[u8]>>, String> {
    if !ingle_proto::control::image_dimensions_ok(bytes) {
        return Err("That GIF is too large.".into());
    }
    let mut dec = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    dec.set_limits(limits()).map_err(|e| e.to_string())?;
    Ok(dec)
}

/// Decode a PNG, JPEG, WebP or GIF (all its frames) into textures-to-be.
pub fn decode(bytes: &[u8], max_side: u32) -> Result<Decoded, String> {
    let fmt = image::guess_format(bytes).map_err(|e| e.to_string())?;
    if fmt == image::ImageFormat::Gif {
        let dec = gif_decoder(bytes)?;
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
            let img = load_limited(bytes)?.to_rgba8();
            frames = vec![(to_color_image(&shrink(img, max_side)), Duration::ZERO)];
        }
        return Ok(Decoded { frames });
    }
    let img = load_limited(bytes)?.to_rgba8();
    Ok(Decoded { frames: vec![(to_color_image(&shrink(img, max_side)), Duration::ZERO)] })
}

pub fn max_side_for_chat() -> u32 {
    MAX_SIDE
}

/// A server emoji from any image: up to 128 pixels across as PNG, or (for an animated GIF)
/// a GIF up to 96 pixels across. Returns the bytes and whether it's animated.
pub fn make_emoji(bytes: &[u8]) -> Result<(Vec<u8>, bool), String> {
    const STILL: u32 = 128;
    const MOVING: u32 = 96;
    let bad = || "That file isn't an image Ingle can read.".to_string();
    let fmt = image::guess_format(bytes).map_err(|_| bad())?;
    if fmt == image::ImageFormat::Gif {
        let dec = gif_decoder(bytes)?;
        let frames: Vec<image::Frame> = dec.into_frames().take(MAX_FRAMES).collect::<Result<_, _>>().map_err(|_| bad())?;
        if frames.len() > 1 {
            let (w, h) = frames[0].buffer().dimensions();
            if bytes.len() <= 256 * 1024 && w.max(h) <= 160 {
                return Ok((bytes.to_vec(), true));
            }
            let mut out = Vec::new();
            {
                let mut enc = image::codecs::gif::GifEncoder::new_with_speed(&mut out, 10);
                enc.set_repeat(image::codecs::gif::Repeat::Infinite).map_err(|e| e.to_string())?;
                for f in frames {
                    let delay = f.delay();
                    let small = shrink(f.into_buffer(), MOVING);
                    enc.encode_frame(image::Frame::from_parts(small, 0, 0, delay)).map_err(|e| e.to_string())?;
                }
            }
            if out.len() as u64 > ingle_proto::control::MAX_AVATAR_BYTES {
                return Err("That animated emoji is too big even after shrinking it; try a shorter GIF.".into());
            }
            return Ok((out, true));
        }
    }
    let img = load_limited(bytes).map_err(|_| bad())?.to_rgba8();
    let (w, h) = img.dimensions();
    let small = if w.max(h) > STILL { shrink(img, STILL) } else { img };
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(small).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok((out, false))
}

/// An emoji name from a file name: "Party Cat!.png" → "Party_Cat".
pub fn emoji_name_from(file: &str) -> String {
    let stem = file.rsplit(['/', '\\']).next().unwrap_or(file);
    let stem = stem.rsplit_once('.').map(|(a, _)| a).unwrap_or(stem);
    let mut n: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    while n.contains("__") {
        n = n.replace("__", "_");
    }
    let mut n = n.trim_matches('_').chars().take(32).collect::<String>();
    if n.len() < 2 {
        n = format!("emoji{n}");
    }
    n
}

/// A profile picture from any image: the middle square, 256×256, as PNG.
pub fn make_avatar(bytes: &[u8]) -> Result<Vec<u8>, String> {
    make_avatar_crop(bytes, 0.5, 0.5, 1.0)
}

/// A profile picture from part of an image: the square centred at (`cx`, `cy`) (0–1 across
/// and down the image) whose side is `frac` of the image's shorter side.
pub fn make_avatar_crop(bytes: &[u8], cx: f32, cy: f32, frac: f32) -> Result<Vec<u8>, String> {
    let img = load_limited(bytes).map_err(|_| "That file isn't an image Ingle can read (or it's too large).".to_string())?.to_rgba8();
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
    if !ingle_proto::control::image_dimensions_ok(bytes) {
        return Err(format!("{name} is too large (at most 8192 pixels across; animated GIFs at most 4096)."));
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

/// What a file dialog was opened for. Dialogs run on their own thread: the window (and,
/// through the shared state it reads each frame, voice) must never wait on one.
#[derive(Clone, Debug, PartialEq)]
pub enum PickFor {
    Avatar,
    ServerIcon(String),
    Emoji(String),
    Attach(String, ingle_proto::control::ChannelId),
}

type Picked = (PickFor, Vec<(String, Vec<u8>)>);

fn picks() -> &'static std::sync::Mutex<Vec<Picked>> {
    static P: std::sync::OnceLock<std::sync::Mutex<Vec<Picked>>> = std::sync::OnceLock::new();
    P.get_or_init(Default::default)
}

static DIALOG_CLOSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A file dialog closed since the last call. (Keys pressed in it can leave the window
/// thinking they're still held down.)
pub fn dialog_closed() -> bool {
    DIALOG_CLOSED.swap(false, std::sync::atomic::Ordering::SeqCst)
}

fn wanted() -> &'static std::sync::Mutex<Option<(PickFor, bool)>> {
    static W: std::sync::OnceLock<std::sync::Mutex<Option<(PickFor, bool)>>> = std::sync::OnceLock::new();
    W.get_or_init(Default::default)
}

/// Asks for the system's picture chooser. It opens right after this frame is drawn (see
/// `run_wanted_dialog`), when nothing is holding the shared state, so voice and messages
/// carry on while it's open. Results come from `take_picked`.
pub fn pick_async(what: PickFor, multiple: bool, ctx: &egui::Context) {
    *wanted().lock().unwrap() = Some((what, multiple));
    ctx.request_repaint();
}

/// Opens a chooser asked for with `pick_async`. Call between frames only.
pub fn run_wanted_dialog() {
    let Some((what, multiple)) = wanted().lock().unwrap().take() else { return };
    let files = if multiple { pick_images() } else { pick_image().into_iter().collect() };
    DIALOG_CLOSED.store(true, std::sync::atomic::Ordering::SeqCst);
    if !files.is_empty() {
        picks().lock().unwrap().push((what, files));
    }
}

/// Pictures chosen since the last call.
pub fn take_picked() -> Vec<Picked> {
    std::mem::take(&mut *picks().lock().unwrap())
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

    #[test]
    fn emoji_pictures_are_small() {
        let mut big = image::RgbaImage::new(600, 300);
        big.put_pixel(0, 0, image::Rgba([1, 2, 3, 255]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgba8(big).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let (out, animated) = super::make_emoji(&png).unwrap();
        assert!(!animated);
        let got = image::load_from_memory(&out).unwrap();
        assert_eq!((got.width(), got.height()), (128, 64));
        assert_eq!(super::emoji_name_from("C:\\pics\\Party Cat!.png"), "Party_Cat");
        assert_eq!(super::emoji_name_from("x.gif"), "emojix");
        assert!(ingle_proto::control::valid_emoji_name(&super::emoji_name_from("a b c d e f g h i j k l m n o p q r s t.png")));
    }

    #[test]
    fn decompression_bombs_are_not_decoded() {
        // A tiny GIF claiming a 60000x60000 canvas.
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&60000u16.to_le_bytes());
        gif.extend_from_slice(&60000u16.to_le_bytes());
        gif.extend_from_slice(&[0, 0, 0, 0x2c, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x4c, 1, 0, 0x3b]);
        assert!(super::decode(&gif, 400).is_err());
        assert!(super::load_limited(&gif).is_err());
        assert!(super::check_attachment("bomb.gif", &gif).is_err());
        // A PNG header claiming 100000x100000.
        let mut png = png(2, 2);
        png[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        png[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        assert!(super::decode(&png, 400).is_err());
    }
}
