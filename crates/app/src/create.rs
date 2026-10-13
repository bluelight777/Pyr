//! "Create a server": rent one from DigitalOcean, or set up a Linux server you already have.
//! The work itself is in `ingle-host`; this is the wizard around it.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, RichText, Sense, Stroke, vec2};
use ingle_engine::state::AppState;
use ingle_host::digitalocean::{self, Catalog, RentRequest};
use ingle_host::install::{self, InstallOptions};
use ingle_host::ssh::{Auth, Login};
use ingle_host::{Failure, Outcome, Payloads, Progress, StepState};
use parking_lot::Mutex;

use crate::app::{Act, IngleApp};
use crate::dialogs::{field_label, input, title};
use crate::{icons, theme};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Choose,
    Rent,
    Own,
    Manual,
    Working,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Rent,
    Own,
}

type Job<T> = Arc<Mutex<Option<T>>>;

fn spawn_job<T: Send + 'static>(ctx: &egui::Context, f: impl FnOnce() -> T + Send + 'static) -> Job<T> {
    let job: Job<T> = Arc::new(Mutex::new(None));
    let (j, ctx) = (job.clone(), ctx.clone());
    let _ = std::thread::Builder::new().name("ingle-setup".into()).spawn(move || {
        let r = f();
        *j.lock() = Some(r);
        ctx.request_repaint();
    });
    job
}

fn take<T>(job: &mut Option<Job<T>>) -> Option<T> {
    let done = job.as_ref()?.lock().take();
    if done.is_some() {
        *job = None;
    }
    done
}

pub struct CreateState {
    pub page: Page,
    kind: Kind,
    name: String,
    // Renting
    token: String,
    catalog: Option<Catalog>,
    catalog_job: Option<Job<Result<Catalog, Failure>>>,
    catalog_error: Option<Failure>,
    plan: String,
    region: String,
    // A server you have
    host: String,
    ssh_port: String,
    user: String,
    use_key: bool,
    password: String,
    key_path: String,
    passphrase: String,
    sudo_password: String,
    form_error: Option<String>,
    saved_files: Option<Result<PathBuf, String>>,
    // Working / done
    progress: Option<Progress>,
    price: f64,
    joined: bool,
    pub join_error: Option<String>,
    show_log: bool,
    delete_job: Option<Job<Result<(), Failure>>>,
    deleted: Option<Result<(), Failure>>,
    probe_job: Option<Job<bool>>,
    reachable_now: Option<bool>,
    copied: bool,
    /// Put back a server from this backup instead of starting a new one.
    restore: Option<(std::path::PathBuf, ingle_engine::backup::BackupInfo)>,
    restore_error: Option<String>,
}

impl CreateState {
    pub fn new(my_name: &str) -> CreateState {
        let name = if my_name.trim().is_empty() || my_name == "Friend" {
            "My Ingle Server".to_string()
        } else {
            format!("{}'s Server", my_name.trim())
        };
        CreateState {
            page: Page::Choose,
            kind: Kind::Rent,
            name,
            token: String::new(),
            catalog: None,
            catalog_job: None,
            catalog_error: None,
            plan: String::new(),
            region: String::new(),
            host: String::new(),
            ssh_port: "22".into(),
            user: "root".into(),
            use_key: false,
            password: String::new(),
            key_path: ingle_host::keys::default_user_key().map(|p| p.display().to_string()).unwrap_or_default(),
            passphrase: String::new(),
            sudo_password: String::new(),
            form_error: None,
            saved_files: None,
            progress: None,
            price: 0.0,
            joined: false,
            join_error: None,
            show_log: false,
            delete_job: None,
            deleted: None,
            probe_job: None,
            reachable_now: None,
            copied: false,
            restore: None,
            restore_error: None,
        }
    }

    /// True while setup work is running (the dialog can't be closed then).
    /// A backup folder was chosen to restore from (see `images::FolderFor::Restore`).
    pub fn set_restore(&mut self, dir: std::path::PathBuf) {
        match ingle_engine::backup::info(&dir).filter(|_| dir.join("database").join("latest.db").is_file()) {
            Some(info) => {
                self.name = info.name.clone();
                self.restore = Some((dir, info));
                self.restore_error = None;
            }
            None => self.restore_error = Some("That folder isn't an Ingle server backup (it needs server.json and database/latest.db).".into()),
        }
    }

    pub fn busy(&self) -> bool {
        self.progress.as_ref().map(|p| !p.is_finished()).unwrap_or(false) || self.delete_job.is_some()
    }

    fn new_progress(&mut self, ctx: &egui::Context) -> Progress {
        let ctx = ctx.clone();
        let p = Progress::new(Arc::new(move || ctx.request_repaint()));
        self.progress = Some(p.clone());
        self.joined = false;
        self.join_error = None;
        self.show_log = false;
        self.deleted = None;
        self.reachable_now = None;
        self.copied = false;
        self.page = Page::Working;
        p
    }
}

