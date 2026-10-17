//! Pointing at a server in the list (like Discord): a card beside it with its name, what's new
//! there, and who's in its voice channels, faces and all. And a little speaker on the icons
//! of servers where people are talking right now.

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, Sense, Stroke, pos2, vec2};
use pyr_engine::state::{AppState, ConnStatus, ServerView};
use pyr_proto::control::{ChannelKind, VoiceParticipant};
use pyr_proto::media::track;

use crate::app::{PyrApp, avatar};
use crate::{icons, theme};

/// Faces shown in a voice channel's row (the rest are "+n").
const FACES: usize = 8;

/// A server's voice channels with people in them, in the order they're listed: (channel's
/// name, who's there).
pub fn busy_voice(s: &ServerView) -> Vec<(String, Vec<&VoiceParticipant>)> {
    let mut chans: Vec<_> = s.channels.iter().filter(|c| c.kind == ChannelKind::Voice).collect();
    chans.sort_by_key(|c| (c.position, c.id));
    chans
        .into_iter()
        .filter_map(|c| {
            let mut here = s.voice_in(c.id);
            here.sort_by_key(|p| p.session);
            (!here.is_empty()).then(|| (c.name.clone(), here))
        })
        .collect()
}

/// The little speaker on a server's icon while people are in its voice channels.
pub fn voice_badge(p: &egui::Painter, icon: Rect, mine: bool) {
    let c = icon.right_top() + vec2(-5.0, 5.0);
    p.circle_filled(c, 10.0, theme::RAIL());
    p.circle_filled(c, 8.5, if mine { theme::ONLINE() } else { theme::mix(theme::ONLINE(), theme::RAIL(), 0.35) });
    icons::speaker(p, Rect::from_center_size(c + vec2(0.5, 0.0), vec2(10.0, 10.0)), Color32::WHITE);
}

