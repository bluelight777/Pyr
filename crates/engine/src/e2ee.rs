//! Per-call end-to-end encryption state (see `ingle_proto::crypto` for the design).

use std::collections::HashMap;
use std::time::{Duration, Instant};

use ingle_proto::b64;
use ingle_proto::control::{ChannelId, SessionId, VoiceParticipant, WrappedKey};
use ingle_proto::crypto::{self, Identity, MediaKey, SessionKeys};
use tracing::debug;

/// How long after announcing a new key we start using it (lets it reach everyone first).
const ACTIVATE_DELAY: Duration = Duration::from_millis(300);
/// How long old epochs stay usable for decrypting in-flight media.
const KEEP_OLD: Duration = Duration::from_secs(6);

struct Peer {
    uid: String,
    session_pub: [u8; 32],
    verified: bool,
    /// (epoch, secret, when a newer one replaced it)
    epochs: Vec<(u16, [u8; 32], Option<Instant>)>,
    keys: HashMap<(u16, u8), MediaKey>,
}

pub struct E2ee {
    identity: Identity,
    my_uid: String,
    channel_id: ChannelId,
    session: SessionKeys,
    my_session: SessionId,
    epoch: u16,
    secret: [u8; 32],
    keys: HashMap<u8, MediaKey>,
    next: Option<(u16, [u8; 32], Instant)>,
    /// The active epoch's secret has been sent to the other participants. Media is only
    /// sent once this is true (nobody could decrypt it otherwise).
    distributed: bool,
    peers: HashMap<SessionId, Peer>,
}

impl E2ee {
    pub fn new(identity: &Identity, channel_id: ChannelId) -> E2ee {
        E2ee {
            my_uid: identity.user_id(),
            identity: identity.clone(),
            channel_id,
            session: SessionKeys::generate(),
            my_session: 0,
            epoch: 0,
            secret: crypto::random_bytes::<32>(),
            keys: HashMap::new(),
            next: None,
            distributed: false,
            peers: HashMap::new(),
        }
    }

    /// (public key, signature) to send with `VoiceJoin`.
    pub fn join_credentials(&self) -> (String, String) {
        let sig = self.session.sign_binding(&self.identity, self.channel_id);
        (b64::encode(&self.session.public()), b64::encode(&sig))
    }

    pub fn set_my_session(&mut self, s: SessionId) {
        self.my_session = s;
    }

    /// Add or refresh a participant. Returns true if this is a new, verified peer.
    pub fn upsert_peer(&mut self, p: &VoiceParticipant) -> bool {
        if p.session == self.my_session || p.channel_id != self.channel_id {
            return false;
        }
        let Some(pk) = b64::decode_array::<32>(&p.e2ee_pub) else { return false };
        if let Some(existing) = self.peers.get(&p.session) {
            if existing.session_pub == pk {
                return false;
            }
        }
        let sig = b64::decode(&p.e2ee_sig).unwrap_or_default();
        let verified = crypto::verify_binding(&p.user_id, self.channel_id, &pk, &sig);
        if !verified {
            debug!(session = p.session, "peer session key signature invalid; ignoring for E2EE");
        }
        self.peers.insert(
            p.session,
            Peer { uid: p.user_id.clone(), session_pub: pk, verified, epochs: Vec::new(), keys: HashMap::new() },
        );
        verified
    }

    pub fn remove_peer(&mut self, s: SessionId) -> bool {
        self.peers.remove(&s).is_some()
    }

    pub fn peer_count(&self) -> (usize, usize) {
        let verified = self.peers.values().filter(|p| p.verified).count();
        let with_keys = self.peers.values().filter(|p| !p.epochs.is_empty()).count();
        (verified, with_keys.min(self.peers.len()))
    }

    pub fn all_peers_keyed(&self) -> bool {
        self.peers.values().all(|p| p.verified && !p.epochs.is_empty())
    }

    /// Start a new epoch and wrap its secret for every verified peer.
    pub fn rotate(&mut self) -> (u16, Vec<WrappedKey>) {
        let base = self.next.map(|n| n.0).unwrap_or(self.epoch);
        let epoch = base.wrapping_add(1);
        let secret = crypto::random_bytes::<32>();
        self.next = Some((epoch, secret, Instant::now() + ACTIVATE_DELAY));
        let keys = self.wrap_for_all(epoch, &secret);
        (epoch, keys)
    }

    fn wrap_for_all(&self, epoch: u16, secret: &[u8; 32]) -> Vec<WrappedKey> {
        let mut out = Vec::new();
        for (s, p) in &self.peers {
            if !p.verified {
                continue;
            }
            if let Some((nonce, ct)) =
                self.session.wrap(&p.session_pub, &self.my_uid, &p.uid, self.channel_id, epoch, secret)
            {
                out.push(WrappedKey { to: *s, nonce: nonce.to_vec(), ct });
            }
        }
        out
    }

