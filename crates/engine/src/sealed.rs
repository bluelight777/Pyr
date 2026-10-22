//! End-to-end encrypted messages, on this side (see `pyr_proto::sealed` for the design).
//!
//! Every message that comes in (as an event, or in a reply to a request) is opened here
//! before anything else sees it ([`on_frame`]), so the rest of the engine, the app and the
//! Claude connector only ever deal with what was written. What goes out is sealed here
//! ([`send`], [`edit`]). The keys live in memory only: they're kept on the server, wrapped for
//! each person who can read there (me included), and fetched again as needed.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pyr_proto::b64;
use pyr_proto::control::{Attachment, ChannelId, Event, FileKind, Lock, Message, MessageId, Request, SEALED_FEATURE, UserId};
use pyr_proto::crypto::Identity;
use pyr_proto::sealed::{self, Body, KeyRef, KeyShare, MAX_KEYS_ASKED, MAX_SHARES, MessageKey, MessageKeys, NewShare, OpenError, SealedFile};

use crate::Shared;
use crate::net;
use crate::server::ServerHandle;

/// How an encrypted picture is uploaded (the server isn't told what it is).
const SEALED_MIME: &str = "application/octet-stream";
/// Keys asked for and not given aren't asked for again for this long (someone who has them
/// gives them when they can; the server keeps the asking).
const ASK_AGAIN: Duration = Duration::from_secs(20);
/// A key given to someone isn't given again for this long, however often they ask.
const GIVE_AGAIN: Duration = Duration::from_secs(60);

/// The key I write with in one conversation, and who I've given it to.
struct Outbound {
    key_ref: KeyRef,
    key: [u8; 32],
    given: HashSet<UserId>,
}

/// What this Pyr knows of end-to-end encryption (in memory only).
pub struct Book {
    mine: MessageKeys,
    me: UserId,
    /// Conversation keys I was given or made: (server, conversation, key) → what it might be
    /// (more than one when someone gave a wrong one too; the one that opened a message first).
    keys: HashMap<(String, ChannelId, KeyRef), Vec<[u8; 32]>>,
    /// Encrypted pictures' keys, by file id.
    files: HashMap<String, [u8; 32]>,
    outbound: HashMap<(String, ChannelId), Outbound>,
    /// Who can read each conversation, with their message keys (checked): forgotten whenever
    /// who's where might have changed.
    readers: HashMap<(String, ChannelId), Vec<(UserId, [u8; 32])>>,
    asked: HashMap<(String, ChannelId, KeyRef), Instant>,
    given: HashMap<(String, ChannelId, UserId, KeyRef), Instant>,
}

impl Book {
    pub fn new(identity: &Identity) -> Book {
        Book {
            mine: MessageKeys::derive(identity),
            me: identity.user_id(),
            keys: HashMap::new(),
            files: HashMap::new(),
            outbound: HashMap::new(),
            readers: HashMap::new(),
            asked: HashMap::new(),
            given: HashMap::new(),
        }
    }

    fn add_key(&mut self, server: &str, channel: ChannelId, key_ref: &KeyRef, key: [u8; 32]) {
        let v = self.keys.entry((server.to_string(), channel, key_ref.clone())).or_default();
        if !v.contains(&key) {
            v.push(key);
        }
    }

    fn key(&self, server: &str, channel: ChannelId, key_ref: &KeyRef) -> Option<[u8; 32]> {
        self.keys.get(&(server.to_string(), channel, key_ref.clone())).and_then(|v| v.first().copied())
    }

    fn take_shares(&mut self, server: &str, channel: ChannelId, shares: &[KeyShare]) {
        for s in shares {
            if let Some(k) = self.mine.unwrap(&self.me, channel, s) {
                self.add_key(server, channel, &s.key, k);
            }
        }
    }

    /// The key an encrypted picture opens with.
    pub fn file_key(&self, id: &str) -> Option<[u8; 32]> {
        self.files.get(id).copied()
    }

    fn forget_readers(&mut self, server: &str) {
        self.readers.retain(|(s, _), _| s != server);
    }
}

