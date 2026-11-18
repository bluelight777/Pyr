#!/bin/bash
# Runs on the VPS (install-site.sh starts it, as root): sets up the props feed's maker,
# pyr-props (docs/PROPS.md, crates/props-feed). It makes the prop bets that Pyr servers'
# casinos open by themselves: it reads public sources (scoreboards, prices, the weather,
# earthquakes), and writes one signed file, https://pyr.gg/props/feed.json, with status.json
# beside it (how each source is doing). Servers whose owners turned the casino's own props
# on read that file; nothing else talks to it.
#
# What it sets up:
#   /usr/local/bin/pyr-props        the program
#   /etc/pyr-props/                 its key (feed-key.txt: it signs the feed and nothing else;
#                                   only its user can read it) and the release key's
#                                   certificate for that key (feed-cert.json, new each release)
#   /etc/pyr-props/keys/            the keys of the sites the games come from, a file each
#                                   (balldontlie, collegefootballdata, the-odds-api): put
#                                   there by pyr-props-key, never by a release
#   /usr/local/bin/pyr-props-key    asks for one of those keys and keeps it there
#   /var/lib/pyr-props/             what it has opened (state.json): a restart loses nothing
#   <the website>/pyr/props/        feed.json and status.json: the one part of the website it
#                                   may write
#   pyr-props.service               as a user of its own (pyr-props), shut in: it reads the
#                                   network and writes those two folders
#
# Run again any time: what's there is kept (the key, the state).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
say() { echo; echo "== $*"; }
note() { echo "   $*"; }

[ "$(id -u)" = 0 ] || { echo "Run as root." >&2; exit 1; }
[ -d "$HERE/props" ] || { echo "No props folder beside this script." >&2; exit 1; }
# Pyr's name before 0.8.0: a website folder set up before the rename may still have it.
OLD=ingle

# The website's folder (as install-site.sh finds it): the one that answers for the site.
SITE=firelitgames.com
ROOT=""
TOKEN=$(head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n')
for R in $(nginx -T 2>/dev/null | awk '/^[[:space:]]*root[[:space:]]/ { r = $2; gsub(";", "", r); print r }' | sort -u); do
  [ -d "$R" ] || continue
  echo "$TOKEN" > "$R/pyr-probe-$TOKEN.txt" 2>/dev/null || continue
  GOT=$(curl -sk --noproxy "*" --max-time 5 --resolve "$SITE:443:127.0.0.1" "https://$SITE/pyr-probe-$TOKEN.txt" || true)
  rm -f "$R/pyr-probe-$TOKEN.txt"
  if [ "$GOT" = "$TOKEN" ]; then ROOT=$R; break; fi
done
[ -n "$ROOT" ] || { echo "Couldn't find the web folder that serves https://$SITE/." >&2; exit 1; }
WEB="$ROOT/pyr"
[ -d "$WEB" ] || WEB="$ROOT/$OLD"
[ -d "$WEB" ] || { echo "The website has no Pyr folder yet ($ROOT/pyr)." >&2; exit 1; }
OUT="$WEB/props"
note "the feed's folder: $OUT (https://pyr.gg/props/)"

say "The program"
case "$(uname -m)" in
  x86_64) NEW="$HERE/props/pyr-props-linux-x86_64" ;;
  aarch64) NEW="$HERE/props/pyr-props-linux-aarch64" ;;
  *) echo "No pyr-props for $(uname -m)." >&2; exit 1 ;;
esac
[ -f "$NEW" ] || { echo "No $NEW in this release." >&2; exit 1; }
chmod 755 "$NEW"
"$NEW" --version >/dev/null || { echo "The new pyr-props doesn't run here." >&2; exit 1; }
note "$("$NEW" --version)"

say "Its user, its key and the certificate for it"
RUN_AS=pyr-props
id "$RUN_AS" >/dev/null 2>&1 || useradd --system --no-create-home --home-dir /var/lib/pyr-props --shell /usr/sbin/nologin "$RUN_AS"
ETC=/etc/pyr-props
STATE=/var/lib/pyr-props
KEYS="$ETC/keys"
mkdir -p "$ETC" "$KEYS" "$STATE" "$OUT"
if [ ! -f "$ETC/feed-key.txt" ]; then
  [ -f "$HERE/props/feed-key.txt" ] || { echo "pyr-props has no key here yet, and none came with this release (make-props-feed.sh with PROPS_KEY)." >&2; exit 1; }
  install -m 600 "$HERE/props/feed-key.txt" "$ETC/feed-key.txt"
  note "its key is in $ETC/feed-key.txt"
fi
# (Kept only there from now on.)
rm -f "$HERE/props/feed-key.txt"
install -m 644 "$HERE/props/feed-cert.json" "$ETC/feed-cert.json.new"
chown -R "$RUN_AS": "$ETC" "$STATE"
chmod 700 "$ETC" "$KEYS"
# (The games' sites' keys: Blue's own, typed in on this server. This puts each where it goes.)
if [ -f "$HERE/props/pyr-props-key" ]; then
  install -m 755 "$HERE/props/pyr-props-key" /usr/local/bin/pyr-props-key
