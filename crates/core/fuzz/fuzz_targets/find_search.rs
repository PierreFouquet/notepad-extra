#![no_main]
//! Fuzz the Find / Replace engine (#33) — the "anything parsing/decoding" target
//! the epic's DoD calls for, since it compiles an arbitrary regex and searches
//! arbitrary text. For *any* pattern + options + text + replacement:
//!
//! * compiling never panics (invalid / empty patterns return `Err`);
//! * every reported match is an in-bounds range on UTF-8 char boundaries;
//! * navigation, go-to-line and replace never panic and stay in bounds.

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;
use notepad_core::find::{goto_line_offset, line_col_of, resume_after};
use notepad_core::{Match, Matcher, SearchOptions};

#[derive(Debug)]
struct Input {
    pattern: String,
    case_sensitive: bool,
    whole_word: bool,
    regex: bool,
    text: String,
    replacement: String,
    offset: usize,
}

// Hand-rolled rather than derived on the core types, so the core stays free of
// the `arbitrary` dependency.
impl<'a> Arbitrary<'a> for Input {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Input {
            pattern: String::arbitrary(u)?,
            case_sensitive: bool::arbitrary(u)?,
            whole_word: bool::arbitrary(u)?,
            regex: bool::arbitrary(u)?,
            text: String::arbitrary(u)?,
            replacement: String::arbitrary(u)?,
            offset: usize::arbitrary(u)?,
        })
    }
}

fn check(text: &str, m: Match) {
    assert!(m.start <= m.end);
    assert!(m.end <= text.len());
    assert!(text.is_char_boundary(m.start));
    assert!(text.is_char_boundary(m.end));
}

fuzz_target!(|input: Input| {
    // Go-to-line and offset→(line,col) are pure and defined for any input.
    let off = goto_line_offset(&input.text, input.offset);
    assert!(off <= input.text.len());
    assert!(input.text.is_char_boundary(off));
    let _ = line_col_of(&input.text, input.offset); // any offset clamps safely

    let options = SearchOptions {
        case_sensitive: input.case_sensitive,
        whole_word: input.whole_word,
        regex: input.regex,
    };
    // An empty or invalid pattern is a normal `Err`, not a crash.
    let Ok(m) = Matcher::new(&input.pattern, options) else {
        return;
    };

    for hit in m.find_all(&input.text) {
        check(&input.text, hit);
    }
    let _ = m.count(&input.text);

    if let Some(first) = m.find_from(&input.text, input.offset) {
        check(&input.text, first);
        let _ = resume_after(&input.text, first);
        let _ = m.ordinal_of(&input.text, first.start);
    }
    if let Some(last) = m.find_last(&input.text) {
        check(&input.text, last);
        let _ = m.find_last_before(&input.text, last.end);
    }

    let _ = m.replace_all(&input.text, &input.replacement);
    if let Some(rep) = m.replace_next(&input.text, off, &input.replacement) {
        check(&rep.text, rep.range);
    }
});
