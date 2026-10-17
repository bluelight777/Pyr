//! The audio engine: one worker thread that mixes playback, feeds the echo canceller,
//! processes the microphone and hands encoded Opus packets to the network.

pub mod capture;
pub mod dfn;
pub mod device;
pub mod jitter;
pub mod mixer;
pub mod resample;
pub mod spectrum;
pub mod stream;
pub mod stream_echo;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pyr_proto::control::SessionId;
use parking_lot::Mutex;
use tracing::{debug, info, warn};

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
    /// When I last said something (ms since 1970; 0: not yet): talking in a call isn't idle.
    pub last_spoke_ms: std::sync::atomic::AtomicI64,
    /// f32 bits, dBFS
    pub mic_level: AtomicU32,
    /// Play your own processed microphone back to you (mic test).
    pub monitor: AtomicBool,
    pub stop: AtomicBool,
    /// Where what the call plays is copied, for taking it back out of our screen share's
    /// sound (see `stream_echo`).
    pub played: Mutex<Option<Arc<PlayedTap>>>,
    /// My voice as it goes out (only while it does), for my voice meter.
    pub mic_meter: MeterTap,
    /// My screen share's sound as it goes out, for my voice meter.
    pub screen_meter: Arc<MeterTap>,
    /// Voice meters are turned off: nothing is measured for them.
    pub meters_off: AtomicBool,
    /// How loud my microphone is sent (`Settings::input_gain`), changed as it's dragged.
    pub input_gain: Gain,
    /// Voice settings changed while the sound flows (sensitivity, input mode, push to talk's
    /// release delay, voice quality): the microphone takes them up within 10 ms, without
    /// starting the sound again (see `CapturePipeline::retune`).
    pub retune: Mutex<Option<Settings>>,
}

/// A volume that can change while sound flows (an f32's bits); 1.0 to start with.
pub struct Gain(AtomicU32);

impl Default for Gain {
    fn default() -> Self {
        Gain(AtomicU32::new(1f32.to_bits()))
    }
}

impl Gain {
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }
}

/// How loud something sounds right now and what it sounds like, for voice meters: its level
/// (dBFS) and how strong each band is, deep to bright (0 to 1, see `spectrum`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Meter {
    pub db: f32,
    pub bands: [f32; spectrum::BANDS],
}

impl Default for Meter {
    fn default() -> Self {
        Meter { db: -100.0, bands: [0.0; spectrum::BANDS] }
    }
}

impl Meter {
    /// Two sounds at once (someone's voice and their screen's sound): the louder of each.
    pub fn merge(&mut self, other: &Meter) {
        self.db = self.db.max(other.db);
        for (a, b) in self.bands.iter_mut().zip(other.bands) {
            *a = a.max(b);
        }
    }
}

/// The latest meter reading of something I send, and when it was taken (one that's older
/// than [`METER_FRESH`] means it has gone quiet).
#[derive(Default)]
pub struct MeterTap(Mutex<Option<(Meter, Instant)>>);

/// How long a meter reading counts: after that without a new one, it's quiet.
pub const METER_FRESH: Duration = Duration::from_millis(120);

impl MeterTap {
    pub fn set(&self, m: Meter, at: Instant) {
        *self.0.lock() = Some((m, at));
    }

    /// The reading, if it's recent.
    pub fn get(&self, now: Instant) -> Option<Meter> {
        self.0.lock().filter(|(_, at)| now.saturating_duration_since(*at) < METER_FRESH).map(|(m, _)| m)
    }
}

/// What a call plays, kept for taking it back out of our screen share's sound
/// (`stream_echo`), while a screen share with sound is running.
#[derive(Default)]
pub struct PlayedTap {
    listeners: std::sync::atomic::AtomicUsize,
    queue: Mutex<Vec<f32>>,
}

impl PlayedTap {
    /// Most kept waiting (1 s of 48 kHz stereo): more means nobody's taking it.
    const MAX: usize = 96_000;

    pub fn subscribe(&self) {
        if self.listeners.fetch_add(1, Ordering::SeqCst) == 0 {
            self.queue.lock().clear();
        }
    }

