//! Pyr Poker's staff: Game Masters (the owner's admins for the game: they gift chips and looks,
//! and drop prize boxes on tables) and Ambassadors (prize boxes only). The owner names them and
//! reads what they gave; each gives at most a day's allowance, which follows the rake. The
//! owner gives as much as they like (0.12.0 servers), and that's in the log too.

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Stroke, StrokeKind, vec2};
use pyr_engine::state::ServerView;
use pyr_poker::net::{PrizeView, Role, TableId, ToServer};
use pyr_poker::progress::{COSMETICS, Kind};

use super::cards::chips_text;
use crate::app::PyrApp;
use crate::theme;

/// A gift or prize box being made up.
#[derive(Clone, Default)]
pub struct Giving {
    pub to: Option<String>,
    pub chips: u64,
    pub item: Option<String>,
    pub note: String,
}

/// How long ago: "just now", "5 min ago", "3 h ago", "2 days ago".
fn ago(at_ms: u64) -> String {
    let now = pyr_proto::now_ms().max(0) as u64;
    let m = now.saturating_sub(at_ms) / 60_000;
    match m {
        0 => "just now".into(),
        1..=59 => format!("{m} min ago"),
        60..=2879 => format!("{} h ago", m / 60),
        _ => format!("{} days ago", m / 1440),
    }
}

fn kind_name(k: Kind) -> &'static str {
    match k {
        Kind::CardBack => "Card back",
        Kind::Felt => "Table",
        Kind::Chips => "Chips",
        Kind::Frame => "Picture frame",
    }
}

/// Picks a look to give (or none), prize-only looks first.
fn look_picker(ui: &mut egui::Ui, id: &str, item: &mut Option<String>) {
    let label = item.as_deref().and_then(pyr_poker::progress::cosmetic).map(|c| format!("{}: {}", kind_name(c.kind), c.name)).unwrap_or_else(|| "No look".into());
    egui::ComboBox::from_id_salt(id).selected_text(label).width(240.0).show_ui(ui, |ui| {
        ui.selectable_value(item, None, "No look");
        let mut all: Vec<_> = COSMETICS.iter().filter(|c| !matches!(c.unlock, pyr_poker::progress::Unlock::Always)).collect();
        all.sort_by_key(|c| !matches!(c.unlock, pyr_poker::progress::Unlock::Prize));
        for c in all {
            let prize = matches!(c.unlock, pyr_poker::progress::Unlock::Prize);
            let text = format!("{}: {}{}", kind_name(c.kind), c.name, if prize { "  ★ prize only" } else { "" });
            ui.selectable_value(item, Some(c.id.to_string()), text);
        }
    });
}

/// No limit: the most one gift box takes (a quadrillion chips: still exact as the box counts).
pub const NO_LIMIT: u64 = 1_000_000_000_000_000;

/// What I can give away here, if anything: the server's owner as much as they like (on a 0.12.0
/// server; every gift's in the log), a Game Master what's left of today's allowance.
pub fn gift_limit(s: &ServerView) -> Option<u64> {
    if owner(s) {
        return Some(NO_LIMIT);
    }
    let v = s.poker.staff.as_ref()?;
    (v.me == Some(Role::GameMaster)).then_some(v.left)
}

/// What I can put in a prize box: the same, but Ambassadors too.
pub fn prize_limit(s: &ServerView) -> Option<u64> {
    if owner(s) {
        return Some(NO_LIMIT);
    }
    let v = s.poker.staff.as_ref()?;
    v.me.map(|_| v.left)
}

/// I give without a limit here: the Owner of the Game (on a 0.13.0 server, as it says); on an
/// 0.12.0 server, its owner.
fn owner(s: &ServerView) -> bool {
    if s.has(pyr_proto::control::CASINO_013_FEATURE) {
        return s.poker.staff.as_ref().is_some_and(|v| v.owner);
    }
    let mine = s.poker.staff.as_ref().map(|v| v.owner).unwrap_or_else(|| s.me.as_ref().is_some_and(|m| m.role == pyr_proto::control::Role::Owner));
    mine && s.has(pyr_proto::control::CASINO_012_FEATURE)
}

