//! The editing model for issue #30: a single-span edit representation, a
//! minimal `diff` between two document versions, and an undo/redo [`History`]
//! with a memory-bounded, coalescing stack.
//!
//! Nothing here touches a toolkit. The render shell reports the editor's new
//! full text as [`crate::Message::Edited`]; the core [`diff`]s it against the
//! stored content and records the delta, so a 1M-line paste costs **one** small
//! [`Edit`] rather than a per-keystroke snapshot. Undo/redo return the [`Edit`]
//! to apply back to the buffer, keeping the whole thing pure and testable.

use std::collections::VecDeque;

/// A single contiguous replacement: the bytes `content[at..at + removed.len()]`
/// become `inserted`. This is the only shape an edit can take, because the shell
/// hands us whole-buffer text and [`diff`] reduces each change to one span.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// Byte offset into the (canonical `\n`) content. Always a char boundary.
    pub at: usize,
    /// Text that used to occupy `at..at + removed.len()`.
    pub removed: String,
    /// Text that now occupies `at..at + inserted.len()`.
    pub inserted: String,
}

impl Edit {
    /// Apply this edit to `content` in place. Panics only on misuse (an edit
    /// applied to content it wasn't derived from); the core never does that.
    pub fn apply(&self, content: &mut String) {
        content.replace_range(self.at..self.at + self.removed.len(), &self.inserted);
    }

    /// The inverse edit: applying this then its inverse is a no-op.
    pub fn invert(&self) -> Edit {
        Edit {
            at: self.at,
            removed: self.inserted.clone(),
            inserted: self.removed.clone(),
        }
    }
}

/// Reduce the change from `old` to `new` to a single minimal span by stripping
/// the common prefix and suffix. Returns `None` when the strings are equal
/// (a cursor move produces no edit). The result is always char-boundary aligned,
/// so it never splits a UTF-8 codepoint even when bytes happen to match mid-char.
///
/// The two scans compare the bytes in blocks ([`common_prefix`],
/// [`common_suffix`]) rather than one at a time: the widget hands the core the
/// whole buffer on every keystroke, so this runs over the entire document per
/// edit and was 75–80 % of the core's per-keystroke cost on a big file. A
/// byte loop ran 6–9× slower than a plain `memcmp` on 13 MB (release: 6.9 ms
/// for a change at the end, 11.4 ms at the start, against 1.1 ms for `==`),
/// and the blockwise form brings both scans down to that `memcmp` figure (#126).
/// There is no up-front `old == new`: a common prefix spanning two equal-length
/// strings *is* equality, so the first scan answers that question for free.
pub fn diff(old: &str, new: &str) -> Option<Edit> {
    let (ob, nb) = (old.as_bytes(), new.as_bytes());

    // Longest common prefix, in bytes, backed off to a char boundary.
    let max_pre = ob.len().min(nb.len());
    let mut p = common_prefix(&ob[..max_pre], &nb[..max_pre]);
    if p == max_pre && ob.len() == nb.len() {
        return None;
    }
    while p > 0 && !old.is_char_boundary(p) {
        p -= 1;
    }

    // Longest common suffix that doesn't overlap the prefix, char-aligned in
    // both strings (the trailing bytes are shared, but the *start* of the suffix
    // could land inside a multi-byte char).
    let max_suf = (ob.len() - p).min(nb.len() - p);
    let mut s = common_suffix(&ob[ob.len() - max_suf..], &nb[nb.len() - max_suf..]);
    while s > 0 && (!old.is_char_boundary(old.len() - s) || !new.is_char_boundary(new.len() - s)) {
        s -= 1;
    }

    Some(Edit {
        at: p,
        removed: old[p..old.len() - s].to_string(),
        inserted: new[p..new.len() - s].to_string(),
    })
}

/// Block sizes for [`common_prefix`] / [`common_suffix`], coarse to fine. A
/// slice compare of each block is one `memcmp` call (`<[u8]>::eq` is
/// specialised to it), so the scan runs at memory speed until the first block
/// that differs, then narrows inside that block with the next size down, and
/// finishes byte by byte within the last 64 (#126). The 4 KiB coarse block
/// keeps the call count small (one per page) while bounding the re-scan of the
/// differing block to a trivial 4 KiB.
const COARSE_BLOCK: usize = 4096;
const FINE_BLOCK: usize = 64;

