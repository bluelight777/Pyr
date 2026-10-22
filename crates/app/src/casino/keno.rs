//! Keno at the app: eighty numbers to pick from (up to ten), twenty balls coming out one by
//! one, the catches lit, and the paytable for however many you picked.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::casino::keno::{self as kn, MAX_PICKS, PAYS};
use pyr_poker::casino::{Bet, Outcome};
use pyr_poker::net::RoundView;

use super::{art, col, glow_text, vgrad};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

pub struct KenoUi {
    pub picks: Vec<u8>,
    pub chips: u64,
    show: Option<(RoundView, Instant, bool)>,
    pub turbo: bool,
    /// Playing by itself: how many rounds, or until stopped (see `auto`).
    pub auto: super::auto::Auto,
    next_at: Option<Instant>,
    seed: u64,
}

impl Default for KenoUi {
    fn default() -> Self {
        KenoUi { picks: Vec::new(), chips: 10, show: None, turbo: false, auto: Default::default(), next_at: None, seed: 0x9e37_79b9 }
    }
}

impl KenoUi {
    fn gap(&self) -> f32 {
        if self.turbo { 0.05 } else { 0.14 }
    }
}

impl PyrApp {
    pub(super) fn keno_round(&mut self, r: &RoundView) {
        let Outcome::Keno(o) = &r.outcome else { return };
        let gap = self.poker.casino.keno.gap();
        self.poker.casino.keno.show = Some((r.clone(), Instant::now(), false));
        for (i, n) in o.drawn.iter().enumerate() {
            let hit = o.picks.contains(n);
            self.poker.casino.sound_in(i as f32 * gap, if hit { Cue::Coin } else { Cue::KenoBall }, 0.0, if hit { 0.8 } else { 0.4 });
        }
    }

    fn keno_play(&mut self, s: &ServerView) {
        let c = &s.poker.casino;
        let ui = &self.poker.casino.keno;
        if ui.picks.is_empty() || ui.chips == 0 || ui.chips > super::chips_now(s) || self.poker.casino.busy() {
            self.poker.casino.keno.auto.stop();
            return;
        }
        let Some(commit) = c.commit else { return };
        self.poker.casino.pending = Some(Instant::now());
        self.poker.casino.celebration = None;
        self.engine.casino_play(&s.key, commit, Bet::Keno { chips: ui.chips, picks: ui.picks.clone() });
    }

