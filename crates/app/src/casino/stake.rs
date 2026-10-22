//! What every slot has from 0.13.0, as Stake's slots do: **Play Feature**, a menu of ways to
//! play beyond a spin (a spin with better odds of the bonus, a wild reel, the bonus bought
//! outright, a bigger one, a mystery one, an all-or-nothing shot), each priced from the game's
//! exact math so it pays back what the game does; and **Fairness**, each slot's own seed, which
//! I can change, the house's seed for the next round (as its fingerprint), the round's number,
//! and any round played again from the three.
//!
//! The menu and the panel are drawn over the slot's page; each slot says which it is
//! (`SlotKind`), and plays what's picked itself.

use eframe::egui::{self, Align2, Color32, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::state::ServerView;
use pyr_poker::casino::feature::Feature;
use pyr_poker::casino::{self, Bet, Game, Outcome, dragon, reelrow, slots};
use pyr_poker::net::RoundView;

use super::{col, glow_text, pics, vgrad_round};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

/// Which slot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotKind {
    /// One of Reel Row's machines.
    Row(u8),
    Sevens,
    Dragon,
}

impl SlotKind {
    /// Its statistics key (and the seeds' key).
    pub fn key(self) -> String {
        match self {
            SlotKind::Row(m) => casino::reels_key(m),
            SlotKind::Sevens => Game::Slots.id().to_string(),
            SlotKind::Dragon => Game::Dragon.id().to_string(),
        }
    }

    /// A feature's price, in hundredths of the spin's bet (none: the slot hasn't it).
    pub fn price(self, f: Feature) -> Option<u64> {
        match self {
            SlotKind::Row(_) => Some(reelrow::price(f)),
            SlotKind::Sevens => Some(slots::price(f)),
            SlotKind::Dragon => dragon::price(f),
        }
    }

    /// Its Play Feature badges (a 3 x 2 sheet: Chance, Wild, Bonus; Super, Mystery, All or
    /// nothing).
    pub fn badges(self) -> String {
        match self {
            SlotKind::Row(m) => format!("rr-{}-features", super::reel_themes::theme(m).id),
            SlotKind::Sevens => "sevens-features".into(),
            SlotKind::Dragon => "dragon-features".into(),
        }
    }

    /// What its bonus is called, and what its Super is.
    pub fn bonus_name(self) -> (String, String) {
        match self {
            SlotKind::Row(m) => {
                let g = reelrow::GAMES[(m as usize).min(11)].0;
                (g.to_string(), format!("Super {g}"))
            }
            SlotKind::Sevens => ("Free Spins".into(), "Super Free Spins".into()),
            SlotKind::Dragon => ("Ember Respins".into(), "Super Ember Respins".into()),
        }
    }

    /// Each feature's name and what it does, in this slot's words.
    pub fn describe(self, f: Feature) -> (String, String) {
        let (bonus, sup) = self.bonus_name();
        match (self, f) {
            (_, Feature::Spin) => ("Spin".into(), "A spin, as ever.".into()),
            (SlotKind::Row(_), Feature::Chance) => ("Bonus Chance".into(), format!("Every spin four times as likely to start the {bonus}.")),
            (SlotKind::Sevens, Feature::Chance) => ("Bonus Chance".into(), "Every spin five times as likely to land the stars for Free Spins.".into()),
            (SlotKind::Dragon, Feature::Chance) => ("Bonus Chance".into(), "Every spin four times as likely to light six fireballs.".into()),
            (SlotKind::Row(_), Feature::Wild) => ("Wild Spin".into(), "The middle reel lands all wild, every spin.".into()),
            (_, Feature::Wild) => ("Wild Spin".into(), "The middle reel lands all wild flames, every spin.".into()),
            (_, Feature::Bonus) => (format!("Buy {bonus}"), format!("Straight into the {bonus}.")),
            (SlotKind::Row(_), Feature::Super) => (format!("Buy {sup}"), "Every prize in it tripled.".into()),
            (SlotKind::Sevens, Feature::Super) => (format!("Buy {sup}"), "Free spins at 10× instead of 3×.".into()),
            (SlotKind::Dragon, Feature::Super) => (format!("Buy {sup}"), "Every prize and jackpot tripled.".into()),
            (_, Feature::Mystery) => ("Mystery Bonus".into(), format!("The {bonus}, and 3 times in 10 the {sup}.")),
            (SlotKind::Sevens, Feature::AllIn) => ("All or Nothing".into(), "1 in 51: 2,500× the bet. Or nothing.".into()),
            (SlotKind::Dragon, Feature::AllIn) => ("All or Nothing".into(), "1 in 51: 2,500× the bet. Or nothing.".into()),
            (_, Feature::AllIn) => ("All or Nothing".into(), "1 in 50.5: 2,500× the bet. Or nothing.".into()),
        }
    }

