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

use super::content::{
    guide_example_is_invalid, guide_invalid_tile_rotation, MeldLabel, TileGroup,
};
use super::layout::push_guide_panel_stroke;
use super::melds_page::{
    guide_example_meld_breaks_after, guide_example_meld_gap_px, guide_example_row_width,
    guide_yaku_tablet_metrics,
};
use super::page_panels::{
    push_guide_tile_placements, push_guide_yaku_tablets, GuideExampleCell,
    GuideExampleCellLayout, TilesExampleLabel,
};
use super::tile_layout::layout_tile_groups_with_max;

/// Tiles intro page — label column left, tiles fill the rest; row heights track tile size.
pub(super) fn layout_tiles_page_grid(
    _cam: &CameraParams,
    groups: &[TileGroup],
    col_x: f32,
    col_w: f32,
    _window_w: f32,
    window_h: f32,
    top: f32,
    bottom: f32,
) -> (
    Vec<ShowcaseTilePlacement>,
    Vec<TilesExampleLabel>,
    Vec<(usize, [f32; 4])>,
) {
    let available_h = (bottom - top).max(1.0);
    let row_gap = 6.0;
    let tile_gap = 3.0;
    let pad = 3.0;
    let label_col_w = (col_w * 0.18).clamp(100.0, 170.0);
    let tile_span_w = (col_w - label_col_w).max(1.0);

    let title_font = typography::size(typography::H24, window_h);
    let sub_font = typography::size(typography::H36, window_h);
    let title_h = title_font * 1.05;
    let sub_line_h = sub_font * 1.02;
    let side_label_h = title_h
        + styled_text::colored_line_block_height(
            "ranks 1–9",
            label_col_w,
            sub_line_h,
            color::PARCHMENT,
            GlossaryMode::Prose,
        );

    let width_for = |span: f32, count: usize| -> f32 {
        let gaps = tile_gap * count.saturating_sub(1) as f32;
        ((span - gaps) / count.max(1) as f32).max(1.0)
    };
    let width_limit = width_for(tile_span_w, 9);

    let tiles_page_total_h = |tile_size: f32| -> f32 {
        let subtitled_row_h = tile_size.max(side_label_h) + pad * 2.0;
        let title_only_row_h = tile_size.max(title_h) + pad * 2.0;
        subtitled_row_h * 3.0 + title_only_row_h * 3.0 + row_gap * 5.0
    };

    let mut tile_size = width_limit;
    let height_budget = available_h * 0.94;
    for _ in 0..40 {
        if tiles_page_total_h(tile_size) <= height_budget {
            break;
        }
        tile_size *= 0.94;
    }
    tile_size = (tile_size * 0.88).max(24.0);

    let subtitled_row_h = tile_size.max(side_label_h) + pad * 2.0;
    let title_only_row_h = tile_size.max(title_h) + pad * 2.0;
    let row_heights: Vec<f32> = groups
        .iter()
        .take(6)
        .map(|group| {
            if group.subtitle.is_some() {
                subtitled_row_h
            } else {
                title_only_row_h
            }
        })
        .collect();
    let row_count = row_heights.len().max(1);
    let total_row_h: f32 = row_heights.iter().sum();
    let even_gap = ((available_h - total_row_h) / (row_count + 1) as f32).max(row_gap);
    let tile_center_in_row = |row_top: f32, row_h: f32| {
        row_top + pad + (row_h - pad * 2.0 - tile_size) * 0.5 + tile_size * 0.5
    };

    let mut placements = Vec::new();
    let mut labels = Vec::new();
    let mut row_y = top + even_gap;
    let tile_start_x = col_x + label_col_w;

    for (group, &row_h) in groups.iter().take(6).zip(row_heights.iter()) {
        labels.push(TilesExampleLabel {
            title_rect: [col_x + pad, row_y + pad, label_col_w - pad, title_h],
            title: group.label,
            subtitle: group.subtitle,
            accent: group.accent,
        });
        push_guide_tile_placements(
            &mut placements,
            &group.tiles,
            tile_start_x,
            tile_center_in_row(row_y, row_h),
            tile_size,
            tile_gap,
        );
        row_y += row_h + even_gap;
    }

    (placements, labels, vec![])
}