    pub fn receive_key(&mut self, from: SessionId, epoch: u16, nonce: &[u8], ct: &[u8]) -> bool {
        let Some(p) = self.peers.get_mut(&from) else { return false };
        if !p.verified {
            return false;
        }
        let Some(secret) = self.session.unwrap(&p.session_pub, &p.uid, &self.my_uid, self.channel_id, epoch, nonce, ct)
        else {
            return false;
        };
        // The sender goes on using its current key until the new one activates, and some of
        // what it sealed with it is still on its way: older keys stay usable for KEEP_OLD after
        // a newer one arrives. (They used to count from when they themselves arrived, so a key
        // that had been in use a while went the moment the next one came, and a moment of
        // everyone's sound was lost whenever someone joined or left.)
        let now = Instant::now();
        for (_, _, replaced) in p.epochs.iter_mut() {
            replaced.get_or_insert(now);
        }
        p.epochs.retain(|(e, _, replaced)| *e != epoch && replaced.is_none_or(|t| now.duration_since(t) < KEEP_OLD));
        p.epochs.push((epoch, secret, None));
        p.keys.retain(|(e, _), _| p.epochs.iter().any(|(pe, _, _)| pe == e));
        true
    }

    fn maybe_activate(&mut self) {
        if let Some((e, s, at)) = self.next {
            if Instant::now() >= at {
                self.epoch = e;
                self.secret = s;
                self.keys.clear();
                self.next = None;
                self.distributed = true;
            }
        }
    }

    /// True once our current key has reached the others (see `distributed`).
    pub fn can_send(&mut self) -> bool {
        self.maybe_activate();
        self.distributed
    }

    /// Current outgoing epoch (call before building a header, then `seal` with the same epoch).
    pub fn current_epoch(&mut self) -> u16 {
        self.maybe_activate();
        self.epoch
    }

    pub fn seal(&mut self, track: u8, seq: u32, aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
        let secret = self.secret;
        self.keys.entry(track).or_insert_with(|| MediaKey::derive(&secret, track)).seal(seq, aad, plaintext)
    }

    pub fn open(&mut self, from: SessionId, epoch: u16, track: u8, seq: u32, aad: &[u8], ct: &[u8]) -> Option<Vec<u8>> {
        let p = self.peers.get_mut(&from)?;
        if !p.keys.contains_key(&(epoch, track)) {
            let (_, secret, _) = p.epochs.iter().find(|(e, _, _)| *e == epoch)?;
            p.keys.insert((epoch, track), MediaKey::derive(secret, track));
        }
        p.keys.get(&(epoch, track))?.open(seq, aad, ct)
    }

    /// Why media from `from` sealed with `epoch` doesn't open, for the log.
    pub fn why_not(&self, from: SessionId, epoch: u16) -> String {
        match self.peers.get(&from) {
            None => "they aren't in the call as far as this end knows".into(),
            Some(p) if !p.verified => "their call key isn't signed by them".into(),
            Some(p) if p.epochs.is_empty() => "they haven't sent this end a key yet".into(),
            Some(p) if !p.epochs.iter().any(|(e, _, _)| *e == epoch) => {
                let have: Vec<u16> = p.epochs.iter().map(|(e, _, _)| *e).collect();
                format!("no key for their epoch {epoch} (have {have:?})")
            }
            Some(_) => "it doesn't open with their key".into(),
        }
    }

