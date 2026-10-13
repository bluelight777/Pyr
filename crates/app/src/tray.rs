//! Ingle's icon by the clock (the Windows notification area, or "system tray").
//!
//! Click the icon to open Ingle; right-click it to mute, deafen, leave voice or quit. The
//! window's close button (and Alt+F4) never quits: it hides Ingle to the icon, so calls and
//! messages keep coming in, like Discord. Only the icon's Quit Ingle (or Settings → Quit)
//! ends it.
//!
//! egui only runs frames while the window is visible, so nothing here waits for one. The icon's
//! clicks go to a small hidden window of our own, whose window procedure runs on the UI thread
//! inside winit's message loop and shows the main window, talks to the engine or quits right
//! there. Later launches of ingle.exe find that hidden window by its name to bring Ingle back
//! ([`show_running`]) or, for the installer, to ask it to quit ([`quit_running`]).

use std::cell::RefCell;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use eframe::egui;
use ingle_engine::Engine;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{CloseHandle, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NIN_SELECT,
    NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, FindWindowW, GetCursorPos, GetSystemMetrics, GetWindowThreadProcessId, HICON, IMAGE_ICON, IsIconic, LR_DEFAULTCOLOR,
    LoadImageW, MENU_ITEM_FLAGS, MF_CHECKED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, PostMessageW, RegisterClassW,
    RegisterWindowMessageW, SM_CXSMICON, SM_CYSMICON, SM_MENUDROPALIGNMENT, SW_HIDE, SW_RESTORE, SW_SHOW, SetForegroundWindow,
    SetMenuDefaultItem, ShowWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTALIGN,
    TPM_RIGHTBUTTON, TrackPopupMenu, WM_APP, WM_CONTEXTMENU, WM_ENDSESSION, WM_MOUSEMOVE, WM_NULL, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::{PCWSTR, w};

/// Class of our hidden window. Its title names the config folder (see [`window_name`]).
const CLASS: PCWSTR = w!("IngleTray");
/// The icon's clicks. With NOTIFYICON_VERSION_4 the event is in the low word of lparam and
/// the icon's position in wparam.
const WM_TRAY: u32 = WM_APP + 1;
/// A later launch of ingle.exe wants the window.
const WM_INGLE_SHOW: u32 = WM_APP + 2;
/// `ingle.exe --quit` (the installer) wants us to quit.
const WM_INGLE_QUIT: u32 = WM_APP + 3;
/// The close button: hide the window once the current frame is done.
const WM_INGLE_HIDE: u32 = WM_APP + 4;
/// Enter or Space on the icon (NIN_SELECT | NINF_KEY).
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const ICON_ID: u32 = 1;

const CMD_OPEN: usize = 1;
const CMD_MUTE: usize = 2;
const CMD_DEAFEN: usize = 3;
const CMD_LEAVE: usize = 4;
const CMD_QUIT: usize = 5;

struct State {
    tray: HWND,
    main: HWND,
    engine: Arc<Engine>,
    ctx: egui::Context,
    icon: HICON,
    tip: String,
    taskbar_created: u32,
}

thread_local! {
    // Only touched on the UI thread: by the app and by our window procedure.
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// The icon is showing, so hiding the window is safe: there's a way back.
static SHOWN: AtomicBool = AtomicBool::new(false);

/// Reads the state without holding a borrow across Win32 calls (those can dispatch messages
/// that end up back here, or in an egui frame that asks [`is_active`]).
fn with_state<R>(f: impl FnOnce(&State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow().ok().and_then(|s| s.as_ref().map(f)))
}

/// The icon by the clock. It's removed when this is dropped.
pub struct Tray(());

impl Tray {
    /// Adds the icon for the app's main window. `config_dir` names this copy of Ingle (one per
    /// profile) so later launches can find it; `tip` is the icon's tooltip.
    pub fn create(cc: &eframe::CreationContext<'_>, engine: Arc<Engine>, config_dir: &Path, tip: &str) -> Option<Tray> {
        let main = match cc.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => HWND(h.hwnd.get() as *mut core::ffi::c_void),
            _ => {
                tracing::warn!("no window handle, so no tray icon");
                return None;
            }
        };
        let name = wide(&window_name(config_dir));
        let (tray, icon, taskbar_created) = unsafe {
            let instance = HINSTANCE(GetModuleHandleW(None).ok()?.0);
            let class = WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance, lpszClassName: CLASS, ..Default::default() };
            // Fails harmlessly when it's already registered (a retry with the other renderer).
            RegisterClassW(&class);
            // A hidden top-level window (not a message-only one): those don't get the
            // "TaskbarCreated" broadcast we need when Explorer restarts.
            let tray = match CreateWindowExW(
                WS_EX_TOOLWINDOW,
                CLASS,
                PCWSTR(name.as_ptr()),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance),
                None,
            ) {
                Ok(h) => h,
                Err(e) => {
                    tracing::warn!("couldn't create the tray window: {e}");
                    ingle_engine::errors::report(format!("Couldn't put Ingle by the clock (tray): {e}"));
                    return None;
                }
            };
            // Icon 1 in ingle.rc, at the size the tray uses (it follows the display scale).
            let (cx, cy) = (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON));
            let icon = LoadImageW(Some(instance), PCWSTR(1usize as *const u16), IMAGE_ICON, cx, cy, LR_DEFAULTCOLOR)
                .map(|h| HICON(h.0))
                .unwrap_or_default();
            (tray, icon, RegisterWindowMessageW(w!("TaskbarCreated")))
        };
        STATE.with(|s| {
            *s.borrow_mut() =
                Some(State { tray, main, engine, ctx: cc.egui_ctx.clone(), icon, tip: tip.to_string(), taskbar_created })
        });
        if add_icon() {
            tracing::info!("tray icon added");
        } else {
            // Often just Windows still starting (Ingle opened with it): the icon is added when
            // the taskbar announces itself ("TaskbarCreated").
            tracing::warn!("couldn't add the tray icon yet; the close button will quit Ingle until it's there");
        }
        Some(Tray(()))
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        remove_icon();
        if let Some(s) = STATE.with(|s| s.borrow_mut().take()) {
            unsafe {
                let _ = DestroyWindow(s.tray);
                if !s.icon.is_invalid() {
                    let _ = DestroyIcon(s.icon);
                }
            }
        }
    }
}

