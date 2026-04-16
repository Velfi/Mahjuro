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

use crate::persistence::{SmokeDetail, SmokeIntensity, SmokeSimQuality};

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

fn grid_dims_for_quality(quality: SmokeSimQuality) -> GridDims {
    match quality {
        // Texture (x, y, z) ↔ world (X, Y, Z); vertical smoke is along Z.
        SmokeSimQuality::Standard => GridDims::new(96, 96, 48),
        SmokeSimQuality::High => GridDims::new(112, 112, 64),
        SmokeSimQuality::Ultra => GridDims::new(MAX_GRID_XY, MAX_GRID_XY, MAX_GRID_Z_UP),
    }
}

// Per-frame impulse budget. Sized to comfortably fit the worst-case
// opening frame: a full hand of sliding tiles + the 24-cell post-deal
// wind sweep + the cursor puff, with headroom.
//
// Must stay in sync with `MAX_INJECTIONS` and the `points` array length
// in `shaders/fluid3_inject.wgsl`.
const MAX_INJECTIONS: usize = 64;

fn handle_pre_step_state(
    pending_clear: &mut bool,
    impulses: &mut Vec<Impulse>,
    intensity: SmokeIntensity,
) -> bool {
    let clearing = *pending_clear;
    if clearing {
        *pending_clear = false;
        impulses.clear();
    }
    if !clearing && matches!(intensity, SmokeIntensity::Off) {
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
    /// shader reads back out of xyz), w = unused.
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
struct VolumeCameraGpu {
    inv_view_proj: [f32; 16],
    view_proj: [f32; 16],
    cam_pos: [f32; 4],
    grid_min: [f32; 4],
    grid_max: [f32; 4],
    grid_size: [f32; 4],
    /// x=max_alpha, y=step_count (as f32), z=light_strength, w=ambient
    params: [f32; 4],
    /// x=render mode: 0=both smoke+flames, 1=smoke only, 2=flames only.
    /// yzw unused (padding for std140 alignment).
    mode: [f32; 4],
}

// ──────────────────────────────────────────────────────────────────────
// Public impulse type
// ──────────────────────────────────────────────────────────────────────

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
    render_bg: Option<wgpu::BindGroup>,

    // Composite pipeline that samples the offscreen target with bilinear
    // filtering and blends it onto the swap chain with premultiplied alpha.
    composite_pipeline: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    composite_sampler: wgpu::Sampler,
    composite_bg: Option<wgpu::BindGroup>,

    offscreen_texture: Option<wgpu::Texture>,
    offscreen_view: Option<wgpu::TextureView>,
    offscreen_w: u32,
    offscreen_h: u32,
    current_detail: Option<SmokeDetail>,

    impulses: Vec<Impulse>,

    grid_min: Vec3,
    grid_max: Vec3,
    grid_size: GridDims,

    screen_w: f32,
    screen_h: f32,

    pending_clear: bool,
    sim_time: f32,
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
        let default_grid = grid_dims_for_quality(SmokeSimQuality::High);
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
        let cam_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fluid3-cam"),
            contents: bytemuck::bytes_of(&VolumeCameraGpu {
                inv_view_proj: Mat4::IDENTITY.to_cols_array(),
                view_proj: Mat4::IDENTITY.to_cols_array(),
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

        // Lightbake: uniforms, src density, dst lit_density, cam, lights.
        let lightbake_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid3-lightbake-bgl"),
            entries: &[
                bgl_uniform(0),
                bgl_tex3d_unfiltered(1),
                bgl_storage3d(2, wgpu::TextureFormat::Rgba16Float),
                bgl_uniform(3),
                bgl_uniform(4),
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
            render_bg: None,
            composite_pipeline,
            composite_layout,
            composite_sampler,
            composite_bg: None,
            offscreen_texture: None,
            offscreen_view: None,
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
        }
    }

    pub fn update_screen_size(&mut self, w: f32, h: f32) {
        self.screen_w = w;
        self.screen_h = h;
        self.current_detail = None;
    }

    pub fn set_detail(
        &mut self,
        device: &wgpu::Device,
        detail: SmokeDetail,
        depth_view: &wgpu::TextureView,
    ) {
        let div = detail.divisor().max(1);
        let target_w = (self.screen_w as u32 / div).max(1);
        let target_h = (self.screen_h as u32 / div).max(1);
        if Some(detail) == self.current_detail
            && self.offscreen_w == target_w
            && self.offscreen_h == target_h
            && self.offscreen_texture.is_some()
        {
            return;
        }
        if let Some(t) = self.offscreen_texture.take() {
            t.destroy();
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fluid3-smoke-offscreen"),
            size: wgpu::Extent3d {
                width: target_w,
                height: target_h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SMOKE_OFFSCREEN_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.composite_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fluid3-composite-bg"),
            layout: &self.composite_layout,
            entries: &[
                bge(0, wgpu::BindingResource::TextureView(&view)),
                bge(1, wgpu::BindingResource::Sampler(&self.composite_sampler)),
                bge(2, wgpu::BindingResource::TextureView(depth_view)),
            ],
        }));
        self.offscreen_texture = Some(texture);
        self.offscreen_view = Some(view);
        self.offscreen_w = target_w;
        self.offscreen_h = target_h;
        self.current_detail = Some(detail);
    }

    pub fn screen_aabb_rect(&self, view_proj: Mat4) -> Option<(u32, u32, u32, u32)> {
        use glam::Vec4;

        let target_w = self.offscreen_w;
        let target_h = self.offscreen_h;
        if target_w == 0 || target_h == 0 {
            return None;
        }
        let mn = self.grid_min;
        let mx = self.grid_max;
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
            return Some((0, 0, target_w, target_h));
        }
        let pad = 1.0;
        let min_x = ((min_x - pad).floor().max(0.0) as u32).min(target_w);
        let min_y = ((min_y - pad).floor().max(0.0) as u32).min(target_h);
        let max_x = ((max_x + pad).ceil().max(0.0) as u32).min(target_w);
        let max_y = ((max_y + pad).ceil().max(0.0) as u32).min(target_h);
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
        intensity: SmokeIntensity,
        sim_quality: SmokeSimQuality,
    ) {
        let clearing = self.pending_clear;
        if !handle_pre_step_state(&mut self.pending_clear, &mut self.impulses, intensity) {
            return;
        }
        self.grid_size = grid_dims_for_quality(sim_quality);

        // Per-intensity tuning: dissipation + drift + curl strength.
        //
        // dissipation is per 60-Hz frame; we rescale to real dt below.
        let (density_dis, drift_speed, curl_strength) = match intensity {
            SmokeIntensity::Off => (0.0_f64, 0.0_f32, 0.0_f32),
            SmokeIntensity::Subtle => (0.820, 4.0, 4.5),
            SmokeIntensity::Strong => (0.885, 6.0, 6.5),
            SmokeIntensity::OverTheTop => (0.935, 8.0, 9.0),
        };

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
                // curl_spatial_scale, curl_time_scale, stored_vel_mix, unused
                force_params: [0.022, 0.55, 1.0, 0.0],
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
        self.impulses.clear();
        queue.write_buffer(&self.injection_buf, 0, bytemuck::bytes_of(&injection));

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

    /// Build (or rebuild) the render bind group for the current depth view
    /// AND the lightbake bind group (which references the renderer-owned
    /// `point_lights_buffer`). Both share the same trigger — depth-texture
    /// recreation on resize — so they're built together.
    pub fn rebuild_render_bind_group(
        &mut self,
        device: &wgpu::Device,
        depth_view: &wgpu::TextureView,
        point_lights_buffer: &wgpu::Buffer,
    ) {
        self.render_bg = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fluid3-render-bg"),
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
            ],
        }));
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
            ],
        }));
    }

    pub fn upload_camera_uniform(
        &mut self,
        queue: &wgpu::Queue,
        view_proj: Mat4,
        cam_pos: Vec3,
        intensity: SmokeIntensity,
        sim_quality: SmokeSimQuality,
    ) {
        self.grid_size = grid_dims_for_quality(sim_quality);
        let inv_vp = view_proj.inverse();
        let (max_alpha, step_count, light_strength, ambient) = match (intensity, sim_quality) {
            (SmokeIntensity::Off, _) => (0.0, 8.0, 0.0, 0.0),
            (SmokeIntensity::Subtle, SmokeSimQuality::Standard) => (0.42, 28.0, 1.00, 0.07),
            (SmokeIntensity::Subtle, SmokeSimQuality::High) => (0.44, 34.0, 1.08, 0.08),
            (SmokeIntensity::Subtle, SmokeSimQuality::Ultra) => (0.46, 42.0, 1.16, 0.09),
            (SmokeIntensity::Strong, SmokeSimQuality::Standard) => (0.58, 36.0, 1.22, 0.10),
            (SmokeIntensity::Strong, SmokeSimQuality::High) => (0.62, 46.0, 1.34, 0.11),
            (SmokeIntensity::Strong, SmokeSimQuality::Ultra) => (0.66, 56.0, 1.46, 0.12),
            (SmokeIntensity::OverTheTop, SmokeSimQuality::Standard) => (0.75, 44.0, 1.45, 0.12),
            (SmokeIntensity::OverTheTop, SmokeSimQuality::High) => (0.80, 56.0, 1.58, 0.14),
            (SmokeIntensity::OverTheTop, SmokeSimQuality::Ultra) => (0.86, 68.0, 1.72, 0.16),
        };
        queue.write_buffer(
            &self.cam_buf,
            0,
            bytemuck::bytes_of(&VolumeCameraGpu {
                inv_view_proj: inv_vp.to_cols_array(),
                view_proj: view_proj.to_cols_array(),
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
                mode: [0.0, 0.0, 0.0, 0.0],
            }),
        );
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

    pub fn render_offscreen(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        globals_bind_group: &wgpu::BindGroup,
        scissor: Option<(u32, u32, u32, u32)>,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let (Some(view), Some(render_bg)) = (self.offscreen_view.as_ref(), self.render_bg.as_ref())
        else {
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

    pub fn draw_composite(&self, pass: &mut wgpu::RenderPass<'_>) {
        let Some(ref bg) = self.composite_bg else {
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
            SmokeIntensity::Off,
        ));
        assert!(impulses.is_empty());
    }
}
