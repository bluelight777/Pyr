//! Stream audio: the sound that goes with a screen share. It is sent only to people who
//! are watching the stream (the server enforces that), as stereo Opus.
//!
//! Sources:
//! * Windows, whole screen: everything the PC plays *except Pyr itself*, so viewers never
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
use pyr_proto::media::{flags, track};
use parking_lot::Mutex;
use tracing::{info, warn};

use super::stream_echo::{FRAME as ECHO_FRAME, StreamEcho};
use super::spectrum::Analyzer;
use super::{Meter, MeterTap, PacketCallback, PlayedTap};

/// 20 ms of 48 kHz stereo, interleaved.
const FRAME_SAMPLES: usize = 960 * 2;
/// Stop sending after this many silent 20 ms frames (0.4 s); resume on the next sound.
const SILENT_FRAMES_BEFORE_PAUSE: u32 = 20;
/// Largest Opus packet we produce, so a datagram always fits in one QUIC packet.
const MAX_PACKET: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamAudioSource {
    /// Everything this PC plays except Pyr itself.
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
    /// My stream's sound muted from my end: nothing of it is sent (the picture goes on).
    pub muted: Arc<AtomicBool>,
}

impl StreamAudio {
    /// Start capturing and sending. Returns once capture is running (or failed to start).
    /// With `played` (what the call plays), the call is taken back out of the stream's sound
    /// if it gets in (see `stream_echo`). With `meter`, what's sent is measured there too
    /// (for my voice meter).
    pub fn start(source: StreamAudioSource, bitrate_kbps: u32, sink: PacketCallback, played: Option<Arc<PlayedTap>>, meter: Option<Arc<MeterTap>>) -> Result<StreamAudio> {
        Self::start_from(Box::new(move || open_capture(source)), format!("{source:?}"), bitrate_kbps, sink, played, meter)
    }

