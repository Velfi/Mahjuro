//! Selected-tile detail panel in the left sidebar stack.

use crate::core::tile::Tile;
use crate::game::engine::GameEngine;
use crate::game::run::RunState;
use crate::game::wall_stats::{
    AbundanceState, ModifierBreakdown, SelectedTileDetails, abundance_color, abundance_state,
};
use crate::render::doc_tile_camera::TOP_DOWN_TILE_ROTATION;
use crate::render::draw_cmd::{ShowcaseTilePlacement, UiFrame};
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextAlign};

use super::super::layout::{LEDGER_FOCUS_GLOW, LEDGER_FOCUS_OUTLINE, WallLayout, text_line_h};
use super::text::{push_plaque, push_text};

pub fn draw_wall_detail_panel(
    frame: &mut UiFrame,
    texts: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
    placements: &mut Vec<ShowcaseTilePlacement>,
    layout: &WallLayout,
    details: &SelectedTileDetails,
    run: &RunState,
    representative: Option<&Tile>,
) {
    let rect = [
        layout.summary_x,
        layout.detail_y,
        layout.summary_w,
        layout.detail_h,
    ];
    frame.quad(GpuInstance {
        rect: [rect[0], rect[1], rect[2], 3.0],
        color: color::alpha(color::BRASS, 0.44),
        user: 0,
    });
    push_plaque(frame, rect, 0.88);

    let pad = 10.0;
    let header_line = text_line_h(layout.caption_px * 0.92);
    push_text(
        texts,
        [
            rect[0] + pad,
            rect[1] + 8.0,
            rect[2] - pad * 2.0,
            header_line,
        ],
        "SELECTED TILE",
        layout.caption_px * 0.92,
        color::alpha(color::BRASS, 0.78),
        true,
        TextAlign::Left,
    );

    let preview_size = (rect[2] * 0.36).min(rect[3] * 0.52).clamp(64.0, 136.0);
    let preview_rect = [
        rect[0] + pad,
        rect[1] + header_line + 10.0,
        preview_size,
        preview_size * 1.08,
    ];
    frame.quad(GpuInstance {
        rect: [
            preview_rect[0] - 2.0,
            preview_rect[1] - 2.0,
            preview_rect[2] + 4.0,
            preview_rect[3] + 4.0,
        ],
        color: LEDGER_FOCUS_GLOW,
        user: 0,
    });
    frame.quad(GpuInstance {
        rect: preview_rect,
        color: color::alpha(color::WALNUT_INK, 0.55),
        user: 0,
    });
    push_border(frame, preview_rect, 1.0, color::alpha(color::STONE, 0.16));

    let exhausted = details.remaining == 0;
    let state = abundance_state(details.face.suit, details.remaining);
    let state_label = match state {
        AbundanceState::Exhausted => "EXHAUSTED",
        AbundanceState::Thin => "THIN",
        AbundanceState::Abundant => "ABUNDANT",
        AbundanceState::Normal => "AVAILABLE",
    };

    if let Some(tile) = representative {
        placements.push(ShowcaseTilePlacement {
            tile: GameEngine::display_tile(*tile, run),
            center_pos: [
                preview_rect[0] + preview_rect[2] * 0.5,
                preview_rect[1] + preview_rect[3] * 0.52,
                0.0,
            ],
            rotation: TOP_DOWN_TILE_ROTATION,
            scale: 1.0,
            size_px: preview_rect[2].min(preview_rect[3]) * 0.82,
            brightness: if exhausted { 0.40 } else { 1.0 },
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
    push_border(frame, preview_rect, 1.5, LEDGER_FOCUS_OUTLINE);
    push_focus_corner(
        frame,
        preview_rect[0] + preview_rect[2] - 6.0,
        preview_rect[1] + 4.0,
    );

    let text_x = preview_rect[0] + preview_rect[2] + 10.0;
    let text_w = (rect[0] + rect[2] - pad - text_x).max(32.0);
    let mut y = rect[1] + header_line + 14.0;
    let title_lh = text_line_h(layout.body_px * 0.94);
    push_text(
        texts,
        [text_x, y, text_w, title_lh],
        details.name.clone(),
        layout.body_px * 0.94,
        color::alpha(color::JADE, if exhausted { 0.75 } else { 0.96 }),
        true,
        TextAlign::Left,
    );
    y += title_lh + 2.0;

    push_text(
        texts,
        [text_x, y, text_w, text_line_h(layout.caption_px)],
        state_label,
        layout.caption_px * 0.92,
        abundance_color(state),
        true,
        TextAlign::Left,
    );
    y += text_line_h(layout.caption_px) + 2.0;

    for line in [
        format!("{} remaining · was {}", details.remaining, details.total),
        format!("{:.1}% draw chance", details.draw_probability * 100.0),
    ] {
        push_text(
            texts,
            [text_x, y, text_w, text_line_h(layout.caption_px)],
            line,
            layout.caption_px,
            color::alpha(color::CHAMPAGNE, if exhausted { 0.76 } else { 0.92 }),
            false,
            TextAlign::Left,
        );
        y += text_line_h(layout.caption_px) + 2.0;
    }

    let body_top = preview_rect[1] + preview_rect[3] + 10.0;
    frame.quad(GpuInstance {
        rect: [rect[0] + pad, body_top - 4.0, rect[2] - pad * 2.0, 1.0],
        color: color::alpha(color::STONE, 0.14),
        user: 0,
    });

    if let Some(mod_line) = modifier_summary(&details.modifiers) {
        push_text(
            texts,
            [
                rect[0] + pad,
                body_top,
                rect[2] - pad * 2.0,
                text_line_h(layout.caption_px),
            ],
            mod_line,
            layout.caption_px * 0.9,
            color::alpha(color::STONE, 0.88),
            false,
            TextAlign::Left,
        );
    }
}

fn push_focus_corner(frame: &mut UiFrame, x: f32, y: f32) {
    frame.quad(GpuInstance {
        rect: [x, y, 4.0, 4.0],
        color: color::alpha(LEDGER_FOCUS_OUTLINE, 0.85),
        user: 0,
    });
}

fn push_border(frame: &mut UiFrame, rect: [f32; 4], t: f32, c: [f32; 4]) {
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

fn modifier_summary(m: &ModifierBreakdown) -> Option<String> {
    let mut parts = Vec::new();
    if m.pearl > 0 {
        parts.push(format!("Pearl ×{}", m.pearl));
    }
    if m.gilded > 0 {
        parts.push(format!("Gilded ×{}", m.gilded));
    }
    if m.polychrome > 0 {
        parts.push(format!("Poly ×{}", m.polychrome));
    }
    if m.debuffed > 0 {
        parts.push(format!("Debuff ×{}", m.debuffed));
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}
