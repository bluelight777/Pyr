//! Reel Row's machines' own sounds (Reel Row Remastered, 0.13.2): each machine's band
//! (`s_<machine>_spin`, `_stop`, `_tease`, `_scatter`, `_trigger`, `_win0` to `_win2`, `_buy`, and
//! its feature's: `_pop`, `_drop`, `_bomb`, `_bombadd`, `_multiply` on the tumbling machines;
//! `_coin`, `_collect`, `_reset`, `_jackpot`, `_grand` on the Hold & Win ones; `_grow`,
//! `_freeze`, `_step` on the wild ones) and the ones they share (`s_all_*`). 0.13.28's own files,
//! carved out of it (`rebuild/sounds-0.13.28`), each decoded once when it's first played.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;

use crate::soundboard::{self, Pcm};

macro_rules! band {
    ($n:literal) => {
        ($n, include_bytes!(concat!("../assets/bands/", $n, ".ings")) as &[u8])
    };
}

/// Every sound, by name. (And the casino's own, 0.13.6's, `c_*`: those a big win plays.)
pub const BANDS: &[(&str, &[u8])] = &[
    band!("c_boom"),
    band!("c_coin"),
    band!("c_tierup"),
    band!("c_winbig"),
    band!("c_winslam"),
    band!("s_all_coin"),
    band!("s_all_count"),
    band!("s_all_jackpot"),
    band!("s_all_reelspin"),
    band!("s_all_reelstop"),
    band!("s_all_tierup"),
    band!("s_all_winbig"),
    band!("s_all_winslam"),
    band!("s_all_winsmall"),
    band!("s_cabaret_buy"),
    band!("s_cabaret_scatter"),
    band!("s_cabaret_spin"),
    band!("s_cabaret_step"),
    band!("s_cabaret_stop"),
    band!("s_cabaret_tease"),
    band!("s_cabaret_trigger"),
    band!("s_cabaret_win0"),
    band!("s_cabaret_win1"),
    band!("s_cabaret_win2"),
    band!("s_cauldron_bomb"),
    band!("s_cauldron_bombadd"),
    band!("s_cauldron_buy"),
    band!("s_cauldron_drop"),
    band!("s_cauldron_multiply"),
    band!("s_cauldron_pop"),
    band!("s_cauldron_scatter"),
    band!("s_cauldron_spin"),
    band!("s_cauldron_stop"),
    band!("s_cauldron_tease"),
    band!("s_cauldron_trigger"),
    band!("s_cauldron_win0"),
    band!("s_cauldron_win1"),
    band!("s_cauldron_win2"),
    band!("s_cosmic_bomb"),
    band!("s_cosmic_bombadd"),
    band!("s_cosmic_buy"),
    band!("s_cosmic_drop"),
    band!("s_cosmic_multiply"),
    band!("s_cosmic_pop"),
    band!("s_cosmic_scatter"),
    band!("s_cosmic_spin"),
    band!("s_cosmic_stop"),
    band!("s_cosmic_tease"),
    band!("s_cosmic_trigger"),
    band!("s_cosmic_win0"),
    band!("s_cosmic_win1"),
    band!("s_cosmic_win2"),
    band!("s_deepsea_buy"),
    band!("s_deepsea_coin"),
    band!("s_deepsea_collect"),
    band!("s_deepsea_grand"),
    band!("s_deepsea_jackpot"),
    band!("s_deepsea_reset"),
    band!("s_deepsea_scatter"),
    band!("s_deepsea_spin"),
    band!("s_deepsea_stop"),
    band!("s_deepsea_tease"),
    band!("s_deepsea_trigger"),
    band!("s_deepsea_win0"),
    band!("s_deepsea_win1"),
    band!("s_deepsea_win2"),
    band!("s_frost_buy"),
    band!("s_frost_freeze"),
    band!("s_frost_scatter"),
    band!("s_frost_spin"),
    band!("s_frost_stop"),
    band!("s_frost_tease"),
    band!("s_frost_trigger"),
    band!("s_frost_win0"),
    band!("s_frost_win1"),
    band!("s_frost_win2"),
    band!("s_goldrush_buy"),
    band!("s_goldrush_coin"),
    band!("s_goldrush_collect"),
    band!("s_goldrush_grand"),
    band!("s_goldrush_jackpot"),
    band!("s_goldrush_reset"),
    band!("s_goldrush_scatter"),
    band!("s_goldrush_spin"),
    band!("s_goldrush_stop"),
    band!("s_goldrush_tease"),
    band!("s_goldrush_trigger"),
    band!("s_goldrush_win0"),
    band!("s_goldrush_win1"),
    band!("s_goldrush_win2"),
    band!("s_grind_buy"),
    band!("s_grind_grow"),
    band!("s_grind_scatter"),
    band!("s_grind_spin"),
    band!("s_grind_stop"),
    band!("s_grind_tease"),
    band!("s_grind_trigger"),
    band!("s_grind_win0"),
    band!("s_grind_win1"),
    band!("s_grind_win2"),
    band!("s_hoard_buy"),
    band!("s_hoard_coin"),
    band!("s_hoard_collect"),
    band!("s_hoard_grand"),
    band!("s_hoard_jackpot"),
    band!("s_hoard_reset"),
    band!("s_hoard_scatter"),
    band!("s_hoard_spin"),
    band!("s_hoard_stop"),
    band!("s_hoard_tease"),
    band!("s_hoard_trigger"),
    band!("s_hoard_win0"),
    band!("s_hoard_win1"),
    band!("s_hoard_win2"),
    band!("s_neon_bomb"),
    band!("s_neon_bombadd"),
    band!("s_neon_buy"),
    band!("s_neon_drop"),
    band!("s_neon_multiply"),
    band!("s_neon_pop"),
    band!("s_neon_scatter"),
    band!("s_neon_spin"),
    band!("s_neon_stop"),
    band!("s_neon_tease"),
    band!("s_neon_trigger"),
    band!("s_neon_win0"),
    band!("s_neon_win1"),
    band!("s_neon_win2"),
    band!("s_scoville_buy"),
    band!("s_scoville_grow"),
    band!("s_scoville_scatter"),
    band!("s_scoville_spin"),
    band!("s_scoville_stop"),
    band!("s_scoville_tease"),
    band!("s_scoville_trigger"),
    band!("s_scoville_win0"),
    band!("s_scoville_win1"),
    band!("s_scoville_win2"),
    band!("s_sugar_bomb"),
    band!("s_sugar_bombadd"),
    band!("s_sugar_buy"),
    band!("s_sugar_drop"),
    band!("s_sugar_multiply"),
    band!("s_sugar_pop"),
    band!("s_sugar_scatter"),
    band!("s_sugar_spin"),
    band!("s_sugar_stop"),
    band!("s_sugar_tease"),
    band!("s_sugar_trigger"),
    band!("s_sugar_win0"),
    band!("s_sugar_win1"),
    band!("s_sugar_win2"),
    band!("s_tomb_buy"),
    band!("s_tomb_coin"),
    band!("s_tomb_collect"),
    band!("s_tomb_grand"),
    band!("s_tomb_jackpot"),
    band!("s_tomb_reset"),
    band!("s_tomb_scatter"),
    band!("s_tomb_spin"),
    band!("s_tomb_stop"),
    band!("s_tomb_tease"),
    band!("s_tomb_trigger"),
    band!("s_tomb_win0"),
    band!("s_tomb_win1"),
    band!("s_tomb_win2"),
];

