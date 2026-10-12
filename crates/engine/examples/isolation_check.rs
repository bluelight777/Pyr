//! Runs a recording through the microphone pipeline in each noise mode and reports, per
//! stretch of time, how loud the result is and how much of it would be sent.
//!
//!   cargo run --release -p ingle-engine --example isolation_check -- test.raw 10.6 21.85
//!
//! `test.raw` is 48 kHz mono f32; the two times split it into: you talking, only someone
//! else (quieter, across the room) talking, and both at once.

use std::time::{Duration, Instant};

use ingle_engine::audio::Controls;
use ingle_engine::audio::capture::CapturePipeline;
use ingle_engine::config::{NoiseSuppression, Settings};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let bytes = std::fs::read(&args[0])?;
    let input: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    let cuts: Vec<f64> = args[1..].iter().map(|s| s.parse().unwrap()).collect();
    let seg_of = |t: f64| cuts.iter().filter(|&&c| t >= c).count();
    println!("{:<12} {:>28} {:>28} {:>28}", "mode", "you talking", "only someone else", "both");
    for mode in [NoiseSuppression::Strong, NoiseSuppression::Isolation] {
        let settings = Settings { noise_suppression: mode, echo_cancellation: false, auto_gain: false, ..Default::default() };
        let mut p = CapturePipeline::new(&settings)?;
        let ctl = Controls::default();
        let t0 = Instant::now();
        // per segment: (sum of squares, samples, frames sent, frames)
        let mut stats = vec![(0f64, 0usize, 0usize, 0usize); cuts.len() + 1];
        for (i, chunk) in input.chunks_exact(480).enumerate() {
            let mut frame = chunk.to_vec();
            let t = i as f64 * 0.01;
            let mut sent = 0usize;
            p.process(&mut frame, &ctl, t0 + Duration::from_millis(i as u64 * 10), &mut |_, _, _| sent += 1);
            let s = &mut stats[seg_of(t)];
            s.0 += frame.iter().map(|v| (*v as f64).powi(2)).sum::<f64>();
            s.1 += frame.len();
            s.2 += sent;
            s.3 += 1;
        }
        let cells: Vec<String> = stats
            .iter()
            .map(|(sq, n, sent, frames)| {
                let db = 10.0 * (sq / *n as f64).max(1e-12).log10();
                // Packets are 20 ms, frames 10 ms.
                format!("{db:6.1} dB, sent {:3.0}% of time", 100.0 * (*sent as f64 * 2.0) / *frames as f64)
            })
            .collect();
        println!("{:<12} {:>28} {:>28} {:>28}", format!("{mode:?}"), cells[0], cells[1], cells[2]);
    }
    Ok(())
}
