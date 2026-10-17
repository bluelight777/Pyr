//! Making sure people are who they say, with no codes to read out: two ways for two Pyrs to
//! check each other, through the server, which only passes the messages on (`Request::
//! Handshake`, `Event::Handshake`; [`crate::control::HANDSHAKE_FEATURE`]).
//!
//! - **Compare words**: both Pyrs show the same six words (each with its picture) when
//!   nobody's in between, and different ones when someone is. You say them to each other (in a
//!   call, on the phone, in person) and say whether they match. A short authentication string
//!   after a commitment, as in Matrix's emoji verification: whoever starts commits to their key
//!   before seeing the other's, so someone in the middle can't search for keys that give both
//!   sides the same words; one try at fooling them works 1 time in 2^48.
//! - **Ask something only they'd know**: one asks a question and types the answer they expect,
//!   the other types theirs. A PAKE (CPace over Ristretto255) tells both whether the answers
//!   matched without either answer leaving its PC; someone in the middle gets one guess at the
//!   answer per question, and nothing to test guesses against afterwards.
//!
//! Both take in who each Pyr thinks is at each end (their ids, as its server shows them), so
//! a server that showed someone else under a name can't make them come out right.

use curve25519_dalek::ristretto::{CompressedRistretto, RistrettoPoint};
use curve25519_dalek::scalar::Scalar;
use curve25519_dalek::traits::Identity as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use x25519_dalek::{PublicKey, StaticSecret};

use crate::b64;
use crate::crypto::random_bytes;
use crate::words::words;

/// What goes between two Pyrs (as JSON in `Request::Handshake`'s `data`).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Handshake {
    /// "Compare words?": whoever starts, and their commitment to their key.
    Words { sid: String, commit: String },
    /// "Yes": the other's key.
    WordsKey { sid: String, key: String },
    /// The starter's key (the one committed to): both show the words now.
    WordsReveal { sid: String, key: String },
    /// What its person said about the words.
    WordsSaid { sid: String, matched: bool },
    /// A question, and the asker's half.
    Ask { sid: String, question: String, ya: String },
    /// The answer's half, and proof of what it gave.
    Answer { sid: String, yb: String, mac: String },
    /// The answers matched (the asker's proof).
    Confirm { sid: String, mac: String },
    /// The answers didn't match.
    Mismatch { sid: String },
    /// Not now (or no longer).
    Decline { sid: String },
}

impl Handshake {
    pub fn sid(&self) -> &str {
        match self {
            Handshake::Words { sid, .. }
            | Handshake::WordsKey { sid, .. }
            | Handshake::WordsReveal { sid, .. }
            | Handshake::WordsSaid { sid, .. }
            | Handshake::Ask { sid, .. }
            | Handshake::Answer { sid, .. }
            | Handshake::Confirm { sid, .. }
            | Handshake::Mismatch { sid }
            | Handshake::Decline { sid } => sid,
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// One from someone else's Pyr (anything else, or anything too big, is nothing).
    pub fn from_json(s: &str) -> Option<Handshake> {
        (s.len() <= HANDSHAKE_MAX).then(|| serde_json::from_str(s).ok()).flatten()
    }
}

/// The most a handshake message can be (bytes of JSON).
pub const HANDSHAKE_MAX: usize = 2048;
/// How many words are compared (48 bits).
pub const WORD_COUNT: usize = 6;
/// The longest question and answer (characters).
pub const QUESTION_MAX: usize = 200;
pub const ANSWER_MAX: usize = 200;

/// A word to compare: its picture and the word.
pub type Word = (&'static str, &'static str);

fn part(h: &mut impl Digest, bytes: &[u8]) {
    h.update((bytes.len() as u32).to_be_bytes());
    h.update(bytes);
}

/// Equal, taking as long whatever they hold.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

fn new_sid() -> ([u8; 16], String) {
    let b = random_bytes::<16>();
    (b, b64::encode(&b))
}

// ------------------------------------------------------------------ compare words

fn commitment(sid: &[u8], key: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"ingle-words-commit-v1\0");
    part(&mut h, sid);
    part(&mut h, key);
    h.finalize().into()
}

/// The words: everything both ends should agree on, hashed. `starter` and `joiner` are ids.
fn words_of(sid: &[u8], starter: &str, joiner: &str, starter_key: &[u8; 32], joiner_key: &[u8; 32], shared: &[u8; 32]) -> Vec<Word> {
    let mut h = Sha512::new();
    h.update(b"ingle-words-v1\0");
    for p in [sid, starter.as_bytes(), joiner.as_bytes(), starter_key, joiner_key, shared] {
        part(&mut h, p);
    }
    words(&h.finalize(), WORD_COUNT)
}

fn shared_secret(secret: &StaticSecret, theirs: &[u8; 32]) -> Option<[u8; 32]> {
    let s = secret.diffie_hellman(&PublicKey::from(*theirs));
    // (A key of the wrong kind would make the secret the same whatever mine is.)
    s.was_contributory().then(|| s.to_bytes())
}

/// Whoever starts comparing words.
pub struct WordsStarter {
    pub sid: String,
    sid_bytes: [u8; 16],
    me: String,
    them: String,
    secret: StaticSecret,
    key: [u8; 32],
}

impl WordsStarter {
    /// `me`: my id; `them`: theirs, as I know them.
    pub fn start(me: &str, them: &str) -> (WordsStarter, Handshake) {
        let (sid_bytes, sid) = new_sid();
        let secret = StaticSecret::from(random_bytes::<32>());
        let key = PublicKey::from(&secret).to_bytes();
        let msg = Handshake::Words { sid: sid.clone(), commit: b64::encode(&commitment(&sid_bytes, &key)) };
        (WordsStarter { sid, sid_bytes, me: me.into(), them: them.into(), secret, key }, msg)
    }

