//! Setting up Ingle servers for people who don't run servers.
//!
//! Two ways, both ending with the same SSH install:
//!
//! * **Rent a server** ([`digitalocean`]): with an API token from the person's own
//!   DigitalOcean account, create a small Ubuntu server ("droplet") that trusts Ingle's SSH
//!   key, wait for it to boot, then install.
//! * **Use a server you have** ([`install`]): log in over SSH with a password or key and
//!   install, on any 64-bit Linux with systemd.
//!
//! The install uploads a static `ingle-server` for the machine's CPU plus
//! `install-ingle-server.sh`, which sets up a systemd service, opens the UDP port in the
//! server's firewall and prints the owner and invite links. Afterwards [`probe`] checks
//! from this PC that the server can really be reached (a cloud firewall in front of the
//! server would block it, and only the provider's website can open that).
//!
//! Work runs on a background thread and reports through [`Progress`], which the UI reads
//! every frame.

pub mod digitalocean;
pub mod install;
pub mod restore;
pub mod keys;
pub mod probe;
pub mod records;
pub mod ssh;

use std::sync::Arc;

use parking_lot::Mutex;

pub use install::{INSTALL_SCRIPT, InstallOptions, Payloads};

/// The UDP port Ingle servers use unless told otherwise.
pub const DEFAULT_PORT: u16 = ingle_proto::DEFAULT_PORT;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepState {
    Running,
    Done,
    Failed,
}

/// What a finished setup produced.
#[derive(Clone, Debug)]
pub struct Ready {
    pub name: String,
    /// Address in the links (IP or domain name).
    pub host: String,
    pub port: u16,
    /// One-time link that makes its first user the owner.
    pub owner_link: String,
    /// Link for friends.
    pub invite_link: String,
    /// Whether this PC could reach the server over UDP (None: not checked).
    pub reachable: Option<bool>,
    pub warnings: Vec<String>,
    /// The rented server, if one was created.
    pub droplet: Option<DropletRef>,
    /// The server's SSH identity (`SHA256:…`), remembered so a later connection can tell
    /// it's the same server.
    pub host_key: String,
}

/// A rented server that exists (and costs money) whatever happened afterwards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropletRef {
    pub id: u64,
    pub name: String,
    pub ip: Option<String>,
    pub region: String,
    pub size: String,
}

#[derive(Clone, Debug)]
pub struct Failure {
    pub error: String,
    /// What the person can do about it.
    pub hint: Option<String>,
    /// Set when a rented server was created before things went wrong: it keeps running
    /// (and billing) until deleted, so the UI offers to retry the install or delete it.
    pub droplet: Option<DropletRef>,
}

impl Failure {
    pub fn new(error: impl Into<String>) -> Failure {
        Failure { error: error.into(), hint: None, droplet: None }
    }
    pub fn hint(mut self, hint: impl Into<String>) -> Failure {
        self.hint = Some(hint.into());
        self
    }
}

impl From<anyhow::Error> for Failure {
    fn from(e: anyhow::Error) -> Failure {
        Failure::new(format!("{e:#}"))
    }
}

#[derive(Clone, Debug)]
pub enum Outcome {
    Ready(Ready),
    Failed(Failure),
}

/// Progress of a running setup, as shown to the person.
#[derive(Clone, Debug, Default)]
pub struct Task {
    pub steps: Vec<(String, StepState)>,
    /// Detailed output (the install script's output and the like).
    pub log: Vec<String>,
    pub outcome: Option<Outcome>,
    pub stop_requested: bool,
}

/// Shared handle to a [`Task`]: the worker writes, the UI reads.
#[derive(Clone)]
pub struct Progress {
    task: Arc<Mutex<Task>>,
    notify: Arc<dyn Fn() + Send + Sync>,
}

impl Default for Progress {
    fn default() -> Progress {
        Progress::new(Arc::new(|| {}))
    }
}

impl Progress {
    /// `notify` is called after every change (the UI passes "repaint").
    pub fn new(notify: Arc<dyn Fn() + Send + Sync>) -> Progress {
        Progress { task: Arc::new(Mutex::new(Task::default())), notify }
    }

    /// Starts a new step; the one before it is done.
    pub fn step(&self, text: impl Into<String>) {
        let text = text.into();
        tracing::info!("setup: {text}");
        {
            let mut t = self.task.lock();
            for s in t.steps.iter_mut() {
                if s.1 == StepState::Running {
                    s.1 = StepState::Done;
                }
            }
            t.steps.push((text, StepState::Running));
        }
        (self.notify)();
    }

    pub fn log(&self, line: impl Into<String>) {
        let line = line.into();
        tracing::debug!("setup log: {line}");
        self.task.lock().log.push(line);
        (self.notify)();
    }

    pub fn finish(&self, outcome: Outcome) {
        {
            let mut t = self.task.lock();
            let state = if matches!(outcome, Outcome::Ready(_)) { StepState::Done } else { StepState::Failed };
            for s in t.steps.iter_mut() {
                if s.1 == StepState::Running {
                    s.1 = state;
                }
            }
            match &outcome {
                Outcome::Ready(r) => tracing::info!("setup finished: {} at {}", r.name, r.host),
                Outcome::Failed(f) => tracing::warn!("setup failed: {}", f.error),
            }
            t.outcome = Some(outcome);
        }
        (self.notify)();
    }

    pub fn snapshot(&self) -> Task {
        self.task.lock().clone()
    }

    pub fn is_finished(&self) -> bool {
        self.task.lock().outcome.is_some()
    }

    /// Asks the worker to stop at the next safe point.
    pub fn request_stop(&self) {
        self.task.lock().stop_requested = true;
    }

    pub fn stop_requested(&self) -> bool {
        self.task.lock().stop_requested
    }
}

/// Quote a string for a POSIX shell command line.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A host name or IP address someone typed, cleaned up: no scheme, user, port or path.
pub fn clean_host(input: &str) -> String {
    let mut s = input.trim();
    if let Some((_, rest)) = s.split_once("://") {
        s = rest;
    }
    if let Some((_, rest)) = s.rsplit_once('@') {
        s = rest;
    }
    let s = s.split('/').next().unwrap_or("");
    // [v6]:port, v4:port or host:port; a bare IPv6 address has several colons.
    if let Some(rest) = s.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("").to_string();
    }
    if s.matches(':').count() == 1 {
        return s.split(':').next().unwrap_or("").to_string();
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_survives_quotes() {
        assert_eq!(sh_quote("plain"), "'plain'");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn hosts_are_cleaned() {
        assert_eq!(clean_host(" 203.0.113.5 "), "203.0.113.5");
        assert_eq!(clean_host("root@203.0.113.5"), "203.0.113.5");
        assert_eq!(clean_host("ssh://root@vps.example.com:22/"), "vps.example.com");
        assert_eq!(clean_host("[2001:db8::1]:22"), "2001:db8::1");
        assert_eq!(clean_host("2001:db8::1"), "2001:db8::1");
    }

    #[test]
    fn progress_marks_steps() {
        let p = Progress::default();
        p.step("one");
        p.step("two");
        let t = p.snapshot();
        assert_eq!(t.steps[0].1, StepState::Done);
        assert_eq!(t.steps[1].1, StepState::Running);
        p.finish(Outcome::Failed(Failure::new("nope")));
        assert_eq!(p.snapshot().steps[1].1, StepState::Failed);
    }
}
