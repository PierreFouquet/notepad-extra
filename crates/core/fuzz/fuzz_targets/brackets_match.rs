#![no_main]
//! Fuzz bracket matching (#41). For *any* text + caret offset, [`brackets::match_at`]
//! must never panic and must return only valid results:
//!
//! * `here` and `partner` land on UTF-8 char boundaries, in bounds;
//! * `here` and `partner` are actual bracket characters that form a real pair;
//! * matching is involutive — matching back from the partner returns `here`.

use arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;
use notepad_core::{BracketMatch, brackets};

#[derive(Debug)]
struct Input {
    text: String,
    caret: usize,
}

// Hand-rolled rather than derived on the core types, so the core stays free of
// the `arbitrary` dependency.
impl<'a> Arbitrary<'a> for Input {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(Input {
            text: String::arbitrary(u)?,
            caret: usize::arbitrary(u)?,
        })
    }
}

fn bracket_at(text: &str, off: usize) -> char {
    assert!(off < text.len());
    assert!(text.is_char_boundary(off));
    let ch = text[off..].chars().next().expect("a character at the offset");
    assert!(
        matches!(ch, '(' | ')' | '[' | ']' | '{' | '}'),
        "reported offset {off} holds {ch:?}, not a bracket"
    );
    ch
}

fn pairs(open: char, close: char) -> bool {
    matches!((open, close), ('(', ')') | ('[', ']') | ('{', '}'))
}

fuzz_target!(|input: Input| {
    let Input { text, caret } = input;

    let Some(m) = brackets::match_at(&text, caret) else {
        return;
    };

    let here = bracket_at(&text, m.here);
    if let Some(p) = m.partner {
        let partner = bracket_at(&text, p);
        // One is an opener and the other its exact closer, in either order.
        assert!(
            pairs(here, partner) || pairs(partner, here),
            "{here:?} and {partner:?} are not a matching pair"
        );
        // Matching back from just past the partner returns to `here`.
        assert_eq!(
            brackets::match_at(&text, p + 1),
            Some(BracketMatch {
                here: p,
                partner: Some(m.here),
            }),
        );
    }
});
