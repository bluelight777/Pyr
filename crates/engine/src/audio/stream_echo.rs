//! Takes Pyr's own sound back out of a screen share's sound.
//!
//! A screen share's sound is everything the PC plays except Pyr (WASAPI process loopback,
//! "exclude" mode, see `stream.rs`). But when the PC's sound goes through a program that plays
//! it again from its own process (Voicemeeter, SteelSeries Sonar, Elgato Wave Link, NVIDIA
//! Broadcast's speaker effects, "Listen to this device"), Windows can't tell Pyr's part from
//! the rest, and the call goes out again in the stream: everyone watching hears themselves a
//! moment after they speak.
//!
//! So the sharer's Pyr compares what it played (the call) with the stream's sound:
//! 1. Is the call in there at all? Cross-correlation with the phase transform (GCC-PHAT, at
//!    16 kHz) finds a delayed copy of the call even under loud game sound, and how late it is.
//!    Until it's found, the stream's sound goes out exactly as captured.
//! 2. Once found, the way it comes back (its delay and how the other program colors it) is
//!    learnt from cross-spectra averaged over many seconds of people talking; game sound has
//!    nothing to do with the call, so it averages away. The learnt impulse response is cleaned
//!    of estimation noise (taps far out are noise only, which says how much to expect).
//! 3. The call, filtered that way, is subtracted, and nothing else is done to the sound: no
//!    suppression or ducking, so the game sounds the same (an echo canceller made for voice,
//!    like WebRTC's, pumps music and dulls it). It only subtracts while that makes the stream
//!    quieter, so a wrong guess can't add anything.
//!
//! Measured on music with three voices leaking in (`examples/stream_echo_eval.rs`): the call
//! comes out 31-37 dB quieter after a few seconds of talking (about 20 dB if the other program
//! compresses it), and the game sound is left within -36 dB of what it was.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

/// 10 ms of 48 kHz stereo, interleaved: what `clean` takes.
pub const FRAME: usize = 960;
/// One channel's samples in a frame.
const B: usize = FRAME / 2;
const RATE: usize = 48_000;

// Finding the call in the stream (mono, decimated to 16 kHz).
const DEC: usize = 3;
/// Stream sound compared each time (256 ms).
const NY: usize = 4096;
/// How far back in what was played to look (512 ms).
const MAX_LAG: usize = 8192;
const NFFT_DET: usize = 16_384;
/// Delays searched: up to 450 ms (the far edge of the window makes false peaks).
const SEARCH_LAG: usize = 7200;
/// How much of the last look's cross-spectrum is kept each time (about 1.5 s of memory).
const DET_KEEP: f32 = 0.85;
/// How far the peak must stand above the rest to count as the call (measured: a leak 20 dB
/// under the game scores 50 to 110 and louder ones 150 to 300; no leak stays under 35).
const PROMINENT: f32 = 50.0;
/// The low-pass for decimating to 16 kHz (31 taps, cut-off 7 kHz).
const DEC_TAPS: usize = 31;

// Learning how the call comes back (per channel, 48 kHz).
const F: usize = 2048;
const HOP: usize = 512;
/// Cross-spectra are averaged over about this many seconds (the way back doesn't change).
const AVERAGE_S: f32 = 30.0;
/// The learnt impulse response starts this far (2 ms) before the delay found.
const PRE: usize = 96;
/// And is this long (30 ms): enough for equalizers and resamplers.
const TAPS: usize = 1440;
const PARTS: usize = TAPS / B;
/// Taps this far out are beyond any way back: they show the estimate's noise.
const NOISE_TAPS: std::ops::Range<usize> = 1600..1980;
const UPDATE_EVERY: usize = RATE / 2;
/// Played sound counts as playing above this mean square (about -60 dBFS).
const PLAYING: f32 = 1e-6;

/// Played sound kept per channel (1.36 s; the detector looks 0.77 s back at 48 kHz).
const HISTORY: usize = 1 << 16;
/// Stream sound kept per channel.
const CAPTURED: usize = 1 << 14;
/// When the played sound gets further ahead of the stream's than this, the stream missed
/// some (Windows sends nothing while nothing plays): start over from where they are.
const MAX_AHEAD: i64 = RATE as i64 / 2;
/// Or further behind than this (nothing has been played for a while).
const MAX_BEHIND: i64 = RATE as i64 / 4;

