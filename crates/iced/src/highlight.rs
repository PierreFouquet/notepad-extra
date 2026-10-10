//! Syntax highlighting for the editor widget (#32).
//!
//! iced ships `iced_highlighter`, but it pulls syntect built with **oniguruma**
//! (a C dependency the epic forbids), so we implement the
//! [`iced_core::text::Highlighter`] trait ourselves on top of
//! [`notepad_syntax`]'s **fancy-regex** `SyntaxSet` / `ThemeSet`. The vendored
//! `text_editor` plugs this in through `highlight_with::<_>()` — never iced's
//! gated `highlighter` feature — so oniguruma never enters the crate tree.
//!
//! The highlighter is fed **one line at a time** and keeps *sparse checkpoints*
//! of the parser + highlighter state: a snapshot of the state *entering* every
//! `CHECKPOINT_INTERVAL`th line (#126). Re-highlighting after an edit resumes
//! from the nearest checkpoint at or before the changed line and re-parses from
//! there — the incremental model iced's editor expects (see the trait's
//! `change_line` / `current_line`, and the protocol note on
//! [`SyntectHighlighter`]'s `change_line`).
//!
//! Only colour is applied for #32 (bold/italic token styling is a later nicety).
//! The theme follows the app's light/dark toggle (#36): the syntect theme is
//! chosen by [`notepad_syntax::highlight_theme`] from the active [`ThemeMode`].

use std::ops::Range;

use iced::{Color, Font};
use iced_core::text::highlighter::{self, Format};
use notepad_syntax::ThemeMode;

use syntect::highlighting::{
    HighlightState, Highlighter as SyntectInner, RangedHighlightIterator, Style,
};
use syntect::parsing::{ParseState, ScopeStack, SyntaxSet};

/// Identifies *what* to highlight: the effective syntax name (from the core's
/// [`notepad_core::Document::language`]) and the light/dark [`ThemeMode`]
/// (#36). `PartialEq` lets the widget rebuild the highlighter only when one of
/// these actually changes — so toggling the theme re-highlights the buffer.
///
/// The syntax is a `&'static str`, not a `String` (#126): every name the core
/// can hand us borrows the process-wide syntax set (`Document::language` is
/// `&'static str`), and the shell builds one `Settings` per frame for the editor
/// widget, which then compares and clones it on every layout. Borrowing makes
/// that a pointer copy instead of a heap allocation per frame, and lets the
/// type be `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    pub syntax: &'static str,
    pub theme: ThemeMode,
}

/// One highlighted span's style, produced by [`SyntectHighlighter`] and turned
/// into an iced [`Format`] by [`to_format`].
#[derive(Debug, Clone, Copy)]
pub struct Highlight(Style);

/// The `to_format` strategy handed to `highlight_with`: map a syntect style to an
/// iced text [`Format`]. Colour only for #32 — the font is left as the editor's
/// own (so the user's font choice and weight are preserved). A plain `fn` (no
/// captures) as the widget requires.
pub fn to_format(highlight: &Highlight, _theme: &iced::Theme) -> Format<Font> {
    Format {
        color: Some(convert_color(highlight.0.foreground)),
        font: None,
    }
}

fn convert_color(c: syntect::highlighting::Color) -> Color {
    Color::from_rgba8(c.r, c.g, c.b, f32::from(c.a) / 255.0)
}

/// Lines between two state checkpoints (#126).
///
/// A snapshot is a deep clone of syntect's `ParseState` + `HighlightState` — a
/// context stack, a scope stack and per-scope style caches, each a heap `Vec`
/// — measured at ~1.5 KB retained per line on a Rust buffer (152 bytes inline
/// plus the vectors). One snapshot *per line*, as before #126, put a 100k-line
/// file at ~146 MB, made every `change_line` drop tens of thousands of them
/// (~90 ms for an edit near the top) and every `highlight_line` clone one.
/// Checkpointing every 32nd line divides the memory by 32 (a 200k-line file
/// keeps ~6k snapshots — a few MB) while bounding the price of an edit at
/// re-highlighting at most 31 extra lines: well under one viewport (≈50–60 rows
/// at 1080p), so an edit never more than roughly doubles the per-frame
/// highlight work iced already does. 64 would halve the memory again for twice
/// that worst case; the memory is already small at 32, so the cheaper edit wins.
const CHECKPOINT_INTERVAL: usize = 32;

