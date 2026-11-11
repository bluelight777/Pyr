//! Pyr Casino's control room (0.13.5), as 0.13.28 has it, for the Owner of the Game and whoever
//! they let in: the casino at a glance; every player, to look up and give or take chips; the
//! whole chips log (and a spreadsheet of it); a word to everyone in the casino; and who else may
//! open it. Everything that moves a chip is in the log.

use std::path::PathBuf;
use std::time::Instant;

use eframe::egui::{self, Align2, Color32, FontId, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::state::ServerView;
use pyr_poker::levels;
use pyr_poker::net::{CmsOverview, CmsPlayer, CmsReq, StaffLog, ToServer};

use super::cards::{chips_short, chips_text};
use super::staff::typed_chips;
use crate::app::{Act, PyrApp};
use crate::casino::levels::{crest, tier_color};
use crate::theme;

// 0.13.28's colors, read off its control room.
const BLUE: Color32 = Color32::from_rgb(0x6a, 0xa8, 0xff);
const HEAD_TOP: Color32 = Color32::from_rgb(0x10, 0x1b, 0x33);
const HEAD_BOTTOM: Color32 = Color32::from_rgb(0x08, 0x0d, 0x19);
const HEAD_LINE: Color32 = Color32::from_rgb(0x46, 0x70, 0xad);
const TAB_TOP: Color32 = Color32::from_rgb(0x1b, 0x24, 0x3a);
const TAB_BOTTOM: Color32 = Color32::from_rgb(0x17, 0x1e, 0x30);
const PICKED_TOP: Color32 = Color32::from_rgb(0x19, 0x23, 0x37);
const PICKED_BOTTOM: Color32 = Color32::from_rgb(0x1c, 0x28, 0x3e);
const GOLD: Color32 = Color32::from_rgb(0xff, 0xe2, 0x9a);
const ORANGE: Color32 = Color32::from_rgb(0xff, 0x8a, 0x2a);
const GREEN: Color32 = Color32::from_rgb(0x3a, 0xe0, 0x7a);
const RAKE: Color32 = Color32::from_rgb(0xd8, 0x3a, 0x4a);
const FRIDAY: Color32 = Color32::from_rgb(0x9f, 0xc0, 0xff);
const PHOENIX: Color32 = Color32::from_rgb(0xff, 0x6a, 0x2a);
const PURPLE: Color32 = Color32::from_rgb(0x9a, 0x6a, 0xf0);
const TOOK: Color32 = Color32::from_rgb(0xff, 0x6a, 0x6a);
const WON: Color32 = Color32::from_rgb(0x6a, 0xe8, 0x8a);
const TAKE_NOW: Color32 = Color32::from_rgb(0xb0, 0x20, 0x2a);

/// The most of the log shown at once (the download has it all).
const LOG_SHOWN: usize = 500;

/// The control room's pages.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum CmsPage {
    #[default]
    Overview,
    Players,
    Log,
    Announce,
    Access,
}

const PAGES: [(CmsPage, &str); 5] = [(CmsPage::Overview, "Overview"), (CmsPage::Players, "Players"), (CmsPage::Log, "Chips log"), (CmsPage::Announce, "Announce"), (CmsPage::Access, "Access")];

/// The control room on screen.
#[derive(Default)]
pub struct CmsUi {
    pub page: CmsPage,
    /// The page last asked for (asked again when another opens, or something's been done).
    pub asked: Option<CmsPage>,
    /// Looking for a player: what's typed, what was last asked for, and when it was last typed
    /// (it's asked for a moment after the typing stops).
    pub query: String,
    pub query_sent: String,
    pub query_typed: Option<Instant>,
    /// The player picked, to give or take chips: their id; how many and the note for the log.
    pub picked: Option<String>,
    pub amount: String,
    pub note: String,
    /// "Take N away now": clicked once, for N chips (a second click takes them).
    pub take_armed: Option<u64>,
    pub filter: String,
    /// A word to everyone in the casino, being written.
    pub text: String,
    /// Someone picked to let in.
    pub grant: Option<String>,
    /// The "done" lines already said.
    pub done_seen: usize,
}

/// Which log a download is: the staff page's or the control room's (they say it their own way).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LogFile {
    Staff,
    Chips,
}

