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

use super::GuideLayout;
use super::economy_flow::{
    draw_between_chambers_band, draw_skip_steps_column, economy_flow_panel_width,
    economy_flow_ring_layout,
};
use super::economy_rules::{draw_economy_rules_band, push_economy_item_cards};
use super::layout::push_guide_panel_stroke;
use super::scoring_diagram::{ScoringPanelStyle, scoring_panel_open};

// ── Economy page (page 5) ─────────────────────────────────────────────────

pub(super) const ECONOMY_ITEM_COLS: usize = 3;
pub(super) const ECONOMY_ITEM_ROWS: usize = 2;
pub(super) const ECONOMY_ICON_COL_FRAC: f32 = 0.26;

#[derive(Clone, Copy)]
pub(super) enum EconomyItemExample {
    Relic(RelicId),
    Zodiac(ZodiacKind),
    Talisman(TalismanKind),
    TilePack(TilePackKind),
    Memorial(MemorialTalismanKind),
    Temptation(TagKind),
}

pub(super) const ECONOMY_ITEM_EXAMPLES: [EconomyItemExample; 6] = [
    EconomyItemExample::Relic(RelicId::GoldIdol),
    EconomyItemExample::Zodiac(ZodiacKind::Dog),
    EconomyItemExample::Talisman(TalismanKind::Pearl),
    EconomyItemExample::TilePack(TilePackKind::Flowers),
    EconomyItemExample::Memorial(MemorialTalismanKind::Hoarder),
    EconomyItemExample::Temptation(TagKind::GoldIngot),
];

pub(super) fn draw_economy_page(
    frame: &mut UiFrame,
    layout: &GuideLayout,
    w: f32,
    h: f32,
    cam: &CameraParams,
    content_top: f32,
    content_floor: f32,
) {
    let gap = 10.0;
    let pad = 10.0;
    let body_font = typography::size(typography::H36, h);
    let section_font = typography::size(typography::H28, h);
    let small_font = typography::size(typography::H42, h);
    let micro_font = typography::size(typography::H45, h);
    let x = layout.content_x;
    let full_w = layout.content_w;
    let mut y = content_top;
    let usable = (content_floor - y).max(1.0);
    let top_row_h = usable * 0.50;
    let items_h = usable - top_row_h - gap;

    let panel_gap = gap;
    let flow_header_h = section_font * 1.0 + 8.0;
    let flow_content_h = (top_row_h - flow_header_h - 8.0).max(1.0);
    let ring = economy_flow_ring_layout(h, small_font, pad, f32::MAX, flow_content_h);
    let flow_w = economy_flow_panel_width(full_w, panel_gap, ring.ring_w);
    let rules_w = full_w - panel_gap - flow_w;

    let flow_content = scoring_panel_open(
        frame,
        [x, y, flow_w, top_row_h],
        economy_intro_copy::SECTION_BETWEEN_CHAMBERS,
        section_font,
        ScoringPanelStyle::Diagram,
    );
    draw_between_chambers_band(
        frame,
        flow_content,
        h,
        body_font,
        small_font,
        micro_font,
        pad,
    );

    let rules_content = scoring_panel_open(
        frame,
        [x + flow_w + panel_gap, y, rules_w, top_row_h],
        economy_intro_copy::SECTION_ECONOMY_RULES,
        section_font,
        ScoringPanelStyle::Cards,
    );
    draw_economy_rules_band(frame, rules_content, body_font, micro_font, pad);
    y += top_row_h + gap;

    push_economy_item_cards(
        frame,
        layout,
        w,
        h,
        cam,
        [x, y, full_w, items_h.max(1.0)],
        small_font,
        body_font,
        pad,
        gap,
    );
}

