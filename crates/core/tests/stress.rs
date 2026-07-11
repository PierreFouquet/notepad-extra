//! Adversarial / stress tests for the update core (epic #25 Definition of Done).
//!
//! These assert the core stays correct and panic-free under abuse: hundreds of
//! opens, million-line pastes, rapid tab churn, and — for find/replace (#33) —
//! catastrophic regexes, huge-document replace-all, and thousands of Find Next.

use notepad_core::find::{self, goto_line_offset};
use notepad_core::{Effect, Matcher, Message, SearchOptions, State, brackets, status, update};
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

#[test]
fn rapid_tab_switch_storm_stays_consistent() {
    // Open a pile of distinct tabs, then jump the selection all over the strip
    // tens of thousands of times. `active` must stay in range and each tab must
    // keep exactly the content it was loaded with (#31).
    let mut s = State::default();
    for i in 0..200 {
        update(
            &mut s,
            Message::FileLoaded {
                path: PathBuf::from(format!("/tmp/tab{i}.txt")),
                content: format!("content {i}\n"),
            },
        );
    }
    assert_eq!(s.docs.len(), 200);
    let mut idx = 0usize;
    for step in 0..50_000 {
        idx = (idx * 7 + step) % s.docs.len();
        update(&mut s, Message::TabSelected(idx));
        assert_eq!(s.active, idx);
        assert!(s.active < s.docs.len());
    }
    for (i, doc) in s.docs.iter().enumerate() {
        assert_eq!(doc.content, format!("content {i}\n"));
    }
}

#[test]
fn closing_thousands_of_dirty_tabs_via_discard_is_bounded() {
    // Every tab is dirty, so every close routes through the confirm → discard
    // path. Closing far past the tab count must never underflow, and each dirty
    // tab must ask before it goes (#31).
    let mut s = State::default();
    for _ in 0..2000 {
        update(&mut s, Message::NewTab);
        update(&mut s, Message::Edited("dirty".into())); // dirties the new active tab
    }
    assert_eq!(s.docs.len(), 2001); // 2000 new + the original blank
    for _ in 0..5000 {
        let id = s.active_doc().id;
        let dirty = s.active_doc().dirty();
        let active = s.active;
        let fx = update(&mut s, Message::TabClosed(active));
        if dirty {
            assert!(
                matches!(fx.first(), Some(Effect::ConfirmClose { .. })),
                "a dirty tab must ask before closing"
            );
            update(&mut s, Message::TabCloseDiscard(id));
        }
        assert!(!s.docs.is_empty());
        assert!(s.active < s.docs.len());
    }
    assert_eq!(s.docs.len(), 1);
}

#[test]
fn quitting_with_thousands_of_dirty_tabs_confirms_with_the_full_count() {
    // A quit with a huge pile of unsaved tabs raises exactly one prompt that
    // counts every dirty document (no per-tab walk), and discard-all then exits
    // in a single step (#69).
    let mut s = State::default();
    for _ in 0..3000 {
        update(&mut s, Message::NewTab);
        update(&mut s, Message::Edited("dirty".into())); // dirties the new active tab
    }
    // 3000 new dirty tabs + the original (clean) blank.
    assert_eq!(s.docs.len(), 3001);
    let fx = update(&mut s, Message::QuitRequested);
    assert!(
        matches!(fx.as_slice(), [Effect::ConfirmQuit { dirty: 3000 }]),
        "one prompt naming every dirty doc, and nothing else"
    );
    assert_eq!(s.docs.len(), 3001, "the prompt changes nothing");
    // The user discards everything: a single Quit, no unbounded teardown.
    assert_eq!(update(&mut s, Message::QuitDiscardAll), vec![Effect::Quit]);
}

#[test]
fn save_all_quit_across_many_tabs_exits_exactly_once() {
    // Save-all-then-quit fires one write per dirty tab and holds the exit until
    // the very last write lands — never early, never twice (#69).
    let n = 1000;
    let mut s = State::default();
    for i in 0..n {
        update(
            &mut s,
            Message::FileLoaded {
                path: PathBuf::from(format!("/tmp/q{i}.txt")),
                content: format!("base {i}\n"),
            },
        );
        update(&mut s, Message::Edited(format!("edit {i}"))); // dirties the active tab
    }
    assert_eq!(s.docs.len(), n);
    let ids: Vec<_> = s.docs.iter().map(|d| d.id).collect();

    // One write per dirty tab, and no premature quit.
    let fx = update(&mut s, Message::QuitSaveAll);
    assert_eq!(fx.len(), n);
    assert!(fx.iter().all(|e| matches!(e, Effect::WriteFile { .. })));
    assert!(!fx.iter().any(|e| matches!(e, Effect::Quit)));

    // Every write but the last lands: the app must stay alive throughout.
    for &id in &ids[..n - 1] {
        let step = update(
            &mut s,
            Message::FileSaved {
                id,
                path: PathBuf::from("/tmp/q.txt"),
            },
        );
        assert!(
            !step.iter().any(|e| matches!(e, Effect::Quit)),
            "must not exit while saves are still outstanding"
        );
    }
    // The final write drains the pending set → exactly one Quit.
    let last = update(
        &mut s,
        Message::FileSaved {
            id: *ids.last().unwrap(),
            path: PathBuf::from("/tmp/q.txt"),
        },
    );
    assert_eq!(
        last.iter().filter(|e| matches!(e, Effect::Quit)).count(),
        1,
        "the last save landing exits once and only once"
    );
}

