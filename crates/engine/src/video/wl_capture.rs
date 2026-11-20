//! Screen capture on Wayland (0.16.4), through the wlr-screencopy protocol: Hyprland, Sway, river,
//! labwc, Wayfire and the others built that way. Blue, Nov 20 2026: "Prognostic is sharing his
//! screen on Omarchy/linux and when I click watch stream it doesn't work." Omarchy is Hyprland:
//! Pyr copied the X11 screen there (XWayland's), which holds only the X11 programs' windows, so
//! what went out was empty. Now each monitor is offered by its name, and its picture copied as
//! the compositor shows it (the pointer in it) into memory shared with it, a frame at a time.
//! KDE and GNOME don't offer this (they share the screen through the desktop portal and
//! PipeWire, not here yet): there it's X11's as before.

use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_frame_v1::{self, ZwlrScreencopyFrameV1};
use wayland_protocols_wlr::screencopy::v1::client::zwlr_screencopy_manager_v1::ZwlrScreencopyManagerV1;

use super::capture::{CaptureHandle, FrameSlot, ScreenSource, SourceKind};
use super::codec::{PixelFormat, RawFrame};

/// A monitor as the compositor tells it.
struct Out {
    output: wl_output::WlOutput,
    /// Its connector ("DP-1"), and what the compositor calls it ("Dell Inc. DELL U2720Q (DP-1)").
    name: String,
    desc: String,
    size: (i32, i32),
}

/// One frame being copied.
#[derive(Default)]
struct Frame {
    /// What it's to be copied into (shared memory): format, width, height, bytes a row.
    shm: Option<(WEnum<wl_shm::Format>, u32, u32, u32)>,
    /// (Version 3) every kind of buffer it takes has been said.
    told: bool,
    y_invert: bool,
    ready: bool,
    failed: bool,
}

#[derive(Default)]
struct State {
    outputs: Vec<Out>,
    frame: Frame,
}

/// The compositor, its monitors, and what copies them.
struct Session {
    queue: EventQueue<State>,
    state: State,
    shm: wl_shm::WlShm,
    manager: ZwlrScreencopyManagerV1,
    /// Whether the manager says when it's said every kind of buffer (version 3).
    told_version: bool,
}

impl Session {
    fn open() -> Result<Session> {
        let conn = Connection::connect_to_env().map_err(|e| anyhow!("can't reach the Wayland compositor: {e}"))?;
        let (globals, queue) = registry_queue_init::<State>(&conn).map_err(|e| anyhow!("the Wayland compositor's list: {e}"))?;
        let qh = queue.handle();
        let manager: ZwlrScreencopyManagerV1 = globals.bind(&qh, 1..=3, ()).map_err(|_| anyhow!("this desktop doesn't let programs copy the screen (no wlr-screencopy)"))?;
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).map_err(|e| anyhow!("no shared memory from the compositor: {e}"))?;
        let mut state = State::default();
        for g in globals.contents().clone_list() {
            if g.interface == "wl_output" {
                let i = state.outputs.len();
                let output = globals.registry().bind::<wl_output::WlOutput, usize, State>(g.name, g.version.min(4), &qh, i);
                state.outputs.push(Out { output, name: String::new(), desc: String::new(), size: (0, 0) });
            }
        }
        let told_version = manager.version() >= 3;
        let mut s = Session { queue, state, shm, manager, told_version };
        // (Their names and sizes.)
        s.queue.roundtrip(&mut s.state).map_err(|e| anyhow!("the Wayland compositor stopped answering: {e}"))?;
        s.queue.roundtrip(&mut s.state).map_err(|e| anyhow!("the Wayland compositor stopped answering: {e}"))?;
        for (i, o) in s.state.outputs.iter_mut().enumerate() {
            if o.name.is_empty() {
                o.name = format!("Monitor {}", i + 1);
            }
        }
        Ok(s)
    }
}

/// The monitors to share, when the compositor lets programs copy them (none otherwise).
pub fn screens() -> Vec<ScreenSource> {
    match Session::open() {
        Ok(s) => s
            .state
            .outputs
            .iter()
            // (Named as the compositor describes it, as Windows' are by Windows; known by its
            // connector.)
            .map(|o| ScreenSource {
                id: format!("wl:{}", o.name),
                name: if o.desc.trim().is_empty() { o.name.clone() } else { o.desc.trim().to_string() },
                kind: SourceKind::Monitor,
                width: o.size.0.max(0) as u32,
                height: o.size.1.max(0) as u32,
            })
            .collect(),
        Err(e) => {
            tracing::info!("screen sharing: {e}; X11's screen instead");
            Vec::new()
        }
    }
}

