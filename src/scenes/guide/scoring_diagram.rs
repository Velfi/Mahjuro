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
use super::layout::push_guide_panel_stroke;
use super::scoring_page::{SCORING_FLOW_MELD, SCORING_STRUCTURE_SLOT_COUNT};
use super::scoring_panels::{
    layout_scoring_group_tiles, push_scoring_cash_in_plaque, push_scoring_formula_colored,
    push_scoring_structure_slots,
};

pub(super) fn scoring_section_title(index: u8, title: &str) -> String {
    format!("{index}. {title}")
}

#[derive(Clone, Copy)]
pub(super) enum ScoringPanelStyle {
    Diagram,
    Cards,
    Formula,
}

/// Draw panel chrome and return the inner content rect.
pub(super) fn scoring_panel_open(
    frame: &mut UiFrame,
    rect: [f32; 4],
    title: &str,
    section_font: f32,
    style: ScoringPanelStyle,
) -> [f32; 4] {
    let [x, y, w, h] = rect;
    let header_h = section_font * 1.0 + 8.0;
    let inset = 8.0;
    let (fill, stroke, header_fill, title_color) = match style {
        ScoringPanelStyle::Diagram => (
            color::alpha(color::WALNUT_DEEP, 0.78),
            color::alpha(color::BRASS, 0.42),
            color::alpha(color::WALNUT_RAISED, 0.82),
            color::CHAMPAGNE,
        ),
        ScoringPanelStyle::Cards => (
            color::alpha(color::WALNUT_DEEP, 0.72),
            color::alpha(color::STONE, 0.38),
            color::alpha(color::WALNUT_RAISED, 0.88),
            color::alpha(color::BRASS, 0.95),
        ),
        ScoringPanelStyle::Formula => (
            color::alpha(color::WALNUT_DEEP, 0.94),
            color::alpha(color::GOLD, 0.62),
            color::alpha(color::WALNUT_SOFT, 0.96),
            color::GOLD,
        ),
    };
    frame.quad(GpuInstance {
        rect,
        color: fill,
        user: 0,
    });
    push_scoring_panel_stroke(frame, rect, stroke, style);
    frame.quad(GpuInstance {
        rect: [x, y, w, header_h],
        color: header_fill,
        user: 0,
    });
    let rule_color = match style {
        ScoringPanelStyle::Formula => color::alpha(color::GOLD, 0.75),
        _ => color::alpha(color::GOLD, 0.55),
    };
    frame.quad(GpuInstance {
        rect: [x, y + header_h - 1.0, w, 1.5],
        color: rule_color,
        user: 0,
    });
    frame.text(TextLabel {
        rect: [x + inset, y + 4.0, w - inset * 2.0, header_h - 6.0],
        text: title.into(),
        color: title_color,
        align: TextAlign::Left,
        font_px: Some(section_font),
        bold: true,
        ..Default::default()
    });
    [
        x + inset,
        y + header_h + 6.0,
        w - inset * 2.0,
        (h - header_h - 8.0).max(1.0),
    ]
}

/// Fit showcase tiles inside a cell without clipping panel chrome.
pub(super) fn scoring_tile_size_for_cell(cell: [f32; 4], tile_count: usize, max_px: f32) -> f32 {
    let [_, _, cw, ch] = cell;
    let n = tile_count.max(1) as f32;
    let by_width = cw / (n * 0.58 + 0.30);
    let by_height = ch * 0.68;
    max_px.min(by_width).min(by_height)
}

pub(super) fn push_scoring_panel_stroke(
    frame: &mut UiFrame,
    rect: [f32; 4],
    stroke: [f32; 4],
    style: ScoringPanelStyle,
) {
    push_guide_panel_stroke(frame, rect, stroke);
    if matches!(style, ScoringPanelStyle::Formula) {
        let [x, y, w, h] = rect;
        let inset = 2.0;
        push_guide_panel_stroke(
            frame,
            [x + inset, y + inset, w - inset * 2.0, h - inset * 2.0],
            color::alpha(color::GOLD, 0.22),
        );
    }
}

