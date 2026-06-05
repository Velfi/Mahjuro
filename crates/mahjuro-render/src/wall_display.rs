//! Wall-tiles-remaining HUD (gameplay lower-right corner).

use crate::decal::{load_ui_font, measure_label_advances};
use crate::draw_cmd::UiFrame;
use crate::theme::{color, typography};
use crate::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};

/// Layout for the compact wall counter in the lower-right corner.
#[derive(Clone, Copy, Debug)]
pub struct WallHudLayout {
    pub block_rect: [f32; 4],
}

/// Compute HUD geometry without drawing (for focus rects / hit tests).
pub fn wall_hud_layout(window_w: f32, window_h: f32, count: usize) -> WallHudLayout {
    let (block_left, block_top, block_w, block_h) =
        wall_hud_block_metrics(window_w, window_h, count);
    WallHudLayout {
        block_rect: [block_left, block_top, block_w, block_h],
    }
}

/// Compact wall counter: small tile icon + remaining count in the lower-right.
pub fn push_wall_remaining_hud(
    frame: &mut UiFrame,
    window_w: f32,
    window_h: f32,
    tiles_left: usize,
) -> WallHudLayout {
    let layout = wall_hud_layout(window_w, window_h, tiles_left);
    let [block_left, block_top, block_w, block_h] = layout.block_rect;
    let icon_w = (window_h * 0.030).clamp(24.0, 38.0);
    let icon_h = icon_w * 1.28;
    let pad = typography::size(typography::H20, window_h) * 0.22;

    frame.overlay_quads([
        GpuInstance {
            rect: [
                block_left - 4.0,
                block_top - 3.0,
                block_w + 8.0,
                block_h + 7.0,
            ],
            color: color::alpha(color::WALNUT_INK, 0.42),
            user: 0,
        },
        GpuInstance {
            rect: [block_left, block_top, block_w, block_h],
            color: [
                color::WALNUT_DEEP[0],
                color::WALNUT_DEEP[1],
                color::WALNUT_DEEP[2],
                0.88,
            ],
            user: 0,
        },
    ]);

    let icon_left = block_left + pad;
    let icon_top = block_top + (block_h - icon_h) * 0.5;
    push_wall_tile_icon(frame, icon_left, icon_top, icon_w, icon_h);

    let gap = icon_w * 0.22;
    let font_px = typography::size(typography::H20, window_h);
    let count_text = format!("{}", tiles_left);
    let h_px = font_px.max(1.0).round().max(1.0) as u32;
    let text_w = if let Some(font) = load_ui_font() {
        let (_, _, advances) = measure_label_advances(font, &count_text, 8192, h_px, Some(font_px));
        advances.iter().sum::<f32>().max(font_px * 1.1)
    } else {
        font_px * count_text.chars().count().max(1) as f32 * 0.62
    };
    let text_h = font_px * 1.32;
    let text_left = icon_left + icon_w + gap;
    let text_top = block_top + (block_h - text_h) * 0.5;
    frame.texts([TextLabel {
        rect: [text_left, text_top, text_w, text_h],
        text: count_text,
        color: color::CHAMPAGNE,
        font_px: Some(font_px),
        align: TextAlign::Left,
        scroll_offset: 0.0,
        flavor_spans: None,
        bold: false,
        italic: false,
        underline: false,
        text_effect: crate::text_effect::TextEffectId::Flat,
        rotation_quarters: 0,
        baseline_shift_px: 0.0,
        clip_rect: None,
        mono: false,
    }]);
    layout
}

fn wall_hud_block_metrics(window_w: f32, window_h: f32, tiles_left: usize) -> (f32, f32, f32, f32) {
    let margin_x = window_w * 0.018;
    let margin_y = window_h * 0.016;
    let icon_w = (window_h * 0.030).clamp(24.0, 38.0);
    let icon_h = icon_w * 1.28;
    let gap = icon_w * 0.22;
    let font_px = typography::size(typography::H20, window_h);
    let count_text = format!("{}", tiles_left);
    let h_px = font_px.max(1.0).round().max(1.0) as u32;
    let text_w = if let Some(font) = load_ui_font() {
        let (_, _, advances) = measure_label_advances(font, &count_text, 8192, h_px, Some(font_px));
        advances.iter().sum::<f32>().max(font_px * 1.1)
    } else {
        font_px * count_text.chars().count().max(1) as f32 * 0.62
    };
    let text_h = font_px * 1.32;
    let pad = font_px * 0.22;
    let block_w = pad * 2.0 + icon_w + gap + text_w;
    let block_h = pad * 2.0 + icon_h.max(text_h);
    let block_left = window_w - margin_x - block_w;
    let block_top = window_h - margin_y - block_h;
    (block_left, block_top, block_w, block_h)
}

fn push_wall_tile_icon(frame: &mut UiFrame, left: f32, top: f32, tile_w: f32, tile_h: f32) {
    let back = [0.86, 0.81, 0.69, 1.0];
    let offset = tile_w * 0.14;
    frame.overlay_quads([
        GpuInstance {
            rect: [left + offset * 0.5, top - offset * 0.35, tile_w, tile_h],
            color: color::alpha(color::WALNUT_INK, 0.35),
            user: 0,
        },
        GpuInstance {
            rect: [left + offset, top - offset, tile_w, tile_h],
            color: color::alpha(back, 0.92),
            user: 0,
        },
        GpuInstance {
            rect: [left, top, tile_w, tile_h],
            color: back,
            user: 0,
        },
        GpuInstance {
            rect: [left, top, tile_w, tile_h * 0.08],
            color: color::alpha(color::CHAMPAGNE, 0.22),
            user: 0,
        },
    ]);
}
