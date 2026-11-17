#!/usr/bin/env bash
# Put the props feed's maker together for a release (dist/props; make-release.sh takes it):
#   KEY=path/to/pyr-release-key-2.txt PROPS_KEY=path/to/pyr-props-feed-key.txt scripts/make-props-feed.sh
#   KEY=path/to/pyr-release-key-2.txt PROPS_PUB=<the feed key's public half> scripts/make-props-feed.sh
# Needs dist/vps/pyr-props-linux-{x86_64,aarch64} (scripts/build-vps-server.sh).
#
# The feed key signs https://pyr.gg/props/feed.json and nothing else; it lives on the VPS
# (/etc/pyr-props/feed-key.txt). The release key certifies it for 400 days (feed-cert.json),
# so each release (once a year at least) brings a fresh certificate. The first time, PROPS_KEY
# names its file (made if it isn't there; kept with the other keys, never in the repository)
# and the key goes up with the release once: setup-props.sh keeps it on the VPS and takes it
# out of what was uploaded. After that, PROPS_PUB (its public half: what this prints, and what
# the VPS says when it's set up) is enough, and the key stays where it is.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${KEY:?the release key file}"
OUT=dist/props
for A in x86_64 aarch64; do
  [ -f "dist/vps/pyr-props-linux-$A" ] || { echo "No dist/vps/pyr-props-linux-$A: run scripts/build-vps-server.sh first." >&2; exit 1; }
done
rm -rf "$OUT" && mkdir -p "$OUT"
cp dist/vps/pyr-props-linux-x86_64 dist/vps/pyr-props-linux-aarch64 "$OUT/"
# (This machine's own build does the signing: the same code.)
PROPS="cargo run -q -p pyr-props --"
if [ -n "${PROPS_KEY:-}" ]; then
  if [ ! -f "$PROPS_KEY" ]; then
    $PROPS keygen --out "$PROPS_KEY" > /dev/null
    echo "made a feed key: $PROPS_KEY (keep it with the other keys)"
  fi
  install -m 600 "$PROPS_KEY" "$OUT/feed-key.txt"
  PROPS_PUB=$($PROPS pubkey --key "$PROPS_KEY")
fi
: "${PROPS_PUB:?the public half of the feed key (or PROPS_KEY, its file)}"
$PROPS certify --release-key "$KEY" --key "$PROPS_PUB" --days 400 --out "$OUT/feed-cert.json" > /dev/null
# (What's made is what the servers will believe: the release key Pyr is built with.)
if [ -n "${PROPS_KEY:-}" ]; then
  $PROPS verify --key "$PROPS_KEY" --cert "$OUT/feed-cert.json" > /dev/null
fi
echo "the feed key: $PROPS_PUB"
ls -la "$OUT"
