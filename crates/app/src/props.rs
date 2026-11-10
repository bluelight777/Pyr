//! Prop bets in chat (the rebuild's; the server keeps them all: see `pyr_poker::props`, after
//! Maanix's PlayProps). A prop's message shows as its card: the question on a lit betting
//! board, each outcome on its jeweled chip with its odds and Bet (how much, and what it pays),
//! what I hold with Cash out, the latest bets, and Settle and Call off for whoever opened it
//! (and moderators). `/prop` (or New prop bet) opens one: a question, two to six outcomes, how
//! deep its market is and when betting closes.

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, RichText, Sense, Stroke, pos2, vec2};
use pyr_engine::state::{AppState, ServerView};
use pyr_poker::props::{self as p, PropState, PropView};

use crate::app::{Act, PyrApp};
use crate::poker::cards::chips_text;
use crate::theme;

/// The outcomes' chips (Meshy's art), in order, and their colors.
pub const CHIPS: [&str; 6] = ["props-chip-emerald", "props-chip-ruby", "props-chip-sapphire", "props-chip-amber", "props-chip-amethyst", "props-chip-pearl"];
pub const COLORS: [Color32; 6] = [
    Color32::from_rgb(0x2e, 0xc2, 0x6e),
    Color32::from_rgb(0xe5, 0x48, 0x4d),
    Color32::from_rgb(0x4a, 0x7d, 0xf0),
    Color32::from_rgb(0xf0, 0xa0, 0x30),
    Color32::from_rgb(0xa8, 0x5c, 0xf0),
    Color32::from_rgb(0xe8, 0xe2, 0xd6),
];

