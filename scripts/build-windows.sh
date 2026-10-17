#!/usr/bin/env bash
# Cross-compile the Windows binaries from Linux (x86_64-pc-windows-gnu, MinGW-w64).
# On a Windows PC you don't need this: run `cargo build --release` there.
#
# Needs: the MinGW-w64 toolchain (Debian/Ubuntu: `apt install mingw-w64 cmake`).
#
#   ./scripts/build-windows.sh -p pyr -p pyr-server
#   ./scripts/build-windows.sh -p pyr-engine --example bot
#
# Output: target-win/x86_64-pc-windows-gnu/release/*.exe
#
# With rustup, first run `rustup target add x86_64-pc-windows-gnu`. Without rustup (a distro
# rustc that has no Windows standard library), set RUSTSRC to a Rust source `library/`
# folder of the same version and the script builds std for Windows with -Zbuild-std.
set -euo pipefail
cd "$(dirname "$0")/.."
ROOT=$(pwd)
TARGET=x86_64-pc-windows-gnu

export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc-win32
export CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++-win32
export AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
# Link libstdc++ (OpenH264 is C++) and libgcc statically so the .exe needs no MinGW DLLs.
export CXXSTDLIB_x86_64_pc_windows_gnu=static=stdc++
GCCLIB=$(dirname "$(x86_64-w64-mingw32-gcc-win32 -print-libgcc-file-name)")

FLAGS=("-L" "native=$GCCLIB" "-C" "link-arg=-static-libgcc")
EXTRA=()
if command -v rustup >/dev/null 2>&1 && rustup target list --installed 2>/dev/null | grep -qx "$TARGET"; then
  : # rustup's Windows std is installed; a normal cross build works.
else
  if [ -z "${RUSTSRC:-}" ] || [ ! -d "$RUSTSRC" ]; then
    echo "No Windows standard library: install rustup's $TARGET target, or set RUSTSRC" >&2
    echo "to a Rust source library/ folder matching your rustc version." >&2
    exit 1
  fi
  export RUSTC_BOOTSTRAP=1
  export __CARGO_TESTS_ONLY_SRC_ROOT="$RUSTSRC"
  EXTRA=(-Zbuild-std=std,panic_unwind)
  # rustup ships these two startup objects with the target; without it we bring our own.
  FLAGS=("-C" "link-self-contained=no" "-L" "native=$ROOT/tools/win-cross" "${FLAGS[@]}")
fi
RUSTFLAGS_TOML=$(printf '"%s",' "${FLAGS[@]}")
cargo build ${PROFILE_ARG:---release} --target "$TARGET" "${EXTRA[@]}" \
  --target-dir "$ROOT/target-win" \
  --config "target.$TARGET.linker=\"x86_64-w64-mingw32-gcc-win32\"" \
  --config "target.$TARGET.ar=\"x86_64-w64-mingw32-ar\"" \
  --config "target.$TARGET.rustflags=[${RUSTFLAGS_TOML%,}]" \
  "$@"
