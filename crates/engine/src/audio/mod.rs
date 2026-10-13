//! The audio engine: one worker thread that mixes playback, feeds the echo canceller,
//! processes the microphone and hands encoded Opus packets to the network.

pub mod capture;
pub mod dfn;
pub mod device;
pub mod jitter;
pub mod mixer;
pub mod resample;
pub mod stream;

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ingle_proto::control::SessionId;
use parking_lot::Mutex;
use tracing::{info, warn};

use crate::config::Settings;
use capture::CapturePipeline;
use device::{InputDevice, OutputDevice};
use mixer::Mixer;

/// Lock-free switches shared with the UI, network and hotkey threads.
#[derive(Default)]
pub struct Controls {
    pub muted: AtomicBool,
    pub deafened: AtomicBool,
    /// Shared with the global push-to-talk key watcher.
    pub ptt_down: Arc<AtomicBool>,
    pub transmitting: AtomicBool,
    /// f32 bits, dBFS
    pub mic_level: AtomicU32,
    /// Play your own processed microphone back to you (mic test).
    pub monitor: AtomicBool,
    pub stop: AtomicBool,
}

impl Controls {
    pub fn mic_level_db(&self) -> f32 {
        f32::from_bits(self.mic_level.load(Ordering::Relaxed))
    }
}

pub enum AudioInput {
    Device(Option<String>),
    /// Generates 480 samples (10 ms, 48 kHz mono) per call. Used by tests and bots.
    Synthetic(Box<dyn FnMut(&mut [f32]) + Send>),
    None,
}

pub enum AudioOutput {
    Device(Option<String>),
    /// Receives 10 ms of 48 kHz stereo (960 interleaved samples) per call.
    Synthetic(Box<dyn FnMut(&[f32]) + Send>),
    None,
}

#[derive(Default, Clone, Debug)]
pub struct AudioStatus {
    pub input_name: Option<String>,
    pub output_name: Option<String>,
    pub input_error: Option<String>,
    pub output_error: Option<String>,
    pub speaking: Vec<SessionId>,
    pub packets_sent: u64,
}

pub type PacketCallback = Box<dyn FnMut(u8, u8, &[u8]) + Send>;

pub struct AudioEngine {
    pub controls: Arc<Controls>,
    pub mixer: Arc<Mutex<Mixer>>,
    pub status: Arc<Mutex<AudioStatus>>,
    thread: Option<JoinHandle<()>>,
}

impl AudioEngine {
    pub fn start(
        settings: Settings,
        input: AudioInput,
        output: AudioOutput,
        sink: PacketCallback,
        ptt: Arc<AtomicBool>,
    ) -> AudioEngine {
        let controls = Arc::new(Controls { ptt_down: ptt, ..Default::default() });
        controls.mic_level.store((-100f32).to_bits(), Ordering::Relaxed);
        let mixer = Arc::new(Mutex::new(Mixer::default()));
        mixer.lock().master = settings.output_volume;
        let status = Arc::new(Mutex::new(AudioStatus::default()));
        let (c, m, s) = (controls.clone(), mixer.clone(), status.clone());
        let thread = std::thread::Builder::new()
            .name("ingle-audio".into())
            .spawn(move || worker(settings, input, output, sink, c, m, s))
            .expect("spawn audio thread");
        AudioEngine { controls, mixer, status, thread: Some(thread) }
    }

    pub fn stop(&mut self) {
        self.controls.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.stop();
    }
}

enum In {
    Dev(InputDevice),
    Synth(Box<dyn FnMut(&mut [f32]) + Send>),
    None,
}

enum Out {
    Dev(OutputDevice),
    Synth(Box<dyn FnMut(&[f32]) + Send>),
    Clock,
}

/// Target amount of audio queued at the sound card.
const OUTPUT_TARGET_MS: f32 = 30.0;

/// Ask the OS to schedule the audio thread like other real-time audio (MMCSS "Pro Audio"
/// on Windows), so a busy PC doesn't make voice crackle.
fn boost_audio_thread() {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::System::Threading::{AvSetMmThreadCharacteristicsW, GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_HIGHEST};
        let mut index = 0u32;
        let _ = AvSetMmThreadCharacteristicsW(windows::core::w!("Pro Audio"), &mut index);
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
    }
}

