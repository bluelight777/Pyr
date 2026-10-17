//! Mixes every speaker's jitter buffer into one stereo 48 kHz stream.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ingle_proto::control::SessionId;
use ingle_proto::media::track;

use super::jitter::{JitterStats, PeerAudio};

/// A soundboard sound playing (48 kHz stereo, interleaved).
struct Effect {
    pcm: Arc<Vec<f32>>,
    pos: usize,
    gain: f32,
    from: SessionId,
}

/// The most soundboard sounds that play at once (the oldest stops for a new one).
const MAX_EFFECTS: usize = 6;

pub struct Mixer {
    peers: HashMap<(SessionId, u8), PeerAudio>,
    effects: Vec<Effect>,
    /// How loud each session's soundboard sounds were in the last block mixed, and when.
    effect_levels: HashMap<SessionId, (f32, Instant)>,
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
            effects: Vec::new(),
            effect_levels: HashMap::new(),
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

    /// Play a soundboard sound (someone's in the call, or a preview: `from` is who played it).
    pub fn play_effect(&mut self, pcm: Arc<Vec<f32>>, gain: f32, from: SessionId) {
        if self.effects.len() >= MAX_EFFECTS {
            self.effects.remove(0);
        }
        self.effects.push(Effect { pcm, pos: 0, gain, from });
    }

    /// Stop the soundboard sounds playing (all, or one person's).
    pub fn stop_effects(&mut self, from: Option<SessionId>) {
        self.effects.retain(|e| from.is_some_and(|f| e.from != f));
    }

    pub fn effects_playing(&self) -> bool {
        !self.effects.is_empty()
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
        // Soundboard sounds (not while deafened: those are lost, like everyone's voice).
        for e in self.effects.iter_mut() {
            let n = (e.pcm.len() - e.pos).min(960);
            if !self.deafened {
                let g = e.gain * self.master;
                for (o, v) in out[..n].iter_mut().zip(&e.pcm[e.pos..e.pos + n]) {
                    *o += v * g;
                }
            }
            // How loud it is (as sent, before anyone's own volume), for its player's meter.
            if n > 0 {
                let sum: f32 = e.pcm[e.pos..e.pos + n].iter().map(|v| v * v).sum();
                let db = 20.0 * ((sum / n as f32).sqrt() * e.gain).max(1e-6).log10();
                let lv = self.effect_levels.entry(e.from).or_insert((-100.0, now));
                *lv = (if now.duration_since(lv.1) < Duration::from_millis(5) { lv.0.max(db) } else { db }, now);
            }
            e.pos += n;
        }
        self.effects.retain(|e| e.pos < e.pcm.len());
        for v in out[..960].iter_mut() {
            *v = soft_clip(*v);
        }
    }

    /// How loud each session sounds right now (dBFS): the loudest of its voice, its screen's
    /// sound and its soundboard sounds, for voice meters. Silent after 120 ms without any.
    pub fn levels(&self, now: Instant) -> Vec<(SessionId, f32)> {
        let fresh = |t: Instant| now.saturating_duration_since(t) < Duration::from_millis(120);
        let mut out: HashMap<SessionId, f32> = HashMap::new();
        for ((s, _), p) in &self.peers {
            if p.last_voice.is_some_and(fresh) {
                let e = out.entry(*s).or_insert(-100.0);
                *e = e.max(p.level_db);
            }
        }
        for (s, (db, at)) in &self.effect_levels {
            if fresh(*at) {
                let e = out.entry(*s).or_insert(-100.0);
                *e = e.max(*db);
            }
        }
        let mut v: Vec<(SessionId, f32)> = out.into_iter().collect();
        v.sort_unstable_by_key(|(s, _)| *s);
        v
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soundboard_sounds_mix_in_and_stop() {
        let mut m = Mixer::default();
        let t = Instant::now();
        let mut out = vec![0f32; 960];
        // 25 ms of a steady level: two full 10 ms blocks and a bit of a third.
        m.play_effect(Arc::new(vec![0.25; 2400]), 0.5, 7);
        m.mix(&mut out, t);
        assert!((out[0] - 0.125).abs() < 1e-6 && (out[959] - 0.125).abs() < 1e-6);
        m.mix(&mut out, t);
        m.mix(&mut out, t);
        assert!((out[0] - 0.125).abs() < 1e-6 && out[480].abs() < 1e-6, "the end of it, then silence");
        assert!(!m.effects_playing());

        // Deafened: not heard. Stopped: gone.
        m.play_effect(Arc::new(vec![0.25; 9600]), 1.0, 7);
        m.play_effect(Arc::new(vec![0.25; 9600]), 1.0, 8);
        m.deafened = true;
        m.mix(&mut out, t);
        assert!(out.iter().all(|v| *v == 0.0));
        m.deafened = false;
        m.stop_effects(Some(7));
        m.mix(&mut out, t);
        assert!((out[0] - 0.25).abs() < 1e-6, "only 8's is left");
        m.stop_effects(None);
        assert!(!m.effects_playing());
    }
}
