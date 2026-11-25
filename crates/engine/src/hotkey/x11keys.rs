//! The keys and mouse buttons held, as X11 sees them (Linux), for push to talk, the mute shortcut
//! and the soundboard's keys (0.16.7). Blue, Nov 25: "while Prog is connected to Pyropoly, his Pyr
//! loses connection however Pyropoly keeps working [...] Pyr is silently crashing for him" (Omarchy,
//! a Wayland desktop with X11's XWayland beside it). Two ways the readers before this could end Pyr
//! without a word, whatever program was in front:
//!
//! - `device_query` reads the keys with Xlib's `XQueryKeymap` into an array that is gone by the
//!   time it's written (`[0; 32].as_mut_ptr()`: a temporary, dropped at the end of its line).
//!   Rust's rules say that's undefined; an optimized build is free to have put something else
//!   there, and it was done 200 times a second, from two threads.
//! - Xlib ends the whole program (`exit`) when its connection to the X server breaks: XWayland
//!   going away under a Wayland desktop took Pyr with it, a call and all.
//!
//! Here x11rb (Rust, no Xlib) asks the same question into memory of its own, and a broken
//! connection is an error: no keys held, and Pyr connects again a few seconds later. (The key
//! numbers as `device_query` maps them: X11's keycode is the kernel's plus 8.)

use std::time::{Duration, Instant};

use device_query::Keycode;
use x11rb::connection::Connection as _;
use x11rb::protocol::xinput::ConnectionExt as _;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

/// How long to wait before connecting again after the X server went away (or wasn't there).
const AGAIN: Duration = Duration::from_secs(3);

struct Conn {
    c: RustConnection,
    root: u32,
    /// The pointer device (XInput 2), for the mouse's side buttons; None without XInput 2.
    pointer: Option<u16>,
}

/// One connection to the X server (when there is one), connected again when it breaks.
pub struct X11Input {
    conn: Option<Conn>,
    next_try: Instant,
    /// Said in the log that it went: once, until it's back.
    lost: bool,
}

impl X11Input {
    /// Connects now, if there's an X server to connect to.
    pub fn new() -> X11Input {
        let mut x = X11Input { conn: None, next_try: Instant::now(), lost: false };
        x.conn = connect();
        x
    }

    /// Whether it's connected to an X server now.
    pub fn here(&self) -> bool {
        self.conn.is_some()
    }

    /// The connection, connecting again first when it's time to (only where an X server's set,
    /// `$DISPLAY`).
    fn conn(&mut self) -> Option<&Conn> {
        if self.conn.is_none() && Instant::now() >= self.next_try && std::env::var_os("DISPLAY").is_some() {
            self.next_try = Instant::now() + AGAIN;
            self.conn = connect();
            if self.conn.is_some() && self.lost {
                self.lost = false;
                tracing::info!("the keys: connected to the X server again");
            }
        }
        self.conn.as_ref()
    }

    fn broke(&mut self, what: &str) {
        if !self.lost {
            self.lost = true;
            tracing::warn!("the keys: lost the X server ({what}); trying again every few seconds");
        }
        self.conn = None;
        self.next_try = Instant::now() + AGAIN;
    }

    /// The keys held now (none when there's no X server to ask).
    pub fn keys(&mut self) -> Vec<Keycode> {
        let Some(c) = self.conn() else { return Vec::new() };
        match c.c.query_keymap().map_err(|e| e.to_string()).and_then(|k| k.reply().map_err(|e| e.to_string())) {
            Ok(r) => held(&r.keys),
            Err(e) => {
                self.broke(&e);
                Vec::new()
            }
        }
    }

    /// Whether mouse button `button` is held (X11's numbering: 8 back, 9 forward), through
    /// XInput 2 (the core protocol only knows 1 to 5, and 4 and 5 are the wheel).
    pub fn button(&mut self, button: usize) -> bool {
        let Some(c) = self.conn() else { return false };
        let Some(pointer) = c.pointer else { return false };
        match c.c.xinput_xi_query_pointer(c.root, pointer).map_err(|e| e.to_string()).and_then(|k| k.reply().map_err(|e| e.to_string())) {
            Ok(r) => r.buttons.get(button / 32).is_some_and(|w| w & (1 << (button % 32)) != 0),
            Err(e) => {
                self.broke(&e);
                false
            }
        }
    }
}

impl Default for X11Input {
    fn default() -> Self {
        X11Input::new()
    }
}

/// The keys held at this moment (a connection of its own, for a one-off question: the paste
/// key). None held when there's no X server.
pub fn keys_now() -> Vec<Keycode> {
    X11Input::new().keys()
}

fn connect() -> Option<Conn> {
    std::env::var_os("DISPLAY")?;
    let (c, screen) = x11rb::connect(None).ok()?;
    let root = c.setup().roots.get(screen)?.root;
    let pointer = match c.xinput_xi_query_version(2, 0).ok().and_then(|v| v.reply().ok()) {
        Some(v) if v.major_version >= 2 => {
            Some(c.xinput_xi_get_client_pointer(x11rb::NONE).ok().and_then(|r| r.reply().ok()).map(|r| r.deviceid).filter(|d| *d != 0).unwrap_or(2))
        }
        _ => None,
    };
    Some(Conn { c, root, pointer })
}