/// Length of the longest common prefix of two equal-length byte slices.
fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    debug_assert_eq!(a.len(), b.len());
    let mut p = 0;
    for block in [COARSE_BLOCK, FINE_BLOCK] {
        while p + block <= a.len() && a[p..p + block] == b[p..p + block] {
            p += block;
        }
    }
    while p < a.len() && a[p] == b[p] {
        p += 1;
    }
    p
}

/// Length of the longest common suffix of two equal-length byte slices: the
/// mirror of [`common_prefix`], walking in from the end.
fn common_suffix(a: &[u8], b: &[u8]) -> usize {
    debug_assert_eq!(a.len(), b.len());
    let n = a.len();
    let mut s = 0;
    for block in [COARSE_BLOCK, FINE_BLOCK] {
        while s + block <= n && a[n - s - block..n - s] == b[n - s - block..n - s] {
            s += block;
        }
    }
    while s < n && a[n - 1 - s] == b[n - 1 - s] {
        s += 1;
    }
    s
}

/// Upper bound on undo depth. Past this the oldest entries are dropped so a
/// long editing session can't grow the history without limit (epic #25's
/// "memory soak under sustained editing").
const MAX_UNDO: usize = 500;

/// One committed history step: an [`Edit`] plus a never-reused id. The id lets
/// the dirty marker name a stack position that survives coalescing (which
/// mutates an entry in place) but changes when a genuinely new step is pushed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    edit: Edit,
    id: u64,
}

/// Where the on-disk ("saved") content sits relative to the undo stack.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Clean {
    /// Saved with an empty undo stack (a fresh or freshly-loaded buffer).
    Empty,
    /// Saved when the entry with this id was on top of the undo stack.
    At(u64),
    /// The saved state is no longer reachable (it was evicted by the depth cap).
    Gone,
}

/// The undo/redo history for one [`crate::Document`]. It tracks where the
/// saved state sits in the stack ([`History::dirty`]), but the unsaved-changes
/// ("•") flag is [`crate::Document::dirty`], which compares content, not stack
/// position, so typing a change and deleting it again reads as clean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    /// Oldest at the front, newest at the back. A deque, not a `Vec`, because
    /// the depth cap evicts from the *front*: once a long session sits at
    /// [`MAX_UNDO`], every keystroke evicts one entry, and `Vec::drain(0..1)`
    /// shifted the whole stack down each time — O(depth) per record. `pop_front`
    /// is O(1) (#126; measured 5× faster for records at the cap).
    undo: VecDeque<Entry>,
    /// Only ever pushed, popped and cleared at the back, so a `Vec` is right.
    redo: Vec<Entry>,
    clean: Clean,
    next_id: u64,
    /// Whether the top entry may absorb the next adjacent keystroke. Sealed by
    /// undo/redo, save, deletes, and newlines so those start a fresh undo step.
    coalescible: bool,
}

impl Default for History {
    fn default() -> Self {
        History {
            undo: VecDeque::new(),
            redo: Vec::new(),
            clean: Clean::Empty,
            next_id: 0,
            coalescible: false,
        }
    }
}

impl History {
    /// A fresh, clean history (the current content is the saved baseline).
    pub fn new() -> Self {
        History::default()
    }

    /// Are there unsaved changes? True unless the undo stack is exactly at the
    /// position it held when the buffer was last saved.
    pub fn dirty(&self) -> bool {
        match self.clean {
            Clean::Gone => true,
            Clean::Empty => !self.undo.is_empty(),
            Clean::At(id) => self.undo.back().map(|e| e.id) != Some(id),
        }
    }

    /// Mark the current content as saved: the present stack position becomes the
    /// clean baseline, so undoing back to it later will clear the dirty flag.
    pub fn mark_saved(&mut self) {
        self.clean = match self.undo.back() {
            Some(e) => Clean::At(e.id),
            None => Clean::Empty,
        };
        self.coalescible = false;
    }

