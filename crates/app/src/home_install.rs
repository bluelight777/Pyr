//! Pyr in your home folder, on any Linux (0.16.7). Blue, Nov 25: "make it so when Prog updates
//! his Pyr on Omarchy it does it fast and automatically like windows does it. Make it super easy
//! to update." The .deb installs Pyr for the whole PC and updates through apt, which asks for
//! your password; Arch (Omarchy) and the other distros have no apt, so their Pyr couldn't update
//! itself at all. On Windows Pyr lives in your own folders and updates itself without a word;
//! here it does the same:
//!
//! ```text
//! ~/.local/share/pyr/app/0.16.7/     pyr, pyr-mcp, pyr-server, vps/, icons/, pyr.desktop, pyr-home.json
//! ~/.local/share/pyr/app/current  -> 0.16.7
//! ~/.local/bin/pyr                -> ~/.local/share/pyr/app/current/pyr
//! ~/.local/share/applications/pyr.desktop (your apps menu, and pyr:// invite links)
//! ```
//!
//! An update (`install`) unpacks the new version's folder beside the others, points `current`
//! at it in one step (a renamed link), and Pyr opens the new one: no password, no package
//! manager, nothing outside your home folder. The version before stays, the ones before that go.
//! `installer/site/install.sh` puts it there the first time (`curl -fsSL https://pyr.gg/install.sh
//! | sh`); a Pyr installed some other way on a PC without the Debian package moves itself here
//! the next time it updates (`update::how`).

use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// In each version's folder (the download brings it): that this is Pyr in your home folder, and
/// which version.
pub const MARKER: &str = "pyr-home.json";
/// The program, in each version's folder.
const PROGRAM: &str = "pyr";
/// What's in a download at most (the program and its files are about 220 MB unpacked).
const MOST_BYTES: u64 = 1_500_000_000;
const MOST_ENTRIES: usize = 2_000;

/// Where your own programs' files go (`$XDG_DATA_HOME`, or `~/.local/share`).
pub fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home().map(|h| h.join(".local").join("share")))
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|p| p.is_absolute())
}

/// The home install's folder: its versions, and `current`.
pub fn root() -> Option<PathBuf> {
    data_home().map(|d| d.join("pyr").join("app"))
}

/// The program `current` points at: the one to open.
pub fn program(root: &Path) -> PathBuf {
    root.join("current").join(PROGRAM)
}

/// The home install this Pyr runs from, if it does.
pub fn running() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    from_exe(&exe, &root()?)
}

/// `exe` is a home install's program (in a version's folder of `root`, with its marker): that
/// root. (Pyr unpacked anywhere else isn't: its next update puts it in `root`.)
pub fn from_exe(exe: &Path, root: &Path) -> Option<PathBuf> {
    let dir = exe.parent()?;
    if !dir.join(MARKER).is_file() {
        return None;
    }
    let root = std::fs::canonicalize(root).ok()?;
    let parent = std::fs::canonicalize(dir.parent()?).ok()?;
    (parent == root).then_some(root)
}

/// The same file in `current`, for a path that's written down somewhere and kept (the Claude
/// connector's, in Claude's settings): a version's folder goes two updates later, `current`
/// stays. Anything not in a home install is as it is.
pub fn lasting(file: &Path) -> PathBuf {
    let (Some(name), Some(dir)) = (file.file_name(), file.parent()) else { return file.to_path_buf() };
    match dir.parent() {
        Some(root) if dir.join(MARKER).is_file() && root.join("current").join(name).exists() => root.join("current").join(name),
        _ => file.to_path_buf(),
    }
}

/// Installed by the Debian package (dpkg keeps a record of it): it updates through apt, as before.
pub fn by_dpkg() -> bool {
    ["/var/lib/dpkg/info/pyr.list", "/var/lib/dpkg/info/pyr:amd64.list"].iter().any(|p| Path::new(p).is_file())
}

