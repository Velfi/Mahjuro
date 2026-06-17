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
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextBlockVerticalAlign, TextLabel};
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

use super::content::page_graffiti;
use super::example_grid::{
    layout_guide_example_grid, push_tile_group_labels, push_tiles_example_labels,
    push_tiles_example_panels,
};
use super::layout::GuideLayout;
use super::page_panels::{GuideExampleCellLayout, push_guide_left_panels};
use super::yaku_intro_page::{FLOWERS_EXAMPLE_ROWS, FLOWERS_ROW_WEIGHTS};
use super::{PAGE_FLOWERS, TileGroup};
pub(super) fn draw_flowers_page(
    frame: &mut UiFrame,
    layout: &GuideLayout,
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

    push_flowers_left_cards(
        frame,
        left_x,
        left_w,
        content_top,
        columns_bottom,
        h,
        body_font,
        line_mul,
    );
    if let Some(scrawl) = page_graffiti(PAGE_FLOWERS) {
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
        FLOWERS_EXAMPLE_ROWS,
        &FLOWERS_ROW_WEIGHTS,
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

pub(super) fn push_flowers_left_cards(
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
    let sections: [(&str, &[&str]); 2] = [
        (
            flowers_intro_copy::SECTION_ALLOWED,
            flowers_intro_copy::ALLOWED_LINES,
        ),
        (
            flowers_intro_copy::SECTION_NOT_ALLOWED,
            flowers_intro_copy::NOT_ALLOWED_LINES,
        ),
    ];
    push_guide_left_panels(
        frame,
        x,
        w,
        top,
        bottom - h * 0.14,
        h,
        body_font,
        line_mul,
        section_font,
        pad,
        inner_w,
        &sections,
    );
}

pub(super) fn guide_column_prose_height(w: f32, body_font: f32, text: &str) -> f32 {
    let pad = 10.0;
    let inner_w = (w - pad * 2.0).max(1.0);
    styled_text::ColoredLineBlock::measure(
        text,
        inner_w,
        body_font,
        color::PARCHMENT,
        GlossaryMode::Prose,
    )
    .height()
}

pub(super) fn push_guide_column_prose(
    frame: &mut UiFrame,
    x: f32,
    w: f32,
    top: f32,
    body_font: f32,
    text: &str,
) {
    let pad = 10.0;
    let inner_w = (w - pad * 2.0).max(1.0);
    let mut labels = Vec::new();
    styled_text::push_colored_line_left(
        &mut labels,
        x + pad,
        top,
        inner_w,
        body_font,
        text,
        color::PARCHMENT,
        GlossaryMode::Prose,
    );
    frame.texts(labels);
}

pub(super) fn guide_column_footer_height(w: f32, h: f32, body_font: f32, text: &str) -> f32 {
    guide_column_prose_height(w, body_font, text) + h * 0.016
}

pub(super) fn push_guide_column_footer_prose(
    frame: &mut UiFrame,
    x: f32,
    w: f32,
    bottom: f32,
    h: f32,
    body_font: f32,
    text: &str,
) {
    let block_h = guide_column_prose_height(w, body_font, text);
    let y = bottom - block_h - h * 0.008;
    push_guide_column_prose(frame, x, w, y, body_font, text);
}

pub(super) fn push_flowers_margin_scrawl(
    frame: &mut UiFrame,
    x: f32,
    w: f32,
    bottom: f32,
    h: f32,
    text: &str,
) {
    let font = typography::size(typography::H42, h);
    let pad = 10.0;
    let inner_w = (w - pad * 2.0).max(1.0);
    let default = color::alpha(color::STONE, 0.72);
    let wrapped = styled_text::wrap_colored_text_multiline(
        text,
        inner_w,
        font / 0.99,
        default,
        true,
        GlossaryMode::Prose,
    );
    let line_h = font;
    let block_h = styled_text::colored_wrapped_rows_height(&wrapped, line_h);
    let y = bottom - block_h - h * 0.008;
    let mut labels = Vec::new();
    styled_text::push_colored_rows_left(
        &mut labels,
        styled_text::ColoredRowsLayout {
            text_left: x + pad,
            top_y: y,
            inner_w,
            line_h,
            fallback_plain: text,
            fallback_color: default,
            italic: true,
            glossary: GlossaryMode::Prose,
        },
        &wrapped,
    );
    frame.texts(labels);
}

pub(super) fn push_melds_left_cards(
    frame: &mut UiFrame,
    x: f32,
    w: f32,
    top: f32,
    bottom: f32,
    h: f32,
    body_font: f32,
    line_mul: f32,
    intro: Option<&str>,
) {
    let section_font = typography::size(typography::H28, h);
    let pad = 10.0;
    let inner_w = (w - pad * 2.0).max(1.0);
    let mut panels_top = top;
    if let Some(text) = intro {
        push_guide_column_prose(frame, x, w, panels_top, body_font, text);
        panels_top += guide_column_prose_height(w, body_font, text) + h * 0.012;
    }
    let sections: [(&str, &[&str]); 1] = [(
        melds_intro_copy::SECTION_SEQUENCE_RULES,
        melds_intro_copy::SEQUENCE_RULE_LINES,
    )];
    push_guide_left_panels(
        frame,
        x,
        w,
        panels_top,
        bottom,
        h,
        body_font,
        line_mul,
        section_font,
        pad,
        inner_w,
        &sections,
    );
}
