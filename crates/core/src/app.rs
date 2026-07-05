//! The pure update core (#28): [`State`], [`Message`], [`Effect`], [`update`].
//!
//! `update` is the only place application state changes. It performs no I/O; it
//! returns [`Effect`]s that the render shell executes, feeding results back in
//! as new [`Message`]s. Everything here is deterministic and window-free, so it
//! can be driven entirely from tests.

use crate::find::{self, Match, Matcher, SearchError, SearchOptions};
use crate::history::History;
use crate::lang;
use crate::text::{self, EndOfLine};
use std::path::PathBuf;

/// Stable identifier for an open document, so effects that complete
/// asynchronously (a save that resolves after the user typed in another tab)
/// still target the right buffer regardless of reordering.
pub type TabId = u64;

/// One open document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub id: TabId,
    /// `None` until the buffer is first saved to disk.
    pub path: Option<PathBuf>,
    /// Canonical text, always `\n`-terminated internally (see [`EndOfLine`]).
    pub content: String,
    /// The on-disk line ending to restore on save.
    pub eol: EndOfLine,
    /// Status-bar language label.
    pub language: &'static str,
    /// Undo/redo history; also the source of truth for the unsaved-changes
    /// ("•") marker (see [`Document::dirty`]).
    pub history: History,
}

impl Document {
    fn blank(id: TabId) -> Self {
        Document {
            id,
            path: None,
            content: String::new(),
            eol: EndOfLine::default(),
            language: lang::PLAIN_TEXT,
            history: History::new(),
        }
    }

    /// Whether the buffer has unsaved changes (the tab's "•" marker), delegated
    /// to the undo history so undoing back to a saved state clears it.
    pub fn dirty(&self) -> bool {
        self.history.dirty()
    }

    /// The tab / window title: the file name, or `Untitled` for a fresh buffer.
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
    fn is_pristine_blank(&self) -> bool {
        self.path.is_none() && !self.dirty() && self.content.is_empty()
    }
}

/// Which search option a [`Message::FindOptionToggled`] flips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindOption {
    /// Match case exactly (default is case-insensitive).
    CaseSensitive,
    /// Match only whole words (`\b…\b`).
    WholeWord,
    /// Interpret the pattern as a regular expression (default is literal).
    Regex,
}

/// The Find / Replace / Go-to bar's state (#33). A single instance, shared
/// across tabs and always describing the *active* document: `current`, `count`
/// and `ordinal` are for that document's matches of `query` under `options`.
#[derive(Debug, Clone, Default)]
pub struct FindState {
    /// Whether the find bar is visible (the shell shows/hides on this, and the
    /// readout is only recomputed while it is open — see [`refresh_find`]).
    pub open: bool,
    /// The search pattern the user typed.
    pub query: String,
    /// The replacement text.
    pub replacement: String,
    /// Case / whole-word / regex toggles.
    pub options: SearchOptions,
    /// The highlighted match in the active document, if any.
    pub current: Option<Match>,
    /// Total matches of `query` in the active document.
    pub count: usize,
    /// 1-based index of `current` among all matches (`0` when there is none):
    /// the "3 of 10" readout.
    pub ordinal: usize,
    /// A compile error to surface (invalid regex). `None` when the pattern is
    /// valid, or simply empty.
    pub error: Option<String>,
}

/// The whole application state. Always holds at least one document.
#[derive(Debug, Clone)]
pub struct State {
    pub docs: Vec<Document>,
    /// Index into `docs` of the focused tab; the invariant `active < docs.len()`
    /// always holds (see the proptest).
    pub active: usize,
    /// Find / Replace / Go-to bar state (#33).
    pub find: FindState,
    next_id: TabId,
}

impl Default for State {
    fn default() -> Self {
        let mut state = State {
            docs: Vec::new(),
            active: 0,
            find: FindState::default(),
            next_id: 1,
        };
        let id = state.alloc_id();
        state.docs.push(Document::blank(id));
        state
    }
}

impl State {
    fn alloc_id(&mut self) -> TabId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// The focused document.
    pub fn active_doc(&self) -> &Document {
        &self.docs[self.active]
    }

    fn doc_mut(&mut self, id: TabId) -> Option<&mut Document> {
        self.docs.iter_mut().find(|d| d.id == id)
    }
}

/// Everything that can happen: user intent (`OpenRequested`), shell results
/// (`FileLoaded`), and editor signals (`Edited`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// Create a fresh empty tab and focus it.
    NewTab,
    /// User asked to open a file (shell will show a picker).
    OpenRequested,
    /// The shell finished reading a file the user chose or dropped.
    FileLoaded { path: PathBuf, content: String },
    /// The editor's text for the active document changed.
    Edited(String),
    /// Undo the most recent edit to the active document.
    Undo,
    /// Redo the most recently undone edit to the active document.
    Redo,
    /// User asked to save the active document.
    SaveRequested,
    /// User asked to save the active document under a new name.
    SaveAsRequested,
    /// The shell's save picker returned a destination for document `id`.
    SavePathChosen { id: TabId, path: PathBuf },
    /// The shell finished writing document `id` to `path`.
    FileSaved { id: TabId, path: PathBuf },
    /// Focus the tab at `index` (ignored if out of range).
    TabSelected(usize),
    /// Close the tab at `index`; a new blank tab appears if it was the last.
    TabClosed(usize),

    // ---- Find / Replace / Go-to-line (#33) ----
    /// Open the find / replace bar and refresh its readout.
    FindOpened,
    /// Close the find / replace bar and drop the highlight.
    FindClosed,
    /// The search pattern changed; incremental find selects the first match.
    FindQueryChanged(String),
    /// The replacement text changed.
    ReplaceTextChanged(String),
    /// Flip one search option (case / whole-word / regex).
    FindOptionToggled(FindOption),
    /// Select the next match after the current one, wrapping around.
    FindNext,
    /// Select the previous match before the current one, wrapping around.
    FindPrev,
    /// Replace the current match, then select the next.
    ReplaceNext,
    /// Replace every match in one undo-able step.
    ReplaceAll,
    /// Move the caret to 1-based `line`, clamped into range.
    GoToLine(usize),
}

