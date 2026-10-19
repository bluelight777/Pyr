#!/bin/bash
# Runs on the VPS (publish.ps1 starts it): puts the Pyr download folder (the page, the
# downloads, latest.json and the Library) into the web folder that serves
# https://firelitgames.com/, at /ingle/: https://pyr.gg serves that same folder (see
# setup-pyr-gg.sh), and copies of Pyr from before it was called Pyr (Ingle 0.7) look for
# their updates and the Library at firelitgames.com/ingle/.
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
  echo "$TOKEN" > "$R/pyr-probe-$TOKEN.txt" 2>/dev/null || continue
  GOT=$(curl -sk --noproxy "*" --max-time 5 --resolve "$SITE:443:127.0.0.1" "https://$SITE/pyr-probe-$TOKEN.txt" || true)
  rm -f "$R/pyr-probe-$TOKEN.txt"
  if [ "$GOT" = "$TOKEN" ]; then ROOT=$R; break; fi
done
if [ -z "$ROOT" ]; then
  echo "Couldn't find the web folder that serves https://$SITE/." >&2
  exit 1
fi
mkdir -p "$ROOT/ingle"
# Everything but latest.json first, so the app never sees a version whose files aren't there yet.
find "$HERE/pyr" -maxdepth 1 -type f ! -name latest.json -exec cp -f {} "$ROOT/ingle/" \;
# The page's pictures and fonts (and any other folders), into folders of the same names.
find "$HERE/pyr" -mindepth 1 -maxdepth 1 -type d -exec cp -rf {} "$ROOT/ingle/" \;
# The Pyr Library (library.tar, from make-release.sh): the plugins' files first, then the
# signed list that names them.
if [ -f "$HERE/library.tar" ]; then
  mkdir -p "$HERE/library-x" "$ROOT/ingle/library"
  tar --no-same-owner --no-same-permissions -xf "$HERE/library.tar" -C "$HERE/library-x"
  if [ -d "$HERE/library-x/library/files" ]; then cp -rf "$HERE/library-x/library/files" "$ROOT/ingle/library/"; fi
  if [ -f "$HERE/library-x/library/index.json" ]; then cp -f "$HERE/library-x/library/index.json" "$ROOT/ingle/library/index.json"; fi
  # (Pyr 0.8.1 and newer read index2.json; index.json is older Pyrs' list.)
  if [ -f "$HERE/library-x/library/index2.json" ]; then cp -f "$HERE/library-x/library/index2.json" "$ROOT/ingle/library/index2.json"; fi
fi
cp -f "$HERE/pyr/latest.json" "$ROOT/ingle/latest.json"
chmod -R a+rX "$ROOT/ingle"
echo "Published to $ROOT/ingle"
# An earlier version of this script could put the files in another site's folder: remove
# those copies (only folders holding a release file).
for R in $(nginx -T 2>/dev/null | awk '/^[[:space:]]*root[[:space:]]/ { r = $2; gsub(";", "", r); print r }' | sort -u); do
  if [ "$R" != "$ROOT" ] && [ -f "$R/ingle/latest.json" ] && grep -q '"manifest"' "$R/ingle/latest.json"; then
    rm -rf "${R:?}/ingle" && echo "Removed a stray copy from $R/ingle"
  fi
done
# The guild server running on this VPS (if one is set up): the pyr-server service, or
# ingle-server for one set up before Pyr was called Pyr (it keeps its name: only its program
# is new). SVC is the service, BIN the program it starts.
SVC=""
for S in pyr-server ingle-server; do
  if systemctl list-unit-files "$S.service" 2>/dev/null | grep -q "^$S\.service"; then SVC=$S; break; fi
done
BIN=""
if [ -n "$SVC" ]; then
  BIN=$(systemctl show -p ExecStart "$SVC" 2>/dev/null | sed -n 's/^ExecStart={ path=\([^ ;]*\).*/\1/p' | head -n 1)
  BIN=${BIN:-/usr/local/bin/$SVC}
