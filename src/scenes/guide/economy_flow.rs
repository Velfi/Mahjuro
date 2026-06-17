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

use super::economy::{
    draw_dot_leader_row, draw_earning_note_row, draw_economy_panel_header, economy_card_body_font,
    economy_measure_text_width, push_economy_item_example, ECONOMY_ICON_COL_FRAC,
    ECONOMY_ITEM_COLS, ECONOMY_ITEM_EXAMPLES, ECONOMY_ITEM_ROWS, EconomyItemExample,
};
use super::layout::push_guide_panel_stroke;
use super::GuideLayout;

pub(super) fn economy_flow_block_inner_pad(pad: f32) -> f32 {
    pad * 0.85
}

pub(super) fn economy_flow_block_line_gap(pad: f32) -> f32 {
    pad * 0.22
}

pub(super) fn economy_flow_font_line_height(font_px: f32, italic: bool) -> f32 {
    let base = load_ui_font()
        .and_then(|font| font.horizontal_line_metrics(font_px))
        .map(|lm| lm.new_line_size)
        .unwrap_or(font_px * 1.25);
    if italic { base * 1.10 } else { base }
}

pub(super) fn economy_flow_wrap_line_h(font_px: f32) -> f32 {
    font_px / 0.99
}

pub(super) fn economy_flow_wrapped_line_count(text: &str, text_w: f32, font_px: f32) -> usize {
    wrap_text(text, text_w.max(1.0), economy_flow_wrap_line_h(font_px))
        .len()
        .max(1)
}

pub(super) fn economy_flow_wrapped_text(text: &str, text_w: f32, font_px: f32) -> String {
    wrap_text(text, text_w.max(1.0), economy_flow_wrap_line_h(font_px)).join("\n")
}

pub(super) fn economy_flow_badge_size(label_font: f32) -> f32 {
    label_font * 1.05
}

pub(super) fn economy_flow_header_metrics(
    step: &economy_intro_copy::FlowStep,
    label_font: f32,
    pad: f32,
    block_w: f32,
) -> (f32, f32, f32, f32) {
    let inner_pad = economy_flow_block_inner_pad(pad);
    let label_font_px = label_font;
    let badge = economy_flow_badge_size(label_font);
    let badge_gap = inner_pad * 0.55;
    let text_w = (block_w - inner_pad * 2.0).max(1.0);
    let title_w = (text_w - badge - badge_gap).max(1.0);
    let title_lines =
        economy_flow_wrapped_line_count(step.label, title_w, label_font_px).max(1) as f32;
    let title_h = economy_flow_font_line_height(label_font_px, false) * title_lines;
    let header_h = title_h.max(badge);
    (badge, badge_gap, title_w, header_h)
}

pub(super) fn economy_flow_block_height_at_width(
    step: &economy_intro_copy::FlowStep,
    label_font: f32,
    line_font: f32,
    pad: f32,
    block_w: f32,
) -> f32 {
    let inner_pad = economy_flow_block_inner_pad(pad);
    let line_font_px = line_font;
    let text_w = (block_w - inner_pad * 2.0).max(1.0);
    let (_, _, _, header_h) = economy_flow_header_metrics(step, label_font, pad, block_w);
    let body_line_h = economy_flow_font_line_height(line_font_px, true);
    let body_lines = economy_flow_wrapped_line_count(step.line, text_w, line_font_px).max(2) as f32;
    inner_pad
        + header_h
        + economy_flow_block_line_gap(pad)
        + body_line_h * body_lines
        + inner_pad
        + pad * 0.18
}

pub(super) fn economy_flow_block_natural_width(
    step: &economy_intro_copy::FlowStep,
    label_font: f32,
    line_font: f32,
    pad: f32,
) -> f32 {
    let inner_pad = economy_flow_block_inner_pad(pad);
    let label_font_px = label_font;
    let line_font_px = line_font;
    let badge = economy_flow_badge_size(label_font);
    let badge_gap = inner_pad * 0.55;
    let label_w = economy_measure_text_width(step.label, label_font_px);
    let line_w = economy_measure_text_width(step.line, line_font_px);
    let header_w = inner_pad + badge + badge_gap + label_w + inner_pad;
    let body_w = inner_pad * 2.0 + line_w;
    header_w.max(body_w).max(120.0)
}

