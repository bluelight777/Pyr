//! The props feed (Pyr 0.15): prop bets that open, close and settle by themselves.
//!
//! A service on pyr.gg (`crates/props-feed`, `pyr-props`) reads public sources every few
//! minutes (scoreboards, exchanges' prices, the weather service, earthquakes and the like),
//! makes props from them the way the big prediction sites do (a series that repeats, an event
//! a game or a day, a few props an event, each naming its source and its rules up front), and
//! says how each came out. What it knows is one signed file, [`FEED_URL`].
//!
//! A Pyr server whose owner turned automatic props on follows that file: it opens a prop of
//! its own for each one there (backed by the casino, with its own odds and its own chips),
//! closes it at the feed's time and settles it as the feed says, with nobody deciding. See
//! `pyr_server::poker::props`.
//!
//! The feed is signed with a key that lives on pyr.gg and signs nothing else. It counts only
//! with a certificate signed by the release key ([`FeedCert`]), and only until the
//! certificate's date (as the Library Keeper's key does, `library::KeeperCert`). So whoever
//! got that key could settle play-money props wrongly until the certificate ran out, and
//! nothing more: never a release, never the Library.

use serde::{Deserialize, Serialize};

use crate::b64;
use crate::crypto::{self, Identity};

/// Where the feed is.
pub const FEED_URL: &str = "https://pyr.gg/props/feed.json";
/// The feed this Pyr reads (`Feed::v`): a newer one is read as far as it's understood.
pub const VERSION: u32 = 1;
/// A signed feed starts with this (nothing else signed can pass for one).
const CONTEXT: &[u8] = b"pyr props feed v1\n";
/// A feed key's certificate starts with this.
const CERT_CONTEXT: &[u8] = b"pyr props feed key v1\n";
/// The most props a feed carries, and the longest a text in it.
pub const MAX_PROPS: usize = 4000;
const MAX_TEXT: usize = 400;

/// Everything the feed knows now: the props taking bets, the ones waiting for their result,
/// and the ones that came out lately (kept a week, so a server that was off catches up).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Feed {
    pub v: u32,
    /// When it was made (Unix ms). A server never goes back to an older one.
    pub made: i64,
    pub props: Vec<FeedProp>,
}

/// One prop of the feed.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct FeedProp {
    /// Its name for good, the same on every server: `nfl:401772345:win`.
    pub id: String,
    /// Its category: `nfl`, `crypto`, `weather` (see `pyr_poker::props::CATEGORIES`; one a
    /// Pyr doesn't know yet shows under its own name).
    pub cat: String,
    /// What it's about, shared by the props of one game or one day: `nfl:401772345`, and its
    /// name, "Chiefs at Bills".
    pub event: String,
    pub event_name: String,
    /// The question, and its two to six outcomes.
    pub q: String,
    pub outcomes: Vec<String>,
    /// Each outcome's chance as the prop opens (they add up to 1): where the odds start.
    pub prior: Vec<f64>,
    /// When betting closes, and when what it's about starts (Unix ms; 0: not said).
    pub closes: i64,
    pub starts: i64,
    /// A line about it as it stands: "Sunday 4:25 PM ET · Highmark Stadium", "Q3 5:21 · 17-20".
    pub detail: String,
    /// Whose numbers settle it ("National Weather Service"), and how, in a sentence or two.
    pub source: String,
    pub rules: String,
    /// How much it's worth showing first (0: not especially).
    pub featured: u8,
    /// How it came out, once it has.
    pub result: Option<FeedResult>,
}

/// How a prop came out.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct FeedResult {
    /// The outcome that won. None: called off (everyone's chips back).
    pub winner: Option<usize>,
    /// What happened, in a few words: "Bills 27, Chiefs 24".
    pub note: String,
    pub at: i64,
}

