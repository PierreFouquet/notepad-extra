//! Line & text operations (#54): sort / reverse / de-duplicate / blank-strip
//! lines, change case, trim trailing whitespace, and convert between tabs and
//! spaces.
//!
//! Every operation is a **pure function of text** — no window, no I/O — so the
//! whole feature is asserted headlessly (epic #25's testing standard). The
//! render shell reads the live selection from its editor widget, hands the byte
//! range to [`apply_to`], and resyncs the rewritten buffer; the *decision* logic
//! (which region to touch, how to transform it) lives here.
//!
//! ## Region scoping
//!
//! Each op runs on the **selection if there is one, else the whole document**
//! (the [`apply_to`] entry point). Line-structured ops ([`TextOp::is_line_op`])
//! grow a partial selection out to whole lines first — you can't sort half a
//! line — while character ops touch exactly the selected span.
//!
//! ## Line endings
//!
//! The in-memory buffer is canonically `\n` (CRLF is re-applied only on save,
//! see [`crate::text::EndOfLine`]), so the line ops split and re-join on `\n`
//! with no CRLF handling. A single trailing newline is treated as the *last
//! line's terminator*, not an extra empty line — matching `sort(1)` — so sorting
//! `"b\na\n"` yields `"a\nb\n"` rather than sprouting a leading blank line.

use crate::find::floor_boundary;
use unicode_segmentation::UnicodeSegmentation;

/// The default tab width used when converting between tabs and spaces. The pure
/// converters take the width as a parameter (so a future width preference can
/// feed them), but the shell has no width control yet and passes this.
pub const DEFAULT_TAB_WIDTH: usize = 4;

/// One line/text transformation. `Copy` — every variant is a plain enum or a
/// couple of `bool`/`usize` fields — so the shell threads it by value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextOp {
    /// Sort lines. `descending` flips the order; `case_insensitive` folds case
    /// for the comparison (the lines themselves keep their original case).
    /// Ordering is by Unicode scalar value, not locale collation.
    SortLines {
        descending: bool,
        case_insensitive: bool,
    },
    /// Reverse the order of the lines.
    ReverseLines,
    /// Drop every line that duplicates an earlier one, keeping the first
    /// occurrence and the original order.
    RemoveDuplicateLines,
    /// Drop lines that are empty or contain only whitespace.
    RemoveBlankLines,
    /// Map every character to upper case.
    Uppercase,
    /// Map every character to lower case.
    Lowercase,
    /// Capitalise the first letter of each word and lower-case the rest, using
    /// Unicode word boundaries (UAX #29) so `don't` → `Don't`, not `Don'T`.
    TitleCase,
    /// Swap the case of every cased character (`aBc` → `AbC`).
    ToggleCase,
    /// Strip trailing spaces and tabs from every line.
    TrimTrailingWhitespace,
    /// Replace each tab with `width` spaces.
    TabsToSpaces { width: usize },
    /// Fold every run of `width` consecutive spaces into one tab, leaving the
    /// remainder as spaces. Applies throughout the region, not just leading
    /// indentation (the "(All)" behaviour).
    SpacesToTabs { width: usize },
}

/// The outcome of [`apply_to`]: the rewritten document plus the byte range the
/// transformed region now occupies, which the shell can re-select.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// The whole document after splicing the transformed region back in.
    pub content: String,
    /// The `[start, end)` byte range the transformed region now spans.
    pub selection: (usize, usize),
}

impl TextOp {
    /// Whether this op operates on whole lines. Line ops grow a partial
    /// selection out to line boundaries in [`region`]; character ops don't.
    pub fn is_line_op(self) -> bool {
        matches!(
            self,
            TextOp::SortLines { .. }
                | TextOp::ReverseLines
                | TextOp::RemoveDuplicateLines
                | TextOp::RemoveBlankLines
                | TextOp::TrimTrailingWhitespace
        )
    }

    /// Transform an isolated region of text (the substring [`apply_to`] carved
    /// out). Pure and total: it never panics and never looks past `text`.
    pub fn apply(self, text: &str) -> String {
        match self {
            TextOp::SortLines {
                descending,
                case_insensitive,
            } => map_lines(text, |lines| {
                sort_lines(lines, descending, case_insensitive)
            }),
            TextOp::ReverseLines => map_lines(text, |mut lines| {
                lines.reverse();
                lines
            }),
            TextOp::RemoveDuplicateLines => map_lines(text, remove_duplicate_lines),
            TextOp::RemoveBlankLines => map_lines(text, |lines| {
                lines.into_iter().filter(|l| !l.trim().is_empty()).collect()
            }),
            TextOp::Uppercase => text.to_uppercase(),
            TextOp::Lowercase => text.to_lowercase(),
            TextOp::TitleCase => title_case(text),
            TextOp::ToggleCase => toggle_case(text),
            TextOp::TrimTrailingWhitespace => map_lines(text, |lines| {
                lines
                    .into_iter()
                    .map(|l| l.trim_end_matches([' ', '\t']))
                    .collect()
            }),
            TextOp::TabsToSpaces { width } => text.replace('\t', &" ".repeat(width)),
            TextOp::SpacesToTabs { width } => spaces_to_tabs(text, width),
        }
    }
}