fn link_button(ui: &mut egui::Ui, text: &str, url: &str) {
    if ui.add(egui::Button::new(RichText::new(format!("{text}  ↗")).color(theme::LINK))).on_hover_text(url).clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
}

fn dim(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).size(12.5).color(theme::DIM));
}

fn muted(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(RichText::new(text.into()).color(theme::MUTED));
}

fn step_heading(ui: &mut egui::Ui, n: u32, text: &str) {
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 11.0, theme::ELEV2);
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, n.to_string(), theme::bold_font(12.0), theme::EMBER2);
        ui.label(RichText::new(text).font(theme::bold_font(14.5)).color(theme::TEXT));
    });
}

/// A copyable block of text (a command or a link).
fn code_box(ui: &mut egui::Ui, text: &str, acts: &mut Vec<Act>) {
    egui::Frame::new().fill(theme::CODE_BG).corner_radius(6).inner_margin(10).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(RichText::new(text).font(theme::mono(12.0)).color(theme::TEXT)).wrap().selectable(true));
    });
    if ui.small_button("Copy").clicked() {
        acts.push(Act::Copy(text.to_string()));
    }
}

/// A big clickable choice with an icon, a title and a description.
pub(crate) fn choice_card(ui: &mut egui::Ui, id: &str, badge: Option<&str>, heading: &str, body: &str, icon: impl FnOnce(&egui::Painter, egui::Rect, Color32)) -> bool {
    let width = ui.available_width();
    let bg = ui.painter().add(egui::Shape::Noop);
    let inner = egui::Frame::new().inner_margin(egui::Margin::same(14)).show(ui, |ui| {
        ui.set_width(width - 28.0);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(10), theme::SIDE);
            icon(ui.painter(), r.shrink(7.0), theme::EMBER);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(heading).font(theme::bold_font(15.0)).color(theme::TEXT));
                    if let Some(b) = badge {
                        egui::Frame::new().fill(theme::EMBER).corner_radius(4).inner_margin(egui::Margin::symmetric(5, 1)).show(ui, |ui| {
                            ui.label(RichText::new(b).font(theme::bold_font(10.5)).color(Color32::from_rgb(0x1b, 0x14, 0x11)));
                        });
                    }
                });
                ui.add(egui::Label::new(RichText::new(body).color(theme::MUTED)).wrap());
            });
        });
    });
    let rect = inner.response.rect;
    let resp = ui.interact(rect, ui.id().with(id), Sense::click());
    let (fill, stroke) = if resp.hovered() { (theme::ELEV2, theme::EMBER) } else { (theme::ELEV, theme::LINE) };
    ui.painter().set(bg, egui::Shape::rect_filled(rect, CornerRadius::same(10), fill));
    ui.painter().rect_stroke(rect, CornerRadius::same(10), Stroke::new(1.0f32, stroke), egui::StrokeKind::Inside);
    ui.add_space(8.0);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn error_box(ui: &mut egui::Ui, f: &Failure) {
    egui::Frame::new()
        .fill(Color32::from_rgb(0x3a, 0x1c, 0x1c))
        .stroke(Stroke::new(1.0f32, theme::LIVE))
        .corner_radius(8)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(&f.error).color(theme::TEXT));
            if let Some(h) = &f.hint {
                ui.label(RichText::new(h).size(12.5).color(theme::MUTED));
            }
        });
}

const FIREWALL_HELP: &[(&str, &str)] = &[
    ("Amazon Lightsail", "the instance's Networking tab: add an IPv4 firewall rule, Custom UDP 7766"),
    ("Amazon EC2", "the instance's security group: add an inbound rule, Custom UDP, port 7766, source 0.0.0.0/0"),
    ("Google Cloud", "VPC network > Firewall: create an ingress rule for UDP 7766 from 0.0.0.0/0"),
    ("Microsoft Azure", "the VM's Networking page: add an inbound port rule for UDP 7766"),
    ("Oracle Cloud", "the subnet's security list: add an ingress rule, UDP, destination port 7766, source 0.0.0.0/0"),
    ("DigitalOcean, Hetzner, Vultr, Linode", "there's no firewall unless you added one; if you did, allow inbound UDP 7766 in it"),
];

fn firewall_help(ui: &mut egui::Ui) {
    muted(ui, "Calls and chat use UDP port 7766. Ingle opens it in the server's own firewall (ufw, firewalld or iptables), but many providers also have a firewall in their website that only you can change. If yours does, let in UDP port 7766 from anywhere (and SSH, TCP port 22, for the setup):");
    ui.add_space(4.0);
    for (who, what) in FIREWALL_HELP {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(RichText::new(format!("{who}: ")).size(12.5).color(theme::TEXT));
            ui.label(RichText::new(*what).size(12.5).color(theme::MUTED));
        });
    }
}

