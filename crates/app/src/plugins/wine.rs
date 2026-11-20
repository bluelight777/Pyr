//! Windows programs on Linux, through Wine (0.16.4). Blue, Nov 20 2026: "make Torch and Conquer
//! work in Omarchy/linux", and "make Pyropoly work in Omarchy/linux": his friend plays on Omarchy
//! (Arch, Hyprland), and both are Windows programs only (Torch & Conquer's source after Oct 25
//! went with the workspace on Nov 10, so there's no Linux build of it yet; Blue: "Both: Wine now,
//! rebuild later"). A program with no Linux files installs its Windows ones and opens through
//! Wine, in a Wine prefix of Pyr's own (its own C: drive, beside the plugins: the user's own Wine
//! is never touched), when Wine's installed; when it isn't, the Library says how to install it.
//! It opens in its own window, as Linux programs do.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Wine, if it's installed (on the PATH): `wine`, or `wine64` where that's its name. Looked for
/// again every few seconds, so installing it while Pyr's open is noticed.
pub fn find() -> Option<PathBuf> {
    // (`PYR_WINE`: a Wine that isn't on the PATH, Ubuntu's /usr/lib/wine/wine64 say, or a
    // stand-in for one in tests.)
    if let Some(w) = pyr_proto::env_var("WINE") {
        let w = PathBuf::from(w);
        return is_program(&w).then_some(w);
    }
    static SEEN: std::sync::Mutex<Option<(Instant, Option<PathBuf>)>> = std::sync::Mutex::new(None);
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, found)) = seen.as_ref()
        && at.elapsed() < Duration::from_secs(5)
    {
        return found.clone();
    }
    let found = look_for(std::env::var_os("PATH").as_deref());
    *seen = Some((Instant::now(), found.clone()));
    found
}

/// The first `wine` (then `wine64`) on a PATH.
pub fn look_for(path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let dirs: Vec<PathBuf> = std::env::split_paths(path?).collect();
    ["wine", "wine64"].iter().find_map(|name| dirs.iter().map(|d| d.join(name)).find(|p| is_program(p)))
}

fn is_program(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        p.is_file()
    }
}

/// Pyr's own Wine prefix, beside the plugins.
pub fn prefix(plugins_dir: &Path) -> PathBuf {
    plugins_dir.join("wine")
}

/// What's set for every program run through Wine: Pyr's prefix; no asking to install Wine's .NET
/// or its web browser (no program of Pyr's needs either, and the questions would come up the first
/// time); and Wine's own debugging kept out of what the program says (which goes in pyr.log).
fn env(cmd: &mut Command, plugins_dir: &Path) {
    cmd.env("WINEPREFIX", prefix(plugins_dir));
    cmd.env("WINEDLLOVERRIDES", "mscoree=;mshtml=");
    cmd.env("WINEDEBUG", "-all");
}

/// A Windows program started through Wine.
pub fn command(wine: &Path, plugins_dir: &Path, exe: &Path) -> Command {
    let mut cmd = Command::new(wine);
    cmd.arg(exe);
    env(&mut cmd, plugins_dir);
    cmd
}

/// Gets Pyr's prefix ready, when it isn't yet (the first program installed through Wine): Wine
/// takes a while setting one up, better here than as the program opens. A prefix that couldn't
/// be readied here is set up when a program first opens instead, so this never stops an install.
pub fn prepare(wine: &Path, plugins_dir: &Path) {
    let pfx = prefix(plugins_dir);
    if pfx.join("system.reg").exists() {
        return;
    }
    let _ = std::fs::create_dir_all(&pfx);
    let mut cmd = Command::new(wine);
    cmd.args(["wineboot", "--init"]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    env(&mut cmd, plugins_dir);
    let started = Instant::now();
    match cmd.spawn() {
        Ok(mut child) => loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    tracing::info!(%status, secs = started.elapsed().as_secs(), "Wine's prefix for Pyr's programs is ready");
                    return;
                }
                Ok(None) if started.elapsed() < Duration::from_secs(180) => std::thread::sleep(Duration::from_millis(250)),
                _ => {
                    tracing::warn!("Wine took over 3 minutes setting up its prefix: it'll finish when the program opens");
                    return;
                }
            }
        },
        Err(e) => tracing::warn!("couldn't start Wine to set up its prefix: {e}"),
    }
}

/// How to install Wine on this system, by its own package manager (from /etc/os-release).
pub fn how_to_install() -> String {
    how_to_install_on(&std::fs::read_to_string("/etc/os-release").unwrap_or_default())
}

pub fn how_to_install_on(os_release: &str) -> String {
    let field = |key: &str| {
        os_release
            .lines()
            .find_map(|l| l.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
            .map(|v| v.trim().trim_matches('"').to_lowercase())
            .unwrap_or_default()
    };
    let ids = format!("{} {}", field("ID"), field("ID_LIKE"));
    let is = |name: &str| ids.split_whitespace().any(|w| w == name);
    if is("arch") {
        "sudo pacman -S wine".into()
    } else if is("debian") || is("ubuntu") {
        "sudo apt install wine".into()
    } else if is("fedora") || is("rhel") {
        "sudo dnf install wine".into()
    } else if is("opensuse") || is("suse") {
        "sudo zypper install wine".into()
    } else {
        "install Wine with your system's package manager".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_how_to_install_wine_on_each_system() {
        // Omarchy is Arch.
        assert_eq!(how_to_install_on("NAME=\"Arch Linux\"\nID=arch\n"), "sudo pacman -S wine");
        assert_eq!(how_to_install_on("ID=endeavouros\nID_LIKE=arch\n"), "sudo pacman -S wine");
        assert_eq!(how_to_install_on("ID=ubuntu\nID_LIKE=debian\n"), "sudo apt install wine");
        assert_eq!(how_to_install_on("ID=linuxmint\nID_LIKE=\"ubuntu debian\"\n"), "sudo apt install wine");
        assert_eq!(how_to_install_on("ID=fedora\n"), "sudo dnf install wine");
        assert_eq!(how_to_install_on("ID=\"opensuse-tumbleweed\"\nID_LIKE=\"opensuse suse\"\n"), "sudo zypper install wine");
        assert_eq!(how_to_install_on(""), "install Wine with your system's package manager");
    }

    #[cfg(unix)]
    #[test]
    fn finds_wine_on_the_path() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("pyr-wine-{}", std::process::id()));
        let (a, b) = (d.join("a"), d.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let path = std::env::join_paths([&a, &b]).unwrap();
        assert_eq!(look_for(Some(&path)), None);
        // (Not a program: not taken.)
        std::fs::write(b.join("wine"), "").unwrap();
        assert_eq!(look_for(Some(&path)), None);
        std::fs::set_permissions(b.join("wine"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(look_for(Some(&path)), Some(b.join("wine")));
        std::fs::write(a.join("wine64"), "").unwrap();
        std::fs::set_permissions(a.join("wine64"), std::fs::Permissions::from_mode(0o755)).unwrap();
        // (`wine` first, wherever it is.)
        assert_eq!(look_for(Some(&path)), Some(b.join("wine")));
        let _ = std::fs::remove_dir_all(&d);
    }
}
