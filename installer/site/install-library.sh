#!/bin/bash
# Runs on the VPS (publish-library.ps1 starts it): puts a new Pyr Library on
# https://pyr.gg/library/ (the web folder pyr/, which copies from before the rename also
# reach under the old name) (library.tar, from make-library-upload.sh: the plugins'
# files first, then the signed list that names them), without touching the rest of the site or
# Pyr's downloads.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
trap 'rm -rf "$HERE"' EXIT
SITE=firelitgames.com
# Find the web folder by asking (as install-site.sh does): drop a test file into each folder
# nginx serves and see which one comes back from https://firelitgames.com/ on this machine.
ROOT=""
TOKEN=$(head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n')
for R in $(nginx -T 2>/dev/null | awk '/^[[:space:]]*root[[:space:]]/ { r = $2; gsub(";", "", r); print r }' | sort -u); do
  [ -d "$R" ] || continue
  echo "$TOKEN" > "$R/pyr-probe-$TOKEN.txt" 2>/dev/null || continue
  GOT=$(curl -sk --noproxy "*" --max-time 5 --resolve "$SITE:443:127.0.0.1" "https://$SITE/pyr-probe-$TOKEN.txt" || true)
  rm -f "$R/pyr-probe-$TOKEN.txt"
  if [ "$GOT" = "$TOKEN" ]; then ROOT=$R; break; fi
done
if [ -z "$ROOT" ]; then
  echo "Couldn't find the web folder that serves https://$SITE/." >&2
  exit 1
fi
# (pyr/ since 0.9.1; before, the folder had the name from before the rename.)
WEB=$ROOT/pyr
[ -d "$WEB" ] || WEB=$ROOT/ingle
if [ ! -f "$WEB/latest.json" ]; then
  echo "There's no Pyr on https://$SITE yet: publish a release first." >&2
  exit 1
fi
if [ ! -f "$HERE/library.tar" ]; then
  echo "library.tar is missing." >&2
  exit 1
fi
mkdir -p "$HERE/library-x" "$WEB/library"
tar --no-same-owner --no-same-permissions -xf "$HERE/library.tar" -C "$HERE/library-x"
if [ ! -f "$HERE/library-x/library/index.json" ]; then
  echo "library.tar has no index.json." >&2
  exit 1
fi
# The files first, so the list never names one that isn't there yet.
if [ -d "$HERE/library-x/library/files" ]; then cp -rf "$HERE/library-x/library/files" "$WEB/library/"; fi
cp -f "$HERE/library-x/library/index.json" "$WEB/library/index.json"
# (Pyr 0.8.1 and newer read index2.json; index.json is older Pyrs' list.)
if [ -f "$HERE/library-x/library/index2.json" ]; then cp -f "$HERE/library-x/library/index2.json" "$WEB/library/index2.json"; fi
chmod -R a+rX "$WEB/library"
echo "The Pyr Library is updated in $WEB/library"
