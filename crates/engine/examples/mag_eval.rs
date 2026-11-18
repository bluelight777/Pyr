//! Mag's room (Blue, Nov 18, on 0.15.0: "Mag doesn't sound very clear, and I can hear myself
//! echo through his mic"): voice isolation on, a TV across the room talking the whole time,
//! you saying something now and then, and the others talking out of your speakers in between
//! (and once while you talk). What the others hear: of their own voices coming back, of the
//! TV, and whether your talking comes through whole.
//!
//!   cargo run --release -p pyr-engine --example mag_eval -- you.wav other.wav tv.wav [TV level dBFS or "none"] [speakers' loss dB]
//!
//! All 48 kHz mono 16-bit; `tv.wav` as `tv_eval`'s (`tools/make-tv-room.py`). You're at -25
//! dBFS at the microphone; the TV at -35 unless told (10 dB under you); the others play at -20
//! dBFS and reach the microphone `loss` dB quieter (10 unless told: speakers by the
//! microphone; 25: a quiet room's), 60 ms later, with a few reflections. `PYR_AB` and
//! `PYR_FIX` work as in Pyr; `TRACE=1` prints your usual level as isolation has it.

use std::time::{Duration, Instant};

use pyr_engine::audio::Controls;
use pyr_engine::audio::capture::CapturePipeline;
use pyr_engine::config::{NoiseSuppression, Settings};

fn read_wav(p: &str) -> Vec<f32> {
    let b = std::fs::read(p).unwrap_or_else(|e| panic!("{p}: {e}"));
    let mut i = 12;
    while i + 8 <= b.len() {
        let n = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        if &b[i..i + 4] == b"data" {
            return b[i + 8..(i + 8 + n).min(b.len())].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect();
        }
        i += 8 + n + (n & 1);
    }
    panic!("{p}: no data");
}

fn db(x: &[f32]) -> f32 {
    let s = x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32;
    if s > 1e-12 { 10.0 * s.log10() } else { -120.0 }
}

fn scaled(x: &[f32], to_db: f32) -> Vec<f32> {
    let g = 10f32.powf((to_db - db(x)) / 20.0);
    x.iter().map(|v| v * g).collect()
}

const SEC: usize = 48_000;
const LENGTH: usize = 360;
/// When you talk (15 s each), and when the others do (the last time over you).
const YOU_AT: [usize; 4] = [0, 100, 200, 300];
const YOU_FOR: usize = 15;
const THEY_AT: [(usize, usize); 4] = [(25, 85), (125, 185), (225, 285), (296, 330)];

fn you_talk(sec: usize) -> bool {
    YOU_AT.iter().any(|&a| sec >= a && sec < a + YOU_FOR)
}

fn they_talk(sec: usize) -> bool {
    THEY_AT.iter().any(|&(a, b)| sec >= a && sec < b)
}

