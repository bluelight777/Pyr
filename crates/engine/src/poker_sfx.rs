//! The sounds of the poker table and the casino's games. The table's own (cards, chips, the
//! check's knock, the pot, the win, and the turn, clock and time bank bells) are 0.13.6's, made
//! fresh each time in `table_sfx`; the casino's games play 0.13.28's own files (`bands`: the
//! casino's `c_*`, the slots' bands) as it does; Plinko's pegs and buckets are made below. 48 kHz
//! stereo, a little to the side they happen on.

use std::f32::consts::TAU;

const RATE: f32 = 48_000.0;

/// Roulette's timing, in seconds from the launch, shared by the wheel on screen and its
/// sounds so the clicks land with the ball. Measured against a real wheel's: the ball goes
/// round the track a good six times, slowing (fast at first, air and friction, then more
/// gently), leaves it, hits a diamond on the way down the cone, and hops a few pockets before it
/// settles; the wheel keeps turning the whole time and after. (Until 0.10.0 the whole spin took
/// 5.2 s, the ball fell in after 2.3, and the wheel braked to a dead stop with it.)
pub mod roulette {
    /// The ball leaves the track.
    pub const LEAVE_S: f32 = 5.6;
    /// It hits a diamond on the way down the cone.
    pub const DIAMOND_S: f32 = LEAVE_S + 0.42;
    /// It drops into the pockets the first time.
    pub const POCKETS_S: f32 = LEAVE_S + 0.7;
    /// Its hops from pocket to pocket: (seconds in the air, how high, of the wheel's radius).
    pub const HOPS: [(f32, f32); 4] = [(0.40, 0.10), (0.32, 0.07), (0.25, 0.045), (0.18, 0.025)];
    /// Still in its pocket (a little rattle after the last hop).
    pub const SETTLED_S: f32 = POCKETS_S + 0.40 + 0.32 + 0.25 + 0.18 + 0.12;
    /// The number's called.
    pub const RESULT_S: f32 = SETTLED_S + 0.35;

    // The ball's speed round the wheel (rad/s): A·e^(-t/τ) + B, 13.5 at the launch and 3.4 as
    // it leaves the track.
    const TAU_S: f32 = 3.2;
    const A: f32 = 12.225;
    const B: f32 = 1.275;

    /// The ball's speed (rad/s, against the table) at `t`.
    pub fn ball_speed(t: f32) -> f32 {
        A * (-t / TAU_S).exp() + B
    }

    /// How far the ball has gone round (radians) by `t`.
    pub fn ball_run(t: f32) -> f32 {
        B * t + A * TAU_S * (1.0 - (-t / TAU_S).exp())
    }

    /// When each hop lands (seconds from the launch).
    pub fn landings() -> [f32; 4] {
        let mut at = POCKETS_S;
        let mut out = [0.0; 4];
        for (i, (d, _)) in HOPS.iter().enumerate() {
            at += d;
            out[i] = at;
        }
        out
    }
}

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
    /// The tick three semitones higher (A6 to the tick's F#6): 0.13.28 ticks a turn's last seconds
    /// in one of the two, and its time bank's in the other (see the table's sounds); Ember
    /// Respins' empty respins tick this one.
    TickUp,
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
    // Bomb Pots.
    /// The prize wheel: a ratchet slowing to a stop, and a ding.
    Wheel,
    /// The fuse burning down: a sizzle.
    Fuse,
    /// The bomb going off.
    Boom,
    // Plinko (0.13.5).
    /// A ball striking a gold peg on this row (from the top): a small bright ring, half a
    /// semitone higher each row down.
    Peg(u8),
    /// A ball dropping into a bucket this many from the middle: a thump and a ding, a step and a
    /// half higher (and a little louder) each bucket out.
    Bucket(u8),
    /// One of the Reel Row machines' own sounds (`bands::BANDS`, 0.13.28's files).
    Band(u16),
    /// One of those, played this many hundredths of a semitone higher (faster; lower when
    /// negative), as 0.13.28's big wins play theirs.
    BandUp(u16, i16),
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

