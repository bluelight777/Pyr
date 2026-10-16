//! Ingle's icon by the clock (the Windows notification area, or "system tray").
//!
//! Click the icon to open Ingle; right-click it to mute, deafen, leave voice, check for
//! updates (and update, when there's one), or quit. The
//! window's close button (and Alt+F4) never quits: it hides Ingle to the icon, so calls and
//! messages keep coming in, like Discord. Only the icon's Quit Ingle (or Settings → Quit)
//! ends it.
//!
//! Unread messages show here too: a dot on the icon (red for mentions and direct messages,
//! light for the rest) with where they are in its tooltip, and the same dot on the taskbar
//! button while the window is open.
//!
//! egui only runs frames while the window is visible, so nothing here waits for one. The icon's
//! clicks go to a small hidden window of our own, whose window procedure runs on the UI thread
//! inside winit's message loop and shows the main window, talks to the engine or quits right
//! there. Later launches of ingle.exe find that hidden window by its name to bring Ingle back
//! ([`show_running`]) or, for the installer, to ask it to quit ([`quit_running`]).

use std::cell::RefCell;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::Duration;

use eframe::egui;
use ingle_engine::Engine;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{CloseHandle, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateDIBSection, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, GetObjectW,
    HBITMAP, HGDIOBJ, ReleaseDC, ScreenToClient,
};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
use windows::Win32::UI::Shell::{
    ITaskbarList3, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NIM_SETVERSION,
    NIN_BALLOONUSERCLICK, NIN_SELECT, NOTIFYICON_VERSION_4, NOTIFYICONDATAW, Shell_NotifyIconW, TaskbarList,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyIcon, DestroyMenu,
    DestroyWindow, FLASHW_ALL, FLASHW_TIMERNOFG, FLASHWINFO, FindWindowW, FlashWindowEx, GetCursorPos, GetForegroundWindow, GetIconInfo, GetSystemMetrics, GetWindowThreadProcessId, HICON, ICONINFO,
    IMAGE_ICON, IsIconic, IsWindowVisible, LR_DEFAULTCOLOR,
    LoadImageW, MENU_ITEM_FLAGS, MF_CHECKED, MF_GRAYED, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, PostMessageW, RegisterClassW,
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
/// What's unread changed (see `watch_unread`): show it on the icon and the taskbar button.
const WM_INGLE_UNREAD: u32 = WM_APP + 5;
/// A check for updates asked for from the icon's menu finished: say what it found.
const WM_INGLE_UPDATE: u32 = WM_APP + 6;
/// Someone's calling (or a call was missed) while Ingle isn't in front: say so (see
/// `watch_unread`).
const WM_INGLE_CALL: u32 = WM_APP + 7;
/// Enter or Space on the icon (NIN_SELECT | NINF_KEY).
const NIN_KEYSELECT: u32 = NIN_SELECT | 1;
const ICON_ID: u32 = 1;

const CMD_OPEN: usize = 1;
const CMD_MUTE: usize = 2;
const CMD_DEAFEN: usize = 3;
const CMD_LEAVE: usize = 4;
const CMD_QUIT: usize = 5;
const CMD_UPDATE: usize = 6;
const CMD_CHECK: usize = 7;

struct State {
    tray: HWND,
    main: HWND,
    engine: Arc<Engine>,
    updater: crate::update::Updater,
    ctx: egui::Context,
    icon: HICON,
    tip: String,
    taskbar_created: u32,
    /// What the icon shows now: the plain icon, or one with a dot, and the tooltip.
    shown: (HICON, String),
    /// The icon with a light dot, and with a red one; and the taskbar's dots (made when
    /// first needed).
    dotted: [Option<HICON>; 2],
    taskbar_dots: [Option<HICON>; 2],
    /// The taskbar, for its button's dot (`None` if Windows wouldn't give it to us).
    taskbar: Option<Option<ITaskbarList3>>,
}

thread_local! {
    // Only touched on the UI thread: by the app and by our window procedure.
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Our hidden window, for the unread watcher's thread to wake.
static TRAY: AtomicIsize = AtomicIsize::new(0);
/// What's unread, as the watcher last saw it.
static UNREAD: std::sync::Mutex<Option<crate::unread::Unread>> = std::sync::Mutex::new(None);
/// A call to tell about with a Windows notice: (title, text), and whether to flash the taskbar
/// button too (someone's calling).
static CALL_NOTE: std::sync::Mutex<Option<(String, String, bool)>> = std::sync::Mutex::new(None);

/// The icon is showing, so hiding the window is safe: there's a way back.
static SHOWN: AtomicBool = AtomicBool::new(false);

/// The main window, for [`window_in_front`] (which the engine's threads call, so it can't use
/// `STATE`).
static MAIN: AtomicIsize = AtomicIsize::new(0);

/// The main window, once it exists (for the hang recorder's thread).
pub fn main_window() -> Option<HWND> {
    let h = MAIN.load(Ordering::Relaxed);
    (h != 0).then(|| HWND(h as *mut core::ffi::c_void))
}

/// Whether Ingle's window is in front of you: the foreground window, not minimized or hidden in
/// the tray. `None` until there's a window. egui runs no frames while the window is hidden or
/// minimized, so the engine asks this rather than trusting what the last frame saw.
pub fn window_in_front() -> Option<bool> {
    let h = MAIN.load(Ordering::Relaxed);
    if h == 0 {
        return None;
    }
    let main = HWND(h as *mut core::ffi::c_void);
    // Plain reads of window state: fine from any thread. A program open inside Ingle that has
    // the keyboard counts as Ingle.
    unsafe {
        let front = GetForegroundWindow();
        let ours = front == main || crate::plugins::programs::is_embedded(front.0 as isize);
        Some(ours && IsWindowVisible(main).as_bool() && !IsIconic(main).as_bool())
    }
}

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
    pub fn create(
        cc: &eframe::CreationContext<'_>,
        engine: Arc<Engine>,
        updater: crate::update::Updater,
        config_dir: &Path,
        tip: &str,
    ) -> Option<Tray> {
        let main = match cc.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Win32(h)) => HWND(h.hwnd.get() as *mut core::ffi::c_void),
            _ => {
                tracing::warn!("no window handle, so no tray icon");
                return None;
            }
        };
        MAIN.store(main.0 as isize, Ordering::Relaxed);
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
            *s.borrow_mut() = Some(State {
                tray,
                main,
                engine: engine.clone(),
                updater,
                ctx: cc.egui_ctx.clone(),
                icon,
                tip: tip.to_string(),
                taskbar_created,
                shown: (icon, tip.to_string()),
                dotted: [None; 2],
                taskbar_dots: [None; 2],
                taskbar: None,
            })
        });
        TRAY.store(tray.0 as isize, Ordering::Relaxed);
        watch_unread(engine);
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
        TRAY.store(0, Ordering::Relaxed);
        if let Some(s) = STATE.with(|s| s.borrow_mut().take()) {
            unsafe {
                let _ = DestroyWindow(s.tray);
                for icon in [Some(s.icon)].into_iter().chain(s.dotted).chain(s.taskbar_dots).flatten() {
                    if !icon.is_invalid() {
                        let _ = DestroyIcon(icon);
                    }
                }
            }
        }
    }
}

