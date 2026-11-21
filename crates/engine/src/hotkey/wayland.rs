//! Push to talk on Wayland (Linux). Blue, Nov 20 2026: "The push to talk keys don't really work
//! on my friends version of linux, Omarchy." Omarchy is Hyprland, a Wayland desktop. On Wayland a
//! program only sees keys while its own window has them, and the way `PttWatcher` reads keys (X11)
//! sees them only while an X11 program has them (a game through Proton, say): in Pyr's own window
//! and in every Wayland program, push to talk never went down. So on Wayland:
//!
//! - Pyr asks the desktop for a global shortcut, "Push to talk (Pyr)" (the XDG GlobalShortcuts
//!   portal), and hears it go down and come back up. KDE and GNOME ask you which key (Pyr suggests
//!   yours). Hyprland's portal only takes keys from Hyprland's binds, so there Pyr adds the bind
//!   itself while push to talk's on (`hyprctl keyword`): one that leaves the key to your game too
//!   (`n`), whatever else is held (`i`, Shift to run, say) and even when a game asks for every key
//!   (`p`). Hyprland forgets binds made that way when its config's read again, so then Pyr makes
//!   it again; and takes it away when push to talk's off or Pyr closes.
//! - In Pyr's own window the key works anyway: the app says when it's held (`PttWatcher::window`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;

/// Whether this is a Wayland desktop.
pub fn is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t.eq_ignore_ascii_case("wayland"))
}

/// The shortcut's id with the desktop: Pyr's own (Hyprland names it `<app id>:<id>`, and two
/// programs asking for the same name end Hyprland's portal), and each profile's its own.
pub fn shortcut_id(profile: Option<&str>) -> String {
    match profile.map(|p| p.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>()).filter(|p| !p.is_empty()) {
        Some(p) => format!("pyr-push-to-talk-{p}"),
        None => "pyr-push-to-talk".into(),
    }
}

/// What the desktop does with push to talk, for the settings to say.
#[derive(Clone, Debug, PartialEq)]
pub enum Note {
    /// Push to talk's off: not asked.
    Idle,
    /// Asking the desktop.
    Asking,
    /// The desktop has the shortcut (KDE, GNOME): what it says sets it off, if it says.
    Desktop(String),
    /// Hyprland: bound by Pyr, to this key (Hyprland's name for it).
    Hyprland(String),
    /// No shortcut from the desktop: why.
    Without(String),
}

struct Inner {
    /// The key (Pyr's name for it) while push to talk's on.
    want: Option<String>,
    asked: bool,
    note: Note,
    /// Hyprland: the shortcut's whole name once it's there, and the key bound to it now.
    hypr_name: Option<String>,
    hypr_bound: Option<String>,
}

pub struct WaylandPtt {
    /// Held: the desktop's shortcut is down.
    pub down: Arc<AtomicBool>,
    inner: Arc<Mutex<Inner>>,
    rt: tokio::runtime::Handle,
    id: String,
    on_change: Arc<dyn Fn() + Send + Sync>,
}

impl WaylandPtt {
    pub fn new(rt: tokio::runtime::Handle, profile: Option<&str>, on_change: Arc<dyn Fn() + Send + Sync>) -> WaylandPtt {
        let inner = Inner { want: None, asked: false, note: Note::Idle, hypr_name: None, hypr_bound: None };
        WaylandPtt { down: Arc::new(AtomicBool::new(false)), inner: Arc::new(Mutex::new(inner)), rt, id: shortcut_id(profile), on_change }
    }

    /// Push to talk on this key (Pyr's name for it), or off (`None`). The first time it's on, the
    /// desktop's asked for the shortcut.
    pub fn set(&self, key: Option<&str>) {
        let start = {
            let mut g = self.inner.lock();
            g.want = key.map(str::to_string);
            let start = key.is_some() && !g.asked;
            if start {
                g.asked = true;
                g.note = Note::Asking;
            }
            start
        };
        if start {
            let (inner, down, id, on_change) = (self.inner.clone(), self.down.clone(), self.id.clone(), self.on_change.clone());
            let trigger = key.and_then(trigger);
            self.rt.spawn(async move { portal(inner, down, id, trigger, on_change).await });
        } else {
            let inner = self.inner.clone();
            let on_change = self.on_change.clone();
            self.rt.spawn_blocking(move || {
                rebind(&inner, &hyprland::Ipc::here());
                on_change();
            });
        }
    }

    pub fn note(&self) -> Note {
        self.inner.lock().note.clone()
    }
}

