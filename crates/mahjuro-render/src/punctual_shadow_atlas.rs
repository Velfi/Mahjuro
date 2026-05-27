//! Per-punctual-light shadow atlas for small interior scenes (gameplay candles).
//!
//! Each candle gets a tile in a shared depth atlas; lit shaders sample the tile
//! matching the point-light index in the punctual loop.

use glam::{Mat4, Vec3};

pub const MAX_PUNCTUAL_SHADOW_LIGHTS: usize = 8;

use crate::gameplay_glb::with_gameplay_glb_cpu;
use crate::room_glb::room_env_world_scale;

/// Shared depth atlas resolution (also used as the live key-light map elsewhere).
pub const PUNCTUAL_SHADOW_ATLAS_SIZE: u32 = 2048;
/// Per-light tile size — 4×2 grid fits [`MAX_PUNCTUAL_SHADOW_LIGHTS`] slots.
pub const PUNCTUAL_SHADOW_TILE_SIZE: u32 = 512;
const GRID_COLS: u32 = 4;

/// CPU-side setup for one gameplay candle shadow pass.
#[derive(Clone, Copy, Debug)]
pub struct PunctualShadowLightSetup {
    pub light_view_proj: Mat4,
    pub atlas_rect: [f32; 4],
}

#[inline]
pub fn atlas_tile_viewport_px(index: usize) -> (f32, f32, f32, f32) {
    let idx = index as u32;
    let col = idx % GRID_COLS;
    let row = idx / GRID_COLS;
    let x = (col * PUNCTUAL_SHADOW_TILE_SIZE) as f32;
    let y = (row * PUNCTUAL_SHADOW_TILE_SIZE) as f32;
    let s = PUNCTUAL_SHADOW_TILE_SIZE as f32;
    (x, y, s, s)
}

#[inline]
pub fn atlas_tile_uv_rect(index: usize) -> [f32; 4] {
    let (x, y, s, _) = atlas_tile_viewport_px(index);
    let inv = 1.0 / PUNCTUAL_SHADOW_ATLAS_SIZE as f32;
    [x * inv, y * inv, s * inv, s * inv]
}

/// Orthographic shadow camera from a candle toward the table center.
pub fn candle_shadow_view_proj(light_world: Vec3, camera_h: f32, env_height_scale: f32) -> Mat4 {
    let extent = camera_h * env_height_scale;
    let half = extent * 0.62;
    let depth = extent * 1.45;
    let target = Vec3::ZERO;
    let eye = light_world;
    let dir = (target - eye).normalize_or_zero();
    let up = if dir.z.abs() > 0.92 {
        Vec3::Y
    } else {
        Vec3::Z
    };
    let view = Mat4::look_at_rh(eye, target, up);
    let proj = Mat4::orthographic_rh(-half, half, -half, half, 0.1, depth);
    proj * view
}

/// Candle punctuals from `gameplay.glb` in the same order as embedded point lights.
pub fn gameplay_candle_punctual_shadow_setup(
    camera_h: f32,
    env_height_scale: f32,
) -> Vec<PunctualShadowLightSetup> {
    with_gameplay_glb_cpu(|opt| {
        let Some(cpu) = opt else {
            return Vec::new();
        };
        let s = room_env_world_scale(camera_h, env_height_scale);
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        cpu.embedded_point_lights
            .iter()
            .filter(|l| l.is_candle)
            .take(MAX_PUNCTUAL_SHADOW_LIGHTS)
            .enumerate()
            .map(|(i, l)| {
                let light_world = (l.pos_doc - center_doc) * s;
                PunctualShadowLightSetup {
                    light_view_proj: candle_shadow_view_proj(
                        light_world,
                        camera_h,
                        env_height_scale,
                    ),
                    atlas_rect: atlas_tile_uv_rect(i),
                }
            })
            .collect()
    })
}
