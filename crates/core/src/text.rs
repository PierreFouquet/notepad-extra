//! Pure text helpers ported from the WebView app's `logic.js`. No I/O, no
//! toolkit — just deterministic string functions the core and shell share.

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
    pub fn detect(content: &str) -> Self {
        if content.contains("\r\n") {
            EndOfLine::Crlf
        } else {
            EndOfLine::Lf
        }
    }

    /// Normalize any `\r\n` to `\n` for the canonical in-memory buffer.
    pub fn to_lf(content: &str) -> String {
        content.replace("\r\n", "\n")
    }

    /// Re-join canonical `\n` text with this ending. Mirrors `logic.js`'s
    /// `eolJoin`, so a CRLF file saved back out keeps its CRLF bytes.
    pub fn join(self, text: &str) -> String {
        match self {
            EndOfLine::Lf => text.to_string(),
            EndOfLine::Crlf => text.replace('\n', "\r\n"),
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
    path.rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or(path)
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
}