/// Whether the icon is showing (so the close button may hide the window).
pub fn is_active() -> bool {
    SHOWN.load(Ordering::Relaxed)
}

/// There's a tray window (the icon may still need adding, e.g. Explorer was restarting).
pub fn available() -> bool {
    with_state(|_| ()).is_some()
}

/// Makes sure the icon is showing, adding it again if it went missing. True if it's there.
pub fn ensure_icon() -> bool {
    SHOWN.load(Ordering::Relaxed) || add_icon()
}

/// A little notice from the icon (Windows shows it as a toast).
pub fn notify(title: &str, text: &str) {
    let Some(mut nid) = with_state(icon_data) else { return };
    nid.uFlags = NIF_INFO;
    nid.dwInfoFlags = NIIF_INFO;
    for (dst, src) in nid.szInfoTitle.iter_mut().zip(title.encode_utf16().take(63)) {
        *dst = src;
    }
    for (dst, src) in nid.szInfo.iter_mut().zip(text.encode_utf16().take(255)) {
        *dst = src;
    }
    unsafe {
        let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
    }
}

/// Quit Ingle for real (Settings → Quit Ingle): the same as the icon's Quit.
pub fn quit() -> ! {
    quit_now("Settings")
}

/// Take the icon away before the process exits by itself (an update quitting the app).
pub fn prepare_exit() {
    remove_icon();
}

/// Hide the window to the tray, once the current frame is done.
pub fn hide_main() {
    if let Some(tray) = with_state(|s| s.tray) {
        unsafe {
            let _ = PostMessageW(Some(tray), WM_INGLE_HIDE, WPARAM(0), LPARAM(0));
        }
    }
}

fn hide_main_now() {
    if let Some(main) = with_state(|s| s.main) {
        unsafe {
            let _ = ShowWindow(main, SW_HIDE);
        }
    }
}

fn show_main() {
    let Some((main, ctx)) = with_state(|s| (s.main, s.ctx.clone())) else { return };
    unsafe {
        let _ = ShowWindow(main, if IsIconic(main).as_bool() { SW_RESTORE } else { SW_SHOW });
        let _ = SetForegroundWindow(main);
        refresh_pointer(main);
    }
    ctx.request_repaint();
}

