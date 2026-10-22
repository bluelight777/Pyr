//! Is what Pyr plays coming back into the microphone?
//!
//! With speakers it is, and the echo canceller takes it out. With headphones it isn't, and the
//! echo canceller only gets in the way: while others talked or music played it turned your
//! voice down for moments at a time, 15 to 55 dB, several times a minute (bug reports 102 to
//! 119), even with its settings tuned (see `capture::tune_aec3`).
//!
//! So the microphone is compared with what's played the way a screen share's sound is (see
//! `stream_echo`): cross-correlation with the phase transform (GCC-PHAT, at 16 kHz) finds a
//! delayed copy of the speakers' sound in the microphone even while you talk over it. Until
//! plenty has been played (6 s of it) with no sign of it in the microphone, the echo
//! canceller's sound goes out, as before. Then the sound without echo cancellation goes out,
//! until the speakers' sound turns up in the microphone again (a quarter of a second of it
//! does), or the two lose step (the speakers changed). Sound that does come back but more than
//! 45 dB weaker (open headphones turned up) is too faint to hear, and is treated the same.

use std::collections::VecDeque;
use std::sync::Arc;

use realfft::{ComplexToReal, RealToComplex};

use super::stream_echo::{DEC, DEC_TAPS, Detector, MAX_LAG, NY, PROMINENT, detector_plans, ring};

/// 10 ms at 48 kHz.
const B: usize = 480;
const RATE: f32 = 48_000.0;
/// What's played, kept (1.36 s; a look reaches 0.77 s back, and what's played runs ahead).
const HISTORY: usize = 1 << 16;
/// The microphone's sound, kept (341 ms; a look takes 256 ms).
const CAPTURED: usize = 1 << 14;
/// How far the played sound runs ahead of the microphone's is followed over a couple of
/// seconds (sound devices hand over their sound in bursts, and some in big ones: 100 ms and
/// more at a time). When it's suddenly a second away from that, the two have lost step: the
/// speakers changed, or stopped for a while. Start over.
const STEP_FOLLOW: f64 = 1.0 / 200.0;
const LOST_STEP: f64 = 48_000.0;
/// When what's followed has moved this far from how the two are lined up (50 ms: the two
/// devices' clocks run apart), line them up again.
const REALIGN: i64 = 2_400;
/// A look can't wait for what's played to catch up for ever (lined up by the usual, the
/// microphone's end is as often ahead of what's played as behind): up to 100 ms of what's
/// about to be played is taken as silence instead.
const UNPLAYED: i64 = 4_800;
/// A look whose best match stands out less than this is no sign of the speakers in the
/// microphone (measured on screen shares: with no leak it stays under 35; a leak 20 dB under
/// the rest scores 50 to 110).
const QUIET: f32 = 35.0;
/// Looks (256 ms each) with plenty played and no sign of it in the microphone before it's
/// taken that none of it comes back: 6 s of it.
const QUIET_LOOKS: u32 = 24;
/// Plenty played: louder than -45 dBFS over a look (people talk at about -20 to -30).
const PLENTY: f32 = 3.2e-5;
/// Played 10 ms frames' levels kept (1.28 s).
const LEVELS: usize = 128;
/// Looks the coupling is the least of (10 s).
const COUPLING_LOOKS: usize = 40;
/// What comes back this much weaker than it was played (dB) is too faint to hear: under your
/// voice by 20 dB and more, and too quiet to open the microphone by itself. (2 dB either side,
/// so it doesn't flip back and forth.)
const FAINT_DB: f32 = -45.0;
/// Once it's been found, it takes three times as long without a sign of it to take it that
/// nothing comes back any more.
const FOUND_QUIET_LOOKS: u32 = 3 * QUIET_LOOKS;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Echo {
    /// Not known yet, or the speakers changed: the echo canceller's sound goes out.
    Unknown,
    /// The speakers' sound comes back into the microphone, this late (ms).
    Found(f32),
    /// Plenty has been played and none of it came back (headphones).
    None,
}