/// When, as the log's spreadsheet has it: "2026-11-11 06:37:25" (UTC).
pub fn utc(at_ms: u64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(at_ms as i64).map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_default()
}

/// The log as a spreadsheet file (CSV), as 0.13.28 writes it: `When (UTC),By,What`, then a row
/// for each entry (newest first), every field quoted.
pub fn log_csv(log: &[StaffLog]) -> String {
    let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
    let mut out = String::from("When (UTC),By,What\r\n");
    for l in log {
        out.push_str(&format!("{},{},{}\r\n", q(&utc(l.at_ms)), q(&l.by), q(&l.what)));
    }
    out
}

/// The file's name: "Pyr staff log - Test Camp - 2026-11-11 06-37-40.csv" (the time UTC; what a
/// file name can't have in it, out of the server's name).
pub fn log_file_name(server: &str, now_ms: u64) -> String {
    let name: String = server.chars().map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '-' } else { c }).collect();
    let name = name.trim().trim_end_matches('.');
    let when = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now_ms as i64).map(|t| t.format("%Y-%m-%d %H-%M-%S").to_string()).unwrap_or_default();
    format!("Pyr staff log - {} - {when}.csv", if name.is_empty() { "Pyr" } else { name })
}

/// Saves the log in Downloads (where there's none, the temporary folder).
fn save_log(server: &str, log: &[StaffLog]) -> std::io::Result<PathBuf> {
    let dir = directories::UserDirs::new().and_then(|u| u.download_dir().map(|d| d.to_path_buf())).unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(log_file_name(server, pyr_proto::now_ms().max(0) as u64));
    std::fs::write(&path, log_csv(log))?;
    Ok(path)
}

/// A log entry's color: green for chips given, red for chips taken, blue for the rest.
fn log_color(what: &str) -> Color32 {
    if what.starts_with("gave") {
        GREEN
    } else if what.starts_with("took") {
        TOOK
    } else {
        BLUE
    }
}

/// A line of text with its capitals' middle at `y` (as egui sets Inter, it's the row's middle).
fn text(p: &egui::Painter, x: f32, y: f32, align: Align2, words: impl ToString, font: FontId, color: Color32) -> Rect {
    p.text(pos2(x, y), align, words, font, color)
}

fn regular(size: f32) -> FontId {
    FontId::proportional(size)
}

/// A card as the control room's: dark, a hairline round it.
fn card(p: &egui::Painter, r: Rect) {
    p.rect_filled(r, theme::r(10), theme::ELEV());
    p.rect_stroke(r, theme::r(10), Stroke::new(1.0f32, theme::LINE()), StrokeKind::Inside);
}

/// The colored bar down a card's left edge.
fn accent(p: &egui::Painter, r: Rect, inset: f32, color: Color32) {
    p.rect_filled(Rect::from_min_max(pos2(r.left(), r.top() + inset), pos2(r.left() + 4.0, r.bottom() - inset)), theme::r(2), color);
}

impl PyrApp {
    /// The control room's tab: it's asked for afresh.
    pub(crate) fn cms_opened(&mut self) {
        self.poker.cms.asked = None;
    }

    /// What the control room's page needs, asked for when it opens (and again when something's
    /// been done there); a look-up a moment after the typing stops.
    fn cms_ask(&mut self, s: &ServerView) {
        let c = &mut self.poker.cms;
        if c.asked != Some(c.page) {
            c.asked = Some(c.page);
            match c.page {
                CmsPage::Overview => self.engine.poker(&s.key, ToServer::Cms { req: CmsReq::Overview }),
                CmsPage::Players => {
                    c.query_sent = c.query.trim().to_string();
                    self.engine.poker(&s.key, ToServer::Cms { req: CmsReq::Players { query: c.query_sent.clone() } });
                }
                CmsPage::Log => self.engine.poker(&s.key, ToServer::StaffLogAll),
                CmsPage::Announce => {}
                CmsPage::Access => self.engine.poker(&s.key, ToServer::Cms { req: CmsReq::Access }),
            }
        }
        if c.page == CmsPage::Players && c.query_typed.is_some_and(|t| t.elapsed().as_secs_f32() > 0.3) && c.query.trim() != c.query_sent {
            c.query_typed = None;
            c.query_sent = c.query.trim().to_string();
            self.engine.poker(&s.key, ToServer::Cms { req: CmsReq::Players { query: c.query_sent.clone() } });
        }
    }

