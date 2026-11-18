#!/usr/bin/env bash
# The props feed's maker by itself, to put a newer one on the VPS without a Pyr release
# (a source that changed shape, a fix): dist/props-upload, for Blue's PC, where
# publish-props.cmd uploads it and runs setup-props.sh there.
#   KEY=path/to/pyr-release-key-2.txt PROPS_PUB=<the feed key's public half> scripts/make-props-upload.sh
# Needs dist/vps/pyr-props-linux-{x86_64,aarch64} (scripts/build-vps-server.sh). No key goes
# in it: the feed's own stays on the VPS, and the certificate made here is for its public half.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${KEY:?the release key file}"
: "${PROPS_PUB:?the public half of the feed key (pyr-props pubkey --key <its file>)}"
OUT=dist/props-upload
KEY="$KEY" PROPS_PUB="$PROPS_PUB" PROPS_KEY="" scripts/make-props-feed.sh > /dev/null
[ ! -e dist/props/feed-key.txt ] || { echo "dist/props has the feed key in it: not for this." >&2; exit 1; }
rm -rf "$OUT" && mkdir -p "$OUT/upload"
cp -r dist/props "$OUT/upload/props"
cp installer/site/setup-props.sh "$OUT/upload/"
cp installer/site/publish-props.ps1 installer/site/publish-props.cmd "$OUT/"
find "$OUT" -type f | sort
echo "$OUT: run publish-props.cmd on Blue's PC"
