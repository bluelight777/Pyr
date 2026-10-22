//! Building an add-on from its source, in a sandbox: no network, nothing of the machine's but
//! the toolchain (read-only), a time limit and a memory limit. Only the plugin kit's crates
//! can be used (vendored: the kit, serde and serde_json), and no build scripts.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};

/// Crates an add-on may use (what's vendored for the sandbox).
pub const ALLOWED_DEPS: &[&str] = &["pyr-plugin", "serde", "serde_json"];
/// The most a built add-on may be.
pub const MAX_WASM: usize = 3_000_000;

pub trait Builder {
    /// The add-on's WebAssembly, built from `files` (its Rust project).
    fn build_addon(&self, files: &[(String, Vec<u8>)]) -> Result<Vec<u8>>;
}

/// What an add-on's Cargo.toml says, checked: its package name, and what it asks for.
#[derive(Debug, PartialEq)]
pub struct Manifest {
    pub package: String,
    pub permissions: Vec<String>,
    pub version: String,
}

/// Reads an add-on's Cargo.toml, refusing what the sandbox won't build: other crates, build
/// scripts, patches, other sources, or a workspace of its own.
pub fn manifest(files: &[(String, Vec<u8>)]) -> Result<Manifest, String> {
    let text = files.iter().find(|(p, _)| p == "Cargo.toml").and_then(|(_, b)| std::str::from_utf8(b).ok()).ok_or("no readable Cargo.toml")?;
    let doc: toml::Table = text.parse().map_err(|e| format!("its Cargo.toml doesn't read: {e}"))?;
    for t in ["build-dependencies", "patch", "replace", "workspace", "target"] {
        if doc.contains_key(t) {
            return Err(format!("its Cargo.toml has [{t}], which the Library's automatic builds don't allow"));
        }
    }
    if files.iter().any(|(p, _)| p == "build.rs" || p.ends_with(".c") || p.ends_with(".cpp") || p.ends_with(".h") || p.ends_with(".s") || p.ends_with(".S")) {
        return Err("it has a build script or non-Rust code".into());
    }
    let pkg = doc.get("package").and_then(|p| p.as_table()).ok_or("its Cargo.toml has no [package]")?;
    if pkg.contains_key("build") || pkg.contains_key("links") {
        return Err("its Cargo.toml sets a build script".into());
    }
    let package = pkg.get("name").and_then(|n| n.as_str()).ok_or("its package has no name")?.to_string();
    if !package.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') || package.is_empty() || package.len() > 64 {
        return Err("its package name isn't plain".into());
    }
    let version = pkg.get("version").and_then(|n| n.as_str()).unwrap_or("").to_string();
    for section in ["dependencies", "dev-dependencies"] {
        if let Some(deps) = doc.get(section).and_then(|d| d.as_table()) {
            for (name, spec) in deps {
                if section == "dependencies" && !ALLOWED_DEPS.contains(&name.as_str()) {
                    return Err(format!("it uses the crate {name:?}: automatic builds allow only {}", ALLOWED_DEPS.join(", ")));
                }
                // Only by version or the kit's path: no git, no other registry.
                if let Some(t) = spec.as_table()
                    && (t.contains_key("git") || t.contains_key("registry") || (t.contains_key("path") && name != "pyr-plugin"))
                {
                    return Err(format!("{name} comes from somewhere other than crates.io"));
                }
            }
        }
    }
    let permissions = pkg
        .get("metadata")
        .and_then(|m| m.get("pyr"))
        .and_then(|p| p.get("permissions"))
        .and_then(|p| p.as_array())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    Ok(Manifest { package, permissions, version })
}

/// The Cargo.toml as the sandbox builds it: the kit from its folder, a workspace of its own,
/// and the Library's release settings.
pub fn sandbox_manifest(files: &[(String, Vec<u8>)]) -> Result<String> {
    let text = files.iter().find(|(p, _)| p == "Cargo.toml").and_then(|(_, b)| std::str::from_utf8(b).ok()).context("Cargo.toml")?;
    let mut doc: toml::Table = text.parse()?;
    if let Some(deps) = doc.get_mut("dependencies").and_then(|d| d.as_table_mut())
        && deps.contains_key("pyr-plugin")
    {
        let mut t = toml::Table::new();
        t.insert("path".into(), toml::Value::String("/kit".into()));
        deps.insert("pyr-plugin".into(), toml::Value::Table(t));
    }
    doc.remove("dev-dependencies");
    doc.remove("profile");
    doc.insert("workspace".into(), toml::Value::Table(toml::Table::new()));
    let mut out = toml::to_string(&doc)?;
    out.push_str("\n[profile.release]\nopt-level = \"s\"\nlto = true\ncodegen-units = 1\npanic = \"abort\"\nstrip = true\n");
    Ok(out)
}

