//! The connector's tools: guides, starting a skin or an add-on, checking them the way the Pyr
//! Library will, showing a skin on Pyr, making sounds, sending them in (signed with Pyr's
//! identity, so only this Pyr can send new versions of them), and where they stand; and
//! Claude's own tools in the servers it's been added to as a bot (see `chat`).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use pyr_engine::{plugin_source, plugins, skins};
use pyr_proto::control::{PluginSubmission, SKIN_KIND, SubmissionStatus};
use pyr_proto::crypto::Identity;
use serde_json::{Value, json};

/// What the tools work with: Pyr's settings folder, a way to reach the library server, and
/// Claude's own copy of the engine (a bot, started when it's first needed).
pub struct Ctx {
    pub config_dir: PathBuf,
    rt: tokio::runtime::Runtime,
    endpoints: std::sync::Mutex<pyr_engine::net::Endpoints>,
    /// The library server: (host, port, its key).
    server: Option<(String, u16, [u8; 32])>,
    bot: std::sync::Mutex<Option<std::sync::Arc<pyr_engine::Engine>>>,
}

impl Ctx {
    pub fn new(config_dir: PathBuf) -> Result<Ctx> {
        Ctx::with_server(config_dir, plugins::server())
    }

    pub fn with_server(config_dir: PathBuf, server: Option<(String, u16, [u8; 32])>) -> Result<Ctx> {
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("pyr-mcp-net").enable_all().build()?;
        Ok(Ctx { config_dir, rt, endpoints: Default::default(), server, bot: Default::default() })
    }

    /// Where Claude keeps its own identity and settings as a bot ([`pyr_engine::claude_dir`]).
    pub fn bot_dir(&self) -> PathBuf {
        pyr_engine::claude_dir(&self.config_dir)
    }

    /// Whether Claude has been added to a server (or is waiting to be), so its engine should
    /// run and keep it online while the connector does.
    pub fn bot_wanted(&self) -> bool {
        let dir = self.bot_dir();
        let waiting = std::fs::read_dir(dir.join("join")).is_ok_and(|d| d.flatten().any(|e| e.path().extension().is_some_and(|x| x == "link")));
        waiting || !pyr_engine::config::load_config(&dir).servers.is_empty()
    }

    /// Claude's own copy of the engine, as the bot Claude (with an identity of its own, never
    /// the person's), started the first time it's needed. It writes Claude's member id (a
    /// public id) next to its identity, for Pyr's Server Settings → Bots.
    pub fn bot(&self) -> Result<std::sync::Arc<pyr_engine::Engine>> {
        let mut bot = self.bot.lock().map_err(|_| anyhow!("Claude's engine stopped"))?;
        if let Some(e) = bot.as_ref() {
            return Ok(e.clone());
        }
        let dir = self.bot_dir();
        std::fs::create_dir_all(&dir).with_context(|| format!("couldn't make {}", dir.display()))?;
        let mut cfg = pyr_engine::config::load_config(&dir);
        if cfg.display_name.trim().is_empty() {
            cfg.display_name = "Claude".into();
            // Sounds off: a bot has no speakers to play them on.
            cfg.settings.sounds = false;
            pyr_engine::config::save_config(&dir, &cfg)?;
        }
        let engine = pyr_engine::Engine::start(pyr_engine::EngineOptions { config_dir: Some(dir.clone()), push_to_talk: false, bot: true, ..Default::default() })?;
        let uid = engine.state().my_uid.clone();
        std::fs::write(dir.join(pyr_engine::CLAUDE_MEMBER_ID), &uid).with_context(|| format!("couldn't write Claude's member id in {}", dir.display()))?;
        let engine = std::sync::Arc::new(engine);
        *bot = Some(engine.clone());
        Ok(engine)
    }

