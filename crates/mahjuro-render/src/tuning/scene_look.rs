//! Per-scene look bundle: post-process tonemap/VHS + room GLB punctual lighting.
//!
//! Persisted in `tuning_overrides.json` under `SceneLookTuning:<scene_key>`.
//! Loads legacy `TonemapTuning:<scene_key>` entries when no unified key exists.

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::room_glb::{RoomEnvFrameTune, RoomEnvLightingTune, SHOP_ENV_HEIGHT_SCALE};
use crate::scene_keys;
use crate::tuning::tonemap::{FALLBACK_SCENE_KEY, KNOWN_SCENE_KEYS, TonemapTuning};

/// Full-screen look for one scene: composite tonemap + `room_glb` / `lit_mesh` lighting.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct SceneLookTuning {
    pub tonemap: TonemapTuning,
    pub room: RoomEnvLightingTune,
    pub room_gltf_height_scale: f32,
}

impl Default for SceneLookTuning {
    fn default() -> Self {
        Self {
            tonemap: TonemapTuning::default(),
            room: RoomEnvLightingTune::SOURCE_DEFAULTS,
            room_gltf_height_scale: SHOP_ENV_HEIGHT_SCALE,
        }
    }
}

/// Current shipped table-room look shared by gameplay and shop.
pub const TABLE_ROOM_SCENE_LOOK: SceneLookTuning = SceneLookTuning {
    tonemap: TonemapTuning {
        exposure: 0.7790626,
        vhs_chromatic: 0.00025,
        vhs_scanline: 0.02579747,
        vhs_grain: 0.0,
        vhs_vignette: 0.4,
    },
    room: RoomEnvLightingTune {
        gltf_light_intensity_scale: 1.0169914,
        linear_exposure: 1.225086,
        linear_exposure_base: 0.0013600638,
        ambient_scale: 0.0,
        gltf_emissive_scale: 0.3257179,
        candle_light_color_mul: [1.973646, 1.6209633, 0.29851973],
        lantern_light_color_mul: [0.9250752, 0.6045183, 0.45128056],
    },
    room_gltf_height_scale: SHOP_ENV_HEIGHT_SCALE,
};

/// Main-menu scene look promoted from persisted scene-look tuning.
pub const MAIN_MENU_SCENE_LOOK: SceneLookTuning = SceneLookTuning {
    tonemap: TonemapTuning {
        exposure: 0.7441103,
        vhs_chromatic: 0.0,
        vhs_scanline: 0.0,
        vhs_grain: 0.0,
        vhs_vignette: 0.0,
    },
    room: RoomEnvLightingTune {
        gltf_light_intensity_scale: 1.0,
        linear_exposure: 0.5001661,
        linear_exposure_base: 0.0036232292,
        ambient_scale: 0.0,
        gltf_emissive_scale: 1.0,
        candle_light_color_mul: [1.2745988, 0.7392675, 0.25491974],
        lantern_light_color_mul: [1.3522706, 1.2127368, 0.95261854],
    },
    room_gltf_height_scale: 1.0,
};

/// Slider rows for [`SceneLookDebugOverlay`](crate::debug_overlays::SceneLookDebugOverlay).
/// Order: tonemap (5) then room / height (12). Must match [`scene_look_row_value`] /
/// [`scene_look_row_set`].
pub const SCENE_LOOK_SLIDER_META: &[(&str, f32, f32, f32)] = &[
    ("Exposure (post)", 0.25, 2.50, 0.01),
    ("VHS Chromatic", 0.0, 0.005, 0.0001),
    ("VHS Scanline", 0.0, 0.20, 0.005),
    ("VHS Grain", 0.0, 0.15, 0.002),
    ("VHS Vignette", 0.0, 0.40, 0.005),
    ("Room height scale", 0.001, 40.0, 0.005),
    ("glTF light intensity", 0.0, 40.0, 0.0025),
    ("Room linear exposure", 0.001, 40.0, 0.0025),
    ("Room linear exposure base", 0.000125, 0.125, 0.000125),
    ("Room ambient", 0.0, 10.0, 0.0025),
    ("glTF emissive scale", 0.1, 48.0, 0.05),
    ("Candle hue", 0.0, 1.0, 1.0 / 360.0),
    ("Candle saturation", 0.0, 1.0, 0.01),
    ("Candle intensity", 0.0, 15.0, 0.05),
    ("Lantern hue", 0.0, 1.0, 1.0 / 360.0),
    ("Lantern saturation", 0.0, 1.0, 0.01),
    ("Lantern intensity", 0.0, 15.0, 0.05),
];

