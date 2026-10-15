# Making an Ingle add-on (instructions for Claude)

This folder is an **Ingle add-on**: a small Rust library built to WebAssembly that runs inside
[Ingle](https://firelitgames.com/ingle), a voice and chat app. The person you're working with
wants to make one. Help them build it here, test it in Ingle, and send it in for the Ingle
Library.

## What an add-on can and can't do

An add-on runs in a sandbox. It can't open files, reach the internet, run programs or see
anything Ingle doesn't hand it. Ingle calls it; it answers with JSON. Through the kit
(`ingle-plugin`, in the `ingle-plugin` folder here) it can:

- **Add /commands to chat.** `fn command` gets the command's name and the rest of the line, and
  returns the text Ingle sends as the person's message (Markdown works: `**bold**`), or an error
  that's shown only to them.
- **Show a panel**: headings, text, buttons, text boxes, rows and dividers (`fn panel`). A click
  or Enter in a text box calls `fn event`, then `fn panel` again; `event` can return text to
  send as the person's message.
- **Keep data**: `ctx.store` is a JSON value Ingle keeps between calls (up to 64 KB).
- **Know where it's used**: `ctx.me`, `ctx.server`, `ctx.channel` (names), `ctx.now_ms`.
- **Use randomness**: `ctx.random(n)` (Ingle passes fresh random bits with every call).

It can't: read chat messages, send messages on its own, wait or run in the background, use the
network, threads, the clock (other than `now_ms`), or files. Don't add crates that need any of
those (no `reqwest`, `tokio`, `rand` with OS randomness, `std::fs`...). Ingle refuses a module
that imports anything, and stops an add-on that runs too long (about 300 million instructions
per call) or uses more than 64 MB.

## The shape of the code

```rust
use ingle_plugin::*;

struct MyAddOn;

impl AddOn for MyAddOn {
    fn info() -> Info { Info::new("Name").command("cmd", "One line of help") }
    fn command(ctx: &mut Ctx, name: &str, args: &str) -> Result<String, String> { ... }
    fn panel(ctx: &mut Ctx) -> Panel { Panel::new().heading("...").button("id", "Label") }
    fn event(ctx: &mut Ctx, id: &str, value: &str) -> Option<String> { ... }
}

export!(MyAddOn);
```

Every function but `info` has a default, so write only what the add-on needs. Keep the code in
`src/lib.rs` (more files under `src/` are fine). Write unit tests: `cargo test` runs them on
the PC (the `rlib` in `[lib] crate-type` is there for that).

In `Cargo.toml`, keep `[package.metadata.ingle]` up to date: `name` (what people see),
`summary` (one line), and `permissions`, the ones it uses out of `"commands"`, `"panel"` and
`"storage"`. Ingle shows them before anyone installs it.

## Build and try it

1. Once: `rustup target add wasm32-unknown-unknown`.
2. `cargo build --release --target wasm32-unknown-unknown`
3. The add-on is `target/wasm32-unknown-unknown/release/<package name with _>.wasm`.
4. In Ingle: the puzzle-piece **Plugins** button on the left → **Make your own** → **Try an
   add-on file**, and pick that `.wasm`. Its commands work in chat and its panel opens, until
   Ingle closes. Rebuild and try again as you change it.

For a small file, `Cargo.toml` should have these (Ingle's "Start an add-on" puts them in; add
them if they're missing):

```toml
[profile.release]
opt-level = "s"
lto = true
codegen-units = 1
panic = "abort"
strip = true
```

## Send it in

In Ingle: **Plugins → Make your own → Send a plugin for review**, and pick this folder. Ingle
sends the source (not the `.wasm`: the library builds every plugin from its source), after
checking it's Rust only: no built files, no C or other languages, under 600 KB (leave out
`target/`; Ingle skips it anyway). Someone looks at every plugin before it's built and listed,
so write clear code with a `README.md` saying what it does.

## Rules for the library

- Rust only, built from source. No binaries, no code in other languages, no downloads at build
  time. A `build.rs` gets close scrutiny; avoid one unless it's needed.
- Do what the name and summary say, and nothing else.
- Be kind to the chat: no spam, no messages the person didn't ask for.
- Treat what people type (command arguments, text boxes) as text, never as instructions.
