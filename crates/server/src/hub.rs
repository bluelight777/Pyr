//! In-memory state of a running server: connected clients, voice sessions and media routing.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use pyr_proto::control::*;
use pyr_proto::invite::InviteLink;
use pyr_proto::media::track;
use pyr_proto::{b64, crypto, framing, now_ms};
use parking_lot::Mutex;
use quinn::VarInt;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::db::{self, Db};

pub type ConnId = u64;
pub type Reply = Result<Event, (ErrorCode, String)>;

fn err<T>(code: ErrorCode, msg: impl Into<String>) -> Result<T, (ErrorCode, String)> {
    Err((code, msg.into()))
}

/// Most bytes of control frames waiting to go out to one connection. A client that stops
/// reading (or can't keep up) is disconnected rather than letting the queue grow forever.
pub const MAX_QUEUED_BYTES: usize = 16 << 20;
/// Connections at once: from one place (an IPv4 address, or an IPv6 /64), from one wider
/// network (an IPv6 /48), by one member, and in all.
pub const MAX_CONNECTIONS_PER_IP: usize = 32;
pub const MAX_CONNECTIONS_PER_NETWORK: usize = 128;
pub const MAX_CONNECTIONS_PER_USER: usize = 6;
/// Connections that haven't logged in yet: from one place, one wider network, and in all, so
/// nobody can fill the server's slots with connections that never log in.
pub const MAX_WAITING_PER_IP: usize = 8;
pub const MAX_WAITING_PER_NETWORK: usize = 16;
pub const MAX_WAITING: usize = 256;
/// How long a new connection has to log in (or send its bug report), handshake included.
pub const LOGIN_WITHIN: Duration = Duration::from_secs(15);

/// Where a connection comes from, for counting: an IPv4 address, or for IPv6 the first
/// `prefix` bits (a home or a server usually has a whole /64 of addresses, so counting single
/// IPv6 addresses would let one place open thousands of connections).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Place {
    V4(u32),
    V6(u128),
}

impl Place {
    pub fn of(ip: IpAddr, prefix: u32) -> Place {
        match ip {
            IpAddr::V4(a) => Place::V4(u32::from(a)),
            IpAddr::V6(a) => match a.to_ipv4_mapped() {
                Some(v4) => Place::V4(u32::from(v4)),
                None => Place::V6(u128::from(a) & (!0u128 << (128 - prefix.min(128)))),
            },
        }
    }

    /// One place (an address, or an IPv6 /64), and the wider network it's in (an IPv6 /48).
    pub fn both(ip: IpAddr) -> (Place, Place) {
        (Place::of(ip, 64), Place::of(ip, 48))
    }
}
fn max_connections() -> usize {
    pyr_proto::env_var("MAX_CONNECTIONS").and_then(|v| v.parse().ok()).unwrap_or(4000)
}
/// A member this new (ms) gets a smaller share of the server's storage (see
/// `Hub::reserve_upload`).
const NEW_MEMBER_MS: i64 = 3 * 24 * 3600 * 1000;
/// Wrong invite codes allowed from one address before it has to wait.
const BAD_INVITES_PER_IP: u32 = 10;
const BAD_INVITE_WINDOW: Duration = Duration::from_secs(600);
/// New members allowed to join in a burst, and how fast that refills (raid protection).
const JOIN_BURST: f64 = 30.0;
const JOINS_PER_HOUR: f64 = 120.0;

/// One connection's queue of outgoing control frames (see [`MAX_QUEUED_BYTES`]).
#[derive(Clone)]
pub struct Outbox {
    tx: mpsc::UnboundedSender<Bytes>,
    queued: Arc<AtomicUsize>,
    conn: quinn::Connection,
}

impl Outbox {
    /// The outbox and what its writer task drains (it reports written bytes with
    /// [`Outbox::sent`]).
    pub fn new(conn: quinn::Connection) -> (Outbox, mpsc::UnboundedReceiver<Bytes>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Outbox { tx, queued: Arc::new(AtomicUsize::new(0)), conn }, rx)
    }

    /// Queues a frame. Frames too big for the other side to accept are dropped; a client too
    /// far behind is disconnected.
    pub fn send(&self, b: Bytes) -> bool {
        if b.len() > pyr_proto::MAX_CONTROL_FRAME + 4 {
            warn!(len = b.len(), "not sending a control frame that's too big for clients");
            return false;
        }
        let queued = self.queued.fetch_add(b.len(), Ordering::Relaxed) + b.len();
        if queued > MAX_QUEUED_BYTES {
            self.queued.fetch_sub(b.len(), Ordering::Relaxed);
            debug!("disconnecting a client that stopped reading");
            self.conn.close(VarInt::from_u32(5), b"too far behind");
            return false;
        }
        self.tx.send(b).is_ok()
    }

    /// The count of bytes waiting, for the writer task to take what it has sent off. (The
    /// writer must not hold an `Outbox` itself: that would keep its own queue open forever, so
    /// it would never finish, and kept the connection — and its memory — after it closed.)
    pub fn queued(&self) -> Arc<AtomicUsize> {
        self.queued.clone()
    }
}

/// Connection counts per place (see [`Place`]), and wrong-invite counts (for throttling).
#[derive(Default)]
struct Limits {
    per_ip: HashMap<Place, usize>,
    per_network: HashMap<Place, usize>,
    total: usize,
    handshaking: usize,
    /// Not logged in yet: in all, and per place and network.
    waiting: usize,
    waiting_per_ip: HashMap<Place, usize>,
    waiting_per_network: HashMap<Place, usize>,
    /// Wrong invite codes per place (an IPv6 /64 is one place, as for connections).
    bad_invites: HashMap<Place, (u32, Instant)>,
    /// Tokens for new members joining (see [`JOIN_BURST`]).
    join_tokens: Option<(f64, Instant)>,
}

/// What to do with a new connection attempt.
pub enum Admit {
    Accept,
    /// Make the other side prove its address first (busy: maybe a spoofed flood).
    Retry,
    Refuse,
}

/// Counts a connection while it's alive (see [`Hub::admit`]).
pub struct ConnSlot {
    hub: Arc<Hub>,
    place: (Place, Place),
    handshaking: bool,
    waiting: bool,
}

/// One less in a count by place (gone from the map at zero).
fn uncount(m: &mut HashMap<Place, usize>, p: Place) {
    if let Some(n) = m.get_mut(&p) {
        *n = n.saturating_sub(1);
        if *n == 0 {
            m.remove(&p);
        }
    }
}

impl ConnSlot {
    /// The handshake is over (either way).
    pub fn handshake_done(&mut self) {
        if std::mem::take(&mut self.handshaking) {
            self.hub.limits.lock().handshaking -= 1;
        }
    }

    /// Logged in: it no longer counts as waiting.
    pub fn logged_in(&mut self) {
        if std::mem::take(&mut self.waiting) {
            let mut l = self.hub.limits.lock();
            l.waiting -= 1;
            uncount(&mut l.waiting_per_ip, self.place.0);
            uncount(&mut l.waiting_per_network, self.place.1);
        }
    }
}

impl Drop for ConnSlot {
    fn drop(&mut self) {
        self.handshake_done();
        self.logged_in();
        let mut l = self.hub.limits.lock();
        l.total -= 1;
        uncount(&mut l.per_ip, self.place.0);
        uncount(&mut l.per_network, self.place.1);
    }
}

pub struct Hub {
    pub db: Db,
    pub fingerprint: [u8; 32],
    pub public_host: String,
    pub port: u16,
    /// Where attachments and profile pictures are kept (one file per id).
    pub files_dir: std::path::PathBuf,
    inner: Mutex<Inner>,
    next_conn: AtomicU64,
    next_session: AtomicU32,
    limits: Mutex<Limits>,
    /// Bug reports taken in the last hour: from each address, and in all (flood limits).
    bug_reports: Mutex<(HashMap<Place, Vec<Instant>>, Vec<Instant>)>,
    /// The same for plugins and skins sent in for the library,
    plugin_submissions: Mutex<(HashMap<Place, Vec<Instant>>, Vec<Instant>)>,
    /// from each signed sender,
    submission_senders: Mutex<HashMap<String, Vec<Instant>>>,
    /// and asks for one's own (`MySubmissions`) from each place.
    submission_asks: Mutex<HashMap<Place, Vec<Instant>>>,
    /// Uploads per member: bytes being received now, and what arrived in the last hour.
    uploads: Mutex<HashMap<UserId, (u64, Vec<(Instant, u64)>)>>,
    /// The invite directory at pyr.gg, while short links can work here (see
    /// [`crate::directory`]).
    pub directory: std::sync::OnceLock<crate::directory::Directory>,
    /// Why short links can't work here (the server's address only works on its own PC, or it
    /// was started without them).
    pub short_links_off_why: std::sync::OnceLock<String>,
    /// One change of the community's own code at a time.
    vanity_change: tokio::sync::Mutex<()>,
}

#[derive(Default)]
struct Inner {
    clients: HashMap<ConnId, Client>,
    voice: HashMap<SessionId, Voice>,
    /// How each person was (muted, deafened) when they last left voice, for a few minutes:
    /// sent back when they rejoin (see `VoiceReady::before`).
    left_voice: HashMap<UserId, (SelfVoiceState, Instant)>,
    /// The call each person was in last, and when they left it: a call moved to a group (see
    /// `Request::Ring`) is only followed from the call its caller has just left.
    last_call: HashMap<UserId, (ChannelId, Instant)>,
}

/// How long the server remembers how someone was when they left voice (an app update takes
/// well under this).
const LEFT_VOICE_KEPT: Duration = Duration::from_secs(15 * 60);
/// How soon after leaving a call its caller can move it (adding people makes a group and moves
/// the call there in a moment).
const MOVED_CALL_WITHIN: Duration = Duration::from_secs(60);

/// What a moderator did to someone in voice (see `Hub::set_moderated`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeratedVoice {
    Muted,
    Deafened,
}

impl ModeratedVoice {
    /// Where the people it's on are kept (a JSON list of user ids in `meta`).
    fn key(self) -> &'static str {
        match self {
            ModeratedVoice::Muted => "server_muted",
            ModeratedVoice::Deafened => "server_deafened",
        }
    }
}

struct Client {
    user_id: UserId,
    /// The app knows group conversations (`GROUPS_CAP`): only such apps hear of them.
    groups: bool,
    /// The app adjusts its shares to its viewers (`VIDEO_FEEDBACK_CAP`).
    feedback: bool,
    conn: quinn::Connection,
    tx: Outbox,
    session: Option<SessionId>,
    session_cell: Arc<AtomicU32>,
    last_typing: HashMap<ChannelId, Instant>,
    /// Its Pyr says it's idle (`SetIdle`).
    idle: bool,
    /// Its person said Do Not Disturb there (`SetDnd`).
    dnd: bool,
}

struct Voice {
    p: VoiceParticipant,
    /// Who sees it: a direct message's or group's call's people (see `DM_CALLS_FEATURE`), or
    /// those who can see a private voice channel; everyone when None.
    audience: Option<Vec<UserId>>,
    /// A direct message's or group's call (not the server's to moderate).
    conversation: bool,
    conn_id: ConnId,
    conn: quinn::Connection,
    tx: Outbox,
    /// published track -> subscriber session -> forwarder
    subs: HashMap<u8, HashMap<SessionId, Subscriber>>,
    last_keyframe_request: HashMap<u8, Instant>,
    /// Its app adjusts its shares to its viewers: it's told how they're getting on.
    feedback: bool,
}

struct Subscriber {
    conn: quinn::Connection,
    tx: mpsc::Sender<(u64, Bytes)>,
    state: Arc<SubState>,
}