    /// Pyr's identity on this PC (never made here: Pyr makes it the first time it opens).
    fn identity(&self) -> Result<Identity> {
        let path = self.config_dir.join("identity.key");
        let text = std::fs::read_to_string(&path).map_err(|_| anyhow!("Pyr hasn't been opened on this PC yet (there's no identity in {}): open Pyr once, then try again.", self.config_dir.display()))?;
        let seed = pyr_proto::b64::decode_array::<32>(text.trim()).ok_or_else(|| anyhow!("{} can't be read", path.display()))?;
        Ok(Identity::from_seed(seed))
    }

    /// The name the person uses in Pyr.
    fn my_name(&self) -> String {
        pyr_engine::config::load_config(&self.config_dir).display_name.trim().to_string()
    }

    fn library(&self) -> Result<(String, u16, [u8; 32])> {
        self.server.clone().ok_or_else(|| anyhow!("This copy of the connector has nowhere to send plugins and skins."))
    }

    /// Sends a plugin or skin in, signed. Its number there.
    fn send(&self, sub: PluginSubmission) -> Result<i64> {
        let me = self.identity()?;
        let server = self.library()?;
        self.rt.block_on(plugins::send_signed(&self.endpoints, server, &me, sub))
    }
}

/// The tools, always in this order.
pub fn list() -> Value {
    let folder = |what: &str| json!({ "type": "string", "description": what });
    let read_only = json!({ "readOnlyHint": true, "openWorldHint": false });
    let makes = json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false });
    let sends = json!({ "readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": true });
    let none = json!({ "type": "object", "additionalProperties": false });
    let mut tools = json!([
        {
            "name": "skin_guide",
            "title": "How Pyr skins are made",
            "description": "How a Pyr skin is made: skin.json's fields, the 25 colors and what each one colors (with Pyr's own, Campfire's, for reference), the picture behind the chat, fonts, sounds, the limits and the checks. Read it before making or changing a skin.",
            "inputSchema": none,
            "annotations": read_only,
        },
        {
            "name": "new_skin",
            "title": "Start a skin",
            "description": "Starts a new skin: makes a folder named after it inside `folder`, with a skin.json that has Campfire's colors written out to change, and the guide. Returns the new folder's path.",
            "inputSchema": { "type": "object", "properties": {
                "folder": folder("The whole path of the folder to make it in (like C:\\Users\\Name\\Documents). Ask the person if you don't know where they keep things."),
                "name": { "type": "string", "description": "The skin's name, like Blood Moon." },
                "author": { "type": "string", "description": "Who made it, shown under it in the Library. The person's Pyr name if left out." },
            }, "required": ["folder", "name"] },
            "annotations": makes,
        },
        {
            "name": "check_skin",
            "title": "Check a skin",
            "description": "Checks a skin's folder the way Pyr and the Pyr Library will: skin.json, every color, whether text stays readable on its colors, the picture, fonts and sounds, and the limits. Says what to fix, if anything.",
            "inputSchema": { "type": "object", "properties": { "folder": folder("The whole path of the skin's folder (the one with skin.json).") }, "required": ["folder"] },
            "annotations": read_only,
        },
        {
            "name": "preview_skin",
            "title": "Show a skin on Pyr",
            "description": "Puts the skin in a folder on the person's Pyr (if Pyr is open, within a second) without installing it, so they see how it looks; Pyr shows a banner while it's on. Use it again after each change. It stays on until stop_preview, or until they stop it in Pyr.",
            "inputSchema": { "type": "object", "properties": { "folder": folder("The whole path of the skin's folder (the one with skin.json).") }, "required": ["folder"] },
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false },
        },
        {
            "name": "stop_preview",
            "title": "Stop showing the skin",
            "description": "Takes the skin shown with preview_skin off Pyr: it goes back to the skin the person wears.",
            "inputSchema": none,
            "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false },
        },
        {
            "name": "make_sound",
            "title": "Make a Pyr sound",
            "description": "Makes a Pyr sound file (.ings: Opus, 10 seconds at most) from an MP3, WAV, OGG, FLAC or M4A file, for a skin's sounds.",
            "inputSchema": { "type": "object", "properties": {
                "input": folder("The whole path of the audio file."),
                "output": folder("The whole path of the .ings file to make (usually in the skin's sounds folder). Next to the input if left out."),
            }, "required": ["input"] },
            "annotations": makes,
        },
        {
            "name": "send_skin",
            "title": "Send a skin to the Pyr Library",
            "description": "Sends the skin in a folder to the Pyr Library for review, signed with the person's Pyr identity. Only when the person asks you to send it. It's checked first (check_skin). To send a new version of one they sent before, give its number as update_of (my_submissions lists them): only this Pyr can do that. Returns its number.",
            "inputSchema": { "type": "object", "properties": {
                "folder": folder("The whole path of the skin's folder (the one with skin.json)."),
                "contact": { "type": "string", "description": "How the reviewer can reach them with questions (optional)." },
                "update_of": { "type": "integer", "description": "The number of a skin they sent before that this is a new version of (optional)." },
            }, "required": ["folder"] },
            "annotations": sends,
        },
        {
            "name": "plugin_guide",
            "title": "How Pyr plugins are made",
            "description": "How Pyr plugins are made: add-ons (Rust built to WebAssembly, run in Pyr's sandbox, with /commands and a panel) and programs, the plugin kit, building and trying one, and sending it in. Read it before making or changing a plugin.",
            "inputSchema": none,
            "annotations": read_only,
        },
        {
            "name": "new_addon",
            "title": "Start an add-on",
            "description": "Starts a new Pyr add-on: makes a folder named after it inside `folder` with the plugin kit, a /hello command and a panel to change, and a CLAUDE.md about add-ons. Returns the new folder's path.",
            "inputSchema": { "type": "object", "properties": {
                "folder": folder("The whole path of the folder to make it in."),
                "name": { "type": "string", "description": "The add-on's name, like Coin Flip." },
            }, "required": ["folder", "name"] },
            "annotations": makes,
        },
        {
            "name": "check_plugin",
            "title": "Check a plugin",
            "description": "Checks a plugin's folder the way the Pyr Library will before it's sent: Rust only (a Cargo.toml and .rs files; no built files, C code or scripts), 600 KB at most. Says what would be sent and what to fix.",
            "inputSchema": { "type": "object", "properties": { "folder": folder("The whole path of the plugin's folder (the one with Cargo.toml).") }, "required": ["folder"] },
            "annotations": read_only,
        },
        {
            "name": "send_plugin",
            "title": "Send a plugin to the Pyr Library",
            "description": "Sends the plugin in a folder (its Rust source, never a built file) to the Pyr Library for review, signed with the person's Pyr identity: Firelit Games reads and builds it before anyone can install it. Only when the person asks you to send it. To send a new version of one they sent before, give its number as update_of (my_submissions lists them): only this Pyr can do that. Returns its number.",
            "inputSchema": { "type": "object", "properties": {
                "folder": folder("The whole path of the plugin's folder (the one with Cargo.toml)."),
                "author": { "type": "string", "description": "Who made it, shown under it in the Library. The person's Pyr name if left out." },
                "contact": { "type": "string", "description": "How the reviewer can reach them with questions (optional)." },
                "kind": { "type": "string", "enum": ["addon", "program"], "description": "What it is, if its Cargo.toml doesn't say." },
                "summary": { "type": "string", "description": "One line on what it does, if its Cargo.toml doesn't say." },
                "update_of": { "type": "integer", "description": "The number of a plugin they sent before that this is a new version of (optional)." },
            }, "required": ["folder"] },
            "annotations": sends,
        },
        {
            "name": "my_submissions",
            "title": "What was sent to the Pyr Library",
            "description": "Lists the plugins and skins sent from this Pyr (by its identity) to the Pyr Library: their numbers, versions, whether they were approved, and the reviewer's note if there is one.",
            "inputSchema": none,
            "annotations": { "readOnlyHint": true, "openWorldHint": true },
        },
    ]);
    // And Claude's own, in the servers it's been added to.
    if let (Some(all), Some(chat)) = (tools.as_array_mut(), crate::chat::list().as_array()) {
        all.extend(chat.iter().cloned());
    }
    tools
}

