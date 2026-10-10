//! Pure text helpers. No I/O, no toolkit — just deterministic string functions
//! the core and shell share.

use std::borrow::Cow;

/// A file's line-ending style. The editor stores text canonically with `\n`
/// (see [`EndOfLine::to_lf`]) and re-applies the original ending on save (see
/// [`EndOfLine::join`]) so bytes round-trip exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EndOfLine {
    /// Unix `\n`.
    #[default]
    Lf,
    /// Windows `\r\n`.
    Crlf,
}

impl EndOfLine {
    /// Detect the dominant line ending: `CRLF` if the content contains any
    /// `\r\n`, otherwise `LF`. Mirrors `logic.js`'s `detectEol`.
    ///
    /// `str::contains` with a two-byte needle is already a memchr-class scan
    /// (measured level with a hand-rolled `find('\r')` loop, and 4× faster on a
    /// lone-`\r` worst case), so this is the single-pass form (#126).
    pub fn detect(content: &str) -> Self {
        if content.contains("\r\n") {
            EndOfLine::Crlf
        } else {
            EndOfLine::Lf
        }
    }

    /// Detect the line ending **and** normalise to canonical `\n` in one step,
    /// taking the buffer by value (#126).
    ///
    /// The load path used to run [`EndOfLine::detect`] and then
    /// [`EndOfLine::to_lf`]: two full scans plus an unconditional copy of the
    /// whole buffer — even for the common LF file, where `to_lf` copied 10+ MB
    /// only to produce an identical string. Here:
    ///
    /// * an LF file is **returned as-is**: no scan beyond the detection pass,
    ///   no copy, the same allocation handed straight back;
    /// * a CRLF file is compacted **in place** (`Vec::dedup_by` drops each `\r`
    ///   that precedes a `\n`, keeping a lone `\r` as before), so the peak
    ///   memory is one buffer rather than two. Removing ASCII bytes cannot break
    ///   UTF-8, so the re-validation on the way back to `String` always succeeds
    ///   (it is the price of `#![forbid(unsafe_code)]`, and far cheaper than a
    ///   second buffer).
    ///
    /// Equivalent to `(detect(&content), to_lf(&content))`, byte for byte.
    pub fn normalize(content: String) -> (Self, String) {
        if !content.contains("\r\n") {
            return (EndOfLine::Lf, content);
        }
        let mut bytes = content.into_bytes();
        // `dedup_by` hands us (`next`, `prev`) where `prev` is the byte already
        // kept: on `\r` followed by `\n` we overwrite the kept `\r` with `\n` and
        // drop the `\n` — the same result as deleting the `\r`, done in one
        // forward pass with no second buffer.
        bytes.dedup_by(|next, prev| {
            if *prev == b'\r' && *next == b'\n' {
                *prev = b'\n';
                true
            } else {
                false
            }
        });
        let text = String::from_utf8(bytes).expect("removing ASCII bytes keeps UTF-8 valid");
        (EndOfLine::Crlf, text)
    }

    /// Normalize any `\r\n` to `\n` for the canonical in-memory buffer.
    ///
    /// Borrowing form of [`EndOfLine::normalize`]: it must return a fresh
    /// `String`, so the one copy is unavoidable — but it is now a plain
    /// `memcpy` followed by the in-place compaction, rather than a searching
    /// `replace` that rebuilt the text segment by segment (#126).
    pub fn to_lf(content: &str) -> String {
        Self::normalize(content.to_owned()).1
    }

    /// Re-join canonical `\n` text with this ending. Mirrors `logic.js`'s
    /// `eolJoin`, so a CRLF file saved back out keeps its CRLF bytes.
    ///
    /// Allocates even for `Lf`, where the result equals the input; callers that
    /// only need to *read* the joined text (the save path encodes it straight to
    /// bytes) should use [`EndOfLine::join_cow`] and skip that copy (#126).
    pub fn join(self, text: &str) -> String {
        self.join_cow(text).into_owned()
    }

    /// [`EndOfLine::join`] without the needless copy: `Lf` borrows `text`
    /// unchanged, `Crlf` builds the re-joined string (#126). `str::replace` with
    /// a `char` needle is the memchr-driven form and measured faster than a
    /// pre-sized split/push loop, so the CRLF arm keeps it.
    pub fn join_cow(self, text: &str) -> Cow<'_, str> {
        match self {
            EndOfLine::Lf => Cow::Borrowed(text),
            EndOfLine::Crlf => Cow::Owned(text.replace('\n', "\r\n")),
        }
    }

    /// Short label for the status bar (#37).
    pub fn label(self) -> &'static str {
        match self {
            EndOfLine::Lf => "LF",
            EndOfLine::Crlf => "CRLF",
        }
    }
}

