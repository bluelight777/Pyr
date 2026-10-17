use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "pyr-directory", version, about = "The invite directory behind Pyr's short links (https://pyr.gg/<code>).")]
struct Cli {
    /// Where the directory keeps its database.
    #[arg(long, default_value = "pyr-directory-data", global = true)]
    data: PathBuf,
    /// Where to listen (nginx is in front of it on pyr.gg).
    #[arg(long, default_value = "127.0.0.1:7781")]
    bind: SocketAddr,
    /// pyr.gg's files: its index.html is the page a short link opens.
    #[arg(long)]
    site: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the directory (default).
    Run,
    /// What a code opens.
    Show { code: String },
    /// Take a code down and keep it from being listed again.
    Block { code: String },
    /// Let a blocked code be listed again.
    Unblock { code: String },
    /// Take the community a code belongs to off the directory, for good (all its codes).
    BlockServer { code: String },
    /// How many communities and codes are listed.
    Stats,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let db = || pyr_directory::db::Db::open(&cli.data.join("directory.db"));
    match &cli.command {
        Some(Cmd::Show { code }) => {
            match db()?.lookup(&code.to_ascii_lowercase()) {
                Some(l) => println!("{}", serde_json::to_string_pretty(&l)?),
                None => println!("{code}: not listed"),
            }
            return Ok(());
        }
        Some(Cmd::Block { code }) => {
            let was = db()?.block_code(&code.to_ascii_lowercase())?;
            println!("{code}: blocked{}", if was { " (it was listed: taken down)" } else { "" });
            return Ok(());
        }
        Some(Cmd::Unblock { code }) => {
            let was = db()?.unblock_code(&code.to_ascii_lowercase())?;
            println!("{code}: {}", if was { "can be listed again" } else { "wasn't blocked" });
            return Ok(());
        }
        Some(Cmd::BlockServer { code }) => {
            match db()?.block_server_of(&code.to_ascii_lowercase())? {
                Some(name) => println!("{name:?} (the community {code} belongs to) is off the directory for good"),
                None => println!("{code}: not listed, so there's no community to block by it"),
            }
            return Ok(());
        }
        Some(Cmd::Stats) => {
            let (servers, codes) = db()?.counts();
            println!("{servers} communities, {codes} codes");
            return Ok(());
        }
        Some(Cmd::Run) | None => {}
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "pyr_directory=info,warn".into()))
        .with_ansi(false)
        .init();
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    rt.block_on(async move {
        let behind_proxy = cli.bind.ip().is_loopback();
        let running = pyr_directory::start(pyr_directory::Config { data_dir: cli.data.clone(), bind: cli.bind, site_dir: cli.site.clone(), behind_proxy }).await?;
        println!("pyr-directory listening on {} (data in {})", running.local_addr, cli.data.display());
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
        running.stop();
        Ok(())
    })
}
