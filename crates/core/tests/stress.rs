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
    assert!(s.active_doc().dirty);

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
