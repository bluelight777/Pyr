#!/usr/bin/env bash
# Put the Pyr Library Keeper together for a release (dist/keeper; make-release.sh takes it):
#   KEY=path/to/pyr-release-key-2.txt KEEPER_PUB=<the Keeper's public key> \
#     LIVE=path/to/index3.json scripts/make-keeper.sh
# Needs dist/vps/pyr-keeper-linux-{x86_64,aarch64}. KEEPER_PUB is the key of the Keeper already
# on the VPS (the "key" in its /etc/pyr-keeper/keeper-cert.json): it never leaves the VPS. The
# release key (the new one since the rebuild: the one before was lost) certifies that same key
# for 400 days (keeper-cert-2.json), so each release (once a year at least) brings a fresh
# certificate; the VPS keeps the one from the key before too, for the list 0.13.28 and older read
# (index3.json), till it runs out. LIVE is the Library's list as it's up now
# (https://pyr.gg/library/index3.json): its programs endorsed again with the new key, for the
# Keeper's list for rebuilt Pyrs (index4.json). For a Keeper set up from nothing, KEEPER_KEY (a
# key file, made the first time) instead of KEEPER_PUB, and dist/library/index2.json instead of
# LIVE.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${KEY:?the release key file}"
OLD_PUB=${OLD_PUB:-J0EGQDSoDplSzp3aFrK7EA8C5Of4w4D40kaO-g7iJcE}
OUT=dist/keeper
rm -rf "$OUT" && mkdir -p "$OUT"
cp dist/vps/pyr-keeper-linux-x86_64 dist/vps/pyr-keeper-linux-aarch64 "$OUT/"
# The plugin kit and the Dice add-on (the sandbox's test build), without build folders.
for d in kit dice; do
  cp -r "plugins/$d" "$OUT/$d" && rm -rf "$OUT/$d/target"
  find "$OUT/$d" -name '.*' -mindepth 1 -prune -exec rm -rf {} +
done
LIB="cargo run -q -p pyr-proto --example library --"
if [ -n "${KEEPER_KEY:-}" ]; then
  if [ ! -f "$KEEPER_KEY" ]; then
    dist/vps/pyr-keeper-linux-x86_64 keygen --out "$KEEPER_KEY" > /dev/null
  fi
  install -m 600 "$KEEPER_KEY" "$OUT/keeper-key.txt"
  KEEPER_PUB=$($LIB pubkey --key "$KEEPER_KEY")
fi
: "${KEEPER_PUB:?the public key of the Keeper on the VPS (or KEEPER_KEY)}"
$LIB pubkey --key "$KEY" > "$OUT/release-key.pub"
$LIB certify --key "$KEY" --keeper "$KEEPER_PUB" --days 400 --out "$OUT/keeper-cert-2.json"
# The Library as it is now, programs endorsed with the new key, for the Keeper to take in.
if [ -n "${LIVE:-}" ]; then
  $LIB endorse --key "$KEY" --trust "$OLD_PUB" --in "$LIVE" --out "$OUT/index4-signed.json"
else
  $LIB endorse --key "$KEY" --in dist/library/index2.json --out "$OUT/index4-signed.json"
fi
ls -la "$OUT"
