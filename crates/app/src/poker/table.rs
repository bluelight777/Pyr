//! The table: the felt, the seats round it, the cards and chips moving, the clock, and the
//! buttons for my turn.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Vec2, pos2, vec2};
use pyr_engine::poker::{TableLive, checks_summary};
use pyr_engine::state::{AppState, ServerView};
use pyr_poker::bots::{self, Level};
use pyr_poker::cards::Card;
use pyr_poker::eval;
use pyr_poker::game::{Action, Event, Variant};
use pyr_poker::net::{SeatView, ToServer};

use super::cards::{self, chips_short, chips_text};
use super::{PreAction, arc};
use crate::app::PyrApp;
use crate::theme;

const TAU: f32 = std::f32::consts::TAU;

/// The felt's two colors (center, edge) for each look.
pub fn felt_colors(id: &str) -> (Color32, Color32) {
    super::felt::cloth(id)
}

pub fn frame_color(id: &str) -> Option<Color32> {
    match id {
        "frame_bronze" => Some(Color32::from_rgb(0xc0, 0x7a, 0x3e)),
        "frame_silver" => Some(Color32::from_rgb(0xd6, 0xdc, 0xe4)),
        "frame_gold" => Some(Color32::from_rgb(0xff, 0xcf, 0x4a)),
        "frame_shark" => Some(Color32::from_rgb(0x4f, 0xc8, 0xff)),
        _ => None,
    }
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_unmultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Where the felt, seats and pot sit.
struct Layout {
    c: Pos2,
    a: f32,
    b: f32,
    n: usize,
    /// The seat drawn at the bottom (mine, when I'm sitting).
    bottom: usize,
}

impl Layout {
    fn seat(&self, seat: usize) -> Pos2 {
        let rel = (seat + self.n - self.bottom) % self.n;
        let t = std::f32::consts::FRAC_PI_2 + rel as f32 / self.n as f32 * TAU;
        super::felt::on_oval(self.c, self.a + 16.0, self.b + 14.0, t)
    }

    /// A point `k` of the way from a seat to the middle.
    fn toward(&self, seat: usize, k: f32) -> Pos2 {
        let s = self.seat(seat);
        s + (self.c - s) * k
    }

    fn dealer(&self) -> Pos2 {
        self.c + vec2(0.0, -self.b * 0.62)
    }

    fn pot(&self) -> Pos2 {
        self.c + vec2(0.0, -self.b * 0.38)
    }
}

/// How long ago, in seconds.
fn ago(t: Instant) -> f32 {
    t.elapsed().as_secs_f32()
}

impl PyrApp {
    pub fn poker_table(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, t: &TableLive) {
        let _ = st;
        let v = &t.view;
        let me = s.poker.me.clone();
        ui.ctx().request_repaint_after(Duration::from_millis(33));
        let full = ui.max_rect();
        let p = ui.painter().clone();
        // The bar along the top: the table, what it plays, the checks, and leaving.
        let top = Rect::from_min_size(full.min, vec2(full.width(), 50.0));
        let log_w = if self.poker.log_open { 300.0 } else { 0.0 };
        let bar_h = 124.0;
        let area = Rect::from_min_max(pos2(full.left(), top.bottom()), pos2(full.right() - log_w, full.bottom() - bar_h));
        // As big as fits, leaving room for the seats round it.
        let b = ((area.height() / 2.0 - 78.0) / 1.2).max(90.0);
        let a = (area.width() * 0.37).min(b / 0.47).max(180.0);
        let b = b.min(a * 0.5);
        let lay = Layout { c: area.center() + vec2(0.0, -4.0), a, b, n: v.seats.len(), bottom: v.my_seat.unwrap_or(0) };
        let felt_id = me.as_ref().map(|m| m.felt.clone()).unwrap_or_default();
        let (lamp, _) = super::felt::cloth(&felt_id);
        super::felt::room(&p, full, lay.c, mix(Color32::from_rgb(0x0b, 0x0a, 0x0c), lamp, 0.22));
        self.table_top_bar(ui, s, t, top);
        if self.poker.cloth.as_ref().is_none_or(|(id, _)| *id != felt_id) {
            self.poker.cloth = Some((felt_id.clone(), super::felt::cloth_texture(ui.ctx(), &felt_id)));
        }
        if let Some((_, tex)) = &self.poker.cloth {
            super::felt::table(&p, lay.c, lay.a, lay.b, tex, "PYR CASINO", &format!("{} · {}", s.name, v.rules.name()));
        }
        self.board_and_pot(&p, &lay, t);
        // The seats.
        for i in 0..v.seats.len() {
            self.seat(ui, s, t, &lay, i);
        }
        // The dealer button.
        if let Some(bt) = v.button
            && v.seats.get(bt).is_some_and(|x| x.is_some())
        {
            let s0 = lay.seat(bt);
            let dir = (lay.c - s0).normalized();
            let side = vec2(-dir.y, dir.x);
            let at = s0 + dir * (lay.b * 0.42) + side * 46.0;
            p.circle_filled(at + vec2(0.0, 2.0), 13.0, Color32::from_black_alpha(90));
            p.circle_filled(at, 13.0, Color32::from_rgb(0xf6, 0xf3, 0xec));
            p.circle_stroke(at, 10.0, Stroke::new(1.0f32, Color32::from_rgb(0xc8, 0xc0, 0xb0)));
            p.text(at, Align2::CENTER_CENTER, "D", theme::bold_font(13.0), Color32::from_rgb(0x22, 0x1c, 0x18));
        }
        self.winners(&p, &lay, t);
        // My cards, big, and how my hand stands.
        self.my_hand(ui, &p, &lay, t);
        // The bottom: my turn's buttons, or what to do when it comes.
        let bar = Rect::from_min_max(pos2(full.left(), full.bottom() - bar_h), pos2(full.right() - log_w, full.bottom()));
        self.action_bar(ui, s, t, bar);
        if self.poker.log_open {
            self.hand_log(ui, t, Rect::from_min_max(pos2(full.right() - log_w, top.bottom()), full.max));
        }
        self.emotes(&p, &lay, t);
        // A Time Bomb: its wheel, fuse and bang over the felt.
        if let Some(tid) = v.tourney
            && s.poker.tourney.get(&tid).is_some_and(|tv| tv.bomb.is_some())
        {
            self.bomb_overlay(ui, s, tid, lay.c, lay.a, lay.b);
        }
        self.table_sounds(&lay, t);
        self.sit_dialogs(ui.ctx(), s, t);
        self.history_window(ui.ctx(), s);
        self.player_window(ui.ctx(), s, t);
        self.prize_dialog(ui.ctx(), s, t.view.id);
        if let Some((text, at)) = self.poker.banner.clone() {
            let k = ago(at);
            if k < 3.0 {
                let alpha = (1.0 - ((k - 2.4) / 0.6).max(0.0)).clamp(0.0, 1.0);
                let r = Rect::from_center_size(lay.c + vec2(0.0, -lay.b * 0.9 - 10.0 * ease(k * 2.0)), vec2(260.0, 52.0));
                p.rect_filled(r, theme::r(26), theme::EMBER().gamma_multiply(alpha));
                p.text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(22.0), theme::on_ember().gamma_multiply(alpha));
            } else {
                self.poker.banner = None;
            }
        }
    }

    /// Sounds for what just happened at the table.
    fn table_sounds(&mut self, lay: &Layout, t: &TableLive) {
        use pyr_engine::poker_sfx::Cue;
        let v = &t.view;
        let n = t.events.len();
        let from = match self.poker.heard {
            Some((id, h, k)) if id == v.id && h == v.hand => k.min(n),
            // Just opened (or a new hand): only what comes from now on... unless it's a new hand.
            Some((id, _, _)) if id == v.id => 0,
            _ => n,
        };
        self.poker.heard = Some((v.id, v.hand, n));
        let me = v.my_seat;
        let pan = |seat: usize| ((lay.seat(seat).x - lay.c.x) / lay.a * 0.7).clamp(-0.8, 0.8);
        let mut cues: Vec<(Cue, f32, f32)> = Vec::new();
        for (_, e) in &t.events[from..] {
            match e {
                Event::DealtFaceDown { .. } => cues.push((Cue::Deal, 0.0, 0.8)),
                Event::Street { .. } => cues.push((Cue::Flip, 0.0, 0.9)),
                Event::Posted { seat, .. } => cues.push((Cue::Chips, pan(*seat), 0.45)),
                Event::Acted { seat, action, stack, .. } => cues.push(match action {
                    Action::Fold => (Cue::Fold, pan(*seat), 0.7),
                    Action::Check => (Cue::Check, pan(*seat), 0.8),
                    _ if *stack == 0 => (Cue::AllIn, pan(*seat), 0.9),
                    _ => (Cue::Chips, pan(*seat), 0.8),
                }),
                Event::Shown { seat, .. } => cues.push((Cue::Flip, pan(*seat), 0.6)),
                Event::Turn { seat, .. } if Some(*seat) == me => cues.push((Cue::Turn, 0.0, 0.8)),
                Event::Won { seats, .. } => {
                    if me.is_some_and(|m| seats.contains(&m)) {
                        cues.push((Cue::Win, 0.0, 0.9));
                    } else {
                        cues.push((Cue::Pot, seats.first().map(|s| pan(*s)).unwrap_or(0.0), 0.7));
                    }
                }
                _ => {}
            }
        }
        // My clock running low: once a turn.
        if let (Some(m), Some(h), Some((cs, deadline, _, _))) = (me, v.hand, t.clock)
            && cs == m
            && v.to_act == Some(m)
            && deadline.saturating_duration_since(Instant::now()) < Duration::from_secs(5)
            && self.poker.hurried != Some((h, t.events.len()))
        {
            self.poker.hurried = Some((h, t.events.len()));
            cues.push((Cue::Hurry, 0.0, 0.8));
        }
        // Anyone's clock: a tick for each of their last five seconds, a call when their time
        // bank starts, and a knock when they're out of time altogether.
        if let (Some(h), Some((cs, deadline, _, bank))) = (v.hand, t.clock)
            && v.to_act == Some(cs)
        {
            let n = t.events.len();
            let mine = me == Some(cs);
            let vol = if mine { 0.8 } else { 0.4 };
            let left = deadline.saturating_duration_since(Instant::now()).as_secs_f32();
            let last = self.poker.clock_heard.filter(|c| c.0 == h && c.1 == n);
            if t.in_bank && !last.is_some_and(|c| c.3) {
                cues.push((Cue::TimeBank, pan(cs), vol));
                self.poker.clock_heard = Some((h, n, 99, true));
            } else if left <= 0.0 {
                // (A bank still to come: the bank's call says so instead.)
                let out = t.in_bank || bank == 0;
                if out && !last.is_some_and(|c| c.2 == 0) && Instant::now().saturating_duration_since(deadline) < Duration::from_secs(2) {
                    cues.push((Cue::TimeOut, pan(cs), vol.max(0.6)));
                    self.poker.clock_heard = Some((h, n, 0, t.in_bank));
                }
            } else if left < 5.0 {
                let sec = left.ceil() as u64;
                if !last.is_some_and(|c| c.2 <= sec) {
                    if !(mine && sec == 5) {
                        cues.push((Cue::Tick, pan(cs), vol));
                    }
                    self.poker.clock_heard = Some((h, n, sec, t.in_bank));
                }
            }
        }
        if self.poker.mute {
            return;
        }
        // A burst (catching up) plays only its last few.
        cues.dedup_by_key(|c| c.0);
        for (c, pan, vol) in cues.into_iter().rev().take(3) {
            self.engine.play_poker(c, pan, vol);
        }
    }

    fn table_top_bar(&mut self, ui: &mut egui::Ui, s: &ServerView, t: &TableLive, r: Rect) {
        let v = &t.view;
        let p = ui.painter().clone();
        p.rect_filled(r, 0, Color32::from_black_alpha(90));
        let back = Rect::from_min_size(r.left_top() + vec2(12.0, 9.0), vec2(96.0, 32.0));
        let back_label = if v.tourney.is_some() { "‹ Tournament" } else { "‹ Lobby" };
        let back = if v.tourney.is_some() { Rect::from_min_size(back.min, vec2(122.0, 32.0)) } else { back };
        if ui.put(back, egui::Button::new(RichText::new(back_label).font(theme::bold_font(13.5))).corner_radius(theme::r(8))).clicked() {
            // (At my own tournament table I stay seated: it's only out of sight.)
            if v.my_seat.is_none() {
                self.engine.poker(&s.key, ToServer::Unwatch { table: v.id });
            }
            self.poker.table = None;
            self.poker.lobby_asked = None;
            if let Some(tid) = v.tourney {
                self.open_tourney(&s.key, tid);
            }
        }
        let name_r = p.text(pos2(back.right() + 16.0, r.center().y - 8.0), Align2::LEFT_CENTER, &v.name, theme::bold_font(15.5), theme::TEXT());
        let hand = v.hand.map(|h| format!(" · hand #{h}")).unwrap_or_default();
        let sub = match v.tourney.and_then(|tid| s.poker.tourney.get(&tid)) {
            Some(tv) => {
                let b = tv.rules.levels[tv.level.min(tv.rules.levels.len() - 1)];
                let next = tv.next_level_ms.map(|ms| format!(" · next {}", super::tourneys::until(ms))).unwrap_or_default();
                let left = tv.entrants.iter().filter(|e| e.place.is_none()).count();
                format!("Level {} · {}/{}{} · {left} left{next}", tv.level + 1, chips_short(b.small), chips_short(b.big), if b.ante > 0 { format!(" ante {}", chips_short(b.ante)) } else { String::new() })
            }
            None => format!("{}{hand}", v.rules.name()),
        };
        let sub_r = p.text(pos2(back.right() + 16.0, r.center().y + 10.0), Align2::LEFT_CENTER, sub, theme::font(12.0), theme::MUTED());
        let words_end = name_r.right().max(sub_r.right()) + 16.0;
        // On the right: log, history, sit out, add chips, leave.
        let mut x = r.right() - 12.0;
        let mut button = |ui: &mut egui::Ui, text: &str, w: f32| -> egui::Response {
            let b = Rect::from_min_size(pos2(x - w, r.top() + 9.0), vec2(w, 32.0));
            x -= w + 8.0;
            ui.put(b, egui::Button::new(RichText::new(text).font(theme::font(13.0))).corner_radius(theme::r(8)))
        };
        let fs = self.poker.casino.fullscreen;
        if button(ui, if fs { "Exit full" } else { "Fullscreen" }, 92.0).on_hover_text(if fs { "Leave fullscreen (Esc)" } else { "Just the table, the whole screen (F11)" }).clicked() || ui.ctx().input(|i| i.key_pressed(egui::Key::F11)) {
            self.poker.casino.fullscreen = !fs;
        }
        if button(ui, if self.poker.mute { "Sound off" } else { "Sound on" }, 92.0).on_hover_text("The table's sounds: cards, chips, your turn").clicked() {
            self.poker.mute = !self.poker.mute;
        }
        if v.tourney.is_none()
            && s.poker.me.as_ref().is_some_and(|m| m.role.is_some())
            && v.prize.is_none()
            && button(ui, "Prize box", 92.0).on_hover_text("Drop a prize box: whoever wins the next hand opens it").clicked()
        {
            self.poker.prize_for = Some(v.id);
            self.engine.poker(&s.key, ToServer::Staff);
        }
        if button(ui, if self.poker.log_open { "Hide log" } else { "Hand log" }, 92.0).clicked() {
            self.poker.log_open = !self.poker.log_open;
        }
        if let Some(&last) = v.recent.last()
            && button(ui, "Last hand", 92.0).clicked()
        {
            self.engine.poker(&s.key, ToServer::History { table: v.id, hand: last });
        }
        if let Some(seat) = v.my_seat {
            let out = v.seats[seat].as_ref().is_some_and(|x| x.sitting_out);
            let tourney = v.tourney.is_some();
            if !tourney && button(ui, "Leave seat", 100.0).clicked() {
                self.engine.poker(&s.key, ToServer::Stand { table: v.id });
            }
            let away = if tourney { "Away" } else { "Sit out" };
            if button(ui, if out { "I'm back" } else { away }, 86.0)
                .on_hover_text(if tourney { "Folded for (and blinded) until you're back" } else { "Not dealt in until you're back" })
                .clicked()
            {
                self.engine.poker(&s.key, ToServer::SitOut { table: v.id, out: !out });
            }
            let in_hand = v.seats[seat].as_ref().is_some_and(|x| x.in_hand) && v.hand.is_some() && v.to_act.is_some();
            if !tourney && !in_hand && button(ui, "Add chips", 92.0).clicked() {
                self.poker.top_up = Some((v.id, v.rules.big_blind * 50));
            }
        }
        // The shuffle checks.
        let (fair, problems) = checks_summary(t);
        let (label, col, tip) = if let Some((h, why)) = problems.last() {
            ("⚠ Shuffle check failed".to_string(), theme::LIVE(), format!("Hand #{h}: {why}. Your Pyr checks every shuffle it's dealt into; this one didn't add up."))
        } else if fair > 0 {
            (
                format!("✓ {fair} shuffle{} checked", if fair == 1 { "" } else { "s" }),
                theme::ONLINE(),
                "Your Pyr added its own random seed to every hand you were dealt, and after each hand checked the dealer's seed against the fingerprint it showed before dealing. Every card matched: nobody could pick the cards, the server included.".to_string(),
            )
        } else {
            ("Shuffles are checked".to_string(), theme::MUTED(), "Sit down: your Pyr adds to each shuffle and checks it when the hand is over.".to_string())
        };
        // Between the table's name and the buttons: in full if it fits, else just the count.
        let full_w = label.chars().count() as f32 * 7.6;
        let label = if x - 14.0 - full_w > words_end {
            label
        } else if fair > 0 && problems.is_empty() {
            format!("✓ {fair}")
        } else if !problems.is_empty() {
            "⚠".to_string()
        } else {
            String::new()
        };
        if !label.is_empty() && x - 14.0 - label.chars().count() as f32 * 7.6 > words_end {
            let at = pos2((r.center().x).min(x - 14.0 - label.chars().count() as f32 * 3.8).max(words_end + label.chars().count() as f32 * 3.8), r.center().y);
            let tr = p.text(at, Align2::CENTER_CENTER, label, theme::bold_font(13.0), col);
            ui.interact(tr.expand(4.0), ui.id().with("checks"), Sense::hover()).on_hover_text(tip);
        }
    }

    fn board_and_pot(&self, p: &egui::Painter, lay: &Layout, t: &TableLive) {
        let v = &t.view;
        let four = self.poker.four_color;
        // Big enough to read from across a room, or with tired eyes.
        let cw = (lay.a * 0.21).clamp(60.0, 124.0);
        let ch = cw * 1.4;
        let gap = cw * 0.14;
        let total = 5.0 * cw + 4.0 * gap;
        let left = lay.c.x - total / 2.0;
        let y = lay.c.y - ch * 0.42;
        // Empty places for the five.
        for i in 0..5 {
            let r = Rect::from_min_size(pos2(left + i as f32 * (cw + gap), y), vec2(cw, ch));
            p.rect_stroke(r, theme::r(6), Stroke::new(1.0f32, Color32::from_white_alpha(22)), StrokeKind::Inside);
        }
        // When each board card came (for the flip).
        let mut came: Vec<Instant> = Vec::new();
        for (at, e) in &t.events {
            if let Event::Street { cards, .. } = e {
                for k in 0..cards.len() {
                    came.push(*at + Duration::from_millis(130 * k as u64));
                }
            }
        }
        // The winning five, to light up at the end.
        let best: Vec<Card> = t.events.iter().rev().find_map(|(_, e)| if let Event::Won { best, .. } = e { Some(best.clone()) } else { None }).unwrap_or_default();
        for (i, c) in v.board.iter().enumerate() {
            let r = Rect::from_min_size(pos2(left + i as f32 * (cw + gap), y), vec2(cw, ch));
            let k = came.get(i).map(|at| if Instant::now() < *at { 0.0 } else { at.elapsed().as_secs_f32() / 0.28 }).unwrap_or(1.0);
            flip(p, r, *c, k, four, !best.is_empty() && !best.contains(c), "");
        }
        // A prize box, for this hand's winner.
        if let Some(prize) = &v.prize {
            super::staff::paint_prize(p, lay.pot() + vec2(-220.0, -14.0), prize, p.ctx().input(|i| i.time) as f32);
        }
        // The pot: chips and how much (with side pots once there's more than one).
        let in_front: u64 = v.seats.iter().flatten().map(|s| s.bet).sum();
        let middle = v.pot.saturating_sub(in_front);
        if middle > 0 && t.ended_at.is_none() {
            let at = lay.pot();
            cards::pile(p, at + vec2(-40.0, 8.0), middle, 9.0, "");
            let label = Rect::from_center_size(at + vec2(36.0, 0.0), vec2(150.0, 32.0));
            p.rect_filled(label, theme::r(16), Color32::from_black_alpha(140));
            p.text(label.center(), Align2::CENTER_CENTER, format!("Pot {}", chips_text(v.pot)), theme::bold_font(17.0), Color32::from_rgb(0xff, 0xe2, 0x9a));
        }
    }

    fn seat(&mut self, ui: &mut egui::Ui, s: &ServerView, t: &TableLive, lay: &Layout, i: usize) {
        let v = &t.view;
        let p = ui.painter().clone();
        let at = lay.seat(i);
        let Some(seat) = &v.seats[i] else {
            // An empty seat: sit here, or (whoever made the table) a bot.
            let r = Rect::from_center_size(at, vec2(64.0, 64.0));
            let resp = ui.interact(r, ui.id().with(("empty", i)), Sense::click());
            let col = if resp.hovered() && v.tourney.is_none() { theme::TEXT() } else { theme::DIM() };
            p.circle_filled(at, 30.0, Color32::from_black_alpha(80));
            dashed_circle(&p, at, 30.0, Stroke::new(1.5f32, col));
            if v.tourney.is_some() {
                // A tournament seats people itself.
                return;
            }
            if v.my_seat.is_none() {
                p.text(at, Align2::CENTER_CENTER, "Sit", theme::bold_font(14.0), col);
                if resp.on_hover_text("Sit here").clicked() {
                    let chips = (v.rules.big_blind * 100).clamp(v.rules.big_blind * v.buy_in.min_bb, v.rules.big_blind * v.buy_in.max_bb);
                    self.poker.buy_in = Some((v.id, i, chips));
                }
            } else {
                crate::icons::plus(&p, Rect::from_center_size(at, vec2(16.0, 16.0)), col, 1.6);
                let mine = s.me.as_ref().is_some_and(|m| m.user_id == v.owner);
                let resp = resp.on_hover_text(if mine { "Add a bot here (right-click to pick how good)" } else { "An empty seat" });
                if mine {
                    if resp.clicked() {
                        self.engine.poker(&s.key, ToServer::AddBot { table: v.id, seat: i, level: Level::Regular });
                    }
                    resp.context_menu(|ui| {
                        for l in [Level::Rookie, Level::Regular, Level::Shark] {
                            if ui.button(format!("Add a {} bot", l.name())).clicked() {
                                self.engine.poker(&s.key, ToServer::AddBot { table: v.id, seat: i, level: l });
                                ui.close();
                            }
                        }
                    });
                }
            }
            return;
        };
        let out = seat.folded || seat.sitting_out || (!seat.in_hand && v.hand.is_some());
        let fade: f32 = if out { 0.45 } else { 1.0 };
        // (Big enough to see who's who across the table.)
        let r0 = 44.0;
        // Which way the middle is (cards and bets go that way).
        let side = if (lay.c.x - at.x).abs() < 40.0 { 1.0 } else { (lay.c.x - at.x).signum() };
        // The clock round whoever's turn it is, and a glow.
        let turn = v.to_act == Some(i);
        if turn {
            for k in 0..6 {
                p.circle_filled(at, r0 + 6.0 + k as f32 * 3.0, theme::GLOW().gamma_multiply(0.05));
            }
        }
        // The picture: theirs (friends' real faces), or a bot's.
        p.circle_filled(at + vec2(0.0, 3.0), r0 + 4.0, Color32::from_black_alpha(120));
        let ring = frame_color(&seat.frame).unwrap_or(Color32::from_rgb(0x2a, 0x26, 0x2c));
        p.circle_filled(at, r0 + 3.5, ring.gamma_multiply(fade.max(0.6)));
        match seat.bot {
            Some(level) => bot_face(&p, at, r0, &seat.name, level),
            None => {
                let face = self.face(ui.ctx(), s, &seat.player);
                crate::app::avatar(&p, at, r0, &seat.player, &seat.name, false, theme::SIDE(), face);
            }
        }
        if out {
            p.circle_filled(at, r0, Color32::from_black_alpha(140));
        }
        if let Some((cs, deadline, total, _)) = t.clock
            && cs == i
            && turn
        {
            let left = deadline.saturating_duration_since(Instant::now()).as_secs_f32();
            let frac = (left / total.as_secs_f32()).clamp(0.0, 1.0);
            let col = if frac > 0.5 { Color32::from_rgb(0x5f, 0xd8, 0x7d) } else if frac > 0.2 { Color32::from_rgb(0xf5, 0xc4, 0x51) } else { Color32::from_rgb(0xef, 0x4b, 0x4b) };
            p.circle_stroke(at, r0 + 3.0, Stroke::new(5.0f32, Color32::from_black_alpha(160)));
            arc(&p, at, r0 + 3.0, frac, Stroke::new(5.0f32, col));
            // The last ten seconds, counted down big on their picture; and the time bank,
            // named above it.
            if left < 10.0 || t.in_bank {
                let pulse = 0.85 + 0.15 * (left.fract() * std::f32::consts::TAU).cos();
                p.circle_filled(at, r0 - 4.0, Color32::from_black_alpha(170));
                p.text(at, Align2::CENTER_CENTER, format!("{}", left.ceil() as u64), theme::bold_font(34.0 * pulse.max(0.9)), col);
            }
            if t.in_bank {
                let r = Rect::from_center_size(at + vec2(0.0, -r0 - 16.0), vec2(104.0, 24.0));
                p.rect_filled(r, theme::r(12), Color32::from_rgb(0x2a, 0x5a, 0xc8));
                p.text(r.center(), Align2::CENTER_CENTER, "TIME BANK", theme::bold_font(12.5), Color32::WHITE);
            }
        } else if turn {
            p.circle_stroke(at, r0 + 3.0, Stroke::new(2.5f32, theme::GLOW()));
        }
        // Their cards: face down beside the picture, or shown over it at the showdown.
        if seat.in_hand && !seat.folded && Some(i) != v.my_seat {
            let k = v.rules.hole_cards();
            let dealt = deal_progress(t, i);
            let shown = !seat.cards.is_empty();
            let cw = if shown { if k > 2 { 40.0 } else { 48.0 } } else { 32.0 };
            let ch = cw * 1.4;
            let spread = if shown { cw * 0.78 } else { cw * 0.42 };
            let base = if shown { at + vec2(0.0, -14.0) } else { at + vec2(side * (r0 + 6.0), -10.0) };
            for c in 0..k {
                let off = (c as f32 - (k as f32 - 1.0) / 2.0) * spread;
                let target = Rect::from_center_size(base + vec2(off, if shown { 0.0 } else { (c as f32 - (k as f32 - 1.0) / 2.0).abs() * 2.0 }), vec2(cw, ch));
                let prog = dealt.get(c).copied().unwrap_or(1.0);
                if prog <= 0.0 {
                    continue;
                }
                let r = target.translate((lay.dealer() - target.center()) * (1.0 - ease(prog)));
                match seat.cards.get(c) {
                    Some(card) => flip(&p, r, *card, shown_progress(t, i), self.poker.four_color, false, &seat.card_back),
                    None => cards::back(&p, r, &seat.card_back),
                }
            }
        }
        // Name and chips on a plate under the picture.
        let plate = Rect::from_center_size(at + vec2(0.0, r0 + 16.0), vec2(164.0, 50.0));
        p.rect_filled(plate.translate(vec2(0.0, 3.0)), theme::r(25), Color32::from_black_alpha(110));
        p.rect_filled(plate, theme::r(25), Color32::from_rgba_unmultiplied(0x12, 0x11, 0x16, 238));
        p.rect_stroke(plate, theme::r(25), Stroke::new(1.0f32, if turn { theme::GLOW() } else { Color32::from_white_alpha(22) }), StrokeKind::Inside);
        p.line_segment([plate.left_top() + vec2(18.0, 1.0), plate.right_top() + vec2(-18.0, 1.0)], Stroke::new(1.0f32, Color32::from_white_alpha(28)));
        let name: String = seat.name.chars().take(15).collect();
        p.text(plate.center() + vec2(0.0, -10.0), Align2::CENTER_CENTER, name, theme::bold_font(15.5), Color32::from_rgb(0xf2, 0xee, 0xe8).gamma_multiply(fade.max(0.7)));
        let (stack, stack_col) = if seat.sitting_out {
            ("Sitting out".to_string(), theme::MUTED())
        } else if seat.all_in && seat.stack == 0 {
            ("ALL IN".to_string(), Color32::from_rgb(0xff, 0x7a, 0x4a))
        } else {
            (chips_text(seat.stack), Color32::from_rgb(0xff, 0xd2, 0x5e))
        };
        p.text(plate.center() + vec2(0.0, 11.0), Align2::CENTER_CENTER, stack, theme::bold_font(15.5), stack_col.gamma_multiply(fade.max(0.7)));
        // Level, on the picture's edge; and for bots, how good they are.
        let lv = at + vec2(-r0 * 0.8, -r0 * 0.6);
        p.circle_filled(lv, 11.5, Color32::from_rgb(0x16, 0x14, 0x1a));
        p.circle_stroke(lv, 11.5, Stroke::new(1.5f32, Color32::from_rgb(0xff, 0xc4, 0x4a).gamma_multiply(fade.max(0.6))));
        p.text(lv, Align2::CENTER_CENTER, seat.level.to_string(), theme::bold_font(10.5), Color32::WHITE.gamma_multiply(fade.max(0.6)));
        if let Some(level) = seat.bot {
            let (label, col) = level_tag(level);
            let tag = Rect::from_center_size(plate.center_top() + vec2(0.0, -1.0), vec2(8.0 + 6.6 * label.len() as f32, 15.0));
            p.rect_filled(tag, theme::r(8), col.gamma_multiply(fade.max(0.6)));
            p.text(tag.center(), Align2::CENTER_CENTER, label, theme::bold_font(9.5), Color32::WHITE);
        }
        // Their bet in front of them.
        if seat.bet > 0 {
            let b = lay.toward(i, 0.36) + vec2(side * 0.0, 0.0);
            cards::pile(&p, b + vec2(-14.0, 6.0), seat.bet, 8.0, "");
            let lr = Rect::from_min_size(b + vec2(4.0, -9.0), vec2(14.0 + 7.4 * chips_short(seat.bet).len() as f32, 18.0));
            p.rect_filled(lr, theme::r(9), Color32::from_black_alpha(120));
            p.text(lr.center(), Align2::CENTER_CENTER, chips_short(seat.bet), theme::bold_font(12.0), Color32::from_rgb(0xff, 0xe9, 0xb0));
        }
        // What they just did.
        if let Some((text, col, age)) = last_action(t, i)
            && age < 2.2
        {
            let alpha = (1.0 - (age - 1.6).max(0.0) / 0.6).clamp(0.0, 1.0);
            let pop = 0.85 + 0.15 * ease(age * 6.0);
            let r = Rect::from_center_size(at + vec2(0.0, -r0 - 16.0 - 6.0 * ease(age * 4.0)), vec2((16.0 + 7.8 * text.len() as f32) * pop, 24.0 * pop));
            p.rect_filled(r.translate(vec2(0.0, 2.0)), theme::r(12), Color32::from_black_alpha((100.0 * alpha) as u8));
            p.rect_filled(r, theme::r(12), col.gamma_multiply(alpha));
            p.text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(12.5), Color32::WHITE.gamma_multiply(alpha));
        }
        // The hand they showed.
        if let Some(name) = t.events.iter().rev().find_map(|(_, e)| if let Event::Shown { seat: si, hand, .. } = e { (*si == i && !hand.is_empty()).then(|| hand.clone()) } else { None }) {
            let r = Rect::from_center_size(plate.center_bottom() + vec2(0.0, 12.0), vec2(14.0 + 6.6 * name.len() as f32, 20.0));
            p.rect_filled(r, theme::r(10), Color32::from_rgba_unmultiplied(0x2a, 0x1e, 0x08, 235));
            p.rect_stroke(r, theme::r(10), Stroke::new(1.0f32, Color32::from_rgb(0xc8, 0x9a, 0x3a)), StrokeKind::Inside);
            p.text(r.center(), Align2::CENTER_CENTER, name, theme::bold_font(11.5), Color32::from_rgb(0xff, 0xe2, 0x9a));
        }
        // Click a person: their profile card (as anywhere in Pyr).
        let hit = ui.interact(Rect::from_center_size(at, vec2(2.0 * r0, 2.0 * r0)), ui.id().with(("seat", i)), Sense::click());
        let hit = if seat.bot.is_some() {
            hit.on_hover_text(format!("{} bot · plays fair: it sees only its own cards\nRight-click: its hands and history", seat.bot.map(|l| l.name()).unwrap_or("")))
        } else {
            hit.on_hover_text("Right-click: their hands and history")
        };
        // Right-click anyone (a bot too): hands played, won, and their last hands.
        if hit.secondary_clicked() {
            self.engine.poker(&s.key, ToServer::Player { table: v.id, seat: i });
        }
    }

    /// Chips flying to the winners, their glow and what they won.
    fn winners(&self, p: &egui::Painter, lay: &Layout, t: &TableLive) {
        for (at, e) in &t.events {
            let Event::Won { seats, amounts, hand, .. } = e else { continue };
            let k = ago(*at);
            if k > 4.0 {
                continue;
            }
            for (s, amt) in seats.iter().zip(amounts) {
                let to = lay.seat(*s);
                let fly = ease(k / 0.7);
                let pos = lay.pot() + (to - lay.pot()) * fly;
                if k < 0.9 {
                    cards::pile(p, pos, *amt, 8.0, "");
                }
                let glow = (1.0 - (k - 3.0).max(0.0)).clamp(0.0, 1.0);
                p.circle_stroke(to, 38.0, Stroke::new(3.0f32, Color32::from_rgb(0xff, 0xd2, 0x4a).gamma_multiply(glow)));
                let rise = to + vec2(0.0, -58.0 - 22.0 * ease(k / 1.5));
                p.text(rise, Align2::CENTER_CENTER, format!("+{}", chips_text(*amt)), theme::bold_font(18.0), Color32::from_rgb(0xff, 0xd2, 0x4a).gamma_multiply(glow));
            }
            if let Some(h) = hand
                && k < 3.5
            {
                let alpha = (1.0 - (k - 2.8).max(0.0) / 0.7).clamp(0.0, 1.0);
                let r = Rect::from_center_size(lay.c + vec2(0.0, lay.b * 0.42), vec2(30.0 + 10.0 * h.len() as f32, 36.0));
                p.rect_filled(r, theme::r(18), Color32::from_rgba_unmultiplied(0x1a, 0x12, 0x0c, (220.0 * alpha) as u8));
                p.rect_stroke(r, theme::r(18), Stroke::new(1.5f32, Color32::from_rgb(0xff, 0xd2, 0x4a).gamma_multiply(alpha)), StrokeKind::Inside);
                p.text(r.center(), Align2::CENTER_CENTER, h, theme::bold_font(17.0), Color32::from_rgb(0xff, 0xe2, 0x9a).gamma_multiply(alpha));
            }
        }
    }

    fn my_hand(&mut self, ui: &mut egui::Ui, p: &egui::Painter, lay: &Layout, t: &TableLive) {
        let v = &t.view;
        let Some(me) = v.my_seat else { return };
        let Some(seat) = &v.seats[me] else { return };
        if seat.cards.is_empty() || !seat.in_hand {
            return;
        }
        let k = seat.cards.len();
        let cw = if k > 2 { 80.0 } else { 96.0 };
        let ch = cw * 1.4;
        // Beside my picture, leaning on the rail, big enough to read at a glance.
        let base = lay.seat(me) + vec2(54.0 + cw * 0.5 + cw * 0.72 * (k as f32 - 1.0) / 2.0, -44.0);
        let dealt = deal_progress(t, me);
        // Streamer mode: face down until I peek (Space held, or the pointer on them), so a stream
        // never shows my cards.
        let span = Rect::from_center_size(base, vec2(cw * (1.0 + 0.72 * (k as f32 - 1.0)) + 12.0, ch + 12.0));
        let streamer = self.streamer_mode();
        let peek = !streamer || ui.ctx().input(|i| i.key_down(egui::Key::Space)) && !ui.ctx().wants_keyboard_input() || ui.rect_contains_pointer(span);
        for (c, card) in seat.cards.iter().enumerate() {
            let off = (c as f32 - (k as f32 - 1.0) / 2.0) * cw * 0.72;
            let tilt = (c as f32 - (k as f32 - 1.0) / 2.0) * 4.0;
            let target = Rect::from_center_size(base + vec2(off, tilt.abs()), vec2(cw, ch));
            let prog = dealt.get(c).copied().unwrap_or(1.0);
            if prog <= 0.0 {
                continue;
            }
            let r = target.translate((lay.dealer() - target.center()) * (1.0 - ease(prog)));
            // Turned over as it lands.
            if peek {
                flip(p, r, *card, ((prog - 0.6) / 0.4).clamp(0.0, 1.0), self.poker.four_color, seat.folded, &seat.card_back);
            } else {
                cards::back(p, r, &seat.card_back);
            }
        }
        if !peek {
            // (Under my cards, clear of my name plate, like the hand's words.)
            let cards_left = base.x - cw * 0.5 - cw * 0.72 * (k as f32 - 1.0) / 2.0;
            let left = cards_left.max(lay.seat(me).x + 90.0);
            let r = Rect::from_min_size(pos2(left, base.y + ch * 0.5 + 10.0), vec2(250.0, 28.0));
            p.rect_filled(r, theme::r(14), Color32::from_rgba_unmultiplied(0x14, 0x10, 0x0a, 225));
            p.text(r.center(), Align2::CENTER_CENTER, "Streamer mode · hold Space to peek", theme::bold_font(12.5), Color32::from_rgb(0xff, 0xe2, 0x9a));
            return;
        }
        // What I have now, and (if asked for) my chance of winning.
        if seat.folded {
            return;
        }
        let what = describe_mine(v.rules.variant, &seat.cards, &v.board);
        // Who I'm up against: everyone still in the hand.
        let live = v.seats.iter().flatten().filter(|s| s.in_hand && !s.folded).count().saturating_sub(1);
        let eq = if self.poker.hide_odds { None } else { self.chance(v.rules.variant, &seat.cards, &v.board, live, ui.ctx()) };
        // Under my cards, clear of the board and of my name plate.
        let cards_left = base.x - cw * 0.5 - cw * 0.72 * (k as f32 - 1.0) / 2.0;
        let left = cards_left.max(lay.seat(me).x + 90.0);
        let top = base.y + ch * 0.5 + 10.0;
        let Some(eq) = eq else {
            // Just my hand in words.
            let r = Rect::from_min_size(pos2(left, top), vec2(22.0 + 8.6 * what.chars().count() as f32, 30.0));
            p.rect_filled(r, theme::r(15), Color32::from_rgba_unmultiplied(0x14, 0x10, 0x0a, 225));
            p.rect_stroke(r, theme::r(15), Stroke::new(1.0f32, Color32::from_rgb(0x8a, 0x6a, 0x2a)), StrokeKind::Inside);
            p.text(r.center(), Align2::CENTER_CENTER, what, theme::bold_font(15.5), Color32::from_rgb(0xff, 0xe2, 0x9a));
            return;
        };
        meter(p, Rect::from_min_size(pos2(left, top), vec2(280.0, 72.0)), &what, eq, live);
    }

    /// My chance of winning this hand against everyone still in it (dealt out thousands of
    /// times, off the screen's thread so the table never stutters); `None` until it's known.
    fn chance(&mut self, variant: Variant, hole: &[Card], board: &[Card], live: usize, ctx: &egui::Context) -> Option<f64> {
        let key = format!("{hole:?}{board:?}{live}");
        if let Some((k, eq)) = &self.poker.odds
            && *k == key
        {
            return Some(*eq);
        }
        if let Some((k, rx)) = &self.poker.odds_job {
            if *k == key {
                if let Ok(eq) = rx.try_recv() {
                    self.poker.odds = Some((key, eq));
                    self.poker.odds_job = None;
                    return Some(eq);
                }
                // (Still working: the last hand's chance would mislead; the new street's
                // comes in a moment.)
                return self.poker.odds.as_ref().filter(|(k, _)| k.starts_with(&format!("{hole:?}"))).map(|(_, e)| *e);
            }
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let (h, b, c) = (hole.to_vec(), board.to_vec(), ctx.clone());
        let sims = if variant == Variant::Omaha { 3000 } else { 6000 };
        std::thread::spawn(move || {
            let seed: u64 = h.iter().chain(&b).fold(live as u64, |a, c| a.wrapping_mul(53).wrapping_add(c.index() as u64));
            let mut rng = pyr_poker::fair::Stream::new(pyr_poker::fair::seed_from(seed));
            let _ = tx.send(bots::equity(variant, &h, &b, live.max(1), sims, &mut rng));
            c.request_repaint();
        });
        self.poker.odds_job = Some((key, rx));
        self.poker.odds.as_ref().filter(|(k, _)| k.starts_with(&format!("{hole:?}"))).map(|(_, e)| *e)
    }

    fn action_bar(&mut self, ui: &mut egui::Ui, s: &ServerView, t: &TableLive, bar: Rect) {
        let v = &t.view;
        let p = ui.painter().clone();
        let mut m = egui::Mesh::default();
        m.colored_vertex(bar.left_top(), Color32::from_black_alpha(0));
        m.colored_vertex(bar.right_top(), Color32::from_black_alpha(0));
        m.colored_vertex(bar.right_bottom(), Color32::from_black_alpha(200));
        m.colored_vertex(bar.left_bottom(), Color32::from_black_alpha(200));
        m.add_triangle(0, 1, 2);
        m.add_triangle(0, 2, 3);
        p.add(Shape::mesh(m));
        let Some(me) = v.my_seat else {
            let msg = if v.tourney.is_some() { "Watching this tournament table." } else { "Watching. Click an empty seat to sit down." };
            p.text(bar.center(), Align2::CENTER_CENTER, msg, theme::font(14.0), theme::MUTED());
            return;
        };
        let hand = v.hand.unwrap_or(0);
        if self.poker.pre_hand != Some(hand) {
            self.poker.pre = PreAction::None;
            self.poker.pre_hand = Some(hand);
        }
        // Emotes, bottom left (when there's room).
        let mut ex = bar.left() + 16.0;
        if bar.width() > 980.0 {
            for e in ["👍", "😂", "😮", "😎", "🔥", "GG"] {
                let r = Rect::from_min_size(pos2(ex, bar.bottom() - 52.0), vec2(38.0, 36.0));
                ex += 42.0;
                let resp = ui.interact(r, ui.id().with(("emote", e)), Sense::click());
                p.rect_filled(r, theme::r(18), if resp.hovered() { Color32::from_white_alpha(30) } else { Color32::from_white_alpha(12) });
                crate::emoji::paint_text(&p, r.center(), Align2::CENTER_CENTER, e, theme::bold_font(15.0), theme::TEXT(), None);
                if resp.on_hover_text("Send to the table").clicked() {
                    self.engine.poker(&s.key, ToServer::Emote { table: v.id, emote: e.into() });
                }
            }
        }
        let seat = v.seats[me].as_ref();
        if v.bomb {
            // (A Time Bomb gone off: nothing to choose.)
            p.text(pos2(bar.right() - 16.0, bar.center().y), Align2::RIGHT_CENTER, "The bomb's gone off: everyone's all in, every hand", theme::bold_font(15.0), Color32::from_rgb(0xff, 0x8a, 0x6a));
            return;
        }
        let legal = v.legal;
        // Pre-actions do their thing when the turn comes.
        if let Some(l) = legal
            && self.poker.pre != PreAction::None
        {
            let a = match self.poker.pre {
                PreAction::CheckFold => Some(if l.can_check { Action::Check } else { Action::Fold }),
                PreAction::Check => l.can_check.then_some(Action::Check),
                PreAction::CallAny => Some(Action::Call),
                PreAction::None => None,
            };
            self.poker.pre = PreAction::None;
            if let Some(a) = a {
                self.engine.poker(&s.key, ToServer::Act { table: v.id, hand, action: a });
                return;
            }
        }
        let right = bar.right() - 16.0;
        let Some(l) = legal else {
            // Not my turn: tick what to do when it comes.
            let in_hand = seat.is_some_and(|s| s.in_hand && !s.folded) && v.to_act.is_some();
            if in_hand {
                let opts = [(PreAction::CheckFold, "Check / Fold"), (PreAction::Check, "Check"), (PreAction::CallAny, "Call any")];
                let mut x = right - 3.0 * 132.0;
                for (pa, label) in opts {
                    let r = Rect::from_min_size(pos2(x, bar.center().y - 20.0), vec2(124.0, 40.0));
                    x += 132.0;
                    let on = self.poker.pre == pa;
                    let resp = ui.interact(r, ui.id().with(("pre", label)), Sense::click());
                    p.rect_filled(r, theme::r(10), if on { theme::EMBER().gamma_multiply(0.85) } else { theme::ELEV() });
                    p.rect_stroke(r, theme::r(10), Stroke::new(1.0f32, theme::LINE()), StrokeKind::Inside);
                    let box_r = Rect::from_center_size(r.left_center() + vec2(18.0, 0.0), vec2(14.0, 14.0));
                    p.rect_stroke(box_r, theme::r(3), Stroke::new(1.5f32, if on { theme::on_ember() } else { theme::MUTED() }), StrokeKind::Inside);
                    if on {
                        crate::icons::tick(&p, box_r.shrink(2.0), theme::on_ember());
                    }
                    p.text(r.left_center() + vec2(32.0, 0.0), Align2::LEFT_CENTER, label, theme::bold_font(13.0), if on { theme::on_ember() } else { theme::TEXT() });
                    if resp.clicked() {
                        self.poker.pre = if on { PreAction::None } else { pa };
                    }
                }
            } else if seat.is_some_and(|s| s.sitting_out) {
                p.text(pos2(right, bar.center().y), Align2::RIGHT_CENTER, "You're sitting out. Click I'm back to be dealt in.", theme::font(14.0), theme::MUTED());
            } else {
                p.text(pos2(right, bar.center().y), Align2::RIGHT_CENTER, "Waiting for the next hand…", theme::font(14.0), theme::MUTED());
            }
            return;
        };
        // My turn: Fold, Check or Call, and Bet or Raise with its amount.
        let key = (hand, me, l.min_to);
        if self.poker.raise_for != Some(key) {
            self.poker.raise_for = Some(key);
            self.poker.raise = l.min_to;
            self.poker.raise_text = l.min_to.to_string();
        }
        let mut act: Option<Action> = None;
        let h = 54.0;
        let y = bar.bottom() - h - 16.0;
        let btn = |ui: &mut egui::Ui, r: Rect, top: &str, bottom: &str, fill: Color32, id: &str, key: &str| -> bool {
            let resp = ui.interact(r, ui.id().with(("act", id)), Sense::click());
            let f = if resp.hovered() { mix(fill, Color32::WHITE, 0.12) } else { fill };
            let press = if resp.is_pointer_button_down_on() { 2.0 } else { 0.0 };
            let r = r.translate(vec2(0.0, press));
            p.rect_filled(r.translate(vec2(0.0, 4.0 - press)), theme::r(14), mix(fill, Color32::BLACK, 0.5));
            // Lit from the top.
            let mut m = egui::Mesh::default();
            let top_c = mix(f, Color32::WHITE, 0.16);
            let bot_c = mix(f, Color32::BLACK, 0.12);
            let inner = r.shrink2(vec2(0.0, 0.0));
            p.rect_filled(inner, theme::r(14), bot_c);
            let hi = Rect::from_min_max(inner.min + vec2(3.0, 2.0), pos2(inner.max.x - 3.0, inner.center().y));
            m.colored_vertex(hi.left_top(), top_c.gamma_multiply(0.55));
            m.colored_vertex(hi.right_top(), top_c.gamma_multiply(0.55));
            m.colored_vertex(hi.right_bottom(), Color32::TRANSPARENT);
            m.colored_vertex(hi.left_bottom(), Color32::TRANSPARENT);
            m.add_triangle(0, 1, 2);
            m.add_triangle(0, 2, 3);
            p.rect_filled(Rect::from_min_max(inner.min, pos2(inner.max.x, inner.center().y + 6.0)), theme::r(14), f);
            p.add(Shape::mesh(m));
            p.rect_stroke(r, theme::r(14), Stroke::new(1.0f32, mix(f, Color32::WHITE, 0.25)), StrokeKind::Inside);
            if bottom.is_empty() {
                p.text(r.center(), Align2::CENTER_CENTER, top, theme::bold_font(18.0), Color32::WHITE);
            } else {
                p.text(r.center() + vec2(0.0, -9.0), Align2::CENTER_CENTER, top, theme::bold_font(16.0), Color32::WHITE);
                p.text(r.center() + vec2(0.0, 11.0), Align2::CENTER_CENTER, bottom, theme::bold_font(14.0), Color32::from_rgb(0xff, 0xec, 0xc0));
            }
            p.text(r.right_top() + vec2(-8.0, 6.0), Align2::RIGHT_TOP, key, theme::font(10.0), Color32::from_white_alpha(90));
            resp.clicked()
        };
        let (w, fold_w, gap) = (172.0, 132.0, 10.0);
        let span = 2.0 * w + fold_w + 2.0 * gap;
        let left = right - span;
        // Fold, Check or Call, Bet or Raise: left to right.
        let r = Rect::from_min_size(pos2(left, y), vec2(fold_w, h));
        if btn(ui, r, "Fold", "", Color32::from_rgb(0xb0, 0x2e, 0x38), "fold", "F") {
            act = Some(Action::Fold);
        }
        let r = Rect::from_min_size(pos2(left + fold_w + gap, y), vec2(w, h));
        if l.can_check {
            if btn(ui, r, "Check", "", Color32::from_rgb(0x23, 0x8a, 0x58), "check", "C") {
                act = Some(Action::Check);
            }
        } else if btn(ui, r, "Call", &chips_text(l.call), Color32::from_rgb(0x2a, 0x6a, 0xc0), "call", "C") {
            act = Some(Action::Call);
        }
        if l.can_raise {
            let r = Rect::from_min_size(pos2(right - w, y), vec2(w, h));
            let all_in = self.poker.raise >= l.max_to;
            let top = if all_in { "All in" } else if l.is_bet { "Bet" } else { "Raise to" };
            if btn(ui, r, top, &chips_text(self.poker.raise), if all_in { Color32::from_rgb(0xc8, 0x2c, 0x22) } else { Color32::from_rgb(0xdc, 0x6c, 0x1a) }, "raise", "R") {
                act = Some(if all_in { Action::AllIn } else { Action::Raise { to: self.poker.raise } });
            }
        }
        // The amount, over the buttons: presets, a slider and a box.
        if l.can_raise && l.max_to > l.min_to {
            let pot = v.pot;
            let presets: [(&str, u64); 5] = [("Min", l.min_to), ("½ Pot", l.call + pot / 2), ("¾ Pot", l.call + pot * 3 / 4), ("Pot", l.call + pot), ("All in", l.max_to)];
            let in_front = v.seats[me].as_ref().map(|s| s.bet).unwrap_or(0);
            let row_y = y - 40.0;
            let panel = Rect::from_min_max(pos2(left - 8.0, row_y - 8.0), pos2(right + 8.0, y - 8.0));
            p.rect_filled(panel, theme::r(12), Color32::from_rgba_unmultiplied(0x10, 0x0e, 0x12, 210));
            let mut px = left;
            let mut lit = false;
            for (label, amt) in presets {
                let to = if label == "Min" || label == "All in" { amt } else { (in_front + amt).clamp(l.min_to, l.max_to) };
                let r = Rect::from_min_size(pos2(px, row_y), vec2(50.0, 24.0));
                px += 54.0;
                let on = !lit && self.poker.raise == to;
                lit |= on;
                let resp = ui.interact(r, ui.id().with(("preset", label)), Sense::click());
                p.rect_filled(r, theme::r(12), if on { Color32::from_rgb(0xdc, 0x6c, 0x1a) } else if resp.hovered() { Color32::from_white_alpha(34) } else { Color32::from_white_alpha(14) });
                p.text(r.center(), Align2::CENTER_CENTER, label, theme::bold_font(11.0), if on { Color32::WHITE } else { theme::TEXT() });
                if resp.clicked() {
                    self.poker.raise = to;
                    self.poker.raise_text = to.to_string();
                }
            }
            let box_w = 78.0;
            let sr = Rect::from_min_max(pos2(px + 6.0, row_y), pos2(right - box_w - 10.0, row_y + 24.0));
            let mut val = self.poker.raise as f64;
            let resp = ui.put(sr, egui::Slider::new(&mut val, l.min_to as f64..=l.max_to as f64).show_value(false).logarithmic(l.max_to > 20 * l.min_to.max(1)));
            if resp.changed() {
                // Rounded to the small blind, so amounts read nicely.
                let step = v.rules.small_blind.max(1);
                let rounded = ((val as u64).div_ceil(step) * step).clamp(l.min_to, l.max_to);
                self.poker.raise = if val as u64 >= l.max_to { l.max_to } else { rounded };
                self.poker.raise_text = self.poker.raise.to_string();
            }
            let tr = Rect::from_min_size(pos2(right - box_w, row_y - 1.0), vec2(box_w, 26.0));
            let te = ui.put(tr, egui::TextEdit::singleline(&mut self.poker.raise_text).horizontal_align(egui::Align::Center).desired_width(box_w));
            if te.changed()
                && let Ok(n) = self.poker.raise_text.trim().replace(',', "").parse::<u64>()
            {
                self.poker.raise = n.clamp(l.min_to, l.max_to);
            }
        }
        // Keys: F fold, C check or call, R bet or raise (unless typing).
        if !ui.ctx().wants_keyboard_input() {
            ui.input(|i| {
                if i.key_pressed(egui::Key::F) {
                    act = Some(Action::Fold);
                } else if i.key_pressed(egui::Key::C) {
                    act = Some(if l.can_check { Action::Check } else { Action::Call });
                } else if i.key_pressed(egui::Key::R) && l.can_raise {
                    act = Some(if self.poker.raise >= l.max_to { Action::AllIn } else { Action::Raise { to: self.poker.raise } });
                }
            });
        }
        if let Some((cs, deadline, _, bank)) = t.clock
            && cs == me
        {
            let left = deadline.saturating_duration_since(Instant::now()).as_secs();
            let txt = if bank > 0 { format!("{left}s · time bank {}s", bank / 1000) } else { format!("{left}s") };
            let lx = right - span - 24.0;
            p.text(pos2(lx, y + h / 2.0), Align2::RIGHT_CENTER, txt, theme::bold_font(13.0), if left < 6 { Color32::from_rgb(0xff, 0x8a, 0x6a) } else { theme::MUTED() });
            if bank > 0 && left < 6 {
                let r = Rect::from_min_size(pos2(lx - 120.0, y + h / 2.0 - 40.0), vec2(120.0, 26.0));
                if ui.put(r, egui::Button::new(RichText::new("Use time bank").size(12.0))).clicked() {
                    self.engine.poker(&s.key, ToServer::TimeBank { table: v.id });
                }
            }
        }
        if let Some(a) = act {
            // A fold when I could check: checked instead (folding for nothing helps nobody).
            let a = if a == Action::Fold && l.can_check { Action::Check } else { a };
            self.engine.poker(&s.key, ToServer::Act { table: v.id, hand, action: a });
        }
    }

    fn hand_log(&self, ui: &mut egui::Ui, t: &TableLive, r: Rect) {
        let v = &t.view;
        let streamer = self.streamer_mode();
        let p = ui.painter();
        p.rect_filled(r, 0, Color32::from_rgb(0x14, 0x11, 0x10));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r.shrink(12.0)));
        child.label(RichText::new("This hand").font(theme::bold_font(15.0)).color(theme::TEXT()));
        child.add_space(6.0);
        egui::ScrollArea::vertical().stick_to_bottom(true).show(&mut child, |ui| {
            for (_, e) in &t.events {
                if let Some(line) = describe_event(e, &v.seats, streamer) {
                    ui.label(RichText::new(line).size(12.5).color(theme::BODY()));
                }
            }
        });
    }

    fn emotes(&self, p: &egui::Painter, lay: &Layout, t: &TableLive) {
        for (seat, e, at) in &t.emotes {
            let k = ago(*at);
            if k > 3.0 {
                continue;
            }
            let pos = lay.seat(*seat) + vec2(36.0, -40.0 - 30.0 * ease(k / 3.0));
            let alpha = (1.0 - (k - 2.2).max(0.0) / 0.8).clamp(0.0, 1.0);
            p.circle_filled(pos, 20.0, Color32::from_black_alpha((150.0 * alpha) as u8));
            crate::emoji::paint_text(p, pos, Align2::CENTER_CENTER, e, theme::bold_font(18.0), Color32::WHITE.gamma_multiply(alpha), None);
        }
    }

    fn sit_dialogs(&mut self, ctx: &egui::Context, s: &ServerView, t: &TableLive) {
        let v = &t.view;
        let bank = s.poker.me.as_ref().map(|m| m.chips).unwrap_or(0);
        let (lo, hi) = (v.rules.big_blind * v.buy_in.min_bb, v.rules.big_blind * v.buy_in.max_bb);
        if let Some((table, seat, mut chips)) = self.poker.buy_in {
            let mut open = true;
            let mut sit = false;
            egui::Window::new("Sit down").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).open(&mut open).show(ctx, |ui| {
                ui.set_width(340.0);
                let top = hi.min(bank);
                if top < lo {
                    ui.label(RichText::new(format!("This table needs {} chips to sit down, and you have {}. Take the daily chips in the lobby, or find a table with smaller blinds.", chips_text(lo), chips_text(bank))).color(theme::MUTED()));
                    return;
                }
                chips = chips.clamp(lo, top);
                ui.label(RichText::new(format!("How many chips? You have {}.", chips_text(bank))).color(theme::MUTED()));
                ui.add(egui::Slider::new(&mut chips, lo..=top).step_by(v.rules.big_blind as f64));
                ui.label(RichText::new(format!("{} big blinds", chips / v.rules.big_blind)).size(12.5).color(theme::DIM()));
                ui.add_space(8.0);
                if ui.add(theme::primary_button(&format!("Sit with {}", chips_text(chips)))).clicked() {
                    sit = true;
                }
            });
            if sit {
                self.engine.poker(&s.key, ToServer::Sit { table, seat, chips });
                self.poker.buy_in = None;
            } else {
                self.poker.buy_in = open.then_some((table, seat, chips));
            }
        }
        if let Some((table, mut chips)) = self.poker.top_up {
            let have = v.my_seat.and_then(|m| v.seats[m].as_ref()).map(|s| s.stack).unwrap_or(0);
            let room = hi.saturating_sub(have).min(bank);
            let mut open = true;
            let mut add = false;
            egui::Window::new("Add chips").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0)).open(&mut open).show(ctx, |ui| {
                ui.set_width(320.0);
                if room == 0 {
                    ui.label(RichText::new("You have as many chips here as the table allows.").color(theme::MUTED()));
                    return;
                }
                chips = chips.clamp(1, room);
                ui.add(egui::Slider::new(&mut chips, 1..=room));
                if ui.add(theme::primary_button(&format!("Add {}", chips_text(chips)))).clicked() {
                    add = true;
                }
            });
            if add {
                self.engine.poker(&s.key, ToServer::TopUp { table, chips });
                self.poker.top_up = None;
            } else {
                self.poker.top_up = open.then_some((table, chips));
            }
        }
    }

    /// Someone at the table, right-clicked: how they've played here (and everywhere, for
    /// people), and their last hands, each one opening the whole hand.
    fn player_window(&mut self, ctx: &egui::Context, s: &ServerView, t: &TableLive) {
        let Some((table, pv)) = &s.poker.player else { return };
        if *table != t.view.id {
            return;
        }
        let mut open = true;
        let four = self.poker.four_color;
        let owner = s.me.as_ref().is_some_and(|m| m.user_id == t.view.owner);
        let title = match pv.bot {
            Some(l) => format!("{} · {} bot", pv.name, l.name()),
            None => pv.name.clone(),
        };
        let pct = |a: u64, b: u64| if b == 0 { "–".to_string() } else { format!("{:.0}%", a as f64 * 100.0 / b as f64) };
        let mut open_hand = None;
        let mut remove = false;
        let gm = s.poker.me.as_ref().is_some_and(|m| m.role == Some(pyr_poker::net::Role::GameMaster));
        let them = t.view.seats.get(pv.seat).cloned().flatten().map(|x| x.player).unwrap_or_default();
        let me_uid = s.me.as_ref().map(|m| m.user_id.clone());
        let left = s.poker.staff.as_ref().map(|v| v.left).unwrap_or(0);
        if gm && s.poker.staff.is_none() {
            self.engine.poker(&s.key, ToServer::Staff);
        }
        let mut gift = self.poker.giving.clone();
        let mut give = false;
        egui::Window::new(title).id(egui::Id::new("poker-player")).collapsible(false).default_width(460.0).open(&mut open).show(ctx, |ui| {
            let tally = |ui: &mut egui::Ui, heading: &str, t: &pyr_poker::net::Tally, net: bool| {
                ui.label(RichText::new(heading).strong().size(15.0));
                egui::Grid::new(heading).num_columns(4).spacing([18.0, 4.0]).show(ui, |ui| {
                    let cell = |ui: &mut egui::Ui, k: &str, v: String| {
                        ui.label(RichText::new(k).color(theme::MUTED()).size(13.5));
                        ui.label(RichText::new(v).strong().size(15.0));
                    };
                    cell(ui, "Hands played", chips_text(t.hands));
                    cell(ui, "Hands won", format!("{} ({})", chips_text(t.won), pct(t.won, t.hands)));
                    ui.end_row();
                    cell(ui, "Showdowns", chips_text(t.showdowns));
                    cell(ui, "Won at showdown", pct(t.showdowns_won, t.showdowns));
                    ui.end_row();
                    cell(ui, "Plays the hand (VPIP)", pct(t.vpip, t.hands));
                    cell(ui, "Raises first (PFR)", pct(t.pfr, t.hands));
                    ui.end_row();
                    cell(ui, "Biggest pot won", chips_text(t.biggest_pot));
                    if net {
                        let sign = if t.net >= 0 { "+" } else { "−" };
                        cell(ui, "Chips up or down", format!("{sign}{}", chips_text(t.net.unsigned_abs())));
                    } else {
                        cell(ui, "Best hand", if t.best_hand.is_empty() { "–".into() } else { t.best_hand.clone() });
                    }
                    ui.end_row();
                });
            };
            tally(ui, "At this table", &pv.here, true);
            if let Some(o) = &pv.overall {
                ui.add_space(8.0);
                tally(ui, "On this server, all time", o, false);
            }
            ui.add_space(10.0);
            ui.label(RichText::new(format!("Last hands here ({})", pv.hands.len())).strong().size(15.0));
            if pv.hands.is_empty() {
                ui.label(RichText::new("No hands finished here yet.").color(theme::MUTED()));
            }
            egui::ScrollArea::vertical().max_height(340.0).show(ui, |ui| {
                for h in &pv.hands {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
                    let p = ui.painter();
                    if resp.hovered() && h.kept {
                        p.rect_filled(r, theme::r(8), Color32::from_white_alpha(14));
                    }
                    // Their two (or four) cards, or backs when nobody saw them.
                    let mut x = r.left() + 6.0;
                    let n = if h.cards.is_empty() { if t.view.rules.hole_cards() > 2 { 4 } else { 2 } } else { h.cards.len() };
                    for k in 0..n {
                        let cr = Rect::from_min_size(pos2(x, r.top() + 4.0), vec2(31.0, 44.0));
                        match h.cards.get(k) {
                            Some(c) => cards::face(p, cr, *c, four, false),
                            None => cards::back(p, cr, ""),
                        }
                        x += 26.0;
                    }
                    x += 16.0;
                    let col = if h.net > 0 { Color32::from_rgb(0x6a, 0xd8, 0x7a) } else if h.net < 0 { Color32::from_rgb(0xf0, 0x7a, 0x6a) } else { theme::MUTED() };
                    p.text(pos2(x, r.top() + 8.0), Align2::LEFT_TOP, format!("#{} · {}", h.number, h.how), theme::bold_font(13.5), theme::TEXT());
                    let board: String = h.board.iter().map(|c| format!("{}{} ", Card::rank_label(c.rank()), c.suit_symbol())).collect();
                    let board = if board.is_empty() { "No flop".to_string() } else { format!("Board {board}") };
                    p.text(pos2(x, r.top() + 29.0), Align2::LEFT_TOP, board, theme::font(12.5), theme::MUTED());
                    let sign = if h.net > 0 { "+" } else if h.net < 0 { "−" } else { "" };
                    p.text(pos2(r.right() - 8.0, r.center().y), Align2::RIGHT_CENTER, format!("{sign}{}", chips_text(h.net.unsigned_abs())), theme::bold_font(15.0), col);
                    let resp = if h.kept { resp.on_hover_text("Open the whole hand") } else { resp };
                    if resp.clicked() && h.kept {
                        open_hand = Some(h.number);
                    }
                }
            });
            if pv.bot.is_some() && owner {
                ui.add_space(8.0);
                if ui.button("Take this bot away").clicked() {
                    remove = true;
                }
            }
            // A Game Master: a gift, right here.
            if gm && pv.bot.is_none() && Some(&them) != me_uid.as_ref() {
                ui.add_space(10.0);
                ui.label(RichText::new(format!("Gift {} (left to give today: {})", pv.name, chips_text(left))).strong().size(14.0));
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut gift.chips).range(0..=left).speed(100.0).suffix(" chips"));
                    for n in [1_000u64, 5_000, 25_000] {
                        if n <= left && ui.small_button(format!("+{}", chips_text(n))).clicked() {
                            gift.chips = (gift.chips + n).min(left);
                        }
                    }
                    if ui.add_enabled(gift.chips > 0, theme::primary_button("Give")).clicked() {
                        give = true;
                    }
                });
            }
        });
        if let Some(n) = open_hand {
            self.engine.poker(&s.key, ToServer::History { table: t.view.id, hand: n });
        }
        if give && !them.is_empty() {
            self.engine.poker(&s.key, ToServer::Gift { uid: them.clone(), chips: gift.chips, item: None, note: String::new() });
            gift = Default::default();
        }
        self.poker.giving = gift;
        if remove {
            self.engine.poker(&s.key, ToServer::RemoveBot { table: t.view.id, seat: pv.seat });
            open = false;
        }
        if !open {
            self.engine.clear_poker_player(&s.key);
        }
    }

    fn history_window(&mut self, ctx: &egui::Context, s: &ServerView) {
        let Some((_, h)) = &s.poker.history else { return };
        let streamer = self.streamer_mode();
        let mut open = true;
        let seats: Vec<Option<SeatView>> = h
            .seats
            .iter()
            .map(|x| {
                x.as_ref().map(|(uid, chips)| SeatView {
                    player: uid.clone(),
                    // (A bot's name is on the table it sits at.)
                    name: s.members.get(uid).map(|m| m.name.clone()).or_else(|| s.poker.tables.values().flat_map(|t| t.view.seats.iter().flatten()).find(|x| x.player == *uid).map(|x| x.name.clone())).unwrap_or_else(|| if uid.starts_with("bot:") { "Bot".into() } else { "Someone".into() }),
                    stack: *chips,
                    bot: None,
                    sitting_out: false,
                    level: 0,
                    card_back: String::new(),
                    frame: String::new(),
                    bet: 0,
                    in_hand: true,
                    folded: false,
                    all_in: false,
                    cards: vec![],
                })
            })
            .collect();
        egui::Window::new(format!("Hand #{}", h.number)).collapsible(false).default_width(420.0).open(&mut open).show(ctx, |ui| {
            ui.label(RichText::new(format!("{} · dealt with a shuffle anyone can check", h.rules.name())).color(theme::MUTED()));
            ui.add_space(6.0);
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for e in &h.events {
                    if let Some(line) = describe_event(e, &seats, streamer) {
                        ui.label(RichText::new(line).size(13.0));
                    }
                }
            });
            ui.add_space(6.0);
            let hex = |b: &[u8; 32]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
            ui.collapsing("Check the shuffle yourself", |ui| {
                ui.label(RichText::new(format!("The dealer's promise (before the hand): {}", hex(&h.commit))).monospace().size(11.0));
                ui.label(RichText::new(format!("The dealer's seed (after): {}", hex(&h.dealer))).monospace().size(11.0));
                for sd in &h.seeds {
                    ui.label(RichText::new(format!("A player's seed: {}", hex(sd))).monospace().size(11.0));
                }
                ui.label(RichText::new("SHA-256 of \"pyr-poker commit v1\" and the dealer's seed gives the promise; the deck is shuffled (Fisher–Yates, SHA-256 in counter mode) from SHA-256 of \"pyr-poker deck v1\", the dealer's seed, the players' seeds in order and the hand's number.").size(11.5).color(theme::DIM()));
            });
        });
        if !open {
            self.engine.clear_poker_history(&s.key);
        }
    }
}