    /// What it pays back, said (the slot's way).
    pub fn rtp(self) -> &'static str {
        match self {
            SlotKind::Row(_) => "RTP 99%",
            SlotKind::Sevens => "RTP 98% + jackpot",
            SlotKind::Dragon => "RTP 98.5% + pools",
        }
    }
}

/// Where a feature's badge is on its sheet.
pub fn badge_index(f: Feature) -> usize {
    match f {
        Feature::Chance => 0,
        Feature::Wild => 1,
        Feature::Bonus | Feature::Spin => 2,
        Feature::Super => 3,
        Feature::Mystery => 4,
        Feature::AllIn => 5,
    }
}

/// A badge drawn in `r`.
pub fn badge(p: &egui::Painter, ctx: &egui::Context, kind: SlotKind, f: Feature, r: Rect, tint: Color32) {
    let i = badge_index(f);
    let (cx, cy) = ((i % 3) as f32, (i / 3) as f32);
    let uv = Rect::from_min_max(pos2(cx / 3.0, cy / 2.0), pos2((cx + 1.0) / 3.0, (cy + 1.0) / 2.0));
    pics::part(p, ctx, &kind.badges(), uv, r, (r.width() * 3.0) as u32 + 1, tint);
}

/// The Play Feature menu and Fairness panel's state on a slot's page.
#[derive(Default)]
pub struct StakeUi {
    /// The menu's open; a buy waits for its Yes.
    pub menu: bool,
    pub confirm: Option<Feature>,
    /// The feature every spin plays with (Chance or Wild), until it's turned off.
    pub armed: Option<Feature>,
    /// Fairness: open (on which tab), the seed being typed, and the round being played again.
    pub fairness: Option<FairTab>,
    pub seed_edit: String,
    pub editing: bool,
    pub verify: VerifyForm,
    /// The seed each round of mine was played from (round's number -> seed), for checking.
    pub seeds_used: Vec<(u64, String, String)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FairTab {
    #[default]
    Seeds,
    Verify,
}

#[derive(Default, Clone)]
pub struct VerifyForm {
    pub server: String,
    pub client: String,
    pub nonce: String,
    /// The round it's for (its bet, as played).
    pub round: Option<RoundView>,
    pub result: Option<Result<String, String>>,
}

/// What came of a click in the menu.
pub enum Pick {
    /// Play this one now (a buy, said yes to).
    Buy(Feature),
    /// Spins play with this from now on (or with nothing).
    Arm(Option<Feature>),
}

impl PyrApp {
    /// The Play Feature menu over `area`: a card for each of the slot's features, its badge,
    /// what it does, what it costs at this bet and what it pays back. Buys ask first.
    pub(super) fn play_feature_menu(&mut self, ui: &mut egui::Ui, kind: SlotKind, area: Rect, stake: u64, spin_cost: u64, accent: Color32, chips: u64) -> Option<Pick> {
        let st = self.stake_ui(kind);
        if !st.menu {
            return None;
        }
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        let mut out = None;
        // A dark veil over the slot; a click outside closes it.
        p.rect_filled(area, 0, Color32::from_black_alpha(205));
        let w = (area.width() - 60.0).min(940.0);
        let h = (area.height() - 50.0).min(600.0);
        let panel = Rect::from_center_size(area.center(), vec2(w, h));
        let veil = ui.interact(area, ui.id().with("pf-veil"), Sense::click());
        if veil.clicked() && !veil.interact_pointer_pos().is_some_and(|q| panel.contains(q)) {
            let st = self.stake_ui_mut(kind);
            st.menu = false;
            st.confirm = None;
            return None;
        }
        vgrad_round(&p, panel, 22.0, super::mix(accent, Color32::BLACK, 0.82), Color32::from_rgb(8, 6, 10));
        p.rect_stroke(panel, theme::r(22), Stroke::new(2.0f32, accent.gamma_multiply(0.8)), StrokeKind::Inside);
        glow_text(&p, pos2(panel.center().x, panel.top() + 34.0), Align2::CENTER_CENTER, "PLAY FEATURE", theme::bold_font(26.0), Color32::WHITE, accent.gamma_multiply(0.6));
        p.text(pos2(panel.center().x, panel.top() + 62.0), Align2::CENTER_CENTER, format!("At a bet of {}: each priced to pay back what the game does ({})", chips_text(spin_cost), kind.rtp()), theme::font(13.0), col::MUTED);
        // Close.
        let x = Rect::from_center_size(pos2(panel.right() - 28.0, panel.top() + 28.0), vec2(30.0, 30.0));
        let xr = ui.interact(x, ui.id().with("pf-close"), Sense::click());
        p.circle_filled(x.center(), 15.0, if xr.hovered() { Color32::from_white_alpha(40) } else { Color32::from_white_alpha(16) });
        let d = 5.5;
        let xs = Stroke::new(2.2f32, Color32::WHITE);
        p.line_segment([x.center() + vec2(-d, -d), x.center() + vec2(d, d)], xs);
        p.line_segment([x.center() + vec2(-d, d), x.center() + vec2(d, -d)], xs);
        if xr.clicked() {
            let st = self.stake_ui_mut(kind);
            st.menu = false;
            st.confirm = None;
            return None;
        }
        let feats: Vec<Feature> = [Feature::Chance, Feature::Wild, Feature::Bonus, Feature::Super, Feature::Mystery, Feature::AllIn].into_iter().filter(|f| kind.price(*f).is_some()).collect();
        let cols = 3usize;
        let rows = feats.len().div_ceil(cols);
        let grid = Rect::from_min_max(pos2(panel.left() + 24.0, panel.top() + 86.0), pos2(panel.right() - 24.0, panel.bottom() - 22.0));
        let gap = 14.0;
        let cw = (grid.width() - gap * (cols as f32 - 1.0)) / cols as f32;
        let ch = ((grid.height() - gap * (rows as f32 - 1.0)) / rows as f32).min(250.0);
        let armed = self.stake_ui(kind).armed;
        let confirm = self.stake_ui(kind).confirm;
        let time = ctx.input(|i| i.time) as f32;
        for (i, f) in feats.iter().enumerate() {
            let r = Rect::from_min_size(pos2(grid.left() + (i % cols) as f32 * (cw + gap), grid.top() + (i / cols) as f32 * (ch + gap)), vec2(cw, ch));
            let price_h = kind.price(*f).unwrap_or(100);
            let cost = casino::feature::cost(spin_cost, price_h);
            // While a buy is being asked about, the cards beneath stay still.
            let resp = ui.interact(r, ui.id().with(("pf", i)), if confirm.is_some() { Sense::hover() } else { Sense::click() });
            let on = armed == Some(*f);
            let hover = confirm.is_none() && resp.hovered();
            let fill_top = if on { super::mix(accent, Color32::BLACK, 0.55) } else if hover { Color32::from_rgb(0x2a, 0x22, 0x30) } else { Color32::from_rgb(0x18, 0x14, 0x1c) };
            vgrad_round(&p, r, 16.0, fill_top, Color32::from_rgb(0x0c, 0x0a, 0x0e));
            p.rect_stroke(r, theme::r(16), Stroke::new(if on { 2.5f32 } else { 1.0 }, if on { accent } else if hover { accent.gamma_multiply(0.7) } else { Color32::from_white_alpha(30) }), StrokeKind::Inside);
            // The badge, bobbing a little when it's hovered.
            let bob = if hover { (time * 4.0).sin() * 3.0 } else { 0.0 };
            let bs = (ch * 0.44).min(cw * 0.5);
            let br = Rect::from_center_size(pos2(r.center().x, r.top() + 14.0 + bs / 2.0 + bob), vec2(bs, bs));
            if hover || on {
                super::radial(&p, br.center(), bs * 0.75, accent.gamma_multiply(0.35), Color32::TRANSPARENT);
            }
            badge(&p, &ctx, kind, *f, br, Color32::WHITE);
            let (name, what) = kind.describe(*f);
            let ty = br.bottom() + 8.0;
            p.text(pos2(r.center().x, ty + 8.0), Align2::CENTER_CENTER, &name, theme::bold_font(15.5), Color32::WHITE);
            let g = p.layout(what, theme::font(12.0), col::MUTED, cw - 24.0);
            let gh = g.size().y;
            p.galley(pos2(r.center().x - g.size().x / 2.0, ty + 20.0), g, col::MUTED);
            // Its price: chips, and times the bet.
            let price_y = (ty + 26.0 + gh + 12.0).min(r.bottom() - 18.0);
            let x = if price_h % 100 == 0 { format!("{}×", price_h / 100) } else { format!("{:.2}×", price_h as f64 / 100.0) };
            let tag = format!("{}  ·  {x}", chips_text(cost));
            let gg = p.layout_no_wrap(tag, theme::bold_font(14.0), col::GOLD2);
            let pill = Rect::from_center_size(pos2(r.center().x, price_y), gg.size() + vec2(22.0, 10.0));
            p.rect_filled(pill, theme::r(13), if cost > chips { Color32::from_rgb(0x4a, 0x10, 0x10) } else { Color32::from_black_alpha(160) });
            p.galley(pill.center() - gg.size() / 2.0, gg, if cost > chips { col::LOSE } else { col::GOLD2 });
            if on {
                p.text(pos2(r.right() - 12.0, r.top() + 14.0), Align2::RIGHT_CENTER, "ON", theme::bold_font(12.0), accent);
            }
            if confirm.is_none() && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                if f.spins() {
                    out = Some(Pick::Arm(if on { None } else { Some(*f) }));
                    self.stake_ui_mut(kind).menu = false;
                } else {
                    self.stake_ui_mut(kind).confirm = Some(*f);
                }
            }
        }
        // A buy, asked about first.
        if let Some(f) = confirm {
            let cost = casino::feature::cost(spin_cost, kind.price(f).unwrap_or(100));
            let (name, what) = kind.describe(f);
            let d = Rect::from_center_size(panel.center(), vec2(420.0, 250.0));
            p.rect_filled(panel, theme::r(22), Color32::from_black_alpha(150));
            vgrad_round(&p, d, 18.0, Color32::from_rgb(0x22, 0x1a, 0x28), Color32::from_rgb(0x0e, 0x0a, 0x10));
            p.rect_stroke(d, theme::r(18), Stroke::new(2.0f32, accent), StrokeKind::Inside);
            badge(&p, &ctx, kind, f, Rect::from_center_size(pos2(d.center().x, d.top() + 56.0), vec2(80.0, 80.0)), Color32::WHITE);
            p.text(pos2(d.center().x, d.top() + 112.0), Align2::CENTER_CENTER, &name, theme::bold_font(19.0), Color32::WHITE);
            p.text(pos2(d.center().x, d.top() + 136.0), Align2::CENTER_CENTER, what, theme::font(12.5), col::MUTED);
            let ok = cost <= chips;
            let yes = Rect::from_min_size(pos2(d.center().x + 6.0, d.bottom() - 62.0), vec2(180.0, 46.0));
            let no = Rect::from_min_size(pos2(d.center().x - 186.0, d.bottom() - 62.0), vec2(180.0, 46.0));
            let yr = ui.interact(yes, ui.id().with("pf-yes"), Sense::click());
            let nr = ui.interact(no, ui.id().with("pf-no"), Sense::click());
            p.rect_filled(yes, theme::r(23), if !ok { Color32::from_white_alpha(20) } else if yr.hovered() { col::GOLD2 } else { col::GOLD });
            p.text(yes.center(), Align2::CENTER_CENTER, format!("BUY  {}", chips_text(cost)), theme::bold_font(16.0), if ok { Color32::from_rgb(0x2a, 0x10, 0x04) } else { Color32::from_white_alpha(80) });
            p.rect_filled(no, theme::r(23), if nr.hovered() { Color32::from_white_alpha(40) } else { Color32::from_white_alpha(20) });
            p.text(no.center(), Align2::CENTER_CENTER, "Not now", theme::bold_font(15.0), Color32::WHITE);
            if !ok {
                p.text(pos2(d.center().x, d.bottom() - 8.0), Align2::CENTER_BOTTOM, "Not enough chips for this one at this bet", theme::font(11.5), col::LOSE);
            }
            if yr.clicked() && ok {
                out = Some(Pick::Buy(f));
                let st = self.stake_ui_mut(kind);
                st.confirm = None;
                st.menu = false;
            }
            if nr.clicked() {
                self.stake_ui_mut(kind).confirm = None;
            }
        }
        let _ = stake;
        out
    }

