//! A TV in the room (Blue, Nov 17: "I don't want to hear Mag's mother's TV coming through his
//! mic, it's spanish television in the background"): a TV across the room talking the whole
//! time, and you saying something now and then, minutes apart, as Pyr starts (echo
//! cancellation and automatic gain on, the microphone opening by itself when you talk). What
//! the others hear of the TV while you're quiet, minute by minute, and whether your own
//! talking still comes through whole.
//!
//!   cargo run -p pyr-engine --example tv_eval -- you.wav tv.wav [TV level dBFS] [mode]
//!
//! Both 48 kHz mono 16-bit; `tv.wav` is the TV as it reaches the microphone across the room
//! (speech with music under it, through a room's echo), six minutes or more. The TV is played
//! at -45 dBFS unless told (your voice is at -25: a TV 20 dB under you). `mode`: strong or
//! isolation (both, unless told). `PYR_AB` and `PYR_FIX` work as in Pyr.

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
/// When you talk (seconds from the start, 15 s each): minutes apart, the TV talking all along.
const YOU_AT: [usize; 4] = [0, 100, 200, 300];
const YOU_FOR: usize = 15;
const LENGTH: usize = 360;

/// What's sent (decoded), the microphone opening by itself as in Pyr.
fn run(settings: &Settings, mic: &[f32]) -> (Vec<f32>, Vec<(f32, f32, bool)>) {
    let mut cap = CapturePipeline::new(settings).unwrap();
    let mut dec = opus::Decoder::new(48_000, opus::Channels::Mono).unwrap();
    let ctl = Controls::default();
    let t0 = Instant::now();
    let mut out = vec![0.0f32; mic.len()];
    let mut frame = vec![0.0f32; 480];
    let mut pcm = vec![0.0f32; 960];
    let mut views = Vec::with_capacity(mic.len() / 480);
    for (k, chunk) in mic.chunks_exact(480).enumerate() {
        frame.copy_from_slice(chunk);
        let mut packets: Vec<Vec<u8>> = Vec::new();
        let now = t0 + Duration::from_millis(10 * k as u64);
        cap.process(&mut frame, &ctl, now, &mut |_, _, p| packets.push(p.to_vec()));
        views.push(cap.isolation_view(now));
        let end = (k + 1) * 480;
        let n = packets.len();
        for (i, p) in packets.iter().enumerate() {
            let at = end as i64 - 960 * (n - i) as i64;
            if let Ok(m) = dec.decode_float(p, &mut pcm, false) {
                for j in 0..m {
                    let t = at + j as i64;
                    if t >= 0 && (t as usize) < out.len() {
                        out[t as usize] = pcm[j];
                    }
                }
            }
        }
    }
    (out, views)
}