/// A bot's picture: a disc in its own colors with a suit on it (never a person's face).
fn bot_face(p: &egui::Painter, at: Pos2, r: f32, name: &str, level: Level) {
    let h = name.bytes().fold(7u32, |h, b| h.wrapping_mul(31).wrapping_add(b as u32));
    let palette = [
        (0x3b, 0x2a, 0x6e, 0x8b, 0x6c, 0xf0),
        (0x0f, 0x4c, 0x5c, 0x3c, 0xc8, 0xd8),
        (0x5c, 0x1f, 0x2b, 0xf0, 0x6a, 0x6a),
        (0x2a, 0x4a, 0x1e, 0x9a, 0xd8, 0x5a),
        (0x5a, 0x3a, 0x0c, 0xf5, 0xb5, 0x42),
        (0x1c, 0x2c, 0x5a, 0x6a, 0x9c, 0xff),
    ];
    let (a0, a1, a2, b0, b1, b2) = palette[(h as usize) % palette.len()];
    let (dark, light) = (Color32::from_rgb(a0, a1, a2), Color32::from_rgb(b0, b1, b2));
    for k in 0..8 {
        let f = k as f32 / 7.0;
        p.circle_filled(at + vec2(-r * 0.18 * f, -r * 0.22 * f), r * (1.0 - 0.55 * f), mix(dark, light, f * 0.8));
    }
    let suit = (h / 7 % 4) as u8;
    let mark = Rect::from_center_size(at + vec2(0.0, 1.0), vec2(r * 0.95, r * 0.95));
    super::cards::suit(p, mark.translate(vec2(0.0, 1.5)), suit, Color32::from_black_alpha(90));
    super::cards::suit(p, mark, suit, Color32::from_rgba_unmultiplied(0xff, 0xff, 0xff, 225));
    if level == Level::Shark {
        // A fin on top.
        let tip = at + vec2(r * 0.15, -r * 1.02);
        p.add(Shape::convex_polygon(vec![at + vec2(-r * 0.32, -r * 0.78), tip, at + vec2(r * 0.34, -r * 0.8)], Color32::from_rgb(0x9c, 0xb8, 0xc8), Stroke::NONE));
    }
}

