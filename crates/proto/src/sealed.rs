//! End-to-end encrypted messages (0.9): what's written in channels, direct messages and
//! groups, and the pictures sent with it, are encrypted by the sender's Pyr and opened only by
//! the Pyrs of the people who can read there. The server keeps and passes on ciphertext.
//!
//! ## Keys
//! * **Message key.** Every identity has a long-term X25519 key derived from its identity seed
//!   ([`MessageKeys::derive`]; the same on every PC that has the identity), published signed by
//!   the identity ([`MessageKey`]), so the server can't swap it for one of its own.
//! * **Conversation keys.** Each sender encrypts with a random 32-byte key of their own per
//!   conversation ([`KeyRef`]: whose, and a random id), made fresh each time their Pyr starts
//!   and whenever anyone stops being able to read there (left, removed, lost a role). It's
//!   given to everyone who can read there, themselves included (their other PCs), wrapped for
//!   each with X25519 between the giver's and the receiver's message keys ([`KeyShare`]): only
//!   those two can open it, and the receiver knows who gave it.
//! * **History.** Someone who joins later asks for the keys of what they scroll back to; the
//!   Pyr of anyone who has one and agrees they can read there wraps it for them.
//!
//! ## Messages
//! A message is a [`Body`] (its text, and for each picture its name, size and own key),
//! encrypted with ChaCha20-Poly1305 under the sender's conversation key, and signed by the
//! sender's identity together with where it was sent and a commitment to the key ([`Sealed`]).
//! So the server can't read it, change it, put it in another conversation, or pass off another
//! key as the one it was written with. Pictures are encrypted with their own random key before
//! they're uploaded.
//!
//! ## What the server still sees
//! Who wrote when and where, which message a reply answers, reactions, how big messages and
//! pictures are, and who can read where (it decides that, as before: whoever the server lets
//! read a conversation gets its keys).

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey as XPublicKey, StaticSecret};

use crate::b64;
use crate::control::{ChannelId, UserId};
use crate::crypto::{self, Identity};

/// What an older Pyr shows for an encrypted message (the server puts it in `content`).
pub const PLACEHOLDER: &str = "🔒 This message is end-to-end encrypted. Update Pyr to read it.";
/// The most ciphertext (base64) one message may carry: 4000 characters of text at up to 4
/// bytes each, ten pictures' names and keys, and room to spare.
pub const MAX_SEALED_CT: usize = 48 * 1024;
/// The most key shares sent in one `ShareKeys`.
pub const MAX_SHARES: usize = 500;
/// The most keys asked for at once (`GetKeys`).
pub const MAX_KEYS_ASKED: usize = 100;

const KEY_LABEL: &[u8] = b"pyr-message-key-v1\0";
const SHARE_LABEL: &[u8] = b"pyr-keyshare-v1\0";
const SEALED_LABEL: &[u8] = b"pyr-sealed-v1\0";
const COMMIT_LABEL: &[u8] = b"pyr-key-commit-v1\0";
const FILE_LABEL: &[u8] = b"pyr-sealed-file-v1";

/// A sender's conversation key: whose it is (only they write with it) and its id.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct KeyRef {
    pub owner: UserId,
    pub id: String,
}

impl KeyRef {
    pub fn well_formed(&self) -> bool {
        crypto::is_valid_user_id(&self.owner) && valid_key_id(&self.id)
    }
}

/// A new key id: 12 random bytes.
pub fn new_key_id() -> String {
    b64::encode(&crypto::random_bytes::<12>())
}

pub fn valid_key_id(id: &str) -> bool {
    id.len() == 16 && b64::decode_array::<12>(id).is_some()
}

/// A message as the server keeps it: ciphertext, and the author's signature.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Sealed {
    pub key: KeyRef,
    pub nonce: String,
    pub ct: String,
    pub sig: String,
}

impl Sealed {
    /// Sizes and encodings (what the server checks; it can't check more).
    pub fn well_formed(&self) -> bool {
        self.key.well_formed()
            && b64::decode_array::<12>(&self.nonce).is_some()
            && b64::decode_array::<64>(&self.sig).is_some()
            && !self.ct.is_empty()
            && self.ct.len() <= MAX_SEALED_CT
            && b64::decode(&self.ct).is_ok_and(|c| c.len() > 16)
    }
}

/// Someone's message key, signed by their identity.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MessageKey {
    pub user: UserId,
    pub key: String,
    pub sig: String,
}

