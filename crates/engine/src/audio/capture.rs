//! Microphone processing: echo cancellation, noise suppression, gain control,
//! voice activity / push-to-talk gating, and Opus encoding.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use pyr_proto::media::{flags, track};
use sonora::config::{AdaptiveDigital, EchoCanceller, GainController2, HighPassFilter, NoiseSuppressionLevel};

use super::Controls;
use crate::config::{InputMode, NoiseSuppression, Settings};

pub type PacketSink<'a> = &'a mut dyn FnMut(u8, u8, &[u8]);

pub struct CapturePipeline {
    apm: sonora::AudioProcessing,
    /// The same processing without echo cancellation, and whether what's played comes back
    /// into the microphone (see `mic_echo`): when nothing does (headphones), the echo
    /// canceller only gets in the way, and this goes out instead.
    plain: Option<sonora::AudioProcessing>,
    echo: Option<super::mic_echo::MicEcho>,
    plain_out: Vec<f32>,
    /// The plain processing's sound, held back as long as the echo canceller holds its own
    /// (`AEC_LATENCY`), so the two line up while one takes over from the other.
    plain_line: std::collections::VecDeque<f32>,
    /// How much of what goes out is the echo canceller's (1) rather than the plain
    /// processing's (0): it moves over in 100 ms, so the switch can't be heard.
    aec_share: f32,
    /// Whether the speakers' sound could account for much of this 10 ms the microphone heard.
    far: bool,
    /// Something's been playing from the speakers in the last moment (voice isolation doesn't
    /// learn your level then).
    playing: bool,
    /// What Voice & Video was last told about the echo canceller (see `Controls::echo`).
    told_echo: Option<(u8, bool)>,
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
    /// How loud your confident speech has been (see `isolate`).
    iso_levels: Levels,
    /// Confident speech much quieter than your level, in a row (10 ms frames).
    iso_below: u32,
    iso_gain: f32,
    mine_until: Instant,
    settings: Settings,
    pub stream_delay_ms: i32,
    pub sent: u64,
    encoded: Vec<u8>,
    /// What my voice sounds like as it goes out, band by band (for my voice meter).
    analyzer: super::spectrum::Analyzer,
    /// When my voice last went out (to start the bands afresh after a silence).
    went_out: Option<Instant>,
    /// Watching for my voice going up and down for the others (see `swells`).
    swells: super::swells::Swells,
    /// This frame's level after echo cancellation and automatic gain.
    apm_db: f32,
    /// The last frame's levels: coming in, after echo cancellation and automatic gain, after
    /// noise suppression, going out (for measuring tools).
    stages: [f32; 4],
}

/// How long speech counts as going on after RNNoise last heard it (bridges the quiet parts of
/// words and the gaps between them).
const SPEECH_HOLD_MS: u64 = 350;
/// Voice isolation: speech must be within this many dB of your usual level to be taken as
/// you, and once it is, stays you down to this many dB (people trail off within sentences).
const ISO_START_DB: f32 = 12.0;
const ISO_KEEP_DB: f32 = 20.0;
/// Voice isolation: after this much confident speech (10 ms frames) all much quieter than
/// your level, it's taken as your level now.
const ISO_RELEARN_FRAMES: u32 = 3000;
/// Voice isolation: whether speech is detected in DeepFilterNet's cleaned sound, and the
/// look-ahead in 10 ms frames (see `tune_isolation`). Detecting it in the microphone's own
/// sound needs no look-ahead of its own: DeepFilterNet's output is 30 ms behind its input, so
/// the decision is already that far ahead of the sound it applies to. (Measured on speech
/// over 10 kinds of mechanical noise: same quality either way, 30 ms less delay this way.)
const ISO_DEFAULT_TUNING: (bool, usize) = (false, 0);
/// See `iso_start_lsnr`.
const ISO_START_LSNR: f32 = 3.0;
/// The echo canceller's sound comes out this much later (samples, 8 ms) than the same
/// processing's without it: the plain sound is held back as long (measured, see
/// `the_plain_and_echo_cancelled_sound_line_up`).
const AEC_LATENCY: usize = 384;
/// Voice isolation: how much is let through of other people's voices, and of other sounds.
const ISO_OTHER_VOICE: f32 = 0.05;
const ISO_FLOOR: f32 = 0.03;

