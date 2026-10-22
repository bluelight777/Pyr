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

fn write_wav(p: &str, x: &[f32]) {
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
    let _ = std::fs::write(p, b);
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
/// quieter, smeared by a few reflections; `drift_ppm`: the microphone's clock and the speakers'
/// apart by that much (the delay grows).
#[derive(Clone, Copy)]
struct Room {
    delay_ms: usize,
    loss_db: f32,
    drift_ppm: f32,
}

/// What's sent (decoded), with the microphone open the whole time: `you` into the microphone,
/// `far` out of the speakers (and, through `room`, back into the microphone).
struct Run {
    out: Vec<f32>,
    said: Vec<String>,
    echo_states: Vec<String>,
    /// The speakers' sound as it reached the microphone (the echo alone).
    echo: Vec<f32>,
    /// Each 10 ms frame's levels: coming in, after echo cancellation and automatic gain, after
    /// noise suppression, going out.
    stages: Vec<[f32; 4]>,
}

fn run(settings: &Settings, you: &[f32], far: &[f32], room: Option<Room>, mic_gain: f32) -> Run {
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
    let mut echo_sig = vec![0.0f32; n];
    if let Some(r) = room {
        let g = 10f32.powf(-r.loss_db / 20.0);
        for (extra, amp) in [(0usize, 1.0f32), (7 * 48, 0.45), (19 * 48, 0.25), (41 * 48, 0.12)] {
            for (i, m) in echo_sig.iter_mut().enumerate() {
                // (Between samples when the clocks drift apart.)
                let at = i as f64 - (r.delay_ms * 48 + extra) as f64 - r.drift_ppm as f64 * 1e-6 * i as f64;
                if at >= 0.0 {
                    let j = at.floor() as usize;
                    let frac = (at - j as f64) as f32;
                    let a = far.get(j).copied().unwrap_or(0.0);
                    let b = far.get(j + 1).copied().unwrap_or(0.0);
                    *m += (a + (b - a) * frac) * g * amp;
                }
            }
        }
    }
    for (m, e) in mic.iter_mut().zip(&echo_sig) {
        *m += e;
    }
    // A microphone's own hiss (`NOISE` dBFS, -70 by default), and it can't send more than full
    // scale.
    let hiss = 10f32.powf(std::env::var("NOISE").ok().and_then(|v| v.parse().ok()).unwrap_or(-70.0f32) / 20.0) * 3f32.sqrt();
    let mut seed = 0x2545_f491u32;
    for v in mic.iter_mut() {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        *v = (*v + hiss * (seed as f32 / u32::MAX as f32 * 2.0 - 1.0)).clamp(-1.0, 1.0);
    }
    let mut out = vec![0.0f32; n];
    let mut said = Vec::new();
    let mut echo = Vec::new();
    let mut was = None;
    let mut stages = Vec::with_capacity(n / 480);
    // (`DUMP=dir`: what came in, what voice isolation's noise remover got, and what went out.)
    let mut before_dfn = Vec::new();
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
        stages.push(cap.stage_levels());
        if std::env::var("DUMP").is_ok() {
            before_dfn.extend_from_slice(cap.before_dfn());
        }
        let state = cap.echo_state();
        if state.map(|s| std::mem::discriminant(&s)) != was.map(|s: pyr_engine::audio::mic_echo::Echo| std::mem::discriminant(&s)) {
            echo.push(format!("{:.1}s {:?}", k as f32 / 100.0, state.unwrap()));
            was = state;
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
    if let Ok(dir) = std::env::var("DUMP") {
        let _ = std::fs::create_dir_all(&dir);
        let tag = std::env::var("TAG").unwrap_or_default();
        write_wav(&format!("{dir}/{tag}mic.wav"), &mic);
        write_wav(&format!("{dir}/{tag}before_dfn.wav"), &before_dfn);
        write_wav(&format!("{dir}/{tag}out.wav"), &out);
    }
    Run { out, said, echo_states: echo, echo: echo_sig, stages }
}

const W: usize = 9600;

/// (your talking turned down 10 dB or more, of how much; the echo that got through, dB).
fn score(you: &[f32], far: &[f32], out: &[f32], stages: &[[f32; 4]], echo_sig: &[f32]) -> (usize, usize, Option<f32>, usize) {
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
    // Of those, the ones while you were clearly (6 dB) louder than the echo.
    let mut clear = 0;
    {
        let mut k = W;
        let mut prev = false;
        while k + W <= you.len().min(out.len()) {
            let talking = db(&you[k..k + W]) > -45.0;
            if talking && prev {
                let g = db(&out[k..k + W]) - db(&you[k..k + W]);
                let e = echo_sig.get(k..k + W).map(db).unwrap_or(-120.0);
                if g < usual - 10.0 && db(&you[k..k + W]) > e + 6.0 {
                    clear += 1;
                }
            }
            prev = talking;
            k += W;
        }
    }
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
                    // Each stage's gain over the window (dB): echo cancellation and automatic
                    // gain, noise suppression, voice isolation.
                    let (a, b) = (k / 480, ((k + W) / 480).min(stages.len()));
                    let mean = |i: usize| {
                        let e = stages[a..b].iter().map(|s| 10f32.powf(s[i] / 10.0)).sum::<f32>() / (b - a).max(1) as f32;
                        10.0 * e.max(1e-12).log10()
                    };
                    list.push(format!(
                        "{:.1}s:{:.0}[aec{:+.0} ns{:+.0} iso{:+.0}]",
                        k as f32 / 48_000.0,
                        g - usual,
                        mean(1) - mean(0),
                        mean(2) - mean(1),
                        mean(3) - mean(2)
                    ));
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
    (down, gains.len(), echo, clear)
}

/// The echo left under your talking (dB against what went out), in 200 ms windows where you
/// talk and the speakers' sound reaches the microphone: how much of the echo is in what went
/// out (least squares, at the delay where it's most alike).
fn echo_under_talking(you: &[f32], echo: &[f32], out: &[f32]) -> Option<f32> {
    let n = you.len().min(echo.len()).min(out.len());
    // The delay through the processing (0 to 60 ms), where the output is most like the echo.
    let corr = |d: usize| -> f64 { (0..n - d).step_by(3).map(|i| out[i + d] as f64 * echo[i] as f64).sum::<f64>().abs() };
    let d = (0..=60).map(|ms| ms * 48).max_by(|a, b| corr(*a).total_cmp(&corr(*b)))?;
    let (mut left, mut total) = (0f64, 0f64);
    let mut k = 0;
    while k + W + d <= n {
        if db(&you[k..k + W]) > -45.0 && db(&echo[k..k + W]) > -70.0 {
            let o = &out[k + d..k + d + W];
            let e = &echo[k..k + W];
            let ee: f64 = e.iter().map(|v| (*v as f64).powi(2)).sum();
            let oe: f64 = o.iter().zip(e).map(|(a, b)| *a as f64 * *b as f64).sum();
            let oo: f64 = o.iter().map(|v| (*v as f64).powi(2)).sum();
            if ee > 0.0 {
                left += oe * oe / ee;
                total += oo;
            }
        }
        k += W;
    }
    (total > 0.0).then(|| (10.0 * (left.max(1e-20) / total).log10()) as f32)
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
    let speakers = Some(Room { delay_ms: 25, loss_db: 18.0, drift_ppm: 0.0 });
    let quiet_room = Some(Room { delay_ms: 25, loss_db: 34.0, drift_ppm: 0.0 });
    // Open headphones turned up: a little of them reaches the microphone.
    let leaky = Some(Room { delay_ms: 25, loss_db: 50.0, drift_ppm: 0.0 });
    // A USB microphone and the PC's speakers, each on its own clock.
    let drifting = Some(Room { delay_ms: 25, loss_db: 18.0, drift_ppm: 100.0 });
    let music = tunes(-18.0);
    // A hot microphone: your talking 20 dB louder (a mixer or a microphone turned right up), its
    // loudest moments cut off at full scale (bug reports 102 to 119: the voice isolation said
    // the sound was clipping, thousands of times).
    let hot = std::env::var("HOT").map(|v| v.parse::<f32>().unwrap_or(20.0)).unwrap_or(20.0);
    let scenes: Vec<(&str, &Vec<f32>, Option<Room>, f32)> = vec![
        ("headphones, others talking", &voices, headphones, 0.0),
        ("headphones, music playing", &music, headphones, 0.0),
        ("headphones, loud music and voices", &loud, headphones, 0.0),
        ("speakers, others talking", &voices, speakers, 0.0),
        ("speakers, music playing", &music, speakers, 0.0),
        ("speakers turned low, music playing", &music, quiet_room, 0.0),
        ("hot mic, headphones, others talking", &voices, headphones, hot),
        ("hot mic, headphones, music playing", &music, headphones, hot),
        ("hot mic, speakers, others talking", &voices, speakers, hot),
        ("leaky headphones, music playing", &music, leaky, 0.0),
        ("speakers, music, clocks apart", &music, drifting, 0.0),
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
            // (`PTT=1`: push to talk, held all along: the microphone always open, so what's left
            // of the echo goes out whether or not you talk.)
            if std::env::var("PTT").is_ok_and(|v| v == "1") {
                s.input_mode = pyr_engine::config::InputMode::PushToTalk;
            }
            (n, s)
        })
        .filter(|(n, _)| std::env::var("SETUP").map(|w| n.contains(&w)).unwrap_or(true))
        .collect();
    for (sname, s) in &setups {
        println!("== {sname}");
        for (name, far, room, hot_db) in &scenes {
            // (`ONLY=part` of the scene's name, or `ONLY==name` exactly.)
            if only.as_deref().is_some_and(|o| match o.strip_prefix('=') {
                Some(exact) => *name != exact,
                None => !name.contains(o),
            }) {
                continue;
            }
            // (A hot microphone's talking as it comes in: louder, its peaks cut off.)
            let you: Vec<f32> = if *hot_db > 0.0 {
                let g = 10f32.powf(hot_db / 20.0);
                you_talk.iter().map(|v| (v * g).clamp(-1.0, 1.0)).collect()
            } else {
                you_talk.clone()
            };
            let mut r = run(s, &you, far, *room, 1.0);
            // What went out comes out a little later than it went in (voice isolation's noise
            // remover alone takes 30 ms): line it up with your talking first, so a word's start
            // arriving late isn't taken for it being turned down (`ALIGN=0`: as it comes).
            if std::env::var("ALIGN").map(|v| v != "0").unwrap_or(true) {
                let n = you.len().min(r.out.len());
                let corr = |d: usize| -> f64 { (0..n - d).step_by(2).map(|i| r.out[i + d] as f64 * you[i] as f64).sum::<f64>() };
                let d = (0..=100).map(|ms| ms * 48).max_by(|a, b| corr(*a).total_cmp(&corr(*b))).unwrap_or(0);
                r.out.drain(..d);
                r.out.extend(std::iter::repeat_n(0.0, d));
                r.stages.drain(..(d / 480).min(r.stages.len()));
            }
            let (out, said, found) = (&r.out, &r.said, &r.echo_states);
            let (down, of, echo, clear) = score(&you, far, out, &r.stages, &r.echo);
            let echo = echo.map(|e| format!("  echo through: {e:+.0} dB")).unwrap_or_default();
            let under = room.and_then(|_| echo_under_talking(&you, &r.echo, out)).map(|e| format!(", under talking {e:+.0} dB")).unwrap_or_default();
            let clear = room.map(|_| format!(" ({clear} clearly over the echo)")).unwrap_or_default();
            println!("  {name:<40} your talking turned down: {down:>3} of {of:>3}{clear}{echo}{under}");
            if !found.is_empty() {
                println!("      speakers in the microphone: {}", found.join(", "));
            }
            for w in said.iter().take(2) {
                println!("      noticed {w}");
            }
        }
    }
}
