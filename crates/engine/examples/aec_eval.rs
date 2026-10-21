//! Your voice while others talk or music plays (echo cancellation and automatic gain on, as
//! Pyr starts): whether it keeps coming through whole, or gets turned down in and out (bug
//! reports 27 and 32 to 41: "my voice went up and down for the others").
//!
//!   cargo run --release -p pyr-engine --example aec_eval -- you.wav other.wav music.wav
//!
//! All 48 kHz mono 16-bit. For each scene: how much of your talking (each 200 ms in the middle
//! of a sentence) came out turned down 10 dB or more than your usual, and how loud what the
//! speakers played came back out (an echo that got through).

use std::time::{Duration, Instant};

use pyr_engine::audio::Controls;
use pyr_engine::audio::capture::CapturePipeline;
use pyr_engine::config::{NoiseSuppression, Settings};

fn read_wav(p: &str) -> Vec<f32> {
    let b = std::fs::read(p).unwrap();
    let mut i = 12;
    while i + 8 <= b.len() {
        let n = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        if &b[i..i + 4] == b"data" {
            return b[i + 8..(i + 8 + n).min(b.len())].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect();
        }
        i += 8 + n + (n & 1);
    }
    panic!("no data");
}

fn db(x: &[f32]) -> f32 {
    let s = x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32;
    if s > 1e-12 { 10.0 * s.log10() } else { -120.0 }
}

fn scaled(x: &[f32], to_db: f32) -> Vec<f32> {
    let g = 10f32.powf((to_db - db(x)) / 20.0);
    x.iter().map(|v| v * g).collect()
}

/// A room: the speakers' sound comes back into the microphone `delay_ms` later, `loss_db`
/// quieter, smeared by a few reflections.
#[derive(Clone, Copy)]
struct Room {
    delay_ms: usize,
    loss_db: f32,
}

/// What's sent (decoded), with the microphone open the whole time: `you` into the microphone,
/// `far` out of the speakers (and, through `room`, back into the microphone).
fn run(settings: &Settings, you: &[f32], far: &[f32], room: Option<Room>, mic_gain: f32) -> (Vec<f32>, Vec<String>) {
    let mut cap = CapturePipeline::new(settings).unwrap();
    cap.stream_delay_ms = 60;
    let mut dec = opus::Decoder::new(48_000, opus::Channels::Mono).unwrap();
    let ctl = Controls::default();
    ctl.ptt_down.store(true, std::sync::atomic::Ordering::Relaxed);
    let t0 = Instant::now();
    let n = you.len().max(far.len());
    // The echo: the far end through the room (direct path and three reflections).
    let mut mic = vec![0.0f32; n];
    for i in 0..n {
        mic[i] = you.get(i).copied().unwrap_or(0.0) * mic_gain;
    }
    if let Some(r) = room {
        let g = 10f32.powf(-r.loss_db / 20.0);
        let d = r.delay_ms * 48;
        for (extra, amp) in [(0usize, 1.0f32), (7 * 48, 0.45), (19 * 48, 0.25), (41 * 48, 0.12)] {
            for i in 0..n {
                if let Some(v) = far.get(i) {
                    if i + d + extra < n {
                        mic[i + d + extra] += v * g * amp;
                    }
                }
            }
        }
    }
    let mut out = vec![0.0f32; n];
    let mut said = Vec::new();
    let mut frame = vec![0.0f32; 480];
    let mut stereo = vec![0.0f32; 960];
    let mut pcm = vec![0.0f32; 960];
    for k in 0..n / 480 {
        // The speakers first (as Pyr's audio thread does), then the microphone.
        for i in 0..480 {
            let v = far.get(k * 480 + i).copied().unwrap_or(0.0);
            stereo[2 * i] = v;
            stereo[2 * i + 1] = v;
        }
        cap.render(&stereo);
        frame.copy_from_slice(&mic[k * 480..k * 480 + 480]);
        let mut packets: Vec<Vec<u8>> = Vec::new();
        cap.process(&mut frame, &ctl, t0 + Duration::from_millis(10 * k as u64), &mut |_, _, p| packets.push(p.to_vec()));
        if let Some(w) = ctl.swelled.lock().take() {
            said.push(format!("at {:.1} s: {w}", k as f32 / 100.0));
        }
        if std::env::var("STATS").is_ok() && k % 1000 == 999 {
            println!("      {:>4.0} s: {}", k as f32 / 100.0, cap.echo_stats());
        }
        let end = (k + 1) * 480;
        let m = packets.len();
        for (j, p) in packets.iter().enumerate() {
            let at = end as i64 - 960 * (m - j) as i64;
            if let Ok(got) = dec.decode_float(p, &mut pcm, false) {
                for q in 0..got {
                    let t = at + q as i64;
                    if t >= 0 && (t as usize) < out.len() {
                        out[t as usize] = pcm[q];
                    }
                }
            }
        }
    }
    (out, said)
}

const W: usize = 9600;

