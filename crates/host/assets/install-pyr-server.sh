#!/bin/sh
# Pyr server setup for Linux: any distribution with systemd, on x86-64 or ARM64.
#
# Installs pyr-server as a service that starts by itself (also after a reboot), lets its
# UDP port through the server's firewall, and prints your owner link (the first person to
# open it becomes the owner) and an invite link for friends.
#
#   sudo sh install-pyr-server.sh --binary ./pyr-server-linux-x86_64
#
# Options:
#   --binary FILE        the pyr-server program for this machine (required)
#   --public-host HOST   IP address or domain name friends connect to (default: detected)
#   --name NAME          name for a new community (default: My Pyr Server)
#   --port PORT          UDP port to use (default: 7766)
#   --restore FILE       put back a server from a backup (a .tar of pyr.db, files/ and its
#                        identity, as Pyr's "Restore from a backup" makes). Whatever is
#                        already here is kept in /var/lib/pyr-server-before-restore-<time>.
#
# Running it again updates the program and keeps the community and its messages, which
# live in /var/lib/pyr-server. A server set up before Pyr's rename (its service had the old
# name, OLD below) becomes this one, with its community, messages, files and identity.
set -eu
# The server's service name before Pyr's rename (see pyr_proto::legacy), and its database's.
OLD=ingle-server
OLD_DB=ingle.db

step() { echo "==> $*"; }
warn() { echo "WARNING: $*"; }
die() { echo "ERROR: $*"; exit 1; }

if [ "$(id -u)" != 0 ]; then
  if command -v sudo >/dev/null 2>&1; then
    exec sudo sh "$0" "$@"
  fi
  die "Run this as root (or with sudo)."
fi

BIN=""
HOST=""
NAME=""
PORT=7766
PORT_GIVEN=""
RESTORE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --binary) BIN=${2:-}; shift 2 ;;
    --public-host) HOST=${2:-}; shift 2 ;;
    --name) NAME=${2:-}; shift 2 ;;
    --port) PORT=${2:-}; PORT_GIVEN=1; shift 2 ;;
    --restore) RESTORE=${2:-}; shift 2 ;;
    -h | --help) sed -n '2,20p' "$0"; exit 0 ;;
    *) die "Unknown option: $1 (see --help)" ;;
  esac
done
[ -n "$BIN" ] || die "Use --binary to say where the pyr-server program is."
[ -f "$BIN" ] || die "$BIN doesn't exist."
case "$PORT" in
  '' | *[!0-9]*) die "--port must be a number." ;;
esac

ARCH=$(uname -m)
case "$ARCH" in
  x86_64 | amd64 | aarch64 | arm64) ;;
  *) die "This server's processor ($ARCH) isn't supported yet: Pyr needs a 64-bit x86 or ARM server." ;;
esac
[ -d /run/systemd/system ] || die "This server doesn't run systemd, which the setup needs. Ubuntu, Debian, Fedora, Rocky, Alma and most VPS images do."

# A server set up before the rename (the $OLD service, its data in /var/lib/$OLD) is moved
# over, on the port it had; if Pyr's then won't start, everything is put back the way it was.
MIGRATED=""
if [ -f /etc/systemd/system/$OLD.service ] || [ -e /usr/local/bin/$OLD ]; then
  step "Moving the server from before the rename over to Pyr"
  OLD_PORT=$(sed -n 's/.*--bind [^ ]*:\([0-9][0-9]*\).*/\1/p' /etc/systemd/system/$OLD.service 2>/dev/null | head -n 1)
  if [ -z "$PORT_GIVEN" ] && [ -n "$OLD_PORT" ]; then
    PORT=$OLD_PORT
  fi
  systemctl stop $OLD >/dev/null 2>&1 || true
  systemctl disable $OLD >/dev/null 2>&1 || true
  if [ -d /var/lib/$OLD ] && [ ! -e /var/lib/pyr-server ]; then
    mv /var/lib/$OLD /var/lib/pyr-server
    MIGRATED="data"
    echo "    its community, messages and files are in /var/lib/pyr-server now"
  else
    MIGRATED="service"
  fi
  # Set aside (not deleted) until Pyr's server is running.
  [ -f /etc/systemd/system/$OLD.service ] && mv -f /etc/systemd/system/$OLD.service /etc/systemd/system/$OLD.service.before-pyr
  [ -d /etc/systemd/system/$OLD.service.d ] && mv -f /etc/systemd/system/$OLD.service.d /etc/systemd/system/$OLD.service.d.before-pyr
  [ -e /usr/local/bin/$OLD ] && mv -f /usr/local/bin/$OLD /usr/local/bin/$OLD.before-pyr
  systemctl daemon-reload
