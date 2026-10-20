//! Craps at the app: the layout (pass line and odds, don't pass, field, place numbers,
//! hardways and the one-roll props), the puck on the point, the dice tumbling down the felt,
//! and the stickman's call.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::casino::craps::{self as cr, Bets, HARD_NUMBERS, PLACE_NUMBERS};
use pyr_poker::casino::Outcome;
use pyr_poker::net::RoundView;

use super::{art, chip_stack, col, glow_text, vgrad};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

const ROLL_SECS: f32 = 1.1;

#[derive(Default)]
pub struct CrapsUi {
    /// The bets as I want them for the next roll (sent before it).
    pub desired: Option<Bets>,
    roll_after: bool,
    show: Option<(RoundView, Instant)>,
    pub history: Vec<(u8, u8)>,
}

#[derive(Clone, Copy, PartialEq)]
enum Spot {
    Pass,
    Odds,
    DontPass,
    Field,
    Place(usize),
    Hard(usize),
    AnySeven,
    AnyCraps,
}

fn slot(b: &mut Bets, s: Spot) -> &mut u64 {
    match s {
        Spot::Pass => &mut b.pass,
        Spot::Odds => &mut b.pass_odds,
        Spot::DontPass => &mut b.dont_pass,
        Spot::Field => &mut b.field,
        Spot::Place(i) => &mut b.place[i],
        Spot::Hard(i) => &mut b.hard[i],
        Spot::AnySeven => &mut b.any_seven,
        Spot::AnyCraps => &mut b.any_craps,
    }
}

/// The stickman's call for a roll.
fn call(o: &cr::Outcome) -> String {
    let sum = o.dice.0 + o.dice.1;
    let hard = o.dice.0 == o.dice.1;
    match (o.point_before, o.point_after) {
        (None, Some(p)) => format!("THE POINT IS {p}"),
        (None, None) => match sum {
            7 => "WINNER SEVEN!".into(),
            11 => "YO-LEVEN! WINNER".into(),
            2 => "CRAPS, ACES".into(),
            3 => "CRAPS, ACE-DEUCE".into(),
            _ => "CRAPS, TWELVE".into(),
        },
        (Some(p), None) if sum == p => {
            if hard {
                format!("{p} THE HARD WAY! WINNER")
            } else {
                format!("{p}, WINNER! MARK IT")
            }
        }
        (Some(_), None) => "SEVEN OUT".into(),
        (Some(_), Some(_)) => {
            if hard && HARD_NUMBERS.contains(&sum) {
                format!("{sum} THE HARD WAY")
            } else if matches!(sum, 2 | 3 | 4 | 9 | 10 | 11 | 12) {
                format!("{sum}, FIELD WINS")
            } else {
                format!("{sum}, ROLL AGAIN")
            }
        }
    }
}

impl PyrApp {
    pub(super) fn craps_round(&mut self, r: &RoundView) {
        let Outcome::Craps(o) = &r.outcome else { return };
        let ui = &mut self.poker.casino.craps;
        ui.show = Some((r.clone(), Instant::now()));
        ui.history.insert(0, o.dice);
        ui.history.truncate(16);
        // What's left on the table is what I'll want next roll (change it if not).
        ui.desired = r.craps.as_ref().map(|t| t.bets.clone());
        self.poker.casino.sound_in(0.0, Cue::DiceRoll, 0.3, 0.8);
        let back = o.paid + o.kept;
        if o.paid > 0 {
            self.poker.casino.sound_in(ROLL_SECS + 0.2, Cue::WinSmall, 0.0, 0.8);
            self.poker.casino.sound_in(ROLL_SECS + 0.35, Cue::Chips, 0.0, 0.6);
        }
        self.poker.casino.settle_at(Instant::now() + Duration::from_secs_f32(ROLL_SECS + 0.3));
        self.poker.casino.celebrate(back, o.wagered, false, ROLL_SECS + 0.3);
    }

