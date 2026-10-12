//! Ingle client engine: everything except the UI.
//!
//! The UI owns an [`Engine`], reads [`AppState`] every frame, and calls the command
//! methods. All networking runs on a tokio runtime; audio and video run on their own
//! threads.

pub mod audio;
pub mod files;
pub mod call;
pub mod config;
pub mod e2ee;
pub mod hotkey;
pub mod net;
pub mod server;
pub mod sfx;
pub mod state;
pub mod video;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::Result;
use ingle_proto::control::*;
use ingle_proto::crypto::{self, Identity};
use ingle_proto::invite::InviteLink;
use ingle_proto::media::track;
use parking_lot::{Mutex, RwLock, RwLockReadGuard};
use tracing::warn;

use crate::audio::{AudioEngine, AudioInput, AudioOutput};
use crate::call::{Call, CallEvent};
use crate::config::{Config, ServerEntry, Settings};
use crate::server::ServerHandle;
use crate::state::*;
use crate::video::{PublisherConfig, VideoStore};

/// Builds audio I/O for calls. Tests and bots use synthetic sources/sinks.
pub type AudioIoFactory = Box<dyn Fn(&Settings) -> (AudioInput, AudioOutput) + Send + Sync>;

/// Bitrate of screen-share sound (stereo Opus). Discord's free tier tops out at 96 kbps.
pub const STREAM_AUDIO_KBPS: u32 = 128;

/// What sound goes with a screen share of `source_id`, if we can capture it here.
pub fn stream_audio_source(source_id: &str) -> Option<audio::stream::StreamAudioSource> {
    use audio::stream::StreamAudioSource;
    if source_id == video::capture::TEST_PATTERN_ID {
        return Some(StreamAudioSource::Synthetic);
    }
    #[cfg(windows)]
    {
        if source_id.starts_with("mon:") {
            return Some(StreamAudioSource::SystemExceptSelf);
        }
        if source_id.starts_with("win:") {
            return video::capture::window_process_id(source_id).map(StreamAudioSource::Process);
        }
    }
    None
}

/// Whether a screen share of `source_id` can include sound on this system.
pub fn can_share_sound(source_id: &str) -> bool {
    stream_audio_source(source_id).is_some()
}

#[derive(Default)]
pub struct CallCounters {
    pub audio_sent: AtomicU64,
    pub undecryptable: AtomicU64,
    pub video_sent: AtomicU64,
    pub video_bytes: AtomicU64,
    pub video_received: AtomicU64,
}

impl CallCounters {
    fn reset(&self) {
        for c in [&self.audio_sent, &self.undecryptable, &self.video_sent, &self.video_bytes, &self.video_received] {
            c.store(0, Ordering::Relaxed);
        }
    }
}

pub struct Shared {
    pub state: RwLock<AppState>,
    pub video: Arc<VideoStore>,
    pub repaint: Arc<dyn Fn() + Send + Sync>,
    pub identity: Identity,
    pub config_dir: PathBuf,
    pub config: Mutex<Config>,
    pub servers: Mutex<HashMap<String, ServerHandle>>,
    pub call: Mutex<Option<Call>>,
    pub call_events: Mutex<Option<(String, tokio::sync::mpsc::UnboundedSender<CallEvent>)>>,
    pub call_counters: CallCounters,
    pub endpoints: std::sync::Mutex<net::Endpoints>,
    pub rt: tokio::runtime::Handle,
    /// (server, channel) the user is looking at — messages there aren't "unread".
    pub active_view: Mutex<Option<(String, ChannelId)>>,
    pub voice_prefs: Mutex<(bool, bool)>,
    pub ptt_down: Arc<AtomicBool>,
    audio_factory: Option<AudioIoFactory>,
    mic_test: Mutex<Option<AudioEngine>>,
    last_video_bytes: Mutex<(std::time::Instant, u64)>,
    /// Images downloaded from servers (attachments and profile pictures), by file id. Kept
    /// in memory only, so nothing deleted on a server lingers on this PC.
    pub files: Mutex<files::FileCache>,
    sfx: Option<sfx::Sfx>,
    /// The app's window is in front (messages in the open channel then make no sound).
    pub window_focused: AtomicBool,
    /// A direct message conversation just opened with `open_dm`, for the UI to show.
    pub opened_dm: Mutex<Option<(String, ChannelId)>>,
}

impl Shared {
    /// Plays a notification sound, if they're on.
    pub fn play(&self, sound: sfx::Sound) {
        let Some(sfx) = &self.sfx else { return };
        let (on, device, volume) = {
            let st = self.state.read();
            (st.settings.sounds, st.settings.output_device.clone(), st.settings.sound_volume)
        };
        if on {
            sfx.play(sound, device, volume);
        }
    }

    pub fn save_config_locked(&self, cfg: &Config) {
        if let Err(e) = config::save_config(&self.config_dir, cfg) {
            warn!("couldn't save settings: {e:#}");
        }
    }

    pub fn audio_io(&self) -> (AudioInput, AudioOutput) {
        let settings = self.state.read().settings.clone();
        match &self.audio_factory {
            Some(f) => f(&settings),
            None => call::default_audio_io(&settings),
        }
    }

    pub fn forward_call_event(&self, key: &str, ev: CallEvent) {
        if let Some((k, tx)) = self.call_events.lock().as_ref() {
            if k == key {
                let _ = tx.send(ev);
            }
        }
    }

    pub fn server_disconnected(&self, key: &str) {
        self.forward_call_event(key, CallEvent::Disconnected);
    }

    /// Tear down the call without telling the server (it already knows, or is gone).
    pub fn end_call_local(&self, reason: &str) {
        let call = self.call.lock().take();
        if let Some(c) = call {
            c.shutdown(false);
            self.video.clear();
            let mut st = self.state.write();
            st.call = None;
            st.toast(reason.to_string(), false);
        }
        *self.call_events.lock() = None;
        (self.repaint)();
    }