impl Echo {
    /// The same kind (a different delay found is no change).
    fn kind(self) -> u8 {
        match self {
            Echo::Unknown => 0,
            Echo::Found(_) => 1,
            Echo::None => 2,
        }
    }
}

pub struct MicEcho {
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Arc<dyn ComplexToReal<f32>>,
    /// What was played (mono), by index (`played_total` counts samples).
    played: [Vec<f32>; 1],
    played_total: i64,
    /// What the microphone heard.
    captured: [Vec<f32>; 1],
    captured_total: i64,
    /// The played index microphone sample 0 lines up with, and the first microphone sample
    /// that does (since the two last lost step).
    base: Option<i64>,
    since: i64,
    /// How far what's played runs ahead of what the microphone heard (samples), followed.
    offset: Option<f64>,
    next_look: i64,
    det: Detector,
    state: Echo,
    /// Looks in a row with plenty played and no sign of it.
    quiet: u32,
    /// Each played 10 ms frame's mean square, by frame (`played_total / B`).
    levels: Vec<f32>,
    /// The speakers' sound comes back this much weaker (power ratio): the least over the last
    /// looks that found it (the ones you were quiet in).
    couplings: VecDeque<f32>,
    /// What comes back is too faint to hear (see `FAINT_DB`).
    faint: bool,
    /// (For measuring tools: looks taken, and the last one's prominence and what was played.)
    looks: u64,
    last_look: (f32, f32),
}

impl Default for MicEcho {
    fn default() -> Self {
        Self::new()
    }
}

impl MicEcho {
    pub fn new() -> MicEcho {
        let (fwd, inv) = detector_plans();
        MicEcho {
            fwd,
            inv,
            played: [vec![0.0; HISTORY]],
            played_total: 0,
            captured: [vec![0.0; CAPTURED]],
            captured_total: 0,
            base: None,
            since: 0,
            offset: None,
            next_look: 0,
            det: Detector::new(),
            state: Echo::Unknown,
            quiet: 0,
            levels: vec![0.0; LEVELS],
            couplings: VecDeque::new(),
            faint: false,
            looks: 0,
            last_look: (0.0, 0.0),
        }
    }

    /// For measuring tools: how the two are lined up and what the looks found.
    #[doc(hidden)]
    pub fn debug(&self) -> String {
        format!(
            "played {} heard {} offset {:?} base {:?} looks {} quiet {} last prominence {:.1} played {:.1} dB, {:?}{}",
            self.played_total,
            self.captured_total,
            self.offset.map(|o| o.round()),
            self.base,
            self.looks,
            self.quiet,
            self.last_look.0,
            10.0 * self.last_look.1.max(1e-12).log10(),
            self.state,
            if self.faint { " (faint)" } else { "" }
        )
    }

    pub fn state(&self) -> Echo {
        self.state
    }

    /// Whether the echo canceller is needed: unless nothing played comes back into the
    /// microphone (headphones), or what does is too faint to hear.
    pub fn needs_canceller(&self) -> bool {
        match self.state {
            Echo::Unknown => true,
            Echo::Found(_) => !self.faint,
            Echo::None => false,
        }
    }

    /// What comes back is too faint to hear.
    pub fn faint(&self) -> bool {
        self.faint
    }

    /// What's about to be played (48 kHz mono, 10 ms).
    pub fn played(&mut self, mono: &[f32]) {
        let mut ms = 0.0;
        for v in mono {
            self.played[0][ring(self.played_total, HISTORY)] = *v;
            self.played_total += 1;
            ms += v * v;
        }
        let f = (self.played_total - 1).div_euclid(B as i64);
        self.levels[ring(f, LEVELS)] = ms / mono.len().max(1) as f32;
    }

    /// The speakers changed: whatever came back before may not now.
    pub fn restart(&mut self) -> bool {
        self.base = None;
        self.offset = None;
        self.start_over()
    }

