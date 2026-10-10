# Testing the native rewrite

Test infrastructure for the native (iced) rewrite, set up in **issue #27** as
Phase 0's final piece. It exists so the epic's (#25) Definition of Done is
*enforced*, not aspirational. Everything here targets the pure, UI-free
`notepad-core` crate (#28 groundwork): because `update(State, Message) -> Effect`
does no I/O and touches no window, every layer below runs with **no GPU and no
display**.

## Layers

| Layer | Where | Run it |
| --- | --- | --- |
| Unit tests | `#[cfg(test)]` in each `crates/core/src/*.rs` | `cargo test -p notepad-core` |
| Property tests (`proptest`) | per-module `mod tests` in `crates/core/src/*.rs` and `crates/syntax/src/lib.rs` | `cargo test -p notepad-core -p notepad-syntax` |
| Adversarial / stress | `crates/core/tests/stress.rs` | `cargo test -p notepad-core --test stress` |
| Fuzzing (`cargo-fuzz`) | `crates/core/fuzz/` | see below (nightly) |
| Coverage gate | `scripts/coverage.sh` | `scripts/coverage.sh` |

CI wires all of these together in [`.github/workflows/native-ci.yml`](../.github/workflows/native-ci.yml).
The RustSec advisory scan (`cargo audit`, #129) runs from its own
[`.github/workflows/audit.yml`](../.github/workflows/audit.yml) on every PR *and*
weekly — kept separate (#133) because GitHub disables a workflow that carries a
`schedule` after 60 days of repository inactivity, and that must never take the
PR gates down with it.

## Property tests

`proptest` drives invariants that must hold for *every* input, e.g.:

- No message stream can empty the document list or push `active` out of bounds.
- Loading content then saving reproduces the original bytes exactly (either EOL).
- The `Document`'s cached character / word / line totals, line-start index and
  dirty flag (#126) equal a from-scratch recount after **every** message path
  that rewrites a buffer — edit, undo/redo, replace-next/all, text operations,
  load, reload, save, encoding change — over a "seam" alphabet (newlines,
  apostrophes, CJK, emoji, regional indicators, joiners, combining marks); and
  the indexed `(line, column)` conversions agree with the pure `find::*`
  helpers for any text and any (even out-of-range or mid-glyph) arguments
  (`document.rs` `mod tests`).

## Adversarial / stress

Hundreds of open operations, million-line pastes, and rapid tab churn — asserting
the core stays panic-free and structurally sound, including the catastrophic-regex
guard (`catastrophic_regex_pattern_stays_linear`: the linear-time `regex` engine is
the guard, #63) and the #126 cost pins below.

It also pins the per-frame cost the status bar depends on (#126): ten thousand
`status()` + `dirty()` calls on a 200k-line document must finish inside a loose
wall-clock bound that the old recount-per-call code would miss by orders of
magnitude, so a full recount cannot silently return to the redraw path.

## Fuzzing

Fuzz targets live in `crates/core/fuzz/` and require the **nightly** toolchain
plus `cargo-fuzz`:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
for target in $(cargo fuzz list --fuzz-dir crates/core/fuzz); do   # the same loop CI runs (#98)
  cargo fuzz run --fuzz-dir crates/core/fuzz "$target" -- -max_total_time=15
done
```

- `update_sequence` — arbitrary `Message` streams through `update`; asserts the
  no-panic / in-bounds / non-empty invariants.
- `text_helpers` — EOL detection/join, `basename`, `extension_of`, language
  detection must never panic on any input.
- `edit_diff` — for any pair of strings, `diff` yields an edit that applies and
  inverts back, never splitting a UTF-8 char (#30; now the blockwise scans, #126).
- `find_search` — any pattern + options + text + replacement: compiling never
  panics, searching and replacing stay in bounds (#33).
- `prefs_parse` — `Preferences::from_json` on arbitrary bytes never panics (#38).
- `brackets_match` — `brackets::match_at` on any text + offset returns only valid,
  in-bounds pairs (#41).
- `encoding_decode` — `decode` and `decode_with` on arbitrary bytes never panic or
  error (#50, #59, #103).

The core's few dependencies are all pure Rust; the fuzz crate is a **detached**
workspace so it is never pulled into `cargo test`.

Being detached, it carries its **own committed `Cargo.lock`** — the root one does
not cover it. `cargo fuzz` accepts no `--locked` flag and forwards nothing to
cargo, so CI asserts the lockfile separately, with
`cargo metadata --locked --manifest-path crates/core/fuzz/Cargo.toml` ahead of the
build; without that the fuzz job would silently re-resolve its dependencies on
every run. Because `notepad-core` and `notepad-syntax` are path dependencies here,
changing *their* dependencies makes this lockfile stale too, and CI will fail
until it is regenerated:

```sh
cargo generate-lockfile --manifest-path crates/core/fuzz/Cargo.toml
```

## Coverage gate

```sh
scripts/coverage.sh              # fails under the gate (default 99%)
COVERAGE_GATE=100 scripts/coverage.sh --html   # write an HTML report
```

Requires `cargo install cargo-llvm-cov`. The gate rises toward the ~100% DoD
target as the core grows; pure logic with no I/O has little excuse to miss it.

The gate must not depend on what the property tests happen to draw (#142). Run
it once more with a single proptest case:

```sh
PROPTEST_CASES=1 scripts/coverage.sh   # must pass too: no line may rely on a lucky draw
```

CI runs this step too, with `PROPTEST_RNG_SEED=1` pinned so that its result
reproduces locally and cannot flake. Without the variable the seed is random, so
repeated local runs try other draws.

If that fails, or a file's margin shrinks, find the lines no deterministic test
reaches. Run with no proptest cases at all and read the report:

```sh
PROPTEST_CASES=0 scripts/coverage.sh --text --output-path target/cov.txt   # then look for 0 counts
```

The gate may fail at zero cases. That is expected while a `mod tests` still
holds helpers that only a proptest calls. Look at every line counted 0:

- **A production line**, above `mod tests`, needs a deterministic unit test:
  a synthetic `Message` asserted against `State` and the returned `Effect`s.
- **A test-module line that only a proptest reaches**, such as a "reference"
  oracle, needs one too: call it from a plain `#[test]`.
- **A strategy closure** is reached only when its branch is drawn. Sample the
  strategy with proptest's fixed-seed `TestRunner::deterministic()` and assert
  that every branch comes up, as `the_random_message_stream_reaches_every_kind_of_message`
  does for `app.rs`.

The script clears the previous run's profile data first, so a run never reports
lines that only an earlier run reached.

## Render shell (`crates/iced`)

The thin iced shell (#28) does no application logic of its own — it renders the
core's `State` and executes the `Effect`s it returns. Its wiring to the core is
tested **headlessly** (no window, no GPU) in `crates/iced` `mod tests`: driving
`Shell::update` with synthetic messages and asserting the core state and editor
buffer, e.g. typing marks the document dirty, switching tabs swaps the buffer, a
failed read surfaces an error without touching the docs.

`view` is exercised too, and still headlessly: `mod ui_tests` (#70) uses iced
0.14's [`iced_test`] `Simulator`, which builds the **real widget tree** with the
tiny-skia software renderer (no window, no GPU), finds widgets by their visible
text, synthesises clicks / keystrokes, and returns the `Message`s they emit —
which the tests feed back through `Shell::update`. Each is written to fail if its
widget is missing or mis-wired: clicking **New** opens a second tab, the in-tab
`×` closes a clean tab, **Find** reveals the find bar, **About** opens the modal,
and typing marks the document dirty. Selectors match on text, so these run
unchanged on all three CI OSes with no fonts and no GPU.

The CI `shell` job additionally launches the built binary under `xvfb` (software
renderer) and treats a clean startup as success — the complementary "the real
window actually comes up" check that the headless Simulator, by design, does not
make.

[`iced_test`]: https://docs.rs/iced_test

## Definition of Done — coverage map

| DoD requirement | Covered by |
| --- | --- |
| Unit tests on pure logic | per-module `mod tests` |
| Property-based tests (`proptest`) | per-module proptests across the core and syntax crates |
| Adversarial / stress | `tests/stress.rs` |
| Edge & error paths | `io.rs` (bad UTF-8, missing file), `text.rs` |
| Fuzz targets (`cargo-fuzz`) | `fuzz/fuzz_targets/*` |
| CI coverage gate (~100% logic) | `native-ci.yml` → `scripts/coverage.sh` |
| Render-shell wiring (headless) | `crates/iced` `mod tests` |
| Render-shell `view` interaction (headless) | `crates/iced` `mod ui_tests` (`iced_test` Simulator) |
| Windowed launch under `xvfb` | `native-ci.yml` → `shell` job smoke |
| Packaged install/launch under `xvfb` | deferred — needs packaging (#43/#44) |
