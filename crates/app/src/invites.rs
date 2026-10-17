//! Invites like Discord's: short links (`https://pyr.gg/<code>`) looked up at pyr.gg, the
//! choices for a new invite (how long it lasts, how many can use it), and the card an invite
//! link shows in chat.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use parking_lot::Mutex;
use pyr_proto::directory;
use pyr_proto::invite::InviteLink;

/// What a short link opens, as far as known.
#[derive(Clone, Debug, PartialEq)]
pub enum Lookup {
    /// Being looked up.
    Pending,
    Found(directory::Lookup),
    /// No such invite (any more).
    Missing,
    /// pyr.gg couldn't be asked: why.
    Failed(String),
}

/// What short links opened (looked up once, and again once the answer is a few minutes old).
#[derive(Default)]
pub struct Lookups {
    map: Arc<Mutex<HashMap<String, (Instant, Lookup)>>>,
}

impl Lookups {
    /// What `code` opens. Asks pyr.gg (in the background) the first time, and repaints `ctx`
    /// once it knows.
    pub fn get(&self, code: &str, ctx: &egui::Context) -> Lookup {
        let mut m = self.map.lock();
        if let Some((at, v)) = m.get(code) {
            let keep = match v {
                Lookup::Pending => Duration::from_secs(30),
                Lookup::Found(_) => Duration::from_secs(300),
                Lookup::Missing => Duration::from_secs(60),
                Lookup::Failed(_) => Duration::from_secs(15),
            };
            if at.elapsed() < keep {
                return v.clone();
            }
        }
        if m.len() > 300 {
            m.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(60));
        }
        m.insert(code.to_string(), (Instant::now(), Lookup::Pending));
        let (map, code, ctx) = (self.map.clone(), code.to_string(), ctx.clone());
        std::thread::spawn(move || {
            let found = fetch(&code);
            map.lock().insert(code, (Instant::now(), found));
            ctx.request_repaint();
        });
        Lookup::Pending
    }

    /// Asks again next time (Try again).
    pub fn forget(&self, code: &str) {
        self.map.lock().remove(code);
    }
}

/// Where the directory is: pyr.gg, or `PYR_DIRECTORY` (a test one).
pub fn base() -> String {
    pyr_proto::env_var("DIRECTORY")
        .map(|d| d.trim().trim_end_matches('/').to_string())
        .filter(|d| d.starts_with("https://") || d.starts_with("http://"))
        .unwrap_or_else(|| directory::DIRECTORY.to_string())
}

fn fetch(code: &str) -> Lookup {
    let base = base();
    let host = base.split_once("://").map_or(base.as_str(), |(_, h)| h);
    let mut cfg = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(10)))
        .http_status_as_error(false)
        .user_agent(concat!("Pyr/", env!("CARGO_PKG_VERSION")));
    if host.starts_with("127.0.0.1") || host.starts_with("localhost") {
        cfg = cfg.proxy(None);
    }
    let agent: ureq::Agent = cfg.build().into();
    match agent.get(&format!("{base}/api/v1/invites/{code}")).call() {
        Err(e) => Lookup::Failed(format!("Couldn't reach pyr.gg ({e}).")),
        Ok(mut r) => match r.status().as_u16() {
            200 => match r.body_mut().with_config().limit(64 * 1024).read_json::<directory::Lookup>() {
                // (Only an invite link that's what it says it is: it's what gets joined.)
                Ok(l) if l.link.starts_with("pyr://") && InviteLink::parse(&l.link).is_ok() => Lookup::Found(l),
                _ => Lookup::Failed("pyr.gg's answer didn't make sense.".into()),
            },
            404 => Lookup::Missing,
            429 => Lookup::Failed("Too many invites looked up from here just now. Try again in a minute.".into()),
            s => Lookup::Failed(format!("pyr.gg answered HTTP {s}.")),
        },
    }
}

