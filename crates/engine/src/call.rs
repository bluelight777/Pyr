//! An active voice call: E2EE key management, audio engine, video publish/subscribe.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use bytes::Bytes;
use ingle_proto::control::*;
use ingle_proto::media::{self, AAD_LEN, AudioHeader, VideoHeader, flags, track};
use parking_lot::Mutex;
use quinn::VarInt;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::Shared;
use crate::audio::stream::StreamAudio;
use crate::audio::{AudioEngine, AudioInput, AudioOutput, PacketCallback};
use crate::e2ee::E2ee;
use crate::server::ServerHandle;
use crate::state::{CallStats, CallView, VideoKey};
use crate::video::codec::EncodedFrame;
use crate::video::adapt::{PathSample, RateControl, ViewerReport};
use crate::video::{FrameSlot, Publisher, PublisherConfig, Receiver, SendLink, capture};

pub enum CallEvent {
    Upserted(VoiceParticipant),
    Removed(SessionId),
    KeyframeRequested(u8),
    /// How the viewers of our camera or screen share are getting on (from the server).
    VideoFeedback(u8, Vec<ViewerFeedback>),
    Key { from: SessionId, epoch: u16, nonce: Vec<u8>, ct: Vec<u8> },
    Disconnected,
}

/// Our camera or screen share being sent. Dropping it stops capture, which can take a moment
/// (Windows' capture waits for its thread), so it's dropped away from the UI thread and the
/// call lock (see `crate::drop_later`).
pub struct VideoOut {
    publisher: Publisher,
    cfg: PublisherConfig,
    net_task: tokio::task::JoinHandle<()>,
    rate_task: tokio::task::JoinHandle<()>,
    /// How much the network can take for it (see `video::adapt`).
    rate: Arc<Mutex<RateControl>>,
    /// Where to remember the bitrate it settled at (for the next share on this server), and
    /// under what (server key, track), and when it started.
    remember: (Arc<Mutex<HashMap<(String, u8), u32>>>, (String, u8), Instant),
}

impl Drop for VideoOut {
    fn drop(&mut self) {
        self.net_task.abort();
        self.rate_task.abort();
        // (Only once it has had time to settle; a little under it, to be safe.)
        if self.remember.2.elapsed() > Duration::from_secs(10) {
            let settled = self.rate.lock().target() as f64 * 0.9;
            self.remember.0.lock().insert(self.remember.1.clone(), settled as u32);
        }
    }
}

/// What starting a camera or screen share needs from the call, taken out of it so the slow part
/// (opening the capture, which can take seconds) runs without holding the call.
pub struct VideoCtx {
    pub session: SessionId,
    server_key: String,
    conn: quinn::Connection,
    e2ee: Arc<Mutex<E2ee>>,
    seqs: Arc<[AtomicU32; 4]>,
    started: Instant,
}