    pub(super) fn craps_page(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let now = Instant::now();
        let p = ui.painter().clone();
        let c = s.poker.casino.clone();
        let table = c.craps.clone();
        if self.poker.casino.craps.desired.is_none() && c.open {
            self.poker.casino.craps.desired = Some(table.bets.clone());
        }
        // My bets didn't go on (refused): no roll, and the layout shows the table as it is.
        if self.poker.casino.craps.roll_after && !self.poker.casino.busy() && self.poker.casino.craps.desired.as_ref() != Some(&table.bets) {
            self.poker.casino.craps.roll_after = false;
            self.poker.casino.craps.desired = Some(table.bets.clone());
        }
        // Roll once my bets are on the table.
        if self.poker.casino.craps.roll_after && !self.poker.casino.busy() && self.poker.casino.craps.desired.as_ref() == Some(&table.bets) {
            self.poker.casino.craps.roll_after = false;
            if let Some(commit) = c.commit {
                self.poker.casino.pending = Some(now);
                self.poker.casino.celebration = None;
                self.engine.casino_craps_roll(&s.key, commit);
            }
        }
        vgrad(&p, area, Color32::from_rgb(0x08, 0x1e, 0x12), Color32::from_rgb(0x03, 0x0c, 0x07));
        let felt = Rect::from_min_max(area.min + vec2(24.0, 20.0), area.max - vec2(24.0, 108.0));
        p.rect_filled(felt.expand(14.0), theme::r(30), Color32::from_rgb(0x3a, 0x22, 0x12));
        p.rect_filled(felt, theme::r(22), col::FELT);
        p.rect_stroke(felt, theme::r(22), Stroke::new(2.0f32, col::GOLD_DARK), StrokeKind::Inside);
        let desired = self.poker.casino.craps.desired.clone().unwrap_or_default();
        let w = felt.width();
        let line = Stroke::new(2.0f32, Color32::from_white_alpha(200));
        let mut spots: Vec<(Spot, Rect, String)> = Vec::new();
        // Place numbers along the top.
        let main = Rect::from_min_max(felt.min + vec2(18.0, 16.0), pos2(felt.left() + w * 0.68, felt.bottom() - 16.0));
        let bw = main.width() / 6.0;
        let place_h = main.height() * 0.32;
        for (i, n) in PLACE_NUMBERS.iter().enumerate() {
            let r = Rect::from_min_size(pos2(main.left() + i as f32 * bw, main.top()), vec2(bw, place_h));
            spots.push((Spot::Place(i), r, format!("Place {n}: pays {}", ["9 to 5", "7 to 5", "7 to 6", "7 to 6", "7 to 5", "9 to 5"][i])));
            p.rect_stroke(r, 0, line, StrokeKind::Inside);
            let label = if *n == 6 { "SIX".to_string() } else if *n == 9 { "NINE".to_string() } else { n.to_string() };
            let size = (place_h * 0.26).min(bw * 0.24);
            p.text(r.center() - vec2(0.0, place_h * 0.16), Align2::CENTER_CENTER, label, theme::bold_font(size), if *n == 6 || *n == 8 { col::RED } else { col::GOLD2 });
            p.text(r.center() + vec2(0.0, place_h * 0.1), Align2::CENTER_CENTER, ["PLACE 9:5", "PLACE 7:5", "PLACE 7:6", "PLACE 7:6", "PLACE 7:5", "PLACE 9:5"][i], theme::bold_font((bw * 0.1).clamp(9.0, 12.0)), Color32::from_white_alpha(150));
            if table.point == Some(*n) {
                // The puck: ON.
                let pc = r.right_top() + vec2(-20.0, 20.0);
                p.circle_filled(pc + vec2(1.0, 2.0), 16.0, Color32::from_black_alpha(120));
                p.circle_filled(pc, 16.0, Color32::WHITE);
                p.circle_stroke(pc, 16.0, Stroke::new(2.0f32, Color32::BLACK));
                p.text(pc, Align2::CENTER_CENTER, "ON", theme::bold_font(11.0), Color32::BLACK);
            }
        }
        if table.point.is_none() {
            let pc = pos2(main.left() + 22.0, main.top() + place_h + 22.0);
            p.circle_filled(pc, 16.0, Color32::from_rgb(0x18, 0x18, 0x1c));
            p.circle_stroke(pc, 16.0, Stroke::new(2.0f32, Color32::WHITE));
            p.text(pc, Align2::CENTER_CENTER, "OFF", theme::bold_font(10.0), Color32::WHITE);
        }
        // Field, don't pass, pass line.
        let rest = Rect::from_min_max(pos2(main.left(), main.top() + place_h), main.max);
        let band = rest.height() / 3.0;
        let field = Rect::from_min_size(rest.min, vec2(rest.width(), band));
        let dont = Rect::from_min_size(pos2(rest.left(), rest.top() + band), vec2(rest.width(), band * 0.8));
        let pass = Rect::from_min_max(pos2(rest.left(), rest.top() + band * 1.8), rest.max);
        p.rect_stroke(field, 0, line, StrokeKind::Inside);
        p.text(field.left_center() + vec2(18.0, -10.0), Align2::LEFT_CENTER, "FIELD", theme::bold_font(22.0), col::GOLD2);
        p.text(field.left_center() + vec2(18.0, 14.0), Align2::LEFT_CENTER, "2 pays double · 12 pays triple", theme::font(12.0), Color32::from_white_alpha(200));
        p.text(field.center() + vec2(field.width() * 0.12, 0.0), Align2::CENTER_CENTER, "2 · 3 · 4 · 9 · 10 · 11 · 12", theme::bold_font(18.0), Color32::WHITE);
        spots.push((Spot::Field, field, "Field: one roll; 2, 3, 4, 9, 10, 11 or 12 wins".into()));
        p.rect_stroke(dont, 0, line, StrokeKind::Inside);
        p.text(dont.center(), Align2::CENTER_CENTER, "DON'T PASS BAR", theme::bold_font(18.0), Color32::WHITE);
        spots.push((Spot::DontPass, dont, "Don't pass: wins on 2 or 3 at the come-out, then on a 7 before the point".into()));
        p.rect_stroke(pass, 0, line, StrokeKind::Inside);
        let pass_label = Rect::from_min_size(pass.min, vec2(pass.width() * 0.72, pass.height()));
        p.text(pass_label.center(), Align2::CENTER_CENTER, "PASS LINE", theme::bold_font(26.0), Color32::WHITE);
        spots.push((Spot::Pass, pass_label, "Pass line: wins on 7 or 11 at the come-out, then on the point before a 7".into()));
        let odds = Rect::from_min_max(pos2(pass_label.right(), pass.top()), pass.max);
        if table.point.is_some() {
            p.rect_stroke(odds.shrink(6.0), theme::r(10), Stroke::new(1.5f32, col::GOLD), StrokeKind::Inside);
            p.text(odds.center() - vec2(0.0, 16.0), Align2::CENTER_CENTER, "ODDS", theme::bold_font(14.0), col::GOLD2);
            p.text(odds.center() + vec2(0.0, 2.0), Align2::CENTER_CENTER, "true odds, no edge", theme::font(10.5), Color32::from_white_alpha(180));
            spots.push((Spot::Odds, odds, format!("Odds behind the pass line: up to {}× it, paid at true odds", table.point.map(cr::max_odds).unwrap_or(3))));
        }
        // The props: any seven, hardways, any craps.
        let props = Rect::from_min_max(pos2(main.right() + 18.0, main.top()), pos2(felt.right() - 18.0, felt.bottom() - 16.0));
        p.rect_stroke(props, theme::r(10), line, StrokeKind::Inside);
        let ph = props.height() / 4.0;
        let seven = Rect::from_min_size(props.min, vec2(props.width(), ph * 0.9));
        p.text(seven.center(), Align2::CENTER_CENTER, "ANY SEVEN  ·  4 to 1", theme::bold_font(15.0), col::RED);
        spots.push((Spot::AnySeven, seven, "Any seven: one roll, pays 4 to 1".into()));
        for (i, n) in HARD_NUMBERS.iter().enumerate() {
            let (cx, cy) = (i % 2, i / 2);
            let r = Rect::from_min_size(pos2(props.left() + cx as f32 * props.width() / 2.0, props.top() + ph * 0.9 + cy as f32 * ph * 1.05), vec2(props.width() / 2.0, ph * 1.05));
            p.rect_stroke(r, 0, Stroke::new(1.0f32, Color32::from_white_alpha(120)), StrokeKind::Inside);
            let half = n / 2;
            let d = r.height() * 0.32;
            art::die(&p, r.center() - vec2(d * 0.62, 8.0), d, half, 0.0, Color32::from_rgb(0xf4, 0xf0, 0xe8));
            art::die(&p, r.center() + vec2(d * 0.62, -8.0), d, half, 0.0, Color32::from_rgb(0xf4, 0xf0, 0xe8));
            p.text(r.center_bottom() - vec2(0.0, 12.0), Align2::CENTER_CENTER, format!("{} to 1", if *n == 4 || *n == 10 { 7 } else { 9 }), theme::bold_font(12.0), col::GOLD2);
            spots.push((Spot::Hard(i), r, format!("Hard {n}: {half} and {half} before a 7 or an easy {n}")));
        }
        let crapsr = Rect::from_min_max(pos2(props.left(), props.top() + ph * 3.0), props.max);
        p.text(crapsr.center(), Align2::CENTER_CENTER, "ANY CRAPS  ·  7 to 1", theme::bold_font(15.0), col::RED);
        spots.push((Spot::AnyCraps, crapsr, "Any craps (2, 3 or 12): one roll, pays 7 to 1".into()));
        // Clicks.
        let rolling = self.poker.casino.craps.show.as_ref().is_some_and(|(_, at)| (now - *at).as_secs_f32() < ROLL_SECS + 0.3);
        let mut hover_tip = None;
        for (spot, r, tip) in &spots {
            let resp = ui.interact(*r, ui.id().with(("craps", format!("{tip}"))), Sense::click());
            if resp.hovered() {
                p.rect_filled(*r, 0, Color32::from_white_alpha(18));
                hover_tip = Some(tip.clone());
            }
            if rolling || self.poker.casino.busy() {
                continue;
            }
            let mut want = desired.clone();
            if resp.clicked() {
                let chip = self.poker.casino.chip;
                *slot(&mut want, *spot) += chip;
                if *spot == Spot::Odds
                    && let Some(pt) = table.point
                {
                    want.pass_odds = want.pass_odds.min(want.pass * cr::max_odds(pt));
                }
            }
            if resp.secondary_clicked() {
                *slot(&mut want, *spot) = 0;
            }
            if want != desired {
                match cr::check_change(&table, &want) {
                    Ok(()) => {
                        self.poker.casino.craps.desired = Some(want);
                        self.poker.casino.sound_in(0.0, pyr_engine::poker_sfx::Cue::Chips, 0.0, 0.4);
                    }
                    Err(e) => hover_tip = Some(e),
                }
            }
        }
        // Chips on the table.
        let desired = self.poker.casino.craps.desired.clone().unwrap_or_default();
        for (spot, r, _) in &spots {
            let mut d = desired.clone();
            let amount = *slot(&mut d, *spot);
            if amount > 0 {
                let at = match spot {
                    Spot::Pass => r.center() + vec2(r.width() * 0.32, 4.0),
                    Spot::DontPass => r.center() + vec2(r.width() * 0.32, 4.0),
                    Spot::Field => r.center() - vec2(r.width() * 0.3, -4.0),
                    _ => r.center() + vec2(0.0, r.height() * 0.22),
                };
                chip_stack(&p, at, 15.0, amount);
            }
        }
        if let Some(tip) = hover_tip {
            p.text(pos2(felt.left() + 8.0, felt.bottom() + 24.0), Align2::LEFT_CENTER, tip, theme::bold_font(13.0), col::GOLD2);
        }
        // The dice, and the call.
        let show = self.poker.casino.craps.show.clone();
        if let Some((r, at)) = &show
            && let Outcome::Craps(o) = &r.outcome
        {
            let k = (now - *at).as_secs_f32();
            // (They come to rest in the middle of the layout, by the don't pass bar.)
            let land = pos2(main.center().x - 37.0, dont.center().y);
            for (i, face) in [o.dice.0, o.dice.1].iter().enumerate() {
                let u = (k / ROLL_SECS).min(1.0);
                let start = pos2(felt.right() - 30.0, felt.top() + 40.0 + i as f32 * 50.0);
                let end = land + vec2(i as f32 * 74.0, (i as f32 - 0.5) * 18.0);
                let e = super::ease_out(u);
                let bounce = ((u * 9.0).sin() * (1.0 - u) * 40.0).abs();
                let q = start + (end - start) * e - vec2(0.0, bounce);
                let shown = if u < 0.85 { ((k * 23.0 + i as f32 * 3.0) as u8 % 6) + 1 } else { *face };
                let angle = (1.0 - u) * 9.0 + i as f32;
                art::die(&p, q, 56.0, shown, angle, Color32::from_rgb(0xd8, 0x1f, 0x2e));
            }
            if k > ROLL_SECS {
                let a = ((k - ROLL_SECS) / 0.25).min(1.0);
                // The stickman's call, over the dice, on a dark plate so it reads on any part of
                // the layout.
                let text = call(o);
                let galley = p.layout_no_wrap(text.clone(), theme::bold_font(28.0), col::GOLD2);
                let plate = Rect::from_center_size(pos2(land.x + 37.0, land.y - 66.0), galley.size() + vec2(36.0, 14.0));
                p.rect_filled(plate, theme::r(14), Color32::from_black_alpha((200.0 * a) as u8));
                p.rect_stroke(plate, theme::r(14), Stroke::new(1.0f32, col::GOLD.gamma_multiply(0.6 * a)), StrokeKind::Inside);
                glow_text(&p, plate.center(), Align2::CENTER_CENTER, &text, theme::bold_font(28.0), col::GOLD2.gamma_multiply(a), Color32::from_black_alpha((120.0 * a) as u8));
                // What the roll decided, under the dice.
                let lines: Vec<(String, Color32)> = o
                    .decided
                    .iter()
                    .take(5)
                    .map(|d| {
                        // (Place bets and hardways stay up when they win: what comes back is the win.)
                        let stays = d.bet.starts_with("Place") || d.bet.starts_with("Hard");
                        let profit = if stays { d.returned } else { d.returned.saturating_sub(d.stake) };
                        if d.won { (format!("{} wins +{}", d.bet, chips_text(profit)), col::WIN) } else { (format!("{} loses {}", d.bet, chips_text(d.stake)), col::LOSE) }
                    })
                    .collect();
                if !lines.is_empty() {
                    let wmax = lines.iter().map(|(t, _)| t.len()).max().unwrap_or(0) as f32 * 8.2 + 28.0;
                    let list = Rect::from_min_size(pos2(land.x + 37.0 - wmax / 2.0, land.y + 40.0), vec2(wmax, lines.len() as f32 * 21.0 + 12.0));
                    p.rect_filled(list, theme::r(12), Color32::from_black_alpha((190.0 * a) as u8));
                    for (i, (t, color)) in lines.iter().enumerate() {
                        p.text(pos2(list.center().x, list.top() + 16.0 + i as f32 * 21.0), Align2::CENTER_CENTER, t, theme::bold_font(14.0), color.gamma_multiply(a));
                    }
                }
            }
        }
        // Shooter's streak and recent rolls.
        let hist = self.poker.casino.craps.history.clone();
        let mut x = felt.right() - 8.0;
        for (a, b) in hist.iter().take(10) {
            let t = format!("{}", a + b);
            let r = Rect::from_min_size(pos2(x - 30.0, felt.bottom() + 12.0), vec2(30.0, 24.0));
            p.rect_filled(r, theme::r(6), if a + b == 7 { col::RED } else { Color32::from_white_alpha(30) });
            p.text(r.center(), Align2::CENTER_CENTER, t, theme::bold_font(12.5), Color32::WHITE);
            x -= 34.0;
        }
        if table.streak > 0 {
            p.text(pos2(x - 8.0, felt.bottom() + 24.0), Align2::RIGHT_CENTER, format!("Shooter's points: {}", table.streak), theme::bold_font(12.5), col::GOLD);
        }
        // The controls.
        let bar = Rect::from_min_max(pos2(area.left() + 20.0, area.bottom() - 74.0), pos2(area.right() - 20.0, area.bottom() - 10.0));
        self.craps_controls(ui, s, bar, rolling);
    }