pub(super) fn zodiac_icon_asset(kind: ZodiacKind) -> &'static str {
    match kind {
        ZodiacKind::Mouse => "textures/zodiacs/zodiac_mouse.png",
        ZodiacKind::Rat => "textures/zodiacs/zodiac_rat.png",
        ZodiacKind::Ox => "textures/zodiacs/zodiac_ox.png",
        ZodiacKind::Tiger => "textures/zodiacs/zodiac_tiger.png",
        ZodiacKind::Rabbit => "textures/zodiacs/zodiac_rabbit.png",
        ZodiacKind::Dragon => "textures/zodiacs/zodiac_dragon.png",
        ZodiacKind::Snake => "textures/zodiacs/zodiac_snake.png",
        ZodiacKind::Horse => "textures/zodiacs/zodiac_horse.png",
        ZodiacKind::Goat => "textures/zodiacs/zodiac_goat.png",
        ZodiacKind::Monkey => "textures/zodiacs/zodiac_monkey.png",
        ZodiacKind::Rooster => "textures/zodiacs/zodiac_rooster.png",
        ZodiacKind::Dog => "textures/zodiacs/zodiac_dog.png",
        ZodiacKind::Pig => "textures/zodiacs/zodiac_pig.png",
        ZodiacKind::Qilin => "textures/zodiacs/zodiac_qilin.png",
        ZodiacKind::Phoenix => "textures/zodiacs/zodiac_phoenix.png",
        ZodiacKind::Crane => "textures/zodiacs/zodiac_crane.png",
        ZodiacKind::Koi => "textures/zodiacs/zodiac_koi.png",
    }
}

pub(super) fn economy_relic_extents(relic_id: RelicId, icon_span: f32) -> ([f32; 3], f32) {
    let visual = relic_visual(relic_id);
    let base = icon_span * 0.32;
    let seed = (relic_id as u32).wrapping_mul(2654435761) ^ 0x9E3779B9;
    let r0 = ((seed >> 8) & 0xFF) as f32 / 255.0;
    let r2 = ((seed >> 24) & 0xFF) as f32 / 255.0;
    let face = base * (0.65 + r0 * 0.45);
    let thick = base * (0.04 + r2 * 0.02);
    ([face * 2.0, thick * 2.0, face * 2.0], visual.ui_tilt_x_deg)
}

pub(super) fn push_economy_icon_image(
    frame: &mut UiFrame,
    icon_rect: [f32; 4],
    path: &'static str,
    width_over_height: f32,
) {
    let [ix, iy, iw, ih] = icon_rect;
    let icon_cx = ix + iw * 0.5;
    let icon_cy = iy + ih * 0.5;
    let max_w = iw * 0.86;
    let max_h = ih * 0.88;
    let (quad_w, quad_h) = if width_over_height >= max_w / max_h {
        let w = max_w;
        (w, w / width_over_height)
    } else {
        let h = max_h;
        (h * width_over_height, h)
    };
    frame.image_quads(vec![ImageQuad {
        inst: GpuInstance {
            rect: [
                icon_cx - quad_w * 0.5,
                icon_cy - quad_h * 0.5,
                quad_w,
                quad_h,
            ],
            color: [1.0, 1.0, 1.0, 0.98],
            user: 0,
        },
        source: ImageQuadSource::Asset { path },
        clip_rect: Some(icon_rect),
    }]);
}

pub(super) fn economy_icon_object3d_pos(
    w: f32,
    h: f32,
    cam: &CameraParams,
    icon_cx: f32,
    icon_cy: f32,
) -> [f32; 3] {
    let world = world_on_camera_ray_plane_z(w, h, cam, icon_cx, icon_cy, 0.0);
    object3d_pos_triple_for_world_center(w, h, world)
}

/// Orients a mesh face toward the guide camera (oblique views need more than pitch-only).
pub(super) fn economy_icon_face_camera_rotation(
    w: f32,
    h: f32,
    cam: &CameraParams,
    icon_cx: f32,
    icon_cy: f32,
    local_normal: Vec3,
) -> [f32; 3] {
    let center = world_on_camera_ray_plane_z(w, h, cam, icon_cx, icon_cy, 0.0);
    let eye = Vec3::from_array(cam.eye);
    let mut toward_camera = eye - center;
    if toward_camera.length_squared() < 1e-8 {
        return camera_facing_euler_xyz_rad(cam.eye, cam.target);
    }
    toward_camera = toward_camera.normalize();
    let normal = local_normal.normalize();
    if toward_camera.dot(normal).abs() > 0.97 {
        return camera_facing_euler_xyz_rad(cam.eye, cam.target);
    }
    let q = Quat::from_rotation_arc(normal, toward_camera);
    mat4_to_euler_xyz_rad(Mat4::from_quat(q.normalize()))
}

