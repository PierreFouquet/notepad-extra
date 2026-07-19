//! Filesystem-watch plumbing for the external-change feature (#51).
//!
//! The pure reload/conflict *decision* logic lives in [`notepad_core`]; this
//! module is the edge that turns real `notify` filesystem events into the shell's
//! [`crate::Message::DiskEvent`]s. It bridges `notify`'s callback thread to an
//! async stream and coalesces bursts — an editor's temp-file + rename save (and
//! our own [`notepad_core::io`] atomic write) fires several events in a rush —
//! into one debounced batch per file, so a save never triggers a reload storm.
//!
//! Nothing here decides anything: it only reports *which* watched files changed.
//! The core compares fingerprints and owns the reload/conflict state machine.

use notepad_core::DiskMeta;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long the filesystem must go quiet before a change batch is emitted. An
/// atomic save (write temp, then rename over the target) fires a burst of
/// create/modify/rename/remove events; coalescing them within this window turns
/// the burst into a single "it changed" rather than a reload flurry.
pub const WATCH_DEBOUNCE: Duration = Duration::from_millis(300);

/// Fingerprint bytes already in hand (just read, or about to be written) together
/// with `path`'s mtime and length, forming the [`DiskMeta`] baseline the core
/// compares against. The `hash` is the authority on content; mtime/len are the
/// shell's cheap gate before it bothers to re-hash on a watch event.
pub fn fingerprint(path: &Path, bytes: &[u8]) -> DiskMeta {
    DiskMeta {
        modified: std::fs::metadata(path).and_then(|m| m.modified()).ok(),
        len: bytes.len() as u64,
        hash: hash_bytes(bytes),
    }
}

/// A stable hash of `bytes` (std `DefaultHasher`). Not cryptographic — it only
/// distinguishes "same content" from "changed content", so a collision could at
/// worst miss a change, never corrupt data.
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

/// Given the directories an event burst touched, return the watched files that
/// live in them — our *own* stored paths, so the shell always matches and re-stats
/// them. Matching by directory (not by exact event path) is what makes the watch
/// robust: an atomic rename writes a temp file, and a symlinked open file (e.g.
/// `CLAUDE.md -> AGENTS.md`) is written under its *target's* name — both land in
/// the watched directory under a name that isn't the one we opened, yet both must
/// still trigger a re-check of the file we hold open.
fn files_in_touched_dirs(
    dirs_hit: &HashSet<PathBuf>,
    by_dir: &HashMap<PathBuf, Vec<PathBuf>>,
) -> Vec<PathBuf> {
    dirs_hit
        .iter()
        .flat_map(|d| by_dir.get(d).into_iter().flatten().cloned())
        .collect()
}

/// A path's parent directory in canonical form (symlinks resolved), falling back
/// to the plain parent when it can't be canonicalized (e.g. it no longer exists).
///
/// Canonicalising is what keeps directory matching consistent across backends:
/// macOS FSEvents reports realpaths (a temp dir under `/var/folders` surfaces as
/// `/private/var/folders`), so the watched directory and the event paths must be
/// compared in the same canonical form or they never match.
fn canonical_parent(p: &Path) -> Option<PathBuf> {
    p.parent().map(|parent| {
        parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf())
    })
}

/// Record which *watched* directories an event burst touched (an event path whose
/// canonical parent is a directory we watch), so [`files_in_touched_dirs`] can map
/// them back to the open files to re-check.
fn note_touched_dirs(
    dirs_hit: &mut HashSet<PathBuf>,
    paths: Vec<PathBuf>,
    by_dir: &HashMap<PathBuf, Vec<PathBuf>>,
) {
    for p in paths {
        if let Some(dir) = canonical_parent(&p)
            && by_dir.contains_key(&dir)
        {
            dirs_hit.insert(dir);
        }
    }
}

