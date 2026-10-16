//! Settings → Claude: Ingle's Claude connector (`ingle-mcp`, installed next to Ingle) set up in
//! the Claude desktop app, which starts the MCP servers listed under `mcpServers` in its
//! `claude_desktop_config.json` (`%APPDATA%\Claude` on Windows; a copy of Claude from the
//! Microsoft Store may keep its own under `%LOCALAPPDATA%\Packages\Claude_*`, so any that's
//! there gets it too). Also the line that sets it up in Claude Code.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// The connector's name in Claude's settings (its tools show up under it).
pub const NAME: &str = "ingle";

/// The connector next to this Ingle, if it's there.
pub fn connector() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let p = exe.with_file_name(if cfg!(windows) { "ingle-mcp.exe" } else { "ingle-mcp" });
    p.is_file().then_some(p)
}

/// What the connector is started with: nothing, or this Ingle's settings folder when it isn't
/// the usual one (a test profile, or one `INGLE_HOME` picked, which Claude's apps don't pass on
/// to what they start), so it signs with the same identity.
pub fn args(config_dir: &Path) -> Vec<String> {
    if std::env::var_os("INGLE_HOME").is_none() && config_dir == ingle_engine::config::config_dir(None) {
        Vec::new()
    } else {
        vec!["--config-dir".into(), config_dir.display().to_string()]
    }
}

/// The Claude desktop app's settings files on this PC (whether there yet or not): the usual one
/// first, then a Microsoft Store copy's. Empty where there's no Claude desktop app (Linux).
pub fn claude_configs() -> Vec<PathBuf> {
    #[cfg_attr(not(any(windows, target_os = "macos")), allow(unused_mut))]
    let mut out = Vec::new();
    #[cfg(windows)]
    {
        if let Some(roaming) = std::env::var_os("APPDATA") {
            out.push(PathBuf::from(roaming).join("Claude").join("claude_desktop_config.json"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            if let Ok(rd) = std::fs::read_dir(PathBuf::from(local).join("Packages")) {
                for e in rd.flatten() {
                    if e.file_name().to_string_lossy().starts_with("Claude_") {
                        let dir = e.path().join("LocalCache").join("Roaming").join("Claude");
                        if dir.is_dir() {
                            out.push(dir.join("claude_desktop_config.json"));
                        }
                    }
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            out.push(PathBuf::from(home).join("Library/Application Support/Claude/claude_desktop_config.json"));
        }
    }
    out
}

/// Whether a Claude settings file has the connector.
#[derive(Clone, Debug, PartialEq)]
pub enum Setup {
    /// No entry (or no file yet).
    Missing,
    /// This Ingle's connector, started the way it should be.
    Here,
    /// An entry for another copy (where it points).
    Elsewhere(String),
    /// The file isn't JSON Ingle can read: it's left alone.
    Unreadable(String),
}

fn read(path: &Path) -> Result<Option<Map<String, Value>>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{} can't be read: {e}", path.display())),
    };
    // (Saved by Notepad, it can start with a byte order mark.)
    let text = text.trim_start_matches('\u{feff}');
    if text.trim().is_empty() {
        return Ok(Some(Map::new()));
    }
    match serde_json::from_str::<Value>(text) {
        Ok(Value::Object(m)) => Ok(Some(m)),
        Ok(_) => Err(format!("{} isn't a settings file Ingle understands", path.display())),
        Err(e) => Err(format!("{} has a mistake in it ({e}), so Ingle won't change it", path.display())),
    }
}

/// Whether `config` has the connector at `exe`, started with `args`.
pub fn status(config: &Path, exe: &Path, args: &[String]) -> Setup {
    let m = match read(config) {
        Ok(Some(m)) => m,
        Ok(None) => return Setup::Missing,
        Err(e) => return Setup::Unreadable(e),
    };
    let Some(entry) = m.get("mcpServers").and_then(|s| s.get(NAME)) else { return Setup::Missing };
    let command = entry.get("command").and_then(Value::as_str).unwrap_or("");
    let same_args = entry.get("args").and_then(Value::as_array).map(|a| a.iter().map(|v| v.as_str().unwrap_or("")).eq(args.iter().map(String::as_str))).unwrap_or(args.is_empty());
    let same_exe = if cfg!(windows) { command.eq_ignore_ascii_case(&exe.display().to_string()) } else { Path::new(command) == exe };
    if same_exe && same_args { Setup::Here } else { Setup::Elsewhere(command.to_string()) }
}

/// The connector's entry, as Claude's settings list it.
fn entry(exe: &Path, args: &[String]) -> Value {
    json!({ "command": exe.display().to_string(), "args": args })
}

/// Writes `m` over `config`: the old file kept next to it first (`….ingle-backup.json`), and
/// the new one swapped in whole.
fn write(config: &Path, m: Map<String, Value>) -> Result<(), String> {
    if let Some(dir) = config.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("couldn't make {}: {e}", dir.display()))?;
    }
    if config.exists() {
        std::fs::copy(config, config.with_extension("ingle-backup.json")).map_err(|e| format!("couldn't keep a copy of {}: {e}", config.display()))?;
    }
    let text = serde_json::to_string_pretty(&Value::Object(m)).map_err(|e| e.to_string())? + "\n";
    let tmp = config.with_extension("json.ingle-new");
    std::fs::write(&tmp, text).map_err(|e| format!("couldn't write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, config).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("couldn't replace {}: {e}", config.display())
    })
}