    /// Their key came: mine to send them, and the words.
    pub fn finish(&self, their_key: &str) -> Option<(Handshake, Vec<Word>)> {
        let theirs = b64::decode_array::<32>(their_key)?;
        let shared = shared_secret(&self.secret, &theirs)?;
        let words = words_of(&self.sid_bytes, &self.me, &self.them, &self.key, &theirs, &shared);
        Some((Handshake::WordsReveal { sid: self.sid.clone(), key: b64::encode(&self.key) }, words))
    }
}

/// Whoever was asked to compare words.
pub struct WordsJoiner {
    sid_bytes: [u8; 16],
    commit: [u8; 32],
    me: String,
    them: String,
    secret: StaticSecret,
    key: [u8; 32],
}

impl WordsJoiner {
    /// Saying yes to `Words` from `them` (their id as I know them): my key to send.
    pub fn join(me: &str, them: &str, sid: &str, commit: &str) -> Option<(WordsJoiner, Handshake)> {
        let sid_bytes = b64::decode_array::<16>(sid)?;
        let commit = b64::decode_array::<32>(commit)?;
        let secret = StaticSecret::from(random_bytes::<32>());
        let key = PublicKey::from(&secret).to_bytes();
        let msg = Handshake::WordsKey { sid: sid.to_string(), key: b64::encode(&key) };
        Some((WordsJoiner { sid_bytes, commit, me: me.into(), them: them.into(), secret, key }, msg))
    }

