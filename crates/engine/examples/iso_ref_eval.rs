//! Voice isolation over a long stretch of talking: whether your own voice keeps coming through
//! whole (not turned down, in and out) when something louder than you happens now and then,
//! when you get quieter, and whether a quieter voice across the room stays out.
//!
//!   cargo run --release -p pyr-engine --example iso_ref_eval -- you.wav other.wav
//!
//! Both 48 kHz mono 16-bit. Run it with PYR_AB=isoref for how it was before 0.9.1.
//! For each scene: how much of your talking (each 200 ms in the middle of a sentence) came
//! out turned down 10 dB or more, and how loud the other voice alone came out.

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

/// What's sent (decoded), with the microphone open the whole time.
fn run(input: &[f32]) -> Vec<f32> {
    let s = Settings { noise_suppression: NoiseSuppression::Isolation, auto_gain: false, echo_cancellation: false, ..Settings::default() };
    let mut cap = CapturePipeline::new(&s).unwrap();
    let mut dec = opus::Decoder::new(48_000, opus::Channels::Mono).unwrap();
    let ctl = Controls::default();
    ctl.ptt_down.store(true, std::sync::atomic::Ordering::Relaxed);
    let t0 = Instant::now();
    let mut out = vec![0.0f32; input.len()];
    let mut frame = vec![0.0f32; 480];
    let mut pcm = vec![0.0f32; 960];
    for (k, chunk) in input.chunks_exact(480).enumerate() {
        frame.copy_from_slice(chunk);
        let mut packets: Vec<Vec<u8>> = Vec::new();
        cap.process(&mut frame, &ctl, t0 + Duration::from_millis(10 * k as u64), &mut |_, _, p| packets.push(p.to_vec()));
        if let Some(w) = ctl.swelled.lock().take() {
            println!("    noticed at {:.1} s: {w}", k as f32 / 100.0);
        }
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
    out
}

/// A scene: the microphone's sound, and for each sample whether it's you (1), the other (2),
/// both (3) or neither (0).
struct Scene {
    name: &'static str,
    mix: Vec<f32>,
    who: Vec<u8>,
}

fn place(mix: &mut Vec<f32>, who: &mut Vec<u8>, at: usize, x: &[f32], tag: u8) {
    if mix.len() < at + x.len() {
        mix.resize(at + x.len(), 0.0);
        who.resize(at + x.len(), 0);
    }
    for (i, v) in x.iter().enumerate() {
        mix[at + i] += v;
        if v.abs() > 1e-4 {
            who[at + i] |= tag;
        }
    }
}

const W: usize = 9600;

fn score(sc: &Scene, out: &[f32]) -> String {
    // Your talking: 200 ms windows that are you alone, and so is the one before (mid-sentence).
    let mut yours = 0;
    let mut down = 0;
    let mut other = Vec::new();
    let mut k = W;
    let mut prev_you = false;
    while k + W <= sc.mix.len() {
        let w = &sc.who[k..k + W];
        let you = w.iter().filter(|&&t| t == 1).count() > W * 9 / 10 && db(&sc.mix[k..k + W]) > -45.0;
        if you && prev_you {
            yours += 1;
            if db(&out[k..k + W]) - db(&sc.mix[k..k + W]) < -10.0 {
                down += 1;
            }
        }
        if w.iter().filter(|&&t| t == 2).count() > W * 9 / 10 {
            other.push(db(&out[k..k + W]) - db(&sc.mix[k..k + W]));
        }
        prev_you = you;
        k += W;
    }
    let o = if other.is_empty() { String::new() } else { format!("  the other voice alone: {:+.0} dB", other.iter().sum::<f32>() / other.len() as f32) };
    format!("{:<44} your talking turned down: {down:>3} of {yours:>3}{o}", sc.name)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let you = read_wav(&args[1]);
    let other = read_wav(&args[2]);
    let sec = 48_000;
    let mut scenes = Vec::new();

    // Loud moments: you talk at -30 dBFS for a minute; every 12 s, 2 s of you laughing or
    // shouting 12 dB louder.
    {
        let (mut mix, mut who) = (Vec::new(), Vec::new());
        let normal = scaled(&you, -30.0);
        let mut at = 0;
        for i in 0..4 {
            place(&mut mix, &mut who, at, &normal, 1);
            at += normal.len();
            if i < 3 {
                place(&mut mix, &mut who, at, &scaled(&you[..2 * sec], -18.0), 4);
                at += 2 * sec + sec / 2;
            }
        }
        scenes.push(Scene { name: "you, with a loud moment every 15 s", mix, who });
    }
    // Something loud and voice-like now and then (a game's voices from a speaker, a loud
    // teammate in the room): 2 s at -14 dBFS while you pause, every 10 s.
    {
        let (mut mix, mut who) = (Vec::new(), Vec::new());
        let normal = scaled(&you, -30.0);
        let loud = scaled(&other[..2 * sec], -14.0);
        let mut at = 0;
        for _ in 0..4 {
            place(&mut mix, &mut who, at, &normal, 1);
            at += normal.len() + sec / 4;
            place(&mut mix, &mut who, at, &loud, 4);
            at += loud.len() + sec / 4;
        }
        scenes.push(Scene { name: "you, and a loud voice from a speaker", mix, who });
    }
    // You get quieter (you lean back, or Windows turns the microphone down): 10 s at -20 dBFS,
    // then 50 s at -32.
    {
        let (mut mix, mut who) = (Vec::new(), Vec::new());
        place(&mut mix, &mut who, 0, &scaled(&you[..10 * sec], -20.0), 4);
        let quiet = scaled(&you, -32.0);
        let mut at = 10 * sec + sec / 2;
        for _ in 0..3 {
            place(&mut mix, &mut who, at, &quiet, 1);
            at += quiet.len();
        }
        scenes.push(Scene { name: "you, then 12 dB quieter", mix, who });
    }
    // Someone across the room: you at -25 dBFS, then them alone at -42, then both.
    {
        let (mut mix, mut who) = (Vec::new(), Vec::new());
        let me = scaled(&you, -25.0);
        let them = scaled(&other, -42.0);
        place(&mut mix, &mut who, 0, &me, 1);
        let at = me.len();
        place(&mut mix, &mut who, at, &them, 2);
        let at = at + them.len();
        place(&mut mix, &mut who, at, &me, 1);
        place(&mut mix, &mut who, at, &them, 2);
        scenes.push(Scene { name: "you, then someone across the room", mix, who });
    }

    let mut seed = 11u32;
    for sc in scenes.iter_mut() {
        for v in sc.mix.iter_mut() {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            *v += ((seed >> 8) as f32 / (1 << 24) as f32 - 0.5) * 0.002;
        }
        let out = run(&sc.mix);
        println!("{}", score(sc, &out));
    }
}