/// Opens one message in place: its text and pictures, and whether it could be (`Lock`).
/// The key it needs, when this Pyr doesn't have it.
fn open_message(book: &mut Book, server: &str, m: &mut Message) -> Option<KeyRef> {
    let Some(s) = m.sealed.clone() else {
        m.lock = Lock::Plain;
        return None;
    };
    let candidates = book.keys.get(&(server.to_string(), m.channel_id, s.key.clone())).cloned().unwrap_or_default();
    if candidates.is_empty() {
        m.lock = Lock::Waiting;
        m.content.clear();
        m.attachments.clear();
        return Some(s.key);
    }
    let mut result = Err(OpenError::Forged);
    for (i, k) in candidates.iter().enumerate() {
        result = sealed::open(m.channel_id, &m.author, k, &s);
        if result.is_ok() {
            if i > 0 {
                // The right one: first from now on.
                if let Some(v) = book.keys.get_mut(&(server.to_string(), m.channel_id, s.key.clone())) {
                    v.swap(0, i);
                }
            }
            break;
        }
    }
    let from_server = std::mem::take(&mut m.attachments);
    match result {
        Ok(body) => {
            m.content = body.text;
            m.attachments = body
                .files
                .into_iter()
                // Only what was sent with it here (the server links pictures to messages).
                .filter_map(|f| {
                    let on_server = from_server.iter().find(|a| a.id == f.id)?;
                    if f.key.is_empty() {
                        // A picture sent before messages were encrypted (kept by an edit).
                        return Some(on_server.clone());
                    }
                    let key = b64::decode_array::<32>(&f.key)?;
                    book.files.insert(f.id.clone(), key);
                    let side = |v: u32| if v <= pyr_proto::control::MAX_IMAGE_SIDE { v } else { 0 };
                    Some(Attachment { id: f.id, name: f.name, mime: f.mime, size: f.size, width: side(f.width), height: side(f.height) })
                })
                .collect();
            m.lock = Lock::Open;
        }
        Err(e) => {
            m.content.clear();
            m.lock = if e == OpenError::Forged { Lock::Forged } else { Lock::Damaged };
        }
    }
    None
}

/// Opens what a frame from the server carries, and takes the keys it brings; the keys still
/// missing (conversation, key). Also notes when who can read where might have changed.
pub(crate) fn on_frame(shared: &Shared, server: &str, ev: &mut Event) -> Vec<(ChannelId, KeyRef)> {
    let mut missing = Vec::new();
    // (Before the book's lock: the state's is always taken first.)
    let roles_changed = match ev {
        Event::MemberUpserted { member } => {
            let st = shared.state.read_recursive();
            !st.server(server).and_then(|s| s.members.get(&member.user_id)).is_some_and(|m| m.role == member.role && m.roles == member.roles)
        }
        _ => false,
    };
    let mut book = shared.sealed.lock();
    match ev {
        Event::MessageCreated { message, .. } | Event::MessageUpdated { message } => {
            if let Some(k) = open_message(&mut book, server, message) {
                missing.push((message.channel_id, k));
            }
        }
        Event::History { messages, .. } => {
            for m in messages.iter_mut() {
                if let Some(k) = open_message(&mut book, server, m) {
                    missing.push((m.channel_id, k));
                }
            }
        }
        Event::KeyShares { channel_id, shares } => book.take_shares(server, *channel_id, shares),
        // (Coming and going doesn't change who reads where; joining and roles do.)
        Event::MemberUpserted { .. } if roles_changed => book.forget_readers(server),
        Event::MemberRemoved { .. }
        | Event::ChannelUpserted { .. }
        | Event::ChannelDeleted { .. }
        | Event::RoleDeleted { .. }
        | Event::GroupUpserted { .. }
        | Event::GroupRemoved { .. }
        | Event::DmOpened { .. } => book.forget_readers(server),
        _ => {}
    }
    missing.sort();
    missing.dedup();
    missing
}

/// A new connection to a server: publish my message key there (if it encrypts), and find out
/// again who reads where.
pub(crate) fn start(shared: &Shared, server: &str) {
    let on = shared.state.read_recursive().server(server).is_some_and(|s| s.has(SEALED_FEATURE));
    let key = {
        let mut book = shared.sealed.lock();
        book.forget_readers(server);
        book.mine.publication(&shared.identity)
    };
    if on {
        if let Some(h) = shared.servers.lock().get(server).cloned() {
            h.notify(Request::PublishMessageKey { key });
        }
    }
}

/// Opens again what's waiting in a conversation (after keys came).
pub(crate) fn reopen(shared: &Shared, server: &str, channel: ChannelId) {
    {
        let mut st = shared.state.write();
        let mut book = shared.sealed.lock();
        if let Some(log) = st.server_mut(server).and_then(|s| s.logs.get_mut(&channel)) {
            for m in log.messages.iter_mut().filter(|m| m.sealed.is_some() && m.lock != Lock::Open) {
                open_message(&mut book, server, m);
            }
        }
    }
    (shared.repaint)();
}