/// Runs a tool (`None`: there's no such tool).
pub fn call(ctx: &Ctx, name: &str, args: &Value) -> Option<Result<String>> {
    if crate::chat::NAMES.contains(&name) {
        return Some(ctx.bot().and_then(|engine| crate::chat::call(&engine, name, args).unwrap_or_else(|| Err(anyhow!("Unknown tool: {name}")))));
    }
    Some(match name {
        "skin_guide" => Ok(skins::GUIDE.to_string()),
        "new_skin" => new_skin(ctx, args),
        "check_skin" => check_skin(args),
        "preview_skin" => preview_skin(ctx, args),
        "stop_preview" => stop_preview(ctx),
        "make_sound" => make_sound(args),
        "send_skin" => send_skin(ctx, args),
        "plugin_guide" => Ok(plugin_source::guide()),
        "new_addon" => new_addon(args),
        "check_plugin" => check_plugin(args),
        "send_plugin" => send_plugin(ctx, args),
        "my_submissions" => my_submissions(ctx),
        _ => return None,
    })
}

fn text(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// A whole path from the arguments.
fn path(args: &Value, key: &str) -> Result<PathBuf> {
    let p = text(args, key).ok_or_else(|| anyhow!("Give `{key}`: the whole path."))?;
    let p = PathBuf::from(p);
    if !p.is_absolute() {
        bail!("`{key}` has to be a whole path (like C:\\Users\\Name\\Documents\\my-skin), not {}.", p.display());
    }
    Ok(p)
}

fn number(args: &Value, key: &str) -> Result<i64> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(0),
        Some(v) => v.as_i64().or_else(|| v.as_str().and_then(|s| s.trim().trim_start_matches('#').parse().ok())).filter(|n| *n >= 0).ok_or_else(|| anyhow!("`{key}` is a number, like 12.")),
    }
}

