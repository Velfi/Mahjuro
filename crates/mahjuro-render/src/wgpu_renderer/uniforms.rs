#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Globals {
    pub screen: [f32; 2],
    pub time: f32,
    pub gamma: f32,
    pub cursor_pos: [f32; 2],
    pub transition_progress: f32,
    pub quality_level: f32,
    pub moon_phase: f32,
    /// `[0]` cascade quality; `[1]` main-menu pride rainbow; `[2]` moonlit-water disc strength (0 = hidden).
    pub _globals_pad: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct BloomParams {
    pub data0: [f32; 4],
    pub data1: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct TonemapParams {
    /// Linear HDR multiplier applied before ACES.
    pub exposure: f32,
    /// 0 = ACES fitted (SDR swapchain); 1 = linear × exposure (HDR swapchain);
    /// 2 = linear × exposure, no VHS (journal prepass float target).
    pub mode: f32,
    /// 1.0 = VHS branch enabled; 0.0 = clean tonemap. Independent of the
    /// per-component amounts so the Options toggle can hard-gate everything
    /// without zeroing the per-scene values the player tuned.
    pub vhs_enabled: f32,
    /// Seconds since renderer start. Drives the VHS noise animation; when
    /// `vhs_enabled = 0` the shader short-circuits before reading it.
    pub time: f32,
    /// Chromatic-aberration UV split (R + B channels).
    pub vhs_chromatic: f32,
    /// Peak scanline darkening (0..~0.2 sensible).
    pub vhs_scanline: f32,
    /// Animated grain amplitude (0..~0.1 sensible).
    pub vhs_grain: f32,
    /// Vignette corner darkening (0..~0.4 sensible).
    pub vhs_vignette: f32,
    /// Monotonic frame counter for in-place VHS grain re-roll (not spatial scroll).
    pub grain_frame: f32,
    /// User display-gamma slider (`AppSettings::gamma`, default 1.0). Applied as
    /// `pow(color, 1/gamma)` at the end of the composite so the HDR scene path
    /// respects it — the per-shader gamma in `lit_mesh`/`tile_3d` is a no-op on
    /// the HDR path, which previously left only the UI honoring the slider.
    pub gamma: f32,
}

/// Shared by `emissive_probe_update.wgsl` and `emissive_probe_apply.wgsl` (must match WGSL layout).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ProbeGiFrameUniform {
    pub inv_view_proj: [f32; 16],
    pub view_proj: [f32; 16],
    pub world_min: [f32; 4],
    pub world_max: [f32; 4],
    /// x = nx, y = ny, z = nz, w = probe_count (nx×ny×nz).
    pub grid_dims: [u32; 4],
    /// xy = full-res width/height; z = max march distance (world); w = indirect strength scale.
    pub screen_march: [f32; 4],
    pub cam_pos: [f32; 4],
    /// x = sphere direction samples, y = march steps, zw unused.
    pub sample_params: [u32; 4],
}

/// Per-frame camera + tonemap data for `tile_3d.wgsl` group 0 binding 0.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct TileFrameUniform {
    pub view_proj: [f32; 16],
    pub cam_pos: [f32; 3],
    pub _pad0: f32,
    /// x = ACES HDR path on; y = linear exposure; z = hemispheric ambient; w = inv doc scale.
    pub tile_post_params: [f32; 4],
    /// x = embedded glTF inverse-square intensity scale for tiles.
    pub tile_punctual_params: [f32; 4],
}

/// One instanced showcase / glTF-prop tile for `tile_3d.wgsl` vertex buffer slot 1.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Tile3dInstance {
    pub model: [f32; 16],
    pub tile_visual_params: [f32; 4],
    pub tile_decal_atlas_uv: [f32; 4],
    pub tile_material_seed: f32,
    pub tile_opacity: f32,
    pub _pad: [f32; 2],
}

/// One shadow-caster instance (model only) for instanced tile outline draws.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct TileShadowInstance {
    pub model: [f32; 16],
}

/// Room-environment variant of [`TileUniform`] for `room_glb.wgsl`.
///
/// Field names are room-specific to avoid mixing tile semantics when writing shop/hallway/archive/main-menu
/// glTF environment uniforms. Layout intentionally matches WGSL `RoomEnvUniform`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct RoomEnvUniform {
    pub view_proj: [f32; 16],
    pub model: [f32; 16],
    /// x/y/w preserved for layout parity with tile path; y = room unlit debug.
    pub room_debug_params: [f32; 4],
    pub cam_pos: [f32; 3],
    /// Shared room linear HDR exposure gain before tonemap (`room_glb.wgsl`).
    pub room_linear_exposure: f32,
    /// x = ambient scale; y = inverse doc scale; z = emissive scale; w = moon phase.
    pub room_env_params: [f32; 4],
    /// Room post/tone params (`hdr_tonemap` parity); w = main-menu rainbow scene time.
    pub room_post_params: [f32; 4],
    /// x = floor z; y = height in world units; z = density per world unit; w = reserved.
    pub room_height_fog_params: [f32; 4],
    /// xyz = base linear HDR fog target color; w = tint gradient start distance in world units.
    pub room_height_fog_color: [f32; 4],
    /// xyz = distance tint color; w = tint gradient exponential scale in world units.
    pub room_height_fog_far_color: [f32; 4],
}

/// Per-frame data for `tile_outline.wgsl` group 0 binding 0.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct TileOutlineFrameUniform {
    pub view_proj: [f32; 16],
    pub hdr_tonemap: [f32; 4],
}

/// One outlined tile instance for `tile_outline.wgsl` storage buffer (binding 1).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct TileOutlineInstance {
    pub model: [f32; 16],
    pub base_color_factor: [f32; 4],
}

/// View + shader tuning consumed by the 3D flame pipeline. Mirrors
/// `FlameView` in `shaders/flame.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct FlameViewUniform {
    pub view_proj: [f32; 16],
    pub view_pos: [f32; 4],
    pub tuning: [f32; 8],
    pub _pad: [f32; 4],
}
