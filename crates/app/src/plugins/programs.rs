//! Programs from the Ingle Library open inside Ingle. Ingle starts the program and, on Windows,
//! takes its window in: no frame or taskbar button of its own, kept right over the program's
//! page in Ingle's window (moving, resizing, minimizing and hiding to the tray with Ingle), and
//! closed when Ingle quits. Pop out puts it back on its own.
//!
//! The program's window stays a window of its own (owned by Ingle's) rather than becoming part
//! of Ingle's, so nothing Ingle draws can go over it: it steps aside while a menu or dialog is
//! open, and makes room for toasts and calls ringing. Elsewhere (there are no programs for
//! Linux yet) a program opens in a window of its own, as before.

use std::process::Child;
use std::time::{Duration, Instant};

use eframe::egui::{self, Rect};

use super::install::InstalledPlugin;

/// A program Ingle started.
pub struct Program {
    pub id: String,
    pub name: String,
    child: Child,
    pub started: Instant,
    /// Out on its own (Pop out): Ingle leaves its window alone.
    pub popped: bool,
    #[cfg(windows)]
    win: win::Embedded,
}

impl Program {
    /// Its window has been found and taken in (or it's out on its own).
    pub fn ready(&self) -> bool {
        #[cfg(windows)]
        {
            self.popped || self.win.hwnd.is_some()
        }
        #[cfg(not(windows))]
        {
            true
        }
    }

    /// Opening for a while now with no window to show (it may have one Ingle can't find).
    pub fn slow(&self) -> bool {
        !self.ready() && self.started.elapsed() > Duration::from_secs(20)
    }
}

/// What Ingle shows over a program's page, for the program to keep clear of (its window can't
/// be drawn over).
#[derive(Default, Clone)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Overlays {
    /// A dialog, menu or picture is open: the program steps aside altogether.
    pub covering: bool,
    /// Toasts, a call ringing: the program makes room for these.
    pub keep_clear: Vec<Rect>,
}

#[derive(Default)]
pub struct Programs {
    pub open: Vec<Program>,
}

impl Programs {
    pub fn get(&self, id: &str) -> Option<&Program> {
        self.open.iter().find(|p| p.id == id)
    }

    /// Starts an installed program (after checking its files): inside Ingle on Windows.
    pub fn start(&mut self, dir: &std::path::Path, p: &InstalledPlugin) -> Result<(), String> {
        if self.get(&p.id).is_some() {
            return Ok(());
        }
        let child = super::install::spawn(dir, p)?;
        #[cfg(windows)]
        win::adopt(&child);
        self.open.push(Program {
            id: p.id.clone(),
            name: p.name.clone(),
            started: Instant::now(),
            popped: !cfg!(windows),
            #[cfg(windows)]
            win: win::Embedded::new(child.id()),
            child,
        });
        Ok(())
    }

    /// Each frame: programs that quit are let go of (their names returned), new ones' windows
    /// are found and taken in, and each is put where it goes: `shown` over `area` (points, in
    /// Ingle's window), clear of `over`; the others hidden.
    pub fn step(&mut self, ctx: &egui::Context, shown: Option<&str>, area: Option<Rect>, over: &Overlays) -> Vec<String> {
        let mut closed = Vec::new();
        self.open.retain_mut(|p| match p.child.try_wait() {
            Ok(None) => true,
            _ => {
                #[cfg(windows)]
                p.win.forget();
                closed.push(p.name.clone());
                false
            }
        });
        #[cfg(windows)]
        {
            let ppp = ctx.pixels_per_point();
            for p in &mut self.open {
                if p.popped {
                    continue;
                }
                let here = (shown == Some(p.id.as_str())).then_some(area).flatten();
                let place = here.filter(|_| !over.covering).map(|a| clear_of(a, &over.keep_clear)).filter(|a| a.width() > 40.0 && a.height() > 40.0);
                p.win.step(place.map(|a| a * ppp));
            }
            win::set_embedded(self.open.iter().filter(|p| !p.popped).filter_map(|p| p.win.hwnd.map(|h| (h, p.win.pid, p.win.cut_down()))).collect());
        }
        #[cfg(not(windows))]
        let _ = (ctx, shown, area, over);
        closed
    }

