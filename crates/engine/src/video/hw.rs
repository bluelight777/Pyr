//! A screen share encoded on the graphics card, end to end (Windows, 0.13.26).
//!
//! Windows' screen capture gives each frame as a texture on the graphics card. Here the frame
//! stays there: the capture copies it into one of its own textures (`Copies`), the card's video
//! engine scales it to the share's size and makes it NV12 (`Processor`), and Media Foundation's
//! hardware H.264 encoder (NVIDIA's, AMD's or Intel's) encodes it (`Encoder`). Only the H.264
//! comes back to the processor. While a share is on the card, the card gives Pyr a higher
//! priority (`priority`).
//!
//! When any of it doesn't work, the share goes on the processor's way (each frame read back and
//! encoded by OpenH264, as before 0.13.26) and stays there; a new graphics device for the
//! capture (a driver restart) tries the card again. `PYR_AB=hwenc`: the processor's way always.
//!
//! What 0.13.28 does, read from its program (rebuild/notes-hw.md). This only runs on a Windows
//! PC with a graphics card: it's checked there, A/B against 0.13.28.

use std::collections::VecDeque;
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use tracing::{debug, info};
use windows::Win32::Foundation::LUID;
use windows::Win32::Graphics::Direct3D10::ID3D10Multithread;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::{DXGI_ERROR_WAS_STILL_DRAWING, IDXGIDevice};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_UI4, VariantClear};
use windows::core::{GUID, Interface, PWSTR};

use super::codec::{PixelFormat, RawFrame};

/// The encoder's maker, for the call's stats (`PublisherStats::encoder` is this plus one).
pub const VENDOR_NVIDIA: u8 = 0;
pub const VENDOR_AMD: u8 = 1;
pub const VENDOR_INTEL: u8 = 2;
pub const VENDOR_OTHER: u8 = 3;

/// A texture on the graphics card, handed from the capture's thread to the encoder's. Sound
/// because the capture's device is made safe to use from several threads first (`Copies::new`).
pub struct SendTex(pub ID3D11Texture2D);
unsafe impl Send for SendTex {}

/// A frame the capture kept on the graphics card: a copy of Windows' frame in one of the
/// capture's own textures, and its size.
pub struct CardFrame {
    pub texture: SendTex,
    pub width: u32,
    pub height: u32,
}

impl CardFrame {
    /// The graphics device it's on.
    pub fn device(&self) -> Result<ID3D11Device> {
        Ok(unsafe { self.texture.0.GetDevice()? })
    }
}

/// What the card's encoder made of a frame (there can be none, or more than one, at a time).
pub struct Packet {
    pub data: Vec<u8>,
    pub key: bool,
}

// --- The card's priority ------------------------------------------------------------------------

/// Shares on the card now (the card's priority for Pyr is raised while there's one).
static ON_CARD: AtomicUsize = AtomicUsize::new(0);

/// Raises the card's priority for Pyr (`on`, the first share on the card) or puts it back
/// (the last one ending). With a device, its work on the card gets the highest priority too.
pub fn priority(device: Option<&ID3D11Device>, on: bool) {
    // D3DKMT_SCHEDULINGPRIORITYCLASS: 2 normal, 4 high.
    let class = if on {
        (ON_CARD.fetch_add(1, Ordering::SeqCst) == 0).then_some(4)
    } else {
        (ON_CARD.fetch_sub(1, Ordering::SeqCst) == 1).then_some(2)
    };
    if let Some(class) = class {
        unsafe {
            if let Ok(gdi) = LoadLibraryW(windows::core::w!("gdi32.dll")) {
                if let Some(f) = GetProcAddress(gdi, windows::core::s!("D3DKMTSetProcessSchedulingPriorityClass")) {
                    type SetClass = unsafe extern "system" fn(windows::Win32::Foundation::HANDLE, i32) -> i32;
                    let f: SetClass = std::mem::transmute(f);
                    let status = f(GetCurrentProcess(), class);
                    debug!("video: the card's priority for Pyr set to class {class} (status {status:#x})");
                }
            }
        }
    }
    if let Some(d) = device {
        let r = unsafe { d.cast::<IDXGIDevice>().and_then(|x| x.SetGPUThreadPriority(7)) };
        if let Err(e) = r {
            debug!("video: the capture's device keeps its priority: {e}");
        }
    }
}

// --- The capture's side: frames kept on the card ------------------------------------------------

/// The capture's own textures, three in turn, each frame copied into the next: Windows wants its
/// frame back at once, and the encoder may still be reading the last copy.
pub struct Copies {
    textures: Vec<ID3D11Texture2D>,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    next: usize,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
}

// (The device is made safe to use from several threads in `new`.)
unsafe impl Send for Copies {}

impl Copies {
    /// For the capture's device: made safe to use from the encoder's thread too, and the card's
    /// priority for Pyr raised (until this is dropped).
    pub fn new(device: &ID3D11Device, context: &ID3D11DeviceContext) -> Result<Copies> {
        let mt: ID3D10Multithread = context.cast().context("the capture's device can't be shared between threads")?;
        unsafe {
            let _ = mt.SetMultithreadProtected(true);
        }
        priority(Some(device), true);
        Ok(Copies { textures: Vec::new(), device: device.clone(), context: context.clone(), next: 0, width: 0, height: 0, format: DXGI_FORMAT(0) })
    }

