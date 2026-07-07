//! Pure render-geometry maths for the vendored `text_editor` widget (#79).
//!
//! The native widget (`crates/iced/src/text_editor.rs`) is where the hardest
//! numeric code lives — line-number gutter sizing, scrollbar thumb geometry — yet
//! it sits in the render shell, *outside* the coverage gate the epic's Definition
//! of Done sets for logic (#25 / #27). These functions are the renderer-free part
//! of that maths, lifted here into the pure core so the gate covers them and they
//! get unit + property tests, while the widget keeps only the thin
//! `iced`-typed assembly (building `Rectangle`s from these numbers).
//!
//! Everything here is plain `f32` / integer arithmetic — no `iced` types — so it
//! stays in the dependency-free core.

/// Padding, in logical px, on each side of the numbers inside the line-number
/// gutter (#41). Also used by the widget to right-align the numbers within the
/// reserved strip, so it is public.
pub const GUTTER_PAD: f32 = 6.0;

/// Minimum on-screen scrollbar thumb length in logical px (#34), so the thumb
/// stays grabbable even for a very long document.
pub const MIN_THUMB_LEN: f32 = 24.0;

/// Number of decimal digits in `n`, at least 1 (a zero- or one-line document
/// still shows "1"). Used to size the gutter to its largest line number.
pub fn digit_count(n: usize) -> u32 {
    n.max(1).ilog10() + 1
}

/// Width in logical px of the line-number gutter for a document of `line_count`
/// lines at `text_size` px, or `0.0` when disabled (#41).
///
/// Sized to hold the widest line number (its digit count) plus [`GUTTER_PAD`] on
/// each side. A single source of truth so `layout`, `update`, `mouse_interaction`
/// and `draw` all reserve exactly the same strip — they must agree or clicks land
/// on the wrong column.
pub fn gutter_width(enabled: bool, line_count: usize, text_size: f32) -> f32 {
    if !enabled {
        return 0.0;
    }
    // ~0.6 em per digit is a safe overestimate for the proportional default font;
    // right-aligned numbers simply sit within the reserved column.
    let digits = digit_count(line_count) as f32;
    (digits * text_size * 0.6).ceil() + GUTTER_PAD * 2.0
}

/// The width remaining after insetting a strip of `gutter` px off the left of a
/// region `width` px wide, clamped at `0.0` so a gutter wider than the region can
/// never produce a negative width (#41).
pub fn inset_width(width: f32, gutter: f32) -> f32 {
    (width - gutter).max(0.0)
}

/// One axis of scrollbar-thumb geometry (#34): how long the thumb is and how far
/// along its track it sits, plus the document's maximum scroll distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScrollThumb {
    /// Thumb length in px, never below [`MIN_THUMB_LEN`].
    pub length: f32,
    /// Thumb start position measured from the track's start, in px.
    pub offset_along_track: f32,
    /// The largest meaningful scroll offset (content beyond the viewport), at
    /// least `1.0` so callers can divide by it safely.
    pub max_scroll: f32,
}

