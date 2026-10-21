//! Persisted client configuration and the local identity key.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pyr_proto::control::{ChannelId, MessageId, UserId};
use pyr_proto::crypto::Identity;
use pyr_proto::{APP_NAME, b64};
use serde::{Deserialize, Serialize};

pub use crate::soundboard::SoundRef;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ServerEntry {
    /// Server identity: base64url SHA-256 of its certificate.
    pub key: String,
    pub host: String,
    pub port: u16,
    /// Invite code used to join (only needed until the first successful login).
    #[serde(default)]
    pub invite: Option<String>,
    /// Last known name, shown while connecting.
    #[serde(default)]
    pub name: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum InputMode {
    #[default]
    VoiceActivity,
    PushToTalk,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NoiseSuppression {
    Off,
    /// WebRTC's statistical noise suppressor (very cheap).
    Standard,
    /// RNNoise neural suppressor: removes keyboards, fans, dogs.
    #[default]
    Strong,
    /// Only your voice: DeepFilterNet3 cleans the sound (typing, clicks, fans and machines
    /// too), and only the close voice at the mic (yours) gets through: TVs, people talking in
    /// the room and sounds between your words are cut.
    Isolation,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CameraQuality {
    Low,
    #[default]
    Standard,
    High,
}

impl CameraQuality {
    /// (width, height, fps, bitrate bps)
    pub fn params(self) -> (u32, u32, u32, u32) {
        match self {
            CameraQuality::Low => (640, 360, 24, 500_000),
            CameraQuality::Standard => (1280, 720, 30, 1_500_000),
            CameraQuality::High => (1920, 1080, 30, 3_000_000),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            CameraQuality::Low => "360p",
            CameraQuality::Standard => "720p",
            CameraQuality::High => "1080p",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ScreenPreset {
    /// 720p30, ~2 Mbps — for slow connections.
    Saver,
    /// 1080p30, ~4 Mbps.
    Standard,
    /// 1080p60, ~6 Mbps — games and video.
    #[default]
    Smooth,
    /// 1440p30, ~8 Mbps — small text / code.
    Sharp,
}

impl ScreenPreset {
    /// (max height, fps, bitrate bps)
    pub fn params(self) -> (u32, u32, u32) {
        match self {
            ScreenPreset::Saver => (720, 30, 2_000_000),
            ScreenPreset::Standard => (1080, 30, 4_000_000),
            ScreenPreset::Smooth => (1080, 60, 6_000_000),
            ScreenPreset::Sharp => (1440, 30, 8_000_000),
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ScreenPreset::Saver => "720p 30fps (data saver)",
            ScreenPreset::Standard => "1080p 30fps",
            ScreenPreset::Smooth => "1080p 60fps (smooth)",
            ScreenPreset::Sharp => "1440p 30fps (sharp text)",
        }
    }
    pub const ALL: [ScreenPreset; 4] = [ScreenPreset::Saver, ScreenPreset::Standard, ScreenPreset::Smooth, ScreenPreset::Sharp];
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub input_device: Option<String>,
    pub output_device: Option<String>,
    pub input_mode: InputMode,
    /// Key name understood by `device_query` (e.g. "LControl", "CapsLock", "Grave", "F13"),
    /// or "Mouse4"/"Mouse5".
    pub ptt_key: String,
    pub ptt_release_ms: u32,
    /// Automatically decide when you're talking (otherwise use `vad_threshold_db`).
    pub vad_auto: bool,
    pub vad_threshold_db: f32,
    pub noise_suppression: NoiseSuppression,
    pub echo_cancellation: bool,
    pub auto_gain: bool,
    pub voice_bitrate_kbps: u32,
    /// Mark voice packets so routers and Wi-Fi send them first (Windows; like Discord's
    /// "Quality of Service High Packet Priority"). A few routers misbehave with it.
    pub voice_priority: bool,
    pub input_gain: f32,
    pub output_volume: f32,
    pub camera_device: Option<String>,
    pub camera_quality: CameraQuality,
    pub screen_preset: ScreenPreset,
    pub share_audio: bool,
    /// Open Pyr when Windows starts (like Discord), and then only in the tray.
    pub start_with_windows: bool,
    pub start_hidden: bool,
    /// No longer used: the close button always keeps Pyr running by the clock (Windows).
    pub close_to_tray: bool,
    /// The one-time notice that Pyr is still running by the clock has been shown.
    pub tray_note_seen: bool,
    /// Notification sounds (messages, people joining voice, streams).
    pub sounds: bool,
    pub sound_volume: f32,
    /// Ring when someone calls you in a direct message (and the ringback when you call),
    /// even with the other sounds off.
    pub ring: bool,
    /// Emoji picked most recently, newest first (the picker's "Frequently used").
    pub recent_emoji: Vec<String>,
    /// Skin tone for emoji that have them: 0 = default yellow, 1–5 light to dark.
    pub emoji_tone: u8,
    /// A GIPHY API key of your own (otherwise the one built into Pyr, if any).
    pub giphy_key: String,
    /// Underline misspelled words in the message box (right-click it to switch off).
    pub spellcheck: bool,
    /// Its dictionary: "en-US", "en-GB", "es" or "fr".
    pub spell_language: String,
    /// Words added to the dictionary (right-click one → Add to dictionary).
    pub spell_words: Vec<String>,
    /// Ask before deleting a message (holding Shift when clicking Delete skips it anyway).
    pub confirm_delete: bool,
    /// How loud soundboard sounds play here, 0–1 (anyone's, yours included).
    pub soundboard_volume: f32,
    /// The soundboard's favorites (★), in the order they were starred.
    pub sound_favorites: Vec<SoundRef>,
    /// Sounds played most recently, newest first.
    pub sound_recent: Vec<SoundRef>,
    /// Played for everyone in the channel when you join voice.
    pub join_sound: Option<SoundRef>,
    /// People whose soundboard sounds you don't hear.
    pub soundboard_muted: Vec<UserId>,
    /// Keys that play a sound while you're in voice: ("LControl+Key1", sound).
    pub sound_keys: Vec<(String, SoundRef)>,
    /// GIFs kept in the GIF picker's Favorites (the star on a GIF, like Discord's), newest first.
    pub gif_favorites: Vec<GifFavorite>,
    /// The skin in use: one installed from the Pyr Library, by its id (empty: Campfire,
    /// Pyr's own).
    pub skin: String,
    /// Plugins and skins starred in the Library (`skin:<id>` for a skin), oldest first.
    pub library_favorites: Vec<String>,
    /// How the Library is sorted: "name", "author", "newest" or "favorites".
    pub library_sort: String,
    /// Pyr Poker installed (from Plugins): only then is it in the channel list of the servers
    /// that have it.
    pub poker_installed: bool,
    /// Streamer mode: my cards at the poker table stay face down until I peek, the hand
    /// strength meter with them, and pop-ups don't show what messages say. (It also turns on by
    /// itself while I'm sharing my screen; see the app.)
    pub streamer_mode: bool,
    /// Streamer mode at a casino game or a poker table: room kept clear down one side of it for
    /// a camera and overlays ("", "left" or "right"), and whether the game fills Pyr's window
    /// (Pyr's server and channel lists put away while it's open).
    pub stream_room: String,
    pub stream_stage: bool,
    /// Send a report by itself when something goes wrong that Pyr can tell (screen sharing
    /// or the camera won't start or stops working, a crash or a freeze last time), so it gets
    /// fixed: what happened and Pyr's log, to the bug report server (see `bugs::auto_report`).
    pub auto_reports: bool,
    /// The one-time notice that a report was sent by itself has been shown.
    pub auto_report_told: bool,
    /// Idle by my own say-so (shown as a moon until I change it), instead of after five
    /// minutes away from the keyboard and mouse (see `idle`).
    pub idle_by_hand: bool,
    /// Vouch for the people I make sure of, on the servers we share: then people who made sure
    /// of me see them as vouched for (see `verify`).
    pub vouch: bool,
    /// How voice meters look: "bars", "spectrum", "wave", "dots", "glow" or "off" (empty: bars).
    pub voice_meter: String,
    /// Do Not Disturb, like Discord's: no message sounds, calls don't ring or pop up a Windows
    /// notice, and everyone sees a red dot on my picture (see `idle`, which tells servers).
    pub dnd: bool,
    /// What calls ring with (none: Pyr's own ring; see `ring`), and someone's own ringtone,
    /// by their id (in place of that).
    pub ringtone: Option<Ringtone>,
    pub ringtones: std::collections::BTreeMap<UserId, Ringtone>,
    /// When moving pictures in chat (GIFs, animated WebP and PNG) play: "" always, "hover"
    /// while pointed at, "never" (they show still, marked GIF).
    pub play_gifs: String,
}

/// A ringtone: a soundboard sound (built in, or one of a server's), or a sound file from this
/// PC, kept in the config folder's `rings` as a Pyr sound (see `Engine::add_ringtone_file`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Ringtone {
    /// What it's called: the sound's name, or the file's.
    pub name: String,
    /// A soundboard sound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sound: Option<SoundRef>,
    /// A sound file from this PC: its name in `rings`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub file: String,
}

impl Settings {
    /// Whether voice meters show (`voice_meter` is "off", or "off:" and the look they had).
    pub fn voice_meters_on(&self) -> bool {
        !(self.voice_meter == "off" || self.voice_meter.starts_with("off:"))
    }
}

impl Ringtone {
    /// Which sound it is (two ringtones with the same key sound the same).
    pub fn key(&self) -> String {
        match &self.sound {
            Some(s) => format!("sound:{}:{}", s.server, s.id),
            None => format!("file:{}", self.file),
        }
    }
}

/// A favorite GIF: just its links (the GIF itself is fetched from GIPHY or KLIPY each time it's
/// shown, as they ask), so the picker can show it and send it again.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct GifFavorite {
    /// The link as posted in chat (with its size: GIPHY's after a `?`, KLIPY's after a `#`).
    pub url: String,
    /// A smaller moving picture for the picker's grid (empty: `url` itself).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub preview: String,
    /// The preview's size, to lay out the grid before it loads (0 when unknown).
    pub w: u16,
    pub h: u16,
    /// KLIPY's name for it, to tell KLIPY when it's shared (empty when unknown).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub slug: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// A picture from chat (a meme, say) kept as a copy of its own: the copy's name in the
    /// favorites folder (empty: a GIF from GIPHY or KLIPY, at `url`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub file: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            input_device: None,
            output_device: None,
            input_mode: InputMode::VoiceActivity,
            ptt_key: "Grave".into(),
            ptt_release_ms: 200,
            vad_auto: true,
            vad_threshold_db: -50.0,
            noise_suppression: NoiseSuppression::Strong,
            echo_cancellation: true,
            auto_gain: true,
            voice_bitrate_kbps: 64,
            voice_priority: true,
            input_gain: 1.0,
            output_volume: 1.0,
            camera_device: None,
            camera_quality: CameraQuality::Standard,
            screen_preset: ScreenPreset::Smooth,
            share_audio: true,
            start_with_windows: true,
            start_hidden: true,
            close_to_tray: true,
            tray_note_seen: false,
            sounds: true,
            sound_volume: 0.7,
            ring: true,
            recent_emoji: Vec::new(),
            emoji_tone: 0,
            giphy_key: String::new(),
            spellcheck: true,
            spell_language: "en-US".into(),
            spell_words: Vec::new(),
            confirm_delete: true,
            soundboard_volume: 0.7,
            sound_favorites: Vec::new(),
            sound_recent: Vec::new(),
            join_sound: None,
            soundboard_muted: Vec::new(),
            sound_keys: Vec::new(),
            gif_favorites: Vec::new(),
            skin: String::new(),
            library_favorites: Vec::new(),
            library_sort: String::new(),
            poker_installed: false,
            streamer_mode: false,
            stream_room: String::new(),
            stream_stage: true,
            voice_meter: String::new(),
            dnd: false,
            ringtone: None,
            ringtones: Default::default(),
            play_gifs: String::new(),
            auto_reports: true,
            auto_report_told: false,
            idle_by_hand: false,
            vouch: true,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Config {
    pub display_name: String,
    pub servers: Vec<ServerEntry>,
    pub settings: Settings,
    pub user_volumes: HashMap<UserId, f32>,
    /// Volume of each person's stream audio (screen share sound).
    pub stream_volumes: HashMap<UserId, f32>,
    /// server key -> channel -> newest message id seen
    pub last_read: HashMap<String, HashMap<ChannelId, MessageId>>,
    /// Muted (no message sound, no unread dot): whole servers, and channels per server.
    pub muted_servers: std::collections::HashSet<String>,
    pub muted_channels: HashMap<String, std::collections::HashSet<ChannelId>>,
    /// People I've blocked (their messages are hidden; servers keep them from contacting me).
    pub blocked: std::collections::HashSet<UserId>,
    /// Mute and deafen, kept across restarts and updates (like Discord) so nobody comes back
    /// from an update with their microphone on. `None` until this PC first saves them.
    pub self_voice: Option<SelfVoice>,
    /// Where backups of servers I own go (`None`: "Pyr Server Backups" in Documents).
    pub backup_dir: Option<PathBuf>,
    /// People I made sure are who they say, by id (see `verify`).
    pub verified: std::collections::BTreeMap<UserId, Verified>,
}

/// Someone I made sure is who they say (see `verify`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Verified {
    /// Their name when I made sure of them: someone else showing up under it is flagged.
    pub name: String,
    pub how: VerifiedHow,
    /// When (ms since 1970).
    pub at_ms: i64,
}

/// How I made sure of someone.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerifiedHow {
    /// We compared words.
    Words,
    /// I asked something only they'd know (or they asked me, and it could only be them).
    Question,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(default)]
pub struct SelfVoice {
    pub muted: bool,
    pub deafened: bool,
}

/// Where the config lives. `profile` lets several identities coexist on one PC (for testing).
pub fn config_dir(profile: Option<&str>) -> PathBuf {
    let profile = profile.filter(|p| valid_profile(p));
    if let Some(dir) = pyr_proto::env_var("HOME") {
        let base = PathBuf::from(dir);
        return match profile {
            Some(p) => base.join("profiles").join(p),
            None => base,
        };
    }
    let base = directories::ProjectDirs::from("", "", APP_NAME)
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(format!(".{}", pyr_proto::APP_ID)));
    match profile {
        Some(p) => base.join("profiles").join(p),
        None => base,
    }
}

pub fn load_or_create_identity(dir: &Path) -> Result<Identity> {
    let path = dir.join("identity.key");
    if let Ok(text) = std::fs::read_to_string(&path) {
        if let Some(seed) = b64::decode_array::<32>(text.trim()) {
            return Ok(Identity::from_seed(seed));
        }
        anyhow::bail!("{} is corrupted; move it away to create a new identity", path.display());
    }
    create_private_dir(dir)?;
    let id = Identity::generate();
    write_atomic(&path, b64::encode(&id.seed()).as_bytes())?;
    Ok(id)
}

pub fn load_config(dir: &Path) -> Config {
    std::fs::read(dir.join("config.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_config(dir: &Path, cfg: &Config) -> Result<()> {
    create_private_dir(dir)?;
    let data = serde_json::to_vec_pretty(cfg)?;
    write_atomic(&dir.join("config.json"), &data)
}

/// Writes a file in one go (a new file renamed over the old), readable only by this user on
/// Linux from the moment it exists: the identity key is the user's whole identity, and the
/// config holds their servers and keys for services.
fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    use std::io::Write;
    let tmp = path.with_extension("tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp).with_context(|| format!("writing {}", tmp.display()))?;
    f.write_all(data).with_context(|| format!("writing {}", tmp.display()))?;
    let _ = f.sync_all();
    drop(f);
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
}

/// Makes the config folder (only this user may open it, on Linux).
fn create_private_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Pyr had another name before 0.8.0 (see `pyr_proto::legacy`). The first time Pyr starts, it
/// takes over the folders from then: the settings, the identity (so it's the same person on
/// every server), and test profiles are copied (the old folder stays as it was, as a backup), and the plugins and
/// installed skins, which can be big, are moved. The settings come over only while Pyr has
/// none of its own; the plugins and skins whenever Pyr has none (0.8.0 left the skins
/// behind, so a skin being worn "wasn't installed": they come over on the next start). Not
/// with `PYR_HOME` (test setups). Run before anything opens the config folder (the app, the
/// connector). Returns what it did, for the log.
pub fn migrate_from_old_name() -> Vec<String> {
    if pyr_proto::env_set("HOME") {
        return Vec::new();
    }
    let dirs = |name: &str| directories::ProjectDirs::from("", "", name);
    let (Some(new), Some(old)) = (dirs(APP_NAME), dirs(pyr_proto::legacy::APP_NAME)) else { return Vec::new() };
    migrate_folders(old.config_dir(), new.config_dir(), old.data_local_dir(), new.data_local_dir())
}

/// Files in the old folder that stay behind: a running copy's lock and hand-over, its logs
/// (`legacy::LOG_FILES`), and a half-finished update.
const LEFT_BEHIND: &[&str] = &["instance.lock", "handoff", "resume.json"];

fn left_behind(name: &std::ffi::OsStr) -> bool {
    LEFT_BEHIND.iter().chain(pyr_proto::legacy::LOG_FILES).any(|l| name == *l)
}

/// What a successful copy of the settings says (the app greets people who came over).
pub const COPIED_SETTINGS: &str = "copied the settings and identity from before the rename";

/// What's kept in the local (not roaming) data folder and comes over from the old one: the
/// plugins and the installed skins.
const DATA_FOLDERS: &[(&str, &str)] = &[("plugins", "plugins"), ("skins", "skins")];

/// `migrate_from_old_name` with the folders given (the config folders, and the data folders that
/// hold `plugins` and `skins`).
pub fn migrate_folders(old_config: &Path, new_config: &Path, old_data: &Path, new_data: &Path) -> Vec<String> {
    let mut did = Vec::new();
    let has_files = |d: &Path| std::fs::read_dir(d).map(|mut r| r.next().is_some()).unwrap_or(false);
    if has_files(old_config) && !has_files(new_config) {
        match copy_tree(old_config, new_config) {
            Ok(n) => {
                let _ = std::fs::write(new_config.join("copied-over.txt"), format!("Copied from {} by Pyr, from before the rename (0.8.0).\n", old_config.display()));
                did.push(format!("{COPIED_SETTINGS} ({n} files) from {} to {}", old_config.display(), new_config.display()));
            }
            Err(e) => did.push(format!("couldn't copy the settings from before the rename from {}: {e:#}", old_config.display())),
        }
    }
    // (Moved only into an empty place: Pyr's own plugins and skins are never mixed with the
    // old ones.)
    for (folder, what) in DATA_FOLDERS {
        let (from, to) = (old_data.join(folder), new_data.join(folder));
        if !has_files(&from) || has_files(&to) {
            continue;
        }
        let _ = std::fs::create_dir_all(new_data);
        let _ = std::fs::remove_dir(&to);
        let how = match std::fs::rename(&from, &to) {
            Ok(()) => Ok("moved"),
            // (Another drive, or a file in use: copied instead, the old ones left as they are.)
            Err(_) => copy_tree(&from, &to).map(|_| "copied"),
        };
        match how {
            Ok(how) => did.push(format!("{how} the {what} from before the rename from {} to {}", from.display(), to.display())),
            Err(e) => did.push(format!("couldn't bring the {what} from before the rename over from {}: {e:#}", from.display())),
        }
    }
    did
}

/// Copies a folder and everything in it (but `LEFT_BEHIND` and links), keeping it private;
/// how many files.
fn copy_tree(from: &Path, to: &Path) -> Result<usize> {
    create_private_dir(to)?;
    let mut n = 0;
    for entry in std::fs::read_dir(from).with_context(|| format!("reading {}", from.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        let kind = entry.file_type()?;
        if left_behind(&name) || kind.is_symlink() {
            continue;
        }
        let (src, dst) = (entry.path(), to.join(&name));
        if kind.is_dir() {
            n += copy_tree(&src, &dst)?;
        } else if kind.is_file() {
            std::fs::copy(&src, &dst).with_context(|| format!("copying {}", src.display()))?;
            n += 1;
        }
    }
    Ok(n)
}

/// A profile name is a plain word: never a path (which could point the config, and the
/// identity key, somewhere else, even a network share).
pub fn valid_profile(p: &str) -> bool {
    !p.is_empty() && p.len() <= 32 && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The folders from before the rename come over once: the settings and identity copied (the old ones kept),
    /// the plugins and skins moved, a running copy's lock and the logs left behind.
    #[test]
    fn old_folders_come_over_once() {
        let root = std::env::temp_dir().join(format!("pyr-migrate-{}-{}", std::process::id(), pyr_proto::now_ms()));
        let (old_cfg, new_cfg) = (root.join(pyr_proto::legacy::APP_NAME).join("config"), root.join("Pyr/config"));
        let (old_data, new_data) = (root.join(pyr_proto::legacy::APP_NAME).join("data"), root.join("Pyr/data"));
        std::fs::create_dir_all(old_cfg.join("profiles/b")).unwrap();
        std::fs::create_dir_all(old_data.join("plugins/chim")).unwrap();
        std::fs::create_dir_all(old_data.join("skins/blood-moon")).unwrap();
        for (f, t) in [("config.json", "{}"), ("identity.key", "seed"), ("instance.lock", ""), (pyr_proto::legacy::LOG_FILES[0], "log"), ("profiles/b/identity.key", "b")] {
            std::fs::write(old_cfg.join(f), t).unwrap();
        }
        std::fs::write(old_data.join("plugins/installed.json"), "[]").unwrap();
        std::fs::write(old_data.join("plugins/chim/chim.exe"), "exe").unwrap();
        std::fs::write(old_data.join("skins/installed.json"), "[]").unwrap();
        std::fs::write(old_data.join("skins/blood-moon/skin.json"), "{}").unwrap();
        let did = migrate_folders(&old_cfg, &new_cfg, &old_data, &new_data);
        assert_eq!(did.len(), 3, "{did:?}");
        assert_eq!(std::fs::read_to_string(new_cfg.join("identity.key")).unwrap(), "seed");
        assert_eq!(std::fs::read_to_string(new_cfg.join("profiles/b/identity.key")).unwrap(), "b");
        assert!(!new_cfg.join("instance.lock").exists() && !new_cfg.join(pyr_proto::legacy::LOG_FILES[0]).exists());
        assert!(old_cfg.join("identity.key").exists(), "the old copy stays");
        assert_eq!(std::fs::read_to_string(new_data.join("plugins/chim/chim.exe")).unwrap(), "exe");
        assert_eq!(std::fs::read_to_string(new_data.join("skins/blood-moon/skin.json")).unwrap(), "{}");
        // Once only: Pyr's own settings are never replaced.
        std::fs::write(new_cfg.join("config.json"), "{\"pyr\":1}").unwrap();
        assert!(migrate_folders(&old_cfg, &new_cfg, &old_data, &new_data).is_empty());
        assert_eq!(std::fs::read_to_string(new_cfg.join("config.json")).unwrap(), "{\"pyr\":1}");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Someone who came over with 0.8.0 (settings and plugins moved, skins left in the old
    /// folder): the skins come over on the next start, and nothing else is touched.
    #[test]
    fn skins_left_behind_by_0_8_0_come_over() {
        let root = std::env::temp_dir().join(format!("pyr-migrate-skins-{}-{}", std::process::id(), pyr_proto::now_ms()));
        let (old_cfg, new_cfg) = (root.join(pyr_proto::legacy::APP_NAME).join("config"), root.join("Pyr/config"));
        let (old_data, new_data) = (root.join(pyr_proto::legacy::APP_NAME).join("data"), root.join("Pyr/data"));
        for d in [&old_cfg, &new_cfg, &new_data.join("plugins"), &old_data.join("skins/blood-moon")] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(old_cfg.join("config.json"), "{\"old\":1}").unwrap();
        std::fs::write(new_cfg.join("config.json"), "{\"skin\":\"blood-moon\"}").unwrap();
        std::fs::write(new_data.join("plugins/installed.json"), "[\"pyr\"]").unwrap();
        std::fs::write(old_data.join("skins/installed.json"), "[\"blood-moon\"]").unwrap();
        std::fs::write(old_data.join("skins/blood-moon/background.webp"), "moon").unwrap();
        let did = migrate_folders(&old_cfg, &new_cfg, &old_data, &new_data);
        assert_eq!(did.len(), 1, "{did:?}");
        assert!(did[0].contains("skins"), "{did:?}");
        assert_eq!(std::fs::read_to_string(new_data.join("skins/blood-moon/background.webp")).unwrap(), "moon");
        assert_eq!(std::fs::read_to_string(new_cfg.join("config.json")).unwrap(), "{\"skin\":\"blood-moon\"}");
        assert_eq!(std::fs::read_to_string(new_data.join("plugins/installed.json")).unwrap(), "[\"pyr\"]");
        assert!(migrate_folders(&old_cfg, &new_cfg, &old_data, &new_data).is_empty(), "once");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn identity_persists() {
        let dir = std::env::temp_dir().join(format!("pyr-test-{}", pyr_proto::now_ms()));
        let a = load_or_create_identity(&dir).unwrap();
        let b = load_or_create_identity(&dir).unwrap();
        assert_eq!(a.user_id(), b.user_id());
        let mut cfg = Config::default();
        cfg.display_name = "Blue".into();
        save_config(&dir, &cfg).unwrap();
        assert_eq!(load_config(&dir).display_name, "Blue");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for f in ["identity.key", "config.json"] {
                let mode = std::fs::metadata(dir.join(f)).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o600, "{f} is readable by others");
            }
            assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn profiles_are_names_not_paths() {
        assert!(valid_profile("test-2"));
        for bad in ["", "..", "../x", "a/b", "C:\\x", "\\\\host\\share", "/etc", "x y"] {
            assert!(!valid_profile(bad), "{bad}");
        }
        let base = config_dir(None);
        assert_eq!(config_dir(Some("/etc")), base, "a path is ignored, not used");
        assert_eq!(config_dir(Some("p1")), base.join("profiles").join("p1"));
    }
}
