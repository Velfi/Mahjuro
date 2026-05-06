//! Layered visual-effect toggles for isolating GPU cost and debugging look.
//!
//! Live builds start from [`EffectLayers::BASELINE`] (heavy effects off; game-over
//! moonlit / sunlit water stays on). Turn individual fields on, or assign
//! [`EffectLayers::FULL`], to restore visuals.
//!
//! Suggested re-enable order (roughly cheapest / core-first → polish):
//! 1. `procedural_surface_quality` — felt shells + global effects-quality tier
//! 2. `shadows` — directional shadow map
//! 3. `ssr` — lacquered-table reflections
//! 4. `starfield`, `golden_dust`, `ember_drift` — fullscreen particle backdrops
//! 5. `fullscreen_water_backdrop` — game-over moonlit / sunlit water (enabled in [`BASELINE`])
//! 6. `transition_fullscreen_fx` — dramatic scene transitions
//! 7. `hdr` — HDR swapchain path (still respects Options when layers allow)

use crate::main_render_settings::RenderSettings as AppRenderSettings;
use crate::persistence::EffectsQuality;
use crate::render::wgpu_renderer::RenderSettings as WgpuRenderSettings;

#[derive(Clone, Copy, Debug)]
pub struct EffectLayers {
    pub shadows: bool,
    pub ssr: bool,
    /// Felt fluff / `EffectsQuality` tier (also scales procedural vignette layers).
    pub procedural_surface_quality: bool,
    pub starfield: bool,
    pub golden_dust: bool,
    pub ember_drift: bool,
    pub hdr: bool,
    pub transition_fullscreen_fx: bool,
    pub fullscreen_water_backdrop: bool,
}

impl EffectLayers {
    pub const BASELINE: Self = Self {
        shadows: false,
        ssr: false,
        procedural_surface_quality: false,
        starfield: false,
        golden_dust: false,
        ember_drift: false,
        hdr: false,
        transition_fullscreen_fx: false,
        fullscreen_water_backdrop: true,
    };

    pub const FULL: Self = Self {
        shadows: true,
        ssr: true,
        procedural_surface_quality: true,
        starfield: true,
        golden_dust: true,
        ember_drift: true,
        hdr: true,
        transition_fullscreen_fx: true,
        fullscreen_water_backdrop: true,
    };

    pub fn wgpu_render_settings(
        self,
        gfx: &AppRenderSettings,
        tile_preset: crate::persistence::TilePreset,
        tile_material: crate::persistence::TileMaterial,
        surface_kind: crate::persistence::SurfaceKind,
        tileset_name: String,
        draw_settle_speed: f32,
        sort_settle_speed: f32,
    ) -> WgpuRenderSettings {
        WgpuRenderSettings {
            effects_quality: if self.procedural_surface_quality {
                gfx.effects_quality
            } else {
                EffectsQuality::Off
            },
            tile_preset,
            tile_material,
            surface_kind,
            tileset_name,
            draw_settle_speed,
            sort_settle_speed,
            gamma: gfx.gamma,
            shadows_enabled: gfx.shadows_enabled && self.shadows,
            ssr_enabled: gfx.ssr_enabled && self.ssr,
        }
    }

    pub fn hdr_enabled(self, gfx: &AppRenderSettings) -> bool {
        gfx.hdr_enabled && self.hdr
    }
}
