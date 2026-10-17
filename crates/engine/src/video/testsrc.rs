//! Synthetic screens for trying screen shares over slow, far-away or lossy networks:
//! `test:motion` (a busy moving picture, like a game) and `test:text` (a page of text that
//! scrolls now and then). They aren't in the share picker.
//!
//! Each frame carries the moment it was made in a strip of squares along its top (see
//! [`write_stamp`] and [`read_stamp`]), so whoever watches can tell how late each picture
//! arrives, however much it was scaled or compressed on the way.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::capture::{CaptureHandle, FrameSlot};
use super::codec::{PixelFormat, RawFrame};

pub const MOTION_ID: &str = "test:motion";
pub const TEXT_ID: &str = "test:text";

/// The stamp's squares are the picture's width divided by this.
const BLOCKS_ACROSS: u32 = 60;
/// Squares in the stamp: a white and a black one (what white and black look like after the
/// trip), 32 bits of time, 8 bits of check.
const STAMP_BLOCKS: usize = 42;

/// Milliseconds since 1970 (wall clock: the same in every program on this computer).
pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn stamp_bits(ms: u32) -> [bool; STAMP_BLOCKS] {
    let b = ms.to_be_bytes();
    let check = b[0] ^ b[1] ^ b[2] ^ b[3] ^ 0xA5;
    let mut bits = [false; STAMP_BLOCKS];
    bits[0] = true;
    bits[1] = false;
    for i in 0..32 {
        bits[2 + i] = (ms >> (31 - i)) & 1 == 1;
    }
    for i in 0..8 {
        bits[34 + i] = (check >> (7 - i)) & 1 == 1;
    }
    bits
}

/// Writes `ms` (wall-clock milliseconds, kept to 32 bits) into the top left of an RGBA picture.
pub fn write_stamp(data: &mut [u8], w: u32, h: u32, ms: u32) {
    // (Squares a fraction of the width each, the same as `read_stamp` looks for them.)
    let block = w as f32 / BLOCKS_ACROSS as f32;
    let bits = stamp_bits(ms);
    for y in 0..(block.round() as u32).min(h) {
        for (i, bit) in bits.iter().enumerate() {
            let x0 = (i as f32 * block).round() as u32;
            let x1 = (((i + 1) as f32) * block).round() as u32;
            if x1 > w {
                break;
            }
            let v = if *bit { 255 } else { 0 };
            let row = ((y * w + x0) * 4) as usize;
            for px in data[row..row + ((x1 - x0) * 4) as usize].chunks_exact_mut(4) {
                px.copy_from_slice(&[v, v, v, 255]);
            }
        }
    }
}

/// Reads the stamp back from an RGBA picture (scaled to any size, and compressed): the
/// milliseconds [`write_stamp`] wrote, or None if it can't be read.
pub fn read_stamp(rgba: &[u8], w: u32, h: u32) -> Option<u32> {
    let block = w as f32 / BLOCKS_ACROSS as f32;
    if block < 3.0 || (h as f32) < block || rgba.len() < (w * h * 4) as usize {
        return None;
    }
    // Each square's middle, a few pixels averaged.
    let level = |i: usize| -> f32 {
        let cx = (i as f32 + 0.5) * block;
        let cy = 0.5 * block;
        let r = (block * 0.2).max(1.0);
        let mut sum = 0.0;
        let mut n = 0.0;
        for dy in [-r, 0.0, r] {
            for dx in [-r, 0.0, r] {
                let x = ((cx + dx) as u32).min(w - 1);
                let y = ((cy + dy) as u32).min(h - 1);
                let o = ((y * w + x) * 4) as usize;
                sum += (rgba[o] as f32 + rgba[o + 1] as f32 + rgba[o + 2] as f32) / 3.0;
                n += 1.0;
            }
        }
        sum / n
    };
    let (white, black) = (level(0), level(1));
    if white - black < 80.0 {
        return None;
    }
    let mid = (white + black) / 2.0;
    let mut bits = [false; STAMP_BLOCKS];
    for (i, b) in bits.iter_mut().enumerate() {
        *b = level(i) > mid;
    }
    let mut ms = 0u32;
    for i in 0..32 {
        ms = (ms << 1) | bits[2 + i] as u32;
    }
    (stamp_bits(ms) == bits).then_some(ms)
}

/// A small, fast, repeatable random number source.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u32) -> u32 {
        (self.next() % n.max(1) as u64) as u32
    }
}

/// Makes the frames of one synthetic screen.
pub struct Scene {
    kind: Kind,
    w: u32,
    h: u32,
    /// The motion scene's landscape (wider and taller than the screen, it pans across it),
    /// or the text scene's page.
    tex: Vec<u8>,
    tw: u32,
    th: u32,
    sine: Vec<f32>,
    dots: Vec<(f32, f32, f32, f32)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Motion,
    Text,
}

