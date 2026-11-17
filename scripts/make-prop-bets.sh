#!/usr/bin/env bash
# Prop Bets for the Pyr Library (the program Pyr opens for prop bets: crates/prop-bets), for a
# release, after make-keeper.sh (which empties dist/keeper):
#   KEY=path/to/pyr-release-key-2.txt scripts/make-prop-bets.sh
# Takes its release builds (Windows: target-win, built by scripts/build-windows.sh with the rest,
# `-p prop-bets` among them; Linux: target/release, built with the rest), puts them where the
# website's library keeps them (dist/library/files/prop-bets/: make-release.sh's library.tar
# carries them), and signs its listing (installer/site/library/prop-bets.json) with the release
# key, the program endorsed: dist/keeper/add/prop-bets.json, which setup-keeper.sh takes into
# the Keeper's list for rebuilt Pyrs (index4.json; 0.13.28's index3.json leaves it out, as the
# key before can't endorse it). One name to start on both systems (the Library's `run`):
# prop-bets.exe, the Linux one too.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${KEY:?the release key file}"
[ -d dist/keeper ] || { echo "No dist/keeper: run scripts/make-keeper.sh first." >&2; exit 1; }
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' crates/prop-bets/Cargo.toml | head -n 1)
PYR=$(sed -n '/^\[workspace.package\]/,/^\[/ s/^version = "\(.*\)"/\1/p' Cargo.toml | head -n 1)
LIST=installer/site/library/prop-bets.json
grep -q "\"version\": \"$VERSION\"" "$LIST" || { echo "$LIST isn't for Prop Bets $VERSION (crates/prop-bets/Cargo.toml)." >&2; exit 1; }
WIN=target-win/x86_64-pc-windows-gnu/release/prop-bets.exe
LIN=target/release/prop-bets
for B in "$WIN" "$LIN"; do
  [ -f "$B" ] || { echo "No $B: build Prop Bets for release first (with the rest)." >&2; exit 1; }
  # (Built from its source as it is now.)
  NEWER=$(find crates/prop-bets crates/poker/src -newer "$B" -type f | head -n 1)
  [ -z "$NEWER" ] || { echo "$B is older than $NEWER: build it again." >&2; exit 1; }
done
file -b "$WIN" | grep -q "PE32+ executable" || { echo "$WIN isn't a Windows program." >&2; exit 1; }
file -b "$LIN" | grep -q "ELF 64-bit" || { echo "$LIN isn't a Linux program." >&2; exit 1; }
FILES=dist/library/files
D="$FILES/prop-bets/$VERSION"
rm -rf "${FILES:?}/prop-bets" && mkdir -p "$D/windows" "$D/linux"
cp "$WIN" "$D/windows/prop-bets.exe"
cp "$LIN" "$D/linux/prop-bets.exe"
chmod 755 "$D/linux/prop-bets.exe"
# Its icon: the emblem on its cards.
python3 - crates/app/assets/props/emblem.webp "$FILES/prop-bets/icon.png" <<'PY'
import sys
from PIL import Image
Image.open(sys.argv[1]).convert("RGBA").resize((256, 256), Image.LANCZOS).save(sys.argv[2], optimize=True)
PY
mkdir -p dist/keeper/add
sed "s/{{PYR}}/$PYR/g" "$LIST" > dist/prop-bets-list.json
cargo run -q -p pyr-proto --example library -- sign --endorse --key "$KEY" --in dist/prop-bets-list.json --files "$FILES" --out dist/keeper/add/prop-bets.json
rm -f dist/prop-bets-list.json
ls -la "$D/windows" "$D/linux" "$FILES/prop-bets/icon.png" dist/keeper/add/prop-bets.json