fn level_tag(level: Level) -> (&'static str, Color32) {
    match level {
        Level::Rookie => ("ROOKIE BOT", Color32::from_rgb(0x2f, 0x8a, 0x52)),
        Level::Regular => ("BOT", Color32::from_rgb(0x3d, 0x5f, 0xc4)),
        Level::Shark => ("SHARK BOT", Color32::from_rgb(0xc4, 0x34, 0x3e)),
    }
}

/// A card turning over: `k` from 0 (its back) to 1 (its face).
fn flip(p: &egui::Painter, r: Rect, card: Card, k: f32, four: bool, dim: bool, back: &str) {
    let k = k.clamp(0.0, 1.0);
    if k >= 1.0 {
        cards::face(p, r, card, four, dim);
        return;
    }
    // Squeezed to nothing at the halfway point.
    let w = (1.0 - 2.0 * k).abs();
    let rr = Rect::from_center_size(r.center(), vec2(r.width() * w.max(0.04), r.height()));
    if k < 0.5 {
        cards::back(p, rr, back);
    } else {
        cards::face(p, rr, card, four, dim);
    }
}

/// How far each of a seat's cards has come from the dealer (0 to 1).
fn deal_progress(t: &TableLive, seat: usize) -> Vec<f32> {
    for (at, e) in &t.events {
        if let Event::DealtFaceDown { seats, count } = e {
            let n = seats.len();
            let Some(i) = seats.iter().position(|s| *s == seat) else { return vec![1.0; *count] };
            return (0..*count)
                .map(|round| {
                    let start = at.elapsed().as_secs_f32() - (round * n + i) as f32 * 0.07;
                    (start / 0.3).clamp(0.0, 1.0)
                })
                .collect();
        }
    }
    vec![1.0; 4]
}

