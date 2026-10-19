#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Pyr desktop app.

mod app;
mod appearance;
mod autostart;
mod calls;
mod chat;
mod connector;
mod create;
mod dialogs;
mod icons;
mod avatar_edit;
mod emoji;
mod emoji_picker;
mod giphy;
mod klipy;
mod home;
mod soundboard;
mod sounds;
mod spell;
mod images;
mod titlebar;
mod instance;
mod invites;
mod looks;
mod markdown;
mod menu;
mod meter;
mod mixer;
mod ringtone;
mod server_card;
mod server_gifs;
mod plugins;
mod poker;
mod select;
mod settings;
mod stage;
mod textbox;
mod theme;
#[cfg(windows)]
mod tray;
mod trust;

/// The icon by the clock is Windows-only for now; elsewhere the close button quits as before.
#[cfg(not(windows))]
mod tray {
    pub struct Tray;
    pub fn is_active() -> bool {
        false
    }
    pub fn available() -> bool {
        false
    }
    pub fn ensure_icon() -> bool {
        false
    }
    pub fn notify(_title: &str, _text: &str) {}
    pub fn hide_main() {}
    pub fn prepare_exit() {}
}
mod unread;
mod update;
mod update_chip;
mod watchdog;

use std::sync::Arc;

use eframe::egui;
use pyr_engine::{Engine, EngineOptions};

