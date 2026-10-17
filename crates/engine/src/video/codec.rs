//! H.264 encode/decode (OpenH264) with fast color conversion and scaling.

use anyhow::{Result, anyhow, bail};
use fast_image_resize::images::Image;
use fast_image_resize::{PixelType, ResizeAlg, ResizeOptions, Resizer};
use openh264::OpenH264API;
use openh264::decoder::Decoder;
use openh264::encoder::{
    BitRate, Complexity, Encoder, EncoderConfig, FrameRate, FrameType, IntraFramePeriod, RateControlMode, UsageType,
};
use openh264::formats::{YUVSlices, YUVSource};
use openh264_sys2::{
    ENCODER_OPTION_BITRATE, ENCODER_OPTION_FRAME_RATE, ENCODER_OPTION_MAX_BITRATE, SBitrateInfo, SPATIAL_LAYER_0, SPATIAL_LAYER_ALL,
};
use std::ptr::addr_of_mut;
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
    .map_err(|e| anyhow!("color conversion failed: {e:?}"))?;
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
    /// Made for each picture size (see `encode`).
    enc: Option<Encoder>,
    screen: bool,
    /// A screen share showing motion (a game, a video): encoded the way a camera is (see
    /// `set_motion`).
    motion: bool,
    /// Made again for the next frame (what's shared changed kind).
    remake: bool,
    /// What it's asked to do now (see `set_rates`), and what the encoder in use was last set to.
    bitrate: u32,
    fps: f32,
    applied: (u32, f32),
    /// The encoder in use hasn't encoded anything yet (OpenH264 sets itself up on the first
    /// frame, and can't be changed before).
    fresh: bool,
    /// A keyframe at least every this many frames.
    intra_frames: u32,
    force_key: bool,
    pub width: u32,
    pub height: u32,
    /// Frames OpenH264 skipped to keep to its bitrate (what it's asked for is too much for it).
    pub skipped: u64,
}

impl H264Encoder {
    pub fn new(screen: bool, bitrate_bps: u32, fps: u32) -> Result<H264Encoder> {
        // A keyframe every minute by itself, as a safety net: keyframes are big, and viewers who
        // lose something ask for one anyway. (Before 0.8.0, `PYR_AB=adapt`: every 10 s.)
        let every = if pyr_proto::ab::old("adapt") { 10 } else { 60 };
        let mut e = H264Encoder {
            enc: None,
            screen,
            motion: false,
            remake: false,
            bitrate: bitrate_bps,
            fps: fps.max(1) as f32,
            applied: (0, 0.0),
            fresh: true,
            intra_frames: fps.max(1) * every,
            force_key: true,
            width: 0,
            height: 0,
            skipped: 0,
        };
        // (Checked now, so a broken encoder shows when the share starts.)
        e.enc = Some(e.make()?);
        Ok(e)
    }

    fn make(&self) -> Result<Encoder> {
        let cfg = EncoderConfig::new()
            .bitrate(BitRate::from_bps(self.bitrate))
            .max_frame_rate(FrameRate::from_hz(self.fps))
            .rate_control_mode(RateControlMode::Bitrate)
            .usage_type(if self.screen && !self.motion { UsageType::ScreenContentRealTime } else { UsageType::CameraVideoRealTime })
            .skip_frames(true)
            .complexity(if self.screen && !self.motion { Complexity::Low } else { Complexity::Medium })
            // A keyframe whenever the picture changes a lot (a fast pan in a game, at a few
            // frames a second) made shares on slow connections worse: each is several times a
            // normal frame's size. (`PYR_AB=scenecut` puts it back.)
            .scene_change_detect(pyr_proto::ab::old("scenecut"))
            .intra_frame_period(IntraFramePeriod::from_num_frames(self.intra_frames))
            .num_threads(0);
        Ok(Encoder::with_api_config(OpenH264API::from_source(), cfg)?)
    }

    pub fn request_keyframe(&mut self) {
        self.force_key = true;
    }

