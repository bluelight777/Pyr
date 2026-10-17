//! Per-speaker adaptive jitter buffer with Opus decoding, in-band FEC recovery and
//! packet-loss concealment.

use std::collections::{BTreeMap, VecDeque};
use std::time::Instant;

use ingle_proto::media::flags;

pub const FRAME_MS: f32 = 20.0;
/// Samples per channel in one 20 ms Opus frame at 48 kHz.
pub const FRAME: usize = 960;

const MIN_TARGET: usize = 2; // 40 ms
const MAX_TARGET: usize = 10; // 200 ms
const MAX_PACKETS: usize = 64;
/// Queued frames beyond the target at which we drop the backlog at once (~0.5 s).
const SNAP_EXCESS: usize = 25;
/// Longest run of missing frames we conceal; beyond that we jump to the next packet.
const MAX_CONCEAL_GAP: u32 = 4;

#[derive(Default, Clone, Copy, Debug)]
pub struct JitterStats {
    pub received: u64,
    pub late: u64,
    pub lost: u64,
    pub concealed: u64,
    pub fec_recovered: u64,
    pub jitter_ms: f32,
    pub target_ms: u32,
}

pub struct PeerAudio {
    decoder: opus::Decoder,
    channels: usize,
    packets: BTreeMap<u32, Vec<u8>>,
    next_seq: Option<u32>,
    playing: bool,
    /// Decoded interleaved samples waiting to be mixed.
    pcm: VecDeque<f32>,
    missing_run: u32,
    jitter_ms: f32,
    last_arrival: Option<(Instant, u32)>,
    buffering_since: Option<Instant>,
    last_trim: Instant,
    pub stats: JitterStats,
    /// Level (RMS dBFS) of the last real (non-concealed) 10 ms, for speaking indicators.
    pub level_db: f32,
    pub last_voice: Option<Instant>,
    scratch: Vec<f32>,
}

