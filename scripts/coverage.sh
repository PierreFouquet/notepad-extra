#!/usr/bin/env bash
# Coverage gate for the native-rewrite crates (issue #27).
#
# Runs the notepad-core suite under llvm instrumentation and fails if line
# coverage falls below the gate. The epic's Definition of Done targets ~100%
# logic coverage; the pure core has no I/O to excuse, so the bar is high.
#
# Requires: cargo-llvm-cov   ->  cargo install cargo-llvm-cov
# Override the gate with:    COVERAGE_GATE=95 scripts/coverage.sh
# Pass extra flags through:  scripts/coverage.sh --html
set -euo pipefail

GATE="${COVERAGE_GATE:-98}"

exec cargo llvm-cov \
    --package notepad-core \
    --fail-under-lines "$GATE" \
    "$@"
