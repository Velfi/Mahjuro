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

#[cfg(test)]
mod tests {
    use super::hand_slots_for_count;

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
}