pub struct StreamEcho {
    fft: Fft,
    /// What Pyr played, by index (`played_total` counts samples per channel).
    played: [Vec<f32>; 2],
    played_total: i64,
    /// The stream's sound.
    captured: [Vec<f32>; 2],
    captured_total: i64,
    /// The played index that stream sample 0 lines up with, and the first stream sample
    /// that does (since the last time they lost step).
    base: Option<i64>,
    since: i64,
    det: Detector,
    next_look: i64,
    /// The delay the call comes back with, in played samples, once found.
    lag: Option<usize>,
    est: [Estimator; 2],
    /// The stream sample the next learning frame starts at.
    next_learn: i64,
    fir: [Fir; 2],
    /// The played sample the filters expect next.
    fir_next: i64,
    since_update: usize,
    guard: Guard,
    scratch: Vec<f32>,
}

impl Default for StreamEcho {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamEcho {
    pub fn new() -> StreamEcho {
        let fft = Fft::new();
        StreamEcho {
            det: Detector::new(),
            est: [Estimator::new(), Estimator::new()],
            fir: [Fir::new(), Fir::new()],
            fft,
            played: [vec![0.0; HISTORY], vec![0.0; HISTORY]],
            played_total: 0,
            captured: [vec![0.0; CAPTURED], vec![0.0; CAPTURED]],
            captured_total: 0,
            base: None,
            since: 0,
            next_look: 0,
            lag: None,
            next_learn: 0,
            fir_next: 0,
            since_update: 0,
            guard: Guard::default(),
            scratch: vec![0.0; B],
        }
    }

    /// Pyr's own sound, as it's played (48 kHz stereo, interleaved).
    pub fn played(&mut self, x: impl IntoIterator<Item = f32>) {
        let mut it = x.into_iter();
        while let (Some(l), Some(r)) = (it.next(), it.next()) {
            let i = ring(self.played_total, HISTORY);
            self.played[0][i] = l;
            self.played[1][i] = r;
            self.played_total += 1;
        }
    }

    /// How late the call comes back in the stream (ms), once it's been found there (and is
    /// being taken out).
    pub fn found(&self) -> Option<f32> {
        self.lag.map(|l| l as f32 / RATE as f32 * 1000.0)
    }

