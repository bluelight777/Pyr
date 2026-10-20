//! Keeping a screen share (or a camera) smooth over slow, far-away or lossy connections.
//!
//! [`RateControl`] works out how much the network can carry right now, from what the
//! connection to the server says (its round trip against the quietest it's been, the frames
//! still on their way and how late they are, losses) and from what the server says about each
//! viewer's connection ([`ViewerReport`]): like WebRTC's bandwidth estimation, it backs off
//! quickly when queues build up and creeps back up while they don't, so the share never fills
//! the connection (which made everything late, voices too).
//!
//! [`Ladder`] turns that bitrate into a size and frame rate worth sending: a game or video keeps
//! its frame rate and gets smaller; text keeps its sharpness and gets fewer frames a second
//! (what [`Content`] tells apart). The preset picked in Settings is the best it ever sends.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use super::codec::RawFrame;

/// Never asks for less than this (a still picture's worth, a few frames a second).
pub const MIN_BPS: u32 = 150_000;

/// What the QUIC connection says about itself (`quinn::Connection::stats().path`).
#[derive(Clone, Copy, Debug, Default)]
pub struct PathSample {
    pub rtt: Duration,
    pub min_rtt: Duration,
    pub lost: u64,
    pub sent: u64,
}

/// One viewer's connection, as the server sees it (from `Event::VideoFeedback`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewerReport {
    pub session: u32,
    /// Their round trip to the server, and how much of it is queueing (above their quietest).
    pub rtt_ms: u32,
    pub queue_ms: u32,
    /// How late the oldest frame still on its way to them is (beyond a round trip and its
    /// own sending time).
    pub late_ms: u32,
    /// What reached them, kbit/s.
    pub kbps: u32,
    /// Packets lost on the way to them, per thousand.
    pub loss_pm: u32,
    /// Frames the server had to skip for them (they couldn't keep up).
    pub shed: u32,
}

/// What [`RateControl`] made of the network's state: the worst signal, where from, how long
/// a cut takes to show there, and how bad it is (ms, or per thousand lost). From a viewer:
/// what has been reaching them lately (bit/s), the most this share can send them now.
struct Reading {
    signal: Signal,
    why: &'static str,
    react: Duration,
    level: f64,
    room: Option<f64>,
}

