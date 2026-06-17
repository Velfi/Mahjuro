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
use super::content::{t, tile_group_with_subtitle};
use super::layout::push_guide_panel_stroke;
use super::scoring_diagram::{
    SCORING_FLOW_ARROW_ASPECT, SCORING_FLOW_CASH_IN_STAGE, SCORING_FLOW_STAGES,
    ScoringFlowDiagramLayout, ScoringPanelStyle, push_gameplay_cash_in_overlay,
    push_scoring_flow_panel, scoring_flow_cash_in_visual_rect, scoring_guide_tile_caps,
    scoring_panel_open, scoring_tile_size_for_cell,
};
use super::scoring_page::{
    SCORING_CHIP_GROUPS, SCORING_STRUCTURE_FILLED, SCORING_STRUCTURE_SLOT_COUNT,
};
use super::yaku_detail::push_dense_text;
use crate::ui::input::UiAction;

pub(super) fn tutorial_scoring_flow_meld_group() -> TileGroup {
    tile_group_with_subtitle(
        "5-6-7 Pinzu",
        "Selected meld",
        vec![
            t(Suit::Pinzu, 5, 900),
            t(Suit::Pinzu, 6, 901),
            t(Suit::Pinzu, 7, 902),
        ],
        Suit::Pinzu.keyword_color(),
    )
}

/// Compact scoring-flow diagram for the tutorial campaign (page 4).
pub(crate) fn draw_tutorial_scoring_diagram(
    frame: &mut UiFrame,
    ctx: &DrawCtx<'_>,
    flow_outer: [f32; 4],
    w: f32,
    h: f32,
) {
    let _gap = 10.0;
    let pad = 10.0;
    let (flow_tile_max, _) = scoring_guide_tile_caps(w, h);
    let body_font = typography::size(typography::H36, h);
    let section_font = typography::size(typography::H28, h);
    let small_font = typography::size(typography::H42, h);
    let micro_font = typography::size(typography::H45, h);
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
        std::slice::from_ref(&tutorial_scoring_flow_meld_group()),
        flow_content,
        h,
        flow_tile_max,
        body_font,
        small_font,
        micro_font,
        pad,
        glb_cash_in,
    );
}

pub(super) fn push_scoring_structure_slots(
    frame: &mut UiFrame,
    placements: &mut Vec<ShowcaseTilePlacement>,
    tiles: &[Tile],
    rect: [f32; 4],
    tile_size: f32,
) {
    let gap = 4.0;
    let slot_w = (rect[2] - gap * (SCORING_STRUCTURE_SLOT_COUNT.saturating_sub(1)) as f32)
        / SCORING_STRUCTURE_SLOT_COUNT as f32;
    let slot_h = rect[3];
    for slot_i in 0..SCORING_STRUCTURE_SLOT_COUNT {
        let slot = [
            rect[0] + slot_i as f32 * (slot_w + gap),
            rect[1],
            slot_w,
            slot_h,
        ];
        if slot_i < SCORING_STRUCTURE_FILLED {
            let Some(tile) = tiles.get(slot_i) else {
                push_structure_empty_slot(frame, slot);
                continue;
            };
            let size = scoring_tile_size_for_cell(slot, 1, tile_size);
            placements.extend(layout_tiles_in_cell(
                std::slice::from_ref(tile),
                slot,
                size,
                0.50,
                false,
            ));
        } else {
            push_structure_empty_slot(frame, slot);
        }
    }
}

pub(super) fn push_structure_empty_slot(frame: &mut UiFrame, rect: [f32; 4]) {
    let inset = 2.0;
    let inner = [
        rect[0] + inset,
        rect[1] + inset,
        (rect[2] - inset * 2.0).max(1.0),
        (rect[3] - inset * 2.0).max(1.0),
    ];
    frame.quad(GpuInstance {
        rect: inner,
        color: color::alpha(color::WALNUT_DEEP, 0.40),
        user: 0,
    });
    push_guide_panel_stroke(frame, inner, color::alpha(color::STONE, 0.40));
}

