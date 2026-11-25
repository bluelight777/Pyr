#!/usr/bin/env bash
# Build dist/pyr-<version>-linux-x86_64.tar.gz: Pyr for your home folder, on any Linux (0.16.7;
# Arch and Omarchy, Fedora, and the rest, with or without apt). From the release binaries, as
# build-deb.sh, which it goes beside:
#   cargo build --release -p pyr -p pyr-mcp -p pyr-server
#   scripts/build-vps-server.sh
#   installer/linux/build-tar.sh
# It's one version's folder (crates/app/src/home_install.rs): installer/site/install.sh puts it
# in ~/.local/share/pyr/app/<version>/ the first time, and Pyr's own updates after that (signed
# in latest2.json as `linux`). No pyr-bot (the Windows setup leaves it out too, unless asked).
set -euo pipefail
cd "$(dirname "$0")/../.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
REL=${REL:-target/release}
VPS=${VPS:-dist/vps}
L=installer/linux
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
D=$WORK/pyr

mkdir -p "$D"
install -m755 "$REL/pyr" "$REL/pyr-mcp" "$REL/pyr-server" "$D/"
# Built on a new distro, the binaries would otherwise insist on its glibc (see the script).
python3 $L/relax-glibc.py "$D/pyr" "$D/pyr-mcp" "$D/pyr-server"
# The static server programs the app puts on a VPS when it sets one up (beside the program:
# pyr_host::install looks there).
for a in x86_64 aarch64; do
  install -Dm755 "$VPS/pyr-server-linux-$a" "$D/vps/pyr-server-linux-$a"
done
for n in 16 32 48 64 128 256 512; do
  install -Dm644 crates/app/assets/icon/pyr-$n.png "$D/icons/pyr-$n.png"
done
# The apps menu's entry: Pyr writes it out with the program's own path (home_install::integrate).
install -m644 $L/pyr.desktop "$D/pyr.desktop"
install -m644 README.md "$D/README.md"
install -m644 $L/copyright "$D/copyright"
# What makes it Pyr in your home folder, and which version (home_install::MARKER).
printf '{"version": "%s"}\n' "$VERSION" > "$D/pyr-home.json"
chmod 644 "$D/pyr-home.json"

mkdir -p dist
OUT=dist/pyr-$VERSION-linux-x86_64.tar.gz
# The same files make the same bytes: names in order, no owner, one date (the last commit's), and
# no name or date in the gzip header.
WHEN=$(git log -1 --format=%cI 2>/dev/null || echo 2026-11-25T00:00:00Z)
tar --sort=name --owner=0 --group=0 --numeric-owner --mode='u+rwX,go+rX,go-w' --mtime="$WHEN" --format=gnu \
  -C "$D" -cf - . | gzip -9n > "$OUT.part"
mv "$OUT.part" "$OUT"
# (A quick look that it's whole and has what an update needs.)
LIST=$(tar -tzf "$OUT")
for f in ./pyr ./pyr-mcp ./pyr-server ./pyr-home.json ./pyr.desktop ./vps/pyr-server-linux-x86_64 ./vps/pyr-server-linux-aarch64 ./icons/pyr-512.png; do
  grep -qxF "$f" <<<"$LIST" || { echo "$OUT has no $f" >&2; exit 1; }
done
ls -l "$OUT"
