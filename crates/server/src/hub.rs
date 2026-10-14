//! In-memory state of a running server: connected clients, voice sessions and media routing.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::IpAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use ingle_proto::control::*;
use ingle_proto::invite::InviteLink;
use ingle_proto::media::track;
use ingle_proto::{b64, crypto, framing, now_ms};
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
    std::env::var("INGLE_MAX_CONNECTIONS").ok().and_then(|v| v.parse().ok()).unwrap_or(4000)
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
        if b.len() > ingle_proto::MAX_CONTROL_FRAME + 4 {
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
    /// Uploads per member: bytes being received now, and what arrived in the last hour.
    uploads: Mutex<HashMap<UserId, (u64, Vec<(Instant, u64)>)>>,
}

#[derive(Default)]
struct Inner {
    clients: HashMap<ConnId, Client>,
    voice: HashMap<SessionId, Voice>,
    /// How each person was (muted, deafened) when they last left voice, for a few minutes:
    /// sent back when they rejoin (see `VoiceReady::before`).
    left_voice: HashMap<UserId, (SelfVoiceState, Instant)>,
}

/// How long the server remembers how someone was when they left voice (an app update takes
/// well under this).
const LEFT_VOICE_KEPT: Duration = Duration::from_secs(15 * 60);

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
    conn: quinn::Connection,
    tx: Outbox,
    session: Option<SessionId>,
    session_cell: Arc<AtomicU32>,
    last_typing: HashMap<ChannelId, Instant>,
}

struct Voice {
    p: VoiceParticipant,
    conn_id: ConnId,
    conn: quinn::Connection,
    tx: Outbox,
    /// published track -> subscriber session -> forwarder
    subs: HashMap<u8, HashMap<SessionId, Subscriber>>,
    last_keyframe_request: HashMap<u8, Instant>,
}

struct Subscriber {
    conn: quinn::Connection,
    tx: mpsc::Sender<(u64, Bytes)>,
    state: Arc<SubState>,
}

/// Per (subscriber, published track) congestion state.
pub struct SubState {
    need_keyframe: AtomicBool,
    unacked: Mutex<BTreeMap<u64, Instant>>,
    next_index: AtomicU64,
    max_age: Duration,
}

enum Offer {
    Send(u64),
    SkipNeedKeyframe,
}

impl SubState {
    fn new(rtt: Duration) -> SubState {
        SubState {
            need_keyframe: AtomicBool::new(true),
            unacked: Mutex::new(BTreeMap::new()),
            next_index: AtomicU64::new(1),
            // Allow roughly one round trip plus 350 ms of queued video before shedding.
            max_age: Duration::from_millis(350) + rtt.min(Duration::from_millis(400)),
        }
    }

    fn offer(&self, keyframe: bool) -> Offer {
        let congested = self.unacked.lock().first_key_value().map(|(_, t)| t.elapsed() > self.max_age).unwrap_or(false);
        if congested {
            self.need_keyframe.store(true, Ordering::Relaxed);
            return Offer::SkipNeedKeyframe;
        }
        if self.need_keyframe.load(Ordering::Relaxed) {
            if !keyframe {
                return Offer::SkipNeedKeyframe;
            }
            self.need_keyframe.store(false, Ordering::Relaxed);
        }
        let idx = self.next_index.fetch_add(1, Ordering::Relaxed);
        self.unacked.lock().insert(idx, Instant::now());
        Offer::Send(idx)
    }

    fn ack(&self, idx: u64) {
        self.unacked.lock().remove(&idx);
    }
}