pub(super) fn push_scoring_cash_in_plaque(frame: &mut UiFrame, rect: [f32; 4], body_font: f32) {
    let btn_h = (rect[3] * 0.36).clamp(body_font * 1.20, rect[3] * 0.44);
    let btn_w = (rect[2] * 0.78).clamp(body_font * 3.0, rect[2] * 0.88);
    let btn = [
        rect[0] + (rect[2] - btn_w) * 0.5,
        rect[1] + (rect[3] - btn_h) * 0.5,
        btn_w,
        btn_h,
    ];
    let mut quads = Vec::new();
    let mut labels = Vec::new();
    widget::push_button(
        &mut quads,
        &mut labels,
        &mut Vec::new(),
        widget::ButtonSpec {
            rect: btn,
            label: scoring_intro_copy::FLOW_CASH_IN_BUTTON,
            variant: ButtonVariant::Primary,
            state: ButtonState::Rest,
            action: UiAction::Confirm,
        },
    );
    frame.quads(quads);
    for label in labels {
        frame.text(label);
    }
}

pub(super) fn push_scoring_formula_colored(
    frame: &mut UiFrame,
    rect: [f32; 4],
    text: &str,
    font_px: f32,
) {
    let mut labels = Vec::new();
    let line_h = font_px;
    let drawn_h = styled_text::push_colored_line_left(
        &mut labels,
        rect[0],
        rect[1] + (rect[3] - line_h * styled_text::COLORED_ROW_LINE_STEP_MUL) * 0.5,
        rect[2],
        line_h,
        text,
        color::CHAMPAGNE,
        GlossaryMode::Prose,
    );
    let _ = drawn_h;
    frame.texts(labels);
}