async fn ask(h: &ServerHandle, req: Request) -> Result<Event, String> {
    match h.request(req).await {
        Ok(Event::Error { message, .. }) => Err(message),
        Ok(ev) => Ok(ev),
        Err(_) => Err("The connection to the server dropped.".into()),
    }
}

/// Asks the server for keys I don't have (it asks whoever has them, if nobody's given them to
/// me yet), then opens what was waiting for them.
pub(crate) async fn fetch_keys(shared: &Arc<Shared>, server: &str, h: &ServerHandle, channel: ChannelId, refs: Vec<KeyRef>) {
    let refs: Vec<KeyRef> = {
        let mut book = shared.sealed.lock();
        let now = Instant::now();
        book.asked.retain(|_, t| now.duration_since(*t) < ASK_AGAIN);
        let mut out = Vec::new();
        for r in refs {
            let id = (server.to_string(), channel, r.clone());
            if book.key(server, channel, &r).is_none() && !book.asked.contains_key(&id) {
                book.asked.insert(id, now);
                out.push(r);
            }
        }
        out
    };
    if refs.is_empty() {
        return;
    }
    for chunk in refs.chunks(MAX_KEYS_ASKED) {
        // (The reply's keys are taken as it comes in: see `on_frame`.)
        let _ = ask(h, Request::GetKeys { channel_id: channel, keys: chunk.to_vec() }).await;
    }
    reopen(shared, server, channel);
}

/// Whether, as far as this Pyr can tell, `uid` can read a conversation. Direct messages and
/// groups: exactly their people. Channels: everyone here, or for a private one, who its
/// roles let in (someone this Pyr hasn't heard of yet in a very big server: as the server says).
fn may_read(shared: &Shared, server: &str, channel: ChannelId, uid: &str) -> bool {
    let me = shared.identity.user_id();
    let st = shared.state.read_recursive();
    let Some(s) = st.server(server) else { return false };
    if let Some(d) = s.dm(channel) {
        return uid == me || d.with == uid;
    }
    if let Some(g) = s.group(channel) {
        return g.members.iter().any(|m| m == uid);
    }
    match s.channel(channel) {
        None => false,
        Some(ch) if !ch.private => true,
        Some(ch) => s.members.get(uid).is_none_or(|m| ch.visible_to(m.role.is_admin(), &m.roles)),
    }
}

/// Someone asked for keys I have: given, if they can read there.
pub(crate) async fn give(shared: Arc<Shared>, server: String, channel: ChannelId, by: MessageKey, refs: Vec<KeyRef>) {
    let Some(their_pub) = by.verified() else { return };
    if !may_read(&shared, &server, channel, &by.user) {
        return;
    }
    let Some(h) = shared.servers.lock().get(&server).cloned() else { return };
    let unknown: Vec<KeyRef> = {
        let book = shared.sealed.lock();
        refs.iter().filter(|r| book.key(&server, channel, r).is_none()).cloned().collect()
    };
    if !unknown.is_empty() {
        fetch_keys(&shared, &server, &h, channel, unknown).await;
    }
    let shares: Vec<NewShare> = {
        let mut book = shared.sealed.lock();
        let now = Instant::now();
        book.given.retain(|_, t| now.duration_since(*t) < GIVE_AGAIN);
        let mut out = Vec::new();
        for r in &refs {
            let id = (server.clone(), channel, by.user.clone(), r.clone());
            let Some(k) = book.key(&server, channel, r) else { continue };
            if book.given.contains_key(&id) {
                continue;
            }
            if let Some(s) = book.mine.wrap(&book.me, &by.user, &their_pub, channel, r, &k) {
                book.given.insert(id, now);
                out.push(s);
            }
        }
        out
    };
    for chunk in shares.chunks(MAX_SHARES) {
        if let Err(e) = ask(&h, Request::ShareKeys { channel_id: channel, shares: chunk.to_vec() }).await {
            tracing::debug!("couldn't give keys: {e}");
        }
    }
}

/// Who can read a conversation, with their message keys (checked), me among them.
async fn readers(shared: &Shared, server: &str, h: &ServerHandle, channel: ChannelId) -> Result<Vec<(UserId, [u8; 32])>, String> {
    if let Some(r) = shared.sealed.lock().readers.get(&(server.to_string(), channel)) {
        return Ok(r.clone());
    }
    let mut out = Vec::new();
    let mut after: Option<UserId> = None;
    loop {
        let Event::Readers { readers, keyless, more, .. } = ask(h, Request::ChannelReaders { channel_id: channel, after: after.clone() }).await? else {
            return Err("The server answered something else.".into());
        };
        let last = readers.iter().map(|r| &r.user).chain(keyless.iter()).max().cloned();
        for r in readers {
            if let Some(k) = r.verified() {
                out.push((r.user, k));
            }
        }
        if !more || last.is_none() {
            break;
        }
        after = last;
    }
    out.retain(|(u, _)| may_read(shared, server, channel, u));
    let mut book = shared.sealed.lock();
    if !out.iter().any(|(u, _)| *u == book.me) {
        out.push((book.me.clone(), book.mine.public()));
    }
    book.readers.insert((server.to_string(), channel), out.clone());
    Ok(out)
}

