#!/bin/bash
# Puts the website on https://pyr.gg (and https://www.pyr.gg, which sends people on to
# https://pyr.gg) on this VPS: an nginx site with a free Let's Encrypt certificate that renews
# by itself. Until Pyr 0.8.0 is published there, pyr.gg shows the same page and downloads as
# https://pyr.gg/ (it serves that same folder).
#
# Run on the VPS as root:   bash setup-pyr-gg.sh
#
# Safe to run again. It leaves the other sites alone: it notes how each of them answers before
# it changes anything and again after every change, and takes its own change back out if
# anything else answers differently or nginx won't take it.
set -euo pipefail
export LC_ALL=C

DOMAIN=pyr.gg
WWW=www.pyr.gg
OLD_SITE=firelitgames.com
VPS_IP=${VPS_IP:-2.24.121.231}
PYR_DIR=/var/www/pyr.gg          # Pyr's own page, once 0.8.0 is published there
ACME_DIR=/var/www/pyr.gg-acme    # for Let's Encrypt's checks (http://pyr.gg/.well-known/acme-challenge/)
LIVE=/etc/letsencrypt/live/$DOMAIN
DNS_WAIT_SECS=${DNS_WAIT_SECS:-900}
WORK=$(mktemp -d)
NGX=$WORK/nginx-T.txt

say() { printf '\n== %s\n' "$*"; }
note() { printf '   %s\n' "$*"; }
die() { printf '\nSTOPPED: %s\n' "$*" >&2; exit 1; }

PENDING=""   # set while new settings are in place but not yet checked
PREV=""      # a copy of the settings that were there before, or empty for none
CONF=""
LINK=""
on_exit() {
  local rc=$?
  if [ -n "$PENDING" ]; then
    echo "Something went wrong part way; putting nginx's settings back the way they were." >&2
    restore_prev || true
  fi
  rm -rf "$WORK"
  exit $rc
}
trap on_exit EXIT

dump() { nginx -T > "$NGX" 2>/dev/null || true; }

if systemctl is-active --quiet nginx 2>/dev/null; then
  RELOAD="systemctl reload nginx"
else
  RELOAD="nginx -s reload"
fi
reload_nginx() { $RELOAD; sleep 1; }