impl IngleApp {
    pub fn create_dialog(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>, close: &mut bool) {
        ui.set_width(580.0);
        let config_dir = self.engine.identity_path().parent().map(|p| p.to_path_buf()).unwrap_or_default();
        let Some(c) = self.ui.create.as_mut() else {
            *close = true;
            return;
        };
        // Finished jobs.
        if let Some(r) = take(&mut c.catalog_job) {
            match r {
                Ok(cat) => {
                    c.plan = cat.default_plan().map(|p| p.slug.clone()).unwrap_or_default();
                    let near = digitalocean::regions_near(chrono::Local::now().offset().local_minus_utc() / 60);
                    let regions = cat.regions_for(&c.plan);
                    c.region = near
                        .iter()
                        .find(|s| regions.iter().any(|r| &r.slug == *s))
                        .map(|s| s.to_string())
                        .or_else(|| regions.first().map(|r| r.slug.clone()))
                        .unwrap_or_default();
                    c.catalog = Some(cat);
                    c.catalog_error = None;
                }
                Err(f) => c.catalog_error = Some(f),
            }
        }
        if let Some(r) = take(&mut c.delete_job) {
            c.deleted = Some(r);
        }
        if let Some(r) = take(&mut c.probe_job) {
            c.reachable_now = Some(r);
        }
        // Join the new server as its owner as soon as it's ready.
        if let Some(Outcome::Ready(r)) = c.progress.as_ref().and_then(|p| p.snapshot().outcome) {
            if !c.joined {
                c.joined = true;
                acts.push(Act::JoinOwned(r.owner_link.clone()));
            }
        }

        // The modal is as tall as its contents were last frame, so a scroll area inside it
        // has to ask for its full height or it never grows.
        let screen_h = ui.ctx().content_rect().height();
        let max_h = (screen_h - (screen_h * 0.07).clamp(16.0, 80.0) * 2.0 - 60.0).max(300.0);
        egui::ScrollArea::vertical().max_height(max_h).min_scrolled_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
            // Room for the scroll bar, so it never covers text boxes.
            egui::Frame::new().inner_margin(egui::Margin { right: 12, ..Default::default() }).show(ui, |ui| match c.page {
                Page::Choose => choose_page(c, ui, close),
                Page::Rent => rent_page(c, ui, &config_dir),
                Page::Own => own_page(c, ui, &config_dir),
                Page::Manual => manual_page(c, ui, acts, close),
                Page::Working => working_page(c, ui, st, acts, close, &config_dir),
            });
        });
    }
}

fn choose_page(c: &mut CreateState, ui: &mut egui::Ui, close: &mut bool) {
    title(ui, "Create a server");
    muted(ui, "Your server keeps your channels and messages and connects your calls. It stays on, so friends can drop in any time, even when you're offline.");
    match &c.restore {
        None => {
            field_label(ui, "Server name");
            input(ui, &mut c.name, "My Ingle Server");
        }
        Some((dir, info)) => {
            field_label(ui, "Restoring from a backup");
            let when = chrono::DateTime::from_timestamp_millis(info.last_backup)
                .map(|t| t.with_timezone(&chrono::Local).format("%b %-d, %Y at %-I:%M %p").to_string())
                .unwrap_or_default();
            muted(ui, format!("{} — backed up {when}, {} files. It comes back with all its messages, members and roles, as the same server: everyone's app recognises it. Send everyone the new invite link afterwards if its address changes.", info.name, info.files));
            dim(ui, dir.display().to_string());
        }
    }
    ui.horizontal(|ui| {
        if ui.button(if c.restore.is_some() { "Choose another backup…" } else { "Restore from a backup…" }).clicked() {
            crate::images::pick_folder_async(crate::images::FolderFor::Restore, "Choose the server's backup folder", ui.ctx());
        }
        if c.restore.is_some() && ui.button("Start a new server instead").clicked() {
            c.restore = None;
        }
    });
    if let Some(e) = &c.restore_error {
        ui.label(egui::RichText::new(e).color(theme::LIVE));
    }
    ui.add_space(12.0);
    if choice_card(
        ui,
        "rent",
        Some("RECOMMENDED"),
        "Rent a server",
        "From DigitalOcean, $4–6 a month. Ingle creates it and sets it up for you in a few minutes. You pay DigitalOcean directly and can delete it any time.",
        icons::cloud,
    ) {
        c.kind = Kind::Rent;
        c.page = Page::Rent;
    }
    if choice_card(
        ui,
        "own",
        None,
        "Use a server I already have",
        "Any Linux VPS from any provider (Ubuntu, Debian, Fedora…). Ingle sets it up over SSH, or shows you how to do it yourself.",
        icons::server,
    ) {
        c.kind = Kind::Own;
        c.page = Page::Own;
    }
    ui.add_space(2.0);
    let how = if cfg!(windows) { "start Ingle Server from the Start menu" } else { "run ingle-server in a terminal" };
    dim(ui, format!("Hosting on this PC instead? Close this and {how}: it shows links to paste into Join a server. Friends outside your home need UDP port 7766 forwarded on your router, and your PC must stay on."));
    ui.add_space(12.0);
    if ui.button("Cancel").clicked() {
        *close = true;
    }
}

