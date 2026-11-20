//! Parlays (0.3, Pyr 0.16): two to ten picks linked into one bet, every one to win. The slip a
//! parlay is put together on, its page (the slip, what there is to pick from, and my parlays
//! as tickets), and what a pick on any other page does to the slip. The rules and the sums
//! are `pyr_poker::props`'s, the same ones the server goes by: what the slip says it pays is
//! what the server would pay, unless the odds move first.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Shape, Stroke, pos2, vec2};
use pyr_poker::props::{self as p, LegState, ParlayLeg, ParlayState, ParlayView, PropView};

use crate::asks::{self, ask};
use crate::look::*;
use crate::model::{self, Model};
use crate::views::{card, lines, section};
use crate::words::{self, ago, chips_text, count, signed};
use crate::{App, View, note};

/// The parlay being put together.
pub struct Slip {
    /// Its picks: each prop and the outcome picked, in the order they were picked.
    pub legs: Vec<(u64, usize)>,
    /// The stake, as typed.
    pub amount: String,
    /// Why the last pick couldn't go in, and when that was said.
    pub refused: Option<(String, Instant)>,
    /// When it was last sent to be placed (the button waits a moment for the server's word).
    pub placing: Option<Instant>,
    /// The pick that just went in, and when (its row lights a moment).
    pub added: Option<(u64, Instant)>,
}

impl Default for Slip {
    fn default() -> Slip {
        Slip { legs: Vec::new(), amount: "1000".into(), refused: None, placing: None, added: None }
    }
}

/// A pick of the slip, priced for its stake.
pub struct Priced {
    pub prop: u64,
    pub outcome: usize,
    pub odds: f64,
    pub room: f64,
    /// Its prop still takes bets.
    pub open: bool,
    /// Its odds here are the ones its prop opened at: bets since have left the pick longer
    /// on the board, and a parlay pays the less of the two (`pyr_poker::props::leg_price`).
    pub as_opened: bool,
}

/// The slip priced: each pick, what the stake is times, what it pays, and whether what it
/// pays is held down by its legs' rooms (see `pyr_poker::props::parlay_pays`).
pub struct Quote {
    pub legs: Vec<Priced>,
    pub pays: u64,
    pub capped: bool,
}

/// Odds as they're read off a slip: "×2.50", and long ones without the cents.
pub fn odds_text(odds: f64) -> String {
    if odds >= 1000.0 {
        format!("×{}", chips_text(odds.round() as u64))
    } else if odds >= 100.0 {
        format!("×{odds:.0}")
    } else {
        format!("×{odds:.2}")
    }
}

/// The same with the American way of saying them after: "×2.50 (+150)".
pub fn odds_both(odds: f64) -> String {
    let us = p::american(odds);
    if us.is_empty() { odds_text(odds) } else { format!("{} ({us})", odds_text(odds)) }
}

/// What a ticket is called: "3-leg parlay".
pub fn title(t: &ParlayView) -> String {
    format!("{}-leg parlay", t.legs.len())
}

impl App {
    /// The outcome of a prop that's in the slip, if one is.
    pub fn picked(&self, prop: u64) -> Option<usize> {
        self.slip.legs.iter().find(|l| l.0 == prop).map(|l| l.1)
    }

    /// Why a prop can't go in the slip as the slip stands, if it can't (another outcome of a
    /// prop already in it is a change of pick, and always can).
    pub fn pick_blocked(&self, prop: &PropView, now: i64) -> Option<String> {
        if let Err(why) = p::parlay_takes(prop, now) {
            return Some(why.to_string());
        }
        if self.picked(prop.id).is_some() {
            return None;
        }
        for (id, _) in &self.slip.legs {
            if let Some(why) = self.m.props.get(id).and_then(|other| p::parlay_clash(other, prop)) {
                return Some(why.to_string());
            }
        }
        (self.slip.legs.len() >= p::PARLAY_MAX_LEGS).then(|| format!("A parlay has {} legs at most.", p::PARLAY_MAX_LEGS))
    }

    /// An outcome clicked for the slip: in if it can go, out if it was in, or the pick on its
    /// prop changed to it. Says whether the slip changed.
    pub fn pick(&mut self, prop: &PropView, outcome: usize, now: i64) -> bool {
        if let Some(i) = self.slip.legs.iter().position(|l| l.0 == prop.id) {
            if self.slip.legs[i].1 == outcome {
                self.slip.legs.remove(i);
            } else {
                self.slip.legs[i].1 = outcome;
                self.slip.added = Some((prop.id, Instant::now()));
            }
            self.slip.refused = None;
            return true;
        }
        match self.pick_blocked(prop, now) {
            Some(why) => {
                self.slip.refused = Some((why, Instant::now()));
                false
            }
            None => {
                self.slip.legs.push((prop.id, outcome));
                self.slip.added = Some((prop.id, Instant::now()));
                self.slip.refused = None;
                true
            }
        }
    }

