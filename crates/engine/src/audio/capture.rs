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
    /// Voice isolation: how loud your own voice usually is at the mic (dB), the gain being
    /// applied, and whether this frame sounds like you (close and speech).
    iso_ref_db: f32,
    iso_gain: f32,
    iso_near: bool,
    settings: Settings,
    pub stream_delay_ms: i32,
    pub sent: u64,
    encoded: Vec<u8>,
}

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
        let now = Instant::now();
        Ok(CapturePipeline {
            apm,
            denoise,
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
            iso_ref_db: -70.0,
            iso_gain: 1.0,
            iso_near: true,
            settings: settings.clone(),
            stream_delay_ms: 60,
            sent: 0,
            encoded: vec![0u8; 1500],
        })
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
        if ctl.muted.load(Ordering::Relaxed) || ctl.deafened.load(Ordering::Relaxed) {
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
                    if self.denoise.is_some() {
                        // RNNoise's voice probability, with hysteresis.
                        let threshold = if self.gate_open { 0.35 } else { 0.6 };
                        self.voice_prob > threshold && self.level_db > -60.0 && self.iso_near
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

    /// Voice isolation, after RNNoise: keep speech that's about as loud as your own voice and
    /// turn everything else far down. A voice across the room or on a TV reaches your mic
    /// 15+ dB quieter than yours, so the level tells them apart; RNNoise's speech probability
    /// catches the rest (and is why this needs RNNoise).
    fn isolate(&mut self, frame: &mut [f32], db: f32) {
        // Learn your level from confident speech: rise quickly to a louder voice; sink only
        // with speech close to it (you talking more softly), never toward a quieter voice
        // further away, or it would end up learning the other person.
        if self.voice_prob > 0.8 && db > -70.0 {
            if db > self.iso_ref_db {
                self.iso_ref_db += (db - self.iso_ref_db) * 0.3;
            } else if db > self.iso_ref_db - 8.0 {
                self.iso_ref_db += (db - self.iso_ref_db) * 0.005;
            }
        }
        let speech = self.voice_prob > 0.5;
        self.iso_near = db > self.iso_ref_db - 14.0;
        let target: f32 = match (speech, self.iso_near) {
            (true, true) => 1.0,
            (true, false) => 0.03,
            _ => 0.01,
        };
        // Glide to the new gain over the frame (10 ms) so it doesn't click; open faster than close.
        let start = self.iso_gain;
        let end = if target > start { target } else { start + (target - start) * 0.5 };
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
        self.scratch.copy_from_slice(&frame[..480]);
        let _ = self.apm.process_capture_f32(&[&self.scratch], &mut [&mut frame[..480]]);

        if let Some(dn) = self.denoise.as_mut() {
            for i in 0..480 {
                self.rn_in[i] = frame[i] * 32768.0;
            }
            self.voice_prob = dn.process_frame(&mut self.rn_out, &self.rn_in);
            for i in 0..480 {
                frame[i] = self.rn_out[i] / 32768.0;
            }
        }
        let db = rms_db(&frame[..480]);
        if self.settings.noise_suppression == NoiseSuppression::Isolation {
            self.isolate(&mut frame[..480], db);
        }
        // Fast attack, slower release, for a pleasant meter.
        self.level_db = if db > self.level_db { db } else { self.level_db * 0.85 + db * 0.15 };
        ctl.mic_level.store(self.level_db.to_bits(), Ordering::Relaxed);

        let open = self.gate(now, ctl);
        ctl.transmitting.store(open, Ordering::Relaxed);
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
