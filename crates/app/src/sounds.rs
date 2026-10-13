//! Voice sounds, like Discord: you or someone else joining or leaving your voice channel, a
//! screen share starting or stopping there, and you tuning in or out of one. Worked out by
//! comparing each frame's state with the last. (Message pings are played by the engine.)
//!
//! `step` only says which sounds are due: the caller plays them once it has let go of the
//! shared state (playing reads the sound settings from it; see `Engine::state`).

use std::collections::{HashMap, HashSet};

use ingle_engine::sfx::Sound;
use ingle_engine::state::{AppState, VideoKey};
use ingle_proto::control::{ChannelId, SessionId, TrackKind};

#[derive(Default)]
pub struct VoiceSounds {
    /// (server, channel, my session) of the call last frame.
    call: Option<(String, ChannelId, SessionId)>,
    /// Everyone in that channel last frame, and whether they were sharing their screen.
    people: HashMap<SessionId, bool>,
    /// Streams I was watching.
    watching: HashSet<VideoKey>,
    /// Who was watching each screen share in the channel (by the sharer's session).
    viewers: HashMap<SessionId, HashSet<SessionId>>,
}

impl VoiceSounds {
    /// Compare with last frame; returns the sounds to play.
    pub fn step(&mut self, st: &AppState) -> Vec<Sound> {
        let mut out = Vec::new();
        let call = st.call.as_ref().map(|c| (c.server_key.clone(), c.channel_id, c.session));
        let server = call.as_ref().and_then(|(k, _, _)| st.server(k));
        let people: HashMap<SessionId, bool> = match (&call, server) {
            (Some((_, ch, _)), Some(s)) => s
                .voice
                .values()
                .filter(|p| p.channel_id == *ch)
                .map(|p| (p.session, p.tracks.iter().any(|t| t.kind == TrackKind::Screen)))
                .collect(),
            _ => HashMap::new(),
        };
        let is_screen = |k: &VideoKey| -> bool {
            server
                .and_then(|s| s.voice.get(&k.session))
                .map(|p| p.tracks.iter().any(|t| t.id == k.track && t.kind == TrackKind::Screen))
                .unwrap_or(false)
        };
        let watching: HashSet<VideoKey> = st.call.as_ref().map(|c| c.watching.iter().copied().filter(|k| is_screen(k)).collect()).unwrap_or_default();
        let viewers: HashMap<SessionId, HashSet<SessionId>> = match (&call, server) {
            (Some((_, ch, _)), Some(s)) => s
                .voice
                .values()
                .filter(|p| p.channel_id == *ch)
                .filter_map(|p| p.tracks.iter().find(|t| t.kind == TrackKind::Screen).map(|t| (p.session, t.viewers.iter().copied().collect())))
                .collect(),
            _ => HashMap::new(),
        };

        // Dropped and getting back in (the server restarting for an update, say): stay quiet,
        // and when back, take the channel as it is now without chiming for everyone.
        if call.is_none() && st.voice_rejoin.is_some() {
            return out;
        }
        let rejoined = matches!((&self.call, &call), (Some(a), Some(b)) if a.0 == b.0 && a.1 == b.1 && a.2 != b.2);
        if rejoined {
            self.call = call;
            self.people = people;
            self.watching = watching;
            self.viewers = viewers;
            return out;
        }
        let same_call = call.is_some() && call == self.call;
        if call != self.call {
            match (&self.call, &call) {
                (None, Some(_)) | (Some(_), Some(_)) => out.push(Sound::Join),
                (Some(_), None) => out.push(Sound::Leave),
                (None, None) => {}
            }
        } else if same_call {
            let my_session = call.as_ref().map(|c| c.2);
            let joined = people.keys().any(|s| !self.people.contains_key(s) && Some(*s) != my_session);
            let left = self.people.keys().any(|s| !people.contains_key(s) && Some(*s) != my_session);
            let started = people.iter().any(|(s, on)| *on && self.people.get(s) == Some(&false));
            let stopped = people.iter().any(|(s, on)| !*on && self.people.get(s) == Some(&true));
            if joined {
                out.push(Sound::Join);
            }
            if left {
                out.push(Sound::Leave);
            }
            if started {
                out.push(Sound::StreamStart);
            }
            if stopped {
                out.push(Sound::StreamStop);
            }
            // Tuning out on purpose (the stream itself still going) gets its own blip.
            if watching.iter().any(|k| !self.watching.contains(k)) {
                out.push(Sound::WatchStart);
            }
            if self.watching.iter().any(|k| !watching.contains(k) && people.get(&k.session) == Some(&true)) {
                out.push(Sound::WatchStop);
            }
            // Someone else tuning in to (or out of) a stream in the call — your own included:
            // everyone in the call hears it, like Discord.
            let me = my_session.unwrap_or_default();
            let mut tuned_in = false;
            let mut tuned_out = false;
            for (sharer, now) in &viewers {
                let Some(before) = self.viewers.get(sharer) else { continue };
                tuned_in |= now.iter().any(|v| !before.contains(v) && *v != me);
                tuned_out |= before.iter().any(|v| !now.contains(v) && *v != me && people.contains_key(v));
            }
            if tuned_in {
                out.push(Sound::WatchStart);
            } else if tuned_out {
                out.push(Sound::WatchStop);
            }
        }
        self.call = call;
        self.people = people;
        self.watching = watching;
        self.viewers = viewers;
        out
    }
}