    /// Puts a program's window back on its own, with its frame and taskbar button.
    pub fn pop_out(&mut self, id: &str) {
        if let Some(p) = self.open.iter_mut().find(|p| p.id == id) {
            #[cfg(windows)]
            p.win.release();
            p.popped = true;
        }
    }

    /// Takes a popped-out program back in.
    pub fn pop_in(&mut self, id: &str) {
        if let Some(p) = self.open.iter_mut().find(|p| p.id == id) {
            if cfg!(windows) {
                p.popped = false;
            }
        }
    }

    /// Closes a program (asks it to, like its own close button would; it goes once it has).
    pub fn close(&mut self, id: &str) {
        if let Some(p) = self.open.iter_mut().find(|p| p.id == id) {
            #[cfg(windows)]
            p.win.close();
            #[cfg(not(windows))]
            let _ = p.child.kill();
        }
    }

    /// Ingle is quitting: close every program it took in. One that's still open after a moment
    /// (asking whether to save something, say) is put out on its own to finish, rather than
    /// ended; one that's stopped responding is ended.
    pub fn close_all(&mut self) {
        // (Done already from the tray's Quit or the installer: see `close_all_now`.)
        #[cfg(windows)]
        if win::closed_already() {
            return;
        }
        #[cfg(windows)]
        for p in &mut self.open {
            if !p.popped {
                p.win.close();
            }
        }
        let until = Instant::now() + Duration::from_millis(1500);
        for p in &mut self.open {
            if p.popped {
                continue;
            }
            while Instant::now() < until && matches!(p.child.try_wait(), Ok(None)) {
                std::thread::sleep(Duration::from_millis(50));
            }
            if !matches!(p.child.try_wait(), Ok(None)) {
                continue;
            }
            #[cfg(windows)]
            {
                if p.win.hung() {
                    let _ = p.child.kill();
                } else {
                    p.win.release();
                    p.popped = true;
                }
            }
            #[cfg(not(windows))]
            let _ = p.child.kill();
        }
        // (What's still open stays open after Ingle.)
        #[cfg(windows)]
        win::let_go();
    }
}

/// `area` less the strips `keep` covers at its top or bottom (whichever side each is nearer).
#[cfg_attr(not(windows), allow(dead_code))]
fn clear_of(area: Rect, keep: &[Rect]) -> Rect {
    let mut a = area;
    for k in keep {
        if !k.intersects(a) {
            continue;
        }
        if k.center().y < a.center().y {
            a.min.y = a.min.y.max(k.max.y + 8.0);
        } else {
            a.max.y = a.max.y.min(k.min.y - 8.0);
        }
    }
    a
}

/// What's left of a program's frame around the part it draws in, in pixels (Windows still draws
/// it): some programs keep a frame Ingle can't take off, like an undecorated window that keeps
/// room for its shadow. Ingle makes such a window that much bigger, so the part the program
/// draws in fills the page to its edges, and cuts the rest off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
struct Insets {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl Insets {
    /// The most a frame (a title bar included, on a big screen) could be.
    const MAX: i32 = 200;

    /// From the window's outer rectangle and its own area's, both (left, top, right, bottom)
    /// on the screen. Anything odd (the window moving between the two looks, say) is none.
    fn measured(window: (i32, i32, i32, i32), client: (i32, i32, i32, i32)) -> Insets {
        let i = Insets { left: client.0 - window.0, top: client.1 - window.1, right: window.2 - client.2, bottom: window.3 - client.3 };
        let ok = |v: i32| (0..=Insets::MAX).contains(&v);
        if ok(i.left) && ok(i.top) && ok(i.right) && ok(i.bottom) { i } else { Insets::default() }
    }

