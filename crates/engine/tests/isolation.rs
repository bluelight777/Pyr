//! Voice isolation: its noise remover (DeepFilterNet3, built into Pyr) loads, and mechanical
//! sounds with nobody talking (typing, a fan's hum) send nothing at all.
//!
//! How well it keeps your words while removing such sounds was measured with real recordings
//! (see examples/iso_eval.rs); this guards the parts a change could quietly break.

use std::f32::consts::PI;
use std::time::{Duration, Instant};

use pyr_engine::audio::Controls;
use pyr_engine::audio::capture::CapturePipeline;
use pyr_engine::config::{NoiseSuppression, Settings};

#[test]
fn isolation_loads_its_model_and_typing_alone_sends_nothing() {
    let settings =
        Settings { noise_suppression: NoiseSuppression::Isolation, echo_cancellation: false, auto_gain: false, ..Default::default() };
    if let Err(e) = pyr_engine::audio::dfn::Dfn::new() {
        panic!("the voice isolation model is built in and loads: {e}");
    }
    let mut p = CapturePipeline::new(&settings).unwrap();
    assert!(p.has_dfn(), "the voice isolation model is built in and loads");

    // Ten seconds of typing (sharp clicks every 60 to 190 ms) over a fan's hum and hiss.
    let ctl = Controls::default();
    let t0 = Instant::now();
    let mut rng = 0x1234_5678u32;
    let mut rand = move || {
        rng ^= rng << 13;
        rng ^= rng >> 17;
        rng ^= rng << 5;
        rng
    };
    let (mut next_click, mut sent, mut loud) = (2400usize, 0usize, 0f32);
    for i in 0..1000 {
        let mut frame = vec![0f32; 480];
        for (k, s) in frame.iter_mut().enumerate() {
            let n = i * 480 + k;
            *s = 0.03 * (n as f32 * 2.0 * PI * 120.0 / 48_000.0).sin() + (rand() as f32 / u32::MAX as f32 - 0.5) * 0.004;
            if n >= next_click {
                let t = (n - next_click) as f32;
                *s += 0.6 * (-t / 40.0).exp() * if n % 2 == 0 { 1.0 } else { -1.0 } * (rand() as f32 / u32::MAX as f32);
                if t > 240.0 {
                    next_click = n + 2_880 + rand() as usize % 6_240;
                }
            }
            loud = loud.max(s.abs());
        }
        p.process(&mut frame, &ctl, t0 + Duration::from_millis(i as u64 * 10), &mut |_, _, _| sent += 1);
    }
    assert!(loud > 0.3, "the clicks are loud ({loud})");
    assert_eq!(sent, 0, "typing and a fan with nobody talking: nothing is sent");
}