#[test]
fn many_large_tabs_are_retained_then_freed() {
    // A proxy for the "memory footprint with many large tabs" concern: open a few
    // hundred tabs each holding a sizeable buffer, prove every buffer survives
    // intact, then close them all and prove the document list shrinks back to a
    // single tab (no lingering entries) (#31).
    let mut s = State::default();
    let big = "lorem ipsum dolor sit amet\n".repeat(2000); // ~54 KB per tab
    for i in 0..250 {
        update(
            &mut s,
            Message::FileLoaded {
                path: PathBuf::from(format!("/tmp/big{i}.txt")),
                content: big.clone(),
            },
        );
    }
    assert_eq!(s.docs.len(), 250);
    for doc in &s.docs {
        assert_eq!(doc.content.len(), big.len());
    }
    // Freshly loaded tabs are clean, so each closes without a confirmation.
    for _ in 0..250 {
        update(&mut s, Message::TabClosed(0));
        assert!(!s.docs.is_empty());
    }
    assert_eq!(s.docs.len(), 1);
    assert!(s.active_doc().content.is_empty(), "ends on a fresh blank");
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

#[test]
fn rapid_zoom_storm_stays_within_bounds_and_leaves_editing_intact() {
    // Editor zoom (#35): tens of thousands of interleaved zoom ops must always
    // keep the font size in range, and must never disturb the buffer or its
    // undo history — zoom is an app-wide view setting, not a document edit.
    let mut s = State::default();
    update(&mut s, Message::Edited("hello".into()));
    let before = s.active_doc().content.clone();

    for i in 0..50_000 {
        let msg = match i % 3 {
            0 => Message::ZoomIn,
            1 => Message::ZoomOut,
            _ => Message::ZoomReset,
        };
        // Zoom's only effect is to persist the new size (#38): it never emits a
        // document-mutating effect and never touches the buffer.
        let fx = update(&mut s, msg);
        assert!(fx.iter().all(|e| matches!(e, Effect::SavePreferences(_))));
        assert!(s.font_size() >= State::MIN_FONT_SIZE);
        assert!(s.font_size() <= State::MAX_FONT_SIZE);
    }

    // The buffer, dirty flag and undo are untouched by all that zooming.
    assert_eq!(s.active_doc().content, before);
    assert!(s.active_doc().dirty());
    update(&mut s, Message::Undo);
    assert_eq!(s.active_doc().content, "");
}

#[test]
fn monotonic_zoom_to_each_extreme_settles_exactly_on_the_bound() {
    // Push far past either end: the value must pin exactly at the bound, never
    // oscillate, underflow, or overflow.
    let mut s = State::default();
    for _ in 0..10_000 {
        update(&mut s, Message::ZoomIn);
    }
    assert_eq!(s.font_size(), State::MAX_FONT_SIZE);
    for _ in 0..10_000 {
        update(&mut s, Message::ZoomOut);
    }
    assert_eq!(s.font_size(), State::MIN_FONT_SIZE);
}

// ---- Bracket matching (#41) ----

#[test]
fn deeply_nested_brackets_match_without_stack_overflow() {
    // 100k nested pairs: the depth scan is iterative, so this must resolve
    // without recursing (a recursive matcher would blow the stack here).
    let n = 100_000;
    let text = format!("{}{}", "(".repeat(n), ")".repeat(n));

    // The outermost '(' (caret at 0) pairs with the very last ')'.
    let m = brackets::match_at(&text, 0).expect("outer opener matches");
    assert_eq!(m.here, 0);
    assert_eq!(m.partner, Some(text.len() - 1));

    // The innermost pair sits back-to-back at the middle: '(' at n-1, ')' at n.
    let inner = brackets::match_at(&text, n).expect("innermost opener matches");
    assert_eq!(inner.here, n - 1);
    assert_eq!(inner.partner, Some(n));
}

#[test]
fn unbalanced_bracket_storm_scans_fully_without_panicking() {
    // A single enormous run of openers with no closer: matching from the front
    // scans the whole document to conclude there is no partner, and from the back
    // finds nothing to its left — either way, no panic and no false match.
    let text = "(".repeat(200_000);
    let from_front = brackets::match_at(&text, 0).expect("touches the first '('");
    assert_eq!(
        from_front,
        brackets::BracketMatch {
            here: 0,
            partner: None
        }
    );
    let from_back = brackets::match_at(&text, text.len()).expect("touches the last '('");
    assert_eq!(from_back.here, text.len() - 1);
    assert_eq!(from_back.partner, None);
}

#[test]
fn sweeping_the_caret_across_a_huge_bracket_document_stays_bounded() {
    // A million-character line of balanced pairs; probing thousands of caret
    // positions must never panic and must keep matched pairs consistent.
    let text = "()".repeat(500_000); // 1,000,000 chars
    for k in (0..text.len()).step_by(997) {
        if let Some(m) = brackets::match_at(&text, k)
            && let Some(p) = m.partner
        {
            // Every "()" is a self-contained pair, so partners are adjacent.
            assert_eq!(p.abs_diff(m.here), 1);
        }
    }
}