impl Scene {
    pub fn for_id(id: &str, w: u32, h: u32) -> Option<Scene> {
        let kind = match id {
            MOTION_ID => Kind::Motion,
            TEXT_ID => Kind::Text,
            _ => return None,
        };
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let (tw, th, tex) = match kind {
            Kind::Motion => {
                let (tw, th) = (w + w / 2, h + h / 2);
                (tw, th, landscape(tw, th, &mut rng))
            }
            Kind::Text => {
                let th = h * 4;
                (w, th, page(w, th, &mut rng))
            }
        };
        let sine = (0..1024).map(|i| (i as f32 / 1024.0 * std::f32::consts::TAU).sin()).collect();
        let dots = (0..160)
            .map(|_| {
                let a = rng.below(6283) as f32 / 1000.0;
                let s = 2.0 + rng.below(60) as f32 / 10.0;
                (rng.below(w) as f32, rng.below(h) as f32, a.cos() * s, a.sin() * s)
            })
            .collect();
        Some(Scene { kind, w, h, tex, tw, th, sine, dots })
    }

    /// Frame number `i` (at `fps`), stamped with `stamp_ms`.
    pub fn frame(&mut self, i: u64, fps: u32, stamp_ms: u32) -> RawFrame {
        let (w, h) = (self.w, self.h);
        let mut data = vec![0u8; (w * h * 4) as usize];
        let t = i as f32 / fps.max(1) as f32;
        match self.kind {
            Kind::Motion => {
                // The landscape pans (like a game's camera), with a patch of shimmering
                // "water" and sparks flying about (which no motion search can predict).
                let ox = ((t * 140.0) as u32) % self.tw;
                let oy = (((t * 0.7).sin() * 0.5 + 0.5) * (self.th - h) as f32) as u32;
                self.copy_window(&mut data, ox, oy);
                self.water(&mut data, t);
                self.sparks(&mut data, i);
            }
            Kind::Text => {
                // Reading a document: every 3 s it scrolls down a few lines (over 0.4 s).
                let step = 3.0;
                let n = (t / step).floor();
                let frac = ((t - n * step) / 0.4).min(1.0);
                let ease = frac * frac * (3.0 - 2.0 * frac);
                let y = ((n + ease) * 96.0) as u32 % (self.th - h);
                self.copy_window(&mut data, 0, y);
                // A toolbar across the top (where the stamp goes), and a blinking caret.
                for row in 0..(h / 20).max(24).min(h) {
                    let o = (row * w * 4) as usize;
                    for px in data[o..o + (w * 4) as usize].chunks_exact_mut(4) {
                        px.copy_from_slice(&[214, 218, 224, 255]);
                    }
                }
                if (t * 2.0) as u64 % 2 == 0 {
                    let (cx, cy) = (w / 3, h / 2);
                    for yy in cy..(cy + 18).min(h) {
                        let o = ((yy * w + cx) * 4) as usize;
                        data[o..o + 8].copy_from_slice(&[0, 0, 0, 255, 0, 0, 0, 255]);
                    }
                }
            }
        }
        write_stamp(&mut data, w, h, stamp_ms);
        RawFrame { width: w, height: h, format: PixelFormat::Rgba, data }
    }

    fn copy_window(&self, data: &mut [u8], ox: u32, oy: u32) {
        let (w, h, tw) = (self.w, self.h, self.tw);
        for y in 0..h {
            let ty = (oy + y) % self.th;
            let dst = (y * w * 4) as usize;
            let first = (tw - ox).min(w);
            let src = ((ty * tw + ox) * 4) as usize;
            data[dst..dst + (first * 4) as usize].copy_from_slice(&self.tex[src..src + (first * 4) as usize]);
            if first < w {
                let rest = w - first;
                let src = (ty * tw * 4) as usize;
                data[dst + (first * 4) as usize..dst + (w * 4) as usize].copy_from_slice(&self.tex[src..src + (rest * 4) as usize]);
            }
        }
    }

    fn sin(&self, x: f32) -> f32 {
        let i = (x * (1024.0 / std::f32::consts::TAU)) as i64;
        self.sine[i.rem_euclid(1024) as usize]
    }

