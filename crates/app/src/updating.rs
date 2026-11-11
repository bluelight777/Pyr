//! The window while Pyr updates itself (0.13.29). Pyr used to close and leave nothing on screen
//! until the new version opened ("Pyr just disappears for a moment"); now its window shows
//! "Updating Pyr" the whole time, and closes once the new Pyr's window is up.
//!
//! How the two meet: before the installer starts, the updating Pyr leaves a file in its settings
//! folder (`update::mark_updating`) and lets go of the one-copy lock, so the new Pyr starts as the
//! only copy (instead of handing itself to this one). The new Pyr takes the file away once its
//! window is up (`update::shown`), and this one sees that and closes. If the update doesn't
//! install, or the new Pyr doesn't open, this screen says so and offers to open Pyr.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Rect, RichText, Sense, Stroke, pos2, vec2};

use crate::app::PyrApp;
use crate::theme;
use crate::update::Installing;

/// How long the new Pyr may take to open after it's installed before this screen says so.
const LATE: Duration = Duration::from_secs(45);

#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    /// The installer's running (on Linux, maybe asking for the password first).
    Installing,
    /// Installed: the new Pyr is opening (since when).
    Opening(Instant),
    /// The new Pyr's window is up: this one goes (since when).
    Done(Instant),
    /// Installed, but the new Pyr hasn't opened.
    Late,
    /// It didn't install (why).
    Failed(String),
}

pub struct Updating {
    pub version: String,
    pub installing: Option<Installing>,
    pub since: Instant,
    pub stage: Stage,
    dir: PathBuf,
    /// Still running everything else (Linux, while the password is asked for: this Pyr hasn't let
    /// go of anything yet, and carries on as it was if the update's cancelled).
    pub holding_on: bool,
    /// Development builds' preview (`PYR_DEV_UPDATING`): nothing's installed and nothing closes.
    preview: bool,
}

impl Updating {
    pub fn new(version: String, installing: Installing, dir: PathBuf, holding_on: bool) -> Updating {
        Updating { version, installing: Some(installing), since: Instant::now(), stage: Stage::Installing, dir, holding_on, preview: false }
    }

    /// Development: `PYR_DEV_UPDATING=installing|password|opening|late|failed` shows the screen
    /// as it looks at that point (nothing's installed, and it doesn't close). `handoff` (Linux)
    /// goes through an update without installing anything: "installed" after three seconds,
    /// this Pyr lets go and opens Pyr again, and closes once that one's window is up.
    pub fn preview(dir: PathBuf) -> Option<Updating> {
        let v = pyr_proto::env_var("DEV_UPDATING").filter(|v| !v.is_empty())?;
        let now = Instant::now();
        #[cfg(not(windows))]
        if v == "handoff" {
            crate::update::mark_updating(&dir, "0.14.1");
            let installing = Installing::pretend(Duration::from_secs(3));
            return Some(Updating::new("0.14.1".into(), installing, dir, true));
        }
        let (stage, holding_on) = match v.as_str() {
            "password" => (Stage::Installing, true),
            "opening" => (Stage::Opening(now), false),
            "late" => (Stage::Late, false),
            "failed" => (Stage::Failed("the installer stopped with code 2".into()), false),
            _ => (Stage::Installing, false),
        };
        Some(Updating { version: "0.14.1".into(), installing: None, since: now, stage, dir, holding_on, preview: true })
    }
}