pub(super) const SCORING_FLOW_ARROW_ASPECT: f32 = 210.0 / 150.0;
pub(super) const SCORING_FLOW_STAGES: usize = 4;
pub(super) const SCORING_FLOW_CASH_IN_STAGE: usize = 2;

pub(super) fn scoring_guide_tile_caps(w: f32, h: f32) -> (f32, f32) {
    let scale = metrics::scene_scale(w, h);
    let flow = (52.0 * scale).max(h * 0.065);
    let values = (56.0 * scale).max(h * 0.070);
    (flow, values)
}

pub(super) fn scoring_flow_ui_scale(content_h: f32) -> f32 {
    (content_h / 420.0).clamp(1.0, 2.8)
}

/// Shared lane geometry for the scoring flow diagram (titles, graphics, arrows).
pub(super) struct ScoringFlowDiagramLayout {
    cx: f32,
    cy: f32,
    cw: f32,
    diagram_h: f32,
    title_h: f32,
    caption_h: f32,
    visual_h: f32,
    lane_w: f32,
    lane_gap: f32,
    arrow_slot_w: f32,
    lane_pad_x: f32,
    graphic_row_h: f32,
    graphic_row_y: f32,
    flow_tile: f32,
    reminder_h: f32,
    arrow_h_max: f32,
}

impl ScoringFlowDiagramLayout {
    fn new(
        content: [f32; 4],
        body_font: f32,
        caption_font: f32,
        micro_font: f32,
        pad: f32,
        tile_max: f32,
    ) -> Self {
        let [cx, cy, cw, ch] = content;
        let reminder_font = micro_font;
        let reminder_h = reminder_font * 1.28;
        let diagram_h = (ch - reminder_h - pad * 1.35).max(1.0);
        let title_h = body_font * 1.02;
        let caption_h = caption_font * 1.12;
        let visual_top = cy + title_h + caption_h + pad * 0.35;
        let visual_h = (cy + diagram_h - visual_top - pad * 0.25).max(1.0);
        let ui_scale = scoring_flow_ui_scale(ch);
        let arrow_h_max = 38.0 * ui_scale;
        let arrow_h = (visual_h * 0.22).clamp(24.0, arrow_h_max);
        let arrow_img_w = arrow_h * SCORING_FLOW_ARROW_ASPECT;
        let arrow_slot_w = arrow_img_w + pad * 0.55;
        let lane_gap = pad * 0.40;
        let lane_w = (cw
            - lane_gap * (SCORING_FLOW_STAGES - 1) as f32
            - arrow_slot_w * (SCORING_FLOW_STAGES - 1) as f32)
            / SCORING_FLOW_STAGES as f32;
        let lane_pad_x = pad * 0.30;
        let lane_inner_w = (lane_w - lane_pad_x * 2.0).max(1.0);
        let probe = [0.0, 0.0, lane_inner_w, visual_h];
        let flow_tile = scoring_tile_size_for_cell(probe, SCORING_STRUCTURE_SLOT_COUNT, tile_max)
            .min(scoring_tile_size_for_cell(probe, 3, tile_max));
        let graphic_row_h = (flow_tile * 1.18).min(visual_h * 0.72);
        let graphic_row_y = visual_top + (visual_h - graphic_row_h) * 0.5;
        Self {
            cx,
            cy,
            cw,
            diagram_h,
            title_h,
            caption_h,
            visual_h,
            lane_w,
            lane_gap,
            arrow_slot_w,
            lane_pad_x,
            graphic_row_h,
            graphic_row_y,
            flow_tile,
            reminder_h,
            arrow_h_max,
        }
    }

    fn from_flow_outer(
        flow_outer: [f32; 4],
        section_font: f32,
        body_font: f32,
        caption_font: f32,
        micro_font: f32,
        pad: f32,
        tile_max: f32,
    ) -> Self {
        Self::new(
            scoring_flow_inner_content_rect(flow_outer, section_font),
            body_font,
            caption_font,
            micro_font,
            pad,
            tile_max,
        )
    }