/// The key I write with in a conversation, given to everyone who can read there now: a new
/// one if anyone it was given to can't anymore.
async fn outbound(shared: &Shared, server: &str, h: &ServerHandle, channel: ChannelId) -> Result<(KeyRef, [u8; 32]), String> {
    let readers = readers(shared, server, h, channel).await?;
    let now: HashSet<UserId> = readers.iter().map(|(u, _)| u.clone()).collect();
    let (key_ref, key, shares) = {
        let book = shared.sealed.lock();
        let (key_ref, key, given) = match book.outbound.get(&(server.to_string(), channel)) {
            Some(o) if o.given.is_subset(&now) => (o.key_ref.clone(), o.key, o.given.clone()),
            _ => (KeyRef { owner: book.me.clone(), id: sealed::new_key_id() }, pyr_proto::crypto::random_bytes::<32>(), HashSet::new()),
        };
        let shares: Vec<NewShare> =
            readers.iter().filter(|(u, _)| !given.contains(u)).filter_map(|(u, k)| book.mine.wrap(&book.me, u, k, channel, &key_ref, &key)).collect();
        (key_ref, key, shares)
    };
    for chunk in shares.chunks(MAX_SHARES) {
        ask(h, Request::ShareKeys { channel_id: channel, shares: chunk.to_vec() }).await?;
    }
    let mut book = shared.sealed.lock();
    book.add_key(server, channel, &key_ref, key);
    let o = book.outbound.entry((server.to_string(), channel)).or_insert_with(|| Outbound { key_ref: key_ref.clone(), key, given: HashSet::new() });
    if o.key_ref != key_ref {
        *o = Outbound { key_ref: key_ref.clone(), key, given: HashSet::new() };
    }
    o.given.extend(shares.into_iter().map(|s| s.to));
    Ok((key_ref, key))
}

/// Whether a server encrypts messages.
pub fn on_for(shared: &Shared, server: &str) -> bool {
    shared.state.read_recursive().server(server).is_some_and(|s| s.has(SEALED_FEATURE))
}

/// Sends a message (with pictures: file name, bytes), encrypted where the server encrypts.
/// `progress` hears "Uploading 1 of 2…" and such.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn send(
    shared: &Arc<Shared>,
    server: &str,
    h: &ServerHandle,
    channel: ChannelId,
    text: String,
    reply_to: Option<MessageId>,
    nonce: String,
    files: Vec<(String, Arc<Vec<u8>>)>,
    progress: &(dyn Fn(Option<String>) + Send + Sync),
) -> Result<(), String> {
    let on = on_for(shared, server);
    // Pictures only. (The server can't look inside an encrypted one, so it's checked here, the
    // way the server checks one that isn't; the app checks before it gets this far.)
    if files.iter().any(|(_, bytes)| pyr_proto::control::sniff_image(bytes).is_none()) {
        return Err("Only PNG, JPEG, GIF and WebP images can be sent.".into());
    }
    let conn = if files.is_empty() { None } else { Some(h.connection().ok_or("not connected")?) };
    let key = if on { Some(outbound(shared, server, h, channel).await?) } else { None };
    let mut ids = Vec::new();
    let mut body = Body { text: text.clone(), files: Vec::new() };
    for (i, (name, bytes)) in files.iter().enumerate() {
        progress(Some(format!("Uploading {} of {}…", i + 1, files.len())));
        let conn = conn.as_ref().expect("connected for files");
        let mime = pyr_proto::control::sniff_image(bytes).unwrap_or(SEALED_MIME);
        let id = if on {
            let (fkey, ct) = sealed::seal_file(bytes);
            let id = net::file_put(conn, FileKind::Sealed, "", SEALED_MIME, &ct).await.map_err(|e| format!("Couldn't upload {name}: {e:#}"))?;
            shared.sealed.lock().files.insert(id.clone(), fkey);
            let (width, height) = pyr_proto::control::image_size(bytes).unwrap_or((0, 0));
            body.files.push(SealedFile { id: id.clone(), name: name.clone(), mime: mime.to_string(), size: bytes.len() as u64, width, height, key: b64::encode(&fkey) });
            id
        } else {
            net::file_put(conn, FileKind::Attachment, name, mime, bytes).await.map_err(|e| format!("Couldn't upload {name}: {e:#}"))?
        };
        // We have the bytes already: no need to download our own upload.
        shared.files.lock().insert(id.clone(), bytes.clone());
        ids.push(id);
    }
    progress(None);
    let req = match key {
        Some((key_ref, key)) => {
            let s = sealed::seal(&shared.identity, channel, &key_ref, &key, &body);
            Request::SendMessage { channel_id: channel, content: String::new(), reply_to, nonce, attachments: ids, sealed: Some(s) }
        }
        None => Request::SendMessage { channel_id: channel, content: text, reply_to, nonce, attachments: ids, sealed: None },
    };
    ask(h, req).await.map(|_| ())
}

