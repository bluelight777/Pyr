#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! Ingle desktop app.

mod app;
mod autostart;
mod chat;
mod create;
mod dialogs;
mod icons;
mod avatar_edit;
mod emoji;
mod emoji_picker;
mod giphy;
mod home;
mod soundboard;
mod sounds;
mod spell;
mod images;
mod titlebar;
mod instance;
mod markdown;
mod select;
mod stage;
mod theme;
#[cfg(windows)]
mod tray;

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
mod watchdog;

use std::sync::Arc;

use eframe::egui;
use ingle_engine::{Engine, EngineOptions};

fn main() -> eframe::Result<()> {
    let mut profile: Option<String> = None;
    let mut join: Option<String> = None;
    let mut renderer: Option<String> = std::env::var("INGLE_RENDERER").ok();
    let mut quit = false;
    let mut autostart = false;
    let link_prefix = format!("{}://", ingle_proto::APP_ID);
    let all: Vec<String> = std::env::args().skip(1).collect();
    // Opened for an invite link (the ingle:// handler): that link is all we take, so nothing
    // smuggled into the link can pass as a command-line option.
    let all = match all.iter().find(|a| a.starts_with(&link_prefix)) {
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
            "--help" | "-h" => {
                println!("{} — voice, video, screen share and chat\n", ingle_proto::APP_NAME);
                println!("  --profile NAME   use a separate identity (run two copies on one PC)");
                println!("  --join LINK      join a server from an invite link on start");
                println!("  --renderer wgpu|glow");
                println!("  --quit           ask a running Ingle to quit (the installer uses this)");
                return Ok(());
            }
            other if other.starts_with(&link_prefix) => join = Some(other.to_string()),
            _ => {}
        }
    }
    // A profile is a plain name, never a path.
    let profile = profile.filter(|p| ingle_engine::config::valid_profile(p));
    // One copy per profile: a second launch (say, from clicking an invite link) passes its
    // link to the copy that's already open and quits.
    let config_dir = ingle_engine::config::config_dir(profile.as_deref());
    if quit {
        // Before replacing files, the installer asks a running copy (which may be hidden in
        // the tray) to quit, and waits for it. Never opens a window.
        #[cfg(windows)]
        tray::quit_running(&config_dir);
        return Ok(());
    }
    let (_instance_lock, handoff) = match instance::start(&config_dir, join.as_deref()) {
        instance::Start::First { lock, handoff } => (lock, handoff),
        instance::Start::AlreadyRunning => {
            // Bring the running copy's window back, even from the tray. It picks up our
            // invite link (if any) from the handoff file once its window is showing.
            #[cfg(windows)]
            tray::show_running(&config_dir);
            return Ok(());
        }
    };
    let log_path = init_logging(profile.as_deref());
    install_crash_report(log_path.clone());

    let title = match &profile {
        Some(p) => format!("{} ({p})", ingle_proto::APP_NAME),
        None => ingle_proto::APP_NAME.to_string(),
    };
    // Opening with Windows follows the setting (only for your own identity, not test profiles).
    let start_settings = ingle_engine::config::load_config(&config_dir).settings;
    if profile.is_none() {
        autostart::apply(start_settings.start_with_windows);
    }
    // Started with Windows: stay in the tray (the icon opens the window), like Discord.
    let hidden = autostart && cfg!(windows) && start_settings.start_hidden;
    let viewport = egui::ViewportBuilder::default()
        .with_visible(!hidden)
        .with_title(&title)
        // Windows: Ingle draws its own dark title strip (titlebar.rs) instead of the white one.
        .with_decorations(!titlebar::enabled())
        .with_app_id(ingle_proto::APP_ID)
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
    let mut last_err = None;
    for r in order {
        let options = eframe::NativeOptions { viewport: viewport.clone(), renderer: r, ..Default::default() };
        let (profile, join, handoff, app_title) = (profile.clone(), join.clone(), handoff.clone(), title.clone());
        #[cfg(windows)]
        let (tray_dir, tray_tip) = (config_dir.clone(), title.clone());
        let res = eframe::run_native(
            &title,
            options,
            Box::new(move |cc| {
                theme::install(&cc.egui_ctx);
                let ctx = cc.egui_ctx.clone();
                let engine = Engine::start(EngineOptions {
                    profile,
                    repaint: Arc::new(move || ctx.request_repaint()),
                    ..Default::default()
                })
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?;
                let engine = Arc::new(engine);
                let ctx = cc.egui_ctx.clone();
                let updater = update::Updater::start(Arc::new(move || ctx.request_repaint()));
                #[cfg(windows)]
                let tray = tray::Tray::create(cc, engine.clone(), &tray_dir, &tray_tip);
                // Whether you're looking at Ingle, asked of Windows itself: frames stop while
                // it's hidden in the tray or minimized (see Engine::set_focus_probe).
                #[cfg(windows)]
                engine.set_focus_probe(tray::window_in_front);
                // Writes what Ingle was doing to the log if its window ever stops responding.
                watchdog::start(engine.clone());
                #[cfg(not(windows))]
                let tray: Option<tray::Tray> = None;
                Ok(Box::new(app::IngleApp::new(engine, handoff, join, tray, updater, app_title)))
            }),
        );
        match res {
            Ok(()) => return Ok(()),
            Err(e) => {
                tracing::warn!("renderer {r:?} failed: {e}");
                last_err = Some(e);
            }
        }
    }
    let err = last_err.expect("at least one renderer was tried");
    fatal_message(&format!(
        "{} couldn't open its window: {err}\n\nTry updating your graphics driver. Details are in {}",
        ingle_proto::APP_NAME,
        log_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "the log".into())
    ));
    Err(err)
}

/// Log to `ingle.log` in the config folder (the Windows app has no console), and also to
/// the terminal in debug builds. Returns the log file's path.
fn init_logging(profile: Option<&str>) -> Option<std::path::PathBuf> {
    use tracing_subscriber::fmt::writer::MakeWriterExt;
    let filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn,ingle=info,ingle_engine=info".into())
    };
    let dir = ingle_engine::config::config_dir(profile);
    let path = dir.join("ingle.log");
    // Keep the last run's log: after a freeze or a crash it's the one that says what happened
    // (and it goes along with bug reports).
    let _ = std::fs::rename(&path, dir.join("ingle.previous.log"));
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
        tracing::error!("crash in thread {thread}: {info}");
        default(info);
        // Only the UI thread's crash ends the app; others are logged and the app carries on.
        if thread == "main" {
            fatal_message(&format!(
                "{} ran into a problem and has to close.\n\n{info}\n\nDetails are in {}",
                ingle_proto::APP_NAME,
                log_path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "the log".into())
            ));
        }
    }));
}

#[cfg(windows)]
fn fatal_message(text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::HSTRING;
    unsafe {
        MessageBoxW(None, &HSTRING::from(text), &HSTRING::from(ingle_proto::APP_NAME), MB_OK | MB_ICONERROR);
    }
}

#[cfg(not(windows))]
fn fatal_message(text: &str) {
    eprintln!("{text}");
}