fn main() -> eframe::Result<()> {
    let mut profile: Option<String> = None;
    let mut join: Option<String> = None;
    let mut renderer: Option<String> = pyr_proto::env_var("RENDERER");
    let mut quit = false;
    let mut autostart = false;
    let mut reopened: u32 = 0;
    // Invite links: pyr://, and ingle:// from before the rename (both open Pyr).
    let link_prefixes = [format!("{}://", pyr_proto::APP_ID), format!("{}://", pyr_proto::OLD_APP_ID)];
    let is_link = |a: &str| link_prefixes.iter().any(|p| a.get(..p.len()).is_some_and(|x| x.eq_ignore_ascii_case(p)));
    let all: Vec<String> = std::env::args().skip(1).collect();
    // Opened for an invite link (the pyr:// handler): that link is all we take, so nothing
    // smuggled into the link can pass as a command-line option.
    let all = match all.iter().find(|a| is_link(a)) {
        Some(link) => vec![link.clone()],
        None => all,
    };
    let mut args = all.into_iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--profile" => profile = args.next(),
            "--join" => join = args.next(),
            "--renderer" => renderer = args.next(),
            "--quit" => quit = true,
            // Started by Windows at sign-in (see autostart.rs).
            "--autostart" => autostart = true,
            // Started again after the graphics card went away under the window (how many
            // times in a row), see `relaunch`.
            "--reopened" => reopened = args.next().and_then(|n| n.parse().ok()).unwrap_or(1),
            "--help" | "-h" => {
                println!("{} — voice, video, screen share and chat\n", pyr_proto::APP_NAME);
                println!("  --profile NAME   use a separate identity (run two copies on one PC)");
                println!("  --join LINK      join a server from an invite link on start");
                println!("  --renderer wgpu|glow");
                println!("  --quit           ask a running Pyr to quit (the installer uses this)");
                return Ok(());
            }
            other if is_link(other) => join = Some(other.to_string()),
            _ => {}
        }
    }
    // A profile is a plain name, never a path.
    let profile = profile.filter(|p| pyr_engine::config::valid_profile(p));
    // One copy per profile: a second launch (say, from clicking an invite link) passes its
    // link to the copy that's already open and quits.
    let config_dir = pyr_engine::config::config_dir(profile.as_deref());
    if quit {
        // Before replacing files, the installer asks a running copy (which may be hidden in
        // the tray) to quit, and waits for it. Never opens a window.
        #[cfg(windows)]
        tray::quit_running(&config_dir);
        return Ok(());
    }
    // The first start after Ingle became Pyr: Ingle's settings, identity and plugins come over.
    let migrated = pyr_engine::config::migrate_from_ingle();
    let came_from_ingle = migrated.iter().any(|m| m.starts_with("copied Ingle's settings"));
    let (instance_lock, handoff) = match instance::start(&config_dir, join.as_deref()) {
        instance::Start::First { lock, handoff } => (lock, handoff),
        instance::Start::AlreadyRunning => {
            // Bring the running copy's window back, even from the tray. It picks up our
            // invite link (if any) from the handoff file once its window is showing.
            #[cfg(windows)]
            tray::show_running(&config_dir);
            return Ok(());
        }
    };
    REOPENED.store(reopened, std::sync::atomic::Ordering::Relaxed);
    let log_path = init_logging(profile.as_deref());
    install_crash_report(log_path.clone());
    for m in &migrated {
        tracing::info!("from Ingle: {m}");
    }
    // Claude's settings that started Ingle's connector start Pyr's now (your own identity
    // only, never a test profile's).
    if profile.is_none()
        && !pyr_proto::env_set("HOME")
        && let Some(exe) = connector::connector()
    {
        let args = connector::args(&config_dir);
        for c in connector::claude_configs() {
            match connector::migrate_from_ingle(&c, &exe, &args) {
                Ok(true) => tracing::info!("from Ingle: Claude's settings ({}) start Pyr's connector now", c.display()),
                Ok(false) => {}
                Err(e) => tracing::warn!("couldn't move Claude's settings over from Ingle's connector: {e}"),
            }
        }
    }

    let title = match &profile {
        Some(p) => format!("{} ({p})", pyr_proto::APP_NAME),
        None => pyr_proto::APP_NAME.to_string(),
    };
    // Opening with Windows follows the setting (only for your own identity, not test profiles).
    let start_settings = pyr_engine::config::load_config(&config_dir).settings;
    if profile.is_none() {
        autostart::apply(start_settings.start_with_windows);
    }
    // Started with Windows: stay in the tray (the icon opens the window), like Discord.
    let hidden = autostart && cfg!(windows) && start_settings.start_hidden;
    let viewport = egui::ViewportBuilder::default()
        .with_visible(!hidden)
        .with_title(&title)
        // Windows: Pyr draws its own dark title strip (titlebar.rs) instead of the white one.
        .with_decorations(!titlebar::enabled())
        .with_app_id(pyr_proto::APP_ID)
        .with_inner_size([1280.0, 800.0])
        .with_min_inner_size([960.0, 600.0])
        .with_icon(Arc::new(theme::app_icon()));

    // wgpu (DirectX 12 / Vulkan / Metal) is the default; fall back to OpenGL if it fails.
    let order: Vec<eframe::Renderer> = match renderer.as_deref() {
        Some("glow") => vec![eframe::Renderer::Glow],
        Some("wgpu") => vec![eframe::Renderer::Wgpu],
        _ if cfg!(windows) || cfg!(target_os = "macos") => vec![eframe::Renderer::Wgpu, eframe::Renderer::Glow],
        _ => vec![eframe::Renderer::Glow, eframe::Renderer::Wgpu],
    };
    // What lives on if the window has to be opened again (the graphics card went away: its
    // driver restarted, or Windows took it back), so a call carries on: the engine and the
    // updater. And where repaints go: the window there is now.
    let kept: Arc<std::sync::Mutex<Option<(Arc<Engine>, update::Updater)>>> = Default::default();
    let window: Arc<std::sync::Mutex<Option<egui::Context>>> = Default::default();
    let repaint: Arc<dyn Fn() + Send + Sync> = {
        let w = window.clone();
        Arc::new(move || {
            if let Some(c) = w.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
                c.request_repaint();
            }
        })
    };
    let mut last_err = None;
    let mut join = join;
    'renderers: for r in order {
        loop {
            let options = eframe::NativeOptions { viewport: viewport.clone(), renderer: r, ..Default::default() };
            let (profile_now, join, handoff, app_title) = (profile.clone(), join.take(), handoff.clone(), title.clone());
            let (kept_now, window_now, repaint_now) = (kept.clone(), window.clone(), repaint.clone());
            #[cfg(windows)]
            let (tray_dir, tray_tip) = (config_dir.clone(), title.clone());
            let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                eframe::run_native(
                    &title,
                    options,
                    Box::new(move |cc| {
                        theme::install(&cc.egui_ctx);
                        *window_now.lock().unwrap_or_else(|e| e.into_inner()) = Some(cc.egui_ctx.clone());
                        // The engine and updater: started the first time, the same ones after.
                        let mut slot = kept_now.lock().unwrap_or_else(|e| e.into_inner());
                        let (engine, updater) = match slot.as_ref() {
                            Some((e, u)) => (e.clone(), u.clone()),
                            None => {
                                let engine = Engine::start(EngineOptions { profile: profile_now, repaint: repaint_now.clone(), audio: dev_voice(), ..Default::default() })
                                    .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?;
                                let engine = Arc::new(engine);
                                let updater = update::Updater::start(repaint_now.clone(), engine.shared.config_dir.clone());
                                *slot = Some((engine.clone(), updater.clone()));
                                (engine, updater)
                            }
                        };
                        drop(slot);
                        #[cfg(windows)]
                        let tray = tray::Tray::create(cc, engine.clone(), updater.clone(), &tray_dir, &tray_tip);
                        // Whether you're looking at Pyr, asked of Windows itself: frames stop
                        // while it's hidden in the tray or minimized (see Engine::set_focus_probe).
                        #[cfg(windows)]
                        engine.set_focus_probe(tray::window_in_front);
                        // Writes what Pyr was doing to the log if its window ever stops
                        // responding.
                        watchdog::start(engine.clone());
                        // A crash or a freeze the last time Pyr was open is reported by itself
                        // (once, even if a second renderer is tried).
                        static LAST_RUN: std::sync::Once = std::sync::Once::new();
                        LAST_RUN.call_once(|| engine.report_last_run_trouble());
                        // The first start after Ingle became Pyr says so (once).
                        static RENAMED: std::sync::Once = std::sync::Once::new();
                        if came_from_ingle {
                            RENAMED.call_once(|| {
                                engine.shared.state.write().toast(
                                    "Ingle is Pyr now: the same app, with your servers, friends and identity. Its new home is pyr.gg.",
                                    false,
                                )
                            });
                        }
                        #[cfg(not(windows))]
                        let tray: Option<tray::Tray> = None;
                        let plugins = plugins::Plugins::new(&engine.shared.config_dir, repaint_now.clone());
                        Ok(Box::new(app::PyrApp::new(engine, handoff, join, tray, updater, plugins, app_title)))
                    }),
                )
            }));
            match run {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(e)) => {
                    tracing::warn!("renderer {r:?} failed: {e}");
                    last_err = Some(e);
                    continue 'renderers;
                }
                // The graphics card went away under the window (its driver restarted, or Windows
                // took it back): Pyr starts again by itself, back in the call it was in (a few
                // times in a row at most).
                Err(panic) if reopened < 3 && gpu_lost(&panic_text(&*panic)) => {
                    tracing::warn!("the graphics card went away under the window ({}); starting Pyr again", panic_text(&*panic));
                    let engine = kept.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(e, _)| e.clone());
                    if let Some(engine) = &engine {
                        let call = engine.state().call.as_ref().map(|c| (c.server_key.clone(), c.channel_id));
                        if let Some((key, channel)) = call {
                            update::save_resume(&engine.shared.config_dir, &key, Some(channel));
                        }
                    }
                    // (The new copy must find no Pyr already running.)
                    drop(instance_lock);
                    relaunch(profile.as_deref(), renderer.as_deref(), reopened + 1);
                    std::process::exit(0);
                }
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
    }
    let err = last_err.expect("at least one renderer was tried");
    fatal_message(&format!(
        "{} couldn't open its window: {err}\n\nTry updating your graphics driver. Details are in {}",
        pyr_proto::APP_NAME,
        log_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "the log".into())
    ));
    Err(err)
}

