//! Adversarial / stress tests for the update core (epic #25 Definition of Done).
//!
//! These assert the core stays correct and panic-free under abuse: hundreds of
//! opens, million-line pastes, rapid tab churn, and — for find/replace (#33) —
//! catastrophic regexes, huge-document replace-all, and thousands of Find Next.

use notepad_core::find::{self, goto_line_offset};
use notepad_core::{Effect, Matcher, Message, SearchOptions, State, status, update};
use std::path::PathBuf;

/// A regex-mode [`SearchOptions`].
fn regex_opts() -> SearchOptions {
    SearchOptions {
        regex: true,
        ..SearchOptions::default()
    }
}

#[test]
fn hundreds_of_open_operations_stay_bounded() {
    let mut s = State::default();
    for i in 0..500 {
        update(
            &mut s,
            Message::FileLoaded {
                path: PathBuf::from(format!("/tmp/file{i}.txt")),
                content: format!("contents of file {i}\n"),
            },
        );
        assert!(!s.docs.is_empty());
        assert!(s.active < s.docs.len());
    }
    // The first open reused the pristine blank tab; the other 499 added tabs.
    assert_eq!(s.docs.len(), 500);
    assert_eq!(s.active, 499);
}

#[test]
fn million_line_paste_round_trips() {
    let mut s = State::default();
    let huge: String = "x\n".repeat(1_000_000); // ~2 MB, 1M newlines
    update(&mut s, Message::Edited(huge.clone()));
    assert!(s.active_doc().dirty());

    let id = s.active_doc().id;
    let fx = update(
        &mut s,
        Message::SavePathChosen {
            id,
            path: PathBuf::from("/tmp/huge.txt"),
        },
    );
    match fx.first() {
        Some(Effect::WriteFile { content, .. }) => {
            assert_eq!(content.len(), huge.len());
            assert_eq!(content.matches('\n').count(), 1_000_000);
        }
        other => panic!("expected WriteFile, got {other:?}"),
    }
}

#[test]
fn million_line_paste_is_a_single_undo_step() {
    // The diff-based history stores the whole paste as one Edit, so a single
    // undo must wipe it and a single redo must restore it — no per-line churn.
    let mut s = State::default();
    let huge: String = "x\n".repeat(1_000_000);
    update(&mut s, Message::Edited(huge.clone()));
    assert_eq!(s.active_doc().content.len(), huge.len());

    update(&mut s, Message::Undo);
    assert_eq!(
        s.active_doc().content,
        "",
        "one undo removes the whole paste"
    );
    assert!(!s.active_doc().dirty());

    update(&mut s, Message::Redo);
    assert_eq!(s.active_doc().content, huge, "one redo restores it");
    assert!(s.active_doc().dirty());
}

#[test]
fn huge_single_line_edit_then_undo() {
    // A multi-hundred-MB one-liner: edit inside it, then undo, panic-free. The
    // in-place replacement is its own history step (not coalesced), so we get two.
    let mut s = State::default();
    let base: String = "a".repeat(5_000_000);
    update(&mut s, Message::Edited(base.clone()));
    let mut edited = base.clone();
    edited.replace_range(0..1, "B"); // replace the first char — a replacement, not an insert
    update(&mut s, Message::Edited(edited.clone()));
    assert_eq!(s.active_doc().content, edited);

    update(&mut s, Message::Undo);
    assert_eq!(s.active_doc().content, base);
    update(&mut s, Message::Undo);
    assert_eq!(s.active_doc().content, "");
}

#[test]
fn edit_storm_then_full_undo_walks_back_consistently() {
    // Thousands of newline-terminated appends (each its own history step), then
    // undo every step: the buffer must stay internally consistent and the depth
    // cap must keep the history bounded without ever panicking.
    let mut s = State::default();
    let mut content = String::new();
    for i in 0..3000 {
        content = format!("{content}line{i}\n");
        update(&mut s, Message::Edited(content.clone()));
        assert_eq!(s.active_doc().content, content);
    }
    // Undo far more times than the (capped) history depth; each step must leave a
    // valid buffer and never panic.
    for _ in 0..4000 {
        update(&mut s, Message::Undo);
        assert!(!s.docs.is_empty());
    }
    // The oldest edits were capped away, so undo can't reach the empty buffer;
    // what matters is it stayed panic-free and consistent throughout.
    let remaining = s.active_doc().content.clone();
    update(&mut s, Message::Redo);
    assert!(s.active_doc().content.starts_with(&remaining));
}

#[test]
fn rapid_tab_churn_never_underflows() {
    let mut s = State::default();
    for _ in 0..1000 {
        update(&mut s, Message::NewTab);
    }
    // Hammer the "close the first tab" path far more times than there are tabs;
    // the core must always keep at least one document and a valid active index.
    for _ in 0..5000 {
        update(&mut s, Message::TabClosed(0));
        assert!(!s.docs.is_empty());
        assert!(s.active < s.docs.len());
    }
    assert_eq!(s.docs.len(), 1);
}

// ---- Find / Replace / Go-to-line adversarial cases (#33) ----

