//! The call view: video grid / spotlight, screen-share tiles, call controls.

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use ingle_engine::state::{AppState, ServerView, VideoKey};
use ingle_proto::media::track;

use crate::app::{Act, Dialog, IngleApp, avatar, icon_button};
use crate::{icons, theme};

/// Buttons under the call (Leave, a little wider, included).
const CALL_BUTTONS: f32 = 6.0;

#[derive(Clone)]
struct Tile {
    key: VideoKey,
    user_id: String,
    name: String,
    speaking: bool,
    muted: bool,
    deaf: bool,
    is_me: bool,
    screen: bool,
    /// Screen share we are (or could be) watching.
    watching: bool,
    has_camera: bool,
    volume: f32,
    /// Screen share that carries sound.
    has_sound: bool,
    stream_volume: f32,
    face: Option<egui::TextureId>,
    /// Who's watching this screen share (names).
    viewers: Vec<String>,
    /// I've muted their soundboard sounds.
    sb_muted: bool,
    /// I can server mute or deafen them (a moderator): the server, and whether they are now
    /// (`None`: that one isn't mine to do).
    moderate: Option<(String, Option<bool>, Option<bool>)>,
    /// The soundboard sound they just played, while it shows over them.
    pop: Option<Pop>,
}

/// A soundboard sound someone played, shown over their tile.
#[derive(Clone)]
struct Pop {
    emoji: String,
    name: String,
    /// Seconds since it was played.
    age: f32,
    /// The picture, for one of the server's own emoji.
    tex: Option<egui::TextureId>,
}

fn grid_layout(n: usize, area: Rect, gap: f32) -> Vec<Rect> {
    if n == 0 {
        return Vec::new();
    }
    let (w, h) = (area.width(), area.height());
    let mut best = (1usize, 0.0f32);
    for cols in 1..=n {
        let rows = n.div_ceil(cols);
        let tw = ((w - gap * (cols as f32 - 1.0)) / cols as f32).min((h - gap * (rows as f32 - 1.0)) / rows as f32 * 16.0 / 9.0);
        if tw > best.1 {
            best = (cols, tw);
        }
    }
    let (cols, tw) = best;
    let th = tw * 9.0 / 16.0;
    let rows = n.div_ceil(cols);
    let total_h = rows as f32 * th + (rows as f32 - 1.0) * gap;
    let mut out = Vec::new();
    for i in 0..n {
        let r = i / cols;
        let c = i % cols;
        let in_row = if r == rows - 1 { n - r * cols } else { cols };
        let row_w = in_row as f32 * tw + (in_row as f32 - 1.0) * gap;
        let x0 = area.center().x - row_w / 2.0;
        let y0 = area.center().y - total_h / 2.0;
        out.push(Rect::from_min_size(pos2(x0 + c as f32 * (tw + gap), y0 + r as f32 * (th + gap)), vec2(tw, th)));
    }
    out
}

/// One big tile and the others in a row under it. Like Discord, the row's tiles keep their
/// height and get narrower to fit a narrow window (down to nearly square-ish, then smaller).
fn focus_layout(tiles: &[Tile], focus: VideoKey, area: Rect, gap: f32) -> Vec<(usize, Rect)> {
    let others: Vec<usize> = (0..tiles.len()).filter(|i| tiles[*i].key != focus).collect();
    let n = others.len().max(1) as f32;
    let mut strip_h = (area.height() * 0.2).clamp(70.0, 150.0);
    // Room for each: full 16:9, or narrower (to 1:1) when the window is narrow...
    let room = (area.width() - gap * (n - 1.0)) / n;
    let mut tw = (strip_h * 16.0 / 9.0).min(room);
    if tw < strip_h {
        // ...and past that, the whole row shrinks.
        strip_h = tw.max(40.0);
        tw = tw.max(40.0);
    }
    let main = Rect::from_min_max(area.min, pos2(area.right(), area.bottom() - strip_h - gap));
    let mut v = vec![(tiles.iter().position(|t| t.key == focus).unwrap_or(0), main)];
    let total = n * tw + (n - 1.0) * gap;
    let x0 = area.center().x - total / 2.0;
    for (j, i) in others.into_iter().enumerate() {
        v.push((i, Rect::from_min_size(pos2(x0 + j as f32 * (tw + gap), area.bottom() - strip_h), vec2(tw, strip_h))));
    }
    v
}

/// Fit (contain) or fill (cover) an image of `size` into `rect`; returns (draw rect, uv rect).
fn fit(rect: Rect, size: egui::Vec2, cover: bool) -> (Rect, Rect) {
    let full_uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    if size.x <= 0.0 || size.y <= 0.0 {
        return (rect, full_uv);
    }
    let ra = rect.width() / rect.height();
    let ia = size.x / size.y;
    if cover {
        if ia > ra {
            let u = ra / ia;
            (rect, Rect::from_min_max(pos2(0.5 - u / 2.0, 0.0), pos2(0.5 + u / 2.0, 1.0)))
        } else {
            let v = ia / ra;
            (rect, Rect::from_min_max(pos2(0.0, 0.5 - v / 2.0), pos2(1.0, 0.5 + v / 2.0)))
        }
    } else if ia > ra {
        let h = rect.width() / ia;
        (Rect::from_center_size(rect.center(), vec2(rect.width(), h)), full_uv)
    } else {
        let w = rect.height() * ia;
        (Rect::from_center_size(rect.center(), vec2(w, rect.height())), full_uv)
    }
}

