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

/// What a capture gives the encoder: a frame in memory, or (Windows' screen capture, for the
/// graphics card's encoder, 0.13.26) one kept on the graphics card.
pub enum Captured {
    Raw(RawFrame),
    #[cfg(windows)]
    Card(super::hw::CardFrame),
}

impl Captured {
    pub fn size(&self) -> (u32, u32) {
        match self {
            Captured::Raw(f) => (f.width, f.height),
            #[cfg(windows)]
            Captured::Card(c) => (c.width, c.height),
        }
    }

    /// The frame, if it's in memory.
    pub fn raw(&self) -> Option<&RawFrame> {
        match self {
            Captured::Raw(f) => Some(f),
            #[cfg(windows)]
            Captured::Card(_) => None,
        }
    }
}

/// Holds only the newest frame; the encoder takes it when ready (older frames are dropped).
#[derive(Default)]
pub struct FrameSlot {
    frame: Mutex<Option<Captured>>,
    cv: Condvar,
    /// The capture may keep its frames on the graphics card (set before it starts).
    on_card: AtomicBool,
}

impl FrameSlot {
    pub fn put(&self, f: RawFrame) {
        self.put_captured(Captured::Raw(f));
    }

    pub fn put_captured(&self, f: Captured) {
        *self.frame.lock() = Some(f);
        self.cv.notify_one();
    }

    /// The newest frame in memory. (Frames kept on the card only go to a slot that asked for
    /// them, whose encoder takes them with `take_captured_wait`.)
    pub fn take_wait(&self, timeout: Duration) -> Option<RawFrame> {
        match self.take_captured_wait(timeout)? {
            Captured::Raw(f) => Some(f),
            #[cfg(windows)]
            Captured::Card(_) => None,
        }
    }

    pub fn take_captured_wait(&self, timeout: Duration) -> Option<Captured> {
        let mut g = self.frame.lock();
        if g.is_none() {
            self.cv.wait_for(&mut g, timeout);
        }
        g.take()
    }

    /// Lets the capture keep its frames on the graphics card for the card's encoder (Windows'
    /// screen capture; others always give frames in memory).
    pub fn keep_on_card(&self, yes: bool) {
        self.on_card.store(yes, Ordering::Relaxed);
    }

    pub fn on_card(&self) -> bool {
        self.on_card.load(Ordering::Relaxed)
    }
}

/// A running capture; dropping it stops capturing.
pub struct CaptureHandle {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    #[cfg(windows)]
    wgc: Option<windows_capture::capture::CaptureControl<win::WgcHandler, Box<dyn std::error::Error + Send + Sync>>>,
    /// What Windows' capture says about itself (see `trouble`).
    #[cfg(windows)]
    watch: Option<Arc<win::WgcWatch>>,
    /// Our green frame in place of Windows' yellow one.
    #[cfg(windows)]
    border: Option<super::border::ShareBorder>,
}

/// What went wrong with a capture that's still meant to be running.
#[derive(Clone, Debug, PartialEq)]
pub enum Trouble {
    /// What was being captured is gone (the window closed, the screen was unplugged).
    Gone(String),
    /// Capturing stopped working (a game changing the display, say, or the graphics driver
    /// restarting): starting it again may well work.
    Broken(String),
}

impl CaptureHandle {
    /// For a preview: no "you're sharing" frame around the screen (Pyr's green one).
    #[cfg_attr(not(windows), allow(unused_mut))]
    pub fn without_border(mut self) -> CaptureHandle {
        #[cfg(windows)]
        drop(self.border.take());
        self
    }

    pub(crate) fn thread(stop: Arc<AtomicBool>, t: std::thread::JoinHandle<()>) -> CaptureHandle {
        CaptureHandle {
            stop,
            thread: Some(t),
            #[cfg(windows)]
            wgc: None,
            #[cfg(windows)]
            watch: None,
            #[cfg(windows)]
            border: None,
        }
    }

