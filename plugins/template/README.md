# My Ingle add-on

An add-on for [Ingle](https://firelitgames.com/ingle), made with the Ingle plugin kit.

- `/hello Mara` in chat says hello.
- Its panel counts clicks and can wave in chat.

## Build

```
rustup target add wasm32-unknown-unknown     # once
cargo build --release --target wasm32-unknown-unknown
cargo test                                   # the unit tests, on your PC
```

Try it in Ingle: Plugins → Make your own → Try an add-on file, then pick
`target/wasm32-unknown-unknown/release/<name>.wasm`.

Making it with Claude? `CLAUDE.md` tells Claude how add-ons work.
