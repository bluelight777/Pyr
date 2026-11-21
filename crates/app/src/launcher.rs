//! Linux: a Pyr run from a source build (`cargo build`, not the Debian package) puts itself in
//! the desktop's app launcher, the way the package's /usr/share/applications/pyr.desktop does,
//! so it shows up on Super, in the dock, and opens pyr:// invite links. The entry it writes
//! carries `X-Pyr-Source-Build=true`; Pyr only ever rewrites an entry with that line (when the
//! program moves), never one the package or the person made.

#[cfg(target_os = "linux")]
pub fn ensure() {
    let Ok(exe) = std::env::current_exe().and_then(|e| e.canonicalize()) else { return };
    // The package's own entry already covers the packaged program.
    if Path::new("/usr/bin/pyr").canonicalize().ok().as_deref() == Some(exe.as_path()) {
        return;
    }
    let Some(data) = data_home() else { return };
    let apps = data.join("applications");
    let entry_path = apps.join(format!("{}.desktop", pyr_proto::APP_ID));
    let existing = std::fs::read_to_string(&entry_path).ok();
    let want = entry(&exe);
    match &existing {
        Some(text) if text == &want => return,
        Some(text) if !text.lines().any(|l| l.trim() == MARK) => return,
        _ => {}
    }
    let icon_dir = data.join("icons/hicolor/256x256/apps");
    let wrote = std::fs::create_dir_all(&icon_dir)
        .and_then(|_| std::fs::write(icon_dir.join(format!("{}.png", pyr_proto::APP_ID)), ICON))
        .and_then(|_| std::fs::create_dir_all(&apps))
        .and_then(|_| write_atomic(&entry_path, &want));
    match wrote {
        Ok(()) => {
            tracing::info!("app launcher entry for this source build: {}", entry_path.display());
            // Lets the desktop see the new entry and the pyr:// handler now (best effort).
            let _ = std::process::Command::new("update-desktop-database")
                .arg(&apps)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        Err(e) => tracing::warn!("couldn't add Pyr to the app launcher: {e}"),
    }
}

#[cfg(not(target_os = "linux"))]
pub fn ensure() {}

#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
const MARK: &str = "X-Pyr-Source-Build=true";

#[cfg(target_os = "linux")]
const ICON: &[u8] = include_bytes!("../assets/icon/pyr-256.png");

#[cfg(target_os = "linux")]
fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
}

/// The package's entry (installer/linux/pyr.desktop), pointing at this program.
#[cfg(any(target_os = "linux", test))]
fn entry(exe: &std::path::Path) -> String {
    // Desktop entry Exec quoting: the path in double quotes, with ", `, $ and \ escaped.
    let mut quoted = String::from("\"");
    for c in exe.display().to_string().chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    format!(
        "[Desktop Entry]\nType=Application\nName=Pyr\nGenericName=Voice and Video Chat\n\
         Comment=Voice, webcams, screen sharing and text channels for your group\n\
         Exec={quoted} %u\nIcon={id}\nTerminal=false\nCategories=Network;Chat;InstantMessaging;\n\
         Keywords=voice;video;chat;call;screen share;webcam;\nMimeType=x-scheme-handler/{id};\n\
         StartupWMClass={id}\nStartupNotify=true\nX-Pyr-Source-Build=true\n",
        id = pyr_proto::APP_ID
    )
}

#[cfg(target_os = "linux")]
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("desktop.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_points_at_this_program_and_is_marked() {
        let e = entry(std::path::Path::new("/home/someone/pyr/target/release/pyr"));
        assert!(e.contains("Exec=\"/home/someone/pyr/target/release/pyr\" %u\n"), "{e}");
        assert!(e.contains("MimeType=x-scheme-handler/pyr;"), "{e}");
        assert!(e.lines().any(|l| l == "X-Pyr-Source-Build=true"), "{e}");
    }

    #[test]
    fn exec_path_is_quoted_safely() {
        let e = entry(std::path::Path::new("/home/a b/$x`y\"z\\w/pyr"));
        assert!(e.contains("Exec=\"/home/a b/\\$x\\`y\\\"z\\\\w/pyr\" %u\n"), "{e}");
    }
}
