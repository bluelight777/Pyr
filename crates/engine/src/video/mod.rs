//! Video: capture → scale → H.264 encode on the way out; reorder → decode → texture on the way in.

#[cfg(windows)]
pub mod border;
pub mod adapt;
pub mod capture;
pub mod codec;
pub mod testsrc;

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use tracing::{debug, warn};

pub use capture::{CaptureHandle, FrameSlot, ScreenSource, SourceKind, Trouble, list_cameras, list_screens};
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

#[derive(Clone, Copy, Debug)]
pub struct PublisherConfig {
    pub screen: bool,
    pub max_width: u32,
    pub max_height: u32,
    pub fps: u32,
    pub bitrate: u32,
}

pub type EncodedSink = Box<dyn FnMut(EncodedFrame, u32, u32) + Send>;
pub type PreviewSink = Box<dyn FnMut(u32, u32, Vec<u8>) + Send>;
/// Starts the same capture again (after it stopped working by itself).
pub type Restart = Box<dyn FnMut() -> anyhow::Result<CaptureHandle> + Send>;

/// What a share's capture did by itself (told to `Publisher::start`'s `on_event`).
#[derive(Clone, Debug, PartialEq)]
pub enum CaptureEvent {
    /// It stopped working and was started again: why it stopped.
    Restarted(String),
    /// It's over: what was being captured is gone (`gone`), or it stopped working and couldn't
    /// be started again.
    Ended { why: String, gone: bool },
}

/// A capture that stops working is started again this many times a minute at most.
const RESTARTS_PER_MINUTE: usize = 3;
/// When starting it again doesn't work (right after the graphics driver restarted, the screens
/// aren't back yet), it's tried every so often for a while before the share ends.
#[cfg(not(test))]
const RETRY_FOR: Duration = Duration::from_secs(30);
#[cfg(not(test))]
const RETRY_EVERY: Duration = Duration::from_secs(2);
#[cfg(test)]
const RETRY_FOR: Duration = Duration::from_millis(1500);
#[cfg(test)]
const RETRY_EVERY: Duration = Duration::from_millis(300);

#[derive(Default)]
pub struct PublisherStats {
    pub frames: AtomicU64,
    pub bytes: AtomicU64,
    pub skipped: AtomicU64,
    pub width: AtomicU64,
    pub height: AtomicU64,
    /// The frame rate and bitrate it's sending at now (see `adapt`), and whether it's taken
    /// for text (kept sharp) rather than motion (kept smooth).
    pub fps: AtomicU32,
    pub target_kbps: AtomicU32,
    pub text: AtomicBool,
    /// Sent smaller or at fewer frames a second than the preset, for want of bitrate.
    pub lowered: AtomicBool,
}

/// What the network side tells a publisher about how its frames are getting on (see
/// `call::VideoCtx::start` and `adapt::RateControl`).
#[derive(Default)]
pub struct SendLink {
    /// Frames handed to the network but not yet acknowledged by the server.
    pub inflight: AtomicUsize,
    /// The bitrate the network can take now, bit/s (0: not known yet).
    pub target_bps: AtomicU32,
    /// How late the oldest frame still on its way is, ms.
    pub late_ms: AtomicU32,
    /// Our round trip to the server, ms.
    pub rtt_ms: AtomicU32,
}

/// Keyframes asked for (by new viewers, or after losses) come at most this often: in between,
/// requests wait for the next (each is several frames' worth of data, and several viewers'
/// requests often come together).
const KEYFRAME_GAP: Duration = Duration::from_millis(400);

/// Encodes frames from a capture source and hands them to the network, and looks after the
/// capture (its thread owns it): one that stops working by itself (a game changing the display,
/// the graphics driver restarting) is started again with `restart`.
pub struct Publisher {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    pub keyframe: Arc<AtomicBool>,
    pub link: Arc<SendLink>,
    pub stats: Arc<PublisherStats>,
}