/// Installs the downloaded update (`tarball`, a .tar.gz of a version's folder) in `root`: checked
/// against `sha256` as it's read (the bytes checked are the bytes unpacked), unpacked into a new
/// folder beside the others, then `current` points at it. Returns the new version's folder.
pub fn install(tarball: &Path, sha256: &str, version: &str, root: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(tarball).map_err(|e| format!("couldn't read the download: {e}"))?;
    if pyr_proto::release::sha256_hex(&bytes) != sha256.to_lowercase() {
        return Err("the downloaded update changed on disk, so it wasn't installed".into());
    }
    if !plain_name(version) {
        return Err(format!("the update's version ({version}) isn't one Pyr can name a folder after"));
    }
    make_dir(root)?;
    let incoming = root.join(format!(".incoming-{}", random()));
    std::fs::create_dir(&incoming).map_err(|e| format!("couldn't make a folder for the update: {e}"))?;
    let done = unpack(&bytes, &incoming).and_then(|()| check(&incoming, version)).and_then(|()| put_in_place(&incoming, root, version));
    if done.is_err() {
        let _ = std::fs::remove_dir_all(&incoming);
    }
    let previous = done?;
    tidy(root, version, previous.as_deref());
    Ok(root.join(version))
}

/// A version: digits, letters, dots and dashes (a folder's name, never a path).
fn plain_name(v: &str) -> bool {
    !v.is_empty() && v.len() <= 40 && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') && !v.starts_with('.')
}

fn random() -> String {
    pyr_proto::crypto::random_bytes::<8>().iter().map(|b| format!("{b:02x}")).collect()
}

fn make_dir(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("couldn't make {}: {e}", dir.display()))
}

/// Unpacks a .tar.gz into `dest`: its folders and plain files only, each under `dest` (no link,
/// no device, nothing outside it, nothing written twice), the programs runnable.
fn unpack(bytes: &[u8], dest: &Path) -> Result<(), String> {
    use std::os::unix::fs::OpenOptionsExt;
    let bad = |e: std::io::Error| format!("the download isn't a Pyr update ({e})");
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    let (mut total, mut count) = (0u64, 0usize);
    for entry in archive.entries().map_err(bad)? {
        let mut entry = entry.map_err(bad)?;
        count += 1;
        if count > MOST_ENTRIES {
            return Err("the download has far too many files to be a Pyr update".into());
        }
        let path = entry.path().map_err(bad)?.into_owned();
        let Some(rel) = relative(&path) else {
            return Err(format!("the download has a file where none can go ({})", path.display()));
        };
        let to = dest.join(&rel);
        match entry.header().entry_type() {
            // ("./": the folder itself.)
            tar::EntryType::Directory if rel.as_os_str().is_empty() => {}
            tar::EntryType::Directory => make_dir(&to)?,
            tar::EntryType::Regular | tar::EntryType::Continuous if rel.as_os_str().is_empty() => {
                return Err("the download has a file with no name".into());
            }
            tar::EntryType::Regular | tar::EntryType::Continuous => {
                total += entry.size();
                if total > MOST_BYTES {
                    return Err("the download unpacks to far more than a Pyr update".into());
                }
                if let Some(dir) = to.parent() {
                    make_dir(dir)?;
                }
                let runnable = entry.header().mode().map(|m| m & 0o111 != 0).unwrap_or(false);
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(if runnable { 0o755 } else { 0o644 })
                    .open(&to)
                    .map_err(|e| format!("couldn't write {}: {e}", rel.display()))?;
                std::io::copy(&mut (&mut entry).take(MOST_BYTES), &mut file).map_err(|e| format!("couldn't unpack {}: {e}", rel.display()))?;
                file.sync_all().map_err(|e| format!("couldn't write {}: {e}", rel.display()))?;
            }
            other => return Err(format!("the download has something that isn't a file or a folder ({other:?}: {})", rel.display())),
        }
    }
    Ok(())
}