fn rms_db(x: &[f32]) -> f32 {
    let s = x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32;
    if s > 1e-10 { 10.0 * s.log10() } else { -100.0 }
}

/// Levels heard (1 dB steps from -80 dBFS), what's older counting less: half as much after
/// every 30 seconds of what's counted.
#[derive(Default)]
struct Levels {
    bins: Vec<f32>,
    total: f32,
}

impl Levels {
    const FADE: f32 = 0.999_769; // 0.5^(1/3000): half-life of 3000 frames of 10 ms
    /// Enough to go by: half a second.
    const ENOUGH: f32 = 50.0;

    fn add(&mut self, db: f32) {
        if self.bins.is_empty() {
            self.bins = vec![0.0; 80];
        }
        self.bins.iter_mut().for_each(|b| *b *= Self::FADE);
        self.total *= Self::FADE;
        let i = ((db + 80.0).floor().max(0.0) as usize).min(79);
        self.bins[i] += 1.0;
        self.total += 1.0;
    }

    /// The usual level (dBFS): what three quarters of it is at or under, once there's enough.
    fn usual(&self) -> Option<f32> {
        if self.total < Self::ENOUGH {
            return None;
        }
        let mut seen = 0.0;
        for (i, b) in self.bins.iter().enumerate() {
            seen += b;
            if seen >= self.total * 0.75 {
                return Some(i as f32 - 80.0 + 0.5);
            }
        }
        None
    }
}

/// AEC3's own settings, changed from WebRTC's defaults (see vendor/sonora/PYR.md). With the
/// defaults, while others talked or music played, your voice was turned down 15 to 90 dB for
/// a moment, three to six times a minute, headphones or not (bug reports 27 and 32 to 41).
///
/// The echo path is first taken to be 20 dB weaker than the speakers (`default_gain`, 1.0
/// before): with headphones there's none, and until the canceller had learned that it took
/// every loud moment from the speakers to be coming back into the microphone, and took your
/// voice out with it. Its tail is taken to die away sooner (`default_len`, 0.83 before): the
/// canceller's own filter learns the room as it is. Telling whether there's any echo at all
/// uses the newer of the two ways (transparent mode, `Hmm`). `PYR_AB=aectune` for the defaults
/// (0.11.0).
///
/// 0.12.0 (bug reports 102 to 119): a microphone that clips isn't taken to be all echo
/// (`echo_can_saturate`, `PYR_AB=aecsat`); what's left of the echo after the canceller's filter
/// is only taken out where it's about as loud as your voice (`PYR_AB=aecmask`); and with
/// headphones the canceller stands aside altogether (see `mic_echo`, `PYR_AB=aecpath`).
/// Measured with `examples/aec_eval.rs` (your talking, 200 ms at a time, turned down 10 dB or
/// more, of 308, with Pyr's own settings, standard noise suppression and voice isolation; and
/// the echo that got through), 0.11.0 against 0.12.0:
///
/// |                                     | 0.11.0                | 0.12.0                |
/// |-------------------------------------|-----------------------|-----------------------|
/// | headphones, others talking          | 1, 1, 2 (-87 dB)      | 0, 0, 0 (-86 dB)      |
/// | headphones, music playing           | 1, 2, 1 (-87 dB)      | 0, 0, 0 (-85 dB)      |
/// | headphones, loud music and voices   | 11, 11, 12 (-97 dB)   | 0, 0, 0 (-95 dB)      |
/// | speakers, others talking            | 2, 2, 2 (-87 dB)      | 0, 0, 0 (-86 dB)      |
/// | speakers, music playing             | 6, 6, 7 (-77 dB)      | 4, 5, 6 (-77 dB)      |
/// | speakers turned low, music playing  | 5, 5, 5 (-77 dB)      | 1, 2, 1 (-75 dB)      |
/// | speakers, clocks 100 ppm apart      | 4, 5, 6 (-53 dB)      | 3, 2, 3 (-48 dB)      |
///
/// With the microphone open all along (push to talk held), what's left of the echo went from
/// -65 to -77 dB to at most 3 dB more.
fn tune_aec3(c: &mut sonora::config::EchoCanceller3Config) {
    if !pyr_proto::ab::old("aecsat") {
        c.ep_strength.echo_can_saturate = false;
    }
    if pyr_proto::ab::old("aectune") {
        return;
    }
    c.ep_strength.default_gain = 0.1;
    c.ep_strength.default_len = 0.5;
    c.ep_strength.nearend_len = 0.5;
    // (0.12.0) What's left of the echo is only taken out where it's as loud as your voice
    // (WebRTC's own setting for while you're talking, used all the time: its stricter one cut
    // your voice at the start of what you said while the speakers played). `PYR_AB=aecmask`
    // for the stricter one.
    if !pyr_proto::ab::old("aecmask") {
        c.suppressor.normal_tuning = c.suppressor.nearend_tuning.clone();
    }
}

