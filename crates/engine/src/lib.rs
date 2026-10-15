//! Ingle client engine: everything except the UI.
//!
//! The UI owns an [`Engine`], reads [`AppState`] every frame, and calls the command
//! methods. All networking runs on a tokio runtime; audio and video run on their own
//! threads.

pub mod audio;
pub mod backup;
pub mod bugs;
pub mod files;
pub mod call;
pub mod config;
pub mod e2ee;
pub mod hotkey;
pub mod qos;

/// Problems found where there's no app state at hand (saving settings, a background upload):
/// shown as error notices by the app (`take_errors`).
pub mod errors {
    use std::sync::Mutex;
    static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());

    pub fn report(text: impl Into<String>) {
        let mut p = PENDING.lock().unwrap_or_else(|e| e.into_inner());
        if p.len() < 20 {
            p.push(text.into());
        }
    }

    pub fn take() -> Vec<String> {
        std::mem::take(&mut *PENDING.lock().unwrap_or_else(|e| e.into_inner()))
    }
}
pub mod net;
pub mod plugins;
pub mod server;
pub mod sfx;
pub mod soundboard;
pub mod state;
pub mod video;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use anyhow::{Result, anyhow};
use ingle_proto::control::*;
use ingle_proto::crypto::{self, Identity};
use ingle_proto::invite::InviteLink;
use ingle_proto::media::track;
use parking_lot::{Mutex, RwLock, RwLockReadGuard};
use tracing::{debug, warn};

use crate::audio::{AudioEngine, AudioInput, AudioOutput};
use crate::call::{Call, CallEvent};
use crate::config::{Config, ServerEntry, Settings};
use crate::soundboard::SoundRef;
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
    /// What calls play, for taking it back out of our screen share's sound.
    pub played: Arc<audio::PlayedTap>,
    audio_factory: Option<AudioIoFactory>,
    mic_test: Mutex<Option<AudioEngine>>,
    /// The screen share picker's live preview: (source, capture, its frames).
    screen_preview: Mutex<Option<(String, Option<video::CaptureHandle>, Arc<video::FrameSlot>)>>,
    last_video_bytes: Mutex<(std::time::Instant, u64)>,
    /// Images downloaded from servers (attachments and profile pictures), by file id. Kept
    /// in memory only, so nothing deleted on a server lingers on this PC.
    pub files: Mutex<files::FileCache>,
    sfx: Option<sfx::Sfx>,
    /// The app's window is in front (messages in the open channel then make no sound and
    /// count as read). Set from the app's frames; see `focus_probe` for when there are none.
    pub window_focused: AtomicBool,
    /// Asks the system directly whether the window is in front (`None`: it can't tell). The app
    /// only runs frames while its window is showing, so after it's hidden to the tray or
    /// minimized `window_focused` still says what the last frame saw.
    pub focus_probe: Mutex<Option<Box<dyn Fn() -> Option<bool> + Send + Sync>>>,
    /// A direct message conversation just opened with `open_dm`, for the UI to show.
    pub opened_dm: Mutex<Option<(String, ChannelId)>>,
    /// What our screen share is showing (to share it again after a reconnect).
    pub screen_source: Mutex<Option<String>>,
    /// Counts starts and stops of our [camera, screen share]: a start that's still opening when
    /// a newer start or a stop comes doesn't go live.
    video_gen: [AtomicU64; 2],
    /// Read marks changed that aren't saved yet (see `mark_read`).
    read_unsaved: AtomicBool,
    /// Soundboard sounds fetched and decoded so far.
    pub sounds: Mutex<soundboard::Cache>,
    /// Sound files known to be on a server: (server key, file id).
    sounds_on: Mutex<std::collections::HashSet<(String, String)>>,
}

/// What sound files are, to the file streams.
const SOUND_MIME: &str = "audio/x-ingle-sound";

/// Play `sound` in the call on server `key` (see `Engine::play_sound`).
/// Plays `sound` for everyone in our voice channel (and notes it in the recent sounds).
fn play_soundboard_on(shared: &Arc<Shared>, sound: SoundRef) {
    let Some(key) = shared.call.lock().as_ref().map(|c| c.server_key.clone()) else {
        shared.state.write().toast("Join a voice channel to play sounds.", true);
        (shared.repaint)();
        return;
    };
    {
        let mut cfg = shared.config.lock();
        let recent = &mut cfg.settings.sound_recent;
        recent.retain(|s| !s.same(&sound));
        recent.insert(0, sound.clone());
        recent.truncate(12);
        let recent = recent.clone();
        shared.save_config_locked(&cfg);
        drop(cfg);
        shared.state.write().settings.sound_recent = recent;
    }
    let s2 = shared.clone();
    shared.rt.spawn(async move {
        if let Err(e) = play_sound_in_call(&s2, &key, &sound).await {
            s2.state.write().toast(format!("{e:#}"), true);
            (s2.repaint)();
        }
    });
}

/// Plays samples to us only: through the call when we're in one (in step with everything
/// else), otherwise on the notification sounds' own stream.
fn preview_pcm_on(shared: &Arc<Shared>, pcm: soundboard::Pcm, volume: f32) {
    let (volume, device) = {
        let st = shared.state.read_recursive();
        (volume.clamp(0.0, 1.0) * st.settings.soundboard_volume.clamp(0.0, 1.0), st.settings.output_device.clone())
    };
    let call = shared.call.lock().as_ref().and_then(|c| c.audio.as_ref().map(|a| (a.mixer.clone(), c.session)));
    match (call, &shared.sfx) {
        (Some((mixer, me)), _) => mixer.lock().play_effect(pcm, volume, me),
        (None, Some(sfx)) => sfx.play_stereo(pcm, device, volume),
        (None, None) => {}
    }
}

async fn play_sound_in_call(shared: &Arc<Shared>, key: &str, sound: &SoundRef) -> Result<()> {
    let h = shared.servers.lock().get(key).cloned().ok_or_else(|| anyhow!("not connected to that server"))?;
    for attempt in 0..2 {
        let wire = if sound.is_builtin() || sound.server == key {
            sound.wire_id()
        } else {
            // From another server: sent to this one first. Sound files are named by their
            // content, so one this server has already isn't sent again.
            let file = shared.sound_file(&sound.server, &sound.id).await?;
            let id = ingle_proto::sound::sound_id(&file);
            let here = (key.to_string(), id.clone());
            if !shared.sounds_on.lock().contains(&here) {
                let conn = h.connection().ok_or_else(|| anyhow!("not connected to that server"))?;
                let got = net::file_put(&conn, FileKind::Sound, &sound.name, SOUND_MIME, &file).await?;
                shared.sounds_on.lock().insert((key.to_string(), got));
            }
            id
        };
        let req = Request::PlaySound { sound: wire.clone(), name: sound.name.clone(), emoji: sound.emoji.clone(), volume: sound.volume };
        match h.request(req).await.map_err(|_| anyhow!("connection lost"))? {
            // The server let go of a sound sent earlier: send it again.
            Event::Error { code: ErrorCode::NotFound, .. } if attempt == 0 && !sound.is_builtin() && sound.server != key => {
                shared.sounds_on.lock().remove(&(key.to_string(), wire));
            }
            Event::Error { message, .. } => anyhow::bail!(message),
            _ => return Ok(()),
        }
    }
    Ok(())
}

/// How often changed read marks for channels are saved.
const READ_MARKS_SAVED_EVERY: std::time::Duration = std::time::Duration::from_secs(15);

/// Drop something that can take a moment to let go (a capture, decoders) on a thread of its
/// own, so neither the UI nor whoever holds a lock waits for it.
pub(crate) fn drop_later<T: Send + 'static>(x: T) {
    let _ = std::thread::Builder::new().name("ingle-teardown".into()).spawn(move || drop(x));
}

