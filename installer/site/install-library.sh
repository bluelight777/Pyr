#!/bin/bash
# Runs on the VPS (publish-library.ps1 starts it): new versions of the Library's programs between
# Pyr releases. Since the rebuild the Library Keeper keeps the lists Pyr reads (index4.json, and
# index3.json for 0.9 to 0.13.28), so this takes a Keeper's way in, as a release takes Pyr Prop
# Bets': library.tar has the programs' files (library/files/...), put in the web folder's
# library/files first, so a list never names a file that isn't there yet; keeper/add/ has their
# listings, signed with the release key and each program endorsed by it, which the Keeper takes
# into its lists (one it has listed already changes nothing; one taken off the Library stays off).
# The rest of the site, Pyr's downloads and the Keeper's settings stay as they are.
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
if [ ! -f "$WEB/latest2.json" ]; then
  echo "There's no rebuilt Pyr on https://$SITE yet: publish a release first." >&2
  exit 1
fi
KEEPER=/usr/local/bin/pyr-keeper
CONFIG=/etc/pyr-keeper/keeper.json
STATE=/var/lib/pyr-keeper
if [ ! -x "$KEEPER" ] || [ ! -f "$CONFIG" ]; then
  echo "The Library Keeper isn't set up on this server: publish a Pyr release first." >&2
  exit 1
fi
if [ ! -f "$HERE/library.tar" ]; then
  echo "library.tar is missing." >&2
  exit 1
fi
ADDS=$(ls "$HERE"/keeper/add/*.json 2>/dev/null || true)
if [ -z "$ADDS" ]; then
  echo "There's nothing to list (keeper/add/ is empty)." >&2
  exit 1
fi
mkdir -p "$HERE/library-x" "$WEB/library"
tar --no-same-owner --no-same-permissions -xf "$HERE/library.tar" -C "$HERE/library-x"
if [ ! -d "$HERE/library-x/library/files" ]; then
  echo "library.tar has no files." >&2
  exit 1
fi
cp -rf "$HERE/library-x/library/files" "$WEB/library/"
chmod -R a+rX "$WEB/library"
echo "The programs' files are in $WEB/library/files"
# The listings, into the Keeper (stopped meanwhile, so it can't put its list up over them), as
# the user it runs as.
KUSER=$(systemctl show -p User --value pyr-keeper 2>/dev/null || true)
systemctl stop pyr-keeper > /dev/null 2>&1 || true
FAILED=""
for A in $ADDS; do
  install -m 644 ${KUSER:+-o "$KUSER"} "$A" "$STATE/import.json"
  if [ -n "$KUSER" ]; then
    runuser -u "$KUSER" -- "$KEEPER" import "$STATE/import.json" --config "$CONFIG" && OK=1 || OK=""
  else
    "$KEEPER" import "$STATE/import.json" --config "$CONFIG" && OK=1 || OK=""
  fi
  rm -f "$STATE/import.json"
  if [ -n "$OK" ]; then
    echo "Listed: $(basename "$A" .json)"
  else
    echo "Couldn't list $(basename "$A" .json) (above)." >&2
    FAILED="$FAILED $(basename "$A" .json)"
  fi
done
systemctl start pyr-keeper > /dev/null 2>&1 || true
sleep 3
if ! systemctl is-active --quiet pyr-keeper; then
  journalctl -u pyr-keeper -n 30 --no-pager >&2 || true
  echo "The Library Keeper didn't start again (above)." >&2
  exit 1
fi
if [ -n "$FAILED" ]; then
  echo "Not listed:$FAILED" >&2
  exit 1
fi
echo "The Pyr Library is updated."
