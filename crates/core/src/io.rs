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
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create dir: {e}"))?;
    }
    fs::write(path, content).map_err(|e| format!("Failed to save file: {e}"))
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
    fn open_external_forwards_safe_url_argv_to_spawn() {
        // On the safe-URL path the entry point hands the spawn step the platform
        // argv from `opener_argv` — verified without launching a real handler by
        // capturing what the injected spawn receives.
        let url = "https://github.com/PierreFouquet/notepad-extra";
        let spawned = std::cell::Cell::new(false);
        let result = open_external_with(url, |program: &str, args: &[String]| {
            assert!(!program.is_empty());
            assert_eq!(args.last().map(String::as_str), Some(url));
            spawned.set(true);
            Ok(())
        });
        assert!(result.is_ok());
        assert!(spawned.get(), "spawn must run for a safe URL");
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
    fn open_external_guard_runs_before_spawn() {
        // The URL guard rejects an unsafe URL *before* the spawn is reached, so
        // the injected spawn (which would panic) is never invoked.
        let result = open_external_with("javascript:alert(1)", |_: &str, _: &[String]| {
            panic!("spawn must not run for an unsafe URL");
        });
        assert!(result.is_err());
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
