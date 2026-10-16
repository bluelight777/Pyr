//! Identities and end-to-end media encryption.
//!
//! ## Identity
//! Every person has one Ed25519 key pair stored on their own computer. The public key *is*
//! the user id — there is no email, phone number or password, and the same identity works
//! on every server. Logging in proves possession of the key by signing a transcript that
//! includes a server nonce and TLS keying material (so a login cannot be relayed).
//!
//! ## E2EE for voice / video / screen share
//! * On joining a voice channel each participant creates a fresh X25519 *session* key and
//!   signs it with their identity key (binding it to the channel). Everyone verifies these
//!   signatures, so the server cannot substitute keys for an existing person.
//! * Each sender picks a random 32-byte media secret per *epoch* and sends it to every other
//!   participant wrapped with `X25519(session_a, session_b)` → HKDF → ChaCha20-Poly1305.
//! * When anyone joins or leaves, every sender starts a new epoch, so people who left can't
//!   decrypt new media and newcomers can't decrypt old media.
//! * Frames are sealed with ChaCha20-Poly1305 using a per-(secret, track) key and the
//!   packet sequence number as nonce. The server only ever sees ciphertext.
//! * Everyone in a call can compare a short "call code" computed from all participants'
//!   identity and session keys; if the codes match, nobody is being intercepted.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey as XPublicKey, StaticSecret};

use crate::b64;
use crate::control::ChannelId;

/// Cryptographically secure random bytes from the OS.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::getrandom(&mut b).expect("operating system RNG unavailable");
    b
}

/// A person's long-term identity.
pub struct Identity {
    key: SigningKey,
}

impl Identity {
    pub fn generate() -> Identity {
        Identity { key: SigningKey::from_bytes(&random_bytes::<32>()) }
    }

    pub fn from_seed(seed: [u8; 32]) -> Identity {
        Identity { key: SigningKey::from_bytes(&seed) }
    }

    /// The 32-byte secret seed. Treat like a password.
    pub fn seed(&self) -> [u8; 32] {
        self.key.to_bytes()
    }

    pub fn user_id(&self) -> String {
        b64::encode(self.key.verifying_key().as_bytes())
    }

    pub fn sign(&self, msg: &[u8]) -> [u8; 64] {
        self.key.sign(msg).to_bytes()
    }
}

impl Clone for Identity {
    fn clone(&self) -> Self {
        Identity::from_seed(self.seed())
    }
}

fn verifying_key(user_id: &str) -> Option<VerifyingKey> {
    let pk = b64::decode_array::<32>(user_id)?;
    VerifyingKey::from_bytes(&pk).ok()
}

pub fn is_valid_user_id(user_id: &str) -> bool {
    user_id.len() == 43 && verifying_key(user_id).is_some()
}

/// Verify an Ed25519 signature made by `user_id`.
pub fn verify(user_id: &str, msg: &[u8], sig: &[u8]) -> bool {
    let Some(vk) = verifying_key(user_id) else { return false };
    let Ok(sig) = Signature::from_slice(sig) else { return false };
    vk.verify_strict(msg, &sig).is_ok()
}

/// What a client signs to log in.
pub fn login_transcript(server_nonce: &[u8], tls_exporter: &[u8], user_id: &str) -> Vec<u8> {
    let mut t = b"ingle-login-v1\0".to_vec();
    t.extend_from_slice(&(server_nonce.len() as u16).to_be_bytes());
    t.extend_from_slice(server_nonce);
    t.extend_from_slice(&(tls_exporter.len() as u16).to_be_bytes());
    t.extend_from_slice(tls_exporter);
    t.extend_from_slice(user_id.as_bytes());
    t
}

/// TLS exporter label used for the login transcript.
pub const LOGIN_EXPORTER_LABEL: &[u8] = b"EXPORTER-ingle-login";

/// What a signed submission (or asking for one's own) proves: this Ingle id, on this
/// connection (the server's nonce and the TLS keying material, so it can't be replayed or
/// relayed), for exactly this content (`digest`, see `control::submission_digest`).
pub fn submission_transcript(server_nonce: &[u8], tls_exporter: &[u8], user_id: &str, digest: &[u8]) -> Vec<u8> {
    let mut t = b"ingle-submission-v1\0".to_vec();
    for part in [server_nonce, tls_exporter, user_id.as_bytes(), digest] {
        t.extend_from_slice(&(part.len() as u16).to_be_bytes());
        t.extend_from_slice(part);
    }
    t
}

