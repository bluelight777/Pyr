//! One client connection: login handshake, control requests, and media ingest.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use bytes::BytesMut;
use ingle_proto::control::*;
use ingle_proto::media::{self, AudioHeader, VideoHeader, track};
use ingle_proto::{MAX_MESSAGE_CHARS, MAX_VIDEO_FRAME, PROTOCOL_VERSION, crypto, framing};
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::db::new_invite_code;
use crate::hub::{ConnId, ConnSlot, Hub, Outbox, Reply, frame};

/// Most video frames being received at once on one connection.
const MAX_VIDEO_READS: usize = 24;
/// Longest message `nonce` a client may send (it's echoed to everyone).
const MAX_NONCE_LEN: usize = 64;
/// Usable invites one member may have made at a time.
const MAX_INVITES_PER_MEMBER: usize = 25;
/// Requests per second (and burst) one connection may make, all kinds together.
const REQ_RATE: f64 = 20.0;
const REQ_BURST: f64 = 80.0;
/// Soundboard: a few sounds in a row, then one every couple of seconds.
const SOUND_RATE: f64 = 0.5;
const SOUND_BURST: f64 = 4.0;
/// History pages are kept well under the frame limit (long messages add up).
const HISTORY_PAGE_BYTES: usize = 700 * 1024;
/// Video one person may send, in bytes per second and burst (Ingle's sharpest screen share
/// plus a camera is about 1.5 MB/s): frames past this are dropped, not passed on to
/// everyone watching.
const VIDEO_BYTES_PER_SEC: f64 = 5.0 * 1024.0 * 1024.0;
const VIDEO_BURST_BYTES: f64 = 24.0 * 1024.0 * 1024.0;

pub async fn handle(hub: Arc<Hub>, incoming: quinn::Incoming, mut slot: ConnSlot) {
    let remote = incoming.remote_address();
    let conn = match incoming.await {
        Ok(c) => c,
        Err(e) => {
            debug!(%remote, "handshake failed: {e}");
            return;
        }
    };
    slot.handshake_done();
    match session(hub, conn.clone()).await {
        Ok(()) => debug!(%remote, "connection closed"),
        Err(e) => debug!(%remote, "connection ended: {e:#}"),
    }
    conn.close(quinn::VarInt::from_u32(0), b"bye");
    drop(slot);
}

async fn read_frame<T: serde::de::DeserializeOwned>(recv: &mut quinn::RecvStream) -> Result<Option<T>> {
    let mut len = [0u8; 4];
    match recv.read_exact(&mut len).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(_)) => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let n = framing::body_len(len)?;
    let mut body = vec![0u8; n];
    recv.read_exact(&mut body).await?;
    Ok(Some(framing::decode(&body)?))
}

async fn session(hub: Arc<Hub>, conn: quinn::Connection) -> Result<()> {
    let (mut send, mut recv) = timeout(Duration::from_secs(10), conn.accept_bi()).await.context("no control stream")??;
    let mut preamble = [0u8; 4];
    timeout(Duration::from_secs(10), recv.read_exact(&mut preamble)).await.context("no preamble")??;
    if &preamble != ingle_proto::CONTROL_PREAMBLE {
        bail!("not an Ingle client");
    }

    let nonce = crypto::random_bytes::<32>();
    let hello = Event::Hello { proto: PROTOCOL_VERSION, server_name: hub.info().name, nonce: nonce.to_vec() };
    send.write_all(&frame(0, hello)).await?;

    let login: ClientFrame =
        timeout(Duration::from_secs(20), read_frame(&mut recv)).await.context("login timeout")??.ok_or_else(|| anyhow!("closed"))?;
    let reject = |code: ErrorCode, msg: &str| frame(login.id, Event::error(code, msg));
    // Not logging in, just a bug report (anyone may send one where they're collected).
    if let Request::ReportBug { report } = login.req {
        let reply = match hub.take_bug_report(conn.remote_address().ip(), report) {
            Ok(()) => frame(login.id, Event::Ok),
            Err(msg) => reject(ErrorCode::RateLimited, &msg),
        };
        send.write_all(&reply).await?;
        let _ = send.finish();
        tokio::time::sleep(Duration::from_millis(300)).await;
        return Ok(());
    }
    let Request::Login { proto, user_id, sig, name, invite, client } = login.req else {
        send.write_all(&reject(ErrorCode::BadRequest, "expected login")).await?;
        bail!("first request was not a login");
    };
    if proto != PROTOCOL_VERSION {
        let msg = format!("This server speaks protocol v{PROTOCOL_VERSION}; your app speaks v{proto}. Please update.");
        send.write_all(&reject(ErrorCode::VersionMismatch, &msg)).await?;
        let _ = send.finish();
        tokio::time::sleep(Duration::from_millis(300)).await;
        bail!("protocol mismatch");
    }
    let mut exporter = [0u8; 32];
    conn.export_keying_material(&mut exporter, crypto::LOGIN_EXPORTER_LABEL, b"")
        .map_err(|_| anyhow!("TLS exporter unavailable"))?;
    // Exactly the way we write ids: the same key spelled with padding or spaces would
    // otherwise count as someone else (a way around a ban) or bloat everything it's sent in.
    let canonical = crypto::is_valid_user_id(&user_id)
        && ingle_proto::b64::decode(&user_id).map(|b| ingle_proto::b64::encode(&b) == user_id).unwrap_or(false);
    if !canonical || !crypto::verify(&user_id, &crypto::login_transcript(&nonce, &exporter, &user_id), &sig) {
        send.write_all(&reject(ErrorCode::Unauthorized, "identity proof failed")).await?;
        bail!("bad login signature");
    }
    let name = clean_member_name(&name).unwrap_or_else(|| "Anonymous".to_string());
    let client: String = client.chars().filter(|c| !c.is_control()).take(48).collect();
    if let Err((code, msg)) = hub.authorize(&user_id, &name, invite.as_deref(), conn.remote_address().ip()) {
        send.write_all(&reject(code, &msg)).await?;
        let _ = send.finish();
        tokio::time::sleep(Duration::from_millis(300)).await;
        bail!("login refused: {msg}");
    }
    info!(user = %&user_id[..8], %name, %client, remote = %conn.remote_address(), "logged in");

    let (tx, mut rx) = Outbox::new(conn.clone());
    let queued = tx.queued();
    // Ends when the connection does (once every `Outbox` for it is gone) or a write fails.
    let writer = tokio::spawn(async move {
        while let Some(b) = rx.recv().await {
            if send.write_all(&b).await.is_err() {
                break;
            }
            queued.fetch_sub(b.len(), Ordering::Relaxed);
        }
        let _ = send.finish();
    });

    let conn_id = hub.alloc_conn_id();
    let session_cell = Arc::new(AtomicU32::new(0));
    if let Err((code, msg)) = hub.register(conn_id, &user_id, conn.clone(), tx.clone(), session_cell.clone(), login.id) {
        tx.send(frame(login.id, Event::error(code, msg.clone())));
        drop(tx);
        let _ = timeout(Duration::from_secs(1), writer).await;
        bail!("not registered: {msg}");
    }

    let dg = tokio::spawn(datagram_loop(hub.clone(), conn.clone(), session_cell.clone()));
    let uni = tokio::spawn(uni_loop(hub.clone(), conn.clone(), session_cell.clone()));
    let files = tokio::spawn(file_loop(hub.clone(), conn.clone(), user_id.clone()));

    let mut ctx = Ctx {
        hub: &hub,
        conn_id,
        uid: user_id.clone(),
        msg_tokens: Bucket::new(1.5, 10.0),
        react_tokens: Bucket::new(3.0, 15.0),
        req_tokens: Bucket::new(REQ_RATE, REQ_BURST),
        sound_tokens: Bucket::new(SOUND_RATE, SOUND_BURST),
    };
    let result = async {
        loop {
            let Some(f) = read_frame::<ClientFrame>(&mut recv).await? else { return Ok(()) };
            let id = f.id;
            let reply = if ctx.req_tokens.take(cost(&f.req)) {
                ctx.handle(f.req)
            } else {
                Err((ErrorCode::RateLimited, "Slow down a little.".into()))
            };
            let ev = reply.unwrap_or_else(|(code, message)| Event::Error { code, message });
            if id != 0 {
                let mut b = frame(id, ev);
                if b.len() > ingle_proto::MAX_CONTROL_FRAME {
                    b = frame(id, Event::error(ErrorCode::Internal, "That reply was too big to send."));
                }
                tx.send(b);
            }
        }
    }
    .await;

    hub.unregister(conn_id);
    dg.abort();
    uni.abort();
    files.abort();
    drop(tx);
    let _ = timeout(Duration::from_secs(1), writer).await;
    result
}

