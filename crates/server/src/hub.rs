//! In-memory state of a running server: connected clients, voice sessions and media routing.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use ingle_proto::control::*;
use ingle_proto::invite::InviteLink;
use ingle_proto::media::track;
use ingle_proto::{b64, crypto, framing, now_ms};
use parking_lot::Mutex;
use quinn::VarInt;
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::db::{self, Db};

pub type ConnId = u64;
pub type Reply = Result<Event, (ErrorCode, String)>;

fn err<T>(code: ErrorCode, msg: impl Into<String>) -> Result<T, (ErrorCode, String)> {
    Err((code, msg.into()))
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
}

#[derive(Default)]
struct Inner {
    clients: HashMap<ConnId, Client>,
    voice: HashMap<SessionId, Voice>,
}

struct Client {
    user_id: UserId,
    conn: quinn::Connection,
    tx: mpsc::UnboundedSender<Bytes>,
    session: Option<SessionId>,
    session_cell: Arc<AtomicU32>,
    last_typing: HashMap<ChannelId, Instant>,
}

struct Voice {
    p: VoiceParticipant,
    conn_id: ConnId,
    conn: quinn::Connection,
    tx: mpsc::UnboundedSender<Bytes>,
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
        })
    }

    pub fn alloc_conn_id(&self) -> ConnId {
        self.next_conn.fetch_add(1, Ordering::Relaxed)
    }

    pub fn info(&self) -> ServerInfo {
        ServerInfo {
            name: self.db.get_meta("name").unwrap_or_else(|| "Ingle Server".into()),
            description: self.db.get_meta("description").unwrap_or_default(),
            fingerprint: b64::encode(&self.fingerprint),
        }
    }

    pub fn invite_link(&self, code: &str) -> String {
        InviteLink {
            host: self.public_host.clone(),
            port: self.port,
            code: Some(code.to_string()),
            fingerprint: self.fingerprint,
        }
        .to_url()
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

    fn broadcast_except(&self, except: ConnId, ev: Event) {
        let b = frame(0, ev);
        for (id, c) in self.inner.lock().clients.iter() {
            if *id != except {
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

    /// Decide whether `uid` may log in, redeeming an invite for newcomers.
    pub fn authorize(&self, uid: &str, name: &str, invite: Option<&str>) -> Result<(), (ErrorCode, String)> {
        match self.db.get_user(uid) {
            Some(u) if u.banned => err(ErrorCode::Banned, "You are banned from this server."),
            Some(u) => {
                let _ = self.db.touch_user(uid, name);
                // An existing member presenting an owner/admin invite gets upgraded.
                if let Some(code) = invite {
                    if let Ok(role) = self.db.redeem_invite(code) {
                        if role > u.role {
                            let _ = self.db.set_role(uid, role);
                        }
                    }
                }
                Ok(())
            }
            None => {
                let Some(code) = invite else {
                    return err(ErrorCode::InviteInvalid, "You need an invite link to join this server.");
                };
                let role = match self.db.redeem_invite(code) {
                    Ok(r) => r,
                    Err(db::InviteError::NotFound) => return err(ErrorCode::InviteInvalid, "That invite doesn't exist."),
                    Err(db::InviteError::Expired) => return err(ErrorCode::InviteInvalid, "That invite has expired."),
                    Err(db::InviteError::UsedUp) => {
                        return err(ErrorCode::InviteInvalid, "That invite has already been used.");
                    }
                };
                self.db.insert_user(uid, name, role).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
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
        tx: mpsc::UnboundedSender<Bytes>,
        session_cell: Arc<AtomicU32>,
        re: u64,
    ) {
        let mut inner = self.inner.lock();
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
        let ready = Event::Ready {
            me: me.clone(),
            server: self.info(),
            channels: self.db.channels(),
            members,
            voice: inner.voice.values().map(|v| v.p.clone()).collect(),
            roles: self.db.roles(),
            features: [ROLES_FEATURE, FILES_FEATURE, AVATARS_FEATURE].iter().map(|s| s.to_string()).collect(),
        };
        let _ = tx.send(frame(re, ready));
        if !was_online {
            let b = frame(0, Event::MemberUpserted { member: me });
            for (id, c) in inner.clients.iter() {
                if *id != conn_id {
                    let _ = c.tx.send(b.clone());
                }
            }
        }
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

    fn remove_session_locked(inner: &mut Inner, session: SessionId) {
        let Some(v) = inner.voice.remove(&session) else { return };
        for other in inner.voice.values_mut() {
            for subs in other.subs.values_mut() {
                subs.remove(&session);
            }
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
        let (Some(pk), Ok(sig)) = (b64::decode_array::<32>(&e2ee_pub), b64::decode(&e2ee_sig)) else {
            return err(ErrorCode::BadRequest, "bad E2EE key");
        };
        if !crypto::verify_binding(&uid, channel_id, &pk, &sig) {
            return err(ErrorCode::BadRequest, "E2EE key signature is invalid");
        }
        // One voice session per person: leave any previous one (this or another device).
        let stale: Vec<SessionId> = inner.voice.values().filter(|v| v.p.user_id == uid).map(|v| v.p.session).collect();
        for s in stale {
            Self::remove_session_locked(&mut inner, s);
        }
        let session = self.next_session.fetch_add(1, Ordering::Relaxed);
        let client = inner.clients.get_mut(&conn_id).expect("checked above");
        client.session = Some(session);
        client.session_cell.store(session, Ordering::Relaxed);
        let p = VoiceParticipant {
            session,
            user_id: uid,
            channel_id,
            self_mute,
            self_deaf,
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
        Ok(Event::VoiceReady { session, participants })
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

    pub fn publish(&self, conn_id: ConnId, t: TrackInfo) -> Reply {
        let ok = matches!(
            (t.id, t.kind),
            (track::CAMERA, TrackKind::Camera) | (track::SCREEN, TrackKind::Screen) | (track::SCREEN_AUDIO, TrackKind::ScreenAudio)
        );
        if !ok || t.width > 7680 || t.height > 4320 || t.fps > 240 {
            return err(ErrorCode::BadRequest, "invalid track");
        }
        self.with_my_voice(conn_id, |v| {
            v.p.tracks.retain(|x| x.id != t.id);
            v.p.tracks.push(t);
            v.p.tracks.sort_by_key(|x| x.id);
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
        Ok(Event::Ok)
    }

    pub fn request_keyframe(&self, conn_id: ConnId, session: SessionId, track_id: u8) -> Reply {
        let mut inner = self.inner.lock();
        if Self::session_of(&inner, conn_id).is_none() {
            return err(ErrorCode::BadRequest, "not in voice");
        }
        if let Some(p) = inner.voice.get_mut(&session) {
            Self::maybe_request_keyframe(p, track_id, Duration::from_millis(400));
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

    pub fn route_e2ee(&self, conn_id: ConnId, epoch: u16, keys: Vec<WrappedKey>) -> Reply {
        let inner = self.inner.lock();
        let Some(from) = Self::session_of(&inner, conn_id) else {
            return err(ErrorCode::BadRequest, "not in voice");
        };
        let Some(channel) = inner.voice.get(&from).map(|v| v.p.channel_id) else {
            return err(ErrorCode::BadRequest, "not in voice");
        };
        for k in keys {
            if k.to == from {
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
        if v.p.self_mute && track_id == track::MIC {
            return;
        }
        match track_id {
            track::MIC => {
                for o in inner.voice.values() {
                    if o.p.channel_id == v.p.channel_id && o.p.session != session && !o.p.self_deaf {
                        out.push(o.conn.clone());
                    }
                }
            }
            // Stream audio only goes to people watching the stream (like Discord).
            track::SCREEN_AUDIO => {
                if let Some(subs) = v.subs.get(&track::SCREEN) {
                    for (s, sub) in subs {
                        if inner.voice.get(s).map(|o| !o.p.self_deaf).unwrap_or(false) {
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
        self.broadcast_except(conn_id, Event::Typing { channel_id, user_id: uid.to_string() });
        Ok(Event::Ok)
    }

    /// Remove a member from the server: tell their open connections and close them.
    pub fn eject(&self, uid: &str, reason: &str) {
        let mut inner = self.inner.lock();
        let conns: Vec<(ConnId, quinn::Connection, mpsc::UnboundedSender<Bytes>)> = inner
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