/// The window only hears about the pointer when it lands somewhere new (winit), and ignores
/// clicks until it knows where the pointer is (egui). So after hiding, a click on the very
/// pixel of the last one (the close button, with the window brought back from the keyboard)
/// would do nothing until the mouse moved. Tell the window afresh where the pointer is: first
/// somewhere it can't be, then where it really is.
unsafe fn refresh_pointer(main: HWND) {
    let mut p = POINT::default();
    unsafe {
        if GetCursorPos(&mut p).is_err() || !ScreenToClient(main, &mut p).as_bool() {
            return;
        }
        let at = |x: i32, y: i32| LPARAM((((y as u16 as u32) << 16) | x as u16 as u32) as isize);
        let _ = PostMessageW(Some(main), WM_MOUSEMOVE, WPARAM(0), at(-32000, -32000));
        let _ = PostMessageW(Some(main), WM_MOUSEMOVE, WPARAM(0), at(p.x, p.y));
    }
}

/// Quit without waiting for a frame (there are none while the window is hidden): what closing
/// the window does, then exit.
fn quit_now(from: &str) -> ! {
    tracing::info!("quitting ({from})");
    let state = with_state(|s| (s.main, s.engine.clone()));
    remove_icon();
    if let Some((main, engine)) = state {
        unsafe {
            let _ = ShowWindow(main, SW_HIDE);
        }
        // Leave voice, save settings and read markers, close server connections.
        engine.shutdown();
        // A moment for those goodbyes to go out before the process ends.
        std::thread::sleep(Duration::from_millis(300));
    }
    std::process::exit(0);
}

fn show_menu(tray: HWND, x: i32, y: i32) {
    let Some(engine) = with_state(|s| s.engine.clone()) else { return };
    let (muted, deafened) = engine.voice_prefs();
    let leave = {
        let st = engine.state();
        st.call.as_ref().map(|c| {
            match st.server(&c.server_key).and_then(|s| s.channel(c.channel_id)) {
                Some(ch) => format!("Leave voice ({})", ch.name),
                None => "Leave voice".to_string(),
            }
        })
    };
    let leave = leave.map(|s| wide(&s));
    let check = |on: bool| -> MENU_ITEM_FLAGS { if on { MF_CHECKED } else { MF_UNCHECKED } };
    let cmd = unsafe {
        let Ok(menu) = CreatePopupMenu() else { return };
        let _ = AppendMenuW(menu, MF_STRING, CMD_OPEN, w!("Open Ingle"));
        let _ = SetMenuDefaultItem(menu, CMD_OPEN as u32, 0);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING | check(muted), CMD_MUTE, w!("Mute"));
        let _ = AppendMenuW(menu, MF_STRING | check(deafened), CMD_DEAFEN, w!("Deafen"));
        if let Some(label) = &leave {
            let _ = AppendMenuW(menu, MF_STRING, CMD_LEAVE, PCWSTR(label.as_ptr()));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, CMD_QUIT, w!("Quit Ingle"));
        // Without this the menu doesn't go away when you click somewhere else.
        let _ = SetForegroundWindow(tray);
        let align = if GetSystemMetrics(SM_MENUDROPALIGNMENT) != 0 { TPM_RIGHTALIGN } else { TPM_LEFTALIGN };
        let flags = TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | align;
        let cmd = TrackPopupMenu(menu, flags, x, y, None, tray, None).0 as usize;
        let _ = PostMessageW(Some(tray), WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
        cmd
    };
    match cmd {
        CMD_OPEN => show_main(),
        CMD_MUTE => engine.set_mute(!muted),
        CMD_DEAFEN => engine.set_deafen(!deafened),
        CMD_LEAVE => engine.leave_voice(),
        CMD_QUIT => quit_now("tray icon"),
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            match (lparam.0 as u32) & 0xffff {
                NIN_SELECT | NIN_KEYSELECT => show_main(),
                WM_CONTEXTMENU => {
                    let x = (wparam.0 & 0xffff) as u16 as i16 as i32;
                    let y = ((wparam.0 >> 16) & 0xffff) as u16 as i16 as i32;
                    show_menu(hwnd, x, y);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_INGLE_SHOW => {
            show_main();
            LRESULT(0)
        }
        WM_INGLE_HIDE => {
            hide_main_now();
            LRESULT(0)
        }
        WM_INGLE_QUIT => quit_now("asked to by another launch, like the installer"),
        WM_ENDSESSION if wparam.0 != 0 => {
            // Windows is signing out or shutting down: save what quitting would save.
            if let Some(engine) = with_state(|s| s.engine.clone()) {
                engine.shutdown();
            }
            remove_icon();
            LRESULT(0)
        }
        _ if msg != 0 && with_state(|s| s.taskbar_created) == Some(msg) => {
            // Explorer restarted (or the taskbar was rebuilt) and dropped every icon: add ours
            // again.
            if SHOWN.load(Ordering::Relaxed) {
                remove_icon();
            }
            add_icon();
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn icon_data(s: &State) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: s.tray,
        uID: ICON_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: WM_TRAY,
        hIcon: s.icon,
        ..Default::default()
    };
    for (dst, src) in nid.szTip.iter_mut().zip(s.tip.encode_utf16().take(127)) {
        *dst = src;
    }
    nid
}

fn add_icon() -> bool {
    let Some(mut nid) = with_state(icon_data) else { return false };
    let ok = unsafe { Shell_NotifyIconW(NIM_ADD, &nid).as_bool() };
    if ok {
        nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
        unsafe {
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &nid);
        }
    }
    SHOWN.store(ok, Ordering::Relaxed);
    ok
}

fn remove_icon() {
    if SHOWN.swap(false, Ordering::Relaxed) {
        if let Some(nid) = with_state(icon_data) {
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &nid);
            }
        }
    }
}