fn rent_page(c: &mut CreateState, ui: &mut egui::Ui, config_dir: &std::path::Path) {
    title(ui, "Rent a server from DigitalOcean");
    muted(ui, "DigitalOcean is a large, dependable cloud provider. A small server costs $4–6 a month, charged only for the time it exists, and you can delete it whenever you like.");

    if let Some(cat) = &c.catalog {
        ui.add_space(10.0);
        let mut change = false;
        ui.horizontal_wrapped(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            icons::done(ui.painter(), r, theme::ONLINE);
            let who = cat.account.as_ref().map(|a| format!(" ({})", a.email)).unwrap_or_default();
            ui.label(RichText::new(format!("Connected to your DigitalOcean account{who}")).color(theme::TEXT));
            if ui.small_button("Use another token").clicked() {
                change = true;
            }
        });
        if change {
            c.catalog = None;
            c.token.clear();
        }
    }
    if c.catalog.is_none() {
        token_steps(c, ui);
    }

    let mut create = false;
    if let Some(cat) = &c.catalog {
        ui.add_space(10.0);
        ui.label(RichText::new("Pick a size and a place").font(theme::bold_font(14.5)).color(theme::TEXT));
        let cheapest = cat.plans.first().map(|p| p.slug.clone());
        for p in &cat.plans {
            let selected = c.plan == p.slug;
            let tag = if p.slug == digitalocean::RECOMMENDED_SIZE {
                "  (recommended)"
            } else if Some(&p.slug) == cheapest.as_ref() {
                "  (fine for a few friends)"
            } else {
                ""
            };
            let text = format!("${:.0}/month — {}{tag}", p.price_monthly, p.summary());
            if ui.add(egui::Button::selectable(selected, text).min_size(vec2(ui.available_width(), 30.0))).clicked() {
                c.plan = p.slug.clone();
            }
        }
        let regions = cat.regions_for(&c.plan);
        if !regions.iter().any(|r| r.slug == c.region) {
            c.region = regions.first().map(|r| r.slug.clone()).unwrap_or_default();
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Place").color(theme::MUTED));
            let current = regions.iter().find(|r| r.slug == c.region).map(|r| r.name.clone()).unwrap_or_else(|| c.region.clone());
            egui::ComboBox::from_id_salt("do-region").selected_text(current).width(240.0).show_ui(ui, |ui| {
                for r in &regions {
                    ui.selectable_value(&mut c.region, r.slug.clone(), &r.name);
                }
            });
        });
        dim(ui, "Pick the place closest to you and your friends: it keeps voice delay low.");
        ui.add_space(6.0);
        dim(ui, "Ingle uses the token only now, to create the server, and doesn't save it.");
        ui.add_space(10.0);
        let price = cat.plans.iter().find(|p| p.slug == c.plan).map(|p| p.price_monthly).unwrap_or(0.0);
        let ok = !c.plan.is_empty() && !c.region.is_empty() && !c.name.trim().is_empty();
        if ui.add_enabled(ok, theme::primary_button(&format!("Create my server (${price:.2}/month)")).min_size(vec2(240.0, 36.0))).clicked() {
            c.price = price;
            create = true;
        }
    }
    if create {
        let cat = c.catalog.as_ref();
        let size_label = cat.and_then(|k| k.plans.iter().find(|p| p.slug == c.plan)).map(|p| p.memory_label()).unwrap_or_default();
        let region_label = cat.and_then(|k| k.regions.iter().find(|r| r.slug == c.region)).map(|r| r.name.clone()).unwrap_or_default();
        let req = RentRequest {
            token: c.token.trim().to_string(),
            name: c.name.trim().to_string(),
            region: c.region.clone(),
            size: c.plan.clone(),
            price_monthly: c.price,
            size_label,
            region_label,
            restore: c.restore.as_ref().map(|(d, _)| d.clone()),
        };
        let progress = c.new_progress(ui.ctx());
        let dir = config_dir.to_path_buf();
        let _ = std::thread::Builder::new()
            .name("ingle-setup".into())
            .spawn(move || digitalocean::rent(req, dir, Payloads::find(), progress));
        return;
    }
    ui.add_space(12.0);
    if ui.button("Back").clicked() {
        c.page = Page::Choose;
    }
}

