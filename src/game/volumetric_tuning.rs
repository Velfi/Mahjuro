//! Tunable parameters for global volumetric effects (ambient dust, etc.).
//!
//! These values live on top of the fluid simulation: `dust_strength`
//! seeds a low, FBM-modulated density floor across the whole grid so
//! lamp light catches motes as god-ray shafts without any explicit
//! emitter. Live-edited via the debug menu's "Volumetric..." overlay
//! and can be promoted to a persistent override via `persistence`.
//!
//! The `haze_*` fields drive the procedural mountain-haze shader (a
//! screen-space FBM fog, independent of the fluid sim) — also live-edited
//! from the same overlay so art direction and volumetric motion can be
//! tuned side-by-side.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct VolumetricTuning {
    /// Ambient dust density floor written into the fluid grid every inject
    /// pass. 0.0 disables; typical useful range 0.002..0.03.
    pub dust_strength: f32,

    /// Overall density multiplier for the procedural mountain haze shader.
    /// 0.0 disables the haze; 1.0 is the default look.
    pub haze_density: f32,
    /// Haze colour in linear RGB (before additive gamma-correction in the
    /// shader). Kept as three sliders rather than a packed vec3 so each
    /// channel can be tweaked independently from the overlay.
    pub haze_color_r: f32,
    pub haze_color_g: f32,
    pub haze_color_b: f32,
    /// Vertical position (0=top, 1=bottom of screen) where the haze band
    /// is thickest — use 0.55 for a typical horizon read.
    pub haze_horizon_y: f32,
    /// Wind-drift speed multiplier. 0 freezes the noise; 1.0 default.
    pub haze_drift_speed: f32,
}

impl VolumetricTuning {
    pub const fn legacy_default() -> Self {
        Self {
            dust_strength: 0.0,
            haze_density: 1.0,
            haze_color_r: 0.080,
            haze_color_g: 0.105,
            haze_color_b: 0.145,
            haze_horizon_y: 0.55,
            haze_drift_speed: 1.0,
        }
    }
}

impl Default for VolumetricTuning {
    fn default() -> Self {
        Self::legacy_default()
    }
}
