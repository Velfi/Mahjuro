//! Volumetric smoke — density-only, no fluid simulation.
//!
//! This module used to run a full Navier-Stokes + BiMocq solver. That system
//! was doing too much work, fighting its own tuning for the cursor-trail use
//! case, and losing density through long-advection BiMocq reconstructions.
//! It's been replaced by a simpler pipeline:
//!
//!   1. advect      — each voxel backtraces along `drift + curl_noise`,
//!                    bilinear-samples the previous density, applies
//!                    dissipation. See `shaders/fluid3_advect.wgsl`.
//!   2. inject      — gaussian splat of impulse points into the density
//!                    field. See `shaders/fluid3_inject.wgsl`.
//!   3. lightbake   — per-voxel candle lighting, writes `lit_density`.
//!                    See `shaders/fluid3_lightbake.wgsl` (unchanged).
//!   4. raymarch    — fullscreen volumetric composite pass, reads the
//!                    pre-lit density. See `shaders/fluid3_volume.wgsl`
//!                    and `shaders/fluid3_composite.wgsl` (unchanged).
//!
//! The 3D density texture is `Rgba16Float`: `w` = density, `xyz` = a decaying
//! velocity stash that the inject pass writes and the advect pass reads back
//! the next frame. This lets tile drags and scripted wind gusts still nudge
//! the smoke even though there's no real velocity field.

use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use crate::persistence::{SmokeAmount, SmokeQuality};

/// Pixel format used for the offscreen smoke render target. `Rgba16Float`
/// gives the volume shader headroom for HDR-style lighting accumulation
/// before the composite pass blends it onto the sRGB swap chain.
const SMOKE_OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

// ──────────────────────────────────────────────────────────────────────
// Grid configuration
// ──────────────────────────────────────────────────────────────────────

/// Horizontal extent along world X and Y (table plane).
const MAX_GRID_XY: u32 = 128;
/// Vertical resolution along world +Z.
const MAX_GRID_Z_UP: u32 = 80;
const WG: u32 = 4;

#[derive(Clone, Copy)]
struct GridDims {
    x: u32,
    y: u32,
    z: u32,
}

impl GridDims {
    const fn new(x: u32, y: u32, z: u32) -> Self {
        Self { x, y, z }
    }
}

impl From<GridDims> for Vec3 {
    fn from(value: GridDims) -> Self {
        Vec3::new(value.x as f32, value.y as f32, value.z as f32)
    }
}

fn grid_dims_for_quality(quality: SmokeQuality) -> GridDims {
    match quality {
        // Texture (x, y, z) ↔ world (X, Y, Z); vertical smoke is along Z.
        // Off keeps a tiny grid allocated so resources stay bound; the sim
        // is short-circuited elsewhere.
        SmokeQuality::Off => GridDims::new(64, 64, 32),
        SmokeQuality::Low => GridDims::new(80, 80, 40),
        SmokeQuality::Medium => GridDims::new(96, 96, 48),
        SmokeQuality::High => GridDims::new(112, 112, 64),
        SmokeQuality::Ultra => GridDims::new(MAX_GRID_XY, MAX_GRID_XY, MAX_GRID_Z_UP),
    }
}

// Per-frame impulse budget. Sized to comfortably fit the worst-case
// opening frame: a full hand of sliding tiles + the 24-cell post-deal
// wind sweep + the cursor puff, with headroom.
//
// Must stay in sync with `MAX_INJECTIONS` and the `points` array length
// in `shaders/fluid3_inject.wgsl`.
const MAX_INJECTIONS: usize = 64;

/// Max opaque occluders (bugs, etc.) that can cast shadows into the smoke
/// each frame. Must stay in sync with `MAX_OCCLUDERS` in
/// `shaders/fluid3_lightbake.wgsl`.
pub const MAX_BUG_OCCLUDERS: usize = 16;

