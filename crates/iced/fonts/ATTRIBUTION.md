# Bundled font — attribution & licensing

Notepad Extra is **fully offline** (epic #25): text must render without depending
on any system-installed font. To guarantee that with the smallest possible
footprint, **exactly one** font family is vendored here and embedded into the
binary as bytes (`include_bytes!`, registered at startup — see `src/fonts.rs`).
Everything else in the two font pickers (#61) comes from the **OS's own installed
fonts**, which iced's text backend already loads; those are a best-effort bonus,
not a guarantee.

## The one bundled family: DejaVu Sans Mono

| File(s) | Styles | Licence | Upstream |
| --- | --- | --- | --- |
| `dejavu-sans-mono/DejaVuSansMono*.ttf` | Book, Bold, Oblique, Bold-Oblique | DejaVu Fonts Licence (Bitstream Vera derivative) — see `LICENSE` | Debian `fonts-dejavu-core` |

Why this font, specifically:

- **Monospaced** — the editor buffer is the surface where guaranteed, correctly
  aligned rendering matters most.
- **Trivially clean licensing** — the DejaVu/Bitstream-Vera licence is permissive,
  GPL-3-compatible, and (unlike OFL) carries **no Reserved Font Name**, so there
  is none of the subset/rename friction that makes multi-font vendoring awkward
  for packagers.
- **Already shipped by both distros** — Debian `fonts-dejavu-core` and Fedora
  `dejavu-sans-mono-fonts`. So although we embed the bytes for the portable
  Windows / macOS / AppImage builds, a Debian or Fedora package can **de-vendor**
  it to the system copy with no conflict (#17 / #7 / #8). Unmodified,
  byte-for-byte upstream — never subset or edited.

## Everything else: the user's own OS fonts

The editor-font and UI-font pickers list this bundled family **plus** every font
family installed on the running machine (enumerated from iced's global font
database). Those system choices are:

- **not bundled or redistributed** by us (no licensing burden — they are the
  user's own fonts), and
- **not guaranteed** — they render only if actually installed. If a picked family
  is missing, that is a local OS-font matter; the bundled DejaVu Sans Mono remains
  the default and the offline safety net, so the app always renders.

## Re-vendoring

Refresh by replacing the files under `dejavu-sans-mono/` with the same-named,
**unmodified** files from Debian's `fonts-dejavu-core` package (or upstream
DejaVu), keeping `LICENSE` in step.
