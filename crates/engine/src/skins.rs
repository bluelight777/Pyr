//! Skins ([`pyr_proto::skin`]) on this PC: a skin's folder gathered and checked (fonts parsed,
//! not just told by their first bytes), sounds made from audio files, new skins started, and a
//! skin's submission for the Pyr Library. Used by the app and by the Claude connector.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use pyr_proto::control::{PluginSubmission, SKIN_KIND, SourceFile};
use pyr_proto::skin::{self, Report, Use};

/// How to make a skin (skins/README.md): for the connector's guide and new skins' folders.
pub const GUIDE: &str = include_str!("../../../skins/README.md");

/// Where the Claude connector puts a skin for Pyr to try (in Pyr's settings folder): its
/// files, then [`PREVIEW_STAMP`] saying which (written last; removed to stop).
pub const PREVIEW_DIR: &str = "skin-preview";
pub const PREVIEW_STAMP: &str = "skin-preview.json";

/// Folders never part of a skin.
const SKIP_DIRS: &[&str] = &[".git", ".github", "target", "node_modules", ".vscode", ".idea"];

/// Every file in a skin's folder (path in the skin with `/`, bytes), in a stable order. Hidden
/// files (a dot first) and version control are left out; links aren't followed.
pub fn gather(folder: &Path) -> Result<Vec<(String, Vec<u8>)>> {
    if !folder.join(skin::MANIFEST).is_file() {
        bail!("{} has no skin.json: pick the skin's own folder", folder.display());
    }
    let mut out = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).with_context(|| format!("couldn't read {}", dir.display()))? {
            let e = e?;
            let name = e.file_name().to_string_lossy().to_string();
            let meta = std::fs::symlink_metadata(e.path())?;
            if name.starts_with('.') || meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    stack.push(e.path());
                }
                continue;
            }
            if meta.len() as usize > skin::MAX_TOTAL {
                bail!("{name} is {} MB: far more than a skin can be", meta.len() >> 20);
            }
            let rel: Vec<String> = e.path().strip_prefix(folder)?.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
            let bytes = std::fs::read(e.path())?;
            // The guide `start` puts in a new skin's folder isn't part of the skin.
            if rel.len() == 1 && rel[0] == "GUIDE.md" && bytes == GUIDE.as_bytes() {
                continue;
            }
            out.push((rel.join("/"), bytes));
        }
        if out.len() > skin::MAX_FILES * 4 {
            bail!("{} has far more files than a skin can have ({} at most)", folder.display(), skin::MAX_FILES);
        }
    }
    out.sort();
    Ok(out)
}

/// Whether a font file parses (the text renderer is only handed fonts that do).
pub fn font_ok(bytes: &[u8]) -> bool {
    ab_glyph::FontRef::try_from_slice(bytes).is_ok()
}

/// The checks every skin gets (see [`skin::check`]), and its fonts parsed.
pub fn check(files: &[(String, Vec<u8>)]) -> Report {
    let mut r = skin::check(files);
    if r.ok() {
        for (path, what) in r.manifest.uses() {
            if what == Use::Font && !files.iter().any(|(p, b)| p == path && font_ok(b)) {
                r.problems.push(format!("{path}: the font can't be read (is it a whole .ttf or .otf file?)."));
            }
        }
    }
    r
}

/// The skin in a folder someone picked: the folder itself, or the one skin a little way inside
/// it (a skin's folder from a .zip sits one level down). None if there's no skin.json there.
pub fn find_folder(folder: &Path) -> Option<PathBuf> {
    if folder.join(skin::MANIFEST).is_file() {
        return Some(folder.to_path_buf());
    }
    let mut found = Vec::new();
    let mut level = vec![folder.to_path_buf()];
    for _ in 0..2 {
        let mut next = Vec::new();
        for dir in &level {
            let Ok(rd) = std::fs::read_dir(dir) else { continue };
            for e in rd.flatten().take(200) {
                let name = e.file_name().to_string_lossy().to_string();
                let is_dir = std::fs::symlink_metadata(e.path()).is_ok_and(|m| m.is_dir());
                if !is_dir || name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                if e.path().join(skin::MANIFEST).is_file() {
                    found.push(e.path());
                } else {
                    next.push(e.path());
                }
            }
        }
        if !found.is_empty() {
            break;
        }
        level = next;
    }
    (found.len() == 1).then(|| found.remove(0))
}

/// A folder's skin, checked.
pub fn check_folder(folder: &Path) -> Result<(Vec<(String, Vec<u8>)>, Report)> {
    let files = gather(folder)?;
    let report = check(&files);
    Ok((files, report))
}

/// A Pyr sound file (Opus, 10 seconds at most) from an audio file (MP3, WAV, OGG, FLAC, M4A).
pub fn make_sound(input: &Path) -> Result<Vec<u8>> {
    let bytes = std::fs::read(input).with_context(|| format!("couldn't read {}", input.display()))?;
    let ext = input.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase());
    let imported = crate::soundboard::import(bytes, ext.as_deref())?;
    let max = pyr_proto::sound::MAX_MS as usize * 48 * 2;
    let pcm = &imported.pcm[..imported.pcm.len().min(max)];
    if pcm.iter().all(|v| v.abs() < 1e-4) {
        bail!("{} is silent", input.display());
    }
    crate::soundboard::encode(pcm)
}