impl Drop for WaylandPtt {
    /// Pyr closing: the bind it made goes (left behind it would do nothing, as the shortcut's
    /// gone with Pyr, but it's not Pyr's to leave).
    fn drop(&mut self) {
        let mut g = self.inner.lock();
        if let (Some(old), Some(ipc)) = (g.hypr_bound.take(), hyprland::Ipc::here()) {
            let _ = ipc.request(&format!("keyword unbind ,{old}"));
        }
    }
}

/// The key as the XDG shortcuts spec names it, for the desktop to suggest (KDE, GNOME): not the
/// mouse's buttons or a modifier on its own, which they don't take.
pub fn trigger(key: &str) -> Option<String> {
    Some(match key {
        "Grave" => "grave".into(),
        "CapsLock" => "Caps_Lock".into(),
        "Space" => "space".into(),
        "Tab" => "Tab".into(),
        "V" | "B" | "T" => key.to_lowercase(),
        k if k.starts_with('F') && k[1..].parse::<u8>().is_ok() => k.into(),
        _ => return None,
    })
}

/// The key as Hyprland's binds name it (its name in the keyboard's layout, or the mouse button's
/// code: BTN_SIDE, back, is 275 and BTN_EXTRA, forward, 276).
pub fn hypr_key(key: &str) -> Option<String> {
    Some(match key {
        "Grave" => "grave".into(),
        "CapsLock" => "Caps_Lock".into(),
        "LControl" => "Control_L".into(),
        "RControl" => "Control_R".into(),
        "LAlt" => "Alt_L".into(),
        "RAlt" => "Alt_R".into(),
        "LShift" => "Shift_L".into(),
        "RShift" => "Shift_R".into(),
        "Space" => "space".into(),
        "Tab" => "Tab".into(),
        "Mouse4" => "mouse:275".into(),
        "Mouse5" => "mouse:276".into(),
        "V" | "B" | "T" => key.into(),
        k if k.starts_with('F') && k[1..].parse::<u8>().is_ok() => k.into(),
        _ => return None,
    })
}

/// The bind Pyr asks Hyprland for, the best first: `n` leaves the key to the program in front too,
/// `i` goes off whatever else is held, `p` even when a program asks for every key (a game, a
/// remote desktop). An older Hyprland that doesn't know a flag says so, and the next is tried.
pub const BIND_FLAGS: [&str; 3] = ["bindnip", "bindni", "bindn"];

/// Bring Hyprland's bind into line with what's wanted: the key bound to the shortcut while push to
/// talk's on, none while it's off. (One at a time; the state's not held while Hyprland's asked.)
fn rebind(inner: &Mutex<Inner>, ipc: &Option<hyprland::Ipc>) {
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let Some(ipc) = ipc else { return };
    let (name, want, bound) = {
        let g = inner.lock();
        (g.hypr_name.clone(), g.want.as_deref().and_then(hypr_key), g.hypr_bound.clone())
    };
    let Some(name) = name else { return };
    if bound == want {
        return;
    }
    if let Some(old) = bound {
        match ipc.request(&format!("keyword unbind ,{old}")) {
            Ok(r) if r.trim() == "ok" => {}
            r => tracing::info!(reply = ?r, "push to talk: Hyprland didn't take the old key's bind away"),
        }
        inner.lock().hypr_bound = None;
    }
    let Some(key) = want else {
        inner.lock().note = Note::Idle;
        return;
    };
    let mut why = String::new();
    for flags in BIND_FLAGS {
        match ipc.request(&format!("keyword {flags} ,{key},global,{name}")) {
            Ok(r) if r.trim() == "ok" => {
                tracing::info!(%flags, %key, %name, "push to talk: Hyprland binds it");
                let mut g = inner.lock();
                g.hypr_bound = Some(key.clone());
                g.note = Note::Hyprland(key);
                return;
            }
            Ok(r) => why = r.trim().to_string(),
            Err(e) => why = e.to_string(),
        }
    }
    tracing::warn!(%key, %why, "push to talk: Hyprland wouldn't bind it");
    inner.lock().note = Note::Without(format!("Hyprland wouldn't bind {key}: {why}"));
}

/// What a portal signal does to the key: Activated holds it, Deactivated lets go, and only for
/// Pyr's own shortcut; anything else leaves it as it is.
fn edge(member: Option<&str>, which: &str, ours: &str) -> Option<bool> {
    if which != ours {
        return None;
    }
    match member {
        Some("Activated") => Some(true),
        Some("Deactivated") => Some(false),
        _ => None,
    }
}

