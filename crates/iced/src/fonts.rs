//! Font bundling + family resolution (#61).
//!
//! Notepad Extra must render **fully offline** (epic #25). To guarantee that with
//! the smallest footprint, exactly **one** family — DejaVu Sans Mono — is embedded
//! into the binary as bytes ([`BUNDLED_FONT_FACES`], registered at startup via the
//! iced application builder's `.font(..)`). It is the default for both the editor
//! and the UI, so a fresh install with *zero* system fonts still renders.
//!
//! Every *other* choice in the two font pickers comes from the machine's own
//! installed fonts: iced's global font database already loads the OS fonts (see
//! [`available_families`]), so the user can select any of them. Those are
//! best-effort — not bundled, not redistributed, not guaranteed — while the
//! bundled family remains the offline safety net. See `fonts/ATTRIBUTION.md`.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock};

use iced::Font;

/// The single family we bundle and can always render, whatever the host has
/// installed. Matches [`core::State::DEFAULT_EDITOR_FONT`] /
/// [`core::State::DEFAULT_UI_FONT`] so the core's default resolves to a real
/// embedded face.
pub const BUNDLED_FAMILY: &str = "DejaVu Sans Mono";

/// The embedded faces of [`BUNDLED_FAMILY`] (Book / Bold / Oblique / Bold-Oblique),
/// byte-for-byte upstream. `main` hands each to the iced builder's `.font(..)` so
/// they load into the global font database at startup. Kept as `&'static [u8]`
/// (not owned) so registration never copies them.
pub const BUNDLED_FONT_FACES: &[&[u8]] = &[
    include_bytes!("../fonts/dejavu-sans-mono/DejaVuSansMono.ttf"),
    include_bytes!("../fonts/dejavu-sans-mono/DejaVuSansMono-Bold.ttf"),
    include_bytes!("../fonts/dejavu-sans-mono/DejaVuSansMono-Oblique.ttf"),
    include_bytes!("../fonts/dejavu-sans-mono/DejaVuSansMono-BoldOblique.ttf"),
];

/// Resolve a family *name* (from the core prefs or a picker) to an [`iced::Font`].
///
/// iced's [`Font::with_name`] takes a `&'static str`, but the chosen family is a
/// runtime string, so we intern it (see [`intern`]) — leaking a *bounded* number
/// of short strings, one per distinct family ever selected — and reuse it after.
/// That makes this safe to call from `view` on every frame without leaking per
/// frame. An empty / whitespace name falls back to the bundled family, matching
/// the core's own normalisation, so the editor is never left unrenderable.
pub fn resolve(name: &str) -> Font {
    let name = name.trim();
    let name = if name.is_empty() {
        BUNDLED_FAMILY
    } else {
        name
    };
    Font::with_name(intern(name))
}

/// Intern `name` to a `'static` string so it can go into [`Font::with_name`].
/// The bundled family is already `'static`, so it short-circuits with no leak;
/// any other name is leaked once and cached, so repeated resolves of the same
/// family reuse the same allocation.
fn intern(name: &str) -> &'static str {
    if name == BUNDLED_FAMILY {
        return BUNDLED_FAMILY;
    }
    static CACHE: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = cache.lock().expect("font intern cache poisoned");
    if let Some(&s) = map.get(name) {
        return s;
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    map.insert(name.to_owned(), leaked);
    leaked
}

/// The families the pickers offer: the bundled family plus every family in iced's
/// global font database — which already has the OS's installed fonts loaded (its
/// backend calls `load_system_fonts` on init). Sorted and de-duplicated, computed
/// once and cached.
///
/// Accessing the global font system here forces its (one-time) initialisation,
/// which is exactly what we want — the same database the renderer draws from is
/// the one we enumerate, so a listed family is one iced can actually resolve.
pub fn available_families() -> &'static [String] {
    static FAMILIES: OnceLock<Vec<String>> = OnceLock::new();
    FAMILIES.get_or_init(|| {
        let mut set: BTreeSet<String> = BTreeSet::new();
        // The bundled family is always offered, even if font enumeration finds
        // nothing (a headless / fontless host).
        set.insert(BUNDLED_FAMILY.to_string());
        if let Ok(mut system) = iced_graphics::text::font_system().write() {
            for face in system.raw().db().faces() {
                if let Some((family, _language)) = face.families.first() {
                    set.insert(family.clone());
                }
            }
        }
        set.into_iter().collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use iced::font::Family;

    /// The valid `sfnt` magic numbers a TrueType/OpenType file can start with.
    fn is_sfnt(bytes: &[u8]) -> bool {
        matches!(
            bytes.get(0..4),
            Some([0x00, 0x01, 0x00, 0x00]) // TrueType outlines
                | Some(b"true")            // Apple TrueType
                | Some(b"OTTO")            // CFF / OpenType outlines
                | Some(b"ttcf") // TrueType collection
        )
    }

    #[test]
    fn all_bundled_faces_are_real_fonts() {
        // The offline guarantee (#61) rests on these embedding correctly: every
        // face must be present, non-trivial, and a valid sfnt.
        assert_eq!(BUNDLED_FONT_FACES.len(), 4, "Book/Bold/Oblique/BoldOblique");
        for (i, face) in BUNDLED_FONT_FACES.iter().enumerate() {
            assert!(face.len() > 10_000, "face {i} implausibly small");
            assert!(is_sfnt(face), "face {i} is not a valid sfnt font");
        }
    }

    #[test]
    fn resolve_maps_a_name_to_that_family() {
        assert_eq!(resolve("Arial").family, Family::Name("Arial"));
        assert_eq!(
            resolve(BUNDLED_FAMILY).family,
            Family::Name("DejaVu Sans Mono")
        );
    }

    #[test]
    fn resolve_trims_and_defaults_empty_to_the_bundled_family() {
        assert_eq!(
            resolve("   ").family,
            Family::Name("DejaVu Sans Mono"),
            "an empty/whitespace name falls back to the bundled family"
        );
        assert_eq!(
            resolve("  Fira Code  ").family,
            Family::Name("Fira Code"),
            "surrounding whitespace is trimmed"
        );
    }

    #[test]
    fn interning_is_stable_across_calls() {
        // Two resolves of the same non-bundled name reuse one interned &'static
        // str, so `view` calling `resolve` every frame does not leak per frame.
        let (a, b) = ("Some Installed Face", "Some Installed Face");
        let (Family::Name(x), Family::Name(y)) = (resolve(a).family, resolve(b).family) else {
            panic!("expected named families");
        };
        assert!(
            std::ptr::eq(x, y),
            "same name must intern to the same pointer"
        );
    }

    #[test]
    fn available_families_always_contains_the_bundled_family() {
        let families = available_families();
        assert!(
            families.iter().any(|f| f == BUNDLED_FAMILY),
            "the bundled family must always be offered"
        );
        // Sorted + de-duplicated (BTreeSet invariant), so the picker is orderly.
        let mut sorted = families.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(&sorted, families, "families must be sorted and unique");
    }
}