impl PyrApp {
    /// A frame while updating: everything else waits.
    pub(crate) fn updating_frame(&mut self, ctx: &egui::Context) {
        let now = Instant::now();
        ctx.request_repaint_after(Duration::from_millis(33));
        // Closing the window now just goes: the installer carries on and opens the new Pyr. (On
        // Linux, while the update's being installed, this Pyr is the one that opens the new one,
        // so it stays.)
        if ctx.input(|i| i.viewport().close_requested()) {
            if self.updating.as_ref().is_some_and(|u| u.holding_on && !u.preview) {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else {
                std::process::exit(0);
            }
        }
        let Some(u) = self.updating.as_mut() else { return };
        if !u.preview {
            // How the installer's doing.
            if let Some(done) = u.installing.as_mut().and_then(|i| i.poll()) {
                u.installing = None;
                match done {
                    Ok(()) if u.holding_on => {
                        // (Linux: installed. Now let go of everything, and open the new Pyr.)
                        u.holding_on = false;
                        self.leave_for_update();
                        let u = self.updating.as_mut().expect("still updating");
                        u.stage = match crate::update::reopen() {
                            Ok(()) => Stage::Opening(now),
                            Err(e) => Stage::Failed(e),
                        };
                    }
                    Ok(()) => u.stage = Stage::Opening(now),
                    // (Still holding on: nothing was let go of, so Pyr just carries on.)
                    Err(e) if u.holding_on => {
                        self.updating = None;
                        crate::watchdog::not_quitting();
                        crate::update::not_updating(&self.engine.shared.config_dir);
                        self.engine.shared.state.write().toast(format!("Update failed: {e}"), true);
                        self.updater.dismiss_error(self.update_seen.clone());
                        return;
                    }
                    Err(e) => u.stage = Stage::Failed(e),
                }
            }
            let u = self.updating.as_mut().expect("still updating");
            // The new Pyr's window is up (it took the file away): this one goes.
            if matches!(u.stage, Stage::Installing | Stage::Opening(_) | Stage::Late) && !u.holding_on && !crate::update::still_updating(&u.dir) {
                u.stage = Stage::Done(now);
            }
            match u.stage {
                Stage::Opening(at) if now - at > LATE => u.stage = Stage::Late,
                Stage::Done(at) if now - at > Duration::from_millis(400) => {
                    tracing::info!("the new Pyr is open: closing this one");
                    std::process::exit(0);
                }
                _ => {}
            }
        }
        crate::titlebar::show(ctx, pyr_proto::APP_NAME, |_, x, _| x);
        let stage = self.updating.as_ref().map(|u| u.stage.clone()).unwrap_or(Stage::Installing);
        let holding_on = self.updating.as_ref().is_some_and(|u| u.holding_on);
        let version = self.updating.as_ref().map(|u| u.version.clone()).unwrap_or_default();
        let since = self.updating.as_ref().map(|u| u.since).unwrap_or(now);
        let mut open_pyr = false;
        let mut close = false;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::MAIN())).show(ctx, |ui| {
            let full = ui.max_rect();
            let p = ui.painter().clone();
            let t = (now - since).as_secs_f32();
            // An ember glow behind the flame, breathing slowly.
            let c = pos2(full.center().x, full.top() + full.height() * 0.40);
            let breath = 0.5 + 0.5 * (t * 1.6).sin();
            crate::casino::radial(&p, c, 200.0 * (1.0 + 0.05 * breath), Color32::from_rgba_unmultiplied(0xff, 0x74, 0x24, (46.0 + 16.0 * breath) as u8), Color32::TRANSPARENT);
            crate::casino::radial(&p, c, 100.0, Color32::from_rgba_unmultiplied(0xff, 0x9a, 0x3a, (30.0 + 12.0 * breath) as u8), Color32::TRANSPARENT);
            let tile = Rect::from_center_size(c, vec2(96.0, 96.0));
            p.rect_filled(tile, theme::r(28), Color32::from_rgb(0x24, 0x1d, 0x19));
            p.rect_stroke(tile, theme::r(28), Stroke::new(1.0_f32, Color32::from_rgb(0x3a, 0x2e, 0x26)), egui::StrokeKind::Inside);
            crate::icons::flame(&p, tile.shrink(19.0));
            let failed = matches!(stage, Stage::Failed(_));
            let late = matches!(stage, Stage::Late);
            let title = match &stage {
                Stage::Failed(_) => "The update didn't install".to_string(),
                Stage::Late => format!("{} is updated", pyr_proto::APP_NAME),
                Stage::Done(_) => format!("{} is up to date", pyr_proto::APP_NAME),
                _ if version.is_empty() => format!("Updating {}", pyr_proto::APP_NAME),
                _ => format!("Updating {} to {version}", pyr_proto::APP_NAME),
            };
            let y = tile.bottom() + 40.0;
            p.text(pos2(c.x, y), Align2::CENTER_CENTER, title, theme::bold_font(26.0), theme::TEXT());
            let line = match &stage {
                Stage::Installing if holding_on => "Your PC asks for your password to install it.".to_string(),
                Stage::Installing => format!("{} opens again by itself in a moment, back where you were.", pyr_proto::APP_NAME),
                Stage::Opening(_) | Stage::Done(_) => format!("Opening the new {}…", pyr_proto::APP_NAME),
                Stage::Late => format!("But the new {} hasn't opened yet. Open it yourself:", pyr_proto::APP_NAME),
                Stage::Failed(e) => format!("{}. {} is as it was.", capitalize(e), pyr_proto::APP_NAME),
            };
            let g = p.layout(line, theme::font(15.0), theme::MUTED(), (full.width() - 80.0).min(560.0));
            let gy = y + 30.0;
            p.galley(pos2(c.x - g.size().x / 2.0, gy), g.clone(), theme::MUTED());
            let below = gy + g.size().y + 26.0;
            if failed || late {
                // What to do: open Pyr (the new one, or this one as it was), or just close this.
                let w = 150.0;
                let open = Rect::from_min_size(pos2(c.x - w - 6.0, below), vec2(w, 40.0));
                let shut = Rect::from_min_size(pos2(c.x + 6.0, below), vec2(w, 40.0));
                if ui.put(open, egui::Button::new(RichText::new(format!("Open {}", pyr_proto::APP_NAME)).font(theme::bold_font(15.0)).color(theme::on_ember())).fill(theme::EMBER()).corner_radius(theme::r(10))).clicked() {
                    open_pyr = true;
                }
                if ui.put(shut, egui::Button::new(RichText::new("Close").font(theme::bold_font(15.0)).color(theme::TEXT())).fill(theme::ELEV()).corner_radius(theme::r(10))).clicked() {
                    close = true;
                }
            } else {
                // A bar with an ember running along it (how long an install takes isn't known).
                let bar = Rect::from_center_size(pos2(c.x, below + 6.0), vec2(260.0, 6.0));
                p.rect_filled(bar, theme::r(3), Color32::from_rgb(0x2a, 0x22, 0x1e));
                let run = 0.32;
                let u = ((t * 0.55) % (1.0 + run)) - run;
                let a = bar.left() + bar.width() * u.max(0.0);
                let b = bar.left() + bar.width() * (u + run).min(1.0);
                if b > a {
                    let seg = Rect::from_min_max(pos2(a, bar.top()), pos2(b, bar.bottom()));
                    p.rect_filled(seg.expand(3.0), theme::r(6), Color32::from_rgba_unmultiplied(0xff, 0x8a, 0x2a, 40));
                    p.rect_filled(seg, theme::r(3), theme::EMBER());
                }
                if matches!(stage, Stage::Installing) && !holding_on && t > 20.0 {
                    p.text(pos2(c.x, bar.bottom() + 26.0), Align2::CENTER_CENTER, "Still installing: on a busy PC it can take a minute.", theme::font(13.0), theme::DIM());
                }
            }
            let _ = ui.allocate_rect(full, Sense::hover());
        });
        if open_pyr {
            match crate::update::reopen() {
                Ok(()) => {
                    // (That Pyr takes the file away when it's up; this one goes then.)
                    if let Some(u) = self.updating.as_mut() {
                        u.stage = Stage::Opening(Instant::now());
                    }
                }
                Err(e) => self.engine.shared.state.write().toast(e, true),
            }
        }
        if close && !self.updating.as_ref().is_some_and(|u| u.preview) {
            std::process::exit(0);
        }
    }

    /// Lets go of everything for the new Pyr: the tray icon and programs, the servers and the
    /// call (saved first), and the one-copy lock.
    pub(crate) fn leave_for_update(&mut self) {
        self.ui.gif_sizes.save(true);
        crate::tray::prepare_exit();
        self.engine.shutdown();
        crate::instance::release();
    }
}

/// "the installer stopped" → "The installer stopped".
fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}
