//! `pyr-props`: the props feed's maker (see the library's notes and `docs/PROPS.md`).
//!
//!   pyr-props run --state DIR --out DIR --key FILE --cert FILE
//!       The service: asks each source on its own clock, and writes `feed.json` (signed) and
//!       `status.json` into `--out` whenever something changed (and every twenty minutes
//!       anyway). What it has opened is kept in `--state`, so a restart loses nothing.
//!   pyr-props check [--only nfl,crypto]
//!       Asks every source once, from nothing, and says what each would open and what it
//!       couldn't read. Writes nothing. For setting up, and for when a source changes shape.
//!   pyr-props keygen --out FILE
//!       A new feed key (prints its public half).
//!   pyr-props pubkey --key FILE
//!       A feed key's public half.
//!   pyr-props certify --release-key FILE --key PUBLIC --days N --out FILE
//!       The release key's certificate for a feed key (done where the release key is kept,
//!       never on the server).
//!   pyr-props verify --key FILE --cert FILE [--release-key PUBLIC]
//!       Whether a certificate is for that feed key, signed by the release key, and how long
//!       it has left (before the service is started with them).
//!   pyr-props status FILE
//!       A status.json, for people: each source, whether it reads, and what it has open.

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Parser, Subcommand};
use pyr_proto::crypto::Identity;
use pyr_proto::props_feed::{self, FeedCert};
use pyr_props::engine::{self, HEARTBEAT, Maker, State};
use pyr_props::http::Web;
use pyr_props::{sources, when};

#[derive(Parser)]
#[command(name = "pyr-props", version, about = "Makes the props feed Pyr servers follow")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Runs the service.
    Run {
        /// Where its state is kept (state.json).
        #[arg(long)]
        state: PathBuf,
        /// Where feed.json and status.json go (the website's props folder).
        #[arg(long)]
        out: PathBuf,
        /// The feed key (as `keygen` wrote it).
        #[arg(long)]
        key: PathBuf,
        /// The release key's certificate for it (as `certify` wrote it).
        #[arg(long)]
        cert: PathBuf,
        /// A contact for the sources to see beside pyr.gg (an email address, say).
        #[arg(long, default_value = "")]
        contact: String,
        /// Only these sources (their names, comma-separated).
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        /// One look at every source, the files written, and done.
        #[arg(long)]
        once: bool,
    },
    /// Asks every source once and says what it would open. Writes nothing.
    Check {
        #[arg(long, value_delimiter = ',')]
        only: Vec<String>,
        #[arg(long, default_value = "")]
        contact: String,
    },
    /// Makes a feed key.
    Keygen {
        #[arg(long)]
        out: PathBuf,
    },
    /// Says a feed key's public half.
    Pubkey {
        #[arg(long)]
        key: PathBuf,
    },
    /// Checks a feed key's certificate.
    Verify {
        #[arg(long)]
        key: PathBuf,
        #[arg(long)]
        cert: PathBuf,
        /// The release key's public half (Pyr's own, unless said).
        #[arg(long)]
        release_key: Option<String>,
    },
    /// Says what a status.json says, for people.
    Status { file: PathBuf },
    /// Certifies a feed key with the release key.
    Certify {
        #[arg(long)]
        release_key: PathBuf,
        /// The feed key's public half.
        #[arg(long)]
        key: String,
        #[arg(long, default_value_t = 365)]
        days: i64,
        #[arg(long)]
        out: PathBuf,
    },
}

fn load_key(path: &Path) -> Result<Identity, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    pyr_proto::b64::decode_array::<32>(text.trim()).map(Identity::from_seed).ok_or_else(|| format!("{} isn't a key", path.display()))
}

fn main() {
    let cli = Cli::parse();
    if let Err(e) = match cli.cmd {
        Cmd::Run { state, out, key, cert, contact, only, once } => run(&state, &out, &key, &cert, &contact, &only, once),
        Cmd::Check { only, contact } => check(&only, &contact),
        Cmd::Keygen { out } => keygen(&out),
        Cmd::Certify { release_key, key, days, out } => certify(&release_key, &key, days, &out),
        Cmd::Verify { key, cert, release_key } => verify(&key, &cert, release_key.as_deref()),
        Cmd::Status { file } => status(&file),
        Cmd::Pubkey { key } => load_key(&key).map(|k| println!("{}", k.user_id())),
    } {
        eprintln!("pyr-props: {e}");
        std::process::exit(1);
    }
}

