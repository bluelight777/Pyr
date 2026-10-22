//! The hang recorder (Windows). If Pyr's window stops responding, it writes to the log what
//! Pyr was doing: the step the window was in, which of Pyr's locks were held, how much CPU
//! each of Pyr's threads used, Pyr's memory and how busy the whole PC was. A freeze can
//! then be explained afterwards instead of guessed at. (The log of the run before a restart is
//! kept as pyr.previous.log, and goes with bug reports.)
//!
//! It runs on a thread of its own and touches no lock the window might be stuck on.

use std::sync::Mutex;

static STEP: Mutex<&'static str> = Mutex::new("starting");

/// Pyr is quitting (to install an update, or from the tray): its window going quiet now is
/// the goodbye, not a freeze, so nothing more is watched (bug report 20: quitting to install
/// an update was logged as the window not responding, and reported by itself next time).
#[cfg_attr(not(windows), allow(dead_code))]
static QUITTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Call when Pyr starts quitting.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn quitting() {
    QUITTING.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// When Pyr was started by Windows at sign-in (see `started_with_windows`).
#[cfg_attr(not(windows), allow(dead_code))]
static WITH_WINDOWS: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// Pyr was started by Windows at sign-in: a short freeze in its first minutes, while the PC is
/// still starting everything else, is logged but not reported (bug report 21: 6 seconds just
/// after sign-in, with Pyr itself all but idle).
pub fn started_with_windows() {
    let _ = WITH_WINDOWS.set(std::time::Instant::now());
}

/// Whether a freeze of `took` now is the PC starting up rather than Pyr.
#[cfg_attr(not(windows), allow(dead_code))]
fn while_windows_starts(took: std::time::Duration) -> bool {
    WITH_WINDOWS.get().is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(180)) && took < std::time::Duration::from_secs(15)
}

/// What the window is doing now (named in the log if it hangs). Never waits.
pub fn step(s: &'static str) {
    if let Ok(mut g) = STEP.try_lock() {
        *g = s;
    }
}

#[cfg(windows)]
pub use win::start;

#[cfg(not(windows))]
pub fn start(_engine: std::sync::Arc<pyr_engine::Engine>) {}

/// How full the PC's memory is (percent in use), where Pyr can tell (Windows).
#[cfg(windows)]
pub fn pc_memory_in_use() -> Option<u32> {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut ms = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut ms).is_ok().then_some(ms.dwMemoryLoad) }
}

#[cfg(not(windows))]
pub fn pc_memory_in_use() -> Option<u32> {
    None
}