    /// 10 ms of the screen share's sound (`FRAME` samples, interleaved stereo), with Pyr's
    /// own sound taken out, in place.
    pub fn clean(&mut self, frame: &mut [f32]) {
        debug_assert_eq!(frame.len(), FRAME);
        if self.played_total == 0 {
            // Nothing played yet: nothing to take out.
            return;
        }
        // Line the two up: stream sample k goes with played sample base + k (this frame ends
        // where the played sound so far does, when they're in step).
        let ahead = self.base.map_or(0, |base| self.played_total - (base + self.captured_total + B as i64));
        if self.base.is_none() || !(-MAX_BEHIND..=MAX_AHEAD).contains(&ahead) {
            self.restart();
        }
        let base = self.base.unwrap_or(0);
        let k = self.captured_total;
        for i in 0..B {
            let j = ring(k + i as i64, CAPTURED);
            self.captured[0][j] = frame[2 * i];
            self.captured[1][j] = frame[2 * i + 1];
        }
        self.captured_total += B as i64;
        let now = base + self.captured_total;
        // 1. Look for the call in the stream every 256 ms.
        if self.captured_total >= self.next_look
            && self.captured_total - self.since >= (NY * DEC + DEC_TAPS) as i64
            && now >= ((NY + MAX_LAG) * DEC + DEC_TAPS) as i64
            && now <= self.played_total
        {
            self.next_look = self.captured_total + (NY * DEC) as i64;
            if let Some(lag) = self.det.look(&self.fft, &self.played, now, &self.captured, self.captured_total) {
                // A different delay (by over 1 ms) means a different way back: learn it anew.
                if self.lag.is_none_or(|l| l.abs_diff(lag) > DEC * 16) {
                    tracing::info!(delay_ms = lag as f32 / 48.0, "screen share sound has the call in it: taking it out");
                    self.relearn(lag);
                }
            }
        }
        let Some(lag) = self.lag else { return };
        let d = lag.saturating_sub(PRE) as i64;

        // 2. Learn how it comes back, from the stream so far (frames of `F`, every `HOP`).
        while self.next_learn + F as i64 <= self.captured_total {
            let c0 = self.next_learn;
            let r0 = base + c0 - d;
            if r0 + F as i64 > self.played_total {
                break;
            }
            if r0 > self.played_total - HISTORY as i64 && r0 >= 0 {
                for ch in 0..2 {
                    self.est[ch].learn(&self.fft, &self.played[ch], r0, &self.captured[ch], c0);
                }
            }
            self.next_learn += HOP as i64;
        }
        self.since_update += B;
        if self.since_update >= UPDATE_EVERY {
            self.since_update = 0;
            for ch in 0..2 {
                if let Some(h) = self.est[ch].response(&self.fft) {
                    self.fir[ch].set(&self.fft, &h);
                }
            }
        }

        // 3. Take it out. The played sound this needs must be there (it's played well before
        // it comes back) and not yet overwritten.
        let start = base + k - d;
        if start < 0 || start + B as i64 > self.played_total || start <= self.played_total - HISTORY as i64 {
            return;
        }
        if start != self.fir_next {
            for f in &mut self.fir {
                f.resync();
            }
        }
        self.fir_next = start + B as i64;
        let (mut e_in, mut e_out, mut e_echo) = (0f32, 0f32, 0f32);
        let mut echo = [[0f32; B]; 2];
        for (ch, out) in echo.iter_mut().enumerate() {
            for (i, v) in self.scratch.iter_mut().enumerate() {
                *v = self.played[ch][ring(start + i as i64, HISTORY)];
            }
            self.fir[ch].run(&self.fft, &self.scratch, out);
        }
        for i in 0..B {
            for ch in 0..2 {
                let y = frame[2 * i + ch];
                let e = echo[ch][i];
                e_in += y * y;
                e_out += (y - e) * (y - e);
                e_echo += e * e;
            }
        }
        let scale = self.guard.step(e_in, e_out, e_echo);
        let from = self.guard.last;
        if scale > 0.0 || from > 0.0 {
            for i in 0..B {
                // Ramp across the frame, so a change makes no click.
                let s = from + (scale - from) * (i as f32 + 1.0) / B as f32;
                frame[2 * i] -= s * echo[0][i];
                frame[2 * i + 1] -= s * echo[1][i];
            }
        }
        self.guard.last = scale;
    }

    /// The two lost their lining up (the stream missed some, or nothing's been played): start
    /// again from now.
    fn restart(&mut self) {
        self.base = Some(self.played_total - self.captured_total - B as i64);
        self.since = self.captured_total;
        self.det = Detector::new();
        self.next_look = 0;
        if self.lag.take().is_some() {
            tracing::info!("screen share sound and the call lost step; looking for the call again");
        }
        self.reset_learning();
    }

    fn relearn(&mut self, lag: usize) {
        self.lag = Some(lag);
        self.reset_learning();
    }