/// Development builds only: `PYR_DEV_VOICE` puts a made-up voice in place of the
/// microphone (`speech`: words and pauses, deep and bright sounds; anything else: silence),
/// and plays nothing, to see voice meters and speaking rings on a PC without sound. Never in
/// release builds.
#[cfg(debug_assertions)]
fn dev_voice() -> Option<pyr_engine::AudioIoFactory> {
    use pyr_engine::audio::{AudioInput, AudioOutput};
    let v = pyr_proto::env_var("DEV_VOICE").filter(|v| !v.is_empty())?;
    let speech = v == "speech";
    Some(Box::new(move |_settings| {
        let mut voice = DevVoice::default();
        let input: Box<dyn FnMut(&mut [f32]) + Send> = Box::new(move |out| {
            if speech {
                voice.fill(out)
            } else {
                out.fill(0.0)
            }
        });
        (AudioInput::Synthetic(input), AudioOutput::None)
    }))
}

#[cfg(not(debug_assertions))]
fn dev_voice() -> Option<pyr_engine::AudioIoFactory> {
    None
}

/// A made-up voice (development builds): a buzz at a voice's pitch shaped by the resonances
/// of vowels that change each syllable, a hiss now and then (an "s"), in words and pauses.
#[cfg(debug_assertions)]
#[derive(Default)]
struct DevVoice {
    /// Samples made so far (48 kHz).
    n: u64,
    phase: f64,
    noise: u32,
}