/// Per (subscriber, published track) congestion state.
pub struct SubState {
    need_keyframe: AtomicBool,
    /// The keyframe they wait for is to catch them up after falling behind (not their first,
    /// when they've just started watching): the frames skipped meanwhile are shed, and their
    /// sharer hears so and sends less.
    behind: AtomicBool,
    /// Frames on their way: (when they were handed over, their size).
    unacked: Mutex<BTreeMap<u64, (Instant, usize)>>,
    next_index: AtomicU64,
    /// Before 0.8.0 (`PYR_AB=shed`): how old a frame on its way could get before the viewer
    /// was taken to be stuck.
    old_max_age: Duration,
    /// Bytes that got to them, and frames skipped for them, since the last report.
    delivered: AtomicU64,
    shed: AtomicU32,
    /// For the reports: their round trips lately (when, srtt), their losses at the last
    /// report, and when that was.
    rtts: Mutex<std::collections::VecDeque<(Instant, Duration)>>,
    last_report: Mutex<Option<(Instant, u64, u64)>>,
    /// What got to them, bit/s, as of the last report.
    rate: AtomicU64,
}

enum Offer {
    Send(u64),
    /// Not to them: they wait for a keyframe. `shed`: because they fell behind (not because
    /// they've only just started watching).
    Skip { shed: bool },
}

impl SubState {
    fn new(rtt: Duration) -> SubState {
        SubState {
            need_keyframe: AtomicBool::new(true),
            behind: AtomicBool::new(false),
            unacked: Mutex::new(BTreeMap::new()),
            next_index: AtomicU64::new(1),
            old_max_age: Duration::from_millis(350) + rtt.min(Duration::from_millis(400)),
            delivered: AtomicU64::new(0),
            shed: AtomicU32::new(0),
            rtts: Mutex::new(std::collections::VecDeque::new()),
            last_report: Mutex::new(None),
            rate: AtomicU64::new(0),
        }
    }

    /// How late the oldest frame on its way to them is: beyond a round trip and the time its
    /// own bytes take at the rate they've been getting (with room for a keyframe's).
    fn late(&self, rtt: Duration) -> Duration {
        let Some((_, (at, bytes))) = self.unacked.lock().first_key_value().map(|(i, v)| (*i, *v)) else { return Duration::ZERO };
        let rate = self.rate.load(Ordering::Relaxed).max(300_000) as f64;
        let own = Duration::from_secs_f64(bytes as f64 * 8.0 * 1.5 / rate);
        at.elapsed().saturating_sub(rtt + own + Duration::from_millis(40))
    }

    /// Whether the next frame goes to them. `rtt`: their round trip now.
    fn offer(&self, keyframe: bool, bytes: usize, rtt: Duration) -> Offer {
        // Stuck: their oldest frame is so late (beyond its round trip and its own sending
        // time) that they'd be watching the past; skipping to a fresh keyframe catches them up.
        // (Their sharer is told how they're doing, see `Hub::feedback_loop`, and sends less
        // before this; it's for the one who can't keep up whatever happens.)
        let congested = if pyr_proto::ab::old("shed") {
            self.unacked.lock().first_key_value().map(|(_, (t, _))| t.elapsed() > self.old_max_age).unwrap_or(false)
        } else {
            self.late(rtt) > (rtt * 2).max(Duration::from_millis(700))
        };
        if congested {
            self.behind.store(true, Ordering::Relaxed);
            if !self.need_keyframe.swap(true, Ordering::Relaxed) {
                // (Each read on its own: `late` locks `unacked` too.)
                let unacked = self.unacked.lock().len();
                let late_ms = self.late(rtt).as_millis() as u64;
                debug!(target: "pyr_media", ev = "congested", unacked, late_ms);
            }
            return Offer::Skip { shed: true };
        }
        if self.need_keyframe.load(Ordering::Relaxed) {
            if !keyframe {
                return Offer::Skip { shed: self.behind.load(Ordering::Relaxed) };
            }
            self.need_keyframe.store(false, Ordering::Relaxed);
            self.behind.store(false, Ordering::Relaxed);
        }
        let idx = self.next_index.fetch_add(1, Ordering::Relaxed);
        self.unacked.lock().insert(idx, (Instant::now(), bytes));
        Offer::Send(idx)
    }

    fn ack(&self, idx: u64, delivered: Option<usize>) {
        self.unacked.lock().remove(&idx);
        if let Some(b) = delivered {
            self.delivered.fetch_add(b as u64, Ordering::Relaxed);
        }
    }

    /// How they're getting on (for their sharer's `Event::VideoFeedback`), from their
    /// connection: since the last report.
    fn report(&self, session: SessionId, conn: &quinn::Connection, now: Instant) -> ViewerFeedback {
        let p = conn.stats().path;
        let base = {
            let mut r = self.rtts.lock();
            if r.is_empty() && !p.min_rtt.is_zero() {
                r.push_back((now, p.min_rtt));
            }
            r.push_back((now, p.rtt));
            while r.front().is_some_and(|(t, _)| now.duration_since(*t) > Duration::from_secs(12)) {
                r.pop_front();
            }
            r.iter().map(|(_, d)| *d).min().unwrap_or(p.rtt)
        };
        let delivered = self.delivered.swap(0, Ordering::Relaxed);
        let (kbps, loss_pm) = {
            let mut last = self.last_report.lock();
            let out = match *last {
                Some((t, lost, sent)) => {
                    let dt = now.duration_since(t).as_secs_f64().max(0.05);
                    let bps = delivered as f64 * 8.0 / dt;
                    self.rate.store(bps as u64, Ordering::Relaxed);
                    let ds = p.sent_packets.saturating_sub(sent);
                    let pm = if ds > 0 { p.lost_packets.saturating_sub(lost) * 1000 / ds } else { 0 };
                    ((bps / 1000.0) as u32, pm as u32)
                }
                None => (0, 0),
            };
            *last = Some((now, p.lost_packets, p.sent_packets));
            out
        };
        ViewerFeedback {
            session,
            rtt_ms: p.rtt.as_millis() as u32,
            queue_ms: p.rtt.saturating_sub(base).as_millis() as u32,
            late_ms: self.late(p.rtt).as_millis() as u32,
            kbps,
            loss_pm,
            shed: self.shed.swap(0, Ordering::Relaxed),
        }
    }
}

/// Sends frames to one subscriber, one QUIC stream per frame. Frames that are not
/// acknowledged within two seconds are abandoned (stream reset) so they can't pile up.
async fn forward_task(conn: quinn::Connection, mut rx: mpsc::Receiver<(u64, Bytes)>, state: Arc<SubState>) {
    let prioritize = !pyr_proto::ab::old("fair");
    while let Some((idx, frame)) = rx.recv().await {
        let sent = Instant::now();
        let mut stream = match conn.open_uni().await {
            Ok(s) => s,
            Err(_) => break,
        };
        // Oldest frames first (see `media::frame_priority`).
        if prioritize {
            let _ = stream.set_priority(pyr_proto::media::frame_priority(idx));
        }
        if stream.write_all(&frame).await.is_err() {
            state.ack(idx, None);
            continue;
        }
        let _ = stream.finish();
        let st = state.clone();
        let bytes = frame.len();
        tokio::spawn(async move {
            let done = tokio::time::timeout(Duration::from_secs(2), stream.stopped()).await;
            if done.is_err() {
                let _ = stream.reset(VarInt::from_u32(0));
                st.behind.store(true, Ordering::Relaxed);
                st.need_keyframe.store(true, Ordering::Relaxed);
            }
            let ok = matches!(done, Ok(Ok(None)));
            debug!(target: "pyr_media", ev = "fwd_ack", idx, bytes, ms = sent.elapsed().as_millis() as u64, ok);
            st.ack(idx, ok.then_some(bytes));
        });
    }
}

fn env_mb(name: &str, default_mb: u64) -> u64 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default_mb) << 20
}

/// Room reserved for one upload (see [`Hub::reserve_upload`]).
pub struct UploadSlot<'a> {
    hub: &'a Hub,
    uid: String,
    size: u64,
    done: bool,
}

impl UploadSlot<'_> {
    /// The file was stored: count it towards the hourly allowance.
    pub fn stored(mut self) {
        self.done = true;
        let mut up = self.hub.uploads.lock();
        if let Some(e) = up.get_mut(&self.uid) {
            e.0 = e.0.saturating_sub(self.size);
            e.1.push((Instant::now(), self.size));
        }
    }
}

impl Drop for UploadSlot<'_> {
    fn drop(&mut self) {
        if !self.done {
            let mut up = self.hub.uploads.lock();
            if let Some(e) = up.get_mut(&self.uid) {
                e.0 = e.0.saturating_sub(self.size);
            }
        }
    }
}

pub fn frame(re: u64, ev: Event) -> Bytes {
    Bytes::from(framing::encode(&ServerFrame { re, ev }))
}

impl Hub {
    pub fn new(db: Db, fingerprint: [u8; 32], public_host: String, port: u16, files_dir: std::path::PathBuf) -> Arc<Hub> {
        let _ = std::fs::create_dir_all(&files_dir);
        Arc::new(Hub {
            db,
            fingerprint,
            public_host,
            port,
            files_dir,
            inner: Mutex::new(Inner::default()),
            next_conn: AtomicU64::new(1),
            next_session: AtomicU32::new(1),
            limits: Mutex::new(Limits::default()),
            bug_reports: Mutex::new((HashMap::new(), Vec::new())),
            plugin_submissions: Mutex::new((HashMap::new(), Vec::new())),
            submission_senders: Mutex::new(HashMap::new()),
            submission_asks: Mutex::new(HashMap::new()),
            uploads: Mutex::new(HashMap::new()),
            directory: std::sync::OnceLock::new(),
            short_links_off_why: std::sync::OnceLock::new(),
            vanity_change: tokio::sync::Mutex::new(()),
        })
    }

    // ------------------------------------------------------------ short links

    /// The invite directory, while short links can work here at all.
    pub fn directory(&self) -> Option<&crate::directory::Directory> {
        self.directory.get()
    }

    /// Short links are on: the server lists its invites at pyr.gg.
    pub fn short_links_on(&self) -> bool {
        self.directory().is_some() && self.db.get_meta("short_links").as_deref() != Some("0")
    }

    /// Something pyr.gg is told about changed (an invite taken back or used up, the name):
    /// it's told again in a moment.
    pub fn invites_changed(&self) {
        if let Some(d) = self.directory() {
            d.nudge();
        }
    }

    /// An invite just made, with its short link once pyr.gg lists it (or why it doesn't).
    pub async fn with_short_link(&self, ev: Event) -> Event {
        let Event::InviteCreated { code, link, expires_at, max_uses, uses, .. } = ev else { return ev };
        let mut short = None;
        let mut short_problem = None;
        let listed = self.db.invite(&code).is_some_and(|i| i.listed);
        if let (Some(dir), true) = (self.directory(), listed && self.short_links_on()) {
            match dir.sync().await {
                Ok(r) if !r.taken.contains(&code) => short = Some(pyr_proto::directory::short_url(&code)),
                Ok(_) => short_problem = Some("pyr.gg has that code for another community already (very rare): this invite works as a full link.".into()),
                Err(e) => short_problem = Some(format!("Couldn't list it at pyr.gg ({e}): this invite works as a full link.")),
            }
        }
        Event::InviteCreated { code, link, short, short_problem, expires_at, max_uses, uses }
    }

    /// The invites that still work (`ListInvites`): everyone's (`all`) or `uid`'s.
    pub fn invites_event(&self, uid: &str, all: bool) -> Event {
        let on = self.short_links_on();
        let vanity = self.db.get_meta("vanity").filter(|v| !v.is_empty());
        let invites = self
            .db
            .usable_invites(if all { None } else { Some(uid) })
            .into_iter()
            .filter(|i| Some(&i.code) != vanity.as_ref())
            .map(|i| InviteInfo {
                short: (on && i.listed).then(|| pyr_proto::directory::short_url(&i.code)),
                code: i.code,
                created_by: i.created_by,
                created_at: i.created_at,
                uses: i.uses,
                max_uses: i.max_uses,
                expires_at: i.expires_at,
                bot: i.bot,
            })
            .collect();
        Event::Invites { invites, vanity: vanity.filter(|_| on), short_links: on, short_links_problem: self.short_links_off_why.get().cloned() }
    }

