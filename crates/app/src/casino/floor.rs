//! The casino floor: the Phoenix Jackpot climbing, big wins as they happen, Pyr Friday Night
//! counting down, and a tile for every game.

use eframe::egui::{self, Align2, Color32, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::state::ServerView;
use pyr_poker::casino::Game;

use super::{art, col, glow_text, radial, vgrad_round};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

/// The floor's tiles: poker, then the games against the house.
const TILES: [(&str, &str, &str); 9] = [
    ("poker", "Poker", "Hold'em and Omaha against friends and bots, tables and tournaments."),
    ("timebomb", "Time Bomb", "Three seats, one fuse, up to 1,000× the buy-in. When it blows, everyone's all in."),
    ("slots", "Pyr Sevens", "Five reels, twenty lines, wild flames, free spins and the Phoenix Jackpot."),
    ("blackjack", "Blackjack", "Beat the dealer to 21. Blackjack pays 3 to 2."),
    ("roulette", "Roulette", "Single zero. Red or black, or straight up at 35 to 1."),
    ("baccarat", "Baccarat", "Player, Banker or Tie: the high rollers' favorite."),
    ("craps", "Craps", "Pass line, odds, place bets and hardways."),
    ("dice", "Dice", "Pick your chance. 2% pays 49.5 times."),
    ("keno", "Keno", "Ten for ten pays 100,000 to 1."),
];

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

impl PyrApp {
    pub fn casino_floor(&mut self, ui: &mut egui::Ui, s: &ServerView) {
        self.casino_open(s);
        let c = s.poker.casino.clone();
        let t = ui.ctx().input(|i| i.time) as f32;
        let w = (ui.available_width() - 56.0).max(320.0);
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            // The Phoenix Jackpot.
            let (r, resp) = ui.allocate_exact_size(vec2(w, 132.0), Sense::click());
            let p = ui.painter();
            vgrad_round(p, r, 16.0, Color32::from_rgb(0x2a, 0x0c, 0x08), Color32::from_rgb(0x10, 0x06, 0x0a));
            radial(p, r.left_center() + vec2(r.width() * 0.3, 10.0), r.width() * 0.45, col::EMBER.gamma_multiply(0.28 + 0.05 * (t * 2.0).sin()), Color32::TRANSPARENT);
            p.rect_stroke(r, theme::r(16), Stroke::new(1.5f32, col::GOLD_DARK), StrokeKind::Inside);
            let shown = self.poker.casino.slots.shown_jackpot(c.jackpot);
            let flame = Rect::from_center_size(r.left_center() + vec2(64.0, 0.0), vec2(92.0, 92.0));
            art::paint_symbol(p, flame, pyr_poker::casino::slots::WILD, Color32::WHITE);
            p.text(pos2(flame.right() + 18.0, r.top() + 30.0), Align2::LEFT_CENTER, "PHOENIX JACKPOT", theme::bold_font(15.0), col::GOLD);
            glow_text(p, pos2(flame.right() + 16.0, r.top() + 74.0), Align2::LEFT_CENTER, &chips_text(shown), theme::bold_font(44.0), col::GOLD2, col::EMBER.gamma_multiply(0.45));
            p.text(pos2(flame.right() + 18.0, r.bottom() - 18.0), Align2::LEFT_CENTER, "Five wild flames on a line of Pyr Sevens wins it. It grows with every spin.", theme::font(12.5), col::MUTED);
            if r.width() > 760.0 {
                let b = Rect::from_center_size(r.right_center() - vec2(110.0, 0.0), vec2(170.0, 48.0));
                p.rect_filled(b, theme::r(24), if resp.hovered() { col::GOLD2 } else { col::GOLD });
                p.text(b.center(), Align2::CENTER_CENTER, "Spin for it", theme::bold_font(17.0), Color32::from_rgb(0x2a, 0x10, 0x04));
            }
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.poker.casino.game = Some(Game::Slots);
            }
        });
        ui.add_space(10.0);
        // Big wins as they happen, and Pyr Friday Night.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.add_space(28.0);
            let fw = (w * 0.38).clamp(260.0, 420.0);
            let (r, resp) = ui.allocate_exact_size(vec2(fw, 76.0), Sense::click());
            let p = ui.painter();
            vgrad_round(p, r, 12.0, Color32::from_rgb(0x14, 0x1c, 0x3a), Color32::from_rgb(0x0a, 0x0e, 0x22));
            p.rect_stroke(r, theme::r(12), Stroke::new(1.0f32, Color32::from_rgb(0x4a, 0x6a, 0xc8)), StrokeKind::Inside);
            p.text(r.left_top() + vec2(14.0, 12.0), Align2::LEFT_TOP, "PYR FRIDAY NIGHT", theme::bold_font(13.0), Color32::from_rgb(0x9f, 0xc0, 0xff));
            p.text(r.left_top() + vec2(14.0, 32.0), Align2::LEFT_TOP, format!("{} chips to win · free to enter", chips_text(c.friday_prize)), theme::bold_font(15.0), Color32::WHITE);
            p.text(r.left_top() + vec2(14.0, 54.0), Align2::LEFT_TOP, if c.friday_ms > 0 { format!("Fridays 10 PM Eastern · starts in {}", until(c.friday_ms)) } else { "Fridays 10 PM Eastern".into() }, theme::font(12.0), col::MUTED);
            if resp.on_hover_text("A free tournament every Friday night. Its prize money is the week's rake from the cash tables and the casino. Register under Tournaments.").clicked() {
                self.poker.tab = crate::poker::Tab::Tourneys;
            }
            ui.add_space(10.0);
            let tw = (w - fw - 10.0).max(200.0);
            let (r, _) = ui.allocate_exact_size(vec2(tw, 76.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(r, theme::r(12), Color32::from_rgb(0x12, 0x10, 0x18));
            p.rect_stroke(r, theme::r(12), Stroke::new(1.0f32, col::GOLD_DARK), StrokeKind::Inside);
            p.text(r.left_top() + vec2(14.0, 12.0), Align2::LEFT_TOP, "BIG WINS", theme::bold_font(13.0), col::GOLD);
            let line = Rect::from_min_max(r.left_top() + vec2(10.0, 34.0), r.right_bottom() - vec2(10.0, 8.0));
            let clip = p.with_clip_rect(line);
            if c.big_wins.is_empty() {
                clip.text(line.left_center() + vec2(4.0, 0.0), Align2::LEFT_CENTER, "Nobody's hit big yet. Be the first on the board.", theme::font(13.0), col::MUTED);
            } else {
                let items: Vec<String> = c.big_wins.iter().take(12).map(|b| format!("{} won {} on {} ({})", b.name, chips_text(b.paid), b.game.name(), b.what)).collect();
                let text = items.join("      ★      ");
                let galley = clip.layout_no_wrap(text.clone(), theme::bold_font(13.5), col::GOLD2);
                let span = galley.size().x + 80.0;
                let x = line.left() - ((t * 55.0) % span);
                for k in 0..3 {
                    clip.galley(pos2(x + k as f32 * span, line.center().y - galley.size().y / 2.0), galley.clone(), col::GOLD2);
                }
            }
        });
        ui.add_space(16.0);
        // The games.
        // (Nine tiles: three rows of three when there's room, else two or one across.)
        let cols = if w >= 822.0 {
            3
        } else if w >= 548.0 {
            2
        } else {
            1
        };
        let tw = (w - (cols - 1) as f32 * 14.0) / cols as f32;
        for row in TILES.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(28.0);
                for (k, (id, name, pitch)) in row.iter().enumerate() {
                    if k > 0 {
                        ui.add_space(14.0);
                    }
                    let (r, resp) = ui.allocate_exact_size(vec2(tw, 222.0), Sense::click());
                    let p = ui.painter();
                    let hover = resp.hovered();
                    let lift = if hover { -3.0 } else { 0.0 };
                    let r = r.translate(vec2(0.0, lift));
                    p.rect_filled(r.translate(vec2(0.0, 6.0)), theme::r(16), Color32::from_black_alpha(90));
                    vgrad_round(p, r, 16.0, Color32::from_rgb(0x1e, 0x1a, 0x28), Color32::from_rgb(0x12, 0x10, 0x18));
                    let art_r = Rect::from_min_size(r.min, vec2(r.width(), 128.0));
                    let felt = if *id == "roulette" || *id == "craps" || *id == "blackjack" || *id == "baccarat" || *id == "poker" {
                        (col::FELT, col::FELT_DARK)
                    } else if *id == "timebomb" {
                        (Color32::from_rgb(0x5a, 0x1a, 0x10), Color32::from_rgb(0x1a, 0x08, 0x08))
                    } else {
                        (Color32::from_rgb(0x3a, 0x12, 0x18), Color32::from_rgb(0x16, 0x08, 0x0c))
                    };
                    super::oval(p, art_r.expand2(vec2(art_r.width() * 0.1, 20.0)).intersect(art_r), felt.0, felt.1);
                    art::tile(&p.with_clip_rect(art_r), art_r.shrink(8.0), id, t);
                    p.rect_stroke(r, theme::r(16), Stroke::new(if hover { 2.0 } else { 1.0 }, if hover { col::GOLD } else { col::GOLD_DARK.gamma_multiply(0.7) }), StrokeKind::Inside);
                    p.text(pos2(r.left() + 16.0, art_r.bottom() + 20.0), Align2::LEFT_CENTER, *name, theme::bold_font(18.0), col::TEXT);
                    let galley = p.layout(pitch.to_string(), theme::font(12.5), col::MUTED, r.width() - 32.0);
                    p.galley(pos2(r.left() + 16.0, art_r.bottom() + 34.0), galley, col::MUTED);
                    if *id == "timebomb" {
                        let going = s.poker.tourneys.as_ref().map(|l| l.iter().filter(|x| x.rules.bomb.is_some() && x.phase == pyr_poker::net::TourneyPhase::Running).count()).unwrap_or(0);
                        let foot = if going > 0 { format!("{going} going off now") } else { "Buy-ins 100 to 10,000".to_string() };
                        p.text(pos2(r.left() + 16.0, r.bottom() - 16.0), Align2::LEFT_CENTER, foot, theme::font(11.5), theme::DIM());
                    } else if let Some(g) = Game::from_id(id) {
                        let st = c.stats.get(g.id());
                        let foot = match st {
                            Some(st) if st.best_win > 0 => format!("Your best: +{}", chips_text(st.best_win)),
                            _ => format!("House edge {}.{:02}%", g.house_edge_bp() / 100, g.house_edge_bp() % 100),
                        };
                        p.text(pos2(r.left() + 16.0, r.bottom() - 16.0), Align2::LEFT_CENTER, foot, theme::font(11.5), theme::DIM());
                    } else {
                        let tables = s.poker.lobby.as_ref().map(|l| l.len()).unwrap_or(0);
                        p.text(pos2(r.left() + 16.0, r.bottom() - 16.0), Align2::LEFT_CENTER, format!("{tables} table{} open", if tables == 1 { "" } else { "s" }), theme::font(11.5), theme::DIM());
                    }
                    if hover {
                        let b = Rect::from_min_size(pos2(r.right() - 78.0, r.bottom() - 30.0), vec2(64.0, 24.0));
                        p.rect_filled(b, theme::r(12), col::GOLD);
                        p.text(b.center(), Align2::CENTER_CENTER, "Play ›", theme::bold_font(12.5), Color32::from_rgb(0x2a, 0x10, 0x04));
                    }
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        match Game::from_id(id) {
                            Some(g) => self.poker.casino.game = Some(g),
                            None if *id == "timebomb" => self.poker.tab = crate::poker::Tab::Tourneys,
                            None => self.poker.tab = crate::poker::Tab::Tables,
                        }
                    }
                }
            });
            ui.add_space(14.0);
        }
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            ui.label(RichText::new("Play chips only: they can't be bought or cashed out. Every round is checked by your own Pyr.").color(theme::DIM()).size(12.5));
            if ui.link(RichText::new("How?").size(12.5)).clicked() {
                self.poker.casino.fairness_open = true;
            }
        });
    }
}