pub(super) fn draw_economy_flow_block(
    frame: &mut UiFrame,
    rect: [f32; 4],
    step: &economy_intro_copy::FlowStep,
    label_font: f32,
    line_font: f32,
    pad: f32,
    window_h: f32,
) {
    let badge_font = typography::size(typography::H45, window_h);
    let [x, y, w, h] = rect;
    frame.quad(GpuInstance {
        rect,
        color: color::alpha(color::WALNUT_SOFT, 0.52),
        user: 0,
    });
    push_guide_panel_stroke(frame, rect, color::alpha(color::BRASS, 0.42));

    let inner_pad = economy_flow_block_inner_pad(pad);
    let text_w = (w - inner_pad * 2.0).max(1.0);
    let label_font_px = label_font;
    let (badge, badge_gap, title_w, header_h) =
        economy_flow_header_metrics(step, label_font, pad, w);
    let title_y = y + inner_pad;
    let badge_rect = [x + inner_pad, title_y, badge, badge];
    frame.quad(GpuInstance {
        rect: badge_rect,
        color: color::alpha(color::WALNUT_DEEP, 0.94),
        user: 0,
    });
    push_guide_panel_stroke(frame, badge_rect, color::alpha(color::BRASS, 0.38));
    frame.text(TextLabel {
        rect: badge_rect,
        text: step.num.to_string(),
        color: color::CHAMPAGNE,
        align: TextAlign::Center,
        font_px: Some(badge_font),
        bold: true,
        ..Default::default()
    });

    let title_x = badge_rect[0] + badge + badge_gap;
    let title_h = economy_flow_font_line_height(label_font_px, false)
        * economy_flow_wrapped_line_count(step.label, title_w, label_font_px).max(1) as f32;
    frame.text(TextLabel {
        rect: [title_x, title_y, title_w, header_h.max(title_h)],
        text: economy_flow_wrapped_text(step.label, title_w, label_font_px),
        color: color::CHAMPAGNE,
        align: TextAlign::Left,
        block_vertical_align: TextBlockVerticalAlign::Top,
        font_px: Some(label_font_px),
        bold: true,
        ..Default::default()
    });

    let line_font_px = line_font;
    let line_y = title_y + header_h + economy_flow_block_line_gap(pad);
    let body_line_h = economy_flow_font_line_height(line_font_px, true);
    let body_lines = economy_flow_wrapped_line_count(step.line, text_w, line_font_px).max(2) as f32;
    let body_content_h = body_line_h * body_lines;
    let line_h = (y + h - inner_pad - line_y).max(body_content_h);
    frame.text(TextLabel {
        rect: [x + inner_pad, line_y, text_w, line_h],
        text: economy_flow_wrapped_text(step.line, text_w, line_font_px),
        color: color::alpha(color::STONE, 0.88),
        align: TextAlign::Left,
        block_vertical_align: TextBlockVerticalAlign::Top,
        font_px: Some(line_font_px),
        italic: true,
        ..Default::default()
    });
}

pub(super) fn economy_flow_ring_block_sizes(label_font: f32, line_font: f32, pad: f32) -> [f32; 2] {
    let block_w = economy_intro_copy::FLOW_STEPS
        .iter()
        .map(|step| economy_flow_block_natural_width(step, label_font, line_font, pad))
        .fold(0.0f32, f32::max)
        .max(1.0);
    let block_h = economy_intro_copy::FLOW_STEPS
        .iter()
        .map(|step| economy_flow_block_height_at_width(step, label_font, line_font, pad, block_w))
        .fold(0.0f32, f32::max)
        .max(1.0);
    [block_w, block_h]
}

pub(super) struct EconomyFlowRingLayout {
    pub(super) label_font: f32,
    pub(super) line_font: f32,
    pub(super) block_w: f32,
    pub(super) block_h: f32,
    pub(super) ring_w: f32,
    pub(super) ring_h: f32,
    pub(super) arrow_font: f32,
    pub(super) h_gutter: f32,
    pub(super) v_gutter: f32,
}

pub(super) fn economy_flow_ring_layout(
    window_h: f32,
    _caption_font: f32,
    pad: f32,
    max_w: f32,
    max_h: f32,
) -> EconomyFlowRingLayout {
    let arrow_font = typography::size(typography::H36, window_h);
    let h_gutter = arrow_font * 1.15;
    let v_gutter = arrow_font * 1.10;
    let mut label_font = typography::size(typography::H36, window_h);
    let mut line_font = typography::size(typography::H42, window_h);
    let [mut block_w, mut block_h] = economy_flow_ring_block_sizes(label_font, line_font, pad);
    let mut ring_w = block_w * 2.0 + h_gutter;
    let mut ring_h = block_h * 2.0 + v_gutter;
    if ring_w > max_w || ring_h > max_h {
        let scale = (max_w / ring_w).min(max_h / ring_h).min(1.0);
        label_font = typography::tier_at_most(label_font * scale, window_h);
        line_font = typography::tier_at_most(line_font * scale, window_h);
        [block_w, block_h] = economy_flow_ring_block_sizes(label_font, line_font, pad);
        ring_w = block_w * 2.0 + h_gutter;
        ring_h = block_h * 2.0 + v_gutter;
    }
    EconomyFlowRingLayout {
        label_font,
        line_font,
        block_w,
        block_h,
        ring_w,
        ring_h,
        arrow_font,
        h_gutter,
        v_gutter,
    }
}