impl VideoCtx {
    /// Start capturing and encoding. It goes live once the call installs it
    /// (`Call::install_video`). `generation`: this start's (see `Shared::video_gen`), so a
    /// capture that ends by itself only takes its own share down.
    pub fn start(&self, shared: &Arc<Shared>, track_id: u8, source: &str, cfg: PublisherConfig, generation: u64) -> Result<(VideoOut, TrackInfo)> {
        let slot = Arc::new(FrameSlot::default());
        let screen = track_id == track::SCREEN;
        let mut open: crate::video::Restart = {
            let (source, slot) = (source.to_string(), slot.clone());
            if screen {
                Box::new(move || capture::start_screen(&source, cfg.fps, slot.clone()))
            } else {
                Box::new(move || capture::start_camera(&source, cfg.max_width, cfg.max_height, cfg.fps, slot.clone()))
            }
        };
        let capture = open()?;
        // What the capture does by itself: started again (and reported), or over.
        let on_event = {
            let s = shared.clone();
            let what = if screen { "screen sharing" } else { "the camera" };
            Box::new(move |ev: crate::video::CaptureEvent| match ev {
                crate::video::CaptureEvent::Restarted(why) => {
                    crate::bugs::auto_report(&s, &format!("{what} stopped working, and was started again"), &why);
                }
                crate::video::CaptureEvent::Ended { why, gone } => {
                    warn!("{what} ended: {why}");
                    let text = if gone { format!("Stopped sharing: {why}.") } else { format!("{} stopped working: {why}", if screen { "Screen sharing" } else { "Your camera" }) };
                    s.state.write().toast(text, !gone);
                    if !gone {
                        crate::bugs::auto_report(&s, &format!("{what} stopped working"), &why);
                    }
                    // Taken down elsewhere: stopping it waits for this thread.
                    let s2 = s.clone();
                    s.rt.spawn_blocking(move || crate::stop_video_on(&s2, track_id, Some(generation)));
                    (s.repaint)();
                }
            })
        };
        let link = Arc::new(SendLink::default());
        // Starting where the last share on this server settled (or at the preset's bitrate).
        let key = (self.server_key.clone(), track_id);
        let start_bps = shared.video_rates.lock().get(&key).copied().unwrap_or(cfg.bitrate).min(cfg.bitrate);
        let rate = Arc::new(Mutex::new(RateControl::new(cfg.bitrate, start_bps, Instant::now())));
        let (tx, rx) = mpsc::unbounded_channel::<(u32, Vec<u8>)>();
        let (e2ee, seqs, lk, started) = (self.e2ee.clone(), self.seqs.clone(), link.clone(), self.started);
        let counters = shared.clone();
        let sink = Box::new(move |f: EncodedFrame, w: u32, h: u32| {
            let Some((seq, pkt)) = seal_video(&e2ee, &seqs, track_id, &f, started.elapsed().as_millis() as u32) else {
                return;
            };
            debug!(target: "ingle_media", ev = "enc", track = track_id, seq, key = f.keyframe, bytes = pkt.len(), w, h);
            counters.call_counters.video_sent.fetch_add(1, Ordering::Relaxed);
            counters.call_counters.video_bytes.fetch_add(pkt.len() as u64, Ordering::Relaxed);
            lk.inflight.fetch_add(1, Ordering::Relaxed);
            let _ = tx.send((seq, pkt));
        });
        let video = shared.video.clone();
        let me = self.session;
        let preview = Box::new(move |w, h, px| video.put(VideoKey { session: me, track: track_id }, w, h, px));
        let (fps, max_w, max_h) = (cfg.fps, cfg.max_width, cfg.max_height);
        let publisher = Publisher::start(cfg, capture, slot, sink, Some(preview), link.clone(), Some(open), on_event)?;
        let net_task = shared.rt.spawn(video_send_loop(self.conn.clone(), rx, link.clone(), rate.clone()));
        let rate_task = shared.rt.spawn(rate_loop(self.conn.clone(), rate.clone(), link, track_id));
        let info = TrackInfo {
            id: track_id,
            kind: if track_id == track::SCREEN { TrackKind::Screen } else { TrackKind::Camera },
            width: max_w,
            height: max_h,
            fps,
            viewers: Vec::new(),
        };
        Ok((VideoOut { publisher, cfg, net_task, rate_task, rate, remember: (shared.video_rates.clone(), key, Instant::now()) }, info))
    }
}

pub struct Call {
    pub server_key: String,
    pub channel_id: ChannelId,
    pub session: SessionId,
    handle: ServerHandle,
    conn: quinn::Connection,
    e2ee: Arc<Mutex<E2ee>>,
    pub audio: Option<AudioEngine>,
    camera: Option<VideoOut>,
    screen: Option<VideoOut>,
    /// Sound for our screen share (only while sharing, and only if available).
    stream_audio: Option<StreamAudio>,
    receivers: HashMap<VideoKey, Receiver>,
    watching: HashSet<VideoKey>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    /// Per-track packet counters. They never restart during a session because the
    /// counter is the AEAD nonce.
    seqs: Arc<[AtomicU32; 4]>,
    started: Instant,
    /// A moderator muted me here: nothing goes out from the microphone (the server wouldn't
    /// pass it on anyway), and I don't show as speaking.
    server_muted: bool,
    /// A moderator deafened me here: I hear nobody (the server sends me nothing), but unlike
    /// deafening myself it doesn't mute me.
    server_deafened: bool,
}

fn seal_audio(e2ee: &Mutex<E2ee>, seqs: &[AtomicU32; 4], track_id: u8, fl: u8, opus: &[u8]) -> Option<Bytes> {
    let mut e = e2ee.lock();
    if !e.can_send() {
        return None;
    }
    let seq = seqs[track_id as usize].fetch_add(1, Ordering::Relaxed);
    let epoch = e.current_epoch();
    let h = AudioHeader { track: track_id, flags: fl, epoch, seq, session: 0 }.to_bytes();
    let ct = e.seal(track_id, seq, &h[..AAD_LEN], opus);
    drop(e);
    let mut out = Vec::with_capacity(h.len() + ct.len());
    out.extend_from_slice(&h);
    out.extend_from_slice(&ct);
    Some(Bytes::from(out))
}

fn seal_video(e2ee: &Mutex<E2ee>, seqs: &[AtomicU32; 4], track_id: u8, f: &EncodedFrame, ts_ms: u32) -> Option<(u32, Vec<u8>)> {
    let mut e = e2ee.lock();
    if !e.can_send() {
        return None;
    }
    let seq = seqs[track_id as usize].fetch_add(1, Ordering::Relaxed);
    let epoch = e.current_epoch();
    let mut h = VideoHeader {
        track: track_id,
        flags: if f.keyframe { flags::KEYFRAME } else { 0 },
        codec: media::CODEC_H264,
        epoch,
        seq,
        session: 0,
        ts_ms,
        len: 0,
    };
    let aad = h.to_bytes();
    let ct = e.seal(track_id, seq, &aad[..AAD_LEN], &f.data);
    drop(e);
    h.len = ct.len() as u32;
    let mut out = Vec::with_capacity(media::VIDEO_HEADER_LEN + ct.len());
    out.extend_from_slice(&h.to_bytes());
    out.extend_from_slice(&ct);
    Some((seq, out))
}