    /// Gives the community its own code (`SetVanity`; the caller checked it's the owner):
    /// an everyday invite with that code, listed at pyr.gg, that never runs out. The old one
    /// stops working. Answered once pyr.gg has it (or says why not).
    pub async fn set_vanity(&self, owner: &str, code: Option<String>) -> Reply {
        let _one = self.vanity_change.lock().await;
        let Some(dir) = self.directory() else {
            let why = self.short_links_off_why.get().cloned().unwrap_or_else(|| "Short links are off for this server.".into());
            return err(ErrorCode::BadRequest, why);
        };
        if !self.short_links_on() {
            return err(ErrorCode::BadRequest, "Turn short links on first.");
        }
        let new = code.map(|c| c.trim().to_ascii_lowercase()).filter(|c| !c.is_empty());
        let old = self.db.get_meta("vanity").filter(|v| !v.is_empty());
        if new == old {
            return Ok(self.invites_event(owner, true));
        }
        let Some(new) = new else {
            // Taken away: its invite stops working now, and pyr.gg hears of it in a moment.
            if let Some(o) = &old {
                let _ = self.db.delete_invite(o);
            }
            let _ = self.db.set_meta("vanity", "");
            dir.nudge();
            return Ok(self.invites_event(owner, true));
        };
        if let Some(why) = pyr_proto::directory::vanity_problem(&new) {
            return err(ErrorCode::BadRequest, why);
        }
        if self.db.invite(&new).is_some() {
            return err(ErrorCode::BadRequest, "That's one of this server's invites already. Pick another.");
        }
        let made = self
            .db
            .create_invite(&new, owner, None, None, Role::Member)
            .and_then(|()| self.db.set_listed(&new, true))
            .and_then(|()| self.db.set_meta("vanity", &new));
        if let Err(e) = made {
            let _ = self.db.delete_invite(&new);
            let _ = self.db.set_meta("vanity", old.as_deref().unwrap_or(""));
            return err(ErrorCode::Internal, e.to_string());
        }
        // (The old one isn't listed in what pyr.gg is told now, as anything.)
        if let Some(o) = &old {
            let _ = self.db.set_listed(o, false);
        }
        let refused = match dir.sync().await {
            Ok(r) => r.vanity_refused,
            Err(e) => Some(format!("Couldn't reach pyr.gg: {e}. Try again in a moment.")),
        };
        match refused {
            None => {
                if let Some(o) = &old {
                    let _ = self.db.delete_invite(o);
                }
                Ok(self.invites_event(owner, true))
            }
            Some(why) => {
                // Back to how it was.
                let _ = self.db.delete_invite(&new);
                let _ = self.db.set_meta("vanity", old.as_deref().unwrap_or(""));
                if let Some(o) = &old {
                    let _ = self.db.set_listed(o, true);
                }
                dir.nudge();
                err(ErrorCode::BadRequest, why)
            }
        }
    }

    /// Whether to take a new connection from `ip`; while it's alive, keep the slot.
    pub fn admit(self: &Arc<Self>, ip: IpAddr, address_validated: bool) -> (Admit, Option<ConnSlot>) {
        let place = Place::both(ip);
        let mut l = self.limits.lock();
        let count = |m: &HashMap<Place, usize>, p: Place| m.get(&p).copied().unwrap_or(0);
        if l.total >= max_connections()
            || count(&l.per_ip, place.0) >= MAX_CONNECTIONS_PER_IP
            || count(&l.per_network, place.1) >= MAX_CONNECTIONS_PER_NETWORK
            || l.waiting >= MAX_WAITING
            || count(&l.waiting_per_ip, place.0) >= MAX_WAITING_PER_IP
            || count(&l.waiting_per_network, place.1) >= MAX_WAITING_PER_NETWORK
        {
            return (Admit::Refuse, None);
        }
        // Lots of handshakes at once: make new ones prove their address first.
        if !address_validated && l.handshaking >= 32 {
            return (Admit::Retry, None);
        }
        l.total += 1;
        l.handshaking += 1;
        l.waiting += 1;
        *l.per_ip.entry(place.0).or_default() += 1;
        *l.per_network.entry(place.1).or_default() += 1;
        *l.waiting_per_ip.entry(place.0).or_default() += 1;
        *l.waiting_per_network.entry(place.1).or_default() += 1;
        (Admit::Accept, Some(ConnSlot { hub: self.clone(), place, handshaking: true, waiting: true }))
    }

    /// Whether `ip` has guessed too many invite codes lately.
    fn invite_throttled(&self, ip: IpAddr) -> bool {
        let place = Place::of(ip, 64);
        let mut l = self.limits.lock();
        match l.bad_invites.get(&place) {
            Some((n, since)) if since.elapsed() < BAD_INVITE_WINDOW => *n >= BAD_INVITES_PER_IP,
            Some(_) => {
                l.bad_invites.remove(&place);
                false
            }
            None => false,
        }
    }

    fn note_bad_invite(&self, ip: IpAddr) {
        let mut l = self.limits.lock();
        if l.bad_invites.len() > 100_000 {
            l.bad_invites.retain(|_, (_, t)| t.elapsed() < BAD_INVITE_WINDOW);
        }
        let e = l.bad_invites.entry(Place::of(ip, 64)).or_insert((0, Instant::now()));
        if e.1.elapsed() >= BAD_INVITE_WINDOW {
            *e = (0, Instant::now());
        }
        e.0 += 1;
    }

    /// One new member may join now (a slow trickle is always fine; a flood of fresh
    /// accounts is not).
    fn take_join_token(&self) -> bool {
        let mut l = self.limits.lock();
        let now = Instant::now();
        let (tokens, at) = l.join_tokens.unwrap_or((JOIN_BURST, now));
        let tokens = (tokens + now.duration_since(at).as_secs_f64() * JOINS_PER_HOUR / 3600.0).min(JOIN_BURST);
        if tokens < 1.0 {
            l.join_tokens = Some((tokens, now));
            return false;
        }
        l.join_tokens = Some((tokens - 1.0, now));
        true
    }