    /// Their key came, as they promised it (else nothing): the words.
    pub fn finish(&self, their_key: &str) -> Option<Vec<Word>> {
        let theirs = b64::decode_array::<32>(their_key)?;
        if !same(&commitment(&self.sid_bytes, &theirs), &self.commit) {
            return None;
        }
        let shared = shared_secret(&self.secret, &theirs)?;
        Some(words_of(&self.sid_bytes, &self.them, &self.me, &theirs, &self.key, &shared))
    }
}

// ------------------------------------------------------------------ ask something

/// The answer as it's compared: its letters and digits, lowercase ("Rex the Dog!" and "rex the
/// dog" are the same answer).
pub fn normalize(answer: &str) -> String {
    answer.chars().flat_map(char::to_lowercase).filter(|c| c.is_alphanumeric()).collect()
}

/// The secret point both ends get to only with the same answer (and the same view of who's
/// asking whom, what).
fn generator(sid: &[u8], asker: &str, answerer: &str, question: &str, answer: &str) -> RistrettoPoint {
    let mut h = Sha512::new();
    h.update(b"ingle-ask-v1\0");
    for p in [sid, asker.as_bytes(), answerer.as_bytes(), question.as_bytes(), normalize(answer).as_bytes()] {
        part(&mut h, p);
    }
    RistrettoPoint::from_uniform_bytes(&h.finalize().into())
}

fn random_scalar() -> Scalar {
    Scalar::from_bytes_mod_order_wide(&random_bytes::<64>())
}

/// A half sent over: a valid point that isn't the identity (which would give the secret away).
fn point(s: &str) -> Option<([u8; 32], RistrettoPoint)> {
    let bytes = b64::decode_array::<32>(s)?;
    let p = CompressedRistretto(bytes).decompress()?;
    (p != RistrettoPoint::identity()).then_some((bytes, p))
}

fn session_key(sid: &[u8], asker: &str, answerer: &str, ya: &[u8; 32], yb: &[u8; 32], k: &RistrettoPoint) -> [u8; 64] {
    let mut h = Sha512::new();
    h.update(b"ingle-ask-isk-v1\0");
    for p in [sid, asker.as_bytes(), answerer.as_bytes(), ya, yb, k.compress().as_bytes()] {
        part(&mut h, p);
    }
    h.finalize().into()
}

fn mac(isk: &[u8; 64], who: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"ingle-ask-mac-v1\0");
    part(&mut h, who);
    h.update(isk);
    h.finalize().into()
}

/// The asker, from asking until the answer comes.
pub struct Asking {
    pub sid: String,
    sid_bytes: [u8; 16],
    asker: String,
    answerer: String,
    secret: Scalar,
    ya: [u8; 32],
}

impl Asking {
    /// `asker`: my id; `answerer`: theirs, as I know them.
    pub fn start(asker: &str, answerer: &str, question: &str, answer: &str) -> (Asking, Handshake) {
        let (sid_bytes, sid) = new_sid();
        let secret = random_scalar();
        let ya = (secret * generator(&sid_bytes, asker, answerer, question, answer)).compress().to_bytes();
        let msg = Handshake::Ask { sid: sid.clone(), question: question.to_string(), ya: b64::encode(&ya) };
        (Asking { sid, sid_bytes, asker: asker.into(), answerer: answerer.into(), secret, ya }, msg)
    }

    /// Their answer came: `Some(Confirm)` (to send them) if the answers matched, `None` if not.
    pub fn finish(&self, yb: &str, their_mac: &str) -> Option<Handshake> {
        let (yb, ybp) = point(yb)?;
        let isk = session_key(&self.sid_bytes, &self.asker, &self.answerer, &self.ya, &yb, &(self.secret * ybp));
        let got = b64::decode(their_mac).ok()?;
        same(&got, &mac(&isk, b"answer")).then(|| Handshake::Confirm { sid: self.sid.clone(), mac: b64::encode(&mac(&isk, b"ask")) })
    }
}

/// The answerer's side, for a question that came (`asker`: their id as I know them;
/// `answerer`: mine): the reply to send, and the confirmation that will mean the answers
/// matched (see `confirmed`). `None` if what came can't be answered.
pub fn answer(asker: &str, answerer: &str, sid: &str, question: &str, ya: &str, answer: &str) -> Option<(Handshake, [u8; 32])> {
    let sid_bytes = b64::decode_array::<16>(sid)?;
    let (ya, yap) = point(ya)?;
    let secret = random_scalar();
    let yb = (secret * generator(&sid_bytes, asker, answerer, question, answer)).compress().to_bytes();
    let isk = session_key(&sid_bytes, asker, answerer, &ya, &yb, &(secret * yap));
    let reply = Handshake::Answer { sid: sid.to_string(), yb: b64::encode(&yb), mac: b64::encode(&mac(&isk, b"answer")) };
    Some((reply, mac(&isk, b"ask")))
}