    /// What the control room did, said in a toast (the server's words); the page is asked for
    /// again, its numbers having moved.
    pub(crate) fn cms_done(&mut self, s: &ServerView, acts: &mut Vec<Act>) {
        let done = &s.poker.cms.done;
        if done.is_empty() {
            self.poker.cms.done_seen = 0;
            return;
        }
        self.engine.take_cms_done(&s.key);
        if self.poker.cms.done_seen == done.len() {
            return;
        }
        self.poker.cms.done_seen = done.len();
        for d in done {
            acts.push(Act::Toast(d.clone(), false));
        }
        let c = &mut self.poker.cms;
        c.amount.clear();
        c.take_armed = None;
        c.asked = None;
    }

    /// "Download the whole log": the whole log asked for, and saved when it comes.
    pub(crate) fn download_staff_log(&mut self, s: &ServerView, kind: LogFile) {
        self.poker.log_download = Some((s.key.clone(), s.poker.staff_log_seq, Instant::now(), kind));
        self.engine.poker(&s.key, ToServer::StaffLogAll);
    }

    /// The log asked for has come: saved in Downloads, said in a toast, its folder opened.
    pub(crate) fn staff_log_saved(&mut self, s: &ServerView, acts: &mut Vec<Act>) {
        let Some((key, seq, at, kind)) = self.poker.log_download.clone() else { return };
        if key != s.key {
            return;
        }
        if s.poker.staff_log_seq <= seq {
            // (Refused, or lost: the server's said why, if it could.)
            if at.elapsed().as_secs() >= 20 {
                self.poker.log_download = None;
            }
            return;
        }
        self.poker.log_download = None;
        let log = s.poker.staff_log_all.clone().unwrap_or_default();
        match save_log(&s.name, &log) {
            Ok(path) => {
                let n = chips_text(log.len() as u64);
                let words = match kind {
                    LogFile::Staff => format!("Saved the staff log ({n} entries) in Downloads: {}", path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()),
                    LogFile::Chips => format!("Saved the chips log ({n} entries) in Downloads"),
                };
                acts.push(Act::Toast(words, false));
                crate::settings::show_in_folder(&path);
            }
            Err(e) => {
                let words = match kind {
                    LogFile::Staff => format!("Couldn't save the staff log: {e}"),
                    LogFile::Chips => format!("Couldn't save the log: {e}"),
                };
                acts.push(Act::Toast(words, true));
            }
        }
    }