#[test]
fn catastrophic_regex_pattern_stays_linear() {
    // `(a+)+$` is the textbook exponential-backtracking bomb: a backtracking
    // engine hangs for effectively forever on a long run of 'a' that fails the
    // trailing anchor. The `regex` crate is a finite automaton, so this returns
    // at once — completing at all *is* the assertion (no hang, no panic).
    let m = Matcher::new(r"(a+)+$", regex_opts()).expect("compiles");
    let evil = format!("{}!", "a".repeat(100_000)); // the '!' defeats the `$`
    assert_eq!(m.count(&evil), 0);
    // A second classic, run for its side effect of simply terminating.
    let m2 = Matcher::new(r"(x+x+)+y", regex_opts()).expect("compiles");
    let _ = m2.count(&"x".repeat(100_000));
}

#[test]
fn replace_all_on_a_huge_document_is_bounded_and_undoable() {
    let mut s = State::default();
    let huge = "foo bar\n".repeat(500_000); // ~3.5 MB, half a million matches
    update(&mut s, Message::Edited(huge));
    update(&mut s, Message::FindOpened);
    update(&mut s, Message::FindQueryChanged("foo".into()));
    assert_eq!(s.find.count, 500_000);
    update(&mut s, Message::ReplaceTextChanged("qux".into()));
    update(&mut s, Message::ReplaceAll);
    assert_eq!(s.find.count, 0);
    assert!(s.active_doc().content.starts_with("qux bar\n"));
    // The whole replace-all collapses to a single undo step.
    update(&mut s, Message::Undo);
    assert!(s.active_doc().content.starts_with("foo bar\n"));
}

#[test]
fn search_a_million_line_document() {
    let mut s = State::default();
    update(&mut s, Message::Edited("needle\n".repeat(1_000_000)));
    update(&mut s, Message::FindOpened);
    update(&mut s, Message::FindQueryChanged("needle".into()));
    assert_eq!(s.find.count, 1_000_000);
    update(&mut s, Message::FindNext);
    let hit = s.find.current.expect("a match");
    assert!(hit.end <= s.active_doc().content.len());
}

#[test]
fn find_in_a_single_enormous_line() {
    // One line, no newlines, ~2 MB — catches off-by-one line handling on big
    // input and proves go-to clamps when there is only one line.
    let line = "ab".repeat(1_000_000);
    let m = Matcher::new("ab", SearchOptions::default()).expect("compiles");
    assert_eq!(m.count(&line), 1_000_000);
    assert_eq!(goto_line_offset(&line, 5), 0);
    // The last match's end is exactly the buffer length.
    let last = m.find_last(&line).expect("a match");
    assert_eq!(last.end, line.len());
}

#[test]
fn thousands_of_find_next_calls_stay_in_bounds() {
    let mut s = State::default();
    update(&mut s, Message::Edited("x.x.x.x.x".into()));
    update(&mut s, Message::FindOpened);
    update(&mut s, Message::FindQueryChanged("x".into()));
    for _ in 0..10_000 {
        update(&mut s, Message::FindNext);
        let hit = s.find.current.expect("always on a match");
        assert!(hit.end <= s.active_doc().content.len());
    }
}

#[test]
fn go_to_line_never_leaves_the_buffer_on_a_huge_document() {
    let content = "line\n".repeat(200_000);
    // Way past the end clamps to the final line's start; the offset is always a
    // valid, in-bounds char boundary.
    for line in [0, 1, 100_000, 200_000, 5_000_000] {
        let off = goto_line_offset(&content, line);
        assert!(off <= content.len());
        assert!(content.is_char_boundary(off));
    }
    // Sanity against a known position: line 3 starts after two "line\n" chunks.
    assert_eq!(goto_line_offset(&content, 3), 10);
    // `line_col_of` round-trips that offset back to (line 2, column 0).
    assert_eq!(find::line_col_of(&content, 10), (2, 0));
}

#[test]
fn status_on_a_huge_single_line_stays_correct() {
    // A single one-megabyte line: the caret at the end reports a character
    // column of len+1, and selecting the whole line counts every character —
    // computed without materialising anything pathological.
    let mut s = State::default();
    let huge = "x".repeat(1_000_000);
    update(
        &mut s,
        Message::FileLoaded {
            path: PathBuf::from("/tmp/big.txt"),
            content: huge.clone(),
        },
    );
    let doc = s.active_doc();
    let end = doc.content.len();
    let st = status(doc, end, None);
    assert_eq!(st.line, 1);
    assert_eq!(st.column, 1_000_001);
    assert_eq!(st.chars, 1_000_000);
    assert_eq!(st.lines, 1);
    assert_eq!(st.selection, 0);
    // Selecting the whole line counts all million characters.
    assert_eq!(status(doc, end, Some(0)).selection, 1_000_000);
}

#[test]
fn status_on_a_million_line_document_stays_correct() {
    // A million "a\n" lines: the trailing newline yields a final empty line, so
    // the caret at end-of-text sits on line 1,000,001, column 1.
    let mut s = State::default();
    let content = "a\n".repeat(1_000_000);
    update(
        &mut s,
        Message::FileLoaded {
            path: PathBuf::from("/tmp/lines.txt"),
            content,
        },
    );
    let doc = s.active_doc();
    let st = status(doc, doc.content.len(), None);
    assert_eq!(st.lines, 1_000_001);
    assert_eq!(st.line, 1_000_001);
    assert_eq!(st.column, 1);
    assert_eq!(st.chars, 2_000_000);
}
