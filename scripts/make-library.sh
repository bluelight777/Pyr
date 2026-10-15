#!/usr/bin/env bash
# Assemble the Ingle Library for firelitgames.com/ingle/library from plugins built from their
# reviewed source:
#   KEY=path/to/ingle-release-key.txt scripts/make-library.sh
# The plugins' files must already be under dist/library/files, in the layout
# installer/site/library/plugins.json names them (Dice: built from plugins/dice for
# wasm32-unknown-unknown; programs such as Chimbrowser: built from their own source). Signs
# the list with the release key into dist/library/index.json; make-release.sh puts
# dist/library on the website with the release.
set -euo pipefail
cd "$(dirname "$0")/.."
FILES=${FILES:-dist/library/files}
mkdir -p dist/library
cargo run -q -p ingle-proto --example library -- sign --key "$KEY" --in installer/site/library/plugins.json --files "$FILES" --out dist/library/index.json
