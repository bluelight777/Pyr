//! Levels and the Pyr VIP Club on screen (0.13.5), drawn as 0.13.28 drew them: the tier's crest
//! on the player card with its title and what the next level pays, today's three missions and
//! Ember Back under it, the VIP Club page (every tier, its crest and perks, where you are and
//! what's next), and the crest bursting onto the screen with the chips a level paid.
//!
//! (On a server from before levels, `RecordView::ember_bp` is 0: the old card stays.)

use std::time::Instant;

use eframe::egui::{self, Align2, Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_poker::levels;
use pyr_engine::state::ServerView;
use pyr_poker::net::{MissionView, RecordView, ToServer};

use crate::app::PyrApp;
use crate::casino::pics;
use crate::poker::cards::chips_text;
use crate::theme;

/// Whether a server knows levels (it sends a tier's share of the house's edge).
pub fn has_levels(me: &RecordView) -> bool {
    me.ember_bp > 0
}

/// A tier's crest (0, Spark, to 9, Eternal Flame) in `r`.
pub fn crest(p: &egui::Painter, ctx: &egui::Context, tier: usize, r: Rect) -> bool {
    crest_tinted(p, ctx, tier, r, Color32::WHITE)
}

/// A tier's crest drawn through `tint` (white at an alpha: faded).
fn crest_tinted(p: &egui::Painter, ctx: &egui::Context, tier: usize, r: Rect, tint: Color32) -> bool {
    let i = tier.min(9) as f32;
    pics::part(p, ctx, "tier-crests", Rect::from_min_max(pos2(i / 10.0, 0.0), pos2((i + 1.0) / 10.0, 1.0)), r, 1920, tint)
}

/// A tier's color (its title, its ring).
pub fn tier_color(t: usize) -> Color32 {
    // (0.13.28's, read off its tier frames.)
    match t {
        0 => Color32::from_rgb(0xd0, 0x8a, 0x5a),
        1 => Color32::from_rgb(0xc8, 0x82, 0x4a),
        2 => Color32::from_rgb(0xc8, 0xd0, 0xdc),
        3 => Color32::from_rgb(0xff, 0xc8, 0x40),
        4 => Color32::from_rgb(0xff, 0x9a, 0x3a),
        5 => Color32::from_rgb(0x4a, 0xe0, 0x7a),
        6 => Color32::from_rgb(0xff, 0x4a, 0x3a),
        7 => Color32::from_rgb(0xff, 0xd8, 0x5a),
        8 => Color32::from_rgb(0x6a, 0xa8, 0xff),
        _ => Color32::from_rgb(0xc0, 0x7a, 0xff),
    }
}


/// A day's mission's color, by its corner of the casino (0.13.28's, read off its bars).
fn corner_color(i: usize) -> Color32 {
    match i {
        0 => Color32::from_rgb(46, 184, 106),
        1 => Color32::from_rgb(216, 58, 74),
        _ => Color32::from_rgb(255, 138, 42),
    }
}

const CORNERS: [&str; 3] = ["TODAY · POKER", "TODAY · TABLES", "TODAY · SLOTS & ORIGINALS"];

/// A percent from hundredths of one ("7.5%", "5%").
pub fn percent(bp: u64) -> String {
    if bp % 100 == 0 { format!("{}%", bp / 100) } else { format!("{}%", (bp as f64 / 100.0)) }
}

/// What the line under my name says: my title, and the way to the next level (or star).
pub fn next_line(me: &RecordView) -> (String, String) {
    let title = levels::title(me.level);
    let to_go = me.level_to.saturating_sub(me.xp);
    if me.level >= levels::MAX_LEVEL {
        return (format!("{title} · ★ {}", me.stars), format!("{} XP to the next star (pays {})", chips_text(to_go), chips_text(me.next_reward)));
    }
    // ("almost there" in the last 15% of a level, as 0.13.28 said it.)
    let near = to_go * 100 <= (me.level_to - me.level_from).max(1) * 15;
    let lead = if near { "almost there: " } else { "" };
    (title, format!("{lead}{} XP to level {} (pays {})", chips_text(to_go), me.level + 1, chips_text(me.next_reward)))
}

/// The dark inside of a crest's ring and of its level's pill.
const CREST_DARK: Color32 = Color32::from_rgb(20, 16, 22);

/// The crest in its ring (the level's progress around it) with the level on a pill under it, as
/// 0.13.28 draws it (read off its screen at 6x): `size` sets the picture (half again as wide),
/// its glow and where the pill sits (the player card's 32, the VIP card's 62); `ring` is the
/// radius of the dark disc, its faint ring and the progress round it (32 and 60). `halo`: the
/// VIP card's wide soft glow of the tier's color, over all that's under it.
pub fn crest_badge(p: &egui::Painter, ctx: &egui::Context, me: &RecordView, c: Pos2, size: f32, ring: f32, halo: bool) {
    let t = me.tier as usize;
    let tc = tier_color(t);
    if halo {
        crate::casino::radial(p, c, 220.0, tc.gamma_multiply(0.18), Color32::TRANSPARENT);
    }
    // Its glow: four faint discs of the tier's color, a step apart.
    for i in 0..4 {
        p.circle_filled(c, size * 1.17 * (1.0 - 0.0483 * i as f32), tc.gamma_multiply(0.035));
    }
    p.circle_filled(c, ring, CREST_DARK);
    p.circle_stroke(c, ring, Stroke::new(3.0f32, tc.gamma_multiply(0.3)));
    let span = (me.level_to.saturating_sub(me.level_from)).max(1) as f32;
    let frac = (me.xp.saturating_sub(me.level_from) as f32 / span).clamp(0.0, 1.0);
    crate::poker::arc(p, c, ring, frac, Stroke::new(4.0f32, tc));
    let s = size * 1.5;
    crest(p, ctx, t, Rect::from_center_size(c - vec2(0.0, size * 0.0775), vec2(s, s)));
    // The level on a pill, edged in the tier's color.
    let level = p.layout_no_wrap(me.level.to_string(), theme::bold_font(11.5), Color32::WHITE);
    let pill = Rect::from_center_size(c + vec2(0.0, size * 0.890625), vec2(level.size().x + 14.0, 17.0));
    p.rect_filled(pill, egui::CornerRadius::same(9), CREST_DARK);
    p.rect_stroke(pill, egui::CornerRadius::same(9), Stroke::new(1.5f32, tc), StrokeKind::Inside);
    p.galley(pill.center() - level.size() / 2.0, level, Color32::WHITE);
}

/// A Claim button as 0.13.28's: the accent with dark letters when there's something to claim,
/// grey with muted ones when there isn't (its letters `size`: Ember Back's 13.5, the weekly
/// bonus's 13).
fn claim_button(ready: bool, size: f32) -> egui::Button<'static> {
    egui::Button::new(RichText::new("Claim").font(theme::bold_font(size)).color(if ready { theme::on_ember() } else { theme::MUTED() }))
        .fill(if ready { theme::EMBER() } else { theme::ELEV2() })
        .corner_radius(theme::r(10))
}

