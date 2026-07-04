use std::fs;
use std::path::Path;

/// Whether `url` is safe to hand to the OS URL handler for the About dialog's
/// external links. It must be an `https` URL with an actual host and no control
/// characters or whitespace — belt-and-braces even though the URL is passed to
/// the launcher as a single argument (never through a shell).
pub fn is_safe_external_url(url: &str) -> bool {
    const PREFIX: &str = "https://";
    url.starts_with(PREFIX)
        && url.len() > PREFIX.len()
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// Read a file at `path` and return a JSON object with `path` and `content`.
pub fn read_file_at(path: &Path) -> Result<serde_json::Value, String> {
    match fs::read_to_string(path) {
        Ok(content) => {
            let path_str = path.to_string_lossy().to_string();
            Ok(serde_json::json!({ "path": path_str, "content": content }))
        }
        Err(e) => Err(format!("Failed to read file: {}", e)),
    }
}

/// Write `content` to `path`, creating parent directories as needed.
pub fn write_file_at(content: &str, path: &Path) -> Result<serde_json::Value, String> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir: {}", e))?;
        }
    }
    fs::write(path, content).map_err(|e| format!("Failed to save file: {}", e))?;
    let path_str = path.to_string_lossy().to_string();
    Ok(serde_json::json!({ "path": path_str }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_write_and_read_roundtrip() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("subdir").join("test.txt");
        let content = "hello world";

        let write_res = write_file_at(content, &file_path);
        assert!(write_res.is_ok(), "write failed: {:?}", write_res);
        assert!(file_path.exists());

        let read_res = read_file_at(&file_path).expect("read should succeed");
        assert_eq!(read_res["content"], content);
    }

    #[test]
    fn test_read_nonexistent() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("nope.txt");
        let read_res = read_file_at(&file_path);
        assert!(read_res.is_err());
    }

    #[test]
    fn test_read_returns_path_and_content_keys() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("keys.txt");
        write_file_at("data", &file_path).expect("write");

        let res = read_file_at(&file_path).expect("read");
        assert_eq!(res["content"], "data");
        assert_eq!(res["path"], file_path.to_string_lossy().to_string());
    }

    #[test]
    fn test_write_returns_path_key() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("out.txt");
        let res = write_file_at("x", &file_path).expect("write");
        assert_eq!(res["path"], file_path.to_string_lossy().to_string());
        assert!(
            res.get("content").is_none(),
            "write result should not echo content"
        );
    }

    #[test]
    fn test_write_creates_nested_directories() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("a").join("b").join("c.txt");
        assert!(!file_path.parent().unwrap().exists());
        write_file_at("nested", &file_path).expect("write");
        assert!(file_path.exists());
        assert_eq!(read_file_at(&file_path).unwrap()["content"], "nested");
    }

    #[test]
    fn test_write_overwrites_existing_file() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("over.txt");
        write_file_at("first", &file_path).expect("write 1");
        write_file_at("second", &file_path).expect("write 2");
        assert_eq!(read_file_at(&file_path).unwrap()["content"], "second");
    }

    #[test]
    fn test_write_empty_content() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("empty.txt");
        write_file_at("", &file_path).expect("write empty");
        assert!(file_path.exists());
        assert_eq!(read_file_at(&file_path).unwrap()["content"], "");
    }

    #[test]
    fn test_unicode_roundtrip() {
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("unicode.txt");
        let content = "café — Ω 漢字 🦀\nsecond line";
        write_file_at(content, &file_path).expect("write");
        assert_eq!(read_file_at(&file_path).unwrap()["content"], content);
    }

    #[test]
    fn test_crlf_bytes_preserved_exactly() {
        // The editor sends CRLF when a file uses CRLF; bytes must be written verbatim.
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("crlf.txt");
        let content = "line1\r\nline2\r\n";
        write_file_at(content, &file_path).expect("write");

        let raw = fs::read(&file_path).expect("read raw bytes");
        assert_eq!(raw, content.as_bytes());
        assert_eq!(read_file_at(&file_path).unwrap()["content"], content);
    }

    #[test]
    fn test_is_safe_external_url_accepts_https() {
        assert!(is_safe_external_url(
            "https://github.com/PierreFouquet/notepad-extra"
        ));
        assert!(is_safe_external_url(
            "https://github.com/PierreFouquet/notepad-extra/issues/new/choose"
        ));
    }

    #[test]
    fn test_is_safe_external_url_rejects_non_https_schemes() {
        assert!(!is_safe_external_url("http://example.com"));
        assert!(!is_safe_external_url("file:///etc/passwd"));
        assert!(!is_safe_external_url("javascript:alert(1)"));
        assert!(!is_safe_external_url("ftp://example.com"));
        assert!(!is_safe_external_url("HTTPS://example.com")); // scheme is case-sensitive here
    }

    #[test]
    fn test_is_safe_external_url_rejects_empty_and_bare_scheme() {
        assert!(!is_safe_external_url(""));
        assert!(!is_safe_external_url("https://")); // no host
    }

    #[test]
    fn test_is_safe_external_url_rejects_whitespace_and_control_chars() {
        assert!(!is_safe_external_url("https://exa mple.com"));
        assert!(!is_safe_external_url("https://example.com\n"));
        assert!(!is_safe_external_url("https://example.com\t/x"));
        assert!(!is_safe_external_url("https://example.com\u{0000}"));
    }

    #[test]
    fn test_read_invalid_utf8_errors() {
        // read_file_at is UTF-8 only; non-UTF-8 input should surface an error, not panic.
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("binary.bin");
        fs::write(&file_path, [0xFF, 0xFE, 0x00, 0x01]).expect("write bytes");
        assert!(read_file_at(&file_path).is_err());
    }

    #[test]
    fn test_read_file_at_accepts_string_path() {
        // Mirrors the `read_file` command (used by drag-and-drop), which receives
        // the dropped path as a String and reads it via read_file_at(Path::new(..)).
        let dir = tempdir().expect("tempdir");
        let file_path = dir.path().join("dropped.txt");
        write_file_at("dropped content", &file_path).expect("write");

        let path_string: String = file_path.to_string_lossy().to_string();
        let res = read_file_at(Path::new(&path_string)).expect("read via string path");
        assert_eq!(res["content"], "dropped content");
        assert_eq!(res["path"], path_string);
    }

    #[test]
    fn test_read_file_at_string_path_missing_errors() {
        // A dropped path that doesn't resolve (e.g. a directory or deleted file)
        // must return an error the frontend can skip, not panic.
        let dir = tempdir().expect("tempdir");
        let missing = dir.path().join("nope.txt").to_string_lossy().to_string();
        assert!(read_file_at(Path::new(&missing)).is_err());
    }
}
