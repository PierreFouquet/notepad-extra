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
`text_editor` widget), [`vendor/cosmic-text/VENDORED.md`](vendor/cosmic-text/VENDORED.md)
(the patched cosmic-text crate), [`.github/pull_request_template.md`](.github/pull_request_template.md)
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
  tests, fuzz targets and the `Document`'s own from-scratch recount and index
  sizing at load; production code converts positions through `Document`.
- **Offline, pure Rust, no unsafe in the core** (§1 item 5).

## 4. Performance model

Why: `Shell::view` is rebuilt after **every message batch** — each keystroke,
scroll step and pointer movement over the editor (pointer motion is a message,
the shell tracks it for the context menu) — and every `Shell::update` other than
pointer tracking (`EditorCursorMoved`) and `WindowResized` forces a full repaint
(`repaint_nudge`, a software-renderer damage workaround). The widgets'
`draw`/`layout` run on **every redraw**, which includes the caret blink
every 500 ms (`Focus::CURSOR_BLINK_INTERVAL_MILLIS` in the vendored widget) even
while the app is otherwise idle. Work placed in `view`, or in anything `view`
calls, is paid on every mouse movement; work in `draw`/`layout` is paid twice a
second forever. Issue #126 was exactly this: the status bar re-segmented the
whole document into words on every `view` (~50 ms at 200 000 lines).

### Budgets per code path

