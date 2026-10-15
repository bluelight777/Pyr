//! Persisted client configuration and the local identity key.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ingle_proto::control::{ChannelId, MessageId, UserId};
use ingle_proto::crypto::Identity;
use ingle_proto::{APP_NAME, b64};
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
    /// Open Ingle when Windows starts (like Discord), and then only in the tray.
    pub start_with_windows: bool,
    pub start_hidden: bool,
    /// No longer used: the close button always keeps Ingle running by the clock (Windows).
    pub close_to_tray: bool,
    /// The one-time notice that Ingle is still running by the clock has been shown.
    pub tray_note_seen: bool,
    /// Notification sounds (messages, people joining voice, streams).
    pub sounds: bool,
    pub sound_volume: f32,
    /// Emoji picked most recently, newest first (the picker's "Frequently used").
    pub recent_emoji: Vec<String>,
    /// Skin tone for emoji that have them: 0 = default yellow, 1–5 light to dark.
    pub emoji_tone: u8,
    /// A GIPHY API key of your own (otherwise the one built into Ingle, if any).
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
    /// The soundboard's favourites (★), in the order they were starred.
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
    /// Where backups of servers I own go (`None`: "Ingle Server Backups" in Documents).
    pub backup_dir: Option<PathBuf>,
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
    if let Ok(dir) = std::env::var("INGLE_HOME") {
        let base = PathBuf::from(dir);
        return match profile {
            Some(p) => base.join("profiles").join(p),
            None => base,
        };
    }
    let base = directories::ProjectDirs::from("", "", APP_NAME)
        .map(|d| d.config_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(format!(".{}", ingle_proto::APP_ID)));
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

/// A profile name is a plain word: never a path (which could point the config, and the
/// identity key, somewhere else, even a network share).
pub fn valid_profile(p: &str) -> bool {
    !p.is_empty() && p.len() <= 32 && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_persists() {
        let dir = std::env::temp_dir().join(format!("ingle-test-{}", ingle_proto::now_ms()));
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