/// Whether a WebAssembly module imports nothing (Pyr runs add-ons with no imports at all).
pub fn imports_nothing(wasm: &[u8]) -> Result<(), String> {
    if wasm.len() < 8 || &wasm[..4] != b"\0asm" {
        return Err("not WebAssembly".into());
    }
    let mut i = 8;
    let leb = |b: &[u8], i: &mut usize| -> Option<u64> {
        let (mut v, mut s) = (0u64, 0);
        loop {
            let x = *b.get(*i)?;
            *i += 1;
            v |= ((x & 0x7f) as u64) << s;
            if x & 0x80 == 0 {
                return Some(v);
            }
            s += 7;
            if s > 63 {
                return None;
            }
        }
    };
    while i < wasm.len() {
        let id = wasm[i];
        i += 1;
        let len = leb(wasm, &mut i).ok_or("bad section")? as usize;
        if id == 2 {
            let mut j = i;
            let count = leb(wasm, &mut j).ok_or("bad imports")?;
            if count > 0 {
                return Err(format!("it imports {count} things (add-ons may import nothing)"));
            }
        }
        i = i.checked_add(len).ok_or("bad section")?;
        if i > wasm.len() {
            return Err("cut short".into());
        }
    }
    Ok(())
}

/// Files sent in that the sandbox's build never sees: the lock file (the vendored crates
/// decide), and settings for cargo or rustup in the plugin's own folder (`.cargo/config.toml`
/// could change how it's linked, `rust-toolchain.toml` which compiler builds it).
pub fn left_out_of_build(path: &str) -> bool {
    path == "Cargo.lock" || path.starts_with(".cargo/") || path == ".cargo" || path == "rust-toolchain" || path == "rust-toolchain.toml"
}

/// The real thing: bubblewrap round cargo.
pub struct Sandbox {
    /// Where rustup put the toolchain (RUSTUP_HOME, CARGO_HOME): read-only inside.
    pub rustup_home: PathBuf,
    pub cargo_home: PathBuf,
    /// The plugin kit's folder, and the vendored crates (read-only inside).
    pub kit: PathBuf,
    pub vendor: PathBuf,
    /// Somewhere to build (a fresh folder in it each time).
    pub work: PathBuf,
    pub bwrap: PathBuf,
    pub timeout: Duration,
    pub memory_mb: u64,
}

impl Builder for Sandbox {
    fn build_addon(&self, files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
        let m = manifest(files).map_err(anyhow::Error::msg)?;
        let dir = self.work.join(format!("b{}", std::process::id() as u64 * 1_000_003 + std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as u64));
        let src = dir.join("src-tree");
        std::fs::create_dir_all(&src)?;
        let result = (|| -> Result<Vec<u8>> {
            for (path, bytes) in files {
                let Some(rel) = safe_rel(path) else { bail!("{path}: not a plain path") };
                if left_out_of_build(path) {
                    continue;
                }
                let to = src.join(rel);
                std::fs::create_dir_all(to.parent().context("parent")?)?;
                std::fs::write(&to, bytes)?;
            }
            std::fs::write(src.join("Cargo.toml"), sandbox_manifest(files)?)?;
            std::fs::create_dir_all(dir.join("cargo"))?;
            std::fs::write(
                dir.join("cargo/config.toml"),
                "[source.crates-io]\nreplace-with = \"vendored\"\n[source.vendored]\ndirectory = \"/vendor\"\n[net]\noffline = true\n",
            )?;
            let mut cmd = Command::new("timeout");
            cmd.arg("--kill-after=10").arg(format!("{}", self.timeout.as_secs()));
            cmd.arg("prlimit").arg(format!("--as={}", self.memory_mb * 1024 * 1024)).arg("--nproc=256").arg("--");
            cmd.arg(&self.bwrap);
            cmd.args(["--unshare-all", "--die-with-parent", "--new-session", "--clearenv"]);
            for ro in ["/usr", "/etc/alternatives", "/etc/ssl"] {
                cmd.args(["--ro-bind-try", ro, ro]);
            }
            for l in ["/lib", "/lib64", "/bin"] {
                cmd.args(["--ro-bind-try", l, l]);
            }
            cmd.arg("--ro-bind").arg(&self.rustup_home).arg("/rustup");
            cmd.arg("--ro-bind").arg(&self.cargo_home).arg("/cargo-bin");
            cmd.arg("--ro-bind").arg(&self.kit).arg("/kit");
            cmd.arg("--ro-bind").arg(&self.vendor).arg("/vendor");
            cmd.arg("--bind").arg(&src).arg("/src");
            cmd.arg("--bind").arg(dir.join("cargo")).arg("/cargo");
            cmd.args(["--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp", "--chdir", "/src"]);
            cmd.args(["--setenv", "PATH", "/cargo-bin/bin:/usr/bin:/bin", "--setenv", "HOME", "/tmp", "--setenv", "RUSTUP_HOME", "/rustup", "--setenv", "CARGO_HOME", "/cargo"]);
            cmd.args(["--setenv", "RUSTUP_TOOLCHAIN", "stable", "--setenv", "CARGO_TARGET_DIR", "/tmp/target"]);
            // The module is copied out of /tmp (which goes with the sandbox) into /src.
            cmd.args(["sh", "-c", "cargo build --release --offline --target wasm32-unknown-unknown --lib 2>&1 | tail -n 40; cp /tmp/target/wasm32-unknown-unknown/release/*.wasm /src/out.wasm"]);
            let out = cmd.output().context("starting the sandbox")?;
            let wasm_path = src.join("out.wasm");
            if !wasm_path.exists() {
                // (The sandbox's own complaints, if it didn't start, are on stderr.)
                let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
                let tail: String = log.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
                bail!("it didn't build ({}):\n{tail}", out.status);
            }
            let wasm = std::fs::read(&wasm_path)?;
            if wasm.len() > MAX_WASM {
                bail!("it built to {} KB (at most {} KB)", wasm.len() / 1000, MAX_WASM / 1000);
            }
            imports_nothing(&wasm).map_err(anyhow::Error::msg)?;
            let _ = m;
            Ok(wasm)
        })();
        let _ = std::fs::remove_dir_all(&dir);
        result
    }
}

