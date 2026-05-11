#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct Globals {
    pub screen: [f32; 2],
    pub time: f32,
    pub gamma: f32,
    pub cursor_pos: [f32; 2],
    pub transition_progress: f32,
    pub quality_level: f32,
    pub moon_phase: f32,
    pub _globals_pad: [f32; 3],
}

/// Per-frame art-direction knobs for the procedural mountain-haze shader.
/// `density = 0` turns the haze off; see
/// [`crate::game::volumetric_tuning::VolumetricTuning`] for the slider
/// ranges that drive these values from the debug overlay.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct HazeUniform {
    /// RGB haze colour (linear) + density multiplier in the alpha slot.
    pub color_density: [f32; 4],
    /// `x` = horizon y (0..1), `y` = drift-speed multiplier,
    /// `z` = fog-wall center x (0..1), `w` = wall half-width in UV (0 = full width).
    pub params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct BloomParams {
    pub data0: [f32; 4],
    pub data1: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct TonemapParams {
    pub exposure: f32,
    /// 0 = ACES fitted (SDR or HDR swapchain); 1 = linear × exposure (journal prepass float target).
    pub mode: f32,
    pub _pad: [f32; 2],
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

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct CameraUniform {
    pub view_proj: [f32; 16],
    pub model: [f32; 16],
    pub base_color_factor: [f32; 4],
    /// World-space camera position, used for fresnel/view-dependent effects in tile_3d.wgsl.
    pub cam_pos: [f32; 3],
    /// Per-tile instance seed — any finite float. Read by `tile_3d.wgsl` to
    /// offset procedural noise so every tile's tortoise-shell pattern (and
    /// future material variations) is unique. Not all materials sample it.
    pub tile_seed: f32,
    /// xy = atlas origin, zw = scale — maps face UV 0..1 into the showcase decal atlas.
    pub decal_atlas_uv: [f32; 4],
    /// x = use shop-style ACES HDR path (`1`/`0`); y = linear exposure; z = hemispheric ambient scale;
    /// w = `shop_glb` linear-HDR bloom pre-pass (`1` = output linear `hdr` RGB, skip ACES/γ).
    /// Matches `SsrGlobals.felt.yzw` on `lit_mesh` for the same frame (except `w`).
    pub hdr_tonemap: [f32; 4],
}

/// Per-frame data for `tile_outline.wgsl` group 0 binding 0.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct TileOutlineFrameUniform {
    pub view_proj: [f32; 16],
    pub hdr_tonemap: [f32; 4],
}

/// One outlined tile instance for `tile_outline.wgsl` storage buffer (binding 1).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct TileOutlineInstance {
    pub model: [f32; 16],
    pub base_color_factor: [f32; 4],
}

/// View uniform consumed by the 3D flame pipeline. Mirrors
/// `FlameView` in `shaders/flame.wgsl`: just the matrices the
/// billboard vertex shader needs. Kept separate from `SsrGlobals`
/// because the SSR layout restricts its uniform to the fragment
/// stage, and the flame vertex shader needs `view_proj`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(super) struct FlameViewUniform {
    pub view_proj: [f32; 16],
    pub view_pos: [f32; 4],
}
