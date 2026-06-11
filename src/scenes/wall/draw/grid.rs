//! Tile ledger grid — row bands and tile slips without per-cell counters.

use std::collections::HashMap;

use crate::core::tile::{Suit, Tile};
use crate::game::engine::GameEngine;
use crate::game::run::RunState;
use crate::game::wall_ledger::{WallLedgerFaceGroup, WallTileEntry};
use crate::game::wall_stats::WallStats;
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
use super::text::push_text_maybe_clip;
use super::tile_placement::{
    ledger_grid_tile_size, ledger_tile_brightness, showcase_tile_center_in_rect,
};

const EXHAUSTED_SCALE: f32 = 0.90;

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
    window_w: f32,
    window_h: f32,
) {
    draw_row_bands(frame, layout);
    draw_row_labels(frame, texts, layout, window_h);

    for (idx, entry) in stats.entries.iter().enumerate() {
        if !screen.face_visible(entry) {
            continue;
        }
        let Some(cell) = grid_cell_rect(layout, idx) else {
            continue;
        };
        let exhausted = entry.locations.in_wall == 0;
        let focused = focus == Some(LedgerNav::Tile(idx))
            || (screen.selected.suit == entry.suit && screen.selected.rank == entry.rank);

        let (slip, tile_area) = cell_areas(cell);
        push_tile_slip(frame, slip, exhausted, focused);

        if let Some(group) = groups.get(&(entry.suit, entry.rank)) {
            if let Some(rep) = representative_entry(&group.copies) {
                push_cell_tile(
                    placements, rep, exhausted, focused, tile_area, run, window_w, window_h,
                );
            }
        } else {
            push_cell_tile_from_face(
                placements, entry.suit, entry.rank, exhausted, focused, tile_area, run, window_w,
                window_h,
            );
        }

        let mod_mark = entry.modifiers.pearl
            + entry.modifiers.gilded
            + entry.modifiers.polychrome
            + entry.modifiers.debuffed;
        if mod_mark > 0 {
            push_text_maybe_clip(
                texts,
                [slip[0] + slip[2] - 11.0, slip[1] + 2.0, 10.0, 10.0],
                "◆",
                layout.caption_px,
                color::alpha(color::GOLD, 0.85),
                false,
                TextAlign::Center,
                None,
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

fn cell_areas(cell: [f32; 4]) -> ([f32; 4], [f32; 4]) {
    let slip_inset = 1.0;
    let slip = [
        cell[0] + slip_inset,
        cell[1] + 0.5,
        cell[2] - slip_inset * 2.0,
        cell[3] - 1.0,
    ];
    (slip, slip)
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
            color: color::alpha(color::WALNUT_BRIGHT, if exhausted { 0.18 } else { 0.34 }),
            user: 0,
        });
        if exhausted {
            push_corner_stamp(frame, slip, 0.20);
        }
    } else if !exhausted {
        frame.quad(GpuInstance {
            rect: slip,
            color: color::alpha(color::WALNUT_BRIGHT, 0.14),
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
    focused: bool,
    tile_area: [f32; 4],
    run: &RunState,
    window_w: f32,
    window_h: f32,
) {
    push_cell_tile_inner(
        placements,
        GameEngine::display_tile(entry.tile, run),
        exhausted,
        focused,
        tile_area,
        window_w,
        window_h,
    );
}

fn push_cell_tile_from_face(
    placements: &mut Vec<ShowcaseTilePlacement>,
    suit: Suit,
    rank: u8,
    exhausted: bool,
    focused: bool,
    tile_area: [f32; 4],
    run: &RunState,
    window_w: f32,
    window_h: f32,
) {
    let tile = Tile::new(suit, rank, 0);
    push_cell_tile_inner(
        placements,
        GameEngine::display_tile(tile, run),
        exhausted,
        focused,
        tile_area,
        window_w,
        window_h,
    );
}

fn push_cell_tile_inner(
    placements: &mut Vec<ShowcaseTilePlacement>,
    tile: Tile,
    exhausted: bool,
    focused: bool,
    tile_area: [f32; 4],
    window_w: f32,
    window_h: f32,
) {
    let tile_size = ledger_grid_tile_size(tile_area, focused);
    placements.push(ShowcaseTilePlacement {
        tile,
        center_pos: showcase_tile_center_in_rect(tile_area, tile_size, window_w, window_h),
        rotation: TOP_DOWN_TILE_ROTATION,
        scale: if exhausted { EXHAUSTED_SCALE } else { 1.0 },
        size_px: tile_size,
        brightness: ledger_tile_brightness(exhausted, focused),
        opacity: 1.0,
        selected: focused && !exhausted,
        hovered: focused && exhausted,
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
