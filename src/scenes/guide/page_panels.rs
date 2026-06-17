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

use super::layout::push_guide_panel_stroke;
use super::melds_page::{guide_yaku_tablet_metrics, GUIDE_YAKU_TABLET_SCALE};
use super::yaku_detail::push_dense_text_lines;
use super::TileGroup;
pub(super) fn tiles_section_panel_height(
    heading: &str,
    lines: &[&str],
    inner_w: f32,
    section_font: f32,
    body_font: f32,
    line_mul: f32,
    pad: f32,
) -> f32 {
    let heading_h = widget::plain_text_block_height(heading, inner_w, section_font, line_mul);
    let body_line_h = body_font * line_mul;
    let body_h = styled_text::colored_lines_block_height(
        lines,
        inner_w,
        body_line_h,
        color::PARCHMENT,
        GlossaryMode::Panel,
    );
    pad + heading_h + 6.0 + body_h + pad
}

pub(super) fn push_tiles_left_cards(
    frame: &mut UiFrame,
    x: f32,
    w: f32,
    top: f32,
    bottom: f32,
    h: f32,
    body_font: f32,
    line_mul: f32,
) {
    let section_font = typography::size(typography::H28, h);
    let pad = 10.0;
    let inner_w = (w - pad * 2.0).max(1.0);
    let sections: &[(&str, &[&str])] = &[
        (
            tiles_intro_copy::SECTION_NUMBER_SUITS,
            tiles_intro_copy::NUMBER_SUIT_LINES,
        ),
        (
            tiles_intro_copy::SECTION_HONOR_SUITS,
            tiles_intro_copy::HONOR_LINES,
        ),
        (
            tiles_intro_copy::SECTION_FLOWERS,
            tiles_intro_copy::FLOWER_LINES,
        ),
    ];
    push_guide_left_panels(
        frame,
        x,
        w,
        top,
        bottom,
        h,
        body_font,
        line_mul,
        section_font,
        pad,
        inner_w,
        sections,
    );
}

pub(super) fn push_guide_left_panels(
    frame: &mut UiFrame,
    x: f32,
    w: f32,
    top: f32,
    bottom: f32,
    _h: f32,
    body_font: f32,
    line_mul: f32,
    section_font: f32,
    pad: f32,
    inner_w: f32,
    sections: &[(&str, &[&str])],
) {
    let available = (bottom - top).max(1.0);
    let min_gap = 4.0;

    let mut eff_line_mul = line_mul;
    let mut heights: Vec<f32> = sections
        .iter()
        .map(|(heading, lines)| {
            tiles_section_panel_height(
                heading,
                lines,
                inner_w,
                section_font,
                body_font,
                eff_line_mul,
                pad,
            )
        })
        .collect();
    let mut total_natural: f32 =
        heights.iter().sum::<f32>() + min_gap * (sections.len().saturating_sub(1)) as f32;
    if total_natural > available && total_natural > 0.0 {
        eff_line_mul = line_mul * (available / total_natural) * 0.97;
        heights = sections
            .iter()
            .map(|(heading, lines)| {
                tiles_section_panel_height(
                    heading,
                    lines,
                    inner_w,
                    section_font,
                    body_font,
                    eff_line_mul,
                    pad,
                )
            })
            .collect();
        total_natural =
            heights.iter().sum::<f32>() + min_gap * (sections.len().saturating_sub(1)) as f32;
    }
    let section_gap = if sections.len() > 1 && total_natural <= available {
        ((available - total_natural) / (sections.len() - 1) as f32).min(10.0)
    } else {
        min_gap
    };

    let mut y = top;
    let stroke = color::alpha(color::STONE, 0.32);
    let fill = color::alpha(color::WALNUT_RAISED, 0.22);

    for (idx, ((heading, lines), &panel_h)) in sections.iter().zip(heights.iter()).enumerate() {
        frame.quad(GpuInstance {
            rect: [x, y, w, panel_h],
            color: fill,
            user: 0,
        });
        push_guide_panel_stroke(frame, [x, y, w, panel_h], stroke);

        let mut cursor = y + pad;
        cursor += push_dense_text_lines(
            frame,
            [x + pad, cursor, inner_w, 0.0],
            heading,
            section_font,
            color::CHAMPAGNE,
            eff_line_mul,
        );
        cursor += 6.0;

        let body_line_h = body_font * eff_line_mul;
        let mut labels = Vec::new();
        for line in *lines {
            let line_h = styled_text::push_colored_line_left(
                &mut labels,
                x + pad,
                cursor,
                inner_w,
                body_line_h,
                line,
                color::PARCHMENT,
                GlossaryMode::Panel,
            );
            cursor += line_h;
        }
        for label in labels {
            frame.text(label);
        }

        y += panel_h;
        if idx + 1 < sections.len() {
            y += section_gap;
        }
    }
}