/// (your talking turned down 10 dB or more, of how much; the echo that got through, dB).
fn score(you: &[f32], far: &[f32], out: &[f32]) -> (usize, usize, Option<f32>) {
    // Your talking: 200 ms windows that are you (and the one before too: mid-sentence).
    let mut gains = Vec::new();
    let mut k = W;
    let mut prev = false;
    while k + W <= you.len().min(out.len()) {
        let talking = db(&you[k..k + W]) > -45.0;
        if talking && prev {
            gains.push(db(&out[k..k + W]) - db(&you[k..k + W]));
        }
        prev = talking;
        k += W;
    }
    let mut sorted = gains.clone();
    sorted.sort_by(f32::total_cmp);
    let usual = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    let down = gains.iter().filter(|g| **g < usual - 10.0).count();
    if std::env::var("DIPS").is_ok() {
        // When (seconds) and how deep.
        let mut k = W;
        let mut prev = false;
        let mut list = Vec::new();
        while k + W <= you.len().min(out.len()) {
            let talking = db(&you[k..k + W]) > -45.0;
            if talking && prev {
                let g = db(&out[k..k + W]) - db(&you[k..k + W]);
                if g < usual - 10.0 {
                    list.push(format!("{:.1}s:{:.0}", k as f32 / 48_000.0, g - usual));
                }
            }
            prev = talking;
            k += W;
        }
        println!("      dips: {}", list.join(" "));
    }
    // The echo alone: windows with the far end playing and you silent.
    let mut echo = Vec::new();
    let mut k = 0;
    while k + W <= far.len().min(out.len()) {
        let you_quiet = you.get(k..k + W).map(|w| db(w) < -60.0).unwrap_or(true);
        if you_quiet && db(&far[k..k + W]) > -40.0 {
            echo.push(db(&out[k..k + W]) - db(&far[k..k + W]));
        }
        k += W;
    }
    let echo = (!echo.is_empty()).then(|| echo.iter().sum::<f32>() / echo.len() as f32);
    (down, gains.len(), echo)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let you_raw = read_wav(&args[1]);
    let other = read_wav(&args[2]);
    let music = read_wav(&args[3]);
    let sec = 48_000;
    // You talk on and off for a minute and a half (sentences, then a pause), at -26 dBFS.
    let you_talk = {
        let me = scaled(&you_raw, -26.0);
        let mut v = Vec::new();
        let mut at = 0;
        while v.len() < 90 * sec {
            let piece = &me[at..(at + 6 * sec).min(me.len())];
            v.extend_from_slice(piece);
            v.extend(std::iter::repeat_n(0.0, sec * 3 / 2));
            at = (at + 6 * sec) % (me.len() - 6 * sec);
        }
        v
    };
    let n = you_talk.len();
    // What the speakers play: someone else talking now and then, or music all along.
    let voices = {
        let o = scaled(&other, -20.0);
        let mut v = vec![0.0f32; n];
        let mut at = 2 * sec;
        let mut from = 0;
        while at + 4 * sec < n {
            for i in 0..4 * sec {
                v[at + i] = o[(from + i) % o.len()];
            }
            from += 4 * sec;
            at += 9 * sec;
        }
        v
    };
    let tunes = |level: f32| {
        let m = scaled(&music, level);
        (0..n).map(|i| m[i % m.len()]).collect::<Vec<f32>>()
    };
    let loud = {
        // Music, loud enough to clip now and then (a stream turned up), with voices over it.
        let mut v = tunes(-9.0);
        for (a, b) in v.iter_mut().zip(&voices) {
            *a = (*a + b * 2.0).clamp(-1.0, 1.0);
        }
        v
    };
    let setups: Vec<(&str, Settings)> = vec![
        ("Pyr's own settings (strong noise suppression)", Settings::default()),
        ("standard noise suppression", Settings { noise_suppression: NoiseSuppression::Standard, ..Settings::default() }),
        ("voice isolation", Settings { noise_suppression: NoiseSuppression::Isolation, ..Settings::default() }),
    ];
    let headphones = None;
    let speakers = Some(Room { delay_ms: 25, loss_db: 18.0 });
    let quiet_room = Some(Room { delay_ms: 25, loss_db: 34.0 });
    let music = tunes(-18.0);
    let scenes: Vec<(&str, &Vec<f32>, Option<Room>)> = vec![
        ("headphones, others talking", &voices, headphones),
        ("headphones, music playing", &music, headphones),
        ("headphones, loud music and voices", &loud, headphones),
        ("speakers, others talking", &voices, speakers),
        ("speakers, music playing", &music, speakers),
        ("speakers turned low, music playing", &music, quiet_room),
    ];
    let only = std::env::var("ONLY").ok();
    let setups: Vec<(&str, Settings)> = setups
        .into_iter()
        .map(|(n, mut s)| {
            if std::env::var("AEC").is_ok_and(|v| v == "0") {
                s.echo_cancellation = false;
            }
            if std::env::var("AGC").is_ok_and(|v| v == "0") {
                s.auto_gain = false;
            }
            (n, s)
        })
        .filter(|(n, _)| std::env::var("SETUP").map(|w| n.contains(&w)).unwrap_or(true))
        .collect();
    for (sname, s) in &setups {
        println!("== {sname}");
        for (name, far, room) in &scenes {
            if only.as_deref().is_some_and(|o| !name.contains(o)) {
                continue;
            }
            let (out, said) = run(s, &you_talk, far, *room, 1.0);
            let (down, of, echo) = score(&you_talk, far, &out);
            let echo = echo.map(|e| format!("  echo through: {e:+.0} dB")).unwrap_or_default();
            println!("  {name:<40} your talking turned down: {down:>3} of {of:>3}{echo}");
            for w in said.iter().take(2) {
                println!("      noticed {w}");
            }
        }
    }
}
