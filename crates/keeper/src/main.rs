//! pyr-keeper: the Pyr Library Keeper (see the crate's docs).
//!
//!   pyr-keeper keygen --out keeper-key.txt      a new key (prints its public half)
//!   pyr-keeper run --config keeper.json         every `poll_secs`, for good
//!   pyr-keeper once --config keeper.json        one pass, then stop
//!   pyr-keeper release <submission> --config …  let a held one be looked at again
//!   pyr-keeper import <list> --config …       take in a list the release key signed
//!   pyr-keeper unlist <id or name> --config … take a plugin or skin off the list

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use pyr_keeper::build::Sandbox;
use pyr_keeper::review::Claude;
use pyr_keeper::{Config, Keeper};

#[derive(Parser)]
#[command(name = "pyr-keeper", about = "The Pyr Library Keeper")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Keygen {
        #[arg(long)]
        out: PathBuf,
    },
    Run {
        #[arg(long)]
        config: PathBuf,
    },
    Once {
        #[arg(long)]
        config: PathBuf,
    },
    /// Take in a list Firelit Games signed with the release key (new listings, endorsed
    /// programs), and put it up.
    Import {
        list: PathBuf,
        #[arg(long)]
        config: PathBuf,
    },
    /// Build an add-on's folder in the sandbox, as a check that building works here.
    Selftest {
        folder: PathBuf,
        #[arg(long)]
        config: PathBuf,
    },
    /// Take a submission off the held list (after a person has looked, or to try again).
    Release {
        submission: i64,
        #[arg(long)]
        config: PathBuf,
    },
    /// Take a plugin or skin off the list (by its id, or its name), and put the list up again.
    Unlist {
        which: String,
        #[arg(long)]
        config: PathBuf,
    },
}

/// For the commands that only sign and put up lists: nothing's reviewed or built.
struct NoReview;
impl pyr_keeper::review::Reviewer for NoReview {
    fn review(&self, _: &pyr_keeper::review::Request) -> Result<pyr_keeper::review::Verdict> {
        anyhow::bail!("not reviewing")
    }
}
struct NoBuild;
impl pyr_keeper::build::Builder for NoBuild {
    fn build_addon(&self, _: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
        anyhow::bail!("not building")
    }
}

fn load(config: &PathBuf) -> Result<Config> {
    serde_json::from_str(&std::fs::read_to_string(config).with_context(|| format!("reading {}", config.display()))?).context("the config")
}

fn sandbox(cfg: &Config) -> Sandbox {
    Sandbox {
        rustup_home: cfg.rustup_home.clone(),
        cargo_home: cfg.cargo_home.clone(),
        kit: cfg.kit.clone(),
        vendor: cfg.vendor.clone(),
        work: cfg.work.clone(),
        bwrap: cfg.bwrap.clone(),
        timeout: Duration::from_secs(600),
        memory_mb: 4096,
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())).init();
    match Cli::parse().cmd {
        Cmd::Keygen { out } => {
            anyhow::ensure!(!out.exists(), "{} is there already", out.display());
            let id = pyr_proto::crypto::Identity::generate();
            std::fs::write(&out, pyr_proto::b64::encode(&id.seed()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&out, std::fs::Permissions::from_mode(0o600))?;
            }
            println!("{}", id.user_id());
        }
        Cmd::Import { list, config } => {
            let cfg = load(&config)?;
            let mut k = Keeper::new(cfg, &NoReview, &NoBuild)?;
            k.import(&std::fs::read_to_string(&list)?)?;
            println!("taken in and put up: {} plugins, {} skins", k.state.library.plugins.len(), k.state.library.skins.len());
        }
        Cmd::Unlist { which, config } => {
            let cfg = load(&config)?;
            let mut k = Keeper::new(cfg, &NoReview, &NoBuild)?;
            let name = k.unlist(&which)?;
            println!("took {name} off the list and put it up again: {} plugins, {} skins", k.state.library.plugins.len(), k.state.library.skins.len());
        }
        Cmd::Selftest { folder, config } => {
            let cfg = load(&config)?;
            let mut files = Vec::new();
            let mut stack = vec![folder.clone()];
            while let Some(d) = stack.pop() {
                for e in std::fs::read_dir(&d)? {
                    let p = e?.path();
                    if p.is_dir() {
                        if p.file_name().is_some_and(|n| n != "target") {
                            stack.push(p);
                        }
                    } else {
                        let rel = p.strip_prefix(&folder)?.to_string_lossy().replace('\\', "/");
                        files.push((rel, std::fs::read(&p)?));
                    }
                }
            }
            let sandbox = sandbox(&cfg);
            let wasm = pyr_keeper::build::Builder::build_addon(&sandbox, &files)?;
            println!("built in the sandbox: {} bytes of WebAssembly, importing nothing", wasm.len());
        }
        Cmd::Release { submission, config } => {
            let cfg = load(&config)?;
            let path = cfg.state_dir.join("state.json");
            let mut st: pyr_keeper::State = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
            let was = st.held.remove(&submission);
            std::fs::write(&path, serde_json::to_vec_pretty(&st)?)?;
            println!("{}", if was.is_some() { "it'll be looked at again" } else { "it wasn't held" });
        }
        Cmd::Run { config } | Cmd::Once { config } => {
            let once = matches!(Cli::parse().cmd, Cmd::Once { .. });
            let cfg = load(&config)?;
            let api_key = std::fs::read_to_string(&cfg.api_key_file).context("the Claude API key")?.trim().to_string();
            let model = if cfg.model.is_empty() {
                // (A key the API turns down: the owner hears of it, and it's tried again.)
                loop {
                    match Claude::newest_model(&api_key) {
                        Ok(m) => break m,
                        Err(e) => {
                            tracing::warn!("couldn't ask the Claude API for its models: {e:#}");
                            if let Some(t) = pyr_keeper::alert::trouble(&e)
                                && let Ok(store) = pyr_keeper::store::Store::open(&cfg.guild_db)
                            {
                                pyr_keeper::alert::raise(&store, &cfg.state_dir, t);
                            }
                            if once {
                                return Err(e);
                            }
                            std::thread::sleep(Duration::from_secs(300));
                        }
                    }
                }
            } else {
                cfg.model.clone()
            };
            tracing::info!("reviewing with {model}");
            let reviewer = Claude { api_key, model };
            let builder = sandbox(&cfg);
            let poll = Duration::from_secs(cfg.poll_secs.max(10));
            let mut k = Keeper::new(cfg, &reviewer, &builder)?;
            loop {
                match k.pass() {
                    Ok(n) if n > 0 => tracing::info!("decided {n}"),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("a pass went wrong: {e:#}"),
                }
                if once {
                    break;
                }
                std::thread::sleep(poll);
            }
        }
    }
    Ok(())
}
