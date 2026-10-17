//! A small SSH client for setting up servers: log in, run commands, upload files.
//!
//! Server host keys: a key we expect (one we gave a rented server, the one seen last time, or
//! one the person checked) must match exactly; otherwise this PC's `~/.ssh/known_hosts` is
//! checked (a changed key there is refused too, a matching one is fine). A server never seen
//! before isn't logged into at all: [`ConnectError::NewHostKey`] gives its fingerprint for the
//! person to check first, before any password or key is sent.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use parking_lot::Mutex;
use russh::client;
use russh::keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use russh::{ChannelMsg, Disconnect, MethodKind};

#[derive(Clone)]
pub enum Auth {
    Password(String),
    /// A private key file (OpenSSH, PEM or PKCS#8), optionally protected by a passphrase.
    KeyFile { path: PathBuf, passphrase: Option<String> },
    /// A key already in memory (Pyr's own key for servers it rents).
    Key(Arc<PrivateKey>),
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Auth::Password(_) => f.write_str("Password(..)"),
            Auth::KeyFile { path, .. } => write!(f, "KeyFile({})", path.display()),
            Auth::Key(_) => f.write_str("Key(..)"),
        }
    }
}

#[derive(Clone)]
pub struct Login {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: Auth,
    /// For a user who isn't root and needs a password for sudo (defaults to the login
    /// password when logging in with one).
    pub sudo_password: Option<String>,
    /// The server's host key fingerprint (`SHA256:…`) when we know it: any other key is
    /// refused before a password or key is sent.
    pub host_key: Option<String>,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Login")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("auth", &self.auth)
            .field("sudo_password", &self.sudo_password.as_ref().map(|_| ".."))
            .field("host_key", &self.host_key)
            .finish()
    }
}

impl Login {
    pub fn is_root(&self) -> bool {
        self.user == "root"
    }
}

/// Why logging in failed, sorted by what the person should do about it.
#[derive(Debug)]
pub enum ConnectError {
    /// Nothing answered, or the connection was refused (wrong address, server still
    /// booting, SSH not running or on another port).
    Unreachable(String),
    /// The server answered but didn't accept the login.
    Denied(String),
    /// The key file couldn't be used.
    BadKey(String),
    /// The server isn't the one we expected (its host key changed): maybe someone is in the
    /// way, so nothing was sent.
    HostKey(String),
    /// A server never seen before: nothing was sent. Its identity key (type and `SHA256:…`
    /// fingerprint) is for the person to check before logging in with it as [`Login::host_key`].
    NewHostKey { kind: String, fingerprint: String },
    Other(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::Unreachable(s)
            | ConnectError::Denied(s)
            | ConnectError::BadKey(s)
            | ConnectError::HostKey(s)
            | ConnectError::Other(s) => f.write_str(s),
            ConnectError::NewHostKey { kind, fingerprint } => {
                write!(f, "This is the first time Pyr connects to this server. Its SSH identity is {kind} key {fingerprint}.")
            }
        }
    }
}

impl std::error::Error for ConnectError {}

struct Client {
    host: String,
    port: u16,
    expected: Option<String>,
    host_key: Arc<Mutex<Option<String>>>,
    /// Why the key was refused, if it was.
    refused: Arc<Mutex<Option<String>>>,
    /// A key nobody has checked yet (its type and fingerprint): not logged into.
    unknown: Arc<Mutex<Option<(String, String)>>>,
}