    pub fn unsubscribe(&self) {
        let _ = self.listeners.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1));
    }

    /// The call played this (48 kHz stereo, interleaved).
    pub fn push(&self, x: &[f32]) {
        if self.listeners.load(Ordering::Relaxed) == 0 {
            return;
        }
        let mut q = self.queue.lock();
        if q.len() + x.len() <= Self::MAX {
            q.extend_from_slice(x);
        }
    }

    /// Everything played since the last time.
    pub fn take(&self, out: &mut Vec<f32>) {
        let mut q = self.queue.lock();
        out.extend_from_slice(&q);
        q.clear();
    }
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
    /// A stand-in sound card, made (and made again to reopen it) by this.
    #[cfg(test)]
    Fake(Opener<InputDevice>),
    None,
}

pub enum AudioOutput {
    Device(Option<String>),
    /// Receives 10 ms of 48 kHz stereo (960 interleaved samples) per call.
    Synthetic(Box<dyn FnMut(&[f32]) + Send>),
    /// A stand-in sound card, made (and made again to reopen it) by this.
    #[cfg(test)]
    Fake(Opener<OutputDevice>),
    None,
}

/// Opens a sound device (again, when it has stopped): the one asked for, or with `true`, the
/// default one standing in for it if the one asked for won't open.
pub type Opener<T> = Arc<dyn Fn(bool) -> anyhow::Result<T> + Send + Sync>;

#[derive(Default, Clone, Debug)]
pub struct AudioStatus {
    pub input_name: Option<String>,
    pub output_name: Option<String>,
    pub input_error: Option<String>,
    pub output_error: Option<String>,
    pub speaking: Vec<SessionId>,
    /// How each other session sounds right now (see `Mixer::meters`), every 20 ms.
    pub meters: Vec<(SessionId, Meter)>,
    /// How loud each other session's voice and stream sound are (see `Mixer::track_levels`),
    /// every 20 ms, for the mixer panel (whether or not voice meters are on).
    pub levels: Vec<(SessionId, u8, f32)>,
    /// How loud each other session's voice and stream usually are (see `Mixer::loudness`),
    /// every 20 ms, for the mixer's Even out.
    pub loudness: Vec<(SessionId, u8, f32)>,
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
        let controls = Arc::new(Controls { ptt_down: ptt, meters_off: AtomicBool::new(!settings.voice_meters_on()), ..Default::default() });
        controls.mic_level.store((-100f32).to_bits(), Ordering::Relaxed);
        controls.input_gain.set(settings.input_gain);
        let mixer = Arc::new(Mutex::new(Mixer::default()));
        mixer.lock().master = settings.output_volume;
        mixer.lock().meters = settings.voice_meters_on();
        let status = Arc::new(Mutex::new(AudioStatus::default()));
        let (c, m, s) = (controls.clone(), mixer.clone(), status.clone());
        let thread = std::thread::Builder::new()
            .name("pyr-audio".into())
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
    Dev(Link<InputDevice>),
    Synth(Box<dyn FnMut(&mut [f32]) + Send>),
    None,
}

enum Out {
    Dev(Link<OutputDevice>),
    Synth(Box<dyn FnMut(&[f32]) + Send>),
    Clock,
}

/// Target amount of audio queued at the sound card.
const OUTPUT_TARGET_MS: f32 = 30.0;

/// How long a working sound card may go without calling back before it counts as stuck.
const STALL: Duration = Duration::from_secs(1);
/// A device that stops again within this long of coming back waits longer before the next try.
const SETTLED: Duration = Duration::from_secs(10);
/// A device that has been down this long is mentioned on the call screen (and in a message).
const TELL_AFTER: Duration = Duration::from_millis(1500);
/// While a device is down, only the one asked for is tried at first (it's usually back in a
/// moment); after this long, the default device may stand in for it.
const FALL_BACK_AFTER: Duration = if cfg!(test) { Duration::from_secs(2) } else { Duration::from_secs(10) };
/// How often a stand-in checks whether the device asked for is back.
const SWITCH_BACK_EVERY: Duration = if cfg!(test) { Duration::from_secs(1) } else { Duration::from_secs(5) };

/// How long to wait before opening a device again, after `failures` tries in a row.
fn retry_delay(failures: u32) -> Duration {
    match failures {
        0 => Duration::ZERO,
        1 => Duration::from_millis(500),
        2 => Duration::from_secs(1),
        3 => Duration::from_secs(2),
        _ => Duration::from_secs(5),
    }
}

