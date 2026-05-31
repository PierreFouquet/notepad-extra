# Notepad Extra

Notepad but extra. A small, fast, **fully offline** text/code editor for Windows, macOS and Linux,
built with Rust and [Tauri](https://tauri.app/). Inspired by Notepad++.

## Features

- Cross-platform: Windows (x64 + ARM64), macOS (Intel + Apple Silicon), Linux (x64 + ARM64)
- **Works fully offline** — CodeMirror is vendored locally, no network access at runtime
- Tabbed editing with unsaved-change indicators
- Syntax highlighting: Plain Text, JavaScript, JSON, Rust, Python, C, C++, Java, Markdown, HTML, XML, CSS, Shell, YAML
- **Find / Replace / Go-to-line** (`Ctrl+F` / `Ctrl+H` / `Ctrl+G`, `F3` for next) with regex & case options
- **Word wrap** toggle
- **Zoom** the editor font (`Ctrl++` / `Ctrl+-` / `Ctrl+0`)
- **Light / Dark theme** toggle (remembers your choice, along with wrap & zoom)
- **Status bar**: line/column, selection length, document length & line count, EOL style, encoding
- Preserves a file's original line endings (LF / CRLF) on save
- Open / Save / **Save As** with broad file-type filters
- Line numbers, bracket matching, active-line highlight

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

Pushing a version tag builds and publishes installers for every platform via
`.github/workflows/release.yml`:

```bash
# bump the version in Cargo.toml and tauri.conf.json first, then:
git tag v0.2.0
git push origin v0.2.0
```

The workflow produces, as a draft GitHub release:

- **Linux**: `.deb`, `.rpm`, `.AppImage` (x86_64 and aarch64)
- **macOS**: `.dmg` / `.app` (Intel and Apple Silicon)
- **Windows**: `.msi` / `.exe` NSIS installer (x64 and ARM64)

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
