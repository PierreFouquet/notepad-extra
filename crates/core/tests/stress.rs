//! Adversarial / stress tests for the update core (epic #25 Definition of Done).
//!
//! These assert the core stays correct and panic-free under abuse: hundreds of
//! opens, million-line pastes, rapid tab churn, and — for find/replace (#33) —
//! catastrophic regexes, huge-document replace-all, and thousands of Find Next.

use notepad_core::find::{self, goto_line_offset};
use notepad_core::{
    Effect, FileEncoding, Matcher, Message, SearchOptions, State, brackets, status, update,
};
use std::path::PathBuf;
use std::time::{Duration, Instant};

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
                encoding: FileEncoding::default(),
                disk: None,
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
    assert_eq!(s.active_doc().content().len(), huge.len());

    update(&mut s, Message::Undo);
    assert_eq!(
        s.active_doc().content(),
        "",
        "one undo removes the whole paste"
    );
    assert!(!s.active_doc().dirty());

    update(&mut s, Message::Redo);
    assert_eq!(s.active_doc().content(), huge, "one redo restores it");
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
    assert_eq!(s.active_doc().content(), edited);

    update(&mut s, Message::Undo);
    assert_eq!(s.active_doc().content(), base);
    update(&mut s, Message::Undo);
    assert_eq!(s.active_doc().content(), "");
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
        assert_eq!(s.active_doc().content(), content);
    }
    // Undo far more times than the (capped) history depth; each step must leave a
    // valid buffer and never panic.
    for _ in 0..4000 {
        update(&mut s, Message::Undo);
        assert!(!s.docs.is_empty());
    }
    // The oldest edits were capped away, so undo can't reach the empty buffer;
    // what matters is it stayed panic-free and consistent throughout.
    let remaining = s.active_doc().content().to_string();
    update(&mut s, Message::Redo);
    assert!(s.active_doc().content().starts_with(&remaining));
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
                encoding: FileEncoding::default(),
                disk: None,
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
        assert_eq!(doc.content(), format!("content {i}\n"));
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
                encoding: FileEncoding::default(),
                disk: None,
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
                disk: None,
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
            disk: None,
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
                encoding: FileEncoding::default(),
                disk: None,
            },
        );
    }
    assert_eq!(s.docs.len(), 250);
    for doc in &s.docs {
        assert_eq!(doc.content().len(), big.len());
    }
    // Freshly loaded tabs are clean, so each closes without a confirmation.
    for _ in 0..250 {
        update(&mut s, Message::TabClosed(0));
        assert!(!s.docs.is_empty());
    }
    assert_eq!(s.docs.len(), 1);
    assert!(s.active_doc().content().is_empty(), "ends on a fresh blank");
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
    assert!(s.active_doc().content().starts_with("qux bar\n"));
    // The whole replace-all collapses to a single undo step.
    update(&mut s, Message::Undo);
    assert!(s.active_doc().content().starts_with("foo bar\n"));
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
    assert!(hit.end <= s.active_doc().content().len());
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
        assert!(hit.end <= s.active_doc().content().len());
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
            encoding: FileEncoding::default(),
            disk: None,
        },
    );
    let doc = s.active_doc();
    let end = doc.content().len();
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
            encoding: FileEncoding::default(),
            disk: None,
        },
    );
    let doc = s.active_doc();
    let st = status(doc, doc.content().len(), None);
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
    let before = s.active_doc().content().to_string();

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
    assert_eq!(s.active_doc().content(), before);
    assert!(s.active_doc().dirty());
    update(&mut s, Message::Undo);
    assert_eq!(s.active_doc().content(), "");
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

// ---- Cached document totals and line index (#126) ----

/// A 200k-line document loaded through the real `FileLoaded` path, then dirtied
/// by one keystroke so neither readout can short-circuit on a pristine buffer.
fn two_hundred_thousand_lines() -> State {
    let mut s = State::default();
    let content = "lorem ipsum dolor sit amet\n".repeat(200_000);
    update(
        &mut s,
        Message::FileLoaded {
            path: PathBuf::from("/tmp/big.txt"),
            content: content.clone(),
            encoding: FileEncoding::default(),
            disk: None,
        },
    );
    update(&mut s, Message::Edited(format!("{content}x")));
    assert!(s.active_doc().dirty());
    s
}

