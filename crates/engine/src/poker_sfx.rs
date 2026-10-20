//! The poker table's sounds, made here rather than recorded: cards snapping off the deck and
//! turning over, chips clicking into the pot, a knock for a check, a soft chime when it's your
//! turn, and a little rising run when you win. 48 kHz stereo, a little to the side they
//! happen on.

use std::f32::consts::TAU;

const RATE: f32 = 48_000.0;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Cue {
    /// A card dealt (slid off the deck).
    Deal,
    /// A card turned face up.
    Flip,
    /// Chips put in (a bet, a call, a blind).
    Chips,
    /// A knock on the table.
    Check,
    /// Cards pushed away.
    Fold,
    /// Everything in.
    AllIn,
    /// The pot sliding over to whoever won it.
    Pot,
    /// It's my turn.
    Turn,
    /// I won.
    Win,
    /// My clock is running out.
    Hurry,
    /// A second going by in someone's last five.
    Tick,
    /// Someone's time ran out and their time bank started.
    TimeBank,
    /// Someone's time (and bank) ran out: folded (or checked) for them.
    TimeOut,
    // Pyr Casino.
    /// Slot reels set spinning: a ratchet's whirr.
    ReelSpin,
    /// A reel coming to rest.
    ReelStop,
    /// A small win: two bright notes.
    WinSmall,
    /// A big win: a rising run and coins pouring.
    WinBig,
    /// The Phoenix Jackpot: a fanfare.
    Jackpot,
    /// A roulette ball circling the wheel.
    BallRoll,
    /// The ball rattling into a pocket.
    BallDrop,
    /// Dice shaken and thrown down the table.
    DiceRoll,
    /// A keno ball out of the machine.
    KenoBall,
    /// One coin (a win counting up).
    Coin,
    // Time Bombs.
    /// The prize wheel: a ratchet slowing to a stop, and a ding.
    Wheel,
    /// The fuse burning down: a sizzle.
    Fuse,
    /// The bomb going off.
    Boom,
}

/// A small, fast noise (the same every time, so a sound is the same every time).
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

fn ms(n: f32) -> usize {
    (n / 1000.0 * RATE) as usize
}

/// Filtered noise with an envelope: a card's swish or snap.
fn swish(out: &mut [f32], at: usize, len: usize, amp: f32, bright: f32, seed: u32) {
    let mut n = Noise(seed.max(1));
    let (mut lp, mut prev) = (0.0f32, 0.0f32);
    for i in 0..len {
        let t = i as f32 / len as f32;
        let env = (t / 0.08).min(1.0) * (1.0 - t).powf(2.5);
        let x = n.next();
        lp += (x - lp) * bright;
        // A touch of high-pass so it's crisp, not a rumble.
        let hp = lp - prev * 0.6;
        prev = lp;
        if let Some(o) = out.get_mut(at + i) {
            *o += hp * env * amp;
        }
    }
}

/// A clay chip's click: a short burst of two inharmonic partials and a tick of noise.
fn click(out: &mut [f32], at: usize, amp: f32, pitch: f32, seed: u32) {
    let len = ms(70.0);
    let mut n = Noise(seed.max(1));
    for i in 0..len {
        let t = i as f32 / RATE;
        let env = (-t * 90.0).exp();
        let tone = (TAU * pitch * t).sin() * 0.6 + (TAU * pitch * 2.76 * t).sin() * 0.35 + (TAU * pitch * 5.4 * t).sin() * 0.15;
        let tick = if i < ms(3.0) { n.next() * (1.0 - i as f32 / ms(3.0) as f32) } else { 0.0 };
        if let Some(o) = out.get_mut(at + i) {
            *o += (tone * env * 0.7 + tick * 0.5) * amp;
        }
    }
}

/// A soft bell note.
fn bell(out: &mut [f32], at: usize, freq: f32, dur_ms: f32, amp: f32) {
    let len = ms(dur_ms);
    for i in 0..len {
        let t = i as f32 / RATE;
        let attack = (i as f32 / ms(5.0) as f32).min(1.0);
        let env = attack * (-t * 5.0 / (dur_ms / 1000.0)).exp();
        let w = (TAU * freq * t).sin() + 0.3 * (TAU * 2.0 * freq * t).sin() + 0.08 * (TAU * 3.01 * freq * t).sin();
        if let Some(o) = out.get_mut(at + i) {
            *o += w * env * amp;
        }
    }
}