    /// The casino's Control room tab (lined up with the casino's other pages).
    pub(crate) fn cms_page(&mut self, ui: &mut egui::Ui, s: &ServerView) {
        self.cms_ask(s);
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            let w = (ui.available_width() - 28.0).max(600.0);
            ui.vertical(|ui| {
                ui.set_width(w);
                ui.spacing_mut().item_spacing.y = 0.0;
                self.cms_header(ui, w);
                ui.add_space(22.0);
                match self.poker.cms.page {
                    CmsPage::Overview => self.cms_overview(ui, s, w),
                    CmsPage::Players => self.cms_players(ui, s, w),
                    CmsPage::Log => self.cms_log(ui, s, w),
                    CmsPage::Announce => self.cms_announce(ui, s),
                    CmsPage::Access => self.cms_access(ui, s),
                }
            });
        });
    }

    /// "PYR CASINO / Control room", and the pages to pick from.
    fn cms_header(&mut self, ui: &mut egui::Ui, w: f32) {
        let (r, _) = ui.allocate_exact_size(vec2(w, 96.0), Sense::hover());
        let p = ui.painter().clone();
        crate::casino::vgrad_round(&p, r, 14.0, HEAD_TOP, HEAD_BOTTOM);
        p.rect_stroke(r, theme::r(14), Stroke::new(1.0f32, HEAD_LINE), StrokeKind::Inside);
        text(&p, r.left() + 24.0, r.top() + 24.0, Align2::LEFT_CENTER, "PYR CASINO", theme::bold_font(11.0), BLUE);
        text(&p, r.left() + 24.0, r.top() + 50.5, Align2::LEFT_CENTER, "Control room", theme::bold_font(26.0), Color32::WHITE);
        text(&p, r.left() + 24.0, r.top() + 76.5, Align2::LEFT_CENTER, "Everything that moves a chip is in the log.", regular(12.5), theme::MUTED());
        // The pages, right to left from the card's right edge.
        let font = theme::bold_font(13.5);
        let widths: Vec<f32> = PAGES.iter().map(|(_, n)| p.layout_no_wrap(n.to_string(), font.clone(), Color32::WHITE).size().x + 28.0).collect();
        let total: f32 = widths.iter().sum::<f32>() + 8.0 * (PAGES.len() - 1) as f32;
        let mut x = r.right() - 16.0 - total;
        for ((page, name), tw) in PAGES.iter().zip(widths) {
            let tab = Rect::from_min_size(pos2(x, r.center().y - 17.0), vec2(tw, 34.0));
            let resp = ui.interact(tab, ui.id().with(("cms-tab", *name)), Sense::click());
            let on = self.poker.cms.page == *page;
            if on {
                p.rect_filled(tab, theme::r(17), BLUE);
            } else {
                let (top, bottom) = if resp.hovered() { (crate::casino::mix(TAB_TOP, Color32::WHITE, 0.06), crate::casino::mix(TAB_BOTTOM, Color32::WHITE, 0.06)) } else { (TAB_TOP, TAB_BOTTOM) };
                crate::casino::vgrad_round(&p, tab, 17.0, top, bottom);
            }
            text(&p, tab.center().x, tab.center().y, Align2::CENTER_CENTER, name, font.clone(), Color32::WHITE);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
                self.poker.cms.page = *page;
            }
            x += tw + 8.0;
        }
    }

    /// The casino at a glance.
    fn cms_overview(&mut self, ui: &mut egui::Ui, s: &ServerView, w: f32) {
        let Some(o) = s.poker.cms.overview.clone() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Opening the control room…").color(theme::MUTED()));
            });
            return;
        };
        let staff = chips_text(o.staff);
        let paid_back = if o.wagered == 0 { 0.0 } else { o.paid as f64 * 100.0 / o.wagered as f64 };
        let stats: [(&str, String, String, Color32); 8] = [
            ("PLAYERS", chips_text(o.players), format!("{staff} staff"), BLUE),
            ("CHIPS IN PLAY", chips_short(o.chips), format!("{} at tables", chips_short(o.at_tables)), GOLD),
            ("BET IN THE CASINO", chips_short(o.wagered), format!("{} rounds", chips_text(o.rounds)), ORANGE),
            ("PAID BACK", format!("{paid_back:.2}%"), format!("{} paid", chips_short(o.paid)), GREEN),
            ("RAKE THIS WEEK", chips_short(o.rake_week), "poker's cash tables".into(), RAKE),
            ("FRIDAY NIGHT POOL", chips_short(o.friday_pool), "the next Friday's prize".into(), FRIDAY),
            ("PHOENIX JACKPOT", chips_short(o.jackpot), "Pyr Sevens".into(), PHOENIX),
            ("TABLES", chips_text(o.tables), format!("{} tournaments · {} log entries", chips_text(o.tourneys), chips_text(o.log)), PURPLE),
        ];
        let cw = (w - 12.0) / 2.0;
        for pair in stats.chunks(2) {
            let (row, _) = ui.allocate_exact_size(vec2(w, 96.0), Sense::hover());
            let p = ui.painter();
            for (i, (caption, value, sub, color)) in pair.iter().enumerate() {
                let r = Rect::from_min_size(pos2(row.left() + i as f32 * (cw + 12.0), row.top()), vec2(cw, 96.0));
                card(p, r);
                accent(p, r, 14.0, *color);
                text(p, r.left() + 18.0, r.top() + 20.5, Align2::LEFT_CENTER, caption, theme::bold_font(10.5), *color);
                text(p, r.left() + 18.0, r.top() + 52.5, Align2::LEFT_CENTER, value, theme::bold_font(26.0), Color32::WHITE);
                text(p, r.left() + 18.0, r.top() + 80.5, Align2::LEFT_CENTER, sub, regular(12.0), theme::MUTED());
            }
            ui.add_space(12.0);
        }
        ui.add_space(14.0);
        self.cms_tiers(ui, &o, w);
        ui.add_space(18.0);
        // The biggest wins, and the latest in the log: side by side, as tall as the longer.
        let rows = o.big_wins.len().max(o.recent.len());
        let h = 48.0 + rows as f32 * 26.0;
        let (both, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
        let p = ui.painter();
        let wins = Rect::from_min_size(both.min, vec2(cw, h));
        let latest = Rect::from_min_size(pos2(both.left() + cw + 12.0, both.top()), vec2(cw, h));
        card(p, wins);
        card(p, latest);
        text(p, wins.left() + 18.0, wins.top() + 22.0, Align2::LEFT_CENTER, "BIG WINS", theme::bold_font(10.5), GOLD);
        text(p, latest.left() + 18.0, latest.top() + 22.0, Align2::LEFT_CENTER, "THE LATEST IN THE LOG", theme::bold_font(10.5), BLUE);
        for (i, b) in o.big_wins.iter().enumerate() {
            let y = wins.top() + 50.0 + i as f32 * 26.0;
            text(p, wins.left() + 18.0, y, Align2::LEFT_CENTER, format!("{} · {}", b.name, b.game.name()), regular(12.5), theme::TEXT());
            let won = format!("+{} ({}×)", chips_text(b.paid.saturating_sub(b.wagered)), b.paid / b.wagered.max(1));
            text(p, wins.right() - 18.0, y, Align2::RIGHT_CENTER, won, theme::bold_font(12.5), WON);
        }
        let clip = p.with_clip_rect(latest.shrink2(vec2(18.0, 0.0)).intersect(p.clip_rect()));
        for (i, l) in o.recent.iter().enumerate() {
            let y = latest.top() + 50.0 + i as f32 * 26.0;
            text(&clip, latest.left() + 18.0, y, Align2::LEFT_CENTER, format!("{} {}", l.by, l.what), regular(12.0), theme::TEXT());
        }
    }

    /// The VIP Club: how many players are at each tier (bars), and the chips they hold.
    fn cms_tiers(&mut self, ui: &mut egui::Ui, o: &CmsOverview, w: f32) {
        let (r, _) = ui.allocate_exact_size(vec2(w, 230.0), Sense::hover());
        let p = ui.painter();
        card(p, r);
        text(p, r.left() + 18.0, r.top() + 22.0, Align2::LEFT_CENTER, "THE VIP CLUB · PLAYERS AT EACH TIER", theme::bold_font(10.5), GOLD);
        let n = levels::TIERS.len();
        let col = (r.width() - 36.0) / n as f32;
        let base = r.top() + 184.0;
        let most = o.tier_players.iter().copied().max().unwrap_or(0).max(1);
        for t in 0..n {
            let cx = r.left() + 18.0 + (t as f32 + 0.5) * col;
            let players = o.tier_players.get(t).copied().unwrap_or(0);
            let chips = o.by_tier.get(t).copied().unwrap_or(0);
            let c = tier_color(t);
            let top = if players == 0 {
                let line = Rect::from_min_max(pos2(cx - 24.0, base - 2.0), pos2(cx + 24.0, base));
                p.rect_filled(line, 0, c.gamma_multiply(0.86));
                base - 2.0
            } else {
                let h = 110.0 * players as f32 / most as f32;
                let bar = Rect::from_min_max(pos2(cx - 24.5, base - h), pos2(cx + 24.5, base));
                crate::casino::vgrad_round(p, bar, 4.0, c, theme::mix(c, theme::ELEV(), 0.52));
                base - h
            };
            text(p, cx, top - 10.0, Align2::CENTER_CENTER, chips_text(players), theme::bold_font(12.0), Color32::WHITE);
            crest(p, ui.ctx(), t, Rect::from_center_size(pos2(cx, base + 15.5), vec2(26.0, 26.0)));
            text(p, cx, base + 36.0, Align2::CENTER_CENTER, format!("{} · {}", levels::TIERS[t].0, chips_short(chips)), regular(11.0), theme::MUTED());
        }
    }

    /// Every player (most chips first), to look up; one picked, to give or take chips.
    fn cms_players(&mut self, ui: &mut egui::Ui, s: &ServerView, w: f32) {
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.poker.cms.query).hint_text("Look for a player").desired_width(320.0));
            if r.changed() {
                self.poker.cms.query_typed = Some(Instant::now());
            }
            if let Some(rows) = &s.poker.cms.players {
                ui.label(RichText::new(format!("{} shown, most chips first · click one to give or take chips", chips_text(rows.len() as u64))).color(theme::MUTED()).size(14.5));
            }
        });
        ui.add_space(16.0);
        let Some(rows) = s.poker.cms.players.clone() else {
            ui.spinner();
            return;
        };
        if self.poker.cms.query_typed.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
        }
        for row in &rows {
            let picked = self.poker.cms.picked.as_deref() == Some(row.uid.as_str());
            let (r, resp) = ui.allocate_exact_size(vec2(w, 56.0), Sense::click());
            self.cms_player_row(ui, r, row, picked);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                let c = &mut self.poker.cms;
                c.picked = if picked { None } else { Some(row.uid.clone()) };
                c.take_armed = None;
            }
            if picked {
                ui.add_space(6.0);
                self.cms_adjust(ui, s, row);
                ui.add_space(6.0);
            }
            ui.add_space(12.0);
        }
    }

    fn cms_player_row(&self, ui: &egui::Ui, r: Rect, row: &CmsPlayer, picked: bool) {
        let p = ui.painter();
        if picked {
            crate::casino::vgrad_round(p, r, 10.0, PICKED_TOP, PICKED_BOTTOM);
        } else {
            p.rect_filled(r, theme::r(10), theme::ELEV());
        }
        let t = levels::tier(row.level);
        crest(p, ui.ctx(), t, Rect::from_center_size(pos2(r.left() + 29.5, r.center().y), vec2(36.0, 36.0)));
        text(p, r.left() + 60.0, r.top() + 19.0, Align2::LEFT_CENTER, &row.name, theme::bold_font(15.0), Color32::WHITE);
        let mut line = format!("Level {} · {}", row.level, levels::title(row.level));
        if let Some(role) = row.role {
            line.push_str(&format!(" · {}", role.name()));
        }
        text(p, r.left() + 60.0, r.top() + 39.0, Align2::LEFT_CENTER, line, regular(12.0), tier_color(t));
        let cols: [(&str, String); 6] = [
            ("CHIPS", chips_text(row.chips)),
            ("AT TABLES", chips_short(row.at_tables)),
            ("HANDS", chips_text(row.hands)),
            ("ROUNDS", chips_text(row.rounds)),
            ("BET", chips_short(row.wagered)),
            ("PAID", chips_short(row.paid)),
        ];
        let step = (r.width() - 330.0) / cols.len() as f32;
        for (i, (caption, value)) in cols.iter().enumerate() {
            let x = r.left() + 300.0 + i as f32 * step;
            text(p, x, r.top() + 18.0, Align2::LEFT_CENTER, caption, theme::bold_font(9.5), theme::DIM());
            text(p, x, r.top() + 38.0, Align2::LEFT_CENTER, value, theme::bold_font(14.5), if i == 0 { GOLD } else { theme::TEXT() });
        }
    }

    /// Under the player picked: chips to give or take (taking is clicked twice), and a note for
    /// the log.
    fn cms_adjust(&mut self, ui: &mut egui::Ui, s: &ServerView, row: &CmsPlayer) {
        let c = &mut self.poker.cms;
        let n = typed_chips(&c.amount);
        if c.take_armed.is_some_and(|a| a != n) {
            c.take_armed = None;
        }
        let mut send: Option<i64> = None;
        ui.horizontal(|ui| {
            ui.set_height(30.0);
            ui.add_space(20.0);
            ui.label(RichText::new("Chips").color(theme::MUTED()).size(14.5));
            ui.add(egui::TextEdit::singleline(&mut c.amount).hint_text("e.g. 50,000").desired_width(140.0));
            ui.add(egui::TextEdit::singleline(&mut c.note).hint_text("A note for the log (optional)").desired_width(260.0).char_limit(140));
            if ui.add_enabled(n > 0, theme::primary_button("Give").min_size(vec2(0.0, 30.0))).clicked() {
                send = Some(n.min(i64::MAX as u64) as i64);
            }
            match c.take_armed {
                Some(a) => {
                    let b = egui::Button::new(RichText::new(format!("Take {} away now", chips_text(a))).color(Color32::WHITE).font(theme::bold_font(14.0))).fill(TAKE_NOW).corner_radius(theme::r(6)).min_size(vec2(0.0, 30.0));
                    if ui.add(b).clicked() {
                        send = Some(-(a.min(i64::MAX as u64) as i64));
                    }
                }
                None => {
                    if ui.add_enabled(n > 0, egui::Button::new(RichText::new("Take away").size(14.0)).fill(theme::ELEV2()).min_size(vec2(0.0, 30.0))).clicked() {
                        c.take_armed = Some(n);
                    }
                }
            }
            ui.label(RichText::new("Both go in the log, with their balance before and after.").color(theme::MUTED()).size(12.0));
        });
        if let Some(delta) = send {
            c.take_armed = None;
            self.engine.poker(&s.key, ToServer::Cms { req: CmsReq::Adjust { uid: row.uid.clone(), delta, note: c.note.trim().to_string() } });
        }
    }

    /// The whole log, newest first (the first 500 shown), to filter; and the lot as a
    /// spreadsheet.
    fn cms_log(&mut self, ui: &mut egui::Ui, s: &ServerView, w: f32) {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.poker.cms.filter).hint_text("Filter: a name, \"took\", \"gave\"…").desired_width(320.0));
            if ui.add(theme::primary_button("Download the whole log").min_size(vec2(0.0, 30.0))).on_hover_text("A spreadsheet file (CSV) in your Downloads").clicked() {
                self.download_staff_log(s, LogFile::Chips);
            }
        });
        ui.add_space(22.0);
        let Some(log) = &s.poker.staff_log_all else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Reading the log…").color(theme::MUTED()));
            });
            return;
        };
        let filter = self.poker.cms.filter.trim().to_lowercase();
        let shown: Vec<&StaffLog> = log.iter().filter(|l| filter.is_empty() || l.by.to_lowercase().contains(&filter) || l.what.to_lowercase().contains(&filter) || utc(l.at_ms).contains(&filter)).collect();
        let n = chips_text(shown.len() as u64);
        let more = if shown.len() > LOG_SHOWN { " (the first 500 shown)" } else { "" };
        let count = if filter.is_empty() { format!("{n} entries{more}") } else { format!("{n} matching{more}") };
        ui.label(RichText::new(count).color(theme::MUTED()).size(14.5));
        ui.add_space(18.0);
        for l in shown.into_iter().take(LOG_SHOWN) {
            let (r, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
            if ui.is_rect_visible(r) {
                let p = ui.painter();
                p.rect_filled(r, theme::r(6), theme::ELEV());
                p.rect_filled(Rect::from_min_max(pos2(r.left(), r.top() + 6.0), pos2(r.left() + 3.0, r.bottom() - 6.0)), theme::r(1), log_color(&l.what));
                let y = r.center().y;
                text(p, r.left() + 14.0, y, Align2::LEFT_CENTER, utc(l.at_ms), regular(12.0), theme::DIM());
                text(p, r.left() + 170.0, y, Align2::LEFT_CENTER, &l.by, theme::bold_font(12.5), theme::TEXT());
                let clip = p.with_clip_rect(Rect::from_min_max(pos2(r.left() + 300.0, r.top()), pos2(r.right() - 8.0, r.bottom())).intersect(p.clip_rect()));
                text(&clip, r.left() + 300.0, y, Align2::LEFT_CENTER, &l.what, regular(12.5), theme::TEXT());
            }
            ui.add_space(9.0);
        }
    }

    /// A word to everyone in the casino.
    fn cms_announce(&mut self, ui: &mut egui::Ui, s: &ServerView) {
        ui.label(RichText::new("Say something to everyone in Pyr Casino right now: it shows big at the top of their casino and as a toast.").color(theme::MUTED()).size(14.5));
        ui.add_space(12.0);
        let c = &mut self.poker.cms;
        ui.add(egui::TextEdit::multiline(&mut c.text).hint_text("Happy Friday! Pyr Friday Night starts at 10 PM…").desired_width(640.0).desired_rows(3).char_limit(280));
        ui.add_space(12.0);
        let ok = !c.text.trim().is_empty();
        if ui.add_enabled(ok, theme::primary_button("Send to everyone").min_size(vec2(0.0, 30.0))).clicked() {
            let text = std::mem::take(&mut c.text).trim().to_string();
            self.engine.poker(&s.key, ToServer::Cms { req: CmsReq::Announce { text } });
        }
        ui.add_space(6.0);
        ui.label(RichText::new("It goes in the log too.").color(theme::DIM()).size(12.0));
    }

    /// Who else may open the control room (the Owner of the Game lets them in, and out).
    fn cms_access(&mut self, ui: &mut egui::Ui, s: &ServerView) {
        ui.add_space(6.0);
        ui.label(RichText::new("Who can open the control room, besides the Owner of the Game:").color(theme::MUTED()).size(14.5));
        ui.add_space(18.0);
        let Some((owner, people)) = s.poker.cms.access.clone() else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Asking who's let in…").color(theme::MUTED()));
            });
            return;
        };
        ui.spacing_mut().item_spacing.y = 8.0;
        if people.is_empty() {
            ui.label(RichText::new("Nobody yet.").color(theme::MUTED()).size(14.5));
        }
        for (name, uid) in &people {
            ui.horizontal(|ui| {
                ui.label(RichText::new(name).strong().size(14.5));
                if owner && ui.small_button("Take out").clicked() {
                    self.engine.poker(&s.key, ToServer::Cms { req: CmsReq::SetAccess { uid: uid.clone(), on: false } });
                }
            });
        }
        if !owner {
            return;
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let pick = &mut self.poker.cms.grant;
            let who = pick.as_ref().and_then(|u| s.members.get(u.as_str())).map(|m| m.name.clone()).unwrap_or_else(|| "Pick someone".into());
            egui::ComboBox::from_id_salt("cms-grant").selected_text(who).width(220.0).show_ui(ui, |ui| {
                let mut ms: Vec<_> = s.members.iter().filter(|(u, _)| !s.is_bot(u) && !people.iter().any(|(_, p)| p == *u)).collect();
                ms.sort_by_key(|(_, m)| m.name.to_lowercase());
                for (u, m) in ms {
                    ui.selectable_value(pick, Some(u.to_string()), &m.name);
                }
            });
            if ui.add_enabled(pick.is_some(), theme::primary_button("Let them in").min_size(vec2(0.0, 30.0))).clicked()
                && let Some(uid) = pick.take()
            {
                self.engine.poker(&s.key, ToServer::Cms { req: CmsReq::SetAccess { uid, on: true } });
            }
        });
        ui.add_space(10.0);
        ui.label(RichText::new("They'll see everything here and can give and take chips (all of it in the log). Only you let people in or out.").color(theme::MUTED()).size(14.5));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The spreadsheet is 0.13.28's to the byte (its quotes and line ends).
    #[test]
    fn the_log_saves_as_0_13_28s() {
        let log = vec![
            StaffLog { at_ms: 1_794_379_045_000, by: "Olive".into(), what: "took 1,000 chips from Sweep (control room) (\u{201c}test take\u{201d}) · 247,153 → 246,153".into() },
            StaffLog { at_ms: 1_794_379_018_000, by: "Ol\"ive".into(), what: "gave 5,000 chips".into() },
        ];
        let csv = log_csv(&log);
        assert_eq!(
            csv,
            "When (UTC),By,What\r\n\"2026-11-11 06:37:25\",\"Olive\",\"took 1,000 chips from Sweep (control room) (\u{201c}test take\u{201d}) · 247,153 → 246,153\"\r\n\"2026-11-11 06:36:58\",\"Ol\"\"ive\",\"gave 5,000 chips\"\r\n"
        );
        assert_eq!(log_file_name("Test Camp", 1_794_379_060_000), "Pyr staff log - Test Camp - 2026-11-11 06-37-40.csv");
        assert_eq!(log_file_name("a/b: c?", 1_794_379_060_000), "Pyr staff log - a-b- c- - 2026-11-11 06-37-40.csv");
    }

    #[test]
    fn log_entries_are_colored_as_0_13_28s() {
        assert_eq!(log_color("gave Probe 50 chips"), GREEN);
        assert_eq!(log_color("took 0 chips from Probe"), TOOK);
        assert_eq!(log_color("took Probe out of the control room"), TOOK);
        assert_eq!(log_color("let Probe into the control room"), BLUE);
        assert_eq!(log_color("announced: \u{201c}probe\u{201d}"), BLUE);
    }
}