/// How far a seat's shown cards have turned over.
fn shown_progress(t: &TableLive, seat: usize) -> f32 {
    t.events.iter().find_map(|(at, e)| if let Event::Shown { seat: s, .. } = e { (*s == seat).then(|| ago(*at) / 0.3) } else { None }).unwrap_or(1.0)
}

/// What a seat did last this round: text, color, seconds ago.
fn last_action(t: &TableLive, seat: usize) -> Option<(String, Color32, f32)> {
    // A raise with nothing to raise yet this round is a bet.
    let opened_before = |i: usize| {
        t.events[..i]
            .iter()
            .rev()
            .take_while(|(_, e)| !matches!(e, Event::Street { .. }))
            .any(|(_, e)| matches!(e, Event::Posted { .. } | Event::Acted { action: Action::Raise { .. } | Action::AllIn, .. }))
    };
    t.events.iter().enumerate().rev().find_map(|(i, (at, e))| match e {
        Event::Acted { seat: s, action, amount, stack } if *s == seat => {
            let (text, col) = match action {
                Action::Fold => ("Fold".to_string(), Color32::from_rgb(0x8a, 0x2a, 0x30)),
                Action::Check => ("Check".to_string(), Color32::from_rgb(0x2b, 0x7a, 0x52)),
                Action::Call if *stack == 0 => ("All in".to_string(), Color32::from_rgb(0xc0, 0x3a, 0x1c)),
                Action::Call => (format!("Call {}", chips_short(*amount)), Color32::from_rgb(0x2b, 0x60, 0xa8)),
                Action::Raise { to } if *stack == 0 => (format!("All in {}", chips_short(*to)), Color32::from_rgb(0xc0, 0x3a, 0x1c)),
                Action::Raise { to } => (format!("{} {}", if opened_before(i) { "Raise" } else { "Bet" }, chips_short(*to)), Color32::from_rgb(0xc8, 0x5a, 0x16)),
                Action::AllIn => ("All in".to_string(), Color32::from_rgb(0xc0, 0x3a, 0x1c)),
            };
            Some((text, col, ago(*at)))
        }
        Event::Posted { seat: s, what, amount } if *s == seat => {
            let name = match what {
                pyr_poker::game::Posting::SmallBlind => "Small blind",
                pyr_poker::game::Posting::BigBlind => "Big blind",
                pyr_poker::game::Posting::Ante => "Ante",
            };
            Some((format!("{name} {}", chips_short(*amount)), Color32::from_rgb(0x55, 0x48, 0x40), ago(*at)))
        }
        _ => None,
    })
}

