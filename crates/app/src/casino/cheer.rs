//! Someone's big win, as it happens: a banner sliding in at the top of the casino's pages (0.13.0)
//! and, for a big big win (a jackpot, 100 times the bet for 10,000 chips or more, a million
//! chips up), its moment (0.13.7): Congrats! and Buy a drink · 5,000 on the banner for everyone
//! else, and a toast for the winner as each comes. As 0.13.28 has them.

use std::time::Instant;

use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::state::ServerView;
use pyr_poker::net::{CHEER_DRINK_CHIPS, CasinoReq, ToServer};

use super::{col, ease_out};
use crate::app::{Act, PyrApp};
use crate::poker::cards::chips_text;
use crate::theme;

/// How long a banner shows: a big win's five seconds (0.13.0's), its moment's twelve.
const SHOWN: f32 = 5.0;
const SHOWN_MOMENT: f32 = 12.0;
/// Sliding in (from 48 points up) and out, at either end.
const SLIDE: f32 = 0.35;
/// The buttons: their height, their width by their letters (as the banner's own, 0.13.28's:
/// Congrats! 91, Buy a drink · 5,000 163), between them, after them, and before the first (after
/// the words' room).
const BUTTON_H: f32 = 26.0;
const BUTTON_LETTER: f32 = 7.2;
const BUTTON_PAD: f32 = 26.0;
const BUTTON_GAP: f32 = 8.0;
const BUTTON_END: f32 = 10.0;
const BUTTON_LEAD: f32 = 4.0;
/// A button not pointed at (Pyr gold, a little dimmed), and one said and done.
const BUTTON: Color32 = Color32::from_rgb(214, 169, 70);
const BUTTON_DONE: Color32 = Color32::from_rgba_premultiplied(18, 18, 18, 18);

/// The banner on now: since when, its words, and its moment (`BigWin::cheer`, 0 for none).
#[derive(Clone, Debug)]
pub struct WinBanner {
    pub since: Instant,
    pub text: String,
    pub cheer: u64,
}