/// Whether the n-th failure in a row gets a line in the log: the first few, then fewer and
/// fewer (a PC without a microphone would otherwise say so every 5 s).
fn worth_logging(n: u32) -> bool {
    n <= 3 || n.is_power_of_two()
}

/// A microphone or speakers, as the audio thread needs them.
trait Device: Send + 'static {
    fn name(&self) -> &str;
    /// The default device, playing the part of the one asked for.
    fn stand_in(&self) -> bool;
    /// Why it should be reopened, if it should.
    fn trouble(&mut self, stall: Duration) -> Option<String>;
}

impl Device for InputDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn stand_in(&self) -> bool {
        self.stand_in
    }

    fn trouble(&mut self, stall: Duration) -> Option<String> {
        InputDevice::trouble(self, stall)
    }
}

impl Device for OutputDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn stand_in(&self) -> bool {
        self.stand_in
    }

    fn trouble(&mut self, stall: Duration) -> Option<String> {
        OutputDevice::trouble(self, stall)
    }
}

/// What happened to a device this time round.
#[derive(Debug, PartialEq)]
enum Change {
    None,
    /// It stopped working.
    Lost,
    /// It works again, for the first time, or as a different device.
    Back,
    /// It has been down a while: time to tell the user.
    Tell,
}

/// A sound device, kept working. When it stops (or wasn't there to begin with) it's reopened
/// in the background, each try a little later than the last, until it's back; the call and
/// the other device carry on meanwhile. Before this, a microphone or speakers that stopped
/// (a virtual device restarting, a USB hiccup) stayed silent until Pyr was restarted.
///
/// If the device asked for stays away, the default one stands in for it until it's back.
struct Link<T: Device> {
    /// "speakers" or "microphone", for the log.
    what: &'static str,
    open: Opener<T>,
    dev: Option<T>,
    /// The device's name, as last seen.
    name: String,
    up_since: Option<Instant>,
    down_since: Option<Instant>,
    /// Failed tries (and comebacks that didn't last) in a row.
    failures: u32,
    next_try: Instant,
    /// When a stand-in next looks for the device asked for.
    next_check: Instant,
    /// From when the default device may stand in for the one asked for, while it's down.
    fall_back_from: Instant,
    /// A try in progress (on a thread of its own: a stuck driver can take seconds to answer).
    trying: Option<mpsc::Receiver<anyhow::Result<T>>>,
    told: bool,
}

impl<T: Device> Link<T> {
    /// Open the device now (before the first sound). If that fails, the error comes back and
    /// the device is tried again later.
    fn start(what: &'static str, open: Opener<T>) -> (Link<T>, Option<String>) {
        let now = Instant::now();
        let mut link = Link {
            what,
            open,
            dev: None,
            name: what.to_string(),
            up_since: None,
            down_since: None,
            failures: 0,
            next_try: now,
            next_check: now + SWITCH_BACK_EVERY,
            fall_back_from: now,
            trying: None,
            told: false,
        };
        match (link.open)(true) {
            Ok(d) => {
                info!("{what}: {}", d.name());
                link.name = d.name().to_string();
                link.dev = Some(d);
                link.up_since = Some(now);
                (link, None)
            }
            Err(e) => {
                let e = format!("{e:#}");
                warn!("{what} unavailable: {e}");
                link.down_since = Some(now);
                link.failures = 1;
                link.next_try = now + retry_delay(1);
                // The caller shows this error.
                link.told = true;
                (link, Some(e))
            }
        }
    }