async fn video_send_loop(conn: quinn::Connection, mut rx: mpsc::UnboundedReceiver<(u32, Vec<u8>)>, link: Arc<SendLink>, rate: Arc<Mutex<RateControl>>) {
    let prioritize = !ingle_proto::ab::old("fair");
    let mut n: u64 = 0;
    while let Some((seq, buf)) = rx.recv().await {
        let sent = Instant::now();
        let mut s = match conn.open_uni().await {
            Ok(s) => s,
            Err(_) => {
                link.inflight.fetch_sub(1, Ordering::Relaxed);
                break;
            }
        };
        // Oldest frames first (see `media::frame_priority`).
        if prioritize {
            let _ = s.set_priority(media::frame_priority(n));
        }
        n += 1;
        rate.lock().on_sent(seq, buf.len(), sent);
        if s.write_all(&buf).await.is_err() {
            link.inflight.fetch_sub(1, Ordering::Relaxed);
            rate.lock().on_acked(seq, false, Instant::now());
            continue;
        }
        let _ = s.finish();
        let (lk, rc) = (link.clone(), rate.clone());
        let bytes = buf.len();
        tokio::spawn(async move {
            // All of it acknowledged (not stopped by the server, which takes no more when it's
            // swamped), within 2 s.
            let done = tokio::time::timeout(Duration::from_secs(2), s.stopped()).await;
            let ok = matches!(done, Ok(Ok(None)));
            if done.is_err() {
                let _ = s.reset(VarInt::from_u32(0));
            }
            debug!(target: "ingle_media", ev = "ack", seq, bytes, ms = sent.elapsed().as_millis() as u64, ok);
            rc.lock().on_acked(seq, ok, Instant::now());
            lk.inflight.fetch_sub(1, Ordering::Relaxed);
        });
    }
}

/// Every 100 ms: how much the network can take for one outgoing camera or screen share (see
/// `video::adapt::RateControl`), told to its encoder.
async fn rate_loop(conn: quinn::Connection, rate: Arc<Mutex<RateControl>>, link: Arc<SendLink>, track_id: u8) {
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut said = (0u32, "");
    loop {
        tick.tick().await;
        if conn.close_reason().is_some() {
            break;
        }
        let p = conn.stats().path;
        let now = Instant::now();
        let sample = PathSample { rtt: p.rtt, min_rtt: p.min_rtt, lost: p.lost_packets, sent: p.sent_packets };
        let (target, late, why, acked, base) = {
            let mut r = rate.lock();
            let t = r.tick(now, sample);
            (t, r.oldest_late(now), r.why, r.acked_bps(now), r.base_rtt())
        };
        link.target_bps.store(target, Ordering::Relaxed);
        link.late_ms.store(late.as_millis() as u32, Ordering::Relaxed);
        link.rtt_ms.store(p.rtt.as_millis() as u32, Ordering::Relaxed);
        // (The media log: when it changes course, and twice a second.)
        if tracing::enabled!(target: "ingle_media", tracing::Level::DEBUG) && (why != said.1 || target.abs_diff(said.0) > said.0 / 10) {
            said = (target, why);
            debug!(
                target: "ingle_media",
                ev = "rate",
                track = track_id,
                kbps = target / 1000,
                why,
                acked_kbps = (acked / 1000.0) as u32,
                late_ms = late.as_millis() as u64,
                rtt = p.rtt.as_millis() as u64,
                base = base.as_millis() as u64
            );
        }
    }
}

/// For measuring calls over test networks (`ingle_media` at debug level): the connection's
/// round trip, congestion window and losses, twice a second.
async fn path_log_loop(conn: quinn::Connection) {
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    loop {
        tick.tick().await;
        if conn.close_reason().is_some() {
            break;
        }
        if tracing::enabled!(target: "ingle_media", tracing::Level::DEBUG) {
            let p = conn.stats().path;
            debug!(
                target: "ingle_media",
                ev = "path",
                rtt = p.rtt.as_micros() as u64,
                min_rtt = p.min_rtt.as_micros() as u64,
                cwnd = p.cwnd,
                lost = p.lost_packets,
                sent = p.sent_packets,
                cong = p.congestion_events
            );
        }
    }
}

/// Packets in a row from someone that don't open (about two seconds of their sound) before
/// it's worth a line in the log.
const UNDECRYPTABLE_LOG_AFTER: u32 = 100;