    /// Copy live call info (speaking, stats, E2EE status) into the UI state.
    pub fn refresh_call_view(&self) {
        let snapshot = {
            let guard = self.call.lock();
            let Some(c) = guard.as_ref() else { return };
            let (speaking, transmitting, mic_db, jitter, audio_err) = match &c.audio {
                Some(a) => {
                    let st = a.status.lock().clone();
                    let mix = a.mixer.lock().stats();
                    (
                        st.speaking,
                        a.controls.transmitting.load(Ordering::Relaxed),
                        a.controls.mic_level_db(),
                        Some(mix),
                        st.input_error.or(st.output_error),
                    )
                }
                None => (Vec::new(), false, -100.0, None, Some("audio not running".into())),
            };
            (
                c.session,
                speaking,
                transmitting,
                mic_db,
                jitter,
                audio_err,
                c.call_code(),
                c.e2ee_ready(),
                c.watching(),
                c.video_on(track::CAMERA),
                c.video_on(track::SCREEN),
                c.stream_audio_on(),
                c.conn_rtt(),
            )
        };
        let (session, speaking, transmitting, mic_db, jitter, audio_err, code, ready, watching, cam, scr, scr_audio, rtt) = snapshot;
        let now = std::time::Instant::now();
        let video_bytes = self.call_counters.video_bytes.load(Ordering::Relaxed);
        let kbps = {
            let mut l = self.last_video_bytes.lock();
            let dt = now.duration_since(l.0).as_secs_f64();
            if dt >= 1.0 {
                let k = ((video_bytes.saturating_sub(l.1)) as f64 * 8.0 / 1000.0 / dt) as u32;
                *l = (now, video_bytes);
                Some(k)
            } else {
                None
            }
        };
        let mut st = self.state.write();
        st.mic_level_db = mic_db;
        if let Some(cv) = st.call.as_mut() {
            let mut sp: std::collections::HashSet<SessionId> = speaking.into_iter().collect();
            if transmitting && mic_db > -55.0 {
                sp.insert(session);
            }
            cv.speaking = sp;
            cv.transmitting = transmitting;
            cv.call_code = code;
            cv.e2ee_ready = ready;
            cv.watching = watching;
            cv.camera_on = cam;
            cv.screen_on = scr;
            cv.stream_audio_on = scr_audio;
            cv.audio_error = audio_err;
            cv.stats.rtt_ms = rtt;
            cv.stats.audio_sent = self.call_counters.audio_sent.load(Ordering::Relaxed);
            cv.stats.undecryptable = self.call_counters.undecryptable.load(Ordering::Relaxed);
            cv.stats.video_frames_sent = self.call_counters.video_sent.load(Ordering::Relaxed);
            cv.stats.video_frames_received = self.call_counters.video_received.load(Ordering::Relaxed);
            if let Some(k) = kbps {
                cv.stats.video_send_kbps = k;
            }
            if let Some(j) = jitter {
                cv.stats.audio_received = j.received;
                cv.stats.audio_lost = j.lost;
                cv.stats.audio_concealed = j.concealed;
                cv.stats.audio_fec_recovered = j.fec_recovered;
                cv.stats.jitter_ms = j.jitter_ms;
                cv.stats.buffer_ms = j.target_ms;
            }
        }
        drop(st);
        (self.repaint)();
    }
}

pub struct EngineOptions {
    /// Use a separate identity/config (lets you run two copies on one PC).
    pub profile: Option<String>,
    /// Override the config directory entirely.
    pub config_dir: Option<PathBuf>,
    pub repaint: Arc<dyn Fn() + Send + Sync>,
    /// Audio I/O factory for tests/bots; `None` uses the sound card.
    pub audio: Option<AudioIoFactory>,
    /// Watch the global push-to-talk key.
    pub push_to_talk: bool,
}

impl Default for EngineOptions {
    fn default() -> Self {
        EngineOptions { profile: None, config_dir: None, repaint: Arc::new(|| {}), audio: None, push_to_talk: true }
    }
}

pub struct Engine {
    pub shared: Arc<Shared>,
    _rt: Option<tokio::runtime::Runtime>,
    _ptt: Option<hotkey::PttWatcher>,
}

