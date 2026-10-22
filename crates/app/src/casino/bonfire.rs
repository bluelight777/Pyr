//! The Bonfire at the app: Pyr Casino's giant prize pool, given away by raffle (see
//! `pyr_poker::casino::bonfire`). Its card heads the casino floor: Meshy's bonfire burning, embers
//! rising, the pool, the clock to when it's lit, my embers and my chance. When it's lit, everyone
//! in Pyr sees the show: the fire roars, ten Sparks fly out of it, three Blazes land on a name
//! reel, then the Grand Ember, slowest of all, and the fireworks.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::casino::bonfire::{self as bf, Tier};
use pyr_poker::net::{BonfireDraw, BonfireView, ToServer};

use super::{col, ease_back, ease_out, glow_text, hgrad, pics, radial, vgrad};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

/// How often the Bonfire's asked for while the casino's open.
const ASK_S: u64 = 6;
/// The show's parts (seconds from its start): the fire lit, the Sparks, the Blazes, the Grand
/// Ember, and the end (when a click closes it; it closes itself a while after).
const SPARKS_AT: f32 = 2.6;
const SPARK_GAP: f32 = 0.5;
const BLAZES_AT: f32 = 8.4;
const BLAZE_S: f32 = 3.0;
const GRAND_AT: f32 = 17.6;
const GRAND_SPIN: f32 = 6.0;
const END_AT: f32 = GRAND_AT + GRAND_SPIN + 1.0;
const CLOSE_AT: f32 = END_AT + 22.0;
/// A draw older than this when it comes (the app was away): not shown.
const STALE_MS: u64 = 90_000;

const EMBER: Color32 = Color32::from_rgb(0xff, 0x7a, 0x1a);
const FLAME: Color32 = Color32::from_rgb(0xff, 0xb0, 0x3a);

fn tier_color(t: Tier) -> Color32 {
    match t {
        Tier::Grand => col::GOLD2,
        Tier::Blaze => Color32::from_rgb(0xff, 0x8a, 0x3a),
        Tier::Spark => Color32::from_rgb(0xff, 0xd0, 0x8a),
    }
}

#[derive(Default)]
pub struct BonfireUi {
    asked: Option<Instant>,
    /// The fire being shown lit (its number) and when the show began; the last one closed.
    show: Option<(u64, Instant)>,
    closed: Option<u64>,
    /// The show's sounds already played (a bit each).
    sounded: u64,
    /// My embers as shown, and the last time they grew (by how many).
    shown_embers: Option<(u64, u64)>,
    grew: Option<(u64, Instant)>,
    /// The Bonfire's own window: how it works, the last fire, the most embers.
    pub panel: bool,
}

/// h(n, salt) in 0..1, the same every time.
fn h01(n: u32, salt: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9e37_79b9) ^ salt.wrapping_mul(0x85eb_ca6b);
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b_3c6d);
    x ^= x >> 12;
    (x % 10_000) as f32 / 10_000.0
}

/// Embers rising out of a fire at `base`, over `area`.
fn embers(p: &egui::Painter, area: Rect, base: Pos2, spread: f32, n: u32, t: f32, strength: f32) {
    for i in 0..n {
        let speed = 0.09 + h01(i, 1) * 0.16;
        let k = (t * speed + h01(i, 2)).fract();
        let sway = (t * (0.6 + h01(i, 3)) + i as f32).sin() * spread * 0.18 * k;
        let x = base.x + (h01(i, 4) - 0.5) * spread * (0.4 + k) + sway;
        let y = base.y - k * area.height() * (0.7 + h01(i, 5) * 0.5);
        if !area.contains(pos2(x, y)) {
            continue;
        }
        let a = (1.0 - k) * (k * 10.0).min(1.0) * strength;
        let r = 0.9 + h01(i, 6) * 2.2;
        let c = if h01(i, 7) > 0.7 { col::GOLD2 } else { FLAME };
        p.circle_filled(pos2(x, y), r * 2.4, c.gamma_multiply(0.12 * a));
        p.circle_filled(pos2(x, y), r, c.gamma_multiply(a));
    }
}

/// `n` as a time: "13:42:07", or "2d 4h" further off.
fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 48 * 3600 {
        return format!("{}d {}h", s / 86_400, (s % 86_400) / 3600);
    }
    format!("{:02}:{:02}:{:02}", s / 3600, (s / 60) % 60, s % 60)
}