/// Receive audio datagrams for the whole call.
async fn datagram_loop(shared: Arc<Shared>, conn: quinn::Connection, e2ee: Arc<Mutex<E2ee>>) {
    // Per sender: packets in a row that didn't open, and whether that's in the log.
    let mut failing: HashMap<SessionId, (u32, bool)> = HashMap::new();
    loop {
        let d = match conn.read_datagram().await {
            Ok(d) => d,
            Err(e) => {
                debug!("no more voice packets: {e}");
                break;
            }
        };
        let Some(h) = AudioHeader::parse(&d) else { continue };
        let ct = &d[media::AUDIO_HEADER_LEN..];
        let pt = e2ee.lock().open(h.session, h.epoch, h.track, h.seq, &d[..AAD_LEN], ct);
        let Some(pt) = pt else {
            shared.call_counters.undecryptable.fetch_add(1, Ordering::Relaxed);
            let f = failing.entry(h.session).or_default();
            f.0 += 1;
            if f.0 >= UNDECRYPTABLE_LOG_AFTER && !f.1 {
                f.1 = true;
                let why = e2ee.lock().why_not(h.session, h.epoch);
                warn!(session = h.session, epoch = h.epoch, "someone's sound doesn't decrypt: {why}");
            }
            continue;
        };
        if !failing.is_empty() {
            if let Some((_, true)) = failing.remove(&h.session) {
                info!(session = h.session, "their sound decrypts again");
            }
        }
        let mixer = shared.call.lock().as_ref().and_then(|c| c.audio.as_ref().map(|a| a.mixer.clone()));
        if let Some(m) = mixer {
            m.lock().push(h.session, h.track, h.seq, h.flags, pt, Instant::now());
        }
    }
}

/// Receive video frames (one QUIC stream each), a few dozen at a time, each read no further
/// than its header says.
async fn uni_loop(shared: Arc<Shared>, conn: quinn::Connection, e2ee: Arc<Mutex<E2ee>>) {
    let slots = Arc::new(tokio::sync::Semaphore::new(48));
    while let Ok(mut s) = conn.accept_uni().await {
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            let _ = s.stop(VarInt::from_u32(0));
            continue;
        };
        let shared = shared.clone();
        let e2ee = e2ee.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let read = async {
                let mut head = [0u8; media::VIDEO_HEADER_LEN];
                s.read_exact(&mut head).await.ok()?;
                let h = VideoHeader::parse(&head)?;
                if h.len as usize > ingle_proto::MAX_VIDEO_FRAME {
                    return None;
                }
                let mut data = vec![0u8; media::VIDEO_HEADER_LEN + h.len as usize];
                data[..media::VIDEO_HEADER_LEN].copy_from_slice(&head);
                s.read_exact(&mut data[media::VIDEO_HEADER_LEN..]).await.ok()?;
                Some(data)
            };
            let Ok(Some(data)) = tokio::time::timeout(Duration::from_secs(5), read).await else {
                let _ = s.stop(VarInt::from_u32(0));
                return;
            };
            let Some(h) = VideoHeader::parse(&data) else { return };
            let ct = &data[media::VIDEO_HEADER_LEN..];
            let Some(pt) = e2ee.lock().open(h.session, h.epoch, h.track, h.seq, &data[..AAD_LEN], ct) else {
                shared.call_counters.undecryptable.fetch_add(1, Ordering::Relaxed);
                return;
            };
            shared.call_counters.video_received.fetch_add(1, Ordering::Relaxed);
            let key = VideoKey { session: h.session, track: h.track };
            if let Some(call) = shared.call.lock().as_ref() {
                if let Some(r) = call.receivers.get(&key) {
                    r.push(h.seq, h.is_keyframe(), pt);
                }
            }
        });
    }
}