/// Our own connection's signals, or a viewer's (a cut for one shows on the other's only by
/// chance: they're told apart when deciding whether to cut again).
fn family(why: &str) -> &'static str {
    if why.starts_with("viewer") {
        "viewer"
    } else {
        "ours"
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Signal {
    Fine,
    /// Queues are building: back off a little.
    Over,
    /// Badly backed up (or losing a lot): back off a lot.
    Severe,
}

/// Queueing above the quietest round trip that means "too much".
const QUEUE_OVER: Duration = Duration::from_millis(45);
const QUEUE_SEVERE: Duration = Duration::from_millis(180);
/// How late a frame can be (beyond a round trip and its own sending time) before it means
/// "too much": this, plus one and a half round trips (one lost packet makes its frame a round
/// trip late, being sent again: that's no reason to send less)
const LATE_OVER: Duration = Duration::from_millis(150);
/// and this, plus two and a half.
const LATE_SEVERE: Duration = Duration::from_millis(450);

/// How late is too late (see `LATE_OVER`) with a round trip of `rtt`.
fn late_limits(rtt: Duration) -> (Duration, Duration) {
    (LATE_OVER + rtt.mul_f64(1.5), LATE_SEVERE + rtt.mul_f64(2.5))
}
/// The quietest round trip is the least over this long (so it follows a route that changed).
const BASE_WINDOW: Duration = Duration::from_secs(12);
/// What reached a viewer is the average over this long (a report while frames are skipped for
/// them can say much less than their road carries, one after a stall much more).
const REACHED_WINDOW: Duration = Duration::from_secs(3);
/// How long a share stays a little below a viewer's full road after backing off for them.
const VIEWER_CEILING_FOR: Duration = Duration::from_secs(30);

/// A frame handed to QUIC and not yet acknowledged.
#[derive(Clone, Copy, Debug)]
struct Unacked {
    seq: u32,
    bytes: usize,
    at: Instant,
}

/// How much one outgoing video track can send (see the module's documentation).
pub struct RateControl {
    max: f64,
    target: f64,
    /// No raising before this (after backing off, while the effect isn't visible yet).
    hold_until: Instant,
    last_cut: Option<Instant>,
    /// What the last cut was for, and how bad it was then.
    cut_level: Option<(&'static str, f64)>,
    /// What got through when we last overshot: going past it is careful.
    capacity: Option<(f64, Instant)>,
    /// After backing off for a viewer: not above this until then (their road was full: a share
    /// right at its edge queues up at every burst, and finding the edge again every few seconds
    /// made it stutter).
    ceiling: Option<(f64, Instant)>,
    /// Backed off at least once in this share (the start can ramp up faster).
    cut_ever: bool,
    /// Round trip samples (when, srtt), for the quietest of the last `BASE_WINDOW`.
    rtts: VecDeque<(Instant, Duration)>,
    srtt: Duration,
    last_path: Option<(u64, u64)>,
    loss: f64,
    unacked: VecDeque<Unacked>,
    /// Bytes acknowledged and handed over, with when (the last couple of seconds).
    acked: VecDeque<(Instant, usize)>,
    sent: VecDeque<(Instant, usize)>,
    /// The latest from the server about each viewer, and what reached each lately (kbit/s).
    viewers: HashMap<u32, (ViewerReport, Instant)>,
    reached: HashMap<u32, VecDeque<(Instant, u32)>>,
    over_ticks: u32,
    last_tick: Option<Instant>,
    created: Instant,
    /// Why it last changed course (for the media log).
    pub why: &'static str,
}

impl RateControl {
    /// `max_bps`: the preset's bitrate; `start_bps`: where to start (what worked last time, or
    /// part of the preset's).
    pub fn new(max_bps: u32, start_bps: u32, now: Instant) -> RateControl {
        RateControl {
            max: max_bps.max(MIN_BPS) as f64,
            target: start_bps.clamp(MIN_BPS, max_bps.max(MIN_BPS)) as f64,
            hold_until: now,
            last_cut: None,
            cut_level: None,
            capacity: None,
            ceiling: None,
            cut_ever: false,
            rtts: VecDeque::new(),
            srtt: Duration::ZERO,
            last_path: None,
            loss: 0.0,
            unacked: VecDeque::new(),
            acked: VecDeque::new(),
            sent: VecDeque::new(),
            viewers: HashMap::new(),
            reached: HashMap::new(),
            over_ticks: 0,
            last_tick: None,
            created: now,
            why: "start",
        }
    }

    pub fn target(&self) -> u32 {
        self.target as u32
    }

    /// A frame went to QUIC.
    pub fn on_sent(&mut self, seq: u32, bytes: usize, now: Instant) {
        self.unacked.push_back(Unacked { seq, bytes, at: now });
        self.sent.push_back((now, bytes));
        // (Never more than a few seconds' worth: frames give up after 2 s anyway.)
        while self.unacked.len() > 600 {
            self.unacked.pop_front();
        }
    }

    /// The server has all of frame `seq` (`ok`), or it was given up on.
    pub fn on_acked(&mut self, seq: u32, ok: bool, now: Instant) {
        if let Some(i) = self.unacked.iter().position(|u| u.seq == seq) {
            let u = self.unacked.remove(i).expect("found");
            if ok {
                self.acked.push_back((now, u.bytes));
            }
        }
    }

    /// The latest about this track's viewers (replaces what came before).
    pub fn on_viewers(&mut self, reports: &[ViewerReport], now: Instant) {
        self.viewers.retain(|s, _| reports.iter().any(|r| r.session == *s));
        self.reached.retain(|s, _| reports.iter().any(|r| r.session == *s));
        for r in reports {
            self.viewers.insert(r.session, (*r, now));
            let q = self.reached.entry(r.session).or_default();
            if r.kbps > 0 {
                q.push_back((now, r.kbps));
            }
            while q.front().is_some_and(|(t, _)| now.duration_since(*t) > REACHED_WINDOW) {
                q.pop_front();
            }
        }
    }

    /// What reached a viewer over the last few seconds, bit/s (while their road is full,
    /// about what it carries: a cut for them goes there, not below). The average: one report
    /// can count a burst of frames that arrived together.
    fn reached_bps(&self, session: u32) -> Option<f64> {
        let q = self.reached.get(&session)?;
        if q.is_empty() {
            return None;
        }
        let mean = q.iter().map(|(_, k)| *k as f64).sum::<f64>() / q.len() as f64;
        (mean > 0.0).then_some(mean * 1000.0)
    }

    /// Bit/s over the last `over` (or since the start, if that's shorter).
    fn rate(&self, q: &VecDeque<(Instant, usize)>, now: Instant, over: Duration) -> f64 {
        let span = over.min(now.saturating_duration_since(self.created)).max(Duration::from_millis(200));
        let bytes: usize = q.iter().filter(|(t, _)| now.saturating_duration_since(*t) <= span).map(|(_, b)| b).sum();
        bytes as f64 * 8.0 / span.as_secs_f64()
    }

    /// What got through over the last second, bit/s.
    pub fn acked_bps(&self, now: Instant) -> f64 {
        self.rate(&self.acked, now, Duration::from_secs(1))
    }

    /// The quietest round trip lately.
    pub fn base_rtt(&self) -> Duration {
        self.rtts.iter().map(|(_, r)| *r).min().unwrap_or(self.srtt)
    }

    /// How late the oldest frame still on its way is: beyond a round trip and the time its
    /// own bytes take at the current rate (with room for a keyframe's).
    pub fn oldest_late(&self, now: Instant) -> Duration {
        let Some(u) = self.unacked.front() else { return Duration::ZERO };
        let own = Duration::from_secs_f64(u.bytes as f64 * 8.0 * 1.5 / self.target.max(MIN_BPS as f64));
        now.saturating_duration_since(u.at).saturating_sub(self.srtt + own + Duration::from_millis(40))
    }

    /// What the network says now, from our connection and the viewers': the worst of them,
    /// with how long a cut takes to show there and how bad it is (ms).
    fn signal(&self, now: Instant) -> Reading {
        let mut worst = Reading { signal: Signal::Fine, why: "fine", react: Duration::ZERO, level: 0.0, room: None };
        // (A cut shows on our own connection a round trip later; on a viewer's, after it has
        // reached them too, and the report back (sent every half second).)
        let ours = self.srtt + Duration::from_millis(250);
        let mut see = |signal: Signal, why: &'static str, react: Duration, level: f64, room: Option<f64>| {
            if signal > worst.signal || (signal == worst.signal && signal != Signal::Fine && level > worst.level) {
                worst = Reading { signal, why, react, level, room };
            }
        };
        let queue = self.srtt.saturating_sub(self.base_rtt());
        let q = queue.as_secs_f64() * 1000.0;
        if queue > QUEUE_SEVERE {
            see(Signal::Severe, "queue", ours, q, None);
        } else if queue > QUEUE_OVER {
            see(Signal::Over, "queue", ours, q, None);
        }
        let late = self.oldest_late(now);
        let (over, severe) = late_limits(self.srtt);
        // (A share's first frames are late anyway on a long road: QUIC starts slowly, doubling
        // what it sends each round trip. Only being badly late counts until that's over.)
        let started = !self.acked.is_empty() && now.duration_since(self.created) > self.srtt * 3 + Duration::from_millis(300);
        let l = late.as_secs_f64() * 1000.0;
        if late > severe {
            see(Signal::Severe, "late", ours, l, None);
        } else if late > over && started {
            see(Signal::Over, "late", ours, l, None);
        }
        if self.loss > 0.12 {
            see(Signal::Severe, "loss", ours, self.loss * 1000.0, None);
        }
        for (r, at) in self.viewers.values() {
            if now.duration_since(*at) > Duration::from_secs(3) {
                continue;
            }
            let theirs = ours + Duration::from_millis(r.rtt_ms as u64 + 500);
            let (over, severe) = late_limits(Duration::from_millis(r.rtt_ms as u64));
            let (late, queue) = (Duration::from_millis(r.late_ms as u64), Duration::from_millis(r.queue_ms as u64));
            let level = r.late_ms.max(r.queue_ms) as f64;
            let room = self.reached_bps(r.session);
            if r.shed > 0 || late > severe || queue > QUEUE_SEVERE {
                see(Signal::Severe, "viewer", theirs, level.max(severe.as_secs_f64() * 1000.0), room);
            } else if late > over || queue > QUEUE_OVER {
                see(Signal::Over, "viewer", theirs, level, room);
            }
            if r.loss_pm > 120 {
                see(Signal::Severe, "viewer_loss", theirs, r.loss_pm as f64, room);
            }
        }
        worst
    }

    /// Every 100 ms or so: takes in the connection's state, decides, and returns the bitrate to
    /// send at.
    pub fn tick(&mut self, now: Instant, path: PathSample) -> u32 {
        let dt = self.last_tick.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.1).clamp(0.0, 1.0);
        self.last_tick = Some(now);
        self.srtt = path.rtt;
        // (The share starts during a call: the connection's quietest round trip so far is a
        // good start for the quietest lately.)
        if self.rtts.is_empty() && !path.min_rtt.is_zero() {
            self.rtts.push_back((now, path.min_rtt));
        }
        if !path.rtt.is_zero() {
            self.rtts.push_back((now, path.rtt));
        }
        while self.rtts.front().is_some_and(|(t, _)| now.duration_since(*t) > BASE_WINDOW) {
            self.rtts.pop_front();
        }
        if let Some((lost, sent)) = self.last_path {
            let (dl, ds) = (path.lost.saturating_sub(lost), path.sent.saturating_sub(sent));
            if ds > 0 {
                let frac = dl as f64 / ds as f64;
                self.loss += (frac - self.loss) * (dt / 1.0).min(1.0);
            }
        }
        self.last_path = Some((path.lost, path.sent));
        let keep = Duration::from_secs(2);
        while self.acked.front().is_some_and(|(t, _)| now.duration_since(*t) > keep) {
            self.acked.pop_front();
        }
        while self.sent.front().is_some_and(|(t, _)| now.duration_since(*t) > keep) {
            self.sent.pop_front();
        }

        let reading = self.signal(now);
        let acked = self.acked_bps(now);
        let sent = self.rate(&self.sent, now, Duration::from_secs(1));
        let since_cut = self.last_cut.map(|t| now.duration_since(t)).unwrap_or(Duration::MAX);
        // Once a cut has had time to show (`react`), another only if it's getting worse there
        // (or still bad a while later): a backlog takes a moment to clear, and cutting at every
        // report while it does would go far too low. (A viewer's loss and lateness are one
        // backlog: either one, again, waits for the cut for the other to show.)
        let again = since_cut > reading.react
            && match self.cut_level {
                None => true,
                Some((why, level)) if why == reading.why => reading.level > 1.2 * level || since_cut > reading.react * 2,
                Some((why, _)) if family(why) == family(reading.why) => since_cut > reading.react * 2,
                Some(_) => true,
            };
        // How far: from what got through to the server (our own connection), or what reached
        // the viewer whose road is full (their own is what they can take: halving what we send
        // instead went far below it, and took ages to climb back).
        let through = |floor: f64| reading.room.unwrap_or(acked).max(floor * self.target).min(self.target);
        let room = reading.room.unwrap_or(acked);
        match reading.signal {
            Signal::Severe if again => {
                let to = if reading.room.is_some() { 0.7 * through(0.3) } else { 0.5 * through(0.3) };
                self.cut(to, room, now, reading.react * 2, &reading);
            }
            Signal::Over => {
                self.over_ticks += 1;
                if self.over_ticks >= 2 && again {
                    let to = 0.85 * through(0.5);
                    self.cut(to, room, now, reading.react, &reading);
                }
            }
            Signal::Fine => {
                self.over_ticks = 0;
                if now >= self.hold_until {
                    let near = self.capacity.is_some_and(|(c, _)| self.target > 0.9 * c);
                    let factor = if near {
                        // Close to where it overshot before: carefully.
                        1.0 + 0.05 * dt
                    } else if !self.cut_ever {
                        // A new share that hasn't hit anything yet: find the room quickly.
                        1.25f64.powf(dt)
                    } else {
                        1.10f64.powf(dt)
                    };
                    let mut next = self.target * factor;
                    // Not far past what's actually being sent (a still screen sends little:
                    // its rate isn't proof the network can carry more).
                    let ceiling = (1.5 * sent + 300_000.0).max(self.target);
                    next = next.min(ceiling);
                    if let Some((most, until)) = self.ceiling {
                        if now < until {
                            next = next.min(most.max(self.target));
                        } else {
                            self.ceiling = None;
                        }
                    }
                    if next > self.target {
                        self.why = if near { "probe" } else { "ramp" };
                    }
                    self.target = next;
                    // The connection got better: forget where it overshot.
                    if let Some((c, at)) = self.capacity {
                        if self.target > 1.15 * c && now.duration_since(at) > Duration::from_secs(10) {
                            self.capacity = None;
                        }
                    }
                }
            }
            _ => {}
        }
        self.target = self.target.clamp(MIN_BPS as f64, self.max);
        self.target as u32
    }

    /// Backs off to `to`. `room`: what got through where it was too much (going past it again
    /// is careful).
    fn cut(&mut self, to: f64, room: f64, now: Instant, hold: Duration, reading: &Reading) {
        self.target = to.max(MIN_BPS as f64).min(self.target);
        self.last_cut = Some(now);
        self.cut_level = Some((reading.why, reading.level));
        self.cut_ever = true;
        self.hold_until = now + hold;
        self.over_ticks = 0;
        if room > 0.25 * self.target {
            self.capacity = Some((room, now));
        }
        // A viewer's road: some room below its edge for a while (bursts, keyframes, their
        // voices), before looking for more.
        if let Some(r) = reading.room {
            self.ceiling = Some(((0.85 * r).max(self.target), now + VIEWER_CEILING_FOR));
        }
        self.why = reading.why;
    }
}

