//! A plugin's source, gathered from its folder to send in for the Pyr Library, with the
//! checks Pyr makes first (Rust only: a Cargo.toml and .rs files, no programs or C code
//! inside); and the plugin kit, written into a new folder to start an add-on from. Used by
//! Pyr's Plugins page and by the Claude connector.

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

/// Folders that are never part of the source.
const SKIP_DIRS: &[&str] = &["target", ".git", ".github", ".idea", ".vscode", "node_modules", "dist"];
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

/// Reads a project folder (skipping build output and version control).
pub fn gather(folder: &Path) -> Gathered {
    let mut g = Gathered { folder: folder.to_path_buf(), ..Default::default() };
    let mut stack = vec![folder.to_path_buf()];
    let mut seen_rs = false;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            g.problems.push(format!("couldn't read {}", dir.display()));
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let path = e.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
            let name = e.file_name().to_string_lossy().to_string();
            if meta.file_type().is_symlink() {
                g.notes.push(format!("{name}: a link, left out"));
                continue;
            }
            if meta.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push(path);
                }
                continue;
            }
            let rel: Vec<String> = path.strip_prefix(folder).unwrap_or(&path).components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
            let rel = rel.join("/");
            let e = ext(&path);
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
            if name == "build.rs" {
                g.notes.push(format!("{rel}: a build script (it runs while the plugin is built, so it's looked at closely)"));
            }
            match std::fs::read(&path) {
                Ok(bytes) => {
                    g.size += bytes.len();
                    g.files.push(SourceFile { path: rel, data: pyr_proto::b64::encode(&bytes) });
                }
                Err(e) => g.problems.push(format!("{rel}: couldn't read it ({e})")),
            }
        }
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
        None => g.problems.push("no Cargo.toml at the top of the folder: pick the plugin's own folder".into()),
        Some(f) => {
            let text = String::from_utf8(pyr_proto::b64::decode(&f.data).unwrap_or_default()).unwrap_or_default();
            read_manifest(&text, &mut g);
        }
    }
    g
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
}
