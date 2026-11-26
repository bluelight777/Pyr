//! Calls in direct messages ringing (see `DM_CALLS_FEATURE`): the ringtone while someone's
//! calling you, the ringback while you're calling someone, and what happens when nobody
//! answers. On a little thread of its own, so it rings with Pyr's window hidden too (the
//! window draws nothing then).

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use parking_lot::RwLock;

use crate::Shared;
use crate::config::Ringtone;
use crate::sfx::Sound;
use crate::state::{AppState, RING_FOR, RingState};

/// How often the ringtone and the ringback play.
pub(crate) const RING_EVERY: Duration = Duration::from_millis(2000);
const RINGBACK_EVERY: Duration = Duration::from_millis(3000);
/// A call we started but aren't in (joining it failed, say) stops ringing after this.
const JOIN_GRACE: Duration = Duration::from_secs(20);

/// Starts the ringer, unless it's going already. It stops by itself once nothing rings.
pub(crate) fn ensure(shared: &Arc<Shared>) {
    if shared.ringer.swap(true, Ordering::SeqCst) {
        return;
    }
    let s = shared.clone();
    let started = std::thread::Builder::new().name("pyr-ring".into()).spawn(move || {
        run(&s);
        s.ringer.store(false, Ordering::SeqCst);
        // Something started ringing just as this was stopping: keep going.
        if busy(&s) {
            ensure(&s);
        }
    });
    if started.is_err() {
        shared.ringer.store(false, Ordering::SeqCst);
    }
}

/// Something rings: a call coming in, or one we started that isn't answered yet.
fn busy(shared: &Shared) -> bool {
    let st = shared.state.read_recursive();
    !st.incoming_calls.is_empty() || st.outgoing_call.as_ref().is_some_and(|o| o.state == RingState::Ringing)
}

/// (0.16.8) The ringtone calls ring with when none's been chosen: Sunrise, one of the ten
/// ringtones every Pyr has (Blue, Nov 26: "make the sunrise ringtone the default ringtone on
/// Pyr"). Until 0.16.8 it was Pyr's own ring, which is still there to choose.
pub const DEFAULT_RING: &str = "ring-sunrise";

/// The default ringtone (see `DEFAULT_RING`).
pub fn default_ringtone() -> Ringtone {
    crate::soundboard::RINGS
        .iter()
        .find(|b| b.id == DEFAULT_RING)
        .map(|b| Ringtone { name: b.name.to_string(), sound: Some(b.sound_ref()), file: String::new() })
        .unwrap_or_else(Ringtone::pyr_ring)
}

/// The ringtone for a call from `from`: their own, or the one for everyone, or the default.
pub fn ringtone_for(st: &AppState, from: Option<&str>) -> Ringtone {
    from.and_then(|u| st.settings.ringtones.get(u).cloned())
        .or_else(|| st.settings.ringtone.clone())
        .unwrap_or_else(default_ringtone)
}

/// A ringtone that's one of the sounds every Pyr has (the ten ringtones, the soundboard's),
/// decoded right here if it isn't yet: so a call's very first ring is its own ringtone, not
/// Pyr's ring while it gets ready (the default's first of all). None for anything else.
pub(crate) fn built_in_now(shared: &Shared, tone: &Ringtone) -> Option<Arc<Vec<f32>>> {
    let snd = tone.sound.as_ref().filter(|s| s.server.is_empty())?;
    let key = tone.key();
    // (Never called with the cache's lock held: it takes it, here and below.)
    let cached = shared.ring_clips.lock().get(&key).cloned();
    if cached.is_some() {
        return cached;
    }
    let pcm = Arc::new(crate::soundboard::decode(crate::soundboard::builtin_file(&snd.id)?).ok()?);
    shared.ring_clips.lock().insert(key, pcm.clone());
    Some(pcm)
}

/// Gets a ringtone's sound ready to play (fetched and decoded in the background, once).
pub(crate) fn warm(shared: &Arc<Shared>, tone: &Ringtone) {
    // (Pyr's own ring is made in its code: nothing to get.)
    if tone.is_pyr_ring() {
        return;
    }
    let key = tone.key();
    if shared.ring_clips.lock().contains_key(&key) {
        return;
    }
    let (s, tone) = (shared.clone(), tone.clone());
    shared.rt.spawn(async move {
        let pcm: anyhow::Result<Arc<Vec<f32>>> = match &tone.sound {
            Some(snd) => s.sound_pcm(&snd.server, &snd.id).await.map(|p| p.clone()),
            None => {
                let path = s.config_dir.join(RINGS_DIR).join(&tone.file);
                match tokio::task::spawn_blocking(move || -> anyhow::Result<Arc<Vec<f32>>> { Ok(Arc::new(crate::soundboard::decode(&std::fs::read(path)?)?)) }).await {
                    Ok(r) => r,
                    Err(e) => Err(anyhow::anyhow!("{e}")),
                }
            }
        };
        match pcm {
            Ok(p) => {
                s.ring_clips.lock().insert(key, p);
            }
            Err(e) => tracing::warn!("couldn't get the ringtone {} ready: {e:#}", tone.name),
        }
    });
}

