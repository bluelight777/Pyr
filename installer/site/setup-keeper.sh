#!/bin/bash
# Runs on the VPS (install-site.sh starts it, as root): sets up the Pyr Library Keeper, which
# lists new versions of what's already in the Pyr Library by itself once they pass its checks,
# a sandboxed build and Claude's review (see crates/keeper).
#
# What it sets up:
#   /usr/local/bin/pyr-keeper            the program
#   /etc/pyr-keeper/                     its key, the release key's certificate for it, the
#                                        Claude API key (asked for the first time), keeper.json
#   /opt/pyr-keeper/                     the Rust toolchain for WebAssembly, the plugin kit and
#                                        its crates (vendored: builds have no network)
#   /var/lib/pyr-keeper/                 its state, log (log.jsonl) and build folder
#   pyr-keeper.service                   as the guild server's own user (it reads and updates
#                                        that server's submissions), with the library folder
#                                        the only part of the website it may write
#
# Run again any time: what's there is kept (the key, the API key, the state).
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
say() { echo; echo "== $*"; }
note() { echo "   $*"; }

[ "$(id -u)" = 0 ] || { echo "Run as root." >&2; exit 1; }

# The guild server: whose submissions these are.
SVC=""
for S in pyr-server ingle-server; do
  if systemctl list-unit-files "$S.service" 2>/dev/null | grep -q "^$S\.service"; then SVC=$S; break; fi
done
[ -n "$SVC" ] || { echo "There's no guild server here (pyr-server.service) to keep the Library for." >&2; exit 1; }
DATA=$(systemctl show -p ExecStart "$SVC" 2>/dev/null | sed -n 's/.*--data \([^ ;]*\).*/\1/p' | head -n 1)
DATA=${DATA:-/var/lib/$SVC}
RUN_AS=$(systemctl show -p User --value "$SVC" 2>/dev/null || true)
[ -n "$RUN_AS" ] && [ "$RUN_AS" != root ] || { echo "The guild server runs as root; the Keeper won't (give the server a user of its own first)." >&2; exit 1; }
DB="$DATA/ingle.db"
[ -f "$DB" ] || DB="$DATA/pyr.db"
[ -f "$DB" ] || { echo "Can't find the guild server's database in $DATA." >&2; exit 1; }
note "guild server: $SVC as $RUN_AS, database $DB"

# The website's library folder (as install-library.sh finds it).
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
LIB="$ROOT/ingle/library"
mkdir -p "$LIB/files"
note "library folder: $LIB"

say "The sandbox (bubblewrap) and the Rust toolchain for WebAssembly"
command -v bwrap >/dev/null || { apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq bubblewrap >/dev/null; }
# A C linker: build scripts and macros (serde's) are built for this machine, not WebAssembly.
command -v cc >/dev/null || { apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq gcc >/dev/null; }
# Ubuntu 24.04 (kernel.apparmor_restrict_unprivileged_userns = 1) lets a program other than
# root's make a sandbox (a user namespace) only with an AppArmor profile that allows it, and
# bubblewrap's package doesn't bring one: without it the sandbox can't even set up its own
# network ("loopback: Failed RTM_NEWADDR"). This is the profile Ubuntu documents for it.
if [ "$(sysctl -n kernel.apparmor_restrict_unprivileged_userns 2>/dev/null || echo 0)" = 1 ] && [ ! -f /etc/apparmor.d/bwrap ]; then
  cat > /etc/apparmor.d/bwrap <<'PROFILE'
abi <abi/4.0>,
include <tunables/global>

profile bwrap /usr/bin/bwrap flags=(unconfined) {
  userns,

  include if exists <local/bwrap>
}
PROFILE
  apparmor_parser -r /etc/apparmor.d/bwrap
fi
# (As the user the Keeper runs as: root may make one where it can't.)
runuser -u "$RUN_AS" -- bwrap --unshare-all --die-with-parent --ro-bind / / true || { echo "bubblewrap can't make a sandbox here." >&2; exit 1; }
OPT=/opt/pyr-keeper
mkdir -p "$OPT"
export RUSTUP_HOME=$OPT/rustup CARGO_HOME=$OPT/cargo
if [ ! -x "$OPT/cargo/bin/cargo" ]; then
  curl -sSf --proto '=https' --tlsv1.2 https://sh.rustup.rs | sh -s -- -y -q --no-modify-path --profile minimal --default-toolchain stable --target wasm32-unknown-unknown
fi
"$OPT/cargo/bin/rustup" update stable -q >/dev/null 2>&1 || true
"$OPT/cargo/bin/rustup" target add wasm32-unknown-unknown >/dev/null
note "$("$OPT/cargo/bin/rustc" --version)"

say "The plugin kit and its crates"
rm -rf "$OPT/kit" && cp -r "$HERE/keeper/kit" "$OPT/kit"
# A little workspace using what add-ons may use, to vendor exactly that.
VW=$(mktemp -d)
mkdir -p "$VW/src"
cat > "$VW/Cargo.toml" <<EOF
[package]
name = "vendor-these"
version = "0.0.0"
edition = "2024"
[dependencies]
pyr-plugin = { path = "$OPT/kit" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
EOF
touch "$VW/src/lib.rs"
(cd "$VW" && "$OPT/cargo/bin/cargo" vendor -q "$OPT/vendor.new" >/dev/null)
rm -rf "$OPT/vendor" && mv "$OPT/vendor.new" "$OPT/vendor"
rm -rf "$VW"
chmod -R a+rX "$OPT"
note "vendored: $(ls "$OPT/vendor" | wc -l) crates"

say "The Keeper"
case "$(uname -m)" in
  x86_64) install -m 755 "$HERE/keeper/pyr-keeper-linux-x86_64" /usr/local/bin/pyr-keeper ;;
  aarch64) install -m 755 "$HERE/keeper/pyr-keeper-linux-aarch64" /usr/local/bin/pyr-keeper ;;
  *) echo "No pyr-keeper for $(uname -m)." >&2; exit 1 ;;
