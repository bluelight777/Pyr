use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use ingle_proto::control::Role;
use ingle_proto::invite::InviteLink;

#[derive(Parser)]
#[command(name = "ingle-server", version, about = "Host an Ingle community: text channels, voice, webcams and screen sharing.")]
struct Cli {
    /// Folder where the server keeps its database and TLS key.
    #[arg(long, default_value = "ingle-data", global = true)]
    data: PathBuf,
    /// UDP address to listen on.
    #[arg(long, global = true, default_value_t = SocketAddr::from(([0, 0, 0, 0], ingle_proto::DEFAULT_PORT)))]
    bind: SocketAddr,
    /// Host name or public IP your friends will connect to (goes into invite links).
    /// Remembered for later starts; `--public-host ""` goes back to the local address.
    #[arg(long, global = true)]
    public_host: Option<String>,
    /// Name of the community (used the first time the server starts).
    #[arg(long)]
    name: Option<String>,
    #[command(subcommand)]
    command: Option<Cmd>,
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
    /// Set up the community without starting it and print its links as
    /// INGLE_NAME=, INGLE_OWNER_LINK= and INGLE_INVITE_LINK= lines (for setup scripts).
    /// The owner link is the unused one, or a new one-time link while the community has no
    /// owner yet (empty once someone owns it).
    Links,
}

fn make_invite(cli: &Cli, owner: bool, uses: Option<u32>, hours: Option<u64>) -> Result<String> {
    std::fs::create_dir_all(&cli.data)?;
    let tls = ingle_server::tls::load_or_create(&cli.data)?;
    let db = ingle_server::db::Db::open(&cli.data.join("ingle.db"))?;
    let (code, uses, role) = if owner {
        (format!("owner-{}", ingle_server::db::new_invite_code(16)), Some(1), Role::Owner)
    } else {
        (ingle_server::db::new_invite_code(10), uses, Role::Member)
    };
    let exp = hours.map(|h| ingle_proto::now_ms() + h.min(24 * 365 * 10) as i64 * 3_600_000);
    // Owner links always expire (a week unless asked for sooner).
    let exp = if owner { Some(exp.unwrap_or_else(ingle_server::owner_link_expiry).min(ingle_server::owner_link_expiry())) } else { exp };
    db.create_invite(&code, "server", uses, exp, role)?;
    let host = cli
        .public_host
        .clone()
        .filter(|h| !h.trim().is_empty())
        .or_else(|| db.get_meta("public_host").filter(|h| !h.is_empty()))
        .or_else(ingle_server::detect_lan_ip)
        .unwrap_or_else(|| "127.0.0.1".into());
    Ok(InviteLink::new(host, cli.bind.port(), Some(code), tls.fingerprint).to_url())
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
    let cli = Cli::parse();
    match cli.command {
        Some(Cmd::Invite { uses, hours }) => {
            println!("{}", make_invite(&cli, false, uses, hours)?);
            return Ok(());
        }
        Some(Cmd::OwnerInvite) => {
            println!("{}", make_invite(&cli, true, None, None)?);
            return Ok(());
        }
        Some(Cmd::Links) => {
            let links = ingle_server::setup_links(&ingle_server::ServerConfig {
                data_dir: cli.data.clone(),
                bind: cli.bind,
                public_host: cli.public_host.clone(),
                name: cli.name.clone(),
            })?;
            println!("INGLE_NAME={}", links.name);
            println!("INGLE_OWNER_LINK={}", links.owner);
            println!("INGLE_INVITE_LINK={}", links.invite);
            return Ok(());
        }
        _ => {}
    }

    // Colours only in a real terminal: under systemd the journal would show the escape codes,
    // and the classic Windows console prints them literally.
    use std::io::IsTerminal;
    let colour = std::io::stdout().is_terminal() && !cfg!(windows);
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "ingle_server=info,warn".into()),
        )
        .with_ansi(colour)
        .init();

    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async move {
        let server = ingle_server::start(ingle_server::ServerConfig {
            data_dir: cli.data.clone(),
            bind: cli.bind,
            public_host: cli.public_host.clone(),
            name: cli.name.clone(),
        })
        .await?;

        let info = server.hub.info();
        println!();
        println!("  {} server \"{}\" is running on UDP {}", ingle_proto::APP_NAME, info.name, server.local_addr);
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