/// The table's own sounds as 0.13.6 made them new (chips that ring, cards that slide and snap, the
/// felt, the rail, a better win; the turn, hurry, tick and time bank bells with them): a fresh one
/// each time, as 0.13.28 makes them (`table_sfx`). Kept under 0.9 as every sound here is.
fn table_sound(c: Cue) -> Option<Vec<f32>> {
    use crate::table_sfx as t;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    let mut rng = t::Rng::new(now ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    let mut o = match c {
        Cue::Deal => t::deal(&mut rng),
        Cue::Flip => t::street(&mut rng),
        Cue::Chips => t::chips(&mut rng),
        Cue::Check => t::check(&mut rng),
        Cue::Fold => t::fold(&mut rng),
        Cue::AllIn => t::all_in(&mut rng),
        Cue::Pot => t::pot(&mut rng),
        Cue::Turn => t::turn(),
        Cue::Win => t::win(&mut rng),
        Cue::Hurry => t::hurry(),
        Cue::Tick => t::tick(1.0, &mut rng),
        Cue::TickUp => t::tick(2f32.powf(3.0 / 12.0), &mut rng),
        Cue::TimeBank => t::time_bank(),
        Cue::TimeOut => t::time_out(&mut rng),
        _ => return None,
    };
    let peak = o.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    if peak > 0.9 {
        for x in &mut o {
            *x *= 0.9 / peak;
        }
    }
    Some(o)
}

/// What 0.13.28 plays from its own files (the casino's `c_<cue>`, carved out of it: `bands`) for the
/// casino's cues: every game's but Plinko's pegs and buckets, made here, and the poker table's own
/// (chips, cards, the felt), made fresh each time (`table_sfx`) wherever they're heard. Heard off
/// 0.13.28 at dice, Keno, roulette and craps; the slots' own band replaces some at the slots.
pub fn casino_file(c: Cue) -> Option<&'static str> {
    Some(match c {
        Cue::ReelSpin => "c_reelspin",
        Cue::ReelStop => "c_reelstop",
        Cue::WinSmall => "c_winsmall",
        Cue::WinBig => "c_winbig",
        Cue::Jackpot => "c_jackpot",
        Cue::BallRoll => "c_ballroll",
        Cue::BallDrop => "c_balldrop",
        Cue::DiceRoll => "c_diceroll",
        Cue::KenoBall => "c_kenoball",
        Cue::Coin => "c_coin",
        Cue::Wheel => "c_wheel",
        Cue::Fuse => "c_fuse",
        Cue::Boom => "c_boom",
        _ => return None,
    })
}