    fn reset_learning(&mut self) {
        self.est = [Estimator::new(), Estimator::new()];
        self.fir = [Fir::new(), Fir::new()];
        // Learn from what's already here (since the two last lost step).
        self.next_learn = self.since.max(self.captured_total - (4 * F) as i64);
        self.since_update = 0;
        self.guard = Guard::default();
    }
}

/// FFT plans, shared.
struct Fft {
    det_fwd: Arc<dyn RealToComplex<f32>>,
    det_inv: Arc<dyn ComplexToReal<f32>>,
    est_fwd: Arc<dyn RealToComplex<f32>>,
    est_inv: Arc<dyn ComplexToReal<f32>>,
    fir_fwd: Arc<dyn RealToComplex<f32>>,
    fir_inv: Arc<dyn ComplexToReal<f32>>,
}

impl Fft {
    fn new() -> Fft {
        let mut p = RealFftPlanner::<f32>::new();
        Fft {
            det_fwd: p.plan_fft_forward(NFFT_DET),
            det_inv: p.plan_fft_inverse(NFFT_DET),
            est_fwd: p.plan_fft_forward(F),
            est_inv: p.plan_fft_inverse(F),
            fir_fwd: p.plan_fft_forward(2 * B),
            fir_inv: p.plan_fft_inverse(2 * B),
        }
    }
}

/// Index `i` of a ring of `len` (a power of two).
fn ring(i: i64, len: usize) -> usize {
    (i & (len as i64 - 1)) as usize
}

/// An inverse transform (the first and last bins of a real signal's spectrum are real).
fn inverse(plan: &Arc<dyn ComplexToReal<f32>>, spec: &mut [Complex<f32>], out: &mut [f32]) {
    spec[0].im = 0.0;
    let n = spec.len();
    spec[n - 1].im = 0.0;
    let _ = plan.process(spec, out);
}

/// Finds a delayed copy of what was played in the stream: GCC-PHAT on 16 kHz mono.
struct Detector {
    lowpass: [f32; DEC_TAPS],
    acc: Vec<Complex<f32>>,
    y: Vec<f32>,
    r: Vec<f32>,
    ys: Vec<Complex<f32>>,
    rs: Vec<Complex<f32>>,
    cc: Vec<f32>,
    mags: Vec<f32>,
    /// The last look's best delay, if it stood out.
    candidate: Option<usize>,
}

impl Detector {
    fn new() -> Detector {
        // Windowed-sinc low-pass at 7 kHz (Hamming).
        let mut lowpass = [0f32; DEC_TAPS];
        let fc = 7_000.0 / RATE as f32;
        let mid = (DEC_TAPS - 1) as f32 / 2.0;
        let mut sum = 0.0;
        for (t, v) in lowpass.iter_mut().enumerate() {
            let x = t as f32 - mid;
            let s = if x == 0.0 { 2.0 * fc } else { (2.0 * std::f32::consts::PI * fc * x).sin() / (std::f32::consts::PI * x) };
            let w = 0.54 - 0.46 * (2.0 * std::f32::consts::PI * t as f32 / (DEC_TAPS - 1) as f32).cos();
            *v = s * w;
            sum += *v;
        }
        lowpass.iter_mut().for_each(|v| *v /= sum);
        Detector {
            lowpass,
            acc: vec![Complex::default(); NFFT_DET / 2 + 1],
            y: vec![0.0; NFFT_DET],
            r: vec![0.0; NFFT_DET],
            ys: vec![Complex::default(); NFFT_DET / 2 + 1],
            rs: vec![Complex::default(); NFFT_DET / 2 + 1],
            cc: vec![0.0; NFFT_DET],
            mags: Vec::with_capacity(SEARCH_LAG + 1),
            candidate: None,
        }
    }

    /// 16 kHz mono of `n` samples ending (exclusive) at 48 kHz index `end`, from a ring.
    fn decimate(&self, src: &[Vec<f32>; 2], end: i64, n: usize, out: &mut [f32]) {
        let len = src[0].len();
        let first = end - (n * DEC) as i64;
        for (i, o) in out[..n].iter_mut().enumerate() {
            let at = first + (i * DEC) as i64;
            let mut acc = 0.0;
            for (t, h) in self.lowpass.iter().enumerate() {
                let j = ring(at - t as i64, len);
                acc += h * (src[0][j] + src[1][j]);
            }
            *o = 0.5 * acc;
        }
    }

