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
  per-file attribution for packaging is tracked in **#77**.

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

The remaining differences from a pristine upstream file are cosmetic (import
grouping, `rustfmt`) and the ordinary 0.14.0 → 0.14.2 patch drift.

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