/// Steps 1–3 of renting: account, token, paste.
fn token_steps(c: &mut CreateState, ui: &mut egui::Ui) {
    step_heading(ui, 1, "Make a DigitalOcean account");
    ui.horizontal_wrapped(|ui| {
        link_button(ui, "Sign up at DigitalOcean", digitalocean::SIGNUP_URL);
        dim(ui, "Pay with a card or PayPal. Already have an account? Skip this.");
    });

    step_heading(ui, 2, "Make an API token for Ingle");
    ui.horizontal_wrapped(|ui| {
        link_button(ui, "Open the API tokens page", digitalocean::TOKENS_URL);
    });
    dim(ui, "Click Generate New Token. Name it Ingle and pick the shortest expiration (Ingle only needs it while it sets the server up). Choose Custom Scopes and tick droplet (create, read, delete), ssh_key (create, read), regions (read), sizes (read) and account (read): nothing more. Click Generate Token, then copy the token it shows (it starts with dop_v1_). You can delete the token once your server is running.");

    step_heading(ui, 3, "Paste the token here");
    let loading = c.catalog_job.is_some();
    let mut go = false;
    ui.horizontal(|ui| {
        let r = ui.add_enabled(
            !loading,
            egui::TextEdit::singleline(&mut c.token).password(true).hint_text("dop_v1_…").desired_width(360.0).margin(egui::Margin::symmetric(10, 8)),
        );
        if r.changed() {
            c.catalog = None;
            c.catalog_error = None;
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if loading {
            ui.spinner();
        } else if c.catalog.is_none() && (ui.add(theme::primary_button("Continue").min_size(vec2(96.0, 32.0))).clicked() || enter) {
            go = true;
        }
    });
    if go {
        let token = c.token.trim().to_string();
        c.catalog_error = None;
        c.catalog_job = Some(spawn_job(ui.ctx(), move || digitalocean::load_catalog(&token)));
    }
    if let Some(f) = &c.catalog_error {
        ui.add_space(4.0);
        error_box(ui, f);
    }
}

fn own_page(c: &mut CreateState, ui: &mut egui::Ui, config_dir: &std::path::Path) {
    title(ui, "Use a server you already have");
    muted(ui, "Any 64-bit Linux server with systemd works: Ubuntu 22.04 or newer, Debian 11 or newer, Fedora, Rocky or Alma Linux, from any provider. Ingle logs in once over SSH to set it up and doesn't save your password.");
    field_label(ui, "Server address");
    input(ui, &mut c.host, "IP address or domain name, like 203.0.113.5");
    ui.columns(2, |cols| {
        field_label(&mut cols[0], "User");
        input(&mut cols[0], &mut c.user, "root");
        field_label(&mut cols[1], "SSH port");
        input(&mut cols[1], &mut c.ssh_port, "22");
    });
    field_label(ui, "Sign in with");
    ui.horizontal(|ui| {
        ui.radio_value(&mut c.use_key, false, "Password");
        ui.radio_value(&mut c.use_key, true, "Key file");
    });
    if c.use_key {
        input(ui, &mut c.key_path, "Path to your private key, like C:\\Users\\you\\.ssh\\id_ed25519");
        ui.add_space(4.0);
        ui.add(egui::TextEdit::singleline(&mut c.passphrase).password(true).hint_text("Key passphrase (if it has one)").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)));
        dim(ui, "OpenSSH, PEM or PKCS#8 keys. PuTTY .ppk keys need exporting first (PuTTYgen: Conversions > Export OpenSSH key).");
    } else {
        ui.add(egui::TextEdit::singleline(&mut c.password).password(true).hint_text("The server's password").desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)));
        dim(ui, "Your provider's welcome email or control panel has it.");
    }
    if c.user.trim() != "root" && !c.user.trim().is_empty() {
        field_label(ui, "Password for sudo (if needed)");
        ui.add(egui::TextEdit::singleline(&mut c.sudo_password).password(true).hint_text(if c.use_key { "This user's password" } else { "Leave empty to use the password above" }).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)));
    }
    if let Some(e) = &c.form_error {
        ui.add_space(4.0);
        ui.label(RichText::new(e).color(theme::LIVE).size(12.5));
    }
    ui.add_space(8.0);
    egui::CollapsingHeader::new(RichText::new("Firewalls: what to check").color(theme::TEXT)).id_salt("fw-help").show(ui, firewall_help);
    ui.add_space(10.0);
    let mut start = false;
    ui.horizontal(|ui| {
        if ui.add(theme::primary_button("Set it up").min_size(vec2(120.0, 36.0))).clicked() {
            start = true;
        }
        if ui.button("Back").clicked() {
            c.page = Page::Choose;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add(egui::Button::new(RichText::new("Set it up by hand instead").color(theme::LINK)).frame(false)).clicked() {
                c.page = Page::Manual;
            }
        });
    });
    if start {
        match login_from_form(c) {
            Err(e) => c.form_error = Some(e),
            Ok(login) => {
                c.form_error = None;
                let opts = InstallOptions {
                    name: c.name.trim().to_string(),
                    public_host: login.host.clone(),
                    port: ingle_host::DEFAULT_PORT,
                    payloads: Payloads::find(),
                    wait_for_boot: None,
                    restore: c.restore.as_ref().map(|(d, _)| d.clone()),
                };
                let progress = c.new_progress(ui.ctx());
                let dir = config_dir.to_path_buf();
                let _ = std::thread::Builder::new()
                    .name("ingle-setup".into())
                    .spawn(move || install::setup_existing(login, opts, progress, Some(dir)));
            }
        }
    }
}

