//! The emoji picker, like Discord's: search, the server's own emoji, frequently used, every
//! emoji by category with a skin tone choice, and a preview of the one under the pointer.

use eframe::egui::{self, Color32, CornerRadius, Rect, RichText, Sense, Stroke, pos2, vec2};
use ingle_engine::state::AppState;

use crate::app::IngleApp;
use crate::emoji::{self, EmojiRef};
use crate::theme;

const CELL: f32 = 40.0;
const COLS: usize = 9;
const RECENT_MAX: usize = 27;

#[derive(Default)]
pub struct PickerState {
    pub search: String,
    /// Put the cursor in the search box (set when the picker opens).
    pub focus_search: bool,
    /// Section to scroll to (set by the category bar).
    jump: Option<usize>,
    /// The tone choice row is open.
    tones_open: bool,
}

/// One cell: what it shows, what picking it types, and its names for the preview.
struct Cell {
    e: EmojiRef,
    text: String,
    label: String,
}

/// How an emoji picked from the picker is written in a message.
pub fn custom_text(name: &str, id: &str, animated: bool) -> String {
    if animated { format!("<a:{name}:{id}>") } else { format!("<:{name}:{id}>") }
}

/// Group icons for the category bar.
const GROUP_ICONS: [&str; 9] = ["😀", "👋", "🐻", "🍔", "⚽", "🚗", "💡", "❤\u{fe0f}", "🏁"];
const TONE_HANDS: [&str; 6] = ["👏", "👏🏻", "👏🏼", "👏🏽", "👏🏾", "👏🏿"];

