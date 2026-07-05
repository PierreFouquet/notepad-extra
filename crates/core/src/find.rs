//! Pure Find / Replace / Go-to-line over the text buffer (#33), replacing
//! CodeMirror's `getSearchCursor`.
//!
//! Everything here is a deterministic function of `(text, pattern, options)` —
//! no cursor state, no widget, no I/O — so the whole feature is exercised as
//! plain assertions with no window or GPU (epic #25's testing standard). The
//! [`crate::app`] layer holds the live search cursor and drives these
//! primitives in response to `Message`s.
//!
//! ## Why the `regex` crate (and not `fancy-regex`)
//!
//! Search runs on whatever the user types, including hostile input. The `regex`
//! crate is a finite-automaton engine with **linear-time** matching: it never
//! backtracks, so no pattern — however pathological — can hang the UI. That
//! *is* the epic's "catastrophic-regex guard", achieved by construction rather
//! than with a timeout. The trade-off is no backreferences / lookaround; those
//! are a later, opt-in concern (`fancy-regex`) and are intentionally
//! unsupported this round.

use regex::{Regex, RegexBuilder};

/// A half-open byte range `[start, end)` into the searched text. Byte offsets
/// (not char indices) because that is what slicing and the buffer speak; the
/// engine only ever yields offsets on UTF-8 char boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
}

impl Match {
    /// A zero-width match (e.g. from `a*` or `^`), which navigation must step
    /// over by a char rather than by its length to make progress.
    pub fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// The match's length in bytes.
    pub fn len(self) -> usize {
        self.end - self.start
    }
}

/// How a pattern is interpreted — mirrors the find bar's checkboxes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SearchOptions {
    /// Case-sensitive when `true`; case-insensitive (the editor default) when
    /// `false`.
    pub case_sensitive: bool,
    /// Match only whole words, by wrapping the pattern in `\b…\b`.
    pub whole_word: bool,
    /// Treat the pattern as a regular expression; otherwise it is a literal
    /// string (every regex metacharacter is escaped and matched as typed).
    pub regex: bool,
}

/// Why a pattern could not be turned into a [`Matcher`]. Surfaced to the user
/// (e.g. in the find bar) instead of panicking — invalid input is expected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchError {
    /// An empty pattern matches nothing on purpose (the user has just started
    /// typing); the find bar shows this as "no matches", not an error.
    EmptyPattern,
    /// The regex failed to compile: bad syntax, or it blew the size limit.
    InvalidRegex(String),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchError::EmptyPattern => write!(f, "empty search"),
            SearchError::InvalidRegex(msg) => write!(f, "invalid regex: {msg}"),
        }
    }
}

/// The rewritten text plus where the replacement landed, returned by
/// [`Matcher::replace_next`] so the shell can select the new text and continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    /// The full buffer after the single replacement.
    pub text: String,
    /// The byte range the inserted replacement now occupies.
    pub range: Match,
}

/// Cap the compiled program so a monster pattern (`a{999999}{999999}`) fails to
/// build instead of eating memory — the compile-time half of the regex guard.
/// Comfortably fits any realistic search.
const COMPILE_SIZE_LIMIT: usize = 1 << 20; // 1 MiB

/// A compiled search: the user's pattern as a finite-automaton matcher, plus
/// whether replacements expand `$1` group references (regex mode) or are
/// inserted verbatim (literal mode).
#[derive(Debug, Clone)]
pub struct Matcher {
    re: Regex,
    expand: bool,
}

impl Matcher {
    /// Compile `pattern` under `options`. Returns a [`SearchError`] for an empty
    /// pattern or a regex that will not build; never panics.
    pub fn new(pattern: &str, options: SearchOptions) -> Result<Matcher, SearchError> {
        if pattern.is_empty() {
            return Err(SearchError::EmptyPattern);
        }
        // Literal mode: escape everything so metacharacters match as typed.
        // Regex mode: wrap in a *non-capturing* group so `whole_word` boundaries
        // and top-level alternation (`a|b`) compose without renumbering the
        // user's own capture groups (`$1` keeps meaning what they wrote).
        let core = if options.regex {
            format!("(?:{pattern})")
        } else {
            regex::escape(pattern)
        };
        let full = if options.whole_word {
            format!(r"\b{core}\b")
        } else {
            core
        };
        let re = RegexBuilder::new(&full)
            .case_insensitive(!options.case_sensitive)
            // `^`/`$` anchor to line boundaries, which is what an editor's find
            // is expected to do (and what CodeMirror did).
            .multi_line(true)
            .size_limit(COMPILE_SIZE_LIMIT)
            .build()
            .map_err(|e| SearchError::InvalidRegex(e.to_string()))?;
        Ok(Matcher {
            re,
            expand: options.regex,
        })
    }

