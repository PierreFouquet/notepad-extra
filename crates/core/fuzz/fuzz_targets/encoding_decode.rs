#![no_main]
//! Fuzz the multi-encoding decode path (#50, folding in #59/#103). On open we
//! hand `decode` the raw bytes of *any* file — text, a legacy code page, UTF-16,
//! or outright binary — and on *reopen* we hand `decode_with` those same bytes
//! plus a user-chosen encoding. Neither may ever panic or error: a binary or
//! undetectable file must open lossily (#59), so for arbitrary bytes both must:
//!
//! * return a valid Unicode `String` that is always representable again — the
//!   decoded text re-encodes as UTF-8 without ever tripping the lossy guard;
//! * report an encoding that is self-consistent — if its label is one the picker
//!   knows, it round-trips through `from_label` to the very same encoding (a
//!   guard against the label/table drift `from_label` and `label` could suffer);
//! * force-decode under every offered encoding without panicking.

use libfuzzer_sys::fuzz_target;
use notepad_core::encoding::{self, FileEncoding};

fuzz_target!(|bytes: &[u8]| {
    // Auto-detect path: never panics, never errors, always valid Unicode text.
    let (text, enc) = encoding::decode(bytes);

    // Decoded text is always representable, so re-encoding at the default UTF-8
    // encoding never trips the lossy guard (#103's "UTF-8 round-trips" floor).
    assert!(
        encoding::encode_for_save(&text, FileEncoding::default()).is_ok(),
        "decoded text failed to re-encode as UTF-8",
    );

    // The detected encoding is self-consistent: if the picker names this label,
    // parsing it back yields exactly this encoding (an exotic auto-detected guess
    // outside the table simply isn't in `from_label`, which is fine).
    let label = enc.label();
    if let Some(parsed) = FileEncoding::from_label(label) {
        assert_eq!(parsed, enc, "label {label:?} round-trips to a different encoding");
    }

    // Forced-decode path (reopen): every offered encoding force-decodes the same
    // arbitrary bytes without panicking, and the forced text is likewise always
    // representable back as UTF-8.
    for &opt in encoding::options() {
        let want = FileEncoding::from_label(opt).expect("options() label must parse");
        let (forced, _got) = encoding::decode_with(bytes, want);
        assert!(
            encoding::encode_for_save(&forced, FileEncoding::default()).is_ok(),
            "force-decoded text ({opt}) failed to re-encode as UTF-8",
        );

        // Strict reopen path (#50): Ok or a named Err, but never a panic. When it
        // does accept the bytes, that text is likewise always representable again.
        if let Ok((strict, _)) = encoding::decode_strict(bytes, want) {
            assert!(
                encoding::encode_for_save(&strict, FileEncoding::default()).is_ok(),
                "strict-decoded text ({opt}) failed to re-encode as UTF-8",
            );
        }
    }
});