impl PyrApp {
    /// Keeps the Bonfire's state coming while the casino's open.
    pub fn bonfire_keep(&mut self, s: &ServerView) {
        if !s.has(pyr_proto::control::CASINO_012_FEATURE) {
            return;
        }
        if self.poker.casino.bonfire.asked.is_none_or(|t| t.elapsed() > Duration::from_secs(ASK_S)) {
            self.poker.casino.bonfire.asked = Some(Instant::now());
            self.engine.poker(&s.key, ToServer::Bonfire);
        }
        // My embers growing: "+n" floats up from them.
        if let Some((v, _)) = &s.poker.bonfire {
            let ui = &mut self.poker.casino.bonfire;
            match ui.shown_embers {
                Some((round, e)) if round == v.round && v.my_embers > e => ui.grew = Some((v.my_embers - e, Instant::now())),
                _ => {}
            }
            ui.shown_embers = Some((v.round, v.my_embers));
        }
    }

    /// The Bonfire's card, heading the casino floor.
    pub(super) fn bonfire_hero(&mut self, ui: &mut egui::Ui, s: &ServerView, w: f32) {
        let Some((v, at)) = s.poker.bonfire.clone() else { return };
        let ctx = ui.ctx().clone();
        let t = ctx.input(|i| i.time) as f32;
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            let h = (w * 0.27).clamp(210.0, 270.0);
            let (r, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
            let p = ui.painter().clone();
            let base = Color32::from_rgb(0x08, 0x05, 0x06);
            p.rect_filled(r.translate(vec2(0.0, 10.0)), theme::r(22), Color32::from_black_alpha(130));
            p.rect_filled(r, theme::r(20), base);
            // The fire on the right; its edge melting into the dark on the left.
            let art = Rect::from_min_max(pos2(r.left() + r.width() * 0.32, r.top()), r.max);
            pics::cover_rounded(&p, &ctx, "bonfire", art, 20.0, (0.5, 0.55), 1600, Color32::WHITE);
            hgrad(&p, Rect::from_min_max(art.min, pos2(art.left() + art.width() * 0.4, art.bottom())), base, base.gamma_multiply(0.0));
            let fire = pos2(art.left() + art.width() * 0.5, art.top() + art.height() * 0.62);
            let flicker = 0.75 + 0.25 * ((t * 7.3).sin() * 0.5 + (t * 3.1).sin() * 0.5);
            radial(&p, fire, art.height() * 0.75, EMBER.gamma_multiply(0.16 * flicker), Color32::TRANSPARENT);
            embers(&p.with_clip_rect(r.shrink(2.0)), r, fire, art.width() * 0.3, 70, t, 1.0);
            // The words: the name, the pool, the clock.
            let x = r.left() + 30.0;
            glow_text(&p, pos2(x, r.top() + 34.0), Align2::LEFT_CENTER, "THE BONFIRE", theme::bold_font(30.0), col::GOLD2, EMBER.gamma_multiply(0.55 + 0.15 * flicker));
            p.text(pos2(x, r.top() + 62.0), Align2::LEFT_CENTER, "Every round you play throws embers on the fire", theme::font(13.5), Color32::from_white_alpha(190));
            let pool = chips_text(v.pool);
            glow_text(&p, pos2(x, r.top() + h * 0.48), Align2::LEFT_CENTER, &pool, theme::bold_font((h * 0.22).clamp(40.0, 60.0)), col::GOLD, EMBER.gamma_multiply(0.45));
            p.text(pos2(x, r.top() + h * 0.48 + 38.0), Align2::LEFT_CENTER, "chips in the fire", theme::bold_font(12.5), col::MUTED);
            let left = v.ms_left.saturating_sub(at.elapsed().as_millis() as u64);
            let soon = left < 10 * 60_000;
            let lit = format!("LIT IN {}", clock(left));
            let g = p.layout_no_wrap(lit.clone(), theme::bold_font(15.0), if soon { col::GOLD2 } else { Color32::WHITE });
            let pill = Rect::from_min_size(pos2(x, r.bottom() - 52.0), g.size() + vec2(40.0, 16.0));
            p.rect_filled(pill, theme::r(16), if soon { EMBER.gamma_multiply(0.55 + 0.25 * (t * 4.0).sin().abs()) } else { Color32::from_black_alpha(170) });
            p.rect_stroke(pill, theme::r(16), Stroke::new(1.0f32, EMBER.gamma_multiply(0.8)), StrokeKind::Inside);
            flame(&p, pill.left_center() + vec2(15.0, 0.0), 7.0, t);
            p.galley(pill.left_center() + vec2(28.0, -g.size().y / 2.0), g, Color32::WHITE);
            // Mine: embers and the chance they give.
            let chance = if v.total_embers > 0 { v.my_embers as f64 / v.total_embers as f64 } else { 0.0 };
            let mine = if v.my_embers > 0 {
                format!("Your embers {}  ·  {} chance at the Grand Ember", chips_text(v.my_embers), percent(chance))
            } else {
                "Play any game to throw your first ember".to_string()
            };
            let mx = pill.right() + 14.0;
            p.text(pos2(mx, pill.center().y - 8.0), Align2::LEFT_CENTER, mine, theme::bold_font(13.0), col::GOLD2);
            p.text(pos2(mx, pill.center().y + 10.0), Align2::LEFT_CENTER, format!("14 winners: the Grand Ember half · 3 Blazes · 10 Sparks  ·  {}, {}", people(v.entrants), ember_count(v.total_embers)), theme::font(11.5), Color32::from_white_alpha(160));
            if let Some((n, when)) = self.poker.casino.bonfire.grew
                && when.elapsed() < Duration::from_millis(1_600)
            {
                let k = when.elapsed().as_secs_f32() / 1.6;
                glow_text(&p, pos2(mx + 40.0, pill.center().y - 24.0 - 20.0 * ease_out(k)), Align2::LEFT_CENTER, &format!("+{}", ember_count(n)), theme::bold_font(14.0), FLAME.gamma_multiply(1.0 - k), Color32::from_black_alpha((150.0 * (1.0 - k)) as u8));
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("How the Bonfire works, the last one lit, and who's thrown the most embers").clicked() {
                self.poker.casino.bonfire.panel = true;
            }
            ctx.request_repaint_after(Duration::from_millis(33));
        });
        self.bonfire_panel(&ctx, s, &v, at);
    }

