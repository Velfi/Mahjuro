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
use super::economy::{
    ECONOMY_ICON_COL_FRAC, ECONOMY_ITEM_COLS, ECONOMY_ITEM_EXAMPLES, ECONOMY_ITEM_ROWS,
    EconomyItemExample, draw_dot_leader_row, draw_earning_note_row, draw_economy_panel_header,
    economy_card_body_font, economy_measure_text_width, push_economy_item_example,
};
use super::economy_flow::draw_skip_steps_column;
use super::layout::push_guide_panel_stroke;

pub(super) fn draw_economy_rules_band(
    frame: &mut UiFrame,
    content: [f32; 4],
    caption_font: f32,
    micro_font: f32,
    pad: f32,
) {
    let [cx, cy, cw, ch] = content;
    let inner_pad = pad * 0.6;
    let mut body_font = caption_font;
    let mut row_h = body_font * 1.12;
    let header_h = micro_font * 1.02;
    let body_color = color::alpha(color::PARCHMENT, 0.90);
    let yen_color = color::keyword::GOLD;
    let top_y = cy + inner_pad * 0.35;
    let bottom_y = cy + ch - inner_pad;
    let col_gap = pad * 0.48;
    let col_w = ((cw - inner_pad * 2.0 - col_gap * 2.0) / 3.0).max(1.0);

    let earning_x = cx + inner_pad;
    let store_x = earning_x + col_w + col_gap;
    let skip_x = store_x + col_w + col_gap;
    let body_h = bottom_y - top_y;
    let earn_body_h = body_h - header_h - pad * 0.14;
    let earn_units = economy_intro_copy::EARNING_CLEAR_ROWS.len() as f32
        + economy_intro_copy::EARNING_NOTE_ROWS.len() as f32 * 1.85;
    let earn_needed = earn_units * row_h + pad * 0.06;
    if earn_body_h > earn_needed {
        row_h *= (earn_body_h / earn_needed).min(1.28);
        body_font *= (earn_body_h / earn_needed).sqrt().min(1.12);
    }

    for div_x in [store_x - col_gap * 0.5, skip_x - col_gap * 0.5] {
        frame.quad(GpuInstance {
            rect: [div_x - 0.5, top_y, 1.0, body_h],
            color: color::alpha(color::UMBER, 0.36),
            user: 0,
        });
    }

    // Panel 1 — Earning Yen
    let earn_pad = pad * 0.35;
    let earn_x = earning_x + earn_pad;
    let earn_w = (col_w - earn_pad * 2.0).max(1.0);
    draw_economy_panel_header(
        frame,
        earn_x,
        top_y,
        earn_w,
        header_h,
        economy_intro_copy::SECTION_EARNING,
        micro_font,
    );
    let mut ey = top_y + header_h + pad * 0.14;
    let earning_value_col_w = economy_intro_copy::EARNING_CLEAR_ROWS
        .iter()
        .map(|(_, value)| economy_measure_text_width(value, body_font))
        .fold(0.0f32, f32::max)
        .max(1.0);
    for (label, value) in economy_intro_copy::EARNING_CLEAR_ROWS {
        if ey + row_h > bottom_y {
            break;
        }
        draw_dot_leader_row(
            frame,
            earn_x,
            ey,
            earn_w,
            row_h,
            label,
            value,
            body_font,
            earning_value_col_w,
            body_color,
            yen_color,
        );
        ey += row_h;
    }
    ey += pad * 0.06;
    for note in economy_intro_copy::EARNING_NOTE_ROWS {
        if ey + row_h > bottom_y {
            break;
        }
        let line_color = if note.label == "Interest" {
            yen_color
        } else {
            body_color
        };
        ey += draw_earning_note_row(
            frame, earn_x, ey, earn_w, row_h, note.label, note.line, body_font, body_color,
            line_color,
        );
    }

    // Panel 2 — The Storeroom
    let store_pad = pad * 0.35;
    let store_inner_x = store_x + store_pad;
    let store_inner_w = (col_w - store_pad * 2.0).max(1.0);
    draw_economy_panel_header(
        frame,
        store_inner_x,
        top_y,
        store_inner_w,
        header_h,
        economy_intro_copy::SECTION_STOREROOM,
        micro_font,
    );
    let footer_h = row_h * 1.05;
    let store_body_top = top_y + header_h + pad * 0.14;
    let store_body_h = (bottom_y - footer_h - pad * 0.08 - store_body_top).max(row_h);
    let store_line_count = economy_intro_copy::STOREROOM_LINES.len() as f32;
    let store_row_h = if store_body_h > store_line_count * row_h {
        store_body_h / store_line_count
    } else {
        row_h
    };
    let mut sty = store_body_top;
    for line in economy_intro_copy::STOREROOM_LINES {
        if sty + store_row_h > bottom_y - footer_h {
            break;
        }
        frame.text(TextLabel {
            rect: [store_inner_x, sty, store_inner_w, store_row_h],
            text: (*line).into(),
            color: body_color,
            align: TextAlign::Left,
            font_px: Some(body_font),
            ..Default::default()
        });
        sty += store_row_h;
    }
    let footer_y = bottom_y - footer_h;
    if footer_y > sty + pad * 0.08 {
        frame.quad(GpuInstance {
            rect: [store_inner_x, footer_y - pad * 0.08, store_inner_w, 1.0],
            color: color::alpha(color::UMBER, 0.34),
            user: 0,
        });
    }
    frame.text(TextLabel {
        rect: [store_inner_x, footer_y, store_inner_w, footer_h],
        text: economy_intro_copy::STOREROOM_CAPACITY_FOOTER.into(),
        color: color::alpha(color::PARCHMENT, 0.86),
        align: TextAlign::Left,
        font_px: Some(micro_font),
        ..Default::default()
    });

    // Panel 3 — Skipping
    let skip_pad = pad * 0.35;
    let skip_inner_x = skip_x + skip_pad;
    let skip_inner_w = (col_w - skip_pad * 2.0).max(1.0);
    draw_economy_panel_header(
        frame,
        skip_inner_x,
        top_y,
        skip_inner_w,
        header_h,
        economy_intro_copy::SECTION_SKIPPING,
        micro_font,
    );
    let skip_body_top = top_y + header_h + pad * 0.14;
    let skip_body_h = bottom_y - skip_body_top;
    draw_skip_steps_column(
        frame,
        skip_inner_x,
        skip_body_top,
        skip_inner_w,
        skip_body_h,
        body_font,
        pad,
        body_color,
    );
}

