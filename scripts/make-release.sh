#!/usr/bin/env bash
# Assemble a release for firelitgames.com/ingle from the built installers:
#   KEY=path/to/ingle-release-key.txt scripts/make-release.sh "What's new in this version"
# Output: dist/release/ with upload/ingle/ (the website folder, with a signed latest.json),
# upload/install-site.sh and publish.cmd / publish.ps1 to put it on the VPS.
set -euo pipefail
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
NOTES=${1:-}
BASE=https://firelitgames.com/ingle
OUT=dist/release
rm -rf "$OUT" && mkdir -p "$OUT/upload/ingle"
SITE=$OUT/upload/ingle
# The page, with this release filled in (the page also reads latest.json when it opens), and
# what it shows (pictures, fonts).
WIN_MB=$(( ($(stat -c %s "dist/Ingle-Setup-$VERSION.exe") + 500000) / 1000000 ))
TODAY=$(LC_ALL=C date +"%B %-d, %Y")
sed -e "s/{{VERSION}}/$VERSION/g" -e "s/{{WIN_MB}}/$WIN_MB/g" -e "s/{{DATE}}/$TODAY/g" installer/site/index.html > "$SITE/index.html"
cp installer/site/ingle-icon.png "$SITE/"
cp -r installer/site/assets "$SITE/"
cp "dist/Ingle-Setup-$VERSION.exe" "$SITE/" && cp "dist/Ingle-Setup-$VERSION.exe" "$SITE/Ingle-Setup.exe"
if [ -f "dist/ingle_${VERSION}_amd64.deb" ]; then
  cp "dist/ingle_${VERSION}_amd64.deb" "$SITE/" && cp "dist/ingle_${VERSION}_amd64.deb" "$SITE/ingle_amd64.deb"
  DEB=(--deb "$SITE/ingle_${VERSION}_amd64.deb")
else
  echo "no .deb for $VERSION; the Linux download keeps the last one" >&2
  rm -f "$SITE/ingle_amd64.deb"
  DEB=()
fi
# Every version released so far (apps show how many updates they're behind).
grep -qx "$VERSION" installer/site/releases.txt || echo "$VERSION" >> installer/site/releases.txt
RELEASED=$(paste -sd, installer/site/releases.txt)
cargo run -q -p ingle-proto --example release -- sign --key "$KEY" --version "$VERSION" --notes "$NOTES" --released "$RELEASED" \
  --base-url "$BASE" --windows "$SITE/Ingle-Setup-$VERSION.exe" "${DEB[@]}" --out "$SITE/latest.json"
cp installer/site/install-site.sh "$OUT/upload/"
mkdir -p "$OUT/upload/server" && cp dist/vps/ingle-server-linux-x86_64 dist/vps/ingle-server-linux-aarch64 "$OUT/upload/server/"
cp installer/site/publish.ps1 installer/site/publish.cmd "$OUT/"
ls -la "$SITE"