/// A name as a folder name: lowercase letters, digits and dashes.
pub fn slug(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Starts a skin in `folder/<name>`: a skin.json with Campfire's colors written out to change,
/// and the guide. Returns the skin's folder.
pub fn start(folder: &Path, name: &str, author: &str) -> Result<PathBuf> {
    let slug = slug(name);
    if slug.is_empty() {
        bail!("give it a name with some letters in it");
    }
    let dir = folder.join(&slug);
    if dir.exists() {
        bail!("there's already a folder called {slug} there");
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(skin::MANIFEST), skin::starter(name.trim(), author.trim()))?;
    std::fs::write(dir.join("GUIDE.md"), GUIDE)?;
    Ok(dir)
}

/// A skin's submission for the Pyr Library (what skin.json says about it, and its files);
/// `update_of`: one of mine it's a new version of.
pub fn submission(files: &[(String, Vec<u8>)], report: &Report, contact: &str, update_of: i64) -> Result<PluginSubmission> {
    if !report.ok() {
        return Err(anyhow!("the skin isn't right yet: {}", report.problems.join(" ")));
    }
    let m = &report.manifest;
    Ok(PluginSubmission {
        name: m.name.trim().to_string(),
        kind: SKIN_KIND.into(),
        version: m.version.trim().to_string(),
        summary: m.summary.trim().to_string(),
        author: m.author.trim().to_string(),
        contact: contact.trim().to_string(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        files: files.iter().map(|(p, b)| SourceFile { path: p.clone(), data: pyr_proto::b64::encode(b) }).collect(),
        update_of,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pyr-skins-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_new_skin_checks_out_and_becomes_a_submission() {
        let base = temp_dir("new");
        let dir = start(&base, "Blood Moon!", "Blue").unwrap();
        assert!(dir.ends_with("blood-moon"));
        assert!(start(&base, "Blood Moon!", "Blue").is_err(), "not over an existing folder");
        // Hidden files and version control don't come along.
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        std::fs::write(dir.join(".git/HEAD"), b"ref").unwrap();
        std::fs::write(dir.join(".DS_Store"), b"x").unwrap();
        let (files, report) = check_folder(&dir).unwrap();
        // (Nor does the guide put next to it, unless it was changed.)
        assert_eq!(files.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), vec!["skin.json"]);
        assert!(report.ok(), "{:?}", report.problems);
        assert_eq!(report.manifest.name, "Blood Moon!");
        let sub = submission(&files, &report, "blue@example.com", 3).unwrap();
        assert_eq!((sub.kind.as_str(), sub.update_of, sub.files.len()), (SKIN_KIND, 3, 1));
        std::fs::write(dir.join("GUIDE.md"), "My own notes").unwrap();
        assert_eq!(gather(&dir).unwrap().len(), 2);
        // A font that only looks like one is caught here (the app would otherwise crash on it).
        std::fs::create_dir_all(dir.join("fonts")).unwrap();
        std::fs::write(dir.join("fonts/Body.ttf"), [vec![0, 1, 0, 0], vec![0; 200]].concat()).unwrap();
        std::fs::write(dir.join("fonts/OFL.txt"), b"SIL Open Font License").unwrap();
        let json = std::fs::read_to_string(dir.join("skin.json")).unwrap();
        std::fs::write(dir.join("skin.json"), json.replacen("\"colors\"", "\"fonts\": { \"body\": \"fonts/Body.ttf\" },\n  \"colors\"", 1)).unwrap();
        let (files, report) = check_folder(&dir).unwrap();
        assert!(report.problems.iter().any(|p| p.contains("can't be read")), "{:?}", report.problems);
        assert!(submission(&files, &report, "", 0).is_err());
        assert!(check_folder(&base).is_err(), "no skin.json");
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn the_skin_a_little_way_inside_a_folder_is_found() {
        let top = std::env::temp_dir().join(format!("pyr-find-skin-{}", pyr_proto::now_ms()));
        let inner = top.join("download").join("my-skin");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::create_dir_all(top.join("other")).unwrap();
        assert_eq!(find_folder(&top), None);
        std::fs::write(inner.join(skin::MANIFEST), "{}").unwrap();
        assert_eq!(find_folder(&top), Some(inner.clone()));
        assert_eq!(find_folder(&inner), Some(inner.clone()));
        let second = top.join("other").join("skin-two");
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(second.join(skin::MANIFEST), "{}").unwrap();
        assert_eq!(find_folder(&top), None, "two skins: which one isn't guessed");
        let _ = std::fs::remove_dir_all(&top);
    }

    #[test]
    fn sounds_are_made_from_audio_files() {
        let base = temp_dir("sound");
        // A second of a 440 Hz tone as a WAV file.
        let rate = 44_100u32;
        let samples: Vec<i16> = (0..rate).map(|i| ((i as f32 * 440.0 * std::f32::consts::TAU / rate as f32).sin() * 8000.0) as i16).collect();
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + samples.len() as u32 * 2).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&rate.to_le_bytes());
        wav.extend_from_slice(&(rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(samples.len() as u32 * 2).to_le_bytes());
        for s in &samples {
            wav.extend_from_slice(&s.to_le_bytes());
        }
        std::fs::write(base.join("tone.wav"), &wav).unwrap();
        let ings = make_sound(&base.join("tone.wav")).unwrap();
        let (fmt, _) = pyr_proto::sound::packets(&ings).expect("a Pyr sound file");
        assert!((900..=1100).contains(&fmt.length_ms()), "{} ms", fmt.length_ms());
        std::fs::write(base.join("silence.wav"), [&wav[..44], &vec![0u8; samples.len() * 2][..]].concat()).unwrap();
        assert!(make_sound(&base.join("silence.wav")).unwrap_err().to_string().contains("silent"));
        let _ = std::fs::remove_dir_all(base);
    }
}
