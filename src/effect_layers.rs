//! Layered visual-effect toggles for isolating GPU cost and debugging look.
//!
//! Live builds start from [`EffectLayers::BASELINE`] (directional shadows + table
//! SSR on; heavier effects off; game-over moonlit / sunlit water stays on). Turn
//! individual fields on to restore heavier visuals.

use crate::main_render_settings::RenderSettings as AppRenderSettings;
use crate::persistence::{EffectsQuality, SurfaceKind, TileMaterial, TilePreset};
use crate::render::wgpu_renderer::RenderSettings as WgpuRenderSettings;

/// Inputs for [`EffectLayers::wgpu_render_settings`].
pub struct WgpuRenderSettingsParams<'a> {
    pub gfx: &'a AppRenderSettings,
    pub tile_preset: TilePreset,
    pub tile_material: TileMaterial,
    pub surface_kind: SurfaceKind,
    pub tileset_name: String,
    pub draw_settle_speed: f32,
    pub sort_settle_speed: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct EffectLayers {
    pub shadows: bool,
    pub ssr: bool,
    /// Felt fluff / `EffectsQuality` tier (also scales procedural vignette layers).
    pub procedural_surface_quality: bool,
    pub starfield: bool,
    /// Procedural rain vignette + CPU rain field (main menu exterior).
    pub rain: bool,
    pub golden_dust: bool,
    #[allow(dead_code)]
    pub ember_drift: bool,
    pub hdr: bool,
    pub transition_fullscreen_fx: bool,
    pub fullscreen_water_backdrop: bool,
}

impl EffectLayers {
    pub const BASELINE: Self = Self {
        shadows: true,
        ssr: true,
        procedural_surface_quality: false,
        starfield: false,
        rain: false,
        golden_dust: false,
        ember_drift: false,
        hdr: false,
        transition_fullscreen_fx: false,
        fullscreen_water_backdrop: true,
    };

    pub fn wgpu_render_settings(self, p: &WgpuRenderSettingsParams<'_>) -> WgpuRenderSettings {
        WgpuRenderSettings {
            effects_quality: if self.procedural_surface_quality {
                p.gfx.effects_quality
            } else {
                EffectsQuality::Off
            },
            tile_preset: p.tile_preset,
            tile_material: p.tile_material,
            surface_kind: p.surface_kind,
            tileset_name: p.tileset_name.clone(),
            draw_settle_speed: p.draw_settle_speed,
            sort_settle_speed: p.sort_settle_speed,
            gamma: p.gfx.gamma,
            shadows_enabled: p.gfx.shadows_enabled && self.shadows,
            ssr_enabled: p.gfx.ssr_enabled && self.ssr,
            vhs_enabled: p.gfx.vhs_enabled,
        }
    }

    pub fn hdr_enabled(self, gfx: &AppRenderSettings) -> bool {
        gfx.hdr_enabled && self.hdr
    }
}