/// The hand strength meter over my cards: what I have, my chance of winning in big figures, and
/// a bar that fills and colours with it (red when it's weak, gold in between, green when strong).
fn meter(p: &egui::Painter, r: Rect, what: &str, eq: f64, live: usize) {
    let eq = eq.clamp(0.0, 1.0) as f32;
    p.rect_filled(r, theme::r(14), Color32::from_rgba_unmultiplied(0x14, 0x10, 0x0a, 232));
    p.rect_stroke(r, theme::r(14), Stroke::new(1.0f32, Color32::from_rgb(0x8a, 0x6a, 0x2a)), StrokeKind::Inside);
    // Red, through gold, to green; an even share of the pot sits in the gold.
    let fair = 1.0 / (live.max(1) + 1) as f32;
    let t = if eq < fair { 0.5 * eq / fair } else { 0.5 + 0.5 * (eq - fair) / (1.0 - fair).max(0.01) };
    let mix = |a: [u8; 3], b: [u8; 3], f: f32| Color32::from_rgb(
        (a[0] as f32 + (b[0] as f32 - a[0] as f32) * f) as u8,
        (a[1] as f32 + (b[1] as f32 - a[1] as f32) * f) as u8,
        (a[2] as f32 + (b[2] as f32 - a[2] as f32) * f) as u8,
    );
    let col = if t < 0.5 { mix([0xe0, 0x4a, 0x2e], [0xf2, 0xb8, 0x2e], t * 2.0) } else { mix([0xf2, 0xb8, 0x2e], [0x4c, 0xd0, 0x6a], (t - 0.5) * 2.0) };
    let inner = r.shrink2(vec2(12.0, 8.0));
    p.text(inner.left_top(), Align2::LEFT_TOP, what, theme::bold_font(15.5), Color32::from_rgb(0xff, 0xe2, 0x9a));
    p.text(inner.right_top() + vec2(0.0, -3.0), Align2::RIGHT_TOP, format!("{:.0}%", eq * 100.0), theme::bold_font(26.0), col);
    // The bar, with a tick at an even share.
    let bar = Rect::from_min_max(pos2(inner.left(), inner.bottom() - 14.0), pos2(inner.right(), inner.bottom()));
    p.rect_filled(bar, theme::r(7), Color32::from_white_alpha(22));
    if eq > 0.005 {
        let fill = Rect::from_min_max(bar.min, pos2(bar.left() + (bar.width() * eq).max(bar.height()), bar.bottom()));
        p.rect_filled(fill, theme::r(7), col);
    }
    let x = bar.left() + bar.width() * fair;
    p.line_segment([pos2(x, bar.top() - 3.0), pos2(x, bar.bottom() + 3.0)], Stroke::new(2.0f32, Color32::from_white_alpha(150)));
    let vs = match live { 0 | 1 => "to win vs 1 player".to_string(), n => format!("to win vs {n} players") };
    p.text(inner.left_top() + vec2(0.0, 21.0), Align2::LEFT_TOP, vs, theme::font(12.5), Color32::from_rgb(0xc8, 0xb8, 0x98));
}

