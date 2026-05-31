use notepad_extra::{read_file_at, write_file_at};
use std::fs;
use tempfile::tempdir;

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

#[test]
fn integration_overwrite_then_read_latest() {
    let dir = tempdir().expect("tempdir");
    let file_path = dir.path().join("doc.txt");

    write_file_at("v1", &file_path).expect("write v1");
    write_file_at("v2 longer content", &file_path).expect("write v2");

    let r = read_file_at(&file_path).expect("read");
    assert_eq!(r["content"], "v2 longer content");
}

#[test]
fn integration_eol_preserved_through_roundtrip() {
    let dir = tempdir().expect("tempdir");

    let lf = dir.path().join("lf.txt");
    write_file_at("a\nb\nc", &lf).expect("write lf");
    assert_eq!(fs::read(&lf).unwrap(), b"a\nb\nc");

    let crlf = dir.path().join("crlf.txt");
    write_file_at("a\r\nb\r\nc", &crlf).expect("write crlf");
    assert_eq!(fs::read(&crlf).unwrap(), b"a\r\nb\r\nc");
}

#[test]
fn integration_read_missing_is_error() {
    let dir = tempdir().expect("tempdir");
    let missing = dir.path().join("does-not-exist.txt");
    assert!(read_file_at(&missing).is_err());
}
