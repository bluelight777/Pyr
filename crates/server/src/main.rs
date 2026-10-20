use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use pyr_proto::control::Role;
use pyr_proto::invite::InviteLink;

#[derive(Parser)]
#[command(name = "pyr-server", version, about = "Host a Pyr community: text channels, voice, webcams and screen sharing.")]
struct Cli {
    /// Folder where the server keeps its database and TLS key.
    #[arg(long, default_value = "pyr-data", global = true)]
    data: PathBuf,
    /// UDP address to listen on.
    #[arg(long, global = true, default_value_t = SocketAddr::from(([0, 0, 0, 0], pyr_proto::DEFAULT_PORT)))]
    bind: SocketAddr,
    /// Host name or public IP your friends will connect to (goes into invite links).
    /// Remembered for later starts; `--public-host ""` goes back to the local address.
    #[arg(long, global = true)]
    public_host: Option<String>,
    /// Name of the community (used the first time the server starts).
    #[arg(long)]
    name: Option<String>,
    /// The invite directory that makes invites short links (https://pyr.gg/<code>), or "off"
    /// for none. (PYR_DIRECTORY sets it too.)
    #[arg(long, global = true)]
    directory: Option<String>,
    #[command(subcommand)]
    command: Option<Cmd>,
}

impl Cli {
    /// The invite directory to list invites at, if any.
    fn invite_directory(&self) -> Option<String> {
        let d = self
            .directory
            .clone()
            .or_else(|| pyr_proto::env_var("DIRECTORY"))
            .unwrap_or_else(|| pyr_proto::directory::DIRECTORY.to_string());
        let d = d.trim().trim_end_matches('/').to_string();
        (!d.is_empty() && !d.eq_ignore_ascii_case("off") && !d.eq_ignore_ascii_case("none")).then_some(d)
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the server (default).
    Run,
    /// Create a new invite link (works while the server is running).
    Invite {
        /// How many people can use it (unlimited if omitted).
        #[arg(long)]
        uses: Option<u32>,
        /// Hours until it expires (never if omitted).
        #[arg(long)]
        hours: Option<u64>,
    },
    /// Create a new one-time link that grants ownership (if you lost the first one). It works
    /// for a week.
    OwnerInvite,
    /// Mark bug reports fixed (by number), as the owner's Mark fixed does: new and approved
    /// ones only. Works while the server is running (publishing a release that fixes them does
    /// this on the VPS).
    MarkFixed {
        /// The reports' numbers, as #3 shows in Bug reports (3, not #3).
        #[arg(required = true)]
        ids: Vec<i64>,
    },
    /// Set up the community without starting it and print its links as
    /// PYR_NAME=, PYR_OWNER_LINK= and PYR_INVITE_LINK= lines (for setup scripts).
    /// The owner link is the unused one, or a new one-time link while the community has no
    /// owner yet (empty once someone owns it).
    Links,
}

fn make_invite(cli: &Cli, owner: bool, uses: Option<u32>, hours: Option<u64>) -> Result<String> {
    pyr_server::private_data_dir(&cli.data)?;
    let tls = pyr_server::tls::load_or_create(&cli.data)?;
    let db = pyr_server::db::Db::open(&pyr_server::db::path_in(&cli.data)?)?;
    let (code, uses, role) = if owner {
        (format!("owner-{}", pyr_server::db::new_invite_code(16)), Some(1), Role::Owner)
    } else {
        // (The code is the short link too: https://pyr.gg/<code>.)
        (pyr_proto::directory::new_code(), uses, Role::Member)
    };
    let exp = hours.map(|h| pyr_proto::now_ms() + h.min(24 * 365 * 10) as i64 * 3_600_000);
    // Owner links always expire (a week unless asked for sooner).
    let exp = if owner { Some(exp.unwrap_or_else(pyr_server::owner_link_expiry).min(pyr_server::owner_link_expiry())) } else { exp };
    db.create_invite(&code, "server", uses, exp, role)?;
    let host = cli
        .public_host
        .clone()
        .filter(|h| !h.trim().is_empty())
        .or_else(|| db.get_meta("public_host").filter(|h| !h.is_empty()))
        .or_else(pyr_server::detect_lan_ip)
        .unwrap_or_else(|| "127.0.0.1".into());
    let full = InviteLink::new(host.clone(), cli.bind.port(), Some(code.clone()), tls.fingerprint).to_url();
    // An everyday invite is listed at pyr.gg too (while short links are on), and its short
    // link is the one to share. The running server's own lists include it from now on.
    let short_links = db.get_meta("short_links").as_deref() != Some("0");
    if let (false, Some(base), true, true) = (owner, cli.invite_directory(), short_links, pyr_server::directory::reachable_host(&host)) {
        db.set_listed(&code, true)?;
        let signing = pyr_server::directory::Signing::new(&tls.cert_der, &tls.key_der)?;
        let a = pyr_server::directory::announcement(&db, &cli.data.join("files"), &host, cli.bind.port(), None, false);
        match pyr_server::directory::announce_once(&base, &signing, &a) {
            Ok(r) if !r.taken.contains(&code) => {
                eprintln!("(The full link, which works without pyr.gg: {full})");
                return Ok(pyr_proto::directory::short_url(&code));
            }
            Ok(_) => {
                db.set_listed(&code, false)?;
                eprintln!("(pyr.gg has that code for another community already, so here's the full link.)");
            }
            Err(e) => eprintln!("(Couldn't list it at pyr.gg: {e}. Here's the full link, which works without it.)"),
        }
    }
    Ok(full)
}

fn main() {
    if let Err(e) = run() {
        eprintln!("\nError: {e:#}");
        // Started by double-click, the console window would close before anyone could read this.
        if cfg!(windows) {
            eprintln!("\nPress Enter to close this window.");
            let _ = std::io::stdin().read_line(&mut String::new());
        }
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut cli = Cli::parse();
    // Run without --data before the rename, the server kept everything in another folder
    // (`legacy::DATA_DIR`): it becomes ./pyr-data (rather than a new, empty community starting
    // there), or the server carries on in it if it can't be renamed.
    let old_data = std::path::Path::new(pyr_proto::legacy::DATA_DIR);
    if cli.data == std::path::Path::new("pyr-data") && !cli.data.exists() && old_data.join(pyr_proto::legacy::DB_FILE).is_file() && std::fs::rename(old_data, &cli.data).is_err() {
        cli.data = old_data.to_path_buf();
    }
    match cli.command {
        Some(Cmd::Invite { uses, hours }) => {
            println!("{}", make_invite(&cli, false, uses, hours)?);
            return Ok(());
        }
        Some(Cmd::OwnerInvite) => {
            println!("{}", make_invite(&cli, true, None, None)?);
            return Ok(());
        }
        Some(Cmd::MarkFixed { ids }) => {
            use pyr_proto::control::BugStatus;
            if !cli.data.join(pyr_server::db::DB_FILE).exists() && !cli.data.join(pyr_proto::legacy::DB_FILE).exists() {
                anyhow::bail!("there's no server in {}", cli.data.display());
            }
            let db = pyr_server::db::Db::open(&pyr_server::db::path_in(&cli.data)?)?;
            for (id, was) in db.mark_bugs_fixed(&ids)? {
                match was {
                    Some(BugStatus::New | BugStatus::Approved) => println!("Bug report #{id}: marked fixed"),
                    Some(BugStatus::Fixed) => println!("Bug report #{id}: already marked fixed"),
                    Some(BugStatus::Rejected) => println!("Bug report #{id}: rejected, left as it is"),
                    None => println!("Bug report #{id}: not on this server"),
                }
            }
            return Ok(());
        }
        Some(Cmd::Links) => {
            let links = pyr_server::setup_links(&pyr_server::ServerConfig {
                data_dir: cli.data.clone(),
                bind: cli.bind,
                public_host: cli.public_host.clone(),
                name: cli.name.clone(),
            })?;
            println!("PYR_NAME={}", links.name);
            println!("PYR_OWNER_LINK={}", links.owner);
            println!("PYR_INVITE_LINK={}", links.invite);
            return Ok(());
        }
        _ => {}
    }

    // Colors only in a real terminal: under systemd the journal would show the escape codes,
    // and the classic Windows console prints them literally.
    use std::io::IsTerminal;
    let color = std::io::stdout().is_terminal() && !cfg!(windows);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "pyr_server=info,warn".into()),
        )
        .with_ansi(color)
        .init();

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    let invite_directory = cli.invite_directory();
    rt.block_on(async move {
        let server = pyr_server::start_with(
            pyr_server::ServerConfig { data_dir: cli.data.clone(), bind: cli.bind, public_host: cli.public_host.clone(), name: cli.name.clone() },
            invite_directory,
        )
        .await?;

        let info = server.hub.info();
        println!();
        println!("  {} server \"{}\" is running on UDP {}", pyr_proto::APP_NAME, info.name, server.local_addr);
        println!("  Data folder: {}", cli.data.display());
        println!();
        if let Some(owner) = server.owner_link() {
            println!("  YOUR OWNER LINK (use it yourself first; it works once and makes you the owner):");
            println!("    {owner}");
            println!();
        }
        println!("  Invite link for friends:");
        println!("    {}", server.invite_link());
        println!();
        if server.public_host.is_none() {
            println!("  These links use your local network address. For friends outside your home,");
            println!(
                "  forward UDP port {} on your router and restart with --public-host <your public IP or domain>.",
                server.local_addr.port()
            );
            println!();
        }
        println!("  Press Ctrl+C to stop.");

        // Ctrl+C, or systemd stopping/restarting the service (an update): close everyone's
        // connection properly so their apps reconnect right away instead of timing out.
        #[cfg(unix)]
        {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            tokio::select! {
                r = tokio::signal::ctrl_c() => r?,
                _ = term.recv() => {}
            }
        }
        #[cfg(not(unix))]
        tokio::signal::ctrl_c().await?;
        println!("Shutting down...");
        server.shutdown().await;
        Ok(())
    })
}
