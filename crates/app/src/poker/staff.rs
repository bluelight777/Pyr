//! Pyr Poker's staff: Game Masters (the owner's admins for the game: they gift chips and looks,
//! and drop prize boxes on tables) and Ambassadors (prize boxes only). The owner names them and
//! reads what they gave; each gives at most a day's allowance, which follows the rake.

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

fn chips_box(ui: &mut egui::Ui, chips: &mut u64, max: u64) {
    ui.add(egui::DragValue::new(chips).range(0..=max.max(0)).speed(100.0).suffix(" chips"));
    for n in [1_000u64, 5_000, 25_000] {
        if n <= max && ui.small_button(format!("+{}", chips_text(n))).clicked() {
            *chips = (*chips + n).min(max);
        }
    }
}

impl PyrApp {
    pub fn my_poker_role(&self, s: &ServerView) -> Option<Role> {
        s.poker.me.as_ref().and_then(|m| m.role)
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
        ui.label(RichText::new("Game Masters run the game with you: they gift chips and looks, and drop prize boxes on tables. Ambassadors drop prize boxes (whoever wins the next hand opens it). Each gives at most a day's allowance, which follows the rake: a Game Master a day's rake (the week's average), an Ambassador a quarter of it.").color(theme::MUTED()).size(13.5));
        ui.add_space(8.0);
        ui.label(RichText::new(format!("Raked this week: {} chips", chips_text(v.rake_week))).size(14.0));
        if let Some(r) = v.me {
            ui.label(RichText::new(format!("You're a {}. Left to give today: {} of {} chips.", r.name(), chips_text(v.left), chips_text(v.allowance))).strong().size(15.0));
        }
        ui.add_space(12.0);

        // Game Masters: a gift.
        if v.me == Some(Role::GameMaster) {
            ui.label(RichText::new("Give a gift").strong().size(16.0));
            let mut g = self.poker.giving.clone();
            ui.horizontal(|ui| {
                let who = g.to.as_ref().and_then(|u| s.members.get(u.as_str())).map(|m| m.name.clone()).unwrap_or_else(|| "Pick someone".into());
                egui::ComboBox::from_id_salt("gift-to").selected_text(who).width(200.0).show_ui(ui, |ui| {
                    let mut ms: Vec<_> = s.members.iter().filter(|(u, _)| s.me.as_ref().is_none_or(|m| m.user_id != **u)).collect();
                    ms.sort_by_key(|(_, m)| m.name.to_lowercase());
                    for (u, m) in ms {
                        ui.selectable_value(&mut g.to, Some(u.to_string()), &m.name);
                    }
                });
                chips_box(ui, &mut g.chips, v.left);
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
                for r in [Role::GameMaster, Role::Ambassador] {
                    if ui.add_enabled(pick.is_some(), egui::Button::new(format!("Make {}", r.name()))).clicked()
                        && let Some(u) = pick.take()
                    {
                        self.engine.poker(&s.key, ToServer::SetRole { uid: u, role: Some(r) });
                    }
                }
            });
            ui.add_space(14.0);
            ui.label(RichText::new("What they gave").strong().size(16.0));
            if v.log.is_empty() {
                ui.label(RichText::new("Nothing yet.").color(theme::MUTED()));
            }
            egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
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

    /// A Game Master's or Ambassador's prize box, being made up for a table.
    pub fn prize_dialog(&mut self, ctx: &egui::Context, s: &ServerView, table: TableId) {
        let Some(g) = self.poker.prize_for.filter(|t| *t == table).map(|_| self.poker.giving.clone()) else { return };
        let left = s.poker.staff.as_ref().map(|v| v.left).unwrap_or(0);
        let mut g = g;
        let mut open = true;
        let mut send = false;
        egui::Window::new("Drop a prize box").collapsible(false).resizable(false).open(&mut open).show(ctx, |ui| {
            ui.label(RichText::new("Whoever wins the next hand opens it (a person: if a bot wins, it waits for the next).").color(theme::MUTED()));
            ui.add_space(6.0);
            ui.horizontal(|ui| chips_box(ui, &mut g.chips, left));
            ui.label(RichText::new(format!("Left to give today: {} chips", chips_text(left))).size(12.5).color(theme::DIM()));
            look_picker(ui, "prize-look", &mut g.item);
            ui.add_space(8.0);
            if ui.add_enabled(g.chips > 0 || g.item.is_some(), theme::primary_button("Drop it on the table")).clicked() {
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
