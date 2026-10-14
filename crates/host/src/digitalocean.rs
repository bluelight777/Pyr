//! Renting a server from DigitalOcean with the person's own account and API token.
//!
//! The person pays DigitalOcean directly; Ingle only uses the token to create one small
//! Ubuntu server ("droplet") that trusts Ingle's SSH key, then installs Ingle Server on it.
//! The token isn't saved anywhere.
//!
//! API reference: <https://docs.digitalocean.com/reference/api/>. `INGLE_DO_API` points
//! at another base URL (for tests).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::install::{self, InstallOptions, Payloads};
use crate::records::{self, HostedServer};
use crate::ssh::{Auth, Login};
use crate::{DropletRef, Failure, Outcome, Progress};

pub const SIGNUP_URL: &str = "https://cloud.digitalocean.com/registrations/new";
/// Where tokens are made ("Generate New Token").
pub const TOKENS_URL: &str = "https://cloud.digitalocean.com/account/api/tokens";
pub const DROPLETS_URL: &str = "https://cloud.digitalocean.com/droplets";
pub const PRICING_URL: &str = "https://www.digitalocean.com/pricing/droplets";
/// Ubuntu 24.04 LTS (supported until 2029).
pub const IMAGE: &str = "ubuntu-24-04-x64";
/// The plan picked unless the person chooses another: 1 GB of memory leaves room to spare.
pub const RECOMMENDED_SIZE: &str = "s-1vcpu-1gb";

fn base_url() -> String {
    std::env::var("INGLE_DO_API").unwrap_or_else(|_| "https://api.digitalocean.com".into()).trim_end_matches('/').to_string()
}

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: u16,
    pub id: String,
    pub message: String,
}

impl ApiError {
    /// A sentence for the person, and what to do about it.
    pub fn explain(&self) -> Failure {
        let msg = self.message.trim();
        let lower = msg.to_lowercase();
        match self.status {
            0 => Failure::new(format!("Couldn't reach DigitalOcean: {msg}")).hint("Check your internet connection and try again."),
            401 => Failure::new("DigitalOcean didn't accept the token.")
                .hint("Copy the whole token again (it starts with dop_v1_). DigitalOcean shows a token only once, so if it's lost, generate a new one."),
            403 => Failure::new("This token isn't allowed to do that.").hint(
                "Generate a new token with Custom Scopes: droplet (create, read, delete), ssh_key (create, read), regions (read), sizes (read) and account (read).",
            ),
            422 if lower.contains("limit") => Failure::new(format!("DigitalOcean: {msg}"))
                .hint("Delete a droplet you no longer use, or ask DigitalOcean support to raise your droplet limit."),
            422 if lower.contains("verif") || lower.contains("billing") || lower.contains("payment") => Failure::new(format!("DigitalOcean: {msg}"))
                .hint("Finish setting up your DigitalOcean account (confirm your email address and add a payment method), then try again."),
            429 => Failure::new("DigitalOcean says there were too many requests.").hint("Wait a minute, then try again."),
            s if s >= 500 => Failure::new(format!("DigitalOcean had a problem (error {s}): {msg}")).hint("Try again in a few minutes."),
            s => Failure::new(format!("DigitalOcean refused (error {s}): {msg}")),
        }
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DigitalOcean error {}: {}", self.status, self.message)
    }
}

impl std::error::Error for ApiError {}

pub struct Api {
    token: String,
    base: String,
    agent: ureq::Agent,
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct Account {
    pub email: String,
    pub status: String,
    pub status_message: String,
    pub email_verified: bool,
    pub droplet_limit: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone, Default, PartialEq)]
#[serde(default)]
pub struct Size {
    pub slug: String,
    pub memory: u32,
    pub vcpus: u32,
    pub disk: u32,
    /// Included outgoing traffic per month, in TB.
    pub transfer: f64,
    pub price_monthly: f64,
    pub regions: Vec<String>,
    pub available: bool,
    pub description: String,
}

impl Size {
    /// "1 GB" or "512 MB" (memory, which is how DigitalOcean names its plans).
    pub fn memory_label(&self) -> String {
        if self.memory >= 1024 { format!("{} GB", self.memory / 1024) } else { format!("{} MB", self.memory) }
    }

