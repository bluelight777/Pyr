//! One task per joined server: connect, log in, keep the shared state in sync, reconnect.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ingle_proto::control::*;
use ingle_proto::framing;
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};

use crate::Shared;
use crate::call::CallEvent;
use crate::config::ServerEntry;
use crate::net::{self, Fatal};
use crate::state::{ConnStatus, ServerView};

enum Outgoing {
    Req { req: Request, reply: Option<oneshot::Sender<Event>> },
    Close,
}

#[derive(Clone)]
pub struct ServerHandle {
    pub key: String,
    out: mpsc::UnboundedSender<Outgoing>,
    conn: Arc<Mutex<Option<quinn::Connection>>>,
}

impl ServerHandle {
    /// Send a request and get its reply (an `Event::Error` if not connected).
    pub fn request(&self, req: Request) -> oneshot::Receiver<Event> {
        let (tx, rx) = oneshot::channel();
        if let Err(mpsc::error::SendError(Outgoing::Req { reply: Some(tx), .. })) =
            self.out.send(Outgoing::Req { req, reply: Some(tx) })
        {
            let _ = tx.send(Event::error(ErrorCode::Internal, "not connected"));
        }
        rx
    }

    /// Fire-and-forget request.
    pub fn notify(&self, req: Request) {
        let _ = self.out.send(Outgoing::Req { req, reply: None });
    }

    pub fn connection(&self) -> Option<quinn::Connection> {
        self.conn.lock().clone()
    }

    pub fn close(&self) {
        let _ = self.out.send(Outgoing::Close);
    }
}

pub fn spawn(shared: Arc<Shared>, entry: ServerEntry) -> ServerHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    let conn = Arc::new(Mutex::new(None));
    let handle = ServerHandle { key: entry.key.clone(), out: tx, conn: conn.clone() };
    shared.rt.spawn(run(shared.clone(), entry, rx, conn));
    handle
}

fn set_status(shared: &Shared, key: &str, status: ConnStatus) {
    if let Some(s) = shared.state.write().server_mut(key) {
        s.status = status;
    }
    (shared.repaint)();
}

async fn run(
    shared: Arc<Shared>,
    entry: ServerEntry,
    mut out_rx: mpsc::UnboundedReceiver<Outgoing>,
    conn_slot: Arc<Mutex<Option<quinn::Connection>>>,
) {
    let key = entry.key.clone();
    let Some(fp) = ingle_proto::b64::decode_array::<32>(&key) else {
        set_status(&shared, &key, ConnStatus::Failed("invalid server fingerprint".into()));
        return;
    };
    let mut attempt: u32 = 0;
    loop {
        let name = shared.config.lock().display_name.clone();
        let invite = shared.config.lock().servers.iter().find(|s| s.key == key).and_then(|s| s.invite.clone());
        let result = net::connect_and_login(
            &shared.endpoints,
            &entry.host,
            entry.port,
            fp,
            &shared.identity,
            &name,
            invite.as_deref(),
        )
        .await;
        match result {
            Ok(li) => {
                attempt = 0;
                info!(server = %entry.host, "connected");
                *conn_slot.lock() = Some(li.conn.clone());
                let end = session(&shared, &key, li, &mut out_rx).await;
                *conn_slot.lock() = None;
                shared.server_disconnected(&key);
                match end {
                    SessionEnd::Closed => return,
                    SessionEnd::Kicked(reason) => {
                        set_status(&shared, &key, ConnStatus::Failed(reason));
                        return;
                    }
                    SessionEnd::Lost(e) => {
                        warn!(server = %entry.host, "connection lost: {e}");
                        set_status(&shared, &key, ConnStatus::Reconnecting { attempt: 1, error: e });
                    }
                }
            }
            Err(e) => {
                if let Some(f) = e.downcast_ref::<Fatal>() {
                    set_status(&shared, &key, ConnStatus::Failed(f.0.clone()));
                    // Wait for a retry/close request, failing anything sent meanwhile.
                    while let Some(o) = out_rx.recv().await {
                        match o {
                            Outgoing::Close => return,
                            Outgoing::Req { reply, .. } => {
                                if let Some(r) = reply {
                                    let _ = r.send(Event::error(ErrorCode::Internal, f.0.clone()));
                                }
                            }
                        }
                    }
                    return;
                }
                attempt += 1;
                debug!("connect attempt {attempt} failed: {e:#}");
                set_status(&shared, &key, ConnStatus::Reconnecting { attempt, error: format!("{e:#}") });
            }
        }
        // Quick retries first (a server restarting for an update is back in a few seconds),
        // then slower ones.
        let delay = match attempt {
            0 => Duration::from_millis(500),
            1..=10 => Duration::from_secs(1),
            n => Duration::from_secs((2 * (n as u64 - 10)).min(30)),
        };
        let wake = tokio::time::Instant::now() + delay;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(wake) => break,
                o = out_rx.recv() => match o {
                    None | Some(Outgoing::Close) => return,
                    Some(Outgoing::Req { reply, .. }) => {
                        if let Some(r) = reply {
                            let _ = r.send(Event::error(ErrorCode::Internal, "not connected right now"));
                        }
                    }
                },
            }
        }
    }
}