/// Apply `op` to `content`, restricted to the region implied by `selection`.
///
/// `selection` is a byte range from the editor; `None` or an empty range means
/// "the whole document". The range is clamped onto char boundaries, so any
/// value the widget supplies — stale, reversed, mid-glyph, out of range — is
/// safe. Line ops widen a partial selection to whole lines first.
pub fn apply_to(content: &str, op: TextOp, selection: Option<(usize, usize)>) -> Applied {
    let (start, end) = region(content, op, selection);
    let transformed = op.apply(&content[start..end]);
    let mut new_content = String::with_capacity(content.len() - (end - start) + transformed.len());
    new_content.push_str(&content[..start]);
    new_content.push_str(&transformed);
    let new_end = new_content.len();
    new_content.push_str(&content[end..]);
    Applied {
        content: new_content,
        selection: (start, new_end),
    }
}

/// The `[start, end)` region of `content` an op transforms. A non-empty
/// selection scopes it; anything else means the whole document. Line ops widen
/// the span to cover every line it touches.
fn region(content: &str, op: TextOp, selection: Option<(usize, usize)>) -> (usize, usize) {
    let (a, b) = match selection {
        Some((x, y)) => {
            let lo = floor_boundary(content, x.min(y));
            let hi = floor_boundary(content, x.max(y));
            if lo < hi {
                (lo, hi)
            } else {
                (0, content.len()) // an empty selection acts on the whole document
            }
        }
        None => (0, content.len()),
    };
    if op.is_line_op() {
        let start = content[..a].rfind('\n').map_or(0, |i| i + 1);
        let end = content[b..].find('\n').map_or(content.len(), |i| b + i);
        (start, end)
    } else {
        (a, b)
    }
}

