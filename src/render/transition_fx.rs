use std::f32::consts::PI;

use crate::core::tile::{Suit, Tile};
use crate::render::draw_cmd::{TileFaceQuad, UiFrame};
use crate::render::theme::color;
use crate::render::wgpu_renderer::GpuInstance;

/// ~1080p; scales particle-style transitions so tile count tracks screen area.
const TRANSITION_REF_AREA: f32 = 1920.0 * 1080.0;

/// Grid overlays target ~`divisor` columns via `w / divisor`; clamps only enforce a
/// readable minimum size and stop tiles from dominating very small windows.
fn grid_overlay_tile_short_px(w: f32, divisor: f32, min_px: f32, max_frac_w: f32) -> f32 {
    (w / divisor).clamp(min_px, w * max_frac_w)
}

/// Fixed-layout transitions at reference resolution use `base_at_ref` tiles; scales with
/// window area (capped) so density stays similar from laptop to 4K.
fn scaled_overlay_tile_count(base_at_ref: f32, w: f32, h: f32, min_c: usize, max_c: usize) -> usize {
    let n = (base_at_ref * w * h / TRANSITION_REF_AREA).round() as i32;
    n.max(min_c as i32).min(max_c as i32) as usize
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayTransitionKind {
    TileTeeth,
    ForestOfTiles,
    GalaxyOfTiles,
    Maelstrom,
    TileWaterfall,
    ShufflingFan,
}

pub fn push_overlay_transition(
    frame: &mut UiFrame,
    kind: OverlayTransitionKind,
    progress: f32,
    window: (f32, f32),
) {
    match kind {
        OverlayTransitionKind::TileTeeth => push_tile_teeth(frame, progress, window),
        OverlayTransitionKind::ForestOfTiles => push_forest_of_tiles(frame, progress, window),
        OverlayTransitionKind::GalaxyOfTiles => push_galaxy_of_tiles(frame, progress, window),
        OverlayTransitionKind::Maelstrom => push_maelstrom(frame, progress, window),
        OverlayTransitionKind::TileWaterfall => push_tile_waterfall(frame, progress, window),
        OverlayTransitionKind::ShufflingFan => push_shuffling_fan(frame, progress, window),
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

    let tile_size = grid_overlay_tile_short_px(w, 18.5, 30.0, 0.056);
    let tooth_overlap = tile_size * 0.18;
    let column_step = tile_size * 1.22;
    let cols = (w / column_step).ceil() as usize + 1;
    let row_step = tile_size - tooth_overlap;
    let rows = (h / row_step).ceil() as usize + 2;
    let overshoot = tile_size * 1.35;
    let close_span = h * 0.5 + tile_size * 0.35;
    let seq = tile_sequence();
    let mut quads = Vec::with_capacity(cols * rows * 3);
    let mut faces = Vec::with_capacity(cols * rows);

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
        let pitch: f32 = if from_top { 0.30 } else { -0.30 };
        let yaw: f32 = ((col % 3) as f32 - 1.0) * 0.045;
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
            push_flat_tile(
                &mut quads,
                &mut faces,
                tile,
                [left, top, tile_w, tile_h],
                [shadow_dx, shadow_dy],
                [
                    face[0] * brightness,
                    face[1] * brightness,
                    face[2] * brightness,
                    0.98,
                ],
                shadow_alpha,
                edge_alpha,
                0.14 + local_cover * 0.10,
            );
        }
    }

    flush_tiles(frame, quads, faces);
}