/// Shared memory a frame's copied into, kept while frames stay that size.
struct Shm {
    pool: wl_shm_pool::WlShmPool,
    buffer: wl_buffer::WlBuffer,
    map: memmap2::MmapMut,
    key: (u32, u32, u32, u32),
}

impl Drop for Shm {
    fn drop(&mut self) {
        self.buffer.destroy();
        self.pool.destroy();
    }
}

fn shared_memory(size: usize) -> Result<(OwnedFd, memmap2::MmapMut)> {
    // SAFETY: memfd_create with a valid name; the descriptor is checked and then owned.
    let fd = unsafe { libc::memfd_create(c"pyr-screen".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(anyhow!("no shared memory: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: fd is a new descriptor of our own.
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let file = std::fs::File::from(fd.try_clone()?);
    file.set_len(size as u64)?;
    // SAFETY: the memory is ours and the compositor's; it's only read once it says the copy's done.
    let map = unsafe { memmap2::MmapMut::map_mut(&file)? };
    Ok((fd, map))
}

/// The copy's pixels as Pyr's frames have them (BGRA, top row first).
pub fn to_bgra(src: &[u8], format: wl_shm::Format, width: u32, height: u32, stride: u32, y_invert: bool) -> Option<Vec<u8>> {
    let (w, h, stride) = (width as usize, height as usize, stride as usize);
    if stride < w * 4 || src.len() < stride * h {
        return None;
    }
    // (In memory, little-endian: Xrgb8888/Argb8888 are B G R A; Xbgr8888/Abgr8888 are R G B A.)
    let swap = match format {
        wl_shm::Format::Xrgb8888 | wl_shm::Format::Argb8888 => false,
        wl_shm::Format::Xbgr8888 | wl_shm::Format::Abgr8888 => true,
        _ => return None,
    };
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        let from = if y_invert { h - 1 - y } else { y };
        let row = &src[from * stride..from * stride + w * 4];
        let to = &mut out[y * w * 4..(y + 1) * w * 4];
        to.copy_from_slice(row);
        for px in to.chunks_exact_mut(4) {
            if swap {
                px.swap(0, 2);
            }
            // (Opaque: "X" formats leave it unset.)
            px[3] = 255;
        }
    }
    Some(out)
}

/// Copies monitor `id` ("wl:<its name>") `fps` times a second into `slot`.
pub fn start(id: &str, fps: u32, slot: Arc<FrameSlot>) -> Result<CaptureHandle> {
    let name = id.strip_prefix("wl:").ok_or_else(|| anyhow!("unknown capture source"))?.to_string();
    let mut s = Session::open()?;
    let which = s.state.outputs.iter().position(|o| o.name == name).ok_or_else(|| anyhow!("that monitor isn't there any more ({name})"))?;
    let stop = Arc::new(AtomicBool::new(false));
    let st = stop.clone();
    let t = std::thread::Builder::new()
        .name("pyr-wlcapture".into())
        .spawn(move || {
            let period = Duration::from_secs_f64(1.0 / fps.max(1) as f64);
            let mut next = Instant::now();
            let mut shm: Option<Shm> = None;
            let mut said = false;
            while !st.load(Ordering::Relaxed) {
                match copy_frame(&mut s, which, &mut shm) {
                    Ok(Some(frame)) => slot.put(frame),
                    Ok(None) => {}
                    Err(e) => {
                        if !said {
                            tracing::warn!("screen sharing (Wayland): {e}");
                            said = true;
                        }
                        if e.to_string().contains("stopped answering") {
                            return;
                        }
                    }
                }
                next += period;
                match next.checked_duration_since(Instant::now()) {
                    Some(d) => std::thread::sleep(d),
                    None => next = Instant::now(),
                }
            }
        })
        .map_err(|e| anyhow!("couldn't start capturing: {e}"))?;
    Ok(CaptureHandle::thread(stop, t))
}

/// One frame of monitor `which`: asked for, copied into shared memory, read out.
fn copy_frame(s: &mut Session, which: usize, shm: &mut Option<Shm>) -> Result<Option<RawFrame>> {
    let qh = s.queue.handle();
    s.state.frame = Frame::default();
    let frame = s.manager.capture_output(1, &s.state.outputs[which].output, &qh, ());
    let gone = |e: wayland_client::DispatchError| anyhow!("the Wayland compositor stopped answering: {e}");
    // What it's to be copied into (with version 3, once every kind's been said).
    while !(s.state.frame.failed || s.state.frame.shm.is_some() && (!s.told_version || s.state.frame.told)) {
        s.queue.blocking_dispatch(&mut s.state).map_err(gone)?;
    }
    if s.state.frame.failed {
        frame.destroy();
        return Err(anyhow!("the compositor wouldn't copy the monitor"));
    }
    let (format, width, height, stride) = s.state.frame.shm.expect("said");
    let WEnum::Value(fmt) = format else {
        frame.destroy();
        return Err(anyhow!("the compositor's copy is in a format Pyr doesn't know"));
    };
    let key = (fmt as u32, width, height, stride);
    if shm.as_ref().is_none_or(|m| m.key != key) {
        *shm = None;
        let size = stride as usize * height as usize;
        let (fd, map) = shared_memory(size)?;
        let pool = s.shm.create_pool(fd.as_fd(), size as i32, &qh, ());
        let buffer = pool.create_buffer(0, width as i32, height as i32, stride as i32, fmt, &qh, ());
        *shm = Some(Shm { pool, buffer, map, key });
    }
    let m = shm.as_ref().expect("made");
    frame.copy(&m.buffer);
    while !(s.state.frame.ready || s.state.frame.failed) {
        s.queue.blocking_dispatch(&mut s.state).map_err(gone)?;
    }
    frame.destroy();
    if s.state.frame.failed {
        return Err(anyhow!("the compositor couldn't copy the monitor"));
    }
    let data = to_bgra(&m.map, fmt, width, height, stride, s.state.frame.y_invert).ok_or_else(|| anyhow!("the compositor's copy is in a format Pyr doesn't take ({fmt:?})"))?;
    Ok(Some(RawFrame { width, height, format: PixelFormat::Bgra, data }))
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_output::WlOutput, usize> for State {
    fn event(state: &mut Self, _: &wl_output::WlOutput, event: wl_output::Event, i: &usize, _: &Connection, _: &QueueHandle<Self>) {
        let Some(o) = state.outputs.get_mut(*i) else { return };
        match event {
            wl_output::Event::Name { name } => o.name = name,
            wl_output::Event::Description { description } => o.desc = description,
            wl_output::Event::Mode { flags: WEnum::Value(flags), width, height, .. } if flags.contains(wl_output::Mode::Current) => o.size = (width, height),
            _ => {}
        }
    }
}

impl Dispatch<wl_shm::WlShm, ()> for State {
    fn event(_: &mut Self, _: &wl_shm::WlShm, _: wl_shm::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for State {
    fn event(_: &mut Self, _: &wl_shm_pool::WlShmPool, _: wl_shm_pool::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<wl_buffer::WlBuffer, ()> for State {
    fn event(_: &mut Self, _: &wl_buffer::WlBuffer, _: wl_buffer::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwlrScreencopyManagerV1, ()> for State {
    fn event(_: &mut Self, _: &ZwlrScreencopyManagerV1, _: <ZwlrScreencopyManagerV1 as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwlrScreencopyFrameV1, ()> for State {
    fn event(state: &mut Self, _: &ZwlrScreencopyFrameV1, event: zwlr_screencopy_frame_v1::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        use zwlr_screencopy_frame_v1::Event as E;
        let f = &mut state.frame;
        match event {
            E::Buffer { format, width, height, stride } => f.shm = Some((format, width, height, stride)),
            E::Flags { flags: WEnum::Value(flags) } => f.y_invert = flags.contains(zwlr_screencopy_frame_v1::Flags::YInvert),
            E::BufferDone => f.told = true,
            E::Ready { .. } => f.ready = true,
            E::Failed => f.failed = true,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_read_as_pyrs_frames_are() {
        // Two pixels a row and a spare byte pair (stride 10), two rows: blue then red, as
        // Xrgb8888 has them in memory (B G R X).
        let src = [255, 0, 0, 7, 255, 0, 0, 7, 9, 9, 0, 0, 255, 7, 0, 0, 255, 7, 9, 9];
        let out = to_bgra(&src, wl_shm::Format::Xrgb8888, 2, 2, 10, false).unwrap();
        assert_eq!(out, [255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 255, 255, 0, 0, 255, 255]);
        // Upside down: the last row first.
        let flipped = to_bgra(&src, wl_shm::Format::Xrgb8888, 2, 2, 10, true).unwrap();
        assert_eq!(&flipped[..8], &[0, 0, 255, 255, 0, 0, 255, 255]);
        // R G B order (Xbgr8888) turned round.
        let rgb = to_bgra(&[1, 2, 3, 0], wl_shm::Format::Xbgr8888, 1, 1, 4, false).unwrap();
        assert_eq!(rgb, [3, 2, 1, 255]);
        // Too short, or a format it doesn't take: nothing.
        assert!(to_bgra(&src[..10], wl_shm::Format::Xrgb8888, 2, 2, 10, false).is_none());
        assert!(to_bgra(&src, wl_shm::Format::Rgb565, 2, 2, 10, false).is_none());
    }
}