/// First row of each punctual-tint HSL group — used for color swatches in the overlay.
pub const SCENE_LOOK_CANDLE_TINT_ROW: usize = 11;
pub const SCENE_LOOK_LANTERN_TINT_ROW: usize = 14;

pub const SCENE_LOOK_SLIDER_COUNT: usize = 17;

pub fn scene_look_row_value(look: &SceneLookTuning, row: usize) -> f32 {
    match row {
        0..=4 => look.tonemap.field_at(row),
        5 => look.room_gltf_height_scale,
        6 => look.room.gltf_light_intensity_scale,
        7 => look.room.linear_exposure,
        8 => look.room.linear_exposure_base,
        9 => look.room.ambient_scale,
        10 => look.room.gltf_emissive_scale,
        11..=13 => {
            punctual_tint_mul_to_hsv(look.room.candle_light_color_mul).component_at(row - 11)
        }
        14..=16 => {
            punctual_tint_mul_to_hsv(look.room.lantern_light_color_mul).component_at(row - 14)
        }
        _ => 0.0,
    }
}

pub fn scene_look_row_set(look: &mut SceneLookTuning, row: usize, v: f32) {
    let (_, lo, hi, _) = SCENE_LOOK_SLIDER_META[row.min(SCENE_LOOK_SLIDER_COUNT - 1)];
    let v = v.clamp(lo, hi);
    match row {
        0..=4 => look.tonemap.set_field_at(row, v),
        5 => look.room_gltf_height_scale = v,
        6 => look.room.gltf_light_intensity_scale = v,
        7 => look.room.linear_exposure = v,
        8 => look.room.linear_exposure_base = v,
        9 => look.room.ambient_scale = v,
        10 => look.room.gltf_emissive_scale = v,
        11..=13 => {
            let (h, s, i) = punctual_tint_mul_to_hsv(look.room.candle_light_color_mul);
            let (h, s, i) = (h, s, i).with_component(row - 11, v);
            look.room.candle_light_color_mul = hsv_to_punctual_tint_mul(h, s, i);
        }
        14..=16 => {
            let (h, s, i) = punctual_tint_mul_to_hsv(look.room.lantern_light_color_mul);
            let (h, s, i) = (h, s, i).with_component(row - 14, v);
            look.room.lantern_light_color_mul = hsv_to_punctual_tint_mul(h, s, i);
        }
        _ => {}
    }
}

#[derive(Clone, Debug, Default)]
pub struct SceneLookTuningSet {
    pub default_look: SceneLookTuning,
    pub per_scene: FxHashMap<String, SceneLookTuning>,
}

impl SceneLookTuningSet {
    pub fn load() -> Self {
        #[cfg(feature = "debug-menu")]
        {
            Self::load_with_overrides()
        }
        #[cfg(not(feature = "debug-menu"))]
        {
            Self::load_code_values()
        }
    }

    #[cfg(not(feature = "debug-menu"))]
    fn load_code_values() -> Self {
        let default_look = shipped_scene_look(FALLBACK_SCENE_KEY).unwrap_or_default();
        let mut per_scene = FxHashMap::default();
        for &key in KNOWN_SCENE_KEYS {
            if let Some(look) = shipped_scene_look(key) {
                per_scene.insert(key.to_string(), look);
            }
        }
        Self {
            default_look,
            per_scene,
        }
    }

    #[cfg(feature = "debug-menu")]
    fn load_with_overrides() -> Self {
        let default_look = load_scene_look(FALLBACK_SCENE_KEY);
        let mut per_scene = FxHashMap::default();
        for &key in KNOWN_SCENE_KEYS {
            let storage = storage_key(key);
            if mahjuro_gfx_types::has_tuning_override(&storage) {
                per_scene.insert(key.to_string(), load_scene_look(key));
                continue;
            }
            for &legacy in scene_keys::legacy_aliases(key) {
                let legacy_storage = storage_key(legacy);
                if mahjuro_gfx_types::has_tuning_override(&legacy_storage) {
                    per_scene.insert(key.to_string(), load_scene_look(key));
                    break;
                }
            }
            if !per_scene.contains_key(key)
                && let Some(look) = shipped_scene_look(key)
            {
                per_scene.insert(key.to_string(), look);
            }
        }
        Self {
            default_look,
            per_scene,
        }
    }

