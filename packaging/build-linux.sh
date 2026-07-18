#!/usr/bin/env bash
# Build the native Linux packages for notepad-extra (issue #43). Resolves the
# workspace root itself, so it runs from anywhere. Shared by the packaging CI
# smoke jobs (#44) and the release workflow (#45) so both build identically.
#
# Usage: packaging/build-linux.sh [deb|rpm|appimage|all]   (default: all)
#
# Requires (installed by CI): cargo-deb, cargo-generate-rpm; AppImage also needs
# curl (fetches linuxdeploy on demand). The binary itself needs no system
# GUI-toolkit build deps — the whole point of the native rewrite (#25).
set -euo pipefail

FORMAT="${1:-all}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> building release binary (notepad-iced → notepad-extra)"
cargo build --release --package notepad-iced

build_deb() {
  echo "==> cargo deb"
  cargo deb --package notepad-iced --no-build
  ls -1 "$ROOT"/target/debian/*.deb
}

build_rpm() {
  echo "==> cargo generate-rpm"
  cargo generate-rpm -p crates/iced
  ls -1 "$ROOT"/target/generate-rpm/*.rpm
}

build_appimage() {
  echo "==> AppImage"
  bash "$ROOT/packaging/appimage/build-appimage.sh"
}

case "$FORMAT" in
  deb)      build_deb ;;
  rpm)      build_rpm ;;
  appimage) build_appimage ;;
  all)      build_deb; build_rpm; build_appimage ;;
  *) echo "unknown format '$FORMAT' (want: deb|rpm|appimage|all)" >&2; exit 2 ;;
esac