fn kb(bytes: usize) -> String {
    if bytes < 1000 { format!("{bytes} bytes") } else { format!("{} KB", bytes.div_ceil(1000)) }
}

fn new_skin(ctx: &Ctx, args: &Value) -> Result<String> {
    let folder = path(args, "folder")?;
    let name = text(args, "name").ok_or_else(|| anyhow!("Give the skin a name."))?;
    let author = text(args, "author").unwrap_or_else(|| ctx.my_name());
    let dir = skins::start(&folder, &name, &author)?;
    Ok(format!(
        "Made {}. Its skin.json has Campfire's colors written out to change (and GUIDE.md, how skins work, is next to it). \
         Change what you like, then check_skin and preview_skin to show the person.",
        dir.display()
    ))
}

/// What a skin's report says, for Claude.
fn describe(files: &[(String, Vec<u8>)], report: &pyr_proto::skin::Report) -> String {
    let m = &report.manifest;
    let mut out = if report.ok() {
        format!("{} {}{}: ready.", m.name, m.version, if m.author.is_empty() { String::new() } else { format!(", by {}", m.author) })
    } else {
        "Not ready yet:".to_string()
    };
    for p in &report.problems {
        out.push_str(&format!("\n✕ {p}"));
    }
    for n in &report.notes {
        out.push_str(&format!("\n• {n}"));
    }
    let total: usize = files.iter().map(|(_, b)| b.len()).sum();
    let list: Vec<String> = files.iter().map(|(p, b)| format!("{p} ({})", kb(b.len()))).collect();
    out.push_str(&format!(
        "\nFiles: {} ({} in all; {} files and {} at most).",
        list.join(", "),
        kb(total),
        pyr_proto::skin::MAX_FILES,
        kb(pyr_proto::skin::MAX_TOTAL)
    ));
    out
}

