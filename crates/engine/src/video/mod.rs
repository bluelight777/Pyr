//! Video: capture → scale → H.264 encode on the way out; reorder → decode → texture on the way in.

pub mod capture;
pub mod codec;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tracing::{debug, warn};

pub use capture::{CaptureHandle, FrameSlot, ScreenSource, SourceKind, list_cameras, list_screens};
use codec::{EncodedFrame, H264Decoder, H264Encoder, Scaler, fit_size, preview_rgba, to_i420};

use crate::state::{SharedFrame, VideoFrame, VideoKey};

/// Latest decoded frame per video tile, read by the UI.
pub struct VideoStore {
    frames: Mutex<HashMap<VideoKey, SharedFrame>>,
    generation: AtomicU64,
    repaint: Arc<dyn Fn() + Send + Sync>,
}

impl VideoStore {
    pub fn new(repaint: Arc<dyn Fn() + Send + Sync>) -> VideoStore {
        VideoStore { frames: Mutex::new(HashMap::new()), generation: AtomicU64::new(1), repaint }
    }

    pub fn put(&self, key: VideoKey, width: u32, height: u32, rgba: Vec<u8>) {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed);
        self.frames.lock().insert(key, Arc::new(VideoFrame { width, height, rgba, generation }));
        (self.repaint)();
    }

    pub fn get(&self, key: VideoKey) -> Option<SharedFrame> {
        self.frames.lock().get(&key).cloned()
    }

    pub fn remove(&self, key: VideoKey) {
        self.frames.lock().remove(&key);
        (self.repaint)();
    }

    pub fn remove_session(&self, session: u32) {
        self.frames.lock().retain(|k, _| k.session != session);
    }

    pub fn clear(&self) {
        self.frames.lock().clear();
    }
}

pub struct PublisherConfig {
    pub screen: bool,
    pub max_width: u32,
    pub max_height: u32,
    pub fps: u32,
    pub bitrate: u32,
}

pub type EncodedSink = Box<dyn FnMut(EncodedFrame, u32, u32) + Send>;
pub type PreviewSink = Box<dyn FnMut(u32, u32, Vec<u8>) + Send>;

#[derive(Default)]
pub struct PublisherStats {
    pub frames: AtomicU64,
    pub bytes: AtomicU64,
    pub skipped: AtomicU64,
    pub width: AtomicU64,
    pub height: AtomicU64,
}

/// Encodes frames from a capture source and hands them to the network.
pub struct Publisher {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    _capture: CaptureHandle,
    pub keyframe: Arc<AtomicBool>,
    /// Frames handed to the network but not yet acknowledged by the server.
    pub inflight: Arc<AtomicUsize>,
    pub stats: Arc<PublisherStats>,
}

