//! Inviting someone to a table (0.13.6): "+ Invite" at the top of any poker, blackjack,
//! roulette, baccarat or craps table opens the list to pick them from (as 0.13.28 has it,
//! measured off its screen: "Invite to this table", Look for someone, everyone here, online
//! first). They get a card in the corner of their Pyr with Join (see `corner_invites`).
//!
//! Servers from 0.13.6 on take them (the ones with `voice_invite`, which came with it; 0.13.28's
//! server names no feature of its own for it).

use eframe::egui::{self, RichText};
use pyr_engine::state::ServerView;
use pyr_poker::casino::Game;
use pyr_poker::net::{TableRef, ToServer};
use pyr_proto::control::RelationKind;

use crate::app::PyrApp;
use crate::theme;

/// The list's width (0.13.28's: 600 with its frame) and how tall it gets before it scrolls.
const LIST_W: f32 = 586.0;
const LIST_H: f32 = 260.0;

/// Who can be invited: everyone here but me, bots and anyone I've blocked; online first, then
/// by name; with a query, the names that have it in them.
pub fn people(s: &ServerView, me: &str, query: &str) -> Vec<(String, String, bool)> {
    let q = query.trim().to_lowercase();
    let mut list: Vec<(String, String, bool)> = s
        .members
        .values()
        .filter(|m| m.user_id != me && !m.bot && s.relations.get(&m.user_id) != Some(&RelationKind::Blocked))
        .filter(|m| q.is_empty() || m.name.to_lowercase().contains(&q))
        .map(|m| (m.user_id.clone(), m.name.clone(), m.online))
        .collect();
    list.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())).then_with(|| a.0.cmp(&b.0)));
    list
}

impl PyrApp {
    /// The table I'm at in this casino game (one that has tables), if this server takes
    /// invites.
    pub(crate) fn casino_invite_table(&self, s: &ServerView, game: Game) -> Option<TableRef> {
        if !s.has(pyr_proto::control::VOICE_INVITE_FEATURE) {
            return None;
        }
        let c = &self.poker.casino;
        let at = |a: &Option<(String, u64)>| a.as_ref().filter(|(k, _)| *k == s.key).map(|(_, id)| *id);
        let tables = s.has(pyr_proto::control::CASINO_012_FEATURE);
        match game {
            Game::Blackjack if tables => at(&c.bjt.at).map(TableRef::Blackjack),
            Game::Baccarat if tables => at(&c.bact.at).map(TableRef::Baccarat),
            Game::Craps if tables => at(&c.crt.at).map(TableRef::Craps),
            Game::Roulette if s.has(pyr_proto::control::CASINO_013_FEATURE) => at(&c.rlt.at).map(TableRef::Roulette),
            Game::Domino if s.has(pyr_proto::control::DOMINO_FEATURE) => at(&c.domt.at).map(TableRef::Domino),
            _ => None,
        }
    }

    /// The list under "+ Invite" (`resp`), opened by clicking it: click someone and they're
    /// invited to `table`.
    pub(crate) fn invite_list(&mut self, resp: &egui::Response, s: &ServerView, table: TableRef) {
        if resp.clicked() {
            self.poker.casino.invite_query.clear();
        }
        // (Who I am from the server's view: the app's state is already read for this frame, and
        // reading it again here could wait on itself.)
        let me = s.me.as_ref().map(|m| m.user_id.clone()).unwrap_or_default();
        let mut chose: Option<(String, String)> = None;
        egui::Popup::menu(resp).gap(0.0).align_alternatives(&[egui::RectAlign::BOTTOM_END]).show(|ui| {
            crate::menu::style(ui);
            ui.set_width(LIST_W);
            // (6 under the heading, 2 under the box: each widget's space after it is the spacing
            // when it's put down.)
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.label(RichText::new("Invite to this table").font(theme::bold_font(14.0)).color(theme::BODY()));
            ui.spacing_mut().item_spacing.y = 2.0;
            let q = &mut self.poker.casino.invite_query;
            ui.add(egui::TextEdit::singleline(q).hint_text("Look for someone").desired_width(f32::INFINITY));
            let list = people(s, &me, q);
            egui::ScrollArea::vertical().max_height(LIST_H).show(ui, |ui| {
                for (uid, name, online) in &list {
                    let label = if *online { name.clone() } else { format!("{name} (offline)") };
                    if ui.add(egui::Button::new(label).min_size(egui::vec2(ui.available_width(), 0.0))).clicked() {
                        chose = Some((uid.clone(), name.clone()));
                        ui.close();
                    }
                }
            });
        });
        if let Some((uid, name)) = chose {
            self.engine.poker(&s.key, ToServer::InviteToTable { uid, table });
            self.toast(format!("Invited {name} to the table"), false);
        }
    }
}

#[cfg(test)]
mod tests {
    use pyr_proto::control::{Member, RelationKind, Role};

    fn member(id: &str, name: &str, online: bool, bot: bool) -> Member {
        Member { user_id: id.into(), name: name.into(), role: Role::Member, online, bot, ..Default::default() }
    }

    /// Online first, then by name; never me, a bot, or anyone I've blocked; a query narrows it.
    #[test]
    fn who_can_be_invited() {
        let mut s = pyr_engine::state::ServerView::new("k".into(), "Test Camp".into(), String::new());
        for m in [member("me", "Carol", true, false), member("b", "bob", false, false), member("o", "Olive", true, false), member("p", "PI1", true, false), member("c", "Claude", true, true), member("x", "Xena", true, false), member("a", "Amy", false, false)] {
            s.members.insert(m.user_id.clone(), m);
        }
        s.relations.insert("x".into(), RelationKind::Blocked);
        let names: Vec<String> = super::people(&s, "me", "").into_iter().map(|p| p.1).collect();
        assert_eq!(names, ["Olive", "PI1", "Amy", "bob"]);
        let names: Vec<String> = super::people(&s, "me", " o").into_iter().map(|p| p.1).collect();
        assert_eq!(names, ["Olive", "bob"]);
    }
}