    pub fn resolve(&self, scene_key: Option<&str>) -> SceneLookTuning {
        match scene_key {
            Some(k) => self.per_scene.get(k).copied().unwrap_or(self.default_look),
            None => self.default_look,
        }
    }

    pub fn has_override(&self, scene_key: Option<&str>) -> bool {
        scene_key.is_some_and(|k| self.per_scene.contains_key(k))
    }

    pub fn set(&mut self, scene_key: Option<&str>, look: SceneLookTuning) {
        match scene_key {
            Some(k) => {
                self.per_scene.insert(k.to_string(), look);
            }
            None => self.default_look = look,
        }
    }

    pub fn clear(&mut self, scene_key: Option<&str>) {
        match scene_key {
            Some(k) => {
                self.per_scene.remove(k);
            }
            None => self.default_look = SceneLookTuning::default(),
        }
    }
}

/// Scene keys for embedded GLB room environments. Each gets its own
/// [`RoomEnvFrameTune`] every frame (not the active scene's globals).
pub const GLTF_ENV_SCENE_KEYS: &[&str] = &[
    scene_keys::SHOP,
    scene_keys::HALLWAY,
    scene_keys::STAIRWAY,
    scene_keys::ARCHIVE,
    scene_keys::MAIN_MENU,
    scene_keys::GAMEPLAY,
    "tutorial",
];

/// Orthographic / oblique [`ShowcaseTileBatch`](crate::draw_cmd::DrawCmd::ShowcaseTileBatch)
/// on flat UI backdrops (no embedded room GLB). These scenes need [`RoomEnvLightingTune::DOC_TILE`]
/// so tiles stay readable under the HDR + ACES path — prefer exposure/ambient over brighter
/// point lights (specular streaks on black).
pub const DOC_TILE_SCENE_KEYS: &[&str] = &[
    "guide",
    "yaku_journal",
    "wall_ledger",
    "tile_anchor_lab",
    "tile_stress_lab",
    "material_viewer",
    "tile_select",
];

