//! Installing Ingle Server on a Linux machine over SSH.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::ssh::{Auth, ConnectError, Login, Session};
use crate::{Failure, Progress, sh_quote};

/// The setup script that runs on the server (also handy for setting one up by hand).
pub const INSTALL_SCRIPT: &str = include_str!("../assets/install-ingle-server.sh");

/// File name of the setup script when saved for doing it by hand.
pub const SCRIPT_NAME: &str = "install-ingle-server.sh";

/// Where the Linux server programs that get uploaded live.
#[derive(Clone, Debug)]
pub struct Payloads {
    pub dir: PathBuf,
}

impl Payloads {
    pub fn file_name(arch: &str) -> String {
        format!("ingle-server-linux-{arch}")
    }

    /// The program for a CPU as `uname -m` names it.
    pub fn for_machine(&self, uname_m: &str) -> Result<PathBuf> {
        let arch = match uname_m.trim() {
            "x86_64" | "amd64" => "x86_64",
            "aarch64" | "arm64" | "armv8l" => "aarch64",
            other => bail!("This server's processor ({other}) isn't supported: Ingle needs a 64-bit x86 or ARM server."),
        };
        let p = self.dir.join(Self::file_name(arch));
        if !p.is_file() {
            bail!(
                "The Linux server program for {arch} is missing from this Ingle install ({}). Reinstall Ingle to get it back.",
                p.display()
            );
        }
        Ok(p)
    }

    /// The usual place, next to the app: `<app folder>/vps` on Windows (and when run from a
    /// build folder), `/usr/lib/ingle/vps` for the Linux package. `INGLE_VPS_DIR` overrides.
    pub fn find() -> Payloads {
        if let Some(d) = std::env::var_os("INGLE_VPS_DIR") {
            return Payloads { dir: PathBuf::from(d) };
        }
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
        let mut candidates = Vec::new();
        if let Some(d) = &exe_dir {
            candidates.push(d.join("vps"));
            // /usr/bin/ingle -> /usr/lib/ingle/vps
            candidates.push(d.join("..").join("lib").join("ingle").join("vps"));
            // target/release/ingle -> dist/vps (running from a source checkout)
            candidates.push(d.join("..").join("..").join("dist").join("vps"));
        }
        candidates.push(PathBuf::from("/usr/lib/ingle/vps"));
        let dir = candidates
            .iter()
            .find(|d| d.join(Self::file_name("x86_64")).is_file())
            .cloned()
            .unwrap_or_else(|| candidates.first().cloned().unwrap_or_else(|| PathBuf::from("vps")));
        Payloads { dir }
    }
}

#[derive(Clone, Debug)]
pub struct InstallOptions {
    /// Name for a new community.
    pub name: String,
    /// Address in the invite links: what friends connect to.
    pub public_host: String,
    pub port: u16,
    pub payloads: Payloads,
    /// A freshly created server: keep trying to log in while it boots (up to this long).
    pub wait_for_boot: Option<Duration>,
}

/// What the install produced.
#[derive(Clone, Debug)]
pub struct Installed {
    pub name: String,
    pub owner_link: String,
    pub invite_link: String,
    pub warnings: Vec<String>,
    pub host_key: String,
}

/// Facts about the server gathered before installing.
#[derive(Debug, Default)]
struct Machine {
    arch: String,
    os: String,
    root: bool,
    systemd: bool,
    sudo: bool,
}

