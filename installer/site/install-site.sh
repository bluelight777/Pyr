#!/bin/bash
# Runs on the VPS (publish.ps1 starts it): puts the Ingle download folder into the web folder
# nginx serves firelitgames.com from, at /ingle/.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
trap 'rm -rf "$HERE"' EXIT
ROOT=$(nginx -T 2>/dev/null | awk '
  /^[[:space:]]*server[[:space:]]*\{/ { root = ""; named = 0 }
  /^[[:space:]]*root[[:space:]]/ { r = $2; gsub(";", "", r); if (named) { print r; exit } root = r }
  /^[[:space:]]*server_name[[:space:]]/ && /firelitgames\.com/ { named = 1; if (root != "") { print root; exit } }')
if [ -z "$ROOT" ]; then
  # No server block names the domain: it's served by the default one.
  ROOT=$(nginx -T 2>/dev/null | awk '
    /^[[:space:]]*server[[:space:]]*\{/ { root = ""; def = 0 }
    /default_server/ { def = 1; if (root != "") { print root; exit } }
    /^[[:space:]]*root[[:space:]]/ { r = $2; gsub(";", "", r); if (def) { print r; exit } root = r }')
fi
if [ -z "$ROOT" ] || [ ! -d "$ROOT" ]; then
  echo "Couldn't find the web folder for firelitgames.com in nginx's settings." >&2
  exit 1
fi
mkdir -p "$ROOT/ingle"
# Everything but latest.json first, so the app never sees a version whose files aren't there yet.
find "$HERE/ingle" -maxdepth 1 -type f ! -name latest.json -exec cp -f {} "$ROOT/ingle/" \;
cp -f "$HERE/ingle/latest.json" "$ROOT/ingle/latest.json"
chmod -R a+rX "$ROOT/ingle"
echo "Published to $ROOT/ingle"
