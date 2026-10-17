//! A server's own GIFs (`GIFS_FEATURE`), like Discord's stickers: moving (or still) pictures
//! that people whose role has "Manage emoji, GIFs and sounds" add for everyone, and anyone
//! there sends from the GIF picker. A message sends one as `<g:name:id>` and it shows big, in
//! place of the text.

use std::time::Duration;

use eframe::egui::{self, Color32, CornerRadius, Rect, RichText, Sense, Stroke, pos2, vec2};
use ingle_engine::state::{AppState, ServerView};
use ingle_proto::control::{GIFS_FEATURE, MAX_SERVER_GIFS, ServerGif, perm, server_gif_token, server_gif_tokens};

use crate::app::{Act, IngleApp};
use crate::theme;

/// Whether I can add and change server `s`'s GIFs.
pub fn can_manage(s: &ServerView) -> bool {
    s.has(GIFS_FEATURE) && s.can(perm::MANAGE_EMOJI) && s.status.is_connected()
}

/// A message's text with the server GIFs it sends taken out (they show as pictures), and them.
pub fn split(content: &str) -> (String, Vec<(String, String)>) {
    let gifs = server_gif_tokens(content);
    let mut text = content.to_string();
    for (name, id) in &gifs {
        text = text.replace(&server_gif_token(name, id), "");
        // (A token written by hand, with a name the tokens would have cleaned.)
        text = text.replace(&format!("<g:{name}:{id}>"), "");
    }
    (text.trim().to_string(), gifs)
}

/// A file name as a GIF's name: "party_cat.gif" → "party cat".
pub fn name_from(file: &str) -> String {
    let stem = std::path::Path::new(file).file_stem().and_then(|s| s.to_str()).unwrap_or("GIF");
    let n: String = stem.replace(['_', '-'], " ").chars().filter(|c| !matches!(c, '<' | '>' | ':')).take(ingle_proto::control::MAX_GIF_NAME).collect();
    if n.trim().is_empty() { "GIF".into() } else { n.trim().to_string() }
}

