//! Settings → Claude: Pyr's Claude connector (`pyr-mcp`, installed next to Pyr) set up in
//! the Claude desktop app, which starts the MCP servers listed under `mcpServers` in its
//! `claude_desktop_config.json` (`%APPDATA%\Claude` on Windows; a copy of Claude from the
//! Microsoft Store may keep its own under `%LOCALAPPDATA%\Packages\Claude_*`, so any that's
//! there gets it too). Also the line that sets it up in Claude Code.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// The connector's name in Claude's settings (its tools show up under it).
pub const NAME: &str = "pyr";

/// The connector next to this Pyr, if it's there.
pub fn connector() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let p = exe.with_file_name(if cfg!(windows) { "pyr-mcp.exe" } else { "pyr-mcp" });
    p.is_file().then_some(p)
}

/// What the connector is started with: nothing, or this Pyr's settings folder when it isn't
/// the usual one (a test profile, or one `PYR_HOME` picked, which Claude's apps don't pass on
/// to what they start), so it signs with the same identity.
pub fn args(config_dir: &Path) -> Vec<String> {
    if !pyr_proto::env_set("HOME") && config_dir == pyr_engine::config::config_dir(None) {
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
    /// This Pyr's connector, started the way it should be.
    Here,
    /// An entry for another copy (where it points).
    Elsewhere(String),
    /// The file isn't JSON Pyr can read: it's left alone.
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
        Ok(_) => Err(format!("{} isn't a settings file Pyr understands", path.display())),
        Err(e) => Err(format!("{} has a mistake in it ({e}), so Pyr won't change it", path.display())),
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

/// Writes `m` over `config`: the old file kept next to it first (`….pyr-backup.json`), and
/// the new one swapped in whole.
fn write(config: &Path, m: Map<String, Value>) -> Result<(), String> {
    if let Some(dir) = config.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("couldn't make {}: {e}", dir.display()))?;
    }
    if config.exists() {
        std::fs::copy(config, config.with_extension("pyr-backup.json")).map_err(|e| format!("couldn't keep a copy of {}: {e}", config.display()))?;
    }
    let text = serde_json::to_string_pretty(&Value::Object(m)).map_err(|e| e.to_string())? + "\n";
    let tmp = config.with_extension("json.pyr-new");
    std::fs::write(&tmp, text).map_err(|e| format!("couldn't write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, config).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("couldn't replace {}: {e}", config.display())
    })
}

