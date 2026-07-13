//! Filesystem + URL helpers, ported from the Tauri app's `src/lib.rs`. These are
//! the *executors* the render shell calls when it performs an [`crate::Effect`];
//! `update` itself never calls them. Return types are plain `Result<_, String>`
//! (no `serde_json`) now that there is no JS bridge to cross.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

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
/// panicking. Kept for the preferences file, which is always UTF-8 JSON;
/// documents open through [`read_file_bytes`] + `encoding::decode` instead so
/// any encoding — including binary — loads (#50/#59).
pub fn read_file(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("Failed to read file: {e}"))
}

/// Read `path` as raw bytes for the multi-encoding document loader (#50/#59).
/// Unlike [`read_file`], this **never** errors on the file's *content*: any byte
/// sequence — a legacy code page, UTF-16, or an outright binary — reads back and
/// is handed to `encoding::decode`. Only genuine I/O failures (missing file, a
/// directory, permission denied) surface an error, so a binary file *opens*
/// (lossily) rather than being rejected.
pub fn read_file_bytes(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path).map_err(|e| format!("Failed to read file: {e}"))
}

/// Write raw `bytes` to `path`, creating parent directories as needed — the
/// byte-level counterpart to [`write_file`] for documents saved in a chosen
/// encoding (#50). Callers encode the EOL-joined text with
/// `encoding::encode_for_save` first (which also blocks a lossy save), so line
/// endings and the encoding's BOM round-trip exactly.
pub fn write_file_bytes(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir: {e}"))?;
    }
    fs::write(path, bytes).map_err(|e| format!("Failed to save file: {e}"))
}

/// Write `content` to `path` verbatim, creating parent directories as needed.
/// Callers pass EOL-joined bytes (see [`crate::EndOfLine::join`]) so line
/// endings round-trip exactly.
pub fn write_file(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir: {e}"))?;
    }
    fs::write(path, content).map_err(|e| format!("Failed to save file: {e}"))
}

/// The sibling temp path an atomic write stages into: `{path}.tmp` in the *same*
/// directory, so the follow-up rename stays on one filesystem (the only place
/// rename is atomic). Appending — not replacing the extension — keeps it beside
/// the real file (`preferences.json` → `preferences.json.tmp`).
fn tmp_sibling(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".tmp");
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(name),
        _ => PathBuf::from(name),
    }
}

/// Write `content` to `path` atomically: stage it in a sibling temp file, flush
/// that to disk, then `rename` it over `path`. Because the rename is atomic on a
/// single filesystem, a crash or power-cut mid-write leaves either the old file
/// or the complete new one — never the truncated half-write a plain in-place
/// [`write_file`] can produce. Used for the preferences file (#96), where a torn
/// write would otherwise reset every setting to defaults (`Preferences::from_json`
/// recovers to defaults on invalid JSON). A failed rename cleans up the temp so a
/// stray `.tmp` is never left behind.
pub fn write_file_atomic(path: &Path, content: &str) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir: {e}"))?;
    }
    let tmp = tmp_sibling(path);
    // Write and fsync the temp before the rename, so the bytes are durable on
    // disk by the time the rename publishes them.
    let write = (|| {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()
    })();
    if let Err(e) = write {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Failed to save file: {e}"));
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp); // don't leave a stray temp on a failed rename
        format!("Failed to save file: {e}")
    })
}

/// The platform command — program name and argument list — that hands `url` to
/// the OS's default handler (the user's own browser), or `None` when `url` is
/// not a [safe external URL](is_safe_external_url).
///
/// Split out from [`open_external`] so the platform mapping is pure and testable
/// on its own, separately from the [`spawn_detached`] step that actually launches
/// the handler. The app itself never touches the network — it only asks the OS to
/// open the link (#40).
///
/// * **Windows:** `rundll32 url.dll,FileProtocolHandler <url>`.
/// * **macOS:** `open <url>`.
/// * **Linux/other:** `xdg-open <url>`.
fn opener_argv(url: &str) -> Option<(&'static str, Vec<String>)> {
    if !is_safe_external_url(url) {
        return None;
    }
    #[cfg(target_os = "windows")]
    {
        Some((
            "rundll32.exe",
            vec!["url.dll,FileProtocolHandler".to_string(), url.to_string()],
        ))
    }
    #[cfg(target_os = "macos")]
    {
        Some(("open", vec![url.to_string()]))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Some(("xdg-open", vec![url.to_string()]))
    }
}