fn key_statement(public: &[u8; 32]) -> Vec<u8> {
    [KEY_LABEL, public.as_slice()].concat()
}

impl MessageKey {
    /// The key, if it's well formed and `user` signed it.
    pub fn verified(&self) -> Option<[u8; 32]> {
        let key = b64::decode_array::<32>(&self.key)?;
        let sig = b64::decode_array::<64>(&self.sig)?;
        crypto::verify(&self.user, &key_statement(&key), &sig).then_some(key)
    }
}

/// A conversation key given to someone (`ShareKeys`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct NewShare {
    pub key: KeyRef,
    pub to: UserId,
    pub nonce: String,
    pub ct: String,
}

impl NewShare {
    pub fn well_formed(&self) -> bool {
        self.key.well_formed()
            && crypto::is_valid_user_id(&self.to)
            && b64::decode_array::<12>(&self.nonce).is_some()
            && b64::decode(&self.ct).is_ok_and(|c| c.len() == 32 + 16)
    }
}

/// A conversation key given to me, by `from`.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KeyShare {
    pub key: KeyRef,
    pub from: MessageKey,
    pub nonce: String,
    pub ct: String,
}

/// What's inside a message.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Body {
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<SealedFile>,
}

/// A picture sent with a message: what the server knew of it before (it now gets only
/// ciphertext), and the key it's encrypted with.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SealedFile {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    pub key: String,
}

/// Why a message couldn't be opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// Not signed by its author, for this conversation and key: someone changed it.
    Forged,
    /// Signed, but it doesn't decrypt or isn't a message.
    Damaged,
}

/// My message key pair.
pub struct MessageKeys {
    secret: StaticSecret,
    public: [u8; 32],
}

impl MessageKeys {
    /// From the identity (so every PC with it has the same one).
    pub fn derive(identity: &Identity) -> MessageKeys {
        let hk = Hkdf::<Sha256>::new(Some(KEY_LABEL), &identity.seed());
        let mut okm = [0u8; 32];
        hk.expand(b"x25519", &mut okm).expect("32 bytes is a valid HKDF length");
        let secret = StaticSecret::from(okm);
        let public = XPublicKey::from(&secret).to_bytes();
        MessageKeys { secret, public }
    }

    pub fn public(&self) -> [u8; 32] {
        self.public
    }

    /// What I publish (`PublishMessageKey`).
    pub fn publication(&self, identity: &Identity) -> MessageKey {
        MessageKey {
            user: identity.user_id(),
            key: b64::encode(&self.public),
            sig: b64::encode(&identity.sign(&key_statement(&self.public))),
        }
    }

    fn share_cipher(&self, their_pub: &[u8; 32], from: &str, to: &str, channel: ChannelId, key: &KeyRef) -> Option<(ChaCha20Poly1305, Vec<u8>)> {
        let shared = self.secret.diffie_hellman(&XPublicKey::from(*their_pub));
        if !shared.was_contributory() {
            return None;
        }
        let mut info = SHARE_LABEL.to_vec();
        for part in [from.as_bytes(), to.as_bytes(), &channel.to_be_bytes(), key.owner.as_bytes(), key.id.as_bytes()] {
            info.extend_from_slice(&(part.len() as u16).to_be_bytes());
            info.extend_from_slice(part);
        }
        let hk = Hkdf::<Sha256>::new(Some(SHARE_LABEL), shared.as_bytes());
        let mut okm = [0u8; 32];
        hk.expand(&info, &mut okm).ok()?;
        Some((ChaCha20Poly1305::new(Key::from_slice(&okm)), info))
    }

    /// Give conversation key `key` (`secret`) to `to` (whose message key I've checked).
    pub fn wrap(&self, me: &str, to: &str, their_pub: &[u8; 32], channel: ChannelId, key: &KeyRef, secret: &[u8; 32]) -> Option<NewShare> {
        let (cipher, aad) = self.share_cipher(their_pub, me, to, channel, key)?;
        let nonce = crypto::random_bytes::<12>();
        let ct = cipher.encrypt(Nonce::from_slice(&nonce), Payload { msg: secret, aad: &aad }).ok()?;
        Some(NewShare { key: key.clone(), to: to.to_string(), nonce: b64::encode(&nonce), ct: b64::encode(&ct) })
    }

