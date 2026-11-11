//! Notification sounds: new messages, people joining and leaving voice, screen shares
//! starting and stopping, tuning in to one, and calls ringing. Two sets to choose from (Settings
//! → Notifications → Sound profile, 0.13.2): Classic, the short chimes made here (what everyone
//! starts with), and Ember, Pyr's own (recorded off 0.13.28 as it plays them, sample for sample);
//! a skin's own sounds play in place of either. They play on the chosen speakers on their own
//! little stream, so they work in and out of calls.

use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use crate::audio::device::OutputDevice;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sound {
    Message,
    /// Someone joins my voice channel: it stands out (0.13.23), so nobody comes in unheard.
    Join,
    /// Someone leaves it.
    Leave,
    /// I join a voice channel (the chime everyone's joining had before 0.13.23).
    YouJoin,
    /// I leave one.
    YouLeave,
    StreamStart,
    StreamStop,
    WatchStart,
    WatchStop,
    /// Someone else starts watching a stream in my call (or stops).
    ViewerJoin,
    ViewerLeave,
    /// Someone's calling you (played every couple of seconds while it rings).
    Ring,
    /// You're calling someone and it's ringing at their end.
    Ringback,
    /// I mute myself with the mute shortcut: a step down; unmute: a step up (0.13.20).
    Mute,
    Unmute,
}

/// What to play: one of the chimes, 48 kHz stereo (a soundboard sound heard only here), or a
/// ringtone's sound (48 kHz stereo that plays like a chime).
enum Clip {
    /// A chime: Ember's (true) or Classic's.
    Chime(Sound, bool),
    Stereo(Arc<Vec<f32>>),
    Ring(Arc<Vec<f32>>),
}

enum Msg {
    Play { clip: Clip, device: Option<String>, volume: f32 },
    /// Stop the soundboard sounds playing here.
    Hush,
    /// A skin's sounds (48 kHz stereo) in place of these chimes.
    Overrides(HashMap<Sound, Arc<Vec<f32>>>),
}

impl Sound {
    /// As 0.13.28 lists them (`pyr_proto::skin::SOUNDS`).
    pub const ALL: [Sound; 15] = [
        Sound::Message,
        Sound::Join,
        Sound::Leave,
        Sound::YouJoin,
        Sound::YouLeave,
        Sound::StreamStart,
        Sound::StreamStop,
        Sound::WatchStart,
        Sound::WatchStop,
        Sound::ViewerJoin,
        Sound::ViewerLeave,
        Sound::Ring,
        Sound::Ringback,
        Sound::Mute,
        Sound::Unmute,
    ];

    /// Its name in a skin (`pyr_proto::skin::SOUNDS`).
    pub fn name(self) -> &'static str {
        match self {
            Sound::Message => "message",
            Sound::Join => "join",
            Sound::Leave => "leave",
            Sound::YouJoin => "you_join",
            Sound::YouLeave => "you_leave",
            Sound::StreamStart => "stream_start",
            Sound::StreamStop => "stream_stop",
            Sound::WatchStart => "watch_start",
            Sound::WatchStop => "watch_stop",
            Sound::ViewerJoin => "viewer_join",
            Sound::ViewerLeave => "viewer_leave",
            Sound::Ring => "ring",
            Sound::Ringback => "ringback",
            Sound::Mute => "mute",
            Sound::Unmute => "unmute",
        }
    }

    /// A skin's sound that plays for this one when the skin hasn't it (as 0.13.28's skins have
    /// it: `you_join` is the skin's `join` if it hasn't one of its own, and so on).
    pub fn stands_in(self) -> Option<Sound> {
        match self {
            Sound::YouJoin => Some(Sound::Join),
            Sound::YouLeave => Some(Sound::Leave),
            Sound::ViewerJoin => Some(Sound::WatchStart),
            Sound::ViewerLeave => Some(Sound::WatchStop),
            _ => None,
        }
    }

    pub fn from_name(name: &str) -> Option<Sound> {
        Sound::ALL.into_iter().find(|s| s.name() == name)
    }
}

pub struct Sfx {
    tx: mpsc::Sender<Msg>,
}