/// A snapshot of the parser and highlighter state *at the start* of a line.
/// Both syntect types are `Clone`, which is what makes a checkpoint possible.
#[derive(Debug, Clone)]
struct LineState {
    parser: ParseState,
    highlight: HighlightState,
}

/// A syntect-backed [`Highlighter`](highlighter::Highlighter) over the shared,
/// process-wide fancy-regex sets (so constructing one loads nothing).
pub struct SyntectHighlighter {
    syntaxes: &'static SyntaxSet,
    inner: SyntectInner<'static>,
    /// `checkpoints[k]` is the state entering line `k * CHECKPOINT_INTERVAL`
    /// (#126). `checkpoints[0]` is the pristine start-of-document state, so the
    /// vector is never empty, and a checkpoint is only ever taken on the way
    /// *past* its line, so every one sits at or before `current_line`:
    /// `checkpoints.len() == current_line / CHECKPOINT_INTERVAL + 1` always.
    checkpoints: Vec<LineState>,
    /// The live state entering `current_line`, advanced *in place* by each
    /// `highlight_line` — no per-line clone; one is taken only when a checkpoint
    /// line is reached.
    state: LineState,
    current_line: usize,
    /// The spans of the most recently highlighted line, handed out by reference
    /// (#126). Reusing one buffer across lines means a warm highlighter
    /// allocates nothing for its own output per line (syntect's `parse_line`
    /// still returns its own `Vec` of scope ops — that one is syntect's API).
    spans: Vec<(Range<usize>, Highlight)>,
}

impl SyntectHighlighter {
    fn build(settings: &Settings) -> Self {
        let syntaxes = notepad_syntax::syntax_set();
        // An unknown / "Plain Text" name resolves to syntect's plain-text syntax,
        // which emits only default-styled spans — i.e. visually unhighlighted —
        // so the view needs no separate "no highlighting" code path.
        let syntax = syntaxes
            .find_syntax_by_name(settings.syntax)
            .unwrap_or_else(|| syntaxes.find_syntax_plain_text());

        // The paired theme is borrowed from the process-wide embedded set, so the
        // reference is `'static` too — no `Box::leak` (which iced_highlighter
        // needs) and no self-borrow (#36).
        let theme = notepad_syntax::highlight_theme(settings.theme);

        let inner = SyntectInner::new(theme);
        let initial = LineState {
            parser: ParseState::new(syntax),
            highlight: HighlightState::new(&inner, ScopeStack::new()),
        };

        Self {
            syntaxes,
            inner,
            checkpoints: vec![initial.clone()],
            state: initial,
            current_line: 0,
            spans: Vec::new(),
        }
    }
}

impl highlighter::Highlighter for SyntectHighlighter {
    type Settings = Settings;
    type Highlight = Highlight;
    type Iterator<'a> = std::iter::Cloned<std::slice::Iter<'a, (Range<usize>, Highlight)>>;

    fn new(settings: &Self::Settings) -> Self {
        Self::build(settings)
    }

    fn update(&mut self, new_settings: &Self::Settings) {
        *self = Self::build(new_settings);
    }