    /// Back to not knowing, lined up afresh from here (`base` and `offset` set by the caller, or
    /// on the next 10 ms heard).
    fn start_over(&mut self) -> bool {
        self.since = self.captured_total;
        self.det = Detector::new();
        self.next_look = 0;
        self.quiet = 0;
        self.couplings.clear();
        self.faint = false;
        let was = self.state;
        self.state = Echo::Unknown;
        was != Echo::Unknown
    }

    /// 10 ms the microphone heard (48 kHz mono, before any processing). Returns what's now
    /// known, when that changes.
    pub fn heard(&mut self, mic: &[f32]) -> Option<Echo> {
        debug_assert_eq!(mic.len(), B);
        if self.played_total == 0 {
            // Nothing played yet: nothing could come back.
            return None;
        }
        let before = self.state;
        for (i, v) in mic.iter().enumerate() {
            self.captured[0][ring(self.captured_total + i as i64, CAPTURED)] = *v;
        }
        self.captured_total += B as i64;
        // Line the two up: microphone sample k goes with played sample base + k, base being how
        // far what's played usually runs ahead (so this frame ends about where what's played so
        // far does). Each device's sound comes in bursts; what's followed is the usual.
        let ahead = (self.played_total - self.captured_total) as f64;
        match self.offset {
            Some(o) if (ahead - o).abs() <= LOST_STEP => self.offset = Some(o + (ahead - o) * STEP_FOLLOW),
            Some(_) => {
                // Lost step.
                self.offset = Some(ahead);
                self.base = Some(ahead as i64);
                self.start_over();
            }
            None => {
                self.offset = Some(ahead);
                self.base = Some(ahead as i64);
                self.start_over();
            }
        }
        let usual = self.offset.unwrap_or(ahead).round() as i64;
        let mut base = self.base.unwrap_or(usual);
        if (usual - base).abs() > REALIGN {
            // The clocks ran apart: lined up afresh. What's been learnt about the echo stands;
            // only the looks so far are of no more use.
            if let Echo::Found(ms) = self.state {
                self.state = Echo::Found(ms + (usual - base) as f32 / RATE * 1000.0);
            }
            base = usual;
            self.base = Some(base);
            self.det = Detector::new();
        }
        let now = base + self.captured_total;
        // Look every 256 ms, once there's enough of both (and what's played that the look needs
        // is there, and still kept).
        if self.captured_total >= self.next_look
            && self.captured_total - self.since >= (NY * DEC + DEC_TAPS) as i64
            && now >= ((NY + MAX_LAG) * DEC + DEC_TAPS) as i64
            && now <= self.played_total + UNPLAYED
            && self.played_total.max(now) - (now - ((NY + MAX_LAG) * DEC + DEC_TAPS) as i64) < HISTORY as i64
        {
            self.next_look = self.captured_total + (NY * DEC) as i64;
            // (What isn't played yet is silence for now; it's written over as it's played.)
            for i in self.played_total..now {
                self.played[0][ring(i, HISTORY)] = 0.0;
            }
            let found = self.det.look(&self.fwd, &self.inv, &self.played, now, &self.captured, self.captured_total);
            self.looks += 1;
            if let Some((prominence, lag)) = self.det.last {
                let span = (NY * DEC) as i64;
                let played = self.power(&self.played[0], now - span, now);
                self.last_look = (prominence, played);
                if prominence < QUIET {
                    if played >= PLENTY {
                        self.quiet += 1;
                    }
                } else {
                    self.quiet = 0;
                }
                if let Some(lag) = found {
                    self.state = Echo::Found(lag as f32 / RATE * 1000.0);
                } else if prominence > PROMINENT && self.state == Echo::None {
                    // A sign of it: back to the echo canceller's sound while it's made sure of.
                    self.state = Echo::Unknown;
                    self.faint = false;
                }
                let needed = if matches!(self.state, Echo::Found(_)) { FOUND_QUIET_LOOKS } else { QUIET_LOOKS };
                if self.quiet >= needed {
                    self.state = Echo::None;
                    self.couplings.clear();
                    self.faint = false;
                }
                // How much weaker it comes back: the microphone against what was played that
                // long before (least over the last looks: the ones without you talking).
                if prominence > PROMINENT && played >= PLENTY {
                    let lag = lag as i64;
                    let heard = self.power(&self.captured[0], self.captured_total - span, self.captured_total);
                    let back = self.power(&self.played[0], now - span - lag, now - lag);
                    if back > 0.0 {
                        self.couplings.push_back(heard / back);
                        if self.couplings.len() > COUPLING_LOOKS {
                            self.couplings.pop_front();
                        }
                    }
                    if let Some(c) = self.coupling_db() {
                        if c <= FAINT_DB - 2.0 {
                            self.faint = true;
                        } else if c > FAINT_DB + 2.0 {
                            self.faint = false;
                        }
                    }
                }
            }
        }
        (self.state.kind() != before.kind()).then_some(self.state)
    }

