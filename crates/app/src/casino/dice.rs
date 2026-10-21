//! Dice at the app: a slider from 0 to 100 to pick the chance (under or over), the
//! multiplier it pays, and the roll racing to its number.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::casino::dice::{self as dc, MAX_CHANCE, MIN_CHANCE};
use pyr_poker::casino::{Bet, Outcome};
use pyr_poker::net::RoundView;

use super::{col, glow_text, radial, times, vgrad};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

const ROLL_SECS: f32 = 0.7;

pub struct DiceUi {
    pub chips: u64,
    pub chance: u16,
    pub over: bool,
    show: Option<(RoundView, Instant, bool)>,
    pub history: Vec<(u16, bool)>,
    pub auto_left: u32,
    next_at: Option<Instant>,
}

impl Default for DiceUi {
    fn default() -> Self {
        DiceUi { chips: 10, chance: 5_000, over: false, show: None, history: Vec::new(), auto_left: 0, next_at: None }
    }
}

impl PyrApp {
    pub(super) fn dice_round(&mut self, r: &RoundView) {
        let Outcome::Dice(_) = &r.outcome else { return };
        self.poker.casino.dice.show = Some((r.clone(), Instant::now(), false));
        self.poker.casino.sound_in(0.0, Cue::DiceRoll, 0.0, 0.5);
    }

    fn dice_roll(&mut self, s: &ServerView) {
        let c = &s.poker.casino;
        let ui = &self.poker.casino.dice;
        if ui.chips == 0 || ui.chips > super::chips_now(s) || self.poker.casino.busy() {
            self.poker.casino.dice.auto_left = 0;
            return;
        }
        let Some(commit) = c.commit else { return };
        self.poker.casino.pending = Some(Instant::now());
        self.poker.casino.celebration = None;
        self.engine.casino_play(&s.key, commit, Bet::Dice { chips: ui.chips, chance: ui.chance, over: ui.over });
    }

