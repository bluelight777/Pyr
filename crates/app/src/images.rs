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
    /// A profile picture's own color (see `accent_of`), for what's behind it in a call.
    pub accent: Option<egui::Color32>,
}

pub enum Tex {
    Loading,
    Ready { frames: Vec<(egui::TextureHandle, Duration)>, size: egui::Vec2, total: Duration },
    /// Not a picture (or none we can read).
    Failed,
}

impl Tex {
    /// How many frames (0 while loading; 1 for a still picture).
    pub fn frame_count(&self) -> usize {
        match self {
            Tex::Ready { frames, .. } => frames.len(),
            _ => 0,
        }
    }

    /// Frame `i` (wrapping around), its size and how long it shows.
    pub fn frame_at(&self, i: usize) -> Option<(egui::TextureId, egui::Vec2, Duration)> {
        let Tex::Ready { frames, size, .. } = self else { return None };
        let (tex, d) = &frames[i % frames.len()];
        Some((tex.id(), *size, *d))
    }

    /// Which frame shows `t` seconds in (they loop).
    pub fn index_at(&self, t: f64) -> usize {
        let Tex::Ready { frames, total, .. } = self else { return 0 };
        if frames.len() < 2 || total.is_zero() {
            return 0;
        }
        let mut at = Duration::from_secs_f64(t.max(0.0) % total.as_secs_f64());
        for (i, (_, d)) in frames.iter().enumerate() {
            if at < *d {
                return i;
            }
            at -= *d;
        }
        0
    }

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
    tx: mpsc::Sender<(String, std::sync::Arc<Vec<u8>>, u32, bool)>,
    rx: mpsc::Receiver<(String, Result<Decoded, String>)>,
    /// Profile pictures' own colors (see `accent_of`), by key; kept when their textures go.
    accents: HashMap<String, egui::Color32>,
    /// Decoded pictures waiting to become textures (a few each frame, see `UPLOAD_PER_FRAME`).
    waiting: std::collections::VecDeque<(String, Result<Decoded, String>)>,
    /// Bytes made into textures this pass, and which pass that is.
    uploaded: (u64, usize),
}

/// Picture bytes made into textures in one frame at most (always at least one picture): many
/// finishing together are spread over a few frames, so none of them stalls the window.
const UPLOAD_PER_FRAME: usize = 16 << 20;

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
        let (tx, jobs) = mpsc::channel::<(String, std::sync::Arc<Vec<u8>>, u32, bool)>();
        let (done, rx) = mpsc::channel();
        let _ = std::thread::Builder::new().name("ingle-images".into()).spawn(move || {
            while let Ok((key, bytes, max_side, still)) = jobs.recv() {
                let mut r = decode(&bytes, max_side, still);
                // A profile picture's color, for its call tile (like Discord's).
                if key.starts_with("av:") {
                    if let Ok(d) = &mut r {
                        d.accent = d.frames.first().and_then(|f| accent_of(&f.0));
                    }
                }
                if done.send((key, r)).is_err() {
                    break;
                }
            }
        });
        Images { tex: HashMap::new(), used: HashMap::new(), bytes: 0, tx, rx, accents: HashMap::new(), waiting: Default::default(), uploaded: (0, 0) }
    }
}

impl Images {
    /// The texture for `key`, decoding `bytes` (once) if needed, every frame of a moving
    /// picture. `max_side` limits the size.
    pub fn get(&mut self, ctx: &egui::Context, key: &str, bytes: Option<std::sync::Arc<Vec<u8>>>, max_side: u32) -> &Tex {
        self.get_as(ctx, key, bytes, max_side, false)
    }

    /// Just the first frame of a moving picture (for pictures that only move now and then:
    /// the frames are decoded under another key when they do, see `get`).
    pub fn get_still(&mut self, ctx: &egui::Context, key: &str, bytes: Option<std::sync::Arc<Vec<u8>>>, max_side: u32) -> &Tex {
        self.get_as(ctx, key, bytes, max_side, true)
    }