impl FeedProp {
    /// Whether it's well formed (a server opens nothing that isn't).
    pub fn check(&self) -> Result<(), String> {
        let name_ok = |s: &str, max: usize| !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b':' | b'.' | b'_' | b'-'));
        if !name_ok(&self.id, 96) {
            return Err(format!("bad id {:?}", self.id));
        }
        if !name_ok(&self.cat, 24) || self.cat.contains(':') {
            return Err(format!("{}: bad category {:?}", self.id, self.cat));
        }
        if !self.event.is_empty() && !name_ok(&self.event, 96) {
            return Err(format!("{}: bad event", self.id));
        }
        let n = self.outcomes.len();
        if !(2..=6).contains(&n) {
            return Err(format!("{}: {n} outcomes", self.id));
        }
        if self.prior.len() != n || self.prior.iter().any(|p| !p.is_finite() || *p <= 0.0 || *p >= 1.0) {
            return Err(format!("{}: bad starting odds", self.id));
        }
        let sum: f64 = self.prior.iter().sum();
        if (sum - 1.0).abs() > 0.02 {
            return Err(format!("{}: the starting odds add up to {sum:.3}", self.id));
        }
        for t in [&self.q, &self.event_name, &self.detail, &self.source, &self.rules].into_iter().chain(&self.outcomes) {
            if t.len() > MAX_TEXT || t.chars().any(|c| c.is_control()) {
                return Err(format!("{}: a text too long or with control characters", self.id));
            }
        }
        if self.closes <= 0 {
            return Err(format!("{}: no closing time", self.id));
        }
        if let Some(r) = &self.result {
            if r.winner.is_some_and(|w| w >= n) {
                return Err(format!("{}: a winner that isn't an outcome", self.id));
            }
            if r.note.len() > MAX_TEXT || r.note.chars().any(|c| c.is_control()) {
                return Err(format!("{}: bad result note", self.id));
            }
        }
        Ok(())
    }
}

/// The file on the website: the feed exactly as signed, its signature, and the release key's
/// certificate for the key that signed it.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SignedFeed {
    pub feed: String,
    pub sig: String,
    pub cert: FeedCert,
}

/// The release key's word that `key` (a public key, as a user id) may sign the props feed
/// until `not_after` (Unix seconds).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct FeedCert {
    pub key: String,
    pub not_after: i64,
    pub sig: String,
}

fn cert_message(key: &str, not_after: i64) -> Vec<u8> {
    [CERT_CONTEXT, format!("{key}\n{not_after}").as_bytes()].concat()
}

fn message(feed: &str) -> Vec<u8> {
    [CONTEXT, feed.as_bytes()].concat()
}

/// Certifies the feed's key (signed with the release key).
pub fn certify(release: &Identity, key: &str, not_after: i64) -> FeedCert {
    FeedCert { key: key.to_string(), not_after, sig: b64::encode(&release.sign(&cert_message(key, not_after))) }
}

/// Whether a certificate is the release key's (`release_key`: its public half).
pub fn cert_ok(cert: &FeedCert, release_key: &str) -> bool {
    b64::decode(&cert.sig).is_ok_and(|sig| crypto::verify(release_key, &cert_message(&cert.key, cert.not_after), &sig))
}

/// Signs a feed with its key (`cert`: the release key's certificate for that key).
pub fn sign(key: &Identity, cert: &FeedCert, feed: &Feed) -> SignedFeed {
    let text = serde_json::to_string(feed).expect("a feed serializes");
    let sig = key.sign(&message(&text));
    SignedFeed { feed: text, sig: b64::encode(&sig), cert: cert.clone() }
}