/// What to say about it: what's left today, or that there's no limit.
pub fn limit_words(max: u64) -> String {
    if max >= NO_LIMIT {
        "no limit: you're the Owner of the Game (every gift goes in the log)".to_string()
    } else {
        format!("left to give today: {}", chips_text(max))
    }
}

pub fn chips_box(ui: &mut egui::Ui, chips: &mut u64, max: u64) {
    let speed = if max >= NO_LIMIT { 1_000.0 } else { 100.0 };
    ui.add(egui::DragValue::new(chips).range(0..=max).speed(speed).suffix(" chips"));
    let steps: [u64; 3] = if max >= NO_LIMIT { [10_000, 100_000, 1_000_000] } else { [1_000, 5_000, 25_000] };
    for n in steps {
        if n <= max && ui.small_button(format!("+{}", chips_text(n))).clicked() {
            *chips = (*chips + n).min(max);
        }
    }
}

impl PyrApp {
    pub fn my_poker_role(&self, s: &ServerView) -> Option<Role> {
        s.poker.me.as_ref().and_then(|m| m.role)
    }

    /// "Give chips…" in the menu from right-clicking someone's name, for a Game Master here (or
    /// the server's owner): it opens a small box to give them chips from today's allowance (the
    /// owner, any number). The server checks it all. True when it was chosen.
    pub(crate) fn give_chips_item(&mut self, ui: &mut egui::Ui, s: &ServerView, uid: &str) -> bool {
        if !s.has(pyr_proto::control::POKER_FEATURE) || s.is_bot(uid) || s.me.as_ref().is_none_or(|m| m.user_id == uid) {
            return false;
        }
        // Whether I'm a Game Master here is asked of the server now and then, so it's known
        // without opening the casino first.
        if s.poker.staff.is_none() && self.poker.staff_asked.get(&s.key).is_none_or(|t| t.elapsed().as_secs() >= 30) {
            self.poker.staff_asked.insert(s.key.clone(), std::time::Instant::now());
            self.engine.poker(&s.key, ToServer::Staff);
        }
        let role = s.poker.staff.as_ref().and_then(|v| v.me).or_else(|| self.my_poker_role(s));
        let owner = gift_limit(s).is_some_and(|m| m >= NO_LIMIT);
        if role != Some(Role::GameMaster) && !owner {
            return false;
        }
        let tip = if owner { "A gift of chips, as many as you like (you're the Owner of the Game): it goes in the log" } else { "A gift of chips from what you have left to give today, as a Game Master" };
        if ui.button("Give chips…").on_hover_text(tip).clicked() {
            self.poker.giving = Giving { to: Some(uid.to_string()), ..Default::default() };
            self.poker.gift_box = Some(s.key.clone());
            self.poker.gift_sent = None;
            // (What's left to give today, fresh.)
            self.engine.poker(&s.key, ToServer::Staff);
            return true;
        }
        false
    }

