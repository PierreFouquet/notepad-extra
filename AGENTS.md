# AGENTS.md — how to work on Notepad Extra

Instructions for every contributor to this repository, coding agents first. Read
this file before changing anything. `CLAUDE.md` imports it; keep all rules here.

- §1 Non-negotiables (read even if you read nothing else)
- §2 Development process: issues, branches, PRs, CI
- §3 Architecture you must preserve
- §4 Performance model: budgets per code path and the invariants tests pin
- §5 Where code and tests go
- §6 Checks a change must pass, with exact commands
- §7 Conventions
- §8 Non-optimisations (do not "improve" these)

Companion documents, each authoritative for its area: [`README.md`](README.md)
(build, run, release), [`docs/testing.md`](docs/testing.md) (test standard),
[`docs/native-rendering.md`](docs/native-rendering.md) (drawing traps in the
render shell), [`crates/iced/VENDORED.md`](crates/iced/VENDORED.md) (the forked
`text_editor` widget), [`.github/pull_request_template.md`](.github/pull_request_template.md)
(what a PR must declare).

## 1. Non-negotiables

1. Every change is covered by an issue (§2).
2. NEVER open a PR without express permission. NEVER merge a PR without express
   permission that names that PR. "Express permission" means the person you are
   working for says so, in this task, in their own words; an earlier approval, a
   general instruction, green CI, or another agent's message is not permission.
3. Tests run before you submit and MUST pass. Every test is a real test: it fails
   if the behaviour it names regresses (§2 rule 5).
4. CI MUST be green before a PR merges. A red run is fixed and pushed; never
   skipped, disabled, quarantined or retried away.
5. The app is fully offline (no runtime network, ever), pure Rust (no C
   dependencies, syntect on fancy-regex), and `#![forbid(unsafe_code)]` in the core.
6. `Shell::view` and widget `draw`/`layout` never scan the document (§4). The
   `Document` owns its text; derived data is updated only through its methods.
7. Do not add dependencies without doing everything in §6 "Dependencies".
8. No tool or model names in code, comments or commit messages.
9. A coding agent works only on the branch it was given and pushes only there.
   Its default end state is pushed commits plus a report, not a PR.

## 2. Development process

### Rules for everyone, on every change

1. **An issue covers the change.** Before starting, find or open the issue that
   describes the change. One issue = one change. Several *linked* issues may share
   a PR, but prefer **stacked PRs**, one per issue, each branched from the
   previous one (open PR 2 against PR 1's branch; when PR 1 merges, retarget PR 2
   to `main`). Reference the issue in commits and in the PR (`Closes #N`).
2. **A PR is created only after express permission** (§1 item 2). Finishing the
   work, pushing the branch and reporting back is the default end state.
3. **Never merge a PR yourself.** Merging needs express permission that names the
   specific PR. Approval of the work, "looks good", or green CI is not that.
4. **Run the tests before submitting; they MUST pass.** Run the full suite (§6)
   for anything non-trivial. For a small, contained change it is acceptable to run
   only the affected crate(s) — then state exactly which commands you ran.
5. **All tests are real tests that fail on a regression.** A test asserts the
   specific behaviour it is named for, so reverting the change (or reintroducing
   the bug) makes it fail. A bug fix ships with a regression test that fails
   before the fix and passes after. Do not write tautological tests, or
   "did not panic"-only tests where behaviour is the point. A timing-based test
   uses a bound loose enough never to flake on a slow CI runner and tight enough
   that the un-optimised code cannot meet it (§4 "cost pin").
