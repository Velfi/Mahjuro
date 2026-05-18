//! Per-scene look bundle: post-process tonemap/VHS + room GLB punctual lighting.
//!
//! Persisted in `tuning_overrides.json` under `SceneLookTuning:<scene_key>`.
//! Loads legacy `TonemapTuning:<scene_key>` entries when no unified key exists.

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

use crate::game::tonemap_tuning::{
    self, TonemapTuning, FALLBACK_SCENE_KEY, KNOWN_SCENE_KEYS,
};
use crate::render::room_glb::{RoomEnvLightingTune, SHOP_ENV_HEIGHT_SCALE};

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

/// Slider rows for [`SceneLookDebugOverlay`](crate::debug_overlays::SceneLookDebugOverlay).
/// Order: tonemap (6) then room / height (9). Must match [`scene_look_row_value`] /
/// [`scene_look_row_set`].
pub const SCENE_LOOK_SLIDER_META: &[(&str, f32, f32, f32)] = &[
    ("Exposure (post)", 0.25, 2.50, 0.01),
    ("VHS Chromatic", 0.0, 0.005, 0.0001),
    ("VHS Scanline", 0.0, 0.20, 0.005),
    ("VHS Grain", 0.0, 0.10, 0.002),
    ("VHS Vignette", 0.0, 0.40, 0.005),
    ("Film Grain", 0.0, 0.12, 0.002),
    ("Room height scale", 0.001, 40.0, 0.005),
    ("glTF light intensity", 0.0, 40.0, 0.0025),
    ("Room linear exposure", 0.001, 40.0, 0.0025),
    ("Room ambient", 0.0, 10.0, 0.0025),
    ("Lit-mesh glTF scale", 0.0, 20.0, 0.005),
    ("glTF emissive scale", 0.1, 48.0, 0.05),
    ("Candle tint R", 0.0, 15.0, 0.0025),
    ("Candle tint G", 0.0, 15.0, 0.0025),
    ("Candle tint B", 0.0, 15.0, 0.0025),
];

pub const SCENE_LOOK_SLIDER_COUNT: usize = 15;

pub fn scene_look_row_value(look: &SceneLookTuning, row: usize) -> f32 {
    match row {
        0..=5 => look.tonemap.field_at(row),
        6 => look.room_gltf_height_scale,
        7 => look.room.gltf_light_intensity_scale,
        8 => look.room.linear_exposure,
        9 => look.room.ambient_scale,
        10 => look.room.lit_mesh_gltf_punctual_scale,
        11 => look.room.gltf_emissive_scale,
        12 => look.room.candle_light_color_mul[0],
        13 => look.room.candle_light_color_mul[1],
        14 => look.room.candle_light_color_mul[2],
        _ => 0.0,
    }
}

pub fn scene_look_row_set(look: &mut SceneLookTuning, row: usize, v: f32) {
    let (_, lo, hi, _) = SCENE_LOOK_SLIDER_META[row.min(SCENE_LOOK_SLIDER_COUNT - 1)];
    let v = v.clamp(lo, hi);
    match row {
        0..=5 => look.tonemap.set_field_at(row, v),
        6 => look.room_gltf_height_scale = v,
        7 => look.room.gltf_light_intensity_scale = v,
        8 => look.room.linear_exposure = v,
        9 => look.room.ambient_scale = v,
        10 => look.room.lit_mesh_gltf_punctual_scale = v,
        11 => look.room.gltf_emissive_scale = v,
        12 => look.room.candle_light_color_mul[0] = v,
        13 => look.room.candle_light_color_mul[1] = v,
        14 => look.room.candle_light_color_mul[2] = v,
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
        let default_look = load_scene_look(FALLBACK_SCENE_KEY);
        let mut per_scene = FxHashMap::default();
        for &key in KNOWN_SCENE_KEYS {
            let storage = storage_key(key);
            if crate::persistence::has_tuning_override(&storage) {
                per_scene.insert(key.to_string(), load_scene_look(key));
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

pub fn storage_key(scene_key: &str) -> String {
    format!("SceneLookTuning:{scene_key}")
}

fn load_scene_look(scene_key: &str) -> SceneLookTuning {
    let unified = storage_key(scene_key);
    if crate::persistence::has_tuning_override(&unified) {
        return crate::persistence::load_tuning_override(&unified);
    }
    SceneLookTuning {
        tonemap: crate::persistence::load_tuning_override::<TonemapTuning>(&tonemap_tuning::storage_key(
            scene_key,
        )),
        ..SceneLookTuning::default()
    }
}

pub fn save_scene_look(scene_key: &str, look: &SceneLookTuning) -> anyhow::Result<()> {
    crate::persistence::save_tuning_override(&storage_key(scene_key), look)
}

pub fn clear_scene_look(scene_key: &str) -> anyhow::Result<()> {
    crate::persistence::clear_tuning_override(&storage_key(scene_key))
}

/// All scene keys available in the overlay scene picker (plus `_default`).
pub const OVERLAY_SCENE_KEYS: &[&str] = &[
    FALLBACK_SCENE_KEY,
    KNOWN_SCENE_KEYS[0],
    KNOWN_SCENE_KEYS[1],
    KNOWN_SCENE_KEYS[2],
    KNOWN_SCENE_KEYS[3],
    KNOWN_SCENE_KEYS[4],
    KNOWN_SCENE_KEYS[5],
    KNOWN_SCENE_KEYS[6],
    KNOWN_SCENE_KEYS[7],
];

pub fn overlay_scene_keys() -> &'static [&'static str] {
    OVERLAY_SCENE_KEYS
}
