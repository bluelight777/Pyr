//! One client connection: login handshake, control requests, and media ingest.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use bytes::{Bytes, BytesMut};
use ingle_proto::control::*;
use ingle_proto::media::{self, AudioHeader, VideoHeader, track};
use ingle_proto::{MAX_MESSAGE_CHARS, MAX_VIDEO_FRAME, PROTOCOL_VERSION, crypto, framing};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::db::new_invite_code;
use crate::hub::{ConnId, Hub, Reply, frame};

pub async fn handle(hub: Arc<Hub>, incoming: quinn::Incoming) {
    let remote = incoming.remote_address();
    let conn = match incoming.await {
        Ok(c) => c,
        Err(e) => {
            debug!(%remote, "handshake failed: {e}");
            return;
        }
    };
    match session(hub, conn.clone()).await {
        Ok(()) => debug!(%remote, "connection closed"),
        Err(e) => debug!(%remote, "connection ended: {e:#}"),
    }
    conn.close(quinn::VarInt::from_u32(0), b"bye");
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
    if !crypto::verify(&user_id, &crypto::login_transcript(&nonce, &exporter, &user_id), &sig) {
        send.write_all(&reject(ErrorCode::Unauthorized, "identity proof failed")).await?;
        bail!("bad login signature");
    }
    let name = clean_name(&name, 32).unwrap_or_else(|| "Anonymous".to_string());
    if let Err((code, msg)) = hub.authorize(&user_id, &name, invite.as_deref()) {
        send.write_all(&reject(code, &msg)).await?;
        let _ = send.finish();
        tokio::time::sleep(Duration::from_millis(300)).await;
        bail!("login refused: {msg}");
    }
    info!(user = %&user_id[..8], %name, %client, remote = %conn.remote_address(), "logged in");

    let (tx, mut rx) = mpsc::unbounded_channel::<Bytes>();
    let writer = tokio::spawn(async move {
        while let Some(b) = rx.recv().await {
            if send.write_all(&b).await.is_err() {
                break;
            }
        }
        let _ = send.finish();
    });

    let conn_id = hub.alloc_conn_id();
    let session_cell = Arc::new(AtomicU32::new(0));
    hub.register(conn_id, &user_id, conn.clone(), tx.clone(), session_cell.clone(), login.id);

    let dg = tokio::spawn(datagram_loop(hub.clone(), conn.clone(), session_cell.clone()));
    let uni = tokio::spawn(uni_loop(hub.clone(), conn.clone(), session_cell.clone()));
    let files = tokio::spawn(file_loop(hub.clone(), conn.clone(), user_id.clone()));

    let mut ctx = Ctx { hub: &hub, conn_id, uid: user_id.clone(), msg_tokens: 10.0, last_refill: Instant::now() };
    let result = async {
        loop {
            let Some(f) = read_frame::<ClientFrame>(&mut recv).await? else { return Ok(()) };
            let id = f.id;
            let reply = ctx.handle(f.req);
            let ev = reply.unwrap_or_else(|(code, message)| Event::Error { code, message });
            if id != 0 {
                let _ = tx.send(frame(id, ev));
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

/// Video frames: one unidirectional stream per frame.
async fn uni_loop(hub: Arc<Hub>, conn: quinn::Connection, session: Arc<AtomicU32>) {
    while let Ok(mut stream) = conn.accept_uni().await {
        let hub = hub.clone();
        let session = session.clone();
        tokio::spawn(async move {
            let Ok(Ok(data)) =
                timeout(Duration::from_secs(5), stream.read_to_end(MAX_VIDEO_FRAME + media::VIDEO_HEADER_LEN)).await
            else {
                return;
            };
            let s = session.load(Ordering::Relaxed);
            if s == 0 {
                return;
            }
            let Some(h) = VideoHeader::parse(&data) else { return };
            if data.len() != media::VIDEO_HEADER_LEN + h.len as usize {
                return;
            }
            let mut b = BytesMut::from(&data[..]);
            media::set_session(&mut b, s);
            hub.forward_video(s, h.track, h.is_keyframe(), b.freeze());
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
            // A picture sent in a direct message is only for its two people.
            let private = row
                .message_id
                .and_then(|m| hub.db.get_message(m))
                .and_then(|m| hub.db.dm_members(m.channel_id))
                .map(|(a, b)| a != uid && b != uid)
                .unwrap_or(false);
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
        FileRequest::Put { kind, name, mime: _, size } => {
            let limit = match kind {
                FileKind::Attachment => MAX_ATTACHMENT_BYTES,
                FileKind::Avatar => MAX_AVATAR_BYTES,
            };
            if size == 0 || size > limit {
                send.write_all(&fail(&format!("Images can be up to {} MB.", limit >> 20))).await?;
                let _ = send.finish();
                return Ok(());
            }
            let cap: u64 = std::env::var("INGLE_STORAGE_MB").ok().and_then(|v| v.parse().ok()).unwrap_or(5 * 1024) << 20;
            if hub.db.files_total() + size > cap {
                send.write_all(&fail("This server's storage for images is full.")).await?;
                let _ = send.finish();
                return Ok(());
            }
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
            let id = match kind {
                // Profile pictures are named by their content, so an app can tell whether the
                // server already has its current one.
                FileKind::Avatar => avatar_id(uid, &bytes),
                FileKind::Attachment => crypto::random_bytes::<16>().iter().map(|b| format!("{b:02x}")).collect(),
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
            send.write_all(&reply(FileReply { ok: true, id, mime: mime.to_string(), size, error: String::new() })).await?;
            let _ = send.finish();
        }
    }
    // Let the reply reach the other side before the stream is dropped.
    let _ = timeout(Duration::from_secs(10), send.stopped()).await;
    Ok(())
}

struct Ctx<'a> {
    hub: &'a Hub,
    conn_id: ConnId,
    uid: String,
    msg_tokens: f64,
    last_refill: Instant,
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
        let now = Instant::now();
        let dt = now.duration_since(self.last_refill).as_secs_f64();
        self.last_refill = now;
        self.msg_tokens = (self.msg_tokens + dt * 1.5).min(10.0);
        if self.msg_tokens < 1.0 {
            return Err((ErrorCode::RateLimited, "You're sending messages too fast.".into()));
        }
        self.msg_tokens -= 1.0;
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
                if let Some((x, y)) = db.dm_members(channel_id) {
                    let other = if x == self.uid { y } else { x };
                    if db.blocked_between(&self.uid, &other) {
                        return Err((ErrorCode::Forbidden, "You can't message this person.".into()));
                    }
                }
                let reply_to = reply_to.filter(|id| db.get_message(*id).map(|m| m.channel_id == channel_id).unwrap_or(false));
                let message = db
                    .insert_message_with(channel_id, &self.uid, &content, reply_to, &attachments)
                    .map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast_in(channel_id, Event::MessageCreated { message, nonce });
                Ok(Event::Ok)
            }
            Request::EditMessage { message_id, content } => {
                let Some(content) = clean_content(&content) else { return bad("message is empty or too long") };
                let Some(m) = db.get_message(message_id) else { return Err((ErrorCode::NotFound, "gone".into())) };
                if m.author != self.uid {
                    return Err((ErrorCode::Forbidden, "You can only edit your own messages.".into()));
                }
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
                let emoji = emoji.trim().to_string();
                // A server emoji (`<:name:id>`, which must exist here) or a real emoji.
                let custom = emoji.starts_with('<') && emoji.ends_with('>');
                if custom {
                    let id = emoji.trim_end_matches('>').rsplit(':').next().unwrap_or("");
                    if emoji.len() > 120 || !db.emojis().iter().any(|e| e.id == id) {
                        return bad("not an emoji on this server");
                    }
                } else if emoji.is_empty() || emoji.len() > 32 || emoji.chars().any(|c| c.is_ascii_alphanumeric() || c.is_whitespace()) {
                    return bad("not an emoji");
                }
                let Some(m) = db.get_message(message_id) else { return Err((ErrorCode::NotFound, "gone".into())) };
                self.can_see(m.channel_id)?;
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
                let (messages, complete) = db.history(channel_id, before, limit);
                Ok(Event::History { channel_id, messages, complete })
            }
            Request::Typing { channel_id } => match self.can_see(channel_id) {
                Ok(()) => hub.typing(self.conn_id, &self.uid, channel_id),
                Err(_) => Ok(Event::Ok),
            },
            Request::OpenDm { user_id } => {
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
                let code = new_invite_code(8);
                let expires_at = expires_secs.map(|s| ingle_proto::now_ms() + (s.min(365 * 86400) as i64) * 1000);
                db.create_invite(&code, &self.uid, max_uses, expires_at, Role::Member)
                    .map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                Ok(Event::InviteCreated { link: hub.invite_link(&code), code })
            }
            Request::SetRole { user_id, role } => {
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
            Request::Kick { user_id } => self.remove(user_id, false),
            Request::Ban { user_id } => self.remove(user_id, true),
            Request::UpdateProfile { name } => {
                let Some(name) = clean_name(&name, 32) else { return bad("name is empty") };
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
                    if p & !self.perms() != 0 {
                        return Err((ErrorCode::Forbidden, "You can't give a role permissions you don't have.".into()));
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
                // Every role being added or taken away must be below your own highest one.
                for id in before.iter().filter(|r| !after.contains(r)).chain(after.iter().filter(|r| !before.contains(r))) {
                    if let Some(r) = all.iter().find(|r| r.id == *id) {
                        if !self.can_manage_role(r) {
                            return Err((ErrorCode::Forbidden, format!("\"{}\" is at or above your own highest role.", r.name)));
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
                self.other_member(&user_id)?;
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
                db.clear_relation(&self.uid, &user_id, &[0, 1]).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                self.tell_relation(&user_id);
                Ok(Event::Ok)
            }
            Request::Block { user_id } => {
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