    pub(super) fn keno_page(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let now = Instant::now();
        let p = ui.painter().clone();
        vgrad(&p, area, Color32::from_rgb(0x1a, 0x0c, 0x22), Color32::from_rgb(0x08, 0x05, 0x0e));
        let gap = self.poker.casino.keno.gap();
        let show = self.poker.casino.keno.show.clone();
        let k = show.as_ref().map(|(_, at, _)| (now - *at).as_secs_f32()).unwrap_or(99.0);
        let (drawn, hits_all): (Vec<u8>, Vec<u8>) = match &show {
            Some((r, _, _)) => match &r.outcome {
                Outcome::Keno(o) => {
                    let n = ((k / gap) as usize + 1).min(o.drawn.len());
                    (o.drawn[..n].to_vec(), o.hits.clone())
                }
                _ => (vec![], vec![]),
            },
            None => (vec![], vec![]),
        };
        let done = show.as_ref().is_none_or(|(r, _, _)| matches!(&r.outcome, Outcome::Keno(o) if k >= o.drawn.len() as f32 * gap + 0.2));
        // The board: 8 rows of 10.
        let board_w = (area.width() * 0.62).min((area.height() - 190.0) * 1.25);
        let cell = board_w / 10.0;
        let board = Rect::from_min_size(pos2(area.left() + 30.0, area.top() + 24.0), vec2(board_w, cell * 8.0));
        p.rect_filled(board.expand(10.0), theme::r(16), Color32::from_black_alpha(140));
        let picks = self.poker.casino.keno.picks.clone();
        let busy = self.poker.casino.busy() || !done;
        for n in 1..=80u8 {
            let i = (n - 1) as usize;
            let r = Rect::from_min_size(pos2(board.left() + (i % 10) as f32 * cell, board.top() + (i / 10) as f32 * cell), vec2(cell, cell)).shrink(3.0);
            let resp = ui.interact(r, ui.id().with(("keno", n)), Sense::click());
            let picked = picks.contains(&n);
            let is_drawn = drawn.contains(&n);
            let hit = picked && is_drawn;
            let fill = if hit {
                col::GOLD
            } else if picked {
                Color32::from_rgb(0x7a, 0x3a, 0xd0)
            } else if resp.hovered() && !busy {
                Color32::from_white_alpha(40)
            } else {
                Color32::from_white_alpha(16)
            };
            p.rect_filled(r, theme::r(10), fill);
            if is_drawn && !hit {
                art::ball(&p, r.center(), r.width() * 0.38, n, false);
            } else {
                p.text(r.center(), Align2::CENTER_CENTER, n.to_string(), theme::bold_font(cell * 0.32), if hit { Color32::BLACK } else { Color32::WHITE });
            }
            if hit {
                p.rect_stroke(r, theme::r(10), Stroke::new(2.0 + (k * 6.0).sin().abs() * 2.0, col::GOLD2), StrokeKind::Outside);
            }
            if !busy && resp.clicked() {
                let ui_s = &mut self.poker.casino.keno;
                if picked {
                    ui_s.picks.retain(|x| *x != n);
                } else if ui_s.picks.len() < MAX_PICKS {
                    ui_s.picks.push(n);
                }
                ui_s.show = None;
            }
        }
        // The balls out so far, in a row under the board.
        let step = ((board.width() - 8.0) / 20.0).min(36.0);
        for (i, n) in drawn.iter().enumerate() {
            let hit = picks.contains(n);
            let at = pos2(board.left() + step / 2.0 + i as f32 * step, board.bottom() + 36.0);
            art::ball(&p, at, step * 0.44, *n, hit);
        }
        // The paytable for this many picks.
        let side = Rect::from_min_max(pos2(board.right() + 34.0, board.top()), pos2(area.right() - 24.0, board.bottom()));
        p.rect_filled(side, theme::r(16), Color32::from_black_alpha(150));
        let np = picks.len();
        p.text(side.left_top() + vec2(18.0, 20.0), Align2::LEFT_CENTER, if np == 0 { "PICK 1 TO 10 NUMBERS".to_string() } else { format!("{np} PICKED · WHAT CATCHES PAY") }, theme::bold_font(13.0), col::GOLD);
        if np == 0 {
            let galley = p.layout(
                "Click numbers on the board, or Quick pick. Twenty balls come out; the more of yours they catch, the more you win.\n\nPick 1 and catch it: 3.8 times your bet.\nPick 5 and catch all five: 800 times.\nPick 10 and catch all ten: 100,000 times.".to_string(),
                theme::font(14.0),
                col::TEXT,
                side.width() - 36.0,
            );
            p.galley(side.left_top() + vec2(18.0, 44.0), galley, col::TEXT);
        }
        if np > 0 {
            let caught_now = drawn.iter().filter(|d| picks.contains(d)).count();
            let mut y = side.top() + 52.0;
            for caught in (0..=np).rev() {
                let m = PAYS[np - 1][caught];
                if m == 0 {
                    continue;
                }
                let on = done && caught == hits_all.len() && show.is_some();
                let r = Rect::from_min_size(pos2(side.left() + 12.0, y - 14.0), vec2(side.width() - 24.0, 30.0));
                if on || caught == caught_now && !done {
                    p.rect_filled(r, theme::r(8), col::GOLD.gamma_multiply(if on { 0.5 } else { 0.18 }));
                }
                p.text(pos2(side.left() + 24.0, y), Align2::LEFT_CENTER, format!("Catch {caught}"), theme::bold_font(15.0), Color32::WHITE);
                let mult = if m % 10 == 0 { format!("{}×", chips_text(m / 10)) } else { format!("{}.{}×", m / 10, m % 10) };
                p.text(pos2(side.right() - 24.0, y), Align2::RIGHT_CENTER, format!("{mult}   {}", chips_text(kn::payout(self.poker.casino.keno.chips, np, caught))), theme::bold_font(15.0), col::GOLD2);
                y += 34.0;
            }
        }
        // The result.
        if done
            && let Some((r, _, settled)) = &show
            && let Outcome::Keno(o) = &r.outcome
        {
            let line = format!("Caught {} of {}", o.hits.len(), o.picks.len());
            glow_text(&p, pos2(side.center().x, side.bottom() - 62.0), Align2::CENTER_CENTER, &line, theme::bold_font(30.0), Color32::WHITE, Color32::from_black_alpha(140));
            p.text(pos2(side.center().x, side.bottom() - 26.0), Align2::CENTER_CENTER, if o.paid > 0 { format!("You won {}", chips_text(o.paid)) } else { "No win this time".into() }, theme::bold_font(21.0), if o.paid > 0 { col::WIN } else { col::MUTED });
            if !*settled {
                // Called out over the board too, the size a stream's viewers can read.
                let (title, color) = if o.paid > 0 { (format!("{} OF {} · WIN {}", o.hits.len(), o.picks.len(), chips_text(o.paid)), col::GOLD2) } else { (format!("{} OF {}", o.hits.len(), o.picks.len()), col::TEXT) };
                self.poker.casino.announce(title, if o.paid > 0 { String::new() } else { "No win this time".to_string() }, color, (0.32, 0.42), 0.1);
                let ui_s = &mut self.poker.casino.keno;
                if let Some(sh) = ui_s.show.as_mut() {
                    sh.2 = true;
                }
                // (A big win, 50 times the bet and more, stops it when that's set.)
                if ui_s.auto.spun(o.paid >= o.chips * 50) {
                    ui_s.next_at = Some(now + Duration::from_millis(if o.paid > 0 { 1500 } else { 700 }));
                }
                if o.paid > 0 {
                    self.poker.casino.sound_in(0.0, Cue::WinSmall, 0.0, 0.8);
                }
                self.poker.casino.settle_balance();
                self.poker.casino.celebrate(o.paid, o.chips, false, 0.0);
            }
        }
        if self.poker.casino.keno.auto.on() && self.poker.casino.keno.next_at.is_some_and(|t| now >= t) {
            self.poker.casino.keno.next_at = None;
            self.keno_play(s);
        }
        // The controls.
        let cb = Rect::from_min_max(pos2(area.left() + 20.0, area.bottom() - 84.0), pos2(area.right() - 20.0, area.bottom() - 14.0));
        p.rect_filled(cb, theme::r(18), Color32::from_black_alpha(170));
        let bet = self.poker.casino.keno.chips;
        let picked = self.poker.casino.chip;
        let auto = self.poker.casino.keno.auto;
        let mut x = cb.right() - 16.0;
        let mut button_resp = |ui: &mut egui::Ui, text: &str, w: f32, primary: bool, enabled: bool| -> egui::Response {
            let r = Rect::from_min_size(pos2(x - w, cb.center().y - 22.0), vec2(w, 44.0));
            x -= w + 10.0;
            let resp = ui.interact(r, ui.id().with(("kn", text)), Sense::click());
            let fill = if !enabled { Color32::from_white_alpha(18) } else if primary { if resp.hovered() { col::GOLD2 } else { col::GOLD } } else if resp.hovered() { Color32::from_white_alpha(46) } else { Color32::from_white_alpha(24) };
            ui.painter().rect_filled(r, theme::r(12), fill);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(15.0), if primary && enabled { Color32::from_rgb(0x2a, 0x10, 0x04) } else if enabled { Color32::WHITE } else { Color32::from_white_alpha(70) });
            resp
        };
        let space = ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) && !ui.ctx().wants_keyboard_input();
        let ok = !picks.is_empty() && bet > 0 && bet <= super::chips_now(s) && !busy;
        macro_rules! button {
            ($ui:expr, $t:expr, $w:expr, $p:expr, $e:expr) => {{
                let en = $e;
                button_resp($ui, $t, $w, $p, en).clicked() && en
            }};
        }
        if auto.on() {
            if button!(ui, &format!("Stop ({})", auto.left_text()), 130.0, true, true) {
                self.poker.casino.keno.auto.stop();
            }
        } else if button!(ui, "Play", 130.0, true, ok) || (space && ok) {
            self.keno_play(s);
        }
        // Playing by itself: 10 to 1,000 rounds, or until it's stopped (its menu).
        let resp = button_resp(ui, "Auto ⌄", 96.0, false, !auto.on() && ok);
        if !auto.on() && ok && let Some(n) = super::auto::menu(&resp, &mut self.poker.casino.keno.auto) {
            self.poker.casino.keno.auto.left = n;
            self.poker.casino.keno.next_at = Some(now);
        }
        let turbo = self.poker.casino.keno.turbo;
        if button!(ui, if turbo { "Turbo on" } else { "Turbo" }, 96.0, false, true) {
            self.poker.casino.keno.turbo = !turbo;
        }
        if button!(ui, "Quick pick", 110.0, false, !busy) {
            // Ten numbers (or as many as are picked now) at random.
            let ui_s = &mut self.poker.casino.keno;
            let want = if ui_s.picks.is_empty() { 10 } else { ui_s.picks.len() };
            ui_s.picks.clear();
            let mut x = ui_s.seed ^ (now.elapsed().as_nanos() as u64) ^ (std::process::id() as u64) ^ (ui.ctx().input(|i| i.time) * 1e6) as u64;
            while ui_s.picks.len() < want {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let n = (x % 80) as u8 + 1;
                if !ui_s.picks.contains(&n) {
                    ui_s.picks.push(n);
                }
            }
            ui_s.seed = x;
            ui_s.show = None;
        }
        if button!(ui, "Clear", 80.0, false, !busy && !picks.is_empty()) {
            self.poker.casino.keno.picks.clear();
            self.poker.casino.keno.show = None;
        }
        if button!(ui, &format!("+{}", chips_text(picked.min(1_000))), 86.0, false, !busy) {
            self.poker.casino.keno.chips = (bet + picked).min(1_000);
        }
        // The bet (right-click it for the least).
        let color = if bet > super::chips_now(s) { col::LOSE } else { col::GOLD2 };
        if let Some(br) = self.rack_and_bet(ui, cb, x, 500, format!("Bet {}", chips_text(bet)), color) {
            let bresp = ui.interact(br.expand2(vec2(6.0, 8.0)), ui.id().with("kn-bet"), Sense::click());
            if bresp.on_hover_text("Right-click: back to 1").secondary_clicked() && !busy {
                self.poker.casino.keno.chips = 1;
            }
        }
    }
}
