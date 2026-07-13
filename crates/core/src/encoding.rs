//! Character-encoding detection, decode and encode (#50, folding in #59/#103).
//!
//! Notepad Extra reads and writes text as UTF-8 internally; this module is the
//! pure boundary that turns raw file **bytes** into that internal `String` on
//! open and back into bytes on save. It lives in the core (epic #25's testing
//! standard) so every path is unit- and fuzz-tested with no toolkit or I/O in
//! the loop: the render shell reads/writes the actual file and calls these
//! functions around the raw bytes.
//!
//! Three guarantees hold by construction:
//! - **Never panics on arbitrary bytes.** [`decode`] always yields *some* text
//!   (malformed sequences become U+FFFD), so a binary or mis-detected file
//!   opens rather than erroring (#59).
//! - **UTF-8 round-trips byte-identically.** A valid-UTF-8 file with no BOM
//!   decodes as UTF-8 and re-encodes to the exact same bytes (#103).
//! - **Saves are never silently lossy.** [`encode_for_save`] refuses (returns
//!   `Err`) when the buffer holds a character the chosen legacy encoding cannot
//!   represent, rather than writing a corrupted file (#103's defined policy).
//!
//! Encoding is [`encoding_rs`] (the WHATWG Encoding Standard) plus [`chardetng`]
//! for the no-BOM detection heuristic — both pure Rust, offline, no C.

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::{
    Encoding, GBK, ISO_8859_2, ISO_8859_15, SHIFT_JIS, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252,
};

/// A text encoding as tracked per document: the underlying [`encoding_rs`]
/// encoding plus whether a byte-order / UTF-8 signature (BOM) is part of it.
///
/// `bom` is meaningful only for UTF-8 and UTF-16 (legacy code pages have no
/// BOM); it distinguishes the `"UTF-8"` and `"UTF-8 BOM"` picker entries and
/// records, for a decoded file, whether the on-disk bytes carried a signature
/// so a round-tripped save reproduces it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileEncoding {
    enc: &'static Encoding,
    bom: bool,
}

/// The default: UTF-8 with no BOM. Keeps the common path byte-identical.
impl Default for FileEncoding {
    fn default() -> Self {
        FileEncoding {
            enc: UTF_8,
            bom: false,
        }
    }
}

/// One picker entry: a stable human label bound to an encoding + BOM flag. The
/// single [`ENCODINGS`] table is the source of truth so `label` / `from_label` /
/// [`options`] cannot drift apart (asserted by `options_match_table`).
struct Entry {
    label: &'static str,
    enc: &'static Encoding,
    bom: bool,
}

/// The encodings offered in the picker, in display order. UTF-16 defaults to a
/// BOM because BOM-less UTF-16 is ambiguous and conventionally signed; the
/// legacy code pages never carry a BOM. `decode` can still auto-detect any
/// [`encoding_rs`] encoding regardless of what this list exposes.
static ENCODINGS: &[Entry] = &[
    Entry {
        label: "UTF-8",
        enc: UTF_8,
        bom: false,
    },
    Entry {
        label: "UTF-8 BOM",
        enc: UTF_8,
        bom: true,
    },
    Entry {
        label: "UTF-16 LE",
        enc: UTF_16LE,
        bom: true,
    },
    Entry {
        label: "UTF-16 BE",
        enc: UTF_16BE,
        bom: true,
    },
    Entry {
        label: "Windows-1252",
        enc: WINDOWS_1252,
        bom: false,
    },
    Entry {
        label: "ISO-8859-2",
        enc: ISO_8859_2,
        bom: false,
    },
    Entry {
        label: "ISO-8859-15",
        enc: ISO_8859_15,
        bom: false,
    },
    Entry {
        label: "Shift_JIS",
        enc: SHIFT_JIS,
        bom: false,
    },
    Entry {
        label: "GBK",
        enc: GBK,
        bom: false,
    },
];

/// The picker labels, in display order. Kept in lock-step with [`ENCODINGS`] by
/// the `options_match_table` test.
static OPTIONS: &[&str] = &[
    "UTF-8",
    "UTF-8 BOM",
    "UTF-16 LE",
    "UTF-16 BE",
    "Windows-1252",
    "ISO-8859-2",
    "ISO-8859-15",
    "Shift_JIS",
    "GBK",
];