    /// The box "Give chips…" opens: how many and a note, with what's left to give today. It
    /// stays up until the gift has gone through (what's left drops by it, or with no limit, it's
    /// in the log), then says so: after the frame (`acts`), as the shared state's being read
    /// while the window's drawn and a toast is written into it.
    pub(crate) fn gift_box(&mut self, ctx: &egui::Context, st: &pyr_engine::state::AppState, acts: &mut Vec<crate::app::Act>) {
        let Some(key) = self.poker.gift_box.clone() else { return };
        let (Some(s), Some(to)) = (st.server(&key), self.poker.giving.to.clone()) else {
            self.poker.gift_box = None;
            return;
        };
        let name = s.member_name(&to);
        let left = s.poker.staff.as_ref().and(gift_limit(s));
        let log_top = s.poker.staff.as_ref().and_then(|v| v.log.first()).map(|l| l.at_ms);
        // Gone through: what's left dropped by the gift (or, with no limit, it's in the log).
        if let (Some((before, chips, at, top)), Some(now_left)) = (self.poker.gift_sent, left) {
            if (now_left < NO_LIMIT && now_left + chips <= before) || (now_left >= NO_LIMIT && log_top > top) {
                acts.push(crate::app::Act::Toast(format!("Gave {name} {} chips.", chips_text(chips)), false));
                self.poker.gift_box = None;
                self.poker.gift_sent = None;
                self.poker.giving = Default::default();
                return;
            }
            // (Refused: the server's reason shows as an error; the box is back to try again.)
            if at.elapsed().as_secs() >= 6 {
                self.poker.gift_sent = None;
            }
        }
        let mut g = self.poker.giving.clone();
        let mut open = true;
        let mut give = false;
        let sending = self.poker.gift_sent.is_some();
        egui::Window::new(RichText::new(format!("Give chips to {name}")).font(theme::bold_font(16.0)))
            .id(egui::Id::new("gift-box"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(380.0);
                let Some(left) = left else {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Checking what you have left to give today…").color(theme::MUTED()));
                    });
                    return;
                };
                let words = limit_words(left);
                let mut c = words.chars();
                let words = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
                ui.label(RichText::new(words).color(theme::MUTED()));
                ui.add_space(4.0);
                ui.horizontal(|ui| chips_box(ui, &mut g.chips, left));
                ui.add(egui::TextEdit::singleline(&mut g.note).hint_text("A note (optional)").desired_width(f32::INFINITY).char_limit(140));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let ok = g.chips > 0 && g.chips <= left && !sending;
                    if ui.add_enabled(ok, theme::primary_button(&format!("Give {} chips", chips_text(g.chips)))).clicked() {
                        give = true;
                    }
                    if sending {
                        ui.spinner();
                    }
                });
            });
        if give {
            let before = left.unwrap_or(0);
            self.engine.poker(&key, ToServer::Gift { uid: to, chips: g.chips, item: None, note: g.note.clone() });
            self.poker.gift_sent = Some((before, g.chips, std::time::Instant::now(), log_top));
        }
        self.poker.giving = g;
        if !open {
            self.poker.gift_box = None;
            self.poker.gift_sent = None;
        }
    }

    /// The lobby's Staff tab (lined up with the lobby's other tabs).
    pub fn staff_tab(&mut self, ui: &mut egui::Ui, s: &ServerView) {
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            ui.vertical(|ui| {
                ui.set_max_width((ui.available_width() - 28.0).min(880.0));
                self.staff_tab_inner(ui, s);
            });
        });
    }

    fn staff_tab_inner(&mut self, ui: &mut egui::Ui, s: &ServerView) {
        let Some(v) = s.poker.staff.clone() else {
            ui.label(RichText::new("Loading…").color(theme::MUTED()));
            return;
        };
        ui.add_space(6.0);
        ui.label(RichText::new("Game Masters run the game with the Owner of the Game, on every server: they gift chips and looks, and drop prize boxes on tables. Ambassadors drop prize boxes (whoever wins the next hand opens it). Each gives at most a day's allowance, which follows the rake: a Game Master a day's rake (the week's average), an Ambassador a quarter of it.").color(theme::MUTED()).size(13.5));
        ui.add_space(8.0);
        ui.label(RichText::new(format!("Raked this week: {} chips", chips_text(v.rake_week))).size(14.0));
        let limit = gift_limit(s);
        if limit.is_some_and(|m| m >= NO_LIMIT) {
            ui.label(RichText::new("You're the Owner of the Game: you give as many chips as you like, on every server. Every gift goes in the log below.").strong().size(15.0));
        } else if let Some(r) = v.me {
            ui.label(RichText::new(format!("You're a {}. Left to give today: {} of {} chips.", r.name(), chips_text(v.left), chips_text(v.allowance))).strong().size(15.0));
        }
        ui.add_space(12.0);

        // Game Masters (and the owner): a gift.
        if let Some(limit) = limit {
            ui.label(RichText::new("Give a gift").strong().size(16.0));
            let mut g = self.poker.giving.clone();
            ui.horizontal(|ui| {
                // (The owner can give to themselves too; nobody else can.)
                let owner = owner(s);
                let mine = |u: &str| s.me.as_ref().is_some_and(|m| m.user_id == u);
                let who = g.to.as_ref().and_then(|u| s.members.get(u.as_str()).map(|m| if mine(u) { format!("{} (you)", m.name) } else { m.name.clone() })).unwrap_or_else(|| "Pick someone".into());
                egui::ComboBox::from_id_salt("gift-to").selected_text(who).width(200.0).show_ui(ui, |ui| {
                    if owner && let Some(me) = s.me.as_ref() {
                        ui.selectable_value(&mut g.to, Some(me.user_id.to_string()), format!("{} (you)", s.members.get(me.user_id.as_str()).map(|m| m.name.as_str()).unwrap_or("Me")));
                    }
                    let mut ms: Vec<_> = s.members.iter().filter(|(u, _)| !mine(u)).collect();
                    ms.sort_by_key(|(_, m)| m.name.to_lowercase());
                    for (u, m) in ms {
                        ui.selectable_value(&mut g.to, Some(u.to_string()), &m.name);
                    }
                });
                chips_box(ui, &mut g.chips, limit);
            });
            ui.horizontal(|ui| {
                look_picker(ui, "gift-look", &mut g.item);
                ui.add(egui::TextEdit::singleline(&mut g.note).hint_text("A note (optional)").desired_width(260.0).char_limit(140));
                let ok = g.to.is_some() && (g.chips > 0 || g.item.is_some());
                if ui.add_enabled(ok, theme::primary_button("Give")).clicked()
                    && let Some(to) = g.to.clone()
                {
                    self.engine.poker(&s.key, ToServer::Gift { uid: to, chips: g.chips, item: g.item.clone(), note: g.note.clone() });
                    g = Default::default();
                }
            });
            self.poker.giving = g;
            ui.add_space(14.0);
        }

        // The staff, and (for the owner) naming them.
        ui.label(RichText::new("Staff").strong().size(16.0));
        if v.staff.is_empty() {
            ui.label(RichText::new("Nobody yet.").color(theme::MUTED()));
        }
        for m in &v.staff {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&m.name).strong().size(14.5));
                ui.label(RichText::new(m.role.name()).color(if m.role == Role::GameMaster { theme::EMBER2() } else { Color32::from_rgb(0x7a, 0xb8, 0xff) }));
                if v.owner && ui.small_button("Remove").clicked() {
                    self.engine.poker(&s.key, ToServer::SetRole { uid: m.uid.clone(), role: None });
                }
            });
        }
        if v.owner {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let pick = &mut self.poker.staff_pick;
                let who = pick.as_ref().and_then(|u| s.members.get(u.as_str())).map(|m| m.name.clone()).unwrap_or_else(|| "Pick someone".into());
                egui::ComboBox::from_id_salt("staff-pick").selected_text(who).width(200.0).show_ui(ui, |ui| {
                    let mut ms: Vec<_> = s.members.iter().collect();
                    ms.sort_by_key(|(_, m)| m.name.to_lowercase());
                    for (u, m) in ms {
                        ui.selectable_value(pick, Some(u.to_string()), &m.name);
                    }
                });
                // (Game Masters are named on the home server only, and count everywhere.)
                let home = s.key == pyr_poker::net::HOME_SERVER;
                let roles: &[Role] = if home { &[Role::GameMaster, Role::Ambassador] } else { &[] };
                for &r in roles {
                    if ui.add_enabled(pick.is_some(), egui::Button::new(format!("Make {}", r.name()))).clicked()
                        && let Some(u) = pick.take()
                    {
                        self.engine.poker(&s.key, ToServer::SetRole { uid: u, role: Some(r) });
                    }
                }
            });
            if s.key != pyr_poker::net::HOME_SERVER {
                ui.label(RichText::new("Game Masters and Ambassadors are named on Brethren of Blood, Pyr Casino's home, and are staff on every server.").color(theme::MUTED()).size(12.5));
            }
            ui.add_space(14.0);
            ui.label(RichText::new("What they gave").strong().size(16.0));
            if v.log.is_empty() {
                ui.label(RichText::new("Nothing yet.").color(theme::MUTED()));
            }
            // (Tall enough to read: inside the page's own scrolling it would shrink to what's left
            // on the screen.)
            egui::ScrollArea::vertical().id_salt("staff-log").min_scrolled_height(320.0).max_height(420.0).show(ui, |ui| {
                for l in &v.log {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(ago(l.at_ms)).color(theme::DIM()).size(12.5));
                        ui.label(RichText::new(&l.by).strong().size(13.5));
                        ui.label(RichText::new(&l.what).size(13.5));
                    });
                }
            });
        }
    }

    /// A Game Master's or Ambassador's prize box (or the owner's), being made up for a table.
    pub fn prize_dialog(&mut self, ctx: &egui::Context, s: &ServerView, table: TableId, has_prize: bool) {
        let Some(g) = self.poker.prize_for.filter(|t| *t == table).map(|_| self.poker.giving.clone()) else { return };
        let left = prize_limit(s).unwrap_or(0);
        let mut g = g;
        let mut open = true;
        let mut send = false;
        egui::Window::new("Drop a prize box").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
            ui.label(RichText::new("Whoever wins the hand it lands on opens it, a person or a bot (a bot's chips go on its stack).").color(theme::MUTED()));
            ui.add_space(6.0);
            ui.horizontal(|ui| chips_box(ui, &mut g.chips, left));
            let words = limit_words(left);
            let mut c = words.chars();
            let words = c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default();
            ui.label(RichText::new(words).size(12.5).color(theme::DIM()));
            look_picker(ui, "prize-look", &mut g.item);
            ui.add_space(8.0);
            // (One box to a table: one dropped by someone else while this was open, or this one
            // sent twice, was turned away as an error, bug report 129.)
            if has_prize {
                ui.label(RichText::new("This table has a prize box on it now: it goes to the next hand's winner. Drop yours once it's been opened.").color(theme::MUTED()));
            }
            if ui.add_enabled(!has_prize && (g.chips > 0 || g.item.is_some()), theme::primary_button("Drop it on the table")).clicked() {
                send = true;
            }
        });
        if send {
            self.engine.poker(&s.key, ToServer::DropPrize { table, chips: g.chips, item: g.item.clone() });
            self.poker.giving = Default::default();
            self.poker.prize_for = None;
            return;
        }
        self.poker.giving = g;
        if !open {
            self.poker.prize_for = None;
        }
    }
}