    /// The Bonfire's window: how it works, who's thrown the most embers, the last fire's
    /// winners (checked by this Pyr), and for the owner, Light it now.
    fn bonfire_panel(&mut self, ctx: &egui::Context, s: &ServerView, v: &BonfireView, at: Instant) {
        if !self.poker.casino.bonfire.panel {
            return;
        }
        let mut open = true;
        let mut light = false;
        egui::Window::new("The Bonfire")
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(format!("{} chips in the fire · lit in {}", chips_text(v.pool), clock(v.ms_left.saturating_sub(at.elapsed().as_millis() as u64)))).strong().size(16.0).color(col::GOLD2));
                ui.add_space(6.0);
                ui.label(format!(
                    "Every round you play in Pyr Casino, in any game, throws embers on the fire: one for the round, and one more for every {} chips you bet. The fire starts at {} chips and grows by {}% of every chip bet (the house's cut, never anyone's winnings).",
                    bf::EMBER_CHIPS,
                    chips_text(bf::SEED_POOL),
                    bf::CUT_BP as f64 / 100.0
                ));
                ui.add_space(4.0);
                ui.label("When it's lit, fourteen embers are drawn, each with its owner's share of all the embers, and nobody wins twice: the Grand Ember takes half the pool, three Blazes a tenth each, ten Sparks a fiftieth each. Whatever nobody won goes on the next fire.");
                ui.add_space(4.0);
                ui.label(egui::RichText::new(format!("Fair: the house fixed this fire's seed before its first ember (its fingerprint: {}…). When it's lit, the seed and everyone's embers are shown, and your Pyr draws it again to be sure.", hexs(&v.commit[..6]))).size(12.0).color(col::MUTED));
                ui.add_space(10.0);
                ui.label(egui::RichText::new("The most embers on this fire").strong());
                if v.top.is_empty() {
                    ui.label(egui::RichText::new("Nobody yet: be the first.").color(col::MUTED));
                }
                for (n, (name, e)) in v.top.iter().enumerate() {
                    let chance = if v.total_embers > 0 { *e as f64 / v.total_embers as f64 } else { 0.0 };
                    ui.label(format!("{}. {name}: {} ({})", n + 1, ember_count(*e), percent(chance)));
                }
                if let Some(d) = &v.last {
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new(format!("The last fire (number {}): {} chips, {}", d.round, chips_text(d.pool), people(d.entries.len()))).strong());
                    for w in &d.winners {
                        ui.label(egui::RichText::new(format!("{}: {} +{}", w.tier.name(), w.name, chips_text(w.chips))).color(tier_color(w.tier)));
                    }
                    let ok = bf::check(d, None).is_ok();
                    ui.label(egui::RichText::new(if ok { "Drawn again by your Pyr: it checks out." } else { "Drawn again by your Pyr, it came out differently!" }).size(12.0).color(if ok { col::WIN } else { col::LOSE }));
                }
                if v.can_light {
                    ui.add_space(10.0);
                    if ui.button("Light it now (you're the server's owner)").clicked() {
                        light = true;
                    }
                }
            });
        if light {
            self.engine.poker(&s.key, ToServer::BonfireLight);
        }
        if !open {
            self.poker.casino.bonfire.panel = false;
        }
    }

    /// My embers, small, in the casino's top bar.
    pub(super) fn bonfire_pill(&mut self, p: &egui::Painter, s: &ServerView, right: f32, cy: f32) -> f32 {
        let Some((v, _)) = &s.poker.bonfire else { return right };
        let t = p.ctx().input(|i| i.time) as f32;
        let text = chips_text(v.my_embers);
        let g = p.layout_no_wrap(text.clone(), theme::bold_font(13.0), col::GOLD2);
        let r = Rect::from_min_max(pos2(right - g.size().x - 38.0, cy - 15.0), pos2(right, cy + 15.0));
        p.rect_filled(r, theme::r(15), Color32::from_black_alpha(150));
        p.rect_stroke(r, theme::r(15), Stroke::new(1.0f32, EMBER.gamma_multiply(0.6)), StrokeKind::Inside);
        flame(p, r.left_center() + vec2(14.0, 0.0), 6.5, t);
        p.galley(r.left_center() + vec2(26.0, -g.size().y / 2.0), g, col::GOLD2);
        if let Some((n, when)) = self.poker.casino.bonfire.grew
            && when.elapsed() < Duration::from_millis(1_400)
        {
            let k = when.elapsed().as_secs_f32() / 1.4;
            p.text(r.center_bottom() + vec2(0.0, 8.0 - 18.0 * ease_out(k)), Align2::CENTER_CENTER, format!("+{n}"), theme::bold_font(12.0), FLAME.gamma_multiply(1.0 - k));
        }
        r.left() - 8.0
    }

    /// The show when the Bonfire's lit, over everything (wherever I am in Pyr).
    pub fn bonfire_show(&mut self, ctx: &egui::Context, s: &ServerView) {
        let Some((d, came, checked)) = s.poker.bonfire_lit.clone() else { return };
        let fresh = pyr_proto::now_ms().max(0) as u64 <= d.at_ms + STALE_MS || came.elapsed() < Duration::from_millis(STALE_MS);
        if self.poker.casino.bonfire.closed == Some(d.round) || !fresh {
            return;
        }
        let start = match self.poker.casino.bonfire.show {
            Some((round, at)) if round == d.round => at,
            _ => {
                let now = Instant::now();
                self.poker.casino.bonfire.show = Some((d.round, now));
                self.poker.casino.bonfire.sounded = 0;
                now
            }
        };
        let k = start.elapsed().as_secs_f32();
        if k > CLOSE_AT {
            self.poker.casino.bonfire.closed = Some(d.round);
            return;
        }
        self.bonfire_sounds(&d, k, s);
        let me = s.me.as_ref().map(|m| m.user_id.clone()).unwrap_or_default();
        let screen = ctx.content_rect();
        let mut close = false;
        egui::Area::new(egui::Id::new(("bonfire-show", d.round))).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let resp = ui.allocate_rect(screen, Sense::click());
            if resp.clicked() && k > END_AT - 2.0 {
                close = true;
            }
            let p = ui.painter_at(screen);
            let t = ctx.input(|i| i.time) as f32;
            let fade = (k / 0.8).clamp(0.0, 1.0);
            p.rect_filled(screen, 0, Color32::from_black_alpha((235.0 * fade) as u8));
            // The fire: Meshy's bonfire, zooming slowly, roaring as the Grand Ember comes.
            let roar = if k > GRAND_AT { ((k - GRAND_AT) / GRAND_SPIN).min(1.0) } else { 0.0 };
            let zoom = 1.0 + k * 0.006 + roar * 0.05;
            let art_h = screen.height() * 0.78 * zoom;
            let art = Rect::from_center_size(pos2(screen.center().x, screen.top() + screen.height() * 0.46), vec2(art_h * 1376.0 / 768.0, art_h));
            let mut ap = p.clone();
            ap.set_opacity(fade * (0.55 + 0.25 * roar));
            pics::part(&ap, ctx, "bonfire", Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), art, 1600, Color32::WHITE);
            vgrad(&p, Rect::from_min_max(screen.min, pos2(screen.right(), screen.top() + screen.height() * 0.25)), Color32::from_black_alpha((200.0 * fade) as u8), Color32::TRANSPARENT);
            vgrad(&p, Rect::from_min_max(pos2(screen.left(), screen.bottom() - screen.height() * 0.3), screen.max), Color32::TRANSPARENT, Color32::from_black_alpha((230.0 * fade) as u8));
            let fire = pos2(art.center().x, art.top() + art.height() * 0.6);
            let flicker = 0.8 + 0.2 * (t * 9.0).sin() * (t * 3.7).cos();
            radial(&p, fire, screen.height() * (0.5 + 0.3 * roar), EMBER.gamma_multiply((0.22 + 0.25 * roar) * flicker * fade), Color32::TRANSPARENT);
            embers(&p, screen, fire, screen.width() * (0.18 + 0.12 * roar), 140 + (160.0 * roar) as u32, t * (1.0 + roar * 1.5), fade);
            // The title, and what's in the fire.
            let title = if k < GRAND_AT { "THE BONFIRE IS LIT" } else { "THE GRAND EMBER" };
            let ta = (k / 0.6).clamp(0.0, 1.0);
            glow_text(&p, pos2(screen.center().x, screen.top() + 58.0), Align2::CENTER_CENTER, title, theme::bold_font(44.0), col::GOLD2.gamma_multiply(ta), EMBER.gamma_multiply(0.7 * ta));
            p.text(pos2(screen.center().x, screen.top() + 98.0), Align2::CENTER_CENTER, format!("{} chips  ·  {}  ·  {}", chips_text(d.pool), people(d.entries.len()), ember_count(d.entries.iter().map(|e| e.embers).sum())), theme::bold_font(16.0), Color32::WHITE.gamma_multiply(ta * 0.85));
            if d.winners.is_empty() {
                if k > 1.5 {
                    glow_text(&p, screen.center(), Align2::CENTER_CENTER, "Nobody threw an ember this time: it all goes on the next fire", theme::bold_font(22.0), Color32::WHITE, Color32::from_black_alpha(160));
                }
            } else {
                self.bonfire_winners(&p, ctx, s, &d, screen, fire, k, t, &me);
            }
            if checked.is_err() && k > END_AT {
                p.text(pos2(screen.center().x, screen.bottom() - 70.0), Align2::CENTER_CENTER, "Your Pyr drew it again and it came out differently: see the casino's fairness check", theme::bold_font(14.0), col::LOSE);
            }
            if k > END_AT - 2.0 {
                p.text(pos2(screen.center().x, screen.bottom() - 36.0), Align2::CENTER_CENTER, "Click to go on", theme::font(13.0), Color32::from_white_alpha(150));
            }
        });
        if close {
            self.poker.casino.bonfire.closed = Some(d.round);
        }
        ctx.request_repaint();
    }

    /// The winners as they're drawn: Sparks flying out of the fire into a column on the left,
    /// Blazes landing on a name reel and going to the right, then the Grand Ember.
    #[allow(clippy::too_many_arguments)]
    fn bonfire_winners(&mut self, p: &egui::Painter, ctx: &egui::Context, s: &ServerView, d: &BonfireDraw, screen: Rect, fire: Pos2, k: f32, t: f32, me: &str) {
        let names: Vec<&str> = d.entries.iter().map(|e| e.name.as_str()).collect();
        let sparks: Vec<_> = d.winners.iter().filter(|w| w.tier == Tier::Spark).collect();
        let blazes: Vec<_> = d.winners.iter().filter(|w| w.tier == Tier::Blaze).collect();
        let grand = d.winners.iter().find(|w| w.tier == Tier::Grand);
        let row_h = 30.0;
        // Sparks: out of the fire, up, and into the column on the left.
        let col_l = pos2(screen.left() + 40.0, screen.top() + 150.0);
        for (n, w) in sparks.iter().enumerate() {
            let at = SPARKS_AT + n as f32 * SPARK_GAP;
            if k < at {
                continue;
            }
            let u = ((k - at) / 0.7).min(1.0);
            let to = col_l + vec2(0.0, n as f32 * row_h);
            let mid = pos2((fire.x + to.x) / 2.0, screen.top() + 120.0);
            let e = ease_out(u);
            let a_ = fire + (mid - fire) * e;
            let b_ = mid + (to - mid) * e;
            let q = a_ + (b_ - a_) * e;
            if u < 1.0 {
                p.circle_filled(q, 9.0 * (1.0 - u) + 3.0, FLAME);
                p.circle_filled(q, 18.0 * (1.0 - u) + 4.0, FLAME.gamma_multiply(0.25));
            } else {
                winner_row(p, to, w.tier, &w.name, w.chips, w.uid == me, true);
            }
        }
        // Blazes: a name reel in the middle, slowing onto the winner; then off to the right.
        let col_r = pos2(screen.right() - 40.0, screen.top() + 150.0);
        for (n, w) in blazes.iter().enumerate() {
            let at = BLAZES_AT + n as f32 * BLAZE_S;
            if k < at {
                continue;
            }
            let u = k - at;
            let reel_s = BLAZE_S * 0.72;
            if u < BLAZE_S {
                let c = pos2(screen.center().x, screen.top() + screen.height() * 0.62);
                name_reel(p, c, &names, &w.name, u / reel_s, 1.0, Tier::Blaze, t);
                if u >= reel_s {
                    let pop = ease_back(((u - reel_s) / 0.3).min(1.0));
                    glow_text(p, c + vec2(0.0, 48.0), Align2::CENTER_CENTER, &format!("A BLAZE: +{}", chips_text(w.chips)), theme::bold_font(22.0 * pop.max(0.1)), tier_color(Tier::Blaze), Color32::from_black_alpha(170));
                }
            } else {
                winner_row(p, col_r + vec2(0.0, n as f32 * row_h), w.tier, &w.name, w.chips, w.uid == me, false);
            }
        }
        // The Grand Ember: the slowest reel, past the biggest fires, then the fireworks.
        let Some(g) = grand else { return };
        if k < GRAND_AT {
            return;
        }
        let u = k - GRAND_AT;
        let c = pos2(screen.center().x, screen.top() + screen.height() * 0.58);
        if u < GRAND_SPIN {
            // (Slowing past the people with the most embers on the way.)
            let mut top: Vec<&pyr_poker::net::BonfireEntry> = d.entries.iter().collect();
            top.sort_by(|a, b| b.embers.cmp(&a.embers));
            let mut seq: Vec<&str> = names.clone();
            for e in top.iter().take(3) {
                seq.push(e.name.as_str());
            }
            name_reel(p, c, &seq, &g.name, u / GRAND_SPIN, 1.5, Tier::Grand, t);
            return;
        }
        let v = u - GRAND_SPIN;
        let pop = ease_back((v / 0.45).min(1.0));
        // Fireworks round the edge, coins falling.
        let colors = [col::GOLD2, EMBER, Color32::from_rgb(0x6a, 0xe8, 0x8a), Color32::from_rgb(0x6a, 0xa8, 0xff), Color32::from_rgb(0xff, 0x6a, 0xc8), Color32::WHITE];
        for n in 0..16u32 {
            let at = n as f32 * 0.55;
            let kk = v - at;
            if !(0.0..2.4).contains(&kk) {
                continue;
            }
            let x = screen.left() + screen.width() * (0.1 + 0.8 * h01(n, 3));
            let y = screen.top() + screen.height() * (0.12 + 0.4 * h01(n, 5));
            let color = colors[n as usize % colors.len()];
            let life = (1.0 - kk / 2.4).max(0.0);
            if kk < 0.3 {
                // (The bang: a flash where it bursts.)
                radial(p, pos2(x, y), 70.0 * (1.0 - kk / 0.3), Color32::WHITE.gamma_multiply(0.8 * (1.0 - kk / 0.3)), Color32::TRANSPARENT);
            }
            radial(p, pos2(x, y), 140.0, color.gamma_multiply(0.10 * life), Color32::TRANSPARENT);
            for sp in 0..44u32 {
                let a = sp as f32 / 44.0 * std::f32::consts::TAU + h01(n, sp) * 0.25;
                let reach = 0.75 + 0.35 * h01(n, sp + 100);
                let rr = 135.0 * reach * ease_out((kk / 1.1).min(1.0));
                let drop = kk * kk * 22.0;
                let q = pos2(x + a.cos() * rr, y + a.sin() * rr + drop);
                // (A streak behind each spark as it flies.)
                let tail = pos2(x + a.cos() * rr * 0.82, y + a.sin() * rr * 0.82 + drop * 0.8);
                p.line_segment([tail, q], Stroke::new(2.2 * life + 0.4, color.gamma_multiply(0.55 * life)));
                p.circle_filled(q, 3.4 * life + 0.6, color.gamma_multiply(life));
            }
        }
        for n in 0..60u32 {
            let speed = 160.0 + h01(n, 9) * 220.0;
            let y = screen.top() - 20.0 + ((v * speed + h01(n, 11) * screen.height()) % (screen.height() + 40.0));
            let x = screen.left() + h01(n, 13) * screen.width();
            let wob = (v * 6.0 + n as f32).sin();
            p.add(egui::epaint::EllipseShape::filled(pos2(x, y), vec2(7.0 * wob.abs().max(0.2), 7.0), col::GOLD));
        }
        // The winner: their picture in a ring of gold, a crown, their name and the prize.
        let ring = 78.0 * pop.max(0.05);
        radial(p, c, ring * 2.6, col::GOLD.gamma_multiply(0.5), Color32::TRANSPARENT);
        p.circle_filled(c, ring, Color32::from_rgb(0x1a, 0x10, 0x06));
        if g.uid.starts_with("bot:") {
            crate::poker::table::bot_face(p, c, ring * 0.86, &g.name, pyr_poker::bots::Level::Regular);
        } else {
            let face = self.face(ctx, s, &g.uid);
            crate::app::avatar(p, c, ring * 0.86, &g.uid, &g.name, false, Color32::BLACK, face);
        }
        p.circle_stroke(c, ring, Stroke::new(5.0f32, col::GOLD));
        crown(p, c - vec2(0.0, ring + 26.0 * pop), 48.0 * pop);
        let mine = g.uid == me;
        let who = if mine { "YOU WIN THE GRAND EMBER!".to_string() } else { format!("{} WINS THE GRAND EMBER!", g.name.to_uppercase()) };
        glow_text(p, c + vec2(0.0, ring + 46.0), Align2::CENTER_CENTER, &who, theme::bold_font(40.0 * pop.max(0.1)), col::GOLD2, EMBER.gamma_multiply(0.8));
        glow_text(p, c + vec2(0.0, ring + 92.0), Align2::CENTER_CENTER, &format!("{} chips", chips_text(g.chips)), theme::bold_font(34.0 * pop.max(0.1)), Color32::WHITE, Color32::from_black_alpha(170));
        // Mine, if I won anything else.
        if !mine && let Some(w) = d.winners.iter().find(|w| w.uid == me) {
            p.text(c + vec2(0.0, ring + 132.0), Align2::CENTER_CENTER, format!("And you won a {}: +{} chips!", w.tier.name(), chips_text(w.chips)), theme::bold_font(18.0), tier_color(w.tier));
        }
    }

    /// The show's sounds, each once.
    fn bonfire_sounds(&mut self, d: &BonfireDraw, k: f32, s: &ServerView) {
        let me = s.me.as_ref().map(|m| m.user_id.clone()).unwrap_or_default();
        let mut cues: Vec<(u32, Cue, f32)> = vec![(0, Cue::Boom, 0.55)];
        let sparks = d.winners.iter().filter(|w| w.tier == Tier::Spark).count();
        for n in 0..sparks {
            cues.push((1 + n as u32, Cue::Coin, 0.6));
        }
        let blazes = d.winners.iter().filter(|w| w.tier == Tier::Blaze).count();
        for n in 0..blazes {
            cues.push((20 + n as u32, Cue::ReelSpin, 0.6));
            cues.push((30 + n as u32, Cue::ReelStop, 0.8));
        }
        let has_grand = d.winners.iter().any(|w| w.tier == Tier::Grand);
        if has_grand {
            cues.push((40, Cue::Wheel, 0.7));
            cues.push((41, Cue::Jackpot, 0.95));
        }
        let at = |bit: u32| -> f32 {
            match bit {
                0 => 0.2,
                1..=19 => SPARKS_AT + (bit - 1) as f32 * SPARK_GAP + 0.55,
                20..=29 => BLAZES_AT + (bit - 20) as f32 * BLAZE_S,
                30..=39 => BLAZES_AT + (bit - 30) as f32 * BLAZE_S + BLAZE_S * 0.72,
                40 => GRAND_AT + 0.2,
                _ => GRAND_AT + GRAND_SPIN,
            }
        };
        for (bit, cue, vol) in cues {
            let mask = 1u64 << bit;
            if self.poker.casino.bonfire.sounded & mask == 0 && k >= at(bit) {
                self.poker.casino.bonfire.sounded |= mask;
                self.engine.play_poker(cue, 0.0, vol);
            }
        }
        // Mine: the celebration, once.
        let mask = 1u64 << 50;
        if self.poker.casino.bonfire.sounded & mask == 0 && k >= END_AT - 1.0 {
            self.poker.casino.bonfire.sounded |= mask;
            if let Some(w) = d.winners.iter().find(|w| w.uid == me) {
                self.engine.play_poker(Cue::WinBig, 0.0, 0.9);
                let _ = w;
            }
        }
    }
}

