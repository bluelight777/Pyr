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
            .name("pyr-ptt".into())
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

/// Shortcuts that work anywhere: the keys that play soundboard sounds and the mute shortcut
/// (0.13.20), key combinations ("LControl+Key1") polled like push-to-talk, so they work in
/// games without taking the keys away from them. `fire(i)` is called when combination `i` goes
/// down (all its keys held, at least one just now), unless a longer one going off with it holds
/// it (Ctrl + M inside Ctrl + Shift + M), or it's a key you type with on its own and Pyr's own
/// message box has the keys (`typing`): there it just types. It can also record the next
/// combination pressed, for setting one.
pub struct ComboWatcher {
    stop: Arc<AtomicBool>,
    combos: Arc<Mutex<Vec<Vec<device_query::Keycode>>>>,
    recording: Arc<AtomicBool>,
    recorded: Arc<Mutex<Option<String>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Which of `combos` go off when the keys `now` are held, `new` of them just pressed: every one
/// whose keys are all held, one of them just now, except one inside a longer one that's going
/// off too, and a key you type with on its own while Pyr's own message box has the keys.
pub fn going_off(combos: &[Vec<device_query::Keycode>], now: &[device_query::Keycode], new: &[device_query::Keycode], typing: bool) -> Vec<usize> {
    let held: Vec<usize> = (0..combos.len()).filter(|&i| !combos[i].is_empty() && combos[i].iter().all(|k| now.contains(k)) && combos[i].iter().any(|k| new.contains(k))).collect();
    held.iter()
        .copied()
        .filter(|&i| {
            let c = &combos[i];
            let inside_longer = held.iter().any(|&j| combos[j].len() > c.len() && c.iter().all(|k| combos[j].contains(k)));
            let typed = typing && types_alone(c);
            !inside_longer && !typed
        })
        .collect()
}

/// Whether keys are one you type with, on its own (a letter, a digit, Space, Enter…), not a
/// function key, the number pad or Insert/Home/End/Page Up/Page Down, and nothing held with it.
fn types_alone(keys: &[device_query::Keycode]) -> bool {
    use device_query::Keycode::*;
    if keys.is_empty() || keys.iter().any(is_modifier) {
        return false;
    }
    !keys.iter().all(|k| {
        matches!(
            k,
            F1 | F2 | F3 | F4 | F5 | F6 | F7 | F8 | F9 | F10 | F11 | F12 | F13 | F14 | F15 | F16 | F17 | F18 | F19 | F20 | Insert | Home | End | PageUp | PageDown
        ) || k.to_string().starts_with("Numpad")
    })
}

/// What a combination does in most apps (Copy, Paste…), when it's one of those: a shortcut that
/// works anywhere can't go on it, as it would go off every time (0.13.28's list).
pub fn taken_by_apps(combo: &str) -> Option<&'static str> {
    use device_query::Keycode::*;
    let keys = parse_combo(combo)?;
    let mods: Vec<_> = keys.iter().filter(|k| is_modifier(k)).collect();
    let rest: Vec<_> = keys.iter().filter(|k| !is_modifier(k)).collect();
    let [m] = mods[..] else { return None };
    let [key] = rest[..] else { return None };
    match (m, key) {
        (LControl | RControl, C) => Some("Copy"),
        (LControl | RControl, X) => Some("Cut"),
        (LControl | RControl, V) => Some("Paste"),
        (LControl | RControl, A) => Some("Select all"),
        (LControl | RControl, Z) => Some("Undo"),
        (LControl | RControl, Y) => Some("Redo"),
        (LControl | RControl, S) => Some("Save"),
        (LControl | RControl, F) => Some("Find"),
        (LAlt | RAlt, Tab) => Some("Switch windows"),
        (LAlt | RAlt, F4) => Some("Close the window"),
        _ => None,
    }
}