    /// The leftmost match whose start is at or after byte `from`. `from` is
    /// clamped into range and onto a char boundary, so any offset is safe.
    pub fn find_from(&self, text: &str, from: usize) -> Option<Match> {
        let from = floor_boundary(text, from);
        self.re.find_at(text, from).map(to_match)
    }

    /// The rightmost match whose start is strictly before byte `before` — the
    /// primitive behind "find previous". Scans forward (the automaton only goes
    /// one way) keeping the last qualifying match; linear, and it never
    /// materialises the whole match list.
    pub fn find_last_before(&self, text: &str, before: usize) -> Option<Match> {
        let before = before.min(text.len());
        let mut last = None;
        for m in self.re.find_iter(text) {
            if m.start() < before {
                last = Some(to_match(m));
            } else {
                break;
            }
        }
        last
    }

    /// The last match in `text`, for wrap-around on "find previous".
    pub fn find_last(&self, text: &str) -> Option<Match> {
        self.re.find_iter(text).last().map(to_match)
    }

    /// Total number of non-overlapping matches, for the "N matches" readout.
    /// Streams the automaton, so it allocates nothing even on huge inputs.
    pub fn count(&self, text: &str) -> usize {
        self.re.find_iter(text).count()
    }

    /// 1-based index of the match that starts at byte `start`, or `0` if no
    /// match starts there — for the "3 of 10" readout.
    pub fn ordinal_of(&self, text: &str, start: usize) -> usize {
        let mut n = 0;
        for m in self.re.find_iter(text) {
            n += 1;
            if m.start() == start {
                return n;
            }
            if m.start() > start {
                break;
            }
        }
        0
    }

    /// Every non-overlapping match, left to right. Materialises the list, so
    /// callers on possibly-huge inputs prefer [`Matcher::count`] +
    /// [`Matcher::find_from`]; kept for tests and bounded uses.
    pub fn find_all(&self, text: &str) -> Vec<Match> {
        self.re.find_iter(text).map(to_match).collect()
    }

    /// Replace every match, returning the new text and the replacement count in
    /// a single pass. In regex mode `replacement` may reference capture groups
    /// (`$1`, `${name}`); in literal mode it is inserted verbatim.
    pub fn replace_all(&self, text: &str, replacement: &str) -> (String, usize) {
        let mut count = 0usize;
        let out = self
            .re
            .replace_all(text, |caps: &regex::Captures| {
                count += 1;
                let mut dst = String::new();
                self.expand_into(caps, replacement, &mut dst);
                dst
            })
            .into_owned();
        (out, count)
    }

    /// Replace the next match at or after byte `from`, wrapping to the top if
    /// there is none after `from`. Returns the rewritten text and the byte range
    /// the replacement occupies (so the shell can select it); `None` if the
    /// pattern matches nowhere.
    pub fn replace_next(&self, text: &str, from: usize, replacement: &str) -> Option<Replacement> {
        let from = floor_boundary(text, from);
        let caps = self
            .re
            .captures_at(text, from)
            .or_else(|| self.re.captures(text))?;
        let m = caps.get(0).expect("group 0 always present on a match");
        let (start, end) = (m.start(), m.end());
        let mut rep = String::new();
        self.expand_into(&caps, replacement, &mut rep);
        let mut out = String::with_capacity(text.len() - (end - start) + rep.len());
        out.push_str(&text[..start]);
        out.push_str(&rep);
        out.push_str(&text[end..]);
        let new_end = start + rep.len();
        Some(Replacement {
            text: out,
            range: Match {
                start,
                end: new_end,
            },
        })
    }

    /// Write the replacement for one match into `dst`: expanded (`$1`) in regex
    /// mode, verbatim in literal mode so a `$` in a literal find/replace is not
    /// treated as a group reference.
    fn expand_into(&self, caps: &regex::Captures, replacement: &str, dst: &mut String) {
        if self.expand {
            caps.expand(replacement, dst);
        } else {
            dst.push_str(replacement);
        }
    }
}

