//! Stream audio: the sound that goes with a screen share. It is sent only to people who
//! are watching the stream (the server enforces that), as stereo Opus.
//!
//! Sources:
//! * Windows, whole screen: everything the PC plays *except Ingle itself*, so viewers never
//!   hear the call echoed back (WASAPI process loopback, "exclude" mode).
//! * Windows, one window: just that app and its child processes ("include" mode).
//! * The test pattern: a synthetic stereo chord, for tests and demos.
//!
//! Process loopback needs Windows 10 build 20348 or newer (Windows 11 is fine). Elsewhere a
//! screen share simply has no sound for now.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use ingle_proto::media::{flags, track};
use parking_lot::Mutex;
use tracing::{info, warn};

use super::PacketCallback;

/// 20 ms of 48 kHz stereo, interleaved.
const FRAME_SAMPLES: usize = 960 * 2;
/// Stop sending after this many silent 20 ms frames (0.4 s); resume on the next sound.
const SILENT_FRAMES_BEFORE_PAUSE: u32 = 20;
/// Largest Opus packet we produce, so a datagram always fits in one QUIC packet.
const MAX_PACKET: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamAudioSource {
    /// Everything this PC plays except Ingle itself.
    SystemExceptSelf,
    /// One application (by process id) and its child processes.
    Process(u32),
    /// A synthetic stereo chord: 523 Hz on the left, 784 Hz on the right.
    Synthetic,
}

pub const SYNTH_LEFT_HZ: f32 = 523.25;
pub const SYNTH_RIGHT_HZ: f32 = 784.0;

pub struct StreamAudio {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    pub packets: Arc<AtomicU64>,
    /// Set if capture stopped working after it started.
    pub error: Arc<Mutex<Option<String>>>,
}

