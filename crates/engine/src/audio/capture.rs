//! Microphone processing: echo cancellation, noise suppression, gain control,
//! voice activity / push-to-talk gating, and Opus encoding.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use ingle_proto::media::{flags, track};
use sonora::config::{AdaptiveDigital, EchoCanceller, GainController2, HighPassFilter, NoiseSuppressionLevel};

use super::Controls;
use crate::config::{InputMode, NoiseSuppression, Settings};

pub type PacketSink<'a> = &'a mut dyn FnMut(u8, u8, &[u8]);

pub struct CapturePipeline {
    apm: sonora::AudioProcessing,
    denoise: Option<Box<nnnoiseless::DenoiseState<'static>>>,
    /// Voice isolation's noise remover (DeepFilterNet3). RNNoise then only tells when there's
    /// speech; its own cleaned audio isn't used.
    dfn: Option<super::dfn::Dfn>,
    /// The frame as it was before DeepFilterNet (for speech detection on it).
    pre: Vec<f32>,
    /// DeepFilterNet's estimate of the speech-to-noise ratio in the latest frame (dB).
    lsnr: f32,
    /// Voice isolation: detect speech in the cleaned sound (true) or in the microphone's
    /// own (false), and how many 10 ms frames ahead it hears.
    iso_vad_on_clean: bool,
    /// Voice isolation: speech only *starts* when DeepFilterNet also hears the voice standing
    /// out from the noise by this much (dB). RNNoise alone takes some ticks and clacks for
    /// speech; DeepFilterNet doesn't.
    iso_start_lsnr: f32,
    encoder: opus::Encoder,
    acc: Vec<f32>,
    prev: Vec<f32>,
    scratch: Vec<f32>,
    rn_in: Vec<f32>,
    rn_out: Vec<f32>,
    mono_render: Vec<f32>,
    gate_open: bool,
    hang_until: Instant,
    ptt_until: Instant,
    level_db: f32,
    voice_prob: f32,
    /// Speech is going on (RNNoise is confident, or was in the last moment): held through
    /// the quiet parts of words, so nothing is cut in the middle of one.
    speech_until: Instant,
    /// Loudness envelope (dB): jumps up at once, falls slowly, so the quiet parts of a word
    /// (s, f, t, its end) count as loud as the word.
    env_db: f32,
    /// The same envelope of what the microphone picked up, before automatic gain (which
    /// lifts a far voice once you stop talking, so only the raw level says how close it is).
    raw_env_db: f32,
    /// The microphone's level over the last 50 ms (before automatic gain): a voice's
    /// syllable-to-syllable loudness, which tells near from far better than its peaks.
    raw_recent: [f32; 5],
    raw_recent_at: usize,
    /// Voice isolation hears 30 ms ahead: the sound waits here while its fate is decided, so
    /// the start of your first word isn't clipped while it's being recognized.
    iso_delay: std::collections::VecDeque<f32>,
    /// Voice isolation: how loud your own voice usually is at the mic (dB), the gain being
    /// applied, and until when the voice going on is yours (close to the mic, and speech).
    iso_ref_db: f32,
    iso_gain: f32,
    mine_until: Instant,
    settings: Settings,
    pub stream_delay_ms: i32,
    pub sent: u64,
    encoded: Vec<u8>,
}

/// How long speech counts as going on after RNNoise last heard it (bridges the quiet parts of
/// words and the gaps between them).
const SPEECH_HOLD_MS: u64 = 350;
/// Voice isolation: speech must be within this many dB of your usual level to be taken as
/// you, and once it is, stays you down to this many dB (people trail off within sentences).
const ISO_START_DB: f32 = 12.0;
const ISO_KEEP_DB: f32 = 20.0;
/// Voice isolation: whether speech is detected in DeepFilterNet's cleaned sound, and the
/// look-ahead in 10 ms frames (see `tune_isolation`). Detecting it in the microphone's own
/// sound needs no look-ahead of its own: DeepFilterNet's output is 30 ms behind its input, so
/// the decision is already that far ahead of the sound it applies to. (Measured on speech
/// over 10 kinds of mechanical noise: same quality either way, 30 ms less delay this way.)
const ISO_DEFAULT_TUNING: (bool, usize) = (false, 0);
/// See `iso_start_lsnr`.
const ISO_START_LSNR: f32 = 3.0;
/// Voice isolation: how much is let through of other people's voices, and of other sounds.
const ISO_OTHER_VOICE: f32 = 0.05;
const ISO_FLOOR: f32 = 0.03;