/// Adds the connector to `config` (or points its entry at this Ingle), keeping everything else.
pub fn add(config: &Path, exe: &Path, args: &[String]) -> Result<(), String> {
    let mut m = read(config)?.unwrap_or_default();
    let servers = m.entry("mcpServers").or_insert_with(|| Value::Object(Map::new()));
    let Some(servers) = servers.as_object_mut() else {
        return Err(format!("{}'s mcpServers isn't a list Ingle understands, so it's left alone", config.display()));
    };
    servers.insert(NAME.into(), entry(exe, args));
    write(config, m)
}

/// Takes the connector out of `config`, keeping everything else.
pub fn remove(config: &Path) -> Result<(), String> {
    let Some(mut m) = read(config)? else { return Ok(()) };
    let gone = m.get_mut("mcpServers").and_then(Value::as_object_mut).is_some_and(|s| s.remove(NAME).is_some());
    if gone { write(config, m) } else { Ok(()) }
}

/// The setup to paste into another app's MCP settings.
pub fn snippet(exe: &Path, args: &[String]) -> String {
    serde_json::to_string_pretty(&json!({ "mcpServers": { NAME: entry(exe, args) } })).unwrap_or_default()
}

/// The command that adds it to Claude Code.
pub fn claude_code_line(exe: &Path, args: &[String]) -> String {
    let quote = |s: &str| if s.contains(' ') || s.is_empty() { format!("\"{s}\"") } else { s.to_string() };
    let mut line = format!("claude mcp add --scope user {NAME} -- {}", quote(&exe.display().to_string()));
    for a in args {
        line.push(' ');
        line.push_str(&quote(a));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claudes_settings_get_the_connector_and_keep_everything_else() {
        let dir = std::env::temp_dir().join(format!("ingle-connector-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = dir.join("Claude").join("claude_desktop_config.json");
        let exe = PathBuf::from(if cfg!(windows) { r"C:\Program Files\Ingle\ingle-mcp.exe" } else { "/opt/ingle/ingle-mcp" });
        // No file yet: made.
        assert_eq!(status(&config, &exe, &[]), Setup::Missing);
        add(&config, &exe, &[]).unwrap();
        assert_eq!(status(&config, &exe, &[]), Setup::Here);
        assert!(!config.with_extension("ingle-backup.json").exists(), "nothing to back up the first time");
        // Someone else's servers and Claude's own settings stay, and the old file is kept.
        std::fs::write(&config, "\u{feff}{ \"mcpServers\": { \"files\": { \"command\": \"npx\", \"args\": [\"-y\", \"files\"] } }, \"preferences\": { \"x\": 1 } }").unwrap();
        assert_eq!(status(&config, &exe, &[]), Setup::Missing);
        let args = vec!["--config-dir".to_string(), "/tmp/ingle test".to_string()];
        add(&config, &exe, &args).unwrap();
        assert_eq!(status(&config, &exe, &args), Setup::Here);
        assert_eq!(status(&config, &exe, &[]), Setup::Elsewhere(exe.display().to_string()), "started differently");
        let m: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(m["mcpServers"]["files"]["command"], "npx");
        assert_eq!(m["preferences"]["x"], 1);
        assert!(std::fs::read_to_string(config.with_extension("ingle-backup.json")).unwrap().contains("\"files\""));
        // Taken out again, leaving the rest.
        remove(&config).unwrap();
        let m: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert!(m["mcpServers"].get(NAME).is_none() && m["mcpServers"]["files"].is_object());
        // A file with a mistake in it is never overwritten.
        std::fs::write(&config, "{ \"mcpServers\": { ").unwrap();
        assert!(matches!(status(&config, &exe, &[]), Setup::Unreadable(_)));
        assert!(add(&config, &exe, &[]).is_err());
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "{ \"mcpServers\": { ");
        // For other apps.
        assert!(snippet(&exe, &[]).contains("\"mcpServers\""));
        assert!(claude_code_line(&exe, &args).ends_with("--config-dir \"/tmp/ingle test\""));
        let _ = std::fs::remove_dir_all(dir);
    }
}