/// The final path segment, splitting on both `/` and `\` so a Windows path
/// opened on Linux (or vice-versa) still yields the bare file name.
pub fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The lower-relevant extension of a filename (no dot), or `None` for
/// extension-less names and dotfiles (`.bashrc` has no extension).
pub fn extension_of(path: &str) -> Option<&str> {
    let name = basename(path);
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => Some(&name[i + 1..]),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// The pre-#126 bodies, kept as the oracle the single-pass / zero-copy
    /// forms are checked against: `to_lf` was a searching `replace`, `join` an
    /// unconditional copy.
    mod reference {
        pub fn to_lf(content: &str) -> String {
            content.replace("\r\n", "\n")
        }

        pub fn join(crlf: bool, text: &str) -> String {
            if crlf {
                text.replace('\n', "\r\n")
            } else {
                text.to_string()
            }
        }
    }

    #[test]
    fn detect_prefers_crlf_when_present() {
        assert_eq!(EndOfLine::detect("a\r\nb"), EndOfLine::Crlf);
        assert_eq!(EndOfLine::detect("a\nb"), EndOfLine::Lf);
        assert_eq!(EndOfLine::detect(""), EndOfLine::Lf);
        // A lone \r (old Mac) is not CRLF.
        assert_eq!(EndOfLine::detect("a\rb"), EndOfLine::Lf);
    }

    #[test]
    fn crlf_round_trips_through_lf_and_join() {
        let original = "line1\r\nline2\r\n";
        let eol = EndOfLine::detect(original);
        let canonical = EndOfLine::to_lf(original);
        assert_eq!(canonical, "line1\nline2\n");
        assert_eq!(eol.join(&canonical), original);
    }

    #[test]
    fn lf_join_is_identity() {
        assert_eq!(EndOfLine::Lf.join("a\nb\n"), "a\nb\n");
    }

    #[test]
    fn join_cow_borrows_for_lf_and_owns_for_crlf() {
        // The save path's whole point (#126): an LF document is handed back as
        // the very same borrow — no allocation — while CRLF must build new text.
        let text = "a\nb\n";
        let lf = EndOfLine::Lf.join_cow(text);
        assert!(
            matches!(lf, Cow::Borrowed(b) if std::ptr::eq(b, text)),
            "LF join must borrow the input itself, got {lf:?}"
        );
        let crlf = EndOfLine::Crlf.join_cow(text);
        assert!(
            matches!(&crlf, Cow::Owned(s) if s == "a\r\nb\r\n"),
            "CRLF join must produce new text, got {crlf:?}"
        );
    }

    #[test]
    fn normalize_returns_an_lf_buffer_untouched_without_copying() {
        // The common case the load path pays for (#126): no `\r\n` means the
        // *same allocation* comes back — same pointer, same capacity — rather
        // than a byte-identical copy.
        let mut input = String::with_capacity(64);
        input.push_str("one\ntwo\rthree\n"); // a lone \r is not CRLF and is kept
        let (ptr, cap) = (input.as_ptr(), input.capacity());
        let (eol, out) = EndOfLine::normalize(input);
        assert_eq!(eol, EndOfLine::Lf);
        assert_eq!(out, "one\ntwo\rthree\n");
        assert_eq!(out.as_ptr(), ptr, "LF text must not be copied");
        assert_eq!(out.capacity(), cap, "LF text must keep its allocation");
    }

    #[test]
    fn normalize_compacts_crlf_in_place() {
        // CRLF is detected and rewritten in the same buffer: the result is the
        // canonical text and the allocation is reused (pointer unchanged).
        let input = String::from("a\r\nb\r\n\r\nc\r\r\nd\r");
        let ptr = input.as_ptr();
        let (eol, out) = EndOfLine::normalize(input);
        assert_eq!(eol, EndOfLine::Crlf);
        // Each `\r\n` → `\n`; the stray `\r` before `\r\n` and the trailing lone
        // `\r` survive, exactly as `replace("\r\n", "\n")` leaves them.
        assert_eq!(out, "a\nb\n\nc\r\nd\r");
        assert_eq!(out.as_ptr(), ptr, "CRLF text is compacted in place");
        // And the multi-byte neighbours of a CRLF are untouched.
        let (_, out) = EndOfLine::normalize("é\r\n😀\r\n".to_string());
        assert_eq!(out, "é\n😀\n");
    }

    #[test]
    fn basename_handles_both_separators() {
        assert_eq!(basename("/home/p/notes.txt"), "notes.txt");
        assert_eq!(basename(r"C:\Users\p\notes.txt"), "notes.txt");
        assert_eq!(basename("bare.txt"), "bare.txt");
        assert_eq!(basename("/trailing/dir/"), "");
    }

    #[test]
    fn extension_rules() {
        assert_eq!(extension_of("a/b/main.rs"), Some("rs"));
        assert_eq!(extension_of("archive.tar.gz"), Some("gz"));
        assert_eq!(extension_of("Dockerfile"), None);
        assert_eq!(extension_of(".bashrc"), None);
        assert_eq!(extension_of("ends.with.dot."), None);
    }

    #[test]
    fn eol_label_strings() {
        assert_eq!(EndOfLine::Lf.label(), "LF");
        assert_eq!(EndOfLine::Crlf.label(), "CRLF");
    }

    proptest! {
        /// The single-pass owned form agrees with the two-step borrowed one —
        /// and with the pre-#126 `replace` — on any text, including stray `\r`,
        /// `\r\r\n` runs (which, like `replace`, leave a `\r\n` behind: one pass,
        /// no re-scan of its own output) and multi-byte neighbours.
        #[test]
        fn normalize_matches_detect_then_to_lf(s in "[a-c\r\né😀]{0,60}") {
            let (eol, out) = EndOfLine::normalize(s.clone());
            prop_assert_eq!(eol, EndOfLine::detect(&s));
            prop_assert_eq!(&out, &EndOfLine::to_lf(&s));
            prop_assert_eq!(&out, &reference::to_lf(&s));
        }

        /// `join` / `join_cow` are the same text as the pre-#126 copy, for both
        /// endings and any canonical input.
        #[test]
        fn join_cow_matches_join_and_reference(s in "[a-c\n\ré]{0,60}") {
            for (eol, crlf) in [(EndOfLine::Lf, false), (EndOfLine::Crlf, true)] {
                let expected = reference::join(crlf, &s);
                prop_assert_eq!(eol.join(&s), expected.clone());
                let joined = eol.join_cow(&s);
                prop_assert_eq!(joined.as_ref(), expected.as_str());
            }
        }
    }
}
