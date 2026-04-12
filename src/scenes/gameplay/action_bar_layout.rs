//! Bottom action bar: sort tablets, discard bowl, play mirror, optional cash-in, journal lift.
//!
//! Pixel rects and [`crate::render::draw_cmd::TableSurfaceAnchor`] lift share one place so
//! spacing / height above the felt stay in sync with [`crate::render::table_space`].
//!
//! **World +Z (playerward):** [`crate::render::table_space::pixel_to_table_world`] maps layout
//! `py` → world `z`. Add [`action_hud_world_z_py_nudge`] to the anchor’s `py` when building
//! [`crate::render::draw_cmd::TableSurfaceAnchor`] — **not** to the lift component (that is world +Y).

use crate::ui::layout::LayoutResult;

/// Tunable vertical gap from the bottom of the window to the sort+journal row (scaled).
/// Low values keep that row near the bottom edge — playerward vs the hand rack.
const ACTION_BAR_BOTTOM_INSET_SCALE: f32 = 0.0;
const ACTION_BAR_BOTTOM_INSET_MIN: f32 = 0.0;
/// Space below the hand rack to the discard/play row centerline (scaled / floor).
/// Large enough that bowl/mirror sit clearly in front of the tilted tiles (toward the camera).
const HAND_TO_BOWL_ROW_GAP_SCALE: f32 = 42.0;
const HAND_TO_BOWL_ROW_GAP_MIN: f32 = 30.0;
/// Clearance between bowl/mirror row and sort row when vertical space is tight (scaled / min).
const BOWL_TO_SORT_CLEARANCE_SCALE: f32 = 0.5;
const BOWL_TO_SORT_CLEARANCE_MIN: f32 = 0.5;
/// Third component of [`crate::render::draw_cmd::TableSurfaceAnchor`] for wood tablets / bowl / mirror.
const ACTION_HUD_TABLE_LIFT_SCALE: f32 = 44.0;
const ACTION_HUD_TABLE_LIFT_MIN: f32 = 32.0;
const ACTION_HUD_WORLDZ_PY_NUDGE_SCALE: f32 = 100.0;
const ACTION_HUD_WORLDZ_PY_NUDGE_MIN: f32 = 14.0;

/// Extra **screen Y** for bottom-action [`TableSurfaceAnchor`] `py`. Maps to **world +Z** via
/// [`crate::render::table_space::pixel_to_table_world`] — pulls bowl/mirror/sort/journal toward the
/// player along the table without changing world height (`lift_y`).
#[inline]
pub fn action_hud_world_z_py_nudge(layout_scale: f32) -> f32 {
    (ACTION_HUD_WORLDZ_PY_NUDGE_SCALE * layout_scale).max(ACTION_HUD_WORLDZ_PY_NUDGE_MIN)
}

/// `(x, y, w, h)` layout rects and scalar anchors for the bottom HUD row.
#[derive(Clone, Copy, Debug)]
pub struct ActionBarLayout {
    /// UI scale factor including [`crate::scenes::DrawCtx::ui_scale`].
    pub scale: f32,
    pub container_w: f32,
    pub container_x: f32,
    pub suit_btn_rect: (f32, f32, f32, f32),
    pub rank_btn_rect: (f32, f32, f32, f32),
    pub discard_btn_rect: (f32, f32, f32, f32),
    pub play_btn_rect: (f32, f32, f32, f32),
    pub trigger_btn_rect: (f32, f32, f32, f32),
    /// Height above table for [`crate::render::draw_cmd::TableSurfaceAnchor`] on action meshes.
    pub action_hud_table_lift: f32,
}

