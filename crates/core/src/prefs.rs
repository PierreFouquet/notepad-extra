//! Persisted user preferences (#38): the app-wide render settings that survive
//! across runs — the editor zoom (#35) and word-wrap toggle (#34) today, with a
//! version field so the light/dark theme (#36) and future settings can be added
//! later without breaking an existing config.
//!
//! This lives in the pure core (not the render shell) on purpose: the
//! serialize/parse round-trip, the corrupt-config recovery, and the schema
//! migration are then unit-, property- and fuzz-testable with no window in the
//! loop — exactly what the epic's Definition of Done for #38 asks for. The shell
//! owns only *where* the file lives and the actual read/write syscalls.

use crate::app::State;
use serde::{Deserialize, Serialize};

/// The current on-disk schema version. Bump this only when the *meaning* of an
/// existing field changes and [`Preferences::migrated`] must transform it;
/// purely additive fields (guarded by `#[serde(default)]`) need no bump, because
/// an older file simply falls back to the field's default.
pub const CURRENT_VERSION: u32 = 1;

/// The persistable preferences, mirrored to a JSON file. Every field carries a
/// `serde` default so a partial file (an older schema, or one a user hand-edited
/// and left a key out of) loads the rest and fills the gap with the app default,
/// rather than failing the whole parse.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Preferences {
    /// Schema version of the file this was read from (or the current version for
    /// a freshly snapshotted one). Used by [`Preferences::migrated`].
    #[serde(default = "default_version")]
    pub version: u32,
    /// Editor font size in points (#35). Applied through [`State::apply_preferences`],
    /// which clamps it into the valid zoom range, so an out-of-range value in a
    /// hand-edited file can never break the font-size invariant.
    #[serde(default = "default_font_size")]
    pub font_size: u16,
    /// Whether soft word-wrap is on (#34).
    #[serde(default = "default_word_wrap")]
    pub word_wrap: bool,
}

fn default_version() -> u32 {
    CURRENT_VERSION
}

fn default_font_size() -> u16 {
    State::DEFAULT_FONT_SIZE
}

fn default_word_wrap() -> bool {
    false
}

impl Default for Preferences {
    /// The out-of-the-box preferences: the same values a fresh [`State`] starts
    /// with, so a missing or unreadable config behaves identically to first run.
    fn default() -> Self {
        Preferences {
            version: default_version(),
            font_size: default_font_size(),
            word_wrap: default_word_wrap(),
        }
    }
}

impl Preferences {
    /// Serialize to the on-disk JSON string. Pretty-printed so the file stays
    /// readable and hand-editable (offline, local-only — #38).
    ///
    /// `Preferences` is a fixed struct of primitives, so serialization cannot
    /// fail; the `unwrap_or_default` is a belt-and-braces fallback that keeps
    /// this total rather than ever panicking.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Parse preferences from `text`, recovering to defaults rather than
    /// erroring. Missing fields fall back per-field via their `serde` defaults;
    /// a wholly corrupt / non-JSON file yields [`Preferences::default`]. Either
    /// way this never panics and never blocks startup — the #38 DoD's
    /// "missing / corrupt config recovers to defaults" guarantee, by construction.
    ///
    /// Unknown fields are ignored (serde's default), so a file written by a
    /// *newer* build with extra keys still loads on an older one.
    pub fn from_json(text: &str) -> Preferences {
        serde_json::from_str::<Preferences>(text)
            .unwrap_or_default()
            .migrated()
    }

    /// Forward-migrate a config read from an older schema to the current one.
    /// Additive-only so far, so this just stamps the current version; when a
    /// future version changes a field's meaning, its transform slots in here as a
    /// `match self.version { .. }` arm before the version is bumped.
    fn migrated(mut self) -> Preferences {
        self.version = CURRENT_VERSION;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let prefs = Preferences {
            version: CURRENT_VERSION,
            font_size: 22,
            word_wrap: true,
        };
        assert_eq!(Preferences::from_json(&prefs.to_json()), prefs);
    }

    #[test]
    fn default_matches_a_fresh_state() {
        // A missing config must behave exactly like first run: the parsed
        // defaults have to equal what a brand-new `State` reports.
        let state = State::default();
        let prefs = Preferences::default();
        assert_eq!(prefs.font_size, state.font_size());
        assert_eq!(prefs.word_wrap, state.word_wrap());
    }

    #[test]
    fn corrupt_input_recovers_to_defaults() {
        for junk in [
            "",
            "not json",
            "{",
            "[]",
            "null",
            "\0\0\0",
            "{\"font_size\":",
        ] {
            assert_eq!(
                Preferences::from_json(junk),
                Preferences::default(),
                "corrupt input {junk:?} should recover to defaults"
            );
        }
    }

    #[test]
    fn wrong_typed_field_recovers_to_defaults() {
        // A key present but of the wrong type fails the parse; we recover to the
        // whole-struct default rather than panicking.
        let json = r#"{"version":1,"font_size":"huge","word_wrap":true}"#;
        assert_eq!(Preferences::from_json(json), Preferences::default());
    }

    #[test]
    fn missing_fields_fall_back_per_field() {
        // Only word_wrap is present: font_size and version come from defaults,
        // and the rest of the file still loads.
        let prefs = Preferences::from_json(r#"{"word_wrap":true}"#);
        assert!(prefs.word_wrap);
        assert_eq!(prefs.font_size, default_font_size());
        assert_eq!(prefs.version, CURRENT_VERSION);
    }

    #[test]
    fn unknown_fields_are_ignored_forward_compat() {
        // A file written by a newer build carrying a not-yet-known key (e.g. the
        // theme from #36) must still load on this build.
        let prefs = Preferences::from_json(
            r#"{"version":9,"font_size":18,"word_wrap":true,"theme":"dark"}"#,
        );
        assert_eq!(prefs.font_size, 18);
        assert!(prefs.word_wrap);
        // An older schema is migrated up to the current version on load.
        assert_eq!(prefs.version, CURRENT_VERSION);
    }

    use proptest::prelude::*;

    proptest! {
        /// Any preferences value survives a serialize → parse round-trip
        /// unchanged (the version is stamped to current on load, which matches
        /// what we write). This is the load/save round-trip the #38 DoD requires.
        #[test]
        fn json_round_trips_any_prefs(font_size in any::<u16>(), word_wrap in any::<bool>()) {
            let prefs = Preferences { version: CURRENT_VERSION, font_size, word_wrap };
            prop_assert_eq!(Preferences::from_json(&prefs.to_json()), prefs);
        }

        /// Parsing arbitrary bytes never panics and always yields *some* valid
        /// preferences — the corrupt-config-recovery guarantee, as a property.
        #[test]
        fn parsing_arbitrary_text_never_panics(text in any::<String>()) {
            let prefs = Preferences::from_json(&text);
            // Whatever came out is a well-formed, current-version value.
            prop_assert_eq!(prefs.version, CURRENT_VERSION);
        }
    }
}
