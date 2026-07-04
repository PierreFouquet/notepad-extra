//! The editing model for issue #30: a single-span edit representation, a
//! minimal `diff` between two document versions, and an undo/redo [`History`]
//! with a memory-bounded, coalescing stack.
//!
//! Nothing here touches a toolkit. The render shell reports the editor's new
//! full text as [`crate::Message::Edited`]; the core [`diff`]s it against the
//! stored content and records the delta, so a 1M-line paste costs **one** small
//! [`Edit`] rather than a per-keystroke snapshot. Undo/redo return the [`Edit`]
//! to apply back to the buffer, keeping the whole thing pure and testable.

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
pub fn diff(old: &str, new: &str) -> Option<Edit> {
    if old == new {
        return None;
    }
    let (ob, nb) = (old.as_bytes(), new.as_bytes());

    // Longest common prefix, in bytes, backed off to a char boundary.
    let max_pre = ob.len().min(nb.len());
    let mut p = 0;
    while p < max_pre && ob[p] == nb[p] {
        p += 1;
    }
    while p > 0 && !old.is_char_boundary(p) {
        p -= 1;
    }

    // Longest common suffix that doesn't overlap the prefix, char-aligned in
    // both strings (the trailing bytes are shared, but the *start* of the suffix
    // could land inside a multi-byte char).
    let max_suf = (ob.len() - p).min(nb.len() - p);
    let mut s = 0;
    while s < max_suf && ob[ob.len() - 1 - s] == nb[nb.len() - 1 - s] {
        s += 1;
    }
    while s > 0 && (!old.is_char_boundary(old.len() - s) || !new.is_char_boundary(new.len() - s)) {
        s -= 1;
    }

    Some(Edit {
        at: p,
        removed: old[p..old.len() - s].to_string(),
        inserted: new[p..new.len() - s].to_string(),
    })
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

/// The undo/redo history for one [`crate::Document`]. Also the single source of
/// truth for the unsaved-changes ("•") flag via [`History::dirty`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    undo: Vec<Entry>,
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
            undo: Vec::new(),
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
            Clean::At(id) => self.undo.last().map(|e| e.id) != Some(id),
        }
    }

    /// Mark the current content as saved: the present stack position becomes the
    /// clean baseline, so undoing back to it later will clear the dirty flag.
    pub fn mark_saved(&mut self) {
        self.clean = match self.undo.last() {
            Some(e) => Clean::At(e.id),
            None => Clean::Empty,
        };
        self.coalescible = false;
    }

    /// Record a new edit. Invalidates the redo stack, coalesces adjacent typing
    /// into the previous step where sensible, and enforces the depth cap.
    pub fn record(&mut self, edit: Edit) {
        self.redo.clear();

        if self.can_coalesce(&edit) {
            self.undo
                .last_mut()
                .unwrap()
                .edit
                .inserted
                .push_str(&edit.inserted);
            // Still an open run of typing; leave `coalescible` set.
        } else {
            let id = self.next_id;
            self.next_id += 1;
            // A pure, newline-free insertion can start/continue a typing run.
            self.coalescible = edit.removed.is_empty() && !edit.inserted.contains('\n');
            self.undo.push(Entry { edit, id });
        }

        self.enforce_cap();
    }

    /// Undo the most recent step, returning the [`Edit`] the caller must apply
    /// to the buffer to revert it. `None` when there is nothing to undo.
    pub fn undo(&mut self) -> Option<Edit> {
        let entry = self.undo.pop()?;
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
        self.undo.push(entry);
        self.coalescible = false;
        Some(forward)
    }

    /// Whether `edit` should merge into the current top entry instead of pushing
    /// a new step: an open typing run, both sides pure insertions, textually
    /// adjacent, no newline, and not sitting on the saved boundary (merging
    /// there would silently keep the buffer "clean" after an edit).
    fn can_coalesce(&self, edit: &Edit) -> bool {
        if !self.coalescible || !edit.removed.is_empty() || edit.inserted.contains('\n') {
            return false;
        }
        match self.undo.last() {
            Some(top) => {
                top.edit.removed.is_empty()
                    && edit.at == top.edit.at + top.edit.inserted.len()
                    && !matches!(self.clean, Clean::At(id) if id == top.id)
            }
            None => false,
        }
    }

    /// Drop the oldest entries once the stack exceeds [`MAX_UNDO`]. If the saved
    /// baseline is among the evicted entries it becomes unreachable.
    fn enforce_cap(&mut self) {
        if self.undo.len() <= MAX_UNDO {
            return;
        }
        let overflow = self.undo.len() - MAX_UNDO;
        let clean_evicted = match self.clean {
            Clean::Empty => true, // the empty-stack baseline is now behind us
            Clean::At(id) => self.undo[..overflow].iter().any(|e| e.id == id),
            Clean::Gone => false,
        };
        if clean_evicted {
            self.clean = Clean::Gone;
        }
        self.undo.drain(0..overflow);
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

    // ---- diff ----

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
}