    /// Whether it has stopped working by itself (looked at about once a second while sharing).
    pub fn trouble(&self) -> Option<Trouble> {
        if self.stop.load(Ordering::Relaxed) {
            return None;
        }
        #[cfg(windows)]
        if let (Some(c), Some(w)) = (&self.wgc, &self.watch) {
            return w.trouble(c.is_finished());
        }
        match &self.thread {
            Some(t) if t.is_finished() => Some(Trouble::Broken("capturing stopped".into())),
            _ => None,
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
        #[cfg(windows)]
        drop(self.border.take());
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

/// The static part of the test card (color bars over a warm gradient), RGBA.
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

/// A colorful moving test card (RGBA). Deterministic for a given frame index.
pub fn test_pattern(w: u32, h: u32, index: u64) -> RawFrame {
    let mut data = test_background(w, h);
    draw_box(&mut data, w, h, index);
    RawFrame { width: w, height: h, format: PixelFormat::Rgba, data }
}

fn start_test_pattern(slot: Arc<FrameSlot>, w: u32, h: u32, fps: u32) -> CaptureHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let st = stop.clone();
    let t = std::thread::Builder::new()
        .name("pyr-testpattern".into())
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

/// Made-up screens, for trying the share picker (0.13.14, any build): `PYR_FAKE_SCREENS=N`
/// lists the first N of these after the real ones (four at most). Modelled on Blue's own.
const FAKE_SCREENS: [(&str, u32, u32); 4] = [("XB273K GP", 3840, 2160), ("M420SL", 1920, 1080), ("Studio Display", 5120, 2880), ("Portable 16", 2560, 1600)];

/// Made-up windows (`PYR_FAKE_WINDOWS=N`: N of them, these ten over and over, forty at most).
const FAKE_WINDOWS: [(&str, u32, u32); 10] = [
    ("#general - Firelit Friends", 3862, 2110),
    ("Steam", 3840, 2088),
    ("hot-property - File Explorer", 1613, 961),
    ("Watching a long film with a very long name, season 6 episode 20 - a streaming site \u{2014} Mozilla Firefox", 3862, 2110),
    ("Friends List", 548, 990),
    ("(Unsaved) - Blender 5.2", 3862, 2110),
    ("Settings", 1800, 1398),
    ("Untitled - Notepad", 1280, 720),
    ("Spotify Premium", 1600, 1000),
    ("Calculator", 400, 640),
];

/// What can be shared, as 0.13.28 lists it: the screens, the windows, then the test pictures
/// (the share picker shows them under Screens, Windows and Other).
pub fn list_screens() -> Vec<ScreenSource> {
    let mut v = platform_screens();
    let count = |name: &str| pyr_proto::env_var(name).and_then(|n| n.trim().parse::<usize>().ok()).unwrap_or(0);
    // (Named as the real ones were once, "Screen 2 — XB273K GP", numbered after them; the picker
    // shows the number on the tile and the rest under it.)
    let real = v.iter().filter(|s| s.kind == SourceKind::Monitor).count();
    for (i, (name, w, h)) in FAKE_SCREENS.iter().take(count("FAKE_SCREENS")).enumerate() {
        let name = format!("Screen {} \u{2014} {name}", i + real + 1);
        v.push(ScreenSource { id: format!("fake-screen:{i}"), name, kind: SourceKind::Monitor, width: *w, height: *h });
    }
    for i in 0..count("FAKE_WINDOWS").min(40) {
        let (name, w, h) = FAKE_WINDOWS[i % FAKE_WINDOWS.len()];
        v.push(ScreenSource { id: format!("fake:{i}"), name: name.into(), kind: SourceKind::Window, width: w, height: h });
    }
    v.push(ScreenSource {
        id: TEST_PATTERN_ID.into(),
        name: "Test pattern".into(),
        kind: SourceKind::Test,
        width: 1920,
        height: 1080,
    });
    // For trying a share over a slow or far-away connection with any build:
    // `PYR_TEST_SOURCES=1` lists the moving and the scrolling test pictures too, and the desktop
    // that sits still and then turns into a game (testsrc.rs).
    if pyr_proto::env_set("TEST_SOURCES") {
        for (id, name) in [
            (super::testsrc::MOTION_ID, "Test: moving picture"),
            (super::testsrc::TEXT_ID, "Test: scrolling text"),
            (super::testsrc::DESK_ID, "Test: still desktop, then a game"),
        ] {
            v.push(ScreenSource { id: id.into(), name: name.into(), kind: SourceKind::Test, width: 1920, height: 1080 });
        }
    }
    v
}

pub fn start_screen(id: &str, fps: u32, slot: Arc<FrameSlot>) -> Result<CaptureHandle> {
    if id == TEST_PATTERN_ID {
        return Ok(start_test_pattern(slot, 1920, 1080, fps));
    }
    // (Synthetic screens for testing over slow networks, not in the picker: video/testsrc.rs.)
    if let Some(h) = super::testsrc::start(id, 1920, 1080, fps, slot.clone()) {
        return Ok(h);
    }
    platform_start_screen(id, fps, slot)
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows::Win32::Graphics::Direct3D11::ID3D11Device;
    use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
    use windows_capture::d3d11::SendDirectX;
    use windows_capture::frame::Frame;
    use windows_capture::graphics_capture_api::InternalCaptureControl;

    /// What Windows' capture tells us about itself, for `CaptureHandle::trouble`.
    pub struct WgcWatch {
        /// "window" or "screen".
        pub what: &'static str,
        /// Why it stopped (a frame couldn't be read).
        error: parking_lot::Mutex<Option<String>>,
        /// What was being captured went away.
        closed: AtomicBool,
        /// Its graphics device, to ask whether Windows took it away (a game changing the
        /// display mode or the graphics driver restarting does that).
        device: parking_lot::Mutex<Option<SendDirectX<ID3D11Device>>>,
    }

    impl WgcWatch {
        pub fn new(what: &'static str) -> Arc<WgcWatch> {
            Arc::new(WgcWatch { what, error: Default::default(), closed: AtomicBool::new(false), device: Default::default() })
        }

        pub fn trouble(&self, finished: bool) -> Option<Trouble> {
            if self.closed.load(Ordering::Relaxed) {
                return Some(Trouble::Gone(if self.what == "window" { "the window you were sharing closed".into() } else { "the screen you were sharing went away".into() }));
            }
            if finished {
                let why = self.error.lock().clone().unwrap_or_else(|| "Windows stopped capturing it".into());
                return Some(Trouble::Broken(format!("the {} capture stopped: {why}", self.what)));
            }
            // (The device is thread-safe: D3D11 devices are, unless made single-threaded.)
            let lost = self.device.lock().as_ref().and_then(|d| unsafe { d.0.GetDeviceRemovedReason() }.err());
            lost.map(|e| Trouble::Broken(format!("Windows took the {} capture's graphics device away ({e})", self.what)))
        }
    }

    pub struct WgcHandler {
        slot: Arc<FrameSlot>,
        buf: Vec<u8>,
        /// Frames closer together than this are skipped (Windows versions that can't be
        /// told a frame rate send one per screen refresh).
        gap: Duration,
        last: Option<std::time::Instant>,
        watch: Arc<WgcWatch>,
        /// Frames kept on the graphics card for its encoder (0.13.26), when the slot wants them:
        /// copies of Windows' frames in textures of the capture's own.
        copies: Option<crate::video::hw::Copies>,
    }

    impl GraphicsCaptureApiHandler for WgcHandler {
        type Flags = (Arc<FrameSlot>, Duration, Arc<WgcWatch>);
        type Error = Box<dyn std::error::Error + Send + Sync>;

        fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
            let (slot, gap, watch) = ctx.flags;
            *watch.device.lock() = Some(SendDirectX::new(ctx.device.clone()));
            let copies = if slot.on_card() {
                match crate::video::hw::Copies::new(&ctx.device, &ctx.device_context) {
                    Ok(c) => Some(c),
                    Err(e) => {
                        tracing::warn!("video: the capture's frames can't stay on the graphics card ({e:#}); reading them back");
                        None
                    }
                }
            } else {
                None
            };
            Ok(WgcHandler { slot, buf: Vec::new(), gap, last: None, watch, copies })
        }

        fn on_closed(&mut self) -> Result<(), Self::Error> {
            self.watch.closed.store(true, Ordering::Relaxed);
            Ok(())
        }

        fn on_frame_arrived(&mut self, frame: &mut Frame, _control: InternalCaptureControl) -> Result<(), Self::Error> {
            let now = std::time::Instant::now();
            // (Frames kept on the card are taken twice as often: a copy there costs little, and
            // the encoder takes the newest.)
            let gap = if self.copies.is_some() { self.gap / 2 } else { self.gap };
            if let Some(t) = self.last {
                // A little slack so 60 fps from a 60 Hz screen isn't halved.
                if now.duration_since(t) + Duration::from_millis(2) < gap {
                    return Ok(());
                }
            }
            self.last = Some(now);
            if let Some(copies) = self.copies.as_mut() {
                match copies.copy(frame.as_raw_texture(), frame.width(), frame.height()) {
                    Ok(f) => {
                        self.slot.put_captured(Captured::Card(f));
                        return Ok(());
                    }
                    Err(e) => {
                        tracing::warn!("video: couldn't keep the frame on the graphics card ({e:#}); reading frames back from now on");
                        self.copies = None;
                    }
                }
            }
            let fb = match frame.buffer() {
                Ok(fb) => fb,
                Err(e) => {
                    *self.watch.error.lock() = Some(e.to_string());
                    return Err(e.into());
                }
            };
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
            // (Windows' own name for the monitor, as it shows in Display settings; "Display 2" when
            // it has none.)
            let name = m.name().ok().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("Display {}", i + 1));
            // Known by Windows' own name for it too (\\.\DISPLAY2), so starting it again after
            // the graphics driver restarts (when screens can be numbered anew) finds this one,
            // never another.
            let device = m.device_name().unwrap_or_default();
            v.push(ScreenSource {
                id: if device.is_empty() { format!("mon:{}", i + 1) } else { format!("mon:{}:{device}", i + 1) },
                name,
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
            if title.trim().is_empty() || w.process_id().ok() == Some(me) || cloaked(w.as_raw_hwnd()) {
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

/// Whether Windows keeps a window hidden though it's "visible" (cloaked: "Windows Input
/// Experience", an app that's asleep, a window on another virtual desktop): not in the share
/// picker (0.13.14).
#[cfg(windows)]
fn cloaked(hwnd: *mut std::ffi::c_void) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
    let mut cloaked = 0u32;
    // SAFETY: a window handle Windows just listed, and a u32 for the answer.
    let got = unsafe { DwmGetWindowAttribute(HWND(hwnd), DWMWA_CLOAKED, (&raw mut cloaked).cast(), std::mem::size_of::<u32>() as u32) };
    got.is_ok() && cloaked != 0
}

/// The process that owns a shared window (for capturing just that app's sound).
#[cfg(windows)]
pub fn window_process_id(source_id: &str) -> Option<u32> {
    let hwnd: usize = source_id.strip_prefix("win:")?.parse().ok()?;
    let w = windows_capture::window::Window::from_raw_hwnd(hwnd as *mut std::ffi::c_void);
    w.process_id().ok()
}

/// Windows' capture usually starts in well under a second; it's been seen to hang while a game
/// was changing the display. Past this it counts as not starting (and the stuck start is
/// stopped if it ever finishes).
#[cfg(windows)]
const START_WITHIN: Duration = Duration::from_secs(10);

#[cfg(windows)]
fn platform_start_screen(id: &str, fps: u32, slot: Arc<FrameSlot>) -> Result<CaptureHandle> {
    let (tx, rx) = std::sync::mpsc::channel();
    let id = id.to_string();
    std::thread::Builder::new().name("pyr-capture-start".into()).spawn(move || {
        // Too late: whoever asked has given up on it, so it stops again.
        if let Err(std::sync::mpsc::SendError(late)) = tx.send(start_wgc_source(&id, fps, slot)) {
            drop(late);
        }
    })?;
    match rx.recv_timeout(START_WITHIN) {
        Ok(r) => r,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(anyhow!("Windows didn't start capturing within {} seconds", START_WITHIN.as_secs())),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(anyhow!("the capture couldn't start")),
    }
}

#[cfg(windows)]
fn start_wgc_source(id: &str, fps: u32, slot: Arc<FrameSlot>) -> Result<CaptureHandle> {
    use windows_capture::monitor::Monitor;
    use windows_capture::settings::{
        ColorFormat, CursorCaptureSettings, DirtyRegionSettings, MinimumUpdateIntervalSettings, SecondaryWindowSettings,
        Settings,
    };
    use super::border::Target;
    use windows_capture::window::Window;
    use windows_capture::graphics_capture_api::GraphicsCaptureApi;
    // (Twice as often as the frame rate, 0.13.26: from a 144 Hz screen Windows gave a 60 fps
    // share every third refresh, 48 a second; the share takes them in turn.)
    let gap = Duration::from_micros(1_000_000 / fps.max(1) as u64 / 2);
    // Older Windows 10 can't be given a frame rate or told to hide the cursor setting;
    // leave those at Windows' defaults there (the handler skips extra frames itself).
    let interval = if GraphicsCaptureApi::is_minimum_update_interval_supported().unwrap_or(false) {
        MinimumUpdateIntervalSettings::Custom(gap)
    } else {
        MinimumUpdateIntervalSettings::Default
    };
    let cursor = if GraphicsCaptureApi::is_cursor_settings_supported().unwrap_or(false) {
        CursorCaptureSettings::WithCursor
    } else {
        CursorCaptureSettings::Default
    };
    let control = if let Some(rest) = id.strip_prefix("mon:") {
        let (idx, device) = rest.split_once(':').unwrap_or((rest, ""));
        let idx: usize = idx.parse()?;
        // The very screen picked: by Windows' name for it when there is one (its number can
        // change when the graphics driver restarts or a screen is unplugged); never another.
        let m = if device.is_empty() {
            Monitor::from_index(idx).map_err(|e| anyhow!("screen {idx} not found: {e}"))?
        } else {
            Monitor::enumerate()
                .map_err(|e| anyhow!("screen {idx} not found: {e}"))?
                .into_iter()
                .find(|m| m.device_name().is_ok_and(|d| d == device))
                .ok_or_else(|| anyhow!("screen {idx} not found (it's gone, or not back yet)"))?
        };
        let target = Target::Monitor(m.as_raw_hmonitor() as isize);
        let watch = win::WgcWatch::new("screen");
        let make = |border| {
            Settings::new(
                m,
                cursor,
                border,
                SecondaryWindowSettings::Default,
                interval,
                DirtyRegionSettings::Default,
                ColorFormat::Bgra8,
                (slot.clone(), gap, watch.clone()),
            )
        };
        (start_wgc(make, target).map_err(|e| anyhow!("screen capture failed: {e}"))?, watch)
    } else if let Some(h) = id.strip_prefix("win:") {
        let hwnd: usize = h.parse()?;
        let w = Window::from_raw_hwnd(hwnd as *mut std::ffi::c_void);
        if !w.is_valid() {
            return Err(anyhow!("that window was closed"));
        }
        let target = Target::Window(hwnd as isize);
        let watch = win::WgcWatch::new("window");
        let make = |border| {
            Settings::new(
                w,
                cursor,
                border,
                SecondaryWindowSettings::Default,
                interval,
                DirtyRegionSettings::Default,
                ColorFormat::Bgra8,
                (slot.clone(), gap, watch.clone()),
            )
        };
        (start_wgc(make, target).map_err(|e| anyhow!("window capture failed: {e}"))?, watch)
    } else {
        return Err(anyhow!("unknown capture source"));
    };
    let ((control, border), watch) = control;
    Ok(CaptureHandle { stop: Arc::new(AtomicBool::new(false)), thread: None, wgc: Some(control), watch: Some(watch), border })
}

/// Starts Windows' capture without its yellow frame and puts up Pyr's green one instead.
/// Where Windows doesn't allow turning its frame off, its own frame stays.
#[cfg(windows)]
fn start_wgc<T: TryInto<windows_capture::settings::GraphicsCaptureItemType> + Send + 'static>(
    make: impl Fn(windows_capture::settings::DrawBorderSettings) -> windows_capture::settings::Settings<(Arc<FrameSlot>, Duration, Arc<win::WgcWatch>), T>,
    target: super::border::Target,
) -> Result<(windows_capture::capture::CaptureControl<win::WgcHandler, Box<dyn std::error::Error + Send + Sync>>, Option<super::border::ShareBorder>), String> {
    use windows_capture::capture::GraphicsCaptureApiHandler;
    use windows_capture::graphics_capture_api::GraphicsCaptureApi;
    use windows_capture::settings::DrawBorderSettings;
    if GraphicsCaptureApi::is_border_settings_supported().unwrap_or(false) {
        match win::WgcHandler::start_free_threaded(make(DrawBorderSettings::WithoutBorder)) {
            Ok(c) => return Ok((c, Some(super::border::ShareBorder::show(target)))),
            Err(e) => tracing::warn!("capture without Windows' frame failed ({e}); using its frame"),
        }
    }
    win::WgcHandler::start_free_threaded(make(DrawBorderSettings::Default)).map(|c| (c, None)).map_err(|e| e.to_string())
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
        .name("pyr-x11capture".into())
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
            .name("pyr-camera".into())
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
