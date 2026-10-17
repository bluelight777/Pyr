#!/usr/bin/env bash
# Build static Linux server binaries (musl, no dependencies) for x86-64 and ARM64 VPSes.
# The app uploads these to a VPS when it sets one up, so they must run on any distro.
#
# Needs: cargo-zigbuild (cargo install cargo-zigbuild) and Zig (pip install ziglang).
# With rustup: rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl.
# Without rustup's musl targets, set RUSTSRC to a Rust source library/ folder matching your
# rustc version and the standard library is built from source (-Zbuild-std).
#
#   ./scripts/build-vps-server.sh            # both
#   ./scripts/build-vps-server.sh x86_64     # one
#
# Output: dist/vps/pyr-server-linux-x86_64, dist/vps/pyr-server-linux-aarch64, and the same for
# pyr-directory (the invite directory behind short links, which runs on pyr.gg's VPS only).
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
ARCHES=("${@:-x86_64 aarch64}")
read -r -a ARCHES <<< "${ARCHES[*]}"
mkdir -p "$ROOT/dist/vps"
for ARCH in "${ARCHES[@]}"; do
  TARGET=$ARCH-unknown-linux-musl
  EXTRA=()
  if ! { command -v rustup >/dev/null 2>&1 && rustup target list --installed 2>/dev/null | grep -qx "$TARGET"; }; then
    if [ -z "${RUSTSRC:-}" ] || [ ! -d "$RUSTSRC" ]; then
      echo "No $TARGET standard library: install it with rustup, or set RUSTSRC." >&2
      exit 1
    fi
    export RUSTC_BOOTSTRAP=1
    export __CARGO_TESTS_ONLY_SRC_ROOT="$RUSTSRC"
    EXTRA=(-Zbuild-std=std,panic_unwind)
    # rustup's musl target ships its own C runtime objects ("self-contained"); without it,
    # Zig supplies them.
    UPPER=$(echo "$TARGET" | tr 'a-z-' 'A-Z_')
    export "CARGO_TARGET_${UPPER}_RUSTFLAGS=-C link-self-contained=no"
  fi
  cargo zigbuild --release -p pyr-server -p pyr-directory --target "$TARGET" "${EXTRA[@]}" --target-dir "$ROOT/target-musl"
  for BIN in pyr-server pyr-directory; do
    cp "$ROOT/target-musl/$TARGET/release/$BIN" "$ROOT/dist/vps/$BIN-linux-$ARCH"
    echo "built dist/vps/$BIN-linux-$ARCH"
  done
done
