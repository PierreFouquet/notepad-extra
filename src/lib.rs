use std::fs;
use std::path::Path;

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
}
