//! Idle, like Discord: after five minutes without a key or the mouse anywhere on the PC (and
//! nothing said in a call), you're away: Pyr tells your servers you're idle, and everyone sees a
//! moon on your picture instead of the green dot. Touching anything brings the green dot back.
//! You can also say you're idle yourself (`Engine::set_idle_by_hand`), and then it stays until
//! you change it. Do Not Disturb (`Engine::set_dnd`) is told to servers the same way.
//!
//! Servers since 0.13.28 also hear whether you're away (`SetIdle::away`), apart from the moon:
//! only being away counts for a server's AFK channel, never Idle set by hand. And Pyr only goes
//! along with a move to the AFK channel when it has seen you away that long itself
//! (`afk_move_ok`), since an older server counts Idle set by hand as away.

use std::sync::atomic::Ordering;
use std::sync::{Arc, Weak};
use std::time::Duration;

use pyr_proto::control::{AWAY_AFTER_SECS, AWAY_FEATURE, DND_FEATURE, IDLE_FEATURE, Request};

use crate::Shared;
use crate::state::ConnStatus;

/// How long without input before you're away, and idle (Discord's is commonly given as five
/// minutes too).
pub const IDLE_AFTER: Duration = Duration::from_secs(AWAY_AFTER_SECS);

/// How often Pyr looks (and at the latest how soon a server hears you're back).
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

/// How long it's been quiet: nothing on the PC (where the system says; otherwise in Pyr's own
/// window), and nothing said in a call. Each is how long ago.
pub fn quiet(system: Option<Duration>, in_pyr: Duration, spoke: Option<Duration>) -> Duration {
    let quiet = system.map_or(in_pyr, |s| s.min(in_pyr));
    spoke.map_or(quiet, |s| s.min(quiet))
}

/// Whether you're away (and idle): quiet (see `quiet`) for `IDLE_AFTER`.
pub fn idle_now(system: Option<Duration>, in_pyr: Duration, spoke: Option<Duration>) -> bool {
    quiet(system, in_pyr, spoke) >= IDLE_AFTER
}

/// Whether to go along with a server moving me to its AFK channel after `mins` minutes away:
/// only when it's been that quiet here (`quiet`), give or take a minute for the server's clock
/// and Pyr's, and never before Pyr would say I'm away at all. (A server from before 0.13.28
/// counts Idle set by hand as away, and could move someone who's there.)
pub fn afk_move_ok(quiet: Duration, mins: u32) -> bool {
    let need = Duration::from_secs(u64::from(mins) * 60).max(IDLE_AFTER);
    quiet + Duration::from_secs(60) >= need
}

fn ago(now_ms: i64, then_ms: i64) -> Duration {
    Duration::from_millis(now_ms.saturating_sub(then_ms).max(0) as u64)
}

/// How long ago I last said something in a call (None: not in one, or not since it began).
fn spoke_ago(shared: &Shared, now_ms: i64) -> Option<Duration> {
    let call = shared.call.lock();
    let last = call.as_ref().and_then(|c| c.audio.as_ref()).map(|a| a.controls.last_spoke_ms.load(Ordering::Relaxed));
    last.filter(|t| *t > 0).map(|t| ago(now_ms, t))
}

/// How long the PC's been without a key or the mouse, as the system says (None: it can't).
fn system_quiet(shared: &Shared, now_ms: i64) -> Option<Duration> {
    match shared.pretend_input_ms.load(Ordering::Relaxed) {
        0 => since_last_input(),
        t => Some(ago(now_ms, t)),
    }
}

/// How long it's been quiet here now (see `quiet`; for `afk_move_ok`): the PC as the system
/// said at the idle thread's last look (a few seconds ago at most), Pyr's window and calls as
/// they are now. (Takes the call's lock: not while holding the state's.)
pub(crate) fn quiet_now(shared: &Shared) -> Duration {
    let now = pyr_proto::now_ms();
    let system = match shared.last_input_ms.load(Ordering::Relaxed) {
        0 => None,
        t => Some(ago(now, t)),
    };
    quiet(system, ago(now, shared.last_activity_ms.load(Ordering::Relaxed)), spoke_ago(shared, now))
}