fn login_from_form(c: &CreateState) -> Result<Login, String> {
    let host = ingle_host::clean_host(&c.host);
    if host.is_empty() {
        return Err("Type the server's address.".into());
    }
    let port: u16 = c.ssh_port.trim().parse().map_err(|_| "The SSH port must be a number (usually 22).".to_string())?;
    let user = c.user.trim().to_string();
    if user.is_empty() {
        return Err("Type the user to log in as (often root).".into());
    }
    let auth = if c.use_key {
        let path = c.key_path.trim().trim_matches('"').to_string();
        if path.is_empty() {
            return Err("Type the path to your key file.".into());
        }
        Auth::KeyFile { path: PathBuf::from(path), passphrase: Some(c.passphrase.clone()).filter(|p| !p.is_empty()) }
    } else {
        if c.password.is_empty() {
            return Err("Type the server's password.".into());
        }
        Auth::Password(c.password.clone())
    };
    // The host key seen when this server was set up before (if it was) is filled in by
    // `setup_existing` from Ingle's records.
    Ok(Login { host, port, user, auth, sudo_password: Some(c.sudo_password.clone()).filter(|p| !p.is_empty()), host_key: None })
}

fn downloads_folder() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|u| u.download_dir().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| std::env::temp_dir())
        .join("Ingle server files")
}

pub(crate) fn open_folder(path: &std::path::Path) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(path).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

fn manual_page(c: &mut CreateState, ui: &mut egui::Ui, acts: &mut Vec<Act>, close: &mut bool) {
    title(ui, "Set up a server by hand");
    muted(ui, "For a 64-bit Linux server with systemd. You'll copy two files to it and run one command.");
    let host = {
        let h = ingle_host::clean_host(&c.host);
        if h.is_empty() { "YOUR-SERVER".to_string() } else { h }
    };
    let user = if c.user.trim().is_empty() { "root".to_string() } else { c.user.trim().to_string() };
    let folder = match &c.saved_files {
        Some(Ok(f)) => f.clone(),
        _ => downloads_folder(),
    };

    step_heading(ui, 1, "Save the server files on this PC");
    ui.horizontal(|ui| {
        if ui.add(theme::primary_button("Save the files").min_size(vec2(130.0, 32.0))).clicked() {
            let target = downloads_folder();
            c.saved_files = Some(install::save_manual_files(&Payloads::find(), &target).map(|_| target).map_err(|e| format!("{e:#}")));
        }
        if let Some(Ok(f)) = &c.saved_files {
            if ui.button("Open the folder").clicked() {
                open_folder(f);
            }
        }
    });
    match &c.saved_files {
        Some(Ok(f)) => dim(ui, format!("Saved in {}", f.display())),
        Some(Err(e)) => {
            ui.label(RichText::new(e).color(theme::LIVE).size(12.5));
        }
        None => dim(ui, format!("They go into {}", folder.display())),
    }

    step_heading(ui, 2, "Copy them to the server");
    let shell = if cfg!(windows) { "In PowerShell or Command Prompt on this PC:" } else { "In a terminal on this PC:" };
    dim(ui, shell);
    let sep = std::path::MAIN_SEPARATOR;
    let f = folder.display().to_string();
    // Into your home folder on the server (not /tmp, where other accounts could swap the
    // files before they run).
    let scp = format!(
        "scp \"{f}{sep}ingle-server-linux-x86_64\" \"{f}{sep}{}\" {user}@{host}:",
        install::SCRIPT_NAME
    );
    code_box(ui, &scp, acts);
    dim(ui, "For an ARM server (like Oracle Cloud's free Ampere ones or Hetzner CAX), copy ingle-server-linux-aarch64 instead.");

    step_heading(ui, 3, "Run the setup on the server");
    let run = format!(
        "ssh -t {user}@{host} sh ./{} --binary ./ingle-server-linux-x86_64 --public-host {host}",
        install::SCRIPT_NAME
    );
    code_box(ui, &run, acts);
    dim(ui, "It installs Ingle Server as a service that starts by itself, opens UDP port 7766 in the server's firewall, and prints your owner link and an invite link. Running it again later updates the server and keeps everything.");

    step_heading(ui, 4, "Check your provider's firewall");
    egui::CollapsingHeader::new(RichText::new("Where to find it").color(theme::TEXT)).id_salt("fw-help-manual").show(ui, firewall_help);

    step_heading(ui, 5, "Join with your owner link");
    dim(ui, "Paste the owner link the setup printed into Join a server. The first person to use it becomes the owner, so use it yourself first.");
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.add(theme::primary_button("Join a server").min_size(vec2(130.0, 34.0))).clicked() {
            *close = true;
            acts.push(Act::OpenJoin);
        }
        if ui.button("Back").clicked() {
            c.page = Page::Own;
        }
    });
}

