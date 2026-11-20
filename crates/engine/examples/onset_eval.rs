//! Your words' beginnings (Blue, Nov 19, on 0.16.0: "It seems like it's cutting off the
//! beginning of each of my words and sometimes I don't see the voice visualizer moving when I
//! speak", and "it's also cutting off the beginning of some of Mag's words too"): someone
//! talking as people do in a call, phrases with pauses between them, through Pyr's microphone
//! processing as a call has it (echo cancellation and automatic gain on, the microphone
//! opening by itself). For each phrase: how much of its start the others don't hear, whether
//! it went out at all, and whether the voice meter moved for it (it moves only while the
//! microphone is open).
//!
//!   cargo run --release -p pyr-engine --example onset_eval -- talk.wav talk.txt [options]
//!
//! `talk.wav` and `talk.txt` from `tools/make-talk.py` (48 kHz mono 16-bit, the phrases'
//! starts and ends). Options:
//!   --level DB      your voice at the microphone, dBFS over your speech (-25 unless told)
//!   --noise DB      a fan's hum in the room at this level (only a faint hiss unless told)
//!   --tv FILE DB    a TV across the room (`tools/make-tv-room.py`) at this level
//!   --far FILE      the others talking (as played, -20 dBFS), all along...
//!   --gaps          ...or only in your pauses of 2 s or more
//!   --loss DB       ...out of speakers, reaching the microphone this much quieter (60 ms
//!                   later, with reflections); without it, in headphones (none of it does)
//!   --strong        standard noise suppression instead of voice isolation
//!   --from S        count phrases from S seconds on (60 unless told: after the first minute)
//! `PYR_AB` and `PYR_FIX` work as in Pyr; `TRACE=1` prints every phrase.
//!
//! A phrase starts at its first moment within 15 dB of its loudest (10 ms at a time; a breath
//! before it doesn't count), and its start is lost up to the first of its loud moments (within
//! 25 dB of its loudest) that goes out within 10 dB of how loud its loud moments go out at most
//! (their 90th percentile): voice isolation turns what isn't you 26 dB down, and a closed
//! microphone sends nothing.

use std::sync::atomic::Ordering;
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