#[cfg(debug_assertions)]
impl DevVoice {
    fn fill(&mut self, out: &mut [f32]) {
        use std::f64::consts::TAU;
        const RATE: f64 = 48_000.0;
        // Vowels' first two resonances (Hz): a, e, i, o, u, and the "uh" in between.
        const VOWELS: [(f64, f64); 6] = [(730.0, 1090.0), (530.0, 1840.0), (270.0, 2290.0), (570.0, 840.0), (300.0, 870.0), (520.0, 1190.0)];
        let t = self.n as f64 / RATE;
        // Words for 2.6 s, then a pause of 0.9 s.
        let phrase = t % 3.5;
        let talking = phrase < 2.6;
        let syllable = (t * 4.2).floor() as usize;
        let within = (t * 4.2).fract();
        let (f1, f2) = VOWELS[(syllable * 7 + syllable / 3) % VOWELS.len()];
        let pitch = 125.0 + 30.0 * (t * 0.9).sin() + 12.0 * (t * 5.0).sin();
        // Each syllable swells and fades; louder at the start of a phrase.
        let env = if talking { (within * std::f64::consts::PI).sin().powf(0.7) * (0.75 + 0.25 * (1.0 - phrase / 2.6)) } else { 0.0 };
        let hiss = talking && syllable % 5 == 3 && within > 0.55;
        let weight = |f: f64| {
            let g = |c: f64, w: f64| (-((f - c) / w).powi(2)).exp();
            g(f1, 110.0) + 0.7 * g(f2, 160.0) + 0.35 * g(2600.0, 250.0) + 0.04
        };
        let harmonics: Vec<(f64, f64)> = (1..=36).map(|k| k as f64 * pitch).filter(|f| *f < 9000.0).map(|f| (f / pitch, weight(f) / (f / pitch).sqrt())).collect();
        for o in out.iter_mut() {
            self.phase = (self.phase + TAU * pitch / RATE) % (TAU * 1000.0);
            let mut v: f64 = harmonics.iter().map(|(k, a)| a * (self.phase * k).sin()).sum();
            v *= 0.055 * env;
            if hiss {
                // xorshift noise, mostly bright
                self.noise ^= self.noise << 13;
                self.noise ^= self.noise >> 17;
                self.noise ^= self.noise << 5;
                if self.noise == 0 {
                    self.noise = 0x9e37_79b9;
                }
                v += (self.noise as f64 / u32::MAX as f64 - 0.5) * 0.09;
            }
            *o = v as f32;
            self.n += 1;
        }
    }
}

/// Log to `pyr.log` in the config folder (the Windows app has no console), and also to
/// the terminal in debug builds. Returns the log file's path.
fn init_logging(profile: Option<&str>) -> Option<std::path::PathBuf> {
    use tracing_subscriber::fmt::writer::MakeWriterExt;
    let filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn,pyr=info,pyr_engine=info".into())
    };
    let dir = pyr_engine::config::config_dir(profile);
    let path = dir.join("pyr.log");
    // Keep the last run's log: after a freeze or a crash it's the one that says what happened
    // (and it goes along with bug reports).
    let _ = std::fs::rename(&path, dir.join("pyr.previous.log"));
    let file = std::fs::create_dir_all(&dir).ok().and_then(|_| std::fs::File::create(&path).ok());
    match file {
        Some(f) => {
            let writer = std::sync::Mutex::new(f).and(std::io::stderr);
            tracing_subscriber::fmt().with_env_filter(filter()).with_ansi(false).with_writer(writer).init();
            Some(path)
        }
        None => {
            tracing_subscriber::fmt().with_env_filter(filter()).init();
            None
        }
    }
}

