//! Sound card access through cpal (WASAPI on Windows, CoreAudio on macOS, ALSA on Linux).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use parking_lot::Mutex;
use tracing::{debug, info, warn};

use super::resample::Resampler;

/// How long a new stream gets to start calling back (some devices, e.g. Bluetooth headsets,
/// take a moment to wake up).
const START_GRACE: Duration = Duration::from_secs(3);

/// How a sound card stream is doing, kept up to date by the stream's own thread.
///
/// Windows can stop calling a stream back without a word: when a virtual device (NVIDIA
/// Broadcast, a GoXLR) restarts or changes format, say. Pyr then stopped playing anyone (or
/// stopped hearing your microphone) until it was restarted. This lets the audio thread notice
/// and reopen the device instead.
#[derive(Default)]
pub struct Health {
    /// Callbacks so far: the sound card asking for sound, or handing some over.
    callbacks: AtomicU64,
    failed: AtomicBool,
    why: Mutex<Option<String>>,
}

impl Health {
    fn tick(&self) {
        self.callbacks.fetch_add(1, Ordering::Relaxed);
    }

    /// The stream is finished (or plays to a device that isn't the one it should any more).
    fn fail(&self, why: String) {
        *self.why.lock() = Some(why);
        self.failed.store(true, Ordering::Release);
    }

    /// What a stream's error means for it. Most errors end the stream on Windows (its thread
    /// stops), which the stall check catches anyway; these say so outright.
    fn on_error(&self, what: &str, e: cpal::Error) {
        use cpal::ErrorKind as K;
        match e.kind() {
            // A glitch, or the OS turning down a priority boost: the stream carries on.
            K::Xrun | K::RealtimeDenied => debug!("{what} stream: {e}"),
            // Moved to the new default device by itself.
            K::DeviceChanged => info!("{what} stream: {e}"),
            K::DeviceNotAvailable | K::StreamInvalidated | K::HostUnavailable | K::DeviceBusy => {
                warn!("{what} stream error: {e}");
                self.fail(e.to_string());
            }
            _ => warn!("{what} stream error: {e}"),
        }
    }
}

/// The audio thread's side of a stream's `Health`.
struct Watch {
    health: Arc<Health>,
    seen: u64,
    since: Instant,
}

impl Watch {
    fn new(health: Arc<Health>) -> Watch {
        Watch { health, seen: 0, since: Instant::now() }
    }

    /// Why the stream should be reopened, if it should: it failed, or it hasn't called back
    /// for `stall`.
    fn trouble(&mut self, now: Instant, stall: Duration) -> Option<String> {
        if self.health.failed.load(Ordering::Acquire) {
            return Some(self.health.why.lock().clone().unwrap_or_else(|| "it failed".into()));
        }
        let n = self.health.callbacks.load(Ordering::Relaxed);
        if n != self.seen {
            self.seen = n;
            self.since = now;
            return None;
        }
        let quiet = now.saturating_duration_since(self.since);
        if n == 0 {
            (quiet >= stall.max(START_GRACE)).then(|| format!("it never started ({:.1} s)", quiet.as_secs_f32()))
        } else {
            (quiet >= stall).then(|| format!("no sound went through for {:.1} s", quiet.as_secs_f32()))
        }
    }
}

fn device_label(d: &cpal::Device) -> String {
    d.description().map(|x| x.name().to_string()).unwrap_or_else(|_| "Unknown device".to_string())
}

fn device_id(d: &cpal::Device) -> Option<String> {
    d.id().ok().map(|i| i.to_string())
}

/// (id, human name) for every input and output device.
pub fn list_devices() -> (Vec<(String, String)>, Vec<(String, String)>) {
    let host = cpal::default_host();
    let collect = |it: Option<Vec<cpal::Device>>| -> Vec<(String, String)> {
        it.unwrap_or_default().iter().filter_map(|d| Some((device_id(d)?, device_label(d)))).collect()
    };
    let inputs = collect(host.input_devices().ok().map(|i| i.collect()));
    let outputs = collect(host.output_devices().ok().map(|i| i.collect()));
    (inputs, outputs)
}