/// Whether the asker's confirmation is the one that means the answers matched.
pub fn confirmed(expected: &[u8; 32], their_mac: &str) -> bool {
    b64::decode(their_mac).is_ok_and(|m| same(&m, expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compare words with each end's view of who's who: (starter's (me, them), joiner's (me, them)).
    fn compare(starter_view: (&str, &str), joiner_view: (&str, &str)) -> (Vec<Word>, Vec<Word>) {
        let (s, start) = WordsStarter::start(starter_view.0, starter_view.1);
        let Handshake::Words { sid, commit } = start else { panic!() };
        let (j, key) = WordsJoiner::join(joiner_view.0, joiner_view.1, &sid, &commit).unwrap();
        let Handshake::WordsKey { key, .. } = key else { panic!() };
        let (reveal, a) = s.finish(&key).unwrap();
        let Handshake::WordsReveal { key, .. } = reveal else { panic!() };
        (a, j.finish(&key).unwrap())
    }

    #[test]
    fn the_same_words_on_both_ends_and_different_ones_with_someone_in_between() {
        let (a, b) = compare(("blue", "mara"), ("mara", "blue"));
        assert_eq!(a.len(), WORD_COUNT);
        assert_eq!(a, b);
        // A server that showed each of them someone else under the other's name.
        let (a, b) = compare(("blue", "not-mara"), ("mara", "blue"));
        assert_ne!(a, b);
        let (a, b) = compare(("blue", "mara"), ("mara", "not-blue"));
        assert_ne!(a, b);
        // Different every time.
        assert_ne!(compare(("blue", "mara"), ("mara", "blue")).0, compare(("blue", "mara"), ("mara", "blue")).0);
    }

    #[test]
    fn a_key_that_isnt_the_one_promised_gets_no_words() {
        let (_s, start) = WordsStarter::start("blue", "mara");
        let Handshake::Words { sid, commit } = start else { panic!() };
        let (j, _) = WordsJoiner::join("mara", "blue", &sid, &commit).unwrap();
        // Someone in the middle swapping in a key of their own, chosen after seeing mine.
        let other = PublicKey::from(&StaticSecret::from(random_bytes::<32>())).to_bytes();
        assert!(j.finish(&b64::encode(&other)).is_none());
        // A key of the wrong kind (all zeros) gives no secret.
        let (s2, _) = WordsStarter::start("blue", "mara");
        assert!(s2.finish(&b64::encode(&[0u8; 32])).is_none());
        assert!(Handshake::from_json(&"x".repeat(HANDSHAKE_MAX + 1)).is_none());
    }

    fn ask(asker_view: (&str, &str), answerer_view: (&str, &str), expected: &str, typed: &str) -> (bool, bool) {
        let (asking, msg) = Asking::start(asker_view.0, asker_view.1, "Our first raid boss?", expected);
        let back = Handshake::from_json(&msg.to_json()).unwrap();
        let Handshake::Ask { sid, question, ya } = back else { panic!() };
        let (reply, want) = answer(answerer_view.0, answerer_view.1, &sid, &question, &ya, typed).unwrap();
        let Handshake::Answer { yb, mac, .. } = reply else { panic!() };
        match asking.finish(&yb, &mac) {
            Some(Handshake::Confirm { mac, .. }) => (true, confirmed(&want, &mac)),
            _ => (false, false),
        }
    }

    #[test]
    fn the_same_answer_matches_on_both_ends() {
        assert_eq!(ask(("blue", "mara"), ("blue", "mara"), "The Lich King", "the lich-king!"), (true, true));
    }

    #[test]
    fn a_different_answer_or_a_different_person_doesnt() {
        assert_eq!(ask(("blue", "mara"), ("blue", "mara"), "The Lich King", "the dragon"), (false, false));
        assert_eq!(ask(("blue", "impostor"), ("blue", "mara"), "lich king", "lich king"), (false, false));
        assert_eq!(ask(("blue", "mara"), ("impostor", "mara"), "lich king", "lich king"), (false, false));
    }

    #[test]
    fn nothing_odd_is_taken() {
        assert!(answer("a", "b", "not a sid", "q", &b64::encode(&[1u8; 32]), "x").is_none());
        assert!(answer("a", "b", &b64::encode(&[7u8; 16]), "q", &b64::encode(&[0u8; 32]), "x").is_none());
        let (asking, _) = Asking::start("a", "b", "q", "x");
        assert!(asking.finish(&b64::encode(&[0u8; 32]), "AAAA").is_none());
        assert_eq!(normalize("  Rex, the DOG! "), "rexthedog");
        let m = Handshake::Mismatch { sid: "s".into() };
        assert_eq!(Handshake::from_json(&m.to_json()), Some(m));
        assert!(Handshake::from_json(r#"{"t":"something_new","sid":"s"}"#).is_none());
    }
}
