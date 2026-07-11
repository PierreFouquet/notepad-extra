//! The status-bar readout (#37): caret line/column, selection length, document
//! length & line count, EOL style, and encoding.
//!
//! It is a **pure function** of the active [`Document`] plus the caret /
//! selection the render shell reads from its editor widget — no window, no I/O —
//! so the whole readout is asserted headlessly (epic #25's testing standard).
//! The shell only converts its cursor to byte offsets (via
//! [`crate::find::offset_at`]) and renders the returned [`StatusBar`]; all the
//! counting lives here.
//!
//! Every count is **character**-based, not byte-based: a multi-byte glyph is a
//! single column and a single selected character, which is what a user expects
//! (bytes would over-count `é`, `😀`, …).

use crate::app::Document;
use crate::find;

/// The text encoding shown in the status bar. Fixed to UTF-8 for now — the core
/// reads and writes UTF-8 only (see [`crate::io::read_file`]). Per-document
/// encoding detection/selection is tracked separately in #50, which will thread
/// a real encoding through [`status`] in place of this constant.
pub const ENCODING: &str = "UTF-8";

/// A snapshot of everything the status bar displays, derived purely from a
/// document's text and the caret / selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusBar {
    /// 1-based line of the caret.
    pub line: usize,
    /// 1-based column of the caret, counted in characters.
    pub column: usize,
    /// Number of characters currently selected (`0` when there is no selection).
    pub selection: usize,
    /// Total number of characters in the document.
    pub chars: usize,
    /// Total number of lines. A trailing newline counts a final empty line,
    /// matching [`find::line_count`] and the editor widget's own line count.
    pub lines: usize,
    /// Line-ending style label: `"LF"` or `"CRLF"`.
    pub eol: &'static str,
    /// Encoding label (currently always [`ENCODING`]).
    pub encoding: &'static str,
    /// Language label (e.g. `"Rust"`, `"Plain Text"`).
    pub language: &'static str,
}

/// Compute the [`StatusBar`] for `doc` with the caret at byte offset `caret` and
/// the other end of the selection, if any, at byte offset `anchor`.
///
/// Offsets are clamped into range and onto char boundaries, so any value the
/// widget supplies — stale, mid-glyph, or out of range — is safe: this never
/// panics. `anchor == Some(caret)` (an empty selection) reports `selection: 0`.
pub fn status(doc: &Document, caret: usize, anchor: Option<usize>) -> StatusBar {
    let text = &doc.content;
    let (line, column) = char_position(text, caret);
    StatusBar {
        line: line + 1,
        column: column + 1,
        selection: anchor.map_or(0, |a| char_span(text, a, caret)),
        chars: text.chars().count(),
        lines: find::line_count(text),
        eol: doc.eol.label(),
        encoding: ENCODING,
        language: doc.language(),
    }
}

/// 0-based (line, character-column) of byte `offset`, clamped into range and
/// onto a char boundary. Unlike [`find::line_col_of`] (which returns a *byte*
/// column for the widget's cursor), the column here counts characters, so a
/// multi-byte glyph advances it by one.
fn char_position(text: &str, offset: usize) -> (usize, usize) {
    let offset = find::floor_boundary(text, offset);
    let prefix = &text[..offset];
    let line = prefix.bytes().filter(|&b| b == b'\n').count();
    let line_start = prefix.rfind('\n').map_or(0, |i| i + 1);
    let column = text[line_start..offset].chars().count();
    (line, column)
}

