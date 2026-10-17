//! Ringtones: what calls ring with (Settings → Notifications), and someone's own (right-click
//! them → Ringtone): Pyr's own ring, any soundboard sound (built in, or one of your servers'),
//! or a sound file from this PC, picked and cut like a soundboard sound. The engine plays them
//! (`pyr_engine::ring`).

use eframe::egui::{self, Align, Layout, Rect, RichText, Sense, vec2};
use pyr_engine::config::Ringtone;
use pyr_engine::soundboard::{self as sb, SoundRef};
use pyr_engine::state::AppState;

use crate::app::{Act, Dialog, PyrApp};
use crate::theme;

/// What a ringtone's called (none: Pyr's own ring).
pub fn name_of(t: Option<&Ringtone>) -> String {
    t.map(|t| t.name.clone()).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Pyr ring".into())
}

impl PyrApp {
    /// The ringtones to choose from, in a menu: for everyone (`for_uid` none), or someone's
    /// own (then "the same as everyone" comes first). Choosing one plays it. True when one was
    /// chosen (or the file picker opened).
    pub(crate) fn ringtone_choices(&mut self, ui: &mut egui::Ui, st: &AppState, for_uid: Option<&str>, acts: &mut Vec<Act>) -> bool {
        ui.set_min_width(220.0);
        let current = match for_uid {
            Some(u) => st.settings.ringtones.get(u),
            None => st.settings.ringtone.as_ref(),
        };
        let uid = for_uid.map(str::to_string);
        let mut chose = false;
        let mut pick = |ui: &mut egui::Ui, label: &str, tone: Option<Ringtone>, on: bool, acts: &mut Vec<Act>| {
            if ui.add(egui::Button::selectable(on, label)).clicked() {
                // (What rings now: for someone set back to the same as everyone, that one.)
                let hear = tone.clone().or_else(|| if uid.is_some() { st.settings.ringtone.clone() } else { None });
                acts.push(Act::SetRingtone { uid: uid.clone(), tone });
                acts.push(Act::PreviewRingtone(hear));
                chose = true;
            }
        };
        match for_uid {
            Some(_) => pick(ui, &format!("Same as everyone ({})", name_of(st.settings.ringtone.as_ref())), None, current.is_none(), acts),
            None => pick(ui, "Pyr ring", None, current.is_none(), acts),
        }
        ui.separator();
        ui.label(RichText::new("SOUNDS").font(theme::bold_font(11.0)).color(theme::DIM()));
        let is = |r: &SoundRef| current.and_then(|c| c.sound.as_ref()).is_some_and(|s| s.same(r));
        for b in sb::BUILTINS {
            let r = b.sound_ref();
            let on = is(&r);
            pick(ui, b.name, Some(Ringtone { name: b.name.to_string(), sound: Some(r), file: String::new() }), on, acts);
        }
        // Each server's own sounds.
        for s in st.servers.iter().filter(|s| !s.info.sounds.is_empty()) {
            ui.menu_button(format!("{}'s sounds", s.name), |ui| {
                crate::menu::style(ui);
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for snd in &s.info.sounds {
                        let r = SoundRef { server: s.key.clone(), id: snd.id.clone(), name: snd.name.clone(), emoji: snd.emoji.clone(), volume: snd.volume };
                        let on = is(&r);
                        pick(ui, &snd.name, Some(Ringtone { name: snd.name.clone(), sound: Some(r), file: String::new() }), on, acts);
                    }
                });
            });
        }
        ui.separator();
        let file_on = current.is_some_and(|c| c.sound.is_none() && !c.file.is_empty());
        if file_on {
            if let Some(c) = current {
                pick(ui, &c.name, Some(c.clone()), true, acts);
            }
        }
        if ui.button("A sound file from your PC…").on_hover_text("An MP3, WAV, OGG, OPUS, FLAC or M4A: you choose up to 10 seconds of it").clicked() {
            self.ui.sb.editor = Some(crate::soundboard::SoundEditor::ringtone(for_uid.map(str::to_string)));
            self.ui.dialog = Some(Dialog::SoundEditor);
            chose = true;
        }
        chose
    }

    /// Settings → Notifications: the ringtone for everyone, to hear and change, in a card.
    pub(crate) fn ringtone_setting(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) {
        egui::Frame::new().fill(theme::SIDE()).corner_radius(10).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(24.0, 40.0), Sense::hover());
                crate::icons::bell(ui.painter(), Rect::from_center_size(r.center(), vec2(22.0, 22.0)), theme::MUTED());
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new("Ringtone").size(12.5).color(theme::MUTED()));
                    ui.label(RichText::new(name_of(st.settings.ringtone.as_ref())).font(theme::bold_font(15.0)).color(theme::TEXT()));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.menu_button("Change…", |ui| {
                        crate::menu::style(ui);
                        if self.ringtone_choices(ui, st, None, acts) {
                            ui.close();
                        }
                    });
                    if ui.button("▶  Play").clicked() {
                        acts.push(Act::PreviewRingtone(st.settings.ringtone.clone()));
                    }
                });
            });
        });
        let own = st.settings.ringtones.len();
        let note = match own {
            0 => "Calls ring with this. To give someone their own ringtone, right-click them and choose Ringtone.".to_string(),
            1 => "Calls ring with this, except from the 1 person you gave their own (right-click someone and choose Ringtone).".to_string(),
            n => format!("Calls ring with this, except from the {n} people you gave their own (right-click someone and choose Ringtone)."),
        };
        ui.add_space(2.0);
        ui.label(RichText::new(note).size(12.0).color(theme::DIM()));
        ui.add_space(4.0);
    }

    /// "Ringtone" in the menu from right-clicking someone: theirs to choose. True when one was
    /// chosen.
    pub(crate) fn ringtone_menu(&mut self, ui: &mut egui::Ui, st: &AppState, uid: &str, acts: &mut Vec<Act>) -> bool {
        let mut chose = false;
        let label = match st.settings.ringtones.get(uid) {
            Some(t) => format!("Ringtone: {}", name_of(Some(t))),
            None => "Ringtone".to_string(),
        };
        ui.menu_button(label, |ui| {
            crate::menu::style(ui);
            chose = self.ringtone_choices(ui, st, Some(uid), acts);
            if chose {
                ui.close();
            }
        });
        chose
    }
}