fn rms_db(x: &[f32]) -> f32 {
    let s = x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32;
    if s > 1e-10 { 10.0 * s.log10() } else { -100.0 }
}

impl CapturePipeline {
    pub fn new(settings: &Settings) -> anyhow::Result<CapturePipeline> {
        let config = sonora::Config {
            echo_canceller: settings.echo_cancellation.then(EchoCanceller::default),
            noise_suppression: (settings.noise_suppression == NoiseSuppression::Standard).then(|| {
                sonora::config::NoiseSuppression { level: NoiseSuppressionLevel::High, ..Default::default() }
            }),
            gain_controller2: settings.auto_gain.then(|| GainController2 {
                adaptive_digital: Some(AdaptiveDigital::default()),
                ..Default::default()
            }),
            high_pass_filter: Some(HighPassFilter::default()),
            ..Default::default()
        };
        let apm = sonora::AudioProcessing::builder()
            .config(config)
            .capture_config(sonora::StreamConfig::new(48_000, 1))
            .render_config(sonora::StreamConfig::new(48_000, 1))
            .build();
        let mut encoder = opus::Encoder::new(48_000, opus::Channels::Mono, opus::Application::Voip)?;
        encoder.set_bitrate(opus::Bitrate::Bits((settings.voice_bitrate_kbps.clamp(16, 256) * 1000) as i32))?;
        encoder.set_vbr(true)?;
        encoder.set_inband_fec(true)?;
        encoder.set_packet_loss_perc(10)?;
        encoder.set_complexity(10)?;
        let denoise = matches!(settings.noise_suppression, NoiseSuppression::Strong | NoiseSuppression::Isolation)
            .then(nnnoiseless::DenoiseState::new);
        let dfn = if settings.noise_suppression == NoiseSuppression::Isolation {
            match super::dfn::Dfn::new() {
                Ok(d) => Some(d),
                Err(e) => {
                    // Still isolates voices, with RNNoise's cleaning instead.
                    tracing::warn!("voice isolation without DeepFilterNet: {e}");
                    None
                }
            }
        } else {
            None
        };
        let (iso_vad_on_clean, iso_lookahead) = ISO_DEFAULT_TUNING;
        let now = Instant::now();
        Ok(CapturePipeline {
            apm,
            denoise,
            dfn,
            pre: vec![0.0; 480],
            lsnr: 0.0,
            iso_vad_on_clean,
            iso_start_lsnr: ISO_START_LSNR,
            encoder,
            acc: Vec::with_capacity(960),
            prev: vec![0.0; 960],
            scratch: vec![0.0; 480],
            rn_in: vec![0.0; 480],
            rn_out: vec![0.0; 480],
            mono_render: vec![0.0; 480],
            gate_open: false,
            hang_until: now,
            ptt_until: now,
            level_db: -100.0,
            voice_prob: 0.0,
            speech_until: now,
            env_db: -100.0,
            raw_env_db: -100.0,
            raw_recent: [0.0; 5],
            raw_recent_at: 0,
            iso_delay: std::iter::repeat_n(0.0, iso_lookahead * 480).collect(),
            iso_ref_db: -100.0,
            iso_gain: 1.0,
            mine_until: now,
            settings: settings.clone(),
            stream_delay_ms: 60,
            sent: 0,
            encoded: vec![0u8; 1500],
        })
    }

    /// For measuring tools: voice isolation's speech detection on the cleaned sound or the
    /// microphone's own, and its look-ahead in 10 ms frames.
    #[doc(hidden)]
    pub fn tune_isolation(&mut self, vad_on_clean: bool, lookahead_frames: usize, start_lsnr: f32) {
        self.iso_vad_on_clean = vad_on_clean;
        self.iso_start_lsnr = start_lsnr;
        self.iso_delay = std::iter::repeat_n(0.0, lookahead_frames * 480).collect();
    }

    /// For measuring tools: voice isolation as it was before DeepFilterNet.
    #[doc(hidden)]
    pub fn without_dfn(&mut self) {
        self.dfn = None;
    }

    /// Whether voice isolation runs DeepFilterNet (it falls back to RNNoise if the model
    /// couldn't load).
    pub fn has_dfn(&self) -> bool {
        self.dfn.is_some()
    }

    /// For measuring tools: speech probability, raw envelope, learned level of your voice,
    /// isolation gain.
    pub fn debug_state(&self) -> (f32, f32, f32, f32) {
        (self.voice_prob, self.raw_env_db, self.iso_ref_db, self.iso_gain)
    }

