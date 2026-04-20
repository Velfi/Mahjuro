//! Tunable parameters for the shop scene's back-wall smoke curtain.
//!
//! The curtain is a row of wind impulses that seed density into the
//! volumetric fluid sim. These values control how thick and how active
//! the curtain reads; live-edited via the debug menu's "Shop Smoke..."
//! overlay.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ShopSmokeTuning {
    /// Number of wind emitters spread along the curtain row.
    pub emitter_count: u32,
    /// Base density injected per gust (before roll/billow modulation).
    pub density_base: f32,
    /// Amplitude of the roll-sine density modulation.
    pub density_roll_amp: f32,
    /// Amplitude of the billow-sine density modulation.
    pub density_billow_amp: f32,
    /// Gust radius as a fraction of scene height (base).
    pub radius_base: f32,
    /// Radius billow-amp (as a fraction of scene height).
    pub radius_billow_amp: f32,
    /// Forward (toward-camera) velocity baseline.
    pub forward_velocity_base: f32,
    /// Forward velocity breathe-amp.
    pub forward_velocity_breathe_amp: f32,
    /// Curtain vertical lift as a fraction of scene height.
    pub lift_fraction: f32,
}

impl ShopSmokeTuning {
    /// Values that match the pre-debug-modal hardcoded constants in
    /// `scenes/shop.rs` (the "too dense" baseline the user wants to tune
    /// down).
    pub const fn legacy_default() -> Self {
        Self {
            emitter_count: 9,
            density_base: 0.11,
            density_roll_amp: 0.045,
            density_billow_amp: 0.025,
            radius_base: 0.14,
            radius_billow_amp: 0.03,
            forward_velocity_base: 10.0,
            forward_velocity_breathe_amp: 14.0,
            lift_fraction: 0.55,
        }
    }
}

impl Default for ShopSmokeTuning {
    fn default() -> Self {
        Self::legacy_default()
    }
}