/// My hand in words: "Pair of Kings", or before the flop "Ace-King suited".
fn describe_mine(variant: Variant, hole: &[Card], board: &[Card]) -> String {
    if board.len() >= 3 {
        let v = match variant {
            Variant::Holdem => eval::best_five(&[hole, board].concat()).0,
            Variant::Omaha => eval::best_omaha(hole, board).0,
        };
        return v.describe();
    }
    if variant == Variant::Holdem && hole.len() == 2 {
        let (a, b) = (hole[0], hole[1]);
        let (hi, lo) = if a.rank() >= b.rank() { (a, b) } else { (b, a) };
        if hi.rank() == lo.rank() {
            return format!("Pair of {}", Card::rank_plural(hi.rank()));
        }
        let suited = if hi.suit() == lo.suit() { "suited" } else { "offsuit" };
        return format!("{}-{} {suited}", Card::rank_name(hi.rank()), Card::rank_name(lo.rank()));
    }
    // Omaha before the flop: the best pair in hand, and how the suits pair up.
    let mut suits = [0u8; 4];
    for c in hole {
        suits[c.suit() as usize] += 1;
    }
    let pairs = suits.iter().filter(|n| **n >= 2).count();
    let suited = match pairs {
        2 => " double-suited",
        1 => " suited",
        _ => "",
    };
    let mut ranks: Vec<u8> = hole.iter().map(|c| c.rank()).collect();
    ranks.sort_unstable_by(|a, b| b.cmp(a));
    let pair = ranks.windows(2).find(|w| w[0] == w[1]).map(|w| w[0]);
    match (pair, ranks.first()) {
        (Some(r), _) => format!("Pair of {}{suited}", Card::rank_plural(r)),
        (None, Some(r)) => format!("{} high{suited}", Card::rank_name(*r)),
        _ => String::new(),
    }
}