pub(super) fn layout_guide_example_grid(
    cam: &CameraParams,
    groups: &[TileGroup],
    col_x: f32,
    col_w: f32,
    window_w: f32,
    window_h: f32,
    top: f32,
    bottom: f32,
    rows: &[&[usize]],
    row_weights: &[f32],
    tablet_row_reserve: f32,
    cell_layout: GuideExampleCellLayout,
) -> (
    Vec<ShowcaseTilePlacement>,
    Vec<TilesExampleLabel>,
    Vec<(usize, [f32; 4])>,
    Vec<GuideExampleCell>,
) {
    let usable_h = (bottom - top).max(1.0);
    let row_gap = 3.0;
    let weight_sum: f32 = row_weights.iter().sum();
    let mut placements = Vec::new();
    let mut labels = Vec::new();
    let mut panels = Vec::new();
    let mut cells = Vec::new();
    let mut row_y = top;

    for (row_i, indices) in rows.iter().enumerate() {
        let row_weight = row_weights.get(row_i).copied().unwrap_or(1.0);
        let row_h = usable_h * (row_weight / weight_sum) - row_gap * 0.5;
        let cell_ws = tiles_row_cell_widths(indices, groups, col_w, row_gap);
        let mut cell_x = col_x;
        for (col_i, &gi) in indices.iter().enumerate() {
            if gi >= groups.len() {
                continue;
            }
            let cw = cell_ws[col_i];
            let cell = [cell_x, row_y, cw, row_h];
            if groups[gi].framed {
                panels.push((gi, cell));
            }
            let (p, l, tiles_bottom) = layout_tile_group_cell(
                cam,
                &groups[gi],
                cell,
                window_w,
                window_h,
                tablet_row_reserve,
                cell_layout,
            );
            placements.extend(p);
            if let Some(lbl) = l {
                labels.push(lbl);
            }
            cells.push(GuideExampleCell {
                group_index: gi,
                rect: cell,
                tiles_bottom,
            });
            cell_x += cw + row_gap;
        }
        row_y += row_h + row_gap;
    }

    (placements, labels, panels, cells)
}

pub(super) fn tiles_row_cell_widths(
    indices: &[usize],
    groups: &[TileGroup],
    col_w: f32,
    gap: f32,
) -> Vec<f32> {
    let n = indices.len().max(1);
    let usable = (col_w - gap * (n.saturating_sub(1)) as f32).max(1.0);
    let weights: Vec<f32> = indices
        .iter()
        .map(|&i| {
            let tiles = groups.get(i).map(|g| g.tiles.len()).unwrap_or(1);
            tiles as f32 + if tiles >= 4 { 0.6 } else { 0.35 }
        })
        .collect();
    let sum: f32 = weights.iter().sum();
    weights.iter().map(|w| usable * w / sum).collect()
}

pub(super) fn layout_tile_group_cell(
    _cam: &CameraParams,
    group: &TileGroup,
    cell: [f32; 4],
    _window_w: f32,
    window_h: f32,
    tablet_row_reserve: f32,
    cell_layout: GuideExampleCellLayout,
) -> (Vec<ShowcaseTilePlacement>, Option<TilesExampleLabel>, f32) {
    let [cx, cy, cw, ch] = cell;
    let pad = if group.framed { 12.0 } else { 4.0 };
    let title_font = typography::size(typography::H28, window_h);
    let sub_font = typography::size(typography::H45, window_h);
    let inner_w = (cw - pad * 2.0).max(1.0);
    let title_h = title_font * 1.05;
    let sub_line_h = sub_font * 1.02;
    let sub_h = if cell_layout.compact_headers {
        0.0
    } else {
        group
            .subtitle
            .map(|sub| {
                styled_text::colored_line_block_height(
                    sub,
                    inner_w,
                    sub_line_h,
                    color::PARCHMENT,
                    GlossaryMode::Prose,
                )
            })
            .unwrap_or(0.0)
    };
    let label_tile_gap = (window_h * 0.012).clamp(8.0, 14.0);
    let tile_area_top = cy + pad + title_h + sub_h + label_tile_gap;
    let tile_area_h = (cy + ch - pad - tile_area_top - tablet_row_reserve).max(20.0);

    let n = group.tiles.len().max(1);
    let invalid = guide_example_is_invalid(group);
    let max_tile = cell_layout
        .fixed_tile_px
        .map(|px| px.min(tile_area_h * 0.88))
        .unwrap_or_else(|| {
            (cw / (n as f32 + 0.35 + if invalid { 0.25 } else { 0.0 }))
                .min(tile_area_h * 0.88)
                .min(window_h * cell_layout.tile_height_cap)
                .max(24.0)
        });
    let inter_tile_gap = if invalid { max_tile * 0.12 } else { 0.0 };
    let meld_gap = if invalid {
        0.0
    } else {
        guide_example_meld_gap_px(window_h, max_tile)
    };
    let meld_breaks = if invalid {
        vec![false; n]
    } else {
        guide_example_meld_breaks_after(&group.tiles)
    };
    let tile_center_y = (tile_area_top + max_tile * 0.5).min(cy + ch - pad - max_tile * 0.5);
    let group_w = if invalid {
        max_tile * n as f32 + inter_tile_gap * (n.saturating_sub(1) as f32)
    } else {
        guide_example_row_width(max_tile, &group.tiles, meld_gap)
    };
    let start_x = if tablet_row_reserve > 0.0 {
        cx + pad
    } else {
        cx + (cw - group_w) * 0.5
    };
    let mut placements = Vec::with_capacity(n);
    let mut cursor_x = start_x;

    for (tile_i, tile) in group.tiles.iter().enumerate() {
        let px = cursor_x + max_tile * 0.5;
        placements.push(ShowcaseTilePlacement {
            tile: *tile,
            center_pos: [px, tile_center_y, 0.0],
            rotation: if invalid {
                guide_invalid_tile_rotation(tile_i)
            } else {
                DOC_TILE_ROTATION
            },
            scale: 1.0,
            size_px: max_tile,
            brightness: 1.0,
            opacity: 1.0,
            selected: false,
            hovered: false,
            outline: false,
            glow: false,
            glow_color: None,
            outline_sel: None,
            pick_id: None,
            overlay_rect_group: None,
        });
        cursor_x += max_tile;
        if invalid {
            cursor_x += inter_tile_gap;
        } else if meld_breaks.get(tile_i).copied().unwrap_or(false) {
            cursor_x += meld_gap;
        }
    }

    let label = TilesExampleLabel {
        title_rect: [cx + pad, cy + pad, cw - pad * 2.0, title_h],
        title: group.label,
        subtitle: if cell_layout.compact_headers {
            None
        } else {
            group.subtitle
        },
        accent: group.accent,
    };

    let tiles_bottom = tile_center_y + max_tile * 0.5;
    (placements, Some(label), tiles_bottom)
}