/// Audio datagrams: rewrite the session id and fan out.
async fn datagram_loop(hub: Arc<Hub>, conn: quinn::Connection, session: Arc<AtomicU32>) {
    let mut targets = Vec::new();
    while let Ok(d) = conn.read_datagram().await {
        let s = session.load(Ordering::Relaxed);
        if s == 0 {
            continue;
        }
        let Some(h) = AudioHeader::parse(&d) else { continue };
        if h.track != track::MIC && h.track != track::SCREEN_AUDIO {
            continue;
        }
        hub.audio_targets(s, h.track, &mut targets);
        if targets.is_empty() {
            continue;
        }
        let mut b = BytesMut::from(&d[..]);
        media::set_session(&mut b, s);
        let b = b.freeze();
        for t in &targets {
            let _ = t.send_datagram(b.clone());
        }
    }
}

/// Video frames: one unidirectional stream per frame. Only from someone in voice, a few at
/// a time, and each read no further than its header says.
async fn uni_loop(hub: Arc<Hub>, conn: quinn::Connection, session: Arc<AtomicU32>) {
    let slots = Arc::new(tokio::sync::Semaphore::new(MAX_VIDEO_READS));
    let budget = Arc::new(parking_lot::Mutex::new(Bucket::new(VIDEO_BYTES_PER_SEC, VIDEO_BURST_BYTES)));
    while let Ok(mut stream) = conn.accept_uni().await {
        if session.load(Ordering::Relaxed) == 0 {
            let _ = stream.stop(quinn::VarInt::from_u32(0));
            continue;
        }
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            // Too many at once: drop this frame (the sender sends a keyframe when asked).
            let _ = stream.stop(quinn::VarInt::from_u32(0));
            continue;
        };
        let hub = hub.clone();
        let session = session.clone();
        let budget = budget.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let read = async {
                let mut head = [0u8; media::VIDEO_HEADER_LEN];
                stream.read_exact(&mut head).await.ok()?;
                let h = VideoHeader::parse(&head)?;
                let len = h.len as usize;
                if len > MAX_VIDEO_FRAME || !budget.lock().take(len as f64) {
                    return None;
                }
                let mut data = BytesMut::with_capacity(media::VIDEO_HEADER_LEN + len);
                data.extend_from_slice(&head);
                data.resize(media::VIDEO_HEADER_LEN + len, 0);
                stream.read_exact(&mut data[media::VIDEO_HEADER_LEN..]).await.ok()?;
                Some((h, data))
            };
            let Ok(Some((h, mut data))) = timeout(Duration::from_secs(5), read).await else {
                let _ = stream.stop(quinn::VarInt::from_u32(0));
                return;
            };
            let s = session.load(Ordering::Relaxed);
            if s == 0 {
                return;
            }
            media::set_session(&mut data, s);
            hub.forward_video(s, h.track, h.is_keyframe(), data.freeze());
        });
    }
}

/// File streams: uploads and downloads of images (see [`FILE_PREAMBLE`]).
async fn file_loop(hub: Arc<Hub>, conn: quinn::Connection, uid: String) {
    // At most a few transfers at once per connection.
    let slots = Arc::new(tokio::sync::Semaphore::new(4));
    while let Ok((send, recv)) = conn.accept_bi().await {
        let (hub, uid, slots) = (hub.clone(), uid.clone(), slots.clone());
        tokio::spawn(async move {
            let Ok(_permit) = slots.acquire_owned().await else { return };
            if let Err(e) = file_stream(&hub, &uid, send, recv).await {
                debug!("file stream ended: {e:#}");
            }
        });
    }
}

async fn file_stream(hub: &Hub, uid: &str, mut send: quinn::SendStream, mut recv: quinn::RecvStream) -> Result<()> {
    let mut preamble = [0u8; 4];
    timeout(Duration::from_secs(10), recv.read_exact(&mut preamble)).await??;
    if &preamble != FILE_PREAMBLE {
        bail!("not a file stream");
    }
    let req: FileRequest = timeout(Duration::from_secs(10), read_frame(&mut recv)).await??.ok_or_else(|| anyhow!("no request"))?;
    let reply = |r: FileReply| framing::encode(&r);
    let fail = |msg: &str| reply(FileReply { ok: false, id: String::new(), mime: String::new(), size: 0, error: msg.to_string() });
    match req {
        FileRequest::Get { id } => {
            let (Some(row), Some(path)) = (hub.db.file(&id), hub.file_path(&id)) else {
                send.write_all(&fail("That file is gone.")).await?;
                let _ = send.finish();
                return Ok(());
            };
            // A picture sent in a direct message is only for its two people. (If its message
            // can't be found, nobody gets it: never fail open.)
            let private = match row.message_id {
                None => row.kind == FileKind::Attachment && row.owner != uid,
                Some(m) => match hub.db.get_message(m) {
                    None => true,
                    Some(msg) => hub.db.dm_members(msg.channel_id).map(|(a, b)| a != uid && b != uid).unwrap_or(false),
                },
            };
            let bytes = if private { Vec::new() } else { tokio::fs::read(&path).await.unwrap_or_default() };
            if bytes.len() as u64 != row.size || private {
                send.write_all(&fail("That file is gone.")).await?;
                let _ = send.finish();
                return Ok(());
            }
            send.write_all(&reply(FileReply { ok: true, id, mime: row.mime, size: row.size, error: String::new() })).await?;
            send.write_all(&bytes).await?;
            let _ = send.finish();
        }
        FileRequest::Backup { item } => {
            backup_item(hub, uid, &item, &mut send).await?;
        }
        FileRequest::Put { kind, name, mime: _, size } => {
            let limit = match kind {
                FileKind::Attachment => MAX_ATTACHMENT_BYTES,
                FileKind::Avatar => MAX_AVATAR_BYTES,
                FileKind::Sound => ingle_proto::sound::MAX_BYTES as u64,
            };
            if size == 0 || size > limit {
                let msg = if kind == FileKind::Sound {
                    format!("Sounds can be up to {} KB.", limit >> 10)
                } else {
                    format!("Images can be up to {} MB.", limit >> 20)
                };
                send.write_all(&fail(&msg)).await?;
                let _ = send.finish();
                return Ok(());
            }
            if kind == FileKind::Sound {
                return sound_put(hub, uid, &name, size, send, recv).await;
            }
            let slot = match hub.reserve_upload(uid, size) {
                Ok(s) => s,
                Err(msg) => {
                    send.write_all(&fail(&msg)).await?;
                    let _ = send.finish();
                    return Ok(());
                }
            };
            let bytes = timeout(Duration::from_secs(300), recv.read_to_end(size as usize + 1)).await??;
            if bytes.len() as u64 != size {
                send.write_all(&fail("The upload was cut short.")).await?;
                let _ = send.finish();
                return Ok(());
            }
            let Some(mime) = sniff_image(&bytes) else {
                send.write_all(&fail("Only PNG, JPEG, GIF and WebP images can be sent.")).await?;
                let _ = send.finish();
                return Ok(());
            };
            let (width, height) = image_size(&bytes).unwrap_or((0, 0));
            // Pictures that claim to be enormous are refused: decoding one would make every
            // app that shows it run out of memory.
            if !image_dimensions_ok(&bytes) {
                send.write_all(&fail("That image is too large (at most 8192 pixels across).")).await?;
                let _ = send.finish();
                return Ok(());
            }
            let id = match kind {
                // Profile pictures are named by their content, so an app can tell whether the
                // server already has its current one.
                FileKind::Avatar => avatar_id(uid, &bytes),
                // (Sounds are handled by `sound_put` before this.)
                FileKind::Attachment | FileKind::Sound => crypto::random_bytes::<16>().iter().map(|b| format!("{b:02x}")).collect(),
            };
            let path = hub.file_path(&id).ok_or_else(|| anyhow!("bad id"))?;
            let tmp = path.with_extension("part");
            tokio::fs::write(&tmp, &bytes).await?;
            tokio::fs::rename(&tmp, &path).await?;
            let name: String = name.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\')).take(100).collect();
            hub.db.insert_file(&crate::db::FileRow {
                id: id.clone(),
                owner: uid.to_string(),
                kind,
                name: if name.trim().is_empty() { "image".into() } else { name },
                mime: mime.to_string(),
                size,
                width,
                height,
                message_id: None,
            })?;
            slot.stored();
            send.write_all(&reply(FileReply { ok: true, id, mime: mime.to_string(), size, error: String::new() })).await?;
            let _ = send.finish();
        }
    }
    // Let the reply reach the other side before the stream is dropped.
    let _ = timeout(Duration::from_secs(10), send.stopped()).await;
    Ok(())
}