impl FileEncoding {
    /// The label shown in the status-bar picker. An exact `(enc, bom)` match in
    /// [`ENCODINGS`] wins; an encoding auto-detected outside the table (a rare
    /// exotic [`chardetng`] guess) falls back to its canonical WHATWG name so
    /// the status bar still names it.
    pub fn label(self) -> &'static str {
        ENCODINGS
            .iter()
            .find(|e| e.enc == self.enc && e.bom == self.bom)
            .map(|e| e.label)
            .unwrap_or_else(|| self.enc.name())
    }

    /// Parse a picker label back into a [`FileEncoding`]; `None` for a label not
    /// in the table (treated as a no-op by the caller, like an unknown language).
    pub fn from_label(label: &str) -> Option<FileEncoding> {
        ENCODINGS
            .iter()
            .find(|e| e.label == label)
            .map(|e| FileEncoding {
                enc: e.enc,
                bom: e.bom,
            })
    }
}

/// The picker's labels, in display order.
pub fn options() -> &'static [&'static str] {
    OPTIONS
}

/// Auto-detect the encoding of `bytes` and decode to text. Never fails and never
/// panics — a binary or undetectable file opens lossily (#59). The policy
/// reconciles #59 and #103:
///
/// 1. A recognised **BOM** wins (UTF-8 / UTF-16 LE / UTF-16 BE): decode the
///    remainder as that encoding, `bom: true`.
/// 2. else **valid UTF-8** → UTF-8 with no BOM, so the default path is exact.
/// 3. else the **[`chardetng`] guess**, which always returns a concrete
///    encoding (Windows-1252 for the undetermined / binary case — the WHATWG
///    fallback, and byte-preserving, so a binary file round-trips instead of
///    filling with U+FFFD).
///
/// Malformed sequences under the chosen encoding decode to U+FFFD.
pub fn decode(bytes: &[u8]) -> (String, FileEncoding) {
    if let Some((enc, bom_len)) = Encoding::for_bom(bytes) {
        let (text, _) = enc.decode_without_bom_handling(&bytes[bom_len..]);
        return (text.into_owned(), FileEncoding { enc, bom: true });
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return (
            text.to_owned(),
            FileEncoding {
                enc: UTF_8,
                bom: false,
            },
        );
    }
    // We decode trusted local files, not scriptable web content, so both
    // ISO-2022-JP and UTF-8 are allowed guesses (the UTF-8 case is already
    // handled above, so `Allow` only affects mixed/edge byte streams).
    let mut detector = EncodingDetector::new(Iso2022JpDetection::Allow);
    detector.feed(bytes, true);
    let enc = detector.guess(None, Utf8Detection::Allow);
    let (text, _) = enc.decode_without_bom_handling(bytes);
    (text.into_owned(), FileEncoding { enc, bom: false })
}

/// Force-decode `bytes` as `want` — used by *reopen*, when the user picks a
/// specific encoding for a clean tab to fix a wrong auto-detect. A leading BOM
/// matching `want`'s family is stripped (so U+FEFF never leaks into the buffer)
/// and reflected in the returned `bom`; otherwise all bytes are decoded.
pub fn decode_with(bytes: &[u8], want: FileEncoding) -> (String, FileEncoding) {
    let (body, bom) = match Encoding::for_bom(bytes) {
        Some((bom_enc, bom_len)) if bom_enc == want.enc => (&bytes[bom_len..], true),
        _ => (bytes, false),
    };
    let (text, _) = want.enc.decode_without_bom_handling(body);
    (text.into_owned(), FileEncoding { enc: want.enc, bom })
}

/// Strictly decode `bytes` as `want` for an explicit *reopen* ("Reopen as…").
/// Like [`decode_with`] a matching leading BOM is stripped (and reflected in the
/// returned `bom`), but this **refuses** (returns `Err`) when the bytes are not
/// valid `want` — i.e. a decode would have to substitute U+FFFD for a malformed
/// sequence. This mirrors [`encode_for_save`]'s lossy guard on the save side: an
/// incompatible reopen is blocked and the buffer left untouched, rather than
/// silently filling the view with mojibake. (The single-byte legacy code pages
/// are total — every byte maps — so a reopen under them always succeeds; the
/// block bites UTF-8 / UTF-16 / Shift_JIS / GBK, whose multi-byte forms can be
/// malformed.)
pub fn decode_strict(bytes: &[u8], want: FileEncoding) -> Result<(String, FileEncoding), String> {
    let (body, bom) = match Encoding::for_bom(bytes) {
        Some((bom_enc, bom_len)) if bom_enc == want.enc => (&bytes[bom_len..], true),
        _ => (bytes, false),
    };
    match want
        .enc
        .decode_without_bom_handling_and_without_replacement(body)
    {
        Some(text) => Ok((text.into_owned(), FileEncoding { enc: want.enc, bom })),
        None => {
            let n = count_malformed(body, want.enc);
            Err(format!(
                "Cannot reopen as {}: {} byte sequence(s) not valid {}",
                want.label(),
                n,
                want.label()
            ))
        }
    }
}