async fn connect(login: &Login, opts: &InstallOptions, progress: &Progress) -> Result<Session, Failure> {
    let started = Instant::now();
    let mut announced = false;
    loop {
        match Session::connect(login, Duration::from_secs(15)).await {
            Ok(s) => return Ok(s),
            Err(e) => {
                // A server that's still booting refuses connections, and for a moment after
                // SSH starts it may not have our key yet.
                let retry = match (&e, opts.wait_for_boot) {
                    (ConnectError::Unreachable(_), Some(limit)) => started.elapsed() < limit,
                    (ConnectError::Denied(_), Some(limit)) => started.elapsed() < limit.min(Duration::from_secs(120)),
                    _ => false,
                };
                if !retry || progress.stop_requested() {
                    return Err(connect_failure(e, login));
                }
                if !announced {
                    progress.log("The server is still starting up; trying again…");
                    announced = true;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

fn connect_failure(e: ConnectError, login: &Login) -> Failure {
    let hint = match &e {
        ConnectError::Unreachable(_) => Some(format!(
            "Check the address and that the server is running. If your provider has a firewall in its website, it must allow SSH (TCP port {}).",
            login.port
        )),
        ConnectError::Denied(_) => Some(match login.auth {
            Auth::Password(_) => "Check the user name and password (your provider's welcome email or control panel has them). Some servers only allow keys: then use your key file.".to_string(),
            _ => "Check the user name, and that this key is allowed on the server (in ~/.ssh/authorized_keys for that user).".to_string(),
        }),
        ConnectError::BadKey(_) => None,
        ConnectError::Other(_) => None,
    };
    Failure { error: e.to_string(), hint, droplet: None }
}

async fn inspect(s: &mut Session) -> Result<Machine> {
    let out = s
        .output(
            "uname -m; id -u; (. /etc/os-release 2>/dev/null && echo \"$PRETTY_NAME\") || uname -s; \
             if [ -d /run/systemd/system ]; then echo systemd; else echo no-systemd; fi; \
             if command -v sudo >/dev/null 2>&1; then echo sudo; else echo no-sudo; fi",
        )
        .await?;
    let lines: Vec<&str> = out.stdout.lines().map(str::trim).collect();
    if lines.len() < 5 {
        bail!("couldn't look around on the server: {}{}", out.stdout.trim(), out.stderr.trim());
    }
    Ok(Machine {
        arch: lines[0].to_string(),
        root: lines[1] == "0",
        os: lines[2].to_string(),
        systemd: lines[3] == "systemd",
        sudo: lines[4] == "sudo",
    })
}

/// How to become root: nothing (already root), `sudo -n`, or `sudo -S` with a password.
enum Elevate {
    None,
    Sudo,
    SudoPassword(String),
}

async fn elevation(s: &mut Session, login: &Login, m: &Machine) -> Result<Elevate, Failure> {
    if m.root {
        return Ok(Elevate::None);
    }
    if !m.sudo {
        return Err(Failure::new(format!("\"{}\" isn't root, and this server has no sudo.", login.user))
            .hint("Log in as root instead, or install sudo and give this user sudo rights."));
    }
    let free = s.output("sudo -n true").await.map_err(Failure::from)?;
    if free.ok() {
        return Ok(Elevate::Sudo);
    }
    let password = login.sudo_password.clone().filter(|p| !p.is_empty()).or_else(|| match &login.auth {
        Auth::Password(p) => Some(p.clone()),
        _ => None,
    });
    let Some(pw) = password else {
        return Err(Failure::new(format!("\"{}\" needs a password for sudo.", login.user))
            .hint("Type the user's password in the sudo password box, or log in as root."));
    };
    let check = s.run("sudo -S -p '' true", format!("{pw}\n").as_bytes(), &mut |_| {}).await.map_err(Failure::from)?;
    if !check.ok() {
        return Err(Failure::new(format!("sudo didn't accept the password for \"{}\", or the user isn't allowed to use sudo.", login.user))
            .hint("Check the password, or log in as root."));
    }
    Ok(Elevate::SudoPassword(pw))
}

/// Logs in, uploads the server program and setup script, and runs the setup.
pub async fn install(login: &Login, opts: &InstallOptions, progress: &Progress) -> Result<Installed, Failure> {
    progress.step(format!("Connecting to {}", login.host));
    let mut s = connect(login, opts, progress).await?;
    if !s.host_key.is_empty() {
        progress.log(format!("Server key: {}", s.host_key));
    }
    let result = install_with(&mut s, login, opts, progress).await;
    let host_key = s.host_key.clone();
    s.close().await;
    result.map(|mut i| {
        i.host_key = host_key;
        i
    })
}

async fn install_with(s: &mut Session, login: &Login, opts: &InstallOptions, progress: &Progress) -> Result<Installed, Failure> {
    progress.step("Checking the server");
    let m = inspect(s).await.map_err(Failure::from)?;
    progress.log(format!("{} ({}), logged in as {}", m.os, m.arch, login.user));
    if !m.systemd {
        return Err(Failure::new(format!("This server ({}) doesn't use systemd, which Ingle's setup needs.", m.os))
            .hint("Ubuntu 22.04 or newer, Debian 11 or newer, Fedora, Rocky or Alma Linux all work."));
    }
    let binary = opts.payloads.for_machine(&m.arch).map_err(Failure::from)?;
    let elevate = elevation(s, login, &m).await?;
    if progress.stop_requested() {
        return Err(Failure::new("Stopped."));
    }

    let data = std::fs::read(&binary).with_context(|| format!("reading {}", binary.display())).map_err(Failure::from)?;
    progress.step(format!("Uploading Ingle Server ({:.1} MB)", data.len() as f64 / 1_048_576.0));
    let tmp = s.output("mktemp -d /tmp/ingle-setup.XXXXXX").await.map_err(Failure::from)?;
    let dir = tmp.stdout.trim().to_string();
    if !tmp.ok() || !dir.starts_with("/tmp/ingle-setup.") {
        return Err(Failure::new(format!("Couldn't make a temporary folder on the server: {}", tmp.stderr.trim())));
    }
    let bin_path = format!("{dir}/ingle-server");
    let script_path = format!("{dir}/{SCRIPT_NAME}");
    let uploaded = async {
        s.upload(&data, &bin_path).await?;
        s.upload(INSTALL_SCRIPT.as_bytes(), &script_path).await
    }
    .await;
    if let Err(e) = uploaded {
        let _ = s.output(&format!("rm -rf {}", sh_quote(&dir))).await;
        return Err(Failure::new(format!("Uploading to the server failed: {e:#}")).hint("Check that the server's disk isn't full, then try again."));
    }

    let mut cmd = format!(
        "sh {} --binary {} --public-host {} --port {}",
        sh_quote(&script_path),
        sh_quote(&bin_path),
        sh_quote(&opts.public_host),
        opts.port
    );
    if !opts.name.trim().is_empty() {
        cmd.push_str(&format!(" --name {}", sh_quote(opts.name.trim())));
    }
    let (cmd, stdin) = match &elevate {
        Elevate::None => (cmd, String::new()),
        Elevate::Sudo => (format!("sudo -n {cmd}"), String::new()),
        Elevate::SudoPassword(pw) => (format!("sudo -S -p '' {cmd}"), format!("{pw}\n")),
    };
    let mut warnings = Vec::new();
    let mut error = None;
    let (mut owner, mut invite, mut name) = (None, None, None);
    let out = s
        .run(&cmd, stdin.as_bytes(), &mut |line: &str| {
            if let Some(step) = line.strip_prefix("==> ") {
                progress.step(step.to_string());
            } else if let Some(w) = line.strip_prefix("WARNING: ") {
                warnings.push(w.to_string());
                progress.log(line.to_string());
            } else if let Some(e) = line.strip_prefix("ERROR: ") {
                error = Some(e.to_string());
                progress.log(line.to_string());
            } else if let Some(v) = line.strip_prefix("INGLE_OWNER_LINK=") {
                owner = Some(v.to_string());
            } else if let Some(v) = line.strip_prefix("INGLE_INVITE_LINK=") {
                invite = Some(v.to_string());
            } else if let Some(v) = line.strip_prefix("INGLE_NAME=") {
                name = Some(v.to_string());
            } else if !line.starts_with("INGLE_") {
                progress.log(line.to_string());
            }
        })
        .await;
    let _ = s.output(&format!("rm -rf {}", sh_quote(&dir))).await;
    let out = out.map_err(Failure::from)?;
    match (owner, invite) {
        (Some(owner_link), Some(invite_link)) if out.ok() => Ok(Installed {
            name: name.unwrap_or_else(|| opts.name.clone()),
            owner_link,
            invite_link,
            warnings,
            host_key: String::new(),
        }),
        _ => Err(Failure::new(error.unwrap_or_else(|| {
            format!("The setup on the server failed (exit code {}).", out.status.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into()))
        }))
        .hint("The details are in the log below. Fix the problem on the server and try again: running the setup again is safe.")),
    }
}

/// The full "use a server I have" setup: install over SSH, then check that this PC can
/// reach the server over UDP.
pub fn setup_existing(login: Login, opts: InstallOptions, progress: Progress, records_dir: Option<PathBuf>) {
    let outcome = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Err(e) => Err(Failure::new(format!("couldn't start: {e}"))),
        Ok(rt) => rt.block_on(install(&login, &opts, &progress)),
    };
    let outcome = outcome.map(|i| finish(&login, &opts, i, None, &progress));
    if let (Ok(ready), Some(dir)) = (&outcome, &records_dir) {
        let _ = crate::records::save(
            dir,
            &crate::records::HostedServer {
                name: ready.name.clone(),
                host: ready.host.clone(),
                port: ready.port,
                provider: "ssh".into(),
                ssh_user: login.user.clone(),
                ssh_port: login.port,
                invite_link: Some(ready.invite_link.clone()),
                created_ms: ingle_proto::now_ms(),
                installed: true,
                ..Default::default()
            },
        );
    }
    progress.finish(match outcome {
        Ok(r) => crate::Outcome::Ready(r),
        Err(f) => crate::Outcome::Failed(f),
    });
}

/// Checks reachability and builds the result.
pub(crate) fn finish(login: &Login, opts: &InstallOptions, i: Installed, droplet: Option<crate::DropletRef>, progress: &Progress) -> crate::Ready {
    progress.step("Checking that friends can reach it");
    let reachable = crate::probe::reachable_blocking(&i.owner_link, 3, Duration::from_secs(4));
    if let Err(e) = &reachable {
        progress.log(format!("Couldn't reach the server over UDP: {e:#}"));
    }
    let _ = login;
    crate::Ready {
        name: i.name,
        host: opts.public_host.clone(),
        port: opts.port,
        owner_link: i.owner_link,
        invite_link: i.invite_link,
        reachable: Some(reachable.is_ok()),
        warnings: i.warnings,
        droplet,
    }
}

/// Saves the server program and setup script into `folder`, for setting a server up by
/// hand. Returns the files written.
pub fn save_manual_files(payloads: &Payloads, folder: &Path) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
    let mut written = Vec::new();
    for arch in ["x86_64", "aarch64"] {
        let src = payloads.dir.join(Payloads::file_name(arch));
        if src.is_file() {
            let dst = folder.join(Payloads::file_name(arch));
            std::fs::copy(&src, &dst).with_context(|| format!("copying {}", src.display()))?;
            written.push(dst);
        }
    }
    if written.is_empty() {
        bail!("The Linux server programs are missing from this Ingle install ({}).", payloads.dir.display());
    }
    let script = folder.join(SCRIPT_NAME);
    std::fs::write(&script, INSTALL_SCRIPT).with_context(|| format!("writing {}", script.display()))?;
    written.push(script);
    Ok(written)
}
