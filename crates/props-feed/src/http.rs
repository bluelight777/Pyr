//! Reading the sources: plain HTTPS GETs that say who's asking, never hurried, never big.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Something that fetches a URL's text (the web; canned answers in tests).
pub trait Fetch: Sync {
    fn get(&self, url: &str) -> Result<String, String>;
}

/// The most a source's answer may be (a scoreboard is a few hundred kilobytes).
const MAX_BYTES: u64 = 6 << 20;
/// The least time between two requests to one host: nothing here needs to hurry, and the
/// sources are other people's servers.
const GAP: Duration = Duration::from_millis(1200);

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

impl Fetch for Web {
    fn get(&self, url: &str) -> Result<String, String> {
        let host = host_of(url).to_string();
        // (A moment since the last request to this host.)
        let wait = {
            let mut last = self.last.lock().unwrap_or_else(|e| e.into_inner());
            let now = Instant::now();
            let wait = last.get(&host).map(|t| (*t + GAP).saturating_duration_since(now)).unwrap_or_default();
            last.insert(host.clone(), now + wait);
            wait
        };
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        let mut resp = self.agent.get(url).header("Accept", "application/json, text/plain, */*").call().map_err(|e| format!("{host}: {e}"))?;
        let status = resp.status().as_u16();
        if status != 200 {
            return Err(format!("{host} answered HTTP {status}"));
        }
        resp.body_mut().with_config().limit(MAX_BYTES).read_to_string().map_err(|e| format!("{host}: {e}"))
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