#[test]
fn status_and_dirty_on_a_200k_line_document_cost_nothing_per_frame() {
    // The shell asks for the status readout and every tab's dirty flag on every
    // redraw. Before #126 each `status()` recounted the whole buffer (chars,
    // UAX #29 words and newlines — ~50 ms per call at this size in release, far
    // more in debug) and `dirty()` compared the whole buffer against its saved
    // copy. Ten thousand frames' worth of those calls would have taken the old
    // code many minutes; with the cached totals, the indexed caret maths and the
    // dirty flag they finish in well under a second even unoptimised. The bound
    // is deliberately loose for a slow CI runner — it only trips if an O(n)
    // recount sneaks back onto the per-frame path.
    let s = two_hundred_thousand_lines();
    let doc = s.active_doc();
    let lines = doc.line_count();
    assert_eq!(lines, 200_001);

    let started = Instant::now();
    let mut checksum = 0usize;
    for frame in 0..10_000usize {
        // Walk the caret across the whole document so the line index is
        // binary-searched at many depths, not just at one hot entry.
        let caret = doc.offset_at((frame * 7919) % lines, frame % 30);
        let st = status(doc, caret, None);
        checksum += st.chars + st.words + st.lines + st.line + st.column;
        checksum += usize::from(doc.dirty());
    }
    let elapsed = started.elapsed();
    assert!(checksum > 0);
    assert!(
        elapsed < Duration::from_secs(5),
        "10_000 status()+dirty() calls took {elapsed:?}: a per-frame recount is back (#126)"
    );

    // The cached numbers are the real ones.
    let text = doc.content();
    let st = status(doc, text.len(), None);
    assert_eq!(st.chars, text.chars().count());
    assert_eq!(st.words, 200_000 * 5 + 1); // five per line, plus the appended "x"
    assert_eq!(st.lines, 200_001);
    assert_eq!((st.line, st.column), (200_001, 2)); // after the trailing "x"
    // And the indexed conversions agree with the pure scanning helpers.
    for line in [0, 1, 99_999, 200_000, 999_999] {
        assert_eq!(doc.line_start(line), goto_line_offset(text, line + 1));
        assert_eq!(doc.offset_at(line, 7), find::offset_at(text, line, 7));
    }
    for offset in [0, 27, text.len() / 2, text.len(), usize::MAX] {
        assert_eq!(doc.line_col_of(offset), find::line_col_of(text, offset));
    }
}

#[test]
fn the_cr_flag_on_a_200k_line_document_costs_nothing_per_message() {
    // The shell reads `has_cr` before every position conversion, several
    // times per message, to choose between the line index and a walk of the
    // editor's lines (#153). It is a cached count: a hundred thousand reads
    // take microseconds, where a scan for a `\r` costs about 0.4 ms each at
    // this size: 37 s for the lot, measured.
    let mut s = two_hundred_thousand_lines();
    let started = Instant::now();
    let with_cr = (0..100_000)
        .filter(|_| std::hint::black_box(s.active_doc()).has_cr())
        .count();
    let elapsed = started.elapsed();
    assert_eq!(with_cr, 0);
    assert!(
        elapsed < Duration::from_secs(2),
        "100_000 has_cr() calls took {elapsed:?}: a scan of the document is back (#153)"
    );
    // A `\r` at the very end, past every line the edit path recounts, is seen.
    let content = format!("{}\r", s.active_doc().content());
    update(&mut s, Message::Edited(content));
    assert!(s.active_doc().has_cr());
}

