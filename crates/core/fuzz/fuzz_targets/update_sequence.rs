#![no_main]
//! Fuzz the pure update core with arbitrary message streams. The core must
//! never panic and must preserve its structural invariants after every step,
//! no matter what sequence of events the shell feeds it.

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;
use notepad_core::{Message, State, TabId, update};
use std::path::PathBuf;

// Built here rather than deriving `Arbitrary` on the core types, so the core
// stays dependency-free.
fn arb_message(u: &mut Unstructured) -> arbitrary::Result<Message> {
    Ok(match u.int_in_range(0u8..=9)? {
        0 => Message::NewTab,
        1 => Message::OpenRequested,
        2 => Message::SaveRequested,
        3 => Message::SaveAsRequested,
        4 => Message::Edited(String::arbitrary(u)?),
        5 => Message::TabSelected(u.int_in_range(0usize..=8)?),
        6 => Message::TabClosed(u.int_in_range(0usize..=8)?),
        7 => Message::FileLoaded {
            path: PathBuf::from(String::arbitrary(u)?),
            content: String::arbitrary(u)?,
        },
        8 => Message::SavePathChosen {
            id: TabId::arbitrary(u)?,
            path: PathBuf::from(String::arbitrary(u)?),
        },
        _ => Message::FileSaved {
            id: TabId::arbitrary(u)?,
            path: PathBuf::from(String::arbitrary(u)?),
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
    }
});