fn pct(v: &[f32], q: f32) -> f32 {
    if v.is_empty() {
        return f32::NAN;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    s[((s.len() - 1) as f32 * q).round() as usize]
}

const SEC: usize = 48_000;
const F: usize = 480;

struct Phrase {
    a: usize,
    b: usize,
    level: f32,
    name: String,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let you_clean = read_wav(&args[1]);
    let phrases: Vec<Phrase> = std::fs::read_to_string(&args[2])
        .unwrap()
        .lines()
        .filter_map(|l| {
            let w: Vec<&str> = l.split_whitespace().collect();
            Some(Phrase { a: w.first()?.parse().ok()?, b: w.get(1)?.parse().ok()?, level: w.get(2)?.parse().ok()?, name: w.get(3).unwrap_or(&"").to_string() })
        })
        .collect();
    let (mut level, mut noise_db, mut tv, mut far_file, mut gaps, mut loss, mut strong, mut from) = (-25.0f32, None::<f32>, None::<(String, f32)>, None::<String>, false, None::<f32>, false, 60.0f32);
    let mut i = 3;
    while i < args.len() {
        let next = |k: usize| args.get(k).cloned().expect("an option needs a value");
        match args[i].as_str() {
            "--level" => (level, i) = (next(i + 1).parse().unwrap(), i + 1),
            "--noise" => (noise_db, i) = (Some(next(i + 1).parse().unwrap()), i + 1),
            "--tv" => (tv, i) = (Some((next(i + 1), next(i + 2).parse().unwrap())), i + 2),
            "--far" => (far_file, i) = (Some(next(i + 1)), i + 1),
            "--gaps" => gaps = true,
            "--loss" => (loss, i) = (Some(next(i + 1).parse().unwrap()), i + 1),
            "--strong" => strong = true,
            "--from" => (from, i) = (next(i + 1).parse().unwrap(), i + 1),
            other => panic!("what's {other}?"),
        }
        i += 1;
    }
    let n = you_clean.len() / F * F;
    let g = 10f32.powf((level + 25.0) / 20.0);
    let you: Vec<f32> = you_clean[..n].iter().map(|v| v * g).collect();
    // What the speakers play: the others, all along or in your longer pauses.
    let mut far = vec![0.0f32; n];
    if let Some(f) = &far_file {
        let other = scaled(&read_wav(f), -20.0);
        let mut at = 0;
        let plays = |t: usize| {
            !gaps || {
                let next = phrases.iter().find(|p| p.b > t);
                let last_end = phrases.iter().filter(|p| p.b <= t).map(|p| p.b).max().unwrap_or(0);
                match next {
                    // (In a pause of 2 s or more, from half a second after you stop to half a second before you start.)
                    Some(p) if p.a <= t => false,
                    Some(p) => p.a.saturating_sub(last_end) >= 2 * SEC && t >= last_end + SEC / 2 && t + SEC / 2 <= p.a,
                    None => t >= last_end + SEC / 2,
                }
            }
        };
        let mut k = 0;
        while k < n {
            let on = plays(k);
            for j in k..(k + F).min(n) {
                if on {
                    far[j] = other[at % other.len()];
                    at += 1;
                }
            }
            k += F;
        }
    }
    // The microphone: you, a faint hiss, and the room's hum, a TV, the speakers if told.
    let mut mic = you.clone();
    let mut seed = 0x2545_f491u32;
    let mut white = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    let mut room = vec![0.0f32; n];
    if let Some(d) = noise_db {
        // (A fan: pink noise, Paul Kellet's filter.)
        let (mut b0, mut b1, mut b2) = (0.0f32, 0.0f32, 0.0f32);
        for v in room.iter_mut() {
            let w = white();
            b0 = 0.99765 * b0 + w * 0.099_046;
            b1 = 0.963 * b1 + w * 0.296_516_4;
            b2 = 0.57 * b2 + w * 1.052_691_3;
            *v = b0 + b1 + b2 + w * 0.1848;
        }
        room = scaled(&room, d);
    }
    if let Some((f, d)) = &tv {
        let t = scaled(&read_wav(f), *d);
        for (j, v) in room.iter_mut().enumerate() {
            *v += t[j % t.len()];
        }
    }
    let mut echo = vec![0.0f32; n];
    if let Some(l) = loss {
        let g = 10f32.powf(-l / 20.0);
        for (extra, amp) in [(0usize, 1.0f32), (7 * 48, 0.45), (19 * 48, 0.25), (41 * 48, 0.12)] {
            let d = 60 * 48 + extra;
            for j in d..n {
                echo[j] += far[j - d] * g * amp;
            }
        }
    }
    for j in 0..n {
        mic[j] = (mic[j] + room[j] + echo[j] + 10f32.powf(-70.0 / 20.0) * 3f32.sqrt() * white()).clamp(-1.0, 1.0);
    }
    let mode = if strong { "standard noise suppression" } else { "voice isolation" };
    println!(
        "{mode}; you at {level:.0} dBFS, {} phrases in {:.0} s; the room: {}{}; the others: {} (PYR_AB={:?} PYR_FIX={:?})",
        phrases.len(),
        n as f32 / SEC as f32,
        noise_db.map(|d| format!("a fan at {d:.0} dBFS")).unwrap_or("quiet".into()),
        tv.as_ref().map(|(_, d)| format!(", a TV at {d:.0} dBFS")).unwrap_or_default(),
        match (&far_file, loss) {
            (None, _) => "quiet".to_string(),
            (Some(_), l) => format!(
                "{}{}",
                if gaps { "talking in your longer pauses" } else { "talking all along" },
                l.map(|l| format!(", out of speakers {l:.0} dB down at the microphone")).unwrap_or(", in headphones".into())
            ),
        },
        std::env::var("PYR_AB").unwrap_or_default(),
        std::env::var("PYR_FIX").unwrap_or_default()
    );

    let s = Settings { noise_suppression: if strong { NoiseSuppression::Strong } else { NoiseSuppression::Isolation }, ..Settings::default() };
    let mut cap = CapturePipeline::new(&s).unwrap();
    cap.stream_delay_ms = 60;
    let mut dec = opus::Decoder::new(48_000, opus::Channels::Mono).unwrap();
    let ctl = Controls::default();
    let t0 = Instant::now();
    let mut out = vec![0.0f32; n];
    let mut frame = vec![0.0f32; F];
    let mut stereo = vec![0.0f32; 2 * F];
    let mut pcm = vec![0.0f32; 960];
    let mut open = Vec::with_capacity(n / F);
    let mut views = Vec::with_capacity(n / F);
    // (DUMP=file: each 10 ms, for looking into: frame, the microphone's level, speech
    // probability, DeepFilterNet's speech-to-noise, your usual level, the dips, taken as yours,
    // the microphone open, isolation's gain, the level after echo cancellation and automatic
    // gain, and after the noise remover, and whether a voice across the room was heard lately.)
    let mut dump = std::env::var_os("DUMP").map(|p| std::io::BufWriter::new(std::fs::File::create(p).unwrap()));
    for k in 0..n / F {
        for j in 0..F {
            let v = far[k * F + j];
            stereo[2 * j] = v;
            stereo[2 * j + 1] = v;
        }
        cap.render(&stereo);
        frame.copy_from_slice(&mic[k * F..(k + 1) * F]);
        let mut packets: Vec<Vec<u8>> = Vec::new();
        let now = t0 + Duration::from_millis(10 * k as u64);
        cap.process(&mut frame, &ctl, now, &mut |_, _, p| packets.push(p.to_vec()));
        open.push(ctl.transmitting.load(Ordering::Relaxed));
        views.push(cap.isolation_view(now));
        if let Some(d) = dump.as_mut() {
            use std::io::Write;
            let v = cap.isolation_view(now);
            let (prob, _, _, gain) = cap.debug_state();
            let st = cap.stage_levels();
            let _ = writeln!(d, "{k} {:.2} {prob:.3} {:.1} {:.1} {:.1} {} {} {gain:.3} {:.2} {:.2} {}", st[0], cap.debug_lsnr(), v.1, v.0, v.2 as u8, *open.last().unwrap() as u8, st[1], st[2], cap.far_voice_heard(now) as u8);
        }
        let end = (k + 1) * F;
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
    // How far behind the microphone what goes out is: the lag (0 to 120 ms) at which the two
    // match best, over your phrases (at 12 kHz).
    let lag = {
        let dec4 = |x: &[f32]| x.chunks_exact(4).map(|c| c.iter().sum::<f32>() / 4.0).collect::<Vec<f32>>();
        let (y, o) = (dec4(&you), dec4(&out));
        let mut best = (0usize, f32::MIN);
        for l in 0..=1440 {
            let mut s = 0.0f64;
            for p in phrases.iter().take(40) {
                for t in p.a / 4..(p.b / 4).min(o.len().saturating_sub(l + 1)) {
                    s += (y[t] * o[t + l]) as f64;
                }
            }
            if s as f32 > best.1 {
                best = (l, s as f32);
            }
        }
        best.0 * 4
    };
    // Each phrase, 10 ms at a time (in the microphone's time): its loud moments, what came in
    // and how much louder or quieter they went out.
    struct Seen {
        pi: usize,
        /// The loud moments' frames, and each one's level going out against coming in (dB).
        loud: Vec<usize>,
        d: Vec<f32>,
        /// How loud the loud moments go out at most (their 90th percentile, against coming in).
        r: f32,
        /// The share of the phrase the microphone was open for (the meter moved).
        meter: f32,
    }
    let mut seen: Vec<Seen> = Vec::new();
    for (pi, p) in phrases.iter().enumerate() {
        if (p.a as f32) < from * SEC as f32 || p.b / F + lag / F + 4 >= n / F {
            continue;
        }
        let frames: Vec<usize> = (p.a / F..p.b / F).collect();
        let c: Vec<f32> = frames.iter().map(|&f| db(&you[f * F..(f + 1) * F])).collect();
        let top = pct(&c, 0.95);
        // (Its start: the first moment within 15 dB of its loudest, not a breath before it.)
        let begin = (0..frames.len()).find(|&j| c[j] > top - 15.0).unwrap_or(0);
        let loud: Vec<usize> = (begin..frames.len()).filter(|&j| c[j] > top - 25.0).collect();
        let d: Vec<f32> = loud.iter().map(|&j| db(&out[frames[j] * F + lag..(frames[j] + 1) * F + lag]) - c[j]).collect();
        // (The microphone decides 30 ms ahead of the sound it lets through: the meter's frames
        // for this phrase are the decisions' frames.)
        let meter = frames.iter().filter(|&&f| open[(f + 3).min(open.len() - 1)]).count() as f32 / frames.len() as f32;
        seen.push(Seen { pi, loud: loud.iter().map(|&j| frames[j]).collect(), r: pct(&d, 0.9), d, meter });
    }
    // A phrase that never went out has even its loudest going out far under the others'.
    let usual = pct(&seen.iter().map(|s| s.r).collect::<Vec<f32>>(), 0.5);
    let mut starts = Vec::new();
    let (mut never, mut over50, mut over100, mut over300, mut meter_still, mut meter_short) = (0, 0, 0, 0, 0, 0);
    let (mut lost_frames, mut all_frames) = (0.0f32, 0usize);
    for s in &seen {
        let p = &phrases[s.pi];
        let ok: Vec<bool> = s.d.iter().map(|&v| v > s.r - 10.0).collect();
        let first = (0..ok.len()).find(|&j| ok[j] && ok.get(j + 1).copied().unwrap_or(true));
        let went = s.r > usual - 15.0 && first.is_some();
        let lost_ms = first.map(|j| (s.loud[j] - s.loud[0]) as f32 * 10.0);
        let lost = if went { ok.iter().filter(|&&o| !o).count() as f32 / ok.len().max(1) as f32 } else { 1.0 };
        if went {
            let ms = lost_ms.unwrap();
            starts.push(ms);
            over50 += (ms > 50.0) as usize;
            over100 += (ms > 100.0) as usize;
            over300 += (ms > 300.0) as usize;
        } else {
            never += 1;
        }
        meter_still += (s.meter < 0.05) as usize;
        meter_short += (s.meter < 0.5) as usize;
        lost_frames += lost * s.loud.len() as f32;
        all_frames += s.loud.len();
        if std::env::var_os("TRACE").is_some() {
            let v = views[(p.a / F).min(views.len() - 1)];
            println!(
                "  {:6.1} s {:+5.1} dB {:4.1} s: {} meter {:3.0}%, lost {:3.0}% (your level then {:.0}, dips {:.0}) {}",
                p.a as f32 / SEC as f32,
                p.level,
                (p.b - p.a) as f32 / SEC as f32,
                if went { format!("start lost {:4.0} ms,", lost_ms.unwrap()) } else { "never went out,     ".into() },
                s.meter * 100.0,
                lost * 100.0,
                v.1,
                v.0,
                p.name
            );
        }
    }
    let k = seen.len();
    println!(
        "  {k} phrases from {from:.0} s (out {:.0} ms behind): never went out {never}; start lost: median {:.0} ms, 90% under {:.0} ms, over 50 ms {over50}, over 100 ms {over100}, over 300 ms {over300}; your loud moments lost {:.1}%; the meter still {meter_still}, moving for less than half {meter_short}",
        lag as f32 / 48.0,
        pct(&starts, 0.5),
        pct(&starts, 0.9),
        100.0 * lost_frames / all_frames.max(1) as f32
    );
    // While you're quiet (from a second after a phrase to a fifth of a second before the next):
    // each second, what went out.
    let (mut quiet, mut heard, mut loud) = (0, 0, 0);
    let mut t = (from as usize) * SEC;
    while t + SEC + lag <= n {
        let w = t..t + SEC;
        t += SEC;
        if phrases.iter().any(|p| w.start < p.b + SEC && w.end + SEC / 5 > p.a) {
            continue;
        }
        let o = db(&out[w.start + lag..w.end + lag]);
        quiet += 1;
        heard += (o > -60.0) as usize;
        loud += (o > -45.0) as usize;
    }
    println!("  while you're quiet: something went out (over -60 dBFS) {heard} of {quiet} s, loud (over -45) {loud} s");
}
