//! Bracket matching (#41): given the caret, find the bracket adjacent to it and
//! its partner.
//!
//! Like the status-bar readout (#37), this is a **pure function** of the buffer
//! text plus the caret byte offset the render shell reads from its editor
//! widget — no window, no I/O — so the whole thing is asserted headlessly (the
//! epic #25 testing standard). The shell calls [`match_at`] with the caret it
//! already computes for [`crate::status`] and highlights the returned offsets.
//!
//! Only the three ASCII pairs `()`, `[]`, `{}` are matched. Matching is by
//! nesting depth alone; it is **not** string/comment-aware, because the native
//! build has no syntax model yet — that arrives with syntax highlighting (#32),
//! at which point matching can learn to skip brackets inside strings/comments.

/// One bracket adjacent to the caret and its partner, as byte offsets into the
/// document. `here` is always a bracket character; `partner` is `Some` only when
/// the bracket is balanced by the *correct* opposite kind, and `None` when it is
/// unbalanced or closed by the wrong kind (e.g. the `]` in `([)]`). The shell
/// highlights `here` either way, in a "matched" or "unmatched" style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BracketMatch {
    /// Byte offset of the bracket character adjacent to the caret.
    pub here: usize,
    /// Byte offset of the matching bracket, or `None` when unbalanced.
    pub partner: Option<usize>,
}

/// Find the bracket adjacent to `caret` and its partner, or `None` when the
/// caret touches no bracket.
///
/// The caret sits *between* characters, so two positions are "adjacent": the
/// character immediately before it and the one at/after it. Following the common
/// editor convention (CodeMirror, Vim's `%`), the character **before** the caret
/// wins when both are brackets — that highlights the bracket you just typed.
///
/// `caret` is clamped into range and onto a char boundary, so any value the
/// widget hands us (stale, mid-glyph, past the end) is safe: this never panics.
pub fn match_at(text: &str, caret: usize) -> Option<BracketMatch> {
    let caret = crate::find::floor_boundary(text, caret);

    // The character ending at the caret (its byte offset is caret - its length).
    if let Some(ch) = text[..caret].chars().next_back()
        && is_bracket(ch)
    {
        return Some(resolve(text, caret - ch.len_utf8(), ch));
    }
    // Otherwise the character starting at the caret.
    if let Some(ch) = text[caret..].chars().next()
        && is_bracket(ch)
    {
        return Some(resolve(text, caret, ch));
    }
    None
}

/// Match the bracket `ch` at byte offset `off`, scanning the text by nesting
/// depth. An opener scans forward, a closer scans backward; both start at their
/// own bracket as depth 1 and stop at the character that returns depth to 0. That
/// partner is reported only if it is the correct opposite kind, else `None`.
///
/// The scan is **iterative** (no recursion, so arbitrarily deep nesting can't
/// overflow the stack) and walks raw bytes: every bracket is single-byte ASCII,
/// and no UTF-8 lead/continuation byte can equal one, so a multi-byte glyph is
/// simply skipped over. It is O(distance to the partner) — linear in the worst
/// case, with no backtracking.
fn resolve(text: &str, off: usize, ch: char) -> BracketMatch {
    let bytes = text.as_bytes();
    let unmatched = BracketMatch {
        here: off,
        partner: None,
    };

    if let Some(want) = closing_of(ch) {
        // Opener: walk forward to the closer that balances it.
        let mut depth = 1i32;
        for (i, &b) in bytes.iter().enumerate().skip(off + 1) {
            if is_open(b) {
                depth += 1;
            } else if is_close(b) {
                depth -= 1;
                if depth == 0 {
                    return BracketMatch {
                        here: off,
                        partner: (b == want).then_some(i),
                    };
                }
            }
        }
        unmatched
    } else if let Some(want) = opening_of(ch) {
        // Closer: walk backward to the opener that balances it.
        let mut depth = 1i32;
        for i in (0..off).rev() {
            let b = bytes[i];
            if is_close(b) {
                depth += 1;
            } else if is_open(b) {
                depth -= 1;
                if depth == 0 {
                    return BracketMatch {
                        here: off,
                        partner: (b == want).then_some(i),
                    };
                }
            }
        }
        unmatched
    } else {
        unmatched
    }
}

