//! Add-ons: WebAssembly run inside Ingle in a sandbox.
//!
//! A module may import nothing at all (so it can't reach files, the network or anything else
//! on the PC), and gets a fuel budget per call and a memory limit; each runs on a thread of its
//! own, so a slow one never holds up Ingle's window. Ingle and an add-on swap JSON: see the
//! plugin kit (`plugins/kit`) for the other side. An add-on that traps (a panic, running out of
//! fuel) is stopped until it's started again.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use wasmi::{Config, Engine, Linker, Memory, Module, Store, StoreLimits, StoreLimitsBuilder, TypedFunc};

/// Instructions (roughly) per call, memory, module, answer and data sizes.
pub const FUEL: u64 = 300_000_000;
pub const MEMORY_MAX: usize = 64 << 20;
pub const MODULE_MAX: usize = 8 << 20;
pub const RESPONSE_MAX: usize = 256 << 10;
pub const STORE_MAX: usize = 64 << 10;
/// Longest message an add-on can have sent (a chat message's limit).
pub const SAY_MAX: usize = 4000;
pub const WIDGETS_MAX: usize = 200;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct CommandInfo {
    pub name: String,
    #[serde(default)]
    pub help: String,
}

/// What an add-on says it is.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AddonInfo {
    pub name: String,
    #[serde(default)]
    pub commands: Vec<CommandInfo>,
}