    /// Copies Windows' frame (`w`x`h` of `texture`, made even) into the next texture.
    pub fn copy(&mut self, texture: &ID3D11Texture2D, w: u32, h: u32) -> Result<CardFrame> {
        let d = desc(texture);
        let w = w.min(d.Width).max(2) & !1;
        let h = h.min(d.Height).max(2) & !1;
        if (self.width, self.height, self.format) != (w, h, d.Format) || self.textures.is_empty() {
            self.textures.clear();
            let td = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: d.Format,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            for _ in 0..3 {
                self.textures.push(texture2d(&self.device, &td)?);
            }
            (self.width, self.height, self.format, self.next) = (w, h, d.Format, 0);
        }
        let t = self.textures[self.next].clone();
        self.next = (self.next + 1) % self.textures.len();
        let area = D3D11_BOX { left: 0, top: 0, front: 0, right: w, bottom: h, back: 1 };
        unsafe { self.context.CopySubresourceRegion(&t, 0, 0, 0, 0, texture, 0, Some(&area)) };
        Ok(CardFrame { texture: SendTex(t), width: w, height: h })
    }
}

impl Drop for Copies {
    fn drop(&mut self) {
        priority(None, false);
    }
}

// --- The encoder's side -------------------------------------------------------------------------

/// What one share does on the card: its encoder, the video engine that feeds it, and the small
/// copies for the preview. A new graphics device for the capture starts a new one.
#[derive(Default)]
pub struct Card {
    encoder: Option<Encoder>,
    processor: Option<Processor>,
    /// For the small copies (the preview, and telling text from motion).
    small: Option<Processor>,
    small_readback: Readback,
    /// For reading a whole frame back (the processor's way, after the card's stopped working).
    readback: Option<(ID3D11Texture2D, u32, u32)>,
    /// Why the card's way stopped working, once it has (the share stays on the processor's way).
    pub failed: Option<String>,
    /// This card's encoders don't make keyframes when asked: a new encoder for each.
    new_for_keyframes: bool,
}

impl Card {
    /// The maker of the encoder in use (`VENDOR_*`), if there is one.
    pub fn vendor(&self) -> Option<u8> {
        self.encoder.as_ref().map(|e| e.vendor)
    }

    /// Encodes `frame` at `w`x`h`: the encoder made for that size, frame rate and bitrate (or the
    /// one in use), the frame scaled and made NV12 by the card's video engine, then encoded. On
    /// an error the encoder and video engine are let go and `failed` says why.
    pub fn frame(&mut self, frame: &CardFrame, w: u32, h: u32, fps: u32, bitrate: u32, keyframe: bool) -> Result<Vec<Packet>> {
        let r = self.try_frame(frame, w, h, fps, bitrate, keyframe);
        if let Err(e) = &r {
            self.failed = Some(format!("{e:#}"));
            self.encoder = None;
            self.processor = None;
        }
        r
    }

    fn try_frame(&mut self, frame: &CardFrame, w: u32, h: u32, fps: u32, bitrate: u32, mut keyframe: bool) -> Result<Vec<Packet>> {
        let device = frame.device()?;
        let (w, h) = (w & !1, h & !1);
        // (An encoder that won't make keyframes when asked is made anew for each, and so is
        // every encoder after it on this card.)
        if self.encoder.as_ref().is_some_and(|e| !e.can_ask_keyframe) {
            self.new_for_keyframes = true;
        }
        let anew = keyframe && self.new_for_keyframes;
        let fits = self.encoder.as_ref().is_some_and(|e| (e.fps, e.width, e.height) == (fps.max(1), w, h));
        if anew || !fits {
            self.encoder = None;
            let mut e = Encoder::new(&device, w, h, fps, bitrate)?;
            e.can_ask_keyframe = !self.new_for_keyframes;
            if anew {
                debug!("video: a new encoder for a keyframe ({})", e.name);
            } else {
                info!("video: encoding on the graphics card ({}), {w}x{h} at {fps} fps", e.name);
            }
            self.encoder = Some(e);
            // (A new encoder starts with one.)
            keyframe = false;
        }
        let same = self.processor.as_ref().is_some_and(|p| (p.in_w, p.in_h, p.out_w, p.out_h) == (frame.width, frame.height, w, h));
        if !same {
            self.processor = Some(Processor::new(&device, frame.width, frame.height, w, h, DXGI_FORMAT_NV12, 8)?);
        }
        let nv12 = self.processor.as_mut().expect("made above").convert(frame)?;
        let e = self.encoder.as_mut().expect("made above");
        e.set_bitrate(bitrate);
        e.encode(&nv12, keyframe)
    }

    /// A copy of `frame` at most `max_w`x`max_h` in memory, made on the card: the one asked for
    /// a moment ago (`None` until it's ready; reading back what's still being made would wait
    /// for it).
    pub fn small_copy(&mut self, frame: &CardFrame, max_w: u32, max_h: u32) -> Result<Option<RawFrame>> {
        let (w, h) = super::codec::fit_size(frame.width, frame.height, max_w, max_h);
        let same = self.small.as_ref().is_some_and(|p| (p.in_w, p.in_h, p.out_w, p.out_h) == (frame.width, frame.height, w, h));
        if !same {
            let device = frame.device()?;
            self.small = Some(Processor::new(&device, frame.width, frame.height, w, h, DXGI_FORMAT_B8G8R8A8_UNORM, 1)?);
        }
        let t = self.small.as_mut().expect("made above").convert(frame)?;
        self.small_readback.read(&t)
    }