/// The device with this id (only that one), or the default device.
fn find_device(input: bool, id: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    if let Some(id) = id {
        let found = id.parse::<cpal::DeviceId>().ok().and_then(|parsed| host.device_by_id(&parsed));
        return found.ok_or_else(|| anyhow!(if input { "the microphone chosen isn't there" } else { "the speakers chosen aren't there" }));
    }
    let d = if input { host.default_input_device() } else { host.default_output_device() };
    d.ok_or_else(|| anyhow!(if input { "no microphone found" } else { "no speakers or headphones found" }))
}

/// Open the device asked for; if that doesn't work and `fall_back` allows, the default device
/// stands in for it (marked `stand_in`).
fn open_or_default<T>(what: &str, id: Option<&str>, fall_back: bool, open: impl Fn(Option<&str>) -> Result<T>) -> Result<(T, bool)> {
    match open(id) {
        Ok(d) => Ok((d, false)),
        Err(e) if fall_back && id.is_some() => {
            warn!("{what} {} unavailable ({e:#}); using the default", id.unwrap_or_default());
            Ok((open(None)?, true))
        }
        Err(e) => Err(e),
    }
}

pub struct InputDevice {
    /// What keeps the sound flowing: the cpal stream (or a test's stand-in).
    _stream: Box<dyn Send>,
    cons: rtrb::Consumer<f32>,
    resampler: Resampler,
    pending: Vec<f32>,
    raw: Vec<f32>,
    watch: Watch,
    pub name: String,
    /// The default device, playing the part of one that wasn't there.
    pub stand_in: bool,
}

fn build_input<T>(device: &cpal::Device, config: cpal::StreamConfig, mut prod: rtrb::Producer<f32>, health: Arc<Health>) -> Result<cpal::Stream>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let ch = config.channels.max(1) as usize;
    let h = health.clone();
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            h.tick();
            for frame in data.chunks(ch) {
                let mut s = 0f32;
                for v in frame {
                    s += f32::from_sample_(*v);
                }
                let _ = prod.push(s / ch as f32);
            }
        },
        move |e| health.on_error("microphone", e),
        None,
    )?;
    Ok(stream)
}

impl InputDevice {
    /// The microphone with this id (the default one standing in if it can't be opened), or the
    /// default microphone.
    pub fn open(id: Option<&str>) -> Result<InputDevice> {
        InputDevice::open_with(id, true)
    }

    /// As `open`; without `fall_back`, only the microphone asked for will do.
    pub fn open_with(id: Option<&str>, fall_back: bool) -> Result<InputDevice> {
        let (mut d, stand_in) = open_or_default("microphone", id, fall_back, InputDevice::open_exact)?;
        d.stand_in = stand_in;
        Ok(d)
    }

    fn open_exact(id: Option<&str>) -> Result<InputDevice> {
        let device = find_device(true, id)?;
        let name = device_label(&device);
        let supported = device.default_input_config().context("microphone has no usable format")?;
        let format = supported.sample_format();
        let config = supported.config();
        let rate = config.sample_rate;
        let (prod, cons) = rtrb::RingBuffer::new((rate as usize).max(48_000));
        let health = Arc::new(Health::default());
        let h = health.clone();
        let stream = match format {
            SampleFormat::F32 => build_input::<f32>(&device, config, prod, h)?,
            SampleFormat::I16 => build_input::<i16>(&device, config, prod, h)?,
            SampleFormat::I32 => build_input::<i32>(&device, config, prod, h)?,
            SampleFormat::U16 => build_input::<u16>(&device, config, prod, h)?,
            SampleFormat::F64 => build_input::<f64>(&device, config, prod, h)?,
            other => return Err(anyhow!("unsupported microphone sample format {other:?}")),
        };
        stream.play()?;
        Ok(InputDevice::new(Box::new(stream), cons, rate, health, name))
    }

    fn new(stream: Box<dyn Send>, cons: rtrb::Consumer<f32>, rate: u32, health: Arc<Health>, name: String) -> InputDevice {
        InputDevice {
            _stream: stream,
            cons,
            resampler: Resampler::new(rate, 48_000),
            pending: Vec::with_capacity(4096),
            raw: Vec::with_capacity(4096),
            watch: Watch::new(health),
            name,
            stand_in: false,
        }
    }