/// Looks at what's unread every so often (the window runs no frames while it's hidden, so it
/// can't be the one to notice) and has the window procedure show it when it changes, or when
/// the window is back on the taskbar (its button comes without a dot).
fn watch_unread(engine: Arc<Engine>) {
    let _ = std::thread::Builder::new().name("ingle-unread".into()).spawn(move || {
        let mut last: Option<crate::unread::Unread> = None;
        let mut visible = false;
        let mut again = 0u8;
        // Calls ringing that were told about already, and how many were missed by then.
        let mut rang: Vec<(String, i64, std::time::Instant)> = Vec::new();
        let mut missed_told = engine.state().missed_calls;
        loop {
            std::thread::sleep(Duration::from_millis(700));
            let tray = TRAY.load(Ordering::Relaxed);
            if tray == 0 {
                continue;
            }
            // Someone calling, or a call missed, while Ingle's window isn't in front (it rings
            // all the same: see the engine's `ring`): a Windows notice, and the taskbar button
            // flashes for a call.
            let note = {
                let st = engine.state();
                let mut note = None;
                for c in &st.incoming_calls {
                    if !rang.iter().any(|r| r.0 == c.server_key && r.1 == c.channel_id && r.2 == c.since) {
                        let who = st.server(&c.server_key).map(|s| s.member_name(&c.from)).unwrap_or_default();
                        let what = if c.video { "A video call. Open Ingle to answer." } else { "Open Ingle to answer." };
                        note = Some((format!("{who} is calling you"), what.to_string(), true));
                    }
                }
                rang = st.incoming_calls.iter().map(|c| (c.server_key.clone(), c.channel_id, c.since)).collect();
                if st.missed_calls > missed_told {
                    missed_told = st.missed_calls;
                    note = note.or_else(|| Some((format!("Missed call from {}", st.last_missed_from), "They called while you were away.".to_string(), false)));
                }
                note
            };
            if note.is_some() && window_in_front() != Some(true) {
                *CALL_NOTE.lock().unwrap_or_else(|e| e.into_inner()) = note;
                unsafe {
                    let _ = PostMessageW(Some(HWND(tray as *mut core::ffi::c_void)), WM_INGLE_CALL, WPARAM(0), LPARAM(0));
                }
            }
            let now = crate::unread::Unread::of(&engine.state());
            let shown = main_window().is_some_and(|h| unsafe { IsWindowVisible(h).as_bool() });
            if shown && !visible {
                // Explorer takes a moment to make the button: put the dot on a few times.
                again = 3;
            }
            visible = shown;
            if last.as_ref() != Some(&now) || again > 0 {
                again = again.saturating_sub(1);
                *UNREAD.lock().unwrap_or_else(|e| e.into_inner()) = Some(now.clone());
                last = Some(now);
                unsafe {
                    let _ = PostMessageW(Some(HWND(tray as *mut core::ffi::c_void)), WM_INGLE_UNREAD, WPARAM(0), LPARAM(0));
                }
            }
        }
    });
}

