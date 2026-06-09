//! Hand rack layout: screen slots and tile pixel size vs tile count.

use crate::game::game_mode::HAND_SIZE;
use crate::ui::focus_nav::rect_contains;
use crate::ui::layout::LayoutResult;

/// Ideal hand length for tile scale (default [`HAND_SIZE`]).
pub(crate) const HAND_TILE_REFERENCE_COUNT: usize = HAND_SIZE;

/// Hand sizes tuned beyond the reference (ordeals / relic modifiers).
pub(crate) const HAND_TILE_COUNT_MAX: usize = 20;

/// At [`HAND_TILE_COUNT_MAX`] tiles, size is at least this fraction of the reference.
const HAND_TILE_SIZE_FLOOR_FRAC: f32 = 0.78;

pub(super) fn hand_slots_for_count(
    layout: &LayoutResult,
    hand_len: usize,
) -> Vec<(f32, f32, f32, f32)> {
    if hand_len == 0 {
        return Vec::new();
    }

    let base_count = layout.hand_slot_count;
    let slot_w = layout.hand_slot_w;
    let (strip_x, strip_y, strip_w) = layout.hand_strip_origin();
    let sh = layout.hand_slot_h;

    if hand_len <= base_count {
        let center_offset = if hand_len < base_count {
            ((base_count - hand_len) as f32 * slot_w) * 0.5
        } else {
            0.0
        };
        return (0..hand_len)
            .map(|i| {
                (
                    strip_x + center_offset + i as f32 * slot_w,
                    strip_y,
                    slot_w,
                    sh,
                )
            })
            .collect();
    }

    let wide_slot_w = strip_w / hand_len as f32;
    (0..hand_len)
        .map(|i| (strip_x + i as f32 * wide_slot_w, strip_y, wide_slot_w, sh))
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

/// Resolve the hand tile under the cursor for mouse hover / focus.
///
/// Keyboard / controller spatial nav uses tile-sized screen bounds
/// ([`hand_tile_screen_rect`]); full-height GLB slot strips are layout-only.
/// Prefer the raycast pick when it agrees with the projected 3D screen bounds;
/// otherwise hit-test the one-frame-stale projected rects.
pub(crate) fn hand_tile_pick_at_cursor(
    picked: Option<usize>,
    projected: &[(usize, [f32; 4])],
    cx: f32,
    cy: f32,
) -> Option<usize> {
    if let Some(idx) = picked {
        let bounds_agree = projected
            .iter()
            .any(|(i, r)| *i == idx && rect_contains(*r, cx, cy));
        if bounds_agree || projected.is_empty() {
            return Some(idx);
        }
    }
    projected
        .iter()
        .filter(|(_, r)| rect_contains(*r, cx, cy))
        .min_by(|(_, a), (_, b)| {
            (a[2] * a[3])
                .partial_cmp(&(b[2] * b[3]))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| *i)
}

/// Screen bounds for one hand tile — focus nav, tooltips, and mouse hit fallback.
///
/// Prefer the renderer's projected 3D tile AABB (`projected`, one frame stale).
/// Before the first projection pass, approximate from the GLB slot strip.
pub(crate) fn hand_tile_screen_rect(
    index: usize,
    slot: (f32, f32, f32, f32),
    hand_len: usize,
    slot_w: f32,
    author_scale: f32,
    projected: &[(usize, [f32; 4])],
) -> [f32; 4] {
    if let Some((_, rect)) = projected.iter().find(|(i, _)| *i == index) {
        if rect[2] > 1.0 && rect[3] > 1.0 && rect[0].is_finite() && rect[1].is_finite() {
            return *rect;
        }
    }
    hand_tile_screen_rect_from_slot(slot, hand_len, slot_w, author_scale)
}

/// Tile-sized screen rect anchored to the bottom of a full-height GLB slot strip.
pub(crate) fn hand_tile_screen_rect_from_slot(
    slot: (f32, f32, f32, f32),
    hand_len: usize,
    slot_w: f32,
    author_scale: f32,
) -> [f32; 4] {
    let (x, y, w, strip_h) = slot;
    let tile_size = hand_tile_size_from_slot_width(slot_w, hand_len, author_scale);
    let tw = tile_size.max(16.0);
    let th = (tile_size * 1.4).max(16.0);
    let tx = x + (w - tw) * 0.5;
    // GLB strip is centered on `hand_tiles_*` markers; tiles sit on that line,
    // not at the bottom of the full-height layout band.
    let surface_y = y + strip_h * 0.5;
    let ty = surface_y - th * 0.5;
    [tx, ty, tw, th]
}

/// Screen rect for hand-tile inspect tooltips (same bounds as focus nav).
pub(crate) fn hand_tile_tooltip_rect(
    index: usize,
    slot: (f32, f32, f32, f32),
    hand_len: usize,
    slot_w: f32,
    author_scale: f32,
    projected: &[(usize, [f32; 4])],
) -> [f32; 4] {
    hand_tile_screen_rect(index, slot, hand_len, slot_w, author_scale, projected)
}

#[cfg(test)]
mod tests {
    use super::{
        HAND_TILE_COUNT_MAX, HAND_TILE_REFERENCE_COUNT, hand_slots_for_count,
        hand_tile_layout_size_px,
    };

    #[test]
    fn hand_tile_screen_rect_from_slot_is_tile_sized_not_full_strip() {
        let mut layout = crate::ui::layout::UiLayout::new();
        let solved = layout.solve(1920.0, 1080.0);
        let slots = hand_slots_for_count(&solved, 14);
        let (_, _, slot_w, strip_h) = slots[0];
        let rect = super::hand_tile_screen_rect_from_slot(slots[0], 14, slot_w, 1.0);
        assert!(rect[3] < strip_h * 0.25, "focus rect should be tile-sized");
        assert!(rect[2] <= slot_w + 0.01);
        let surface_y = slots[0].1 + strip_h * 0.5;
        assert!((rect[1] + rect[3] * 0.5 - surface_y).abs() < 1.0);
    }

    #[test]
    fn hand_tile_screen_rect_prefers_projected_bounds() {
        let slot = (100.0, 200.0, 80.0, 900.0);
        let projected = [(3, [110.0, 850.0, 60.0, 88.0])];
        let rect = super::hand_tile_screen_rect(3, slot, 14, 80.0, 1.0, &projected);
        assert_eq!(rect, [110.0, 850.0, 60.0, 88.0]);
    }

    #[test]
    fn hand_slots_for_count_centers_short_hands() {
        let mut layout = crate::ui::layout::UiLayout::new();
        let solved = layout.solve(1400.0, 900.0);
        let (strip_x, _, _) = solved.hand_strip_origin();

        let centered = hand_slots_for_count(&solved, 10);

        assert_eq!(centered.len(), 10);
        assert!((centered[0].0 - (strip_x + solved.hand_slot_w * 2.0)).abs() < 0.01);
        assert!((centered[0].2 - solved.hand_slot_w).abs() < 0.01);
    }

    #[test]
    fn hand_slots_for_count_compresses_wide_hands_into_strip() {
        let mut layout = crate::ui::layout::UiLayout::new();
        let solved = layout.solve(1400.0, 900.0);
        let (strip_x, _, strip_w) = solved.hand_strip_origin();

        let slots = hand_slots_for_count(&solved, 16);

        assert_eq!(slots.len(), 16);
        assert!(slots[0].0 >= strip_x - 0.01);
        assert!((slots[15].0 + slots[15].2 - (strip_x + strip_w)).abs() < 0.01);
        assert!(slots[0].2 < solved.hand_slot_w);
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
    fn hand_tile_pick_rejects_raycast_outside_projected_bounds() {
        let rects = [(3, [100.0, 500.0, 40.0, 60.0])];
        assert_eq!(
            super::hand_tile_pick_at_cursor(Some(7), &rects, 10.0, 10.0),
            None
        );
    }

    #[test]
    fn hand_tile_pick_accepts_raycast_inside_projected_bounds() {
        let rects = [(7, [100.0, 500.0, 40.0, 60.0])];
        assert_eq!(
            super::hand_tile_pick_at_cursor(Some(7), &rects, 120.0, 530.0),
            Some(7)
        );
    }

    #[test]
    fn hand_tile_pick_falls_back_to_projected_rect() {
        let rects = [(2, [10.0, 20.0, 30.0, 40.0]), (5, [50.0, 20.0, 30.0, 40.0])];
        assert_eq!(
            super::hand_tile_pick_at_cursor(None, &rects, 60.0, 35.0),
            Some(5)
        );
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