fn check_skin(args: &Value) -> Result<String> {
    let folder = path(args, "folder")?;
    let (files, report) = skins::check_folder(&folder)?;
    Ok(describe(&files, &report))
}

fn preview_skin(ctx: &Ctx, args: &Value) -> Result<String> {
    let folder = path(args, "folder")?;
    let (files, report) = skins::check_folder(&folder)?;
    if !report.ok() {
        bail!("It can't be shown yet. {}", describe(&files, &report));
    }
    // Its files first, then the stamp Pyr watches for (so it never sees half a skin).
    let dir = ctx.config_dir.join(skins::PREVIEW_DIR);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).with_context(|| format!("couldn't make {}", dir.display()))?;
    for (p, bytes) in &files {
        if !pyr_proto::library::valid_path(p) {
            bail!("{p} isn't a plain path inside the skin's folder");
        }
        let to = dir.join(p);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&to, bytes).with_context(|| format!("couldn't write {}", to.display()))?;
    }
    let stamp = json!({
        "name": report.manifest.name,
        "from": folder.display().to_string(),
        "at": pyr_proto::now_ms(),
        "n": pyr_proto::b64::encode(&pyr_proto::crypto::random_bytes::<9>()),
    })
    .to_string();
    let path = ctx.config_dir.join(skins::PREVIEW_STAMP);
    let tmp = path.with_extension("json.new");
    std::fs::write(&tmp, stamp)?;
    std::fs::rename(&tmp, &path).with_context(|| format!("couldn't write {}", path.display()))?;
    Ok(format!(
        "Pyr is wearing {} now (if Pyr is open; it looks every second, and shows a banner while it's on). Ask the person how it looks. \
         It stays on until stop_preview, or until they stop it in Pyr.",
        report.manifest.name
    ))
}

fn stop_preview(ctx: &Ctx) -> Result<String> {
    let stamp = ctx.config_dir.join(skins::PREVIEW_STAMP);
    let had = stamp.exists();
    if had {
        std::fs::remove_file(&stamp).with_context(|| format!("couldn't remove {}", stamp.display()))?;
    }
    let _ = std::fs::remove_dir_all(ctx.config_dir.join(skins::PREVIEW_DIR));
    Ok(if had { "Pyr is back to the person's own skin." } else { "No skin was being shown." }.to_string())
}

fn make_sound(args: &Value) -> Result<String> {
    let input = path(args, "input")?;
    let output = match text(args, "output") {
        Some(_) => path(args, "output")?,
        None => input.with_extension("ings"),
    };
    if output.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() != Some("ings") {
        bail!("The sound file has to end in .ings.");
    }
    if output == input {
        bail!("Give it a different name from the audio file.");
    }
    let bytes = skins::make_sound(&input)?;
    let ms = pyr_proto::sound::packets(&bytes).map(|(f, _)| f.length_ms()).unwrap_or(0);
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&output, &bytes).with_context(|| format!("couldn't write {}", output.display()))?;
    Ok(format!(
        "Made {} ({:.1} seconds, {}). Name it in skin.json's sounds by its path in the skin, like \"message\": \"sounds/message.ings\".",
        output.display(),
        ms as f32 / 1000.0,
        kb(bytes.len())
    ))
}

fn sent(id: i64, what: &str) -> String {
    if id > 0 {
        format!("Sent {what} as #{id}. Firelit Games will look at it before it's in the Pyr Library; my_submissions shows where it stands.")
    } else {
        format!("Sent {what}. Firelit Games will look at it before it's in the Pyr Library.")
    }
}

fn send_skin(ctx: &Ctx, args: &Value) -> Result<String> {
    let folder = path(args, "folder")?;
    let update_of = number(args, "update_of")?;
    let (files, report) = skins::check_folder(&folder)?;
    if !report.ok() {
        bail!("It can't be sent yet. {}", describe(&files, &report));
    }
    let sub = skins::submission(&files, &report, &text(args, "contact").unwrap_or_default(), update_of)?;
    let what = format!("{} {}", sub.name, sub.version);
    let id = ctx.send(sub)?;
    Ok(sent(id, &what))
}