    /// Mean square of a ring between two indexes.
    fn power(&self, ring_buf: &[f32], from: i64, to: i64) -> f32 {
        let n = (to - from).max(1);
        (from..to).map(|i| ring_buf[ring(i, ring_buf.len())]).map(|v| v * v).sum::<f32>() / n as f32
    }

    /// How loud (mean square) the speakers' sound could be in the microphone's last 10 ms: none
    /// with headphones; if it's known how late and how much weaker it comes back, what was
    /// played then (and its echoes, 100 ms after) that much weaker; otherwise, as loud as
    /// anything played in the last 450 ms.
    pub fn could_be_echo(&self) -> f32 {
        let Some(base) = self.base else { return 0.0 };
        let now = base + self.captured_total;
        let frame = |i: i64| i.div_euclid(B as i64);
        let loudest = |from: i64, to: i64| {
            let (a, b) = (frame(from), frame(to));
            let newest = frame(self.played_total - 1);
            (a..=b.min(newest)).filter(|f| newest - f < LEVELS as i64).map(|f| self.levels[ring(f, LEVELS)]).fold(0.0f32, f32::max)
        };
        match self.state {
            Echo::None => 0.0,
            Echo::Found(ms) if !self.couplings.is_empty() => {
                let lag = (ms / 1000.0 * RATE) as i64;
                let coupling = self.couplings.iter().copied().fold(f32::MAX, f32::min);
                coupling * loudest(now - lag - (RATE * 0.1) as i64, now - lag + (RATE * 0.02) as i64)
            }
            _ => loudest(now - (RATE * 0.45) as i64, now),
        }
    }

