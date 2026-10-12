//! A small SSH client for setting up servers: log in, run commands, upload files.
//!
//! Server host keys are trusted on first use (there's nothing to compare them with for
//! a server someone just bought); the fingerprint is reported so it can be recorded.

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
    /// A key already in memory (Ingle's own key for servers it rents).
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

#[derive(Clone, Debug)]
pub struct Login {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: Auth,
    /// For a user who isn't root and needs a password for sudo (defaults to the login
    /// password when logging in with one).
    pub sudo_password: Option<String>,
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
    Other(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::Unreachable(s) | ConnectError::Denied(s) | ConnectError::BadKey(s) | ConnectError::Other(s) => {
                f.write_str(s)
            }
        }
    }
}

impl std::error::Error for ConnectError {}

struct Client {
    host_key: Arc<Mutex<Option<String>>>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let fp = key.public_key().fingerprint(HashAlg::Sha256).to_string();
        *self.host_key.lock() = Some(fp);
        Ok(true)
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
        let handler = Client { host_key: host_key.clone() };
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
                let answers = prompts.iter().map(|_| password.to_string()).collect();
                r = handle.authenticate_keyboard_interactive_respond(answers).await.map_err(err)?;
            }
        }
    }
    Ok(false)
}