/// (A second either side of your talking: your last words and the gate closing.)
fn near_you(sec: usize) -> bool {
    you_talk(sec) || you_talk(sec.saturating_sub(2)) || you_talk(sec + 1)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let you = scaled(&read_wav(&args[1]), -25.0);
    let other = scaled(&read_wav(&args[2]), -20.0);
    let tv_db: Option<f32> = match args.get(4).map(|s| s.as_str()) {
        Some("none") => None,
        v => Some(v.and_then(|v| v.parse().ok()).unwrap_or(-35.0)),
    };
    let loss: f32 = args.get(5).and_then(|v| v.parse().ok()).unwrap_or(10.0);
    let n = LENGTH * SEC;
    // What the speakers play: the others, when they talk.
    let mut far = vec![0.0f32; n];
    let mut at_other = 0;
    for &(a, b) in &THEY_AT {
        for i in a * SEC..b * SEC {
            far[i] = other[at_other % other.len()];
            at_other += 1;
        }
    }
    // The microphone: the TV all along, you now and then, the speakers through the room, hiss.
    let mut mic = vec![0.0f32; n];
    if let Some(level) = tv_db {
        let tv = scaled(&read_wav(&args[3]), level);
        assert!(tv.len() >= n, "the TV recording needs {LENGTH} s");
        mic.copy_from_slice(&tv[..n]);
    }
    for (i, &at) in YOU_AT.iter().enumerate() {
        let part = &you[(i * YOU_FOR * SEC) % you.len()..][..YOU_FOR * SEC];
        for (j, v) in part.iter().enumerate() {
            mic[at * SEC + j] += v;
        }
    }
    let mut echo = vec![0.0f32; n];
    let g = 10f32.powf(-loss / 20.0);
    for (extra, amp) in [(0usize, 1.0f32), (7 * 48, 0.45), (19 * 48, 0.25), (41 * 48, 0.12)] {
        let d = 60 * 48 + extra;
        for i in d..n {
            echo[i] += far[i - d] * g * amp;
        }
    }
    let mut seed = 0x2545_f491u32;
    for (v, e) in mic.iter_mut().zip(&echo) {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *v = (*v + e + 10f32.powf(-70.0 / 20.0) * 3f32.sqrt() * (seed as f32 / u32::MAX as f32 * 2.0 - 1.0)).clamp(-1.0, 1.0);
    }
    println!(
        "TV {}; you at -25 dBFS for {YOU_FOR} s at {YOU_AT:?} s; the others out of the speakers at {THEY_AT:?} s, reaching the microphone at {:.0} dBFS (PYR_AB={:?} PYR_FIX={:?})",
        tv_db.map(|d| format!("at {d:.0} dBFS all along")).unwrap_or("off".into()),
        db(&echo[30 * SEC..80 * SEC]),
        std::env::var("PYR_AB").unwrap_or_default(),
        std::env::var("PYR_FIX").unwrap_or_default()
    );
    let s = Settings { noise_suppression: NoiseSuppression::Isolation, ..Settings::default() };
    let mut cap = CapturePipeline::new(&s).unwrap();
    cap.stream_delay_ms = 60;
    let mut dec = opus::Decoder::new(48_000, opus::Channels::Mono).unwrap();
    let ctl = Controls::default();
    let t0 = Instant::now();
    let mut out = vec![0.0f32; n];
    let mut frame = vec![0.0f32; 480];
    let mut stereo = vec![0.0f32; 960];
    let mut pcm = vec![0.0f32; 960];
    let mut views = Vec::with_capacity(n / 480);
    for k in 0..n / 480 {
        for i in 0..480 {
            let v = far[k * 480 + i];
            stereo[2 * i] = v;
            stereo[2 * i + 1] = v;
        }
        cap.render(&stereo);
        frame.copy_from_slice(&mic[k * 480..(k + 1) * 480]);
        let mut packets: Vec<Vec<u8>> = Vec::new();
        let now = t0 + Duration::from_millis(10 * k as u64);
        cap.process(&mut frame, &ctl, now, &mut |_, _, p| packets.push(p.to_vec()));
        views.push(cap.isolation_view(now));
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
    if std::env::var_os("TRACE").is_some() {
        let refs: Vec<String> = (0..LENGTH / 15).map(|i| format!("{:.0}", views[i * 1500].1)).collect();
        println!("  your usual level every 15 s: {}", refs.join(" "));
        let secs: Vec<String> = (0..LENGTH).map(|sec| format!("{:.0}", db(&out[sec * SEC..(sec + 1) * SEC]))).collect();
        println!("  each second going out: {}", secs.join(" "));
    }
    // Each second while you're quiet: what went out, the others talking or not.
    for (what, they) in [("the others' own voices coming back (they talk, you're quiet)", true), ("nobody talking but the TV", false)] {
        let (mut e, mut all, mut heard, mut loud) = (0.0f64, 0, 0, 0);
        for sec in 0..LENGTH {
            if near_you(sec) || they_talk(sec) != they || (!they && they_talk(sec.saturating_sub(1))) {
                continue;
            }
            let o = db(&out[sec * SEC..(sec + 1) * SEC]);
            e += 10f64.powf(o as f64 / 10.0);
            all += 1;
            heard += (o > -60.0) as usize;
            loud += (o > -45.0) as usize;
        }
        println!("  {what}: {:.0} dBFS going out; heard (over -60 dBFS) {heard} of {all} s, loud (over -45) {loud} s", 10.0 * (e / all.max(1) as f64).max(1e-12).log10());
    }
    // You: each 200 ms of your talking, turned down 10 dB or more than it came in; alone, and
    // while the others talk too.
    for (what, with_them) in [("your talking, the others quiet", false), ("your talking over the others", true)] {
        let (mut yours, mut down, mut gone) = (0, 0, 0);
        let mut you_out = 0.0f64;
        for &at in &YOU_AT {
            let mut k = (at + 1) * SEC;
            while k + SEC / 5 <= (at + YOU_FOR) * SEC {
                let w = k..k + SEC / 5;
                k += SEC / 5;
                if they_talk(w.start / SEC) != with_them {
                    continue;
                }
                // (What you said alone: the TV and the echo aren't your talking.)
                let said = {
                    let i = YOU_AT.iter().position(|&a| a == at).unwrap();
                    let from = (i * YOU_FOR * SEC) % you.len() + (w.start - at * SEC);
                    db(&you[from..from + SEC / 5])
                };
                if said > -40.0 {
                    let o = db(&out[w]);
                    yours += 1;
                    you_out += 10f64.powf(o as f64 / 10.0);
                    down += (o - said < -10.0) as usize;
                    gone += (o - said < -20.0) as usize;
                }
            }
        }
        println!("  {what}: {:.0} dBFS going out; turned down 10 dB or more: {down} of {yours} (20 dB or more: {gone})", 10.0 * (you_out / yours.max(1) as f64).max(1e-12).log10());
    }
}