    /// Reserves room for an upload of `size` bytes by `uid`: within the server's storage, the
    /// member's own share of it, and how much one member may upload in an hour.
    pub fn reserve_upload(&self, uid: &str, size: u64) -> Result<UploadSlot<'_>, String> {
        let total_cap = env_mb("PYR_STORAGE_MB", 5 * 1024);
        // Each member's share: at most a tenth of it (a few accounts can't fill it), and much
        // less in their first days (new accounts made to fill it get little each).
        let mut user_cap = env_mb("PYR_USER_STORAGE_MB", 1024).min(total_cap / 10);
        if self.db.get_user(uid).is_some_and(|u| now_ms() - u.joined_at < NEW_MEMBER_MS) {
            user_cap = user_cap.min(env_mb("PYR_NEW_MEMBER_STORAGE_MB", 100));
        }
        let hourly_cap = env_mb("PYR_USER_UPLOAD_MB_PER_HOUR", 400);
        let mut up = self.uploads.lock();
        let in_flight_all: u64 = up.values().map(|v| v.0).sum();
        let mine = up.entry(uid.to_string()).or_default();
        mine.1.retain(|(t, _)| t.elapsed() < Duration::from_secs(3600));
        let last_hour: u64 = mine.1.iter().map(|(_, n)| n).sum();
        if self.db.files_total() + in_flight_all + size > total_cap {
            return Err("This server's storage for images is full.".into());
        }
        if self.db.files_total_of(uid) + mine.0 + size > user_cap {
            return Err("You've used all the image storage this server gives each person. Delete some old images first.".into());
        }
        if last_hour + mine.0 + size > hourly_cap {
            return Err("You've uploaded a lot in the last hour. Try again a bit later.".into());
        }
        if self.db.unsent_files_of(uid) >= 40 {
            return Err("Too many images waiting to be sent. Send or remove some first.".into());
        }
        mine.0 += size;
        Ok(UploadSlot { hub: self, uid: uid.to_string(), size, done: false })
    }

    pub fn alloc_conn_id(&self) -> ConnId {
        self.next_conn.fetch_add(1, Ordering::Relaxed)
    }

    pub fn info(&self) -> ServerInfo {
        ServerInfo {
            name: self.db.get_meta("name").unwrap_or_else(|| "Pyr Server".into()),
            description: self.db.get_meta("description").unwrap_or_default(),
            fingerprint: b64::encode(&self.fingerprint),
            icon: self.db.get_meta("icon").filter(|i| !i.is_empty()),
            emojis: self.db.emojis(),
            sounds: self.db.sounds(),
            gifs: self.db.gifs(),
            giphy_key: self.db.get_meta("giphy_key").unwrap_or_default(),
            klipy_key: self.db.get_meta("klipy_key").unwrap_or_default(),
            bug_reports: self.db.get_meta("bug_reports").as_deref() == Some("1"),
            new_bug_reports: self.db.new_bug_count(),
            plugin_submissions: self.db.get_meta("plugin_submissions").as_deref() == Some("1"),
            new_plugin_submissions: self.db.new_submission_count(false),
            new_skin_submissions: self.db.new_submission_count(true),
        }
    }

    /// `info` as `user` is sent it: only the owner is told how many bug reports are waiting.
    /// Every member's app gets the GIF search keys: GIPHY and KLIPY want each search to come
    /// straight from the app, never through a server, so a key for them is one every member's
    /// app has (nobody else is sent them).
    fn info_as(mut info: ServerInfo, user: Option<&db::UserRow>) -> ServerInfo {
        if !user.is_some_and(|u| u.role == Role::Owner) {
            info.new_bug_reports = 0;
            info.new_plugin_submissions = 0;
            info.new_skin_submissions = 0;
        }
        if user.is_none() {
            info.giphy_key.clear();
            info.klipy_key.clear();
        }
        info
    }

    /// The server's info as `uid` is sent it (see `info_as`).
    pub fn info_for(&self, uid: &str) -> ServerInfo {
        Self::info_as(self.info(), self.db.get_user(uid).as_ref())
    }

    /// Tells every open app that the server's info changed, each as it's sent it.
    pub fn broadcast_info(&self) {
        let full = self.info();
        let inner = self.inner.lock();
        // (Three kinds of people at most: the owner, members, and anyone no longer one.)
        let mut frames: HashMap<(bool, bool), Bytes> = HashMap::new();
        for c in inner.clients.values() {
            let user = self.db.get_user(&c.user_id);
            let kind = (user.as_ref().is_some_and(|u| u.role == Role::Owner), user.is_none());
            let b = frames.entry(kind).or_insert_with(|| frame(0, Event::ServerUpdated { server: Self::info_as(full.clone(), user.as_ref()) }));
            let _ = c.tx.send(b.clone());
        }
    }

    pub fn invite_link(&self, code: &str) -> String {
        InviteLink::new(self.public_host.clone(), self.port, Some(code.to_string()), self.fingerprint).to_url()
    }

    pub fn online_count(&self) -> usize {
        self.inner.lock().clients.len()
    }

    /// People online (one each, however many of their devices are connected; bots too).
    pub fn online_people(&self) -> usize {
        self.inner.lock().clients.values().map(|c| &c.user_id).collect::<HashSet<_>>().len()
    }

    fn broadcast_locked(inner: &Inner, ev: Event) {
        let b = frame(0, ev);
        for c in inner.clients.values() {
            let _ = c.tx.send(b.clone());
        }
    }

    /// Who's in voice, changed: told to everyone, or for a private conversation's call only to
    /// its people.
    fn voice_news_locked(inner: &Inner, audience: Option<&Vec<UserId>>, ev: Event) {
        let Some(who) = audience else { return Self::broadcast_locked(inner, ev) };
        let f = frame(0, ev);
        for c in inner.clients.values().filter(|c| who.contains(&c.user_id)) {
            let _ = c.tx.send(f.clone());
        }
    }

    pub fn broadcast(&self, ev: Event) {
        Self::broadcast_locked(&self.inner.lock(), ev);
    }

    /// Sends a channel's event to those who can see it: everyone, or for a direct message
    /// just its two people.
    pub fn broadcast_in(&self, channel_id: ChannelId, ev: Event) {
        self.broadcast_in_except(channel_id, None, ev)
    }

    fn broadcast_in_except(&self, channel_id: ChannelId, except: Option<ConnId>, ev: Event) {
        // A direct message's two people, or a group's members (with apps that know groups), or
        // those who can see a private channel.
        let dm = self.db.dm_members(channel_id);
        let group = if dm.is_none() { self.db.group(channel_id).map(|g| g.members) } else { None };
        let viewers: Option<HashSet<UserId>> =
            if dm.is_none() && group.is_none() { self.db.channel(channel_id).filter(|c| c.private).map(|c| self.db.viewers(&c).into_iter().collect()) } else { None };
        let b = frame(0, ev);
        for (id, c) in self.inner.lock().clients.iter() {
            if Some(*id) == except {
                continue;
            }
            if viewers.as_ref().is_some_and(|v| !v.contains(&c.user_id)) {
                continue;
            }
            if let Some((a, z)) = &dm {
                if c.user_id != *a && c.user_id != *z {
                    continue;
                }
            }
            if let Some(members) = &group {
                if !c.groups || !members.contains(&c.user_id) {
                    continue;
                }
            }
            let _ = c.tx.send(b.clone());
        }
    }

    /// Sends to one member's other connections that know group conversations.
    pub fn send_to_group_members_except(&self, uid: &str, except: ConnId, ev: Event) {
        let b = frame(0, ev);
        for (id, c) in self.inner.lock().clients.iter() {
            if *id != except && c.groups && c.user_id == uid {
                let _ = c.tx.send(b.clone());
            }
        }
    }

    /// Sends to every open connection of these members that knows group conversations.
    pub fn send_to_group_members(&self, uids: &[UserId], ev: Event) {
        let b = frame(0, ev);
        for c in self.inner.lock().clients.values().filter(|c| c.groups && uids.contains(&c.user_id)) {
            let _ = c.tx.send(b.clone());
        }
    }

    /// A group's members changed: its call is seen by who's in it now. Who left leaves the
    /// call; who came gets told who's in it.
    pub fn group_call_members(&self, channel_id: ChannelId, members: &[UserId], left: Option<&str>) {
        let mut inner = self.inner.lock();
        let gone: Vec<SessionId> =
            inner.voice.values().filter(|v| v.p.channel_id == channel_id && Some(v.p.user_id.as_str()) == left).map(|v| v.p.session).collect();
        for s in gone {
            Self::remove_session_locked(&mut inner, s);
        }
        let mut news = Vec::new();
        for v in inner.voice.values_mut().filter(|v| v.p.channel_id == channel_id) {
            v.audience = Some(members.to_vec());
            news.push(v.p.clone());
        }
        let audience = members.to_vec();
        for p in news {
            Self::voice_news_locked(&inner, Some(&audience), Event::VoiceUpserted { participant: p });
        }
    }

    /// Sends to every open connection of one member.
    pub fn send_to_user(&self, uid: &str, ev: Event) {
        let b = frame(0, ev);
        for c in self.inner.lock().clients.values() {
            if c.user_id == uid {
                let _ = c.tx.send(b.clone());
            }
        }
    }

    /// Whether someone's an admin (or the owner), and their roles: what decides which channels
    /// they see and post in.
    pub fn access_of(&self, uid: &str) -> (bool, Vec<RoleId>) {
        let admin = self.db.get_user(uid).is_some_and(|u| u.role.is_admin());
        (admin, self.db.member_roles(uid))
    }

    /// Whether someone can see a channel (a server channel: conversations are checked by who's
    /// in them).
    pub fn sees(&self, uid: &str, ch: &Channel) -> bool {
        if !ch.private {
            return true;
        }
        let (admin, roles) = self.access_of(uid);
        ch.visible_to(admin, &roles)
    }

    /// Who's in a private voice channel's voice sees it again as who can see it now (and
    /// everyone sees a channel's that isn't private anymore).
    fn refresh_voice_audiences_locked(&self, inner: &mut Inner, channels: &[Channel]) {
        for v in inner.voice.values_mut().filter(|v| !v.conversation) {
            if let Some(ch) = channels.iter().find(|c| c.id == v.p.channel_id) {
                v.audience = if ch.private { Some(self.db.viewers(ch)) } else { None };
            }
        }
    }

    /// Who's talking in a channel, for someone who can see it now (`VoiceUpserted`s), or their
    /// leaving, for someone who can't (`VoiceRemoved`s).
    fn voice_frames_locked(inner: &Inner, channel_id: ChannelId, visible: bool) -> Vec<Bytes> {
        inner
            .voice
            .values()
            .filter(|v| v.p.channel_id == channel_id)
            .map(|v| {
                if visible {
                    frame(0, Event::VoiceUpserted { participant: v.p.clone() })
                } else {
                    frame(0, Event::VoiceRemoved { session: v.p.session, user_id: v.p.user_id.clone(), channel_id })
                }
            })
            .collect()
    }

    /// A channel was made or changed: everyone who can see it is told (`ChannelUpserted`),
    /// everyone who can't is told it's gone (`ChannelDeleted`, if they had it); anyone in its
    /// voice who can't see it now is out of it.
    pub fn channel_changed(&self, ch: &Channel) {
        let viewers: HashSet<UserId> = if ch.private { self.db.viewers(ch).into_iter().collect() } else { HashSet::new() };
        let sees = |uid: &str| !ch.private || viewers.contains(uid);
        let mut inner = self.inner.lock();
        let out: Vec<SessionId> = inner.voice.values().filter(|v| v.p.channel_id == ch.id && !sees(&v.p.user_id)).map(|v| v.p.session).collect();
        for s in out {
            Self::remove_session_locked(&mut inner, s);
        }
        self.refresh_voice_audiences_locked(&mut inner, std::slice::from_ref(ch));
        let mut shown = vec![frame(0, Event::ChannelUpserted { channel: ch.clone() })];
        shown.extend(Self::voice_frames_locked(&inner, ch.id, true));
        let mut hidden = vec![frame(0, Event::ChannelDeleted { channel_id: ch.id })];
        hidden.extend(Self::voice_frames_locked(&inner, ch.id, false));
        for c in inner.clients.values() {
            for f in if sees(&c.user_id) { &shown } else { &hidden } {
                let _ = c.tx.send(f.clone());
            }
        }
    }

    /// Someone's roles (or admin) changed: their apps get the channels they can see now and
    /// lose the ones they can't, and they're out of a voice channel they can't see anymore.
    pub fn access_changed(&self, uid: &str) {
        let channels = self.db.channels();
        let (admin, roles) = self.access_of(uid);
        let mut inner = self.inner.lock();
        let out: Vec<SessionId> = inner
            .voice
            .values()
            .filter(|v| v.p.user_id == uid && !v.conversation)
            .filter(|v| channels.iter().any(|ch| ch.id == v.p.channel_id && !ch.visible_to(admin, &roles)))
            .map(|v| v.p.session)
            .collect();
        for s in out {
            Self::remove_session_locked(&mut inner, s);
        }
        self.refresh_voice_audiences_locked(&mut inner, &channels);
        let mut frames: Vec<Bytes> = Vec::new();
        for ch in &channels {
            let visible = ch.visible_to(admin, &roles);
            frames.push(if visible { frame(0, Event::ChannelUpserted { channel: ch.clone() }) } else { frame(0, Event::ChannelDeleted { channel_id: ch.id }) });
            // Only a private channel can have come or gone.
            if ch.private {
                frames.extend(Self::voice_frames_locked(&inner, ch.id, visible));
            }
        }
        for c in inner.clients.values().filter(|c| c.user_id == uid) {
            for f in &frames {
                let _ = c.tx.send(f.clone());
            }
        }
    }

    /// Connected right now, from anywhere.
    pub fn is_online(&self, uid: &str) -> bool {
        Self::is_online_locked(&self.inner.lock(), uid)
    }

    fn is_online_locked(inner: &Inner, uid: &str) -> bool {
        inner.clients.values().any(|c| c.user_id == uid)
    }

    /// Idle: connected, and idle on every connection.
    fn is_idle_locked(inner: &Inner, uid: &str) -> bool {
        let mut theirs = inner.clients.values().filter(|c| c.user_id == uid).peekable();
        theirs.peek().is_some() && theirs.all(|c| c.idle)
    }

    /// Do Not Disturb: said on any of their connections.
    fn is_dnd_locked(inner: &Inner, uid: &str) -> bool {
        inner.clients.values().any(|c| c.user_id == uid && c.dnd)
    }

    /// Do Not Disturb on a connection, or off (`SetDnd`): everyone's told when that changes
    /// how its member shows.
    pub fn set_dnd(&self, conn_id: ConnId, dnd: bool) {
        let mut inner = self.inner.lock();
        let Some(uid) = inner.clients.get(&conn_id).map(|c| c.user_id.clone()) else { return };
        let was = Self::is_dnd_locked(&inner, &uid);
        if let Some(c) = inner.clients.get_mut(&conn_id) {
            c.dnd = dnd;
        }
        if Self::is_dnd_locked(&inner, &uid) != was {
            if let Some(m) = self.member_locked(&inner, &uid) {
                Self::broadcast_locked(&inner, Event::MemberUpserted { member: m });
            }
        }
    }

    /// A connection's Pyr is idle, or back (`SetIdle`): everyone's told when that changes
    /// how its member shows.
    pub fn set_idle(&self, conn_id: ConnId, idle: bool) {
        let mut inner = self.inner.lock();
        let Some(uid) = inner.clients.get(&conn_id).map(|c| c.user_id.clone()) else { return };
        let was = Self::is_idle_locked(&inner, &uid);
        if let Some(c) = inner.clients.get_mut(&conn_id) {
            c.idle = idle;
        }
        if Self::is_idle_locked(&inner, &uid) != was {
            if let Some(m) = self.member_locked(&inner, &uid) {
                Self::broadcast_locked(&inner, Event::MemberUpserted { member: m });
            }
        }
    }

    fn member_locked(&self, inner: &Inner, uid: &str) -> Option<Member> {
        let u = self.db.get_user(uid)?;
        if u.banned {
            return None;
        }
        Some(Member {
            user_id: uid.to_string(),
            name: u.name,
            role: u.role,
            online: Self::is_online_locked(inner, uid),
            joined_at: u.joined_at,
            roles: self.db.member_roles(uid),
            avatar: u.avatar,
            idle: Self::is_idle_locked(inner, uid),
            bot: u.bot,
            dnd: Self::is_dnd_locked(inner, uid),
        })
    }

    /// Decide whether `uid` may log in, redeeming an invite for newcomers. `ip` is where
    /// they connect from (wrong invite codes from one address are throttled).
    pub fn authorize(&self, uid: &str, name: &str, invite: Option<&str>, ip: IpAddr) -> Result<(), (ErrorCode, String)> {
        match self.db.get_user(uid) {
            Some(u) if u.banned => err(ErrorCode::Banned, "You are banned from this server."),
            Some(u) => {
                let _ = self.db.touch_user(uid, name);
                // An existing member presenting an owner/admin invite gets upgraded. (Their
                // everyday invite isn't used up again each time they connect.)
                if let Some(code) = invite {
                    if self.db.invite_role(code).map(|r| r > u.role).unwrap_or(false) {
                        if let Ok(role) = self.db.redeem_invite(code) {
                            if role > u.role {
                                let _ = self.db.set_role(uid, role);
                            }
                        }
                    }
                }
                Ok(())
            }
            None => {
                let Some(code) = invite else {
                    return err(ErrorCode::InviteInvalid, "You need an invite link to join this server.");
                };
                if self.invite_throttled(ip) {
                    return err(ErrorCode::RateLimited, "Too many wrong invite links from your address. Try again in a few minutes.");
                }
                let bot = self.db.invite_is_bot(code);
                let role = match self.db.redeem_invite(code) {
                    Ok(r) => r,
                    Err(e) => {
                        self.note_bad_invite(ip);
                        return match e {
                            db::InviteError::NotFound => err(ErrorCode::InviteInvalid, "That invite doesn't exist."),
                            db::InviteError::Expired => err(ErrorCode::InviteInvalid, "That invite has expired."),
                            db::InviteError::UsedUp => err(ErrorCode::InviteInvalid, "That invite has already been used."),
                        };
                    }
                };
                // Owner links (and bot invites, one bot each) always work; everyday invites are
                // rate-limited against raids of throwaway accounts.
                if role < Role::Owner && !bot && !self.take_join_token() {
                    return err(ErrorCode::RateLimited, "Lots of people are joining right now. Try again in a few minutes.");
                }
                let role = if bot { Role::Member } else { role };
                if !self.db.insert_member(uid, name, role, bot).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    return err(ErrorCode::Banned, "You are banned from this server.");
                }
                let _ = self.db.set_joined_with(uid, code);
                info!(%uid, %name, ?role, bot, "new member joined");
                // (pyr.gg's count of members, and an invite that's now used up.)
                self.invites_changed();
                Ok(())
            }
        }
    }

    /// Register an authenticated connection and queue its `Ready` snapshot. Done under one
    /// lock so that no broadcast can slip in before `Ready`.
    pub fn register(
        &self,
        conn_id: ConnId,
        uid: &str,
        conn: quinn::Connection,
        tx: Outbox,
        session_cell: Arc<AtomicU32>,
        re: u64,
        groups: bool,
        feedback: bool,
    ) -> Result<(), (ErrorCode, String)> {
        let mut inner = self.inner.lock();
        // Checked again under the lock: a ban since the login was authorized wins.
        if !db::user_is_member(&self.db, &uid.to_string()) {
            return err(ErrorCode::Banned, "You are not a member of this server.");
        }
        if inner.clients.values().filter(|c| c.user_id == uid).count() >= MAX_CONNECTIONS_PER_USER {
            return err(ErrorCode::RateLimited, "You're connected from too many places at once.");
        }
        let was_online = Self::is_online_locked(&inner, uid);
        let was_idle = Self::is_idle_locked(&inner, uid);
        inner.clients.insert(
            conn_id,
            Client {
                user_id: uid.to_string(),
                groups,
                feedback,
                conn,
                tx: tx.clone(),
                session: None,
                session_cell,
                last_typing: HashMap::new(),
                idle: false,
                dnd: false,
            },
        );
        let mut members = self.db.members();
        for m in &mut members {
            m.online = Self::is_online_locked(&inner, &m.user_id);
            m.idle = Self::is_idle_locked(&inner, &m.user_id);
            m.dnd = Self::is_dnd_locked(&inner, &m.user_id);
        }
        let me = members.iter().find(|m| m.user_id == uid).cloned().unwrap_or(Member {
            user_id: uid.to_string(),
            online: true,
            joined_at: now_ms(),
            ..Default::default()
        });
        let (admin, my_roles) = (me.role.is_admin(), me.roles.clone());
        let channels: Vec<Channel> = self.db.channels().into_iter().filter(|c| c.visible_to(admin, &my_roles)).collect();
        let mut ready = Event::Ready {
            me: me.clone(),
            server: self.info_for(uid),
            channels,
            members,
            // (Private conversations' calls only for their own people.)
            voice: inner.voice.values().filter(|v| v.audience.as_ref().is_none_or(|who| who.iter().any(|w| w == uid))).map(|v| v.p.clone()).collect(),
            roles: self.db.roles(),
            features: [ROLES_FEATURE, FILES_FEATURE, AVATARS_FEATURE, DMS_FEATURE, ICONS_FEATURE, EMOJI_FEATURE, FRIENDS_FEATURE, BACKUP_FEATURE, GIPHY_FEATURE, BUGS_FEATURE, CHANNEL_NEWEST_FEATURE, SOUNDBOARD_FEATURE, VOICE_MOD_FEATURE, KLIPY_FEATURE, PLUGINS_FEATURE, DM_CALLS_FEATURE, GROUPS_FEATURE, IDLE_FEATURE, DND_FEATURE, BIG_FILES_FEATURE, GIFS_FEATURE, HANDSHAKE_FEATURE, VOUCHES_FEATURE, VOUCHES_FOR_ME_FEATURE, PRIVATE_CHANNELS_FEATURE, BOTS_FEATURE, INVITES_FEATURE].iter().map(|s| s.to_string()).collect(),
            dms: self.db.dms_of(uid),
            relations: self.db.relations_of(uid),
            groups: if groups { self.db.groups_of(uid) } else { Vec::new() },
        };
        // A very big community could outgrow one frame: then leave out members who've been
        // away longest (their names show up again when they come back).
        let mut b = frame(re, ready.clone());
        while b.len() > pyr_proto::MAX_CONTROL_FRAME {
            let Event::Ready { members, .. } = &mut ready else { break };
            let offline: Vec<usize> = (0..members.len()).filter(|i| !members[*i].online && members[*i].user_id != uid).collect();
            if offline.is_empty() {
                break;
            }
            let drop: HashSet<usize> = offline[..offline.len().div_ceil(2)].iter().copied().collect();
            let mut i = 0;
            members.retain(|_| {
                i += 1;
                !drop.contains(&(i - 1))
            });
            b = frame(re, ready.clone());
        }
        tx.send(b);
        // Online now, or (here again from somewhere) no longer idle.
        if !was_online || was_idle {
            let b = frame(0, Event::MemberUpserted { member: me });
            for (id, c) in inner.clients.iter() {
                if *id != conn_id {
                    let _ = c.tx.send(b.clone());
                }
            }
        }
        Ok(())
    }

    pub fn unregister(&self, conn_id: ConnId) {
        let mut inner = self.inner.lock();
        if let Some(session) = inner.clients.get(&conn_id).and_then(|c| c.session) {
            Self::remove_session_locked(&mut inner, session);
        }
        let was_idle = inner.clients.get(&conn_id).map(|c| Self::is_idle_locked(&inner, &c.user_id));
        let was_dnd = inner.clients.get(&conn_id).map(|c| Self::is_dnd_locked(&inner, &c.user_id));
        if let Some(c) = inner.clients.remove(&conn_id) {
            // Gone, idle now that the one place they weren't idle has gone, or no longer Do
            // Not Disturb now that the place they said so has gone.
            if !Self::is_online_locked(&inner, &c.user_id)
                || was_idle != Some(Self::is_idle_locked(&inner, &c.user_id))
                || was_dnd != Some(Self::is_dnd_locked(&inner, &c.user_id))
            {
                if let Some(m) = self.member_locked(&inner, &c.user_id) {
                    Self::broadcast_locked(&inner, Event::MemberUpserted { member: m });
                }
            }
        }
    }

    // ------------------------------------------------------------------ voice

    /// Updates a screen share's list of viewers from who's subscribed; true if it changed.
    fn sync_viewers(v: &mut Voice) -> bool {
        let mut viewers: Vec<SessionId> = v.subs.get(&track::SCREEN).map(|m| m.keys().copied().collect()).unwrap_or_default();
        viewers.sort_unstable();
        match v.p.tracks.iter_mut().find(|t| t.id == track::SCREEN) {
            Some(t) if t.viewers != viewers => {
                t.viewers = viewers;
                true
            }
            _ => false,
        }
    }

    fn remove_session_locked(inner: &mut Inner, session: SessionId) {
        let Some(v) = inner.voice.remove(&session) else { return };
        inner.left_voice.retain(|_, (_, at)| at.elapsed() < LEFT_VOICE_KEPT);
        let was = SelfVoiceState { self_mute: v.p.self_mute, self_deaf: v.p.self_deaf };
        inner.left_voice.insert(v.p.user_id.clone(), (was, Instant::now()));
        inner.last_call.retain(|_, (_, at)| at.elapsed() < MOVED_CALL_WITHIN);
        inner.last_call.insert(v.p.user_id.clone(), (v.p.channel_id, Instant::now()));
        let mut changed = Vec::new();
        for other in inner.voice.values_mut() {
            for subs in other.subs.values_mut() {
                subs.remove(&session);
            }
            if Self::sync_viewers(other) {
                changed.push((other.p.clone(), other.audience.clone()));
            }
        }
        for (p, audience) in changed {
            Self::voice_news_locked(inner, audience.as_ref(), Event::VoiceUpserted { participant: p });
        }
        if let Some(c) = inner.clients.get_mut(&v.conn_id) {
            if c.session == Some(session) {
                c.session = None;
                c.session_cell.store(0, Ordering::Relaxed);
            }
        }
        Self::voice_news_locked(
            inner,
            v.audience.as_ref(),
            Event::VoiceRemoved { session, user_id: v.p.user_id.clone(), channel_id: v.p.channel_id },
        );
    }

    fn session_of(inner: &Inner, conn_id: ConnId) -> Option<SessionId> {
        inner.clients.get(&conn_id).and_then(|c| c.session)
    }

    pub fn join_voice(
        &self,
        conn_id: ConnId,
        channel_id: ChannelId,
        e2ee_pub: String,
        e2ee_sig: String,
        self_mute: bool,
        self_deaf: bool,
    ) -> Reply {
        // A voice channel, or a private conversation's call: a direct message's (for its two
        // people, while neither has blocked the other) or a group's (for its members).
        let dm = self.db.dm_members(channel_id);
        // (A private voice channel is seen, and talked in, only by those who can see it.)
        let (audience, conversation) = match self.db.channel(channel_id) {
            Some(c) if c.kind == ChannelKind::Voice => (c.private.then(|| self.db.viewers(&c)), false),
            Some(_) => return err(ErrorCode::NotFound, "No such voice channel."),
            None => match self.db.private_members(channel_id) {
                Some(who) => (Some(who), true),
                None => return err(ErrorCode::NotFound, "No such voice channel."),
            },
        };
        let mut inner = self.inner.lock();
        let Some(client) = inner.clients.get(&conn_id) else { return err(ErrorCode::Unauthorized, "not logged in") };
        let uid = client.user_id.clone();
        // Bots chat; they don't talk.
        if self.db.get_user(&uid).is_some_and(|u| u.bot) {
            return err(ErrorCode::Forbidden, "Bots can't join voice channels.");
        }
        if let Some(who) = &audience {
            if !who.contains(&uid) || (conversation && dm.is_none() && !client.groups) {
                return err(ErrorCode::NotFound, "No such voice channel.");
            }
        }
        if let Some((a, b)) = &dm {
            if self.db.blocked_between(a, b) {
                return err(ErrorCode::Forbidden, "You can't call this person.");
            }
        }
        let (Some(pk), Some(sig)) = (b64::decode_array::<32>(&e2ee_pub), b64::decode_array::<64>(&e2ee_sig)) else {
            return err(ErrorCode::BadRequest, "bad E2EE key");
        };
        if !crypto::verify_binding(&uid, channel_id, &pk, &sig) {
            return err(ErrorCode::BadRequest, "E2EE key signature is invalid");
        }
        // Passed on to everyone as we'd write them, not as sent (padding, spaces...).
        let (e2ee_pub, e2ee_sig) = (b64::encode(&pk), b64::encode(&sig));
        // One voice session per person: leave any previous one (this or another device).
        let stale: Vec<SessionId> = inner.voice.values().filter(|v| v.p.user_id == uid).map(|v| v.p.session).collect();
        for s in stale {
            Self::remove_session_locked(&mut inner, s);
        }
        let before = inner.left_voice.remove(&uid).filter(|(_, at)| at.elapsed() < LEFT_VOICE_KEPT).map(|(was, _)| was);
        let session = self.next_session.fetch_add(1, Ordering::Relaxed);
        let client = inner.clients.get_mut(&conn_id).expect("checked above");
        client.session = Some(session);
        client.session_cell.store(session, Ordering::Relaxed);
        // A moderator's mute or deafen follows them from channel to channel, and visit to visit
        // (but not into a private conversation's call: that isn't the server's to moderate).
        let server_mute = !conversation && self.moderated(ModeratedVoice::Muted).contains(&uid);
        let server_deaf = !conversation && self.moderated(ModeratedVoice::Deafened).contains(&uid);
        let p = VoiceParticipant {
            session,
            user_id: uid,
            channel_id,
            // Deafened means muted too (as `set_voice_state` keeps it).
            self_mute: self_mute || self_deaf,
            self_deaf,
            server_mute,
            server_deaf,
            tracks: Vec::new(),
            e2ee_pub,
            e2ee_sig,
        };
        let voice = Voice {
            p: p.clone(),
            audience: audience.clone(),
            conversation,
            conn_id,
            conn: client.conn.clone(),
            tx: client.tx.clone(),
            subs: HashMap::new(),
            last_keyframe_request: HashMap::new(),
            feedback: client.feedback,
        };
        inner.voice.insert(session, voice);
        Self::voice_news_locked(&inner, audience.as_ref(), Event::VoiceUpserted { participant: p });
        let participants = inner.voice.values().filter(|v| v.p.channel_id == channel_id).map(|v| v.p.clone()).collect();
        debug!(session, channel_id, "voice join");
        Ok(Event::VoiceReady { session, participants, before })
    }

    pub fn leave_voice(&self, conn_id: ConnId) -> Reply {
        let mut inner = self.inner.lock();
        if let Some(session) = Self::session_of(&inner, conn_id) {
            Self::remove_session_locked(&mut inner, session);
        }
        Ok(Event::Ok)
    }

    fn with_my_voice<F: FnOnce(&mut Voice) -> bool>(&self, conn_id: ConnId, f: F) -> Reply {
        let mut inner = self.inner.lock();
        let Some(session) = Self::session_of(&inner, conn_id) else {
            return err(ErrorCode::BadRequest, "not in a voice channel");
        };
        let Some(v) = inner.voice.get_mut(&session) else { return err(ErrorCode::BadRequest, "not in voice") };
        if f(v) {
            let (p, audience) = (v.p.clone(), v.audience.clone());
            Self::voice_news_locked(&inner, audience.as_ref(), Event::VoiceUpserted { participant: p });
        }
        Ok(Event::Ok)
    }

    pub fn set_voice_state(&self, conn_id: ConnId, self_mute: bool, self_deaf: bool) -> Reply {
        self.with_my_voice(conn_id, |v| {
            v.p.self_mute = self_mute || self_deaf;
            v.p.self_deaf = self_deaf;
            true
        })
    }

    /// Who's server muted, or server deafened (`ModeratedVoice`): kept with the server's
    /// settings, so it lasts.
    pub fn moderated(&self, what: ModeratedVoice) -> HashSet<UserId> {
        self.db.get_meta(what.key()).and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
    }

    /// Server mute or deafen someone, or take it off (who may is checked by the caller): it's
    /// kept, and if they're in voice now, everyone sees it and the server stops passing on
    /// their voice (muted) or everyone else's to them (deafened).
    pub fn set_moderated(&self, what: ModeratedVoice, user_id: &str, on: bool) -> Reply {
        let mut inner = self.inner.lock();
        let mut all = self.moderated(what);
        let changed = if on { all.insert(user_id.to_string()) } else { all.remove(user_id) };
        if changed {
            let mut list: Vec<&UserId> = all.iter().collect();
            list.sort();
            if let Err(e) = self.db.set_meta(what.key(), &serde_json::to_string(&list).unwrap_or_default()) {
                return err(ErrorCode::Internal, e.to_string());
            }
        }
        let mut now = Vec::new();
        // (Not in a private conversation's call: that isn't the server's to moderate.)
        for v in inner.voice.values_mut().filter(|v| v.p.user_id == user_id && !v.conversation) {
            let flag = match what {
                ModeratedVoice::Muted => &mut v.p.server_mute,
                ModeratedVoice::Deafened => &mut v.p.server_deaf,
            };
            if *flag != on {
                *flag = on;
                now.push((v.p.clone(), v.audience.clone()));
            }
        }
        for (p, audience) in now {
            Self::voice_news_locked(&inner, audience.as_ref(), Event::VoiceUpserted { participant: p });
        }
        Ok(Event::Ok)
    }

    /// They left this call a moment ago (see `MOVED_CALL_WITHIN`).
    pub fn just_left_call(&self, uid: &str, channel_id: ChannelId) -> bool {
        self.inner.lock().last_call.get(uid).is_some_and(|(c, at)| *c == channel_id && at.elapsed() < MOVED_CALL_WITHIN)
    }

    /// Who's in a call now (on any of their devices).
    pub fn in_call(&self, channel_id: ChannelId) -> HashSet<UserId> {
        self.inner.lock().voice.values().filter(|v| v.p.channel_id == channel_id).map(|v| v.p.user_id.clone()).collect()
    }

    /// The voice channel (or direct message's call) a connection is in.
    pub fn voice_channel_of(&self, conn_id: ConnId) -> Option<ChannelId> {
        let inner = self.inner.lock();
        Self::session_of(&inner, conn_id).and_then(|s| inner.voice.get(&s)).map(|v| v.p.channel_id)
    }

    /// Ends the call in the direct message between two people (one blocked the other).
    pub fn end_dm_call(&self, x: &str, y: &str) {
        let mut pair = vec![x.to_string(), y.to_string()];
        pair.sort();
        let mut inner = self.inner.lock();
        let sessions: Vec<SessionId> = inner
            .voice
            .values()
            .filter(|v| v.audience.as_ref().is_some_and(|a| a.len() == 2 && a.iter().all(|u| pair.contains(u))) && self.db.dm_members(v.p.channel_id).is_some())
            .map(|v| v.p.session)
            .collect();
        for s in sessions {
            Self::remove_session_locked(&mut inner, s);
        }
    }

    pub fn publish(&self, conn_id: ConnId, t: TrackInfo) -> Reply {
        let ok = matches!(
            (t.id, t.kind),
            (track::CAMERA, TrackKind::Camera) | (track::SCREEN, TrackKind::Screen) | (track::SCREEN_AUDIO, TrackKind::ScreenAudio)
        );
        if !ok || t.width > 7680 || t.height > 4320 || t.fps > 240 {
            return err(ErrorCode::BadRequest, "invalid track");
        }
        let mut t = t;
        t.viewers.clear();
        self.with_my_voice(conn_id, |v| {
            v.p.tracks.retain(|x| x.id != t.id);
            v.p.tracks.push(t);
            v.p.tracks.sort_by_key(|x| x.id);
            Self::sync_viewers(v);
            true
        })
    }

    pub fn unpublish(&self, conn_id: ConnId, track_id: u8) -> Reply {
        self.with_my_voice(conn_id, |v| {
            v.subs.remove(&track_id);
            let before = v.p.tracks.len();
            v.p.tracks.retain(|x| x.id != track_id);
            before != v.p.tracks.len()
        })
    }

    pub fn subscribe(&self, conn_id: ConnId, session: SessionId, track_id: u8, subscribe: bool) -> Reply {
        let mut inner = self.inner.lock();
        let Some(me) = Self::session_of(&inner, conn_id) else {
            return err(ErrorCode::BadRequest, "join the voice channel first");
        };
        let Some(my_channel) = inner.voice.get(&me).map(|v| v.p.channel_id) else {
            return err(ErrorCode::BadRequest, "not in voice");
        };
        let (sub_conn, rtt) = match inner.clients.get(&conn_id) {
            Some(c) => (c.conn.clone(), c.conn.rtt()),
            None => return err(ErrorCode::Unauthorized, "not logged in"),
        };
        let Some(publisher) = inner.voice.get_mut(&session) else {
            return err(ErrorCode::NotFound, "that person left");
        };
        if publisher.p.channel_id != my_channel {
            return err(ErrorCode::Forbidden, "not in the same voice channel");
        }
        if !subscribe {
            if let Some(m) = publisher.subs.get_mut(&track_id) {
                m.remove(&me);
            }
            if Self::sync_viewers(publisher) {
                let (p, audience) = (publisher.p.clone(), publisher.audience.clone());
                Self::voice_news_locked(&inner, audience.as_ref(), Event::VoiceUpserted { participant: p });
            }
            return Ok(Event::Ok);
        }
        if track_id != track::CAMERA && track_id != track::SCREEN {
            return err(ErrorCode::BadRequest, "only video tracks can be subscribed");
        }
        if !publisher.p.tracks.iter().any(|t| t.id == track_id) {
            return err(ErrorCode::NotFound, "that stream has ended");
        }
        let subs = publisher.subs.entry(track_id).or_default();
        if !subs.contains_key(&me) {
            let state = Arc::new(SubState::new(rtt));
            let (tx, rx) = mpsc::channel(32);
            tokio::spawn(forward_task(sub_conn.clone(), rx, state.clone()));
            subs.insert(me, Subscriber { conn: sub_conn, tx, state });
        }
        // A new viewer always needs a keyframe.
        publisher.last_keyframe_request.insert(track_id, Instant::now());
        let _ = publisher.tx.send(frame(0, Event::KeyframeRequested { track: track_id }));
        if Self::sync_viewers(publisher) {
            let (p, audience) = (publisher.p.clone(), publisher.audience.clone());
            Self::voice_news_locked(&inner, audience.as_ref(), Event::VoiceUpserted { participant: p });
        }
        Ok(Event::Ok)
    }

    pub fn request_keyframe(&self, conn_id: ConnId, session: SessionId, track_id: u8) -> Reply {
        let mut inner = self.inner.lock();
        let Some(me) = Self::session_of(&inner, conn_id) else {
            return err(ErrorCode::BadRequest, "not in voice");
        };
        let my_channel = inner.voice.get(&me).map(|v| v.p.channel_id);
        if let Some(p) = inner.voice.get_mut(&session) {
            // Only for people watching it: same channel, and subscribed to that track.
            if Some(p.p.channel_id) == my_channel && p.subs.get(&track_id).map(|s| s.contains_key(&me)).unwrap_or(false) {
                Self::maybe_request_keyframe(p, track_id, Duration::from_millis(400));
            }
        }
        Ok(Event::Ok)
    }

    fn maybe_request_keyframe(p: &mut Voice, track_id: u8, min_interval: Duration) {
        let now = Instant::now();
        let due = p.last_keyframe_request.get(&track_id).map(|t| now.duration_since(*t) >= min_interval).unwrap_or(true);
        debug!(target: "pyr_media", ev = "kf_wanted", track = track_id, sent = due, every_ms = min_interval.as_millis() as u64);
        if due {
            p.last_keyframe_request.insert(track_id, now);
            let _ = p.tx.send(frame(0, Event::KeyframeRequested { track: track_id }));
        }
    }

    /// A soundboard sound, for everyone in the player's voice channel (them included).
    pub fn play_sound(&self, conn_id: ConnId, sound: String, name: String, emoji: String, volume: f32) -> Reply {
        let inner = self.inner.lock();
        let Some(session) = Self::session_of(&inner, conn_id) else {
            return err(ErrorCode::BadRequest, "Join a voice channel to play sounds.");
        };
        let Some(v) = inner.voice.get(&session) else {
            return err(ErrorCode::BadRequest, "Join a voice channel to play sounds.");
        };
        if v.p.server_mute || v.p.server_deaf {
            return err(ErrorCode::Forbidden, "A moderator muted you here, so your sounds don't play.");
        }
        let channel_id = v.p.channel_id;
        let b = frame(0, Event::SoundPlayed { session, channel_id, sound, name, emoji, volume });
        for v in inner.voice.values().filter(|v| v.p.channel_id == channel_id) {
            let _ = v.tx.send(b.clone());
        }
        Ok(Event::Ok)
    }

    pub fn route_e2ee(&self, conn_id: ConnId, epoch: u16, keys: Vec<WrappedKey>) -> Reply {
        let inner = self.inner.lock();
        let Some(from) = Self::session_of(&inner, conn_id) else {
            return err(ErrorCode::BadRequest, "not in voice");
        };
        let Some(channel) = inner.voice.get(&from).map(|v| v.p.channel_id) else {
            return err(ErrorCode::BadRequest, "not in voice");
        };
        let mut seen = HashSet::new();
        for k in keys {
            // One key per person, of a sensible size.
            if k.to == from || !seen.insert(k.to) || k.nonce.len() > 32 || k.ct.len() > 256 {
                continue;
            }
            if let Some(v) = inner.voice.get(&k.to) {
                if v.p.channel_id == channel {
                    let _ = v.tx.send(frame(0, Event::E2eeKey { from, epoch, nonce: k.nonce, ct: k.ct }));
                }
            }
        }
        Ok(Event::Ok)
    }

    /// Who should receive an audio datagram sent by `session` on `track_id`.
    pub fn audio_targets(&self, session: SessionId, track_id: u8, out: &mut Vec<quinn::Connection>) {
        out.clear();
        let inner = self.inner.lock();
        let Some(v) = inner.voice.get(&session) else { return };
        // Muted, by themselves or a moderator (a moderator's deafen doesn't mute). Muting
        // yourself leaves your stream's sound on, like Discord; a server mute silences both.
        // A server deafen mutes the microphone too (like deafening yourself): someone who
        // can't hear the call isn't heard in it either.
        if ((v.p.self_mute || v.p.server_deaf) && track_id == track::MIC) || v.p.server_mute {
            return;
        }
        let deaf = |o: &Voice| o.p.self_deaf || o.p.server_deaf;
        match track_id {
            track::MIC => {
                for o in inner.voice.values() {
                    if o.p.channel_id == v.p.channel_id && o.p.session != session && !deaf(o) {
                        out.push(o.conn.clone());
                    }
                }
            }
            // Stream audio only goes to people watching the stream (like Discord).
            track::SCREEN_AUDIO => {
                if let Some(subs) = v.subs.get(&track::SCREEN) {
                    for (s, sub) in subs {
                        if inner.voice.get(s).map(|o| !deaf(o)).unwrap_or(false) {
                            out.push(sub.conn.clone());
                        }
                    }
                }
            }
            _ => {}
        }
    }

    pub fn forward_video(&self, session: SessionId, track_id: u8, keyframe: bool, data: Bytes) {
        let mut inner = self.inner.lock();
        let Some(v) = inner.voice.get_mut(&session) else { return };
        if !v.p.tracks.iter().any(|t| t.id == track_id) {
            return;
        }
        let mut need_kf = false;
        if let Some(subs) = v.subs.get(&track_id) {
            for (viewer, sub) in subs {
                match sub.state.offer(keyframe, data.len(), sub.conn.rtt()) {
                    Offer::Send(idx) => {
                        if sub.tx.try_send((idx, data.clone())).is_err() {
                            sub.state.ack(idx, None);
                            sub.state.behind.store(true, Ordering::Relaxed);
                            sub.state.need_keyframe.store(true, Ordering::Relaxed);
                            sub.state.shed.fetch_add(1, Ordering::Relaxed);
                            debug!(target: "pyr_media", ev = "shed", viewer, why = "queue_full");
                            need_kf = true;
                        }
                    }
                    // (Someone who's only just started watching waits for their first keyframe
                    // without it counting against their sharer's connection: it used to halve
                    // the share's bitrate every time someone tuned in.)
                    Offer::Skip { shed } => {
                        if shed {
                            sub.state.shed.fetch_add(1, Ordering::Relaxed);
                        }
                        debug!(target: "pyr_media", ev = "shed", viewer, why = if shed { "behind" } else { "first_key" });
                        need_kf = true
                    }
                }
            }
        }
        if need_kf {
            Self::maybe_request_keyframe(v, track_id, Duration::from_millis(1500));
        }
    }

    /// Twice a second, forever: tells everyone sharing a camera or screen (whose app knows
    /// how, `VIDEO_FEEDBACK_CAP`) how each of their viewers' connections is getting on with it,
    /// so they send what their slowest viewer can take instead of what makes them fall
    /// behind. (`PYR_AB=feedback`: not told.)
    pub async fn feedback_loop(self: Arc<Self>) {
        if pyr_proto::ab::old("feedback") {
            return;
        }
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            let now = Instant::now();
            let inner = self.inner.lock();
            for v in inner.voice.values().filter(|v| v.feedback) {
                for (track, subs) in &v.subs {
                    if subs.is_empty() {
                        continue;
                    }
                    let viewers: Vec<ViewerFeedback> = subs.iter().map(|(s, sub)| sub.state.report(*s, &sub.conn, now)).collect();
                    let _ = v.tx.send(frame(0, Event::VideoFeedback { track: *track, viewers }));
                }
            }
        }
    }

    // ------------------------------------------------------------------- text

    pub fn typing(&self, conn_id: ConnId, uid: &str, channel_id: ChannelId) -> Reply {
        {
            let mut inner = self.inner.lock();
            let Some(c) = inner.clients.get_mut(&conn_id) else { return Ok(Event::Ok) };
            let now = Instant::now();
            if let Some(t) = c.last_typing.get(&channel_id) {
                if now.duration_since(*t) < Duration::from_millis(1500) {
                    return Ok(Event::Ok);
                }
            }
            c.last_typing.insert(channel_id, now);
        }
        self.broadcast_in_except(channel_id, Some(conn_id), Event::Typing { channel_id, user_id: uid.to_string() });
        Ok(Event::Ok)
    }

    /// Remove a member from the server: tell their open connections and close them.
    pub fn eject(&self, uid: &str, reason: &str) {
        let mut inner = self.inner.lock();
        let conns: Vec<(ConnId, quinn::Connection, Outbox)> = inner
            .clients
            .iter()
            .filter(|(_, c)| c.user_id == uid)
            .map(|(id, c)| (*id, c.conn.clone(), c.tx.clone()))
            .collect();
        for (id, conn, tx) in conns {
            if let Some(s) = inner.clients.get(&id).and_then(|c| c.session) {
                Self::remove_session_locked(&mut inner, s);
            }
            let _ = tx.send(frame(0, Event::Kicked { reason: reason.to_string() }));
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(400)).await;
                conn.close(VarInt::from_u32(4), b"removed");
            });
        }
        Self::broadcast_locked(&inner, Event::MemberRemoved { user_id: uid.to_string() });
    }

    pub fn member(&self, uid: &str) -> Option<Member> {
        let inner = self.inner.lock();
        self.member_locked(&inner, uid)
    }

    /// Take a bug report (sent instead of logging in), if the owner collects them and the
    /// sender isn't flooding: at most 5 an hour from one address, 60 in all, and 2,000
    /// waiting to be read.
    pub fn take_bug_report(&self, ip: IpAddr, mut r: BugReport) -> Result<(), String> {
        // (Counted per place: one IPv6 /64 is one sender, whichever of its addresses.)
        let place = Place::of(ip, 64);
        if self.db.get_meta("bug_reports").as_deref() != Some("1") {
            return Err("This server doesn't collect bug reports.".into());
        }
        let hour = Duration::from_secs(3600);
        {
            let mut g = self.bug_reports.lock();
            let (per_ip, all) = &mut *g;
            all.retain(|t| t.elapsed() < hour);
            per_ip.retain(|_, v| {
                v.retain(|t| t.elapsed() < hour);
                !v.is_empty()
            });
            let mine = per_ip.entry(place).or_default();
            if mine.len() >= 5 || all.len() >= 60 {
                return Err("Lots of reports just now. Please try again in a while.".into());
            }
            mine.push(Instant::now());
            all.push(Instant::now());
        }
        if self.db.new_bug_count() >= 2000 {
            return Err("The bug inbox is full right now.".into());
        }
        let clip = |s: &mut String, max: usize| {
            let s2: String = s.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect();
            *s = s2;
            while s.len() > max {
                s.pop();
            }
        };
        clip(&mut r.title, BUG_TITLE_MAX);
        clip(&mut r.body, BUG_BODY_MAX);
        clip(&mut r.log, BUG_LOG_MAX);
        clip(&mut r.app_version, 40);
        clip(&mut r.os, 80);
        if r.title.trim().is_empty() && r.body.trim().is_empty() {
            return Err("The report is empty.".into());
        }
        let id = self.db.add_bug(&r).map_err(|e| format!("{e:#}"))?;
        info!(id, "bug report received");
        // Tell the owner's open apps there's something new (it shows as a count).
        self.broadcast_info();
        Ok(())
    }

    /// Whether a plugin or skin from `ip` would be taken now: checked before the rest of a
    /// skin's files are read, so nobody's files are taken in for nothing.
    pub fn may_take_submission(&self, ip: IpAddr) -> Result<(), String> {
        if self.db.get_meta("plugin_submissions").as_deref() != Some("1") {
            return Err("This server doesn't collect plugins or skins.".into());
        }
        let place = Place::of(ip, 64);
        let hour = Duration::from_secs(3600);
        let g = self.plugin_submissions.lock();
        let (per_ip, all) = &*g;
        let mine = per_ip.get(&place).map(|v| v.iter().filter(|t| t.elapsed() < hour).count()).unwrap_or(0);
        if mine >= SUBMISSIONS_PER_PLACE || all.iter().filter(|t| t.elapsed() < hour).count() >= SUBMISSIONS_IN_ALL {
            return Err("Lots of plugins and skins sent just now. Please try again in a while.".into());
        }
        Ok(())
    }

    /// Take a plugin or skin sent in for the library (instead of logging in), if the owner
    /// collects them and the sender isn't flooding: at most 6 an hour from one place (and from
    /// one sender), 60 in all, and 300 waiting. A plugin's files must be a Rust project's (a
    /// Cargo.toml at the top) with plain relative paths, 600 KB in all; a skin's pass
    /// [`pyr_proto::skin::check`]. `signed`: its signature was checked (by the connection);
    /// only then is the sender kept, and only then can it be a new version of one of theirs.
    pub fn take_plugin_submission(&self, ip: IpAddr, mut sub: PluginSubmission, signed: bool) -> Result<i64, String> {
        let place = Place::of(ip, 64);
        if self.db.get_meta("plugin_submissions").as_deref() != Some("1") {
            return Err("This server doesn't collect plugins or skins.".into());
        }
        if !signed {
            sub.author_id.clear();
            if sub.update_of != 0 {
                return Err("Only a signed plugin or skin can be a new version of one (the Claude connector signs them).".into());
            }
        }
        sub.sig.clear();
        let hour = Duration::from_secs(3600);
        {
            let mut g = self.plugin_submissions.lock();
            let (per_ip, all) = &mut *g;
            all.retain(|t| t.elapsed() < hour);
            per_ip.retain(|_, v| {
                v.retain(|t| t.elapsed() < hour);
                !v.is_empty()
            });
            let mine = per_ip.entry(place).or_default();
            if mine.len() >= SUBMISSIONS_PER_PLACE || all.len() >= SUBMISSIONS_IN_ALL {
                return Err("Lots of plugins and skins sent just now. Please try again in a while.".into());
            }
            if !sub.author_id.is_empty() {
                let mut by = self.submission_senders.lock();
                by.retain(|_, v| {
                    v.retain(|t| t.elapsed() < hour);
                    !v.is_empty()
                });
                let theirs = by.entry(sub.author_id.clone()).or_default();
                if theirs.len() >= SUBMISSIONS_PER_PLACE {
                    return Err("You've sent a lot just now. Please try again in a while.".into());
                }
                theirs.push(Instant::now());
            }
            mine.push(Instant::now());
            all.push(Instant::now());
        }
        if self.db.new_submission_count(false) + self.db.new_submission_count(true) >= 300 {
            return Err("The inbox for plugins and skins is full right now.".into());
        }
        let clip = |s: &mut String, max: usize| {
            let s2: String = s.chars().filter(|c| !c.is_control()).collect();
            *s = s2.trim().to_string();
            while s.len() > max {
                s.pop();
            }
        };
        clip(&mut sub.name, 60);
        clip(&mut sub.kind, 10);
        clip(&mut sub.version, 30);
        clip(&mut sub.summary, 200);
        clip(&mut sub.author, 80);
        clip(&mut sub.contact, 120);
        clip(&mut sub.app_version, 40);
        if sub.name.is_empty() {
            return Err("It needs a name.".into());
        }
        let skin = sub.kind == SKIN_KIND;
        if !skin && sub.kind != "program" && sub.kind != "addon" {
            return Err("A plugin is a program or an add-on (or it's a skin).".into());
        }
        let (max_files, max_size) = if skin { (pyr_proto::skin::MAX_FILES, pyr_proto::skin::MAX_TOTAL) } else { (SUBMISSION_FILES_MAX, SUBMISSION_MAX) };
        if sub.files.is_empty() || sub.files.len() > max_files {
            return Err(format!("It has between 1 and {max_files} files."));
        }
        let mut size = 0usize;
        let mut paths = std::collections::HashSet::new();
        let mut decoded = Vec::with_capacity(sub.files.len());
        for f in &sub.files {
            if !plain_relative_path(&f.path) {
                return Err(format!("{:?} isn't a plain path inside its folder.", f.path.chars().take(80).collect::<String>()));
            }
            if !paths.insert(f.path.as_str()) {
                return Err(format!("{} is in it twice.", f.path));
            }
            let bytes = b64::decode(&f.data).map_err(|_| format!("{} isn't readable.", f.path))?;
            size += bytes.len();
            if size > max_size {
                return Err(format!("It's bigger than {} KB.", max_size / 1000));
            }
            decoded.push((f.path.clone(), bytes));
        }
        if skin {
            let report = pyr_proto::skin::check(&decoded);
            if !report.ok() {
                let mut why = report.problems.join(" ");
                while why.len() > 900 {
                    why.pop();
                }
                return Err(format!("The skin isn't right yet: {why}"));
            }
        } else if !paths.contains("Cargo.toml") {
            return Err("Plugins are Rust projects: a Cargo.toml goes at the top.".into());
        }
        drop(decoded);
        // The names it's sent in under: whoever has one, has it (see `author_names`).
        let mut names = vec![sub.author.clone()];
        if skin {
            names.push(crate::db::skin_author(&sub.files));
        }
        // A new version: of one of the sender's own, of the same kind (said before whose name
        // it's under, which is the closer reason when someone sends a version of another's).
        let mut changes = String::new();
        if sub.update_of != 0 {
            let Some((before, before_files)) = self.db.submission(sub.update_of) else {
                return Err(format!("There's no #{} here to send a new version of.", sub.update_of));
            };
            if before.author_id.is_empty() || before.author_id != sub.author_id {
                return Err(format!("#{} was sent by someone else: only whoever sent it can send a new version.", sub.update_of));
            }
            if (before.kind == SKIN_KIND) != skin {
                return Err(format!("#{} is a {}: a new version of it is one too.", sub.update_of, if skin { "plugin" } else { "skin" }));
            }
            let before_files: Vec<SourceFile> = serde_json::from_str(&before_files).unwrap_or_default();
            changes = describe_changes(&manifest_of(&before_files), &manifest_of(&sub.files));
        }
        let claims = self.author_names(&names, &sub.author_id)?;
        let files = serde_json::to_string(&sub.files).map_err(|e| e.to_string())?;
        self.claim_author_names(claims, &sub.author_id)?;
        let id = self.db.add_submission(&sub, &files, size as u64, &changes).map_err(|e| format!("{e:#}"))?;
        info!(id, name = %sub.name, kind = %sub.kind, update_of = sub.update_of, signed, "sent in for the library");
        self.broadcast_info();
        Ok(id)
    }

    /// Author names in the Pyr Library (Blue: "once an Author name is taken on the
    /// plugin/skin/addon exchange, don't allow anyone else to take it"): the first signed sender
    /// to send a plugin or skin in under a name has it, and every name that looks the same
    /// (`author_key`); nobody else can send one in under it, signed or not. Names with
    /// "Firelit" in them, and Pyr's own, are the owner's (Firelit Games'). A signed sender gets
    /// the names they use that nobody has yet (the ones returned, `claim_author_names` once
    /// the rest is right); one that isn't signed gets none.
    fn author_names(&self, names: &[String], author_id: &str) -> Result<Vec<(String, String)>, String> {
        let owner = !author_id.is_empty() && self.db.owner_ids().iter().any(|o| o == author_id);
        let firelit = author_key(OWNER_NAME_PART);
        let mut claim: Vec<(String, String)> = Vec::new();
        for name in names {
            let key = author_key(name);
            if key.is_empty() || claim.iter().any(|(k, _)| *k == key) {
                continue;
            }
            let ours = key.contains(&firelit) || OWNER_NAMES.iter().any(|n| author_key(n) == key);
            if ours && !owner {
                return Err(format!("The author name \u{201c}{}\u{201d} is Firelit Games' own. Use a name of your own.", name.trim()));
            }
            match self.db.author_name(&key) {
                Some((_, who)) if who != author_id => return Err(name_taken(name)),
                Some(_) => {}
                None if !author_id.is_empty() => claim.push((key, name.trim().to_string())),
                None => {}
            }
        }
        Ok(claim)
    }

    /// The names `author_names` found for a signed sender, theirs now (unless someone got one
    /// in between).
    fn claim_author_names(&self, claims: Vec<(String, String)>, author_id: &str) -> Result<(), String> {
        for (key, name) in claims {
            if !self.db.claim_author_name(&key, &name, author_id).map_err(|e| format!("{e:#}"))? {
                return Err(name_taken(&name));
            }
        }
        Ok(())
    }

    /// What one sender (proven by the connection) sent in here, at most 30 asks an hour from
    /// one place.
    pub fn my_submissions(&self, ip: IpAddr, author_id: &str) -> Result<Vec<SubmissionEntry>, String> {
        let place = Place::of(ip, 64);
        let hour = Duration::from_secs(3600);
        {
            let mut g = self.submission_asks.lock();
            g.retain(|_, v| {
                v.retain(|t| t.elapsed() < hour);
                !v.is_empty()
            });
            let mine = g.entry(place).or_default();
            if mine.len() >= 30 {
                return Err("Lots of asks just now. Please try again in a while.".into());
            }
            mine.push(Instant::now());
        }
        Ok(self.db.submissions_by(author_id, 50))
    }

    pub fn file_path(&self, id: &str) -> Option<std::path::PathBuf> {
        // Ids are hex we made ourselves; anything else never touches the disk.
        (id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())).then(|| self.files_dir.join(id))
    }

    /// Overwrite files with zeros before removing them, so deleted images don't linger on
    /// the disk (as far as the disk allows: SSDs and backups may still keep old blocks).
    pub fn wipe_files(&self, ids: &[String]) {
        for id in ids {
            if let Some(path) = self.file_path(id) {
                wipe_file(&path);
            }
        }
    }

    /// Voice sessions currently in a channel (used when deleting it).
    pub fn evict_channel(&self, channel_id: ChannelId) {
        let mut inner = self.inner.lock();
        let sessions: Vec<SessionId> =
            inner.voice.values().filter(|v| v.p.channel_id == channel_id).map(|v| v.p.session).collect();
        for s in sessions {
            Self::remove_session_locked(&mut inner, s);
        }
    }
}