pub(super) fn push_scoring_tile_values_panel(
    frame: &mut UiFrame,
    groups: &[TileGroup],
    content: [f32; 4],
    tile_size: f32,
    caption_font: f32,
    micro_font: f32,
    pad: f32,
) {
    let [cx, cy, cw, ch] = content;
    let caption_h = push_dense_text(
        frame,
        [cx, cy, cw, 0.0],
        scoring_intro_copy::TILE_VALUES_CAPTION,
        micro_font,
        color::alpha(color::PARCHMENT, 0.88),
    );
    let examples_top = cy + caption_h + pad * 0.45;
    let examples_h = (cy + ch - examples_top - pad * 0.15).max(1.0);
    let col_count = SCORING_CHIP_GROUPS.len().max(1);
    let col_gap = pad * 0.65;
    let col_w = (cw - col_gap * (col_count.saturating_sub(1)) as f32) / col_count as f32;
    let value_font = caption_font;
    let name_h = caption_font * 1.02;
    let value_h = value_font * 1.10;
    let text_h = name_h + value_h + pad * 0.25;
    let tile_h = (examples_h - text_h).max(1.0);
    let mut placements = Vec::new();

    for (i, &gi) in SCORING_CHIP_GROUPS.iter().enumerate() {
        let Some(group) = groups.get(gi) else {
            continue;
        };
        let col_x = cx + i as f32 * (col_w + col_gap);
        let tile_area = [col_x + pad * 0.10, examples_top, col_w - pad * 0.20, tile_h];
        let tile_px = scoring_tile_size_for_cell(tile_area, 1, tile_size);
        placements.extend(layout_scoring_group_tiles(
            groups, gi, tile_area, tile_px, 0.50, false,
        ));
        let name_y = examples_top + tile_h + pad * 0.18;
        frame.text(TextLabel {
            rect: [col_x, name_y, col_w, name_h],
            text: group.label.into(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(micro_font),
            bold: true,
            ..Default::default()
        });
        if let Some(subtitle) = group.subtitle {
            frame.text(TextLabel {
                rect: [col_x, name_y + name_h, col_w, value_h],
                text: subtitle.into(),
                color: color::alpha(color::BRASS, 0.95),
                align: TextAlign::Center,
                font_px: Some(value_font),
                bold: true,
                ..Default::default()
            });
        }
    }
    if !placements.is_empty() {
        frame
            .cmds
            .push(DrawCmd::ShowcaseTileBatch(placements.into()));
    }
}

pub(super) fn push_scoring_yaku_relics_panel(
    frame: &mut UiFrame,
    content: [f32; 4],
    caption_font: f32,
    body_font: f32,
    micro_font: f32,
    pad: f32,
) {
    let [x, y, w, h] = content;
    let intro_font = micro_font;
    let mut cursor = y + pad * 0.15;
    for line in [
        scoring_intro_copy::YAKU_RELICS_INTRO,
        scoring_intro_copy::YAKU_RELICS_CASH_IN,
        scoring_intro_copy::YAKU_RELICS_RELICS,
    ] {
        let mut labels = Vec::new();
        let drawn = styled_text::push_colored_line_left(
            &mut labels,
            x,
            cursor,
            w,
            intro_font,
            line,
            color::alpha(color::PARCHMENT, 0.90),
            GlossaryMode::Prose,
        );
        frame.texts(labels);
        cursor += drawn + pad * 0.18;
    }

    let table_top = cursor + pad * 0.25;
    let table_h = (y + h - table_top).max(1.0);
    let header_h = body_font * 1.02;
    let row_h = ((table_h - header_h) / 4.0).max(body_font * 1.05);
    let col_example_w = w * 0.40;
    let col_num_w = (w - col_example_w) * 0.5;
    let header_y = table_top;
    frame.text(TextLabel {
        rect: [x, header_y, col_example_w, header_h],
        text: scoring_intro_copy::YAKU_TABLE_HEADER_EXAMPLE.into(),
        color: color::CHAMPAGNE,
        align: TextAlign::Left,
        font_px: Some(caption_font),
        bold: true,
        ..Default::default()
    });
    frame.text(TextLabel {
        rect: [x + col_example_w, header_y, col_num_w, header_h],
        text: scoring_intro_copy::YAKU_TABLE_HEADER_FU.into(),
        color: color::keyword::FU,
        align: TextAlign::Right,
        font_px: Some(caption_font),
        bold: true,
        ..Default::default()
    });
    frame.text(TextLabel {
        rect: [x + col_example_w + col_num_w, header_y, col_num_w, header_h],
        text: scoring_intro_copy::YAKU_TABLE_HEADER_HAN.into(),
        color: color::keyword::HAN,
        align: TextAlign::Right,
        font_px: Some(caption_font),
        bold: true,
        ..Default::default()
    });
    frame.quad(GpuInstance {
        rect: [x, header_y + header_h - 1.0, w, 1.0],
        color: color::alpha(color::BRASS, 0.42),
        user: 0,
    });

    let rows: [(&str, String, String); 3] = [
        (
            YakuKind::Tanyao.name(),
            format!("+{} Fu", YakuKind::Tanyao.fu_bonus()),
            format!("+{:.1} Han", YakuKind::Tanyao.han_bonus()),
        ),
        (
            YakuKind::Yakuhai.name(),
            format!("+{} Fu", YakuKind::Yakuhai.fu_bonus()),
            format!("+{:.1} Han", YakuKind::Yakuhai.han_bonus()),
        ),
        (
            scoring_intro_copy::YAKU_TABLE_RELIC_ROW,
            format!("+{} Fu", scoring_intro_copy::RELIC_EXAMPLE_FU),
            format!("+{:.1} Han", scoring_intro_copy::RELIC_EXAMPLE_HAN),
        ),
    ];

    let mut row_y = header_y + header_h;
    for (name, fu, han) in rows {
        frame.text(TextLabel {
            rect: [x, row_y, col_example_w, row_h],
            text: name.into(),
            color: color::PARCHMENT,
            align: TextAlign::Left,
            font_px: Some(micro_font),
            ..Default::default()
        });
        frame.text(TextLabel {
            rect: [x + col_example_w, row_y, col_num_w, row_h],
            text: fu,
            color: color::keyword::FU,
            align: TextAlign::Right,
            font_px: Some(micro_font),
            ..Default::default()
        });
        frame.text(TextLabel {
            rect: [x + col_example_w + col_num_w, row_y, col_num_w, row_h],
            text: han,
            color: color::keyword::HAN,
            align: TextAlign::Right,
            font_px: Some(micro_font),
            ..Default::default()
        });
        row_y += row_h;
    }
}

pub(super) fn push_scoring_final_score_panel(
    frame: &mut UiFrame,
    rect: [f32; 4],
    _window_w: f32,
    window_h: f32,
    title: &str,
    section_font: f32,
    micro_font: f32,
    pad: f32,
) {
    let content = scoring_panel_open(frame, rect, title, section_font, ScoringPanelStyle::Formula);
    let [x, y, w, h] = content;
    let eq_font = typography::size(typography::H24, window_h);
    let detail_font = micro_font;
    let eq_h = h * 0.24;
    let detail_h = h * 0.34;
    let example_h = h * 0.18;

    push_scoring_panel_background(
        frame,
        [x, y + 1.0, w, eq_h - 2.0],
        color::alpha(color::GOLD, 0.12),
        color::alpha(color::GOLD, 0.50),
    );
    push_scoring_formula_colored(
        frame,
        [x, y, w, eq_h],
        scoring_intro_copy::FINAL_EQUATION,
        eq_font,
    );

    let detail_y = y + eq_h;
    let detail_line_h = detail_h * 0.5;
    for (i, line) in [
        scoring_intro_copy::FINAL_FU_LINE,
        scoring_intro_copy::FINAL_HAN_LINE,
    ]
    .iter()
    .enumerate()
    {
        let mut labels = Vec::new();
        let _ = styled_text::push_colored_line_left(
            &mut labels,
            x,
            detail_y + detail_line_h * i as f32,
            w,
            detail_font,
            line,
            color::alpha(color::PARCHMENT, 0.92),
            GlossaryMode::Prose,
        );
        frame.texts(labels);
    }

    let example_y = detail_y + detail_h;
    let mut example_labels = Vec::new();
    let _ = styled_text::push_colored_line_left(
        &mut example_labels,
        x,
        example_y + (example_h - detail_font * styled_text::COLORED_ROW_LINE_STEP_MUL) * 0.5,
        w,
        detail_font,
        scoring_intro_copy::FINAL_EXAMPLE,
        color::alpha(color::BRASS, 0.95),
        GlossaryMode::Prose,
    );
    frame.texts(example_labels);

    let _ = pad;
}

pub(super) fn layout_scoring_group_tiles(
    groups: &[TileGroup],
    group_index: usize,
    cell: [f32; 4],
    tile_size: f32,
    y_center: f32,
    align_start: bool,
) -> Vec<ShowcaseTilePlacement> {
    let Some(group) = groups.get(group_index) else {
        return Vec::new();
    };
    layout_tiles_in_cell(&group.tiles, cell, tile_size, y_center, align_start)
}

pub(super) fn layout_tiles_in_cell(
    tiles: &[Tile],
    cell: [f32; 4],
    tile_size: f32,
    y_center: f32,
    align_start: bool,
) -> Vec<ShowcaseTilePlacement> {
    let [cx, cy, cw, ch] = cell;
    let n = tiles.len().max(1);
    let size = tile_size.min(cw / (n as f32 * 0.5 + 0.12)).min(ch * 0.72);
    let row_w = size * n as f32;
    let start_x = if align_start {
        cx + size * 0.22
    } else {
        cx + (cw - row_w) * 0.5
    };
    let center_y = cy + ch * y_center.clamp(0.25, 0.75);
    tiles
        .iter()
        .enumerate()
        .map(|(i, tile)| ShowcaseTilePlacement {
            tile: *tile,
            center_pos: [start_x + size * (i as f32 + 0.5), center_y, 0.0],
            rotation: DOC_TILE_ROTATION,
            scale: 1.0,
            size_px: size,
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
        })
        .collect()
}

pub(super) fn push_scoring_panel_background(
    frame: &mut UiFrame,
    rect: [f32; 4],
    fill: [f32; 4],
    stroke: [f32; 4],
) {
    frame.quad(GpuInstance {
        rect,
        color: fill,
        user: 0,
    });
    push_guide_panel_stroke(frame, rect, stroke);
}