/// Show what's unread on the icon (its dot and tooltip) and on the taskbar button (UI thread).
fn show_unread() {
    let Some(u) = UNREAD.lock().unwrap_or_else(|e| e.into_inner()).clone() else { return };
    // 0: nothing, 1: unread messages, 2: mentions or direct messages.
    let level = if u.pings > 0 {
        2
    } else if u.any() {
        1
    } else {
        0
    };
    let what = if u.pings > 0 {
        format!("{} mention{} or direct message{}", u.pings, if u.pings == 1 { "" } else { "s" }, if u.pings == 1 { "" } else { "s" })
    } else {
        "Unread messages".to_string()
    };
    // Only GDI in here, which sends no window messages, so the state can stay borrowed.
    let todo = STATE.with(|cell| {
        let mut b = cell.try_borrow_mut().ok()?;
        let s = b.as_mut()?;
        let icon = match level {
            0 => s.icon,
            _ => *s.dotted[level - 1].get_or_insert_with(|| unsafe { dotted_icon(s.icon, level == 2) }.unwrap_or(s.icon)),
        };
        s.shown = (icon, u.tooltip(&s.tip));
        let nid = icon_data(s);
        let dot = match level {
            0 => None,
            _ => {
                let (cx, cy) = unsafe { (GetSystemMetrics(SM_CXSMICON), GetSystemMetrics(SM_CYSMICON)) };
                Some(*s.taskbar_dots[level - 1].get_or_insert_with(|| unsafe { dot_icon(cx, cy, level == 2) }.unwrap_or_default()))
            }
        };
        let taskbar = s.taskbar.get_or_insert_with(taskbar_list).clone();
        Some((nid, s.main, dot, taskbar))
    });
    let Some((mut nid, main, dot, taskbar)) = todo else { return };
    unsafe {
        if SHOWN.load(Ordering::Relaxed) {
            nid.uFlags = NIF_ICON | NIF_TIP | NIF_SHOWTIP;
            let _ = Shell_NotifyIconW(NIM_MODIFY, &nid);
        }
        if let Some(t) = taskbar {
            let text = wide(&what);
            let _ = match dot.filter(|d| !d.is_invalid()) {
                Some(d) => t.SetOverlayIcon(main, d, PCWSTR(text.as_ptr())),
                None => t.SetOverlayIcon(main, HICON::default(), PCWSTR::null()),
            };
        }
    }
}

/// The taskbar's interface for putting a dot on our button.
fn taskbar_list() -> Option<ITaskbarList3> {
    unsafe {
        // Usually done already on this thread (by winit); harmless if so.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let t: ITaskbarList3 = CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| tracing::warn!("no taskbar badge: {e}"))
            .ok()?;
        t.HrInit().map_err(|e| tracing::warn!("no taskbar badge: {e}")).ok()?;
        Some(t)
    }
}

