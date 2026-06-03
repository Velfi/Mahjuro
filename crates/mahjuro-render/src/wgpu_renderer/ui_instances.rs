#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuInstance {
    pub rect: [f32; 4],
    pub color: [f32; 4],
    /// Text quad effects: low byte = [`crate::text_effect::TextEffectId`].
    /// Solid colour quads keep `0`.
    pub user: u32,
}

/// Per-frame settings threaded from the app into `WgpuRenderer::render`:
/// quality tiers, tile-look choices, animation settle speeds, gamma, and
/// the shadow/SSR toggles. Grouped so the render entry point takes one
/// value instead of ten individual params.
#[derive(Clone)]
pub struct RenderSettings {
    pub effects_quality: mahjuro_gfx_types::EffectsQuality,
    /// User Options → Effects tier for the shooting-star wipe only (see
    /// `Globals._globals_pad[0]` / `shooting_star_cascade.wgsl`).
    pub cascade_effects_quality: mahjuro_gfx_types::EffectsQuality,
    pub tile_preset: mahjuro_gfx_types::TilePreset,
    pub tile_material: mahjuro_gfx_types::TileMaterial,
    /// Subdirectory of `assets/textures/tile_sets/` whose PNGs should be used for tile faces.
    pub tileset_name: String,
    pub draw_settle_speed: f32,
    pub sort_settle_speed: f32,
    pub gamma: f32,
    pub shadow_quality: mahjuro_gfx_types::ShadowQuality,
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

/// Instance for the `arc_ring_quad_pipeline` — square bbox plus fill/track
/// colours and ring params (`inner_r_norm`, `progress`, …). See
/// `shaders/arc_ring_quad.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ArcRingQuadInstance {
    pub rect: [f32; 4],
    pub fill_color: [f32; 4],
    pub track_color: [f32; 4],
    pub params: [f32; 4],
}
