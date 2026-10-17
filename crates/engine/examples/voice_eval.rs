//! Measures the microphone pipeline on real recorded speech: how much of your words get cut
//! (the thing that sounds bad), and how far other voices and noise are turned down.
//!
//!   cargo run --release -p pyr-engine --example voice_eval -- <dir with 48 kHz mono wavs> [out dir]
//!
//! Uses three speech clips (A = you, B and C = other people) and makes scenes from them.
//! Each run is compared with the same scene through "Strong" (RNNoise only, always sending),
//! so what's measured is what the gate and voice isolation take away on top of RNNoise.

use std::path::Path;
use std::time::{Duration, Instant};

use pyr_engine::audio::capture::CapturePipeline;
use pyr_engine::audio::Controls;
use pyr_engine::config::{NoiseSuppression, Settings};

fn read_wav(p: &Path) -> Vec<f32> {
    let b = std::fs::read(p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    // Find the "data" chunk (16-bit PCM mono 48 kHz, as made by ffmpeg).
    let mut i = 12;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let n = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        if id == b"data" {
            return b[i + 8..(i + 8 + n).min(b.len())].chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect();
        }
        i += 8 + n + (n & 1);
    }
    panic!("no data in {}", p.display());
}

fn write_wav(p: &Path, x: &[f32]) {
    let mut b = Vec::with_capacity(44 + x.len() * 2);
    let n = (x.len() * 2) as u32;
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

fn db(e: f64) -> f64 {
    10.0 * (e.max(1e-12)).log10()
}

/// Scale to an active-speech RMS level (dBFS), measured over the louder frames.
fn level(x: &[f32], target_db: f32) -> Vec<f32> {
    let mut es: Vec<f64> = x.chunks(480).map(|c| c.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / 480.0).collect();
    es.sort_by(|a, b| b.partial_cmp(a).unwrap());
    let top = &es[..es.len() / 2];
    let cur = db(top.iter().sum::<f64>() / top.len() as f64) as f32;
    let g = 10f32.powf((target_db - cur) / 20.0);
    x.iter().map(|v| v * g).collect()
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
    }
}

/// Fan-like noise (pinkish) at an RMS level in dBFS.
fn noise(len: usize, db_fs: f32, seed: u64) -> Vec<f32> {
    let mut r = Rng(seed | 1);
    let (mut b0, mut b1, mut b2) = (0.0f32, 0.0f32, 0.0f32);
    let raw: Vec<f32> = (0..len)
        .map(|_| {
            let w = r.next();
            b0 = 0.99765 * b0 + w * 0.0990460;
            b1 = 0.96300 * b1 + w * 0.2965164;
            b2 = 0.57000 * b2 + w * 1.0526913;
            b0 + b1 + b2 + w * 0.1848
        })
        .collect();
    let rms = (raw.iter().map(|v| v * v).sum::<f32>() / len as f32).sqrt();
    let g = 10f32.powf(db_fs / 20.0) / rms;
    raw.into_iter().map(|v| v * g).collect()
}

fn mix(parts: &[(&[f32], usize)], len: usize) -> Vec<f32> {
    let mut out = vec![0.0; len];
    for (x, at) in parts {
        for (i, v) in x.iter().enumerate() {
            if at + i < len {
                out[at + i] += v;
            }
        }
    }
    out
}

