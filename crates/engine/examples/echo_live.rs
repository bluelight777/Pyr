//! Whether Pyr finds the speakers in the microphone with real sound devices (the default ones):
//! plays bursts of noise like people talking, about -20 dBFS, and runs the microphone's
//! processing as a call does, saying each second what it knows.
//!
//!   cargo run --release -p pyr-engine --example echo_live -- 30

use std::time::{Duration, Instant};

use pyr_engine::audio::Controls;
use pyr_engine::audio::capture::CapturePipeline;
use pyr_engine::audio::device::{InputDevice, OutputDevice};
use pyr_engine::config::Settings;

fn main() -> anyhow::Result<()> {
    let secs: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(30);
    let mut out = OutputDevice::open(None)?;
    let mut inp = InputDevice::open(None)?;
    let mut cap = CapturePipeline::new(&Settings::default())?;
    let ctl = Controls::default();
    let start = Instant::now();
    let mut seed = 0x1234_5678u32;
    let mut lp = 0f32;
    let mut played = 0u64;
    let (mut heard, mut said) = (0u64, 0u64);
    let mut frame = [0f32; 480];
    let mut mix = vec![0f32; 960];
    while start.elapsed() < Duration::from_secs(secs) {
        while out.buffered_ms() < 30.0 {
            for i in 0..480 {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                lp += 0.3 * ((seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0 - lp);
                let t = (played * 480 + i as u64) as f32 / 48_000.0;
                let v = if (t / 3.1).fract() < 0.6 { 0.3 * lp } else { 0.0 };
                mix[2 * i] = v;
                mix[2 * i + 1] = v;
            }
            cap.render(&mix);
            out.write(&mix);
            played += 1;
        }
        while inp.read_frame(&mut frame) {
            cap.process(&mut frame, &ctl, Instant::now(), &mut |_, _, _| {});
            heard += 1;
        }
        if start.elapsed().as_secs() > said {
            said = start.elapsed().as_secs();
            println!("{said:>3} s: played {played} frames, heard {heard}: {} | {}", cap.echo_words(), cap.echo_debug());
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    Ok(())
}
