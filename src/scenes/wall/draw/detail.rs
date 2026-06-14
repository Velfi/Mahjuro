//! Selected-tile detail panel in the left sidebar stack.

use crate::core::tile::Tile;
use crate::game::engine::GameEngine;
use crate::game::run::RunState;
use crate::game::wall_ledger::WallLedgerMode;
use crate::game::wall_stats::{ModifierBreakdown, SelectedTileDetails};
use crate::render::doc_tile_camera::TOP_DOWN_TILE_ROTATION;
use crate::render::draw_cmd::{ShowcaseTilePlacement, UiFrame};
use crate::render::theme::{color, typography};
use crate::render::vocabulary_colors::GlossaryMode;
use crate::render::wgpu_renderer::{TextAlign, TextBlockVerticalAlign, TextLabel};
use crate::ui::styled_text::{
    StyledBlockStyle, push_styled_text_block_at_font_px, styled_line_block_height_at_font_px,
};

use super::super::layout::{LEDGER_FOCUS_GLOW, LEDGER_FOCUS_OUTLINE, WallLayout, text_line_h};
use super::text::{push_clipped_quad, push_plaque_clipped, push_text_maybe_clip};
use super::tile_placement::{ledger_tile_brightness, showcase_tile_center_in_rect};

pub fn draw_wall_detail_panel(
    frame: &mut UiFrame,
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    placements: &mut Vec<ShowcaseTilePlacement>,
    layout: &WallLayout,
    rect: [f32; 4],
    details: &SelectedTileDetails,
    run: &RunState,
    representative: Option<&Tile>,
    window_w: f32,
    window_h: f32,
    scroll_clip: [f32; 4],
    mode: WallLedgerMode,
) {
    let clip = Some(scroll_clip);
    if rect_intersects(rect, scroll_clip) {
        push_clipped_quad(
            frame,
            [rect[0], rect[1], rect[2], 3.0],
            color::alpha(color::BRASS, 0.44),
            scroll_clip,
        );
        push_plaque_clipped(frame, rect, 0.88, scroll_clip);
    }

    let pad = layout.detail_pad();
    let inner_w = rect[2] - pad * 2.0;
    let caption_lh = text_line_h(layout.caption_px);
    let header_lh = caption_lh;
    let top_pad = (8.0 * layout.jr).max(6.0);

    let mut y = rect[1] + top_pad;
    push_text_maybe_clip(
        texts,
        [rect[0] + pad, y, inner_w, header_lh],
        "SELECTED TILE",
        layout.caption_px,
        color::alpha(color::CHAMPAGNE, 0.92),
        true,
        TextAlign::Left,
        clip,
    );
    y += header_lh + layout.section_inner_gap() + 4.0;

    let preview_size = layout.detail_preview_size(inner_w, scroll_clip[3]);
    let preview_rect = [
        rect[0] + (rect[2] - preview_size) * 0.5,
        y,
        preview_size,
        preview_size * 1.08,
    ];
    if rect_intersects(preview_rect, scroll_clip) {
        draw_tile_preview(
            frame,
            placements,
            preview_rect,
            details,
            run,
            representative,
            window_w,
            window_h,
            scroll_clip,
        );
    }
    y = preview_rect[1] + preview_rect[3] + layout.detail_pad();

    let exhausted = details.remaining == 0;

    let title_lh = text_line_h(layout.body_px);
    push_text_maybe_clip(
        texts,
        [rect[0] + pad, y, inner_w, title_lh],
        details.name.clone(),
        layout.body_px,
        color::alpha(color::JADE, if exhausted { 0.75 } else { 0.96 }),
        true,
        TextAlign::Center,
        clip,
    );
    y += title_lh + 8.0;

    push_text_maybe_clip(
        texts,
        [rect[0] + pad, y, inner_w, caption_lh],
        "COPIES",
        layout.caption_px,
        color::alpha(color::CHAMPAGNE, 0.92),
        true,
        TextAlign::Left,
        clip,
    );
    y += caption_lh + layout.section_inner_gap() + 2.0;

    draw_copies_panel(
        frame,
        texts,
        layout,
        rect,
        pad,
        inner_w,
        y,
        details,
        mode,
        scroll_clip,
        clip,
    );
    y += copies_panel_height(layout, mode);

    y += 4.0;

    if let Some(mod_line) = modifier_summary(&details.modifiers) {
        y += 2.0;
        push_text_maybe_clip(
            texts,
            [rect[0] + pad, y, inner_w, caption_lh],
            mod_line,
            layout.caption_px,
            color::alpha(color::STONE, 0.88),
            false,
            TextAlign::Center,
            clip,
        );
        y += caption_lh + 2.0;
    }

    y += 6.0;
    if rect_intersects([rect[0] + pad, y, inner_w, 1.0], scroll_clip) {
        push_divider(frame, rect[0] + pad, y, inner_w, scroll_clip);
    }
    y += 8.0;

    push_text_maybe_clip(
        texts,
        [rect[0] + pad, y, inner_w, caption_lh],
        "ABOUT",
        layout.caption_px,
        color::alpha(color::CHAMPAGNE, 0.92),
        true,
        TextAlign::Left,
        clip,
    );
    let body_top = y + caption_lh + layout.section_inner_gap();
    let body_font = layout.caption_px;
    let about_color = color::alpha(color::UMBER, if exhausted { 0.72 } else { 0.86 });
    let wrapped_h = styled_line_block_height_at_font_px(
        &details.about,
        inner_w,
        body_font,
        GlossaryMode::Prose,
        about_color,
    )
    .max(text_line_h(body_font));
    if rect_intersects([rect[0] + pad, body_top, inner_w, wrapped_h], scroll_clip) {
        push_styled_text_block_at_font_px(
            texts,
            [rect[0] + pad, body_top, inner_w, wrapped_h],
            &details.about,
            body_font,
            StyledBlockStyle {
                tier: typography::H42,
                color: about_color,
                padding: 0.0,
                align: TextAlign::Left,
                glossary: GlossaryMode::Prose,
                vertical_align: Some(TextBlockVerticalAlign::Top),
                clip_rect: clip,
            },
        );
    }
}

