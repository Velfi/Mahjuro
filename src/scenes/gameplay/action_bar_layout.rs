//! Bottom action bar: discard bowl, play mirror, optional cash-in, journal lift.
//!
//! Pixel rects and [`crate::render::draw_cmd::WorldSurfaceAnchor`] lift share one place so
//! spacing / height above the felt stay in sync with [`crate::render::world_space`].
//!
//! **Depth (−Y playerward):** [`crate::render::world_space::pixel_to_world`] maps larger layout
//! `py` to **more negative** world `y`. Add [`action_hud_world_z_py_nudge`] to the anchor’s `py` when building
//! [`crate::render::draw_cmd::WorldSurfaceAnchor`] — **not** to the lift component (that is world +Z).

use crate::ui::layout::LayoutResult;

/// Tunable vertical gap from the bottom of the window to the journal row (scaled).
/// Low values keep that row near the bottom edge — playerward vs the hand rack.
const ACTION_BAR_BOTTOM_INSET_SCALE: f32 = 0.0;
const ACTION_BAR_BOTTOM_INSET_MIN: f32 = 0.0;
/// Space below the hand rack to the discard/play row centerline (scaled / floor).
/// Large enough that bowl/mirror sit clearly in front of the tilted tiles (toward the camera).
const HAND_TO_BOWL_ROW_GAP_SCALE: f32 = 42.0;
const HAND_TO_BOWL_ROW_GAP_MIN: f32 = 30.0;
/// Clearance between bowl/mirror row and journal row when vertical space is tight (scaled / min).
const BOWL_TO_JOURNAL_CLEARANCE_SCALE: f32 = 0.5;
const BOWL_TO_JOURNAL_CLEARANCE_MIN: f32 = 0.5;
/// Third component of [`crate::render::draw_cmd::WorldSurfaceAnchor`] for wood tablets / bowl / mirror.
const ACTION_HUD_TABLE_LIFT_SCALE: f32 = 44.0;
const ACTION_HUD_TABLE_LIFT_MIN: f32 = 32.0;
const ACTION_HUD_WORLDZ_PY_NUDGE_SCALE: f32 = 100.0;
const ACTION_HUD_WORLDZ_PY_NUDGE_MIN: f32 = 14.0;

/// Extra **screen Y** for bottom-action [`WorldSurfaceAnchor`] `py`. Maps toward **−Y** (playerward)
/// via [`crate::render::world_space::pixel_to_world`] — pulls bowl/mirror/journal toward the
/// player along the table without changing world height (`lift_z`).
#[inline]
pub fn action_hud_world_z_py_nudge(layout_scale: f32) -> f32 {
    (ACTION_HUD_WORLDZ_PY_NUDGE_SCALE * layout_scale).max(ACTION_HUD_WORLDZ_PY_NUDGE_MIN)
}

/// `(x, y, w, h)` layout rects and scalar anchors for the bottom HUD row.
#[derive(Clone, Copy, Debug)]
pub struct ActionBarLayout {
    /// Pixel scale from window size (`min / 600`).
    pub scale: f32,
    pub container_w: f32,
    pub container_x: f32,
    pub journal_btn_rect: (f32, f32, f32, f32),
    /// Center-x for the Journal book in the bottom row.
    pub journal_btn_cx: f32,
    pub discard_btn_rect: (f32, f32, f32, f32),
    pub play_btn_rect: (f32, f32, f32, f32),
    pub trigger_btn_rect: (f32, f32, f32, f32),
    /// Height above table for [`crate::render::draw_cmd::WorldSurfaceAnchor`] on action meshes.
    pub action_hud_table_lift: f32,
}

/// Shared layout for the gameplay HUD stack above/below the hand rack:
/// structure showcase, yaku tablets, and bottom action objects.
#[derive(Clone, Copy, Debug)]
pub struct GameplayHudLayout {
    pub yaku_panel_h: f32,
    pub structure_tag_h: f32,
    pub structure_meld_h: f32,
    pub structure_strip_top: f32,
    pub yaku_row_y: f32,
    pub action_bar: ActionBarLayout,
}