/// Start watching `files`. Returns a receiver of debounced change batches (each a
/// non-empty set of the *open* files whose directory saw activity) and the live
/// watcher — which the caller **must keep alive** for as long as it wants events;
/// dropping it stops the watch and lets the debounce thread exit. `None` if a
/// watcher could not be created at all.
///
/// Each file's **parent directory** is watched (non-recursive), not the file node,
/// and events are mapped back to open files by directory (see
/// [`files_in_touched_dirs`]) — so atomic rename-replaces, deletes, and writes to
/// a symlink's target are all caught, and the shell then re-stats the file it
/// actually holds open. Access-only events (open/read/close-without-write) are
/// ignored so the shell's own fingerprint reads don't feed back as fresh changes.
pub fn spawn_watch(
    files: Vec<PathBuf>,
) -> Option<(
    iced::futures::channel::mpsc::UnboundedReceiver<Vec<PathBuf>>,
    RecommendedWatcher,
)> {
    // Group the watched files by the (canonical) directory we'll watch for each.
    // Keying by the canonical directory — and watching it — keeps the watch and
    // the event paths in the same path form on every backend (see
    // [`canonical_parent`]); the stored values stay the *original* opened paths,
    // which is what the shell holds tabs and baselines for.
    let mut by_dir: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for f in &files {
        if let Some(dir) = canonical_parent(f) {
            by_dir.entry(dir).or_default().push(f.clone());
        }
    }

    // notify's callback thread → a std channel of raw event paths.
    let (raw_tx, raw_rx) = std::sync::mpsc::channel::<Vec<PathBuf>>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<Event>| {
        if let Ok(event) = res {
            // Ignore access-only events (open / read / close-without-write): the
            // shell reads a file to fingerprint it, which would otherwise re-arm
            // the watch in an endless feedback loop.
            if matches!(event.kind, EventKind::Access(_)) {
                return;
            }
            // Sending fails only once the debounce thread has exited (its receiver
            // dropped), which is exactly when we no longer care — ignore the error.
            let _ = raw_tx.send(event.paths);
        }
    })
    .ok()?;
    for dir in by_dir.keys() {
        // Best-effort: a directory we cannot watch simply reports no changes for
        // the files under it, rather than failing the whole watch.
        let _ = watcher.watch(dir, RecursiveMode::NonRecursive);
    }

    // Debounce thread: coalesce a burst into one batch after a quiet window, then
    // emit the open files whose directory saw activity. It exits when either
    // channel closes — the watcher is dropped (raw side disconnects) or the
    // subscription is torn down (the batch receiver is dropped).
    let (batch_tx, batch_rx) = iced::futures::channel::mpsc::unbounded::<Vec<PathBuf>>();
    std::thread::spawn(move || {
        while let Ok(first) = raw_rx.recv() {
            let mut dirs_hit = HashSet::new();
            note_touched_dirs(&mut dirs_hit, first, &by_dir);
            // Keep draining until the filesystem goes quiet for a full window (a
            // `Timeout`) or the watcher goes away (`Disconnected`); either ends the
            // `while let`.
            while let Ok(more) = raw_rx.recv_timeout(WATCH_DEBOUNCE) {
                note_touched_dirs(&mut dirs_hit, more, &by_dir);
            }
            let hits = files_in_touched_dirs(&dirs_hit, &by_dir);
            if !hits.is_empty() && batch_tx.unbounded_send(hits).is_err() {
                break; // the subscription was torn down; stop.
            }
        }
    });

    Some((batch_rx, watcher))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An event on *any* file in a watched directory — including a sibling that we
    /// did not open, such as a symlink's target (`CLAUDE.md -> AGENTS.md`) or an
    /// atomic-save temp file — maps back to the open file(s) in that directory.
    #[test]
    fn a_sibling_change_maps_back_to_the_open_file_in_that_dir() {
        let mut by_dir: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        by_dir.insert(PathBuf::from("/t"), vec![PathBuf::from("/t/CLAUDE.md")]);
        by_dir.insert(
            PathBuf::from("/other"),
            vec![PathBuf::from("/other/note.txt")],
        );

        let mut dirs_hit = HashSet::new();
        // The real write lands on the symlink target `AGENTS.md`, a sibling we
        // never opened — but it shares the watched directory.
        note_touched_dirs(&mut dirs_hit, vec![PathBuf::from("/t/AGENTS.md")], &by_dir);
        // An event in a directory we don't watch is ignored.
        note_touched_dirs(
            &mut dirs_hit,
            vec![PathBuf::from("/elsewhere/x.txt")],
            &by_dir,
        );

        assert_eq!(dirs_hit, [PathBuf::from("/t")].into_iter().collect());
        // We re-check the file we actually opened, not the sibling event path.
        assert_eq!(
            files_in_touched_dirs(&dirs_hit, &by_dir),
            vec![PathBuf::from("/t/CLAUDE.md")]
        );
    }

    #[test]
    fn fingerprint_same_bytes_same_hash_and_len() {
        // Same content hashes identically (the touch-vs-edit distinction); the
        // path need not exist — `modified` just falls back to `None`.
        let a = fingerprint(Path::new("/does/not/exist"), b"hello");
        let b = fingerprint(Path::new("/nor/this"), b"hello");
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.len, 5);
        assert_ne!(a.hash, fingerprint(Path::new("/x"), b"hello!").hash);
    }

    /// End-to-end over the real filesystem: a modification to a watched file must
    /// surface as a debounced batch naming that file. Exercises the actual
    /// `notify` backend, the parent-directory watch, the debounce thread and the
    /// watched-set filter together.
    #[test]
    fn spawn_watch_reports_a_real_modification() {
        use std::io::Write;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("watched.txt");
        std::fs::write(&path, b"initial").expect("seed the file");

        // Start watching *after* seeding, so only the modification below is seen.
        let (mut rx, _watcher) = spawn_watch(vec![path.clone()]).expect("a watcher");

        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .and_then(|mut f| f.write_all(b" more"))
            .expect("append to the watched file");

        // Poll for a batch mentioning our file. Generous deadline: a real inotify
        // event plus the 300ms debounce window, kept robust on slow CI.
        use iced::futures::channel::mpsc::TryRecvError;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut seen = false;
        while std::time::Instant::now() < deadline {
            match rx.try_recv() {
                Ok(paths) if paths.contains(&path) => {
                    seen = true;
                    break;
                }
                Ok(_) => {} // some other batch; keep waiting
                Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(50)),
                Err(TryRecvError::Closed) => break,
            }
        }
        assert!(
            seen,
            "a real modification to a watched file should surface as a batch"
        );
    }

    /// The exact regression that shipped broken first: the open file is a symlink
    /// (`link.md -> real.md`) and the editor writes the *target*. Watching by
    /// directory and re-emitting the opened path must still surface it — matching
    /// the event's own path (`real.md`) against the opened path (`link.md`) would
    /// miss it, which is why the bar never appeared.
    #[cfg(unix)]
    #[test]
    fn a_write_through_a_symlink_target_still_surfaces_the_open_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let real = dir.path().join("real.md");
        let link = dir.path().join("link.md");
        std::fs::write(&real, b"initial").expect("seed the real file");
        std::os::unix::fs::symlink(&real, &link).expect("symlink link.md -> real.md");

        // We open the *symlink*, as the user opened `CLAUDE.md -> AGENTS.md`.
        let (mut rx, _watcher) = spawn_watch(vec![link.clone()]).expect("a watcher");

        // The editor writes the target `real.md` — a sibling we never named.
        std::fs::write(&real, b"initial + external edit").expect("write the target");

        use iced::futures::channel::mpsc::TryRecvError;
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let mut seen = false;
        while std::time::Instant::now() < deadline {
            match rx.try_recv() {
                // The batch names the file we *opened* (the symlink), not the event
                // path — that is what the shell holds a tab and a baseline for.
                Ok(paths) if paths.contains(&link) => {
                    seen = true;
                    break;
                }
                Ok(_) => {}
                Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(50)),
                Err(TryRecvError::Closed) => break,
            }
        }
        assert!(
            seen,
            "editing a symlink's target must surface the opened symlink path"
        );
    }

    /// Dropping the watcher stops the watch: after it is gone, a later change
    /// produces no further batches and the channel winds down.
    #[test]
    fn dropping_the_watcher_stops_events() {
        use std::io::Write;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("watched.txt");
        std::fs::write(&path, b"initial").expect("seed the file");

        let (mut rx, watcher) = spawn_watch(vec![path.clone()]).expect("a watcher");
        drop(watcher); // stop watching

        // Give the teardown a moment, then change the file.
        std::thread::sleep(Duration::from_millis(100));
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .and_then(|mut f| f.write_all(b" more"))
            .expect("append after dropping the watcher");

        // No batch naming the file should arrive; the channel closes once the
        // debounce thread notices the raw side is gone.
        use iced::futures::channel::mpsc::TryRecvError;
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            match rx.try_recv() {
                Ok(paths) => assert!(
                    !paths.contains(&path),
                    "no events should arrive after the watcher is dropped"
                ),
                Err(TryRecvError::Closed) => break, // clean shutdown we expect
                Err(TryRecvError::Empty) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }
}