fn working_page(c: &mut CreateState, ui: &mut egui::Ui, _st: &AppState, acts: &mut Vec<Act>, close: &mut bool, config_dir: &std::path::Path) {
    let Some(progress) = c.progress.clone() else {
        c.page = Page::Choose;
        return;
    };
    let task = progress.snapshot();
    let heading = match (&task.outcome, c.kind) {
        (Some(Outcome::Ready(r)), _) => format!("{} is ready", r.name),
        (Some(Outcome::Failed(_)), _) => "The setup didn't finish".to_string(),
        (None, Kind::Rent) => "Creating your server".to_string(),
        (None, Kind::Own) => "Setting up your server".to_string(),
    };
    title(ui, &heading);

    let steps = |ui: &mut egui::Ui| {
        for (text, state) in &task.steps {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                match state {
                    StepState::Done => icons::done(ui.painter(), r, theme::ONLINE),
                    StepState::Failed => icons::failed(ui.painter(), r),
                    StepState::Running => {
                        ui.put(r, egui::Spinner::new().size(16.0).color(theme::EMBER2));
                    }
                }
                let col = if *state == StepState::Running { theme::TEXT } else { theme::MUTED };
                ui.label(RichText::new(text).color(col));
            });
        }
    };
    if !matches!(task.outcome, Some(Outcome::Ready(_))) {
        steps(ui);
    }
    if task.outcome.is_none() {
        ui.add_space(6.0);
        dim(ui, if c.kind == Kind::Rent { "This takes a few minutes. Keep Ingle open." } else { "This takes about a minute. Keep Ingle open." });
    }

    match &task.outcome {
        None => {
            ui.add_space(8.0);
            if task.stop_requested {
                dim(ui, "Stopping…");
            } else if ui.button("Stop").clicked() {
                progress.request_stop();
            }
        }
        Some(Outcome::Ready(r)) => {
            ui.add_space(10.0);
            let reachable = c.reachable_now.or(r.reachable);
            if let Some(e) = &c.join_error {
                ui.label(RichText::new(format!("Couldn't open it here: {e}")).color(theme::LIVE));
            } else if reachable == Some(false) {
                muted(ui, format!("It's running at {} and it's in the list on the left, where you'll be its owner once this PC can reach it.", r.host));
            } else {
                muted(ui, format!("It's running at {}. You've joined it as the owner: it's in the list on the left.", r.host));
            }
            if reachable == Some(false) {
                ui.add_space(8.0);
                egui::Frame::new()
                    .fill(Color32::from_rgb(0x3a, 0x2a, 0x16))
                    .stroke(Stroke::new(1.0f32, theme::EMBER2))
                    .corner_radius(8)
                    .inner_margin(10)
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new(format!("This PC can't reach the server on UDP port {} yet, so calls and chat won't connect.", r.port)).color(theme::TEXT));
                        ui.label(RichText::new("A firewall in your provider's website is probably blocking it. Let in UDP port 7766 from anywhere there, then check again.").size(12.5).color(theme::MUTED));
                        egui::CollapsingHeader::new(RichText::new("Where to find it").color(theme::TEXT)).id_salt("fw-help-done").show(ui, firewall_help);
                        ui.horizontal(|ui| {
                            if c.probe_job.is_some() {
                                ui.spinner();
                                ui.label(RichText::new("Checking…").color(theme::MUTED));
                            } else if ui.button("Check again").clicked() {
                                let link = r.owner_link.clone();
                                c.probe_job = Some(spawn_job(ui.ctx(), move || {
                                    ingle_host::probe::reachable_blocking(&link, 2, std::time::Duration::from_secs(4)).is_ok()
                                }));
                            }
                        });
                    });
            } else if c.reachable_now == Some(true) {
                ui.label(RichText::new("Reachable now.").color(theme::ONLINE));
            }
            field_label(ui, "Invite link for friends");
            egui::Frame::new().fill(theme::CODE_BG).corner_radius(6).inner_margin(10).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.add(egui::Label::new(RichText::new(ingle_proto::invite::InviteLink::shorten(&r.invite_link)).font(theme::mono(12.0)).color(theme::TEXT)).wrap().selectable(true));
            });
            ui.add_space(4.0);
            if ui.add(theme::primary_button("Copy invite link").min_size(vec2(150.0, 32.0))).clicked() {
                acts.push(Act::Copy(ingle_proto::invite::InviteLink::shorten(&r.invite_link)));
                c.copied = true;
            }
            if c.copied {
                ui.label(RichText::new("Copied. Paste it into a message to your friends.").size(12.5).color(theme::ONLINE));
            }
            dim(ui, "Friends install Ingle (Windows: the setup .exe; Linux: the .deb package), click + and then Join a server, and paste this link.");
            if let Some(d) = &r.droplet {
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    dim(ui, format!("On DigitalOcean it's the droplet \"{}\" and costs ${:.2} a month until you delete it there.", d.name, c.price));
                    link_button(ui, "Open DigitalOcean", digitalocean::DROPLETS_URL);
                });
            }
            for w in &r.warnings {
                ui.label(RichText::new(format!("Note: {w}")).size(12.5).color(theme::EMBER2));
            }
            ui.add_space(12.0);
            if ui.add(egui::Button::new(RichText::new("Done").font(theme::bold_font(14.0)).color(theme::TEXT)).fill(theme::ELEV2).min_size(vec2(110.0, 34.0))).clicked() {
                *close = true;
            }
            ui.add_space(8.0);
            egui::CollapsingHeader::new(RichText::new("What Ingle did").color(theme::MUTED)).id_salt("setup-steps").show(ui, steps);
        }
        Some(Outcome::Failed(f)) => {
            ui.add_space(8.0);
            error_box(ui, f);
            let mut retry = false;
            if let Some(d) = &f.droplet {
                ui.add_space(8.0);
                egui::Frame::new().fill(theme::ELEV).corner_radius(8).inner_margin(10).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    match &c.deleted {
                        Some(Ok(())) => {
                            ui.label(RichText::new(format!("The server \"{}\" was deleted. DigitalOcean only charges for the time it existed.", d.name)).color(theme::TEXT));
                        }
                        _ => {
                            let ip = d.ip.as_deref().map(|ip| format!(" at {ip}")).unwrap_or_default();
                            ui.label(RichText::new(format!("Your DigitalOcean server \"{}\"{ip} was created and costs money until it's deleted.", d.name)).color(theme::TEXT));
                            if let Some(Err(e)) = &c.deleted {
                                ui.label(RichText::new(format!("Deleting failed: {}", e.error)).color(theme::LIVE).size(12.5));
                            }
                            ui.horizontal(|ui| {
                                if d.ip.is_some() && ui.add(theme::primary_button("Try the setup again")).clicked() {
                                    retry = true;
                                }
                                if c.delete_job.is_some() {
                                    ui.spinner();
                                } else if ui.add(theme::danger_button("Delete the server")).clicked() {
                                    let (token, id, dir) = (c.token.trim().to_string(), d.id, config_dir.to_path_buf());
                                    c.delete_job = Some(spawn_job(ui.ctx(), move || digitalocean::delete(&token, id, &dir)));
                                }
                                link_button(ui, "Open DigitalOcean", digitalocean::DROPLETS_URL);
                            });
                        }
                    }
                });
            }
            if retry {
                if let Some(d) = f.droplet.clone() {
                    let (name, dir) = (c.name.trim().to_string(), config_dir.to_path_buf());
                    let restore = c.restore.as_ref().map(|(d, _)| d.clone());
                    let progress = c.new_progress(ui.ctx());
                    let _ = std::thread::Builder::new()
                        .name("ingle-setup".into())
                        .spawn(move || digitalocean::retry_install(d, name, dir, Payloads::find(), progress, restore));
                    return;
                }
            }
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                let back_to = if c.kind == Kind::Rent { Page::Rent } else { Page::Own };
                let can_leave = c.delete_job.is_none();
                if ui.add_enabled(can_leave, theme::primary_button("Back")).clicked() {
                    c.page = back_to;
                    c.progress = None;
                }
                if ui.add_enabled(can_leave, egui::Button::new("Close")).clicked() {
                    *close = true;
                }
            });
        }
    }

    if !task.log.is_empty() {
        ui.add_space(10.0);
        egui::CollapsingHeader::new(RichText::new("Details").color(theme::MUTED)).id_salt("setup-log").default_open(c.show_log).show(ui, |ui| {
            egui::ScrollArea::vertical().max_height(180.0).stick_to_bottom(true).show(ui, |ui| {
                for line in &task.log {
                    ui.add(egui::Label::new(RichText::new(line).font(theme::mono(11.5)).color(theme::MUTED)).wrap().selectable(true));
                }
            });
        });
    }
}