#[test]
fn typing_into_a_200k_line_document_keeps_the_cached_totals_exact() {
    // Keystrokes arrive as the whole new text; the document must update its
    // totals from the diff'd span alone (#126) and stay exact while the edits
    // land mid-document, where every later line start has to shift. Chars and
    // lines are re-derived cheaply after each keystroke; the (expensive) word
    // oracle runs at the end over the final text.
    let mut s = two_hundred_thousand_lines();
    let mut caret = s.active_doc().line_start(100_000); // a byte offset
    let mut text = s.active_doc().content().to_string();
    for ch in "don't 你好 😀\n".chars().cycle().take(120) {
        text.insert(caret, ch);
        caret += ch.len_utf8();
        update(&mut s, Message::Edited(text.clone()));
        let doc = s.active_doc();
        assert_eq!(doc.content(), text);
        assert_eq!(doc.char_count(), text.chars().count());
        assert_eq!(doc.line_count(), find::line_count(&text));
        assert_eq!(
            doc.line_start(100_001),
            find::goto_line_offset(&text, 100_002)
        );
    }
    let doc = s.active_doc();
    assert_eq!(
        doc.word_count(),
        unicode_segmentation::UnicodeSegmentation::unicode_words(text.as_str()).count()
    );
    // Undo the whole typing run: the counts walk back to the loaded baseline.
    for _ in 0..200 {
        update(&mut s, Message::Undo);
    }
    let doc = s.active_doc();
    assert!(!doc.dirty());
    assert_eq!(doc.line_count(), 200_001);
    assert_eq!(doc.word_count(), 200_000 * 5);
}

#[test]
fn typing_at_the_top_of_a_200k_line_document_diffs_at_memory_speed() {
    // Every keystroke reaches the core as the whole new text, and `diff` finds
    // the one changed span by scanning both buffers from each end. A keystroke
    // at the very top leaves no common prefix, so the suffix scan covers the
    // entire document. This pins the scan alone, with nothing else in the timed
    // region (no `String` rebuild, no line-index shift), so the ratio between
    // the two implementations is the whole signal: on this 5.4 MB fixture in a
    // debug build one scan is ~0.5 ms once the prefix and suffix compare 4 KiB
    // blocks with `memcmp`, against ~25 ms with the byte-at-a-time loop (#126).
    // Four hundred scans take ~0.2 s and ~10 s respectively; the 2 s bound is
    // ten times the measured cost (a slow CI runner) and a fifth of what the byte
    // loop needs on a fast machine, so the un-optimised code cannot meet it.
    let text = "lorem ipsum dolor sit amet\n".repeat(200_000);
    let mut next = String::with_capacity(text.len() + 1);
    next.push('x');
    next.push_str(&text);
    let started = Instant::now();
    for i in 0..400u32 {
        let edit = if i % 2 == 0 {
            notepad_core::diff(&text, &next)
        } else {
            notepad_core::diff(&next, &text)
        }
        .expect("the buffers differ");
        assert_eq!(edit.at, 0, "the change is the single char at the top");
        assert_eq!(edit.removed.len() + edit.inserted.len(), 1);
    }
    let elapsed = started.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "400 whole-document diffs took {elapsed:?}: `diff` is scanning byte by byte again (#126)"
    );
}

#[test]
fn typing_at_the_top_of_a_200k_line_document_lands_single_char_edits() {
    // The same keystrokes through the real `update()` path: each one reaches the
    // core as the whole new text and must land as a single-character edit with
    // exact cached totals, and undo must walk every one of them back (#126).
    let mut s = two_hundred_thousand_lines();
    let baseline_chars = s.active_doc().char_count();
    for i in 0..200u8 {
        let text = s.active_doc().content();
        let mut next = String::with_capacity(text.len() + 1);
        next.push(char::from(b'a' + i % 26));
        next.push_str(text);
        update(&mut s, Message::Edited(next));
    }

    // The edits really landed, as single-character edits with exact totals.
    let doc = s.active_doc();
    let typed: String = (0..200u8)
        .rev()
        .map(|i| char::from(b'a' + i % 26))
        .collect();
    assert!(doc.content().starts_with(&typed));
    assert_eq!(doc.char_count(), baseline_chars + 200);
    assert_eq!(doc.line_count(), 200_001);
    assert_eq!(
        doc.word_count(),
        200_000 * 5 + 1,
        "the letters joined the first word"
    );
    assert_eq!(doc.content().len(), 5_400_001 + 200);
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
