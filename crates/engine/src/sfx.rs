//! Notification sounds: new messages, people joining and leaving voice, screen shares
//! starting and stopping, and tuning in to one. The sounds are made here (short chimes), not
//! loaded from files, and play on the chosen speakers on their own little stream, so they
//! work in and out of calls.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::audio::device::OutputDevice;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Message,
    Join,
    Leave,
    StreamStart,
    StreamStop,
    WatchStart,
    WatchStop,
}

struct Play {
    sound: Sound,
    device: Option<String>,
    volume: f32,
}

pub struct Sfx {
    tx: mpsc::Sender<Play>,
}

const RATE: f32 = 48_000.0;

impl Sfx {
    pub fn start() -> Sfx {
        let (tx, rx) = mpsc::channel::<Play>();
        let _ = std::thread::Builder::new().name("sfx".into()).spawn(move || run(rx));
        Sfx { tx }
    }

    pub fn play(&self, sound: Sound, device: Option<String>, volume: f32) {
        let _ = self.tx.send(Play { sound, device, volume });
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

fn run(rx: mpsc::Receiver<Play>) {
    let mut out: Option<(OutputDevice, Option<String>)> = None;
    // Sounds still playing: (samples, position, volume).
    let mut playing: Vec<(Vec<f32>, usize, f32)> = Vec::new();
    let mut idle_since = Instant::now();
    let mut chunk = vec![0.0f32; 960];
    loop {
        let wait = if playing.is_empty() { Duration::from_millis(500) } else { Duration::from_millis(0) };
        match rx.recv_timeout(wait) {
            Ok(p) => {
                let same = out.as_ref().map(|(_, d)| *d == p.device).unwrap_or(false);
                if !same {
                    out = OutputDevice::open(p.device.as_deref()).ok().map(|d| (d, p.device.clone()));
                }
                // The same sound twice at once just sounds louder; skip the copy.
                if playing.len() < 6 {
                    playing.push((render(p.sound), 0, p.volume.clamp(0.0, 2.0)));
                }
                while let Ok(p) = rx.try_recv() {
                    if playing.len() < 6 {
                        playing.push((render(p.sound), 0, p.volume.clamp(0.0, 2.0)));
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        if playing.is_empty() {
            // Let go of the speakers when nothing has played for a while.
            if out.is_some() && idle_since.elapsed() > Duration::from_secs(5) {
                out = None;
            }
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
            for (samples, pos, vol) in playing.iter_mut() {
                for i in 0..480 {
                    let v = samples.get(*pos + i).copied().unwrap_or(0.0) * *vol;
                    chunk[2 * i] += v;
                    chunk[2 * i + 1] += v;
                }
                *pos += 480;
            }
            for c in chunk.iter_mut() {
                *c = c.clamp(-1.0, 1.0);
            }
            dev.write(&chunk);
            playing.retain(|(s, pos, _)| *pos < s.len());
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
        for s in [Sound::Message, Sound::Join, Sound::Leave, Sound::StreamStart, Sound::StreamStop, Sound::WatchStart, Sound::WatchStop] {
            let v = render(s);
            let peak = v.iter().fold(0.0f32, |a, x| a.max(x.abs()));
            assert!(v.len() > 4800 && v.len() < 48_000, "{s:?} {}", v.len());
            assert!(peak > 0.05 && peak < 0.6, "{s:?} peak {peak}");
        }
    }
}
