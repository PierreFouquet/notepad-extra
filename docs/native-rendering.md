# Native GUI rendering (the vendored `text_editor`)

Notes for anyone touching the iced render shell (`crates/iced`), especially the
**vendored** `crates/iced/src/text_editor.rs`. Application behaviour lives in the
pure `notepad-core`; this file is only about the *drawing* half, where the
non-obvious traps are. If you're adding a gutter, an overlay, a decoration, a
scrollbar, or any custom `draw` code, read this first — each rule below cost real
debugging. See [docs/testing.md](testing.md) for how the logic is tested.

## Software renderer, no GPU

The shell renders with **`tiny-skia`** (CPU) via `softbuffer`, not `wgpu` — a
deliberate #26 choice so the app needs no GPU and installs on minimal systems.
`iced` is built `default-features = false` with just `tiny-skia`, `x11`,
`wayland`, `thread-pool`, `advanced`. Consequences that surface constantly:

- The compositor only repaints **damaged regions** (see [Damage traps](#damage-traps-iced-014--tiny-skia)).
- iced's `image` feature is off, so the window icon is decoded from
  `icons/icon.png` by the `png` crate into raw RGBA (`window_icon` in `main.rs`).
  macOS ignores runtime window icons — its dock icon comes from the `.app`
  bundle at packaging time (#43).

## Why the widget is vendored, and the seam

iced 0.14's stock `text_editor` draws no scrollbar and exposes no scroll offset,
so word wrap (#34) forced us to copy it into the tree. The extension seam is the
**`ScrollOffset` trait**, implemented for the concrete
`iced_graphics::text::Editor`, which reaches into its cosmic-text `buffer()`:

| Method | Returns | Used by |
| --- | --- | --- |
| `scroll_top` | pixels scrolled past the top | vertical scrollbar thumb |
| `content_height` | full wrapped document height | scrollbar sizing |
| `line_height` | pixels per line | thumb-drag → line delta |
| `visible_line_tops` | `(buffer line, y-top)` per visible line | line-number gutter (#41) |
| `glyph_bounds` | `(x, y-top, width)` of a glyph | bracket-match box (#41) |

Everything drawn by hand — scrollbars (#34), the gutter, active-line and bracket
highlights (#41) — lives in that widget's `draw`. Dependencies (`iced_core`,
`iced_graphics`) are **pinned** to the exact versions iced 0.14 resolves so cargo
unifies the types and nothing extra is vendored for the source-built story (#17).
The vendored copy keeps upstream's `#[cfg(feature = "highlighter"/"advanced")]`
gates; both features are declared (empty) in `Cargo.toml` so `-D warnings` stays
clean. Syntax highlighting (#32) deliberately leaves the `highlighter` feature
**off**: enabling it pulls `iced_highlighter`, which builds syntect against
**oniguruma** — a C dependency #25/#17 forbid. Instead the shell plugs its own
`iced_core::text::Highlighter` into the widget's *unconditional* `highlight_with`
(`crates/iced/src/highlight.rs`), backed by the shared **fancy-regex** syntect set
in the `notepad-syntax` crate (pure Rust, no C dep, verified by
`cargo tree | grep -i onig` staying empty). That crate also owns extension /
whole-filename detection and the grouped picker catalogue, drawn from the
**two-face** extended syntax set (~213 languages, so TOML/TypeScript/… resolve —
syntect's own defaults do not).

## Where decorations live (MVU split)

Keep logic in the pure core and draw in the shell, mirroring the status bar
(#37): the shell reads the widget's live caret (`Content::cursor()`, no renderer
needed) and calls a pure core function.

- **Bracket matching** — `notepad_core::brackets::match_at(text, caret)` returns
  byte offsets; `Shell::active_bracket()` maps them to `(line, column)` and hands
  them to the widget. Fully headless-testable.
- **Active-line / line numbers** — pure *positioning* only; drawn in the widget
  from the caret and the cosmic-text layout. "View smoke-tested" per the DoD.
- **Caret moves from the core** — an `Effect::RevealRange { start, end }` (find,
  go-to-line, replace) is byte offsets on the same canonical text the editor
  holds; `run_effect` converts them to the widget's `(line, byte-column)` cursor
  with `find::line_col_of`, and `Shell::status` converts back with
  `find::offset_at`. Reuse those, don't re-derive columns.

## Damage traps (iced 0.14 + tiny-skia)

The compositor computes per-primitive damage by diffing the previous and current
frames and only repaints changed rectangles. Get a damage rect wrong and your
decoration is correct in memory but paints only on a **full** redraw (e.g. a
window resize) — it lags edits and scrolling.

1. **`Alignment::Right` breaks `fill_text` damage.** A text item's damage rect is
   `Rectangle::new(position, size)` — measured *rightward* from `position`
   (`iced_graphics::text::Text::visible_bounds`). A right-aligned render draws
   glyphs *leftward* from `position`, so the damage sits to the right of the
   glyphs and never covers them. **Do:** left-anchor the render
   (`Alignment::Default`) at a position you right-align yourself. The gutter
   estimates digit width as `chars * text_size * 0.6` — the same factor
   `gutter_width` reserves with, so they stay consistent.

2. **Anything outside the editor's text bounds isn't damaged on a plain edit.**
   The editor's repaint region is its text rectangle. A left gutter sits
   *outside* it, so a bare edit never marks it dirty; quads drawn *inside*
   `text_bounds` (active-line band, bracket boxes) repaint fine. The same
   editor-bounds limitation is behind #34's horizontal-smear fix: with wrap off,
   panning the render sideways revealed a stale strip, fixed by re-running
   `editor.update` at the full content width so iced marks the whole strip dirty.

3. **Don't read `buffer().layout_runs()` for per-line geometry.** Its iterator
   lazily stops at the first un-shaped line, so it truncates and lags. Instead
   walk `buffer.lines` from `scroll.line`, accumulating
   `line.layout_opt().map_or(1, Vec::len) * line_height` — exactly how
   `scroll_top` / `content_height` compute, and always current. Use
   `layout_runs()` only when you need a shaped glyph's pixel `x` (e.g.
   `glyph_bounds`), and treat "not found" as "off screen → draw nothing".

4. **A stock widget's text changing with no co-located pointer event can
   under-damage — force a full redraw.** `pick_list` draws its selected label
   with a vertically-*centred* `fill_text`, but the damage rect (trap 1's
   `Rectangle::new(position, size)`) is anchored at the label's centre and only
   extends *downward*, so it covers just the bottom half of the glyphs. A hover
   re-damages the button's quad and clears the rest — which hides the bug — but a
   **programmatic** label change with no pointer over the widget (the language
   picker following a file's auto-detected syntax, #32) leaves the old glyphs'
   top halves on screen: "garbled" until you hover. You can't patch a stock
   widget's `draw`, so the shell forces **one full redraw** on the frame the
   language changes: `Shell::update` flips a `repaint_nudge` bit whenever the
   active tab's `language()` changes, and `Shell::app_style` nudges the background
   clear-colour by `1/255` when it's set — iced's `present` does a whole-surface
   repaint when the clear-colour differs from the last frame's, clearing the
   stale pixels. Reserve this hammer for rare, genuinely-programmatic changes;
   normal edits already damage correctly. (The status bar's language cell dodges
   the same bug only by luck — its neighbours repack when the shorter text
   changes width, incidentally damaging the region.)

## Scrolling internals

- **cosmic-text stores scroll as `{ line, vertical, horizontal }`**, where `line`
  is the *topmost visible buffer-line index* and `vertical` is only the sub-line
  pixel offset within it. Returning `vertical` alone makes the thumb jump to
  near-top on every new paragraph; the absolute offset is the rows above
  `scroll.line` times `line_height`, plus `vertical` (see `scroll_top`).
- **`min_bounds().height` is not the document height.** It routes through
  `text::measure`, which only walks the *shaped, visible* window, so it reports
  ~one viewport tall regardless of length. Count every buffer line's wrapped rows
  instead (`content_height`).
- **Vertical scroll is line-quantised** (`Action::Scroll { lines }`), so dragging
  the vertical thumb translates a target pixel offset into a *relative line
  delta*. Horizontal scroll is different — see below.
- **Horizontal pan is the widget's own, not cosmic-text's.** iced's tiny-skia
  renderer ignores cosmic-text's `scroll.horizontal`, so with wrap off the widget
  keeps its own `horizontal_offset` in `State`, pans both the render and input
  hit-testing by it, and follows the caret sideways in `RedrawRequested` (only
  when the caret actually moves, so a scrollbar drag away from the caret sticks).

## Coordinate offset (the gutter inset)

The gutter shrinks the text area from the left. Every place that maps between a
screen point and a text position must subtract the gutter, or clicks land on the
wrong character. They must all use the **same** `gutter_width(...)` (a pure fn of
enabled/line-count/text-size, identical everywhere and unit-tested):

- `layout` — subtract it from the wrap viewport width (else wrapped rows run
  under the gutter);
- `update` (scrollbar hit-test) and `Update::from_event` (click/drag) — subtract
  it alongside `padding.left`;
- `input_method` (pre-edit caret) and `draw` (`text_bounds`, translation).

## Theme colours in a generic widget

`draw` is generic over `Theme: Catalog`, which only exposes
`theme.palette() -> Option<Palette>` (via the `theme::Base` supertrait) — **not**
`extended_palette()`, which is inherent to the concrete `iced::Theme`. Use
`palette()` for semantic colours (`success`/`danger` for matched/unmatched
brackets) with a fixed fallback, and `theme.style(&self.class, status)` for the
widget's own `Style` (background, text `value`, `selection`). `faint(color, a)`
derives translucent tints from the theme's text colour so they read in both light
and dark.

## Light / dark theme + syntect pairing (#36)

The UI theme is one app-wide `ThemeMode { Light, Dark }` in the pure core
(persisted via #38, default light). The shell maps it two ways each frame: to the
`iced::Theme` chrome (`Shell::theme` → `Theme::Light`/`Dark`, wired with `.theme()`
on the application builder) and to the editor's syntect highlight theme, which
`notepad_syntax::highlight_theme(mode)` pairs — **InspiredGitHub** for light,
**Monokai Extended** for dark — the app's default light/dark pairing (light by
default, Monokai for dark). Monokai isn't in syntect's *own* default theme set, so
the themes come from `two-face` (already vendored for the syntaxes), keeping the
app offline and oniguruma-free.

Switching re-highlights for free: the vendored `text_editor` compares the
highlighter's `Settings` (which carry the `ThemeMode`) and calls `update()` when
they differ, rebuilding our `SyntectHighlighter` from line 0. The whole-window
repaint is free too — the new `iced::Theme` changes the clear colour, so the
software compositor redraws the surface (no repaint-nudge needed here, unlike the
`pick_list` trap above).

The toolbar button's label names the theme you'll
switch *to* (☾ Dark / ☀ Light), not the current state — a plain action button, not
a lit on/off toggle (which read as if the word labelled the current mode). A
🌙 emoji isn't in the bundled DejaVu Sans Mono, so ☾ stands in for it.

## Dialogs are in-app panels, not native windows

The `rfd` `xdg-portal` backend (chosen to stay off GTK build deps) exposes
no *message* dialog, and native modals aren't headless-testable. So the
confirm-close bar (#31), About panel (#40), find/replace bar (#33) and drag-drop
overlay (#42) are all ordinary in-app widgets pushed into the layout — keeping
the app fully offline and its wiring testable. Only file open/save use a real
native picker.

## A layout gotcha

The root column must be `width(Fill).height(Fill)`. Left at the default `Shrink`,
iced's flex layout hands the editor a wrap width equal to the widest non-fill
sibling (the toolbar) instead of the window width, so `Wrapping::Word` has no real
boundary and long lines overflow with nothing to scroll (#34).

## Verifying a visual change

`view`/`draw` need a real renderer, so they aren't unit-tested; CI only smoke-
launches the binary under `xvfb`. To actually *look* at a change locally without
a display, drive it headlessly:

```sh
Xvfb :99 -screen 0 1280x820x24 &
DISPLAY=:99 cargo run -p notepad-iced &
WID=$(DISPLAY=:99 xdotool search --name "Notepad Extra" | tail -1)
DISPLAY=:99 xdotool windowfocus "$WID"
DISPLAY=:99 xdotool type --window "$WID" 'fn main() {}'   # keys need --window
DISPLAY=:99 xdotool key --window "$WID" Return            # newlines need explicit Return
DISPLAY=:99 import -window root shot.png                  # ImageMagick screenshot
```

A decoration that looks right only *after* you resize the window is a repaint-
region problem (traps 1–2), not a logic bug.