    /// "1 GB memory, 25 GB disk, 1 TB transfer"
    pub fn summary(&self) -> String {
        let mem = if self.memory >= 1024 { format!("{} GB", self.memory / 1024) } else { format!("{} MB", self.memory) };
        let cpu = if self.vcpus == 1 { "1 CPU".to_string() } else { format!("{} CPUs", self.vcpus) };
        let transfer = if self.transfer >= 1.0 { format!("{} TB", self.transfer) } else { format!("{} GB", (self.transfer * 1000.0).round()) };
        format!("{mem} memory, {cpu}, {} GB disk, {transfer} transfer", self.disk)
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Default, PartialEq)]
#[serde(default)]
pub struct Region {
    pub slug: String,
    pub name: String,
    pub available: bool,
    pub sizes: Vec<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct Droplet {
    pub id: u64,
    pub name: String,
    pub status: String,
    pub networks: Networks,
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct Networks {
    pub v4: Vec<Address>,
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct Address {
    pub ip_address: String,
    #[serde(rename = "type")]
    pub kind: String,
}

impl Droplet {
    pub fn public_ipv4(&self) -> Option<String> {
        self.networks.v4.iter().find(|a| a.kind == "public" && !a.ip_address.is_empty()).map(|a| a.ip_address.clone())
    }
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default)]
struct SshKey {
    id: u64,
    public_key: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Links {
    pages: Option<Pages>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Pages {
    next: Option<String>,
}

impl Api {
    pub fn new(token: &str) -> Api {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .user_agent(concat!("Ingle/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Api { token: token.trim().to_string(), base: base_url(), agent }
    }

    /// The full address for an API path. A full address (a "next page" link) is only used if
    /// it's on the API itself: the token is never sent anywhere else.
    fn url(&self, path: &str) -> String {
        if path.starts_with(&format!("{}/", self.base)) {
            path.to_string()
        } else if path.starts_with('/') {
            format!("{}{path}", self.base)
        } else {
            format!("{}/invalid", self.base)
        }
    }

    fn check(&self, resp: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Result<ureq::http::Response<ureq::Body>, ApiError> {
        let mut resp = resp.map_err(|e| ApiError { status: 0, id: "network".into(), message: e.to_string() })?;
        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(resp);
        }
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Body {
            id: String,
            message: String,
        }
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        let body: Body = serde_json::from_str(&text).unwrap_or_default();
        let message = if body.message.is_empty() { text.chars().take(200).collect() } else { body.message };
        Err(ApiError { status, id: body.id, message })
    }

    fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ApiError> {
        let resp = self.agent.get(self.url(path)).header("Authorization", format!("Bearer {}", self.token)).call();
        let mut resp = self.check(resp)?;
        resp.body_mut().read_json().map_err(|e| ApiError { status: 0, id: "decode".into(), message: format!("unexpected answer: {e}") })
    }

    fn post<T: serde::de::DeserializeOwned>(&self, path: &str, body: serde_json::Value) -> Result<T, ApiError> {
        let resp = self.agent.post(self.url(path)).header("Authorization", format!("Bearer {}", self.token)).send_json(body);
        let mut resp = self.check(resp)?;
        resp.body_mut().read_json().map_err(|e| ApiError { status: 0, id: "decode".into(), message: format!("unexpected answer: {e}") })
    }

    /// GETs every page of a list.
    fn list<T: serde::de::DeserializeOwned>(&self, path: &str, key: &str) -> Result<Vec<T>, ApiError> {
        let mut out = Vec::new();
        let mut next = Some(format!("{path}?per_page=200"));
        let mut pages = 0;
        while let Some(p) = next.take() {
            let mut v: serde_json::Value = self.get(&p)?;
            let items = v.get_mut(key).map(serde_json::Value::take).unwrap_or_default();
            let items: Vec<T> = serde_json::from_value(items)
                .map_err(|e| ApiError { status: 0, id: "decode".into(), message: format!("unexpected answer: {e}") })?;
            out.extend(items);
            let links: Links = v.get("links").cloned().and_then(|l| serde_json::from_value(l).ok()).unwrap_or_default();
            // Only follow "next" links that stay on the API.
            next = links.pages.and_then(|p| p.next).filter(|n| n.starts_with(&format!("{}/", self.base)));
            pages += 1;
            if pages > 20 {
                break;
            }
        }
        Ok(out)
    }

    pub fn account(&self) -> Result<Account, ApiError> {
        #[derive(Deserialize)]
        struct R {
            account: Account,
        }
        self.get::<R>("/v2/account").map(|r| r.account)
    }

    pub fn sizes(&self) -> Result<Vec<Size>, ApiError> {
        self.list("/v2/sizes", "sizes")
    }

    pub fn regions(&self) -> Result<Vec<Region>, ApiError> {
        self.list("/v2/regions", "regions")
    }

    /// Adds Ingle's public key to the account (or finds it if it's there already) and
    /// returns its id.
    pub fn ensure_ssh_key(&self, public_openssh: &str, name: &str) -> Result<u64, ApiError> {
        #[derive(Deserialize)]
        struct R {
            ssh_key: SshKey,
        }
        let blob = |k: &str| k.split_whitespace().nth(1).unwrap_or("").to_string();
        let mine = blob(public_openssh);
        match self.post::<R>("/v2/account/keys", json!({ "name": name, "public_key": public_openssh })) {
            Ok(r) => Ok(r.ssh_key.id),
            // Already on the account (DigitalOcean refuses duplicates): look it up.
            Err(e) if e.status == 422 => {
                let keys: Vec<SshKey> = self.list("/v2/account/keys", "ssh_keys")?;
                keys.into_iter().find(|k| blob(&k.public_key) == mine).map(|k| k.id).ok_or(e)
            }
            Err(e) => Err(e),
        }
    }

    /// `user_data` is the cloud-init setup the server runs on first boot (here: the SSH host
    /// key Ingle made for it).
    pub fn create_droplet(&self, name: &str, region: &str, size: &str, ssh_key: u64, user_data: &str) -> Result<Droplet, ApiError> {
        #[derive(Deserialize)]
        struct R {
            droplet: Droplet,
        }
        let body = json!({
            "name": name,
            "region": region,
            "size": size,
            "image": IMAGE,
            "ssh_keys": [ssh_key],
            "backups": false,
            "ipv6": false,
            "monitoring": false,
            "user_data": user_data,
        });
        self.post::<R>("/v2/droplets", body).map(|r| r.droplet)
    }

    pub fn droplet(&self, id: u64) -> Result<Droplet, ApiError> {
        #[derive(Deserialize)]
        struct R {
            droplet: Droplet,
        }
        self.get::<R>(&format!("/v2/droplets/{id}")).map(|r| r.droplet)
    }

    pub fn delete_droplet(&self, id: u64) -> Result<(), ApiError> {
        let resp = self.agent.delete(self.url(&format!("/v2/droplets/{id}"))).header("Authorization", format!("Bearer {}", self.token)).call();
        match self.check(resp) {
            Ok(_) => Ok(()),
            Err(e) if e.status == 404 => Ok(()), // already gone
            Err(e) => Err(e),
        }
    }
}

/// What the person picks from: plans with prices, and places.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    pub account: Option<Account>,
    /// Cheapest shared-CPU plans first.
    pub plans: Vec<Size>,
    pub regions: Vec<Region>,
}

impl Catalog {
    /// Regions where `size` can be created, in the order DigitalOcean lists them.
    pub fn regions_for(&self, size: &str) -> Vec<&Region> {
        let plan = self.plans.iter().find(|p| p.slug == size);
        self.regions
            .iter()
            .filter(|r| r.available && plan.map(|p| p.regions.contains(&r.slug)).unwrap_or(true) && (r.sizes.is_empty() || r.sizes.iter().any(|s| s == size)))
            .collect()
    }

    pub fn default_plan(&self) -> Option<&Size> {
        self.plans.iter().find(|p| p.slug == RECOMMENDED_SIZE).or(self.plans.first())
    }
}

/// Is this a plain "Basic" plan (s-1vcpu-1gb, s-1vcpu-512mb-10gb…), not a premium variant?
fn is_basic(slug: &str) -> bool {
    let Some(rest) = slug.strip_prefix("s-") else { return false };
    let parts: Vec<&str> = rest.split('-').collect();
    let cpu = parts.first().map(|p| p.ends_with("vcpu") && p.trim_end_matches("vcpu").parse::<u32>().is_ok()).unwrap_or(false);
    let mem = parts.get(1).map(|p| p.ends_with("gb") || p.ends_with("mb")).unwrap_or(false);
    let extra_ok = parts.len() == 2 || (parts.len() == 3 && parts[2].ends_with("gb"));
    cpu && mem && extra_ok
}

/// Checks the token and loads plans and regions.
pub fn load_catalog(token: &str) -> Result<Catalog, Failure> {
    let token = token.trim();
    if token.is_empty() {
        return Err(Failure::new("Paste your DigitalOcean token first."));
    }
    if token.chars().any(char::is_whitespace) {
        return Err(Failure::new("That doesn't look like a token: it has spaces in it.").hint("Copy just the token (it starts with dop_v1_)."));
    }
    let api = Api::new(token);
    let mut plans: Vec<Size> = api.sizes().map_err(|e| e.explain())?.into_iter().filter(|s| s.available && is_basic(&s.slug)).collect();
    plans.sort_by(|a, b| a.price_monthly.total_cmp(&b.price_monthly).then(a.slug.cmp(&b.slug)));
    plans.dedup_by(|a, b| a.slug == b.slug);
    plans.truncate(4);
    if plans.is_empty() {
        return Err(Failure::new("DigitalOcean didn't offer any small servers to this account."));
    }
    let regions = api.regions().map_err(|e| e.explain())?;
    // Needs the account:read scope, which a custom token may not have: that's fine.
    let account = api.account().ok();
    if let Some(a) = &account {
        if a.status == "locked" {
            return Err(Failure::new(format!("Your DigitalOcean account is locked: {}", a.status_message))
                .hint("Sort it out with DigitalOcean (often a payment method is missing), then try again."));
        }
    }
    Ok(Catalog { account, plans, regions })
}

/// What to rent.
#[derive(Clone)]
pub struct RentRequest {
    pub token: String,
    /// Name of the community (the droplet is named after it).
    pub name: String,
    pub region: String,
    pub size: String,
    pub price_monthly: f64,
    /// For showing: "1 GB" and "New York 3" (the slugs if empty).
    pub size_label: String,
    pub region_label: String,
    /// Put back a server from this backup instead of starting a new one.
    pub restore: Option<crate::restore::Backup>,
}

impl std::fmt::Debug for RentRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never the token.
        f.debug_struct("RentRequest").field("name", &self.name).field("region", &self.region).field("size", &self.size).finish_non_exhaustive()
    }
}

/// A droplet name from a community name: "Game Night!" -> "ingle-game-night".
pub fn droplet_name(community: &str) -> String {
    let mut s = String::from("ingle-");
    let mut dash = true;
    for c in community.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            s.push(c);
            dash = false;
        } else if !dash {
            s.push('-');
            dash = true;
        }
        if s.len() >= 40 {
            break;
        }
    }
    let s = s.trim_end_matches('-').to_string();
    if s == "ingle" { "ingle-server".into() } else { s }
}

fn install_options(name: &str, ip: &str, payloads: &Payloads, restore: Option<crate::restore::Backup>) -> InstallOptions {
    InstallOptions {
        name: name.to_string(),
        public_host: ip.to_string(),
        port: crate::DEFAULT_PORT,
        payloads: payloads.clone(),
        wait_for_boot: Some(Duration::from_secs(240)),
        restore,
    }
}

/// Rents a server and installs Ingle on it. Runs on a worker thread; the result goes to
/// `progress`.
pub fn rent(req: RentRequest, config_dir: PathBuf, payloads: Payloads, progress: Progress) {
    let outcome = rent_inner(&req, &config_dir, &payloads, &progress);
    progress.finish(match outcome {
        Ok(r) => Outcome::Ready(r),
        Err(f) => Outcome::Failed(f),
    });
}

fn rent_inner(req: &RentRequest, config_dir: &Path, payloads: &Payloads, progress: &Progress) -> Result<crate::Ready, Failure> {
    // Fail before spending money if the server program isn't there.
    payloads.for_machine("x86_64").map_err(Failure::from)?;
    let api = Api::new(&req.token);

    progress.step("Adding Ingle's SSH key to your DigitalOcean account");
    let key = crate::keys::load_or_create(config_dir).map_err(Failure::from)?;
    let key_id = api.ensure_ssh_key(&key.public_openssh, "Ingle").map_err(|e| e.explain())?;
    if progress.stop_requested() {
        return Err(Failure::new("Stopped before creating a server."));
    }

    let name = droplet_name(&req.name);
    let or = |label: &str, slug: &str| if label.is_empty() { slug.to_string() } else { label.to_string() };
    progress.step(format!(
        "Creating a {} server in {} (${:.2}/month)",
        or(&req.size_label, &req.size),
        or(&req.region_label, &req.region),
        req.price_monthly
    ));
    // The server's own SSH identity is made here and handed to it at creation, so the first
    // connection can already tell the real server from anyone in between.
    let (host_private, host_public, host_fp) = crate::keys::new_host_key().map_err(Failure::from)?;
    let droplet =
        api.create_droplet(&name, &req.region, &req.size, key_id, &host_key_cloud_config(&host_private, &host_public)).map_err(|e| e.explain())?;
    let mut dref = DropletRef { id: droplet.id, name: name.clone(), ip: None, region: req.region.clone(), size: req.size.clone() };
    progress.log(format!("Droplet {} (id {}) created.", name, droplet.id));
    let mut record = HostedServer {
        name: req.name.clone(),
        port: crate::DEFAULT_PORT,
        provider: "digitalocean".into(),
        ssh_user: "root".into(),
        ssh_port: 22,
        droplet_id: Some(droplet.id),
        region: Some(req.region.clone()),
        size: Some(req.size.clone()),
        host_key: Some(host_fp.clone()),
        created_ms: ingle_proto::now_ms(),
        ..Default::default()
    };
    let _ = records::save(config_dir, &record);
    let with_droplet = |mut f: Failure, d: &DropletRef| {
        f.droplet = Some(d.clone());
        f
    };

    progress.step("Waiting for the server to start");
    let started = Instant::now();
    let ip = loop {
        match api.droplet(droplet.id) {
            Ok(d) if d.status == "active" => {
                if let Some(ip) = d.public_ipv4() {
                    break ip;
                }
            }
            Ok(_) => {}
            Err(e) if e.status == 0 || e.status == 429 || e.status >= 500 => progress.log(format!("({e}; still waiting)")),
            Err(e) => return Err(with_droplet(e.explain(), &dref)),
        }
        if started.elapsed() > Duration::from_secs(300) {
            return Err(with_droplet(
                Failure::new("The server took too long to start.").hint("Try the setup again in a minute, or delete the server and create a new one."),
                &dref,
            ));
        }
        std::thread::sleep(Duration::from_secs(5));
    };
    dref.ip = Some(ip.clone());
    record.host = ip.clone();
    let _ = records::save(config_dir, &record);
    progress.log(format!("The server's address is {ip}."));

    let ready = install_on(&dref, &req.name, &key.key, Some(host_fp), payloads, progress, req.restore.clone()).map_err(|f| with_droplet(f, &dref))?;
    record.installed = true;
    record.name = ready.name.clone();
    record.invite_link = Some(ready.invite_link.clone());
    let _ = records::save(config_dir, &record);
    Ok(ready)
}

/// cloud-init: replace the image's host keys with ours (and make no others), so the server's
/// SSH identity is the one we know.
fn host_key_cloud_config(private: &str, public: &str) -> String {
    let indented: String = private.lines().map(|l| format!("    {l}\n")).collect();
    format!("#cloud-config\nssh_deletekeys: true\nssh_genkeytypes: []\nssh_keys:\n  ed25519_private: |\n{indented}  ed25519_public: {public}\n")
}

fn install_on(
    d: &DropletRef,
    name: &str,
    key: &std::sync::Arc<russh::keys::PrivateKey>,
    host_key: Option<String>,
    payloads: &Payloads,
    progress: &Progress,
    restore: Option<crate::restore::Backup>,
) -> Result<crate::Ready, Failure> {
    let ip = d.ip.clone().ok_or_else(|| Failure::new("The server has no address yet."))?;
    let login = Login { host: ip.clone(), port: 22, user: "root".into(), auth: Auth::Key(key.clone()), sudo_password: None, host_key };
    let opts = install_options(name, &ip, payloads, restore);
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| Failure::new(e.to_string()))?;
    let installed = rt.block_on(install::install(&login, &opts, progress));
    drop(rt);
    let installed = installed?;
    Ok(install::finish(&login, &opts, installed, Some(d.clone()), progress))
}