fn push_forest_of_tiles(frame: &mut UiFrame, progress: f32, window: (f32, f32)) {
    let (w, h) = window;
    let t = progress.clamp(0.0, 1.0);
    let cover = phased_cover(t);
    if cover <= 0.001 || w <= 1.0 || h <= 1.0 {
        return;
    }

    frame.quad(GpuInstance {
        rect: [0.0, 0.0, w, h],
        color: color::alpha(color::OBSIDIAN, 0.12 + cover * 0.42),
    });

    let tile_w = grid_overlay_tile_short_px(w, 17.0, 30.0, 0.062);
    let tile_h = tile_w * 1.34;
    let col_step = tile_w * 0.74;
    let cols = (w / col_step).ceil() as usize + 3;
    let seq = tile_sequence();
    let mut quads = Vec::with_capacity(cols * 12 * 3);
    let mut faces = Vec::with_capacity(cols * 12);

    for col in 0..cols {
        let x = -tile_w * 0.35 + col as f32 * col_step;
        let lane = col as f32 / cols.max(1) as f32;
        let lane_bias = ((col % 5) as f32 - 2.0) * 0.035;
        let local_cover = smoothstep((cover + lane_bias).clamp(0.0, 1.0));
        if local_cover <= 0.001 {
            continue;
        }
        let stack_height = ((h * (0.22 + 0.92 * local_cover)) / (tile_h * 0.56)).ceil() as usize;
        let sway = ((t * PI * 3.4) + col as f32 * 0.45).sin() * tile_w * 0.08;
        let lift = tile_h * 0.48;
        for row in 0..stack_height {
            let top = h - tile_h + lift - row as f32 * tile_h * 0.56;
            if top > h + tile_h || top + tile_h < -tile_h * 0.2 {
                continue;
            }
            let depth = row as f32 / stack_height.max(1) as f32;
            let left = x + sway * (0.35 + depth * 0.65);
            let tile = seq[(col * 3 + row) % seq.len()];
            let alpha = (0.34 + 0.60 * (1.0 - depth) + 0.10 * local_cover).clamp(0.0, 1.0);
            let face = color::lighten(color::PARCHMENT, 0.04 + lane * 0.08);
            push_flat_tile(
                &mut quads,
                &mut faces,
                tile,
                [left, top, tile_w, tile_h],
                [tile_w * 0.05, tile_h * 0.07],
                [face[0], face[1], face[2], alpha],
                0.10 + local_cover * 0.12,
                alpha,
                0.06 + (1.0 - depth) * 0.10,
            );
        }
    }

    flush_tiles(frame, quads, faces);
}

fn push_maelstrom(frame: &mut UiFrame, progress: f32, window: (f32, f32)) {
    let (w, h) = window;
    let t = progress.clamp(0.0, 1.0);
    let cover = phased_cover(t);
    if cover <= 0.001 || w <= 1.0 || h <= 1.0 {
        return;
    }

    frame.quad(GpuInstance {
        rect: [0.0, 0.0, w, h],
        color: color::alpha(color::OBSIDIAN, 0.16 + cover * 0.46),
    });

    let seq = tile_sequence();
    let count = scaled_overlay_tile_count(96.0, w, h, 56, 220);
    let max_radius = w.max(h) * 0.72;
    let min_radius = w.min(h) * 0.08;
    let cx = w * (0.52 + (t * PI * 1.3).sin() * 0.05);
    let cy = h * (0.48 + (t * PI * 1.9).cos() * 0.04);
    let mut quads = Vec::with_capacity(count * 3);
    let mut faces = Vec::with_capacity(count);

    for i in 0..count {
        let f = i as f32 / count as f32;
        let arm = (i % 4) as f32;
        let phase = t * PI * 3.8;
        let angle = phase + f * PI * 5.6 + arm * PI * 0.5;
        let radius = min_radius + (1.0 - smoothstep(cover)) * max_radius * (0.20 + f * 0.92);
        let orbit = radius + (f * PI * 5.0 + t * PI * 2.0).sin() * w.min(h) * 0.02;
        let min_d = w.min(h);
        let size = (min_d * (0.045 + (1.0 - f) * 0.055)).clamp(min_d * 0.028, min_d * 0.078);
        let tile_w = size;
        let tile_h = size * 1.34;
        let left = cx + angle.cos() * orbit - tile_w * 0.5;
        let top = cy + angle.sin() * orbit - tile_h * 0.5;
        let alpha = (0.26 + cover * 0.52 + (1.0 - f) * 0.22).clamp(0.0, 1.0);
        let face = color::lighten(color::PARCHMENT, 0.03 + (1.0 - f) * 0.10);
        push_flat_tile(
            &mut quads,
            &mut faces,
            seq[(i * 7) % seq.len()],
            [left, top, tile_w, tile_h],
            [angle.cos() * tile_w * 0.08, angle.sin() * tile_h * 0.08],
            [face[0], face[1], face[2], alpha],
            0.12 + cover * 0.12,
            alpha,
            0.08 + cover * 0.10,
        );
    }

    flush_tiles(frame, quads, faces);
}