/// Structure-strip metrics used when `has_structure` is true — only read
/// by `compute_action_bar` in that case, so grouping them makes the
/// "structure is off" call site short.
pub struct StructureStrip {
    pub has_structure: bool,
    pub strip_top: f32,
    pub tag_h: f32,
    pub meld_h: f32,
}

/// `hand_slots` as `(x, y, w, h)` per slot (same as [`LayoutResult::hand_slots`] flattened in gameplay).
/// `structure` only used when its `has_structure` flag is true (cash-in beside mat).
/// `layout_scale` must match the value used for yaku/structure stacking above the hand.
pub fn compute_action_bar(
    layout: &LayoutResult,
    hand_slots: &[(f32, f32, f32, f32)],
    layout_scale: f32,
    structure: StructureStrip,
) -> ActionBarLayout {
    let StructureStrip {
        has_structure,
        strip_top: structure_strip_top,
        tag_h: structure_tag_h,
        meld_h: structure_meld_h,
    } = structure;
    let scale = layout_scale;
    let btn_w = (120.0 * layout_scale).max(60.0);
    let btn_h = (32.0 * layout_scale).max(20.0);
    let btn_gap = 12.0 * layout_scale;
    // Shared HUD content container. This is intentionally tied to the hand
    // strip, not to the current number of bottom action objects; otherwise
    // removing action buttons shrinks the structure showcase tiles.
    let container_w = layout.hand_strip.w.min(layout.window_w * 0.92);
    // Anchor container left edge to hand strip left (HAND_X_PAD_RATIO = 16%) so
    // structure and hand share the same left margin.
    let container_x = layout.hand_strip.x;
    let btn_y = layout.window_h
        - btn_h
        - (ACTION_BAR_BOTTOM_INSET_SCALE * layout_scale).max(ACTION_BAR_BOTTOM_INSET_MIN);

    // Journal row: single book centered at the bottom.
    let journal_btn_w = btn_w * 0.55;
    let journal_btn_rect = (
        (layout.window_w - journal_btn_w) * 0.5,
        btn_y,
        journal_btn_w,
        btn_h,
    );
    let journal_btn_cx = journal_btn_rect.0 + journal_btn_w * 0.5;

    let yaku_panel_h = (33.0 * layout_scale).max(24.0).min(layout.window_h * 0.10);
    let bowl_diam = (yaku_panel_h * 2.4).min(layout.window_h * 0.18);
    let rack_bottom = hand_slots
        .first()
        .map(|s| s.1 + s.3)
        .unwrap_or_else(|| layout.hand_strip.y + layout.hand_strip.h);

    let row_gap = (HAND_TO_BOWL_ROW_GAP_SCALE * layout_scale).max(HAND_TO_BOWL_ROW_GAP_MIN);
    let mut bowl_cy = rack_bottom + row_gap + bowl_diam * 0.5;
    let max_cy = btn_y
        - (BOWL_TO_JOURNAL_CLEARANCE_SCALE * layout_scale).max(BOWL_TO_JOURNAL_CLEARANCE_MIN)
        - bowl_diam * 0.5;
    if bowl_cy > max_cy {
        bowl_cy = max_cy;
    }
    // Bowl sits just left of the journal; mirror just right of it.
    let journal_row_left = journal_btn_rect.0;
    let journal_row_right = journal_btn_rect.0 + journal_btn_w;
    let side_gap = btn_gap * 2.0;
    let bowl_cx = (journal_row_left - side_gap - bowl_diam * 0.5).max(bowl_diam * 0.5 + 4.0);
    let mirror_cx = (journal_row_right + side_gap + bowl_diam * 0.5)
        .min(layout.window_w - bowl_diam * 0.5 - 4.0);
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
    let trigger_right_nudge = (36.0 * layout_scale).max(24.0);
    let trigger_w = (btn_w * 1.15)
        .max(bowl_diam * 0.90)
        .max(84.0 * layout_scale);
    let trigger_h = (btn_h * 1.10)
        .max(bowl_diam * 0.42)
        .max(28.0 * layout_scale);
    let trigger_btn_rect = if has_structure {
        let meld_center_y = structure_strip_top + structure_tag_h + structure_meld_h * 0.5;
        // Trigger button sits just right of the mirror (which is at right:15%).
        let mut bx = mirror_cx + bowl_diam * 0.5 + trigger_gap + trigger_right_nudge;
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
        journal_btn_rect,
        journal_btn_cx,
        discard_btn_rect,
        play_btn_rect,
        trigger_btn_rect,
        action_hud_table_lift,
    }
}