impl IngleApp {
    /// One of server `s`'s GIFs in `r`: moving (when `plays`), cut to fill, rounded.
    fn paint_server_gif(&mut self, ui: &egui::Ui, s: &ServerView, id: &str, r: Rect, plays: bool, fill: bool) -> Option<egui::Vec2> {
        // (Only the server's own are fetched, never a file a hand-typed <g:…> points at.)
        let bytes = match s.info.gifs.iter().any(|g| g.id == id).then(|| self.engine.file(&s.key, id)) {
            Some(ingle_engine::files::FileState::Ready(b)) => Some(b),
            _ => None,
        };
        let t = if plays && self.ui.anim_on { self.ui.anim_clock } else { 0.0 };
        let tex = self.images.get(ui.ctx(), &format!("sgif:{id}"), bytes, 480);
        let moving = tex.frame_count() > 1;
        match tex.frame(t) {
            Some((tex, size, next)) => {
                let uv = if fill { crate::chat::cover_uv(r, size) } else { Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)) };
                ui.painter().add(egui::epaint::RectShape::filled(r, CornerRadius::same(8), Color32::WHITE).with_texture(tex, uv));
                if let (Some(n), true) = (next, plays && self.ui.anim_on) {
                    ui.ctx().request_repaint_after(n);
                }
                if moving && !plays {
                    crate::chat::gif_badge(ui.painter(), r);
                }
                Some(size)
            }
            None => {
                ui.painter().rect_filled(r, CornerRadius::same(8), theme::ELEV());
                ui.ctx().request_repaint_after(Duration::from_millis(100));
                None
            }
        }
    }

    /// A server GIF sent in a message: big, like Discord's stickers (or its name, if it's gone).
    pub(crate) fn server_gif_embed(&mut self, ui: &mut egui::Ui, s: &ServerView, name: &str, id: &str) {
        let known = s.info.gifs.iter().find(|g| g.id == id).map(|g| (g.width, g.height));
        let (w, h) = known.filter(|(w, h)| *w > 0 && *h > 0).map(|(w, h)| (w as f32, h as f32)).unwrap_or((240.0, 180.0));
        let scale = (300.0 / w).min(240.0 / h).min(2.0);
        let size = vec2(w * scale, h * scale);
        // Not (or no longer) one of the server's: just its name, unless it was shown already.
        if known.is_none() && !self.images.has(&format!("sgif:{id}")) {
            ui.label(RichText::new(format!("GIF: {name} (not on this server)")).italics().color(theme::DIM()));
            return;
        }
        let (r, resp) = ui.allocate_exact_size(size, Sense::hover());
        let pointed = ui.rect_contains_pointer(r);
        let plays = match self.ui.play_gifs.as_str() {
            "never" => false,
            "hover" => pointed,
            _ => true,
        };
        self.paint_server_gif(ui, s, id, r, plays, false);
        resp.on_hover_text(format!("{name} · a GIF for everyone on {}", s.name));
    }

    /// The GIF picker's tab of server `s`'s GIFs. Returns what to send (its token).
    pub(crate) fn server_gifs_tab(&mut self, ui: &mut egui::Ui, s: &ServerView, acts: &mut Vec<Act>) -> Option<String> {
        let mut send = None;
        let manage = can_manage(s);
        ui.horizontal(|ui| {
            ui.label(RichText::new("GIFS FOR EVERYONE HERE").font(theme::bold_font(11.0)).color(theme::DIM()));
            if manage {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("+ Add GIFs…").on_hover_text("GIF, WebP or PNG (moving or still), for everyone here").clicked() {
                        crate::images::pick_async(crate::images::PickFor::ServerGif(s.key.clone()), true, ui.ctx());
                    }
                });
            }
        });
        ui.add_space(4.0);
        if s.info.gifs.is_empty() {
            let text = if manage {
                "No GIFs here yet. Add some for everyone with + Add GIFs…, or open a picture someone sent and click Add to the server's GIFs."
            } else {
                "No GIFs here yet. People whose role has Manage emoji, GIFs and sounds can add some for everyone."
            };
            ui.label(RichText::new(text).color(theme::MUTED()));
            return None;
        }
        const GAP: f32 = 6.0;
        let col_w = (crate::chat::PICKER_W - GAP) / 2.0;
        let gifs: Vec<ServerGif> = s.info.gifs.clone();
        egui::ScrollArea::vertical().id_salt("server-gifs").max_height(340.0).auto_shrink([false, true]).show(ui, |ui| {
            // Two columns, each GIF at its own height, each going under the shorter column.
            let mut heights = [0.0f32; 2];
            let places: Vec<(usize, f32, f32)> = gifs
                .iter()
                .map(|g| {
                    let h = if g.width > 0 && g.height > 0 { (col_w * g.height as f32 / g.width as f32).clamp(60.0, 260.0) } else { col_w * 0.75 };
                    let c = if heights[0] <= heights[1] { 0 } else { 1 };
                    let y = heights[c];
                    heights[c] += h + GAP;
                    (c, y, h)
                })
                .collect();
            let total = heights[0].max(heights[1]);
            let (area, _) = ui.allocate_exact_size(vec2(crate::chat::PICKER_W, total), Sense::hover());
            for (g, &(c, y, h)) in gifs.iter().zip(&places) {
                let r = Rect::from_min_size(area.min + vec2(c as f32 * (col_w + GAP), y), vec2(col_w, h));
                if !ui.is_rect_visible(r) {
                    continue;
                }
                let resp = ui.interact(r, ui.id().with(("server-gif", &g.id)), Sense::click());
                self.paint_server_gif(ui, s, &g.id, r, true, true);
                if resp.hovered() {
                    ui.painter().rect_stroke(r, CornerRadius::same(8), Stroke::new(2.0f32, theme::EMBER()), egui::StrokeKind::Inside);
                }
                let resp = resp.on_hover_text(&g.name);
                if resp.clicked() {
                    send = Some(server_gif_token(&g.name, &g.id));
                }
                if manage {
                    resp.context_menu(|ui| {
                        crate::menu::style(ui);
                        if crate::menu::danger(ui, "Remove from the server").clicked() {
                            acts.push(Act::RemoveServerGif { key: s.key.clone(), id: g.id.clone() });
                            ui.close();
                        }
                    });
                }
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Click one to send it · everyone here has these").size(11.0).color(theme::DIM()));
        send
    }

    /// Server Settings → GIFs: the server's GIFs, to add, rename and remove.
    pub(crate) fn gifs_editor(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, acts: &mut Vec<Act>) {
        let _ = st;
        ui.label(
            RichText::new(format!(
                "Up to {MAX_SERVER_GIFS} GIFs everyone here can send from the GIF picker, like Discord's stickers. GIF, WebP or PNG, \
                 moving or still, up to {} MB each.",
                crate::images::upload_limit(s) >> 20
            ))
            .color(theme::MUTED()),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add(theme::primary_button("Add GIFs…")).clicked() {
                crate::images::pick_async(crate::images::PickFor::ServerGif(s.key.clone()), true, ui.ctx());
            }
            ui.label(RichText::new(format!("{} of {MAX_SERVER_GIFS} used", s.info.gifs.len())).size(12.0).color(theme::DIM()));
        });
        ui.add_space(8.0);
        if s.info.gifs.is_empty() {
            ui.label(RichText::new("No GIFs yet.").color(theme::DIM()));
            return;
        }
        let gifs = s.info.gifs.clone();
        egui::ScrollArea::vertical().max_height(380.0).show(ui, |ui| {
            egui::Grid::new("gif-list").num_columns(4).spacing(vec2(12.0, 8.0)).striped(true).show(ui, |ui| {
                ui.label(RichText::new("GIF").font(theme::bold_font(11.0)).color(theme::DIM()));
                ui.label(RichText::new("NAME").font(theme::bold_font(11.0)).color(theme::DIM()));
                ui.label(RichText::new("ADDED BY").font(theme::bold_font(11.0)).color(theme::DIM()));
                ui.label("");
                ui.end_row();
                for g in &gifs {
                    let (r, _) = ui.allocate_exact_size(vec2(72.0, 54.0), Sense::hover());
                    self.paint_server_gif(ui, s, &g.id, r, true, true);
                    let mut name = self.ui.emoji_names.get(&g.id).cloned().unwrap_or_else(|| g.name.clone());
                    let resp = ui.add_sized([200.0, 26.0], egui::TextEdit::singleline(&mut name).char_limit(ingle_proto::control::MAX_GIF_NAME));
                    if resp.changed() {
                        self.ui.emoji_names.insert(g.id.clone(), name.clone());
                    }
                    if resp.lost_focus() {
                        let n = name.trim().to_string();
                        if !n.is_empty() && n != g.name {
                            acts.push(Act::RenameServerGif { key: s.key.clone(), id: g.id.clone(), name: n });
                        }
                        self.ui.emoji_names.remove(&g.id);
                    }
                    ui.label(RichText::new(s.member_name(&g.by)).color(theme::MUTED()));
                    if crate::app::icon_button(ui, 26.0, "Remove", false, |p, r, _| crate::icons::trash(p, r, theme::LIVE())).clicked() {
                        acts.push(Act::RemoveServerGif { key: s.key.clone(), id: g.id.clone() });
                    }
                    ui.end_row();
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_of_server_gifs_shows_them_not_their_tokens() {
        let t = server_gif_token("party cat", "abc123");
        let (text, gifs) = split(&format!("look {t}"));
        assert_eq!(text, "look");
        assert_eq!(gifs, vec![("party cat".to_string(), "abc123".to_string())]);
        assert_eq!(split(&t).0, "");
        assert_eq!(name_from("Party_Cat-2.gif"), "Party Cat 2");
        assert_eq!(name_from("<:>.gif"), "GIF");
    }
}
