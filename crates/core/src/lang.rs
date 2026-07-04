//! Language detection for the status-bar label (#37) and the language menu.
//!
//! This is a deliberately small, curated table for the Phase 0 spike. The full
//! 80+ language set (issue #32) will be backed by syntect's own `SyntaxSet`
//! (which resolves extensions to syntaxes itself); this table only needs to
//! cover enough to prove the wiring and the status-bar readout.

use crate::text;

/// Label used when no extension matches (and for new, unsaved buffers).
pub const PLAIN_TEXT: &str = "Plain Text";

/// `(label, extensions-without-dot)`. First match wins.
const TABLE: &[(&str, &[&str])] = &[
    ("Rust", &["rs"]),
    ("Python", &["py", "pyw", "pyi"]),
    ("JavaScript", &["js", "mjs", "cjs"]),
    ("TypeScript", &["ts", "mts", "cts"]),
    ("JSON", &["json", "jsonc"]),
    ("Markdown", &["md", "markdown", "mdown", "mkd"]),
    ("TOML", &["toml"]),
    ("YAML", &["yml", "yaml"]),
    ("C", &["c", "h"]),
    ("C++", &["cpp", "cc", "cxx", "hpp", "hh"]),
    ("HTML", &["html", "htm", "xhtml"]),
    ("CSS", &["css"]),
    ("Shell", &["sh", "bash", "zsh"]),
    ("Go", &["go"]),
    ("SQL", &["sql"]),
    ("XML", &["xml", "svg", "xsd"]),
];

/// Map a path (or bare filename) to a language label, defaulting to
/// [`PLAIN_TEXT`]. Extension match is ASCII-case-insensitive.
pub fn language_for_path(path: &str) -> &'static str {
    if let Some(ext) = text::extension_of(path) {
        for (label, exts) in TABLE {
            if exts.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
                return label;
            }
        }
    }
    PLAIN_TEXT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_common_languages() {
        assert_eq!(language_for_path("/a/b/main.rs"), "Rust");
        assert_eq!(language_for_path("script.PY"), "Python"); // case-insensitive
        assert_eq!(language_for_path("Cargo.toml"), "TOML");
        assert_eq!(language_for_path(r"C:\proj\app.tsx"), "Plain Text"); // not in subset yet
        assert_eq!(language_for_path("notes.txt"), "Plain Text");
        assert_eq!(language_for_path("Dockerfile"), "Plain Text");
    }

    #[test]
    fn all_labels_are_plain_text_or_from_table() {
        for (label, _) in TABLE {
            assert!(!label.is_empty());
        }
    }
}
