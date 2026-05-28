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
pub(crate) struct CameraUniform {
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
    /// w = tile shader inverse document scale for embedded glTF punctual attenuation.
    /// Matches `SsrGlobals.felt.yzw` on `lit_mesh` for the same frame, except `w`.
    pub hdr_tonemap: [f32; 4],
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

/// View uniform consumed by the 3D flame pipeline. Mirrors
/// `FlameView` in `shaders/flame.wgsl`: just the matrices the
/// billboard vertex shader needs. Kept separate from `SsrGlobals`
/// because the SSR layout restricts its uniform to the fragment
/// stage, and the flame vertex shader needs `view_proj`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct FlameViewUniform {
    pub view_proj: [f32; 16],
    pub view_pos: [f32; 4],
}
