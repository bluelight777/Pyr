//! A prop's own page (how its odds went, each outcome with Bet, what I hold, how it
//! settles), my bets, the best at props, results, and the two windows (a prop of my own; the
//! casino's own props, for whoever manages the server).

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke, pos2, vec2};
use pyr_poker::props::{self as p, PropState, PropView};

use crate::asks::{self, ask};
use crate::look::*;
use crate::model::{Model, stake_and_worth};
use crate::views::{card, lines, section, standing};
use crate::words::{self, chips_text, count, local_when, mid_sentence, pct, signed, times};
use crate::{App, Betting, View, note};

impl App {
    // ------------------------------------------------------------ a prop's page

    pub fn prop_page(&mut self, ui: &mut egui::Ui, id: u64, now: i64) {
        let Some(prop) = self.m.props.get(&id).cloned() else { return };
        // How its odds went comes when it's looked up: when its page opens, and every so often.
        if self.looked.is_none_or(|(was, when)| was != id || when.elapsed() > Duration::from_secs(45)) {
            ask(asks::look(id));
            self.looked = Some((id, Instant::now()));
        }
        let cat = prop.category_id();
        let (col, _) = category_look(&cat);
        ui.horizontal(|ui| {
            if ui.button(RichText::new("‹ Back").font(bold_font(13.0))).clicked() {
                self.go_back();
            }
            let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
            emblem(ui.painter(), &cat, r, &self.art);
            let name = ui.add(egui::Label::new(RichText::new(p::category_name(&cat)).font(bold_font(12.5)).color(mix(col, Color32::WHITE, 0.3))).sense(Sense::click()));
            if resp.clicked() || name.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.back.clear();
                self.go(View::Category(cat.clone()));
            }
            let of = if !prop.event_name.is_empty() { format!("· {}", prop.event_name) } else { self.m.channel_name(prop.channel_id).map(|c| format!("· #{c}")).unwrap_or_default() };
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // (A Pyr from before 0.15 doesn't know how.)
                if !self.m.old_pyr && ui.button("Share").on_hover_text("Post its card in the channel showing in Pyr").clicked() {
                    ask(asks::share(prop.id));
                }
                // (What it's of, in the room that's left.)
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(of).size(12.5).color(MUTED)).truncate());
                });
            });
        });
        ui.add_space(6.0);
        card().inner_margin(egui::Margin::symmetric(20, 16)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Its category's picture across the top of the card, falling away to the left
            // (where the question is) and down.
            if let Some((t, focus)) = self.art.category(&cat) {
                let m = ui.max_rect();
                let pic = Rect::from_min_max(pos2(m.left() - 19.0, m.top() - 15.0), pos2(m.right() + 19.0, m.top() + 124.0));
                let pnt = ui.painter().with_clip_rect(pic);
                pnt.add(cover_rounded(t, pic, 11.0, focus, Color32::from_gray(170)));
                pnt.add(gradient(pic, 11.0, CARD, CARD.gamma_multiply(0.12)));
                pnt.add(gradient_down(pic, 11.0, Color32::TRANSPARENT, CARD));
            }
            ui.add(egui::Label::new(RichText::new(&prop.question).font(bold_font(22.0)).color(Color32::WHITE)).wrap());
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                let (text, c) = standing(&prop, now);
                ui.label(RichText::new(text).font(bold_font(13.0)).color(c));
                if prop.note.is_empty() {
                    if prop.starts_at > 0 && Model::live(&prop) {
                        // (A game starts as its betting closes. A price is taken, a deadline
                        // passes, some time after: that's when it's decided.)
                        let what = if prop.starts_at <= prop.closes_at + 60_000 { if now >= prop.starts_at { "started" } else { "starts" } } else { "decided" };
                        ui.label(RichText::new(format!("· {what} {}", mid_sentence(&local_when(prop.starts_at, now)))).size(13.0).color(MUTED));
                    }
                } else {
                    ui.label(RichText::new(format!("· {}", prop.note)).size(13.0).color(BODY));
                }
            });
            if !prop.detail.is_empty() && Model::live(&prop) {
                ui.label(RichText::new(&prop.detail).size(12.5).color(MUTED));
            }
            ui.add_space(8.0);
            chart(ui, &prop, now);
            ui.add_space(8.0);
            self.outcomes(ui, &prop, now);
            self.holdings(ui, &prop, now);
            self.admin(ui, &prop);
        });
        // The other props of its game (or its day, its coin): a click away.
        let mut others: Vec<PropView> = match prop.event.is_empty() {
            true => Vec::new(),
            false => self.m.props.values().filter(|x| x.event == prop.event && x.id != prop.id && Model::live(x)).cloned().collect(),
        };
        others.sort_by_key(|x| (std::cmp::Reverse(x.featured), x.id));
        others.truncate(12);
        if !others.is_empty() {
            ui.add_space(8.0);
            let sport = SPORTS.contains(&cat.as_str());
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                let heading = match (sport, prop.event_name.is_empty()) {
                    (true, _) => "MORE ON THIS GAME".to_string(),
                    (false, false) => format!("MORE ON {}", prop.event_name.to_uppercase()),
                    (false, true) => "MORE LIKE IT".to_string(),
                };
                ui.label(RichText::new(heading).size(10.5).strong().color(DIM));
                for other in &others {
                    self.prop_row(ui, other, crate::model::row_label(other), sport, !sport, !sport, now);
                }
            });
        }
        ui.add_space(8.0);
        // How it settles, and what's on it.
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            if prop.house {
                ui.label(RichText::new("HOW IT SETTLES").size(10.5).strong().color(DIM));
                let by = if prop.source.is_empty() { "By itself.".to_string() } else { format!("By itself, from {}.", prop.source) };
                ui.add(egui::Label::new(RichText::new(format!("{by} {}", prop.rules)).size(13.0).color(BODY)).wrap());
                ui.add_space(6.0);
            } else {
                ui.label(RichText::new("HOW IT SETTLES").size(10.5).strong().color(DIM));
                ui.add(egui::Label::new(RichText::new(format!("{} opened it, put up {} chips to back its odds, and settles it (or a moderator does). A winning share pays a chip.", prop.author_name, chips_text(prop.backing))).size(13.0).color(BODY)).wrap());
                ui.add_space(6.0);
            }
            ui.label(RichText::new(format!("{} · {} · {}", count(prop.pool, "chip in", "chips in"), count(prop.bets as u64, "bet", "bets"), count(prop.bettors as u64, "person", "people"))).size(12.5).color(MUTED));
            if prop.state == PropState::Settled && !prop.winners.is_empty() {
                ui.label(RichText::new(format!("Paid: {}", prop.winners.iter().map(|(n, c)| format!("{n} +{}", chips_text(*c))).collect::<Vec<_>>().join(" · "))).size(12.5).color(GOLD));
            }
            if !prop.latest.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new("LATEST BETS").size(10.5).strong().color(DIM));
                for l in prop.latest.iter().take(8) {
                    let o = prop.outcomes.get(l.outcome).cloned().unwrap_or_default();
                    ui.horizontal(|ui| {
                        let c = color_of(&prop.outcomes, l.outcome);
                        let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 4.0, c);
                        let what = if l.chips >= 0 { format!("{} bet {} on {o}", l.name, chips_text(l.chips as u64)) } else { format!("{} cashed out {o} for {}", l.name, chips_text(l.chips.unsigned_abs())) };
                        ui.label(RichText::new(what).size(12.5).color(BODY));
                        ui.label(RichText::new(words::ago(now - l.at)).size(11.5).color(DIM));
                    });
                }
            }
        });
    }

    /// Each outcome: its chip, name and odds, a bar, and Bet (or, settled, who won).
    fn outcomes(&mut self, ui: &mut egui::Ui, prop: &PropView, now: i64) {
        let prices = prop.prices();
        let open = prop.open_at(now);
        // (Whoever opened a prop settles it, so doesn't bet on it. Nobody opened the casino's.)
        let mine = !prop.house && prop.author == self.m.me;
        if prop.state == PropState::Settled {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
                cover(ui.painter(), self.art.won.as_ref(), r, Color32::WHITE);
                ui.vertical(|ui| {
                    let won = prop.winner.and_then(|w| prop.outcomes.get(w)).cloned().unwrap_or_default();
                    ui.label(RichText::new(format!("Settled: {won}")).font(bold_font(17.0)).color(GOLD));
                    let who = if prop.winners.is_empty() { "Nobody had chips on it.".to_string() } else { "Winners were paid a chip a share.".to_string() };
                    ui.label(RichText::new(who).size(12.5).color(MUTED));
                });
            });
            ui.add_space(6.0);
        } else if prop.state == PropState::Void {
            ui.label(RichText::new("Called off: everyone's chips went back to them.").italics().color(MUTED));
            ui.add_space(6.0);
        } else if !open {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                cover(ui.painter(), self.art.lock.as_ref(), r, Color32::WHITE);
                let words = if prop.house { "Betting has closed: it settles by itself when the result is in." } else { "Betting has closed: it pays out when it's settled." };
                ui.label(RichText::new(words).color(MUTED));
            });
            ui.add_space(4.0);
        }
        for (i, name) in prop.outcomes.iter().enumerate() {
            let price = prices.get(i).copied().unwrap_or(0.0);
            let won = prop.winner == Some(i);
            let faded = prop.state == PropState::Settled && !won;
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::hover());
            let pnt = ui.painter();
            let chip = Rect::from_center_size(pos2(row.left() + 18.0, row.center().y), vec2(34.0, 34.0));
            let col = color_of(&prop.outcomes, i);
            match self.art.chips.get(hue(&prop.outcomes, i)) {
                Some(t) => {
                    pnt.image(t.id(), chip, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), if faded { Color32::from_gray(90) } else { Color32::WHITE });
                }
                None => {
                    pnt.circle_filled(chip.center(), 16.0, col);
                }
            }
            let x = row.left() + 46.0;
            let bar_w = (row.width() - 46.0 - 100.0).max(60.0);
            // (Its name in the room its odds leave it.)
            let named = lines(pnt, name, bold_font(15.0), if faded { DIM } else { TEXT }, bar_w - if bar_w > 260.0 { 112.0 } else { 62.0 }, 1);
            pnt.galley(pos2(x, row.top() + 13.0 - named.size().y / 2.0), named, TEXT);
            // Odds that just moved: an arrow beside them for a moment.
            let mut pct_col = if faded { DIM } else if won { GOLD } else { col };
            if let Some(m) = self.m.moved.get(&(prop.id, i)) {
                let left = fading(m.at, 2.0);
                if left > 0.0 {
                    let up = price as f32 > m.from;
                    pct_col = mix(pct_col, if up { UP } else { DOWN }, left);
                    pnt.text(pos2(x + bar_w - 40.0, row.top() + 13.0), Align2::RIGHT_CENTER, if up { "▲" } else { "▼" }, FontId::proportional(10.0), (if up { UP } else { DOWN }).gamma_multiply(left));
                    ui.ctx().request_repaint_after(Duration::from_millis(40));
                }
            }
            // What a chip on it pays, about (a big bet moves the odds, and pays a little less).
            if open && bar_w > 260.0 {
                pnt.text(pos2(x + bar_w - 58.0, row.top() + 13.5), Align2::RIGHT_CENTER, times(price), FontId::proportional(12.0), DIM);
            }
            pnt.text(pos2(x + bar_w, row.top() + 13.0), Align2::RIGHT_CENTER, if matches!(prop.state, PropState::Settled) { if won { "won".to_string() } else { String::new() } } else { pct(price) }, bold_font(15.0), pct_col);
            let bar = Rect::from_min_size(pos2(x, row.top() + 27.0), vec2(bar_w, 7.0));
            let shown = ui.ctx().animate_value_with_time(ui.id().with(("page-bar", prop.id, i)), price as f32, 0.35);
            pnt.rect_filled(bar, CornerRadius::same(3), Color32::from_white_alpha(18));
            pnt.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * shown, bar.height())), CornerRadius::same(3), if faded { Color32::from_gray(70) } else { col });
            if open && !mine {
                let b = Rect::from_min_size(pos2(row.right() - 84.0, row.center().y - 16.0), vec2(84.0, 32.0));
                let picked = self.betting.as_ref().is_some_and(|bt| bt.id == prop.id && bt.outcome == i);
                let button = egui::Button::new(RichText::new("Bet").font(bold_font(13.5)).color(ON_EMBER)).fill(col).corner_radius(16);
                let button = if picked { button.stroke(Stroke::new(2.0_f32, Color32::WHITE)) } else { button };
                if ui.put(b, button).on_hover_text(format!("Bet chips on {name}: each share pays a chip if it wins")).clicked() {
                    self.betting = if picked { None } else { Some(Betting { id: prop.id, outcome: i, amount: self.betting.as_ref().map(|b| b.amount.clone()).unwrap_or_else(|| "1000".into()) }) };
                }
            }
            if self.betting.as_ref().is_some_and(|bt| bt.id == prop.id && bt.outcome == i) {
                if open && !mine {
                    self.bet_panel(ui, prop, i);
                } else {
                    self.betting = None;
                }
            }
        }
    }

    /// Picking how much to bet on an outcome: a box and chips buttons, what it pays and where
    /// it leaves the odds, and the bet.
    fn bet_panel(&mut self, ui: &mut egui::Ui, prop: &PropView, outcome: usize) {
        let have = self.m.chips;
        let col = color_of(&prop.outcomes, outcome);
        let Some(bt) = self.betting.as_mut() else { return };
        let mut done = false;
        egui::Frame::new().fill(Color32::from_black_alpha(110)).stroke(Stroke::new(1.0_f32, col.gamma_multiply(0.55))).corner_radius(10).inner_margin(egui::Margin::symmetric(14, 12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            // (Enter in the box places the bet.)
            let mut entered = false;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                let field = egui::TextEdit::singleline(&mut bt.amount).desired_width(124.0).font(bold_font(17.0)).margin(egui::Margin::symmetric(10, 6)).hint_text("chips");
                entered = ui.add(field).lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                ui.add_space(4.0);
                for (label, n) in [("100", 100u64), ("1K", 1_000), ("10K", 10_000), ("100K", 100_000), ("1M", 1_000_000)] {
                    if ui.add(egui::Button::new(RichText::new(label).font(bold_font(12.5))).corner_radius(14)).clicked() {
                        bt.amount = n.to_string();
                    }
                }
                if let Some(h) = have {
                    if ui.add(egui::Button::new(RichText::new("All in").font(bold_font(12.5)).color(EMBER2)).corner_radius(14)).on_hover_text(format!("All {} of your chips", chips_text(h))).clicked() {
                        bt.amount = h.min(p::MAX_BET).to_string();
                    }
                }
            });
            ui.add_space(4.0);
            let chips: u64 = bt.amount.replace([',', ' '], "").parse().unwrap_or(0);
            let shares = if chips > 0 { p::shares_for(&prop.q, prop.b, outcome, chips as f64) } else { 0.0 };
            let name = prop.outcomes.get(outcome).cloned().unwrap_or_default();
            let ok = (p::MIN_BET..=p::MAX_BET).contains(&chips) && have.is_none_or(|h| chips <= h);
            if chips < p::MIN_BET {
                ui.label(RichText::new(format!("At least {} chips.", p::MIN_BET)).size(12.5).color(MUTED));
            } else if chips > p::MAX_BET {
                ui.label(RichText::new(format!("At most {} chips a bet.", chips_text(p::MAX_BET))).size(12.5).color(MUTED));
            } else if have.is_some_and(|h| chips > h) {
                ui.label(RichText::new(format!("You have {} chips.", chips_text(have.unwrap_or(0)))).size(12.5).color(LIVE));
            } else {
                let pays = shares.floor() as u64;
                let profit = pays as i64 - chips as i64;
                // (What the odds will read once the bet's in.)
                let before = prop.prices().get(outcome).copied().unwrap_or(0.0);
                let mut q = prop.q.clone();
                q[outcome] += shares;
                let after = p::prices(&q, prop.b)[outcome];
                let w = ((ui.available_width() - 16.0) / 3.0).clamp(84.0, 190.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    for (label, short, value, c) in [("PAYS IF IT WINS", "PAYS", chips_text(pays), GOLD), ("PROFIT", "PROFIT", signed(profit), if profit >= 0 { UP } else { DOWN }), ("ITS ODDS", "ODDS", format!("{} → {}", pct(before), pct(after)), TEXT)] {
                        let (r, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
                        let pnt = ui.painter();
                        pnt.text(pos2(r.left(), r.top() + 8.0), Align2::LEFT_CENTER, if w < 110.0 { short } else { label }, bold_font(10.0), DIM);
                        pnt.text(pos2(r.left(), r.top() + 27.0), Align2::LEFT_CENTER, value, bold_font(16.5), c);
                    }
                });
            }
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                let label = if ok { format!("Bet {} on {name}", chips_text(chips)) } else { "Place bet".to_string() };
                if ui.add_enabled(ok, primary_button(&label)).clicked() || (entered && ok) {
                    // (What it was shown to pay, less a little: if the odds moved more than
                    // that, the server says so instead.)
                    ask(asks::bet(prop.id, outcome, chips, shares * 0.98));
                    done = true;
                }
                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
        });
        if done {
            self.betting = None;
        }
        ui.add_space(4.0);
    }

    /// What I hold, worth now, with Cash out.
    fn holdings(&mut self, ui: &mut egui::Ui, prop: &PropView, now: i64) {
        if prop.mine.is_empty() {
            return;
        }
        ui.add_space(6.0);
        egui::Frame::new().fill(Color32::from_rgb(0x2b, 0x23, 0x12)).corner_radius(10).inner_margin(egui::Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("YOURS").size(10.5).strong().color(GOLD));
            if !Model::live(prop) {
                let (cost, worth) = stake_and_worth(prop);
                let net = worth as i64 - cost as i64;
                let words = match prop.state {
                    PropState::Void => format!("Called off: your {} chips came back.", chips_text(cost)),
                    _ if worth > 0 => format!("You put in {} and it paid {} ({}).", chips_text(cost), chips_text(worth), signed(net)),
                    _ => format!("You put in {} and it didn't win.", chips_text(cost)),
                };
                ui.label(RichText::new(words).size(13.0).color(if net > 0 { UP } else { BODY }));
                return;
            }
            for h in prop.mine.iter().filter(|h| h.shares > 1e-9) {
                let name = prop.outcomes.get(h.outcome).cloned().unwrap_or_default();
                let worth = prop.worth(h).max(0.0).floor() as u64;
                let diff = worth as i64 - h.cost as i64;
                ui.horizontal_wrapped(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 4.0, color_of(&prop.outcomes, h.outcome));
                    ui.label(RichText::new(format!("{name}: pays {} if it wins", chips_text(h.shares.floor() as u64))).font(bold_font(13.0)).color(TEXT));
                    ui.label(RichText::new(format!("· paid {} · worth {} now", chips_text(h.cost), chips_text(worth))).size(12.5).color(MUTED));
                    ui.label(RichText::new(format!("({})", signed(diff))).size(12.5).color(if diff >= 0 { UP } else { DOWN }));
                    if prop.open_at(now) && ui.small_button("Cash out").on_hover_text(format!("Sell your {name} back for {} chips", chips_text(worth))).clicked() {
                        ask(asks::cash_out(prop.id, h.outcome));
                    }
                });
            }
        });
    }

    /// Settle (pick what happened) and Call off, for whoever may.
    fn admin(&mut self, ui: &mut egui::Ui, prop: &PropView) {
        if !prop.can_settle || !Model::live(prop) {
            return;
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            // (One of the casino's own settles by itself: a manager may only call it off.)
            if !prop.house {
                ui.menu_button(RichText::new("Settle…").font(bold_font(13.0)), |ui| {
                    ui.label(RichText::new("What happened? Winners are paid a chip a share.").size(12.0).color(DIM));
                    for (i, o) in prop.outcomes.iter().enumerate() {
                        if ui.button(o).clicked() {
                            ask(asks::settle(prop.id, i));
                            ui.close();
                        }
                    }
                });
            }
            let hint = if prop.house { "Everyone gets back what they put in" } else { "Everyone gets back what they put in (once others have bet, only a moderator can)" };
            if ui.small_button("Call off").on_hover_text(hint).clicked() {
                ask(asks::void(prop.id));
            }
        });
    }

    // ------------------------------------------------------------ my bets

    pub fn mine_page(&mut self, ui: &mut egui::Ui, now: i64) {
        let live: Vec<PropView> = self.m.mine_live().into_iter().cloned().collect();
        let over: Vec<PropView> = self.m.mine_over().into_iter().take(60).cloned().collect();
        ui.add_space(4.0);
        ui.label(RichText::new("My bets").font(bold_font(23.0)).color(TEXT));
        ui.add_space(8.0);
        let (cost, worth) = live.iter().map(stake_and_worth).fold((0u64, 0u64), |a, b| (a.0 + b.0, a.1 + b.1));
        let (won, lost) = over.iter().map(stake_and_worth).fold((0i64, 0i64), |a, (c, w)| if w >= c { (a.0 + (w - c) as i64, a.1) } else { (a.0, a.1 + (c - w) as i64) });
        let tiles = [
            ("IN PLAY", chips_text(cost), TEXT),
            ("WORTH NOW", chips_text(worth), TEXT),
            ("UP OR DOWN", signed(worth as i64 - cost as i64), if worth > cost { UP } else if worth < cost { DOWN } else { TEXT }),
            ("WON SO FAR", signed(won - lost), if won > lost { UP } else if won < lost { DOWN } else { TEXT }),
        ];
        let gap = 10.0;
        let across = if ui.available_width() >= 600.0 { 4 } else { 2 };
        let w = ((ui.available_width() - gap * (across - 1) as f32) / across as f32).floor();
        for row in tiles.chunks(across) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (label, value, col) in row {
                    let (r, _) = ui.allocate_exact_size(vec2(w, 64.0), Sense::hover());
                    let pnt = ui.painter();
                    pnt.rect_filled(r, CornerRadius::same(10), CARD);
                    pnt.rect_stroke(r, CornerRadius::same(10), Stroke::new(1.0_f32, CARD_EDGE), egui::StrokeKind::Inside);
                    pnt.text(pos2(r.left() + 14.0, r.top() + 18.0), Align2::LEFT_CENTER, *label, bold_font(10.5), DIM);
                    pnt.text(pos2(r.left() + 14.0, r.top() + 41.0), Align2::LEFT_CENTER, value, bold_font(19.0), *col);
                }
            });
            ui.add_space(gap - 6.0);
        }
        if live.is_empty() && over.is_empty() {
            return note(ui, "No bets of yours yet", "Bet on a prop and it shows here, with what it's worth now.");
        }
        if !live.is_empty() {
            section(ui, "To come out");
            for prop in &live {
                self.mine_row(ui, prop, now);
            }
        }
        if !over.is_empty() {
            section(ui, "Over");
            for prop in &over {
                self.mine_row(ui, prop, now);
            }
        }
    }

    /// One of my bets: the prop, what I hold, what it's worth (or what it paid), Cash out.
    fn mine_row(&mut self, ui: &mut egui::Ui, prop: &PropView, now: i64) {
        let (cost, worth) = stake_and_worth(prop);
        let net = worth as i64 - cost as i64;
        // (The row takes clicks from the start, so that Cash out, drawn in it after, is over
        // it and takes its own.)
        let row = ui.scope_builder(egui::UiBuilder::new().id_salt(("mine", prop.id)).sense(Sense::click()), |ui| {
            card().inner_margin(egui::Margin::symmetric(14, 9)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                emblem(ui.painter(), &prop.category_id(), r, &self.art);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.set_max_width(ui.available_width() - 190.0);
                    ui.add(egui::Label::new(RichText::new(&prop.question).font(bold_font(13.5)).color(TEXT)).truncate());
                    let held: Vec<String> = prop.mine.iter().filter(|h| h.shares > 1e-9 || h.cost > 0).map(|h| format!("{} ({} chips)", prop.outcomes.get(h.outcome).cloned().unwrap_or_default(), chips_text(h.cost))).collect();
                    let (text, c) = standing(prop, now);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(held.join(", ")).size(12.0).color(MUTED));
                        ui.label(RichText::new(format!("· {text}")).size(12.0).color(c));
                    });
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if prop.open_at(now) {
                        let held: Vec<_> = prop.mine.iter().filter(|h| h.shares > 1e-9).collect();
                        let worth_of = |h: &p::Holding| chips_text(prop.worth(h).max(0.0).floor() as u64);
                        let name_of = |h: &p::Holding| prop.outcomes.get(h.outcome).cloned().unwrap_or_default();
                        if let [h] = held.as_slice() {
                            if ui.small_button("Cash out").on_hover_text(format!("Sell your {} back for {} chips", name_of(h), worth_of(h))).clicked() {
                                ask(asks::cash_out(prop.id, h.outcome));
                            }
                        } else if held.len() > 1 {
                            // (Which of them: each sells by itself.)
                            ui.menu_button(RichText::new("Cash out…").size(12.5), |ui| {
                                for h in &held {
                                    if ui.button(format!("{}: {} chips", name_of(h), worth_of(h))).clicked() {
                                        ask(asks::cash_out(prop.id, h.outcome));
                                        ui.close();
                                    }
                                }
                            });
                        }
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                            ui.label(RichText::new(signed(net)).font(bold_font(15.0)).color(if net > 0 { UP } else if net < 0 { DOWN } else { MUTED }));
                            let what = match prop.state {
                                PropState::Open | PropState::Closed => format!("worth {}", chips_text(worth)),
                                PropState::Settled if worth == 0 => "didn't win".to_string(),
                                PropState::Settled => format!("paid {}", chips_text(worth)),
                                PropState::Void => "chips back".to_string(),
                            };
                            ui.label(RichText::new(what).size(11.5).color(DIM));
                        });
                    });
                });
            });
            });
        });
        if row.response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.go(View::Prop(prop.id));
        }
        ui.add_space(4.0);
    }

    // ------------------------------------------------------------ the best at props

    pub fn leaders(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.label(RichText::new("Best at props").font(bold_font(23.0)).color(TEXT));
        ui.label(RichText::new("What everyone won less what they put in, on props that are over.").size(13.0).color(MUTED));
        ui.add_space(8.0);
        if self.m.old_pyr {
            return note(ui, "Update Pyr for this", "The board of the best at props came with Pyr 0.15.");
        }
        if !self.m.house_here {
            return note(ui, "Not on this server yet", "Its Pyr is older than the board of the best at props (Pyr 0.15).");
        }
        ui.horizontal(|ui| {
            for (days, name) in [(7u32, "7 days"), (30, "30 days"), (0, "All time")] {
                let on = self.days == days;
                let label = RichText::new(name).font(bold_font(13.0)).color(if on { ON_EMBER } else { TEXT });
                if ui.add(egui::Button::new(label).fill(if on { EMBER2 } else { ELEV }).corner_radius(14)).clicked() {
                    self.days = days;
                }
            }
        });
        // Asked for when the page opens, when the span changes, and every minute it's open.
        if self.stats_asked.is_none_or(|(d, at)| d != self.days || at.elapsed() > Duration::from_secs(60)) {
            ask(asks::stats(self.days));
            self.stats_asked = Some((self.days, Instant::now()));
        }
        ui.add_space(8.0);
        let Some(stats) = self.m.stats.clone().filter(|s| s.days == self.days) else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Counting…").color(MUTED));
            });
            return;
        };
        if stats.leaders.is_empty() {
            return note(ui, "Nobody yet", "Nothing has come out in that time. Bet on something and be the first.");
        }
        if stats.leaders.len() >= 3 && ui.available_width() >= 520.0 {
            self.podium(ui, &stats.leaders[..3]);
            ui.add_space(10.0);
        }
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            let w = ui.available_width();
            let cols = [44.0, (w - 44.0 - 4.0 * 104.0).max(120.0), 104.0, 104.0, 104.0, 104.0];
            let row = |ui: &mut egui::Ui, cells: [(String, Color32, bool); 6], fill: Option<Color32>| {
                let (r, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
                if let Some(f) = fill {
                    ui.painter().rect_filled(r, CornerRadius::same(6), f);
                }
                let mut x = r.left();
                for (i, (text, col, strong)) in cells.into_iter().enumerate() {
                    let font = if strong { bold_font(13.5) } else { FontId::proportional(13.0) };
                    if i <= 1 {
                        ui.painter().text(pos2(x + 8.0, r.center().y), Align2::LEFT_CENTER, text, font, col);
                    } else {
                        ui.painter().text(pos2(x + cols[i] - 8.0, r.center().y), Align2::RIGHT_CENTER, text, font, col);
                    }
                    x += cols[i];
                }
            };
            let head = |t: &str| (t.to_string(), DIM, false);
            row(ui, [head("#"), head("WHO"), head("WON"), head("PUT IN"), head("PROPS"), head("BIGGEST")], None);
            for (i, l) in stats.leaders.iter().enumerate() {
                let me = l.uid == self.m.me;
                let medal = match i {
                    0 => GOLD,
                    1 => Color32::from_rgb(0xcf, 0xd6, 0xdc),
                    2 => Color32::from_rgb(0xd8, 0x9a, 0x6a),
                    _ => MUTED,
                };
                let net = if l.net > 0 { UP } else if l.net < 0 { DOWN } else { MUTED };
                row(
                    ui,
                    [((i + 1).to_string(), medal, true), (l.name.clone(), if me { GOLD } else { TEXT }, me || i < 3), (signed(l.net), net, true), (chips_text(l.staked), MUTED, false), (format!("{} ({} won)", l.props, l.wins), MUTED, false), (if l.biggest > 0 { signed(l.biggest) } else { "–".to_string() }, MUTED, false)],
                    if me { Some(Color32::from_rgb(0x33, 0x2a, 0x16)) } else if i % 2 == 1 { Some(Color32::from_white_alpha(5)) } else { None },
                );
            }
        });
        ui.add_space(6.0);
        let mine = match stats.rank {
            0 => "Nothing of yours has come out in that time.".to_string(),
            n => format!("You're number {n} of {}: {} chips.", stats.players, signed(stats.me.net)),
        };
        ui.label(RichText::new(mine).size(13.0).color(MUTED));
    }

    /// The best three on their steps: the winner in the middle, a step higher.
    fn podium(&self, ui: &mut egui::Ui, top: &[p::PropLeader]) {
        let gap = 10.0;
        let w = ((ui.available_width() - 2.0 * gap) / 3.0).min(280.0).floor();
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 136.0), Sense::hover());
        let pnt = ui.painter();
        let left = r.center().x - (3.0 * w + 2.0 * gap) / 2.0;
        for (slot, (place, h)) in [(1usize, 116.0f32), (0, 134.0), (2, 106.0)].into_iter().enumerate() {
            let Some(l) = top.get(place) else { continue };
            let medal = [GOLD, Color32::from_rgb(0xcf, 0xd6, 0xdc), Color32::from_rgb(0xd8, 0x9a, 0x6a)][place];
            let x = left + slot as f32 * (w + gap);
            let b = Rect::from_min_max(pos2(x, r.bottom() - h), pos2(x + w, r.bottom()));
            pnt.add(gradient_corner(b, 12.0, mix(CARD, medal, 0.24), CARD));
            pnt.rect_stroke(b, CornerRadius::same(12), Stroke::new(1.0_f32, mix(CARD_EDGE, medal, 0.5)), egui::StrokeKind::Inside);
            // Its place, on a medal.
            let m = pos2(b.left() + 30.0, b.top() + 30.0);
            pnt.circle_filled(m, 15.0, mix(medal, Color32::BLACK, 0.12));
            pnt.circle_stroke(m, 15.0, Stroke::new(1.5_f32, mix(medal, Color32::WHITE, 0.45)));
            pnt.text(m, Align2::CENTER_CENTER, (place + 1).to_string(), bold_font(15.0), ON_EMBER);
            let name = lines(pnt, &l.name, bold_font(15.5), if l.uid == self.m.me { GOLD } else { TEXT }, b.width() - 70.0, 1);
            pnt.galley(pos2(b.left() + 54.0, m.y - name.size().y / 2.0), name, TEXT);
            pnt.text(pos2(b.left() + 16.0, b.top() + 66.0), Align2::LEFT_CENTER, signed(l.net), bold_font(20.0), if l.net > 0 { UP } else if l.net < 0 { DOWN } else { MUTED });
            pnt.text(pos2(b.left() + 16.0, b.top() + 89.0), Align2::LEFT_CENTER, format!("won {} of {}", l.wins, count(l.props as u64, "prop", "props")), FontId::proportional(12.0), MUTED);
        }
    }

    // ------------------------------------------------------------ results

    pub fn results(&mut self, ui: &mut egui::Ui, now: i64) {
        let over: Vec<PropView> = self.m.over().into_iter().take(80).cloned().collect();
        ui.add_space(4.0);
        ui.label(RichText::new("Results").font(bold_font(23.0)).color(TEXT));
        ui.add_space(8.0);
        if over.is_empty() {
            return note(ui, "Nothing has come out yet", "Props show here once they're settled or called off.");
        }
        for prop in &over {
            let inner = card().inner_margin(egui::Margin::symmetric(14, 9)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                    emblem(ui.painter(), &prop.category_id(), r, &self.art);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.set_max_width(ui.available_width() - 150.0);
                        ui.add(egui::Label::new(RichText::new(&prop.question).font(bold_font(13.5)).color(TEXT)).truncate());
                        let (text, c) = standing(prop, now);
                        let note = if prop.note.is_empty() { String::new() } else { format!(" · {}", prop.note) };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(text).font(bold_font(12.0)).color(c));
                            ui.add(egui::Label::new(RichText::new(note).size(12.0).color(MUTED)).truncate());
                        });
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.with_layout(egui::Layout::top_down(egui::Align::Max), |ui| {
                                if prop.mine.is_empty() {
                                    ui.label(RichText::new(format!("{} in", chips_text(prop.pool))).size(12.5).color(MUTED));
                                } else {
                                    let (cost, worth) = stake_and_worth(prop);
                                    let net = worth as i64 - cost as i64;
                                    ui.label(RichText::new(format!("You: {}", signed(net))).font(bold_font(13.5)).color(if net > 0 { UP } else if net < 0 { DOWN } else { MUTED }));
                                }
                                if prop.settled_at > 0 {
                                    ui.label(RichText::new(words::ago(now - prop.settled_at)).size(11.5).color(DIM));
                                }
                            });
                        });
                    });
                });
            });
            let resp = ui.interact(inner.response.rect, ui.id().with(("over", prop.id)), Sense::click());
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.go(View::Prop(prop.id));
            }
            ui.add_space(4.0);
        }
    }

    // ------------------------------------------------------------ the windows

    /// The New prop bet window: the channel it goes in and its category, then as `/prop`'s
    /// in Pyr.
    pub fn new_prop_window(&mut self, ctx: &egui::Context) {
        let Some(mut np) = self.new_prop.clone() else { return };
        let mut open = true;
        let mut done = false;
        egui::Window::new(RichText::new("New prop bet").font(bold_font(17.0))).id(egui::Id::new("new-prop")).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).frame(egui::Frame::window(&ctx.style()).inner_margin(16)).open(&mut open).show(ctx, |ui| {
            let w = 440.0_f32.min(ctx.content_rect().width() - 56.0);
            ui.set_width(w);
            ui.add(egui::Label::new(RichText::new("Everyone who sees the channel bets Pyr Casino chips on it. The odds move with every bet; winners get a chip for every share. You back it and settle it (so you don't bet on it).").size(12.5).color(MUTED)).wrap());
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("CHANNEL").size(11.0).strong().color(DIM));
                    let name = np.channel.and_then(|c| self.m.channel_name(c)).map(|n| format!("#{n}")).unwrap_or_else(|| "Pick one".into());
                    egui::ComboBox::from_id_salt("new-prop-channel").selected_text(name).show_ui(ui, |ui| {
                        for (id, name) in &self.m.channels {
                            ui.selectable_value(&mut np.channel, Some(*id), format!("#{name}"));
                        }
                    });
                });
                if self.m.house_here {
                    ui.vertical(|ui| {
                        ui.label(RichText::new("CATEGORY").size(11.0).strong().color(DIM));
                        // (All of them at once: no hunting down a little list.)
                        egui::ComboBox::from_id_salt("new-prop-category").selected_text(p::category_name(&np.category)).height(520.0).show_ui(ui, |ui| {
                            for (id, name) in p::CATEGORIES {
                                ui.selectable_value(&mut np.category, id.to_string(), name);
                            }
                        });
                    });
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new("QUESTION").size(11.0).strong().color(DIM));
            ui.add(egui::TextEdit::singleline(&mut np.question).hint_text("Will it rain on Saturday?").desired_width(f32::INFINITY));
            ui.add_space(6.0);
            ui.label(RichText::new("OUTCOMES").size(11.0).strong().color(DIM));
            let mut remove = None;
            let hues: Vec<usize> = (0..np.outcomes.len()).map(|i| hue(&np.outcomes, i)).collect();
            for (i, o) in np.outcomes.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                    if let Some(t) = self.art.chips.get(hues[i]) {
                        ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    ui.add(egui::TextEdit::singleline(o).desired_width(w - 90.0).char_limit(30));
                    if i >= 2 && ui.small_button("×").on_hover_text("Take this outcome out").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                np.outcomes.remove(i);
            }
            if np.outcomes.len() < p::MAX_OUTCOMES && ui.small_button("+ Outcome").clicked() {
                np.outcomes.push(String::new());
            }
            ui.add_space(6.0);
            ui.label(RichText::new("HOW MUCH THE ODDS MOVE").size(11.0).strong().color(DIM));
            let n = np.outcomes.iter().filter(|o| !o.trim().is_empty()).count().max(2);
            ui.horizontal(|ui| {
                for (d, name) in p::DEPTHS {
                    ui.selectable_value(&mut np.depth, d, name).on_hover_text(format!("Deeper moves less with each bet, and takes more to back: {} chips here.", chips_text(p::backing(n, d as f64))));
                }
            });
            // What it takes from me, and why.
            let backing = p::backing(n, np.depth as f64);
            let short = self.m.chips.is_some_and(|h| h < backing);
            let have = match self.m.chips {
                Some(h) if short => format!(" You have {}.", chips_text(h)),
                _ => String::new(),
            };
            ui.add(egui::Label::new(RichText::new(format!("You put up {} chips to back its odds, so every winning share gets paid. What's left of them comes back to you when you settle it.{have}", chips_text(backing))).size(12.0).color(if short { LIVE } else { MUTED })).wrap());
            ui.add_space(6.0);
            ui.label(RichText::new("BETTING CLOSES").size(11.0).strong().color(DIM));
            ui.horizontal_wrapped(|ui| {
                for (secs, name) in [(0u64, "When I settle it"), (3600, "In an hour"), (86_400, "In a day"), (7 * 86_400, "In a week")] {
                    ui.selectable_value(&mut np.closes_in, secs, name);
                }
            });
            ui.add_space(10.0);
            let check = p::check_new(&np.question, &np.outcomes);
            if let Err(e) = &check {
                if !np.question.trim().is_empty() {
                    ui.label(RichText::new(e).size(12.0).color(LIVE));
                }
            }
            ui.horizontal(|ui| {
                let ok = check.is_ok() && !short && np.channel.is_some();
                if ui.add_enabled(ok, primary_button("Open the prop")).clicked() {
                    if let (Ok((q, outcomes)), Some(channel)) = (check, np.channel) {
                        ask(asks::open(channel, &q, &outcomes, np.depth, np.closes_in, &np.category));
                        done = true;
                    }
                }
                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
        });
        self.new_prop = if open && !done { Some(np) } else { None };
    }

    /// The casino's own props on this server: on or off, which categories, how deep (whoever
    /// manages the server).
    pub fn setting_window(&mut self, ctx: &egui::Context) {
        let Some(mut s) = self.setting.clone() else { return };
        let mut open = true;
        let mut done = false;
        egui::Window::new(RichText::new("The casino's own props").font(bold_font(17.0))).id(egui::Id::new("props-auto")).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).frame(egui::Frame::window(&ctx.style()).inner_margin(16)).open(&mut open).show(ctx, |ui| {
            let w = 460.0_f32.min(ctx.content_rect().width() - 56.0);
            ui.set_width(w);
            ui.add(egui::Label::new(RichText::new("Props that open, close and settle by themselves, from public sources: games, crypto, the weather, science and more (Pyr Casino's, from this server's own casino). The casino backs them, so winners are always paid; nobody on the server settles anything.").size(12.5).color(MUTED)).wrap());
            ui.add_space(8.0);
            ui.checkbox(&mut s.on, RichText::new("On, on this server").font(bold_font(14.0)));
            ui.add_space(6.0);
            ui.add_enabled_ui(s.on, |ui| {
                ui.label(RichText::new("CATEGORIES").size(11.0).strong().color(DIM));
                let across = if w >= 400.0 { 3 } else { 2 };
                egui::Grid::new("props-auto-categories").num_columns(across).spacing([22.0, 4.0]).show(ui, |ui| {
                    // (Members' own aren't the casino's. Pyr Casino's are each server's own,
                    // made from its casino's numbers.)
                    for (k, (id, name)) in p::CATEGORIES.iter().filter(|c| c.0 != p::COMMUNITY).enumerate() {
                        let mut on = !s.off.iter().any(|c| c == id);
                        if ui.checkbox(&mut on, *name).changed() {
                            s.off.retain(|c| c != id);
                            if !on {
                                s.off.push(id.to_string());
                            }
                        }
                        if k % across == across - 1 {
                            ui.end_row();
                        }
                    }
                });
                ui.add_space(6.0);
                ui.label(RichText::new("HOW MUCH A BET MOVES THEIR ODDS").size(11.0).strong().color(DIM));
                ui.horizontal(|ui| {
                    for (d, name) in p::DEPTHS {
                        ui.selectable_value(&mut s.depth, d, name).on_hover_text(format!("Deeper moves less with each bet. The most the casino can lose on an even prop this deep: {} chips.", chips_text(p::backing(2, d as f64))));
                    }
                });
                ui.add(egui::Label::new(RichText::new("Pick what suits how many chips people here have: Friendly for a new casino, High roller where millions change hands. It counts for props opened from now on.").size(12.0).color(MUTED)).wrap());
            });
            ui.add_space(8.0);
            if let Some(a) = &self.m.auto {
                let feed = if !a.feed_error.is_empty() {
                    (format!("The props feed can't be read just now: {}", a.feed_error), LIVE)
                } else if a.feed_at > 0 {
                    (format!("The props feed was made {}: {} props in it.", words::ago(words::now_ms() - a.feed_at), a.feed_props), MUTED)
                } else if a.on {
                    ("The props feed hasn't been read yet: it will be in a moment.".to_string(), MUTED)
                } else {
                    ("The props feed is read once they're on.".to_string(), MUTED)
                };
                ui.add(egui::Label::new(RichText::new(feed.0).size(12.0).color(feed.1)).wrap());
                ui.add_space(8.0);
            }
            ui.horizontal(|ui| {
                if ui.add(primary_button("Save")).clicked() {
                    ask(asks::auto(s.on, &s.off, s.depth));
                    done = true;
                }
                if ui.button("Cancel").clicked() {
                    done = true;
                }
            });
        });
        self.setting = if open && !done { Some(s) } else { None };
    }

    /// A win: gold falls across the window for a moment.
    pub fn celebrate(&mut self, ctx: &egui::Context) {
        let Some(at) = self.m.won_at else { return };
        let t = at.elapsed().as_secs_f32();
        if t > 2.6 {
            self.m.won_at = None;
            return;
        }
        ctx.request_repaint();
        let pnt = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("celebrate")));
        let screen = ctx.content_rect();
        let mut seed = 0x2545_f491u32;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32
        };
        for i in 0..90 {
            let (x0, vx, vy, size, spin) = (rnd(), rnd() - 0.5, 0.25 + rnd() * 0.5, 4.0 + rnd() * 5.0, rnd() * 6.0);
            let delay = rnd() * 0.5;
            let tt = (t - delay).max(0.0);
            let pos = pos2(screen.left() + screen.width() * (x0 + vx * 0.25 * tt), screen.top() - 20.0 + screen.height() * (vy * tt + 0.35 * tt * tt));
            let col = [GOLD, EMBER2, EMBER, COLORS[0], COLORS[2], COLORS[4]][i % 6].gamma_multiply((1.0 - t / 2.6).clamp(0.0, 1.0));
            let a = spin + tt * 5.0;
            let d = vec2(a.cos(), a.sin()) * size;
            pnt.add(Shape::convex_polygon(vec![pos + d, pos + vec2(-d.y, d.x) * 0.45, pos - d, pos - vec2(-d.y, d.x) * 0.45], col, Stroke::NONE));
        }
    }
}

