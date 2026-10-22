//! Every slot's Info panel, like a big casino site's game page: its Description (cover art,
//! what it pays back, how it plays and what it pays), the Game Statistics on this server
//! (everyone's spins in the last day, week, four weeks and all time; its biggest and luckiest
//! wins), Your Statistics, and its Challenges (the achievements it has to chase).

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::state::ServerView;
use pyr_poker::casino::{Game, reels as rr, slots as sv};
use pyr_poker::net::BigWin;

use super::reel_themes::{Humor, theme};
use super::{art, col, glow_text, pics, radial, vgrad_round};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

/// The panel's tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InfoTab {
    Description,
    GameStats,
    Mine,
    Challenges,
}

/// Game Statistics' own tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StatsTab {
    #[default]
    Statistics,
    BigWins,
    LuckyWins,
}

/// A symbol in a paytable: its picture (a cell of a sheet, or Pyr Sevens' BAR, drawn), its
/// name, and what three, four and five pay.
struct PayRow {
    sheet: String,
    cell: Option<usize>,
    drawn: Option<u8>,
    name: String,
    pays: [String; 3],
}

/// A slot, as its Info panel tells it.
struct About {
    key: String,
    name: &'static str,
    cover: String,
    /// Where the cover's cut from the picture (a fraction across).
    focus: f32,
    by: &'static str,
    tags: Vec<String>,
    paragraphs: Vec<String>,
    /// The paytable: what it's in (a heading), its rows, and the specials (picture, words).
    pays_head: String,
    pays: Vec<PayRow>,
    specials: Vec<(PayRow, String)>,
    challenges: Vec<&'static str>,
    accent: Color32,
}

/// "99.00%" from hundredths of a percent.
pub fn percent_bp(bp: u32) -> String {
    format!("{}.{:02}%", bp / 100, bp % 100)
}

/// What a slot pays back, as its tag says it ("RTP: 99.00%").
pub fn rtp_tag(key: &str) -> Option<String> {
    pyr_poker::casino::slot_rtp_bp(key).map(|bp| format!("RTP: {}", percent_bp(bp)))
}

/// "a" or "an", by the word after it.
fn a(word: &str) -> String {
    let an = word.chars().next().is_some_and(|c| "aeiouAEIOU".contains(c));
    format!("{} {word}", if an { "an" } else { "a" })
}

/// A symbol's name without its "(wild)".
fn plain(name: &str) -> &str {
    name.trim_end_matches(" (wild)")
}

/// "×23.20" from ten-thousandths (or "×600" when it's whole).
fn times_tt(tt: u64) -> String {
    if tt % 10_000 == 0 { format!("×{}", tt / 10_000) } else { format!("×{}.{:02}", tt / 10_000, tt % 10_000 / 100) }
}

/// Where each machine's cover is cut from its picture (the middle unless said).
fn focus(id: &str) -> f32 {
    match id {
        "goldrush" => 0.3,
        "grind" => 0.42,
        _ => 0.5,
    }
}