enum SessionEnd {
    Closed,
    Kicked(String),
    Lost(String),
}

async fn session(
    shared: &Arc<Shared>,
    key: &str,
    li: net::LoggedIn,
    out_rx: &mut mpsc::UnboundedReceiver<Outgoing>,
) -> SessionEnd {
    let net::LoggedIn { conn, mut send, mut recv, ready } = li;
    apply_ready(shared, key, ready);
    crate::sync_avatar(shared.clone(), key.to_string());
    crate::resume_voice(shared.clone(), key.to_string());
    crate::backup::spawn_schedule(shared.clone(), key.to_string(), conn.clone());
    crate::sync_blocks(shared, key);
    {
        // Membership is established; the invite is no longer needed (and isn't sent again, so
        // it isn't used up by reconnecting).
        let mut cfg = shared.config.lock();
        if let Some(e) = cfg.servers.iter_mut().find(|s| s.key == key) {
            e.invite = None;
            if let Some(name) = shared.state.read().server(key).map(|s| s.info.name.clone()) {
                e.name = name;
            }
        }
        shared.save_config_locked(&cfg);
    }
    // Reader task: never cancel a read mid-frame.
    let (frames_tx, mut frames_rx) = mpsc::unbounded_channel::<ServerFrame>();
    let reader = tokio::spawn(async move {
        loop {
            match net::read_frame::<ServerFrame>(&mut recv).await {
                Ok(Some(f)) => {
                    if frames_tx.send(f).is_err() {
                        break;
                    }
                }
                _ => break,
            }
        }
    });
    let mut pending: HashMap<u64, oneshot::Sender<Event>> = HashMap::new();
    let mut next_id: u64 = 2;
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    let end = loop {
        tokio::select! {
            f = frames_rx.recv() => {
                let Some(f) = f else {
                    let reason = conn.close_reason().map(|r| r.to_string()).unwrap_or_else(|| "connection closed".into());
                    break SessionEnd::Lost(reason);
                };
                if let Event::Kicked { reason } = &f.ev {
                    let reason = reason.clone();
                    shared.state.write().toast(reason.clone(), true);
                    break SessionEnd::Kicked(reason);
                }
                if f.re != 0 {
                    if let Some(tx) = pending.remove(&f.re) {
                        let _ = tx.send(f.ev);
                        continue;
                    }
                }
                apply_event(shared, key, f.ev);
            }
            o = out_rx.recv() => match o {
                None | Some(Outgoing::Close) => {
                    conn.close(quinn::VarInt::from_u32(0), b"bye");
                    break SessionEnd::Closed;
                }
                Some(Outgoing::Req { req, reply }) => {
                    let id = if reply.is_some() { next_id += 1; next_id } else { 0 };
                    let bytes = framing::encode(&ClientFrame { id, req });
                    if let Err(e) = send.write_all(&bytes).await {
                        if let Some(r) = reply { let _ = r.send(Event::error(ErrorCode::Internal, "connection lost")); }
                        break SessionEnd::Lost(e.to_string());
                    }
                    if let Some(r) = reply { pending.insert(id, r); }
                }
            },
            _ = tick.tick() => {
                let rtt = conn.rtt().as_millis() as u32;
                let mut st = shared.state.write();
                if let Some(s) = st.server_mut(key) {
                    s.rtt_ms = rtt;
                    // expire typing indicators
                    let now = Instant::now();
                    for m in s.typing.values_mut() {
                        m.retain(|_, t| now.duration_since(*t) < Duration::from_secs(8));
                    }
                }
                drop(st);
                (shared.repaint)();
            }
        }
    };
    reader.abort();
    for (_, tx) in pending.drain() {
        let _ = tx.send(Event::error(ErrorCode::Internal, "connection lost"));
    }
    end
}

fn sort_channels(v: &mut [Channel]) {
    v.sort_by_key(|c| (matches!(c.kind, ChannelKind::Voice), c.position, c.id));
}