/// A winner's line in a column: their prize's name, theirs, and the chips.
fn winner_row(p: &egui::Painter, at: Pos2, tier: Tier, name: &str, chips: u64, mine: bool, left: bool) {
    let text = format!("{}  {}  +{}", tier.name().to_uppercase(), name, chips_text(chips));
    let g = p.layout_no_wrap(text, theme::bold_font(14.0), tier_color(tier));
    let w = g.size().x + 26.0;
    let r = if left { Rect::from_min_size(at - vec2(0.0, 12.0), vec2(w, 26.0)) } else { Rect::from_min_size(at - vec2(w, 12.0), vec2(w, 26.0)) };
    p.rect_filled(r, theme::r(13), Color32::from_black_alpha(200));
    p.rect_stroke(r, theme::r(13), Stroke::new(if mine { 2.0f32 } else { 1.0 }, if mine { col::GOLD2 } else { tier_color(tier).gamma_multiply(0.6) }), StrokeKind::Inside);
    p.galley(r.left_center() + vec2(13.0, -g.size().y / 2.0), g, tier_color(tier));
}

/// A name reel: names rushing past, slowing to a stop on `winner` as `u` goes 0 to 1 (`size`:
/// how big).
fn name_reel(p: &egui::Painter, c: Pos2, names: &[&str], winner: &str, u: f32, size: f32, tier: Tier, t: f32) {
    let u = u.clamp(0.0, 1.0);
    let w = 420.0 * size;
    let h = 64.0 * size;
    let frame = Rect::from_center_size(c, vec2(w, h * 2.2));
    p.rect_filled(frame, theme::r(18), Color32::from_black_alpha(215));
    p.rect_stroke(frame, theme::r(18), Stroke::new(2.0f32, tier_color(tier).gamma_multiply(0.8 + 0.2 * (t * 5.0).sin())), StrokeKind::Inside);
    // How far it's gone: fast, then easing out onto the winner (one name a row).
    let rows = 40.0 + 20.0 * size;
    let pos = rows * (1.0 - (1.0 - u).powi(3));
    let clip = p.with_clip_rect(frame.shrink(4.0));
    let n = names.len().max(1);
    let base = pos.floor();
    for off in -2..=2 {
        let row = base as i64 + off as i64;
        let y = c.y + (off as f32 - (pos - base)) * h;
        // The last row it reaches is the winner's.
        let name = if row == rows as i64 { winner } else { names[(row.rem_euclid(n as i64)) as usize] };
        let dist = ((y - c.y) / h).abs();
        let a = (1.0 - dist * 0.55).clamp(0.15, 1.0);
        let landed = u >= 1.0 && row == rows as i64;
        let color = if landed { tier_color(tier) } else { Color32::WHITE };
        clip.text(pos2(c.x, y), Align2::CENTER_CENTER, name, theme::bold_font(30.0 * size * (1.0 - dist * 0.25)), color.gamma_multiply(a));
    }
    // The line it stops on.
    p.line_segment([pos2(frame.left() + 10.0, c.y - h * 0.5), pos2(frame.right() - 10.0, c.y - h * 0.5)], Stroke::new(1.0f32, tier_color(tier).gamma_multiply(0.4)));
    p.line_segment([pos2(frame.left() + 10.0, c.y + h * 0.5), pos2(frame.right() - 10.0, c.y + h * 0.5)], Stroke::new(1.0f32, tier_color(tier).gamma_multiply(0.4)));
}