/// TLS exporter label for the submission transcript.
pub const SUBMISSION_EXPORTER_LABEL: &[u8] = b"EXPORTER-ingle-submission";

/// SHA-256 of a DER certificate (the server's pinned identity).
pub fn fingerprint(der: &[u8]) -> [u8; 32] {
    Sha256::digest(der).into()
}

fn digits(hash: &[u8], groups: usize) -> String {
    hash.chunks(3)
        .take(groups)
        .map(|c| {
            let v = ((c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32) % 10_000;
            format!("{v:04}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// A stable, human-comparable code for one identity (e.g. "0417 2280 9133 5052 7710 0381").
/// Six groups (about 80 bits), so nobody can make another identity with the same code.
pub fn safety_code(user_id: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"ingle-safety-v2");
    h.update(user_id.as_bytes());
    digits(&h.finalize(), 6)
}

/// A short code everyone in a call can compare. It covers every participant's identity
/// and session key, so it only matches if everybody sees the same set of keys.
pub fn call_code(participants: &[(String, [u8; 32])]) -> String {
    let mut v: Vec<&(String, [u8; 32])> = participants.iter().collect();
    v.sort();
    let mut h = Sha256::new();
    h.update(b"ingle-call-v2");
    for (uid, key) in v {
        h.update((uid.len() as u16).to_be_bytes());
        h.update(uid.as_bytes());
        h.update(key);
    }
    // Six groups (about 80 bits): too many to search for a hidden extra participant whose key
    // happens to give the same code.
    digits(&h.finalize(), 6)
}

/// What a participant signs to bind their session key to a voice channel.
pub fn binding_message(channel_id: ChannelId, session_pub: &[u8; 32]) -> Vec<u8> {
    let mut m = b"ingle-e2ee-session-v1\0".to_vec();
    m.extend_from_slice(&channel_id.to_be_bytes());
    m.extend_from_slice(session_pub);
    m
}

pub fn verify_binding(user_id: &str, channel_id: ChannelId, session_pub: &[u8; 32], sig: &[u8]) -> bool {
    verify(user_id, &binding_message(channel_id, session_pub), sig)
}

/// The per-voice-session X25519 key pair.
pub struct SessionKeys {
    secret: StaticSecret,
    public: [u8; 32],
}

impl SessionKeys {
    pub fn generate() -> SessionKeys {
        let secret = StaticSecret::from(random_bytes::<32>());
        let public = XPublicKey::from(&secret).to_bytes();
        SessionKeys { secret, public }
    }

    pub fn public(&self) -> [u8; 32] {
        self.public
    }

    /// Sign this session key with `identity`, binding it to `channel_id`.
    pub fn sign_binding(&self, identity: &Identity, channel_id: ChannelId) -> [u8; 64] {
        identity.sign(&binding_message(channel_id, &self.public))
    }

    fn wrap_key(
        &self,
        their_pub: &[u8; 32],
        sender_uid: &str,
        recipient_uid: &str,
        channel_id: ChannelId,
        epoch: u16,
    ) -> Option<(ChaCha20Poly1305, Vec<u8>)> {
        let shared = self.secret.diffie_hellman(&XPublicKey::from(*their_pub));
        if !shared.was_contributory() {
            return None;
        }
        let mut info = b"ingle-wrap-v1\0".to_vec();
        info.extend_from_slice(sender_uid.as_bytes());
        info.push(0);
        info.extend_from_slice(recipient_uid.as_bytes());
        info.push(0);
        info.extend_from_slice(&channel_id.to_be_bytes());
        info.extend_from_slice(&epoch.to_be_bytes());
        let hk = Hkdf::<Sha256>::new(Some(b"ingle-wrap-salt-v1"), shared.as_bytes());
        let mut okm = [0u8; 32];
        hk.expand(&info, &mut okm).ok()?;
        Some((ChaCha20Poly1305::new(Key::from_slice(&okm)), info))
    }

    /// Wrap our media secret for one recipient. Returns `(nonce, ciphertext)`.
    pub fn wrap(
        &self,
        their_pub: &[u8; 32],
        my_uid: &str,
        their_uid: &str,
        channel_id: ChannelId,
        epoch: u16,
        media_secret: &[u8; 32],
    ) -> Option<([u8; 12], Vec<u8>)> {
        let (cipher, aad) = self.wrap_key(their_pub, my_uid, their_uid, channel_id, epoch)?;
        let nonce = random_bytes::<12>();
        let ct = cipher
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: media_secret, aad: &aad })
            .ok()?;
        Some((nonce, ct))
    }

    /// Unwrap a media secret that `sender_uid` wrapped for us.
    pub fn unwrap(
        &self,
        their_pub: &[u8; 32],
        sender_uid: &str,
        my_uid: &str,
        channel_id: ChannelId,
        epoch: u16,
        nonce: &[u8],
        ct: &[u8],
    ) -> Option<[u8; 32]> {
        if nonce.len() != 12 {
            return None;
        }
        let (cipher, aad) = self.wrap_key(their_pub, sender_uid, my_uid, channel_id, epoch)?;
        let pt = cipher.decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad: &aad }).ok()?;
        <[u8; 32]>::try_from(pt.as_slice()).ok()
    }
}

