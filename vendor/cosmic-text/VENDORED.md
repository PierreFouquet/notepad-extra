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

- **`src/shape.rs`:** upstream `1e0074c`, applied verbatim (its hunks match the
  commit; only line offsets differ). The assert is gone, and each paragraph's
  spans are built with the first paragraph's direction, so the whole line is
  laid out in the first paragraph's base direction. A
  `// NOTEPAD-EXTRA(#149): …` comment marks the spot, so
  `rg NOTEPAD-EXTRA vendor/` finds it.
- **`Cargo.toml`:** the published (normalised) manifest without its
  `[[test]]`, `[[bench]]`, `[dev-dependencies.*]` and `[profile.test]` sections.
  They describe building cosmic-text on its own and point at files not copied
  here.
- **Not copied:** the tests, benches, examples, fonts, screenshots and helper
  scripts of the published package (5.6 MB). The library sources (`src/`), both
  licence files, `README.md` and `CHANGELOG.md` are unchanged.

Checking the copy: `diff -r` against the crates.io package (for example
`~/.cargo/registry/src/*/cosmic-text-0.15.0/src`) shows exactly the `shape.rs`
change above.

## Licence

`MIT OR Apache-2.0`, as upstream. `LICENSE-MIT` and `LICENSE-APACHE` are kept
here unchanged, and the top-level `THIRD-PARTY.md` records the copy for distro
licensing review.

## Effect on packaging and audits

- **`Cargo.lock`:** the `cosmic-text` entry has no `source` / `checksum`, because
  it now resolves to this path.
- **`scripts/packaging-deps.sh`:** the Gentoo `CRATES=` list is a crates.io
  download list, so it leaves cosmic-text out (the copy ships in the source
  tree). The RPM `bundled(crate(…))` provides keep it: the crate is still bundled
  into the binary.
- **`scripts/license-audit.sh`:** unchanged. It lists every non-workspace
  package whatever its source, and the licence is the same.
- **`cargo audit`:** RustSec advisories match crates.io packages, so this copy is
  not checked against them while the patch is in place. Watch upstream
  advisories for cosmic-text by hand until the patch is dropped.
- **Debian (#7)**, which unbundles crates, would need the same patch applied to
  its `rust-cosmic-text`.

## When to drop it

Delete `vendor/cosmic-text` and the `[patch.crates-io]` entry once the iced
release this workspace uses depends on a cosmic-text release that contains
`1e0074c`. Then regenerate `Cargo.lock` and run `scripts/packaging-deps.sh`. The
regression tests for #149 in `crates/iced/src/text_editor.rs` must still pass
without the patch.
