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
- Tauri CLI: `cargo install tauri-cli --version "^2.0.0" --locked`

### Setup & Run

1. Clone the repository
2. Run `cargo tauri dev` to start the application in development mode with hot-reloading.

### Build

Run `cargo tauri build` to compile the final, optimized executable for your current platform. 

## Usage

- Click "Open" to open a text file
- Click "Save" to save the current file
- Click "New" to start a new file
- Select language from dropdown for syntax highlighting

## License

This project is licensed under the GPL-3.0 License - see the [LICENSE](LICENSE) file for details.