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

use super::TileGroup;
use super::example_grid::{
    layout_tiles_page_grid, push_tile_group_labels, push_tiles_example_labels,
    push_tiles_example_panels,
};
use super::flowers_page::{guide_column_footer_height, push_guide_column_footer_prose};
use super::layout::GuideLayout;
use super::page_panels::push_tiles_left_cards;

// ── Tiles intro page (page 0) ─────────────────────────────────────────────

pub(super) fn draw_tiles_page(
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
    let left_w = layout.content_w * 0.38;
    let gutter = layout.content_w * 0.02;
    let right_w = layout.content_w - left_w - gutter;
    let left_x = layout.content_x;
    let right_x = left_x + left_w + gutter;
    let columns_bottom = content_floor - h * 0.006;
    let body_font = typography::size(typography::H42, h);
    let line_mul = 1.12;
    let footer_h = guide_column_footer_height(left_w, h, body_font, tiles_intro_copy::INTRO);

    push_tiles_left_cards(
        frame,
        left_x,
        left_w,
        content_top,
        columns_bottom - footer_h,
        h,
        body_font,
        line_mul,
    );
    push_guide_column_footer_prose(
        frame,
        left_x,
        left_w,
        columns_bottom,
        h,
        body_font,
        tiles_intro_copy::INTRO,
    );

    let (placements, labels, panels) = layout_tiles_page_grid(
        cam,
        groups,
        right_x,
        right_w,
        w,
        h,
        content_top,
        columns_bottom,
    );
    push_tiles_example_panels(frame, groups, &panels);
    if !placements.is_empty() {
        frame
            .cmds
            .push(DrawCmd::ShowcaseTileBatch(placements.into()));
    }
    push_tiles_example_labels(frame, groups, &labels, h, scale);
}

// ── Melds page (page 1) ───────────────────────────────────────────────────

pub(super) const MELDS_EXAMPLE_ROWS: &[&[usize]] = &[&[0, 1], &[2, 3, 4], &[5, 6]];
pub(super) const MELDS_ROW_WEIGHTS: [f32; 3] = [0.28, 0.38, 0.34];