    fn get_as(&mut self, ctx: &egui::Context, key: &str, bytes: Option<std::sync::Arc<Vec<u8>>>, max_side: u32, still: bool) -> &Tex {
        self.collect(ctx);
        let pass = ctx.cumulative_pass_nr();
        self.used.insert(key.to_string(), pass);
        if self.bytes > TEX_BUDGET || self.tex.len() > TEX_MAX {
            self.evict(pass);
        }
        if !self.tex.contains_key(key) {
            if let Some(b) = bytes {
                self.tex.insert(key.to_string(), Tex::Loading);
                let _ = self.tx.send((key.to_string(), b, max_side, still));
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }
        self.tex.get(key).unwrap_or(&Tex::Loading)
    }

    /// Picture `key`'s own color, once it's been decoded (profile pictures only: "av:" keys).
    pub fn accent(&self, key: &str) -> Option<egui::Color32> {
        self.accents.get(key).copied()
    }

    /// Whether `key` is ready to show (without asking for it).
    pub fn has(&self, key: &str) -> bool {
        matches!(self.tex.get(key), Some(Tex::Ready { .. }))
    }

    /// Whether `key`'s bytes turned out not to be a picture (without asking for it).
    pub fn failed(&self, key: &str) -> bool {
        matches!(self.tex.get(key), Some(Tex::Failed))
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

    /// Make finished decodes into textures, up to `UPLOAD_PER_FRAME` a frame.
    fn collect(&mut self, ctx: &egui::Context) {
        while let Ok(done) = self.rx.try_recv() {
            self.waiting.push_back(done);
        }
        let pass = ctx.cumulative_pass_nr();
        if self.uploaded.0 != pass {
            self.uploaded = (pass, 0);
        }
        while self.uploaded.1 < UPLOAD_PER_FRAME {
            let Some((key, r)) = self.waiting.pop_front() else { break };
            if let Some(a) = r.as_ref().ok().and_then(|d| d.accent) {
                self.accents.insert(key.clone(), a);
            }
            if let Ok(d) = &r {
                self.uploaded.1 += d.frames.iter().map(|f| f.0.size[0] * f.0.size[1] * 4).sum::<usize>().max(1);
            }
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
                    if frames.is_empty() { Tex::Failed } else { Tex::Ready { frames, size, total } }
                }
                Err(e) => {
                    tracing::debug!("couldn't show picture {key}: {e}");
                    Tex::Failed
                }
            };
            self.bytes += tex_bytes(&t);
            if let Some(old) = self.tex.insert(key, t) {
                self.bytes = self.bytes.saturating_sub(tex_bytes(&old));
            }
            ctx.request_repaint();
        }
        if !self.waiting.is_empty() {
            ctx.request_repaint();
        }
    }
}

/// A picture's own color, the way Discord picks a profile picture's: the hue most of it has
/// (counting the more colorful pixels more, and leaving out blacks, whites and greys), the
/// average of its pixels of about that hue; or, for a picture with hardly any color (black and
/// white, say), its average. None if it's all see-through.
pub fn accent_of(img: &egui::ColorImage) -> Option<egui::Color32> {
    const BINS: usize = 24;
    // (A few thousand pixels are plenty.)
    let step = (img.pixels.len() / 6000).max(1);
    // Per hue: weight, and the weighted sums of red, green and blue.
    let mut hues = [[0f32; 4]; BINS];
    let mut all = [0f32; 4];
    for c in img.pixels.iter().step_by(step) {
        let [r, g, b, a] = c.to_srgba_unmultiplied();
        if a < 128 {
            continue;
        }
        let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
        all = [all[0] + 1.0, all[1] + r, all[2] + g, all[3] + b];
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let (v, s) = (max, if max > 0.0 { (max - min) / max } else { 0.0 });
        if v < 0.18 || s < 0.25 {
            continue;
        }
        let d = max - min;
        let h = 60.0
            * if max == r {
                ((g - b) / d).rem_euclid(6.0)
            } else if max == g {
                (b - r) / d + 2.0
            } else {
                (r - g) / d + 4.0
            };
        // The more colorful, the more it counts (a big background counts for its size).
        let w = s * v.sqrt();
        let i = ((h / 360.0 * BINS as f32) as usize).min(BINS - 1);
        hues[i] = [hues[i][0] + w, hues[i][1] + r * w, hues[i][2] + g * w, hues[i][3] + b * w];
    }
    if all[0] < 1.0 {
        return None;
    }
    let to = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    let vivid: f32 = hues.iter().map(|h| h[0]).sum();
    if vivid < all[0] * 0.004 {
        return Some(egui::Color32::from_rgb(to(all[1] / all[0]), to(all[2] / all[0]), to(all[3] / all[0])));
    }
    // The strongest hue, with its neighbors (a color can straddle two).
    let around = |i: usize| [(i + BINS - 1) % BINS, i, (i + 1) % BINS];
    let score = |i: usize| around(i).iter().enumerate().map(|(k, j)| hues[*j][0] * if k == 1 { 1.0 } else { 0.5 }).sum::<f32>();
    let best = (0..BINS).max_by(|a, b| score(*a).total_cmp(&score(*b))).unwrap_or(0);
    let mut sum = [0f32; 4];
    for j in around(best) {
        for k in 0..4 {
            sum[k] += hues[j][k];
        }
    }
    Some(egui::Color32::from_rgb(to(sum[1] / sum[0]), to(sum[2] / sum[0]), to(sum[3] / sum[0])))
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

/// Most memory one moving picture's decoded frames take: past it, they're made smaller.
const ANIM_BUDGET: usize = 192 << 20;

/// A moving picture's frames, within [`limits`]: a GIF, an animated WebP or an animated PNG
/// (`None`: a still picture).
fn animation(bytes: &[u8]) -> Result<Option<image::Frames<'_>>, String> {
    if !ingle_proto::control::image_dimensions_ok(bytes) {
        return Err("That picture is too large.".into());
    }
    let err = |e: image::ImageError| e.to_string();
    let cur = || std::io::Cursor::new(bytes);
    match image::guess_format(bytes).map_err(err)? {
        image::ImageFormat::Gif => {
            let mut d = image::codecs::gif::GifDecoder::new(cur()).map_err(err)?;
            d.set_limits(limits()).map_err(err)?;
            Ok(Some(d.into_frames()))
        }
        image::ImageFormat::WebP => {
            let mut d = image::codecs::webp::WebPDecoder::new(cur()).map_err(err)?;
            if !d.has_animation() {
                return Ok(None);
            }
            d.set_limits(limits()).map_err(err)?;
            Ok(Some(d.into_frames()))
        }
        image::ImageFormat::Png => {
            let d = image::codecs::png::PngDecoder::with_limits(cur(), limits()).map_err(err)?;
            if !d.is_apng().map_err(err)? {
                return Ok(None);
            }
            Ok(Some(d.apng().map_err(err)?.into_frames()))
        }
        _ => Ok(None),
    }
}

