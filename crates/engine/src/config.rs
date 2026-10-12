//! Persisted client configuration and the local identity key.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use ingle_proto::control::{ChannelId, MessageId, UserId};
use ingle_proto::crypto::Identity;
use ingle_proto::{APP_NAME, b64};
use serde::{Deserialize, Serialize};

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
    /// RNNoise plus only letting through the close voice at the mic (yours): mutes TVs,
    /// people talking in the room and sounds between your words.
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
    pub input_gain: f32,
    pub output_volume: f32,
    pub camera_device: Option<String>,
    pub camera_quality: CameraQuality,
    pub screen_preset: ScreenPreset,
    pub share_audio: bool,
    /// Windows: the window's close button hides Ingle to its icon by the clock (the
    /// notification area) instead of quitting, so calls and messages keep coming in.
    pub close_to_tray: bool,
    /// The one-time note explaining `close_to_tray` has been shown.
    pub tray_note_seen: bool,
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
            input_gain: 1.0,
            output_volume: 1.0,
            camera_device: None,
            camera_quality: CameraQuality::Standard,
            screen_preset: ScreenPreset::Smooth,
            share_audio: true,
            close_to_tray: true,
            tray_note_seen: false,
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
}

/// Where the config lives. `profile` lets several identities coexist on one PC (for testing).
pub fn config_dir(profile: Option<&str>) -> PathBuf {
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
    std::fs::create_dir_all(dir)?;
    let id = Identity::generate();
    write_atomic(&path, b64::encode(&id.seed()).as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(id)
}

pub fn load_config(dir: &Path) -> Config {
    std::fs::read(dir.join("config.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_config(dir: &Path, cfg: &Config) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let data = serde_json::to_vec_pretty(cfg)?;
    write_atomic(&dir.join("config.json"), &data)
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
    Ok(())
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
        let _ = std::fs::remove_dir_all(dir);
    }
}