    /// The Play Feature button on a slot's controls: the armed feature's badge, or the bonus
    /// buy's; true when it's clicked.
    pub(super) fn play_feature_button(&mut self, ui: &mut egui::Ui, kind: SlotKind, r: Rect, accent: Color32, enabled: bool) -> bool {
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        let resp = ui.interact(r, ui.id().with(("pf-button", kind.key())), Sense::click());
        let armed = self.stake_ui(kind).armed;
        let time = ctx.input(|i| i.time) as f32;
        let pulse = 0.5 + 0.5 * (time * 2.5).sin();
        let fill = if armed.is_some() { super::mix(accent, Color32::BLACK, 0.45) } else if resp.hovered() && enabled { Color32::from_rgb(0x3a, 0x2a, 0x40) } else { Color32::from_rgb(0x24, 0x1a, 0x2a) };
        vgrad_round(&p, r, r.height() / 2.0, fill, super::mix(fill, Color32::BLACK, 0.5));
        p.rect_stroke(r, theme::r((r.height() / 2.0) as u8), Stroke::new(1.5f32, accent.gamma_multiply(0.5 + 0.4 * pulse)), StrokeKind::Inside);
        let bs = r.height() * 1.25;
        let br = Rect::from_center_size(pos2(r.left() + bs * 0.42, r.center().y - 2.0), vec2(bs, bs));
        badge(&p, &ctx, kind, armed.unwrap_or(Feature::Bonus), br, if enabled { Color32::WHITE } else { Color32::from_gray(120) });
        let (top, sub) = match armed {
            Some(f) => (kind.describe(f).0.to_uppercase(), "on · click to change".to_string()),
            None => ("PLAY FEATURE".to_string(), "buy the bonus & more".to_string()),
        };
        let tx = br.right() - bs * 0.05;
        p.text(pos2(tx, r.center().y - 7.0), Align2::LEFT_CENTER, top, theme::bold_font(13.0), if enabled { Color32::WHITE } else { Color32::from_white_alpha(90) });
        p.text(pos2(tx, r.center().y + 9.0), Align2::LEFT_CENTER, sub, theme::font(10.5), col::MUTED);
        let clicked = resp.on_hover_text("Play Feature: buy the bonus, or spin with better odds").on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
        if clicked && enabled {
            let st = self.stake_ui_mut(kind);
            st.menu = !st.menu;
            st.confirm = None;
        }
        clicked
    }

