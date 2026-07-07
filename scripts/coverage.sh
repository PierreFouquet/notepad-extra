#!/usr/bin/env bash
# Coverage gate for the native-rewrite logic crates (issue #27).
#
# Runs the pure-logic suites (notepad-core and notepad-syntax) under llvm
# instrumentation and fails if line coverage falls below the gate. The epic's
# Definition of Done targets ~100% logic coverage; these crates have no I/O to
# excuse, so the bar is high. The iced shell (notepad-iced) is view-heavy and
# smoke-tested, not gated here — both packages must be named explicitly so each
# one's own `#[cfg(test)]` suite runs (listing only core would compile
# notepad-syntax into the graph but never execute its tests, under-reporting it).
#
# Requires: cargo-llvm-cov   ->  cargo install cargo-llvm-cov
# Override the gate with:    COVERAGE_GATE=95 scripts/coverage.sh
# Pass extra flags through:  scripts/coverage.sh --html
set -euo pipefail

GATE="${COVERAGE_GATE:-98}"

exec cargo llvm-cov \
    --package notepad-core \
    --package notepad-syntax \
    --fail-under-lines "$GATE" \
    "$@"