fi
# Run again on a server that's already Pyr's: it keeps the port it has (unless --port says).
if [ -z "$PORT_GIVEN" ] && [ -f /etc/systemd/system/pyr-server.service ]; then
  CUR_PORT=$(sed -n 's/.*--bind [^ ]*:\([0-9][0-9]*\).*/\1/p' /etc/systemd/system/pyr-server.service | head -n 1)
  if [ -n "$CUR_PORT" ]; then
    PORT=$CUR_PORT
  fi
fi
# Puts the server from before the rename back (Pyr's didn't start).
undo_migration() {
  [ -n "$MIGRATED" ] || return 0
  warn "Putting the server from before the rename back the way it was."
  systemctl stop pyr-server >/dev/null 2>&1 || true
  systemctl disable pyr-server >/dev/null 2>&1 || true
  rm -f /etc/systemd/system/pyr-server.service /usr/local/bin/pyr-server
  if [ "$MIGRATED" = data ] && [ ! -e /var/lib/$OLD ]; then
    # (Its database under the name that program knows: Pyr renames it when it starts.)
    for X in "" -wal -shm; do
      [ -e "/var/lib/pyr-server/pyr.db$X" ] && [ ! -e "/var/lib/pyr-server/$OLD_DB$X" ] && mv "/var/lib/pyr-server/pyr.db$X" "/var/lib/pyr-server/$OLD_DB$X"
    done
    mv /var/lib/pyr-server /var/lib/$OLD
    if id $OLD >/dev/null 2>&1; then
      chown -R "$OLD:$(id -gn $OLD)" /var/lib/$OLD
    fi
  fi
  [ -f /etc/systemd/system/$OLD.service.before-pyr ] && mv -f /etc/systemd/system/$OLD.service.before-pyr /etc/systemd/system/$OLD.service
  [ -d /etc/systemd/system/$OLD.service.d.before-pyr ] && mv -f /etc/systemd/system/$OLD.service.d.before-pyr /etc/systemd/system/$OLD.service.d
  [ -e /usr/local/bin/$OLD.before-pyr ] && mv -f /usr/local/bin/$OLD.before-pyr /usr/local/bin/$OLD
  systemctl daemon-reload
  systemctl enable $OLD >/dev/null 2>&1 || true
  systemctl start $OLD >/dev/null 2>&1 || true
}

step "Installing the Pyr server program"
systemctl stop pyr-server >/dev/null 2>&1 || true
install -m 0755 "$BIN" /usr/local/bin/pyr-server
VERSION=$(/usr/local/bin/pyr-server --version 2>&1) || {
  undo_migration
  die "The pyr-server program doesn't run on this machine ($ARCH): $VERSION"
}
echo "    $VERSION"

if ! id pyr-server >/dev/null 2>&1; then
  NOLOGIN=$(command -v nologin || echo /bin/false)
  if command -v useradd >/dev/null 2>&1; then
    if getent group pyr-server >/dev/null 2>&1; then
      useradd --system --gid pyr-server --home-dir /var/lib/pyr-server --no-create-home --shell "$NOLOGIN" pyr-server
    else
      useradd --system --user-group --home-dir /var/lib/pyr-server --no-create-home --shell "$NOLOGIN" pyr-server
    fi
  elif command -v adduser >/dev/null 2>&1; then
    adduser -S -D -H -h /var/lib/pyr-server -s "$NOLOGIN" pyr-server
  else
    die "Couldn't create the pyr-server user: this system has no useradd or adduser."
  fi
