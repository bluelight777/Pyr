//! The poker table's sounds as 0.13.6 made them: chips that ring, cards that slide and land on the
//! felt, a knock on the rail for a check, the pot pushed over, and a better win. Made fresh every
//! time one plays (a little different each time, as 0.13.28's are), from what 0.13.28's sound like:
//! recorded off it at a table of bots and measured (`rebuild/measured/tablesounds.md`).
//!
//! Each is mono at 48 kHz; `poker_sfx` places it to a side and sets how loud.

use std::f32::consts::TAU;

const RATE: f32 = 48_000.0;

fn ms(n: f32) -> usize {
    (n.max(0.0) / 1000.0 * RATE) as usize
}

/// A small random generator (xorshift64*): each sound gets its own from a seed.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        // (Spread the seed's bits; never zero.)
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Rng((z ^ (z >> 31)) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// From 0 to 1.
    pub fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// From `a` to `b`.
    pub fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f()
    }

    /// White noise, -1 to 1.
    fn noise(&mut self) -> f32 {
        self.f() * 2.0 - 1.0
    }
}

fn add(o: &mut [f32], i: usize, x: f32) {
    if let Some(s) = o.get_mut(i) {
        *s += x;
    }
}

/// A two-pole filter (RBJ's biquads), run sample by sample.
#[derive(Clone, Copy)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    fn new(b: [f32; 3], a: [f32; 3]) -> Biquad {
        Biquad { b0: b[0] / a[0], b1: b[1] / a[0], b2: b[2] / a[0], a1: a[1] / a[0], a2: a[2] / a[0], x1: 0.0, x2: 0.0, y1: 0.0, y2: 0.0 }
    }

    fn lowpass(f: f32, q: f32) -> Biquad {
        let w = TAU * f / RATE;
        let (c, al) = (w.cos(), w.sin() / (2.0 * q));
        Biquad::new([(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0], [1.0 + al, -2.0 * c, 1.0 - al])
    }

    fn highpass(f: f32, q: f32) -> Biquad {
        let w = TAU * f / RATE;
        let (c, al) = (w.cos(), w.sin() / (2.0 * q));
        Biquad::new([(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0], [1.0 + al, -2.0 * c, 1.0 - al])
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// A clay chip striking another: a sharp tick and its ring, four partials (1, 2.234, 3.468 and
/// 5.07 times the lowest, which is 2,050 to 2,310 Hz) dying fast and then ringing on softly, the
/// clay's rattle and knock under them.
fn chip(o: &mut [f32], at: usize, f1: f32, amp: f32, rng: &mut Rng) {
    chip_with(o, at, f1, amp, 1.0, [1.0; 4], 0.3, rng);
}

/// A chip with `under` of the rattle and the knock under its ring, and its partials ringing on
/// `tail` times as loud (the pot's stacks, as 0.13.28's: nothing between their partials, 30 to 45
/// dB under them, from 1 ms after the tick, and the upper partials ringing on longer); `long`: how
/// much of the rattle's longer part (a bet's 0.3, the all-in's pour none).
fn chip_with(o: &mut [f32], at: usize, f1: f32, amp: f32, under: f32, tail: [f32; 4], long: f32, rng: &mut Rng) {
    const PARTS: [(f32, f32, f32, f32, f32); 4] = [
        // (ratio, amplitude, fast decay s, tail amplitude, tail decay s)
        (1.0, 1.0, 0.0092, 0.035, 0.045),
        (2.234, 0.95, 0.0054, 0.022, 0.032),
        (3.468, 0.78, 0.0036, 0.012, 0.022),
        (5.07, 0.55, 0.0021, 0.005, 0.020),
    ];
    let len = ms(330.0);
    for ((r, a, d1, a2, d2), k) in PARTS.into_iter().zip(tail) {
        let a2 = a2 * k;
        let f = f1 * r;
        let ph = rng.range(0.0, TAU);
        for i in 0..len {
            let t = i as f32 / RATE;
            let env = a * (-t / d1).exp() + a2 * (-t / d2).exp();
            if env < 1e-5 {
                break;
            }
            add(o, at + i, (TAU * f * t + ph).sin() * env * amp * 0.25);
        }
    }
    // The tick itself, and the clay's faint rattle under the ring (about 50 dB down, gone in a
    // tenth of a second).
    let mut hp = Biquad::highpass(5000.0, 0.7);
    let mut hp2 = Biquad::highpass(1000.0, 0.7);
    let mut lp2 = Biquad::lowpass(RATTLE.1, 0.5);
    let mut body = Biquad::lowpass(1200.0, 0.6);
    for i in 0..ms(300.0) {
        let t = i as f32 / RATE;
        let tick = if i < ms(0.6) {
            let e = 1.0 - i as f32 / ms(0.6) as f32;
            hp.run(rng.noise()) * e * 0.45
        } else {
            0.0
        };
        let rattle = lp2.run(hp2.run(rng.noise())) * ((-t / 0.028).exp() + long * (-t / 0.07).exp()) * RATTLE.0;
        // (And the knock of it, low down, through the table.)
        let thud = body.run(rng.noise()) * (-t / 0.012).exp() * 0.05;
        add(o, at + i, (tick + (rattle + thud) * under) * amp);
    }
}

/// How loud the rattle under a chip's ring is, and how high it goes (Hz): bright, as 0.13.28's
/// (its top third-octaves, 16 and 20 kHz, only 8 dB under its 4 kHz one a few ms on).
const RATTLE: (f32, f32) = (0.015, 20_000.0);

/// How loud the felt under a stack of chips is, against its clicks.
const CHIP_FELT: f32 = 0.16;

/// Something set down on the felt (a card, a stack of chips), as 0.13.28's land: the low thump
/// (about 100 Hz) dying in 17 ms, and the cloth's body under it, broad from 150 to 600 Hz and
/// dying a little faster (measured in sixth-octaves over each 10 ms of 0.13.28's, under its turned
/// cards and its chips alike).
fn felt(o: &mut [f32], at: usize, amp: f32, rng: &mut Rng) {
    let f = rng.range(92.0, 104.0);
    let mut body = Biquad::lowpass(FELT.2, FELT.3);
    for i in 0..ms(300.0) {
        let t = i as f32 / RATE;
        let attack = (t / 0.002).min(1.0);
        let tone = (TAU * f * t).sin() * (-t / FELT.0).exp();
        let cloth = body.run(rng.noise()) * (-t / FELT.1).exp() * FELT.4;
        add(o, at + i, (tone + cloth) * attack * amp);
    }
}

/// The felt: the thump's and the body's time constants (s), the body's low-pass (Hz, Q) and how
/// loud it is against the thump.
const FELT: (f32, f32, f32, f32, f32) = (0.017, 0.015, 350.0, 0.5, 10.0);

/// A resonance as 0.13.28's cards sound through: two poles at `f0` (`f0 / q` wide) and a zero at
/// DC only, so -6 dB an octave either side of the peak with the top end kept (no zero at
/// Nyquist, as a band-pass biquad has: 0.13.28's slides hold up to 24 kHz). 0 dB at the peak.
#[derive(Clone, Copy, Default)]
struct Reso {
    a1: f32,
    a2: f32,
    g: f32,
    x1: f32,
    y1: f32,
    y2: f32,
}

impl Reso {
    fn new(f0: f32, q: f32) -> Reso {
        let mut r = Reso::default();
        r.tune(f0, q);
        r
    }

    /// Retuned in place (its state kept), for a sweep.
    fn tune(&mut self, f0: f32, q: f32) {
        let w = TAU * f0 / RATE;
        let r = (-std::f32::consts::PI * (f0 / q) / RATE).exp();
        self.a1 = 2.0 * r * w.cos();
        self.a2 = -r * r;
        // |1 - e^-jw| over |1 - a1 e^-jw - a2 e^-2jw| at the peak, divided out.
        let num = ((1.0 - w.cos()).powi(2) + w.sin().powi(2)).sqrt();
        let (re, im) = (1.0 - self.a1 * w.cos() - self.a2 * (2.0 * w).cos(), self.a1 * w.sin() + self.a2 * (2.0 * w).sin());
        self.g = (re * re + im * im).sqrt() / num;
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = (x - self.x1) + self.a1 * self.y1 + self.a2 * self.y2;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y * self.g
    }
}

/// A card (or the cards) sliding over the cloth as 0.13.28 makes it: noise through a resonance
/// whose peak moves from `f1` to `f2` Hz over the slide (a geometric sweep: up as a dealt card
/// speeds off the shoe, down as folded cards slow), `q` wide; rising over `attack` ms, holding
/// `hold` ms, then dying away (`tau` ms), the last `fade` of it faded out. (Fitted to 0.13.28's
/// mean spectrogram of each, band by band in 5 ms steps: `measured/cardsounds.md`.)
#[allow(clippy::too_many_arguments)]
fn card_slide(o: &mut [f32], at: usize, len: usize, (attack, hold, tau, fade): (f32, f32, f32, f32), amp: f32, (f1, f2, q): (f32, f32, f32), rng: &mut Rng) {
    let mut res = Reso::new(f1, q);
    let len_ms = len as f32 / RATE * 1000.0;
    let fl = (fade * len_ms).max(0.05);
    for i in 0..len {
        let t = i as f32 / RATE * 1000.0;
        if i % 16 == 0 {
            res.tune(f1 * (f2 / f1).powf(i as f32 / len as f32), q);
        }
        let env = (t / attack.max(0.1)).min(1.0) * (-(t - attack - hold).max(0.0) / tau.max(1.0)).exp();
        let end = if t > len_ms - fl { 0.5 - 0.5 * (std::f32::consts::PI * ((len_ms - t) / fl).clamp(0.0, 1.0)).cos() } else { 1.0 };
        add(o, at + i, res.run(rng.noise()) * env * end * amp);
    }
}

/// A card turned over, its edge snapping down on the cloth as 0.13.28 makes it: a burst of noise
/// dying in about a millisecond and a half, rung through the card's three resonances (1.73, 3.18
/// and 5.04 kHz, a seventh to a ninth as wide as each: where 0.13.28's click peaks) over a bright
/// floor; within 2 dB of 0.13.28's in every sixth-octave from 600 Hz up, its first 5 ms.
fn card_click(o: &mut [f32], at: usize, amp: f32, rng: &mut Rng) {
    let mut modes = [Reso::new(1730.0, 7.0), Reso::new(3175.0, 9.0), Reso::new(5040.0, 9.0)];
    const GAINS: [f32; 3] = [CLICK.0, CLICK.1, CLICK.2];
    // (All of it thinning out under 1.2 kHz, as 0.13.28's: a one-pole high-pass.)
    let k = (-TAU * 1200.0 / RATE).exp();
    let mut low = 0.0f32;
    for i in 0..ms(30.0) {
        let t = i as f32 / RATE;
        let n = rng.noise();
        let kick = n * (-t / CLICK.4).exp();
        let mut x = n * (-t / CLICK.5).exp() * CLICK.3;
        for (m, g) in modes.iter_mut().zip(GAINS) {
            x += m.run(kick) * g;
        }
        low = x + k * (low - x);
        add(o, at + i, (x - low) * amp);
    }
}

/// The card's click: its resonances' gains (1.78, 3.18, 5.04 kHz), the bright floor's, and the
/// two bursts' times (s): what rings the resonances, and the floor.
const CLICK: (f32, f32, f32, f32, f32, f32) = (3.3, 1.7, 0.95, 0.25, 0.00035, 0.00136);

/// The room 0.13.28 puts its cards in (its deals, turned cards and folds; not the chips or the
/// rail): measured off its stereo, where a card in the middle comes out the same on both sides
/// but for this, 99.8% of the difference explained. Four combs a side (764, 813, 874 and 928
/// samples on the left, each 16 later on the right) feeding back 0.651 through a one-pole
/// low-pass (0.646), each at 0.025, into two all-passes (441 and 556 samples on the left, 458
/// and 579 on the right; 0.5). For a mono sound, each side's room; it starts 16 ms on.
pub fn room(x: &[f32]) -> [Vec<f32>; 2] {
    const COMBS: [usize; 4] = [764, 813, 874, 928];
    const SPREAD: usize = 16;
    const PASSES: [[usize; 2]; 2] = [[441, 556], [458, 579]];
    let (feed, damp, gain, ap) = (0.6509f32, 0.6462f32, 0.02502f32, 0.4994f32);
    let n = x.len() + ms(400.0);
    let mut out = [vec![0.0f32; n], vec![0.0f32; n]];
    for (side, o) in out.iter_mut().enumerate() {
        for d in COMBS {
            let d = d + if side == 1 { SPREAD } else { 0 };
            // y[i] = x[i-d] + feed f[i-d]; f = (1 - damp) y + damp f[-1]
            let mut f = vec![0.0f32; n];
            let mut last = 0.0f32;
            for i in 0..n {
                let y = if i >= d { x.get(i - d).copied().unwrap_or(0.0) + feed * f[i - d] } else { 0.0 };
                last = (1.0 - damp) * y + damp * last;
                f[i] = last;
                o[i] += y * gain;
            }
        }
        for d in PASSES[side] {
            // y[i] = -ap x[i] + x[i-d] + ap y[i-d]
            let src = o.clone();
            for i in 0..n {
                let back = if i >= d { src[i - d] + ap * o[i - d] } else { 0.0 };
                o[i] = -ap * src[i] + back;
            }
        }
    }
    out
}

/// A table sound placed `pan` (-1 left to 1 right; the middle 0.707 a side), 48 kHz stereo
/// interleaved; the cards with 0.13.28's room. The room isn't placed with the card: for a card
/// at a side, 0.13.28's is louder on the far side (heard on its folds: 0.34 of the pan, the
/// whole room fed what both sides get on average).
pub fn place(mono: &[f32], pan: f32, with_room: bool) -> Vec<f32> {
    let pan = pan.clamp(-1.0, 1.0);
    let (l, r) = (((1.0 - pan) * 0.5).sqrt(), ((1.0 + pan) * 0.5).sqrt());
    if !with_room {
        return mono.iter().flat_map(|x| [x * l, x * r]).collect();
    }
    let feed: Vec<f32> = mono.iter().map(|x| x * (l + r) * 0.5).collect();
    let [wl, wr] = room(&feed);
    let (kl, kr) = (1.0 + 0.34 * pan, 1.0 - 0.34 * pan);
    (0..wl.len()).flat_map(|i| {
        let x = mono.get(i).copied().unwrap_or(0.0);
        [x * l + wl[i] * kl, x * r + wr[i] * kr]
    }).collect()
}

/// A knuckle on the rail: a dull tap and the wood's low ring (120 Hz most, 155 and 245 with it),
/// with the padding's rustle under it.
fn knock(o: &mut [f32], at: usize, amp: f32, pitch: f32, rng: &mut Rng) {
    const MODES: [(f32, f32, f32); 7] = [
        // (Hz, amplitude, decay s)
        (120.0, 1.0, 0.0155),
        (155.0, 0.28, 0.009),
        (245.0, 0.62, 0.010),
        (300.0, 0.08, 0.015),
        (380.0, 0.08, 0.012),
        (540.0, 0.25, 0.0095),
        (690.0, 0.421, 0.00365),
    ];
    for (k, (f, a, d)) in MODES.into_iter().enumerate() {
        let f = f * pitch * rng.range(0.985, 1.015);
        // (The rail's lowest ring dies away slower at the end.)
        let tail = if k == 0 { 0.03 } else { 0.0 };
        for i in 0..ms(500.0) {
            let t = i as f32 / RATE;
            let attack = (t / 0.0015).min(1.0);
            add(o, at + i, (TAU * f * t).sin() * a * ((-t / d).exp() + tail * (-t / 0.06).exp()) * attack * amp * 0.56);
        }
    }
    // The tap, dull, and the padding's rustle after it, duller (little above 1 kHz).
    let mut lp = Biquad::lowpass(1287.0, 0.7);
    let mut lp2 = Biquad::lowpass(1287.0, 0.7);
    let mut hp = Biquad::highpass(300.0, 0.7);
    let mut lp3 = Biquad::lowpass(936.0, 0.6);
    let mut hp2 = Biquad::highpass(3000.0, 0.7);
    for i in 0..ms(400.0) {
        let t = i as f32 / RATE;
        let tap = lp2.run(lp.run(rng.noise())) * (-t / 0.0015).exp() * 2.0;
        let rustle = lp3.run(hp.run(rng.noise())) * ((-t / 0.022).exp() * 0.16 + (-t / 0.065).exp() * 0.03);
        let crack = if i < ms(1.5) { hp2.run(rng.noise()) * 0.53 * (1.0 - i as f32 / ms(1.5) as f32) } else { 0.0 };
        add(o, at + i, (tap + rustle + crack) * amp);
    }
}

/// A bell note as 0.13.28's table strikes them: a sine and its partial 3.93 times higher (a fifth as
/// loud, dying three times faster), struck in 3 ms and dying away (`decay` s), cut off at `len`.
fn bell(o: &mut [f32], at: usize, f: f32, amp: f32, decay: f32, len: usize) {
    for i in 0..len {
        let t = i as f32 / RATE;
        let attack = (t / 0.003).min(1.0);
        let x = (TAU * f * t).sin() * (-t / decay).exp() + (TAU * 3.93 * f * t).sin() * 0.21 * (-t / (decay * 0.33)).exp();
        add(o, at + i, x * attack * amp);
    }
}

/// Chips put down: a stack meeting the felt and settling, a click or six, quicker and quicker.
pub fn chips(rng: &mut Rng) -> Vec<f32> {
    let mut o = vec![0.0; ms(700.0)];
    stack(&mut o, 0, 0.95, true, rng);
    o
}

/// One stack of chips coming down: (on the felt, the first click 4 to 5 ms after the stack meets
/// it, as 0.13.28's) and its clicks, two to seven (four or five most often), each gap about 0.7 of
/// the last, each click a little softer.
fn stack(o: &mut [f32], at: usize, amp: f32, on_felt: bool, rng: &mut Rng) {
    let mut t = 0.0f32;
    if on_felt {
        felt(o, at, amp * CHIP_FELT, rng);
        t = rng.range(3.8, 5.0);
    }
    const HOW_MANY: [(usize, f32); 6] = [(2, 0.07), (3, 0.19), (4, 0.29), (5, 0.33), (6, 0.07), (7, 0.05)];
    let mut pick = rng.f();
    let mut n = 4;
    for (k, p) in HOW_MANY {
        if pick < p {
            n = k;
            break;
        }
        pick -= p;
    }
    let mut gap = rng.range(30.0, 58.0);
    let mut a = 1.0f32;
    for _ in 0..n {
        chip(o, at + ms(t), rng.range(2050.0, 2310.0), amp * a * rng.range(0.88, 1.08), rng);
        t += gap;
        gap = (gap * rng.range(0.58, 0.85)).max(10.0);
        a *= 0.9;
    }
}

/// How hard the check knocks: as 0.13.28's, at the table's most (0.691 a side) two times in three
/// from a side seat and a third in the middle (heard: 17 of 25 from the sides, and 67% and 31%
/// before), its tap, rings and rustle fitted with it band by band over its first 40 ms against 25
/// checks with nothing else sounding (sixth octaves, 250 Hz to 6 kHz, within 3.6 dB, most within 2,
/// but its 356 Hz at 5-15 ms: the rebuild's rustle had been 9-14 dB too bright over 1 kHz).
const KNOCK: f32 = 1.235;

/// A check: two knocks on the rail.
pub fn check(rng: &mut Rng) -> Vec<f32> {
    let mut o = vec![0.0; ms(450.0)];
    let pitch = rng.range(0.97, 1.03);
    // (Each knock its own strength: 0.13.28's peaks spread 2 dB either way.)
    let hard = 10f32.powf(rng.range(-3.2, 3.2) / 20.0);
    knock(&mut o, 0, KNOCK * hard, pitch, rng);
    knock(&mut o, ms(rng.range(105.0, 135.0)), KNOCK * rng.range(0.7, 1.0), pitch * rng.range(0.98, 1.02), rng);
    o
}

/// A fold: the cards slid away, slowing (duller and duller), and landing on the felt 0.27 s later.
pub fn fold(rng: &mut Rng) -> Vec<f32> {
    let mut o = vec![0.0; ms(750.0)];
    card_slide(&mut o, 0, ms(350.0), (43.75, 42.4, 119.2, 0.549), FOLD.0, (2934.0, 908.0, 1.154), rng);
    felt(&mut o, ms(270.0), FOLD.1, rng);
    o
}

/// The fold's slide and felt, how loud.
const FOLD: (f32, f32) = (0.939, 0.148);

/// A card dealt off the shoe (the casino's tables): a quick slide rising in pitch as it goes, and
/// the card landing on the felt as it ends, 88 to 119 ms on (each card its own: 0.13.28's land
/// anywhere in there, following the slide's end).
pub fn card(rng: &mut Rng) -> Vec<f32> {
    let mut o = vec![0.0; ms(560.0)];
    let len = ms(rng.range(88.0, 119.0));
    card_slide(&mut o, 0, len, (13.3, 10.9, 44.3, 0.02), DEAL.0, (2251.0, 3093.0, 1.171), rng);
    felt(&mut o, len, DEAL.1, rng);
    o
}

/// The hand dealt at the poker table: the cards' slide as one off the shoe, and the deck's thump on
/// the felt 42 ms in (0.13.28's, every hand: 43-44 ms; its casino's cards have no such thump).
pub fn deal(rng: &mut Rng) -> Vec<f32> {
    let mut o = card(rng);
    felt(&mut o, ms(42.0), DEAL.2, rng);
    o
}

/// The dealt card's slide and felt, and the hand's thump, how loud.
const DEAL: (f32, f32, f32) = (0.766, 0.1, 0.069);

/// A card turned up (on the board, a hand shown, the casino's tables): a light slide rising in
/// pitch, the card's click 37 ms in, and the felt under it 6 ms after the click.
pub fn street(rng: &mut Rng) -> Vec<f32> {
    let mut o = vec![0.0; ms(520.0)];
    card_slide(&mut o, 0, ms(37.0), (10.8, 14.9, 32.1, 0.273), STREET.0, (864.0, 2060.0, 0.731), rng);
    card_click(&mut o, ms(37.0), STREET.1, rng);
    felt(&mut o, ms(42.8), STREET.2, rng);
    o
}

/// The turned card's slide, click and felt, how loud.
const STREET: (f32, f32, f32) = (0.098, 3.65, 0.135);

/// The pot pushed over to someone else, and stacked in front of them in three goes (its two parts
/// as one sound: `pot_parts`).
pub fn pot(rng: &mut Rng) -> Vec<f32> {
    let (push, stacks) = pot_parts(rng);
    let mut o = vec![0.0; stacks[0].len()];
    for part in stacks.iter().chain([&push]) {
        for (a, b) in o.iter_mut().zip(part) {
            *a += b;
        }
    }
    o
}

/// The pot in its two parts, as 0.13.28 places them apart (`poker_sfx`: the push halfway toward
/// the winner, the stacks in front of them), as 0.13.28 makes them, the same length every time:
/// the chips pushed over the felt (through the slides' resonance, 1.04 kHz rising to 2.13 by its
/// end, dying from the start, faded out from 403 to 493 ms over a quieter tail that dies on), then
/// three stacks of four chips, 520, 660 and 790 ms on; in each, the clicks about 28, 22 and 18 ms
/// apart, the last three 2.5, 4.6 and 6.5 dB softer than the first; no felt under them
/// (`measured/cardsounds.md`).
pub fn pot_parts(rng: &mut Rng) -> (Vec<f32>, [Vec<f32>; 3]) {
    let mut push = vec![0.0; ms(600.0)];
    let mut res = Reso::new(PUSH.0, PUSH.2);
    let n = push.len();
    for (i, x) in push.iter_mut().enumerate() {
        let t = i as f32 / RATE * 1000.0;
        if i % 16 == 0 {
            res.tune(PUSH.0 * (PUSH.1 / PUSH.0).powf((t / 493.0).min(1.0)), PUSH.2);
        }
        let fade = if t < 403.0 {
            1.0
        } else if t < 493.0 {
            0.5 + 0.5 * (std::f32::consts::PI * (t - 403.0) / 90.0).cos()
        } else {
            0.0
        };
        let tail = if t < 403.0 { 0.0 } else { PUSH.3 * (-(t - 403.0) / 57.0).exp() };
        let end = ((n - i) as f32 / ms(20.0) as f32).min(1.0);
        *x = res.run(rng.noise()) * (t / 2.0).min(1.0) * (-t / PUSH.4).exp() * fade.max(tail) * end * POT.0;
    }
    let stacks = [520.0, 660.0, 790.0].map(|start| {
        let mut o = vec![0.0; ms(790.0 + 420.0)];
        let mut c = start;
        for (a, gap) in [(1.0, rng.range(22.5, 33.0)), (0.747, rng.range(17.5, 27.0)), (0.591, rng.range(14.5, 21.0)), (0.473, 0.0)] {
            chip_with(&mut o, ms(c), rng.range(2050.0, 2310.0), POT.1 * a * rng.range(0.65, 1.15), 0.0, POT_RING, 0.3, rng);
            c += gap;
        }
        o
    });
    (push, stacks)
}

/// How loud the pot's push is, and its stacks' first clicks.
const POT: (f32, f32) = (0.90, 0.778);

/// The push's resonance (Hz at the start and the end, Q), its tail under the fade (0.2: 14 dB
/// under, dying 57 ms) and how fast it dies from the start (ms): fitted to 0.13.28's showdowns
/// where the push came first (20 pots), and to its end lined up at the stacks (50).
const PUSH: (f32, f32, f32, f32, f32) = (1043.0, 2132.0, 0.746, 0.2, 250.0);

/// How much longer the pot's chips ring on, partial by partial (as 0.13.28's, 12 to 40 ms after a
/// stack's last click).
const POT_RING: [f32; 4] = [1.0, 1.5, 3.9, 11.0];

/// Everything in, as 0.13.28's: the stacks shoved over the felt (360 ms, through the slides'
/// resonance, 505 Hz rising to 1.14 kHz), landing with a thump and the chips pouring from 2.5 ms
/// after it (twenty-six, 360 to 700 ms on, quicker and quicker, now and then two together,
/// softer and softer: `pour_times`), over a low A and E swelling from 320 ms and cut off 1.4 s on
/// (`measured/cardsounds.md`). As one sound (placed: `all_in_stereo`).
pub fn all_in(rng: &mut Rng) -> Vec<f32> {
    let (mut o, chips, swell) = all_in_parts(rng);
    for (at, c) in chips {
        for (i, x) in c.into_iter().enumerate() {
            add(&mut o, at + i, x);
        }
    }
    for (a, b) in o.iter_mut().zip(swell) {
        *a += b;
    }
    o
}

/// When the all-in's chips land (ms from the shove): 0.13.28's pour, 27 lone all-ins of its own
/// measured chip by chip. The first at 360, the second 24-33 ms on, the third at 400 whenever the
/// second came, the fourth 6-21 ms after it, the fifth at 431-446 whatever the fourth did, then
/// one time in two another 6-11 ms after it, the next at 457-470; from there each 15.4 ms on
/// (a little less each time) give or take 40%, twenty-six in all: the last about 700 ms on.
fn pour_times(rng: &mut Rng) -> Vec<f32> {
    let mut t = vec![360.0, 360.0 + rng.range(24.0, 33.0), rng.range(399.0, 401.0)];
    t.push(t[2] + rng.range(6.0, 21.0));
    t.push(rng.range(431.0, 446.0));
    if rng.range(0.0, 1.0) < 0.45 {
        t.push(t[4] + rng.range(6.0, 11.0));
    }
    t.push(rng.range(457.0, 470.0));
    let mut k = 0;
    while t.len() < 26 {
        let last = t[t.len() - 1];
        t.push(last + 15.4 * (-(k as f32) / 33.0).exp() * rng.range(0.6, 1.4));
        k += 1;
    }
    t
}

/// The all-in's parts: the shove and its thump, each chip poured (where it starts, and it), and
/// the swell.
#[allow(clippy::type_complexity)]
pub fn all_in_parts(rng: &mut Rng) -> (Vec<f32>, Vec<(usize, Vec<f32>)>, Vec<f32>) {
    let mut o = vec![0.0; ms(1700.0)];
    card_slide(&mut o, 0, ms(360.0), (34.6, 81.2, 154.6, 0.055), ALLIN.0, (505.0, 1143.0, 0.827), rng);
    felt(&mut o, ms(357.5), ALLIN.1, rng);
    let mut chips = Vec::new();
    let mut a = ALLIN.2;
    for (k, t) in pour_times(rng).into_iter().enumerate() {
        let mut c = vec![0.0; ms(340.0)];
        let soft = match k {
            0 => 0.9,
            // (0.13.28's third, at 400 ms, 2.6 dB softer than the chips either side of it.)
            2 => 0.74,
            _ => 1.0,
        };
        chip_with(&mut c, 0, rng.range(2050.0, 2310.0), a * rng.range(0.8, 1.1) * soft, POUR_RATTLE.0, [1.0; 4], POUR_RATTLE.1, rng);
        chips.push((ms(t), c));
        a *= 0.973;
    }
    // The swell: A1 and E2 (6.4 dB under it) rising together from 320 ms for 0.485 s, dying
    // 0.335 s, cut off at 1.397 s (a click, as 0.13.28's are).
    let mut swell = vec![0.0; ms(1700.0)];
    let at = ms(320.0);
    for i in 0..ms(1077.0) {
        let t = i as f32 / RATE;
        let env = (t / 0.485).min(1.0) * (-(t - 0.485).max(0.0) / 0.335).exp();
        swell[at + i] = ((TAU * 55.0 * t).sin() + (TAU * 82.41 * t).sin() * 0.479) * env * SWELL;
    }
    (o, chips, swell)
}

/// The all-in placed as 0.13.28 places it (48 kHz stereo, interleaved), `pan` being the seat's, as
/// the chips' are: the shove and its thump there (R/L 5.1 dB from a side seat), each chip poured
/// somewhere about there (0.39 either way: 0.13.28's pour is 17-18 dB wider than its shove), the
/// swell in the middle but for its lean left (0.13.28's two tones 0.55 and 0.68 dB stronger on the
/// left, from the middle seat or a side one: `SWELL_PAN`); all of it in the room (`place`'s), fed
/// 4 dB hotter than the cards.
pub fn all_in_stereo(pan: f32, rng: &mut Rng) -> Vec<f32> {
    let (shove, chips, swell) = all_in_parts(rng);
    let n = shove.len();
    let (mut l, mut r, mut feed) = (vec![0.0f32; n], vec![0.0f32; n], vec![0.0f32; n]);
    let mut feed_swell = vec![0.0f32; n];
    let p0 = pan.clamp(-1.0, 1.0);
    let mut put = |x: &[f32], at: usize, p: f32, swell: bool| {
        let (gl, gr) = (((1.0 - p) / 2.0).sqrt(), ((1.0 + p) / 2.0).sqrt());
        for (i, v) in x.iter().enumerate() {
            if let Some(j) = (at + i < n).then_some(at + i) {
                l[j] += v * gl;
                r[j] += v * gr;
                let fed = if swell { &mut feed_swell } else { &mut feed };
                fed[j] += v * (gl + gr) / 2.0 * ALLIN_ROOM;
            }
        }
    };
    put(&shove, 0, p0, false);
    for (at, c) in &chips {
        put(c, *at, (p0 + rng.range(-0.39, 0.39)).clamp(-1.0, 1.0), false);
    }
    put(&swell, 0, SWELL_PAN, true);
    let [wl, wr] = room(&feed);
    // (The swell's room as loud on both sides wherever the all-in is: 0.13.28's swell leans the
    // same from a side seat as from the middle, where the room louder on the far side would lean
    // it more; its room rings on after it's cut off, 45 dB under it.)
    let [sl, sr] = room(&feed_swell);
    let (kl, kr) = (1.0 + 0.34 * p0, 1.0 - 0.34 * p0);
    let mut out = vec![0.0f32; wl.len() * 2];
    for i in 0..wl.len() {
        out[2 * i] = l.get(i).copied().unwrap_or(0.0) + wl[i] * kl + sl[i];
        out[2 * i + 1] = r.get(i).copied().unwrap_or(0.0) + wr[i] * kr + sr[i];
    }
    out
}

/// The all-in's shove, its thump and its first chip, how loud.
const ALLIN: (f32, f32, f32) = (0.9, 0.30, 0.815);

/// How loud the all-in's swell is (its A1).
const SWELL: f32 = 0.118;

/// The poured chips' rattle: how much (a bet's: 1) and how much of its longer part (a bet's: 0.3).
/// 0.13.28's pour leaves only the room by 720 ms (its top bands dying 0.35 dB a ms after the last
/// chip, the side as loud as the middle), where thirty chips' long rattles had rung on 15-30 dB
/// over it (heard at 12 all-ins alone for 1.6 s, both apps watching).
const POUR_RATTLE: (f32, f32) = (1.0, 0.0);

/// Where the all-in's swell is put, wherever the all-in is: a little to the left (R/L -0.6 dB).
const SWELL_PAN: f32 = -0.069;

/// How much hotter the all-in is fed to the room than the cards (4 dB).
const ALLIN_ROOM: f32 = 1.585;

/// I won: a bell run up C, E, G, B, D, G (77 ms apart), and the pot pouring in (sixteen chips or
/// so) from 0.38 s.
pub fn win(rng: &mut Rng) -> Vec<f32> {
    let mut o = vec![0.0; ms(2000.0)];
    for (k, f) in [523.25, 659.25, 783.99, 987.77, 1174.66, 1567.98].iter().enumerate() {
        bell(&mut o, ms(k as f32 * 77.0), *f, 0.184, 0.193, ms(1500.0));
    }
    let mut t = rng.range(370.0, 420.0);
    let mut gap = rng.range(40.0, 50.0);
    for _ in 0..(14 + (rng.f() * 4.0) as usize) {
        chip(&mut o, ms(t), rng.range(2050.0, 2310.0), 0.47 * rng.range(0.85, 1.05), rng);
        t += gap * rng.range(0.75, 1.25);
        gap = (gap * 0.965).max(20.0);
    }
    o
}

/// My turn: two bell notes, A5 and then E6, 110.5 ms on (as 0.13.28 strikes it: heard seated, twice,
/// 111.0 and 111.1 ms by half its peak; the rebuild's had been 118).
pub fn turn() -> Vec<f32> {
    let mut o = vec![0.0; ms(1000.0)];
    for (at, f, amp, decay, len, _) in TURN {
        bell(&mut o, ms(at), f, amp, decay, ms(len));
    }
    o
}

/// My turn's bells: when (ms), the note, how loud, how long they ring (s), for how long (ms), and how
/// far each is put from where it's placed (0.13.28 spreads them as it does the time bank's: A5 0.09
/// to the left, E6 0.09 to the right; R/L -0.76 and +0.87 dB over their first 10 ms, seated).
const TURN: [(f32, f32, f32, f32, f32, f32); 2] = [(0.0, 880.0, 0.355, 0.091, 700.0, -0.09), (110.5, 1318.5, 0.276, 0.103, 880.0, 0.09)];

/// My turn, placed `pan` (48 kHz stereo, interleaved): each bell put where 0.13.28 puts it, and the
/// whole as loud at its loudest wherever it's placed (as the time bank).
pub fn turn_stereo(pan: f32) -> Vec<f32> {
    spread(ms(1000.0), &TURN, pan)
}

/// Bells struck one after another and spread across where they're placed, as 0.13.28 plays its
/// chords of bells: each placed `pan` + its own offset, then the whole turned to as loud at its
/// loudest as in the middle.
fn spread(n: usize, bells: &[(f32, f32, f32, f32, f32, f32)], pan: f32) -> Vec<f32> {
    let one = |pan: f32| {
        let mut st = vec![0.0f32; n * 2];
        for &(at, f, amp, decay, len, off) in bells {
            let mut m = vec![0.0f32; n];
            bell(&mut m, ms(at), f, amp, decay, ms(len));
            for (o, x) in st.iter_mut().zip(place(&m, (pan + off).clamp(-1.0, 1.0), false)) {
                *o += x;
            }
        }
        st
    };
    let peak = |st: &[f32]| st.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    let mut st = one(pan);
    let k = peak(&one(0.0)) / peak(&st).max(1e-9);
    for x in &mut st {
        *x *= k;
    }
    st
}

/// Someone's time (and bank) out, folded or checked for them: two knocks on the rail, then bells E5
/// and C#5, falling (struck 270 and 430 ms after the first knock, as 0.13.28 strikes them: heard
/// at 11 time-outs, 270.1 and 430.6 ms, where the rebuild had struck them at 245 and 394).
pub fn time_out(rng: &mut Rng) -> Vec<f32> {
    let mut o = vec![0.0; ms(1040.0)];
    let pitch = rng.range(0.97, 1.03);
    // (The knocks as loud as 0.13.28's in the middle (within 0.5 dB at four time-outs), the second
    // nearly as strong as the first, and the bells: so that from a side seat the whole is turned
    // down when it'd go over (`held`), time by time, as 0.13.28's is, its near side 0.3 to 2 dB
    // quieter there than the pan alone would make it.)
    knock(&mut o, 0, 1.34, pitch, rng);
    knock(&mut o, ms(130.0), 1.34 * rng.range(0.9, 1.0), pitch * rng.range(0.98, 1.02), rng);
    bell(&mut o, ms(270.0), 659.25, 0.258, 0.098, ms(600.0));
    bell(&mut o, ms(430.5), 554.37, 0.275, 0.087, ms(600.0));
    o
}

/// My clock's last five seconds: two short high bells, A6, 120 ms apart, the second a little softer
/// and longer (0.13.28's, heard seated twice: each fitted, 99.85% explained, the second 119.85 ms
/// after the first by half its peak; the rebuild's had been one).
pub fn hurry() -> Vec<f32> {
    let mut o = vec![0.0; ms(520.0)];
    bell(&mut o, 0, 1760.3, 0.404, 0.0326, ms(380.0));
    bell(&mut o, ms(120.7), 1760.3, 0.346, 0.0371, ms(380.0));
    o
}

/// A time bank starting: B5, D#6 and B6, rising, 95 ms apart (each struck when 0.13.28 strikes it:
/// heard at 27 banks, the second 94.9 and the third 190.0 ms after the first).
pub fn time_bank() -> Vec<f32> {
    let mut o = vec![0.0; ms(900.0)];
    for (at, f, amp, decay, _) in BANK {
        bell(&mut o, ms(at), f, amp, decay, ms(700.0));
    }
    o
}

/// The time bank's bells: when (ms), the note, how loud, how long they ring, and how far each is put
/// from where the bank is placed (0.13.28 spreads them: the first 0.15 to the left, the last 0.15
/// to the right). Each as loud as 0.13.28's over its first 10 ms, within 0.3 dB at each seat heard.
const BANK: [(f32, f32, f32, f32, f32); 3] = [(0.0, 987.77, 0.333, 0.091, -0.15), (95.0, 1244.51, 0.278, 0.084, 0.0), (190.0, 1975.53, 0.221, 0.087, 0.15)];

/// A time bank starting, placed `pan` (48 kHz stereo, interleaved): each bell put where 0.13.28 puts
/// it (spread 0.15 either way of `pan`), and the whole as loud at its loudest wherever it's placed,
/// as 0.13.28's is (from a side seat its near side no louder than from the middle, by 0.0 to 0.5 dB
/// a bell, its far side 4.3 to 5.9 dB quieter: heard at the middle seat and a side seat, each bell's
/// first 10 ms, within 0.1 dB of this).
pub fn time_bank_stereo(pan: f32) -> Vec<f32> {
    spread(ms(900.0), &BANK.map(|(at, f, amp, decay, off)| (at, f, amp, decay, 700.0, off)), pan)
}

/// A second going by on a clock: a wood block's tick (F#6, 1,480 Hz, and 4,031, gone in 40 ms), with a
/// faint echo; `pitch` times higher (three semitones, A6, for the other of 0.13.28's two).
pub fn tick(pitch: f32, rng: &mut Rng) -> Vec<f32> {
    let mut o = vec![0.0; ms(200.0)];
    // (Fitted to 0.13.28's, tick by tick: each partial starts at its own chance phase, the low one
    // dying in 6.8 ms and the high, 0.39 as loud, in 2.8; struck at once, with a snap of noise;
    // 98.5% of each of its ticks explained so.)
    let block = |o: &mut [f32], at: usize, amp: f32, rng: &mut Rng| {
        let (p1, p2) = (rng.f() * TAU, rng.f() * TAU);
        for i in 0..ms(120.0) {
            let t = i as f32 / RATE;
            let x = (TAU * 1479.98 * pitch * t + p1).sin() * (-t / 0.0068).exp() + (TAU * 4030.6 * pitch * t + p2).sin() * 0.39 * (-t / 0.0028).exp();
            let snap = if i < ms(0.7) { rng.noise() * 0.3 } else { 0.0 };
            add(o, at + i, (x + snap) * amp);
        }
    };
    block(&mut o, 0, 0.523, rng);
    block(&mut o, ms(32.0), 0.008, rng);
    o
}
