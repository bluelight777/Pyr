//! What can be shared, and frames of one of them: for trying screen capture (Wayland's
//! wlr-screencopy, X11's) on a test screen.
//!
//!     screen_grab [ID] [OUT.ppm] [FRAMES]
//!
//! Lists the sources; then takes FRAMES frames (1) of ID (the first monitor) at 10 a second,
//! saying how big and how long each took, and writes the last as a PPM.

use std::sync::Arc;
use std::time::{Duration, Instant};

use pyr_engine::video::capture::{FrameSlot, SourceKind, list_screens, start_screen};

fn main() {
    let _ = tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).with_writer(std::io::stderr).try_init();
    let mut args = std::env::args().skip(1);
    let screens = list_screens();
    for s in &screens {
        println!("{} | {} | {}x{} | {:?}", s.id, s.name, s.width, s.height, s.kind);
    }
    let id = args.next().unwrap_or_else(|| screens.iter().find(|s| s.kind == SourceKind::Monitor).map(|s| s.id.clone()).unwrap_or_default());
    let out = args.next().unwrap_or_else(|| "grab.ppm".into());
    let frames: usize = args.next().and_then(|n| n.parse().ok()).unwrap_or(1);
    let slot = Arc::new(FrameSlot::default());
    let started = Instant::now();
    let handle = match start_screen(&id, 10, slot.clone()) {
        Ok(h) => h,
        Err(e) => {
            println!("couldn't start {id}: {e}");
            std::process::exit(1);
        }
    };
    let mut last = None;
    for i in 0..frames {
        match slot.take_wait(Duration::from_secs(5)) {
            Some(f) => {
                println!("frame {}: {}x{} after {:.2} s", i + 1, f.width, f.height, started.elapsed().as_secs_f32());
                last = Some(f);
            }
            None => {
                println!("frame {}: none in 5 s", i + 1);
                break;
            }
        }
    }
    drop(handle);
    let Some(f) = last else { std::process::exit(1) };
    let mut ppm = format!("P6\n{} {}\n255\n", f.width, f.height).into_bytes();
    for px in f.data.chunks_exact(4) {
        ppm.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    std::fs::write(&out, ppm).expect("write");
    println!("wrote {out}");
}