/// Sends frames to one subscriber, one QUIC stream per frame. Frames that are not
/// acknowledged within two seconds are abandoned (stream reset) so they can't pile up.
async fn forward_task(conn: quinn::Connection, mut rx: mpsc::Receiver<(u64, Bytes)>, state: Arc<SubState>) {
    while let Some((idx, frame)) = rx.recv().await {
        let mut stream = match conn.open_uni().await {
            Ok(s) => s,
            Err(_) => break,
        };
        if stream.write_all(&frame).await.is_err() {
            state.ack(idx);
            continue;
        }
        let _ = stream.finish();
        let st = state.clone();
        tokio::spawn(async move {
            let done = tokio::time::timeout(Duration::from_secs(2), stream.stopped()).await;
            if done.is_err() {
                let _ = stream.reset(VarInt::from_u32(0));
                st.need_keyframe.store(true, Ordering::Relaxed);
            }
            st.ack(idx);
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
            uploads: Mutex::new(HashMap::new()),
        })
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
        let total_cap = env_mb("INGLE_STORAGE_MB", 5 * 1024);
        // Each member's share: at most a tenth of it (a few accounts can't fill it), and much
        // less in their first days (new accounts made to fill it get little each).
        let mut user_cap = env_mb("INGLE_USER_STORAGE_MB", 1024).min(total_cap / 10);
        if self.db.get_user(uid).is_some_and(|u| now_ms() - u.joined_at < NEW_MEMBER_MS) {
            user_cap = user_cap.min(env_mb("INGLE_NEW_MEMBER_STORAGE_MB", 100));
        }
        let hourly_cap = env_mb("INGLE_USER_UPLOAD_MB_PER_HOUR", 400);
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
            name: self.db.get_meta("name").unwrap_or_else(|| "Ingle Server".into()),
            description: self.db.get_meta("description").unwrap_or_default(),
            fingerprint: b64::encode(&self.fingerprint),
            icon: self.db.get_meta("icon").filter(|i| !i.is_empty()),
            emojis: self.db.emojis(),
            sounds: self.db.sounds(),
            giphy_key: self.db.get_meta("giphy_key").unwrap_or_default(),
            klipy_key: self.db.get_meta("klipy_key").unwrap_or_default(),
            bug_reports: self.db.get_meta("bug_reports").as_deref() == Some("1"),
            new_bug_reports: self.db.new_bug_count(),
        }
    }

    /// `info` as `user` is sent it: only the owner is told how many bug reports are waiting.
    /// Every member's app gets the GIF search keys: GIPHY and KLIPY want each search to come
    /// straight from the app, never through a server, so a key for them is one every member's
    /// app has (nobody else is sent them).
    fn info_as(mut info: ServerInfo, user: Option<&db::UserRow>) -> ServerInfo {
        if !user.is_some_and(|u| u.role == Role::Owner) {
            info.new_bug_reports = 0;
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

    fn broadcast_locked(inner: &Inner, ev: Event) {
        let b = frame(0, ev);
        for c in inner.clients.values() {
            let _ = c.tx.send(b.clone());
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
        let dm = self.db.dm_members(channel_id);
        let b = frame(0, ev);
        for (id, c) in self.inner.lock().clients.iter() {
            if Some(*id) == except {
                continue;
            }
            if let Some((a, z)) = &dm {
                if c.user_id != *a && c.user_id != *z {
                    continue;
                }
            }
            let _ = c.tx.send(b.clone());
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

    fn is_online_locked(inner: &Inner, uid: &str) -> bool {
        inner.clients.values().any(|c| c.user_id == uid)
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
                // Owner links always work; everyday invites are rate-limited against raids of
                // throwaway accounts.
                if role < Role::Owner && !self.take_join_token() {
                    return err(ErrorCode::RateLimited, "Lots of people are joining right now. Try again in a few minutes.");
                }
                if !self.db.insert_user(uid, name, role).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    return err(ErrorCode::Banned, "You are banned from this server.");
                }
                let _ = self.db.set_joined_with(uid, code);
                info!(%uid, %name, ?role, "new member joined");
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
        inner.clients.insert(
            conn_id,
            Client {
                user_id: uid.to_string(),
                conn,
                tx: tx.clone(),
                session: None,
                session_cell,
                last_typing: HashMap::new(),
            },
        );
        let mut members = self.db.members();
        for m in &mut members {
            m.online = Self::is_online_locked(&inner, &m.user_id);
        }
        let me = members.iter().find(|m| m.user_id == uid).cloned().unwrap_or(Member {
            user_id: uid.to_string(),
            name: String::new(),
            role: Role::Member,
            online: true,
            joined_at: now_ms(),
            roles: Vec::new(),
            avatar: None,
        });
        let mut ready = Event::Ready {
            me: me.clone(),
            server: self.info_for(uid),
            channels: self.db.channels(),
            members,
            voice: inner.voice.values().map(|v| v.p.clone()).collect(),
            roles: self.db.roles(),
            features: [ROLES_FEATURE, FILES_FEATURE, AVATARS_FEATURE, DMS_FEATURE, ICONS_FEATURE, EMOJI_FEATURE, FRIENDS_FEATURE, BACKUP_FEATURE, GIPHY_FEATURE, BUGS_FEATURE, CHANNEL_NEWEST_FEATURE, SOUNDBOARD_FEATURE, VOICE_MOD_FEATURE, KLIPY_FEATURE].iter().map(|s| s.to_string()).collect(),
            dms: self.db.dms_of(uid),
            relations: self.db.relations_of(uid),
        };
        // A very big community could outgrow one frame: then leave out members who've been
        // away longest (their names show up again when they come back).
        let mut b = frame(re, ready.clone());
        while b.len() > ingle_proto::MAX_CONTROL_FRAME {
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
        if !was_online {
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
        if let Some(c) = inner.clients.remove(&conn_id) {
            if !Self::is_online_locked(&inner, &c.user_id) {
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
        let mut changed = Vec::new();
        for other in inner.voice.values_mut() {
            for subs in other.subs.values_mut() {
                subs.remove(&session);
            }
            if Self::sync_viewers(other) {
                changed.push(other.p.clone());
            }
        }
        for p in changed {
            Self::broadcast_locked(inner, Event::VoiceUpserted { participant: p });
        }
        if let Some(c) = inner.clients.get_mut(&v.conn_id) {
            if c.session == Some(session) {
                c.session = None;
                c.session_cell.store(0, Ordering::Relaxed);
            }
        }
        Self::broadcast_locked(
            inner,
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
        match self.db.channel(channel_id) {
            Some(c) if c.kind == ChannelKind::Voice => {}
            _ => return err(ErrorCode::NotFound, "No such voice channel."),
        }
        let mut inner = self.inner.lock();
        let Some(client) = inner.clients.get(&conn_id) else { return err(ErrorCode::Unauthorized, "not logged in") };
        let uid = client.user_id.clone();
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
        // A moderator's mute or deafen follows them from channel to channel, and visit to visit.
        let server_mute = self.moderated(ModeratedVoice::Muted).contains(&uid);
        let server_deaf = self.moderated(ModeratedVoice::Deafened).contains(&uid);
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
            conn_id,
            conn: client.conn.clone(),
            tx: client.tx.clone(),
            subs: HashMap::new(),
            last_keyframe_request: HashMap::new(),
        };
        inner.voice.insert(session, voice);
        Self::broadcast_locked(&inner, Event::VoiceUpserted { participant: p });
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
            let p = v.p.clone();
            Self::broadcast_locked(&inner, Event::VoiceUpserted { participant: p });
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
        for v in inner.voice.values_mut().filter(|v| v.p.user_id == user_id) {
            let flag = match what {
                ModeratedVoice::Muted => &mut v.p.server_mute,
                ModeratedVoice::Deafened => &mut v.p.server_deaf,
            };
            if *flag != on {
                *flag = on;
                now.push(v.p.clone());
            }
        }
        for p in now {
            Self::broadcast_locked(&inner, Event::VoiceUpserted { participant: p });
        }
        Ok(Event::Ok)
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
                let p = publisher.p.clone();
                Self::broadcast_locked(&inner, Event::VoiceUpserted { participant: p });
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
            let p = publisher.p.clone();
            Self::broadcast_locked(&inner, Event::VoiceUpserted { participant: p });
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
            for sub in subs.values() {
                match sub.state.offer(keyframe) {
                    Offer::Send(idx) => {
                        if sub.tx.try_send((idx, data.clone())).is_err() {
                            sub.state.ack(idx);
                            sub.state.need_keyframe.store(true, Ordering::Relaxed);
                            need_kf = true;
                        }
                    }
                    Offer::SkipNeedKeyframe => need_kf = true,
                }
            }
        }
        if need_kf {
            Self::maybe_request_keyframe(v, track_id, Duration::from_millis(1500));
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
