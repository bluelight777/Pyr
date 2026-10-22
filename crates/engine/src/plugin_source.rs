//! A plugin's source, gathered to send in for the Pyr Library from its folder or from a .zip
//! of it (its Cargo.toml at the top, or in a folder inside), with the checks Pyr makes first
//! (Rust only: a Cargo.toml and .rs files, no programs or C code inside); and the plugin kit,
//! written into a new folder to start an add-on from. Used by Pyr's Plugins page and by the
//! Claude connector.

use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use pyr_proto::control::{PluginSubmission, SUBMISSION_FILES_MAX, SUBMISSION_MAX, SourceFile};

/// What's found in a folder: the files to send, what Cargo.toml says, and what's wrong (if
/// anything is, it can't be sent) or worth a look.
#[derive(Clone, Debug, Default)]
pub struct Gathered {
    pub folder: PathBuf,
    pub files: Vec<SourceFile>,
    pub size: usize,
    pub name: String,
    pub kind: String,
    /// Its kind is settled by its source (its Cargo.toml says, or it's built with Pyr's
    /// add-on kit), so the form to send it in doesn't ask (Maanix's add-on went in as a program).
    pub kind_fixed: bool,
    pub version: String,
    pub summary: String,
    pub problems: Vec<String>,
    pub notes: Vec<String>,
}

/// Folders that are never part of the source (`.cargo` neither: settings in it would change how
/// the Library builds it; `__MACOSX` is what a Mac adds to a .zip).
const SKIP_DIRS: &[&str] = &["target", ".git", ".github", ".idea", ".vscode", "node_modules", "dist", ".cargo", "__MACOSX"];
/// Files left out wherever they are: what Finder and File Explorer leave in folders.
const SKIP_FILES: &[&str] = &[".DS_Store", "Thumbs.db", "desktop.ini"];
/// Files looked through at most (a folder chosen by mistake, a whole drive say, stops here).
const LIST_MAX: usize = 5_000;
/// Never sent: built programs and libraries, object files, archives.
const BINARIES: &[&str] = &[
    "exe", "dll", "so", "dylib", "a", "lib", "o", "obj", "rlib", "wasm", "pdb", "jar", "class", "pyc", "msi", "deb", "rpm", "zip", "7z", "rar",
    "gz", "xz", "tar", "bin", "com", "scr", "sys", "node",
];
/// Not Rust: C, C++, Objective-C and assembly (Rust only: no C header, no build of other code).
const OTHER_CODE: &[&str] = &["c", "cc", "cpp", "cxx", "h", "hh", "hpp", "hxx", "m", "mm", "s", "asm", "go", "py", "ps1", "bat", "cmd", "sh", "vbs"];

fn ext(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default()
}

/// A file found: where it is (from the top of what was chosen, with `/`), how big it says it
/// is, and where to read it from.
struct Found {
    rel: String,
    size: u64,
    at: At,
    link: bool,
    /// In a .zip, locked with a password.
    locked: bool,
}

enum At {
    Disk(PathBuf),
    Zipped(usize),
}

/// Reads a plugin's source from its folder or from a .zip of it, leaving out build output and
/// version control. Its Cargo.toml can be at the top, or in a folder inside (a .zip of the
/// plugin's folder, or the folder it's in): the one folder with the shallowest Cargo.toml is
/// what's sent.
pub fn gather(path: &Path) -> Gathered {
    let mut g = Gathered { folder: path.to_path_buf(), ..Default::default() };
    if path.is_dir() {
        let found = list_folder(path, &mut g);
        take(found, &mut g, |f| match &f.at {
            At::Disk(p) => std::fs::read(p).map_err(|e| e.to_string()),
            At::Zipped(_) => Err("not in a .zip".into()),
        });
    } else if ext(path) == "zip" {
        let opened = std::fs::File::open(path).map_err(|e| e.to_string()).and_then(|f| zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|e| e.to_string()));
        match opened {
            Ok(mut z) => {
                let found = list_zip(&mut z, &mut g);
                take(found, &mut g, |f| match f.at {
                    At::Zipped(i) => read_zipped(&mut z, i, f.size),
                    At::Disk(_) => Err("not on disk".into()),
                });
            }
            Err(e) => g.problems.push(format!("the .zip can't be opened ({e})")),
        }
    } else if path.exists() {
        g.problems.push("that's a file: choose the plugin's folder, or a .zip of it".into());
    } else {
        g.problems.push(format!("there's nothing at {}", path.display()));
    }
    g
}