impl Shared {
    /// Whether you can see Ingle right now: its window is in front, not hidden or minimized.
    pub fn window_in_front(&self) -> bool {
        if let Some(seen) = self.focus_probe.lock().as_ref().and_then(|p| p()) {
            return seen;
        }
        self.window_focused.load(Ordering::Relaxed)
    }

    /// Whether you're looking at this conversation right now (open, with the window in front).
    pub fn looking_at(&self, key: &str, channel_id: ChannelId) -> bool {
        let open = self.active_view.lock().as_ref().map(|(k, c)| k == key && *c == channel_id).unwrap_or(false);
        open && self.window_in_front()
    }

    /// Remember that you've read `channel_id` up to `id` (what comes after is unread after a
    /// restart). With `save` it's saved at once (direct messages); otherwise within
    /// `READ_MARKS_SAVED_EVERY`.
    pub(crate) fn mark_read(&self, key: &str, channel_id: ChannelId, id: MessageId, save: bool) {
        let mut cfg = self.config.lock();
        let entry = cfg.last_read.entry(key.to_string()).or_default();
        if entry.get(&channel_id).map(|seen| *seen < id).unwrap_or(true) {
            entry.insert(channel_id, id);
            if save {
                self.save_config_locked(&cfg);
            } else {
                self.read_unsaved.store(true, Ordering::Relaxed);
            }
        }
    }

    /// A channel just got unread messages: if this PC has no read mark for it yet, everything
    /// before `id` counts as read (so the unread ones still show after a restart, instead of
    /// the channel starting out read then).
    pub(crate) fn mark_read_before(&self, key: &str, channel_id: ChannelId, id: MessageId) {
        let mut cfg = self.config.lock();
        let entry = cfg.last_read.entry(key.to_string()).or_default();
        if !entry.contains_key(&channel_id) {
            entry.insert(channel_id, id - 1);
            self.read_unsaved.store(true, Ordering::Relaxed);
        }
    }

    /// A sound file: built in, already here, or fetched from its server (and decoded, so it's
    /// ready to play).
    pub(crate) async fn sound_file(self: &Arc<Self>, server: &str, id: &str) -> Result<Arc<Vec<u8>>> {
        // One look at the cache (the lock is held to the end of an `if let`, so locking it again
        // inside would wait on itself forever: built-in sounds always have a file).
        let cached = {
            let cache = self.sounds.lock();
            cache.pcm(server, id).and_then(|_| cache.file(server, id))
        };
        if let Some(f) = cached {
            return Ok(f);
        }
        let bytes = match soundboard::builtin_file(id).filter(|_| server.is_empty()) {
            Some(b) => b.to_vec(),
            None => {
                let conn = self
                    .servers
                    .lock()
                    .get(server)
                    .and_then(|h| h.connection())
                    .ok_or_else(|| anyhow!("not connected to that sound's server"))?;
                let (_, b) = net::file_get(&conn, id).await?;
                b
            }
        };
        let (bytes, pcm) = tokio::task::spawn_blocking(move || {
            let pcm = soundboard::decode(&bytes)?;
            anyhow::Ok((bytes, pcm))
        })
        .await??;
        let file = Arc::new(bytes);
        self.sounds.lock().insert(server, id, file.clone(), Arc::new(pcm));
        if !server.is_empty() {
            self.sounds_on.lock().insert((server.to_string(), id.to_string()));
        }
        Ok(file)
    }

    /// A sound's samples (fetching it first if need be).
    pub(crate) async fn sound_pcm(self: &Arc<Self>, server: &str, id: &str) -> Result<soundboard::Pcm> {
        if let Some(p) = self.sounds.lock().pcm(server, id) {
            return Ok(p);
        }
        self.sound_file(server, id).await?;
        self.sounds.lock().pcm(server, id).ok_or_else(|| anyhow!("that sound couldn't be read"))
    }

    /// Someone in a voice channel on server `key` played a sound: play it if that's our call
    /// (and they're not someone whose sounds we've muted), and show who played what.
    pub(crate) fn sound_played(self: &Arc<Self>, key: &str, session: SessionId, channel_id: ChannelId, sound: String, name: String, emoji: String, volume: f32) {
        let ours = self.call.lock().as_ref().is_some_and(|c| c.server_key == key && c.channel_id == channel_id);
        if !ours {
            return;
        }
        let (muted, gain) = {
            let mut st = self.state.write();
            let uid = st.server(key).and_then(|s| s.voice.get(&session)).map(|p| p.user_id.clone()).unwrap_or_default();
            let muted = st.settings.soundboard_muted.contains(&uid);
            let gain = volume.clamp(0.0, 1.0) * st.settings.soundboard_volume.clamp(0.0, 1.0);
            if let Some(cv) = st.call.as_mut() {
                let now = std::time::Instant::now();
                cv.sounds.retain(|s| now.duration_since(s.at) < std::time::Duration::from_secs(4));
                cv.sounds.push(state::SoundShown { session, name, emoji, at: now });
            }
            (muted, gain)
        };
        (self.repaint)();
        if muted || gain <= 0.0 {
            return;
        }
        let (server, id) = match sound.strip_prefix(ingle_proto::sound::BUILTIN_PREFIX) {
            Some(b) => (String::new(), b.to_string()),
            None => (key.to_string(), sound),
        };
        let shared = self.clone();
        self.rt.spawn(async move {
            match shared.sound_pcm(&server, &id).await {
                Ok(pcm) => {
                    if let Some(a) = shared.call.lock().as_ref().and_then(|c| c.audio.as_ref()) {
                        a.mixer.lock().play_effect(pcm, gain, session);
                    }
                }
                Err(e) => debug!("couldn't play a soundboard sound ({id}): {e:#}"),
            }
        });
    }

    /// Plays a notification sound, if they're on.
    pub fn play(&self, sound: sfx::Sound) {
        let Some(sfx) = &self.sfx else { return };
        let (on, device, volume) = {
            let st = self.state.read_recursive();
            (st.settings.sounds, st.settings.output_device.clone(), st.settings.sound_volume)
        };
        if on {
            sfx.play(sound, device, volume);
        }
    }

    pub fn save_config_locked(&self, cfg: &Config) {
        if let Err(e) = config::save_config(&self.config_dir, cfg) {
            warn!("couldn't save settings: {e:#}");
            errors::report(format!("Couldn't save your settings: {e:#}"));
        }
    }

    /// Mute and deafen as they are now (in `voice_prefs`): saved, so a restart (an update,
    /// say) brings you back the same way, and put into effect in the call.
    pub(crate) fn apply_voice_prefs(&self) {
        let (muted, deafened) = *self.voice_prefs.lock();
        {
            let mut cfg = self.config.lock();
            let now = Some(config::SelfVoice { muted, deafened });
            if cfg.self_voice != now {
                cfg.self_voice = now;
                self.save_config_locked(&cfg);
            }
        }
        if let Some(c) = self.call.lock().as_mut() {
            c.set_mute(muted, deafened);
        }
        if let Some(cv) = self.state.write().call.as_mut() {
            cv.muted = muted || deafened;
            cv.deafened = deafened;
        }
        (self.repaint)();
    }