    /// A pick made away from the parlay's own page (Shift and a click on an outcome, or Add to
    /// parlay on a prop's page): what came of it is said at the top.
    pub fn pick_from_afar(&mut self, prop: &PropView, outcome: usize, now: i64) {
        let was = self.picked(prop.id);
        let changed = self.pick(prop, outcome, now);
        // (The Parlays page opens on the slip it went on.)
        self.parlay_mine &= !changed;
        let name = prop.outcomes.get(outcome).cloned().unwrap_or_default();
        let text = match (changed, was, self.slip.refused.as_ref()) {
            (false, _, Some((why, _))) => (why.clone(), true),
            (_, Some(o), _) if o == outcome => (format!("{name} is out of your parlay."), false),
            _ => (format!("{name} is in your parlay: {}. Parlays, at the side, has the slip.", count(self.slip.legs.len() as u64, "pick", "picks")), false),
        };
        self.m.said = Some((text.0, text.1, Instant::now()));
    }

    /// The slip as it stands, priced for a stake (picks whose props are gone left out).
    pub fn slip_quote(&self, chips: u64, now: i64) -> Quote {
        let stake = chips.max(p::MIN_BET);
        let legs: Vec<Priced> = self
            .slip
            .legs
            .iter()
            .filter_map(|(id, outcome)| {
                let prop = self.m.props.get(id)?;
                let q = prop.leg(*outcome, stake);
                let on_the_board = p::leg_quote(&prop.parlay_book(), prop.b, *outcome, stake).odds;
                Some(Priced { prop: *id, outcome: *outcome, odds: q.odds, room: q.room, open: prop.open_at(now), as_opened: q.odds < on_the_board * 0.99 })
            })
            .collect();
        let whole: f64 = legs.iter().map(|l| l.odds).product::<f64>() * stake as f64;
        let pays = p::parlay_pays(stake, legs.iter().map(|l| (l.odds, l.room)));
        Quote { capped: whole > pays + 0.5, pays: pays.floor() as u64, legs }
    }

    /// What's no longer a pick goes from the slip: props the server forgot, and ones that are
    /// over (one that only closed stays, and is said to be in the way).
    pub fn slip_tidy(&mut self) {
        let props = &self.m.props;
        self.slip.legs.retain(|(id, _)| props.get(id).is_some_and(Model::live));
        if self.slip.refused.as_ref().is_some_and(|r| r.1.elapsed() > Duration::from_secs(7)) {
            self.slip.refused = None;
        }
        if self.slip.placing.is_some_and(|t| t.elapsed() > Duration::from_secs(4)) {
            self.slip.placing = None;
        }
    }

    /// My parlays still riding.
    pub fn riding(&self) -> usize {
        self.m.parlays.iter().filter(|t| t.state == ParlayState::Open).count()
    }

    // ------------------------------------------------------------ the page

