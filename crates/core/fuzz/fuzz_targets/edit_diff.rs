#![no_main]
//! Fuzz the diff/apply/invert core of the editing model (#30). For any pair of
//! strings, `diff` must produce an edit that turns `old` into `new` when applied,
//! and whose inverse turns it back — never panicking or splitting a UTF-8 char.

use libfuzzer_sys::fuzz_target;
use notepad_core::diff;

fuzz_target!(|pair: (String, String)| {
    let (old, new) = pair;
    match diff(&old, &new) {
        None => {
            // `diff` returns `None` only when the strings are already equal.
            assert_eq!(old, new);
        }
        Some(edit) => {
            // `at` and the removed span must land on char boundaries.
            assert!(old.is_char_boundary(edit.at));
            assert!(old.is_char_boundary(edit.at + edit.removed.len()));

            // Forward: old -> new.
            let mut buf = old.clone();
            edit.apply(&mut buf);
            assert_eq!(buf, new, "diff did not turn old into new");

            // Inverse: new -> old.
            edit.invert().apply(&mut buf);
            assert_eq!(buf, old, "inverse did not restore old");
        }
    }
});