/// A level reached, shown big for a few seconds: the crest bursting onto the screen with the
/// chips it paid (a whole new tier, bigger still).
pub struct Burst {
    pub level: u32,
    pub tier_up: bool,
    pub chips: u64,
    pub since: Instant,
}

const BURST_SECS: f32 = 3.6;

impl PyrApp {
    /// Under the player card on the casino's home: today's three missions and Ember Back.
    pub(crate) fn levels_row(&mut self, ui: &mut egui::Ui, s: &ServerView, me: &RecordView) {
        if !has_levels(me) {
            return;
        }
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            let w = (ui.available_width() - 28.0).max(600.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 76.0), Sense::hover());
            let gap = 10.0;
            let cw = (w - 3.0 * gap) / 4.0;
            let p = ui.painter().clone();
            for i in 0..3 {
                let c = Rect::from_min_size(pos2(r.left() + i as f32 * (cw + gap), r.top()), vec2(cw, r.height()));
                mission_card(&p, c, i, me.missions.get(i));
            }
            // Ember Back.
            let c = Rect::from_min_size(pos2(r.left() + 3.0 * (cw + gap), r.top()), vec2(cw, r.height()));
            // An ember's dark, down the card, edged in the tier's color.
            crate::casino::vgrad_round(&p, c, theme::rf(12.0), Color32::from_rgb(58, 22, 12), Color32::from_rgb(26, 10, 8));
            let tc = tier_color(me.tier as usize);
            p.rect_stroke(c, theme::r(12), Stroke::new(1.0f32, tc.gamma_multiply(0.7)), StrokeKind::Inside);
            let x = c.left() + 14.0;
            p.text(pos2(x, c.top() + 14.0), Align2::LEFT_CENTER, format!("EMBER BACK · {} AT {}", percent(me.ember_bp), levels::TIERS[(me.tier as usize).min(9)].0.to_uppercase()), theme::bold_font(10.0), tc);
            p.text(pos2(x, c.top() + 40.0), Align2::LEFT_CENTER, format!("{} chips", chips_text(me.ember_back)), theme::bold_font(20.0), crate::casino::col::GOLD2);
            let under = p.layout_no_wrap("a share of the house's edge on your play".into(), theme::font(11.0), theme::MUTED());
            // (Kept inside the card: 0.13.28's ran past its edge.)
            let k = ((c.width() - 24.0) / under.size().x).min(1.0);
            let under = if k < 1.0 { p.layout_no_wrap("a share of the house's edge on your play".into(), theme::font(11.0 * k), theme::MUTED()) } else { under };
            p.galley(pos2(x, c.bottom() - 13.0 - under.size().y / 2.0), under, theme::MUTED());
            let b = Rect::from_min_size(pos2(c.right() - 86.0, c.top() + 24.0), vec2(74.0, 32.0));
            let ready = me.ember_back > 0;
            let resp = ui.put(b, claim_button(ready, 13.5));
            // (0.13.28's words on Claim, pointed at.)
            if resp.on_hover_text("Ember Back builds with every hand and spin: the higher your tier, the bigger your share. Claim it any time.").clicked() && ready {
                self.engine.poker(&s.key, ToServer::ClaimEmber);
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        });
    }

    /// The Pyr VIP Club: where I am, its perks, the weekly bonus, and every tier.
    pub(crate) fn vip_page(&mut self, ui: &mut egui::Ui, s: &ServerView, me: &RecordView) {
        let ctx = ui.ctx().clone();
        // (Kept fresh: the lobby, and my record with it, is asked for every few seconds.)
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        let t = (me.tier as usize).min(9);
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            let w = (ui.available_width() - 28.0).max(600.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 170.0), Sense::hover());
            let p = ui.painter().clone();
            // The same dark for every tier, edged in the tier's color (0.13.28's).
            let tc = tier_color(t);
            crate::casino::vgrad_round(&p, r, theme::rf(16.0), Color32::from_rgb(36, 16, 24), Color32::from_rgb(14, 8, 12));
            p.rect_stroke(r, theme::r(16), Stroke::new(1.5f32, tc.gamma_multiply(0.8)), StrokeKind::Inside);
            crest_badge(&p, &ctx, me, pos2(r.left() + 110.0, r.center().y), 62.0, 60.0, true);
            let x = r.left() + 210.0;
            p.text(pos2(x, r.top() + 30.0), Align2::LEFT_CENTER, "PYR VIP CLUB", theme::bold_font(12.0), tc);
            crate::casino::glow_text(&p, pos2(x, r.top() + 64.0), Align2::LEFT_CENTER, &levels::title(me.level), theme::bold_font(30.0), Color32::WHITE, tc.gamma_multiply(0.6));
            let next = if t + 1 < levels::TIERS.len() {
                let (first, _) = levels::tier_levels(t + 1);
                let n = first - me.level;
                format!("Level {} · {} more level{} to {}", me.level, n, if n == 1 { "" } else { "s" }, levels::TIERS[t + 1].0)
            } else {
                format!("Level {} · ★ {}", me.level, me.stars)
            };
            p.text(pos2(x, r.top() + 98.0), Align2::LEFT_CENTER, next, theme::font(14.0), theme::MUTED());
            p.text(
                pos2(x, r.top() + 124.0),
                Align2::LEFT_CENTER,
                format!("Ember Back {} of the house's edge · daily chips ×{:.2}", percent(me.ember_bp), levels::daily_x100(t) as f64 / 100.0),
                theme::font(12.5),
                theme::DIM(),
            );
            // The weekly VIP bonus: a darker box edged in the tier's color.
            let wb = Rect::from_min_size(pos2(r.right() - 250.0, r.top() + 40.0), vec2(226.0, 90.0));
            p.rect_filled(wb, theme::r(12), Color32::from_black_alpha(110));
            p.rect_stroke(wb, theme::r(12), Stroke::new(1.0f32, tc.gamma_multiply(0.6)), StrokeKind::Inside);
            p.text(pos2(wb.left() + 14.0, wb.top() + 16.0), Align2::LEFT_CENTER, "WEEKLY VIP BONUS", theme::bold_font(10.5), tc);
            p.text(pos2(wb.left() + 14.0, wb.top() + 42.0), Align2::LEFT_CENTER, format!("{} chips", chips_text(me.weekly_ready)), theme::bold_font(19.0), crate::casino::col::GOLD2);
            let under = if t == 0 { "from Ember tier up".to_string() } else { format!("this week's so far: {}", chips_text(me.weekly_building)) };
            p.text(pos2(wb.left() + 14.0, wb.bottom() - 22.0), Align2::LEFT_CENTER, under, theme::font(11.0), theme::MUTED());
            let b = Rect::from_min_size(pos2(wb.right() - 78.0, wb.top() + 26.0), vec2(66.0, 30.0));
            let ready = me.weekly_ready > 0;
            let resp = ui.put(b, claim_button(ready, 13.0));
            // (No words on it: 0.13.28 has none, pointed at.)
            if resp.clicked() && ready {
                self.engine.poker(&s.key, ToServer::ClaimWeekly);
            }
        });
        // (24 under it, as 0.13.28's: egui's own spacing makes up the rest.)
        ui.add_space(18.0);
        // Every tier.
        let cols = 4;
        for row in 0..levels::TIERS.len().div_ceil(cols) {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                let w = (ui.available_width() - 28.0).max(600.0);
                let gap = 12.0;
                let cw = (w - gap * (cols as f32 - 1.0)) / cols as f32;
                // (Tall enough for every perk: 0.13.28's ran out past the card's bottom.)
                let (r, _) = ui.allocate_exact_size(vec2(w, 284.0), Sense::hover());
                let p = ui.painter().clone();
                for c in 0..cols {
                    let i = row * cols + c;
                    if i >= levels::TIERS.len() {
                        break;
                    }
                    tier_card(&p, &ctx, Rect::from_min_size(pos2(r.left() + c as f32 * (cw + gap), r.top()), vec2(cw, r.height())), i, t);
                }
            });
            // (12 between rows, as 0.13.28's.)
            ui.add_space(6.0);
        }
    }

    /// The level-up burst, over the casino (as 0.13.28's: the crest with "LEVEL 13" and the
    /// level's title, the chips it paid counting up; a new tier bigger, "EMBER!").
    pub(crate) fn level_burst(&mut self, ctx: &egui::Context, area: Rect) {
        let Some(b) = &self.poker.burst else { return };
        let t = b.since.elapsed().as_secs_f32();
        let life = if b.tier_up { TIER_UP_SECS } else { BURST_SECS };
        if t > life {
            self.poker.burst = None;
            return;
        }
        let tier = levels::tier(b.level);
        let col = tier_color(tier);
        // Over the page in an area of its own, as 0.13.28's (filmed): it takes the page's clicks
        // while it shows (a click on Roll rolls nothing; the keys still go through), a click from
        // 0.7 s in sends it away, and it's over what was showing when it came (the toasts already
        // up are under its shade; a toast coming after it is over it).
        let mut gone = false;
        egui::Area::new(egui::Id::new("level-burst")).order(egui::Order::Foreground).fixed_pos(area.min).show(ctx, |ui| {
            if ui.allocate_rect(area, Sense::click()).clicked() && t > 0.7 {
                gone = true;
            }
            let p = ui.painter_at(area);
            if b.tier_up {
                tier_up_burst(&p, ctx, area, b, t, tier, col);
            } else {
                level_up_burst(&p, ctx, area, b, t, tier, col);
            }
        });
        if gone {
            self.poker.burst = None;
        }
        ctx.request_repaint();
    }
}