pub fn wipe_file(path: &std::path::Path) {
    use std::io::{Seek, Write};
    if let Ok(meta) = std::fs::metadata(path) {
        if let Ok(mut f) = std::fs::OpenOptions::new().write(true).open(path) {
            let zeros = vec![0u8; 64 * 1024];
            let mut left = meta.len();
            let _ = f.seek(std::io::SeekFrom::Start(0));
            while left > 0 {
                let n = left.min(zeros.len() as u64) as usize;
                if f.write_all(&zeros[..n]).is_err() {
                    break;
                }
                left -= n as u64;
            }
            let _ = f.sync_all();
        }
        let _ = std::fs::remove_file(path);
    }
}

/// A path inside a plugin's folder as sent in: parts separated by `/`, none empty, `.` or
/// `..`, no backslashes, colons or control characters, and not too long.
/// Plugins and skins taken in an hour: from one place (and one signed sender), and in all.
const SUBMISSIONS_PER_PLACE: usize = 6;
const SUBMISSIONS_IN_ALL: usize = 60;

/// Why a plugin or skin sent in under someone else's author name isn't taken.
fn name_taken(name: &str) -> String {
    format!("The author name \u{201c}{}\u{201d} is taken: someone else sends plugins and skins in under it. Use a name of your own.", name.trim())
}