    /// Looks again (`played_end` is the played index lined up with `captured_end`). Returns the
    /// delay in played (48 kHz) samples once the same one has stood out twice in a row.
    fn look(&mut self, fft: &Fft, played: &[Vec<f32>; 2], played_end: i64, captured: &[Vec<f32>; 2], captured_end: i64) -> Option<usize> {
        let mut y = std::mem::take(&mut self.y);
        let mut r = std::mem::take(&mut self.r);
        self.decimate(captured, captured_end, NY, &mut y);
        y[NY..].fill(0.0);
        self.decimate(played, played_end, NY + MAX_LAG, &mut r);
        r[NY + MAX_LAG..].fill(0.0);
        let playing = r[MAX_LAG..NY + MAX_LAG].iter().map(|v| v * v).sum::<f32>() / NY as f32 > PLAYING;
        let mut found = None;
        if playing {
            let _ = fft.det_fwd.process(&mut y, &mut self.ys);
            let _ = fft.det_fwd.process(&mut r, &mut self.rs);
            for ((a, ys), rs) in self.acc.iter_mut().zip(&self.ys).zip(&self.rs) {
                let c = ys * rs.conj();
                let m = c.norm();
                let c = if m > 1e-20 { c / m } else { Complex::default() };
                *a = *a * DET_KEEP + c * (1.0 - DET_KEEP);
            }
            self.ys.copy_from_slice(&self.acc);
            inverse(&fft.det_inv, &mut self.ys, &mut self.cc);
            // Stream sample k lines up with played sample k - lag at index (lag - MAX_LAG) mod N.
            self.mags.clear();
            let (mut best, mut best_v) = (0, f32::MIN);
            for lag in 0..=SEARCH_LAG {
                let v = self.cc[(lag + NFFT_DET - MAX_LAG) % NFFT_DET];
                if v > best_v {
                    best_v = v;
                    best = lag;
                }
                self.mags.push(v.abs());
            }
            let mid = self.mags.len() / 2;
            let median = *self.mags.select_nth_unstable_by(mid, |a, b| a.total_cmp(b)).1;
            let prominence = best_v / median.max(1e-30);
            if prominence > PROMINENT {
                if self.candidate.is_some_and(|c| c.abs_diff(best) <= 16) {
                    found = Some(best * DEC);
                }
                self.candidate = Some(best);
            } else {
                self.candidate = None;
            }
        }
        self.y = y;
        self.r = r;
        found
    }
}

/// Learns one channel's way back: cross-spectra of what was played and what the stream got.
struct Estimator {
    sxx: Vec<f32>,
    syx: Vec<Complex<f32>>,
    window: Vec<f32>,
    x: Vec<f32>,
    y: Vec<f32>,
    xs: Vec<Complex<f32>>,
    ys: Vec<Complex<f32>>,
    frames: u32,
}

impl Estimator {
    fn new() -> Estimator {
        let window = (0..F).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / F as f32).cos()).collect();
        Estimator {
            sxx: vec![0.0; F / 2 + 1],
            syx: vec![Complex::default(); F / 2 + 1],
            window,
            x: vec![0.0; F],
            y: vec![0.0; F],
            xs: vec![Complex::default(); F / 2 + 1],
            ys: vec![Complex::default(); F / 2 + 1],
            frames: 0,
        }
    }

    /// One frame: played sound from `r0`, stream sound from `c0` (both `F` long).
    fn learn(&mut self, fft: &Fft, played: &[f32], r0: i64, captured: &[f32], c0: i64) {
        let mut power = 0.0;
        for i in 0..F {
            let x = played[ring(r0 + i as i64, played.len())];
            power += x * x;
            self.x[i] = x * self.window[i];
            self.y[i] = captured[ring(c0 + i as i64, captured.len())] * self.window[i];
        }
        if power / (F as f32) < PLAYING {
            return;
        }
        let _ = fft.est_fwd.process(&mut self.x, &mut self.xs);
        let _ = fft.est_fwd.process(&mut self.y, &mut self.ys);
        let keep = (-(HOP as f32) / RATE as f32 / AVERAGE_S).exp();
        for ((sxx, syx), (x, y)) in self.sxx.iter_mut().zip(self.syx.iter_mut()).zip(self.xs.iter().zip(&self.ys)) {
            *sxx = *sxx * keep + x.norm_sqr();
            *syx = *syx * keep + y * x.conj();
        }
        self.frames += 1;
    }

    /// The impulse response learnt so far, cleaned of noise (`TAPS` long).
    fn response(&mut self, fft: &Fft) -> Option<Vec<f32>> {
        if self.frames < 4 {
            return None;
        }
        let mean = self.sxx.iter().sum::<f32>() / self.sxx.len() as f32;
        let reg = 1e-3 * mean.max(1e-30);
        for ((h, sxx), syx) in self.xs.iter_mut().zip(&self.sxx).zip(&self.syx) {
            *h = syx / (sxx + reg);
        }
        let mut h = vec![0f32; F];
        inverse(&fft.est_inv, &mut self.xs, &mut h);
        h.iter_mut().for_each(|v| *v /= F as f32);
        // Wiener-like shrinking of each tap against the noise level seen far out.
        let noise = h[NOISE_TAPS].iter().map(|v| v * v).sum::<f32>() / NOISE_TAPS.len() as f32 + 1e-30;
        let power: Vec<f32> = h.iter().map(|v| v * v).collect();
        let mut out = vec![0f32; TAPS];
        let mut run: f32 = power[..8].iter().sum();
        for (i, o) in out.iter_mut().enumerate() {
            // A 17-tap moving average of the power around tap i.
            if i + 8 < F {
                run += power[i + 8];
            }
            if i >= 9 {
                run -= power[i - 9];
            }
            let p = run / 17.0;
            let g = (1.0 - 2.0 * noise / p.max(1e-30)).clamp(0.0, 1.0);
            *o = h[i] * g;
        }
        // Fade out the last 3 ms.
        let fade = 144;
        for i in 0..fade {
            out[TAPS - fade + i] *= 0.5 + 0.5 * (std::f32::consts::PI * (i as f32 + 0.5) / fade as f32).cos();
        }
        Some(out)
    }
}

