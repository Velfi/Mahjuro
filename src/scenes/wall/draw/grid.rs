//! Tile ledger grid — row bands, recessed slips, quiet exhausted state.

use std::collections::HashMap;

use crate::core::tile::{Suit, Tile};
use crate::game::engine::GameEngine;
use crate::game::run::RunState;
use crate::game::wall_ledger::{WallLedgerFaceGroup, WallTileEntry};
use crate::game::wall_stats::{
    AbundanceState, WallCountView, WallStats, abundance_color, abundance_state_for_display,
};
use crate::render::doc_tile_camera::TOP_DOWN_TILE_ROTATION;
use crate::render::draw_cmd::ShowcaseTilePlacement;
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextAlign};

use super::super::focus::LedgerNav;
use super::super::layout::{
    LEDGER_FOCUS_GLOW, LEDGER_FOCUS_OUTLINE, ROW_LABELS, WallLayout, grid_cell_rect, grid_row_rect,
    row_label_font_px, row_suit_color, text_line_h,
};
use super::super::state::WallScreenState;
use super::text::{push_text, push_text_maybe_clip};

const EXHAUSTED_BRIGHTNESS: f32 = 0.22;
const EXHAUSTED_SCALE: f32 = 0.84;
const COUNTER_BAND_FRAC: f32 = 0.23;
const TILE_SIZE_MUL: f32 = 1.06;

pub fn draw_tile_ledger_grid(
    frame: &mut crate::render::draw_cmd::UiFrame,
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    placements: &mut Vec<ShowcaseTilePlacement>,
    layout: &WallLayout,
    screen: &WallScreenState,
    stats: &WallStats,
    groups: &HashMap<(Suit, u8), &WallLedgerFaceGroup>,
    run: &RunState,
    focus: Option<LedgerNav>,
    h: f32,
) {
    draw_row_bands(frame, layout);
    draw_row_labels(frame, texts, layout, h);

    for (idx, entry) in stats.entries.iter().enumerate() {
        if !screen.face_visible(entry) {
            continue;
        }
        let Some(cell) = grid_cell_rect(layout, idx) else {
            continue;
        };
        let exhausted = entry.display_count == 0;
        let focused = focus == Some(LedgerNav::Tile(idx))
            || (screen.selected.suit == entry.suit && screen.selected.rank == entry.rank);

        let (slip, tile_area, count_rect, was_rect) = cell_areas(cell, layout);
        push_tile_slip(frame, slip, exhausted, focused);

        if let Some(group) = groups.get(&(entry.suit, entry.rank)) {
            if let Some(rep) = representative_entry(&group.copies) {
                push_cell_tile(placements, rep, exhausted, tile_area, run);
            }
        } else {
            push_cell_tile_from_face(
                placements, entry.suit, entry.rank, exhausted, tile_area, run,
            );
        }

        let abundance = abundance_state_for_display(entry.suit, entry.display_count, screen.view);
        let count_color = count_color_for_cell(exhausted, focused, abundance);
        push_text(
            texts,
            count_rect,
            format!("×{}", entry.display_count),
            layout.count_px * 1.06,
            count_color,
            !exhausted || focused || abundance == AbundanceState::Abundant,
            TextAlign::Center,
        );
        if entry.total > 0 && screen.view == WallCountView::Remaining {
            push_text(
                texts,
                was_rect,
                format!("was {}", entry.total),
                layout.caption_px * 0.60,
                color::alpha(color::UMBER, if exhausted { 0.28 } else { 0.42 }),
                false,
                TextAlign::Center,
            );
        }

        let mod_mark = entry.modifiers.pearl
            + entry.modifiers.gilded
            + entry.modifiers.polychrome
            + entry.modifiers.debuffed;
        if mod_mark > 0 {
            push_text(
                texts,
                [slip[0] + slip[2] - 11.0, slip[1] + 2.0, 10.0, 10.0],
                "◆",
                layout.caption_px * 0.7,
                color::alpha(color::GOLD, 0.85),
                false,
                TextAlign::Center,
            );
        }
    }
}