/// Computes the full gameplay HUD stack from the hand rack upward, then
/// delegates bottom-row geometry to [`compute_action_bar`]. This keeps
/// yaku tablet placement, structure showcase placement, popup anchors, and
/// focus/click rects on the same geometry.
pub fn compute_gameplay_hud_layout(
    layout: &LayoutResult,
    hand_slots: &[(f32, f32, f32, f32)],
    has_structure: bool,
    showcase_present: bool,
) -> GameplayHudLayout {
    let layout_scale = (layout.window_w.min(layout.window_h)) / 600.0;
    let yaku_panel_h = (33.0 * layout_scale).max(24.0).min(layout.window_h * 0.10);
    let yaku_panel_gap = 8.0 * layout_scale;
    let structure_tag_h = if showcase_present {
        (17.0 * layout_scale).max(14.0)
    } else {
        0.0
    };
    let structure_meld_h = if showcase_present {
        (46.0 * layout_scale).max(38.0)
    } else {
        0.0
    };
    let structure_pad = if showcase_present {
        (5.0 * layout_scale).max(3.0)
    } else {
        0.0
    };
    let structure_block_h = if showcase_present {
        structure_tag_h + structure_meld_h + structure_pad
    } else {
        0.0
    };

    // HUD panels use HAND_HUD_STACK_Y_FRAC so they clear the physical rack,
    // whose mesh anchor sits lower on the table.
    let (_, slot_y, _, slot_h) = hand_slots.first().copied().unwrap_or((
        0.0,
        layout.hand_strip.y,
        100.0,
        layout.hand_strip.h,
    ));
    let tile_center_y = slot_y + slot_h * crate::ui::layout::HAND_HUD_STACK_Y_FRAC;
    let clear_above_tiles = (34.0 * layout_scale).max(26.0);
    let band_top_above_tiles = (tile_center_y - clear_above_tiles).max(4.0);
    let min_yaku_y = layout.modifier_strip.y + layout.modifier_strip.h + (4.0 * layout_scale);

    let mut yaku_row_y = band_top_above_tiles - yaku_panel_h;
    let mut structure_strip_top = if showcase_present {
        yaku_row_y - yaku_panel_gap - structure_block_h
    } else {
        band_top_above_tiles
    };
    if yaku_row_y < min_yaku_y {
        yaku_row_y = min_yaku_y;
        if showcase_present {
            structure_strip_top = yaku_row_y - yaku_panel_gap - structure_block_h;
        }
    }
    if showcase_present && structure_strip_top < min_yaku_y {
        let deficit = min_yaku_y - structure_strip_top;
        structure_strip_top += deficit;
        yaku_row_y += deficit;
    }

    let action_bar = compute_action_bar(
        layout,
        hand_slots,
        layout_scale,
        StructureStrip {
            has_structure,
            strip_top: structure_strip_top,
            tag_h: structure_tag_h,
            meld_h: structure_meld_h,
        },
    );

    GameplayHudLayout {
        yaku_panel_h,
        structure_tag_h,
        structure_meld_h,
        structure_strip_top,
        yaku_row_y,
        action_bar,
    }
}