fn about(game: Game, machine: u8, v13: bool) -> Option<About> {
    match game {
        Game::Slots => {
            let key = Game::Slots.id().to_string();
            // (Pyr Sevens' sheet: cherry, lemon, plum, grapes, bell, diamond, seven, flame, star;
            // the BAR is drawn.)
            let cell = |s: u8| match s {
                sv::BAR => None,
                sv::DIAMOND => Some(5),
                sv::SEVEN => Some(6),
                sv::WILD => Some(7),
                sv::STAR => Some(8),
                s => Some(s as usize),
            };
            let row = |s: u8| PayRow {
                sheet: "sevens-symbols".into(),
                cell: cell(s),
                drawn: (s == sv::BAR).then_some(s),
                name: sv::symbol_name(s).to_string(),
                pays: [0, 1, 2].map(|k| format!("×{}", sv::PAYS[s as usize][k])),
            };
            let order = [sv::WILD, sv::SEVEN, sv::DIAMOND, sv::BAR, sv::BELL, sv::GRAPES, sv::PLUM, sv::LEMON, sv::CHERRY];
            Some(About {
                tags: [rtp_tag(&key).unwrap_or_default(), "Slots".into(), "20 lines".into(), "Free spins".into(), "Phoenix Jackpot".into(), "Provably fair".into(), "Only on Pyr".into()].into(),
                key,
                name: "Pyr Sevens",
                cover: "tile-sevens".into(),
                focus: 0.5,
                by: "Pyr Originals",
                paragraphs: vec![
                    "Five reels, three rows and twenty lines. Three or more of a symbol in a row on a line, starting from the left, pays times the line bet.".into(),
                    "The wild flame stands in for every symbol but the star, and pays on its own. Five flames on a line win the Phoenix Jackpot, which grows with every spin anyone makes.".into(),
                    format!(
                        "Three, four or five stars anywhere pay {}, {} or {} times the bet and start {}, {} or {} free spins. Every win in them is tripled, and more stars add more spins.",
                        sv::STAR_PAYS[0],
                        sv::STAR_PAYS[1],
                        sv::STAR_PAYS[2],
                        sv::STAR_SPINS[0],
                        sv::STAR_SPINS[1],
                        sv::STAR_SPINS[2]
                    ),
                    "Over a long run the reels pay back 98% of what's bet, and the jackpot the 1% it takes from each spin: 99% in all.".into(),
                ]
                .into_iter()
                .chain(v13.then(|| features_line(super::stake::SlotKind::Sevens)))
                .collect(),
                pays_head: "A line pays, times the line bet (the bet is 20 line bets)".into(),
                pays: order.iter().map(|s| row(*s)).collect(),
                specials: vec![(
                    PayRow { sheet: "sevens-symbols".into(), cell: Some(8), drawn: None, name: "Star".into(), pays: Default::default() },
                    format!("Scatter: three or more anywhere start free spins and pay {}, {} or {} times the bet.", sv::STAR_PAYS[0], sv::STAR_PAYS[1], sv::STAR_PAYS[2]),
                )],
                challenges: vec!["casino_first", "free_spins", "jackpot", "big_win", "whale"],
                accent: col::EMBER,
            })
        }
        Game::Reels if v13 => {
            use pyr_poker::casino::reelrow as rw;
            let th = theme(machine);
            let key = pyr_poker::casino::reels_key(machine);
            let sheet = format!("reels-{}-symbols", th.id);
            let row = |s: usize| PayRow {
                sheet: sheet.clone(),
                cell: Some(s),
                drawn: None,
                name: plain(th.symbols[s]).to_string(),
                pays: [0, 1, 2].map(|k| times_tt(rr::PAY[s][k] * rw::POINT_TENTHOUSANDTHS)),
            };
            let blank = |s: usize| PayRow { sheet: sheet.clone(), cell: Some(s), drawn: None, name: plain(th.symbols[s]).to_string(), pays: Default::default() };
            let (game_name, game_what) = rw::GAMES[(machine as usize).min(11)];
            let mut tags = vec![rtp_tag(&key).unwrap_or_default(), "Slots".into(), "243 ways".into(), game_name.to_string(), "Play Feature".into()];
            if th.humor.is_some() {
                tags.push("Funny".into());
            }
            tags.extend(["Provably fair".to_string(), "Only on Pyr".to_string()]);
            let wild = plain(th.symbols[rr::WILD as usize]);
            let mut paragraphs = vec![format!(
                "{} Five reels, three rows and 243 ways to win: three or more of a symbol on reels side by side from the left, in any row, pays for every way it's made. The {} stands in for every pay symbol, on reels 2, 3 and 4.",
                th.tagline,
                wild.to_lowercase()
            )];
            paragraphs.push(format!("Three or more {} anywhere (about 1 spin in 176) start its own mini game, the {game_name}: {game_what} It's worth about 59 times the bet on average, and up to 800 times.", th.scatters));
            paragraphs.push("No choice in it changes the odds: what's under what comes out of the round's random numbers in the order things are opened, so every pick has the same chances.".into());
            paragraphs.push(features_line(super::stake::SlotKind::Row(machine)));
            match th.humor {
                Some(Humor::Heat) => paragraphs.push("It's a funny one: every spin that doesn't pay turns up the heat, from MILD all the way to SCOVILLED, and the chili sweats it out. Wins cool it down. (All for laughs: the heat never changes where the reels stop.)".into()),
                Some(Humor::Dentist) => paragraphs.push("It's a funny one: a sugar count beside the cabinet adds up every gram the reels land, and its host gets queasier the higher it goes. (For show: it never changes where the reels stop.)".into()),
                Some(Humor::Office) => paragraphs.push("It's a funny one: it runs on caffeine. The mug drains with every spin, wins top it up and a big one's an espresso shot; run dry and the reels slow down and doze off. (For show: where the reels stop is the house's either way.)".into()),
                Some(Humor::FishTales) => paragraphs.push("It's a funny one: your three biggest catches go up on the board beside the cabinet, and its host slips on the ice when you miss.".into()),
                None => {}
            }
            paragraphs.push("Over a long run it pays back 99% of what's bet: about 65% from the reels, the rest from the mini game. Every Play Feature pays back the same.".into());
            let specials = vec![
                (blank(rr::WILD as usize), "Wild, on reels 2, 3 and 4: it stands in for every pay symbol.".to_string()),
                (blank(rr::SCATTER as usize), format!("Scatter: three or more anywhere start the {game_name}.")),
            ];
            Some(About {
                key,
                name: th.name,
                cover: format!("rr-{}-bonus", th.id),
                focus: 0.5,
                by: "Reel Row · by Chimbita",
                tags,
                paragraphs,
                pays_head: "A way pays, times the bet (each way it's made pays)".into(),
                pays: (0..rr::PAY_SYMBOLS as usize).rev().map(row).collect(),
                specials,
                challenges: vec!["casino_first", "clean_sweep", "big_win", "whale"],
                accent: th.accent,
            })
        }
        Game::Reels => {
            let th = theme(machine);
            let key = pyr_poker::casino::reels_key(machine);
            let wheel = rr::bonus_kind(machine) == rr::BonusKind::Wheel;
            let sheet = format!("reels-{}-symbols", th.id);
            let row = |s: usize| PayRow {
                sheet: sheet.clone(),
                cell: Some(s),
                drawn: None,
                name: plain(th.symbols[s]).to_string(),
                pays: [0, 1, 2].map(|k| times_tt(rr::PAY[s][k] * rr::POINT_TENTHOUSANDTHS)),
            };
            let blank = |s: usize| PayRow { sheet: sheet.clone(), cell: Some(s), drawn: None, name: plain(th.symbols[s]).to_string(), pays: Default::default() };
            let mut tags = vec![rtp_tag(&key).unwrap_or_default(), "Slots".into(), "243 ways".into(), if wheel { "Prize wheel".into() } else { "Pick-and-dodge".into() }, "Bonus buy".into()];
            if th.humor.is_some() {
                tags.push("Funny".into());
            }
            tags.extend(["Provably fair".to_string(), "Only on Pyr".to_string()]);
            let wild = plain(th.symbols[rr::WILD as usize]);
            let mut paragraphs = vec![format!(
                "{} Five reels, three rows and 243 ways to win: three or more of a symbol on reels side by side from the left, in any row, pays for every way it's made. The {} stands in for every pay symbol, on reels 2, 3 and 4.",
                th.tagline,
                wild.to_lowercase()
            )];
            let lo = rr::OUTER.iter().filter(|x| **x != rr::UPGRADE).min().copied().unwrap_or(0);
            let hi = rr::OUTER.iter().max().copied().unwrap_or(0);
            if wheel {
                paragraphs.push(format!(
                    "Three or more {} anywhere spin the {}: {} slices worth {} to {} times the bet, and one Upgrade, which spins a second wheel worth {} to {} times.",
                    th.scatters,
                    th.bonus,
                    rr::OUTER.len() - 1,
                    lo,
                    hi,
                    rr::INNER[0],
                    rr::INNER[rr::INNER.len() - 1]
                ));
            } else {
                paragraphs.push(format!(
                    "Three or more {} anywhere start {}: {} covered tiles hide chips, {}, {} and {} {}. Each of those uses up a life; with none left, it's {} Find every safe tile for {}: {} times the bet on top.",
                    th.scatters,
                    th.bonus,
                    rr::TILES,
                    a(th.gold),
                    a(th.life),
                    rr::HAZARDS,
                    th.hazards,
                    th.busted,
                    th.finale,
                    rr::FINALE_X
                ));
            }
            paragraphs.push(format!("Or buy the bonus straight away, for {} times the bet.", rr::BUY_X));
            match th.humor {
                Some(Humor::Heat) => paragraphs.push("It's a funny one: every spin that doesn't pay turns up the heat, from MILD all the way to SCOVILLED, and the chili sweats it out. Wins cool it down. (All for laughs: the heat never changes where the reels stop.)".into()),
                Some(Humor::Dentist) => paragraphs.push("It's a funny one: a sugar count beside the cabinet adds up every gram the reels land, and its host gets queasier the higher it goes. (For show: it never changes where the reels stop.)".into()),
                Some(Humor::Office) => paragraphs.push("It's a funny one: it runs on caffeine. The mug drains with every spin, wins top it up and a big one's an espresso shot; run dry and the reels slow down and doze off. (For show: where the reels stop is the house's either way.)".into()),
                Some(Humor::FishTales) => paragraphs.push("It's a funny one: your three biggest catches go up on the board beside the cabinet, and its host slips on the ice when you miss.".into()),
                None => {}
            }
            paragraphs.push("Over a long run it pays back 99% of what's bet, bonus and all, and a bought bonus the same.".into());
            let mut specials = vec![
                (blank(rr::WILD as usize), format!("Wild, on reels 2, 3 and 4: it stands in for every pay symbol.")),
                (blank(rr::SCATTER as usize), if wheel { format!("Scatter: three or more anywhere spin the {}.", th.bonus) } else { format!("Scatter: three or more anywhere start {}.", th.bonus) }),
            ];
            if !wheel {
                specials.push((blank(8), format!("Only in {}: each one uses up a life.", th.bonus)));
            }
            Some(About {
                key,
                name: th.name,
                cover: format!("reels-{}-tile", th.id),
                focus: focus(th.id),
                by: "Reel Row · by Chimbita",
                tags,
                paragraphs,
                pays_head: "A way pays, times the bet (each way it's made pays)".into(),
                pays: (0..rr::PAY_SYMBOLS as usize).rev().map(row).collect(),
                specials,
                challenges: vec!["casino_first", if wheel { "upgrade" } else { "clean_sweep" }, "big_win", "whale"],
                accent: th.accent,
            })
        }
        Game::Dragon => {
            use pyr_poker::casino::dragon as dg;
            let key = Game::Dragon.id().to_string();
            const CELL: [usize; 9] = [8, 7, 2, 1, 0, 4, 3, 6, 5];
            const NAMES: [&str; 6] = ["Sapphire", "Ruby", "Sword", "Chalice", "Crown", "Chest"];
            let row = |s: usize| PayRow {
                sheet: "dragon-symbols".into(),
                cell: Some(CELL[s]),
                drawn: None,
                name: NAMES[s].to_string(),
                pays: [0, 1, 2].map(|k| times_tt(dg::PAY[s][k] * dg::POINT_TENTHOUSANDTHS)),
            };
            let blank = |s: usize| PayRow { sheet: "dragon-symbols".into(), cell: Some(CELL[s]), drawn: None, name: String::new(), pays: Default::default() };
            let ages: Vec<String> = dg::AGES.iter().map(|(g, name, m)| format!("{name} ×{m} ({g} eggs)")).collect();
            Some(About {
                tags: [rtp_tag(&key).unwrap_or_default(), "Slots".into(), "243 ways".into(), "Hold & spin".into(), "4 jackpots".into(), "Provably fair".into(), "Only on Pyr".into()].into(),
                key,
                name: "Ember Dragon",
                cover: "tile-dragon".into(),
                focus: 0.5,
                by: "Pyr Originals",
                paragraphs: vec![
                    "Five reels, three rows and 243 ways to win: three or more of a symbol on reels side by side from the left, in any row, pays for every way it's made. The dragon coin stands in for every pay symbol, on reels 2, 3 and 4.".into(),
                    format!("Every egg that lands feeds your dragon, and as it grows it multiplies every reel win: {}. Each bet has its own dragon, kept for you between spins.", ages.join(", ")),
                    format!("Six or more fireballs anywhere start Ember Respins: the fireballs are held and the rest of the board spins again, {} times; every new fireball sticks and gives {} more. Each holds a prize of 1 to 25 times the bet, or a jackpot. Fill all fifteen spots for the Grand. When they're over the dragon has spent its fire, and hatches again from an egg.", dg::RESPINS, dg::RESPINS),
                    "Four jackpots: Mini 10, Minor 25, Major 100 and Grand 1,000 times the bet, each with a pool on top that grows with every spin on this server and pays its share for your bet (all of it at the most bet).".into(),
                    "Over a long run it pays back 99% of what's bet: about 98.5% from the reels and Ember Respins, and the half a percent that goes into the pools back out of them.".into(),
                ]
                .into_iter()
                .chain(v13.then(|| features_line(super::stake::SlotKind::Dragon)))
                .collect(),
                pays_head: "A way pays, times the bet (each way it's made pays; the dragon multiplies it)".into(),
                pays: (0..dg::PAY_SYMBOLS as usize).rev().map(row).collect(),
                specials: vec![
                    (blank(dg::WILD as usize), "Wild, on reels 2, 3 and 4: it stands in for every pay symbol.".to_string()),
                    (blank(dg::FIRE as usize), "Fireball: six or more anywhere start Ember Respins. Each holds a prize or a jackpot.".to_string()),
                    (blank(dg::EGG as usize), "Egg: feeds your dragon, which grows to multiply reel wins up to 5 times.".to_string()),
                ],
                challenges: vec!["casino_first", "elder", "full_board", "dragon_jackpot", "big_win"],
                accent: col::EMBER,
            })
        }
        _ => None,
    }
}