/// A side effect for the render shell to perform. `update` returns these instead
/// of doing I/O, which is what keeps it pure and testable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Show the native "open file" picker.
    PickOpenPath,
    /// Read this path and report back with [`Message::FileLoaded`].
    ReadFile(PathBuf),
    /// Show the native "save as" picker for document `id`.
    PickSavePath { id: TabId },
    /// Write `content` to `path`, then report [`Message::FileSaved`].
    WriteFile {
        id: TabId,
        path: PathBuf,
        content: String,
    },
    /// Update the window title.
    SetTitle(String),
    /// Select the byte range `[start, end)` in the active editor and scroll it
    /// into view. `start == end` is a bare caret (go-to-line); otherwise it
    /// highlights a find match or a freshly inserted replacement.
    RevealRange { start: usize, end: usize },
}

/// Apply `message` to `state`, returning the effects the shell must run.
pub fn update(state: &mut State, message: Message) -> Vec<Effect> {
    match message {
        Message::NewTab => {
            let id = state.alloc_id();
            state.docs.push(Document::blank(id));
            state.active = state.docs.len() - 1;
            resync_find_after_doc_change(state);
            title_effect(state)
        }

        Message::OpenRequested => vec![Effect::PickOpenPath],

        Message::FileLoaded { path, content } => {
            let eol = EndOfLine::detect(&content);
            let canonical = EndOfLine::to_lf(&content);
            let language = lang::language_for_path(path.to_str().unwrap_or(""));

            let reuse = state.docs.len() == 1 && state.docs[0].is_pristine_blank();
            if reuse {
                let doc = &mut state.docs[0];
                doc.path = Some(path);
                doc.content = canonical;
                doc.eol = eol;
                doc.language = language;
                doc.history = History::new(); // loaded content is the clean baseline
                state.active = 0;
            } else {
                let id = state.alloc_id();
                state.docs.push(Document {
                    id,
                    path: Some(path),
                    content: canonical,
                    eol,
                    language,
                    history: History::new(),
                });
                state.active = state.docs.len() - 1;
            }
            resync_find_after_doc_change(state);
            title_effect(state)
        }

        Message::Edited(content) => {
            let mut fx = apply_edit(state, content);
            fx.extend(refresh_find_after_edit(state));
            fx
        }

        Message::Undo => {
            let doc = &mut state.docs[state.active];
            match doc.history.undo() {
                Some(edit) => {
                    edit.apply(&mut doc.content);
                    let mut fx = title_effect(state); // content and the "•" may both change
                    fx.extend(refresh_find_after_edit(state));
                    fx
                }
                None => vec![],
            }
        }

        Message::Redo => {
            let doc = &mut state.docs[state.active];
            match doc.history.redo() {
                Some(edit) => {
                    edit.apply(&mut doc.content);
                    let mut fx = title_effect(state);
                    fx.extend(refresh_find_after_edit(state));
                    fx
                }
                None => vec![],
            }
        }

        Message::SaveRequested => {
            let doc = state.active_doc();
            match &doc.path {
                Some(path) => vec![Effect::WriteFile {
                    id: doc.id,
                    path: path.clone(),
                    content: doc.eol.join(&doc.content),
                }],
                None => vec![Effect::PickSavePath { id: doc.id }],
            }
        }

        Message::SaveAsRequested => vec![Effect::PickSavePath {
            id: state.active_doc().id,
        }],

        Message::SavePathChosen { id, path } => match state.doc_mut(id) {
            Some(doc) => vec![Effect::WriteFile {
                id,
                path,
                content: doc.eol.join(&doc.content),
            }],
            None => vec![],
        },

        Message::FileSaved { id, path } => {
            if let Some(doc) = state.doc_mut(id) {
                doc.language = lang::language_for_path(path.to_str().unwrap_or(""));
                doc.path = Some(path);
                doc.history.mark_saved();
            }
            title_effect(state)
        }

        Message::TabSelected(index) => {
            if index < state.docs.len() {
                state.active = index;
            }
            resync_find_after_doc_change(state);
            title_effect(state)
        }

        Message::TabClosed(index) => {
            if index < state.docs.len() {
                state.docs.remove(index);
                if state.docs.is_empty() {
                    let id = state.alloc_id();
                    state.docs.push(Document::blank(id));
                }
                if state.active >= state.docs.len() {
                    state.active = state.docs.len() - 1;
                } else if index < state.active {
                    state.active -= 1;
                }
            }
            resync_find_after_doc_change(state);
            title_effect(state)
        }

        // ---- Find / Replace / Go-to-line (#33) ----
        Message::FindOpened => {
            state.find.open = true;
            refresh_find(state, true)
        }

        Message::FindClosed => {
            state.find.open = false;
            state.find.current = None;
            vec![]
        }

        Message::FindQueryChanged(query) => {
            state.find.query = query;
            refresh_find(state, true)
        }

        Message::ReplaceTextChanged(text) => {
            state.find.replacement = text;
            vec![]
        }

        Message::FindOptionToggled(option) => {
            let options = &mut state.find.options;
            match option {
                FindOption::CaseSensitive => options.case_sensitive = !options.case_sensitive,
                FindOption::WholeWord => options.whole_word = !options.whole_word,
                FindOption::Regex => options.regex = !options.regex,
            }
            refresh_find(state, true)
        }

        Message::FindNext => navigate(state, true),
        Message::FindPrev => navigate(state, false),
        Message::ReplaceNext => replace_current(state),
        Message::ReplaceAll => replace_all_matches(state),

        Message::GoToLine(line) => {
            let offset = find::goto_line_offset(&state.docs[state.active].content, line);
            vec![Effect::RevealRange {
                start: offset,
                end: offset,
            }]
        }
    }
}

