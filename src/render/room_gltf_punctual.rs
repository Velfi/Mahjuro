//! Runtime [`KHR_lights_punctual`] → [`PointLight`] / [`SpotLight`] for room GLBs.
//!
//! Shared by shop, hallway, archive, and main menu. Room-specific behavior is selected via
//! [`RoomPunctualProfile`] (shop candle flicker, main-menu black-body node tints).

use crate::render::blackbody;
use crate::render::room_env_gltf::{RoomGltfEmbeddedPointLight, RoomGltfEmbeddedSpotLight};
use crate::render::room_glb::{
    RoomEnvLightingTune, RoomGlbCpu, glb_punctual_range_world_upload, room_env_world_scale,
};
use crate::render::wgpu_renderer::{MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS, PointLight, SpotLight};
use crate::render::world_space::surface_anchor_from_world_xyz;

/// Per-room punctual build behavior (color / intensity only — positions come from glTF).
#[derive(Clone, Copy, Debug)]
pub enum RoomPunctualProfile {
    Standard,
    /// `light_candle*`: glTF white × [`RoomEnvLightingTune::candle_light_color_mul`] + flicker.
    ShopCandles {
        flame_time_s: f32,
        lamp_flicker: f32,
    },
    /// `moonlight` / `door_light`: Kelvin tints when glTF `color` is unit white.
    MainMenu,
}

#[inline]
pub fn room_glb_has_embedded_lights(cpu: &RoomGlbCpu) -> bool {
    !cpu.embedded_point_lights.is_empty() || !cpu.embedded_spot_lights.is_empty()
}

/// Linear RGB for embedded glTF punctuals. `light_candle*` / `light_lantern*` nodes multiply glTF
/// color by [`RoomEnvLightingTune::candle_light_color_mul`] /
/// [`RoomEnvLightingTune::lantern_light_color_mul`].
#[inline]
pub fn gltf_punctual_linear_rgb(
    raw: [f32; 3],
    is_candle: bool,
    is_lantern: bool,
    tune: &RoomEnvLightingTune,
) -> [f32; 3] {
    let mul = if is_candle {
        tune.candle_light_color_mul
    } else if is_lantern {
        tune.lantern_light_color_mul
    } else {
        return raw;
    };
    [
        (raw[0] * mul[0]).clamp(0.0, 1.0),
        (raw[1] * mul[1]).clamp(0.0, 1.0),
        (raw[2] * mul[2]).clamp(0.0, 1.0),
    ]
}

const MAIN_MENU_MOONLIGHT_TEMP_K: f32 = 5500.0;
const MAIN_MENU_DOOR_LIGHT_TEMP_K: f32 = 2500.0;

#[inline]
fn is_near_unit_white(rgb: [f32; 3]) -> bool {
    rgb[0] > 0.98 && rgb[1] > 0.98 && rgb[2] > 0.98
}

fn main_menu_point_color(l: &RoomGltfEmbeddedPointLight, tune: &RoomEnvLightingTune) -> [f32; 3] {
    let base = gltf_punctual_linear_rgb(l.color_linear, l.is_candle, l.is_lantern, tune);
    if !is_near_unit_white(base) {
        return base;
    }
    match l.node_name.as_str() {
        "moonlight" => blackbody::blackbody_rgb_linear(MAIN_MENU_MOONLIGHT_TEMP_K),
        "door_light" => blackbody::blackbody_rgb_linear(MAIN_MENU_DOOR_LIGHT_TEMP_K),
        _ => base,
    }
}

fn point_color(
    profile: RoomPunctualProfile,
    l: &RoomGltfEmbeddedPointLight,
    tune: &RoomEnvLightingTune,
) -> [f32; 3] {
    match profile {
        RoomPunctualProfile::MainMenu => main_menu_point_color(l, tune),
        _ => gltf_punctual_linear_rgb(l.color_linear, l.is_candle, l.is_lantern, tune),
    }
}

