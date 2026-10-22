#!/bin/bash
# Runs on the VPS (publish.ps1 starts it): puts the Pyr download folder (the page, the
# downloads, latest.json and the Library) into the web folder that serves
# https://firelitgames.com/, at /pyr/: https://pyr.gg serves that same folder (see
# setup-pyr-gg.sh). Copies from before Pyr's rename (0.7) look for their updates and the
# Library under the old name (OLD below): that path stays, a link to the same folder.
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
trap 'rm -rf "$HERE"' EXIT
SITE=firelitgames.com
# Pyr's name before 0.8.0 (see pyr_proto::legacy): only for finding and moving what has it.
OLD=ingle
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
# The folder had the old name until 0.9.1: it becomes pyr/, and the old name a link to it
# (copies from before the rename look there). Put back as it was if the website then can't
# be reached through the link (nginx set not to follow links).
if [ -d "$ROOT/$OLD" ] && [ ! -L "$ROOT/$OLD" ] && [ ! -e "$ROOT/pyr" ]; then
  mv "$ROOT/$OLD" "$ROOT/pyr" && ln -s pyr "$ROOT/$OLD"
  GOT=$(curl -sk --noproxy "*" --max-time 5 -o /dev/null -w "%{http_code}" --resolve "$SITE:443:127.0.0.1" "https://$SITE/$OLD/latest.json" || true)
  if [ "$GOT" = 200 ]; then
    echo "The website's folder is $ROOT/pyr now (the old name is a link to it)"
  else
    rm -f "$ROOT/$OLD" && mv "$ROOT/pyr" "$ROOT/$OLD"
    echo "The website's folder keeps its old name: nginx here doesn't follow links." >&2
  fi
fi
WEB=$ROOT/pyr
[ -d "$ROOT/$OLD" ] && [ ! -L "$ROOT/$OLD" ] && WEB=$ROOT/$OLD
mkdir -p "$WEB"
# Everything but latest.json first, so the app never sees a version whose files aren't there yet.
find "$HERE/pyr" -maxdepth 1 -type f ! -name latest.json -exec cp -f {} "$WEB/" \;
# The page's pictures and fonts (and any other folders), into folders of the same names.
find "$HERE/pyr" -mindepth 1 -maxdepth 1 -type d -exec cp -rf {} "$WEB/" \;
# The Pyr Library (library.tar, from make-release.sh): the plugins' files first, then the
# signed list that names them.
if [ -f "$HERE/library.tar" ]; then
  mkdir -p "$HERE/library-x" "$WEB/library"
  tar --no-same-owner --no-same-permissions -xf "$HERE/library.tar" -C "$HERE/library-x"
  if [ -d "$HERE/library-x/library/files" ]; then cp -rf "$HERE/library-x/library/files" "$WEB/library/"; fi
  if [ -f "$HERE/library-x/library/index.json" ]; then cp -f "$HERE/library-x/library/index.json" "$WEB/library/index.json"; fi
  # (Pyr 0.8.1 and newer read index2.json; index.json is older Pyrs' list.)
  if [ -f "$HERE/library-x/library/index2.json" ]; then cp -f "$HERE/library-x/library/index2.json" "$WEB/library/index2.json"; fi
fi
cp -f "$HERE/pyr/latest.json" "$WEB/latest.json"
chmod -R a+rX "$WEB"
echo "Published to $WEB"
# An earlier version of this script could put the files in another site's folder: remove
# those copies (only folders holding a release file).
for R in $(nginx -T 2>/dev/null | awk '/^[[:space:]]*root[[:space:]]/ { r = $2; gsub(";", "", r); print r }' | sort -u); do
  if [ "$R" != "$ROOT" ] && [ -f "$R/$OLD/latest.json" ] && grep -q '"manifest"' "$R/$OLD/latest.json"; then
    rm -rf "${R:?}/$OLD" && echo "Removed a stray copy from $R/$OLD"
  fi
