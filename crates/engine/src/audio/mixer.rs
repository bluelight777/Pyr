//! Mixes every speaker's jitter buffer into one stereo 48 kHz stream.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ingle_proto::control::SessionId;
use ingle_proto::media::track;

use super::jitter::{JitterStats, PeerAudio};

pub struct Mixer {
    peers: HashMap<(SessionId, u8), PeerAudio>,
    volumes: HashMap<SessionId, f32>,
    /// Separate volume for someone's stream audio (screen share sound).
    stream_volumes: HashMap<SessionId, f32>,
    pub master: f32,
    pub deafened: bool,
    scratch: Vec<f32>,
}

impl Default for Mixer {
    fn default() -> Self {
        Mixer {
            peers: HashMap::new(),
            volumes: HashMap::new(),
            stream_volumes: HashMap::new(),
            master: 1.0,
            deafened: false,
            scratch: vec![0.0; 960],
        }
    }
}

#[inline]
fn soft_clip(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.9 { x } else { x.signum() * (0.9 + 0.1 * ((a - 0.9) / 0.1).tanh()) }
}

impl Mixer {
    pub fn push(&mut self, session: SessionId, track_id: u8, seq: u32, flags: u8, payload: Vec<u8>, now: Instant) {
        let channels = if track_id == track::SCREEN_AUDIO { 2 } else { 1 };
        self.peers.entry((session, track_id)).or_insert_with(|| PeerAudio::new(channels)).push(seq, flags, payload, now);
    }

    pub fn remove_session(&mut self, session: SessionId) {
        self.peers.retain(|(s, _), _| *s != session);
        self.volumes.remove(&session);
        self.stream_volumes.remove(&session);
    }

    /// Forget one track (e.g. stream audio after we stop watching).
    pub fn remove_track(&mut self, session: SessionId, track_id: u8) {
        self.peers.remove(&(session, track_id));
    }

    pub fn set_stream_volume(&mut self, session: SessionId, volume: f32) {
        self.stream_volumes.insert(session, volume.clamp(0.0, 3.0));
    }

    pub fn set_volume(&mut self, session: SessionId, volume: f32) {
        self.volumes.insert(session, volume.clamp(0.0, 3.0));
    }

    /// Mix 10 ms into `out` (480 stereo frames, interleaved L/R).
    pub fn mix(&mut self, out: &mut [f32], now: Instant) {
        out[..960].fill(0.0);
        for ((session, track_id), peer) in self.peers.iter_mut() {
            let ch = peer.channels();
            let buf = &mut self.scratch[..480 * ch];
            if !peer.take_10ms(buf, now) || self.deafened {
                continue;
            }
            let vols = if *track_id == track::SCREEN_AUDIO { &self.stream_volumes } else { &self.volumes };
            let gain = self.master * vols.get(session).copied().unwrap_or(1.0);
            if ch == 2 {
                for (o, s) in out[..960].iter_mut().zip(buf.iter()) {
                    *o += s * gain;
                }
            } else {
                for (i, s) in buf.iter().enumerate() {
                    out[2 * i] += s * gain;
                    out[2 * i + 1] += s * gain;
                }
            }
        }
        for v in out[..960].iter_mut() {
            *v = soft_clip(*v);
        }
    }

    /// Sessions whose microphone produced audible audio in the last 250 ms.
    pub fn speaking(&self, now: Instant) -> Vec<SessionId> {
        let mut v: Vec<SessionId> = self
            .peers
            .iter()
            .filter(|((_, t), p)| {
                *t == track::MIC
                    && p.level_db > -55.0
                    && p.last_voice.map(|lv| now.duration_since(lv) < Duration::from_millis(250)).unwrap_or(false)
            })
            .map(|((s, _), _)| *s)
            .collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    pub fn stats(&self) -> JitterStats {
        let mut s = JitterStats::default();
        let mut n = 0.0;
        for p in self.peers.values() {
            s.received += p.stats.received;
            s.late += p.stats.late;
            s.lost += p.stats.lost;
            s.concealed += p.stats.concealed;
            s.fec_recovered += p.stats.fec_recovered;
            s.jitter_ms += p.stats.jitter_ms;
            s.target_ms = s.target_ms.max(p.buffered_ms());
            n += 1.0;
        }
        if n > 0.0 {
            s.jitter_ms /= n;
        }
        s
    }
}