    fn change_line(&mut self, line: usize) {
        // Protocol (checked against `iced_graphics::text::Editor::highlight`):
        // after this call iced re-highlights every line from `current_line()`
        // up to the last *visible* line, so resuming a few lines *before* the
        // changed one is sound — those lines merely get their identical spans
        // re-applied (`BufferLine::set_attrs_list` is a no-op on equal
        // attributes). That is what makes sparse checkpoints possible (#126).
        //
        // Nothing we hold describes `line` or anything after it once
        // `line >= current_line`: the live state enters `current_line`, which
        // depends only on the lines before it, and every checkpoint sits at or
        // before `current_line`. So keep resuming where we are — which also
        // makes an out-of-range `line` a no-op rather than a reset.
        if line >= self.current_line {
            return;
        }
        // Rewind to the last checkpoint at or before `line` and drop the stale
        // ones after it. The `.min` is a belt-and-braces clamp: the invariant on
        // `checkpoints` gives `len - 1 == current_line / INTERVAL >= line /
        // INTERVAL`, so the index is in range and `checkpoints[0]` survives.
        let k = (line / CHECKPOINT_INTERVAL).min(self.checkpoints.len() - 1);
        self.checkpoints.truncate(k + 1);
        self.state = self.checkpoints[k].clone();
        self.current_line = k * CHECKPOINT_INTERVAL;
    }

    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        // Advance the live state in place: `parse_line` moves the parser past
        // this line, and the ranged iterator moves the highlighter's scope and
        // style stacks as it walks the ops — once drained, `state` is the state
        // *entering the next line*. Draining eagerly into the reusable `spans`
        // buffer also keeps the borrows of `line` and `ops` out of the returned
        // iterator (its lifetime is tied to `self` alone).
        let ops = self
            .state
            .parser
            .parse_line(line, self.syntaxes)
            .unwrap_or_default();

        self.spans.clear();
        self.spans.extend(
            RangedHighlightIterator::new(&mut self.state.highlight, &ops, line, &self.inner)
                .map(|(style, _text, range)| (range, Highlight(style))),
        );

        self.current_line += 1;
        if self.current_line.is_multiple_of(CHECKPOINT_INTERVAL) {
            // Snapshot the state entering this checkpoint line. In sequence this
            // always appends (see the `checkpoints` invariant); the replace arm
            // is defensive.
            let k = self.current_line / CHECKPOINT_INTERVAL;
            if let Some(slot) = self.checkpoints.get_mut(k) {
                *slot = self.state.clone();
            } else {
                debug_assert_eq!(k, self.checkpoints.len());
                self.checkpoints.push(self.state.clone());
            }
        }

