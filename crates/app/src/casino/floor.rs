//! The casino floor: the house's games. The Phoenix Jackpot climbing, wins as they happen, then
//! a card for every game, by kind: the slots (Pyr Sevens and Reel Row's twelve machines), the
//! table games, and Pyr's own. (Poker, Flash Pots, Pyr Friday Night and tournaments are under
//! Poker.) The art is Meshy's (`casino::pics`).

use eframe::egui::{self, Align2, Color32, Rect, RichText, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::state::ServerView;
use pyr_poker::casino::{Game, reels as rr};

use super::reel_themes::THEMES;
use super::{col, glow_text, hgrad, pics, radial, vgrad_round_fn};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

/// "3d 4h", "2h 05m", "12m": how long until.
pub fn until(ms: u64) -> String {
    let now = pyr_proto::now_ms().max(0) as u64;
    let left = ms.saturating_sub(now) / 1000;
    let (d, h, m) = (left / 86_400, left % 86_400 / 3600, left % 3600 / 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{m}m")
    }
}

/// What a card on the floor opens.
#[derive(Clone, Copy)]
enum Opens {
    Game(Game),
    Machine(u8),
}

/// A game's card: its picture (cut to a portrait about `focus`, a fraction across), its name,
/// its badges, what it pays back, and what it is (on hovering).
struct Card {
    opens: Opens,
    name: &'static str,
    art: String,
    focus: f32,
    badges: Vec<(&'static str, Color32)>,
    foot: String,
    pitch: String,
}

const WHEEL: Color32 = Color32::from_rgb(0x8a, 0x5a, 0xff);
const FUNNY: Color32 = Color32::from_rgb(0xff, 0x6a, 0xa8);
const NEW: Color32 = Color32::from_rgb(0x3a, 0xd0, 0x8a);

/// "RTP 99.00%": what a game pays back (a slot's by design; the others' with the best bets).
fn rtp_line(key: &str, g: Game) -> String {
    let bp = pyr_poker::casino::slot_rtp_bp(key).unwrap_or(10_000u32.saturating_sub(g.house_edge_bp() as u32));
    format!("RTP {}", super::info::percent_bp(bp))
}

fn game_card(g: Game, art: &str, focus: f32, badges: Vec<(&'static str, Color32)>) -> Card {
    Card { opens: Opens::Game(g), name: g.name(), art: art.into(), focus, badges, foot: rtp_line(g.id(), g), pitch: g.pitch().into() }
}

impl PyrApp {
    pub fn casino_floor(&mut self, ui: &mut egui::Ui, s: &ServerView) {
        self.casino_open(s);
        let w = (ui.available_width() - 56.0).max(320.0);
        // The Bonfire, biggest of all (on a server that has it).
        self.bonfire_keep(s);
        if s.poker.bonfire.is_some() {
            self.bonfire_hero(ui, s, w);
            ui.add_space(14.0);
        }
        self.jackpot_hero(ui, s, w);
        ui.add_space(12.0);
        self.live_wins(ui, s, w);
        ui.add_space(24.0);
        // The slots: Pyr Sevens, then Reel Row's machines (on a server that has them).
        let reel_row = s.has(pyr_proto::control::CASINO_012_FEATURE);
        let mut slots = vec![game_card(Game::Slots, "tile-sevens", 0.5, vec![("JACKPOT", col::EMBER)])];
        if reel_row {
            slots.push(game_card(Game::Dragon, "tile-dragon", 0.5, vec![("JACKPOTS", col::EMBER), ("NEW", NEW)]));
        }
        if reel_row {
            for (m, th) in THEMES.iter().enumerate() {
                let mut badges = Vec::new();
                if rr::bonus_kind(m as u8) == rr::BonusKind::Wheel {
                    badges.push(("WHEEL", WHEEL));
                }
                if th.humor.is_some() {
                    badges.push(("FUNNY", FUNNY));
                }
                if badges.is_empty() {
                    badges.push(("NEW", NEW));
                }
                let key = pyr_poker::casino::reels_key(m as u8);
                slots.push(Card {
                    opens: Opens::Machine(m as u8),
                    name: th.name,
                    art: format!("reels-{}-tile", th.id),
                    focus: if th.id == "goldrush" { 0.3 } else { 0.5 },
                    badges,
                    foot: rtp_line(&key, Game::Reels),
                    pitch: format!("{} 243 ways, {}.", th.tagline, if rr::bonus_kind(m as u8) == rr::BonusKind::Wheel { "and a prize wheel" } else { "and a pick-and-dodge bonus" }),
                });
            }
        }
        let note = (!reel_row).then_some("Twelve more machines come when this server's Pyr is updated to 0.12.");
        self.floor_section(ui, s, w, "Slots", &format!("{} game{} · every one pays back about 99%", slots.len(), if slots.len() == 1 { "" } else { "s" }), &slots, note);
        // (On a 0.12.0 server: tables with seats for people and bots.)
        let seats = |n: &'static str| if reel_row { vec![(n, col::GOLD_DARK), ("NEW", NEW)] } else { vec![] };
        let tables = vec![
            game_card(Game::Blackjack, "tile-blackjack", 0.38, if reel_row { vec![("7 SEATS", col::GOLD_DARK)] } else { vec![] }),
            game_card(Game::Roulette, "tile-roulette", 0.5, vec![]),
            game_card(Game::Baccarat, "tile-baccarat", 0.5, seats("5 SEATS")),
            game_card(Game::Craps, "tile-craps", 0.5, seats("5 SEATS")),
        ];
        self.floor_section(ui, s, w, "Table games", "Dealt and rolled by the house, checked by your Pyr", &tables, None);
        let originals = vec![game_card(Game::Dice, "tile-dice", 0.5, vec![]), game_card(Game::Keno, "tile-keno", 0.5, vec![])];
        self.floor_section(ui, s, w, "Pyr Originals", "Quick games: pick your odds", &originals, None);
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            ui.label(RichText::new("Play chips only: they can't be bought or cashed out. Every round is checked by your own Pyr.").color(theme::DIM()).size(12.5));
            if ui.link(RichText::new("How?").size(12.5)).clicked() {
                self.poker.casino.fairness_open = true;
            }
        });
    }

    /// The Phoenix Jackpot: the phoenix rising on the right, the pool climbing on the left.
    fn jackpot_hero(&mut self, ui: &mut egui::Ui, s: &ServerView, w: f32) {
        let c = &s.poker.casino;
        let t = ui.ctx().input(|i| i.time) as f32;
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            let h = (w * 0.24).clamp(184.0, 236.0);
            let (r, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
            let p = ui.painter().clone();
            let base = Color32::from_rgb(0x16, 0x07, 0x06);
            p.rect_filled(r.translate(vec2(0.0, 10.0)), theme::r(22), Color32::from_black_alpha(120));
            p.rect_filled(r, theme::r(20), base);
            let ph = Rect::from_min_max(pos2(r.right() - r.width() * 0.5, r.top()), r.max);
            pics::cover_rounded(&p, &ctx, "tile-phoenix", ph, 20.0, (0.5, 0.45), 1024, Color32::WHITE);
            // (The phoenix's edge melts into the dark.)
            hgrad(&p, Rect::from_min_max(ph.min, pos2(ph.left() + ph.width() * 0.45, ph.bottom())), base, base.gamma_multiply(0.0));
            radial(&p, pos2(r.left() + w * 0.18, r.center().y + 20.0), w * 0.32, col::EMBER.gamma_multiply(0.20 + 0.04 * (t * 1.7).sin()), Color32::TRANSPARENT);
            // Sparks rising.
            let clip = p.with_clip_rect(r.shrink(2.0));
            for i in 0..34 {
                let fi = i as f32;
                let speed = 0.08 + (fi * 0.37).fract() * 0.12;
                let k = (t * speed + (fi * 0.618).fract()) % 1.0;
                let x = r.left() + ((fi * 0.4142).fract() * 0.98 + 0.01) * r.width() + (t * 0.9 + fi).sin() * 8.0;
                let y = r.bottom() - k * (r.height() + 10.0);
                let a = (1.0 - k) * (k / 0.1).min(1.0);
                clip.circle_filled(pos2(x, y), 1.2 + (fi * 0.73).fract() * 1.8, col::GOLD2.gamma_multiply(0.75 * a));
            }
            let x = r.left() + 30.0;
            p.text(pos2(x, r.top() + 34.0), Align2::LEFT_CENTER, "PHOENIX JACKPOT", theme::bold_font(15.0), col::GOLD);
            let shown = self.poker.casino.slots.shown_jackpot(c.jackpot);
            let size = (h * 0.27).clamp(44.0, 62.0);
            glow_text(&p, pos2(x - 2.0, r.top() + 34.0 + size * 0.95), Align2::LEFT_CENTER, &chips_text(shown), theme::bold_font(size), col::GOLD2, col::EMBER.gamma_multiply(0.5));
            let g = p.layout("Five wild flames on a line of Pyr Sevens win it all. It grows with every spin.".into(), theme::font(13.5), col::TEXT, (w * 0.46).max(220.0));
            let gy = r.top() + 34.0 + size * 1.6;
            p.galley(pos2(x, gy), g.clone(), col::TEXT);
            let b = Rect::from_min_size(pos2(x, (gy + g.size().y + 14.0).min(r.bottom() - 54.0)), vec2(168.0, 42.0));
            let hover = resp.hovered();
            p.rect_filled(b.translate(vec2(0.0, 3.0)), theme::r(21), Color32::from_black_alpha(120));
            vgrad_round_fn(&p, b, 21.0, |k| super::mix(if hover { col::GOLD2 } else { col::GOLD }, col::GOLD_DARK, k * 0.6));
            p.text(b.center(), Align2::CENTER_CENTER, "Spin for it  ›", theme::bold_font(16.0), Color32::from_rgb(0x2a, 0x10, 0x04));
            p.rect_stroke(r, theme::r(20), Stroke::new(if hover { 2.0 } else { 1.5f32 }, if hover { col::GOLD } else { col::GOLD_DARK }), StrokeKind::Inside);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.poker.casino.game = Some(Game::Slots);
            }
        });
    }

    /// Wins as they happen, scrolling by (smoothly: see `text_exact`).
    fn live_wins(&mut self, ui: &mut egui::Ui, s: &ServerView, w: f32) {
        let c = &s.poker.casino;
        let time = ui.ctx().input(|i| i.time);
        let t = time as f32;
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 44.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(r, theme::r(22), Color32::from_rgba_unmultiplied(0x0e, 0x0b, 0x12, 225));
            p.rect_stroke(r, theme::r(22), Stroke::new(1.0f32, col::GOLD_DARK.gamma_multiply(0.8)), StrokeKind::Inside);
            let tag = Rect::from_min_size(r.left_top() + vec2(6.0, 6.0), vec2(106.0, 32.0));
            p.rect_filled(tag, theme::r(16), Color32::from_rgb(0x3a, 0x0e, 0x0e));
            let pulse = 0.55 + 0.45 * (t * 3.0).sin().abs();
            p.circle_filled(tag.left_center() + vec2(16.0, 0.0), 4.5, col::LOSE.gamma_multiply(pulse));
            p.text(tag.left_center() + vec2(28.0, 0.0), Align2::LEFT_CENTER, "LIVE WINS", theme::bold_font(12.0), col::TEXT);
            let line = Rect::from_min_max(pos2(tag.right() + 12.0, r.top()), pos2(r.right() - 14.0, r.bottom()));
            let clip = p.with_clip_rect(line);
            if c.big_wins.is_empty() {
                clip.text(line.left_center(), Align2::LEFT_CENTER, "Nobody's hit big yet. Be the first on the board.", theme::font(13.0), col::MUTED);
            } else {
                let items: Vec<String> = c.big_wins.iter().take(12).map(|b| format!("{} won {} on {} ({})", b.name, chips_text(b.paid), b.game.name(), b.what)).collect();
                // (A star after each, the last too: where the line comes round again looks like
                // anywhere else.)
                const SEP: &str = "      ★      ";
                let text = items.join(SEP) + SEP;
                let galley = clip.layout_no_wrap(text, theme::bold_font(13.5), col::GOLD2);
                let span = galley.size().x.max(1.0);
                // Where it's got to, worked out in full (a long-open Pyr's clock is too big for
                // an f32 to keep the fractions of a pixel), and as many times round as it takes
                // to fill the line. Its height on a whole pixel: only across does it move.
                let x = line.left() - ((time * 55.0) % span as f64) as f32;
                let ppp = ui.ctx().pixels_per_point();
                let y = ((line.center().y - galley.size().y / 2.0) * ppp).round() / ppp;
                let times = (line.width() / span).ceil() as usize + 1;
                for k in 0..times {
                    super::text_exact(&clip, pos2(x + k as f32 * span, y), &galley, col::GOLD2);
                }
                // Every frame (in step with the screen), not every so often.
                ui.ctx().request_repaint();
            }
        });
    }

    /// A row of the floor: its heading, then its games' cards (as many across as fit).
    #[allow(clippy::too_many_arguments)]
    fn floor_section(&mut self, ui: &mut egui::Ui, s: &ServerView, w: f32, title: &str, sub: &str, cards: &[Card], note: Option<&str>) {
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 34.0), Sense::hover());
            let p = ui.painter();
            let g = p.layout_no_wrap(title.to_string(), theme::bold_font(21.0), col::GOLD2);
            let tw = g.size().x;
            p.galley(pos2(r.left(), r.center().y - g.size().y / 2.0), g, col::GOLD2);
            p.text(pos2(r.left() + tw + 14.0, r.center().y + 2.0), Align2::LEFT_CENTER, sub, theme::font(13.0), col::MUTED);
        });
        ui.add_space(10.0);
        let gap = 14.0;
        let cols = (((w + gap) / (178.0 + gap)).floor() as usize).clamp(2, 8);
        let cw = (w - gap * (cols - 1) as f32) / cols as f32;
        let ch = cw * 4.0 / 3.0;
        for row in cards.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(28.0);
                for (k, card) in row.iter().enumerate() {
                    if k > 0 {
                        ui.add_space(gap);
                    }
                    self.floor_card(ui, s, card, vec2(cw, ch));
                }
            });
            ui.add_space(gap);
        }
        if let Some(n) = note {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                ui.label(RichText::new(n).color(col::MUTED).size(12.5));
            });
            ui.add_space(8.0);
        }
        ui.add_space(14.0);
    }

    fn floor_card(&mut self, ui: &mut egui::Ui, s: &ServerView, card: &Card, size: egui::Vec2) {
        let ctx = ui.ctx().clone();
        let (r, resp) = ui.allocate_exact_size(size, Sense::click());
        let hover = resp.hovered();
        // (It lifts as it's hovered.)
        let lift = ctx.animate_bool_with_time(resp.id.with("lift"), hover, 0.14);
        let r = r.translate(vec2(0.0, -5.0 * lift));
        let p = ui.painter().clone();
        p.rect_filled(r.translate(vec2(0.0, 8.0 + 4.0 * lift)), theme::r(18), Color32::from_black_alpha((110.0 + 50.0 * lift) as u8));
        if lift > 0.01 {
            radial(&p, r.center(), r.width() * 0.9, col::GOLD.gamma_multiply(0.10 * lift), Color32::TRANSPARENT);
        }
        let tint = Color32::from_gray((215.0 + 40.0 * lift) as u8);
        if !pics::cover_rounded(&p, &ctx, &card.art, r, 16.0, (card.focus, 0.5), (r.width() * 2.0) as u32, tint) {
            p.rect_filled(r, theme::r(16), Color32::from_rgb(0x1e, 0x1a, 0x28));
        }
        // Dark at the foot, for its name.
        vgrad_round_fn(&p, r, 16.0, |k| Color32::from_black_alpha((((k - 0.45) / 0.55).clamp(0.0, 1.0).powf(1.2) * 235.0) as u8));
        let name = p.layout(card.name.to_string(), theme::bold_font((r.width() * 0.095).clamp(14.0, 18.0)), Color32::WHITE, r.width() - 24.0);
        let ny = r.bottom() - 34.0 - name.size().y;
        p.galley(pos2(r.left() + 12.0, ny), name, Color32::WHITE);
        p.text(pos2(r.left() + 12.0, r.bottom() - 20.0), Align2::LEFT_CENTER, &card.foot, theme::bold_font(11.5), col::GOLD.gamma_multiply(0.95));
        // Its best for me, if it's been played.
        let key = match card.opens {
            Opens::Game(g) => g.id().to_string(),
            Opens::Machine(m) => pyr_poker::casino::reels_key(m),
        };
        if let Some(st) = s.poker.casino.stats.get(&key).filter(|st| st.best_win > 0) {
            p.text(pos2(r.right() - 12.0, r.bottom() - 20.0), Align2::RIGHT_CENTER, format!("best +{}", crate::poker::cards::chips_short(st.best_win)), theme::font(11.0), col::WIN);
        }
        // Badges.
        let mut bx = r.left() + 10.0;
        for (b, color) in &card.badges {
            let g = p.layout_no_wrap(b.to_string(), theme::bold_font(10.5), Color32::WHITE);
            let br = Rect::from_min_size(pos2(bx, r.top() + 10.0), vec2(g.size().x + 14.0, 20.0));
            p.rect_filled(br, theme::r(10), *color);
            p.galley(br.center() - g.size() / 2.0, g, Color32::WHITE);
            bx = br.right() + 6.0;
        }
        // Hovered: play.
        if lift > 0.01 {
            let c = r.center() - vec2(0.0, r.height() * 0.08);
            let rad = 26.0 * (0.7 + 0.3 * lift);
            p.circle_filled(c, rad + 6.0, Color32::from_black_alpha((90.0 * lift) as u8));
            p.circle_filled(c, rad, col::GOLD.gamma_multiply(lift));
            let k = rad * 0.42;
            p.add(Shape::convex_polygon(vec![c + vec2(-k * 0.7, -k), c + vec2(k * 1.1, 0.0), c + vec2(-k * 0.7, k)], Color32::from_rgb(0x2a, 0x10, 0x04).gamma_multiply(lift), Stroke::NONE));
        }
        p.rect_stroke(r, theme::r(16), Stroke::new(1.0 + lift, super::mix(Color32::from_white_alpha(28), col::GOLD, lift)), StrokeKind::Inside);
        if resp.on_hover_text(&card.pitch).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            match card.opens {
                Opens::Game(g) => self.poker.casino.game = Some(g),
                Opens::Machine(m) => self.reels_open(m),
            }
        }
    }
}