impl Publisher {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        cfg: PublisherConfig,
        capture: CaptureHandle,
        slot: Arc<FrameSlot>,
        mut sink: EncodedSink,
        mut preview: Option<PreviewSink>,
        link: Arc<SendLink>,
        mut restart: Option<Restart>,
        mut on_event: Box<dyn FnMut(CaptureEvent) + Send>,
    ) -> anyhow::Result<Publisher> {
        let stop = Arc::new(AtomicBool::new(false));
        let keyframe = Arc::new(AtomicBool::new(true));
        let stats = Arc::new(PublisherStats::default());
        let mut enc = H264Encoder::new(cfg.screen, cfg.bitrate, cfg.fps)?;
        let (st, kf, lk, sts) = (stop.clone(), keyframe.clone(), link.clone(), stats.clone());
        let thread = std::thread::Builder::new()
            .name(if cfg.screen { "ingle-screen-enc" } else { "ingle-camera-enc" }.into())
            .spawn(move || {
                // Size, frame rate and bitrate follow what the network can take (see `adapt`);
                // `INGLE_AB=adapt` keeps the preset's, `INGLE_AB=pace` the old limit on frames
                // in flight (~120 ms of them, whatever the round trip).
                let (adapting, old_pace) = (!ingle_proto::ab::old("adapt"), ingle_proto::ab::old("pace"));
                let mut period = Duration::from_secs_f64(1.0 / cfg.fps.max(1) as f64);
                let max_inflight = ((cfg.fps as usize * 12) / 100).max(2);
                let mut ladder = adapt::Ladder::new(cfg.max_height, cfg.fps, cfg.bitrate, Instant::now());
                let mut content = adapt::ContentWatch::new(Instant::now());
                let mut rung = adapt::Rung { height: cfg.max_height, fps: cfg.fps };
                let mut placed = false;
                let mut last_decide = Instant::now() - Duration::from_secs(1);
                // Whether OpenH264 skipped each of the last few frames it was given.
                let mut tries: std::collections::VecDeque<bool> = std::collections::VecDeque::new();
                let mut last_key: Option<Instant> = None;
                let mut next_due = Instant::now();
                let mut last: Option<codec::RawFrame> = None;
                let mut last_encode = Instant::now();
                let mut scaler = Scaler::default();
                let mut preview_scaler = Scaler::default();
                let mut n: u64 = 0;
                let mut capture = capture;
                let mut checked = Instant::now();
                let mut restarts: Vec<Instant> = Vec::new();
                let mut over = false;
                // Starting it again failed: why it broke, the last failure, until when it's tried
                // again, and when next.
                let mut retrying: Option<(String, String, Instant, Instant)> = None;
                while !st.load(Ordering::Relaxed) {
                    // The capture, looked at once a second.
                    if !over && checked.elapsed() >= Duration::from_secs(1).min(RETRY_EVERY) {
                        checked = Instant::now();
                        // Waiting for what's captured to come back (a driver restart): tried
                        // again every so often, then given up on.
                        if let Some((why, err, until, next)) = retrying.take() {
                            if Instant::now() < next {
                                retrying = Some((why, err, until, next));
                            } else {
                                match restart.as_mut().map(|f| f()) {
                                    Some(Ok(c)) => {
                                        warn!("the capture came back after {err}; started it again");
                                        capture = c;
                                        restarts.push(Instant::now());
                                        kf.store(true, Ordering::Relaxed);
                                        on_event(CaptureEvent::Restarted(format!("{why} (it came back after a moment: {err})")));
                                    }
                                    Some(Err(e)) if Instant::now() < until => retrying = Some((why, format!("{e:#}"), until, Instant::now() + RETRY_EVERY)),
                                    Some(Err(e)) => {
                                        over = true;
                                        on_event(CaptureEvent::Ended { why: format!("{why}, and starting it again didn't work: {e:#}"), gone: false });
                                    }
                                    None => {
                                        over = true;
                                        on_event(CaptureEvent::Ended { why, gone: false });
                                    }
                                }
                            }
                        } else {
                        match capture.trouble() {
                            None => {}
                            Some(Trouble::Gone(why)) => {
                                over = true;
                                on_event(CaptureEvent::Ended { why, gone: true });
                            }
                            Some(Trouble::Broken(why)) => {
                                restarts.retain(|t| t.elapsed() < Duration::from_secs(60));
                                let again = if restarts.len() < RESTARTS_PER_MINUTE { restart.as_mut().map(|f| f()) } else { None };
                                match again {
                                    Some(Ok(c)) => {
                                        warn!("the capture stopped working ({why}); started it again");
                                        capture = c;
                                        restarts.push(Instant::now());
                                        kf.store(true, Ordering::Relaxed);
                                        on_event(CaptureEvent::Restarted(why));
                                    }
                                    Some(Err(e)) => {
                                        // (Not back yet: the share holds its last picture meanwhile.)
                                        warn!("the capture stopped working ({why}) and didn't start again yet ({e:#}); trying again for a while");
                                        let now = Instant::now();
                                        retrying = Some((why, format!("{e:#}"), now + RETRY_FOR, now + RETRY_EVERY));
                                    }
                                    None => {
                                        over = true;
                                        on_event(CaptureEvent::Ended { why, gone: false });
                                    }
                                }
                            }
                        }
                        }
                    }
                    let fresh = slot.take_wait(Duration::from_millis(40));
                    let now = Instant::now();
                    let have_new = fresh.is_some();
                    if let Some(f) = fresh {
                        if adapting && cfg.screen {
                            content.feed(&f, now);
                        }
                        last = Some(f);
                    }
                    let Some(frame) = last.as_mut() else { continue };
                    if adapting && now.duration_since(last_decide) >= Duration::from_millis(100) {
                        last_decide = now;
                        let target = match lk.target_bps.load(Ordering::Relaxed) {
                            0 => cfg.bitrate,
                            t => t.min(cfg.bitrate),
                        };
                        let kind = if cfg.screen { content.kind() } else { adapt::Content::Motion };
                        // (OpenH264 skips a few frames after a keyframe to pay for it: only a
                        // lasting habit counts.)
                        let struggling = tries.len() >= 60 && tries.iter().filter(|s| **s).count() * 2 > tries.len();
                        let next = if placed { ladder.decide(target, kind, struggling, now) } else { ladder.place(target, kind, now) };
                        placed = true;
                        enc.set_motion(kind == adapt::Content::Motion);
                        if next != rung {
                            debug!(target: "ingle_media", ev = "rung", h = next.height, fps = next.fps, kbps = target / 1000, text = (kind == adapt::Content::Text), struggling);
                            rung = next;
                            period = Duration::from_secs_f64(1.0 / rung.fps.max(1) as f64);
                            tries.clear();
                        }
                        enc.set_rates(target, rung.fps as f32);
                        sts.fps.store(rung.fps, Ordering::Relaxed);
                        sts.target_kbps.store(target / 1000, Ordering::Relaxed);
                        sts.text.store(kind == adapt::Content::Text, Ordering::Relaxed);
                        // (A picture smaller than the preset's because the window shared is
                        // smaller isn't lowered; a text share keeping its size and frame rate on
                        // less bitrate isn't either: it looks as good.)
                        sts.lowered.store(rung != ladder.top(), Ordering::Relaxed);
                    }
                    // A keyframe that's been asked for, unless one went out a moment ago (then
                    // it waits for the next).
                    let want_key = kf.load(Ordering::Relaxed) && (old_pace || last_key.is_none_or(|t| now.duration_since(t) >= KEYFRAME_GAP));
                    // Re-send the last frame for keyframe requests and on static screens (≥2 fps).
                    let repeat = !have_new && (want_key || now.duration_since(last_encode) > Duration::from_millis(500));
                    if !have_new && !repeat {
                        continue;
                    }
                    if now < next_due && !want_key {
                        continue;
                    }
                    // Not while the network is behind with what it has already (skipping
                    // frames is better than piling them up: they'd all arrive late).
                    let behind = if old_pace {
                        lk.inflight.load(Ordering::Relaxed) >= max_inflight
                    } else {
                        let rtt = lk.rtt_ms.load(Ordering::Relaxed) as usize;
                        let most = (rung.fps as usize * (rtt + 1000) / 1000).max(4);
                        lk.late_ms.load(Ordering::Relaxed) > 300 || lk.inflight.load(Ordering::Relaxed) >= most
                    };
                    if behind && !want_key {
                        sts.skipped.fetch_add(1, Ordering::Relaxed);
                        continue;
                    }
                    if want_key {
                        kf.store(false, Ordering::Relaxed);
                        enc.request_keyframe();
                    }
                    let (max_w, max_h) = if adapting { (rung.height * 16 / 9 + 8, rung.height) } else { (cfg.max_width, cfg.max_height) };
                    let (tw, th) = fit_size(frame.width, frame.height, max_w, max_h);
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
                    let skipped_before = enc.skipped;
                    match enc.encode(&yuv) {
                        Ok(Some(e)) => {
                            sts.frames.fetch_add(1, Ordering::Relaxed);
                            sts.bytes.fetch_add(e.data.len() as u64, Ordering::Relaxed);
                            sts.width.store(tw as u64, Ordering::Relaxed);
                            sts.height.store(th as u64, Ordering::Relaxed);
                            if e.keyframe {
                                last_key = Some(now);
                            }
                            sink(e, tw, th);
                        }
                        Ok(None) => {}
                        Err(e) => warn!("encode failed: {e:#}"),
                    }
                    tries.push_back(enc.skipped > skipped_before);
                    if tries.len() > 90 {
                        tries.pop_front();
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
                // (The capture stops here, with the thread that looked after it.)
                drop(capture);
            })?;
        Ok(Publisher { stop, thread: Some(thread), keyframe, link, stats })
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

/// Before 0.8.0 (`INGLE_AB=recv`): how long a missing frame was waited for, and how many
/// frames after it.
const OLD_REORDER_WAIT: Duration = Duration::from_millis(120);
const OLD_REORDER_FRAMES: usize = 8;
/// A missing frame is still on its way (QUIC sends a lost packet again, a round trip later on
/// whichever leg lost it: the sharer's to the server, or the server's to us). It's waited for
/// about that long, at least this long,
const MIN_WAIT: Duration = Duration::from_millis(450);
/// and at most this long, before asking for a fresh keyframe instead.
const MAX_WAIT: Duration = Duration::from_millis(1200);
/// Frames kept while waiting, at most (a few seconds of a share).
const MAX_PENDING: usize = 180;
/// Frames waiting for the decoder, at most: more than that and the newest are dropped (the
/// decoder asks for a fresh keyframe), so nobody can pile up memory by sending faster than
/// pictures can be decoded.
const QUEUE: usize = 48;

/// How long to wait for a missing frame with `rtt` (our round trip to the server).
pub fn missing_frame_wait(rtt: Duration) -> Duration {
    (rtt * 3 + Duration::from_millis(150)).clamp(MIN_WAIT, MAX_WAIT)
}

impl Receiver {
    /// `rtt`: our round trip to the server now (how long a frame lost on the way can take to
    /// come again).
    pub fn start(key: VideoKey, store: Arc<VideoStore>, request_keyframe: Box<dyn Fn() + Send>, rtt: Box<dyn Fn() -> Duration + Send>) -> Receiver {
        let (tx, rx) = crossbeam_channel::bounded::<Incoming>(QUEUE);
        let stats = Arc::new(ReceiverStats::default());
        let st = stats.clone();
        let old = ingle_proto::ab::old("recv");
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
                let mut last_request: Option<Instant> = None;
                // Asks for a keyframe, not again until one has had time to come (a round trip to
                // the sharer and back, and the frame itself), unless one came meanwhile.
                let ask = |st: &ReceiverStats, last_request: &mut Option<Instant>| {
                    let again = if old { Duration::from_millis(500) } else { (rtt() * 2 + Duration::from_millis(250)).max(Duration::from_millis(600)) };
                    if last_request.is_none_or(|t| t.elapsed() > again) {
                        *last_request = Some(Instant::now());
                        st.keyframe_requests.fetch_add(1, Ordering::Relaxed);
                        debug!(target: "ingle_media", ev = "kfreq");
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
                                let too_many = if old { pending.len() > 2 } else { pending.len() > MAX_PENDING };
                                if too_many {
                                    let n = pending.len() as u64;
                                    pending.clear();
                                    st.dropped.fetch_add(n, Ordering::Relaxed);
                                    debug!(target: "ingle_media", ev = "drop", n, why = "no_key");
                                }
                                if !pending.is_empty() || expected.is_none() {
                                    ask(&st, &mut last_request);
                                }
                                break;
                            };
                            let older: Vec<u32> = pending.range(..k).map(|(s, _)| *s).collect();
                            st.dropped.fetch_add(older.len() as u64, Ordering::Relaxed);
                            if !older.is_empty() {
                                debug!(target: "ingle_media", ev = "drop", n = older.len(), why = "before_key");
                            }
                            for s in older {
                                pending.remove(&s);
                            }
                            expected = Some(k);
                            need_key = false;
                            if !old {
                                last_request = None;
                            }
                        }
                        let Some(exp) = expected else { break };
                        if let Some(f) = pending.remove(&exp) {
                            debug!(target: "ingle_media", ev = "dec", seq = f.seq, key = f.keyframe, waited = f.at.elapsed().as_millis() as u64);
                            if f.keyframe && !old {
                                last_request = None;
                            }
                            // Catching up (the next one's here already): only the newest
                            // picture is worth showing.
                            let show = old || !pending.contains_key(&(exp + 1));
                            match dec.decode(&f.data, show) {
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
                        // The expected frame is missing.
                        let Some((_, oldest)) = pending.iter().next() else { break };
                        if !old {
                            // A keyframe after it is here already: nothing before that matters.
                            if let Some(k) = pending.iter().find(|(_, f)| f.keyframe).map(|(s, _)| *s) {
                                debug!(target: "ingle_media", ev = "drop", n = k - exp, why = "skip_to_key");
                                st.dropped.fetch_add((k - exp) as u64, Ordering::Relaxed);
                                for s in pending.range(..k).map(|(s, _)| *s).collect::<Vec<_>>() {
                                    pending.remove(&s);
                                }
                                expected = Some(k);
                                continue;
                            }
                        }
                        // Wait for it a while (it's being sent again), then give up on it.
                        let late = if old {
                            oldest.at.elapsed() > OLD_REORDER_WAIT || pending.len() > OLD_REORDER_FRAMES
                        } else {
                            oldest.at.elapsed() > missing_frame_wait(rtt()) || pending.len() > MAX_PENDING
                        };
                        if late {
                            debug!(target: "ingle_media", ev = "drop", n = 1, why = "gave_up", missing = exp, waiting = pending.len());
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
        debug!(target: "ingle_media", ev = "rx", seq, key = keyframe, bytes = data.len());
        if let Some(tx) = &self.tx {
            if let Err(crossbeam_channel::TrySendError::Full(_)) = tx.try_send(Incoming { seq, keyframe, data, at: Instant::now() }) {
                self.stats.dropped.fetch_add(1, Ordering::Relaxed);
                debug!(target: "ingle_media", ev = "drop", n = 1, why = "queue_full");
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A capture that has already stopped working (its thread is done).
    fn broken() -> CaptureHandle {
        CaptureHandle::thread(Arc::new(AtomicBool::new(false)), std::thread::spawn(|| {}))
    }

    /// A capture that keeps going until it's stopped.
    fn working() -> CaptureHandle {
        let stop = Arc::new(AtomicBool::new(false));
        let s = stop.clone();
        CaptureHandle::thread(
            stop,
            std::thread::spawn(move || {
                while !s.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }),
        )
    }

    #[test]
    fn a_capture_that_stops_working_is_started_again_a_few_times_then_given_up_on() {
        let cfg = PublisherConfig { screen: true, max_width: 320, max_height: 180, fps: 10, bitrate: 200_000 };
        let events = Arc::new(Mutex::new(Vec::new()));
        let starts = Arc::new(AtomicUsize::new(0));
        let (ev, st) = (events.clone(), starts.clone());
        let publisher = Publisher::start(
            cfg,
            broken(),
            Arc::new(FrameSlot::default()),
            Box::new(|_, _, _| {}),
            None,
            Arc::new(SendLink::default()),
            Some(Box::new(move || {
                st.fetch_add(1, Ordering::SeqCst);
                Ok(broken())
            })),
            Box::new(move |e| ev.lock().push(e)),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !events.lock().iter().any(|e| matches!(e, CaptureEvent::Ended { .. })) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(publisher);
        let events = events.lock().clone();
        assert_eq!(starts.load(Ordering::SeqCst), RESTARTS_PER_MINUTE);
        assert_eq!(events.len(), RESTARTS_PER_MINUTE + 1, "{events:?}");
        assert!(events[..RESTARTS_PER_MINUTE].iter().all(|e| matches!(e, CaptureEvent::Restarted(_))));
        assert_eq!(events.last(), Some(&CaptureEvent::Ended { why: "capturing stopped".into(), gone: false }));
    }

    #[test]
    fn a_capture_that_comes_back_after_a_moment_carries_on() {
        // (Like the screens right after the graphics driver restarts: not there at first.)
        let cfg = PublisherConfig { screen: true, max_width: 320, max_height: 180, fps: 10, bitrate: 200_000 };
        let events = Arc::new(Mutex::new(Vec::new()));
        let ev = events.clone();
        let tries = Arc::new(AtomicUsize::new(0));
        let t2 = tries.clone();
        let publisher = Publisher::start(
            cfg,
            broken(),
            Arc::new(FrameSlot::default()),
            Box::new(|_, _, _| {}),
            None,
            Arc::new(SendLink::default()),
            Some(Box::new(move || {
                if t2.fetch_add(1, Ordering::SeqCst) < 2 { Err(anyhow::anyhow!("screen 2 not found")) } else { Ok(working()) }
            })),
            Box::new(move |e| ev.lock().push(e)),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while events.lock().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(publisher);
        let got = events.lock().clone();
        assert!(matches!(got.as_slice(), [CaptureEvent::Restarted(why)] if why.contains("came back")), "{got:?}");
        assert_eq!(tries.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn a_capture_that_cant_start_again_ends_the_share() {
        let cfg = PublisherConfig { screen: true, max_width: 320, max_height: 180, fps: 10, bitrate: 200_000 };
        let events = Arc::new(Mutex::new(Vec::new()));
        let ev = events.clone();
        let publisher = Publisher::start(
            cfg,
            broken(),
            Arc::new(FrameSlot::default()),
            Box::new(|_, _, _| {}),
            None,
            Arc::new(SendLink::default()),
            Some(Box::new(|| Err(anyhow::anyhow!("no screen")))),
            Box::new(move |e| ev.lock().push(e)),
        )
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while events.lock().is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(publisher);
        assert_eq!(
            events.lock().clone(),
            vec![CaptureEvent::Ended { why: "capturing stopped, and starting it again didn't work: no screen".into(), gone: false }]
        );
    }
}