    /// `frame` read back into memory, whole (for the processor's way).
    pub fn read_back(&mut self, frame: &CardFrame) -> Result<RawFrame> {
        let t = &frame.texture.0;
        let device = unsafe { t.GetDevice()? };
        let context = unsafe { device.GetImmediateContext()? };
        let d = desc(t);
        if d.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
            bail!("can't read back a {:?} texture", d.Format);
        }
        let staging = match &self.readback {
            Some((s, w, h)) if (*w, *h) == (d.Width, d.Height) => s.clone(),
            _ => {
                let s = texture2d(&device, &staging_desc(d.Width, d.Height))?;
                self.readback = Some((s.clone(), d.Width, d.Height));
                s
            }
        };
        unsafe {
            context.CopyResource(&staging, t);
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut m)).context("couldn't read the frame back")?;
            let data = rows(&m, d.Width, d.Height);
            context.Unmap(&staging, 0);
            Ok(RawFrame { width: d.Width, height: d.Height, format: PixelFormat::Bgra, data })
        }
    }
}

/// A frame's rows out of a mapped texture (four bytes a pixel), without the row padding.
unsafe fn rows(m: &D3D11_MAPPED_SUBRESOURCE, w: u32, h: u32) -> Vec<u8> {
    let row = w as usize * 4;
    let mut data = vec![0u8; row * h as usize];
    for y in 0..h as usize {
        let src = unsafe { std::slice::from_raw_parts((m.pData as *const u8).add(y * m.RowPitch as usize), row) };
        data[y * row..(y + 1) * row].copy_from_slice(src);
    }
    data
}

fn staging_desc(w: u32, h: u32) -> D3D11_TEXTURE2D_DESC {
    D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    }
}

/// Small copies read back without waiting: each is copied into one of three textures the
/// processor can read, and read the next time round, once the card has done it.
#[derive(Default)]
struct Readback {
    staging: Vec<ID3D11Texture2D>,
    /// Which of them have a copy waiting to be read, oldest first.
    pending: VecDeque<usize>,
    width: u32,
    height: u32,
}

impl Readback {
    fn read(&mut self, t: &ID3D11Texture2D) -> Result<Option<RawFrame>> {
        let device = unsafe { t.GetDevice()? };
        let context = unsafe { device.GetImmediateContext()? };
        let d = desc(t);
        if d.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
            bail!("can't read back a {:?} texture", d.Format);
        }
        if (self.width, self.height) != (d.Width, d.Height) || self.staging.is_empty() {
            let sd = staging_desc(d.Width, d.Height);
            self.staging = (0..3).map(|_| texture2d(&device, &sd)).collect::<Result<_>>()?;
            self.pending.clear();
            (self.width, self.height) = (d.Width, d.Height);
        }
        let mut out = None;
        if let Some(&i) = self.pending.front() {
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            match unsafe { context.Map(&self.staging[i], 0, D3D11_MAP_READ, D3D11_MAP_FLAG_DO_NOT_WAIT.0 as u32, Some(&mut m)) } {
                Ok(()) => {
                    let data = unsafe { rows(&m, d.Width, d.Height) };
                    unsafe { context.Unmap(&self.staging[i], 0) };
                    self.pending.pop_front();
                    out = Some(RawFrame { width: d.Width, height: d.Height, format: PixelFormat::Bgra, data });
                }
                // (Not done yet: read next time.)
                Err(e) if e.code() == DXGI_ERROR_WAS_STILL_DRAWING => {}
                Err(e) => return Err(anyhow::Error::from(e).context("couldn't read the frame back")),
            }
        }
        if let Some(free) = (0..self.staging.len()).find(|i| !self.pending.contains(i)) {
            unsafe { context.CopyResource(&self.staging[free], t) };
            self.pending.push_back(free);
        }
        Ok(out)
    }
}

// --- The card's video engine --------------------------------------------------------------------

/// The card's video engine, scaling frames of one size to another and making them NV12 (for the
/// encoder) or BGRA (for the small copies), into textures of its own taken in turn.
struct Processor {
    outputs: Vec<(ID3D11Texture2D, ID3D11VideoProcessorOutputView)>,
    /// The views it reads the capture's textures through (the capture has three it takes in turn).
    inputs: Vec<(ID3D11Texture2D, ID3D11VideoProcessorInputView)>,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    in_w: u32,
    in_h: u32,
    out_w: u32,
    out_h: u32,
    next: usize,
}