pub(super) fn economy_card_body_font(
    available_h: f32,
    inner_w: f32,
    lines: &[&str],
    start_font: f32,
    min_font: f32,
    row_gap: f32,
) -> f32 {
    let mut font = start_font;
    loop {
        let mut needed = 0.0f32;
        for line in lines {
            let wrapped = styled_text::wrap_colored_text_multiline(
                line,
                inner_w,
                font / 0.99,
                color::PARCHMENT,
                true,
                GlossaryMode::Prose,
            );
            needed += styled_text::colored_wrapped_rows_height(&wrapped, font) + row_gap;
        }
        if needed <= available_h || font <= min_font {
            return font;
        }
        font *= 0.94;
    }
}

pub(super) fn push_economy_item_example(
    frame: &mut UiFrame,
    w: f32,
    h: f32,
    cam: &CameraParams,
    example: EconomyItemExample,
    icon_rect: [f32; 4],
    card_index: usize,
) {
    let [ix, iy, iw, ih] = icon_rect;
    let icon_cx = ix + iw * 0.5;
    let icon_cy = iy + ih * 0.5;
    let icon_span = iw.min(ih);
    let pos = economy_icon_object3d_pos(w, h, cam, icon_cx, icon_cy);
    let anim_id = card_index as u64;

    match example {
        EconomyItemExample::Relic(relic_id) => {
            let (extents, _) = economy_relic_extents(relic_id, icon_span);
            let rarity = all_relic_defs()
                .iter()
                .find(|d| d.id == relic_id)
                .map(|d| d.rarity)
                .unwrap_or(crate::core::relic::Rarity::Common);
            frame.object3d(Object3d {
                pos,
                extents,
                rotation: economy_icon_face_camera_rotation(w, h, cam, icon_cx, icon_cy, Vec3::Y),
                color: color::rarity(rarity.tier()),
                kind: Object3dKind::Relic {
                    relic_id,
                    glow: 0.12,
                    silhouette: false,
                    debuffed: false,
                },
                hover_target: 0.0,
                anim_id,
            });
        }
        EconomyItemExample::Zodiac(kind) => {
            push_economy_icon_image(frame, icon_rect, zodiac_icon_asset(kind), 1.0 / 3.0);
        }
        EconomyItemExample::Talisman(kind) => {
            let accent = kind.accent_color();
            let tablet = for_sale_talisman_tablet_extent(icon_span * 0.88);
            let face_base = crate::render::talisman_mesh::talisman_face_camera_rotation(14.0);
            let toward =
                economy_icon_face_camera_rotation(w, h, cam, icon_cx, icon_cy, Vec3::NEG_Y);
            frame.object3d(Object3d {
                pos,
                extents: crate::render::talisman_mesh::talisman_object_extents(tablet),
                rotation: compose_rotation_euler(
                    rot_euler_xyz_rad(face_base[0], face_base[1], face_base[2]),
                    [
                        toward[0].to_degrees(),
                        toward[1].to_degrees(),
                        toward[2].to_degrees(),
                    ],
                ),
                color: [
                    (accent[0] * 1.15).min(1.0),
                    (accent[1] * 1.15).min(1.0),
                    (accent[2] * 1.15).min(1.0),
                    1.0,
                ],
                kind: Object3dKind::Talisman { kind },
                hover_target: 0.0,
                anim_id,
            });
        }
        EconomyItemExample::TilePack(kind) => {
            let pack_h = ih * 0.68;
            let pack_w = pack_h * PACK_ASPECT_W_OVER_H;
            let pack_t = pack_h * 0.11;
            frame.object3d(Object3d {
                pos,
                extents: [pack_w, pack_t, pack_h],
                rotation: economy_icon_face_camera_rotation(
                    w,
                    h,
                    cam,
                    icon_cx,
                    icon_cy,
                    Vec3::NEG_Y,
                ),
                color: kind.pack_texture_tint(),
                kind: Object3dKind::Pack {
                    kind,
                    pick_id: None,
                },
                hover_target: 0.0,
                anim_id,
            });
        }
        EconomyItemExample::Memorial(kind) => {
            let accent = kind.accent_color();
            let tablet = for_sale_talisman_tablet_extent(icon_span * 0.88);
            let face_base = crate::render::talisman_mesh::talisman_face_camera_rotation(14.0);
            let toward =
                economy_icon_face_camera_rotation(w, h, cam, icon_cx, icon_cy, Vec3::NEG_Y);
            frame.object3d(Object3d {
                pos,
                extents: crate::render::talisman_mesh::talisman_object_extents(tablet),
                rotation: compose_rotation_euler(
                    rot_euler_xyz_rad(face_base[0], face_base[1], face_base[2]),
                    [
                        toward[0].to_degrees(),
                        toward[1].to_degrees(),
                        toward[2].to_degrees(),
                    ],
                ),
                color: [
                    (accent[0] * 1.15).min(1.0),
                    (accent[1] * 1.15).min(1.0),
                    (accent[2] * 1.15).min(1.0),
                    1.0,
                ],
                kind: Object3dKind::MemorialTalisman { kind },
                hover_target: 0.0,
                anim_id,
            });
        }
        EconomyItemExample::Temptation(tag) => {
            let max_side = icon_span * 0.72;
            frame.image_quads(vec![ImageQuad {
                inst: GpuInstance {
                    rect: [
                        icon_cx - max_side * 0.5,
                        icon_cy - max_side * 0.5,
                        max_side,
                        max_side,
                    ],
                    color: [1.0, 1.0, 1.0, 0.98],
                    user: 0,
                },
                source: temptation_icon_source(tag),
                clip_rect: Some(icon_rect),
            }]);
        }
    }
}