/// A new tier's burst lasts longer.
const TIER_UP_SECS: f32 = 6.5;

/// A new tier reached, as 0.13.28 shows it: measured off its screen, filmed at 1280 by 800 (four
/// times) and at 1600 by 1000 (`rebuild/measured/levels.md`). A level's burst a quarter bigger,
/// darker, longer, with more rays and sparkles flying off the crest: the old tier's crest shown
/// first, the new one bursting out in front of it as the old one fades.
fn tier_up_burst(p: &egui::Painter, ctx: &egui::Context, area: Rect, b: &Burst, t: f32, tier: usize, col: Color32) {
    let p = &p.with_clip_rect(area);
    let k = (area.height() / 800.0).min(242.0 / 211.2);
    let c = pos2(area.center().x, area.top() + area.height() * 0.44);
    let full = 264.0 * k;
    // In over 0.25 s, out over the last 0.6 s.
    let fade = (t / 0.25).min(1.0).min(((TIER_UP_SECS - t) / 0.6).clamp(0.0, 1.0));
    // The page darkens (black at 200).
    p.rect_filled(area, 0.0, Color32::from_black_alpha((200.0 * fade).round() as u8));
    // Twenty-eight rays of the tier's color, turning slowly behind (with the app's clock, as
    // 0.13.28's: where they are when it comes is where they'd got to).
    let now = ctx.input(|i| i.time);
    burst_rays(p, c, area, 28, 0.5, k, ((now * 0.2508) % std::f64::consts::TAU) as f32, col.gamma_multiply(0.05 * fade));
    // A ring goes out from it, thinning and fading (whole from the start): a level's path, a
    // quarter farther, its stroke a little thinner (7.9 at first).
    let u = t / 0.93;
    if u < 1.0 {
        let w = 7.9 * (1.0 - u) * k;
        let mid = 1.25 * (75.0 + 275.0 * (1.0 - (1.0 - u).powi(3))) * k;
        p.circle_stroke(c, mid - w / 2.0, Stroke::new(w, col.gamma_multiply(0.95 * (1.0 - u))));
    }
    // Six faint discs of the tier's color, the new crest's whole size from the start.
    glow_discs(p, c, full, col.gamma_multiply(0.05 * fade));
    // Fourteen sparkles fly off it, each its own way round (2.399 rad apart), one after another
    // (with the app's clock too): born on the old crest's rim (120 out), gone at 192, shrinking
    // and fading as they go.
    for i in 0..14 {
        let life = ((now * 0.7 + 0.37 * i as f64).fract()) as f32;
        let a = i as f32 * 2.399;
        let at = c + vec2(a.cos(), a.sin()) * (120.0 + 72.0 * life) * k;
        p.circle_filled(at, 3.1 * (1.0 - life) * k, col.gamma_multiply((1.0 - life) * fade));
    }
    // The old tier's crest, smaller, fading as the new one comes.
    let old = (0.5 + 6.0 * t).min((1.0 - t / 0.65).max(0.0).powf(1.3)).clamp(0.0, 1.0);
    if old > 0.0 && tier > 0 {
        let s = 240.0 * k;
        crest_tinted(p, ctx, tier - 1, Rect::from_center_size(c, vec2(s, s)), Color32::WHITE.gamma_multiply(old));
    }
    // The new one pops in, past its size and back (as a level's).
    let size = full * ease_out_back((t - 0.09) / 0.57, 1.2);
    if size > 0.0 {
        crest_tinted(p, ctx, tier, Rect::from_center_size(c, vec2(size, size)), Color32::WHITE.gamma_multiply(fade));
    }
    // "FLAME!" pops in after it, white edged in the tier's color.
    let pop = ease_out_back((t - 0.45) / 0.43, 1.2);
    if pop > 0.0 {
        let big = format!("{}!", levels::TIERS[tier].0.to_uppercase());
        outlined(p, pos2(c.x, c.y + 162.0 * k), &big, theme::bold_font(74.375 * k * pop), Color32::WHITE.gamma_multiply(fade), col.gamma_multiply(fade));
    }
    // What the tier gives (its size doesn't grow with the page, its place does).
    let felt = matches!(tier, 3 | 5 | 8);
    let sub = format!("Level {} · a new tier: its frame{} yours", b.level, if felt { " and felt are" } else { " is" });
    p.text(pos2(c.x, c.y + 228.0 * k), Align2::CENTER_CENTER, sub, theme::bold_font(16.0), col.gamma_multiply(fade));
    // The chips it paid, counting up from 0.8 s, over a second.
    if b.chips > 0 && t >= 0.8 {
        let x = (t - 0.8).min(1.0);
        let shown = (b.chips as f64 * (1.0 - (1.0 - x as f64).powi(3))).round() as u64;
        outlined(p, pos2(c.x, c.y + 288.0 * k), &format!("+{} chips", chips_text(shown)), theme::bold_font(45.6 * k), crate::casino::col::GOLD2.gamma_multiply(fade), Color32::from_black_alpha((200.0 * fade) as u8));
    }
}