impl Processor {
    fn new(device: &ID3D11Device, in_w: u32, in_h: u32, out_w: u32, out_h: u32, format: DXGI_FORMAT, count: usize) -> Result<Processor> {
        unsafe {
            let video_device: ID3D11VideoDevice = device.cast().context("the graphics device has no video engine")?;
            let context = device.GetImmediateContext()?;
            let video_context: ID3D11VideoContext = context.cast().context("the graphics device has no video context")?;
            let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
                InputWidth: in_w,
                InputHeight: in_h,
                OutputFrameRate: DXGI_RATIONAL { Numerator: 60, Denominator: 1 },
                OutputWidth: out_w,
                OutputHeight: out_h,
                Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
            };
            let enumerator = video_device.CreateVideoProcessorEnumerator(&content).context("the video engine can't take this size")?;
            let processor = video_device.CreateVideoProcessor(&enumerator, 0).context("couldn't start the video engine")?;
            video_context.VideoProcessorSetStreamFrameFormat(&processor, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
            // The capture: RGB, 0 to 255. NV12 out: 16 to 235 (BT.601, as the encoder's told).
            video_context.VideoProcessorSetStreamColorSpace(&processor, 0, &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 });
            let range = if format == DXGI_FORMAT_NV12 { 1 << 4 } else { 0 };
            video_context.VideoProcessorSetOutputColorSpace(&processor, &D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: range });
            video_context.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
            let src = windows::Win32::Foundation::RECT { left: 0, top: 0, right: in_w as i32, bottom: in_h as i32 };
            let dst = windows::Win32::Foundation::RECT { left: 0, top: 0, right: out_w as i32, bottom: out_h as i32 };
            video_context.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&src));
            video_context.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&dst));
            video_context.VideoProcessorSetOutputTargetRect(&processor, true, Some(&dst));
            let mut p = Processor { outputs: Vec::new(), inputs: Vec::new(), video_device, video_context, enumerator, processor, in_w, in_h, out_w, out_h, next: 0 };
            for _ in 0..count.max(1) {
                let mut td = D3D11_TEXTURE2D_DESC {
                    Width: out_w,
                    Height: out_h,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: format,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                    CPUAccessFlags: 0,
                    MiscFlags: 0,
                };
                // (NV12 the encoder reads straight off the card, where the card allows it.)
                let t = if format == DXGI_FORMAT_NV12 {
                    td.BindFlags = (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_VIDEO_ENCODER.0) as u32;
                    match texture2d(device, &td) {
                        Ok(t) => t,
                        Err(_) => {
                            td.BindFlags = D3D11_BIND_RENDER_TARGET.0 as u32;
                            texture2d(device, &td)?
                        }
                    }
                } else {
                    texture2d(device, &td)?
                };
                let vd = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                    ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                    Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 } },
                };
                let mut view = None;
                p.video_device.CreateVideoProcessorOutputView(&t, &p.enumerator, &vd, Some(&mut view)).context("the video engine can't write this format")?;
                p.outputs.push((t, view.ok_or_else(|| anyhow!("no output view"))?));
            }
            Ok(p)
        }
    }

    /// `frame` scaled and converted into the next of its textures.
    fn convert(&mut self, frame: &CardFrame) -> Result<ID3D11Texture2D> {
        unsafe {
            let src = &frame.texture.0;
            let view = match self.inputs.iter().find(|(t, _)| t == src) {
                Some((_, v)) => v.clone(),
                None => {
                    let vd = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                        FourCC: 0,
                        ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                        Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPIV { MipSlice: 0, ArraySlice: 0 } },
                    };
                    let mut view = None;
                    self.video_device.CreateVideoProcessorInputView(src, &self.enumerator, &vd, Some(&mut view)).context("the video engine can't read the capture")?;
                    let view = view.ok_or_else(|| anyhow!("no input view"))?;
                    if self.inputs.len() > 5 {
                        self.inputs.remove(0);
                    }
                    self.inputs.push((src.clone(), view.clone()));
                    view
                }
            };
            let (out, out_view) = self.outputs[self.next].clone();
            self.next = (self.next + 1) % self.outputs.len();
            let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                OutputIndex: 0,
                InputFrameOrField: 0,
                PastFrames: 0,
                FutureFrames: 0,
                ppPastSurfaces: std::ptr::null_mut(),
                pInputSurface: ManuallyDrop::new(Some(view)),
                ppFutureSurfaces: std::ptr::null_mut(),
                ppPastSurfacesRight: std::ptr::null_mut(),
                pInputSurfaceRight: ManuallyDrop::new(None),
                ppFutureSurfacesRight: std::ptr::null_mut(),
            };
            let r = self.video_context.VideoProcessorBlt(&self.processor, &out_view, 0, std::slice::from_ref(&stream));
            ManuallyDrop::drop(&mut stream.pInputSurface);
            ManuallyDrop::drop(&mut stream.pInputSurfaceRight);
            r.context("the video engine couldn't convert the frame")?;
            Ok(out)
        }
    }
}

// --- Media Foundation's hardware encoder --------------------------------------------------------