fn keygen(out: &Path) -> Result<(), String> {
    if out.exists() {
        return Err(format!("{} is there already: a key is made once", out.display()));
    }
    let id = Identity::generate();
    std::fs::write(out, pyr_proto::b64::encode(&id.seed())).map_err(|e| format!("writing {}: {e}", out.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(out, std::fs::Permissions::from_mode(0o600));
    }
    println!("{}", id.user_id());
    Ok(())
}

fn certify(release_key: &Path, key: &str, days: i64, out: &Path) -> Result<(), String> {
    if !pyr_proto::crypto::is_valid_user_id(key) {
        return Err("--key is the feed key's public half (what keygen printed)".into());
    }
    let release = load_key(release_key)?;
    let not_after = pyr_proto::now_ms() / 1000 + days.clamp(1, 3650) * 86_400;
    let cert = props_feed::certify(&release, key, not_after);
    std::fs::write(out, serde_json::to_string_pretty(&cert).map_err(|e| e.to_string())?).map_err(|e| format!("writing {}: {e}", out.display()))?;
    println!("certified by {} until {}", release.user_id(), when::iso_date(not_after * 1000));
    Ok(())
}

fn load_cert(path: &Path) -> Result<FeedCert, String> {
    serde_json::from_str(&std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?).map_err(|e| format!("{} isn't a certificate: {e}", path.display()))
}

fn verify(key: &Path, cert: &Path, release_key: Option<&str>) -> Result<(), String> {
    let key = load_key(key)?;
    let cert = load_cert(cert)?;
    if cert.key != key.user_id() {
        return Err("the certificate is for another key".into());
    }
    let release = release_key.unwrap_or(pyr_proto::release::RELEASE_KEY);
    if !props_feed::cert_ok(&cert, release) {
        return Err("the certificate isn't signed by the release key".into());
    }
    let left = cert.not_after - pyr_proto::now_ms() / 1000;
    if left <= 0 {
        return Err(format!("the certificate ran out on {}", when::iso_date(cert.not_after * 1000)));
    }
    println!("the feed key {} is certified until {} ({} days)", key.user_id(), when::iso_date(cert.not_after * 1000), left / 86_400);
    Ok(())
}

fn status(file: &Path) -> Result<(), String> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(file).map_err(|e| format!("reading {}: {e}", file.display()))?).map_err(|e| format!("{} isn't a status file: {e}", file.display()))?;
    let text = |v: &serde_json::Value| v.as_str().unwrap_or("").to_string();
    println!("made {} by pyr-props {}", text(&v["made"]), text(&v["version"]));
    let mut bad = 0;
    for (name, s) in v["sources"].as_object().into_iter().flatten() {
        let (ok, error) = (s["ok"].as_bool().unwrap_or(false), text(&s["error"]));
        let never = s["last_run"].is_null();
        let word = if never { "not read yet".to_string() } else if ok { "ok".to_string() } else { format!("FAILED: {error}") };
        if !ok && !never {
            bad += 1;
        }
        println!("  {name:<10} {word}  ({} not over, {} opened, {} settled; last read well {})", s["not_over"].as_u64().unwrap_or(0), s["opened"].as_u64().unwrap_or(0), s["settled"].as_u64().unwrap_or(0), s["last_ok"].as_str().unwrap_or("never"));
        for t in s["trouble"].as_array().into_iter().flatten().take(3) {
            println!("             couldn't: {}", text(t));
        }
    }
    for (cat, c) in v["categories"].as_object().into_iter().flatten() {
        println!("  {cat:<14} {} open, {} waiting for their results, {} over", c["open"].as_u64().unwrap_or(0), c["waiting"].as_u64().unwrap_or(0), c["over"].as_u64().unwrap_or(0));
    }
    match bad {
        0 => Ok(()),
        1 => Err("1 source isn't reading".into()),
        n => Err(format!("{n} sources aren't reading")),
    }
}

