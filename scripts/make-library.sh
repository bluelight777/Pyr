#!/usr/bin/env bash
# Assemble the Pyr Library for pyr.gg/library from plugins built from their
# reviewed source, and skins:
#   KEY=path/to/pyr-release-key.txt scripts/make-library.sh
# The plugins' files must already be under dist/library/files, in the layout
# installer/site/library/plugins.json names them (Dice: built from plugins/dice for
# wasm32-unknown-unknown; programs such as Chimbrowser: built from their own source). Signs
# the list with the release key into dist/library/index.json; make-release.sh puts
# dist/library on the website with the release.
set -euo pipefail
cd "$(dirname "$0")/.."
FILES=${FILES:-dist/library/files}
mkdir -p dist/library
# Skins Firelit Games makes are in skins/<id>/ (their pictures drawn by tools/make-skin-art.py):
# each listed skin's folder goes under its "dir" (like blood-moon/1.0), hidden files left out.
python3 - "$FILES" <<'PY'
import json, os, shutil, sys
files = sys.argv[1]
for s in json.load(open("installer/site/library/plugins.json")).get("skins", []):
    src = os.path.join("skins", s["id"])
    if not os.path.isdir(src):
        continue
    dst = os.path.join(files, s["dir"])
    shutil.rmtree(dst, ignore_errors=True)
    shutil.copytree(src, dst, ignore=shutil.ignore_patterns(".*"))
    print(f"skin {s['id']}: {', '.join(sorted(os.listdir(dst)))}")
PY
cargo run -q -p pyr-proto --example library -- sign --key "$KEY" --in installer/site/library/plugins.json --files "$FILES" --out dist/library/index.json