/// What's being shared, as far as encoding goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    /// Most of the picture changes all the time (a game, a video): keep the frame rate.
    Motion,
    /// Mostly still, with small changes (text, a desktop): keep it sharp.
    Text,
}

/// Tells [`Content`] apart from how much of the picture changes from frame to frame (a small
/// grid of samples, cheap enough for every frame).
pub struct ContentWatch {
    last: Vec<u8>,
    /// (when, share of the grid that changed, which of its cells did)
    recent: VecDeque<(Instant, f32, Vec<bool>)>,
    kind: Content,
    /// When it last switched, and since when it has seemed the other kind.
    since: Instant,
    other_since: Option<Instant>,
}

/// A share of the picture changing in a quarter of the frames or more that means something's playing
/// there (a video in a window: a typing cursor, a clock or a spinner are a few cells).
const LIVELY_MOTION: f32 = 0.05;
const GRID_W: u32 = 48;
const GRID_H: u32 = 27;

impl ContentWatch {
    pub fn new(now: Instant) -> ContentWatch {
        // (Taken for text until it shows motion: what a share was before 0.8.0.)
        ContentWatch { last: Vec::new(), recent: VecDeque::new(), kind: Content::Text, since: now - Duration::from_secs(60), other_since: None }
    }

    pub fn kind(&self) -> Content {
        self.kind
    }