/// Count the U+FFFD replacements a lossy decode of `bytes` as `enc` would emit —
/// how many malformed sequences an incompatible reopen would have to paper over.
/// Only called on the error path (a reopen is about to be blocked), so the extra
/// decode pass is fine.
fn count_malformed(bytes: &[u8], enc: &'static Encoding) -> usize {
    let (text, _) = enc.decode_without_bom_handling(bytes);
    text.chars().filter(|&c| c == '\u{FFFD}').count()
}

/// Encode `text` for saving as `encoding`, prepending a BOM when the encoding
/// carries one. UTF-8 and UTF-16 can represent all of Unicode and always
/// succeed. A legacy code page that cannot represent some character returns
/// `Err` naming the encoding and the count — the caller **blocks the save** and
/// leaves the file untouched, rather than writing a corrupted file (#103).
///
/// `encoding_rs` cannot *encode* to UTF-16, so UTF-16 is produced by hand from
/// [`str::encode_utf16`].
pub fn encode_for_save(text: &str, encoding: FileEncoding) -> Result<Vec<u8>, String> {
    let enc = encoding.enc;
    if enc == UTF_8 {
        let mut out = Vec::with_capacity(text.len() + 3);
        if encoding.bom {
            out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        }
        out.extend_from_slice(text.as_bytes());
        return Ok(out);
    }
    if enc == UTF_16LE || enc == UTF_16BE {
        let big_endian = enc == UTF_16BE;
        let mut out = Vec::with_capacity(text.len() * 2 + 2);
        if encoding.bom {
            out.extend_from_slice(if big_endian {
                &[0xFE, 0xFF]
            } else {
                &[0xFF, 0xFE]
            });
        }
        for unit in text.encode_utf16() {
            let pair = if big_endian {
                unit.to_be_bytes()
            } else {
                unit.to_le_bytes()
            };
            out.extend_from_slice(&pair);
        }
        return Ok(out);
    }
    // Legacy code page: `encode` substitutes an HTML numeric reference for any
    // character it cannot map and reports `had_errors`. We refuse rather than
    // write that substitution.
    let (bytes, _, had_errors) = enc.encode(text);
    if had_errors {
        let n = count_unmappable(text, enc);
        return Err(format!(
            "Cannot save as {}: {} character(s) not representable",
            encoding.label(),
            n
        ));
    }
    Ok(bytes.into_owned())
}

