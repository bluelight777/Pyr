//! # The Ingle plugin kit
//!
//! Make an **add-on** for [Ingle](https://firelitgames.com/ingle) in Rust. An add-on is built
//! to WebAssembly and runs inside Ingle in a sandbox: it can't reach the internet, your files
//! or anything else on the PC. Ingle calls it, it answers. It can:
//!
//! - add **/commands** to chat (what it answers is sent as the person's message),
//! - show a **panel** of headings, text, buttons and text boxes,
//! - keep a little **data** of its own (a JSON value, up to 64 KB).
//!
//! ```ignore
//! use ingle_plugin::*;
//!
//! struct Hello;
//!
//! impl AddOn for Hello {
//!     fn info() -> Info {
//!         Info::new("Hello").command("hello", "Says hello to someone: /hello Mara")
//!     }
//!     fn command(ctx: &mut Ctx, _name: &str, args: &str) -> Result<String, String> {
//!         let who = if args.trim().is_empty() { "everyone" } else { args.trim() };
//!         Ok(format!("{} says hello to {who}!", ctx.me))
//!     }
//! }
//!
//! export!(Hello);
//! ```
//!
//! Build it with `cargo build --release --target wasm32-unknown-unknown` (once:
//! `rustup target add wasm32-unknown-unknown`), and try the `.wasm` file in Ingle: Plugins →
//! Make your own → Try an add-on file.
//!
//! ## How Ingle talks to an add-on
//!
//! The module exports `memory`, `ingle_alloc(len) -> ptr`, `ingle_free(ptr, len)` and
//! `ingle_call(ptr, len) -> i64`, and imports nothing (Ingle refuses a module that imports
//! anything). Ingle writes a JSON request into memory it got from `ingle_alloc`, calls
//! `ingle_call`, and reads the JSON answer at `(result >> 32, result & 0xffffffff)`, then frees
//! it with `ingle_free`. [`export!`] writes all of that for you.

use serde::{Deserialize, Serialize};
pub use serde_json::{self, Value, json};

/// What Ingle passes with every call.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Ctx {
    /// Your add-on's data: what it left here last time (`null` at first). Change it and Ingle
    /// keeps the new value (up to 64 KB of JSON).
    pub store: Value,
    /// New random bits with every call, for dice and the like (see [`Ctx::random`]).
    pub seed: u64,
    /// The time, in milliseconds since 1970.
    pub now_ms: i64,
    /// The name of the person using it, and the server and channel they're looking at (empty
    /// when none is open).
    pub me: String,
    pub server: String,
    pub channel: String,
}

impl Ctx {
    /// A random number from `0` to `n - 1` (`n` at least 1).
    pub fn random(&mut self, n: u64) -> u64 {
        // splitmix64 over the seed Ingle passed in.
        self.seed = self.seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        z % n.max(1)
    }
}

/// A request from Ingle.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// What the add-on is called and which /commands it has.
    Info,
    /// Someone typed `/name args` in chat.
    Command { name: String, args: String },
    /// Draw the panel.
    Panel,
    /// A button was clicked (`value` empty), or a text box changed and Enter was pressed.
    Event { id: String, value: String },
}

/// A request with what Ingle passes along.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Call {
    #[serde(flatten)]
    pub request: Request,
    #[serde(default)]
    pub ctx: Ctx,
}

/// The add-on's answer.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Response {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<Info>,
    /// Sent to chat as the person's message (only when they used a command or clicked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub say: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<Panel>,
    /// The add-on's new data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store: Option<Value>,
    /// Shown to the person instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Info {
    pub name: String,
    #[serde(default)]
    pub commands: Vec<CommandInfo>,
}

impl Info {
    pub fn new(name: &str) -> Info {
        Info { name: name.to_string(), commands: Vec::new() }
    }