/// A soundboard sound (`FileKind::Sound`): checked (it has to be a sound file Ingle can play,
/// see `ingle_proto::sound`) and named by its content, so the same sound is kept once however
/// often it's sent.
async fn sound_put(hub: &Hub, uid: &str, name: &str, size: u64, mut send: quinn::SendStream, mut recv: quinn::RecvStream) -> Result<()> {
    let reply = |r: FileReply| framing::encode(&r);
    let fail = |msg: &str| reply(FileReply { ok: false, id: String::new(), mime: String::new(), size: 0, error: msg.to_string() });
    let slot = match hub.reserve_upload(uid, size) {
        Ok(s) => s,
        Err(msg) => {
            send.write_all(&fail(&msg)).await?;
            let _ = send.finish();
            return Ok(());
        }
    };
    let bytes = timeout(Duration::from_secs(120), recv.read_to_end(size as usize + 1)).await??;
    if bytes.len() as u64 != size {
        send.write_all(&fail("The upload was cut short.")).await?;
        let _ = send.finish();
        return Ok(());
    }
    if ingle_proto::sound::check(&bytes).is_none() {
        send.write_all(&fail("That isn't a sound Ingle can play.")).await?;
        let _ = send.finish();
        return Ok(());
    }
    const MIME: &str = "audio/x-ingle-sound";
    let id = ingle_proto::sound::sound_id(&bytes);
    let have = hub.db.file(&id).is_some_and(|f| f.kind == FileKind::Sound) && hub.file_path(&id).is_some_and(|p| p.is_file());
    if !have {
        let path = hub.file_path(&id).ok_or_else(|| anyhow!("bad id"))?;
        let tmp = path.with_extension("part");
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;
        let name: String = name.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\')).take(100).collect();
        hub.db.insert_file(&crate::db::FileRow {
            id: id.clone(),
            owner: uid.to_string(),
            kind: FileKind::Sound,
            name: if name.trim().is_empty() { "sound".into() } else { name },
            mime: MIME.to_string(),
            size,
            width: 0,
            height: 0,
            message_id: None,
        })?;
        slot.stored();
    }
    send.write_all(&reply(FileReply { ok: true, id, mime: MIME.to_string(), size, error: String::new() })).await?;
    let _ = send.finish();
    let _ = timeout(Duration::from_secs(10), send.stopped()).await;
    Ok(())
}

/// A soundboard sound's name: 1–32 characters, no control characters.
fn clean_sound_name(name: &str) -> Option<String> {
    let n: String = name.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string();
    (!n.is_empty() && n.chars().count() <= MAX_SOUND_NAME).then_some(n)
}

/// What a sound shows with it: one emoji, a few characters at most (or nothing).
fn clean_sound_emoji(e: &str) -> Option<String> {
    let e = e.trim();
    (e.len() <= 80 && !e.chars().any(|c| c.is_control() || c.is_whitespace())).then(|| e.to_string())
}

struct Ctx<'a> {
    hub: &'a Hub,
    conn_id: ConnId,
    uid: String,
    /// Messages and edits.
    msg_tokens: Bucket,
    react_tokens: Bucket,
    /// Every request (weighted by [`cost`]).
    req_tokens: Bucket,
    /// Soundboard sounds played.
    sound_tokens: Bucket,
}

/// A token bucket: `rate` per second, holding at most `burst`.
struct Bucket {
    tokens: f64,
    rate: f64,
    burst: f64,
    at: Instant,
}

impl Bucket {
    fn new(rate: f64, burst: f64) -> Bucket {
        Bucket { tokens: burst, rate, burst, at: Instant::now() }
    }

    fn take(&mut self, n: f64) -> bool {
        let now = Instant::now();
        self.tokens = (self.tokens + now.duration_since(self.at).as_secs_f64() * self.rate).min(self.burst);
        self.at = now;
        if self.tokens < n {
            return false;
        }
        self.tokens -= n;
        true
    }
}

/// How much of the request allowance a request uses: database-heavy or things that grow the
/// database cost more.
fn cost(req: &Request) -> f64 {
    match req {
        Request::Ping { .. } | Request::Typing { .. } | Request::VoiceState { .. } => 0.5,
        Request::FetchHistory { .. } => 3.0,
        Request::E2eeKeys { keys, .. } => 1.0 + keys.len() as f64 / 16.0,
        Request::CreateInvite { .. }
        | Request::FriendRequest { .. }
        | Request::OpenDm { .. }
        | Request::AddEmoji { .. }
        | Request::AddSound { .. }
        | Request::CreateChannel { .. }
        | Request::CreateRole { .. }
        | Request::SetAvatar { .. }
        | Request::SetServerIcon { .. }
        | Request::SetGiphyKey { .. }
        | Request::UpdateProfile { .. }
        | Request::UpdateServer { .. }
        | Request::Block { .. } => 4.0,
        _ => 1.0,
    }
}

/// A member id as we write them (anything else can't be anyone).
fn valid_uid(uid: &str) -> Result<(), (ErrorCode, String)> {
    if crypto::is_valid_user_id(uid) { Ok(()) } else { Err((ErrorCode::NotFound, "No such member.".into())) }
}

/// A Unicode emoji reaction: no letters, digits (except in keycaps), spaces or controls, and
/// short.
fn valid_unicode_reaction(e: &str) -> bool {
    let keycap = e.contains('\u{20e3}');
    !e.is_empty()
        && e.len() <= 32
        && e.chars().all(|c| {
            if c.is_ascii() {
                keycap && (c.is_ascii_digit() || c == '#' || c == '*')
            } else {
                !c.is_control() && !c.is_whitespace() && !is_invisible(c) || matches!(c, '\u{200d}' | '\u{fe0f}' | '\u{e0020}'..='\u{e007f}')
            }
        })
}

fn bad<T>(msg: &str) -> Result<T, (ErrorCode, String)> {
    Err((ErrorCode::BadRequest, msg.to_string()))
}

fn clean_content(raw: &str) -> Option<String> {
    let s = raw.trim_matches(|c: char| c.is_whitespace());
    if s.is_empty() || s.chars().count() > MAX_MESSAGE_CHARS {
        return None;
    }
    // Strip control characters except newlines and tabs.
    Some(s.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect())
}