/// Checks a signed feed against the release key's public half (its signer's certificate, the
/// certificate's date, the signature) and reads it. Props that aren't well formed are left
/// out, not trusted.
pub fn open(signed_json: &str, release_key: &str) -> Result<Feed, String> {
    let signed: SignedFeed = serde_json::from_str(signed_json).map_err(|e| format!("not a props feed: {e}"))?;
    if !cert_ok(&signed.cert, release_key) {
        return Err("the props feed's signer isn't certified by Firelit Games".into());
    }
    let sig = b64::decode(&signed.sig).map_err(|_| "bad signature encoding".to_string())?;
    if !crypto::verify(&signed.cert.key, &message(&signed.feed), &sig) {
        return Err("the props feed isn't signed by its key".into());
    }
    let mut feed: Feed = serde_json::from_str(&signed.feed).map_err(|e| format!("bad props feed: {e}"))?;
    if feed.made / 1000 > signed.cert.not_after {
        return Err("the props feed was signed after its signer's certificate ran out".into());
    }
    feed.props.truncate(MAX_PROPS);
    feed.props.retain(|p| p.check().is_ok());
    let mut seen = std::collections::HashSet::new();
    feed.props.retain(|p| seen.insert(p.id.clone()));
    Ok(feed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prop(id: &str) -> FeedProp {
        FeedProp {
            id: id.into(),
            cat: "nfl".into(),
            event: "nfl:1".into(),
            event_name: "Chiefs at Bills".into(),
            q: "Who wins: Chiefs at Bills?".into(),
            outcomes: vec!["Chiefs".into(), "Bills".into()],
            prior: vec![0.42, 0.58],
            closes: 1_800_000_000_000,
            starts: 1_800_000_000_000,
            source: "ESPN".into(),
            rules: "Settles on the final score.".into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_feed_counts_only_signed_by_a_key_the_release_key_certified() {
        let release = Identity::generate();
        let key = Identity::generate();
        let cert = certify(&release, &key.user_id(), 2_000_000_000);
        let feed = Feed { v: VERSION, made: 1_700_000_000_000, props: vec![prop("nfl:1:win"), prop("nfl:1:total")] };
        let text = serde_json::to_string(&sign(&key, &cert, &feed)).unwrap();
        assert_eq!(open(&text, &release.user_id()).unwrap(), feed);
        // Another release key: not its certificate.
        assert!(open(&text, &Identity::generate().user_id()).unwrap_err().contains("certified"));
        // A certificate the feed's key made for itself counts for nothing.
        let own = certify(&key, &key.user_id(), 2_000_000_000);
        let t2 = serde_json::to_string(&sign(&key, &own, &feed)).unwrap();
        assert!(open(&t2, &release.user_id()).unwrap_err().contains("certified"));
        // A certificate stretched, or a feed changed after it was signed.
        let mut signed = sign(&key, &cert, &feed);
        signed.cert.not_after += 1;
        assert!(open(&serde_json::to_string(&signed).unwrap(), &release.user_id()).is_err());
        let mut signed = sign(&key, &cert, &feed);
        signed.feed = signed.feed.replace("Bills", "Bulls");
        assert!(open(&serde_json::to_string(&signed).unwrap(), &release.user_id()).unwrap_err().contains("isn't signed"));
        // Signed after the certificate ran out.
        let late = Feed { made: 2_000_000_001_000, ..feed.clone() };
        assert!(open(&serde_json::to_string(&sign(&key, &cert, &late)).unwrap(), &release.user_id()).unwrap_err().contains("ran out"));
        // The Library Keeper's certificate (another context) isn't a feed certificate.
        let keeper = crate::library::certify(&release, &key.user_id(), 2_000_000_000);
        let mut signed = sign(&key, &cert, &feed);
        signed.cert.sig = keeper.sig;
        assert!(open(&serde_json::to_string(&signed).unwrap(), &release.user_id()).is_err());
    }

    #[test]
    fn props_that_aren_t_well_formed_are_left_out() {
        let release = Identity::generate();
        let key = Identity::generate();
        let cert = certify(&release, &key.user_id(), 2_000_000_000);
        let mut bad_odds = prop("nfl:2:win");
        bad_odds.prior = vec![0.9, 0.9];
        let mut no_close = prop("nfl:3:win");
        no_close.closes = 0;
        let mut bad_winner = prop("nfl:4:win");
        bad_winner.result = Some(FeedResult { winner: Some(2), ..Default::default() });
        let mut bad_id = prop("NFL 5");
        bad_id.cat = "nfl".into();
        let mut called_off = prop("nfl:6:win");
        called_off.result = Some(FeedResult { winner: None, note: "Postponed".into(), at: 5 });
        let feed = Feed { v: VERSION, made: 1, props: vec![prop("nfl:1:win"), bad_odds, no_close, bad_winner, bad_id, called_off, prop("nfl:1:win")] };
        let got = open(&serde_json::to_string(&sign(&key, &cert, &feed)).unwrap(), &release.user_id()).unwrap();
        assert_eq!(got.props.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["nfl:1:win", "nfl:6:win"]);
    }

    #[test]
    fn a_newer_feed_reads_as_far_as_it_s_understood() {
        let release = Identity::generate();
        let key = Identity::generate();
        let cert = certify(&release, &key.user_id(), 2_000_000_000);
        let text = r#"{"v":2,"made":7,"new_thing":[1,2],"props":[{"id":"x:1","cat":"crypto","q":"Bitcoin above $100,000 on Friday?","outcomes":["Yes","No"],"prior":[0.3,0.7],"closes":9,"later":true}]}"#;
        let signed = SignedFeed { feed: text.into(), sig: b64::encode(&key.sign(&message(text))), cert };
        let got = open(&serde_json::to_string(&signed).unwrap(), &release.user_id()).unwrap();
        assert_eq!((got.v, got.props.len(), got.props[0].cat.as_str()), (2, 1, "crypto"));
    }
}