    /// Adds `/name`, with a line of help shown as people type it.
    pub fn command(mut self, name: &str, help: &str) -> Info {
        self.commands.push(CommandInfo { name: name.to_string(), help: help.to_string() });
        self
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CommandInfo {
    /// Without the slash: letters, digits and dashes.
    pub name: String,
    pub help: String,
}

/// A panel: its widgets, top to bottom.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Panel {
    pub widgets: Vec<Widget>,
}

impl Panel {
    pub fn new() -> Panel {
        Panel::default()
    }
    pub fn heading(mut self, text: impl Into<String>) -> Panel {
        self.widgets.push(Widget::Heading { text: text.into() });
        self
    }
    pub fn text(mut self, text: impl Into<String>) -> Panel {
        self.widgets.push(Widget::Text { text: text.into() });
        self
    }
    pub fn button(mut self, id: impl Into<String>, label: impl Into<String>) -> Panel {
        self.widgets.push(Widget::Button { id: id.into(), label: label.into() });
        self
    }
    /// A text box: an `Event` with its `id` and text comes when Enter is pressed in it.
    pub fn input(mut self, id: impl Into<String>, hint: impl Into<String>) -> Panel {
        self.widgets.push(Widget::Input { id: id.into(), hint: hint.into() });
        self
    }
    /// Buttons (or other widgets) side by side.
    pub fn row(mut self, widgets: Vec<Widget>) -> Panel {
        self.widgets.push(Widget::Row { widgets });
        self
    }
    pub fn divider(mut self) -> Panel {
        self.widgets.push(Widget::Divider);
        self
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Widget {
    Heading { text: String },
    Text { text: String },
    Button { id: String, label: String },
    Input { id: String, hint: String },
    Row { widgets: Vec<Widget> },
    Divider,
}

/// What an add-on does. Every function has a default, so write only the ones you need.
pub trait AddOn {
    /// Its name and /commands.
    fn info() -> Info;

    /// Someone typed `/name args`: what to send as their message, or why not (shown to them).
    fn command(_ctx: &mut Ctx, name: &str, _args: &str) -> Result<String, String> {
        Err(format!("/{name} isn't one of this add-on's commands"))
    }

    /// The panel, drawn when it's opened and after every event.
    fn panel(_ctx: &mut Ctx) -> Panel {
        Panel::new().text("This add-on has no panel.")
    }

    /// A button or text box in the panel: optionally something to send as the person's message.
    fn event(_ctx: &mut Ctx, _id: &str, _value: &str) -> Option<String> {
        None
    }
}

/// Handles one call (what [`export!`] runs; handy in tests too).
pub fn handle<A: AddOn>(call: Call) -> Response {
    let mut ctx = call.ctx;
    let before = ctx.store.clone();
    let mut r = Response::default();
    match call.request {
        Request::Info => r.info = Some(A::info()),
        Request::Command { name, args } => match A::command(&mut ctx, &name, &args) {
            Ok(say) => r.say = Some(say),
            Err(e) => r.error = Some(e),
        },
        Request::Panel => r.panel = Some(A::panel(&mut ctx)),
        Request::Event { id, value } => {
            r.say = A::event(&mut ctx, &id, &value);
            r.panel = Some(A::panel(&mut ctx));
        }
    }
    if ctx.store != before {
        r.store = Some(ctx.store);
    }
    r
}

#[doc(hidden)]
pub fn __alloc(len: i32) -> i32 {
    let mut v = Vec::<u8>::with_capacity(len.max(0) as usize);
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p as usize as i32
}

#[doc(hidden)]
/// # Safety
/// `ptr`/`len` must be what `__alloc` (or `__call`) handed out.
pub unsafe fn __free(ptr: i32, len: i32) {
    unsafe { drop(Vec::from_raw_parts(ptr as usize as *mut u8, 0, len.max(0) as usize)) }
}

#[doc(hidden)]
/// # Safety
/// `ptr`/`len` must be memory from `__alloc` that Ingle filled in.
pub unsafe fn __call<A: AddOn>(ptr: i32, len: i32) -> i64 {
    let input = unsafe { Vec::from_raw_parts(ptr as usize as *mut u8, len.max(0) as usize, len.max(0) as usize) };
    let response = match serde_json::from_slice::<Call>(&input) {
        Ok(call) => handle::<A>(call),
        Err(e) => Response { error: Some(format!("couldn't read Ingle's request: {e}")), ..Default::default() },
    };
    let mut out = serde_json::to_vec(&response).unwrap_or_default();
    out.shrink_to_fit();
    let (p, l) = (out.as_mut_ptr() as usize as i64, out.len() as i64);
    std::mem::forget(out);
    (p << 32) | l
}

/// Makes `A` the add-on this module exports (write it once, after your `impl AddOn`).
#[macro_export]
macro_rules! export {
    ($addon:ty) => {
        #[unsafe(no_mangle)]
        pub extern "C" fn ingle_alloc(len: i32) -> i32 {
            $crate::__alloc(len)
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn ingle_free(ptr: i32, len: i32) {
            unsafe { $crate::__free(ptr, len) }
        }
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn ingle_call(ptr: i32, len: i32) -> i64 {
            unsafe { $crate::__call::<$addon>(ptr, len) }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Counter;

    impl AddOn for Counter {
        fn info() -> Info {
            Info::new("Counter").command("count", "Counts")
        }
        fn command(ctx: &mut Ctx, _name: &str, _args: &str) -> Result<String, String> {
            let n = ctx.store.as_u64().unwrap_or(0) + 1;
            ctx.store = json!(n);
            Ok(format!("{n}"))
        }
        fn panel(ctx: &mut Ctx) -> Panel {
            Panel::new().heading("Count").text(format!("{}", ctx.store)).button("reset", "Reset")
        }
        fn event(ctx: &mut Ctx, id: &str, _value: &str) -> Option<String> {
            if id == "reset" {
                ctx.store = json!(0);
            }
            None
        }
    }

    fn call(json: Value) -> Response {
        handle::<Counter>(serde_json::from_value(json).unwrap())
    }

    #[test]
    fn requests_and_answers() {
        assert_eq!(call(json!({"op": "info"})).info.unwrap().commands[0].name, "count");
        let r = call(json!({"op": "command", "name": "count", "args": "", "ctx": {"store": 41}}));
        assert_eq!((r.say.as_deref(), r.store), (Some("42"), Some(json!(42))));
        let r = call(json!({"op": "event", "id": "reset", "value": "", "ctx": {"store": 5}}));
        assert_eq!(r.store, Some(json!(0)));
        assert_eq!(r.panel.unwrap().widgets[1], Widget::Text { text: "0".into() });
        // Data that didn't change isn't sent back.
        assert_eq!(call(json!({"op": "panel", "ctx": {"store": 5}})).store, None);
        let s = serde_json::to_value(Panel::new().row(vec![Widget::Button { id: "a".into(), label: "A".into() }]).divider()).unwrap();
        assert_eq!(s, json!({"widgets": [{"type": "row", "widgets": [{"type": "button", "id": "a", "label": "A"}]}, {"type": "divider"}]}));
    }

    #[test]
    fn random_numbers_stay_in_range_and_vary() {
        let mut ctx = Ctx { seed: 12345, ..Default::default() };
        let rolls: Vec<u64> = (0..200).map(|_| ctx.random(6)).collect();
        assert!(rolls.iter().all(|r| *r < 6));
        for face in 0..6 {
            assert!(rolls.contains(&face), "{face} never came up");
        }
        assert_eq!(Ctx::default().random(0), 0);
    }
}
