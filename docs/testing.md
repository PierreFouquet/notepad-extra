# Testing the native rewrite

Test infrastructure for the native (iced) rewrite, set up in **issue #27** as
Phase 0's final piece. It exists so the epic's (#25) Definition of Done is
*enforced*, not aspirational. Everything here targets the pure, UI-free
`notepad-core` crate (#28 groundwork): because `update(State, Message) -> Effect`
does no I/O and touches no window, every layer below runs with **no GPU and no
display**.

## Layers

| Layer | Where | Run it |
|-------|-------|--------|

| Unit tests | `#[cfg(test)]` in each `crates/core/src/*.rs` | `cargo test -p notepad-core` |
| Property tests (`proptest`) | `app.rs` `mod tests` | `cargo test -p notepad-core` |
| Adversarial / stress | `crates/core/tests/stress.rs` | `cargo test -p notepad-core --test stress` |
| Fuzzing (`cargo-fuzz`) | `crates/core/fuzz/` | see below (nightly) |
| Coverage gate | `scripts/coverage.sh` | `scripts/coverage.sh` |

CI wires all of these together in [`.github/workflows/native-ci.yml`](../.github/workflows/native-ci.yml).

## Property tests

`proptest` drives invariants that must hold for *every* input, e.g.:

- No message stream can empty the document list or push `active` out of bounds.
- Loading content then saving reproduces the original bytes exactly (either EOL).

## Adversarial / stress

Hundreds of open operations, million-line pastes, and rapid tab churn — asserting
the core stays panic-free and structurally sound. Catastrophic-regex cases join
this file once find/replace (#33) exists.

## Fuzzing

Fuzz targets live in `crates/core/fuzz/` and require the **nightly** toolchain
plus `cargo-fuzz`:

```sh
rustup toolchain install nightly
cargo install cargo-fuzz
cargo fuzz run --fuzz-dir crates/core/fuzz update_sequence
cargo fuzz run --fuzz-dir crates/core/fuzz text_helpers
```

- `update_sequence` — arbitrary `Message` streams through `update`; asserts the
  no-panic / in-bounds / non-empty invariants.
- `text_helpers` — EOL detection/join, `basename`, `extension_of`, language
  detection must never panic on any input.

The core stays dependency-free; the fuzz crate is a **detached** workspace so it
is never pulled into `cargo test`.

## Coverage gate

```sh
scripts/coverage.sh              # fails under the gate (default 98%)
COVERAGE_GATE=100 scripts/coverage.sh --html   # write an HTML report
```

Requires `cargo install cargo-llvm-cov`. The gate rises toward the ~100% DoD
target as the core grows; pure logic with no I/O has little excuse to miss it.

## Render shell (`crates/iced`)

The thin iced shell (#28) does no application logic of its own — it renders the
core's `State` and executes the `Effect`s it returns. Its wiring to the core is
tested **headlessly** (no window, no GPU) in `crates/iced` `mod tests`: driving
`Shell::update` with synthetic messages and asserting the core state and editor
buffer, e.g. typing marks the document dirty, switching tabs swaps the buffer, a
failed read surfaces an error without touching the docs.

The `view` needs a real renderer, so it is not unit-tested; instead the CI
`shell` job launches the built binary under `xvfb` (software renderer) and treats
a clean startup as success — proving the window actually comes up.

## Definition of Done — coverage map

| DoD requirement | Covered by |
|-----------------|------------|

| Unit tests on pure logic | per-module `mod tests` |
| Property-based tests (`proptest`) | `app.rs` proptests |
| Adversarial / stress | `tests/stress.rs` |
| Edge & error paths | `io.rs` (bad UTF-8, missing file), `text.rs` |
| Fuzz targets (`cargo-fuzz`) | `fuzz/fuzz_targets/*` |
| CI coverage gate (~100% logic) | `native-ci.yml` → `scripts/coverage.sh` |
| Render-shell wiring (headless) | `crates/iced` `mod tests` |
| Windowed launch under `xvfb` | `native-ci.yml` → `shell` job smoke |
| Packaged install/launch under `xvfb` | deferred — needs packaging (#43/#44) |
