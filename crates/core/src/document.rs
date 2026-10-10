//! The document model (#126): one open buffer, its text, and the derived data
//! the shell reads on **every frame** — kept in sync structurally rather than
//! recomputed.
//!
//! The status bar (#37/#55) shows the document's character, word and line
//! totals plus the caret's line/column, and the tab strip shows the unsaved
//! ("•") marker. `Shell::view` asks for all of them on every redraw — every
//! keystroke, scroll, mouse move and the 500 ms caret blink — and until #126 each
//! answer was recomputed from scratch: `chars().count()`, `unicode_words()` and a
//! `\n` scan over the whole buffer, a whole-buffer compare against the saved
//! copy, and an O(n) walk from the top of the text to turn the widget's
//! `(line, column)` caret into a byte offset. At 200k lines that was ~50 ms per
//! frame, and it grew with the document.
//!
//! [`Document`] therefore owns its text privately and keeps three derived
//! values alongside it:
//!
//! * the character and UAX #29 word totals ([`Document::char_count`],
//!   [`Document::word_count`]), and the number of `\r`s, which says whether the
//!   editor's lines can disagree with the line index ([`Document::has_cr`],
//!   #153),
//! * a **line-start index** — the byte offset of every line's first byte —
//!   which doubles as the line count and makes `(line, column)` ↔ byte-offset
//!   conversion a binary search ([`Document::line_start`],
//!   [`Document::offset_at`], [`Document::line_col_of`]),
//! * whether the text is byte-equal to the saved baseline, so
//!   [`Document::dirty`] is a flag read, not a compare.
//!
//! Every mutation goes through a `Document` method, so there is no path that
//! rewrites the buffer and forgets the derived data: a load/reload recounts in
//! one pass, while an edit, undo, redo or replace updates everything
//! **incrementally** from its single-span [`Edit`] — the totals by delta, the
//! index by splicing, and the words by recounting only the lines the edit
//! touched. The last of these relies on how UAX #29 treats line breaks: a word
//! boundary falls after every `\n` and before every `\n` not preceded by `\r`
//! (WB3a/WB3b; WB3 keeps `\r\n` as one segment), so the only segments that can
//! contain `\n` are `\n` itself and `\r\n`, neither of which holds a word
//! character. Cutting the text at each `\n` therefore never splits or merges a
//! word-like segment, and segmentation restarts identically after a `\n` and at
//! the start of text, so the word count of a text is the sum of its lines' word
//! counts and a change confined to some lines can only move the words on those
//! lines.

use crate::app::{DiskMeta, DiskStatus, TabId};
use crate::encoding::FileEncoding;
use crate::find;
use crate::history::{Edit, History};
use crate::text::{self, EndOfLine};
use std::path::PathBuf;
use unicode_segmentation::UnicodeSegmentation;

/// One open document.
///
/// The text is private: read it with [`Document::content`] and change it only
/// through the crate's mutation methods ([`Document::load`],
/// [`Document::commit`], [`Document::undo`], [`Document::redo`]), which keep
/// the cached totals, the line-start index and the dirty flag in step (#126).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub id: TabId,
    /// `None` until the buffer is first saved to disk.
    pub path: Option<PathBuf>,
    /// Canonical text, always `\n`-terminated internally (see [`EndOfLine`]).
    /// Private so every rewrite passes through a method that also updates the
    /// derived data below — the structural guarantee #126 asks for.
    content: String,
    /// The on-disk line ending to restore on save.
    pub eol: EndOfLine,
    /// The syntax auto-detected from the file's extension (#32), or
    /// [`notepad_syntax::PLAIN_TEXT`] for an unsaved / unknown buffer. Refreshed
    /// on every load and save; the manual override, if any, takes precedence.
    pub detected_lang: &'static str,
    /// A per-tab manual language override set from the picker (#32). `None` means
    /// "auto-detect" (use [`Document::detected_lang`]). It is deliberately *not*
    /// touched by load/save, so opening or re-saving a file never clobbers a
    /// language the user chose by hand; it lasts the tab's lifetime.
    pub manual_lang: Option<&'static str>,
    /// The text encoding this document reads and writes on disk (#50/#59/#103).
    /// Detected on open, changed via the status-bar picker
    /// ([`crate::Message::SetEncoding`]), and used to encode the bytes on save.
    /// The internal text is always UTF-8 regardless; this only names the on-disk
    /// representation, orthogonally to [`Document::eol`].
    pub encoding: FileEncoding,
    /// Undo/redo history. Private because popping an entry without applying it
    /// would desynchronise the stack from the text; use [`Document::undo`] /
    /// [`Document::redo`], which do both.
    history: History,
    /// How the buffer currently relates to the file on disk (#51). Set by the
    /// external-change watch; `InSync` for a fresh or in-sync tab. Read by the
    /// shell to raise the reload / conflict / "source gone" bar.
    pub disk_status: DiskStatus,
    /// The on-disk fingerprint captured at the last load/save (#51), or `None` for
    /// a never-saved buffer. The baseline the watch compares fresh stats against
    /// to tell an external change from our own write.
    pub(crate) disk: Option<DiskMeta>,
    /// Cached character and word totals of `content` (#126).
    totals: Totals,
    /// Byte offset of the first byte of every line of `content`, in order.
    /// Entry 0 is always `0`; every other entry is the position just after a
    /// `\n`, so the length is the line count ([`find::line_count`] semantics: a
    /// trailing `\n` yields a final empty line). Binary-searched by the
    /// `(line, column)` conversions and spliced by incremental edits (#126).
    line_starts: Vec<usize>,
    /// The last content that was saved to disk, with its own totals and the
    /// encoding it was written with. Deciding whether the buffer still matches
    /// it is what the "•" marker means.
    saved: Baseline,
    /// Whether `content` is byte-equal to `saved.text`, re-derived whenever
    /// either side changes so [`Document::dirty`] is O(1) (#126). Exact — never
    /// a hash: the compare that refreshes it short-circuits on the lengths and
    /// the cached totals and only reads the bytes when those all agree.
    matches_saved: bool,
}