    /// A screen share showing motion (a game, a video) is encoded the way a camera is: in
    /// OpenH264's screen mode, every frame that changes a lot from the last becomes a keyframe
    /// (it can't be turned off there), which at a few frames a second is nearly all of them;
    /// and it may only lower the quality so far, skipping frames instead. Text and desktops
    /// stay in screen mode, which keeps them sharp. (Changing costs a keyframe.)
    pub fn set_motion(&mut self, motion: bool) {
        if self.screen && motion != self.motion {
            self.motion = motion;
            self.remake = true;
        }
    }

    /// The bitrate and frame rate to encode at from the next frame on (the network's and the
    /// share's, see `video::adapt`).
    pub fn set_rates(&mut self, bitrate_bps: u32, fps: f32) {
        self.bitrate = bitrate_bps.max(50_000);
        self.fps = fps.clamp(1.0, 60.0);
    }

    /// Tells OpenH264 the new bitrate and frame rate without starting over.
    fn apply_rates(&mut self) {
        let Some(enc) = self.enc.as_mut() else { return };
        let (bitrate, fps) = (self.bitrate as i32, self.fps);
        let raising = self.bitrate > self.applied.0;
        // SAFETY: the options and their argument types are OpenH264's (codec_app_def.h); the
        // encoder is set up (it has encoded a frame).
        unsafe {
            let api = enc.raw_api();
            let mut target = SBitrateInfo { iLayer: SPATIAL_LAYER_ALL, iBitrate: bitrate };
            // (It won't take a bitrate above its most, so that goes up first, or down after.)
            let mut most = SBitrateInfo { iLayer: SPATIAL_LAYER_0, iBitrate: bitrate.saturating_mul(3) / 2 };
            if raising {
                api.set_option(ENCODER_OPTION_MAX_BITRATE, addr_of_mut!(most).cast());
            }
            api.set_option(ENCODER_OPTION_BITRATE, addr_of_mut!(target).cast());
            if !raising {
                api.set_option(ENCODER_OPTION_MAX_BITRATE, addr_of_mut!(most).cast());
            }
            let mut f = fps;
            api.set_option(ENCODER_OPTION_FRAME_RATE, addr_of_mut!(f).cast());
        }
        self.applied = (self.bitrate, self.fps);
    }

    pub fn encode(&mut self, img: &I420) -> Result<Option<EncodedFrame>> {
        if self.width != img.width || self.height != img.height || self.enc.is_none() || self.remake {
            // A new size: a new encoder, made for the bitrate and frame rate wanted now (set up
            // again for a new size, OpenH264 would go back to the ones it started with). Its
            // first frame is a keyframe.
            if self.width != 0 || self.enc.is_none() || self.remake {
                self.enc = Some(self.make()?);
            }
            self.remake = false;
            self.width = img.width;
            self.height = img.height;
            self.applied = (self.bitrate, self.fps);
            self.fresh = true;
            self.force_key = false;
        }
        if !self.fresh && (self.bitrate, self.fps) != self.applied {
            self.apply_rates();
        }
        let enc = self.enc.as_mut().expect("made above");
        if self.force_key && !self.fresh {
            enc.force_intra_frame();
        }
        self.force_key = false;
        let w = img.width as usize;
        let h = img.height as usize;
        let src = YUVSlices::new((&img.y, &img.u, &img.v), (w, h), (w, w.div_ceil(2), w.div_ceil(2)));
        let bs = enc.encode(&src)?;
        self.fresh = false;
        let kind = bs.frame_type();
        if matches!(kind, FrameType::Skip | FrameType::Invalid) {
            self.skipped += 1;
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

    /// Decodes a frame; with `show`, the picture comes back too (without, it's only decoded,
    /// for catching up: the next frame's here already).
    pub fn decode(&mut self, data: &[u8], show: bool) -> Result<Option<DecodedFrame>> {
        let Some(yuvf) = self.dec.decode(data)? else { return Ok(None) };
        if !show {
            return Ok(None);
        }
        let (w, h) = yuvf.dimensions();
        // Pyr never sends more than 1440p; anything past 4K is junk (or someone trying to
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
            .map_err(|e| anyhow!("color conversion failed: {e:?}"))?;
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
            if let Some(d) = dec.decode(&e.data, true).unwrap() {
                assert_eq!((d.width, d.height), (640, 360));
                // Compare the color of a flat region with the source.
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
