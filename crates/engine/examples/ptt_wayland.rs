//! Push to talk's watcher on its own, saying when it goes down and up and what the desktop does
//! with its key: for trying it on Wayland (Linux; see `hotkey::wayland`), against a real desktop
//! or a stand-in for one.
//!
//!     ptt_wayland KEY [SECONDS:WHAT]...
//!
//! KEY: Pyr's name for the key (`hotkey::KEY_CHOICES`). WHAT, that many seconds in: another key,
//! `off` (push to talk's not the input mode any more), `on`, or `end`.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

fn main() {
    // (What it says on the way, with RUST_LOG=info.)
    let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).with_writer(std::io::stderr).try_init();
    let mut args = std::env::args().skip(1);
    let mut key = args.next().unwrap_or_else(|| "Mouse4".into());
    let mut steps: Vec<(f32, String)> = args.filter_map(|a| a.split_once(':').and_then(|(t, w)| Some((t.parse().ok()?, w.to_string())))).collect();
    steps.sort_by(|a, b| a.0.total_cmp(&b.0));
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().expect("a runtime");
    let down = Arc::new(AtomicBool::new(false));
    let start = Instant::now();
    let w = pyr_engine::hotkey::PttWatcher::start(key.clone(), true, down.clone(), Some(rt.handle().clone()), None, Arc::new(|| {}));
    let mut was = (false, None);
    let mut on = true;
    loop {
        let now = (down.load(Ordering::Relaxed), w.wayland_note());
        if now != was {
            println!("{:6.2}s down={} note={:?}", start.elapsed().as_secs_f32(), now.0, now.1);
            was = now;
        }
        if let Some((at, what)) = steps.first().cloned()
            && start.elapsed().as_secs_f32() >= at
        {
            steps.remove(0);
            println!("{:6.2}s -> {what}", start.elapsed().as_secs_f32());
            match what.as_str() {
                "end" => break,
                "off" => on = false,
                "on" => on = true,
                k => key = k.to_string(),
            }
            w.set(&key, on);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(w);
    println!("{:6.2}s ended", start.elapsed().as_secs_f32());
}