/// One channel's echo, from what was played: partitioned overlap-save convolution with the
/// learnt response, 10 ms at a time. A new response fades in over one frame.
struct Fir {
    parts: Vec<Vec<Complex<f32>>>,
    next: Option<Vec<Vec<Complex<f32>>>>,
    /// Spectra of the last `PARTS` blocks of played sound, newest first.
    history: Vec<Vec<Complex<f32>>>,
    prev: Vec<f32>,
    buf: Vec<f32>,
    acc: Vec<Complex<f32>>,
    out: Vec<f32>,
}

impl Fir {
    fn new() -> Fir {
        let bins = B + 1;
        Fir {
            parts: vec![vec![Complex::default(); bins]; PARTS],
            next: None,
            history: vec![vec![Complex::default(); bins]; PARTS],
            prev: vec![0.0; B],
            buf: vec![0.0; 2 * B],
            acc: vec![Complex::default(); bins],
            out: vec![0.0; 2 * B],
        }
    }

    /// The played sound skipped: forget the blocks before.
    fn resync(&mut self) {
        self.prev.fill(0.0);
        for h in &mut self.history {
            h.fill(Complex::default());
        }
    }

    fn set(&mut self, fft: &Fft, h: &[f32]) {
        let mut parts = Vec::with_capacity(PARTS);
        for p in 0..PARTS {
            self.buf[..B].copy_from_slice(&h[p * B..(p + 1) * B]);
            self.buf[B..].fill(0.0);
            let mut spec = vec![Complex::default(); B + 1];
            let _ = fft.fir_fwd.process(&mut self.buf, &mut spec);
            parts.push(spec);
        }
        self.next = Some(parts);
    }

    fn filter(&mut self, fft: &Fft, parts: &[Vec<Complex<f32>>], out: &mut [f32; B]) {
        self.acc.fill(Complex::default());
        for (h, x) in parts.iter().zip(&self.history) {
            for ((a, h), x) in self.acc.iter_mut().zip(h).zip(x) {
                *a += h * x;
            }
        }
        inverse(&fft.fir_inv, &mut self.acc, &mut self.out);
        let n = (2 * B) as f32;
        for (o, v) in out.iter_mut().zip(&self.out[B..]) {
            *o = v / n;
        }
    }

    fn run(&mut self, fft: &Fft, x: &[f32], out: &mut [f32; B]) {
        self.buf[..B].copy_from_slice(&self.prev);
        self.buf[B..].copy_from_slice(x);
        self.prev.copy_from_slice(x);
        self.history.rotate_right(1);
        let _ = fft.fir_fwd.process(&mut self.buf, &mut self.history[0]);
        let parts = std::mem::take(&mut self.parts);
        self.filter(fft, &parts, out);
        if let Some(next) = self.next.take() {
            let mut new = [0f32; B];
            self.filter(fft, &next, &mut new);
            for (i, (o, n)) in out.iter_mut().zip(new).enumerate() {
                let t = (i as f32 + 0.5) / B as f32;
                *o = *o * (1.0 - t) + n * t;
            }
            self.parts = next;
        } else {
            self.parts = parts;
        }
    }
}

/// Only subtract while it makes the stream quieter (over the last few seconds).
struct Guard {
    before: f32,
    after: f32,
    on: bool,
    last: f32,
}

impl Default for Guard {
    fn default() -> Self {
        Guard { before: 0.0, after: 0.0, on: true, last: 0.0 }
    }
}