#[cfg(windows)]
mod win {
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use pyr_engine::Engine;
    use windows::Win32::Foundation::{CloseHandle, FILETIME, HLOCAL, HWND, LPARAM, LocalFree, WPARAM};
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next};
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetCurrentProcessId, GetProcessTimes, GetSystemTimes, GetThreadDescription, GetThreadTimes, OpenThread,
        THREAD_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, SMTO_ABORTIFHUNG, SMTO_BLOCK, SendMessageTimeoutW, WM_NULL};

    /// Ask the window to answer within this long; slower than that counts as not responding.
    const ANSWER_WITHIN_MS: u32 = 4000;

    /// Start watching the main window (call once the tray module knows it).
    pub fn start(engine: Arc<Engine>) {
        // (Once: when the window is opened again, the same watch carries on, on the new one.)
        static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        let Some(main) = crate::tray::main_window() else { return };
        if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        // A window handle is just a number; it's fine to use from another thread.
        let main = main.0 as isize;
        let _ = std::thread::Builder::new().name("pyr-watchdog".into()).spawn(move || run(HWND(main as *mut _), engine));
    }

    fn ticks(f: FILETIME) -> u64 {
        (u64::from(f.dwHighDateTime) << 32) | u64::from(f.dwLowDateTime)
    }

    /// CPU time used so far (100 ns ticks), by the PC and by Pyr and each of its threads.
    struct Sample {
        at: Instant,
        sys_busy: u64,
        sys_total: u64,
        process: u64,
        threads: HashMap<u32, u64>,
    }

    fn sample() -> Sample {
        let at = Instant::now();
        let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        let (sys_busy, sys_total) = unsafe {
            if GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).is_ok() {
                // Kernel time includes idle time.
                let total = ticks(kernel) + ticks(user);
                (total.saturating_sub(ticks(idle)), total)
            } else {
                (0, 0)
            }
        };
        let process = unsafe {
            let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
            if GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u).is_ok() { ticks(k) + ticks(u) } else { 0 }
        };
        let mut threads = HashMap::new();
        unsafe {
            let me = GetCurrentProcessId();
            if let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) {
                let mut te = THREADENTRY32 { dwSize: std::mem::size_of::<THREADENTRY32>() as u32, ..Default::default() };
                let mut ok = Thread32First(snap, &mut te).is_ok();
                while ok {
                    if te.th32OwnerProcessID == me {
                        if let Ok(h) = OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, te.th32ThreadID) {
                            let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
                            if GetThreadTimes(h, &mut c, &mut e, &mut k, &mut u).is_ok() {
                                threads.insert(te.th32ThreadID, ticks(k) + ticks(u));
                            }
                            let _ = CloseHandle(h);
                        }
                    }
                    ok = Thread32Next(snap, &mut te).is_ok();
                }
                let _ = CloseHandle(snap);
            }
        }
        Sample { at, sys_busy, sys_total, process, threads }
    }

    fn thread_name(id: u32) -> String {
        unsafe {
            let Ok(h) = OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, id) else { return format!("thread {id}") };
            let name = match GetThreadDescription(h) {
                Ok(p) if !p.is_null() => {
                    let s = p.to_string().unwrap_or_default();
                    let _ = LocalFree(Some(HLOCAL(p.0 as *mut _)));
                    s
                }
                _ => String::new(),
            };
            let _ = CloseHandle(h);
            if name.is_empty() { format!("thread {id}") } else { name }
        }
    }

    /// Pyr's and the PC's memory: (Pyr's working set MB, Pyr's private MB, PC memory in
    /// use %, PC free MB).
    fn memory() -> (u64, u64, u32, u64) {
        unsafe {
            let mut pmc = PROCESS_MEMORY_COUNTERS_EX { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
            let (ws, private) =
                if GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS, pmc.cb).is_ok() {
                    (pmc.WorkingSetSize as u64 >> 20, pmc.PrivateUsage as u64 >> 20)
                } else {
                    (0, 0)
                };
            let mut ms = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
            let (load, free) = if GlobalMemoryStatusEx(&mut ms).is_ok() { (ms.dwMemoryLoad, ms.ullAvailPhys >> 20) } else { (0, 0) };
            (ws, private, load, free)
        }
    }

    /// One line about CPU and memory between two samples; with `threads`, Pyr's busiest
    /// threads too (the window's own marked).
    fn describe(a: &Sample, b: &Sample, ui_thread: u32, threads: bool) -> String {
        let wall = b.at.duration_since(a.at).as_secs_f64().max(0.001) * 1e7;
        let pc = if b.sys_total > a.sys_total { 100.0 * (b.sys_busy - a.sys_busy) as f64 / (b.sys_total - a.sys_total) as f64 } else { 0.0 };
        let pyr = 100.0 * b.process.saturating_sub(a.process) as f64 / wall;
        let (ws, private, load, free) = memory();
        let mut line = format!(
            "Pyr used {pyr:.0}% of a CPU core (the PC {pc:.0}% of all), {ws} MB of memory ({private} MB its own); the PC's memory {load}% in use, {free} MB free"
        );
        if threads {
            let mut busy: Vec<(u32, f64)> =
                b.threads.iter().map(|(id, t)| (*id, 100.0 * t.saturating_sub(*a.threads.get(id).unwrap_or(t)) as f64 / wall)).collect();
            busy.sort_by(|x, y| y.1.total_cmp(&x.1));
            let ui = busy.iter().find(|(id, _)| *id == ui_thread).map(|x| x.1).unwrap_or(0.0);
            let top: Vec<String> = busy.iter().filter(|(_, p)| *p >= 1.0).take(6).map(|(id, p)| format!("{} {p:.0}%", thread_name(*id))).collect();
            line.push_str(&format!(
                "; the window's thread {ui:.0}% ({}); busiest: {}",
                if ui < 5.0 { "waiting" } else { "working" },
                if top.is_empty() { "none".to_string() } else { top.join(", ") }
            ));
        }
        line
    }

    fn run(main: HWND, engine: Arc<Engine>) {
        let ui_thread = unsafe { GetWindowThreadProcessId(main, None) };
        let mut prev = sample();
        let mut hung_since: Option<Instant> = None;
        let mut last_report = Instant::now();
        let mut last_health = Instant::now();
        loop {
            std::thread::sleep(Duration::from_secs(2));
            if super::QUITTING.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            // (The window there is now: after it's opened again, the new one.)
            let main = crate::tray::main_window().unwrap_or(main);
            // Answered by the window's thread whenever it checks its messages (even while the
            // window is hidden in the tray).
            let answered = unsafe {
                let mut r = 0usize;
                SendMessageTimeoutW(main, WM_NULL, WPARAM(0), LPARAM(0), SMTO_ABORTIFHUNG | SMTO_BLOCK, ANSWER_WITHIN_MS, Some(&mut r)).0 != 0
            };
            if super::QUITTING.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let now = sample();
            match (answered, hung_since) {
                (false, None) => {
                    hung_since = Some(Instant::now());
                    last_report = Instant::now();
                    let step = *super::STEP.lock().unwrap_or_else(|e| e.into_inner());
                    let sh = &engine.shared;
                    tracing::warn!(
                        "Pyr's window stopped responding (it was {step}; shared state {}, call {}). {}",
                        if sh.state.is_locked_exclusive() { "being written" } else if sh.state.is_locked() { "being read" } else { "free" },
                        if sh.call.is_locked() { "busy" } else { "free" },
                        describe(&prev, &now, ui_thread, true)
                    );
                }
                (false, Some(_)) if last_report.elapsed() >= Duration::from_secs(20) => {
                    last_report = Instant::now();
                    tracing::warn!("still not responding: {}", describe(&prev, &now, ui_thread, true));
                }
                (true, Some(t)) => {
                    tracing::info!("Pyr's window responds again after {} s", t.elapsed().as_secs());
                    // A freeze it came out of is a bug too: reported by itself (with the log of
                    // what it was doing). Not one while Windows is still starting up.
                    if t.elapsed() >= Duration::from_secs(4) && super::while_windows_starts(t.elapsed()) {
                        tracing::info!("(not reported: Windows was still starting up)");
                    } else if t.elapsed() >= Duration::from_secs(4) {
                        pyr_engine::bugs::auto_report(
                            &engine.shared,
                            "Pyr stopped responding for a while",
                            &format!("Pyr's window didn't respond for {} s, then carried on.", t.elapsed().as_secs()),
                        );
                    }
                    hung_since = None;
                }
                _ => {}
            }
            // While something's heavy, a line a minute, to see a slowdown coming.
            if answered && last_health.elapsed() >= Duration::from_secs(60) {
                let (_, private, load, _) = memory();
                let pyr = 100.0 * now.process.saturating_sub(prev.process) as f64 / (now.at.duration_since(prev.at).as_secs_f64().max(0.001) * 1e7);
                if pyr > 60.0 || load > 90 || private > 1500 {
                    tracing::info!("busy: {}", describe(&prev, &now, ui_thread, true));
                }
                last_health = Instant::now();
            }
            prev = now;
        }
    }
}
