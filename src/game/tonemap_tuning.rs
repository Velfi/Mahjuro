//! Per-scene tonemap + VHS overlay tuning.
//!
//! One [`TonemapTuning`] is resolved every frame from the active scene key
//! (`gameplay`, `shop`, `pick_blind`, …) and uploaded into
//! [`crate::render::wgpu_renderer::TonemapParams`]. Live-edited from the
//! Debug → Tuning → Tonemap... overlay; the overlay's Save action persists
//! the value through [`crate::persistence`] under
//! `TonemapTuning:<scene_key>` (or `TonemapTuning:_default` for the
//! fallback used by scenes without their own override).
//!
//! `vhs_*` amounts are the **absolute** values fed to the shader. A scene
//! that wants the look really cranked can push them above the conservative
//! defaults; a scene that should stay clean (e.g. `pick_blind` shrine
//! reveal) can zero individual knobs without touching the global Options
//! toggle. The Options "VHS overlay: ON/OFF" gate still wins — when it's
//! off, every per-scene VHS amount is ignored.

use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};

/// Per-scene tonemap + VHS knobs. All terms are read by
/// `shaders/tonemap_composite.wgsl`; ranges chosen so the slider in the
/// debug overlay covers "off" through "obvious effect" without going
/// outside what the shader can sensibly draw.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct TonemapTuning {
    /// Linear HDR multiplier applied before ACES. `1.0` = neutral.
    pub exposure: f32,
    /// Chromatic-aberration UV split for the R/B channels. `0.0015` ≈ 3 px
    /// at 1080p; >`0.005` reads as a deliberate "broken signal" look.
    pub vhs_chromatic: f32,
    /// Peak scanline darkening (multiplied into the final color).
    pub vhs_scanline: f32,
    /// Animated grain amplitude (added to the final color, signed).
    pub vhs_grain: f32,
    /// Maximum corner darkening from the radial vignette.
    pub vhs_vignette: f32,
    /// 70s photochemical film grain (luminance-masked, animated). Not gated on
    /// the Options VHS toggle.
    #[serde(default = "default_film_grain_for_deserialize")]
    pub film_grain: f32,
}

fn default_film_grain_for_deserialize() -> f32 {
    TonemapTuning::shipping_default().film_grain
}

impl TonemapTuning {
    /// Shipped default: clean ACES tonemap with subtle 70s film grain; VHS
    /// branch dormant unless a scene tunes chromatic / scanline / tape grain.
    pub const fn shipping_default() -> Self {
        Self {
            exposure: 1.0,
            vhs_chromatic: 0.0,
            vhs_scanline: 0.0,
            vhs_grain: 0.0,
            vhs_vignette: 0.0,
            film_grain: 0.038,
        }
    }
}

impl Default for TonemapTuning {
    fn default() -> Self {
        Self::shipping_default()
    }
}

/// Slider metadata: `(label, min, max, step)`. The order here drives
/// cursor → field mapping in the debug overlay; keep it in lockstep with
/// [`TonemapTuning::field_at`] / [`TonemapTuning::field_at_mut`].
pub const TONEMAP_SLIDER_META: &[(&str, f32, f32, f32)] = &[
    ("Exposure", 0.25, 2.50, 0.01),
    ("VHS Chromatic", 0.0, 0.005, 0.0001),
    ("VHS Scanline", 0.0, 0.20, 0.005),
    ("VHS Grain", 0.0, 0.10, 0.002),
    ("VHS Vignette", 0.0, 0.40, 0.005),
    ("Film Grain", 0.0, 0.12, 0.002),
];

impl TonemapTuning {
    pub fn field_at(&self, i: usize) -> f32 {
        match i {
            0 => self.exposure,
            1 => self.vhs_chromatic,
            2 => self.vhs_scanline,
            3 => self.vhs_grain,
            4 => self.vhs_vignette,
            5 => self.film_grain,
            _ => 0.0,
        }
    }