    /// Check on the device, and work on getting it back if it's down. Call it all the time.
    fn step(&mut self, now: Instant) -> Change {
        if let Some(rx) = &self.trying {
            match rx.try_recv() {
                Ok(Ok(d)) => {
                    self.trying = None;
                    return self.got(d, now);
                }
                Ok(Err(e)) => self.no_luck(now, format!("{e:#}")),
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.no_luck(now, "opening it went wrong".into()),
            }
        }
        if let Some(d) = self.dev.as_mut() {
            if let Some(why) = d.trouble(STALL) {
                let settled = self.up_since.is_some_and(|t| now.duration_since(t) >= SETTLED);
                self.failures = if settled { 0 } else { self.failures + 1 };
                if worth_logging(self.failures.max(1)) {
                    warn!("{} stopped working ({}): {why}; reconnecting", self.what, self.name);
                }
                // Letting go of a stuck stream can take a moment too.
                crate::drop_later(self.dev.take());
                self.up_since = None;
                self.down_since = Some(now);
                self.next_try = now + retry_delay(self.failures);
                self.fall_back_from = now + FALL_BACK_AFTER;
                self.told = false;
                return Change::Lost;
            }
            // Standing in for the device asked for: see now and then whether it's back.
            if d.stand_in() && self.trying.is_none() && now >= self.next_check {
                self.next_check = now + SWITCH_BACK_EVERY;
                self.try_open(false, now);
            }
            return Change::None;
        }
        if self.trying.is_none() && now >= self.next_try {
            self.try_open(now >= self.fall_back_from, now);
        }
        if !self.told && self.down_since.is_some_and(|t| now.duration_since(t) >= TELL_AFTER) {
            self.told = true;
            return Change::Tell;
        }
        Change::None
    }

    fn try_open(&mut self, fall_back: bool, now: Instant) {
        let (tx, rx) = mpsc::channel();
        let open = self.open.clone();
        let spawned = std::thread::Builder::new().name(format!("pyr-open-{}", self.what)).spawn(move || {
            let _ = tx.send(open(fall_back));
        });
        match spawned {
            Ok(_) => self.trying = Some(rx),
            Err(_) => self.next_try = now + Duration::from_secs(1),
        }
    }

    /// A try came back with a device.
    fn got(&mut self, d: T, now: Instant) -> Change {
        if let Some(cur) = &self.dev {
            // A look for the device asked for, while another stands in for it.
            if d.stand_in() || !cur.stand_in() {
                crate::drop_later(d);
                return Change::None;
            }
            info!("{} “{}” is back; switching to it from “{}”", self.what, d.name(), cur.name());
            crate::drop_later(self.dev.take());
        } else {
            let down = self.down_since.map(|t| now.duration_since(t)).unwrap_or_default();
            let note = if d.stand_in() { " (standing in for the one chosen)" } else { "" };
            info!("{} back after {:.1} s: {}{note}", self.what, down.as_secs_f32(), d.name());
        }
        self.name = d.name().to_string();
        self.dev = Some(d);
        self.up_since = Some(now);
        self.down_since = None;
        self.next_check = now + SWITCH_BACK_EVERY;
        self.told = false;
        Change::Back
    }

    /// A try failed.
    fn no_luck(&mut self, now: Instant, e: String) {
        self.trying = None;
        if self.dev.is_some() {
            // Only a look for the device asked for; the stand-in carries on.
            debug!("the {} chosen still won't open: {e}", self.what);
            return;
        }
        self.failures += 1;
        self.next_try = now + retry_delay(self.failures);
        if worth_logging(self.failures) {
            warn!("{} still unavailable (try {}): {e}", self.what, self.failures);
        }
    }
}

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