    pub fn parlay_page(&mut self, ui: &mut egui::Ui, now: i64) {
        ui.add_space(4.0);
        self.parlay_banner(ui);
        ui.add_space(10.0);
        if !self.m.parlays_here {
            return note(ui, "No parlays on this server yet", "Its owner's Pyr server needs Pyr 0.16 or newer for parlays. Single bets work as ever.");
        }
        // Two sides to it: a new parlay (the slip, and what there is to pick from), and mine
        // as tickets.
        let (riding, over): (Vec<ParlayView>, Vec<ParlayView>) = self.m.parlays.iter().cloned().partition(|t| t.state == ParlayState::Open);
        let picks = self.slip.legs.len();
        let new_one = if picks > 0 { format!("New parlay  {picks}") } else { "New parlay".to_string() };
        let mine = if riding.is_empty() { "Your parlays".to_string() } else { format!("Your parlays  {} riding", riding.len()) };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            for (label, side) in [(new_one, false), (mine, true)] {
                let on = self.parlay_mine == side;
                let text = RichText::new(label).font(bold_font(13.5)).color(if on { ON_EMBER } else { TEXT });
                if ui.add(egui::Button::new(text).fill(if on { EMBER2 } else { ELEV }).corner_radius(15).min_size(vec2(0.0, 30.0))).clicked() {
                    self.parlay_mine = side;
                }
            }
        });
        ui.add_space(10.0);
        if !self.parlay_mine {
            self.slip_card(ui, now);
            self.pick_from(ui, now);
            return;
        }
        if riding.is_empty() && over.is_empty() {
            return note(ui, "No parlays of yours yet", "Put two or more picks on a slip under New parlay. Once it's placed, its ticket is here, each pick ticked off as it comes in.");
        }
        if !riding.is_empty() {
            section(ui, "Riding");
            for t in &riding {
                self.ticket(ui, t, now);
            }
        }
        if !over.is_empty() {
            section(ui, "Settled");
            for t in over.iter().take(40) {
                self.ticket(ui, t, now);
            }
        }
    }

    /// What a parlay is, over its picture: a chain of gold rings on the felt (a light crosses
    /// it now and then). Without the picture, rings drawn here that swing a little.
    fn parlay_banner(&mut self, ui: &mut egui::Ui) {
        let w = ui.available_width();
        let words = "Link two to ten picks into one bet. Every pick has to win, and their odds multiply. A pick that's called off counts as 1.00.";
        let narrow = w < 470.0;
        let text = lines(ui.painter(), words, FontId::proportional(13.0), BODY, if narrow { w - 36.0 } else { (w * 0.52).max(250.0) }, 4);
        let h = (text.size().y + 64.0).max(126.0);
        let (r, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
        let pnt = ui.painter().with_clip_rect(r);
        let t = ui.input(|i| i.time) as f32;
        match self.art.parlay.as_ref() {
            Some(art) if narrow => {
                pnt.add(cover_rounded(art, r, 14.0, (0.5, 0.42), Color32::from_gray(225)));
                pnt.add(gradient(r, 14.0, Color32::from_black_alpha(215), Color32::from_black_alpha(150)));
            }
            Some(art) => {
                // The picture down the right (enough of it to see the rings whole), coming
                // out of the dark the words are on.
                let dark = Color32::from_rgb(0x12, 0x0d, 0x0a);
                pnt.rect_filled(r, CornerRadius::same(14), dark);
                let at = Rect::from_min_max(pos2(r.left() + r.width() * 0.40, r.top()), r.max);
                pnt.add(cover_rounded(art, at, 14.0, (0.46, 0.44), Color32::from_gray(235)));
                pnt.add(gradient(at, 14.0, dark, Color32::from_rgba_unmultiplied(0x12, 0x0d, 0x0a, 0)));
                // A light crossing the gold, every few seconds (the picture only: not the dark
                // beside it).
                let phase = (t % 7.0) / 1.3;
                if phase < 1.0 {
                    let lit = Rect::from_min_max(pos2(at.left() + at.width() * 0.22, at.top()), at.max);
                    let lean = h * 0.55;
                    let band = 46.0;
                    let x = lit.left() + (lit.width() + lean + band) * phase - band;
                    let a = (phase * std::f32::consts::PI).sin() * 30.0;
                    pnt.with_clip_rect(lit).add(Shape::convex_polygon(vec![pos2(x, r.top()), pos2(x + band, r.top()), pos2(x + band - lean, r.bottom()), pos2(x - lean, r.bottom())], Color32::from_white_alpha(a as u8), Stroke::NONE));
                    ui.ctx().request_repaint_after(Duration::from_millis(33));
                } else {
                    ui.ctx().request_repaint_after(Duration::from_millis(400));
                }
            }
            None => {
                pnt.add(gradient(r, 14.0, Color32::from_rgb(0x3a, 0x22, 0x12), Color32::from_rgb(0x1f, 0x18, 0x14)));
                for k in 0..5 {
                    let c = pos2(r.right() - 34.0 - k as f32 * 22.0, r.center().y + (t * 0.9 + k as f32 * 0.8).sin() * 3.0);
                    let col = if k % 2 == 0 { GOLD } else { EMBER2 };
                    pnt.add(Shape::ellipse_stroke(c, if k % 2 == 0 { vec2(15.0, 9.5) } else { vec2(9.5, 15.0) }, Stroke::new(3.0_f32, col.gamma_multiply(0.85 - k as f32 * 0.14))));
                }
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }
        }
        pnt.rect_stroke(r, CornerRadius::same(14), Stroke::new(1.0_f32, mix(CARD_EDGE, GOLD, 0.3)), egui::StrokeKind::Inside);
        pnt.text(pos2(r.left() + 20.0, r.top() + 30.0), Align2::LEFT_CENTER, "Parlays", bold_font(25.0), Color32::WHITE);
        pnt.galley(pos2(r.left() + 20.0, r.top() + 50.0), text, BODY);
    }

    /// The slip: its picks down a chain, the stake, what it pays, and Place.
    fn slip_card(&mut self, ui: &mut egui::Ui, now: i64) {
        let have = self.m.chips;
        let chips: u64 = self.slip.amount.replace([',', ' '], "").parse().unwrap_or(0);
        let quote = self.slip_quote(chips, now);
        let n = quote.legs.len();
        let mut out: Option<u64> = None;
        let mut open_prop: Option<u64> = None;
        let mut clear = false;
        let mut place = false;
        egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0_f32, mix(CARD_EDGE, GOLD, 0.35))).corner_radius(12).inner_margin(egui::Margin::symmetric(14, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new("YOUR SLIP").size(10.5).strong().color(GOLD));
                if n > 0 {
                    ui.label(RichText::new(format!("· {}", count(n as u64, "pick", "picks"))).size(11.5).color(MUTED));
                }
            });
            ui.add_space(4.0);
            if n == 0 {
                ui.label(RichText::new("Nothing in it yet. Pick an outcome below: one to a game, and outside the games one to a category.").size(13.0).color(MUTED));
                ui.label(RichText::new("Anywhere else in Pyr Prop Bets, hold Shift and click an outcome to put it in.").size(12.0).color(DIM));
            }
            for (k, leg) in quote.legs.iter().enumerate() {
                let Some(prop) = self.m.props.get(&leg.prop) else { continue };
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
                let pnt = ui.painter();
                if resp.hovered() {
                    pnt.rect_filled(r, CornerRadius::same(8), ELEV);
                }
                // (Just in: its row lights, and fades.)
                if let Some((_, at)) = self.slip.added.filter(|a| a.0 == leg.prop) {
                    let left = fading(at, 0.9);
                    if left > 0.0 {
                        pnt.rect_filled(r, CornerRadius::same(8), EMBER.gamma_multiply(0.22 * left));
                        ui.ctx().request_repaint_after(Duration::from_millis(33));
                    }
                }
                // The chain down the left: a ring a pick, each linked to the next.
                let node = pos2(r.left() + 13.0, r.center().y);
                let col = color_of(&prop.outcomes, leg.outcome);
                if k + 1 < n {
                    pnt.line_segment([node + vec2(0.0, 8.0), node + vec2(0.0, 36.0)], Stroke::new(2.0_f32, mix(GOLD, CARD, 0.45)));
                }
                pnt.circle_filled(node, 7.5, CARD);
                pnt.circle_stroke(node, 7.5, Stroke::new(2.5_f32, if leg.open { GOLD } else { LIVE }));
                pnt.circle_filled(node, 3.0, col);
                let icon = Rect::from_center_size(pos2(r.left() + 42.0, r.center().y), vec2(24.0, 24.0));
                emblem(pnt, &prop.category_id(), icon, &self.art);
                let x = r.left() + 62.0;
                let right = r.right() - 34.0;
                let odds = pnt.layout_no_wrap(odds_text(leg.odds), bold_font(15.0), if leg.open { GOLD } else { DIM });
                let room = (right - 12.0 - odds.size().x - x).max(60.0);
                let pick = lines(pnt, &prop.outcomes[leg.outcome.min(prop.outcomes.len().saturating_sub(1))], bold_font(14.5), TEXT, room, 1);
                pnt.galley(pos2(x, r.top() + 5.0), pick, TEXT);
                let under = match (leg.open, leg.as_opened) {
                    (false, _) => format!("Betting closed: take it out · {}", prop.question),
                    (true, true) => format!("At the odds it opened at · {}", prop.question),
                    (true, false) => prop.question.clone(),
                };
                let q = lines(pnt, &under, FontId::proportional(11.5), if leg.open { MUTED } else { LIVE }, room, 1);
                pnt.galley(pos2(x, r.top() + 25.0), q, MUTED);
                pnt.galley(pos2(right - odds.size().x, r.center().y - odds.size().y / 2.0), odds, GOLD);
                // Take it out.
                let x_rect = Rect::from_center_size(pos2(r.right() - 14.0, r.center().y), vec2(24.0, 24.0));
                let x_resp = ui.interact(x_rect, ui.id().with(("slip-out", leg.prop)), Sense::click());
                let pnt = ui.painter();
                if x_resp.hovered() {
                    pnt.circle_filled(x_rect.center(), 11.0, ELEV2);
                }
                // (Drawn: the fonts here have no cross.)
                let (c, s) = (x_rect.center(), Stroke::new(1.6_f32, if x_resp.hovered() { TEXT } else { DIM }));
                pnt.line_segment([c + vec2(-3.6, -3.6), c + vec2(3.6, 3.6)], s);
                pnt.line_segment([c + vec2(-3.6, 3.6), c + vec2(3.6, -3.6)], s);
                if x_resp.on_hover_text("Take it out of the parlay").clicked() {
                    out = Some(leg.prop);
                } else if resp.on_hover_text(if leg.as_opened { format!("{} · In a parlay a pick pays the less of its odds now and the odds its prop opened at: bets since have left this one longer on the board.", p::american(leg.odds)) } else { p::american(leg.odds) }).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    open_prop = Some(leg.prop);
                }
            }
            if let Some((why, _)) = &self.slip.refused {
                ui.add_space(2.0);
                ui.label(RichText::new(why).size(12.5).color(LIVE));
            }
            if n == 0 {
                return;
            }
            ui.add_space(8.0);
            let mut entered = false;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                let field = egui::TextEdit::singleline(&mut self.slip.amount).desired_width(124.0).font(bold_font(17.0)).margin(egui::Margin::symmetric(10, 6)).hint_text("chips").background_color(ELEV);
                entered = ui.add(field).lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(4.0);
                for (label, v) in [("100", 100u64), ("1K", 1_000), ("10K", 10_000), ("100K", 100_000), ("1M", 1_000_000)] {
                    if ui.add(egui::Button::new(RichText::new(label).font(bold_font(12.5))).corner_radius(14)).clicked() {
                        self.slip.amount = v.to_string();
                    }
                }
                if let Some(h) = have {
                    if ui.add(egui::Button::new(RichText::new("All in").font(bold_font(12.5)).color(EMBER2)).corner_radius(14)).on_hover_text(format!("All {} of your chips", chips_text(h))).clicked() {
                        self.slip.amount = h.min(p::MAX_BET).to_string();
                    }
                }
            });
            ui.add_space(4.0);
            let closed = quote.legs.iter().any(|l| !l.open);
            let ok_stake = (p::MIN_BET..=p::MAX_BET).contains(&chips) && have.is_none_or(|h| chips <= h);
            let wins = quote.pays > chips;
            let ok = n >= p::PARLAY_MIN_LEGS && !closed && ok_stake && wins && self.slip.placing.is_none();
            if n < p::PARLAY_MIN_LEGS {
                ui.label(RichText::new("One more pick makes it a parlay.").size(12.5).color(MUTED));
            } else if chips < p::MIN_BET {
                ui.label(RichText::new(format!("At least {} chips.", p::MIN_BET)).size(12.5).color(MUTED));
            } else if chips > p::MAX_BET {
                ui.label(RichText::new(format!("At most {} chips a parlay.", chips_text(p::MAX_BET))).size(12.5).color(MUTED));
            } else if have.is_some_and(|h| chips > h) {
                ui.label(RichText::new(format!("You have {} chips.", chips_text(have.unwrap_or(0)))).size(12.5).color(LIVE));
            }
            if n >= p::PARLAY_MIN_LEGS && chips >= p::MIN_BET && chips <= p::MAX_BET {
                // (What it pays runs to its number as picks come and go, while a number that
                // size can run without losing its last digits.)
                let pays = if quote.pays < 8_000_000 { ui.ctx().animate_value_with_time(ui.id().with("slip-pays"), quote.pays as f32, 0.25).round() as u64 } else { quote.pays };
                let profit = pays as i64 - chips as i64;
                let times = if chips > 0 { pays as f64 / chips as f64 } else { 1.0 };
                let w = ((ui.available_width() - 16.0) / 3.0).clamp(84.0, 210.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for (label, short, value, c) in [("THE ODDS, MULTIPLIED", "ODDS", odds_both(times), TEXT), ("PAYS IF THEY ALL WIN", "PAYS", chips_text(pays), GOLD), ("PROFIT", "PROFIT", signed(profit), if profit > 0 { UP } else { MUTED })] {
                        let (r, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
                        let pnt = ui.painter();
                        pnt.text(pos2(r.left(), r.top() + 8.0), Align2::LEFT_CENTER, if w < 150.0 { short } else { label }, bold_font(10.0), DIM);
                        pnt.text(pos2(r.left(), r.top() + 27.0), Align2::LEFT_CENTER, value, bold_font(16.5), c);
                    }
                });
                if quote.capped {
                    ui.label(RichText::new("That's as much as these props' markets pay out on these picks: a smaller stake keeps more of the odds.").size(12.0).color(MUTED));
                } else if !wins {
                    ui.label(RichText::new("That would pay no more than its stake: these picks are all but sure, or the stake is far past what their markets take.").size(12.0).color(MUTED));
                }
            }
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                let label = if ok { format!("Place parlay: {} to win {}", chips_text(chips), chips_text(quote.pays)) } else { "Place parlay".to_string() };
                if ui.add_enabled(ok, primary_button(&label)).clicked() || (entered && ok) {
                    place = true;
                }
                if ui.button("Clear").clicked() {
                    clear = true;
                }
            });
        });
        if place {
            // (What it was shown to pay, less a little: if the odds moved more than that, the
            // server says so instead.)
            let legs: Vec<(u64, usize)> = quote.legs.iter().map(|l| (l.prop, l.outcome)).collect();
            ask(asks::parlay(&legs, chips, quote.pays as f64 * 0.98));
            self.slip.placing = Some(Instant::now());
        }
        if let Some(id) = out {
            self.slip.legs.retain(|l| l.0 != id);
            self.slip.refused = None;
        }
        if clear {
            self.slip.legs.clear();
            self.slip.refused = None;
        }
        if let Some(id) = open_prop {
            self.go(View::Prop(id));
        }
    }

    /// What there is to pick from: a category at a time, its games as on its own page, where
    /// a click on an outcome puts it in the slip.
    fn pick_from(&mut self, ui: &mut egui::Ui, now: i64) {
        let cats: Vec<(String, usize)> = self
            .m
            .categories()
            .into_iter()
            .filter(|(c, _)| p::parlay_kind(true, c))
            .map(|(c, _)| {
                let n = self.m.props.values().filter(|x| x.category_id() == c && p::parlay_takes(x, now).is_ok()).count();
                (c, n)
            })
            .filter(|c| c.1 > 0)
            .collect();
        section(ui, "Pick from");
        if cats.is_empty() {
            ui.label(RichText::new(if self.m.auto.as_ref().is_some_and(|a| a.on) { "Nothing is taking bets right now. More come by themselves." } else { "The casino's own props aren't on here: a parlay is made of those." }).size(13.0).color(MUTED));
            return;
        }
        if !self.slip_cat.as_ref().is_some_and(|c| cats.iter().any(|x| &x.0 == c)) {
            self.slip_cat = Some(cats[0].0.clone());
        }
        let chosen = self.slip_cat.clone().unwrap_or_default();
        egui::ScrollArea::horizontal().id_salt("pick-cats").auto_shrink([false, true]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
            ui.horizontal(|ui| {
                for (c, n) in &cats {
                    let on = *c == chosen;
                    let has = self.slip.legs.iter().filter(|l| self.m.props.get(&l.0).is_some_and(|x| x.category_id() == *c)).count();
                    let name = if has > 0 { format!("{}  {n} · {has} in", p::category_name(c)) } else { format!("{}  {n}", p::category_name(c)) };
                    let label = RichText::new(name).font(bold_font(13.0)).color(if on { ON_EMBER } else { TEXT });
                    if ui.add(egui::Button::new(label).fill(if on { EMBER2 } else { ELEV }).corner_radius(14)).clicked() {
                        self.slip_cat = Some(c.clone());
                    }
                }
            });
        });
        ui.add_space(8.0);
        let events: Vec<(String, i64, Vec<PropView>)> = self
            .m
            .events(&chosen)
            .into_iter()
            .map(|e| (e.name, e.starts, e.props.into_iter().filter(|x| p::parlay_takes(x, now).is_ok()).cloned().collect::<Vec<_>>()))
            .filter(|e| !e.2.is_empty())
            .take(60)
            .collect();
        for (name, starts, props) in &events {
            self.event_card(ui, &chosen, name, *starts, props, now);
            ui.add_space(8.0);
        }
    }

    /// A parlay of mine as a ticket: its picks down a chain (each as it came out), the stake
    /// and what it pays (or paid), torn along a dotted line.
    pub fn ticket(&mut self, ui: &mut egui::Ui, t: &ParlayView, now: i64) {
        let (won, left) = t.won_of();
        let (word, col) = match t.state {
            ParlayState::Open if won > 0 => (format!("Riding · {won} of {left} in"), EMBER2),
            ParlayState::Open => ("Riding".to_string(), EMBER2),
            ParlayState::Won => ("Won".to_string(), GOLD),
            ParlayState::Lost => ("Lost".to_string(), DOWN),
            ParlayState::Void => ("Called off".to_string(), MUTED),
        };
        let mut open_prop: Option<u64> = None;
        // (Where it tears: the dotted line's height, for the notches at its ends.)
        let mut tear = 0.0f32;
        let edge = if t.state == ParlayState::Won { mix(CARD_EDGE, GOLD, 0.6) } else { CARD_EDGE };
        let framed = card().stroke(Stroke::new(1.0_f32, edge)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(title(t)).font(bold_font(14.5)).color(TEXT));
                ui.label(RichText::new(format!("· {}", ago(now - t.placed_at))).size(12.0).color(DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let g = ui.painter().layout_no_wrap(word.clone(), bold_font(11.5), col);
                    let (r, _) = ui.allocate_exact_size(vec2(g.size().x + 18.0, 20.0), Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius::same(10), col.gamma_multiply(0.16));
                    ui.painter().galley(pos2(r.left() + 9.0, r.center().y - g.size().y / 2.0), g, col);
                });
            });
            ui.add_space(4.0);
            let n = t.legs.len();
            for (k, leg) in t.legs.iter().enumerate() {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), Sense::click());
                let known = self.m.props.contains_key(&leg.prop);
                let pnt = ui.painter();
                if known && resp.hovered() {
                    pnt.rect_filled(r, CornerRadius::same(8), ELEV);
                }
                let node = pos2(r.left() + 13.0, r.center().y);
                if k + 1 < n {
                    pnt.line_segment([node + vec2(0.0, 8.0), node + vec2(0.0, 28.0)], Stroke::new(2.0_f32, LINE));
                }
                leg_mark(pnt, node, leg.state);
                let icon = Rect::from_center_size(pos2(r.left() + 40.0, r.center().y), vec2(20.0, 20.0));
                emblem(pnt, &leg.category, icon, &self.art);
                let gone = matches!(leg.state, LegState::Lost | LegState::Void);
                let x = r.left() + 58.0;
                let odds = pnt.layout_no_wrap(if leg.state == LegState::Void { "×1.00".to_string() } else { odds_text(leg.odds) }, bold_font(13.5), if gone { DIM } else { GOLD });
                let room = (r.right() - 10.0 - odds.size().x - x).max(60.0);
                let pick = lines(pnt, &leg.pick, bold_font(13.5), if gone { MUTED } else { TEXT }, room, 1);
                pnt.galley(pos2(x, r.top() + 2.0), pick, TEXT);
                // (Its prop's time as it is now, where this window has the prop: a game
                // that's put back takes its picks' times with it.)
                let closes = self.m.props.get(&leg.prop).map(|x| x.closes_at).unwrap_or(leg.closes_at);
                let under = leg_words(leg, closes, now, t.state != ParlayState::Open);
                let q = lines(pnt, &under, FontId::proportional(11.5), MUTED, room, 1);
                pnt.galley(pos2(x, r.top() + 20.0), q, MUTED);
                pnt.galley(pos2(r.right() - 4.0 - odds.size().x, r.center().y - odds.size().y / 2.0), odds, GOLD);
                if known && resp.on_hover_text(&leg.question).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    open_prop = Some(leg.prop);
                }
            }
            // Torn along here: the stake on one side, what it pays on the other.
            ui.add_space(4.0);
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
            tear = r.center().y;
            let mut x = r.left();
            // (However wide it's given: a line of at most a few hundred dashes.)
            while x < r.right().min(r.left() + 4000.0) {
                ui.painter().line_segment([pos2(x, r.center().y), pos2((x + 5.0).min(r.right()), r.center().y)], Stroke::new(1.0_f32, LINE));
                x += 9.0;
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} chips", chips_text(t.chips))).font(bold_font(13.0)).color(BODY));
                let times = if t.chips > 0 { t.pays as f64 / t.chips as f64 } else { 1.0 };
                if t.state == ParlayState::Open {
                    ui.label(RichText::new(format!("at {}", odds_both(times))).size(12.5).color(MUTED));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (text, c) = match t.state {
                        ParlayState::Open => (format!("pays {}", chips_text(t.pays)), GOLD),
                        ParlayState::Won => (format!("paid {} ({})", chips_text(t.paid), signed(t.paid as i64 - t.chips as i64)), UP),
                        ParlayState::Lost => ("didn't come in".to_string(), MUTED),
                        ParlayState::Void => ("chips back".to_string(), MUTED),
                    };
                    ui.label(RichText::new(text).font(bold_font(14.0)).color(c));
                });
            });
        });
        // The ticket's notches, either side of where it tears.
        let r = framed.response.rect;
        let y = if tear > 0.0 { tear } else { r.bottom() - 40.0 };
        for x in [r.left(), r.right()] {
            ui.painter().circle_filled(pos2(x, y), 6.0, MAIN);
            ui.painter().circle_stroke(pos2(x, y), 6.0, Stroke::new(1.0_f32, edge));
        }
        ui.painter().rect_filled(Rect::from_min_max(pos2(r.left() - 8.0, y - 7.0), pos2(r.left() - 0.5, y + 7.0)), CornerRadius::ZERO, MAIN);
        ui.painter().rect_filled(Rect::from_min_max(pos2(r.right() + 0.5, y - 7.0), pos2(r.right() + 8.0, y + 7.0)), CornerRadius::ZERO, MAIN);
        if let Some(id) = open_prop {
            self.go(View::Prop(id));
        }
        ui.add_space(6.0);
    }

    /// My parlays that are riding, in short, on My bets (the rest are on their own page).
    pub fn parlays_in_mine(&mut self, ui: &mut egui::Ui, now: i64) {
        let riding: Vec<ParlayView> = self.m.parlays.iter().filter(|t| t.state == ParlayState::Open).cloned().collect();
        if riding.is_empty() {
            return;
        }
        section(ui, "Parlays riding");
        for t in riding.iter().take(6) {
            self.ticket(ui, t, now);
        }
        if ui.link(RichText::new("All your parlays").size(12.5)).clicked() {
            self.back.clear();
            self.parlay_mine = true;
            self.go(View::Parlay);
        }
    }
}