/// Project a world-space AABB to screen space, returning
/// `(min_x, min_y, max_x, max_y)` in float pixels, or `None` if the box
/// is entirely behind the camera. Partial-behind returns the full target
/// (can't safely clip a straddling volume without a proper clipper).
fn project_world_aabb(
    view_proj: Mat4,
    mn: Vec3,
    mx: Vec3,
    target_w: u32,
    target_h: u32,
) -> Option<(f32, f32, f32, f32)> {
    use glam::Vec4;
    let corners = [
        Vec3::new(mn.x, mn.y, mn.z),
        Vec3::new(mx.x, mn.y, mn.z),
        Vec3::new(mn.x, mx.y, mn.z),
        Vec3::new(mx.x, mx.y, mn.z),
        Vec3::new(mn.x, mn.y, mx.z),
        Vec3::new(mx.x, mn.y, mx.z),
        Vec3::new(mn.x, mx.y, mx.z),
        Vec3::new(mx.x, mx.y, mx.z),
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    let mut behind = 0;
    for c in corners {
        let clip: Vec4 = view_proj * Vec4::new(c.x, c.y, c.z, 1.0);
        if clip.w <= 0.001 {
            behind += 1;
            continue;
        }
        let ndc_x = clip.x / clip.w;
        let ndc_y = clip.y / clip.w;
        let sx = (ndc_x * 0.5 + 0.5) * target_w as f32;
        let sy = (1.0 - (ndc_y * 0.5 + 0.5)) * target_h as f32;
        min_x = min_x.min(sx);
        min_y = min_y.min(sy);
        max_x = max_x.max(sx);
        max_y = max_y.max(sy);
    }
    if behind == 8 {
        return None;
    }
    if behind > 0 {
        return Some((0.0, 0.0, target_w as f32, target_h as f32));
    }
    Some((min_x, min_y, max_x, max_y))
}

fn handle_pre_step_state(
    pending_clear: &mut bool,
    impulses: &mut Vec<Impulse>,
    quality: SmokeQuality,
) -> bool {
    let clearing = *pending_clear;
    if clearing {
        *pending_clear = false;
        impulses.clear();
    }
    if !clearing && matches!(quality, SmokeQuality::Off) {
        impulses.clear();
        return false;
    }
    true
}

// ──────────────────────────────────────────────────────────────────────
// Uniform structs (std140-friendly: vec4 alignment everywhere)
// ──────────────────────────────────────────────────────────────────────

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FluidUniformsGpu {
    grid_size: [f32; 4],
    grid_min: [f32; 4],
    grid_max: [f32; 4],
    inv_extent: [f32; 4],
    /// x = dt, y = density_dissipation, z = drift_speed (+Z world u/s),
    /// w = curl_strength (world u/s amplitude)
    params: [f32; 4],
    /// x = curl_spatial_scale, y = curl_time_scale,
    /// z = stored_vel_mix (0..1 — scales the injected velocity the advect
    /// shader reads back out of xyz), w = ambient dust floor density (read
    /// by `fluid3_inject.wgsl` to seed an FBM baseline; 0 disables).
    force_params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct InjectionPointGpu {
    pos_radius: [f32; 4],
    vel_density: [f32; 4],
    /// Kept only so the shader struct layout matches what Rust writes;
    /// the new inject shader doesn't read these fields.
    temperature_phase: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct InjectionParamsGpu {
    points: [InjectionPointGpu; MAX_INJECTIONS],
    active_count: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OccluderGpu {
    /// xyz = world position, w = gaussian radius (world units).
    pos_radius: [f32; 4],
    /// x = strength (density-equivalent multiplier), yzw unused.
    params: [f32; 4],
}

/// Matches `Occluders` in `shaders/fluid3_lightbake.wgsl`: `count` first,
/// then the fixed-size array. Field order must stay in sync with WGSL.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OccludersGpu {
    /// x = active count, yzw padding.
    count: [u32; 4],
    items: [OccluderGpu; MAX_BUG_OCCLUDERS],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct VolumeCameraGpu {
    inv_view_proj: [f32; 16],
    view_proj: [f32; 16],
    /// Last frame's view-projection. Used by the volume shader to
    /// reproject this frame's first-hit world position into previous
    /// screen space for temporal reprojection (TAA).
    prev_view_proj: [f32; 16],
    cam_pos: [f32; 4],
    grid_min: [f32; 4],
    grid_max: [f32; 4],
    grid_size: [f32; 4],
    /// x=max_alpha, y=step_count (as f32), z=light_strength, w=ambient
    params: [f32; 4],
    /// x=render mode: 0=both smoke+flames, 1=smoke only, 2=flames only.
    /// y=history_valid (1.0 when the history texture contains a valid
    ///   previous frame — i.e. same size, same quality, not just cleared).
    /// z=frame_index (wraps at 256) — cycles the per-pixel jitter so TAA
    ///   accumulation converges to the true integral rather than burning
    ///   in a static pattern.
    /// w unused.
    mode: [f32; 4],
}

// ──────────────────────────────────────────────────────────────────────
// Public impulse type
// ──────────────────────────────────────────────────────────────────────

/// One world-space opaque occluder queued for the current frame. The
/// lightbake treats these as extra optical depth along each candle's
/// shadow ray, so the object darkens smoke behind it.
#[derive(Clone, Copy)]
pub struct BugOccluder {
    pub world_pos: Vec3,
    pub radius: f32,
    pub strength: f32,
}

/// One world-space impulse queued for the current frame.
#[derive(Clone, Copy)]
pub struct Impulse {
    pub world_pos: Vec3,
    pub world_vel: Vec3,
    pub radius: f32,
    pub density: f32,
    /// Accepted for API stability but ignored — the old fluid solver used
    /// this to feed a per-voxel temperature field; the simplified pipeline
    /// approximates thermal tint from height alone.
    pub temperature: f32,
    /// Accepted for API stability but ignored (was dead weight in the old
    /// solver too — the inject shader never read it).
    pub phase: f32,
}

// ──────────────────────────────────────────────────────────────────────
// FluidSim
// ──────────────────────────────────────────────────────────────────────

pub struct FluidSim {
    // Ping-pong density textures. `w` = density, `xyz` = decaying velocity
    // stash written by the inject pass and read by the advect pass next
    // frame so tile drags and scripted wind gusts still nudge smoke.
    #[allow(dead_code)]
    vd: [wgpu::Texture; 2],
    vd_view: [wgpu::TextureView; 2],

    /// Pre-lit smoke field. After the inject pass, the lightbake walks every
    /// voxel and writes `(rgb = lit smoke colour, a = density)` here. The
    /// volumetric raymarch samples this directly.
    #[allow(dead_code)]
    lit_density: wgpu::Texture,
    lit_density_view: wgpu::TextureView,

    linear_sampler: wgpu::Sampler,

    fluid_uniforms_buf: wgpu::Buffer,
    injection_buf: wgpu::Buffer,
    cam_buf: wgpu::Buffer,
    occluders_buf: wgpu::Buffer,
    pending_occluders: Vec<BugOccluder>,

    /// Tiny staging buffers for toggling the render mode via
    /// `encoder.copy_buffer_to_buffer`. Debug-only.
    #[cfg(debug_assertions)]
    mode_buf_smoke_only: wgpu::Buffer,
    #[cfg(debug_assertions)]
    mode_buf_default: wgpu::Buffer,

    // Compute pipelines.
    advect_pipeline: wgpu::ComputePipeline,
    inject_pipeline: wgpu::ComputePipeline,
    lightbake_pipeline: wgpu::ComputePipeline,
    lightbake_layout: wgpu::BindGroupLayout,
    /// Built lazily by `rebuild_render_bind_group` because it references
    /// the renderer-owned `point_lights_buffer`.
    lightbake_bg: Option<wgpu::BindGroup>,

    /// advect_bgs[0] reads vd[0] and writes vd[1]; advect_bgs[1] reverses.
    advect_bgs: [wgpu::BindGroup; 2],
    /// inject_bgs[0] reads vd[1] and writes vd[0]; inject_bgs[1] reverses.
    /// Ordering is chosen so the final density each frame lands in vd[0],
    /// where the lightbake and raymarch bind groups expect it.
    inject_bgs: [wgpu::BindGroup; 2],

    // Volume render pipeline. Renders into an offscreen Rgba16Float target
    // (NOT the swap chain) using REPLACE blending — the offscreen target is
    // cleared each frame and the shader writes premultiplied colour.
    render_pipeline: wgpu::RenderPipeline,
    render_layout: wgpu::BindGroupLayout,
    // Ping-pong render bgs. Slot i binds the OTHER slot's offscreen
    // texture as `history_tex`, so rendering into slot i reads slot
    // (1-i) as history. The bgs are rebuilt any time the offscreen
    // textures are (re)allocated (resize, quality change).
    render_bgs: [Option<wgpu::BindGroup>; 2],

    // Composite pipeline that samples the offscreen target with bilinear
    // filtering and blends it onto the swap chain with premultiplied alpha.
    composite_pipeline: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    composite_sampler: wgpu::Sampler,
    // Two composite bgs — one for each possible "current" offscreen
    // target in the ping-pong. We rebuild both during `set_detail` and
    // pick the right one at composite time based on `current_index`.
    composite_bgs: [Option<wgpu::BindGroup>; 2],

    // Ping-pong offscreen targets. Frame N writes into
    // `offscreen_texture[current_index]` while sampling the previous
    // frame's output from the other slot as its TAA history.
    offscreen_texture: [Option<wgpu::Texture>; 2],
    offscreen_view: [Option<wgpu::TextureView>; 2],
    /// Linear sampler used by the volume shader to sample the history
    /// texture in previous-frame screen space.
    history_sampler: wgpu::Sampler,
    /// Index of the offscreen slot we'll render *into* this frame.
    /// The other slot is bound as history.
    current_index: usize,
    /// Set to false whenever history would be invalid: resize, quality
    /// change, first frame after clear. Cleared on the next frame once
    /// the "previous" slot actually has fresh contents.
    history_valid: bool,
    /// Rolling frame counter (wraps at 256). Drives the per-pixel jitter
    /// so each frame sees a different offset and TAA averages them.
    frame_index: u32,
    /// Last frame's view-projection, cached for reprojection.
    prev_view_proj: Mat4,

    offscreen_w: u32,
    offscreen_h: u32,
    current_detail: Option<SmokeQuality>,

    impulses: Vec<Impulse>,

    grid_min: Vec3,
    grid_max: Vec3,
    grid_size: GridDims,

    screen_w: f32,
    screen_h: f32,

    pending_clear: bool,
    sim_time: f32,
    /// Ambient dust floor density written into `force_params.w` and read
    /// by the inject shader to seed an FBM-modulated baseline across the
    /// whole grid. 0.0 disables.
    dust_strength: f32,
    /// World-space AABB of the region that currently contains (or recently
    /// contained) smoke. `None` when the grid is empty. Drives the scissor
    /// for the offscreen raymarch so a single cursor puff doesn't pay for
    /// a full-screen ray-march across the entire grid bounds.
    ///
    /// Grown each frame by an advection margin derived from the fastest
    /// recent impulse (so velocity-driven smoke stays inside the rect).
    /// Shrinks very slowly, and only on frames with no new impulses — we
    /// don't have a density readback, so we err toward a generous box and
    /// let the dissipation-powered shrink eventually close it.
    active_world_aabb: Option<(Vec3, Vec3)>,
    /// Fastest injected velocity magnitude in the last few frames,
    /// decaying each frame. Drives per-frame growth of `active_world_aabb`
    /// so velocity-driven plumes don't outrun the scissor.
    active_aabb_max_speed: f32,
}

impl FluidSim {
    pub fn new(
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
        globals_layout: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        screen_w: f32,
        screen_h: f32,
    ) -> Self {
        // ── 3D textures ────────────────────────────────────────────────
        let extent3d = wgpu::Extent3d {
            width: MAX_GRID_XY,
            height: MAX_GRID_XY,
            depth_or_array_layers: MAX_GRID_Z_UP,
        };
        let make_3d = |label: &str, format: wgpu::TextureFormat| -> wgpu::Texture {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: extent3d,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D3,
                format,
                usage: wgpu::TextureUsages::STORAGE_BINDING
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let vd = [
            make_3d("fluid3-vd-a", wgpu::TextureFormat::Rgba16Float),
            make_3d("fluid3-vd-b", wgpu::TextureFormat::Rgba16Float),
        ];
        // Pre-lit smoke field — same dims/format so the lightbake can write
        // through `texture_storage_3d<rgba16float, write>` and the raymarch
        // can sample it filtered.
        let lit_density = make_3d("fluid3-lit-density", wgpu::TextureFormat::Rgba16Float);

        let view_desc = wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D3),
            ..Default::default()
        };
        let vd_view = [vd[0].create_view(&view_desc), vd[1].create_view(&view_desc)];
        let lit_density_view = lit_density.create_view(&view_desc);

        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fluid3-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        // ── Uniform buffers ────────────────────────────────────────────
        let default_grid = grid_dims_for_quality(SmokeQuality::High);
        let fluid_uniforms_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fluid3-uniforms"),
            contents: bytemuck::bytes_of(&FluidUniformsGpu {
                grid_size: [
                    default_grid.x as f32,
                    default_grid.y as f32,
                    default_grid.z as f32,
                    0.0,
                ],
                grid_min: [-100.0, -100.0, 0.0, 0.0],
                grid_max: [100.0, 100.0, 60.0, 0.0],
                inv_extent: [1.0 / 200.0, 1.0 / 200.0, 1.0 / 60.0, 0.0],
                params: [1.0 / 60.0, 0.995, 6.0, 4.0],
                force_params: [0.012, 0.35, 1.0, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let injection_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fluid3-injection"),
            contents: bytemuck::bytes_of(&InjectionParamsGpu {
                points: [InjectionPointGpu {
                    pos_radius: [0.0; 4],
                    vel_density: [0.0; 4],
                    temperature_phase: [0.0; 4],
                }; MAX_INJECTIONS],
                active_count: [0; 4],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let occluders_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fluid3-occluders"),
            contents: bytemuck::bytes_of(&OccludersGpu {
                count: [0; 4],
                items: [OccluderGpu {
                    pos_radius: [0.0; 4],
                    params: [0.0; 4],
                }; MAX_BUG_OCCLUDERS],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let cam_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fluid3-cam"),
            contents: bytemuck::bytes_of(&VolumeCameraGpu {
                inv_view_proj: Mat4::IDENTITY.to_cols_array(),
                view_proj: Mat4::IDENTITY.to_cols_array(),
                prev_view_proj: Mat4::IDENTITY.to_cols_array(),
                cam_pos: [0.0; 4],
                grid_min: [-100.0, -100.0, 0.0, 0.0],
                grid_max: [100.0, 100.0, 60.0, 0.0],
                grid_size: [
                    default_grid.x as f32,
                    default_grid.y as f32,
                    default_grid.z as f32,
                    0.0,
                ],
                params: [0.5, 36.0, 1.5, 0.1],
                mode: [0.0; 4],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        #[cfg(debug_assertions)]
        let mode_buf_smoke_only = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fluid3-mode-smoke-only"),
            contents: bytemuck::cast_slice(&[1.0f32, 0.0, 0.0, 0.0]),
            usage: wgpu::BufferUsages::COPY_SRC,
        });
        #[cfg(debug_assertions)]
        let mode_buf_default = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fluid3-mode-default"),
            contents: bytemuck::cast_slice(&[0.0f32, 0.0, 0.0, 0.0]),
            usage: wgpu::BufferUsages::COPY_SRC,
        });

        // ── Shader modules ─────────────────────────────────────────────
        let make_shader = |label: &str, src: &str| -> wgpu::ShaderModule {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            })
        };
        let advect_shader = make_shader(
            "fluid3-advect",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/fluid3_advect.wgsl"
            )),
        );
        let inject_shader = make_shader(
            "fluid3-inject",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/fluid3_inject.wgsl"
            )),
        );
        let lightbake_shader = make_shader(
            "fluid3-lightbake",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/fluid3_lightbake.wgsl"
            )),
        );
        let volume_shader = make_shader(
            "fluid3-volume",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/fluid3_volume.wgsl"
            )),
        );

        // ── Compute pipelines ──────────────────────────────────────────
        // Advect: uniforms, src density (filterable), sampler, dst density.
        let advect_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid3-advect-bgl"),
            entries: &[
                bgl_uniform(0),
                bgl_tex3d_float(1),
                bgl_sampler(2),
                bgl_storage3d(3, wgpu::TextureFormat::Rgba16Float),
            ],
        });
        let advect_pl_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fluid3-advect-pl"),
            bind_group_layouts: &[Some(&advect_layout)],
            immediate_size: 0,
        });
        let advect_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fluid3-advect-pipeline"),
            layout: Some(&advect_pl_layout),
            module: &advect_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // Inject: uniforms, injection uniform, src density, dst density.
        let inject_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid3-inject-bgl"),
            entries: &[
                bgl_uniform(0),
                bgl_uniform(1),
                bgl_tex3d_unfiltered(2),
                bgl_storage3d(3, wgpu::TextureFormat::Rgba16Float),
            ],
        });
        let inject_pl_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fluid3-inject-pl"),
            bind_group_layouts: &[Some(&inject_layout)],
            immediate_size: 0,
        });
        let inject_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fluid3-inject-pipeline"),
            layout: Some(&inject_pl_layout),
            module: &inject_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // Lightbake: uniforms, src density, dst lit_density, cam, lights, occluders.
        let lightbake_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid3-lightbake-bgl"),
            entries: &[
                bgl_uniform(0),
                bgl_tex3d_unfiltered(1),
                bgl_storage3d(2, wgpu::TextureFormat::Rgba16Float),
                bgl_uniform(3),
                bgl_uniform(4),
                bgl_uniform(5),
            ],
        });
        let lightbake_pl_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fluid3-lightbake-pl"),
            bind_group_layouts: &[Some(&lightbake_layout)],
            immediate_size: 0,
        });
        let lightbake_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fluid3-lightbake-pipeline"),
                layout: Some(&lightbake_pl_layout),
                module: &lightbake_shader,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });

        // Advect/inject bind groups. The flow each frame is:
        //   vd[0]  --advect-->  vd[1]  --inject-->  vd[0]
        // so the final density always lands in vd[0] for lightbake/raymarch.
        let advect_bgs = [
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fluid3-advect-bg-0-to-1"),
                layout: &advect_layout,
                entries: &[
                    bge(0, fluid_uniforms_buf.as_entire_binding()),
                    bge(1, wgpu::BindingResource::TextureView(&vd_view[0])),
                    bge(2, wgpu::BindingResource::Sampler(&linear_sampler)),
                    bge(3, wgpu::BindingResource::TextureView(&vd_view[1])),
                ],
            }),
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fluid3-advect-bg-1-to-0"),
                layout: &advect_layout,
                entries: &[
                    bge(0, fluid_uniforms_buf.as_entire_binding()),
                    bge(1, wgpu::BindingResource::TextureView(&vd_view[1])),
                    bge(2, wgpu::BindingResource::Sampler(&linear_sampler)),
                    bge(3, wgpu::BindingResource::TextureView(&vd_view[0])),
                ],
            }),
        ];
        let inject_bgs = [
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fluid3-inject-bg-1-to-0"),
                layout: &inject_layout,
                entries: &[
                    bge(0, fluid_uniforms_buf.as_entire_binding()),
                    bge(1, injection_buf.as_entire_binding()),
                    bge(2, wgpu::BindingResource::TextureView(&vd_view[1])),
                    bge(3, wgpu::BindingResource::TextureView(&vd_view[0])),
                ],
            }),
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fluid3-inject-bg-0-to-1"),
                layout: &inject_layout,
                entries: &[
                    bge(0, fluid_uniforms_buf.as_entire_binding()),
                    bge(1, injection_buf.as_entire_binding()),
                    bge(2, wgpu::BindingResource::TextureView(&vd_view[0])),
                    bge(3, wgpu::BindingResource::TextureView(&vd_view[1])),
                ],
            }),
        ];

        // ── Render (volumetric raymarch) pipeline ──────────────────────
        let render_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid3-render-bgl"),
            entries: &[
                // 0: VolumeCamera uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 1: lit_density 3D texture (filterable)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D3,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                // 2: filtering sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // 3: depth texture
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Depth,
                    },
                    count: None,
                },
                // 4: point lights
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 5: density 3D texture (unfiltered — used via textureLoad
                // to sample fluid velocity near candle wicks for flame bend)
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D3,
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
                // 6: opaque occluders (bugs) — used by the raymarch's
                // forward in-scatter term so bug silhouettes cut visible
                // shafts of darkness through god rays from the lamp.
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                // 7: TAA history texture — previous frame's raymarch
                // output, sampled at the reprojected UV of this pixel's
                // first-hit world position. Must be the OTHER slot of
                // the ping-pong (never the one we're writing to).
                wgpu::BindGroupLayoutEntry {
                    binding: 7,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                // 8: filtering sampler for the history texture.
                wgpu::BindGroupLayoutEntry {
                    binding: 8,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fluid3-render-pl"),
                bind_group_layouts: &[Some(globals_layout), Some(&render_layout)],
                immediate_size: 0,
            });
        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fluid3-render-pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &volume_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &volume_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: SMOKE_OFFSCREEN_FORMAT,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // ── Composite pipeline ──────────────────────────────────────────
        let composite_shader = make_shader(
            "fluid3-composite",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/fluid3_composite.wgsl"
            )),
        );
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid3-composite-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Depth,
                    },
                    count: None,
                },
            ],
        });
        let composite_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fluid3-composite-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let history_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fluid3-history-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let composite_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("fluid3-composite-pl"),
                bind_group_layouts: &[Some(&composite_layout)],
                immediate_size: 0,
            });
        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fluid3-composite-pipeline"),
            layout: Some(&composite_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &composite_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &composite_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            vd,
            vd_view,
            lit_density,
            lit_density_view,
            linear_sampler,
            fluid_uniforms_buf,
            injection_buf,
            cam_buf,
            occluders_buf,
            pending_occluders: Vec::new(),
            #[cfg(debug_assertions)]
            mode_buf_smoke_only,
            #[cfg(debug_assertions)]
            mode_buf_default,
            advect_pipeline,
            inject_pipeline,
            lightbake_pipeline,
            lightbake_layout,
            lightbake_bg: None,
            advect_bgs,
            inject_bgs,
            render_pipeline,
            render_layout,
            render_bgs: [None, None],
            composite_pipeline,
            composite_layout,
            composite_sampler,
            composite_bgs: [None, None],
            offscreen_texture: [None, None],
            offscreen_view: [None, None],
            history_sampler,
            current_index: 0,
            history_valid: false,
            frame_index: 0,
            prev_view_proj: Mat4::IDENTITY,
            offscreen_w: 0,
            offscreen_h: 0,
            current_detail: None,
            impulses: Vec::new(),
            grid_min: Vec3::new(-100.0, -100.0, 0.0),
            grid_max: Vec3::new(100.0, 100.0, 60.0),
            grid_size: default_grid,
            screen_w,
            screen_h,
            pending_clear: false,
            sim_time: 0.0,
            dust_strength: 0.0,
            active_world_aabb: None,
            active_aabb_max_speed: 0.0,
        }
    }

    /// Set the ambient-dust floor density. See `dust_strength` field doc.
    pub fn set_dust_strength(&mut self, v: f32) {
        self.dust_strength = v.max(0.0);
    }

    pub fn update_screen_size(&mut self, w: f32, h: f32) {
        self.screen_w = w;
        self.screen_h = h;
        self.current_detail = None;
    }

    pub fn set_detail(
        &mut self,
        device: &wgpu::Device,
        quality: SmokeQuality,
        depth_view: &wgpu::TextureView,
    ) {
        let div = quality.target_divisor().max(1);
        let target_w = (self.screen_w as u32 / div).max(1);
        let target_h = (self.screen_h as u32 / div).max(1);
        if Some(quality) == self.current_detail
            && self.offscreen_w == target_w
            && self.offscreen_h == target_h
            && self.offscreen_texture[0].is_some()
            && self.offscreen_texture[1].is_some()
        {
            return;
        }
        for slot in 0..2 {
            if let Some(t) = self.offscreen_texture[slot].take() {
                t.destroy();
            }
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(if slot == 0 {
                    "fluid3-smoke-offscreen-0"
                } else {
                    "fluid3-smoke-offscreen-1"
                }),
                size: wgpu::Extent3d {
                    width: target_w,
                    height: target_h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: SMOKE_OFFSCREEN_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.composite_bgs[slot] = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(if slot == 0 {
                    "fluid3-composite-bg-0"
                } else {
                    "fluid3-composite-bg-1"
                }),
                layout: &self.composite_layout,
                entries: &[
                    bge(0, wgpu::BindingResource::TextureView(&view)),
                    bge(1, wgpu::BindingResource::Sampler(&self.composite_sampler)),
                    bge(2, wgpu::BindingResource::TextureView(depth_view)),
                ],
            }));
            self.offscreen_texture[slot] = Some(texture);
            self.offscreen_view[slot] = Some(view);
        }
        self.offscreen_w = target_w;
        self.offscreen_h = target_h;
        self.current_detail = Some(quality);
        // Fresh offscreen targets have undefined contents; next frame's
        // raymarch must skip the TAA blend. Render bg needs rebuilding
        // so it picks up the new texture views.
        self.history_valid = false;
        self.render_bgs = [None, None];
    }

    /// Screen-space rect tight to the currently-active portion of the
    /// smoke field, unioned with an optional flame AABB (candle-light
    /// bounds) so the raymarch's flame sub-pass isn't scissored out when
    /// there's no smoke in the grid. Returns `None` when both sources
    /// contribute nothing — the caller should skip the raymarch in that
    /// case. Smoke volume is clamped to the grid bounds; flame volume
    /// isn't (candles may sit above the grid).
    pub fn screen_aabb_rect(
        &self,
        view_proj: Mat4,
        flame_aabb: Option<(Vec3, Vec3)>,
    ) -> Option<(u32, u32, u32, u32)> {
        let target_w = self.offscreen_w;
        let target_h = self.offscreen_h;
        if target_w == 0 || target_h == 0 {
            return None;
        }

        let smoke_rect = self.active_world_aabb.and_then(|(mn, mx)| {
            let mn = mn.max(self.grid_min);
            let mx = mx.min(self.grid_max);
            if mn.x >= mx.x || mn.y >= mx.y || mn.z >= mx.z {
                None
            } else {
                project_world_aabb(view_proj, mn, mx, target_w, target_h)
            }
        });
        let flame_rect = flame_aabb
            .and_then(|(mn, mx)| project_world_aabb(view_proj, mn, mx, target_w, target_h));

        let rect = match (smoke_rect, flame_rect) {
            (Some(a), Some(b)) => Some((
                a.0.min(b.0),
                a.1.min(b.1),
                a.2.max(b.2),
                a.3.max(b.3),
            )),
            (Some(r), None) | (None, Some(r)) => Some(r),
            (None, None) => None,
        }?;

        let pad = 1.0;
        let min_x = ((rect.0 - pad).floor().max(0.0) as u32).min(target_w);
        let min_y = ((rect.1 - pad).floor().max(0.0) as u32).min(target_h);
        let max_x = ((rect.2 + pad).ceil().max(0.0) as u32).min(target_w);
        let max_y = ((rect.3 + pad).ceil().max(0.0) as u32).min(target_h);
        if max_x <= min_x || max_y <= min_y {
            return None;
        }
        Some((min_x, min_y, max_x - min_x, max_y - min_y))
    }

    pub fn set_grid_bounds(&mut self, grid_min: Vec3, grid_max: Vec3) {
        self.grid_min = grid_min;
        self.grid_max = grid_max;
    }

    /// Queue a world-space impulse for the current frame. `temperature` and
    /// `phase` are accepted for API stability but ignored internally — the
    /// simplified pipeline doesn't run a temperature field.
    pub fn inject_impulse(
        &mut self,
        world_pos: Vec3,
        world_vel: Vec3,
        radius: f32,
        density: f32,
        temperature: f32,
        phase: f32,
    ) {
        if self.impulses.len() < MAX_INJECTIONS {
            self.impulses.push(Impulse {
                world_pos,
                world_vel,
                radius,
                density,
                temperature,
                phase,
            });
        }
    }

    pub fn clear(&mut self) {
        self.pending_clear = true;
        // A wipe changes the density field discontinuously; blending the
        // next frame's raymarch against the pre-wipe history would smear
        // stale smoke over blank. Force a one-frame TAA skip.
        self.history_valid = false;
    }

    /// Update the tracked world-space AABB of active smoke. Called once
    /// per frame from `step()` after the impulse buffer has been packed
    /// and before it's drained.
    ///
    /// Model: the AABB shrinks each frame in proportion to the density
    /// dissipation factor the sim applies, plus a drift pad in +Z to
    /// keep rising smoke inside the rect. Each impulse expands the AABB
    /// to cover its influence sphere with a generous margin so edge
    /// wisps don't get clipped. When the decayed extent falls below a
    /// few grid cells, the AABB collapses to `None` and the raymarch
    /// skips entirely.
    fn update_active_aabb(
        &mut self,
        clearing: bool,
        density_dis_step: f32,
        drift_speed: f32,
        dt: f32,
    ) {
        if clearing {
            self.active_world_aabb = None;
            self.active_aabb_max_speed = 0.0;
            return;
        }

        // Dust floor: smoke exists everywhere. Skip the whole tracker and
        // keep the rect pinned to the full grid so ambient haze isn't
        // scissored out.
        if self.dust_strength > 0.0 {
            self.active_world_aabb = Some((self.grid_min, self.grid_max));
            return;
        }

        // Curl noise amplitude — matches the `curl_strength` constant
        // passed to the advect shader. Kept in sync by hand; if that
        // tuning changes, update here too.
        const CURL_SPEED: f32 = 6.5;

        let had_impulses = !self.impulses.is_empty();

        // Update the rolling max-speed estimate from this frame's
        // impulses, decaying the previous value. Advected velocity drives
        // how fast the plume can escape last frame's rect, and it decays
        // as the velocity field dissipates.
        self.active_aabb_max_speed *= density_dis_step;
        for imp in &self.impulses {
            if imp.density <= 0.0 || imp.radius <= 0.0 {
                continue;
            }
            let s = imp.world_vel.length();
            if s > self.active_aabb_max_speed {
                self.active_aabb_max_speed = s;
            }
        }

        // Union in each new impulse's influence sphere. Generous pad:
        // the inject kernel falls off to ~2× the nominal radius, and
        // the impulse's own velocity will carry density outward over
        // the next few frames. Lookahead of 0.5s at injection speed
        // covers the bulk of the travel before dissipation catches up.
        const IMPULSE_LOOKAHEAD: f32 = 0.5;
        for imp in &self.impulses {
            if imp.density <= 0.0 || imp.radius <= 0.0 {
                continue;
            }
            let pad = imp.radius * 4.0 + imp.world_vel.length() * IMPULSE_LOOKAHEAD;
            let pad_v = Vec3::splat(pad);
            let imp_mn = imp.world_pos - pad_v;
            let imp_mx = imp.world_pos + pad_v;
            self.active_world_aabb = Some(match self.active_world_aabb {
                Some((mn, mx)) => (mn.min(imp_mn), mx.max(imp_mx)),
                None => (imp_mn, imp_mx),
            });
        }

        // Per-frame growth: extend every face by the distance density
        // can travel this frame under drift + curl + the strongest
        // recent impulse velocity. This is the crucial step — without
        // it, velocity-driven puffs escape the rect before it can catch
        // up. +Z gets an extra drift bump since drift is unidirectional.
        if let Some((mut mn, mut mx)) = self.active_world_aabb {
            let travel = (drift_speed + CURL_SPEED + self.active_aabb_max_speed) * dt;
            let travel_v = Vec3::splat(travel);
            mn -= travel_v;
            mx += travel_v;
            mx.z += drift_speed * dt;

            // Shrink only on frames with no new impulses, and very
            // slowly (fourth-root of the density step — roughly 1/4 the
            // rate the density itself decays). Without a density
            // readback we can't know the real bounds, so we prefer to
            // err large and let many idle frames gradually close the
            // box. Typical dissipation = 0.885^(dt*60) per frame; to
            // the 0.25 power that's ~0.97/frame, fully collapsing over
            // ~2 seconds of idle.
            if !had_impulses {
                let shrink = density_dis_step.powf(0.25);
                let center = (mn + mx) * 0.5;
                let half = (mx - mn) * 0.5 * shrink;
                mn = center - half;
                mx = center + half;
            }

            // Drop the box only when it has fallen below a single grid
            // cell on every axis — smaller than that, the raymarch
            // couldn't produce a visible sample anyway.
            let extent = mx - mn;
            let cell = {
                let grid_extent = self.grid_max - self.grid_min;
                grid_extent
                    / Vec3::new(
                        self.grid_size.x.max(1) as f32,
                        self.grid_size.y.max(1) as f32,
                        self.grid_size.z.max(1) as f32,
                    )
            };
            if extent.x < cell.x && extent.y < cell.y && extent.z < cell.z {
                self.active_world_aabb = None;
                self.active_aabb_max_speed = 0.0;
            } else {
                self.active_world_aabb = Some((mn, mx));
            }
        }
    }

    /// True when the render bind groups need to be (re)built — either
    /// depth view changed (renderer sets its own dirty flag) or the
    /// offscreen ping-pong textures were reallocated by `set_detail`
    /// and the bgs were cleared. Safe to call every frame.
    pub fn render_bgs_need_rebuild(&self) -> bool {
        self.render_bgs[0].is_none() || self.render_bgs[1].is_none()
    }

    /// Replace the list of opaque occluders (e.g. shop bugs) sampled by
    /// the lightbake this frame. Anything beyond `MAX_BUG_OCCLUDERS` is
    /// dropped silently.
    pub fn set_occluders(&mut self, occluders: &[BugOccluder]) {
        self.pending_occluders.clear();
        self.pending_occluders
            .extend(occluders.iter().take(MAX_BUG_OCCLUDERS).copied());
    }

    fn active_grid_extent(&self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.grid_size.x,
            height: self.grid_size.y,
            depth_or_array_layers: self.grid_size.z,
        }
    }

    fn dispatch_3d_pass(
        encoder: &mut wgpu::CommandEncoder,
        label: &'static str,
        pipeline: &wgpu::ComputePipeline,
        bind_group: &wgpu::BindGroup,
        wg_x: u32,
        wg_y: u32,
        wg_z: u32,
    ) {
        let mut p = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(label),
            timestamp_writes: None,
        });
        p.set_pipeline(pipeline);
        p.set_bind_group(0, bind_group, &[]);
        p.dispatch_workgroups(wg_x, wg_y, wg_z);
    }

    /// Run one simulation step. Call before beginning the render pass.
    pub fn step(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        dt: f32,
        quality: SmokeQuality,
    ) {
        let clearing = self.pending_clear;
        if !handle_pre_step_state(&mut self.pending_clear, &mut self.impulses, quality) {
            return;
        }
        self.grid_size = grid_dims_for_quality(quality);

        // Drift, curl, and dissipation are sim-look constants — they're
        // tuned for "smoke that reads as a candle plume" and don't need
        // to scale with the user's perf knob. Off is handled above by the
        // pre-step short-circuit.
        let (density_dis, drift_speed, curl_strength) = (0.885_f64, 6.0_f32, 6.5_f32);

        let extent = self.grid_max - self.grid_min;
        let inv_extent = Vec3::new(
            1.0 / extent.x.max(1e-3),
            1.0 / extent.y.max(1e-3),
            1.0 / extent.z.max(1e-3),
        );
        let dt_clamped = dt.min(0.05);
        self.sim_time = (self.sim_time + dt_clamped) % 3600.0;

        // Make per-step density dissipation framerate-independent.
        // pow(c, dt*60) is the closed-form continuous decay equivalent and
        // reproduces the old behaviour exactly when dt = 1/60.
        let dt_scale = dt_clamped * 60.0;
        let density_dis_step = (density_dis as f32).powf(dt_scale);

        queue.write_buffer(
            &self.fluid_uniforms_buf,
            0,
            bytemuck::bytes_of(&FluidUniformsGpu {
                grid_size: [
                    self.grid_size.x as f32,
                    self.grid_size.y as f32,
                    self.grid_size.z as f32,
                    self.sim_time,
                ],
                grid_min: [self.grid_min.x, self.grid_min.y, self.grid_min.z, 0.0],
                grid_max: [self.grid_max.x, self.grid_max.y, self.grid_max.z, 0.0],
                inv_extent: [inv_extent.x, inv_extent.y, inv_extent.z, 0.0],
                params: [dt_clamped, density_dis_step, drift_speed, curl_strength],
                // curl_spatial_scale, curl_time_scale, stored_vel_mix, dust_strength
                force_params: [0.038, 0.55, 1.0, self.dust_strength],
            }),
        );

        // ── Pack impulses ──────────────────────────────────────────────
        let mut injection = InjectionParamsGpu {
            points: [InjectionPointGpu {
                pos_radius: [0.0; 4],
                vel_density: [0.0; 4],
                temperature_phase: [0.0; 4],
            }; MAX_INJECTIONS],
            active_count: [self.impulses.len().min(MAX_INJECTIONS) as u32, 0, 0, 0],
        };
        for (i, imp) in self.impulses.iter().take(MAX_INJECTIONS).enumerate() {
            injection.points[i] = InjectionPointGpu {
                pos_radius: [
                    imp.world_pos.x,
                    imp.world_pos.y,
                    imp.world_pos.z,
                    imp.radius,
                ],
                vel_density: [imp.world_vel.x, imp.world_vel.y, imp.world_vel.z, imp.density],
                temperature_phase: [imp.temperature, imp.phase, 0.0, 0.0],
            };
        }
        self.update_active_aabb(clearing, density_dis_step, drift_speed, dt_clamped);
        self.impulses.clear();
        queue.write_buffer(&self.injection_buf, 0, bytemuck::bytes_of(&injection));

        // ── Pack occluders ─────────────────────────────────────────────
        let mut occ = OccludersGpu {
            count: [
                self.pending_occluders.len().min(MAX_BUG_OCCLUDERS) as u32,
                0,
                0,
                0,
            ],
            items: [OccluderGpu {
                pos_radius: [0.0; 4],
                params: [0.0; 4],
            }; MAX_BUG_OCCLUDERS],
        };
        for (i, o) in self
            .pending_occluders
            .iter()
            .take(MAX_BUG_OCCLUDERS)
            .enumerate()
        {
            occ.items[i] = OccluderGpu {
                pos_radius: [o.world_pos.x, o.world_pos.y, o.world_pos.z, o.radius],
                params: [o.strength, 0.0, 0.0, 0.0],
            };
        }
        queue.write_buffer(&self.occluders_buf, 0, bytemuck::bytes_of(&occ));

        let wg_x = self.grid_size.x.div_ceil(WG);
        let wg_y = self.grid_size.y.div_ceil(WG);
        let wg_z = self.grid_size.z.div_ceil(WG);

        if clearing {
            // Clear both ping-pong slices so nothing leaks across the wipe.
            let full_range = wgpu::ImageSubresourceRange {
                aspect: wgpu::TextureAspect::All,
                base_mip_level: 0,
                mip_level_count: None,
                base_array_layer: 0,
                array_layer_count: None,
            };
            encoder.clear_texture(&self.vd[0], &full_range);
            encoder.clear_texture(&self.vd[1], &full_range);
        } else {
            // 1. Advect vd[0] → vd[1], applying dissipation + drift + curl.
            Self::dispatch_3d_pass(
                encoder,
                "fluid3-advect",
                &self.advect_pipeline,
                &self.advect_bgs[0],
                wg_x,
                wg_y,
                wg_z,
            );
            // 2. Inject impulses from vd[1] → vd[0] so the final density
            //    lands in vd[0] for the lightbake and raymarch.
            Self::dispatch_3d_pass(
                encoder,
                "fluid3-inject",
                &self.inject_pipeline,
                &self.inject_bgs[0],
                wg_x,
                wg_y,
                wg_z,
            );
        }

        // 3. Lightbake: walk every voxel of vd[0], evaluate the candle
        // point lights, write pre-lit colour into lit_density. Skipped on
        // the very first frame until `rebuild_render_bind_group` has been
        // called with the renderer-owned point lights buffer.
        if let Some(ref bg) = self.lightbake_bg {
            let mut p = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fluid3-lightbake"),
                timestamp_writes: None,
            });
            p.set_pipeline(&self.lightbake_pipeline);
            p.set_bind_group(0, bg, &[]);
            p.dispatch_workgroups(wg_x, wg_y, wg_z);
        }

        // Silence unused-method warning for `active_grid_extent` — we no
        // longer issue texture-to-texture copies but the helper is still
        // useful for future ops.
        let _ = self.active_grid_extent();
    }

    /// Build (or rebuild) the render bind groups for the current depth
    /// view AND the lightbake bind group (which references the
    /// renderer-owned `point_lights_buffer`). Called on depth-texture
    /// recreation (resize) — and re-attempted lazily if the offscreen
    /// ping-pong textures hadn't been allocated yet when this was first
    /// invoked.
    ///
    /// The render bgs are ping-ponged: slot `i` binds the OTHER slot's
    /// offscreen view as the TAA history input, so writing into slot
    /// `i` reads slot `1-i` as history (matching the read-don't-write
    /// invariant that wgpu requires for a texture being used as both
    /// attachment and sampled input in the same frame).
    pub fn rebuild_render_bind_group(
        &mut self,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
        point_lights_buffer: &wgpu::Buffer,
    ) {
        self.lightbake_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fluid3-lightbake-bg"),
            layout: &self.lightbake_layout,
            entries: &[
                bge(0, self.fluid_uniforms_buf.as_entire_binding()),
                bge(1, wgpu::BindingResource::TextureView(&self.vd_view[0])),
                bge(
                    2,
                    wgpu::BindingResource::TextureView(&self.lit_density_view),
                ),
                bge(3, self.cam_buf.as_entire_binding()),
                bge(4, point_lights_buffer.as_entire_binding()),
                bge(5, self.occluders_buf.as_entire_binding()),
            ],
        }));

        for slot in 0..2 {
            let history_slot = 1 - slot;
            let Some(history_view) = self.offscreen_view[history_slot].as_ref() else {
                // Offscreen textures not allocated yet — `render_offscreen`
                // will call back into this helper once they are.
                self.render_bgs[slot] = None;
                continue;
            };
            self.render_bgs[slot] = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(if slot == 0 {
                    "fluid3-render-bg-0"
                } else {
                    "fluid3-render-bg-1"
                }),
                layout: &self.render_layout,
                entries: &[
                    bge(0, self.cam_buf.as_entire_binding()),
                    bge(
                        1,
                        wgpu::BindingResource::TextureView(&self.lit_density_view),
                    ),
                    bge(2, wgpu::BindingResource::Sampler(&self.linear_sampler)),
                    bge(3, wgpu::BindingResource::TextureView(depth_view)),
                    bge(4, point_lights_buffer.as_entire_binding()),
                    bge(5, wgpu::BindingResource::TextureView(&self.vd_view[0])),
                    bge(6, self.occluders_buf.as_entire_binding()),
                    bge(7, wgpu::BindingResource::TextureView(history_view)),
                    bge(8, wgpu::BindingResource::Sampler(&self.history_sampler)),
                ],
            }));
        }
    }

    pub fn upload_camera_uniform(
        &mut self,
        queue: &wgpu::Queue,
        view_proj: Mat4,
        cam_pos: Vec3,
        quality: SmokeQuality,
        amount: SmokeAmount,
    ) {
        self.grid_size = grid_dims_for_quality(quality);
        let inv_vp = view_proj.inverse();
        // step_count is a *floor* — the volume shader auto-bumps it to
        // match voxel resolution along the ray, so blocky silhouettes
        // can't slip in. Lowered from the pre-TAA values now that
        // temporal reprojection reconstructs the per-pixel integral over
        // multiple frames: 48–120 jittered steps converges to the same
        // image as 56–160 non-jittered steps in 6–8 frames.
        let (step_count, light_strength, ambient) = match quality {
            SmokeQuality::Off => (8.0, 0.0, 0.0),
            SmokeQuality::Low => (40.0, 1.20, 0.10),
            SmokeQuality::Medium => (56.0, 1.30, 0.11),
            SmokeQuality::High => (80.0, 1.45, 0.12),
            SmokeQuality::Ultra => (112.0, 1.60, 0.14),
        };
        let max_alpha = if matches!(quality, SmokeQuality::Off) {
            0.0
        } else {
            amount.max_alpha()
        };
        let history_flag = if self.history_valid { 1.0 } else { 0.0 };
        let frame_f = (self.frame_index & 0xFF) as f32;
        queue.write_buffer(
            &self.cam_buf,
            0,
            bytemuck::bytes_of(&VolumeCameraGpu {
                inv_view_proj: inv_vp.to_cols_array(),
                view_proj: view_proj.to_cols_array(),
                prev_view_proj: self.prev_view_proj.to_cols_array(),
                cam_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
                grid_min: [self.grid_min.x, self.grid_min.y, self.grid_min.z, 0.0],
                grid_max: [self.grid_max.x, self.grid_max.y, self.grid_max.z, 0.0],
                grid_size: [
                    self.grid_size.x as f32,
                    self.grid_size.y as f32,
                    self.grid_size.z as f32,
                    0.0,
                ],
                params: [max_alpha, step_count, light_strength, ambient],
                mode: [0.0, history_flag, frame_f, 0.0],
            }),
        );
        // Cache this frame's view_proj so next frame can reproject into
        // this frame's screen space.
        self.prev_view_proj = view_proj;
    }

    #[cfg(debug_assertions)]
    pub fn set_render_mode_encoder(&self, encoder: &mut wgpu::CommandEncoder, smoke_only: bool) {
        let src = if smoke_only {
            &self.mode_buf_smoke_only
        } else {
            &self.mode_buf_default
        };
        let offset = std::mem::offset_of!(VolumeCameraGpu, mode) as u64;
        encoder.copy_buffer_to_buffer(src, 0, &self.cam_buf, offset, 16);
    }

    /// Clear the offscreen smoke target without running the raymarch.
    /// Called when the active-AABB scissor is empty (no smoke in the
    /// grid) so the subsequent composite samples a transparent texture
    /// and contributes nothing. Skipping only the draw (keeping the
    /// clear) is essentially free compared to the ~15ms fullscreen
    /// raymarch.
    pub fn clear_offscreen(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let idx = self.current_index;
        let Some(view) = self.offscreen_view[idx].as_ref() else {
            return;
        };
        let _ = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fluid3-smoke-offscreen-clear"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes,
            multiview_mask: None,
        });
    }

    pub fn render_offscreen(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        globals_bind_group: &wgpu::BindGroup,
        scissor: Option<(u32, u32, u32, u32)>,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let idx = self.current_index;
        let (Some(view), Some(render_bg)) = (
            self.offscreen_view[idx].as_ref(),
            self.render_bgs[idx].as_ref(),
        ) else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fluid3-smoke-offscreen-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            occlusion_query_set: None,
            timestamp_writes,
            multiview_mask: None,
        });
        if let Some((x, y, w, h)) = scissor {
            if w > 0 && h > 0 && x + w <= self.offscreen_w && y + h <= self.offscreen_h {
                pass.set_scissor_rect(x, y, w, h);
            }
        }
        pass.set_pipeline(&self.render_pipeline);
        pass.set_bind_group(0, globals_bind_group, &[]);
        pass.set_bind_group(1, render_bg, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Advance the TAA ping-pong one frame. Call exactly once per frame
    /// after the composite reads the current slot — this flips which
    /// slot the next raymarch writes into and marks history as valid
    /// (the slot just composited becomes history input next frame).
    /// Separated from `render_offscreen` so the same slot is used by
    /// both the raymarch write and the composite read within a frame.
    pub fn advance_taa_frame(&mut self) {
        // After this frame's raymarch finishes, the slot we just wrote
        // is the new "previous" for the next frame. Flip and mark valid.
        self.current_index = 1 - self.current_index;
        self.history_valid = true;
        self.frame_index = self.frame_index.wrapping_add(1);
    }

    pub fn draw_composite(&self, pass: &mut wgpu::RenderPass<'_>) {
        // The composite reads the slot we just rendered into this
        // frame. `advance_taa_frame` flips `current_index` *after* the
        // composite, so `current_index` here still points at the
        // just-written target.
        let Some(ref bg) = self.composite_bgs[self.current_index] else {
            return;
        };
        pass.set_pipeline(&self.composite_pipeline);
        pass.set_bind_group(0, bg, &[]);
        pass.draw(0..3, 0..1);
    }
}

