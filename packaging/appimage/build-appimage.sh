#!/usr/bin/env bash
# Assemble an AppDir from the release binary + the #15 assets and package it into
# a single-file AppImage with linuxdeploy (issue #43). Software-rendered iced
# needs no GPU plugin. linuxdeploy is fetched on demand; APPIMAGE_EXTRACT_AND_RUN
# lets it (and the produced AppImage) run in containers without FUSE.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"
export APPIMAGE_EXTRACT_AND_RUN=1

ARCH="$(uname -m)"
VERSION="$(sed -n 's/^version = "\([^"]*\)"$/\1/p' Cargo.toml | head -n 1)"
[ -n "$VERSION" ] || { echo "workspace version missing from Cargo.toml" >&2; exit 1; }
APPID="io.github.PierreFouquet.NotepadExtra"
BIN="$ROOT/target/release/notepad-extra"
[ -x "$BIN" ] || { echo "release binary missing — run 'cargo build --release -p notepad-iced' first" >&2; exit 1; }

# --- assemble the AppDir (mirrors the deb/rpm install layout) ---
APPDIR="$ROOT/target/appimage/AppDir"
rm -rf "$APPDIR"
install -Dm755 "$BIN"                                       "$APPDIR/usr/bin/notepad-extra"
install -Dm644 "$ROOT/packaging/linux/$APPID.desktop"       "$APPDIR/usr/share/applications/$APPID.desktop"
install -Dm644 "$ROOT/packaging/linux/$APPID.metainfo.xml"  "$APPDIR/usr/share/metainfo/$APPID.metainfo.xml"
install -Dm644 "$ROOT/packaging/linux/notepad-extra.1"      "$APPDIR/usr/share/man/man1/notepad-extra.1"
for pair in 32x32:32x32 64x64:64x64 128x128:128x128 "128x128@2x:256x256"; do
  src="${pair%%:*}"; dst="${pair##*:}"
  install -Dm644 "$ROOT/icons/$src.png" "$APPDIR/usr/share/icons/hicolor/$dst/apps/$APPID.png"
done

# --- linuxdeploy (downloaded on demand; pinned to the arch we're on) ---
TOOLS="$ROOT/target/appimage/tools"; mkdir -p "$TOOLS"
LD="$TOOLS/linuxdeploy-$ARCH.AppImage"
if [ ! -x "$LD" ]; then
  echo "==> fetching linuxdeploy ($ARCH)"
  curl -fSL -o "$LD" \
    "https://github.com/linuxdeploy/linuxdeploy/releases/download/continuous/linuxdeploy-$ARCH.AppImage"
  chmod +x "$LD"
fi

# --- build (linuxdeploy writes the AppImage into the CWD) ---
cd "$ROOT/target/appimage"
rm -f ./*.AppImage
export OUTPUT="notepad-extra-$VERSION-$ARCH.AppImage"
"$LD" --appdir "$APPDIR" \
  -d "$APPDIR/usr/share/applications/$APPID.desktop" \
  -i "$APPDIR/usr/share/icons/hicolor/256x256/apps/$APPID.png" \
  --output appimage
echo "==> AppImage:"; ls -1 "$ROOT/target/appimage/"*.AppImage