    fn lane_x(&self, stage: usize) -> f32 {
        self.cx + stage as f32 * (self.lane_w + self.lane_gap + self.arrow_slot_w)
    }

    fn lane_graphic_row(&self, stage: usize) -> [f32; 4] {
        let lane_x = self.lane_x(stage);
        [
            lane_x + self.lane_pad_x,
            self.graphic_row_y,
            (self.lane_w - self.lane_pad_x * 2.0).max(1.0),
            self.graphic_row_h,
        ]
    }

    fn arrow_rect(&self, stage: usize) -> [f32; 4] {
        let lane_x = self.lane_x(stage);
        let arrow_h = (self.visual_h * 0.22).clamp(24.0, self.arrow_h_max);
        let arrow_w = arrow_h * SCORING_FLOW_ARROW_ASPECT;
        let arrow_x =
            lane_x + self.lane_w + (self.lane_gap + self.arrow_slot_w) * 0.5 - arrow_w * 0.5;
        let arrow_y = self.graphic_row_y + self.graphic_row_h * 0.5 - arrow_h * 0.5;
        [arrow_x, arrow_y, arrow_w, arrow_h]
    }
}

/// Graphic-row target for the authored cash-in mesh (step 3).
pub(super) fn scoring_flow_cash_in_visual_rect(
    flow_outer: [f32; 4],
    section_font: f32,
    body_font: f32,
    caption_font: f32,
    micro_font: f32,
    pad: f32,
    tile_max: f32,
) -> [f32; 4] {
    let layout = ScoringFlowDiagramLayout::from_flow_outer(
        flow_outer,
        section_font,
        body_font,
        caption_font,
        micro_font,
        pad,
        tile_max,
    );
    layout.lane_graphic_row(SCORING_FLOW_CASH_IN_STAGE)
}

pub(super) fn scoring_flow_inner_content_rect(flow_outer: [f32; 4], section_font: f32) -> [f32; 4] {
    let [x, y, w, h] = flow_outer;
    let header_h = section_font * 1.0 + 8.0;
    let inset = 8.0;
    [
        x + inset,
        y + header_h + 6.0,
        (w - inset * 2.0).max(1.0),
        (h - header_h - 8.0).max(1.0),
    ]
}

/// Draw the authored gameplay cash-in mesh into `cash_in_visual` via an overlay camera.
pub fn push_gameplay_cash_in_overlay(
    frame: &mut UiFrame,
    ctx: &DrawCtx<'_>,
    w: f32,
    h: f32,
    cash_in_visual: [f32; 4],
    _room_env_key: &'static str,
) -> bool {
    let env_h = ctx.room_gltf_height_scale.max(0.01);
    let Some(cam) = gameplay_glb::gameplay_cash_in_camera_for_screen_rect_if_present(
        w,
        h,
        env_h,
        cash_in_visual,
        0.62,
        true,
    ) else {
        return false;
    };

    // Keep the guide camera for showcase tiles; cash-in glTF draws via overlay camera.
    frame.gameplay_cash_in_overlay_camera = Some(cam);
    frame.gameplay_env_cash_in_only = true;
    frame.gameplay_cash_in_button_visible = true;
    frame.gameplay_cash_in_glow = 0.0;
    frame.gameplay_cash_in_overlay_simple_shade = true;

    let room_glb_lights = gameplay_glb::gameplay_glb_has_embedded_lights();
    let mut overlay_lighting = SceneLighting::default();
    overlay_lighting.room_glb_brdf = room_glb_lights;
    frame.gameplay_cash_in_overlay_lighting = Some(overlay_lighting);
    true
}