// ──────────────────────────────────────────────────────────────────────
// Bind group layout helpers
// ──────────────────────────────────────────────────────────────────────

fn bge(binding: u32, resource: wgpu::BindingResource<'_>) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry { binding, resource }
}

fn bgl_uniform(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn bgl_storage3d(binding: u32, format: wgpu::TextureFormat) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::StorageTexture {
            access: wgpu::StorageTextureAccess::WriteOnly,
            format,
            view_dimension: wgpu::TextureViewDimension::D3,
        },
        count: None,
    }
}

fn bgl_tex3d_float(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            multisampled: false,
            view_dimension: wgpu::TextureViewDimension::D3,
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
        },
        count: None,
    }
}

fn bgl_tex3d_unfiltered(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            multisampled: false,
            view_dimension: wgpu::TextureViewDimension::D3,
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
        },
        count: None,
    }
}

fn bgl_sampler(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_step_drains_pending_impulses() {
        let mut pending_clear = false;
        let mut impulses = vec![Impulse {
            world_pos: Vec3::ZERO,
            world_vel: Vec3::X,
            radius: 1.0,
            density: 1.0,
            temperature: 0.0,
            phase: 0.0,
        }];

        assert!(!handle_pre_step_state(
            &mut pending_clear,
            &mut impulses,
            SmokeQuality::Off,
        ));
        assert!(impulses.is_empty());
    }
}
