#![no_main]
//! Fuzz the pure update core with arbitrary message streams. The core must
//! never panic and must preserve its structural invariants after every step,
//! no matter what sequence of events the shell feeds it.

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;
use notepad_core::{FindOption, Message, State, TabId, update};
use std::path::PathBuf;

// Built here rather than deriving `Arbitrary` on the core types, so the core
// stays free of the `arbitrary` dependency.
fn arb_message(u: &mut Unstructured) -> arbitrary::Result<Message> {
    Ok(match u.int_in_range(0u8..=24)? {
        0 => Message::NewTab,
        1 => Message::OpenRequested,
        2 => Message::SaveRequested,
        3 => Message::SaveAsRequested,
        4 => Message::Edited(String::arbitrary(u)?),
        5 => Message::Undo,
        6 => Message::Redo,
        7 => Message::TabSelected(u.int_in_range(0usize..=8)?),
        8 => Message::TabClosed(u.int_in_range(0usize..=8)?),
        9 => Message::FileLoaded {
            path: PathBuf::from(String::arbitrary(u)?),
            content: String::arbitrary(u)?,
        },
        10 => Message::SavePathChosen {
            id: TabId::arbitrary(u)?,
            path: PathBuf::from(String::arbitrary(u)?),
        },
        11 => Message::FileSaved {
            id: TabId::arbitrary(u)?,
            path: PathBuf::from(String::arbitrary(u)?),
        },
        // Find / Replace / Go-to (#33): exercise the whole bar under fuzzing, so
        // stale-offset regressions (e.g. undo/redo shrinking the buffer) surface.
        12 => Message::FindOpened,
        13 => Message::FindClosed,
        14 => Message::FindQueryChanged(String::arbitrary(u)?),
        15 => Message::ReplaceTextChanged(String::arbitrary(u)?),
        16 => Message::FindOptionToggled(match u.int_in_range(0u8..=2)? {
            0 => FindOption::CaseSensitive,
            1 => FindOption::WholeWord,
            _ => FindOption::Regex,
        }),
        17 => Message::FindNext,
        18 => Message::FindPrev,
        19 => Message::ReplaceNext,
        20 => Message::ReplaceAll,
        21 => Message::GoToLine(usize::arbitrary(u)?),
        // Close-with-unsaved guard (#31): drive the discard / save-then-close /
        // abandon paths with arbitrary ids so pending_close can never strand.
        22 => Message::TabCloseDiscard(TabId::arbitrary(u)?),
        23 => Message::TabCloseSave(TabId::arbitrary(u)?),
        _ => Message::SaveAbandoned {
            id: TabId::arbitrary(u)?,
        },
    })
}

fuzz_target!(|data: &[u8]| {
    let mut u = Unstructured::new(data);
    let mut state = State::default();
    for _ in 0..256 {
        if u.is_empty() {
            break;
        }
        let Ok(msg) = arb_message(&mut u) else { break };
        let _ = update(&mut state, msg);
        assert!(!state.docs.is_empty(), "docs must never be empty");
        assert!(
            state.active < state.docs.len(),
            "active index must stay in bounds"
        );
        // A highlighted find match must never dangle past the active buffer.
        if let Some(hit) = state.find.current {
            let content = &state.active_doc().content;
            assert!(hit.end <= content.len(), "match end past buffer");
            assert!(content.is_char_boundary(hit.start) && content.is_char_boundary(hit.end));
        }
    }
});