thread_local! {
    /// Media Foundation started on this thread.
    static MF_STARTED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// An H.264 encoder on the graphics card, for one size and frame rate.
struct Encoder {
    /// A keyframe asked for: the count of frames handed in then (it's to come within six).
    asked: Option<u64>,
    /// The stream's SPS and PPS (Annex B), put before a keyframe that comes without them.
    header: Vec<u8>,
    name: String,
    /// What it made, not yet given out.
    packets: Vec<Packet>,
    start: Instant,
    transform: IMFTransform,
    activate: IMFActivate,
    _manager: IMFDXGIDeviceManager,
    /// For an asynchronous encoder: it says when it wants a frame and when one's made.
    events: Option<IMFMediaEventGenerator>,
    codec: Option<ICodecAPI>,
    /// Frames handed in, and frames passed over because it didn't want one yet.
    frames: u64,
    skipped: u64,
    in_id: u32,
    out_id: u32,
    /// Frames it asked for and hasn't been given (an asynchronous encoder; all of them else).
    inputs_wanted: u32,
    /// The size of the buffer it writes into, for an encoder that doesn't bring its own.
    out_size: u32,
    width: u32,
    height: u32,
    fps: u32,
    bitrate: u32,
    provides_samples: bool,
    can_ask_keyframe: bool,
    vendor: u8,
}

/// Sets one of ICodecAPI's settings (an encoder takes the ones it knows).
unsafe fn set_value(api: &ICodecAPI, key: &GUID, mut v: VARIANT) -> windows::core::Result<()> {
    let r = unsafe { api.SetValue(key, &v) };
    let _ = unsafe { VariantClear(&mut v) };
    r
}

fn var_u32(x: u32) -> VARIANT {
    let mut v = VARIANT::default();
    unsafe {
        let a = &mut *v.Anonymous.Anonymous;
        a.vt = VT_UI4;
        a.Anonymous.ulVal = x;
    }
    v
}

fn var_true() -> VARIANT {
    let mut v = VARIANT::default();
    unsafe {
        let a = &mut *v.Anonymous.Anonymous;
        a.vt = VT_BOOL;
        a.Anonymous.boolVal = windows::Win32::Foundation::VARIANT_TRUE;
    }
    v
}

impl Encoder {
    /// The first of the card's hardware H.264 encoders that starts for `w`x`h` at `fps` and
    /// `bitrate`, reading NV12 textures off the capture's `device`.
    fn new(device: &ID3D11Device, w: u32, h: u32, fps: u32, bitrate: u32) -> Result<Encoder> {
        if !MF_STARTED.with(|s| s.get()) {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                MFStartup(MF_VERSION, MFSTARTUP_FULL).context("Media Foundation didn't start")?;
            }
            MF_STARTED.with(|s| s.set(true));
        }
        let (vendor_id, luid) = unsafe {
            let dxgi: IDXGIDevice = device.cast()?;
            let d = dxgi.GetAdapter()?.GetDesc()?;
            (d.VendorId, d.AdapterLuid)
        };
        // The encoders of the card the capture is on, or all of them where Windows can't say.
        let list = encoders(Some(luid)).or_else(|_| encoders(None))?;
        if list.is_empty() {
            bail!("no hardware H.264 encoder");
        }
        let mut errors = Vec::new();
        for a in list {
            match Encoder::start(&a, device, w, h, fps, bitrate, vendor_id) {
                Ok(e) => return Ok(e),
                Err(e) => {
                    unsafe {
                        let _ = a.ShutdownObject();
                    }
                    errors.push(format!("{e:#}"));
                }
            }
        }
        bail!("no hardware encoder would start: {}", errors.join("; "))
    }

    #[allow(clippy::too_many_arguments)]
    fn start(activate: &IMFActivate, device: &ID3D11Device, w: u32, h: u32, fps: u32, bitrate: u32, vendor_id: u32) -> Result<Encoder> {
        unsafe {
            let name = friendly_name(activate).unwrap_or_else(|| "hardware encoder".to_string());
            let transform: IMFTransform = activate
                .ActivateObject()
                .map_err(|e| windows::core::Error::new(e.code(), format!("{name} wouldn't start")))?;
            let mut is_async = false;
            if let Ok(attrs) = transform.GetAttributes() {
                if attrs.GetUINT32(&MF_TRANSFORM_ASYNC).is_ok_and(|v| v != 0) {
                    attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1).context("couldn't unlock the asynchronous encoder")?;
                    is_async = true;
                }
                let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
            }
            // It reads the frames straight off the capture's graphics device.
            let mut token = 0u32;
            let mut manager = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager).context("no device manager")?;
            let manager = manager.ok_or_else(|| anyhow!("no device manager"))?;
            manager.ResetDevice(device, token).context("the device manager wouldn't take the device")?;
            transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)?;
            let codec: Option<ICodecAPI> = transform.cast().ok();
            let (mut in_id, mut out_id) = ([0u32], [0u32]);
            if transform.GetStreamIDs(&mut in_id, &mut out_id).is_err() {
                (in_id, out_id) = ([0], [0]);
            }
            let (in_id, out_id) = (in_id[0], out_id[0]);
            let fps1 = fps.max(1);
            if let Some(c) = &codec {
                // Constant bitrate, low latency, no B-frames, a keyframe every minute by itself.
                let _ = set_value(c, &CODECAPI_AVEncCommonRateControlMode, var_u32(eAVEncCommonRateControlMode_CBR.0 as u32));
                let _ = set_value(c, &CODECAPI_AVEncCommonMeanBitRate, var_u32(bitrate));
                let _ = set_value(c, &CODECAPI_AVLowLatencyMode, var_true());
                let _ = set_value(c, &CODECAPI_AVEncMPVDefaultBPictureCount, var_u32(0));
                let _ = set_value(c, &CODECAPI_AVEncMPVGOPSize, var_u32(fps1 * 60));
            }
            let out = MFCreateMediaType()?;
            out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
            out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
            out.SetUINT64(&MF_MT_FRAME_SIZE, ((w as u64) << 32) | h as u64)?;
            out.SetUINT64(&MF_MT_FRAME_RATE, ((fps1 as u64) << 32) | 1)?;
            out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
            out.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
            out.SetUINT32(&MF_MT_VIDEO_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
            let _ = out.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT601.0 as u32);
            let _ = out.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32);
            transform
                .SetOutputType(out_id, &out, 0)
                .map_err(|e| windows::core::Error::new(e.code(), format!("{name} won't encode {w}x{h} at {fps} fps")))?;
            let input = nv12_type(&transform, in_id, w, h, fps)?;
            transform
                .SetInputType(in_id, &input, 0)
                .map_err(|e| windows::core::Error::new(e.code(), format!("{name} won't take NV12 frames")))?;
            let events = if is_async { Some(transform.cast::<IMFMediaEventGenerator>().context("the asynchronous encoder has no events")?) } else { None };
            transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            let info = transform.GetOutputStreamInfo(out_id)?;
            let provides = (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32;
            let header = sequence_header(&transform, out_id);
            Ok(Encoder {
                asked: None,
                header,
                vendor: vendor(&name, vendor_id),
                name,
                packets: Vec::new(),
                start: Instant::now(),
                transform,
                activate: activate.clone(),
                _manager: manager,
                events,
                codec,
                frames: 0,
                skipped: 0,
                in_id,
                out_id,
                inputs_wanted: if is_async { 0 } else { u32::MAX },
                out_size: (w * h * 3 / 2).max(info.cbSize),
                width: w,
                height: h,
                fps: fps1,
                bitrate,
                provides_samples: info.dwFlags & provides != 0,
                can_ask_keyframe: true,
            })
        }
    }

    /// A new bitrate, unless it's within 4% of the one set (a hundred kilobits at least).
    fn set_bitrate(&mut self, bps: u32) {
        let bps = bps.max(100_000);
        if (bps as f64 / self.bitrate.max(1) as f64 - 1.0).abs() < 0.04 {
            return;
        }
        self.bitrate = bps;
        if let Some(c) = &self.codec {
            if let Err(e) = unsafe { set_value(c, &CODECAPI_AVEncCommonMeanBitRate, var_u32(bps)) } {
                debug!("video: {} wouldn't change its bitrate: {e}", self.name);
            }
        }
    }

    /// Hands it the NV12 `texture` (a keyframe if `keyframe`), and gives what it's made.
    fn encode(&mut self, texture: &ID3D11Texture2D, keyframe: bool) -> Result<Vec<Packet>> {
        self.pump(Duration::ZERO, false)?;
        if self.inputs_wanted == 0 {
            self.pump(Duration::from_millis(8), true)?;
            if self.inputs_wanted == 0 {
                // (Not ready for another frame yet: this one's passed over.)
                self.skipped += 1;
                return Ok(std::mem::take(&mut self.packets));
            }
        }
        if keyframe {
            let asked = self.codec.as_ref().is_some_and(|c| unsafe { set_value(c, &CODECAPI_AVEncVideoForceKeyFrame, var_u32(1)) }.is_ok());
            if !asked {
                debug!("video: {} won't be asked for a keyframe; it's made anew for each", self.name);
                self.can_ask_keyframe = false;
            }
            if self.asked.is_none() {
                self.asked = Some(self.frames);
            }
        }
        unsafe {
            let buffer = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, texture, 0, false).context("couldn't hand the frame to the encoder")?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            sample.SetSampleTime((self.start.elapsed().as_nanos() / 100) as i64)?;
            sample.SetSampleDuration(10_000_000 / self.fps.max(1) as i64)?;
            match self.transform.ProcessInput(self.in_id, &sample, 0) {
                Ok(()) => {}
                // (A synchronous encoder with what it made still to give: given, then again.)
                Err(e) if e.code() == MF_E_NOTACCEPTING && self.events.is_none() => {
                    while self.output()? {}
                    self.transform.ProcessInput(self.in_id, &sample, 0).context("the encoder wouldn't take the frame")?;
                }
                Err(e) => return Err(anyhow::Error::from(e).context("the encoder wouldn't take the frame")),
            }
        }
        self.frames += 1;
        if self.asked.is_some_and(|n| self.frames > n + 6) {
            debug!("video: {} didn't make the keyframe asked for; it's made anew for each", self.name);
            self.can_ask_keyframe = false;
            self.asked = None;
        }
        if self.events.is_some() {
            self.inputs_wanted = self.inputs_wanted.saturating_sub(1);
            self.pump(Duration::from_millis(4), false)?;
        } else {
            while self.output()? {}
        }
        Ok(std::mem::take(&mut self.packets))
    }

    /// An asynchronous encoder's news, until it wants a frame (`want_input`) or has made one, or
    /// `wait` is up.
    fn pump(&mut self, wait: Duration, want_input: bool) -> Result<()> {
        let Some(events) = self.events.clone() else { return Ok(()) };
        let until = Instant::now() + wait;
        let had = self.packets.len();
        loop {
            match unsafe { events.GetEvent(MF_EVENT_FLAG_NO_WAIT) } {
                Ok(ev) => match unsafe { ev.GetType() } {
                    Ok(t) if t == METransformNeedInput.0 as u32 => self.inputs_wanted = self.inputs_wanted.wrapping_add(1),
                    Ok(t) if t == METransformHaveOutput.0 as u32 => {
                        self.output()?;
                    }
                    _ => {}
                },
                Err(e) if e.code() == MF_E_NO_EVENTS_AVAILABLE => {
                    let done = if want_input { self.inputs_wanted != 0 } else { self.packets.len() > had };
                    if done || Instant::now() >= until {
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(e) => return Err(anyhow::Error::from(e).context("the encoder stopped working")),
            }
        }
    }

    /// Takes one thing it's made (`true`: worth asking again).
    fn output(&mut self) -> Result<bool> {
        unsafe {
            let sample = if self.provides_samples {
                None
            } else {
                let s = MFCreateSample()?;
                s.AddBuffer(&MFCreateMemoryBuffer(self.out_size)?)?;
                Some(s)
            };
            let mut out = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: self.out_id, pSample: ManuallyDrop::new(sample), dwStatus: 0, pEvents: ManuallyDrop::new(None) }];
            let mut status = 0u32;
            let r = self.transform.ProcessOutput(0, &mut out, &mut status);
            let sample = ManuallyDrop::take(&mut out[0].pSample);
            drop(ManuallyDrop::take(&mut out[0].pEvents));
            match r {
                Ok(()) => {}
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(false),
                // (It changed what it makes: what it offers now is taken.)
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                    let t = self.transform.GetOutputAvailableType(self.out_id, 0)?;
                    self.transform.SetOutputType(self.out_id, &t, 0)?;
                    self.header = sequence_header(&self.transform, self.out_id);
                    return Ok(true);
                }
                Err(e) => return Err(anyhow::Error::from(e).context("the encoder failed")),
            }
            let Some(sample) = sample else { return Ok(false) };
            let buffer = sample.ConvertToContiguousBuffer()?;
            let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
            buffer.Lock(&mut p, None, Some(&mut len))?;
            let bytes = if p.is_null() { Vec::new() } else { std::slice::from_raw_parts(p, len as usize).to_vec() };
            let _ = buffer.Unlock();
            if bytes.is_empty() {
                return Ok(true);
            }
            let mut data = annex_b(bytes);
            let types = nal_types(&data);
            let key = sample.GetUINT32(&MFSampleExtension_CleanPoint).is_ok_and(|v| v != 0) || types.contains(&5);
            if key {
                self.asked = None;
            }
            // (The parameter sets the stream brings, kept; a keyframe without them gets them.)
            if types.contains(&7) && types.contains(&8) {
                let h = parameter_sets(&data);
                if !h.is_empty() {
                    self.header = h;
                }
            }
            if key && !types.contains(&7) {
                if self.header.is_empty() {
                    self.header = sequence_header(&self.transform, self.out_id);
                }
                if !self.header.is_empty() {
                    let mut with = self.header.clone();
                    with.extend_from_slice(&data);
                    data = with;
                }
            }
            self.packets.push(Packet { data, key });
            Ok(true)
        }
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
            let _ = self.activate.ShutdownObject();
        }
    }
}