    /// `start`, with capture opened (on the capture thread) by `open`.
    fn start_from(
        open: OpenCapture,
        label: String,
        bitrate_kbps: u32,
        sink: PacketCallback,
        played: Option<Arc<PlayedTap>>,
        meter: Option<Arc<MeterTap>>,
    ) -> Result<StreamAudio> {
        let stop = Arc::new(AtomicBool::new(false));
        let packets = Arc::new(AtomicU64::new(0));
        let error = Arc::new(Mutex::new(None));
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<Result<()>>();
        let muted = Arc::new(AtomicBool::new(false));
        let (st, pk, er, mu) = (stop.clone(), packets.clone(), error.clone(), muted.clone());
        let thread = std::thread::Builder::new()
            .name("pyr-stream-audio".into())
            .spawn(move || run(open, label, bitrate_kbps, sink, played, meter, st, pk, er, mu, ready_tx))
            .map_err(|e| anyhow!("couldn't start the stream audio thread: {e}"))?;
        match ready_rx.recv_timeout(Duration::from_secs(4)) {
            Ok(Ok(())) => Ok(StreamAudio { stop, thread: Some(thread), packets, error, muted }),
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

/// Opens capture (on the thread that reads it).
type OpenCapture = Box<dyn FnOnce() -> Result<Box<dyn Capture>> + Send>;

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

#[allow(clippy::too_many_arguments)]
fn run(
    open: OpenCapture,
    label: String,
    bitrate_kbps: u32,
    mut sink: PacketCallback,
    played: Option<Arc<PlayedTap>>,
    meter: Option<Arc<MeterTap>>,
    stop: Arc<AtomicBool>,
    packets: Arc<AtomicU64>,
    error: Arc<Mutex<Option<String>>>,
    muted: Arc<AtomicBool>,
    ready: std::sync::mpsc::Sender<Result<()>>,
) {
    let setup = move || -> Result<(Box<dyn Capture>, opus::Encoder)> {
        let mut enc = opus::Encoder::new(48_000, opus::Channels::Stereo, opus::Application::Audio)?;
        enc.set_bitrate(opus::Bitrate::Bits((bitrate_kbps.clamp(32, 256) * 1000) as i32))?;
        // A lost packet is rebuilt from the next one (as voice's are) instead of guessed at:
        // what makes a stream's sound skip on a busy connection.
        enc.set_inband_fec(true)?;
        enc.set_packet_loss_perc(10)?;
        enc.set_vbr(true)?;
        enc.set_complexity(9)?;
        Ok((open()?, enc))
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
    info!(source = label, "stream audio started");
    // What the call played, to take back out (its own 10 ms at a time, in step with capture).
    let mut echo = played.map(|tap| {
        tap.subscribe();
        (tap, StreamEcho::new(), Vec::with_capacity(ECHO_FRAME * 8))
    });
    let mut captured: VecDeque<f32> = VecDeque::with_capacity(FRAME_SAMPLES * 4);
    let mut ten_ms = vec![0f32; ECHO_FRAME];
    let mut pending: VecDeque<f32> = VecDeque::with_capacity(FRAME_SAMPLES * 4);
    let mut frame = vec![0f32; FRAME_SAMPLES];
    let mut out = vec![0u8; MAX_PACKET];
    // What's sent, measured for my voice meter.
    let mut analyzer = meter.as_ref().map(|_| (Analyzer::new(), vec![0f32; FRAME_SAMPLES / 2]));
    // Count of silent frames in a row; above the limit we are "paused" and send nothing.
    // Start paused, so the first packet is marked as the start of a talkspurt.
    let mut silent_run = SILENT_FRAMES_BEFORE_PAUSE + 1;
    while !stop.load(Ordering::Relaxed) {
        if let Err(e) = cap.read(&mut captured) {
            warn!("stream audio stopped: {e:#}");
            *error.lock() = Some(format!("{e:#}"));
            break;
        }
        match echo.as_mut() {
            Some((tap, ec, buf)) => {
                buf.clear();
                tap.take(buf);
                ec.played(buf.iter().copied());
                while captured.len() >= ECHO_FRAME {
                    for (d, s) in ten_ms.iter_mut().zip(captured.drain(..ECHO_FRAME)) {
                        *d = s;
                    }
                    ec.clean(&mut ten_ms);
                    pending.extend(ten_ms.iter().copied());
                }
            }
            None => pending.extend(captured.drain(..)),
        }
        // Never let a hiccup build up delay: keep at most ~100 ms queued.
        while pending.len() > FRAME_SAMPLES * 5 {
            pending.drain(..FRAME_SAMPLES);
        }
        while pending.len() >= FRAME_SAMPLES {
            for (d, s) in frame.iter_mut().zip(pending.drain(..FRAME_SAMPLES)) {
                *d = s;
            }
            // (Muted from my end: as if nothing were playing. Silence goes out for a moment
            // before it pauses, like after any sound, and silence it is.)
            if muted.load(Ordering::Relaxed) {
                frame.fill(0.0);
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
            if let (Some(tap), Some((an, mono))) = (meter.as_ref(), analyzer.as_mut()) {
                super::spectrum::mono(&frame, 2, mono);
                let sum: f32 = mono.iter().map(|v| v * v).sum();
                let db = 10.0 * (sum / mono.len() as f32).max(1e-10).log10();
                if fl & flags::TALKSPURT != 0 {
                    an.clear();
                }
                an.feed(&mono[..480]);
                let bands = an.feed(&mono[480..]);
                tap.set(Meter { db, bands }, Instant::now());
            }
        }
    }
    if let Some((tap, ..)) = echo {
        tap.unsubscribe();
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

    /// A PC whose sound goes through another program that plays it all again (so Windows
    /// can't leave Pyr out): each read plays 10 ms of the call (into `tap`, as the call does)
    /// and captures it back `delay` samples later, with nothing else playing.
    struct LeakyPc {
        tap: Arc<PlayedTap>,
        call: Vec<f32>,
        delay: usize,
        at: usize,
    }

    impl Capture for LeakyPc {
        fn read(&mut self, out: &mut VecDeque<f32>) -> Result<()> {
            if self.at + 960 > self.call.len() {
                std::thread::sleep(Duration::from_millis(5));
                return Ok(());
            }
            self.tap.push(&self.call[self.at..self.at + 960]);
            for i in self.at..self.at + 960 {
                out.push_back(if i >= 2 * self.delay { 0.7 * self.call[i - 2 * self.delay] } else { 0.0 });
            }
            self.at += 960;
            Ok(())
        }
    }

    /// The stream's sound (decoded), per second, in dBFS.
    fn levels(packets: &[(u8, u8, Vec<u8>)]) -> Vec<f32> {
        let mut dec = opus::Decoder::new(48_000, opus::Channels::Stereo).unwrap();
        let mut pcm = vec![0f32; 960 * 2];
        let mut all = Vec::new();
        for (_, _, d) in packets {
            let n = dec.decode_float(d, &mut pcm, false).unwrap();
            all.extend_from_slice(&pcm[..2 * n]);
        }
        all.chunks(96_000).map(|c| 10.0 * (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).max(1e-12).log10()).collect()
    }

    #[test]
    fn the_call_is_taken_back_out_of_a_streams_sound() {
        // People taking turns: bursts of colored noise.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut lp = 0f32;
        let call: Vec<f32> = (0..48_000 * 14)
            .flat_map(|i| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                lp += 0.3 * ((seed >> 40) as f32 / (1u64 << 23) as f32 - 1.0 - lp);
                let talking = (i as f32 / 48_000.0 * 0.7).fract() < 0.6;
                let v = if talking { 0.3 * lp } else { 0.0 };
                [v, v]
            })
            .collect();
        let tap = Arc::new(PlayedTap::default());
        let got: Arc<Mutex<Vec<(u8, u8, Vec<u8>)>>> = Arc::default();
        let (g, t, c) = (got.clone(), tap.clone(), call.clone());
        let pc = Box::new(move || Ok(Box::new(LeakyPc { tap: t, call: c, delay: 60 * 48, at: 0 }) as Box<dyn Capture>));
        let mut sa = StreamAudio::start_from(pc, "leaky".into(), 128, Box::new(move |t, f, d| g.lock().push((t, f, d.to_vec()))), Some(tap.clone()), None).unwrap();
        let until = Instant::now() + Duration::from_secs(60);
        while got.lock().len() < 14 * 50 * 6 / 10 && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(300));
        sa.stop();
        let l = levels(&got.lock());
        assert!(l.len() >= 6, "only {} s of stream sound", l.len());
        // The first second goes out as captured; once the call's been found and learnt it's
        // taken out (the stream mostly stops: there's nothing else to send).
        let (first, last) = (l[0], *l.last().unwrap());
        assert!(first > -40.0, "the leak should be heard at first: {first:.1} dBFS");
        assert!(last < first - 20.0, "the call wasn't taken out: {l:?}");
    }

    #[test]
    fn muted_from_my_end_sends_none_of_its_sound() {
        let got = Arc::new(AtomicU64::new(0));
        let g = got.clone();
        let mut sa = StreamAudio::start(StreamAudioSource::Synthetic, 128, Box::new(move |_, _, _| { g.fetch_add(1, Ordering::Relaxed); }), None, None).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        assert!(got.load(Ordering::Relaxed) > 5, "sending");
        sa.muted.store(true, Ordering::Relaxed);
        // (The moment of silence that goes before a pause.)
        std::thread::sleep(Duration::from_millis(20 * SILENT_FRAMES_BEFORE_PAUSE as u64 + 300));
        let at = got.load(Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(got.load(Ordering::Relaxed), at, "muted: nothing more goes out");
        sa.muted.store(false, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(400));
        assert!(got.load(Ordering::Relaxed) > at + 5, "unmuted: its sound goes out again");
        sa.stop();
    }

    #[test]
    fn synthetic_source_sends_stereo_opus() {
        let got: Arc<Mutex<Vec<(u8, u8, Vec<u8>)>>> = Arc::default();
        let g = got.clone();
        let meter = Arc::new(MeterTap::default());
        let mut sa = StreamAudio::start(
            StreamAudioSource::Synthetic,
            128,
            Box::new(move |t, f, d| g.lock().push((t, f, d.to_vec()))),
            None,
            Some(meter.clone()),
        )
        .unwrap();
        std::thread::sleep(Duration::from_millis(400));
        // What's sent shows on my meter: the chord (523 and 784 Hz) in the middle bands.
        let m = meter.get(Instant::now()).expect("the stream's sound on my meter");
        assert!(m.db > -30.0, "{m:?}");
        let loudest = m.bands.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i).unwrap();
        assert!((4..=8).contains(&loudest), "{m:?}");
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
