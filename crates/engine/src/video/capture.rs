//! Frame sources: screen capture, webcams and a synthetic test pattern.
//!
//! * Windows: Windows Graphics Capture (monitors and individual windows), Media Foundation webcams.
//! * Linux: X11 screen capture, V4L2 webcams. (Wayland portal capture is on the roadmap.)
//! * macOS: webcams via AVFoundation; screen capture is on the roadmap.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use parking_lot::{Condvar, Mutex};

use super::codec::{PixelFormat, RawFrame};

/// Holds only the newest frame; the encoder takes it when ready (older frames are dropped).
#[derive(Default)]
pub struct FrameSlot {
    frame: Mutex<Option<RawFrame>>,
    cv: Condvar,
}

impl FrameSlot {
    pub fn put(&self, f: RawFrame) {
        *self.frame.lock() = Some(f);
        self.cv.notify_one();
    }

    pub fn take_wait(&self, timeout: Duration) -> Option<RawFrame> {
        let mut g = self.frame.lock();
        if g.is_none() {
            self.cv.wait_for(&mut g, timeout);
        }
        g.take()
    }
}

/// A running capture; dropping it stops capturing.
pub struct CaptureHandle {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    #[cfg(windows)]
    wgc: Option<windows_capture::capture::CaptureControl<win::WgcHandler, Box<dyn std::error::Error + Send + Sync>>>,
}