/// A key type as `ssh-keygen -l` names it (ED25519, ECDSA, RSA).
fn key_kind(algorithm: &str) -> String {
    let a = algorithm.to_ascii_lowercase();
    if a.contains("ed25519") {
        "ED25519".into()
    } else if a.contains("ecdsa") {
        "ECDSA".into()
    } else if a.contains("rsa") {
        "RSA".into()
    } else {
        algorithm.to_string()
    }
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let public = key.public_key();
        let fp = public.fingerprint(HashAlg::Sha256).to_string();
        *self.host_key.lock() = Some(fp.clone());
        if let Some(expected) = &self.expected {
            if *expected != fp {
                *self.refused.lock() = Some(format!(
                    "{} isn't the server Pyr expected: its identity key is {fp}, not {expected}. Someone may be \
                     intercepting the connection, so nothing was sent. (If you reinstalled or rebuilt the server, \
                     remove it from Pyr's list of servers you set up and try again.)",
                    self.host
                ));
                return Ok(false);
            }
            return Ok(true);
        }
        // What `ssh` on this PC has seen for this server, if anything.
        if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
            let path = std::path::PathBuf::from(home).join(".ssh").join("known_hosts");
            match russh::keys::check_known_hosts_path(&self.host, self.port, &public, &path) {
                Err(russh::keys::Error::KeyChanged { line }) => {
                    *self.refused.lock() = Some(format!(
                        "{}'s identity key isn't the one your PC's SSH remembers (line {line} of {}). Someone may be \
                         intercepting the connection, so nothing was sent. If the server was reinstalled, remove that line \
                         and try again.",
                        self.host,
                        path.display()
                    ));
                    return Ok(false);
                }
                // Your own SSH has logged in here with this very key: it's the server you know.
                Ok(true) => return Ok(true),
                _ => {}
            }
        }
        // Never seen: not trusted on first use. Nothing is sent until the person has checked it.
        *self.unknown.lock() = Some((key_kind(public.algorithm().as_str()), fp));
        Ok(false)
    }
}

pub struct Session {
    handle: client::Handle<Client>,
    /// SHA256 fingerprint of the server's host key, as `ssh` prints it.
    pub host_key: String,
}

/// Output of a finished command.
#[derive(Debug, Default)]
pub struct Output {
    pub status: Option<u32>,
    pub stdout: String,
    pub stderr: String,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.status == Some(0)
    }
}

fn load_key(auth: &Auth) -> Result<Option<Arc<PrivateKey>>, ConnectError> {
    match auth {
        Auth::Password(_) => Ok(None),
        Auth::Key(k) => Ok(Some(k.clone())),
        Auth::KeyFile { path, passphrase } => {
            let text = std::fs::read_to_string(path)
                .map_err(|e| ConnectError::BadKey(format!("Couldn't read the key file {}: {e}", path.display())))?;
            if text.contains("PuTTY-User-Key-File") {
                return Err(ConnectError::BadKey(
                    "That's a PuTTY key (.ppk). Open it in PuTTYgen and use Conversions > Export OpenSSH key, then pick the exported file."
                        .into(),
                ));
            }
            if text.trim_start().starts_with("ssh-") || text.trim_start().starts_with("ecdsa-") {
                return Err(ConnectError::BadKey(
                    "That's a public key (.pub). Pick the private key file next to it (the one without .pub).".into(),
                ));
            }
            let pass = passphrase.as_deref().filter(|p| !p.is_empty());
            match russh::keys::decode_secret_key(&text, pass) {
                Ok(k) => Ok(Some(Arc::new(k))),
                Err(russh::keys::Error::KeyIsEncrypted) => Err(ConnectError::BadKey(
                    "This key is protected by a passphrase. Type it in the passphrase box.".into(),
                )),
                Err(_) if pass.is_some() => Err(ConnectError::BadKey("The passphrase for this key isn't right.".into())),
                Err(e) => Err(ConnectError::BadKey(format!("Couldn't use the key file {}: {e}", path.display()))),
            }
        }
    }
}

fn describe_io(host: &str, port: u16, e: &std::io::Error) -> String {
    use std::io::ErrorKind::*;
    match e.kind() {
        ConnectionRefused => format!("{host} refused the connection on port {port}: SSH isn't running there, or it uses another port."),
        TimedOut | WouldBlock => format!("{host} didn't answer on port {port}. Check the address, and that the server is on."),
        NotFound => format!("Couldn't find {host}. Check the address."),
        _ => {
            let s = e.to_string();
            if s.contains("failed to lookup") || s.contains("No such host") || s.contains("Name or service not known") {
                format!("Couldn't find {host}. Check the address.")
            } else {
                format!("Couldn't connect to {host} port {port}: {s}")
            }
        }
    }
}