/// A burst's rays round `c`: `n` of them from `turn` (radians), each `share` of its part of the
/// turn wide (as an angle, at 800 high: narrower in angle on a bigger page, as 0.13.28's) and 6 px
/// wider than that (3 either side, a page's height over 800 times).
fn burst_rays(p: &egui::Painter, c: Pos2, area: Rect, n: usize, share: f32, k: f32, turn: f32, color: Color32) {
    use std::f32::consts::TAU;
    let reach = area.width().hypot(area.height());
    // Each side widens by this much for every pixel out.
    let spread = (TAU / n as f32 * share / 2.0).tan() / k;
    let base = 3.0 * k;
    for i in 0..n {
        let a = i as f32 / n as f32 * TAU + turn;
        let dir = vec2(a.cos(), a.sin());
        let side = vec2(-dir.y, dir.x);
        let far = c + dir * reach;
        let w = base + spread * reach;
        let pts = vec![c - side * base, far - side * w, far + side * w, c + side * base];
        p.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    }
}

/// A burst's glow: six faint discs round its crest (`size` its whole size), a step apart.
fn glow_discs(p: &egui::Painter, c: Pos2, size: f32, color: Color32) {
    for i in (0..6).rev() {
        p.circle_filled(c, size * (0.3211 + 0.0812 * i as f32), color);
    }
}