/// Append per-frame env tunes for doc-tile scenes not already covered by [`GLTF_ENV_SCENE_KEYS`].
pub fn push_doc_tile_env_frame_tunes(
    env_frame_tunes: &mut Vec<(&'static str, RoomEnvFrameTune)>,
    scene_look: &SceneLookTuningSet,
    overlay_persist_key: Option<&str>,
    overlay_look: Option<SceneLookTuning>,
    apply_room: impl Fn(RoomEnvLightingTune) -> RoomEnvLightingTune,
) {
    for &key in DOC_TILE_SCENE_KEYS {
        if env_frame_tunes.iter().any(|(k, _)| *k == key) {
            continue;
        }
        let look = resolve_scene_look_with_overlay(
            scene_look,
            overlay_persist_key,
            overlay_look,
            Some(key),
        );
        let room = if scene_look.has_override(Some(key)) {
            apply_room(look.room)
        } else {
            apply_room(RoomEnvLightingTune::DOC_TILE)
        };
        env_frame_tunes.push((key, room_env_frame_from_scene_look(&look, room)));
    }
}

/// Build per-frame room env tuning from a resolved scene look bundle.
pub fn room_env_frame_from_scene_look(
    look: &SceneLookTuning,
    room: RoomEnvLightingTune,
) -> RoomEnvFrameTune {
    RoomEnvFrameTune::from_room_and_height(room, look.room_gltf_height_scale)
}

/// Resolve look for `scene_key`, applying the scene-look debug overlay when it
/// targets that bucket (or `_default` when the scene has no override).
pub fn resolve_scene_look_with_overlay(
    set: &SceneLookTuningSet,
    overlay_persist_key: Option<&str>,
    overlay_look: Option<SceneLookTuning>,
    scene_key: Option<&str>,
) -> SceneLookTuning {
    if let (Some(overlay_key), Some(look)) = (overlay_persist_key, overlay_look) {
        let editing_this = match scene_key {
            None => overlay_key == FALLBACK_SCENE_KEY,
            Some(k) => overlay_key == k,
        };
        if editing_this {
            return look;
        }
        if overlay_key == FALLBACK_SCENE_KEY {
            if let Some(k) = scene_key {
                if !set.has_override(Some(k)) {
                    return look;
                }
            } else {
                return look;
            }
        }
    }
    set.resolve(scene_key)
}

pub fn storage_key(scene_key: &str) -> String {
    format!("SceneLookTuning:{scene_key}")
}

#[cfg(feature = "debug-menu")]
fn load_scene_look(scene_key: &str) -> SceneLookTuning {
    let unified = storage_key(scene_key);
    if mahjuro_gfx_types::has_tuning_override(&unified) {
        return mahjuro_gfx_types::load_tuning_override(&unified);
    }
    for &legacy in crate::scene_keys::legacy_aliases(scene_key) {
        let legacy_unified = storage_key(legacy);
        if mahjuro_gfx_types::has_tuning_override(&legacy_unified) {
            return mahjuro_gfx_types::load_tuning_override(&legacy_unified);
        }
    }
    let mut look = shipped_scene_look(scene_key).unwrap_or_default();
    look.tonemap = load_tonemap_with_legacy(scene_key);
    look
}

fn shipped_scene_look(scene_key: &str) -> Option<SceneLookTuning> {
    match scene_key {
        scene_keys::MAIN_MENU => Some(MAIN_MENU_SCENE_LOOK),
        scene_keys::GAMEPLAY | scene_keys::SHOP => Some(TABLE_ROOM_SCENE_LOOK),
        _ => None,
    }
}

#[cfg(feature = "debug-menu")]
fn load_tonemap_with_legacy(scene_key: &str) -> TonemapTuning {
    let key = crate::tuning::tonemap::storage_key(scene_key);
    if mahjuro_gfx_types::has_tuning_override(&key) {
        return mahjuro_gfx_types::load_tuning_override(&key);
    }
    for &legacy in crate::scene_keys::legacy_aliases(scene_key) {
        let legacy_key = crate::tuning::tonemap::storage_key(legacy);
        if mahjuro_gfx_types::has_tuning_override(&legacy_key) {
            return mahjuro_gfx_types::load_tuning_override(&legacy_key);
        }
    }
    mahjuro_gfx_types::load_tuning_override(&key)
}

pub fn save_scene_look(scene_key: &str, look: &SceneLookTuning) -> anyhow::Result<()> {
    mahjuro_gfx_types::save_tuning_override(&storage_key(scene_key), look)
}

pub fn clear_scene_look(scene_key: &str) -> anyhow::Result<()> {
    mahjuro_gfx_types::clear_tuning_override(&storage_key(scene_key))
}

/// All scene keys available in the overlay scene picker (plus `_default`).
pub const OVERLAY_SCENE_KEYS: &[&str] = &[
    FALLBACK_SCENE_KEY,
    scene_keys::MAIN_MENU,
    scene_keys::SHOP,
    scene_keys::HALLWAY,
    scene_keys::GAMEPLAY,
    scene_keys::ARCHIVE,
    scene_keys::OPTIONS,
    scene_keys::STAIRWAY,
    scene_keys::VICTORY,
    scene_keys::DEFEAT,
    "tutorial",
    "showcase",
    "tile_pack_celebration",
    "guide",
    "yaku_journal",
];

pub fn overlay_scene_keys() -> &'static [&'static str] {
    OVERLAY_SCENE_KEYS
}

pub fn scene_look_row_is_hue(row: usize) -> bool {
    matches!(
        row,
        SCENE_LOOK_CANDLE_TINT_ROW | SCENE_LOOK_LANTERN_TINT_ROW
    )
}

pub fn scene_look_row_is_saturation(row: usize) -> bool {
    matches!(row, 12 | 15)
}

/// Linear RGB preview (channels clamped to 0–1) for a punctual tint swatch.
pub fn punctual_tint_preview_linear(rgb_mul: [f32; 3]) -> [f32; 3] {
    let peak = rgb_mul[0].max(rgb_mul[1]).max(rgb_mul[2]);
    if peak <= 1e-8 {
        return [0.0; 3];
    }
    [
        (rgb_mul[0] / peak).clamp(0.0, 1.0),
        (rgb_mul[1] / peak).clamp(0.0, 1.0),
        (rgb_mul[2] / peak).clamp(0.0, 1.0),
    ]
}