    fn water(&self, data: &mut [u8], t: f32) {
        let (w, h) = (self.w, self.h);
        let (x0, y0, x1, y1) = (w / 8, h * 5 / 8, w * 5 / 8, h * 15 / 16);
        // (Worked out for every other pixel and row, which is plenty busy for an encoder and
        // keeps the test screen cheap to make.)
        for y in (y0..y1).step_by(2) {
            let fy = y as f32 * 0.045;
            let a = self.sin(fy + t * 3.1);
            for x in (x0..x1).step_by(2) {
                let fx = x as f32 * 0.037;
                let v = self.sin(fx + t * 2.3 + a) + self.sin((fx + fy) * 0.7 - t * 4.0);
                let c = (v * 50.0) as i32;
                let px = [(40 + c / 2).clamp(0, 255) as u8, (110 + c).clamp(0, 255) as u8, (170 + c).clamp(0, 255) as u8, 255];
                for yy in y..(y + 2).min(y1) {
                    let o = ((yy * w + x) * 4) as usize;
                    data[o..o + 4].copy_from_slice(&px);
                    if x + 1 < x1 {
                        data[o + 4..o + 8].copy_from_slice(&px);
                    }
                }
            }
        }
    }

    fn sparks(&mut self, data: &mut [u8], i: u64) {
        let (w, h) = (self.w as f32, self.h as f32);
        let size = (self.w / 240).max(2);
        for (n, d) in self.dots.iter_mut().enumerate() {
            d.0 = (d.0 + d.2).rem_euclid(w);
            d.1 = (d.1 + d.3).rem_euclid(h);
            let hue = ((n as u64 * 37 + i * 5) % 255) as u8;
            let (x0, y0) = (d.0 as u32, d.1 as u32);
            for y in y0..(y0 + size).min(self.h) {
                for x in x0..(x0 + size).min(self.w) {
                    let o = ((y * self.w + x) * 4) as usize;
                    data[o..o + 4].copy_from_slice(&[255, hue, 255 - hue, 255]);
                }
            }
        }
    }
}

/// Hills, sky and clutter: lots of detail, like a game's world.
fn landscape(w: u32, h: u32, rng: &mut Rng) -> Vec<u8> {
    let mut data = vec![0u8; (w * h * 4) as usize];
    // Value noise at a few scales, for a rough, detailed ground.
    let grid = |s: u32, rng: &mut Rng| -> (u32, Vec<f32>) {
        let gw = w / s + 2;
        let gh = h / s + 2;
        (gw, (0..gw * gh).map(|_| rng.below(1000) as f32 / 1000.0).collect())
    };
    let scales = [97u32, 23, 5];
    let grids: Vec<(u32, u32, Vec<f32>)> = scales.iter().map(|&s| {
        let (gw, g) = grid(s, rng);
        (s, gw, g)
    }).collect();
    for y in 0..h {
        let horizon = h as f32 * 0.35;
        for x in 0..w {
            let mut n = 0.0;
            let mut amp = 0.6;
            for (s, gw, g) in &grids {
                let (fx, fy) = (x as f32 / *s as f32, y as f32 / *s as f32);
                let (ix, iy) = (fx as u32, fy as u32);
                let (tx, ty) = (fx - ix as f32, fy - iy as f32);
                let at = |a: u32, b: u32| g[(b * gw + a) as usize];
                let top = at(ix, iy) * (1.0 - tx) + at(ix + 1, iy) * tx;
                let bot = at(ix, iy + 1) * (1.0 - tx) + at(ix + 1, iy + 1) * tx;
                n += (top * (1.0 - ty) + bot * ty) * amp;
                amp *= 0.5;
            }
            let o = ((y * w + x) * 4) as usize;
            let c = if (y as f32) < horizon + n * 120.0 - 60.0 {
                let k = y as f32 / horizon.max(1.0);
                [(90.0 + 80.0 * k) as u8, (150.0 + 60.0 * k) as u8, 235, 255]
            } else {
                let v = n / 1.05;
                [(60.0 + 120.0 * v) as u8, (90.0 + 110.0 * v) as u8, (40.0 + 60.0 * v) as u8, 255]
            };
            data[o..o + 4].copy_from_slice(&c);
        }
    }
    // Buildings and crates: sharp edges.
    for _ in 0..140 {
        let bw = 20 + rng.below(w / 14);
        let bh = 20 + rng.below(h / 6);
        let x0 = rng.below(w - bw);
        let y0 = h / 3 + rng.below(h - h / 3 - bh);
        let col = [rng.below(200) as u8 + 30, rng.below(200) as u8 + 30, rng.below(200) as u8 + 30, 255];
        for y in y0..y0 + bh {
            for x in x0..x0 + bw {
                let o = ((y * w + x) * 4) as usize;
                let edge = x == x0 || y == y0 || x == x0 + bw - 1 || y == y0 + bh - 1 || (x - x0) % 12 == 0;
                data[o..o + 4].copy_from_slice(if edge { &[15, 15, 20, 255] } else { &col });
            }
        }
    }
    data
}