/// `hand_slots` as `(x, y, w, h)` per slot (same as [`LayoutResult::hand_slots`] flattened in gameplay).
/// `structure_*` only used when `has_structure` (cash-in beside mat).
/// `layout_scale` must match the value used for yaku/structure stacking above the hand.
pub fn compute_action_bar(
    layout: &LayoutResult,
    hand_slots: &[(f32, f32, f32, f32)],
    layout_scale: f32,
    ui_scale: f32,
    has_structure: bool,
    structure_strip_top: f32,
    structure_tag_h: f32,
    structure_meld_h: f32,
) -> ActionBarLayout {
    let scale = (layout.window_w.min(layout.window_h)) / 600.0 * ui_scale;
    let btn_w = (120.0 * layout_scale).max(60.0);
    let btn_h = (32.0 * layout_scale).max(20.0);
    let btn_gap = 12.0 * layout_scale;
    let container_w = (btn_w * 4.0 + btn_gap * 3.0).min(layout.window_w * 0.92);
    let container_x = (layout.window_w - container_w) * 0.5;
    let btn_y = layout.window_h
        - btn_h
        - (ACTION_BAR_BOTTOM_INSET_SCALE * layout_scale).max(ACTION_BAR_BOTTOM_INSET_MIN);

    let sort_container_w = btn_w * 2.0 + btn_gap;
    let sort_container_x = (layout.window_w - sort_container_w) * 0.5;
    let suit_btn_rect = (sort_container_x, btn_y, btn_w, btn_h);
    let rank_btn_rect = (sort_container_x + btn_w + btn_gap, btn_y, btn_w, btn_h);

    let yaku_panel_h = (33.0 * layout_scale).max(24.0).min(layout.window_h * 0.10);
    let bowl_diam = (yaku_panel_h * 2.4).min(layout.window_h * 0.18);
    let rack_bottom = hand_slots
        .first()
        .map(|s| s.1 + s.3)
        .unwrap_or_else(|| layout.hand_strip.y + layout.hand_strip.h);

    let row_gap = (HAND_TO_BOWL_ROW_GAP_SCALE * layout_scale).max(HAND_TO_BOWL_ROW_GAP_MIN);
    let mut bowl_cy = rack_bottom + row_gap + bowl_diam * 0.5;
    let max_cy = btn_y
        - (BOWL_TO_SORT_CLEARANCE_SCALE * layout_scale).max(BOWL_TO_SORT_CLEARANCE_MIN)
        - bowl_diam * 0.5;
    if bowl_cy > max_cy {
        bowl_cy = max_cy;
    }
    let bowl_inset = bowl_diam * 0.30;
    let mut bowl_cx = container_x + bowl_inset + bowl_diam * 0.5;
    let mut mirror_cx = container_x + container_w - bowl_inset - bowl_diam * 0.5;
    bowl_cx = bowl_cx.max(bowl_diam * 0.5 + 4.0);
    mirror_cx = mirror_cx.min(layout.window_w - bowl_diam * 0.5 - 4.0);
    let mirror_cy = bowl_cy;

    let discard_btn_rect = (
        bowl_cx - bowl_diam * 0.5,
        bowl_cy - bowl_diam * 0.5,
        bowl_diam,
        bowl_diam,
    );
    let play_btn_rect = (
        mirror_cx - bowl_diam * 0.5,
        mirror_cy - bowl_diam * 0.5,
        bowl_diam,
        bowl_diam,
    );

    let trigger_gap = 10.0 * layout_scale;
    let trigger_w = (btn_w * 1.15)
        .max(bowl_diam * 0.90)
        .max(84.0 * layout_scale);
    let trigger_h = (btn_h * 1.10)
        .max(bowl_diam * 0.42)
        .max(28.0 * layout_scale);
    let trigger_btn_rect = if has_structure {
        let meld_center_y = structure_strip_top + structure_tag_h + structure_meld_h * 0.5;
        let mut bx = container_x + container_w + trigger_gap;
        let max_bx = layout.window_w - trigger_w - (8.0 * layout_scale).max(4.0);
        if bx > max_bx {
            bx = max_bx;
        }
        (bx, meld_center_y - trigger_h * 0.5, trigger_w, trigger_h)
    } else {
        (
            mirror_cx - trigger_w * 0.5,
            mirror_cy - bowl_diam * 0.5 - trigger_gap - trigger_h,
            trigger_w,
            trigger_h,
        )
    };

    let action_hud_table_lift =
        (ACTION_HUD_TABLE_LIFT_SCALE * layout_scale).max(ACTION_HUD_TABLE_LIFT_MIN);

    ActionBarLayout {
        scale,
        container_w,
        container_x,
        suit_btn_rect,
        rank_btn_rect,
        discard_btn_rect,
        play_btn_rect,
        trigger_btn_rect,
        action_hud_table_lift,
    }
}