impl PeerAudio {
    pub fn new(channels: usize) -> PeerAudio {
        let ch = if channels == 2 { opus::Channels::Stereo } else { opus::Channels::Mono };
        PeerAudio {
            decoder: opus::Decoder::new(48_000, ch).expect("opus decoder"),
            channels: if channels == 2 { 2 } else { 1 },
            packets: BTreeMap::new(),
            next_seq: None,
            playing: false,
            pcm: VecDeque::with_capacity(FRAME * 4),
            missing_run: 0,
            jitter_ms: 0.0,
            last_arrival: None,
            buffering_since: None,
            last_trim: Instant::now(),
            stats: JitterStats::default(),
            level_db: -100.0,
            last_voice: None,
            scratch: vec![0.0; FRAME * 2 * 6],
        }
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    fn target_frames(&self) -> usize {
        // Hold roughly 3x the measured jitter plus one frame of headroom.
        let frames = ((self.jitter_ms * 3.0) / FRAME_MS).ceil() as usize + 1;
        frames.clamp(MIN_TARGET, MAX_TARGET)
    }

    pub fn push(&mut self, seq: u32, pkt_flags: u8, payload: Vec<u8>, now: Instant) {
        self.stats.received += 1;
        let talkspurt = pkt_flags & flags::TALKSPURT != 0 && !ingle_proto::ab::old("jitter");
        if talkspurt {
            // Someone starts talking again. The time since their last packet is the pause, not
            // the network (nothing is sent while they're quiet, so sequence numbers don't move),
            // and the packet after this one comes right with it (this one is the 20 ms before
            // they started, see `CapturePipeline::process`): neither says anything about jitter.
            // (Counting the pause made every sentence after one start up to 100 ms late.)
            self.last_arrival = None;
        } else if let Some((t0, s0)) = self.last_arrival {
            if seq > s0 {
                let expected = (seq - s0) as f32 * FRAME_MS;
                let actual = now.duration_since(t0).as_secs_f32() * 1000.0;
                let d = (actual - expected).abs().min(500.0);
                self.jitter_ms += (d - self.jitter_ms) / 16.0;
            }
        }
        if !talkspurt && self.last_arrival.map(|(_, s)| seq > s).unwrap_or(true) {
            self.last_arrival = Some((now, seq));
        }
        if pkt_flags & flags::TALKSPURT != 0 && self.packets.is_empty() && self.missing_run > 0 {
            // New talkspurt after silence: re-buffer from here.
            self.playing = false;
            self.next_seq = None;
        }
        if let Some(n) = self.next_seq {
            if seq < n {
                self.stats.late += 1;
                return;
            }
        }
        self.packets.insert(seq, payload);
        while self.packets.len() > MAX_PACKETS {
            let first = *self.packets.keys().next().expect("non-empty");
            self.packets.remove(&first);
            // Playback fell far behind (e.g. the audio thread was starved): don't crawl
            // through the evicted range with concealment, carry on from what we still have.
            if let Some(n) = self.next_seq {
                if n <= first {
                    self.next_seq = Some(first + 1);
                }
            }
        }
        self.stats.jitter_ms = self.jitter_ms;
    }

    fn decode_into_pcm(&mut self, packet: Option<&[u8]>, fec: bool) -> bool {
        let n = FRAME * self.channels;
        let out = &mut self.scratch[..n * 6];
        let res = match packet {
            Some(p) => self.decoder.decode_float(p, &mut out[..if fec { n } else { n * 6 }], fec),
            None => self.decoder.decode_float(&[], &mut out[..n], false),
        };
        match res {
            Ok(samples) => {
                let total = samples * self.channels;
                self.pcm.extend(out[..total].iter().copied());
                true
            }
            Err(_) => {
                self.pcm.extend(std::iter::repeat_n(0.0, n));
                false
            }
        }
    }

    /// Produce one more 20 ms of audio into the PCM queue (or nothing while buffering).
    fn pull_frame(&mut self, now: Instant) -> bool {
        if !self.playing {
            if self.packets.is_empty() {
                self.buffering_since = None;
                return false;
            }
            let since = *self.buffering_since.get_or_insert(now);
            let waited_ms = now.duration_since(since).as_millis() as f32;
            let target = self.target_frames();
            if self.packets.len() < target && waited_ms < target as f32 * FRAME_MS {
                return false;
            }
            self.playing = true;
            self.buffering_since = None;
            self.next_seq = self.packets.keys().next().copied();
            self.missing_run = 0;
        }
        let Some(mut seq) = self.next_seq else { return false };
        let target = self.target_frames();

        // Far behind (over half a second more than we want): snap back to the newest audio
        // in one jump instead of playing catch-up for seconds.
        if self.packets.len() > target + SNAP_EXCESS {
            while self.packets.len() > target + 2 {
                let first = *self.packets.keys().next().expect("non-empty");
                self.packets.remove(&first);
            }
            seq = seq.max(*self.packets.keys().next().expect("non-empty"));
            self.next_seq = Some(seq);
            self.stats.lost += 1;
        }
        // A hole of several frames before the oldest packet we hold (a loss burst, or
        // packets dropped while we were stalled): skip it rather than conceal it all.
        if !self.packets.contains_key(&seq) {
            if let Some(&first) = self.packets.keys().next() {
                if first > seq + MAX_CONCEAL_GAP {
                    self.stats.lost += 1;
                    seq = first;
                    self.next_seq = Some(seq);
                }
            }
        }

        // Latency trimming: if too much audio is queued (e.g. after a network burst or a
        // CPU stall), skip packets — gently when slightly over target, quickly when far over.
        let excess = self.packets.len().saturating_sub(target);
        let interval_ms = if excess > 8 { 40 } else if excess > 3 { 400 } else { u128::MAX };
        if now.duration_since(self.last_trim).as_millis() > interval_ms {
            self.packets.remove(&seq);
            self.next_seq = Some(seq + 1);
            self.last_trim = now;
            return self.pull_frame(now);
        }

        if let Some(p) = self.packets.remove(&seq) {
            self.decode_into_pcm(Some(&p), false);
            self.missing_run = 0;
            self.mark_voice(now);
        } else if let Some(next) = self.packets.get(&(seq + 1)).cloned() {
            // Recover the lost frame from the next packet's in-band FEC data.
            self.decode_into_pcm(Some(&next), true);
            self.stats.fec_recovered += 1;
            self.stats.lost += 1;
            self.missing_run = 0;
        } else {
            self.decode_into_pcm(None, false);
            self.stats.concealed += 1;
            self.missing_run += 1;
            if self.missing_run >= 5 && self.packets.is_empty() {
                // The speaker paused (or the stream died): stop and re-buffer later.
                self.playing = false;
                self.next_seq = None;
                self.stats.target_ms = (self.target_frames() as f32 * FRAME_MS) as u32;
                return true;
            }
            if self.missing_run == 1 && !self.packets.is_empty() {
                self.stats.lost += 1;
            }
        }
        self.next_seq = Some(seq + 1);
        self.stats.target_ms = (self.target_frames() as f32 * FRAME_MS) as u32;
        true
    }

    fn mark_voice(&mut self, now: Instant) {
        self.last_voice = Some(now);
    }

    /// Take 10 ms (480 frames) of interleaved audio, decoding as needed. Returns false
    /// (and leaves `out` untouched) when there is nothing to play.
    pub fn take_10ms(&mut self, out: &mut [f32], now: Instant) -> bool {
        let need = 480 * self.channels;
        while self.pcm.len() < need {
            if !self.pull_frame(now) {
                break;
            }
        }
        if self.pcm.len() < need {
            if !self.pcm.is_empty() && !self.playing {
                // Tail of a talkspurt: play what's left, padded with silence.
                let n = self.pcm.len();
                for (i, v) in self.pcm.drain(..).enumerate() {
                    out[i] = v;
                }
                out[n..need].fill(0.0);
                return true;
            }
            return false;
        }
        let mut sum = 0f32;
        for (i, v) in self.pcm.drain(..need).enumerate() {
            out[i] = v;
            sum += v * v;
        }
        let rms = (sum / need as f32).sqrt();
        self.level_db = if rms > 1e-6 { 20.0 * rms.log10() } else { -100.0 };
        true
    }

    pub fn buffered_ms(&self) -> u32 {
        (self.packets.len() as f32 * FRAME_MS) as u32 + (self.pcm.len() / self.channels / 48) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn encoder() -> opus::Encoder {
        let mut e = opus::Encoder::new(48_000, opus::Channels::Mono, opus::Application::Voip).unwrap();
        e.set_inband_fec(true).unwrap();
        e.set_packet_loss_perc(20).unwrap();
        e
    }

    fn tone_frame(start: usize) -> Vec<f32> {
        (0..FRAME).map(|i| ((start + i) as f32 / 48_000.0 * 440.0 * std::f32::consts::TAU).sin() * 0.3).collect()
    }

    #[test]
    fn plays_through_loss_with_fec_and_plc() {
        let mut enc = encoder();
        let mut peer = PeerAudio::new(1);
        let t0 = Instant::now();
        let mut out = vec![0f32; 480];
        let mut played = 0;
        for seq in 0..100u32 {
            let pcm = tone_frame(seq as usize * FRAME);
            let pkt = enc.encode_vec_float(&pcm, 1500).unwrap();
            let lost = seq % 10 == 5; // 10% loss
            let now = t0 + Duration::from_millis(seq as u64 * 20);
            if !lost {
                peer.push(seq, if seq == 0 { flags::TALKSPURT } else { 0 }, pkt, now);
            }
            for half in 0..2 {
                if peer.take_10ms(&mut out, now + Duration::from_millis(half * 10)) {
                    played += 1;
                }
            }
        }
        assert!(played > 180, "played {played} of 200 10ms blocks");
        assert!(peer.stats.fec_recovered >= 8, "fec {}", peer.stats.fec_recovered);
        assert!(peer.level_db > -20.0, "level {}", peer.level_db);
    }

    /// After the audio thread stalls for seconds, latency must come back down at once.
    #[test]
    fn recovers_quickly_after_a_stall() {
        let mut enc = encoder();
        let mut peer = PeerAudio::new(1);
        let t0 = Instant::now();
        let mut out = vec![0f32; 480];
        // Normal playback for a second.
        let mut seq = 0u32;
        for _ in 0..50 {
            let now = t0 + Duration::from_millis(seq as u64 * 20);
            peer.push(seq, if seq == 0 { flags::TALKSPURT } else { 0 }, enc.encode_vec_float(&tone_frame(0), 1500).unwrap(), now);
            peer.take_10ms(&mut out, now);
            peer.take_10ms(&mut out, now + Duration::from_millis(10));
            seq += 1;
        }
        // Stall: 3 s of packets arrive while nothing is played.
        for _ in 0..150 {
            let now = t0 + Duration::from_millis(seq as u64 * 20);
            peer.push(seq, 0, enc.encode_vec_float(&tone_frame(0), 1500).unwrap(), now);
            seq += 1;
        }
        let now = t0 + Duration::from_millis(seq as u64 * 20);
        assert!(peer.take_10ms(&mut out, now));
        let target_ms = peer.target_frames() as u32 * 20;
        assert!(peer.buffered_ms() <= target_ms + 60, "still {} ms queued (target {target_ms})", peer.buffered_ms());
        // And it keeps playing the newest audio.
        assert!(peer.next_seq.unwrap() + (peer.packets.len() as u32) >= seq - 1);
    }

    /// A burst of lost packets is skipped instead of concealed frame by frame.
    #[test]
    fn long_loss_burst_is_skipped() {
        let mut enc = encoder();
        let mut peer = PeerAudio::new(1);
        let t0 = Instant::now();
        let mut out = vec![0f32; 480];
        for seq in 0..10u32 {
            let now = t0 + Duration::from_millis(seq as u64 * 20);
            peer.push(seq, if seq == 0 { flags::TALKSPURT } else { 0 }, enc.encode_vec_float(&tone_frame(0), 1500).unwrap(), now);
            peer.take_10ms(&mut out, now);
            peer.take_10ms(&mut out, now + Duration::from_millis(10));
        }
        let before = peer.stats.concealed;
        // 10 packets lost, then the stream continues.
        for seq in 20..30u32 {
            let now = t0 + Duration::from_millis(seq as u64 * 20);
            peer.push(seq, 0, enc.encode_vec_float(&tone_frame(0), 1500).unwrap(), now);
            peer.take_10ms(&mut out, now);
            peer.take_10ms(&mut out, now + Duration::from_millis(10));
        }
        assert!(peer.stats.concealed - before <= 6, "concealed {} frames", peer.stats.concealed - before);
        assert!(peer.next_seq.unwrap() >= 25, "caught up to the live stream");
    }

    /// Talking, a pause, talking again (the way the microphone sends it: nothing during the
    /// pause, and the 20 ms before the new words sent along with them): the pause isn't
    /// jitter, so the new words play at once instead of waiting for a bigger buffer.
    #[test]
    fn a_pause_in_talking_is_not_jitter() {
        let mut enc = encoder();
        let mut peer = PeerAudio::new(1);
        let t0 = Instant::now();
        let mut out = vec![0f32; 480];
        let mut seq = 0u32;
        let mut ms = 0u64;
        let mut started_after = Vec::new();
        for spurt in 0..4 {
            // The start of the words: the 20 ms before, and the first 20 ms, together.
            let at = t0 + Duration::from_millis(ms);
            peer.push(seq, flags::TALKSPURT, enc.encode_vec_float(&tone_frame(0), 1500).unwrap(), at);
            peer.push(seq + 1, 0, enc.encode_vec_float(&tone_frame(0), 1500).unwrap(), at);
            seq += 2;
            let mut first_sound = None;
            // Half a second of talking, then a 2 s pause.
            for i in 0..50u64 {
                let now = at + Duration::from_millis(i * 10);
                if i % 2 == 0 && i > 0 {
                    peer.push(seq, 0, enc.encode_vec_float(&tone_frame(0), 1500).unwrap(), now);
                    seq += 1;
                }
                if peer.take_10ms(&mut out, now) && first_sound.is_none() {
                    first_sound = Some(i * 10);
                }
            }
            for i in 50..250u64 {
                peer.take_10ms(&mut out, at + Duration::from_millis(i * 10));
            }
            ms += 2500;
            if spurt > 0 {
                started_after.push(first_sound.unwrap());
            }
        }
        assert!(started_after.iter().all(|&m| m == 0), "words after a pause played after {started_after:?} ms");
        assert!(peer.stats.jitter_ms < 5.0, "jitter {} ms on a steady stream", peer.stats.jitter_ms);
        assert_eq!(peer.target_frames(), MIN_TARGET);
    }

    #[test]
    fn late_packets_are_dropped_and_buffer_adapts() {
        let mut enc = encoder();
        let mut peer = PeerAudio::new(1);
        let t0 = Instant::now();
        let mut out = vec![0f32; 480];
        for seq in 0..50u32 {
            let pkt = enc.encode_vec_float(&tone_frame(0), 1500).unwrap();
            // heavy jitter: every other packet 60 ms late
            let delay = if seq % 2 == 0 { 0 } else { 60 };
            peer.push(seq, 0, pkt, t0 + Duration::from_millis(seq as u64 * 20 + delay));
            let _ = peer.take_10ms(&mut out, t0 + Duration::from_millis(seq as u64 * 20));
        }
        assert!(peer.stats.jitter_ms > 10.0);
        assert!(peer.target_frames() > MIN_TARGET);
    }
}