/// Back-out easing: 0 at 0, past 1 a little (by about 5% at c1 1.2), 1 at 1.
fn ease_out_back(x: f32, c1: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let c3 = c1 + 1.0;
    1.0 + c3 * (x - 1.0).powi(3) + c1 * (x - 1.0).powi(2)
}

/// A level reached (not a new tier), as 0.13.28 shows it: measured off its screen, filmed at 30 a
/// second at 1280 by 800 and at 700, 900 and 1000 high (`rebuild/measured/levels.md`). Everything
/// sized by the page's height (to 1000 high: the crest 0.264 of it, 242 at most), the crest's
/// middle 0.44 of the way down.
fn level_up_burst(p: &egui::Painter, ctx: &egui::Context, area: Rect, b: &Burst, t: f32, tier: usize, col: Color32) {
    // (Over the casino's page only: 0.13.28 draws nothing on the sidebar, rays neither.)
    let p = &p.with_clip_rect(area);
    let k = (area.height() / 800.0).min(242.0 / 211.2);
    let c = pos2(area.center().x, area.top() + area.height() * 0.44);
    // In over 0.23 s, out over the last 0.6 s.
    let fade = (t / 0.23).min(1.0).min(((BURST_SECS - t) / 0.6).clamp(0.0, 1.0));
    // The page darkens; eighteen rays of the tier's color turn slowly behind (with the app's
    // clock, as 0.13.28's).
    p.rect_filled(area, 0.0, Color32::from_black_alpha((139.0 * fade).round() as u8));
    let now = ctx.input(|i| i.time);
    burst_rays(p, c, area, 18, 0.25, k, ((now * 0.254) % std::f64::consts::TAU) as f32, col.gamma_multiply(0.05 * fade));
    // A ring goes out from it, thinning and fading (whole from the start: it doesn't fade in).
    let u = t / 0.93;
    if u < 1.0 {
        let w = 8.5 * (1.0 - u) * k;
        let mid = (75.0 + 275.0 * (1.0 - (1.0 - u).powi(3))) * k;
        // (egui strokes a circle outside its radius.)
        p.circle_stroke(c, mid - w / 2.0, Stroke::new(w, col.gamma_multiply(0.95 * (1.0 - u))));
    }
    // Six faint discs of the tier's color, the crest's whole size from the start; the crest pops
    // in over them, past its size and back.
    glow_discs(p, c, 211.2 * k, col.gamma_multiply(0.05 * fade));
    let size = 211.2 * k * ease_out_back((t - 0.09) / 0.57, 1.2);
    if size > 0.0 {
        crest_tinted(p, ctx, tier, Rect::from_center_size(c, vec2(size, size)), Color32::WHITE.gamma_multiply(fade));
    }
    // "LEVEL 11" pops in after it, white edged in the tier's color.
    let pop = ease_out_back((t - 0.42) / 0.43, 1.2);
    if pop > 0.0 {
        outlined(p, pos2(c.x, c.y + 129.6 * k), &format!("LEVEL {}", b.level), theme::bold_font(59.5 * k * pop), Color32::WHITE.gamma_multiply(fade), col.gamma_multiply(fade));
    }
    // The level's title (its size doesn't grow with the page, its place does).
    p.text(pos2(c.x, c.y + 182.5 * k), Align2::CENTER_CENTER, levels::title(b.level), theme::bold_font(16.0), col.gamma_multiply(fade));
    // The chips it paid, counting up from 0.78 s, over a second.
    if b.chips > 0 && t >= 0.78 {
        let x = (t - 0.78).min(1.0);
        let shown = (b.chips as f64 * (1.0 - (1.0 - x as f64).powi(3))).round() as u64;
        outlined(p, pos2(c.x, c.y + 230.4 * k), &format!("+{} chips", chips_text(shown)), theme::bold_font(36.5 * k), crate::casino::col::GOLD2.gamma_multiply(fade), Color32::from_black_alpha((200.0 * fade) as u8));
    }
}

