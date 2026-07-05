#![no_main]
//! Fuzz the preferences config parser (#38) — the "anything parsing/decoding"
//! target the epic's DoD calls for. `Preferences::from_json` runs at startup on
//! whatever bytes the config file holds, which a user may have hand-edited or
//! which may be truncated/corrupt. For *any* input it must:
//!
//! * never panic (recover to defaults instead of erroring);
//! * always yield a current-schema value whose font size, once applied, stays
//!   inside the editor's zoom bounds — so a bad config can never break the
//!   font-size invariant on the next run.

use libfuzzer_sys::fuzz_target;
use notepad_core::{Preferences, State};

fuzz_target!(|text: String| {
    let prefs = Preferences::from_json(&text);

    // Migration always stamps the current version.
    assert_eq!(prefs.version, notepad_core::prefs::CURRENT_VERSION);

    // Applying whatever parsed out must land the live font size inside range:
    // `apply_preferences` clamps, so no config value can escape the bounds.
    let mut state = State::default();
    state.apply_preferences(&prefs);
    assert!(state.font_size() >= State::MIN_FONT_SIZE);
    assert!(state.font_size() <= State::MAX_FONT_SIZE);

    // Re-serializing the applied prefs and parsing again is a fixed point.
    let reparsed = Preferences::from_json(&state.preferences().to_json());
    assert_eq!(reparsed, state.preferences());
});