    /// Why the microphone should be reopened, if it should (see `Health`).
    pub fn trouble(&mut self, stall: Duration) -> Option<String> {
        self.watch.trouble(Instant::now(), stall)
    }

    /// Fill `out` with the next 10 ms (480 samples at 48 kHz) if available.
    pub fn read_frame(&mut self, out: &mut [f32]) -> bool {
        let avail = self.cons.slots();
        if avail > 0 {
            self.raw.clear();
            if let Ok(chunk) = self.cons.read_chunk(avail) {
                let (a, b) = chunk.as_slices();
                self.raw.extend_from_slice(a);
                self.raw.extend_from_slice(b);
                chunk.commit_all();
            }
            let raw = std::mem::take(&mut self.raw);
            self.resampler.process(&raw, &mut self.pending);
            self.raw = raw;
        }
        // Don't let a stalled consumer build up latency.
        if self.pending.len() > 48_000 / 5 {
            let drop = self.pending.len() - 480 * 2;
            self.pending.drain(..drop);
        }
        if self.pending.len() >= 480 {
            out[..480].copy_from_slice(&self.pending[..480]);
            self.pending.drain(..480);
            true
        } else {
            false
        }
    }
}

pub struct OutputDevice {
    /// What keeps the sound flowing: the cpal stream (or a test's stand-in).
    _stream: Box<dyn Send>,
    prod: rtrb::Producer<f32>,
    capacity: usize,
    rate: u32,
    channels: usize,
    rs_l: Resampler,
    rs_r: Resampler,
    l: Vec<f32>,
    r: Vec<f32>,
    tmp_l: Vec<f32>,
    tmp_r: Vec<f32>,
    watch: Watch,
    pub name: String,
    /// The default device, playing the part of one that wasn't there.
    pub stand_in: bool,
}

fn build_output<T>(device: &cpal::Device, config: cpal::StreamConfig, mut cons: rtrb::Consumer<f32>, health: Arc<Health>) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let h = health.clone();
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            h.tick();
            let n = cons.slots().min(data.len());
            if let Ok(chunk) = cons.read_chunk(n) {
                let (a, b) = chunk.as_slices();
                for (d, s) in data.iter_mut().zip(a.iter().chain(b.iter())) {
                    *d = T::from_sample_(*s);
                }
                chunk.commit_all();
            }
            for d in data[n..].iter_mut() {
                *d = T::from_sample_(0.0f32);
            }
        },
        move |e| health.on_error("speaker", e),
        None,
    )?;
    Ok(stream)
}

impl OutputDevice {
    /// The speakers with this id (the default ones standing in if they can't be opened), or
    /// the default speakers.
    pub fn open(id: Option<&str>) -> Result<OutputDevice> {
        OutputDevice::open_with(id, true)
    }

    /// As `open`; without `fall_back`, only the speakers asked for will do.
    pub fn open_with(id: Option<&str>, fall_back: bool) -> Result<OutputDevice> {
        let (mut d, stand_in) = open_or_default("speakers", id, fall_back, OutputDevice::open_exact)?;
        d.stand_in = stand_in;
        Ok(d)
    }

    fn open_exact(id: Option<&str>) -> Result<OutputDevice> {
        let device = find_device(false, id)?;
        let name = device_label(&device);
        let supported = device.default_output_config().context("speakers have no usable format")?;
        let format = supported.sample_format();
        let config = supported.config();
        let rate = config.sample_rate;
        let channels = config.channels.max(1) as usize;
        let capacity = rate as usize * channels / 2; // 500 ms
        let (prod, cons) = rtrb::RingBuffer::new(capacity);
        let health = Arc::new(Health::default());
        let h = health.clone();
        let stream = match format {
            SampleFormat::F32 => build_output::<f32>(&device, config, cons, h)?,
            SampleFormat::I16 => build_output::<i16>(&device, config, cons, h)?,
            SampleFormat::I32 => build_output::<i32>(&device, config, cons, h)?,
            SampleFormat::U16 => build_output::<u16>(&device, config, cons, h)?,
            SampleFormat::F64 => build_output::<f64>(&device, config, cons, h)?,
            other => return Err(anyhow!("unsupported speaker sample format {other:?}")),
        };
        stream.play()?;
        Ok(OutputDevice::new(Box::new(stream), prod, capacity, rate, channels, health, name))
    }