/// A knuckle on felt: a low, damped thump.
fn knock(out: &mut [f32], at: usize, amp: f32) {
    let len = ms(90.0);
    let mut n = Noise(99);
    let mut lp = 0.0f32;
    for i in 0..len {
        let t = i as f32 / RATE;
        let env = (-t * 55.0).exp();
        let f = 150.0 - 60.0 * (t * 20.0).min(1.0);
        lp += (n.next() - lp) * 0.08;
        if let Some(o) = out.get_mut(at + i) {
            *o += ((TAU * f * t).sin() * 0.8 + lp * 0.9) * env * amp;
        }
    }
}

/// The sound, mono.
pub fn render_mono(c: Cue) -> Vec<f32> {
    let len = match c {
        Cue::Deal | Cue::Flip | Cue::Check | Cue::Hurry | Cue::Tick => ms(220.0),
        Cue::TimeBank => ms(700.0),
        Cue::TimeOut => ms(800.0),
        Cue::ReelSpin => ms(650.0),
        Cue::ReelStop => ms(160.0),
        Cue::WinSmall => ms(520.0),
        Cue::WinBig => ms(2200.0),
        Cue::Jackpot => ms(3600.0),
        Cue::BallRoll => ms(3200.0),
        Cue::BallDrop => ms(900.0),
        Cue::DiceRoll => ms(1100.0),
        Cue::KenoBall => ms(180.0),
        Cue::Coin => ms(140.0),
        Cue::Wheel => ms(4600.0),
        Cue::Fuse => ms(900.0),
        Cue::Boom => ms(2600.0),
        Cue::Fold | Cue::Chips | Cue::Turn => ms(420.0),
        Cue::AllIn | Cue::Pot => ms(900.0),
        Cue::Win => ms(1300.0),
    };
    let mut o = vec![0.0f32; len];
    match c {
        Cue::Deal => swish(&mut o, 0, ms(110.0), 0.55, 0.55, 7),
        Cue::Flip => {
            swish(&mut o, 0, ms(60.0), 0.45, 0.7, 11);
            swish(&mut o, ms(25.0), ms(40.0), 0.35, 0.9, 13);
        }
        Cue::Check => {
            knock(&mut o, 0, 0.7);
            knock(&mut o, ms(105.0), 0.55);
        }
        Cue::Fold => swish(&mut o, 0, ms(260.0), 0.4, 0.25, 17),
        Cue::Chips => {
            for (k, (d, p)) in [(0.0, 2900.0), (38.0, 3300.0), (61.0, 2700.0), (95.0, 3100.0)].iter().enumerate() {
                click(&mut o, ms(*d), 0.4 - k as f32 * 0.05, *p, 21 + k as u32);
            }
        }
        Cue::AllIn => {
            swish(&mut o, 0, ms(380.0), 0.35, 0.3, 23);
            for k in 0..9 {
                click(&mut o, ms(40.0 + k as f32 * 37.0 + (k % 3) as f32 * 9.0), 0.36, 2600.0 + (k * 197 % 900) as f32, 31 + k);
            }
        }
        Cue::Pot => {
            swish(&mut o, 0, ms(500.0), 0.3, 0.2, 41);
            for k in 0..7 {
                click(&mut o, ms(180.0 + k as f32 * 52.0), 0.3, 2800.0 + (k * 233 % 700) as f32, 51 + k);
            }
        }
        Cue::Turn => {
            bell(&mut o, 0, 880.0, 300.0, 0.18);
            bell(&mut o, ms(90.0), 1318.5, 320.0, 0.14);
        }
        Cue::Win => {
            for (k, f) in [523.25, 659.25, 783.99, 1046.5].iter().enumerate() {
                bell(&mut o, ms(k as f32 * 95.0), *f, 600.0, 0.16);
            }
            for k in 0..6 {
                click(&mut o, ms(420.0 + k as f32 * 60.0), 0.22, 3000.0 + (k * 151 % 600) as f32, 61 + k);
            }
        }
        Cue::Hurry => bell(&mut o, 0, 1567.98, 160.0, 0.16),
        // A clock's tick: a wood block's crack with a little ring.
        Cue::Tick => {
            click(&mut o, 0, 0.42, 1900.0, 71);
            bell(&mut o, 0, 2093.0, 70.0, 0.07);
        }
        // Time bank: a rising pair, "more time".
        Cue::TimeBank => {
            bell(&mut o, 0, 987.77, 380.0, 0.17);
            bell(&mut o, ms(140.0), 1479.98, 480.0, 0.16);
        }
        // A ratchet: quick clicks, speeding up, over a low whirr.
        Cue::ReelSpin => {
            swish(&mut o, 0, ms(620.0), 0.18, 0.08, 81);
            let mut t = 0.0f32;
            let mut k = 0;
            while t < 600.0 {
                click(&mut o, ms(t), 0.16, 1400.0 + (k % 3) as f32 * 120.0, 83 + k);
                t += (55.0 - k as f32 * 2.5).max(22.0);
                k += 1;
            }
        }
        // A reel stops: a damped thunk and a latch's click.
        Cue::ReelStop => {
            knock(&mut o, 0, 0.55);
            click(&mut o, ms(8.0), 0.25, 2100.0, 91);
        }
        Cue::WinSmall => {
            bell(&mut o, 0, 1046.5, 320.0, 0.16);
            bell(&mut o, ms(110.0), 1567.98, 380.0, 0.15);
        }
        // Up a major arpeggio, then coins.
        Cue::WinBig => {
            for (k, f) in [523.25, 659.25, 783.99, 1046.5, 1318.5, 1567.98].iter().enumerate() {
                bell(&mut o, ms(k as f32 * 85.0), *f, 700.0, 0.13);
            }
            for k in 0..22 {
                let at = 520.0 + k as f32 * 62.0 + (k * 37 % 23) as f32;
                click(&mut o, ms(at), 0.17, 3200.0 + (k * 211 % 900) as f32, 101 + k);
            }
        }
        // A fanfare: two rising runs, a held chord, and a shower of coins.
        Cue::Jackpot => {
            for (k, f) in [392.0, 523.25, 659.25, 783.99].iter().enumerate() {
                bell(&mut o, ms(k as f32 * 120.0), *f, 600.0, 0.12);
            }
            for (k, f) in [523.25, 659.25, 783.99, 1046.5].iter().enumerate() {
                bell(&mut o, ms(560.0 + k as f32 * 120.0), *f, 700.0, 0.12);
            }
            for f in [523.25, 659.25, 783.99, 1046.5, 1318.5] {
                bell(&mut o, ms(1100.0), f, 2200.0, 0.09);
            }
            for k in 0..40 {
                let at = 1200.0 + k as f32 * 55.0 + (k * 29 % 31) as f32;
                click(&mut o, ms(at), 0.12, 3000.0 + (k * 173 % 1100) as f32, 151 + k);
            }
        }
        // The ball round the wheel: a hum that rises and falls with each lap, slowing down.
        Cue::BallRoll => {
            let len = o.len();
            let mut n = Noise(171);
            let mut lp = 0.0f32;
            for (i, x) in o.iter_mut().enumerate() {
                let t = i as f32 / RATE;
                let f = len as f32 / RATE;
                let laps = 5.0 * t - 1.1 * t * t;
                let wobble = 0.65 + 0.35 * (TAU * laps).sin();
                let env = (t / 0.15).min(1.0) * (1.0 - t / f).max(0.0).powf(0.7);
                lp += (n.next() - lp) * 0.05;
                *x += (lp * 0.8 + (TAU * 210.0 * t).sin() * 0.05) * wobble * env * 0.5;
            }
        }
        // Clicks against the frets, closer together, then still.
        Cue::BallDrop => {
            let mut t = 0.0f32;
            let mut gap = 140.0f32;
            let mut k = 0;
            while t < 820.0 && gap > 12.0 {
                click(&mut o, ms(t), 0.3 * (1.0 - t / 1100.0), 2600.0 + (k * 131 % 500) as f32, 191 + k);
                t += gap;
                gap *= 0.72;
                k += 1;
            }
        }
        // A rattle in the hand, then two dice bouncing down the felt.
        Cue::DiceRoll => {
            for k in 0..9 {
                click(&mut o, ms(k as f32 * 38.0 + (k * 13 % 11) as f32), 0.22, 1700.0 + (k * 97 % 400) as f32, 211 + k);
            }
            for (k, (at, a)) in [(420.0, 0.55), (560.0, 0.42), (690.0, 0.3), (790.0, 0.2), (860.0, 0.12)].iter().enumerate() {
                knock(&mut o, ms(*at), *a);
                click(&mut o, ms(*at + 6.0), *a * 0.5, 1900.0, 231 + k as u32);
            }
        }
        // A soft pop.
        Cue::KenoBall => {
            let len = o.len();
            for (i, x) in o.iter_mut().enumerate().take(len) {
                let t = i as f32 / RATE;
                let f = 520.0 + 380.0 * (-t * 30.0).exp();
                *x += (TAU * f * t).sin() * (-t * 28.0).exp() * 0.3;
            }
        }
        Cue::Coin => {
            bell(&mut o, 0, 2793.8, 120.0, 0.08);
            click(&mut o, 0, 0.1, 3400.0, 241);
        }
        // The wheel's pegs clicking past the pointer, slower and slower, then a ding.
        Cue::Wheel => {
            let mut t = 0.0f32;
            let mut gap = 34.0f32;
            let mut k = 0u32;
            while t < 4000.0 {
                click(&mut o, ms(t), 0.2, 2300.0 + (k * 53 % 300) as f32, 301 + k);
                t += gap;
                gap *= 1.045;
                k += 1;
            }
            bell(&mut o, ms(4080.0), 1046.5, 500.0, 0.16);
            bell(&mut o, ms(4080.0), 1568.0, 500.0, 0.1);
        }
        // A lit fuse: crackling, hissing noise that flickers.
        Cue::Fuse => {
            let mut n = Noise(313);
            let (mut lp, mut prev) = (0.0f32, 0.0f32);
            let len = o.len();
            for (i, x) in o.iter_mut().enumerate().take(len) {
                let t = i as f32 / len as f32;
                let env = (t / 0.05).min(1.0) * (1.0 - t).powf(0.8);
                let v = n.next();
                lp += (v - lp) * 0.55;
                let hp = lp - prev;
                prev = lp;
                let flicker = 0.6 + 0.4 * ((i as f32 / RATE * 37.0).sin() * (i as f32 / RATE * 11.0).sin()).abs();
                let crackle = if n.next() > 0.985 { n.next() * 0.8 } else { 0.0 };
                *x += (hp * 0.35 * flicker + crackle) * env * 0.6;
            }
        }
        // The bomb: a deep thud falling away, a roar of noise, then crackles as it settles.
        Cue::Boom => {
            let mut n = Noise(331);
            let mut lp = 0.0f32;
            let mut lp2 = 0.0f32;
            let len = o.len();
            for (i, x) in o.iter_mut().enumerate().take(len) {
                let t = i as f32 / RATE;
                let f = 32.0 + 70.0 * (-t * 7.0).exp();
                let thud = (TAU * f * t).sin() * (-t * 2.2).exp() * (t / 0.004).min(1.0);
                let v = n.next();
                lp += (v - lp) * (0.32 * (-t * 1.6).exp() + 0.02);
                lp2 += (lp - lp2) * 0.25;
                let roar = lp2 * (-t * 1.9).exp() * 1.6;
                let crackle = if t > 0.25 && n.next() > 0.993 { n.next() * 0.4 * (-t * 1.2).exp() } else { 0.0 };
                *x += thud * 0.85 + roar + crackle;
            }
        }
        // Out of time: two knocks and a falling tone.
        Cue::TimeOut => {
            knock(&mut o, 0, 0.75);
            knock(&mut o, ms(120.0), 0.75);
            bell(&mut o, ms(230.0), 659.25, 450.0, 0.15);
            bell(&mut o, ms(380.0), 440.0, 420.0, 0.15);
        }
    }
    // Never past full scale.
    let peak = o.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if peak > 0.9 {
        for x in &mut o {
            *x *= 0.9 / peak;
        }
    }
    o
}

