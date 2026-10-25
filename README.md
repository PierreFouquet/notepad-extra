# Notepad Extra

Notepad but extra. A small, fast, **fully offline** text/code editor for Windows, macOS and
Linux, built as a native Rust GUI with [iced](https://iced.rs/). Inspired by Notepad++.

## Features

- Cross-platform: Windows (x64 + ARM64), macOS (Intel + Apple Silicon), Linux (x64 + ARM64)
- **Works fully offline** — no runtime network access of any kind; fonts are embedded as bytes and all syntax data is compiled in
- Tabbed editing with unsaved-change indicators, and a guard against closing a tab or quitting the app with unsaved work
- **Syntax highlighting for 200+ languages** — C/C++/C#, Java, JavaScript/TypeScript/JSX, Python, Go, Rust, Ruby, PHP, Swift, Kotlin, HTML/CSS/SCSS, Markdown, SQL, YAML/TOML, shell, PowerShell, Haskell, and many more, grouped in the language menu (powered by [syntect](https://github.com/trishume/syntect) with the pure-Rust fancy-regex backend — no native highlighting dependency)
- Automatic language detection from the file extension
- **Find / Replace / Go-to-line** (`Ctrl+F` / `Ctrl+H` / `Ctrl+G`, `F3` for next) with regex, case and whole-word options
- **Multi-encoding** open/save — BOM/heuristic detection on open, per-tab encoding, a convert picker and a separate "Reopen as…", with lossy saves blocked
- **Word wrap** toggle
- **Zoom** the editor font (`Ctrl++` / `Ctrl+-` / `Ctrl+0`)
- **Light / Dark theme** toggle (remembers your choice, along with wrap, zoom, fonts and the gutter)
- Separate **editor and UI font pickers** — bundled DejaVu Sans Mono is always available; the rest come from your installed fonts
- **Status bar**: line/column, selection length, document length & line count, EOL style, encoding
- **About dialog** with version, license and links (opens in your own browser; the app never fetches anything itself)
- Preserves a file's original line endings (LF / CRLF) on save
- Open / Save / **Save As** with broad file-type filters, plus drag-and-drop and command-line file opening
- Line numbers, bracket matching, active-line highlight

### Adding a language

Languages come from syntect's bundled grammar set (the extended
[`two-face`](https://github.com/CosmicHorrorDev/two-face) set, ~200 languages), compiled
into the binary — there is no per-language table to maintain and no mode files to vendor.
To cover a language the set doesn't already highlight:

- If it's an **extension the grammar doesn't list** but which maps to an existing grammar,
  add a `(extension, syntax-name)` row to `EXT_ALIASES` in
  [`crates/syntax/src/lib.rs`](crates/syntax/src/lib.rs) — a test asserts every alias resolves.
- If it's a **genuinely new grammar**, add its `.sublime-syntax` to the set the crate loads.

Everything stays offline and pure-Rust (fancy-regex, no oniguruma C dependency).

## Keyboard shortcuts

| Action | Shortcut |
| --- | --- |
| New | `Ctrl/Cmd + N` |
| Open | `Ctrl/Cmd + O` |
| Save | `Ctrl/Cmd + S` |
| Save As | `Ctrl/Cmd + Shift + S` |
| Undo / Redo | `Ctrl/Cmd + Z` / `Ctrl/Cmd + Shift + Z` or `Ctrl/Cmd + Y` |
| Find | `Ctrl/Cmd + F` |
| Replace | `Ctrl/Cmd + H` |
| Find next / previous | `F3` / `Shift + F3` |
| Go to line | `Ctrl/Cmd + G` |

> Find, Replace and Go-to-line share one bar; each shortcut opens it with that
> field focused, so you can type straight away.
| Zoom in / out / reset | `Ctrl/Cmd + +` / `-` / `0` |
| Close find bar / About panel | `Esc` |

## Development

Contributing — the process (issues, branches, PRs, CI), the architecture to preserve, the
performance budgets every change is measured against, and the exact checks to run — is in
[`AGENTS.md`](AGENTS.md). It applies to coding agents and humans alike; read it first.

### Prerequisites

- Rust (latest stable) via [rustup](https://rustup.rs/)
- **Linux only** — the editor is software-rendered (no GPU required), but it dlopens the
  X11/Wayland, xkbcommon and fontconfig client libraries at runtime, plus `libdbus-1` for
  the Open/Save dialogs (spoken to the desktop portal; falls back to `zenity` without it).
  On a typical desktop they are already installed; CI adds just:

  ```bash
  sudo apt-get update && sudo apt-get install -y \
    libxkbcommon0 libxkbcommon-x11-0
  ```

  (The packaged `.deb`/`.rpm` declare the full runtime dependency set — see
  [`crates/iced/Cargo.toml`](crates/iced/Cargo.toml).)

Windows and macOS need no extra system packages; they use their native window and dialog backends.

### Run & build

```bash
cargo run                # run the app
cargo run -- notes.txt   # run the app and open files
cargo build --release    # optimised binary → target/release/notepad-extra
```

The built binary is named `notepad-extra`. Native installers are produced by the
`packaging/` scripts (see [Releases](#releases)).

### Tests

```bash
cargo test --package notepad-core --package notepad-syntax   # pure logic: unit + property + stress
cargo test --package notepad-iced --all-targets              # shell wiring + headless UI simulator
scripts/coverage.sh                                          # coverage gate (needs `cargo install cargo-llvm-cov`)
```

Fuzz targets live under `crates/core/fuzz/` (nightly + `cargo-fuzz`). Everything runs in
[`.github/workflows/native-ci.yml`](.github/workflows/native-ci.yml) (the advisory scan in
[`.github/workflows/audit.yml`](.github/workflows/audit.yml)); see
[docs/testing.md](docs/testing.md) for the full test standard and
[docs/native-rendering.md](docs/native-rendering.md) for the render-shell drawing notes.

## Releases

Installers are built and attached automatically **when you publish a GitHub Release**
([`.github/workflows/release.yml`](.github/workflows/release.yml), triggered on `release: published`):

1. Bump the version in **`Cargo.toml`** (`[workspace.package]`), the metainfo `<release>` entry
   ([`packaging/linux/io.github.PierreFouquet.NotepadExtra.metainfo.xml`](packaging/linux/io.github.PierreFouquet.NotepadExtra.metainfo.xml))
   and the man page `.TH` line ([`packaging/linux/notepad-extra.1`](packaging/linux/notepad-extra.1)).
   A test enforces that all three agree, so none can silently drift.
2. Re-check the OS floors in [`packaging/support-floors.toml`](packaging/support-floors.toml)
   against the vendor lifecycle pages it cites (see [Supported OS versions](#supported-os-versions)).
   If a floor has moved, raise it in its own issue first; a test keeps the bundle metadata,
   the `.msi` launch condition and this README in step with that file.
3. On GitHub, **Releases → Draft a new release**, create the tag (e.g. `v0.5.0`), and click **Publish**.
4. CI builds every platform and uploads the bundles onto that release.

> A manual `workflow_dispatch` run is also available; it saves the packages as run artifacts instead.
>
> Only the current release is supported - older versions receive no further fixes.

Artifacts produced:

| Platform | Files | Covers |
| --- | --- | --- |
| Linux x86_64 + aarch64 | `.deb`, `.rpm`, `.AppImage` | Debian 13+ / Ubuntu 22.04+ (deb), Fedora 43+ / openSUSE Leap 16.0+ (rpm), **any distro** with glibc 2.35+ (AppImage) |
| macOS Intel + Apple Silicon | `.dmg` / `.app` | macOS 15 (Sequoia)+ — declared in the `.app` as `LSMinimumSystemVersion` |
| Windows x64 + ARM64 | `.msi` (x64) / portable `.exe` (x64 + ARM64) | Windows 11 23H2+ (build 22631 or newer; the `.msi` refuses older client Windows); ARM64 has no `.msi` — use the portable `.exe` |

> GitHub-hosted ARM runners (`*-arm`) are free for public repositories; on private repos those jobs are billable.

## Supported OS versions

An OS release is supported for exactly as long as its vendor's standard support lasts: the
day a release goes end-of-life it is no longer supported here, and paid or opt-in extensions
(Windows ESU, Debian LTS, Ubuntu ESM) do not count. Nor does the Windows Long-Term Servicing
Channel, which Microsoft reserves for fixed-purpose devices: an LTSC release still in support
does not keep an end-of-life Windows version on the list. The floors therefore move on the
vendors' dates:

| Platform | Minimum | Vendor support ends | Next review |
| --- | --- | --- | --- |
| macOS | macOS 15 (Sequoia)+ — enforced by the `.app`'s `LSMinimumSystemVersion` | Sep 2027 (expected: Apple drops a major when the third one after it ships) | Sep 2027 → macOS 26 |
| Windows | Windows 11 23H2+ (build 22631) — the `.msi` refuses older client Windows; the portable `.exe` has the same floor but does not check it | 2026-11-10 (23H2 Enterprise/Education) | 2026-11-11 → 24H2 (build 26100) |
| Debian | Debian 13 | Aug 2028 (projected: about a year after Debian 14 ships) | Aug 2028 |
| Ubuntu | Ubuntu 22.04 LTS | 2027-05-31 | 2027-04-17 (the build runner retires) → 24.04 |
| Fedora | Fedora 43 | 2026-11-17 (four weeks after Fedora 45 ships) | 2026-11-17 → 44 |
| openSUSE | Leap 16.0 | 2027-10-31 | 2027-10-31 |
| any Linux | glibc 2.35 — the packages are linked on Ubuntu 22.04 | 2027-05-31 (with Ubuntu 22.04) | 2027-04-17 → glibc 2.39 |

Windows Server is not gated (it is not a client SKU), and nothing blocks an install on Linux:
the Linux floors are what the packages are built for and tested on. The floors, their sources
and the review dates live in [`packaging/support-floors.toml`](packaging/support-floors.toml),
the single source of truth; a test keeps the bundle metadata, the `.msi` launch condition, the
build runner and this README in step with it. Raising a floor is its own issue.

## Project layout

```text
AGENTS.md                # Contributor & coding-agent guide: process, architecture, performance model, checks
Cargo.toml               # Cargo workspace root (virtual manifest — shared version & profiles)
crates/syntax/           # Shared syntect-backed language catalogue
crates/core/             # Pure update core (State / Message / Effect) — no window, no GPU
crates/iced/             # Thin iced render shell — builds the `notepad-extra` binary
vendor/cosmic-text/      # cosmic-text 0.15.0 + an unreleased upstream fix, via [patch.crates-io] (#149)
packaging/               # Native packaging scripts + Linux desktop/metainfo/man assets
icons/                   # Application icons (all platforms)
docs/testing.md          # Test standard
docs/native-rendering.md # Render-shell drawing notes & traps
scripts/                 # Helper scripts (coverage gate, licence / packaging / vendored-crate checks)
```

## License

Licensed under the GPL-3.0-or-later License — see [LICENSE](LICENSE) for details.

The combined work includes a small amount of vendored third-party code and one
bundled font under GPL-compatible permissive licenses — notably the iced
`text_editor` widget (MIT), a patched copy of the cosmic-text crate (MIT OR
Apache-2.0) and DejaVu Sans Mono (Bitstream Vera derivative).
[THIRD-PARTY.md](THIRD-PARTY.md) is the canonical record of what is vendored,
from where, and under which license.