/// Split `text` into lines, hand them to `f`, and re-join. A single trailing
/// newline is preserved as the final line's terminator (see the module docs), so
/// the transform sees the real lines and never the empty tail `split('\n')`
/// would otherwise append.
fn map_lines<'a, F>(text: &'a str, f: F) -> String
where
    F: FnOnce(Vec<&'a str>) -> Vec<&'a str>,
{
    let (body, trailing) = match text.strip_suffix('\n') {
        Some(body) => (body, "\n"),
        None => (text, ""),
    };
    let lines = f(body.split('\n').collect());
    let mut out = lines.join("\n");
    out.push_str(trailing);
    out
}

/// Stable sort of `lines` by Unicode scalar order, optionally case-folded for
/// the comparison and/or descending. Case folding compares on a lower-cased key
/// computed once per line (not once per comparison), and the returned lines keep
/// their original casing.
fn sort_lines(lines: Vec<&str>, descending: bool, case_insensitive: bool) -> Vec<&str> {
    let mut lines = lines;
    if case_insensitive {
        // Decorate with a folded key so `to_lowercase` runs once per line.
        let mut keyed: Vec<(String, &str)> =
            lines.into_iter().map(|l| (l.to_lowercase(), l)).collect();
        keyed.sort_by(|(x, _), (y, _)| x.cmp(y));
        if descending {
            keyed.reverse();
        }
        keyed.into_iter().map(|(_, l)| l).collect()
    } else {
        lines.sort_unstable();
        if descending {
            lines.reverse();
        }
        lines
    }
}

/// Drop lines that duplicate an earlier one, keeping the first occurrence and
/// the original order.
fn remove_duplicate_lines(lines: Vec<&str>) -> Vec<&str> {
    let mut seen = std::collections::HashSet::new();
    lines.into_iter().filter(|l| seen.insert(*l)).collect()
}

/// Title-case: capitalise the first character of each Unicode word segment and
/// lower-case the rest. Non-word segments (spaces, punctuation) pass through a
/// no-op, so the exact spacing and punctuation are preserved.
fn title_case(text: &str) -> String {
    text.split_word_bounds()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect()
}

/// Swap the case of every cased character, leaving case-less characters (digits,
/// punctuation, most CJK) untouched.
fn toggle_case(text: &str) -> String {
    text.chars()
        .flat_map(|c| {
            if c.is_uppercase() {
                c.to_lowercase().collect::<Vec<_>>()
            } else if c.is_lowercase() {
                c.to_uppercase().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

/// Fold every run of `width` consecutive spaces into a tab, leaving the
/// remainder as spaces. A `width` of `0` is a no-op guard (never divides by
/// zero). Runs never cross a non-space character, so newlines break them.
fn spaces_to_tabs(text: &str, width: usize) -> String {
    if width == 0 {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut run = 0usize;
    for ch in text.chars() {
        if ch == ' ' {
            run += 1;
            if run == width {
                out.push('\t');
                run = 0;
            }
        } else {
            out.extend(std::iter::repeat_n(' ', run));
            run = 0;
            out.push(ch);
        }
    }
    out.extend(std::iter::repeat_n(' ', run));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Convenience: apply an op to the whole of `text`.
    fn whole(op: TextOp, text: &str) -> String {
        apply_to(text, op, None).content
    }

    const ASC: TextOp = TextOp::SortLines {
        descending: false,
        case_insensitive: false,
    };
    const DESC: TextOp = TextOp::SortLines {
        descending: true,
        case_insensitive: false,
    };
    const ASC_CI: TextOp = TextOp::SortLines {
        descending: false,
        case_insensitive: true,
    };

    #[test]
    fn sort_ascending_and_descending() {
        assert_eq!(whole(ASC, "banana\napple\ncherry"), "apple\nbanana\ncherry");
        assert_eq!(
            whole(DESC, "banana\napple\ncherry"),
            "cherry\nbanana\napple"
        );
    }

    #[test]
    fn sort_preserves_a_single_trailing_newline_like_coreutils() {
        // The trailing newline terminates the last line rather than becoming a
        // leading blank line after the sort.
        assert_eq!(whole(ASC, "b\na\n"), "a\nb\n");
        assert_eq!(whole(ASC, "b\na"), "a\nb");
    }

    #[test]
    fn case_insensitive_sort_folds_case_but_keeps_original_casing() {
        assert_eq!(
            whole(ASC_CI, "Banana\napple\nCherry"),
            "apple\nBanana\nCherry"
        );
        // Case-sensitive would sort the capitals first.
        assert_eq!(whole(ASC, "Banana\napple\nCherry"), "Banana\nCherry\napple");
    }

    #[test]
    fn reverse_lines() {
        assert_eq!(whole(TextOp::ReverseLines, "a\nb\nc"), "c\nb\na");
        assert_eq!(whole(TextOp::ReverseLines, "a\nb\nc\n"), "c\nb\na\n");
    }

    #[test]
    fn remove_duplicate_lines_keeps_first_occurrence_and_order() {
        assert_eq!(
            whole(TextOp::RemoveDuplicateLines, "a\nb\na\nc\nb"),
            "a\nb\nc"
        );
    }

    #[test]
    fn remove_blank_lines_drops_empty_and_whitespace_only() {
        assert_eq!(
            whole(TextOp::RemoveBlankLines, "a\n\nb\n   \n\t\nc"),
            "a\nb\nc"
        );
    }

    #[test]
    fn case_conversions() {
        assert_eq!(whole(TextOp::Uppercase, "Hello, World"), "HELLO, WORLD");
        assert_eq!(whole(TextOp::Lowercase, "Hello, World"), "hello, world");
        assert_eq!(whole(TextOp::ToggleCase, "Hello, World"), "hELLO, wORLD");
    }

    #[test]
    fn title_case_uses_word_boundaries() {
        assert_eq!(whole(TextOp::TitleCase, "hello world"), "Hello World");
        assert_eq!(whole(TextOp::TitleCase, "HELLO WORLD"), "Hello World");
        // An apostrophe stays inside the word (UAX #29), so it is not a boundary.
        assert_eq!(whole(TextOp::TitleCase, "don't stop"), "Don't Stop");
        // A hyphen is a boundary, so both halves capitalise.
        assert_eq!(whole(TextOp::TitleCase, "well-known"), "Well-Known");
    }

    #[test]
    fn trim_trailing_whitespace_per_line() {
        assert_eq!(
            whole(TextOp::TrimTrailingWhitespace, "a  \nb\t\n  c \n"),
            "a\nb\n  c\n"
        );
        // Idempotent.
        let once = whole(TextOp::TrimTrailingWhitespace, "a  \nb\t");
        assert_eq!(whole(TextOp::TrimTrailingWhitespace, &once), once);
    }

    #[test]
    fn tabs_and_spaces_round_trip_for_pure_indentation() {
        let w = DEFAULT_TAB_WIDTH;
        let spaced = whole(TextOp::TabsToSpaces { width: w }, "\tx\n\t\ty");
        assert_eq!(spaced, "    x\n        y");
        let tabbed = whole(TextOp::SpacesToTabs { width: w }, &spaced);
        assert_eq!(tabbed, "\tx\n\t\ty");
    }

    #[test]
    fn spaces_to_tabs_leaves_a_short_remainder_as_spaces() {
        // Five spaces at width 4 -> one tab + one space.
        assert_eq!(whole(TextOp::SpacesToTabs { width: 4 }, "     x"), "\t x");
        // A width of zero is a guarded no-op.
        assert_eq!(whole(TextOp::SpacesToTabs { width: 0 }, "   x"), "   x");
    }

    #[test]
    fn selection_scopes_the_op_and_line_ops_widen_to_whole_lines() {
        let text = "zebra\napple\nmango\n";
        // Select only inside "apple" (bytes 6..8, "pp"): a line op still sorts
        // the whole lines the selection touches — here just the one line, so no
        // change — proving it widened rather than sorting the two characters.
        let applied = apply_to(text, ASC, Some((6, 8)));
        assert_eq!(applied.content, text);

        // Select from inside line 1 to inside line 2: sorts those two lines,
        // leaving line 3 untouched.
        let applied = apply_to(text, ASC, Some((2, 8)));
        assert_eq!(applied.content, "apple\nzebra\nmango\n");
    }

    #[test]
    fn character_op_touches_exactly_the_selection() {
        let text = "hello world";
        // Upper-case only "world".
        let applied = apply_to(text, TextOp::Uppercase, Some((6, 11)));
        assert_eq!(applied.content, "hello WORLD");
        // The re-select range covers the transformed span.
        assert_eq!(applied.selection, (6, 11));
    }

    #[test]
    fn reversed_and_out_of_range_selection_never_panics() {
        let text = "abé"; // 'é' spans bytes 2..4
        // Reversed order and a wildly out-of-range end both clamp safely.
        let applied = apply_to(text, TextOp::Uppercase, Some((999, 0)));
        assert_eq!(applied.content, "ABÉ");
        // A mid-glyph offset floors onto the boundary rather than panicking.
        let _ = apply_to(text, TextOp::Uppercase, Some((3, 3)));
    }

    #[test]
    fn empty_document_is_left_empty() {
        assert_eq!(whole(ASC, ""), "");
        assert_eq!(whole(TextOp::RemoveBlankLines, ""), "");
        assert_eq!(whole(TextOp::Uppercase, ""), "");
    }

    proptest! {
        /// A sort is a permutation: same multiset of lines, just reordered, and
        /// the result is idempotent.
        #[test]
        fn sort_is_a_permutation_and_idempotent(text in "[a-c\n]{0,40}") {
            let sorted = whole(ASC, &text);
            let mut before: Vec<&str> = text.split('\n').collect();
            let mut after: Vec<&str> = sorted.split('\n').collect();
            before.sort_unstable();
            after.sort_unstable();
            prop_assert_eq!(before, after);
            prop_assert_eq!(whole(ASC, &sorted), sorted);
        }

        /// De-duplication is idempotent and its output is a subsequence of the
        /// input with no repeats.
        #[test]
        fn dedupe_is_idempotent_subset(text in "[a-c\n]{0,40}") {
            let once = whole(TextOp::RemoveDuplicateLines, &text);
            prop_assert_eq!(whole(TextOp::RemoveDuplicateLines, &once), once.clone());
            // Compare the *logical* lines — strip a single trailing newline first,
            // exactly as `map_lines` does, so the terminator isn't miscounted as a
            // second empty line. No logical line may equal an earlier one.
            let body = once.strip_suffix('\n').unwrap_or(&once);
            let mut seen = std::collections::HashSet::new();
            for l in body.split('\n') {
                prop_assert!(seen.insert(l), "duplicate line survived: {:?}", l);
            }
        }

        /// Splicing preserves the untouched prefix and suffix around the region,
        /// and never panics on any text / selection.
        #[test]
        fn apply_to_preserves_surrounding_text(
            text in ".{0,60}",
            a in 0usize..70,
            b in 0usize..70,
        ) {
            let applied = apply_to(&text, TextOp::Uppercase, Some((a, b)));
            let lo = floor_boundary(&text, a.min(b));
            let hi = floor_boundary(&text, a.max(b));
            // With a real selection the prefix/suffix are verbatim; an empty one
            // means the whole document, so only assert the invariant when lo<hi.
            if lo < hi {
                prop_assert!(applied.content.starts_with(&text[..lo]));
                prop_assert!(applied.content.ends_with(&text[hi..]));
            }
        }
    }
}