/// The byte offset a *forward* search should resume from after visiting `m`, so
/// non-empty matches stay non-overlapping and a zero-width match still advances
/// by one char instead of matching itself forever.
pub fn resume_after(text: &str, m: Match) -> usize {
    if m.is_empty() {
        boundary_after(text, m.start)
    } else {
        m.end
    }
}

/// Number of lines in `text`: one more than the count of `\n`. An empty string
/// is one (empty) line; a trailing `\n` yields a final empty line — the cursor
/// model an editor uses (and what [`goto_line_offset`] clamps against).
pub fn line_count(text: &str) -> usize {
    text.bytes().filter(|&b| b == b'\n').count() + 1
}

/// Byte offset of the start of 1-based `line`, clamped to `[1, line_count]`:
/// a go-to of 0-or-below lands on the first line, past-the-end on the last.
/// Never panics, never returns an offset off a char boundary.
pub fn goto_line_offset(text: &str, line: usize) -> usize {
    let line = line.max(1);
    if line == 1 {
        return 0;
    }
    let mut newlines = 0usize;
    let mut last_line_start = 0usize;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            newlines += 1;
            last_line_start = i + 1;
            if newlines == line - 1 {
                return i + 1;
            }
        }
    }
    // Fewer lines than requested → the start of the final line.
    last_line_start
}

/// The 0-based line and byte-column of `offset` within `text` (clamped into
/// range and onto a char boundary). Column is a **byte** index within its line,
/// matching what iced's `text_editor` cursor uses — the shell turns a [`Match`]
/// range into an editor selection with this. Pure, so it is tested here rather
/// than in the window-bound shell.
pub fn line_col_of(text: &str, offset: usize) -> (usize, usize) {
    let offset = floor_boundary(text, offset);
    let prefix = &text[..offset];
    let line = prefix.bytes().filter(|&b| b == b'\n').count();
    let line_start = prefix.rfind('\n').map_or(0, |i| i + 1);
    (line, offset - line_start)
}

/// The byte offset of the 0-based `line` and byte-`column` iced's cursor
/// reports — the inverse of [`line_col_of`]. The column is clamped to the
/// line's own length (so it can't spill into the next line) and floored onto a
/// char boundary; `line` / `column` may be anything (they saturate), so any
/// cursor the widget hands us maps to a safe, in-bounds offset. Pure, so the
/// shell's cursor→offset conversion is proven here, not in the window-bound
/// shell.
pub fn offset_at(text: &str, line: usize, column: usize) -> usize {
    let line_start = goto_line_offset(text, line.saturating_add(1));
    let line_end = text[line_start..]
        .find('\n')
        .map_or(text.len(), |i| line_start + i);
    floor_boundary(text, line_start.saturating_add(column).min(line_end))
}

fn to_match(m: regex::Match) -> Match {
    Match {
        start: m.start(),
        end: m.end(),
    }
}

