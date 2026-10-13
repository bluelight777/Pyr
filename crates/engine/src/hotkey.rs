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

/// Keys that play soundboard sounds: key combinations ("LControl+Key1") polled like
/// push-to-talk, so they work in games without taking the keys away from them. `fire(i)` is
/// called when combination `i` goes down (all its keys held, at least one just now). It can
/// also record the next combination pressed, for setting one.
pub struct ComboWatcher {
    stop: Arc<AtomicBool>,
    combos: Arc<Mutex<Vec<Vec<device_query::Keycode>>>>,
    recording: Arc<AtomicBool>,
    recorded: Arc<Mutex<Option<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Modifier keys: held with another key, they make a combination; alone they're never one.
fn is_modifier(k: &device_query::Keycode) -> bool {
    use device_query::Keycode::*;
    matches!(k, LControl | RControl | LShift | RShift | LAlt | RAlt | LMeta | RMeta | Command | RCommand | LOption | ROption)
}

/// "LControl+Key1" as keys (None if a name isn't a key).
pub fn parse_combo(combo: &str) -> Option<Vec<device_query::Keycode>> {
    combo.split('+').map(|k| device_query::Keycode::from_str(k.trim()).ok()).collect()
}

/// Whether a combination is a key you type with, pressed on its own (a letter, a digit, Space,
/// Enter…): it would go off while chatting, so Ctrl, Alt or Shift has to be held with it.
/// Function keys, the number pad and Insert/Home/End/Page Up/Page Down work alone.
pub fn needs_modifier(combo: &str) -> bool {
    use device_query::Keycode::*;
    let Some(keys) = parse_combo(combo) else { return false };
    if keys.iter().any(is_modifier) {
        return false;
    }
    !keys.iter().all(|k| {
        matches!(
            k,
            F1 | F2 | F3 | F4 | F5 | F6 | F7 | F8 | F9 | F10 | F11 | F12 | F13 | F14 | F15 | F16 | F17 | F18 | F19 | F20 | Insert | Home | End | PageUp | PageDown
        ) || k.to_string().starts_with("Numpad")
    })
}

/// "Left Ctrl + 1" for "LControl+Key1".
pub fn combo_label(combo: &str) -> String {
    combo
        .split('+')
        .map(|k| match k {
            k if k.starts_with("Key") && k.len() == 4 => k[3..].to_string(),
            k if k.starts_with("Numpad") => format!("Num {}", &k[6..]),
            k => key_label(k),
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

impl ComboWatcher {
    pub fn start(combos: &[String], fire: Arc<dyn Fn(usize) + Send + Sync>, on_recorded: Arc<dyn Fn() + Send + Sync>) -> ComboWatcher {
        let stop = Arc::new(AtomicBool::new(false));
        let combos = Arc::new(Mutex::new(combos.iter().map(|c| parse_combo(c).unwrap_or_default()).collect::<Vec<_>>()));
        let recording = Arc::new(AtomicBool::new(false));
        let recorded = Arc::new(Mutex::new(None));
        let (st, cs, rec, out) = (stop.clone(), combos.clone(), recording.clone(), recorded.clone());
        let thread = std::thread::Builder::new()
            .name("ingle-sound-keys".into())
            .spawn(move || {
                use device_query::{DeviceQuery, DeviceState};
                let Some(state) = DeviceState::checked_new() else {
                    tracing::warn!("soundboard keys unavailable: can't read keyboard state");
                    return;
                };
                let mut was: Vec<device_query::Keycode> = Vec::new();
                while !st.load(Ordering::Relaxed) {
                    let now = state.get_keys();
                    let new: Vec<_> = now.iter().filter(|k| !was.contains(k)).cloned().collect();
                    if !new.is_empty() {
                        if rec.load(Ordering::Relaxed) {
                            // The first key that isn't a modifier, with the modifiers held.
                            if let Some(key) = new.iter().find(|k| !is_modifier(k)) {
                                let mut parts: Vec<String> = now.iter().filter(|k| is_modifier(k)).map(|k| k.to_string()).collect();
                                parts.sort();
                                parts.push(key.to_string());
                                *out.lock() = Some(parts.join("+"));
                                rec.store(false, Ordering::Relaxed);
                                on_recorded();
                            }
                        } else {
                            for (i, combo) in cs.lock().iter().enumerate() {
                                if !combo.is_empty() && combo.iter().all(|k| now.contains(k)) && combo.iter().any(|k| new.contains(k)) {
                                    fire(i);
                                }
                            }
                        }
                    }
                    was = now;
                    std::thread::sleep(Duration::from_millis(12));
                }
            })
            .ok();
        ComboWatcher { stop, combos, recording, recorded, thread }
    }

    pub fn set_combos(&self, combos: &[String]) {
        *self.combos.lock() = combos.iter().map(|c| parse_combo(c).unwrap_or_default()).collect();
    }

    /// Record the next combination pressed (`take_recorded` gets it).
    pub fn record(&self, on: bool) {
        *self.recorded.lock() = None;
        self.recording.store(on, Ordering::Relaxed);
    }

    pub fn take_recorded(&self) -> Option<String> {
        self.recorded.lock().take()
    }
}

impl Drop for ComboWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_read_and_say() {
        use device_query::Keycode;
        assert_eq!(parse_combo("LControl+Key1"), Some(vec![Keycode::LControl, Keycode::Key1]));
        assert_eq!(parse_combo("F13"), Some(vec![Keycode::F13]));
        assert_eq!(parse_combo("LControl+Nope"), None);
        assert_eq!(combo_label("LControl+Key1"), "Left Ctrl + 1");
        assert_eq!(combo_label("LAlt+Numpad3"), "Left Alt + Num 3");
        assert_eq!(combo_label("F13"), "F13");
        // Keys you type with need a modifier; others work alone.
        for typing in ["A", "Key1", "Space", "Enter", "Slash", "Backspace", "Up"] {
            assert!(needs_modifier(typing), "{typing}");
        }
        for fine in ["F8", "Numpad7", "PageUp", "LControl+A", "LShift+Key1", "RAlt+Space"] {
            assert!(!needs_modifier(fine), "{fine}");
        }
    }
}
