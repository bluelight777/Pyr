#!/bin/sh
# Ingle server setup for Linux: any distribution with systemd, on x86-64 or ARM64.
#
# Installs ingle-server as a service that starts by itself (also after a reboot), lets its
# UDP port through the server's firewall, and prints your owner link (the first person to
# open it becomes the owner) and an invite link for friends.
#
#   sudo sh install-ingle-server.sh --binary ./ingle-server-linux-x86_64
#
# Options:
#   --binary FILE        the ingle-server program for this machine (required)
#   --public-host HOST   IP address or domain name friends connect to (default: detected)
#   --name NAME          name for a new community (default: My Ingle Server)
#   --port PORT          UDP port to use (default: 7766)
#
# Running it again updates the program and keeps the community and its messages, which
# live in /var/lib/ingle-server.
set -eu

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
while [ $# -gt 0 ]; do
  case "$1" in
    --binary) BIN=${2:-}; shift 2 ;;
    --public-host) HOST=${2:-}; shift 2 ;;
    --name) NAME=${2:-}; shift 2 ;;
    --port) PORT=${2:-}; shift 2 ;;
    -h | --help) sed -n '2,20p' "$0"; exit 0 ;;
    *) die "Unknown option: $1 (see --help)" ;;
  esac
done
[ -n "$BIN" ] || die "Use --binary to say where the ingle-server program is."
[ -f "$BIN" ] || die "$BIN doesn't exist."
case "$PORT" in
  '' | *[!0-9]*) die "--port must be a number." ;;
esac

ARCH=$(uname -m)
case "$ARCH" in
  x86_64 | amd64 | aarch64 | arm64) ;;
  *) die "This server's processor ($ARCH) isn't supported yet: Ingle needs a 64-bit x86 or ARM server." ;;
esac
[ -d /run/systemd/system ] || die "This server doesn't run systemd, which the setup needs. Ubuntu, Debian, Fedora, Rocky, Alma and most VPS images do."

step "Installing the Ingle server program"
systemctl stop ingle-server >/dev/null 2>&1 || true
install -m 0755 "$BIN" /usr/local/bin/ingle-server
VERSION=$(/usr/local/bin/ingle-server --version 2>&1) ||
  die "The ingle-server program doesn't run on this machine ($ARCH): $VERSION"
echo "    $VERSION"

if ! id ingle-server >/dev/null 2>&1; then
  NOLOGIN=$(command -v nologin || echo /bin/false)
  if command -v useradd >/dev/null 2>&1; then
    if getent group ingle-server >/dev/null 2>&1; then
      useradd --system --gid ingle-server --home-dir /var/lib/ingle-server --no-create-home --shell "$NOLOGIN" ingle-server
    else
      useradd --system --user-group --home-dir /var/lib/ingle-server --no-create-home --shell "$NOLOGIN" ingle-server
    fi
  elif command -v adduser >/dev/null 2>&1; then
    adduser -S -D -H -h /var/lib/ingle-server -s "$NOLOGIN" ingle-server
  else
    die "Couldn't create the ingle-server user: this system has no useradd or adduser."
  fi
fi
GROUP=$(id -gn ingle-server)
mkdir -p /var/lib/ingle-server
chown -R "ingle-server:$GROUP" /var/lib/ingle-server
chmod 750 /var/lib/ingle-server

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
set -- --data /var/lib/ingle-server --bind "0.0.0.0:$PORT" --public-host "$HOST"
if [ -n "$NAME" ]; then
  set -- "$@" --name "$NAME"
fi
if command -v runuser >/dev/null 2>&1; then
  LINKS=$(runuser -u ingle-server -- /usr/local/bin/ingle-server "$@" links 2>&1) || die "Couldn't set up the community: $LINKS"
else
  LINKS=$(/usr/local/bin/ingle-server "$@" links 2>&1) || die "Couldn't set up the community: $LINKS"
  chown -R "ingle-server:$GROUP" /var/lib/ingle-server
fi
OWNER=$(printf '%s\n' "$LINKS" | sed -n 's/^INGLE_OWNER_LINK=//p')
INVITE=$(printf '%s\n' "$LINKS" | sed -n 's/^INGLE_INVITE_LINK=//p')
CNAME=$(printf '%s\n' "$LINKS" | sed -n 's/^INGLE_NAME=//p')
if [ -z "$OWNER" ] || [ -z "$INVITE" ]; then
  die "Couldn't set up the community: $LINKS"
fi

step "Letting Ingle through the firewall (UDP port $PORT)"
OPENED=""
if command -v ufw >/dev/null 2>&1 && ufw status 2>/dev/null | grep -q "^Status: active"; then
  ufw allow "$PORT/udp" comment "Ingle" >/dev/null && OPENED="ufw"
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
cat > /etc/systemd/system/ingle-server.service <<UNIT
[Unit]
Description=Ingle server (voice, webcams, screen sharing and chat)
After=network-online.target
Wants=network-online.target

[Service]
User=ingle-server
Group=$GROUP
ExecStart=/usr/local/bin/ingle-server --data /var/lib/ingle-server --bind 0.0.0.0:$PORT
Restart=always
RestartSec=3
LimitNOFILE=65536
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=strict
ProtectHome=yes
ReadWritePaths=/var/lib/ingle-server
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
    if systemctl is-active --quiet ingle-server; then
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
systemctl enable ingle-server >/dev/null 2>&1
systemctl restart ingle-server || true
if ! server_up; then
  # Some servers (older containers) can't apply the extra protections from UMask= down to
  # SystemCallArchitectures=: try once without them (keeping a low port's one capability).
  warn "The server didn't start with the extra protections; trying without them."
  sed -i '/^UMask=/,/^SystemCallArchitectures=/d' /etc/systemd/system/ingle-server.service
  if [ -n "$CAPS" ]; then
    sed -i "/^ReadWritePaths=/a AmbientCapabilities=$CAPS" /etc/systemd/system/ingle-server.service
  fi
  systemctl daemon-reload
  systemctl restart ingle-server || true
  if ! server_up; then
    journalctl -u ingle-server -n 20 --no-pager 2>/dev/null || true
    die "The server didn't start (see the log above)."
  fi
fi

echo
echo "Ingle server \"$CNAME\" is running on $HOST, UDP port $PORT."
echo "It starts by itself after a reboot. Its data is in /var/lib/ingle-server."
echo
echo "Your owner link (open it in Ingle first: it works once and makes you the owner):"
echo "    $OWNER"
echo "Invite link for friends:"
echo "    $INVITE"
echo
echo "INGLE_NAME=$CNAME"
echo "INGLE_OWNER_LINK=$OWNER"
echo "INGLE_INVITE_LINK=$INVITE"
echo "INGLE_DONE=1"