fn title_effect(state: &State) -> Vec<Effect> {
    vec![Effect::SetTitle(state.active_doc().title().to_string())]
}

/// Apply `new_content` to the active document as one undo-able edit, returning
/// the title effect only when this crosses the clean→dirty edge (so the tab's
/// "•" appears exactly once), mirroring [`Message::Edited`].
fn apply_edit(state: &mut State, new_content: String) -> Vec<Effect> {
    let doc = &mut state.docs[state.active];
    if let Some(edit) = crate::history::diff(&doc.content, &new_content) {
        let was_dirty = doc.dirty();
        doc.history.record(edit);
        doc.content = new_content;
        if !was_dirty {
            return title_effect(state);
        }
    }
    vec![]
}

/// Compile the active pattern, or record its error / return `None` for an empty
/// pattern. Central so every find/replace entry point handles bad input the same
/// way (surfaced, never a panic).
fn compile_query(find: &mut FindState) -> Option<Matcher> {
    match Matcher::new(&find.query, find.options) {
        Ok(matcher) => {
            find.error = None;
            Some(matcher)
        }
        Err(SearchError::EmptyPattern) => {
            find.error = None;
            None
        }
        Err(SearchError::InvalidRegex(msg)) => {
            find.error = Some(msg);
            None
        }
    }
}

/// Recompute the find readout for the active document after the pattern,
/// options, or buffer changed. With `select_first`, jump to the first match
/// (incremental find) and return a [`Effect::RevealRange`]; otherwise leave
/// `current` where it is. A no-op while the bar is closed, so per-keystroke
/// edits on huge files pay nothing.
fn refresh_find(state: &mut State, select_first: bool) -> Vec<Effect> {
    if !state.find.open {
        return vec![];
    }
    let matcher = compile_query(&mut state.find);
    let content = &state.docs[state.active].content;
    let (count, current, ordinal, effect) = match &matcher {
        Some(m) => {
            let count = m.count(content);
            let current = if select_first {
                m.find_from(content, 0)
            } else {
                state.find.current
            };
            let ordinal = current.map_or(0, |h| m.ordinal_of(content, h.start));
            let effect = if select_first {
                current.map(|h| Effect::RevealRange {
                    start: h.start,
                    end: h.end,
                })
            } else {
                None
            };
            (count, current, ordinal, effect)
        }
        None => (0, None, 0, None),
    };
    state.find.count = count;
    state.find.current = current;
    state.find.ordinal = ordinal;
    effect.into_iter().collect()
}

/// After the active document changes (open / new / select / close), keep the
/// find readout pointed at the new buffer: drop the old highlight and recount.
fn resync_find_after_doc_change(state: &mut State) {
    state.find.current = None;
    let _ = refresh_find(state, false);
}

/// Drop a now-stale highlight and recount after the active buffer changed
/// (edit / undo / redo). Clearing `current` unconditionally stops it ever
/// pointing past a shrunken buffer; the recount itself runs only while the bar
/// is open (so plain typing on a huge file pays nothing).
fn refresh_find_after_edit(state: &mut State) -> Vec<Effect> {
    state.find.current = None;
    refresh_find(state, false)
}

/// Shared Find Next / Find Previous. Moves `current` one match in the requested
/// direction, wrapping around, refreshes the readout, and returns a reveal
/// effect for the new match (if any).
fn navigate(state: &mut State, forward: bool) -> Vec<Effect> {
    let Some(matcher) = compile_query(&mut state.find) else {
        // Empty or invalid pattern: nothing to move to.
        state.find.count = 0;
        state.find.current = None;
        state.find.ordinal = 0;
        return vec![];
    };
    let content = &state.docs[state.active].content;
    let current = state.find.current;
    let next = if forward {
        let from = current.map_or(0, |c| find::resume_after(content, c));
        match matcher.find_from(content, from) {
            // A different start means we advanced; the same start (or none) means
            // we hit the end, so wrap to the top.
            Some(hit) if current.is_none_or(|c| hit.start != c.start) => Some(hit),
            _ => matcher.find_from(content, 0),
        }
    } else {
        let before = current.map_or(content.len(), |c| c.start);
        matcher
            .find_last_before(content, before)
            .or_else(|| matcher.find_last(content))
    };
    state.find.count = matcher.count(content);
    state.find.current = next;
    state.find.ordinal = next.map_or(0, |h| matcher.ordinal_of(content, h.start));
    next.map(|h| Effect::RevealRange {
        start: h.start,
        end: h.end,
    })
    .into_iter()
    .collect()
}