async fn event_loop(shared: Arc<Shared>, mut rx: mpsc::UnboundedReceiver<CallEvent>) {
    while let Some(ev) = rx.recv().await {
        let mut end = false;
        let mut lost = false;
        {
            let mut guard = shared.call.lock();
            let Some(call) = guard.as_mut() else { break };
            match ev {
                CallEvent::Upserted(p) => call.on_participant(&shared, &p),
                CallEvent::Removed(s) => {
                    if s == call.session {
                        end = true;
                    } else {
                        call.on_removed(&shared, s);
                    }
                }
                CallEvent::VideoFeedback(t, viewers) => {
                    let out = if t == track::CAMERA { call.camera.as_ref() } else { call.screen.as_ref() };
                    if let Some(v) = out.filter(|_| !ingle_proto::ab::old("feedback")) {
                        let reports: Vec<ViewerReport> = viewers
                            .iter()
                            .map(|f| ViewerReport {
                                session: f.session,
                                rtt_ms: f.rtt_ms,
                                queue_ms: f.queue_ms,
                                late_ms: f.late_ms,
                                kbps: f.kbps,
                                loss_pm: f.loss_pm,
                                shed: f.shed,
                            })
                            .collect();
                        for r in &reports {
                            debug!(target: "ingle_media", ev = "feedback", viewer = r.session, rtt = r.rtt_ms, queue = r.queue_ms, late = r.late_ms, kbps = r.kbps, loss_pm = r.loss_pm, shed = r.shed);
                        }
                        v.rate.lock().on_viewers(&reports, Instant::now());
                        shared.call_counters.video_feedback.fetch_add(1, Ordering::Relaxed);
                    }
                }
                CallEvent::KeyframeRequested(t) => {
                    debug!(target: "ingle_media", ev = "kfask", track = t);
                    let out = if t == track::CAMERA { call.camera.as_ref() } else { call.screen.as_ref() };
                    if let Some(v) = out {
                        v.publisher.keyframe.store(true, Ordering::Relaxed);
                    }
                }
                CallEvent::Key { from, epoch, nonce, ct } => {
                    if !call.e2ee.lock().receive_key(from, epoch, &nonce, &ct) {
                        debug!(from, epoch, "couldn't use an E2EE key");
                    }
                }
                CallEvent::Disconnected => lost = true,
            }
        }
        if lost {
            shared.call_lost();
            break;
        }
        if end {
            // (A direct message's call ends when the other person blocks you, say.)
            let dm = {
                let st = shared.state.read_recursive();
                st.call.as_ref().is_some_and(|c| st.is_dm_call(&c.server_key, c.channel_id))
            };
            shared.end_call_local(if dm { "The call ended." } else { "You left the voice channel." });
            break;
        }
        shared.refresh_call_view();
    }
}

async fn status_loop(shared: Arc<Shared>) {
    let mut tick = tokio::time::interval(Duration::from_millis(120));
    loop {
        tick.tick().await;
        if shared.call.lock().is_none() {
            break;
        }
        shared.refresh_call_view();
    }
}

impl Call {
    pub async fn join(shared: Arc<Shared>, key: String, channel_id: ChannelId) -> Result<()> {
        let handle = shared.servers.lock().get(&key).cloned().ok_or_else(|| anyhow!("not connected to that server"))?;
        let conn = handle.connection().ok_or_else(|| anyhow!("not connected to that server"))?;
        let (muted, deafened) = *shared.voice_prefs.lock();
        let mut e2ee = E2ee::new(&shared.identity, channel_id);
        let (e2ee_pub, e2ee_sig) = e2ee.join_credentials();
        // Start collecting call events *before* joining: other participants react to our
        // arrival (e.g. by sending us their keys) before our join request even returns.
        let (ev_tx, ev_rx) = mpsc::unbounded_channel();
        *shared.call_events.lock() = Some((key.clone(), ev_tx));
        let sent = (muted || deafened, deafened);
        let reply = handle
            .request(Request::VoiceJoin { channel_id, e2ee_pub, e2ee_sig, self_mute: sent.0, self_deaf: sent.1 })
            .await
            .map_err(|_| anyhow!("connection lost"));
        let (session, participants, before) = match reply {
            Ok(Event::VoiceReady { session, participants, before }) => (session, participants, before),
            Ok(Event::Error { message, .. }) => {
                *shared.call_events.lock() = None;
                bail!(message)
            }
            Err(e) => {
                *shared.call_events.lock() = None;
                return Err(e);
            }
            _ => {
                *shared.call_events.lock() = None;
                bail!("unexpected reply")
            }
        };
        // Just updated from a version that didn't remember mute and deafen (0.6.1 or older): a
        // microphone the server saw muted when Ingle closed stays muted (nobody's heard who
        // didn't mean to be), but you hear everyone again, and you're told (coming back
        // silently muted and deafened looked like the update had done it: bug report #3).
        // From then on this PC remembers (and is what counts).
        if let Some(b) = before {
            let unknown = shared.config.lock().self_voice.is_none();
            if unknown {
                info!(muted = b.self_mute, deafened = b.self_deaf, "rejoining voice the way I left it");
                let muted = b.self_mute || b.self_deaf;
                *shared.voice_prefs.lock() = (muted, false);
                shared.apply_voice_prefs();
                if muted {
                    shared
                        .state
                        .write()
                        .toast("You're muted: your microphone was off when Ingle closed to update. Click the microphone to talk.", false);
                }
            }
        }
        // Mute or deafen changed while joining (or just above): tell everyone. Nothing has been
        // sent from the microphone yet; that starts below.
        let (muted, deafened) = *shared.voice_prefs.lock();
        if (muted || deafened, deafened) != sent {
            handle.notify(Request::VoiceState { self_mute: muted || deafened, self_deaf: deafened });
        }
        e2ee.set_my_session(session);
        for p in &participants {
            e2ee.upsert_peer(p);
        }
        let (epoch, keys) = e2ee.rotate();
        if !keys.is_empty() {
            handle.notify(Request::E2eeKeys { epoch, keys });
        }
        let e2ee = Arc::new(Mutex::new(e2ee));
        let seqs: Arc<[AtomicU32; 4]> = Arc::new(Default::default());
        shared.call_counters.reset();

        let mut call = Call {
            server_key: key.clone(),
            channel_id,
            session,
            handle,
            conn: conn.clone(),
            e2ee: e2ee.clone(),
            audio: None,
            camera: None,
            screen: None,
            stream_audio: None,
            receivers: HashMap::new(),
            watching: HashSet::new(),
            tasks: Vec::new(),
            seqs,
            started: Instant::now(),
            server_muted: participants.iter().any(|p| p.session == session && p.server_mute),
            server_deafened: participants.iter().any(|p| p.session == session && p.server_deaf),
        };
        call.start_audio(&shared, muted, deafened);
        call.tasks.push(tokio::spawn(datagram_loop(shared.clone(), conn.clone(), e2ee.clone())));
        call.tasks.push(tokio::spawn(uni_loop(shared.clone(), conn.clone(), e2ee.clone())));
        if tracing::enabled!(target: "ingle_media", tracing::Level::DEBUG) {
            call.tasks.push(tokio::spawn(path_log_loop(conn.clone())));
        }
        for p in &participants {
            call.on_participant(&shared, p);
        }
        {
            let mut st = shared.state.write();
            st.call = Some(CallView {
                server_key: key,
                channel_id,
                session,
                muted: muted || deafened,
                deafened,
                camera_on: false,
                screen_on: false,
                stream_audio_on: false,
                transmitting: false,
                speaking: HashSet::new(),
                watching: HashSet::new(),
                call_code: e2ee.lock().call_code(),
                e2ee_ready: false,
                stats: CallStats::default(),
                audio_error: None,
                video_error: None,
                joined_at: Instant::now(),
                sounds: Vec::new(),
            });
        }
        *shared.call.lock() = Some(call);
        // A click on mute or deafen in the last moment, before the call was here to take it.
        if *shared.voice_prefs.lock() != (muted, deafened) {
            shared.apply_voice_prefs();
        }
        // Now that the call exists, process everything that arrived meanwhile (in order).
        let events = tokio::spawn(event_loop(shared.clone(), ev_rx));
        if let Some(c) = shared.call.lock().as_mut() {
            c.tasks.push(events);
        }
        tokio::spawn(status_loop(shared.clone()));
        info!(session, channel_id, "joined voice");
        Ok(())
    }