/// Adds the connector to `config` (or points its entry at this Pyr), keeping everything else.
pub fn add(config: &Path, exe: &Path, args: &[String]) -> Result<(), String> {
    let mut m = read(config)?.unwrap_or_default();
    let servers = m.entry("mcpServers").or_insert_with(|| Value::Object(Map::new()));
    let Some(servers) = servers.as_object_mut() else {
        return Err(format!("{}'s mcpServers isn't a list Pyr understands, so it's left alone", config.display()));
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

/// The connector's name in Claude's settings before Pyr was called Pyr.
const OLD_NAME: &str = "ingle";

/// Claude's settings still start Ingle's connector (`ingle` → `ingle-mcp`), which went with
/// Ingle: it becomes this Pyr's (`pyr` → `pyr-mcp`, the old file kept next to it). Anything
/// else under that name is left alone. Whether it changed anything.
pub fn migrate_from_ingle(config: &Path, exe: &Path, args: &[String]) -> Result<bool, String> {
    let Some(mut m) = read(config)? else { return Ok(false) };
    let Some(servers) = m.get_mut("mcpServers").and_then(Value::as_object_mut) else { return Ok(false) };
    if !servers.get(OLD_NAME).is_some_and(starts_ingles) {
        return Ok(false);
    }
    servers.remove(OLD_NAME);
    servers.entry(NAME).or_insert_with(|| entry(exe, args));
    write(config, m).map(|()| true)
}

/// An `mcpServers` entry that starts Ingle's connector (`ingle-mcp`).
fn starts_ingles(e: &Value) -> bool {
    e.get("command").and_then(Value::as_str).is_some_and(|c| {
        let file = c.rsplit(['/', '\\']).next().unwrap_or("");
        file.eq_ignore_ascii_case("ingle-mcp") || file.eq_ignore_ascii_case("ingle-mcp.exe")
    })
}

/// Claude Code's own settings file (`~/.claude.json`), where `claude mcp add --scope user`
/// puts what it starts.
pub fn claude_code_config() -> Option<PathBuf> {
    let home = if cfg!(windows) { std::env::var_os("USERPROFILE") } else { std::env::var_os("HOME") };
    Some(PathBuf::from(home?).join(".claude.json"))
}

/// Whether Claude Code still starts Ingle's connector (set up before Pyr was called Pyr; that
/// connector went with Ingle). Only read, never changed: Claude Code writes that file all the
/// time, so it's switched over with Claude Code's own commands ([`claude_code_switch`]).
pub fn claude_code_has_ingles(config: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(config) else { return false };
    // (Most of the time it isn't there at all: no need to read the whole file as JSON.)
    if !text.contains("ingle-mcp") {
        return false;
    }
    serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("mcpServers").and_then(|s| s.get(OLD_NAME)).map(starts_ingles))
        .unwrap_or(false)
}

/// The commands that take Ingle's connector out of Claude Code and put this Pyr's in.
pub fn claude_code_switch(exe: &Path, args: &[String]) -> [String; 2] {
    [format!("claude mcp remove --scope user {OLD_NAME}"), claude_code_line(exe, args)]
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

    /// Claude's settings that started Ingle's connector start Pyr's instead; an entry called
    /// "ingle" that isn't Ingle's connector is left alone.
    #[test]
    fn ingles_connector_becomes_pyrs() {
        let dir = std::env::temp_dir().join(format!("pyr-connector-mig-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("claude_desktop_config.json");
        let exe = PathBuf::from(if cfg!(windows) { r"C:\Users\a\AppData\Local\Programs\Pyr\pyr-mcp.exe" } else { "/usr/bin/pyr-mcp" });
        std::fs::write(&config, r#"{"mcpServers":{"ingle":{"command":"C:\\Users\\a\\AppData\\Local\\Programs\\Ingle\\ingle-mcp.exe","args":[]},"other":{"command":"x"}},"theme":"dark"}"#).unwrap();
        assert_eq!(migrate_from_ingle(&config, &exe, &[]), Ok(true));
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert!(v["mcpServers"].get("ingle").is_none());
        assert_eq!(v["mcpServers"]["pyr"]["command"], exe.display().to_string());
        assert_eq!((v["mcpServers"]["other"]["command"].as_str(), v["theme"].as_str()), (Some("x"), Some("dark")));
        assert_eq!(migrate_from_ingle(&config, &exe, &[]), Ok(false), "once");
        std::fs::write(&config, r#"{"mcpServers":{"ingle":{"command":"/home/a/my-ingle-tool"}}}"#).unwrap();
        assert_eq!(migrate_from_ingle(&config, &exe, &[]), Ok(false), "not Ingle's connector");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Claude Code's settings are only read: when they still start Ingle's connector, Pyr
    /// says so and gives the two commands that switch it, and the file stays as it was.
    #[test]
    fn claude_code_still_on_ingles_connector_is_told() {
        let dir = std::env::temp_dir().join(format!("pyr-connector-code-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join(".claude.json");
        assert!(!claude_code_has_ingles(&config), "no file");
        let old = r#"{"numStartups":3,"mcpServers":{"ingle":{"type":"stdio","command":"C:\\Users\\a\\AppData\\Local\\Programs\\Ingle\\ingle-mcp.exe","args":[],"env":{}}},"projects":{}}"#;
        std::fs::write(&config, old).unwrap();
        assert!(claude_code_has_ingles(&config));
        assert_eq!(std::fs::read_to_string(&config).unwrap(), old, "left as it was");
        // Pyr's connector, a project-only entry, someone else's "ingle", or a file that isn't JSON: nothing to say.
        for other in [
            r#"{"mcpServers":{"pyr":{"command":"C:\\Pyr\\pyr-mcp.exe"}}}"#,
            r#"{"projects":{"C:/x":{"mcpServers":{"ingle":{"command":"ingle-mcp"}}}}}"#,
            r#"{"mcpServers":{"ingle":{"command":"/home/a/my-ingle-tool"}},"note":"ingle-mcp"}"#,
            "{ not json ingle-mcp",
        ] {
            std::fs::write(&config, other).unwrap();
            assert!(!claude_code_has_ingles(&config), "{other}");
        }
        let exe = PathBuf::from("/usr/bin/pyr-mcp");
        assert_eq!(claude_code_switch(&exe, &[]), ["claude mcp remove --scope user ingle".to_string(), "claude mcp add --scope user pyr -- /usr/bin/pyr-mcp".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn claudes_settings_get_the_connector_and_keep_everything_else() {
        let dir = std::env::temp_dir().join(format!("pyr-connector-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = dir.join("Claude").join("claude_desktop_config.json");
        let exe = PathBuf::from(if cfg!(windows) { r"C:\Program Files\Pyr\pyr-mcp.exe" } else { "/opt/pyr/pyr-mcp" });
        // No file yet: made.
        assert_eq!(status(&config, &exe, &[]), Setup::Missing);
        add(&config, &exe, &[]).unwrap();
        assert_eq!(status(&config, &exe, &[]), Setup::Here);
        assert!(!config.with_extension("pyr-backup.json").exists(), "nothing to back up the first time");
        // Someone else's servers and Claude's own settings stay, and the old file is kept.
        std::fs::write(&config, "\u{feff}{ \"mcpServers\": { \"files\": { \"command\": \"npx\", \"args\": [\"-y\", \"files\"] } }, \"preferences\": { \"x\": 1 } }").unwrap();
        assert_eq!(status(&config, &exe, &[]), Setup::Missing);
        let args = vec!["--config-dir".to_string(), "/tmp/pyr test".to_string()];
        add(&config, &exe, &args).unwrap();
        assert_eq!(status(&config, &exe, &args), Setup::Here);
        assert_eq!(status(&config, &exe, &[]), Setup::Elsewhere(exe.display().to_string()), "started differently");
        let m: Value = serde_json::from_str(&std::fs::read_to_string(&config).unwrap()).unwrap();
        assert_eq!(m["mcpServers"]["files"]["command"], "npx");
        assert_eq!(m["preferences"]["x"], 1);
        assert!(std::fs::read_to_string(config.with_extension("pyr-backup.json")).unwrap().contains("\"files\""));
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
        assert!(claude_code_line(&exe, &args).ends_with("--config-dir \"/tmp/pyr test\""));
        let _ = std::fs::remove_dir_all(dir);
    }
}