/// A little flame, flickering.
fn flame(p: &egui::Painter, c: Pos2, r: f32, t: f32) {
    let f = 1.0 + 0.12 * (t * 11.0).sin();
    p.circle_filled(c + vec2(0.0, r * 0.2), r * 0.9, EMBER);
    p.add(egui::Shape::convex_polygon(vec![c + vec2(-r * 0.85, r * 0.15), c + vec2(0.0, -r * 1.6 * f), c + vec2(r * 0.85, r * 0.15)], EMBER, Stroke::NONE));
    p.circle_filled(c + vec2(0.0, r * 0.3), r * 0.45, col::GOLD2);
}

/// A crown over the Grand Ember's winner: a band and three points, a jewel on each.
fn crown(p: &egui::Painter, c: Pos2, s: f32) {
    if s <= 0.5 {
        return;
    }
    let band = Rect::from_min_max(c + vec2(-s, s * 0.05), c + vec2(s, s * 0.5));
    for (x, tip) in [(-s * 0.68, -s * 0.38), (0.0, -s * 0.62), (s * 0.68, -s * 0.38)] {
        let pts = vec![c + vec2(x - s * 0.36, s * 0.1), c + vec2(x, tip), c + vec2(x + s * 0.36, s * 0.1)];
        p.add(egui::Shape::convex_polygon(pts, col::GOLD, Stroke::new(1.0f32, col::GOLD_DARK)));
        p.circle_filled(c + vec2(x, tip), s * 0.11, col::GOLD2);
    }
    p.rect_filled(band, theme::r(3), col::GOLD);
    p.rect_stroke(band, theme::r(3), Stroke::new(1.0f32, col::GOLD_DARK), StrokeKind::Inside);
    p.circle_filled(band.center(), s * 0.1, Color32::from_rgb(0xd2, 0x2f, 0x3c));
}

/// A chance in words: "12.5%", "0.08%".
/// "1 person", "3 people".
fn people(n: usize) -> String {
    if n == 1 { "1 person".to_string() } else { format!("{n} people") }
}

/// "1 ember", "1,204 embers".
fn ember_count(n: u64) -> String {
    if n == 1 { "1 ember".to_string() } else { format!("{} embers", chips_text(n)) }
}

fn percent(x: f64) -> String {
    let pct = x * 100.0;
    if pct >= 10.0 {
        format!("{pct:.0}%")
    } else if pct >= 1.0 {
        format!("{pct:.1}%")
    } else if pct > 0.0 {
        format!("{pct:.2}%")
    } else {
        "0%".into()
    }
}

fn hexs(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
