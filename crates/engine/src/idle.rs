//! Idle, like Discord: after five minutes without a key or the mouse anywhere on the PC (and
//! nothing said in a call), Ingle tells your servers you're idle, and everyone sees a moon on
//! your picture instead of the green dot. Touching anything brings the green dot back. You can
//! also say you're idle yourself (`Engine::set_idle_by_hand`), and then it stays until you
//! change it. Do Not Disturb (`Engine::set_dnd`) is told to servers the same way.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::Duration;

use ingle_proto::control::{DND_FEATURE, IDLE_FEATURE, Request};

use crate::Shared;
use crate::state::ConnStatus;

/// How long without input before you're idle (Discord's is commonly given as five minutes too).
pub const IDLE_AFTER: Duration = Duration::from_secs(5 * 60);

/// How often Ingle looks (and at the latest how soon a server hears you're back).
pub const CHECK_EVERY: Duration = Duration::from_secs(5);

/// How long since the last key press or mouse move anywhere on this PC, when the system says.
#[cfg(windows)]
pub fn since_last_input() -> Option<Duration> {
    use windows::Win32::System::SystemInformation::GetTickCount;
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    // SAFETY: `info` is a LASTINPUTINFO with its size filled in, as the call wants.
    if !unsafe { GetLastInputInfo(&mut info) }.as_bool() {
        return None;
    }
    // Both are milliseconds since Windows started, and wrap every 49.7 days together.
    let now = unsafe { GetTickCount() };
    Some(Duration::from_millis(u64::from(now.wrapping_sub(info.dwTime))))
}

/// Under X11 on Linux: the screensaver extension's idle time (what screen lockers go by).
/// Asked on the idle thread only, which keeps its own connection to the X server.
#[cfg(target_os = "linux")]
pub fn since_last_input() -> Option<Duration> {
    use x11rb::connection::Connection as _;
    use x11rb::protocol::screensaver::ConnectionExt as _;
    type X = Option<(x11rb::rust_connection::RustConnection, u32)>;
    thread_local! {
        static CONN: std::cell::RefCell<Option<X>> = const { std::cell::RefCell::new(None) };
    }
    CONN.with(|cell| {
        let mut cell = cell.borrow_mut();
        let conn = cell.get_or_insert_with(|| {
            std::env::var_os("DISPLAY")?;
            let (c, screen) = x11rb::connect(None).ok()?;
            let root = c.setup().roots.get(screen)?.root;
            Some((c, root))
        });
        let (c, root) = conn.as_ref()?;
        let info = c.screensaver_query_info(*root).ok()?.reply().ok()?;
        Some(Duration::from_millis(u64::from(info.ms_since_user_input)))
    })
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn since_last_input() -> Option<Duration> {
    None
}

/// Whether you're idle: nothing on the PC (where the system says; otherwise in Ingle's own
/// window) for `IDLE_AFTER`, and nothing said in a call for as long. Each is how long ago.
pub fn idle_now(system: Option<Duration>, in_ingle: Duration, spoke: Option<Duration>) -> bool {
    let quiet = system.map_or(in_ingle, |s| s.min(in_ingle));
    let quiet = spoke.map_or(quiet, |s| s.min(quiet));
    quiet >= IDLE_AFTER
}

fn ago(now_ms: i64, then_ms: i64) -> Duration {
    Duration::from_millis(now_ms.saturating_sub(then_ms).max(0) as u64)
}

/// Starts the thread that looks every `CHECK_EVERY` (and when woken by `wake`).
pub(crate) fn start(shared: &Arc<Shared>) {
    let weak: Weak<Shared> = Arc::downgrade(shared);
    let thread = std::thread::Builder::new().name("ingle-idle".into()).spawn(move || {
        loop {
            match weak.upgrade() {
                Some(shared) => check(&shared),
                None => break,
            }
            std::thread::park_timeout(CHECK_EVERY);
        }
    });
    match thread {
        Ok(t) => *shared.idle_thread.lock() = Some(t.thread().clone()),
        Err(e) => tracing::warn!("couldn't start watching for idle: {e}"),
    }
}

/// Looks again now (a server just connected, or you changed it by hand).
pub(crate) fn wake(shared: &Shared) {
    if let Some(t) = shared.idle_thread.lock().as_ref() {
        t.unpark();
    }
}

/// Works out whether you're idle, shows it, and tells every server that doesn't know yet.
fn check(shared: &Arc<Shared>) {
    let now = ingle_proto::now_ms();
    let in_ingle = ago(now, shared.last_activity_ms.load(Ordering::Relaxed));
    let spoke = shared.call.lock().as_ref().and_then(|c| c.audio.as_ref()).map(|a| a.controls.last_spoke_ms.load(Ordering::Relaxed)).filter(|t| *t > 0).map(|t| ago(now, t));
    let (by_hand, dnd) = {
        let st = shared.state.read_recursive();
        (st.settings.idle_by_hand, st.settings.dnd)
    };
    let idle = by_hand || idle_now(since_last_input(), in_ingle, spoke);
    let (mut tell, mut tell_dnd) = (Vec::new(), Vec::new());
    let changed = {
        let mut st = shared.state.write();
        let changed = st.my_idle != idle;
        st.my_idle = idle;
        for s in st.servers.iter_mut().filter(|s| s.status == ConnStatus::Connected) {
            if s.has(IDLE_FEATURE) && s.idle_sent != idle {
                s.idle_sent = idle;
                tell.push(s.key.clone());
            }
            // (Do Not Disturb goes the same way: each server hears when it changes.)
            if s.has(DND_FEATURE) && s.dnd_sent != dnd {
                s.dnd_sent = dnd;
                tell_dnd.push(s.key.clone());
            }
        }
        changed
    };
    if !tell.is_empty() || !tell_dnd.is_empty() {
        let servers = shared.servers.lock();
        for key in &tell {
            if let Some(h) = servers.get(key) {
                h.notify(Request::SetIdle { idle });
            }
        }
        for key in &tell_dnd {
            if let Some(h) = servers.get(key) {
                h.notify(Request::SetDnd { dnd });
            }
        }
    }
    if changed {
        tracing::info!("{}", if idle { "idle" } else { "back from idle" });
        (shared.repaint)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn five_quiet_minutes_make_you_idle_and_anything_brings_you_back() {
        let s = Duration::from_secs;
        assert!(idle_now(Some(s(301)), s(600), None));
        assert!(!idle_now(Some(s(299)), s(600), None));
        // Using Ingle counts, whatever the system says.
        assert!(!idle_now(Some(s(900)), s(5), None));
        // So does talking in a call.
        assert!(!idle_now(Some(s(900)), s(900), Some(s(30))));
        assert!(idle_now(Some(s(900)), s(900), Some(s(400))));
        // Where the system can't say, Ingle's own window is what counts.
        assert!(idle_now(None, s(600), None));
        assert!(!idle_now(None, s(60), None));
        assert_eq!(ago(10_000, 4_000), s(6));
        assert_eq!(ago(4_000, 10_000), s(0), "a clock that went back is just now");
    }
}