/// A white page covered in lines of "words".
fn page(w: u32, h: u32, rng: &mut Rng) -> Vec<u8> {
    let mut data = vec![255u8; (w * h * 4) as usize];
    let margin = w / 12;
    let line = 24u32.max(h / 180);
    let glyph = line * 3 / 5;
    let mut y = line;
    while y + line < h {
        let mut x = margin;
        // Now and then a short line (a paragraph's end) or a heading.
        let end = if rng.below(9) == 0 { margin + rng.below(w / 2) } else { w - margin };
        let heading = rng.below(25) == 0;
        while x < end {
            let word = 4 + rng.below(if heading { 18 } else { 11 });
            let ww = word * glyph / 2;
            if x + ww > end {
                break;
            }
            for yy in y..y + if heading { glyph + 4 } else { glyph } {
                for xx in x..x + ww {
                    // Letters: thin vertical strokes with gaps, like text.
                    if (xx - x) % (glyph / 2).max(2) < (glyph / 5).max(1) || yy == y || yy == y + glyph - 1 {
                        let o = ((yy * w + xx) * 4) as usize;
                        data[o..o + 3].copy_from_slice(if heading { &[30, 60, 140] } else { &[34, 34, 38] });
                    }
                }
            }
            x += ww + glyph / 2;
        }
        y += if heading { line * 2 } else { line };
    }
    data
}

/// Runs a synthetic screen at `fps` into `slot`.
pub fn start(id: &str, w: u32, h: u32, fps: u32, slot: Arc<FrameSlot>) -> Option<CaptureHandle> {
    let mut scene = Scene::for_id(id, w, h)?;
    let stop = Arc::new(AtomicBool::new(false));
    let st = stop.clone();
    let t = std::thread::Builder::new()
        .name("ingle-testscreen".into())
        .spawn(move || {
            let start = Instant::now();
            let mut i = 0u64;
            while !st.load(Ordering::Relaxed) {
                slot.put(scene.frame(i, fps, now_ms() as u32));
                i += 1;
                let next = start + Duration::from_secs_f64(i as f64 / fps.max(1) as f64);
                match next.checked_duration_since(Instant::now()) {
                    Some(d) => std::thread::sleep(d),
                    // Running late (a slow computer): skip ahead instead of rushing.
                    None => i = (start.elapsed().as_secs_f64() * fps.max(1) as f64) as u64 + 1,
                }
            }
        })
        .expect("spawn test screen");
    Some(CaptureHandle::thread(stop, t))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::codec::{H264Decoder, H264Encoder, Scaler, to_i420};

    #[test]
    fn the_stamp_survives_scaling_and_compression() {
        let ms = 0x1234_5678;
        let mut scene = Scene::for_id(MOTION_ID, 1920, 1080).unwrap();
        let mut enc = H264Encoder::new(true, 600_000, 30).unwrap();
        let mut dec = H264Decoder::new().unwrap();
        let mut scaler = Scaler::default();
        let mut read = 0;
        for i in 0..12u64 {
            let mut f = scene.frame(i, 30, ms + i as u32);
            assert_eq!(read_stamp(&f.data, 1920, 1080), Some(ms + i as u32));
            // Down to 640x360 at a low bitrate, the way a struggling share would send it.
            let small = scaler.scale(&mut f, 640, 360).unwrap().unwrap();
            let Some(e) = enc.encode(&to_i420(&small).unwrap()).unwrap() else { continue };
            if let Some(d) = dec.decode(&e.data, true).unwrap() {
                assert_eq!(read_stamp(&d.rgba, d.width, d.height), Some(ms + i as u32), "frame {i}");
                read += 1;
            }
        }
        // (At this bitrate the encoder skips some frames: every one that comes out has its
        // stamp.)
        assert!(read >= 5, "read {read}");
        // Not a stamp: nothing.
        assert_eq!(read_stamp(&vec![128u8; 640 * 360 * 4], 640, 360), None);
    }

    #[test]
    fn the_text_page_scrolls_and_keeps_its_stamp() {
        let mut scene = Scene::for_id(TEXT_ID, 1280, 720).unwrap();
        let a = scene.frame(30, 30, 1);
        let b = scene.frame(60, 30, 2);
        let c = scene.frame(105, 30, 3);
        assert_eq!(read_stamp(&c.data, 1280, 720), Some(3));
        // Between scrolls (1 s and 2 s in): the same page; after the next scroll (3.5 s): moved.
        let below = |f: &RawFrame| f.data[(200 * 1280 * 4) as usize..].to_vec();
        assert_eq!(below(&a), below(&b));
        assert_ne!(below(&a), below(&c));
    }
}