impl StreamAudio {
    /// Start capturing and sending. Returns once capture is running (or failed to start).
    pub fn start(source: StreamAudioSource, bitrate_kbps: u32, sink: PacketCallback) -> Result<StreamAudio> {
        let stop = Arc::new(AtomicBool::new(false));
        let packets = Arc::new(AtomicU64::new(0));
        let error = Arc::new(Mutex::new(None));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        let (st, pk, er) = (stop.clone(), packets.clone(), error.clone());
        let thread = std::thread::Builder::new()
            .name("ingle-stream-audio".into())
            .spawn(move || run(source, bitrate_kbps, sink, st, pk, er, ready_tx))
            .map_err(|e| anyhow!("couldn't start the stream audio thread: {e}"))?;
        match ready_rx.recv_timeout(Duration::from_secs(4)) {
            Ok(Ok(())) => Ok(StreamAudio { stop, thread: Some(thread), packets, error }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                // Don't block on a capture API that never answered; the thread exits by itself
                // if it ever does.
                stop.store(true, Ordering::Relaxed);
                Err(anyhow!("the system didn't start audio capture in time"))
            }
        }
    }

    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for StreamAudio {
    fn drop(&mut self) {
        // Don't wait for the capture thread: it notices within ~100 ms, and we may be
        // dropped while the call lock is held.
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Where 48 kHz stereo samples come from.
trait Capture {
    /// Wait briefly for audio, then append whatever arrived (interleaved stereo f32).
    fn read(&mut self, out: &mut VecDeque<f32>) -> Result<()>;
}

struct Synth {
    start: Instant,
    produced: u64,
}

impl Capture for Synth {
    fn read(&mut self, out: &mut VecDeque<f32>) -> Result<()> {
        std::thread::sleep(Duration::from_millis(5));
        let due = (self.start.elapsed().as_micros() * 48 / 1000) as u64;
        while self.produced < due {
            let t = self.produced as f32 / 48_000.0;
            out.push_back((t * SYNTH_LEFT_HZ * std::f32::consts::TAU).sin() * 0.15);
            out.push_back((t * SYNTH_RIGHT_HZ * std::f32::consts::TAU).sin() * 0.15);
            self.produced += 1;
        }
        Ok(())
    }
}

fn open_capture(source: StreamAudioSource) -> Result<Box<dyn Capture>> {
    match source {
        StreamAudioSource::Synthetic => Ok(Box::new(Synth { start: Instant::now(), produced: 0 })),
        #[cfg(windows)]
        StreamAudioSource::SystemExceptSelf => Ok(Box::new(win::Loopback::open(std::process::id(), false)?)),
        #[cfg(windows)]
        StreamAudioSource::Process(pid) => Ok(Box::new(win::Loopback::open(pid, true)?)),
        #[cfg(not(windows))]
        _ => Err(anyhow!("sharing sound isn't available on this system yet")),
    }
}

fn run(
    source: StreamAudioSource,
    bitrate_kbps: u32,
    mut sink: PacketCallback,
    stop: Arc<AtomicBool>,
    packets: Arc<AtomicU64>,
    error: Arc<Mutex<Option<String>>>,
    ready: std::sync::mpsc::Sender<Result<()>>,
) {
    let setup = || -> Result<(Box<dyn Capture>, opus::Encoder)> {
        let mut enc = opus::Encoder::new(48_000, opus::Channels::Stereo, opus::Application::Audio)?;
        enc.set_bitrate(opus::Bitrate::Bits((bitrate_kbps.clamp(32, 256) * 1000) as i32))?;
        enc.set_vbr(true)?;
        enc.set_complexity(9)?;
        Ok((open_capture(source)?, enc))
    };
    let (mut cap, mut enc) = match setup() {
        Ok(x) => {
            let _ = ready.send(Ok(()));
            x
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    info!(?source, "stream audio started");
    let mut pending: VecDeque<f32> = VecDeque::with_capacity(FRAME_SAMPLES * 4);
    let mut frame = vec![0f32; FRAME_SAMPLES];
    let mut out = vec![0u8; MAX_PACKET];
    // Count of silent frames in a row; above the limit we are "paused" and send nothing.
    // Start paused, so the first packet is marked as the start of a talkspurt.
    let mut silent_run = SILENT_FRAMES_BEFORE_PAUSE + 1;
    while !stop.load(Ordering::Relaxed) {
        if let Err(e) = cap.read(&mut pending) {
            warn!("stream audio stopped: {e:#}");
            *error.lock() = Some(format!("{e:#}"));
            break;
        }
        // Never let a hiccup build up delay: keep at most ~100 ms queued.
        while pending.len() > FRAME_SAMPLES * 5 {
            pending.drain(..FRAME_SAMPLES);
        }
        while pending.len() >= FRAME_SAMPLES {
            for (d, s) in frame.iter_mut().zip(pending.drain(..FRAME_SAMPLES)) {
                *d = s;
            }
            let peak = frame.iter().fold(0f32, |m, v| m.max(v.abs()));
            let fl = if peak < 1e-4 {
                // About -80 dBFS: nothing is playing. Send a short tail, then pause.
                silent_run = silent_run.saturating_add(1);
                if silent_run > SILENT_FRAMES_BEFORE_PAUSE {
                    continue;
                }
                0
            } else {
                let resumed = silent_run > SILENT_FRAMES_BEFORE_PAUSE;
                silent_run = 0;
                if resumed { flags::TALKSPURT } else { 0 }
            };
            match enc.encode_float(&frame, &mut out) {
                Ok(n) => {
                    sink(track::SCREEN_AUDIO, fl, &out[..n]);
                    packets.fetch_add(1, Ordering::Relaxed);
                }
                Err(e) => warn!("stream audio encode failed: {e}"),
            }
        }
    }
    info!("stream audio stopped");
}

#[cfg(windows)]
mod win {
    use super::*;
    use wasapi::{AudioCaptureClient, AudioClient, Direction, Handle, SampleType, StreamMode, WaveFormat};

    pub struct Loopback {
        client: AudioClient,
        capture: AudioCaptureClient,
        event: Handle,
        bytes: VecDeque<u8>,
    }

    impl Loopback {
        /// `include == false`: everything except `pid`'s process tree; `true`: only that tree.
        pub fn open(pid: u32, include: bool) -> Result<Loopback> {
            let _ = wasapi::initialize_mta();
            let mut client = AudioClient::new_application_loopback_client(pid, include)
                .map_err(|e| anyhow!("Windows refused app audio capture (needs Windows 10 build 20348+ or Windows 11): {e}"))?;
            let format = WaveFormat::new(32, 32, &SampleType::Float, 48_000, 2, None);
            let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 200_000 };
            client.initialize_client(&format, &Direction::Capture, &mode).map_err(|e| anyhow!("audio capture setup failed: {e}"))?;
            let event = client.set_get_eventhandle().map_err(|e| anyhow!("audio capture setup failed: {e}"))?;
            let capture = client.get_audiocaptureclient().map_err(|e| anyhow!("audio capture setup failed: {e}"))?;
            client.start_stream().map_err(|e| anyhow!("couldn't start audio capture: {e}"))?;
            Ok(Loopback { client, capture, event, bytes: VecDeque::new() })
        }
    }

    impl Capture for Loopback {
        fn read(&mut self, out: &mut VecDeque<f32>) -> Result<()> {
            // A timeout just means nothing is playing right now.
            if self.event.wait_for_event(100).is_err() {
                return Ok(());
            }
            loop {
                let frames = self.capture.get_next_packet_size().map_err(|e| anyhow!("audio capture failed: {e}"))?.unwrap_or(0);
                if frames == 0 {
                    break;
                }
                let before = self.bytes.len();
                let info = self.capture.read_from_device_to_deque(&mut self.bytes).map_err(|e| anyhow!("audio capture failed: {e}"))?;
                if info.flags.silent {
                    for b in self.bytes.iter_mut().skip(before) {
                        *b = 0;
                    }
                }
            }
            while self.bytes.len() >= 4 {
                let b = [
                    self.bytes.pop_front().unwrap_or(0),
                    self.bytes.pop_front().unwrap_or(0),
                    self.bytes.pop_front().unwrap_or(0),
                    self.bytes.pop_front().unwrap_or(0),
                ];
                out.push_back(f32::from_le_bytes(b));
            }
            Ok(())
        }
    }

    impl Drop for Loopback {
        fn drop(&mut self) {
            let _ = self.client.stop_stream();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_source_sends_stereo_opus() {
        let got: Arc<Mutex<Vec<(u8, u8, Vec<u8>)>>> = Arc::default();
        let g = got.clone();
        let mut sa = StreamAudio::start(
            StreamAudioSource::Synthetic,
            128,
            Box::new(move |t, f, d| g.lock().push((t, f, d.to_vec()))),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(400));
        sa.stop();
        let pk = got.lock();
        assert!(pk.len() >= 12, "only {} packets in 400 ms", pk.len());
        assert!(pk.iter().all(|(t, _, d)| *t == track::SCREEN_AUDIO && d.len() <= MAX_PACKET));
        assert_eq!(pk[0].1 & flags::TALKSPURT, flags::TALKSPURT, "first packet starts a talkspurt");
        assert!(pk[1..].iter().all(|(_, f, _)| f & flags::TALKSPURT == 0));
        // Decode and check the channels stayed apart.
        let mut dec = opus::Decoder::new(48_000, opus::Channels::Stereo).unwrap();
        let mut pcm = vec![0f32; 960 * 2];
        let (mut l, mut r) = (Vec::new(), Vec::new());
        for (_, _, d) in pk.iter() {
            let n = dec.decode_float(d, &mut pcm, false).unwrap();
            for i in 0..n {
                l.push(pcm[2 * i]);
                r.push(pcm[2 * i + 1]);
            }
        }
        let ratio = |x: &[f32], f: f32| {
            let w = std::f32::consts::TAU * f / 48_000.0;
            let c = 2.0 * w.cos();
            let (mut s1, mut s2) = (0f32, 0f32);
            for v in x {
                let s0 = v + c * s1 - s2;
                s2 = s1;
                s1 = s0;
            }
            (s1 * s1 + s2 * s2 - c * s1 * s2) / (x.iter().map(|v| v * v).sum::<f32>() * x.len() as f32 / 2.0)
        };
        let tail = |x: &Vec<f32>| x[x.len() - 9600..].to_vec();
        let (l, r) = (tail(&l), tail(&r));
        assert!(ratio(&l, SYNTH_LEFT_HZ) > 0.8 && ratio(&l, SYNTH_RIGHT_HZ) < 0.05, "left channel");
        assert!(ratio(&r, SYNTH_RIGHT_HZ) > 0.8 && ratio(&r, SYNTH_LEFT_HZ) < 0.05, "right channel");
    }
}
