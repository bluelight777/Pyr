//! Invites in the corner (0.13.6): to someone's voice channel ("Invite to 🔊 Lounge" on their
//! name) or to their table ("+ Invite"). Each is a card in the bottom right corner of Pyr,
//! wherever you are in it, with Not now and Join, for a minute, its bar running down: 0.13.28's
//! card, measured off its screen (332 by 126, 16 from the right and 4 from the bottom). Better
//! than 0.13.28: a newer card under an older one no longer covers its edge (they overlapped by
//! four); they stand 10 apart.

use std::time::Duration;

use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, pos2, vec2};
use pyr_engine::state::AppState;
use pyr_poker::casino::Game;
use pyr_poker::net::TableRef;
use pyr_proto::control::ChannelId;

use crate::app::{Act, PyrApp, avatar};
use crate::theme;

/// How long a card stays (seconds).
const LASTS: f32 = 60.0;
const W: f32 = 332.0;
const H: f32 = 126.0;
/// Inside the frame (its line and margin, 15 a side).
const IN_W: f32 = W - 30.0;
const IN_H: f32 = H - 30.0;
/// The buttons' width, two to a card, 8 apart.
const BUTTON_W: f32 = (IN_W - 8.0) / 2.0;

#[derive(Clone)]
enum What {
    Voice(ChannelId),
    Table(TableRef, String),
}

/// What a card says under the inviter's name.
fn says(what: &What, channel: &str) -> String {
    match what {
        What::Voice(_) => format!("invites you to 🔊 {channel}"),
        What::Table(t, name) => format!("invites you to {}: {name}", t.game_name()),
    }
}

impl PyrApp {
    /// Whether I'm at that table now (an invite to it then goes).
    fn at_table(&self, key: &str, table: TableRef) -> bool {
        if self.poker.open.as_deref() != Some(key) {
            return false;
        }
        let c = &self.poker.casino;
        let here = |a: &Option<(String, u64)>, id: u64| a.as_ref().is_some_and(|(k, i)| k == key && *i == id);
        match table {
            TableRef::Poker(id) => self.poker.table == Some(id),
            TableRef::Blackjack(id) => self.poker.table.is_none() && c.game == Some(Game::Blackjack) && here(&c.bjt.at, id),
            TableRef::Roulette(id) => self.poker.table.is_none() && c.game == Some(Game::Roulette) && here(&c.rlt.at, id),
            TableRef::Baccarat(id) => self.poker.table.is_none() && c.game == Some(Game::Baccarat) && here(&c.bact.at, id),
            TableRef::Craps(id) => self.poker.table.is_none() && c.game == Some(Game::Craps) && here(&c.crt.at, id),
            TableRef::Domino(id) => self.poker.table.is_none() && c.game == Some(Game::Domino) && here(&c.domt.at, id),
        }
    }

    /// Join from a table invite: that server's Pyr Casino, at that table (off any other first).
    pub(crate) fn go_to_table(&mut self, key: &str, table: TableRef) {
        self.ui.home = false;
        self.ui.plugins_open = false;
        self.ui.server = Some(key.to_string());
        self.ui.call_view = false;
        if self.poker.open.as_deref() != Some(key) {
            self.open_poker(key);
        }
        if let Some(old) = self.poker.table.take() {
            self.engine.poker(key, pyr_poker::net::ToServer::Unwatch { table: old });
        }
        self.poker.tourney = None;
        self.poker.tab = crate::poker::Tab::Floor;
        self.bj_table_leave();
        self.craps_table_leave();
        self.bac_table_leave();
        self.rl_table_leave();
        self.dom_table_leave();
        let c = &mut self.poker.casino;
        c.hold = None;
        c.shown_chips = None;
        let at = Some((key.to_string(), 0));
        let at = |id: u64| at.clone().map(|(k, _)| (k, id));
        match table {
            TableRef::Poker(id) => {
                c.game = None;
                self.open_table(key, id);
            }
            TableRef::Blackjack(id) => {
                c.game = Some(Game::Blackjack);
                c.bjt.at = at(id);
            }
            TableRef::Roulette(id) => {
                c.game = Some(Game::Roulette);
                c.rlt.at = at(id);
            }
            TableRef::Baccarat(id) => {
                c.game = Some(Game::Baccarat);
                c.bact.at = at(id);
            }
            TableRef::Craps(id) => {
                c.game = Some(Game::Craps);
                c.crt.at = at(id);
            }
            TableRef::Domino(id) => {
                c.game = Some(Game::Domino);
                c.domt.at = at(id);
            }
        }
    }