pub(super) fn push_scoring_flow_panel(
    frame: &mut UiFrame,
    groups: &[TileGroup],
    content: [f32; 4],
    window_h: f32,
    tile_max: f32,
    body_font: f32,
    caption_font: f32,
    micro_font: f32,
    pad: f32,
    glb_cash_in: bool,
) {
    let flow =
        ScoringFlowDiagramLayout::new(content, body_font, caption_font, micro_font, pad, tile_max);
    let mut placements = Vec::new();
    let mut flow_arrows = Vec::new();

    let steps = [
        (
            scoring_intro_copy::FLOW_STEP_SELECT,
            scoring_intro_copy::FLOW_SELECT_CAPTION,
        ),
        (
            scoring_intro_copy::FLOW_STEP_PLAY,
            scoring_intro_copy::FLOW_PLAY_CAPTION,
        ),
        (
            scoring_intro_copy::FLOW_STEP_CASH_IN,
            scoring_intro_copy::FLOW_CASH_IN_CAPTION,
        ),
        (
            scoring_intro_copy::FLOW_STEP_SCORE,
            scoring_intro_copy::FLOW_SCORE_CAPTION,
        ),
    ];

    for (i, (title, caption)) in steps.iter().enumerate() {
        let lane_x = flow.lane_x(i);
        let lane = [lane_x, flow.cy, flow.lane_w, flow.diagram_h];
        frame.text(TextLabel {
            rect: [lane[0], lane[1], lane[2], flow.title_h],
            text: title.to_string(),
            color: color::CHAMPAGNE,
            align: TextAlign::Center,
            font_px: Some(body_font),
            bold: true,
            ..Default::default()
        });
        frame.text(TextLabel {
            rect: [lane[0], lane[1] + flow.title_h, lane[2], flow.caption_h],
            text: caption.to_string(),
            color: color::alpha(color::PARCHMENT, 0.88),
            align: TextAlign::Center,
            font_px: Some(micro_font),
            ..Default::default()
        });

        let graphic = flow.lane_graphic_row(i);
        match i {
            0 => {
                placements.extend(layout_scoring_group_tiles(
                    groups,
                    SCORING_FLOW_MELD,
                    graphic,
                    flow.flow_tile,
                    0.50,
                    false,
                ));
            }
            1 => {
                let Some(group) = groups.get(SCORING_FLOW_MELD) else {
                    continue;
                };
                push_scoring_structure_slots(
                    frame,
                    &mut placements,
                    &group.tiles,
                    graphic,
                    flow.flow_tile,
                );
            }
            2 => {
                if !glb_cash_in {
                    push_scoring_cash_in_plaque(frame, graphic, body_font);
                }
            }
            3 => {
                let eq_font = typography::size(typography::H24, window_h);
                push_scoring_formula_colored(
                    frame,
                    graphic,
                    scoring_intro_copy::FLOW_SCORE_FORMULA,
                    eq_font,
                );
            }
            _ => {}
        }

        if i + 1 < SCORING_FLOW_STAGES {
            let arrow_rect = flow.arrow_rect(i);
            flow_arrows.push(ImageQuad {
                inst: GpuInstance {
                    rect: arrow_rect,
                    color: [1.0, 1.0, 1.0, 0.95],
                    user: 0,
                },
                source: ImageQuadSource::Asset {
                    path: scoring_intro_copy::FLOW_ARROW_ASSET,
                },
                clip_rect: None,
            });
        }
    }

    if !flow_arrows.is_empty() {
        frame.image_quads(flow_arrows);
    }

    if glb_cash_in {
        frame.gameplay_environment();
    }

    if !placements.is_empty() {
        frame
            .cmds
            .push(DrawCmd::ShowcaseTileBatch(placements.into()));
    }

    frame.text(TextLabel {
        rect: [
            flow.cx,
            flow.cy + flow.diagram_h + pad * 0.35,
            flow.cw,
            flow.reminder_h,
        ],
        text: scoring_intro_copy::FLOW_REMINDER.into(),
        color: color::PARCHMENT,
        align: TextAlign::Center,
        font_px: Some(micro_font),
        ..Default::default()
    });
}