/// Count the characters in `text` that `enc` cannot represent. Only called on
/// the error path (a save is about to be blocked), so per-character re-encoding
/// is fine; the legacy code pages here map each character independently, so a
/// single-character probe is accurate.
fn count_unmappable(text: &str, enc: &'static Encoding) -> usize {
    let mut buf = [0u8; 4];
    text.chars()
        .filter(|c| {
            let one = c.encode_utf8(&mut buf);
            let (_, _, had_errors) = enc.encode(one);
            had_errors
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn fe(label: &str) -> FileEncoding {
        FileEncoding::from_label(label).expect("known label")
    }

    #[test]
    fn options_match_table() {
        assert_eq!(OPTIONS.len(), ENCODINGS.len());
        for (opt, entry) in OPTIONS.iter().zip(ENCODINGS.iter()) {
            assert_eq!(*opt, entry.label);
        }
    }

    #[test]
    fn label_from_label_round_trip() {
        for label in options() {
            let enc = FileEncoding::from_label(label).expect("known label");
            assert_eq!(enc.label(), *label);
        }
        assert_eq!(FileEncoding::from_label("nonsense"), None);
    }

    #[test]
    fn default_is_utf8_no_bom() {
        let d = FileEncoding::default();
        assert_eq!(d.label(), "UTF-8");
        assert_eq!(d, fe("UTF-8"));
    }

    #[test]
    fn empty_bytes_decode_as_utf8() {
        let (text, enc) = decode(b"");
        assert_eq!(text, "");
        assert_eq!(enc, fe("UTF-8"));
    }

    #[test]
    fn plain_utf8_decodes_without_bom() {
        let (text, enc) = decode("héllo 世界".as_bytes());
        assert_eq!(text, "héllo 世界");
        assert_eq!(enc, fe("UTF-8"));
    }

    #[test]
    fn utf8_bom_is_detected_and_stripped() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("hi".as_bytes());
        let (text, enc) = decode(&bytes);
        assert_eq!(text, "hi");
        assert_eq!(enc, fe("UTF-8 BOM"));
    }

    #[test]
    fn bom_only_file_decodes_to_empty() {
        let (text, enc) = decode(&[0xEF, 0xBB, 0xBF]);
        assert_eq!(text, "");
        assert_eq!(enc, fe("UTF-8 BOM"));
    }

    #[test]
    fn utf16_le_round_trips_with_bom() {
        let text = "Hello, 世界! 😀";
        let bytes = encode_for_save(text, fe("UTF-16 LE")).unwrap();
        assert_eq!(&bytes[..2], &[0xFF, 0xFE]); // LE BOM
        let (decoded, enc) = decode(&bytes);
        assert_eq!(decoded, text);
        assert_eq!(enc, fe("UTF-16 LE"));
    }

    #[test]
    fn utf16_be_round_trips_with_bom() {
        let text = "Hello, 世界! 😀";
        let bytes = encode_for_save(text, fe("UTF-16 BE")).unwrap();
        assert_eq!(&bytes[..2], &[0xFE, 0xFF]); // BE BOM
        let (decoded, enc) = decode(&bytes);
        assert_eq!(decoded, text);
        assert_eq!(enc, fe("UTF-16 BE"));
    }

    #[test]
    fn windows_1252_round_trips_via_forced_decode() {
        let text = "café — résumé"; // é and the em dash both live in Windows-1252
        let bytes = encode_for_save(text, fe("Windows-1252")).unwrap();
        // The em dash is a single 0x97 byte in Windows-1252, not multi-byte UTF-8.
        assert!(bytes.contains(&0x97));
        let (decoded, enc) = decode_with(&bytes, fe("Windows-1252"));
        assert_eq!(decoded, text);
        assert_eq!(enc, fe("Windows-1252"));
    }

    #[test]
    fn iso_8859_variants_round_trip() {
        // ISO-8859-2 (Central European): 'ř' is representable there but not in 8859-15.
        let text = "Dvořák";
        let bytes = encode_for_save(text, fe("ISO-8859-2")).unwrap();
        let (decoded, _) = decode_with(&bytes, fe("ISO-8859-2"));
        assert_eq!(decoded, text);

        // ISO-8859-15 (Latin-9): the euro sign is its headline addition over Latin-1.
        let euro = "10 € each";
        let bytes = encode_for_save(euro, fe("ISO-8859-15")).unwrap();
        let (decoded, _) = decode_with(&bytes, fe("ISO-8859-15"));
        assert_eq!(decoded, euro);
    }

    #[test]
    fn shift_jis_and_gbk_round_trip() {
        let jp = "日本語テキスト";
        let bytes = encode_for_save(jp, fe("Shift_JIS")).unwrap();
        let (decoded, _) = decode_with(&bytes, fe("Shift_JIS"));
        assert_eq!(decoded, jp);

        let zh = "中文文本";
        let bytes = encode_for_save(zh, fe("GBK")).unwrap();
        let (decoded, _) = decode_with(&bytes, fe("GBK"));
        assert_eq!(decoded, zh);
    }

    #[test]
    fn lossy_legacy_save_is_blocked_with_a_named_error() {
        let err = encode_for_save("😀", fe("Windows-1252")).unwrap_err();
        assert!(
            err.contains("Windows-1252"),
            "error names the encoding: {err}"
        );
        assert!(err.contains('1'), "error counts one bad char: {err}");
    }

    #[test]
    fn lossy_count_is_accurate() {
        // Two emoji, both unrepresentable in Windows-1252, around mappable text.
        let err = encode_for_save("a😀b🎉c", fe("Windows-1252")).unwrap_err();
        assert!(err.contains('2'), "error counts two bad chars: {err}");
    }

    #[test]
    fn unicode_targets_never_reject() {
        let text = "emoji 😀 kanji 日本 accents éàü";
        assert!(encode_for_save(text, fe("UTF-8")).is_ok());
        assert!(encode_for_save(text, fe("UTF-8 BOM")).is_ok());
        assert!(encode_for_save(text, fe("UTF-16 LE")).is_ok());
        assert!(encode_for_save(text, fe("UTF-16 BE")).is_ok());
    }

    #[test]
    fn utf8_bom_flag_controls_the_signature() {
        assert_eq!(encode_for_save("x", fe("UTF-8")).unwrap(), b"x");
        assert_eq!(
            encode_for_save("x", fe("UTF-8 BOM")).unwrap(),
            vec![0xEF, 0xBB, 0xBF, b'x']
        );
    }

    #[test]
    fn decode_with_strips_only_a_matching_bom() {
        // A UTF-8 BOM in front of Windows-1252 bytes must NOT be stripped when the
        // forced encoding is Windows-1252 (the signature bytes belong to no legacy
        // BOM), so they decode as visible characters rather than vanishing.
        let bytes = [0xEF, 0xBB, 0xBF, b'x'];
        let (decoded, enc) = decode_with(&bytes, fe("Windows-1252"));
        assert_eq!(decoded.chars().count(), 4);
        assert_eq!(enc, fe("Windows-1252"));
    }

    #[test]
    fn decode_strict_accepts_valid_bytes_and_strips_bom() {
        // Round-trip a UTF-16 LE file (with BOM): a strict reopen succeeds and the
        // BOM is stripped, exactly like the lossy forced decode on good input.
        let text = "Hello, 世界!";
        let bytes = encode_for_save(text, fe("UTF-16 LE")).unwrap();
        let (decoded, enc) = decode_strict(&bytes, fe("UTF-16 LE")).unwrap();
        assert_eq!(decoded, text);
        assert_eq!(enc, fe("UTF-16 LE"));
    }

    #[test]
    fn decode_strict_blocks_invalid_utf8() {
        // A lone 0xFF is not a valid UTF-8 sequence, so a strict reopen as UTF-8 is
        // refused with an error naming the encoding — rather than showing U+FFFD.
        let err = decode_strict(&[b'h', b'i', 0xFF], fe("UTF-8")).unwrap_err();
        assert!(err.contains("UTF-8"), "error names the encoding: {err}");
        assert!(
            err.contains('1'),
            "error counts the one bad sequence: {err}"
        );
    }

    #[test]
    fn decode_strict_blocks_odd_length_utf16() {
        // An odd byte count can't be whole UTF-16 code units → blocked.
        assert!(decode_strict(&[0x41, 0x00, 0x42], fe("UTF-16 LE")).is_err());
    }

    #[test]
    fn decode_strict_never_blocks_a_total_single_byte_code_page() {
        // Every byte is a valid Windows-1252 character, so a strict reopen under it
        // always succeeds — even for arbitrary binary.
        let bytes: Vec<u8> = (0u8..=255).collect();
        assert!(decode_strict(&bytes, fe("Windows-1252")).is_ok());
    }

    #[test]
    fn arbitrary_binary_opens_without_error() {
        // A run of high bytes is not valid UTF-8; it must still decode to text.
        let bytes: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
        let (text, _) = decode(&bytes);
        assert!(!text.is_empty());
    }

    proptest! {
        /// `decode` never panics and always yields some text for any bytes.
        #[test]
        fn decode_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
            let _ = decode(&bytes);
            for label in options() {
                let want = FileEncoding::from_label(label).unwrap();
                let _ = decode_with(&bytes, want);
                let _ = decode_strict(&bytes, want); // Ok or Err, never a panic
            }
        }

        /// Any valid UTF-8 that is not itself a BOM-prefixed string round-trips
        /// byte-identically through the default (UTF-8, no BOM) path.
        #[test]
        fn valid_utf8_round_trips_byte_identically(s in ".{0,300}") {
            prop_assume!(!s.starts_with('\u{FEFF}'));
            let bytes = s.as_bytes();
            let (text, enc) = decode(bytes);
            prop_assert_eq!(&text, &s);
            prop_assert_eq!(enc, FileEncoding::default());
            let reencoded = encode_for_save(&text, enc).unwrap();
            prop_assert_eq!(reencoded, bytes.to_vec());
        }
    }
}