/// "3 hours ago", "just now".
fn ago(ms: u64) -> String {
    let now = pyr_proto::now_ms().max(0) as u64;
    let s = now.saturating_sub(ms) / 1000;
    match s {
        0..60 => "just now".into(),
        60..3_600 => format!("{} min ago", s / 60),
        3_600..86_400 => {
            let h = s / 3_600;
            format!("{h} hour{} ago", if h == 1 { "" } else { "s" })
        }
        _ => {
            let d = s / 86_400;
            format!("{d} day{} ago", if d == 1 { "" } else { "s" })
        }
    }
}

/// The share of `wagered` paid back, as a percentage ("–" when nothing's been bet).
fn paid_back(paid: u64, wagered: u64) -> String {
    if wagered == 0 { "–".into() } else { format!("{:.2}%", paid as f64 * 100.0 / wagered as f64) }
}

/// A pill to click (a tab): returns whether it was clicked.
fn pill(ui: &mut egui::Ui, r: Rect, id: &str, text: &str, on: bool, accent: Color32) -> bool {
    let resp = ui.interact(r, ui.id().with(("info-pill", id)), Sense::click());
    let p = ui.painter();
    let fill = if on { accent } else if resp.hovered() { Color32::from_white_alpha(34) } else { Color32::from_white_alpha(14) };
    p.rect_filled(r, theme::r(17), fill);
    if !on {
        p.rect_stroke(r, theme::r(17), Stroke::new(1.0f32, Color32::from_white_alpha(22)), StrokeKind::Inside);
    }
    let dark = (accent.r() as u32 + accent.g() as u32 + accent.b() as u32) > 420;
    p.text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(13.0), if on { if dark { Color32::from_rgb(0x1a, 0x0c, 0x04) } else { Color32::WHITE } } else { col::TEXT });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A pill's width for its words.