    /// Record a new edit. Invalidates the redo stack, coalesces adjacent typing
    /// into the previous step where sensible, and enforces the depth cap.
    pub fn record(&mut self, edit: Edit) {
        self.redo.clear();

        // A pure, newline-free insertion can start/continue a typing run.
        // Decided once here: it gates coalescing *and* seeds `coalescible`.
        let typing = edit.removed.is_empty() && !edit.inserted.contains('\n');
        match self.coalescible_top(&edit, typing) {
            // Append in place — the new text is moved onto the open step, with
            // no clone of either side.
            Some(top) => top.edit.inserted.push_str(&edit.inserted),
            // Still an open run of typing; `coalescible` stays set.
            None => {
                let id = self.next_id;
                self.next_id += 1;
                self.coalescible = typing;
                self.undo.push_back(Entry { edit, id });
            }
        }

        self.enforce_cap();
    }

    /// Undo the most recent step, returning the [`Edit`] the caller must apply
    /// to the buffer to revert it. `None` when there is nothing to undo.
    pub fn undo(&mut self) -> Option<Edit> {
        let entry = self.undo.pop_back()?;
        let inverse = entry.edit.invert();
        self.redo.push(entry);
        self.coalescible = false;
        Some(inverse)
    }

    /// Redo the most recently undone step, returning the [`Edit`] to re-apply.
    /// `None` when there is nothing to redo.
    pub fn redo(&mut self) -> Option<Edit> {
        let entry = self.redo.pop()?;
        let forward = entry.edit.clone();
        self.undo.push_back(entry);
        self.coalescible = false;
        Some(forward)
    }

    /// The top entry `edit` should merge into instead of becoming a new step —
    /// or `None` to push. Merging needs an open typing run, both sides pure
    /// newline-free insertions (`typing`, computed by the caller), textual
    /// adjacency, and the top not sitting on the saved boundary (merging there
    /// would silently keep the buffer "clean" after an edit).
    fn coalescible_top(&mut self, edit: &Edit, typing: bool) -> Option<&mut Entry> {
        if !self.coalescible || !typing {
            return None;
        }
        let clean = self.clean;
        self.undo.back_mut().filter(|top| {
            top.edit.removed.is_empty()
                && edit.at == top.edit.at + top.edit.inserted.len()
                && !matches!(clean, Clean::At(id) if id == top.id)
        })
    }

    /// Drop the oldest entries once the stack exceeds [`MAX_UNDO`]. If the saved
    /// baseline is among the evicted entries it becomes unreachable. Each
    /// eviction is an O(1) `pop_front` (#126) — `record` pushes one entry, so
    /// at the cap this loop runs exactly once per keystroke.
    fn enforce_cap(&mut self) {
        while self.undo.len() > MAX_UNDO
            && let Some(evicted) = self.undo.pop_front()
        {
            match self.clean {
                // The empty-stack baseline is now behind us.
                Clean::Empty => self.clean = Clean::Gone,
                Clean::At(id) if id == evicted.id => self.clean = Clean::Gone,
                Clean::At(_) | Clean::Gone => {}
            }
        }
    }