fn worker(
    settings: Settings,
    input: AudioInput,
    output: AudioOutput,
    mut sink: PacketCallback,
    ctl: Arc<Controls>,
    mixer: Arc<Mutex<Mixer>>,
    status: Arc<Mutex<AudioStatus>>,
) {
    boost_audio_thread();
    let mut cap = match CapturePipeline::new(&settings) {
        Ok(c) => Some(c),
        Err(e) => {
            warn!("audio processing unavailable: {e:#}");
            status.lock().input_error = Some(format!("{e:#}"));
            None
        }
    };
    let mut inp = match input {
        AudioInput::Device(id) => match InputDevice::open(id.as_deref()) {
            Ok(d) => {
                info!("microphone: {}", d.name);
                status.lock().input_name = Some(d.name.clone());
                In::Dev(d)
            }
            Err(e) => {
                warn!("microphone unavailable: {e:#}");
                status.lock().input_error = Some(format!("{e:#}"));
                In::None
            }
        },
        AudioInput::Synthetic(f) => In::Synth(f),
        AudioInput::None => In::None,
    };
    let mut out = match output {
        AudioOutput::Device(id) => match OutputDevice::open(id.as_deref()) {
            Ok(d) => {
                info!("speakers: {}", d.name);
                status.lock().output_name = Some(d.name.clone());
                Out::Dev(d)
            }
            Err(e) => {
                warn!("speakers unavailable: {e:#}");
                status.lock().output_error = Some(format!("{e:#}"));
                Out::Clock
            }
        },
        AudioOutput::Synthetic(f) => Out::Synth(f),
        AudioOutput::None => Out::Clock,
    };
    if let (Some(c), Out::Dev(_)) = (cap.as_mut(), &out) {
        // speaker queue + device buffers + mic buffering, roughly
        c.stream_delay_ms = (OUTPUT_TARGET_MS as i32) + 30;
    }

    let start = Instant::now();
    let mut cap_frames: u64 = 0;
    let mut out_frames: u64 = 0;
    let mut frame = [0f32; 480];
    let mut mix = vec![0f32; 960];
    let mut monitor: VecDeque<f32> = VecDeque::new();
    let mut last_status = Instant::now();

    while !ctl.stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        let due = (now.duration_since(start).as_micros() / 10_000) as u64;
        let deaf = ctl.deafened.load(Ordering::Relaxed);
        let monitoring = ctl.monitor.load(Ordering::Relaxed);

        // Playback first, so the echo canceller sees the far end before the near end.
        let produce = |mix: &mut Vec<f32>, cap: &mut Option<CapturePipeline>, monitor: &mut VecDeque<f32>| {
            {
                let mut m = mixer.lock();
                m.deafened = deaf;
                m.mix(mix, now);
            }
            if monitoring {
                for i in 0..480 {
                    let v = monitor.pop_front().unwrap_or(0.0);
                    mix[2 * i] += v;
                    mix[2 * i + 1] += v;
                }
            }
            if let Some(c) = cap.as_mut() {
                c.render(mix);
            }
        };
        match &mut out {
            Out::Dev(d) => {
                let mut guard = 0;
                while d.buffered_ms() < OUTPUT_TARGET_MS && guard < 10 {
                    produce(&mut mix, &mut cap, &mut monitor);
                    d.write(&mix);
                    guard += 1;
                }
            }
            Out::Synth(f) => {
                while out_frames < due {
                    produce(&mut mix, &mut cap, &mut monitor);
                    f(&mix);
                    out_frames += 1;
                }
            }
            Out::Clock => {
                while out_frames < due {
                    produce(&mut mix, &mut cap, &mut monitor);
                    out_frames += 1;
                }
            }
        }

        // Microphone.
        let mut process = |frame: &mut [f32; 480], cap: &mut Option<CapturePipeline>, monitor: &mut VecDeque<f32>| {
            if let Some(c) = cap.as_mut() {
                c.process(frame, &ctl, now, &mut *sink);
                if monitoring && c.level_db() > -100.0 {
                    if monitor.len() < 48_00 * 2 {
                        monitor.extend(frame.iter().copied());
                    }
                }
            }
        };
        match &mut inp {
            In::Dev(d) => {
                while d.read_frame(&mut frame) {
                    process(&mut frame, &mut cap, &mut monitor);
                }
            }
            In::Synth(g) => {
                while cap_frames < due {
                    g(&mut frame);
                    process(&mut frame, &mut cap, &mut monitor);
                    cap_frames += 1;
                }
            }
            In::None => {}
        }

        if now.duration_since(last_status) > Duration::from_millis(80) {
            last_status = now;
            let speaking = mixer.lock().speaking(now);
            let mut st = status.lock();
            st.speaking = speaking;
            st.packets_sent = cap.as_ref().map(|c| c.sent).unwrap_or(0);
        }
        std::thread::sleep(Duration::from_millis(3));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mic → processing → Opus → jitter buffer → mixer → speaker, all in-process.
    #[test]
    fn loopback_through_codec_and_mixer() {
        let packets: Arc<Mutex<Vec<(u8, u8, Vec<u8>)>>> = Arc::default();
        let p2 = packets.clone();
        let mut phase = 0usize;
        let mut settings = Settings::default();
        settings.noise_suppression = crate::config::NoiseSuppression::Off; // a pure tone isn't speech
        settings.vad_auto = false;
        settings.vad_threshold_db = -40.0;
        settings.echo_cancellation = false;
        settings.auto_gain = false;
        let mut engine = AudioEngine::start(
            settings,
            AudioInput::Synthetic(Box::new(move |buf| {
                for v in buf.iter_mut() {
                    *v = (phase as f32 / 48_000.0 * 440.0 * std::f32::consts::TAU).sin() * 0.25;
                    phase += 1;
                }
            })),
            AudioOutput::None,
            Box::new(move |t, f, data| p2.lock().push((t, f, data.to_vec()))),
            Arc::new(AtomicBool::new(false)),
        );
        std::thread::sleep(Duration::from_millis(500));
        engine.stop();
        let pk = packets.lock();
        assert!(pk.len() >= 20, "only {} packets", pk.len());
        assert_eq!(pk[0].1 & ingle_proto::media::flags::TALKSPURT, ingle_proto::media::flags::TALKSPURT);

        // Decode them through the mixer.
        let mut m = Mixer::default();
        let t0 = Instant::now();
        for (i, (_, f, d)) in pk.iter().enumerate() {
            m.push(7, ingle_proto::media::track::MIC, i as u32, *f, d.clone(), t0);
        }
        let mut out = vec![0f32; 960];
        let mut energy = 0f32;
        for k in 0..30 {
            m.mix(&mut out, t0 + Duration::from_millis(100 + k * 10));
            energy += out.iter().map(|v| v * v).sum::<f32>();
        }
        assert!(energy > 10.0, "decoded audio is silent ({energy})");
        assert_eq!(m.speaking(t0 + Duration::from_millis(390)), vec![7]);
    }
}
