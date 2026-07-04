//! Filesystem + URL helpers, ported from the Tauri app's `src/lib.rs`. These are
//! the *executors* the render shell calls when it performs an [`crate::Effect`];
//! `update` itself never calls them. Return types are plain `Result<_, String>`
//! (no `serde_json`) now that there is no JS bridge to cross.

use std::fs;
use std::path::Path;

/// Whether `url` is safe to hand to the OS URL handler for the About dialog's
/// external links (#40). It must be an `https` URL with an actual host and no
/// control characters or whitespace.
pub fn is_safe_external_url(url: &str) -> bool {
    const PREFIX: &str = "https://";
    url.starts_with(PREFIX)
        && url.len() > PREFIX.len()
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// Read `path` as UTF-8 text. Non-UTF-8 input surfaces an error rather than
/// panicking (multi-encoding support is tracked separately, #50/#59).
pub fn read_file(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("Failed to read file: {e}"))
}

/// Write `content` to `path` verbatim, creating parent directories as needed.
/// Callers pass EOL-joined bytes (see [`crate::EndOfLine::join`]) so line
/// endings round-trip exactly.
pub fn write_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir: {e}"))?;
        }
    }
    fs::write(path, content).map_err(|e| format!("Failed to save file: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn write_then_read_roundtrip() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("subdir").join("test.txt");
        write_file(&path, "hello world").expect("write");
        assert!(path.exists());
        assert_eq!(read_file(&path).expect("read"), "hello world");
    }

    #[test]
    fn write_creates_nested_dirs() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("a").join("b").join("c.txt");
        assert!(!path.parent().unwrap().exists());
        write_file(&path, "nested").expect("write");
        assert_eq!(read_file(&path).expect("read"), "nested");
    }

    #[test]
    fn write_overwrites() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("over.txt");
        write_file(&path, "first").expect("w1");
        write_file(&path, "second").expect("w2");
        assert_eq!(read_file(&path).expect("read"), "second");
    }

    #[test]
    fn crlf_bytes_preserved_exactly() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("crlf.txt");
        let content = "line1\r\nline2\r\n";
        write_file(&path, content).expect("write");
        assert_eq!(fs::read(&path).expect("raw"), content.as_bytes());
    }

    #[test]
    fn read_missing_errors() {
        let dir = tempdir().expect("tempdir");
        assert!(read_file(&dir.path().join("nope.txt")).is_err());
    }

    #[test]
    fn read_non_utf8_errors_not_panics() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("binary.bin");
        fs::write(&path, [0xFF, 0xFE, 0x00, 0x01]).expect("write bytes");
        assert!(read_file(&path).is_err());
    }

    #[test]
    fn safe_url_accepts_https_only() {
        assert!(is_safe_external_url(
            "https://github.com/PierreFouquet/notepad-extra"
        ));
        assert!(!is_safe_external_url("http://example.com"));
        assert!(!is_safe_external_url("file:///etc/passwd"));
        assert!(!is_safe_external_url("javascript:alert(1)"));
        assert!(!is_safe_external_url("https://")); // no host
        assert!(!is_safe_external_url("https://exa mple.com")); // whitespace
        assert!(!is_safe_external_url("https://example.com\n")); // control
        assert!(!is_safe_external_url("")); // empty
    }
}