fn point_intensity(
    profile: RoomPunctualProfile,
    l: &RoomGltfEmbeddedPointLight,
    tune: &RoomEnvLightingTune,
    candle_index: &mut u32,
) -> f32 {
    let mut intensity = (l.intensity * tune.gltf_light_intensity_scale).max(0.0);
    if let RoomPunctualProfile::ShopCandles {
        flame_time_s,
        lamp_flicker,
    } = profile
        && l.is_candle
    {
        let seed = candle_index
            .wrapping_mul(0x9E37_79B9)
            .wrapping_add(0xA5A5_A5A5);
        *candle_index += 1;
        let phase = (seed as f32 * 2.328_306e-10).fract();
        intensity *= lamp_flicker;
        intensity *=
            crate::render::flame_volume::shop_candle_flicker_multiplier(phase, flame_time_s);
    }
    intensity
}

/// Build point lights from decoded [`RoomGlbCpu`] punctual data.
pub fn embedded_point_lights_runtime(
    cpu: &RoomGlbCpu,
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
    profile: RoomPunctualProfile,
    asset_label: &'static str,
) -> Vec<PointLight> {
    if cpu.embedded_point_lights.is_empty() {
        return Vec::new();
    }
    let s = room_env_world_scale(h, env_h);
    let center_doc = cpu
        .environment_bounds_doc
        .map(|b| b.center())
        .unwrap_or(glam::Vec3::ZERO);
    let budget = MAX_POINT_LIGHTS.saturating_sub(2);
    if cpu.embedded_point_lights.len() > budget {
        log::warn!(
            "{asset_label}: {} point lights exceed budget ({budget}) — truncating",
            cpu.embedded_point_lights.len(),
        );
    }
    let mut candle_index = 0u32;
    cpu.embedded_point_lights
        .iter()
        .take(budget)
        .map(|l| {
            let world = (l.pos_doc - center_doc) * s;
            let radius = glb_punctual_range_world_upload(h, s, l.range_doc);
            PointLight {
                pos: surface_anchor_from_world_xyz(w, h, world),
                radius,
                color: point_color(profile, l, tune),
                intensity: point_intensity(profile, l, tune, &mut candle_index),
            }
        })
        .collect()
}

/// Build spotlights from decoded [`RoomGlbCpu`] punctual data.
pub fn embedded_spot_lights_runtime(
    cpu: &RoomGlbCpu,
    w: f32,
    h: f32,
    env_h: f32,
    tune: &RoomEnvLightingTune,
    asset_label: &'static str,
) -> Vec<SpotLight> {
    if cpu.embedded_spot_lights.is_empty() {
        return Vec::new();
    }
    let s = room_env_world_scale(h, env_h);
    let center_doc = cpu
        .environment_bounds_doc
        .map(|b| b.center())
        .unwrap_or(glam::Vec3::ZERO);
    if cpu.embedded_spot_lights.len() > MAX_SPOT_LIGHTS {
        log::warn!(
            "{asset_label}: {} spot lights exceed {MAX_SPOT_LIGHTS} — truncating",
            cpu.embedded_spot_lights.len(),
        );
    }
    cpu.embedded_spot_lights
        .iter()
        .take(MAX_SPOT_LIGHTS)
        .filter_map(|l| spot_from_embedded(cpu, l, w, h, s, center_doc, tune))
        .collect()
}

fn spot_from_embedded(
    _cpu: &RoomGlbCpu,
    l: &RoomGltfEmbeddedSpotLight,
    w: f32,
    h: f32,
    s: f32,
    center_doc: glam::Vec3,
    tune: &RoomEnvLightingTune,
) -> Option<SpotLight> {
    let dir_w = l.dir_doc.normalize_or_zero();
    if dir_w.length_squared() < 1e-12 {
        return None;
    }
    let world = (l.pos_doc - center_doc) * s;
    let radius = glb_punctual_range_world_upload(h, s, l.range_doc);
    let cos_outer = l.outer_cone_rad.cos();
    let cos_inner = l.inner_cone_rad.cos().max(cos_outer);
    Some(SpotLight {
        pos: surface_anchor_from_world_xyz(w, h, world),
        dir: dir_w.to_array(),
        radius,
        cos_outer,
        cos_inner,
        color: gltf_punctual_linear_rgb(l.color_linear, l.is_candle, l.is_lantern, tune),
        intensity: (l.intensity * tune.gltf_light_intensity_scale).max(0.0),
    })
}