esac
ETC=/etc/pyr-keeper
mkdir -p "$ETC"
[ -f "$ETC/keeper-key.txt" ] || install -m 600 "$HERE/keeper/keeper-key.txt" "$ETC/keeper-key.txt"
# (Kept only there from now on.)
rm -f "$HERE/keeper/keeper-key.txt"
install -m 644 "$HERE/keeper/keeper-cert.json" "$ETC/keeper-cert.json"
if [ ! -s "$ETC/api-key" ]; then
  echo
  echo "The Keeper asks Claude (through the Claude API) to review each new version."
  echo "Paste a Claude API key (from console.anthropic.com), then press Enter."
  echo "(It doesn't show while you paste. It's kept in $ETC/api-key, readable only by the Keeper.)"
  read -rs KEY
  echo
  [ -n "$KEY" ] || { echo "No key: the Keeper isn't started. Run this again with one." >&2; exit 1; }
  umask 077
  printf '%s\n' "$KEY" > "$ETC/api-key"
  umask 022
fi
STATE=/var/lib/pyr-keeper
mkdir -p "$STATE/work"
cat > "$ETC/keeper.json" <<EOF
{
  "guild_db": "$DB",
  "web_library": "$LIB",
  "state_dir": "$STATE",
  "key_file": "$ETC/keeper-key.txt",
  "cert_file": "$ETC/keeper-cert.json",
  "release_key": "$(cat "$HERE/keeper/release-key.pub")",
  "api_key_file": "$ETC/api-key",
  "rustup_home": "$OPT/rustup",
  "cargo_home": "$OPT/cargo",
  "kit": "$OPT/kit",
  "vendor": "$OPT/vendor",
  "work": "$STATE/work",
  "poll_secs": 60,
  "valid_days": 30
}
EOF
chown -R "$RUN_AS": "$STATE" "$ETC"
chmod 700 "$ETC"
# The library folder is the one part of the website the Keeper may write. A folder on the way
# to it that only the web server may enter (Hostinger's /var/www/<site>, say) lets the Keeper
# through, and no more: passing, not listing (an ACL for its user alone).
D=$(dirname "$LIB")
while [ "$D" != / ]; do
  if ! runuser -u "$RUN_AS" -- test -x "$D"; then
    command -v setfacl >/dev/null || { apt-get update -qq && DEBIAN_FRONTEND=noninteractive apt-get install -y -qq acl >/dev/null; }
    setfacl -m "u:$RUN_AS:--x" "$D"
    note "let $RUN_AS through $D (only through)"
  fi
  D=$(dirname "$D")
done
chown -R "$RUN_AS" "$LIB"
chmod -R u+rwX,a+rX "$LIB"

say "A test build in the sandbox (the Dice add-on)"
rm -rf "$STATE/selftest" && cp -r "$HERE/keeper/dice" "$STATE/selftest" && chown -R "$RUN_AS": "$STATE/selftest"
runuser -u "$RUN_AS" -- /usr/local/bin/pyr-keeper selftest "$STATE/selftest" --config "$ETC/keeper.json"
rm -rf "$STATE/selftest"

say "The Library's list as the Keeper keeps it (index3.json, for Pyr 0.9 and newer)"
if [ ! -f "$STATE/state.json" ] || [ "${1:-}" = "--import" ]; then
  install -m 644 -o "$RUN_AS" "$HERE/keeper/index3-signed.json" "$STATE/import.json"
  runuser -u "$RUN_AS" -- /usr/local/bin/pyr-keeper import "$STATE/import.json" --config "$ETC/keeper.json"
  rm -f "$STATE/import.json"
fi

cat > /etc/systemd/system/pyr-keeper.service <<EOF
[Unit]
Description=Pyr Library Keeper (lists new versions of Library plugins and skins once they pass)
After=network-online.target $SVC.service
Wants=network-online.target

[Service]
User=$RUN_AS
ExecStart=/usr/local/bin/pyr-keeper run --config $ETC/keeper.json
Restart=on-failure
RestartSec=30
# Only what it needs: its own folders, the server's database folder, the library folder.
ProtectSystem=strict
ReadWritePaths=$STATE $DATA $LIB
ProtectHome=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectControlGroups=yes
LockPersonality=yes

[Install]
WantedBy=multi-user.target
EOF
systemctl daemon-reload
systemctl enable --now pyr-keeper.service >/dev/null
systemctl restart pyr-keeper.service
sleep 3
if systemctl is-active --quiet pyr-keeper.service; then
  note "the Keeper is running (journalctl -u pyr-keeper to see what it does)"
else
  journalctl -u pyr-keeper -n 30 --no-pager >&2
  echo "The Keeper didn't start (above)." >&2
  exit 1
fi
