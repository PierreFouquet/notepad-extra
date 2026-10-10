# Vendored crate: `vendor/cosmic-text`

This is **cosmic-text 0.15.0** with one upstream fix applied. The workspace
substitutes it for the crates.io release via `[patch.crates-io]` in the root
`Cargo.toml`. It is a dependency, not a workspace member (`exclude = ["vendor"]`),
so the workspace's fmt, clippy and test gates do not run over it.

## Why

cosmic-text 0.15.0 panics on a buffer line that holds bidi paragraphs of
different directions (#149). cosmic-text splits buffer lines only at `\n`, `\r`,
`\r\n` and `\n\r`, but `unicode_bidi` also starts a paragraph after U+2029
PARAGRAPH SEPARATOR, U+0085 NEXT LINE and U+001C-U+001E. `ShapeLine::build` then
ran `assert_eq!(line_rtl, rtl)` for every paragraph. The assert also fires in
release builds, so a line like `"abc\u{2029}שלום"` panicked the editor on open or
paste, and a tab label on such a file name panicked too. The panic also poisons
iced's shared font-system lock, so the app cannot recover from it.

Upstream fixed it on `main` in
[`1e0074c`](https://github.com/pop-os/cosmic-text/commit/1e0074c83926041c16f9ee76afafe91819927013)
("fix: don't panic on lines with mixed-direction paragraph separators"). As of
2026-10-10 no release contains it. Every release through 0.19.0 still has the
assert. In any case iced 0.14, the newest iced release, requires
`cosmic-text = "0.15"`, so no newer cosmic-text release can be used without
forking iced as well.

## What differs from the crates.io release

The exact diff from the published crate is
[`notepad-extra.patch`](notepad-extra.patch). In words:

- **`src/shape.rs`:** upstream `1e0074c`, applied verbatim (its hunks match the
  commit; only line offsets differ).
  - The `assert_eq!(line_rtl, rtl)` in `ShapeLine::build` is gone.
  - Each paragraph's spans are built with the first paragraph's direction
    (`rtl`) instead of their own, so the whole line is laid out in the first
    paragraph's base direction while every run still reads in its own
    direction.
  - The doc comments on `ShapeLine::new` and `ShapeLine::build` lose their
    `# Panics` sections and describe that behaviour instead.
  - Ours: a `// NOTEPAD-EXTRA(#149): …` comment marks the loop, so
    `rg NOTEPAD-EXTRA vendor/` finds it.
- **`Cargo.toml`:** the published (normalised) manifest without its
  `[[test]]`, `[[bench]]`, `[dev-dependencies.*]` and `[profile.test]` sections.
  They describe building cosmic-text on its own and point at files not copied
  here. A `NOTEPAD-EXTRA(#149)` comment at the top says so.
- **Not copied:** the tests, benches, examples, fonts, screenshots and helper
  scripts of the published package (5.6 MB). The library sources (`src/`), both
  licence files, `README.md` and `CHANGELOG.md` are unchanged apart from the
  above.
- **Ours, not upstream's:** this file, `notepad-extra.patch` and
  `upstream.toml`, which records the release's name, version and crates.io
  sha256.

## Checking the copy

`bash scripts/vendored-crates.sh check` downloads cosmic-text 0.15.0 from
crates.io and checks the sha256 recorded in `upstream.toml`, which is the
checksum `Cargo.lock` carried before the patch. It then applies
`notepad-extra.patch` and requires the result to equal this directory, file for
file. CI runs it in the licence job, so an edit here that the patch does not
record fails the build, and so does a patch that stops applying.

To change the copy, edit the files and then regenerate the patch from a clean
extract of the release (`git diff --no-index --no-prefix a b`, with `a` = the
release's copied files and `b` = this directory without our three files).

## Licence

`MIT OR Apache-2.0`, as upstream. `LICENSE-MIT` and `LICENSE-APACHE` are kept
here unchanged, and the top-level `THIRD-PARTY.md` records the copy for distro
licensing review.

## Effect on packaging and audits

- **`Cargo.lock`:** the `cosmic-text` entry has no `source` / `checksum`, because
  it now resolves to this path. `scripts/vendored-crates.sh check` stands in for
  the checksum (above).
- **`scripts/packaging-deps.sh`:**
  - The Gentoo `CRATES=` list is a crates.io download list, so it leaves
    cosmic-text out; the copy ships in the source tree.
  - The RPM `bundled(crate(…))` provides keep it: the crate is still bundled
    into the binary.
  - The Debian gap report lists it as PATCHED.
- **`scripts/license-audit.sh`:** unchanged. It lists every non-workspace
  package whatever its source, and the licence is the same.
- **`cargo audit`:** it matches advisories only against crates.io packages, so
  on its own it skips this copy. `bash scripts/vendored-crates.sh audit` runs it
  over a copy of `Cargo.lock` in which cosmic-text keeps its crates.io source, so
  advisories for 0.15.0 still fail CI. `audit.yml` runs it on every PR and weekly.
- **Debian (#7)**, which unbundles crates, would need `notepad-extra.patch`
  applied to its `rust-cosmic-text`.

## When to drop it

Delete `vendor/cosmic-text` and the `[patch.crates-io]` entry once the iced
release this workspace uses depends on a cosmic-text release that contains
`1e0074c`. Then regenerate `Cargo.lock` and run `scripts/packaging-deps.sh`. The
regression tests for #149 in `crates/iced/src/text_editor.rs` must still pass
without the patch.