fi
# Update it to the same version. Everyone in it drops for a few seconds and reconnects by
# themselves.
if [ -n "$SVC" ] && [ -x "$BIN" ]; then
  case "$(uname -m)" in
    x86_64) NEW="$HERE/server/pyr-server-linux-x86_64" ;;
    aarch64) NEW="$HERE/server/pyr-server-linux-aarch64" ;;
    *) NEW="" ;;
  esac
  if [ -n "$NEW" ] && [ -f "$NEW" ]; then
    # Copied from Windows, it arrives without permission to run.
    chmod 0755 "$NEW"
    OLD_V=$("$BIN" --version 2>/dev/null || echo unknown)
    NEW_V=$("$NEW" --version 2>/dev/null || echo "")
    # Extra walls around the service (it already runs as its own user): no capabilities,
    # devices, kernel settings or new privileges. Added once; taken away again below if the
    # server won't start with them.
    DROPIN=/etc/systemd/system/$SVC.service.d/hardening.conf
    NEW_DROPIN=""
    PORT_NOW=$(systemctl show -p ExecStart "$SVC" 2>/dev/null | sed -n 's/.*--bind [^ ]*:\([0-9]*\).*/\1/p' | head -n 1)
    if [ ! -f "$DROPIN" ] && [ "$(systemctl show -p User --value "$SVC" 2>/dev/null)" = "$SVC" ] \
       && [ -n "$PORT_NOW" ] && [ "$PORT_NOW" -ge 1024 ]; then
      mkdir -p "$(dirname "$DROPIN")"
      cat > "$DROPIN" <<'HARDEN'
[Service]
UMask=0077
CapabilityBoundingSet=
AmbientCapabilities=
PrivateDevices=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
RestrictNamespaces=yes
RestrictSUIDSGID=yes
RestrictRealtime=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
SystemCallArchitectures=native
HARDEN
      NEW_DROPIN=1
      systemctl daemon-reload
    fi
    if [ -z "$NEW_V" ]; then
      echo "The new guild server program won't run on this VPS; left the old one alone." >&2
    elif ! cmp -s "$NEW" "$BIN" || [ -n "$NEW_DROPIN" ]; then
      cp -f "$BIN" "$BIN.previous"
      install -m 0755 "$NEW" "$BIN.new"
      mv -f "$BIN.new" "$BIN"
      systemctl restart "$SVC" || true
      sleep 2
      if systemctl is-active --quiet "$SVC"; then
        echo "Guild server updated: $OLD_V -> $NEW_V"
      else
        if [ -n "$NEW_DROPIN" ]; then
          echo "The guild server didn't start with the extra protections; trying without them." >&2
          rm -f "$DROPIN"
          systemctl daemon-reload
          systemctl restart "$SVC" || true
          sleep 2
        fi
        if systemctl is-active --quiet "$SVC"; then
          echo "Guild server updated: $OLD_V -> $NEW_V"
        else
          echo "The new guild server didn't start; putting the old one back." >&2
          mv -f "$BIN.previous" "$BIN"
          systemctl restart "$SVC" || true
        fi
      fi
    else
      echo "Guild server already up to date ($OLD_V)"
    fi
  fi
fi
# Short links (https://pyr.gg/<code>): the invite directory, pyr-directory, a service of its
# own. The first time, setup-pyr-gg.sh sets it up along with pyr.gg's nginx settings for it
# (checking every other site on this server before and after); after that each release
# brings a new version, swapped in here (and the old one put back if the new one won't start).
case "$(uname -m)" in
  x86_64) DIR_NEW="$HERE/server/pyr-directory-linux-x86_64" ;;
  aarch64) DIR_NEW="$HERE/server/pyr-directory-linux-aarch64" ;;
  *) DIR_NEW="" ;;