impl Guard {
    /// How much of the echo estimate to subtract this frame.
    fn step(&mut self, e_in: f32, e_out: f32, e_echo: f32) -> f32 {
        if e_echo > 1e-9 {
            let keep = (-0.01f32 / 4.0).exp();
            self.before = self.before * keep + e_in;
            self.after = self.after * keep + e_out;
            self.on = self.after <= self.before;
        }
        let target = if self.on { 1.0 } else { 0.0 };
        self.last + (target - self.last) * 0.1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic noise.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
        }
    }

    /// `secs` of a call (bursts of colored noise, like people taking turns) and of game sound
    /// (steady noise and a chord, different in each ear), 48 kHz stereo.
    fn sounds(secs: usize) -> (Vec<f32>, Vec<f32>) {
        let n = secs * RATE;
        let (mut a, mut b) = (Rng(0x1234_5678), Rng(0x9abc_def1));
        let (mut call, mut game) = (Vec::with_capacity(2 * n), Vec::with_capacity(2 * n));
        let (mut lp, mut gl, mut gr) = (0f32, 0f32, 0f32);
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            let talking = (t * 0.7).fract() < 0.6;
            lp += 0.3 * (a.next() - lp);
            let v = if talking { 0.25 * lp * (1.0 + (t * 23.0).sin()) } else { 0.0 };
            call.extend([v, v]);
            gl += 0.05 * (b.next() - gl);
            gr += 0.05 * (b.next() - gr);
            let chord = 0.08 * ((t * 220.0 * std::f32::consts::TAU).sin() + (t * 277.2 * std::f32::consts::TAU).sin());
            game.extend([0.4 * gl + chord, 0.4 * gr + 0.7 * chord]);
        }
        (call, game)
    }

    /// Runs `secs` of stream sound through, with the call coming back `delay` samples late at
    /// `gain`; returns (stream as captured, as cleaned, the echo alone).
    fn run(secs: usize, delay: usize, gain: f32) -> (Vec<f32>, Vec<f32>, Vec<f32>, StreamEcho) {
        let (call, game) = sounds(secs);
        let echo: Vec<f32> = (0..call.len()).map(|i| if i >= 2 * delay { gain * call[i - 2 * delay] } else { 0.0 }).collect();
        let captured: Vec<f32> = game.iter().zip(&echo).map(|(g, e)| g + e).collect();
        let mut ec = StreamEcho::new();
        let mut out = captured.clone();
        for (f, chunk) in out.chunks_exact_mut(FRAME).enumerate() {
            ec.played(call[f * FRAME..(f + 1) * FRAME].iter().copied());
            ec.clean(chunk);
        }
        (captured, out, echo, ec)
    }

    fn energy(x: &[f32]) -> f64 {
        x.iter().map(|v| (*v as f64) * (*v as f64)).sum()
    }

    #[test]
    fn the_call_is_found_and_taken_out_of_the_stream() {
        let (captured, out, echo, ec) = run(16, 40 * 48, 0.8);
        let found = ec.found().expect("the call wasn't found in the stream");
        assert!((found - 40.0).abs() < 1.0, "found it {found} ms late, not 40");
        // From 8 s on: the echo left, against the echo that leaked in.
        let from = 8 * RATE * 2;
        let left: Vec<f32> = out[from..].iter().zip(&captured[from..]).zip(&echo[from..]).map(|((o, c), e)| o - (c - e)).collect();
        let ratio = 10.0 * (energy(&left) / energy(&echo[from..])).log10();
        assert!(ratio < -20.0, "only {ratio:.1} dB of the call taken out");
    }

    #[test]
    fn a_stream_without_the_call_goes_out_untouched() {
        let (captured, out, _, ec) = run(12, 0, 0.0);
        assert!(ec.found().is_none());
        assert!(captured == out, "the stream's sound was changed");
    }

    #[test]
    fn missing_sound_on_either_side_does_no_harm() {
        let (call, game) = sounds(6);
        let mut ec = StreamEcho::new();
        let mut frame = vec![0f32; FRAME];
        // Nothing played yet; then played sound with no stream for a second; then a burst of
        // played sound only; then both again.
        for f in 0..600 {
            if !(100..200).contains(&f) {
                frame.copy_from_slice(&game[f * FRAME..(f + 1) * FRAME]);
                let before = frame.clone();
                ec.clean(&mut frame);
                if ec.found().is_none() {
                    assert_eq!(frame, before);
                }
            }
            if f >= 50 {
                ec.played(call[f * FRAME..(f + 1) * FRAME].iter().copied());
            }
            if f == 300 {
                ec.played(call[..FRAME * 60].iter().copied());
            }
        }
        assert!(frame.iter().all(|v| v.is_finite()));
    }
}
