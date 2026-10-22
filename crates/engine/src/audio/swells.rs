//! Noticing my voice going up and down for the others while I talk: the gain my microphone's
//! processing gives each 200 ms of my talking (what goes out over what came in), and when it's
//! turned down 10 dB or more and back up again and again, which part of the processing did it.
//!
//! Gain, not loudness: talking itself gets louder and softer all the time, but the processing
//! should give it about the same gain throughout (measured: within ±3 dB on steady speech in
//! every noise mode). Noise suppression's own gain isn't counted: it follows what's in the
//! sound (it takes more off a hiss than a vowel). Only the middle of sentences counts (a
//! 200 ms stretch of talking that follows another), so a sentence's start, which voice
//! isolation opens on, isn't counted.

use std::collections::VecDeque;

/// 10 ms frames in a stretch.
const STRETCH: usize = 20;
/// Quieter than this coming in isn't talking.
const TALKING_DB: f32 = -50.0;
/// Turned down: this much under the usual gain.
const DIP_DB: f32 = 10.0;
/// How many times turned down and back up within `RECENT` stretches (a minute) make it
/// "up and down".
const DIPS: usize = 3;
const RECENT: usize = 300;

/// The parts of the processing, in order.
pub const STAGES: [&str; 3] = ["echo cancellation and automatic gain", "noise suppression", "voice isolation"];

#[derive(Default)]
pub struct Swells {
    /// This stretch so far: energy coming in, after each stage, and frames sent.
    sums: [f64; 4],
    frames: usize,
    open: usize,
    /// Frames the speakers' sound could account for much of what came in.
    far: usize,
    /// Recent stretches: the gain of each stage (dB) and the level coming in, when it was
    /// talking (and the one before).
    recent: VecDeque<Option<[f32; 4]>>,
    last_talking: bool,
    /// Stretches since it was last noticed (so it's said once in a while, not every second).
    since_told: usize,
    told: bool,
}

fn db(e: f64) -> f32 {
    if e > 1e-12 { (10.0 * e.log10()) as f32 } else { -120.0 }
}

impl Swells {
    /// One 10 ms frame's levels (dBFS): coming in, after echo cancellation and automatic gain,
    /// after noise suppression, and going out; whether it was sent; and whether the speakers'
    /// sound could account for much of what came in (then the echo canceller taking it out
    /// isn't my voice turned down: a stretch with much of that isn't counted). Says what
    /// happened when my voice has been going up and down.
    pub fn feed(&mut self, levels: [f32; 4], sent: bool, far: bool) -> Option<String> {
        for (s, l) in self.sums.iter_mut().zip(levels) {
            *s += 10f64.powf(l as f64 / 10.0);
        }
        self.frames += 1;
        self.open += sent as usize;
        self.far += far as usize;
        if self.frames < STRETCH {
            return None;
        }
        let e: Vec<f32> = self.sums.iter().map(|s| db(s / STRETCH as f64)).collect();
        let talking = self.open == STRETCH && e[0] > TALKING_DB && self.far <= STRETCH / 4;
        let gains = (talking && self.last_talking).then(|| [e[1] - e[0], e[2] - e[1], e[3] - e[2], e[0]]);
        self.last_talking = talking;
        self.sums = [0.0; 4];
        self.frames = 0;
        self.open = 0;
        self.far = 0;
        self.recent.push_back(gains);
        if self.recent.len() > RECENT {
            self.recent.pop_front();
        }
        self.since_told += 1;
        // (Once a minute at most.)
        if self.told && self.since_told < RECENT {
            return None;
        }
        // My voice turned down in the middle of my talking, then back up: stretches whose gain
        // is `DIP_DB` under the usual. (Noise suppression's own gain follows what's in the
        // sound, a hiss or a consonant, and varies by itself: what's watched is what turns my
        // voice down, automatic gain and voice isolation.)
        let talking: Vec<[f32; 4]> = self.recent.iter().flatten().copied().collect();
        if talking.len() < 20 {
            return None;
        }
        let usual = |i: usize| {
            let mut v: Vec<f32> = talking.iter().map(|g| g[i]).collect();
            v.sort_by(f32::total_cmp);
            v[v.len() / 2]
        };
        let usual = [usual(0), usual(1), usual(2), usual(3)];
        let mut dips = 0;
        let mut deepest = 0f32;
        let mut moved = [0f32; 3];
        let mut in_dip = false;
        for g in self.recent.iter() {
            let Some(g) = g else {
                in_dip = false;
                continue;
            };
            // (Only talking as loud as usual: noise suppression in the echo canceller's stage
            // takes the quiet ends of words down, and that's not my voice being turned down.)
            let under = (usual[0] + usual[2]) - (g[0] + g[2]);
            if under >= DIP_DB && g[3] > usual[3] - 6.0 {
                if !in_dip {
                    dips += 1;
                }
                in_dip = true;
                deepest = deepest.max(under);
                for i in 0..3 {
                    moved[i] += (usual[i] - g[i]).max(0.0);
                }
            } else {
                in_dip = false;
            }
        }
        if dips < DIPS {
            return None;
        }
        let worst = if moved[2] >= moved[0] { 2 } else { 0 };
        self.since_told = 0;
        self.told = true;
        self.recent.clear();
        Some(format!(
            "my voice went down and back up for the others {dips} times within {} s of talking (by up to {deepest:.0} dB), turned down by the {}",
            RECENT * STRETCH / 100,
            STAGES[worst]
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn talk(sw: &mut Swells, secs: usize, iso_db: impl Fn(usize) -> f32) -> Vec<String> {
        let mut said = Vec::new();
        for f in 0..secs * 100 {
            // Talking that itself gets louder and softer (a syllable every 150 ms).
            let level = -30.0 + 8.0 * ((f as f32) * 0.42).sin();
            let iso = iso_db(f);
            if let Some(s) = sw.feed([level, level + 5.0, level + 4.0, level + 4.0 + iso], true, false) {
                said.push(s);
            }
        }
        said
    }

    #[test]
    fn steady_processing_of_lively_talking_isnt_a_swell() {
        let mut sw = Swells::default();
        assert!(talk(&mut sw, 60, |_| 0.0).is_empty());
    }

    #[test]
    fn echo_taken_out_while_the_speakers_play_isnt_a_swell() {
        let mut sw = Swells::default();
        // Every 1.6 s, 0.6 s of the speakers' sound coming in as loud as my talking, taken out
        // by the echo canceller (30 dB under what came in).
        let mut said = Vec::new();
        for f in 0..3000 {
            let level = -30.0 + 8.0 * ((f as f32) * 0.42).sin();
            let far = f % 160 < 60;
            let apm = if far { level - 30.0 } else { level + 5.0 };
            if let Some(s) = sw.feed([level, apm, apm - 1.0, apm - 1.0], true, far) {
                said.push(s);
            }
        }
        assert!(said.is_empty(), "{said:?}");
    }

    #[test]
    fn isolation_turning_the_voice_down_and_up_is_noticed_and_named() {
        let mut sw = Swells::default();
        // Down 26 dB for 0.6 s every 1.6 s.
        let said = talk(&mut sw, 30, |f| if f % 900 < 60 { -26.0 } else { 0.0 });
        assert_eq!(said.len(), 1, "said once: {said:?}");
        assert!(said[0].contains("by the voice isolation"), "{}", said[0]);
    }
}