/// Document-wide totals the status bar shows, cached so a frame reads two
/// integers instead of walking the buffer (#126), plus the `\r` count the shell
/// reads on every message (#153). Lines are not here: the line-start index's
/// length *is* the line count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Totals {
    chars: usize,
    words: usize,
    crs: usize,
}

impl Totals {
    /// Count `text` from scratch — the one-pass recount a load pays once.
    fn of(text: &str) -> Totals {
        Totals {
            chars: text.chars().count(),
            words: words(text),
            crs: crs(text),
        }
    }
}

/// The on-disk baseline a document is compared against for its dirty marker.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Baseline {
    /// The text as last loaded from or saved to disk.
    text: String,
    /// `text`'s totals, kept so a dirty refresh can rule out equality without
    /// reading the bytes (a different count means different text).
    totals: Totals,
    /// `text`'s line count, for the same cheap pre-check.
    lines: usize,
    /// The encoding the baseline was written with. Choosing a different save
    /// encoding on an otherwise unchanged buffer still marks it dirty (a
    /// *convert*), so the next Save re-encodes it.
    encoding: FileEncoding,
}

impl Document {
    /// A fresh, empty, untitled buffer.
    pub(crate) fn blank(id: TabId) -> Self {
        Document {
            id,
            path: None,
            content: String::new(),
            eol: EndOfLine::default(),
            detected_lang: notepad_syntax::PLAIN_TEXT,
            manual_lang: None,
            encoding: FileEncoding::default(),
            history: History::new(),
            disk_status: DiskStatus::InSync,
            disk: None,
            totals: Totals::default(),
            line_starts: vec![0],
            saved: Baseline {
                text: String::new(),
                totals: Totals::default(),
                lines: 1,
                encoding: FileEncoding::default(),
            },
            matches_saved: true,
        }
    }

    // ---- readers --------------------------------------------------------------