fi
# The certificate has to be for the key that's here, from the release key Pyr servers believe.
if ! "$NEW" verify --key "$ETC/feed-key.txt" --cert "$ETC/feed-cert.json.new"; then
  rm -f "$ETC/feed-cert.json.new"
  echo "This release's certificate isn't for the feed key on this server: the feed's maker stays as it was." >&2
  exit 1
fi
mv -f "$ETC/feed-cert.json.new" "$ETC/feed-cert.json"

# The feed's folder is the one part of the website it may write. A folder on the way to it
# that only the web server may enter lets it through, and no more (an ACL for its user alone;
# from the top down: a closed folder makes every one under it look closed too).
P=""
for PART in $(dirname "$OUT" | tr '/' ' '); do
  P="$P/$PART"
  if ! runuser -u "$RUN_AS" -- test -x "$P"; then
    command -v setfacl >/dev/null || { apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq acl >/dev/null; }
    setfacl -m "u:$RUN_AS:--x" "$P"
    note "let $RUN_AS through $P (only through)"
  fi
done
chown -R "$RUN_AS" "$OUT"
chmod -R u+rwX,a+rX "$OUT"

say "The service"
# (Home folders are hidden from it, unless the website itself is in one.)
case "$OUT" in /home/*|/root/*) HOMES=no ;; *) HOMES=yes ;; esac
WAS=$(systemctl is-active pyr-props.service 2>/dev/null || true)
systemctl stop pyr-props.service >/dev/null 2>&1 || true
[ -f /usr/local/bin/pyr-props ] && cp -f /usr/local/bin/pyr-props /usr/local/bin/pyr-props.previous
install -m 755 "$NEW" /usr/local/bin/pyr-props
cat > /etc/systemd/system/pyr-props.service <<EOF
[Unit]
Description=Pyr props feed (makes https://pyr.gg/props/feed.json)
After=network-online.target
Wants=network-online.target

[Service]
User=$RUN_AS
ExecStart=/usr/local/bin/pyr-props run --state $STATE --out $OUT --key $ETC/feed-key.txt --cert $ETC/feed-cert.json --keys $KEYS
Restart=always
RestartSec=20
# Shut in: it reads the network and writes its state and the feed's folder, nothing else.
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=$HOMES
PrivateTmp=yes
PrivateDevices=yes
ProtectKernelTunables=yes
ProtectControlGroups=yes
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX AF_NETLINK
ReadWritePaths=$STATE $OUT
MemoryMax=300M

[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl enable pyr-props.service >/dev/null 2>&1
systemctl start pyr-props.service
sleep 3
if ! systemctl is-active --quiet pyr-props.service; then
  echo "pyr-props didn't stay up:" >&2
  journalctl -u pyr-props.service -n 20 --no-pager >&2 || true
  if [ -f /usr/local/bin/pyr-props.previous ] && [ "$WAS" = active ]; then
    echo "Putting the one before back." >&2
    mv -f /usr/local/bin/pyr-props.previous /usr/local/bin/pyr-props
    systemctl restart pyr-props.service || true
  fi
  exit 1
fi
note "pyr-props.service is running as $RUN_AS"

say "The feed"
# The first time, it asks every source before it writes (a few minutes: it's gentle with them,
# a request a second to each). After that the file's there from before.
for _ in $(seq 1 60); do
  [ -f "$OUT/feed.json" ] && [ -f "$OUT/status.json" ] && break
  sleep 5
done
if [ -f "$OUT/status.json" ]; then
  /usr/local/bin/pyr-props status "$OUT/status.json" || note "(sources that aren't reading are tried again by themselves; this page says how they're doing: https://pyr.gg/props/status.json)"
  GOT=$(curl -sk --noproxy "*" --max-time 8 -o /dev/null -w "%{http_code}" --resolve "pyr.gg:443:127.0.0.1" "https://pyr.gg/props/feed.json" || true)
  if [ "$GOT" = 200 ]; then
    note "https://pyr.gg/props/feed.json answers"
  else
    note "https://pyr.gg/props/feed.json answered $GOT from here: look at pyr.gg's nginx settings"
  fi
else
  note "No feed yet after five minutes. See: journalctl -u pyr-props -n 50"
fi

# The games come from three sites, each with a free key of Blue's own: a league whose site
# has no key yet opens nothing (and counts above as not reading) until it's given one.
WAITING=""
for SITE in "balldontlie:the NFL, the NBA and MLB" "collegefootballdata:college football and college basketball" "the-odds-api:the NHL"; do
  [ -s "$KEYS/${SITE%%:*}" ] || WAITING="$WAITING
   ${SITE#*:}: ssh -t root@2.24.121.231 pyr-props-key ${SITE%%:*}"
done
if [ -n "$WAITING" ]; then
  say "The games wait for their sites' keys"
  note "Each of these asks for a key and keeps it here (docs/PROPS.md says where each key comes from):$WAITING"
  note "Everything else in the feed needs no key."
fi