impl IngleApp {
    pub fn stage(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, st: &AppState, s: &ServerView, acts: &mut Vec<Act>) {
        let Some(call) = st.call.as_ref() else { return };
        let channel_name = s.channel(call.channel_id).map(|c| c.name.clone()).unwrap_or_default();

        // ---- header
        let topic = s.channel(call.channel_id).map(|c| c.topic.trim().to_string()).unwrap_or_default();
        egui::TopBottomPanel::top("stage-header")
            .exact_height(52.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::STAGE).inner_margin(egui::Margin::symmetric(16, 0)))
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    // Right to left first (chat, view, the encryption pill), so they never sit on
                    // top of the name and topic, which get what's left and are cut to fit.
                    let wide = ui.available_width() > 820.0;
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let open = self.ui.call_chat_open;
                        let unread = s.unread.contains(&call.channel_id);
                        let mentions = s.mentions.get(&call.channel_id).copied().unwrap_or(0);
                        let tip = match (open, unread) {
                            (true, _) => "Hide chat",
                            (false, true) => "Show chat (new messages)",
                            (false, false) => "Show chat",
                        };
                        let b = icon_button(ui, 32.0, tip, open, |p, r, c| icons::chat(p, r, c));
                        // The call's chat has messages you haven't seen: a badge on its button,
                        // like the channel list's.
                        if !open && (unread || mentions > 0) {
                            let c = b.rect.right_top() + vec2(-3.0, 4.0);
                            if mentions > 0 {
                                ui.painter().circle_filled(c, 8.0, theme::STAGE);
                                ui.painter().circle_filled(c, 6.5, theme::LIVE);
                                ui.painter().text(c, egui::Align2::CENTER_CENTER, mentions.min(9).to_string(), theme::bold_font(9.0), Color32::WHITE);
                            } else {
                                ui.painter().circle_filled(c, 5.0, theme::STAGE);
                                ui.painter().circle_filled(c, 3.5, theme::TEXT);
                            }
                        }
                        if b.clicked() {
                            self.ui.call_chat_open = !open;
                        }
                        ui.add_space(6.0);
                        let grid = self.ui.grid_view;
                        let tip = if grid { "Speaker view: whoever talks gets the big spot" } else { "Grid view: everyone the same size" };
                        if ui.button(if grid { "Speaker view" } else { "Grid view" }).on_hover_text(tip).clicked() {
                            self.ui.grid_view = !grid;
                            self.ui.focus = None;
                        }
                        ui.add_space(8.0);
                        let others = s.voice_in(call.channel_id).len().saturating_sub(1);
                        let (fill, stroke, text) = if call.e2ee_ready || others == 0 {
                            (
                                Color32::from_rgba_unmultiplied(0x6c, 0xc0, 0x70, 26),
                                Color32::from_rgba_unmultiplied(0x6c, 0xc0, 0x70, 70),
                                if wide { format!("End-to-end encrypted · call code {}", call.call_code) } else { call.call_code.clone() },
                            )
                        } else {
                            (theme::ELEV, theme::LINE, if wide { "Exchanging encryption keys…".to_string() } else { "Keys…".to_string() })
                        };
                        let resp = egui::Frame::new()
                            .fill(fill)
                            .stroke(Stroke::new(1.0f32, stroke))
                            .corner_radius(12)
                            .inner_margin(egui::Margin::symmetric(10, 3))
                            .show(ui, |ui| {
                                // The header lays out right to left: the text first, then the
                                // lock to its left.
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(text).size(12.5).color(Color32::from_rgb(0xbf, 0xe3, 0xbc)));
                                    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                                    icons::lock(ui.painter(), r, theme::ONLINE);
                                });
                            })
                            .response
                            .interact(Sense::click());
                        if resp
                            .on_hover_text(format!(
                                "End-to-end encrypted, call code {}. Everyone in the call should see the same code. If they do, no one (not even the server) can listen in. Click to copy.",
                                call.call_code
                            ))
                            .clicked()
                        {
                            acts.push(Act::Copy(call.call_code.clone()));
                        }
                        ui.add_space(8.0);
                        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                            icons::speaker(ui.painter(), r, theme::MUTED);
                            let w = ui.available_width();
                            crate::emoji::label_line(ui, &channel_name, theme::bold_font(15.5), theme::TEXT, w, &mut |_| None);
                            // The channel's topic, like a text channel's header.
                            if !topic.is_empty() && ui.available_width() > 60.0 {
                                let (sep, _) = ui.allocate_exact_size(vec2(1.0, 22.0), Sense::hover());
                                ui.painter().rect_filled(sep, 0, theme::LINE);
                                ui.add(egui::Label::new(RichText::new(&topic).size(13.0).color(theme::MUTED)).truncate()).on_hover_text(&topic);
                            }
                        });
                    });
                });
            });

        // ---- controls
        egui::TopBottomPanel::bottom("stage-controls")
            .exact_height(84.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::STAGE))
            .show_inside(ui, |ui| self.call_controls(ui, st, acts));

        // ---- tiles
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::STAGE)).show_inside(ui, |ui| {
            let mut tiles: Vec<Tile> = Vec::new();
            let mut parts: Vec<_> = s.voice_in(call.channel_id);
            parts.sort_by_key(|p| (p.session != call.session, p.session));
            for p in &parts {
                let me = p.session == call.session;
                let name = s.member_name(&p.user_id);
                let has_cam = p.tracks.iter().any(|t| t.id == track::CAMERA);
                let face = self.face(ui.ctx(), s, &p.user_id);
                let pop = crate::soundboard::recent_pop(call, p.session).map(|(x, age)| Pop {
                    emoji: x.emoji.clone(),
                    name: x.name.clone(),
                    age,
                    tex: crate::soundboard::custom_emoji_id(&x.emoji).and_then(|id| self.custom_emoji_tex(ui.ctx(), &s.key, &id)),
                });
                tiles.push(Tile {
                    key: VideoKey { session: p.session, track: track::CAMERA },
                    user_id: p.user_id.clone(),
                    name: name.clone(),
                    speaking: call.speaking.contains(&p.session),
                    muted: p.self_mute || p.server_mute,
                    deaf: p.self_deaf || p.server_deaf,
                    is_me: me,
                    screen: false,
                    watching: false,
                    has_camera: has_cam,
                    volume: st.user_volumes.get(&p.user_id).copied().unwrap_or(1.0),
                    has_sound: false,
                    stream_volume: 1.0,
                    face,
                    viewers: Vec::new(),
                    sb_muted: st.settings.soundboard_muted.contains(&p.user_id),
                    moderate: {
                        let (m, d) = crate::app::moderate(st, s, &p.user_id);
                        (m || d).then(|| (s.key.clone(), m.then_some(p.server_mute), d.then_some(p.server_deaf)))
                    },
                    pop,
                });
                if p.tracks.iter().any(|t| t.id == track::SCREEN) {
                    let key = VideoKey { session: p.session, track: track::SCREEN };
                    tiles.push(Tile {
                        key,
                        user_id: p.user_id.clone(),
                        name,
                        speaking: false,
                        muted: false,
                        deaf: false,
                        is_me: me,
                        screen: true,
                        watching: me || call.watching.contains(&key),
                        has_camera: false,
                        volume: st.user_volumes.get(&p.user_id).copied().unwrap_or(1.0),
                        has_sound: p.tracks.iter().any(|t| t.id == track::SCREEN_AUDIO),
                        stream_volume: st.stream_volumes.get(&p.user_id).copied().unwrap_or(1.0),
                        face: None,
                        viewers: p
                            .tracks
                            .iter()
                            .find(|t| t.id == track::SCREEN)
                            .map(|t| t.viewers.iter().filter_map(|v| s.voice.get(v)).map(|v| s.member_name(&v.user_id)).collect())
                            .unwrap_or_default(),
                        sb_muted: false,
                        moderate: None,
                        pop: None,
                    });
                }
            }
            if self.ui.focus.map(|f| !tiles.iter().any(|t| t.key == f)).unwrap_or(false) {
                self.ui.focus = None;
            }
            let focus = self.ui.focus.or_else(|| if self.ui.grid_view { None } else { self.follow_speaker(&tiles) });
            let area = ui.max_rect().shrink2(vec2(16.0, 8.0));
            let gap = 10.0;
            let rects: Vec<(usize, Rect)> = match focus {
                Some(f) if tiles.len() > 1 => focus_layout(&tiles, f, area, gap),
                _ => grid_layout(tiles.len(), area, gap).into_iter().enumerate().collect(),
            };
            for (i, rect) in rects {
                let t = tiles[i].clone();
                self.tile(ui, ctx, &t, rect, acts);
            }
            if tiles.len() == 1 {
                ui.painter().text(
                    pos2(area.center().x, area.bottom() - 14.0),
                    egui::Align2::CENTER_BOTTOM,
                    "You're the only one here. Invite a friend from the server menu.",
                    theme::font(13.0),
                    theme::DIM,
                );
            }
        });
    }

    /// The tile of whoever is talking (not you), kept for a moment after they stop so the view
    /// doesn't jump at every pause; `None` until someone talks.
    fn follow_speaker(&mut self, tiles: &[Tile]) -> Option<VideoKey> {
        let now = std::time::Instant::now();
        let current = self.ui.speaker_focus.filter(|(k, _)| tiles.iter().any(|t| t.key == *k));
        let speaking: Vec<VideoKey> = tiles.iter().filter(|t| t.speaking && !t.is_me && !t.screen).map(|t| t.key).collect();
        let next = match current {
            Some((k, _)) if speaking.contains(&k) => Some((k, now)),
            // Someone else talks: switch, unless the last switch was a moment ago.
            Some((k, at)) => match speaking.first() {
                Some(s) if now.duration_since(at) > std::time::Duration::from_millis(1500) => Some((*s, now)),
                _ => Some((k, at)),
            },
            None => speaking.first().map(|s| (*s, now)),
        };
        self.ui.speaker_focus = next;
        next.map(|(k, _)| k)
    }

    fn tile(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, t: &Tile, rect: Rect, acts: &mut Vec<Act>) {
        let resp = ui.allocate_rect(rect, Sense::click());
        let p = ui.painter_at(rect);
        // A watched stream's resolution and frame rate, in the corner left of its badges.
        let mut quality: Option<String> = None;
        p.rect_filled(rect, CornerRadius::same(12), theme::TILE);
        let tex = if t.screen && !t.watching { None } else if t.screen || t.has_camera { self.texture(ctx, t.key) } else { None };
        match tex {
            Some((id, size)) => {
                let (r, uv) = fit(rect, size, !t.screen);
                if t.screen {
                    p.rect_filled(rect, CornerRadius::same(12), Color32::BLACK);
                }
                let mut mesh = egui::Mesh::with_texture(id);
                mesh.add_rect_with_uv(r, uv, Color32::WHITE);
                p.add(egui::Shape::mesh(mesh));
                // Round the video's corners by painting over them with the stage colour.
                p.rect_stroke(rect.expand(6.0), CornerRadius::same(18), Stroke::new(12.0f32, theme::STAGE), StrokeKind::Inside);
                if t.screen && !t.is_me {
                    let fps = self.fps.get(&t.key).map(|f| f.2).unwrap_or(0.0);
                    quality = Some(format!("{}p · {:.0} fps", size.y as u32, fps));
                }
            }
            None => {
                if t.screen {
                    p.rect_filled(rect, CornerRadius::same(12), Color32::from_rgb(0x16, 0x12, 0x10));
                    // LIVE, who's sharing and Watch Stream, stacked in the room above the name
                    // label; a small tile (the row under the big one) keeps what fits.
                    let (top, bottom) = (rect.top() + 8.0, rect.bottom() - 40.0);
                    let room = bottom - top;
                    let show_badge = room >= 64.0;
                    let show_text = room >= 100.0 && rect.width() >= 240.0;
                    let btn_h = if room >= 100.0 { 34.0 } else { 30.0 };
                    let mut h = if t.is_me { 0.0 } else { btn_h };
                    if show_badge {
                        h += 30.0;
                    }
                    if show_text || t.is_me {
                        h += 26.0;
                    }
                    let x = rect.center().x;
                    let mut y = top + ((room - h) / 2.0).max(0.0);
                    if show_badge {
                        let badge = Rect::from_center_size(pos2(x, y + 11.0), vec2(48.0, 22.0));
                        p.rect_filled(badge, CornerRadius::same(5), theme::LIVE);
                        p.text(badge.center(), egui::Align2::CENTER_CENTER, "LIVE", theme::bold_font(12.0), Color32::WHITE);
                        y += 30.0;
                    }
                    if show_text || t.is_me {
                        p.text(pos2(x, y + 9.0), egui::Align2::CENTER_CENTER, format!("{} is sharing their screen", t.name), theme::font(14.0), theme::TEXT);
                        y += 26.0;
                    }
                    if !t.is_me {
                        let b = Rect::from_center_size(pos2(x, y + btn_h / 2.0), vec2(140f32.min(rect.width() - 16.0), btn_h));
                        let br = ui.put(b, theme::primary_button("Watch Stream"));
                        if br.clicked() {
                            acts.push(Act::Watch { session: t.key.session, track: track::SCREEN, on: true });
                            // Like Discord: the stream you start watching takes the big spot.
                            self.ui.focus = Some(t.key);
                        }
                    }
                } else {
                    let r = (rect.height().min(rect.width()) * 0.2).clamp(18.0, 64.0);
                    avatar(&p, rect.center(), r, &t.user_id, &t.name, t.speaking, theme::TILE, t.face);
                    if t.has_camera {
                        p.text(rect.center() + vec2(0.0, r + 16.0), egui::Align2::CENTER_CENTER, "starting video…", theme::font(11.5), theme::DIM);
                    }
                }
            }
        }
        // name label
        let label = if t.screen { format!("{}'s screen", t.name) } else if t.is_me { format!("{} (you)", t.name) } else { t.name.clone() };
        let galley = ui.fonts_mut(|f| f.layout_no_wrap(label, theme::bold_font(12.5), theme::TEXT));
        let icon_w = if t.muted || t.deaf { 18.0 } else { 0.0 };
        let lr = Rect::from_min_size(rect.left_bottom() + vec2(10.0, -34.0), vec2(galley.size().x + 16.0 + icon_w, 24.0));
        p.rect_filled(lr, CornerRadius::same(6), Color32::from_black_alpha(150));
        if t.deaf {
            icons::headphones(&p, Rect::from_min_size(lr.min + vec2(6.0, 4.0), vec2(16.0, 16.0)), theme::LIVE, true);
        } else if t.muted {
            icons::mic(&p, Rect::from_min_size(lr.min + vec2(6.0, 4.0), vec2(16.0, 16.0)), theme::LIVE, true);
        }
        p.galley(lr.min + vec2(8.0 + icon_w, 12.0 - galley.size().y / 2.0), galley, theme::TEXT);
        let mut right = rect.right() - 10.0;
        if t.screen && t.watching && !t.is_me {
            let b = Rect::from_min_size(pos2(right - 48.0, rect.top() + 10.0), vec2(48.0, 22.0));
            p.rect_filled(b, CornerRadius::same(5), theme::LIVE);
            p.text(b.center(), egui::Align2::CENTER_CENTER, "LIVE", theme::bold_font(12.0), Color32::WHITE);
            right = b.left() - 6.0;
        }
        if t.screen && !t.viewers.is_empty() {
            // How many are watching (who, on hover), like Discord.
            let n = t.viewers.len().to_string();
            let g = ui.painter().layout_no_wrap(n, theme::bold_font(12.0), Color32::WHITE);
            let b = Rect::from_min_size(pos2(right - (30.0 + g.size().x), rect.top() + 10.0), vec2(30.0 + g.size().x, 22.0));
            p.rect_filled(b, CornerRadius::same(5), Color32::from_black_alpha(170));
            icons::eye(&p, Rect::from_min_size(b.min + vec2(6.0, 4.0), vec2(16.0, 14.0)), Color32::WHITE);
            p.galley(pos2(b.left() + 25.0, b.center().y - g.size().y / 2.0), g, Color32::WHITE);
            let id = ui.id().with(("viewers", t.key.session));
            let names = t.viewers.join("\n");
            ui.interact(b, id, egui::Sense::hover()).on_hover_text(format!("Watching:\n{names}"));
            right = b.left() - 6.0;
        }
        if let Some(q) = quality {
            let g = ui.fonts_mut(|f| f.layout_no_wrap(q, theme::font(11.5), theme::MUTED));
            let b = Rect::from_min_size(pos2(right - g.size().x - 12.0, rect.top() + 10.0), vec2(g.size().x + 12.0, 22.0));
            p.rect_filled(b, CornerRadius::same(5), Color32::from_black_alpha(150));
            p.galley(b.min + vec2(6.0, 11.0 - g.size().y / 2.0), g, theme::MUTED);
        }
        if t.screen && t.has_sound && (t.watching || t.is_me) {
            // Little speaker next to the name: this stream has sound (muted look at 0%).
            let s = Rect::from_min_size(lr.right_top() + vec2(6.0, 0.0), vec2(24.0, 24.0));
            p.rect_filled(s, CornerRadius::same(6), Color32::from_black_alpha(150));
            let c = if t.stream_volume <= 0.001 && !t.is_me { theme::LIVE } else { theme::TEXT };
            icons::speaker(&p, s.shrink(4.0), c);
        }
        if t.speaking {
            p.rect_stroke(rect, CornerRadius::same(12), Stroke::new(3.0f32, theme::GLOW), StrokeKind::Inside);
        }
        // A soundboard sound they just played: its emoji pops up over them, and what it was
        // shows in the corner.
        if let Some(pop) = &t.pop {
            let face_r = (rect.height().min(rect.width()) * 0.2).clamp(18.0, 64.0);
            let size = (face_r * 1.1).clamp(22.0, 64.0);
            let at = rect.center() + vec2(face_r * 0.95, -face_r * 0.75);
            let tex = pop.tex;
            crate::soundboard::paint_pop(&p, at, size, &pop.emoji, pop.age, &mut |_| tex);
            let (_, _, alpha) = crate::soundboard::pop_motion(pop.age);
            let g = ui.fonts_mut(|f| f.layout_no_wrap(pop.name.clone(), theme::bold_font(12.0), theme::TEXT.gamma_multiply(alpha)));
            let b = Rect::from_min_size(rect.left_top() + vec2(10.0, 10.0), vec2(g.size().x + 34.0, 24.0));
            p.rect_filled(b, CornerRadius::same(6), Color32::from_black_alpha((150.0 * alpha) as u8));
            icons::soundboard(&p, Rect::from_min_size(b.min + vec2(7.0, 4.0), vec2(16.0, 16.0)), theme::EMBER2.gamma_multiply(alpha));
            p.galley(pos2(b.left() + 28.0, b.center().y - g.size().y / 2.0), g, theme::TEXT);
            ctx.request_repaint();
        }
        if resp.hovered() {
            p.rect_stroke(rect, CornerRadius::same(12), Stroke::new(1.0f32, theme::LINE), StrokeKind::Inside);
        }
        // Full screen: for a stream (or camera) that's showing video.
        let can_full = tex.is_some();
        let mut go_full = false;
        // (Checked by pointer position: once the pointer is on the button, the tile itself
        // no longer counts as hovered.)
        if can_full && ui.rect_contains_pointer(rect) {
            let b = Rect::from_min_size(rect.right_bottom() + vec2(-44.0, -44.0), vec2(34.0, 34.0));
            let br = ui.interact(b, ui.id().with(("full", t.key.session, t.key.track)), Sense::click());
            p.rect_filled(b, CornerRadius::same(6), Color32::from_black_alpha(if br.hovered() { 210 } else { 150 }));
            icons::fullscreen(&p, b.shrink(9.0), theme::TEXT, false);
            go_full |= br.on_hover_text("Full screen (double-click)").clicked();
        }
        if can_full && resp.double_clicked() {
            go_full = true;
            self.ui.pending_click = None;
        } else if resp.clicked() && !go_full {
            // Wait a moment before focusing: the click may be the start of a double-click.
            if can_full {
                self.ui.pending_click = Some((t.key, std::time::Instant::now()));
            } else {
                self.ui.focus = if self.ui.focus == Some(t.key) { None } else { Some(t.key) };
            }
        }
        if let Some((k, at)) = self.ui.pending_click {
            if k == t.key {
                let wait = std::time::Duration::from_millis(350);
                if at.elapsed() >= wait {
                    self.ui.pending_click = None;
                    self.ui.focus = if self.ui.focus == Some(k) { None } else { Some(k) };
                } else {
                    ctx.request_repaint_after(wait - at.elapsed());
                }
            }
        }
        if go_full {
            self.ui.fullscreen = Some(t.key);
        }
        let (uid, name, key) = (t.user_id.clone(), t.name.clone(), t.key);
        let is_me = t.is_me;
        let screen = t.screen;
        let watching = t.watching;
        let vol = t.volume;
        let (has_sound, svol) = (t.has_sound, t.stream_volume);
        let sb_muted = t.sb_muted;
        let moderate = t.moderate.clone().filter(|_| !screen);
        // (Ticking your own Mute or Deafen leaves the menu open, like Discord.)
        egui::Popup::context_menu(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            if is_me && !screen {
                // Your own Mute and Deafen, as in the voice channel list (deafened counts as
                // muted too), like Discord.
                let (muted, deafened) = self.engine.voice_prefs();
                let mut m = muted || deafened;
                if ui.checkbox(&mut m, "Mute").clicked() {
                    acts.push(Act::Mute(m));
                }
                let mut d = deafened;
                if ui.checkbox(&mut d, "Deafen").clicked() {
                    acts.push(Act::Deafen(d));
                }
                ui.separator();
            }
            if !is_me {
                // Mute this person (or just their stream's sound) on this PC, like Discord.
                let (now, label) = if screen { (svol, "Mute stream") } else { (vol, "Mute") };
                let muted = now <= 0.001;
                if ui.checkbox(&mut muted.clone(), if screen && !has_sound { "Mute stream (it has no sound now)" } else { label }).clicked() {
                    let slot = (uid.clone(), screen);
                    let volume = if muted {
                        self.ui.premute_volume.remove(&slot).filter(|v| *v > 0.001).unwrap_or(1.0)
                    } else {
                        self.ui.premute_volume.insert(slot, now);
                        0.0
                    };
                    acts.push(if screen { Act::StreamVolume { uid: uid.clone(), volume } } else { Act::Volume { uid: uid.clone(), volume } });
                    ui.close();
                }
                // Their soundboard sounds, for me only (everyone else still hears them).
                if !screen && ui.checkbox(&mut sb_muted.clone(), "Mute their soundboard").clicked() {
                    acts.push(Act::MuteSoundboard { uid: uid.clone(), muted: !sb_muted });
                    ui.close();
                }
                ui.separator();
            }
            if !is_me && screen {
                if has_sound {
                    ui.label(RichText::new("Stream volume").size(12.0).color(theme::MUTED));
                    let mut v = svol * 100.0;
                    if ui.add(egui::Slider::new(&mut v, 0.0..=200.0).suffix("%")).changed() {
                        acts.push(Act::StreamVolume { uid: uid.clone(), volume: v / 100.0 });
                    }
                } else {
                    ui.label(RichText::new("This stream has no sound").size(12.0).color(theme::DIM));
                }
            } else if !is_me {
                ui.label(RichText::new(format!("{name} volume")).size(12.0).color(theme::MUTED));
                let mut v = vol * 100.0;
                if ui.add(egui::Slider::new(&mut v, 0.0..=200.0).suffix("%")).changed() {
                    acts.push(Act::Volume { uid: uid.clone(), volume: v / 100.0 });
                }
            }
            // A moderator's Server Mute and Server Deafen, in red like Discord's.
            if let Some((server, mute, deaf)) = &moderate {
                if !is_me {
                    ui.separator();
                }
                if let Some(mut on) = *mute {
                    if ui.checkbox(&mut on, RichText::new("Server Mute").color(theme::LIVE)).clicked() {
                        acts.push(Act::ServerMute { key: server.clone(), uid: uid.clone(), muted: on });
                    }
                }
                if let Some(mut on) = *deaf {
                    if ui.checkbox(&mut on, RichText::new("Server Deafen").color(theme::LIVE)).clicked() {
                        acts.push(Act::ServerDeafen { key: server.clone(), uid: uid.clone(), deafened: on });
                    }
                }
                ui.separator();
            }
            if screen && watching && !is_me && ui.button("Stop watching").clicked() {
                acts.push(Act::Watch { session: key.session, track: track::SCREEN, on: false });
                ui.close();
            }
            if can_full && ui.button("Full screen").clicked() {
                self.ui.fullscreen = Some(key);
                ui.close();
            }
            if ui.button(if self.ui.focus == Some(key) { "Exit focus" } else { "Focus" }).clicked() {
                self.ui.focus = if self.ui.focus == Some(key) { None } else { Some(key) };
                ui.close();
            }
        });
    }

    #[allow(unused_assignments)]
    fn call_controls(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) {
        let Some(call) = st.call.as_ref() else { return };
        {
            let stt = &call.stats;
            let loss = if stt.audio_received + stt.audio_lost > 0 {
                stt.audio_lost as f32 * 100.0 / (stt.audio_received + stt.audio_lost) as f32
            } else {
                0.0
            };
            let mut lines = vec![format!("{} ms ping · jitter {:.0} ms", stt.rtt_ms, stt.jitter_ms), format!("buffer {} ms · loss {:.1}%", stt.buffer_ms, loss)];
            if call.camera_on || call.screen_on {
                lines.push(format!("sending {} kbps", stt.video_send_kbps));
            }
            if let Some(err) = &call.audio_error {
                lines = vec!["Audio problem:".into(), err.clone()];
            }
            // Keep the text clear of the call buttons (they're centred, `CALL_BUTTONS` wide);
            // anything longer is cut with "…" and shown in full on hover.
            let r = ui.max_rect();
            let buttons_left = r.center().x - (CALL_BUTTONS * 54.0 + (CALL_BUTTONS - 1.0) * 14.0 + 18.0) / 2.0;
            let width = (buttons_left - r.left() - 28.0).max(60.0);
            let colour = if call.audio_error.is_some() { theme::LIVE } else { theme::DIM };
            let mut cut = false;
            for (i, l) in lines.iter().enumerate() {
                let mut job = egui::text::LayoutJob::simple(l.clone(), theme::font(11.0), colour, width);
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                job.wrap.overflow_character = Some('…');
                let galley = ui.fonts_mut(|f| f.layout_job(job));
                cut |= galley.elided;
                let pos = pos2(r.left() + 16.0, r.center().y - 14.0 + i as f32 * 14.0 - galley.size().y / 2.0);
                ui.painter().galley(pos, galley, colour);
            }
            if cut {
                let area = Rect::from_min_size(pos2(r.left() + 16.0, r.center().y - 22.0), vec2(width, 16.0 + lines.len() as f32 * 14.0));
                ui.interact(area, ui.id().with("call-stats"), Sense::hover()).on_hover_text(lines.join("\n"));
            }
        }
        let (muted, deafened) = self.engine.voice_prefs();
        // Deafened is muted too (unmuting undeafens), like Discord.
        let muted = muted || deafened;
        let full = ui.max_rect();
        let n = CALL_BUTTONS;
        let size = 54.0;
        let gap = 14.0;
        let total = n * size + (n - 1.0) * gap + 18.0;
        let mut x = full.center().x - total / 2.0;
        let y = full.center().y - 4.0;
        let mut button = |ui: &mut egui::Ui, w: f32, fill: Color32, tip: &str, draw: &dyn Fn(&egui::Painter, Rect)| -> egui::Response {
            let r = Rect::from_center_size(pos2(x + w / 2.0, y), vec2(w, size));
            x += w + gap;
            let resp = ui.allocate_rect(r, Sense::click());
            let f = if resp.hovered() { fill.gamma_multiply(1.25) } else { fill };
            ui.painter().rect_filled(r, CornerRadius::same((size / 2.0) as u8), f);
            draw(ui.painter(), Rect::from_center_size(r.center(), vec2(24.0, 24.0)));
            resp.on_hover_text(tip)
        };
        let base = Color32::from_rgb(0x2a, 0x23, 0x1f);
        let on = Color32::from_rgb(0x3a, 0x2a, 0x20);
        let (server_muted, server_deafened) = Self::moderated_me(st);
        let mic_off = muted || server_muted;
        let tip = if server_muted { crate::app::SERVER_MUTED } else if muted { "Unmute" } else { "Mute" };
        if button(ui, size, if mic_off { Color32::from_rgb(0x4a, 0x1f, 0x1f) } else { base }, tip, &|p, r| {
            icons::mic(p, r, if mic_off { theme::LIVE } else { theme::TEXT }, mic_off)
        })
        .clicked()
        {
            if server_muted {
                acts.push(Act::Toast(crate::app::SERVER_MUTED.into(), false));
            } else {
                acts.push(Act::Mute(!muted));
            }
        }
        if button(ui, size, if call.camera_on { on } else { base }, if call.camera_on { "Turn off camera" } else { "Turn on camera" }, &|p, r| {
            icons::camera(p, r, if call.camera_on { theme::EMBER2 } else { theme::TEXT }, false)
        })
        .clicked()
        {
            acts.push(Act::Camera(!call.camera_on));
        }
        if button(ui, size, if call.screen_on { on } else { base }, if call.screen_on { "Stop sharing" } else { "Share your screen" }, &|p, r| {
            icons::screen(p, r, if call.screen_on { theme::EMBER2 } else { theme::TEXT })
        })
        .clicked()
        {
            if call.screen_on {
                acts.push(Act::StopScreen);
            } else {
                acts.push(Act::RefreshDevices);
                self.ui.dialog = Some(Dialog::ScreenPicker { selected: None });
            }
        }
        let sb_open = self.soundboard_open();
        let sb = button(ui, size, if sb_open { on } else { base }, "Soundboard", &|p, r| {
            icons::soundboard(p, r, if sb_open { theme::EMBER2 } else { theme::TEXT })
        });
        self.soundboard_anchor(crate::soundboard::FROM_CALL, sb.rect);
        if sb.clicked() {
            self.toggle_soundboard(st, crate::soundboard::FROM_CALL);
        }
        let deaf = deafened || server_deafened;
        let tip = if server_deafened { crate::app::SERVER_DEAFENED } else if deafened { "Undeafen" } else { "Deafen" };
        if button(ui, size, if deaf { Color32::from_rgb(0x4a, 0x1f, 0x1f) } else { base }, tip, &|p, r| {
            icons::headphones(p, r, if deaf { theme::LIVE } else { theme::TEXT }, deaf)
        })
        .clicked()
        {
            if server_deafened {
                acts.push(Act::Toast(crate::app::SERVER_DEAFENED.into(), false));
            } else {
                acts.push(Act::Deafen(!deafened));
            }
        }
        if button(ui, size + 18.0, theme::LIVE, "Leave call", &|p, r| icons::hangup(p, r, Color32::WHITE)).clicked() {
            acts.push(Act::Leave);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_row_shrinks_to_fit_narrow_windows() {
        let tile = |session: u32| Tile {
            key: VideoKey { session, track: track::CAMERA },
            user_id: String::new(),
            name: String::new(),
            speaking: false,
            muted: false,
            deaf: false,
            is_me: false,
            screen: false,
            watching: false,
            has_camera: false,
            volume: 1.0,
            has_sound: false,
            stream_volume: 1.0,
            face: None,
            viewers: Vec::new(),
            sb_muted: false,
            moderate: None,
            pop: None,
        };
        for (w, n) in [(1400.0, 4), (620.0, 4), (380.0, 4), (300.0, 6), (1000.0, 2)] {
            let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(w, 700.0));
            let tiles: Vec<Tile> = (1..=n).map(tile).collect();
            let rects = focus_layout(&tiles, tiles[0].key, area, 10.0);
            assert_eq!(rects.len(), n as usize);
            assert_eq!(rects[0].0, 0, "the focused tile comes first");
            assert!(rects[0].1.height() > rects[1].1.height() * 2.0, "{w}: the focused one is big");
            for (_, r) in &rects {
                assert!(area.expand(0.5).contains_rect(*r), "{w}x{n}: {r:?} spills out of {area:?}");
            }
            let row = &rects[1..];
            for pair in row.windows(2) {
                assert!(pair[0].1.right() <= pair[1].1.left(), "no overlap");
            }
            // Wide windows keep 16:9; narrow ones get skinnier (like Discord's).
            let r = row[0].1;
            if w >= 1400.0 {
                assert!((r.width() / r.height() - 16.0 / 9.0).abs() < 0.01);
            } else if w <= 620.0 {
                assert!(r.width() / r.height() < 16.0 / 9.0 - 0.1, "{w}: {r:?}");
            }
        }
    }

    #[test]
    fn grid_fits_inside_area() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 600.0));
        for n in 1..12 {
            let rects = grid_layout(n, area, 10.0);
            assert_eq!(rects.len(), n);
            for r in rects {
                assert!(area.expand(0.5).contains_rect(r), "{n}: {r:?}");
            }
        }
    }

    #[test]
    fn fit_contain_and_cover() {
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(160.0, 90.0));
        let (r, uv) = fit(rect, vec2(100.0, 100.0), false);
        assert!((r.width() - 90.0).abs() < 0.01 && uv == Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
        let (r, uv) = fit(rect, vec2(100.0, 100.0), true);
        assert_eq!(r, rect);
        assert!((uv.height() - 0.5625).abs() < 0.01);
    }
}

