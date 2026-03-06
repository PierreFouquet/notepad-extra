# Notepad Extra

Notepad but extra. For Windows, Mac, and Linux, built with Rust and Tauri.

## Features

- Cross-platform support (Linux, macOS, Windows)
- Syntax highlighting for multiple languages (JavaScript, Rust, Markdown, HTML, CSS)
- Open and save files
- Monokai theme
- Line numbers

## Development

### Prerequisites

- Rust (latest stable)
- Node.js and npm (for frontend dependencies, if needed)
- Tauri CLI: `cargo install tauri-cli --version "~1.5"`

### Setup

1. Clone the repository
2. Run `cargo tauri dev` to start development mode

### Build

Run `cargo tauri build` to build the application for your platform.

## Run test

Run `cargo run` to test running the application.

## Usage

- Click "Open" to open a text file
- Click "Save" to save the current file
- Click "New" to start a new file
- Select language from dropdown for syntax highlighting

## License

This project is licensed under the GPL-3.0 License - see the [LICENSE](LICENSE) file for details.