impl CaptureHandle {
    fn thread(stop: Arc<AtomicBool>, t: std::thread::JoinHandle<()>) -> CaptureHandle {
        CaptureHandle {
            stop,
            thread: Some(t),
            #[cfg(windows)]
            wgc: None,
        }
    }
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        #[cfg(windows)]
        if let Some(c) = self.wgc.take() {
            let _ = c.stop();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Monitor,
    Window,
    Test,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScreenSource {
    pub id: String,
    pub name: String,
    pub kind: SourceKind,
    pub width: u32,
    pub height: u32,
}

pub const TEST_PATTERN_ID: &str = "test:pattern";

/// The static part of the test card (colour bars over a warm gradient), RGBA.
fn test_background(w: u32, h: u32) -> Vec<u8> {
    let mut data = vec![0u8; (w * h * 4) as usize];
    let bars: [[u8; 3]; 7] =
        [[230, 230, 230], [230, 200, 40], [40, 200, 210], [40, 190, 60], [210, 50, 200], [210, 50, 40], [40, 60, 210]];
    for y in 0..h {
        for x in 0..w {
            let o = ((y * w + x) * 4) as usize;
            let c = if y < h * 2 / 3 {
                bars[(x * 7 / w) as usize]
            } else {
                let g = (x * 255 / w.max(1)) as u8;
                [255, (120 + g / 3).min(255), 40]
            };
            data[o..o + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    data
}

fn draw_box(data: &mut [u8], w: u32, h: u32, index: u64) {
    let box_size = (h / 5).max(8);
    let t = index as f32 / 30.0;
    let bx = (((t * 1.3).sin() * 0.5 + 0.5) * (w - box_size) as f32) as u32;
    let by = (((t * 1.7).cos() * 0.5 + 0.5) * (h * 2 / 3 - box_size) as f32) as u32;
    for y in by..(by + box_size).min(h) {
        let row = ((y * w + bx) * 4) as usize;
        let end = row + (box_size.min(w - bx) * 4) as usize;
        for px in data[row..end].chunks_exact_mut(4) {
            px.copy_from_slice(&[20, 20, 20, 255]);
        }
    }
}

/// A colourful moving test card (RGBA). Deterministic for a given frame index.
pub fn test_pattern(w: u32, h: u32, index: u64) -> RawFrame {
    let mut data = test_background(w, h);
    draw_box(&mut data, w, h, index);
    RawFrame { width: w, height: h, format: PixelFormat::Rgba, data }
}

fn start_test_pattern(slot: Arc<FrameSlot>, w: u32, h: u32, fps: u32) -> CaptureHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let st = stop.clone();
    let t = std::thread::Builder::new()
        .name("ingle-testpattern".into())
        .spawn(move || {
            let start = Instant::now();
            let background = test_background(w, h);
            let mut i = 0u64;
            while !st.load(Ordering::Relaxed) {
                let mut data = background.clone();
                draw_box(&mut data, w, h, i);
                slot.put(RawFrame { width: w, height: h, format: PixelFormat::Rgba, data });
                i += 1;
                let next = start + Duration::from_secs_f64(i as f64 / fps.max(1) as f64);
                if let Some(d) = next.checked_duration_since(Instant::now()) {
                    std::thread::sleep(d);
                }
            }
        })
        .expect("spawn test pattern");
    CaptureHandle::thread(stop, t)
}

// ------------------------------------------------------------------ screens

pub fn list_screens() -> Vec<ScreenSource> {
    let mut v = platform_screens();
    v.push(ScreenSource {
        id: TEST_PATTERN_ID.into(),
        name: "Test pattern".into(),
        kind: SourceKind::Test,
        width: 1920,
        height: 1080,
    });
    v
}

pub fn start_screen(id: &str, fps: u32, slot: Arc<FrameSlot>) -> Result<CaptureHandle> {
    if id == TEST_PATTERN_ID {
        return Ok(start_test_pattern(slot, 1920, 1080, fps));
    }
    platform_start_screen(id, fps, slot)
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
    use windows_capture::frame::Frame;
    use windows_capture::graphics_capture_api::InternalCaptureControl;

    pub struct WgcHandler {
        slot: Arc<FrameSlot>,
        buf: Vec<u8>,
    }

    impl GraphicsCaptureApiHandler for WgcHandler {
        type Flags = Arc<FrameSlot>;
        type Error = Box<dyn std::error::Error + Send + Sync>;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            Ok(WgcHandler { slot: ctx.flags, buf: Vec::new() })
        }

        fn on_frame_arrived(&mut self, frame: &mut Frame, _control: InternalCaptureControl) -> Result<(), Self::Error> {
            let fb = frame.buffer()?;
            let (w, h) = (fb.width(), fb.height());
            let data = fb.as_nopadding_buffer(&mut self.buf).to_vec();
            self.slot.put(RawFrame { width: w, height: h, format: PixelFormat::Bgra, data });
            Ok(())
        }
    }
}

#[cfg(windows)]
fn platform_screens() -> Vec<ScreenSource> {
    use windows_capture::monitor::Monitor;
    use windows_capture::window::Window;
    let mut v = Vec::new();
    if let Ok(mons) = Monitor::enumerate() {
        for (i, m) in mons.iter().enumerate() {
            let (w, h) = (m.width().unwrap_or(0), m.height().unwrap_or(0));
            let name = m.name().unwrap_or_else(|_| format!("Display {}", i + 1));
            v.push(ScreenSource {
                id: format!("mon:{}", i + 1),
                name: format!("Screen {} — {}", i + 1, name),
                kind: SourceKind::Monitor,
                width: w,
                height: h,
            });
        }
    }
    if let Ok(wins) = Window::enumerate() {
        let me = std::process::id();
        for w in wins {
            let Ok(title) = w.title() else { continue };
            if title.trim().is_empty() || w.process_id().ok() == Some(me) {
                continue;
            }
            let (ww, wh) = (w.width().unwrap_or(0).max(0) as u32, w.height().unwrap_or(0).max(0) as u32);
            if ww < 64 || wh < 64 {
                continue;
            }
            v.push(ScreenSource {
                id: format!("win:{}", w.as_raw_hwnd() as usize),
                name: title,
                kind: SourceKind::Window,
                width: ww,
                height: wh,
            });
        }
    }
    v
}

/// The process that owns a shared window (for capturing just that app's sound).
#[cfg(windows)]
pub fn window_process_id(source_id: &str) -> Option<u32> {
    let hwnd: usize = source_id.strip_prefix("win:")?.parse().ok()?;
    let w = windows_capture::window::Window::from_raw_hwnd(hwnd as *mut std::ffi::c_void);
    w.process_id().ok()
}

#[cfg(windows)]
fn platform_start_screen(id: &str, fps: u32, slot: Arc<FrameSlot>) -> Result<CaptureHandle> {
    use windows_capture::capture::GraphicsCaptureApiHandler;
    use windows_capture::monitor::Monitor;
    use windows_capture::settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings,
        SecondaryWindowSettings, Settings,
    };
    use windows_capture::window::Window;
    let interval = MinimumUpdateIntervalSettings::Custom(Duration::from_micros(1_000_000 / fps.max(1) as u64));
    let control = if let Some(idx) = id.strip_prefix("mon:") {
        let idx: usize = idx.parse()?;
        let m = Monitor::from_index(idx).map_err(|e| anyhow!("screen {idx} not found: {e}"))?;
        let settings = Settings::new(
            m,
            CursorCaptureSettings::WithCursor,
            DrawBorderSettings::Default,
            SecondaryWindowSettings::Default,
            interval,
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            slot,
        );
        win::WgcHandler::start_free_threaded(settings).map_err(|e| anyhow!("screen capture failed: {e}"))?
    } else if let Some(h) = id.strip_prefix("win:") {
        let hwnd: usize = h.parse()?;
        let w = Window::from_raw_hwnd(hwnd as *mut std::ffi::c_void);
        if !w.is_valid() {
            return Err(anyhow!("that window was closed"));
        }
        let settings = Settings::new(
            w,
            CursorCaptureSettings::WithCursor,
            DrawBorderSettings::Default,
            SecondaryWindowSettings::Default,
            interval,
            DirtyRegionSettings::Default,
            ColorFormat::Bgra8,
            slot,
        );
        win::WgcHandler::start_free_threaded(settings).map_err(|e| anyhow!("window capture failed: {e}"))?
    } else {
        return Err(anyhow!("unknown capture source"));
    };
    Ok(CaptureHandle { stop: Arc::new(AtomicBool::new(false)), thread: None, wgc: Some(control) })
}

#[cfg(target_os = "linux")]
fn platform_screens() -> Vec<ScreenSource> {
    use x11rb::connection::Connection;
    let Ok((conn, screen_num)) = x11rb::connect(None) else { return Vec::new() };
    let screen = &conn.setup().roots[screen_num];
    vec![ScreenSource {
        id: "x11:root".into(),
        name: "Entire screen".into(),
        kind: SourceKind::Monitor,
        width: screen.width_in_pixels as u32,
        height: screen.height_in_pixels as u32,
    }]
}

#[cfg(target_os = "linux")]
fn platform_start_screen(id: &str, fps: u32, slot: Arc<FrameSlot>) -> Result<CaptureHandle> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};
    if id != "x11:root" {
        return Err(anyhow!("unknown capture source"));
    }
    let (conn, screen_num) = x11rb::connect(None).map_err(|e| anyhow!("can't open the X display: {e}"))?;
    let stop = Arc::new(AtomicBool::new(false));
    let st = stop.clone();
    let t = std::thread::Builder::new()
        .name("ingle-x11capture".into())
        .spawn(move || {
            let screen = conn.setup().roots[screen_num].clone();
            let (w, h) = (screen.width_in_pixels, screen.height_in_pixels);
            let period = Duration::from_secs_f64(1.0 / fps.max(1) as f64);
            let mut next = Instant::now();
            while !st.load(Ordering::Relaxed) {
                let reply = conn.get_image(ImageFormat::Z_PIXMAP, screen.root, 0, 0, w, h, !0).ok().and_then(|c| c.reply().ok());
                if let Some(img) = reply {
                    if img.depth >= 24 && img.data.len() >= w as usize * h as usize * 4 {
                        slot.put(RawFrame { width: w as u32, height: h as u32, format: PixelFormat::Bgra, data: img.data });
                    }
                }
                next += period;
                match next.checked_duration_since(Instant::now()) {
                    Some(d) => std::thread::sleep(d),
                    None => next = Instant::now(),
                }
            }
        })
        .expect("spawn capture thread");
    Ok(CaptureHandle::thread(stop, t))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_screens() -> Vec<ScreenSource> {
    Vec::new()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_start_screen(_id: &str, _fps: u32, _slot: Arc<FrameSlot>) -> Result<CaptureHandle> {
    Err(anyhow!("screen sharing isn't supported on this platform yet"))
}

// ------------------------------------------------------------------ cameras

pub const TEST_CAMERA_ID: &str = "test:camera";

pub fn list_cameras() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = Vec::new();
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    if let Ok(cams) = nokhwa::query(nokhwa::utils::ApiBackend::Auto) {
        for c in cams {
            v.push((format!("cam:{}", c.index()), c.human_name()));
        }
    }
    v.push((TEST_CAMERA_ID.into(), "Test pattern camera".into()));
    v
}

pub fn start_camera(id: &str, w: u32, h: u32, fps: u32, slot: Arc<FrameSlot>) -> Result<CaptureHandle> {
    if id == TEST_CAMERA_ID {
        return Ok(start_test_pattern(slot, w, h, fps));
    }
    #[cfg(any(windows, target_os = "linux", target_os = "macos"))]
    {
        use nokhwa::Camera;
        use nokhwa::pixel_format::RgbFormat;
        use nokhwa::utils::{CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType, Resolution};
        let index_str = id.strip_prefix("cam:").ok_or_else(|| anyhow!("unknown camera"))?.to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let st = stop.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let t = std::thread::Builder::new()
            .name("ingle-camera".into())
            .spawn(move || {
                let index = match index_str.parse::<u32>() {
                    Ok(i) => CameraIndex::Index(i),
                    Err(_) => CameraIndex::String(index_str.clone()),
                };
                let req = RequestedFormat::new::<RgbFormat>(RequestedFormatType::Closest(CameraFormat::new(
                    Resolution::new(w, h),
                    FrameFormat::MJPEG,
                    fps,
                )));
                let mut cam = match Camera::new(index.clone(), req) {
                    Ok(c) => c,
                    Err(_) => {
                        // Some cameras don't do MJPEG; let the driver choose.
                        let any = RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate);
                        match Camera::new(index, any) {
                            Ok(c) => c,
                            Err(e) => {
                                let _ = ready_tx.send(Err(format!("couldn't open the camera: {e}")));
                                return;
                            }
                        }
                    }
                };
                if let Err(e) = cam.open_stream() {
                    let _ = ready_tx.send(Err(format!("couldn't start the camera: {e}")));
                    return;
                }
                let _ = ready_tx.send(Ok(()));
                while !st.load(Ordering::Relaxed) {
                    match cam.frame() {
                        Ok(buf) => {
                            if let Ok(img) = buf.decode_image::<RgbFormat>() {
                                let (fw, fh) = (img.width(), img.height());
                                slot.put(RawFrame { width: fw, height: fh, format: PixelFormat::Rgb, data: img.into_raw() });
                            }
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(20)),
                    }
                }
                let _ = cam.stop_stream();
            })
            .expect("spawn camera thread");
        match ready_rx.recv_timeout(Duration::from_secs(8)) {
            Ok(Ok(())) => Ok(CaptureHandle::thread(stop, t)),
            Ok(Err(e)) => {
                let _ = t.join();
                Err(anyhow!(e))
            }
            Err(_) => {
                stop.store(true, Ordering::Relaxed);
                Err(anyhow!("the camera didn't respond"))
            }
        }
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = (w, h, fps, slot);
        Err(anyhow!("cameras aren't supported on this platform"))
    }
}
