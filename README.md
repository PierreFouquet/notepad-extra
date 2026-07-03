# Notepad Extra

Notepad but extra. A small, fast, **fully offline** text/code editor for Windows, macOS and Linux,
built with Rust and [Tauri](https://tauri.app/). Inspired by Notepad++.

## Features

- Cross-platform: Windows (x64 + ARM64), macOS (Intel + Apple Silicon), Linux (x64 + ARM64)
- **Works fully offline** — CodeMirror is vendored locally, no network access at runtime
- Tabbed editing with unsaved-change indicators
- **Syntax highlighting for 80+ languages** — C/C++/C#, Java, JavaScript/TypeScript/JSX, Python, Go, Rust, Ruby, PHP, Swift, Kotlin, HTML/CSS/SCSS, Markdown, SQL, YAML/TOML, shell, PowerShell, Haskell, and many more, grouped in the language menu (all modes vendored locally)
- Automatic language detection from the file extension
- **Find / Replace / Go-to-line** (`Ctrl+F` / `Ctrl+H` / `Ctrl+G`, `F3` for next) with regex & case options
- **Word wrap** toggle
- **Zoom** the editor font (`Ctrl++` / `Ctrl+-` / `Ctrl+0`)
- **Light / Dark theme** toggle (remembers your choice, along with wrap & zoom)
- **Status bar**: line/column, selection length, document length & line count, EOL style, encoding
- **About dialog** with version, license and links (opens in your own browser; the app never fetches anything itself)
- Preserves a file's original line endings (LF / CRLF) on save
- Open / Save / **Save As** with broad file-type filters
- Line numbers, bracket matching, active-line highlight

### Adding a language

All languages are defined in one table — `LANGUAGES` in [`src-tauri/dist/logic.js`](src-tauri/dist/logic.js).
Add a row (value, label, group, extensions), drop the matching CodeMirror mode file
under `src-tauri/dist/vendor/codemirror/mode/`, add it to `MODE_SCRIPTS`, then run
`node scripts/gen-index.js` to regenerate the dropdown and `<script>` tags in `index.html`.

## Keyboard shortcuts

| Action | Shortcut |
| --- | --- |
| New | `Ctrl/Cmd + N` |
| Open | `Ctrl/Cmd + O` |
| Save | `Ctrl/Cmd + S` |
| Save As | `Ctrl/Cmd + Shift + S` |
| Find | `Ctrl/Cmd + F` |
| Replace | `Ctrl/Cmd + H` |
| Find next / previous | `F3` / `Shift + F3` |
| Go to line | `Ctrl/Cmd + G` |
| Zoom in / out / reset | `Ctrl/Cmd + +` / `-` / `0` |

## Development

### Prerequisites

- Rust (latest stable) via [rustup](https://rustup.rs/)
- Tauri CLI: `cargo install tauri-cli --version "^2.0.0" --locked`
- Node.js (only to run the frontend logic tests — **not** required to build the app)
- Linux only — system libraries:

  ```bash
  sudo apt-get update && sudo apt-get install -y \
    libwebkit2gtk-4.1-dev build-essential curl wget file \
    libssl-dev libayatana-appindicator3-dev librsvg2-dev patchelf libfuse2
  ```

### Run & build

```bash
cargo tauri dev     # run with hot-reload
cargo tauri build   # build optimized installers for the current platform
```

### Tests

```bash
cargo test                          # Rust backend (file I/O, EOL handling, error cases)
node --test tests/frontend/*.test.js   # frontend logic (language/EOL/path helpers)
```

Both suites also run automatically in CI (`.github/workflows/ci.yml`).

## Releases

Installers are built and attached automatically **when you publish a GitHub
Release** (`.github/workflows/release.yml`, triggered on `release: published`):

1. Bump the version in `Cargo.toml` and `tauri.conf.json`.
2. On GitHub, **Releases → Draft a new release**, create the tag (e.g. `v0.2.0`),
   and click **Publish**.
3. CI builds every platform and uploads the bundles onto that release.

> A manual `workflow_dispatch` run is also available; it builds into a fresh draft release instead.

Artifacts produced:

| Platform | Files | Covers |
| --- | --- | --- |
| Linux x86_64 + aarch64 | `.deb`, `.rpm`, `.AppImage` | Debian/Ubuntu (deb), Fedora/RHEL/openSUSE (rpm), **any distro incl. Arch** (AppImage) |
| macOS Intel + Apple Silicon | `.dmg` / `.app` | macOS 10.15+ |
| Windows x64 + ARM64 | `.msi` / NSIS `.exe` | Windows 10/11 |

> GitHub-hosted ARM runners (`*-arm`) are free for public repositories; on private repos those jobs are billable.

## Project layout

```text
Cargo.toml            # Rust crate (root)
build.rs              # Tauri build script
tauri.conf.json       # Tauri app configuration
capabilities/         # Tauri v2 capability/permission files
src/                  # Rust: main.rs (commands) + lib.rs (file I/O)
src-tauri/dist/       # Frontend (HTML/CSS/JS) + vendored CodeMirror (offline)
tests/                # Rust integration tests + frontend logic tests
```

## License

Licensed under the GPL-3.0 License — see [LICENSE](LICENSE) for details.