    /// Open a conversation key given to me (`None`: not for me, or not from who it says).
    pub fn unwrap(&self, me: &str, channel: ChannelId, share: &KeyShare) -> Option<[u8; 32]> {
        let their_pub = share.from.verified()?;
        let nonce = b64::decode_array::<12>(&share.nonce)?;
        let ct = b64::decode(&share.ct).ok()?;
        let (cipher, aad) = self.share_cipher(&their_pub, &share.from.user, me, channel, &share.key)?;
        let pt = cipher.decrypt(Nonce::from_slice(&nonce), Payload { msg: &ct, aad: &aad }).ok()?;
        <[u8; 32]>::try_from(pt.as_slice()).ok()
    }
}

/// Commits to a conversation key: the author signs this, so nobody can hand out another key
/// that opens their message to something else.
fn commitment(key: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(COMMIT_LABEL);
    h.update(key);
    h.finalize().into()
}

/// What a message's encryption and signature are bound to.
fn header(channel: ChannelId, author: &str, key_ref: &KeyRef, key: &[u8; 32]) -> Vec<u8> {
    let mut h = SEALED_LABEL.to_vec();
    for part in [&channel.to_be_bytes()[..], author.as_bytes(), key_ref.owner.as_bytes(), key_ref.id.as_bytes(), &commitment(key)] {
        h.extend_from_slice(&(part.len() as u16).to_be_bytes());
        h.extend_from_slice(part);
    }
    h
}

fn signed_part(header: &[u8], nonce: &[u8; 12], ct: &[u8]) -> Vec<u8> {
    [header, nonce.as_slice(), ct].concat()
}

/// Encrypt and sign a message by `identity` in `channel`, with its own key `key_ref` (`key`).
pub fn seal(identity: &Identity, channel: ChannelId, key_ref: &KeyRef, key: &[u8; 32], body: &Body) -> Sealed {
    let author = identity.user_id();
    let header = header(channel, &author, key_ref, key);
    let nonce = crypto::random_bytes::<12>();
    let plain = serde_json::to_vec(body).expect("a body serializes");
    let ct = ChaCha20Poly1305::new(Key::from_slice(key))
        .encrypt(Nonce::from_slice(&nonce), Payload { msg: &plain, aad: &header })
        .expect("sealing can't fail");
    let sig = identity.sign(&signed_part(&header, &nonce, &ct));
    Sealed { key: key_ref.clone(), nonce: b64::encode(&nonce), ct: b64::encode(&ct), sig: b64::encode(&sig) }
}

/// Open a message `author` wrote in `channel`, with the key its `Sealed::key` names.
pub fn open(channel: ChannelId, author: &str, key: &[u8; 32], s: &Sealed) -> Result<Body, OpenError> {
    // Only its author writes with a key.
    if s.key.owner != author {
        return Err(OpenError::Forged);
    }
    let nonce = b64::decode_array::<12>(&s.nonce).ok_or(OpenError::Damaged)?;
    let sig = b64::decode_array::<64>(&s.sig).ok_or(OpenError::Forged)?;
    let ct = b64::decode(&s.ct).map_err(|_| OpenError::Damaged)?;
    let header = header(channel, author, &s.key, key);
    if !crypto::verify(author, &signed_part(&header, &nonce, &ct), &sig) {
        return Err(OpenError::Forged);
    }
    let plain = ChaCha20Poly1305::new(Key::from_slice(key))
        .decrypt(Nonce::from_slice(&nonce), Payload { msg: &ct, aad: &header })
        .map_err(|_| OpenError::Damaged)?;
    let mut body: Body = serde_json::from_slice(&plain).map_err(|_| OpenError::Damaged)?;
    if body.text.chars().count() > crate::MAX_MESSAGE_CHARS || body.files.len() > crate::control::MAX_ATTACHMENTS_PER_MESSAGE {
        return Err(OpenError::Damaged);
    }
    body.text = clean_text(&body.text);
    for f in &mut body.files {
        f.name = f.name.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\')).take(100).collect();
        if f.name.trim().is_empty() {
            f.name = "image".into();
        }
    }
    Ok(body)
}

/// Text as a message may hold it: trimmed, without control characters (but newlines and
/// tabs). What the server did to text before it couldn't read it.
pub fn clean_text(raw: &str) -> String {
    raw.trim().chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect()
}

/// Encrypt a picture with its own new key: (key, ciphertext).
pub fn seal_file(bytes: &[u8]) -> ([u8; 32], Vec<u8>) {
    let key = crypto::random_bytes::<32>();
    // (A key used once, so a fixed nonce.)
    let ct = ChaCha20Poly1305::new(Key::from_slice(&key))
        .encrypt(Nonce::from_slice(&[0u8; 12]), Payload { msg: bytes, aad: FILE_LABEL })
        .expect("sealing can't fail");
    (key, ct)
}