    /// The Fairness button (a shield with a tick).
    pub(super) fn fairness_button(&mut self, ui: &mut egui::Ui, kind: SlotKind, r: Rect) {
        let p = ui.painter().clone();
        let resp = ui.interact(r, ui.id().with(("fair-button", kind.key())), Sense::click());
        let on = self.stake_ui(kind).fairness.is_some();
        p.rect_filled(r, theme::r((r.height() / 2.0) as u8), if on { col::GOLD } else if resp.hovered() { Color32::from_white_alpha(40) } else { Color32::from_white_alpha(18) });
        let c = pos2(r.left() + r.height() * 0.55, r.center().y);
        let s = r.height() * 0.3;
        let ink = if on { Color32::BLACK } else { col::WIN };
        p.add(egui::Shape::convex_polygon(vec![c + vec2(-s, -s * 0.9), c + vec2(0.0, -s * 1.2), c + vec2(s, -s * 0.9), c + vec2(s * 0.85, s * 0.3), c + vec2(0.0, s * 1.2), c + vec2(-s * 0.85, s * 0.3)], Color32::TRANSPARENT, Stroke::new(1.6f32, ink)));
        p.line_segment([c + vec2(-s * 0.45, 0.0), c + vec2(-s * 0.1, s * 0.4)], Stroke::new(1.8f32, ink));
        p.line_segment([c + vec2(-s * 0.1, s * 0.4), c + vec2(s * 0.5, -s * 0.4)], Stroke::new(1.8f32, ink));
        p.text(pos2(c.x + s * 1.6, r.center().y), Align2::LEFT_CENTER, "Fairness", theme::bold_font(12.0), if on { Color32::BLACK } else { col::TEXT });
        if resp.on_hover_text("Your seed for this slot, the house's, and every round played again").clicked() {
            let st = self.stake_ui_mut(kind);
            st.fairness = if st.fairness.is_some() { None } else { Some(FairTab::Seeds) };
            st.editing = false;
        }
    }

