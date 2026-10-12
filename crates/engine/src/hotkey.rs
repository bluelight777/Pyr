//! Global push-to-talk: polls the key/mouse state without grabbing the key, so the key
//! still works in your game or other apps (Discord-style). Works on Windows, macOS
//! (needs Accessibility permission) and X11. Wayland needs the GlobalShortcuts portal (roadmap).

use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

pub struct PttWatcher {
    stop: Arc<AtomicBool>,
    pub key: Arc<Mutex<String>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Key names offered in settings (device_query names).
pub const KEY_CHOICES: &[&str] = &[
    "Grave", "CapsLock", "LControl", "RControl", "LAlt", "RAlt", "LShift", "RShift", "Space", "Tab", "V", "B", "T", "F13",
    "F14", "F15", "F8", "F9", "F10", "Mouse4", "Mouse5",
];

pub fn key_label(name: &str) -> String {
    match name {
        "Grave" => "` (backtick)".into(),
        "LControl" => "Left Ctrl".into(),
        "RControl" => "Right Ctrl".into(),
        "LAlt" => "Left Alt".into(),
        "RAlt" => "Right Alt".into(),
        "LShift" => "Left Shift".into(),
        "RShift" => "Right Shift".into(),
        "Mouse4" => "Mouse button 4 (back)".into(),
        "Mouse5" => "Mouse button 5 (forward)".into(),
        other => other.to_string(),
    }
}

impl PttWatcher {
    /// Start polling. `down` follows the key state; returns None if no input backend
    /// is available (e.g. no X display).
    pub fn start(key: String, down: Arc<AtomicBool>, on_change: Arc<dyn Fn() + Send + Sync>) -> PttWatcher {
        let stop = Arc::new(AtomicBool::new(false));
        let key = Arc::new(Mutex::new(key));
        let (st, k) = (stop.clone(), key.clone());
        let thread = std::thread::Builder::new()
            .name("ingle-ptt".into())
            .spawn(move || {
                use device_query::{DeviceQuery, DeviceState, Keycode};
                let Some(state) = DeviceState::checked_new() else {
                    tracing::warn!("push-to-talk unavailable: can't read keyboard state");
                    return;
                };
                let mut was = false;
                while !st.load(Ordering::Relaxed) {
                    let name = k.lock().clone();
                    let pressed = match name.as_str() {
                        "Mouse4" => state.get_mouse().button_pressed.get(4).copied().unwrap_or(false),
                        "Mouse5" => state.get_mouse().button_pressed.get(5).copied().unwrap_or(false),
                        other => match Keycode::from_str(other) {
                            Ok(code) => state.get_keys().contains(&code),
                            Err(_) => false,
                        },
                    };
                    if pressed != was {
                        was = pressed;
                        down.store(pressed, Ordering::Relaxed);
                        on_change();
                    }
                    std::thread::sleep(Duration::from_millis(8));
                }
            })
            .ok();
        PttWatcher { stop, key, thread }
    }

    pub fn set_key(&self, key: &str) {
        *self.key.lock() = key.to_string();
    }
}

impl Drop for PttWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