pub fn open_file(key: &[u8; 32], ct: &[u8]) -> Option<Vec<u8>> {
    ChaCha20Poly1305::new(Key::from_slice(key)).decrypt(Nonce::from_slice(&[0u8; 12]), Payload { msg: ct, aad: FILE_LABEL }).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_of(id: &Identity) -> (MessageKeys, MessageKey) {
        let k = MessageKeys::derive(id);
        let p = k.publication(id);
        (k, p)
    }

    #[test]
    fn a_message_opens_only_for_who_was_given_its_key_and_only_as_written() {
        let (alice, bob, eve) = (Identity::generate(), Identity::generate(), Identity::generate());
        let ((ak, apub), (bk, bpub), (ek, _)) = (key_of(&alice), key_of(&bob), key_of(&eve));
        assert_eq!(MessageKeys::derive(&alice).public(), ak.public(), "the same on every PC");
        let ch = 7;
        let kref = KeyRef { owner: alice.user_id(), id: new_key_id() };
        let secret = crypto::random_bytes::<32>();
        let share = ak.wrap(&alice.user_id(), &bob.user_id(), &bpub.verified().unwrap(), ch, &kref, &secret).unwrap();
        assert!(share.well_formed());
        let given = KeyShare { key: share.key.clone(), from: apub.clone(), nonce: share.nonce.clone(), ct: share.ct.clone() };
        assert_eq!(bk.unwrap(&bob.user_id(), ch, &given), Some(secret));
        // Not for Eve, not in another conversation, not from someone else.
        assert_eq!(ek.unwrap(&eve.user_id(), ch, &given), None);
        assert_eq!(bk.unwrap(&bob.user_id(), ch + 1, &given), None);
        let mut forged_from = given.clone();
        forged_from.from = key_of(&eve).1;
        assert_eq!(bk.unwrap(&bob.user_id(), ch, &forged_from), None);
        // A message key someone else signed isn't theirs.
        let mut stolen = apub.clone();
        stolen.user = bob.user_id();
        assert_eq!(stolen.verified(), None);

        let body = Body { text: "meet at the fire".into(), files: vec![] };
        let s = seal(&alice, ch, &kref, &secret, &body);
        assert!(s.well_formed());
        assert_eq!(open(ch, &alice.user_id(), &secret, &s), Ok(body.clone()));
        // Elsewhere, from someone else, or with another key: no.
        assert_eq!(open(ch + 1, &alice.user_id(), &secret, &s), Err(OpenError::Forged));
        assert_eq!(open(ch, &bob.user_id(), &secret, &s), Err(OpenError::Forged));
        assert_eq!(open(ch, &alice.user_id(), &crypto::random_bytes::<32>(), &s), Err(OpenError::Forged));
        // Changed in any way: no.
        let mut ct = b64::decode(&s.ct).unwrap();
        ct[3] ^= 1;
        let changed = Sealed { ct: b64::encode(&ct), ..s.clone() };
        assert_eq!(open(ch, &alice.user_id(), &secret, &changed), Err(OpenError::Forged));
        // Bob can't write with Alice's key, even knowing it.
        let as_bob = seal(&bob, ch, &kref, &secret, &body);
        assert_eq!(open(ch, &bob.user_id(), &secret, &as_bob), Err(OpenError::Forged));
    }

    #[test]
    fn pictures_open_with_their_own_key() {
        let (key, ct) = seal_file(b"\x89PNG not really");
        assert_eq!(open_file(&key, &ct).as_deref(), Some(&b"\x89PNG not really"[..]));
        assert_eq!(open_file(&crypto::random_bytes::<32>(), &ct), None);
    }

    #[test]
    fn text_is_cleaned_when_opened() {
        let id = Identity::generate();
        let kref = KeyRef { owner: id.user_id(), id: new_key_id() };
        let key = crypto::random_bytes::<32>();
        let s = seal(&id, 1, &kref, &key, &Body { text: "  hi\u{7}\nthere\t ".into(), files: vec![] });
        assert_eq!(open(1, &id.user_id(), &key, &s).unwrap().text, "hi\nthere");
        let long = seal(&id, 1, &kref, &key, &Body { text: "x".repeat(crate::MAX_MESSAGE_CHARS + 1), files: vec![] });
        assert_eq!(open(1, &id.user_id(), &key, &long), Err(OpenError::Damaged));
    }
}