    pub fn audio_io(&self) -> (AudioInput, AudioOutput) {
        let settings = self.state.read_recursive().settings.clone();
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

    /// The server connection dropped during a call: end it here quietly and remember it, so
    /// it's rejoined when the server is back (see `resume_voice`).
    pub fn call_lost(&self) {
        qos::set_video(false);
        let call = self.call.lock().take();
        if let Some(c) = call {
            c.shutdown(false);
            self.video.clear();
            let screen = self.screen_source.lock().clone();
            let mut st = self.state.write();
            if let Some(cv) = st.call.take() {
                st.voice_rejoin = Some(state::VoiceRejoin {
                    server_key: cv.server_key,
                    channel_id: cv.channel_id,
                    camera: cv.camera_on,
                    screen: screen.filter(|_| cv.screen_on),
                    since: std::time::Instant::now(),
                });
            }
        }
        *self.call_events.lock() = None;
        (self.repaint)();
    }

    /// Tear down the call without telling the server (it already knows, or is gone).
    pub fn end_call_local(&self, reason: &str) {
        let call = self.call.lock().take();
        if let Some(c) = call {
            c.shutdown(false);
            self.video.clear();
            let mut st = self.state.write();
            st.call = None;
            st.resume_offer = None;
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
            qos::set_video(cam || scr);
            cv.stream_audio_on = scr_audio;
            // A microphone or speaker problem: say so (once), not only on the call screen.
            if let Some(e) = audio_err.as_ref().filter(|e| cv.audio_error.as_ref() != Some(*e) && e.as_str() != "audio not running") {
                errors::report(format!("Sound problem in the call: {e}"));
            }
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
    /// Keys that play soundboard sounds.
    _keys: Option<hotkey::ComboWatcher>,
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
            voice_rejoin: None,
            resume_offer: None,
            my_name: cfg.display_name.clone(),
            servers: cfg
                .servers
                .iter()
                .map(|e| ServerView::new(e.key.clone(), e.name.clone(), format!("{}:{}", e.host, e.port)))
                .collect(),
            call: None,
            toasts: Vec::new(),
            bug_send: Default::default(),
            bug_sends_done: 0,
            backups: HashMap::new(),
            bugs: HashMap::new(),
            plugin_send: Default::default(),
            plugin_sends_done: 0,
            submissions: HashMap::new(),
            submission_files: HashMap::new(),
            devices: DeviceLists::default(),
            settings: cfg.settings.clone(),
            user_volumes: cfg.user_volumes.clone(),
            stream_volumes: cfg.stream_volumes.clone(),
            mic_level_db: -100.0,
            mic_test: false,
            muted_servers: cfg.muted_servers.clone(),
            blocked: cfg.blocked.clone(),
            muted_channels: cfg.muted_channels.iter().flat_map(|(k, cs)| cs.iter().map(move |c| (k.clone(), *c))).collect(),
        };
        // Tests and bots (which bring their own audio) stay quiet.
        let sfx = if opts.audio.is_none() { Some(sfx::Sfx::start()) } else { None };
        let ptt_down = Arc::new(AtomicBool::new(false));
        let ptt_key = cfg.settings.ptt_key.clone();
        qos::set_enabled(cfg.settings.voice_priority);
        if cfg.settings.noise_suppression == config::NoiseSuppression::Isolation {
            // Unpacked in the background now, so joining voice doesn't wait for it.
            audio::dfn::preload();
        }
        let entries = cfg.servers.clone();
        let voice_prefs = cfg.self_voice.map(|v| (v.muted, v.deafened)).unwrap_or_default();
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
            screen_source: Mutex::new(None),
            voice_prefs: Mutex::new(voice_prefs),
            ptt_down: ptt_down.clone(),
            played: Arc::default(),
            audio_factory: opts.audio,
            mic_test: Mutex::new(None),
            screen_preview: Mutex::new(None),
            last_video_bytes: Mutex::new((std::time::Instant::now(), 0)),
            files: Mutex::new(files::FileCache::default()),
            sfx,
            window_focused: AtomicBool::new(true),
            focus_probe: Mutex::new(None),
            video_gen: [AtomicU64::new(0), AtomicU64::new(0)],
            read_unsaved: AtomicBool::new(false),
            sounds: Mutex::new(soundboard::Cache::default()),
            sounds_on: Mutex::new(Default::default()),
        });
        // Read marks for channels are saved a few at a time (reading a busy channel moves them
        // with every message), so what you've read stays read after a restart or a crash.
        {
            let weak = Arc::downgrade(&shared);
            shared.rt.spawn(async move {
                let mut tick = tokio::time::interval(READ_MARKS_SAVED_EVERY);
                loop {
                    tick.tick().await;
                    let Some(shared) = weak.upgrade() else { break };
                    if shared.read_unsaved.swap(false, Ordering::Relaxed) {
                        let cfg = shared.config.lock();
                        shared.save_config_locked(&cfg);
                    }
                }
            });
        }
        for e in entries {
            let h = server::spawn(shared.clone(), e.clone());
            shared.servers.lock().insert(e.key.clone(), h);
        }
        let ptt = if opts.push_to_talk {
            Some(hotkey::PttWatcher::start(ptt_key, ptt_down, opts.repaint.clone()))
        } else {
            None
        };
        let keys = if opts.push_to_talk {
            let combos: Vec<String> = shared.state.read_recursive().settings.sound_keys.iter().map(|(c, _)| c.clone()).collect();
            let s2 = shared.clone();
            Some(hotkey::ComboWatcher::start(
                &combos,
                Arc::new(move |i| {
                    let sound = s2.state.read_recursive().settings.sound_keys.get(i).map(|(_, s)| s.clone());
                    // Only in a call: elsewhere the key is just a key.
                    if let Some(sound) = sound.filter(|_| s2.call.lock().is_some()) {
                        play_soundboard_on(&s2, sound);
                    }
                }),
                opts.repaint.clone(),
            ))
        } else {
            None
        };
        let engine = Engine { shared, _rt: rt, _ptt: ptt, _keys: keys };
        Ok(engine)
    }

    /// The shared state, for the UI to draw a frame from. Hold it only while drawing: engine
    /// methods called meanwhile read it again (never write it), which is why the engine's own
    /// reads are `read_recursive` — a plain read waits behind a waiting writer, and a thread
    /// that already holds a read would then wait for itself (Ingle froze that way when a voice
    /// chime played while voice state was being updated).
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
            && self.shared.state.read_recursive().server(&key).map(|s| !matches!(s.status, ConnStatus::Failed(_))).unwrap_or(false);
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
        if self.shared.state.read_recursive().call.as_ref().map(|c| c.server_key == key).unwrap_or(false) {
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

    /// Sets (or with none, removes) a server's icon: uploads the PNG, then asks the server.
    pub fn set_server_icon(&self, key: &str, png: Option<Vec<u8>>) {
        let shared = self.shared.clone();
        let key = key.to_string();
        self.shared.rt.clone().spawn(async move {
            let Some(h) = shared.servers.lock().get(&key).cloned() else { return };
            let file_id = match png {
                Some(bytes) => {
                    let Some(conn) = h.connection() else {
                        shared.state.write().toast("Not connected to that server right now.", true);
                        return;
                    };
                    match net::file_put(&conn, FileKind::Avatar, "icon.png", "image/png", &bytes).await {
                        Ok(id) => {
                            shared.files.lock().insert(id.clone(), Arc::new(bytes));
                            Some(id)
                        }
                        Err(e) => {
                            shared.state.write().toast(format!("Couldn't upload the icon: {e:#}"), true);
                            (shared.repaint)();
                            return;
                        }
                    }
                }
                None => None,
            };
            if let Ok(Event::Error { message, .. }) = h.request(Request::SetServerIcon { file_id }).await {
                shared.state.write().toast(message, true);
            }
            (shared.repaint)();
        });
    }

    /// Adds one of a server's own emoji: uploads the picture (PNG, or a GIF when animated),
    /// then names it.
    pub fn add_emoji(&self, key: &str, name: &str, bytes: Vec<u8>, animated: bool) {
        let shared = self.shared.clone();
        let (key, name) = (key.to_string(), name.to_string());
        self.shared.rt.clone().spawn(async move {
            let Some(h) = shared.servers.lock().get(&key).cloned() else { return };
            let Some(conn) = h.connection() else {
                shared.state.write().toast("Not connected to that server right now.", true);
                return;
            };
            let mime = if animated { "image/gif" } else { "image/png" };
            let file_id = match net::file_put(&conn, FileKind::Avatar, "emoji", mime, &bytes).await {
                Ok(id) => id,
                Err(e) => {
                    shared.state.write().toast(format!("Couldn't upload the emoji: {e:#}"), true);
                    (shared.repaint)();
                    return;
                }
            };
            shared.files.lock().insert(file_id.clone(), Arc::new(bytes));
            match h.request(Request::AddEmoji { name: name.clone(), file_id, animated }).await {
                Ok(Event::Error { message, .. }) => shared.state.write().toast(message, true),
                Ok(_) => shared.state.write().toast(format!("Added :{name}:"), false),
                Err(_) => {}
            }
            (shared.repaint)();
        });
    }

    pub fn remove_emoji(&self, key: &str, id: &str) {
        self.request(key, Request::RemoveEmoji { id: id.to_string() });
    }

    pub fn rename_emoji(&self, key: &str, id: &str, name: &str) {
        self.request(key, Request::RenameEmoji { id: id.to_string(), name: name.to_string() });
    }

    // ------------------------------------------------------------- soundboard

    /// Play a sound for everyone in your voice channel. Any sound plays in any call: one from
    /// another server (or a built-in one) is sent to the call's server first, if it isn't
    /// there already.
    pub fn play_soundboard(&self, sound: SoundRef) {
        play_soundboard_on(&self.shared, sound);
    }

    /// Record the next key combination pressed, for a soundboard key (`take_recorded_sound_key`
    /// has it once it's pressed); `false` stops recording.
    pub fn record_sound_key(&self, on: bool) {
        if let Some(k) = &self._keys {
            k.record(on);
        }
    }

    pub fn take_recorded_sound_key(&self) -> Option<String> {
        self._keys.as_ref()?.take_recorded()
    }

    /// Whether soundboard keys work here (they need the keyboard's state, like push-to-talk).
    pub fn sound_keys_available(&self) -> bool {
        self._keys.is_some()
    }

    /// Hear sound samples yourself (the add-a-sound dialog trying out its cut).
    pub fn preview_pcm(&self, pcm: soundboard::Pcm, volume: f32) {
        preview_pcm_on(&self.shared, pcm, volume);
    }

    /// Hear a sound yourself, nobody else (trying it out).
    pub fn preview_sound(&self, sound: SoundRef) {
        let shared = self.shared.clone();
        self.shared.rt.spawn(async move {
            let (server, id) = (sound.server.clone(), sound.id.clone());
            let pcm = match shared.sound_pcm(&server, &id).await {
                Ok(p) => p,
                Err(e) => {
                    shared.state.write().toast(format!("Couldn't play {}: {e:#}", sound.name), true);
                    (shared.repaint)();
                    return;
                }
            };
            preview_pcm_on(&shared, pcm, sound.volume);
        });
    }

    /// Stop every soundboard sound playing here (anyone's).
    pub fn stop_sounds(&self) {
        if let Some(a) = self.shared.call.lock().as_ref().and_then(|c| c.audio.as_ref()) {
            a.mixer.lock().stop_effects(None);
        }
        if let Some(sfx) = &self.shared.sfx {
            sfx.hush();
        }
    }

    /// Whether soundboard sounds are playing in the call.
    pub fn sounds_playing(&self) -> bool {
        self.shared.call.lock().as_ref().and_then(|c| c.audio.as_ref()).is_some_and(|a| a.mixer.lock().effects_playing())
    }

    /// Add a sound to a server's soundboard: `file` is a sound file (see `soundboard::encode`).
    pub fn add_sound(&self, key: &str, name: &str, emoji: &str, volume: f32, file: Vec<u8>) {
        let shared = self.shared.clone();
        let (key, name, emoji) = (key.to_string(), name.to_string(), emoji.to_string());
        self.shared.rt.clone().spawn(async move {
            let Some(h) = shared.servers.lock().get(&key).cloned() else { return };
            let Some(conn) = h.connection() else {
                shared.state.write().toast("Not connected to that server right now.", true);
                return;
            };
            let file_id = match net::file_put(&conn, FileKind::Sound, &name, SOUND_MIME, &file).await {
                Ok(id) => id,
                Err(e) => {
                    shared.state.write().toast(format!("Couldn't upload the sound: {e:#}"), true);
                    (shared.repaint)();
                    return;
                }
            };
            if let Ok(pcm) = soundboard::decode(&file) {
                shared.sounds.lock().insert(&key, &file_id, Arc::new(file), Arc::new(pcm));
            }
            match h.request(Request::AddSound { file_id, name: name.clone(), emoji, volume }).await {
                Ok(Event::Error { message, .. }) => shared.state.write().toast(message, true),
                Ok(_) => shared.state.write().toast(format!("Added {name} to the soundboard."), false),
                Err(_) => {}
            }
            (shared.repaint)();
        });
    }

    /// Add a sound from samples picked in the add-a-sound dialog (48 kHz stereo): the part
    /// from `start` to `end` seconds is prepared (faded, evened out, cut to the limit) and
    /// encoded off the UI thread, then uploaded.
    #[allow(clippy::too_many_arguments)]
    pub fn add_sound_from(&self, key: &str, name: &str, emoji: &str, volume: f32, pcm: soundboard::Pcm, start: f32, end: f32) {
        let shared = self.shared.clone();
        let (key, name, emoji) = (key.to_string(), name.to_string(), emoji.to_string());
        let me = Engine { shared: self.shared.clone(), _rt: None, _ptt: None, _keys: None };
        self.shared.rt.spawn(async move {
            let made = tokio::task::spawn_blocking(move || soundboard::encode(&soundboard::prepare(&pcm, start, end))).await;
            match made {
                Ok(Ok(file)) => me.add_sound(&key, &name, &emoji, volume, file),
                Ok(Err(e)) => {
                    shared.state.write().toast(format!("Couldn't make the sound: {e:#}"), true);
                    (shared.repaint)();
                }
                Err(_) => {}
            }
        });
    }

    pub fn edit_sound(&self, key: &str, id: &str, name: Option<String>, emoji: Option<String>, volume: Option<f32>) {
        self.request(key, Request::EditSound { id: id.to_string(), name, emoji, volume });
    }

    pub fn remove_sound(&self, key: &str, id: &str) {
        self.request(key, Request::RemoveSound { id: id.to_string() });
    }

    /// Get a server's sounds ready to play (fetched and decoded in the background), so the
    /// first time one is played it plays at once.
    pub fn prefetch_sounds(&self, key: &str) {
        let ids: Vec<String> = {
            let st = self.shared.state.read_recursive();
            let Some(s) = st.server(key) else { return };
            let cache = self.shared.sounds.lock();
            s.info.sounds.iter().filter(|x| cache.pcm(key, &x.id).is_none()).map(|x| x.id.clone()).collect()
        };
        if ids.is_empty() {
            return;
        }
        let shared = self.shared.clone();
        let key = key.to_string();
        self.shared.rt.spawn(async move {
            for id in ids {
                if let Err(e) = shared.sound_file(&key, &id).await {
                    debug!("couldn't fetch sound {id}: {e:#}");
                }
            }
        });
    }

    /// A sound's samples, if they're here already (for drawing it).
    pub fn sound_pcm(&self, sound: &SoundRef) -> Option<soundboard::Pcm> {
        self.shared.sounds.lock().pcm(&sound.server, &sound.id)
    }

    // ------------------------------------------------------------- friends

    /// Servers (connected, with friends) where we're both members; ones where something is
    /// already recorded between us first.
    fn shared_servers(&self, uid: &str) -> Vec<String> {
        let st = self.shared.state.read_recursive();
        let mut v: Vec<(bool, String)> = st
            .servers
            .iter()
            .filter(|s| s.has(FRIENDS_FEATURE) && s.status.is_connected() && s.members.contains_key(uid))
            .map(|s| (!s.relations.contains_key(uid), s.key.clone()))
            .collect();
        v.sort();
        v.into_iter().map(|x| x.1).collect()
    }

    /// Sends a friend request (or accepts theirs), through a server we share.
    pub fn add_friend(&self, uid: &str) {
        match self.shared_servers(uid).first() {
            Some(k) => self.request(k, Request::FriendRequest { user_id: uid.to_string() }),
            None => self.toast("You need to share a server with them (and be connected) to add them.", true),
        }
    }

    pub fn accept_friend(&self, uid: &str) {
        let keys: Vec<String> = {
            let st = self.shared.state.read_recursive();
            st.servers.iter().filter(|s| s.relations.get(uid) == Some(&RelationKind::Incoming)).map(|s| s.key.clone()).collect()
        };
        for k in keys {
            self.request(&k, Request::FriendAccept { user_id: uid.to_string() });
        }
    }

    /// Declines, cancels or unfriends, everywhere.
    pub fn remove_friend(&self, uid: &str) {
        let keys: Vec<String> = {
            let st = self.shared.state.read_recursive();
            st.servers
                .iter()
                .filter(|s| matches!(s.relations.get(uid), Some(RelationKind::Friend | RelationKind::Incoming | RelationKind::Outgoing)))
                .map(|s| s.key.clone())
                .collect()
        };
        for k in keys {
            self.request(&k, Request::FriendRemove { user_id: uid.to_string() });
        }
    }

    /// Blocks someone on every server we share (and on this PC: their messages fold away).
    pub fn block(&self, uid: &str) {
        self.set_blocked_locally(uid, true);
        for k in self.shared_servers(uid) {
            self.request(&k, Request::Block { user_id: uid.to_string() });
        }
    }

    pub fn unblock(&self, uid: &str) {
        self.set_blocked_locally(uid, false);
        let keys: Vec<String> = {
            let st = self.shared.state.read_recursive();
            st.servers.iter().filter(|s| s.has(FRIENDS_FEATURE) && s.status.is_connected()).map(|s| s.key.clone()).collect()
        };
        for k in keys {
            self.request(&k, Request::Unblock { user_id: uid.to_string() });
        }
    }

    fn set_blocked_locally(&self, uid: &str, on: bool) {
        {
            let mut cfg = self.shared.config.lock();
            if on {
                cfg.blocked.insert(uid.to_string());
            } else {
                cfg.blocked.remove(uid);
            }
            self.shared.save_config_locked(&cfg);
        }
        let mut st = self.shared.state.write();
        if on {
            st.blocked.insert(uid.to_string());
        } else {
            st.blocked.remove(uid);
        }
        drop(st);
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
        if self.handle(key).map(|h| h.connection().is_none()).unwrap_or(true) {
            self.toast("Couldn't react: not connected to that server right now.", true);
            return;
        }
        // Never silently: if the server says no, say why.
        self.request_then(key, Request::React { message_id, emoji: emoji.to_string(), add }, |shared, ev| {
            if let Event::Error { message, .. } = ev {
                shared.state.write().toast(format!("Couldn't react: {message}"), true);
            }
        });
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
            let mut newest = None;
            if let Some(s) = st.server_mut(&k) {
                let dm = s.dm(channel_id).is_some();
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
                    if !older {
                        newest = log.messages.last().map(|m| (m.id, dm));
                    }
                }
            }
            drop(st);
            // Opened before its messages arrived: they're read now that you can see them.
            if let Some((id, dm)) = newest.filter(|_| shared.looking_at(&k, channel_id)) {
                shared.mark_read(&k, channel_id, id, dm);
            }
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

    /// How to ask the system whether the window is in front, for when the app isn't running
    /// frames (see `Shared::focus_probe`).
    pub fn set_focus_probe(&self, probe: impl Fn() -> Option<bool> + Send + Sync + 'static) {
        *self.shared.focus_probe.lock() = Some(Box::new(probe));
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
        let known = self.shared.state.read_recursive().server(key).and_then(|s| s.dm_with(user_id)).map(|d| d.id);
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
        let left = self.shared.active_view.lock().take();
        if let Some((k, c)) = left {
            self.left_view(&k, c);
        }
    }

    /// Tell the engine what the user is looking at. Its unread markers clear once the window is
    /// in front: what comes in while you're in another window stays unread until you're back.
    pub fn set_active_channel(&self, key: &str, channel_id: ChannelId) {
        let here = (key.to_string(), channel_id);
        let left = self.shared.active_view.lock().replace(here.clone());
        if let Some((k, c)) = left.filter(|l| *l != here) {
            self.left_view(&k, c);
        }
        if !self.shared.window_in_front() {
            return;
        }
        let newest = {
            let mut st = self.shared.state.write();
            let Some(s) = st.server_mut(key) else { return };
            s.unread.remove(&channel_id);
            s.mentions.remove(&channel_id);
            s.logs.get(&channel_id).and_then(|l| l.messages.last().map(|m| m.id))
        };
        if let Some(id) = newest {
            // Saved now, so read messages don't come back as unread after a crash or an update
            // restart.
            self.shared.mark_read(key, channel_id, id, true);
        }
    }

    /// You moved on from a conversation: its "NEW" line goes, once what it marked was seen.
    fn left_view(&self, key: &str, channel_id: ChannelId) {
        let mut st = self.shared.state.write();
        if let Some(s) = st.server_mut(key) {
            if !s.unread.contains(&channel_id) && !s.mentions.contains_key(&channel_id) {
                s.new_since.remove(&channel_id);
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

    /// Server mute someone in voice, or unmute them (a moderator, `perm::MUTE_MEMBERS`).
    pub fn server_mute(&self, key: &str, user_id: &str, muted: bool) {
        self.request(key, Request::ServerMute { user_id: user_id.into(), muted });
    }

    /// Server deafen someone in voice, or undeafen them (`perm::DEAFEN_MEMBERS`).
    pub fn server_deafen(&self, key: &str, user_id: &str, deafened: bool) {
        self.request(key, Request::ServerDeafen { user_id: user_id.into(), deafened });
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
            match Call::join(shared.clone(), key.clone(), channel_id).await {
                Err(e) => {
                    shared.state.write().toast(format!("Couldn't join voice: {e:#}"), true);
                    if shared.call.lock().is_none() {
                        *shared.call_events.lock() = None;
                    }
                }
                Ok(()) => {
                    // Your join sound, for everyone there (not when a call comes back after
                    // the server restarted: that isn't joining).
                    let join = shared.state.read_recursive().settings.join_sound.clone();
                    let has = shared.state.read_recursive().server(&key).is_some_and(|s| s.has(SOUNDBOARD_FEATURE));
                    if let Some(sound) = join.filter(|_| has) {
                        // A moment for the others to hear we're here (and get our keys).
                        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                        if let Err(e) = play_sound_in_call(&shared, &key, &sound).await {
                            debug!("join sound didn't play: {e:#}");
                        }
                    }
                }
            }
            (shared.repaint)();
        });
    }

    /// The offer to turn the camera or screen share back on after a drop: done with (`camera`,
    /// `screen`: which part).
    pub fn settle_resume_offer(&self, camera: bool, screen: bool) {
        let mut st = self.shared.state.write();
        if let Some(o) = st.resume_offer.as_mut() {
            o.camera &= !camera;
            o.screen &= !screen;
            if !o.camera && !o.screen {
                st.resume_offer = None;
            }
        }
        drop(st);
        (self.shared.repaint)();
    }

    pub fn leave_voice(&self) {
        qos::set_video(false);
        {
            let mut st = self.shared.state.write();
            st.voice_rejoin = None;
            st.resume_offer = None;
        }
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
        {
            let mut p = self.shared.voice_prefs.lock();
            p.0 = muted;
            if !muted {
                p.1 = false;
            }
        }
        self.shared.apply_voice_prefs();
    }

    pub fn set_deafen(&self, deafened: bool) {
        self.shared.voice_prefs.lock().1 = deafened;
        self.shared.apply_voice_prefs();
    }

    /// Back up a server I own now (see `backup.rs`); progress shows in `AppState::backups`.
    pub fn backup_now(&self, key: &str) {
        let (shared, key) = (self.shared.clone(), key.to_string());
        self.shared.rt.spawn(async move {
            match backup::run(shared.clone(), key).await {
                Ok(i) => shared.state.write().toast(format!("Backed up {} ({} files).", i.name, i.files), false),
                Err(e) => shared.state.write().toast(format!("Couldn't back up the server: {e:#}"), true),
            }
            (shared.repaint)();
        });
    }

    /// Where backups of this server go.
    pub fn backup_folder(&self, key: &str) -> std::path::PathBuf {
        let name = self.shared.state.read_recursive().server(key).map(|s| s.info.name.clone()).unwrap_or_default();
        backup::folder(&self.shared, key, &name)
    }

    /// Send an Ingle bug report (see `bugs.rs`). How it goes shows in `AppState::bug_send`, with
    /// a thank-you notice once it's sent; a report that doesn't go is logged, and the app keeps
    /// its text. `turn_on`: the bug server, when it's one of my servers and I'm its owner:
    /// switch collecting on there first.
    pub fn report_bug(&self, title: &str, body: &str, include_log: bool, turn_on: Option<&str>) {
        let shared = self.shared.clone();
        let report = ingle_proto::control::BugReport {
            title: title.trim().to_string(),
            body: body.trim().to_string(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            os: bugs::os_name(),
            log: if include_log { bugs::log_tail(&shared, ingle_proto::control::BUG_LOG_MAX - 1000) } else { String::new() },
        };
        let mine = turn_on.and_then(|k| self.handle(k));
        shared.state.write().bug_send = BugSend::Sending;
        self.shared.rt.spawn(async move {
            let r = async {
                if let Some(h) = mine {
                    let answer = tokio::time::timeout(std::time::Duration::from_secs(15), h.request(Request::SetBugReports { on: true })).await;
                    match answer {
                        Ok(Ok(Event::Ok)) => {}
                        Ok(Ok(Event::Error { message, .. })) => anyhow::bail!("couldn't switch on collecting: {message}"),
                        _ => anyhow::bail!("your server didn't answer when asked to collect bug reports"),
                    }
                }
                bugs::send(shared.clone(), report).await
            }
            .await;
            let mut st = shared.state.write();
            match r {
                Ok(()) => {
                    st.bug_send = BugSend::Sent;
                    st.toast("Thanks! Your bug report was sent.", false);
                }
                Err(e) => {
                    let message = format!("{e:#}");
                    warn!("bug report not sent: {message}");
                    st.bug_send = BugSend::Failed { not_collecting: bugs::is_not_collecting(&message), message };
                }
            }
            st.bug_sends_done += 1;
            drop(st);
            (shared.repaint)();
        });
    }

    /// Owner only: a server's bug reports, into `AppState::bugs`.
    pub fn list_bugs(&self, key: &str) {
        let k = key.to_string();
        self.request_then(key, Request::ListBugs, move |shared, ev| match ev {
            Event::Bugs { mut reports } => {
                bugs::sort(&mut reports);
                let name = shared.state.read().server(&k).map(|s| s.info.name.clone()).unwrap_or_default();
                bugs::save_approved(shared, &k, &name, &reports);
                shared.state.write().bugs.insert(k, reports);
            }
            Event::Error { message, .. } => shared.state.write().toast(message, true),
            _ => {}
        });
    }

    /// Owner only: approve (for fixing), reject or mark fixed.
    pub fn set_bug_status(&self, key: &str, id: i64, status: ingle_proto::control::BugStatus) {
        let changed = {
            let mut st = self.shared.state.write();
            let name = st.server(key).map(|s| s.info.name.clone()).unwrap_or_default();
            st.bugs.get_mut(key).map(|list| {
                if let Some(b) = list.iter_mut().find(|b| b.id == id) {
                    b.status = status;
                }
                (name, list.clone())
            })
        };
        // What's approved now, for Claude (see `bugs::save_approved`).
        if let Some((name, list)) = changed {
            bugs::save_approved(&self.shared, key, &name, &list);
        }
        self.request_then(key, Request::SetBugStatus { id, status }, move |shared, ev| {
            if let Event::Error { message, .. } = ev {
                shared.state.write().toast(format!("Couldn't change the report: {message}"), true);
            }
        });
    }

    /// Owner only: collect Ingle bug reports on this server, or stop.
    pub fn set_bug_reports(&self, key: &str, on: bool) {
        self.request_then(key, Request::SetBugReports { on }, move |shared, ev| {
            if let Event::Error { message, .. } = ev {
                shared.state.write().toast(message, true);
            }
        });
    }

    /// Send a plugin in for the Ingle Library (see `plugins.rs`). How it goes shows in
    /// `AppState::plugin_send`, like a bug report. `turn_on`: the library server, when it's
    /// mine: switch collecting on there first.
    pub fn submit_plugin(&self, mut submission: ingle_proto::control::PluginSubmission, turn_on: Option<&str>) {
        let shared = self.shared.clone();
        submission.app_version = env!("CARGO_PKG_VERSION").to_string();
        let mine = turn_on.and_then(|k| self.handle(k));
        shared.state.write().plugin_send = BugSend::Sending;
        self.shared.rt.spawn(async move {
            let r = async {
                if let Some(h) = mine {
                    let answer = tokio::time::timeout(std::time::Duration::from_secs(15), h.request(Request::SetPluginSubmissions { on: true })).await;
                    match answer {
                        Ok(Ok(Event::Ok)) => {}
                        Ok(Ok(Event::Error { message, .. })) => anyhow::bail!("couldn't switch on collecting: {message}"),
                        _ => anyhow::bail!("your server didn't answer when asked to collect plugins"),
                    }
                }
                plugins::send(shared.clone(), submission).await
            }
            .await;
            let mut st = shared.state.write();
            match r {
                Ok(()) => {
                    st.plugin_send = BugSend::Sent;
                    st.toast("Sent! Your plugin will be looked at before it's listed.", false);
                }
                Err(e) => {
                    let message = format!("{e:#}");
                    warn!("plugin not sent: {message}");
                    st.plugin_send = BugSend::Failed { not_collecting: plugins::is_not_collecting(&message), message };
                }
            }
            st.plugin_sends_done += 1;
            drop(st);
            (shared.repaint)();
        });
    }

    /// Owner only: the plugins sent in to a server, into `AppState::submissions`.
    pub fn list_submissions(&self, key: &str) {
        let k = key.to_string();
        self.request_then(key, Request::ListPluginSubmissions, move |shared, ev| {
            if let Event::PluginSubmissions { mut list } = ev {
                plugins::sort(&mut list);
                shared.state.write().submissions.insert(k, list);
            }
        });
    }

    /// Owner only: one submission's files, into `AppState::submission_files` (to look at).
    pub fn open_submission(&self, key: &str, id: i64) {
        let k = key.to_string();
        self.request_then(key, Request::GetPluginSubmission { id }, move |shared, ev| {
            if let Event::PluginSubmission { entry, files } = ev {
                shared.state.write().submission_files.insert((k, entry.id), files);
            }
        });
    }

    /// Owner only: approve (its source is saved to this PC for building, see
    /// `plugins::save_approved`), reject, mark listed, or put back.
    pub fn set_submission_status(&self, key: &str, id: i64, status: ingle_proto::control::SubmissionStatus) {
        use ingle_proto::control::SubmissionStatus;
        let k = key.to_string();
        let set = move |shared: &Arc<Shared>, handle: Option<ServerHandle>| {
            if let Some(h) = handle {
                let shared = shared.clone();
                shared.rt.clone().spawn(async move {
                    match tokio::time::timeout(std::time::Duration::from_secs(15), h.request(Request::SetPluginSubmissionStatus { id, status })).await {
                        Ok(Ok(Event::Ok)) => {}
                        Ok(Ok(Event::Error { message, .. })) => shared.state.write().toast(format!("Couldn't change the plugin: {message}"), true),
                        _ => shared.state.write().toast("Couldn't change the plugin: the server didn't answer.", true),
                    }
                    (shared.repaint)();
                });
            }
        };
        {
            let mut st = self.shared.state.write();
            if let Some(e) = st.submissions.get_mut(key).and_then(|l| l.iter_mut().find(|e| e.id == id)) {
                e.status = status;
            }
        }
        if status != SubmissionStatus::Approved {
            set(&self.shared, self.handle(key));
            return;
        }
        // Approved: save its source here first (for building), then say so to the server.
        let handle = self.handle(key);
        self.request_then(key, Request::GetPluginSubmission { id }, move |shared, ev| match ev {
            Event::PluginSubmission { entry, files } => match plugins::save_approved(&shared.config_dir, &k, &entry, &files) {
                Ok(dir) => {
                    tracing::info!("approved plugin saved to {}", dir.display());
                    shared.state.write().toast(format!("Approved. Its source is saved for building in {}", dir.display()), false);
                    set(shared, handle);
                }
                Err(e) => shared.state.write().toast(format!("Couldn't save the plugin's source, so it wasn't approved: {e:#}"), true),
            },
            Event::Error { message, .. } => shared.state.write().toast(format!("Couldn't get the plugin: {message}"), true),
            _ => {}
        });
    }

    /// Owner only: collect plugins for the Ingle Library on this server, or stop.
    pub fn set_plugin_submissions(&self, key: &str, on: bool) {
        self.request_then(key, Request::SetPluginSubmissions { on }, move |shared, ev| {
            if let Event::Error { message, .. } = ev {
                shared.state.write().toast(message, true);
            }
        });
    }

    /// Owner only: the GIPHY key everyone's GIF search on this server uses ("" turns it off).
    pub fn set_giphy_key(&self, key: &str, giphy_key: &str) {
        let on = !giphy_key.trim().is_empty();
        self.request_then(key, Request::SetGiphyKey { key: giphy_key.trim().to_string() }, move |shared, ev| match ev {
            Event::Error { message, .. } => shared.state.write().toast(format!("Couldn't set up GIF search: {message}"), true),
            _ => shared
                .state
                .write()
                .toast(if on { "GIF search is on for everyone in this server." } else { "GIF search is off on this server." }, false),
        });
    }

    /// Owner only: the KLIPY app key everyone's KLIPY GIF search on this server uses ("" turns
    /// it off).
    pub fn set_klipy_key(&self, key: &str, klipy_key: &str) {
        let on = !klipy_key.trim().is_empty();
        self.request_then(key, Request::SetKlipyKey { key: klipy_key.trim().to_string() }, move |shared, ev| match ev {
            Event::Error { message, .. } => shared.state.write().toast(format!("Couldn't set up KLIPY: {message}"), true),
            _ => shared
                .state
                .write()
                .toast(if on { "KLIPY GIF search is on for everyone in this server." } else { "KLIPY GIF search is off on this server." }, false),
        });
    }

    /// Where backups of servers I own go (`None`: "Ingle Server Backups" in Documents).
    pub fn set_backup_dir(&self, dir: Option<std::path::PathBuf>) {
        let mut cfg = self.shared.config.lock();
        cfg.backup_dir = dir;
        self.shared.save_config_locked(&cfg);
    }

    pub fn can_back_up(&self, key: &str) -> bool {
        backup::can_back_up(&self.shared, key)
    }

    pub fn voice_prefs(&self) -> (bool, bool) {
        *self.shared.voice_prefs.lock()
    }

    pub fn start_camera(&self) {
        let settings = self.shared.state.read_recursive().settings.clone();
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
        *self.shared.screen_source.lock() = Some(source_id.to_string());
        let preset = self.shared.state.read_recursive().settings.screen_preset;
        let (max_h, fps, bitrate) = preset.params();
        let cfg = PublisherConfig { screen: true, max_width: max_h * 16 / 9 + 8, max_height: max_h, fps, bitrate };
        self.start_video(track::SCREEN, source_id, cfg);
    }

    fn start_video(&self, track_id: u8, source: &str, cfg: PublisherConfig) {
        let shared = self.shared.clone();
        let source = source.to_string();
        let which = (track_id == track::SCREEN) as usize;
        // This start wins over any earlier one still opening.
        let generation = shared.video_gen[which].fetch_add(1, Ordering::SeqCst) + 1;
        // Opening a camera or a capture can take seconds: never on the UI thread, and never while
        // holding the call (the UI, voice and the network need it meanwhile).
        self.shared.rt.spawn_blocking(move || {
            let current = || shared.video_gen[which].load(Ordering::SeqCst) == generation;
            // What's being sent now stops first (a camera can't be opened twice).
            let (ctx, old) = match shared.call.lock().as_mut() {
                Some(c) => (Some(c.video_ctx()), Some(c.take_video(&shared, track_id))),
                None => (None, None),
            };
            drop(old);
            let what = if track_id == track::SCREEN { "share your screen" } else { "start your camera" };
            let Some(ctx) = ctx else {
                shared.state.write().toast(format!("Couldn't {what}: join a voice channel first"), true);
                return;
            };
            if !current() {
                return;
            }
            let installed = match ctx.start(&shared, track_id, &source, cfg) {
                Ok((out, info)) => {
                    let mut call = shared.call.lock();
                    match call.as_mut() {
                        // Still in the same call, and nobody pressed stop meanwhile.
                        Some(c) if c.session == ctx.session && current() => {
                            let replaced = c.install_video(track_id, out, info);
                            drop(call);
                            drop(replaced);
                            true
                        }
                        _ => {
                            drop(call);
                            drop(out);
                            false
                        }
                    }
                }
                Err(e) => {
                    if current() {
                        shared.state.write().toast(format!("Couldn't {what}: {e:#}"), true);
                    }
                    false
                }
            };
            if installed && track_id == track::SCREEN && shared.state.read_recursive().settings.share_audio {
                // Sound for the stream. Started without holding the call lock, since the
                // system can take a moment to set up capture.
                if let Some(src) = stream_audio_source(&source) {
                    let sink = shared.call.lock().as_ref().map(|c| c.stream_audio_sink(&shared));
                    if let Some(sink) = sink {
                        // The call is taken back out of the stream's sound, in case it gets in
                        // (not from the test pattern, which never has it).
                        let played = (src != audio::stream::StreamAudioSource::Synthetic).then(|| shared.played.clone());
                        match audio::stream::StreamAudio::start(src, STREAM_AUDIO_KBPS, sink, played) {
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
        self.stop_video(track::CAMERA);
    }

    pub fn stop_screen(&self) {
        self.stop_video(track::SCREEN);
    }

    /// Stop sending our camera or screen share. Quick on the calling (UI) thread: capture lets
    /// go on a thread of its own.
    fn stop_video(&self, track_id: u8) {
        // A start that's still opening doesn't go live after this.
        self.shared.video_gen[(track_id == track::SCREEN) as usize].fetch_add(1, Ordering::SeqCst);
        let old = self.shared.call.lock().as_mut().map(|c| c.take_video(&self.shared, track_id));
        drop_later(old);
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
            let st = self.shared.state.read_recursive();
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
            let st = self.shared.state.read_recursive();
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
        let old = self.shared.state.read_recursive().settings.clone();
        if old == settings {
            return;
        }
        {
            let mut cfg = self.shared.config.lock();
            cfg.settings = settings.clone();
            self.shared.save_config_locked(&cfg);
        }
        self.shared.state.write().settings = settings.clone();
        qos::set_enabled(settings.voice_priority);
        if settings.noise_suppression == config::NoiseSuppression::Isolation && old.noise_suppression != settings.noise_suppression {
            audio::dfn::preload();
        }
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
        if old.sound_keys != settings.sound_keys {
            if let Some(k) = &self._keys {
                k.set_combos(&settings.sound_keys.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>());
            }
        }
        // Someone's soundboard just muted: their sounds playing now stop too.
        let newly_muted: Vec<&String> = settings.soundboard_muted.iter().filter(|u| !old.soundboard_muted.contains(u)).collect();
        if !newly_muted.is_empty() {
            let sessions: Vec<SessionId> = {
                let st = self.shared.state.read_recursive();
                st.call
                    .as_ref()
                    .and_then(|c| st.server(&c.server_key))
                    .map(|s| s.voice.values().filter(|p| newly_muted.contains(&&p.user_id)).map(|p| p.session).collect())
                    .unwrap_or_default()
            };
            if let Some(a) = self.shared.call.lock().as_ref().and_then(|c| c.audio.as_ref()) {
                let mut mixer = a.mixer.lock();
                for s in sessions {
                    mixer.stop_effects(Some(s));
                }
            }
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
        let settings = self.shared.state.read_recursive().settings.clone();
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

    /// A live preview of a screen or window in the share picker (`None` stops it). A few
    /// frames a second, without the "you're sharing" frame; nothing is sent anywhere.
    pub fn preview_screen(&self, source: Option<&str>) {
        let old = {
            let mut p = self.shared.screen_preview.lock();
            if p.as_ref().map(|x| x.0.as_str()) == source {
                return;
            }
            let old = p.take();
            if let Some(source) = source {
                let slot = Arc::new(video::FrameSlot::default());
                *p = Some((source.to_string(), None, slot.clone()));
                let (shared, source) = (self.shared.clone(), source.to_string());
                self.shared.rt.spawn_blocking(move || {
                    let cap = video::capture::start_screen(&source, 4, slot.clone()).map(|c| c.without_border());
                    let unwanted = {
                        let mut p = shared.screen_preview.lock();
                        match (p.as_mut(), cap) {
                            // Still the one wanted (this very request): keep it running.
                            (Some(x), Ok(c)) if Arc::ptr_eq(&x.2, &slot) && x.1.is_none() => {
                                x.1 = Some(c);
                                None
                            }
                            (_, Ok(c)) => Some(c),
                            (_, Err(e)) => {
                                tracing::info!("no preview of {source}: {e:#}");
                                None
                            }
                        }
                    };
                    // Stopping a capture waits for it: never while holding the preview.
                    drop(unwanted);
                    (shared.repaint)();
                });
            }
            old
        };
        // The one being replaced stops on a thread of its own (this is the UI thread).
        drop_later(old);
    }

    /// The preview's newest frame, scaled to at most `max_w` wide, as RGBA.
    pub fn preview_frame(&self, max_w: u32) -> Option<(u32, u32, Vec<u8>)> {
        let slot = self.shared.screen_preview.lock().as_ref().map(|x| x.2.clone())?;
        let f = slot.take_wait(std::time::Duration::ZERO)?;
        let bpp = f.bytes_per_pixel();
        if f.width == 0 || f.height == 0 || f.data.len() < (f.width * f.height) as usize * bpp {
            return None;
        }
        let step = f.width.div_ceil(max_w.max(1)).max(1);
        let (w, h) = (f.width / step, f.height / step);
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let i = (((y * step) * f.width + x * step) as usize) * bpp;
                let px = &f.data[i..i + bpp];
                match f.format {
                    video::codec::PixelFormat::Bgra => out.extend_from_slice(&[px[2], px[1], px[0], 255]),
                    video::codec::PixelFormat::Rgba => out.extend_from_slice(&[px[0], px[1], px[2], 255]),
                    video::codec::PixelFormat::Rgb => out.extend_from_slice(&[px[0], px[1], px[2], 255]),
                }
            }
        }
        Some((w, h, out))
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
/// People blocked on this PC are blocked on every server too (one joined later, say).
pub(crate) fn sync_blocks(shared: &Arc<Shared>, key: &str) {
    let todo: Vec<String> = {
        let st = shared.state.read_recursive();
        let Some(s) = st.server(key).filter(|s| s.has(FRIENDS_FEATURE)) else { return };
        st.blocked.iter().filter(|u| s.members.contains_key(*u) && s.relations.get(*u) != Some(&RelationKind::Blocked)).cloned().collect()
    };
    let Some(h) = shared.servers.lock().get(key).cloned() else { return };
    for u in todo {
        h.notify(Request::Block { user_id: u });
    }
}

/// Back on a server after the connection dropped mid-call: rejoin that call. A camera or
/// screen share that was on isn't turned back on by itself (the screens or windows may have
/// changed meanwhile, and you may have moved on): the app offers it (`AppState::resume_offer`).
pub(crate) fn resume_voice(shared: Arc<Shared>, key: String) {
    let r = {
        let mut st = shared.state.write();
        match &st.voice_rejoin {
            Some(r) if r.server_key == key && st.call.is_none() => {
                if r.since.elapsed() > VOICE_REJOIN_FOR {
                    st.voice_rejoin = None;
                    return;
                }
                r.clone()
            }
            _ => return,
        }
    };
    let rt = shared.rt.clone();
    rt.spawn(async move {
        // The channel may be gone, or the server may say no now.
        let exists = shared.state.read_recursive().server(&key).map(|s| s.channel(r.channel_id).is_some()).unwrap_or(false);
        let result = if exists { Call::join(shared.clone(), key.clone(), r.channel_id).await } else { Err(anyhow::anyhow!("the channel is gone")) };
        {
            let mut st = shared.state.write();
            // Only if nothing else happened meanwhile (the user leaving or joining elsewhere).
            if st.voice_rejoin.as_ref().map(|x| x.since) == Some(r.since) {
                st.voice_rejoin = None;
            }
            if let Err(e) = &result {
                st.toast(format!("Couldn't get back into voice: {e:#}"), true);
            }
        }
        if result.is_err() {
            if shared.call.lock().is_none() {
                *shared.call_events.lock() = None;
            }
            (shared.repaint)();
            return;
        }
        if r.camera || r.screen.is_some() {
            shared.state.write().resume_offer =
                Some(ResumeOffer { server_key: key.clone(), channel_id: r.channel_id, camera: r.camera, screen: r.screen.is_some() });
        }
        (shared.repaint)();
    });
}

pub(crate) fn sync_avatar(shared: Arc<Shared>, key: String) {
    let rt = shared.rt.clone();
    rt.spawn(async move {
        let (has_feature, current) = {
            let st = shared.state.read_recursive();
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
                        errors::report(format!("Couldn't upload your profile picture: {e:#}"));
                        return;
                    }
                }
            }
            None => None,
        };
        if let Ok(Event::Error { message, .. }) = h.request(Request::SetAvatar { file_id }).await {
            tracing::warn!("couldn't set the profile picture: {message}");
            errors::report(format!("Couldn't set your profile picture: {message}"));
        }
    });
}
