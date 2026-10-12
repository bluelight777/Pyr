//! Voice sounds, like Discord: you or someone else joining or leaving your voice channel, a
//! screen share starting or stopping there, and you tuning in or out of one. Worked out by
//! comparing each frame's state with the last. (Message pings are played by the engine.)

use std::collections::{HashMap, HashSet};

use ingle_engine::Engine;
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
}

impl VoiceSounds {
    pub fn step(&mut self, st: &AppState, engine: &Engine) {
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

        let same_call = call.is_some() && call == self.call;
        if call != self.call {
            match (&self.call, &call) {
                (None, Some(_)) | (Some(_), Some(_)) => engine.play_sound(Sound::Join),
                (Some(_), None) => engine.play_sound(Sound::Leave),
                (None, None) => {}
            }
        } else if same_call {
            let my_session = call.as_ref().map(|c| c.2);
            let joined = people.keys().any(|s| !self.people.contains_key(s) && Some(*s) != my_session);
            let left = self.people.keys().any(|s| !people.contains_key(s) && Some(*s) != my_session);
            let started = people.iter().any(|(s, on)| *on && self.people.get(s) == Some(&false));
            let stopped = people.iter().any(|(s, on)| !*on && self.people.get(s) == Some(&true));
            if joined {
                engine.play_sound(Sound::Join);
            }
            if left {
                engine.play_sound(Sound::Leave);
            }
            if started {
                engine.play_sound(Sound::StreamStart);
            }
            if stopped {
                engine.play_sound(Sound::StreamStop);
            }
            // Tuning out on purpose (the stream itself still going) gets its own blip.
            if watching.iter().any(|k| !self.watching.contains(k)) {
                engine.play_sound(Sound::WatchStart);
            }
            if self.watching.iter().any(|k| !watching.contains(k) && people.get(&k.session) == Some(&true)) {
                engine.play_sound(Sound::WatchStop);
            }
        }
        self.call = call;
        self.people = people;
        self.watching = watching;
    }
}