    pub fn set_field_at(&mut self, i: usize, value: f32) {
        let (_, min, max, _) = TONEMAP_SLIDER_META[i.min(TONEMAP_SLIDER_META.len() - 1)];
        let v = value.clamp(min, max);
        match i {
            0 => self.exposure = v,
            1 => self.vhs_chromatic = v,
            2 => self.vhs_scanline = v,
            3 => self.vhs_grain = v,
            4 => self.vhs_vignette = v,
            5 => self.film_grain = v,
            _ => {}
        }
    }
}

// ── Per-scene store ─────────────────────────────────────────────────────

/// Sentinel scene key for the "any scene" fallback. Not a real
/// `active_scene_key`; used only as the persistence-overrides key suffix.
pub const FALLBACK_SCENE_KEY: &str = "_default";

/// Every key the resolver might receive from `active_scene_key`. Listed
/// explicitly so the debug overlay can offer scene-by-scene editing
/// even when the player isn't currently in that scene (eventually) and
/// so callers loading at boot can warm the right entries.
pub const KNOWN_SCENE_KEYS: &[&str] = &[
    "gameplay",
    "shop",
    "pick_blind",
    "main_menu_exterior",
    "tutorial",
    "collection",
    "showcase",
    "tile_pack_celebration",
];

/// In-memory per-scene tunings. The `_default` slot is consulted when a
/// scene has no entry of its own (or `active_scene_key` is `None` for
/// scenes that don't register one).
#[derive(Clone, Debug, Default)]
pub struct TonemapTuningSet {
    pub default_tuning: TonemapTuning,
    pub per_scene: FxHashMap<String, TonemapTuning>,
}

impl TonemapTuningSet {
    /// Load every persisted override into a fresh set. Missing keys fall
    /// back to [`TonemapTuning::default`] so a brand-new install has a
    /// neutral baseline before the overlay touches anything.
    pub fn load() -> Self {
        let default_tuning = crate::persistence::load_tuning_override::<TonemapTuning>(
            &storage_key(FALLBACK_SCENE_KEY),
        );
        let mut per_scene = FxHashMap::default();
        for &key in KNOWN_SCENE_KEYS {
            // `load_tuning_override` returns `T::default()` when the key
            // is absent; we only want entries that actually live on disk
            // so the overlay can show "(default)" for untouched scenes.
            if crate::persistence::has_tuning_override(&storage_key(key)) {
                let t =
                    crate::persistence::load_tuning_override::<TonemapTuning>(&storage_key(key));
                per_scene.insert(key.to_string(), t);
            }
        }
        Self {
            default_tuning,
            per_scene,
        }
    }

    /// Resolve the effective tuning for `scene_key`. Falls back to the
    /// `_default` entry when the scene has no override of its own.
    pub fn resolve(&self, scene_key: Option<&str>) -> TonemapTuning {
        match scene_key {
            Some(k) => self
                .per_scene
                .get(k)
                .copied()
                .unwrap_or(self.default_tuning),
            None => self.default_tuning,
        }
    }

    /// Whether `scene_key` has its own override (vs falling back to default).
    pub fn has_override(&self, scene_key: Option<&str>) -> bool {
        match scene_key {
            Some(k) => self.per_scene.contains_key(k),
            None => false,
        }
    }

    /// Update the in-memory entry for `scene_key`. Does not persist.
    pub fn set(&mut self, scene_key: Option<&str>, tuning: TonemapTuning) {
        match scene_key {
            Some(k) => {
                self.per_scene.insert(k.to_string(), tuning);
            }
            None => self.default_tuning = tuning,
        }
    }

    /// Drop the override for `scene_key` so it falls back to default.
    pub fn clear(&mut self, scene_key: Option<&str>) {
        match scene_key {
            Some(k) => {
                self.per_scene.remove(k);
            }
            None => self.default_tuning = TonemapTuning::default(),
        }
    }
}

/// Persistence key for `scene_key` (or [`FALLBACK_SCENE_KEY`] for the
/// `None` / fallback slot).
pub fn storage_key(scene_key: &str) -> String {
    format!("TonemapTuning:{scene_key}")
}
