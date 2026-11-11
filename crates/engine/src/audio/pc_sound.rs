//! The PC's own sound, for voice isolation (0.13.2): everything the PC plays but Pyr (a match,
//! a game: WASAPI process loopback in "exclude" mode, as a screen share's sound is taken), handed
//! to the echo canceller with what the call plays, so what of it comes back into the microphone
//! from the speakers is taken out too, not only the call. Windows 10 build 20348 or newer, or
//! Windows 11; elsewhere there's none.
//!
//! The capture pipeline adds it to what the echo canceller is handed only while the speakers
//! can be heard in the microphone, or that isn't known yet (see `CapturePipeline::pc_sound`).
//! With headphones it can't come back, and handing it over anyway only gave the canceller more
//! to turn your voice down by: with voice isolation on and a game playing, your voice went down
//! and back up for the others, by 20 to 54 dB, three times a minute (bug reports 657 to 675).
//! What the canceller does with the call's own sound is as it was.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use parking_lot::Mutex;

/// 10 ms at 48 kHz.
const FRAME: usize = 480;
/// What's waiting is kept short, so it's handed over about as it plays: past 60 ms of it, the
/// oldest goes (down to 20 ms).
const MOST: usize = 6 * FRAME;
const KEEP: usize = 2 * FRAME;

pub struct PcSound {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    /// The PC's sound as it's heard (mono, 48 kHz), waiting to be handed over.
    queue: Arc<Mutex<VecDeque<f32>>>,
}

impl PcSound {
    /// Starts hearing the PC's own sound (`PYR_AB=pcsound`: not at all, as before 0.13.2).
    /// None where it can't be heard (not Windows, or Windows too old: said in the log).
    pub fn start() -> Option<PcSound> {
        if pyr_proto::ab::old("pcsound") {
            return None;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let queue: Arc<Mutex<VecDeque<f32>>> = Arc::default();
        let (s, q) = (stop.clone(), queue.clone());
        // (Opened on its own thread, which takes a moment: the call doesn't wait for it.)
        let thread = std::thread::Builder::new()
            .name("pyr-pc-sound".into())
            .spawn(move || {
                let mut source = match open() {
                    Ok(src) => src,
                    Err(e) => {
                        tracing::warn!("voice isolation can't hear the PC's own sound: {e}");
                        return;
                    }
                };
                tracing::info!("voice isolation takes the PC's own sound out of the microphone");
                let mut stereo: VecDeque<f32> = VecDeque::new();
                while !s.load(Ordering::Relaxed) {
                    if let Err(e) = source.read(&mut stereo) {
                        tracing::warn!("voice isolation stopped hearing the PC's own sound: {e}");
                        return;
                    }
                    let mut q = q.lock();
                    while stereo.len() >= 2 {
                        let (l, r) = (stereo.pop_front().unwrap_or(0.0), stereo.pop_front().unwrap_or(0.0));
                        q.push_back(0.5 * (l + r));
                    }
                    if q.len() > MOST {
                        let drop = q.len() - KEEP;
                        q.drain(..drop);
                    }
                }
            })
            .ok()?;
        Some(PcSound { stop, thread: Some(thread), queue })
    }

    /// The next 10 ms of the PC's sound into `frame`: false while there's none (nothing
    /// playing, or not heard yet), and `frame` is left as it was.
    pub fn take(&self, frame: &mut [f32]) -> bool {
        let mut q = self.queue.lock();
        if q.len() < FRAME {
            return false;
        }
        for v in frame.iter_mut().take(FRAME) {
            *v = q.pop_front().unwrap_or(0.0);
        }
        true
    }
}

impl Drop for PcSound {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Where the PC's sound comes from: stereo, 48 kHz, interleaved.
trait Source {
    /// Waits a little for sound and appends what came (a timeout is just nothing playing).
    fn read(&mut self, out: &mut VecDeque<f32>) -> anyhow::Result<()>;
}

#[cfg(windows)]
fn open() -> anyhow::Result<Box<dyn Source>> {
    Ok(Box::new(win::Loopback::open()?))
}

#[cfg(not(windows))]
fn open() -> anyhow::Result<Box<dyn Source>> {
    Err(anyhow::anyhow!("only Windows lets Pyr hear the PC's own sound"))
}

#[cfg(windows)]
mod win {
    use super::*;
    use anyhow::anyhow;
    use wasapi::{AudioCaptureClient, AudioClient, Direction, Handle, SampleType, StreamMode, WaveFormat};

    /// Everything the PC plays except Pyr's own process tree (Pyr's sound is the call, which
    /// the echo canceller already has).
    pub struct Loopback {
        client: AudioClient,
        capture: AudioCaptureClient,
        event: Handle,
        bytes: VecDeque<u8>,
    }

    impl Loopback {
        pub fn open() -> anyhow::Result<Loopback> {
            let _ = wasapi::initialize_mta();
            let mut client = AudioClient::new_application_loopback_client(std::process::id(), false)
                .map_err(|e| anyhow!("Windows refused app audio capture (needs Windows 10 build 20348+ or Windows 11): {e}"))?;
            let format = WaveFormat::new(32, 32, &SampleType::Float, 48_000, 2, None);
            // (A short buffer: it's handed to the echo canceller as it plays.)
            let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: 100_000 };
            client.initialize_client(&format, &Direction::Capture, &mode).map_err(|e| anyhow!("audio capture setup failed: {e}"))?;
            let event = client.set_get_eventhandle().map_err(|e| anyhow!("audio capture setup failed: {e}"))?;
            let capture = client.get_audiocaptureclient().map_err(|e| anyhow!("audio capture setup failed: {e}"))?;
            client.start_stream().map_err(|e| anyhow!("couldn't start audio capture: {e}"))?;
            Ok(Loopback { client, capture, event, bytes: VecDeque::new() })
        }
    }

    impl Source for Loopback {
        fn read(&mut self, out: &mut VecDeque<f32>) -> anyhow::Result<()> {
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
                let b = [self.bytes.pop_front().unwrap_or(0), self.bytes.pop_front().unwrap_or(0), self.bytes.pop_front().unwrap_or(0), self.bytes.pop_front().unwrap_or(0)];
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
