# Third-party code and assets vendored into this repository

Notepad Extra is licensed `GPL-3.0-or-later` (see [LICENSE](LICENSE)). A small
amount of third-party code and one font family are **vendored** — copied into
this tree rather than pulled as a dependency. This file is the canonical record
of what, from where, and under which license, for distro licensing review
(Debian `debian/copyright`, Fedora license audit — #7 / #8 / #17 / #47).

Ordinary crate dependencies (resolved via `Cargo.lock`) are *not* listed here;
their licenses travel with the crates themselves. For the audited, per-crate
licence inventory those distro reviews consume — regenerated from `cargo
metadata` by [`scripts/license-audit.sh`](scripts/license-audit.sh) — see
[packaging/CRATE-LICENSES.md](packaging/CRATE-LICENSES.md).

## Vendored source code

### `crates/iced/src/text_editor.rs` — iced `text_editor` widget

- **Upstream:** the `text_editor` widget from
  [iced](https://github.com/iced-rs/iced)'s `iced_widget` crate, vendored from
  **iced 0.14** (first taken from 0.14.0; the workspace currently resolves
  `iced_widget` 0.14.2).
- **License:** **MIT** — Copyright 2019 Héctor Ramón, Iced contributors. The
  full permission notice is preserved as the file's header comment, as the MIT
  license requires. The license text is reproduced below.
- **Local changes:** extended with a visible scrollbar (#34), a line-number
  gutter / active-line / bracket-match rendering (#41), syntect highlighting
  hooks (#32), and unfocused range-selection painting (#33). Every divergence
  is tagged `// NOTEPAD-EXTRA(#NN): …`; provenance and the re-vendor policy are
  documented in [crates/iced/VENDORED.md](crates/iced/VENDORED.md).
- **Combined-work licensing:** MIT is GPL-compatible, so the file participates
  in the `GPL-3.0-or-later` combined work; the MIT notice governs the vendored
  portions themselves.

## Vendored assets

### `crates/iced/fonts/dejavu-sans-mono/` — DejaVu Sans Mono

- **Upstream:** the [DejaVu fonts](https://dejavu-fonts.github.io/) (via Debian
  `fonts-dejavu-core`), byte-for-byte unmodified.
- **License:** DejaVu Fonts License (a Bitstream Vera derivative, permissive
  and GPL-compatible).
- **Details:** see
  [crates/iced/fonts/ATTRIBUTION.md](crates/iced/fonts/ATTRIBUTION.md), which
  records the rationale, styles, and de-vendoring path for distro packages.

## MIT License (iced)

```plaintext
Copyright 2019 Héctor Ramón, Iced contributors

Permission is hereby granted, free of charge, to any person obtaining a copy of
this software and associated documentation files (the "Software"), to deal in
the Software without restriction, including without limitation the rights to
use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
the Software, and to permit persons to whom the Software is furnished to do so,
subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS
FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR
COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
```