    /// The canonical (`\n`-terminated) text.
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Total number of characters in the document (#37). O(1): cached (#126).
    pub fn char_count(&self) -> usize {
        self.totals.chars
    }

    /// Total number of words in the document (#55), per Unicode word
    /// segmentation (UAX #29, via [`UnicodeSegmentation::unicode_words`]): `don't`
    /// is one word, `你好` is two, punctuation and whitespace count nothing.
    /// O(1): cached and maintained incrementally (#126).
    pub fn word_count(&self) -> usize {
        self.totals.words
    }

    /// Whether the text holds a `\r` (#153). The editor ends a line at a lone
    /// `\r` and takes `\r\n` and `\n\r` as one ending each, while the line
    /// index breaks at `\n` alone and counts a `\r` as a column, so the two
    /// disagree on positions after one. The shell asks this before every
    /// position conversion and, when it is true, converts through the editor's
    /// own lines instead of [`Document::offset_at`] / [`Document::line_col_of`].
    /// O(1): the count is kept with the other totals, so the check costs no
    /// scan per message.
    pub fn has_cr(&self) -> bool {
        self.totals.crs > 0
    }

    /// Total number of lines: one more than the count of `\n`, exactly as
    /// [`find::line_count`] defines it (a trailing `\n` yields a final empty
    /// line). O(1): the line-start index's length (#126).
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Byte offset of the first byte of 0-based `line`, clamped to the last line
    /// (so any line number the widget or a go-to hands us is safe). O(1).
    pub fn line_start(&self, line: usize) -> usize {
        self.line_starts[line.min(self.line_starts.len() - 1)]
    }

    /// The byte offset of the 0-based `line` and byte-`column` iced's cursor
    /// reports — the indexed counterpart of [`find::offset_at`], with the same
    /// clamping: the line saturates onto the last line, the column onto the
    /// line's own end (never spilling into the next line), and the result is
    /// floored onto a char boundary. O(1) instead of a scan from the top, which
    /// the shell paid several times per frame (#126).
    pub fn offset_at(&self, line: usize, column: usize) -> usize {
        let line = line.min(self.line_starts.len() - 1);
        let start = self.line_starts[line];
        // The line ends at its `\n` (the byte before the next line's start), or
        // at end-of-text for the last line.
        let end = self
            .line_starts
            .get(line + 1)
            .map_or(self.content.len(), |next| next - 1);
        find::floor_boundary(&self.content, start.saturating_add(column).min(end))
    }

    /// The 0-based line and **byte** column of `offset` (clamped into range and
    /// onto a char boundary) — the indexed counterpart of [`find::line_col_of`],
    /// which the shell uses to turn core byte ranges into widget cursors. A
    /// binary search over the line-start index, O(log lines), instead of
    /// counting every `\n` before the offset (#126).
    pub fn line_col_of(&self, offset: usize) -> (usize, usize) {
        let offset = find::floor_boundary(&self.content, offset);
        let line = self.line_of(offset);
        (line, offset - self.line_starts[line])
    }

    /// Whether the buffer has unsaved changes (the tab's "•" marker): the text
    /// differs from the last saved baseline, or the save encoding was changed
    /// (a pending *convert*). O(1): the text compare is a cached flag refreshed
    /// on every mutation, the encoding compare is two enum fields (#126).
    /// Exact — reverting the text to the saved bytes clears it.
    pub fn dirty(&self) -> bool {
        !self.matches_saved || self.encoding != self.saved.encoding
    }

    /// The effective language: the manual override if the user set one, otherwise
    /// the extension-detected syntax. This is what the status bar shows and what
    /// the render shell highlights with (#32).
    pub fn language(&self) -> &'static str {
        self.manual_lang.unwrap_or(self.detected_lang)
    }

    /// The on-disk fingerprint captured at the last load/save (#51), or `None` for
    /// a never-saved buffer. The shell reads it to do the cheap `mtime`/`len` gate
    /// before deciding whether to hash the file on a watch event.
    pub fn disk_meta(&self) -> Option<&DiskMeta> {
        self.disk.as_ref()
    }

    /// The tab / window title: the file name, or `Untitled` for a fresh buffer.
    ///
    /// Deliberately the basename, not the full path (#97 item 4). The window shows
    /// just the file name (the shell appends " — Notepad Extra"), so a deep
    /// path can't crowd the app name out of the title bar.
    pub fn title(&self) -> &str {
        match &self.path {
            Some(p) => {
                let s = p.to_str().unwrap_or("");
                let name = text::basename(s);
                if name.is_empty() { "Untitled" } else { name }
            }
            None => "Untitled",
        }
    }

    /// A lone, untouched `Untitled` buffer that opening a file should replace
    /// rather than leave behind (ports `logic.js`'s `shouldReuseBlankTab`).
    pub(crate) fn is_pristine_blank(&self) -> bool {
        self.path.is_none() && !self.dirty() && self.content.is_empty()
    }

    /// The 0-based line holding byte `offset` (a char boundary ≤ `len`): the
    /// last line start at or before it. Binary search over the index.
    fn line_of(&self, offset: usize) -> usize {
        // Entry 0 is `0 <= offset`, so the partition point is at least 1.
        self.line_starts.partition_point(|&start| start <= offset) - 1
    }

    // ---- mutation -------------------------------------------------------------

    /// Adopt `text` as both the buffer and the clean on-disk baseline: a fresh
    /// load, a reload after an external change, or a reopen under another
    /// encoding. Re-counts everything in a single pass (the one O(n) a load
    /// pays), resets the history, and takes `text` by move so the only copy made
    /// is the baseline's (#126). Path and language are the caller's business:
    /// a reload keeps them, a load sets them.
    pub(crate) fn load(
        &mut self,
        text: String,
        eol: EndOfLine,
        encoding: FileEncoding,
        disk: Option<DiskMeta>,
    ) {
        self.totals = Totals::of(&text);
        self.line_starts = line_starts(&text);
        self.saved = Baseline {
            text: text.clone(),
            totals: self.totals,
            lines: self.line_starts.len(),
            encoding,
        };
        self.content = text;
        self.eol = eol;
        self.encoding = encoding;
        self.history = History::new(); // loaded content is the clean baseline
        self.disk_status = DiskStatus::InSync;
        self.disk = disk;
        self.matches_saved = true;
    }

