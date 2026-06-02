//! Persisted visual/audio settings mirrored from the options screen.
//! Grouped so they can be synced in one go from `OptionsScene` state.

pub struct RenderSettings {
    pub effects_quality: crate::persistence::EffectsQuality,
    pub tile_preset: crate::persistence::TilePreset,
    pub tile_material: crate::persistence::TileMaterial,
    pub tileset_name: String,
    pub gamma: f32,
    pub graphics_mode: mahjuro_gfx_types::GraphicsMode,
    pub hdr_enabled: bool,
    /// Master toggle for the per-scene VHS overlay (sourced from the
    /// Options scene). When `false`, every per-scene VHS amount in
    /// [`crate::game::tonemap_tuning::TonemapTuning`] is ignored at the
    /// shader level.
    pub vhs_enabled: bool,
}

impl RenderSettings {
    #[inline]
    pub fn shadow_quality(&self) -> mahjuro_gfx_types::ShadowQuality {
        self.graphics_mode.shadow_quality()
    }

    #[inline]
    pub fn ssr_enabled(&self) -> bool {
        self.graphics_mode.ssr_enabled()
    }
}
