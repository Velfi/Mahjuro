#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuInstance {
    pub rect: [f32; 4],
    pub color: [f32; 4],
    /// Text quad effects: low byte = [`crate::render::text_effect::TextEffectId`].
    /// Solid colour quads keep `0`.
    pub user: u32,
}

/// Per-frame settings threaded from the app into `WgpuRenderer::render`:
/// quality tiers, tile-look choices, animation settle speeds, gamma, and
/// the shadow/SSR toggles. Grouped so the render entry point takes one
/// value instead of ten individual params.
#[derive(Clone)]
pub struct RenderSettings {
    pub effects_quality: crate::persistence::EffectsQuality,
    pub tile_preset: crate::persistence::TilePreset,
    pub tile_material: crate::persistence::TileMaterial,
    /// Which procedural surface the table mesh routes through (walnut wood
    /// or green felt). Matches the user's Options-scene choice.
    pub surface_kind: crate::persistence::SurfaceKind,
    /// Subdirectory of `assets/textures/tile_sets/` whose PNGs should be used for tile faces.
    pub tileset_name: String,
    pub draw_settle_speed: f32,
    pub sort_settle_speed: f32,
    pub gamma: f32,
    pub shadows_enabled: bool,
    pub ssr_enabled: bool,
    /// Master Options-toggle gate for the VHS overlay. The per-effect
    /// amounts (chromatic / scanline / grain / vignette) live on the
    /// renderer itself — pushed via `set_tonemap_tuning` from the per-scene
    /// resolver — so this field is a hard kill switch only.
    pub vhs_enabled: bool,
}

/// Instance for the `gradient_quad_pipeline` — same `rect`/`color` payload
/// as `GpuInstance` plus a per-instance `feather` vec4 that drives the
/// shader's alpha falloff. See `shaders/gradient_quad.wgsl` for the exact
/// contract; `feather.x` = edge softness fraction, `feather.y` = axial↔radial
/// blend, `feather.zw` reserved.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GradientQuadInstance {
    pub rect: [f32; 4],
    pub color: [f32; 4],
    pub feather: [f32; 4],
}