/// The props a message's text links to (`pyr://prop/<id>`).
pub fn ids_in(text: &str) -> Vec<u64> {
    let mut out = Vec::new();
    for w in text.split_whitespace() {
        if let Some(id) = p::id_in_link(w) {
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out
}

/// A prop's message as it's sent: a line for Pyrs that don't show props, and its link.
pub fn message_for(prop: &PropView) -> String {
    format!("📊 Prop bet: {}\n{}", prop.question, p::link(prop.id))
}

/// A message's text with its props' lines left out (the card says it all).
pub fn without_links(text: &str) -> String {
    text.lines().filter(|l| p::id_in_link(l.trim()).is_none() && !l.starts_with("📊 Prop bet:")).collect::<Vec<_>>().join("\n").trim().to_string()
}

/// "62%"
fn pct(x: f64) -> String {
    format!("{}%", (x * 100.0).round() as i64)
}

/// "in 2 h", "in 5 min".
fn time_left(ms: i64) -> String {
    let m = (ms / 60_000).max(1);
    if m < 60 {
        format!("in {m} min")
    } else if m < 48 * 60 {
        format!("in {} h", m / 60)
    } else {
        format!("in {} days", m / 1440)
    }
}

/// A bet being picked on a card: the prop, the outcome, and how much (as typed).
#[derive(Clone, Debug, Default)]
pub struct Betting {
    pub key: String,
    pub id: u64,
    pub outcome: usize,
    pub amount: String,
}

/// The New prop bet window's fields.
#[derive(Clone, Debug)]
pub struct NewProp {
    pub key: String,
    pub channel: i64,
    pub question: String,
    pub outcomes: Vec<String>,
    pub depth: u64,
    pub closes_in: u64,
}

impl NewProp {
    pub fn new(key: String, channel: i64, question: String, outcomes: Vec<String>) -> NewProp {
        let outcomes = if outcomes.len() >= 2 { outcomes } else { vec!["Yes".into(), "No".into()] };
        NewProp { key, channel, question, outcomes, depth: p::DEPTHS[0].0, closes_in: 0 }
    }
}

impl PyrApp {
    /// A prop's card in a message (asked for from the server the first time it shows).
    pub(crate) fn prop_card(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, id: u64, acts: &mut Vec<Act>) {
        let Some(prop) = s.poker.props.get(&id).cloned() else {
            acts.push(Act::PropGet { key: s.key.clone(), ids: vec![id] });
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Loading the prop bet…").color(theme::MUTED()));
            });
            return;
        };
        let now = pyr_proto::now_ms();
        let w = 440.0_f32.min(ui.available_width());
        let ctx = ui.ctx().clone();
        egui::Frame::new().fill(Color32::from_rgb(0x14, 0x0f, 0x0d)).stroke(Stroke::new(1.0_f32, Color32::from_rgb(0x5a, 0x40, 0x22))).corner_radius(12).show(ui, |ui| {
            ui.set_width(w);
            // The board: its art behind the question.
            let (top, _) = ui.allocate_exact_size(vec2(w, 96.0), Sense::hover());
            let p = ui.painter();
            if !crate::casino::pics::cover_rounded(p, &ctx, "props-banner", top, 12.0, (0.42, 0.38), 640, Color32::from_gray(150)) {
                p.rect_filled(top, CornerRadius::same(12), Color32::from_rgb(0x2a, 0x18, 0x10));
            }
            p.rect_filled(Rect::from_min_max(top.left_top() + vec2(14.0, 12.0), top.right_bottom() - vec2(14.0, 10.0)), CornerRadius::same(8), Color32::from_black_alpha(150));
            let emblem = Rect::from_min_size(top.left_top() + vec2(22.0, 20.0), vec2(30.0, 30.0));
            crate::casino::pics::cover(p, &ctx, "props-emblem", emblem, 64, Color32::WHITE);
            let state = match prop.state {
                PropState::Open if prop.open_at(now) => match prop.closes_at {
                    0 => "OPEN".to_string(),
                    c => format!("OPEN · CLOSES {}", time_left(c - now).to_uppercase()),
                },
                PropState::Open | PropState::Closed => "BETTING CLOSED".into(),
                PropState::Settled => "SETTLED".into(),
                PropState::Void => "CALLED OFF".into(),
            };
            p.text(emblem.right_center() + vec2(8.0, -7.0), Align2::LEFT_CENTER, format!("PROP BET #{}", prop.id), theme::bold_font(11.5), Color32::from_rgb(0xff, 0xd3, 0x7a));
            p.text(emblem.right_center() + vec2(8.0, 8.0), Align2::LEFT_CENTER, state, theme::bold_font(10.5), Color32::from_rgb(0xd8, 0xc8, 0xb0));
            let q = ui.fonts_mut(|f| f.layout(prop.question.clone(), theme::bold_font(16.0), Color32::WHITE, w - 56.0));
            let qh = q.size().y;
            ui.painter().galley(pos2(top.left() + 26.0, top.bottom() - 14.0 - qh), q, Color32::WHITE);
            // The rest, on the card's velvet (its place kept before what's drawn over it).
            let body_top = ui.cursor().top();
            let slot = ui.painter().add(egui::Shape::Noop);
            // (Kept inside the velvet's gilded corners.)
            let inner = egui::Frame::new().inner_margin(egui::Margin { left: 30, right: 30, top: 12, bottom: 30 }).show(ui, |ui| {
                ui.set_width(w - 60.0);
                self.prop_outcomes(ui, s, &st.my_uid, &prop, now, acts);
                self.prop_mine(ui, s, &prop, now, acts);
                prop_footer(ui, &prop);
                self.prop_admin(ui, s, &prop, acts);
            });
            let body = Rect::from_min_max(pos2(top.left() + 1.0, body_top), pos2(top.right() - 1.0, inner.response.rect.bottom() - 1.0));
            if let Some(bg) = crate::casino::pics::cover_rounded_shape(&ctx, "props-card-bg", body, 11.0, (0.5, 0.75), 640, Color32::from_gray(200)) {
                ui.painter().set(slot, bg);
            }
        });
    }

    /// Each outcome: its chip, name and odds, a bar, and Bet (or, settled, who won).
    fn prop_outcomes(&mut self, ui: &mut egui::Ui, s: &ServerView, my_uid: &str, prop: &PropView, now: i64, acts: &mut Vec<Act>) {
        let ctx = ui.ctx().clone();
        let prices = prop.prices();
        let open = prop.open_at(now);
        // (Whoever opened it settles it, so doesn't bet on it.)
        let mine = prop.author == my_uid;
        if prop.state == PropState::Settled {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(54.0, 54.0), Sense::hover());
                crate::casino::pics::cover(ui.painter(), &ctx, "props-won", r, 128, Color32::WHITE);
                ui.vertical(|ui| {
                    let won = prop.winner.and_then(|w| prop.outcomes.get(w)).cloned().unwrap_or_default();
                    ui.label(RichText::new(format!("Settled: {won}")).font(theme::bold_font(17.0)).color(Color32::from_rgb(0xff, 0xd3, 0x7a)));
                    let who = if prop.winners.is_empty() {
                        "Nobody had chips on it.".to_string()
                    } else {
                        prop.winners.iter().map(|(n, c)| format!("{n} +{}", chips_text(*c))).collect::<Vec<_>>().join(" · ")
                    };
                    ui.label(RichText::new(who).size(12.5).color(theme::MUTED()));
                });
            });
            ui.add_space(6.0);
        } else if prop.state == PropState::Void {
            ui.label(RichText::new("Called off: everyone's chips went back to them.").italics().color(theme::MUTED()));
            ui.add_space(6.0);
        } else if !open {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
                crate::casino::pics::cover(ui.painter(), &ctx, "props-lock", r, 64, Color32::WHITE);
                ui.label(RichText::new("Betting has closed: it pays out when it's settled.").color(theme::MUTED()));
            });
            ui.add_space(4.0);
        }
        for (i, name) in prop.outcomes.iter().enumerate() {
            let price = prices.get(i).copied().unwrap_or(0.0);
            let won = prop.winner == Some(i);
            let faded = prop.state == PropState::Settled && !won;
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
            let pnt = ui.painter();
            let chip = Rect::from_center_size(pos2(row.left() + 17.0, row.center().y), vec2(32.0, 32.0));
            crate::casino::pics::cover(pnt, &ctx, CHIPS[i % 6], chip, 64, if faded { Color32::from_gray(90) } else { Color32::WHITE });
            let col = COLORS[i % 6];
            let x = row.left() + 42.0;
            let bar_w = row.width() - 42.0 - 132.0;
            pnt.text(pos2(x, row.top() + 12.0), Align2::LEFT_CENTER, name, theme::bold_font(14.0), if faded { theme::DIM() } else { theme::TEXT() });
            pnt.text(pos2(x + bar_w, row.top() + 12.0), Align2::RIGHT_CENTER, pct(price), theme::bold_font(14.0), if faded { theme::DIM() } else { col });
            let bar = Rect::from_min_size(pos2(x, row.top() + 24.0), vec2(bar_w, 6.0));
            pnt.rect_filled(bar, CornerRadius::same(3), Color32::from_white_alpha(18));
            pnt.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * price as f32, bar.height())), CornerRadius::same(3), if faded { Color32::from_gray(70) } else { col });
            if open && !mine {
                let b = Rect::from_min_size(pos2(row.right() - 76.0, row.center().y - 15.0), vec2(76.0, 30.0));
                let picked = self.ui.prop_betting.as_ref().is_some_and(|bt| bt.key == s.key && bt.id == prop.id && bt.outcome == i);
                let button = egui::Button::new(RichText::new("Bet").font(theme::bold_font(13.0)).color(Color32::from_rgb(0x1b, 0x14, 0x11))).fill(col).corner_radius(15);
                let button = if picked { button.stroke(Stroke::new(2.0_f32, Color32::WHITE)) } else { button };
                let resp = ui.put(b, button);
                if resp.on_hover_text(format!("Bet chips on {name}: each share pays a chip if it wins")).clicked() {
                    self.ui.prop_betting = if picked { None } else { Some(Betting { key: s.key.clone(), id: prop.id, outcome: i, amount: self.ui.prop_betting.as_ref().map(|b| b.amount.clone()).unwrap_or_else(|| "1000".into()) }) };
                    // (How many chips I have, for All in and the checks: the casino's record.)
                    if s.poker.me.is_none() {
                        self.engine.poker(&s.key, pyr_poker::net::ToServer::Lobby);
                    }
                }
            }
            if self.ui.prop_betting.as_ref().is_some_and(|bt| bt.key == s.key && bt.id == prop.id && bt.outcome == i) {
                self.prop_bet_panel(ui, s, prop, i, acts);
            }
        }
    }

    /// Picking how much to bet on an outcome: chips buttons and a box, what it pays, and Place.
    fn prop_bet_panel(&mut self, ui: &mut egui::Ui, s: &ServerView, prop: &PropView, outcome: usize, acts: &mut Vec<Act>) {
        let Some(mut bt) = self.ui.prop_betting.clone() else { return };
        let have = s.poker.me.as_ref().map(|m| m.chips);
        egui::Frame::new().fill(Color32::from_black_alpha(90)).corner_radius(8).inner_margin(10).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                for (label, n) in [("100", 100u64), ("1K", 1_000), ("10K", 10_000), ("100K", 100_000), ("1M", 1_000_000)] {
                    if ui.small_button(label).clicked() {
                        bt.amount = n.to_string();
                    }
                }
                if let Some(h) = have {
                    if ui.small_button("All in").on_hover_text(format!("All {} of your chips", chips_text(h))).clicked() {
                        bt.amount = h.min(p::MAX_BET).to_string();
                    }
                }
                ui.add(egui::TextEdit::singleline(&mut bt.amount).desired_width(110.0).hint_text("chips"));
            });
            let chips: u64 = bt.amount.replace([',', ' '], "").parse().unwrap_or(0);
            let shares = if chips > 0 { p::shares_for(&prop.q, prop.b, outcome, chips as f64) } else { 0.0 };
            let name = prop.outcomes.get(outcome).cloned().unwrap_or_default();
            let line = if chips < p::MIN_BET {
                format!("At least {} chips.", p::MIN_BET)
            } else if have.is_some_and(|h| chips > h) {
                format!("You have {} chips.", chips_text(have.unwrap_or(0)))
            } else {
                let profit = shares.floor() as i64 - chips as i64;
                format!("Pays {} chips if {name} wins ({}{}).", chips_text(shares.floor() as u64), if profit >= 0 { "+" } else { "-" }, chips_text(profit.unsigned_abs()))
            };
            ui.label(RichText::new(line).size(12.5).color(theme::MUTED()));
            ui.horizontal(|ui| {
                let ok = chips >= p::MIN_BET && have.is_none_or(|h| chips <= h);
                if ui.add_enabled(ok, theme::primary_button("Place bet")).clicked() {
                    // (What it was shown to pay, less a little: if the odds moved more than
                    // that, the server says so instead.)
                    acts.push(Act::PropBet { key: s.key.clone(), id: prop.id, outcome, chips, min_shares: shares * 0.98 });
                    self.ui.prop_betting = None;
                    return;
                }
                if ui.button("Cancel").clicked() {
                    self.ui.prop_betting = None;
                    return;
                }
                self.ui.prop_betting = Some(bt.clone());
            });
        });
    }

    /// What I hold, worth now, with Cash out.
    fn prop_mine(&mut self, ui: &mut egui::Ui, s: &ServerView, prop: &PropView, now: i64, acts: &mut Vec<Act>) {
        if prop.mine.is_empty() || matches!(prop.state, PropState::Settled | PropState::Void) {
            return;
        }
        ui.add_space(4.0);
        for h in &prop.mine {
            let name = prop.outcomes.get(h.outcome).cloned().unwrap_or_default();
            let worth = prop.worth(h).max(0.0).floor() as u64;
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("You: {name}, pays {} · paid {} · worth {} now", chips_text(h.shares.floor() as u64), chips_text(h.cost), chips_text(worth))).size(12.5).color(theme::TEXT()));
                if prop.open_at(now) && ui.small_button("Cash out").on_hover_text(format!("Sell your {name} back for {} chips", chips_text(worth))).clicked() {
                    acts.push(Act::PropCashOut { key: s.key.clone(), id: prop.id, outcome: h.outcome });
                }
            });
        }
    }

    /// Settle (pick what happened) and Call off, for whoever opened it and moderators.
    fn prop_admin(&mut self, ui: &mut egui::Ui, s: &ServerView, prop: &PropView, acts: &mut Vec<Act>) {
        if !prop.can_settle || matches!(prop.state, PropState::Settled | PropState::Void) {
            return;
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.menu_button(RichText::new("Settle…").font(theme::bold_font(13.0)), |ui| {
                crate::menu::style(ui);
                ui.label(RichText::new("What happened? Winners are paid a chip a share.").size(12.0).color(theme::DIM()));
                for (i, o) in prop.outcomes.iter().enumerate() {
                    if ui.button(o).clicked() {
                        acts.push(Act::PropSettle { key: s.key.clone(), id: prop.id, outcome: i });
                        ui.close();
                    }
                }
            });
            if ui.small_button("Call off").on_hover_text("Everyone gets back what they put in (once others have bet, only a moderator can)").clicked() {
                acts.push(Act::PropVoid { key: s.key.clone(), id: prop.id });
            }
        });
    }

    /// The New prop bet window (from `/prop`).
    pub(crate) fn new_prop_window(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) {
        let Some(mut np) = self.ui.new_prop.clone() else { return };
        if st.server(&np.key).is_none() {
            self.ui.new_prop = None;
            return;
        }
        let mut open = true;
        let mut done = false;
        egui::Window::new(RichText::new("New prop bet").font(theme::bold_font(17.0)))
            .id(egui::Id::new("new-prop"))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .min_width(460.0)
            .max_width(460.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
                    crate::casino::pics::cover(ui.painter(), ctx, "props-emblem", r, 96, Color32::WHITE);
                    ui.add(
                        egui::Label::new(
                            RichText::new("Everyone here bets Pyr Casino chips on it. The odds move with every bet; winners get a chip for every share. You back it and settle it (so you don't bet on it).")
                                .size(12.5)
                                .color(theme::MUTED()),
                        )
                        .wrap(),
                    );
                });
                ui.add_space(8.0);
                ui.label(RichText::new("QUESTION").size(11.0).strong().color(theme::DIM()));
                ui.add(egui::TextEdit::singleline(&mut np.question).hint_text("Will it rain on Saturday?").desired_width(f32::INFINITY));
                ui.add_space(6.0);
                ui.label(RichText::new("OUTCOMES").size(11.0).strong().color(theme::DIM()));
                let mut remove = None;
                for (i, o) in np.outcomes.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                        crate::casino::pics::cover(ui.painter(), ctx, CHIPS[i % 6], r, 48, Color32::WHITE);
                        ui.add(egui::TextEdit::singleline(o).desired_width(300.0).char_limit(30));
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
                ui.label(RichText::new("HOW MUCH THE ODDS MOVE").size(11.0).strong().color(theme::DIM()));
                let n = np.outcomes.iter().filter(|o| !o.trim().is_empty()).count().max(2);
                ui.horizontal(|ui| {
                    for (d, name) in p::DEPTHS {
                        ui.selectable_value(&mut np.depth, d, name)
                            .on_hover_text(format!("Deeper moves less with each bet, and takes more to back: {} chips here.", chips_text(p::backing(n, d as f64))));
                    }
                });
                // What it takes from me, and why.
                let backing = p::backing(n, np.depth as f64);
                let have = st.server(&np.key).and_then(|s| s.poker.me.as_ref().map(|m| m.chips));
                let short = have.is_some_and(|h| h < backing);
                ui.add(egui::Label::new(
                    RichText::new(format!(
                        "You put up {} chips to back its odds, so every winning share gets paid. What's left of them comes back to you when you settle it.{}",
                        chips_text(backing),
                        match have {
                            Some(h) if short => format!(" You have {}.", chips_text(h)),
                            _ => String::new(),
                        }
                    ))
                    .size(12.0)
                    .color(if short { theme::LIVE() } else { theme::MUTED() }),
                ).wrap());
                ui.add_space(6.0);
                ui.label(RichText::new("BETTING CLOSES").size(11.0).strong().color(theme::DIM()));
                ui.horizontal(|ui| {
                    for (secs, name) in [(0u64, "When I settle it"), (3600, "In an hour"), (86_400, "In a day"), (7 * 86_400, "In a week")] {
                        ui.selectable_value(&mut np.closes_in, secs, name);
                    }
                });
                ui.add_space(10.0);
                let check = p::check_new(&np.question, &np.outcomes);
                if let Err(e) = &check {
                    if !np.question.trim().is_empty() {
                        ui.label(RichText::new(e).size(12.0).color(theme::LIVE()));
                    }
                }
                ui.horizontal(|ui| {
                    if ui.add_enabled(check.is_ok() && !short, theme::primary_button("Open the prop")).clicked() {
                        if let Ok((q, outcomes)) = check {
                            acts.push(Act::PropOpen { key: np.key.clone(), channel: np.channel, question: q, outcomes, depth: np.depth, closes_in: np.closes_in });
                            done = true;
                        }
                    }
                    if ui.button("Cancel").clicked() {
                        done = true;
                    }
                });
            });
        self.ui.new_prop = if open && !done { Some(np) } else { None };
    }
}