/// On a crash, write what happened to the log and tell the person where it is.
fn install_crash_report(log_path: Option<std::path::PathBuf>) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let what = info.to_string();
        let main = thread == "main";
        // (The graphics card going away under the window: Pyr starts again, see `main`.)
        let recovered = main && gpu_lost(&what) && REOPENED.load(std::sync::atomic::Ordering::Relaxed) < 3;
        if recovered {
            tracing::error!("the window crashed (the graphics card went away), starting Pyr again: {info}");
        } else {
            tracing::error!("crash in thread {thread}: {info}");
        }
        default(info);
        // Pyr carries on: reported by itself, now. (A crash that ends Pyr is reported the
        // next time it starts, from the log.)
        if !main || recovered {
            pyr_engine::bugs::report_crash(&thread, &what);
        }
        // Only the UI thread's crash ends the app; others are logged and the app carries on.
        if main && !recovered {
            fatal_message(&format!(
                "{} ran into a problem and has to close.\n\n{info}\n\nDetails are in {}",
                pyr_proto::APP_NAME,
                log_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "the log".into())
            ));
        }
    }));
}

/// How many times in a row Pyr was started again after the graphics card went away.
static REOPENED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Whether a crash is the graphics card going away under the window (its driver restarted, or
/// Windows took it back): drawing or showing a frame failed for that.
fn gpu_lost(what: &str) -> bool {
    const SIGNS: [&str; 11] = [
        // egui's renderer, when the graphics card can't give it memory any more (after "Acquiring
        // a texture failed": the card was out of memory, or resetting).
        "Failed to create staging buffer",
        "Surface::present",
        "Surface::get_current_texture",
        "Surface::configure",
        "Device is lost",
        "device lost",
        "DeviceLost",
        "Parent device is lost",
        "has been suspended",
        "0x887A0005",
        "DXGI_ERROR_DEVICE",
    ];
    SIGNS.iter().any(|s| what.contains(s))
}

/// Starts Pyr again (after the graphics card went away under its window): the same profile
/// and renderer, and how many times in a row it's been started again.
fn relaunch(profile: Option<&str>, renderer: Option<&str>, reopened: u32) {
    let Ok(exe) = std::env::current_exe() else { return };
    let mut cmd = std::process::Command::new(exe);
    if let Some(p) = profile {
        cmd.args(["--profile", p]);
    }
    if let Some(r) = renderer {
        cmd.args(["--renderer", r]);
    }
    cmd.args(["--reopened", &reopened.to_string()]);
    if let Err(e) = cmd.spawn() {
        tracing::warn!("couldn't start Pyr again: {e}");
    }
}

/// What a caught crash said.
fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    panic.downcast_ref::<String>().cloned().or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
}

#[cfg(windows)]
fn fatal_message(text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::HSTRING;
    unsafe {
        MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(pyr_proto::APP_NAME), MB_OK | MB_ICONERROR);
    }
}

#[cfg(not(windows))]
fn fatal_message(text: &str) {
    eprintln!("{text}");
}

#[cfg(test)]
mod tests {
    use super::gpu_lost;

    /// The crashes in bug reports 8, 13 and 14 were the graphics card going away (Pyr starts
    /// again, back where it was); a bug in Pyr isn't.
    #[test]
    fn graphics_card_crashes_are_told_apart() {
        assert!(gpu_lost(
            "panicked at egui-wgpu-0.33.3/src/renderer.rs:958:17:\nFailed to create staging buffer for index data. Index count: 44526. Required index buffer size: 178104. Actual size 570912 and capacity: 570912 (bytes)"
        ));
        assert!(gpu_lost("panicked at wgpu-27.0.1/src/backend/wgpu_core.rs:3858:38:\nError in Surface::present: Validation Error\n\nCaused by:\n  Parent device is lost"));
        assert!(!gpu_lost("panicked at crates/app/src/chat.rs:10:5:\nindex out of bounds: the len is 3 but the index is 7"));
    }
}