    pub fn feed(&mut self, f: &RawFrame, now: Instant) -> Content {
        let bpp = f.bytes_per_pixel() as u32;
        let mut grid = Vec::with_capacity((GRID_W * GRID_H) as usize);
        for gy in 0..GRID_H {
            let y = (gy * 2 + 1) * f.height / (GRID_H * 2);
            for gx in 0..GRID_W {
                let x = (gx * 2 + 1) * f.width / (GRID_W * 2);
                let o = ((y * f.width + x) * bpp) as usize;
                let p = &f.data[o..o + 3];
                grid.push(((p[0] as u32 + 2 * p[1] as u32 + p[2] as u32) / 4) as u8);
            }
        }
        if self.last.len() == grid.len() {
            let cells: Vec<bool> = grid.iter().zip(&self.last).map(|(a, b)| a.abs_diff(*b) > 3).collect();
            let changed = cells.iter().filter(|c| **c).count();
            self.recent.push_back((now, changed as f32 / grid.len() as f32, cells));
        }
        self.last = grid;
        while self.recent.front().is_some_and(|(t, ..)| now.duration_since(*t) > Duration::from_secs(3)) {
            self.recent.pop_front();
        }
        // Motion: in most frames of the last few seconds, a fair part of the picture moved (a
        // game, a video, even in a window); text: in few of them (typing, a scroll now and
        // then). In between, it stays what it was.
        if self.recent.len() >= 8 {
            let busy = self.recent.iter().filter(|(_, c, _)| *c > 0.15).count() as f32 / self.recent.len() as f32;
            // A video in a window (a stream in a browser, a clip in a chat): a part of the
            // picture that changes in most frames, however small a share of the whole it is,
            // and however still the person in it (0.9.1: such a share was taken for text, sent
            // at 5 frames a second, and looked like a slideshow).
            let n = self.recent.len();
            let mut counts = vec![0u32; self.last.len()];
            for (_, _, cells) in &self.recent {
                for (k, c) in cells.iter().enumerate() {
                    counts[k] += *c as u32;
                }
            }
            // (A cell is lively when it changed in a quarter of the frames or more: a blinking cursor
            // changes in a few, a video's noise and movement in many.)
            let lively = counts.iter().filter(|c| **c as usize * 4 >= n).count() as f32 / counts.len().max(1) as f32;
            let seems = if busy > 0.6 || lively >= LIVELY_MOTION {
                Some(Content::Motion)
            } else if busy < 0.3 && lively < LIVELY_MOTION / 2.0 {
                Some(Content::Text)
            } else {
                None
            };
            match seems {
                Some(k) if k != self.kind => {
                    // A while of the other kind before switching (a scroll isn't a video), and
                    // not back and forth (each switch can change the picture's size).
                    let wait = if k == Content::Motion { Duration::from_secs(2) } else { Duration::from_secs(4) };
                    let first = *self.other_since.get_or_insert(now);
                    if now.duration_since(first) >= wait && now.duration_since(self.since) >= Duration::from_secs(10) {
                        self.kind = k;
                        self.since = now;
                        self.other_since = None;
                    }
                }
                _ => self.other_since = None,
            }
        }
        self.kind
    }
}

/// A size and frame rate to send at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rung {
    /// The picture's height (its width follows the screen's shape).
    pub height: u32,
    pub fps: u32,
}

