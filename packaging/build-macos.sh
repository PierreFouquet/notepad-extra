#!/usr/bin/env bash
# Build the macOS .app (cargo-bundle) and wrap it in a compressed .dmg (issue
# #43). Run on macOS. cargo-bundle reads [package.metadata.bundle] in
# crates/iced/Cargo.toml and assembles the .icns from the PNG icon sources.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

command -v cargo-bundle >/dev/null 2>&1 || cargo install cargo-bundle --locked

echo "==> cargo bundle (.app)"
cargo bundle --release --package notepad-iced --format osx
APP="$ROOT/target/release/bundle/osx/Notepad Extra.app"
[ -d "$APP" ] || { echo "expected app bundle not found at: $APP" >&2; exit 1; }

VERSION="$(grep -m1 'version = ' "$ROOT/Cargo.toml" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+')"
DMG="$ROOT/target/release/bundle/osx/notepad-extra-${VERSION}-$(uname -m).dmg"
echo "==> hdiutil (.dmg)"
rm -f "$DMG"
hdiutil create -volname "Notepad Extra" -srcfolder "$APP" -ov -format UDZO "$DMG"
echo "==> dmg: $DMG"