impl Ctx<'_> {
    fn role(&self) -> Role {
        self.hub.db.get_user(&self.uid).map(|u| u.role).unwrap_or(Role::Member)
    }

    /// What this member may do: everything for admins and the owner, otherwise what their
    /// roles allow.
    fn perms(&self) -> u64 {
        if self.role().is_admin() {
            return perm::ALL;
        }
        let mine = self.hub.db.member_roles(&self.uid);
        self.hub.db.roles().iter().filter(|r| mine.contains(&r.id)).fold(0, |acc, r| acc | r.perms)
    }

    /// Someone else who's a member here.
    fn other_member(&self, uid: &str) -> Result<(), (ErrorCode, String)> {
        if uid == self.uid {
            return Err((ErrorCode::BadRequest, "That's you.".into()));
        }
        match self.hub.db.get_user(uid) {
            Some(u) if !u.banned => Ok(()),
            _ => Err((ErrorCode::NotFound, "They aren't a member of this server.".into())),
        }
    }

    /// Tells both people where they now stand with each other.
    fn tell_relation(&self, other: &str) {
        let db = &self.hub.db;
        self.hub.send_to_user(&self.uid, Event::RelationUpdated { user_id: other.to_string(), kind: db.relation(&self.uid, other) });
        self.hub.send_to_user(other, Event::RelationUpdated { user_id: self.uid.clone(), kind: db.relation(other, &self.uid) });
    }

    fn require(&self, p: u64, what: &str) -> Result<(), (ErrorCode, String)> {
        if self.perms() & p == p { Ok(()) } else { Err((ErrorCode::Forbidden, format!("You don't have permission to {what}."))) }
    }

    /// Where someone stands: the owner above admins above every custom role (by its
    /// position); members with no roles at the bottom.
    fn rank_of(&self, uid: &str) -> i64 {
        match self.hub.db.get_user(uid).map(|u| u.role) {
            Some(Role::Owner) => i64::MAX,
            Some(Role::Admin) => i64::MAX - 1,
            _ => {
                let mine = self.hub.db.member_roles(uid);
                self.hub.db.roles().iter().filter(|r| mine.contains(&r.id)).map(|r| r.position as i64).max().unwrap_or(i64::MIN)
            }
        }
    }

    /// Roles at or above your own highest one are out of your reach (unless you're an admin).
    fn can_manage_role(&self, r: &RoleDef) -> bool {
        self.role().is_admin() || (r.position as i64) < self.rank_of(&self.uid)
    }

    /// A text channel, or a direct message I'm in.
    fn can_see(&self, channel_id: ChannelId) -> Result<(), (ErrorCode, String)> {
        let db = &self.hub.db;
        if db.channel(channel_id).is_some() {
            return Ok(());
        }
        match db.dm_members(channel_id) {
            Some((a, b)) if a == self.uid || b == self.uid => Ok(()),
            _ => Err((ErrorCode::NotFound, "No such channel.".into())),
        }
    }

    fn take_message_token(&mut self) -> Result<(), (ErrorCode, String)> {
        if !self.msg_tokens.take(1.0) {
            return Err((ErrorCode::RateLimited, "You're sending messages too fast.".into()));
        }
        Ok(())
    }

    /// In a direct message, refuse if either has blocked the other.
    fn not_blocked_in(&self, channel_id: ChannelId) -> Result<(), (ErrorCode, String)> {
        let db = &self.hub.db;
        if let Some((x, y)) = db.dm_members(channel_id) {
            let other = if x == self.uid { y } else { x };
            if db.blocked_between(&self.uid, &other) {
                return Err((ErrorCode::Forbidden, "You can't message this person.".into()));
            }
        }
        Ok(())
    }

    fn handle(&mut self, req: Request) -> Reply {
        let hub = self.hub;
        let db = &hub.db;
        match req {
            Request::Login { .. } => bad("already logged in"),
            Request::Ping { t } => Ok(Event::Pong { t }),

            Request::SendMessage { channel_id, content, reply_to, nonce, attachments } => {
                self.take_message_token()?;
                if nonce.len() > MAX_NONCE_LEN {
                    return bad("bad message nonce");
                }
                if attachments.len() > MAX_ATTACHMENTS_PER_MESSAGE {
                    return bad("too many attachments");
                }
                for id in &attachments {
                    match db.file(id) {
                        Some(f) if f.owner == self.uid && f.kind == FileKind::Attachment && f.message_id.is_none() => {}
                        _ => return bad("an attachment is missing; try sending it again"),
                    }
                }
                let content = match clean_content(&content) {
                    Some(c) => c,
                    None if !attachments.is_empty() && content.trim().is_empty() => String::new(),
                    None => return bad("message is empty or too long"),
                };
                self.can_see(channel_id)?;
                self.not_blocked_in(channel_id)?;
                let reply_to = reply_to.filter(|id| db.get_message(*id).map(|m| m.channel_id == channel_id).unwrap_or(false));
                let message = db
                    .insert_message_with(channel_id, &self.uid, &content, reply_to, &attachments)
                    .map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast_in(channel_id, Event::MessageCreated { message, nonce });
                Ok(Event::Ok)
            }
            Request::EditMessage { message_id, content } => {
                self.take_message_token()?;
                let Some(content) = clean_content(&content) else { return bad("message is empty or too long") };
                let Some(m) = db.get_message(message_id) else { return Err((ErrorCode::NotFound, "gone".into())) };
                if m.author != self.uid {
                    return Err((ErrorCode::Forbidden, "You can only edit your own messages.".into()));
                }
                self.not_blocked_in(m.channel_id)?;
                if let Ok(Some(message)) = db.edit_message(message_id, &content) {
                    hub.broadcast_in(message.channel_id, Event::MessageUpdated { message });
                }
                Ok(Event::Ok)
            }
            Request::DeleteMessage { message_id } => {
                let Some(m) = db.get_message(message_id) else { return Ok(Event::Ok) };
                // In direct messages only the author can; elsewhere moderators can too.
                let in_dm = db.dm_members(m.channel_id).is_some();
                if m.author != self.uid && (in_dm || self.perms() & perm::MANAGE_MESSAGES == 0) {
                    return Err((ErrorCode::Forbidden, "You can't delete that message.".into()));
                }
                let files = db.delete_message(message_id).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.wipe_files(&files);
                hub.broadcast_in(m.channel_id, Event::MessageDeleted { channel_id: m.channel_id, message_id });
                Ok(Event::Ok)
            }
            Request::React { message_id, emoji, add } => {
                if !self.react_tokens.take(1.0) {
                    return Err((ErrorCode::RateLimited, "You're reacting too fast.".into()));
                }
                let emoji = emoji.trim().to_string();
                // A server emoji (`<:name:id>`, which must exist here; kept as the server
                // writes it) or a real emoji.
                let emoji = if emoji.starts_with('<') && emoji.ends_with('>') && emoji.len() <= 120 {
                    let id = emoji.trim_end_matches('>').rsplit(':').next().unwrap_or("");
                    match db.emojis().into_iter().find(|e| e.id == id) {
                        Some(e) if e.animated => format!("<a:{}:{}>", e.name, e.id),
                        Some(e) => format!("<:{}:{}>", e.name, e.id),
                        None => return bad("not an emoji on this server"),
                    }
                } else if valid_unicode_reaction(&emoji) {
                    emoji
                } else {
                    return bad("not an emoji");
                };
                let Some(m) = db.get_message(message_id) else { return Err((ErrorCode::NotFound, "gone".into())) };
                self.can_see(m.channel_id)?;
                if add {
                    self.not_blocked_in(m.channel_id)?;
                }
                if add && db.distinct_reactions(message_id) >= 20 {
                    return bad("too many different reactions");
                }
                db.set_reaction(message_id, &emoji, &self.uid, add).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if let Some(message) = db.get_message(message_id) {
                    hub.broadcast_in(message.channel_id, Event::MessageUpdated { message });
                }
                Ok(Event::Ok)
            }
            Request::FetchHistory { channel_id, before, limit } => {
                self.can_see(channel_id)?;
                let (mut messages, mut complete) = db.history(channel_id, before, limit);
                // Long messages add up: keep the page well inside one frame, dropping the
                // oldest (the app asks for those next).
                let size = |m: &Message| {
                    m.content.len()
                        + 400
                        + m.attachments.iter().map(|a| a.name.len() + 160).sum::<usize>()
                        + m.reactions.iter().map(|r| r.emoji.len() + 24 + r.users.iter().map(|u| u.len() + 3).sum::<usize>()).sum::<usize>()
                };
                let mut total: usize = messages.iter().map(size).sum();
                while total > HISTORY_PAGE_BYTES && messages.len() > 1 {
                    total -= size(&messages.remove(0));
                    complete = false;
                }
                Ok(Event::History { channel_id, messages, complete })
            }
            Request::Typing { channel_id } => match self.can_see(channel_id).and_then(|_| self.not_blocked_in(channel_id)) {
                Ok(()) => hub.typing(self.conn_id, &self.uid, channel_id),
                Err(_) => Ok(Event::Ok),
            },
            Request::OpenDm { user_id } => {
                valid_uid(&user_id)?;
                if user_id == self.uid {
                    return bad("you can't message yourself");
                }
                match db.get_user(&user_id) {
                    Some(u) if !u.banned => {}
                    _ => return Err((ErrorCode::NotFound, "They aren't a member of this server.".into())),
                }
                if db.blocked_between(&self.uid, &user_id) {
                    return Err((ErrorCode::Forbidden, "You can't message this person.".into()));
                }
                let (id, new) = db.open_dm(&self.uid, &user_id).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if new {
                    let theirs = Dm { id, with: self.uid.clone(), last_message: None, last_from: None };
                    hub.send_to_user(&user_id, Event::DmOpened { dm: theirs });
                }
                let mine = db.dms_of(&self.uid).into_iter().find(|d| d.id == id);
                let (last_message, last_from) = mine.map(|d| (d.last_message, d.last_from)).unwrap_or((None, None));
                Ok(Event::DmOpened { dm: Dm { id, with: user_id, last_message, last_from } })
            }

            Request::CreateChannel { name, kind } => {
                self.require(perm::MANAGE_CHANNELS, "manage channels")?;
                let Some(name) = clean_channel_name(&name, kind) else { return bad("channel name is empty") };
                let channel = db.create_channel(&name, kind).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast(Event::ChannelUpserted { channel });
                Ok(Event::Ok)
            }
            Request::UpdateChannel { channel_id, name, topic } => {
                self.require(perm::MANAGE_CHANNELS, "manage channels")?;
                let Some(ch) = db.channel(channel_id) else { return Err((ErrorCode::NotFound, "No such channel.".into())) };
                let name = match name {
                    Some(n) => Some(clean_channel_name(&n, ch.kind).ok_or((ErrorCode::BadRequest, "empty name".into()))?),
                    None => None,
                };
                let topic = topic.map(|t| t.chars().filter(|c| !c.is_control()).take(300).collect::<String>());
                if let Ok(Some(channel)) = db.update_channel(channel_id, name.as_deref(), topic.as_deref()) {
                    hub.broadcast(Event::ChannelUpserted { channel });
                }
                Ok(Event::Ok)
            }
            Request::DeleteChannel { channel_id } => {
                self.require(perm::MANAGE_CHANNELS, "manage channels")?;
                hub.evict_channel(channel_id);
                let (existed, files) = db.delete_channel(channel_id).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.wipe_files(&files);
                if existed {
                    hub.broadcast(Event::ChannelDeleted { channel_id });
                }
                Ok(Event::Ok)
            }
            Request::CreateInvite { max_uses, expires_secs } => {
                if db.invites_by(&self.uid) >= MAX_INVITES_PER_MEMBER {
                    return bad("You've made a lot of invite links already. Use one of those.");
                }
                let code = new_invite_code(10);
                let expires_at = expires_secs.map(|s| ingle_proto::now_ms() + (s.min(365 * 86400) as i64) * 1000);
                db.create_invite(&code, &self.uid, max_uses, expires_at, Role::Member)
                    .map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                Ok(Event::InviteCreated { link: hub.invite_link(&code), code })
            }
            Request::SetRole { user_id, role } => {
                valid_uid(&user_id)?;
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the owner can change roles.".into()));
                }
                if user_id == self.uid || role == Role::Owner {
                    return bad("ownership can't be changed here");
                }
                match db.get_user(&user_id) {
                    Some(u) if !u.banned && u.role != Role::Owner => {}
                    _ => return Err((ErrorCode::NotFound, "No such member.".into())),
                }
                db.set_role(&user_id, role).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if let Some(member) = hub.member(&user_id) {
                    hub.broadcast(Event::MemberUpserted { member });
                }
                Ok(Event::Ok)
            }
            Request::Kick { user_id } | Request::Ban { user_id } if user_id == self.uid => bad("you can't remove yourself"),
            Request::Kick { user_id } | Request::Ban { user_id } if !crypto::is_valid_user_id(&user_id) => {
                Err((ErrorCode::NotFound, "No such member.".into()))
            }
            Request::Kick { user_id } => self.remove(user_id, false),
            Request::Ban { user_id } => self.remove(user_id, true),
            Request::UpdateProfile { name } => {
                let Some(name) = clean_member_name(&name) else { return bad("Pick another name (not empty, not @everyone or @here).") };
                db.touch_user(&self.uid, &name).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if let Some(member) = hub.member(&self.uid) {
                    hub.broadcast(Event::MemberUpserted { member });
                }
                Ok(Event::Ok)
            }
            Request::UpdateServer { name, description } => {
                self.require(perm::MANAGE_SERVER, "change the server's settings")?;
                if let Some(n) = name.and_then(|n| clean_name(&n, 48)) {
                    let _ = db.set_meta("name", &n);
                }
                if let Some(d) = description {
                    let d: String = d.chars().filter(|c| !c.is_control()).take(300).collect();
                    let _ = db.set_meta("description", &d);
                }
                hub.broadcast(Event::ServerUpdated { server: hub.info() });
                Ok(Event::Ok)
            }

            Request::VoiceJoin { channel_id, e2ee_pub, e2ee_sig, self_mute, self_deaf } => {
                hub.join_voice(self.conn_id, channel_id, e2ee_pub, e2ee_sig, self_mute, self_deaf)
            }
            Request::VoiceLeave => hub.leave_voice(self.conn_id),
            Request::VoiceState { self_mute, self_deaf } => hub.set_voice_state(self.conn_id, self_mute, self_deaf),
            Request::Publish { track } => hub.publish(self.conn_id, track),
            Request::Unpublish { track } => hub.unpublish(self.conn_id, track),
            Request::Subscribe { session, track } => hub.subscribe(self.conn_id, session, track, true),
            Request::Unsubscribe { session, track } => hub.subscribe(self.conn_id, session, track, false),
            Request::RequestKeyframe { session, track } => hub.request_keyframe(self.conn_id, session, track),
            Request::E2eeKeys { epoch, keys } => {
                if keys.len() > 256 {
                    return bad("too many keys");
                }
                hub.route_e2ee(self.conn_id, epoch, keys)
            }

            // ---- roles
            Request::CreateRole { name, color, perms, hoist } => {
                self.require(perm::MANAGE_ROLES, "manage roles")?;
                let Some(name) = clean_name(&name, 32) else { return bad("role name is empty") };
                if db.roles().len() >= 100 {
                    return bad("this server has the most roles it can have (100)");
                }
                // You can't hand out powers you don't have.
                let perms = perms & perm::ALL;
                if perms & !self.perms() != 0 {
                    return Err((ErrorCode::Forbidden, "You can't create a role with permissions you don't have.".into()));
                }
                let role = db.create_role(&name, color & 0xffffff, perms, hoist).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast(Event::RoleUpserted { role });
                Ok(Event::Ok)
            }
            Request::UpdateRole { role_id, name, color, perms, hoist, shift } => {
                self.require(perm::MANAGE_ROLES, "manage roles")?;
                let Some(mut role) = db.role(role_id) else { return Err((ErrorCode::NotFound, "No such role.".into())) };
                if !self.can_manage_role(&role) {
                    return Err((ErrorCode::Forbidden, "That role is at or above your own highest role.".into()));
                }
                if let Some(n) = name {
                    role.name = clean_name(&n, 32).ok_or((ErrorCode::BadRequest, "role name is empty".into()))?;
                }
                if let Some(c) = color {
                    role.color = c & 0xffffff;
                }
                if let Some(p) = perms {
                    let p = p & perm::ALL;
                    // Only permissions you have yourself can be switched on or off.
                    if (p ^ role.perms) & !self.perms() != 0 {
                        return Err((ErrorCode::Forbidden, "You can only change permissions you have yourself.".into()));
                    }
                    role.perms = p;
                }
                if let Some(h) = hoist {
                    role.hoist = h;
                }
                let mut changed = vec![];
                if shift != 0 {
                    // Swap places with the neighbour above or below.
                    let all = db.roles(); // highest first
                    if let Some(i) = all.iter().position(|r| r.id == role_id) {
                        let j = if shift > 0 { i.checked_sub(1) } else { Some(i + 1).filter(|j| *j < all.len()) };
                        if let Some(j) = j {
                            let mut other = all[j].clone();
                            if !self.can_manage_role(&other) {
                                return Err((ErrorCode::Forbidden, "You can't move a role above your own highest role.".into()));
                            }
                            std::mem::swap(&mut role.position, &mut other.position);
                            if role.position == other.position {
                                role.position += shift.signum();
                            }
                            changed.push(other);
                        }
                    }
                }
                changed.push(role);
                for r in changed {
                    db.save_role(&r).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                    hub.broadcast(Event::RoleUpserted { role: r });
                }
                Ok(Event::Ok)
            }
            Request::DeleteRole { role_id } => {
                self.require(perm::MANAGE_ROLES, "manage roles")?;
                let Some(role) = db.role(role_id) else { return Ok(Event::Ok) };
                if !self.can_manage_role(&role) {
                    return Err((ErrorCode::Forbidden, "That role is at or above your own highest role.".into()));
                }
                let had = db.delete_role(role_id).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast(Event::RoleDeleted { role_id });
                for uid in had {
                    if let Some(member) = hub.member(&uid) {
                        hub.broadcast(Event::MemberUpserted { member });
                    }
                }
                Ok(Event::Ok)
            }
            Request::SetMemberRoles { user_id, roles } => {
                self.require(perm::MANAGE_ROLES, "manage roles")?;
                let Some(target) = db.get_user(&user_id).filter(|u| !u.banned) else {
                    return Err((ErrorCode::NotFound, "No such member.".into()));
                };
                if user_id != self.uid && !self.role().is_admin() && self.rank_of(&user_id) >= self.rank_of(&self.uid) {
                    return Err((ErrorCode::Forbidden, "You can only change the roles of people below you.".into()));
                }
                if target.role == Role::Owner && self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the owner can change the owner's roles.".into()));
                }
                let all = db.roles();
                let before = db.member_roles(&user_id);
                let mut after: Vec<RoleId> = roles.into_iter().filter(|id| all.iter().any(|r| r.id == *id)).collect();
                after.sort();
                after.dedup();
                // Every role being added or taken away must be below your own highest one, and
                // you can only hand out roles whose powers you have yourself.
                let mine = self.perms();
                for id in before.iter().filter(|r| !after.contains(r)).chain(after.iter().filter(|r| !before.contains(r))) {
                    if let Some(r) = all.iter().find(|r| r.id == *id) {
                        if !self.can_manage_role(r) {
                            return Err((ErrorCode::Forbidden, format!("\"{}\" is at or above your own highest role.", r.name)));
                        }
                        if !before.contains(id) && r.perms & !mine != 0 {
                            return Err((ErrorCode::Forbidden, format!("\"{}\" has permissions you don't have.", r.name)));
                        }
                    }
                }
                db.set_member_roles(&user_id, &after).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if let Some(member) = hub.member(&user_id) {
                    hub.broadcast(Event::MemberUpserted { member });
                }
                Ok(Event::Ok)
            }
            Request::SetAvatar { file_id } => {
                if let Some(id) = &file_id {
                    match db.file(id) {
                        Some(f) if f.owner == self.uid && f.kind == FileKind::Avatar => {}
                        _ => return bad("upload the picture first"),
                    }
                }
                let old = db.set_avatar(&self.uid, file_id.as_deref()).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if let Some(old) = old.filter(|o| Some(o) != file_id.as_ref()) {
                    if db.release_avatar(&old) {
                        hub.wipe_files(&[old]);
                    }
                }
                if let Some(member) = hub.member(&self.uid) {
                    hub.broadcast(Event::MemberUpserted { member });
                }
                Ok(Event::Ok)
            }
            Request::FriendRequest { user_id } => {
                valid_uid(&user_id)?;
                self.other_member(&user_id)?;
                if db.outgoing_requests(&self.uid) >= 200 {
                    return bad("You have a lot of friend requests waiting already.");
                }
                if db.blocked_between(&self.uid, &user_id) {
                    return Err((ErrorCode::Forbidden, "You can't add this person as a friend.".into()));
                }
                let ie = |e: anyhow::Error| (ErrorCode::Internal, e.to_string());
                match db.relation(&self.uid, &user_id) {
                    Some(RelationKind::Friend | RelationKind::Outgoing) => {}
                    Some(RelationKind::Incoming) => {
                        db.clear_relation(&self.uid, &user_id, &[0]).map_err(ie)?;
                        db.set_relation(&self.uid, &user_id, 1).map_err(ie)?;
                        db.set_relation(&user_id, &self.uid, 1).map_err(ie)?;
                    }
                    _ => db.set_relation(&self.uid, &user_id, 0).map_err(ie)?,
                }
                self.tell_relation(&user_id);
                Ok(Event::Ok)
            }
            Request::FriendAccept { user_id } => {
                valid_uid(&user_id)?;
                if db.relation(&self.uid, &user_id) != Some(RelationKind::Incoming) {
                    return bad("There's no friend request from them.");
                }
                let ie = |e: anyhow::Error| (ErrorCode::Internal, e.to_string());
                db.clear_relation(&self.uid, &user_id, &[0]).map_err(ie)?;
                db.set_relation(&self.uid, &user_id, 1).map_err(ie)?;
                db.set_relation(&user_id, &self.uid, 1).map_err(ie)?;
                self.tell_relation(&user_id);
                Ok(Event::Ok)
            }
            Request::FriendRemove { user_id } => {
                valid_uid(&user_id)?;
                db.clear_relation(&self.uid, &user_id, &[0, 1]).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                self.tell_relation(&user_id);
                Ok(Event::Ok)
            }
            Request::Block { user_id } => {
                valid_uid(&user_id)?;
                if user_id == self.uid {
                    return bad("You can't block yourself.");
                }
                let ie = |e: anyhow::Error| (ErrorCode::Internal, e.to_string());
                db.clear_relation(&self.uid, &user_id, &[0, 1]).map_err(ie)?;
                db.set_relation(&self.uid, &user_id, 2).map_err(ie)?;
                self.tell_relation(&user_id);
                Ok(Event::Ok)
            }
            Request::Unblock { user_id } => {
                valid_uid(&user_id)?;
                db.unblock(&self.uid, &user_id).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                self.tell_relation(&user_id);
                Ok(Event::Ok)
            }
            Request::AddEmoji { name, file_id, animated } => {
                self.require(perm::MANAGE_EMOJI, "add emoji")?;
                if !valid_emoji_name(&name) {
                    return bad("Emoji names are 2–32 letters, numbers or underscores.");
                }
                match db.file(&file_id) {
                    Some(f) if f.owner == self.uid && f.kind == FileKind::Avatar => {}
                    _ => return bad("upload the picture first"),
                }
                let all = db.emojis();
                if all.len() >= MAX_CUSTOM_EMOJI {
                    return bad(&format!("This server already has {MAX_CUSTOM_EMOJI} emoji, the most it can have."));
                }
                if all.iter().any(|e| e.name.eq_ignore_ascii_case(&name)) {
                    return bad(&format!("There's already an emoji called :{name}:."));
                }
                let e = CustomEmoji { id: file_id, name, animated, by: self.uid.clone() };
                if !db.add_emoji(&e).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    return bad("That picture is already one of this server's emoji.");
                }
                hub.broadcast(Event::ServerUpdated { server: hub.info() });
                Ok(Event::Ok)
            }
            Request::RemoveEmoji { id } => {
                self.require(perm::MANAGE_EMOJI, "remove emoji")?;
                if db.remove_emoji(&id).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    if db.release_avatar(&id) {
                        hub.wipe_files(&[id]);
                    }
                    hub.broadcast(Event::ServerUpdated { server: hub.info() });
                }
                Ok(Event::Ok)
            }
            Request::RenameEmoji { id, name } => {
                self.require(perm::MANAGE_EMOJI, "rename emoji")?;
                if !valid_emoji_name(&name) {
                    return bad("Emoji names are 2–32 letters, numbers or underscores.");
                }
                if db.emojis().iter().any(|e| e.id != id && e.name.eq_ignore_ascii_case(&name)) {
                    return bad(&format!("There's already an emoji called :{name}:."));
                }
                db.rename_emoji(&id, &name).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast(Event::ServerUpdated { server: hub.info() });
                Ok(Event::Ok)
            }
            Request::AddSound { file_id, name, emoji, volume } => {
                self.require(perm::MANAGE_EMOJI, "add sounds")?;
                let Some(name) = clean_sound_name(&name) else {
                    return bad(&format!("Sound names are 1–{MAX_SOUND_NAME} characters."));
                };
                let Some(emoji) = clean_sound_emoji(&emoji) else { return bad("That emoji won't do.") };
                let Some(f) = db.file(&file_id).filter(|f| f.kind == FileKind::Sound) else { return bad("upload the sound first") };
                let Some(length_ms) =
                    hub.file_path(&f.id).and_then(|p| std::fs::read(p).ok()).and_then(|b| ingle_proto::sound::check(&b)).map(|s| s.length_ms())
                else {
                    return bad("upload the sound first");
                };
                let all = db.sounds();
                if all.len() >= MAX_SERVER_SOUNDS {
                    return bad(&format!("This server already has {MAX_SERVER_SOUNDS} sounds, the most it can have."));
                }
                let s = ServerSound { id: file_id, name, emoji, volume: volume.clamp(0.0, 1.0), by: self.uid.clone(), length_ms };
                if !db.add_sound(&s).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    return bad("That sound is already on this server's soundboard.");
                }
                hub.broadcast(Event::ServerUpdated { server: hub.info() });
                Ok(Event::Ok)
            }
            Request::EditSound { id, name, emoji, volume } => {
                self.require(perm::MANAGE_EMOJI, "change sounds")?;
                let name = match name {
                    Some(n) => Some(clean_sound_name(&n).ok_or((ErrorCode::BadRequest, format!("Sound names are 1–{MAX_SOUND_NAME} characters.")))?),
                    None => None,
                };
                let emoji = match emoji {
                    Some(e) => Some(clean_sound_emoji(&e).ok_or((ErrorCode::BadRequest, "That emoji won't do.".to_string()))?),
                    None => None,
                };
                let volume = volume.map(|v| v.clamp(0.0, 1.0));
                if !db.edit_sound(&id, name.as_deref(), emoji.as_deref(), volume).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    return Err((ErrorCode::NotFound, "That sound is gone.".into()));
                }
                hub.broadcast(Event::ServerUpdated { server: hub.info() });
                Ok(Event::Ok)
            }
            Request::RemoveSound { id } => {
                self.require(perm::MANAGE_EMOJI, "remove sounds")?;
                if db.remove_sound(&id).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    hub.wipe_files(&[id]);
                    hub.broadcast(Event::ServerUpdated { server: hub.info() });
                }
                Ok(Event::Ok)
            }
            Request::PlaySound { sound, name, emoji, volume } => {
                if !self.sound_tokens.take(1.0) {
                    return Err((ErrorCode::RateLimited, "Easy on the sounds: try again in a moment.".into()));
                }
                let known = ingle_proto::sound::is_builtin(&sound) || db.file(&sound).is_some_and(|f| f.kind == FileKind::Sound);
                if !known {
                    return Err((ErrorCode::NotFound, "upload the sound first".into()));
                }
                let name: String = name.chars().filter(|c| !c.is_control()).take(MAX_SOUND_NAME).collect();
                let emoji = clean_sound_emoji(&emoji).unwrap_or_default();
                hub.play_sound(self.conn_id, sound, name, emoji, volume.clamp(0.0, 1.0))
            }
            Request::SetServerIcon { file_id } => {
                self.require(perm::MANAGE_SERVER, "change the server's icon")?;
                if let Some(id) = &file_id {
                    match db.file(id) {
                        Some(f) if f.owner == self.uid && f.kind == FileKind::Avatar => {}
                        _ => return bad("upload the picture first"),
                    }
                }
                let old = db.get_meta("icon").filter(|i| !i.is_empty());
                db.set_meta("icon", file_id.as_deref().unwrap_or("")).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if let Some(old) = old.filter(|o| Some(o) != file_id.as_ref()) {
                    if db.release_avatar(&old) {
                        hub.wipe_files(&[old]);
                    }
                }
                hub.broadcast(Event::ServerUpdated { server: hub.info() });
                Ok(Event::Ok)
            }
            Request::ListBugs => {
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the server's owner can read bug reports.".into()));
                }
                Ok(Event::Bugs { reports: db.bugs(500) })
            }
            Request::SetBugStatus { id, status } => {
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the server's owner can approve bug reports.".into()));
                }
                if !db.set_bug_status(id, status).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    return Err((ErrorCode::NotFound, "That report is gone.".into()));
                }
                hub.broadcast(Event::ServerUpdated { server: hub.info() });
                Ok(Event::Ok)
            }
            Request::SetBugReports { on } => {
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the server's owner can do that.".into()));
                }
                db.set_meta("bug_reports", if on { "1" } else { "" }).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast(Event::ServerUpdated { server: hub.info() });
                Ok(Event::Ok)
            }
            Request::ReportBug { .. } => bad("send bug reports instead of logging in"),
            Request::SetGiphyKey { key } => {
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the server's owner can set up GIF search.".into()));
                }
                let key = key.trim();
                if key.len() > 64 || !key.bytes().all(|b| b.is_ascii_alphanumeric()) {
                    return bad("that doesn't look like a GIPHY API key (letters and numbers, from developers.giphy.com)");
                }
                db.set_meta("giphy_key", key).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast(Event::ServerUpdated { server: hub.info() });
                Ok(Event::Ok)
            }
            Request::Unknown => Err((ErrorCode::BadRequest, "This server is older than your app and can't do that yet.".into())),
        }
    }

    fn remove(&self, user_id: UserId, ban: bool) -> Reply {
        if ban {
            self.require(perm::BAN, "ban members")?;
        } else {
            self.require(perm::KICK, "kick members")?;
        }
        if self.rank_of(&user_id) >= self.rank_of(&self.uid) {
            return Err((ErrorCode::Forbidden, "You can't remove someone with the same or a higher role.".into()));
        }
        if let Err(e) = self.hub.db.remove_user(&user_id, ban) {
            warn!("remove_user failed: {e}");
            return Err((ErrorCode::Internal, e.to_string()));
        }
        let reason = if ban { "You were banned from this server." } else { "You were kicked from this server." };
        self.hub.eject(&user_id, reason);
        Ok(Event::Ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn uid() -> String {
        crypto::Identity::generate().user_id()
    }

    fn hub() -> Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("ingle-conn-test-{}", crypto::random_bytes::<6>().iter().map(|b| format!("{b:02x}")).collect::<String>()));
        Hub::new(Db::open_in_memory().unwrap(), [7u8; 32], "example.org".into(), 7777, dir)
    }

    fn ctx<'a>(hub: &'a Hub, uid: &str) -> Ctx<'a> {
        Ctx {
            hub,
            conn_id: 1,
            uid: uid.to_string(),
            msg_tokens: Bucket::new(1.5, 10.0),
            react_tokens: Bucket::new(3.0, 15.0),
            req_tokens: Bucket::new(REQ_RATE, REQ_BURST),
            sound_tokens: Bucket::new(SOUND_RATE, SOUND_BURST),
        }
    }

    fn forbidden(r: Reply) -> bool {
        matches!(r, Err((ErrorCode::Forbidden, _)))
    }

    #[test]
    fn role_managers_cant_hand_out_powers_they_lack() {
        let hub = hub();
        let (owner, bob, carol) = (uid(), uid(), uid());
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        hub.db.insert_user(&bob, "Bob", Role::Member).unwrap();
        hub.db.insert_user(&carol, "Carol", Role::Member).unwrap();
        // "Role manager" first, then "Moderator" (new roles go to the bottom, so it's below).
        let manager = hub.db.create_role("Role manager", 0, perm::MANAGE_ROLES, false).unwrap();
        let moderator = hub.db.create_role("Moderator", 0, perm::KICK | perm::BAN, false).unwrap();
        let helper = hub.db.create_role("Helper", 0, 0, false).unwrap();
        hub.db.set_member_roles(&bob, &[manager.id]).unwrap();
        let mut b = ctx(&hub, &bob);
        // Giving himself (or Carol) the Moderator role would give powers he doesn't have.
        assert!(forbidden(b.handle(Request::SetMemberRoles { user_id: bob.clone(), roles: vec![manager.id, moderator.id] })));
        assert!(forbidden(b.handle(Request::SetMemberRoles { user_id: carol.clone(), roles: vec![moderator.id] })));
        // A role with nothing he lacks is fine.
        assert!(b.handle(Request::SetMemberRoles { user_id: carol.clone(), roles: vec![helper.id] }).is_ok());
        // Nor can he switch Kick/Ban off (or on) on a lower role.
        let r = b.handle(Request::UpdateRole { role_id: moderator.id, name: None, color: None, perms: Some(perm::KICK), hoist: None, shift: 0 });
        assert!(forbidden(r));
        // The owner can.
        let mut o = ctx(&hub, &owner);
        assert!(o.handle(Request::SetMemberRoles { user_id: carol.clone(), roles: vec![moderator.id] }).is_ok());
    }

    #[test]
    fn echoed_fields_are_bounded_and_ids_checked() {
        let hub = hub();
        let (owner, bob) = (uid(), uid());
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        hub.db.insert_user(&bob, "Bob", Role::Member).unwrap();
        let ch = hub.db.create_channel("general", ChannelKind::Text).unwrap();
        let mut b = ctx(&hub, &bob);
        let send = |c: &mut Ctx, nonce: String| {
            c.handle(Request::SendMessage { channel_id: ch.id, content: "hi".into(), reply_to: None, nonce, attachments: vec![] })
        };
        assert!(send(&mut b, "x".repeat(MAX_NONCE_LEN + 1)).is_err(), "a huge nonce would be echoed to everyone");
        assert!(send(&mut b, "n1".into()).is_ok());
        // Ids that can't be anyone are refused before they reach the database or a broadcast.
        let mut o = ctx(&hub, &owner);
        assert!(o.handle(Request::Ban { user_id: "x".repeat(5000) }).is_err());
        assert!(b.handle(Request::Block { user_id: "y".repeat(5000) }).is_err());
        assert!(b.handle(Request::FriendRequest { user_id: format!("{owner}=") }).is_err());
        // Names that would read as @everyone or show as nothing.
        assert!(b.handle(Request::UpdateProfile { name: "@everyone".into() }).is_err());
        assert!(b.handle(Request::UpdateProfile { name: "\u{200b}\u{3164}".into() }).is_err());
        assert!(b.handle(Request::UpdateProfile { name: "Bo\u{202e}b".into() }).is_ok());
        assert_eq!(hub.db.get_user(&bob).unwrap().name, "Bob");
    }

    #[test]
    fn reactions_are_real_emoji_and_custom_ones_are_rewritten() {
        let hub = hub();
        let (owner, bob) = (uid(), uid());
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        hub.db.insert_user(&bob, "Bob", Role::Member).unwrap();
        let ch = hub.db.create_channel("general", ChannelKind::Text).unwrap();
        let mid = hub.db.insert_message(ch.id, &owner, "hello", None).unwrap().id;
        let id = "0123456789abcdef0123456789abcdef".to_string();
        hub.db.add_emoji(&CustomEmoji { id: id.clone(), name: "party".into(), animated: false, by: owner.clone() }).unwrap();
        let mut b = ctx(&hub, &bob);
        let react = |c: &mut Ctx, e: &str| c.handle(Request::React { message_id: mid, emoji: e.into(), add: true });
        assert!(react(&mut b, "👍🏽").is_ok());
        assert!(react(&mut b, "1\u{fe0f}\u{20e3}").is_ok());
        assert!(react(&mut b, "<<>>").is_err());
        assert!(react(&mut b, "a").is_err());
        assert!(react(&mut b, "\u{200b}").is_err());
        assert!(react(&mut b, &format!("<:anything at all here:{id}>")).is_ok());
        let m = hub.db.get_message(mid).unwrap();
        assert!(m.reactions.iter().any(|r| r.emoji == format!("<:party:{id}>")), "{:?}", m.reactions);
        assert!(!m.reactions.iter().any(|r| r.emoji.contains("anything")));
        assert!(react(&mut b, "<:nope:ffffffffffffffffffffffffffffffff>").is_err());
    }

    #[test]
    fn every_picker_emoji_is_a_valid_reaction() {
        let index: serde_json::Value = serde_json::from_str(include_str!("../../app/assets/emoji/index.json")).unwrap();
        let mut bad = Vec::new();
        let mut n = 0;
        for g in index["groups"].as_array().unwrap() {
            for it in g["items"].as_array().unwrap() {
                let mut all = vec![it["e"].as_str().unwrap().to_string()];
                if let Some(t) = it["t"].as_array() {
                    for v in t {
                        all.push(v[0].as_str().unwrap_or_else(|| v.as_str().unwrap()).to_string());
                    }
                }
                for e in all {
                    n += 1;
                    if !super::valid_unicode_reaction(&e) {
                        bad.push(format!("{} {:?}", it["n"], e.chars().map(|c| format!("{:x}", c as u32)).collect::<Vec<_>>()));
                    }
                }
            }
        }
        assert!(n > 2500, "{n}");
        assert!(bad.is_empty(), "{} of {n} refused: {:?}", bad.len(), &bad[..bad.len().min(30)]);
    }

    #[test]
    fn history_pages_stay_inside_one_frame() {
        let hub = hub();
        let a = uid();
        hub.db.insert_user(&a, "A", Role::Owner).unwrap();
        let ch = hub.db.create_channel("general", ChannelKind::Text).unwrap();
        // 100 messages of 4000 four-byte characters: 1.6 MB, more than a frame.
        let big: String = "😀".repeat(MAX_MESSAGE_CHARS);
        for _ in 0..100 {
            hub.db.insert_message(ch.id, &a, &big, None).unwrap();
        }
        let mut c = ctx(&hub, &a);
        let Ok(ev) = c.handle(Request::FetchHistory { channel_id: ch.id, before: None, limit: 100 }) else { panic!() };
        let Event::History { messages, complete, .. } = &ev else { panic!() };
        assert!(!complete);
        assert!(messages.len() < 100 && !messages.is_empty());
        assert!(frame(1, ev.clone()).len() < ingle_proto::MAX_CONTROL_FRAME);
    }

    #[test]
    fn blocked_people_cant_edit_react_or_type_in_dms() {
        let hub = hub();
        let (a, b) = (uid(), uid());
        hub.db.insert_user(&a, "A", Role::Member).unwrap();
        hub.db.insert_user(&b, "B", Role::Member).unwrap();
        let (dm, _) = hub.db.open_dm(&a, &b).unwrap();
        let mid = hub.db.insert_message(dm, &b, "hi", None).unwrap().id;
        hub.db.set_relation(&a, &b, 2).unwrap();
        let mut cb = ctx(&hub, &b);
        assert!(forbidden(cb.handle(Request::EditMessage { message_id: mid, content: "something mean".into() })));
        assert!(forbidden(cb.handle(Request::React { message_id: mid, emoji: "😠".into(), add: true })));
        assert_eq!(hub.db.get_message(mid).unwrap().content, "hi");
    }

    #[test]
    fn invites_capped_and_requests_rate_limited() {
        let hub = hub();
        let a = uid();
        hub.db.insert_user(&a, "A", Role::Member).unwrap();
        let mut c = ctx(&hub, &a);
        for _ in 0..MAX_INVITES_PER_MEMBER {
            assert!(c.handle(Request::CreateInvite { max_uses: None, expires_secs: None }).is_ok());
        }
        assert!(c.handle(Request::CreateInvite { max_uses: None, expires_secs: None }).is_err());
        let mut bucket = Bucket::new(REQ_RATE, REQ_BURST);
        let allowed = (0..1000).filter(|_| bucket.take(1.0)).count();
        assert!(allowed < 120, "{allowed}");
        // Kicking someone retires the invites they made.
        hub.db.remove_user(&a, false).unwrap();
        assert_eq!(hub.db.invites_by(&a), 0);
    }

    #[test]
    fn invites_and_bans_at_login() {
        let hub = hub();
        let ip: std::net::IpAddr = "203.0.113.5".parse().unwrap();
        let (a, b) = (uid(), uid());
        hub.db.create_invite("fivetimes1", "server", Some(5), None, Role::Member).unwrap();
        hub.authorize(&a, "A", Some("fivetimes1"), ip).unwrap();
        // Reconnecting with the same invite doesn't use it up.
        for _ in 0..10 {
            hub.authorize(&a, "A", Some("fivetimes1"), ip).unwrap();
        }
        hub.authorize(&b, "B", Some("fivetimes1"), ip).unwrap();
        // A ban sticks: joining again with an invite doesn't undo it.
        hub.db.remove_user(&b, true).unwrap();
        assert!(hub.authorize(&b, "B", Some("fivetimes1"), ip).is_err());
        assert!(!hub.db.insert_user(&b, "B", Role::Member).unwrap());
        // Guessing invite codes gets throttled per address.
        for i in 0..BAD_INVITES {
            assert!(matches!(hub.authorize(&uid(), "X", Some(&format!("guess{i}")), ip), Err((ErrorCode::InviteInvalid, _))));
        }
        assert!(matches!(hub.authorize(&uid(), "X", Some("fivetimes1"), ip), Err((ErrorCode::RateLimited, _))));
        let other: std::net::IpAddr = "198.51.100.7".parse().unwrap();
        assert!(hub.authorize(&uid(), "Y", Some("fivetimes1"), other).is_ok());
    }

    const BAD_INVITES: u32 = 10;

    #[test]
    fn canonical_ids_only() {
        let id = uid();
        assert!(crypto::is_valid_user_id(&id));
        assert!(!crypto::is_valid_user_id(&format!("{id}=")));
        assert!(!crypto::is_valid_user_id(&format!(" {id}")));
    }

    #[test]
    fn decompression_bombs_are_refused() {
        // A GIF that says it's 60000x60000 (with one tiny frame): the canvas alone would be
        // gigabytes.
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&60000u16.to_le_bytes());
        gif.extend_from_slice(&60000u16.to_le_bytes());
        gif.extend_from_slice(&[0, 0, 0, 0x2c, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x4c, 1, 0, 0x3b]);
        assert!(!image_dimensions_ok(&gif));
        // Many big frames: too much in all.
        let mut anim = b"GIF89a".to_vec();
        anim.extend_from_slice(&4000u16.to_le_bytes());
        anim.extend_from_slice(&4000u16.to_le_bytes());
        anim.extend_from_slice(&[0, 0, 0]);
        for _ in 0..60 {
            anim.extend_from_slice(&[0x2c, 0, 0, 0, 0, 0xa0, 0x0f, 0xa0, 0x0f, 0, 2, 2, 0x4c, 1, 0]);
        }
        anim.push(0x3b);
        assert!(!image_dimensions_ok(&anim));
        // A normal small GIF is fine.
        let mut ok = b"GIF89a".to_vec();
        ok.extend_from_slice(&64u16.to_le_bytes());
        ok.extend_from_slice(&64u16.to_le_bytes());
        ok.extend_from_slice(&[0, 0, 0, 0x2c, 0, 0, 0, 0, 64, 0, 64, 0, 0, 2, 2, 0x4c, 1, 0, 0x3b]);
        assert!(image_dimensions_ok(&ok));
    }
}