impl Engine {
    pub fn start(opts: EngineOptions) -> Result<Engine> {
        let config_dir = opts.config_dir.clone().unwrap_or_else(|| config::config_dir(opts.profile.as_deref()));
        std::fs::create_dir_all(&config_dir)?;
        let identity = config::load_or_create_identity(&config_dir)?;
        let mut cfg = config::load_config(&config_dir);
        if cfg.display_name.is_empty() {
            cfg.display_name = std::env::var("USERNAME")
                .or_else(|_| std::env::var("USER"))
                .ok()
                .and_then(|n| clean_name(&n, 32))
                .unwrap_or_else(|| "Friend".into());
        }
        let (rt, handle) = match tokio::runtime::Handle::try_current() {
            Ok(h) => (None, h),
            Err(_) => {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .thread_name("ingle-net")
                    .enable_all()
                    .build()?;
                let h = rt.handle().clone();
                (Some(rt), h)
            }
        };
        let my_uid = identity.user_id();
        let state = AppState {
            safety_code: crypto::safety_code(&my_uid),
            my_uid,
            my_name: cfg.display_name.clone(),
            servers: cfg
                .servers
                .iter()
                .map(|e| ServerView::new(e.key.clone(), e.name.clone(), format!("{}:{}", e.host, e.port)))
                .collect(),
            call: None,
            toasts: Vec::new(),
            devices: DeviceLists::default(),
            settings: cfg.settings.clone(),
            user_volumes: cfg.user_volumes.clone(),
            stream_volumes: cfg.stream_volumes.clone(),
            mic_level_db: -100.0,
            mic_test: false,
            muted_servers: cfg.muted_servers.clone(),
            muted_channels: cfg.muted_channels.iter().flat_map(|(k, cs)| cs.iter().map(move |c| (k.clone(), *c))).collect(),
        };
        // Tests and bots (which bring their own audio) stay quiet.
        let sfx = if opts.audio.is_none() { Some(sfx::Sfx::start()) } else { None };
        let ptt_down = Arc::new(AtomicBool::new(false));
        let ptt_key = cfg.settings.ptt_key.clone();
        let entries = cfg.servers.clone();
        let shared = Arc::new(Shared {
            state: RwLock::new(state),
            video: Arc::new(VideoStore::new(opts.repaint.clone())),
            repaint: opts.repaint.clone(),
            identity,
            config_dir,
            config: Mutex::new(cfg),
            servers: Mutex::new(HashMap::new()),
            call: Mutex::new(None),
            call_events: Mutex::new(None),
            call_counters: CallCounters::default(),
            endpoints: std::sync::Mutex::new(net::Endpoints::default()),
            rt: handle,
            active_view: Mutex::new(None),
            opened_dm: Mutex::new(None),
            voice_prefs: Mutex::new((false, false)),
            ptt_down: ptt_down.clone(),
            audio_factory: opts.audio,
            mic_test: Mutex::new(None),
            last_video_bytes: Mutex::new((std::time::Instant::now(), 0)),
            files: Mutex::new(files::FileCache::default()),
            sfx,
            window_focused: AtomicBool::new(true),
        });
        for e in entries {
            let h = server::spawn(shared.clone(), e.clone());
            shared.servers.lock().insert(e.key.clone(), h);
        }
        let ptt = if opts.push_to_talk {
            Some(hotkey::PttWatcher::start(ptt_key, ptt_down, opts.repaint.clone()))
        } else {
            None
        };
        let engine = Engine { shared, _rt: rt, _ptt: ptt };
        Ok(engine)
    }