    /// The Fairness panel: my seed for this slot (change it, or roll a new one), the house's
    /// next seed's fingerprint and the round's number; the last rounds here, each checked; and
    /// any round played again from the house's seed, my seed and its number.
    pub(super) fn fairness_panel(&mut self, ctx: &egui::Context, s: &ServerView, kind: SlotKind) {
        let Some(tab) = self.stake_ui(kind).fairness else { return };
        let key = kind.key();
        let mine = self.engine.casino_seed(&s.key, &key);
        let c = &s.poker.casino;
        let hex = |b: &[u8; 32]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let mut open = true;
        let mut tab_now = tab;
        let rounds: Vec<RoundView> = c.rounds.iter().rev().filter(|r| r.outcome.stats_key() == key).take(12).cloned().collect();
        let used = self.stake_ui(kind).seeds_used.clone();
        egui::Window::new("Fairness").collapsible(false).resizable(false).default_width(560.0).open(&mut open).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut tab_now, FairTab::Seeds, "Seeds");
                ui.selectable_value(&mut tab_now, FairTab::Verify, "Verify");
            });
            ui.separator();
            match tab_now {
                FairTab::Seeds => {
                    ui.label(RichText::new("Every round here comes from three things: the house's seed, your seed and the round's number. The house shows its seed's fingerprint before the round and the seed itself after it, so it can't change it; your seed is yours, so it can't know the round before you play it.").size(13.0));
                    ui.add_space(8.0);
                    ui.label(RichText::new("YOUR SEED (THIS SLOT)").size(11.5).strong().color(theme::MUTED()));
                    let editing = self.stake_ui(kind).editing;
                    ui.horizontal(|ui| {
                        if editing {
                            let st = self.stake_ui_mut(kind);
                            ui.add(egui::TextEdit::singleline(&mut st.seed_edit).desired_width(300.0).char_limit(64).hint_text("anything you like"));
                            if ui.button("Save").clicked() {
                                let typed = self.stake_ui(kind).seed_edit.clone();
                                self.engine.set_casino_seed(&s.key, &key, &typed);
                                self.stake_ui_mut(kind).editing = false;
                            }
                            if ui.button("Cancel").clicked() {
                                self.stake_ui_mut(kind).editing = false;
                            }
                        } else {
                            ui.label(RichText::new(&mine).monospace().size(15.0).color(col::GOLD2));
                            if ui.button("Change").clicked() {
                                let st = self.stake_ui_mut(kind);
                                st.seed_edit = mine.clone();
                                st.editing = true;
                            }
                            if ui.button("New random seed").clicked() {
                                self.engine.set_casino_seed(&s.key, &key, "");
                            }
                        }
                    });
                    ui.add_space(8.0);
                    ui.label(RichText::new("THE HOUSE'S NEXT SEED (ITS FINGERPRINT, SHA-256)").size(11.5).strong().color(theme::MUTED()));
                    ui.label(RichText::new(c.commit.map(|x| hex(&x)).unwrap_or_else(|| "…".into())).monospace().size(11.5));
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("NEXT ROUND'S NUMBER (NONCE): {}", c.nonce)).size(11.5).strong().color(theme::MUTED()));
                    ui.add_space(10.0);
                    ui.label(RichText::new(format!("This Pyr has played {} round{} here again and found every one exactly as the house said.", chips_text(c.checked), if c.checked == 1 { "" } else { "s" })).color(col::WIN));
                    for pr in &c.problems {
                        ui.label(RichText::new(format!("⚠ {pr}")).color(col::LOSE));
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new("The seed your Pyr sends is SHA-256 of \"pyr client seed v1\", your seed, the slot and the round's number; the round's seed is SHA-256 of \"pyr-casino round v1\", the house's seed, that, the game and the number.").size(11.0).color(theme::DIM()));
                }
                FairTab::Verify => {
                    ui.label(RichText::new("Your last rounds here. Pick one to play it again from its seeds; or type any seeds and number.").size(13.0));
                    ui.add_space(4.0);
                    egui::ScrollArea::vertical().max_height(170.0).show(ui, |ui| {
                        for r in &rounds {
                            let seed_used = used.iter().find(|(n, k, _)| *n == r.nonce && *k == key).map(|(_, _, t)| t.clone());
                            let ok = seed_used.as_ref().map(|t| casino::client_seed(t, &key, r.nonce) == r.client_seed);
                            let mark = match ok {
                                Some(true) => "✓ your seed",
                                Some(false) => "⚠ not your seed",
                                None => "✓",
                            };
                            let text = format!("#{}  ·  {}  ·  paid {}  ·  {mark}", r.nonce, r.outcome.headline(), chips_text(r.outcome.paid()));
                            if ui.selectable_label(false, text).clicked() {
                                let st = self.stake_ui_mut(kind);
                                st.verify = VerifyForm { server: hex(&r.server_seed), client: seed_used.unwrap_or_default(), nonce: r.nonce.to_string(), round: Some(r.clone()), result: None };
                            }
                        }
                        if rounds.is_empty() {
                            ui.label(RichText::new("No rounds here yet.").color(theme::DIM()));
                        }
                    });
                    ui.separator();
                    let st = self.stake_ui_mut(kind);
                    egui::Grid::new("fair-verify").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        ui.label("House seed");
                        ui.add(egui::TextEdit::singleline(&mut st.verify.server).desired_width(420.0).font(egui::TextStyle::Monospace));
                        ui.end_row();
                        ui.label("Your seed");
                        ui.add(egui::TextEdit::singleline(&mut st.verify.client).desired_width(420.0));
                        ui.end_row();
                        ui.label("Round's number");
                        ui.add(egui::TextEdit::singleline(&mut st.verify.nonce).desired_width(120.0));
                        ui.end_row();
                    });
                    if ui.add_enabled(st.verify.round.is_some(), egui::Button::new("Play it again")).clicked() {
                        st.verify.result = Some(replay(&st.verify, &key));
                    }
                    match &st.verify.result {
                        Some(Ok(t)) => {
                            ui.label(RichText::new(t).color(col::WIN));
                        }
                        Some(Err(e)) => {
                            ui.label(RichText::new(e).color(col::LOSE));
                        }
                        None => {}
                    }
                }
            }
        });
        let st = self.stake_ui_mut(kind);
        if !open {
            st.fairness = None;
        } else {
            st.fairness = Some(tab_now);
        }
    }

    /// A slot round played from my seed for it (and the round noted, to check by).
    pub(super) fn play_seeded(&mut self, s: &ServerView, kind: SlotKind, bet: Bet) {
        let Some(commit) = s.poker.casino.commit else { return };
        let nonce = s.poker.casino.nonce;
        let key = kind.key();
        let seed = self.engine.casino_seed(&s.key, &key);
        let st = self.stake_ui_mut(kind);
        st.seeds_used.push((nonce, key.clone(), seed));
        if st.seeds_used.len() > 200 {
            st.seeds_used.remove(0);
        }
        self.engine.casino_play_seeded(&s.key, commit, nonce, &key, bet);
    }

    pub(super) fn stake_ui(&self, kind: SlotKind) -> &StakeUi {
        match kind {
            SlotKind::Row(_) => &self.poker.casino.row.stake,
            SlotKind::Sevens => &self.poker.casino.slots.stake,
            SlotKind::Dragon => &self.poker.casino.dragon.stake,
        }
    }

    pub(super) fn stake_ui_mut(&mut self, kind: SlotKind) -> &mut StakeUi {
        match kind {
            SlotKind::Row(_) => &mut self.poker.casino.row.stake,
            SlotKind::Sevens => &mut self.poker.casino.slots.stake,
            SlotKind::Dragon => &mut self.poker.casino.dragon.stake,
        }
    }
}

