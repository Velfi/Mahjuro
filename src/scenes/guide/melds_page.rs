#![allow(unused_imports)]
use crate::core::hand::{MeldKind, validate_selection};
use crate::core::memorial_talisman::MemorialTalismanKind;
use crate::core::progression::PlayerProgress;
use crate::core::relic::{RelicId, all_relic_defs, relic_visual};
use crate::core::tag::TagKind;
use crate::core::talisman::TalismanKind;
use crate::core::tile::{Suit, Tile};
use crate::core::tile_pack::{PACK_ASPECT_W_OVER_H, TilePackKind};
use crate::core::yaku::{YakuKind, detect_yaku_with_wind};
use crate::core::zodiac::ZodiacKind;
use crate::persistence::TilePreset;
use crate::render::consumable_prop_scale::for_sale_talisman_tablet_extent;
use crate::render::decal::load_ui_font;
use crate::render::doc_tile_camera::{DOC_TILE_ROTATION, doc_tile_camera};
use crate::render::draw_cmd::{
    CameraParams, DrawCmd, ImageQuad, ImageQuadSource, Object3d, Object3dKind, SceneLighting,
    ShowcaseTilePlacement, UiFrame, camera_facing_euler_xyz_rad,
};
use crate::render::gameplay_glb;
use crate::render::scene_keys;
use crate::render::showcase_tile_layout::{
    ShowcaseTileLabelGaps, showcase_tile_group_label_anchor, showcase_tile_merge_projected_group,
};
use crate::render::table_transform::{
    compose_rotation_euler, mat4_to_euler_xyz_rad, rot_euler_xyz_rad,
};
use crate::render::theme::{ButtonState, ButtonVariant, color, metrics, typography};
use crate::render::vocabulary_colors::{GlossaryMode, text_effect_for_glossary_tint};
use crate::render::wgpu_renderer::{
    GpuInstance, TextAlign, TextBlockVerticalAlign, TextLabel,
};
use crate::render::world_space::{
    object3d_pos_triple_for_world_center, world_on_camera_ray_plane_z,
};
use crate::ui::chart_primitives::{ChartClip, push_yaku_pill, yaku_pill_width};
use crate::ui::clip::intersect_rect;
use crate::ui::controller_hints::screen_footer_reserve;
use crate::ui::styled_text;
use crate::ui::styled_text::push_keyword_label;
use crate::ui::temptation_icons::temptation_icon_source;
use crate::ui::widget::{self, wrap_text};
use crate::ui::widget_tree::{FlatItem, FocusId, TreeState};

use crate::scenes::archive_career::{yaku_pill_face, yaku_pill_ink, yaku_pill_rim};
use crate::scenes::economy_intro_copy;
use crate::scenes::flowers_intro_copy;
use crate::scenes::header_chrome::{HeaderChromeMetrics, HeaderTitleLayout};
use crate::scenes::melds_intro_copy;
use crate::scenes::scoring_intro_copy;
use crate::scenes::tanuki_tips_intro_copy;
use crate::scenes::tiles_intro_copy;
use crate::scenes::yaku_intro_copy;
use crate::scenes::{BackgroundId, DrawCtx};

use glam::{Mat4, Quat, Vec3};

use super::example_grid::{
    layout_guide_example_grid, push_tile_group_labels, push_tiles_example_labels,
    push_tiles_example_panels,
};
use super::page_panels::GuideExampleCellLayout;
use super::tiles_page::{MELDS_EXAMPLE_ROWS, MELDS_ROW_WEIGHTS};
use super::flowers_page::{
    guide_column_footer_height, push_guide_column_footer_prose, push_melds_left_cards,
};
use super::layout::GuideLayout;
use super::content::page_graffiti;
use super::flowers_page::push_flowers_margin_scrawl;
use super::{PAGE_MELDS, TileGroup};
pub(super) fn draw_melds_page(
    frame: &mut UiFrame,
    layout: &GuideLayout,
    _progress: &PlayerProgress,
    w: f32,
    h: f32,
    scale: f32,
    groups: &[TileGroup],
    cam: &CameraParams,
    content_top: f32,
    content_floor: f32,
) {
    let body_font = typography::size(typography::H42, h);
    let line_mul = 1.12;
    let left_w = layout.content_w * 0.38;
    let gutter = layout.content_w * 0.02;
    let right_w = layout.content_w - left_w - gutter;
    let left_x = layout.content_x;
    let right_x = left_x + left_w + gutter;
    let columns_bottom = content_floor - h * 0.006;
    let graffiti = page_graffiti(PAGE_MELDS);
    let graffiti_reserve = if graffiti.is_some() { h * 0.14 } else { 0.0 };

    push_melds_left_cards(
        frame,
        left_x,
        left_w,
        content_top,
        columns_bottom - graffiti_reserve,
        h,
        body_font,
        line_mul,
        Some(melds_intro_copy::INTRO),
    );
    if let Some(scrawl) = graffiti {
        push_flowers_margin_scrawl(frame, left_x, left_w, columns_bottom, h, scrawl);
    }

    let (placements, labels, panels, _cells) = layout_guide_example_grid(
        cam,
        groups,
        right_x,
        right_w,
        w,
        h,
        content_top,
        columns_bottom,
        MELDS_EXAMPLE_ROWS,
        &MELDS_ROW_WEIGHTS,
        0.0,
        GuideExampleCellLayout::default(),
    );
    push_tiles_example_panels(frame, groups, &panels);
    if !placements.is_empty() {
        frame
            .cmds
            .push(DrawCmd::ShowcaseTileBatch(placements.into()));
    }
    push_tiles_example_labels(frame, groups, &labels, h, scale);
}

