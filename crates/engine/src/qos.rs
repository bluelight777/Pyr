//! Voice packets first: marks Ingle's traffic as voice on Windows (qWAVE, as Discord's
//! "Quality of Service High Packet Priority" does), so home routers and Wi-Fi send it ahead
//! of downloads and other traffic. Voice gets Windows' "Voice" class (DSCP 56, Wi-Fi's voice
//! queue); while you send video on the same connection it steps down to "Audio/Video"
//! (DSCP 40, Wi-Fi's video queue), so a screen share doesn't crowd the voice queue.
//!
//! Inside the connection voice already goes first: QUIC packs datagrams (voice) ahead of
//! stream data (video, chat) in every packet.
//!
//! Elsewhere (Linux, macOS) this does nothing: the QUIC library sets the whole TOS byte of
//! every packet itself.

use std::net::SocketAddr;
use std::sync::OnceLock;

use parking_lot::Mutex;

struct Flow {
    socket: u64,
    dest: SocketAddr,
    /// The flow while marked (`None`: not marked, or Windows refused).
    id: Option<u32>,
}

#[derive(Default)]
#[cfg_attr(not(windows), allow(dead_code))]
struct State {
    enabled: bool,
    video: bool,
    flows: Vec<Flow>,
    /// The qWAVE handle (Windows), opened on first use; `Some(0)` = unavailable.
    handle: Option<isize>,
    warned: bool,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State { enabled: true, ..Default::default() }))
}

/// A connection to `dest` from the UDP socket `socket` (see `bind_endpoint_raw`).
pub fn watch(socket: u64, dest: SocketAddr) {
    let mut st = state().lock();
    if st.flows.iter().any(|f| f.socket == socket && f.dest == dest) {
        return;
    }
    st.flows.push(Flow { socket, dest, id: None });
    if st.enabled {
        sys::apply(&mut st);
    }
}

/// The setting (on by default; a few routers misbehave with marked packets).
pub fn set_enabled(on: bool) {
    let mut st = state().lock();
    if st.enabled != on {
        st.enabled = on;
        sys::apply(&mut st);
    }
}

/// Whether you're sending video (camera or screen) right now.
pub fn set_video(on: bool) {
    let mut st = state().lock();
    if st.video != on {
        st.video = on;
        sys::apply(&mut st);
    }
}

/// For tests and the log: how many connections are marked, and as what.
pub fn marked() -> (usize, &'static str) {
    let st = state().lock();
    let n = st.flows.iter().filter(|f| f.id.is_some()).count();
    (n, if st.video { "audio/video" } else { "voice" })
}

#[cfg(windows)]
mod sys {
    use super::State;
    use std::net::SocketAddr;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::NetworkManagement::QoS::{
        QOS_NON_ADAPTIVE_FLOW, QOS_TRAFFIC_TYPE, QOS_VERSION, QOSAddSocketToFlow, QOSCreateHandle, QOSRemoveSocketFromFlow,
        QOSSetFlow, QOSSetTrafficType, QOSTrafficTypeAudioVideo, QOSTrafficTypeVoice,
    };
    use windows::Win32::Networking::WinSock::{SOCKADDR, SOCKET};

    fn sockaddr(a: &SocketAddr) -> Vec<u8> {
        // SOCKADDR_IN / SOCKADDR_IN6 laid out by hand (family, port in network order, address).
        match a {
            SocketAddr::V4(v4) => {
                let mut b = vec![0u8; 16];
                b[0..2].copy_from_slice(&2u16.to_le_bytes()); // AF_INET
                b[2..4].copy_from_slice(&v4.port().to_be_bytes());
                b[4..8].copy_from_slice(&v4.ip().octets());
                b
            }
            SocketAddr::V6(v6) => {
                let mut b = vec![0u8; 28];
                b[0..2].copy_from_slice(&23u16.to_le_bytes()); // AF_INET6
                b[2..4].copy_from_slice(&v6.port().to_be_bytes());
                b[4..8].copy_from_slice(&v6.flowinfo().to_be_bytes());
                b[8..24].copy_from_slice(&v6.ip().octets());
                b[24..28].copy_from_slice(&v6.scope_id().to_le_bytes());
                b
            }
        }
    }

    pub fn apply(st: &mut State) {
        let handle = match st.handle {
            Some(h) => h,
            None => {
                let mut h = HANDLE::default();
                let v = QOS_VERSION { MajorVersion: 1, MinorVersion: 0 };
                let ok = unsafe { QOSCreateHandle(&v, &mut h) }.as_bool();
                let h = if ok { h.0 as isize } else { 0 };
                if !ok {
                    tracing::warn!("voice priority: Windows QoS isn't available: {}", windows::core::Error::from_thread());
                }
                st.handle = Some(h);
                h
            }
        };
        if handle == 0 {
            return;
        }
        let h = HANDLE(handle as *mut core::ffi::c_void);
        let kind: QOS_TRAFFIC_TYPE = if st.video { QOSTrafficTypeAudioVideo } else { QOSTrafficTypeVoice };
        let (enabled, mut warned) = (st.enabled, st.warned);
        for f in st.flows.iter_mut() {
            match (enabled, f.id) {
                (true, None) => {
                    let addr = sockaddr(&f.dest);
                    let mut id = 0u32;
                    let ok = unsafe {
                        QOSAddSocketToFlow(h, SOCKET(f.socket as usize), Some(addr.as_ptr() as *const SOCKADDR), kind, Some(QOS_NON_ADAPTIVE_FLOW), &mut id)
                    }
                    .as_bool();
                    if ok {
                        tracing::info!(dest = %f.dest, "voice priority on");
                        f.id = Some(id);
                    } else if !warned {
                        warned = true;
                        tracing::warn!(dest = %f.dest, "voice priority: Windows refused: {}", windows::core::Error::from_thread());
                    }
                }
                (true, Some(id)) => {
                    let ok = unsafe {
                        QOSSetFlow(h, id, QOSSetTrafficType, std::mem::size_of::<QOS_TRAFFIC_TYPE>() as u32, &kind as *const _ as *const _, None, None)
                    }
                    .as_bool();
                    if !ok && !warned {
                        warned = true;
                        tracing::warn!("voice priority: couldn't change the traffic type: {}", windows::core::Error::from_thread());
                    }
                }
                (false, Some(id)) => {
                    unsafe {
                        let _ = QOSRemoveSocketFromFlow(h, Some(SOCKET(f.socket as usize)), id, None);
                    }
                    f.id = None;
                }
                (false, None) => {}
            }
        }
        st.warned = warned;
    }
}

#[cfg(not(windows))]
mod sys {
    pub fn apply(_: &mut super::State) {}
}