fn apply_ready(shared: &Shared, key: &str, ready: Event) {
    let Event::Ready { me, server, mut channels, members, voice, mut roles, features, dms, relations } = ready else { return };
    let last_read = shared.config.lock().last_read.get(key).cloned().unwrap_or_default();
    roles.sort_by_key(|r| (std::cmp::Reverse(r.position), r.id));
    sort_channels(&mut channels);
    let mut st = shared.state.write();
    let my_uid = st.my_uid.clone();
    let s = match st.server_mut(key) {
        Some(s) => s,
        None => {
            st.servers.push(ServerView::new(key.to_string(), server.name.clone(), String::new()));
            st.servers.last_mut().expect("just pushed")
        }
    };
    s.status = ConnStatus::Connected;
    s.name = server.name.clone();
    s.info = server;
    s.me = Some(me);
    s.channels = channels;
    s.members = members.into_iter().map(|m| (m.user_id.clone(), m)).collect();
    s.voice = voice.into_iter().map(|p| (p.session, p)).collect();
    s.roles = roles;
    s.features = features;
    s.relations = relations.into_iter().map(|r| (r.user_id, r.kind)).collect();
    // Direct messages that came in while we were away show as unread.
    s.dms.clear();
    for dm in dms {
        if let Some(last) = dm.last_message.filter(|_| dm.last_from.as_deref() != Some(my_uid.as_str())) {
            if last_read.get(&dm.id).map(|seen| *seen < last).unwrap_or(true) {
                s.unread.insert(dm.id);
                s.mentions.entry(dm.id).or_insert(1);
            }
        }
        s.upsert_dm(dm);
    }
    // History may have changed while we were away; reload lazily.
    for log in s.logs.values_mut() {
        log.loaded = false;
    }
    drop(st);
    (shared.repaint)();
}

fn log_newest(s: &ServerView, cid: ChannelId) -> Option<MessageId> {
    s.logs.get(&cid).and_then(|l| l.messages.last().map(|m| m.id))
}