/// How a prop's odds went: a line an outcome, from when it opened to now, drawn as high as
/// they got. Under the pointer: what they were then.
fn chart(ui: &mut egui::Ui, prop: &PropView, now: i64) {
    if prop.history.len() < 2 {
        return;
    }
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 176.0), Sense::hover());
    let pnt = ui.painter().with_clip_rect(r.expand(2.0));
    pnt.rect_filled(r, CornerRadius::same(10), Color32::from_black_alpha(70));
    let plot = Rect::from_min_max(r.min + vec2(42.0, 14.0), r.max - vec2(50.0, 26.0));
    let live = Model::live(prop);
    let first = prop.history[0].at;
    // (Up to now while it's to come out; to when it was settled once it is.)
    let last = if live { now } else { prop.settled_at.max(prop.history.last().map_or(first, |h| h.at)) }.max(first + 1);
    // Every point there is, and where they stand (or stood when it came out).
    let mut points: Vec<(i64, Vec<f32>)> = prop.history.iter().map(|h| (h.at, h.p.clone())).collect();
    if live {
        points.push((last, prop.prices().iter().map(|x| *x as f32).collect()));
    } else if let Some(end) = points.last().map(|p| p.1.clone()) {
        // (Over: they stood as the last bet left them until it came out.)
        points.push((last, end));
    }
    let n = prop.outcomes.len();
    // As high as they got, to the next line up.
    let most = points.iter().flat_map(|p| p.1.iter().copied()).fold(0.0f32, f32::max);
    let (top, step) = match most {
        m if m <= 0.24 => (0.25f32, 0.05f32),
        m if m <= 0.48 => (0.5, 0.1),
        m if m <= 0.72 => (0.75, 0.25),
        _ => (1.0, 0.25),
    };
    let x_of = |t: i64| plot.left() + plot.width() * ((t - first) as f32 / (last - first) as f32).clamp(0.0, 1.0);
    let y_of = |v: f32| plot.bottom() - plot.height() * (v / top).clamp(0.0, 1.0);
    for k in 0..=((top / step).round() as i32) {
        let v = k as f32 * step;
        let y = y_of(v);
        pnt.line_segment([pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.0_f32, Color32::from_white_alpha(if k == 0 { 30 } else { 12 })));
        pnt.text(pos2(plot.left() - 7.0, y), Align2::RIGHT_CENTER, format!("{}%", (v * 100.0).round() as i32), FontId::proportional(10.0), DIM);
    }
    // When it opened, and now.
    pnt.text(pos2(plot.left(), plot.bottom() + 13.0), Align2::LEFT_CENTER, format!("opened {}", words::ago(now - first)), FontId::proportional(10.0), DIM);
    pnt.text(pos2(plot.right(), plot.bottom() + 13.0), Align2::RIGHT_CENTER, if live { "now".to_string() } else { words::ago(now - last) }, FontId::proportional(10.0), DIM);
    // Where each stands; the likeliest is drawn last, over a wash of its color.
    let ends: Vec<f32> = (0..n).map(|i| points.last().and_then(|p| p.1.get(i).copied()).unwrap_or(0.0)).collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|a, b| ends[*a].total_cmp(&ends[*b]));
    let lead = order.last().copied();
    for &i in &order {
        let col = color_of(&prop.outcomes, i);
        // Odds hold between bets: steps, not slopes.
        let mut pts: Vec<Pos2> = Vec::with_capacity(points.len() * 2);
        let mut before: Option<f32> = None;
        for (t, p) in &points {
            let Some(y) = p.get(i).copied() else { continue };
            if let Some(b) = before {
                if Some(i) == lead {
                    let from = pts.last().map_or(plot.left(), |q| q.x);
                    pnt.rect_filled(Rect::from_min_max(pos2(from, y_of(b)), pos2(x_of(*t), plot.bottom())), CornerRadius::ZERO, col.gamma_multiply(0.10));
                }
                pts.push(pos2(x_of(*t), y_of(b)));
            }
            pts.push(pos2(x_of(*t), y_of(y)));
            before = Some(y);
        }
        if pts.len() >= 2 {
            pnt.add(Shape::line(pts, Stroke::new(if Some(i) == lead { 2.4_f32 } else { 1.8_f32 }, col)));
        }
    }
    // What the likeliest six stand at, down the right: pushed apart where they'd sit on each
    // other.
    let mut labels: Vec<(f32, usize)> = order.iter().rev().take(6).map(|i| (y_of(ends[*i]), *i)).collect();
    labels.sort_by(|a, b| a.0.total_cmp(&b.0));
    for k in 1..labels.len() {
        labels[k].0 = labels[k].0.max(labels[k - 1].0 + 12.0);
    }
    let mut floor = plot.bottom() + 4.0;
    for l in labels.iter_mut().rev() {
        l.0 = l.0.min(floor);
        floor = l.0 - 12.0;
    }
    for (y, i) in &labels {
        let col = color_of(&prop.outcomes, *i);
        pnt.circle_filled(pos2(plot.right(), y_of(ends[*i])), 3.0, col);
        pnt.text(pos2(plot.right() + 8.0, *y), Align2::LEFT_CENTER, pct(ends[*i] as f64), bold_font(11.0), col);
    }
    // Under the pointer: a line down that moment, and what each was then.
    let Some(at) = resp.hover_pos().filter(|q| q.x >= plot.left() - 6.0 && q.x <= plot.right() + 6.0) else { return };
    let f = ((at.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
    let t = first + ((last - first) as f64 * f as f64) as i64;
    let x = plot.left() + plot.width() * f;
    pnt.line_segment([pos2(x, plot.top()), pos2(x, plot.bottom())], Stroke::new(1.0_f32, Color32::from_white_alpha(70)));
    let then = points.iter().take_while(|p| p.0 <= t).last().or(points.first());
    let mut rows: Vec<(usize, f32)> = (0..n).filter_map(|i| then.and_then(|p| p.1.get(i).copied()).map(|v| (i, v))).collect();
    rows.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (i, v) in &rows {
        pnt.circle_filled(pos2(x, y_of(*v)), 3.5, color_of(&prop.outcomes, *i));
    }
    rows.truncate(6);
    let head = pnt.layout_no_wrap(local_when(t, now), FontId::proportional(11.0), MUTED);
    let set: Vec<_> = rows.iter().map(|(i, v)| (color_of(&prop.outcomes, *i), lines(&pnt, &prop.outcomes[*i], FontId::proportional(12.0), TEXT, 170.0, 1), pnt.layout_no_wrap(pct(*v as f64), bold_font(12.0), color_of(&prop.outcomes, *i)))).collect();
    let w = set.iter().map(|l| l.1.size().x + l.2.size().x + 34.0).fold(head.size().x, f32::max) + 22.0;
    let h = 10.0 + head.size().y + 5.0 + set.len() as f32 * 17.0 + 7.0;
    let left = if x + 14.0 + w > r.right() - 4.0 { x - 14.0 - w } else { x + 14.0 };
    let b = Rect::from_min_size(pos2(left, (at.y - h / 2.0).min(r.bottom() - h - 4.0).max(r.top() + 4.0)), vec2(w, h));
    pnt.rect_filled(b, CornerRadius::same(8), SIDE);
    pnt.rect_stroke(b, CornerRadius::same(8), Stroke::new(1.0_f32, LINE), egui::StrokeKind::Inside);
    let mut y = b.top() + 10.0;
    pnt.galley(pos2(b.left() + 11.0, y), head.clone(), MUTED);
    y += head.size().y + 5.0;
    for (col, name, odds) in set {
        pnt.circle_filled(pos2(b.left() + 15.0, y + 8.0), 3.5, col);
        pnt.galley(pos2(b.left() + 25.0, y + 8.0 - name.size().y / 2.0), name, TEXT);
        pnt.galley(pos2(b.right() - 11.0 - odds.size().x, y + 8.0 - odds.size().y / 2.0), odds.clone(), col);
        y += 17.0;
    }
}