/// Open `url` in the user's default browser via the OS handler, for the About
/// dialog's external links (#40). Refuses anything that is not a
/// [safe external URL](is_safe_external_url) — so a non-`https` or malformed URL
/// can never reach a shell command — and spawns the handler *detached* (never
/// waiting on it), so a slow browser can't stall the editor. The app makes no
/// network request of its own; it only delegates to the OS.
pub fn open_external(url: &str) -> Result<(), String> {
    open_external_with(url, spawn_detached)
}

/// [`open_external`] with the process spawn injected, so the URL guard and the
/// argv wiring can be exercised without launching a real browser. The public
/// wrapper passes [`spawn_detached`]; tests pass a stand-in that records what it
/// receives (or forces a failure) instead of touching the OS.
fn open_external_with(
    url: &str,
    spawn: impl FnOnce(&str, &[String]) -> Result<(), String>,
) -> Result<(), String> {
    let (program, args) =
        opener_argv(url).ok_or_else(|| format!("Refusing to open unsafe URL: {url}"))?;
    spawn(program, &args)
}

/// Spawn `program` with `args` as a *detached* child — launched but never waited
/// on, so a slow browser can't stall the editor — mapping a launch failure to an
/// error string. Split from [`open_external`] so this last OS-touching step can be
/// driven in tests with a harmless command instead of a real URL handler.
fn spawn_detached(program: &str, args: &[String]) -> Result<(), String> {
    std::process::Command::new(program)
        .args(args)
        .spawn()
        .map(|_child| ()) // detached: don't wait on the handler
        .map_err(|e| format!("Failed to open link: {e}"))
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
    fn tmp_sibling_uses_the_current_dir_when_path_has_no_parent() {
        let path = PathBuf::from("preferences.json");
        assert_eq!(tmp_sibling(&path), PathBuf::from("preferences.json.tmp"));
    }

    #[test]
    fn write_reports_an_error_when_a_parent_cannot_be_created() {
        let dir = tempdir().expect("tempdir");
        let blocked = dir.path().join("blocked");
        fs::write(&blocked, "x").expect("seed a file");
        let path = blocked.join("nested").join("child.txt");
        assert!(write_file(&path, "data").is_err());
    }

    #[test]
    fn atomic_write_then_read_roundtrip_and_creates_dirs() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("cfg").join("preferences.json");
        write_file_atomic(&path, "{\"v\":1}").expect("write");
        assert_eq!(read_file(&path).expect("read"), "{\"v\":1}");
    }

    #[test]
    fn atomic_write_reports_an_error_when_the_temp_file_cannot_be_created() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("preferences.json");
        let tmp = dir.path().join("preferences.json.tmp");
        fs::create_dir(&tmp).expect("reserve the temp path as a directory");
        assert!(write_file_atomic(&path, "data").is_err());
    }

    #[test]
    fn atomic_write_overwrites_and_leaves_no_temp() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("preferences.json");
        write_file_atomic(&path, "first").expect("w1");
        write_file_atomic(&path, "second").expect("w2");
        assert_eq!(read_file(&path).expect("read"), "second");
        // The staged temp is renamed away, never left beside the real file.
        assert!(!dir.path().join("preferences.json.tmp").exists());
    }

    #[test]
    fn atomic_write_survives_a_torn_write() {
        // #96: model a crash *after* the temp is staged but *before* the rename —
        // the temp holds a half-written new snapshot, the rename never happened.
        // Because the previous file is only replaced by the atomic rename, readers
        // still see the last good bytes, not a truncated file that would reset to
        // defaults.
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("preferences.json");
        write_file_atomic(&path, "GOOD").expect("commit good");
        fs::write(dir.path().join("preferences.json.tmp"), "TORN").expect("stage torn temp");
        assert_eq!(read_file(&path).expect("read"), "GOOD");
    }

    #[test]
    fn atomic_write_over_a_directory_errors_and_keeps_no_temp() {
        // If the target path is a directory, the rename can't publish over it: the
        // write surfaces an error (never a panic) and cleans up its staged temp.
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("preferences.json");
        fs::create_dir(&target).expect("make target a dir");
        assert!(write_file_atomic(&target, "data").is_err());
        assert!(!dir.path().join("preferences.json.tmp").exists());
    }

    #[test]
    fn read_missing_errors() {
        let dir = tempdir().expect("tempdir");
        assert!(read_file(&dir.path().join("nope.txt")).is_err());
    }

    #[test]
    fn read_file_as_utf8_still_errors_on_non_utf8() {
        // The UTF-8 text reader stays strict — it backs the preferences file,
        // which must be valid UTF-8 JSON. Documents take the byte path instead.
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("binary.bin");
        fs::write(&path, [0xFF, 0xFE, 0x00, 0x01]).expect("write bytes");
        assert!(read_file(&path).is_err());
    }

    #[test]
    fn read_file_bytes_reads_arbitrary_bytes_back_never_errors_on_content() {
        // #59: a non-UTF-8 / binary file must *open*. The byte reader returns the
        // exact bytes (for `encoding::decode` to interpret) rather than erroring
        // the way the UTF-8 text reader does.
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("binary.bin");
        let bytes = [0xFF, 0xFE, 0x00, 0x01, 0x80, 0x9F];
        fs::write(&path, bytes).expect("write bytes");
        assert_eq!(read_file_bytes(&path).expect("read bytes"), bytes);
    }

    #[test]
    fn read_file_bytes_still_errors_on_io_failure() {
        // Only genuine I/O — a missing file or a directory — surfaces an error;
        // never the file's content.
        let dir = tempdir().expect("tempdir");
        assert!(read_file_bytes(&dir.path().join("nope.bin")).is_err());
        assert!(read_file_bytes(dir.path()).is_err());
    }

    #[test]
    fn write_file_bytes_roundtrips_and_creates_dirs() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("nested").join("doc.txt");
        let bytes = [0xEF, 0xBB, 0xBF, b'h', b'i']; // UTF-8 BOM + "hi"
        write_file_bytes(&path, &bytes).expect("write");
        assert_eq!(fs::read(&path).expect("raw"), bytes);
    }

    #[test]
    fn write_file_bytes_reports_an_error_when_a_parent_cannot_be_created() {
        let dir = tempdir().expect("tempdir");
        let blocked = dir.path().join("blocked");
        fs::write(&blocked, "x").expect("seed a file");
        let path = blocked.join("nested").join("child.bin");
        assert!(write_file_bytes(&path, b"data").is_err());
    }

    #[test]
    fn write_file_bytes_reports_an_error_when_the_write_itself_fails() {
        // The target's parent already exists (so `create_dir_all` is skipped), but
        // the path names an existing directory, so the write can't succeed — the
        // `fs::write` error surfaces rather than panicking.
        let dir = tempdir().expect("tempdir");
        assert!(write_file_bytes(dir.path(), b"data").is_err());
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

    #[test]
    fn write_through_a_file_as_directory_errors() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("iamafile");
        fs::write(&file, "x").expect("seed file");
        // A regular file cannot be a parent directory: create_dir_all must fail
        // and surface an error rather than panicking.
        let nested = file.join("sub").join("child.txt");
        assert!(write_file(&nested, "data").is_err());
    }

    #[test]
    fn read_a_directory_errors_not_panics() {
        // Reading a directory as if it were a file is a bad-path case (like a
        // permission-denied or missing file): it must surface an error through
        // the same `Result`, never panic (#29). Root-independent, unlike a
        // chmod-based permission test.
        let dir = tempdir().expect("tempdir");
        assert!(read_file(dir.path()).is_err());
    }

    // ---- External-link opener (#40) ----

    #[test]
    fn opener_argv_builds_a_command_for_safe_https() {
        // Whatever the platform, a safe https URL maps to *some* handler command
        // whose final argument is the URL verbatim (the program itself differs
        // per OS: xdg-open / open / rundll32).
        let url = "https://github.com/PierreFouquet/notepad-extra";
        let (program, args) = opener_argv(url).expect("safe url maps to a command");
        assert!(!program.is_empty());
        assert_eq!(args.last().map(String::as_str), Some(url));
    }

    #[test]
    fn opener_argv_rejects_unsafe_urls() {
        // The pure guard that keeps a non-https or malformed URL from ever
        // reaching a shell command.
        assert!(opener_argv("http://example.com").is_none());
        assert!(opener_argv("file:///etc/passwd").is_none());
        assert!(opener_argv("javascript:alert(1)").is_none());
        assert!(opener_argv("").is_none());
    }

    #[test]
    fn open_external_refuses_unsafe_urls_without_spawning() {
        // The public entry point returns an error for an unsafe URL — and,
        // because the guard runs *before* the spawn, never launches a process.
        assert!(open_external("javascript:alert(1)").is_err());
        assert!(open_external("http://example.com").is_err());
        assert!(open_external("").is_err());
    }

    #[test]
    // `&spy` is deliberate: reusing one stand-in across both calls exercises its
    // body on the safe path (a fresh closure per call would leave the never-run
    // one uncovered). Clippy's needless-borrow heuristic doesn't see the reuse.
    #[allow(clippy::needless_borrows_for_generic_args)]
    fn open_external_spawns_iff_the_url_is_safe() {
        // The URL guard runs *before* the spawn: an unsafe URL is rejected without
        // the spawn ever being reached, while a safe URL forwards its platform argv
        // (from `opener_argv`) to the spawn. One call-counting stand-in proves both,
        // without launching a real handler.
        let calls = std::cell::Cell::new(0u32);
        let forwarded = std::cell::RefCell::new(None);
        let spy = |program: &str, args: &[String]| {
            calls.set(calls.get() + 1);
            assert!(!program.is_empty());
            *forwarded.borrow_mut() = args.last().cloned();
            Ok(())
        };

        // Unsafe URL: the guard rejects it and the spawn is never invoked.
        assert!(open_external_with("javascript:alert(1)", &spy).is_err());
        assert_eq!(calls.get(), 0, "spawn must not run for an unsafe URL");

        // Safe URL: the spawn runs exactly once and receives the URL as its last arg.
        let url = "https://github.com/PierreFouquet/notepad-extra";
        assert!(open_external_with(url, &spy).is_ok());
        assert_eq!(calls.get(), 1, "spawn runs once for a safe URL");
        assert_eq!(forwarded.borrow().as_deref(), Some(url));
    }

    #[test]
    fn open_external_propagates_spawn_failure() {
        // A spawn failure (e.g. the handler binary is missing) surfaces through
        // the same `Result` rather than panicking.
        let result = open_external_with(
            "https://example.com/ok",
            |_program: &str, _args: &[String]| Err("boom".to_string()),
        );
        assert_eq!(result, Err("boom".to_string()));
    }

    #[test]
    fn spawn_detached_launches_a_harmless_command() {
        // The real spawn path: a no-op system command launches and detaches
        // cleanly, returning Ok without waiting on the child.
        #[cfg(windows)]
        let (program, args) = ("cmd", vec!["/C".to_string(), "exit".to_string()]);
        #[cfg(not(windows))]
        let (program, args) = ("true", Vec::<String>::new());
        assert!(spawn_detached(program, &args).is_ok());
    }

    #[test]
    fn spawn_detached_maps_launch_failure_to_error() {
        // A program that cannot be launched yields an error string, never a panic.
        assert!(spawn_detached("notepad-extra-nonexistent-program-xyz", &[]).is_err());
    }
}