impl IngleApp {
    /// Draws the picker for server `key` (for its own emoji); returns what was picked.
    pub(crate) fn emoji_picker(&mut self, ui: &mut egui::Ui, st: &AppState, key: Option<&str>) -> Option<String> {
        let mut picked: Option<String> = None;
        let mut new_tone: Option<u8> = None;
        let tone = st.settings.emoji_tone.min(5);
        let server = key.and_then(|k| st.server(k));
        let customs: Vec<Cell> = server
            .map(|s| {
                s.info
                    .emojis
                    .iter()
                    .map(|c| Cell {
                        e: EmojiRef::Custom { name: c.name.clone(), id: c.id.clone(), animated: c.animated },
                        text: custom_text(&c.name, &c.id, c.animated),
                        label: format!(":{}:", c.name),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let data = emoji::data();
        let mut ps = std::mem::take(&mut self.ui.picker);
        let ctx = ui.ctx().clone();
        let key_s = key.map(|k| k.to_string());
        let mut custom_tex = |id: &str| -> Option<egui::TextureId> {
            let k = key_s.clone()?;
            self.custom_emoji_tex(&ctx, &k, id)
        };

        ui.set_width(CELL * COLS as f32 + 60.0);
        // Search and skin tone.
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut ps.search)
                    .hint_text("Find the perfect emoji")
                    .desired_width(CELL * COLS as f32 + 60.0 - 48.0)
                    .id(egui::Id::new("emoji-search")),
            );
            if ps.focus_search || (!r.has_focus() && ui.ctx().memory(|m| m.focused().is_none())) {
                r.request_focus();
                ps.focus_search = false;
            }
            let hand = EmojiRef::Unicode(emoji::cell_of(TONE_HANDS[tone as usize]).unwrap_or(0));
            let resp = emoji::widget(ui, 28.0, &hand, &mut custom_tex).on_hover_text("Skin tone");
            if resp.clicked() {
                ps.tones_open = !ps.tones_open;
            }
        });
        if ps.tones_open {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Skin tone").size(12.0).color(theme::MUTED));
                for (i, h) in TONE_HANDS.iter().enumerate() {
                    let e = EmojiRef::Unicode(emoji::cell_of(h).unwrap_or(0));
                    let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
                    if i as u8 == tone {
                        ui.painter().rect_filled(rect, CornerRadius::same(6), theme::HOVER);
                    }
                    emoji::paint(ui.painter(), rect.shrink(4.0), &e, &mut custom_tex);
                    if resp.clicked() {
                        new_tone = Some(i as u8);
                        ps.tones_open = false;
                    }
                }
            });
        }
        ui.add_space(4.0);

        // Sections: (title, cells).
        let q = ps.search.trim().trim_matches(':').to_lowercase();
        let mut sections: Vec<(String, Vec<Cell>)> = Vec::new();
        let unicode_cell = |it: &emoji::Item| -> Option<Cell> {
            let (text, _) = it.toned(tone);
            let cell = emoji::cell_of(text).or_else(|| emoji::cell_of(&it.e))?;
            let name = it.s.first().map(|s| format!(":{s}:")).unwrap_or_else(|| it.n.clone());
            Some(Cell { e: EmojiRef::Unicode(cell), text: text.to_string(), label: name })
        };
        if q.is_empty() {
            let recent: Vec<Cell> = st
                .settings
                .recent_emoji
                .iter()
                .filter_map(|t| {
                    let chars: Vec<char> = t.chars().collect();
                    match emoji::match_at(&chars, 0)? {
                        (_, EmojiRef::Custom { name, id, animated }) => customs
                            .iter()
                            .any(|c| matches!(&c.e, EmojiRef::Custom { id: i, .. } if *i == id))
                            .then(|| Cell { e: EmojiRef::Custom { name: name.clone(), id, animated }, text: t.clone(), label: format!(":{name}:") }),
                        (_, e) => Some(Cell { e, text: t.clone(), label: t.clone() }),
                    }
                })
                .collect();
            if !customs.is_empty() {
                let name = server.map(|s| s.name.clone()).unwrap_or_default();
                sections.push((name, customs));
            }
            if !recent.is_empty() {
                sections.push(("Frequently used".into(), recent));
            }
            for g in &data.groups {
                sections.push((g.name.clone(), g.items.iter().filter_map(unicode_cell).collect()));
            }
        } else {
            let mut found: Vec<Cell> = customs.into_iter().filter(|c| c.label.to_lowercase().contains(&q)).collect();
            let mut scored: Vec<(u8, Cell)> = Vec::new();
            for g in &data.groups {
                for it in &g.items {
                    let score = if it.s.iter().any(|s| s == &q) {
                        0
                    } else if it.s.iter().any(|s| s.starts_with(&q)) {
                        1
                    } else if it.n.to_lowercase().split(' ').any(|w| w.starts_with(&q)) {
                        2
                    } else if it.s.iter().any(|s| s.contains(&q)) || it.n.to_lowercase().contains(&q) {
                        3
                    } else {
                        continue;
                    };
                    if let Some(c) = unicode_cell(it) {
                        scored.push((score, c));
                    }
                }
            }
            scored.sort_by_key(|s| s.0);
            found.extend(scored.into_iter().map(|s| s.1));
            sections.push((format!("Results for \u{201c}{}\u{201d}", ps.search.trim()), found));
        }

        let mut hovered: Option<(EmojiRef, String)> = None;
        ui.horizontal_top(|ui| {
            // Category bar.
            if q.is_empty() {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    for (i, (title, cells)) in sections.iter().enumerate() {
                        let icon = if title == "Frequently used" {
                            None
                        } else if let Some(g) = data.groups.iter().position(|g| &g.name == title) {
                            emoji::cell_of(GROUP_ICONS[g]).map(EmojiRef::Unicode)
                        } else {
                            cells.first().map(|c| c.e.clone())
                        };
                        let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(6), theme::HOVER);
                        }
                        match &icon {
                            Some(e) => emoji::paint(ui.painter(), rect.shrink(6.0), e, &mut custom_tex),
                            None => crate::icons::clock(ui.painter(), rect.shrink(8.0), theme::MUTED),
                        }
                        if resp.on_hover_text(title).clicked() {
                            ps.jump = Some(i);
                        }
                    }
                });
            }
            egui::ScrollArea::vertical().id_salt("emoji-grid").max_height(330.0).min_scrolled_height(330.0).auto_shrink([false, false]).show(
                ui,
                |ui| ui.vertical(|ui| {
                    ui.set_width(CELL * COLS as f32);
                    for (i, (title, cells)) in sections.iter().enumerate() {
                        let h = ui.label(RichText::new(title.to_uppercase()).font(theme::bold_font(11.5)).color(theme::MUTED));
                        if ps.jump == Some(i) {
                            h.scroll_to_me(Some(egui::Align::TOP));
                            ps.jump = None;
                        }
                        if cells.is_empty() {
                            ui.label(RichText::new("No emoji match that.").color(theme::DIM));
                        }
                        for row in cells.chunks(COLS) {
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                for c in row {
                                    let (rect, resp) = ui.allocate_exact_size(vec2(CELL, CELL), Sense::click());
                                    if !ui.is_rect_visible(rect) {
                                        continue;
                                    }
                                    if resp.hovered() {
                                        ui.painter().rect_filled(rect, CornerRadius::same(6), theme::HOVER);
                                        hovered = Some((c.e.clone(), c.label.clone()));
                                    }
                                    emoji::paint(ui.painter(), rect.shrink(5.0), &c.e, &mut custom_tex);
                                    if resp.clicked() {
                                        picked = Some(c.text.clone());
                                    }
                                }
                            });
                        }
                        ui.add_space(6.0);
                    }
                }),
            );
        });
        // Preview of the one under the pointer.
        ui.separator();
        ui.horizontal(|ui| {
            ui.set_height(30.0);
            match &hovered {
                Some((e, label)) => {
                    emoji::widget(ui, 28.0, e, &mut custom_tex);
                    ui.label(RichText::new(label).font(theme::bold_font(14.0)).color(theme::TEXT));
                }
                None => {
                    ui.label(RichText::new("Pick an emoji").color(theme::DIM));
                }
            }
        });
        drop(custom_tex);
        if let Some(t) = new_tone {
            let mut s = st.settings.clone();
            s.emoji_tone = t;
            self.engine.update_settings(s);
        }
        if let Some(p) = &picked {
            let mut s = self.engine.state().settings.clone();
            s.recent_emoji.retain(|r| r != p);
            s.recent_emoji.insert(0, p.clone());
            s.recent_emoji.truncate(RECENT_MAX);
            self.engine.update_settings(s);
            ps.search.clear();
        }
        self.ui.picker = ps;
        picked
    }
}