/// Text with a dark edge round it (big words over a busy game).
fn outlined(p: &egui::Painter, at: Pos2, text: &str, font: egui::FontId, color: Color32, edge: Color32) {
    for (dx, dy) in [(-2.0, 0.0), (2.0, 0.0), (0.0, -2.0), (0.0, 2.0), (-1.5, -1.5), (1.5, 1.5), (-1.5, 1.5), (1.5, -1.5)] {
        p.text(at + vec2(dx, dy), Align2::CENTER_CENTER, text, font.clone(), edge);
    }
    p.text(at, Align2::CENTER_CENTER, text, font, color);
}

fn mission_card(p: &egui::Painter, c: Rect, i: usize, m: Option<&MissionView>) {
    let col = corner_color(i);
    let done = m.is_some_and(|m| m.done);
    if done {
        // Done: the card a quarter of its color (the floor through it), edged in it.
        p.rect_filled(c, theme::r(12), col.gamma_multiply(0.25));
        p.rect_stroke(c, theme::r(12), Stroke::new(1.0f32, col), StrokeKind::Inside);
    } else {
        // To do: a card like the player's.
        p.rect_filled(c, theme::r(12), theme::ELEV());
        p.rect_stroke(c, theme::r(12), Stroke::new(1.0f32, theme::LINE()), StrokeKind::Inside);
    }
    // Its color down the left edge, done or not.
    let edge = Rect::from_min_max(pos2(c.left(), c.top() + 10.0), pos2(c.left() + 4.0, c.bottom() - 10.0));
    p.rect_filled(edge, theme::r(2), col);
    let x = c.left() + 14.0;
    p.text(pos2(x, c.top() + 14.0), Align2::LEFT_CENTER, CORNERS[i], theme::bold_font(10.0), col);
    let Some(m) = m else { return };
    let text = p.layout_no_wrap(m.text.clone(), theme::bold_font(13.5), theme::TEXT());
    p.galley(pos2(x, c.top() + 33.0 - text.size().y / 2.0), text, theme::TEXT());
    let bar = Rect::from_min_size(pos2(x, c.top() + 50.0), vec2(c.width() - 28.0, 7.0));
    p.rect_filled(bar, egui::CornerRadius::same(4), Color32::from_rgb(25, 21, 19));
    let frac = if m.target > 0 { (m.progress as f32 / m.target as f32).clamp(0.0, 1.0) } else { 0.0 };
    if frac > 0.0 {
        p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * frac, bar.height())), egui::CornerRadius::same(4), col);
    }
    let left = if m.done { "Done".to_string() } else { format!("{} / {}", m.progress, m.target) };
    p.text(pos2(x, c.bottom() - 10.0), Align2::LEFT_CENTER, left, theme::font(11.5), if m.done { Color32::WHITE } else { theme::MUTED() });
    p.text(pos2(c.right() - 14.0, c.bottom() - 10.0), Align2::RIGHT_CENTER, format!("+{} XP · +{}", m.xp, chips_text(m.chips)), theme::bold_font(11.5), crate::casino::col::GOLD2);
}

