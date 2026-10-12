//! Framing a new picture before it's saved, like Discord's "Edit Image": drag to move it,
//! zoom with the slider or the mouse wheel, and the circle shows what people will see.
//! Used for your profile picture and for a server's icon.

use std::sync::Arc;

use eframe::egui::{self, Color32, CornerRadius, Rect, RichText, Sense, Stroke, pos2, vec2};

use crate::app::{Act, IngleApp};
use crate::theme;

/// What the framed picture becomes.
#[derive(Clone, Debug, PartialEq)]
pub enum CropTarget {
    Me,
    /// A server's icon (by server key).
    Server(String),
}

pub struct AvatarCrop {
    pub bytes: Arc<Vec<u8>>,
    pub target: CropTarget,
    /// Image size in pixels.
    w: f32,
    h: f32,
    /// 1 = the whole shorter side fits the circle; up to 5.
    zoom: f32,
    /// Centre of the circle, 0–1 across and down the image.
    cx: f32,
    cy: f32,
    key: String,
}

impl AvatarCrop {
    pub fn new(bytes: Vec<u8>, target: CropTarget) -> Result<AvatarCrop, String> {
        let img = image::load_from_memory(&bytes).map_err(|_| "That file isn't an image Ingle can read.".to_string())?;
        let key = format!("crop:{}", ingle_proto::release::sha256_hex(&bytes));
        Ok(AvatarCrop {
            bytes: Arc::new(bytes),
            target,
            w: img.width().max(1) as f32,
            h: img.height().max(1) as f32,
            zoom: 1.0,
            cx: 0.5,
            cy: 0.5,
            key,
        })
    }

    /// Side of the chosen square as a share of the image's shorter side.
    fn frac(&self) -> f32 {
        1.0 / self.zoom
    }

    /// Keep the square inside the picture.
    fn clamp(&mut self) {
        let side = self.w.min(self.h) * self.frac();
        let (hx, hy) = (side / 2.0 / self.w, side / 2.0 / self.h);
        self.cx = self.cx.clamp(hx, 1.0 - hx);
        self.cy = self.cy.clamp(hy, 1.0 - hy);
    }
}

const BOX: f32 = 320.0;
const CIRCLE: f32 = 260.0;

impl IngleApp {
    pub(crate) fn avatar_editor(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let Some(mut c) = self.ui.avatar_crop.take() else { return };
        let mut keep = true;
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("avatar-crop-dim")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(r, CornerRadius::ZERO, Color32::from_black_alpha(170));
            let _ = resp;
        });
        let title = if c.target == CropTarget::Me { "Edit Image" } else { "Edit Server Icon" };
        egui::Window::new(title)
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, 0.0))
            .frame(egui::Frame::new().fill(theme::SIDE).corner_radius(12).inner_margin(18).stroke(Stroke::new(1.0f32, theme::LINE)))
            .title_bar(false)
            .show(ctx, |ui| {
                ui.set_width(BOX);
                ui.label(RichText::new(title).font(theme::bold_font(18.0)).color(theme::TEXT));
                ui.add_space(10.0);
                let (area, resp) = ui.allocate_exact_size(vec2(BOX, BOX), Sense::drag());
                let p = ui.painter().with_clip_rect(area);
                p.rect_filled(area, CornerRadius::same(8), theme::STAGE);
                // How big an image pixel is on screen: the chosen square fills the circle.
                let side_px = c.w.min(c.h) * c.frac();
                let scale = CIRCLE / side_px;
                // Drag to move the picture; the mouse wheel zooms.
                if resp.dragged() {
                    let d = resp.drag_delta();
                    c.cx -= d.x / scale / c.w;
                    c.cy -= d.y / scale / c.h;
                }
                if resp.hovered() {
                    let wheel = ui.input(|i| i.smooth_scroll_delta.y);
                    if wheel != 0.0 {
                        c.zoom = (c.zoom * (1.0 + wheel * 0.002)).clamp(1.0, 5.0);
                    }
                }
                c.clamp();
                let scale = CIRCLE / (c.w.min(c.h) * c.frac());
                let size = vec2(c.w * scale, c.h * scale);
                let min = area.center() - vec2(c.cx * size.x, c.cy * size.y);
                let tex = self.images.get(ui.ctx(), &c.key, Some(c.bytes.clone()), 2048).frame(0.0).map(|f| f.0);
                match tex {
                    Some(t) => {
                        p.image(t, Rect::from_min_size(min, size), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    None => {
                        p.text(area.center(), egui::Align2::CENTER_CENTER, "Loading…", theme::font(14.0), theme::DIM);
                    }
                }
                // Darken everything outside the circle, then outline it.
                let r = CIRCLE / 2.0;
                let ring = BOX; // wide enough to cover the corners
                p.circle_stroke(area.center(), r + ring / 2.0, Stroke::new(ring, Color32::from_black_alpha(150)));
                p.circle_stroke(area.center(), r, Stroke::new(2.0f32, Color32::WHITE));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("🖼").size(13.0).color(theme::MUTED));
                    ui.spacing_mut().slider_width = BOX - 60.0;
                    ui.add(egui::Slider::new(&mut c.zoom, 1.0..=5.0).show_value(false));
                    ui.label(RichText::new("🖼").size(19.0).color(theme::MUTED));
                });
                ui.label(RichText::new("Drag to move, zoom with the slider or your mouse wheel.").size(12.0).color(theme::DIM));
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("Reset").clicked() {
                        c.zoom = 1.0;
                        c.cx = 0.5;
                        c.cy = 0.5;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(theme::primary_button("Apply")).clicked() {
                            c.clamp();
                            let (bytes, cx, cy, frac) = (c.bytes.clone(), c.cx, c.cy, c.frac());
                            acts.push(match &c.target {
                                CropTarget::Me => Act::SetAvatarCropped { bytes, cx, cy, frac },
                                CropTarget::Server(key) => Act::SetServerIcon { key: key.clone(), bytes: Some((bytes, cx, cy, frac)) },
                            });
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                });
            });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            keep = false;
        }
        if keep {
            self.ui.avatar_crop = Some(c);
        }
    }
}