/// A path in an archive as one under the folder it's unpacked into: plain names only (a
/// leading "./" is fine; "./" alone is the folder itself, an empty path), a few folders deep at
/// most.
fn relative(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::Normal(n) if n.len() <= 120 => out.push(n),
            _ => return None,
        }
    }
    (out.components().count() <= 6).then_some(out)
}

/// The unpacked folder is the version it says it is, with a program to run.
fn check(dir: &Path, version: &str) -> Result<(), String> {
    #[derive(serde::Deserialize)]
    struct Marker {
        version: String,
    }
    let text = std::fs::read_to_string(dir.join(MARKER)).map_err(|_| "the download isn't Pyr for your home folder".to_string())?;
    let m: Marker = serde_json::from_str(&text).map_err(|_| "the download isn't Pyr for your home folder".to_string())?;
    if m.version != version {
        return Err(format!("the download is Pyr {}, not {version}", m.version));
    }
    use std::os::unix::fs::PermissionsExt;
    match std::fs::symlink_metadata(dir.join(PROGRAM)) {
        Ok(md) if md.is_file() && md.permissions().mode() & 0o111 != 0 => Ok(()),
        _ => Err("the download has no Pyr program in it".into()),
    }
}

/// The unpacked folder becomes `root/<version>` and `current` points at it (a new link renamed
/// over the old one: there's never a moment without one). What `current` pointed at before.
fn put_in_place(incoming: &Path, root: &Path, version: &str) -> Result<Option<String>, String> {
    let current = root.join("current");
    let previous = std::fs::read_link(&current).ok().and_then(|p| p.to_str().map(String::from)).filter(|p| plain_name(p));
    let target = root.join(version);
    // (The same version again, as after a failed start: the one there goes aside first, and
    // goes for good below.)
    let aside = root.join(format!(".old-{}", random()));
    if std::fs::symlink_metadata(&target).is_ok() {
        std::fs::rename(&target, &aside).map_err(|e| format!("couldn't move the old {version} aside: {e}"))?;
    }
    if let Err(e) = std::fs::rename(incoming, &target) {
        let _ = std::fs::rename(&aside, &target);
        return Err(format!("couldn't put the update in place: {e}"));
    }
    let link = root.join(format!(".current-{}", random()));
    let switched = std::os::unix::fs::symlink(version, &link).and_then(|()| std::fs::rename(&link, &current));
    if let Err(e) = switched {
        let _ = std::fs::remove_file(&link);
        return Err(format!("couldn't switch to the new version: {e}"));
    }
    let _ = std::fs::remove_dir_all(&aside);
    Ok(previous.filter(|p| p != version))
}

/// The versions' folders that aren't `current` or the one before go (and anything an update that
/// stopped half way left).
fn tidy(root: &Path, current: &str, previous: Option<&str>) {
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let leftover = name.starts_with(".incoming-") || name.starts_with(".old-") || name.starts_with(".current-");
        let version = plain_name(&name) && name.chars().next().is_some_and(|c| c.is_ascii_digit());
        if name == "current" || name == current || Some(name.as_str()) == previous || !(leftover || version) {
            continue;
        }
        // (A real folder or a stray link of ours, never what a link points at.)
        match std::fs::symlink_metadata(e.path()) {
            Ok(md) if md.is_dir() => {
                let _ = std::fs::remove_dir_all(e.path());
            }
            Ok(_) => {
                let _ = std::fs::remove_file(e.path());
            }
            Err(_) => {}
        }
    }
}

/// Where the desktop finds Pyr: your apps menu (a .desktop file that opens `current`'s program,
/// and takes pyr:// invite links), its icon, and `pyr` in a terminal (~/.local/bin). Done after
/// every update (cheap, and puts back anything that went). What didn't work, for the log.
pub fn integrate(root: &Path) -> Vec<String> {
    let (Some(data), Some(home)) = (data_home(), home()) else { return vec!["no home folder".into()] };
    integrate_into(root, &data, &home.join(".local").join("bin"), true)
}

