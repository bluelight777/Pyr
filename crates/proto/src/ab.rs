//! Side-by-side tests of how calls behave over slow or far-away networks: `PYR_AB` (a comma
//! list) puts back how some part of the media worked before 0.8.0, one at a time, so the old
//! and new ways can be measured back to back with only that one thing different. Nobody else
//! sets it; unset, everything works the new way.
//!
//! The parts: `jitter` (voice: pauses counted as jitter), `recv` (video: a missing frame waited
//! for 120 ms or 8 frames), `pace` (video: at most ~120 ms of frames in flight),
//! `adapt` (video: the preset's size, frame rate and bitrate, whatever the network),
//! `scenecut` (video: a keyframe at every big change), `fair` (QUIC: frames sent side by side),
//! `dgram` (QUIC: seconds of sound queued), `feedback` (server: no reports on viewers),
//! `shed` (server: a viewer counted as stuck after 350 ms plus a round trip), `isoref` (voice
//! isolation: your usual level followed louder moments quickly, before 0.9.1).
//!
//! `PYR_CC=cubic` puts back QUIC's CUBIC congestion control instead of BBR (see [`bbr`]).

use std::sync::OnceLock;

/// Whether `part` works the way it did before (see the module's documentation).
pub fn old(part: &str) -> bool {
    static PARTS: OnceLock<Vec<String>> = OnceLock::new();
    PARTS
        .get_or_init(|| crate::env_var("AB").map(|v| v.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()).unwrap_or_default())
        .iter()
        .any(|p| p == part || p == "all")
}

/// QUIC connections use BBR congestion control (0.8.0), unless `PYR_CC=cubic`.
///
/// CUBIC (QUIC's usual one) halves what it sends at every lost packet, so on a long road with
/// a little loss (Australia to a server in the US: a 200 ms round trip, a packet in a thousand
/// lost) it couldn't keep a 3 Mbit/s screen share going: frames piled up behind it and
/// arrived a second late. BBR goes by the bandwidth and round trip it measures instead.
/// Measured with the same share over that road, emulated, with only this different: 9
/// freezes in 50 s and the slowest 5% of frames 0.9 s late or more with CUBIC; none, and
/// 0.29 s, with BBR. On a short road (8 ms) it did as well or better.
pub fn bbr() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| !crate::env_var("CC").is_some_and(|v| v.eq_ignore_ascii_case("cubic")))
}