fn pill_w(ui: &egui::Ui, text: &str) -> f32 {
    ui.painter().layout_no_wrap(text.to_string(), theme::bold_font(13.0), Color32::WHITE).size().x + 30.0
}

impl PyrApp {
    /// Opens or closes the Info panel (on its Description).
    pub fn info_toggle(&mut self) {
        self.poker.casino.info = if self.poker.casino.info.is_some() { None } else { Some(InfoTab::Description) };
    }

    /// The Info panel, over the game (Esc or a click beside it closes it).
    pub(super) fn info_panel(&mut self, ctx: &egui::Context, s: &ServerView, area: Rect) {
        let Some(tab) = self.poker.casino.info else { return };
        let Some(game) = self.poker.casino.game else { return };
        let v13 = s.has(pyr_proto::control::CASINO_013_FEATURE);
        let Some(mut ab) = about(game, self.poker.casino.reels.machine, v13) else {
            self.poker.casino.info = None;
            return;
        };
        // A Reel Row machine's top prize (its progressive jackpot's pool as it stands).
        if v13 && game == Game::Reels {
            ab.paragraphs.insert(1.min(ab.paragraphs.len()), super::floor::top_words(s, self.poker.casino.reels.machine));
        }
        // The game's statistics: asked for as it opens, and every 15 seconds while they show.
        let supported = s.has(pyr_proto::control::CASINO_012_FEATURE);
        let stale = match &self.poker.casino.info_asked {
            Some((k, at)) => *k != ab.key || (tab == InfoTab::GameStats && at.elapsed() > Duration::from_secs(15)),
            None => true,
        };
        if supported && stale {
            self.poker.casino.info_asked = Some((ab.key.clone(), Instant::now()));
            self.engine.casino_stats(&s.key, &ab.key);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.poker.casino.info = None;
            return;
        }
        // The game, dimmed: a click on it closes the panel.
        let closed = egui::Area::new(egui::Id::new("casino-info-back"))
            .order(egui::Order::Middle)
            .fixed_pos(area.min)
            .show(ctx, |ui| {
                let (r, resp) = ui.allocate_exact_size(area.size(), Sense::click());
                ui.painter().rect_filled(r, 0, Color32::from_black_alpha(175));
                resp.clicked()
            })
            .inner;
        if closed {
            self.poker.casino.info = None;
            return;
        }
        let w = (area.width() - 40.0).clamp(320.0, 940.0);
        let h = (area.height() - 36.0).clamp(300.0, 680.0);
        let card = Rect::from_center_size(area.center(), vec2(w, h));
        egui::Area::new(egui::Id::new("casino-info")).order(egui::Order::Foreground).fixed_pos(card.min).show(ctx, |ui| {
            let (card, _) = ui.allocate_exact_size(card.size(), Sense::hover());
            let p = ui.painter().clone();
            p.rect_filled(card.translate(vec2(0.0, 14.0)), theme::r(24), Color32::from_black_alpha(150));
            vgrad_round(&p, card, 22.0, Color32::from_rgb(0x1e, 0x19, 0x28), Color32::from_rgb(0x0d, 0x0b, 0x13));
            radial(&p, card.left_top() + vec2(card.width() * 0.25, 0.0), card.width() * 0.55, ab.accent.gamma_multiply(0.10), Color32::TRANSPARENT);
            p.rect_stroke(card, theme::r(22), Stroke::new(1.5f32, col::GOLD_DARK), StrokeKind::Inside);
            // The tabs, and the close button.
            let mut x = card.left() + 20.0;
            for (t, name) in [(InfoTab::Description, "Description"), (InfoTab::GameStats, "Game Statistics"), (InfoTab::Mine, "Your Statistics"), (InfoTab::Challenges, "Challenges")] {
                let pw = pill_w(ui, name);
                if x + pw > card.right() - 64.0 {
                    break;
                }
                if pill(ui, Rect::from_min_size(pos2(x, card.top() + 18.0), vec2(pw, 34.0)), name, name, tab == t, col::GOLD) {
                    self.poker.casino.info = Some(t);
                }
                x += pw + 8.0;
            }
            let close = Rect::from_center_size(pos2(card.right() - 36.0, card.top() + 35.0), vec2(34.0, 34.0));
            let cr = ui.interact(close, ui.id().with("info-close"), Sense::click());
            p.circle_filled(close.center(), 17.0, if cr.hovered() { Color32::from_white_alpha(40) } else { Color32::from_white_alpha(16) });
            crate::icons::close(&p, close.shrink(11.0), col::TEXT);
            if cr.on_hover_text("Close (Esc)").clicked() {
                self.poker.casino.info = None;
            }
            p.line_segment([pos2(card.left() + 20.0, card.top() + 66.0), pos2(card.right() - 20.0, card.top() + 66.0)], Stroke::new(1.0f32, Color32::from_white_alpha(18)));
            let body = Rect::from_min_max(pos2(card.left() + 22.0, card.top() + 78.0), pos2(card.right() - 22.0, card.bottom() - 16.0));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body).layout(egui::Layout::top_down(egui::Align::Min)));
            child.set_clip_rect(body);
            egui::ScrollArea::vertical().id_salt(("casino-info-body", tab as u8)).auto_shrink([false, false]).show(&mut child, |ui| {
                ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
                match tab {
                    InfoTab::Description => self.info_description(ui, &ab),
                    InfoTab::GameStats => self.info_game_stats(ui, s, &ab, supported),
                    InfoTab::Mine => self.info_mine(ui, s, &ab),
                    InfoTab::Challenges => self.info_challenges(ui, s, &ab),
                }
                ui.add_space(8.0);
            });
        });
    }

    fn info_description(&mut self, ui: &mut egui::Ui, ab: &About) {
        let ctx = ui.ctx().clone();
        let w = ui.available_width() - 14.0;
        // The cover beside the words (above them when it's narrow).
        let wide = w >= 600.0;
        let cover_w = if wide { (w * 0.3).clamp(180.0, 250.0) } else { (w * 0.5).clamp(140.0, 220.0) };
        let cover_h = cover_w * 4.0 / 3.0;
        let text_w = if wide { w - cover_w - 26.0 } else { w };
        let p = ui.painter().clone();
        // Lay the words out first, to know how tall the top is.
        let title = p.layout_no_wrap(ab.name.to_string(), theme::bold_font(30.0), col::GOLD2);
        let paras: Vec<_> = ab.paragraphs.iter().map(|t| p.layout(t.clone(), theme::font(14.0), col::TEXT, text_w)).collect();
        // The tags, wrapped into rows.
        let mut rows: Vec<Vec<(String, f32)>> = vec![Vec::new()];
        let mut row_w = 0.0;
        for t in ab.tags.iter().filter(|t| !t.is_empty()) {
            let tw = p.layout_no_wrap(t.clone(), theme::bold_font(12.5), Color32::WHITE).size().x + 26.0;
            if row_w + tw > text_w && !rows.last().unwrap().is_empty() {
                rows.push(Vec::new());
                row_w = 0.0;
            }
            rows.last_mut().unwrap().push((t.clone(), tw));
            row_w += tw + 8.0;
        }
        let tags_h = rows.len() as f32 * 36.0;
        let words_h = title.size().y + 28.0 + tags_h + 10.0 + paras.iter().map(|g| g.size().y + 10.0).sum::<f32>();
        let top_h = if wide { cover_h.max(words_h) } else { cover_h + 18.0 + words_h };
        let (top, _) = ui.allocate_exact_size(vec2(w, top_h), Sense::hover());
        // The cover.
        let cover = Rect::from_min_size(if wide { top.min } else { pos2(top.center().x - cover_w / 2.0, top.top()) }, vec2(cover_w, cover_h));
        p.rect_filled(cover.translate(vec2(0.0, 8.0)), theme::r(18), Color32::from_black_alpha(120));
        if !pics::cover_rounded(&p, &ctx, &ab.cover, cover, 16.0, (ab.focus, 0.5), 512, Color32::WHITE) {
            p.rect_filled(cover, theme::r(16), Color32::from_white_alpha(10));
        }
        // (The machine's name on its cover, like a box.)
        super::vgrad(&p.with_clip_rect(cover), Rect::from_min_max(pos2(cover.left(), cover.bottom() - cover_h * 0.34), cover.max), Color32::TRANSPARENT, Color32::from_black_alpha(215));
        let name_g = p.layout(ab.name.to_uppercase(), theme::bold_font((cover_w * 0.1).clamp(14.0, 22.0)), col::GOLD2, cover_w - 20.0);
        p.galley(pos2(cover.center().x - name_g.size().x / 2.0, cover.bottom() - 14.0 - name_g.size().y), name_g, col::GOLD2);
        p.rect_stroke(cover, theme::r(16), Stroke::new(1.5f32, ab.accent.gamma_multiply(0.8)), StrokeKind::Inside);
        // The words.
        let mut y = if wide { top.top() } else { cover.bottom() + 18.0 };
        let x = if wide { cover.right() + 26.0 } else { top.left() };
        let th = title.size().y;
        p.galley(pos2(x, y), title, col::GOLD2);
        y += th + 2.0;
        p.text(pos2(x, y), Align2::LEFT_TOP, ab.by, theme::font(13.0), col::MUTED);
        y += 26.0;
        for row in &rows {
            let mut tx = x;
            for (i, (t, tw)) in row.iter().enumerate() {
                let r = Rect::from_min_size(pos2(tx, y), vec2(*tw, 28.0));
                let first = t.starts_with("RTP");
                p.rect_filled(r, theme::r(14), if first { col::GOLD.gamma_multiply(0.95) } else { Color32::from_white_alpha(16) });
                if !first {
                    p.rect_stroke(r, theme::r(14), Stroke::new(1.0f32, Color32::from_white_alpha(26)), StrokeKind::Inside);
                }
                p.text(r.center(), Align2::CENTER_CENTER, t, theme::bold_font(12.5), if first { Color32::from_rgb(0x22, 0x10, 0x04) } else { col::TEXT });
                tx += tw + 8.0;
                let _ = i;
            }
            y += 36.0;
        }
        y += 10.0;
        for g in paras {
            let gh = g.size().y;
            p.galley(pos2(x, y), g, col::TEXT);
            y += gh + 10.0;
        }
        ui.add_space(22.0);
        // The paytable.
        let (hr, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
        p.text(hr.left_center(), Align2::LEFT_CENTER, "PAYTABLE", theme::bold_font(14.0), col::GOLD);
        p.text(pos2(hr.left() + 90.0, hr.center().y), Align2::LEFT_CENTER, &ab.pays_head, theme::font(12.5), col::MUTED);
        ui.add_space(6.0);
        let cols = if w >= 640.0 { 3 } else if w >= 420.0 { 2 } else { 1 };
        let gap = 10.0;
        let cw = (w - gap * (cols - 1) as f32) / cols as f32;
        for chunk in ab.pays.chunks(cols) {
            let (r, _) = ui.allocate_exact_size(vec2(w, 84.0), Sense::hover());
            for (i, pr) in chunk.iter().enumerate() {
                let c = Rect::from_min_size(pos2(r.left() + i as f32 * (cw + gap), r.top()), vec2(cw, 76.0));
                p.rect_filled(c, theme::r(14), Color32::from_white_alpha(9));
                p.rect_stroke(c, theme::r(14), Stroke::new(1.0f32, Color32::from_white_alpha(18)), StrokeKind::Inside);
                let pic = Rect::from_min_size(c.left_top() + vec2(8.0, 8.0), vec2(60.0, 60.0));
                symbol(&p, &ctx, pr, pic);
                let tx = pic.right() + 10.0;
                p.text(pos2(tx, c.top() + 18.0), Align2::LEFT_CENTER, &pr.name, theme::bold_font(13.5), col::TEXT);
                let colw = ((c.right() - tx - 8.0) / 3.0).max(40.0);
                for (k, n) in ["5", "4", "3"].iter().enumerate() {
                    let cx = tx + k as f32 * colw;
                    p.text(pos2(cx, c.top() + 42.0), Align2::LEFT_CENTER, format!("{n}×"), theme::font(11.0), col::MUTED);
                    p.text(pos2(cx, c.top() + 58.0), Align2::LEFT_CENTER, &pr.pays[2 - k], theme::bold_font(13.5), if k == 0 { col::GOLD2 } else { col::TEXT });
                }
            }
        }
        for (pr, what) in &ab.specials {
            let g = p.layout(what.clone(), theme::font(13.0), col::TEXT, w - 90.0);
            let hgt = g.size().y.max(48.0) + 16.0;
            let (r, _) = ui.allocate_exact_size(vec2(w, hgt + 8.0), Sense::hover());
            let c = Rect::from_min_size(r.min, vec2(w, hgt));
            p.rect_filled(c, theme::r(14), Color32::from_white_alpha(6));
            let pic = Rect::from_min_size(c.left_top() + vec2(10.0, (hgt - 52.0) / 2.0), vec2(52.0, 52.0));
            symbol(&p, &ctx, pr, pic);
            p.text(pos2(pic.right() + 14.0, c.top() + 18.0), Align2::LEFT_CENTER, &pr.name, theme::bold_font(13.5), col::GOLD2);
            p.galley(pos2(pic.right() + 14.0, c.top() + 30.0), g, col::TEXT);
        }
        ui.add_space(12.0);
        let (r, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
        let g = p.layout(
            "Every spin is played from the house's secret seed and one from your Pyr, and your Pyr plays it again afterwards to check it. Chips are only for play: they can't be bought or cashed out.".into(),
            theme::font(12.0),
            theme::DIM(),
            w,
        );
        p.galley(r.min, g, theme::DIM());
    }

    fn info_game_stats(&mut self, ui: &mut egui::Ui, s: &ServerView, ab: &About, supported: bool) {
        let w = ui.available_width() - 14.0;
        let p = ui.painter().clone();
        let msg = |ui: &mut egui::Ui, text: &str| {
            let (r, _) = ui.allocate_exact_size(vec2(w, 120.0), Sense::hover());
            let g = ui.painter().layout(text.to_string(), theme::font(14.0), col::MUTED, w - 40.0);
            ui.painter().galley(r.center() - g.size() / 2.0, g, col::MUTED);
        };
        if !supported {
            msg(ui, "This server's Pyr is older than 0.12: its game statistics come when it's updated.");
            return;
        }
        let Some(gs) = s.poker.casino.game_stats.get(&ab.key).cloned() else {
            msg(ui, "Asking the house for the numbers…");
            return;
        };
        // Statistics, Big Wins, Lucky Wins.
        let (r, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
        let mut x = r.left();
        let cur = self.poker.casino.info_stats;
        for (t, name) in [(StatsTab::Statistics, "Statistics"), (StatsTab::BigWins, "Big Wins"), (StatsTab::LuckyWins, "Lucky Wins")] {
            let pw = pill_w(ui, name);
            if pill(ui, Rect::from_min_size(pos2(x, r.top()), vec2(pw, 32.0)), &format!("st-{name}"), name, cur == t, ab.accent) {
                self.poker.casino.info_stats = t;
            }
            x += pw + 8.0;
        }
        if gs.playing > 0 {
            let t = format!("● {} playing now", gs.playing);
            p.text(pos2(r.right(), r.top() + 16.0), Align2::RIGHT_CENTER, t, theme::bold_font(13.0), col::WIN);
        }
        ui.add_space(10.0);
        match cur {
            StatsTab::Statistics => {
                // What it pays back by design, and what it has paid back here.
                let (r, _) = ui.allocate_exact_size(vec2(w, 104.0), Sense::hover());
                let all = gs.periods.last().cloned().unwrap_or_default();
                let cards = [
                    ("RTP", pyr_poker::casino::slot_rtp_bp(&ab.key).map(percent_bp).unwrap_or("–".into()), "What it pays back over the long run, by design"),
                    ("PAID BACK HERE", paid_back(all.paid, all.wagered), "Of every chip bet on it on this server, all time"),
                    ("TOTAL PLAYS", chips_text(all.rounds), "Spins on this server, all time"),
                ];
                let cw = (w - 20.0) / 3.0;
                for (i, (head, big, sub)) in cards.iter().enumerate() {
                    let c = Rect::from_min_size(pos2(r.left() + i as f32 * (cw + 10.0), r.top()), vec2(cw, 96.0));
                    vgrad_round(&p, c, 14.0, Color32::from_white_alpha(16), Color32::from_white_alpha(6));
                    p.rect_stroke(c, theme::r(14), Stroke::new(1.0f32, if i == 0 { col::GOLD_DARK } else { Color32::from_white_alpha(20) }), StrokeKind::Inside);
                    p.text(c.left_top() + vec2(16.0, 14.0), Align2::LEFT_TOP, *head, theme::bold_font(11.5), col::MUTED);
                    glow_text(&p, c.left_top() + vec2(16.0, 32.0), Align2::LEFT_TOP, big, theme::bold_font(26.0), if i == 0 { col::GOLD2 } else { col::TEXT }, Color32::from_black_alpha(90));
                    let g = p.layout(sub.to_string(), theme::font(11.0), theme::DIM(), cw - 28.0);
                    p.galley(c.left_top() + vec2(16.0, 68.0), g, theme::DIM());
                }
                ui.add_space(12.0);
                // The periods.
                let heads = ["", "Plays", "Wagered", "Paid out", "Paid back"];
                let col_x = |i: usize| -> f32 { if i == 0 { 16.0 } else { w * (0.28 + 0.18 * (i - 1) as f32) } };
                let (r, _) = ui.allocate_exact_size(vec2(w, 34.0), Sense::hover());
                for (i, h) in heads.iter().enumerate() {
                    p.text(pos2(r.left() + col_x(i), r.center().y), Align2::LEFT_CENTER, *h, theme::bold_font(12.0), col::MUTED);
                }
                let names = ["Last 24 hours", "Last 7 days", "Last 28 days", "All time"];
                for (i, name) in names.iter().enumerate() {
                    let ps = gs.periods.get(i).cloned().unwrap_or_default();
                    let (r, _) = ui.allocate_exact_size(vec2(w, 44.0), Sense::hover());
                    let row = r.shrink2(vec2(0.0, 3.0));
                    p.rect_filled(row, theme::r(10), Color32::from_white_alpha(if i % 2 == 0 { 10 } else { 5 }));
                    let cells = [name.to_string(), chips_text(ps.rounds), chips_text(ps.wagered), chips_text(ps.paid), paid_back(ps.paid, ps.wagered)];
                    for (k, t) in cells.iter().enumerate() {
                        let color = if k == 0 {
                            col::TEXT
                        } else if k == 4 && ps.wagered > 0 {
                            if ps.paid >= ps.wagered { col::WIN } else { col::GOLD2 }
                        } else {
                            col::TEXT
                        };
                        p.text(pos2(row.left() + col_x(k), row.center().y), Align2::LEFT_CENTER, t, if k == 0 { theme::bold_font(13.5) } else { theme::font(13.5) }, color);
                    }
                }
                ui.add_space(10.0);
                let (r, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
                let g = p.layout(
                    "Paid back over a short time swings well above and below the RTP; the more spins, the closer it comes. Everyone's spins on this server count.".into(),
                    theme::font(12.0),
                    theme::DIM(),
                    w,
                );
                p.galley(r.min, g, theme::DIM());
            }
            StatsTab::BigWins | StatsTab::LuckyWins => {
                let list: &Vec<BigWin> = if cur == StatsTab::BigWins { &gs.big } else { &gs.lucky };
                if list.is_empty() {
                    msg(ui, if cur == StatsTab::BigWins { "No big wins on it yet. Yours could be the first." } else { "No lucky wins on it yet: the wins that paid the most times their bet show here." });
                    return;
                }
                let heads = ["#", "Player", "Bet", "Win", "Multiplier", "When"];
                let xs = [14.0, 52.0, w * 0.40, w * 0.55, w * 0.71, w * 0.85];
                let (r, _) = ui.allocate_exact_size(vec2(w, 32.0), Sense::hover());
                for (i, h) in heads.iter().enumerate() {
                    p.text(pos2(r.left() + xs[i], r.center().y), Align2::LEFT_CENTER, *h, theme::bold_font(12.0), col::MUTED);
                }
                for (i, b) in list.iter().enumerate() {
                    let (r, _) = ui.allocate_exact_size(vec2(w, 44.0), Sense::hover());
                    let row = r.shrink2(vec2(0.0, 3.0));
                    let medal = match i {
                        0 => Some(col::GOLD),
                        1 => Some(Color32::from_rgb(0xc8, 0xcc, 0xd4)),
                        2 => Some(Color32::from_rgb(0xd0, 0x8a, 0x4a)),
                        _ => None,
                    };
                    p.rect_filled(row, theme::r(10), Color32::from_white_alpha(if i % 2 == 0 { 10 } else { 5 }));
                    if let Some(m) = medal {
                        p.circle_filled(pos2(row.left() + xs[0] + 8.0, row.center().y), 11.0, m);
                        p.text(pos2(row.left() + xs[0] + 8.0, row.center().y), Align2::CENTER_CENTER, (i + 1).to_string(), theme::bold_font(11.5), Color32::from_rgb(0x1a, 0x10, 0x04));
                    } else {
                        p.text(pos2(row.left() + xs[0] + 8.0, row.center().y), Align2::CENTER_CENTER, (i + 1).to_string(), theme::bold_font(12.0), col::MUTED);
                    }
                    let name = p.with_clip_rect(Rect::from_min_max(pos2(row.left() + xs[1], row.top()), pos2(row.left() + xs[2] - 10.0, row.bottom())));
                    name.text(pos2(row.left() + xs[1], row.center().y), Align2::LEFT_CENTER, &b.name, theme::bold_font(13.5), col::TEXT);
                    let m = pyr_poker::casino::multiple_x100(b.paid, b.wagered);
                    let cells = [chips_text(b.wagered), chips_text(b.paid), super::times(m), ago(b.at_ms)];
                    for (k, t) in cells.iter().enumerate() {
                        let color = match k {
                            1 => col::WIN,
                            2 => col::GOLD2,
                            3 => col::MUTED,
                            _ => col::TEXT,
                        };
                        p.text(pos2(row.left() + xs[k + 2], row.center().y), Align2::LEFT_CENTER, t, theme::bold_font(13.0), color);
                    }
                }
            }
        }
    }

    fn info_mine(&mut self, ui: &mut egui::Ui, s: &ServerView, ab: &About) {
        let w = ui.available_width() - 14.0;
        let p = ui.painter().clone();
        let Some(st) = s.poker.casino.stats.get(&ab.key).cloned().filter(|st| st.rounds > 0) else {
            let (r, _) = ui.allocate_exact_size(vec2(w, 120.0), Sense::hover());
            p.text(r.center() - vec2(0.0, 10.0), Align2::CENTER_CENTER, format!("You haven't played {} yet", ab.name), theme::bold_font(17.0), col::TEXT);
            p.text(r.center() + vec2(0.0, 16.0), Align2::CENTER_CENTER, "Your spins, wins and best moments will show here.", theme::font(13.0), col::MUTED);
            return;
        };
        let net = st.paid as i64 - st.wagered as i64;
        let tiles = [
            ("SPINS", chips_text(st.rounds), col::TEXT),
            ("WAGERED", chips_text(st.wagered), col::TEXT),
            ("PAID BACK", chips_text(st.paid), col::TEXT),
            ("UP OR DOWN", format!("{}{}", if net > 0 { "+" } else if net < 0 { "−" } else { "" }, chips_text(net.unsigned_abs())), if net > 0 { col::WIN } else if net < 0 { col::LOSE } else { col::TEXT }),
            ("YOUR RTP", paid_back(st.paid, st.wagered), col::GOLD2),
            ("BEST WIN", if st.best_win > 0 { format!("+{}", chips_text(st.best_win)) } else { "–".into() }, col::WIN),
            ("BEST MULTIPLIER", if st.best_multiple_x100 > 0 { super::times(st.best_multiple_x100) } else { "–".into() }, col::GOLD2),
        ];
        let cols = if w >= 640.0 { 4 } else { 2 };
        let cw = (w - 10.0 * (cols - 1) as f32) / cols as f32;
        for chunk in tiles.chunks(cols) {
            let (r, _) = ui.allocate_exact_size(vec2(w, 96.0), Sense::hover());
            for (i, (head, big, color)) in chunk.iter().enumerate() {
                let c = Rect::from_min_size(pos2(r.left() + i as f32 * (cw + 10.0), r.top()), vec2(cw, 86.0));
                vgrad_round(&p, c, 14.0, Color32::from_white_alpha(16), Color32::from_white_alpha(6));
                p.rect_stroke(c, theme::r(14), Stroke::new(1.0f32, Color32::from_white_alpha(20)), StrokeKind::Inside);
                p.text(c.left_top() + vec2(16.0, 16.0), Align2::LEFT_TOP, *head, theme::bold_font(11.5), col::MUTED);
                p.text(c.left_top() + vec2(16.0, 40.0), Align2::LEFT_TOP, big, theme::bold_font(24.0), *color);
            }
        }
        ui.add_space(6.0);
        let (r, _) = ui.allocate_exact_size(vec2(w, 40.0), Sense::hover());
        let g = p.layout(format!("Just you, on {}, since your first spin. The game itself pays back 99% over a long run; any one stretch can be well above or below that.", s.name), theme::font(12.0), theme::DIM(), w);
        p.galley(r.min, g, theme::DIM());
    }

    fn info_challenges(&mut self, ui: &mut egui::Ui, s: &ServerView, ab: &About) {
        let w = ui.available_width() - 14.0;
        let p = ui.painter().clone();
        let have: Vec<String> = s.poker.me.as_ref().map(|m| m.achievements.clone()).unwrap_or_default();
        let best = s.poker.casino.stats.get(&ab.key).map(|st| st.best_multiple_x100).unwrap_or(0);
        let done_n = ab.challenges.iter().filter(|c| have.iter().any(|h| h == *c)).count();
        let (r, _) = ui.allocate_exact_size(vec2(w, 34.0), Sense::hover());
        p.text(r.left_center(), Align2::LEFT_CENTER, format!("{done_n} of {} done", ab.challenges.len()), theme::bold_font(15.0), col::GOLD2);
        p.text(r.right_center(), Align2::RIGHT_CENTER, "Achievements earn experience toward your level.", theme::font(12.0), col::MUTED);
        ui.add_space(6.0);
        for id in &ab.challenges {
            let Some(a) = pyr_poker::progress::achievement(id) else { continue };
            let done = have.iter().any(|h| h == id);
            let (r, _) = ui.allocate_exact_size(vec2(w, 78.0), Sense::hover());
            let c = Rect::from_min_size(r.min, vec2(w, 70.0));
            if done {
                vgrad_round(&p, c, 14.0, Color32::from_rgb(0x3a, 0x2c, 0x10), Color32::from_rgb(0x1e, 0x16, 0x08));
                p.rect_stroke(c, theme::r(14), Stroke::new(1.5f32, col::GOLD), StrokeKind::Inside);
            } else {
                p.rect_filled(c, theme::r(14), Color32::from_white_alpha(9));
                p.rect_stroke(c, theme::r(14), Stroke::new(1.0f32, Color32::from_white_alpha(20)), StrokeKind::Inside);
            }
            let badge = pos2(c.left() + 36.0, c.center().y);
            if done {
                p.circle_filled(badge, 20.0, col::GOLD);
                crate::icons::tick(&p, Rect::from_center_size(badge, vec2(20.0, 20.0)), Color32::from_rgb(0x2a, 0x14, 0x04));
            } else {
                p.circle_stroke(badge, 19.0, Stroke::new(2.0f32, Color32::from_white_alpha(60)));
                crate::icons::star(&p, Rect::from_center_size(badge, vec2(20.0, 20.0)), Color32::from_white_alpha(90), false);
            }
            p.text(pos2(c.left() + 70.0, c.top() + 24.0), Align2::LEFT_CENTER, a.name, theme::bold_font(15.0), if done { col::GOLD2 } else { col::TEXT });
            let mut about = a.about.to_string();
            if *id == "big_win" && !done && best > 0 {
                about = format!("{about} Your best on it so far: {}.", super::times(best));
            }
            p.text(pos2(c.left() + 70.0, c.top() + 48.0), Align2::LEFT_CENTER, about, theme::font(13.0), col::MUTED);
            let xp = format!("+{} XP", chips_text(a.xp));
            let xr = Rect::from_center_size(pos2(c.right() - 52.0, c.center().y), vec2(76.0, 28.0));
            p.rect_filled(xr, theme::r(14), if done { col::GOLD.gamma_multiply(0.9) } else { Color32::from_white_alpha(16) });
            p.text(xr.center(), Align2::CENTER_CENTER, xp, theme::bold_font(12.5), if done { Color32::from_rgb(0x22, 0x10, 0x04) } else { col::TEXT });
        }
    }
}

/// A paytable's symbol picture.
fn symbol(p: &egui::Painter, ctx: &egui::Context, pr: &PayRow, r: Rect) {
    match (pr.cell, pr.drawn) {
        (_, Some(s)) => art::paint_symbol(p, r, s, Color32::WHITE),
        (Some(i), None) => {
            pics::cell(p, ctx, &pr.sheet, i, r, (r.width() * 3.0) as u32, Color32::WHITE);
        }
        _ => {}
    }
}

/// A slot's Play Feature, in a few words with what each costs.
fn features_line(kind: super::stake::SlotKind) -> String {
    use pyr_poker::casino::feature::Feature;
    let parts: Vec<String> = [Feature::Chance, Feature::Wild, Feature::Bonus, Feature::Super, Feature::Mystery, Feature::AllIn]
        .into_iter()
        .filter_map(|f| {
            let h = kind.price(f)?;
            let x = if h % 100 == 0 { format!("{}×", h / 100) } else { format!("{:.2}×", h as f64 / 100.0) };
            Some(format!("{} ({x})", kind.describe(f).0))
        })
        .collect();
    format!("Play Feature, beside the spin button, plays it other ways, each priced to pay back the same: {}. Fairness shows your own seed for it (change it any time), the house's next seed's fingerprint, and any round played again from the two.", parts.join(", "))
}