/// Every file in a folder and the folders in it (not the ones never sent, nor links).
fn list_folder(folder: &Path, g: &mut Gathered) -> Vec<Found> {
    let mut out = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    let mut walked = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            g.problems.push(format!("couldn't read {}", dir.display()));
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            walked += 1;
            if walked > LIST_MAX {
                g.problems.push(format!("more than {LIST_MAX} files in it: choose the plugin's own folder"));
                return out;
            }
            let path = e.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
            let name = e.file_name().to_string_lossy().to_string();
            let rel = path.strip_prefix(folder).unwrap_or(&path).components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/");
            if meta.file_type().is_symlink() {
                out.push(Found { rel, size: 0, at: At::Disk(path), link: true, locked: false });
                continue;
            }
            if meta.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push(path);
                }
                continue;
            }
            if SKIP_FILES.contains(&name.as_str()) {
                continue;
            }
            out.push(Found { rel, size: meta.len(), at: At::Disk(path), link: false, locked: false });
        }
    }
    out
}

/// Every file in a .zip, by its place inside (nothing is unpacked: a name that would reach
/// outside it, an absolute one or a drive letter, is left out).
fn list_zip<R: Read + Seek>(z: &mut zip::ZipArchive<R>, g: &mut Gathered) -> Vec<Found> {
    let mut out = Vec::new();
    if z.len() > LIST_MAX {
        g.problems.push(format!("{} files in the .zip: a plugin's source is far fewer (leave out its target folder)", z.len()));
        return out;
    }
    for i in 0..z.len() {
        let Ok(f) = z.by_index_raw(i) else {
            g.problems.push("the .zip is damaged".into());
            return out;
        };
        let name = f.name().map(|n| n.replace('\\', "/")).unwrap_or_default();
        if f.is_dir() || name.ends_with('/') || name.is_empty() {
            continue;
        }
        let parts: Vec<&str> = name.split('/').filter(|p| !p.is_empty()).collect();
        let plain = !name.starts_with('/') && !parts.is_empty() && parts.iter().all(|p| *p != "." && *p != ".." && !p.contains(':') && !p.contains('\0'));
        if !plain {
            g.notes.push(format!("{name}: not a plain path in the .zip, left out"));
            continue;
        }
        let (file, dirs) = parts.split_last().unwrap_or((&"", &[]));
        if dirs.iter().any(|d| SKIP_DIRS.contains(d)) || SKIP_FILES.contains(file) || file.starts_with("._") {
            continue;
        }
        out.push(Found { rel: parts.join("/"), size: f.size(), at: At::Zipped(i), link: f.is_symlink(), locked: f.encrypted() });
    }
    out
}