/// The card's hardware H.264 encoders (`luid`: on that card only).
fn encoders(luid: Option<LUID>) -> Result<Vec<IMFActivate>> {
    unsafe {
        let input = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_NV12 };
        let output = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: MFVideoFormat_H264 };
        let flags = MFT_ENUM_FLAG(MFT_ENUM_FLAG_HARDWARE.0 | MFT_ENUM_FLAG_SORTANDFILTER.0);
        let mut list: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0u32;
        match luid {
            Some(l) => {
                let mut attrs = None;
                MFCreateAttributes(&mut attrs, 1)?;
                let attrs = attrs.ok_or_else(|| anyhow!("no attributes"))?;
                let mut blob = [0u8; 8];
                blob[..4].copy_from_slice(&l.LowPart.to_le_bytes());
                blob[4..].copy_from_slice(&l.HighPart.to_le_bytes());
                attrs.SetBlob(&MFT_ENUM_ADAPTER_LUID, &blob)?;
                MFTEnum2(MFT_CATEGORY_VIDEO_ENCODER, flags, Some(&input), Some(&output), &attrs, &mut list, &mut count)?;
            }
            None => MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER, flags, Some(&input), Some(&output), &mut list, &mut count)?,
        }
        let mut out = Vec::new();
        if !list.is_null() {
            for i in 0..count as usize {
                if let Some(a) = std::ptr::read(list.add(i)) {
                    out.push(a);
                }
            }
            CoTaskMemFree(Some(list as *const _));
        }
        Ok(out)
    }
}