pub fn scene_look_tint_swatch_rgb(look: &SceneLookTuning, row: usize) -> Option<[f32; 3]> {
    let rgb = match row {
        11..=13 => look.room.candle_light_color_mul,
        14..=16 => look.room.lantern_light_color_mul,
        _ => return None,
    };
    Some(punctual_tint_preview_linear(rgb))
}

/// Hue [0, 1), saturation [0, 1], intensity (max RGB channel, capped at 15).
pub fn punctual_tint_mul_to_hsv(rgb_mul: [f32; 3]) -> (f32, f32, f32) {
    let intensity = rgb_mul[0].max(rgb_mul[1]).max(rgb_mul[2]).clamp(0.0, 15.0);
    if intensity <= 1e-8 {
        return (0.0, 0.0, 0.0);
    }
    let (h, s, _) = rgb_unit_to_hsv(
        rgb_mul[0] / intensity,
        rgb_mul[1] / intensity,
        rgb_mul[2] / intensity,
    );
    (h, s, intensity)
}

/// Full-saturation linear RGB at hue `h` (for hue-wheel slider art).
pub fn hue_wheel_preview_linear(hue: f32) -> [f32; 3] {
    let (r, g, b) = hsv_to_rgb_unit(hue.fract(), 1.0, 1.0);
    [r, g, b]
}

/// Inverse of [`punctual_tint_mul_to_hsv`].
pub fn hsv_to_punctual_tint_mul(hue: f32, saturation: f32, intensity: f32) -> [f32; 3] {
    let intensity = intensity.clamp(0.0, 15.0);
    if intensity <= 1e-8 {
        return [0.0; 3];
    }
    let (r, g, b) = hsv_to_rgb_unit(hue.fract(), saturation.clamp(0.0, 1.0), 1.0);
    [r * intensity, g * intensity, b * intensity]
}

trait HslTriple {
    fn component_at(self, index: usize) -> f32;
    fn with_component(self, index: usize, v: f32) -> Self;
}

impl HslTriple for (f32, f32, f32) {
    fn component_at(self, index: usize) -> f32 {
        match index {
            0 => self.0,
            1 => self.1,
            _ => self.2,
        }
    }

    fn with_component(self, index: usize, v: f32) -> Self {
        match index {
            0 => (v.fract(), self.1, self.2),
            1 => (self.0, v.clamp(0.0, 1.0), self.2),
            _ => (self.0, self.1, v),
        }
    }
}

fn rgb_unit_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let v = r.max(g).max(b);
    let min = r.min(g).min(b);
    if v <= 1e-8 {
        return (0.0, 0.0, 0.0);
    }
    let d = v - min;
    if d <= 1e-8 {
        return (0.0, 0.0, v);
    }
    let s = d / v;
    let h = if (v - r).abs() < 1e-8 {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if (v - g).abs() < 1e-8 {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    } / 6.0;
    (h.fract(), s, v)
}

fn hsv_to_rgb_unit(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let h = (h.fract()) * 6.0;
    let i = h.floor();
    let f = h - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match i as i32 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

#[cfg(test)]
mod tests {
    use super::{hsv_to_punctual_tint_mul, punctual_tint_mul_to_hsv};
    use crate::room_glb::{SHOP_GLTF_CANDLE_LIGHT_COLOR_MUL, SHOP_GLTF_LANTERN_LIGHT_COLOR_MUL};

    #[test]
    fn punctual_tint_hsv_round_trip() {
        for rgb in [
            SHOP_GLTF_CANDLE_LIGHT_COLOR_MUL,
            SHOP_GLTF_LANTERN_LIGHT_COLOR_MUL,
        ] {
            let (h, s, i) = punctual_tint_mul_to_hsv(rgb);
            let back = hsv_to_punctual_tint_mul(h, s, i);
            for c in 0..3 {
                assert!(
                    (back[c] - rgb[c]).abs() < 0.02,
                    "channel {c}: {back:?} vs {rgb:?}"
                );
            }
        }
    }
}