pub(super) fn push_tiles_example_panels(
    frame: &mut UiFrame,
    groups: &[TileGroup],
    panels: &[(usize, [f32; 4])],
) {
    for &(gi, rect) in panels {
        let Some(group) = groups.get(gi) else {
            continue;
        };
        let fill = color::alpha(group.accent, 0.10);
        let stroke = color::alpha(group.accent, 0.45);
        frame.quad(GpuInstance {
            rect,
            color: fill,
            user: 0,
        });
        push_guide_panel_stroke(frame, rect, stroke);
    }
}

pub(super) fn push_tiles_example_labels(
    frame: &mut UiFrame,
    _groups: &[TileGroup],
    labels: &[TilesExampleLabel],
    h: f32,
    _scale: f32,
) {
    let title_font = typography::size(typography::H24, h);
    let sub_font = typography::size(typography::H36, h);
    for lbl in labels {
        frame.text(TextLabel {
            rect: lbl.title_rect,
            text: lbl.title.into(),
            color: lbl.accent,
            align: TextAlign::Left,
            font_px: Some(title_font),
            bold: true,
            ..Default::default()
        });
        if let Some(sub) = lbl.subtitle {
            let sub_y = lbl.title_rect[1] + title_font * 1.05;
            let mut labels = Vec::new();
            styled_text::push_colored_line_left(
                &mut labels,
                lbl.title_rect[0],
                sub_y,
                lbl.title_rect[2],
                sub_font * 1.02,
                sub,
                color::PARCHMENT,
                GlossaryMode::Panel,
            );
            for label in labels {
                frame.text(label);
            }
        }
    }
}

pub(super) fn push_tile_group_labels(
    frame: &mut UiFrame,
    labels: &[MeldLabel],
    h: f32,
    scale: f32,
    wrap_long_labels: bool,
) {
    let label_font = typography::size(typography::H42, h);
    let default = color::PARCHMENT;
    for ml in labels {
        let underline_h = (3.0 * scale).max(2.0);
        frame.quad(GpuInstance {
            rect: [ml.x, ml.underline_y, ml.w, underline_h],
            color: ml.color,
            user: 0,
        });
        let mut text_labels = Vec::new();
        if wrap_long_labels {
            let wrapped = styled_text::wrap_colored_text_multiline(
                &ml.text,
                ml.w,
                label_font / 0.99,
                default,
                false,
                GlossaryMode::Prose,
            );
            styled_text::push_colored_rows_in_width(
                &mut text_labels,
                styled_text::ColoredRowsLayout {
                    text_left: ml.x,
                    top_y: ml.y,
                    inner_w: ml.w,
                    line_h: label_font,
                    fallback_plain: &ml.text,
                    fallback_color: default,
                    italic: false,
                    glossary: GlossaryMode::Prose,
                },
                &wrapped,
                TextAlign::Center,
            );
        } else {
            styled_text::push_colored_line_clipped(
                &mut text_labels,
                [ml.x, ml.y, ml.w, label_font * 1.4],
                None,
                &ml.text,
                default,
                label_font,
                TextAlign::Center,
                false,
                GlossaryMode::Prose,
            );
        }
        for label in text_labels {
            frame.text(label);
        }
    }
}