/// How long a frame shows. Browsers treat very short delays as 100 ms; so do we.
fn delay_of(f: &image::Frame) -> Duration {
    let (n, d) = f.delay().numer_denom_ms();
    let ms = if d == 0 { 100 } else { n / d };
    Duration::from_millis(if ms < 20 { 100 } else { ms as u64 })
}

/// Whether a picture moves: a GIF of more than one frame, an animated WebP or an animated PNG
/// (read from how it starts, without decoding it).
pub fn is_animated(bytes: &[u8]) -> bool {
    match ingle_proto::control::sniff_image(bytes) {
        Some("image/gif") => gif_frames(bytes) > 1,
        // A VP8X header with its animation flag.
        Some("image/webp") => bytes.len() > 20 && &bytes[12..16] == b"VP8X" && bytes[20] & 0x02 != 0,
        // An acTL chunk before the picture's data.
        Some("image/png") => {
            let mut i = 8usize;
            while let Some(head) = bytes.get(i..i + 8) {
                let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
                match &head[4..8] {
                    b"acTL" => return true,
                    b"IDAT" | b"IEND" => return false,
                    _ => i = i.saturating_add(12).saturating_add(len),
                }
            }
            false
        }
        _ => false,
    }
}

/// How many frames a GIF has (counted from its blocks, at most 2: one, or more than one).
fn gif_frames(b: &[u8]) -> usize {
    let mut i = 13usize;
    let flags = *b.get(10).unwrap_or(&0);
    if flags & 0x80 != 0 {
        i += 3 * (1usize << ((flags & 7) + 1));
    }
    let skip = |mut i: usize| -> Option<usize> {
        loop {
            let n = *b.get(i)? as usize;
            i += 1;
            if n == 0 {
                return Some(i);
            }
            i += n;
        }
    };
    let mut frames = 0;
    while let Some(&tag) = b.get(i) {
        match tag {
            0x21 => match skip(i + 2) {
                Some(n) => i = n,
                None => break,
            },
            0x2c => {
                frames += 1;
                if frames > 1 {
                    break;
                }
                let Some(d) = b.get(i + 1..i + 10) else { break };
                let mut j = i + 10;
                if d[8] & 0x80 != 0 {
                    j += 3 * (1usize << ((d[8] & 7) + 1));
                }
                match skip(j + 1) {
                    Some(n) => i = n,
                    None => break,
                }
            }
            _ => break,
        }
    }
    frames
}