    pub fn start_audio(&mut self, shared: &Arc<Shared>, muted: bool, deafened: bool) {
        if let Some(mut a) = self.audio.take() {
            a.stop();
        }
        let settings = shared.state.read_recursive().settings.clone();
        let (input, output) = shared.audio_io();
        let e2ee = self.e2ee.clone();
        let seqs = self.seqs.clone();
        let conn = self.conn.clone();
        let counters = shared.clone();
        let sink = Box::new(move |track_id: u8, fl: u8, opus: &[u8]| {
            let Some(pkt) = seal_audio(&e2ee, &seqs, track_id, fl, opus) else { return };
            if conn.send_datagram(pkt).is_ok() {
                counters.call_counters.audio_sent.fetch_add(1, Ordering::Relaxed);
            }
        });
        let engine = AudioEngine::start(settings.clone(), input, output, sink, shared.ptt_down.clone());
        *engine.controls.played.lock() = Some(shared.played.clone());
        engine.controls.muted.store(muted || deafened || self.server_muted, Ordering::Relaxed);
        engine.controls.deafened.store(deafened || self.server_deafened, Ordering::Relaxed);
        engine.controls.monitor.store(shared.state.read_recursive().mic_test, Ordering::Relaxed);
        // Per-person volumes (voice and stream audio).
        {
            let (vols, svols) = {
                let cfg = shared.config.lock();
                (cfg.user_volumes.clone(), cfg.stream_volumes.clone())
            };
            let st = shared.state.read_recursive();
            if let Some(s) = st.server(&self.server_key) {
                let mut m = engine.mixer.lock();
                for p in s.voice.values() {
                    if let Some(v) = vols.get(&p.user_id) {
                        m.set_volume(p.session, *v);
                    }
                    if let Some(v) = svols.get(&p.user_id) {
                        m.set_stream_volume(p.session, *v);
                    }
                }
            }
        }
        self.audio = Some(engine);
    }