const COPIES_COL_GAP: f32 = 12.0;
const COPIES_COL_INSET: f32 = 8.0;
const COPIES_HEADER_DATA_GAP: f32 = 4.0;

fn copies_block_pad_v(layout: &WallLayout) -> f32 {
    (7.0 * layout.jr).max(6.0)
}

/// Width for numeric values (`100.0%`, modifier counts). Sized for [`WallLayout::count_px`].
fn copies_value_col_w(col_w: f32, count_px: f32) -> f32 {
    let need = count_px * 4.6 * 0.58;
    need.clamp(52.0, col_w * 0.44)
}

fn copies_row_count(mode: WallLedgerMode) -> usize {
    let copy_rows = if mode.shows_round_locations() { 4 } else { 1 };
    copy_rows.max(4)
}

fn copies_row_h(layout: &WallLayout) -> f32 {
    text_line_h(layout.caption_px)
        .max(text_line_h(layout.count_px))
        + 3.0
}

fn copies_micro_header_h(layout: &WallLayout) -> f32 {
    text_line_h(layout.caption_px)
}

pub(crate) fn copies_panel_height(layout: &WallLayout, mode: WallLedgerMode) -> f32 {
    let pad_v = copies_block_pad_v(layout);
    pad_v * 2.0
        + copies_micro_header_h(layout)
        + COPIES_HEADER_DATA_GAP
        + copies_row_h(layout) * copies_row_count(mode) as f32
}