fn push_galaxy_of_tiles(frame: &mut UiFrame, progress: f32, window: (f32, f32)) {
    let (w, h) = window;
    let t = progress.clamp(0.0, 1.0);
    let cover = phased_cover(t);
    if cover <= 0.001 || w <= 1.0 || h <= 1.0 {
        return;
    }

    frame.quad(GpuInstance {
        rect: [0.0, 0.0, w, h],
        color: color::alpha(color::OBSIDIAN, 0.18 + cover * 0.44),
    });

    let cx = w * 0.50;
    let cy = h * 0.47;
    let core_r = w.min(h) * (0.08 + cover * 0.10);
    frame.quad(GpuInstance {
        rect: [cx - core_r, cy - core_r, core_r * 2.0, core_r * 2.0],
        color: color::alpha(color::CHAMPAGNE, 0.10 + cover * 0.18),
    });

    let arms = 4usize;
    let total_spiral = scaled_overlay_tile_count(104.0, w, h, 48, 220);
    let per_arm = ((total_spiral + arms - 1) / arms).max(12);
    let seq = tile_sequence();
    let max_radius = w.max(h) * 0.62;
    let spin = t * PI * 1.6;
    let mut quads = Vec::with_capacity(arms * per_arm * 3);
    let mut faces = Vec::with_capacity(arms * per_arm);

    for arm in 0..arms {
        let arm_phase = arm as f32 * (PI * 2.0 / arms as f32);
        let arm_cover = smoothstep((cover + arm as f32 * 0.02 - 0.03).clamp(0.0, 1.0));
        if arm_cover <= 0.001 {
            continue;
        }
        for i in 0..per_arm {
            let f = i as f32 / per_arm.max(1) as f32;
            let radius = max_radius * (0.14 + f.powf(0.82) * 0.94) * (1.02 - arm_cover * 0.18);
            let angle = arm_phase + spin + f * PI * 1.55;
            let drift = ((t * PI * 2.8) + i as f32 * 0.55 + arm as f32).sin() * w.min(h) * 0.012;
            let orbit_x = (radius + drift) * angle.cos();
            let orbit_y = (radius * 0.64 + drift * 0.6) * angle.sin();
            let min_d = w.min(h);
            let size = (min_d * (0.040 + (1.0 - f) * 0.040)).clamp(min_d * 0.026, min_d * 0.068);
            let tile_w = size;
            let tile_h = size * 1.34;
            let left = cx + orbit_x - tile_w * 0.5;
            let top = cy + orbit_y - tile_h * 0.5;
            let alpha = (0.20 + arm_cover * 0.48 + (1.0 - f) * 0.28).clamp(0.0, 1.0);
            let face = color::lighten(color::PARCHMENT, 0.05 + (1.0 - f) * 0.12);
            let shadow_dx = angle.cos() * tile_w * 0.06;
            let shadow_dy = angle.sin() * tile_h * 0.06;
            push_flat_tile(
                &mut quads,
                &mut faces,
                seq[(arm * 7 + i * 3) % seq.len()],
                [left, top, tile_w, tile_h],
                [shadow_dx, shadow_dy],
                [face[0], face[1], face[2], alpha],
                0.10 + arm_cover * 0.10,
                alpha,
                0.08 + arm_cover * 0.10,
            );
        }
    }

    let star_count = scaled_overlay_tile_count(18.0, w, h, 12, 36);
    for i in 0..star_count {
        let f = i as f32 / star_count as f32;
        let angle = spin * 1.4 + f * PI * 2.0;
        let radius = max_radius * (0.70 + f * 0.28);
        let min_dim = w.min(h);
        let size = (min_dim * 0.022).clamp(8.0, min_dim * 0.028);
        frame.quad(GpuInstance {
            rect: [
                cx + angle.cos() * radius - size * 0.5,
                cy + angle.sin() * radius * 0.72 - size * 0.5,
                size,
                size,
            ],
            color: color::alpha(color::CHAMPAGNE, 0.05 + cover * 0.08),
        });
    }

    flush_tiles(frame, quads, faces);
}

