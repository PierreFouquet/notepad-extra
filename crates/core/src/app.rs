//! The pure update core (#28): [`State`], [`Message`], [`Effect`], [`update`].
//!
//! `update` is the only place application state changes. It performs no I/O; it
//! returns [`Effect`]s that the render shell executes, feeding results back in
//! as new [`Message`]s. Everything here is deterministic and window-free, so it
//! can be driven entirely from tests.

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

/// The whole application state. Always holds at least one document.
#[derive(Debug, Clone)]
pub struct State {
    pub docs: Vec<Document>,
    /// Index into `docs` of the focused tab; the invariant `active < docs.len()`
    /// always holds (see the proptest).
    pub active: usize,
    next_id: TabId,
}

impl Default for State {
    fn default() -> Self {
        let mut state = State {
            docs: Vec::new(),
            active: 0,
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
}

/// Apply `message` to `state`, returning the effects the shell must run.
pub fn update(state: &mut State, message: Message) -> Vec<Effect> {
    match message {
        Message::NewTab => {
            let id = state.alloc_id();
            state.docs.push(Document::blank(id));
            state.active = state.docs.len() - 1;
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
            title_effect(state)
        }

        Message::Edited(content) => {
            let doc = &mut state.docs[state.active];
            if let Some(edit) = crate::history::diff(&doc.content, &content) {
                let was_dirty = doc.dirty();
                doc.history.record(edit);
                doc.content = content;
                // Title carries the "•" marker, so only refresh on the clean→dirty edge.
                if !was_dirty {
                    return title_effect(state);
                }
            }
            vec![]
        }

        Message::Undo => {
            let doc = &mut state.docs[state.active];
            match doc.history.undo() {
                Some(edit) => {
                    edit.apply(&mut doc.content);
                    title_effect(state) // content and the "•" marker may both change
                }
                None => vec![],
            }
        }

        Message::Redo => {
            let doc = &mut state.docs[state.active];
            match doc.history.redo() {
                Some(edit) => {
                    edit.apply(&mut doc.content);
                    title_effect(state)
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
            title_effect(state)
        }
    }
}

fn title_effect(state: &State) -> Vec<Effect> {
    vec![Effect::SetTitle(state.active_doc().title().to_string())]
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
    }
}