/// A round played again from the seeds typed: the house's (hex), mine (words) and the number,
/// with the round's own bet; whether it comes out as it did.
fn replay(v: &VerifyForm, key: &str) -> Result<String, String> {
    let r = v.round.as_ref().ok_or("pick a round first")?;
    let server = pyr_poker::net::hex32::parse(v.server.trim()).ok_or("the house's seed is 64 hex digits")?;
    let nonce: u64 = v.nonce.trim().parse().map_err(|_| "the round's number is a whole number")?;
    let client = casino::client_seed(&v.client, key, nonce);
    let mut st = casino::stream(&server, &client, r.outcome.game(), nonce);
    let again = match &r.outcome {
        Outcome::Row(o) => Outcome::Row(reelrow::play(o.machine, o.bet, o.feature, &mut st)),
        Outcome::Slots(o) => Outcome::Slots(slots::play_feature(o.line_bet, o.feature, &mut st, o.jackpot_pool)),
        Outcome::Dragon(o) => Outcome::Dragon(dragon::play_feature(o.bet, o.growth, o.pools, o.feature, &mut st)),
        Outcome::Reels(o) => Outcome::Reels(casino::reels::play(o.machine, o.bet, o.bought, &mut st)),
        _ => return Err("not a slot round".into()),
    };
    let fp = casino::commit(&server);
    let same_seed = fp == r.commit;
    if again == r.outcome {
        Ok(format!("✓ Played again: {} paying {}, exactly as the house said. The house's seed matches the fingerprint it showed first{}.", again.headline(), chips_text(again.paid()), if same_seed { "" } else { " (no: see above)" }))
    } else if !same_seed {
        Err("✗ That isn't the house's seed for this round: its fingerprint doesn't match.".into())
    } else {
        Err(format!("✗ From these seeds the round comes out differently: {} paying {}.", again.headline(), chips_text(again.paid())))
    }
}

