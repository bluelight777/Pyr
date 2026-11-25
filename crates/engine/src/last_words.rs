//! What ended Pyr, in its log, when Pyr didn't end itself (Linux, 0.16.7). Blue, Nov 25: "Pyr is
//! silently crashing for him" (Prognostic, Omarchy). A crash Rust sees (a panic) was in the log
//! already, and reported by itself the next time Pyr starts; but a program can also be ended from
//! under it: by the system for a bad memory access (SIGSEGV, SIGBUS), by a library aborting
//! (SIGABRT), or by a library calling `exit` from one of Pyr's threads (Xlib does when its
//! connection to the X server breaks). Then Pyr just went, with nothing in its log.
//!
//! Now the log says which, and from which of Pyr's threads (`Pyr was ended by …`), so the next
//! start reports it as a crash (`bugs::last_run_trouble`) and a bug report says where to look.
//! A signal's line is written with the log's file descriptor alone (nothing else is safe in a
//! signal handler); then the signal goes on as it would have (Rust's own handler for a stack
//! overflow first, which says so), and Pyr ends as before.

use std::sync::atomic::{AtomicI32, Ordering};

/// The log's file descriptor (-1: none yet).
static LOG_FD: AtomicI32 = AtomicI32::new(-1);

/// What the signals did before (Rust's stack overflow handler for SIGSEGV and SIGBUS).
static mut BEFORE: [Option<libc::sigaction>; 32] = [None; 32];

/// The words in the log that say Pyr was ended (`bugs::last_run_trouble` reads them).
pub const ENDED: &str = "Pyr was ended by";

const SIGNALS: [(libc::c_int, &str); 5] = [
    (libc::SIGSEGV, "SIGSEGV (a bad memory access)"),
    (libc::SIGBUS, "SIGBUS (memory that went away)"),
    (libc::SIGILL, "SIGILL (an instruction the processor can't run)"),
    (libc::SIGFPE, "SIGFPE (a division by zero)"),
    (libc::SIGABRT, "SIGABRT (a library aborted)"),
];

/// From now on, the last words go into the log at `fd` (the log file Pyr writes, kept open).
pub fn install(fd: i32) {
    LOG_FD.store(fd, Ordering::Relaxed);
    // SAFETY: sigaction with handlers that only write to a file descriptor, read the thread's
    // name and put the signal's handling back as it was; atexit with a plain function.
    unsafe {
        for (sig, _) in SIGNALS {
            let mut new: libc::sigaction = std::mem::zeroed();
            new.sa_sigaction = on_signal as extern "C" fn(libc::c_int, *mut libc::siginfo_t, *mut libc::c_void) as usize;
            new.sa_flags = libc::SA_SIGINFO | libc::SA_ONSTACK;
            libc::sigemptyset(&mut new.sa_mask);
            let mut old: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(sig, &new, &mut old) == 0 {
                (*std::ptr::addr_of_mut!(BEFORE))[sig as usize] = Some(old);
            }
        }
        libc::atexit(on_exit);
    }
}

/// The calling thread's name, as the system has it (at most 15 bytes).
fn thread_name(buf: &mut [u8; 16]) -> &[u8] {
    // SAFETY: PR_GET_NAME writes at most 16 bytes, nul-terminated, into the buffer given.
    unsafe {
        libc::prctl(libc::PR_GET_NAME, buf.as_mut_ptr() as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong, 0 as libc::c_ulong);
    }
    let n = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    &buf[..n]
}

/// Writes `parts` to the log, as one line (a write each: safe in a signal handler).
fn say(parts: &[&[u8]]) {
    let fd = LOG_FD.load(Ordering::Relaxed);
    if fd < 0 {
        return;
    }
    for p in parts {
        // SAFETY: write(2) of a buffer we own, to the log's descriptor.
        unsafe {
            libc::write(fd, p.as_ptr() as *const libc::c_void, p.len());
        }
    }
}

extern "C" fn on_signal(sig: libc::c_int, _info: *mut libc::siginfo_t, _ctx: *mut libc::c_void) {
    let what = SIGNALS.iter().find(|(s, _)| *s == sig).map(|(_, w)| *w).unwrap_or("a signal");
    let mut buf = [0u8; 16];
    let thread = thread_name(&mut buf);
    say(&[b"\nERROR pyr: ", ENDED.as_bytes(), b" ", what.as_bytes(), b" in its thread ", thread, b"\n"]);
    // As it was before (Rust's stack overflow handler, or the system's: ending Pyr), and again:
    // a fault happens again as the instruction is run again; a signal sent (abort's) is sent again.
    // SAFETY: sigaction and raise are safe in a signal handler.
    unsafe {
        if let Some(old) = (*std::ptr::addr_of!(BEFORE)).get(sig as usize).copied().flatten() {
            libc::sigaction(sig, &old, std::ptr::null_mut());
        } else {
            libc::signal(sig, libc::SIG_DFL);
        }
        libc::raise(sig);
    }
}

extern "C" fn on_exit() {
    // Pyr's own goodbyes are on its window's thread ("main"); any other thread calling `exit` is
    // a library ending Pyr (Xlib does when the X server goes away).
    let mut buf = [0u8; 16];
    let thread = thread_name(&mut buf).to_vec();
    if thread == b"main" || thread.is_empty() {
        return;
    }
    let trace = std::backtrace::Backtrace::force_capture().to_string();
    say(&[b"\nERROR pyr: ", ENDED.as_bytes(), b" exit() from its thread ", &thread, b" (a library ending the program)\n", trace.as_bytes(), b"\n"]);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A program with the last words in, its log a file: a thread that calls `exit` is named in it,
    /// and so is a bad memory access (the program, this test run again by itself, as a child).
    #[test]
    fn the_log_says_what_ended_it() {
        if let Ok(how) = std::env::var("PYR_LAST_WORDS_TEST") {
            let path = std::env::var("PYR_LAST_WORDS_LOG").unwrap();
            let f = std::fs::File::create(&path).unwrap();
            use std::os::fd::IntoRawFd;
            install(f.into_raw_fd());
            let t = std::thread::Builder::new()
                .name("pyr-test-end".into())
                .spawn(move || {
                    if how == "exit" {
                        std::process::exit(1);
                    }
                    // SAFETY: deliberately not: a write to address 0, to be ended for it.
                    unsafe { std::ptr::write_volatile(std::ptr::null_mut::<u8>(), 1) };
                })
                .unwrap();
            let _ = t.join();
            return;
        }
        let exe = std::env::current_exe().unwrap();
        for how in ["exit", "segv"] {
            let log = std::env::temp_dir().join(format!("pyr-last-words-{}-{how}.log", std::process::id()));
            let out = std::process::Command::new(&exe)
                .args(["--exact", "last_words::tests::the_log_says_what_ended_it", "--nocapture", "--test-threads", "1"])
                .env("PYR_LAST_WORDS_TEST", how)
                .env("PYR_LAST_WORDS_LOG", &log)
                .output()
                .unwrap();
            let said = std::fs::read_to_string(&log).unwrap_or_default();
            let _ = std::fs::remove_file(&log);
            assert!(!out.status.success(), "{how}: it should have ended");
            let want = if how == "exit" { "Pyr was ended by exit() from its thread pyr-test-end" } else { "Pyr was ended by SIGSEGV (a bad memory access) in its thread pyr-test-end" };
            assert!(said.contains(want), "{how}: {said}");
            // The next start reads it as a crash.
            assert_eq!(crate::bugs::last_run_trouble(&said).map(|t| t.0), Some("Pyr crashed"), "{how}");
        }
    }
}