/// Where sound files from this PC chosen as ringtones are kept (in the config folder).
pub const RINGS_DIR: &str = "rings";

fn run(shared: &Arc<Shared>) {
    // When the ringtone last started, and how long to wait before it plays again.
    let mut last_ring: Option<(Instant, Duration)> = None;
    let mut last_back: Option<Instant> = None;
    loop {
        let (now, changed) = step(&shared.state, Instant::now());
        if changed {
            (shared.repaint)();
        }
        let Some((ring, back)) = now else { return };
        // The ringtone, and (when nothing's coming in) the ringback.
        if ring {
            if last_ring.is_none_or(|(t, every)| t.elapsed() >= every) {
                let every = shared.play_ringtone();
                last_ring = Some((Instant::now(), every));
            }
        } else {
            last_ring = None;
        }
        if back && !ring {
            if last_back.is_none_or(|t| t.elapsed() >= RINGBACK_EVERY) {
                shared.play_ring(Sound::Ringback);
                last_back = Some(Instant::now());
            }
        } else if !back {
            last_back = None;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Settles calls that rang out (missed ones, and ours nobody answered), and says what should
/// sound now: (the ringtone, the ringback), `None` once nothing rings; and whether anything
/// changed (to show).
fn step(state: &RwLock<AppState>, now: Instant) -> (Option<(bool, bool)>, bool) {
    let mut changed = false;
    let result = {
        let mut st = state.write();
        // Calls coming in that rang out: missed.
        let mut missed = Vec::new();
        st.incoming_calls.retain(|c| {
            let keep = now.saturating_duration_since(c.since) < RING_FOR;
            if !keep {
                missed.push(c.clone());
            }
            keep
        });
        for c in missed {
            let who = st.server(&c.server_key).map(|s| s.member_name(&c.from)).unwrap_or_default();
            st.missed_call(&c.server_key, c.channel_id, &who);
            changed = true;
        }
        // The call we started.
        let mut back = false;
        let mut ours = false;
        if let Some(o) = st.outgoing_call.clone() {
            let here = st.call.as_ref().is_some_and(|c| c.server_key == o.server_key && c.channel_id == o.channel_id);
            let age = now.saturating_duration_since(o.since);
            if o.state == RingState::Ringing {
                if !here && age > JOIN_GRACE {
                    st.outgoing_call = None;
                    changed = true;
                } else if age >= RING_FOR {
                    if let Some(x) = st.outgoing_call.as_mut() {
                        x.state = RingState::NoAnswer;
                    }
                    changed = true;
                } else {
                    // The ringback, while we're there on our own (in a group, once someone's
                    // answered, the others ring on quietly).
                    let company = st.server(&o.server_key).is_some_and(|s| s.voice_in(o.channel_id).iter().any(|p| p.user_id != st.my_uid));
                    back = here && !company;
                    ours = true;
                }
            }
        }
        let ring = !st.incoming_calls.is_empty();
        (ring || ours).then_some((ring, back))
    };
    (result, changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{IncomingCall, OutgoingCall};

    fn call_view(key: &str, channel_id: i64) -> crate::state::CallView {
        crate::state::CallView {
            server_key: key.into(),
            channel_id,
            session: 1,
            muted: false,
            deafened: false,
            camera_on: false,
            screen_on: false,
            stream_audio_on: false,
            stream_muted: false,
            transmitting: false,
            speaking: Default::default(),
            watching: Default::default(),
            call_code: String::new(),
            e2ee_ready: false,
            stats: Default::default(),
            audio_error: None,
            video_error: None,
            joined_at: Instant::now(),
            sounds: Vec::new(),
        }
    }

    fn incoming(age: Duration, now: Instant) -> IncomingCall {
        IncomingCall { server_key: "k".into(), channel_id: 7, from: "them".into(), video: false, since: now - age }
    }

    #[test]
    fn settings_from_before_ring_with_sunrise() {
        // (0.16.7 and before saved no choice as null; Pyr's ring was what that meant then.)
        let old: crate::config::Settings = serde_json::from_str(r#"{"ring": true, "ringtone": null, "ringtones": {}}"#).unwrap();
        let mut st = AppState::default();
        st.settings = old;
        let t = ringtone_for(&st, Some("anyone"));
        assert_eq!(t.name, "Sunrise");
        // Its sound is built in, and decodes: about seven seconds of 48 kHz stereo.
        let file = crate::soundboard::builtin_file(DEFAULT_RING).expect("Sunrise is built in");
        let pcm = crate::soundboard::decode(file).unwrap();
        let secs = pcm.len() as f32 / 96_000.0;
        assert!((2.0..=10.5).contains(&secs), "{secs} s");
    }

    #[test]
    fn a_call_rings_with_their_own_ringtone_or_everyones() {
        let tone = |name: &str| Ringtone { name: name.into(), sound: None, file: format!("{name}.ings") };
        let name = |t: Ringtone| Some(t.name);
        let mut st = AppState::default();
        // (0.16.8) None chosen: Sunrise, the default.
        assert_eq!(ringtone_for(&st, Some("mara")), default_ringtone());
        assert_eq!(default_ringtone().name, "Sunrise");
        assert_eq!(default_ringtone().sound.map(|s| (s.server, s.id)), Some((String::new(), DEFAULT_RING.to_string())));
        st.settings.ringtone = Some(tone("bells"));
        assert_eq!(name(ringtone_for(&st, Some("mara"))).as_deref(), Some("bells"));
        assert_eq!(name(ringtone_for(&st, None)).as_deref(), Some("bells"));
        st.settings.ringtones.insert("mara".into(), tone("harp"));
        assert_eq!(name(ringtone_for(&st, Some("mara"))).as_deref(), Some("harp"));
        assert_eq!(name(ringtone_for(&st, Some("bob"))).as_deref(), Some("bells"));
        // Hers still, with everyone else back to the default.
        st.settings.ringtone = None;
        assert_eq!(name(ringtone_for(&st, Some("mara"))).as_deref(), Some("harp"));
        assert_eq!(ringtone_for(&st, Some("bob")), default_ringtone());
        // Pyr's own ring, chosen: kept as that (not the default) between runs.
        st.settings.ringtone = Some(Ringtone::pyr_ring());
        assert!(ringtone_for(&st, Some("bob")).is_pyr_ring());
        let back: crate::config::Settings = serde_json::from_str(&serde_json::to_string(&st.settings).unwrap()).unwrap();
        assert_eq!(back.ringtone, Some(Ringtone::pyr_ring()));
        assert!(!default_ringtone().is_pyr_ring());
        st.settings.ringtone = None;
        // Kept between runs.
        let json = serde_json::to_string(&st.settings).unwrap();
        let back: crate::config::Settings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ringtones, st.settings.ringtones);
        assert_eq!(back.ringtone, None);
        // Two ringtones of the same sound are the same sound, whatever they're called.
        assert_eq!(tone("harp").key(), Ringtone { name: "Harp (mine)".into(), ..tone("harp") }.key());
        assert_ne!(tone("harp").key(), tone("bells").key());
    }

    #[test]
    fn rings_run_out() {
        let now = Instant::now() + RING_FOR * 2;
        let mut st = AppState::default();
        st.incoming_calls.push(incoming(Duration::from_secs(5), now));
        st.incoming_calls.push(incoming(RING_FOR + Duration::from_secs(1), now));
        st.outgoing_call = Some(OutgoingCall {
            server_key: "k".into(),
            channel_id: 8,
            to: vec!["them".into()],
            declined: Vec::new(),
            video: false,
            since: now - RING_FOR,
            state: RingState::Ringing,
        });
        let state = RwLock::new(st);
        let (now_rings, changed) = step(&state, now);
        let (ring, back) = now_rings.expect("one call still rings");
        assert!(ring && changed);
        assert!(!back, "not in the call we started, so no ringback");
        let st = state.read();
        assert_eq!(st.incoming_calls.len(), 1, "the old one was missed");
        assert!(st.toasts.iter().any(|t| t.text.contains("missed a call")), "said so");
        assert_eq!(st.missed_calls, 1);
        assert_eq!(st.outgoing_call, None, "a call we aren't in (joining it failed) stops ringing");
        drop(st);
        state.write().incoming_calls.clear();
        assert!(step(&state, now).0.is_none(), "nothing rings now");

        // In the call we started, it rings until nobody's answered for a while.
        let mut st = state.write();
        st.call = Some(call_view("k", 8));
        st.outgoing_call = Some(OutgoingCall {
            server_key: "k".into(),
            channel_id: 8,
            to: vec!["them".into()],
            declined: Vec::new(),
            video: false,
            since: now - Duration::from_secs(5),
            state: RingState::Ringing,
        });
        drop(st);
        assert_eq!(step(&state, now).0, Some((false, true)), "the ringback");

        // A group's call: once someone's there with us, it rings on for the others, quietly.
        let mut s = crate::state::ServerView::new("k".into(), "k".into(), "127.0.0.1:1".into());
        s.voice.insert(
            2,
            pyr_proto::control::VoiceParticipant {
                session: 2,
                user_id: "first".into(),
                channel_id: 8,
                self_mute: false,
                self_deaf: false,
                server_mute: false,
                server_deaf: false,
                tracks: Vec::new(),
                e2ee_pub: String::new(),
                e2ee_sig: String::new(),
            },
        );
        state.write().servers.push(s);
        assert_eq!(step(&state, now).0, Some((false, false)), "still ringing, no ringback");
        assert_eq!(state.read().outgoing_call.as_ref().map(|o| o.state), Some(RingState::Ringing));

        state.write().outgoing_call.as_mut().unwrap().since = now - RING_FOR;
        assert_eq!(step(&state, now).0, None, "nobody answered");
        assert_eq!(state.read().outgoing_call.as_ref().map(|o| o.state), Some(RingState::NoAnswer));
    }
}