pub(super) fn economy_measure_text_width(text: &str, font_px: f32) -> f32 {
    let font_px = font_px.max(8.0);
    if let Some(font) = load_ui_font() {
        text.chars()
            .map(|ch| font.metrics(ch, font_px).advance_width)
            .sum()
    } else {
        font_px * 0.52 * text.chars().count().max(1) as f32
    }
}

pub(super) fn draw_economy_panel_header(
    frame: &mut UiFrame,
    x: f32,
    y: f32,
    w: f32,
    row_h: f32,
    text: &str,
    font: f32,
) {
    frame.text(TextLabel {
        rect: [x, y, w, row_h],
        text: text.into(),
        color: color::alpha(color::BRASS, 0.92),
        align: TextAlign::Left,
        font_px: Some(font),
        bold: true,
        ..Default::default()
    });
}

pub(super) fn draw_dot_leader_row(
    frame: &mut UiFrame,
    x: f32,
    y: f32,
    row_w: f32,
    row_h: f32,
    label: &str,
    value: &str,
    font: f32,
    value_col_w: f32,
    label_color: [f32; 4],
    value_color: [f32; 4],
) {
    let label_w = economy_measure_text_width(label, font).max(1.0);
    let gap = 4.0;
    let dot_char_w = economy_measure_text_width(".", font).max(1.0);
    let value_x = x + row_w - value_col_w;
    let dots_x = x + label_w + gap;
    let dots_w = (value_x - gap - dots_x).max(0.0);

    frame.text(TextLabel {
        rect: [x, y, label_w.min(row_w), row_h],
        text: label.into(),
        color: label_color,
        align: TextAlign::Left,
        font_px: Some(font),
        ..Default::default()
    });
    if dots_w >= dot_char_w {
        let dot_count = (dots_w / dot_char_w).floor() as usize;
        frame.text(TextLabel {
            rect: [dots_x, y, dots_w, row_h],
            text: ".".repeat(dot_count.max(1)),
            color: color::alpha(color::UMBER, 0.72),
            align: TextAlign::Right,
            font_px: Some(font),
            ..Default::default()
        });
    }
    frame.text(TextLabel {
        rect: [value_x, y, value_col_w, row_h],
        text: value.into(),
        color: value_color,
        align: TextAlign::Right,
        font_px: Some(font),
        mono: true,
        ..Default::default()
    });
}

pub(super) fn draw_earning_note_row(
    frame: &mut UiFrame,
    x: f32,
    y: f32,
    w: f32,
    row_h: f32,
    label: &str,
    line: &str,
    font: f32,
    label_color: [f32; 4],
    line_color: [f32; 4],
) -> f32 {
    frame.text(TextLabel {
        rect: [x, y, w, row_h],
        text: label.into(),
        color: label_color,
        align: TextAlign::Left,
        font_px: Some(font),
        ..Default::default()
    });
    let indent = font * 0.35;
    frame.text(TextLabel {
        rect: [x + indent, y + row_h * 0.92, w - indent, row_h],
        text: line.into(),
        color: line_color,
        align: TextAlign::Left,
        font_px: Some(font),
        ..Default::default()
    });
    row_h * 1.85
}
