#![no_main]
//! Fuzz the pure string helpers. These run on every file open and on every
//! keystroke-driven title refresh, so they must never panic on any input —
//! including malformed paths, lone `\r`, and huge unicode blobs.

use libfuzzer_sys::fuzz_target;
use notepad_core::EndOfLine;
use notepad_core::{lang, text};

fuzz_target!(|s: String| {
    let _ = EndOfLine::detect(&s);
    let canonical = EndOfLine::to_lf(&s);
    let _ = EndOfLine::Lf.join(&canonical);
    let _ = EndOfLine::Crlf.join(&canonical);
    let _ = text::basename(&s);
    let _ = text::extension_of(&s);
    let _ = lang::language_for_path(&s);
});