/// A reaction pill: the emoji (in colour) and how many.
pub fn reaction_pill(
    ui: &mut egui::Ui,
    text: &str,
    count: usize,
    mine: bool,
    custom: &mut dyn FnMut(&str) -> Option<egui::TextureId>,
) -> egui::Response {
    let chars: Vec<char> = text.chars().collect();
    let e = emoji::match_at(&chars, 0).map(|m| m.1);
    let n = count.to_string();
    let galley = ui.painter().layout_no_wrap(n, theme::bold_font(13.0), if mine { theme::EMBER2 } else { theme::MUTED });
    let w = 8.0 + 20.0 + 6.0 + galley.size().x + 9.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(w, 28.0), Sense::click());
    let fill = if mine {
        Color32::from_rgb(0x3d, 0x26, 0x18)
    } else if resp.hovered() {
        theme::ELEV2
    } else {
        theme::ELEV
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), fill);
    if mine {
        ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0f32, theme::EMBER), egui::StrokeKind::Inside);
    } else if resp.hovered() {
        ui.painter().rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0f32, theme::LINE), egui::StrokeKind::Inside);
    }
    let er = Rect::from_min_size(pos2(rect.left() + 8.0, rect.center().y - 10.0), vec2(20.0, 20.0));
    match &e {
        Some(e) => emoji::paint(ui.painter(), er, e, custom),
        None => {
            ui.painter().text(er.center(), egui::Align2::CENTER_CENTER, text, theme::font(15.0), theme::TEXT);
        }
    }
    let gp = pos2(er.right() + 6.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(gp, galley, theme::MUTED);
    resp
}