/// Label anchors for a tile example cell (title above tiles).
pub(super) struct TilesExampleLabel {
    pub(super) title_rect: [f32; 4],
    pub(super) title: &'static str,
    pub(super) subtitle: Option<&'static str>,
    pub(super) accent: [f32; 4],
}

/// Layout metadata for one guide example cell (tile group + optional yaku tablets).
pub(super) struct GuideExampleCell {
    pub(super) group_index: usize,
    pub(super) rect: [f32; 4],
    pub(super) tiles_bottom: f32,
}

/// Per-page tuning for [`layout_guide_example_grid`] cells.
#[derive(Clone, Copy)]
pub(super) struct GuideExampleCellLayout {
    pub(super) fixed_tile_px: Option<f32>,
    /// Drop subtitle from header reserve and label draw (yaku intro page).
    pub(super) compact_headers: bool,
    pub(super) tile_height_cap: f32,
}

impl Default for GuideExampleCellLayout {
    fn default() -> Self {
        Self {
            fixed_tile_px: None,
            compact_headers: false,
            tile_height_cap: 0.082,
        }
    }
}

/// Yaku scored by a complete example structure (excluding chicken hand).
pub(crate) fn example_structure_yaku(tiles: &[Tile]) -> Vec<YakuKind> {
    let Some(sets) = validate_selection(tiles) else {
        return Vec::new();
    };
    let mut detected: Vec<_> = detect_yaku_with_wind(tiles, &sets, None, None, None)
        .into_iter()
        .filter(|y| *y != YakuKind::ChickenHand)
        .collect();
    detected.sort_by(YakuKind::cmp_by_base_score);
    detected
}

pub(super) fn push_guide_yaku_tablets(
    frame: &mut UiFrame,
    cell: [f32; 4],
    tiles_bottom: f32,
    yaku: &[YakuKind],
    w: f32,
    h: f32,
) {
    if yaku.is_empty() {
        return;
    }
    let (row_h, pill_max_w) = guide_yaku_tablet_metrics(w, h, yaku.len());
    let caption_px = (row_h * 0.36).clamp(24.0, 44.0);
    let gap = 6.0 * ((w.min(h)) / 600.0).max(0.85) * GUIDE_YAKU_TABLET_SCALE;
    let pad = 4.0;
    let clip = ChartClip {
        top: cell[1],
        bottom: cell[1] + cell[3],
    };
    let row_y = tiles_bottom + (h * 0.008).clamp(4.0, 10.0);
    let pill_ws: Vec<f32> = yaku
        .iter()
        .map(|yk| yaku_pill_width(yk.name(), caption_px, row_h).min(pill_max_w))
        .collect();
    let mut x = cell[0] + pad;
    let mut squircles = Vec::new();
    let mut labels = Vec::new();
    let face = yaku_pill_face();
    let ink = yaku_pill_ink();
    let rim = yaku_pill_rim();
    for (&yk, &pill_w) in yaku.iter().zip(pill_ws.iter()) {
        let drawn_w = push_yaku_pill(
            &mut squircles,
            &mut labels,
            clip,
            x,
            row_y,
            row_h,
            yk.name(),
            pill_w,
            face,
            ink,
            rim,
            caption_px,
        );
        x += drawn_w + gap;
    }
    frame.squircle_quads(squircles);
    for label in labels {
        frame.text(label);
    }
}

pub(super) fn push_guide_tile_placements(
    placements: &mut Vec<ShowcaseTilePlacement>,
    tiles: &[Tile],
    start_x: f32,
    center_y: f32,
    tile_size: f32,
    tile_gap: f32,
) {
    let mut cursor_x = start_x;
    for tile in tiles {
        let px = cursor_x + tile_size * 0.5;
        placements.push(ShowcaseTilePlacement {
            tile: *tile,
            center_pos: [px, center_y, 0.0],
            rotation: DOC_TILE_ROTATION,
            scale: 1.0,
            size_px: tile_size,
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
        cursor_x += tile_size + tile_gap;
    }
}
