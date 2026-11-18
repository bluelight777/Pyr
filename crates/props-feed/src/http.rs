//! Reading the sources: plain HTTPS GETs that say who's asking, never hurried, never big.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Something that fetches a URL's text (the web; canned answers in tests).
pub trait Fetch: Sync {
    fn get(&self, url: &str) -> Result<String, String>;

    /// The same, with a header of the asker's own: a site's key, which goes nowhere else
    /// (never in the address, never in what's said when the request fails).
    fn get_with(&self, url: &str, _header: (&str, &str)) -> Result<String, String> {
        self.get(url)
    }
}

/// The most a source's answer may be (a scoreboard is a few hundred kilobytes).
const MAX_BYTES: u64 = 6 << 20;
/// The least time between two requests to one host: nothing here needs to hurry, and the
/// sources are other people's servers.
const GAP: Duration = Duration::from_millis(1200);

/// A host that allows fewer still: balldontlie's free plan is five requests a minute.
fn gap_for(host: &str) -> Duration {
    match host {
        "api.balldontlie.io" => Duration::from_millis(13_000),
        _ => GAP,
    }
}

/// The web, politely: one request at a time to a host, a moment apart, with a User-Agent that
/// says what this is and where to find its makers (the National Weather Service asks for one).
pub struct Web {
    agent: ureq::Agent,
    last: Mutex<HashMap<String, Instant>>,
}

impl Web {
    pub fn new(contact: &str) -> Web {
        let ua = format!("PyrProps/{} (+https://pyr.gg{}{})", env!("CARGO_PKG_VERSION"), if contact.is_empty() { "" } else { "; " }, contact);
        let cfg = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(25))).http_status_as_error(false).user_agent(ua);
        Web { agent: cfg.build().into(), last: Mutex::new(HashMap::new()) }
    }
}

fn host_of(url: &str) -> &str {
    url.split_once("://").map_or(url, |(_, r)| r).split(['/', '?']).next().unwrap_or("")
}

impl Web {
    fn fetch(&self, url: &str, header: Option<(&str, &str)>) -> Result<String, String> {
        let host = host_of(url).to_string();
        // (A moment since the last request to this host.)
        let wait = {
            let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            let wait = last.get(&host).map(|t| (*t + gap_for(&host)).saturating_duration_since(now)).unwrap_or_default();
            last.insert(host.clone(), now + wait);
            wait
        };
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        let mut req = self.agent.get(url).header("Accept", "application/json, text/plain, */*");
        if let Some((name, value)) = header {
            req = req.header(name, value);
        }
        // (Only the host and what kind of failure it was: an address or an answer could
        // carry a key.)
        let mut resp = req.call().map_err(|e| format!("{host}: {}", plain(&e)))?;
        let status = resp.status().as_u16();
        if status != 200 {
            return Err(format!("{host} {}", said(status)));
        }
        resp.body_mut().with_config().limit(MAX_BYTES).read_to_string().map_err(|e| format!("{host}: {}", plain(&e)))
    }
}

/// What an answer that isn't the page means, in a few words.
fn said(status: u16) -> String {
    match status {
        401 | 403 => format!("doesn't take the key it was given, or this plan hasn't this page (HTTP {status})"),
        402 => "says this plan's allowance is spent (HTTP 402)".to_string(),
        429 => "says that's too many requests for now (HTTP 429)".to_string(),
        _ => format!("answered HTTP {status}"),
    }
}

/// An error as words, with nothing of the request in it (ureq's own name the address).
fn plain(e: &ureq::Error) -> String {
    match e {
        ureq::Error::Timeout(_) => "no answer in time".to_string(),
        ureq::Error::Io(e) => format!("io: {e}"),
        ureq::Error::HostNotFound => "no such host".to_string(),
        ureq::Error::BodyExceedsLimit(_) => "its answer is too big".to_string(),
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) => "its certificate or encryption failed".to_string(),
        ureq::Error::ConnectionFailed => "couldn't connect".to_string(),
        _ => "the request failed".to_string(),
    }
}

impl Fetch for Web {
    fn get(&self, url: &str) -> Result<String, String> {
        self.fetch(url, None)
    }

    fn get_with(&self, url: &str, header: (&str, &str)) -> Result<String, String> {
        self.fetch(url, Some(header))
    }
}

/// Canned answers, for tests: the first whose pattern the URL contains. A URL with none is
/// an error (as a source that's down), and every URL asked for is noted.
#[derive(Default)]
pub struct Canned {
    answers: Vec<(String, String)>,
    pub asked: Mutex<Vec<String>>,
}

impl Canned {
    pub fn new(answers: &[(&str, &str)]) -> Canned {
        Canned { answers: answers.iter().map(|(p, a)| (p.to_string(), a.to_string())).collect(), asked: Mutex::new(Vec::new()) }
    }

    /// Another answer, tried before the ones already there.
    pub fn with(mut self, pattern: &str, answer: &str) -> Canned {
        self.answers.insert(0, (pattern.to_string(), answer.to_string()));
        self
    }

    pub fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl Fetch for Canned {
    fn get(&self, url: &str) -> Result<String, String> {
        self.asked.lock().unwrap().push(url.to_string());
        self.answers.iter().find(|(p, _)| url.contains(p.as_str())).map(|(_, a)| a.clone()).ok_or_else(|| format!("{}: no answer (canned)", host_of(url)))
    }

    /// (What was asked is noted with the header's name and value after it, so a test can
    /// see a key went as a header and not in the address.)
    fn get_with(&self, url: &str, header: (&str, &str)) -> Result<String, String> {
        let got = self.get(url);
        if let Some(last) = self.asked.lock().unwrap().last_mut() {
            last.push_str(&format!(" [{}: {}]", header.0, header.1));
        }
        got
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_told_apart_and_canned_answers_found() {
        assert_eq!(host_of("https://site.api.espn.com/apis/site/v2/sports/football/nfl/scoreboard?dates=1"), "site.api.espn.com");
        assert_eq!(host_of("https://api.drand.sh/info"), "api.drand.sh");
        let c = Canned::new(&[("nfl/scoreboard", "{\"a\":1}")]).with("dates=20261116", "{\"b\":2}");
        assert_eq!(c.get("https://x/nfl/scoreboard").unwrap(), "{\"a\":1}");
        assert_eq!(c.get("https://x/nfl/scoreboard?dates=20261116").unwrap(), "{\"b\":2}");
        assert!(c.get("https://x/nba/scoreboard").is_err());
        assert_eq!(c.asked().len(), 3);
    }
}
