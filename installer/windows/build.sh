#!/usr/bin/env bash
# Build dist/Pyr-Setup-<version>.exe from the cross-compiled binaries (on Linux, with NSIS:
# `apt install nsis`). On Windows, install NSIS and run makensis on pyr.nsi the same way,
# with -DBINDIR pointing at a folder holding pyr.exe, pyr-mcp.exe, pyr-server.exe and pyr-bot.exe.
set -euo pipefail
cd "$(dirname "$0")/../.."
REL=${REL:-target-win/x86_64-pc-windows-gnu/release}
mkdir -p dist/windows
cp "$REL/pyr.exe" "$REL/pyr-mcp.exe" "$REL/pyr-server.exe" dist/windows/
cp "$REL/examples/bot.exe" dist/windows/pyr-bot.exe
# Linux server programs the app uploads to a VPS (scripts/build-vps-server.sh).
mkdir -p dist/windows/vps
cp dist/vps/pyr-server-linux-x86_64 dist/vps/pyr-server-linux-aarch64 dist/windows/vps/
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
# Shortcuts use a copy of the icon named after its contents: Windows keeps icons it has shown by
# file name, so a new icon needs a new name to show (see ICONFILE in pyr.nsi).
ICONFILE="pyr-$(sha256sum crates/app/assets/icon/pyr.ico | cut -c1-12).ico"
makensis -V2 -DVERSION="$VERSION" -DICONFILE="$ICONFILE" installer/windows/pyr.nsi
ls -l dist/Pyr-Setup-*.exe
