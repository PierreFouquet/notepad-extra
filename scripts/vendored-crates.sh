#!/usr/bin/env bash
# Gates for the patched crates under vendor/ (#149).
#
# A crate substituted with `[patch.crates-io]` by a path copy loses two things
# crates.io gave it: the checksum in Cargo.lock, which pinned its exact
# contents, and its place in `cargo audit`, which only matches advisories
# against crates.io packages. This script puts both back.
#
#   check  For each vendor/<crate>/: download the crates.io release named in its
#          upstream.toml, verify the recorded sha256, apply notepad-extra.patch
#          and require the result to match the copy, file for file. An edit to
#          the copy that the patch does not record fails, and so does a patch
#          that no longer applies.
#   audit  `cargo audit` over a copy of Cargo.lock in which every vendored crate
#          keeps its crates.io source, so RustSec advisories for the release it
#          was made from still apply to it. Extra arguments go to cargo audit,
#          and `.cargo/audit.toml` applies as usual.
#
# Both need the network (crates.io / the advisory database).
#
# Usage:
#   scripts/vendored-crates.sh [check]
#   scripts/vendored-crates.sh audit [cargo-audit args...]
#
# Requires: curl, sha256sum, tar, patch (check); cargo-audit (audit).
set -euo pipefail

cd "$(dirname "$0")/.."
export LC_ALL=C

# Files in each vendor/<crate>/ that are ours, not the release's.
OURS=" VENDORED.md notepad-extra.patch upstream.toml "

field() { # field FILE KEY: the value of a `KEY = "value"` line
  sed -n "s/^$2 = \"\(.*\)\"\$/\1/p" "$1"
}

check_one() { # check_one DIR
  local dir=$1 name version sha tmp up entry base rc=0
  name=$(field "$dir/upstream.toml" crate)
  version=$(field "$dir/upstream.toml" version)
  sha=$(field "$dir/upstream.toml" sha256)
  if [ -z "$name" ] || [ -z "$version" ] || [ -z "$sha" ]; then
    echo "error: $dir/upstream.toml needs crate, version and sha256" >&2
    return 1
  fi
  tmp=$(mktemp -d)
  curl -fsSL "https://static.crates.io/crates/$name/$name-$version.crate" -o "$tmp/crate.tar.gz"
  if ! echo "$sha  $tmp/crate.tar.gz" | sha256sum -c --quiet - >/dev/null 2>&1; then
    echo "error: the downloaded $name $version does not match the sha256 in $dir/upstream.toml" >&2
    rm -rf "$tmp"
    return 1
  fi
  tar -xzf "$tmp/crate.tar.gz" -C "$tmp"
  up="$tmp/$name-$version"
  if ! patch -s -p1 -d "$up" --no-backup-if-mismatch < "$dir/notepad-extra.patch"; then
    echo "error: $dir/notepad-extra.patch does not apply to $name $version" >&2
    rm -rf "$tmp"
    return 1
  fi
  for entry in "$dir"/*; do
    base=${entry##*/}
    [[ "$OURS" == *" $base "* ]] && continue
    if ! diff -r "$up/$base" "$entry" >/dev/null 2>&1; then
      echo "error: $entry differs from $name $version + notepad-extra.patch:" >&2
      diff -ru "$up/$base" "$entry" 2>&1 | head -40 >&2 || true
      rc=1
    fi
  done
  rm -rf "$tmp"
  [ $rc -eq 0 ] && echo "$dir: matches $name $version + notepad-extra.patch"
  return $rc
}

do_check() {
  local dir rc=0 found=0
  for dir in vendor/*/; do
    dir=${dir%/}
    [ -f "$dir/upstream.toml" ] || { echo "error: $dir has no upstream.toml" >&2; rc=1; continue; }
    found=1
    check_one "$dir" || rc=1
  done
  [ $found -eq 1 ] || echo "no vendored crates"
  return $rc
}

do_audit() {
  local tmp names
  command -v cargo-audit >/dev/null || { echo "error: cargo-audit is required" >&2; exit 2; }
  names=$(for f in vendor/*/upstream.toml; do [ -f "$f" ] && field "$f" crate; done)
  tmp=$(mktemp -d)
  # Give each vendored package, which has no `source` line, the crates.io one.
  # A crates.io package of the same name already has a `source` line after its
  # version and is left alone.
  awk -v names="$names" '
    BEGIN {
      n = split(names, list, "\n")
      for (i = 1; i <= n; i++) if (list[i] != "") want[list[i]] = 1
      src = "source = \"registry+https://github.com/rust-lang/crates.io-index\""
    }
    {
      if (pending && $0 !~ /^source = /) print src
      pending = 0
      print
    }
    /^\[\[package\]\]$/ { pkg = "" }
    /^name = "/ { pkg = $3; gsub(/"/, "", pkg) }
    /^version = "/ && (pkg in want) { pending = 1 }
  ' Cargo.lock > "$tmp/Cargo.lock"
  local rc=0
  cargo audit --file "$tmp/Cargo.lock" "$@" || rc=$?
  rm -rf "$tmp"
  return $rc
}

case "${1:-check}" in
  check) do_check ;;
  audit) shift; do_audit "$@" ;;
  *) echo "usage: $0 [check | audit [cargo-audit args...]]" >&2; exit 2 ;;
esac