const RATE: f32 = 48_000.0;

impl Sfx {
    pub fn start() -> Sfx {
        let (tx, rx) = mpsc::channel::<Msg>();
        let _ = std::thread::Builder::new().name("sfx".into()).spawn(move || run(rx));
        Sfx { tx }
    }

    /// `ember`: the Ember sound (`Settings::ember_sounds`), or Classic's.
    pub fn play(&self, sound: Sound, ember: bool, device: Option<String>, volume: f32) {
        let _ = self.tx.send(Msg::Play { clip: Clip::Chime(sound, ember), device, volume });
    }

    /// 48 kHz stereo (a soundboard sound tried out away from a call).
    pub fn play_stereo(&self, pcm: Arc<Vec<f32>>, device: Option<String>, volume: f32) {
        let _ = self.tx.send(Msg::Play { clip: Clip::Stereo(pcm), device, volume });
    }

    /// A ringtone's sound (48 kHz stereo): it plays out like a chime.
    pub fn play_ring_clip(&self, pcm: Arc<Vec<f32>>, device: Option<String>, volume: f32) {
        let _ = self.tx.send(Msg::Play { clip: Clip::Ring(pcm), device, volume });
    }

    /// Stop the soundboard sounds playing (chimes finish).
    pub fn hush(&self) {
        let _ = self.tx.send(Msg::Hush);
    }

    /// A skin's sounds in place of these chimes (an empty list: the chimes again).
    pub fn set_overrides(&self, sounds: Vec<(Sound, Arc<Vec<f32>>)>) {
        let _ = self.tx.send(Msg::Overrides(sounds.into_iter().collect()));
    }
}

/// A clip as 48 kHz stereo, and whether it's a chime (a skin's sound in place of one counts
/// as one).
fn stereo(clip: Clip, skin: &HashMap<Sound, Arc<Vec<f32>>>) -> (Arc<Vec<f32>>, bool) {
    match clip {
        Clip::Chime(s, ember_on) => match skin.get(&s).or_else(|| s.stands_in().and_then(|o| skin.get(&o))) {
            Some(pcm) => (pcm.clone(), true),
            None => match ember_on.then(|| ember(s)).flatten() {
                Some(pcm) => (pcm, true),
                None => (Arc::new(render(s).into_iter().flat_map(|v| [v, v]).collect()), true),
            },
        },
        Clip::Stereo(pcm) => (pcm, false),
        Clip::Ring(pcm) => (pcm, true),
    }
}

/// The Ember sounds (0.13.2): 0.13.28 makes them as it plays them; these are what it played,
/// recorded off its sound card at full volume (48 kHz stereo, 24-bit FLAC, every sample).
macro_rules! ember_file {
    ($name:literal) => {
        ($name, include_bytes!(concat!("../assets/ember/", $name, ".flac")) as &[u8])
    };
}
const EMBER: [(&str, &[u8]); 15] = [
    ember_file!("message"),
    ember_file!("join"),
    ember_file!("leave"),
    ember_file!("you_join"),
    ember_file!("you_leave"),
    ember_file!("stream_start"),
    ember_file!("stream_stop"),
    ember_file!("watch_start"),
    ember_file!("watch_stop"),
    ember_file!("viewer_join"),
    ember_file!("viewer_leave"),
    ember_file!("ring"),
    ember_file!("ringback"),
    ember_file!("mute"),
    ember_file!("unmute"),
];

/// An Ember sound, 48 kHz stereo (read the first time it's wanted).
pub fn ember(s: Sound) -> Option<Arc<Vec<f32>>> {
    static READ: std::sync::OnceLock<parking_lot::Mutex<HashMap<Sound, Arc<Vec<f32>>>>> = std::sync::OnceLock::new();
    let read = READ.get_or_init(Default::default);
    if let Some(p) = read.lock().get(&s) {
        return Some(p.clone());
    }
    let (_, file) = EMBER.iter().find(|(n, _)| *n == s.name())?;
    match crate::soundboard::import(file.to_vec(), Some("flac")) {
        Ok(i) => {
            let pcm = Arc::new(i.pcm);
            read.lock().insert(s, pcm.clone());
            Some(pcm)
        }
        Err(e) => {
            tracing::warn!("couldn't read Ember's {} sound: {e:#}", s.name());
            None
        }
    }
}