fn draw_copies_panel(
    frame: &mut UiFrame,
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    layout: &WallLayout,
    rect: [f32; 4],
    pad: f32,
    inner_w: f32,
    y: f32,
    details: &SelectedTileDetails,
    mode: WallLedgerMode,
    scroll_clip: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    let col_w = ((inner_w - COPIES_COL_GAP) * 0.5).max(1.0);
    let block_x = rect[0] + pad;
    let left_x = block_x;
    let right_x = left_x + col_w + COPIES_COL_GAP;
    let divider_x = left_x + col_w + COPIES_COL_GAP * 0.5;
    let row_h = copies_row_h(layout);
    let micro_h = copies_micro_header_h(layout);
    let block_h = copies_panel_height(layout, mode);
    let block_rect = [block_x, y, inner_w, block_h];
    let block_pad_v = copies_block_pad_v(layout);

    push_clipped_quad(
        frame,
        block_rect,
        color::alpha(color::WALNUT_INK, 0.42),
        scroll_clip,
    );
    push_border(
        frame,
        block_rect,
        1.0,
        color::alpha(color::BRASS, 0.14),
        scroll_clip,
    );
    let data_top = y + block_pad_v + micro_h + COPIES_HEADER_DATA_GAP;
    let data_h = block_h - block_pad_v * 2.0 - micro_h - COPIES_HEADER_DATA_GAP;
    push_clipped_quad(
        frame,
        [divider_x - 0.5, data_top, 1.0, data_h],
        color::alpha(color::BRASS, 0.28),
        scroll_clip,
    );

    let micro_y = y + block_pad_v;
    let micro_color = color::alpha(color::BRASS, 0.62);
    draw_stat_row(
        texts,
        layout,
        left_x,
        col_w,
        micro_y,
        "WHERE",
        "",
        micro_color,
        micro_color,
        true,
        clip,
        true,
    );
    draw_stat_row(
        texts,
        layout,
        right_x,
        col_w,
        micro_y,
        "SUPPLY",
        "",
        micro_color,
        micro_color,
        true,
        clip,
        true,
    );

    let copy_rows: &[(&str, usize)] = if mode.shows_round_locations() {
        &[
            ("In wall", details.locations.in_wall),
            ("In hand", details.locations.in_hand),
            ("Played", details.locations.played),
            ("Discarded", details.locations.discarded),
        ]
    } else {
        &[("In wall", details.locations.in_wall)]
    };
    let modifier_rows: [(&str, usize, [f32; 4]); 3] = [
        ("Pearl", details.modifiers.pearl, color::JADE),
        ("Gilded", details.modifiers.gilded, color::GOLD),
        ("Poly", details.modifiers.polychrome, color::keyword::FLOWER),
    ];
    let row_count = copies_row_count(mode);

    for row_idx in 0..row_count {
        let row_y = data_top + row_idx as f32 * row_h;
        if row_idx < copy_rows.len() {
            let (label, count) = copy_rows[row_idx];
            let (value_color, emphasis) = count_value_style(count);
            draw_stat_row(
                texts,
                layout,
                left_x,
                col_w,
                row_y,
                label,
                &format!("{count}"),
                color::STONE,
                value_color,
                emphasis,
                clip,
                false,
            );
        }
        if row_idx == 0 {
            let draw = details.draw_probability;
            let draw_pct = format!("{:.1}%", draw * 100.0);
            let (value_color, emphasis) = if draw > 0.0005 {
                (color::CHAMPAGNE, true)
            } else {
                (color::alpha(color::STONE, 0.72), false)
            };
            draw_stat_row(
                texts,
                layout,
                right_x,
                col_w,
                row_y,
                "Draw %",
                &draw_pct,
                color::STONE,
                value_color,
                emphasis,
                clip,
                false,
            );
        } else if row_idx <= modifier_rows.len() {
            let (label, count, accent) = modifier_rows[row_idx - 1];
            let label_color = if count > 0 {
                color::alpha(accent, 0.94)
            } else {
                color::alpha(color::STONE, 0.72)
            };
            let (value_color, emphasis) = if count > 0 {
                (color::alpha(accent, 0.96), true)
            } else {
                (color::alpha(color::UMBER, 0.55), false)
            };
            draw_stat_row(
                texts,
                layout,
                right_x,
                col_w,
                row_y,
                label,
                &format!("{count}"),
                label_color,
                value_color,
                emphasis,
                clip,
                false,
            );
        }
    }
}

fn count_value_style(count: usize) -> ([f32; 4], bool) {
    if count > 0 {
        (color::CHAMPAGNE, true)
    } else {
        (color::alpha(color::UMBER, 0.58), false)
    }
}

fn draw_stat_row(
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    layout: &WallLayout,
    x: f32,
    col_w: f32,
    y: f32,
    label: &str,
    value: &str,
    label_color: [f32; 4],
    value_color: [f32; 4],
    value_emphasis: bool,
    clip: Option<[f32; 4]>,
    header_only: bool,
) {
    let label_line = text_line_h(layout.caption_px);
    let value_line = text_line_h(layout.count_px);
    let row_line = if header_only {
        label_line
    } else {
        label_line.max(value_line)
    };
    let value_w = copies_value_col_w(col_w, layout.count_px);
    let label_x = x + COPIES_COL_INSET;
    let value_x = x + col_w - COPIES_COL_INSET - value_w;
    let label_w = (value_x - label_x - 6.0).max(1.0);
    push_stat_text(
        texts,
        [label_x, y, label_w, row_line],
        label,
        layout.caption_px,
        label_color,
        header_only,
        TextAlign::Left,
        clip,
    );
    if !header_only {
        push_stat_text(
            texts,
            [value_x, y, value_w, row_line],
            value,
            layout.count_px,
            value_color,
            value_emphasis,
            TextAlign::Right,
            clip,
        );
    }
}

