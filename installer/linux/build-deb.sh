#!/usr/bin/env bash
# Build dist/pyr_<version>_amd64.deb from the release binaries:
#   cargo build --release -p pyr -p pyr-mcp -p pyr-server
#   cargo build --release -p pyr-engine --example bot
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
PKG=$WORK/pyr_${VERSION}_${ARCH}
L=installer/linux

install -Dm755 "$REL/pyr" "$PKG/usr/bin/pyr"
# Pyr was called Ingle before 0.8.0: an Ingle updating itself reopens /usr/bin/ingle.
ln -s pyr "$PKG/usr/bin/ingle"
install -Dm755 "$REL/pyr-mcp" "$PKG/usr/bin/pyr-mcp"
install -Dm755 "$REL/pyr-server" "$PKG/usr/bin/pyr-server"
install -Dm755 "$REL/examples/bot" "$PKG/usr/bin/pyr-bot"
# Built on a new distro, the binaries would otherwise insist on its glibc (see the script).
python3 $L/relax-glibc.py "$PKG/usr/bin/pyr" "$PKG/usr/bin/pyr-mcp" "$PKG/usr/bin/pyr-server" "$PKG/usr/bin/pyr-bot"
# Static Linux server programs the app uploads when it sets up a VPS (x86-64 and ARM64),
# built by scripts/build-vps-server.sh. They have no library dependencies.
VPS=${VPS:-dist/vps}
for a in x86_64 aarch64; do
  install -Dm755 "$VPS/pyr-server-linux-$a" "$PKG/usr/lib/pyr/vps/pyr-server-linux-$a"
done
install -Dm644 $L/pyr.desktop "$PKG/usr/share/applications/pyr.desktop"
for n in 16 32 48 64 128 256 512; do
  install -Dm644 crates/app/assets/icon/pyr-$n.png "$PKG/usr/share/icons/hicolor/${n}x${n}/apps/pyr.png"
done
install -Dm644 $L/pyr-server.service "$PKG/usr/lib/systemd/system/pyr-server.service"
# The in-app updater's root step (checks the package as root, then installs it) and its
# password prompt.
install -Dm755 $L/install-update "$PKG/usr/lib/pyr/install-update"
install -Dm644 $L/com.firelitgames.pyr.install-update.policy "$PKG/usr/share/polkit-1/actions/com.firelitgames.pyr.install-update.policy"
install -Dm644 $L/pyr-server.default "$PKG/etc/default/pyr-server"
install -Dm644 README.md "$PKG/usr/share/doc/pyr/README.md"
install -Dm644 $L/copyright "$PKG/usr/share/doc/pyr/copyright"

# Libraries the binaries link against, with minimum versions (dpkg-shlibdeps needs a
# debian/control to run, so give it a throwaway one).
mkdir -p "$WORK/src/debian"
printf 'Source: pyr\n\nPackage: pyr\nArchitecture: any\n' > "$WORK/src/debian/control"
LINKED=$(cd "$WORK/src" && dpkg-shlibdeps -O "$PKG/usr/bin/pyr" "$PKG/usr/bin/pyr-mcp" "$PKG/usr/bin/pyr-server" "$PKG/usr/bin/pyr-bot" 2>/dev/null \
  | sed -n 's/^shlibs:Depends=//p')
# The oldest glibc that has every version the binaries hard-require (relax-glibc.py made the
# rest optional; dpkg-shlibdeps doesn't know that).
GLIBC_MIN=$(for b in "$PKG"/usr/bin/pyr*; do readelf -V "$b"; done \
  | sed -n 's/.*Name: GLIBC_\([0-9.]*\)  Flags: none.*/\1/p' | sort -V | tail -1)
# Ubuntu 24.04 renamed libasound2 to libasound2t64; accept either.
LINKED=$(echo "$LINKED" | sed -E "s/libc6 \(>= [0-9.]+\)/libc6 (>= $GLIBC_MIN)/; s/libasound2t64( \([^)]*\))?/libasound2t64\1 | libasound2\1/; s/, libx11-6//")
# Loaded at run time (window system, OpenGL), so dpkg-shlibdeps can't see them.
RUNTIME="libx11-6, libx11-xcb1, libxcursor1, libxrandr2, libxi6, libxkbcommon0, libxkbcommon-x11-0, libgl1 | libegl1"

mkdir -p "$PKG/DEBIAN"
cat > "$PKG/DEBIAN/control" <<CONTROL
Package: pyr
Version: $VERSION
Architecture: $ARCH
Maintainer: Firelit Games <noreply@pyr.invalid>
Installed-Size: $(du -sk "$PKG" | cut -f1)
Depends: $LINKED, $RUNTIME
Conflicts: ingle
Replaces: ingle
Provides: ingle
Recommends: libvulkan1, libwayland-client0, libwayland-cursor0, libegl1
Section: net
Priority: optional
Description: voice, webcams and screen sharing with text channels
 Pyr is a lightweight, self-hostable alternative to Discord for groups:
 servers with text channels and voice channels, webcams, screen sharing up to
 1080p60, and end-to-end encrypted calls. There are no accounts; your identity
 is a key stored on your computer.
 .
 The app can set up the server on any Linux VPS you have, over SSH, or
 create one on your own DigitalOcean account, so your group always has a
 place to meet.
 .
 This package contains the desktop app (pyr), its Claude connector
 (pyr-mcp), the community server
 (pyr-server, which can also run as the pyr-server systemd service), the
 static server programs the app installs on VPSes, and a test participant
 (pyr-bot).
CONTROL
for s in postinst prerm postrm; do install -m755 $L/$s "$PKG/DEBIAN/$s"; done
echo /etc/default/pyr-server > "$PKG/DEBIAN/conffiles"

mkdir -p dist
OUT="dist/pyr_${VERSION}_${ARCH}.deb"
# xz rather than the newer zstd default, so older dpkg versions can open it too.
dpkg-deb --root-owner-group -Zxz --build "$PKG" "$OUT"
ls -l "$OUT"
