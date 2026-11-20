#!/usr/bin/env bash
# A Library-only update for pyr.gg (new versions of programs between releases, since the rebuild):
#   KEY=path/to/pyr-release-key-2.txt LIST=path/to/listing.json FILES=path/to/files \
#     scripts/make-library-upload.sh
# LIST is a program's listing (installer/site/library/*.json's form: {"plugins": [...]}, every
# file named, under FILES); it's signed with the release key, the program endorsed by it (as
# make-prop-bets.sh does for a release), and dist/library-upload/ is made: upload/library.tar
# (library/files/ with only the files it names), upload/keeper/add/<id>.json, upload/
# install-library.sh, and publish-library.cmd / publish-library.ps1 to put them on the VPS (the
# Keeper takes the listing in: install-library.sh).
set -euo pipefail
cd "$(dirname "$0")/.."
: "${KEY:?the release key file}" "${LIST:?the listing}" "${FILES:?the files folder}"
OUT=dist/library-upload
rm -rf "$OUT" && mkdir -p "$OUT/upload/keeper/add" "$OUT/stage/library/files"
ID=$(python3 -c 'import json, sys; p = json.load(open(sys.argv[1]))["plugins"]; assert len(p) == 1, "one program a listing"; print(p[0]["id"])' "$LIST")
cargo run -q -p pyr-proto --example library -- sign --endorse --key "$KEY" --in "$LIST" --files "$FILES" --out "$OUT/upload/keeper/add/$ID.json"
python3 - "$OUT/upload/keeper/add/$ID.json" "$FILES" "$OUT/stage/library" <<'PY'
import json, os, shutil, sys
signed, files, out = sys.argv[1], sys.argv[2], sys.argv[3]
index = json.loads(json.load(open(signed))["index"])
named = []
for p in index["plugins"]:
    named += [p.get("icon"), p.get("wasm")] + p.get("windows", []) + p.get("linux", [])
urls = sorted({f["url"] for f in named if f})
for url in urls:
    assert url.startswith("files/") and ".." not in url, url
    dst = os.path.join(out, url)
    os.makedirs(os.path.dirname(dst), exist_ok=True)
    shutil.copyfile(os.path.join(files, url[len("files/"):]), dst)
    # (Programs for Linux run as they are.)
    if "/linux/" in url and url.endswith(".exe"):
        os.chmod(dst, 0o755)
print(f"{len(urls)} files")
PY
tar --owner=0 --group=0 --numeric-owner -cf "$OUT/upload/library.tar" -C "$OUT/stage" library/files
rm -rf "$OUT/stage"
cp installer/site/install-library.sh "$OUT/upload/"
cp installer/site/publish-library.ps1 installer/site/publish-library.cmd "$OUT/"
ls -la "$OUT" "$OUT/upload" "$OUT/upload/keeper/add"