/// How much is in, how many bets, who opened it, and the latest bets.
fn prop_footer(ui: &mut egui::Ui, prop: &PropView) {
    ui.add_space(6.0);
    ui.label(
        RichText::new(format!("{} chips in · {} · opened and backed by {} ({} chips)", chips_text(prop.pool), crate::poker::cards::count(prop.bets as u64, "bet", "bets"), prop.author_name, chips_text(prop.backing)))
            .size(12.0)
            .color(theme::DIM()),
    );
    if !prop.latest.is_empty() {
        let lines: Vec<String> = prop
            .latest
            .iter()
            .take(4)
            .map(|b| {
                let o = prop.outcomes.get(b.outcome).cloned().unwrap_or_default();
                if b.chips >= 0 { format!("{} {} on {o}", b.name, chips_text(b.chips as u64)) } else { format!("{} cashed out {o}", b.name) }
            })
            .collect();
        ui.label(RichText::new(lines.join(" · ")).size(11.5).color(theme::DIM()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_props_message_links_to_it_and_shows_as_its_card() {
        let prop = PropView { id: 12, question: "Will it rain on Saturday?".into(), ..Default::default() };
        let text = message_for(&prop);
        assert_eq!(ids_in(&text), vec![12]);
        assert_eq!(without_links(&text), "");
        assert_eq!(without_links("look at this\npyr://prop/3"), "look at this");
        assert!(ids_in("nothing here").is_empty());
    }

    #[test]
    fn times_left_read_naturally() {
        assert_eq!(time_left(30_000), "in 1 min");
        assert_eq!(time_left(2 * 3_600_000 + 5_000), "in 2 h");
        assert_eq!(time_left(3 * 86_400_000), "in 3 days");
    }
}