/// One part of a backup (see [`ingle_proto::control::BACKUP_MANIFEST`]), for the owner only:
/// it's everything on the server, direct messages and the server's own key included.
async fn backup_item(hub: &Hub, uid: &str, item: &str, send: &mut quinn::SendStream) -> Result<()> {
    use ingle_proto::control::{BACKUP_CERT, BACKUP_DATABASE, BACKUP_FILE_PREFIX, BACKUP_KEY, BACKUP_MANIFEST, BackupManifest};
    let reply = |r: FileReply| framing::encode(&r);
    let fail = |msg: &str| reply(FileReply { ok: false, id: String::new(), mime: String::new(), size: 0, error: msg.to_string() });
    if hub.member(uid).map(|m| m.role) != Some(Role::Owner) {
        send.write_all(&fail("Only the server's owner can back it up.")).await?;
        let _ = send.finish();
        return Ok(());
    }
    let data_dir = hub.files_dir.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let bytes: Result<Vec<u8>, String> = match item {
        BACKUP_MANIFEST => {
            let m = BackupManifest {
                version: env!("CARGO_PKG_VERSION").to_string(),
                made_at: ingle_proto::now_ms(),
                name: hub.info().name,
                files: hub.db.all_files().into_iter().filter(|(id, _)| hub.file_path(id).map(|p| p.is_file()).unwrap_or(false)).collect(),
            };
            serde_json::to_vec(&m).map_err(|e| e.to_string())
        }
        BACKUP_DATABASE => {
            let tag: String = ingle_proto::crypto::random_bytes::<8>().iter().map(|b| format!("{b:02x}")).collect();
            let tmp = data_dir.join(format!("backup-{tag}.db"));
            // Quick (it copies pages), and nothing may write meanwhile anyway.
            let r = hub.db.snapshot(&tmp).and_then(|()| Ok(std::fs::read(&tmp)?));
            crate::hub::wipe_file(&tmp);
            r.map_err(|e| format!("{e:#}"))
        }
        BACKUP_CERT | BACKUP_KEY => tokio::fs::read(data_dir.join(item)).await.map_err(|e| e.to_string()),
        other => match other.strip_prefix(BACKUP_FILE_PREFIX).and_then(|id| hub.file_path(id)) {
            Some(path) => tokio::fs::read(&path).await.map_err(|_| "That file is gone.".to_string()),
            None => Err("Unknown backup item.".to_string()),
        },
    };
    match bytes {
        Ok(b) => {
            send.write_all(&reply(FileReply { ok: true, id: item.to_string(), mime: String::new(), size: b.len() as u64, error: String::new() }))
                .await?;
            send.write_all(&b).await?;
        }
        Err(e) => {
            tracing::warn!("backup of {item} failed: {e}");
            send.write_all(&fail(&format!("The server couldn't read its {item}: {e}"))).await?;
        }
    }
    let _ = send.finish();
    Ok(())

}