/// Runs the install again on a droplet that was created earlier (after a failure).
pub fn retry_install(
    d: DropletRef,
    name: String,
    config_dir: PathBuf,
    payloads: Payloads,
    progress: Progress,
    restore: Option<crate::restore::Backup>,
) {
    let outcome = (|| {
        let key = crate::keys::load_or_create(&config_dir).map_err(Failure::from)?;
        let mut record = records::load(&config_dir)
            .servers
            .into_iter()
            .find(|s| s.droplet_id == Some(d.id))
            .unwrap_or_else(|| HostedServer { provider: "digitalocean".into(), droplet_id: Some(d.id), ..Default::default() });
        let ready = install_on(&d, &name, &key.key, record.host_key.clone(), &payloads, &progress, restore).map_err(|mut f| {
            f.droplet = Some(d.clone());
            f
        })?;
        record.host = d.ip.clone().unwrap_or_default();
        record.port = crate::DEFAULT_PORT;
        record.name = ready.name.clone();
        record.installed = true;
        record.invite_link = Some(ready.invite_link.clone());
        let _ = records::save(&config_dir, &record);
        Ok::<_, Failure>(ready)
    })();
    progress.finish(match outcome {
        Ok(r) => Outcome::Ready(r),
        Err(f) => Outcome::Failed(f),
    });
}