/// One tier on the VIP Club page: its crest, its levels, its perks.
fn tier_card(p: &egui::Painter, ctx: &egui::Context, r: Rect, i: usize, mine: usize) {
    let (first, last) = levels::tier_levels(i);
    let here = i == mine;
    let next = i == mine + 1;
    let reached = i <= mine;
    let tc = tier_color(i);
    // (As 0.13.28's: the tiers reached a little of their color, edged in it, mine edged thicker;
    // the next a plain card edged in its color; the rest plain, their crests dimmed.)
    let (fill, stroke) = if reached {
        (tc.gamma_multiply(0.16), Stroke::new(if here { 2.5f32 } else { 1.0 }, tc))
    } else if next {
        (theme::ELEV(), Stroke::new(1.0f32, tc))
    } else {
        (theme::ELEV(), Stroke::new(1.0f32, theme::LINE()))
    };
    p.rect_filled(r, theme::r(14), fill);
    p.rect_stroke(r, theme::r(14), stroke, StrokeKind::Inside);
    let open = reached || next;
    if next {
        // The next tier's crest glows softly, breathing (0.13.28's: a breath every second and a
        // half or so).
        let t = ctx.input(|i| i.time) as f32;
        crate::casino::radial(p, pos2(r.center().x, r.top() + 56.0), 60.0, tc.gamma_multiply(0.205 + 0.045 * (t * 4.0).sin()), Color32::TRANSPARENT);
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
    let tint = if open { Color32::WHITE } else { Color32::from_gray(89) };
    if let Some(t) = pics::tex(ctx, "tier-crests", 1920) {
        let fi = i as f32;
        p.image(t.id(), Rect::from_center_size(pos2(r.center().x, r.top() + 56.0), vec2(84.0, 84.0)), Rect::from_min_max(pos2(fi / 10.0, 0.0), pos2((fi + 1.0) / 10.0, 1.0)), tint);
    }
    let name_col = if open { tc } else { theme::MUTED() };
    p.text(pos2(r.center().x, r.top() + 112.0), Align2::CENTER_CENTER, levels::TIERS[i].0, theme::bold_font(17.0), name_col);
    let (label, label_col) = if here {
        ("YOU ARE HERE".to_string(), Color32::WHITE)
    } else if next {
        ("NEXT".to_string(), theme::DIM())
    } else if first == last {
        (format!("LEVEL {first}"), theme::DIM())
    } else {
        (format!("LEVELS {first}–{last}"), theme::DIM())
    };
    p.text(pos2(r.center().x, r.top() + 132.0), Align2::CENTER_CENTER, label, theme::bold_font(10.0), label_col);
    let mut perks = vec![format!("Ember Back {} of the house's edge", percent(levels::ember_bp(i))), format!("Daily chips ×{:.2}", levels::daily_x100(i) as f64 / 100.0)];
    if i >= 1 {
        perks.push("Weekly bonus: +20% of your Ember Back".into());
        let felt = matches!(i, 3 | 5 | 8);
        perks.push(format!("The {} frame{}", levels::TIERS[i].0, if felt { " and felt" } else { "" }));
        perks.push("A 100,000-chip chest on arrival".into());
    }
    let text_col = if reached { theme::TEXT() } else { theme::MUTED() };
    let mut y = r.top() + 148.0;
    for perk in perks {
        let g = p.layout(format!("• {perk}"), theme::font(11.5), text_col, r.width() - 28.0);
        let h = g.size().y;
        p.galley(pos2(r.left() + 14.0, y), g, text_col);
        y += h + 4.0;
    }
}