/// An icon's pixels: BGRA with straight alpha, top row first.
unsafe fn icon_pixels(icon: HICON) -> Option<(Vec<u8>, usize, usize)> {
    unsafe {
        let mut ii = ICONINFO::default();
        GetIconInfo(icon, &mut ii).ok()?;
        let read = |bmp: HBITMAP| -> Option<(Vec<u8>, usize, usize)> {
            let mut bm = BITMAP::default();
            if GetObjectW(HGDIOBJ(bmp.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut BITMAP as *mut _)) == 0 {
                return None;
            }
            let (w, h) = (bm.bmWidth, bm.bmHeight);
            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut px = vec![0u8; (w * h * 4) as usize];
            let dc = GetDC(None);
            let lines = GetDIBits(dc, bmp, 0, h as u32, Some(px.as_mut_ptr().cast()), &mut bmi, DIB_RGB_COLORS);
            ReleaseDC(None, dc);
            (lines == h).then_some((px, w as usize, h as usize))
        };
        let color = if ii.hbmColor.is_invalid() { None } else { read(ii.hbmColor) };
        let mask = read(ii.hbmMask);
        let _ = DeleteObject(HGDIOBJ(ii.hbmColor.0));
        let _ = DeleteObject(HGDIOBJ(ii.hbmMask.0));
        let (mut px, w, h) = color?;
        // An icon without alpha says what's see-through in its mask instead.
        if px.chunks(4).all(|p| p[3] == 0) {
            let (m, mw, mh) = mask?;
            if (mw, mh) != (w, h) {
                return None;
            }
            for (p, q) in px.chunks_mut(4).zip(m.chunks(4)) {
                p[3] = if q[0] == 0 { 255 } else { 0 };
            }
        }
        Some((px, w, h))
    }
}

/// An icon from BGRA pixels (straight alpha, top row first).
unsafe fn icon_from_pixels(px: &[u8], w: usize, h: usize) -> Option<HICON> {
    unsafe {
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let dc = GetDC(None);
        let color = CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0);
        ReleaseDC(None, dc);
        let color = color.ok()?;
        if bits.is_null() {
            let _ = DeleteObject(HGDIOBJ(color.0));
            return None;
        }
        std::ptr::copy_nonoverlapping(px.as_ptr(), bits.cast::<u8>(), px.len().min(w * h * 4));
        // The colors carry their own alpha; the mask just has to be there.
        let mask_bits = vec![0u8; w.div_ceil(16) * 2 * h];
        let mask = CreateBitmap(w as i32, h as i32, 1, 1, Some(mask_bits.as_ptr().cast()));
        let ii = ICONINFO { fIcon: true.into(), xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&ii);
        let _ = DeleteObject(HGDIOBJ(color.0));
        let _ = DeleteObject(HGDIOBJ(mask.0));
        icon.ok()
    }
}

/// The tray icon with a dot in its bottom right corner.
unsafe fn dotted_icon(icon: HICON, red: bool) -> Option<HICON> {
    unsafe {
        let (mut px, w, h) = icon_pixels(icon)?;
        let unit = w as f32 / 16.0;
        let (r, ring) = (3.4 * unit, 1.0 * unit);
        let c = (w as f32 - r - ring, h as f32 - r - ring);
        crate::unread::paint_dot(&mut px, w, h, c, r, ring, if red { crate::unread::RED } else { crate::unread::LIGHT });
        icon_from_pixels(&px, w, h)
    }
}

/// Just a dot, for the taskbar button's corner.
unsafe fn dot_icon(w: i32, h: i32, red: bool) -> Option<HICON> {
    let (w, h) = (w.max(8) as usize, h.max(8) as usize);
    let mut px = vec![0u8; w * h * 4];
    let unit = w as f32 / 16.0;
    let ring = 1.0 * unit;
    let r = w as f32 / 2.0 - ring - 0.5;
    crate::unread::paint_dot(&mut px, w, h, (w as f32 / 2.0, h as f32 / 2.0), r, ring, if red { crate::unread::RED } else { crate::unread::LIGHT });
    unsafe { icon_from_pixels(&px, w, h) }
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
    crate::plugins::programs::close_all_now();
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
        // Programs open inside Ingle hide with it.
        crate::plugins::programs::hide_all();
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
        // Programs open inside Ingle close with it.
        crate::plugins::programs::close_all_now();
        // Leave voice, save settings and read markers, close server connections.
        engine.shutdown();
        // A moment for those goodbyes to go out before the process ends.
        std::thread::sleep(Duration::from_millis(300));
    }
    std::process::exit(0);
}

