//! Notification sounds: new messages, people joining and leaving voice, screen shares
//! starting and stopping, tuning in to one, and calls ringing. The sounds are made here (short
//! chimes), not loaded from files, unless a skin brings its own; they play on the chosen
//! speakers on their own little stream, so they work in and out of calls.

use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use crate::audio::device::OutputDevice;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Sound {
    Message,
    Join,
    Leave,
    StreamStart,
    StreamStop,
    WatchStart,
    WatchStop,
    /// Someone's calling you (played every couple of seconds while it rings).
    Ring,
    /// You're calling someone and it's ringing at their end.
    Ringback,
}

/// What to play: one of the chimes, or 48 kHz stereo (a soundboard sound heard only here).
enum Clip {
    Chime(Sound),
    Stereo(Arc<Vec<f32>>),
}

enum Msg {
    Play { clip: Clip, device: Option<String>, volume: f32 },
    /// Stop the soundboard sounds playing here.
    Hush,
    /// A skin's sounds (48 kHz stereo) in place of these chimes.
    Overrides(HashMap<Sound, Arc<Vec<f32>>>),
}

impl Sound {
    pub const ALL: [Sound; 9] =
        [Sound::Message, Sound::Join, Sound::Leave, Sound::StreamStart, Sound::StreamStop, Sound::WatchStart, Sound::WatchStop, Sound::Ring, Sound::Ringback];

    /// Its name in a skin (`ingle_proto::skin::SOUNDS`).
    pub fn name(self) -> &'static str {
        match self {
            Sound::Message => "message",
            Sound::Join => "join",
            Sound::Leave => "leave",
            Sound::StreamStart => "stream_start",
            Sound::StreamStop => "stream_stop",
            Sound::WatchStart => "watch_start",
            Sound::WatchStop => "watch_stop",
            Sound::Ring => "ring",
            Sound::Ringback => "ringback",
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

    pub fn play(&self, sound: Sound, device: Option<String>, volume: f32) {
        let _ = self.tx.send(Msg::Play { clip: Clip::Chime(sound), device, volume });
    }

    /// 48 kHz stereo (a soundboard sound tried out away from a call).
    pub fn play_stereo(&self, pcm: Arc<Vec<f32>>, device: Option<String>, volume: f32) {
        let _ = self.tx.send(Msg::Play { clip: Clip::Stereo(pcm), device, volume });
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
        Clip::Chime(s) => match skin.get(&s) {
            Some(pcm) => (pcm.clone(), true),
            None => (Arc::new(render(s).into_iter().flat_map(|v| [v, v]).collect()), true),
        },
        Clip::Stereo(pcm) => (pcm, false),
    }
}

/// Notes: (frequency Hz, start ms, length ms, loudness).
fn notes(s: Sound) -> &'static [(f32, f32, f32, f32)] {
    match s {
        // A soft two-note "ding".
        Sound::Message => &[(987.8, 0.0, 180.0, 0.55), (1318.5, 70.0, 260.0, 0.45)],
        // Up for joining, down for leaving.
        Sound::Join => &[(523.3, 0.0, 140.0, 0.5), (659.3, 80.0, 140.0, 0.5), (784.0, 160.0, 260.0, 0.5)],
        Sound::Leave => &[(784.0, 0.0, 140.0, 0.5), (659.3, 80.0, 140.0, 0.5), (523.3, 160.0, 260.0, 0.5)],
        // A bright rising pair for a stream going live, falling when it ends.
        Sound::StreamStart => &[(587.3, 0.0, 120.0, 0.45), (880.0, 90.0, 120.0, 0.45), (1174.7, 180.0, 340.0, 0.45)],
        Sound::StreamStop => &[(1174.7, 0.0, 120.0, 0.45), (880.0, 90.0, 120.0, 0.45), (587.3, 180.0, 300.0, 0.45)],
        // Short blips for tuning in and out of someone's stream.
        Sound::WatchStart => &[(1046.5, 0.0, 90.0, 0.4), (1568.0, 60.0, 180.0, 0.4)],
        Sound::WatchStop => &[(1568.0, 0.0, 90.0, 0.4), (1046.5, 60.0, 180.0, 0.4)],
        // A bright "ring-ring": two quick rising pairs.
        Sound::Ring => &[
            (784.0, 0.0, 120.0, 0.55),
            (1046.5, 95.0, 140.0, 0.55),
            (784.0, 260.0, 120.0, 0.55),
            (1046.5, 355.0, 240.0, 0.55),
        ],
        // Softer and lower, for waiting on the other end.
        Sound::Ringback => &[(523.3, 0.0, 260.0, 0.32), (659.3, 170.0, 420.0, 0.3)],
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
        for s in [
            Sound::Message,
            Sound::Join,
            Sound::Leave,
            Sound::StreamStart,
            Sound::StreamStop,
            Sound::WatchStart,
            Sound::WatchStop,
            Sound::Ring,
            Sound::Ringback,
        ] {
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
            assert!(ingle_proto::skin::SOUNDS.iter().any(|(n, _)| *n == s.name()), "{s:?}");
        }
        assert_eq!(ingle_proto::skin::SOUNDS.len(), Sound::ALL.len());
        let mine = Arc::new(vec![0.25f32; 9600]);
        let skin: HashMap<Sound, Arc<Vec<f32>>> = [(Sound::Message, mine.clone())].into();
        let (pcm, chime) = stereo(Clip::Chime(Sound::Message), &skin);
        assert!(Arc::ptr_eq(&pcm, &mine) && chime, "still a chime: it isn't hushed with the soundboard");
        let (pcm, _) = stereo(Clip::Chime(Sound::Join), &skin);
        assert_eq!(pcm.len(), render(Sound::Join).len() * 2, "the others stay Ingle's own");
    }
}