impl Publisher {
    pub fn start(
        cfg: PublisherConfig,
        capture: CaptureHandle,
        slot: Arc<FrameSlot>,
        mut sink: EncodedSink,
        mut preview: Option<PreviewSink>,
        inflight: Arc<AtomicUsize>,
    ) -> anyhow::Result<Publisher> {
        let stop = Arc::new(AtomicBool::new(false));
        let keyframe = Arc::new(AtomicBool::new(true));
        let stats = Arc::new(PublisherStats::default());
        let mut enc = H264Encoder::new(cfg.screen, cfg.bitrate, cfg.fps)?;
        let (st, kf, inf, sts) = (stop.clone(), keyframe.clone(), inflight.clone(), stats.clone());
        let thread = std::thread::Builder::new()
            .name(if cfg.screen { "ingle-screen-enc" } else { "ingle-camera-enc" }.into())
            .spawn(move || {
                let period = Duration::from_secs_f64(1.0 / cfg.fps.max(1) as f64);
                // Allow ~120 ms worth of frames to be in flight before we start skipping.
                let max_inflight = ((cfg.fps as usize * 12) / 100).max(2);
                let mut next_due = Instant::now();
                let mut last: Option<codec::RawFrame> = None;
                let mut last_encode = Instant::now();
                let mut scaler = Scaler::default();
                let mut preview_scaler = Scaler::default();
                let mut n: u64 = 0;
                while !st.load(Ordering::Relaxed) {
                    let fresh = slot.take_wait(Duration::from_millis(40));
                    let now = Instant::now();
                    let have_new = fresh.is_some();
                    if let Some(f) = fresh {
                        last = Some(f);
                    }
                    let Some(frame) = last.as_mut() else { continue };
                    let want_key = kf.load(Ordering::Relaxed);
                    // Re-send the last frame for keyframe requests and on static screens (≥2 fps).
                    let repeat = !have_new && (want_key || now.duration_since(last_encode) > Duration::from_millis(500));
                    if !have_new && !repeat {
                        continue;
                    }
                    if now < next_due && !want_key {
                        continue;
                    }
                    if inf.load(Ordering::Relaxed) >= max_inflight && !want_key {
                        sts.skipped.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    if kf.swap(false, Ordering::Relaxed) {
                        enc.request_keyframe();
                    }
                    let (tw, th) = fit_size(frame.width, frame.height, cfg.max_width, cfg.max_height);
                    let scaled = match scaler.scale(frame, tw, th) {
                        Ok(s) => s,
                        Err(e) => {
                            warn!("scaling failed: {e:#}");
                            continue;
                        }
                    };
                    let src = scaled.as_ref().unwrap_or(&*frame);
                    let yuv = match to_i420(src) {
                        Ok(y) => y,
                        Err(e) => {
                            warn!("{e:#}");
                            continue;
                        }
                    };
                    match enc.encode(&yuv) {
                        Ok(Some(e)) => {
                            sts.frames.fetch_add(1, Ordering::Relaxed);
                            sts.bytes.fetch_add(e.data.len() as u64, Ordering::Relaxed);
                            sts.width.store(tw as u64, Ordering::Relaxed);
                            sts.height.store(th as u64, Ordering::Relaxed);
                            sink(e, tw, th);
                        }
                        Ok(None) => {}
                        Err(e) => warn!("encode failed: {e:#}"),
                    }
                    last_encode = now;
                    next_due = (next_due + period).max(now);
                    n += 1;
                    if let Some(p) = preview.as_mut() {
                        let every = if cfg.screen { 4 } else { 2 };
                        if n % every == 0 {
                            if let Some((w, h, px)) = preview_rgba(&mut preview_scaler, frame, 960, 540) {
                                p(w, h, px);
                            }
                        }
                    }
                }
                debug!("encoder stopped after {n} frames");
            })?;
        Ok(Publisher { stop, thread: Some(thread), _capture: capture, keyframe, inflight, stats })
    }
}

impl Drop for Publisher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Incoming {
    seq: u32,
    keyframe: bool,
    data: Vec<u8>,
    at: Instant,
}

#[derive(Default)]
pub struct ReceiverStats {
    pub frames: AtomicU64,
    pub dropped: AtomicU64,
    pub keyframe_requests: AtomicU64,
}

/// Decodes one remote video track in order, asking for a keyframe after losses.
pub struct Receiver {
    tx: Option<crossbeam_channel::Sender<Incoming>>,
    thread: Option<JoinHandle<()>>,
    pub stats: Arc<ReceiverStats>,
}

const REORDER_WAIT: Duration = Duration::from_millis(120);

impl Receiver {
    pub fn start(key: VideoKey, store: Arc<VideoStore>, request_keyframe: Box<dyn Fn() + Send>) -> Receiver {
        let (tx, rx) = crossbeam_channel::unbounded::<Incoming>();
        let stats = Arc::new(ReceiverStats::default());
        let st = stats.clone();
        let thread = std::thread::Builder::new()
            .name("ingle-video-dec".into())
            .spawn(move || {
                let mut dec = match H264Decoder::new() {
                    Ok(d) => d,
                    Err(e) => {
                        warn!("video decoder unavailable: {e:#}");
                        return;
                    }
                };
                let mut pending: BTreeMap<u32, Incoming> = BTreeMap::new();
                let mut expected: Option<u32> = None;
                let mut need_key = true;
                let mut last_request = Instant::now() - Duration::from_secs(10);
                let ask = |st: &ReceiverStats, last_request: &mut Instant| {
                    if last_request.elapsed() > Duration::from_millis(500) {
                        *last_request = Instant::now();
                        st.keyframe_requests.fetch_add(1, Ordering::Relaxed);
                        request_keyframe();
                    }
                };
                loop {
                    match rx.recv_timeout(Duration::from_millis(30)) {
                        Ok(f) => {
                            if expected.map(|e| f.seq >= e).unwrap_or(true) {
                                pending.insert(f.seq, f);
                            }
                            while let Ok(f) = rx.try_recv() {
                                if expected.map(|e| f.seq >= e).unwrap_or(true) {
                                    pending.insert(f.seq, f);
                                }
                            }
                        }
                        Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                        Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
                    }
                    loop {
                        if need_key {
                            // Drop everything up to the newest keyframe we have.
                            let Some(k) = pending.iter().rev().find(|(_, f)| f.keyframe).map(|(s, _)| *s) else {
                                if pending.len() > 2 {
                                    let n = pending.len() as u64;
                                    pending.clear();
                                    st.dropped.fetch_add(n, Ordering::Relaxed);
                                }
                                if !pending.is_empty() || expected.is_none() {
                                    ask(&st, &mut last_request);
                                }
                                break;
                            };
                            let older: Vec<u32> = pending.range(..k).map(|(s, _)| *s).collect();
                            st.dropped.fetch_add(older.len() as u64, Ordering::Relaxed);
                            for s in older {
                                pending.remove(&s);
                            }
                            expected = Some(k);
                            need_key = false;
                        }
                        let Some(exp) = expected else { break };
                        if let Some(f) = pending.remove(&exp) {
                            match dec.decode(&f.data) {
                                Ok(Some(d)) => {
                                    st.frames.fetch_add(1, Ordering::Relaxed);
                                    store.put(key, d.width, d.height, d.rgba);
                                }
                                Ok(None) => {}
                                Err(e) => {
                                    debug!("decode error on frame {}: {e:#}", f.seq);
                                    need_key = true;
                                    ask(&st, &mut last_request);
                                }
                            }
                            expected = Some(exp + 1);
                            continue;
                        }
                        // The expected frame is missing. Wait briefly for it, then give up.
                        let Some((_, oldest)) = pending.iter().next() else { break };
                        if oldest.at.elapsed() > REORDER_WAIT || pending.len() > 8 {
                            st.dropped.fetch_add(1, Ordering::Relaxed);
                            need_key = true;
                            ask(&st, &mut last_request);
                            continue;
                        }
                        break;
                    }
                }
            })
            .expect("spawn decoder");
        Receiver { tx: Some(tx), thread: Some(thread), stats }
    }

    pub fn push(&self, seq: u32, keyframe: bool, data: Vec<u8>) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(Incoming { seq, keyframe, data, at: Instant::now() });
        }
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.tx.take();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
