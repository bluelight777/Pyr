//! DeepFilterNet3 on a 48 kHz mono WAV, 10 ms at a time: how long each frame takes, and the
//! result to listen to.  cargo run --release -p ingle-engine --example dfn_bench -- in.wav out.wav
use std::time::Instant;

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
    let n = (x.len() * 2) as u32;
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + n).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    for v in [16u32] { b.extend_from_slice(&v.to_le_bytes()); }
    for v in [1u16, 1] { b.extend_from_slice(&v.to_le_bytes()); }
    for v in [48_000u32, 96_000] { b.extend_from_slice(&v.to_le_bytes()); }
    for v in [2u16, 16] { b.extend_from_slice(&v.to_le_bytes()); }
    b.extend_from_slice(b"data");
    b.extend_from_slice(&n.to_le_bytes());
    for v in x { b.extend_from_slice(&((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()); }
    std::fs::write(p, b).unwrap();
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let x = read_wav(&a[1]);
    let t0 = Instant::now();
    let env = |k: &str, d: f32| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let rp = df::tract::RuntimeParams::default_with_ch(1)
        .with_thresholds(env("DFN_MIN", -10.0), env("DFN_MAXERB", 30.0), env("DFN_MAXDF", 20.0))
        .with_atten_lim(env("DFN_ATT", 100.0))
        .with_post_filter(env("DFN_PF", 0.0));
    let t1 = Instant::now();
    let params = df::tract::DfParams::default();
    println!("unpack {:.0} ms", t1.elapsed().as_secs_f64() * 1000.0);
    let t2 = Instant::now();
    let mut m = df::tract::DfTract::new(params, &rp).unwrap();
    println!("build {:.0} ms", t2.elapsed().as_secs_f64() * 1000.0);
    println!("load {:.0} ms, hop {}, lookahead {} frames", t0.elapsed().as_secs_f64() * 1000.0, m.hop_size, m.lookahead);
    let hop = m.hop_size;
    let mut out = Vec::with_capacity(x.len());
    let mut times = Vec::new();
    let mut enh = ndarray::Array2::<f32>::zeros((1, hop));
    for c in x.chunks_exact(hop) {
        let inp = ndarray::ArrayView2::from_shape((1, hop), c).unwrap();
        let t = Instant::now();
        m.process(inp, enh.view_mut()).unwrap();
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        out.extend(enh.iter());
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    println!("per 10 ms frame: mean {:.2} ms, p50 {:.2}, p99 {:.2}, max {:.2} (real-time load {:.0}% of one core)", mean, times[times.len() / 2], times[times.len() * 99 / 100], times[times.len() - 1], mean / 10.0 * 100.0);
    write_wav(&a[2], &out);
}
