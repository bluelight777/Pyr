//! Voice isolation's noise remover: DeepFilterNet3 (Schröter et al., MIT/Apache-2.0), a
//! neural network made for 48 kHz voice that runs in real time on the CPU. Unlike RNNoise it
//! removes sharp, irregular sounds too: typing, mouse clicks, clocks, tools and machines.
//!
//! Run with every stage on every frame. Its default skips stages on frames it judges clean,
//! and switching between the two paths makes the voice choppy (measured: typing under speech
//! scored DNSMOS SIG 2.1 that way, 3.5 like this, clean speech 3.6).

use std::sync::{Mutex, OnceLock};

use df::tract::{DfParams, DfTract, RuntimeParams};

/// Audio comes out this many 10 ms frames after it goes in (its STFT and look-ahead).
pub const LATENCY_FRAMES: usize = 3;

/// The model's files, unpacked once and shared. (The built model itself can't move between
/// threads, so each audio thread builds its own from these.)
fn params() -> Result<DfParams, String> {
    static P: OnceLock<Mutex<Option<Result<DfParams, String>>>> = OnceLock::new();
    let mut p = P.get_or_init(|| Mutex::new(None)).lock().unwrap_or_else(|e| e.into_inner());
    if p.is_none() {
        // DfParams::default() unpacks the built-in model and panics if that fails; it can't
        // here (the bytes are part of the program), but never take the app down over it.
        *p = Some(std::panic::catch_unwind(DfParams::default).map_err(|_| "the voice isolation model is damaged".to_string()));
    }
    p.as_ref().expect("set above").clone()
}

/// Unpack the model in the background, so joining a call doesn't wait for it.
pub fn preload() {
    let _ = std::thread::Builder::new().name("pyr-dfn-load".into()).spawn(|| {
        let _ = params();
    });
}

pub struct Dfn {
    m: DfTract,
    inp: ndarray::Array2<f32>,
    out: ndarray::Array2<f32>,
}

impl Dfn {
    pub fn new() -> Result<Dfn, String> {
        let t = std::time::Instant::now();
        // Every stage on every frame (see above); no attenuation limit.
        let rp = RuntimeParams::default_with_ch(1).with_thresholds(-100.0, 100.0, 100.0).with_atten_lim(100.0);
        let m = DfTract::new(params()?, &rp).map_err(|e| format!("{e:#}"))?;
        tracing::info!("voice isolation model ready in {} ms", t.elapsed().as_millis());
        let hop = m.hop_size;
        Ok(Dfn { m, inp: ndarray::Array2::zeros((1, hop)), out: ndarray::Array2::zeros((1, hop)) })
    }

    /// Clean 10 ms (480 samples, -1..1) in place; returns the model's estimate of the
    /// speech-to-noise ratio (dB). The output is `LATENCY_FRAMES` behind the input.
    pub fn process(&mut self, frame: &mut [f32]) -> f32 {
        let n = frame.len().min(self.inp.len());
        self.inp.as_slice_mut().expect("contiguous")[..n].copy_from_slice(&frame[..n]);
        match self.m.process(self.inp.view(), self.out.view_mut()) {
            Ok(lsnr) => {
                frame[..n].copy_from_slice(&self.out.as_slice().expect("contiguous")[..n]);
                lsnr
            }
            Err(e) => {
                tracing::warn!("voice isolation: {e:#}");
                0.0
            }
        }
    }
}