/// A line for the log.
/// (`streamer`: my own cards stay out of it, as they stay face down at the table.)
fn describe_event(e: &Event, seats: &[Option<SeatView>], streamer: bool) -> Option<String> {
    let name = |s: &usize| seats.get(*s).and_then(|x| x.as_ref()).map(|x| x.name.clone()).unwrap_or_else(|| format!("Seat {}", s + 1));
    let cards = |cs: &[Card]| cs.iter().map(|c| format!("{}{}", Card::rank_label(c.rank()), c.suit_symbol())).collect::<Vec<_>>().join(" ");
    Some(match e {
        Event::HandStarted { number, .. } => format!("Hand #{number}"),
        Event::Posted { seat, what, amount } => format!("{} posts the {} ({})", name(seat), match what {
            pyr_poker::game::Posting::SmallBlind => "small blind",
            pyr_poker::game::Posting::BigBlind => "big blind",
            pyr_poker::game::Posting::Ante => "ante",
        }, chips_text(*amount)),
        Event::Dealt { cards: cs, .. } if streamer => format!("You're dealt {} cards (hidden: streamer mode)", cs.len()),
        Event::Dealt { cards: cs, .. } => format!("You're dealt {}", cards(cs)),
        Event::DealtFaceDown { .. } => return None,
        Event::Acted { seat, action, amount, .. } => match action {
            Action::Fold => format!("{} folds", name(seat)),
            Action::Check => format!("{} checks", name(seat)),
            Action::Call => format!("{} calls {}", name(seat), chips_text(*amount)),
            Action::Raise { to } => format!("{} raises to {}", name(seat), chips_text(*to)),
            Action::AllIn => format!("{} is all in", name(seat)),
        },
        Event::Returned { seat, amount } => format!("{} gets {} back (not called)", name(seat), chips_text(*amount)),
        Event::Street { street, board, .. } => format!("{}: {}", street.name(), cards(board)),
        Event::Turn { .. } => return None,
        Event::Shown { seat, cards: cs, hand } => format!("{} shows {} {}", name(seat), cards(cs), if hand.is_empty() { String::new() } else { format!("({hand})") }),
        Event::Mucked { seat } => format!("{} doesn't show", name(seat)),
        Event::Won { seats: ws, amounts, hand, .. } => {
            let who: Vec<String> = ws.iter().zip(amounts).map(|(s, a)| format!("{} wins {}", name(s), chips_text(*a))).collect();
            format!("{}{}", who.join(", "), hand.as_ref().map(|h| format!(" with {h}")).unwrap_or_default())
        }
        Event::HandEnded { rake, .. } if *rake > 0 => format!("Rake {} (5% of the pot, at most 3 big blinds), into the Pyr Friday Night prize pool", chips_text(*rake)),
        Event::HandEnded { .. } => return None,
    })
}