pub(super) fn economy_flow_panel_width(full_w: f32, panel_gap: f32, ring_w: f32) -> f32 {
    let available = (full_w - panel_gap).max(1.0);
    let chrome_w = 32.0;
    let min_w = available * 0.28;
    let max_w = available * 0.50;
    (ring_w + chrome_w).clamp(min_w, max_w)
}

pub(super) fn push_economy_flow_ring_arrow(frame: &mut UiFrame, rect: [f32; 4], glyph: &str, font: f32) {
    let [x, y, w, h] = rect;
    let side = font * 1.05;
    let cx = x + w * 0.5;
    let cy = y + h * 0.5;
    frame.text(TextLabel {
        rect: [cx - side * 0.5, cy - side * 0.5, side, side],
        text: glyph.into(),
        color: color::alpha(color::CHAMPAGNE, 0.90),
        align: TextAlign::Center,
        font_px: Some(font),
        ..Default::default()
    });
}

pub(super) fn draw_between_chambers_band(
    frame: &mut UiFrame,
    content: [f32; 4],
    window_h: f32,
    _body_font: f32,
    caption_font: f32,
    _micro_font: f32,
    pad: f32,
) {
    let [cx, cy, cw, ch] = content;
    let ring = economy_flow_ring_layout(window_h, caption_font, pad, cw, ch);

    let origin_x = cx + (cw - ring.ring_w) * 0.5;
    let origin_y = cy + (ch - ring.ring_h) * 0.5;
    let x0 = origin_x;
    let x1 = origin_x + ring.block_w + ring.h_gutter;
    let y0 = origin_y;
    let y1 = origin_y + ring.block_h + ring.v_gutter;

    let block_rects = [
        [x0, y0, ring.block_w, ring.block_h],
        [x1, y0, ring.block_w, ring.block_h],
        [x1, y1, ring.block_w, ring.block_h],
        [x0, y1, ring.block_w, ring.block_h],
    ];

    push_economy_flow_ring_arrow(
        frame,
        [x0 + ring.block_w, y0, ring.h_gutter, ring.block_h],
        "\u{27a1}",
        ring.arrow_font,
    );
    push_economy_flow_ring_arrow(
        frame,
        [x1, y0 + ring.block_h, ring.block_w, ring.v_gutter],
        "\u{2b07}",
        ring.arrow_font,
    );
    push_economy_flow_ring_arrow(
        frame,
        [x0 + ring.block_w, y1, ring.h_gutter, ring.block_h],
        "\u{2b05}",
        ring.arrow_font,
    );
    push_economy_flow_ring_arrow(
        frame,
        [x0, y0 + ring.block_h, ring.block_w, ring.v_gutter],
        "\u{2b06}",
        ring.arrow_font,
    );

    for (step, block_rect) in economy_intro_copy::FLOW_STEPS.iter().zip(block_rects) {
        draw_economy_flow_block(
            frame,
            block_rect,
            step,
            ring.label_font,
            ring.line_font,
            pad,
            window_h,
        );
    }
}

pub(super) fn draw_skip_steps_column(
    frame: &mut UiFrame,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    font: f32,
    pad: f32,
    body_color: [f32; 4],
) {
    let steps = economy_intro_copy::SKIP_PATH_STEPS;
    let lines = economy_intro_copy::SKIP_LINES;
    let n = steps.len().min(lines.len());
    if n == 0 {
        return;
    }

    let step_color = color::alpha(color::CHAMPAGNE, 0.92);
    let label_font = font * 1.02;
    let line_font = font * 0.96;
    let label_h = label_font * 1.12;
    let line_gap = pad * 0.14;
    let block_h = (h / n as f32).max(label_h + line_font * 1.2 + line_gap);

    for i in 0..n {
        let block_y = y + i as f32 * block_h;
        frame.text(TextLabel {
            rect: [x, block_y, w, label_h],
            text: steps[i].into(),
            color: step_color,
            align: TextAlign::Left,
            font_px: Some(label_font),
            bold: true,
            ..Default::default()
        });
        frame.text(TextLabel {
            rect: [
                x,
                block_y + label_h + line_gap,
                w,
                block_h - label_h - line_gap,
            ],
            text: lines[i].into(),
            color: body_color,
            align: TextAlign::Left,
            font_px: Some(line_font),
            ..Default::default()
        });
    }
}