/// Where a file goes, made safe: a plain relative path.
pub fn safe_rel(path: &str) -> Option<&Path> {
    (!path.is_empty() && !path.starts_with('/') && !path.split('/').any(|c| c.is_empty() || c == "." || c == ".." || c.contains('\\'))).then(|| Path::new(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_settings_sent_in_are_not_built_with() {
        for p in [".cargo/config.toml", ".cargo/config", "rust-toolchain.toml", "rust-toolchain", "Cargo.lock"] {
            assert!(left_out_of_build(p), "{p}");
        }
        for p in ["Cargo.toml", "src/lib.rs", "pyr-plugin/src/lib.rs", "docs/.cargo-notes.md"] {
            assert!(!left_out_of_build(p), "{p}");
        }
    }

    fn cargo(text: &str) -> Vec<(String, Vec<u8>)> {
        vec![("Cargo.toml".into(), text.as_bytes().to_vec()), ("src/lib.rs".into(), b"".to_vec())]
    }

    #[test]
    fn only_the_kits_crates_and_no_build_scripts() {
        let ok = "[package]\nname = \"pyr-dice\"\nversion = \"1.1.0\"\n[dependencies]\npyr-plugin = { path = \"../kit\" }\nserde = \"1\"\n[package.metadata.pyr]\npermissions = [\"commands\", \"panel\"]\n";
        let m = manifest(&cargo(ok)).unwrap();
        assert_eq!((m.package.as_str(), m.permissions.len(), m.version.as_str()), ("pyr-dice", 2, "1.1.0"));
        for bad in [
            "[package]\nname = \"x\"\n[dependencies]\nreqwest = \"1\"\n",
            "[package]\nname = \"x\"\nbuild = \"b.rs\"\n",
            "[package]\nname = \"x\"\n[build-dependencies]\ncc = \"1\"\n",
            "[package]\nname = \"x\"\n[dependencies]\nserde = { git = \"https://e.vil/serde\" }\n",
            "[package]\nname = \"x\"\n[patch.crates-io]\nserde = { path = \"x\" }\n",
            "[package]\nname = \"x y\"\n",
        ] {
            assert!(manifest(&cargo(bad)).is_err(), "{bad}");
        }
        let mut with_build = cargo(ok);
        with_build.push(("build.rs".into(), b"fn main(){}".to_vec()));
        assert!(manifest(&with_build).is_err());
        let sm = sandbox_manifest(&cargo(ok)).unwrap();
        assert!(sm.contains("path = \"/kit\"") && sm.contains("[workspace]") && sm.contains("panic = \"abort\""), "{sm}");
    }

    #[test]
    fn modules_that_import_are_refused() {
        // (magic, version, an empty type section; then an import section with one import.)
        let none = [b"\0asm".as_slice(), &[1, 0, 0, 0], &[1, 1, 0]].concat();
        assert!(imports_nothing(&none).is_ok());
        let one = [b"\0asm".as_slice(), &[1, 0, 0, 0], &[2, 7, 1, 1, b'e', 1, b'f', 0, 0]].concat();
        assert!(imports_nothing(&one).unwrap_err().contains("imports 1"));
        assert!(imports_nothing(b"MZ......").is_err());
    }

    #[test]
    fn paths_stay_inside() {
        assert!(safe_rel("src/lib.rs").is_some());
        for bad in ["/etc/x", "../x", "a/../../x", "a//b", "", "a\\b"] {
            assert!(safe_rel(bad).is_none(), "{bad}");
        }
    }
}
