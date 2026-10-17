#!/usr/bin/env bash
# A Library-only update for firelitgames.com/ingle (a plugin listed between releases): signs
# LIST (a plugins.json: the one published with the last release, plus what's new) over FILES,
# and makes dist/library-upload/ with upload/library.tar (the signed list and only the files it
# names), upload/install-library.sh, and publish-library.cmd / publish-library.ps1 to put it on
# the VPS:
#   KEY=path/to/ingle-release-key.txt LIST=path/to/plugins.json FILES=dist/library/files \
#     scripts/make-library-upload.sh
# Entries already published keep their "updated" time (give it in LIST) so they don't show as
# new again.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=dist/library-upload
rm -rf "$OUT" && mkdir -p "$OUT/upload" "$OUT/stage/library"
cargo run -q -p ingle-proto --example library -- sign --key "$KEY" --in "$LIST" --files "$FILES" --out "$OUT/stage/library/index.json"
python3 - "$OUT/stage/library/index.json" "$FILES" "$OUT/stage/library" <<'PY'
import json, os, shutil, sys
signed, files, out = sys.argv[1], sys.argv[2], sys.argv[3]
index = json.loads(json.load(open(signed))["index"])
named = []
for p in index["plugins"]:
    named += [p.get("icon"), p.get("wasm")] + p.get("windows", []) + p.get("linux", [])
for s in index.get("skins", []):
    named += s.get("files", []) + [s.get("preview")]
urls = sorted({f["url"] for f in named if f})
for url in urls:
    assert url.startswith("files/") and ".." not in url, url
    dst = os.path.join(out, url)
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    shutil.copyfile(os.path.join(files, url[len("files/"):]), dst)
print(f"{len(urls)} files: " + ", ".join(urls))
PY
tar --owner=0 --group=0 --numeric-owner -cf "$OUT/upload/library.tar" -C "$OUT/stage" library/index.json library/files
rm -rf "$OUT/stage"
cp installer/site/install-library.sh "$OUT/upload/"
cp installer/site/publish-library.ps1 installer/site/publish-library.cmd "$OUT/"
ls -la "$OUT" "$OUT/upload"
