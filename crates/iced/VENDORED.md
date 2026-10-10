# Vendored widget: `src/text_editor.rs`

`src/text_editor.rs` is a **fork of iced's `text_editor` widget**, carried in-tree
because the migration needs capabilities the upstream widget neither renders nor
exposes through its public API. This file records where it came from, why it
diverges, how each divergence is marked, and how to re-sync it with a newer iced.

## Upstream source

- **Widget:** `text_editor` from the `iced_widget` crate (re-exported as
  `iced::widget::text_editor`).
- **Vendored from:** iced **0.14** (`iced_widget` 0.14.x — the workspace currently
  resolves `iced_widget` 0.14.2; the fork was first taken from 0.14.0). The
  `iced_core` / `iced_graphics` crates it builds against are pinned to `=0.14.0`
  in `Cargo.toml`, so the vendored widget compiles against exactly the types iced
  itself uses.
- **Upstream licence:** MIT (the iced project). The fork keeps that licence;
  per-file attribution for packaging (#77): the MIT copyright + permission
  notice is preserved as `src/text_editor.rs`'s header comment, and the
  repo-level record for licensing review is the top-level `THIRD-PARTY.md`.

The widget lays text out through cosmic-text. That crate is itself a patched
copy while #149's fix is unreleased; see
[`vendor/cosmic-text/VENDORED.md`](../../vendor/cosmic-text/VENDORED.md).

## Why it is vendored

iced 0.14's stock `text_editor`:

1. **Renders no scrollbar and exposes no scroll offset (#34).** Word-wrap /
   horizontal-scroll work needs a visible scrollbar and the current scroll
   position, but the generic `text::editor::Editor` trait hides the offset. The
   fork reaches the concrete software-renderer editor's cosmic-text buffer to read
   it (see `ScrollOffset`) and draws real scrollbar thumbs.
2. **Has no line-number gutter, active-line highlight, or bracket matching (#41).**
   These are drawn/measured inside the widget's `layout` and `draw`, which cannot
   be extended from outside.
3. **Will host syntect highlighting (#32).** Also a widget-internal concern.
4. **Paints a range selection only while focused (#33).** Stock iced draws the
   selection highlight inside `if let Some(focus)`, so a find match disappeared the
   moment the find field took focus. The fork paints a *range* selection regardless
   of focus (the caret still blinks only while focused) so find highlighting stays
   visible.

None of these can be layered on top of the upstream widget from the shell, so the
widget is forked. Keeping the fork small and *documented* is the point of this
file — see also **#79** (extracting the fork's pure geometry into tested helpers).

## Divergence catalogue

Every local change is marked in the source with a greppable comment:

```bash
// NOTEPAD-EXTRA(#NN): <one-line reason>
```

so `rg 'NOTEPAD-EXTRA'` lists the full set, and a future diff against upstream is
mechanical. The divergences, by area:

| Area | Issue | Key added items |
| --- | --- | --- |
| Find-match highlighting while unfocused | #33 | the selection-drawing block in `draw` paints a `Selection::Range` regardless of focus (caret still gated on focus) |
| Scrollbar + scroll offset | #34 | `trait ScrollOffset` + its `impl`; `struct BarGeometry`; `fn horizontal_bar`; `fn vertical_bar`; scrollbar drawing in `draw` |
| Line-number gutter, active-line, bracket match | #41 | `struct BracketHighlight`; `const GUTTER_PAD`; `fn gutter_width`, `fn digit_count`, `fn inset_left`, `fn faint`; the `line_numbers` / `active_line` / `bracket` fields; the `line_numbers()` / `active_line()` / `bracket_match()` builder methods + private `gutter()`; gutter/active-line/bracket drawing + gutter reservation in `layout`/`draw` |
| Geometry extracted to tested core helpers | #79 | `use notepad_core::geometry`: `fn inset_left` applies the core's clamp, gutter width and digit count come from `geometry`, and thumb sizing / placement in `horizontal_bar` / `vertical_bar` is `geometry::scroll_thumb` — the renderer-free maths lives in the pure core where it is unit-tested |
| Compiled-out upstream methods | #25 | the upstream `highlight()` / `class()` methods are gated on the private cfgs `notepad_extra_upstream_highlighter` / `notepad_extra_upstream_advanced` (never set) instead of the upstream `highlighter` / `advanced` Cargo features, so `cargo test --all-features` can't switch on code that references the un-linked `iced_highlighter`; the cfg names are declared in `Cargo.toml`'s `[lints.rust] check-cfg` |
| Large-document performance | #126 | `Content::text()` is rebuilt in one editor borrow with an exact pre-size and no per-line `String` (the shell calls it on every keystroke); `struct VerticalExtent` + `ScrollOffset::vertical_extent` (O(1) with wrapping off — one row per line — and a single fused pass with wrapping on; `scroll_top()` / `content_height()` are now conveniences over it); `draw` / `update` / `mouse_interaction` take one `vertical_extent()` and compute bar geometry only for pointer events that can touch a thumb; the no-wrap width *ratchet* in `layout` (the bounds are not handed back down to the viewport while only left-to-right text is on screen — that bounce used to cost two whole-buffer cosmic-text relayouts per layout; a visible right-to-left run makes the width fall back to the visible extent, because cosmic-text right-aligns RTL paragraphs to the buffer width) |
| Caret-only `move_to` | #137 | `Content::move_to` with no selection drops any selection the editor holds (one collapsing `Action::Move`) before moving the caret; upstream keeps the old anchor, so the shell's in-place splice of an undo / redo / replace (and the caret-only reveal of go-to-line) turned it into a selection that the next paste overwrote |
| Offsets through the buffer's own lines | #153 | `Content::offset_of` / `Content::position_of` convert between byte offsets of `text()` and cursor positions by walking the buffer's lines, sharing `text()`'s separator rule (now the free `fn separator`). The shell uses them for text holding a `\r`: cosmic-text breaks lines there, and where it breaks them depends on the edit history, so the core's `\n`-only line index cannot place a cursor |

The remaining differences from a pristine upstream file are cosmetic (import
grouping, `rustfmt`) and the ordinary 0.14.0 → 0.14.2 patch drift.

When re-vendoring, restore each method's body from upstream but keep the
`#[cfg(notepad_extra_upstream_*)]` gate in place of upstream's
`#[cfg(feature = "…")]` — the two are 1:1 (`highlighter` → `…_highlighter`,
`advanced` → `…_advanced`).

## Re-vendor / upgrade policy

To move the fork onto a newer iced:

1. Bump `iced`, `iced_core`, `iced_graphics` in `Cargo.toml` together (keep the
   `iced_core`/`iced_graphics` pins exact so the vendored widget sees iced's own
   types).
2. Fetch the new upstream `text_editor.rs` from the matching `iced_widget` release.
3. Re-apply each `// NOTEPAD-EXTRA(#NN)` hunk on top of it — the markers make the
   set of changes to replay explicit.
4. Update the "Vendored from" version above, run the widget's own tests
   (`cargo test -p notepad-iced text_editor`), and confirm the windowed smoke.

Prefer *upstreaming* a capability (e.g. an exposed scroll offset) over carrying a
divergence, and delete the corresponding `NOTEPAD-EXTRA` hunk if/when upstream
gains it.