pub(super) fn economy_item_title_color(card_index: usize) -> [f32; 4] {
    match card_index {
        0 => color::BRASS,
        1 => color::keyword::FLOWER,
        2 => color::keyword::SEASON,
        3 => color::keyword::TRIGGER,
        4 => color::JADE,
        _ => color::AMBER,
    }
}

pub(super) fn economy_item_role_label(card_index: usize) -> &'static str {
    match card_index {
        0 => "PAST PLAYER'S POWER",
        1 => "YAKUS REWARD MORE",
        2 => "REMAKE YOUR TILES",
        3 => "BUILD THE WALL",
        4 => "YOUR SAD REMAINS",
        _ => "MAKE A CHOICE",
    }
}

pub(super) fn push_economy_item_cards(
    frame: &mut UiFrame,
    layout: &GuideLayout,
    w: f32,
    h: f32,
    cam: &CameraParams,
    outer: [f32; 4],
    small_font: f32,
    title_font: f32,
    pad: f32,
    gap: f32,
) {
    let [_x, y, full_w, full_h] = outer;
    let cell_w = (full_w - gap * (ECONOMY_ITEM_COLS - 1) as f32) / ECONOMY_ITEM_COLS as f32;
    let cell_h = (full_h - gap * (ECONOMY_ITEM_ROWS - 1) as f32) / ECONOMY_ITEM_ROWS as f32;
    let stroke = color::alpha(color::BRASS, 0.32);
    let fill = color::alpha(color::WALNUT_RAISED, 0.28);
    let body_color = color::alpha(color::PARCHMENT, 0.90);
    let role_font = typography::size(typography::H45, h);
    let content_x = layout.content_x;
    let icon_col_w = cell_w * ECONOMY_ICON_COL_FRAC;
    let text_pad = pad * 0.75;
    let text_x_offset = icon_col_w + text_pad;

    for (i, card) in economy_intro_copy::ITEMS.iter().enumerate() {
        let col = i % ECONOMY_ITEM_COLS;
        let row = i / ECONOMY_ITEM_COLS;
        let cx = content_x + col as f32 * (cell_w + gap);
        let cy = y + row as f32 * (cell_h + gap);
        let rect = [cx, cy, cell_w, cell_h];

        frame.quad(GpuInstance {
            rect,
            color: fill,
            user: 0,
        });
        push_guide_panel_stroke(frame, rect, stroke);
        push_guide_panel_stroke(
            frame,
            [rect[0] + 2.0, rect[1] + 2.0, rect[2] - 4.0, rect[3] - 4.0],
            color::alpha(color::BRASS, 0.14),
        );

        let icon_rect = [
            cx + pad,
            cy + pad,
            (icon_col_w - pad * 1.25).max(1.0),
            (cell_h - pad * 2.0).max(1.0),
        ];
        push_economy_item_example(frame, w, h, cam, ECONOMY_ITEM_EXAMPLES[i], icon_rect, i);

        let text_x = cx + text_x_offset;
        let inner_w = (cell_w - text_x_offset - pad).max(1.0);
        let title_color = economy_item_title_color(i);
        let title_h = title_font * 1.08;
        let role_h = role_font * 1.05;
        let text_clip =
            intersect_rect([text_x, cy + pad, inner_w, cell_h - pad * 2.0], rect).unwrap_or(rect);
        let mut title_label = TextLabel {
            rect: [text_clip[0], text_clip[1], text_clip[2], title_h],
            text: card.title.to_uppercase(),
            color: title_color,
            align: TextAlign::Left,
            font_px: Some(title_font),
            bold: true,
            ..Default::default()
        };
        title_label.clip_rect = Some(text_clip);
        frame.text(title_label);
        let mut role_label = TextLabel {
            rect: [text_clip[0], text_clip[1] + title_h, text_clip[2], role_h],
            text: economy_item_role_label(i).into(),
            color: color::alpha(color::STONE, 0.78),
            align: TextAlign::Left,
            font_px: Some(role_font),
            bold: true,
            ..Default::default()
        };
        role_label.clip_rect = Some(text_clip);
        frame.text(role_label);

        let body_top = cy + pad + title_h + role_h + pad * 0.12;
        let body_available = (cy + cell_h - pad - body_top).max(1.0);
        let row_gap = pad * 0.14;
        let min_font = typography::size(typography::H45, h);
        let body_font = economy_card_body_font(
            body_available,
            inner_w,
            card.lines,
            small_font,
            min_font,
            row_gap,
        );
        let mut line_y = body_top;
        let bottom = cy + cell_h - pad;
        for line in card.lines {
            if line_y >= bottom {
                break;
            }
            let wrapped = styled_text::wrap_colored_text_multiline(
                line,
                inner_w,
                body_font / 0.99,
                body_color,
                true,
                GlossaryMode::Prose,
            );
            let block_h = styled_text::colored_wrapped_rows_height(&wrapped, body_font);
            if line_y + block_h > bottom {
                break;
            }
            let mut labels = Vec::new();
            styled_text::push_colored_rows_left(
                &mut labels,
                styled_text::ColoredRowsLayout {
                    text_left: text_x,
                    top_y: line_y,
                    inner_w,
                    line_h: body_font,
                    fallback_plain: line,
                    fallback_color: body_color,
                    italic: false,
                    glossary: GlossaryMode::Prose,
                },
                &wrapped,
            );
            for label in &mut labels {
                label.clip_rect = Some(text_clip);
            }
            frame.texts(labels);
            line_y += block_h + row_gap;
        }
    }
}