/// `integrate` with the folders given; `desktop`: also tell the desktop (its list of apps, and
/// what opens pyr:// links), which tests don't.
pub fn integrate_into(root: &Path, data: &Path, bin: &Path, desktop: bool) -> Vec<String> {
    let mut notes = Vec::new();
    let cur = root.join("current");
    // The apps menu's entry (yours, in your folder: it comes before one the whole PC has).
    let apps = data.join("applications");
    let template = std::fs::read_to_string(cur.join("pyr.desktop")).unwrap_or_else(|_| DESKTOP.to_string());
    let exec = program(root);
    let entry = desktop_entry(&template, &exec);
    if std::fs::read_to_string(apps.join("pyr.desktop")).ok().as_deref() != Some(entry.as_str()) {
        if let Err(e) = make_dir(&apps).and_then(|()| write_atomic(&apps.join("pyr.desktop"), entry.as_bytes())) {
            notes.push(format!("the apps menu's entry: {e}"));
        }
    }
    // The icon, at each size.
    for n in [16, 32, 48, 64, 128, 256, 512] {
        let from = cur.join("icons").join(format!("pyr-{n}.png"));
        let Ok(png) = std::fs::read(&from) else { continue };
        let dir = data.join("icons").join("hicolor").join(format!("{n}x{n}")).join("apps");
        let to = dir.join("pyr.png");
        if std::fs::read(&to).ok().as_deref() != Some(png.as_slice()) {
            if let Err(e) = make_dir(&dir).and_then(|()| write_atomic(&to, &png)) {
                notes.push(format!("the icon ({n}): {e}"));
            }
        }
    }
    // `pyr` in a terminal: a link to `current`'s program (never over a file of yours).
    let link = bin.join(PROGRAM);
    match std::fs::symlink_metadata(&link) {
        Ok(md) if !md.file_type().is_symlink() => notes.push(format!("{} is a file of yours: left as it is", link.display())),
        Ok(_) if std::fs::read_link(&link).ok().as_deref() == Some(exec.as_path()) => {}
        _ => {
            let tmp = bin.join(format!(".pyr-{}", random()));
            let made = make_dir(bin).and_then(|()| {
                std::os::unix::fs::symlink(&exec, &tmp).and_then(|()| std::fs::rename(&tmp, &link)).map_err(|e| e.to_string())
            });
            if let Err(e) = made {
                let _ = std::fs::remove_file(&tmp);
                notes.push(format!("the `pyr` command: {e}"));
            }
        }
    }
    // The desktop's list of apps and which one opens pyr:// links (best effort: not every
    // desktop has these tools, and each reads the files above anyway).
    if !desktop {
        return notes;
    }
    let _ = std::process::Command::new("update-desktop-database").arg(&apps).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
    for scheme in [pyr_proto::APP_ID, pyr_proto::legacy::SCHEME.trim_end_matches("://")] {
        let _ = std::process::Command::new("xdg-mime")
            .args(["default", "pyr.desktop", &format!("x-scheme-handler/{scheme}")])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    notes
}

/// The apps menu's entry when a version's folder has none of its own.
const DESKTOP: &str = include_str!("../../../installer/linux/pyr.desktop");

/// The entry with `exec` as what it opens (quoted, as the desktop entry rules ask for a path with
/// spaces in it), and invite links from before the rename too.
fn desktop_entry(template: &str, exec: &Path) -> String {
    let quoted = format!("\"{}\"", exec.display().to_string().replace('\\', "\\\\").replace('"', "\\\"").replace('`', "\\`").replace('$', "\\$"));
    let legacy = format!("x-scheme-handler/{};", pyr_proto::legacy::SCHEME.trim_end_matches("://"));
    let mut out = String::new();
    for line in template.lines() {
        if let Some(rest) = line.strip_prefix("Exec=") {
            let args = rest.split_once(' ').map(|(_, a)| format!(" {a}")).unwrap_or_default();
            out.push_str(&format!("Exec={quoted}{args}\n"));
            out.push_str(&format!("TryExec={}\n", exec.display()));
        } else if line.starts_with("TryExec=") {
        } else if line.starts_with("MimeType=") && !line.contains(&legacy) {
            out.push_str(&format!("{line}{legacy}\n"));
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Written whole or not at all (a new file renamed over the old).
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_file_name(format!(".{}.{}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), random()));
    std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pyr-home-{name}-{}-{}", std::process::id(), random()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A version's folder as a .tar.gz: (path, contents, runnable) each.
    fn tarball(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        for (path, data, run) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(if *run { 0o755 } else { 0o644 });
            h.set_entry_type(tar::EntryType::Regular);
            h.set_cksum();
            b.append_data(&mut h, path, *data).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    fn version(v: &str) -> Vec<u8> {
        let marker = format!("{{\"version\": \"{v}\"}}");
        tarball(&[
            ("pyr", format!("program {v}").as_bytes(), true),
            ("pyr-mcp", b"connector", true),
            ("vps/pyr-server-linux-x86_64", b"server", true),
            ("icons/pyr-64.png", format!("icon {v}").as_bytes(), false),
            ("pyr.desktop", DESKTOP.as_bytes(), false),
            (MARKER, marker.as_bytes(), false),
        ])
    }

    fn put(dir: &Path, name: &str, bytes: &[u8]) -> (PathBuf, String) {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        (p, pyr_proto::release::sha256_hex(bytes))
    }

    #[test]
    fn an_update_goes_in_beside_the_last_and_current_moves_to_it() {
        let d = tmp("update");
        let root = d.join("share/pyr/app");
        for (i, v) in ["0.16.7", "0.16.8", "0.16.9"].iter().enumerate() {
            let (t, sha) = put(&d, &format!("{v}.tar.gz"), &version(v));
            let dir = install(&t, &sha, v, &root).unwrap();
            assert_eq!(dir, root.join(v));
            assert_eq!(std::fs::read_link(root.join("current")).unwrap(), PathBuf::from(v));
            assert_eq!(std::fs::read(program(&root)).unwrap(), format!("program {v}").as_bytes());
            let mode = |f: &str| std::fs::metadata(root.join(v).join(f)).unwrap().permissions().mode() & 0o777;
            assert_eq!((mode("pyr"), mode("vps/pyr-server-linux-x86_64"), mode("icons/pyr-64.png")), (0o755, 0o755, 0o644));
            // The one before stays (the Pyr that's updating runs from it); the ones before that go.
            let mut names: Vec<String> = std::fs::read_dir(&root).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            let want: Vec<&str> = match i {
                0 => vec!["0.16.7", "current"],
                1 => vec!["0.16.7", "0.16.8", "current"],
                _ => vec!["0.16.8", "0.16.9", "current"],
            };
            assert_eq!(names, want);
        }
        // The same version again (as after a start that went wrong): put back whole.
        let (t, sha) = put(&d, "again.tar.gz", &version("0.16.9"));
        install(&t, &sha, "0.16.9", &root).unwrap();
        assert_eq!(std::fs::read(program(&root)).unwrap(), b"program 0.16.9");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_wrong_or_strange_download_is_never_installed() {
        let d = tmp("refuse");
        let root = d.join("app");
        let (t, sha) = put(&d, "good.tar.gz", &version("0.16.7"));
        install(&t, &sha, "0.16.7", &root).unwrap();
        let before = std::fs::read_link(root.join("current")).unwrap();
        let bad: Vec<(&str, Vec<u8>, &str)> = vec![
            ("changed", version("0.16.8"), "changed on disk"),
            ("another version", version("0.16.9"), "not 0.16.8"),
            ("no program", tarball(&[(MARKER, br#"{"version": "0.16.8"}"#, false)]), "no Pyr program"),
            ("program not runnable", tarball(&[("pyr", b"x", false), (MARKER, br#"{"version": "0.16.8"}"#, false)]), "no Pyr program"),
            ("no marker", tarball(&[("pyr", b"x", true)]), "isn't Pyr for your home folder"),
            ("not a tarball", b"MZ this is an .exe".to_vec(), "isn't a Pyr update"),
        ];
        for (what, bytes, says) in bad {
            let (t, mut sha) = put(&d, "bad.tar.gz", &bytes);
            if what == "changed" {
                sha = pyr_proto::release::sha256_hex(b"something else");
            }
            let e = install(&t, &sha, "0.16.8", &root).unwrap_err();
            assert!(e.contains(says), "{what}: {e}");
            assert_eq!(std::fs::read_link(root.join("current")).unwrap(), before, "{what}: still the one before");
            assert!(!root.join("0.16.8").exists(), "{what}");
        }
        // A path out of its folder, and a link: refused by name (the tar writer won't make the
        // first, so it's written by hand).
        for (name, link) in [("../evil", false), ("/etc/evil", false), ("pyr-link", true)] {
            let mut b = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
            let mut h = tar::Header::new_gnu();
            if link {
                h.set_entry_type(tar::EntryType::Symlink);
                h.set_size(0);
                h.set_link_name("/usr/bin/sh").unwrap();
                h.set_path(name).unwrap();
                h.set_cksum();
                b.append(&h, std::io::empty()).unwrap();
            } else {
                h.set_entry_type(tar::EntryType::Regular);
                h.set_size(1);
                h.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name.as_bytes());
                h.set_cksum();
                b.append(&h, &b"x"[..]).unwrap();
            }
            let bytes = b.into_inner().unwrap().finish().unwrap();
            let (t, sha) = put(&d, "bad.tar.gz", &bytes);
            let e = install(&t, &sha, "0.16.8", &root).unwrap_err();
            assert!(e.contains("where none can go") || e.contains("isn't a file or a folder"), "{name}: {e}");
            assert!(!d.join("evil").exists() && !Path::new("/etc/evil").exists());
        }
        // Nothing half done is left about.
        let left: Vec<String> = std::fs::read_dir(&root).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with('.')).collect();
        assert!(left.is_empty(), "{left:?}");
        assert!(install(&t, &sha, "../0.16.8", &root).is_err(), "a version is a name, never a path");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// As `tar -C <folder> -cf - .` makes it (build-tar.sh): "./" first, then "./pyr" and so on.
    #[test]
    fn an_archive_of_dot_slash_paths_unpacks() {
        let d = tmp("dot");
        let mut b = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast()));
        let mut dir = tar::Header::new_gnu();
        dir.set_entry_type(tar::EntryType::Directory);
        dir.set_mode(0o755);
        dir.set_size(0);
        for p in ["./", "./vps/"] {
            dir.set_path(p).unwrap();
            dir.set_cksum();
            b.append(&dir, std::io::empty()).unwrap();
        }
        for (p, data, mode) in [("./pyr", &b"program"[..], 0o755), ("./vps/pyr-server-linux-x86_64", b"server", 0o755), ("./pyr-home.json", br#"{"version": "0.16.7"}"#, 0o644)] {
            let mut h = tar::Header::new_gnu();
            h.set_entry_type(tar::EntryType::Regular);
            h.set_mode(mode);
            h.set_size(data.len() as u64);
            h.set_path(p).unwrap();
            h.set_cksum();
            b.append(&h, data).unwrap();
        }
        let (t, sha) = put(&d, "dot.tar.gz", &b.into_inner().unwrap().finish().unwrap());
        let root = d.join("app");
        install(&t, &sha, "0.16.7", &root).unwrap();
        assert_eq!(std::fs::read(root.join("0.16.7/vps/pyr-server-linux-x86_64")).unwrap(), b"server");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_home_install_is_told_by_where_its_program_is() {
        let d = tmp("where");
        let root = d.join("app");
        let (t, sha) = put(&d, "v.tar.gz", &version("0.16.7"));
        install(&t, &sha, "0.16.7", &root).unwrap();
        let exe = root.join("0.16.7").join("pyr");
        assert_eq!(from_exe(&exe, &root), Some(std::fs::canonicalize(&root).unwrap()));
        // Pyr unpacked somewhere else, or with no marker, isn't.
        std::fs::create_dir_all(d.join("elsewhere/0.16.7")).unwrap();
        std::fs::copy(root.join("0.16.7").join(MARKER), d.join("elsewhere/0.16.7").join(MARKER)).unwrap();
        assert_eq!(from_exe(&d.join("elsewhere/0.16.7/pyr"), &root), None);
        assert_eq!(from_exe(Path::new("/usr/bin/pyr"), &root), None);
        // A path kept in Claude's settings is `current`'s, which outlasts the version's folder.
        assert_eq!(lasting(&root.join("0.16.7").join("pyr-mcp")), root.join("current").join("pyr-mcp"));
        assert_eq!(lasting(Path::new("/usr/bin/pyr-mcp")), PathBuf::from("/usr/bin/pyr-mcp"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn the_desktop_finds_it_and_your_own_files_are_left_alone() {
        let d = tmp("desk");
        let root = d.join("data/pyr/app");
        let (t, sha) = put(&d, "v.tar.gz", &version("0.16.7"));
        install(&t, &sha, "0.16.7", &root).unwrap();
        let (data, bin) = (d.join("data"), d.join("bin"));
        let notes = integrate_into(&root, &data, &bin, false);
        assert!(notes.is_empty(), "{notes:?}");
        let entry = std::fs::read_to_string(data.join("applications/pyr.desktop")).unwrap();
        let exec = program(&root);
        assert!(entry.contains(&format!("Exec=\"{}\" %u\n", exec.display())), "{entry}");
        assert!(entry.contains(&format!("TryExec={}\n", exec.display())));
        assert!(entry.contains("MimeType=x-scheme-handler/pyr;x-scheme-handler/ingle;"), "{entry}");
        assert_eq!(entry.matches("Exec=").count(), 2, "one Exec, one TryExec");
        assert_eq!(std::fs::read(data.join("icons/hicolor/64x64/apps/pyr.png")).unwrap(), b"icon 0.16.7");
        assert_eq!(std::fs::read_link(bin.join("pyr")).unwrap(), exec);
        // Again after the next update: the same entry and link, the new icon.
        let (t, sha) = put(&d, "v2.tar.gz", &version("0.16.8"));
        install(&t, &sha, "0.16.8", &root).unwrap();
        assert!(integrate_into(&root, &data, &bin, false).is_empty());
        assert_eq!(std::fs::read_to_string(data.join("applications/pyr.desktop")).unwrap(), entry);
        assert_eq!(std::fs::read(data.join("icons/hicolor/64x64/apps/pyr.png")).unwrap(), b"icon 0.16.8");
        // A `pyr` of your own in ~/.local/bin stays yours.
        std::fs::remove_file(bin.join("pyr")).unwrap();
        std::fs::write(bin.join("pyr"), b"#!/bin/sh\necho mine\n").unwrap();
        let notes = integrate_into(&root, &data, &bin, false);
        assert!(notes.iter().any(|n| n.contains("file of yours")), "{notes:?}");
        assert_eq!(std::fs::read(bin.join("pyr")).unwrap(), b"#!/bin/sh\necho mine\n");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_path_with_spaces_is_quoted_for_the_desktop() {
        let e = desktop_entry(DESKTOP, Path::new("/home/a b/.local/share/pyr/app/current/pyr"));
        assert!(e.contains("Exec=\"/home/a b/.local/share/pyr/app/current/pyr\" %u\n"), "{e}");
    }
}
