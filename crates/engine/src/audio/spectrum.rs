//! What a voice sounds like right now, as a handful of bands from deep to bright: for the
//! Spectrum voice meter. Fed 10 ms blocks of mono 48 kHz audio; each feed looks at the last
//! 20 ms through a Hann window (50 Hz between FFT bins) and sums the power in each band.
//! Bands are spaced evenly in pitch (log frequency), from 90 Hz to 11 kHz, and tilted up by 3 dB
//! an octave so a voice (louder low down) fills them evenly.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

/// How many bands.
pub const BANDS: usize = 16;
const BLOCK: usize = 480;
const N: usize = 2 * BLOCK;
const RATE: f32 = 48_000.0;
const LOW_HZ: f32 = 90.0;
const HIGH_HZ: f32 = 11_000.0;
/// The quietest and loudest band that show (dB below a full-scale sine).
const FLOOR_DB: f32 = -68.0;
const TOP_DB: f32 = -22.0;

pub struct Analyzer {
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    /// The last 20 ms, oldest first.
    ring: Vec<f32>,
    input: Vec<f32>,
    output: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// The FFT bins each band starts at (and the last one's end).
    edges: [usize; BANDS + 1],
    /// Each band's tilt (dB) and how many bins it has.
    tilt: [f32; BANDS],
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl Analyzer {
    pub fn new() -> Analyzer {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(N);
        let window = (0..N).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / N as f32).cos()).collect();
        let bin_hz = RATE / N as f32;
        let mut edges = [0usize; BANDS + 1];
        for (i, e) in edges.iter_mut().enumerate() {
            let hz = LOW_HZ * (HIGH_HZ / LOW_HZ).powf(i as f32 / BANDS as f32);
            *e = (hz / bin_hz).round() as usize;
        }
        // Every band at least one bin wide.
        for i in 1..=BANDS {
            if edges[i] <= edges[i - 1] {
                edges[i] = edges[i - 1] + 1;
            }
        }
        let mut tilt = [0f32; BANDS];
        for (i, t) in tilt.iter_mut().enumerate() {
            let hz = LOW_HZ * (HIGH_HZ / LOW_HZ).powf((i as f32 + 0.5) / BANDS as f32);
            *t = 3.0 * (hz / 1000.0).log2();
        }
        let scratch = fft.make_scratch_vec();
        Analyzer { input: fft.make_input_vec(), output: fft.make_output_vec(), fft, window, ring: vec![0.0; N], scratch, edges, tilt }
    }

    /// Forgets what it heard (after a silence, so a new sound doesn't start with the old one).
    pub fn clear(&mut self) {
        self.ring.fill(0.0);
    }

    /// Takes 10 ms (480 samples) of mono audio and returns how strong each band is, 0 to 1.
    pub fn feed(&mut self, block: &[f32]) -> [f32; BANDS] {
        let n = block.len().min(BLOCK);
        self.ring.copy_within(n.., 0);
        let keep = N - n;
        self.ring[keep..].copy_from_slice(&block[..n]);
        for ((x, s), w) in self.input.iter_mut().zip(&self.ring).zip(&self.window) {
            *x = s * w;
        }
        let mut out = [0f32; BANDS];
        if self.fft.process_with_scratch(&mut self.input, &mut self.output, &mut self.scratch).is_err() {
            return out;
        }
        // A full-scale sine through a Hann window peaks at N/4: that's 0 dB.
        let full = (N as f32 / 4.0).powi(2);
        for (b, o) in out.iter_mut().enumerate() {
            let (a, z) = (self.edges[b], self.edges[b + 1].min(self.output.len()));
            let power: f32 = self.output[a.min(z)..z].iter().map(|c| c.norm_sqr()).sum();
            let db = 10.0 * (power / full).max(1e-12).log10() + self.tilt[b];
            *o = ((db - FLOOR_DB) / (TOP_DB - FLOOR_DB)).clamp(0.0, 1.0);
        }
        out
    }
}

/// Stereo (interleaved) or mono samples to mono, into `out`.
pub fn mono(samples: &[f32], channels: usize, out: &mut [f32]) {
    if channels == 2 {
        for (o, pair) in out.iter_mut().zip(samples.chunks_exact(2)) {
            *o = 0.5 * (pair[0] + pair[1]);
        }
    } else {
        let n = out.len().min(samples.len());
        out[..n].copy_from_slice(&samples[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, amp: f32, at: usize) -> Vec<f32> {
        (0..BLOCK).map(|i| amp * (std::f32::consts::TAU * hz * (at + i) as f32 / RATE).sin()).collect()
    }

    #[test]
    fn a_low_note_lights_low_bands_and_a_high_one_high_bands() {
        let mut a = Analyzer::new();
        let mut low = [0f32; BANDS];
        for k in 0..4 {
            low = a.feed(&tone(150.0, 0.3, k * BLOCK));
        }
        let peak = |b: &[f32; BANDS]| b.iter().enumerate().max_by(|x, y| x.1.total_cmp(y.1)).map(|(i, _)| i).unwrap();
        assert!(peak(&low) <= 2, "150 Hz peaks low: {low:?}");
        assert!(low[peak(&low)] > 0.5);
        let mut b = Analyzer::new();
        let mut high = [0f32; BANDS];
        for k in 0..4 {
            high = b.feed(&tone(5000.0, 0.3, k * BLOCK));
        }
        assert!(peak(&high) >= 11, "5 kHz peaks high: {high:?}");
        // Silence is dark.
        let mut c = Analyzer::new();
        let quiet = c.feed(&[0.0; BLOCK]);
        assert!(quiet.iter().all(|v| *v == 0.0));
    }
}