/// What the status says about a device, kept in step with its `Link`: the error is cleared
/// again once the device is back (only if it's still the one this put there).
fn report<T: Device>(change: &Change, link: &Link<T>, name: &mut Option<String>, error: &mut Option<String>, told: &mut Option<String>, down: &str) {
    match change {
        Change::None | Change::Lost => {}
        Change::Back => {
            *name = Some(link.name.clone());
            if error.is_some() && *error == *told {
                *error = None;
            }
            *told = None;
        }
        Change::Tell => {
            *error = Some(down.to_string());
            *told = Some(down.to_string());
        }
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
    // What this thread put in the status as the microphone's or speakers' error.
    let (mut told_in, mut told_out): (Option<String>, Option<String>) = (None, None);
    let mic = |open: Opener<InputDevice>, told: &mut Option<String>| {
        let (link, err) = Link::start("microphone", open);
        let mut st = status.lock();
        match err {
            None => st.input_name = Some(link.name.clone()),
            Some(e) => {
                st.input_error = Some(e.clone());
                *told = Some(e);
            }
        }
        In::Dev(link)
    };
    let mut inp = match input {
        AudioInput::Device(id) => mic(Arc::new(move |fall_back| InputDevice::open_with(id.as_deref(), fall_back)), &mut told_in),
        #[cfg(test)]
        AudioInput::Fake(open) => mic(open, &mut told_in),
        AudioInput::Synthetic(f) => In::Synth(f),
        AudioInput::None => In::None,
    };
    let speakers = |open: Opener<OutputDevice>, told: &mut Option<String>| {
        let (link, err) = Link::start("speakers", open);
        let mut st = status.lock();
        match err {
            None => st.output_name = Some(link.name.clone()),
            Some(e) => {
                st.output_error = Some(e.clone());
                *told = Some(e);
            }
        }
        Out::Dev(link)
    };
    let mut out = match output {
        AudioOutput::Device(id) => speakers(Arc::new(move |fall_back| OutputDevice::open_with(id.as_deref(), fall_back)), &mut told_out),
        #[cfg(test)]
        AudioOutput::Fake(open) => speakers(open, &mut told_out),
        AudioOutput::Synthetic(f) => Out::Synth(f),
        AudioOutput::None => Out::Clock,
    };
    // speaker queue + device buffers + mic buffering, roughly
    let echo_delay_ms = (OUTPUT_TARGET_MS as i32) + 30;
    if let (Some(c), Out::Dev(Link { dev: Some(_), .. })) = (cap.as_mut(), &out) {
        c.stream_delay_ms = echo_delay_ms;
    }

    let start = Instant::now();
    let mut cap_frames: u64 = 0;
    let mut out_frames: u64 = 0;
    let mut frame = [0f32; 480];
    let mut mix = vec![0f32; 960];
    let mut monitor: VecDeque<f32> = VecDeque::new();
    let mut last_status = Instant::now();
    let mut last_levels = Instant::now();

    while !ctl.stop.load(Ordering::Relaxed) {
        let now = Instant::now();
        let due = (now.duration_since(start).as_micros() / 10_000) as u64;
        let deaf = ctl.deafened.load(Ordering::Relaxed);
        let monitoring = ctl.monitor.load(Ordering::Relaxed);

        // Playback first, so the echo canceller sees the far end before the near end (only
        // what's really played: with no speakers there's no echo to cancel).
        let produce = |mix: &mut Vec<f32>, cap: &mut Option<CapturePipeline>, monitor: &mut VecDeque<f32>, played: bool| {
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
            if played {
                if let Some(c) = cap.as_mut() {
                    c.render(mix);
                }
                if let Some(t) = ctl.played.lock().as_ref() {
                    t.push(mix);
                }
            }
        };
        match &mut out {
            Out::Dev(link) => {
                let change = link.step(now);
                if change == Change::Back {
                    if let Some(c) = cap.as_mut() {
                        c.stream_delay_ms = echo_delay_ms;
                    }
                }
                if change != Change::None {
                    let down = format!("your speakers “{}” stopped playing sound; reconnecting them…", link.name);
                    let mut st = status.lock();
                    let st = &mut *st;
                    report(&change, link, &mut st.output_name, &mut st.output_error, &mut told_out, &down);
                }
                if let Some(d) = link.dev.as_mut() {
                    let mut guard = 0;
                    while d.buffered_ms() < OUTPUT_TARGET_MS && guard < 10 {
                        produce(&mut mix, &mut cap, &mut monitor, true);
                        d.write(&mix);
                        guard += 1;
                    }
                    out_frames = due;
                } else {
                    // No speakers right now: mix by the clock all the same, so the speaking
                    // rings keep working and nobody's sound piles up for when they're back.
                    while out_frames < due {
                        produce(&mut mix, &mut cap, &mut monitor, false);
                        out_frames += 1;
                    }
                }
            }
            Out::Synth(f) => {
                while out_frames < due {
                    produce(&mut mix, &mut cap, &mut monitor, true);
                    f(&mix);
                    out_frames += 1;
                }
            }
            Out::Clock => {
                while out_frames < due {
                    produce(&mut mix, &mut cap, &mut monitor, false);
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
            In::Dev(link) => {
                let change = link.step(now);
                if change == Change::Lost {
                    // Nothing's being sent: don't leave the meter and your speaking ring
                    // showing whatever they last did.
                    ctl.transmitting.store(false, Ordering::Relaxed);
                    ctl.mic_level.store((-100f32).to_bits(), Ordering::Relaxed);
                }
                if change != Change::None {
                    let down = format!("your microphone “{}” stopped; reconnecting it…", link.name);
                    let mut st = status.lock();
                    let st = &mut *st;
                    report(&change, link, &mut st.input_name, &mut st.input_error, &mut told_in, &down);
                }
                if let Some(d) = link.dev.as_mut() {
                    while d.read_frame(&mut frame) {
                        process(&mut frame, &mut cap, &mut monitor);
                    }
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

        if now.duration_since(last_levels) > Duration::from_millis(20) {
            last_levels = now;
            let (meters, levels, loudness) = {
                let m = mixer.lock();
                (if ctl.meters_off.load(Ordering::Relaxed) { Vec::new() } else { m.meters(now) }, m.track_levels(now), m.loudness())
            };
            let mut st = status.lock();
            st.meters = meters;
            st.levels = levels;
            st.loudness = loudness;
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
        // The microphone's volume changes as it goes (the mixer's MY MIC): all the way down, it
        // goes quiet.
        let loud = engine.controls.mic_level_db();
        assert!(loud > -30.0, "{loud}");
        // (What was sent until then is what's played back below.)
        let sent = packets.lock().len();
        engine.controls.input_gain.set(0.0);
        assert!(wait_until(3.0, || engine.controls.mic_level_db() < -80.0), "{}", engine.controls.mic_level_db());
        engine.stop();
        let all = packets.lock();
        let pk = &all[..sent];
        assert!(pk.len() >= 20, "only {} packets", pk.len());
        assert_eq!(pk[0].1 & pyr_proto::media::flags::TALKSPURT, pyr_proto::media::flags::TALKSPURT);

        // Decode them through the mixer.
        let mut m = Mixer::default();
        let t0 = Instant::now();
        for (i, (_, f, d)) in pk.iter().enumerate() {
            m.push(7, pyr_proto::media::track::MIC, i as u32, *f, d.clone(), t0);
        }
        let mut out = vec![0f32; 960];
        let mut energy = 0f32;
        for k in 0..30 {
            m.mix(&mut out, t0 + Duration::from_millis(100 + k * 10));
            energy += out.iter().map(|v| v * v).sum::<f32>();
        }
        assert!(energy > 10.0, "decoded audio is silent ({energy})");
        assert_eq!(m.speaking(t0 + Duration::from_millis(390)), vec![7]);
        // The mixer panel's meter for their voice: how loud it came, as sent.
        let levels = m.track_levels(t0 + Duration::from_millis(390));
        assert!(matches!(levels.as_slice(), [(7, t, db)] if *t == pyr_proto::media::track::MIC && *db > -30.0), "{levels:?}");
        assert!(m.track_levels(t0 + Duration::from_secs(5)).is_empty(), "quiet later");
    }

    /// Wait (up to `secs`) until `f` holds.
    fn wait_until(secs: f32, mut f: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs_f32(secs);
        while Instant::now() < end {
            if f() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        f()
    }

    /// Speakers and a microphone that stop (the way Windows' do when a virtual device restarts,
    /// or that report they've gone) are reopened by themselves, and the call carries on
    /// meanwhile. Before, you heard nobody (or nobody heard you) until Pyr was restarted.
    #[test]
    fn stopped_speakers_and_microphone_come_back_by_themselves() {
        use std::sync::atomic::{AtomicU64, AtomicU8};

        use device::fake;

        let mut settings = Settings::default();
        settings.noise_suppression = crate::config::NoiseSuppression::Off; // a pure tone isn't speech
        settings.vad_auto = false;
        settings.vad_threshold_db = -40.0;
        settings.echo_cancellation = false;
        settings.auto_gain = false;

        // The microphone plays a tone; each one opened gets its own switch.
        let mics: Arc<Mutex<Vec<Arc<AtomicU8>>>> = Arc::default();
        let open_mic: Opener<InputDevice> = {
            let mics = mics.clone();
            Arc::new(move |_| {
                let state = Arc::new(AtomicU8::new(fake::OK));
                let n = {
                    let mut m = mics.lock();
                    m.push(state.clone());
                    m.len()
                };
                let mut phase = 0usize;
                Ok(fake::microphone(&format!("Mic {n}"), state, move |buf| {
                    for v in buf.iter_mut() {
                        *v = (phase as f32 / 48_000.0 * 440.0 * std::f32::consts::TAU).sin() * 0.25;
                        phase += 1;
                    }
                }))
            })
        };
        // The speakers count the loud 10 ms they play, per device; `refuse` makes the next few
        // tries to open them fail (the device isn't back yet).
        let speakers: Arc<Mutex<Vec<Arc<AtomicU8>>>> = Arc::default();
        let heard: Arc<Mutex<Vec<u32>>> = Arc::default();
        let refuse = Arc::new(AtomicU32::new(0));
        let open_speakers: Opener<OutputDevice> = {
            let (speakers, heard, refuse) = (speakers.clone(), heard.clone(), refuse.clone());
            Arc::new(move |_| {
                if refuse.load(Ordering::Relaxed) > 0 {
                    refuse.fetch_sub(1, Ordering::Relaxed);
                    anyhow::bail!("not there yet");
                }
                let state = Arc::new(AtomicU8::new(fake::OK));
                let n = {
                    let mut s = speakers.lock();
                    s.push(state.clone());
                    heard.lock().push(0);
                    s.len() - 1
                };
                let h = heard.clone();
                Ok(fake::speakers(&format!("Speakers {n}"), state, move |buf| {
                    if buf.iter().map(|v| v * v).sum::<f32>() > 1.0 {
                        h.lock()[n] += 1;
                    }
                }))
            })
        };

        // What the microphone sends comes straight back as somebody else (session 7) talking.
        let sent = Arc::new(AtomicU64::new(0));
        let (tx, rx) = mpsc::channel::<(u8, u8, Vec<u8>)>();
        let s2 = sent.clone();
        let mut engine = AudioEngine::start(
            settings,
            AudioInput::Fake(open_mic),
            AudioOutput::Fake(open_speakers),
            Box::new(move |t, f, data| {
                s2.fetch_add(1, Ordering::Relaxed);
                let _ = tx.send((t, f, data.to_vec()));
            }),
            Arc::new(AtomicBool::new(false)),
        );
        let mixer = engine.mixer.clone();
        std::thread::spawn(move || {
            let mut seq = 0u32;
            while let Ok((t, f, d)) = rx.recv() {
                mixer.lock().push(7, t, seq, f, d, Instant::now());
                seq += 1;
            }
        });
        let status = engine.status.clone();
        let heard_on = |n: usize| heard.lock().get(n).copied().unwrap_or(0);
        let speaking = || status.lock().speaking == vec![7];

        // Working: the tone goes round and is heard.
        assert!(wait_until(3.0, || heard_on(0) > 30 && speaking()), "heard {:?}", heard.lock());
        assert_eq!(status.lock().output_name.as_deref(), Some("Speakers 0"));
        assert_eq!(status.lock().input_name.as_deref(), Some("Mic 1"));

        // The speakers get stuck without a word: within a couple of seconds there are new ones
        // playing, and the speaking ring kept going meanwhile.
        speakers.lock()[0].store(fake::STUCK, Ordering::Relaxed);
        let stuck_at = Instant::now();
        assert!(wait_until(4.0, || speakers.lock().len() == 2), "the speakers weren't reopened");
        let took = stuck_at.elapsed();
        assert!(took < Duration::from_secs(3), "reopening took {took:?}");
        assert!(wait_until(2.0, || heard_on(1) > 30), "nothing heard on the new speakers: {:?}", heard.lock());
        assert!(speaking());
        assert_eq!(status.lock().output_name.as_deref(), Some("Speakers 1"));
        assert_eq!(status.lock().output_error, None, "a quick recovery isn't worth a message");

        // The microphone goes away (and says so): it's reopened, and sound flows again.
        mics.lock()[0].store(fake::GONE, Ordering::Relaxed);
        assert!(wait_until(3.0, || mics.lock().len() == 2), "the microphone wasn't reopened");
        let before = sent.load(Ordering::Relaxed);
        assert!(wait_until(2.0, || sent.load(Ordering::Relaxed) > before + 20), "nothing sent after reopening the microphone");
        assert_eq!(status.lock().input_name.as_deref(), Some("Mic 2"));
        assert_eq!(status.lock().input_error, None);

        // Speakers that stay away a while: the call screen says so (and the mixing carries on),
        // and the message goes once they're back.
        refuse.store(2, Ordering::Relaxed);
        speakers.lock()[1].store(fake::GONE, Ordering::Relaxed);
        assert!(
            wait_until(4.0, || status.lock().output_error.as_deref().is_some_and(|e| e.contains("Speakers 1") && e.contains("reconnecting"))),
            "no message: {:?}",
            status.lock().output_error
        );
        assert!(wait_until(1.0, &speaking), "the speaking ring stopped while the speakers were away");
        assert!(wait_until(12.0, || speakers.lock().len() == 3), "the speakers never came back");
        assert!(wait_until(2.0, || status.lock().output_error.is_none()), "the message stayed: {:?}", status.lock().output_error);
        assert!(wait_until(2.0, || heard_on(2) > 30), "nothing heard on the third speakers: {:?}", heard.lock());
        assert_eq!(refuse.load(Ordering::Relaxed), 0);
        engine.stop();
    }

    /// Speakers chosen that go away for a while: the default ones stand in (after a moment's
    /// wait for the chosen ones), and the chosen ones take over again once they're back.
    #[test]
    fn default_speakers_stand_in_until_the_chosen_ones_are_back() {
        use std::sync::atomic::AtomicU8;

        use device::fake;

        let there = Arc::new(AtomicBool::new(true));
        let states: Arc<Mutex<Vec<(String, Arc<AtomicU8>)>>> = Arc::default();
        let open: Opener<OutputDevice> = {
            let (there, states) = (there.clone(), states.clone());
            Arc::new(move |fall_back| {
                let chosen = there.load(Ordering::Relaxed);
                if !chosen && !fall_back {
                    anyhow::bail!("the speakers chosen aren't there");
                }
                let name = if chosen { "Headset" } else { "Default speakers" };
                let state = Arc::new(AtomicU8::new(fake::OK));
                states.lock().push((name.to_string(), state.clone()));
                let mut d = fake::speakers(name, state, |_| {});
                d.stand_in = !chosen;
                Ok(d)
            })
        };
        let (mut link, err) = Link::start("speakers", open);
        assert_eq!(err, None);
        assert_eq!(link.name, "Headset");
        let run = |secs: f32, link: &mut Link<OutputDevice>| {
            let mut seen = Vec::new();
            let end = Instant::now() + Duration::from_secs_f32(secs);
            while Instant::now() < end {
                let c = link.step(Instant::now());
                if c != Change::None {
                    seen.push((c, link.name.clone()));
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            seen
        };
        run(0.2, &mut link);

        // The headset goes away (unplugged, say).
        there.store(false, Ordering::Relaxed);
        states.lock()[0].1.store(fake::GONE, Ordering::Relaxed);
        let seen = run(FALL_BACK_AFTER.as_secs_f32() + 2.5, &mut link);
        assert_eq!(
            seen,
            vec![(Change::Lost, "Headset".to_string()), (Change::Tell, "Headset".to_string()), (Change::Back, "Default speakers".to_string())],
        );
        assert!(link.dev.as_ref().is_some_and(|d| d.stand_in));

        // It's plugged in again: back to it.
        there.store(true, Ordering::Relaxed);
        let seen = run(SWITCH_BACK_EVERY.as_secs_f32() + 1.0, &mut link);
        assert_eq!(seen, vec![(Change::Back, "Headset".to_string())]);
        assert!(link.dev.as_ref().is_some_and(|d| !d.stand_in));
        let opened: Vec<String> = states.lock().iter().map(|(n, _)| n.clone()).collect();
        assert_eq!(opened, ["Headset", "Default speakers", "Headset"]);
    }

    #[test]
    fn retries_slow_down_and_logging_thins_out() {
        assert_eq!(retry_delay(0), Duration::ZERO);
        assert!(retry_delay(1) < retry_delay(2) && retry_delay(2) < retry_delay(3) && retry_delay(3) < retry_delay(4));
        assert_eq!(retry_delay(40), Duration::from_secs(5));
        let logged: Vec<u32> = (1..=70).filter(|n| worth_logging(*n)).collect();
        assert_eq!(logged, vec![1, 2, 3, 4, 8, 16, 32, 64]);
    }
}
