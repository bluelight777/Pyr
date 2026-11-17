#!/bin/bash
# pyr.gg's page alone, for a fix to it between releases: the page (installer/site/index.html)
# with the release that's up filled in, and publish-page.cmd to put it on the VPS. The release's
# own values (they're in its page: the version, the Windows download's size in MB, the day):
#   VERSION=0.14.0 WIN_MB=105 DATE="November 16, 2026" scripts/make-page.sh
# → dist/page.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${VERSION:?set VERSION, the release up now}" "${WIN_MB:?set WIN_MB, its Windows download in MB}" "${DATE:?set DATE, the day it came out (November 16, 2026)}"
OUT=dist/page
# The page's stylesheet must be whole (as make-release.sh checks).
python3 - installer/site/index.html <<'PY' || exit 1
import sys
s = open(sys.argv[1]).read()
css = s[s.index("<style>"):s.index("</style>")]
d = 0
for i, ch in enumerate(css):
    d += (ch == "{") - (ch == "}")
    if d < 0:
        sys.exit("index.html: a } with no { before it, line %d" % (s[:s.index("<style>") + i].count("\n") + 1))
if d:
    sys.exit("index.html: %d { never closed in the stylesheet" % d)
PY
rm -rf "${OUT:?}" && mkdir -p "$OUT/upload/pyr"
sed -e "s/{{VERSION}}/$VERSION/g" -e "s/{{WIN_MB}}/$WIN_MB/g" -e "s/{{DATE}}/$DATE/g" installer/site/index.html > "$OUT/upload/pyr/index.html"
if grep -q "{{" "$OUT/upload/pyr/index.html"; then
  echo "index.html: something in {{ }} left unfilled" >&2
  exit 1
fi
cp installer/site/install-page.sh "$OUT/upload/"
cp installer/site/publish-page.ps1 installer/site/publish-page.cmd "$OUT/"
echo "$OUT: run publish-page.cmd on Blue's PC"