/// Whether a recorded combination is Esc pressed to stop recording (Esc on its own, with Alt
/// or Shift too; with Ctrl it's a shortcut, as 0.13.28 has it).
pub fn is_cancel(combo: &str) -> bool {
    use device_query::Keycode::*;
    parse_combo(combo).is_some_and(|keys| keys.contains(&Escape) && !keys.iter().any(|k| matches!(k, LControl | RControl)))
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
/// Enter…). Since 0.13.22 it's allowed, as in Discord: in Pyr's own message boxes it still
/// types, and anywhere else it goes off (and the settings say so). Function keys, the number
/// pad and Insert/Home/End/Page Up/Page Down work alone without a word.
pub fn needs_modifier(combo: &str) -> bool {
    parse_combo(combo).is_some_and(|keys| types_alone(&keys))
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
    pub fn start(combos: &[String], fire: Arc<dyn Fn(usize) + Send + Sync>, on_recorded: Arc<dyn Fn() + Send + Sync>, typing: Arc<dyn Fn() -> bool + Send + Sync>) -> ComboWatcher {
        let stop = Arc::new(AtomicBool::new(false));
        let combos = Arc::new(Mutex::new(combos.iter().map(|c| parse_combo(c).unwrap_or_default()).collect::<Vec<_>>()));
        let recording = Arc::new(AtomicBool::new(false));
        let recorded = Arc::new(Mutex::new(None));
        let (st, cs, rec, out) = (stop.clone(), combos.clone(), recording.clone(), recorded.clone());
        let thread = std::thread::Builder::new()
            .name("pyr-sound-keys".into())
            .spawn(move || {
                use device_query::{DeviceQuery, DeviceState};
                let Some(state) = DeviceState::checked_new() else {
                    tracing::warn!("the mute shortcut and soundboard keys are unavailable: can't read keyboard state");
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
                            let fired = going_off(&cs.lock(), &now, &new, typing());
                            for i in fired {
                                fire(i);
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
        // Keys you type with, on their own (they type in Pyr's message boxes); others don't.
        for typing in ["A", "Key1", "Space", "Enter", "Slash", "Backspace", "Up"] {
            assert!(needs_modifier(typing), "{typing}");
        }
        for fine in ["F8", "Numpad7", "PageUp", "LControl+A", "LShift+Key1", "RAlt+Space"] {
            assert!(!needs_modifier(fine), "{fine}");
        }
    }

    /// 0.13.28's: Copy, Paste and the like are taken (with exactly their modifier); a longer
    /// combination holds a shorter one inside it; a single key types in Pyr's message boxes.
    #[test]
    fn shortcuts_that_work_anywhere_as_0_13_28s() {
        use device_query::Keycode::*;
        for (combo, what) in [
            ("LControl+C", "Copy"),
            ("LControl+X", "Cut"),
            ("RControl+V", "Paste"),
            ("LControl+A", "Select all"),
            ("LControl+Z", "Undo"),
            ("LControl+Y", "Redo"),
            ("LControl+S", "Save"),
            ("LControl+F", "Find"),
            ("LAlt+Tab", "Switch windows"),
            ("LAlt+F4", "Close the window"),
        ] {
            assert_eq!(taken_by_apps(combo), Some(what), "{combo}");
        }
        for free in ["LControl+LShift+V", "LControl+W", "LControl+N", "LControl+Tab", "LShift+Insert", "LControl+Escape", "C", "LAlt+C"] {
            assert_eq!(taken_by_apps(free), None, "{free}");
        }
        assert!(is_cancel("Escape") && is_cancel("LAlt+Escape") && !is_cancel("LControl+Escape") && !is_cancel("M"));
        let combos = vec![vec![LControl, M], vec![LControl, LShift, M], vec![M], vec![F9]];
        // Ctrl + Shift + M: the longer one only (Ctrl + M and M are inside it).
        assert_eq!(going_off(&combos, &[LControl, LShift, M], &[M], false), vec![1]);
        // Ctrl + M: it, not M inside it.
        assert_eq!(going_off(&combos, &[LControl, M], &[M], false), vec![0]);
        // Shift + M (nothing longer going off): M, extra keys or not.
        assert_eq!(going_off(&combos, &[LShift, M], &[M], false), vec![2]);
        // M typed in Pyr's message box: it types; F9 there still goes off.
        assert!(going_off(&combos, &[M], &[M], true).is_empty());
        assert_eq!(going_off(&combos, &[F9], &[F9], true), vec![3]);
        // Keys held from before don't go off again.
        assert!(going_off(&combos, &[LControl, M], &[], false).is_empty());
    }
}