    /// The current text and encoding were just written to disk: they become the
    /// clean baseline, so the "•" clears and undoing back to this point later
    /// clears it again. The totals come from the live cache, so no recount.
    pub(crate) fn mark_saved(&mut self) {
        self.saved.text.clone_from(&self.content);
        self.saved.totals = self.totals;
        self.saved.lines = self.line_starts.len();
        self.saved.encoding = self.encoding;
        self.history.mark_saved();
        self.matches_saved = true;
    }

    /// Replace the buffer with `new_content` as one undo-able step — the path
    /// every whole-text rewrite takes: a keystroke reported by the editor
    /// ([`crate::Message::Edited`]), replace-next / replace-all, and the text
    /// operations (#54). Returns `false`, recording nothing, when the text is
    /// unchanged (a caret move).
    ///
    /// The change is reduced to a single span by [`crate::history::diff`] and
    /// the derived data is updated from that span alone, while `new_content`
    /// is adopted by move: the cost is the diff plus the edited lines, never a
    /// recount of the whole buffer (#126).
    pub(crate) fn commit(&mut self, new_content: String) -> bool {
        let Some(edit) = crate::history::diff(&self.content, &new_content) else {
            return false;
        };
        self.record(edit, new_content);
        true
    }

    /// [`Document::commit`] for a rewrite the core makes itself (replace-next,
    /// replace-all, the text operations), returning a copy of the applied
    /// [`Edit`]. The shell applies that same span to the editor buffer it
    /// already has instead of rebuilding the buffer from the whole text, which
    /// made cosmic-text reshape every line (#137). The copy is O(edit). A
    /// keystroke goes through `commit`: its text came from the editor, so the
    /// buffer needs nothing.
    pub(crate) fn commit_rewrite(&mut self, new_content: String) -> Option<Edit> {
        let edit = crate::history::diff(&self.content, &new_content)?;
        let applied = edit.clone();
        self.record(edit, new_content);
        Some(applied)
    }

    /// Adopt `new_content`, which `edit` turns the current text into, and
    /// record the step in the history.
    fn record(&mut self, edit: Edit, new_content: String) {
        self.splice(&edit, |content| *content = new_content);
        self.history.record(edit);
    }

    /// Undo the most recent step, applying it to the buffer and returning the
    /// applied [`Edit`] (so the caller can reveal where it landed). `None` when
    /// there is nothing to undo.
    pub(crate) fn undo(&mut self) -> Option<Edit> {
        let edit = self.history.undo()?;
        self.splice(&edit, |content| edit.apply(content));
        Some(edit)
    }

    /// Redo the most recently undone step, applying it to the buffer and
    /// returning the applied [`Edit`]. `None` when there is nothing to redo.
    pub(crate) fn redo(&mut self) -> Option<Edit> {
        let edit = self.history.redo()?;
        self.splice(&edit, |content| edit.apply(content));
        Some(edit)
    }