/// Author names that are the library owner's (Firelit Games') alone: Pyr's own, and any
/// with "Firelit" in them.
const OWNER_NAMES: &[&str] = &["Pyr", "Pyr Library", "Pyr Team", "Ingle", "Ingle Library", "Ingle Team"];
const OWNER_NAME_PART: &str = "Firelit";

/// How a new version's files differ from the one before: "Changed: a, b. Added: c. Taken out: d."
pub fn describe_changes(before: &[FileDigest], after: &[FileDigest]) -> String {
    let old: HashMap<&str, &str> = before.iter().map(|f| (f.path.as_str(), f.sha256.as_str())).collect();
    let new: HashMap<&str, &str> = after.iter().map(|f| (f.path.as_str(), f.sha256.as_str())).collect();
    let mut changed: Vec<&str> = after.iter().filter(|f| old.get(f.path.as_str()).is_some_and(|h| *h != f.sha256)).map(|f| f.path.as_str()).collect();
    let mut added: Vec<&str> = after.iter().filter(|f| !old.contains_key(f.path.as_str())).map(|f| f.path.as_str()).collect();
    let mut gone: Vec<&str> = before.iter().filter(|f| !new.contains_key(f.path.as_str())).map(|f| f.path.as_str()).collect();
    for v in [&mut changed, &mut added, &mut gone] {
        v.sort_unstable();
    }
    let mut parts = Vec::new();
    for (label, list) in [("Changed", &changed), ("Added", &added), ("Taken out", &gone)] {
        if !list.is_empty() {
            parts.push(format!("{label}: {}.", list.join(", ")));
        }
    }
    let mut out = if parts.is_empty() { "No files changed.".to_string() } else { parts.join(" ") };
    if out.len() > 300 {
        let mut cut = 297;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
        out.push_str("...");
    }
    out
}