/// The microphone's processing: high-pass filter, echo cancellation (if `echo`), noise
/// suppression (Standard), automatic gain.
fn processing(settings: &Settings, echo: bool) -> sonora::AudioProcessing {
    let config = sonora::Config {
        echo_canceller: echo.then(|| EchoCanceller {
            transparent_mode: if pyr_proto::ab::old("aectune") { Default::default() } else { sonora::config::TransparentModeType::Hmm },
            tune: Some(sonora::config::Aec3Tuning(tune_aec3)),
            ..Default::default()
        }),
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
    sonora::AudioProcessing::builder()
        .config(config)
        .capture_config(sonora::StreamConfig::new(48_000, 1))
        .render_config(sonora::StreamConfig::new(48_000, 1))
        .build()
}

impl CapturePipeline {
    pub fn new(settings: &Settings) -> anyhow::Result<CapturePipeline> {
        let apm = processing(settings, settings.echo_cancellation);
        // (`PYR_AB=aecpath`: the echo canceller's sound always, as before 0.12.0.)
        let both = settings.echo_cancellation && !pyr_proto::ab::old("aecpath");
        let plain = both.then(|| processing(settings, false));
        let echo = both.then(super::mic_echo::MicEcho::new);
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
            plain,
            echo,
            plain_out: vec![0.0; 480],
            plain_line: std::iter::repeat_n(0.0, AEC_LATENCY).collect(),
            aec_share: 1.0,
            far: false,
            playing: false,
            told_echo: None,
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
            iso_levels: Levels::default(),
            iso_below: 0,
            iso_gain: 1.0,
            mine_until: now,
            settings: settings.clone(),
            stream_delay_ms: 60,
            sent: 0,
            encoded: vec![0u8; 1500],
            analyzer: super::spectrum::Analyzer::new(),
            went_out: None,
            swells: Default::default(),
            apm_db: -100.0,
            stages: [-100.0; 4],
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

    /// For measuring tools: the last 10 ms's levels (dBFS) coming in, after echo cancellation
    /// and automatic gain, after noise suppression, and going out.
    #[doc(hidden)]
    pub fn stage_levels(&self) -> [f32; 4] {
        self.stages
    }

    /// For measuring tools: the last 10 ms as it went into voice isolation's noise remover
    /// (after echo cancellation and automatic gain).
    #[doc(hidden)]
    pub fn before_dfn(&self) -> &[f32] {
        &self.pre
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
    /// The echo canceller's own figures (for evaluations): echo return loss, its enhancement,
    /// the delay it found, and how sure the residual echo detector is there's echo.
    pub fn echo_stats(&self) -> String {
        let s = self.apm.statistics();
        format!(
            "erl {:?} erle {:?} delay {:?} divergent {:?}; {}",
            s.echo_return_loss.map(|v| v.round()),
            s.echo_return_loss_enhancement.map(|v| v.round()),
            s.delay_ms,
            s.divergent_filter_fraction.map(|v| (v * 100.0).round()),
            self.echo_words()
        )
    }

    /// What's known about the speakers' sound coming back into the microphone, in words.
    pub fn echo_words(&self) -> String {
        match (&self.echo, self.settings.echo_cancellation) {
            (_, false) => "echo cancellation off".into(),
            (None, true) => "echo cancellation always on".into(),
            (Some(e), true) => Self::echo_words_of(e),
        }
    }

    fn echo_words_of(e: &super::mic_echo::MicEcho) -> String {
        use super::mic_echo::Echo;
        match e.state() {
            Echo::Unknown => "not yet known whether the speakers' sound reaches the microphone (echo cancellation on)".into(),
            Echo::Found(ms) => {
                let weaker = e
                    .coupling_db()
                    .map(|db| match db {
                        db if db < -1.0 => format!(", {:.0} dB weaker", -db),
                        db if db > 1.0 => format!(", {db:.0} dB louder"),
                        _ => ", about as loud".to_string(),
                    })
                    .unwrap_or_default();
                if e.faint() {
                    format!("the speakers' sound reaches the microphone {ms:.0} ms later{weaker}: faintly (echo cancellation on all the same)")
                } else {
                    format!("the speakers' sound reaches the microphone {ms:.0} ms later{weaker} (echo cancellation on)")
                }
            }
            Echo::None => "the speakers' sound doesn't reach the microphone (headphones: echo cancellation stands aside)".into(),
        }
    }

    /// For measuring tools: the speakers-in-the-microphone detector's insides.
    #[doc(hidden)]
    pub fn echo_debug(&self) -> String {
        self.echo.as_ref().map(|e| e.debug()).unwrap_or_else(|| "no detector".into())
    }

    /// For measuring tools: whether the speakers' sound has been found in the microphone.
    pub fn echo_state(&self) -> Option<super::mic_echo::Echo> {
        self.echo.as_ref().map(|e| e.state())
    }

    /// The speakers changed (or came back): whatever came back into the microphone before
    /// may not now, and what does may come back differently.
    pub fn speakers_changed(&mut self) {
        if let Some(e) = self.echo.as_mut()
            && e.restart()
        {
            tracing::info!("speakers changed: echo cancellation on until it's known whether their sound reaches the microphone");
        }
    }

    pub fn render(&mut self, stereo: &[f32]) {
        if !self.settings.echo_cancellation {
            return;
        }
        for i in 0..480 {
            self.mono_render[i] = 0.5 * (stereo[2 * i] + stereo[2 * i + 1]);
        }
        let src = self.mono_render.clone();
        if let Some(e) = self.echo.as_mut() {
            e.played(&src);
        }
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
        // Learn your level from confident speech: the level three quarters of your speech is
        // at or under (recent speech counting most). A laugh, a shout or a loud moment from a
        // game or a speaker doesn't move it: those used to raise it quickly and for good, and
        // then your normal voice was turned down as if it were someone else's, in and out, for
        // as long as Pyr was open. A voice much quieter than yours (across the room) isn't
        // counted, unless for half a minute of speech there's nothing else.
        if self.voice_prob > 0.8 && level > -70.0 && pyr_proto::ab::old("isoref") {
            // (Before 0.9.1: rose quickly toward louder speech, sank only slowly.)
            if level > self.iso_ref_db {
                self.iso_ref_db += (level - self.iso_ref_db) * if self.iso_ref_db < -90.0 { 1.0 } else { 0.05 };
            } else if level > self.iso_ref_db - 8.0 {
                self.iso_ref_db += (level - self.iso_ref_db) * 0.01;
            }
        } else if self.voice_prob > 0.8 && level > -70.0 && speech && !self.far && !self.playing {
            // (Never from what the speakers could account for: someone else's voice coming
            // back from them, quieter than yours, was taken for yours after half a minute of
            // you listening, and went out at full strength, lifted by the automatic gain, till
            // echo cancellation was switched off and on: Chimbita's echo, 0.13.0.)
            if self.iso_ref_db < -90.0 || level > self.iso_ref_db - ISO_START_DB {
                self.iso_levels.add(level);
                self.iso_below = 0;
                match self.iso_levels.usual() {
                    Some(r) => self.iso_ref_db = r,
                    // (Not enough yet: the loudest so far, as a start.)
                    None => self.iso_ref_db = self.iso_ref_db.max(level),
                }
            } else {
                // Only quieter speech for a long while: that's you now (further from the
                // microphone, or it was turned down), not someone across the room.
                self.iso_below += 1;
                if self.iso_below >= ISO_RELEARN_FRAMES {
                    self.iso_levels = Levels::default();
                    self.iso_ref_db = level;
                    self.iso_below = 0;
                }
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

    /// The voice settings that can change while the sound flows (see `Controls::retune`); the
    /// rest (devices, noise suppression, echo cancellation, gain control) start the sound again.
    pub fn retune(&mut self, s: &Settings) {
        if s.voice_bitrate_kbps != self.settings.voice_bitrate_kbps {
            if let Err(e) = self.encoder.set_bitrate(opus::Bitrate::Bits((s.voice_bitrate_kbps.clamp(16, 256) * 1000) as i32)) {
                tracing::warn!("voice quality not changed: {e}");
            }
        }
        self.settings.voice_bitrate_kbps = s.voice_bitrate_kbps;
        self.settings.input_mode = s.input_mode;
        self.settings.vad_auto = s.vad_auto;
        self.settings.vad_threshold_db = s.vad_threshold_db;
        self.settings.ptt_key = s.ptt_key.clone();
        self.settings.ptt_release_ms = s.ptt_release_ms;
    }

    /// Process 10 ms of 48 kHz mono microphone audio.
    pub fn process(&mut self, frame: &mut [f32], ctl: &Controls, now: Instant, sink: PacketSink<'_>) {
        // Voice settings changed just now (the sensitivity let go of, say).
        if let Some(s) = ctl.retune.try_lock().and_then(|mut r| r.take()) {
            self.retune(&s);
        }
        // (As it is now: it can be dragged while I talk, see the call's mixer.)
        let gain = ctl.input_gain.get();
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
        self.far = false;
        self.playing = false;
        if let (Some(plain), Some(echo)) = (self.plain.as_mut(), self.echo.as_mut()) {
            use super::mic_echo::Echo;
            let _ = plain.process_capture_f32(&[&self.scratch], &mut [&mut self.plain_out]);
            self.plain_line.extend(self.plain_out.iter().copied());
            for v in self.plain_out.iter_mut() {
                *v = self.plain_line.pop_front().unwrap_or(0.0);
            }
            let needed = echo.needs_canceller();
            if let Some(now) = echo.heard(&self.scratch) {
                match now {
                    Echo::None => tracing::info!("the speakers' sound doesn't reach the microphone (headphones): echo cancellation stands aside"),
                    Echo::Found(ms) => tracing::info!(delay_ms = ms, "the speakers' sound reaches the microphone"),
                    Echo::Unknown => tracing::info!("the speakers' sound may reach the microphone: echo cancellation on until it's known"),
                }
            }
            if echo.needs_canceller() != needed && matches!(echo.state(), Echo::Found(_)) {
                tracing::info!("{}", Self::echo_words_of(echo));
            }
            let heard = 10f32.powf(raw_db / 10.0);
            let could = echo.could_be_echo();
            // Over to the echo canceller's sound at once (within these 10 ms); away from it
            // over 100 ms.
            let from = self.aec_share;
            // (Voice isolation: the echo canceller's sound always, as before 0.12.0. Without it,
            // what came back from the speakers is a voice, and isolation and the automatic gain
            // sent it on: in 0.12.0 everyone on voice isolation echoed.)
            let iso = self.settings.noise_suppression == NoiseSuppression::Isolation;
            let to = if iso || echo.needs_canceller() { 1.0 } else { (from - 0.1).max(0.0) };
            if from < 1.0 || to < 1.0 {
                for (i, (v, p)) in frame[..480].iter_mut().zip(&self.plain_out).enumerate() {
                    let share = from + (to - from) * (i as f32 + 1.0) / 480.0;
                    *v = *v * share + p * (1.0 - share);
                }
            }
            self.aec_share = to;
            // (Echo the processing took out isn't my voice turned down.)
            self.far = could * 10.0 > heard;
            self.playing = echo.playing();
            // Voice & Video says what's going on.
            let now_told = (match echo.state() {
                Echo::Unknown => 0,
                Echo::Found(_) => 1,
                Echo::None => 2,
            }, iso || echo.needs_canceller());
            if self.told_echo != Some(now_told) {
                self.told_echo = Some(now_told);
                *ctl.echo.lock() = Some(
                    match now_told {
                        (0, _) => "Listening for your speakers in your microphone; until it's known, echo cancellation is on.",
                        (1, _) => "Your speakers can be heard in your microphone, so echo cancellation is on.",
                        (2, true) => "Voice isolation keeps echo cancellation on, headphones or not.",
                        _ => "Your speakers can't be heard in your microphone (headphones), so echo cancellation stands aside: it would only get in your voice's way.",
                    }
                    .to_string(),
                );
            }
        }
        self.apm_db = rms_db(&frame[..480]);

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
        self.stages = [raw_db, self.apm_db, db, rms_db(&frame[..480])];
        if let Some(what) = self.swells.feed(self.stages, open, self.far) {
            let what = format!("{what} ({})", self.echo_words());
            tracing::warn!("{what}");
            *ctl.swelled.lock() = Some(what);
        }
        ctl.transmitting.store(open, Ordering::Relaxed);
        // My voice meter: what goes out, while it does (unless voice meters are off).
        if open && !ctl.meters_off.load(Ordering::Relaxed) {
            if !self.went_out.is_some_and(|t| now.saturating_duration_since(t) < super::METER_FRESH) {
                self.analyzer.clear();
            }
            let bands = self.analyzer.feed(&frame[..480]);
            ctl.mic_meter.set(super::Meter { db: self.level_db, bands }, now);
            self.went_out = Some(now);
        }
        // Something said (not just an open microphone in a quiet room): I'm here.
        if open && self.level_db > -50.0 {
            ctl.last_spoke_ms.store(pyr_proto::now_ms(), Ordering::Relaxed);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds `ms` of steady noise at about -25 dBFS: the sizes of the packets sent, and whether
    /// the microphone is open at the end.
    fn feed(cap: &mut CapturePipeline, ctl: &Controls, now: &mut Instant, seed: &mut u32, ms: u64) -> (Vec<usize>, bool) {
        let mut sizes = Vec::new();
        for _ in 0..ms / 10 {
            let mut frame: Vec<f32> = (0..480)
                .map(|_| {
                    *seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (*seed >> 8) as f32 / (1u32 << 24) as f32 * 0.2 - 0.1
                })
                .collect();
            cap.process(&mut frame, ctl, *now, &mut |_, _, p: &[u8]| sizes.push(p.len()));
            *now += Duration::from_millis(10);
        }
        (sizes, ctl.transmitting.load(Ordering::Relaxed))
    }

    #[test]
    fn voice_settings_change_while_the_sound_flows() {
        let base = Settings {
            noise_suppression: NoiseSuppression::Off,
            echo_cancellation: false,
            auto_gain: false,
            vad_auto: false,
            vad_threshold_db: -40.0,
            voice_bitrate_kbps: 24,
            ..Settings::default()
        };
        let mut cap = CapturePipeline::new(&base).unwrap();
        let ctl = Controls::default();
        let (mut now, mut seed) = (Instant::now(), 7u32);
        let (sent, open) = feed(&mut cap, &ctl, &mut now, &mut seed, 600);
        assert!(open && sent.len() > 20, "louder than the sensitivity: heard ({} packets)", sent.len());
        let small = sent[5..].iter().sum::<usize>() as f32 / (sent.len() - 5) as f32;

        // The sensitivity let go of above the noise: it stops getting through (after the
        // usual 400 ms for the ends of words), with nothing started again.
        *ctl.retune.lock() = Some(Settings { vad_threshold_db: -15.0, ..base.clone() });
        let (_, open) = feed(&mut cap, &ctl, &mut now, &mut seed, 600);
        assert!(!open, "quieter than the new sensitivity");
        assert!(ctl.retune.lock().is_none(), "taken up");
        let (sent, _) = feed(&mut cap, &ctl, &mut now, &mut seed, 300);
        assert!(sent.is_empty(), "nothing sent");

        // Better voice quality: bigger packets straight away.
        *ctl.retune.lock() = Some(Settings { voice_bitrate_kbps: 128, ..base.clone() });
        let (sent, open) = feed(&mut cap, &ctl, &mut now, &mut seed, 600);
        assert!(open, "heard again");
        let big = sent[5..].iter().sum::<usize>() as f32 / (sent.len() - 5) as f32;
        assert!(big > small * 2.0, "{small} bytes a packet at 24 kbps, {big} at 128 kbps");

        // Push to talk, with the key up: quiet once its release delay is over.
        *ctl.retune.lock() = Some(Settings { input_mode: InputMode::PushToTalk, ptt_release_ms: 100, ..base.clone() });
        let (_, open) = feed(&mut cap, &ctl, &mut now, &mut seed, 200);
        assert!(!open, "push to talk, key not held");
        ctl.ptt_down.store(true, Ordering::Relaxed);
        let (_, open) = feed(&mut cap, &ctl, &mut now, &mut seed, 100);
        assert!(open, "key held");
    }

    /// With headphones (nothing played comes back), echo cancellation stands aside, except
    /// with voice isolation, which keeps it on: 0.12.0 let it stand aside there too, and
    /// everyone on voice isolation echoed.
    #[test]
    fn voice_isolation_keeps_echo_cancellation_on() {
        for (ns, stays_on) in [(NoiseSuppression::Isolation, true), (NoiseSuppression::Standard, false)] {
            let settings = Settings { noise_suppression: ns, ..Settings::default() };
            let mut cap = CapturePipeline::new(&settings).unwrap();
            let ctl = Controls::default();
            let mut now = Instant::now();
            let (mut a, mut b) = (1u32, 99u32);
            let mut noise = |x: &mut u32, amp: f32| {
                *x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (*x >> 8) as f32 / (1u32 << 24) as f32 * 2.0 * amp - amp
            };
            for _ in 0..1_500 {
                let played: Vec<f32> = (0..960).map(|_| noise(&mut a, 0.1)).collect();
                cap.render(&played);
                let mut frame: Vec<f32> = (0..480).map(|_| noise(&mut b, 0.05)).collect();
                cap.process(&mut frame, &ctl, now, &mut |_, _, _: &[u8]| {});
                now += Duration::from_millis(10);
            }
            assert_eq!(cap.echo_state(), Some(super::super::mic_echo::Echo::None), "{ns:?}: headphones");
            assert_eq!(cap.aec_share == 1.0, stays_on, "{ns:?}: {}", cap.aec_share);
        }
    }

    /// Voice isolation learns your level from your speech, never from what the speakers could
    /// account for: half a minute of someone else's voice coming back from them, quieter than
    /// yours, doesn't make it "you" (it did: their own voice went back to them as an echo).
    /// A quieter voice with nothing playing still does (you moved away from the microphone).
    #[test]
    fn isolation_never_takes_the_speakers_for_you() {
        let settings = Settings { noise_suppression: NoiseSuppression::Isolation, ..Settings::default() };
        for (far, relearns) in [(true, false), (false, true)] {
            let mut cap = CapturePipeline::new(&settings).unwrap();
            let now = Instant::now();
            cap.iso_ref_db = -25.0;
            cap.iso_levels = Levels::default();
            // Speech 20 dB under yours, for 40 s.
            cap.raw_recent = [10f32.powf(-4.5); 5];
            cap.voice_prob = 0.95;
            cap.speech_until = now + Duration::from_secs(3600);
            cap.far = far;
            cap.playing = far;
            let mut frame = vec![0.01f32; 480];
            for _ in 0..4_000 {
                cap.isolate(now, &mut frame);
            }
            if relearns {
                assert!((cap.iso_ref_db + 45.0).abs() < 1.0, "a quieter voice, nothing playing: you, now ({})", cap.iso_ref_db);
            } else {
                assert_eq!(cap.iso_ref_db, -25.0, "what the speakers could account for isn't you");
            }
        }
    }

    /// Switching between the echo canceller's sound and the plain processing's mixes the two
    /// for 100 ms: they must line up, or the voice would sound hollow while it does.
    #[test]
    fn the_plain_and_echo_cancelled_sound_line_up() {
        for settings in [
            Settings::default(),
            Settings { noise_suppression: NoiseSuppression::Standard, ..Settings::default() },
            Settings { auto_gain: false, ..Settings::default() },
            Settings { noise_suppression: NoiseSuppression::Standard, auto_gain: false, ..Settings::default() },
            Settings { noise_suppression: NoiseSuppression::Off, auto_gain: false, ..Settings::default() },
        ] {
            let mut aec = processing(&settings, true);
            let mut plain = processing(&settings, false);
            let mut seed = 11u32;
            let (mut a_out, mut p_out) = (Vec::new(), Vec::new());
            let silence = vec![0f32; 480];
            let mut lp = 0f32;
            for k in 0..300 {
                // Talking-like bursts of colored noise.
                let frame: Vec<f32> = (0..480)
                    .map(|i| {
                        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        lp += 0.3 * ((seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0 - lp);
                        let t = (k * 480 + i) as f32 / 48_000.0;
                        if (t * 1.3).fract() < 0.6 { 0.2 * lp } else { 0.0 }
                    })
                    .collect();
                let mut out = vec![0f32; 480];
                let _ = aec.process_render_f32(&[&silence], &mut [&mut vec![0f32; 480]]);
                let _ = aec.process_capture_f32(&[&frame], &mut [&mut out]);
                a_out.extend_from_slice(&out);
                let _ = plain.process_capture_f32(&[&frame], &mut [&mut out]);
                p_out.extend_from_slice(&out);
            }
            // (Held back as `process` holds it.)
            let p_out: Vec<f32> = std::iter::repeat_n(0.0, AEC_LATENCY).chain(p_out).take(a_out.len()).collect();
            // From 1 s on: how alike they are, a few samples either way.
            let alike = |lag: i64| -> f64 {
                let (mut ab, mut aa, mut bb) = (0f64, 0f64, 0f64);
                for i in 48_000..a_out.len() - 500 {
                    let a = a_out[i] as f64;
                    let b = p_out[(i as i64 + lag) as usize] as f64;
                    ab += a * b;
                    aa += a * a;
                    bb += b * b;
                }
                ab / (aa * bb).sqrt().max(1e-20)
            };
            let best = (-480..=480).max_by(|a, b| alike(*a).total_cmp(&alike(*b))).unwrap();
            assert_eq!(best, 0, "{:?}: the plain sound is {best} samples off", settings.noise_suppression);
            assert!(alike(0) > 0.99, "not alike: {:.3}", alike(0));
        }
    }
}