fn check(only: &[String], contact: &str) -> Result<(), String> {
    let web = Web::new(contact);
    let now = pyr_proto::now_ms();
    let mut m = Maker::new(State::default(), sources::all(only));
    if m.sources.is_empty() {
        return Err("no such source".into());
    }
    m.step(now, &web, true);
    let mut bad = 0;
    for s in &m.sources {
        let st = m.state.sources.get(s.name()).cloned().unwrap_or_default();
        let props: Vec<_> = m.state.props.values().filter(|k| k.by == s.name()).collect();
        if st.error.is_empty() {
            println!("{:<10} ok: would open {}", s.name(), props.len());
        } else {
            bad += 1;
            println!("{:<10} FAILED: {}", s.name(), st.error);
        }
        for t in &st.trouble {
            println!("           couldn't: {t}");
        }
        for k in props.iter().take(4) {
            let odds: Vec<String> = k.prop.outcomes.iter().zip(&k.prop.prior).map(|(o, p)| format!("{o} {:.0}%", p * 100.0)).collect();
            println!("           {}  [{}]  closes {}", k.prop.q, odds.join(" | "), when::iso(k.prop.closes));
        }
        if props.len() > 4 {
            println!("           … and {} more", props.len() - 4);
        }
    }
    if bad > 0 { Err(format!("{bad} of {} sources couldn't be read", m.sources.len())) } else { Ok(()) }
}

fn run(state_dir: &Path, out: &Path, key: &Path, cert: &Path, contact: &str, only: &[String], once: bool) -> Result<(), String> {
    let key = load_key(key)?;
    let cert = load_cert(cert)?;
    if cert.key != key.user_id() {
        return Err("the certificate is for another key".into());
    }
    std::fs::create_dir_all(state_dir).map_err(|e| format!("{}: {e}", state_dir.display()))?;
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    let state_file = state_dir.join("state.json");
    let state: State = match std::fs::read_to_string(&state_file) {
        Ok(t) => serde_json::from_str(&t).map_err(|e| format!("{} can't be read ({e}): move it aside to start afresh", state_file.display()))?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => State::default(),
        Err(e) => return Err(format!("reading {}: {e}", state_file.display())),
    };
    let web = Web::new(contact);
    let mut m = Maker::new(state, sources::all(only));
    eprintln!("{} pyr-props {}: {} sources, {} props kept, the feed signed by {}", when::iso(pyr_proto::now_ms()), env!("CARGO_PKG_VERSION"), m.sources.len(), m.state.props.len(), key.user_id());
    let mut written = 0i64;
    let mut warned = 0i64;
    loop {
        let now = pyr_proto::now_ms();
        // (A feed signed after its key's certificate ran out counts for nothing.)
        let left = cert.not_after - now / 1000;
        if left < 30 * 86_400 && now - warned > 86_400_000 {
            warned = now;
            eprintln!("{} the feed key's certificate runs out on {}: the next Pyr release brings a new one", when::iso(now), when::iso_date(cert.not_after * 1000));
        }
        let changed = m.step(now, &web, once);
        if changed || now - written >= HEARTBEAT || once {
            let now = pyr_proto::now_ms();
            let wrote = serde_json::to_string(&m.state)
                .map_err(|e| e.to_string())
                .and_then(|s| engine::write_whole(&state_file, &s).map_err(|e| format!("{}: {e}", state_file.display())))
                .and_then(|()| engine::write_whole(&out.join("feed.json"), &engine::signed(&key, &cert, &m.feed(now))).map_err(|e| format!("{}: {e}", out.display())))
                .and_then(|()| engine::write_whole(&out.join("status.json"), &serde_json::to_string_pretty(&m.status(now)).unwrap_or_default()).map_err(|e| format!("{}: {e}", out.display())));
            match wrote {
                Ok(()) => written = now,
                Err(e) => eprintln!("{} couldn't write: {e}", when::iso(now)),
            }
        }
        if once {
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(20));
    }
}