/// All or nothing, revealed: a dial of sixty slices, two of them lit (the one-in-fifty), spun
/// faster and faster then slowing to the answer; once it's landed, what it came to. `t`: seconds
/// since it started. True once it's landed.
#[allow(clippy::too_many_arguments)]
pub fn all_in_dial(p: &egui::Painter, r: Rect, accent: Color32, font: &str, lucky: bool, paid: u64, odds: &str, t: f32) -> bool {
    use super::ease_out;
    p.rect_filled(r, theme::r(20), Color32::from_black_alpha(232));
    p.rect_stroke(r, theme::r(20), Stroke::new(2.0f32, accent), StrokeKind::Inside);
    let c = r.center() + vec2(0.0, 10.0);
    let rad = (r.height() * 0.32).min(r.width() * 0.25);
    glow_text(p, pos2(r.center().x, r.top() + 34.0), Align2::CENTER_CENTER, "ALL OR NOTHING", theme::display_font(font, 32.0), col::GOLD2, accent.gamma_multiply(0.6));
    p.text(pos2(r.center().x, r.top() + 64.0), Align2::CENTER_CENTER, odds, theme::font(14.0), col::MUTED);
    let spin_t = 3.2;
    let k = (t / spin_t).min(1.0);
    let turns = 9.0 * std::f32::consts::TAU;
    // (The pointer's at the top: the lit slices end there for a win, opposite for nothing.)
    let rest = -std::f32::consts::FRAC_PI_2 - std::f32::consts::TAU / 60.0 + if lucky { 0.0 } else { std::f32::consts::PI };
    let ang = rest + turns * (1.0 - ease_out(k));
    p.circle_filled(c, rad + 10.0, Color32::from_rgb(0x30, 0x30, 0x38));
    p.circle_stroke(c, rad + 10.0, Stroke::new(4.0f32, col::GOLD));
    for i in 0..60 {
        let a0 = ang + i as f32 / 60.0 * std::f32::consts::TAU;
        let a1 = a0 + std::f32::consts::TAU / 60.0;
        let lit = i < 2;
        let fill = if lit { accent } else if i % 2 == 0 { Color32::from_rgb(0x1a, 0x1a, 0x22) } else { Color32::from_rgb(0x24, 0x24, 0x2e) };
        p.add(egui::Shape::convex_polygon(vec![c, c + vec2(a0.cos(), a0.sin()) * rad, c + vec2(a1.cos(), a1.sin()) * rad], fill, Stroke::NONE));
    }
    p.circle_filled(c, rad * 0.25, col::GOLD);
    let tip = pos2(c.x, c.y - rad - 14.0);
    p.add(egui::Shape::convex_polygon(vec![tip + vec2(-12.0, -18.0), tip + vec2(12.0, -18.0), tip + vec2(0.0, 8.0)], col::GOLD2, Stroke::new(2.0f32, Color32::BLACK)));
    if k >= 1.0 {
        let (text, color) = if lucky { (format!("WON {}", chips_text(paid)), col::WIN) } else { ("NOTHING".to_string(), col::LOSE) };
        glow_text(p, c + vec2(0.0, rad + 34.0), Align2::CENTER_CENTER, &text, theme::display_font(font, 34.0), color, Color32::from_black_alpha(200));
    }
    k >= 1.0
}