# Every use of a directive in nginx's settings, one a line: "the file<TAB>its words". (Found
# anywhere on a line, for settings written several to a line.)
directive() {
  awk -v d="$1" '
    /^# configuration file / { f = $4; sub(/:$/, "", f); next }
    {
      line = $0
      sub(/#.*$/, "", line)
      re = "(^|[;{} \t])" d "[ \t]+[^;{}]*"
      while (match(line, re)) {
        s = substr(line, RSTART, RLENGTH)
        line = substr(line, RSTART + RLENGTH)
        sub("^.*" d "[ \t]+", "", s)
        sub(/[ \t]+$/, "", s)
        print f "\t" s
      }
    }' "$NGX"
}
# Every name nginx answers for, as written in its settings.
names() {
  directive server_name | cut -f2 | tr ' \t' '\n\n' | awk 'NF' | sort -u
}
# The addresses nginx listens on for a port (80 or 443), as written: 443, [::]:443, 1.2.3.4:443...
listens() {
  directive listen | cut -f2 | awk -v p="$1" '{ n = split($1, s, ":"); if (s[n] == p) print $1 }' | sort -u
}
# The files that give a name to a site.
files_naming() {
  directive server_name | awk -F '\t' -v names="$*" '
    BEGIN { split(names, want, " ") }
    { n = split($2, w, /[ \t]+/); for (i = 1; i <= n; i++) for (j in want) if (w[i] == want[j]) print $1 }' | sort -u
}
# How a site answers: the status and where it sends you.
probe() {
  local port=80
  if [ "$1" = https ]; then port=443; fi
  curl -sk --noproxy '*' --max-time 8 -o /dev/null -w '%{http_code} %{redirect_url}' \
    --resolve "$2:$port:$CONNECT" "$1://$2/" 2>/dev/null || true
}
# How every other site on this server answers (and what answers for a bare address).
snapshot() {
  dump
  local n
  for n in $(names | grep -vxE '_|localhost|pyr\.gg|www\.pyr\.gg' | grep -vE '[*~$/\\]' | awk 'NR <= 25'); do
    printf '%s http %s\n' "$n" "$(probe http "$n")"
    printf '%s https %s\n' "$n" "$(probe https "$n")"
  done
  printf 'address http %s\n' "$(curl -s --noproxy '*' --max-time 8 -o /dev/null -w '%{http_code} %{redirect_url}' "http://$CONNECT/" 2>/dev/null || true)"
  printf 'address https %s\n' "$(curl -sk --noproxy '*' --max-time 8 -o /dev/null -w '%{http_code} %{redirect_url}' "https://$CONNECT/" 2>/dev/null || true)"
  if command -v openssl > /dev/null 2>&1; then
    printf 'address certificate %s\n' "$(openssl s_client -connect "$CONNECT:443" < /dev/null 2>/dev/null | openssl x509 -noout -subject 2>/dev/null || true)"
  fi
}
# Every answer noted before must be the same now (answers that were "nothing listening" may
# change: pyr.gg can be the first site on an address). Tried twice, in case a site was slow.
same_as_before() {
  local now attempt
  for attempt in 1 2; do
    now=$(snapshot)
    if awk 'NR == FNR { k = $1 " " $2; if ($3 != "000") want[k] = $0; next }
            { got[$1 " " $2] = $0 }
            END { for (k in want) if (want[k] != got[k]) { print "   before: " want[k]; print "   now:    " got[k]; bad = 1 }
                  exit bad }' <(printf '%s\n' "$BEFORE") <(printf '%s\n' "$now") > "$WORK/diff.txt"; then
      return 0
    fi
    if [ "$attempt" = 1 ]; then sleep 3; fi
  done
  cat "$WORK/diff.txt" >&2
  return 1
}
restore_prev() {
  if [ -n "$PREV" ]; then
    install -m 0644 "$PREV" "$CONF"
    if [ -n "$LINK" ]; then ln -sfn "$CONF" "$LINK"; fi
  else
    rm -f "$CONF"
    if [ -n "$LINK" ]; then rm -f "$LINK"; fi
  fi
  if nginx -t > /dev/null 2>&1; then reload_nginx || true; fi
}
# Puts a file in place as pyr.gg's nginx settings and keeps it only if nginx takes it and every
# other site still answers as before.
apply_conf() {
  PREV=""
  if [ -f "$CONF" ]; then
    cp -f "$CONF" "$WORK/prev.conf"
    PREV=$WORK/prev.conf
  fi
  PENDING=1
  install -m 0644 "$1" "$CONF"
  if [ -n "$LINK" ]; then ln -sfn "$CONF" "$LINK"; fi
  if ! nginx -t > "$WORK/nginx-t.txt" 2>&1 || grep -q "conflicting server name \"[a-z.]*pyr\.gg\"" "$WORK/nginx-t.txt"; then
    sed 's/^/   /' "$WORK/nginx-t.txt" >&2
    restore_prev
    PENDING=""
    die "nginx didn't take the settings for $DOMAIN (above). They're back the way they were before this step; the other sites are untouched."
  fi
  grep -F "$(basename "$CONF")" "$WORK/nginx-t.txt" | grep -F '[warn]' | sed 's/^.*\[warn\][^:]*: /   nginx says: /' || true
  dump
  if ! grep -qE "^# configuration file ($CONF|${LINK:-/nonexistent}):" "$NGX"; then
    restore_prev
    PENDING=""
    die "nginx doesn't read $CONF (its settings include other files). Took it back out; the other sites are untouched."
  fi
  reload_nginx
  if ! same_as_before; then
    restore_prev
    PENDING=""
    die "Another site on this server answered differently with $DOMAIN's new settings in (above), so they're back the way they were before this step."
  fi
  PENDING=""
}
# 0 when a name points at this VPS and nginx here answers for it.
points_here() {
  local h=$1 ip t got
  ip=$( (getent ahostsv4 "$h" 2>/dev/null || true) | awk 'NR == 1 { print $1 }')
  if [ "$ip" != "$VPS_IP" ]; then
    note "$h points to ${ip:-nothing yet} (waiting for $VPS_IP)"
    return 1
  fi
  t=$(head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n')
  echo "$t" > "$ACME_DIR/.well-known/acme-challenge/$t"
  got=$(curl -s --noproxy '*' --max-time 8 --resolve "$h:80:$CONNECT" "http://$h/.well-known/acme-challenge/$t" 2>/dev/null || true)
  rm -f "$ACME_DIR/.well-known/acme-challenge/$t"
  if [ "$got" != "$t" ]; then
    note "$h points here, but nginx didn't hand back a test file for it"
    return 1
  fi
}

[ "$(id -u)" = 0 ] || die "Run this as root."
command -v nginx > /dev/null 2>&1 || die "nginx isn't installed on this server."
command -v curl > /dev/null 2>&1 || die "curl isn't installed on this server."
nginx -t > /dev/null 2>&1 || die "nginx's settings already have a problem (nginx -t fails). Nothing was changed."
if ip -4 -o addr show 2>/dev/null | grep -qF " $VPS_IP/"; then CONNECT=$VPS_IP; else CONNECT=127.0.0.1; fi

say "Finding the folder that serves https://$OLD_SITE/pyr/"
dump
ROOT=""
TOKEN=$(head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n')
for R in $(directive root | cut -f2 | awk '{ print $1 }' | sort -u); do
  [ -d "$R" ] || continue
  echo "$TOKEN" > "$R/pyr-probe-$TOKEN.txt" 2>/dev/null || continue
  GOT=$(curl -sk --noproxy '*' --max-time 5 --resolve "$OLD_SITE:443:127.0.0.1" "https://$OLD_SITE/pyr-probe-$TOKEN.txt" || true)
  rm -f "$R/pyr-probe-$TOKEN.txt"
  if [ "$GOT" = "$TOKEN" ]; then ROOT=$R; break; fi
done
[ -n "$ROOT" ] || die "Couldn't find the folder that serves https://$OLD_SITE/. Nothing was changed."
PYR=$ROOT/pyr
[ -f "$PYR/index.html" ] && [ -f "$PYR/latest.json" ] || die "$PYR doesn't hold the Pyr page. Nothing was changed."
if [ -f "$PYR_DIR/index.html" ] && [ ! -L "$PYR_DIR" ]; then SITE_ROOT=$PYR_DIR; else SITE_ROOT=$PYR; fi
note "pyr.gg will show $SITE_ROOT"

# Next to the file that has firelitgames.com's settings, named to come after it (so pyr.gg
# never becomes the site nginx shows when it doesn't know the name asked for).
OLD_FILE=$(files_naming "$OLD_SITE" | awk 'NR == 1')
CONF_DIR=$(dirname "${OLD_FILE:-/etc/nginx/nginx.conf}")
case "$OLD_FILE" in *.vhost) EXT=.vhost ;; *) EXT=.conf ;; esac
if [ "$CONF_DIR" = /etc/nginx ]; then
  if grep -qE '^[[:space:]]*include[[:space:]]+/etc/nginx/sites-enabled/' "$NGX"; then CONF_DIR=/etc/nginx/sites-enabled
  elif grep -qE '^[[:space:]]*include[[:space:]]+/etc/nginx/conf\.d/' "$NGX"; then CONF_DIR=/etc/nginx/conf.d
  else die "Couldn't tell where nginx keeps its sites' settings. Nothing was changed."; fi
fi
NAME=zz-$DOMAIN$EXT
if [ "$CONF_DIR" = /etc/nginx/sites-enabled ] && [ -d /etc/nginx/sites-available ]; then
  CONF=/etc/nginx/sites-available/$NAME
  LINK=/etc/nginx/sites-enabled/$NAME
else
  CONF=$CONF_DIR/$NAME
  LINK=""
fi
note "settings go in $CONF${LINK:+ (turned on by $LINK)}"

# Nothing else may already answer for pyr.gg.
OTHER=$(files_naming "$DOMAIN" "$WWW" "*.$DOMAIN" ".$DOMAIN" | grep -vxF -e "$CONF" -e "${LINK:-$CONF}" | tr '\n' ' ' || true)
[ -z "$OTHER" ] || die "Another nginx site already answers for $DOMAIN (in ${OTHER% }). Nothing was changed."

L80=$(listens 80)
L443=$(listens 443)
[ -n "$L80" ] || L80=80
[ -n "$L443" ] || L443=443
# HTTP/2: nginx 1.25.1 and later turn it on per site. Before that it's on an address for every
# site there once one site's listen says so (and saying it again warns), so pyr.gg gets it
# when firelitgames.com has it.
NV=$(nginx -v 2>&1 | sed -n 's|.*nginx/\([0-9][0-9.]*\).*|\1|p')
H2_ON=""
if [ -n "$NV" ] && [ "$(printf '%s\n%s\n' 1.25.1 "$NV" | sort -V | awk 'NR == 1')" = 1.25.1 ]; then
  H2_ON="    http2 on;"
fi

say "Noting how the other sites on this server answer now"
BEFORE=$(snapshot)
note "$(printf '%s\n' "$BEFORE" | wc -l) answers noted"

install -d -m 0755 "$ACME_DIR/.well-known/acme-challenge"

listen_lines() { # port [what follows the address]
  local a extra=${2:-}
  for a in $( [ "$1" = 80 ] && echo "$L80" || echo "$L443" ); do echo "    listen $a$extra;"; done
}

say "Turning on http://$DOMAIN"
if [ -f "$LIVE/fullchain.pem" ] && [ -f "$LIVE/privkey.pem" ] && [ -f "$CONF" ] && grep -q ssl_certificate "$CONF"; then
  note "already on (with its certificate); leaving it as it is for now"
else
  cat > "$WORK/http.conf" <<EOF
# https://$DOMAIN and https://$WWW: Pyr (Pyr's new name), by Firelit Games.
# Written by setup-pyr-gg.sh; running it again writes this again.
server {
$(listen_lines 80)
    server_name $DOMAIN $WWW;
    location ^~ /.well-known/acme-challenge/ {
        root $ACME_DIR;
        default_type text/plain;
    }
    location / {
        root $SITE_ROOT;
        index index.html;
        try_files \$uri \$uri/ =404;
    }
}
EOF
  apply_conf "$WORK/http.conf"
  note "done (the other sites answer as before)"
fi

say "Waiting until $DOMAIN and $WWW point to this server ($VPS_IP)"
WAITED=0
until points_here "$DOMAIN" && points_here "$WWW"; do
  if [ "$WAITED" -ge "$DNS_WAIT_SECS" ]; then
    die "$DOMAIN doesn't point here yet (DNS changes can take up to an hour or two). http://$DOMAIN is ready for when it does: run this again later for https."
  fi
  sleep 20
  WAITED=$((WAITED + 20))
done
note "they do"

if [ -f "$LIVE/fullchain.pem" ] && openssl x509 -checkend 2592000 -noout -in "$LIVE/fullchain.pem" > /dev/null 2>&1 \
   && openssl x509 -noout -text -in "$LIVE/fullchain.pem" 2>/dev/null | grep -q "DNS:$WWW"; then
  say "The certificate for $DOMAIN is already here and good for over 30 more days"
else
  if ! command -v certbot > /dev/null 2>&1; then
    say "Installing certbot (Let's Encrypt's program for certificates)"
    export DEBIAN_FRONTEND=noninteractive NEEDRESTART_SUSPEND=1 NEEDRESTART_MODE=l
    if command -v apt-get > /dev/null 2>&1; then
      apt-get -o DPkg::Lock::Timeout=300 update -qq < /dev/null || true
      apt-get -o DPkg::Lock::Timeout=300 install -y -qq certbot < /dev/null
    elif command -v dnf > /dev/null 2>&1; then
      dnf install -y -q certbot < /dev/null || { dnf install -y -q epel-release < /dev/null && dnf install -y -q certbot < /dev/null; }
    elif command -v yum > /dev/null 2>&1; then
      yum install -y -q certbot < /dev/null || { yum install -y -q epel-release < /dev/null && yum install -y -q certbot < /dev/null; }
    elif command -v snap > /dev/null 2>&1; then
      snap install --classic certbot < /dev/null && ln -sf /snap/bin/certbot /usr/local/bin/certbot
    fi
    command -v certbot > /dev/null 2>&1 || die "Couldn't install certbot. http://$DOMAIN works; https doesn't yet."
  fi
  say "Getting the certificate for $DOMAIN and $WWW from Let's Encrypt"
  ACCOUNTS=/etc/letsencrypt/accounts/acme-v02.api.letsencrypt.org/directory
  N_ACC=0
  if [ -d "$ACCOUNTS" ]; then N_ACC=$(find "$ACCOUNTS" -mindepth 1 -maxdepth 1 -type d | wc -l); fi
  ACC=()
  if [ "$N_ACC" -eq 0 ]; then
    ACC=(--register-unsafely-without-email)
  elif [ "$N_ACC" -gt 1 ]; then
    ACC=(--account "$(find "$ACCOUNTS" -mindepth 1 -maxdepth 1 -type d -printf '%f\n' | sort | awk 'NR == 1')")
  fi
  if ! certbot certonly --non-interactive --agree-tos ${ACC[@]+"${ACC[@]}"} \
       --webroot -w "$ACME_DIR" --cert-name "$DOMAIN" -d "$DOMAIN" -d "$WWW" \
       --keep-until-expiring --expand --deploy-hook "$RELOAD" < /dev/null; then
    die "Let's Encrypt didn't give the certificate (above). http://$DOMAIN works; https doesn't yet."
  fi
  [ -f "$LIVE/fullchain.pem" ] && [ -f "$LIVE/privkey.pem" ] || die "certbot finished but the certificate isn't in $LIVE."
fi

say "Turning on https://$DOMAIN"
TLS="    ssl_certificate $LIVE/fullchain.pem;
    ssl_certificate_key $LIVE/privkey.pem;
    ssl_protocols TLSv1.2 TLSv1.3;
    add_header Strict-Transport-Security \"max-age=31536000\" always;"
cat > "$WORK/https.conf" <<EOF
# https://$DOMAIN and https://$WWW: Pyr (Pyr's new name), by Firelit Games.
# Written by setup-pyr-gg.sh; running it again writes this again. The certificate is
# Let's Encrypt's, in $LIVE, and certbot renews it by itself.
server {
$(listen_lines 80)
    server_name $DOMAIN $WWW;
    location ^~ /.well-known/acme-challenge/ {
        root $ACME_DIR;
        default_type text/plain;
    }
    location / {
        return 301 https://$DOMAIN\$request_uri;
    }
}
server {
$(listen_lines 443 " ssl")
$H2_ON
    server_name $WWW;
$TLS
    return 301 https://$DOMAIN\$request_uri;
}
server {
$(listen_lines 443 " ssl")
$H2_ON
    server_name $DOMAIN;
$TLS
    add_header X-Content-Type-Options nosniff always;
    root $SITE_ROOT;
    index index.html;
    charset utf-8;
    location / {
        try_files \$uri \$uri/ =404;
    }
    # The app's update check and the Library's list: never kept by caches.
    location = /latest.json {
        expires -1;
    }
    location = /library/index.json {
        expires -1;
    }
}
EOF
apply_conf "$WORK/https.conf"
note "done (the other sites answer as before)"

say "Checking https://$DOMAIN from here"
PROBLEMS=0
check() { # what expected got
  if [ "$2" = "$3" ]; then note "ok: $1"; else note "NOT RIGHT: $1 (expected \"$2\", got \"$3\")"; PROBLEMS=$((PROBLEMS + 1)); fi
}
CODE=$(curl -s --noproxy '*' --max-time 10 -o "$WORK/index.html" -w '%{http_code}' --resolve "$DOMAIN:443:$CONNECT" "https://$DOMAIN/" 2>/dev/null || true)
check "https://$DOMAIN/ with a valid certificate" 200 "$CODE"
check "the page is the website" yes "$(grep -qi '<title' "$WORK/index.html" 2>/dev/null && echo yes || echo no)"
check "https://$DOMAIN/latest.json" yes "$(curl -s --noproxy '*' --max-time 10 --resolve "$DOMAIN:443:$CONNECT" "https://$DOMAIN/latest.json" 2>/dev/null | grep -q '"manifest"' && echo yes || echo no)"
check "https://$WWW/ sends people to https://$DOMAIN/" "301 https://$DOMAIN/x?y=1" \
  "$(curl -s --noproxy '*' --max-time 10 -o /dev/null -w '%{http_code} %{redirect_url}' --resolve "$WWW:443:$CONNECT" "https://$WWW/x?y=1" 2>/dev/null || true)"
check "http://$DOMAIN/ sends people to https" "301 https://$DOMAIN/x" \
  "$(curl -s --noproxy '*' --max-time 10 -o /dev/null -w '%{http_code} %{redirect_url}' --resolve "$DOMAIN:80:$CONNECT" "http://$DOMAIN/x" 2>/dev/null || true)"
T=$(head -c 12 /dev/urandom | od -An -tx1 | tr -d ' \n')
echo "$T" > "$ACME_DIR/.well-known/acme-challenge/$T"
check "Let's Encrypt can still reach its checks (for renewing)" "$T" \
  "$(curl -s --noproxy '*' --max-time 10 --resolve "$DOMAIN:80:$CONNECT" "http://$DOMAIN/.well-known/acme-challenge/$T" 2>/dev/null || true)"
rm -f "$ACME_DIR/.well-known/acme-challenge/$T"

# certbot's package renews everything twice a day; make sure something does.
if systemctl list-unit-files 2>/dev/null | grep -qE '^(certbot|snap\.certbot\.renew)\.timer'; then
  note "ok: certbot renews the certificate by itself (its timer)"
elif [ -f /etc/cron.d/certbot ]; then
  note "ok: certbot renews the certificate by itself (/etc/cron.d/certbot)"
elif command -v systemctl > /dev/null 2>&1 && [ -d /etc/systemd/system ]; then
  cat > /etc/systemd/system/pyr-certbot-renew.service <<EOF
[Unit]
Description=Renew Let's Encrypt certificates (set up for pyr.gg)
[Service]
Type=oneshot
ExecStart=$(command -v certbot) renew --quiet
EOF
  cat > /etc/systemd/system/pyr-certbot-renew.timer <<'EOF'
[Unit]
Description=Renew Let's Encrypt certificates twice a day (set up for pyr.gg)
[Timer]
OnCalendar=*-*-* 03,15:17:00
RandomizedDelaySec=3600
Persistent=true
[Install]
WantedBy=timers.target
EOF
  if systemctl daemon-reload > /dev/null 2>&1 && systemctl enable --now pyr-certbot-renew.timer > /dev/null 2>&1; then
    note "ok: added a timer that renews the certificate twice a day"
  else
    rm -f /etc/systemd/system/pyr-certbot-renew.service /etc/systemd/system/pyr-certbot-renew.timer
    note "NOT RIGHT: nothing renews the certificate by itself (it lasts 90 days)"
    PROBLEMS=$((PROBLEMS + 1))
  fi
else
  note "NOT RIGHT: nothing renews the certificate by itself (it lasts 90 days)"
  PROBLEMS=$((PROBLEMS + 1))
fi

echo
if [ "$PROBLEMS" -eq 0 ]; then
  echo "Done: https://$DOMAIN is up, with the same page and downloads as https://$OLD_SITE/pyr/ for now."
  echo "https://$WWW sends people to https://$DOMAIN. The certificate renews by itself."
else
  echo "https://$DOMAIN is on, but $PROBLEMS check(s) above weren't right."
  exit 1
fi