impl PyrApp {
    /// Someone's big win, just now: the banner at the top of the casino's page (`area`: the page
    /// under its bar), for a few seconds; with its moment's buttons if it has one. My own wins
    /// have the celebration instead.
    pub(crate) fn big_win_banner(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let c = &s.poker.casino;
        if c.big_win_seq != self.poker.casino.seen_big_win {
            self.poker.casino.seen_big_win = c.big_win_seq;
            let mine = s.me.as_ref().map(|m| m.name.clone());
            if let Some(w) = c.big_wins.first()
                && Some(&w.name) != mine.as_ref()
            {
                let text = format!("{} just won {} on {} ({})", w.name, chips_text(w.paid), w.game.name(), w.what);
                self.poker.casino.win_banner = Some(WinBanner { since: Instant::now(), text, cheer: w.cheer });
            }
        }
        // (0.13.28's way to see it: the first time the casino opens, someone's big big win.)
        if !self.poker.casino.demo_cheer_shown && std::env::var("PYR_DEMO_CHEER").is_ok_and(|v| v == "1") {
            self.poker.casino.demo_cheer_shown = true;
            self.poker.casino.win_banner = Some(WinBanner { since: Instant::now(), text: "Mara just won 1,212,500 on Dice (49.5x)".into(), cheer: u64::MAX });
        }
        let Some(b) = self.poker.casino.win_banner.clone() else { return };
        let shown = if b.cheer != 0 { SHOWN_MOMENT } else { SHOWN };
        let t = b.since.elapsed().as_secs_f32();
        if t > shown {
            self.poker.casino.win_banner = None;
            return;
        }
        ui.ctx().request_repaint();
        let slide = ease_out((t / SLIDE).min(1.0)) * ((shown - t) / SLIDE).clamp(0.0, 1.0);
        // The words' room (0.13.0's: a width by how many letters), then the buttons'.
        let lead = 56.0 + b.text.chars().count() as f32 * 7.8;
        let sent = c.cheers_sent.get(&b.cheer).copied().unwrap_or_default();
        let font = theme::bold_font(12.5);
        let buttons: Vec<(String, bool, f32)> = if b.cheer == 0 {
            Vec::new()
        } else {
            let words = [
                if sent.0 { "Congrats sent".to_string() } else { "Congrats!".to_string() },
                if sent.1 { "Drink bought".to_string() } else { format!("Buy a drink · {}", chips_text(CHEER_DRINK_CHIPS)) },
            ];
            let done = [sent.0, sent.1];
            words.into_iter().zip(done).map(|(w, d)| {
                let width = w.chars().count() as f32 * BUTTON_LETTER + BUTTON_PAD;
                (w, d, width)
            }).collect()
        };
        let room = if buttons.is_empty() { 0.0 } else { BUTTON_LEAD + buttons.iter().map(|b| b.2).sum::<f32>() + BUTTON_GAP * (buttons.len() - 1) as f32 + BUTTON_END };
        let r = Rect::from_center_size(pos2(area.center().x, area.top() - 24.0 + 48.0 * slide), vec2(lead + room, 36.0));
        let p = ui.painter().clone();
        p.rect_filled(r, theme::r(18), Color32::from_rgba_unmultiplied(0x2a, 0x10, 0x04, 235));
        // (A moment's banner has the heavier rim.)
        p.rect_stroke(r, theme::r(18), Stroke::new(if b.cheer != 0 { 2.0f32 } else { 1.5 }, col::GOLD), StrokeKind::Inside);
        crate::icons::flame(&p, Rect::from_center_size(r.left_center() + vec2(20.0, 0.0), vec2(18.0, 18.0)));
        p.text(pos2(r.left() + lead / 2.0 + 10.0, r.center().y), Align2::CENTER_CENTER, &b.text, theme::bold_font(13.5), col::GOLD2);
        let mut x = r.left() + lead + BUTTON_LEAD;
        for (k, (words, done, width)) in buttons.into_iter().enumerate() {
            let br = Rect::from_min_size(pos2(x, r.center().y - BUTTON_H / 2.0), vec2(width, BUTTON_H));
            x += width + BUTTON_GAP;
            let resp = ui.interact(br, egui::Id::new(("casino-cheer", b.cheer, k)), if done { Sense::hover() } else { Sense::click() });
            if done {
                p.rect_filled(br, theme::r(13), BUTTON_DONE);
                p.text(br.center(), Align2::CENTER_CENTER, words, font.clone(), col::MUTED);
                continue;
            }
            let resp = resp.on_hover_text(if k == 0 { "Tell them: Congrats!".to_string() } else { format!("{} of your chips go to them, with your name on it.", chips_text(CHEER_DRINK_CHIPS)) });
            p.rect_filled(br, theme::r(13), if resp.hovered() { col::GOLD } else { BUTTON });
            p.text(br.center(), Align2::CENTER_CENTER, words, font.clone(), col::NIGHT);
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if resp.clicked() {
                self.engine.poker(&s.key, ToServer::Casino { req: CasinoReq::Cheer { cheer: b.cheer, drink: k == 1 } });
            }
        }
    }

    /// Congrats! and drinks for my big wins, as they come: a toast each, wherever I am in the
    /// casino (0.13.28's words).
    pub(crate) fn cheers_heard(&mut self, s: &ServerView, acts: &mut Vec<Act>) {
        let c = &s.poker.casino;
        if c.cheered_seq == self.poker.casino.seen_cheered {
            return;
        }
        let new = (c.cheered_seq.saturating_sub(self.poker.casino.seen_cheered) as usize).min(c.cheered.len());
        self.poker.casino.seen_cheered = c.cheered_seq;
        for (from, drink, chips) in &c.cheered[c.cheered.len() - new..] {
            acts.push(Act::Toast(if *drink { format!("{from} bought you a drink! +{} chips", chips_text(*chips)) } else { format!("{from}: Congrats!") }, false));
        }
    }
}