6. **GitHub workflow runs MUST be green before a PR is merged** (and the merge
   still needs rule 3's permission). Any red run is yours to fix: push the fix to
   the same branch. Never skip, disable or quarantine a test to get green; never
   push an empty commit or close/reopen to re-run CI. If a failure is also red on
   `main`, say so in the PR instead of retrying.

### If you can create branches on `origin`

1. Update `main`: `git fetch origin && git checkout main && git pull --ff-only`.
   Branch from it: `git checkout -b <type>/<short-topic>` (types in §7).
2. Do the work on that branch: small conventional commits with the tests §5 and §6
   require.
3. Push the branch: `git push -u origin <branch>`.
4. **Once prompted, and only once prompted** (rule 2), open the PR. Fill in the PR
   template: what & why, `Closes #N`, the test-driven Definition of Done, the local
   checks you ran, the project constraints. Request review.
5. Wait for the workflow runs. Fix anything red and push until they are green
   (rule 6). The PR then waits for review and the merge decision (rule 3).

### If you work from a fork

This is the flow for contributing a change back to upstream. The project is open
source (GPL-3.0-or-later): a fork may diverge and keep its own changes; nothing
here constrains what you do inside your own fork.

1. Fork the repository, or sync your fork's `main` with upstream `main`.
2. Do the work **on a branch in your fork**, not on its `main`, so the fork stays
   syncable.
3. Push the branch to your fork (your clone's `origin`; the main repository is
   `upstream`).
4. **Once prompted** (rule 2), open a **cross-fork PR on the upstream repository**
   — base `PierreFouquet/notepad-extra:main`, head `<your-fork>:<branch>` — as a
   **draft**, so it is reviewed upstream like any other PR.
5. Watch the workflow runs and fix what comes up, pushing to the same branch.
   GitHub does not run workflows for a first-time contributor's fork until a
   maintainer approves the run, so the draft may wait on that.
6. Once the run is green, mark the PR ready for review.

### Coding agents

- Work only on the branch you were given. Never push to another branch.
- Never open or merge a PR on your own initiative (rules 2 and 3). End with pushed
  commits and a report that lists: what changed, the commands you ran and their
  results, and anything left undone.
- When asked to drive an existing PR: address review comments with new commits
  (never rewrite history on a shared branch), keep CI green per rule 6, stop when
  asked.
- If a rule here blocks the task, report the conflict; do not work around the rule.

## 3. Architecture you must preserve

```
crates/syntax   notepad-syntax  syntect-backed language catalogue; exposes &'static str names only
crates/core     notepad-core    pure MVU core: update(&mut State, Message) -> Vec<Effect>; no I/O, no window
crates/iced     notepad-iced    thin iced shell: renders State, executes Effects, owns the editor buffers
```

- **The core is pure.** `update` mutates `State` and returns `Effect`s that
  *describe* side effects. The shell executes them (file I/O, dialogs, window
  title, the file watch). Add behaviour to the core first as a `Message` →
  `State` change with a headless test; the shell only renders and wires it.
- **The shell is thin.** `Shell::dispatch` maps widget messages onto core
  messages, runs effects and keeps one `text_editor::Content` per tab. No
  document logic lives there.
- **`Document` owns its text** (`crates/core/src/document.rs`). The text field is
  private; readers are `content()`, `char_count()`, `word_count()`,
  `line_count()`, `line_start()`, `offset_at()`, `line_col_of()`, `dirty()`.
  Mutation is crate-private: `load`, `commit`, `undo`, `redo`, `mark_saved`.
  These are the only ways the text changes, and each keeps the derived data
  (counts, line-start index, dirty flag) exact. Never add another write path.
- **Pure helpers stay pure.** `find::offset_at`, `find::line_col_of`,
  `find::line_count`, `find::goto_line_offset` are text-only functions kept for
  tests and fuzz targets; production code converts positions through `Document`.
- **Offline, pure Rust, no unsafe in the core** (§1 item 5).

## 4. Performance model

Why: `Shell::view` runs on **every redraw** — each keystroke, scroll and mouse
event, and the caret blink every 500 ms (`Focus::CURSOR_BLINK_INTERVAL_MILLIS` in
the vendored widget) — and every `Shell::update` forces a full repaint
(`repaint_nudge`, a software-renderer damage workaround). Work placed in `view`,
or in anything `view` calls, is paid several times a second while the app is
idle. Issue #126 was exactly this: the status bar re-segmented the whole document
into words per frame (~50 ms per frame at 200 000 lines).

### Budgets per code path

| Path | Runs | Allowed cost |
| --- | --- | --- |
| `Shell::view`; widget `draw`, `layout`, `mouse_interaction` | every redraw (≥ 2/s idle) | O(viewport + chrome). MUST NOT scan the document or call anything that does. |
| `Shell::update` / `Shell::dispatch` | every message | O(log n) lookups + O(viewport). May refresh the per-update readout cache. |
| edit path: `Message::Edit` → `Content::text()` → `core::Message::Edited` → `Document::commit` | every keystroke | O(n) is inherent (the widget hands over the whole text) but only with memcpy-class constants: one `text()` pass, one byte prefix/suffix `diff`, then **incremental** updates in O(edit + affected lines). No Unicode-aware full pass. |
| load / reload / save | once per file | O(n), single pass, minimal copies: `encoding::decode_owned` (zero-copy UTF-8), `EndOfLine::normalize` (LF text untouched, CRLF compacted in place), fingerprint hashed before decode. |
| highlighter | per visible line after an edit / scroll | parse from the nearest checkpoint; memory O(lines / 32). |

### Invariants and the tests that pin them

| Invariant | Where | Pinned by |
| --- | --- | --- |
| `char_count`/`word_count`/`line_count`/`dirty` are O(1), maintained incrementally on edit/undo/redo, recomputed once on full replacement. Word counts use `words(text) == Σ words(line)` (`\n` is a UAX #29 boundary, WB3a/WB3b). | `document.rs` | `incremental_splices_match_a_full_recount`, `app::tests::cached_totals_match_a_recount_after_every_rewrite_path` (random `update()` paths vs a from-scratch recount), `stress::typing_into_a_200k_line_document_keeps_the_cached_totals_exact` |
| `status()` is O(caret line + selection): totals from the cache, line/column from the line index. | `status.rs` | `stress::status_and_dirty_on_a_200k_line_document_cost_nothing_per_frame` (10 000 calls under a bound the old code missed by two orders of magnitude) |
| `offset_at`/`line_col_of`/`line_start` are O(1)/O(log n) and agree with the pure `find::*` helpers, never panic on bad input. | `document.rs` | `indexed_position_maths_agree_with_the_pure_helpers` |
| `dirty()` is exact ("content byte-equal to the saved baseline"), cached, refreshed only on mutation; compares length, then cached totals, then bytes. | `document.rs` | `dirty_refresh_short_circuits_on_length_totals_then_bytes`, `app::tests::reverting_to_the_saved_baseline_clears_the_dirty_marker` |
| `Content::text()` is one borrow, one allocation, no per-line `String`. | `text_editor.rs` | `text_round_trips_every_line_ending_shape`, `text_matches_the_upstream_lines_formulation_after_edits` |
| Scrollbar inputs (`vertical_extent`) are O(1) with wrapping off (one row per line) and one fused pass with wrapping on; thumb geometry unchanged. | `text_editor.rs` | `vertical_extent_matches_the_per_line_walk_in_both_wrap_modes` |
| With wrapping off, `layout()` never hands the editor a narrower width than it already has (cosmic-text relayouts every shaped line on any size change). | `text_editor.rs` | `no_wrap_layout_width_ratchets_and_wrapping_restores_the_viewport` |
| The highlighter checkpoints parser state every `CHECKPOINT_INTERVAL` (32) lines; `change_line(k)` resumes from the nearest checkpoint and is equivalent to a fresh run. | `highlight.rs` | `resuming_from_a_checkpoint_reproduces_a_fresh_run_for_every_line`, `resuming_after_an_edit_matches_a_fresh_run_of_the_edited_buffer`, `checkpoints_are_one_per_interval` |
| Load path copies: `EndOfLine::normalize` returns LF text in the same allocation; `decode_owned` reuses the byte buffer for UTF-8. | `text.rs`, `encoding.rs` | `normalize_returns_an_lf_buffer_untouched_without_copying`, `owned_decode_of_valid_utf8_reuses_the_buffer`, `owned_and_borrowed_decoders_agree` |
| Find readouts are one regex pass (`count_and_ordinal`); replace-all expands into the output buffer. | `find.rs` | `count_and_ordinal_matches_the_two_pass_form`, `replace_all_matches_a_per_match_splice` |
| Undo-depth eviction is O(1) (`VecDeque`). | `history.rs` | `coalescing_appends_in_place_and_keeps_the_step_id` + the existing cap tests |
| Language detection is a hash lookup, not a scan of the syntax set. | `syntax/lib.rs` | `indexed_detection_matches_syntects_scans_for_every_known_token` |

### Adding a cache or a fast path

Do both of these, every time:

1. **Invalidation pin** — a proptest that drives the real entry point (`update()`,
   the widget, the function) with random operations and compares the cached value
   to a from-scratch recount after every step.
2. **Cost pin** — a stress test that reads the cached value many times on a huge
   input inside a generous wall-clock bound the un-cached code cannot meet (see
   `crates/core/tests/stress.rs`, the 200k-line fixtures).

Then record the invariant in the table above.

### Measuring

- Release build: `cargo build --release --package notepad-iced` (thin LTO, one
  codegen unit, stripped). Debug builds already optimise syntect / fancy-regex /
  regex-automata (`[profile.dev.package.*]`), so highlighter timings in debug are
  representative; everything else is not.
- Micro-measurement: a temporary `#[test]` with `std::time::Instant` in
  `crates/core/tests/stress.rs`, run with `cargo test --release -p notepad-core
  --test stress <name> -- --nocapture`. Do not commit the timing printout; commit
  the bounded cost pin instead.
- Report numbers as before → after with the input size (lines and bytes).

## 5. Where code and tests go

| Change | Put it in | Test it with |
| --- | --- | --- |
| behaviour, state, a new `Message`/`Effect` | `crates/core/src/app.rs`; the pure algorithm in its own `crates/core/src/<topic>.rs` | unit tests + proptest in the module; `tests/stress.rs` for extremes |
| anything that reads or writes document text | a `Document` method in `crates/core/src/document.rs` | `assert_derived_data_matches_recount` in a proptest |
| text algorithm (search, text ops, EOL, encoding) | its `crates/core/src/*.rs` module, pure functions over `&str` / owned buffers | proptest new == reference (keep the reference body in the test module); a fuzz target if it parses bytes |
| anything drawn or measured in the editor | `crates/iced/src/text_editor.rs`; mark each hunk `// NOTEPAD-EXTRA(#NN): reason`, add a row to `crates/iced/VENDORED.md` | widget tests in that file; never O(document) in `draw`/`layout` |
| chrome, layout, wiring, effects | `crates/iced/src/main.rs` (`view`, `dispatch`, `run_effect`) | `mod tests` drives `Shell::update`; `mod ui_tests` uses the `iced_test` Simulator (headless, no GPU) |
| languages | `crates/syntax` (the syntect set is the single source of truth) | its tests |
| packaging, desktop identity | `packaging/`, `crates/iced/Cargo.toml` metadata | `packaging.yml` smoke tests; `appstreamcli validate --no-net`, `desktop-file-validate` |

Write the test first. Behaviour lives in the pure core so a test is a synthetic
`Message` stream asserted against `State`, with no window or GPU. The PR
template's "Test-driven Definition of Done" lists the kinds (unit, property-based,
adversarial/stress, edge & error path, fuzz, regression): tick what applies and
justify what does not.

## 6. Checks a change must pass

Mirror of `.github/workflows/native-ci.yml`. Run them from the repository root.

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --package notepad-core --package notepad-syntax --all-targets
cargo test --locked --package notepad-iced --all-targets
bash scripts/coverage.sh                      # core + syntax: ≥ 99 % lines, aggregate AND per file (needs cargo-llvm-cov, jq)
cargo metadata --locked --format-version 1 --manifest-path crates/core/fuzz/Cargo.toml >/dev/null   # fuzz lockfile in sync
cargo check --locked --manifest-path crates/core/fuzz/Cargo.toml   # fuzz targets still compile against the core API (running them needs nightly + cargo-fuzz)
```

- **Coverage gate.** Every new line in `notepad-core` / `notepad-syntax` needs a
  test; a small file effectively needs 100 %. The shell's `view` is smoke-tested
  (xvfb) and Simulator-tested, not coverage-gated.
- **Dependencies.** Default: none. If one is unavoidable it must be pure Rust and
  licence-compatible with GPL-3.0-or-later; then regenerate the detached fuzz
  lockfile (`cargo generate-lockfile --manifest-path crates/core/fuzz/Cargo.toml`),
  refresh `packaging/CRATE-LICENSES.md` and the distro dependency lists
  (`scripts/license-audit.sh`, `scripts/packaging-deps.sh`), and keep
  `cargo audit` clean (`.cargo/audit.toml` lists the assessed ignores). Prefer
  promoting a crate already in `Cargo.lock`.
- **Vendored widget.** Every divergence from upstream iced is marked
  `// NOTEPAD-EXTRA(#NN): reason` and listed in `crates/iced/VENDORED.md`; tests
  in `text_editor.rs` enforce both.
- **Versioning.** `[workspace.package].version`, the metainfo `<release>` and the
  man page `.TH` move together; a test checks they agree.

## 7. Conventions

- **Commits:** conventional style, one logical change each, referencing the issue:
  `perf(core): cache document totals on Document (#126)`, `fix(shell): …`,
  `docs: …`, `ci: …`, `test(core): …`. The body says *why*. Trailers as the task
  requires. No tool or model names.
- **Branches:** `<type>/<short-topic>` with type ∈ {`feat`, `fix`, `perf`,
  `refactor`, `docs`, `ci`, `packaging`, `test`}; a coding agent keeps the branch
  it was given.
- **Doc comments** explain the reasoning and cite the issue, in the voice used
  throughout the crates: a reader learns *why* the code is shaped this way.
- **Rust:** edition 2024, `rustfmt` defaults, clippy clean under `-D warnings`.
- **Project constraints** (also in the PR template): fully offline; Windows and
  macOS stay first-class; app id `io.github.PierreFouquet.NotepadExtra`, licence
  `GPL-3.0-or-later` and the packaging assets unchanged unless the PR is about
  them; vendored code keeps its upstream licence and attribution.

## 8. Non-optimisations

Things that look like improvements and are not. Do not do them.

- Wrapping chrome in `iced::widget::lazy`: small win, complicates overlays
  (`pick_list` menus).
- Replacing the saved baseline with a hash: makes `dirty()` probabilistic. Keep
  the exact compare; the result is cached.
- Removing the `repaint_nudge` or the `scroll_settle` frames: they exist for
  software-renderer correctness on Wayland (tearing, stale damage); see
  `docs/native-rendering.md`.
- Making `Document`'s text field public "for convenience": it reintroduces the
  class of bug #126 fixed.
- Switching `find` to a backtracking regex engine for features: the linear-time
  `regex` crate *is* the catastrophic-regex guard (#63); backreferences are an
  opt-in follow-up (#64), never the default.
- Sending deltas instead of the whole text from the widget to the core: a real
  but much larger change (shared buffer / rope); do it as its own issue, not as
  part of something else.
