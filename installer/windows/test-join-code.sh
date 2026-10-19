#!/bin/bash
# Checks setup's reading of its own file name (FindJoinCode in join-code.nsh) under Wine:
# test-join-code.sh WORKDIR
set -e
HERE=$(cd "$(dirname "$0")" && pwd)
W=${1:?work folder}
mkdir -p "$W"
makensis -V1 -DOUT="$W/t.exe" "$HERE/test-join-code.nsi"
fail=0
check() { # file-name expected
  cp "$W/t.exe" "$W/$1"
  rm -f "$W/code.txt"
  (cd "$W" && WINEDEBUG=-all ${WINE:-wine} "./$1") >/dev/null 2>&1 || true
  got=$(cat "$W/code.txt" 2>/dev/null || echo "(nothing)")
  if [ "$got" = "[$2]" ]; then echo "ok   $1 -> [$2]"; else echo "FAIL $1: expected [$2], got $got"; fail=1; fi
  rm -f "$W/$1"
}
check "Pyr-Setup-join-fire-crew.exe" "fire-crew"
check "Pyr-Setup-join-fire-crew (1).exe" "fire-crew"
check "Pyr-Setup-JOIN-Fire-Crew.exe" "fire-crew"
check "Pyr-Setup-join-qrslmnov.exe" "qrslmnov"
check "Pyr-Setup.exe" ""
check "Pyr-Setup-0.9.0.exe" ""
check "Pyr-Setup-join-ab.exe" ""
check "Pyr-Setup-join--abc.exe" ""
check "Pyr-Setup-join-abc-.exe" ""
check "Pyr-Setup-join-a&b.exe" ""
check "Pyr-Setup-join-abc%22x.exe" ""
check "Pyr-Setup-join-abcdefghijabcdefghijabcdefghijab.exe" "abcdefghijabcdefghijabcdefghijab"
check "Pyr-Setup-join-abcdefghijabcdefghijabcdefghijabc.exe" ""
check "Pyr-Setup-join-fire_crew.exe" ""
exit $fail