        self.spans.iter().cloned()
    }

    fn current_line(&self) -> usize {
        self.current_line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_core::text::highlighter::Highlighter as _;

    fn settings(syntax: &'static str) -> Settings {
        Settings {
            syntax,
            theme: ThemeMode::Light,
        }
    }

    /// The set of foreground colours a syntax produces for one line, under a given
    /// theme — used to prove the theme actually drives the palette (#36).
    fn line_colors(
        syntax: &'static str,
        theme: ThemeMode,
        line: &str,
    ) -> std::collections::BTreeSet<[u8; 4]> {
        let mut hl = SyntectHighlighter::new(&Settings { syntax, theme });
        hl.highlight_line(line)
            .map(|(_, h)| {
                let c = h.0.foreground;
                [c.r, c.g, c.b, c.a]
            })
            .collect()
    }

    /// Toggling the theme (#36) changes the palette: the same Rust line comes out
    /// with a different set of colours under the light vs the dark theme, and the
    /// two [`Settings`] compare unequal (which is what makes the widget rebuild
    /// the highlighter on a theme switch).
    #[test]
    fn theme_mode_changes_the_palette() {
        let light = line_colors("Rust", ThemeMode::Light, "fn main() {}");
        let dark = line_colors("Rust", ThemeMode::Dark, "fn main() {}");
        assert!(!light.is_empty() && !dark.is_empty());
        assert_ne!(light, dark, "light and dark themes must colour differently");
        assert_ne!(
            settings("Rust"),
            Settings {
                syntax: "Rust",
                theme: ThemeMode::Dark,
            },
            "a theme change must make Settings compare unequal so the widget rebuilds"
        );
    }

    /// Every span a Rust line produces must carry a colour, and a keyword must
    /// come out a different colour than surrounding whitespace/identifiers —
    /// proving real highlighting (not one flat colour) is happening.
    #[test]
    fn rust_keyword_is_coloured_distinctly() {
        let mut hl = SyntectHighlighter::new(&settings("Rust"));
        let spans: Vec<_> = hl.highlight_line("fn main() {}").collect();
        assert!(!spans.is_empty(), "a Rust line should produce spans");

        // The `fn` keyword occupies bytes 0..2.
        let kw = spans
            .iter()
            .find(|(range, _)| range.start == 0 && range.end >= 2)
            .expect("a span covering the `fn` keyword");
        let kw_color = to_format(&kw.1, &iced::Theme::Light).color.unwrap();

        // A later span (the identifier `main`) should differ in colour.
        let differs = spans
            .iter()
            .any(|(_, h)| to_format(h, &iced::Theme::Light).color.unwrap() != kw_color);
        assert!(differs, "highlighting should use more than one colour");
    }

    /// The reported regression: a TOML buffer must actually highlight (the
    /// language now comes from the extended two-face set). More than one colour
    /// across a representative line proves it, end to end through the highlighter.
    #[test]
    fn toml_highlights_with_multiple_colours() {
        let mut hl = SyntectHighlighter::new(&settings("TOML"));
        let spans: Vec<_> = hl.highlight_line("name = \"notepad-extra\"").collect();
        assert!(!spans.is_empty(), "a TOML line should produce spans");
        let colors: std::collections::BTreeSet<[u8; 4]> = spans
            .iter()
            .map(|(_, h)| {
                let c = h.0.foreground;
                [c.r, c.g, c.b, c.a]
            })
            .collect();
        assert!(
            colors.len() > 1,
            "TOML should be syntax-highlighted, not flat"
        );
    }

    /// Plain Text resolves to the plain-text syntax: spans exist but every one is
    /// the same (default) colour — i.e. visually unhighlighted.
    #[test]
    fn plain_text_is_single_coloured() {
        let mut hl = SyntectHighlighter::new(&settings(notepad_syntax::PLAIN_TEXT));
        let spans: Vec<_> = hl.highlight_line("fn main() {}").collect();
        let colors: std::collections::BTreeSet<[u8; 4]> = spans
            .iter()
            .map(|(_, h)| {
                let c = h.0.foreground;
                [c.r, c.g, c.b, c.a]
            })
            .collect();
        assert!(colors.len() <= 1, "plain text must not colour tokens");
    }

    /// An unknown syntax name must fall back rather than panic.
    #[test]
    fn unknown_syntax_falls_back_to_plain_text() {
        // Asserting only "didn't panic" would leave the name's real claim — the
        // *fall back to plain text* — unchecked: falling back to Rust, or to
        // whatever syntax happened to be first in the set, would pass too. Check
        // the same property `plain_text_is_single_coloured` does.
        let mut hl = SyntectHighlighter::new(&settings("Not A Real Syntax"));
        let spans: Vec<_> = hl.highlight_line("fn main() {}").collect();
        assert!(!spans.is_empty(), "the line still yields spans");
        let colors: std::collections::BTreeSet<[u8; 4]> = spans
            .iter()
            .map(|(_, h)| {
                let c = h.0.foreground;
                [c.r, c.g, c.b, c.a]
            })
            .collect();
        assert!(
            colors.len() <= 1,
            "an unknown syntax must render like plain text, not like code"
        );
    }

    // --- Sparse checkpoints (#126) ---

    /// Rust lines whose highlighting depends on *state carried across lines*:
    /// block comments and raw strings that open on one line and close on a
    /// later one, so resuming from a wrong state would visibly mis-colour the
    /// lines in between. Long enough to span several checkpoints.
    fn stateful_corpus(n: usize) -> Vec<String> {
        (0..n)
            .map(|i| match i % 9 {
                0 => format!("/// doc comment {i}"),
                1 => format!("fn f{i}(x: &str) -> Option<u32> {{"),
                2 => "    let s = \"a string with {} braces\"; /* open".to_string(),
                3 => "       still in the block comment */".to_string(),
                4 => format!("    if x.len() > {i} {{ return Some({i}); }}"),
                5 => "    // trailing comment".to_string(),
                6 => "    let raw = r#\"raw string".to_string(),
                7 => "    continues\"#;".to_string(),
                _ => "}".to_string(),
            })
            .collect()
    }

    type Spans = Vec<(Range<usize>, Style)>;

    fn spans_of(hl: &mut SyntectHighlighter, line: &str) -> Spans {
        hl.highlight_line(line).map(|(r, h)| (r, h.0)).collect()
    }

    /// Every line's spans from a brand-new highlighter run over the whole buffer.
    fn fresh_run(lines: &[String]) -> Vec<Spans> {
        let mut hl = SyntectHighlighter::new(&settings("Rust"));
        lines.iter().map(|l| spans_of(&mut hl, l)).collect()
    }

    /// For every line `k`: a highlighter that has done the whole buffer, told
    /// that `k` changed, resumes at a checkpoint at or (at most 31 lines)
    /// before `k`, and re-highlighting from `current_line()` to the end yields
    /// exactly the spans a fresh run produces for each of those lines. The same
    /// instance is rewound again and again, so repeated rewinds — and
    /// re-taking checkpoints on the way back down — are covered too.
    #[test]
    fn resuming_from_a_checkpoint_reproduces_a_fresh_run_for_every_line() {
        let lines = stateful_corpus(3 * CHECKPOINT_INTERVAL + 7);
        let fresh = fresh_run(&lines);
        assert!(
            fresh.iter().any(|spans| spans.len() > 1),
            "the corpus must actually produce multi-span lines"
        );

        let mut hl = SyntectHighlighter::new(&settings("Rust"));
        for l in &lines {
            let _ = hl.highlight_line(l).count();
        }
        assert_eq!(hl.current_line(), lines.len());

        for k in 0..lines.len() {
            hl.change_line(k);
            let from = hl.current_line();
            assert!(from <= k, "resume point {from} must not be after line {k}");
            assert!(
                k - from < CHECKPOINT_INTERVAL,
                "resume point {from} is more than an interval before {k}"
            );
            assert!(from.is_multiple_of(CHECKPOINT_INTERVAL));
            for (i, line) in lines.iter().enumerate().skip(from) {
                assert_eq!(
                    spans_of(&mut hl, line),
                    fresh[i],
                    "line {i} after change_line({k}) differs from a fresh run"
                );
            }
            assert_eq!(hl.current_line(), lines.len());
        }
    }

    /// An actual edit: line `k` is replaced by text that *changes the state* for
    /// everything after it (an unterminated block comment), then put back. In
    /// both directions the resumed highlighter must match a fresh run over the
    /// current buffer — proving stale checkpoints after the edit are dropped
    /// and re-taken, never reused.
    #[test]
    fn resuming_after_an_edit_matches_a_fresh_run_of_the_edited_buffer() {
        let lines = stateful_corpus(2 * CHECKPOINT_INTERVAL + 5);
        let fresh = fresh_run(&lines);
        let mut hl = SyntectHighlighter::new(&settings("Rust"));
        for l in &lines {
            let _ = hl.highlight_line(l).count();
        }

        for k in 0..lines.len() {
            let mut edited = lines.clone();
            edited[k] = "/* an edit that opens a comment".to_string();
            let fresh_edited = fresh_run(&edited);

            hl.change_line(k);
            let from = hl.current_line();
            for (i, line) in edited.iter().enumerate().skip(from) {
                assert_eq!(
                    spans_of(&mut hl, line),
                    fresh_edited[i],
                    "edited buffer, line {i}, after change_line({k})"
                );
            }

            // Undo the edit: the checkpoints taken while highlighting the edited
            // buffer must be discarded again.
            hl.change_line(k);
            let from = hl.current_line();
            for (i, line) in lines.iter().enumerate().skip(from) {
                assert_eq!(
                    spans_of(&mut hl, line),
                    fresh[i],
                    "restored buffer, line {i}, after change_line({k})"
                );
            }
        }
    }

    /// Checkpoints are sparse — one per `CHECKPOINT_INTERVAL` lines plus the
    /// start-of-document state — which is the whole memory win of #126.
    #[test]
    fn checkpoints_are_one_per_interval() {
        let lines = stateful_corpus(5 * CHECKPOINT_INTERVAL + 3);
        let mut hl = SyntectHighlighter::new(&settings("Rust"));
        for (i, l) in lines.iter().enumerate() {
            assert_eq!(hl.checkpoints.len(), i / CHECKPOINT_INTERVAL + 1);
            let _ = hl.highlight_line(l).count();
        }
        assert_eq!(hl.checkpoints.len(), lines.len() / CHECKPOINT_INTERVAL + 1);
        // A rewind drops the checkpoints after the resume point…
        hl.change_line(CHECKPOINT_INTERVAL + 1);
        assert_eq!(hl.current_line(), CHECKPOINT_INTERVAL);
        assert_eq!(hl.checkpoints.len(), 2);
        // …and a settings change starts over from the pristine state.
        hl.update(&settings("TOML"));
        assert_eq!(hl.current_line(), 0);
        assert_eq!(hl.checkpoints.len(), 1);
    }

    /// `change_line` at or after the current line is a no-op: everything held
    /// describes only lines before `current_line`, so nothing is stale. That is
    /// also what keeps an out-of-range index bounded — it never resets or
    /// empties anything.
    #[test]
    fn change_line_is_bounded() {
        let mut hl = SyntectHighlighter::new(&settings("Rust"));
        for i in 0..5 {
            let _ = hl.highlight_line(&format!("let x{i} = {i};")).count();
        }
        hl.change_line(2);
        assert!(hl.current_line() <= 2);
        assert!(!hl.checkpoints.is_empty());
        for i in 0..40 {
            let _ = hl.highlight_line(&format!("let y{i} = {i};")).count();
        }
        let at = hl.current_line();
        hl.change_line(at); // the line being appended next: nothing to redo
        assert_eq!(hl.current_line(), at);
        hl.change_line(9999); // way past the end
        assert_eq!(hl.current_line(), at);
        assert!(hl.checkpoints.len() == at / CHECKPOINT_INTERVAL + 1);
        hl.change_line(0);
        assert_eq!(hl.current_line(), 0);
        assert_eq!(hl.checkpoints.len(), 1);
    }

    /// Soak: highlighting a very large buffer and a single very long line must
    /// complete without panicking (huge-file highlight-perf focus of #32; the
    /// widget-level soak is #80).
    #[test]
    fn huge_input_highlights_without_panic() {
        let mut hl = SyntectHighlighter::new(&settings("Rust"));
        for i in 0..15_000 {
            let _ = hl
                .highlight_line(&format!("let v{} = {};", i % 97, i))
                .count();
        }
        // The retained state is one checkpoint per interval, not one per line (#126).
        assert_eq!(hl.checkpoints.len(), 15_000 / CHECKPOINT_INTERVAL + 1);
        // A single very long line (mirrors #32's "one enormous line" focus).
        let mut hl = SyntectHighlighter::new(&settings("Rust"));
        let long = "a+".repeat(40_000);
        let _ = hl.highlight_line(&long).count();
    }

    /// Malformed / binary-ish input must not panic the parser.
    #[test]
    fn malformed_input_does_not_panic() {
        let mut hl = SyntectHighlighter::new(&settings("Rust"));
        for line in ["\u{0}\u{1}\u{2}", "```", "\"unterminated", "🙂🙂🙂", ""] {
            let _ = hl.highlight_line(line).count();
        }
    }
}