    /// Apply `edit` to the text via `rewrite` and bring every derived value up
    /// to date **incrementally** (#126). `rewrite` performs the actual text
    /// change — a move of the editor's whole new string, or [`Edit::apply`] in
    /// place for undo/redo — and must realise exactly `edit`.
    ///
    /// * chars and `\r`s: `+ count(inserted) − count(removed)`;
    /// * line index: the starts inside the removed range go, the starts from
    ///   the inserted text come in, and everything after shifts by the length
    ///   delta;
    /// * words: the lines the edit spans are recounted before and after, and
    ///   the difference applied. Correct because no UAX #29 word-like segment
    ///   contains a `\n` (the only segments that do are `\n` and `\r\n`, see the
    ///   module docs), so no word straddles a line and the rest of the
    ///   document's words are untouched;
    /// * dirty: re-derived (cheaply, see [`Document::refresh_dirty`]).
    ///
    /// Cost: O(edit size + the touched lines + the index splice), not O(text).
    fn splice(&mut self, edit: &Edit, rewrite: impl FnOnce(&mut String)) {
        let at = edit.at;
        let removed_end = at + edit.removed.len();
        // Index positions: `lo` is the first line start strictly after `at`
        // (so `lo - 1` is the line holding `at`, whose start survives the edit),
        // `hi` the first strictly after the removed range. Starts in `lo..hi`
        // belong to `\n`s inside the removed text and go with it.
        let lo = self.line_starts.partition_point(|&s| s <= at);
        let hi = self.line_starts.partition_point(|&s| s <= removed_end);
        let span_start = self.line_starts[lo - 1];
        let old_words = words(&self.content[span_start..line_end(&self.content, removed_end)]);
        let removed_chars = edit.removed.chars().count();
        let inserted_chars = edit.inserted.chars().count();
        let removed_crs = crs(&edit.removed);
        let inserted_crs = crs(&edit.inserted);

        rewrite(&mut self.content);

        let inserted_end = at + edit.inserted.len();
        let new_words = words(&self.content[span_start..line_end(&self.content, inserted_end)]);

        let tail = self.line_starts.len() - hi; // entries after the edit, to shift
        self.line_starts.splice(
            lo..hi,
            edit.inserted.match_indices('\n').map(|(i, _)| at + i + 1),
        );
        let shifted_from = self.line_starts.len() - tail;
        for start in &mut self.line_starts[shifted_from..] {
            // Every shifted start is past `removed_end >= removed.len()`, so
            // subtracting first cannot underflow.
            *start = *start - edit.removed.len() + edit.inserted.len();
        }
        self.totals.chars = self.totals.chars - removed_chars + inserted_chars;
        self.totals.crs = self.totals.crs - removed_crs + inserted_crs;
        self.totals.words = self.totals.words - old_words + new_words;
        self.refresh_dirty();
    }

    /// Re-derive [`Document::matches_saved`] after the text changed: compare the
    /// lengths first, then the cached totals, and read the bytes only when all
    /// of those agree — so the O(n) compare runs only when the buffer could
    /// genuinely have returned to its saved state, and never per frame (#126).
    /// Exact by construction: the final arm is the byte equality itself.
    fn refresh_dirty(&mut self) {
        self.matches_saved = self.content.len() == self.saved.text.len()
            && self.totals == self.saved.totals
            && self.line_starts.len() == self.saved.lines
            && self.content == self.saved.text;
    }

    /// Recount everything from scratch and assert the cached values match: the
    /// oracle the invalidation proptests run after every message (#126), kept
    /// here so the private fields need no accessors.
    #[cfg(test)]
    pub(crate) fn assert_derived_data_matches_recount(&self) {
        assert_eq!(
            self.totals,
            Totals::of(&self.content),
            "chars/words/CRs drifted"
        );
        assert_eq!(
            self.line_starts,
            line_starts(&self.content),
            "line index drifted"
        );
        assert_eq!(self.line_count(), find::line_count(&self.content));
        assert_eq!(self.saved.totals, Totals::of(&self.saved.text));
        assert_eq!(self.saved.lines, find::line_count(&self.saved.text));
        assert_eq!(
            self.dirty(),
            self.content != self.saved.text || self.encoding != self.saved.encoding,
            "dirty flag drifted"
        );
    }
}

/// Number of words in `text`, per Unicode word segmentation (UAX #29). This
/// counts only *word-like* segments — runs of letters/numbers, apostrophe-joined
/// words like `don't`, and individual CJK ideographs — while whitespace and
/// standalone punctuation contribute nothing. It therefore differs from a plain
/// whitespace split (`"a,b,c"` is three words, not one; `"你好"` is two, not one),
/// which is the more Unicode-correct behaviour the status bar aims for (#55).
fn words(text: &str) -> usize {
    text.unicode_words().count()
}

/// Number of `\r`s in `text` (#153): a memchr-driven search, so a load pays
/// one fast pass and an edit pays its own span.
fn crs(text: &str) -> usize {
    text.matches('\r').count()
}

/// The line-start index of `text` from scratch: `0`, then the byte after every
/// `\n`. Its length is [`find::line_count`], and the vector is sized to exactly
/// that up front: `match_indices` gives `extend` no length hint, so a growing
/// `Vec` doubles its way up and ends at up to twice the entries it holds — on a
/// 200k-line file, a megabyte of index pinned for the life of the tab for
/// nothing. Counting the newlines first is a vectorised pass far cheaper than
/// the index walk itself (#126).
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = Vec::with_capacity(find::line_count(text));
    starts.push(0);
    starts.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    starts
}

/// The end of the line containing byte `from` (a char boundary): the next `\n`
/// at or after it, or the end of the text.
fn line_end(text: &str, from: usize) -> usize {
    text[from..].find('\n').map_or(text.len(), |i| from + i)
}

/// Test-only text strategies shared by this module's proptests and the
/// `update()`-path invalidation proptest in `app.rs` (#126).
#[cfg(test)]
pub(crate) mod strategies {
    use proptest::prelude::*;