/// Round `i` down to the nearest char boundary (clamped to `text.len()`), so an
/// arbitrary offset is always safe to hand to the regex engine or to slice.
/// Shared with [`crate::status`], which floors widget-supplied offsets too.
pub(crate) fn floor_boundary(text: &str, i: usize) -> usize {
    if i >= text.len() {
        return text.len();
    }
    let mut i = i;
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The first char boundary strictly greater than `i` (clamped to `text.len()`).
fn boundary_after(text: &str, i: usize) -> usize {
    if i >= text.len() {
        return text.len();
    }
    let mut j = i + 1;
    while j < text.len() && !text.is_char_boundary(j) {
        j += 1;
    }
    j
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn opts(case_sensitive: bool, whole_word: bool, regex: bool) -> SearchOptions {
        SearchOptions {
            case_sensitive,
            whole_word,
            regex,
        }
    }

    fn literal(pattern: &str) -> Matcher {
        Matcher::new(pattern, SearchOptions::default()).expect("compiles")
    }

    // ---- compilation & errors ------------------------------------------------

    #[test]
    fn empty_pattern_is_a_named_error() {
        let err = Matcher::new("", SearchOptions::default()).unwrap_err();
        assert_eq!(err, SearchError::EmptyPattern);
        assert_eq!(err.to_string(), "empty search");
    }

    #[test]
    fn invalid_regex_surfaces_not_panics() {
        let err = Matcher::new("(unclosed", opts(false, false, true)).unwrap_err();
        match &err {
            SearchError::InvalidRegex(msg) => assert!(!msg.is_empty()),
            other => panic!("expected InvalidRegex, got {other:?}"),
        }
        assert!(err.to_string().starts_with("invalid regex:"));
    }

    #[test]
    fn literal_mode_escapes_metacharacters() {
        // `a.c` as a literal must not match `abc` — only the exact "a.c".
        let m = literal("a.c");
        assert_eq!(m.find_all("abc a.c"), vec![Match { start: 4, end: 7 }]);
    }

    #[test]
    fn huge_pattern_hits_the_size_limit_gracefully() {
        // A program that would blow past COMPILE_SIZE_LIMIT must fail to build,
        // not allocate wildly or panic.
        let err = Matcher::new(r"a{100000}{100000}", opts(true, false, true)).unwrap_err();
        assert!(matches!(err, SearchError::InvalidRegex(_)));
    }

    // ---- finding -------------------------------------------------------------

    #[test]
    fn case_insensitive_is_the_default() {
        let m = literal("foo");
        assert_eq!(m.count("Foo FOO foo"), 3);
        let sensitive = Matcher::new("foo", opts(true, false, false)).unwrap();
        assert_eq!(sensitive.count("Foo FOO foo"), 1);
    }

    #[test]
    fn whole_word_only_matches_bounded_words() {
        let m = Matcher::new("cat", opts(false, true, false)).unwrap();
        assert_eq!(m.count("cat category concat cat!"), 2); // "cat" and "cat!"
    }

    #[test]
    fn find_from_and_next_walk_forward() {
        let m = literal("ab");
        let text = "ab_ab_ab";
        let first = m.find_from(text, 0).unwrap();
        assert_eq!(first, Match { start: 0, end: 2 });
        let second = m.find_from(text, resume_after(text, first)).unwrap();
        assert_eq!(second, Match { start: 3, end: 5 });
    }

    #[test]
    fn find_last_before_and_last_walk_backward() {
        let m = literal("ab");
        let text = "ab_ab_ab";
        assert_eq!(m.find_last(text), Some(Match { start: 6, end: 8 }));
        assert_eq!(
            m.find_last_before(text, 6),
            Some(Match { start: 3, end: 5 })
        );
        assert_eq!(m.find_last_before(text, 0), None); // nothing before the top
    }

    #[test]
    fn no_match_returns_none_everywhere() {
        let m = literal("zzz");
        assert_eq!(m.find_from("abc", 0), None);
        assert_eq!(m.find_last("abc"), None);
        assert_eq!(m.find_last_before("abc", 3), None);
        assert_eq!(m.count("abc"), 0);
        assert_eq!(m.ordinal_of("abc", 0), 0);
    }

    #[test]
    fn ordinal_reports_position_within_matches() {
        let m = literal("x");
        let text = "x_x_x"; // matches at 0, 2, 4
        assert_eq!(m.ordinal_of(text, 0), 1);
        assert_eq!(m.ordinal_of(text, 2), 2);
        assert_eq!(m.ordinal_of(text, 4), 3);
        assert_eq!(m.ordinal_of(text, 1), 0); // not a match start
    }

    #[test]
    fn empty_match_navigation_makes_progress() {
        // `a*` matches empty everywhere plus "aa"; forward stepping must visit
        // distinct positions and terminate at the tail rather than stalling on a
        // zero-width match.
        let m = Matcher::new("a*", opts(false, false, true)).unwrap();
        let text = "xaax";
        let mut cursor = 0;
        let mut starts = Vec::new();
        for _ in 0..20 {
            let Some(hit) = m.find_from(text, cursor) else {
                break;
            };
            starts.push(hit.start);
            let next = resume_after(text, hit);
            if next == cursor {
                break; // fixed point: the zero-width match at end-of-text
            }
            assert!(next > cursor, "cursor must advance: {cursor} -> {next}");
            cursor = next;
        }
        assert_eq!(starts, vec![0, 1, 3, 4]);
    }

    #[test]
    fn regex_anchors_are_per_line() {
        // multi_line(true): `^` matches at each line start.
        let m = Matcher::new("^b", opts(false, false, true)).unwrap();
        assert_eq!(
            m.find_all("a\nb\nb"),
            vec![Match { start: 2, end: 3 }, Match { start: 4, end: 5 },]
        );
    }

    #[test]
    fn unicode_matches_on_char_boundaries() {
        let m = literal("é");
        let text = "café éclair"; // 'é' is two bytes
        let hits = m.find_all(text);
        assert_eq!(hits.len(), 2);
        for h in hits {
            assert!(text.is_char_boundary(h.start) && text.is_char_boundary(h.end));
        }
    }

    // ---- replacing -----------------------------------------------------------

    #[test]
    fn replace_all_counts_and_rewrites() {
        let m = literal("cat");
        let (out, n) = m.replace_all("cat cat dog", "dog");
        assert_eq!(out, "dog dog dog");
        assert_eq!(n, 2);
    }

    #[test]
    fn replace_all_none_is_identity() {
        let m = literal("zzz");
        let (out, n) = m.replace_all("abc", "x");
        assert_eq!(out, "abc");
        assert_eq!(n, 0);
    }

    #[test]
    fn regex_replace_expands_groups() {
        let m = Matcher::new(r"(\w+)@(\w+)", opts(false, false, true)).unwrap();
        let (out, n) = m.replace_all("a@b c@d", "$2.$1");
        assert_eq!(out, "b.a d.c");
        assert_eq!(n, 2);
    }

    #[test]
    fn literal_replace_does_not_expand_dollar() {
        // A `$1` in a *literal* replace must be inserted verbatim.
        let m = literal("x");
        let (out, _) = m.replace_all("x", "$1");
        assert_eq!(out, "$1");
    }

    #[test]
    fn replace_next_replaces_one_and_reports_range() {
        let m = literal("ab");
        let r = m.replace_next("ab_ab", 0, "Z").unwrap();
        assert_eq!(r.text, "Z_ab");
        assert_eq!(r.range, Match { start: 0, end: 1 });
        // Resuming from the reported range finds the second occurrence.
        let r2 = m.replace_next(&r.text, r.range.end, "Z").unwrap();
        assert_eq!(r2.text, "Z_Z");
    }

    #[test]
    fn replace_next_wraps_to_the_top() {
        let m = literal("ab");
        // Starting past the only match, we wrap around and still replace it.
        let r = m.replace_next("ab", 10, "Z").unwrap();
        assert_eq!(r.text, "Z");
    }

    #[test]
    fn replace_next_no_match_is_none() {
        let m = literal("zzz");
        assert!(m.replace_next("abc", 0, "x").is_none());
    }

    // ---- go-to-line & line counting -----------------------------------------

    #[test]
    fn goto_line_clamps_both_ends() {
        let text = "one\ntwo\nthree";
        assert_eq!(goto_line_offset(text, 1), 0);
        assert_eq!(goto_line_offset(text, 2), 4);
        assert_eq!(goto_line_offset(text, 3), 8);
        assert_eq!(goto_line_offset(text, 0), 0); // below range → first line
        assert_eq!(goto_line_offset(text, 999), 8); // past end → last line start
    }

    #[test]
    fn goto_and_line_count_treat_trailing_newline_as_a_line() {
        assert_eq!(line_count(""), 1);
        assert_eq!(line_count("a"), 1);
        assert_eq!(line_count("a\n"), 2);
        assert_eq!(line_count("a\nb\n"), 3);
        // The empty final line begins at end-of-text.
        assert_eq!(goto_line_offset("a\n", 2), 2);
    }

    #[test]
    fn line_col_maps_offsets_to_editor_positions() {
        let text = "ab\ncdé\nx"; // 'é' is two bytes at line 1
        assert_eq!(line_col_of(text, 0), (0, 0)); // start
        assert_eq!(line_col_of(text, 2), (0, 2)); // end of line 0
        assert_eq!(line_col_of(text, 3), (1, 0)); // start of line 1
        assert_eq!(line_col_of(text, 5), (1, 2)); // before 'é'
        assert_eq!(line_col_of(text, 7), (1, 4)); // after 'é' (2 bytes)
        // Out of range and mid-char offsets are clamped to a boundary, never panic.
        assert_eq!(line_col_of(text, 999), (2, 1));
        assert_eq!(line_col_of(text, 6), (1, 2)); // mid-'é' floors to its start
    }

    #[test]
    fn offset_at_inverts_line_col_of() {
        let text = "ab\ncdé\nx"; // 'é' is two bytes on line 1
        // Every boundary offset round-trips: offset -> (line, byte-col) -> offset.
        for off in [0usize, 1, 2, 3, 4, 5, 7, 8] {
            let (l, c) = line_col_of(text, off);
            assert_eq!(
                offset_at(text, l, c),
                off,
                "offset {off} did not round-trip"
            );
        }
        // A column past the line's end clamps to the newline, never into the next line.
        assert_eq!(offset_at(text, 0, 99), 2); // end of "ab"
        // A line past the end clamps to the last line's start.
        assert_eq!(offset_at(text, 999, 0), 8);
        // Saturating arithmetic: extreme inputs clamp to end-of-text, never overflow.
        assert_eq!(offset_at(text, usize::MAX, usize::MAX), text.len());
    }

    #[test]
    fn match_helpers_report_shape() {
        let m = Match { start: 3, end: 3 };
        assert!(m.is_empty());
        assert_eq!(m.len(), 0);
        let n = Match { start: 3, end: 6 };
        assert!(!n.is_empty());
        assert_eq!(n.len(), 3);
    }

    // ---- property-based invariants (epic #25 Definition of Done) ------------

    proptest! {
        /// No pattern/text pair may ever panic, and every reported match must be
        /// a valid, in-bounds, char-boundary byte range of the text.
        #[test]
        fn matches_are_always_valid_ranges(
            pattern in ".{0,12}",
            text in ".{0,200}",
            case_sensitive in any::<bool>(),
            whole_word in any::<bool>(),
            regex in any::<bool>(),
        ) {
            if let Ok(m) = Matcher::new(&pattern, opts(case_sensitive, whole_word, regex)) {
                for hit in m.find_all(&text) {
                    prop_assert!(hit.start <= hit.end);
                    prop_assert!(hit.end <= text.len());
                    prop_assert!(text.is_char_boundary(hit.start));
                    prop_assert!(text.is_char_boundary(hit.end));
                    // The reported slice is exactly what re-finding at that spot yields.
                    prop_assert_eq!(m.find_from(&text, hit.start), Some(hit));
                }
            }
        }

        /// Walking forward with `find_from` + `resume_after` visits matches in
        /// non-decreasing order, strictly advances the cursor until the
        /// end-of-text fixed point, and always terminates — never loops.
        #[test]
        fn forward_walk_strictly_progresses(
            pattern in ".{1,6}",
            text in ".{0,150}",
            regex in any::<bool>(),
        ) {
            if let Ok(m) = Matcher::new(&pattern, opts(false, false, regex)) {
                let mut cursor = 0usize;
                let mut prev_start: Option<usize> = None;
                let mut steps = 0usize;
                while let Some(hit) = m.find_from(&text, cursor) {
                    if let Some(p) = prev_start {
                        prop_assert!(hit.start >= p, "start went backwards: {} < {p}", hit.start);
                    }
                    prev_start = Some(hit.start);
                    let next = resume_after(&text, hit);
                    prop_assert!(next <= text.len());
                    if next == cursor {
                        break; // end-of-text fixed point (zero-width tail match)
                    }
                    prop_assert!(next > cursor);
                    cursor = next;
                    steps += 1;
                    prop_assert!(steps <= text.len() + 2, "walk did not terminate");
                }
            }
        }

        /// `replace_all` never changes the text when the pattern is absent, and
        /// the reported count equals the number of matches found.
        #[test]
        fn replace_all_count_matches_find_count(
            needle in "[a-c]{1,3}",
            text in "[a-c ]{0,120}",
        ) {
            let m = literal(&needle);
            let expected = m.count(&text);
            let (out, n) = m.replace_all(&text, "X");
            prop_assert_eq!(n, expected);
            if expected == 0 {
                prop_assert_eq!(out, text);
            }
        }

        /// Go-to-line is always clamped to a real, in-bounds char boundary.
        #[test]
        fn goto_line_is_always_in_bounds(text in ".{0,200}", line in 0usize..500) {
            let off = goto_line_offset(&text, line);
            prop_assert!(off <= text.len());
            prop_assert!(text.is_char_boundary(off));
        }

        /// `offset_at` maps any (line, column) — however far out of range — to an
        /// in-bounds char boundary, and never panics.
        #[test]
        fn offset_at_is_always_a_valid_boundary(
            text in ".{0,200}",
            line in 0usize..300,
            column in 0usize..300,
        ) {
            let off = offset_at(&text, line, column);
            prop_assert!(off <= text.len());
            prop_assert!(text.is_char_boundary(off));
        }
    }
}