/// Chooses the [`Rung`] for a bitrate (see the module's documentation).
pub struct Ladder {
    max_h: u32,
    max_fps: u32,
    max_bps: f64,
    /// The preset's bits per pixel: the quality it was made for, kept while lowering.
    bpp: f64,
    current: Rung,
    /// When the target first allowed going up to the rung above (and which), or fell below
    /// what this one needs.
    up_since: Option<(Rung, Instant)>,
    down_since: Option<Instant>,
    last_change: Instant,
    last_down: Option<Instant>,
    /// The kind of content it last chose for, and whether it has chosen yet.
    content: Option<Content>,
    placed: bool,
}

/// Heights the ladder steps through (below the preset's own).
const HEIGHTS: [u32; 5] = [1440, 1080, 720, 540, 360];

impl Ladder {
    pub fn new(max_h: u32, max_fps: u32, max_bps: u32, now: Instant) -> Ladder {
        let max_fps = max_fps.max(1);
        let pixels = (max_h as f64 * 16.0 / 9.0) * max_h as f64 * max_fps as f64;
        Ladder {
            max_h,
            max_fps,
            max_bps: max_bps as f64,
            bpp: (max_bps as f64 / pixels).max(0.02),
            current: Rung { height: max_h, fps: max_fps },
            up_since: None,
            down_since: None,
            last_change: now - Duration::from_secs(60),
            last_down: None,
            content: None,
            placed: false,
        }
    }

    pub fn current(&self) -> Rung {
        self.current
    }

    /// The best it ever sends: the preset's size and frame rate (for any kind of content).
    pub fn top(&self) -> Rung {
        Rung { height: self.max_h, fps: self.max_fps }
    }

    /// Where to start, for `target` bit/s (no waiting: nothing has been sent yet, or what's
    /// shared changed kind).
    pub fn place(&mut self, target: u32, content: Content, now: Instant) -> Rung {
        let rungs = self.rungs(content);
        let r = rungs.iter().copied().find(|r| self.needs(*r, content) <= target as f64).unwrap_or(*rungs.last().expect("one at least"));
        if r.height != self.current.height {
            self.last_change = now;
        }
        self.current = r;
        self.content = Some(content);
        self.placed = true;
        self.current
    }

    /// The rungs, best first, for this kind of content.
    pub fn rungs(&self, content: Content) -> Vec<Rung> {
        let heights: Vec<u32> = std::iter::once(self.max_h).chain(HEIGHTS.iter().copied().filter(|h| *h < self.max_h)).collect();
        let lowest = *heights.last().expect("one at least");
        let mut v = Vec::new();
        match content {
            Content::Motion => {
                if self.max_fps > 30 {
                    // 60 fps was asked for: kept while the picture can stay 720p or more, then
                    // 30 fps from 720p down.
                    for &h in heights.iter().filter(|h| **h >= 720 || **h == self.max_h) {
                        v.push(Rung { height: h, fps: self.max_fps });
                    }
                    for &h in heights.iter().filter(|h| **h <= 720.min(self.max_h)) {
                        v.push(Rung { height: h, fps: 30 });
                    }
                } else {
                    for &h in &heights {
                        v.push(Rung { height: h, fps: self.max_fps });
                    }
                }
                for fps in [20, 15, 10, 5] {
                    if fps < self.max_fps.min(30) {
                        v.push(Rung { height: lowest, fps });
                    }
                }
            }
            Content::Text => {
                for fps in [self.max_fps, 30, 20, 15, 10, 5] {
                    if fps <= self.max_fps && !v.iter().any(|r: &Rung| r.fps == fps) {
                        v.push(Rung { height: self.max_h, fps });
                    }
                }
                for &h in heights.iter().skip(1) {
                    v.push(Rung { height: h, fps: 5 });
                }
            }
        }
        v
    }

    /// The bitrate a rung needs to look as good as the preset (text needs much less: most of
    /// it doesn't change).
    pub fn needs(&self, r: Rung, content: Content) -> f64 {
        let w = r.height as f64 * 16.0 / 9.0;
        let bpp = match content {
            Content::Motion => self.bpp * 0.85,
            Content::Text => self.bpp * 0.4,
        };
        w * r.height as f64 * r.fps as f64 * bpp
    }