    pub fn state(&self) -> RwLockReadGuard<'_, AppState> {
        self.shared.state.read()
    }

    pub fn video_frame(&self, key: VideoKey) -> Option<SharedFrame> {
        self.shared.video.get(key)
    }

    fn toast(&self, text: impl Into<String>, error: bool) {
        self.shared.state.write().toast(text, error);
        (self.shared.repaint)();
    }

    fn handle(&self, key: &str) -> Option<ServerHandle> {
        self.shared.servers.lock().get(key).cloned()
    }

    /// Send a request; show its error (if any) as a toast. `then` runs on success.
    fn request_then(&self, key: &str, req: Request, then: impl FnOnce(&Arc<Shared>, Event) + Send + 'static) {
        let Some(h) = self.handle(key) else { return };
        let shared = self.shared.clone();
        self.shared.rt.spawn(async move {
            match h.request(req).await {
                Ok(Event::Error { message, .. }) => {
                    shared.state.write().toast(message, true);
                    (shared.repaint)();
                }
                Ok(ev) => then(&shared, ev),
                Err(_) => {}
            }
        });
    }

    fn request(&self, key: &str, req: Request) {
        self.request_then(key, req, |_, _| {});
    }

    // ------------------------------------------------------------- servers

    /// A short link only has the start of the server's fingerprint: use the full one of a
    /// server already joined at that address, or else ask the server (and check it matches).
    fn full_fingerprint(&self, link: &InviteLink) -> Result<[u8; 32]> {
        let known = self.shared.config.lock().servers.iter().find_map(|s| {
            let fp = ingle_proto::b64::decode_array::<32>(&s.key)?;
            (s.host == link.host && s.port == link.port && link.matches(&fp)).then_some(fp)
        });
        if let Some(fp) = known {
            return Ok(fp);
        }
        let (shared, link) = (self.shared.clone(), link.clone());
        // On its own thread so this works from inside or outside the runtime.
        std::thread::spawn(move || shared.rt.block_on(net::learn_fingerprint(&shared.endpoints, &link)))
            .join()
            .map_err(|_| anyhow::anyhow!("joining failed"))?
    }

    /// Join a server from an invite link.
    pub fn add_server(&self, invite: &str) -> Result<String> {
        let mut link = InviteLink::parse(invite)?;
        if !link.is_full() {
            link.fingerprint = self.full_fingerprint(&link)?;
            link.fp_len = 32;
        }
        let key = ingle_proto::b64::encode(&link.fingerprint);
        // Already connected (or connecting) to this server at this address: nothing to redo,
        // and reconnecting would drop a call in progress.
        let same_address = self
            .shared
            .config
            .lock()
            .servers
            .iter()
            .any(|s| s.key == key && s.host == link.host && s.port == link.port);
        let live = self.shared.servers.lock().contains_key(&key)
            && self.shared.state.read().server(&key).map(|s| !matches!(s.status, ConnStatus::Failed(_))).unwrap_or(false);
        if same_address && live {
            return Ok(key);
        }
        let entry = ServerEntry {
            key: key.clone(),
            host: link.host.clone(),
            port: link.port,
            invite: link.code.clone(),
            name: link.host.clone(),
        };
        {
            let mut cfg = self.shared.config.lock();
            match cfg.servers.iter_mut().find(|s| s.key == key) {
                Some(existing) => {
                    existing.host = entry.host.clone();
                    existing.port = entry.port;
                    if entry.invite.is_some() {
                        existing.invite = entry.invite.clone();
                    }
                }
                None => cfg.servers.push(entry.clone()),
            }
            self.shared.save_config_locked(&cfg);
        }
        if let Some(old) = self.shared.servers.lock().remove(&key) {
            old.close();
        }
        {
            let mut st = self.shared.state.write();
            if st.server(&key).is_none() {
                st.servers.push(ServerView::new(key.clone(), link.host.clone(), link.authority()));
            } else if let Some(s) = st.server_mut(&key) {
                s.status = ConnStatus::Connecting;
            }
        }
        let h = server::spawn(self.shared.clone(), entry);
        self.shared.servers.lock().insert(key.clone(), h);
        Ok(key)
    }

    pub fn retry_server(&self, key: &str) {
        let entry = self.shared.config.lock().servers.iter().find(|s| s.key == key).cloned();
        let Some(entry) = entry else { return };
        if let Some(old) = self.shared.servers.lock().remove(key) {
            old.close();
        }
        if let Some(s) = self.shared.state.write().server_mut(key) {
            s.status = ConnStatus::Connecting;
        }
        let h = server::spawn(self.shared.clone(), entry);
        self.shared.servers.lock().insert(key.to_string(), h);
    }

    pub fn remove_server(&self, key: &str) {
        if self.shared.state.read().call.as_ref().map(|c| c.server_key == key).unwrap_or(false) {
            self.leave_voice();
        }
        if let Some(h) = self.shared.servers.lock().remove(key) {
            h.close();
        }
        let mut cfg = self.shared.config.lock();
        cfg.servers.retain(|s| s.key != key);
        cfg.last_read.remove(key);
        self.shared.save_config_locked(&cfg);
        drop(cfg);
        self.shared.state.write().servers.retain(|s| s.key != key);
        (self.shared.repaint)();
    }

    // ---------------------------------------------------------------- text

    pub fn send_message(&self, key: &str, channel_id: ChannelId, content: &str, reply_to: Option<MessageId>) {
        self.send_message_with(key, channel_id, content, reply_to, Vec::new());
    }

    /// A message with images (file name, bytes): they're uploaded first, then sent with it.
    pub fn send_message_with(
        &self,
        key: &str,
        channel_id: ChannelId,
        content: &str,
        reply_to: Option<MessageId>,
        files: Vec<(String, Vec<u8>)>,
    ) {
        let content = content.trim();
        if content.is_empty() && files.is_empty() {
            return;
        }
        let files: Vec<(String, Arc<Vec<u8>>)> = files.into_iter().map(|(n, b)| (n, Arc::new(b))).collect();
        let nonce = ingle_proto::b64::encode(&crypto::random_bytes::<9>());
        {
            let mut st = self.shared.state.write();
            if let Some(s) = st.server_mut(key) {
                s.logs.entry(channel_id).or_default().pending.push(PendingMessage {
                    nonce: nonce.clone(),
                    content: content.to_string(),
                    reply_to,
                    created_at: ingle_proto::now_ms(),
                    failed: None,
                    files: files.clone(),
                    progress: (!files.is_empty()).then(|| "Uploading…".to_string()),
                });
            }
        }
        let Some(h) = self.handle(key) else { return };
        let shared = self.shared.clone();
        let key = key.to_string();
        let content = content.to_string();
        self.shared.rt.spawn(async move {
            let set = |shared: &Arc<Shared>, progress: Option<String>, failed: Option<String>| {
                let mut st = shared.state.write();
                if let Some(s) = st.server_mut(&key) {
                    if let Some(p) = s.logs.entry(channel_id).or_default().pending.iter_mut().find(|p| p.nonce == nonce) {
                        p.progress = progress;
                        if failed.is_some() {
                            p.failed = failed;
                        }
                    }
                }
                drop(st);
                (shared.repaint)();
            };
            let mut ids = Vec::new();
            if !files.is_empty() {
                let Some(conn) = h.connection() else {
                    set(&shared, None, Some("not connected".into()));
                    return;
                };
                for (i, (name, bytes)) in files.iter().enumerate() {
                    set(&shared, Some(format!("Uploading {} of {}…", i + 1, files.len())), None);
                    let mime = ingle_proto::control::sniff_image(bytes).unwrap_or("application/octet-stream");
                    match net::file_put(&conn, FileKind::Attachment, name, mime, bytes).await {
                        Ok(id) => {
                            // We have the bytes already: no need to download our own upload.
                            shared.files.lock().insert(id.clone(), bytes.clone());
                            ids.push(id);
                        }
                        Err(e) => {
                            set(&shared, None, Some(format!("Couldn't upload {name}: {e:#}")));
                            return;
                        }
                    }
                }
                set(&shared, None, None);
            }
            let req = Request::SendMessage { channel_id, content, reply_to, nonce: nonce.clone(), attachments: ids };
            if let Ok(Event::Error { message, .. }) = h.request(req).await {
                set(&shared, None, Some(message));
            }
        });
        (self.shared.repaint)();
    }

    // ---------------------------------------------------------------- files

    /// An image from a server, if it's here yet; starts downloading it otherwise.
    pub fn file(&self, key: &str, id: &str) -> files::FileState {
        let state = self.shared.files.lock().get(id);
        if !matches!(state, files::FileState::Missing) {
            return state;
        }
        let Some(conn) = self.handle(key).and_then(|h| h.connection()) else { return files::FileState::Missing };
        self.shared.files.lock().set_loading(id);
        let shared = self.shared.clone();
        let id = id.to_string();
        self.shared.rt.spawn(async move {
            match net::file_get(&conn, &id).await {
                Ok((_, bytes)) => shared.files.lock().insert(id, Arc::new(bytes)),
                Err(e) => shared.files.lock().set_failed(&id, format!("{e:#}")),
            }
            (shared.repaint)();
        });
        files::FileState::Loading
    }

    // ---------------------------------------------------------------- roles

    pub fn create_role(&self, key: &str, name: &str, color: u32, perms: u64, hoist: bool) {
        self.request(key, Request::CreateRole { name: name.to_string(), color, perms, hoist });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_role(
        &self,
        key: &str,
        role_id: RoleId,
        name: Option<String>,
        color: Option<u32>,
        perms: Option<u64>,
        hoist: Option<bool>,
        shift: i32,
    ) {
        self.request(key, Request::UpdateRole { role_id, name, color, perms, hoist, shift });
    }

    pub fn delete_role(&self, key: &str, role_id: RoleId) {
        self.request(key, Request::DeleteRole { role_id });
    }

    pub fn set_member_roles(&self, key: &str, user_id: &str, roles: Vec<RoleId>) {
        self.request(key, Request::SetMemberRoles { user_id: user_id.to_string(), roles });
    }

    // -------------------------------------------------------------- avatar

    /// Your profile picture (a square PNG), or none. Saved on this PC and sent to every
    /// server you're in.
    pub fn set_avatar(&self, png: Option<Vec<u8>>) {
        let path = self.shared.config_dir.join(AVATAR_FILE);
        match &png {
            Some(b) => {
                if let Err(e) = std::fs::write(&path, b) {
                    self.toast(format!("Couldn't save your picture: {e}"), true);
                    return;
                }
            }
            None => {
                let _ = std::fs::remove_file(&path);
            }
        }
        if let Some(b) = png {
            let id = ingle_proto::control::avatar_id(&self.shared.identity.user_id(), &b);
            self.shared.files.lock().insert(id, Arc::new(b));
        }
        let keys: Vec<String> = self.shared.servers.lock().keys().cloned().collect();
        for k in keys {
            sync_avatar(self.shared.clone(), k);
        }
        (self.shared.repaint)();
    }

    /// Your profile picture as saved on this PC.
    pub fn my_avatar(&self) -> Option<Vec<u8>> {
        std::fs::read(self.shared.config_dir.join(AVATAR_FILE)).ok()
    }

    pub fn discard_pending(&self, key: &str, channel_id: ChannelId, nonce: &str) {
        if let Some(s) = self.shared.state.write().server_mut(key) {
            if let Some(l) = s.logs.get_mut(&channel_id) {
                l.pending.retain(|p| p.nonce != nonce);
            }
        }
    }

    pub fn edit_message(&self, key: &str, message_id: MessageId, content: &str) {
        self.request(key, Request::EditMessage { message_id, content: content.to_string() });
    }

    pub fn delete_message(&self, key: &str, message_id: MessageId) {
        self.request(key, Request::DeleteMessage { message_id });
    }

    pub fn react(&self, key: &str, message_id: MessageId, emoji: &str, add: bool) {
        self.request(key, Request::React { message_id, emoji: emoji.to_string(), add });
    }

    /// Load the newest page (first open) or the next older page.
    pub fn load_history(&self, key: &str, channel_id: ChannelId, older: bool) {
        let before = {
            let mut st = self.shared.state.write();
            let Some(s) = st.server_mut(key) else { return };
            let log = s.logs.entry(channel_id).or_default();
            if log.loading || (older && log.complete) {
                return;
            }
            log.loading = true;
            if older { log.messages.first().map(|m| m.id) } else { None }
        };
        let k = key.to_string();
        self.request_then(key, Request::FetchHistory { channel_id, before, limit: 50 }, move |shared, ev| {
            let mut st = shared.state.write();
            if let Some(s) = st.server_mut(&k) {
                let log = s.logs.entry(channel_id).or_default();
                log.loading = false;
                if let Event::History { messages, complete, .. } = ev {
                    if !older {
                        log.messages.clear();
                        log.loaded = true;
                    }
                    if older || log.messages.is_empty() {
                        log.complete = complete;
                    }
                    for m in messages {
                        log.upsert(m);
                    }
                }
            }
            drop(st);
            (shared.repaint)();
        });
    }

    pub fn play_sound(&self, sound: sfx::Sound) {
        self.shared.play(sound);
    }

    /// Plays a sound with these settings (the settings screen's Test button).
    pub fn play_sound_with(&self, sound: sfx::Sound, settings: &Settings) {
        if let Some(sfx) = &self.shared.sfx {
            sfx.play(sound, settings.output_device.clone(), settings.sound_volume);
        }
    }

    /// Whether the window is in front (see `Shared::window_focused`).
    pub fn set_window_focused(&self, focused: bool) {
        self.shared.window_focused.store(focused, Ordering::Relaxed);
    }

    /// Mutes or unmutes a whole server (`channel` none) or one channel or conversation.
    pub fn set_muted(&self, key: &str, channel: Option<ChannelId>, muted: bool) {
        {
            let mut cfg = self.shared.config.lock();
            match channel {
                None if muted => {
                    cfg.muted_servers.insert(key.to_string());
                }
                None => {
                    cfg.muted_servers.remove(key);
                }
                Some(c) if muted => {
                    cfg.muted_channels.entry(key.to_string()).or_default().insert(c);
                }
                Some(c) => {
                    if let Some(set) = cfg.muted_channels.get_mut(key) {
                        set.remove(&c);
                    }
                }
            }
            self.shared.save_config_locked(&cfg);
        }
        let mut st = self.shared.state.write();
        match channel {
            None if muted => {
                st.muted_servers.insert(key.to_string());
            }
            None => {
                st.muted_servers.remove(key);
            }
            Some(c) if muted => {
                st.muted_channels.insert((key.to_string(), c));
            }
            Some(c) => {
                st.muted_channels.remove(&(key.to_string(), c));
            }
        }
    }

    /// Opens (starting it if needed) the direct message conversation with a member of this
    /// server; `take_opened_dm` then says which to show.
    pub fn open_dm(&self, key: &str, user_id: &str) {
        let known = self.shared.state.read().server(key).and_then(|s| s.dm_with(user_id)).map(|d| d.id);
        if let Some(id) = known {
            *self.shared.opened_dm.lock() = Some((key.to_string(), id));
            (self.shared.repaint)();
            return;
        }
        let k = key.to_string();
        self.request_then(key, Request::OpenDm { user_id: user_id.to_string() }, move |shared, ev| {
            if let Event::DmOpened { dm } = ev {
                let id = dm.id;
                if let Some(s) = shared.state.write().server_mut(&k) {
                    s.upsert_dm(dm);
                }
                *shared.opened_dm.lock() = Some((k, id));
                (shared.repaint)();
            }
        });
    }

    pub fn take_opened_dm(&self) -> Option<(String, ChannelId)> {
        self.shared.opened_dm.lock().take()
    }

    /// Nothing in particular is being looked at (the friends list, settings...).
    pub fn clear_active_channel(&self) {
        *self.shared.active_view.lock() = None;
    }

    /// Tell the engine what the user is looking at (clears unread markers there).
    pub fn set_active_channel(&self, key: &str, channel_id: ChannelId) {
        *self.shared.active_view.lock() = Some((key.to_string(), channel_id));
        let newest = {
            let mut st = self.shared.state.write();
            let Some(s) = st.server_mut(key) else { return };
            s.unread.remove(&channel_id);
            s.mentions.remove(&channel_id);
            s.logs.get(&channel_id).and_then(|l| l.messages.last().map(|m| m.id))
        };
        if let Some(id) = newest {
            let mut cfg = self.shared.config.lock();
            let entry = cfg.last_read.entry(key.to_string()).or_default();
            if entry.get(&channel_id) != Some(&id) {
                entry.insert(channel_id, id);
            }
        }
    }

    pub fn typing(&self, key: &str, channel_id: ChannelId) {
        if let Some(h) = self.handle(key) {
            h.notify(Request::Typing { channel_id });
        }
    }

    // --------------------------------------------------------------- admin

    pub fn create_channel(&self, key: &str, name: &str, kind: ChannelKind) {
        self.request(key, Request::CreateChannel { name: name.to_string(), kind });
    }

    pub fn update_channel(&self, key: &str, channel_id: ChannelId, name: Option<String>, topic: Option<String>) {
        self.request(key, Request::UpdateChannel { channel_id, name, topic });
    }

    pub fn delete_channel(&self, key: &str, channel_id: ChannelId) {
        self.request(key, Request::DeleteChannel { channel_id });
    }

    pub fn create_invite(&self, key: &str, max_uses: Option<u32>, expires_secs: Option<u64>) {
        let k = key.to_string();
        self.request_then(key, Request::CreateInvite { max_uses, expires_secs }, move |shared, ev| {
            if let Event::InviteCreated { link, .. } = ev {
                if let Some(s) = shared.state.write().server_mut(&k) {
                    // Servers before 0.3.0 hand out the long form.
                    s.last_invite = Some(InviteLink::shorten(&link));
                }
                (shared.repaint)();
            }
        });
    }

    pub fn kick(&self, key: &str, user_id: &str, ban: bool) {
        let req = if ban { Request::Ban { user_id: user_id.into() } } else { Request::Kick { user_id: user_id.into() } };
        self.request(key, req);
    }

    pub fn set_role(&self, key: &str, user_id: &str, role: Role) {
        self.request(key, Request::SetRole { user_id: user_id.into(), role });
    }

    pub fn update_server(&self, key: &str, name: Option<String>, description: Option<String>) {
        self.request(key, Request::UpdateServer { name, description });
    }

    pub fn set_display_name(&self, name: &str) {
        let Some(name) = clean_name(name, 32) else { return };
        {
            let mut cfg = self.shared.config.lock();
            cfg.display_name = name.clone();
            self.shared.save_config_locked(&cfg);
        }
        self.shared.state.write().my_name = name.clone();
        let keys: Vec<String> = self.shared.servers.lock().keys().cloned().collect();
        for k in keys {
            self.request(&k, Request::UpdateProfile { name: name.clone() });
        }
    }

    // --------------------------------------------------------------- voice

    pub fn join_voice(&self, key: &str, channel_id: ChannelId) {
        let in_same = self
            .shared
            .state
            .read()
            .call
            .as_ref()
            .map(|c| c.server_key == key && c.channel_id == channel_id)
            .unwrap_or(false);
        if in_same {
            return;
        }
        self.leave_voice();
        self.stop_mic_test();
        let shared = self.shared.clone();
        let key = key.to_string();
        self.shared.rt.spawn(async move {
            if let Err(e) = Call::join(shared.clone(), key, channel_id).await {
                shared.state.write().toast(format!("Couldn't join voice: {e:#}"), true);
                if shared.call.lock().is_none() {
                    *shared.call_events.lock() = None;
                }
            }
            (shared.repaint)();
        });
    }

    pub fn leave_voice(&self) {
        let call = self.shared.call.lock().take();
        if let Some(c) = call {
            c.shutdown(true);
        }
        *self.shared.call_events.lock() = None;
        self.shared.video.clear();
        self.shared.state.write().call = None;
        (self.shared.repaint)();
    }

    pub fn set_mute(&self, muted: bool) {
        let deaf = {
            let mut p = self.shared.voice_prefs.lock();
            p.0 = muted;
            if !muted {
                p.1 = false;
            }
            p.1
        };
        self.apply_voice_prefs(muted, deaf);
    }

    pub fn set_deafen(&self, deafened: bool) {
        let muted = {
            let mut p = self.shared.voice_prefs.lock();
            p.1 = deafened;
            p.0
        };
        self.apply_voice_prefs(muted, deafened);
    }

    pub fn voice_prefs(&self) -> (bool, bool) {
        *self.shared.voice_prefs.lock()
    }

    fn apply_voice_prefs(&self, muted: bool, deafened: bool) {
        if let Some(c) = self.shared.call.lock().as_mut() {
            c.set_mute(muted, deafened);
        }
        if let Some(cv) = self.shared.state.write().call.as_mut() {
            cv.muted = muted || deafened;
            cv.deafened = deafened;
        }
        (self.shared.repaint)();
    }

    pub fn start_camera(&self) {
        let settings = self.shared.state.read().settings.clone();
        let device = settings.camera_device.clone().or_else(|| {
            let cams = video::list_cameras();
            cams.first().map(|c| c.0.clone())
        });
        let Some(device) = device else {
            self.toast("No camera found.", true);
            return;
        };
        let (w, h, fps, bitrate) = settings.camera_quality.params();
        let cfg = PublisherConfig { screen: false, max_width: w, max_height: h, fps, bitrate };
        self.start_video(track::CAMERA, &device, cfg);
    }

    pub fn start_screen(&self, source_id: &str) {
        let preset = self.shared.state.read().settings.screen_preset;
        let (max_h, fps, bitrate) = preset.params();
        let cfg = PublisherConfig { screen: true, max_width: max_h * 16 / 9 + 8, max_height: max_h, fps, bitrate };
        self.start_video(track::SCREEN, source_id, cfg);
    }

    fn start_video(&self, track_id: u8, source: &str, cfg: PublisherConfig) {
        let shared = self.shared.clone();
        let source = source.to_string();
        // Opening a camera can block for a while; keep the UI responsive.
        self.shared.rt.spawn_blocking(move || {
            let result = {
                let mut guard = shared.call.lock();
                match guard.as_mut() {
                    Some(c) => {
                        c.stop_video(&shared, track_id);
                        c.start_video(&shared, track_id, &source, cfg)
                    }
                    None => Err(anyhow::anyhow!("join a voice channel first")),
                }
            };
            if let Err(e) = result {
                let what = if track_id == track::SCREEN { "share your screen" } else { "start your camera" };
                shared.state.write().toast(format!("Couldn't {what}: {e:#}"), true);
            } else if track_id == track::SCREEN && shared.state.read().settings.share_audio {
                // Sound for the stream. Started without holding the call lock, since the
                // system can take a moment to set up capture.
                if let Some(src) = stream_audio_source(&source) {
                    let sink = shared.call.lock().as_ref().map(|c| c.stream_audio_sink(&shared));
                    if let Some(sink) = sink {
                        match audio::stream::StreamAudio::start(src, STREAM_AUDIO_KBPS, sink) {
                            Ok(sa) => {
                                let leftover = shared.call.lock().as_mut().map(|c| c.attach_stream_audio(sa));
                                drop(leftover);
                            }
                            Err(e) => {
                                tracing::warn!("stream audio unavailable: {e:#}");
                                shared.state.write().toast(format!("Sharing without sound: {e:#}"), true);
                            }
                        }
                    }
                }
            }
            shared.refresh_call_view();
        });
    }

    pub fn stop_camera(&self) {
        if let Some(c) = self.shared.call.lock().as_mut() {
            c.stop_video(&self.shared, track::CAMERA);
        }
        self.shared.refresh_call_view();
    }

    pub fn stop_screen(&self) {
        if let Some(c) = self.shared.call.lock().as_mut() {
            c.stop_video(&self.shared, track::SCREEN);
        }
        self.shared.refresh_call_view();
    }

    pub fn watch(&self, session: SessionId, track_id: u8, on: bool) {
        if let Some(c) = self.shared.call.lock().as_mut() {
            c.watch(&self.shared, VideoKey { session, track: track_id }, on);
        }
        self.shared.refresh_call_view();
    }

    /// Volume of someone's screen-share sound, separate from their voice.
    pub fn set_stream_volume(&self, user_id: &str, volume: f32) {
        let volume = volume.clamp(0.0, 2.0);
        {
            let mut cfg = self.shared.config.lock();
            cfg.stream_volumes.insert(user_id.to_string(), volume);
            self.shared.save_config_locked(&cfg);
        }
        self.shared.state.write().stream_volumes.insert(user_id.to_string(), volume);
        let sessions: Vec<SessionId> = {
            let st = self.shared.state.read();
            st.call
                .as_ref()
                .and_then(|c| st.server(&c.server_key))
                .map(|s| s.voice.values().filter(|p| p.user_id == user_id).map(|p| p.session).collect())
                .unwrap_or_default()
        };
        if let Some(c) = self.shared.call.lock().as_ref() {
            if let Some(a) = &c.audio {
                for s in sessions {
                    a.mixer.lock().set_stream_volume(s, volume);
                }
            }
        }
    }

    pub fn set_user_volume(&self, user_id: &str, volume: f32) {
        let volume = volume.clamp(0.0, 2.0);
        {
            let mut cfg = self.shared.config.lock();
            cfg.user_volumes.insert(user_id.to_string(), volume);
            self.shared.save_config_locked(&cfg);
        }
        self.shared.state.write().user_volumes.insert(user_id.to_string(), volume);
        let sessions: Vec<SessionId> = {
            let st = self.shared.state.read();
            st.call
                .as_ref()
                .and_then(|c| st.server(&c.server_key))
                .map(|s| s.voice.values().filter(|p| p.user_id == user_id).map(|p| p.session).collect())
                .unwrap_or_default()
        };
        if let Some(c) = self.shared.call.lock().as_ref() {
            if let Some(a) = &c.audio {
                for s in sessions {
                    a.mixer.lock().set_volume(s, volume);
                }
            }
        }
    }

    // ------------------------------------------------------------ settings

    pub fn update_settings(&self, settings: Settings) {
        let old = self.shared.state.read().settings.clone();
        if old == settings {
            return;
        }
        {
            let mut cfg = self.shared.config.lock();
            cfg.settings = settings.clone();
            self.shared.save_config_locked(&cfg);
        }
        self.shared.state.write().settings = settings.clone();
        // Restart audio so device/processing changes take effect.
        let audio_changed = old.input_device != settings.input_device
            || old.output_device != settings.output_device
            || old.noise_suppression != settings.noise_suppression
            || old.echo_cancellation != settings.echo_cancellation
            || old.auto_gain != settings.auto_gain
            || old.voice_bitrate_kbps != settings.voice_bitrate_kbps
            || old.input_mode != settings.input_mode
            || old.vad_auto != settings.vad_auto
            || (old.vad_threshold_db - settings.vad_threshold_db).abs() > 0.01
            || (old.input_gain - settings.input_gain).abs() > 0.001
            || old.ptt_key != settings.ptt_key
            || old.ptt_release_ms != settings.ptt_release_ms;
        if let Some(p) = self._ptt.as_ref() {
            p.set_key(&settings.ptt_key);
        }
        if audio_changed {
            let (m, d) = *self.shared.voice_prefs.lock();
            if let Some(c) = self.shared.call.lock().as_mut() {
                c.start_audio(&self.shared, m, d);
            }
            if self.shared.mic_test.lock().is_some() {
                self.stop_mic_test();
                self.start_mic_test();
            }
        } else if (old.output_volume - settings.output_volume).abs() > 0.001 {
            if let Some(c) = self.shared.call.lock().as_ref() {
                if let Some(a) = &c.audio {
                    a.mixer.lock().master = settings.output_volume;
                }
            }
        }
        (self.shared.repaint)();
    }

    /// Hear yourself (with all processing applied) to check your microphone.
    pub fn start_mic_test(&self) {
        self.shared.state.write().mic_test = true;
        if let Some(c) = self.shared.call.lock().as_ref() {
            if let Some(a) = &c.audio {
                a.controls.monitor.store(true, Ordering::Relaxed);
            }
            return;
        }
        let settings = self.shared.state.read().settings.clone();
        let (input, output) = self.shared.audio_io();
        let engine = AudioEngine::start(settings, input, output, Box::new(|_, _, _| {}), self.shared.ptt_down.clone());
        engine.controls.monitor.store(true, Ordering::Relaxed);
        let controls = engine.controls.clone();
        *self.shared.mic_test.lock() = Some(engine);
        let shared = self.shared.clone();
        self.shared.rt.spawn(async move {
            while shared.mic_test.lock().is_some() {
                shared.state.write().mic_level_db = controls.mic_level_db();
                (shared.repaint)();
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        });
    }

    pub fn stop_mic_test(&self) {
        self.shared.state.write().mic_test = false;
        if let Some(c) = self.shared.call.lock().as_ref() {
            if let Some(a) = &c.audio {
                a.controls.monitor.store(false, Ordering::Relaxed);
            }
        }
        if let Some(mut e) = self.shared.mic_test.lock().take() {
            e.stop();
        }
    }

    /// Re-scan microphones, speakers, cameras and screens (in the background).
    pub fn refresh_devices(&self) {
        let shared = self.shared.clone();
        self.shared.rt.spawn_blocking(move || {
            let (inputs, outputs) = audio::device::list_devices();
            let cameras = video::list_cameras();
            let screens = video::list_screens();
            shared.state.write().devices = DeviceLists { inputs, outputs, cameras, screens };
            (shared.repaint)();
        });
    }

    pub fn identity_path(&self) -> PathBuf {
        self.shared.config_dir.join("identity.key")
    }

    /// Save config (e.g. last-read markers) before quitting.
    pub fn shutdown(&self) {
        self.leave_voice();
        self.stop_mic_test();
        let cfg = self.shared.config.lock().clone();
        self.shared.save_config_locked(&cfg);
        for h in self.shared.servers.lock().values() {
            h.close();
        }
    }
}

const AVATAR_FILE: &str = "avatar.png";

/// Make a server's copy of my profile picture match the one saved on this PC (after
/// connecting, and when it changes). Content-named ids mean nothing is re-sent needlessly.
pub(crate) fn sync_avatar(shared: Arc<Shared>, key: String) {
    let rt = shared.rt.clone();
    rt.spawn(async move {
        let (has_feature, current) = {
            let st = shared.state.read();
            match st.server(&key) {
                Some(s) => (s.has(ingle_proto::control::AVATARS_FEATURE), s.me.as_ref().and_then(|m| m.avatar.clone())),
                None => return,
            }
        };
        if !has_feature {
            return;
        }
        let Some(h) = shared.servers.lock().get(&key).cloned() else { return };
        let local = std::fs::read(shared.config_dir.join(AVATAR_FILE)).ok();
        let wanted = local.as_ref().map(|b| ingle_proto::control::avatar_id(&shared.identity.user_id(), b));
        if wanted == current {
            return;
        }
        let file_id = match local {
            Some(bytes) => {
                let Some(conn) = h.connection() else { return };
                let mime = ingle_proto::control::sniff_image(&bytes).unwrap_or("image/png");
                match net::file_put(&conn, FileKind::Avatar, "avatar.png", mime, &bytes).await {
                    Ok(id) => Some(id),
                    Err(e) => {
                        tracing::warn!("couldn't upload the profile picture: {e:#}");
                        return;
                    }
                }
            }
            None => None,
        };
        if let Ok(Event::Error { message, .. }) = h.request(Request::SetAvatar { file_id }).await {
            tracing::warn!("couldn't set the profile picture: {message}");
        }
    });
}