/// Notes: (frequency Hz, start ms, length ms, loudness).
fn notes(s: Sound) -> &'static [(f32, f32, f32, f32)] {
    match s {
        // A soft two-note "ding".
        Sound::Message => &[(987.8, 0.0, 180.0, 0.55), (1318.5, 70.0, 260.0, 0.45)],
        // Someone coming in: up the same three notes, one higher, and a chord that rings, louder
        // than the rest (0.13.23; 0.13.28's notes, read out of its program).
        Sound::Join => &[
            (523.3, 0.0, 120.0, 0.6),
            (659.3, 70.0, 120.0, 0.6),
            (784.0, 140.0, 120.0, 0.65),
            (1046.5, 210.0, 560.0, 0.7),
            (659.3, 210.0, 560.0, 0.25),
            (784.0, 210.0, 560.0, 0.25),
        ],
        // Up for my joining, down for leaving.
        Sound::YouJoin => &[(523.3, 0.0, 140.0, 0.5), (659.3, 80.0, 140.0, 0.5), (784.0, 160.0, 260.0, 0.5)],
        Sound::Leave | Sound::YouLeave => &[(784.0, 0.0, 140.0, 0.5), (659.3, 80.0, 140.0, 0.5), (523.3, 160.0, 260.0, 0.5)],
        // A bright rising pair for a stream going live, falling when it ends.
        Sound::StreamStart => &[(587.3, 0.0, 120.0, 0.45), (880.0, 90.0, 120.0, 0.45), (1174.7, 180.0, 340.0, 0.45)],
        Sound::StreamStop => &[(1174.7, 0.0, 120.0, 0.45), (880.0, 90.0, 120.0, 0.45), (587.3, 180.0, 300.0, 0.45)],
        // Short blips for tuning in and out of someone's stream.
        Sound::WatchStart | Sound::ViewerJoin => &[(1046.5, 0.0, 90.0, 0.4), (1568.0, 60.0, 180.0, 0.4)],
        Sound::WatchStop | Sound::ViewerLeave => &[(1568.0, 0.0, 90.0, 0.4), (1046.5, 60.0, 180.0, 0.4)],
        // A bright "ring-ring": two quick rising pairs.
        Sound::Ring => &[
            (784.0, 0.0, 120.0, 0.55),
            (1046.5, 95.0, 140.0, 0.55),
            (784.0, 260.0, 120.0, 0.55),
            (1046.5, 355.0, 240.0, 0.55),
        ],
        // Softer and lower, for waiting on the other end.
        Sound::Ringback => &[(523.3, 0.0, 260.0, 0.32), (659.3, 170.0, 420.0, 0.3)],
        // The mute shortcut: a step down for muted, up for unmuted.
        Sound::Mute => &[(523.3, 0.0, 90.0, 0.45), (392.0, 60.0, 170.0, 0.45)],
        Sound::Unmute => &[(392.0, 0.0, 90.0, 0.45), (523.3, 60.0, 170.0, 0.45)],
    }
}

/// The sound as 48 kHz mono samples.
pub fn render(s: Sound) -> Vec<f32> {
    let ns = notes(s);
    let end_ms = ns.iter().map(|n| n.1 + n.2).fold(0.0f32, f32::max);
    let len = (end_ms / 1000.0 * RATE) as usize;
    let mut out = vec![0.0f32; len];
    for &(f, start, dur, amp) in ns {
        let s0 = (start / 1000.0 * RATE) as usize;
        let n = (dur / 1000.0 * RATE) as usize;
        for i in 0..n {
            let t = i as f32 / RATE;
            // Quick attack, smooth decay; a little second harmonic for a bell-ish tone.
            let attack = (i as f32 / (0.004 * RATE)).min(1.0);
            let decay = (-t * 9.0 / (dur / 1000.0).max(0.05)).exp();
            let fade = 1.0 - (i as f32 / n as f32).powi(8);
            let w = (std::f32::consts::TAU * f * t).sin() + 0.25 * (std::f32::consts::TAU * 2.0 * f * t).sin();
            if let Some(o) = out.get_mut(s0 + i) {
                *o += 0.22 * amp * attack * decay * fade * w;
            }
        }
    }
    out
}

