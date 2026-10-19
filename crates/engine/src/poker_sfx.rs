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
        Cue::Deal | Cue::Flip | Cue::Check | Cue::Hurry => ms(220.0),
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
        for c in [Cue::Deal, Cue::Flip, Cue::Chips, Cue::Check, Cue::Fold, Cue::AllIn, Cue::Pot, Cue::Turn, Cue::Win, Cue::Hurry] {
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
