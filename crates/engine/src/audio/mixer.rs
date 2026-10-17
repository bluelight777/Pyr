//! Mixes every speaker's jitter buffer into one stereo 48 kHz stream.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ingle_proto::control::SessionId;
use ingle_proto::media::track;

use super::jitter::{JitterStats, PeerAudio};
use super::spectrum::{Analyzer, BANDS};
use super::{METER_FRESH, Meter};

/// A soundboard sound playing (48 kHz stereo, interleaved).
struct Effect {
    pcm: Arc<Vec<f32>>,
    pos: usize,
    gain: f32,
    from: SessionId,
}

/// The most soundboard sounds that play at once (the oldest stops for a new one).
const MAX_EFFECTS: usize = 6;

/// What one session sounds like, for its voice meter: everything of theirs mixed in the last
/// 10 ms (their voice, their screen's sound, their soundboard sounds; mono, as they sent it,
/// before my volume for them), and the bands that came of it.
struct Listen {
    block: Vec<f32>,
    heard: bool,
    analyzer: Analyzer,
    bands: [f32; BANDS],
    at: Option<Instant>,
}

impl Default for Listen {
    fn default() -> Self {
        Listen { block: vec![0.0; 480], heard: false, analyzer: Analyzer::new(), bands: [0.0; BANDS], at: None }
    }
}

impl Listen {
    /// 10 ms of theirs (mono, or stereo interleaved), times `gain`.
    fn add(&mut self, x: &[f32], channels: usize, gain: f32) {
        if channels == 2 {
            for (o, pair) in self.block.iter_mut().zip(x.chunks_exact(2)) {
                *o += 0.5 * (pair[0] + pair[1]) * gain;
            }
        } else {
            for (o, v) in self.block.iter_mut().zip(x) {
                *o += v * gain;
            }
        }
        self.heard = true;
    }
}

pub struct Mixer {
    peers: HashMap<(SessionId, u8), PeerAudio>,
    effects: Vec<Effect>,
    /// How loud each session's soundboard sounds were in the last block mixed, and when.
    effect_levels: HashMap<SessionId, (f32, Instant)>,
    /// What each session sounds like, for voice meters.
    listens: HashMap<SessionId, Listen>,
    volumes: HashMap<SessionId, f32>,
    /// Separate volume for someone's stream audio (screen share sound).
    stream_volumes: HashMap<SessionId, f32>,
    pub master: f32,
    pub deafened: bool,
    /// Voice meters are on: what each session sounds like is measured (see `meters`).
    pub meters: bool,
    scratch: Vec<f32>,
}