/// Symmetric key for one (sender epoch secret, track).
#[derive(Clone)]
pub struct MediaKey {
    cipher: ChaCha20Poly1305,
}

/// AEAD tag overhead added to every sealed frame.
pub const TAG_LEN: usize = 16;

/// The key that seals the secret parts of a server's backups to the owner who makes them
/// (their identity's seed, run through HKDF): someone who gets hold of the backup folder (a
/// synced cloud folder, say) can't pass for the server without that owner's identity too.
pub fn backup_key(owner: &Identity) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(b"ingle-backup-v1"), &owner.seed());
    let mut okm = [0u8; 32];
    hk.expand(b"server identity", &mut okm).expect("32 bytes is a valid HKDF length");
    okm
}

/// What a sealed file starts with.
const SEALED_MAGIC: &[u8; 12] = b"ingle-seal1\n";

/// Seals `plaintext` with `key` (see [`backup_key`]): a fresh nonce each time.
pub fn seal(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let nonce = random_bytes::<12>();
    let ct = ChaCha20Poly1305::new(Key::from_slice(key))
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad: SEALED_MAGIC })
        .expect("sealing can't fail");
    [SEALED_MAGIC.as_slice(), &nonce, &ct].concat()
}

/// Opens what [`seal`] made with the same key (None: another key, or not sealed, or damaged).
pub fn unseal(key: &[u8; 32], sealed: &[u8]) -> Option<Vec<u8>> {
    let rest = sealed.strip_prefix(SEALED_MAGIC.as_slice())?;
    if rest.len() < 12 + 16 {
        return None;
    }
    let (nonce, ct) = rest.split_at(12);
    ChaCha20Poly1305::new(Key::from_slice(key)).decrypt(Nonce::from_slice(nonce), Payload { msg: ct, aad: SEALED_MAGIC }).ok()
}

fn media_nonce(seq: u32) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[8..12].copy_from_slice(&seq.to_be_bytes());
    n
}

impl MediaKey {
    pub fn derive(secret: &[u8; 32], track: u8) -> MediaKey {
        let hk = Hkdf::<Sha256>::new(Some(b"ingle-media-v1"), secret);
        let mut okm = [0u8; 32];
        hk.expand(&[track], &mut okm).expect("32 bytes is a valid HKDF length");
        MediaKey { cipher: ChaCha20Poly1305::new(Key::from_slice(&okm)) }
    }

    /// Encrypt `plaintext`, returning ciphertext || tag.
    pub fn seal(&self, seq: u32, aad: &[u8], plaintext: &[u8]) -> Vec<u8> {
        self.cipher
            .encrypt(Nonce::from_slice(&media_nonce(seq)), Payload { msg: plaintext, aad })
            .expect("ChaCha20-Poly1305 encryption cannot fail for valid sizes")
    }