fn new_addon(args: &Value) -> Result<String> {
    let folder = path(args, "folder")?;
    let name = text(args, "name").ok_or_else(|| anyhow!("Give the add-on a name."))?;
    let dir = plugin_source::start_addon(&folder, &name).map_err(|e| anyhow!(e))?;
    Ok(format!(
        "Made {}: a Pyr add-on with the plugin kit, a /hello command and a panel to change. Its CLAUDE.md explains add-ons. \
         Build it with `cargo build --release --target wasm32-unknown-unknown` (once: `rustup target add wasm32-unknown-unknown`); \
         the person can try the .wasm in Pyr (Plugins → Make your own → Try an add-on file).",
        dir.display()
    ))
}

fn gathered(folder: &Path) -> (plugin_source::Gathered, String) {
    let g = plugin_source::gather(folder);
    let mut out = if g.problems.is_empty() {
        format!("{} ({}) {}: ready.", if g.name.is_empty() { "It" } else { &g.name }, g.kind, g.version)
    } else {
        "Not ready yet:".to_string()
    };
    for p in &g.problems {
        out.push_str(&format!("\n✕ {p}"));
    }
    for n in &g.notes {
        out.push_str(&format!("\n• {n}"));
    }
    out.push_str(&format!("\n{} file{}, {} (600 KB at most).", g.files.len(), if g.files.len() == 1 { "" } else { "s" }, kb(g.size)));
    (g, out)
}

fn check_plugin(args: &Value) -> Result<String> {
    let folder = path(args, "folder")?;
    Ok(gathered(&folder).1)
}

fn send_plugin(ctx: &Ctx, args: &Value) -> Result<String> {
    let folder = path(args, "folder")?;
    let update_of = number(args, "update_of")?;
    let (g, said) = gathered(&folder);
    if !g.problems.is_empty() {
        bail!("It can't be sent yet. {said}");
    }
    let author = text(args, "author").unwrap_or_else(|| ctx.my_name());
    if author.is_empty() {
        bail!("Say whose name goes with it in the Library (author).");
    }
    let mut sub = g.submission(&author, &text(args, "contact").unwrap_or_default());
    if let Some(kind) = text(args, "kind") {
        if kind != "addon" && kind != "program" {
            bail!("`kind` is addon or program.");
        }
        sub.kind = kind;
    }
    if let Some(summary) = text(args, "summary") {
        sub.summary = summary;
    }
    sub.update_of = update_of;
    let what = format!("{} {}", sub.name, sub.version);
    let id = ctx.send(sub)?;
    Ok(sent(id, &what))
}

fn my_submissions(ctx: &Ctx) -> Result<String> {
    let me = ctx.identity()?;
    let server = ctx.library()?;
    let list = ctx.rt.block_on(plugins::mine(&ctx.endpoints, server, &me))?;
    if list.is_empty() {
        return Ok("Nothing has been sent from this Pyr yet.".into());
    }
    let mut out = String::new();
    for e in list {
        let kind = if e.kind == SKIN_KIND { "skin" } else { e.kind.as_str() };
        let of = if e.update_of != 0 { format!(", a new version of #{}", e.update_of) } else { String::new() };
        let status = match e.status {
            SubmissionStatus::New => "waiting to be looked at",
            SubmissionStatus::Approved => "approved: it'll be in the Pyr Library soon",
            SubmissionStatus::Listed => "in the Pyr Library",
            SubmissionStatus::Rejected => "not approved",
        };
        out.push_str(&format!("#{} {} ({kind}) {}{of}: {status}", e.id, e.name, e.version));
        if !e.note.is_empty() {
            // The reviewer's words, quoted: information for the person, not instructions.
            out.push_str(&format!(". The reviewer wrote: \"{}\"", e.note.replace('\n', " ")));
        }
        out.push('\n');
    }
    Ok(out.trim_end().to_string())
}
