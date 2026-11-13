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

    /// Retunes a low-pass in place (keeping its state), for a sweep.
    fn set_lowpass(&mut self, f: f32, q: f32) {
        let n = Biquad::lowpass(f, q);
        (self.b0, self.b1, self.b2, self.a1, self.a2) = (n.b0, n.b1, n.b2, n.a1, n.a2);
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
/// 5.07 times the lowest, which is 2,050 to 2,310 Hz) dying fast and then ringing on softly.
fn chip(o: &mut [f32], at: usize, f1: f32, amp: f32, rng: &mut Rng) {
    const PARTS: [(f32, f32, f32, f32, f32); 4] = [
        // (ratio, amplitude, fast decay s, tail amplitude, tail decay s)
        (1.0, 1.0, 0.0092, 0.035, 0.045),
        (2.234, 0.95, 0.0054, 0.022, 0.032),
        (3.468, 0.78, 0.0036, 0.012, 0.022),
        (5.07, 0.55, 0.0021, 0.005, 0.020),
    ];
    let len = ms(330.0);
    for (r, a, d1, a2, d2) in PARTS {
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
    let mut lp2 = Biquad::lowpass(9000.0, 0.5);
    let mut body = Biquad::lowpass(1200.0, 0.6);
    for i in 0..ms(300.0) {
        let t = i as f32 / RATE;
        let tick = if i < ms(0.6) {
            let e = 1.0 - i as f32 / ms(0.6) as f32;
            hp.run(rng.noise()) * e * 0.45
        } else {
            0.0
        };
        let rattle = lp2.run(hp2.run(rng.noise())) * ((-t / 0.028).exp() + 0.3 * (-t / 0.07).exp()) * RATTLE;
        // (And the knock of it, low down, through the table.)
        let thud = body.run(rng.noise()) * (-t / 0.012).exp() * 0.05;
        add(o, at + i, (tick + rattle + thud) * amp);
    }
}

/// How loud the rattle under a chip's ring is.
const RATTLE: f32 = 0.006;

/// Something set down on the felt: a soft, low thump (about 100 Hz, mostly gone in a tenth of a
/// second, the table and the cloth ringing on a little longer under it).
fn felt(o: &mut [f32], at: usize, amp: f32, rng: &mut Rng) {
    let f = rng.range(92.0, 104.0);
    let mut lp = Biquad::lowpass(1100.0, 0.6);
    let mut hp2 = Biquad::highpass(300.0, 0.6);
    let mut lp2 = Biquad::lowpass(2400.0, 0.6);
    for i in 0..ms(450.0) {
        let t = i as f32 / RATE;
        let attack = (t / 0.002).min(1.0);
        let tone = (TAU * f * t).sin() * ((-t / 0.02).exp() + 0.08 * (-t / 0.045).exp());
        let rustle = lp.run(rng.noise()) * (-t / 0.016).exp() * 0.6 + lp2.run(hp2.run(rng.noise())) * (-t / 0.05).exp() * 0.06;
        add(o, at + i, (tone + rustle) * attack * amp);
    }
}

/// Something sliding over the cloth (a card, or chips pushed): noise between `low` Hz and a top
/// falling from `top0` to `top1` Hz as it slows, rising over `attack`, holding a moment and fading
/// faster and faster.
#[allow(clippy::too_many_arguments)]
fn slide(o: &mut [f32], at: usize, len: usize, attack: f32, amp: f32, low: f32, top0: f32, top1: f32, rng: &mut Rng) {
    let mut hp = Biquad::highpass(low, 0.5);
    let mut lp = Biquad::lowpass(top0, 0.6);
    let a = ms(attack).max(1);
    let hold = a + ms(20.0);
    let tail = (len / 4).max(1);
    for i in 0..len {
        let k = i as f32 / len as f32;
        if i % 32 == 0 {
            lp.set_lowpass(top0 + (top1 - top0) * k, 0.6);
        }
        let rise = (i as f32 / a as f32).min(1.0);
        let fall = if i < hold { 1.0 } else { (-((i - hold) as f32 / RATE) / (0.43 * len as f32 / RATE)).exp() };
        let end = if len - i < tail { (0.5 - 0.5 * (std::f32::consts::PI * (len - i) as f32 / tail as f32).cos()).max(0.0) } else { 1.0 };
        add(o, at + i, lp.run(hp.run(rng.noise())) * rise * fall * end * amp);
    }
}

/// A knuckle on the rail: a dull tap and the wood's low ring (120 Hz most, 155 and 245 with it),
/// with the padding's rustle under it.
fn knock(o: &mut [f32], at: usize, amp: f32, pitch: f32, rng: &mut Rng) {
    const MODES: [(f32, f32, f32); 6] = [
        // (Hz, amplitude, decay s)
        (120.0, 1.0, 0.018),
        (155.0, 0.28, 0.013),
        (245.0, 0.62, 0.016),
        (300.0, 0.08, 0.025),
        (380.0, 0.08, 0.02),
        (540.0, 0.16, 0.0045),
    ];
    for (k, (f, a, d)) in MODES.into_iter().enumerate() {
        let f = f * pitch * rng.range(0.985, 1.015);
        // (The rail's lowest ring dies away slower at the end.)
        let tail = if k == 0 { 0.06 } else { 0.0 };
        for i in 0..ms(500.0) {
            let t = i as f32 / RATE;
            let attack = (t / 0.0015).min(1.0);
            add(o, at + i, (TAU * f * t).sin() * a * ((-t / d).exp() + tail * (-t / 0.06).exp()) * attack * amp * 0.5);
        }
    }
    // The tap, dull (little above 2 kHz), and the rustle after it.
    let mut lp = Biquad::lowpass(1800.0, 0.7);
    let mut lp2 = Biquad::lowpass(1800.0, 0.7);
    let mut hp = Biquad::highpass(300.0, 0.7);
    let mut lp3 = Biquad::lowpass(2000.0, 0.6);
    let mut hp2 = Biquad::highpass(3000.0, 0.7);
    for i in 0..ms(400.0) {
        let t = i as f32 / RATE;
        let tap = lp2.run(lp.run(rng.noise())) * (-t / 0.0015).exp() * 1.6;
        let rustle = lp3.run(hp.run(rng.noise())) * ((-t / 0.022).exp() * 0.08 + (-t / 0.065).exp() * 0.012);
        let crack = if i < ms(0.6) { hp2.run(rng.noise()) * 0.25 } else { 0.0 };
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

/// One stack of chips coming down: (on the felt) and its clicks, two to seven (four or five most
/// often), each gap about 0.7 of the last, each click a little softer.
fn stack(o: &mut [f32], at: usize, amp: f32, on_felt: bool, rng: &mut Rng) {
    if on_felt {
        felt(o, at, amp * 0.16, rng);
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
    let mut t = 0.0f32;
    let mut gap = rng.range(30.0, 58.0);
    let mut a = 1.0f32;
    for _ in 0..n {
        chip(o, at + ms(t), rng.range(2050.0, 2310.0), amp * a * rng.range(0.88, 1.08), rng);
        t += gap;
        gap = (gap * rng.range(0.58, 0.85)).max(10.0);
        a *= 0.9;
    }
}

/// A check: two knocks on the rail.
pub fn check(rng: &mut Rng) -> Vec<f32> {
    let mut o = vec![0.0; ms(450.0)];
    let pitch = rng.range(0.97, 1.03);
    knock(&mut o, 0, 1.16, pitch, rng);
    knock(&mut o, ms(rng.range(105.0, 135.0)), 1.16 * rng.range(0.7, 1.0), pitch * rng.range(0.98, 1.02), rng);
    o
}

/// A fold: the cards slid away and landing on the felt 0.27 s later.
pub fn fold(rng: &mut Rng) -> Vec<f32> {
    let mut o = vec![0.0; ms(750.0)];
    slide(&mut o, 0, ms(270.0), 40.0, 0.79, 650.0, 9000.0, 3500.0, rng);
    felt(&mut o, ms(270.0), 0.175, rng);
    o
}

/// A card's edge snapping down on the felt: a click of a fraction of a millisecond, most of it
/// around 2 kHz, and the cloth's rustle after it.
fn snap(o: &mut [f32], at: usize, amp: f32, rng: &mut Rng) {
    let ph = rng.range(0.0, TAU);
    let mut hp = Biquad::highpass(300.0, 0.6);
    let mut lp = Biquad::lowpass(6000.0, 0.6);
    for i in 0..ms(200.0) {
        let t = i as f32 / RATE;
        let click = ((TAU * 1900.0 * t + ph).sin() * 0.8 + rng.noise() * 0.5) * (-t / 0.0005).exp();
        let rustle = lp.run(hp.run(rng.noise())) * (-t / 0.03).exp() * 0.05;
        add(o, at + i, (click + rustle) * amp);
    }
}

/// The hand dealt: the cards' quick slide, and one landing (a light snap) 44 ms in.
pub fn deal(rng: &mut Rng) -> Vec<f32> {
    let mut o = vec![0.0; ms(520.0)];
    slide(&mut o, 0, ms(110.0), 8.0, 0.47, 650.0, 10_000.0, 6000.0, rng);
    snap(&mut o, ms(44.0), 0.2, rng);
    felt(&mut o, ms(44.0), 0.1, rng);
    o
}

/// A card turned up on the board (or a hand shown): a light slide, and the card snapping down on
/// the felt 40 ms in.
pub fn street(rng: &mut Rng) -> Vec<f32> {
    let mut o = vec![0.0; ms(520.0)];
    slide(&mut o, 0, ms(46.0), 10.0, 0.06, 400.0, 5500.0, 5000.0, rng);
    snap(&mut o, ms(40.0), 0.9, rng);
    felt(&mut o, ms(42.0), 0.11, rng);
    o
}

/// The pot pushed over to someone else, and stacked in front of them in three goes.
pub fn pot(rng: &mut Rng) -> Vec<f32> {
    let mut o = vec![0.0; ms(1400.0)];
    slide(&mut o, 0, ms(560.0), 50.0, 0.88, 450.0, 3200.0, 2000.0, rng);
    let mut t = 560.0 + rng.range(30.0, 75.0);
    for _ in 0..3 {
        // Three or four chips each (two now and then), close together; no felt under them.
        let n = if rng.f() < 0.1 { 2 } else if rng.f() < 0.5 { 3 } else { 4 };
        let mut c = t;
        let mut gap = rng.range(20.0, 34.0);
        for _ in 0..n {
            chip(&mut o, ms(c), rng.range(2050.0, 2310.0), 0.59 * rng.range(0.85, 1.05), rng);
            c += gap;
            gap = (gap * rng.range(0.7, 0.95)).max(9.0);
        }
        t += rng.range(115.0, 170.0);
    }
    o
}

/// Everything in: the stacks shoved over the felt, then pouring into the pot (about twenty chips,
/// quicker and quicker) over a low A and E swelling under them.
pub fn all_in(rng: &mut Rng) -> Vec<f32> {
    let mut o = vec![0.0; ms(1700.0)];
    // The shove lasts till the chips start pouring.
    let start = rng.range(355.0, 420.0);
    slide(&mut o, 0, ms(start), 40.0, 1.64, 250.0, 1400.0, 900.0, rng);
    felt(&mut o, ms(start), 0.14, rng);
    let mut t = start;
    let mut gap = rng.range(20.0, 30.0);
    for _ in 0..(18 + (rng.f() * 5.0) as usize) {
        chip(&mut o, ms(t), rng.range(2050.0, 2310.0), 0.88 * rng.range(0.7, 1.0), rng);
        t += gap * rng.range(0.6, 1.4);
        gap = (gap * 0.93).max(8.0);
    }
    // The swell: A1 rising over half a second, E2 with it, both cut off 1.15 s on (a click, as
    // 0.13.28's are).
    let at = ms(start);
    for i in 0..ms(1150.0) {
        let t = i as f32 / RATE;
        let a = (t / 0.5).min(1.0) * (-(t - 0.5).max(0.0) / 0.29).exp();
        let e = (t / 0.1).min(1.0) * (-(t - 0.2).max(0.0) / 0.29).exp();
        add(&mut o, at + i, (TAU * 55.0 * t).sin() * a * 0.112 + (TAU * 82.41 * t).sin() * e * 0.049);
    }
    o
}

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

/// My turn: two bell notes, A5 and then E6.
pub fn turn() -> Vec<f32> {
    let mut o = vec![0.0; ms(1000.0)];
    bell(&mut o, 0, 880.0, 0.267, 0.091, ms(700.0));
    bell(&mut o, ms(118.0), 1318.5, 0.24, 0.103, ms(880.0));
    o
}

/// My clock's last five seconds: one short high bell, A6.
pub fn hurry() -> Vec<f32> {
    let mut o = vec![0.0; ms(400.0)];
    bell(&mut o, 0, 1760.0, 0.242, 0.038, ms(380.0));
    o
}

/// A time bank starting: B5, D#6 and B6, rising.
pub fn time_bank() -> Vec<f32> {
    let mut o = vec![0.0; ms(900.0)];
    bell(&mut o, 0, 987.77, 0.257, 0.091, ms(700.0));
    bell(&mut o, ms(98.0), 1244.51, 0.227, 0.084, ms(700.0));
    bell(&mut o, ms(199.0), 1975.53, 0.196, 0.087, ms(700.0));
    o
}

/// A second going by on a clock: a wood block's tick (1,475 Hz, and 4,017, gone in 40 ms), with a
/// faint echo;
/// `pitch` higher in a time bank (1.18: three semitones).
pub fn tick(pitch: f32, rng: &mut Rng) -> Vec<f32> {
    let mut o = vec![0.0; ms(200.0)];
    let block = |o: &mut [f32], at: usize, amp: f32, rng: &mut Rng| {
        let ph = if rng.f() < 0.5 { 0.0 } else { std::f32::consts::PI };
        for i in 0..ms(120.0) {
            let t = i as f32 / RATE;
            let attack = (t / 0.0008).min(1.0);
            let x = (TAU * 1475.0 * pitch * t + ph).sin() * (-t / 0.0064).exp() + (TAU * 4017.0 * pitch * t + ph).sin() * 0.3 * (-t / 0.003).exp();
            let snap = if i < ms(0.7) { rng.noise() * 0.3 } else { 0.0 };
            add(o, at + i, (x * attack + snap) * amp);
        }
    };
    block(&mut o, 0, 0.66, rng);
    block(&mut o, ms(32.0), 0.008, rng);
    o
}