fi
GROUP=$(id -gn pyr-server)
mkdir -p /var/lib/pyr-server
if [ -n "$RESTORE" ]; then
  step "Putting the server back from its backup"
  [ -f "$RESTORE" ] || die "$RESTORE doesn't exist."
  if [ -n "$(ls -A /var/lib/pyr-server 2>/dev/null)" ]; then
    # Never lose what's here: keep a copy first.
    KEPT=/var/lib/pyr-server-before-restore-$(date +%Y%m%d-%H%M%S)
    cp -a /var/lib/pyr-server "$KEPT" || die "Couldn't keep a copy of the server's current data."
    chmod 700 "$KEPT"
    echo "    what was here before is kept in $KEPT"
  fi
  for X in "" -wal -shm; do rm -f "/var/lib/pyr-server/pyr.db$X" "/var/lib/pyr-server/$OLD_DB$X"; done
  tar -x -f "$RESTORE" -C /var/lib/pyr-server || die "Couldn't unpack the backup."
  # (A backup made before 0.9.1 has the database under its old name: the server renames it.)
  [ -f /var/lib/pyr-server/pyr.db ] || [ -f "/var/lib/pyr-server/$OLD_DB" ] || die "The backup has no database in it."
  chown -R "pyr-server:$GROUP" /var/lib/pyr-server
  chmod 700 /var/lib/pyr-server
  echo "    messages, members, roles, files and the server's identity are back"
fi
chown -R "pyr-server:$GROUP" /var/lib/pyr-server
chmod 750 /var/lib/pyr-server

is_private() {
  case "$1" in
    '' | 10.* | 127.* | 192.168.* | 172.1[6-9].* | 172.2[0-9].* | 172.3[01].* | 100.6[4-9].* | 100.[7-9][0-9].* | 100.1[01][0-9].* | 100.12[0-7].*) return 0 ;;
    *) return 1 ;;
  esac
}
if [ -z "$HOST" ]; then
  step "Finding this server's public address"
  HOST=$(ip -4 route get 1.1.1.1 2>/dev/null | sed -n 's/.* src \([0-9.]*\).*/\1/p' | head -n 1 || true)
  if is_private "$HOST"; then
    # A private address: the provider puts the public one in front (AWS, Google Cloud,
    # Azure, Oracle). Ask a public "what's my IP" service.
    for url in https://api.ipify.org https://ifconfig.me/ip https://icanhazip.com; do
      PUB=$(curl -4fsS --max-time 6 "$url" 2>/dev/null || wget -4qO- -T 6 "$url" 2>/dev/null || true)
      PUB=$(printf '%s' "$PUB" | tr -d ' \r\n')
      case "$PUB" in
        '' | *[!0-9.]*) continue ;;
      esac
      HOST=$PUB
      break
    done
  fi
  if is_private "$HOST"; then
    die "Couldn't work out this server's public address. Run the setup again with --public-host <its IP address or domain name>."
  fi
  echo "    $HOST"
fi

step "Setting up the community"
set -- --data /var/lib/pyr-server --bind "0.0.0.0:$PORT" --public-host "$HOST"
if [ -n "$NAME" ]; then
  set -- "$@" --name "$NAME"
fi
if command -v runuser >/dev/null 2>&1; then
  LINKS=$(runuser -u pyr-server -- /usr/local/bin/pyr-server "$@" links 2>&1) || die "Couldn't set up the community: $LINKS"
else
  LINKS=$(/usr/local/bin/pyr-server "$@" links 2>&1) || die "Couldn't set up the community: $LINKS"
  chown -R "pyr-server:$GROUP" /var/lib/pyr-server
fi
OWNER=$(printf '%s\n' "$LINKS" | sed -n 's/^PYR_OWNER_LINK=//p')
INVITE=$(printf '%s\n' "$LINKS" | sed -n 's/^PYR_INVITE_LINK=//p')
CNAME=$(printf '%s\n' "$LINKS" | sed -n 's/^PYR_NAME=//p')
if [ -z "$OWNER" ] || [ -z "$INVITE" ]; then
  die "Couldn't set up the community: $LINKS"
fi

step "Letting Pyr through the firewall (UDP port $PORT)"
OPENED=""
if command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | grep -q "^Status: active"; then
  ufw allow "$PORT/udp" comment "Pyr" >/dev/null && OPENED="ufw"
fi
if command -v firewall-cmd >/dev/null 2>&1 && firewall-cmd --state >/dev/null 2>&1; then
  if firewall-cmd --quiet --permanent --add-port="$PORT/udp" && firewall-cmd --quiet --reload; then
    OPENED="${OPENED:+$OPENED and }firewalld"
  fi
