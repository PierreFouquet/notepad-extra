#!/usr/bin/env bash
# Build the macOS .app (cargo-bundle) and wrap it in a compressed .dmg (issue
# #43). Run on macOS. cargo-bundle reads [package.metadata.bundle] in
# crates/iced/Cargo.toml and assembles the .icns from the PNG icon sources.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

# Pinned (#134): cargo-bundle 0.12 changed the `[package.metadata.bundle]`
# schema (the macOS keys moved under an `osx` table) and rejects the previous
# layout, so an unpinned install broke the dmg build the day it was published.
# Bump this together with the metadata in crates/iced/Cargo.toml.
CARGO_BUNDLE_VERSION="0.12.0"
if ! cargo bundle --version 2>/dev/null | grep -q "cargo-bundle ${CARGO_BUNDLE_VERSION}"; then
  cargo install cargo-bundle --version "${CARGO_BUNDLE_VERSION}" --locked
fi

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
