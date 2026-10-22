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

/// How long landing takes once it starts (for when the last reel is still).
pub fn land_secs(turbo: bool) -> f32 {
    if turbo { 0.32 } else { 0.5 }
}
