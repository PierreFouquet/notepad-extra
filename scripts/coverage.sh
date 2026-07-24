#!/usr/bin/env bash
# Coverage gate for the native-rewrite logic crates (issue #27).
#
# Runs the pure-logic suites (notepad-core and notepad-syntax) under llvm
# instrumentation and fails if line coverage falls below the gate — checked two
# ways so a single under-tested file can't hide behind a high average (#80):
#
#   * COVERAGE_GATE  (default 99) — the AGGREGATE line % across both crates, and
#   * PER_FILE_GATE  (default 99) — the floor every individual source file must
#     clear on its own.
#
# The epic's Definition of Done targets ~100% logic coverage; these crates have
# no I/O to excuse, so the bar is high. The iced shell (notepad-iced) is
# view-heavy and smoke-tested, not gated here — both packages must be named
# explicitly so each one's own `#[cfg(test)]` suite runs (listing only core would
# compile notepad-syntax into the graph but never execute its tests,
# under-reporting it).
#
# Requires: cargo-llvm-cov (cargo install cargo-llvm-cov) and jq.
# Override the gates with:  COVERAGE_GATE=95 PER_FILE_GATE=90 scripts/coverage.sh
# Pass extra flags through:  scripts/coverage.sh --html   (--summary-only is a
#     no-op here — this script prints its own per-file summary below.)
set -euo pipefail

GATE="${COVERAGE_GATE:-99}"
PER_FILE_GATE="${PER_FILE_GATE:-99}"
PKGS=(--package notepad-core --package notepad-syntax)

COV_JSON="$(mktemp)"
trap 'rm -f "$COV_JSON"' EXIT

# Resolve strictly against the committed Cargo.lock, mirroring the `--locked` on
# the workflow's clippy/build/test steps (#129). Without it this gate would
# silently update the lockfile mid-run and measure a dependency set that is not
# the one the rest of CI just tested.
LOCKED=(--locked)

# 1. Run both suites once under instrumentation, keeping the raw profile data so
#    the reports below don't re-run the tests.
cargo llvm-cov --no-report "${LOCKED[@]}" "${PKGS[@]}"

# 2. Forward any passthrough flags (e.g. --html) to a human report. `--summary-only`
#    (the CI default) is dropped: it can't combine with a plain text report, and
#    this script prints its own compact summary in step 4 regardless.
report_args=()
for arg in "$@"; do
    [[ "$arg" == "--summary-only" ]] || report_args+=("$arg")
done
if [[ ${#report_args[@]} -gt 0 ]]; then
    cargo llvm-cov report "${LOCKED[@]}" "${PKGS[@]}" "${report_args[@]}"
fi

# 3. Export the coverage data once as JSON; both gates are enforced from it.
cargo llvm-cov report "${LOCKED[@]}" "${PKGS[@]}" --json --output-path "$COV_JSON"

# 4. Print a compact per-file line-coverage table for the log.
jq -r '.data[0]
    | "line coverage: \(.totals.lines.percent * 100 | round / 100)% aggregate over \(.files | length) files",
      ( .files[]
        | "  \(.summary.lines.percent * 100 | round / 100)%\t\(.summary.lines.covered)/\(.summary.lines.count)\t\(.filename | sub(".*/notepad-extra/"; ""))" )
    ' "$COV_JSON" | { column -t -s $'\t' 2>/dev/null || cat; }

# 5. Guard against llvm-cov silently dropping a file: it omits any source file with
#    no executed regions, so an untested or undeclared module could pass the gate
#    unseen. Require every `.rs` under the gated crates that defines a `fn` to appear
#    in the report; declaration-only files (mod/use/type — e.g. notepad-core's
#    lib.rs) have nothing to cover and are legitimately absent.
reported=$(jq -r '.data[0].files[].filename' "$COV_JSON")
missing=()
while IFS= read -r src; do
    grep -qE '^[[:space:]]*[a-z()" ]*fn[[:space:]]' "$src" || continue
    abs="$(cd "$(dirname "$src")" && pwd)/$(basename "$src")"
    grep -Fxq "$abs" <<<"$reported" || missing+=("$src")
done < <(find crates/core/src crates/syntax/src -name '*.rs')
if [[ ${#missing[@]} -gt 0 ]]; then
    echo "coverage.sh: code-bearing file absent from coverage (untested or not compiled?):" >&2
    printf '  - %s\n' "${missing[@]}" >&2
    exit 1
fi

# 6. Collect any gate violations — the aggregate falling short, or any single file
#    below the per-file floor — and fail with the list if there are any.
offenders=$(jq -r --argjson agg "$GATE" --argjson each "$PER_FILE_GATE" '
    [ ( if .data[0].totals.lines.percent < $agg
        then "aggregate \(.data[0].totals.lines.percent * 100 | round / 100)% < \($agg)%"
        else empty end ),
      ( .data[0].files[]
        | select(.summary.lines.percent < $each)
        | "\(.filename | sub(".*/notepad-extra/"; "")) at \(.summary.lines.percent * 100 | round / 100)% < \($each)%" ) ]
    | .[]' "$COV_JSON")

if [[ -n "$offenders" ]]; then
    echo "coverage.sh: line coverage below gate:" >&2
    echo "$offenders" | sed 's/^/  - /' >&2
    exit 1
fi

echo "coverage.sh: OK — aggregate >= ${GATE}%, every file >= ${PER_FILE_GATE}%."