fn apply_event(shared: &Arc<Shared>, key: &str, ev: Event) {
    let active = shared.active_view.lock().clone();
    let mut forward: Option<CallEvent> = None;
    let mut toast: Option<String> = None;
    let mut dm_note: Option<String> = None;
    let mut ding = false;
    {
        let mut st = shared.state.write();
        let my_uid = st.my_uid.clone();
        let server_muted = st.muted_servers.contains(key);
        let muted_channels: std::collections::HashSet<ChannelId> =
            st.muted_channels.iter().filter(|(k, _)| k == key).map(|(_, c)| *c).collect();
        let blocked = st.blocked.clone();
        let Some(s) = st.server_mut(key) else { return };
        match ev {
            Event::RelationUpdated { user_id, kind } => {
                let before = s.relations.get(&user_id).copied();
                match kind {
                    Some(k) => s.relations.insert(user_id.clone(), k),
                    None => s.relations.remove(&user_id),
                };
                let who = s.member_name(&user_id);
                if kind == Some(RelationKind::Incoming) && before != Some(RelationKind::Incoming) && !blocked.contains(&user_id) {
                    toast = Some(format!("{who} sent you a friend request."));
                    ding = true;
                }
                if kind == Some(RelationKind::Friend) && before == Some(RelationKind::Outgoing) {
                    toast = Some(format!("{who} accepted your friend request."));
                }
            }
            Event::MessageCreated { message, nonce } if blocked.contains(&message.author) => {
                // From someone I blocked: kept (shown folded away), but no ping or unread.
                let cid = message.channel_id;
                let _ = nonce;
                if let Some(log) = s.logs.get_mut(&cid) {
                    if log.loaded {
                        log.upsert(message);
                    }
                }
            }
            Event::MessageCreated { message, nonce } => {
                let cid = message.channel_id;
                let mention = message.author != my_uid && s.pings_me(&message.author, &message.content);
                let log = s.logs.entry(cid).or_default();
                if !nonce.is_empty() {
                    log.pending.retain(|p| p.nonce != nonce);
                }
                let mine = message.author == my_uid;
                log.upsert(message);
                s.typing.entry(cid).or_default().retain(|u, _| !mine || u != &my_uid);
                let viewing = active.as_ref().map(|(k, c)| k == key && *c == cid).unwrap_or(false);
                let in_dm = s.dm(cid).is_some();
                if in_dm {
                    let (with, mid) = (s.dm(cid).map(|d| d.with.clone()).unwrap_or_default(), log_newest(s, cid));
                    s.upsert_dm(Dm { id: cid, with, last_message: mid, last_from: None });
                }
                // A ping for every new message you can see, unless the channel (or server) is
                // muted or you're looking right at it. Mentions and direct messages ping even
                // in a muted server, like Discord.
                let focused = shared.window_focused.load(std::sync::atomic::Ordering::Relaxed);
                let muted = server_muted && !mention && !in_dm || muted_channels.contains(&cid);
                if !mine && !(viewing && focused) && !muted {
                    ding = true;
                }
                if !mine && !viewing {
                    if !muted || mention {
                        s.unread.insert(cid);
                    }
                    // Every direct message counts like a mention (the red badge).
                    if mention || in_dm {
                        *s.mentions.entry(cid).or_default() += 1;
                    }
                    if mention && !in_dm {
                        if let Some(m) = s.logs.get(&cid).and_then(|l| l.messages.last()) {
                            let who = s.member_name(&m.author);
                            let channel = s.channel(cid).map(|c| c.name.clone()).unwrap_or_default();
                            let text: String = m.content.chars().take(80).collect();
                            dm_note = Some(format!("{who} mentioned you in #{channel}: {text}"));
                        }
                    }
                    if in_dm {
                        if let Some(m) = s.logs.get(&cid).and_then(|l| l.messages.last()) {
                            let who = s.member_name(&m.author);
                            let text = if m.content.is_empty() { "sent a picture".to_string() } else { m.content.clone() };
                            let text: String = text.chars().take(80).collect();
                            dm_note = Some(format!("{who}: {text}"));
                        }
                    }
                }
            }
            Event::MessageUpdated { message } => {
                if let Some(log) = s.logs.get_mut(&message.channel_id) {
                    if log.find(message.id).is_some() {
                        log.upsert(message);
                    }
                }
            }
            Event::MessageDeleted { channel_id, message_id } => {
                if let Some(log) = s.logs.get_mut(&channel_id) {
                    // Drop its images from memory too.
                    if let Some(m) = log.find(message_id) {
                        let mut files = shared.files.lock();
                        for a in &m.attachments {
                            files.forget(&a.id);
                        }
                    }
                    log.remove(message_id);
                }
            }
            Event::Typing { channel_id, user_id } => {
                if user_id != my_uid {
                    s.typing.entry(channel_id).or_default().insert(user_id, Instant::now());
                }
            }
            Event::ChannelUpserted { channel } => {
                match s.channels.iter_mut().find(|c| c.id == channel.id) {
                    Some(c) => *c = channel,
                    None => s.channels.push(channel),
                }
                sort_channels(&mut s.channels);
            }
            Event::ChannelDeleted { channel_id } => {
                s.channels.retain(|c| c.id != channel_id);
                s.logs.remove(&channel_id);
                s.unread.remove(&channel_id);
            }
            Event::MemberUpserted { member } => {
                if member.user_id == my_uid {
                    s.me = Some(member.clone());
                }
                s.members.insert(member.user_id.clone(), member);
            }
            Event::MemberRemoved { user_id } => {
                s.members.remove(&user_id);
            }
            Event::RoleUpserted { role } => {
                match s.roles.iter_mut().find(|r| r.id == role.id) {
                    Some(r) => *r = role,
                    None => s.roles.push(role),
                }
                s.roles.sort_by_key(|r| (std::cmp::Reverse(r.position), r.id));
            }
            Event::DmOpened { dm } => s.upsert_dm(dm),
            Event::RoleDeleted { role_id } => {
                s.roles.retain(|r| r.id != role_id);
                for m in s.members.values_mut() {
                    m.roles.retain(|r| *r != role_id);
                }
                if let Some(me) = s.me.as_mut() {
                    me.roles.retain(|r| *r != role_id);
                }
            }
            Event::Unknown => {}
            Event::ServerUpdated { server } => {
                s.name = server.name.clone();
                s.info = server;
            }
            Event::VoiceUpserted { participant } => {
                s.voice.insert(participant.session, participant.clone());
                forward = Some(CallEvent::Upserted(participant));
            }
            Event::VoiceRemoved { session, .. } => {
                s.voice.remove(&session);
                forward = Some(CallEvent::Removed(session));
            }
            Event::KeyframeRequested { track } => forward = Some(CallEvent::KeyframeRequested(track)),
            Event::E2eeKey { from, epoch, nonce, ct } => forward = Some(CallEvent::Key { from, epoch, nonce, ct }),
            Event::Pong { .. } | Event::Ok | Event::Hello { .. } | Event::Ready { .. } => {}
            Event::Error { message, .. } => toast = Some(message),
            Event::History { .. } | Event::InviteCreated { .. } | Event::VoiceReady { .. } | Event::Kicked { .. } => {}
        }
        if let Some(t) = toast {
            st.toast(t, true);
        }
        if let Some(t) = dm_note {
            st.toast(t, false);
        }
    }
    if ding {
        shared.play(crate::sfx::Sound::Message);
    }
    if let Some(ev) = forward {
        shared.forward_call_event(key, ev);
    }
    (shared.repaint)();
}