    fn on_participant(&mut self, shared: &Arc<Shared>, p: &VoiceParticipant) {
        if p.session == self.session {
            self.set_moderated(shared, p.server_mute, p.server_deaf);
            return;
        }
        if p.channel_id != self.channel_id {
            return;
        }
        let new_peer = self.e2ee.lock().upsert_peer(p);
        if new_peer {
            let (epoch, keys) = self.e2ee.lock().rotate();
            if !keys.is_empty() {
                self.handle.notify(Request::E2eeKeys { epoch, keys });
            }
        }
        if let Some(a) = &self.audio {
            let cfg = shared.config.lock();
            let mut m = a.mixer.lock();
            if let Some(v) = cfg.user_volumes.get(&p.user_id) {
                m.set_volume(p.session, *v);
            }
            if let Some(v) = cfg.stream_volumes.get(&p.user_id) {
                m.set_stream_volume(p.session, *v);
            }
        }
        // Cameras are shown automatically; screen shares only when you choose to watch.
        let cam = VideoKey { session: p.session, track: track::CAMERA };
        let has_cam = p.tracks.iter().any(|t| t.id == track::CAMERA);
        if has_cam && !self.receivers.contains_key(&cam) {
            self.subscribe(shared, cam);
        } else if !has_cam && self.receivers.contains_key(&cam) {
            self.receivers.remove(&cam);
            shared.video.remove(cam);
        }
        let scr = VideoKey { session: p.session, track: track::SCREEN };
        if !p.tracks.iter().any(|t| t.id == track::SCREEN) && self.receivers.contains_key(&scr) {
            self.receivers.remove(&scr);
            self.watching.remove(&scr);
            shared.video.remove(scr);
            self.drop_stream_audio_of(p.session);
        }
    }

    fn on_removed(&mut self, shared: &Arc<Shared>, s: SessionId) {
        if self.e2ee.lock().remove_peer(s) {
            let (epoch, keys) = self.e2ee.lock().rotate();
            if !keys.is_empty() {
                self.handle.notify(Request::E2eeKeys { epoch, keys });
            }
        }
        self.receivers.retain(|k, _| k.session != s);
        self.watching.retain(|k| k.session != s);
        if let Some(a) = &self.audio {
            a.mixer.lock().remove_session(s);
        }
        shared.video.remove_session(s);
    }

    fn subscribe(&mut self, shared: &Arc<Shared>, key: VideoKey) {
        let handle = self.handle.clone();
        let rq = Box::new(move || handle.notify(Request::RequestKeyframe { session: key.session, track: key.track }));
        let conn = self.conn.clone();
        self.receivers.insert(key, Receiver::start(key, shared.video.clone(), rq, Box::new(move || conn.rtt())));
        self.handle.notify(Request::Subscribe { session: key.session, track: key.track });
    }

    pub fn watch(&mut self, shared: &Arc<Shared>, key: VideoKey, on: bool) {
        if on {
            self.watching.insert(key);
            if !self.receivers.contains_key(&key) {
                self.subscribe(shared, key);
            }
        } else {
            self.watching.remove(&key);
            self.receivers.remove(&key);
            self.handle.notify(Request::Unsubscribe { session: key.session, track: key.track });
            shared.video.remove(key);
            if key.track == track::SCREEN {
                self.drop_stream_audio_of(key.session);
            }
        }
    }

    /// Stop playing someone's stream audio right away (instead of letting the buffer drain).
    fn drop_stream_audio_of(&self, session: SessionId) {
        if let Some(a) = &self.audio {
            a.mixer.lock().remove_track(session, track::SCREEN_AUDIO);
        }
    }

    /// Where our stream audio packets go: sealed like the microphone, sent as datagrams.
    pub fn stream_audio_sink(&self, shared: &Arc<Shared>) -> PacketCallback {
        let (e2ee, seqs, conn, counters) = (self.e2ee.clone(), self.seqs.clone(), self.conn.clone(), shared.clone());
        Box::new(move |track_id: u8, fl: u8, opus: &[u8]| {
            let Some(pkt) = seal_audio(&e2ee, &seqs, track_id, fl, opus) else { return };
            if conn.send_datagram(pkt).is_ok() {
                counters.call_counters.audio_sent.fetch_add(1, Ordering::Relaxed);
            }
        })
    }

    /// Attach running stream audio to our screen share and tell the others it has sound.
    /// Returns it back if we're no longer sharing.
    pub fn attach_stream_audio(&mut self, sa: StreamAudio) -> Option<StreamAudio> {
        if self.screen.is_none() {
            return Some(sa);
        }
        self.stream_audio = Some(sa);
        let info = TrackInfo { id: track::SCREEN_AUDIO, kind: TrackKind::ScreenAudio, width: 0, height: 0, fps: 0, viewers: Vec::new() };
        self.handle.notify(Request::Publish { track: info });
        None
    }

    /// Our screen share is sending sound (and capture hasn't failed since it started).
    pub fn stream_audio_on(&self) -> bool {
        self.stream_audio.as_ref().map(|s| s.error.lock().is_none()).unwrap_or(false)
    }

    pub fn watching(&self) -> HashSet<VideoKey> {
        self.watching.clone()
    }

    pub fn set_mute(&mut self, muted: bool, deafened: bool) {
        self.apply_mic(muted, deafened);
        self.handle.notify(Request::VoiceState { self_mute: muted || deafened, self_deaf: deafened });
    }

    /// What the microphone and speakers do: my own mute and deafen (which mutes too), and a
    /// moderator's (whose deafen doesn't).
    fn apply_mic(&self, muted: bool, deafened: bool) {
        if let Some(a) = &self.audio {
            a.controls.muted.store(muted || deafened || self.server_muted, Ordering::Relaxed);
            a.controls.deafened.store(deafened || self.server_deafened, Ordering::Relaxed);
        }
    }

