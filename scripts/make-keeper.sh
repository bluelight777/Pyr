#!/usr/bin/env bash
# Put the Pyr Library Keeper together for a release (dist/keeper; make-release.sh takes it):
#   KEY=path/to/pyr-release-key.txt KEEPER_KEY=path/to/keeper-key.txt scripts/make-keeper.sh
# Needs dist/vps/pyr-keeper-linux-{x86_64,aarch64} and dist/library/index2.json. The Keeper's
# key is made the first time (keep it with the release key: the VPS keeps the first one it was
# given, and later certificates must name it). The release key certifies it for 400 days, so
# each release (once a year at least) brings a fresh certificate.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${KEY:?the release key file}" "${KEEPER_KEY:?the Keeper key file}"
OUT=dist/keeper
rm -rf "$OUT" && mkdir -p "$OUT"
cp dist/vps/pyr-keeper-linux-x86_64 dist/vps/pyr-keeper-linux-aarch64 "$OUT/"
# The plugin kit and the Dice add-on (the sandbox's test build), without build folders.
for d in kit dice; do
  cp -r "plugins/$d" "$OUT/$d" && rm -rf "$OUT/$d/target"
  find "$OUT/$d" -name '.*' -mindepth 1 -prune -exec rm -rf {} +
done
LIB="cargo run -q -p pyr-proto --example library --"
if [ ! -f "$KEEPER_KEY" ]; then
  dist/vps/pyr-keeper-linux-x86_64 keygen --out "$KEEPER_KEY" > /dev/null
fi
install -m 600 "$KEEPER_KEY" "$OUT/keeper-key.txt"
$LIB pubkey --key "$KEY" > "$OUT/release-key.pub"
$LIB certify --key "$KEY" --keeper "$($LIB pubkey --key "$KEEPER_KEY")" --days 400 --out "$OUT/keeper-cert.json"
# The Library as it is now, programs endorsed, for the Keeper to start from.
$LIB endorse --key "$KEY" --in dist/library/index2.json --out "$OUT/index3-signed.json"
ls -la "$OUT"