    /// Characters chosen to stress UAX #29 at edit seams (#126): plain letters
    /// and digits, the apostrophe / colon / comma / period mid-word joiners,
    /// multi-byte Latin, CJK ideographs, an emoji, regional indicators (paired
    /// into one flag by WB15/16), a zero-width joiner, a combining mark, a lone
    /// CR, and of course `\n`.
    pub(crate) fn seam_char() -> impl Strategy<Value = char> {
        prop_oneof![
            Just('a'),
            Just('b'),
            Just('1'),
            Just(' '),
            Just('\n'),
            Just('\''),
            Just(':'),
            Just(','),
            Just('.'),
            Just('_'),
            Just('é'),
            Just('你'),
            Just('好'),
            Just('😀'),
            Just('\u{1F1EB}'),
            Just('\u{1F1F7}'),
            Just('\u{200D}'),
            Just('\u{0301}'),
            Just('\r'),
        ]
    }

    pub(crate) fn seam_text(max: usize) -> impl Strategy<Value = String> {
        prop::collection::vec(seam_char(), 0..max).prop_map(|cs| cs.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::strategies::seam_text;
    use super::*;
    use proptest::prelude::*;

    /// A document holding `text` as a clean, loaded baseline.
    fn doc(text: &str) -> Document {
        let mut d = Document::blank(1);
        d.load(
            text.to_string(),
            EndOfLine::Lf,
            FileEncoding::default(),
            None,
        );
        d
    }

    #[test]
    fn a_blank_document_has_one_empty_line_and_is_clean() {
        let d = Document::blank(7);
        assert_eq!(d.id, 7);
        assert_eq!(d.content(), "");
        assert_eq!(d.char_count(), 0);
        assert_eq!(d.word_count(), 0);
        assert_eq!(d.line_count(), 1);
        assert_eq!(d.line_start(0), 0);
        assert!(!d.dirty());
        assert!(d.is_pristine_blank());
        assert_eq!(d.title(), "Untitled");
        assert_eq!(d.language(), notepad_syntax::PLAIN_TEXT);
        assert!(d.disk_meta().is_none());
        d.assert_derived_data_matches_recount();
    }

    #[test]
    fn load_counts_everything_once_and_moves_the_text() {
        let d = doc("don't stop\n你好 a,b,c\n");
        assert_eq!(d.char_count(), "don't stop\n你好 a,b,c\n".chars().count());
        assert_eq!(d.word_count(), 2 + 2 + 3);
        assert_eq!(d.line_count(), 3);
        assert_eq!(d.line_start(0), 0);
        assert_eq!(d.line_start(1), 11);
        assert_eq!(d.line_start(2), "don't stop\n你好 a,b,c\n".len());
        assert_eq!(
            d.line_start(999),
            d.line_start(2),
            "clamps to the last line"
        );
        assert!(!d.dirty());
        assert!(!d.is_pristine_blank());
        d.assert_derived_data_matches_recount();
        // The index is allocated once at its exact final size, never grown
        // (#126): a doubling `Vec` would sit at 4 here, not 3.
        assert_eq!(d.line_starts.capacity(), d.line_starts.len());
        assert_eq!(d.line_starts.len(), 3);
    }

    #[test]
    fn commit_updates_counts_incrementally_and_records_one_step() {
        let mut d = doc("ab\ncd");
        // Replace the newline with a space: two lines merge, two words become one.
        assert!(d.commit("ab cd".into()));
        assert_eq!(d.content(), "ab cd");
        assert_eq!((d.char_count(), d.word_count(), d.line_count()), (5, 2, 1));
        d.assert_derived_data_matches_recount();
        // Join the words: "abcd" is one word, no newline.
        assert!(d.commit("abcd".into()));
        assert_eq!((d.char_count(), d.word_count(), d.line_count()), (4, 1, 1));
        assert!(d.dirty());
        d.assert_derived_data_matches_recount();
        // An identical text is a caret move: nothing recorded, nothing changed.
        assert!(!d.commit("abcd".into()));
        // Two real steps were recorded: undo both lands back on the baseline.
        assert!(d.undo().is_some());
        assert!(d.undo().is_some());
        assert_eq!(d.content(), "ab\ncd");
        assert!(!d.dirty());
        assert!(d.undo().is_none(), "the stack is drained");
        d.assert_derived_data_matches_recount();
    }

    #[test]
    fn the_cr_flag_follows_loads_edits_undo_and_redo() {
        // #153: the shell picks its position conversion from this flag on every
        // message, so it has to be exact after each kind of change.
        assert!(!doc("one\ntwo").has_cr());
        let mut d = doc("one\rtwo\rthree"); // a CR-only file keeps its CRs
        assert!(d.has_cr());
        // Removing one of two CRs leaves the other.
        assert!(d.commit("one two\rthree".into()));
        assert!(d.has_cr());
        // A replacement that removes the last CR inside a wider span.
        assert!(d.commit("one three".into()));
        assert!(!d.has_cr());
        d.assert_derived_data_matches_recount();
        // A paste brings one back; undo takes it away and redo restores it.
        assert!(d.commit("one\r\nthree".into()));
        assert!(d.has_cr());
        assert!(d.undo().is_some());
        assert!(!d.has_cr());
        assert!(d.redo().is_some());
        assert!(d.has_cr());
        d.assert_derived_data_matches_recount();
        // A reload recounts from scratch.
        d.load("plain".into(), EndOfLine::Lf, FileEncoding::default(), None);
        assert!(!d.has_cr());
        d.assert_derived_data_matches_recount();
    }

    #[test]
    fn inserting_newlines_splices_the_line_index_and_shifts_the_tail() {
        let mut d = doc("aa\nbb\ncc");
        // Insert "x\ny\n" at the start of line 1: two new line starts, and the
        // start of "cc" shifts right by 4.
        assert!(d.commit("aa\nx\ny\nbb\ncc".into()));
        assert_eq!(d.line_count(), 5);
        assert_eq!(
            (0..5).map(|l| d.line_start(l)).collect::<Vec<_>>(),
            vec![0, 3, 5, 7, 10]
        );
        assert_eq!(d.word_count(), 5);
        d.assert_derived_data_matches_recount();
        // Delete a span straddling lines ("y\nbb\nc" → "") so starts inside it go.
        assert!(d.commit("aa\nx\nc".into()));
        assert_eq!(d.line_count(), 3);
        assert_eq!(d.word_count(), 3);
        d.assert_derived_data_matches_recount();
    }

    #[test]
    fn a_word_spliced_across_a_line_edge_is_recounted_not_double_counted() {
        // "don" + "'t" are two words on separate lines (the apostrophe rule needs
        // a letter on both sides *within* a line); joining the lines makes one.
        let mut d = doc("don\n't");
        assert_eq!(d.word_count(), 2);
        assert!(d.commit("don't".into()));
        assert_eq!(d.word_count(), 1);
        d.assert_derived_data_matches_recount();
        // Splitting a CJK pair across lines keeps two words either way.
        let mut d = doc("你好");
        assert!(d.commit("你\n好".into()));
        assert_eq!(d.word_count(), 2);
        d.assert_derived_data_matches_recount();
    }

    #[test]
    fn undo_and_redo_apply_in_place_and_track_the_dirty_flag() {
        let mut d = doc("one\ntwo");
        assert!(d.commit("one\ntwo\nthree".into()));
        assert!(d.dirty());
        let undone = d.undo().expect("a step to undo");
        assert_eq!(undone.at, 7);
        assert_eq!(d.content(), "one\ntwo");
        assert!(!d.dirty(), "back on the saved bytes → clean");
        d.assert_derived_data_matches_recount();
        let redone = d.redo().expect("a step to redo");
        assert_eq!(redone.inserted, "\nthree");
        assert_eq!(d.content(), "one\ntwo\nthree");
        assert!(d.dirty());
        d.assert_derived_data_matches_recount();
        assert!(d.redo().is_none(), "nothing left to redo");
    }

    #[test]
    fn mark_saved_adopts_the_text_and_encoding_as_the_baseline() {
        let mut d = doc("a");
        d.commit("ab".into());
        d.encoding = FileEncoding::from_label("Windows-1252").unwrap();
        assert!(d.dirty());
        d.mark_saved();
        assert!(!d.dirty());
        d.assert_derived_data_matches_recount();
        // Changing only the save encoding is a pending convert: dirty again.
        d.encoding = FileEncoding::default();
        assert!(d.dirty());
        d.assert_derived_data_matches_recount();
        d.encoding = FileEncoding::from_label("Windows-1252").unwrap();
        assert!(!d.dirty());
    }

    #[test]
    fn dirty_refresh_short_circuits_on_length_totals_then_bytes() {
        let mut d = doc("ab");
        // A different length is decided without reading any bytes.
        d.commit("abc".into());
        assert!(d.dirty());
        // Same length, chars, words and lines but different bytes ("ab" → "ac"):
        // only the final byte compare can tell, and it must say dirty.
        d.commit("ac".into());
        assert_eq!((d.char_count(), d.word_count(), d.line_count()), (2, 1, 1));
        assert!(d.dirty());
        // Same length but a different word count ("--" has none).
        d.commit("--".into());
        assert_eq!(d.word_count(), 0);
        assert!(d.dirty());
        // Same length, chars and words but a different line count ("a\n").
        d.commit("a\n".into());
        assert_eq!((d.char_count(), d.word_count(), d.line_count()), (2, 1, 2));
        assert!(d.dirty());
        // Back to the exact saved bytes: clean again.
        d.commit("ab".into());
        assert!(!d.dirty());
        d.assert_derived_data_matches_recount();
    }

    #[test]
    fn offset_at_and_line_col_of_match_the_pure_helpers_on_a_mixed_text() {
        let text = "ab\ncdé\nx\n"; // 'é' is two bytes; trailing newline → 4 lines
        let d = doc(text);
        assert_eq!(d.line_count(), 4);
        for off in 0..=text.len() + 2 {
            assert_eq!(
                d.line_col_of(off),
                find::line_col_of(text, off),
                "offset {off}"
            );
        }
        for line in 0..6 {
            assert_eq!(
                d.line_start(line),
                find::goto_line_offset(text, line + 1),
                "line {line}"
            );
            for col in 0..6 {
                assert_eq!(
                    d.offset_at(line, col),
                    find::offset_at(text, line, col),
                    "({line}, {col})"
                );
            }
        }
        // A column past the line's end clamps to its newline, not into the next line.
        assert_eq!(d.offset_at(0, 99), 2);
        // A mid-'é' column floors onto the glyph's start.
        assert_eq!(d.offset_at(1, 3), 5);
        // Saturating extremes land on end-of-text, never panic or overflow.
        assert_eq!(d.offset_at(usize::MAX, usize::MAX), text.len());
        assert_eq!(d.line_col_of(usize::MAX), (3, 0));
    }

    #[test]
    fn title_is_the_basename_or_untitled() {
        let mut d = doc("");
        d.path = Some(PathBuf::from("/deep/path/to/notes.txt"));
        assert_eq!(d.title(), "notes.txt");
        d.path = Some(PathBuf::from("dir/"));
        assert_eq!(d.title(), "Untitled", "an empty basename falls back");
        d.manual_lang = Some("Rust");
        assert_eq!(d.language(), "Rust");
    }

    proptest! {
        /// The indexed `(line, column)` conversions agree with the pure
        /// `find::*` helpers for any text and any arguments — in range, out of
        /// range, and mid-glyph — and never panic (#126).
        #[test]
        fn indexed_position_maths_agree_with_the_pure_helpers(
            text in seam_text(60),
            line in 0usize..80,
            column in 0usize..80,
            offset in 0usize..200,
        ) {
            let d = doc(&text);
            prop_assert_eq!(d.line_count(), find::line_count(&text));
            prop_assert_eq!(d.line_start(line), find::goto_line_offset(&text, line + 1));
            prop_assert_eq!(d.offset_at(line, column), find::offset_at(&text, line, column));
            prop_assert_eq!(d.line_col_of(offset), find::line_col_of(&text, offset));
            // Every line start round-trips through both conversions.
            for l in 0..d.line_count() {
                let start = d.line_start(l);
                prop_assert_eq!(d.line_col_of(start), (l, 0));
                prop_assert_eq!(d.offset_at(l, 0), start);
            }
            // Extreme arguments saturate rather than panic.
            prop_assert_eq!(d.offset_at(usize::MAX, usize::MAX), text.len());
            prop_assert_eq!(d.line_col_of(usize::MAX), find::line_col_of(&text, usize::MAX));
        }

        /// Random single-span edits applied directly through `commit`, with the
        /// cached totals, index and dirty flag checked against a from-scratch
        /// recount after every one, then undone and redone all the way.
        #[test]
        fn incremental_splices_match_a_full_recount(
            base in seam_text(40),
            edits in prop::collection::vec((0usize..60, 0usize..12, seam_text(10)), 0..12),
        ) {
            let mut d = doc(&base);
            for (pos, len, insert) in edits {
                let text = d.content();
                let lo = find::floor_boundary(text, pos);
                let hi = find::floor_boundary(text, lo + len);
                let mut next = String::with_capacity(text.len() + insert.len());
                next.push_str(&text[..lo]);
                next.push_str(&insert);
                next.push_str(&text[hi..]);
                d.commit(next);
                d.assert_derived_data_matches_recount();
            }
            while d.undo().is_some() {
                d.assert_derived_data_matches_recount();
            }
            prop_assert_eq!(d.content(), base.as_str());
            prop_assert!(!d.dirty());
            while d.redo().is_some() {
                d.assert_derived_data_matches_recount();
            }
        }
    }
}