/// The sound, 48 kHz stereo (interleaved), placed `pan` (-1 left to 1 right).
pub fn render(c: Cue, pan: f32) -> Vec<f32> {
    let pan = pan.clamp(-1.0, 1.0);
    let (l, r) = (((1.0 - pan) * 0.5).sqrt(), ((1.0 + pan) * 0.5).sqrt());
    render_mono(c).into_iter().flat_map(|x| [x * l, x * r]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_is_there_and_never_clips() {
        for c in [Cue::Deal, Cue::Flip, Cue::Chips, Cue::Check, Cue::Fold, Cue::AllIn, Cue::Pot, Cue::Turn, Cue::Win, Cue::Hurry, Cue::Tick, Cue::TimeBank, Cue::TimeOut, Cue::ReelSpin, Cue::ReelStop, Cue::WinSmall, Cue::WinBig, Cue::Jackpot, Cue::BallRoll, Cue::BallDrop, Cue::DiceRoll, Cue::KenoBall, Cue::Coin, Cue::Wheel, Cue::Fuse, Cue::Boom] {
            let s = render(c, 0.3);
            assert!(s.len() > 2000, "{c:?}");
            let peak = s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak > 0.02 && peak <= 0.91, "{c:?} {peak}");
            assert!(s.iter().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn panned_left_is_louder_on_the_left() {
        let s = render(Cue::Chips, -0.8);
        let (l, r): (f32, f32) = s.chunks(2).fold((0.0, 0.0), |(a, b), x| (a + x[0].abs(), b + x[1].abs()));
        assert!(l > r * 2.0);
    }
}
