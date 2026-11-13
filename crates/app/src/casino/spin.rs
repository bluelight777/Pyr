//! Reels in motion, for every slot machine: each reel's place on its strip as time goes by,
//! spinning at a steady speed, then landing on its stop with a little overshoot, one reel after
//! another from the left.

use std::time::{Duration, Instant};

use super::ease_back;

/// Reels spin this many symbols a second.
pub const SPEED: f32 = 26.0;

#[derive(Clone, Copy, Debug)]
pub struct Reel {
    /// The strip position of the top row at `t0`, and how fast it's moving (symbols a second,
    /// downwards).
    pub pos0: f32,
    pub t0: Instant,
    pub speed: f32,
    /// Landing: from when, from where, to where, how long.
    pub land: Option<(Instant, f32, f32, f32)>,
}

impl Reel {
    pub fn still(pos: f32) -> Reel {
        Reel { pos0: pos, t0: Instant::now(), speed: 0.0, land: None }
    }

    /// The top row's place on the strip at `now` (a fraction between symbols while it moves).
    pub fn pos(&self, now: Instant) -> f32 {
        if let Some((start, from, to, dur)) = self.land
            && now >= start
        {
            let k = ((now - start).as_secs_f32() / dur).min(1.0);
            return from + (to - from) * ease_back(k);
        }
        self.pos0 - self.speed * now.saturating_duration_since(self.t0).as_secs_f32()
    }

    pub fn landed(&self, now: Instant) -> bool {
        self.land.is_some_and(|(start, _, _, dur)| now >= start + Duration::from_secs_f32(dur))
    }

    /// Moving fast (not yet slowing into its stop): drawn blurred.
    pub fn blurred(&self, now: Instant) -> bool {
        self.speed > 0.0 && !self.landed(now) && self.land.is_none_or(|(st, ..)| now < st)
    }

    /// Sets off spinning from wherever it is at `now` (at `speed` symbols a second).
    pub fn start(&mut self, now: Instant, speed: f32) {
        let pos = self.pos(now);
        *self = Reel { pos0: pos, t0: now, speed, land: None };
    }
}

/// Lands each reel on its stop (`stops[i]` on a strip `lens[i]` long), one after another from
/// `from`: when each comes to rest. (Turbo: quicker.)
pub fn land(reels: &mut [Reel], lens: &[usize], stops: &[usize], from: Instant, turbo: bool) -> Vec<Instant> {
    let gap = if turbo { 0.08 } else { 0.24 };
    let dur = if turbo { 0.32 } else { 0.5 };
    let mut ends = vec![from; reels.len()];
    let mut last = from;
    for (i, r) in reels.iter_mut().enumerate() {
        let len = lens[i].max(1) as f32;
        // (Never before the reel to its left.)
        let want = (from + Duration::from_secs_f32(i as f32 * gap)).max(last - Duration::from_secs_f32(dur * 0.6));
        // Land when the reel is a few symbols above its stop, so it slows down into it.
        let lead = 4.0;
        let p_want = r.pos(want);
        let ahead = (p_want - stops[i] as f32 - lead).rem_euclid(len);
        let start = want + Duration::from_secs_f32(ahead / r.speed.max(1.0));
        let p_start = r.pos(start);
        let to = p_start - lead;
        r.land = Some((start, p_start, to, dur));
        ends[i] = start + Duration::from_secs_f32(dur * 0.55);
        last = last.max(start + Duration::from_secs_f32(dur));
    }
    ends
}

/// Lands each reel on its stop at once, one after another from `from` (Turbo, 0.13.2's "the
/// reels land almost at once"): a reel doesn't wait for its stop to come round on the strip; it's
/// put the right few symbols above it as it starts slowing (under the blur, so it isn't seen).
/// When each comes to rest. (Pyr Sevens' strips are 30 symbols and more: waiting for each stop
/// took up to a second a reel, and a Turbo round of free spins three times 0.13.28's.)
pub fn land_now(reels: &mut [Reel], lens: &[usize], stops: &[usize], from: Instant) -> Vec<Instant> {
    let (gap, dur, lead) = (0.08, land_secs(true), 4.0);
    let mut ends = Vec::with_capacity(reels.len());
    for (i, r) in reels.iter_mut().enumerate() {
        let len = lens[i].max(1) as f32;
        let start = from + Duration::from_secs_f32(i as f32 * gap);
        // (The place `lead` above the stop nearest where the strip's got to.)
        let target = stops[i] as f32 + lead;
        let p_start = target + ((r.pos(start) - target) / len).round() * len;
        r.land = Some((start, p_start, p_start - lead, dur));
        ends.push(start + Duration::from_secs_f32(dur * 0.55));
    }
    ends
}