/// Decode a PNG, JPEG, WebP or GIF into textures-to-be: every frame of a moving one (GIF,
/// animated WebP or animated PNG), made smaller if they'd take too much memory, or with
/// `still`, just its first.
pub fn decode(bytes: &[u8], max_side: u32, still: bool) -> Result<Decoded, String> {
    if !still {
        if let Some(frames) = animation(bytes)? {
            let mut side = max_side.min(MAX_ANIM_SIDE);
            let mut kept: Vec<(image::RgbaImage, Duration)> = Vec::new();
            let mut used = 0usize;
            for f in frames.take(MAX_FRAMES) {
                let f = f.map_err(|e| e.to_string())?;
                let delay = delay_of(&f);
                let img = shrink(f.into_buffer(), side);
                used += img.as_raw().len();
                kept.push((img, delay));
                if used > ANIM_BUDGET {
                    if side <= 96 {
                        // As small as they go: the rest is left out.
                        break;
                    }
                    side = (side * 2 / 3).max(96);
                    for (img, _) in kept.iter_mut() {
                        let big = std::mem::replace(img, image::RgbaImage::new(1, 1));
                        *img = shrink(big, side);
                    }
                    used = kept.iter().map(|(i, _)| i.as_raw().len()).sum();
                }
            }
            if kept.len() > 1 {
                return Ok(Decoded { frames: kept.into_iter().map(|(i, d)| (to_color_image(&i), d)).collect(), accent: None });
            }
            // One frame: as sharp as any still picture (below).
        }
    }
    let img = load_limited(bytes)?.to_rgba8();
    Ok(Decoded { frames: vec![(to_color_image(&shrink(img, max_side)), Duration::ZERO)], accent: None })
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

/// A profile picture (or server icon) from part of an image: the square centred at (`cx`,
/// `cy`) (0–1 across and down the image) whose side is `frac` of the image's shorter side. A
/// moving one stays moving (see `moving_avatar`); a still one is 256×256 PNG.
pub fn make_avatar_crop(bytes: &[u8], cx: f32, cy: f32, frac: f32) -> Result<Vec<u8>, String> {
    if is_animated(bytes) {
        return moving_avatar(bytes, cx, cy, frac);
    }
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

/// The square at (`cx`, `cy`) whose side is `frac` of the shorter side, as (x, y, side).
fn square_in(w: u32, h: u32, cx: f32, cy: f32, frac: f32) -> (u32, u32, u32) {
    let side = ((w.min(h) as f32 * frac.clamp(0.05, 1.0)).round() as u32).clamp(1, w.min(h));
    let x = ((cx * w as f32 - side as f32 / 2.0).round().max(0.0) as u32).min(w - side);
    let y = ((cy * h as f32 - side as f32 / 2.0).round().max(0.0) as u32).min(h - side);
    (x, y, side)
}

/// A moving profile picture (or icon): the square of every frame, as a GIF that loops, at most
/// 128 pixels across and as big as a profile picture can be (`MAX_AVATAR_BYTES`): made smaller,
/// then with every other frame (or third) kept, until it fits.
pub fn moving_avatar(bytes: &[u8], cx: f32, cy: f32, frac: f32) -> Result<Vec<u8>, String> {
    let bad = || "That moving picture couldn't be read (or it's too large).".to_string();
    let frames = animation(bytes).map_err(|_| bad())?.ok_or_else(bad)?;
    // The squares, kept no bigger than needed (big frames are shrunk first: crops are by
    // fraction, so the square stays the same part of the picture).
    let mut squares: Vec<(image::RgbaImage, Duration)> = Vec::new();
    for f in frames.take(MAX_FRAMES) {
        let f = f.map_err(|_| bad())?;
        let delay = delay_of(&f);
        let img = shrink(f.into_buffer(), 512);
        let (w, h) = img.dimensions();
        let (x, y, side) = square_in(w, h, cx, cy, frac);
        let sq = image::imageops::crop_imm(&img, x, y, side, side).to_image();
        squares.push((shrink(sq, 160), delay));
    }
    if squares.len() < 2 {
        return Err(bad());
    }
    for side in [128u32, 112, 96, 80] {
        for every in [1usize, 2, 3] {
            let mut out = Vec::new();
            {
                let mut enc = image::codecs::gif::GifEncoder::new_with_speed(&mut out, 10);
                enc.set_repeat(image::codecs::gif::Repeat::Infinite).map_err(|e| e.to_string())?;
                for chunk in squares.chunks(every) {
                    // (Frames left out: their time goes to the one kept.)
                    let delay: Duration = chunk.iter().map(|c| c.1).sum();
                    // (Never made bigger than it is.)
                    let to = side.min(chunk[0].0.width());
                    let img = image::imageops::resize(&chunk[0].0, to, to, image::imageops::FilterType::Triangle);
                    let frame = image::Frame::from_parts(img, 0, 0, image::Delay::from_saturating_duration(delay));
                    enc.encode_frame(frame).map_err(|e| e.to_string())?;
                }
            }
            if out.len() as u64 <= ingle_proto::control::MAX_AVATAR_BYTES {
                return Ok(out);
            }
        }
    }
    Err("That moving picture is too big to be a profile picture or icon, even made smaller. Try a shorter one.".into())
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

/// The most an image attached on server `s` can be: 25 MB, or 10 MB on servers older than 0.8.
pub fn upload_limit(s: &ingle_engine::state::ServerView) -> u64 {
    use ingle_proto::control::{BIG_FILES_FEATURE, MAX_ATTACHMENT_BYTES, OLD_MAX_ATTACHMENT_BYTES};
    if s.has(BIG_FILES_FEATURE) { MAX_ATTACHMENT_BYTES } else { OLD_MAX_ATTACHMENT_BYTES }
}

/// Image files people can attach (checked by content, not by name), up to `limit` bytes.
pub fn check_attachment(name: &str, bytes: &[u8], limit: u64) -> Result<(), String> {
    if bytes.len() as u64 > limit {
        return Err(format!("{name} is over {} MB.", limit >> 20));
    }
    if ingle_proto::control::sniff_image(bytes).is_none() {
        return Err(format!("{name} isn't a PNG, JPEG, GIF or WebP image."));
    }
    if !ingle_proto::control::image_dimensions_ok(bytes) {
        return Err(format!("{name} is too large (at most 8192 pixels across; animated GIFs at most 4096)."));
    }
    Ok(())
}

/// A picture too big to attach (over `limit`) that can be made smaller to fit instead of
/// being refused.
pub fn can_shrink(bytes: &[u8], limit: u64) -> bool {
    bytes.len() as u64 > limit && ingle_proto::control::sniff_image(bytes).is_some() && ingle_proto::control::image_dimensions_ok(bytes)
}

/// A picture made smaller to fit in `limit` bytes: a moving one as a GIF (smaller, then with
/// every other frame kept if it must), a still one as JPEG (PNG if it has see-through parts),
/// smaller until it fits. Returns the bytes and the file extension that goes with them.
pub fn shrink_to_fit(bytes: &[u8], limit: u64) -> Result<(Vec<u8>, &'static str), String> {
    // (A little room under the limit.)
    let target = limit * 96 / 100;
    if is_animated(bytes) {
        let frames = animation(bytes)?.ok_or("That moving picture couldn't be read.")?;
        // Kept no bigger than 1280 across, and within a budget (smaller if it must).
        let mut side = 1280u32;
        let mut kept: Vec<(image::RgbaImage, Duration)> = Vec::new();
        let mut used = 0usize;
        for f in frames.take(MAX_FRAMES) {
            let f = f.map_err(|e| e.to_string())?;
            let delay = delay_of(&f);
            let img = shrink(f.into_buffer(), side);
            used += img.as_raw().len();
            kept.push((img, delay));
            if used > 2 * ANIM_BUDGET && side > 160 {
                side = (side * 2 / 3).max(160);
                for (img, _) in kept.iter_mut() {
                    let big = std::mem::replace(img, image::RgbaImage::new(1, 1));
                    *img = shrink(big, side);
                }
                used = kept.iter().map(|(i, _)| i.as_raw().len()).sum();
            }
        }
        let (w, h) = kept.first().map(|f| f.0.dimensions()).ok_or("That moving picture has no frames.")?;
        // A GIF's size goes with its pixels: a first guess at how much smaller, then closer.
        let mut scale = ((target as f64 / bytes.len() as f64).sqrt() * 0.95).min(1.0);
        let mut every = 1usize;
        for _ in 0..10 {
            let (sw, sh) = (((w as f64 * scale) as u32).max(16), ((h as f64 * scale) as u32).max(16));
            let mut out = Vec::new();
            {
                let mut enc = image::codecs::gif::GifEncoder::new_with_speed(&mut out, 20);
                enc.set_repeat(image::codecs::gif::Repeat::Infinite).map_err(|e| e.to_string())?;
                for chunk in kept.chunks(every) {
                    let delay: Duration = chunk.iter().map(|c| c.1).sum();
                    let img = image::imageops::resize(&chunk[0].0, sw.min(chunk[0].0.width()), sh.min(chunk[0].0.height()), image::imageops::FilterType::Triangle);
                    enc.encode_frame(image::Frame::from_parts(img, 0, 0, image::Delay::from_saturating_duration(delay))).map_err(|e| e.to_string())?;
                }
            }
            if out.len() as u64 <= target {
                return Ok((out, "gif"));
            }
            // Still too big: smaller, or (once small) fewer frames.
            if sw.max(sh) <= 240 && every < 3 {
                every += 1;
            } else {
                scale *= ((target as f64 / out.len() as f64).sqrt() * 0.95).min(0.9);
            }
        }
        return Err("That moving picture is too big to send, even made smaller.".into());
    }
    let mut img = load_limited(bytes)?;
    let see_through = img.color().has_alpha() && img.to_rgba8().pixels().any(|p| p[3] < 255);
    for _ in 0..12 {
        let mut out = Vec::new();
        if see_through {
            img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        } else {
            let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 88);
            img.to_rgb8().write_with_encoder(enc).map_err(|e| e.to_string())?;
        }
        if out.len() as u64 <= target {
            return Ok((out, if see_through { "png" } else { "jpg" }));
        }
        let (w, h) = (img.width(), img.height());
        img = img.resize((w * 4 / 5).max(16), (h * 4 / 5).max(16), image::imageops::FilterType::Triangle);
    }
    Err("That picture is too big to send, even made smaller.".into())
}

/// A picture being made smaller to fit, then: (the draft it's for, its name, the smaller one
/// and its extension or why not, how big it was).
pub type Shrunk = ((String, ingle_proto::control::ChannelId), String, Result<(Vec<u8>, &'static str), String>, u64);

fn shrunk() -> &'static std::sync::Mutex<Vec<Shrunk>> {
    static S: std::sync::OnceLock<std::sync::Mutex<Vec<Shrunk>>> = std::sync::OnceLock::new();
    S.get_or_init(Default::default)
}

/// Makes a picture smaller to fit `limit`, on a thread of its own (see `take_shrunk`).
pub fn shrink_to_fit_async(draft: (String, ingle_proto::control::ChannelId), name: String, bytes: Vec<u8>, limit: u64, ctx: &egui::Context) {
    let ctx = ctx.clone();
    let _ = std::thread::Builder::new().name("ingle-shrink".into()).spawn(move || {
        let was = bytes.len() as u64;
        let r = shrink_to_fit(&bytes, limit);
        shrunk().lock().unwrap_or_else(|e| e.into_inner()).push((draft, name, r, was));
        ctx.request_repaint();
    });
}

/// Pictures made smaller since last time.
pub fn take_shrunk() -> Vec<Shrunk> {
    std::mem::take(&mut *shrunk().lock().unwrap_or_else(|e| e.into_inner()))
}

/// Files copied (in File Explorer, say) and pasted: their names and bytes (big ones left out).
pub fn clipboard_files() -> Vec<(String, Vec<u8>)> {
    let Ok(mut cb) = arboard::Clipboard::new() else { return Vec::new() };
    let Ok(paths) = cb.get().file_list() else { return Vec::new() };
    let small: Vec<std::path::PathBuf> = paths.into_iter().filter(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() <= 200 << 20)).take(10).collect();
    read_files(&small)
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
    /// A sound file for a server's soundboard (picking nothing is reported too, as no files).
    Sound(String),
    /// An add-on (.wasm) to try (Plugins → Make your own).
    AddonFile,
    /// Pictures to add as a server's own GIFs (its key).
    ServerGif(String),
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

/// Opens a chooser asked for with `pick_async` or `pick_folder_async`. Call between frames only.
pub fn run_wanted_dialog() {
    if let Some((what, title)) = wanted_folder().lock().unwrap().take() {
        let dir = rfd::FileDialog::new().set_title(title).pick_folder();
        DIALOG_CLOSED.store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(d) = dir {
            folders().lock().unwrap().push((what, d));
        }
    }
    let Some((what, multiple)) = wanted().lock().unwrap().take() else { return };
    if let PickFor::Sound(_) = what {
        let file = pick_sound();
        DIALOG_CLOSED.store(true, std::sync::atomic::Ordering::SeqCst);
        picks().lock().unwrap().push((what, file.into_iter().collect()));
        return;
    }
    if let PickFor::AddonFile = what {
        let file = rfd::FileDialog::new()
            .set_title("Choose an add-on to try")
            .add_filter("Ingle add-ons", &["wasm"])
            .pick_file()
            .filter(|p| std::fs::metadata(p).map(|m| m.len() <= 8 << 20).unwrap_or(false));
        DIALOG_CLOSED.store(true, std::sync::atomic::Ordering::SeqCst);
        let files = file.map(|p| read_files(&[p])).unwrap_or_default();
        if !files.is_empty() {
            picks().lock().unwrap().push((what, files));
        }
        return;
    }
    let files = if multiple { pick_images() } else { pick_image().into_iter().collect() };
    DIALOG_CLOSED.store(true, std::sync::atomic::Ordering::SeqCst);
    if !files.is_empty() {
        picks().lock().unwrap().push((what, files));
    }
}

/// The most of a sound file read (far more than the 3 minutes kept of any sound).
const MAX_SOUND_FILE: u64 = 256 << 20;

/// Pick a sound file (its start, for a very big one).
fn pick_sound() -> Option<(String, Vec<u8>)> {
    use std::io::Read;
    let path = rfd::FileDialog::new()
        .set_title("Choose a sound")
        .add_filter("Sounds", &["mp3", "wav", "ogg", "oga", "opus", "flac", "m4a", "aac", "mp4", "ings"])
        .add_filter("All files", &["*"])
        .pick_file()?;
    let name = path.file_name()?.to_string_lossy().to_string();
    let mut bytes = Vec::new();
    std::fs::File::open(&path).ok()?.take(MAX_SOUND_FILE).read_to_end(&mut bytes).ok()?;
    Some((name, bytes))
}

/// What a chosen folder is for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FolderFor {
    /// Where server backups go (Settings).
    Backups,
    /// A backup to bring a server back from (creating a server).
    Restore,
    /// Where to start a new add-on (Plugins → Make your own).
    NewAddon,
    /// A plugin's folder, to send in for the Ingle Library.
    PluginSource,
    /// A skin's folder, to try on (Settings → Appearance).
    TrySkin,
}

fn wanted_folder() -> &'static std::sync::Mutex<Option<(FolderFor, &'static str)>> {
    static W: std::sync::OnceLock<std::sync::Mutex<Option<(FolderFor, &'static str)>>> = std::sync::OnceLock::new();
    W.get_or_init(Default::default)
}

fn folders() -> &'static std::sync::Mutex<Vec<(FolderFor, std::path::PathBuf)>> {
    static F: std::sync::OnceLock<std::sync::Mutex<Vec<(FolderFor, std::path::PathBuf)>>> = std::sync::OnceLock::new();
    F.get_or_init(Default::default)
}

/// Asks for the system's folder chooser, opened between frames like `pick_async` (a dialog
/// opened while drawing would hold the shared state, and with it voice and messages, until it
/// closes). Results come from `take_folders`.
pub fn pick_folder_async(what: FolderFor, title: &'static str, ctx: &egui::Context) {
    *wanted_folder().lock().unwrap() = Some((what, title));
    ctx.request_repaint();
}

/// Folders chosen since the last call.
pub fn take_folders() -> Vec<(FolderFor, std::path::PathBuf)> {
    std::mem::take(&mut *folders().lock().unwrap())
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

    #[test]
    fn a_pictures_own_color_is_its_strongest_hue() {
        let image = |px: &dyn Fn(usize) -> [u8; 4]| {
            let bytes: Vec<u8> = (0..64 * 64).flat_map(px).collect();
            egui::ColorImage::from_rgba_unmultiplied([64, 64], &bytes)
        };
        // Mostly near black, with a blue dragon on it and a few green sparks (Bluelight's).
        let dragon = image(&|i| match i {
            i if i % 5 == 0 => [30, 140, 230, 255],
            i if i % 37 == 0 => [120, 220, 60, 255],
            _ => [8, 10, 14, 255],
        });
        let (h, s, _) = crate::theme::hsl(accent_of(&dragon).unwrap());
        assert!((190.0..230.0).contains(&(h * 360.0)) && s > 0.5, "blue: {} {s}", h * 360.0);
        // Black and white: grey.
        let grey = image(&|i| if i % 2 == 0 { [20, 20, 20, 255] } else { [230, 230, 230, 255] });
        assert!(crate::theme::hsl(accent_of(&grey).unwrap()).1 < 0.1);
        // All see-through: none.
        assert_eq!(accent_of(&image(&|_| [255, 0, 0, 0])), None);
        // Its tile: deep and rich in that blue, darker at the top.
        let (top, bottom) = crate::theme::tile_tint(accent_of(&dragon).unwrap());
        let (ht, st, lt) = crate::theme::hsl(top);
        let (hb, sb, lb) = crate::theme::hsl(bottom);
        assert!(((ht - h).abs() * 360.0) < 8.0 && ((hb - h).abs() * 360.0) < 8.0, "{ht} {hb} {h}");
        assert!(st > 0.4 && sb > 0.4 && lt < lb && lb < 0.25, "{st} {sb} {lt} {lb}");
        // A grey picture keeps a grey tile.
        let (_, gs, _) = crate::theme::hsl(crate::theme::tile_tint(accent_of(&grey).unwrap()).1);
        assert!(gs < 0.1);
    }

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
        let d = decode(&png(3200, 100), 1600, false).unwrap();
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
        let d = decode(&out, 1600, false).unwrap();
        assert_eq!(d.frames.len(), 3);
        assert_eq!(d.frames[1].1, Duration::from_millis(70));
        assert_eq!(ingle_proto::control::image_size(&out), Some((20, 10)));
    }

    /// An animated WebP of `n` frames 16×8 (lossless, 30 ms each), put together by hand:
    /// RIFF, VP8X (animated), ANIM, then an ANMF around each frame's VP8L.
    fn animated_webp(n: usize) -> Vec<u8> {
        let (w, h) = (16u32, 8u32);
        let chunk = |out: &mut Vec<u8>, fourcc: &[u8], data: &[u8]| {
            out.extend_from_slice(fourcc);
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(data);
            if data.len() % 2 == 1 {
                out.push(0);
            }
        };
        let mut body = Vec::new();
        let mut vp8x = vec![0x12, 0, 0, 0];
        vp8x.extend_from_slice(&(w - 1).to_le_bytes()[..3]);
        vp8x.extend_from_slice(&(h - 1).to_le_bytes()[..3]);
        chunk(&mut body, b"VP8X", &vp8x);
        chunk(&mut body, b"ANIM", &[0, 0, 0, 0, 0, 0]);
        for i in 0..n {
            let img = image::RgbaImage::from_pixel(w, h, image::Rgba([(i * 60) as u8, 40, 0, 255]));
            let mut one = Vec::new();
            image::ImageEncoder::write_image(image::codecs::webp::WebPEncoder::new_lossless(&mut one), img.as_raw(), w, h, image::ExtendedColorType::Rgba8).unwrap();
            let vp8l = &one[12..];
            assert_eq!(&vp8l[..4], b"VP8L");
            let mut anmf = vec![0, 0, 0, 0, 0, 0];
            anmf.extend_from_slice(&(w - 1).to_le_bytes()[..3]);
            anmf.extend_from_slice(&(h - 1).to_le_bytes()[..3]);
            anmf.extend_from_slice(&30u32.to_le_bytes()[..3]);
            anmf.push(0);
            anmf.extend_from_slice(vp8l);
            chunk(&mut body, b"ANMF", &anmf);
        }
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&((body.len() + 4) as u32).to_le_bytes());
        out.extend_from_slice(b"WEBP");
        out.extend_from_slice(&body);
        out
    }

    /// An animated PNG of `n` frames 12×6, 40 ms each.
    fn apng(n: u32) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 12, 6);
            enc.set_color(png::ColorType::Rgba);
            enc.set_animated(n, 0).unwrap();
            enc.set_frame_delay(40, 1000).unwrap();
            let mut w = enc.write_header().unwrap();
            for i in 0..n {
                let img = image::RgbaImage::from_pixel(12, 6, image::Rgba([0, (i * 70) as u8, 90, 255]));
                w.write_image_data(img.as_raw()).unwrap();
            }
            w.finish().unwrap();
        }
        out
    }

    #[test]
    fn webp_and_png_pictures_move_too() {
        let webp = animated_webp(3);
        assert!(is_animated(&webp));
        let d = decode(&webp, 1600, false).unwrap();
        assert_eq!(d.frames.len(), 3, "every frame of an animated WebP");
        assert_eq!(d.frames[1].1, Duration::from_millis(30));
        let px = d.frames[2].0.pixels[0];
        assert!((px.r() as i32 - 120).abs() <= 2 && (px.g() as i32 - 40).abs() <= 2 && px.b() <= 2, "frame 3's color: {px:?}");
        // Just the first, when that's all that's wanted.
        assert_eq!(decode(&webp, 1600, true).unwrap().frames.len(), 1);

        let moving = apng(4);
        assert!(is_animated(&moving));
        let d = decode(&moving, 1600, false).unwrap();
        assert_eq!(d.frames.len(), 4, "every frame of an animated PNG");
        assert_eq!(d.frames[3].1, Duration::from_millis(40));

        // Still pictures don't count as moving.
        assert!(!is_animated(&png(4, 4)));
        let mut one = Vec::new();
        image::codecs::gif::GifEncoder::new(&mut one).encode_frame(image::Frame::new(image::RgbaImage::new(4, 4))).unwrap();
        assert!(!is_animated(&one), "a GIF of one frame");
    }

    #[test]
    fn a_huge_animation_is_made_smaller_to_fit_in_memory() {
        // 130 frames kept 640×640 would take 213 MB decoded: they're shrunk until they fit.
        use image::codecs::gif::GifEncoder;
        let mut out = Vec::new();
        {
            let mut enc = GifEncoder::new_with_speed(&mut out, 30);
            for i in 0..130u32 {
                let img = image::RgbaImage::from_pixel(700, 700, image::Rgba([(i * 2) as u8, 0, 0, 255]));
                enc.encode_frame(image::Frame::from_parts(img, 0, 0, image::Delay::from_numer_denom_ms(50, 1))).unwrap();
            }
        }
        let d = decode(&out, 1600, false).unwrap();
        let bytes: usize = d.frames.iter().map(|f| f.0.pixels.len() * 4).sum();
        assert!(bytes <= ANIM_BUDGET, "{bytes}");
        assert_eq!(d.frames.len(), 130, "every frame kept, smaller");
        assert!(d.frames[0].0.size[0] < MAX_ANIM_SIDE as usize);
    }

    #[test]
    fn moving_profile_pictures_stay_moving() {
        let webp = animated_webp(4);
        let gif = make_avatar_crop(&webp, 0.5, 0.5, 1.0).unwrap();
        assert!(is_animated(&gif), "still moving");
        assert!(gif.len() as u64 <= ingle_proto::control::MAX_AVATAR_BYTES);
        let d = decode(&gif, 256, false).unwrap();
        assert_eq!(d.frames.len(), 4);
        assert_eq!(d.frames[0].0.size, [8, 8], "the square of a 16×8 picture");
        // A still one is still the usual PNG.
        let still = make_avatar_crop(&png(40, 20), 0.5, 0.5, 1.0).unwrap();
        assert!(still.starts_with(b"\x89PNG"));
    }

    #[test]
    fn too_big_pictures_are_made_smaller_to_fit() {
        // A noisy still picture (hard to compress): to JPEG, and smaller, under the limit.
        let mut rng = 7u32;
        let img = image::RgbImage::from_fn(900, 700, |_, _| {
            rng ^= rng << 13;
            rng ^= rng >> 17;
            rng ^= rng << 5;
            image::Rgb([rng as u8, (rng >> 8) as u8, (rng >> 16) as u8])
        });
        let mut big = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut big), image::ImageFormat::Png).unwrap();
        let limit = 300 << 10;
        assert!(can_shrink(&big, limit));
        let (small, ext) = shrink_to_fit(&big, limit).unwrap();
        assert!(small.len() as u64 <= limit, "{}", small.len());
        assert_eq!(ext, "jpg");
        assert!(check_attachment("small.jpg", &small, limit).is_ok());
        // A moving one stays moving (a GIF).
        let moving = animated_webp(6);
        let (gif, ext) = shrink_to_fit(&moving, moving.len() as u64 / 2 + 2000).unwrap();
        assert_eq!(ext, "gif");
        assert!(is_animated(&gif));
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
        assert!(check_attachment("a.png", &png(2, 2), 10 << 20).is_ok());
        assert!(check_attachment("a.exe", b"MZ\x90\x00", 10 << 20).is_err());
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
        assert!(super::decode(&gif, 400, false).is_err());
        assert!(super::load_limited(&gif).is_err());
        assert!(super::check_attachment("bomb.gif", &gif, 10 << 20).is_err());
        // A PNG header claiming 100000x100000.
        let mut png = png(2, 2);
        png[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        png[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        assert!(super::decode(&png, 400, false).is_err());
    }
}