impl IngleApp {
    /// A stream or camera filling the whole screen. Returns false (and leaves full screen)
    /// when there's nothing to show, so the normal window draws instead.
    pub fn fullscreen_ui(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) -> bool {
        let Some(key) = self.ui.fullscreen else {
            if self.ui.fullscreen_on {
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
                self.ui.fullscreen_on = false;
            }
            return false;
        };
        let texture = st.call.as_ref().and_then(|_| self.texture(ctx, key));
        let exit_key = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        let Some((id, size)) = texture.filter(|_| !exit_key) else {
            self.ui.fullscreen = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            self.ui.fullscreen_on = false;
            return false;
        };
        if !self.ui.fullscreen_on {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            self.ui.fullscreen_on = true;
        }
        // Who it is, from the call's participants.
        let name = st
            .call
            .as_ref()
            .and_then(|c| st.server(&c.server_key).map(|s| (c, s)))
            .and_then(|(c, s)| s.voice_in(c.channel_id).into_iter().find(|p| p.session == key.session).map(|p| s.member_name(&p.user_id)))
            .unwrap_or_default();
        let mut leave = false;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::BLACK)).show(ctx, |ui| {
            let rect = ui.max_rect();
            let resp = ui.allocate_rect(rect, Sense::click());
            let (r, uv) = fit(rect, size, false);
            let mut mesh = egui::Mesh::with_texture(id);
            mesh.add_rect_with_uv(r, uv, Color32::WHITE);
            ui.painter().add(egui::Shape::mesh(mesh));
            // Controls appear while the mouse moves, then get out of the way.
            let moving = ctx.input(|i| i.pointer.time_since_last_movement() < 2.5);
            if moving {
                let bar = Rect::from_min_size(rect.left_top(), vec2(rect.width(), 52.0));
                ui.painter().rect_filled(bar, CornerRadius::ZERO, Color32::from_black_alpha(150));
                let what = if key.track == track::SCREEN { format!("{name}'s screen") } else { name.clone() };
                ui.painter().text(
                    bar.left_center() + vec2(20.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    format!("{what}  ·  Esc or double-click to exit full screen"),
                    theme::font(14.0),
                    theme::TEXT,
                );
                // Exit at the bottom right, like Discord's.
                let b = Rect::from_min_size(rect.right_bottom() + vec2(-58.0, -58.0), vec2(42.0, 42.0));
                let br = ui.interact(b, ui.id().with("exit-full"), Sense::click());
                ui.painter().rect_filled(b, CornerRadius::same(8), Color32::from_black_alpha(if br.hovered() { 220 } else { 160 }));
                icons::fullscreen(ui.painter(), b.shrink(11.0), theme::TEXT, true);
                leave |= br.on_hover_text("Exit full screen").clicked();
                ctx.request_repaint_after(std::time::Duration::from_millis(500));
            }
            leave |= resp.double_clicked();
            let _ = acts;
        });
        if leave {
            self.ui.fullscreen = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            self.ui.fullscreen_on = false;
        }
        true
    }
}
