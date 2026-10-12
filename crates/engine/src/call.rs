//! An active voice call: E2EE key management, audio engine, video publish/subscribe.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use bytes::Bytes;
use ingle_proto::control::*;
use ingle_proto::media::{self, AAD_LEN, AudioHeader, VideoHeader, flags, track};
use parking_lot::Mutex;
use quinn::VarInt;
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::Shared;
use crate::audio::stream::StreamAudio;
use crate::audio::{AudioEngine, AudioInput, AudioOutput, PacketCallback};
use crate::e2ee::E2ee;
use crate::server::ServerHandle;
use crate::state::{CallStats, CallView, VideoKey};
use crate::video::codec::EncodedFrame;
use crate::video::{FrameSlot, Publisher, PublisherConfig, Receiver, capture};

pub enum CallEvent {
    Upserted(VoiceParticipant),
    Removed(SessionId),
    KeyframeRequested(u8),
    Key { from: SessionId, epoch: u16, nonce: Vec<u8>, ct: Vec<u8> },
    Disconnected,
}

struct VideoOut {
    publisher: Publisher,
    net_task: tokio::task::JoinHandle<()>,
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

fn seal_video(e2ee: &Mutex<E2ee>, seqs: &[AtomicU32; 4], track_id: u8, f: &EncodedFrame, ts_ms: u32) -> Option<Vec<u8>> {
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
    Some(out)
}

async fn video_send_loop(conn: quinn::Connection, mut rx: mpsc::UnboundedReceiver<Vec<u8>>, inflight: Arc<AtomicUsize>) {
    while let Some(buf) = rx.recv().await {
        let mut s = match conn.open_uni().await {
            Ok(s) => s,
            Err(_) => {
                inflight.fetch_sub(1, Ordering::Relaxed);
                break;
            }
        };
        if s.write_all(&buf).await.is_err() {
            inflight.fetch_sub(1, Ordering::Relaxed);
            continue;
        }
        let _ = s.finish();
        let inf = inflight.clone();
        tokio::spawn(async move {
            if tokio::time::timeout(Duration::from_secs(2), s.stopped()).await.is_err() {
                let _ = s.reset(VarInt::from_u32(0));
            }
            inf.fetch_sub(1, Ordering::Relaxed);
        });
    }
}

/// Receive audio datagrams for the whole call.
async fn datagram_loop(shared: Arc<Shared>, conn: quinn::Connection, e2ee: Arc<Mutex<E2ee>>) {
    while let Ok(d) = conn.read_datagram().await {
        let Some(h) = AudioHeader::parse(&d) else { continue };
        let ct = &d[media::AUDIO_HEADER_LEN..];
        let pt = e2ee.lock().open(h.session, h.epoch, h.track, h.seq, &d[..AAD_LEN], ct);
        let Some(pt) = pt else {
            shared.call_counters.undecryptable.fetch_add(1, Ordering::Relaxed);
            continue;
        };
        let mixer = shared.call.lock().as_ref().and_then(|c| c.audio.as_ref().map(|a| a.mixer.clone()));
        if let Some(m) = mixer {
            m.lock().push(h.session, h.track, h.seq, h.flags, pt, Instant::now());
        }
    }
}

/// Receive video frames (one QUIC stream each).
async fn uni_loop(shared: Arc<Shared>, conn: quinn::Connection, e2ee: Arc<Mutex<E2ee>>) {
    while let Ok(mut s) = conn.accept_uni().await {
        let shared = shared.clone();
        let e2ee = e2ee.clone();
        tokio::spawn(async move {
            let Ok(Ok(data)) = tokio::time::timeout(
                Duration::from_secs(5),
                s.read_to_end(ingle_proto::MAX_VIDEO_FRAME + media::VIDEO_HEADER_LEN),
            )
            .await
            else {
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
                CallEvent::KeyframeRequested(t) => {
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
            shared.end_call_local("You left the voice channel.");
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
        let reply = handle
            .request(Request::VoiceJoin { channel_id, e2ee_pub, e2ee_sig, self_mute: muted, self_deaf: deafened })
            .await
            .map_err(|_| anyhow!("connection lost"));
        let (session, participants) = match reply {
            Ok(Event::VoiceReady { session, participants }) => (session, participants),
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
        };
        call.start_audio(&shared, muted, deafened);
        call.tasks.push(tokio::spawn(datagram_loop(shared.clone(), conn.clone(), e2ee.clone())));
        call.tasks.push(tokio::spawn(uni_loop(shared.clone(), conn.clone(), e2ee.clone())));
        for p in &participants {
            call.on_participant(&shared, p);
        }
        {
            let mut st = shared.state.write();
            st.call = Some(CallView {
                server_key: key,
                channel_id,
                session,
                muted,
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
            });
        }
        *shared.call.lock() = Some(call);
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
        let settings = shared.state.read().settings.clone();
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
        engine.controls.muted.store(muted || deafened, Ordering::Relaxed);
        engine.controls.deafened.store(deafened, Ordering::Relaxed);
        engine.controls.monitor.store(shared.state.read().mic_test, Ordering::Relaxed);
        // Per-person volumes (voice and stream audio).
        {
            let (vols, svols) = {
                let cfg = shared.config.lock();
                (cfg.user_volumes.clone(), cfg.stream_volumes.clone())
            };
            let st = shared.state.read();
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
        if p.session == self.session || p.channel_id != self.channel_id {
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
        self.receivers.insert(key, Receiver::start(key, shared.video.clone(), rq));
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
        if let Some(a) = &self.audio {
            a.controls.muted.store(muted || deafened, Ordering::Relaxed);
            a.controls.deafened.store(deafened, Ordering::Relaxed);
        }
        self.handle.notify(Request::VoiceState { self_mute: muted || deafened, self_deaf: deafened });
    }

    pub fn start_video(
        &mut self,
        shared: &Arc<Shared>,
        track_id: u8,
        source: &str,
        cfg: PublisherConfig,
    ) -> Result<(u32, u32)> {
        let slot = Arc::new(FrameSlot::default());
        let capture = if track_id == track::SCREEN {
            capture::start_screen(source, cfg.fps, slot.clone())?
        } else {
            capture::start_camera(source, cfg.max_width, cfg.max_height, cfg.fps, slot.clone())?
        };
        let inflight = Arc::new(AtomicUsize::new(0));
        let (tx, rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (e2ee, seqs, inf, started) = (self.e2ee.clone(), self.seqs.clone(), inflight.clone(), self.started);
        let counters = shared.clone();
        let sink = Box::new(move |f: EncodedFrame, _w: u32, _h: u32| {
            let Some(pkt) = seal_video(&e2ee, &seqs, track_id, &f, started.elapsed().as_millis() as u32) else {
                return;
            };
            counters.call_counters.video_sent.fetch_add(1, Ordering::Relaxed);
            counters.call_counters.video_bytes.fetch_add(pkt.len() as u64, Ordering::Relaxed);
            inf.fetch_add(1, Ordering::Relaxed);
            let _ = tx.send(pkt);
        });
        let video = shared.video.clone();
        let me = self.session;
        let preview = Box::new(move |w, h, px| video.put(VideoKey { session: me, track: track_id }, w, h, px));
        let (fps, max_w, max_h) = (cfg.fps, cfg.max_width, cfg.max_height);
        let publisher = Publisher::start(cfg, capture, slot, sink, Some(preview), inflight.clone())?;
        let net_task = tokio::spawn(video_send_loop(self.conn.clone(), rx, inflight));
        let info = TrackInfo {
            id: track_id,
            kind: if track_id == track::SCREEN { TrackKind::Screen } else { TrackKind::Camera },
            width: max_w,
            height: max_h,
            fps,
            viewers: Vec::new(),
        };
        self.handle.notify(Request::Publish { track: info });
        let out = VideoOut { publisher, net_task };
        if track_id == track::SCREEN {
            self.screen = Some(out);
        } else {
            self.camera = Some(out);
        }
        Ok((max_w, max_h))
    }

    pub fn stop_video(&mut self, shared: &Arc<Shared>, track_id: u8) {
        let out = if track_id == track::SCREEN { self.screen.take() } else { self.camera.take() };
        if track_id == track::SCREEN && self.stream_audio.take().is_some() {
            self.handle.notify(Request::Unpublish { track: track::SCREEN_AUDIO });
        }
        if let Some(v) = out {
            v.net_task.abort();
            drop(v.publisher);
            self.handle.notify(Request::Unpublish { track: track_id });
        }
        shared.video.remove(VideoKey { session: self.session, track: track_id });
    }

    pub fn video_on(&self, track_id: u8) -> bool {
        if track_id == track::SCREEN { self.screen.is_some() } else { self.camera.is_some() }
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
        if let Some(v) = self.camera.take() {
            v.net_task.abort();
        }
        if let Some(v) = self.screen.take() {
            v.net_task.abort();
        }
        self.stream_audio = None;
        self.receivers.clear();
        if let Some(mut a) = self.audio.take() {
            a.stop();
        }
    }
}

/// Make audio I/O for a call (real devices unless the engine was built for tests).
pub fn default_audio_io(settings: &crate::config::Settings) -> (AudioInput, AudioOutput) {
    (AudioInput::Device(settings.input_device.clone()), AudioOutput::Device(settings.output_device.clone()))
}
