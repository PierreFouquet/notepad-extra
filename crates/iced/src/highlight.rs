//! Syntax highlighting for the editor widget (#32).
//!
//! iced ships `iced_highlighter`, but it pulls syntect built with **oniguruma**
//! (a C dependency the epic forbids), so we implement the
//! [`iced_core::text::Highlighter`] trait ourselves on top of
//! [`notepad_syntax`]'s **fancy-regex** `SyntaxSet` / `ThemeSet`. The vendored
//! `text_editor` plugs this in through `highlight_with::<_>()` — never iced's
//! gated `highlighter` feature — so oniguruma never enters the crate tree.
//!
//! The highlighter is fed **one line at a time** and keeps, per line, a snapshot
//! of the parser + highlighter state *entering* that line. Re-highlighting after
//! an edit then only re-parses from the changed line downward — the incremental
//! model iced's editor expects (see the trait's `change_line` / `current_line`).
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
/// [`notepad_core::app::Document::language`]) and the light/dark [`ThemeMode`]
/// (#36). `PartialEq` lets the widget rebuild the highlighter only when one of
/// these actually changes — so toggling the theme re-highlights the buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub syntax: String,
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

/// A per-line snapshot: the parser and highlighter state *at the start* of a
/// line. Both syntect types are `Clone`, which is what makes the incremental
/// resume cheap.
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
    /// `caches[i]` is the state entering line `i`; `caches[0]` is always the
    /// pristine start-of-document state, so the vector is never empty.
    caches: Vec<LineState>,
    /// The start-of-document state, kept to refill the cache on a full reset.
    initial: LineState,
    current_line: usize,
}

impl SyntectHighlighter {
    fn build(settings: &Settings) -> Self {
        let syntaxes = notepad_syntax::syntax_set();
        // An unknown / "Plain Text" name resolves to syntect's plain-text syntax,
        // which emits only default-styled spans — i.e. visually unhighlighted —
        // so the view needs no separate "no highlighting" code path.
        let syntax = syntaxes
            .find_syntax_by_name(&settings.syntax)
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
            caches: vec![initial.clone()],
            initial,
            current_line: 0,
        }
    }
}

impl highlighter::Highlighter for SyntectHighlighter {
    type Settings = Settings;
    type Highlight = Highlight;
    type Iterator<'a> = std::vec::IntoIter<(Range<usize>, Highlight)>;

    fn new(settings: &Self::Settings) -> Self {
        Self::build(settings)
    }

    fn update(&mut self, new_settings: &Self::Settings) {
        *self = Self::build(new_settings);
    }

    fn change_line(&mut self, line: usize) {
        // The state *entering* `line` is unchanged (it depends only on the lines
        // before it), so keep `caches[0..=line]` and drop the now-stale rest;
        // resume highlighting from `line`. Clamp so we never index past the cache
        // and never empty it (`caches[0]` must survive).
        let line = line.min(self.caches.len().saturating_sub(1));
        self.caches.truncate(line + 1);
        self.current_line = line;
    }

    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        // Resume from the snapshot entering this line, cloning it so the cached
        // entry stays the *entering* state (the produced state seeds the next).
        let mut state = self
            .caches
            .get(self.current_line)
            .or_else(|| self.caches.last())
            .cloned()
            .unwrap_or_else(|| self.initial.clone());

        let ops = state
            .parser
            .parse_line(line, self.syntaxes)
            .unwrap_or_default();

        // Collect eagerly so the borrows of `state`, `line` and `inner` are gone
        // by the time we return an owned iterator (no lifetime tangle).
        let spans: Vec<(Range<usize>, Highlight)> =
            RangedHighlightIterator::new(&mut state.highlight, &ops, line, &self.inner)
                .map(|(style, _text, range)| (range, Highlight(style)))
                .collect();

        // Record the state entering the *next* line.
        self.current_line += 1;
        if self.current_line < self.caches.len() {
            self.caches[self.current_line] = state;
        } else {
            self.caches.push(state);
        }

        spans.into_iter()
    }

    fn current_line(&self) -> usize {
        self.current_line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced_core::text::highlighter::Highlighter as _;

    fn settings(syntax: &str) -> Settings {
        Settings {
            syntax: syntax.to_string(),
            theme: ThemeMode::Light,
        }
    }

    /// The set of foreground colours a syntax produces for one line, under a given
    /// theme — used to prove the theme actually drives the palette (#36).
    fn line_colors(
        syntax: &str,
        theme: ThemeMode,
        line: &str,
    ) -> std::collections::BTreeSet<[u8; 4]> {
        let mut hl = SyntectHighlighter::new(&Settings {
            syntax: syntax.to_string(),
            theme,
        });
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
                syntax: "Rust".to_string(),
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

    /// `change_line` resets the resume point and never empties the cache or
    /// panics, even for an out-of-range line index.
    #[test]
    fn change_line_is_bounded() {
        let mut hl = SyntectHighlighter::new(&settings("Rust"));
        for i in 0..5 {
            let _ = hl.highlight_line(&format!("let x{i} = {i};")).count();
        }
        hl.change_line(2);
        assert_eq!(hl.current_line(), 2);
        hl.change_line(9999); // way past the end
        assert!(hl.current_line() < hl.caches.len());
        assert!(!hl.caches.is_empty());
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
