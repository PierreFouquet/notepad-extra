use notepad_extra::{read_file_at, write_file_at};
use tempfile::tempdir;
use std::path::Path;

#[test]
fn integration_write_and_read() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("nested").join("i.txt");
    let content = "integration hello";

    let w = write_file_at(content, &file_path);
    assert!(w.is_ok());
    assert!(file_path.exists());

    let r = read_file_at(&file_path).expect("should read");
    assert_eq!(r["content"], content);
}