fn friendly_name(a: &IMFActivate) -> Option<String> {
    unsafe {
        let mut p = PWSTR::null();
        let mut len = 0u32;
        a.GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut p, &mut len).ok()?;
        if p.is_null() {
            return None;
        }
        let name = String::from_utf16(std::slice::from_raw_parts(p.0, len as usize)).ok();
        CoTaskMemFree(Some(p.0 as *const _));
        name
    }
}

/// The encoder's maker: by its name, else by the card's.
fn vendor(name: &str, vendor_id: u32) -> u8 {
    let n = name.to_lowercase();
    if n.contains("nvidia") || n.contains("nvenc") {
        VENDOR_NVIDIA
    } else if n.contains("amd") || n.contains("radeon") {
        VENDOR_AMD
    } else if n.contains("intel") || n.contains("quick sync") {
        VENDOR_INTEL
    } else {
        match vendor_id {
            0x1002 | 0x1022 => VENDOR_AMD,
            0x10de => VENDOR_NVIDIA,
            0x8086 => VENDOR_INTEL,
            _ => VENDOR_OTHER,
        }
    }
}

/// NV12 frames of `w`x`h` at `fps`: the encoder's own NV12 type when it offers one.
fn nv12_type(t: &IMFTransform, in_id: u32, w: u32, h: u32, fps: u32) -> Result<IMFMediaType> {
    unsafe {
        let mut i = 0;
        let ty = loop {
            match t.GetInputAvailableType(in_id, i) {
                Ok(ty) if ty.GetGUID(&MF_MT_SUBTYPE).ok() == Some(MFVideoFormat_NV12) => break ty,
                Ok(_) => i += 1,
                Err(_) => {
                    let ty = MFCreateMediaType()?;
                    ty.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
                    ty.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
                    break ty;
                }
            }
        };
        ty.SetUINT64(&MF_MT_FRAME_SIZE, ((w as u64) << 32) | h as u64)?;
        ty.SetUINT64(&MF_MT_FRAME_RATE, ((fps.max(1) as u64) << 32) | 1)?;
        ty.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        ty.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1)?;
        Ok(ty)
    }
}

