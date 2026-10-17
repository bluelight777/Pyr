# Test fixtures

- `dice.wasm`: the Dice add-on (`plugins/dice`), built to WebAssembly with
  `cargo build --release --target wasm32-unknown-unknown -p pyr-dice` in `plugins/`. The add-on
  sandbox tests (`src/plugins/addon.rs`) run it. Rebuild it when the kit's protocol changes.