/// What's under a ticket's pick: its question, and how it came out once it has (`closes_at`:
/// when its prop stops taking bets; `over`: the parlay itself is over, lost at another leg: a
/// pick still to come out no longer counts).
fn leg_words(leg: &ParlayLeg, closes_at: i64, now: i64, over: bool) -> String {
    let what = match leg.state {
        LegState::Pending if over => "still to come out, and no longer counted".to_string(),
        LegState::Pending if closes_at > 0 && now < closes_at => format!("closes in {}", words::countdown(closes_at - now)),
        LegState::Pending => "waiting for its result".to_string(),
        LegState::Won => "won".to_string(),
        LegState::Lost => "lost".to_string(),
        LegState::Void => "called off".to_string(),
    };
    let note = if leg.note.is_empty() { String::new() } else { format!(": {}", leg.note) };
    format!("{} · {what}{note}", model::question_short(&leg.question))
}

/// A pick's ring on a ticket's chain: empty while it waits, a tick, a cross, or a dash.
fn leg_mark(pnt: &egui::Painter, c: egui::Pos2, state: LegState) {
    let col = match state {
        LegState::Pending => MUTED,
        LegState::Won => UP,
        LegState::Lost => DOWN,
        LegState::Void => DIM,
    };
    pnt.circle_filled(c, 7.5, if state == LegState::Pending { CARD } else { col.gamma_multiply(0.22) });
    pnt.circle_stroke(c, 7.5, Stroke::new(2.0_f32, col));
    let s = Stroke::new(1.8_f32, col);
    match state {
        LegState::Pending => {}
        LegState::Won => {
            pnt.add(Shape::line(vec![c + vec2(-3.4, 0.2), c + vec2(-0.9, 2.7), c + vec2(3.5, -2.5)], s));
        }
        LegState::Lost => {
            pnt.line_segment([c + vec2(-2.8, -2.8), c + vec2(2.8, 2.8)], s);
            pnt.line_segment([c + vec2(-2.8, 2.8), c + vec2(2.8, -2.8)], s);
        }
        LegState::Void => {
            pnt.line_segment([c + vec2(-3.2, 0.0), c + vec2(3.2, 0.0)], s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odds_read_as_a_slip_has_them() {
        assert_eq!((odds_text(1.909).as_str(), odds_text(2.5).as_str(), odds_text(137.4).as_str(), odds_text(12_345.6).as_str()), ("×1.91", "×2.50", "×137", "×12,346"));
        assert_eq!((odds_both(2.5).as_str(), odds_both(1.0 + 100.0 / 110.0).as_str(), odds_both(1.0).as_str()), ("×2.50 (+150)", "×1.91 (-110)", "×1.00"));
        let t = ParlayView { legs: vec![ParlayLeg::default(); 3], ..Default::default() };
        assert_eq!(title(&t), "3-leg parlay");
        // A ticket's pick: waiting, in, and on a ticket that's lost already.
        let mut leg = ParlayLeg { question: "Who wins: Jets at Dolphins?".into(), closes_at: 10_000, ..Default::default() };
        assert!(leg_words(&leg, leg.closes_at, 20_000, false).ends_with("waiting for its result"));
        assert!(leg_words(&leg, 20_000 + 90_000, 20_000, false).contains("closes in"), "the game was put back");
        assert!(leg_words(&leg, leg.closes_at, 20_000, true).ends_with("no longer counted"));
        leg.state = LegState::Won;
        leg.note = "Dolphins 20, Jets 3".into();
        assert!(leg_words(&leg, leg.closes_at, 20_000, true).ends_with("won: Dolphins 20, Jets 3"));
    }
}