/// Scrollbar-thumb geometry for one axis, shared by the horizontal and vertical
/// bars so the two never disagree. `viewport_len` is the visible extent,
/// `content_len` the total, `offset` the current scroll, and `track_len` the
/// length of the scrollbar track — all in px along the same axis.
///
/// Clamps are chosen to match the widget's original inline maths exactly: the
/// thumb is at least [`MIN_THUMB_LEN`] long, `max_scroll` is at least `1.0` (so
/// the division is safe even with no overflow to scroll), and `offset` is clamped
/// into `[0, max_scroll]` before being mapped onto the track.
pub fn scroll_thumb(
    viewport_len: f32,
    content_len: f32,
    offset: f32,
    track_len: f32,
) -> ScrollThumb {
    let ratio = (viewport_len / content_len).min(1.0);
    let length = (track_len * ratio).max(MIN_THUMB_LEN);
    let max_scroll = (content_len - viewport_len).max(1.0);
    let travel = track_len - length;
    let offset_along_track = travel * (offset.clamp(0.0, max_scroll) / max_scroll);
    ScrollThumb {
        length,
        offset_along_track,
        max_scroll,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn gutter_is_zero_when_disabled() {
        assert_eq!(gutter_width(false, 100_000, 14.0), 0.0);
    }

    #[test]
    fn gutter_grows_with_the_line_count_digits() {
        let one = gutter_width(true, 9, 14.0);
        let two = gutter_width(true, 99, 14.0);
        let three = gutter_width(true, 999, 14.0);
        assert!(two > one, "two digits are wider than one");
        assert!(three > two, "three digits are wider than two");
        // A one-line document still reserves a (one-digit) gutter, never zero.
        assert!(gutter_width(true, 1, 14.0) > 0.0);
    }

    #[test]
    fn gutter_scales_with_font_size() {
        assert!(gutter_width(true, 100, 24.0) > gutter_width(true, 100, 12.0));
    }

    #[test]
    fn digit_count_counts_decimal_digits() {
        assert_eq!(digit_count(0), 1); // empty/one-line document → "1"
        assert_eq!(digit_count(1), 1);
        assert_eq!(digit_count(9), 1);
        assert_eq!(digit_count(10), 2);
        assert_eq!(digit_count(999), 3);
        assert_eq!(digit_count(1000), 4);
    }

    #[test]
    fn inset_width_never_goes_negative() {
        assert_eq!(inset_width(100.0, 30.0), 70.0);
        assert_eq!(inset_width(100.0, 100.0), 0.0);
        // A gutter wider than the region clamps at zero, never negative.
        assert_eq!(inset_width(40.0, 50.0), 0.0);
    }

    #[test]
    fn thumb_is_at_least_the_minimum_length() {
        // A hugely taller-than-viewport document still gets a grabbable thumb.
        let t = scroll_thumb(100.0, 1_000_000.0, 0.0, 300.0);
        assert!(t.length >= MIN_THUMB_LEN);
    }

    #[test]
    fn thumb_sits_at_the_track_start_when_unscrolled() {
        let t = scroll_thumb(100.0, 400.0, 0.0, 300.0);
        assert_eq!(t.offset_along_track, 0.0);
    }

    #[test]
    fn thumb_travels_toward_the_end_as_offset_grows() {
        let track = 300.0;
        let top = scroll_thumb(100.0, 400.0, 0.0, track);
        let mid = scroll_thumb(100.0, 400.0, top.max_scroll / 2.0, track);
        let bot = scroll_thumb(100.0, 400.0, top.max_scroll, track);
        assert!(mid.offset_along_track > top.offset_along_track);
        assert!(bot.offset_along_track > mid.offset_along_track);
    }

    proptest! {
        /// The gutter width is finite, non-negative, and zero exactly when
        /// disabled — for any line count and any sane font size.
        #[test]
        fn gutter_width_is_finite_and_nonnegative(
            enabled in any::<bool>(),
            line_count in 0usize..2_000_000,
            text_size in 1.0f32..200.0,
        ) {
            let w = gutter_width(enabled, line_count, text_size);
            prop_assert!(w.is_finite());
            prop_assert!(w >= 0.0);
            prop_assert_eq!(w == 0.0, !enabled);
        }

        /// `inset_width` is never negative and never exceeds the original width.
        #[test]
        fn inset_width_stays_in_range(width in 0.0f32..10_000.0, gutter in 0.0f32..10_000.0) {
            let w = inset_width(width, gutter);
            prop_assert!(w >= 0.0);
            prop_assert!(w <= width);
        }

        /// Thumb geometry never panics or produces NaN, keeps the minimum length,
        /// and keeps `max_scroll >= 1.0` so callers can divide by it — for any
        /// finite, positive-ish inputs (the ranges the widget can actually pass).
        #[test]
        fn thumb_geometry_is_well_formed(
            viewport in 1.0f32..5_000.0,
            content in 1.0f32..5_000_000.0,
            offset in -10_000.0f32..5_000_000.0,
            track in 1.0f32..5_000.0,
        ) {
            let t = scroll_thumb(viewport, content, offset, track);
            prop_assert!(t.length.is_finite() && t.length >= MIN_THUMB_LEN);
            prop_assert!(t.max_scroll >= 1.0);
            prop_assert!(t.offset_along_track.is_finite());
        }
    }
}