/// The keys a keymap (32 bytes, a bit a keycode) says are held.
pub fn held(keymap: &[u8]) -> Vec<Keycode> {
    let mut out = Vec::new();
    for (i, byte) in keymap.iter().enumerate().take(32) {
        for bit in 0..8u8 {
            if byte & (1 << bit) != 0
                && let Some(k) = keycode(i * 8 + bit as usize)
            {
                out.push(k);
            }
        }
    }
    out
}

/// X11's keycode as `device_query`'s key (the kernel's code plus 8), for the keys it knows.
pub fn keycode(x11: usize) -> Option<Keycode> {
    use Keycode::*;
    Some(match x11.checked_sub(8)? {
        1 => Escape,
        2 => Key1,
        3 => Key2,
        4 => Key3,
        5 => Key4,
        6 => Key5,
        7 => Key6,
        8 => Key7,
        9 => Key8,
        10 => Key9,
        11 => Key0,
        12 => Minus,
        13 => Equal,
        14 => Backspace,
        15 => Tab,
        16 => Q,
        17 => W,
        18 => E,
        19 => R,
        20 => T,
        21 => Y,
        22 => U,
        23 => I,
        24 => O,
        25 => P,
        26 => LeftBracket,
        27 => RightBracket,
        28 => Enter,
        29 => LControl,
        30 => A,
        31 => S,
        32 => D,
        33 => F,
        34 => G,
        35 => H,
        36 => J,
        37 => K,
        38 => L,
        39 => Semicolon,
        40 => Apostrophe,
        41 => Grave,
        42 => LShift,
        43 => BackSlash,
        44 => Z,
        45 => X,
        46 => C,
        47 => V,
        48 => B,
        49 => N,
        50 => M,
        51 => Comma,
        52 => Dot,
        53 => Slash,
        54 => RShift,
        55 => NumpadMultiply,
        56 => LAlt,
        57 => Space,
        58 => CapsLock,
        59 => F1,
        60 => F2,
        61 => F3,
        62 => F4,
        63 => F5,
        64 => F6,
        65 => F7,
        66 => F8,
        67 => F9,
        68 => F10,
        71 => Numpad7,
        72 => Numpad8,
        73 => Numpad9,
        74 => NumpadSubtract,
        75 => Numpad4,
        76 => Numpad5,
        77 => Numpad6,
        78 => NumpadAdd,
        79 => Numpad1,
        80 => Numpad2,
        81 => Numpad3,
        82 => Numpad0,
        83 => NumpadDecimal,
        87 => F11,
        88 => F12,
        96 => NumpadEnter,
        97 => RControl,
        98 => NumpadDivide,
        100 => RAlt,
        102 => Home,
        103 => Up,
        104 => PageUp,
        105 => Left,
        106 => Right,
        107 => End,
        108 => Down,
        109 => PageDown,
        110 => Insert,
        111 => Delete,
        117 => NumpadEquals,
        125 => LMeta,
        126 => RMeta,
        183 => F13,
        184 => F14,
        185 => F15,
        186 => F16,
        187 => F17,
        188 => F18,
        189 => F19,
        190 => F20,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The keys as `device_query` named them, a bit a key in X11's keymap: the push to talk keys
    /// offered, the mute shortcut's and the soundboard's (letters, digits, the number pad).
    #[test]
    fn a_keymap_reads_as_device_query_read_it() {
        let mut map = [0u8; 32];
        // (X11 keycodes: the kernel's plus 8.)
        for kernel in [41usize, 57, 29, 47, 2, 71, 183, 58] {
            let x = kernel + 8;
            map[x / 8] |= 1 << (x % 8);
        }
        let mut keys: Vec<String> = held(&map).iter().map(|k| k.to_string()).collect();
        keys.sort();
        assert_eq!(keys, ["CapsLock", "F13", "Grave", "Key1", "LControl", "Numpad7", "Space", "V"]);
        // Nothing held, and the codes below 8 (none is a key) never read as one.
        assert!(held(&[0u8; 32]).is_empty());
        assert!(held(&[0xffu8, 0, 0, 0]).is_empty());
        assert!(keycode(0).is_none() && keycode(7).is_none());
        // Every key Pyr offers for push to talk that's a key (not a mouse button) has a code.
        let known: Vec<String> = (8..256).filter_map(keycode).map(|k| k.to_string()).collect();
        for k in crate::hotkey::KEY_CHOICES.iter().filter(|k| !k.starts_with("Mouse")) {
            assert!(known.iter().any(|n| n == k), "{k}");
        }
        assert_eq!(known.len(), 108);
    }

    /// No X server: no keys and no buttons, and nothing ends (it tries again later).
    #[test]
    fn no_x_server_is_no_keys() {
        let mut x = X11Input { conn: None, next_try: Instant::now() + Duration::from_secs(3600), lost: false };
        assert!(!x.here());
        assert!(x.keys().is_empty());
        assert!(!x.button(8));
    }
}
