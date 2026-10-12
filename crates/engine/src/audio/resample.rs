//! A small windowed-sinc resampler for converting between device rates and 48 kHz.
//! Works on one channel; create one per channel.

const HALF_TAPS: usize = 16;
const PHASES: usize = 256;

pub struct Resampler {
    step: f64,
    table: Vec<f32>,
    hist: Vec<f32>,
    pos: f64,
    passthrough: bool,
}

fn sinc(x: f64) -> f64 {
    if x.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x) }
}

impl Resampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Resampler {
        let passthrough = in_rate == out_rate;
        let step = in_rate as f64 / out_rate as f64;
        // Low-pass at the lower Nyquist frequency (with a little guard band).
        let cutoff = (out_rate as f64 / in_rate as f64).min(1.0) * 0.95;
        let taps = 2 * HALF_TAPS;
        let mut table = vec![0f32; (PHASES + 1) * taps];
        for p in 0..=PHASES {
            let frac = p as f64 / PHASES as f64;
            let mut sum = 0.0;
            for k in 0..taps {
                // tap k corresponds to input offset (k - HALF_TAPS + 1) relative to floor(pos)
                let x = (k as f64 - (HALF_TAPS as f64 - 1.0)) - frac;
                let w = 0.5 + 0.5 * (std::f64::consts::PI * x / (HALF_TAPS as f64 + 0.5)).cos(); // Hann
                let v = cutoff * sinc(cutoff * x) * if x.abs() <= HALF_TAPS as f64 + 0.5 { w } else { 0.0 };
                table[p * taps + k] = v as f32;
                sum += v;
            }
            // Normalise each phase for unity DC gain.
            for k in 0..taps {
                table[p * taps + k] /= sum as f32;
            }
        }
        Resampler { step, table, hist: vec![0.0; HALF_TAPS], pos: (HALF_TAPS - 1) as f64, passthrough }
    }

    /// Resample `input`, appending output samples to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.passthrough {
            out.extend_from_slice(input);
            return;
        }
        self.hist.extend_from_slice(input);
        let taps = 2 * HALF_TAPS;
        loop {
            let i0 = self.pos.floor() as usize;
            if i0 + HALF_TAPS >= self.hist.len() {
                break;
            }
            let frac = self.pos - i0 as f64;
            let ph = frac * PHASES as f64;
            let p0 = ph.floor() as usize;
            let t = (ph - p0 as f64) as f32;
            let a = &self.table[p0 * taps..p0 * taps + taps];
            let b = &self.table[(p0 + 1) * taps..(p0 + 1) * taps + taps];
            let start = i0 + 1 - HALF_TAPS;
            let src = &self.hist[start..start + taps];
            let mut acc = 0f32;
            for k in 0..taps {
                acc += src[k] * (a[k] + (b[k] - a[k]) * t);
            }
            out.push(acc);
            self.pos += self.step;
        }
        // Drop history we no longer need.
        let keep_from = (self.pos.floor() as usize).saturating_sub(HALF_TAPS - 1);
        if keep_from > 0 {
            self.hist.drain(..keep_from);
            self.pos -= keep_from as f64;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, hz: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (i as f32 / rate as f32 * hz * std::f32::consts::TAU).sin() * 0.5).collect()
    }

    fn dominant_freq(x: &[f32], rate: u32) -> f32 {
        // zero-crossing estimate is plenty for a pure tone
        let crossings = x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        crossings as f32 * rate as f32 / x.len() as f32
    }

    #[test]
    fn converts_44k_to_48k_and_back() {
        let input = tone(44_100, 1000.0, 44_100);
        let mut up = Resampler::new(44_100, 48_000);
        let mut out = Vec::new();
        for chunk in input.chunks(441) {
            up.process(chunk, &mut out);
        }
        assert!((out.len() as i64 - 48_000).abs() < 64, "len {}", out.len());
        let f = dominant_freq(&out[1000..], 48_000);
        assert!((f - 1000.0).abs() < 5.0, "freq {f}");
        let rms = (out[1000..].iter().map(|v| v * v).sum::<f32>() / (out.len() - 1000) as f32).sqrt();
        assert!((rms - 0.3536).abs() < 0.01, "rms {rms}");

        let mut down = Resampler::new(48_000, 44_100);
        let mut back = Vec::new();
        down.process(&out, &mut back);
        let f = dominant_freq(&back[1000..], 44_100);
        assert!((f - 1000.0).abs() < 5.0, "freq {f}");
    }

    #[test]
    fn passthrough() {
        let mut r = Resampler::new(48_000, 48_000);
        let mut out = Vec::new();
        r.process(&[1.0, 2.0], &mut out);
        assert_eq!(out, vec![1.0, 2.0]);
    }
}