    pub fn open(&self, seq: u32, aad: &[u8], ciphertext: &[u8]) -> Option<Vec<u8>> {
        self.cipher
            .decrypt(Nonce::from_slice(&media_nonce(seq)), Payload { msg: ciphertext, aad })
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_backups_open_only_for_their_owner() {
        let (owner, other) = (Identity::generate(), Identity::generate());
        let key = backup_key(&owner);
        assert_eq!(key, backup_key(&Identity::from_seed(owner.seed())), "the same owner, the same key");
        let sealed = seal(&key, b"server key");
        assert_ne!(seal(&key, b"server key"), sealed, "a fresh nonce each time");
        assert!(!sealed.windows(10).any(|w| w == b"server key"));
        assert_eq!(unseal(&key, &sealed).as_deref(), Some(&b"server key"[..]));
        assert!(unseal(&backup_key(&other), &sealed).is_none());
        let mut bad = sealed.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(unseal(&key, &bad).is_none());
        assert!(unseal(&key, b"server key").is_none(), "not sealed at all");
    }

    #[test]
    fn identity_sign_verify() {
        let id = Identity::generate();
        let uid = id.user_id();
        assert!(is_valid_user_id(&uid));
        let sig = id.sign(b"hello");
        assert!(verify(&uid, b"hello", &sig));
        assert!(!verify(&uid, b"hellO", &sig));
        let other = Identity::generate();
        assert!(!verify(&other.user_id(), b"hello", &sig));
        assert_eq!(Identity::from_seed(id.seed()).user_id(), uid);
    }

    #[test]
    fn wrap_unwrap_between_two_sessions() {
        let alice = Identity::generate();
        let bob = Identity::generate();
        let (a, b) = (SessionKeys::generate(), SessionKeys::generate());
        let secret = random_bytes::<32>();
        let (au, bu) = (alice.user_id(), bob.user_id());
        let (nonce, ct) = a.wrap(&b.public(), &au, &bu, 42, 7, &secret).unwrap();
        let got = b.unwrap(&a.public(), &au, &bu, 42, 7, &nonce, &ct).unwrap();
        assert_eq!(got, secret);
        // Wrong epoch, channel or sender identity must fail.
        assert!(b.unwrap(&a.public(), &au, &bu, 42, 8, &nonce, &ct).is_none());
        assert!(b.unwrap(&a.public(), &au, &bu, 43, 7, &nonce, &ct).is_none());
        assert!(b.unwrap(&a.public(), &bu, &au, 42, 7, &nonce, &ct).is_none());
        // A third party (e.g. the server) can't unwrap it.
        let eve = SessionKeys::generate();
        assert!(eve.unwrap(&a.public(), &au, &bu, 42, 7, &nonce, &ct).is_none());
    }

    #[test]
    fn binding_signatures() {
        let id = Identity::generate();
        let s = SessionKeys::generate();
        let sig = s.sign_binding(&id, 5);
        assert!(verify_binding(&id.user_id(), 5, &s.public(), &sig));
        assert!(!verify_binding(&id.user_id(), 6, &s.public(), &sig));
    }

    #[test]
    fn media_seal_open() {
        let secret = random_bytes::<32>();
        let k = MediaKey::derive(&secret, 1);
        let aad = [1u8, 2, 3];
        let ct = k.seal(10, &aad, b"frame data");
        assert_eq!(ct.len(), 10 + TAG_LEN);
        assert_eq!(k.open(10, &aad, &ct).unwrap(), b"frame data");
        assert!(k.open(11, &aad, &ct).is_none(), "wrong seq/nonce must fail");
        assert!(k.open(10, &[1, 2, 4], &ct).is_none(), "tampered header must fail");
        let k_other_track = MediaKey::derive(&secret, 2);
        assert!(k_other_track.open(10, &aad, &ct).is_none());
    }

    #[test]
    fn codes_are_order_independent() {
        let a = ("alice".to_string(), [1u8; 32]);
        let b = ("bob".to_string(), [2u8; 32]);
        assert_eq!(call_code(&[a.clone(), b.clone()]), call_code(&[b.clone(), a.clone()]));
        assert_ne!(call_code(&[a.clone(), b]), call_code(&[a, ("bob".into(), [3u8; 32])]));
        assert_eq!(safety_code("x").len(), 29);
        assert_eq!(call_code(&[("a".into(), [1u8; 32])]).len(), 29);
    }
}
