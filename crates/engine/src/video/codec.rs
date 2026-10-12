//! H.264 encode/decode (OpenH264) with fast colour conversion and scaling.

use anyhow::{Result, anyhow, bail};
use fast_image_resize::images::Image;
use fast_image_resize::{PixelType, ResizeAlg, ResizeOptions, Resizer};
use openh264::OpenH264API;
use openh264::decoder::Decoder;
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod, RateControlMode, UsageType,
};
use openh264::formats::{YUVSlices, YUVSource};
use yuv::{YuvChromaSubsampling, YuvConversionMode, YuvPlanarImage, YuvPlanarImageMut, YuvRange, YuvStandardMatrix};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba,
    Bgra,
    Rgb,
}

/// A captured frame, tightly packed.
#[derive(Clone)]
pub struct RawFrame {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub data: Vec<u8>,
}

impl RawFrame {
    pub fn bytes_per_pixel(&self) -> usize {
        match self.format {
            PixelFormat::Rgb => 3,
            _ => 4,
        }
    }
}

/// Size to encode at: fit within `max_w` x `max_h`, keep aspect, even dimensions.
pub fn fit_size(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let scale = (max_w as f64 / w as f64).min(max_h as f64 / h as f64).min(1.0);
    let ew = ((w as f64 * scale).round() as u32).max(16) & !1;
    let eh = ((h as f64 * scale).round() as u32).max(16) & !1;
    (ew.min(3840), eh.min(2160))
}

pub struct Scaler {
    resizer: Resizer,
}

impl Default for Scaler {
    fn default() -> Self {
        Scaler { resizer: Resizer::new() }
    }
}

impl Scaler {
    /// Resize to exactly `w` x `h` (same pixel format). Returns `None` when no resize is needed.
    pub fn scale(&mut self, frame: &mut RawFrame, w: u32, h: u32) -> Result<Option<RawFrame>> {
        if frame.width == w && frame.height == h {
            return Ok(None);
        }
        let pt = if frame.format == PixelFormat::Rgb { PixelType::U8x3 } else { PixelType::U8x4 };
        let (fw, fh, fmt) = (frame.width, frame.height, frame.format);
        let src = Image::from_slice_u8(fw, fh, &mut frame.data, pt).map_err(|e| anyhow!("bad frame buffer: {e:?}"))?;
        let mut dst = Image::new(w, h, pt);
        let opts = ResizeOptions::new()
            .resize_alg(ResizeAlg::Convolution(fast_image_resize::FilterType::Bilinear))
            .use_alpha(false);
        self.resizer.resize(&src, &mut dst, &opts).map_err(|e| anyhow!("resize failed: {e:?}"))?;
        Ok(Some(RawFrame { width: w, height: h, format: fmt, data: dst.into_vec() }))
    }
}

/// A small RGBA copy of a frame for self-view tiles.
pub fn preview_rgba(scaler: &mut Scaler, frame: &mut RawFrame, max_w: u32, max_h: u32) -> Option<(u32, u32, Vec<u8>)> {
    let (w, h) = fit_size(frame.width, frame.height, max_w, max_h);
    let scaled = scaler.scale(frame, w, h).ok()?;
    let f = scaled.as_ref().unwrap_or(&*frame);
    let px = (f.width * f.height) as usize;
    let mut out = vec![0u8; px * 4];
    match f.format {
        PixelFormat::Rgba => out.copy_from_slice(&f.data[..px * 4]),
        PixelFormat::Bgra => {
            for (o, i) in out.chunks_exact_mut(4).zip(f.data.chunks_exact(4)) {
                o[0] = i[2];
                o[1] = i[1];
                o[2] = i[0];
                o[3] = 255;
            }
        }
        PixelFormat::Rgb => {
            for (o, i) in out.chunks_exact_mut(4).zip(f.data.chunks_exact(3)) {
                o[0] = i[0];
                o[1] = i[1];
                o[2] = i[2];
                o[3] = 255;
            }
        }
    }
    Some((f.width, f.height, out))
}

/// I420 planes with their strides.
pub struct I420 {
    pub width: u32,
    pub height: u32,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}

pub fn to_i420(frame: &RawFrame) -> Result<I420> {
    let (w, h) = (frame.width, frame.height);
    let mut img = YuvPlanarImageMut::<u8>::alloc(w, h, YuvChromaSubsampling::Yuv420);
    let stride = w * frame.bytes_per_pixel() as u32;
    let r = YuvRange::Limited;
    let m = YuvStandardMatrix::Bt601;
    let mode = YuvConversionMode::Balanced;
    match frame.format {
        PixelFormat::Rgba => yuv::rgba_to_yuv420(&mut img, &frame.data, stride, r, m, mode),
        PixelFormat::Bgra => yuv::bgra_to_yuv420(&mut img, &frame.data, stride, r, m, mode),
        PixelFormat::Rgb => yuv::rgb_to_yuv420(&mut img, &frame.data, stride, r, m, mode),
    }
    .map_err(|e| anyhow!("colour conversion failed: {e:?}"))?;
    Ok(I420 {
        width: w,
        height: h,
        y: img.y_plane.borrow().to_vec(),
        u: img.u_plane.borrow().to_vec(),
        v: img.v_plane.borrow().to_vec(),
    })
}

pub struct EncodedFrame {
    pub data: Vec<u8>,
    pub keyframe: bool,
}

pub struct H264Encoder {
    enc: Encoder,
    force_key: bool,
    pub width: u32,
    pub height: u32,
}

