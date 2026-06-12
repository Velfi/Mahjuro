//! Helpers for keyboard/controller focus in scrollable [`FlatItem`](crate::ui::widget_tree::FlatItem) lists.
//!
//! Scenes with a virtual viewport should:
//! 1. Include **every** focus target in the flat list (scroll-adjusted rects), not only
//!    the visible slice — see [`yaku_journal.rs`](../../scenes/yaku_journal.rs).
//! 2. Call [`clamp_index_into_viewport`] when focus moves, **before** rebuilding items /
//!    calling [`TreeState::update_flat`](crate::ui::widget_tree::TreeState::update_flat).
//! 3. Rely on [`TreeState`] preserving an offscreen focus id until the scene scrolls.

/// Keep `index` inside the viewport `[scroll, scroll + visible)`.
///
/// `total` is the number of indexed slots/rows; `visible` is how many fit on screen at once.
pub fn clamp_index_into_viewport(index: usize, scroll: f32, visible: usize, total: usize) -> f32 {
    if total == 0 || visible == 0 {
        return 0.0;
    }
    let max_scroll = total.saturating_sub(visible) as f32;
    let idx = index as f32;
    let vis = visible as f32;
    let mut s = scroll;
    if idx < s {
        s = idx;
    } else if idx >= s + vis {
        s = idx - vis + 1.0;
    }
    s.clamp(0.0, max_scroll)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_down_to_offscreen_row() {
        assert_eq!(clamp_index_into_viewport(12, 0.0, 8, 20), 5.0);
    }

    #[test]
    fn scroll_up_to_offscreen_row() {
        assert_eq!(clamp_index_into_viewport(2, 8.0, 8, 20), 2.0);
    }

    #[test]
    fn no_change_when_already_visible() {
        assert_eq!(clamp_index_into_viewport(5, 3.0, 8, 20), 3.0);
    }
}
