#![no_main]
//! Fuzz the pure string helpers. These run on every file open and on every
//! keystroke-driven title refresh, so they must never panic on any input —
//! including malformed paths, lone `\r`, and huge unicode blobs.

use libfuzzer_sys::fuzz_target;
use notepad_core::EndOfLine;
use notepad_core::{find, text};

fuzz_target!(|s: String| {
    let _ = EndOfLine::detect(&s);
    let canonical = EndOfLine::to_lf(&s);
    let _ = EndOfLine::Lf.join(&canonical);
    let _ = EndOfLine::Crlf.join(&canonical);
    let _ = text::basename(&s);
    let _ = text::extension_of(&s);
    // Language detection (#32): any path string must resolve without panicking.
    let _ = notepad_syntax::detect(&s);

    // Position helpers behind the status bar (#37) and go-to-line: arbitrary
    // byte offsets and (line, column) pairs — including out-of-range and
    // mid-glyph — must clamp to a valid boundary, never panic.
    let _ = find::line_count(&s);
    for off in [0, s.len() / 2, s.len(), s.len().saturating_add(7), usize::MAX] {
        let (line, col) = find::line_col_of(&s, off);
        let _ = find::goto_line_offset(&s, off);
        let _ = find::offset_at(&s, off, off);
        // Round-trip the reported cursor position back to an offset.
        let _ = find::offset_at(&s, line, col);
    }
});
