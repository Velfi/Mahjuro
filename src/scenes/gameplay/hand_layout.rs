//! Hand rack layout: screen slots and tile pixel size vs tile count.

use crate::game::game_mode::HAND_SIZE;

/// Ideal hand length for tile scale (default [`HAND_SIZE`]).
pub(crate) const HAND_TILE_REFERENCE_COUNT: usize = HAND_SIZE;

/// Hand sizes tuned beyond the reference (ordeals / relic modifiers).
pub(crate) const HAND_TILE_COUNT_MAX: usize = 20;

/// At [`HAND_TILE_COUNT_MAX`] tiles, size is at least this fraction of the reference.
const HAND_TILE_SIZE_FLOOR_FRAC: f32 = 0.78;

pub(super) fn hand_slots_for_count(
    layout: &crate::ui::layout::LayoutResult,
    hand_len: usize,
) -> Vec<(f32, f32, f32, f32)> {
    if hand_len == 0 {
        return Vec::new();
    }

    let base_slots = &layout.hand_slots;
    let base_count = base_slots.len();
    if hand_len <= base_count {
        let visible_count = hand_len;
        let slot_w = base_slots.first().map(|r| r.w).unwrap_or(0.0);
        let center_offset = if visible_count < base_count {
            ((base_count - visible_count) as f32 * slot_w) * 0.5
        } else {
            0.0
        };
        return base_slots
            .iter()
            .take(hand_len)
            .map(|r| (r.x + center_offset, r.y, r.w, r.h))
            .collect();
    }

    let slot_w = layout.hand_strip.w / hand_len as f32;
    (0..hand_len)
        .map(|i| {
            (
                layout.hand_strip.x + i as f32 * slot_w,
                layout.hand_strip.y,
                slot_w,
                layout.hand_strip.h,
            )
        })
        .collect()
}

/// Pixel size for one hand tile given the full marker strip span and active tile count.
///
/// Sized for a [`HAND_TILE_REFERENCE_COUNT`]-tile hand; shorter hands cap at that scale,
/// longer hands shrink with a floor toward [`HAND_TILE_COUNT_MAX`].
pub(crate) fn hand_tile_layout_size_px(
    strip_span_px: f32,
    hand_len: usize,
    author_scale: f32,
) -> f32 {
    let n = hand_len.max(1) as f32;
    let ref_n = HAND_TILE_REFERENCE_COUNT as f32;
    let reference = (strip_span_px / ref_n) * author_scale;
    let raw = (strip_span_px / n) * author_scale;

    // Short hands: do not grow past the 14-tile reference size.
    let capped = raw.min(reference);

    if n <= ref_n {
        return capped;
    }

    // Long hands: pure `strip / n` gets tiny; ease toward a readable floor at 20 tiles.
    let t = ((n - ref_n) / (HAND_TILE_COUNT_MAX as f32 - ref_n)).clamp(0.0, 1.0);
    let floor = reference * (1.0 - t * (1.0 - HAND_TILE_SIZE_FLOOR_FRAC));
    capped.max(floor)
}

/// Convenience when layout already exposes per-slot width (`strip_span = slot_w * hand_len`).
pub(crate) fn hand_tile_size_from_slot_width(
    slot_width_px: f32,
    hand_len: usize,
    author_scale: f32,
) -> f32 {
    hand_tile_layout_size_px(
        slot_width_px * hand_len.max(1) as f32,
        hand_len,
        author_scale,
    )
}

#[cfg(test)]
mod tests {
    use super::{
        hand_slots_for_count, hand_tile_layout_size_px, HAND_TILE_COUNT_MAX,
        HAND_TILE_REFERENCE_COUNT,
    };

    #[test]
    fn hand_slots_for_count_centers_short_hands() {
        let mut layout = crate::ui::layout::UiLayout::new();
        let solved = layout.solve(1400.0, 900.0);

        let base_first_x = solved.hand_slots.first().unwrap().x;
        let base_slot_w = solved.hand_slots.first().unwrap().w;
        let centered = hand_slots_for_count(&solved, 10);

        assert_eq!(centered.len(), 10);
        assert!((centered[0].0 - (base_first_x + base_slot_w * 2.0)).abs() < 0.01);
        assert!((centered[0].2 - base_slot_w).abs() < 0.01);
    }

    #[test]
    fn hand_slots_for_count_compresses_wide_hands_into_strip() {
        let mut layout = crate::ui::layout::UiLayout::new();
        let solved = layout.solve(1400.0, 900.0);

        let slots = hand_slots_for_count(&solved, 16);

        assert_eq!(slots.len(), 16);
        assert!(slots[0].0 >= solved.hand_strip.x - 0.01);
        assert!(
            (slots[15].0 + slots[15].2 - (solved.hand_strip.x + solved.hand_strip.w)).abs() < 0.01
        );
        assert!(slots[0].2 < solved.hand_slots[0].w);
    }

    #[test]
    fn hand_tile_size_matches_reference_at_fourteen() {
        let strip = 700.0;
        let size = hand_tile_layout_size_px(strip, HAND_TILE_REFERENCE_COUNT, 1.0);
        assert!((size - strip / HAND_TILE_REFERENCE_COUNT as f32).abs() < 0.01);
    }

    #[test]
    fn hand_tile_size_caps_short_hands_at_reference() {
        let strip = 1400.0;
        let at_14 = hand_tile_layout_size_px(strip, 14, 1.0);
        let at_12 = hand_tile_layout_size_px(strip, 12, 1.0);
        assert!((at_12 - at_14).abs() < 0.01);
        assert!(hand_tile_layout_size_px(strip, 12, 1.0) <= at_14 + 0.01);
    }

    #[test]
    fn hand_tile_size_eases_very_large_hands() {
        let strip = 1400.0;
        let at_14 = hand_tile_layout_size_px(strip, 14, 1.0);
        let linear_20 = strip / HAND_TILE_COUNT_MAX as f32;
        let at_20 = hand_tile_layout_size_px(strip, HAND_TILE_COUNT_MAX, 1.0);
        assert!(at_20 > linear_20);
        assert!(at_20 >= at_14 * 0.78 - 0.01);
        assert!(at_20 <= at_14 + 0.01);
    }
}