    /// The rung for `target` bit/s now. `struggling`: the encoder has been skipping frames
    /// (what it's asked for is too much for the bitrate).
    pub fn decide(&mut self, target: u32, content: Content, struggling: bool, now: Instant) -> Rung {
        // What's shared changed kind (a game started, say): straight to its best rung.
        if self.content != Some(content) {
            self.content = Some(content);
            if self.placed {
                let before = self.current;
                let r = self.place(target, content, now);
                if r.height < before.height || r.fps < before.fps {
                    self.last_down = Some(now);
                }
                self.down_since = None;
                self.up_since = None;
                return r;
            }
        }
        self.placed = true;
        let target = target as f64;
        let rungs = self.rungs(content);
        // The best that fits.
        let fit = rungs.iter().copied().find(|r| self.needs(*r, content) <= target).unwrap_or(*rungs.last().expect("one at least"));
        // Where we are, on this content's ladder (or the nearest: content changed).
        let here = rungs.iter().position(|r| *r == self.current).unwrap_or_else(|| {
            rungs.iter().position(|r| r.height <= self.current.height && r.fps <= self.current.fps).unwrap_or(rungs.len() - 1)
        });
        let fit_at = rungs.iter().position(|r| *r == fit).expect("from the list");
        let mut next = rungs[here];
        if fit_at > here || struggling {
            // Down: at once when far short (or the encoder can't keep up), else after a
            // moment (a dip isn't worth a new keyframe).
            let far = target < 0.7 * self.needs(rungs[here], content);
            let since = *self.down_since.get_or_insert(now);
            // (Fewer frames a second is free; a smaller picture costs a keyframe.)
            let lower = if fit_at > here { rungs[fit_at] } else { rungs[(here + 1).min(rungs.len() - 1)] };
            let wait = if lower.height != rungs[here].height { Duration::from_millis(1500) } else { Duration::from_millis(400) };
            if far || struggling || now.duration_since(since) > wait {
                next = if fit_at > here { rungs[fit_at] } else { rungs[(here + 1).min(rungs.len() - 1)] };
            }
            self.up_since = None;
        } else {
            self.down_since = None;
            if fit_at < here {
                // Up one rung at a time, when there's clearly room for it for a while (a
                // bigger picture costs a keyframe, so it shouldn't flip back and forth).
                let up = rungs[here - 1];
                // (The preset's own needs no more than nearly its own bitrate.)
                let room = target >= (1.3 * self.needs(up, content)).min(0.95 * self.max_bps);
                let size_change = up.height != rungs[here].height;
                let wait = if size_change { Duration::from_secs(5) } else { Duration::from_secs(2) };
                let calm = self.last_down.is_none_or(|t| now.duration_since(t) > Duration::from_secs(8));
                match self.up_since {
                    Some((r, t)) if r == up && room => {
                        if now.duration_since(t) >= wait && calm {
                            next = up;
                        }
                    }
                    _ if room => self.up_since = Some((up, now)),
                    _ => self.up_since = None,
                }
            } else {
                self.up_since = None;
            }
        }
        // A change of size costs a keyframe: not more often than every few seconds (frame rate
        // is free to change).
        if next.height != self.current.height && now.duration_since(self.last_change) < Duration::from_secs(3) && !struggling {
            next = Rung { height: self.current.height, fps: next.fps };
        }
        if next != self.current {
            if next.height < self.current.height || next.fps < self.current.fps {
                self.last_down = Some(now);
            }
            if next.height != self.current.height {
                self.last_change = now;
            }
            self.current = next;
            self.down_since = None;
            self.up_since = None;
        }
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(rtt_ms: u64, lost: u64, sent: u64) -> PathSample {
        PathSample { rtt: Duration::from_millis(rtt_ms), min_rtt: Duration::from_millis(200), lost, sent }
    }

    /// A share whose frames all get through quickly: the rate climbs to the preset's and stays.
    #[test]
    fn a_clear_road_climbs_to_the_preset() {
        let t0 = Instant::now();
        let mut rc = RateControl::new(4_000_000, 2_000_000, t0);
        let mut seq = 0;
        for i in 0..200u64 {
            let now = t0 + Duration::from_millis(i * 100);
            // Sending at what's asked (3 frames per tick), each acknowledged a round trip later.
            for _ in 0..3 {
                let bytes = (rc.target() as f64 / 8.0 / 30.0) as usize;
                rc.on_sent(seq, bytes, now);
                rc.on_acked(seq, true, now + Duration::from_millis(1));
                seq += 1;
            }
            rc.tick(now, path(200, 0, i * 100));
        }
        assert_eq!(rc.target(), 4_000_000);
    }

    /// The round trip grows (our own frames queue up in a router): back off, and settle below
    /// what got through.
    #[test]
    fn a_growing_queue_backs_off() {
        let t0 = Instant::now();
        let mut rc = RateControl::new(4_000_000, 4_000_000, t0);
        let mut seq = 0;
        let capacity = 1_500_000.0;
        let mut queue_ms = 0.0f64;
        for i in 0..300u64 {
            let now = t0 + Duration::from_millis(i * 100);
            let send = rc.target() as f64;
            // A 1.5 Mbit/s link: what's sent beyond that waits in its queue.
            queue_ms = (queue_ms + (send - capacity) / capacity * 100.0).max(0.0);
            let through = (send.min(capacity) / 8.0 / 10.0) as usize;
            rc.on_sent(seq, through, now);
            rc.on_acked(seq, true, now);
            seq += 1;
            rc.tick(now, path(200 + queue_ms as u64, 0, i * 100));
        }
        let t = rc.target() as f64;
        assert!(t < capacity * 1.1 && t > capacity * 0.6, "settled at {t}");
        assert!(queue_ms < 100.0, "queue left: {queue_ms} ms");
    }

    /// Frames stuck on their way (the connection can't send them as fast as they come): back
    /// off hard.
    #[test]
    fn frames_stuck_on_the_way_back_off_hard() {
        let t0 = Instant::now();
        let mut rc = RateControl::new(4_000_000, 4_000_000, t0);
        for i in 0..20u32 {
            rc.on_sent(i, 15_000, t0 + Duration::from_millis(i as u64 * 33));
        }
        // Nothing acknowledged for a second and a half.
        let mut last = 0;
        for i in 0..15u64 {
            last = rc.tick(t0 + Duration::from_millis(i * 100), path(200, 0, 0));
        }
        assert!(last <= 2_000_000, "only down to {last}");
        assert!(rc.why == "late", "{}", rc.why);
    }

    /// A viewer whose connection can't keep up (the server says so): back off for them.
    #[test]
    fn a_struggling_viewer_backs_off() {
        let t0 = Instant::now();
        let mut rc = RateControl::new(4_000_000, 4_000_000, t0);
        rc.tick(t0, path(200, 0, 0));
        rc.on_viewers(&[ViewerReport { session: 7, rtt_ms: 210, queue_ms: 300, late_ms: 0, kbps: 1000, loss_pm: 0, shed: 0 }], t0);
        let t = rc.tick(t0 + Duration::from_millis(100), path(200, 0, 0));
        assert!(t <= 2_000_000, "{t}");
        assert_eq!(rc.why, "viewer");
    }

    /// A viewer's backlog takes a while to clear after a cut: as long as it's clearing, that
    /// one cut is enough (cutting at every report while it clears went far too low).
    #[test]
    fn a_clearing_backlog_isnt_cut_again_and_again() {
        let t0 = Instant::now();
        let mut rc = RateControl::new(4_000_000, 4_000_000, t0);
        let mut seq = 0;
        let mut lowest = u32::MAX;
        for i in 0..60u64 {
            let now = t0 + Duration::from_millis(i * 100);
            let bytes = (rc.target() as f64 / 8.0 / 10.0) as usize;
            rc.on_sent(seq, bytes, now);
            rc.on_acked(seq, true, now);
            seq += 1;
            // Every half second: 600 ms late at first, slowly clearing after the cut.
            if i % 5 == 0 {
                let late = if i < 10 { 600 } else { 600u32.saturating_sub((i as u32 - 10) * 10) };
                rc.on_viewers(&[ViewerReport { session: 1, rtt_ms: 200, queue_ms: 0, late_ms: late, kbps: 3000, loss_pm: 0, shed: 0 }], now);
            }
            lowest = lowest.min(rc.tick(now, path(200, 0, i * 100)));
        }
        // (Once, to about what reached them: 3 Mbit/s.)
        assert!(lowest >= 2_500_000, "cut down to {lowest}");
    }

    /// A viewer whose road takes 3 Mbit/s of a 4 Mbit/s share loses packets and falls behind
    /// while the backlog clears: the share goes to about what reaches them, once, not halving
    /// again at each report (which took it down to 0.5 Mbit/s in the lab), and climbs back to it.
    #[test]
    fn a_viewers_full_road_is_cut_to_what_reaches_them() {
        let t0 = Instant::now();
        let mut rc = RateControl::new(4_000_000, 4_000_000, t0);
        let mut seq = 0;
        let (mut lowest, mut cuts, mut last) = (u32::MAX, 0, rc.target());
        let mut backlog_ms = 0.0f64;
        let mut late_half = Vec::new();
        for i in 0..200u64 {
            let now = t0 + Duration::from_millis(i * 100);
            let send = rc.target() as f64;
            let bytes = (send / 8.0 / 10.0) as usize;
            rc.on_sent(seq, bytes, now);
            rc.on_acked(seq, true, now);
            seq += 1;
            // Their road: 3 Mbit/s, the rest waits until it clears; once the router's buffer
            // (150 ms) is full, what doesn't fit is lost.
            backlog_ms = (backlog_ms + (send - 3_000_000.0) / 3_000_000.0 * 100.0).max(0.0);
            if i % 5 == 0 && i > 0 {
                let reached = (send.min(3_000_000.0) / 1000.0) as u32;
                let loss_pm = if backlog_ms > 150.0 { ((send - 3_000_000.0).max(0.0) / send * 1000.0) as u32 } else { 0 };
                let late = backlog_ms as u32;
                rc.on_viewers(&[ViewerReport { session: 1, rtt_ms: 210, queue_ms: 0, late_ms: late, kbps: reached, loss_pm, shed: 0 }], now);
            }
            let t = rc.tick(now, path(210, 0, i * 100));
            if t < last {
                cuts += 1;
            }
            last = t;
            lowest = lowest.min(t);
            if i >= 100 {
                late_half.push(t as f64);
            }
        }
        assert!(lowest >= 1_900_000, "cut down to {lowest} ({cuts} cuts)");
        assert!(cuts <= 4, "{cuts} cuts in 20 s");
        let mean = late_half.iter().sum::<f64>() / late_half.len() as f64;
        assert!((2_400_000.0..=3_200_000.0).contains(&mean), "settled around {mean}");
    }

    /// A still screen sends little: that doesn't lower the rate, and the rate doesn't climb
    /// far past what's sent either.
    #[test]
    fn a_still_screen_neither_lowers_nor_inflates_the_rate() {
        let t0 = Instant::now();
        let mut rc = RateControl::new(8_000_000, 2_000_000, t0);
        for i in 0..300u32 {
            let now = t0 + Duration::from_millis(i as u64 * 100);
            rc.on_sent(i, 500, now);
            rc.on_acked(i, true, now);
            rc.tick(now, path(200, 0, i as u64 * 10));
        }
        assert_eq!(rc.target(), 2_000_000);
    }

    #[test]
    fn motion_keeps_its_frame_rate_and_text_its_size() {
        let t0 = Instant::now();
        // Standard: 1080p30 at 4 Mbit/s.
        let mut l = Ladder::new(1080, 30, 4_000_000, t0);
        assert_eq!(l.decide(4_000_000, Content::Motion, false, t0), Rung { height: 1080, fps: 30 });
        // A third of that: a smaller picture at the same frame rate (straight away: far short).
        let r = l.decide(1_300_000, Content::Motion, false, t0 + Duration::from_secs(10));
        assert_eq!(r, Rung { height: 540, fps: 30 }, "{:?}", l.rungs(Content::Motion));
        // Text at the same bitrate: full size, fewer frames (after a moment).
        let mut l = Ladder::new(1080, 30, 4_000_000, t0);
        assert_eq!(l.decide(1_300_000, Content::Text, false, t0 + Duration::from_secs(10)).fps, 30);
        let r = l.decide(1_300_000, Content::Text, false, t0 + Duration::from_millis(10_500));
        assert_eq!(r.height, 1080);
        assert!(r.fps < 30 && r.fps >= 15, "{r:?}");
        // Very little: text still full size, a few frames a second.
        let r = l.decide(300_000, Content::Text, false, t0 + Duration::from_secs(20));
        assert_eq!(r, Rung { height: 1080, fps: 5 });
    }

    #[test]
    fn sixty_fps_is_kept_at_720p_before_halving_it() {
        let t0 = Instant::now();
        // Smooth: 1080p60 at 6 Mbit/s.
        let mut l = Ladder::new(1080, 60, 6_000_000, t0);
        let r = l.decide(2_800_000, Content::Motion, false, t0 + Duration::from_secs(10));
        assert_eq!(r, Rung { height: 720, fps: 60 });
        let r = l.decide(1_500_000, Content::Motion, false, t0 + Duration::from_secs(20));
        assert_eq!(r.fps, 30, "{r:?}");
    }

    /// Going back up waits until there's clearly room for a while, one step at a time.
    #[test]
    fn going_up_waits_for_room_and_takes_one_step() {
        let t0 = Instant::now();
        let mut l = Ladder::new(1080, 30, 4_000_000, t0);
        assert_eq!(l.decide(700_000, Content::Motion, false, t0 + Duration::from_secs(5)).height, 360);
        // Room for 1080p again, but not for long enough yet.
        let mut t = t0 + Duration::from_secs(6);
        assert_eq!(l.decide(4_000_000, Content::Motion, false, t).height, 360);
        // After a while: one step (540p), then the next once that has settled.
        let mut heights = Vec::new();
        for _ in 0..400 {
            t += Duration::from_millis(100);
            heights.push(l.decide(4_000_000, Content::Motion, false, t).height);
        }
        heights.dedup();
        assert_eq!(heights, vec![360, 540, 720, 1080]);
    }

    /// Back at the preset's own bitrate, the preset's own size comes back (its bitrate is
    /// less than 1.3 times what its size needs).
    #[test]
    fn the_preset_itself_comes_back_at_its_own_bitrate() {
        let t0 = Instant::now();
        let mut l = Ladder::new(1080, 30, 4_000_000, t0);
        assert_eq!(l.place(2_400_000, Content::Motion, t0).height, 720);
        let mut t = t0;
        for _ in 0..200 {
            t += Duration::from_millis(100);
            l.decide(4_000_000, Content::Motion, false, t);
        }
        assert_eq!(l.current(), Rung { height: 1080, fps: 30 });
    }

    /// A short dip doesn't change the picture's size (that costs a keyframe).
    #[test]
    fn a_short_dip_keeps_the_size() {
        let t0 = Instant::now();
        let mut l = Ladder::new(1080, 30, 4_000_000, t0);
        l.decide(4_000_000, Content::Motion, false, t0);
        let t = t0 + Duration::from_secs(10);
        assert_eq!(l.decide(3_300_000, Content::Motion, false, t).height, 1080);
        assert_eq!(l.decide(3_300_000, Content::Motion, false, t + Duration::from_millis(900)).height, 1080);
        assert_eq!(l.decide(4_000_000, Content::Motion, false, t + Duration::from_millis(1000)).height, 1080);
        // A long one does.
        l.decide(3_300_000, Content::Motion, false, t + Duration::from_secs(2));
        assert_eq!(l.decide(3_300_000, Content::Motion, false, t + Duration::from_secs(4)).height, 720);
    }

    #[test]
    fn a_video_playing_in_a_window_is_motion() {
        use super::super::codec::PixelFormat;
        // A browser at 1080p: still page, a quarter of it a stream of someone sitting and
        // talking (much of it barely changing, the rest a little, every frame), and a
        // blinking cursor. Before 0.9.1 this was taken for text.
        let (w, h) = (1920u32, 1080u32);
        let t0 = Instant::now();
        let mut watch = ContentWatch::new(t0);
        let mut seed = 7u32;
        let mut got = watch.kind();
        for i in 0..300u64 {
            let mut data = vec![230u8; (w * h * 3) as usize];
            for y in 300..800u32 {
                for x in 500..1400u32 {
                    // The video: a face and a room, with the camera's noise and small moves.
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    let base = 60 + ((x / 40 + y / 40) % 5) as u32 * 25;
                    let moving = ((x as u64 + i * 3) / 60 % 2) as u32 * 6;
                    let v = (base + moving + (seed >> 29)) as u8;
                    let o = ((y * w + x) * 3) as usize;
                    data[o..o + 3].copy_from_slice(&[v, v, v]);
                }
            }
            if i / 15 % 2 == 0 {
                for y in 900..920u32 {
                    let o = ((y * w + 100) * 3) as usize;
                    data[o..o + 6].copy_from_slice(&[0; 6]);
                }
            }
            got = watch.feed(&RawFrame { width: w, height: h, format: PixelFormat::Rgb, data }, t0 + Duration::from_millis(i * 33));
        }
        assert_eq!(got, Content::Motion);
        // The same page with nothing playing (only the cursor): text.
        let mut watch = ContentWatch::new(t0);
        let mut got = Content::Motion;
        for i in 0..300u64 {
            let mut data = vec![230u8; (w * h * 3) as usize];
            if i / 15 % 2 == 0 {
                for y in 900..920u32 {
                    let o = ((y * w + 100) * 3) as usize;
                    data[o..o + 6].copy_from_slice(&[0; 6]);
                }
            }
            got = watch.feed(&RawFrame { width: w, height: h, format: PixelFormat::Rgb, data }, t0 + Duration::from_millis(i * 33));
        }
        assert_eq!(got, Content::Text);
    }

    #[test]
    fn content_is_told_apart() {
        use crate::video::testsrc::{MOTION_ID, Scene, TEXT_ID};
        let t0 = Instant::now();
        // (Every frame at 30 fps, and every third: what an encoder that's behind takes.)
        for (w, h, step) in [(640, 360, 1u64), (1920, 1080, 3)] {
            for (id, want) in [(MOTION_ID, Content::Motion), (TEXT_ID, Content::Text)] {
                let mut scene = Scene::for_id(id, w, h).unwrap();
                let mut watch = ContentWatch::new(t0);
                let mut got = watch.kind();
                for i in (0..180u64).step_by(step as usize) {
                    got = watch.feed(&scene.frame(i, 30, 0), t0 + Duration::from_millis(i * 33));
                }
                assert_eq!(got, want, "{id} at {w}x{h}");
            }
        }
    }
}