    fn new(
        stream: Box<dyn Send>,
        prod: rtrb::Producer<f32>,
        capacity: usize,
        rate: u32,
        channels: usize,
        health: Arc<Health>,
        name: String,
    ) -> OutputDevice {
        OutputDevice {
            _stream: stream,
            prod,
            capacity,
            rate,
            channels,
            rs_l: Resampler::new(48_000, rate),
            rs_r: Resampler::new(48_000, rate),
            l: Vec::with_capacity(1024),
            r: Vec::with_capacity(1024),
            tmp_l: Vec::with_capacity(1024),
            tmp_r: Vec::with_capacity(1024),
            watch: Watch::new(health),
            name,
            stand_in: false,
        }
    }

    /// Why the speakers should be reopened, if they should (see `Health`).
    pub fn trouble(&mut self, stall: Duration) -> Option<String> {
        self.watch.trouble(Instant::now(), stall)
    }

    pub fn buffered_ms(&self) -> f32 {
        let used = self.capacity - self.prod.slots();
        used as f32 / self.channels as f32 / self.rate as f32 * 1000.0
    }

    /// Queue 10 ms of 48 kHz stereo (480 interleaved frames).
    pub fn write(&mut self, stereo: &[f32]) {
        self.l.clear();
        self.r.clear();
        for i in 0..480 {
            self.l.push(stereo[2 * i]);
            self.r.push(stereo[2 * i + 1]);
        }
        self.tmp_l.clear();
        self.tmp_r.clear();
        self.rs_l.process(&self.l, &mut self.tmp_l);
        self.rs_r.process(&self.r, &mut self.tmp_r);
        let frames = self.tmp_l.len().min(self.tmp_r.len());
        let n = frames * self.channels;
        if self.prod.slots() < n {
            return; // the device stalled; drop rather than add latency
        }
        if let Ok(mut chunk) = self.prod.write_chunk_uninit(n) {
            let (a, b) = chunk.as_mut_slices();
            let mut i = 0usize;
            let ch = self.channels;
            let (tl, tr) = (&self.tmp_l, &self.tmp_r);
            let put = |slot: &mut std::mem::MaybeUninit<f32>, idx: usize| {
                let frame = idx / ch;
                let c = idx % ch;
                let v = match (ch, c) {
                    (1, _) => 0.5 * (tl[frame] + tr[frame]),
                    (_, 0) => tl[frame],
                    (_, 1) => tr[frame],
                    _ => 0.0,
                };
                slot.write(v);
            };
            for s in a.iter_mut() {
                put(s, i);
                i += 1;
            }
            for s in b.iter_mut() {
                put(s, i);
                i += 1;
            }
            unsafe { chunk.commit_all() };
        }
    }
}

/// Stand-in sound cards for tests. A thread plays the sound card's part at real-time pace;
/// `state` makes it go wrong the ways real ones do.
#[cfg(test)]
pub mod fake {
    use std::sync::atomic::AtomicU8;

    use super::*;

    /// Working normally.
    pub const OK: u8 = 0;
    /// Stops calling back without a word (what Windows does when a virtual device restarts).
    pub const STUCK: u8 = 1;
    /// Reports that the device went away, then stops (what cpal does when it notices).
    pub const GONE: u8 = 2;

    /// Stops the stand-in's thread when the device is dropped.
    struct Running(Arc<AtomicBool>);

    impl Drop for Running {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }

    fn card(name: &str, state: Arc<AtomicU8>, mut every_10ms: impl FnMut() + Send + 'static) -> (Box<dyn Send>, Arc<Health>) {
        let health = Arc::new(Health::default());
        let stop = Arc::new(AtomicBool::new(false));
        let (h, s) = (health.clone(), stop.clone());
        std::thread::Builder::new()
            .name(format!("fake {name}"))
            .spawn(move || {
                let start = Instant::now();
                let mut done = 0u64;
                let mut said = false;
                while !s.load(Ordering::Relaxed) {
                    let due = start.elapsed().as_millis() as u64 / 10;
                    while done < due {
                        done += 1;
                        match state.load(Ordering::Relaxed) {
                            OK => {
                                h.tick();
                                every_10ms();
                            }
                            GONE if !said => {
                                said = true;
                                h.on_error("fake", cpal::Error::new(cpal::ErrorKind::DeviceNotAvailable));
                            }
                            _ => {}
                        }
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            })
            .expect("spawn a fake sound card");
        (Box::new(Running(stop)), health)
    }

    /// A microphone handing over 10 ms of 48 kHz mono from `make` every 10 ms.
    pub fn microphone(name: &str, state: Arc<AtomicU8>, mut make: impl FnMut(&mut [f32]) + Send + 'static) -> InputDevice {
        let (mut prod, cons) = rtrb::RingBuffer::new(48_000);
        let mut buf = vec![0f32; 480];
        let (keep, health) = card(name, state, move || {
            make(&mut buf);
            for v in &buf {
                let _ = prod.push(*v);
            }
        });
        InputDevice::new(keep, cons, 48_000, health, name.to_string())
    }

    /// Speakers taking 10 ms of 48 kHz stereo every 10 ms and handing it to `hear`.
    pub fn speakers(name: &str, state: Arc<AtomicU8>, mut hear: impl FnMut(&[f32]) + Send + 'static) -> OutputDevice {
        let capacity = 48_000; // 500 ms of stereo
        let (prod, mut cons) = rtrb::RingBuffer::new(capacity);
        let mut buf = vec![0f32; 960];
        let (keep, health) = card(name, state, move || {
            buf.fill(0.0);
            let n = cons.slots().min(960);
            if let Ok(chunk) = cons.read_chunk(n) {
                let (a, b) = chunk.as_slices();
                for (d, s) in buf.iter_mut().zip(a.iter().chain(b.iter())) {
                    *d = *s;
                }
                chunk.commit_all();
            }
            hear(&buf);
        });
        OutputDevice::new(keep, prod, capacity, 48_000, 2, health, name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU8;

    use super::*;

    #[test]
    fn notices_a_stream_that_stops_or_goes_away() {
        let state = Arc::new(AtomicU8::new(fake::OK));
        let mut mic = fake::microphone("mic", state.clone(), |b| b.fill(0.1));
        let stall = Duration::from_millis(300);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(mic.trouble(stall), None);
        let mut frame = [0f32; 480];
        assert!(mic.read_frame(&mut frame), "the stand-in microphone hands over sound");

        // Stuck: no word from the device, it just stops calling back.
        state.store(fake::STUCK, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(mic.trouble(stall), None, "not after a moment");
        std::thread::sleep(Duration::from_millis(350));
        let why = mic.trouble(stall).expect("a stuck stream is noticed");
        assert!(why.contains("no sound went through"), "{why}");

        // Gone: the stream says so, which counts at once.
        let state = Arc::new(AtomicU8::new(fake::OK));
        let mut out = fake::speakers("speakers", state.clone(), |_| {});
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(out.trouble(stall), None);
        state.store(fake::GONE, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(60));
        let why = out.trouble(Duration::from_secs(60)).expect("a stream that went away is noticed");
        assert!(why.contains("not available"), "{why}");
    }

    #[test]
    fn glitches_dont_count_as_trouble() {
        let health = Arc::new(Health::default());
        let mut w = Watch::new(health.clone());
        health.tick();
        health.on_error("test", cpal::Error::new(cpal::ErrorKind::Xrun));
        health.on_error("test", cpal::Error::new(cpal::ErrorKind::RealtimeDenied));
        health.on_error("test", cpal::Error::new(cpal::ErrorKind::BackendError));
        assert_eq!(w.trouble(Instant::now(), Duration::from_secs(1)), None);
        health.on_error("test", cpal::Error::new(cpal::ErrorKind::StreamInvalidated));
        assert!(w.trouble(Instant::now(), Duration::from_secs(1)).is_some());
    }

    #[test]
    fn a_new_stream_gets_time_to_start() {
        let mut w = Watch::new(Arc::new(Health::default()));
        let t0 = w.since;
        assert_eq!(w.trouble(t0 + Duration::from_secs(2), Duration::from_secs(1)), None);
        assert!(w.trouble(t0 + START_GRACE, Duration::from_secs(1)).unwrap().contains("never started"));
    }
}