fn push_tile_waterfall(frame: &mut UiFrame, progress: f32, window: (f32, f32)) {
    let (w, h) = window;
    let t = progress.clamp(0.0, 1.0);
    let cover = phased_cover(t);
    if cover <= 0.001 || w <= 1.0 || h <= 1.0 {
        return;
    }

    frame.quad(GpuInstance {
        rect: [0.0, 0.0, w, h],
        color: color::alpha(color::OBSIDIAN, 0.10 + cover * 0.38),
    });

    let tile_w = grid_overlay_tile_short_px(w, 19.0, 28.0, 0.058);
    let tile_h = tile_w * 1.34;
    let col_step = tile_w * 1.08;
    let cols = (w / col_step).ceil() as usize + 2;
    let rows = (h / (tile_h * 0.82)).ceil() as usize + 4;
    let seq = tile_sequence();
    let mut quads = Vec::with_capacity(cols * rows * 3);
    let mut faces = Vec::with_capacity(cols * rows);

    for col in 0..cols {
        let x = -tile_w * 0.2 + col as f32 * col_step;
        let phase = ((col % 7) as f32 - 3.0) * 0.028;
        let local_cover = smoothstep((cover + phase).clamp(0.0, 1.0));
        if local_cover <= 0.001 {
            continue;
        }
        let lead = -tile_h * 1.8 + (h + tile_h * 0.9) * local_cover;
        let spray = ((t * PI * 4.6) + col as f32 * 0.7).sin() * tile_w * 0.06;
        for row in 0..rows {
            let top = lead - row as f32 * tile_h * 0.82;
            if top < -tile_h * 2.0 || top > h + tile_h * 0.3 {
                continue;
            }
            let tile = seq[(col + row * 2) % seq.len()];
            let foam = (row as f32 / rows.max(1) as f32).powf(1.3);
            let alpha = (0.42 + local_cover * 0.42 - foam * 0.16).clamp(0.0, 1.0);
            let face = color::lighten(color::PARCHMENT, 0.02 + local_cover * 0.06);
            push_flat_tile(
                &mut quads,
                &mut faces,
                tile,
                [x + spray, top, tile_w, tile_h],
                [0.0, tile_h * 0.09],
                [face[0], face[1], face[2], alpha],
                0.10 + local_cover * 0.10,
                alpha,
                0.08 + local_cover * 0.08,
            );
        }
    }

    flush_tiles(frame, quads, faces);
}

