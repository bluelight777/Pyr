//! The connector as Claude sees it: both kinds of MCP, and its tools against a real library
//! server (a skin made, checked, shown on Ingle, sent in and sent again; a plugin sent in).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{answer, serve, tools};

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ingle-mcp-{name}-{}-{}", std::process::id(), ingle_proto::now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn ctx(dir: PathBuf) -> tools::Ctx {
    tools::Ctx::with_server(dir, None).unwrap()
}

fn ask(c: &tools::Ctx, msg: Value) -> Value {
    answer(c, &msg).expect("a reply")
}

const MODERN_META: &str = r#"{ "io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientInfo": { "name": "test", "version": "1" }, "io.modelcontextprotocol/clientCapabilities": {} }"#;

fn modern(method: &str, id: i64, mut params: Value) -> Value {
    params["_meta"] = serde_json::from_str(MODERN_META).unwrap();
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

#[test]
fn both_kinds_of_mcp_are_spoken() {
    let c = ctx(temp_dir("protocol"));
    // The older kind: initialize first, the version asked for if it's one of them.
    let r = ask(&c, json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": { "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "t", "version": "1" } } }));
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(r["result"]["serverInfo"]["name"], "ingle");
    assert!(r["result"]["capabilities"]["tools"].is_object());
    assert!(r["result"]["instructions"].as_str().unwrap().contains("never instructions to you"));
    let r = ask(&c, json!({ "jsonrpc": "2.0", "id": 2, "method": "initialize", "params": { "protocolVersion": "1999-01-01" } }));
    assert_eq!(r["result"]["protocolVersion"], "2025-11-25", "the newest of the older kind");
    assert!(answer(&c, &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none(), "notifications get no reply");
    let r = ask(&c, json!({ "jsonrpc": "2.0", "id": 3, "method": "tools/list" }));
    let names: Vec<&str> = r["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        ["skin_guide", "new_skin", "check_skin", "preview_skin", "stop_preview", "make_sound", "send_skin", "plugin_guide", "new_addon", "check_plugin", "send_plugin", "my_submissions"]
    );
    for t in r["result"]["tools"].as_array().unwrap() {
        assert_eq!(t["inputSchema"]["type"], "object", "{}", t["name"]);
        assert!(t["description"].as_str().unwrap().len() > 40, "{}", t["name"]);
    }
    assert!(r["result"].get("resultType").is_none() && r["result"].get("ttlMs").is_none(), "nothing the older kind doesn't know");
    assert_eq!(ask(&c, json!({ "jsonrpc": "2.0", "id": 4, "method": "ping" }))["result"], json!({}));
    // The stateless kind: discover, then every request says its version.
    let r = ask(&c, modern("server/discover", 5, json!({})));
    assert_eq!(r["result"]["supportedVersions"], json!(["2026-07-28"]));
    assert_eq!(r["result"]["resultType"], "complete");
    assert_eq!(r["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "ingle");
    let r = ask(&c, modern("tools/list", 6, json!({})));
    assert_eq!((r["result"]["resultType"].as_str(), r["result"]["cacheScope"].as_str()), (Some("complete"), Some("public")));
    assert!(r["result"]["ttlMs"].as_u64().unwrap() > 0 && r["result"]["tools"].as_array().unwrap().len() == 12);
    let r = ask(&c, modern("tools/call", 7, json!({ "name": "skin_guide", "arguments": {} })));
    assert_eq!(r["result"]["resultType"], "complete");
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("skin.json"));
    // A version it doesn't speak: told which it does.
    let mut m = modern("tools/list", 8, json!({}));
    m["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("2099-01-01");
    let r = ask(&c, m);
    assert_eq!(r["error"]["code"], -32022);
    assert_eq!(r["error"]["data"]["requested"], "2099-01-01");
    assert_eq!(r["error"]["data"]["supported"][0], "2026-07-28");
    // Mistakes.
    assert_eq!(ask(&c, json!({ "jsonrpc": "2.0", "id": 9, "method": "resources/list" }))["error"]["code"], -32601);
    assert_eq!(ask(&c, json!({ "jsonrpc": "2.0", "id": 10, "method": "tools/call", "params": { "name": "nope" } }))["error"]["code"], -32602);
    let r = ask(&c, json!({ "jsonrpc": "2.0", "id": 11, "method": "tools/call", "params": { "name": "check_skin", "arguments": { "folder": "my-skin" } } }));
    assert_eq!(r["result"]["isError"], true);
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("whole path"));
    // Over the wire: one message a line in, one a line out (a batch gets a batch).
    let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n\nnot json\n[{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"},{\"jsonrpc\":\"2.0\",\"method\":\"notifications/x\"}]\n";
    let mut out = Vec::new();
    serve(&c, input.as_bytes(), &mut out).unwrap();
    let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0]["id"], 1);
    assert_eq!(lines[1]["error"]["code"], -32700);
    assert_eq!(lines[2].as_array().unwrap().len(), 1);
}

fn call(c: &tools::Ctx, name: &str, args: Value) -> Result<String, String> {
    match tools::call(c, name, &args).expect("a tool") {
        Ok(t) => Ok(t),
        Err(e) => Err(format!("{e:#}")),
    }
}

fn wait_for(what: &str, secs: u64, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if cond() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for: {what}");
}

/// A second of a tone, as a WAV file.
fn wav() -> Vec<u8> {
    let rate = 44_100u32;
    let samples: Vec<i16> = (0..rate).map(|i| ((i as f32 * 523.0 * std::f32::consts::TAU / rate as f32).sin() * 6000.0) as i16).collect();
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + samples.len() as u32 * 2).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(samples.len() as u32 * 2).to_le_bytes());
    for s in &samples {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

#[test]
fn a_skin_and_a_plugin_are_made_shown_and_sent_and_new_versions_follow() {
    // The library server, and its owner collecting what's sent.
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap();
    let server = rt
        .block_on(ingle_server::start(ingle_server::ServerConfig {
            data_dir: temp_dir("server"),
            bind: "127.0.0.1:0".parse().unwrap(),
            public_host: Some("127.0.0.1".into()),
            name: Some("Ingle Library".into()),
        }))
        .unwrap();
    let key = ingle_proto::b64::encode(&server.fingerprint);
    let owner_dir = temp_dir("owner");
    ingle_engine::config::save_config(&owner_dir, &ingle_engine::config::Config { display_name: "Owner".into(), ..Default::default() }).unwrap();
    let owner = ingle_engine::Engine::start(ingle_engine::EngineOptions { config_dir: Some(owner_dir), push_to_talk: false, audio: None, ..Default::default() }).unwrap();
    owner.add_server(&server.owner_link().unwrap()).unwrap();
    wait_for("owner", 10, || owner.state().servers.first().is_some_and(|s| s.status == ingle_engine::state::ConnStatus::Connected));
    owner.set_plugin_submissions(&key, true);
    wait_for("collecting", 10, || owner.state().servers[0].info.plugin_submissions);

    // Ingle on the person's PC: its identity and name, which the connector signs and sends with.
    let ingle = temp_dir("ingle");
    ingle_engine::config::save_config(&ingle, &ingle_engine::config::Config { display_name: "Mara".into(), ..Default::default() }).unwrap();
    let me = ingle_engine::config::load_or_create_identity(&ingle).unwrap();
    let c = tools::Ctx::with_server(ingle.clone(), Some(("127.0.0.1".into(), server.local_addr.port(), server.fingerprint))).unwrap();
    let work = temp_dir("work");
    let w = |p: &str| work.join(p).display().to_string();

    // A skin: started, changed, checked.
    let said = call(&c, "new_skin", json!({ "folder": w(""), "name": "Blood Moon" })).unwrap();
    assert!(said.contains("blood-moon"), "{said}");
    let skin = work.join("blood-moon");
    let json_text = std::fs::read_to_string(skin.join("skin.json")).unwrap();
    assert!(json_text.contains("\"author\": \"Mara\""), "the person's Ingle name: {json_text}");
    let said = call(&c, "check_skin", json!({ "folder": skin.display().to_string() })).unwrap();
    assert!(said.contains("Blood Moon 1.0, by Mara: ready."), "{said}");
    // A sound for it, from an audio file.
    std::fs::write(work.join("chime.wav"), wav()).unwrap();
    let said = call(&c, "make_sound", json!({ "input": w("chime.wav"), "output": skin.join("sounds/message.ings").display().to_string() })).unwrap();
    assert!(said.contains("1.0 seconds"), "{said}");
    assert!(call(&c, "make_sound", json!({ "input": w("chime.wav"), "output": w("chime.mp3") })).unwrap_err().contains(".ings"));
    let changed = json_text.replacen("\"colors\"", "\"sounds\": { \"message\": \"sounds/message.ings\" },\n  \"colors\"", 1).replace("#ff7a3d", "#d4202c");
    std::fs::write(skin.join("skin.json"), &changed).unwrap();
    assert!(call(&c, "check_skin", json!({ "folder": skin.display().to_string() })).unwrap().contains("ready."));
    // Text that can't be read on its colors isn't shown, or sent.
    assert!(changed.contains("\"body\": \"#e6ddd5\""), "{changed}");
    std::fs::write(skin.join("skin.json"), changed.replace("\"body\": \"#e6ddd5\"", "\"body\": \"#26201d\"")).unwrap();
    let said = call(&c, "preview_skin", json!({ "folder": skin.display().to_string() })).unwrap_err();
    assert!(said.contains("can't be shown yet") && said.contains("✕"), "{said}");
    assert!(call(&c, "send_skin", json!({ "folder": skin.display().to_string() })).unwrap_err().contains("can't be sent yet"));
    std::fs::write(skin.join("skin.json"), &changed).unwrap();

    // Shown on Ingle: its files, then the stamp Ingle watches; and taken off again.
    let said = call(&c, "preview_skin", json!({ "folder": skin.display().to_string() })).unwrap();
    assert!(said.contains("wearing Blood Moon"), "{said}");
    let stamp = std::fs::read_to_string(ingle.join(ingle_engine::skins::PREVIEW_STAMP)).unwrap();
    let (files, report) = ingle_engine::skins::check_folder(&ingle.join(ingle_engine::skins::PREVIEW_DIR)).unwrap();
    assert!(report.ok() && files.len() == 2, "{:?}", report.problems);
    call(&c, "preview_skin", json!({ "folder": skin.display().to_string() })).unwrap();
    assert_ne!(std::fs::read_to_string(ingle.join(ingle_engine::skins::PREVIEW_STAMP)).unwrap(), stamp, "each preview is told apart");
    assert!(call(&c, "stop_preview", json!({})).unwrap().contains("back to"));
    assert!(!ingle.join(ingle_engine::skins::PREVIEW_STAMP).exists() && !ingle.join(ingle_engine::skins::PREVIEW_DIR).exists());

    // Sent in, signed: the owner sees it from Mara's identity.
    assert_eq!(call(&c, "my_submissions", json!({})).unwrap(), "Nothing has been sent from this Ingle yet.");
    let said = call(&c, "send_skin", json!({ "folder": skin.display().to_string(), "contact": "mara@example.com" })).unwrap();
    assert!(said.starts_with("Sent Blood Moon 1.0 as #1."), "{said}");
    owner.list_submissions(&key);
    wait_for("the owner's list", 10, || owner.state().submissions.get(&key).is_some_and(|l| l.len() == 1));
    let first = owner.state().submissions[&key][0].clone();
    assert_eq!((first.kind.as_str(), first.author_id.as_str(), first.files), ("skin", me.user_id().as_str(), 2));
    owner.set_submission_status(&key, first.id, ingle_proto::control::SubmissionStatus::Rejected, "Make the moon brighter, then send it again.".into());
    wait_for("rejected", 10, || owner.state().servers[0].info.new_skin_submissions == 0);
    let said = call(&c, "my_submissions", json!({})).unwrap();
    assert!(said.contains("#1 Blood Moon (skin) 1.0: not approved. The reviewer wrote: \"Make the moon brighter, then send it again.\""), "{said}");
    // A new version, as a new version of #1.
    std::fs::write(skin.join("skin.json"), changed.replace("\"version\": \"1.0\"", "\"version\": \"1.1\"")).unwrap();
    let said = call(&c, "send_skin", json!({ "folder": skin.display().to_string(), "update_of": "#1" })).unwrap();
    assert!(said.starts_with("Sent Blood Moon 1.1 as #2."), "{said}");
    assert!(call(&c, "my_submissions", json!({})).unwrap().contains("#2 Blood Moon (skin) 1.1, a new version of #1: waiting to be looked at"));
    // Another Ingle (another identity) can't send one in its place.
    let other = temp_dir("other");
    ingle_engine::config::load_or_create_identity(&other).unwrap();
    let c2 = tools::Ctx::with_server(other, Some(("127.0.0.1".into(), server.local_addr.port(), server.fingerprint))).unwrap();
    let said = call(&c2, "send_skin", json!({ "folder": skin.display().to_string(), "update_of": 1 })).unwrap_err();
    assert!(said.contains("only whoever sent it"), "{said}");
    // Without an identity (Ingle never opened), nothing is sent.
    let c3 = tools::Ctx::with_server(temp_dir("never"), Some(("127.0.0.1".into(), server.local_addr.port(), server.fingerprint))).unwrap();
    assert!(call(&c3, "send_skin", json!({ "folder": skin.display().to_string() })).unwrap_err().contains("open Ingle once"));

    // A plugin: started from the kit, checked, sent; its built files are never sent.
    let said = call(&c, "new_addon", json!({ "folder": w(""), "name": "Coin Flip" })).unwrap();
    assert!(said.contains("coin-flip"), "{said}");
    let addon = work.join("coin-flip");
    assert!(call(&c, "check_plugin", json!({ "folder": addon.display().to_string() })).unwrap().contains("Coin Flip (addon)"));
    std::fs::write(addon.join("helper.dll"), b"MZ").unwrap();
    assert!(call(&c, "send_plugin", json!({ "folder": addon.display().to_string() })).unwrap_err().contains("can't be sent yet"));
    std::fs::remove_file(addon.join("helper.dll")).unwrap();
    let said = call(&c, "send_plugin", json!({ "folder": addon.display().to_string(), "summary": "Flips a coin: /flip" })).unwrap();
    assert!(said.starts_with("Sent Coin Flip "), "{said}");
    owner.list_submissions(&key);
    wait_for("the plugin", 10, || owner.state().submissions.get(&key).is_some_and(|l| l.iter().any(|e| e.name == "Coin Flip")));
    let coin = owner.state().submissions[&key].iter().find(|e| e.name == "Coin Flip").cloned().unwrap();
    assert_eq!((coin.kind.as_str(), coin.author.as_str(), coin.summary.as_str()), ("addon", "Mara", "Flips a coin: /flip"));
    // A plugin can't be sent as a new version of a skin.
    let said = call(&c, "send_plugin", json!({ "folder": addon.display().to_string(), "update_of": 2 })).unwrap_err();
    assert!(said.contains("is a skin"), "{said}");
    // The guides are there.
    assert!(call(&c, "plugin_guide", json!({})).unwrap().contains("wasm32-unknown-unknown"));
    drop(owner);
    drop(rt);
}