esac
dir_answers() {
  local i
  for i in 1 2 3 4 5 6 7 8 9 10; do
    if [ "$(curl -s --noproxy "*" --max-time 3 http://127.0.0.1:7781/api/v1/health 2>/dev/null || true)" = ok ]; then return 0; fi
    sleep 1
  done
  return 1
}
if [ -n "$DIR_NEW" ] && [ -f "$DIR_NEW" ]; then
  chmod 0755 "$DIR_NEW"
  if [ -f /etc/systemd/system/pyr-directory.service ] && [ -x /usr/local/bin/pyr-directory ]; then
    if cmp -s "$DIR_NEW" /usr/local/bin/pyr-directory; then
      echo "Invite directory already up to date"
    elif ! "$DIR_NEW" --version > /dev/null 2>&1; then
      echo "The new invite directory program won't run on this VPS; left the old one alone." >&2
    else
      cp -f /usr/local/bin/pyr-directory /usr/local/bin/pyr-directory.previous
      install -m 0755 "$DIR_NEW" /usr/local/bin/pyr-directory.new
      mv -f /usr/local/bin/pyr-directory.new /usr/local/bin/pyr-directory
      systemctl restart pyr-directory || true
      if dir_answers; then
        echo "Invite directory updated: $(/usr/local/bin/pyr-directory --version)"
      else
        echo "The new invite directory didn't start; putting the old one back." >&2
        mv -f /usr/local/bin/pyr-directory.previous /usr/local/bin/pyr-directory
        systemctl restart pyr-directory || true
      fi
    fi
  elif [ -f "$HERE/setup-pyr-gg.sh" ]; then
    echo "Turning on short links (https://pyr.gg/<code>): setting up pyr.gg's invite directory"
    bash "$HERE/setup-pyr-gg.sh" --directory="$DIR_NEW" \
      || echo "Short links aren't on yet (above). The release itself is published; the next publish tries again." >&2
  fi
fi
# pyr.gg set up before its settings said "check for a newer page every time" (pyr.gg settings
# v2; browsers kept showing Ingle's old page and pictures for hours after 0.8.0), or before
# invite pages' downloads carried the invite (v3): set up again
# (it checks every other site on this server before and after, and leaves the certificate).
if [ -f "$HERE/setup-pyr-gg.sh" ] && [ -f /etc/systemd/system/pyr-directory.service ] \
   && nginx -T 2>/dev/null | grep -q "server_name pyr.gg;" && ! nginx -T 2>/dev/null | grep -q "pyr.gg settings v3"; then
  echo "Updating pyr.gg's settings (invite pages' downloads carry the invite)"
  bash "$HERE/setup-pyr-gg.sh" || echo "pyr.gg's settings stay as they were (above); the release itself is published." >&2
fi
# Bug reports this release fixed (fixed-bugs.txt: a report's number first on each line) are
# marked fixed on the guild server here, the one Pyr's bug reports go to, as the owner's
# Mark fixed does (only new and approved ones; a rejected report stays rejected).
if [ -f "$HERE/fixed-bugs.txt" ] && [ -n "$SVC" ] && [ -x "$BIN" ]; then
  IDS=$(grep -Eo '^[0-9]+' "$HERE/fixed-bugs.txt" | tr '\n' ' ' || true)
  DATA=$(systemctl show -p ExecStart "$SVC" 2>/dev/null | sed -n 's/.*--data \([^ ;]*\).*/\1/p' | head -n 1)
  DATA=${DATA:-/var/lib/$SVC}
  RUN_AS=$(systemctl show -p User --value "$SVC" 2>/dev/null || true)
  if [ -n "${IDS// /}" ] && [ -f "$DATA/ingle.db" ]; then
    # As the server's own user, so the database's files keep their owner.
    if [ -n "$RUN_AS" ] && [ "$RUN_AS" != "root" ]; then
      runuser -u "$RUN_AS" -- "$BIN" --data "$DATA" mark-fixed $IDS || echo "Couldn't mark the fixed bug reports ($IDS)." >&2
    else
      "$BIN" --data "$DATA" mark-fixed $IDS || echo "Couldn't mark the fixed bug reports ($IDS)." >&2
    fi
  fi
fi