fn talking(sec: usize) -> bool {
    YOU_AT.iter().any(|&a| sec >= a && sec < a + YOU_FOR)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let you = scaled(&read_wav(&args[1]), -25.0);
    let tv_db: f32 = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(-45.0);
    let tv = scaled(&read_wav(&args[2]), tv_db);
    let n = LENGTH * SEC;
    assert!(tv.len() >= n, "the TV recording needs {LENGTH} s");
    // The microphone: the TV all along, you now and then, and the microphone's own hiss.
    let mut mic: Vec<f32> = tv[..n].to_vec();
    for (i, &at) in YOU_AT.iter().enumerate() {
        let part = &you[(i * YOU_FOR * SEC) % you.len()..][..YOU_FOR * SEC];
        for (j, v) in part.iter().enumerate() {
            mic[at * SEC + j] += v;
        }
    }
    let mut seed = 0x2545_f491u32;
    for v in mic.iter_mut() {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *v = (*v + 10f32.powf(-70.0 / 20.0) * 3f32.sqrt() * (seed as f32 / u32::MAX as f32 * 2.0 - 1.0)).clamp(-1.0, 1.0);
    }
    let modes: Vec<(&str, NoiseSuppression)> = [("strong", NoiseSuppression::Strong), ("isolation", NoiseSuppression::Isolation)]
        .into_iter()
        .filter(|(name, _)| args.get(4).is_none_or(|m| m == name))
        .collect();
    println!("TV at {tv_db:.0} dBFS all along; you at -25 dBFS for {YOU_FOR} s at {YOU_AT:?} s (PYR_AB={:?} PYR_FIX={:?})", std::env::var("PYR_AB").unwrap_or_default(), std::env::var("PYR_FIX").unwrap_or_default());
    for (name, ns) in modes {
        let s = Settings { noise_suppression: ns, ..Settings::default() };
        let (out, views) = run(&s, &mic);
        // (`TRACE=1`: how deep the dips between syllables looked, the TV alone and you.)
        if std::env::var_os("TRACE").is_some() && ns == NoiseSuppression::Isolation {
            let pick = |you: bool| {
                let mut d: Vec<f32> = views.iter().enumerate().filter(|(k, v)| v.0 > 0.0 && talking(k / 100) == you && (you || !talking((k / 100).saturating_sub(2)))).map(|(_, v)| v.0).collect();
                d.sort_unstable_by(|a, b| a.total_cmp(b));
                let q = |f: f32| d.get(((d.len().max(1) - 1) as f32 * f) as usize).copied().unwrap_or(0.0);
                format!("p10 {:.0} p50 {:.0} p90 {:.0} (dB), at 25 or more: {:.0}%", q(0.1), q(0.5), q(0.9), 100.0 * d.iter().filter(|v| **v >= 25.0).count() as f32 / d.len().max(1) as f32)
            };
            println!("  dips, the TV alone: {}", pick(false));
            println!("  dips, you talking:  {}", pick(true));
            let refs: Vec<String> = (0..LENGTH / 30).map(|i| format!("{:.0}", views[i * 3000].1)).collect();
            println!("  your usual level every 30 s: {}", refs.join(" "));
        }
        println!("== {name}");
        // The TV alone, each second while you're quiet: what went out, minute by minute. Heard:
        // louder than -60 dBFS.
        let mut line = String::new();
        let mut heard_all = 0;
        let mut alone_all = 0;
        for minute in 0..LENGTH / 60 {
            let (mut e, mut alone, mut heard) = (0.0f64, 0, 0);
            for sec in minute * 60..(minute + 1) * 60 {
                // (A second either side of your talking: your last words and the gate closing.)
                if talking(sec) || talking(sec.saturating_sub(2)) || talking(sec + 1) {
                    continue;
                }
                let o = db(&out[sec * SEC..(sec + 1) * SEC]);
                e += 10f64.powf(o as f64 / 10.0);
                alone += 1;
                if o > -60.0 {
                    heard += 1;
                }
            }
            if alone > 0 {
                let m = 10.0 * (e / alone as f64).max(1e-12).log10();
                line.push_str(&format!("  min {}: {:>4.0} dBFS, heard {:>2} of {:>2} s", minute + 1, m, heard, alone));
                heard_all += heard;
                alone_all += alone;
            }
        }
        println!("  the TV alone, going out:{line}");
        // (`TRACE=1`: each second of the first two minutes: what went out.)
        if std::env::var_os("TRACE").is_some() {
            let secs: Vec<String> = (0..120).map(|sec| format!("{:.0}", db(&out[sec * SEC..(sec + 1) * SEC]))).collect();
            println!("  each second, 0 to 120 s: {}", secs.join(" "));
        }
        println!("  the TV heard (louder than -60 dBFS): {heard_all} of {alone_all} s");
        let full = (0..LENGTH).filter(|&sec| !(talking(sec) || talking(sec.saturating_sub(2)) || talking(sec + 1)) && db(&out[sec * SEC..(sec + 1) * SEC]) > -45.0).count();
        println!("  the TV as loud as it was or louder (over -45 dBFS): {full} s");
        // You: each 200 ms in the middle of your talking, turned down 10 dB or more than it came in.
        let (mut yours, mut down) = (0, 0);
        let mut you_out = 0.0f64;
        for &at in &YOU_AT {
            let mut k = (at + 1) * SEC;
            while k + SEC / 5 <= (at + YOU_FOR) * SEC {
                let w = k..k + SEC / 5;
                let input = db(&mic[w.clone()]);
                if input > -40.0 {
                    let o = db(&out[w]);
                    yours += 1;
                    you_out += 10f64.powf(o as f64 / 10.0);
                    if o - input < -10.0 {
                        down += 1;
                    }
                }
                k += SEC / 5;
            }
        }
        println!("  your talking: {:.0} dBFS going out, turned down 10 dB or more: {down} of {yours}", 10.0 * (you_out / yours.max(1) as f64).max(1e-12).log10());
    }
}