/// The sound, mono.
pub fn render_mono(c: Cue) -> Vec<f32> {
    if let Some(o) = table_sound(c) {
        return o;
    }
    if let Some(i) = casino_file(c).and_then(crate::bands::id) {
        return crate::bands::render(i, 0.0).chunks(2).map(|x| (x[0] + x[1]) * 0.5).collect();
    }
    if let Cue::Band(i) = c {
        return crate::bands::render(i, 0.0).chunks(2).map(|x| (x[0] + x[1]) * 0.5).collect();
    }
    if let Cue::BandUp(i, up) = c {
        return crate::bands::render_up(i, up, 0.0).chunks(2).map(|x| (x[0] + x[1]) * 0.5).collect();
    }
    let len = match c {
        Cue::Peg(_) => ms(90.0),
        Cue::Bucket(_) => ms(390.0),
        _ => 0,
    };
    let mut o = vec![0.0f32; len];
    match c {
        // As 0.13.28 makes them, sample for sample (fitted to what it played, every row and
        // bucket heard: a G6 bell with a click a fifth over it, up half a semitone a row; a knock
        // and a C5 bell, up a step and a half a bucket from the middle).
        Cue::Peg(row) => {
            let f = 1567.98 * 2f32.powf(row as f32 / 24.0);
            bell(&mut o, 0, f, 90.0, 0.05);
            click(&mut o, 0, 0.09, f * 1.5, 91 + row as u32);
        }
        Cue::Bucket(k) => {
            knock(&mut o, 0, 0.5);
            bell(&mut o, ms(10.0), 523.25 * 2f32.powf(1.5 * k as f32 / 12.0), 380.0, 0.1 + 0.02 * k as f32);
        }
        _ => {}
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
    if let Some(i) = casino_file(c).and_then(crate::bands::id) {
        return crate::bands::render(i, pan);
    }
    if let Cue::Band(i) = c {
        return crate::bands::render(i, pan);
    }
    if let Cue::BandUp(i, up) = c {
        return crate::bands::render_up(i, up, pan);
    }
    let pan = pan.clamp(-1.0, 1.0);
    let (l, r) = (((1.0 - pan) * 0.5).sqrt(), ((1.0 + pan) * 0.5).sqrt());
    render_mono(c).into_iter().flat_map(|x| [x * l, x * r]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_is_there_and_never_clips() {
        for c in [Cue::Deal, Cue::Flip, Cue::Chips, Cue::Check, Cue::Fold, Cue::AllIn, Cue::Pot, Cue::Turn, Cue::Win, Cue::Hurry, Cue::Tick, Cue::TickUp, Cue::TimeBank, Cue::TimeOut, Cue::ReelSpin, Cue::ReelStop, Cue::WinSmall, Cue::WinBig, Cue::Jackpot, Cue::BallRoll, Cue::BallDrop, Cue::DiceRoll, Cue::KenoBall, Cue::Coin, Cue::Wheel, Cue::Fuse, Cue::Boom, Cue::Peg(0), Cue::Peg(15), Cue::Bucket(0), Cue::Bucket(8)] {
            let s = render(c, 0.3);
            assert!(s.len() > 2000, "{c:?}");
            let peak = s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak > 0.02 && peak <= 0.91, "{c:?} {peak}");
            assert!(s.iter().all(|x| x.is_finite()));
        }
    }

    #[test]
    fn the_tables_sounds_are_new_each_time_and_its_bells_the_same() {
        for c in [Cue::Chips, Cue::Check, Cue::Fold, Cue::Deal, Cue::Flip, Cue::Pot, Cue::AllIn, Cue::Win, Cue::Tick, Cue::TickUp] {
            let (a, b) = (render_mono(c), render_mono(c));
            assert!(a != b, "{c:?}");
            assert!(a.iter().all(|x| x.is_finite() && x.abs() <= 0.9), "{c:?}");
        }
        for c in [Cue::Turn, Cue::Hurry, Cue::TimeBank] {
            assert_eq!(render_mono(c), render_mono(c), "{c:?}");
        }
    }

    #[test]
    fn panned_left_is_louder_on_the_left() {
        let s = render(Cue::Chips, -0.8);
        let (l, r): (f32, f32) = s.chunks(2).fold((0.0, 0.0), |(a, b), x| (a + x[0].abs(), b + x[1].abs()));
        assert!(l > r * 2.0);
    }

    /// The casino's games play 0.13.28's files, as it plays them: turned down to 0.9 when louder,
    /// the middle full on both sides, and the other side turned down 0.7 of the pan (craps' dice
    /// from the shooter's side, 0.3: 0.79).
    #[test]
    fn the_casinos_sounds_are_0_13_28s_files() {
        for c in [Cue::ReelSpin, Cue::ReelStop, Cue::WinSmall, Cue::WinBig, Cue::Jackpot, Cue::BallRoll, Cue::BallDrop, Cue::DiceRoll, Cue::KenoBall, Cue::Coin, Cue::Wheel, Cue::Fuse, Cue::Boom] {
            let i = crate::bands::id(casino_file(c).unwrap()).unwrap_or_else(|| panic!("{c:?}"));
            assert_eq!(render(c, 0.0), crate::bands::render(i, 0.0), "{c:?}");
        }
        let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!((peak(&render(Cue::DiceRoll, 0.0)) - 0.9).abs() < 1e-6);
        let side = |s: &[f32]| s.chunks(2).fold((0.0f64, 0.0f64), |(a, b), x| (a + (x[0] as f64).powi(2), b + (x[1] as f64).powi(2)));
        let (l0, r0) = side(&render(Cue::DiceRoll, 0.0));
        let (l, r) = side(&render(Cue::DiceRoll, 0.3));
        assert!(((l / l0).sqrt() - 0.79).abs() < 1e-4 && (r / r0 - 1.0).abs() < 1e-9);
        assert!(casino_file(Cue::Chips).is_none() && casino_file(Cue::Peg(0)).is_none());
    }

    /// Plinko's pegs ring half a semitone higher a row down; its buckets a step and a half higher
    /// and a little louder each bucket out from the middle.
    #[test]
    fn plinkos_pegs_climb_by_row_and_its_buckets_by_place() {
        let pitch = |s: &[f32], lo: f32, hi: f32| {
            // (The strongest frequency between lo and hi, by a plain search, 1 Hz steps.)
            let n = s.len().min(4800);
            let mut best = (0.0f32, lo);
            let mut f = lo;
            while f < hi {
                let (mut re, mut im) = (0.0f32, 0.0f32);
                for (i, x) in s[..n].iter().enumerate() {
                    let a = TAU * f * i as f32 / RATE;
                    re += x * a.cos();
                    im += x * a.sin();
                }
                if re * re + im * im > best.0 {
                    best = (re * re + im * im, f);
                }
                f += 1.0;
            }
            best.1
        };
        let p0 = pitch(&render_mono(Cue::Peg(0)), 1400.0, 1700.0);
        let p8 = pitch(&render_mono(Cue::Peg(8)), 1800.0, 2100.0);
        assert!((p0 - 1568.0).abs() < 3.0 && (p8 - 1975.5).abs() < 3.0, "{p0} {p8}");
        let b0 = pitch(&render_mono(Cue::Bucket(0))[480..], 450.0, 750.0);
        let b2 = pitch(&render_mono(Cue::Bucket(2))[480..], 450.0, 750.0);
        assert!((b0 - 523.0).abs() < 3.0 && (b2 - 622.0).abs() < 3.0, "{b0} {b2}");
        assert_eq!(render_mono(Cue::Peg(3)), render_mono(Cue::Peg(3)));
    }
}
