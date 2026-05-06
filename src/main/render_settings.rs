//! Persisted visual/audio settings mirrored from the options screen.
//! Grouped so they can be synced in one go from `OptionsScene` state.

pub struct RenderSettings {
    pub effects_quality: crate::persistence::EffectsQuality,
    pub tile_preset: crate::persistence::TilePreset,
    pub tile_material: crate::persistence::TileMaterial,
    pub surface_kind: crate::persistence::SurfaceKind,
    pub tileset_name: String,
    pub gamma: f32,
    pub shadows_enabled: bool,
    pub ssr_enabled: bool,
    pub hdr_enabled: bool,
    pub ui_scale: f32,
}
