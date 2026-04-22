use std::f32::consts::PI;

use crate::core::tile::{Suit, Tile};
use crate::render::decal::{tile_short_label, tile_suit_emoji};
use crate::render::draw_cmd::UiFrame;
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayTransitionKind {
    TileTeeth,
}

pub fn push_overlay_transition(
    frame: &mut UiFrame,
    kind: OverlayTransitionKind,
    progress: f32,
    window: (f32, f32),
) {
    match kind {
        OverlayTransitionKind::TileTeeth => push_tile_teeth(frame, progress, window),
    }
}

fn push_tile_teeth(frame: &mut UiFrame, progress: f32, window: (f32, f32)) {
    let (w, h) = window;
    let t = progress.clamp(0.0, 1.0);
    let cover = phased_cover(t);
    if cover <= 0.001 || w <= 1.0 || h <= 1.0 {
        return;
    }

    frame.quad(GpuInstance {
        rect: [0.0, 0.0, w, h],
        color: color::alpha(color::OBSIDIAN, 0.14 + cover * 0.36),
    });

    let tile_size = (w / 18.5).clamp(40.0, 68.0);
    let tooth_overlap = tile_size * 0.18;
    let column_step = (tile_size * 1.22).max(42.0);
    let cols = (w / column_step).ceil() as usize + 1;
    let row_step = tile_size - tooth_overlap;
    let rows = (h / row_step).ceil() as usize + 2;
    let overshoot = tile_size * 1.35;
    let close_span = h * 0.5 + tile_size * 0.35;
    let seq = tile_sequence();
    let mut quads = Vec::with_capacity(cols * rows * 3);
    let mut labels = Vec::with_capacity(cols * rows * 2);

    for col in 0..cols {
        let from_top = col % 2 == 0;
        let x = tile_size * 0.5 + col as f32 * column_step;
        let phase_offset = ((col % 5) as f32 - 2.0) * 0.022;
        let local_cover = smoothstep((cover + phase_offset).clamp(0.0, 1.0));
        if local_cover <= 0.001 {
            continue;
        }

        let lead_y = if from_top {
            -overshoot + close_span * local_cover
        } else {
            h + overshoot - close_span * local_cover
        };
        let pitch = if from_top { 0.30 } else { -0.30 };
        let yaw = ((col % 3) as f32 - 1.0) * 0.045;
        let roll = if from_top { PI } else { PI + 0.02 };
        let base_lift = 38.0 + 14.0 * local_cover;

        for row in 0..rows {
            let center_y = if from_top {
                lead_y + row as f32 * row_step
            } else {
                lead_y - row as f32 * row_step
            };
            if center_y < -tile_size * 1.5 || center_y > h + tile_size * 1.5 {
                continue;
            }

            let tile = seq[(col + row) % seq.len()];
            let depth_wave = (((col + row) as f32 * 0.6) + t * PI * 3.0).sin() * 1.6;
            let tile_w = tile_size * (1.02 + yaw.abs() * 0.35);
            let tile_h = tile_size * 1.34;
            let shadow_dx = yaw * tile_size * 0.55;
            let shadow_dy = if from_top {
                tile_size * 0.11 + pitch.abs() * tile_size * 0.05
            } else {
                -tile_size * 0.03
            };
            let top = center_y - tile_h * 0.5;
            let left = x - tile_w * 0.5;
            let brightness = (0.92 + 0.10 * local_cover + depth_wave * 0.01).clamp(0.0, 1.2);
            let face = color::lighten(color::PARCHMENT, 0.02 + local_cover * 0.10);
            let shadow_alpha = (0.14 + local_cover * 0.12).clamp(0.0, 0.30);
            let edge_alpha = (0.75 + local_cover * 0.20).clamp(0.0, 1.0);
            quads.push(GpuInstance {
                rect: [left + shadow_dx, top + shadow_dy, tile_w, tile_h],
                color: color::alpha(color::OBSIDIAN, shadow_alpha),
            });
            quads.push(GpuInstance {
                rect: [left, top, tile_w, tile_h],
                color: [
                    face[0] * brightness,
                    face[1] * brightness,
                    face[2] * brightness,
                    0.98,
                ],
            });
            quads.push(GpuInstance {
                rect: [left, top, tile_w, tile_h * 0.08],
                color: color::alpha(color::CHAMPAGNE, 0.14 + local_cover * 0.10),
            });

            let inset_x = tile_w * 0.10;
            let label_color = match tile.suit {
                Suit::Characters | Suit::Dragon => color::RUBY,
                Suit::Bamboos | Suit::Flower | Suit::Season => color::JADE,
                Suit::Circles => color::GOLD,
                Suit::Wind => color::SLATE,
            };
            labels.push(TextLabel {
                rect: [left + inset_x, top + tile_h * 0.12, tile_w - inset_x * 2.0, tile_h * 0.40],
                text: tile_short_label(&tile),
                color: color::alpha(label_color, edge_alpha),
                font_px: Some(tile_h * 0.24),
                align: TextAlign::Center,
                no_glossary: true,
                scroll_offset: 0.0,
            });
            labels.push(TextLabel {
                rect: [left + inset_x, top + tile_h * 0.56, tile_w - inset_x * 2.0, tile_h * 0.22],
                text: tile_suit_emoji(&tile).to_string(),
                color: color::alpha(label_color, 0.90),
                font_px: Some(tile_h * 0.16),
                align: TextAlign::Center,
                no_glossary: true,
                scroll_offset: 0.0,
            });
        }
    }

    if !quads.is_empty() {
        frame.quads(quads);
    }
    if !labels.is_empty() {
        frame.texts(labels);
    }
}

fn tile_sequence() -> [Tile; 12] {
    [
        Tile::new(Suit::Characters, 1, 0),
        Tile::new(Suit::Characters, 9, 1),
        Tile::new(Suit::Bamboos, 2, 2),
        Tile::new(Suit::Bamboos, 8, 3),
        Tile::new(Suit::Circles, 3, 4),
        Tile::new(Suit::Circles, 7, 5),
        Tile::new(Suit::Wind, 1, 6),
        Tile::new(Suit::Wind, 4, 7),
        Tile::new(Suit::Dragon, 1, 8),
        Tile::new(Suit::Dragon, 2, 9),
        Tile::new(Suit::Dragon, 3, 10),
        Tile::new(Suit::Flower, 1, 11),
    ]
}

fn phased_cover(t: f32) -> f32 {
    const ENTER_END: f32 = 0.40;
    const HOLD_END: f32 = 0.60;
    if t < ENTER_END {
        t / ENTER_END
    } else if t < HOLD_END {
        1.0
    } else {
        1.0 - ((t - HOLD_END) / (1.0 - HOLD_END)).clamp(0.0, 1.0)
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