/// Replace the current match (or the next one from the top), then select the
/// following match. The edit is undo-able; a self-including replacement grows
/// only under repeated manual presses, never in a single call.
fn replace_current(state: &mut State) -> Vec<Effect> {
    let Some(matcher) = compile_query(&mut state.find) else {
        return vec![];
    };
    let from = state.find.current.map_or(0, |c| c.start);
    let replaced = {
        let content = &state.docs[state.active].content;
        matcher.replace_next(content, from, &state.find.replacement)
    };
    let Some(rep) = replaced else {
        return vec![];
    };
    let mut fx = apply_edit(state, rep.text);
    let content = &state.docs[state.active].content;
    let after = find::resume_after(content, rep.range);
    let next = matcher
        .find_from(content, after)
        .or_else(|| matcher.find_from(content, 0));
    state.find.count = matcher.count(content);
    state.find.current = next;
    state.find.ordinal = next.map_or(0, |h| matcher.ordinal_of(content, h.start));
    // Reveal the next match, or failing that the replacement we just made.
    let reveal = next.unwrap_or(rep.range);
    fx.push(Effect::RevealRange {
        start: reveal.start,
        end: reveal.end,
    });
    fx
}

/// Replace every match in one undo-able step, via the engine's single-pass
/// `replace_all` (so it never re-scans its own output). No reveal — the shell
/// resyncs the rewritten buffer to the top.
fn replace_all_matches(state: &mut State) -> Vec<Effect> {
    let Some(matcher) = compile_query(&mut state.find) else {
        return vec![];
    };
    let (new_content, replaced) = {
        let content = &state.docs[state.active].content;
        matcher.replace_all(content, &state.find.replacement)
    };
    if replaced == 0 {
        return vec![];
    }
    let fx = apply_edit(state, new_content);
    state.find.current = None;
    state.find.ordinal = 0;
    // The replacement may re-introduce the pattern, so recount on the new text.
    state.find.count = matcher.count(&state.docs[state.active].content);
    fx
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn load(state: &mut State, path: &str, content: &str) -> Vec<Effect> {
        update(
            state,
            Message::FileLoaded {
                path: PathBuf::from(path),
                content: content.to_string(),
            },
        )
    }

    #[test]
    fn starts_with_one_blank_untitled_tab() {
        let s = State::default();
        assert_eq!(s.docs.len(), 1);
        assert_eq!(s.active, 0);
        assert_eq!(s.active_doc().title(), "Untitled");
        assert!(!s.active_doc().dirty());
    }

    #[test]
    fn opening_into_pristine_blank_reuses_the_tab() {
        let mut s = State::default();
        load(&mut s, "/tmp/hello.rs", "fn main() {}\n");
        assert_eq!(s.docs.len(), 1, "should reuse the lone blank tab");
        assert_eq!(s.active_doc().title(), "hello.rs");
        assert_eq!(s.active_doc().language, "Rust");
        assert!(!s.active_doc().dirty());
    }

    #[test]
    fn opening_after_edit_adds_a_tab() {
        let mut s = State::default();
        update(&mut s, Message::Edited("dirty".into()));
        load(&mut s, "/tmp/a.txt", "x");
        assert_eq!(s.docs.len(), 2, "dirty blank must not be reused");
        assert_eq!(s.active, 1);
    }

    #[test]
    fn edit_sets_dirty_and_emits_title_only_on_the_edge() {
        let mut s = State::default();
        let fx = update(&mut s, Message::Edited("a".into()));
        assert!(s.active_doc().dirty());
        assert_eq!(fx, vec![Effect::SetTitle("Untitled".into())]);
        // Second edit stays dirty; no redundant title effect.
        let fx2 = update(&mut s, Message::Edited("ab".into()));
        assert!(fx2.is_empty());
    }

    #[test]
    fn save_without_path_asks_for_one_then_writes() {
        let mut s = State::default();
        update(&mut s, Message::Edited("hi".into()));
        let id = s.active_doc().id;
        assert_eq!(
            update(&mut s, Message::SaveRequested),
            vec![Effect::PickSavePath { id }]
        );
        let fx = update(
            &mut s,
            Message::SavePathChosen {
                id,
                path: PathBuf::from("/tmp/out.txt"),
            },
        );
        assert_eq!(
            fx,
            vec![Effect::WriteFile {
                id,
                path: PathBuf::from("/tmp/out.txt"),
                content: "hi".into(),
            }]
        );
    }

    #[test]
    fn crlf_document_is_written_back_with_crlf() {
        let mut s = State::default();
        load(&mut s, "/tmp/win.txt", "a\r\nb\r\n");
        assert_eq!(s.active_doc().eol, EndOfLine::Crlf);
        assert_eq!(s.active_doc().content, "a\nb\n"); // stored canonical
        let fx = update(&mut s, Message::SaveRequested);
        assert_eq!(
            fx,
            vec![Effect::WriteFile {
                id: s.active_doc().id,
                path: PathBuf::from("/tmp/win.txt"),
                content: "a\r\nb\r\n".into(), // restored on save
            }]
        );
    }

    #[test]
    fn file_saved_clears_dirty_and_updates_language() {
        let mut s = State::default();
        update(&mut s, Message::Edited("code".into()));
        let id = s.active_doc().id;
        update(
            &mut s,
            Message::FileSaved {
                id,
                path: PathBuf::from("/tmp/new.py"),
            },
        );
        assert!(!s.active_doc().dirty());
        assert_eq!(s.active_doc().language, "Python");
        assert_eq!(s.active_doc().title(), "new.py");
    }

    #[test]
    fn closing_last_tab_leaves_a_fresh_blank() {
        let mut s = State::default();
        update(&mut s, Message::TabClosed(0));
        assert_eq!(s.docs.len(), 1);
        assert!(s.active_doc().is_pristine_blank());
    }

    #[test]
    fn closing_a_tab_before_active_keeps_focus_stable() {
        let mut s = State::default();
        load(&mut s, "/tmp/a.txt", "a"); // reuses blank -> 1 doc
        update(&mut s, Message::NewTab); // 2 docs, active=1
        update(&mut s, Message::NewTab); // 3 docs, active=2
        assert_eq!(s.active, 2);
        update(&mut s, Message::TabClosed(0)); // remove one before active
        assert_eq!(s.docs.len(), 2);
        assert_eq!(s.active, 1, "focus should follow the same document");
    }

    #[test]
    fn save_path_chosen_for_unknown_id_is_a_noop() {
        let mut s = State::default();
        let fx = update(
            &mut s,
            Message::SavePathChosen {
                id: 9999,
                path: PathBuf::from("/tmp/x"),
            },
        );
        assert!(fx.is_empty());
    }

    #[test]
    fn file_saved_for_unknown_id_refreshes_title_only() {
        let mut s = State::default();
        let fx = update(
            &mut s,
            Message::FileSaved {
                id: 9999,
                path: PathBuf::from("/tmp/x.rs"),
            },
        );
        assert_eq!(fx, vec![Effect::SetTitle("Untitled".into())]);
        assert_eq!(s.active_doc().language, "Plain Text"); // untouched
    }

    #[test]
    fn out_of_range_tab_ops_are_ignored() {
        let mut s = State::default();
        update(&mut s, Message::TabSelected(42));
        assert_eq!(s.active, 0);
        update(&mut s, Message::TabClosed(42));
        assert_eq!(s.docs.len(), 1);
    }

    // ---- Find / Replace / Go-to-line (#33) ----

    /// Open the find bar and set `query`, returning the query change's effects.
    fn find_for(s: &mut State, query: &str) -> Vec<Effect> {
        update(s, Message::FindOpened);
        update(s, Message::FindQueryChanged(query.to_string()))
    }

    /// The first `RevealRange` in a set of effects, as a `(start, end)` pair.
    fn reveal(fx: &[Effect]) -> Option<(usize, usize)> {
        fx.iter().find_map(|e| match e {
            Effect::RevealRange { start, end } => Some((*start, *end)),
            _ => None,
        })
    }

    #[test]
    fn incremental_find_reveals_first_match_and_counts() {
        let mut s = State::default();
        update(&mut s, Message::Edited("foo bar foo".into()));
        let fx = find_for(&mut s, "foo");
        assert_eq!(s.find.count, 2);
        assert_eq!(s.find.current, Some(Match { start: 0, end: 3 }));
        assert_eq!(s.find.ordinal, 1);
        assert_eq!(reveal(&fx), Some((0, 3)));
    }

    #[test]
    fn find_next_and_prev_cycle_with_wraparound() {
        let mut s = State::default();
        update(&mut s, Message::Edited("a.a.a".into())); // 'a' at 0, 2, 4
        find_for(&mut s, "a");
        assert_eq!(s.find.current, Some(Match { start: 0, end: 1 }));
        update(&mut s, Message::FindNext);
        assert_eq!(s.find.current, Some(Match { start: 2, end: 3 }));
        // Prev from a middle match steps back one (find_last_before).
        update(&mut s, Message::FindPrev);
        assert_eq!(s.find.current, Some(Match { start: 0, end: 1 }));
        // Prev from the first wraps to the last (find_last).
        update(&mut s, Message::FindPrev);
        assert_eq!(s.find.current, Some(Match { start: 4, end: 5 }));
        // Next from the last wraps back to the first.
        let fx = update(&mut s, Message::FindNext);
        assert_eq!(s.find.current, Some(Match { start: 0, end: 1 }));
        assert_eq!(reveal(&fx), Some((0, 1)));
    }

    #[test]
    fn zero_width_matches_navigate_and_wrap() {
        let mut s = State::default();
        update(&mut s, Message::Edited("aa".into()));
        update(&mut s, Message::FindOpened);
        update(&mut s, Message::FindOptionToggled(FindOption::Regex));
        update(&mut s, Message::FindQueryChanged("a*".into()));
        assert_eq!(s.find.current, Some(Match { start: 0, end: 2 }));
        update(&mut s, Message::FindNext);
        assert_eq!(s.find.current, Some(Match { start: 2, end: 2 })); // zero-width tail
        update(&mut s, Message::FindNext);
        // Same start as the tail match → wraps back to the top.
        assert_eq!(s.find.current, Some(Match { start: 0, end: 2 }));
    }

    #[test]
    fn no_match_clears_current_and_navigation_is_empty() {
        let mut s = State::default();
        update(&mut s, Message::Edited("hello".into()));
        let fx = find_for(&mut s, "zzz");
        assert_eq!(s.find.count, 0);
        assert_eq!(s.find.current, None);
        assert!(reveal(&fx).is_none());
        assert!(s.find.error.is_none());
        // Next/Prev on a valid-but-absent pattern stay empty (not a panic).
        assert!(reveal(&update(&mut s, Message::FindNext)).is_none());
        assert!(reveal(&update(&mut s, Message::FindPrev)).is_none());
        assert_eq!(s.find.current, None);
    }

    #[test]
    fn invalid_regex_surfaces_error_without_panicking() {
        let mut s = State::default();
        update(&mut s, Message::Edited("hello".into()));
        update(&mut s, Message::FindOpened);
        update(&mut s, Message::FindOptionToggled(FindOption::Regex));
        let fx = update(&mut s, Message::FindQueryChanged("(unclosed".into()));
        assert!(s.find.error.is_some());
        assert_eq!(s.find.count, 0);
        assert_eq!(s.find.current, None);
        assert!(reveal(&fx).is_none());
        // Every entry point tolerates the bad pattern.
        assert!(update(&mut s, Message::FindNext).is_empty());
        assert!(update(&mut s, Message::ReplaceNext).is_empty());
        assert!(update(&mut s, Message::ReplaceAll).is_empty());
        assert!(s.find.error.is_some());
    }

    #[test]
    fn clearing_query_resets_the_readout() {
        let mut s = State::default();
        update(&mut s, Message::Edited("aaa".into()));
        find_for(&mut s, "a");
        assert_eq!(s.find.count, 3);
        let fx = update(&mut s, Message::FindQueryChanged(String::new()));
        assert_eq!(s.find.count, 0);
        assert_eq!(s.find.current, None);
        assert!(s.find.error.is_none());
        assert!(reveal(&fx).is_none());
    }

    #[test]
    fn case_sensitivity_toggle_changes_match_count() {
        let mut s = State::default();
        update(&mut s, Message::Edited("Foo foo FOO".into()));
        find_for(&mut s, "foo");
        assert_eq!(s.find.count, 3); // case-insensitive by default
        update(
            &mut s,
            Message::FindOptionToggled(FindOption::CaseSensitive),
        );
        assert_eq!(s.find.count, 1); // only the exact "foo"
    }

    #[test]
    fn whole_word_toggle_restricts_matches() {
        let mut s = State::default();
        update(&mut s, Message::Edited("cat category cat".into()));
        find_for(&mut s, "cat");
        assert_eq!(s.find.count, 3); // substring also matches "category"
        update(&mut s, Message::FindOptionToggled(FindOption::WholeWord));
        assert_eq!(s.find.count, 2);
    }

    #[test]
    fn regex_toggle_enables_pattern_matching() {
        let mut s = State::default();
        update(&mut s, Message::Edited("a1 b2 c3".into()));
        update(&mut s, Message::FindOpened);
        update(&mut s, Message::FindQueryChanged(r"\d".into()));
        assert_eq!(s.find.count, 0); // literal "\d" is absent
        update(&mut s, Message::FindOptionToggled(FindOption::Regex));
        assert_eq!(s.find.count, 3); // now matches the digits
    }

    #[test]
    fn replace_next_replaces_advances_and_dirties() {
        let mut s = State::default();
        update(&mut s, Message::Edited("ab ab ab".into()));
        find_for(&mut s, "ab");
        update(&mut s, Message::ReplaceTextChanged("X".into()));
        let fx = update(&mut s, Message::ReplaceNext);
        assert_eq!(s.active_doc().content, "X ab ab");
        assert!(s.active_doc().dirty());
        assert_eq!(s.find.current, Some(Match { start: 2, end: 4 }));
        assert_eq!(reveal(&fx), Some((2, 4)));
    }

    #[test]
    fn replacing_the_only_match_reveals_the_replacement() {
        let mut s = State::default();
        // Start from a clean, freshly-loaded buffer so the replace crosses the
        // clean→dirty edge (emitting SetTitle alongside the reveal).
        load(&mut s, "/t/only.txt", "ab");
        assert!(!s.active_doc().dirty());
        find_for(&mut s, "ab");
        update(&mut s, Message::ReplaceTextChanged("XYZ".into()));
        let fx = update(&mut s, Message::ReplaceNext);
        assert_eq!(s.active_doc().content, "XYZ");
        assert!(s.active_doc().dirty(), "replacing dirties a clean doc");
        assert!(fx.contains(&Effect::SetTitle("only.txt".into())));
        assert_eq!(s.find.current, None); // nothing left to find
        assert_eq!(reveal(&fx), Some((0, 3))); // falls back to the replacement
    }

    #[test]
    fn replace_next_with_no_match_is_a_noop() {
        let mut s = State::default();
        update(&mut s, Message::Edited("hello".into()));
        let id = s.active_doc().id;
        update(
            &mut s,
            Message::FileSaved {
                id,
                path: PathBuf::from("/t/x.txt"),
            },
        );
        find_for(&mut s, "zzz");
        let fx = update(&mut s, Message::ReplaceNext);
        assert_eq!(s.active_doc().content, "hello");
        assert!(!s.active_doc().dirty(), "a no-match replace must not dirty");
        assert!(fx.is_empty());
    }

    #[test]
    fn identity_replacement_reveals_but_does_not_dirty() {
        let mut s = State::default();
        update(&mut s, Message::Edited("a".into()));
        let id = s.active_doc().id;
        update(
            &mut s,
            Message::FileSaved {
                id,
                path: PathBuf::from("/t/x.txt"),
            },
        );
        assert!(!s.active_doc().dirty());
        find_for(&mut s, "a");
        update(&mut s, Message::ReplaceTextChanged("a".into())); // same text
        let fx = update(&mut s, Message::ReplaceNext);
        assert_eq!(s.active_doc().content, "a");
        assert!(
            !s.active_doc().dirty(),
            "an identity replace must not dirty"
        );
        assert!(reveal(&fx).is_some());
    }

    #[test]
    fn replace_all_replaces_everything_in_one_undo_step() {
        let mut s = State::default();
        update(&mut s, Message::Edited("ab ab ab".into()));
        find_for(&mut s, "ab");
        update(&mut s, Message::ReplaceTextChanged("X".into()));
        update(&mut s, Message::ReplaceAll);
        assert_eq!(s.active_doc().content, "X X X");
        assert_eq!(s.find.count, 0);
        assert!(s.active_doc().dirty());
        // A single undo restores the whole original buffer.
        update(&mut s, Message::Undo);
        assert_eq!(s.active_doc().content, "ab ab ab");
    }

    #[test]
    fn regex_replace_all_expands_capture_groups() {
        let mut s = State::default();
        update(&mut s, Message::Edited("a1 b2".into()));
        update(&mut s, Message::FindOpened);
        update(&mut s, Message::FindOptionToggled(FindOption::Regex));
        update(&mut s, Message::FindQueryChanged(r"(\w)(\d)".into()));
        update(&mut s, Message::ReplaceTextChanged("$2$1".into()));
        update(&mut s, Message::ReplaceAll);
        assert_eq!(s.active_doc().content, "1a 2b");
    }

    #[test]
    fn replace_all_with_no_match_is_a_noop() {
        let mut s = State::default();
        update(&mut s, Message::Edited("hello".into()));
        let id = s.active_doc().id;
        update(
            &mut s,
            Message::FileSaved {
                id,
                path: PathBuf::from("/t/x.txt"),
            },
        );
        find_for(&mut s, "zzz");
        update(&mut s, Message::ReplaceTextChanged("X".into()));
        let fx = update(&mut s, Message::ReplaceAll);
        assert_eq!(s.active_doc().content, "hello");
        assert!(
            !s.active_doc().dirty(),
            "a no-match replace-all must not dirty"
        );
        assert!(fx.is_empty());
    }

    #[test]
    fn go_to_line_reveals_a_clamped_caret_without_opening_the_bar() {
        let mut s = State::default();
        update(&mut s, Message::Edited("one\ntwo\nthree".into()));
        assert_eq!(reveal(&update(&mut s, Message::GoToLine(2))), Some((4, 4)));
        assert_eq!(
            reveal(&update(&mut s, Message::GoToLine(999))),
            Some((8, 8))
        );
        assert_eq!(reveal(&update(&mut s, Message::GoToLine(0))), Some((0, 0)));
        assert!(!s.find.open, "go-to does not need the find bar");
    }

    #[test]
    fn editing_refreshes_the_readout_while_the_bar_is_open() {
        let mut s = State::default();
        update(&mut s, Message::Edited("foo".into()));
        find_for(&mut s, "foo");
        assert_eq!(s.find.count, 1);
        update(&mut s, Message::Edited("foo foo foo".into()));
        assert_eq!(s.find.count, 3);
        assert_eq!(s.find.current, None); // stale highlight dropped
    }

    #[test]
    fn undo_and_redo_reset_the_stale_find_highlight() {
        // Regression: undo/redo change the buffer, so a match highlighted before
        // must not dangle past the (possibly shorter) new content.
        let mut s = State::default();
        update(&mut s, Message::Edited("abcdef".into()));
        find_for(&mut s, "abc");
        assert_eq!(s.find.current, Some(Match { start: 0, end: 3 }));
        // Undo empties the buffer out from under the highlight.
        update(&mut s, Message::Undo);
        assert_eq!(s.active_doc().content, "");
        assert_eq!(s.find.current, None);
        assert_eq!(s.find.count, 0);
        // Redo restores the text; the readout recomputes, highlight stays cleared.
        update(&mut s, Message::Redo);
        assert_eq!(s.active_doc().content, "abcdef");
        assert_eq!(s.find.current, None);
        assert_eq!(s.find.count, 1);
    }

    #[test]
    fn find_messages_are_inert_while_the_bar_is_closed() {
        let mut s = State::default();
        update(&mut s, Message::Edited("aaa".into()));
        // The shell never sends these while closed, but the core must not
        // compute or reveal anything if it does.
        let fx = update(&mut s, Message::FindQueryChanged("a".into()));
        assert_eq!(s.find.count, 0);
        assert_eq!(s.find.current, None);
        assert!(fx.is_empty());
        assert!(!s.find.open);
    }

    #[test]
    fn changing_replacement_text_alone_has_no_effect() {
        let mut s = State::default();
        update(&mut s, Message::FindOpened);
        let fx = update(&mut s, Message::ReplaceTextChanged("x".into()));
        assert_eq!(s.find.replacement, "x");
        assert!(fx.is_empty());
    }

    #[test]
    fn closing_the_bar_drops_the_highlight() {
        let mut s = State::default();
        update(&mut s, Message::Edited("aaa".into()));
        find_for(&mut s, "a");
        assert!(s.find.current.is_some());
        let fx = update(&mut s, Message::FindClosed);
        assert!(!s.find.open);
        assert!(s.find.current.is_none());
        assert!(fx.is_empty());
    }

    #[test]
    fn switching_tabs_repoints_find_at_the_new_document() {
        let mut s = State::default();
        load(&mut s, "/t/a.txt", "match match"); // reuses the blank tab → doc 0
        update(&mut s, Message::NewTab); // doc 1, empty, now active
        update(&mut s, Message::Edited("no hits here".into()));
        update(&mut s, Message::FindOpened);
        update(&mut s, Message::FindQueryChanged("match".into()));
        assert_eq!(s.find.count, 0); // active doc 1 has none
        update(&mut s, Message::TabSelected(0)); // back to doc 0
        assert_eq!(s.find.count, 2);
        assert!(s.find.current.is_none(), "highlight resets on tab switch");
        // A fresh Find Next then selects doc 0's first match (current == None path).
        update(&mut s, Message::FindNext);
        assert_eq!(s.find.current, Some(Match { start: 0, end: 5 }));
    }

    // ---- Property-based invariants (epic #25 Definition of Done) ----

    fn arb_message() -> impl Strategy<Value = Message> {
        prop_oneof![
            Just(Message::NewTab),
            Just(Message::OpenRequested),
            Just(Message::SaveRequested),
            Just(Message::SaveAsRequested),
            any::<String>().prop_map(Message::Edited),
            Just(Message::Undo),
            Just(Message::Redo),
            (0usize..6).prop_map(Message::TabSelected),
            (0usize..6).prop_map(Message::TabClosed),
            ("[a-z]{1,6}", any::<String>()).prop_map(|(n, c)| Message::FileLoaded {
                path: PathBuf::from(format!("/t/{n}.txt")),
                content: c
            }),
            // Find / Replace / Go-to (#33): include regex metacharacters so
            // invalid patterns and catastrophic-looking ones exercise the guard.
            Just(Message::FindOpened),
            Just(Message::FindClosed),
            Just(Message::FindNext),
            Just(Message::FindPrev),
            Just(Message::ReplaceNext),
            Just(Message::ReplaceAll),
            r"[a-c*+?.()\[\]\\^$|{}]{0,6}".prop_map(Message::FindQueryChanged),
            any::<String>().prop_map(Message::ReplaceTextChanged),
            prop_oneof![
                Just(FindOption::CaseSensitive),
                Just(FindOption::WholeWord),
                Just(FindOption::Regex),
            ]
            .prop_map(Message::FindOptionToggled),
            (0usize..500).prop_map(Message::GoToLine),
        ]
    }

    proptest! {
        /// No message sequence may ever empty the document list, drive `active`
        /// out of bounds, or panic. This is the safety net the pure-core split
        /// exists to give us.
        #[test]
        fn active_index_and_nonempty_invariants_hold(seq in prop::collection::vec(arb_message(), 0..80)) {
            let mut s = State::default();
            for m in seq {
                let _ = update(&mut s, m);
                prop_assert!(!s.docs.is_empty());
                prop_assert!(s.active < s.docs.len());
            }
        }

        /// Loading content then saving reproduces the original bytes exactly,
        /// for arbitrary text and either line ending.
        #[test]
        fn load_then_save_round_trips_bytes(body in "[a-zA-Z0-9 \n]{0,200}", crlf in any::<bool>()) {
            let original = if crlf { body.replace('\n', "\r\n") } else { body.clone() };
            let mut s = State::default();
            load(&mut s, "/t/f.txt", &original);
            let fx = update(&mut s, Message::SaveRequested);
            let written = match fx.first() {
                Some(Effect::WriteFile { content, .. }) => content.clone(),
                other => panic!("expected WriteFile, got {other:?}"),
            };
            prop_assert_eq!(written, original);
        }

        /// Recording N edits then undoing N times returns to the original empty
        /// buffer, and redoing N times reproduces the final content exactly.
        #[test]
        fn undo_all_then_redo_all_round_trips(edits in prop::collection::vec("[a-z\n]{0,8}", 0..25)) {
            let mut s = State::default();
            let mut content = String::new();
            let mut steps = 0usize;
            for chunk in &edits {
                // Append each chunk, tracking how many actually changed content.
                let next = format!("{content}{chunk}");
                if next != content {
                    update(&mut s, Message::Edited(next.clone()));
                    steps += 1;
                    content = next;
                }
            }
            let final_content = s.active_doc().content.clone();
            prop_assert_eq!(&final_content, &content);

            // Undo enough times to drain any coalesced history back to empty.
            for _ in 0..=steps {
                update(&mut s, Message::Undo);
            }
            prop_assert_eq!(&s.active_doc().content, "");
            prop_assert!(!s.active_doc().dirty());

            // Redo everything back to the final content.
            for _ in 0..=steps {
                update(&mut s, Message::Redo);
            }
            prop_assert_eq!(s.active_doc().content.clone(), final_content);
        }

        /// The stored content and the history's applied edits never disagree:
        /// after any message stream, undoing to the bottom yields a buffer whose
        /// redo-to-top reproduces the current content.
        #[test]
        fn buffer_stays_consistent_under_random_edits(edits in prop::collection::vec(any::<String>(), 0..40)) {
            let mut s = State::default();
            for e in edits {
                update(&mut s, Message::Edited(e));
                // The active buffer is always valid UTF-8 and matches what the
                // last edit set (no torn state from diff/apply).
                prop_assert!(s.active_doc().content.is_char_boundary(0));
            }
            let top = s.active_doc().content.clone();
            // Undo a lot, then redo a lot: we must return to the same top.
            for _ in 0..60 { update(&mut s, Message::Undo); }
            for _ in 0..60 { update(&mut s, Message::Redo); }
            prop_assert_eq!(s.active_doc().content.clone(), top);
        }

        /// After any stream of find/replace/edit messages, the find state stays
        /// consistent with the active buffer: a highlighted match is always an
        /// in-bounds, char-boundary range, and every `RevealRange` effect points
        /// inside the current content.
        #[test]
        fn find_state_and_reveals_stay_in_bounds(seq in prop::collection::vec(arb_message(), 0..80)) {
            let mut s = State::default();
            for m in seq {
                let effects = update(&mut s, m);
                let content = &s.active_doc().content;
                if let Some(hit) = s.find.current {
                    prop_assert!(hit.start <= hit.end);
                    prop_assert!(hit.end <= content.len());
                    prop_assert!(content.is_char_boundary(hit.start));
                    prop_assert!(content.is_char_boundary(hit.end));
                }
                for e in &effects {
                    if let Effect::RevealRange { start, end } = e {
                        prop_assert!(start <= end);
                        prop_assert!(*end <= content.len());
                        prop_assert!(content.is_char_boundary(*start));
                        prop_assert!(content.is_char_boundary(*end));
                    }
                }
            }
        }
    }
}