fn dashed_circle(p: &egui::Painter, c: Pos2, r: f32, stroke: Stroke) {
    let n = 24;
    for i in 0..n {
        if i % 2 == 1 {
            continue;
        }
        let a0 = i as f32 / n as f32 * TAU;
        let a1 = (i + 1) as f32 / n as f32 * TAU;
        p.line_segment([c + Vec2::angled(a0) * r, c + Vec2::angled(a1) * r], stroke);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyr_poker::cards::parse_cards;

    #[test]
    fn my_hand_in_words() {
        let h = |s: &str| parse_cards(s).unwrap();
        assert_eq!(describe_mine(Variant::Holdem, &h("AhKh"), &[]), "Ace-King suited");
        assert_eq!(describe_mine(Variant::Holdem, &h("7c7d"), &[]), "Pair of Sevens");
        assert_eq!(describe_mine(Variant::Holdem, &h("AhKd"), &h("AsKc2d")), "Two Pair, Aces and Kings");
        assert_eq!(describe_mine(Variant::Omaha, &h("JhJs4h4s"), &[]), "Pair of Jacks double-suited");
        assert_eq!(describe_mine(Variant::Omaha, &h("2d4dJcJh"), &[]), "Pair of Jacks suited");
        assert_eq!(describe_mine(Variant::Omaha, &h("Ac9d5h2s"), &[]), "Ace high");
    }
}