impl Default for Mixer {
    fn default() -> Self {
        Mixer {
            peers: HashMap::new(),
            effects: Vec::new(),
            effect_levels: HashMap::new(),
            listens: HashMap::new(),
            volumes: HashMap::new(),
            stream_volumes: HashMap::new(),
            master: 1.0,
            deafened: false,
            meters: true,
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
    /// Voice meters on or off (off: what was measured is forgotten, and nothing more is).
    pub fn set_meters(&mut self, on: bool) {
        self.meters = on;
        if !on {
            self.listens.clear();
            self.effect_levels.clear();
        }
    }

    pub fn push(&mut self, session: SessionId, track_id: u8, seq: u32, flags: u8, payload: Vec<u8>, now: Instant) {
        let channels = if track_id == track::SCREEN_AUDIO { 2 } else { 1 };
        self.peers.entry((session, track_id)).or_insert_with(|| PeerAudio::new(channels)).push(seq, flags, payload, now);
    }

    pub fn remove_session(&mut self, session: SessionId) {
        self.peers.retain(|(s, _), _| *s != session);
        self.listens.remove(&session);
        self.effect_levels.remove(&session);
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
        let fresh = |t: Instant| now.saturating_duration_since(t) < METER_FRESH;
        for ((session, track_id), peer) in self.peers.iter_mut() {
            let ch = peer.channels();
            let buf = &mut self.scratch[..480 * ch];
            if !peer.take_10ms(buf, now) {
                continue;
            }
            // For their meter (deafened or not: what they sent, not what I hear).
            if self.meters && peer.last_voice.is_some_and(fresh) {
                self.listens.entry(*session).or_default().add(buf, ch, 1.0);
            }
            if self.deafened {
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
            if n > 0 && self.meters {
                let sum: f32 = e.pcm[e.pos..e.pos + n].iter().map(|v| v * v).sum();
                let db = 20.0 * ((sum / n as f32).sqrt() * e.gain).max(1e-6).log10();
                let lv = self.effect_levels.entry(e.from).or_insert((-100.0, now));
                *lv = (if now.duration_since(lv.1) < Duration::from_millis(5) { lv.0.max(db) } else { db }, now);
                self.listens.entry(e.from).or_default().add(&e.pcm[e.pos..e.pos + n], 2, e.gain);
            }
            e.pos += n;
        }
        self.effects.retain(|e| e.pos < e.pcm.len());
        for v in out[..960].iter_mut() {
            *v = soft_clip(*v);
        }
        // What everyone heard just now sounds like, band by band.
        for l in self.listens.values_mut() {
            if !l.heard {
                continue;
            }
            if !l.at.is_some_and(fresh) {
                // Back after a silence: don't start with what they said last time.
                l.analyzer.clear();
            }
            l.bands = l.analyzer.feed(&l.block);
            l.at = Some(now);
            l.block.fill(0.0);
            l.heard = false;
        }
    }

    /// How loud each session's voice and its stream's sound are right now (dBFS, as sent:
    /// before my volumes), for the call's mixer: (session, track, level), only those heard in
    /// the last [`METER_FRESH`].
    pub fn track_levels(&self, now: Instant) -> Vec<(SessionId, u8, f32)> {
        let fresh = |t: Instant| now.saturating_duration_since(t) < METER_FRESH;
        let mut v: Vec<(SessionId, u8, f32)> =
            self.peers.iter().filter(|(_, p)| p.last_voice.is_some_and(fresh)).map(|((s, t), p)| (*s, *t, p.level_db)).collect();
        v.sort_by_key(|(s, t, _)| (*s, *t));
        v
    }

    /// How each session sounds right now, for voice meters: the loudest of its voice, its
    /// screen's sound (when I'm watching) and its soundboard sounds (dBFS), and the bands of
    /// all of them together. Only sessions heard in the last [`METER_FRESH`].
    pub fn meters(&self, now: Instant) -> Vec<(SessionId, Meter)> {
        let fresh = |t: Instant| now.saturating_duration_since(t) < METER_FRESH;
        let mut out: HashMap<SessionId, Meter> = HashMap::new();
        for ((s, _), p) in &self.peers {
            if p.last_voice.is_some_and(fresh) {
                let e = out.entry(*s).or_default();
                e.db = e.db.max(p.level_db);
            }
        }
        for (s, (db, at)) in &self.effect_levels {
            if fresh(*at) {
                let e = out.entry(*s).or_default();
                e.db = e.db.max(*db);
            }
        }
        for (s, m) in out.iter_mut() {
            if let Some(l) = self.listens.get(s).filter(|l| l.at.is_some_and(fresh)) {
                m.bands = l.bands;
            }
        }
        let mut v: Vec<(SessionId, Meter)> = out.into_iter().collect();
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

    #[test]
    fn a_soundboard_sound_shows_on_its_players_meter_and_then_goes_quiet() {
        let mut m = Mixer::default();
        let t = Instant::now();
        let mut out = vec![0f32; 960];
        // 200 ms of a 300 Hz tone (stereo), played by session 7.
        let tone: Vec<f32> = (0..9600).map(|i| 0.3 * (std::f32::consts::TAU * 300.0 * (i / 2) as f32 / 48_000.0).sin()).collect();
        m.play_effect(Arc::new(tone), 1.0, 7);
        for k in 0..4 {
            m.mix(&mut out, t + Duration::from_millis(10 * k));
        }
        let now = t + Duration::from_millis(30);
        let v = m.meters(now);
        assert_eq!(v.len(), 1, "{v:?}");
        let (s, meter) = v[0];
        assert_eq!(s, 7);
        assert!(meter.db > -20.0, "{meter:?}");
        let loudest = meter.bands.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i).unwrap();
        assert!(loudest <= 4 && meter.bands[loudest] > 0.4, "300 Hz is low: {meter:?}");
        // Quiet a while later.
        assert!(m.meters(now + Duration::from_millis(500)).is_empty());
    }

    #[test]
    fn with_voice_meters_off_nothing_is_measured_but_everything_plays() {
        let mut m = Mixer::default();
        let t = Instant::now();
        let tone: Vec<f32> = (0..9600).map(|i| 0.3 * (std::f32::consts::TAU * 300.0 * (i / 2) as f32 / 48_000.0).sin()).collect();
        m.play_effect(Arc::new(tone.clone()), 1.0, 7);
        let mut out = vec![0f32; 960];
        m.mix(&mut out, t);
        assert!(!m.meters(t).is_empty());
        // Turned off: what was measured goes, and nothing more is; the sound still plays.
        m.set_meters(false);
        assert!(m.meters(t).is_empty());
        for k in 1..4 {
            out.fill(0.0);
            m.mix(&mut out, t + Duration::from_millis(10 * k));
            assert!(out.iter().any(|v| v.abs() > 0.1), "still heard");
        }
        assert!(m.meters(t + Duration::from_millis(30)).is_empty());
        assert!(m.listens.is_empty() && m.effect_levels.is_empty());
        // Back on: measured again.
        m.set_meters(true);
        m.play_effect(Arc::new(tone), 1.0, 7);
        m.mix(&mut out, t + Duration::from_millis(40));
        assert_eq!(m.meters(t + Duration::from_millis(40)).first().map(|(s, _)| *s), Some(7));
    }
}