fn run(rx: mpsc::Receiver<Msg>) {
    let mut out: Option<(OutputDevice, Option<String>)> = None;
    // Sounds still playing: (48 kHz stereo, position, volume, a chime).
    let mut playing: Vec<(Arc<Vec<f32>>, usize, f32, bool)> = Vec::new();
    let mut idle_since = Instant::now();
    let mut chunk = vec![0.0f32; 960];
    let mut skin: HashMap<Sound, Arc<Vec<f32>>> = HashMap::new();
    loop {
        let wait = if playing.is_empty() { Duration::from_millis(500) } else { Duration::from_millis(0) };
        let mut next = match rx.recv_timeout(wait) {
            Ok(m) => Some(m),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        while let Some(m) = next.take() {
            match m {
                Msg::Play { clip, device, volume } => {
                    let same = out.as_ref().map(|(_, d)| *d == device).unwrap_or(false);
                    if !same {
                        out = OutputDevice::open(device.as_deref()).ok().map(|d| (d, device.clone()));
                    }
                    // The same sound twice at once just sounds louder; skip the copy.
                    if playing.len() < 6 {
                        let (pcm, chime) = stereo(clip, &skin);
                        playing.push((pcm, 0, volume.clamp(0.0, 2.0), chime));
                    }
                }
                Msg::Hush => playing.retain(|p| p.3),
                Msg::Overrides(sounds) => skin = sounds,
            }
            next = rx.try_recv().ok();
        }
        if playing.is_empty() {
            // Let go of the speakers when nothing has played for a while.
            if out.is_some() && idle_since.elapsed() > Duration::from_secs(5) {
                out = None;
            }
            continue;
        }
        // Speakers that stopped taking sound (see `device::Health`) would keep every sound
        // after this one silent too: let them go, and open them again for the next sound.
        if let Some(why) = out.as_mut().and_then(|(d, _)| d.trouble(Duration::from_secs(1))) {
            tracing::warn!("sound effects: the speakers stopped ({why}); reopening them for the next sound");
            crate::drop_later(out.take());
            playing.clear();
            continue;
        }
        let Some((dev, _)) = out.as_mut() else {
            playing.clear();
            continue;
        };
        // Keep ~40 ms queued.
        while !playing.is_empty() && dev.buffered_ms() < 40.0 {
            for c in chunk.iter_mut() {
                *c = 0.0;
            }
            for (samples, pos, vol, _) in playing.iter_mut() {
                for (i, c) in chunk.iter_mut().enumerate() {
                    *c += samples.get(*pos + i).copied().unwrap_or(0.0) * *vol;
                }
                *pos += 960;
            }
            for c in chunk.iter_mut() {
                *c = c.clamp(-1.0, 1.0);
            }
            dev.write(&chunk);
            playing.retain(|(s, pos, _, _)| *pos < s.len());
        }
        idle_since = Instant::now();
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_are_short_and_not_too_loud() {
        for s in Sound::ALL {
            let v = render(s);
            let peak = v.iter().fold(0.0f32, |a, x| a.max(x.abs()));
            assert!(v.len() > 4800 && v.len() < 48_000, "{s:?} {}", v.len());
            assert!(peak > 0.05 && peak < 0.6, "{s:?} peak {peak}");
        }
    }

    #[test]
    fn a_skins_sound_plays_in_place_of_a_chime() {
        for s in Sound::ALL {
            assert_eq!(Sound::from_name(s.name()), Some(s));
            assert!(pyr_proto::skin::SOUNDS.iter().any(|(n, _)| *n == s.name()), "{s:?}");
        }
        assert_eq!(pyr_proto::skin::SOUNDS.len(), Sound::ALL.len());
        let mine = Arc::new(vec![0.25f32; 9600]);
        let skin: HashMap<Sound, Arc<Vec<f32>>> = [(Sound::Message, mine.clone())].into();
        for ember_on in [false, true] {
            let (pcm, chime) = stereo(Clip::Chime(Sound::Message, ember_on), &skin);
            assert!(Arc::ptr_eq(&pcm, &mine) && chime, "still a chime: it isn't hushed with the soundboard");
        }
        let (pcm, _) = stereo(Clip::Chime(Sound::Join, false), &skin);
        assert_eq!(pcm.len(), render(Sound::Join).len() * 2, "the others stay Pyr's own");
        let (pcm, _) = stereo(Clip::Chime(Sound::Join, true), &skin);
        assert!(Arc::ptr_eq(&pcm, &ember(Sound::Join).unwrap()), "or Ember's");
        // (A skin with only `join`: it's mine joining too, as before `you_join`.)
        let joining = Arc::new(vec![0.5f32; 4800]);
        let skin: HashMap<Sound, Arc<Vec<f32>>> = [(Sound::Join, joining.clone())].into();
        assert!(Arc::ptr_eq(&stereo(Clip::Chime(Sound::YouJoin, false), &skin).0, &joining));
        assert!(!Arc::ptr_eq(&stereo(Clip::Chime(Sound::YouLeave, false), &skin).0, &joining));
    }

    /// Ember's sounds, as 0.13.28 played them at full volume (recorded off it): every one there,
    /// stereo, as long as it was, and the first samples as they came (a stand-in for "the same
    /// file": they're 24-bit, so within 2^-23 of what was recorded).
    #[test]
    fn embers_sounds_are_0_13_28s() {
        let lengths = [
            (Sound::Message, 36_174),
            (Sound::Join, 40_908),
            (Sound::Leave, 37_724),
            (Sound::YouJoin, 50_893),
            (Sound::YouLeave, 50_189),
            (Sound::StreamStart, 57_656),
            (Sound::StreamStop, 33_529),
            (Sound::WatchStart, 28_016),
            (Sound::WatchStop, 28_255),
            (Sound::ViewerJoin, 29_844),
            (Sound::ViewerLeave, 22_964),
            (Sound::Ring, 87_712),
            (Sound::Ringback, 61_033),
            (Sound::Mute, 19_395),
            (Sound::Unmute, 24_141),
        ];
        for (s, frames) in lengths {
            let pcm = ember(s).unwrap_or_else(|| panic!("{s:?}"));
            assert_eq!(pcm.len(), frames * 2, "{s:?}");
            let peak = pcm.iter().fold(0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.15 && peak < 0.5, "{s:?} {peak}");
            assert!(pcm.chunks(2).any(|f| f[0] != f[1]), "{s:?}: stereo");
        }
        // Message's first samples, left and right, as 0.13.28 played them (the right a moment
        // behind the left).
        let m = ember(Sound::Message).unwrap();
        let want = [0.0, 0.0, 0.002_267_12, 0.0, 0.005_064_72, 0.0, 0.009_228_36, 0.0];
        for (i, w) in want.iter().enumerate() {
            assert!((m[i] - w).abs() < 2e-7, "sample {i}: {} vs {w}", m[i]);
        }
    }

    /// Someone coming in is heard over the rest (0.13.23): longer, louder, and up to a high C;
    /// my own joining is the chime it always was.
    #[test]
    fn someone_joining_stands_out() {
        let peak = |s: Sound| render(s).iter().fold(0.0f32, |a, x| a.max(x.abs()));
        assert!(peak(Sound::Join) > peak(Sound::YouJoin) * 1.2, "{} vs {}", peak(Sound::Join), peak(Sound::YouJoin));
        assert!(render(Sound::Join).len() > render(Sound::YouJoin).len() * 3 / 2);
        assert_eq!(notes(Sound::YouJoin), &[(523.3, 0.0, 140.0, 0.5), (659.3, 80.0, 140.0, 0.5), (784.0, 160.0, 260.0, 0.5)]);
        assert_eq!(notes(Sound::Join).iter().map(|n| n.0).fold(0.0f32, f32::max), 1046.5);
    }
}