/// How long a new invite lasts: Discord's choices (7 days unless picked otherwise).
pub const EXPIRE: &[(&str, Option<u64>)] = &[
    ("30 minutes", Some(1800)),
    ("1 hour", Some(3600)),
    ("6 hours", Some(6 * 3600)),
    ("12 hours", Some(12 * 3600)),
    ("1 day", Some(86_400)),
    ("7 days", Some(7 * 86_400)),
    ("Never", None),
];
pub const DEFAULT_EXPIRE: usize = 5;
/// How many people can use it.
pub const USES: &[(&str, Option<u32>)] =
    &[("No limit", None), ("1 use", Some(1)), ("5 uses", Some(5)), ("10 uses", Some(10)), ("25 uses", Some(25)), ("50 uses", Some(50)), ("100 uses", Some(100))];

/// "6 days", "23 hours", "12 minutes": how long until `at` (ms since 1970), rounded down to
/// the biggest unit, as Discord says it.
pub fn time_left(at: i64, now: i64) -> String {
    let s = ((at - now).max(0) / 1000) as u64;
    let (n, unit) = if s >= 86_400 {
        (s / 86_400, "day")
    } else if s >= 3600 {
        (s / 3600, "hour")
    } else if s >= 60 {
        (s / 60, "minute")
    } else {
        (s.max(1), "second")
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// The invite links in a message (short and full ones), in order, each once, at most three.
pub fn links_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split(|c: char| c.is_whitespace()) {
        // (Not `_` or `-`: a fingerprint can end with one.)
        let w = word.trim_matches(|c: char| matches!(c, '<' | '>' | '(' | ')' | '[' | ']' | '"' | '\'' | ',' | '.' | '!' | '?' | ';' | ':' | '*' | '`'));
        let is_short = directory::short_code(w).is_some();
        let is_full = !is_short && is_full_invite(w);
        if (is_short || is_full) && !out.iter().any(|o| o == w) {
            out.push(w.to_string());
            if out.len() == 3 {
                break;
            }
        }
    }
    out
}

/// A full invite link Pyr can join from: `pyr://…` (or `ingle://…`, from before the rename),
/// or the web form on pyr.gg (or firelitgames.com/ingle, before it).
pub fn is_full_invite(url: &str) -> bool {
    let u = url.trim();
    let app = ["pyr://", "ingle://"].iter().any(|s| u.get(..s.len()).is_some_and(|p| p.eq_ignore_ascii_case(s)));
    let web = [pyr_proto::invite::WEB_INVITE, "https://firelitgames.com/ingle/"].iter().any(|w| u.starts_with(w)) && u.contains("join=");
    (app || web) && InviteLink::parse(u).is_ok()
}

/// The server name in a web invite link (`…#n=<name>&join=…`), if it has one.
pub fn name_in(link: &str) -> Option<String> {
    let (_, fragment) = link.split_once('#')?;
    let raw = fragment.split('&').find_map(|p| p.strip_prefix("n="))?;
    let name = percent_decode(raw);
    let name: String = name.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(60).collect();
    (!name.is_empty()).then_some(name)
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A server's initials, for its picture when it has none (as Discord does): "Brethren of
/// Blood" is "BoB".
pub fn initials(name: &str) -> String {
    let s: String = name.split_whitespace().filter_map(|w| w.chars().find(|c| c.is_alphanumeric())).take(4).collect();
    if s.is_empty() { "?".into() } else { s }
}

impl crate::app::PyrApp {
    /// The card an invite link shows under its message, like Discord's: the server's name and
    /// how many are there (from pyr.gg, for a short link), and Join (or Joined, which opens it).
    pub(crate) fn invite_embed(&mut self, ui: &mut egui::Ui, st: &pyr_engine::state::AppState, link: &str, acts: &mut Vec<crate::app::Act>) {
        use crate::app::Act;
        use crate::theme;
        use egui::{Align, Align2, CornerRadius, Layout, RichText, Sense, vec2};
        // What it opens: (what to say instead of a server, the server's name, counts, its full link).
        let (status, name, online, members, full) = match directory::short_code(link) {
            Some(code) => match self.invite_lookups.get(&code, ui.ctx()) {
                Lookup::Found(l) => (None, Some(l.name), l.online, l.members, Some(l.link)),
                Lookup::Pending => (Some("Looking up the invite…"), None, None, None, None),
                Lookup::Missing => (Some("This invite isn't valid any more"), None, None, None, None),
                Lookup::Failed(_) => (Some("Couldn't look this invite up right now"), None, None, None, None),
            },
            None => {
                let host = InviteLink::parse(link).ok().map(|l| l.host);
                (None, name_in(link).or(host), None, None, Some(link.to_string()))
            }
        };
        let joined = full.as_deref().and_then(|f| InviteLink::parse(f).ok()).is_some_and(|l| {
            st.servers.iter().any(|s| pyr_proto::b64::decode_array::<32>(&s.key).is_some_and(|fp| l.matches(&fp)))
        });
        egui::Frame::new().fill(theme::ELEV()).corner_radius(8).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
            ui.set_width(ui.available_width().min(432.0));
            let head = if status == Some("This invite isn't valid any more") { "AN INVITE, BUT…" } else { "AN INVITE TO JOIN A SERVER" };
            ui.label(RichText::new(head).font(theme::bold_font(11.5)).color(theme::DIM()));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(50.0, 50.0), Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(15), theme::SIDE());
                let mark = match &name {
                    Some(n) => initials(n),
                    None if status == Some("This invite isn't valid any more") => "×".into(),
                    None => "…".into(),
                };
                ui.painter().text(r.center(), Align2::CENTER_CENTER, mark, theme::bold_font(17.0), theme::TEXT());
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.add_space(4.0);
                    match (&name, status) {
                        (Some(n), _) => {
                            ui.add(egui::Label::new(RichText::new(n).font(theme::bold_font(16.0)).color(theme::TEXT())).truncate());
                        }
                        (None, Some(s)) => {
                            ui.label(RichText::new(s).font(theme::bold_font(15.0)).color(theme::MUTED()));
                        }
                        (None, None) => {}
                    }
                    let dot = |ui: &mut egui::Ui, c: egui::Color32| {
                        let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, c);
                    };
                    if online.is_some() || members.is_some() {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 5.0;
                            if let Some(o) = online {
                                dot(ui, theme::ONLINE());
                                ui.label(RichText::new(format!("{o} Online")).size(12.5).color(theme::MUTED()));
                                ui.add_space(6.0);
                            }
                            if let Some(m) = members {
                                dot(ui, theme::DIM());
                                ui.label(RichText::new(format!("{m} {}", if m == 1 { "Member" } else { "Members" })).size(12.5).color(theme::MUTED()));
                            }
                        });
                    } else if status == Some("This invite isn't valid any more") {
                        ui.label(RichText::new("Ask for a new one.").size(12.5).color(theme::DIM()));
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(f) = &full {
                        let b = if joined { egui::Button::new(RichText::new("Joined").color(theme::TEXT())) } else { theme::primary_button("Join") };
                        if ui.add(b.min_size(vec2(84.0, 34.0))).clicked() {
                            // (Joined opens it.)
                            acts.push(Act::OfferJoin(if joined { f.clone() } else { link.to_string() }));
                        }
                    }
                });
            });
        });
    }

    /// Server Settings → Invites: the invites that still work (taken back with ×), and for the
    /// owner, short links on pyr.gg and the community's own code.
    pub(crate) fn invites_tab(&mut self, ui: &mut egui::Ui, s: &pyr_engine::state::ServerView, acts: &mut Vec<crate::app::Act>) {
        use crate::app::Act;
        use crate::dialogs::{field_label, notice};
        use crate::theme;
        use egui::{Align, Layout, RichText, vec2};
        let key = s.key.clone();
        // Asked for when the tab opens, and again every half minute while it's open.
        let asked_id = egui::Id::new(("invites-asked", &key));
        let now = ui.input(|i| i.time);
        let asked: Option<f64> = ui.data(|d| d.get_temp(asked_id));
        if asked.is_none_or(|t| now - t > 30.0) {
            ui.data_mut(|d| d.insert_temp(asked_id, now));
            acts.push(Act::ListInvites { key: key.clone() });
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(30));
        let owner = s.me.as_ref().is_some_and(|m| m.role == pyr_proto::control::Role::Owner);
        if let Some(e) = &s.invites_error {
            notice(ui, e, true);
            ui.add_space(6.0);
        }
        let Some(view) = &s.invites else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Getting the invites…").color(theme::MUTED()));
            });
            return;
        };
        if owner {
            field_label(ui, "Short links");
            let mut on = view.short_links;
            if ui.checkbox(&mut on, "List invites at pyr.gg, so they work as short links like pyr.gg/qrslmnov").changed() {
                acts.push(Act::SetShortLinks { key: key.clone(), on });
            }
            ui.label(
                RichText::new(
                    "pyr.gg then knows this server's name and address, how many members it has and how many are online, and its invite \
                     codes: never its messages or who's in it. Off, invites are full links that work without pyr.gg.",
                )
                .size(12.0)
                .color(theme::DIM()),
            );
            if let Some(why) = &view.short_links_problem {
                ui.add_space(4.0);
                notice(ui, why, false);
            }
            if view.short_links {
                field_label(ui, "Custom invite link");
                let edit_id = egui::Id::new(("vanity-edit", &key));
                let mut text: String = ui.data(|d| d.get_temp(edit_id)).unwrap_or_else(|| view.vanity.clone().unwrap_or_default());
                ui.horizontal(|ui| {
                    ui.label(RichText::new("pyr.gg/").font(theme::mono(13.5)).color(theme::MUTED()));
                    let resp = ui.add(egui::TextEdit::singleline(&mut text).hint_text("your-community").char_limit(directory::CODE_MAX).desired_width(220.0).margin(egui::Margin::symmetric(8, 6)));
                    let typed = text.trim().to_ascii_lowercase();
                    let changed = Some(&typed) != view.vanity.as_ref() && !typed.is_empty();
                    if ui.add_enabled(changed, theme::primary_button("Save")).clicked() || (changed && resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))) {
                        acts.push(Act::SetVanity { key: key.clone(), code: Some(typed.clone()) });
                    }
                    if let Some(v) = &view.vanity {
                        if ui.button("Copy").clicked() {
                            acts.push(Act::Copy(directory::short_url(v)));
                        }
                        if ui.button("Remove").clicked() {
                            acts.push(Act::SetVanity { key: key.clone(), code: None });
                            text.clear();
                        }
                    }
                });
                let typed = text.trim().to_ascii_lowercase();
                match (typed.is_empty(), directory::vanity_problem(&typed)) {
                    (false, Some(why)) => {
                        ui.label(RichText::new(why).size(12.0).color(theme::LIVE()));
                    }
                    _ => {
                        ui.label(
                            RichText::new("Anyone with it can join, and it never runs out (like the community's everyday link). Changing it stops the old one working.")
                                .size(12.0)
                                .color(theme::DIM()),
                        );
                    }
                }
                ui.data_mut(|d| d.insert_temp(edit_id, text));
            }
        }
        field_label(ui, if s.can(pyr_proto::control::perm::MANAGE_SERVER) { "Invites" } else { "My invites" });
        if view.invites.is_empty() {
            ui.label(RichText::new("No invites that still work. Invite People makes one.").color(theme::MUTED()));
            return;
        }
        let now_ms = pyr_proto::now_ms();
        egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
            egui::Grid::new(("invites", &key)).num_columns(5).spacing(vec2(14.0, 8.0)).striped(true).show(ui, |ui| {
                for h in ["INVITER", "INVITE CODE", "USES", "EXPIRES", ""] {
                    ui.label(RichText::new(h).font(theme::bold_font(11.0)).color(theme::DIM()));
                }
                ui.end_row();
                for i in &view.invites {
                    let who = if i.created_by == "server" { "The server".to_string() } else { s.member_name(&i.created_by) };
                    ui.label(RichText::new(who).color(theme::TEXT()));
                    let code = ui.add(egui::Label::new(RichText::new(&i.code).font(theme::mono(13.0)).color(theme::TEXT())).sense(egui::Sense::click()));
                    let link = i.short.clone().unwrap_or_else(|| format!("{} (a full link)", i.code));
                    if code.on_hover_text(format!("{link}\nClick to copy")).clicked() {
                        acts.push(Act::Copy(i.short.clone().unwrap_or_else(|| i.code.clone())));
                    }
                    let uses = match i.max_uses {
                        Some(m) => format!("{}/{m}", i.uses),
                        None => i.uses.to_string(),
                    };
                    ui.label(RichText::new(uses).color(theme::BODY()));
                    let expires = match i.expires_at {
                        Some(at) => time_left(at, now_ms),
                        None => "Never".into(),
                    };
                    ui.label(RichText::new(if i.bot { format!("{expires} (bot)") } else { expires }).color(theme::BODY()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.small_button("×").on_hover_text("Take this invite back: it stops working").clicked() {
                            acts.push(Act::RevokeInvite { key: key.clone(), code: i.code.clone() });
                        }
                    });
                    ui.end_row();
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_links_found_in_messages() {
        let fp = pyr_proto::b64::encode(&[3u8; 9]);
        let full = format!("pyr://2.24.121.231/abc/{fp}");
        let text = format!(
            "join us: https://pyr.gg/qrslmnov, or (pyr.gg/invite/abcdefgh). Old: {full} and again https://pyr.gg/qrslmnov! \
             not these: https://pyr.gg/download https://example.com/qrslmnov pyr://nope https://pyr.gg/#notjoin=1"
        );
        assert_eq!(links_in(&text), vec!["https://pyr.gg/qrslmnov".to_string(), "pyr.gg/invite/abcdefgh".to_string(), full.clone()]);
        assert!(links_in("nothing here").is_empty());
        let web = format!("https://pyr.gg/#n=Brethren%20of%20Blood&join=2.24.121.231/abc/{fp}");
        assert!(is_full_invite(&web) && !is_full_invite("https://pyr.gg/qrslmnov"));
        assert_eq!(name_in(&web).as_deref(), Some("Brethren of Blood"));
        assert_eq!(name_in(&full), None);
        assert_eq!(name_in("https://pyr.gg/#n=%E2%9C%A8%20Camp&join=x"), Some("✨ Camp".to_string()));
        assert_eq!(name_in("https://pyr.gg/#n=%zz&join=x"), Some("%zz".to_string()));
    }

    #[test]
    fn times_and_initials() {
        let now = 1_000_000_000;
        assert_eq!(time_left(now + 7 * 86_400_000, now), "7 days");
        assert_eq!(time_left(now + 86_400_000 + 5, now), "1 day");
        assert_eq!(time_left(now + 23 * 3_600_000 + 59 * 60_000, now), "23 hours");
        assert_eq!(time_left(now + 61_000, now), "1 minute");
        assert_eq!(time_left(now - 5, now), "1 second");
        assert_eq!(initials("Brethren of Blood"), "BoB");
        assert_eq!(initials("  "), "?");
        assert_eq!(initials("🔥 Camp"), "C");
        assert_eq!(EXPIRE[DEFAULT_EXPIRE].0, "7 days");
    }
}
