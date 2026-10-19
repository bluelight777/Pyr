#!/usr/bin/env bash
# Assemble a release for pyr.gg from the built installers:
#   KEY=path/to/pyr-release-key.txt scripts/make-release.sh "What's new in this version"
# Output: dist/release/ with upload/pyr/ (the website folder, with a signed latest.json),
# upload/install-site.sh and publish.cmd / publish.ps1 to put it on the VPS.
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
NOTES=${1:-}
BASE=https://pyr.gg
OUT=dist/release
rm -rf "$OUT" && mkdir -p "$OUT/upload/pyr"
SITE=$OUT/upload/pyr
# The page, with this release filled in (the page also reads latest.json when it opens), and
# what it shows (pictures, fonts).
WIN_MB=$(( ($(stat -c %s "dist/Pyr-Setup-$VERSION.exe") + 500000) / 1000000 ))
TODAY=$(LC_ALL=C date +"%B %-d, %Y")
sed -e "s/{{VERSION}}/$VERSION/g" -e "s/{{WIN_MB}}/$WIN_MB/g" -e "s/{{DATE}}/$TODAY/g" installer/site/index.html > "$SITE/index.html"
cp installer/site/pyr-icon.png "$SITE/"
cp -r installer/site/assets "$SITE/"
cp "dist/Pyr-Setup-$VERSION.exe" "$SITE/" && cp "dist/Pyr-Setup-$VERSION.exe" "$SITE/Pyr-Setup.exe"
if [ -f "dist/pyr_${VERSION}_amd64.deb" ]; then
  cp "dist/pyr_${VERSION}_amd64.deb" "$SITE/" && cp "dist/pyr_${VERSION}_amd64.deb" "$SITE/pyr_amd64.deb"
  DEB=(--deb "$SITE/pyr_${VERSION}_amd64.deb")
else
  echo "no .deb for $VERSION; the Linux download keeps the last one" >&2
  rm -f "$SITE/pyr_amd64.deb"
  DEB=()
fi
# Every version released so far (apps show how many updates they're behind).
grep -qx "$VERSION" installer/site/releases.txt || echo "$VERSION" >> installer/site/releases.txt
RELEASED=$(paste -sd, installer/site/releases.txt)
cargo run -q -p pyr-proto --example release -- sign --key "$KEY" --version "$VERSION" --notes "$NOTES" --released "$RELEASED" \
  --base-url "$BASE" --windows "$SITE/Pyr-Setup-$VERSION.exe" "${DEB[@]}" --out "$SITE/latest.json"
# The Pyr Library (scripts/make-library.sh): its signed list and the plugins' files, in one
# tar file (its folders are too deep to copy onto a PC one by one); install-site.sh puts them at
# /library/ on the website.
if [ -f dist/library/index.json ]; then
  # (index2.json: the list Pyr 0.8.1 and newer read; index.json: older Pyrs'.)
  tar --owner=0 --group=0 --numeric-owner -cf "$OUT/upload/library.tar" -C dist library/index.json $( [ -f dist/library/index2.json ] && echo library/index2.json ) library/files
else
  echo "no dist/library/index.json: the website keeps the library it has" >&2
fi
cp installer/site/install-site.sh "$OUT/upload/"
# The bug reports this release fixes (installer/site/fixed-bugs.txt, a number first on each
# line): install-site.sh marks them fixed on the VPS's guild server, where reports go.
if grep -Eq '^[0-9]+' installer/site/fixed-bugs.txt 2>/dev/null; then
  cp installer/site/fixed-bugs.txt "$OUT/upload/"
fi
mkdir -p "$OUT/upload/server" && cp dist/vps/pyr-server-linux-x86_64 dist/vps/pyr-server-linux-aarch64 "$OUT/upload/server/"
# Short links (https://pyr.gg/<code>): the invite directory, and the script that sets it up
# with pyr.gg's nginx settings the first time (install-site.sh runs it then; later releases
# just bring a new pyr-directory).
cp installer/site/setup-pyr-gg.sh "$OUT/upload/"
if [ -f dist/vps/pyr-directory-linux-x86_64 ]; then
  cp dist/vps/pyr-directory-linux-x86_64 "$OUT/upload/server/"
else
  echo "no dist/vps/pyr-directory-linux-x86_64: pyr.gg keeps the invite directory it has" >&2
fi
# The Pyr Library Keeper (dist/keeper, from scripts/make-keeper.sh): install-site.sh keeps it on
# the VPS, and sets it up again there once its Claude API key is in (setup-keeper.sh).
if [ -f dist/keeper/keeper-cert.json ]; then
  mkdir -p "$OUT/upload/keeper" && cp -r dist/keeper/. "$OUT/upload/keeper/"
  cp installer/site/setup-keeper.sh "$OUT/upload/"
else
  echo "no dist/keeper: the Library Keeper isn't in this release" >&2
fi
cp installer/site/publish.ps1 installer/site/publish.cmd "$OUT/"
ls -la "$SITE"
