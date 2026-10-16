#!/usr/bin/env bash
# Build dist/ingle_<version>_amd64.deb from the release binaries:
#   cargo build --release -p ingle -p ingle-mcp -p ingle-server
#   cargo build --release -p ingle-engine --example bot
#   scripts/build-vps-server.sh
#   installer/linux/build-deb.sh
# Needs dpkg-dev (for dpkg-shlibdeps).
set -euo pipefail
cd "$(dirname "$0")/../.."
ROOT=$(pwd)
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=amd64
REL=${REL:-target/release}
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
PKG=$WORK/ingle_${VERSION}_${ARCH}
L=installer/linux

install -Dm755 "$REL/ingle" "$PKG/usr/bin/ingle"
install -Dm755 "$REL/ingle-mcp" "$PKG/usr/bin/ingle-mcp"
install -Dm755 "$REL/ingle-server" "$PKG/usr/bin/ingle-server"
install -Dm755 "$REL/examples/bot" "$PKG/usr/bin/ingle-bot"
# Built on a new distro, the binaries would otherwise insist on its glibc (see the script).
python3 $L/relax-glibc.py "$PKG/usr/bin/ingle" "$PKG/usr/bin/ingle-mcp" "$PKG/usr/bin/ingle-server" "$PKG/usr/bin/ingle-bot"
# Static Linux server programs the app uploads when it sets up a VPS (x86-64 and ARM64),
# built by scripts/build-vps-server.sh. They have no library dependencies.
VPS=${VPS:-dist/vps}
for a in x86_64 aarch64; do
  install -Dm755 "$VPS/ingle-server-linux-$a" "$PKG/usr/lib/ingle/vps/ingle-server-linux-$a"
done
install -Dm644 $L/ingle.desktop "$PKG/usr/share/applications/ingle.desktop"
for n in 16 32 48 64 128 256 512; do
  install -Dm644 crates/app/assets/icon/ingle-$n.png "$PKG/usr/share/icons/hicolor/${n}x${n}/apps/ingle.png"
done
install -Dm644 $L/ingle-server.service "$PKG/usr/lib/systemd/system/ingle-server.service"
# The in-app updater's root step (checks the package as root, then installs it) and its
# password prompt.
install -Dm755 $L/install-update "$PKG/usr/lib/ingle/install-update"
install -Dm644 $L/com.firelitgames.ingle.install-update.policy "$PKG/usr/share/polkit-1/actions/com.firelitgames.ingle.install-update.policy"
install -Dm644 $L/ingle-server.default "$PKG/etc/default/ingle-server"
install -Dm644 README.md "$PKG/usr/share/doc/ingle/README.md"
install -Dm644 $L/copyright "$PKG/usr/share/doc/ingle/copyright"

# Libraries the binaries link against, with minimum versions (dpkg-shlibdeps needs a
# debian/control to run, so give it a throwaway one).
mkdir -p "$WORK/src/debian"
printf 'Source: ingle\n\nPackage: ingle\nArchitecture: any\n' > "$WORK/src/debian/control"
LINKED=$(cd "$WORK/src" && dpkg-shlibdeps -O "$PKG/usr/bin/ingle" "$PKG/usr/bin/ingle-mcp" "$PKG/usr/bin/ingle-server" "$PKG/usr/bin/ingle-bot" 2>/dev/null \
  | sed -n 's/^shlibs:Depends=//p')
# The oldest glibc that has every version the binaries hard-require (relax-glibc.py made the
# rest optional; dpkg-shlibdeps doesn't know that).
GLIBC_MIN=$(for b in "$PKG"/usr/bin/*; do readelf -V "$b"; done \
  | sed -n 's/.*Name: GLIBC_\([0-9.]*\)  Flags: none.*/\1/p' | sort -V | tail -1)
# Ubuntu 24.04 renamed libasound2 to libasound2t64; accept either.
LINKED=$(echo "$LINKED" | sed -E "s/libc6 \(>= [0-9.]+\)/libc6 (>= $GLIBC_MIN)/; s/libasound2t64( \([^)]*\))?/libasound2t64\1 | libasound2\1/; s/, libx11-6//")
# Loaded at run time (window system, OpenGL), so dpkg-shlibdeps can't see them.
RUNTIME="libx11-6, libx11-xcb1, libxcursor1, libxrandr2, libxi6, libxkbcommon0, libxkbcommon-x11-0, libgl1 | libegl1"

mkdir -p "$PKG/DEBIAN"
cat > "$PKG/DEBIAN/control" <<CONTROL
Package: ingle
Version: $VERSION
Architecture: $ARCH
Maintainer: Firelit Games <noreply@ingle.invalid>
Installed-Size: $(du -sk "$PKG" | cut -f1)
Depends: $LINKED, $RUNTIME
Recommends: libvulkan1, libwayland-client0, libwayland-cursor0, libegl1
Section: net
Priority: optional
Description: voice, webcams and screen sharing with text channels
 Ingle is a lightweight, self-hostable alternative to Discord for groups:
 servers with text channels and voice channels, webcams, screen sharing up to
 1080p60, and end-to-end encrypted calls. There are no accounts; your identity
 is a key stored on your computer.
 .
 The app can set up the server on any Linux VPS you have, over SSH, or
 create one on your own DigitalOcean account, so your group always has a
 place to meet.
 .
 This package contains the desktop app (ingle), its Claude connector
 (ingle-mcp), the community server
 (ingle-server, which can also run as the ingle-server systemd service), the
 static server programs the app installs on VPSes, and a test participant
 (ingle-bot).
CONTROL
for s in postinst prerm postrm; do install -m755 $L/$s "$PKG/DEBIAN/$s"; done
echo /etc/default/ingle-server > "$PKG/DEBIAN/conffiles"

mkdir -p dist
OUT="dist/ingle_${VERSION}_${ARCH}.deb"
# xz rather than the newer zstd default, so older dpkg versions can open it too.
dpkg-deb --root-owner-group -Zxz --build "$PKG" "$OUT"
ls -l "$OUT"