async fn portal(inner: Arc<Mutex<Inner>>, down: Arc<AtomicBool>, id: String, trigger: Option<String>, on_change: Arc<dyn Fn() + Send + Sync>) {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    use futures_util::StreamExt;
    // (`why` for the settings to say; what went wrong, for pyr.log.)
    let without = |why: &str, detail: String| {
        tracing::info!(%why, %detail, "push to talk: no shortcut from the desktop (the key works in Pyr's own window)");
        inner.lock().note = Note::Without(why.to_string());
        on_change();
    };
    const NONE: &str = "your desktop has no global shortcuts for programs";
    // (A name of its own with the desktop however Pyr was started, so KDE keeps the key you gave
    // it. An older portal can't: then it goes by how Pyr was started.)
    if let Ok(app) = ashpd::AppID::try_from(pyr_proto::DESKTOP_ID)
        && let Err(e) = ashpd::register_host_app(app).await
    {
        tracing::debug!("push to talk: the portal didn't take Pyr's app id: {e}");
    }
    let gs = match GlobalShortcuts::new().await {
        Ok(g) => g,
        Err(e) => return without(NONE, e.to_string()),
    };
    let session = match gs.create_session().await {
        Ok(s) => s,
        Err(e) => return without(NONE, e.to_string()),
    };
    // (Listening first: a key held as it's bound isn't missed.) One ordered stream for both
    // edges: with Activated and Deactivated on two streams, a quick tap that has both waiting
    // when this wakes could be read release-first, leaving the key held (an open mic). And the
    // portal going away (restarted, say) lets go of the key.
    let (Ok(events), Ok(owner)) = (gs.receive_all_signals().await, gs.receive_owner_changed().await) else {
        return without(NONE, "couldn't listen to its shortcuts".into());
    };
    let (mut events, mut owner) = (Box::pin(events), Box::pin(owner));
    let mut shortcut = NewShortcut::new(id.as_str(), "Push to talk (Pyr)");
    if let Some(t) = &trigger {
        shortcut = shortcut.preferred_trigger(t.as_str());
    }
    let mut says = String::new();
    match gs.bind_shortcuts(&session, &[shortcut], None).await.and_then(|r| r.response()) {
        Ok(bound) => {
            if let Some(s) = bound.shortcuts().iter().find(|s| s.id() == id) {
                says = s.trigger_description().to_string();
            }
        }
        Err(ashpd::Error::Response(ashpd::desktop::ResponseError::Cancelled)) => return without("you said no to Pyr's shortcut", "cancelled".into()),
        // (What some desktops answer doesn't read as the portal's spec has it; the shortcut's
        // there all the same.)
        Err(e) => tracing::info!("push to talk: the desktop's answer to binding it: {e}"),
    }
    let ipc = hyprland::Ipc::here();
    if let Some(ipc) = &ipc {
        // Hyprland: the shortcut's whole name (its app id goes by how Pyr was started), once it's
        // there (the portal tells Hyprland a moment later).
        let mut name = None;
        for _ in 0..20 {
            let i = ipc.clone();
            let id = id.clone();
            name = tokio::task::spawn_blocking(move || i.shortcut_name(&id)).await.ok().flatten();
            if name.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        match name {
            Some(name) => {
                inner.lock().hypr_name = Some(name);
                let (i2, ipc2) = (inner.clone(), Some(ipc.clone()));
                let _ = tokio::task::spawn_blocking(move || rebind(&i2, &ipc2)).await;
                // Hyprland's config read again: its binds are the config's again.
                let (i3, ipc3, oc) = (inner.clone(), ipc.clone(), on_change.clone());
                let _ = std::thread::Builder::new().name("pyr-ptt-hyprland".into()).spawn(move || {
                    ipc3.on_reload(|| {
                        tracing::info!("push to talk: Hyprland read its config again: binding it again");
                        i3.lock().hypr_bound = None;
                        rebind(&i3, &Some(ipc3.clone()));
                        oc();
                    })
                });
            }
            None => {
                tracing::warn!("push to talk: Hyprland didn't list Pyr's shortcut (hyprctl globalshortcuts)");
                inner.lock().note = Note::Without("Hyprland didn't take Pyr's shortcut".into());
            }
        }
    } else {
        inner.lock().note = Note::Desktop(says);
    }
    on_change();
    // (The portal sends a session's shortcuts to whoever has the session only; the id's Pyr's.)
    loop {
        tokio::select! {
            _ = owner.next() => {
                tracing::info!("push to talk: the desktop's portal restarted or went away");
                break;
            }
            event = events.next() => {
                let Some(event) = event else { break };
                let header = event.header();
                let member = header.member().map(|m| m.as_str().to_string());
                // (Activated and Deactivated: session path, shortcut id, timestamp, options.)
                let Ok((_session, which, _at, _options)) = event
                    .body()
                    .deserialize::<(ashpd::zvariant::OwnedObjectPath, String, u64, std::collections::HashMap<String, ashpd::zvariant::OwnedValue>)>()
                else {
                    continue;
                };
                if let Some(held) = edge(member.as_deref(), &which, &id) {
                    down.store(held, Ordering::Relaxed);
                    on_change();
                }
            }
        }
    }
    // (The session's kept until then: closed, the desktop drops the shortcut.) Let go of the
    // key first, so nothing ending here leaves the mic open.
    down.store(false, Ordering::Relaxed);
    on_change();
    let _ = tokio::time::timeout(Duration::from_secs(1), session.close()).await;
    tracing::info!("push to talk: the desktop's shortcuts went away");
}

/// Hyprland's own socket (what `hyprctl` talks to).
pub mod hyprland {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;
    use std::time::Duration;

    #[derive(Clone, Debug)]
    pub struct Ipc {
        pub dir: PathBuf,
    }

    impl Ipc {
        /// The Hyprland this runs in, if it is one.
        pub fn here() -> Option<Ipc> {
            let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok().filter(|s| !s.is_empty() && !s.contains('/'))?;
            let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
            // (Since Hyprland 0.40 under the runtime folder, before it under /tmp.)
            [runtime.map(|r| r.join("hypr").join(&sig)), Some(PathBuf::from("/tmp/hypr").join(&sig))]
                .into_iter()
                .flatten()
                .find(|d| d.join(".socket.sock").exists())
                .map(|dir| Ipc { dir })
        }

        /// One request (as `hyprctl` sends it: "keyword …", "globalshortcuts") and what Hyprland
        /// says back.
        pub fn request(&self, what: &str) -> std::io::Result<String> {
            let mut s = UnixStream::connect(self.dir.join(".socket.sock"))?;
            s.set_read_timeout(Some(Duration::from_secs(2)))?;
            s.set_write_timeout(Some(Duration::from_secs(2)))?;
            s.write_all(what.as_bytes())?;
            let mut out = String::new();
            s.read_to_string(&mut out)?;
            Ok(out)
        }

        /// The whole name Hyprland has for the shortcut `id` (`<app id>:<id>`), once a program's
        /// registered it.
        pub fn shortcut_name(&self, id: &str) -> Option<String> {
            shortcut_name_in(&self.request("globalshortcuts").ok()?, id)
        }

        /// Calls `again` each time Hyprland reads its config again, while Hyprland's there.
        pub fn on_reload(&self, mut again: impl FnMut()) {
            let Ok(s) = UnixStream::connect(self.dir.join(".socket2.sock")) else { return };
            for line in BufReader::new(s).lines() {
                let Ok(line) = line else { return };
                if line.starts_with("configreloaded>>") {
                    again();
                }
            }
        }
    }

    /// From `hyprctl globalshortcuts` ("<app id>:<id> -> <description>" a line).
    pub fn shortcut_name_in(list: &str, id: &str) -> Option<String> {
        list.lines()
            .filter_map(|l| l.split(" -> ").next())
            .map(str::trim)
            .find(|name| name.rsplit_once(':').is_some_and(|(_, i)| i == id))
            .map(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signals_in_order_hold_and_let_go_of_our_key_only() {
        // A quick tap arrives as Activated then Deactivated on the one ordered stream: read in
        // that order, the key ends up let go (two streams could hand them over the other way).
        let mut held = false;
        for (member, which) in [("Activated", "pyr-push-to-talk"), ("Deactivated", "pyr-push-to-talk")] {
            if let Some(h) = edge(Some(member), which, "pyr-push-to-talk") {
                held = h;
            }
        }
        assert!(!held);
        assert_eq!(edge(Some("Activated"), "someone-else", "pyr-push-to-talk"), None);
        assert_eq!(edge(Some("ShortcutsChanged"), "pyr-push-to-talk", "pyr-push-to-talk"), None);
        assert_eq!(edge(None, "pyr-push-to-talk", "pyr-push-to-talk"), None);
    }

    #[test]
    fn keys_as_hyprland_and_the_desktop_name_them() {
        assert_eq!(hypr_key("Mouse4").as_deref(), Some("mouse:275"));
        assert_eq!(hypr_key("Mouse5").as_deref(), Some("mouse:276"));
        assert_eq!(hypr_key("Grave").as_deref(), Some("grave"));
        assert_eq!(hypr_key("LControl").as_deref(), Some("Control_L"));
        assert_eq!(hypr_key("F13").as_deref(), Some("F13"));
        assert_eq!(hypr_key("V").as_deref(), Some("V"));
        assert_eq!(hypr_key("Frobnicate"), None);
        // Every key offered has a name in both places it's needed (the mouse's and the modifiers
        // only Hyprland's).
        for k in crate::hotkey::KEY_CHOICES {
            assert!(hypr_key(k).is_some(), "{k}");
            let plain = !k.starts_with("Mouse") && !["LControl", "RControl", "LAlt", "RAlt", "LShift", "RShift"].contains(k);
            assert_eq!(trigger(k).is_some(), plain, "{k}");
        }
        assert_eq!(trigger("V").as_deref(), Some("v"));
        assert_eq!(trigger("Space").as_deref(), Some("space"));
    }

    #[test]
    fn ids_of_their_own() {
        assert_eq!(shortcut_id(None), "pyr-push-to-talk");
        assert_eq!(shortcut_id(Some("test 2")), "pyr-push-to-talk-test2");
        assert_eq!(shortcut_id(Some("//")), "pyr-push-to-talk");
    }

    #[test]
    fn hyprlands_list_read() {
        let list = "com.obsproject.Studio:record -> Record\ncom.firelitgames.pyr:pyr-push-to-talk -> Push to talk (Pyr)\n:pyr-push-to-talk-x -> Push to talk (Pyr)\n";
        assert_eq!(hyprland::shortcut_name_in(list, "pyr-push-to-talk").as_deref(), Some("com.firelitgames.pyr:pyr-push-to-talk"));
        assert_eq!(hyprland::shortcut_name_in(list, "pyr-push-to-talk-x").as_deref(), Some(":pyr-push-to-talk-x"));
        assert_eq!(hyprland::shortcut_name_in("none", "pyr-push-to-talk"), None);
        assert_eq!(hyprland::shortcut_name_in(list, "record").as_deref(), Some("com.obsproject.Studio:record"));
    }

    /// Against a stand-in for Hyprland's socket: the bind's made with the best flags it takes,
    /// moved when the key changes, taken away when push to talk's off.
    #[test]
    fn hyprland_binds_it_and_takes_it_away() {
        use std::io::{Read, Write};
        use std::os::unix::net::UnixListener;
        let dir = std::env::temp_dir().join(format!("pyr-hypr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let listener = UnixListener::bind(dir.join(".socket.sock")).unwrap();
        let heard = Arc::new(Mutex::new(Vec::<String>::new()));
        let h = heard.clone();
        std::thread::spawn(move || {
            for s in listener.incoming() {
                let Ok(mut s) = s else { return };
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                // (An older Hyprland: no `p`.)
                let reply = if req.starts_with("keyword bindnip") { "Invalid bind flags".to_string() } else { "ok".to_string() };
                h.lock().push(req);
                let _ = s.write_all(reply.as_bytes());
            }
        });
        let ipc = Some(hyprland::Ipc { dir: dir.clone() });
        let inner = Mutex::new(Inner { want: Some("Mouse4".into()), asked: true, note: Note::Asking, hypr_name: Some(":pyr-push-to-talk".into()), hypr_bound: None });
        rebind(&inner, &ipc);
        assert_eq!(inner.lock().note, Note::Hyprland("mouse:275".into()));
        inner.lock().want = Some("F13".into());
        rebind(&inner, &ipc);
        rebind(&inner, &ipc);
        inner.lock().want = None;
        rebind(&inner, &ipc);
        assert_eq!(inner.lock().note, Note::Idle);
        assert_eq!(
            *heard.lock(),
            [
                "keyword bindnip ,mouse:275,global,:pyr-push-to-talk",
                "keyword bindni ,mouse:275,global,:pyr-push-to-talk",
                "keyword unbind ,mouse:275",
                "keyword bindnip ,F13,global,:pyr-push-to-talk",
                "keyword bindni ,F13,global,:pyr-push-to-talk",
                "keyword unbind ,F13",
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