/// The SPS and PPS the encoder's output type carries (only if they're in Annex B).
fn sequence_header(t: &IMFTransform, out_id: u32) -> Vec<u8> {
    unsafe {
        let Ok(ty) = t.GetOutputCurrentType(out_id) else { return Vec::new() };
        let Ok(n) = ty.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER) else { return Vec::new() };
        let mut v = vec![0u8; n as usize];
        if ty.GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut v, None).is_err() {
            return Vec::new();
        }
        if v.starts_with(&[0, 0, 0, 1]) || v.starts_with(&[0, 0, 1]) { v } else { Vec::new() }
    }
}

// --- Textures and H.264 bytes -------------------------------------------------------------------

fn desc(t: &ID3D11Texture2D) -> D3D11_TEXTURE2D_DESC {
    let mut d = D3D11_TEXTURE2D_DESC::default();
    unsafe { t.GetDesc(&mut d) };
    d
}

fn texture2d(device: &ID3D11Device, d: &D3D11_TEXTURE2D_DESC) -> Result<ID3D11Texture2D> {
    let mut t = None;
    unsafe { device.CreateTexture2D(d, None, Some(&mut t)) }
        .map_err(|e| windows::core::Error::new(e.code(), format!("couldn't make a {}x{} texture ({:?})", d.Width, d.Height, d.Format)))?;
    t.ok_or_else(|| anyhow!("no texture made"))
}

/// Annex B (each NAL unit after a start code), from what the encoder made: as it is if it's
/// that already, else read as NAL units after four-byte lengths.
fn annex_b(raw: Vec<u8>) -> Vec<u8> {
    if raw.starts_with(&[0, 0, 0, 1]) || raw.starts_with(&[0, 0, 1]) {
        return raw;
    }
    let mut out = Vec::with_capacity(raw.len() + 16);
    let mut i = 0;
    while i + 4 <= raw.len() {
        let n = u32::from_be_bytes([raw[i], raw[i + 1], raw[i + 2], raw[i + 3]]) as usize;
        if n == 0 || i + 4 + n > raw.len() {
            break;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&raw[i + 4..i + 4 + n]);
        i += 4 + n;
    }
    if out.is_empty() { raw } else { out }
}

/// The type of each NAL unit (after each `00 00 01`).
fn nal_types(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            out.push(data[i + 3] & 0x1f);
            i += 3;
        } else {
            i += 1;
        }
    }
    out
}

/// The SPS and PPS NAL units in `data`, each after a four-byte start code.
fn parameter_sets(data: &[u8]) -> Vec<u8> {
    // Where each NAL unit starts (just after its `00 00 01`).
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    for (k, &s) in starts.iter().enumerate() {
        // It ends where the next start code begins (the zeros before it aren't part of it).
        let end = match starts.get(k + 1) {
            Some(&next) => {
                if next < s + 4 {
                    continue;
                }
                let mut j = next - 4;
                loop {
                    if data[j] != 0 {
                        break Some(j + 1);
                    }
                    if j > s {
                        j -= 1;
                    } else {
                        break None;
                    }
                }
            }
            None => (data.len() > s).then_some(data.len()),
        };
        let Some(end) = end else { continue };
        if matches!(data[s] & 0x1f, 7 | 8) {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(&data[s..end]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_become_start_codes() {
        let raw = vec![0, 0, 0, 3, 0x67, 1, 2, 0, 0, 0, 2, 0x65, 9];
        assert_eq!(annex_b(raw), vec![0, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x65, 9]);
    }

    #[test]
    fn annex_b_stays_as_it_is() {
        let raw = vec![0, 0, 0, 1, 0x65, 42];
        assert_eq!(annex_b(raw.clone()), raw);
    }

    #[test]
    fn nal_types_and_parameter_sets() {
        let d = [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x68, 3, 0, 0, 1, 0x65, 9, 9];
        assert_eq!(nal_types(&d), vec![7, 8, 5]);
        assert_eq!(parameter_sets(&d), vec![0, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x68, 3]);
    }

    #[test]
    fn makers_by_name_then_card() {
        assert_eq!(vendor("NVIDIA H.264 Encoder MFT", 0), VENDOR_NVIDIA);
        assert_eq!(vendor("AMDh264Encoder", 0), VENDOR_AMD);
        assert_eq!(vendor("Intel® Quick Sync Video H.264 Encoder MFT", 0), VENDOR_INTEL);
        assert_eq!(vendor("H.264 Encoder", 0x10de), VENDOR_NVIDIA);
        assert_eq!(vendor("H.264 Encoder", 0x1234), VENDOR_OTHER);
    }
}
