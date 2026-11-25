//! (0.16.7) The keys held, as push to talk and the shortcut keys read them on Linux, a line every
//! time they change, for `seconds` (60 unless told): through `hotkey::x11keys` (x11rb), or with
//! `old` through `device_query` as before 0.16.7 (Xlib). Take the X server away under it (kill
//! the Xvfb it runs on, say, and start another): the new reader says it lost it, carries on, and
//! reads the keys again from the next one; Xlib ends the old one there and then (exit code 1),
//! as it ended Pyr. Blue, Nov 25: "Pyr is silently crashing for him" (Omarchy).
//!
//!   cargo run --release -p pyr-engine --example x11_keys -- [old] [seconds]

#[cfg(target_os = "linux")]
fn main() {
    use std::time::{Duration, Instant};
    tracing_subscriber::fmt().with_env_filter("info").init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let old = args.iter().any(|a| a == "old");
    let secs: u64 = args.iter().find_map(|a| a.parse().ok()).unwrap_or(60);
    let until = Instant::now() + Duration::from_secs(secs);
    println!("reading the keys with {} for {secs} s", if old { "device_query (Xlib)" } else { "x11keys (x11rb)" });
    let mut was: Vec<String> = Vec::new();
    let mut new_reader = pyr_engine::hotkey::x11keys::X11Input::new();
    let old_reader = old.then(device_query::DeviceState::checked_new).flatten();
    while Instant::now() < until {
        let keys = if old {
            use device_query::DeviceQuery;
            old_reader.as_ref().map(|s| s.get_keys()).unwrap_or_default()
        } else {
            new_reader.keys()
        };
        let mut now: Vec<String> = keys.iter().map(|k| k.to_string()).collect();
        now.sort();
        if now != was {
            println!("held: {}", if now.is_empty() { "nothing".to_string() } else { now.join(" + ") });
            was = now;
        }
        std::thread::sleep(Duration::from_millis(8));
    }
    println!("done: still here after {secs} s");
}

#[cfg(not(target_os = "linux"))]
fn main() {
    println!("Linux only");
}