    /// For logs: how much weaker the speakers' sound comes back (dB), once known.
    pub fn coupling_db(&self) -> Option<f32> {
        (!self.couplings.is_empty()).then(|| 10.0 * self.couplings.iter().copied().fold(f32::MAX, f32::min).max(1e-12).log10())
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

    /// Talking-like sound: colored noise in bursts (on for `on` of every `period` seconds).
    fn talk(rng: &mut Rng, n: usize, on: f32, period: f32, level: f32, phase: f32) -> Vec<f32> {
        let mut lp = 0f32;
        (0..n)
            .map(|i| {
                let t = i as f32 / RATE + phase;
                lp += 0.3 * (rng.next() - lp);
                if (t / period).fract() < on { level * lp * (1.0 + 0.8 * (t * 17.0).sin()) } else { 0.0 }
            })
            .collect()
    }

    /// Runs `secs`: the speakers play `far`, the microphone hears `you` and, from `echo_from`
    /// seconds on, the speakers `delay` samples late at `gain` (with two reflections). Returns
    /// each 10 ms frame's state.
    fn run(far: &[f32], you: &[f32], echo_from: Option<f32>, delay: usize, gain: f32) -> Vec<Echo> {
        let n = far.len().min(you.len());
        let mut mic = you.to_vec();
        let mut rng = Rng(77);
        for (i, m) in mic.iter_mut().enumerate().take(n) {
            *m += 1e-4 * rng.next();
            if echo_from.is_some_and(|s| i as f32 / RATE >= s) {
                for (extra, amp) in [(0usize, 1.0f32), (7 * 48, 0.45), (19 * 48, 0.25)] {
                    if i >= delay + extra {
                        *m += gain * amp * far[i - delay - extra];
                    }
                }
            }
        }
        let mut me = MicEcho::new();
        let mut states = Vec::new();
        for k in 0..n / B {
            me.played(&far[k * B..(k + 1) * B]);
            me.heard(&mic[k * B..(k + 1) * B]);
            states.push(me.state());
        }
        states
    }

    fn sounds(secs: usize) -> (Vec<f32>, Vec<f32>) {
        let n = secs * RATE as usize;
        let far = talk(&mut Rng(0x1234_5678), n, 0.55, 3.1, 0.12, 0.0);
        // You talk at other times and over them too.
        let you = talk(&mut Rng(0x9abc_def1), n, 0.5, 2.3, 0.08, 1.0);
        (far, you)
    }

    #[test]
    fn with_headphones_nothing_comes_back() {
        let (far, you) = sounds(20);
        let states = run(&far, &you, None, 0, 0.0);
        let first = states.iter().position(|s| *s == Echo::None).expect("never decided there's no echo");
        assert!(first < 1500, "took {} s", first as f32 / 100.0);
        assert!(states[first..].iter().all(|s| *s == Echo::None), "went back without a reason");
        assert!(!states.iter().any(|s| matches!(s, Echo::Found(_))));
    }

    #[test]
    fn with_speakers_it_is_found_and_never_taken_for_headphones() {
        let (far, you) = sounds(20);
        // 60 ms late, 18 dB weaker; and 30 dB weaker (speakers turned low).
        for gain in [0.126, 0.032] {
            let states = run(&far, &you, Some(0.0), 60 * 48, gain);
            assert!(!states.contains(&Echo::None), "taken for headphones at gain {gain}");
            let Some(Echo::Found(ms)) = states.last() else { panic!("not found at gain {gain}: {:?}", states.last()) };
            assert!((ms - 60.0).abs() < 1.0, "found it {ms} ms late");
        }
    }

    #[test]
    fn speakers_turned_on_later_are_noticed_within_a_second() {
        let (far, you) = sounds(30);
        let states = run(&far, &you, Some(18.0), 45 * 48, 0.126);
        assert_eq!(states[1700], Echo::None, "no echo before the speakers came on");
        // Back to the echo canceller's sound within a second of them playing (they play 55%
        // of the time).
        let back = states[1800..].iter().position(|s| *s != Echo::None).expect("never noticed");
        assert!(back < 200, "took {} s", back as f32 / 100.0);
        assert!(!states[1800 + back..].contains(&Echo::None));
    }

    #[test]
    fn a_faint_leak_needs_no_canceller_but_a_quiet_speaker_does() {
        let (far, you) = sounds(24);
        // (Others as loud as they usually play, about -20 dBFS.)
        let far: Vec<f32> = far.iter().map(|v| v * 2.5).collect();
        // 50 dB weaker (open headphones turned up), over a microphone's own hiss.
        let mut me = MicEcho::new();
        let mut rng = Rng(5);
        let hiss = |rng: &mut Rng| 3e-4 * rng.next();
        let run = |me: &mut MicEcho, rng: &mut Rng, gain: f32| {
            for k in 0..far.len() / B {
                let mic: Vec<f32> = (k * B..(k + 1) * B).map(|i| you[i] + hiss(rng) + if i >= 2400 { gain * far[i - 2400] } else { 0.0 }).collect();
                me.played(&far[k * B..(k + 1) * B]);
                me.heard(&mic);
            }
        };
        run(&mut me, &mut rng, 0.0032);
        assert!(!me.needs_canceller(), "{:?}, coupling {:?}", me.state(), me.coupling_db());
        // 30 dB weaker (speakers turned low): needed.
        let mut me = MicEcho::new();
        run(&mut me, &mut rng, 0.032);
        assert!(me.needs_canceller(), "{:?}, coupling {:?}", me.state(), me.coupling_db());
        assert!(matches!(me.state(), Echo::Found(_)));
    }

    #[test]
    fn losing_step_starts_over_unknown() {
        let (far, you) = sounds(18);
        let mut me = MicEcho::new();
        for k in 0..1500 {
            me.played(&far[k * B..(k + 1) * B]);
            me.heard(&you[k * B..(k + 1) * B]);
        }
        assert_eq!(me.state(), Echo::None);
        // The speakers stop for a moment and play on: that's no change.
        for k in 1500..1520 {
            me.heard(&you[k * B..(k + 1) * B]);
        }
        assert_eq!(me.state(), Echo::None);
        // The speakers stop for good (changed, gone): the microphone goes on (within 2 s,
        // it's taken that they've lost step).
        for k in 1520..1720 {
            me.heard(&you[k * B..(k + 1) * B]);
        }
        assert_eq!(me.state(), Echo::Unknown);
    }

    /// Sound devices hand over their sound in bursts: here the speakers take 50 ms at a time and
    /// the microphone's comes 200 ms at a time, up to 200 ms late.
    fn bursty(far: &[f32], mic: &[f32]) -> Vec<Echo> {
        let frames = far.len().min(mic.len()) / B;
        let mut me = MicEcho::new();
        let mut states = Vec::new();
        let mut played = 0;
        for t in 0..frames {
            // The speakers' queue topped up 50 ms at a time, ahead of when it plays.
            while played < (t + 8).min(frames) && (played <= t + 3 || t % 5 == 0) {
                me.played(&far[played * B..(played + 1) * B]);
                played += 1;
            }
            // The microphone's 200 ms, all at once.
            if t % 20 == 19 {
                for k in t - 19..=t {
                    me.heard(&mic[k * B..(k + 1) * B]);
                    states.push(me.state());
                }
            }
        }
        states
    }

    #[test]
    fn bursty_devices_are_lined_up_all_the_same() {
        let (far, you) = sounds(24);
        // Headphones.
        let states = bursty(&far, &you);
        let first = states.iter().position(|s| *s == Echo::None).expect("never decided there's no echo");
        assert!(first < 1600, "took {} s", first as f32 / 100.0);
        assert!(states[first..].iter().all(|s| *s == Echo::None), "went back without a reason");
        // Speakers, 30 ms late and 18 dB weaker.
        let mut mic = you.clone();
        for i in 1440..mic.len() {
            mic[i] += 0.126 * far[i - 1440];
        }
        let states = bursty(&far, &mic);
        assert!(!states.contains(&Echo::None), "taken for headphones");
        assert!(matches!(states.last(), Some(Echo::Found(_))), "{:?}", states.last());
    }

    #[test]
    fn what_could_be_echo() {
        let (far, you) = sounds(20);
        let mut mic = you.clone();
        for i in 2880..mic.len() {
            mic[i] += 0.126 * far[i - 2880];
        }
        let mut me = MicEcho::new();
        let mut later = Vec::new();
        for k in 0..2000 {
            me.played(&far[k * B..(k + 1) * B]);
            me.heard(&mic[k * B..(k + 1) * B]);
            if k > 1000 {
                later.push((me.could_be_echo(), far[(k * B).saturating_sub(2880)..(k + 1) * B - 2880].iter().map(|v| v * v).sum::<f32>() / B as f32));
            }
        }
        let c = me.coupling_db().expect("coupling not learnt");
        assert!((-20.0..=-14.0).contains(&c), "coupling {c:.1} dB, not about -18");
        // Never much less than the echo really was.
        for (could, was) in later {
            assert!(could >= was * 0.126 * 0.126 * 0.5, "{could} for an echo of {}", was * 0.016);
        }
    }
}