fn draw_row_bands(frame: &mut crate::render::draw_cmd::UiFrame, layout: &WallLayout) {
    for row_idx in 0..ROW_LABELS.len() {
        let band = grid_row_rect(layout, row_idx);
        let tint = if row_idx % 2 == 0 {
            color::alpha(color::WALNUT_DEEP, 0.34)
        } else {
            color::alpha(color::WALNUT_INK, 0.26)
        };
        frame.quad(GpuInstance {
            rect: band,
            color: tint,
            user: 0,
        });
        frame.quad(GpuInstance {
            rect: [band[0], band[1], band[2], 1.0],
            color: color::alpha(color::STONE, 0.16),
            user: 0,
        });
        frame.quad(GpuInstance {
            rect: [band[0], band[1] + band[3] - 1.0, band[2], 1.0],
            color: color::alpha(color::STONE, 0.10),
            user: 0,
        });
    }
}

fn draw_row_labels(
    frame: &mut crate::render::draw_cmd::UiFrame,
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    layout: &WallLayout,
    h: f32,
) {
    let dot = 6.0;
    let text_x = layout.grid_content_x + dot + 4.0;
    let text_w = layout.label_col_w - dot - 6.0;

    for (row_idx, label) in ROW_LABELS.iter().enumerate() {
        let row_y = layout.row_y[row_idx];
        let cell_h = layout.row_cell_h[row_idx];
        let label_px = row_label_font_px(text_w, cell_h, h);
        let line_h = text_line_h(label_px);
        let label_y = row_y + (cell_h - line_h) * 0.5;
        let dot_y_offset = (line_h - dot) * 0.5;
        let label_clip = [layout.grid_content_x, row_y, layout.label_col_w, cell_h];
        frame.quad(GpuInstance {
            rect: [
                layout.grid_content_x + 1.0,
                label_y + dot_y_offset,
                dot,
                dot,
            ],
            color: color::alpha(row_suit_color(row_idx), 0.70),
            user: 0,
        });
        push_text_maybe_clip(
            texts,
            [text_x, label_y, text_w, line_h],
            *label,
            label_px,
            row_suit_color(row_idx),
            true,
            TextAlign::Left,
            Some(label_clip),
        );
    }
}

fn cell_areas(cell: [f32; 4], layout: &WallLayout) -> ([f32; 4], [f32; 4], [f32; 4], [f32; 4]) {
    let counter_h = (cell[3] * COUNTER_BAND_FRAC)
        .max(text_line_h(layout.count_px) + text_line_h(layout.caption_px) + 1.0)
        .min(cell[3] * 0.38);
    let tile_h = (cell[3] - counter_h).max(1.0);
    let slip_inset = 1.0;
    let slip = [
        cell[0] + slip_inset,
        cell[1] + 0.5,
        cell[2] - slip_inset * 2.0,
        cell[3] - 1.0,
    ];
    let tile_area = [slip[0], slip[1], slip[2], tile_h - 0.5];
    let counter_top = cell[1] + tile_h;
    let count_rect = [cell[0], counter_top, cell[2], counter_h * 0.55];
    let was_rect = [
        cell[0],
        counter_top + counter_h * 0.50,
        cell[2],
        counter_h * 0.45,
    ];
    (slip, tile_area, count_rect, was_rect)
}

fn push_tile_slip(
    frame: &mut crate::render::draw_cmd::UiFrame,
    slip: [f32; 4],
    exhausted: bool,
    focused: bool,
) {
    if focused {
        let pad = 2.0;
        frame.quad(GpuInstance {
            rect: [
                slip[0] - pad,
                slip[1] - pad,
                slip[2] + pad * 2.0,
                slip[3] + pad * 2.0,
            ],
            color: LEDGER_FOCUS_GLOW,
            user: 0,
        });
        push_border(frame, slip, 1.5, LEDGER_FOCUS_OUTLINE);
        push_focus_corner(frame, slip[0] + slip[2] - 5.0, slip[1] + 2.0);
        frame.quad(GpuInstance {
            rect: slip,
            color: color::alpha(color::WALNUT_BRIGHT, if exhausted { 0.12 } else { 0.18 }),
            user: 0,
        });
        if exhausted {
            push_corner_stamp(frame, slip, 0.20);
        }
    } else if !exhausted {
        frame.quad(GpuInstance {
            rect: slip,
            color: color::alpha(color::WALNUT_BRIGHT, 0.06),
            user: 0,
        });
    }
}