/// Starts the thread that looks every `CHECK_EVERY` (and when woken by `wake`).
pub(crate) fn start(shared: &Arc<Shared>) {
    let weak: Weak<Shared> = Arc::downgrade(shared);
    let thread = std::thread::Builder::new().name("pyr-idle".into()).spawn(move || {
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

/// Works out whether you're away and idle, shows it, and tells every server that doesn't know
/// yet.
fn check(shared: &Arc<Shared>) {
    let now = pyr_proto::now_ms();
    let system = system_quiet(shared, now);
    // (Kept for `quiet_now`, which mustn't wait on the system.)
    let last_input = system.map_or(0, |d| now - d.as_millis() as i64);
    shared.last_input_ms.store(last_input, Ordering::Relaxed);
    let in_pyr = ago(now, shared.last_activity_ms.load(Ordering::Relaxed));
    let spoke = spoke_ago(shared, now);
    let (by_hand, dnd) = {
        let st = shared.state.read_recursive();
        (st.settings.idle_by_hand, st.settings.dnd)
    };
    let away = idle_now(system, in_pyr, spoke);
    let idle = by_hand || away;
    let (mut tell, mut tell_dnd) = (Vec::new(), Vec::new());
    let changed = {
        let mut st = shared.state.write();
        let changed = st.my_idle != idle || st.my_away != away;
        st.my_idle = idle;
        st.my_away = away;
        for s in st.servers.iter_mut().filter(|s| s.status == ConnStatus::Connected) {
            // (Away only to servers that know it: an older one hears idle alone, as before.)
            let with_away = s.has(AWAY_FEATURE);
            if s.has(IDLE_FEATURE) && (s.idle_sent != idle || (with_away && s.away_sent != away)) {
                s.idle_sent = idle;
                s.away_sent = away;
                tell.push((s.key.clone(), with_away.then_some(away)));
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
        for (key, away) in tell {
            if let Some(h) = servers.get(&key) {
                h.notify(Request::SetIdle { idle, away });
            }
        }
        for key in &tell_dnd {
            if let Some(h) = servers.get(key) {
                h.notify(Request::SetDnd { dnd });
            }
        }
    }
    if changed {
        tracing::info!("{}", match (idle, away) {
            (true, true) => "away (idle)",
            (true, false) => "idle (set by hand), not away",
            _ => "back from idle",
        });
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
        // Using Pyr counts, whatever the system says.
        assert!(!idle_now(Some(s(900)), s(5), None));
        // So does talking in a call.
        assert!(!idle_now(Some(s(900)), s(900), Some(s(30))));
        assert!(idle_now(Some(s(900)), s(900), Some(s(400))));
        // Where the system can't say, Pyr's own window is what counts.
        assert!(idle_now(None, s(600), None));
        assert!(!idle_now(None, s(60), None));
        assert_eq!(ago(10_000, 4_000), s(6));
        assert_eq!(ago(4_000, 10_000), s(0), "a clock that went back is just now");
    }

    /// (0.13.28) Pyr goes along with a server's move to its AFK channel only when it has been
    /// that quiet here itself: an older server moved people whose status was Idle by hand.
    #[test]
    fn a_move_to_afk_needs_that_long_quiet_here() {
        let m = |m: u64| Duration::from_secs(m * 60);
        assert!(afk_move_ok(m(61), 60));
        assert!(afk_move_ok(m(59) + Duration::from_secs(30), 60), "a minute's give for the clocks");
        assert!(!afk_move_ok(m(10), 60), "Idle by hand, but here ten minutes ago");
        assert!(!afk_move_ok(Duration::from_secs(3), 60), "just came into the channel");
        assert!(afk_move_ok(m(5), 5));
        // Not knowing the server's minutes, never before away at all.
        assert!(!afk_move_ok(m(2), 0));
        assert!(afk_move_ok(m(5), 0));
        // Quiet is the PC, Pyr's window and talking, whichever was last.
        assert_eq!(quiet(Some(m(90)), m(70), Some(m(80))), m(70));
        assert_eq!(quiet(None, m(70), None), m(70));
        assert_eq!(quiet(Some(m(2)), m(70), None), m(2), "busy in another program");
    }
}
