//! Adversarial / stress tests for the update core (epic #25 Definition of Done).
//!
//! These assert the core stays correct and panic-free under abuse: hundreds of
//! opens, million-line pastes, and rapid tab churn. Regex-catastrophe cases
//! land here once find/replace (#33) exists.

use notepad_core::{Effect, Message, State, update};
use std::path::PathBuf;

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