    /// The invite cards: the newest lowest, three at most.
    pub(crate) fn invite_cards(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) {
        let mut shown = Vec::new();
        for s in &st.servers {
            let here = st.call.as_ref().filter(|c| c.server_key == s.key).map(|c| c.channel_id);
            for (channel, from, since) in &s.voice_invites {
                // Gone after a minute, and once I'm in that channel anyway.
                if since.elapsed().as_secs_f32() > LASTS || here == Some(*channel) || s.channel(*channel).is_none() {
                    acts.push(Act::AnswerVoiceInvite { key: s.key.clone(), channel: *channel, from: from.clone(), join: false });
                    continue;
                }
                shown.push((s, What::Voice(*channel), from.clone(), *since));
            }
            for (table, from, name, since) in &s.poker.table_invites {
                // Gone after a minute, and once I'm at that table anyway.
                if since.elapsed().as_secs_f32() > LASTS || self.at_table(&s.key, *table) {
                    acts.push(Act::AnswerTableInvite { key: s.key.clone(), table: *table, from: from.clone(), join: false });
                    continue;
                }
                shown.push((s, What::Table(*table, name.clone()), from.clone(), *since));
            }
        }
        shown.sort_by_key(|x| x.3);
        let screen = ctx.content_rect();
        let mut bottom = screen.bottom() - 4.0;
        for (s, what, from, since) in shown.into_iter().rev().take(3) {
            let age = since.elapsed().as_secs_f32();
            let name = s.member_name(&from);
            let face = self.face_moving(ctx, s, &from, false);
            let channel = match &what {
                What::Voice(c) => s.channel(*c).map(|c| c.name.clone()).unwrap_or_default(),
                What::Table(..) => String::new(),
            };
            let mut under = says(&what, &channel);
            if st.servers.len() > 1 {
                under = format!("{under} · {}", s.name);
            }
            // Slides in from the right.
            let ease = 1.0 - (1.0 - (age / 0.28).clamp(0.0, 1.0)).powi(3);
            let pos = pos2(screen.right() - 16.0 - W + (1.0 - ease) * 40.0, bottom - H);
            bottom -= H + 10.0;
            let id = match &what {
                What::Voice(c) => egui::Id::new(("voice-invite", &s.key, *c, &from)),
                What::Table(t, _) => egui::Id::new(("table-invite", &s.key, *t, &from)),
            };
            let mut answer: Option<bool> = None;
            let card = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
                ui.set_opacity(ease);
                egui::Frame::new()
                    .fill(theme::ELEV())
                    .stroke(Stroke::new(1.0_f32, theme::EMBER().gamma_multiply(0.6)))
                    .corner_radius(14)
                    .shadow(egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) })
                    .inner_margin(14)
                    .show(ui, |ui| {
                        let (inner, _) = ui.allocate_exact_size(vec2(IN_W, IN_H), Sense::hover());
                        let p = ui.painter();
                        let row = Rect::from_min_size(inner.min, vec2(IN_W, 46.0));
                        avatar(p, pos2(row.left() + 22.0, row.center().y), 21.0, &from, &name, false, theme::ELEV(), face);
                        let x = row.left() + 54.0;
                        let room = row.right() - x;
                        crate::emoji::paint_text(p, pos2(x, row.center().y - 11.0), Align2::LEFT_CENTER, &name, theme::bold_font(15.0), theme::TEXT(), Some(room));
                        crate::emoji::paint_text(p, pos2(x, row.center().y + 10.0), Align2::LEFT_CENTER, &under, theme::font(12.5), theme::MUTED(), Some(room));
                        // How long it stays.
                        let left = (1.0 - age / LASTS).clamp(0.0, 1.0);
                        let bar = Rect::from_min_size(pos2(inner.left(), row.bottom() + 4.0), vec2(IN_W * left, 2.0));
                        p.rect_filled(bar, 1.0, theme::EMBER().gamma_multiply(0.7));
                        let y = bar.bottom() + 12.0;
                        for (i, join) in [(0.0, false), (1.0, true)] {
                            let r = Rect::from_min_size(pos2(inner.left() + i * (BUTTON_W + 8.0), y), vec2(BUTTON_W, 32.0));
                            let resp = ui.interact(r, id.with(join), Sense::click());
                            let (fill, ink) = if join { (theme::EMBER(), theme::on_ember()) } else { (theme::ELEV2(), theme::TEXT()) };
                            let fill = if resp.hovered() { theme::mix(fill, Color32::WHITE, 0.10) } else { fill };
                            let p = ui.painter();
                            p.rect_filled(r, theme::r(9), fill);
                            p.text(r.center(), Align2::CENTER_CENTER, if join { "Join" } else { "Not now" }, theme::bold_font(13.5), ink);
                            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                answer = Some(join);
                            }
                        }
                    });
            });
            if let Some(join) = answer {
                match &what {
                    What::Voice(c) => acts.push(Act::AnswerVoiceInvite { key: s.key.clone(), channel: *c, from: from.clone(), join }),
                    What::Table(t, _) => acts.push(Act::AnswerTableInvite { key: s.key.clone(), table: *t, from: from.clone(), join }),
                }
            }
            // (A program open inside Pyr keeps clear of it.)
            self.ui.overlay_rects.push(card.response.rect);
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the cards say, as 0.13.28's do.
    #[test]
    fn cards_say_what_0_13_28_says() {
        assert_eq!(says(&What::Table(TableRef::Roulette(3), "Grand Salle".into()), ""), "invites you to roulette: Grand Salle");
        assert_eq!(says(&What::Table(TableRef::Poker(1), "Carol's table".into()), ""), "invites you to poker: Carol's table");
        assert_eq!(says(&What::Voice(4), "Lounge"), "invites you to 🔊 Lounge");
        assert_eq!(IN_W, 302.0);
        assert_eq!(BUTTON_W, 147.0);
    }
}