impl H264Encoder {
    pub fn new(screen: bool, bitrate_bps: u32, fps: u32) -> Result<H264Encoder> {
        let cfg = EncoderConfig::new()
            .bitrate(BitRate::from_bps(bitrate_bps))
            .max_frame_rate(FrameRate::from_hz(fps as f32))
            .rate_control_mode(RateControlMode::Bitrate)
            .usage_type(if screen { UsageType::ScreenContentRealTime } else { UsageType::CameraVideoRealTime })
            .skip_frames(true)
            .complexity(if screen { Complexity::Low } else { Complexity::Medium })
            .intra_frame_period(IntraFramePeriod::from_num_frames(fps.max(1) * 10))
            .num_threads(0);
        let enc = Encoder::with_api_config(OpenH264API::from_source(), cfg)?;
        Ok(H264Encoder { enc, force_key: true, width: 0, height: 0 })
    }

    pub fn request_keyframe(&mut self) {
        self.force_key = true;
    }

    pub fn encode(&mut self, img: &I420) -> Result<Option<EncodedFrame>> {
        if self.width != img.width || self.height != img.height {
            self.width = img.width;
            self.height = img.height;
            self.force_key = true;
        }
        if self.force_key {
            self.enc.force_intra_frame();
            self.force_key = false;
        }
        let w = img.width as usize;
        let h = img.height as usize;
        let src = YUVSlices::new((&img.y, &img.u, &img.v), (w, h), (w, w.div_ceil(2), w.div_ceil(2)));
        let bs = self.enc.encode(&src)?;
        let kind = bs.frame_type();
        if matches!(kind, FrameType::Skip | FrameType::Invalid) {
            return Ok(None);
        }
        let data = bs.to_vec();
        if data.is_empty() {
            return Ok(None);
        }
        Ok(Some(EncodedFrame { data, keyframe: matches!(kind, FrameType::IDR | FrameType::I) }))
    }
}

pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

pub struct H264Decoder {
    dec: Decoder,
}

impl H264Decoder {
    pub fn new() -> Result<H264Decoder> {
        Ok(H264Decoder { dec: Decoder::new()? })
    }

    pub fn decode(&mut self, data: &[u8]) -> Result<Option<DecodedFrame>> {
        let Some(yuvf) = self.dec.decode(data)? else { return Ok(None) };
        let (w, h) = yuvf.dimensions();
        // Ingle never sends more than 1440p; anything past 4K is junk (or someone trying to
        // make us allocate huge pictures).
        if w == 0 || h == 0 || w > 4096 || h > 2304 {
            bail!("video frame of {w}x{h} is too large");
        }
        let (ys, us, vs) = yuvf.strides();
        let planar = YuvPlanarImage {
            y_plane: yuvf.y(),
            y_stride: ys as u32,
            u_plane: yuvf.u(),
            u_stride: us as u32,
            v_plane: yuvf.v(),
            v_stride: vs as u32,
            width: w as u32,
            height: h as u32,
        };
        let mut rgba = vec![0u8; w * h * 4];
        yuv::yuv420_to_rgba(&planar, &mut rgba, (w * 4) as u32, YuvRange::Limited, YuvStandardMatrix::Bt601)
            .map_err(|e| anyhow!("colour conversion failed: {e:?}"))?;
        Ok(Some(DecodedFrame { width: w as u32, height: h as u32, rgba }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::capture::test_pattern;

    #[test]
    fn encode_decode_roundtrip_keeps_the_picture() {
        let mut enc = H264Encoder::new(false, 2_000_000, 30).unwrap();
        let mut dec = H264Decoder::new().unwrap();
        let mut decoded = 0;
        let mut first_key = None;
        for i in 0..20u64 {
            let frame = test_pattern(640, 360, i);
            let yuv = to_i420(&frame).unwrap();
            let Some(e) = enc.encode(&yuv).unwrap() else { continue };
            if first_key.is_none() {
                first_key = Some(e.keyframe);
            }
            if let Some(d) = dec.decode(&e.data).unwrap() {
                assert_eq!((d.width, d.height), (640, 360));
                // Compare the colour of a flat region with the source.
                let px = |buf: &[u8], x: usize, y: usize, bpp: usize| {
                    let o = (y * 640 + x) * bpp;
                    [buf[o] as i32, buf[o + 1] as i32, buf[o + 2] as i32]
                };
                let a = px(&frame.data, 40, 300, 4);
                let b = px(&d.rgba, 40, 300, 4);
                for c in 0..3 {
                    assert!((a[c] - b[c]).abs() < 24, "pixel drift {a:?} vs {b:?}");
                }
                decoded += 1;
            }
        }
        assert_eq!(first_key, Some(true), "first frame must be a keyframe");
        assert!(decoded >= 15, "decoded only {decoded}");
    }

    #[test]
    fn scaling_and_fit() {
        assert_eq!(fit_size(2560, 1440, 1920, 1080), (1920, 1080));
        assert_eq!(fit_size(1001, 601, 4000, 4000), (1000, 600));
        let mut f = test_pattern(320, 180, 0);
        let mut s = Scaler::default();
        let g = s.scale(&mut f, 160, 90).unwrap().unwrap();
        assert_eq!(g.data.len(), 160 * 90 * 4);
        assert!(s.scale(&mut f, 320, 180).unwrap().is_none());
        let (w, h, px) = preview_rgba(&mut s, &mut f, 64, 64).unwrap();
        assert_eq!((w, h, px.len()), (64, 36, 64 * 36 * 4));
    }
}