    /// Current undo depth (used by tests and stress assertions).
    #[cfg(test)]
    pub(crate) fn undo_depth(&self) -> usize {
        self.undo.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    // ---- diff ----

    /// The byte-at-a-time `diff` the blockwise scans replaced (#126), kept
    /// verbatim as the oracle: the new function must return exactly what this
    /// one does for every input, including the char-boundary back-offs and the
    /// no-overlap cap on the suffix.
    fn reference_diff(old: &str, new: &str) -> Option<Edit> {
        if old == new {
            return None;
        }
        let (ob, nb) = (old.as_bytes(), new.as_bytes());

        let max_pre = ob.len().min(nb.len());
        let mut p = 0;
        while p < max_pre && ob[p] == nb[p] {
            p += 1;
        }
        while p > 0 && !old.is_char_boundary(p) {
            p -= 1;
        }

        let max_suf = (ob.len() - p).min(nb.len() - p);
        let mut s = 0;
        while s < max_suf && ob[ob.len() - 1 - s] == nb[nb.len() - 1 - s] {
            s += 1;
        }
        while s > 0
            && (!old.is_char_boundary(old.len() - s) || !new.is_char_boundary(new.len() - s))
        {
            s -= 1;
        }

        Some(Edit {
            at: p,
            removed: old[p..old.len() - s].to_string(),
            inserted: new[p..new.len() - s].to_string(),
        })
    }

    /// `unit` repeated until the result is at least `target` bytes long (empty
    /// for an empty unit), so a shared prefix or suffix can be stretched across
    /// the coarse and fine block sizes without hand-writing kilobyte literals.
    fn stretch(unit: &str, target: usize) -> String {
        unit.repeat(target / unit.len().max(1) + 1)
    }

    /// Pieces mixing ASCII with 2-, 3- and 4-byte chars — including the `é` /
    /// `©` pair whose trailing bytes agree while the chars differ, which is what
    /// drives the suffix scan into its mid-char back-off.
    fn seam_text(max: usize) -> impl Strategy<Value = String> {
        prop::collection::vec(
            prop_oneof![
                Just("a"),
                Just("b"),
                Just("\n"),
                Just("é"),
                Just("©"),
                Just("你"),
                Just("😀"),
            ],
            0..max,
        )
        .prop_map(|parts| parts.concat())
    }

    /// Lengths that put a shared prefix or suffix on either side of, and exactly
    /// on, the 64-byte and 4 KiB block sizes.
    fn block_straddling_len() -> impl Strategy<Value = usize> {
        prop_oneof![Just(0usize), 0usize..130, 4000usize..4200, 8100usize..8300,]
    }

    proptest! {
        /// The blockwise scans return exactly what the byte loop did, for any
        /// pair of strings: equal or empty, sharing nothing, or sharing a prefix
        /// and suffix of any length around the block sizes, with the change
        /// itself made of multi-byte chars whose bytes partially coincide (#126).
        #[test]
        fn chunked_diff_matches_the_reference(
            pre_unit in seam_text(6),
            pre_len in block_straddling_len(),
            old_mid in seam_text(6),
            new_mid in seam_text(6),
            suf_unit in seam_text(6),
            suf_len in block_straddling_len(),
        ) {
            let pre = stretch(&pre_unit, pre_len);
            let suf = stretch(&suf_unit, suf_len);
            let old = format!("{pre}{old_mid}{suf}");
            let new = format!("{pre}{new_mid}{suf}");
            let edit = diff(&old, &new);
            prop_assert_eq!(&edit, &reference_diff(&old, &new));
            prop_assert_eq!(diff(&new, &old), reference_diff(&new, &old));
            prop_assert_eq!(diff(&old, &old), None);
            prop_assert_eq!(diff(&new, &new), None);
            // `None` means equal, and an edit applied to `old` reproduces `new`
            // (with no edit, `buf` is `old`, which must then already be `new`).
            prop_assert_eq!(edit.is_none(), old == new);
            let mut buf = old.clone();
            if let Some(edit) = &edit {
                edit.apply(&mut buf);
            }
            prop_assert_eq!(buf, new);
        }
    }

    #[test]
    fn the_reference_diff_agrees_with_diff_on_every_hand_written_case() {
        // The oracle itself must take every branch deterministically — the
        // equal-strings early return and both char-boundary back-offs — not only
        // when the proptest happens to generate such a pair, or the coverage gate
        // flakes with the random seed. These are the cases the unit tests above
        // pin for `diff`, run through both functions side by side.
        for (old, new) in [
            ("", ""),
            ("same", "same"),
            ("ab", "abc"),
            ("b", "ab"),
            ("cat", "carat"),
            ("abc", "ac"),
            ("cat", "cut"),
            ("abcXbc", "abc"),
            ("ab", "abab"),
            ("a😀b", "a🎉b"), // prefix back-off: the emoji share their first bytes
            ("café", "cafe"),
            ("e", "e\u{0301}"),
            ("é", "©"), // suffix back-off: trailing bytes agree, chars differ
            ("aéaa", "aèaa"),
            ("line1\nline2", "line1\nCHANGED\nline2"),
        ] {
            assert_eq!(
                diff(old, new),
                reference_diff(old, new),
                "{old:?} -> {new:?}"
            );
            assert_eq!(
                diff(new, old),
                reference_diff(new, old),
                "{new:?} -> {old:?}"
            );
        }
    }

    #[test]
    fn diff_narrows_to_the_byte_at_every_block_boundary() {
        // One changed byte, placed first, in the middle and last, in buffers whose
        // length sits on, just under and just over the 64-byte and 4 KiB blocks
        // (and a multiple of both). The expected edit is the single char, so a
        // scan that overshoots a block, stops a block early or mis-sizes the
        // final partial block is caught here rather than by the proptest's luck.
        for len in [
            1, 2, 63, 64, 65, 127, 128, 129, 4095, 4096, 4097, 4159, 4160, 4161, 8191, 8192, 8193,
            12_289,
        ] {
            let base: String = (0..len)
                .map(|i| char::from(b'a' + (i % 26) as u8))
                .collect();
            assert_eq!(diff(&base, &base), None, "len {len}: equal is None");
            for at in [0, len / 2, len - 1] {
                let mut changed = base.clone();
                changed.replace_range(at..at + 1, "Z");
                let expected = Some(Edit {
                    at,
                    removed: base[at..at + 1].to_string(),
                    inserted: "Z".into(),
                });
                assert_eq!(diff(&base, &changed), expected, "len {len}, change at {at}");
                assert_eq!(diff(&changed, &base), expected.map(|e| e.invert()));
                assert_eq!(
                    diff(&base, &changed),
                    reference_diff(&base, &changed),
                    "len {len}, change at {at}: disagrees with the byte loop"
                );
            }
            // A pure insertion and a pure deletion straddling the block seam, so
            // the two buffers have *different* lengths and the suffix scan starts
            // from different offsets in each.
            let mut grown = base.clone();
            grown.insert_str(len / 2, "XY");
            assert_eq!(
                diff(&base, &grown),
                Some(Edit {
                    at: len / 2,
                    removed: String::new(),
                    inserted: "XY".into(),
                }),
                "len {len}: insertion"
            );
            assert_eq!(
                diff(&grown, &base),
                reference_diff(&grown, &base),
                "len {len}: deletion"
            );
        }
    }

    #[test]
    fn diff_of_equal_is_none() {
        assert_eq!(diff("same", "same"), None);
        assert_eq!(diff("", ""), None);
    }

    #[test]
    fn diff_pure_insertion_at_end() {
        assert_eq!(
            diff("ab", "abc"),
            Some(Edit {
                at: 2,
                removed: String::new(),
                inserted: "c".into()
            })
        );
    }

    #[test]
    fn diff_insertion_at_start_and_middle() {
        assert_eq!(
            diff("b", "ab"),
            Some(Edit {
                at: 0,
                removed: String::new(),
                inserted: "a".into()
            })
        );
        assert_eq!(
            diff("cat", "carat"),
            Some(Edit {
                at: 2,
                removed: String::new(),
                inserted: "ra".into()
            })
        );
    }

    #[test]
    fn diff_deletion_and_replacement() {
        assert_eq!(
            diff("abc", "ac"),
            Some(Edit {
                at: 1,
                removed: "b".into(),
                inserted: String::new()
            })
        );
        assert_eq!(
            diff("cat", "cut"),
            Some(Edit {
                at: 1,
                removed: "a".into(),
                inserted: "u".into()
            })
        );
    }

    #[test]
    fn diff_suffix_never_overlaps_the_prefix() {
        // The deleted text repeats the tail that survives it, so the common suffix
        // would happily run back *through* the common prefix if it were not capped
        // by it: "abc|Xbc" -> "abc" shares the prefix "abc" and the trailing "bc".
        // Without the cap the suffix eats into text the prefix already claimed and
        // the two spans cross, which is not merely a wrong edit — it slices
        // `new[3..1]` and panics.
        assert_eq!(
            diff("abcXbc", "abc"),
            Some(Edit {
                at: 3,
                removed: "Xbc".into(),
                inserted: String::new()
            })
        );
        // The mirror case: a pure insertion whose new text repeats the prefix.
        assert_eq!(
            diff("ab", "abab"),
            Some(Edit {
                at: 2,
                removed: String::new(),
                inserted: "ab".into()
            })
        );
    }

    #[test]
    fn diff_respects_utf8_boundaries() {
        // Emoji differ only in their middle bytes; the span must be the whole char.
        let d = diff("a😀b", "a🎉b").unwrap();
        assert_eq!(
            d,
            Edit {
                at: 1,
                removed: "😀".into(),
                inserted: "🎉".into()
            }
        );
        // é (2 bytes) -> e (1 byte): trailing bytes differ, no false suffix match.
        let d = diff("café", "cafe").unwrap();
        assert_eq!(
            d,
            Edit {
                at: 3,
                removed: "é".into(),
                inserted: "e".into()
            }
        );
        // Combining mark appended.
        let d = diff("e", "e\u{0301}").unwrap();
        assert_eq!(d.at, 1);
        assert_eq!(d.inserted, "\u{0301}");
    }

    #[test]
    fn diff_backs_off_a_suffix_that_matches_mid_char() {
        // The cases above all stop the byte-level suffix scan on a boundary already,
        // so none of them ever enter the char-boundary back-off — the loop this
        // function's doc comment is about ("even when bytes happen to match
        // mid-char"). These two do.
        //
        // 'é' is C3 A9 and '©' is C2 A9: the *trailing* byte matches although the
        // chars differ, so the raw suffix scan stops inside both chars and must be
        // backed off to 0 rather than splitting either one.
        assert_eq!("é".as_bytes(), &[0xC3, 0xA9]);
        assert_eq!("©".as_bytes(), &[0xC2, 0xA9]);
        assert_eq!(
            diff("é", "©"),
            Some(Edit {
                at: 0,
                removed: "é".into(),
                inserted: "©".into()
            }),
            "a suffix matching only mid-char must not shrink the span"
        );

        // 'é' (C3 A9) vs 'è' (C3 A8) inside a longer string: the common suffix "aa"
        // is real and must be *kept* — the back-off must not walk past a boundary it
        // has already reached, or the edit span grows to swallow the trailing text.
        assert_eq!(
            diff("aéaa", "aèaa"),
            Some(Edit {
                at: 1,
                removed: "é".into(),
                inserted: "è".into()
            }),
            "a genuine char-aligned suffix is preserved"
        );
    }

    #[test]
    fn apply_then_invert_round_trips() {
        for (old, new) in [
            ("", "hello"),
            ("hello", ""),
            ("cat", "cut"),
            ("a😀b", "a🎉bc"),
            ("line1\nline2", "line1\nCHANGED\nline2"),
        ] {
            let edit = diff(old, new).unwrap();
            let mut buf = old.to_string();
            edit.apply(&mut buf);
            assert_eq!(buf, new, "forward apply");
            edit.invert().apply(&mut buf);
            assert_eq!(buf, old, "inverse apply");
        }
    }

    // ---- History: undo / redo ----

    /// Drive text through a history the way the core does: diff old->new, record,
    /// and keep the running content.
    fn type_into(h: &mut History, content: &mut String, new: &str) {
        if let Some(edit) = diff(content, new) {
            h.record(edit);
            *content = new.to_string();
        }
    }

    fn undo(h: &mut History, content: &mut String) -> bool {
        match h.undo() {
            Some(edit) => {
                edit.apply(content);
                true
            }
            None => false,
        }
    }

    fn redo(h: &mut History, content: &mut String) -> bool {
        match h.redo() {
            Some(edit) => {
                edit.apply(content);
                true
            }
            None => false,
        }
    }

    #[test]
    fn undo_redo_walks_content_back_and_forth() {
        let mut h = History::new();
        let mut c = String::new();
        type_into(&mut h, &mut c, "a");
        type_into(&mut h, &mut c, "ab");
        // Break the typing run so we get two undo steps.
        h.coalescible = false;
        type_into(&mut h, &mut c, "ab\n");
        type_into(&mut h, &mut c, "ab\nx");

        assert_eq!(c, "ab\nx");
        assert!(undo(&mut h, &mut c));
        assert_eq!(c, "ab\n");
        assert!(undo(&mut h, &mut c));
        assert_eq!(c, "ab");
        assert!(redo(&mut h, &mut c));
        assert_eq!(c, "ab\n");
        assert!(redo(&mut h, &mut c));
        assert_eq!(c, "ab\nx");
    }

    #[test]
    fn undo_and_redo_on_empty_stacks_are_noops() {
        let mut h = History::new();
        let mut c = String::new();
        assert!(!undo(&mut h, &mut c));
        assert!(!redo(&mut h, &mut c));
        assert_eq!(c, "");
    }

    #[test]
    fn adjacent_typing_coalesces_into_one_step() {
        let mut h = History::new();
        let mut c = String::new();
        for word in ["h", "he", "hel", "hell", "hello"] {
            type_into(&mut h, &mut c, word);
        }
        assert_eq!(h.undo_depth(), 1, "one continuous run is one undo step");
        assert!(undo(&mut h, &mut c));
        assert_eq!(c, "", "a single undo removes the whole word");
    }

    #[test]
    fn a_newline_breaks_the_typing_run() {
        let mut h = History::new();
        let mut c = String::new();
        type_into(&mut h, &mut c, "hi");
        type_into(&mut h, &mut c, "hi\n"); // newline seals the run
        type_into(&mut h, &mut c, "hi\nx");
        assert_eq!(h.undo_depth(), 3);
    }

    #[test]
    fn a_deletion_is_its_own_step_and_seals_typing() {
        let mut h = History::new();
        let mut c = String::new();
        type_into(&mut h, &mut c, "abc");
        type_into(&mut h, &mut c, "ab"); // delete 'c'
        type_into(&mut h, &mut c, "abd"); // type again -> new step
        assert_eq!(h.undo_depth(), 3);
    }

    #[test]
    fn a_new_edit_after_undo_clears_the_redo_future() {
        let mut h = History::new();
        let mut c = String::new();
        type_into(&mut h, &mut c, "a");
        h.coalescible = false;
        type_into(&mut h, &mut c, "ab");
        undo(&mut h, &mut c); // back to "a", redo has "ab"
        assert_eq!(c, "a");
        type_into(&mut h, &mut c, "aX"); // diverge
        assert!(!redo(&mut h, &mut c), "redo future must be gone");
        assert_eq!(c, "aX");
    }

    // ---- History: dirty tracking ----

    #[test]
    fn fresh_history_is_clean_and_edits_dirty_it() {
        let mut h = History::new();
        let mut c = String::new();
        assert!(!h.dirty());
        type_into(&mut h, &mut c, "x");
        assert!(h.dirty());
    }

    #[test]
    fn undoing_back_to_a_saved_point_clears_dirty() {
        let mut h = History::new();
        let mut c = String::new();
        type_into(&mut h, &mut c, "hello");
        h.mark_saved(); // saved as "hello"
        assert!(!h.dirty());

        h.coalescible = false;
        type_into(&mut h, &mut c, "hello!");
        assert!(h.dirty());
        undo(&mut h, &mut c); // back to the saved "hello"
        assert_eq!(c, "hello");
        assert!(!h.dirty(), "returning to the saved state is clean again");

        redo(&mut h, &mut c);
        assert!(h.dirty());
    }

    #[test]
    fn editing_past_a_saved_point_that_was_undone_stays_dirty() {
        let mut h = History::new();
        let mut c = String::new();
        type_into(&mut h, &mut c, "a");
        h.coalescible = false;
        type_into(&mut h, &mut c, "ab");
        h.mark_saved(); // saved as "ab"
        undo(&mut h, &mut c); // "a", dirty (saved state is in redo)
        assert!(h.dirty());
        type_into(&mut h, &mut c, "aZ"); // diverge; "ab" is now unreachable
        assert!(h.dirty(), "the saved state can no longer be reached");
    }

    #[test]
    fn typing_after_save_does_not_coalesce_into_the_saved_step() {
        let mut h = History::new();
        let mut c = String::new();
        type_into(&mut h, &mut c, "ab");
        h.mark_saved();
        type_into(&mut h, &mut c, "abc"); // adjacent, but must not merge across save
        assert!(h.dirty());
        undo(&mut h, &mut c);
        assert_eq!(c, "ab");
        assert!(!h.dirty(), "the saved 'ab' is intact and reachable");
    }

    #[test]
    fn coalescing_into_an_unsaved_top_step_ignores_a_lower_saved_baseline() {
        // The saved-boundary guard only blocks merging into the *saved* entry
        // itself. With a saved baseline sitting below a newer (unsaved) top step,
        // adjacent typing must still coalesce into that top step.
        let mut h = History::new();
        let mut c = String::new();
        type_into(&mut h, &mut c, "a");
        h.mark_saved(); // clean = At("a")
        h.coalescible = false; // seal so the next type starts a fresh top step
        type_into(&mut h, &mut c, "ab"); // new top step; the saved baseline is below it
        type_into(&mut h, &mut c, "abc"); // adjacent -> coalesces into the "ab" top
        assert_eq!(
            h.undo_depth(),
            2,
            "the two post-save types collapse into one step above the saved one"
        );
        undo(&mut h, &mut c);
        assert_eq!(c, "a");
        assert!(!h.dirty(), "one undo lands back on the saved baseline");
    }

    #[test]
    fn coalescing_appends_in_place_and_keeps_the_step_id() {
        // The merge path extends the open step's text without replacing the
        // entry: the same id stays on top (so a saved marker pinned to it still
        // matches) and the inserted text grows by exactly the new keystroke.
        let mut h = History::new();
        let mut c = String::new();
        type_into(&mut h, &mut c, "a");
        let id = h.undo.back().expect("one step").id;
        type_into(&mut h, &mut c, "ab");
        type_into(&mut h, &mut c, "abc");
        let top = h.undo.back().expect("still one step");
        assert_eq!((top.id, top.edit.inserted.as_str()), (id, "abc"));
        assert_eq!(h.undo_depth(), 1);
        // Non-adjacent typing (a gap at the start) must not merge, even mid-run.
        type_into(&mut h, &mut c, "Xabc");
        assert_eq!(h.undo_depth(), 2);
    }

    // ---- History: depth cap ----

    #[test]
    fn depth_cap_bounds_the_undo_stack() {
        let mut h = History::new();
        let mut c = String::new();
        // Each newline-terminated append is its own (sealed) step.
        for i in 0..(MAX_UNDO + 200) {
            let next = format!("{c}{i}\n");
            type_into(&mut h, &mut c, &next);
        }
        assert_eq!(h.undo_depth(), MAX_UNDO, "history is bounded");
        // The *oldest* steps were the ones evicted: undoing everything that is
        // left walks back exactly MAX_UNDO steps and stops on the first surviving
        // line, never reaching the evicted start of the session.
        let mut undone = 0;
        while undo(&mut h, &mut c) {
            undone += 1;
        }
        assert_eq!(undone, MAX_UNDO);
        assert!(c.starts_with("0\n") && c.ends_with("199\n"), "{c:?}");
        assert_eq!(c.lines().count(), 200, "the 200 evicted steps stay applied");
    }

    #[test]
    fn evicting_the_saved_baseline_marks_it_gone() {
        let mut h = History::new();
        let mut c = String::new();
        h.mark_saved(); // clean at the empty baseline
        for i in 0..(MAX_UNDO + 5) {
            let next = format!("{c}{i}\n");
            type_into(&mut h, &mut c, &next);
        }
        // The empty baseline fell off the bottom: no amount of undo returns clean.
        assert!(h.dirty());
        while undo(&mut h, &mut c) {}
        assert!(h.dirty(), "the original saved state is unreachable");
    }

    #[test]
    fn evicting_a_saved_entry_marks_it_gone() {
        // Saving *after* a real edit pins the clean baseline to that entry (unlike
        // the empty-baseline case above, which is `Clean::Empty`). When the depth
        // cap later drains that entry off the bottom, the saved state is
        // unreachable and the buffer stays dirty for good.
        let mut h = History::new();
        let mut c = String::new();
        type_into(&mut h, &mut c, "0\n"); // a first sealed step
        h.mark_saved(); // clean = At(that entry), not Empty
        assert!(!h.dirty());
        for i in 1..=(MAX_UNDO + 1) {
            let next = format!("{c}{i}\n");
            type_into(&mut h, &mut c, &next);
        }
        assert_eq!(h.undo_depth(), MAX_UNDO, "history stays bounded");
        assert!(
            h.dirty(),
            "the evicted saved entry can never be reached again"
        );
    }

    #[test]
    fn evicting_an_unrelated_entry_leaves_the_saved_baseline_reachable() {
        // The two tests above only evict the baseline *itself*, where "is the
        // baseline among the evicted?" is true — so they pass just as well if that
        // test is inverted. This is the other side: the evicted entry is a stranger,
        // the baseline must survive, and undoing back to it must come up clean.
        let mut h = History::new();
        let mut c = String::new();
        for i in 0..MAX_UNDO {
            let next = format!("{c}{i}\n");
            type_into(&mut h, &mut c, &next);
        }
        assert_eq!(h.undo_depth(), MAX_UNDO, "stack filled to the cap");

        h.mark_saved(); // baseline pinned at the newest entry, far from the bottom
        assert!(!h.dirty());

        // One more edit overflows by one, evicting the *oldest* entry — not the
        // baseline, which is still sitting one step below the top.
        let next = format!("{c}x\n");
        type_into(&mut h, &mut c, &next);
        assert_eq!(h.undo_depth(), MAX_UNDO, "history stays bounded");
        assert!(h.dirty(), "the newest edit is unsaved");

        assert!(undo(&mut h, &mut c));
        assert!(
            !h.dirty(),
            "an unrelated eviction must not strand the saved baseline"
        );
    }
}