/// Changes a message of mine, encrypted where the server encrypts (its pictures stay).
pub(crate) async fn edit(shared: &Arc<Shared>, server: &str, h: &ServerHandle, message_id: MessageId, text: String) -> Result<(), String> {
    if !on_for(shared, server) {
        return ask(h, Request::EditMessage { message_id, content: text, sealed: None }).await.map(|_| ());
    }
    let found = {
        let st = shared.state.read_recursive();
        st.server(server).and_then(|s| s.logs.values().find_map(|l| l.find(message_id).cloned()))
    };
    let Some(m) = found else { return Err("That message isn't loaded here.".into()) };
    if m.sealed.is_some() && m.lock != Lock::Open {
        return Err("That message hasn't been opened here, so it can't be changed.".into());
    }
    let (key_ref, key) = outbound(shared, server, h, m.channel_id).await?;
    let files = {
        let book = shared.sealed.lock();
        m.attachments
            .iter()
            .map(|a| SealedFile {
                id: a.id.clone(),
                name: a.name.clone(),
                mime: a.mime.clone(),
                size: a.size,
                width: a.width,
                height: a.height,
                // (Empty: a picture from before messages were encrypted.)
                key: book.file_key(&a.id).map(|k| b64::encode(&k)).unwrap_or_default(),
            })
            .collect()
    };
    let s = sealed::seal(&shared.identity, m.channel_id, &key_ref, &key, &Body { text, files });
    ask(h, Request::EditMessage { message_id, content: String::new(), sealed: Some(s) }).await.map(|_| ())
}

/// A page of history with everything opened that can be (waiting for keys first, for up to
/// `wait`): for the Claude connector, which reads without a window.
pub(crate) async fn history(
    shared: &Arc<Shared>,
    server: &str,
    h: &ServerHandle,
    channel: ChannelId,
    before: Option<MessageId>,
    limit: u32,
    wait: Duration,
) -> Result<(Vec<Message>, bool), String> {
    let Event::History { mut messages, complete, .. } = ask(h, Request::FetchHistory { channel_id: channel, before, limit }).await? else {
        return Err("The server answered something else.".into());
    };
    let missing: Vec<KeyRef> = messages.iter().filter(|m| m.lock == Lock::Waiting).filter_map(|m| m.sealed.as_ref().map(|s| s.key.clone())).collect();
    if !missing.is_empty() {
        // Asked for (unless that's under way already), then given as the Pyrs that have them
        // answer: until they're all here, or it's been `wait`.
        let deadline = Instant::now() + wait;
        loop {
            fetch_keys(shared, server, h, channel, missing.clone()).await;
            let all = {
                let book = shared.sealed.lock();
                missing.iter().all(|r| book.key(server, channel, r).is_some())
            };
            if all || Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        let mut book = shared.sealed.lock();
        for m in messages.iter_mut().filter(|m| m.lock == Lock::Waiting) {
            open_message(&mut book, server, m);
        }
    }
    Ok((messages, complete))
}

/// A downloaded picture, opened if it's an encrypted one, and checked to be a picture Pyr
/// can show.
pub(crate) fn open_download(shared: &Shared, id: &str, bytes: Vec<u8>) -> Result<Vec<u8>, String> {
    let Some(key) = shared.sealed.lock().file_key(id) else { return Ok(bytes) };
    let plain = sealed::open_file(&key, &bytes).ok_or("That picture doesn't open: it was changed on the way.")?;
    if pyr_proto::control::sniff_image(&plain).is_none() || !pyr_proto::control::image_dimensions_ok(&plain) {
        return Err("That isn't a picture Pyr shows.".into());
    }
    Ok(plain)
}