/// Lands each reel as soon as it can, one after another from `from` (Turbo's pace), wherever its
/// strip has got to: when each comes to rest, and where on its strip (the caller puts the screen
/// there). Bomb Spins' free spins land this way, quickly, as 0.13.28's do.
pub fn land_soon(reels: &mut [Reel], lens: &[usize], from: Instant) -> (Vec<Instant>, Vec<usize>) {
    let (gap, dur, lead) = (0.08, 0.32, 4.0);
    let mut ends = Vec::with_capacity(reels.len());
    let mut stops = Vec::with_capacity(reels.len());
    for (i, r) in reels.iter_mut().enumerate() {
        let len = lens[i].max(1) as f32;
        let start = from + Duration::from_secs_f32(i as f32 * gap);
        let p_start = r.pos(start);
        // (Onto the next whole place `lead` symbols on.)
        let to = (p_start - lead).floor();
        r.land = Some((start, p_start, to, dur));
        ends.push(start + Duration::from_secs_f32(dur * 0.55));
        stops.push(to.rem_euclid(len) as usize);
    }
    (ends, stops)
}

/// How long landing takes once it starts (for when the last reel is still).
pub fn land_secs(turbo: bool) -> f32 {
    if turbo { 0.32 } else { 0.5 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Turbo lands on the stops at once, wherever the strips have got to.
    #[test]
    fn turbo_lands_at_once_on_the_stops() {
        let t0 = Instant::now();
        let mut reels: Vec<Reel> = (0..5).map(|i| Reel { pos0: i as f32 * 7.3, t0, speed: SPEED, land: None }).collect();
        let lens = [30, 32, 32, 32, 30];
        let stops = [3, 31, 0, 17, 29];
        let from = t0 + Duration::from_millis(250);
        let ends = land_now(&mut reels, &lens, &stops, from);
        let done = from + Duration::from_secs(2);
        for i in 0..5 {
            assert!(ends[i] <= from + Duration::from_secs_f32(0.5), "reel {i}");
            assert_eq!((reels[i].pos(done).round() as i64).rem_euclid(lens[i] as i64) as usize, stops[i], "reel {i}");
        }
    }
}

/// Pyr Sevens' and Ember Dragon's sounds since 0.13.2 ("new slot sounds"): the slots' shared band
/// (`s_all_*`, 0.13.28's own files), as 0.13.28 plays them, measured off its output (each sound's
/// file, pitch and loudness, at Pyr's sounds 70% and the games' 100%: 50 paid spins and 40 free
/// ones of each). (cue, volume) pairs, for `CasinoUi::sound_in`.
pub mod band {
    use pyr_engine::poker_sfx::Cue;

    /// The band's `s_all_<what>`, `cents` hundredths of a semitone higher.
    pub fn cue(what: &str, cents: i32) -> Cue {
        super::super::bigwin::cue(&format!("s_all_{what}"), cents).unwrap_or(Cue::ReelStop)
    }

    /// The reels set going (a free spin's a little softer).
    pub fn spin(free: bool) -> (Cue, f32) {
        (cue("reelspin", 0), if free { 0.25 } else { 0.3 })
    }

    /// Reel `i` landing: up the pentatonic reel by reel (0, 2, 4, 7 and 9 semitones), placed under
    /// its reel as before.
    pub fn stop(i: usize) -> (Cue, f32) {
        (cue("reelstop", [0, 200, 400, 700, 900][i.min(4)]), 0.294)
    }

    /// A spin's win: Pyr Sevens' big one (five times the bet and more) is the band's jackpot.
    pub fn win(big: bool) -> (Cue, f32) {
        if big { (cue("jackpot", 0), 0.595) } else { (cue("winsmall", 0), 0.385) }
    }

    /// Free spins won (with the spin's own win, if it paid).
    pub fn free_spins() -> (Cue, f32) {
        (cue("jackpot", 0), 0.475)
    }

    /// Ember Dragon growing up a stage.
    pub fn grew() -> (Cue, f32) {
        (cue("winbig", 0), 0.56)
    }

    /// Ember Respins starting.
    pub fn respins() -> (Cue, f32) {
        (cue("jackpot", 0), 0.664)
    }

    /// A respin that catches fireballs (one that catches none ticks, the table's high tick).
    pub fn caught() -> (Cue, f32) {
        (cue("coin", 0), 0.434)
    }

    /// The respins over: the band's big win, or its jackpot when one's won. (The big win heard,
    /// 0.63; the jackpot's as loud as its other plays make it, not heard.) Ember Dragon's big
    /// spin win, ten times the bet and more, is the big win at 0.49 (not heard either).
    pub fn ended(jackpot: bool) -> (Cue, f32) {
        if jackpot { (cue("jackpot", 0), 0.855) } else { (cue("winbig", 0), 0.63) }
    }

    pub fn dragon_big_win() -> (Cue, f32) {
        (cue("winbig", 0), 0.49)
    }
}