/// What the others hear: the Opus packets actually sent, decoded and put where they belong
/// in time (silence where nothing was sent).
fn run(input: &[f32], settings: &Settings) -> Vec<f32> {
    let mut cap = CapturePipeline::new(settings).unwrap();
    let mut dec = opus::Decoder::new(48_000, opus::Channels::Mono).unwrap();
    let ctl = Controls::default();
    let t0 = Instant::now();
    let mut out = vec![0.0f32; input.len()];
    let mut frame = vec![0.0f32; 480];
    let mut pcm = vec![0.0f32; 960];
    for (k, chunk) in input.chunks_exact(480).enumerate() {
        frame.copy_from_slice(chunk);
        let mut packets: Vec<Vec<u8>> = Vec::new();
        cap.process(&mut frame, &ctl, t0 + Duration::from_millis(10 * k as u64), &mut |_, _, p| packets.push(p.to_vec()));
        if std::env::var("TRACE").is_ok() && settings.noise_suppression == NoiseSuppression::Isolation && k % 5 == 0 {
            let (p, e, r, g) = cap.debug_state();
            eprintln!("{:6.2}s p={p:.2} env={e:6.1} ref={r:6.1} gain={g:.2} sent={}", k as f32 / 100.0, packets.len());
        }
        // The 20 ms block that just completed ends with this 10 ms frame; a talk-spurt start
        // also sends the block before it.
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
    out
}

fn frame_energy(x: &[f32]) -> Vec<f64> {
    x.chunks(480).map(|c| c.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / c.len() as f64).collect()
}

/// Frames where `clean` (your voice alone) is in a word: within 30 dB of its loud frames.
fn word_frames(clean: &[f32]) -> Vec<bool> {
    let e = frame_energy(clean);
    let mut s = e.clone();
    s.sort_by(|a, b| b.partial_cmp(a).unwrap());
    let loud = s[s.len() / 10];
    e.iter().map(|v| db(*v) > db(loud) - 30.0).collect()
}

struct Score {
    onset_cuts: usize,
    mid_cuts: usize,
    cut_pct: f64,
    cuts_per_min: f64,
    soft_pct: f64,
    other_db: f64,
}

/// `you`: your clean voice in the scene (for where your words are); `other`: everything else
/// alone (to measure how far it's turned down where you aren't talking).
fn score(out: &[f32], reference: &[f32], you: &[f32], other: Option<&[f32]>, delay: usize) -> Score {
    let er_all = frame_energy(reference);
    let er0 = frame_energy(reference);
    let mut sorted = er0.clone();
    sorted.sort_by(|a, b| b.partial_cmp(a).unwrap());
    let ref_loud = sorted[sorted.len() / 10];
    let words: Vec<bool> = word_frames(you).iter().zip(&er0).map(|(w, e)| *w && db(*e) > db(ref_loud) - 30.0).collect();
    let eo = frame_energy(&out[delay.min(out.len())..]);
    let er = frame_energy(reference);
    let n = words.len().min(eo.len()).min(er.len());
    let (mut total, mut cut, mut soft, mut runs) = (0usize, 0usize, 0usize, 0usize);
    let (mut onset, mut mid) = (0usize, 0usize);
    let mut in_cut = false;
    for i in 0..n {
        if !words[i] {
            in_cut = false;
            continue;
        }
        total += 1;
        let drop = db(eo[i]) - db(er[i]);
        if drop < -10.0 {
            cut += 1;
            if !in_cut {
                runs += 1;
                // A cut within 100 ms of a phrase starting clips its start; later ones are holes.
                if (i.saturating_sub(10)..i).any(|j| !words[j]) { onset += 1 } else { mid += 1 }
                if std::env::var("ONSETS").is_ok() && i > 150 && onset <= 8 {
                    let ey = frame_energy(you);
                    for j in i.saturating_sub(4)..(i + 6).min(n) {
                        eprintln!("  f{j} word={} you={:6.1} ref={:6.1} out={:6.1}", words[j], db(ey[j]), db(er[j]), db(eo[j]));
                    }
                    eprintln!("  --");
                }
            }
            in_cut = true;
        } else {
            in_cut = false;
            if drop < -4.0 {
                soft += 1;
            }
        }
    }
    let minutes = total as f64 * 0.01 / 60.0;
    let other_db = other
        .map(|o| {
            let eot = frame_energy(o);
            let wy = word_frames(you);
            let (mut a, mut b) = (0.0, 0.0);
            for i in 0..n.min(eot.len()) {
                // Only where you're quiet and they're talking.
                // Compared with the reference (RNNoise only, always on), so it's what the gate
                // and isolation take away from other voices and noise.
                if !wy[i] && db(eot[i]) > -60.0 {
                    a += eo[i];
                    b += er_all[i];
                }
            }
            if b > 0.0 { db(a) - db(b) } else { f64::NAN }
        })
        .unwrap_or(f64::NAN);
    Score {
        onset_cuts: onset,
        mid_cuts: mid,
        cut_pct: 100.0 * cut as f64 / total.max(1) as f64,
        cuts_per_min: runs as f64 / minutes.max(1e-9),
        soft_pct: 100.0 * soft as f64 / total.max(1) as f64,
        other_db,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = Path::new(args.get(1).map(String::as_str).unwrap_or("."));
    let out_dir = args.get(2).map(|s| Path::new(s).to_path_buf());
    let delay: usize = std::env::var("DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    let a = read_wav(&dir.join("198-209-0000.wav"));
    let b = read_wav(&dir.join("3436-172162-0000.wav"));
    let c = read_wav(&dir.join("5703-47212-0000.wav"));
    let gap = 48_000 / 2;

    // You, three ways (normal, soft, loud), in a quiet room with a fan.
    let mut scenes: Vec<(&str, Vec<f32>, Vec<f32>, Option<Vec<f32>>)> = Vec::new();
    for (name, lvl, fan) in [("normal voice, quiet room", -26.0, -70.0), ("soft voice", -38.0, -70.0), ("loud voice", -16.0, -70.0), ("normal voice, noisy fan", -26.0, -45.0)] {
        let you = level(&a, lvl);
        let len = you.len() + gap;
        let n = noise(len, fan, 7);
        let mix_ = mix(&[(&you, gap / 2), (&n, 0)], len);
        let you_at = mix(&[(&you, gap / 2)], len);
        scenes.push((name, mix_, you_at, Some(n)));
    }
    // Someone else in the room / a TV 18 dB below you, taking turns with you.
    {
        let you = level(&a, -26.0);
        let them = level(&b, -44.0);
        let len = you.len() + them.len() + gap * 2;
        let fan = noise(len, -70.0, 9);
        let you_at = mix(&[(&you, gap / 2)], len);
        let them_at = mix(&[(&them, you.len() + gap)], len);
        let other = mix(&[(&them_at, 0), (&fan, 0)], len);
        let scene = mix(&[(&you_at, 0), (&other, 0)], len);
        scenes.push(("you, then a voice 18 dB lower (TV)", scene, you_at, Some(other)));
    }
    // A TV talking under you the whole time, 20 dB lower.
    {
        let you = level(&a, -26.0);
        let mut tv = level(&c, -46.0);
        tv.extend(level(&b, -46.0));
        let len = you.len() + gap;
        let you_at = mix(&[(&you, gap / 2)], len);
        let tv_at = mix(&[(&tv, 0)], len);
        let scene = mix(&[(&you_at, 0), (&tv_at, 0)], len);
        scenes.push(("you over a TV 20 dB lower", scene, you_at, Some(tv_at)));
    }
    // A TV 24 dB lower (a headset mic: your mouth is far closer than the TV), taking turns.
    {
        let you = level(&a, -26.0);
        let them = level(&b, -50.0);
        let len = you.len() + them.len() + gap * 2;
        let fan = noise(len, -70.0, 13);
        let you_at = mix(&[(&you, gap / 2)], len);
        let them_at = mix(&[(&them, you.len() + gap)], len);
        let other = mix(&[(&them_at, 0), (&fan, 0)], len);
        let scene = mix(&[(&you_at, 0), (&other, 0)], len);
        scenes.push(("you, then a voice 24 dB lower", scene, you_at, Some(other)));
    }
    // You talk normally, then 10 dB softer (leaning back, getting tired): never cut.
    {
        let loud = level(&a, -24.0);
        let soft = level(&c, -34.0);
        let len = loud.len() + soft.len() + gap * 2;
        let fan = noise(len, -70.0, 17);
        let you_at = mix(&[(&loud, gap / 2), (&soft, loud.len() + gap)], len);
        scenes.push(("you, then you 10 dB softer", mix(&[(&you_at, 0), (&fan, 0)], len), you_at, Some(fan)));
    }
    // Three different speakers as "you" (voices differ), normal level.
    for (name, x) in [("speaker 2 as you", &b), ("speaker 3 as you", &c)] {
        let you = level(x, -26.0);
        let len = you.len() + gap;
        let fan = noise(len, -70.0, 11);
        let you_at = mix(&[(&you, gap / 2)], len);
        scenes.push((name, mix(&[(&you_at, 0), (&fan, 0)], len), you_at, Some(fan)));
    }

    let base = Settings { echo_cancellation: false, ..Settings::default() };
    let reference_settings = Settings {
        noise_suppression: NoiseSuppression::Strong,
        vad_auto: false,
        vad_threshold_db: -200.0,
        ..base.clone()
    };
    println!("{:<38} {:>10} {:>8} {:>9} {:>9} {:>10} {:>7} {:>5}", "scene", "mode", "cut %", "cuts/min", "soft %", "others dB", "starts", "holes");
    for (name, scene, you, other) in &scenes {
        let reference = run(scene, &reference_settings);
        for (mode, ns) in [("strong", NoiseSuppression::Strong), ("isolation", NoiseSuppression::Isolation)] {
            let s = Settings { noise_suppression: ns, ..base.clone() };
            let out = run(scene, &s);
            let sc = score(&out, &reference, you, other.as_deref(), if ns == NoiseSuppression::Isolation { delay } else { 0 });
            println!(
                "{:<38} {:>10} {:>8.1} {:>9.1} {:>9.1} {:>10.1} {:>7} {:>5}",
                name, mode, sc.cut_pct, sc.cuts_per_min, sc.soft_pct, sc.other_db, sc.onset_cuts, sc.mid_cuts
            );
            if let Some(d) = &out_dir {
                let base = name.replace([' ', ',', '(', ')'], "_");
                write_wav(&d.join(format!("{base}-{mode}.wav")), &out);
                write_wav(&d.join(format!("{base}-you.wav")), you);
                if let Some(o) = other {
                    write_wav(&d.join(format!("{base}-other.wav")), o);
                }
            }
        }
    }
}