impl PyrApp {
    /// The card beside server `s`'s icon (at `icon`) while it's pointed at.
    pub(crate) fn server_card(&mut self, ctx: &egui::Context, st: &AppState, s: &ServerView, icon: Rect) {
        let busy = busy_voice(s);
        let in_my_call = st.call.as_ref().filter(|c| c.server_key == s.key);
        // What's new: the first channel with unread messages (not direct messages).
        let mut unread: Vec<_> = s.channels.iter().filter(|c| c.kind == ChannelKind::Text && s.unread.contains(&c.id)).collect();
        unread.sort_by_key(|c| (c.position, c.id));
        let mentions: u32 = s.mentions.iter().filter(|(c, _)| !s.is_private(**c)).map(|(_, n)| n).sum();
        let status = match &s.status {
            ConnStatus::Connected => None,
            ConnStatus::Failed(e) => Some((format!("Can't connect: {e}"), theme::LIVE())),
            _ => Some(("Connecting…".to_string(), theme::DIM())),
        };
        let at = pos2(icon.right() + 14.0, icon.center().y);
        let area = egui::Area::new(egui::Id::new(("server-card", &s.key)))
            .order(egui::Order::Tooltip)
            .interactable(false)
            .pivot(Align2::LEFT_CENTER)
            .fixed_pos(at)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme::ELEV())
                    .stroke(Stroke::new(1.0, theme::LINE()))
                    .corner_radius(CornerRadius::same(10))
                    .shadow(egui::epaint::Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(110) })
                    .inner_margin(egui::Margin { left: 12, right: 14, top: 10, bottom: 10 })
                    .show(ui, |ui| {
                        ui.set_max_width(320.0);
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        // Its icon and name.
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                            match self.server_icon(ui.ctx(), s) {
                                Some(t) => {
                                    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                                    ui.painter().add(egui::epaint::RectShape::filled(r, CornerRadius::same(6), Color32::WHITE).with_texture(t, uv));
                                }
                                None => {
                                    ui.painter().rect_filled(r, CornerRadius::same(6), theme::EMBER());
                                    let initial: String = s.name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                                    ui.painter().text(r.center(), Align2::CENTER_CENTER, initial, theme::bold_font(11.0), theme::on_ember());
                                }
                            }
                            crate::emoji::label(ui, &s.name, theme::bold_font(15.0), theme::TEXT(), Some(280.0), &mut |_| None);
                        });
                        if let Some((text, color)) = &status {
                            ui.label(egui::RichText::new(text).size(12.5).color(*color));
                        }
                        // What's new.
                        let news = match (mentions, unread.first()) {
                            (0, Some(c)) => Some(format!("New activity in #{}", c.name)),
                            (1, _) => Some("1 mention".to_string()),
                            (n, _) if n > 1 => Some(format!("{n} mentions")),
                            _ => None,
                        };
                        if let Some(n) = news {
                            ui.label(egui::RichText::new(n).size(12.5).color(theme::MUTED()));
                        }
                        // Who's in voice: a row for each channel with people in it.
                        for (name, here) in &busy {
                            let mine = in_my_call.is_some_and(|c| here.iter().any(|p| p.session == c.session));
                            let live = here.iter().any(|p| p.tracks.iter().any(|t| t.id == track::SCREEN));
                            ui.horizontal(|ui| {
                                let (r, _) = ui.allocate_exact_size(vec2(18.0, 22.0), Sense::hover());
                                icons::speaker(ui.painter(), Rect::from_center_size(r.center(), vec2(16.0, 16.0)), if mine { theme::ONLINE() } else { theme::MUTED() });
                                crate::emoji::label(ui, name, theme::font(12.5), theme::MUTED(), Some(110.0), &mut |_| None);
                                // Their faces, a little apart (talking ones lit, in my call).
                                let n = here.len().min(FACES);
                                let (row, _) = ui.allocate_exact_size(vec2(n as f32 * 25.0 + if here.len() > FACES { 28.0 } else { 0.0 }, 22.0), Sense::hover());
                                for (i, p) in here.iter().take(FACES).enumerate() {
                                    let c = pos2(row.left() + 11.0 + i as f32 * 25.0, row.center().y);
                                    let talking = in_my_call.is_some_and(|call| call.speaking.contains(&p.session));
                                    let face = self.face(ui.ctx(), s, &p.user_id);
                                    avatar(ui.painter(), c, 10.0, &p.user_id, &s.member_name(&p.user_id), talking, theme::ELEV(), face);
                                }
                                if here.len() > FACES {
                                    let c = pos2(row.left() + FACES as f32 * 25.0 + 13.0, row.center().y);
                                    ui.painter().circle_filled(c, 11.0, theme::ELEV2());
                                    ui.painter().text(c, Align2::CENTER_CENTER, format!("+{}", here.len() - FACES), theme::bold_font(10.0), theme::TEXT());
                                }
                                if live {
                                    let (b, _) = ui.allocate_exact_size(vec2(34.0, 16.0), Sense::hover());
                                    ui.painter().rect_filled(b, CornerRadius::same(4), theme::LIVE());
                                    ui.painter().text(b.center(), Align2::CENTER_CENTER, "LIVE", theme::bold_font(9.5), Color32::WHITE);
                                }
                            });
                        }
                    });
            });
        // A little point on its left, towards the icon.
        let card = area.response.rect;
        let y = icon.center().y.clamp(card.top() + 12.0, card.bottom() - 12.0);
        let tip = pos2(card.left() - 6.0, y);
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::new(("server-card-point", &s.key))));
        p.add(egui::Shape::convex_polygon(vec![tip, pos2(card.left() + 1.0, y - 7.0), pos2(card.left() + 1.0, y + 7.0)], theme::ELEV(), Stroke::NONE));
        p.line_segment([tip, pos2(card.left(), y - 7.0)], Stroke::new(1.0, theme::LINE()));
        p.line_segment([tip, pos2(card.left(), y + 7.0)], Stroke::new(1.0, theme::LINE()));
    }
}
