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

use super::scoring_diagram::{
    push_gameplay_cash_in_overlay, push_scoring_flow_panel, scoring_flow_cash_in_visual_rect,
    scoring_guide_tile_caps, scoring_panel_open, scoring_section_title, ScoringPanelStyle,
};
use super::scoring_panels::{
    push_scoring_final_score_panel, push_scoring_tile_values_panel, push_scoring_yaku_relics_panel,
};
use super::{GuideLayout, TileGroup};

// ── Scoring page (page 4) ─────────────────────────────────────────────────

pub(super) const SCORING_FLOW_MELD: usize = 0;
pub(super) const SCORING_CHIP_GROUPS: &[usize] = &[1, 2, 3, 4];
pub(super) const SCORING_STRUCTURE_SLOT_COUNT: usize = 6;
pub(super) const SCORING_STRUCTURE_FILLED: usize = 3;

pub(super) fn draw_scoring_page(
    frame: &mut UiFrame,
    ctx: &DrawCtx<'_>,
    layout: &GuideLayout,
    _progress: &PlayerProgress,
    w: f32,
    h: f32,
    _scale: f32,
    groups: &[TileGroup],
    content_top: f32,
    content_floor: f32,
) {
    let gap = 10.0;
    let pad = 10.0;
    let (flow_tile_max, values_tile_max) = scoring_guide_tile_caps(w, h);
    let body_font = typography::size(typography::H36, h);
    let section_font = typography::size(typography::H28, h);
    let small_font = typography::size(typography::H42, h);
    let micro_font = typography::size(typography::H45, h);

    let x = layout.content_x;
    let full_w = layout.content_w;
    let mut y = content_top;
    let usable = (content_floor - y).max(1.0);
    let flow_h = usable * 0.50;
    let bottom_h = usable - flow_h - gap;

    let flow_outer = [x, y, full_w, flow_h];
    let cash_in_visual = scoring_flow_cash_in_visual_rect(
        flow_outer,
        section_font,
        body_font,
        small_font,
        micro_font,
        pad,
        flow_tile_max,
    );
    let glb_cash_in =
        push_gameplay_cash_in_overlay(frame, ctx, w, h, cash_in_visual, scene_keys::GAMEPLAY);

    let flow_content = scoring_panel_open(
        frame,
        flow_outer,
        scoring_intro_copy::SECTION_FLOW,
        section_font,
        ScoringPanelStyle::Diagram,
    );
    push_scoring_flow_panel(
        frame,
        groups,
        flow_content,
        h,
        flow_tile_max,
        body_font,
        small_font,
        micro_font,
        pad,
        glb_cash_in,
    );
    y += flow_h + gap;

    let panel_gap = gap;
    let panel_w = (full_w - panel_gap * 2.0) / 3.0;
    let tiles_content = scoring_panel_open(
        frame,
        [x, y, panel_w, bottom_h],
        &scoring_section_title(1, scoring_intro_copy::SECTION_TILE_VALUES),
        section_font,
        ScoringPanelStyle::Cards,
    );
    push_scoring_tile_values_panel(
        frame,
        groups,
        tiles_content,
        values_tile_max,
        small_font,
        micro_font,
        pad,
    );

    let yaku_content = scoring_panel_open(
        frame,
        [x + panel_w + panel_gap, y, panel_w, bottom_h],
        &scoring_section_title(2, scoring_intro_copy::SECTION_YAKU_RELICS),
        section_font,
        ScoringPanelStyle::Cards,
    );
    push_scoring_yaku_relics_panel(frame, yaku_content, small_font, body_font, micro_font, pad);

    push_scoring_final_score_panel(
        frame,
        [x + (panel_w + panel_gap) * 2.0, y, panel_w, bottom_h],
        w,
        h,
        &scoring_section_title(3, scoring_intro_copy::SECTION_FINAL_SCORE),
        section_font,
        micro_font,
        pad,
    );
}
