//! Windows: the green frame around whatever you're sharing.
//!
//! Windows draws its own yellow frame around a captured screen or window, and lets apps turn
//! it off but not recolor it. So Ingle turns it off and draws a green one: four thin windows
//! along the edges that clicks pass straight through, kept out of the capture itself so
//! viewers never see them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Gdi::{CreateSolidBrush, GetMonitorInfoW, HMONITOR, MONITORINFO};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::core::w;

/// Discord-like green.
const GREEN: COLORREF = COLORREF(0x0081_b543);
const THICK: i32 = 3;

#[derive(Clone, Copy)]
pub enum Target {
    Monitor(isize),
    Window(isize),
}

/// The frame; dropping it removes it.
pub struct ShareBorder {
    hwnds: Arc<Mutex<Vec<isize>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ShareBorder {
    pub fn show(target: Target) -> ShareBorder {
        let hwnds = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (h, s) = (hwnds.clone(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("share-border".into())
            .spawn(move || unsafe {
                if let Err(e) = run(target, &h, &s) {
                    tracing::warn!("share border: {e}");
                }
            })
            .ok();
        ShareBorder { hwnds, stop, thread }
    }
}

impl Drop for ShareBorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(&first) = self.hwnds.lock().unwrap().first() {
            unsafe {
                let _ = PostMessageW(Some(HWND(first as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_CLOSE => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            WM_NCHITTEST => LRESULT(-1), // HTTRANSPARENT
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

unsafe fn run(target: Target, hwnds: &Mutex<Vec<isize>>, stop: &AtomicBool) -> windows::core::Result<()> {
    unsafe {
        let hinst = GetModuleHandleW(None)?;
        let class = w!("IngleShareBorder");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst.into(),
            lpszClassName: class,
            hbrBackground: CreateSolidBrush(GREEN),
            ..Default::default()
        };
        RegisterClassW(&wc); // fails harmlessly if already registered
        let mut ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
        if matches!(target, Target::Monitor(_)) {
            ex |= WS_EX_TOPMOST;
        }
        let mut wins = Vec::new();
        for _ in 0..4 {
            let h = CreateWindowExW(ex, class, w!("Ingle sharing"), WS_POPUP, 0, 0, 1, 1, None, None, Some(hinst.into()), None)?;
            let _ = SetLayeredWindowAttributes(h, COLORREF(0), 255, LWA_ALPHA);
            // Keep it out of the stream. Without this (Windows before 10 2004) it would show up
            // in a whole-screen share, so leave the frame off there.
            if SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE).is_err() {
                for w in wins.into_iter().chain([h]) {
                    let _ = DestroyWindow(w);
                }
                return Ok(());
            }
            wins.push(h);
        }
        *hwnds.lock().unwrap() = wins.iter().map(|h| h.0 as isize).collect();
        SetTimer(Some(wins[0]), 1, 100, None);
        place(target, &wins);
        let mut msg = MSG::default();
        while !stop.load(Ordering::Relaxed) && GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_TIMER {
                place(target, &wins);
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        for h in wins {
            let _ = DestroyWindow(h);
        }
        Ok(())
    }
}

/// Moves the four edges onto the target (or hides them while it's minimized).
unsafe fn place(target: Target, wins: &[HWND]) {
    unsafe {
        let (rect, above): (Option<RECT>, Option<HWND>) = match target {
            Target::Monitor(m) => {
                let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
                let ok = GetMonitorInfoW(HMONITOR(m as *mut _), &mut mi).as_bool();
                (ok.then_some(mi.rcMonitor), Some(HWND_TOPMOST))
            }
            Target::Window(w) => {
                let hwnd = HWND(w as *mut _);
                if !IsWindow(Some(hwnd)).as_bool() || IsIconic(hwnd).as_bool() || !IsWindowVisible(hwnd).as_bool() {
                    (None, None)
                } else {
                    let mut r = RECT::default();
                    let got = DwmGetWindowAttribute(
                        hwnd,
                        DWMWA_EXTENDED_FRAME_BOUNDS,
                        &mut r as *mut RECT as *mut _,
                        std::mem::size_of::<RECT>() as u32,
                    )
                    .is_ok()
                        || GetWindowRect(hwnd, &mut r).is_ok();
                    // Sit just above the shared window, not above everything, so the frame
                    // doesn't float over windows that cover it.
                    let prev = GetWindow(hwnd, GW_HWNDPREV).ok();
                    let ours = prev.map(|p| wins.contains(&p)).unwrap_or(false);
                    let above = if ours { None } else { Some(prev.unwrap_or(HWND_TOP)) };
                    (got.then_some(r), above)
                }
            }
        };
        let Some(r) = rect else {
            for &h in wins {
                let _ = ShowWindow(h, SW_HIDE);
            }
            return;
        };
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let edges = [
            (r.left, r.top, w, THICK),
            (r.left, r.bottom - THICK, w, THICK),
            (r.left, r.top, THICK, h),
            (r.right - THICK, r.top, THICK, h),
        ];
        for (&win, (x, y, ew, eh)) in wins.iter().zip(edges) {
            let mut flags = SWP_NOACTIVATE | SWP_SHOWWINDOW;
            if above.is_none() {
                flags |= SWP_NOZORDER;
            }
            let _ = SetWindowPos(win, above, x, y, ew, eh, flags);
        }
    }
}