| Path | Runs | Allowed cost |
| --- | --- | --- |
| `Shell::view`; widget `draw`, `layout`, `mouse_interaction` | `view`: every message batch; `draw` / `layout` / `mouse_interaction`: every redraw (≥ 2/s idle) | O(viewport + chrome). MUST NOT scan the document or call anything that does. |
| `Shell::update` / `Shell::dispatch` | every message | O(log n) lookups + O(viewport), plus the per-update readout refresh: O(selection) for the status bar and O(distance to the bracket partner) for bracket matching — O(document) for an unmatched opener, once per message, never per frame. |
| edit path: `Message::Edit` → `Content::text()` → `core::Message::Edited` → `Document::commit` | every keystroke | O(n) is inherent (the widget hands over the whole text) but only with memcpy-class constants: one `text()` pass, one byte prefix/suffix `diff`, then **incremental** updates in O(edit + affected lines + the line starts after the edit — a memcpy-class shift of the index tail, 1.6 MB per keystroke at the top of 200k lines). No Unicode-aware full pass. |
| load / reload / save | once per file | O(n), single pass, minimal copies: `encoding::decode_owned` (zero-copy UTF-8), `EndOfLine::normalize` (LF text untouched, CRLF compacted in place), fingerprint hashed before decode. |
| highlighter | per visible line after an edit / scroll | parse from the nearest checkpoint; memory O(lines / 32). Exceptions tracked below: a forward jump past unparsed lines re-parses every skipped line (#138), and the state is per widget, not per tab (#144). |

### Invariants and the tests that pin them

| Invariant | Where | Pinned by |
| --- | --- | --- |
| `char_count`/`word_count`/`line_count`/`dirty` are O(1), maintained incrementally on edit/undo/redo, recomputed once on full replacement. Word counts use `words(text) == Σ words(line)`: no UAX #29 word-like segment contains a `\n` (only `\n` and `\r\n` do — WB3 keeps CR LF together — and neither holds a word character), and segmentation restarts after a `\n` as at the start of text. | `document.rs` | `incremental_splices_match_a_full_recount`, `app::tests::cached_totals_match_a_recount_after_every_rewrite_path` (random `update()` paths vs a from-scratch recount), `stress::typing_into_a_200k_line_document_keeps_the_cached_totals_exact` |
| `status()` is O(caret line + selection): totals from the cache, line/column from the line index. | `status.rs` | `stress::status_and_dirty_on_a_200k_line_document_cost_nothing_per_frame` (10 000 calls under a bound the old code missed by two orders of magnitude) |
| `offset_at`/`line_col_of`/`line_start` are O(1)/O(log n) and agree with the pure `find::*` helpers, never panic on bad input. | `document.rs` | `indexed_position_maths_agree_with_the_pure_helpers` |
| `dirty()` is exact ("content byte-equal to the saved baseline"), cached, refreshed only on mutation; compares length, then cached totals, then bytes. | `document.rs` | `dirty_refresh_short_circuits_on_length_totals_then_bytes`, `app::tests::reverting_to_the_saved_baseline_clears_the_dirty_marker` |
| The status-bar and bracket-match readouts (`Shell::readouts`) are computed once per `Shell::update` — every message except pointer tracking (`EditorCursorMoved`) and `WindowResized`, which skip the repaint nudge too, and the scroll messages (`ScrollNudge`, `ScrollSettle`, the widget's `Scroll` action), which repaint but cannot move the caret — and only *read* by `view`. | `main.rs` | `pointer_tracking_and_resizes_do_not_recompute_the_readouts`, `readouts_follow_caret_moves_edits_and_undo`, `readouts_follow_a_tab_switch`, `readouts_follow_a_find_reveal`, `readouts_follow_language_and_encoding_changes` |
| The two `iced::Theme`s are built once per `ThemeMode` (`app_theme`) and handed out as `Arc` clones. | `main.rs` | `the_iced_theme_is_built_once_per_mode_and_shared_across_frames` |
| `view` splits the tab strip once; clean tab titles are borrowed, only a dirty tab's "•" name allocates. | `main.rs` | `tab_names_borrow_clean_titles_and_allocate_only_the_dirty_marker` |
| `sync_editor_cache` reconciles buffers only when the editor keys drift from the documents; the watched-file set is rebuilt only when a tab's path changes. | `main.rs` | `the_editor_cache_reconciles_whenever_its_keys_drift_from_the_docs`, `the_watched_set_is_shared_until_a_tab_changes_it` |
| `highlight::Settings.syntax` is a `&'static str` borrowed from the syntax set, so the per-frame `Settings` the widget compares and clones is a pointer copy. | `highlight.rs` | the `Settings` type is `Copy`; `theme_mode_changes_the_palette` |
| `Content::text()` is one borrow, one allocation, no per-line `String`. | `text_editor.rs` | `text_round_trips_every_line_ending_shape`, `text_matches_the_upstream_lines_formulation_after_edits` |
| Scrollbar inputs (`vertical_extent`) are O(1) with wrapping off (one row per line) and one fused pass with wrapping on; thumb geometry unchanged. | `text_editor.rs` | `vertical_extent_matches_the_per_line_walk_in_both_wrap_modes` |
| With wrapping off, `layout()` never hands the editor a narrower width than it already has (cosmic-text relayouts every shaped line on any size change) — except while a right-to-left run is on screen, when the width falls back to the visible extent because cosmic-text right-aligns RTL paragraphs to the buffer width. | `text_editor.rs` | `no_wrap_layout_width_ratchets_and_wrapping_restores_the_viewport`, `no_wrap_width_yields_to_the_visible_extent_while_rtl_text_is_on_screen` |
| The highlighter checkpoints parser state every `CHECKPOINT_INTERVAL` (32) lines; `change_line(k)` resumes from the nearest checkpoint and is equivalent to a fresh run. | `highlight.rs` | `resuming_from_a_checkpoint_reproduces_a_fresh_run_for_every_line`, `resuming_after_an_edit_matches_a_fresh_run_of_the_edited_buffer`, `checkpoints_are_one_per_interval` |
| Load path copies: `EndOfLine::normalize` returns LF text in the same allocation; `decode_owned` reuses the byte buffer for UTF-8. | `text.rs`, `encoding.rs` | `normalize_returns_an_lf_buffer_untouched_without_copying`, `owned_decode_of_valid_utf8_reuses_the_buffer`, `owned_and_borrowed_decoders_agree` |
| Find readouts are one regex pass (`count_and_ordinal`); replace-all expands into the output buffer. | `find.rs` | `count_and_ordinal_matches_the_two_pass_form`, `replace_all_matches_a_per_match_splice` |
| Undo-depth eviction is O(1) (`VecDeque::pop_front`) — structural, not timed. | `history.rs` | the cap tests pin *which* entries are evicted and the `Clean` transitions; `coalescing_appends_in_place_and_keeps_the_step_id` pins the in-place append |
| Language detection is a hash lookup, not a scan of the syntax set. | `syntax/lib.rs` | `indexed_detection_matches_syntects_scans_for_every_known_token` |

### Known costs, accepted

Measured on the 200k-line / 12 MB fixture in a release build (#126's audit).
Each is understood and either bounded or inherent; none is a bug to fix in
passing, and a change that moves one is its own issue with its own numbers.

- The find bar re-runs its regex pass over the whole document on every keystroke
  while it is open: 19.6 ms at 12 MB for a case-insensitive literal with 200 000
  hits. Closed, it costs nothing.
- Bracket matching runs once per message (`Shell::readouts`) and walks from the
  caret to its partner — the rest of the document for an unmatched opener: 11 ms
  for a stray `(` at the top of 12 MB.
- `vertical_extent` is O(lines) per frame with wrapping **on** (one fused pass,
  0.78 ms at 200k lines); O(1) with wrapping off.
- `Document::splice` recounts words over the whole touched line: 72 ms per
  keystroke on a single 12 MB line, bounded by the single-line open guard
  (`LARGE_FILE_LINE_BYTES`, 500 KB).
- `refresh_dirty` falls through to a `memcmp` when an edit leaves the length and
  the totals equal to the saved baseline's: +1.8 ms at 12 MB, kept for exactness
  (§8).
- cosmic-text shapes the whole buffer at open (6.5 s + 4.6 s and +7.5 GB of RSS
  at 200k lines, ≈56 µs and ≈37 KB per line) and, with wrapping off, relayouts
  every shaped line on any viewport size change (≈550 ms per resize event at
  200k lines). The open cost is inherent to the widget and the large-file guard
  (`LARGE_FILE_LINES` 50 000, `LARGE_FILE_BYTES` 8 MiB, `LARGE_FILE_LINE_BYTES`
  500 KB) is its lever; the resize relayout has a proposed fix, tracked as #139.
- Saving copies the text once: `Effect::WriteFile` carries an owned `String`
  because the pure core returns effects by value and cannot lend the shell a
  borrow across the call (1 ms at 12 MB, next to the I/O). #147 tracks removing it.

### Known exceptions (tracked)

Paths that currently break a budget above. Each has an issue with measurements
and a proposed fix; do not "fix" one in passing without that issue's numbers, and
do not add another without filing one.

- #137 — undo, redo, replace and the text operations rebuild the whole editor
  buffer (`Content::with_text`), so cosmic-text reshapes every line: ≈1.1 s per
  Ctrl+Z at 20k lines. The edit path's budget row assumes the incremental form.
- #138 — the highlighter protocol is forward-only: a jump past unparsed lines
  (Ctrl+End, a far Go-to-line, a theme or language change while scrolled deep)
  re-parses every skipped line in one frame, ≈40 s for 200k Rust lines. The
  checkpoints only rewind.
- #139 — with wrapping off, every resize event relayouts every shaped line
  (≈550 ms at 200k lines).
- #140 — a clean document holds a second full copy of its text as the saved
  baseline (12 MB per 12 MB document).
- #144 — the highlighter state lives in the one editor widget and survives the
  tab switch, so same-language tabs share parser state (a bug, not a cost: an
  open block comment in one tab colours code in another).

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

Mirror of `.github/workflows/native-ci.yml` (fmt, clippy, tests, windowed smoke,
coverage, fuzz, licences), `.github/workflows/audit.yml` (`cargo audit`, also
weekly) and `.github/workflows/packaging.yml` (metainfo and desktop-file
validation, then the deb / rpm / AppImage / msi / dmg build-install-launch
smokes — those run on every PR but only CI can run them). Run the commands from
the repository root on the **newest stable Rust**: CI installs
`dtolnay/rust-toolchain@stable`, so `rustup update stable` first. A clippy or
coverage pass on an older toolchain is not evidence (#132 needed a follow-up
commit for a lint only the newer clippy raises). Never add a `schedule` trigger
to a workflow that gates PRs: GitHub disables such a workflow — every trigger —
after 60 days of repository inactivity (#133); scheduled work gets its own
workflow file.

```bash
rustup update stable                          # CI runs the newest stable; match it before trusting any result below
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --package notepad-core --package notepad-syntax --all-targets
cargo test --locked --package notepad-iced --all-targets
cargo build --locked --package notepad-iced && xvfb-run -a timeout 10 ./target/debug/notepad-extra   # windowed smoke (Linux, software renderer): exit 124 = the window stayed up
bash scripts/coverage.sh                      # core + syntax: ≥ 99 % lines, aggregate AND per file (needs cargo-llvm-cov, jq)
PROPTEST_CASES=1 bash scripts/coverage.sh     # the same gate with one proptest case: it must still hold (no proptest luck); CI pins PROPTEST_RNG_SEED=1
cargo metadata --locked --format-version 1 --manifest-path crates/core/fuzz/Cargo.toml >/dev/null   # fuzz lockfile in sync
cargo check --locked --manifest-path crates/core/fuzz/Cargo.toml   # fuzz targets still compile against the core API (running them needs nightly + cargo-fuzz)
LICENSE_AUDIT_ONLINE=1 bash scripts/license-audit.sh --check       # no copyleft-only dependency; packaging/CRATE-LICENSES.md matches Cargo.lock
PACKAGING_DEPS_ONLINE=1 bash scripts/packaging-deps.sh --check     # packaging/deps/* match Cargo.lock
cargo audit                                   # RustSec advisories (needs cargo-audit); .cargo/audit.toml holds the assessed ignores
bash scripts/vendored-crates.sh check         # each vendor/<crate> == its crates.io release (sha256-checked) + its notepad-extra.patch
bash scripts/vendored-crates.sh audit         # cargo audit with vendored crates checked as the crates.io release they came from
```

- **Coverage gate.** Every new line in `notepad-core` / `notepad-syntax` needs a
  test; a small file effectively needs 100 %. The shell's `view` is smoke-tested
  (xvfb) and Simulator-tested, not coverage-gated. Coverage must not depend on
  proptest luck: a line reached only by random inputs (including a test-module
  "reference" oracle that only a proptest calls) makes the gate flaky — give it a
  deterministic unit test. Check with `PROPTEST_CASES=1 bash scripts/coverage.sh`.
  CI runs the newest stable Rust; coverage regions differ between compiler
  versions, so verify with the same version when a CI number disagrees with yours.
- **Dependencies.** Default: none. If one is unavoidable it must be pure Rust and
  licence-compatible with GPL-3.0-or-later; then regenerate the detached fuzz
  lockfile (`cargo generate-lockfile --manifest-path crates/core/fuzz/Cargo.toml`),
  refresh `packaging/CRATE-LICENSES.md` and the distro dependency lists
  (`scripts/license-audit.sh`, `scripts/packaging-deps.sh`), and keep
  `cargo audit` clean (`.cargo/audit.toml` lists the assessed ignores). Prefer
  promoting a crate already in `Cargo.lock`. A crate that `dlopen`s a system
  library at runtime (as `rfd` does `libdbus-1`, and winit the X11 / Wayland /
  xkbcommon libraries) must have that library named in the deb `depends` and rpm
  `requires` in `crates/iced/Cargo.toml` and in the README's Linux prerequisites;
  the packaging clean-install smokes are the check (#131 did this for `libdbus-1`).
- **Patched crates.** Stay on the newest release of each crate that the rest of
  the graph allows. When that release still has a bug that hits us and upstream
  has a fix, apply upstream's commit to a copy under `vendor/<crate>/`,
  substituted with `[patch.crates-io]` in the root `Cargo.toml`. Each copy
  carries:
  - a `VENDORED.md` naming the upstream commit and the condition for deleting it;
  - `notepad-extra.patch`, the exact diff from the release;
  - `upstream.toml`, the release's name, version and crates.io sha256;
  - a `// NOTEPAD-EXTRA(#NN)` marker at each change.

  `vendor/` is excluded from the workspace, so our fmt / clippy / test gates do
  not run over upstream code. A patched copy has no Cargo.lock checksum and
  `cargo audit` skips path crates, so `scripts/vendored-crates.sh` stands in for
  both (CI runs `check` in the licence job and `audit` in `audit.yml`). A patched
  crate drops out of the Gentoo download list but stays in the RPM bundled
  provides, and the Debian gap report marks it PATCHED
  (`scripts/packaging-deps.sh`). Today: `vendor/cosmic-text` (#149).
- **Vendored widget.** Every divergence from upstream iced is marked
  `// NOTEPAD-EXTRA(#NN): reason` and listed in `crates/iced/VENDORED.md`. Tests
  in `text_editor.rs` enforce that the markers are well-formed, that one sits
  above each anchored divergence, that every catalogued issue still has a marker
  (add the number to `every_documented_divergence_issue_has_a_marker`) and that
  every marker's issue has a `| #NN |` row in `VENDORED.md`; the row's wording is
  kept by hand.
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
- **Supported OS versions** (#135): an OS release is supported for exactly as
  long as its vendor's standard support lasts, and no longer. Paid or opt-in
  extensions (Windows ESU, Debian LTS, Ubuntu ESM) and the Windows Long-Term
  Servicing Channel do not count. The floors, their sources and their review
  dates live in `packaging/support-floors.toml`; the bundle metadata, the `.msi`
  launch condition, the Linux build runner and the README are copies of it, and
  a shell test (`declared_os_floors_match_the_support_floors_file`) fails when
  any copy drifts. Re-check every floor at each release and on each review date;
  raising a floor is its own issue, never part of another change. Never advertise
  or test on an OS release its vendor has dropped.

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