#[derive(Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Panel {
    #[serde(default)]
    pub widgets: Vec<Widget>,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Widget {
    Heading { text: String },
    Text { text: String },
    Button { id: String, label: String },
    Input { id: String, #[serde(default)] hint: String },
    Row { widgets: Vec<Widget> },
    Divider,
    /// From a newer kit: skipped.
    #[serde(other)]
    Unknown,
}

/// An add-on's answer, as Ingle takes it (sizes already checked).
#[derive(Deserialize, Clone, Debug, Default)]
pub struct Response {
    #[serde(default)]
    pub info: Option<AddonInfo>,
    #[serde(default)]
    pub say: Option<String>,
    #[serde(default)]
    pub panel: Option<Panel>,
    #[serde(default)]
    pub store: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

/// What Ingle tells an add-on about where it's being used.
#[derive(Clone, Debug, Default)]
pub struct Place {
    pub me: String,
    pub server: String,
    pub channel: String,
}

/// A request to an add-on.
#[derive(Clone, Debug)]
pub enum Request {
    Info,
    Command { name: String, args: String },
    Panel,
    Event { id: String, value: String },
}

impl Request {
    fn json(&self) -> Value {
        match self {
            Request::Info => json!({ "op": "info" }),
            Request::Command { name, args } => json!({ "op": "command", "name": name, "args": args }),
            Request::Panel => json!({ "op": "panel" }),
            Request::Event { id, value } => json!({ "op": "event", "id": id, "value": value }),
        }
    }
}

type Reply = mpsc::Receiver<Result<Response, String>>;

struct Job {
    request: Request,
    place: Place,
    reply: mpsc::Sender<Result<Response, String>>,
}

/// A running add-on (its thread stops when this is dropped).
pub struct AddOn {
    pub info: AddonInfo,
    jobs: mpsc::Sender<Job>,
}

struct Running {
    store: Store<StoreLimits>,
    memory: Memory,
    alloc: TypedFunc<i32, i32>,
    free: TypedFunc<(i32, i32), ()>,
    call: TypedFunc<(i32, i32), i64>,
    data: Value,
    data_path: Option<PathBuf>,
}

/// Compiles and starts a module: checked to import nothing and to export what the kit does.
fn start(bytes: &[u8], data_path: Option<PathBuf>) -> Result<Running, String> {
    if bytes.len() > MODULE_MAX {
        return Err(format!("the add-on is bigger than {} MB", MODULE_MAX >> 20));
    }
    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let module = Module::new(&engine, bytes).map_err(|e| format!("not a valid add-on: {e}"))?;
    if let Some(i) = module.imports().next() {
        return Err(format!(
            "add-ons can't use anything from outside their sandbox, and this one asks for {}::{}",
            i.module(),
            i.name()
        ));
    }
    let limits = StoreLimitsBuilder::new().memory_size(MEMORY_MAX).instances(1).memories(1).tables(2).table_elements(100_000).build();
    let mut store = Store::new(&engine, limits);
    store.limiter(|l| l);
    store.set_fuel(FUEL).map_err(|e| e.to_string())?;
    let linker = <Linker<StoreLimits>>::new(&engine);
    let instance = linker.instantiate_and_start(&mut store, &module).map_err(|e| format!("the add-on didn't start: {e}"))?;
    let memory = instance.get_memory(&store, "memory").ok_or("the add-on has no memory to talk through")?;
    let need = |e: wasmi::Error| format!("it isn't an Ingle add-on (made with the plugin kit): {e}");
    let alloc = instance.get_typed_func::<i32, i32>(&store, "ingle_alloc").map_err(need)?;
    let free = instance.get_typed_func::<(i32, i32), ()>(&store, "ingle_free").map_err(need)?;
    let call = instance.get_typed_func::<(i32, i32), i64>(&store, "ingle_call").map_err(need)?;
    let data = data_path
        .as_ref()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    Ok(Running { store, memory, alloc, free, call, data, data_path })
}

fn clip(s: &mut String, max: usize) {
    if s.chars().count() > max {
        *s = s.chars().take(max).collect();
    }
}

/// Keeps what an add-on sends within bounds: text lengths, widget counts, ids.
fn tidy_panel(p: &mut Panel) {
    let mut left = WIDGETS_MAX;
    fn walk(ws: &mut Vec<Widget>, left: &mut usize, depth: usize) {
        ws.retain(|w| !matches!(w, Widget::Unknown));
        if ws.len() > *left {
            ws.truncate(*left);
        }
        *left -= ws.len();
        for w in ws.iter_mut() {
            match w {
                Widget::Heading { text } | Widget::Text { text } => clip(text, 2000),
                Widget::Button { id, label } => {
                    clip(id, 64);
                    clip(label, 60);
                }
                Widget::Input { id, hint } => {
                    clip(id, 64);
                    clip(hint, 120);
                }
                Widget::Row { widgets } => {
                    if depth > 1 {
                        widgets.clear();
                    } else {
                        walk(widgets, left, depth + 1);
                    }
                }
                Widget::Divider | Widget::Unknown => {}
            }
        }
    }
    walk(&mut p.widgets, &mut left, 0);
}

fn tidy_info(i: &mut AddonInfo) -> Result<(), String> {
    clip(&mut i.name, 40);
    if i.name.trim().is_empty() {
        return Err("the add-on has no name".into());
    }
    i.commands.truncate(20);
    for c in &mut i.commands {
        c.name = c.name.trim_start_matches('/').to_lowercase();
        if c.name.is_empty() || c.name.len() > 24 || !c.name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
            return Err(format!("/{} isn't a command name Ingle can use (letters, digits and dashes)", c.name));
        }
        clip(&mut c.help, 120);
    }
    Ok(())
}

impl Running {
    /// One call: the request (with where it's used, its data and fresh random bits) in, the
    /// answer out.
    fn call(&mut self, request: &Request, place: &Place) -> Result<Response, String> {
        let seed = u64::from_le_bytes(ingle_proto::crypto::random_bytes::<8>());
        let mut msg = request.json();
        msg["ctx"] = json!({
            "store": self.data,
            "seed": seed,
            "now_ms": ingle_proto::now_ms(),
            "me": place.me,
            "server": place.server,
            "channel": place.channel,
        });
        let input = serde_json::to_vec(&msg).map_err(|e| e.to_string())?;
        self.store.set_fuel(FUEL).map_err(|e| e.to_string())?;
        let trap = |e: wasmi::Error| {
            let s = e.to_string();
            if s.contains("fuel") { "it took too long, so it was stopped".to_string() } else { format!("it crashed ({s})") }
        };
        let ptr = self.alloc.call(&mut self.store, input.len() as i32).map_err(trap)?;
        self.memory.write(&mut self.store, ptr as u32 as usize, &input).map_err(|_| "it gave Ingle memory it doesn't have".to_string())?;
        let packed = self.call.call(&mut self.store, (ptr, input.len() as i32)).map_err(trap)?;
        let (rptr, rlen) = (((packed as u64) >> 32) as usize, (packed as u64 & 0xffff_ffff) as usize);
        if rlen > RESPONSE_MAX {
            return Err("its answer was too big".into());
        }
        let mut out = vec![0u8; rlen];
        self.memory.read(&self.store, rptr, &mut out).map_err(|_| "its answer wasn't in its memory".to_string())?;
        let _ = self.free.call(&mut self.store, (rptr as i32, rlen as i32));
        let mut r: Response = serde_json::from_slice(&out).map_err(|e| format!("Ingle couldn't read its answer: {e}"))?;
        if let Some(s) = &mut r.say {
            clip(s, SAY_MAX);
        }
        if let Some(e) = &mut r.error {
            clip(e, 300);
        }
        if let Some(p) = &mut r.panel {
            tidy_panel(p);
        }
        if let Some(i) = &mut r.info {
            tidy_info(i)?;
        }
        if let Some(new) = r.store.take() {
            let bytes = serde_json::to_vec(&new).unwrap_or_default();
            if bytes.len() > STORE_MAX {
                r.error.get_or_insert_with(|| format!("its data grew past {} KB, so it wasn't kept", STORE_MAX >> 10));
            } else {
                self.data = new;
                if let Some(path) = &self.data_path {
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let tmp = path.with_extension("json.tmp");
                    if std::fs::write(&tmp, &bytes).is_ok() {
                        let _ = std::fs::rename(&tmp, path);
                    }
                }
            }
        }
        Ok(r)
    }
}

impl AddOn {
    /// Starts an add-on on a thread of its own and asks what it is. `data_path`: where its data
    /// is kept (`None`: nowhere, for one being tried out).
    pub fn load(id: &str, bytes: Vec<u8>, data_path: Option<PathBuf>, repaint: Arc<dyn Fn() + Send + Sync>) -> Result<AddOn, String> {
        let (jobs, rx) = mpsc::channel::<Job>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        // Why it stopped, once it has (it answers nothing after that).
        let stop: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        std::thread::Builder::new()
            .name(format!("ingle-addon-{id}"))
            .stack_size(8 << 20)
            .spawn(move || {
                let mut running = match start(&bytes, data_path) {
                    Ok(r) => {
                        let _ = ready_tx.send(Ok(()));
                        r
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                drop(bytes);
                while let Ok(job) = rx.recv() {
                    if let Some(why) = stop.lock().clone() {
                        let _ = job.reply.send(Err(why));
                        continue;
                    }
                    let r = running.call(&job.request, &job.place);
                    if let Err(e) = &r {
                        // Whatever it was in the middle of may be broken now: stop it.
                        tracing::warn!("add-on stopped: {e}");
                        *stop.lock() = Some(e.clone());
                    }
                    let _ = job.reply.send(r);
                    repaint();
                }
            })
            .map_err(|e| e.to_string())?;
        ready_rx.recv_timeout(Duration::from_secs(20)).map_err(|_| "the add-on didn't start in time".to_string())??;
        let mut addon = AddOn { info: AddonInfo::default(), jobs };
        let info = addon
            .ask(Request::Info, Place::default())
            .recv_timeout(Duration::from_secs(20))
            .map_err(|_| "the add-on didn't answer".to_string())??;
        addon.info = info.info.ok_or("the add-on didn't say what it is")?;
        Ok(addon)
    }

    /// Asks it something; the answer comes on the returned channel (and the window repaints).
    pub fn ask(&self, request: Request, place: Place) -> Reply {
        let (reply, rx) = mpsc::channel();
        if self.jobs.send(Job { request, place, reply: reply.clone() }).is_err() {
            let _ = reply.send(Err("the add-on isn't running".into()));
        }
        rx
    }

    pub fn has_command(&self, name: &str) -> bool {
        self.info.commands.iter().any(|c| c.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dice() -> Vec<u8> {
        // Built from plugins/dice (see the fixture's README).
        std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/dice.wasm")).unwrap()
    }

    fn wait(rx: Reply) -> Result<Response, String> {
        rx.recv_timeout(Duration::from_secs(30)).unwrap()
    }

    #[test]
    fn dice_rolls_and_keeps_its_data() {
        let dir = super::super::library::temp_dir("addon");
        let data = dir.join("data/dice.json");
        let a = AddOn::load("dice", dice(), Some(data.clone()), Arc::new(|| {})).unwrap();
        assert_eq!(a.info.name, "Dice");
        assert!(a.has_command("roll"));
        let place = Place { me: "Rae".into(), server: "Camp".into(), channel: "General".into() };
        let r = wait(a.ask(Request::Command { name: "roll".into(), args: "2d6+1".into() }, place.clone())).unwrap();
        let say = r.say.unwrap();
        assert!(say.starts_with("🎲 2d6+1: "), "{say}");
        // Its data (the last rolls) is kept on disk.
        let kept: Value = serde_json::from_slice(&std::fs::read(&data).unwrap()).unwrap();
        assert_eq!(kept["last"][0], json!(say));
        let r = wait(a.ask(Request::Panel, place.clone())).unwrap();
        let shown = format!("{:?}", r.panel.unwrap());
        assert!(shown.contains("Button { id: \"d20\""), "{shown}");
        let r = wait(a.ask(Request::Command { name: "roll".into(), args: "lots".into() }, place)).unwrap();
        assert!(r.say.is_none() && r.error.unwrap().contains("can't read"));
        // A new start picks its data up again.
        drop(a);
        let a = AddOn::load("dice", dice(), Some(data), Arc::new(|| {})).unwrap();
        let r = wait(a.ask(Request::Panel, Place::default())).unwrap();
        assert!(format!("{:?}", r.panel.unwrap()).contains("2d6+1"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn modules_that_reach_outside_or_run_away_are_stopped() {
        let wat = |src: &str| wat::parse_str(src).unwrap();
        // Asking for anything from outside: refused before it runs.
        let imports = wat(r#"(module (import "env" "open_file" (func)) (memory (export "memory") 1))"#);
        let err = AddOn::load("x", imports, None, Arc::new(|| {})).err().unwrap();
        assert!(err.contains("env::open_file"), "{err}");
        // Not made with the kit.
        let bare = wat(r#"(module (memory (export "memory") 1))"#);
        assert!(AddOn::load("x", bare, None, Arc::new(|| {})).err().unwrap().contains("isn't an Ingle add-on"));
        // Too much memory from the start.
        let huge = wat(r#"(module (memory (export "memory") 2000))"#);
        assert!(AddOn::load("x", huge, None, Arc::new(|| {})).is_err());
        // Loops forever when asked what it is: out of fuel, stopped (and doesn't hang Ingle).
        let spin = wat(
            r#"(module (memory (export "memory") 1)
                 (func (export "ingle_alloc") (param i32) (result i32) i32.const 16)
                 (func (export "ingle_free") (param i32 i32))
                 (func (export "ingle_call") (param i32 i32) (result i64) (loop br 0) i64.const 0))"#,
        );
        let started = std::time::Instant::now();
        let err = AddOn::load("spin", spin, None, Arc::new(|| {})).err().unwrap();
        assert!(err.contains("took too long"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(20));
        // Claims an answer far outside its memory.
        let liar = wat(
            r#"(module (memory (export "memory") 1)
                 (func (export "ingle_alloc") (param i32) (result i32) i32.const 16)
                 (func (export "ingle_free") (param i32 i32))
                 (func (export "ingle_call") (param i32 i32) (result i64) i64.const 0x7fff000000000100))"#,
        );
        assert!(AddOn::load("liar", liar, None, Arc::new(|| {})).err().unwrap().contains("wasn't in its memory"));
    }

    #[test]
    fn what_add_ons_send_is_kept_in_bounds() {
        let mut p = Panel { widgets: (0..500).map(|i| Widget::Text { text: "x".repeat(i * 10) }).collect() };
        tidy_panel(&mut p);
        assert_eq!(p.widgets.len(), WIDGETS_MAX);
        assert!(p.widgets.iter().all(|w| matches!(w, Widget::Text { text } if text.chars().count() <= 2000)));
        let mut i = AddonInfo { name: "Ok".into(), commands: vec![CommandInfo { name: "Roll Dice".into(), help: String::new() }] };
        assert!(tidy_info(&mut i).is_err());
        let mut i = AddonInfo { name: "Ok".into(), commands: vec![CommandInfo { name: "/Roll".into(), help: "h".repeat(500) }] };
        tidy_info(&mut i).unwrap();
        assert_eq!((i.commands[0].name.as_str(), i.commands[0].help.len()), ("roll", 120));
    }
}