/// Number of characters in the byte range between `a` and `b` (order doesn't
/// matter), with both ends clamped onto char boundaries.
fn char_span(text: &str, a: usize, b: usize) -> usize {
    let lo = find::floor_boundary(text, a.min(b));
    let hi = find::floor_boundary(text, a.max(b));
    text[lo..hi].chars().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::History;
    use crate::text::EndOfLine;
    use proptest::prelude::*;

    /// A throwaway document with the given text, LF endings and a plain-text
    /// label — enough to exercise the status readout.
    fn doc(content: &str) -> Document {
        doc_with(content, EndOfLine::Lf, "Plain Text")
    }

    fn doc_with(content: &str, eol: EndOfLine, language: &'static str) -> Document {
        Document {
            id: 1,
            path: None,
            content: content.to_string(),
            eol,
            detected_lang: language,
            manual_lang: None,
            history: History::new(),
            saved_content: content.to_string(),
        }
    }

    #[test]
    fn empty_document_starts_at_line_one_column_one() {
        let s = status(&doc(""), 0, None);
        assert_eq!(s.line, 1);
        assert_eq!(s.column, 1);
        assert_eq!(s.selection, 0);
        assert_eq!(s.chars, 0);
        assert_eq!(s.lines, 1);
    }

    #[test]
    fn caret_reports_one_based_line_and_column() {
        let text = "one\ntwo\nthree";
        // Offset 6 is the third char of "two" (o), i.e. "tw|o" -> line 2, col 3.
        let s = status(&doc(text), 6, None);
        assert_eq!((s.line, s.column), (2, 3));
        // Start of line 2.
        assert_eq!(status(&doc(text), 4, None).line, 2);
        assert_eq!(status(&doc(text), 4, None).column, 1);
    }

    #[test]
    fn column_counts_characters_not_bytes() {
        // "😀" is four UTF-8 bytes; the caret just after it is column 2, not 5.
        let text = "😀x";
        let after_emoji = "😀".len(); // 4
        let s = status(&doc(text), after_emoji, None);
        assert_eq!(s.column, 2);
        // ...and the caret at end of "😀x" is column 3.
        assert_eq!(status(&doc(text), text.len(), None).column, 3);
    }

    #[test]
    fn selection_length_counts_characters_and_is_order_independent() {
        let text = "héllo"; // 5 chars, 'é' is two bytes -> 6 bytes total
        let end = text.len();
        assert_eq!(status(&doc(text), end, Some(0)).selection, 5);
        // Anchor and caret swapped: same length.
        assert_eq!(status(&doc(text), 0, Some(end)).selection, 5);
        // An empty selection (anchor == caret) counts nothing.
        assert_eq!(status(&doc(text), 3, Some(3)).selection, 0);
        // No anchor at all: nothing selected.
        assert_eq!(status(&doc(text), 3, None).selection, 0);
    }

    #[test]
    fn document_length_and_line_count() {
        let text = "a\nbb\nccc"; // 8 chars, 3 lines
        let s = status(&doc(text), 0, None);
        assert_eq!(s.chars, 8);
        assert_eq!(s.lines, 3);
    }

    #[test]
    fn trailing_newline_counts_a_final_empty_line() {
        let text = "a\n";
        let s = status(&doc(text), text.len(), None);
        assert_eq!(s.lines, 2);
        assert_eq!(s.line, 2); // caret on the empty final line
        assert_eq!(s.column, 1);
    }

    #[test]
    fn eol_encoding_and_language_pass_through() {
        let s = status(&doc_with("x", EndOfLine::Crlf, "Rust"), 0, None);
        assert_eq!(s.eol, "CRLF");
        assert_eq!(s.language, "Rust");
        assert_eq!(s.encoding, "UTF-8");
    }

    #[test]
    fn out_of_range_and_mid_char_offsets_are_clamped_never_panic() {
        let text = "abé"; // 'é' occupies bytes 2..4
        // Caret far past the end clamps to end-of-text.
        let s = status(&doc(text), 999, Some(999));
        assert_eq!(s.column, 4); // 3 chars -> column 4
        assert_eq!(s.selection, 0); // both ends clamp to the same spot
        // A mid-'é' offset floors to the 'é' boundary (column 3, not a panic).
        assert_eq!(status(&doc(text), 3, None).column, 3);
        // Selection across a mid-glyph offset floors both ends.
        assert_eq!(status(&doc(text), 3, Some(0)).selection, 2);
    }

    proptest! {
        /// `status` never panics on any (text, caret, anchor) and every field is
        /// internally consistent: the caret line is within the line count, and
        /// the selection can't exceed the document's character count.
        #[test]
        fn status_is_always_sane(
            text in ".{0,300}",
            caret in 0usize..400,
            anchor in proptest::option::of(0usize..400),
        ) {
            let d = doc(&text);
            let s = status(&d, caret, anchor);
            prop_assert!(s.line >= 1);
            prop_assert!(s.column >= 1);
            prop_assert_eq!(s.chars, text.chars().count());
            prop_assert_eq!(s.lines, find::line_count(&text));
            prop_assert!(s.line <= s.lines);
            prop_assert!(s.selection <= s.chars);
            prop_assert_eq!(s.eol, "LF");
            prop_assert_eq!(s.encoding, ENCODING);
        }
    }
}