fn push_focus_corner(frame: &mut crate::render::draw_cmd::UiFrame, x: f32, y: f32) {
    frame.quad(GpuInstance {
        rect: [x, y, 4.0, 4.0],
        color: color::alpha(LEDGER_FOCUS_OUTLINE, 0.85),
        user: 0,
    });
}

fn push_corner_stamp(frame: &mut crate::render::draw_cmd::UiFrame, slip: [f32; 4], alpha: f32) {
    let s = slip[2].min(slip[3]) * 0.16;
    let x0 = slip[0] + slip[2] - s - 1.0;
    let y0 = slip[1] + 1.0;
    frame.quad(GpuInstance {
        rect: [x0, y0 + s * 0.55, s, 1.0],
        color: color::alpha(color::RUBY, alpha),
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [x0 + s * 0.55, y0, 1.0, s * 0.55],
        color: color::alpha(color::RUBY, alpha),
        user: 0,
    });
}

fn count_color_for_cell(exhausted: bool, focused: bool, abundance: AbundanceState) -> [f32; 4] {
    if exhausted {
        color::alpha(color::RUBY, if focused { 0.72 } else { 0.46 })
    } else if abundance == AbundanceState::Abundant {
        color::alpha(color::JADE, 0.98)
    } else {
        abundance_color(abundance)
    }
}

fn push_border(frame: &mut crate::render::draw_cmd::UiFrame, rect: [f32; 4], t: f32, c: [f32; 4]) {
    frame.quad(GpuInstance {
        rect: [rect[0], rect[1], rect[2], t],
        color: c,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [rect[0], rect[1] + rect[3] - t, rect[2], t],
        color: c,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [rect[0], rect[1], t, rect[3]],
        color: c,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: [rect[0] + rect[2] - t, rect[1], t, rect[3]],
        color: c,
        user: 0,
    });
}

fn representative_entry<'a>(entries: &'a [WallTileEntry]) -> Option<&'a WallTileEntry> {
    entries
        .iter()
        .find(|e| !e.drawn)
        .or_else(|| entries.first())
}

fn push_cell_tile(
    placements: &mut Vec<ShowcaseTilePlacement>,
    entry: &WallTileEntry,
    exhausted: bool,
    tile_area: [f32; 4],
    run: &RunState,
) {
    push_cell_tile_inner(
        placements,
        GameEngine::display_tile(entry.tile, run),
        exhausted,
        tile_area,
    );
}

fn push_cell_tile_from_face(
    placements: &mut Vec<ShowcaseTilePlacement>,
    suit: Suit,
    rank: u8,
    exhausted: bool,
    tile_area: [f32; 4],
    run: &RunState,
) {
    let tile = Tile::new(suit, rank, 0);
    push_cell_tile_inner(
        placements,
        GameEngine::display_tile(tile, run),
        exhausted,
        tile_area,
    );
}

fn push_cell_tile_inner(
    placements: &mut Vec<ShowcaseTilePlacement>,
    tile: Tile,
    exhausted: bool,
    tile_area: [f32; 4],
) {
    let tile_size = (tile_area[2] * 0.92 * TILE_SIZE_MUL).min(tile_area[3] * 0.94 * TILE_SIZE_MUL);
    placements.push(ShowcaseTilePlacement {
        tile,
        center_pos: [
            tile_area[0] + tile_area[2] * 0.5,
            tile_area[1] + tile_area[3] * 0.5,
            0.0,
        ],
        rotation: TOP_DOWN_TILE_ROTATION,
        scale: if exhausted { EXHAUSTED_SCALE } else { 1.0 },
        size_px: tile_size,
        brightness: if exhausted { EXHAUSTED_BRIGHTNESS } else { 1.0 },
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
}

pub fn draw_grid_panel_chrome(frame: &mut crate::render::draw_cmd::UiFrame, layout: &WallLayout) {
    let rect = [layout.grid_x, layout.grid_y, layout.grid_w, layout.grid_h];
    super::text::push_plaque(frame, rect, 0.82);
    push_border(frame, rect, 0.5, color::alpha(color::BRASS, 0.10));
}