fn show_menu(tray: HWND, x: i32, y: i32) {
    use crate::update::Status;
    let Some((engine, updater)) = with_state(|s| (s.engine.clone(), s.updater.clone())) else { return };
    // Updating: "Update Ingle to 0.7.3" once there's a newer version (its progress while it
    // downloads), and Check for updates.
    let status = updater.status();
    let update = match &status {
        Status::Available(m) => Some(format!("Update Ingle to {}", m.version)),
        Status::Downloading(done, total) => Some(format!("Downloading the update ({}%)", done * 100 / (*total).max(1))),
        Status::Ready(..) => Some("Installing the update…".to_string()),
        _ => None,
    }
    .map(|l| wide(&l));
    let busy = matches!(status, Status::Downloading(..) | Status::Ready(..));
    let checking = updater.checking();
    let grey = |off: bool| -> MENU_ITEM_FLAGS { if off { MF_GRAYED } else { MENU_ITEM_FLAGS(0) } };
    let (muted, deafened) = engine.voice_prefs();
    let leave = {
        let st = engine.state();
        st.call.as_ref().map(|c| {
            let place = st.call_place(&c.server_key, c.channel_id);
            match (place.is_empty(), st.is_dm_call(&c.server_key, c.channel_id)) {
                (true, _) => "Leave voice".to_string(),
                (false, true) => format!("Hang up ({place})"),
                (false, false) => format!("Leave voice ({place})"),
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
        if let Some(label) = &update {
            let _ = AppendMenuW(menu, MF_STRING | grey(busy), CMD_UPDATE, PCWSTR(label.as_ptr()));
        }
        if !busy {
            let label = if checking { w!("Checking for updates…") } else { w!("Check for updates") };
            let _ = AppendMenuW(menu, MF_STRING | grey(checking), CMD_CHECK, label);
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
        // Download it, with the window open so its Update button (in the title strip) shows how
        // it's going (and the window's frames install it once it's here).
        CMD_UPDATE => {
            if let Status::Available(m) = updater.status() {
                updater.download(m);
            }
            show_main();
        }
        CMD_CHECK => {
            // The answer comes back to our window procedure (Windows notices must come from
            // this thread).
            let tray = TRAY.load(Ordering::Relaxed);
            updater.check_now(crate::update::Asker::Tray, move || {
                if tray != 0 {
                    unsafe {
                        let _ = PostMessageW(Some(HWND(tray as *mut core::ffi::c_void)), WM_INGLE_UPDATE, WPARAM(0), LPARAM(0));
                    }
                }
            });
        }
        _ => {}
    }
}

/// Tells about a call with a Windows notice (and flashes the taskbar button while someone's
/// calling, until Ingle is brought to the front).
fn tell_call() {
    let Some((title, text, flash)) = CALL_NOTE.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
    notify(&title, &text);
    if let (true, Some(main)) = (flash, main_window()) {
        let info = FLASHWINFO { cbSize: std::mem::size_of::<FLASHWINFO>() as u32, hwnd: main, dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG, uCount: 0, dwTimeout: 0 };
        unsafe {
            let _ = FlashWindowEx(&info);
        }
    }
}

/// Says what a check for updates asked for from the menu found, with a Windows notice (the
/// window may well be hidden).
fn tell_update_answers() {
    use crate::update::Checked;
    let answers = with_state(|s| s.updater.take_answers(crate::update::Asker::Tray)).unwrap_or_default();
    for answer in answers {
        match answer {
            Checked::UpToDate => {
                notify("Ingle is up to date", &format!("You have the newest version, Ingle {}.", crate::update::current_version()))
            }
            Checked::Found(v) => notify(
                &format!("Ingle {v} is out"),
                &format!("Click Update at the top of Ingle's window, or right-click the Ingle icon by the clock and choose Update Ingle to {v}."),
            ),
            Checked::Failed(e) => notify("Couldn't check for updates", &e),
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            match (lparam.0 as u32) & 0xffff {
                // The icon, or one of its notices (someone calling, say), clicked.
                NIN_SELECT | NIN_KEYSELECT | NIN_BALLOONUSERCLICK => show_main(),
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
        WM_INGLE_UNREAD => {
            show_unread();
            LRESULT(0)
        }
        WM_INGLE_UPDATE => {
            tell_update_answers();
            LRESULT(0)
        }
        WM_INGLE_CALL => {
            tell_call();
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

/// The icon as it shows now (its dot and tooltip included, so adding it again after Explorer
/// restarts keeps them).
fn icon_data(s: &State) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: s.tray,
        uID: ICON_ID,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
        uCallbackMessage: WM_TRAY,
        hIcon: s.shown.0,
        ..Default::default()
    };
    for (dst, src) in nid.szTip.iter_mut().zip(s.shown.1.encode_utf16().take(127)) {
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