/// A prize box on the felt: a glowing gift box, and what's in it.
pub fn paint_prize(p: &egui::Painter, at: Pos2, prize: &PrizeView, t: f32) {
    let bob = (t * 2.2).sin() * 3.0;
    let c = at + vec2(0.0, bob);
    for k in 0..6 {
        p.circle_filled(c, 30.0 + k as f32 * 6.0, Color32::from_rgba_unmultiplied(0xff, 0xc8, 0x4a, 10));
    }
    let body = Rect::from_center_size(c + vec2(0.0, 7.0), vec2(44.0, 32.0));
    let lid = Rect::from_center_size(c + vec2(0.0, -12.0), vec2(52.0, 12.0));
    let red = Color32::from_rgb(0xc8, 0x2a, 0x3a);
    let gold = Color32::from_rgb(0xff, 0xd2, 0x4a);
    p.rect_filled(body, theme::r(4), red);
    p.rect_filled(lid, theme::r(3), Color32::from_rgb(0xe0, 0x3a, 0x4a));
    p.rect_filled(Rect::from_center_size(body.center(), vec2(8.0, body.height())), 0, gold);
    p.rect_filled(Rect::from_center_size(lid.center(), vec2(8.0, lid.height())), 0, gold);
    p.circle_stroke(lid.center_top() + vec2(-7.0, -4.0), 6.0, Stroke::new(3.0f32, gold));
    p.circle_stroke(lid.center_top() + vec2(7.0, -4.0), 6.0, Stroke::new(3.0f32, gold));
    let words = format!("Prize box: {}", pyr_engine::poker::prize_words(prize));
    let r = Rect::from_center_size(c + vec2(0.0, 40.0), vec2(30.0 + 7.4 * words.chars().count() as f32, 24.0));
    p.rect_filled(r, theme::r(12), Color32::from_rgba_unmultiplied(0x14, 0x10, 0x0a, 225));
    p.rect_stroke(r, theme::r(12), Stroke::new(1.0f32, gold), StrokeKind::Inside);
    p.text(r.center(), egui::Align2::CENTER_CENTER, words, theme::bold_font(12.5), Color32::from_rgb(0xff, 0xe2, 0x9a));
}
