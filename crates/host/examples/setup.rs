//! Drive the server setups from a terminal (used for testing them end to end).
//!
//!   setup ssh --host H [--port 22] [--user root] (--password P | --key FILE [--passphrase P])
//!             [--sudo-password P] [--name NAME] [--public-host H]
//!   setup catalog --token T
//!   setup rent --token T --region R --size S [--name NAME] --config DIR
//!   setup retry --droplet ID --ip IP --config DIR [--name NAME]
//!   setup delete --token T --droplet ID --config DIR
//!   setup save-files --to DIR
//!
//! The Linux server programs come from INGLE_VPS_DIR (or the usual places).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use ingle_host::digitalocean::{self, RentRequest};
use ingle_host::install::{self, InstallOptions};
use ingle_host::ssh::{Auth, Login};
use ingle_host::{Outcome, Payloads, Progress};

fn main() {
    tracing_subscriber::fmt().with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into())).init();
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    let mut opts: HashMap<String, String> = HashMap::new();
    while let Some(a) = args.next() {
        if let Some(k) = a.strip_prefix("--") {
            opts.insert(k.to_string(), args.next().unwrap_or_default());
        }
    }
    let get = |k: &str| opts.get(k).cloned();
    let need = |k: &str| get(k).unwrap_or_else(|| panic!("--{k} is required"));
    let payloads = Payloads::find();

    // Print steps and log lines as they happen.
    let printed = Arc::new(parking_lot_free::Counter::default());
    let progress_cell: Arc<std::sync::OnceLock<Progress>> = Arc::new(std::sync::OnceLock::new());
    let (p2, c2) = (progress_cell.clone(), printed.clone());
    let progress = Progress::new(Arc::new(move || {
        if let Some(p) = p2.get() {
            let t = p.snapshot();
            let (mut steps, mut logs) = c2.get();
            while steps < t.steps.len() {
                println!("STEP {}", t.steps[steps].0);
                steps += 1;
            }
            while logs < t.log.len() {
                println!("  | {}", t.log[logs]);
                logs += 1;
            }
            c2.set(steps, logs);
        }
    }));
    let _ = progress_cell.set(progress.clone());

    match cmd.as_str() {
        "ssh" => {
            let host = need("host");
            let auth = match (get("password"), get("key")) {
                (Some(p), _) => Auth::Password(p),
                (None, Some(k)) => Auth::KeyFile { path: PathBuf::from(k), passphrase: get("passphrase") },
                _ => panic!("--password or --key is required"),
            };
            let login = Login {
                host: host.clone(),
                port: get("port").map(|p| p.parse().expect("port")).unwrap_or(22),
                user: get("user").unwrap_or_else(|| "root".into()),
                auth,
                sudo_password: get("sudo-password"),
                host_key: get("host-key"),
            };
            let opts = InstallOptions {
                name: get("name").unwrap_or_default(),
                public_host: get("public-host").unwrap_or(host),
                port: ingle_host::DEFAULT_PORT,
                payloads,
                wait_for_boot: None,
                // A sealed server key opens with the backing-up owner's identity.key (--identity).
                restore: get("restore").map(|d| ingle_host::restore::Backup {
                    dir: PathBuf::from(d),
                    key: get("identity").and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| ingle_proto::b64::decode_array::<32>(t.trim())).map(|seed| ingle_proto::crypto::backup_key(&ingle_proto::crypto::Identity::from_seed(seed))),
                }),
            };
            install::setup_existing(login, opts, progress.clone(), get("config").map(PathBuf::from));
        }
        "catalog" => {
            match digitalocean::load_catalog(&need("token")) {
                Ok(c) => {
                    if let Some(a) = &c.account {
                        println!("ACCOUNT {} {}", a.email, a.status);
                    }
                    for p in &c.plans {
                        println!("PLAN {} ${:.2} {}", p.slug, p.price_monthly, p.summary());
                    }
                    if let Some(p) = c.default_plan() {
                        println!("DEFAULT {}", p.slug);
                        let regions: Vec<_> = c.regions_for(&p.slug).iter().map(|r| r.slug.clone()).collect();
                        println!("REGIONS {}", regions.join(" "));
                    }
                }
                Err(f) => {
                    println!("FAILED {}", f.error);
                    if let Some(h) = f.hint {
                        println!("HINT {h}");
                    }
                    std::process::exit(1);
                }
            }
            return;
        }
        "rent" => {
            let req = RentRequest {
                token: need("token"),
                name: get("name").unwrap_or_else(|| "My Ingle Server".into()),
                region: need("region"),
                size: need("size"),
                price_monthly: get("price").map(|p| p.parse().unwrap_or(0.0)).unwrap_or(6.0),
                size_label: String::new(),
                region_label: String::new(),
                restore: get("restore").map(PathBuf::from),
            };
            digitalocean::rent(req, PathBuf::from(need("config")), payloads, progress.clone());
        }
        "retry" => {
            let d = ingle_host::DropletRef {
                id: need("droplet").parse().expect("droplet id"),
                name: "ingle".into(),
                ip: Some(need("ip")),
                region: String::new(),
                size: String::new(),
            };
            digitalocean::retry_install(d, get("name").unwrap_or_default(), PathBuf::from(need("config")), payloads, progress.clone(), get("restore").map(PathBuf::from));
        }
        "delete" => {
            match digitalocean::delete(&need("token"), need("droplet").parse().expect("droplet id"), &PathBuf::from(need("config"))) {
                Ok(()) => println!("DELETED"),
                Err(f) => {
                    println!("FAILED {}", f.error);
                    std::process::exit(1);
                }
            }
            return;
        }
        "probe" => {
            match ingle_host::probe::reachable_blocking(&need("link"), 1, std::time::Duration::from_secs(4)) {
                Ok(()) => println!("REACHABLE"),
                Err(e) => println!("UNREACHABLE {e:#}"),
            }
            return;
        }
        "save-files" => {
            for f in install::save_manual_files(&payloads, &PathBuf::from(need("to"))).expect("saving") {
                println!("SAVED {}", f.display());
            }
            return;
        }
        _ => {
            eprintln!("usage: setup ssh|catalog|rent|retry|delete|save-files [--options]");
            std::process::exit(2);
        }
    }
    match progress.snapshot().outcome {
        Some(Outcome::Ready(r)) => {
            println!("READY {}", r.name);
            println!("HOST {}:{}", r.host, r.port);
            println!("OWNER {}", r.owner_link);
            println!("INVITE {}", r.invite_link);
            println!("REACHABLE {:?}", r.reachable);
            for w in &r.warnings {
                println!("WARNING {w}");
            }
            if let Some(d) = r.droplet {
                println!("DROPLET {} {}", d.id, d.ip.unwrap_or_default());
            }
        }
        Some(Outcome::Failed(f)) => {
            println!("FAILED {}", f.error);
            if let Some(h) = f.hint {
                println!("HINT {h}");
            }
            if let Some(d) = f.droplet {
                println!("DROPLET {} {}", d.id, d.ip.unwrap_or_default());
            }
            std::process::exit(1);
        }
        None => {
            println!("FAILED no outcome");
            std::process::exit(1);
        }
    }
}

mod parking_lot_free {
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct Counter(Mutex<(usize, usize)>);

    impl Counter {
        pub fn get(&self) -> (usize, usize) {
            *self.0.lock().unwrap()
        }
        pub fn set(&self, a: usize, b: usize) {
            *self.0.lock().unwrap() = (a, b);
        }
    }
}