/// Whether `ch` is one of the six brackets we match.
fn is_bracket(ch: char) -> bool {
    closing_of(ch).is_some() || opening_of(ch).is_some()
}

/// The closing byte that pairs with opener `ch`, or `None` if `ch` isn't an
/// opener.
fn closing_of(ch: char) -> Option<u8> {
    match ch {
        '(' => Some(b')'),
        '[' => Some(b']'),
        '{' => Some(b'}'),
        _ => None,
    }
}

/// The opening byte that pairs with closer `ch`, or `None` if `ch` isn't a
/// closer.
fn opening_of(ch: char) -> Option<u8> {
    match ch {
        ')' => Some(b'('),
        ']' => Some(b'['),
        '}' => Some(b'{'),
        _ => None,
    }
}

fn is_open(b: u8) -> bool {
    matches!(b, b'(' | b'[' | b'{')
}

fn is_close(b: u8) -> bool {
    matches!(b, b')' | b']' | b'}')
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Byte offsets of the matched pair, or `None`, for a caret at `caret`.
    fn pair(text: &str, caret: usize) -> Option<(usize, Option<usize>)> {
        match_at(text, caret).map(|m| (m.here, m.partner))
    }

    #[test]
    fn caret_before_an_opener_matches_its_closer() {
        // "|()" — caret at 0 sits before '(', which matches ')' at 1.
        assert_eq!(pair("()", 0), Some((0, Some(1))));
        assert_eq!(pair("[]", 0), Some((0, Some(1))));
        assert_eq!(pair("{}", 0), Some((0, Some(1))));
    }

    #[test]
    fn caret_after_a_closer_matches_its_opener() {
        // "()|" — the char before the caret is ')' at 1, matching '(' at 0.
        assert_eq!(pair("()", 2), Some((1, Some(0))));
        // The backward scan resolves ']' and '}' closers too, not just ')'.
        assert_eq!(pair("[]", 2), Some((1, Some(0))));
        assert_eq!(pair("{}", 2), Some((1, Some(0))));
    }

    #[test]
    fn caret_between_two_brackets_prefers_the_one_before_it() {
        // "(|)" — before the caret is '(' at 0 (wins over ')' at 1).
        assert_eq!(pair("()", 1), Some((0, Some(1))));
    }

    #[test]
    fn nested_brackets_match_the_right_partner() {
        // "((()))": the outer '(' at 0 pairs with ')' at 5.
        assert_eq!(pair("((()))", 0), Some((0, Some(5))));
        // The inner '(' at 2 pairs with ')' at 3.
        assert_eq!(pair("((()))", 3), Some((2, Some(3))));
        // Mixed nesting: '{' at 0 pairs with '}' at 5.
        assert_eq!(pair("{[()]}", 0), Some((0, Some(5))));
    }

    #[test]
    fn nested_brackets_match_the_right_partner_walking_backward() {
        // Every case above starts on an *opener*, so they only exercise the forward
        // scan; the backward scan's nesting counter is only reached from a closer
        // with other pairs in between. Without these, inverting that counter still
        // passes the whole suite (the shallow "()|" cases never nest, and "())|"
        // reports unmatched either way).
        //
        // "((()))|" — the outer ')' at 5 skips two nested pairs back to '(' at 0.
        assert_eq!(pair("((()))", 6), Some((5, Some(0))));
        // "((()|))" — the inner ')' at 3 pairs with the '(' immediately before it.
        assert_eq!(pair("((()))", 4), Some((3, Some(2))));
        // Mixed nesting, backward: '}' at 5 skips "[()]" back to '{' at 0.
        assert_eq!(pair("{[()]}", 6), Some((5, Some(0))));
    }

    #[test]
    fn unbalanced_openers_and_closers_report_no_partner() {
        assert_eq!(pair("(", 0), Some((0, None))); // lone opener
        assert_eq!(pair(")", 1), Some((0, None))); // lone closer, caret after it
        // "())": the extra ')' at 2 has no opener to its left.
        assert_eq!(pair("())", 3), Some((2, None)));
    }

    #[test]
    fn wrong_kind_closer_is_treated_as_unmatched() {
        // "(]" — the '(' is balanced (depth-wise) by ']', the wrong kind.
        assert_eq!(pair("(]", 0), Some((0, None)));
        // "([)]" — classic interleave: the outer '(' closes on ']', wrong kind.
        assert_eq!(pair("([)]", 0), Some((0, None)));
    }

    #[test]
    fn caret_touching_no_bracket_is_none() {
        assert_eq!(pair("abc", 1), None);
        assert_eq!(pair("", 0), None);
        // A bracket exists but the caret is nowhere near it.
        assert_eq!(pair("a(b)c", 0), None);
    }

    #[test]
    fn multibyte_neighbours_do_not_break_offsets_or_matching() {
        // "é(x)" — 'é' is two bytes (0..2); '(' is at 2, ')' at 4.
        let text = "é(x)";
        // Caret just after '(' (byte 3): the '(' before it matches ')' at 4.
        assert_eq!(pair(text, 3), Some((2, Some(4))));
        // A multi-byte glyph inside the span must not confuse depth counting.
        let spanned = "(é)";
        assert_eq!(pair(spanned, 0), Some((0, Some("(é".len()))));
    }

    #[test]
    fn out_of_range_or_mid_glyph_caret_is_clamped_never_panics() {
        let text = "(é)"; // 'é' occupies bytes 1..3
        // Caret past the end clamps to end-of-text: char before is ')'.
        assert_eq!(match_at(text, 999).map(|m| m.here), Some("(é".len()));
        // A mid-'é' caret floors onto the 'é' boundary (byte 1); the char before
        // is '(' at 0, so it still matches — and it does not panic.
        assert_eq!(pair(text, 2), Some((0, Some("(é".len()))));
    }

    #[test]
    fn resolve_reports_no_match_for_a_non_bracket() {
        // `match_at` only ever hands `resolve` a bracket (it guards with
        // `is_bracket`), so its final `else` is defensive: a stray non-bracket
        // must report no partner rather than misbehave. Exercised directly since
        // the public entry point can't reach it — this keeps the guard covered
        // deterministically instead of leaving it to a lucky proptest draw.
        let m = resolve("x", 0, 'x');
        assert_eq!((m.here, m.partner), (0, None));
    }

    proptest! {
        /// `match_at` never panics on any (text, caret) and its offsets always
        /// land on bracket characters that form a valid pair.
        #[test]
        fn never_panics_and_offsets_are_valid_brackets(
            text in ".{0,300}",
            caret in 0usize..400,
        ) {
            if let Some(m) = match_at(&text, caret) {
                let here = text[m.here..].chars().next().unwrap();
                prop_assert!(is_bracket(here), "`here` must be a bracket, got {here:?}");
                if let Some(p) = m.partner {
                    let partner = text[p..].chars().next().unwrap();
                    // One is an opener and the other its exact closer.
                    let ok = closing_of(here) == Some(partner as u8)
                        || closing_of(partner) == Some(here as u8);
                    prop_assert!(ok, "{here:?} and {partner:?} are not a pair");
                }
            }
        }

        /// Matching is symmetric: if the bracket at `here` pairs with `partner`,
        /// then a caret just past `partner` matches back to `here`.
        #[test]
        fn matching_is_involutive(text in ".{0,300}", caret in 0usize..400) {
            if let Some(m) = match_at(&text, caret)
                && let Some(p) = m.partner
            {
                let back = match_at(&text, p + 1);
                prop_assert_eq!(
                    back,
                    Some(BracketMatch { here: p, partner: Some(m.here) })
                );
            }
        }
    }
}
