//! Measures how well a screen share's sound has Pyr's own sound (the call) taken out of it
//! (`audio::stream_echo`), and that the game or music sound is left alone.
//!
//!   cargo run --release -p pyr-engine --example stream_echo_eval -- <dir> [out dir]
//!
//! `<dir>` holds `voice.f32` (the call: 48 kHz mono, raw little-endian f32) and `game.f32`
//! (the shared sound: 48 kHz stereo interleaved). Each scene builds what a screen share would
//! capture on a PC whose sound goes through another program (so the call leaks into it), runs
//! the canceller frame by frame as the app does, and reports:
//! * echo left: the call still in the stream, against how much leaked in (dB, lower is better);
//! * game changed: how far the output is from the game sound alone (dB below the game sound;
//!   lower is better), while people talk and while they don't.

use std::path::{Path, PathBuf};

use pyr_engine::audio::stream_echo::{FRAME, StreamEcho};

fn read_f32(p: &Path) -> Vec<f32> {
    let b = std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn write_wav(p: &Path, x: &[f32]) {
    let mut b = Vec::with_capacity(44 + x.len() * 2);
    let data = (x.len() * 2) as u32;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&48_000u32.to_le_bytes());
    b.extend_from_slice(&(48_000u32 * 4).to_le_bytes());
    b.extend_from_slice(&4u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    for v in x {
        b.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(p, b).unwrap();
}

/// How the call gets into the capture: (delay in ms, what the other program does to it).
struct Leak {
    name: &'static str,
    delay_ms: usize,
    gain: f32,
    shape: fn(f32) -> f32,
    /// One-pole low-pass coefficient (0 = none), like an equalizer taking the top off.
    lowpass: f32,
}

fn db(x: f64) -> f64 {
    10.0 * x.max(1e-20).log10()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(args.get(1).expect("usage: stream_echo_eval <dir> [out dir]"));
    let out_dir = args.get(2).map(PathBuf::from);
    let voice = read_f32(&dir.join("voice.f32"));
    let game = read_f32(&dir.join("game.f32"));
    let frames = (voice.len().min(game.len() / 2)) / (FRAME / 2);
    let n = frames * FRAME / 2;
    // The call as played: the voices in both ears.
    let played: Vec<f32> = voice[..n].iter().flat_map(|v| [*v, *v]).collect();
    // Who's talking, per 10 ms.
    let talking: Vec<bool> = (0..frames)
        .map(|f| voice[f * 480..(f + 1) * 480].iter().map(|v| v * v).sum::<f32>() / 480.0 > 1e-5)
        .collect();

    let scenes = [
        Leak { name: "no leak (Windows leaves Pyr out)", delay_ms: 0, gain: 0.0, shape: |x| x, lowpass: 0.0 },
        Leak { name: "Voicemeeter-like: 40 ms, as is", delay_ms: 40, gain: 0.9, shape: |x| x, lowpass: 0.0 },
        Leak { name: "Sonar-like: 70 ms, louder, equalized", delay_ms: 70, gain: 1.4, shape: |x| x, lowpass: 0.35 },
        Leak { name: "with a compressor: 55 ms, squashed", delay_ms: 55, gain: 1.0, shape: |x| (2.5 * x).tanh() / 2.5, lowpass: 0.0 },
        Leak { name: "long way round: 180 ms", delay_ms: 180, gain: 0.8, shape: |x| x, lowpass: 0.0 },
        Leak { name: "quiet: 50 ms, 20 dB down", delay_ms: 50, gain: 0.1, shape: |x| x, lowpass: 0.0 },
    ];
    println!("{:<40} {:>10} {:>16} {:>16} {:>10}", "scene", "echo left", "game changed:", "game changed:", "delay");
    println!("{:<40} {:>10} {:>16} {:>16} {:>10}", "", "(dB)", "talking (dB)", "quiet (dB)", "found");
    for (si, s) in scenes.iter().enumerate() {
        let d = s.delay_ms * 48;
        let mut lp = [0f32; 2];
        let leak: Vec<f32> = (0..n * 2)
            .map(|i| {
                let ch = i % 2;
                let src = if i / 2 >= d { played[i - 2 * d] } else { 0.0 };
                let mut v = (s.shape)(src) * s.gain;
                if s.lowpass > 0.0 {
                    lp[ch] += (1.0 - s.lowpass) * (v - lp[ch]);
                    v = lp[ch];
                }
                v
            })
            .collect();
        let capture: Vec<f32> = (0..n * 2).map(|i| game[i] + leak[i]).collect();
        let mut ec = StreamEcho::new();
        let mut out = capture.clone();
        let t = std::time::Instant::now();
        for f in 0..frames {
            ec.played(played[f * FRAME..(f + 1) * FRAME].iter().copied());
            ec.clean(&mut out[f * FRAME..(f + 1) * FRAME]);
        }
        let took = t.elapsed();
        // Skip the first 10 s (finding the call and learning the way it comes back).
        let from = 1000;
        let (mut e_res, mut e_leak, mut g_t, mut g_q, mut d_t, mut d_q) = (0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
        for f in from..frames {
            for i in f * FRAME..(f + 1) * FRAME {
                let err = (out[i] - game[i]) as f64;
                let g = (game[i] as f64).powi(2);
                if talking[f.saturating_sub(s.delay_ms / 10)] {
                    e_res += err * err;
                    e_leak += (leak[i] as f64).powi(2);
                    g_t += g;
                    d_t += err * err;
                } else {
                    g_q += g;
                    d_q += err * err;
                }
            }
        }
        let echo_left = if s.gain > 0.0 { format!("{:.1}", db(e_res / e_leak)) } else { "-".into() };
        // With no leak, everything that changed is damage to the game sound.
        println!(
            "{:<40} {:>10} {:>16.1} {:>16.1} {:>10}",
            s.name,
            echo_left,
            db(d_t / g_t),
            db(d_q / g_q),
            ec.found().map(|d| format!("{d:.1} ms")).unwrap_or("-".into())
        );
        if si == 0 {
            eprintln!("  ({:.0}x faster than real time)", (frames as f64 / 100.0) / took.as_secs_f64());
        }
        if let Some(o) = &out_dir {
            std::fs::create_dir_all(o).unwrap();
            write_wav(&o.join(format!("{si}-captured.wav")), &capture);
            write_wav(&o.join(format!("{si}-sent.wav")), &out);
        }
    }
}