fn push_shuffling_fan(frame: &mut UiFrame, progress: f32, window: (f32, f32)) {
    let (w, h) = window;
    let t = progress.clamp(0.0, 1.0);
    let cover = phased_cover(t);
    if cover <= 0.001 || w <= 1.0 || h <= 1.0 {
        return;
    }

    frame.quad(GpuInstance {
        rect: [0.0, 0.0, w, h],
        color: color::alpha(color::OBSIDIAN, 0.13 + cover * 0.40),
    });

    let seq = tile_sequence();
    let fan_count = 4usize;
    let total_fan = scaled_overlay_tile_count(68.0, w, h, 36, 160);
    let per_fan = ((total_fan + fan_count - 1) / fan_count).max(8);
    let min_d = w.min(h);
    let base_size = (min_d * 0.075).clamp(34.0, min_d * 0.102);
    let mut quads = Vec::with_capacity(fan_count * per_fan * 3);
    let mut faces = Vec::with_capacity(fan_count * per_fan);

    for fan in 0..fan_count {
        let side = if fan % 2 == 0 { -1.0 } else { 1.0 };
        let fan_t = smoothstep((cover + fan as f32 * 0.035 - 0.04).clamp(0.0, 1.0));
        if fan_t <= 0.001 {
            continue;
        }
        let cx = if side < 0.0 {
            -base_size * 2.2 + w * 0.18 * fan_t
        } else {
            w + base_size * 1.2 - w * 0.18 * fan_t
        };
        let cy = h * (0.20 + fan as f32 * 0.18);
        let arc = PI * (0.48 + 0.06 * fan as f32);
        for i in 0..per_fan {
            let f = if per_fan <= 1 {
                0.5
            } else {
                i as f32 / (per_fan - 1) as f32
            };
            let angle = if side < 0.0 {
                -arc * 0.15 + f * arc
            } else {
                PI - f * arc + arc * 0.15
            };
            let radius = w * (0.12 + 0.58 * fan_t) + (f - 0.5).abs() * w * 0.06;
            let tile_w = base_size * (0.88 + (1.0 - f) * 0.18);
            let tile_h = tile_w * 1.34;
            let left = cx + angle.cos() * radius - tile_w * 0.5;
            let top = cy + angle.sin() * radius - tile_h * 0.5;
            let alpha = (0.34 + fan_t * 0.54 - (f - 0.5).abs() * 0.10).clamp(0.0, 1.0);
            let face = color::lighten(color::PARCHMENT, 0.03 + fan_t * 0.08);
            push_flat_tile(
                &mut quads,
                &mut faces,
                seq[(fan * 5 + i) % seq.len()],
                [left, top, tile_w, tile_h],
                [side * tile_w * 0.09, tile_h * 0.05],
                [face[0], face[1], face[2], alpha],
                0.10 + fan_t * 0.10,
                alpha,
                0.08 + fan_t * 0.08,
            );
        }
    }

    flush_tiles(frame, quads, faces);
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

fn push_flat_tile(
    quads: &mut Vec<GpuInstance>,
    faces: &mut Vec<TileFaceQuad>,
    tile: Tile,
    rect: [f32; 4],
    shadow_offset: [f32; 2],
    fill: [f32; 4],
    shadow_alpha: f32,
    face_alpha: f32,
    sheen_alpha: f32,
) {
    let [left, top, tile_w, tile_h] = rect;
    quads.push(GpuInstance {
        rect: [
            left + shadow_offset[0],
            top + shadow_offset[1],
            tile_w,
            tile_h,
        ],
        color: color::alpha(color::OBSIDIAN, shadow_alpha.clamp(0.0, 1.0)),
    });
    quads.push(GpuInstance { rect, color: fill });
    quads.push(GpuInstance {
        rect: [left, top, tile_w, tile_h * 0.08],
        color: color::alpha(color::CHAMPAGNE, sheen_alpha.clamp(0.0, 1.0)),
    });

    let inset_x = tile_w * 0.10;
    faces.push(TileFaceQuad {
        tile,
        inst: GpuInstance {
            rect: [
                left + inset_x,
                top + tile_h * 0.10,
                tile_w - inset_x * 2.0,
                tile_h * 0.74,
            ],
            color: [1.0, 1.0, 1.0, face_alpha.clamp(0.0, 1.0)],
        },
    });
}

fn flush_tiles(frame: &mut UiFrame, quads: Vec<GpuInstance>, faces: Vec<TileFaceQuad>) {
    if !quads.is_empty() {
        frame.quads(quads);
    }
    if !faces.is_empty() {
        frame.tile_face_quads(faces);
    }
}
