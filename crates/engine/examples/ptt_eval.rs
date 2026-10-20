//! Whether a voice swells and sinks while push-to-talk is held: the microphone pipeline on a
//! speech clip at a few levels, with the gain it applies (what's sent over what came in)
//! measured over every 200 ms of speech.
//!
//!   cargo run --release -p pyr-engine --example ptt_eval -- speech.wav [out dir]

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use pyr_engine::audio::Controls;
use pyr_engine::audio::capture::CapturePipeline;
use pyr_engine::config::{InputMode, NoiseSuppression, Settings};

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

/// What's sent (decoded, in place), with push-to-talk held the whole time (or voice activity).
fn run(input: &[f32], s: &Settings, ptt: bool) -> Vec<f32> {
    let mut cap = CapturePipeline::new(s).unwrap();
    let mut dec = opus::Decoder::new(48_000, opus::Channels::Mono).unwrap();
    let ctl = Controls::default();
    ctl.ptt_down.store(ptt, Ordering::Relaxed);
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

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let speech = read_wav(&args[1]);
    let out_dir = args.get(2);
    // Twice through (the second time is what counts: the gain control has settled), with a
    // room's quiet hiss under it.
    let mut seed = 7u32;
    let mut noise = || {
        seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
        (seed >> 8) as f32 / (1 << 24) as f32 - 0.5
    };
    if std::env::var_os("LOUD_MOMENT").is_some() {
        loud_moment(&speech);
        return;
    }
    let hiss: f32 = std::env::var("HISS").ok().and_then(|v| v.parse().ok()).unwrap_or(0.002);
    for level in [-42.0f32, -30.0, -18.0] {
        let g = 10f32.powf((level - db(&speech)) / 20.0);
        let mut input: Vec<f32> = speech.iter().chain(speech.iter()).map(|v| v * g).collect();
        for v in input.iter_mut() {
            *v += noise() * hiss;
        }
        for (name, ns, agc, ptt) in [
            ("Strong, auto gain, PTT", NoiseSuppression::Strong, true, true),
            ("Strong, no auto gain, PTT", NoiseSuppression::Strong, false, true),
            ("Standard, auto gain, PTT", NoiseSuppression::Standard, true, true),
            ("Isolation, auto gain, PTT", NoiseSuppression::Isolation, true, true),
            ("Strong, auto gain, voice activity", NoiseSuppression::Strong, true, false),
        ] {
            let s = Settings {
                noise_suppression: ns,
                auto_gain: agc,
                echo_cancellation: false,
                input_mode: if ptt { InputMode::PushToTalk } else { InputMode::VoiceActivity },
                ptt_release_ms: 200,
                ..Settings::default()
            };
            let out = run(&input, &s, ptt);
            // The gain on each 200 ms of speech in the second pass.
            let w = 9600;
            let half = speech.len();
            let mut gains = Vec::new();
            let mut k = half;
            while k + w <= input.len() {
                let (a, b) = (db(&input[k..k + w]), db(&out[k..k + w]));
                if a > level - 8.0 {
                    gains.push(b - a);
                }
                k += w;
            }
            gains.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let p = |q: f32| gains[((gains.len() - 1) as f32 * q) as usize];
            // Swings: consecutive speech windows whose gain differs by 6 dB or more.
            let mut ordered = Vec::new();
            let mut k = half;
            while k + w <= input.len() {
                let (a, b) = (db(&input[k..k + w]), db(&out[k..k + w]));
                ordered.push((a > level - 8.0).then_some(b - a));
                k += w;
            }
            // Between words and sentences (nothing said): how loud what's sent is, at its
            // quietest and loudest (a pumping background shows as a wide spread).
            let mut pause = Vec::new();
            let mut k = half;
            while k + 4800 <= input.len() {
                if db(&input[k..k + 4800]) < level - 25.0 {
                    pause.push(db(&out[k..k + 4800]));
                }
                k += 4800;
            }
            pause.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let pq = |q: f32| if pause.is_empty() { f32::NAN } else { pause[((pause.len() - 1) as f32 * q) as usize] };
            let swings = ordered.windows(2).filter(|x| matches!((x[0], x[1]), (Some(a), Some(b)) if (a - b).abs() >= 6.0)).count();
            println!(
                "speech {level:>4} dBFS  {name:<34} gain p10 {:+5.1}  median {:+5.1}  p90 {:+5.1} dB  spread {:4.1}  swings>=6dB {swings}  pauses {:5.1}..{:5.1} dBFS",
                p(0.1),
                p(0.5),
                p(0.9),
                p(0.9) - p(0.1),
                pq(0.1),
                pq(0.9)
            );
            if let Some(d) = out_dir {
                let path = format!("{d}/{}_{}.raw", -level as i32, name.replace([' ', ','], ""));
                let bytes: Vec<u8> = out.iter().flat_map(|v| ((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
                std::fs::write(path, bytes).unwrap();
            }
        }
    }
}

/// Voice isolation after a loud moment (a laugh, a shout: 12 dB louder for a few seconds):
/// whether the normal voice after it still comes through whole. Prints the gain per second.
fn loud_moment(speech: &[f32]) {
    let g = 10f32.powf((-30.0 - db(speech)) / 20.0);
    let n = speech.len();
    let mut input: Vec<f32> = Vec::new();
    input.extend(speech.iter().map(|v| v * g));
    input.extend(speech[..n.min(48_000 * 3)].iter().map(|v| v * g * 4.0));
    let after = input.len();
    input.extend(speech.iter().map(|v| v * g));
    input.extend(speech.iter().map(|v| v * g));
    for (name, ns) in [("Strong", NoiseSuppression::Strong), ("Isolation", NoiseSuppression::Isolation)] {
        let s = Settings { noise_suppression: ns, auto_gain: false, echo_cancellation: false, input_mode: InputMode::PushToTalk, ..Settings::default() };
        let out = run(&input, &s, true);
        let mut line = String::new();
        let mut quiet = 0;
        let mut total = 0;
        let mut k = after;
        while k + 9600 <= input.len() {
            let (a, b) = (db(&input[k..k + 9600]), db(&out[k..k + 9600]));
            if a > -38.0 {
                total += 1;
                if b - a < -10.0 {
                    quiet += 1;
                }
                line += &format!("{:+.0} ", b - a);
            } else {
                line += ". ";
            }
            k += 9600;
        }
        println!("{name:<10} after a loud moment: {quiet}/{total} stretches of speech turned down 10+ dB\n  {line}");
    }
}
