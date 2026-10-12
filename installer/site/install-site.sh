#!/bin/bash
# Runs on the VPS (publish.ps1 starts it): puts the Ingle download folder into the web folder
# that serves https://firelitgames.com/, at /ingle/.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
trap 'rm -rf "$HERE"' EXIT
SITE=firelitgames.com
# Find the web folder by asking: drop a test file into each folder nginx serves and see which
# one comes back from https://firelitgames.com/ on this machine. (Reading nginx's settings
# isn't enough: the same server also hosts other sites, like mail.firelitgames.com.)
ROOT=""
TOKEN=$(head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n')
for R in $(nginx -T 2>/dev/null | awk '/^[[:space:]]*root[[:space:]]/ { r = $2; gsub(";", "", r); print r }' | sort -u); do
  [ -d "$R" ] || continue
  echo "$TOKEN" > "$R/ingle-probe-$TOKEN.txt" 2>/dev/null || continue
  GOT=$(curl -sk --noproxy "*" --max-time 5 --resolve "$SITE:443:127.0.0.1" "https://$SITE/ingle-probe-$TOKEN.txt" || true)
  rm -f "$R/ingle-probe-$TOKEN.txt"
  if [ "$GOT" = "$TOKEN" ]; then ROOT=$R; break; fi
done
if [ -z "$ROOT" ]; then
  echo "Couldn't find the web folder that serves https://$SITE/." >&2
  exit 1
fi
mkdir -p "$ROOT/ingle"
# Everything but latest.json first, so the app never sees a version whose files aren't there yet.
find "$HERE/ingle" -maxdepth 1 -type f ! -name latest.json -exec cp -f {} "$ROOT/ingle/" \;
cp -f "$HERE/ingle/latest.json" "$ROOT/ingle/latest.json"
chmod -R a+rX "$ROOT/ingle"
echo "Published to $ROOT/ingle"
# An earlier version of this script could put the files in another site's folder: remove
# those copies (only folders holding an Ingle release file).
for R in $(nginx -T 2>/dev/null | awk '/^[[:space:]]*root[[:space:]]/ { r = $2; gsub(";", "", r); print r }' | sort -u); do
  if [ "$R" != "$ROOT" ] && [ -f "$R/ingle/latest.json" ] && grep -q '"manifest"' "$R/ingle/latest.json"; then
    rm -rf "${R:?}/ingle" && echo "Removed a stray copy from $R/ingle"
  fi
done