impl Session {
    /// Connects and logs in. Gives up after `timeout` without an answer.
    pub async fn connect(login: &Login, timeout: Duration) -> Result<Session, ConnectError> {
        let key = load_key(&login.auth)?;
        let config = Arc::new(client::Config {
            inactivity_timeout: Some(Duration::from_secs(120)),
            keepalive_interval: Some(Duration::from_secs(15)),
            ..Default::default()
        });
        let host_key = Arc::new(Mutex::new(None));
        let refused = Arc::new(Mutex::new(None));
        let unknown = Arc::new(Mutex::new(None));
        let handler = Client {
            host: login.host.clone(),
            port: login.port,
            expected: login.host_key.clone().filter(|k| !k.is_empty()),
            host_key: host_key.clone(),
            refused: refused.clone(),
            unknown: unknown.clone(),
        };
        let addr = (login.host.as_str(), login.port);
        let tcp = match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr)).await {
            Err(_) => {
                return Err(ConnectError::Unreachable(format!(
                    "{} didn't answer on port {}. Check the address, and that the server is on.",
                    login.host, login.port
                )));
            }
            Ok(Err(e)) => return Err(ConnectError::Unreachable(describe_io(&login.host, login.port, &e))),
            Ok(Ok(s)) => s,
        };
        let _ = tcp.set_nodelay(true);
        let mut handle = match tokio::time::timeout(timeout, client::connect_stream(config, tcp, handler)).await {
            Err(_) => return Err(ConnectError::Unreachable(format!("{} accepted the connection but didn't speak SSH.", login.host))),
            Ok(Err(e)) => {
                if let Some(why) = refused.lock().take() {
                    return Err(ConnectError::HostKey(why));
                }
                if let Some((kind, fingerprint)) = unknown.lock().take() {
                    return Err(ConnectError::NewHostKey { kind, fingerprint });
                }
                return Err(match e {
                    russh::Error::IO(io) => ConnectError::Unreachable(describe_io(&login.host, login.port, &io)),
                    russh::Error::Disconnect | russh::Error::HUP => {
                        ConnectError::Unreachable(format!("{} closed the connection (it may still be starting up).", login.host))
                    }
                    other => ConnectError::Other(format!("SSH connection to {} failed: {other}", login.host)),
                });
            }
            Ok(Ok(h)) => h,
        };

        let user = login.user.clone();
        let denied = |what: &str| ConnectError::Denied(format!("{} didn't accept {what} for the user \"{}\".", login.host, login.user));
        let ok = match (&login.auth, key) {
            (Auth::Password(pw), _) => {
                let r = handle
                    .authenticate_password(user.clone(), pw.clone())
                    .await
                    .map_err(|e| ConnectError::Other(format!("Logging in failed: {e}")))?;
                match r {
                    russh::client::AuthResult::Success => true,
                    russh::client::AuthResult::Failure { remaining_methods, .. } => {
                        // Some servers only take passwords through "keyboard-interactive".
                        if remaining_methods.contains(&MethodKind::KeyboardInteractive) {
                            keyboard_interactive(&mut handle, &user, pw).await?
                        } else if !remaining_methods.contains(&MethodKind::Password) {
                            return Err(ConnectError::Denied(format!(
                                "{} doesn't allow logging in with a password. Use a key file instead (most providers let you add your SSH key when you create the server).",
                                login.host
                            )));
                        } else {
                            false
                        }
                    }
                }
            }
            (_, Some(key)) => {
                let hash = handle.best_supported_rsa_hash().await.ok().flatten().flatten();
                let r = handle
                    .authenticate_publickey(user.clone(), PrivateKeyWithHashAlg::new(key, hash))
                    .await
                    .map_err(|e| ConnectError::Other(format!("Logging in failed: {e}")))?;
                r.success()
            }
            (_, None) => return Err(ConnectError::BadKey("No key to log in with.".into())),
        };
        if !ok {
            return Err(match login.auth {
                Auth::Password(_) => denied("the password"),
                _ => denied("the key"),
            });
        }
        let host_key = host_key.lock().clone().unwrap_or_default();
        Ok(Session { handle, host_key })
    }

    /// Runs `command`, feeding it `stdin`, and calls `on_line` for every line of output
    /// (stdout and stderr) as it arrives.
    pub async fn run(&mut self, command: &str, stdin: &[u8], on_line: &mut (dyn FnMut(&str) + Send)) -> Result<Output> {
        let mut ch = self.handle.channel_open_session().await.context("opening an SSH channel")?;
        ch.exec(true, command).await.context("starting a command")?;
        if !stdin.is_empty() {
            ch.data(stdin).await.context("sending data")?;
        }
        ch.eof().await.context("sending data")?;
        let mut out = Output::default();
        let (mut line_out, mut line_err) = (Vec::new(), Vec::new());
        let feed = |buf: &mut Vec<u8>, data: &[u8], all: &mut String, on_line: &mut (dyn FnMut(&str) + Send)| {
            all.push_str(&String::from_utf8_lossy(data));
            buf.extend_from_slice(data);
            while let Some(i) = buf.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = buf.drain(..=i).collect();
                on_line(String::from_utf8_lossy(&line).trim_end());
            }
        };
        while let Some(msg) = ch.wait().await {
            match msg {
                ChannelMsg::Data { data } => feed(&mut line_out, &data, &mut out.stdout, on_line),
                ChannelMsg::ExtendedData { data, .. } => feed(&mut line_err, &data, &mut out.stderr, on_line),
                ChannelMsg::ExitStatus { exit_status } => out.status = Some(exit_status),
                _ => {}
            }
        }
        for rest in [line_out, line_err] {
            if !rest.is_empty() {
                on_line(String::from_utf8_lossy(&rest).trim_end());
            }
        }
        Ok(out)
    }

    /// Runs a command and returns its output (no streaming).
    pub async fn output(&mut self, command: &str) -> Result<Output> {
        self.run(command, &[], &mut |_| {}).await
    }

    /// Writes `data` to `path` on the server (through `cat`, so it works everywhere).
    pub async fn upload(&mut self, data: &[u8], path: &str) -> Result<()> {
        let cmd = format!("cat > {} && chmod 700 {}", crate::sh_quote(path), crate::sh_quote(path));
        let out = self.run(&cmd, data, &mut |_| {}).await?;
        if !out.ok() {
            bail!("couldn't write {path} on the server: {}", out.stderr.trim());
        }
        // Check it all arrived.
        let size = self.output(&format!("wc -c < {}", crate::sh_quote(path))).await?;
        let got: usize = size.stdout.trim().parse().map_err(|_| anyhow!("couldn't check the uploaded file: {}", size.stderr.trim()))?;
        if got != data.len() {
            bail!("the upload was cut short ({got} of {} bytes)", data.len());
        }
        Ok(())
    }

    pub async fn close(self) {
        let _ = self.handle.disconnect(Disconnect::ByApplication, "", "en").await;
    }
}

async fn keyboard_interactive(handle: &mut client::Handle<Client>, user: &str, password: &str) -> Result<bool, ConnectError> {
    use russh::client::KeyboardInteractiveAuthResponse as R;
    let err = |e: russh::Error| ConnectError::Other(format!("Logging in failed: {e}"));
    let mut r = handle.authenticate_keyboard_interactive_start(user.to_string(), None).await.map_err(err)?;
    for _ in 0..5 {
        match r {
            R::Success => return Ok(true),
            R::Failure { .. } => return Ok(false),
            R::InfoRequest { prompts, .. } => {
                // The password only goes to a hidden prompt asking for a password (not, say,
                // a one-time code or anything shown back on screen).
                if prompts.iter().any(|p| p.echo || !p.prompt.to_lowercase().contains("password")) {
                    return Ok(false);
                }
                let answers = prompts.iter().map(|_| password.to_string()).collect();
                r = handle.authenticate_keyboard_interactive_respond(answers).await.map_err(err)?;
            }
        }
    }
    Ok(false)
}
