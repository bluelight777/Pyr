//! Voice isolation against mechanical noise: runs prepared scenes (speech + noise WAVs, 48 kHz
//! mono) through the real microphone pipeline, Opus and all, and writes what the others would
//! hear, for scoring.
//!   cargo run --release -p pyr-engine --example iso_eval -- <scene dir> <out dir> scene...
//! Each scene is <name>_mix.wav in the scene dir. Variants: strong, iso063, isoA, isoB, isoB2.
use std::path::Path;
use std::time::{Duration, Instant};

use pyr_engine::audio::Controls;
use pyr_engine::audio::capture::CapturePipeline;
use pyr_engine::config::{NoiseSuppression, Settings};

fn read_wav(p: &Path) -> Vec<f32> {
    let b = std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    let mut i = 12;
    while i + 8 <= b.len() {
        let n = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        if &b[i..i + 4] == b"data" {
            return b[i + 8..(i + 8 + n).min(b.len())].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect();
        }
        i += 8 + n + (n & 1);
    }
    panic!("no data in {}", p.display());
}

fn write_wav(p: &Path, x: &[f32]) {
    let n = (x.len() * 2) as u32;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + n).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&48_000u32.to_le_bytes());
    b.extend_from_slice(&96_000u32.to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&n.to_le_bytes());
    for v in x {
        b.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    std::fs::write(p, b).unwrap();
}

/// What the others hear (decoded packets, silence where none were sent), packets sent, and
/// processing time per 10 ms.
fn run(input: &[f32], variant: &str) -> (Vec<f32>, usize, f64) {
    let ns = if variant == "strong" { NoiseSuppression::Strong } else { NoiseSuppression::Isolation };
    let settings = Settings { echo_cancellation: false, noise_suppression: ns, ..Settings::default() };
    let mut cap = CapturePipeline::new(&settings).unwrap();
    match variant {
        "iso063" => cap.without_dfn(),
        "isoA" => cap.tune_isolation(false, 0, -100.0),
        "isoB" => cap.tune_isolation(true, 3, -100.0),
        v if v.starts_with("isoL") => cap.tune_isolation(true, 3, v[4..].parse().unwrap()),
        v if v.starts_with("isoAL") => cap.tune_isolation(false, 0, v[5..].parse().unwrap()),
        _ => {}
    }
    let mut dec = opus::Decoder::new(48_000, opus::Channels::Mono).unwrap();
    let ctl = Controls::default();
    let t0 = Instant::now();
    let mut out = vec![0.0f32; input.len()];
    let mut frame = vec![0.0f32; 480];
    let mut pcm = vec![0.0f32; 960];
    let mut sent = 0;
    let mut busy = Duration::ZERO;
    for (k, chunk) in input.chunks_exact(480).enumerate() {
        frame.copy_from_slice(chunk);
        let mut packets: Vec<Vec<u8>> = Vec::new();
        let t = Instant::now();
        cap.process(&mut frame, &ctl, t0 + Duration::from_millis(10 * k as u64), &mut |_, _, p| packets.push(p.to_vec()));
        busy += t.elapsed();
        if std::env::var("TRACE").is_ok() {
            let (p, e, r, g) = cap.debug_state();
            eprintln!("{variant} {k} p={p:.2} lsnr={:.1} raw={e:.1} ref={r:.1} gain={g:.2} sent={}", cap.debug_lsnr(), packets.len());
        }
        sent += packets.len();
        let end_block = (k + 1) * 480;
        let n = packets.len();
        for (i, p) in packets.iter().enumerate() {
            let at = end_block as i64 - 960 * (n - i) as i64;
            if let Ok(got) = dec.decode_float(p, &mut pcm, false) {
                if at >= 0 {
                    for j in 0..got {
                        if let Some(o) = out.get_mut(at as usize + j) {
                            *o = pcm[j];
                        }
                    }
                }
            }
        }
    }
    let frames = (input.len() / 480).max(1);
    (out, sent, busy.as_secs_f64() * 1000.0 / frames as f64)
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (dir, out) = (Path::new(&a[1]), Path::new(&a[2]));
    std::fs::create_dir_all(out).unwrap();
    let variants: Vec<String> =
        std::env::var("VARIANTS").unwrap_or("strong,iso063,isoA,isoB".into()).split(',').map(String::from).collect();
    println!("{:<28} {:>8} {:>9} {:>9}", "scene", "variant", "packets", "ms/10ms");
    for scene in &a[3..] {
        let x = read_wav(&dir.join(format!("{scene}_mix.wav")));
        for v in &variants {
            let (y, sent, ms) = run(&x, v);
            write_wav(&out.join(format!("{scene}__{v}.wav")), &y);
            println!("{:<28} {:>8} {:>9} {:>9.2}", scene, v, sent, ms);
        }
    }
}