    fn is_none(&self) -> bool {
        *self == Insets::default()
    }

    /// The window's rectangle (x, y, width, height) that puts the part it draws in on `target`.
    fn around(&self, target: (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
        (target.0 - self.left, target.1 - self.top, target.2 + self.left + self.right, target.3 + self.top + self.bottom)
    }
}

/// Ingle hid to the tray (Windows): hide the programs with it (a window it owns doesn't hide by
/// itself). They're put back by the next frame once Ingle's window is back.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn hide_all() {
    #[cfg(windows)]
    win::hide_all();
}

/// Ingle is quitting (or updating) from outside the window's frames (the tray icon's Quit, the
/// installer): close the programs it took in, and end any that won't close.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn close_all_now() {
    #[cfg(windows)]
    win::close_all_now();
}

/// The window in front is one of the programs Ingle took in (so Ingle counts as in front).
#[cfg(windows)]
pub fn is_embedded(hwnd: isize) -> bool {
    win::is_embedded(hwnd)
}

#[cfg(windows)]
mod win {
    //! The Windows side: finding a program's window, taking it in, and keeping it in place.

    use std::sync::Mutex;
    use std::sync::atomic::{AtomicIsize, Ordering};

    use eframe::egui::Rect;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, POINT, RECT, WPARAM};
    use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute};
    use windows::Win32::Graphics::Gdi::{
        ClientToScreen, CreateRectRgn, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, SetWindowRgn,
    };
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JobObjectExtendedLimitInformation, SetInformationJobObject,
    };
    use windows::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, PROCESS_TERMINATE, TerminateProcess};
    use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
    use windows::Win32::UI::WindowsAndMessaging::{
        CHILDID_SELF, EVENT_OBJECT_LOCATIONCHANGE, EnumWindows, GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GWLP_HWNDPARENT, GetClientRect, GetMenu, GetWindow,
        GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowThreadProcessId, IsHungAppWindow, IsWindow, IsWindowVisible, IsZoomed,
        OBJID_WINDOW, PostMessageW, SW_HIDE, SW_RESTORE, SW_SHOWNA, SWP_ASYNCWINDOWPOS, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE,
        SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, ShowWindow, ShowWindowAsync, WINEVENT_OUTOFCONTEXT, WM_CLOSE,
        WS_CAPTION, WS_EX_APPWINDOW, WS_EX_CLIENTEDGE, WS_EX_DLGMODALFRAME, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
        WS_EX_WINDOWEDGE, WS_HSCROLL, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME, WS_VSCROLL,
    };
    use windows::core::{BOOL, PCWSTR};

    use super::Insets;

    /// The frame, title bar and their buttons: what comes off a program's window.
    const FRAME: u32 = WS_CAPTION.0 | WS_THICKFRAME.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0;

    fn hwnd(h: isize) -> HWND {
        HWND(h as *mut core::ffi::c_void)
    }

    /// One program's window.
    pub struct Embedded {
        pub pid: u32,
        pub hwnd: Option<isize>,
        /// Its styles before Ingle took it in (put back by Pop out).
        styles: Option<(isize, isize)>,
        /// Where it was put last (screen pixels), and whether it's showing.
        placed: Option<(i32, i32, i32, i32)>,
        showing: bool,
        /// What's left of its frame (see `Insets`), and the part of its window it's cut down
        /// to, if any (that much frame, and its page's size).
        inset: Insets,
        clipped: Option<(Insets, i32, i32)>,
    }

    impl Embedded {
        pub fn new(pid: u32) -> Embedded {
            Embedded { pid, hwnd: None, styles: None, placed: None, showing: false, inset: Insets::default(), clipped: None }
        }

        /// `place`: where it goes (physical pixels in Ingle's window), or hidden.
        pub fn step(&mut self, place: Option<Rect>) {
            let Some(main) = crate::tray::main_window() else { return };
            if self.hwnd.is_some_and(|h| unsafe { !IsWindow(Some(hwnd(h))).as_bool() }) {
                self.forget();
            }
            if self.hwnd.is_none() {
                match find_window(self.pid) {
                    Some(h) => self.take_in(h, main),
                    None => return,
                }
            }
            let Some(h) = self.hwnd else { return };
            let w = hwnd(h);
            // Not responding: leave it alone until it is again (anything asked of its window
            // would wait for it), and gone full screen (a video, say): until it's back (shown
            // whole meanwhile). Before Ingle has put it anywhere, a window as big as the screen
            // is only how it started (on a small screen, say): it goes in its page.
            if hung(w) {
                return;
            }
            if self.placed.is_some() && fullscreen(w) {
                self.clip(w, None);
                return;
            }
            unsafe {
                // Some programs put their frame (or taskbar button) back, after full screen say:
                // off again.
                let style = GetWindowLongPtrW(w, GWL_STYLE) as u32;
                let ex = GetWindowLongPtrW(w, GWL_EXSTYLE) as u32;
                if style & FRAME != 0 || ex & WS_EX_TOOLWINDOW.0 == 0 {
                    strip(w);
                }
                match place {
                    Some(r) => {
                        let mut origin = POINT { x: r.min.x.round() as i32, y: r.min.y.round() as i32 };
                        let _ = ClientToScreen(main, &mut origin);
                        let target = (origin.x, origin.y, r.width().round() as i32, r.height().round() as i32);
                        // What's left of its frame, looked at while it's where it was put (not
                        // moving): the window goes that much bigger, and is cut down to its page.
                        let now = window_rect(w);
                        if self.placed.is_none() || self.placed.map(|p| (p.0, p.1, p.0 + p.2, p.1 + p.3)) == now {
                            let found = insets(w);
                            if found != self.inset {
                                tracing::info!(pid = self.pid, ?found, "a program's window keeps part of its frame: Ingle covers it");
                                self.inset = found;
                            }
                        }
                        let at = self.inset.around(target);
                        follow(Some((main, w, (r.min.x.round() as i32, r.min.y.round() as i32, target.2, target.3), self.inset)));
                        if self.placed != Some(at) {
                            let _ = SetWindowPos(w, None, at.0, at.1, at.2, at.3, SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS);
                            self.placed = Some(at);
                        }
                        self.clip(w, (!self.inset.is_none()).then_some((self.inset, target.2, target.3)));
                        // (Hidden meanwhile with Ingle, to the tray say: shown again.)
                        if !self.showing || !IsWindowVisible(w).as_bool() {
                            let _ = ShowWindowAsync(w, SW_SHOWNA);
                            self.showing = true;
                        }
                    }
                    None => {
                        if self.showing || IsWindowVisible(w).as_bool() {
                            let _ = ShowWindowAsync(w, SW_HIDE);
                            self.showing = false;
                        }
                    }
                }
            }
        }

        /// Takes its window in: owned by Ingle's (above it, minimized and closed with it), no
        /// frame, no taskbar button, square corners.
        fn take_in(&mut self, h: isize, main: HWND) {
            let w = hwnd(h);
            unsafe {
                self.styles = Some((GetWindowLongPtrW(w, GWL_STYLE), GetWindowLongPtrW(w, GWL_EXSTYLE)));
                if IsZoomed(w).as_bool() {
                    let _ = ShowWindow(w, SW_RESTORE);
                }
                SetWindowLongPtrW(w, GWLP_HWNDPARENT, main.0 as isize);
                strip(w);
                let square = DWMWCP_DONOTROUND;
                let _ = DwmSetWindowAttribute(w, DWMWA_WINDOW_CORNER_PREFERENCE, &square as *const _ as *const core::ffi::c_void, std::mem::size_of_val(&square) as u32);
            }
            self.forget_placement();
            self.hwnd = Some(h);
            self.showing = unsafe { IsWindowVisible(w).as_bool() };
            ensure_hook();
        }

        /// Cuts its window down to the part it draws in (`Insets`), or (none) back to whole.
        fn clip(&mut self, w: HWND, to: Option<(Insets, i32, i32)>) {
            if self.clipped == to {
                return;
            }
            unsafe {
                match to {
                    Some((i, width, height)) => {
                        // (Windows keeps the region and frees it with the window.)
                        let _ = SetWindowRgn(w, Some(CreateRectRgn(i.left, i.top, i.left + width, i.top + height)), true);
                    }
                    None => {
                        let _ = SetWindowRgn(w, None, true);
                    }
                }
            }
            self.clipped = to;
        }

        /// Pop out: a window of its own again, whole, with a frame and a taskbar button (one
        /// that started without a frame gets one too).
        pub fn release(&mut self) {
            let Some(h) = self.hwnd else { return };
            self.clip(hwnd(h), None);
            unsafe { give_back(hwnd(h), self.styles) };
            self.forget_placement();
            self.hwnd = None;
            self.styles = None;
        }

        /// Its window is cut down to the part it draws in (see `clip`).
        pub fn cut_down(&self) -> bool {
            self.clipped.is_some()
        }

        /// Its window isn't responding.
        pub fn hung(&self) -> bool {
            self.hwnd.is_some_and(|h| hung(hwnd(h)))
        }

        /// Ask it to close, like its own close button would.
        pub fn close(&mut self) {
            if let Some(h) = self.hwnd {
                unsafe {
                    let _ = PostMessageW(Some(hwnd(h)), WM_CLOSE, WPARAM(0), LPARAM(0));
                }
            }
        }

        /// It quit.
        pub fn forget(&mut self) {
            self.forget_placement();
            self.hwnd = None;
        }

        fn forget_placement(&mut self) {
            let mine = self.hwnd;
            let mut f = FOLLOW.lock().unwrap_or_else(|e| e.into_inner());
            if f.is_some_and(|x| Some(x.1) == mine) {
                *f = None;
            }
            self.placed = None;
            self.showing = false;
            self.inset = Insets::default();
            self.clipped = None;
        }
    }

    /// A window of its own again (see `Embedded::release`); `styles`: how it was before.
    unsafe fn give_back(w: HWND, styles: Option<(isize, isize)>) {
        unsafe {
            SetWindowLongPtrW(w, GWLP_HWNDPARENT, 0);
            let (style, ex) = styles.unwrap_or((GetWindowLongPtrW(w, GWL_STYLE), GetWindowLongPtrW(w, GWL_EXSTYLE)));
            let style = (style as u32 & !WS_POPUP.0) | FRAME;
            let ex = (ex as u32 & !WS_EX_TOOLWINDOW.0) | WS_EX_APPWINDOW.0;
            SetWindowLongPtrW(w, GWL_STYLE, style as isize);
            SetWindowLongPtrW(w, GWL_EXSTYLE, ex as isize);
            let _ = SetWindowPos(w, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
            let _ = ShowWindow(w, SW_SHOWNA);
        }
    }

    /// A window's outer rectangle on the screen (left, top, right, bottom).
    fn window_rect(w: HWND) -> Option<(i32, i32, i32, i32)> {
        let mut r = RECT::default();
        unsafe { GetWindowRect(w, &mut r) }.ok().map(|_| (r.left, r.top, r.right, r.bottom))
    }

    /// What's left of a window's frame around the part it draws in (see `Insets`). A window with
    /// a menu bar or scroll bars of its own has those outside that part too: it's left as it is.
    fn insets(w: HWND) -> Insets {
        let style = unsafe { GetWindowLongPtrW(w, GWL_STYLE) } as u32;
        if style & (WS_VSCROLL.0 | WS_HSCROLL.0) != 0 || unsafe { !GetMenu(w).is_invalid() } {
            return Insets::default();
        }
        let Some(outer) = window_rect(w) else { return Insets::default() };
        let mut c = RECT::default();
        let mut o = POINT { x: 0, y: 0 };
        unsafe {
            if GetClientRect(w, &mut c).is_err() || !ClientToScreen(w, &mut o).as_bool() {
                return Insets::default();
            }
        }
        Insets::measured(outer, (o.x, o.y, o.x + c.right, o.y + c.bottom))
    }

    /// Not responding (Windows' own test: nothing taken from its messages for 5 seconds).
    fn hung(w: HWND) -> bool {
        unsafe { IsHungAppWindow(w).as_bool() }
    }

    unsafe fn strip(w: HWND) {
        unsafe {
            let style = GetWindowLongPtrW(w, GWL_STYLE) as u32;
            let ex = GetWindowLongPtrW(w, GWL_EXSTYLE) as u32;
            SetWindowLongPtrW(w, GWL_STYLE, ((style & !FRAME) | WS_POPUP.0) as isize);
            let ex_off = WS_EX_APPWINDOW.0 | WS_EX_WINDOWEDGE.0 | WS_EX_CLIENTEDGE.0 | WS_EX_DLGMODALFRAME.0;
            SetWindowLongPtrW(w, GWL_EXSTYLE, ((ex & !ex_off) | WS_EX_TOOLWINDOW.0) as isize);
            let _ = SetWindowPos(w, None, 0, 0, 0, 0, SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }

    /// Covers its whole screen (full screen).
    fn fullscreen(w: HWND) -> bool {
        unsafe {
            let mut r = RECT::default();
            if GetWindowRect(w, &mut r).is_err() {
                return false;
            }
            let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if !GetMonitorInfoW(MonitorFromWindow(w, MONITOR_DEFAULTTONEAREST), &mut mi).as_bool() {
                return false;
            }
            let m = mi.rcMonitor;
            r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
        }
    }

    /// The program's main window: one of its top-level windows with a title, not a tool or
    /// helper window, nobody's (no owner), and the biggest such.
    fn find_window(pid: u32) -> Option<isize> {
        struct Search {
            pid: u32,
            best: Option<(isize, i64)>,
        }
        unsafe extern "system" fn each(w: HWND, lp: LPARAM) -> BOOL {
            let s = unsafe { &mut *(lp.0 as *mut Search) };
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(w, Some(&mut pid)) };
            if pid != s.pid {
                return BOOL(1);
            }
            let ex = unsafe { GetWindowLongPtrW(w, GWL_EXSTYLE) } as u32;
            let helper = ex & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0) != 0 || ex & (WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0) == (WS_EX_LAYERED.0 | WS_EX_TRANSPARENT.0);
            let owned = unsafe { GetWindow(w, GW_OWNER) }.is_ok_and(|o| !o.is_invalid());
            let titled = unsafe { GetWindowTextLengthW(w) } > 0;
            if helper || owned || !titled {
                return BOOL(1);
            }
            let mut r = RECT::default();
            let _ = unsafe { GetWindowRect(w, &mut r) };
            let area = (r.right - r.left) as i64 * (r.bottom - r.top) as i64;
            if area > 100 * 100 && s.best.is_none_or(|b| area > b.1) {
                s.best = Some((w.0 as isize, area));
            }
            BOOL(1)
        }
        let mut s = Search { pid, best: None };
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut s as *mut Search as isize));
        }
        s.best.map(|b| b.0)
    }

    // ---- following Ingle's window as it's dragged (no frames run while it is)

    /// Ingle's window, the program on show, where it goes in Ingle's window (pixels), and what's
    /// left of its frame.
    #[derive(Clone, Copy)]
    struct Follow(isize, isize, (i32, i32, i32, i32), Insets);

    static FOLLOW: Mutex<Option<Follow>> = Mutex::new(None);
    static HOOK: AtomicIsize = AtomicIsize::new(0);
    /// The program windows taken in, their processes, and whether Ingle cut the window down (for
    /// hiding to the tray, counting as Ingle in front, and closing with Ingle).
    static EMBEDDED: Mutex<Vec<(isize, u32, bool)>> = Mutex::new(Vec::new());

    fn follow(f: Option<(HWND, HWND, (i32, i32, i32, i32), Insets)>) {
        *FOLLOW.lock().unwrap_or_else(|e| e.into_inner()) = f.map(|(m, w, r, i)| Follow(m.0 as isize, w.0 as isize, r, i));
    }

    fn ensure_hook() {
        if HOOK.load(Ordering::Relaxed) != 0 {
            return;
        }
        let h: HWINEVENTHOOK = unsafe {
            SetWinEventHook(EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE, None, Some(moved), GetCurrentProcessId(), 0, WINEVENT_OUTOFCONTEXT)
        };
        HOOK.store(h.0 as isize, Ordering::Relaxed);
    }

    /// Ingle's window moved: the program on show goes with it.
    unsafe extern "system" fn moved(_: HWINEVENTHOOK, _event: u32, w: HWND, object: i32, child: i32, _: u32, _: u32) {
        if object != OBJID_WINDOW.0 || child != CHILDID_SELF as i32 {
            return;
        }
        let Some(Follow(main, prog, (x, y, cw, ch), inset)) = *FOLLOW.lock().unwrap_or_else(|e| e.into_inner()) else { return };
        if w.0 as isize != main {
            return;
        }
        unsafe {
            if !IsWindowVisible(hwnd(prog)).as_bool() || hung(hwnd(prog)) {
                return;
            }
            let mut origin = POINT { x, y };
            let _ = ClientToScreen(hwnd(main), &mut origin);
            let at = inset.around((origin.x, origin.y, cw, ch));
            let _ = SetWindowPos(hwnd(prog), None, at.0, at.1, at.2, at.3, SWP_NOACTIVATE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS);
        }
    }

    pub fn set_embedded(list: Vec<(isize, u32, bool)>) {
        *EMBEDDED.lock().unwrap_or_else(|e| e.into_inner()) = list;
    }

    pub fn is_embedded(h: isize) -> bool {
        EMBEDDED.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|e| e.0 == h)
    }

    pub fn hide_all() {
        let list = EMBEDDED.lock().unwrap_or_else(|e| e.into_inner()).clone();
        for (h, _, _) in list {
            unsafe {
                let _ = ShowWindowAsync(hwnd(h), SW_HIDE);
            }
        }
    }

    static CLOSED_ALL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

    pub fn closed_already() -> bool {
        CLOSED_ALL.load(Ordering::Relaxed)
    }

    /// Like `Programs::close_all`, from outside the window's frames.
    pub fn close_all_now() {
        CLOSED_ALL.store(true, Ordering::Relaxed);
        let list = std::mem::take(&mut *EMBEDDED.lock().unwrap_or_else(|e| e.into_inner()));
        for (h, _, _) in &list {
            unsafe {
                let _ = PostMessageW(Some(hwnd(*h)), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        let until = std::time::Instant::now() + std::time::Duration::from_millis(1500);
        for (h, pid, cut_down) in list {
            while std::time::Instant::now() < until && unsafe { IsWindow(Some(hwnd(h))).as_bool() } {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            if !unsafe { IsWindow(Some(hwnd(h))).as_bool() } {
                continue;
            }
            if hung(hwnd(h)) {
                if let Ok(p) = unsafe { OpenProcess(PROCESS_TERMINATE, false, pid) } {
                    unsafe {
                        let _ = TerminateProcess(p, 0);
                        let _ = CloseHandle(p);
                    }
                }
            } else {
                // Still open (asking something, say): out on its own to finish, whole.
                unsafe {
                    if cut_down {
                        let _ = SetWindowRgn(hwnd(h), None, true);
                    }
                    give_back(hwnd(h), None);
                }
            }
        }
        let_go();
    }

    // ---- ending with Ingle: the programs Ingle starts are in a job of its own, which Windows
    // ends when Ingle ends without closing them (a crash), rather than leaving them running
    // hidden or with no taskbar button. Closing on purpose lets them go first.

    static JOB: AtomicIsize = AtomicIsize::new(0);

    fn set_job_limits(job: HANDLE, kill_with_ingle: bool) -> bool {
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        // (A program's own programs may leave the job: browsers keep theirs in jobs of their own.)
        info.BasicLimitInformation.LimitFlags =
            if kill_with_ingle { JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK } else { JOB_OBJECT_LIMIT_BREAKAWAY_OK };
        unsafe {
            SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const core::ffi::c_void, std::mem::size_of_val(&info) as u32)
                .is_ok()
        }
    }

    /// Puts a program Ingle just started in Ingle's job (made the first time).
    pub fn adopt(child: &std::process::Child) {
        use std::os::windows::io::AsRawHandle;
        let mut job = JOB.load(Ordering::Relaxed);
        if job == 0 {
            let Ok(h) = (unsafe { CreateJobObjectW(None, PCWSTR::null()) }) else { return };
            if !set_job_limits(h, true) {
                unsafe {
                    let _ = CloseHandle(h);
                }
                return;
            }
            job = h.0 as isize;
            JOB.store(job, Ordering::Relaxed);
        }
        unsafe {
            let _ = AssignProcessToJobObject(HANDLE(job as *mut core::ffi::c_void), HANDLE(child.as_raw_handle()));
        }
    }

    /// Ingle is closing on purpose: programs still open (popped out, or finishing something)
    /// stay open after it.
    pub fn let_go() {
        let job = JOB.load(Ordering::Relaxed);
        if job != 0 {
            set_job_limits(HANDLE(job as *mut core::ffi::c_void), false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{pos2, vec2};

    #[test]
    fn a_frame_left_on_a_window_is_covered() {
        // An undecorated window that keeps room for its shadow (at 150%): 11 pixels at the
        // sides and bottom, 2 at the top.
        let window = (100, 100, 1100, 800);
        let i = Insets::measured(window, (111, 102, 1089, 789));
        assert_eq!(i, Insets { left: 11, top: 2, right: 11, bottom: 11 });
        // Put so that the part it draws in is exactly the page.
        let page = (108, 108, 3731, 1969);
        let at = i.around(page);
        assert_eq!(at, (97, 106, 3753, 1982));
        assert_eq!((at.0 + i.left, at.1 + i.top, at.2 - i.left - i.right, at.3 - i.top - i.bottom), page);
        // No frame: as it is.
        assert!(Insets::measured(window, window).is_none());
        assert_eq!(Insets::default().around(page), page);
        // Looked at while it moved (the two rectangles from different places): none.
        assert!(Insets::measured(window, (90, 95, 1080, 790)).is_none());
        assert!(Insets::measured(window, (400, 400, 500, 500)).is_none(), "more than a frame could be");
    }

    #[test]
    fn makes_room_for_what_is_over_it() {
        let area = Rect::from_min_size(pos2(100.0, 50.0), vec2(800.0, 600.0));
        assert_eq!(clear_of(area, &[]), area);
        // Toasts at the bottom right: the program ends above them.
        let toasts = Rect::from_min_max(pos2(600.0, 500.0), pos2(900.0, 640.0));
        assert_eq!(clear_of(area, &[toasts]).max.y, 492.0);
        // A call ringing at the top: it starts below it.
        let call = Rect::from_min_max(pos2(300.0, 30.0), pos2(640.0, 200.0));
        let both = clear_of(area, &[toasts, call]);
        assert_eq!((both.min.y, both.max.y), (208.0, 492.0));
        // Elsewhere: no change.
        assert_eq!(clear_of(area, &[Rect::from_min_size(pos2(0.0, 0.0), vec2(50.0, 50.0))]), area);
    }
}