done
# The guild server set up before the rename ($OLD-server: its service, program, user and data
# folder) becomes pyr-server, keeping everything else as it was (its port, its settings). The
# old pieces are set aside, not deleted, until the new one runs (MOVED says what moved).
MOVED=""
OLD_SVC=$OLD-server
if systemctl list-unit-files "$OLD_SVC.service" 2>/dev/null | grep -q "^$OLD_SVC\.service" \
   && ! systemctl list-unit-files pyr-server.service 2>/dev/null | grep -q "^pyr-server\.service"; then
  echo "Renaming the guild server's service to pyr-server"
  OLD_UNIT=$(systemctl show -p FragmentPath --value "$OLD_SVC")
  OLD_BIN=$(systemctl show -p ExecStart "$OLD_SVC" | sed -n 's/^ExecStart={ path=\([^ ;]*\).*/\1/p' | head -n 1)
  OLD_DATA=$(systemctl show -p ExecStart "$OLD_SVC" | sed -n 's/.*--data \([^ ;]*\).*/\1/p' | head -n 1)
  OLD_USER=$(systemctl show -p User --value "$OLD_SVC")
  NEW_BIN=${OLD_BIN//$OLD_SVC/pyr-server}
  NEW_DATA=${OLD_DATA//$OLD_SVC/pyr-server}
  # (The Keeper reads the server's database: it's set up again for the new one, below.)
  systemctl stop pyr-keeper >/dev/null 2>&1 || true
  if [ "$OLD_USER" = "$OLD_SVC" ] && ! id pyr-server >/dev/null 2>&1; then
    useradd --system --no-create-home --home-dir "${NEW_DATA:-/var/lib/pyr-server}" --shell /usr/sbin/nologin pyr-server
  fi
  systemctl stop "$OLD_SVC"
  if [ -n "$OLD_DATA" ] && [ "$NEW_DATA" != "$OLD_DATA" ] && [ ! -e "$NEW_DATA" ]; then
    mv "$OLD_DATA" "$NEW_DATA"
  fi
  [ "$OLD_USER" = "$OLD_SVC" ] && [ -n "$NEW_DATA" ] && chown -R "pyr-server:$(id -gn pyr-server)" "$NEW_DATA"
  [ -n "$OLD_BIN" ] && [ "$NEW_BIN" != "$OLD_BIN" ] && cp -p "$OLD_BIN" "$NEW_BIN"
  sed "s#$OLD_SVC#pyr-server#g" "$OLD_UNIT" > /etc/systemd/system/pyr-server.service
  if [ -d "$OLD_UNIT.d" ]; then
    mkdir -p /etc/systemd/system/pyr-server.service.d
    for F in "$OLD_UNIT.d"/*; do sed "s#$OLD_SVC#pyr-server#g" "$F" > "/etc/systemd/system/pyr-server.service.d/$(basename "$F")"; done
  fi
  systemctl disable "$OLD_SVC" >/dev/null 2>&1 || true
  mv -f "$OLD_UNIT" "$OLD_UNIT.before-pyr"
  [ -d "$OLD_UNIT.d" ] && mv -f "$OLD_UNIT.d" "$OLD_UNIT.d.before-pyr"
  systemctl daemon-reload
  systemctl enable pyr-server >/dev/null 2>&1
  # (Running again at once, as it was: the new program follows below.)
  systemctl start pyr-server || true
  MOVED=1
fi
# Puts the old service back as it was (the renamed one didn't run), the database too.
undo_rename() {
  echo "Putting the guild server's old service back." >&2
  systemctl stop pyr-server >/dev/null 2>&1 || true
  systemctl disable pyr-server >/dev/null 2>&1 || true
  rm -rf /etc/systemd/system/pyr-server.service /etc/systemd/system/pyr-server.service.d
  mv -f "$OLD_UNIT.before-pyr" "$OLD_UNIT"
  [ -d "$OLD_UNIT.d.before-pyr" ] && mv -f "$OLD_UNIT.d.before-pyr" "$OLD_UNIT.d"
  if [ "$NEW_DATA" != "$OLD_DATA" ] && [ -d "$NEW_DATA" ] && [ ! -e "$OLD_DATA" ]; then
    mv "$NEW_DATA" "$OLD_DATA"
  fi
  db_back "$OLD_DATA"
  [ "$OLD_USER" = "$OLD_SVC" ] && chown -R "$OLD_USER:$(id -gn "$OLD_USER")" "$OLD_DATA"
  [ "$NEW_BIN" != "$OLD_BIN" ] && rm -f "$NEW_BIN" "$NEW_BIN.previous"
  systemctl daemon-reload
  systemctl enable "$OLD_SVC" >/dev/null 2>&1 || true
  systemctl start "$OLD_SVC" || true
}
# The database's file before 0.9.1 had the old name: a server put back to a program from
# then needs it so named again (the new program renames it when it starts).
db_back() {
  local D=$1 X
  if [ -n "$D" ] && [ -f "$D/pyr.db" ] && [ ! -e "$D/$OLD.db" ]; then
    for X in "" -wal -shm; do [ -e "$D/pyr.db$X" ] && mv "$D/pyr.db$X" "$D/$OLD.db$X"; done
  fi
  return 0
}
# The guild server running on this VPS (if one is set up): the pyr-server service (or one
# from before the rename that couldn't be renamed). SVC is the service, BIN the program it
# starts.
SVC=""
for S in pyr-server "$OLD_SVC"; do
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
          db_back "$(systemctl show -p ExecStart "$SVC" | sed -n 's/.*--data \([^ ;]*\).*/\1/p' | head -n 1)"
          mv -f "$BIN.previous" "$BIN"
          systemctl restart "$SVC" || true
        fi
      fi
    else
      echo "Guild server already up to date ($OLD_V)"
    fi
  fi
fi
# The renamed service runs (or the old one is put back).
if [ -n "$MOVED" ]; then
  sleep 1
  if systemctl is-active --quiet pyr-server; then
    echo "The guild server is pyr-server now (same community, same links)"
  else
    undo_rename
    MOVED=""
    SVC=$OLD_SVC
    BIN=$OLD_BIN
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
# v2; browsers kept showing the old page and pictures for hours after 0.8.0), before invite
# pages' downloads carried the invite (v3), before the website's folder was pyr/ (v4), or
# before the invite directory could read the page (v5; every invite was a blank page): set up
# again
# (it checks every other site on this server before and after, and leaves the certificate).
if [ -f "$HERE/setup-pyr-gg.sh" ] && [ -f /etc/systemd/system/pyr-directory.service ] \
   && nginx -T 2>/dev/null | grep -q "server_name pyr.gg;" && ! nginx -T 2>/dev/null | grep -q "pyr.gg settings v5"; then
  echo "Updating pyr.gg's settings"
  bash "$HERE/setup-pyr-gg.sh" || echo "pyr.gg's settings stay as they were (above); the release itself is published." >&2
fi
# The Pyr Library Keeper (setup-keeper.sh): kept in /root/pyr-keeper-setup (only root can read
# it: it holds the Keeper's key until it's set up). Once its Claude API key is in, each release
# sets it up again (the new program; its key, list and state stay). Before that it's one
# command away (said below), since it asks for the key. It never holds up the release.
if [ -d "$HERE/keeper" ] && [ -f "$HERE/setup-keeper.sh" ]; then
  KS=/root/pyr-keeper-setup
  rm -rf "$KS" && mkdir -m 700 "$KS" && cp -r "$HERE/keeper" "$HERE/setup-keeper.sh" "$KS/"
  if [ -s /etc/pyr-keeper/api-key ]; then
    echo "Updating the Pyr Library Keeper"
    bash "$KS/setup-keeper.sh" || echo "The Library Keeper wasn't updated (above); the release itself is published." >&2
  else
    echo
    echo "The Pyr Library Keeper (new versions of Library plugins and skins go up once Claude has"
    echo "checked them) isn't on yet. To turn it on, run this (it asks for a Claude API key):"
    echo "  ssh -t root@2.24.121.231 bash /root/pyr-keeper-setup/setup-keeper.sh"
    echo
  fi
fi
# Firelit Poker is part of Pyr now (Pyr Casino, 0.12.0): off the Library, once. The Keeper's
# list (index3.json, what Pyr 0.9 and newer read) loses it here, the Keeper stopped meanwhile;
# the release's own lists (index2.json and index.json, for older Pyrs) already have.
KDONE=/var/lib/pyr-keeper/unlisted-firelit-poker
if [ -x /usr/local/bin/pyr-keeper ] && [ -f /etc/pyr-keeper/keeper.json ] && [ -f /var/lib/pyr-keeper/state.json ] && [ ! -f "$KDONE" ]; then
  KUSER=$(systemctl show -p User --value pyr-keeper 2>/dev/null || true)
  KWAS=$(systemctl is-active pyr-keeper 2>/dev/null || true)
  systemctl stop pyr-keeper >/dev/null 2>&1 || true
  if [ -n "$KUSER" ] && [ "$KUSER" != "root" ]; then
    KOUT=$(runuser -u "$KUSER" -- /usr/local/bin/pyr-keeper unlist firelit-poker --config /etc/pyr-keeper/keeper.json 2>&1) && KOK=1 || KOK=0
  else
    KOUT=$(/usr/local/bin/pyr-keeper unlist firelit-poker --config /etc/pyr-keeper/keeper.json 2>&1) && KOK=1 || KOK=0
  fi
  if [ "$KOK" = 1 ] || printf '%s' "$KOUT" | grep -q "nothing in the list is called"; then
    echo "Firelit Poker is off the Pyr Library (it's Pyr Casino, part of Pyr now)."
    touch "$KDONE"
  else
    echo "Couldn't take Firelit Poker off the Library (it'll be tried again with the next release):" >&2
    printf '%s\n' "$KOUT" | tail -n 3 >&2
  fi
  [ "$KWAS" = "active" ] && systemctl start pyr-keeper >/dev/null 2>&1 || true
fi
# What was set aside when the guild server's service was renamed goes, now that it runs (and
# the Keeper, set up again above, runs as the new user): the old service, program and user.
if [ -n "$MOVED" ] && systemctl is-active --quiet pyr-server; then
  rm -rf "$OLD_UNIT.before-pyr" "$OLD_UNIT.d.before-pyr"
  [ "$NEW_BIN" != "$OLD_BIN" ] && rm -f "$OLD_BIN" "$OLD_BIN.previous"
  if [ "$OLD_USER" = "$OLD_SVC" ] && id "$OLD_SVC" >/dev/null 2>&1; then
    # (Let through folders for the Keeper: those were for the old user.)
    if command -v setfacl >/dev/null; then
      P=""
      for PART in $(echo "$WEB" | tr '/' ' '); do
        P="$P/$PART"
        setfacl -x "u:$OLD_SVC" "$P" 2>/dev/null || true
      done
    fi
    pgrep -u "$OLD_SVC" >/dev/null 2>&1 || userdel "$OLD_SVC" 2>/dev/null || true
  fi
fi
# Bug reports this release fixed (fixed-bugs.txt: a report's number first on each line) are
# marked fixed on the guild server here, the one Pyr's bug reports go to, as the owner's
# Mark fixed does (only new and approved ones; a rejected report stays rejected).
if [ -f "$HERE/fixed-bugs.txt" ] && [ -n "$SVC" ] && [ -x "$BIN" ]; then
  IDS=$(grep -Eo '^[0-9]+' "$HERE/fixed-bugs.txt" | tr '\n' ' ' || true)
  DATA=$(systemctl show -p ExecStart "$SVC" 2>/dev/null | sed -n 's/.*--data \([^ ;]*\).*/\1/p' | head -n 1)
  DATA=${DATA:-/var/lib/$SVC}
  RUN_AS=$(systemctl show -p User --value "$SVC" 2>/dev/null || true)
  if [ -n "${IDS// /}" ] && { [ -f "$DATA/pyr.db" ] || [ -f "$DATA/$OLD.db" ]; }; then
    # As the server's own user, so the database's files keep their owner.
    if [ -n "$RUN_AS" ] && [ "$RUN_AS" != "root" ]; then
      runuser -u "$RUN_AS" -- "$BIN" --data "$DATA" mark-fixed $IDS || echo "Couldn't mark the fixed bug reports ($IDS)." >&2
    else
      "$BIN" --data "$DATA" mark-fixed $IDS || echo "Couldn't mark the fixed bug reports ($IDS)." >&2
    fi
  fi
fi