    pub(super) fn dice_page(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let now = Instant::now();
        let p = ui.painter().clone();
        vgrad(&p, area, Color32::from_rgb(0x10, 0x12, 0x24), Color32::from_rgb(0x06, 0x06, 0x10));
        radial(&p, area.center() - vec2(0.0, 80.0), area.width() * 0.5, Color32::from_rgba_unmultiplied(0x6a, 0x4a, 0xff, 40), Color32::TRANSPARENT);
        let panel = Rect::from_center_size(pos2(area.center().x, area.top() + area.height() * 0.42), vec2((area.width() - 80.0).min(980.0), area.height() * 0.7));
        p.rect_filled(panel, theme::r(24), Color32::from_rgba_unmultiplied(0x14, 0x14, 0x28, 230));
        p.rect_stroke(panel, theme::r(24), Stroke::new(1.0f32, Color32::from_rgb(0x4a, 0x4a, 0x8a)), StrokeKind::Inside);
        let (chance, over) = (self.poker.casino.dice.chance, self.poker.casino.dice.over);
        // The roll, big.
        let show = self.poker.casino.dice.show.clone();
        let k = show.as_ref().map(|(_, at, _)| (now - *at).as_secs_f32()).unwrap_or(99.0);
        let (shown, won) = match &show {
            Some((r, _, _)) => match &r.outcome {
                Outcome::Dice(o) => {
                    if k < ROLL_SECS {
                        let fake = ((k * 7919.0) as u32 % 10_000) as u16;
                        let u = super::ease_out(k / ROLL_SECS);
                        (((fake as f32) * (1.0 - u) + o.roll as f32 * u) as u16, None)
                    } else {
                        (o.roll, Some(o.won))
                    }
                }
                _ => (0, None),
            },
            None => (5_000, None),
        };
        let big = pos2(panel.center().x, panel.top() + panel.height() * 0.22);
        let color = match won {
            Some(true) => col::WIN,
            Some(false) => col::LOSE,
            None => Color32::WHITE,
        };
        glow_text(&p, big, Align2::CENTER_CENTER, &dc::show(shown), theme::bold_font(84.0), color, Color32::from_black_alpha(140));
        if let Some(true) = won
            && let Some((r, _, _)) = &show
        {
            p.text(big + vec2(0.0, 62.0), Align2::CENTER_CENTER, format!("You won {}", chips_text(r.outcome.paid())), theme::bold_font(26.0), col::WIN);
        }
        // Settle once the roll's shown.
        if k >= ROLL_SECS
            && let Some((r, _, settled)) = &show
            && !*settled
            && let Outcome::Dice(o) = &r.outcome
        {
            let ui_s = &mut self.poker.casino.dice;
            ui_s.history.insert(0, (o.roll, o.won));
            ui_s.history.truncate(24);
            if let Some(sh) = ui_s.show.as_mut() {
                sh.2 = true;
            }
            if ui_s.auto_left > 0 {
                ui_s.auto_left -= 1;
                if ui_s.auto_left > 0 {
                    ui_s.next_at = Some(now + Duration::from_millis(350));
                }
            }
            if o.won {
                self.poker.casino.sound_in(0.0, Cue::WinSmall, 0.0, 0.7);
            }
            self.poker.casino.settle_balance();
            self.poker.casino.celebrate(o.paid, o.chips, false, 0.0);
        }
        if self.poker.casino.dice.auto_left > 0 && self.poker.casino.dice.next_at.is_some_and(|t| now >= t) {
            self.poker.casino.dice.next_at = None;
            self.dice_roll(s);
        }
        // The slider.
        let bar = Rect::from_center_size(pos2(panel.center().x, panel.top() + panel.height() * 0.52), vec2(panel.width() - 120.0, 22.0));
        let split = if over { 1.0 - chance as f32 / 10_000.0 } else { chance as f32 / 10_000.0 };
        let sx = bar.left() + bar.width() * split;
        let (left_c, right_c) = if over { (col::RED, col::GREEN) } else { (col::GREEN, col::RED) };
        p.rect_filled(Rect::from_min_max(bar.min, pos2(sx, bar.bottom())), theme::r(11), left_c);
        p.rect_filled(Rect::from_min_max(pos2(sx, bar.top()), bar.max), theme::r(11), right_c);
        for k in 0..=4 {
            let x = bar.left() + bar.width() * k as f32 / 4.0;
            p.text(pos2(x, bar.bottom() + 16.0), Align2::CENTER_CENTER, format!("{}", k * 25), theme::font(12.0), col::MUTED);
        }
        let handle = Rect::from_center_size(pos2(sx, bar.center().y), vec2(28.0, 44.0));
        let resp = ui.interact(bar.expand2(vec2(0.0, 16.0)), ui.id().with("dice-slider"), Sense::click_and_drag());
        p.rect_filled(handle, theme::r(8), Color32::WHITE);
        for dx in [-5.0, 0.0, 5.0] {
            p.line_segment([handle.center() + vec2(dx, -9.0), handle.center() + vec2(dx, 9.0)], Stroke::new(2.0f32, Color32::from_rgb(0x60, 0x60, 0x80)));
        }
        if (resp.dragged() || resp.clicked())
            && let Some(q) = resp.interact_pointer_pos()
        {
            let f = ((q.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
            let ch = if over { 1.0 - f } else { f };
            self.poker.casino.dice.chance = ((ch * 10_000.0) as u16).clamp(MIN_CHANCE, MAX_CHANCE) / 10 * 10;
        }
        // Where the last roll landed.
        if let Some((r, _, _)) = &show
            && let Outcome::Dice(_) = &r.outcome
        {
            let x = bar.left() + bar.width() * shown as f32 / 10_000.0;
            let tip = pos2(x, bar.top() - 6.0);
            p.add(Shape::convex_polygon(vec![tip, tip + vec2(-11.0, -18.0), tip + vec2(11.0, -18.0)], color, Stroke::new(1.5f32, Color32::BLACK)));
            let lr = Rect::from_center_size(tip - vec2(0.0, 34.0), vec2(70.0, 26.0));
            p.rect_filled(lr, theme::r(13), color);
            p.text(lr.center(), Align2::CENTER_CENTER, dc::show(shown), theme::bold_font(14.0), Color32::BLACK);
        }
        // Chance, multiplier, win.
        let stats_y = panel.top() + panel.height() * 0.74;
        let mult = dc::multiplier_x100(chance);
        let bet = self.poker.casino.dice.chips;
        let fields = [
            ("WIN CHANCE", format!("{}.{:02}%", chance / 100, chance % 100)),
            ("PAYS", times(mult)),
            (if over { "ROLL OVER" } else { "ROLL UNDER" }, dc::show(if over { 10_000 - chance } else { chance })),
            ("WIN", chips_text(dc::payout(bet, chance))),
        ];
        let fw = (panel.width() - 80.0) / 4.0;
        for (i, (label, value)) in fields.iter().enumerate() {
            let r = Rect::from_min_size(pos2(panel.left() + 40.0 + i as f32 * fw, stats_y - 30.0), vec2(fw - 14.0, 64.0));
            let resp = ui.interact(r, ui.id().with(("dice-f", i)), Sense::click());
            p.rect_filled(r, theme::r(12), if i == 2 && resp.hovered() { Color32::from_white_alpha(30) } else { Color32::from_white_alpha(14) });
            p.text(r.left_top() + vec2(14.0, 14.0), Align2::LEFT_CENTER, *label, theme::bold_font(11.0), col::MUTED);
            p.text(r.left_top() + vec2(14.0, 40.0), Align2::LEFT_CENTER, value, theme::bold_font(20.0), if i == 3 { col::GOLD2 } else { Color32::WHITE });
            if i == 2 {
                p.text(r.right_top() + vec2(-12.0, 14.0), Align2::RIGHT_CENTER, "↔ flip", theme::bold_font(11.0), if resp.hovered() { col::GOLD2 } else { col::MUTED });
                if resp.on_hover_text("Roll over instead (or under)").clicked() {
                    self.poker.casino.dice.over = !over;
                }
            }
        }
        // History.
        let hist = self.poker.casino.dice.history.clone();
        let mut x = panel.left() + 24.0;
        for (roll, won) in hist.iter().take(14) {
            let t = dc::show(*roll);
            let w = 16.0 + t.len() as f32 * 7.0;
            let r = Rect::from_min_size(pos2(x, panel.bottom() + 14.0), vec2(w, 24.0));
            p.rect_filled(r, theme::r(12), if *won { col::WIN.gamma_multiply(0.3) } else { col::LOSE.gamma_multiply(0.3) });
            p.text(r.center(), Align2::CENTER_CENTER, t, theme::bold_font(12.0), if *won { col::WIN } else { col::LOSE });
            x += w + 6.0;
        }
        // The controls.
        let cb = Rect::from_min_max(pos2(area.left() + 20.0, area.bottom() - 84.0), pos2(area.right() - 20.0, area.bottom() - 14.0));
        p.rect_filled(cb, theme::r(18), Color32::from_black_alpha(170));
        // (A chip on the rack adds to the bet.)
        let picked = self.poker.casino.chip;
        let busy = self.poker.casino.busy() || k < ROLL_SECS;
        let mut x = cb.right() - 16.0;
        let mut button = |ui: &mut egui::Ui, text: &str, w: f32, primary: bool, enabled: bool| -> bool {
            let r = Rect::from_min_size(pos2(x - w, cb.center().y - 22.0), vec2(w, 44.0));
            x -= w + 10.0;
            let resp = ui.interact(r, ui.id().with(("dc", text)), Sense::click());
            let fill = if !enabled { Color32::from_white_alpha(18) } else if primary { if resp.hovered() { col::GOLD2 } else { col::GOLD } } else if resp.hovered() { Color32::from_white_alpha(46) } else { Color32::from_white_alpha(24) };
            ui.painter().rect_filled(r, theme::r(12), fill);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(15.0), if primary && enabled { Color32::from_rgb(0x2a, 0x10, 0x04) } else if enabled { Color32::WHITE } else { Color32::from_white_alpha(70) });
            enabled && resp.clicked()
        };
        let space = ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) && !ui.ctx().wants_keyboard_input();
        let auto = self.poker.casino.dice.auto_left;
        let ok = bet > 0 && bet <= super::chips_now(s) && !busy;
        if auto > 0 {
            if button(ui, &format!("Stop ({auto})"), 130.0, true, true) {
                self.poker.casino.dice.auto_left = 0;
            }
        } else if button(ui, "Roll", 130.0, true, ok) || (space && ok) {
            self.dice_roll(s);
        }
        if button(ui, "Auto 10", 96.0, false, auto == 0 && ok) {
            self.poker.casino.dice.auto_left = 10;
            self.poker.casino.dice.next_at = Some(now);
        }
        if button(ui, "2×", 56.0, false, !busy) {
            self.poker.casino.dice.chips = (bet * 2).clamp(1, 100_000);
        }
        if button(ui, "½", 56.0, false, !busy) {
            self.poker.casino.dice.chips = (bet / 2).max(1);
        }
        if button(ui, &format!("+{}", chips_text(picked)), 90.0, false, !busy) {
            self.poker.casino.dice.chips = (bet + picked).min(100_000);
        }
        let color = if bet > super::chips_now(s) { col::LOSE } else { col::GOLD2 };
        self.rack_and_bet(ui, cb, x, 25_000, format!("Bet {}", chips_text(bet)), color);
    }
}