    /// A moderator muted, deafened, unmuted or undeafened me (`VoiceParticipant::server_mute`,
    /// `server_deaf`).
    fn set_moderated(&mut self, shared: &Shared, muted: bool, deafened: bool) {
        if (self.server_muted, self.server_deafened) == (muted, deafened) {
            return;
        }
        (self.server_muted, self.server_deafened) = (muted, deafened);
        let (m, d) = *shared.voice_prefs.lock();
        self.apply_mic(m, d);
        info!(muted, deafened, "a moderator changed my voice");
    }

    /// What `VideoCtx::start` needs, to start a camera or screen share outside the call lock.
    pub fn video_ctx(&self) -> VideoCtx {
        VideoCtx {
            session: self.session,
            server_key: self.server_key.clone(),
            conn: self.conn.clone(),
            e2ee: self.e2ee.clone(),
            seqs: self.seqs.clone(),
            started: self.started,
        }
    }

    /// A started camera or screen share goes live (the server tells the others). Returns the one
    /// it replaces, to be dropped elsewhere.
    pub fn install_video(&mut self, track_id: u8, out: VideoOut, info: TrackInfo) -> Option<VideoOut> {
        self.handle.notify(Request::Publish { track: info });
        if track_id == track::SCREEN { self.screen.replace(out) } else { self.camera.replace(out) }
    }

    /// Take our camera or screen share (with the share's sound) out of the call; the server
    /// stops forwarding it. Drop what comes back away from the UI thread and the call lock.
    pub fn take_video(&mut self, shared: &Arc<Shared>, track_id: u8) -> (Option<VideoOut>, Option<StreamAudio>) {
        let out = if track_id == track::SCREEN { self.screen.take() } else { self.camera.take() };
        let sound = if track_id == track::SCREEN { self.stream_audio.take() } else { None };
        if sound.is_some() {
            self.handle.notify(Request::Unpublish { track: track::SCREEN_AUDIO });
        }
        if out.is_some() {
            self.handle.notify(Request::Unpublish { track: track_id });
        }
        shared.video.remove(VideoKey { session: self.session, track: track_id });
        (out, sound)
    }

    pub fn video_on(&self, track_id: u8) -> bool {
        if track_id == track::SCREEN { self.screen.is_some() } else { self.camera.is_some() }
    }

    /// What our screen share (or else our camera) is sending now.
    pub fn sending(&self) -> Option<crate::state::Sending> {
        let v = self.screen.as_ref().or(self.camera.as_ref())?;
        let s = &v.publisher.stats;
        let (height, fps, kbps) = (s.height.load(Ordering::Relaxed) as u32, s.fps.load(Ordering::Relaxed), s.target_kbps.load(Ordering::Relaxed));
        if height == 0 {
            return None;
        }
        // (Only what the network asked for counts as lowered: see `PublisherStats::lowered`.)
        let lowered = s.lowered.load(Ordering::Relaxed);
        Some(crate::state::Sending { height, fps: if fps > 0 { fps } else { v.cfg.fps }, kbps: if kbps > 0 { kbps } else { v.cfg.bitrate / 1000 }, lowered })
    }

    pub fn publisher_stats(&self, track_id: u8) -> Option<(u64, u64, u64)> {
        let v = if track_id == track::SCREEN { self.screen.as_ref() } else { self.camera.as_ref() }?;
        let s = &v.publisher.stats;
        Some((s.frames.load(Ordering::Relaxed), s.width.load(Ordering::Relaxed), s.height.load(Ordering::Relaxed)))
    }

    pub fn conn_rtt(&self) -> u32 {
        self.conn.rtt().as_millis() as u32
    }

    pub fn call_code(&self) -> String {
        self.e2ee.lock().call_code()
    }

    pub fn e2ee_ready(&self) -> bool {
        self.e2ee.lock().all_peers_keyed()
    }

    pub fn shutdown(mut self, notify_server: bool) {
        if notify_server {
            self.handle.notify(Request::VoiceLeave);
        }
        for t in self.tasks.drain(..) {
            t.abort();
        }
        // Capture and decoders stop on a thread of their own: Windows' capture can take a
        // moment to let go, and this runs on the UI thread when you leave.
        crate::drop_later((self.camera.take(), self.screen.take(), self.stream_audio.take(), std::mem::take(&mut self.receivers)));
        if let Some(mut a) = self.audio.take() {
            a.stop();
        }
    }
}

/// Make audio I/O for a call (real devices unless the engine was built for tests).
pub fn default_audio_io(settings: &crate::config::Settings) -> (AudioInput, AudioOutput) {
    (AudioInput::Device(settings.input_device.clone()), AudioOutput::Device(settings.output_device.clone()))
}