/// A sound's number (for `poker_sfx::Cue::Band`), by its name.
pub fn id(name: &str) -> Option<u16> {
    BANDS.iter().position(|b| b.0 == name).map(|i| i as u16)
}

fn cache() -> &'static Mutex<HashMap<u16, Pcm>> {
    static CACHE: OnceLock<Mutex<HashMap<u16, Pcm>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// A sound, 48 kHz stereo (interleaved), placed `pan` (-1 left to 1 right).
pub fn render(i: u16, pan: f32) -> Vec<f32> {
    let pcm = {
        let mut c = cache().lock();
        match c.get(&i) {
            Some(p) => p.clone(),
            None => {
                let Some((_, file)) = BANDS.get(i as usize) else { return Vec::new() };
                let p: Pcm = Arc::new(soundboard::decode(file).unwrap_or_default());
                c.insert(i, p.clone());
                p
            }
        }
    };
    let pan = pan.clamp(-1.0, 1.0);
    let (l, r) = ((1.0 - pan.max(0.0)), (1.0 + pan.min(0.0)));
    pcm.chunks(2).flat_map(|x| [x[0] * l, x.get(1).copied().unwrap_or(x[0]) * r]).collect()
}

/// A sound `cents` hundredths of a semitone higher (played faster, so shorter; lower and
/// longer when negative), 48 kHz stereo, placed `pan`.
pub fn render_up(i: u16, cents: i16, pan: f32) -> Vec<f32> {
    let s = render(i, pan);
    if cents == 0 {
        return s;
    }
    let speed = 2f64.powf(cents as f64 / 1200.0);
    let frames = s.len() / 2;
    let n = (frames as f64 / speed) as usize;
    let mut out = Vec::with_capacity(n * 2);
    for k in 0..n {
        let x = k as f64 * speed;
        let j = x as usize;
        let f = (x - j as f64) as f32;
        let j1 = (j + 1).min(frames - 1);
        for ch in 0..2 {
            out.push(s[j * 2 + ch] * (1.0 - f) + s[j1 * 2 + ch] * f);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A sound raised a whole tone is that much shorter, and as loud.
    #[test]
    fn a_sound_raised_is_shorter() {
        let i = id("s_all_tierup").unwrap();
        let (a, b) = (render(i, 0.0), render_up(i, 200, 0.0));
        let ratio = a.len() as f64 / b.len() as f64;
        assert!((ratio - 2f64.powf(2.0 / 12.0)).abs() < 0.001, "{ratio}");
        let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        assert!((peak(&a) - peak(&b)).abs() < 0.05);
        assert_eq!(render_up(i, 0, 0.0), a);
        assert!(render_up(i, -200, 0.0).len() > a.len());
    }

    /// Every band decodes, isn't silent and doesn't clip; each machine has its whole band.
    #[test]
    fn every_band_plays() {
        for (i, (name, _)) in BANDS.iter().enumerate() {
            let s = render(i as u16, 0.0);
            assert!(s.len() > 4_800, "{name}");
            let peak = s.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!(peak > 0.01 && peak <= 1.0, "{name} {peak}");
        }
        for m in ["cabaret", "cauldron", "cosmic", "deepsea", "frost", "goldrush", "grind", "hoard", "neon", "scoville", "sugar", "tomb"] {
            for part in ["buy", "scatter", "spin", "stop", "tease", "trigger", "win0", "win1", "win2"] {
                assert!(id(&format!("s_{m}_{part}")).is_some(), "{m} {part}");
            }
        }
    }
}