    fn craps_controls(&mut self, ui: &mut egui::Ui, s: &ServerView, bar: Rect, rolling: bool) {
        let p = ui.painter().clone();
        p.rect_filled(bar, theme::r(18), Color32::from_black_alpha(170));
        self.chip_rack(ui, pos2(bar.left() + 16.0, bar.center().y - 26.0), 5_000);
        let c = &s.poker.casino;
        let table = c.craps.clone();
        let desired = self.poker.casino.craps.desired.clone().unwrap_or_default();
        let busy = self.poker.casino.busy() || rolling || self.poker.casino.craps.roll_after;
        let on_table = desired.total();
        let need = on_table.saturating_sub(table.bets.total());
        let mut x = bar.right() - 16.0;
        let mut button = |ui: &mut egui::Ui, text: &str, w: f32, primary: bool, enabled: bool| -> bool {
            let r = Rect::from_min_size(pos2(x - w, bar.center().y - 22.0), vec2(w, 44.0));
            x -= w + 10.0;
            let resp = ui.interact(r, ui.id().with(("cr", text)), Sense::click());
            let fill = if !enabled { Color32::from_white_alpha(18) } else if primary { if resp.hovered() { col::GOLD2 } else { col::GOLD } } else if resp.hovered() { Color32::from_white_alpha(46) } else { Color32::from_white_alpha(24) };
            ui.painter().rect_filled(r, theme::r(12), fill);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(15.0), if primary && enabled { Color32::from_rgb(0x2a, 0x10, 0x04) } else if enabled { Color32::WHITE } else { Color32::from_white_alpha(70) });
            enabled && resp.clicked()
        };
        let space = ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) && !ui.ctx().wants_keyboard_input();
        let ok = on_table > 0 && need <= super::chips_now(s) && !busy;
        if button(ui, "Roll", 130.0, true, ok) || (space && ok) {
            if desired != table.bets {
                self.poker.casino.pending = Some(Instant::now());
                self.engine.casino_craps_bets(&s.key, desired.clone());
            }
            self.poker.casino.craps.roll_after = true;
        }
        if button(ui, "Clear", 80.0, false, !busy) {
            // Take down what may come down (the pass line stays once there's a point).
            let mut want = Bets::default();
            if table.point.is_some() {
                want.pass = table.bets.pass;
            }
            self.poker.casino.craps.desired = Some(want);
        }
        p.text(pos2(x - 10.0, bar.center().y), Align2::RIGHT_CENTER, format!("On the table {}", chips_text(on_table)), theme::bold_font(16.0), col::GOLD2);
    }
}