// ── Yaku intro page (page 2) ──────────────────────────────────────────────

pub(super) const YAKU_EXAMPLE_ROWS: &[&[usize]] = &[&[0], &[1], &[2]];
pub(super) const YAKU_ROW_WEIGHTS: [f32; 3] = [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0];
/// Guide yaku intro tablets are drawn larger than the live HUD for readability.
pub(super) const GUIDE_YAKU_TABLET_SCALE: f32 = 1.45;
/// Yaku intro page: left prose column share (remainder is example tiles).
pub(super) const YAKU_INTRO_LEFT_COL_FRAC: f32 = 0.34;

pub(super) fn guide_yaku_tablet_metrics(w: f32, h: f32, tablet_count: usize) -> (f32, f32) {
    let (row_h, pill_w) =
        crate::render::gameplay_glb::gameplay_yaku_tablet_ui_metrics(w, h, tablet_count);
    (
        row_h * GUIDE_YAKU_TABLET_SCALE,
        pill_w * GUIDE_YAKU_TABLET_SCALE,
    )
}

pub(super) fn guide_yaku_tablet_reserve(w: f32, h: f32) -> f32 {
    let (row_h, _) = guide_yaku_tablet_metrics(w, h, 2);
    row_h + (h * 0.012).clamp(6.0, 12.0)
}

/// After tile *i*, insert extra horizontal gap before the next meld begins.
pub(super) fn guide_example_meld_breaks_after(tiles: &[Tile]) -> Vec<bool> {
    let n = tiles.len();
    let mut breaks = vec![false; n];
    if n < 2 {
        return breaks;
    }
    let Some(sets) = validate_selection(tiles) else {
        return breaks;
    };
    let mut id_to_meld = std::collections::HashMap::with_capacity(n);
    for (mi, set) in sets.iter().enumerate() {
        for &id in &set.tile_ids {
            id_to_meld.insert(id, mi);
        }
    }
    for i in 0..n - 1 {
        if let (Some(a), Some(b)) = (
            id_to_meld.get(&tiles[i].id),
            id_to_meld.get(&tiles[i + 1].id),
        ) {
            breaks[i] = a != b;
        }
    }
    breaks
}

pub(super) fn guide_example_meld_gap_count(tiles: &[Tile]) -> usize {
    guide_example_meld_breaks_after(tiles)
        .iter()
        .filter(|&&b| b)
        .count()
}

pub(super) fn guide_example_meld_gap_px(window_h: f32, tile_px: f32) -> f32 {
    (tile_px * 0.28).clamp((window_h * 0.012).max(10.0), 32.0)
}

pub(super) fn guide_example_row_width(tile_px: f32, tiles: &[Tile], meld_gap: f32) -> f32 {
    let n = tiles.len();
    if n == 0 {
        return 0.0;
    }
    tile_px * n as f32 + meld_gap * guide_example_meld_gap_count(tiles) as f32
}

/// Fixed overhead per yaku intro example row (title + gap + tablet band, no subtitle).
pub(super) fn guide_yaku_example_row_overhead(h: f32, tablet_row_reserve: f32) -> f32 {
    let pad = 4.0;
    let title_h = typography::size(typography::H28, h);
    let label_tile_gap = (h * 0.012).clamp(8.0, 14.0);
    pad * 2.0 + title_h + label_tile_gap + tablet_row_reserve
}

/// One tile size for all yaku intro rows so examples read at the same scale.
pub(super) fn guide_yaku_shared_tile_px(
    col_w: f32,
    h: f32,
    usable_h: f32,
    groups: &[TileGroup],
    rows: &[&[usize]],
    row_weights: &[f32],
    tablet_row_reserve: f32,
) -> f32 {
    let overhead = guide_yaku_example_row_overhead(h, tablet_row_reserve);
    let row_gap = 3.0;
    let weight_sum: f32 = row_weights.iter().sum();
    let tile_cap = h * 0.070;
    let floor = (h * 0.044).clamp(38.0, 52.0);
    let mut min_px = f32::MAX;
    let mut tightest_fit = f32::MAX;
    for (row_i, indices) in rows.iter().enumerate() {
        let row_weight = row_weights.get(row_i).copied().unwrap_or(1.0);
        let row_h = usable_h * (row_weight / weight_sum) - row_gap * 0.5;
        let tile_area_h = (row_h - overhead).max(20.0);
        tightest_fit = tightest_fit.min(tile_area_h * 0.88);
        for &gi in indices.iter() {
            if gi >= groups.len() {
                continue;
            }
            let n = groups[gi].tiles.len().max(1) as f32;
            let meld_gap_est = (h * 0.014).clamp(10.0, 22.0);
            let meld_gaps = guide_example_meld_gap_count(&groups[gi].tiles) as f32;
            let px = ((col_w - meld_gap_est * meld_gaps) / (n + 0.15))
                .min(tile_area_h * 0.88)
                .min(tile_cap);
            min_px = min_px.min(px);
        }
    }
    if min_px == f32::MAX {
        tightest_fit.min(floor).max(24.0)
    } else {
        min_px
            .max(floor.min(tightest_fit))
            .min(tightest_fit)
            .max(24.0)
    }
}

