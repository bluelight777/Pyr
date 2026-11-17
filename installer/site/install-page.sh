#!/bin/bash
# Runs on the VPS (publish-page.ps1 starts it): puts a new copy of pyr.gg's page (index.html,
# from make-page.sh) in the web folder, and nothing else: the downloads, the release files, the
# Library, the server and the Keeper stay as they are. For a fix to the page between releases.
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
[ -d "$ROOT/ingle" ] && [ ! -L "$ROOT/ingle" ] && WEB=$ROOT/ingle
# The page reads the release file the rebuilt Pyrs read: it goes only where that's up.
if [ ! -f "$WEB/latest2.json" ]; then
  echo "There's no Pyr 0.14.0 or newer on https://$SITE yet: publish a release first." >&2
  exit 1
fi
# The page it replaces is kept in root's own folder (not on the website), to put back by hand.
KEEP=/root/pyr-page-before-$(date +%Y%m%d-%H%M%S).html
[ -f "$WEB/index.html" ] && cp -p "$WEB/index.html" "$KEEP"
install -m 644 "$HERE/pyr/index.html" "$WEB/index.html.new"
mv -f "$WEB/index.html.new" "$WEB/index.html"
# The website serves it (the invite directory reads it again by itself when it changes).
WANT=$(sha256sum < "$HERE/pyr/index.html" | cut -d' ' -f1)
GOT=$(curl -sk --noproxy "*" --max-time 10 --resolve "$SITE:443:127.0.0.1" "https://$SITE/pyr/index.html" | sha256sum | cut -d' ' -f1)
CODE=$(curl -sk --noproxy "*" --max-time 10 -o /dev/null -w "%{http_code}" --resolve "pyr.gg:443:127.0.0.1" "https://pyr.gg/" || true)
if [ "$GOT" = "$WANT" ] && [ "$CODE" = 200 ]; then
  echo "pyr.gg has the new page (the one before is kept in $KEEP)."
else
  echo "The new page is in $WEB, but the website didn't serve it back (firelitgames.com/pyr/: $([ "$GOT" = "$WANT" ] && echo the new page || echo another page); pyr.gg: $CODE)." >&2
  exit 1
fi