    /// Code everyone in the call can compare to rule out interception.
    pub fn call_code(&self) -> String {
        let mut entries: Vec<(String, [u8; 32])> =
            self.peers.values().filter(|p| p.verified).map(|p| (p.uid.clone(), p.session_pub)).collect();
        entries.push((self.my_uid.clone(), self.session.public()));
        crypto::call_code(&entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ingle_proto::media::{AAD_LEN, AudioHeader, track};

    fn participant(e: &E2ee, session: SessionId, id: &Identity) -> VoiceParticipant {
        let (e2ee_pub, e2ee_sig) = e.join_credentials();
        VoiceParticipant {
            session,
            user_id: id.user_id(),
            channel_id: 9,
            self_mute: false,
            self_deaf: false,
            server_mute: false,
            server_deaf: false,
            tracks: vec![],
            e2ee_pub,
            e2ee_sig,
        }
    }

    #[test]
    fn two_party_exchange_and_rotation() {
        let (ia, ib) = (Identity::generate(), Identity::generate());
        let (mut a, mut b) = (E2ee::new(&ia, 9), E2ee::new(&ib, 9));
        a.set_my_session(1);
        b.set_my_session(2);
        let (pa, pb) = (participant(&a, 1, &ia), participant(&b, 2, &ib));
        assert!(a.upsert_peer(&pb));
        assert!(b.upsert_peer(&pa));
        assert_eq!(a.call_code(), b.call_code());

        let (ea, keys_a) = a.rotate();
        let (eb, keys_b) = b.rotate();
        for k in keys_a {
            assert!(b.receive_key(1, ea, &k.nonce, &k.ct));
        }
        for k in keys_b {
            assert!(a.receive_key(2, eb, &k.nonce, &k.ct));
        }
        std::thread::sleep(ACTIVATE_DELAY + Duration::from_millis(20));

        let epoch = a.current_epoch();
        assert_eq!(epoch, ea);
        let h = AudioHeader { track: track::MIC, flags: 0, epoch, seq: 5, session: 0 }.to_bytes();
        let ct = a.seal(track::MIC, 5, &h[..AAD_LEN], b"opus");
        assert_eq!(b.open(1, epoch, track::MIC, 5, &h[..AAD_LEN], &ct).unwrap(), b"opus");

        // A third person joins; after rotation the old key still decrypts in-flight packets
        // and the newcomer can't read anything sealed before they got a key.
        let ic = Identity::generate();
        let mut c = E2ee::new(&ic, 9);
        c.set_my_session(3);
        let pc = participant(&c, 3, &ic);
        a.upsert_peer(&pc);
        c.upsert_peer(&pa);
        assert!(c.open(1, epoch, track::MIC, 5, &h[..AAD_LEN], &ct).is_none());
        let (ea2, keys) = a.rotate();
        for k in keys {
            if k.to == 3 {
                assert!(c.receive_key(1, ea2, &k.nonce, &k.ct));
            } else {
                assert!(b.receive_key(1, ea2, &k.nonce, &k.ct));
            }
        }
        assert!(b.open(1, epoch, track::MIC, 5, &h[..AAD_LEN], &ct).is_some(), "old epoch still valid briefly");
    }

    /// A key that has been in use for a long time still opens what was sealed with it for a
    /// while after the next one arrives (the sender keeps using it until the new one
    /// activates), and goes some time after that.
    #[test]
    fn long_used_keys_outlive_the_next_one_briefly() {
        let (ia, ib) = (Identity::generate(), Identity::generate());
        let (mut a, mut b) = (E2ee::new(&ia, 9), E2ee::new(&ib, 9));
        a.set_my_session(1);
        b.set_my_session(2);
        a.upsert_peer(&participant(&b, 2, &ib));
        b.upsert_peer(&participant(&a, 1, &ia));
        let (e1, keys) = a.rotate();
        assert!(b.receive_key(1, e1, &keys[0].nonce, &keys[0].ct));
        std::thread::sleep(ACTIVATE_DELAY + Duration::from_millis(20));
        let epoch = a.current_epoch();
        assert_eq!(epoch, e1);
        let h = AudioHeader { track: track::MIC, flags: 0, epoch, seq: 9, session: 0 }.to_bytes();
        let ct = a.seal(track::MIC, 9, &h[..AAD_LEN], b"opus");
        assert_eq!(b.why_not(1, e1 + 1), format!("no key for their epoch {} (have [{e1}])", e1 + 1));
        assert_eq!(b.why_not(5, e1), "they aren't in the call as far as this end knows");

        // A has used that key for a minute when someone joins and it rotates.
        let (e2, keys) = a.rotate();
        assert!(b.receive_key(1, e2, &keys[0].nonce, &keys[0].ct));
        assert!(b.open(1, epoch, track::MIC, 9, &h[..AAD_LEN], &ct).is_some(), "the key in use is gone the moment the next arrived");

        // Long after it was replaced, the next key to arrive clears it out.
        for (e, _, replaced) in b.peers.get_mut(&1).unwrap().epochs.iter_mut() {
            if *e == e1 {
                *replaced = Some(Instant::now() - KEEP_OLD - Duration::from_secs(1));
            }
        }
        let (e3, keys) = a.rotate();
        assert!(b.receive_key(1, e3, &keys[0].nonce, &keys[0].ct));
        let have: Vec<u16> = b.peers[&1].epochs.iter().map(|(e, _, _)| *e).collect();
        assert_eq!(have, vec![e2, e3]);
        assert!(b.open(1, epoch, track::MIC, 9, &h[..AAD_LEN], &ct).is_none());
    }

    #[test]
    fn forged_binding_is_rejected() {
        let (ia, ib) = (Identity::generate(), Identity::generate());
        let a = E2ee::new(&ia, 9);
        let mut b = E2ee::new(&ib, 9);
        let mut forged = participant(&a, 1, &ia);
        forged.user_id = Identity::generate().user_id(); // server claims a different identity
        assert!(!b.upsert_peer(&forged));
        assert!(b.rotate().1.is_empty(), "never send keys to unverified peers");
    }
}
