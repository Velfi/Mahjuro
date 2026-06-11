//! Shared showcase tile sizing and centering for the wall ledger.

use crate::render::doc_tile_camera::{TOP_DOWN_TILE_ROTATION, wall_ledger_camera};
use crate::render::showcase_tile_layout::{
    ShowcaseTileProjectParams, showcase_tile_projected_bounds_px,
};
use mahjuro_gfx_types::TilePreset;

pub const GRID_ACTIVE_BRIGHTNESS: f32 = 1.22;
pub const GRID_FOCUS_BRIGHTNESS: f32 = 1.34;
pub const GRID_EXHAUSTED_BRIGHTNESS: f32 = 0.42;

/// Footprint for a grid cell — height-led, capped by width so neighbors don't collide.
pub fn ledger_grid_tile_size(tile_area: [f32; 4], focused: bool) -> f32 {
    let h_fill = if focused { 0.86 } else { 0.81 };
    let by_height = tile_area[3] * h_fill;
    let width_cap = tile_area[2] * if focused { 1.10 } else { 1.04 };
    by_height.min(width_cap)
}

pub fn ledger_tile_brightness(exhausted: bool, focused: bool) -> f32 {
    if exhausted {
        GRID_EXHAUSTED_BRIGHTNESS
    } else if focused {
        GRID_FOCUS_BRIGHTNESS
    } else {
        GRID_ACTIVE_BRIGHTNESS
    }
}

/// Nudge placement so the tile's projected silhouette is centered in `target_rect`.
pub fn showcase_tile_center_in_rect(
    target_rect: [f32; 4],
    size_px: f32,
    win_w: f32,
    win_h: f32,
) -> [f32; 3] {
    let cam = wall_ledger_camera(win_h);
    let guess = [
        target_rect[0] + target_rect[2] * 0.5,
        target_rect[1] + target_rect[3] * 0.5,
        0.0,
    ];
    let bounds = showcase_tile_projected_bounds_px(&ShowcaseTileProjectParams {
        win_w,
        win_h,
        cam: &cam,
        preset: TilePreset::Chinese,
        center_px: guess,
        rotation_xyz_rad: TOP_DOWN_TILE_ROTATION,
        placement_scale: 1.0,
        size_px,
        use_ray_plane: false,
    });
    let target_cx = target_rect[0] + target_rect[2] * 0.5;
    let target_cy = target_rect[1] + target_rect[3] * 0.5;
    let bounds_cx = (bounds.min_x + bounds.max_x) * 0.5;
    let bounds_cy = (bounds.min_y + bounds.max_y) * 0.5;
    [
        guess[0] + target_cx - bounds_cx,
        guess[1] + target_cy - bounds_cy,
        0.0,
    ]
}