fi
if [ -z "$OPENED" ] && command -v iptables >/dev/null 2>&1; then
  # Plain iptables rules that turn away everything else, like Oracle Cloud's images use.
  RULES=$(iptables -S INPUT 2>/dev/null || true)
  if printf '%s\n' "$RULES" | grep -q -e '^-P INPUT DROP' -e '-j REJECT' -e '-j DROP'; then
    iptables -C INPUT -p udp --dport "$PORT" -j ACCEPT 2>/dev/null || iptables -I INPUT -p udp --dport "$PORT" -j ACCEPT
    OPENED="iptables"
    if command -v netfilter-persistent >/dev/null 2>&1; then
      netfilter-persistent save >/dev/null 2>&1 || warn "Couldn't save the iptables rule; it may be lost when the server restarts."
    elif [ -d /etc/iptables ]; then
      iptables-save > /etc/iptables/rules.v4
    elif [ -f /etc/sysconfig/iptables ]; then
      iptables-save > /etc/sysconfig/iptables
    else
      warn "Opened UDP $PORT in iptables, but couldn't save the rule; it may be lost when the server restarts."
    fi
  fi
fi
if [ -n "$OPENED" ]; then
  echo "    opened in $OPENED"
else
  echo "    no firewall is blocking it on the server itself"
fi

step "Starting the server"
# Ports below 1024 need the one capability that allows them; otherwise none at all.
if [ "$PORT" -lt 1024 ]; then
  CAPS="CAP_NET_BIND_SERVICE"
else
  CAPS=""
fi
cat > /etc/systemd/system/pyr-server.service <<UNIT
[Unit]
Description=Pyr server (voice, webcams, screen sharing and chat)
After=network-online.target
Wants=network-online.target

[Service]
User=pyr-server
Group=$GROUP
ExecStart=/usr/local/bin/pyr-server --data /var/lib/pyr-server --bind 0.0.0.0:$PORT
Restart=always
RestartSec=3
LimitNOFILE=65536
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=strict
ProtectHome=yes
ReadWritePaths=/var/lib/pyr-server
UMask=0077
CapabilityBoundingSet=$CAPS
AmbientCapabilities=$CAPS
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

[Install]
WantedBy=multi-user.target
UNIT
# Waits (at most 30 seconds) for the service to run and listen on its port.
server_up() {
  i=0
  while [ $i -lt 30 ]; do
    if systemctl is-active --quiet pyr-server; then
      if ! command -v ss >/dev/null 2>&1 || ss -uln 2>/dev/null | grep -q "[:.]${PORT}[[:space:]]"; then
        return 0
      fi
    fi
    sleep 1
    i=$((i + 1))
  done
  return 1
}
systemctl daemon-reload
systemctl enable pyr-server >/dev/null 2>&1
systemctl restart pyr-server || true
if ! server_up; then
  # Some servers (older containers) can't apply the extra protections from UMask= down to
  # SystemCallArchitectures=: try once without them (keeping a low port's one capability).
  warn "The server didn't start with the extra protections; trying without them."
  sed -i '/^UMask=/,/^SystemCallArchitectures=/d' /etc/systemd/system/pyr-server.service
  if [ -n "$CAPS" ]; then
    sed -i "/^ReadWritePaths=/a AmbientCapabilities=$CAPS" /etc/systemd/system/pyr-server.service
  fi
  systemctl daemon-reload
  systemctl restart pyr-server || true
  if ! server_up; then
    journalctl -u pyr-server -n 20 --no-pager 2>/dev/null || true
    undo_migration
    die "The server didn't start (see the log above)."
  fi
fi
# Running: what was set aside from before the rename goes.
if [ -n "$MIGRATED" ]; then
  rm -rf /etc/systemd/system/$OLD.service.before-pyr /etc/systemd/system/$OLD.service.d.before-pyr /usr/local/bin/$OLD.before-pyr
  echo "    the server from before the rename is Pyr's now (same community, same links)"
fi

echo
echo "Pyr server \"$CNAME\" is running on $HOST, UDP port $PORT."
echo "It starts by itself after a reboot. Its data is in /var/lib/pyr-server."
echo
echo "Your owner link (open it in Pyr first: it works once and makes you the owner):"
echo "    $OWNER"
echo "Invite link for friends:"
echo "    $INVITE"
echo
echo "PYR_NAME=$CNAME"
echo "PYR_OWNER_LINK=$OWNER"
echo "PYR_INVITE_LINK=$INVITE"
echo "PYR_DONE=1"