/// Deletes a rented server (stops the billing).
pub fn delete(token: &str, droplet_id: u64, config_dir: &Path) -> Result<(), Failure> {
    Api::new(token).delete_droplet(droplet_id).map_err(|e| e.explain())?;
    let _ = records::remove_droplet(config_dir, droplet_id);
    Ok(())
}

/// DigitalOcean regions to suggest first for a UTC offset (in minutes), nearest first.
pub fn regions_near(utc_offset_minutes: i32) -> &'static [&'static str] {
    match utc_offset_minutes / 60 {
        -12..=-7 => &["sfo3", "sfo2", "tor1", "nyc3"],
        -6..=-3 => &["nyc3", "nyc1", "tor1", "sfo3"],
        -2..=0 => &["lon1", "ams3", "fra1", "nyc3"],
        1..=3 => &["fra1", "ams3", "lon1"],
        4..=6 => &["blr1", "fra1", "sgp1"],
        7..=8 => &["sgp1", "syd1", "blr1"],
        _ => &["syd1", "sgp1", "sfo3"],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_plans_are_recognised() {
        assert!(is_basic("s-1vcpu-512mb-10gb"));
        assert!(is_basic("s-1vcpu-1gb"));
        assert!(is_basic("s-2vcpu-4gb"));
        assert!(!is_basic("s-1vcpu-1gb-amd"));
        assert!(!is_basic("s-1vcpu-1gb-intel"));
        assert!(!is_basic("c-2"));
        assert!(!is_basic("g-2vcpu-8gb"));
    }

    #[test]
    fn droplet_names_are_hostnames() {
        assert_eq!(droplet_name("Game Night!"), "ingle-game-night");
        assert_eq!(droplet_name("   "), "ingle-server");
        assert_eq!(droplet_name("Blue's Ingle Server"), "ingle-blue-s-ingle-server");
        assert!(droplet_name(&"x".repeat(100)).len() <= 40);
    }

    #[test]
    fn plan_summary_reads_well() {
        let s = Size { slug: "s-1vcpu-1gb".into(), memory: 1024, vcpus: 1, disk: 25, transfer: 1.0, price_monthly: 6.0, ..Default::default() };
        assert_eq!(s.summary(), "1 GB memory, 1 CPU, 25 GB disk, 1 TB transfer");
        let s = Size { memory: 512, transfer: 0.5, disk: 10, vcpus: 1, ..Default::default() };
        assert_eq!(s.summary(), "512 MB memory, 1 CPU, 10 GB disk, 500 GB transfer");
    }
}