    /// For measuring tools: DeepFilterNet's speech-to-noise estimate (dB).
    #[doc(hidden)]
    pub fn debug_lsnr(&self) -> f32 {
        self.lsnr
    }

    pub fn level_db(&self) -> f32 {
        self.level_db
    }

    /// Feed what the speakers are about to play (stereo, 480 frames) for echo cancellation.
    pub fn render(&mut self, stereo: &[f32]) {
        if !self.settings.echo_cancellation {
            return;
        }
        for i in 0..480 {
            self.mono_render[i] = 0.5 * (stereo[2 * i] + stereo[2 * i + 1]);
        }
        let src = self.mono_render.clone();
        let _ = self.apm.process_render_f32(&[&src], &mut [&mut self.mono_render]);
    }

    fn gate(&mut self, now: Instant, ctl: &Controls) -> bool {
        // (Deafening yourself sets `muted` too; a moderator's deafen leaves you able to talk.)
        if ctl.muted.load(Ordering::Relaxed) {
            return false;
        }
        match self.settings.input_mode {
            InputMode::PushToTalk => {
                if ctl.ptt_down.load(Ordering::Relaxed) {
                    self.ptt_until = now + Duration::from_millis(self.settings.ptt_release_ms as u64);
                }
                now < self.ptt_until
            }
            InputMode::VoiceActivity => {
                let voice = if self.settings.vad_auto {
                    if self.settings.noise_suppression == NoiseSuppression::Isolation {
                        // Your voice (see `isolate`), already held through your words.
                        now < self.mine_until
                    } else if self.denoise.is_some() {
                        // RNNoise's speech detection, held through words (see `track_speech`).
                        now < self.speech_until && self.env_db > -60.0
                    } else {
                        self.level_db > -48.0
                    }
                } else {
                    self.level_db > self.settings.vad_threshold_db
                };
                if voice {
                    self.hang_until = now + Duration::from_millis(400);
                }
                now < self.hang_until
            }
        }
    }

    /// After RNNoise: follow the loudness envelope and whether speech is going on. RNNoise's
    /// speech probability is per 10 ms and dips inside words (on consonants, at their ends),
    /// so it starts speech only when confident and then a lower bar keeps it going, with a
    /// hold that bridges the dips; the envelope jumps up at once and falls slowly
    /// (about 6 dB per 100 ms) for the same reason.
    fn track_speech(&mut self, now: Instant, db: f32, raw_db: f32) {
        self.env_db = if db > self.env_db { db } else { (self.env_db - 0.6).max(db) };
        self.raw_env_db = if raw_db > self.raw_env_db { raw_db } else { (self.raw_env_db - 0.6).max(raw_db) };
        self.raw_recent[self.raw_recent_at] = 10f32.powf(raw_db / 10.0);
        self.raw_recent_at = (self.raw_recent_at + 1) % self.raw_recent.len();
        let going = now < self.speech_until;
        let starts = self.voice_prob > 0.6 && (self.dfn.is_none() || self.lsnr > self.iso_start_lsnr);
        if starts || (going && self.voice_prob > 0.2) {
            self.speech_until = now + Duration::from_millis(SPEECH_HOLD_MS);
        }
    }

    /// Voice isolation: keep your voice and turn everything else far down. A voice across the
    /// room or on a TV reaches your mic 15+ dB quieter than yours, so its level tells them
    /// apart; RNNoise's speech detection tells speech from other sounds. Decided per stretch
    /// of speech (held, see `track_speech`), never per slice of a word, so your words come
    /// through whole.
    fn isolate(&mut self, now: Instant, frame: &mut [f32]) {
        let speech = now < self.speech_until;
        let e = self.raw_recent.iter().sum::<f32>() / self.raw_recent.len() as f32;
        let level = if e > 1e-10 { 10.0 * e.log10() } else { -100.0 };
        // Learn your level from confident speech, gently: it rises toward a louder voice and
        // sinks only with speech close to it (you talking more softly), never toward a quieter
        // voice further away, or it would end up learning the other person.
        if self.voice_prob > 0.8 && level > -70.0 {
            if level > self.iso_ref_db {
                self.iso_ref_db += (level - self.iso_ref_db) * if self.iso_ref_db < -90.0 { 1.0 } else { 0.05 };
            } else if level > self.iso_ref_db - 8.0 {
                self.iso_ref_db += (level - self.iso_ref_db) * 0.01;
            }
        }
        // Becoming "you" takes speech nearly as loud as you; staying "you" through the softer
        // parts of your sentences takes much less (and the hold bridges the gaps).
        let mine = now < self.mine_until;
        let near = level > self.iso_ref_db - if mine { ISO_KEEP_DB } else { ISO_START_DB };
        if speech && near && (self.voice_prob > 0.5 || mine) {
            self.mine_until = now + Duration::from_millis(SPEECH_HOLD_MS);
        }
        let target: f32 = if now < self.mine_until {
            1.0
        } else if speech {
            ISO_OTHER_VOICE
        } else {
            ISO_FLOOR
        };
        // The decision applies to the sound from 30 ms ago (see `iso_delay`).
        self.iso_delay.extend(frame.iter().copied());
        for v in frame.iter_mut() {
            *v = self.iso_delay.pop_front().unwrap_or(0.0);
        }
        // Open at once (within these 10 ms, so it doesn't click); close over about 120 ms.
        let start = self.iso_gain;
        let end = if target >= start { target } else { (start * 0.7).max(target) };
        let n = frame.len() as f32;
        for (i, v) in frame.iter_mut().enumerate() {
            *v *= start + (end - start) * (i as f32 / n);
        }
        self.iso_gain = end;
    }

