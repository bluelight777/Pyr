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
# Update the Ingle guild server running on this VPS (if one is set up) to the same version.
# Everyone in it drops for a few seconds and reconnects by themselves.
if [ -x /usr/local/bin/ingle-server ] && systemctl list-unit-files ingle-server.service >/dev/null 2>&1; then
  case "$(uname -m)" in
    x86_64) NEW="$HERE/server/ingle-server-linux-x86_64" ;;
    aarch64) NEW="$HERE/server/ingle-server-linux-aarch64" ;;
    *) NEW="" ;;
  esac
  if [ -n "$NEW" ] && [ -f "$NEW" ]; then
    # Copied from Windows, it arrives without permission to run.
    chmod 0755 "$NEW"
    OLD_V=$(/usr/local/bin/ingle-server --version 2>/dev/null || echo unknown)
    NEW_V=$("$NEW" --version 2>/dev/null || echo "")
    # Extra walls around the service (it already runs as its own user): no capabilities,
    # devices, kernel settings or new privileges. Added once; taken away again below if the
    # server won't start with them.
    DROPIN=/etc/systemd/system/ingle-server.service.d/hardening.conf
    NEW_DROPIN=""
    PORT_NOW=$(systemctl show -p ExecStart ingle-server 2>/dev/null | sed -n 's/.*--bind [^ ]*:\([0-9]*\).*/\1/p' | head -n 1)
    if [ ! -f "$DROPIN" ] && [ "$(systemctl show -p User --value ingle-server 2>/dev/null)" = "ingle-server" ] \
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
    elif ! cmp -s "$NEW" /usr/local/bin/ingle-server || [ -n "$NEW_DROPIN" ]; then
      cp -f /usr/local/bin/ingle-server /usr/local/bin/ingle-server.previous
      install -m 0755 "$NEW" /usr/local/bin/ingle-server.new
      mv -f /usr/local/bin/ingle-server.new /usr/local/bin/ingle-server
      systemctl restart ingle-server
      sleep 2
      if systemctl is-active --quiet ingle-server; then
        echo "Guild server updated: $OLD_V -> $NEW_V"
      else
        if [ -n "$NEW_DROPIN" ]; then
          echo "The guild server didn't start with the extra protections; trying without them." >&2
          rm -f "$DROPIN"
          systemctl daemon-reload
          systemctl restart ingle-server
          sleep 2
        fi
        if systemctl is-active --quiet ingle-server; then
          echo "Guild server updated: $OLD_V -> $NEW_V"
        else
          echo "The new guild server didn't start; putting the old one back." >&2
          mv -f /usr/local/bin/ingle-server.previous /usr/local/bin/ingle-server
          systemctl restart ingle-server
        fi
      fi
    else
      echo "Guild server already up to date ($OLD_V)"
    fi
  fi
fi