fn push_stat_text(
    texts: &mut Vec<TextLabel>,
    rect: [f32; 4],
    text: &str,
    font_px: f32,
    color: [f32; 4],
    bold: bool,
    align: TextAlign,
    clip: Option<[f32; 4]>,
) {
    texts.push(TextLabel {
        rect,
        text: text.to_string(),
        color,
        font_px: Some(font_px),
        align,
        block_vertical_align: TextBlockVerticalAlign::Top,
        scroll_offset: 0.0,
        flavor_spans: None,
        bold,
        italic: false,
        underline: false,
        text_effect: crate::render::text_effect::TextEffectId::Flat,
        rotation_quarters: 0,
        baseline_shift_px: 0.0,
        clip_rect: clip,
        mono: false,
    });
}

fn rect_intersects(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] + a[2] > b[0] && a[0] < b[0] + b[2] && a[1] + a[3] > b[1] && a[1] < b[1] + b[3]
}

fn draw_tile_preview(
    frame: &mut UiFrame,
    placements: &mut Vec<ShowcaseTilePlacement>,
    preview_rect: [f32; 4],
    details: &SelectedTileDetails,
    run: &RunState,
    representative: Option<&Tile>,
    window_w: f32,
    window_h: f32,
    scroll_clip: [f32; 4],
) {
    push_clipped_quad(
        frame,
        [
            preview_rect[0] - 3.0,
            preview_rect[1] - 3.0,
            preview_rect[2] + 6.0,
            preview_rect[3] + 6.0,
        ],
        LEDGER_FOCUS_GLOW,
        scroll_clip,
    );
    push_clipped_quad(
        frame,
        preview_rect,
        color::alpha(color::WALNUT_INK, 0.55),
        scroll_clip,
    );
    push_border(
        frame,
        preview_rect,
        1.0,
        color::alpha(color::STONE, 0.16),
        scroll_clip,
    );

    let exhausted = details.remaining == 0;
    if let Some(tile) = representative {
        let tile_size = preview_rect[2].min(preview_rect[3]) * 0.84;
        placements.push(ShowcaseTilePlacement {
            tile: GameEngine::display_tile(*tile, run),
            center_pos: showcase_tile_center_in_rect(preview_rect, tile_size, window_w, window_h),
            rotation: TOP_DOWN_TILE_ROTATION,
            scale: 1.0,
            size_px: tile_size,
            brightness: ledger_tile_brightness(exhausted, true),
            opacity: 1.0,
            selected: true,
            hovered: false,
            outline: false,
            glow: false,
            glow_color: None,
            outline_sel: None,
            pick_id: None,
            overlay_rect_group: None,
        });
    }
    push_border(frame, preview_rect, 1.5, LEDGER_FOCUS_OUTLINE, scroll_clip);
    push_focus_corner(
        frame,
        preview_rect[0] + preview_rect[2] - 6.0,
        preview_rect[1] + 4.0,
        scroll_clip,
    );
}

fn push_divider(frame: &mut UiFrame, x: f32, y: f32, w: f32, clip: [f32; 4]) {
    push_clipped_quad(
        frame,
        [x, y, w, 1.0],
        color::alpha(color::STONE, 0.14),
        clip,
    );
}

fn push_focus_corner(frame: &mut UiFrame, x: f32, y: f32, clip: [f32; 4]) {
    push_clipped_quad(
        frame,
        [x, y, 4.0, 4.0],
        color::alpha(LEDGER_FOCUS_OUTLINE, 0.85),
        clip,
    );
}

fn push_border(frame: &mut UiFrame, rect: [f32; 4], t: f32, c: [f32; 4], clip: [f32; 4]) {
    push_clipped_quad(frame, [rect[0], rect[1], rect[2], t], c, clip);
    push_clipped_quad(
        frame,
        [rect[0], rect[1] + rect[3] - t, rect[2], t],
        c,
        clip,
    );
    push_clipped_quad(frame, [rect[0], rect[1], t, rect[3]], c, clip);
    push_clipped_quad(
        frame,
        [rect[0] + rect[2] - t, rect[1], t, rect[3]],
        c,
        clip,
    );
}

fn modifier_summary(m: &ModifierBreakdown) -> Option<String> {
    if m.debuffed > 0 {
        Some(format!("Debuff ×{}", m.debuffed))
    } else {
        None
    }
}