pub fn plain_relative_path(p: &str) -> bool {
    p.len() <= 200
        && !p.starts_with('/')
        && !p.contains(['\\', ':'])
        && !p.chars().any(|c| c.is_control())
        && p.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A viewer whose oldest frame is long overdue is stuck: frames are skipped for them until
    /// a keyframe. With the media log on (which reads how late they are as it says so), this
    /// once locked the same thing twice and froze the whole server.
    #[test]
    fn a_stuck_viewer_is_skipped_with_the_media_log_on() {
        let st = SubState::new(Duration::from_millis(200));
        st.need_keyframe.store(false, Ordering::Relaxed);
        st.unacked.lock().insert(1, (Instant::now() - Duration::from_secs(3), 30_000));
        let subscriber = tracing_subscriber::fmt().with_max_level(tracing::Level::DEBUG).with_test_writer().finish();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let offered = tracing::subscriber::with_default(subscriber, || st.offer(false, 10_000, Duration::from_millis(200)));
            let _ = tx.send(matches!(offered, Offer::Skip { shed: true }));
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(true), "skipped (and didn't hang)");
    }

    /// A viewer keeping up gets every frame, and its report says how it's doing.
    #[test]
    fn a_viewer_keeping_up_gets_frames() {
        let st = SubState::new(Duration::from_millis(200));
        assert!(matches!(st.offer(false, 10_000, Duration::from_millis(50)), Offer::Skip { shed: false }), "waits for a keyframe first (not shed)");
        let Offer::Send(i) = st.offer(true, 40_000, Duration::from_millis(50)) else { panic!("the keyframe goes") };
        st.ack(i, Some(40_000));
        let Offer::Send(_) = st.offer(false, 10_000, Duration::from_millis(50)) else { panic!("then the rest") };
        assert_eq!(st.late(Duration::from_millis(50)), Duration::ZERO);
        assert_eq!(st.delivered.load(Ordering::Relaxed), 40_000);
    }

    /// Frames skipped for someone who fell behind count as shed (their sharer sends less) until
    /// the keyframe that catches them up; the ones before a new viewer's first keyframe don't
    /// (counting those halved a share's bitrate whenever someone started watching).
    #[test]
    fn only_falling_behind_is_shed() {
        let rtt = Duration::from_millis(50);
        let st = SubState::new(rtt);
        for _ in 0..3 {
            assert!(matches!(st.offer(false, 10_000, rtt), Offer::Skip { shed: false }), "a new viewer waiting for a keyframe");
        }
        let Offer::Send(i) = st.offer(true, 40_000, rtt) else { panic!("the keyframe goes") };
        st.ack(i, Some(40_000));
        // Falls behind: a frame on its way for 3 s.
        st.unacked.lock().insert(99, (Instant::now() - Duration::from_secs(3), 30_000));
        assert!(matches!(st.offer(false, 10_000, rtt), Offer::Skip { shed: true }), "stuck");
        st.unacked.lock().clear();
        assert!(matches!(st.offer(false, 10_000, rtt), Offer::Skip { shed: true }), "still waiting to catch up");
        let Offer::Send(_) = st.offer(true, 40_000, rtt) else { panic!("the keyframe catches them up") };
        let Offer::Send(_) = st.offer(false, 10_000, rtt) else { panic!("then the rest") };
    }
}