/// One file out of a .zip: no more than it says it holds (and never more than a plugin can be).
fn read_zipped<R: Read + Seek>(z: &mut zip::ZipArchive<R>, i: usize, size: u64) -> Result<Vec<u8>, String> {
    let f = z.by_index(i).map_err(|e| e.to_string())?;
    let mut bytes = Vec::with_capacity(size.min(SUBMISSION_MAX as u64) as usize);
    f.take(size.min(SUBMISSION_MAX as u64) + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if bytes.len() as u64 != size {
        return Err("it isn't the size the .zip says".into());
    }
    Ok(bytes)
}

/// Where the plugin's own folder is in what was found: the top when its Cargo.toml is there,
/// otherwise the one folder with the shallowest Cargo.toml ("" or "name/"; None: not found).
fn find_root(found: &[Found], g: &mut Gathered) -> Option<String> {
    if found.iter().any(|f| f.rel == "Cargo.toml" && !f.link) {
        return Some(String::new());
    }
    let tomls: Vec<&str> = found.iter().filter(|f| !f.link).filter_map(|f| f.rel.strip_suffix("/Cargo.toml")).collect();
    let Some(depth) = tomls.iter().map(|d| d.matches('/').count()).min() else {
        g.problems.push("no Cargo.toml in it: choose the plugin's own folder (the one with its Cargo.toml), or a .zip of it".into());
        return None;
    };
    let top: Vec<&str> = tomls.into_iter().filter(|d| d.matches('/').count() == depth).collect();
    if top.len() > 1 {
        let some = top.iter().take(3).copied().collect::<Vec<_>>().join(", ");
        g.problems.push(format!("{} projects in it ({some}{}): choose the plugin's own folder", top.len(), if top.len() > 3 { ", …" } else { "" }));
        return None;
    }
    g.notes.push(format!("its Cargo.toml is in {}, so that folder is what's sent", top[0]));
    Some(format!("{}/", top[0]))
}

/// The plugin's own files out of what was found, read and checked.
fn take(mut found: Vec<Found>, g: &mut Gathered, mut read: impl FnMut(&Found) -> Result<Vec<u8>, String>) {
    let Some(root) = find_root(&found, g) else {
        // (Nothing read: just whether there's any Rust at all.)
        if !found.iter().any(|f| f.rel.ends_with(".rs")) {
            g.problems.push("no Rust (.rs) files: plugins are written in Rust".into());
        }
        return;
    };
    if !root.is_empty() && g.folder.is_dir() {
        g.folder = g.folder.join(root.trim_end_matches('/'));
    }
    // Its Cargo.toml first (read whatever else there is).
    let toml = format!("{root}Cargo.toml");
    found.sort_by_key(|f| f.rel != toml);
    let mut seen_rs = false;
    let mut outside = 0usize;
    // What it all comes to (as each file says), read only while it could still be sent.
    let mut total = 0u64;
    for f in &found {
        let Some(rel) = f.rel.strip_prefix(root.as_str()) else {
            outside += 1;
            continue;
        };
        if f.link {
            g.notes.push(format!("{rel}: a link, left out"));
            continue;
        }
        let e = ext(Path::new(rel));
        if BINARIES.contains(&e.as_str()) {
            g.problems.push(format!("{rel}: a built file. Plugins are sent as source only (Rust that's built from it)."));
            continue;
        }
        if OTHER_CODE.contains(&e.as_str()) {
            g.problems.push(format!("{rel}: not Rust. Plugins are Rust only."));
            continue;
        }
        if e == "rs" {
            seen_rs = true;
        }
        if rel.rsplit('/').next() == Some("build.rs") {
            g.notes.push(format!("{rel}: a build script (it runs while the plugin is built, so it's looked at closely)"));
        }
        if f.locked {
            g.problems.push(format!("{rel}: locked with a password in the .zip"));
            continue;
        }
        total += f.size;
        if total > SUBMISSION_MAX as u64 {
            continue;
        }
        match read(f) {
            Ok(bytes) => g.files.push(SourceFile { path: rel.to_string(), data: pyr_proto::b64::encode(&bytes) }),
            Err(e) => g.problems.push(format!("{rel}: couldn't read it ({e})")),
        }
    }
    g.size = total.min(usize::MAX as u64) as usize;
    if outside > 0 && !root.is_empty() {
        g.notes.push(format!("{outside} file{} outside {} left out", if outside == 1 { "" } else { "s" }, root.trim_end_matches('/')));
    }
    g.files.sort_by(|a, b| a.path.cmp(&b.path));
    if g.files.len() > SUBMISSION_FILES_MAX {
        g.problems.push(format!("{} files: a plugin can have up to {SUBMISSION_FILES_MAX}", g.files.len()));
    }
    if g.size > SUBMISSION_MAX {
        g.problems.push(format!("{} KB: a plugin's source can be up to {} KB (leave out pictures you don't need)", g.size / 1000, SUBMISSION_MAX / 1000));
    }
    if !seen_rs {
        g.problems.push("no Rust (.rs) files: plugins are written in Rust".into());
    }
    match g.files.iter().find(|f| f.path == "Cargo.toml") {
        // (Only when it couldn't be read: find_root saw it.)
        None => g.problems.push("its Cargo.toml couldn't be read".into()),
        Some(f) => {
            let text = String::from_utf8(pyr_proto::b64::decode(&f.data).unwrap_or_default()).unwrap_or_default();
            read_manifest(&text, g);
        }
    }
}

/// The name, version and what [package.metadata.pyr] says, from Cargo.toml.
fn read_manifest(text: &str, g: &mut Gathered) {
    let Ok(doc) = text.parse::<toml::Table>() else {
        g.problems.push("Cargo.toml can't be read".into());
        return;
    };
    let pkg = doc.get("package").and_then(|p| p.as_table());
    let s = |t: Option<&toml::Table>, k: &str| t.and_then(|t| t.get(k)).and_then(|v| v.as_str()).unwrap_or("").to_string();
    g.version = s(pkg, "version");
    // (Or under the name from before the rename: plugins made then, see `pyr_proto::legacy`.)
    let meta = pkg.and_then(|p| p.get("metadata"));
    let pyr = meta.and_then(|m| m.get("pyr").or_else(|| m.get(pyr_proto::legacy::APP_ID))).and_then(|i| i.as_table());
    g.name = s(pyr, "name");
    if g.name.is_empty() {
        g.name = s(pkg, "name");
    }
    g.kind = s(pyr, "kind");
    // Built with the add-on kit: an add-on, whatever else it says.
    let kit = doc.get("dependencies").is_some_and(|d| d.get("pyr-plugin").is_some() || d.get(pyr_proto::legacy::KIT_CRATE).is_some());
    g.kind_fixed = kit || matches!(g.kind.as_str(), "addon" | "program");
    if kit {
        g.kind = "addon".into();
    } else if !g.kind_fixed {
        // An add-on is a library built to WebAssembly; anything else is a program.
        g.kind = if doc.get("lib").is_some() { "addon".into() } else { "program".into() };
    }
    g.summary = s(pyr, "summary");
    if g.summary.is_empty() {
        g.summary = s(pkg, "description");
    }
}

impl Gathered {
    pub fn submission(&self, author: &str, contact: &str) -> PluginSubmission {
        PluginSubmission {
            name: self.name.clone(),
            kind: self.kind.clone(),
            version: self.version.clone(),
            summary: self.summary.clone(),
            author: author.trim().to_string(),
            contact: contact.trim().to_string(),
            app_version: String::new(),
            files: self.files.clone(),
            ..Default::default()
        }
    }
}

/// The plugin kit and the starter add-on, as files (for "Start an add-on").
const KIT: &[(&str, &str)] = &[
    ("pyr-plugin/Cargo.toml", include_str!("../../../plugins/kit/Cargo.toml")),
    ("pyr-plugin/src/lib.rs", include_str!("../../../plugins/kit/src/lib.rs")),
    ("src/lib.rs", include_str!("../../../plugins/template/src/lib.rs")),
    ("CLAUDE.md", include_str!("../../../plugins/template/CLAUDE.md")),
    ("README.md", include_str!("../../../plugins/template/README.md")),
];

const TEMPLATE_MANIFEST: &str = include_str!("../../../plugins/template/Cargo.toml");

/// How plugins are made: the plugin kit's README, then the CLAUDE.md that teaches Claude how
/// add-ons work (for the Claude connector's guide).
pub fn guide() -> String {
    format!("{}\n\n---\n\n{}", include_str!("../../../plugins/README.md"), include_str!("../../../plugins/template/CLAUDE.md"))
}

/// Writes a new add-on project (the kit included, so it builds without fetching it) into
/// `folder/<name>`, and returns that folder.
pub fn start_addon(folder: &Path, name: &str) -> Result<PathBuf, String> {
    let slug: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        return Err("give it a name with some letters in it".into());
    }
    let dir = folder.join(&slug);
    if dir.exists() {
        return Err(format!("there's already a folder called {slug} there"));
    }
    let manifest = TEMPLATE_MANIFEST
        .replace("name = \"pyr-hello\"", &format!("name = \"{slug}\""))
        .replace("pyr-plugin = { path = \"../kit\" }", "pyr-plugin = { path = \"pyr-plugin\" }")
        .replace("name = \"Hello\"", &format!("name = \"{}\"", name.replace('"', "'")))
        .replace("# The kit (Pyr's \"Start an add-on\" puts a copy of it in a pyr-plugin folder next to this).\n", "# The plugin kit (a copy of it, in the pyr-plugin folder).\n");
    // A project of its own, outside any other Cargo workspace, built small.
    let manifest = format!(
        "{manifest}\n[workspace]\n\n# A small .wasm.\n[profile.release]\nopt-level = \"s\"\nlto = true\ncodegen-units = 1\npanic = \"abort\"\nstrip = true\n"
    );
    let write = |rel: &str, text: &str| -> Result<(), String> {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, text).map_err(|e| format!("couldn't write {rel}: {e}"))
    };
    write("Cargo.toml", &manifest)?;
    write(".gitignore", "/target\n")?;
    for (rel, text) in KIT {
        if *rel == "src/lib.rs" {
            // Its name in the code too ({:?} writes it as a Rust string, quotes escaped).
            write(rel, &text.replace("Info::new(\"Hello\")", &format!("Info::new({name:?})")))?;
        } else {
            write(rel, text)?;
        }
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pyr-source-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn its_kind_is_what_its_cargo_toml_says() {
        let kind = |toml: &str| {
            let mut g = Gathered::default();
            read_manifest(toml, &mut g);
            (g.kind, g.kind_fixed)
        };
        let kit = "[package]\nname = \"poker\"\n[package.metadata.pyr]\nkind = \"program\"\n[dependencies]\npyr-plugin = { path = \"pyr-plugin\" }\n";
        assert_eq!(kind(kit), ("addon".to_string(), true), "built with the kit: an add-on");
        // Made before the rename: the old kit and the old table name.
        let (id, crate_) = (pyr_proto::legacy::APP_ID, pyr_proto::legacy::KIT_CRATE);
        let old = format!("[package]\nname = \"poker\"\n[package.metadata.{id}]\nkind = \"program\"\n[dependencies]\n{crate_} = {{ path = \"{crate_}\" }}\n");
        assert_eq!(kind(&old), ("addon".to_string(), true), "built with the old kit: an add-on");
        assert_eq!(kind(&format!("[package]\nname = \"x\"\n[package.metadata.{id}]\nkind = \"program\"\n")), ("program".to_string(), true));
        assert_eq!(kind("[package]\nname = \"x\"\n[package.metadata.pyr]\nkind = \"program\"\n"), ("program".to_string(), true));
        assert_eq!(kind("[package]\nname = \"x\"\n[lib]\ncrate-type = [\"cdylib\"]\n"), ("addon".to_string(), false), "a guess: asked");
        assert_eq!(kind("[package]\nname = \"x\"\n"), ("program".to_string(), false));
    }

    #[test]
    fn a_rust_project_is_gathered_and_anything_else_is_named() {
        let dir = temp_dir("gather");
        let p = start_addon(&dir, "Coin Flip!").unwrap();
        assert!(p.ends_with("coin-flip"));
        assert!(start_addon(&dir, "Coin Flip!").is_err(), "not over an existing folder");
        assert!(std::fs::read_to_string(p.join("src/lib.rs")).unwrap().contains(r#"Info::new("Coin Flip!")"#));
        std::fs::create_dir_all(p.join("target/release")).unwrap();
        std::fs::write(p.join("target/release/big.wasm"), b"built").unwrap();
        let g = gather(&p);
        assert!(g.problems.is_empty(), "{:?}", g.problems);
        assert_eq!((g.name.as_str(), g.kind.as_str(), g.kind_fixed), ("Coin Flip!", "addon", true));
        assert!(g.files.iter().any(|f| f.path == "pyr-plugin/src/lib.rs"));
        assert!(!g.files.iter().any(|f| f.path.starts_with("target")), "build output is left out");
        // Built files and other languages are refused.
        std::fs::write(p.join("helper.dll"), b"MZ").unwrap();
        std::fs::write(p.join("src/fast.c"), b"int x;").unwrap();
        let g = gather(&p);
        assert_eq!(g.problems.len(), 2, "{:?}", g.problems);
        // Not a Rust project at all.
        let other = temp_dir("notrust");
        std::fs::write(other.join("notes.txt"), b"hi").unwrap();
        let g = gather(&other);
        assert!(g.problems.iter().any(|p| p.contains("Cargo.toml")) && g.problems.iter().any(|p| p.contains("no Rust")));
        let _ = std::fs::remove_dir_all(dir);
        let _ = std::fs::remove_dir_all(other);
    }

    #[test]
    fn the_folder_it_is_in_works_too() {
        let dir = temp_dir("parent");
        let p = start_addon(&dir, "Coin Flip").unwrap();
        std::fs::write(dir.join("notes.txt"), b"mine, not the plugin's").unwrap();
        // The folder the plugin is in: its own folder is found, and only that's sent.
        let g = gather(&dir);
        assert!(g.problems.is_empty(), "{:?}", g.problems);
        assert_eq!(g.folder, p);
        assert!(g.files.iter().any(|f| f.path == "Cargo.toml") && g.files.iter().any(|f| f.path == "pyr-plugin/Cargo.toml"));
        assert!(!g.files.iter().any(|f| f.path.contains("notes.txt")));
        assert!(g.notes.iter().any(|n| n.contains("coin-flip")), "{:?}", g.notes);
        // Two plugins side by side: which one is meant isn't guessed.
        start_addon(&dir, "Dice Two").unwrap();
        let g = gather(&dir);
        assert!(g.problems.iter().any(|p| p.contains("2 projects")), "{:?}", g.problems);
        // Settings for cargo aren't sent (they'd change how the Library builds it).
        std::fs::create_dir_all(p.join(".cargo")).unwrap();
        std::fs::write(p.join(".cargo/config.toml"), b"[build]\nrustflags = [\"-Clink-arg=x\"]\n").unwrap();
        let g = gather(&p);
        assert!(g.problems.is_empty() && !g.files.iter().any(|f| f.path.starts_with(".cargo")), "{:?}", g.files.iter().map(|f| &f.path).collect::<Vec<_>>());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_zip_of_it_is_read_in_place() {
        use std::io::Write;
        use zip::write::SimpleFileOptions;
        let dir = temp_dir("zip");
        let p = start_addon(&dir, "Coin Flip").unwrap();
        let want = gather(&p);
        // As File Explorer makes it ("Send to → Compressed folder"): the folder inside, and its
        // build output and a Mac's leftovers along with it.
        let zipped = dir.join("coin-flip.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&zipped).unwrap());
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.add_directory("coin-flip/", opts).unwrap();
        for f in &want.files {
            z.start_file(format!("coin-flip/{}", f.path), opts).unwrap();
            z.write_all(&pyr_proto::b64::decode(&f.data).unwrap()).unwrap();
        }
        for (name, data) in [
            ("coin-flip/target/release/big.wasm", &b"built"[..]),
            ("__MACOSX/coin-flip/._Cargo.toml", b"mac"),
            ("coin-flip/.DS_Store", b"mac"),
            ("../escape.rs", b"// outside"),
            ("/etc/absolute.rs", b"// outside"),
        ] {
            z.start_file(name, opts).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
        let g = gather(&zipped);
        assert!(g.problems.is_empty(), "{:?}", g.problems);
        assert_eq!(g.folder, zipped, "a .zip is shown as itself");
        assert_eq!(g.files, want.files, "the same files as from the folder");
        assert_eq!((g.name.as_str(), g.kind.as_str()), ("Coin Flip", "addon"));
        assert!(g.notes.iter().filter(|n| n.contains("not a plain path")).count() == 2, "{:?}", g.notes);
        // A .zip of the plugin's files themselves (Cargo.toml at its top).
        let flat = dir.join("flat.zip");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&flat).unwrap());
        for f in &want.files {
            z.start_file(f.path.as_str(), SimpleFileOptions::default()).unwrap();
            z.write_all(&pyr_proto::b64::decode(&f.data).unwrap()).unwrap();
        }
        z.finish().unwrap();
        let g = gather(&flat);
        assert!(g.problems.is_empty() && g.files == want.files, "{:?}", g.problems);
        // Not a .zip at all, and not anything.
        std::fs::write(dir.join("fake.zip"), b"not a zip").unwrap();
        assert!(gather(&dir.join("fake.zip")).problems[0].contains("can't be opened"));
        assert!(gather(&dir.join("coin-flip/Cargo.toml")).problems[0].contains("a .zip of it"));
        assert!(gather(&dir.join("nothing-here")).problems[0].contains("nothing at"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