/// Our hidden window's title: one per config folder, so per profile.
fn window_name(config_dir: &Path) -> String {
    // FNV-1a, which (unlike std's hasher) is the same in every build, so any version of
    // ingle.exe finds any other.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in config_dir.to_string_lossy().bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("Ingle tray {h:016x}")
}

/// The running copy's hidden window and process id.
fn find_running(config_dir: &Path) -> Option<(HWND, u32)> {
    let name = wide(&window_name(config_dir));
    unsafe {
        let hwnd = FindWindowW(CLASS, PCWSTR(name.as_ptr())).ok()?;
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32));
        (pid != 0 && pid != std::process::id()).then_some((hwnd, pid))
    }
}

/// A later launch (the Start menu, an invite link): bring the running copy's window back, even
/// from the tray. Returns false if no running copy has a tray window (an older version).
pub fn show_running(config_dir: &Path) -> bool {
    let Some((hwnd, pid)) = find_running(config_dir) else { return false };
    unsafe {
        // The person just started us, so we may let the running copy come to the front.
        let _ = AllowSetForegroundWindow(pid);
        PostMessageW(Some(hwnd), WM_INGLE_SHOW, WPARAM(0), LPARAM(0)).is_ok()
    }
}

/// `ingle.exe --quit`, which the installer runs before replacing files: ask the running copy
/// to quit, and wait until it has (at most 10 seconds). Returns false if none was found.
pub fn quit_running(config_dir: &Path) -> bool {
    let Some((hwnd, pid)) = find_running(config_dir) else { return false };
    unsafe {
        let process = OpenProcess(PROCESS_SYNCHRONIZE, false, pid).ok();
        let sent = PostMessageW(Some(hwnd), WM_INGLE_QUIT, WPARAM(0), LPARAM(0)).is_ok();
        if let Some(p) = process {
            if sent {
                WaitForSingleObject(p, 10_000);
            }
            let _ = CloseHandle(p);
        }
        sent
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_name_is_stable_and_per_folder() {
        let a = window_name(Path::new(r"C:\Users\me\AppData\Roaming\Ingle\config"));
        assert_eq!(a, window_name(Path::new(r"C:\Users\me\AppData\Roaming\Ingle\config")));
        assert_ne!(a, window_name(Path::new(r"C:\Users\me\AppData\Roaming\Ingle\config\profiles\b")));
        assert!(a.starts_with("Ingle tray ") && a.len() == "Ingle tray ".len() + 16);
    }
}
