# Ingle plugins

Plugins add things to [Ingle](https://firelitgames.com/ingle). They come from the **Ingle
Library** (the puzzle-piece button in Ingle's server list), and every one is built by Firelit
Games from its Rust source after someone has read it. There are two kinds:

- **Add-ons** run inside Ingle, in a sandbox. They're Rust built to WebAssembly, and they can
  add `/commands` to chat, show a small panel and keep a little data. They can't reach your
  files, the internet or anything else on your PC. [Dice](dice) is one.
- **Programs** are apps of their own that Ingle installs and opens, like
  [Chimbrowser](https://firelitgames.com/ingle/#plugins). They run like any app you install,
  so the review is what stands between you and them: their source is read and built by
  Firelit Games, and Ingle checks every file against the library's signed list before
  installing it and again before opening it.

This folder has:

| Folder | What it is |
|---|---|
| [`kit`](kit) | `ingle-plugin`, the crate an add-on is written with |
| [`template`](template) | a starting point: a `/hello` command and a panel with a counter, plus a `CLAUDE.md` that teaches Claude how add-ons work |
| [`dice`](dice) | Dice, the first add-on in the library: `/roll 2d6+1` |

## Make an add-on with Claude

1. In Ingle: **Plugins → Make your own**, type a name and click **Start an add-on in a
   folder**. Ingle makes a folder with the kit, the template and its `CLAUDE.md`.
2. Open that folder with Claude (Claude Code or Cowork) and say what the add-on should do.
3. Build it (once: `rustup target add wasm32-unknown-unknown`):

   ```
   cargo build --release --target wasm32-unknown-unknown
   cargo test
   ```

4. In Ingle: **Plugins → Make your own → Try an add-on file**, and pick
   `target/wasm32-unknown-unknown/release/<name>.wasm`. Its commands work in chat and its panel
   opens, until Ingle closes. It runs in the same sandbox as any add-on.
5. Happy with it? **Send a plugin for review** and pick its folder. Ingle sends the source
   (Rust only, up to 600 KB, no built files), never the `.wasm`. Nothing is built or listed
   until it has been looked at.

## What an add-on looks like

```rust
use ingle_plugin::*;

struct Coin;

impl AddOn for Coin {
    fn info() -> Info {
        Info::new("Coin Flip").command("flip", "Flip a coin: /flip")
    }
    fn command(ctx: &mut Ctx, _name: &str, _args: &str) -> Result<String, String> {
        let side = if ctx.random(2) == 0 { "heads" } else { "tails" };
        Ok(format!("🪙 {} flipped **{side}**", ctx.me))
    }
}

export!(Coin);
```

What `command` returns is sent as the person's message. `panel` draws headings, text,
buttons, text boxes, rows and dividers; a click calls `event`, which can send a message too.
`ctx.store` is the add-on's own data (a JSON value up to 64 KB), and `ctx.me`, `ctx.server`
and `ctx.channel` say where it's being used. Details are in the kit's documentation
(`cargo doc -p ingle-plugin --open`) and in [`template/CLAUDE.md`](template/CLAUDE.md).

Limits: a module that imports anything is refused, and a call that runs past about 300
million instructions or uses more than 64 MB of memory is stopped. What an add-on sends is
cut at 4,000 characters, and only goes to the channel that's open, as the person's message.

## Building these

```
cargo test                                                   # the kit's, the template's and Dice's tests
cargo build --release --target wasm32-unknown-unknown -p ingle-dice
```

The library's signed list is made with `scripts/make-library.sh` from
`installer/site/library/plugins.json` (see the comments in both).