    /// Process 10 ms of 48 kHz mono microphone audio.
    pub fn process(&mut self, frame: &mut [f32], ctl: &Controls, now: Instant, sink: PacketSink<'_>) {
        let gain = self.settings.input_gain;
        if (gain - 1.0).abs() > 1e-3 {
            for v in frame.iter_mut() {
                *v *= gain;
            }
        }
        if self.settings.echo_cancellation {
            let _ = self.apm.set_stream_delay_ms(self.stream_delay_ms);
        }
        let raw_db = rms_db(&frame[..480]);
        self.scratch.copy_from_slice(&frame[..480]);
        let _ = self.apm.process_capture_f32(&[&self.scratch], &mut [&mut frame[..480]]);

        if let Some(d) = self.dfn.as_mut() {
            // Voice isolation: DeepFilterNet cleans; RNNoise only listens for speech.
            self.pre.copy_from_slice(&frame[..480]);
            self.lsnr = d.process(&mut frame[..480]);
            let src: &[f32] = if self.iso_vad_on_clean { &frame[..480] } else { &self.pre };
            if let Some(dn) = self.denoise.as_mut() {
                for i in 0..480 {
                    self.rn_in[i] = src[i] * 32768.0;
                }
                self.voice_prob = dn.process_frame(&mut self.rn_out, &self.rn_in);
            }
        } else if let Some(dn) = self.denoise.as_mut() {
            for i in 0..480 {
                self.rn_in[i] = frame[i] * 32768.0;
            }
            self.voice_prob = dn.process_frame(&mut self.rn_out, &self.rn_in);
            for i in 0..480 {
                frame[i] = self.rn_out[i] / 32768.0;
            }
        }
        let db = rms_db(&frame[..480]);
        if self.denoise.is_some() {
            self.track_speech(now, db, raw_db);
        }
        if self.settings.noise_suppression == NoiseSuppression::Isolation {
            self.isolate(now, &mut frame[..480]);
        }
        // Fast attack, slower release, for a pleasant meter.
        self.level_db = if db > self.level_db { db } else { self.level_db * 0.85 + db * 0.15 };
        ctl.mic_level.store(self.level_db.to_bits(), Ordering::Relaxed);

        let open = self.gate(now, ctl);
        ctl.transmitting.store(open, Ordering::Relaxed);
        // Something said (not just an open microphone in a quiet room): I'm here.
        if open && self.level_db > -50.0 {
            ctl.last_spoke_ms.store(ingle_proto::now_ms(), Ordering::Relaxed);
        }
        self.acc.extend_from_slice(&frame[..480]);
        if self.acc.len() < 960 {
            return;
        }
        let pcm = std::mem::take(&mut self.acc);
        if open {
            let mut f = 0u8;
            if !self.gate_open {
                // Start of a talkspurt: send the previous 20 ms too so first syllables aren't clipped.
                let prev = self.prev.clone();
                if let Ok(n) = self.encoder.encode_float(&prev, &mut self.encoded) {
                    sink(track::MIC, flags::TALKSPURT, &self.encoded[..n]);
                    self.sent += 1;
                }
                f = 0;
            }
            if let Ok(n) = self.encoder.encode_float(&pcm, &mut self.encoded) {
                sink(track::MIC, f, &self.encoded[..n]);
                self.sent += 1;
            }
        }
        self.gate_open = open;
        self.prev.copy_from_slice(&pcm);
        self.acc = pcm;
        self.acc.clear();
    }
}
