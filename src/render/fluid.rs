//! GPU-accelerated 3D Eulerian fluid simulation for volumetric smoke.
//!
//! Runs a Navier-Stokes step on a 3D grid via compute shaders, then renders
//! the smoke as a ray-marched, depth-occluded fullscreen pass that lights
//! from the same point lights as the rest of the 3D scene.
//!
//! Pipeline per frame:
//!   1. inject       — splat impulses (gaussian) into ping[0] → ping[1]
//!   2. advect       — backtrace+filter+buoyancy+dissipation, ping[1] → ping[0]
//!   3. divergence   — compute ∇·v from ping[0], clear pressure[0]
//!   4. jacobi N×    — pressure_a ↔ pressure_b (forced even so result lands in [0])
//!   5. project      — subtract ∇p from ping[0] → ping[1]
//!   6. copy         — ping[1] → ping[0] so next frame's inject reads "current"
//!
//! Velocity + density share an `Rgba16Float` 3D texture (xyz=velocity, w=density).
//! Pressure and divergence use `R32Float`.

use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;

use crate::persistence::{SmokeDetail, SmokeIntensity};

/// Pixel format used for the offscreen smoke render target. `Rgba16Float`
/// gives the volume shader headroom for HDR-style lighting accumulation
/// before the composite pass blends it onto the sRGB swap chain.
const SMOKE_OFFSCREEN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

// ──────────────────────────────────────────────────────────────────────
// Grid configuration
// ──────────────────────────────────────────────────────────────────────

const GRID_X: u32 = 96;
const GRID_Y: u32 = 48;
const GRID_Z: u32 = 96;
const WG: u32 = 4;

// Per-frame impulse budget. Sized to comfortably fit the worst-case
// opening frame: a full hand of sliding tiles (~14 motion impulses) +
// the 24-cell post-deal wind sweep grid + candle plumes + the cursor
// puff, with headroom. Used to be 32, which silently dropped the
// scene-driven wind gusts (which come last in the inject order) when
// the slide animation overlapped the sweep window — leaving the opening
// smoke curtain with nothing to blow it away.
//
// Must stay in sync with `MAX_INJECTIONS` and the `points` array length
// in `shaders/fluid3_inject.wgsl`.
const MAX_INJECTIONS: usize = 64;

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
    /// x = dt, y = density_dissipation, z = velocity_dissipation, w = buoyancy
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct InjectionPointGpu {
    pos_radius: [f32; 4],
    vel_density: [f32; 4],
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
    /// x=max_alpha, y=step_count (as f32), z=light_strength, w=ambient
    params: [f32; 4],
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
}

// ──────────────────────────────────────────────────────────────────────
// FluidSim
// ──────────────────────────────────────────────────────────────────────

pub struct FluidSim {
    // Storage textures (ping-pong vd; ping-pong pressure; single divergence).
    // The Texture handles are kept alive so the underlying GPU memory backing
    // the bound views isn't freed; we read/write through the views below.
    #[allow(dead_code)]
    vd: [wgpu::Texture; 2],
    vd_view: [wgpu::TextureView; 2],
    #[allow(dead_code)]
    pressure: [wgpu::Texture; 2],
    #[allow(dead_code)]
    pressure_view: [wgpu::TextureView; 2],
    #[allow(dead_code)]
    divergence: wgpu::Texture,
    #[allow(dead_code)]
    divergence_view: wgpu::TextureView,

    /// Pre-lit smoke field. After projection, a small compute pass walks
    /// every voxel and writes `(rgb = lit smoke colour, a = density)` here.
    /// The volumetric raymarch samples this directly and skips its
    /// per-step lighting loop entirely. See `shaders/fluid3_lightbake.wgsl`.
    #[allow(dead_code)]
    lit_density: wgpu::Texture,
    lit_density_view: wgpu::TextureView,

    // Filtered sampler used by advect & volume render.
    linear_sampler: wgpu::Sampler,

    // Uniform buffers.
    fluid_uniforms_buf: wgpu::Buffer,
    injection_buf: wgpu::Buffer,
    cam_buf: wgpu::Buffer,

    // Compute pipelines.
    inject_pipeline: wgpu::ComputePipeline,
    advect_pipeline: wgpu::ComputePipeline,
    divergence_pipeline: wgpu::ComputePipeline,
    jacobi_pipeline: wgpu::ComputePipeline,
    project_pipeline: wgpu::ComputePipeline,
    /// Per-voxel pre-lighting bake. Reads the post-project `vd[0]` and
    /// writes pre-lit colour into `lit_density`.
    lightbake_pipeline: wgpu::ComputePipeline,
    lightbake_layout: wgpu::BindGroupLayout,
    /// Built lazily by `rebuild_render_bind_group` because it references
    /// the renderer-owned `point_lights_buffer`.
    lightbake_bg: Option<wgpu::BindGroup>,

    // Compute bind groups.
    /// inject_bg: src=vd[0], dst=vd[1]
    inject_bg: wgpu::BindGroup,
    /// advect_bg: src=vd[1], dst=vd[0]
    advect_bg: wgpu::BindGroup,
    /// divergence_bg: src=vd[0]
    divergence_bg: wgpu::BindGroup,
    /// jacobi_bgs[0]: read p[0] write p[1]
    /// jacobi_bgs[1]: read p[1] write p[0]
    jacobi_bgs: [wgpu::BindGroup; 2],
    /// project_bg: read vd[0]+pressure[0], write vd[1]
    project_bg: wgpu::BindGroup,

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

    // Offscreen render target. Allocated lazily by `set_detail` so it can
    // be reshaped on resize and on detail-level changes.
    offscreen_texture: Option<wgpu::Texture>,
    offscreen_view: Option<wgpu::TextureView>,
    /// Width of the offscreen target in pixels (`screen_w / detail.divisor()`).
    offscreen_w: u32,
    /// Height of the offscreen target in pixels.
    offscreen_h: u32,
    /// Detail level the current offscreen target was allocated for.
    /// `None` until the first `set_detail` call.
    current_detail: Option<SmokeDetail>,

    // Pending impulses for this frame.
    impulses: Vec<Impulse>,

    // Last camera/bounds set for this frame.
    grid_min: Vec3,
    grid_max: Vec3,

    // Current frame size (for the depth read in the render pass).
    screen_w: f32,
    screen_h: f32,
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
        let _ = surface_format; // surface format is now consumed by the composite pipeline below.
        // ── 3D textures ────────────────────────────────────────────────
        let extent3d = wgpu::Extent3d {
            width: GRID_X,
            height: GRID_Y,
            depth_or_array_layers: GRID_Z,
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
        let pressure = [
            make_3d("fluid3-p-a", wgpu::TextureFormat::R32Float),
            make_3d("fluid3-p-b", wgpu::TextureFormat::R32Float),
        ];
        let divergence = make_3d("fluid3-div", wgpu::TextureFormat::R32Float);
        // Pre-lit smoke field. Same dims/format as `vd` so the lightbake
        // compute can write through `texture_storage_3d<rgba16float, write>`
        // and the raymarch can sample it filtered.
        let lit_density = make_3d("fluid3-lit-density", wgpu::TextureFormat::Rgba16Float);

        let view_desc = wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D3),
            ..Default::default()
        };
        let vd_view = [vd[0].create_view(&view_desc), vd[1].create_view(&view_desc)];
        let lit_density_view = lit_density.create_view(&view_desc);
        let pressure_view = [
            pressure[0].create_view(&view_desc),
            pressure[1].create_view(&view_desc),
        ];
        let divergence_view = divergence.create_view(&view_desc);

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
        let fluid_uniforms_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fluid3-uniforms"),
            contents: bytemuck::bytes_of(&FluidUniformsGpu {
                grid_size: [GRID_X as f32, GRID_Y as f32, GRID_Z as f32, 0.0],
                grid_min: [-100.0, 0.0, -100.0, 0.0],
                grid_max: [100.0, 60.0, 100.0, 0.0],
                inv_extent: [1.0 / 200.0, 1.0 / 60.0, 1.0 / 200.0, 0.0],
                params: [1.0 / 60.0, 0.998, 0.99, 14.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let injection_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fluid3-injection"),
            contents: bytemuck::bytes_of(&InjectionParamsGpu {
                points: [InjectionPointGpu {
                    pos_radius: [0.0; 4],
                    vel_density: [0.0; 4],
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
                grid_min: [-100.0, 0.0, -100.0, 0.0],
                grid_max: [100.0, 60.0, 100.0, 0.0],
                params: [0.5, 36.0, 1.5, 0.1],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // ── Shader modules ─────────────────────────────────────────────
        let make_shader = |label: &str, src: &str| -> wgpu::ShaderModule {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(src.into()),
            })
        };
        let inject_shader = make_shader(
            "fluid3-inject",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/fluid3_inject.wgsl"
            )),
        );
        let advect_shader = make_shader(
            "fluid3-advect",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/fluid3_advect.wgsl"
            )),
        );
        let divergence_shader = make_shader(
            "fluid3-divergence",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/fluid3_divergence.wgsl"
            )),
        );
        let jacobi_shader = make_shader(
            "fluid3-jacobi",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/fluid3_jacobi.wgsl"
            )),
        );
        let project_shader = make_shader(
            "fluid3-project",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/fluid3_project.wgsl"
            )),
        );
        let volume_shader = make_shader(
            "fluid3-volume",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/fluid3_volume.wgsl"
            )),
        );
        let lightbake_shader = make_shader(
            "fluid3-lightbake",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/fluid3_lightbake.wgsl"
            )),
        );

        // ── Bind group layouts ─────────────────────────────────────────
        let inject_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid3-inject-bgl"),
            entries: &[
                bgl_uniform(0),
                bgl_uniform(1),
                bgl_tex3d_float(2),
                bgl_storage3d(3, wgpu::TextureFormat::Rgba16Float),
            ],
        });
        let advect_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid3-advect-bgl"),
            entries: &[
                bgl_uniform(0),
                bgl_tex3d_float(1),
                bgl_sampler(2),
                bgl_storage3d(3, wgpu::TextureFormat::Rgba16Float),
            ],
        });
        let divergence_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid3-div-bgl"),
            entries: &[
                bgl_uniform(0),
                bgl_tex3d_float(1),
                bgl_storage3d(2, wgpu::TextureFormat::R32Float),
                bgl_storage3d(3, wgpu::TextureFormat::R32Float),
            ],
        });
        let jacobi_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid3-jacobi-bgl"),
            entries: &[
                bgl_uniform(0),
                bgl_tex3d_unfiltered(1),
                bgl_tex3d_unfiltered(2),
                bgl_storage3d(3, wgpu::TextureFormat::R32Float),
            ],
        });
        let project_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid3-project-bgl"),
            entries: &[
                bgl_uniform(0),
                bgl_tex3d_float(1),
                bgl_tex3d_unfiltered(2),
                bgl_storage3d(3, wgpu::TextureFormat::Rgba16Float),
            ],
        });
        // Lightbake reads vd[0] (filterable so it can also be sampled by
        // the raymarch elsewhere — for the bake we use textureLoad which
        // doesn't care about filterability), writes lit_density as a
        // storage texture, and references the camera + lights uniforms.
        let lightbake_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid3-lightbake-bgl"),
            entries: &[
                bgl_uniform(0),
                bgl_tex3d_float(1),
                bgl_storage3d(2, wgpu::TextureFormat::Rgba16Float),
                bgl_uniform(3),
                bgl_uniform(4),
            ],
        });

        // ── Compute pipelines ──────────────────────────────────────────
        let make_compute = |label: &str,
                            shader: &wgpu::ShaderModule,
                            layout: &wgpu::BindGroupLayout|
         -> wgpu::ComputePipeline {
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pl),
                module: shader,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            })
        };
        let inject_pipeline = make_compute("fluid3-inject-pl", &inject_shader, &inject_layout);
        let advect_pipeline = make_compute("fluid3-advect-pl", &advect_shader, &advect_layout);
        let divergence_pipeline =
            make_compute("fluid3-div-pl", &divergence_shader, &divergence_layout);
        let jacobi_pipeline = make_compute("fluid3-jacobi-pl", &jacobi_shader, &jacobi_layout);
        let project_pipeline = make_compute("fluid3-project-pl", &project_shader, &project_layout);
        let lightbake_pipeline =
            make_compute("fluid3-lightbake-pl", &lightbake_shader, &lightbake_layout);

        // ── Compute bind groups (fixed src/dst orientations) ───────────
        let inject_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fluid3-inject-bg"),
            layout: &inject_layout,
            entries: &[
                bge(0, fluid_uniforms_buf.as_entire_binding()),
                bge(1, injection_buf.as_entire_binding()),
                bge(2, wgpu::BindingResource::TextureView(&vd_view[0])),
                bge(3, wgpu::BindingResource::TextureView(&vd_view[1])),
            ],
        });
        let advect_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fluid3-advect-bg"),
            layout: &advect_layout,
            entries: &[
                bge(0, fluid_uniforms_buf.as_entire_binding()),
                bge(1, wgpu::BindingResource::TextureView(&vd_view[1])),
                bge(2, wgpu::BindingResource::Sampler(&linear_sampler)),
                bge(3, wgpu::BindingResource::TextureView(&vd_view[0])),
            ],
        });
        let divergence_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fluid3-div-bg"),
            layout: &divergence_layout,
            entries: &[
                bge(0, fluid_uniforms_buf.as_entire_binding()),
                bge(1, wgpu::BindingResource::TextureView(&vd_view[0])),
                bge(2, wgpu::BindingResource::TextureView(&divergence_view)),
                bge(3, wgpu::BindingResource::TextureView(&pressure_view[0])),
            ],
        });
        let jacobi_bgs = [
            // read p[0], write p[1]
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fluid3-jacobi-bg-0"),
                layout: &jacobi_layout,
                entries: &[
                    bge(0, fluid_uniforms_buf.as_entire_binding()),
                    bge(1, wgpu::BindingResource::TextureView(&pressure_view[0])),
                    bge(2, wgpu::BindingResource::TextureView(&divergence_view)),
                    bge(3, wgpu::BindingResource::TextureView(&pressure_view[1])),
                ],
            }),
            // read p[1], write p[0]
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fluid3-jacobi-bg-1"),
                layout: &jacobi_layout,
                entries: &[
                    bge(0, fluid_uniforms_buf.as_entire_binding()),
                    bge(1, wgpu::BindingResource::TextureView(&pressure_view[1])),
                    bge(2, wgpu::BindingResource::TextureView(&divergence_view)),
                    bge(3, wgpu::BindingResource::TextureView(&pressure_view[0])),
                ],
            }),
        ];
        let project_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fluid3-project-bg"),
            layout: &project_layout,
            entries: &[
                bge(0, fluid_uniforms_buf.as_entire_binding()),
                bge(1, wgpu::BindingResource::TextureView(&vd_view[0])),
                bge(2, wgpu::BindingResource::TextureView(&pressure_view[0])),
                bge(3, wgpu::BindingResource::TextureView(&vd_view[1])),
            ],
        });

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
                // 1: density 3D texture (filterable)
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
                // 3: depth texture (non-filterable depth)
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
                // (Lighting moved to a per-voxel compute bake — the
                // raymarch no longer reads the point lights buffer.)
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
                // Render into the offscreen Rgba16Float target. The pass
                // clears the target each frame, so blend = REPLACE: the
                // shader output IS the colour. Premultiplied alpha is
                // applied by the composite pass when blending into the
                // swap chain.
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
            // Offscreen target has no depth attachment — depth occlusion
            // is handled inside the shader by sampling `depth_tex`.
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // ── Composite pipeline ──────────────────────────────────────────
        // Samples the offscreen target with bilinear filtering and blends
        // it onto the swap chain with PREMULTIPLIED_ALPHA. Lives entirely
        // in `fluid3_composite.wgsl`.
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
                // 0: offscreen colour texture (filterable float)
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
                // 1: bilinear sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
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
            // Same matched-attachment trick as the original render pass: the
            // composite runs inside the existing post-smoke pass which has a
            // depth attachment, so we declare a depth state with always-pass
            // and no writes.
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
            pressure,
            pressure_view,
            divergence,
            divergence_view,
            lit_density,
            lit_density_view,
            linear_sampler,
            fluid_uniforms_buf,
            injection_buf,
            cam_buf,
            inject_pipeline,
            advect_pipeline,
            divergence_pipeline,
            jacobi_pipeline,
            project_pipeline,
            lightbake_pipeline,
            lightbake_layout,
            lightbake_bg: None,
            inject_bg,
            advect_bg,
            divergence_bg,
            jacobi_bgs,
            project_bg,
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
            grid_min: Vec3::new(-100.0, 0.0, -100.0),
            grid_max: Vec3::new(100.0, 60.0, 100.0),
            screen_w,
            screen_h,
        }
    }

    pub fn update_screen_size(&mut self, w: f32, h: f32) {
        self.screen_w = w;
        self.screen_h = h;
        // Force the next set_detail() call to reallocate at the new
        // resolution even if the detail level is unchanged.
        self.current_detail = None;
    }

    /// (Re)allocate the offscreen render target for `detail` at the current
    /// screen resolution. Cheap to call every frame — does nothing when the
    /// existing target already matches the requested size and detail level.
    /// Also rebuilds the composite bind group so it points at the new view.
    pub fn set_detail(&mut self, device: &wgpu::Device, detail: SmokeDetail) {
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
        // Drop the previous allocation explicitly so the GPU memory is
        // freed before the new one is created — at native res this matters
        // (~16 MB at 1080p Rgba16F).
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
            ],
        }));
        self.offscreen_texture = Some(texture);
        self.offscreen_view = Some(view);
        self.offscreen_w = target_w;
        self.offscreen_h = target_h;
        self.current_detail = Some(detail);
        log::info!(
            "[FluidSim] smoke offscreen target → {}x{} ({})",
            target_w,
            target_h,
            detail.label()
        );
    }

    /// Project the smoke grid AABB into the offscreen target's pixel space
    /// and return a bounding rect (x, y, w, h). Used as a scissor on the
    /// offscreen pass so the raymarch only runs on pixels the plume can
    /// actually reach.
    ///
    /// Returns `None` when the AABB is fully behind the camera; returns the
    /// full target rect when corners straddle the camera plane (cheap
    /// fallback rather than implementing proper homogeneous clipping).
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
            // NDC y up → pixel y down.
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
            // Some corners straddle the camera plane — fall back to full rect.
            return Some((0, 0, target_w, target_h));
        }
        // Add a 1-pixel safety margin so bilinear sampling at composite
        // time doesn't bleed in zeros from outside the rasterised region.
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

    /// Set the smoke volume bounds for the current frame.
    pub fn set_grid_bounds(&mut self, grid_min: Vec3, grid_max: Vec3) {
        self.grid_min = grid_min;
        self.grid_max = grid_max;
    }

    /// Queue a world-space impulse for the current frame.
    pub fn inject_impulse(&mut self, world_pos: Vec3, world_vel: Vec3, radius: f32, density: f32) {
        if self.impulses.len() < MAX_INJECTIONS {
            self.impulses.push(Impulse {
                world_pos,
                world_vel,
                radius,
                density,
            });
        }
    }

    /// Run one simulation step. Call before beginning the render pass.
    pub fn step(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        dt: f32,
        intensity: SmokeIntensity,
    ) {
        let (density_dis, vel_dis, buoyancy, mut jacobi_iters) = match intensity {
            SmokeIntensity::Off => return,
            SmokeIntensity::Subtle => (0.992, 0.985, 8.0, 12u32),
            SmokeIntensity::Strong => (0.996, 0.99, 14.0, 18),
            SmokeIntensity::OverTheTop => (0.998, 0.993, 22.0, 24),
        };
        // Force even iteration count so the final result lands in pressure[0].
        if jacobi_iters % 2 != 0 {
            jacobi_iters += 1;
        }

        // ── Upload uniforms ────────────────────────────────────────────
        let extent = self.grid_max - self.grid_min;
        let inv_extent = Vec3::new(
            1.0 / extent.x.max(1e-3),
            1.0 / extent.y.max(1e-3),
            1.0 / extent.z.max(1e-3),
        );
        let dt_clamped = dt.min(0.05);
        // Make per-step density/velocity dissipation framerate-independent.
        // The intensity table above lists the *per-frame at 60 Hz reference*
        // multiplier; the shader applies whatever value we upload once per
        // step, so without this rescale a low frame rate causes far fewer
        // multiplies per wall second and the smoke lingers visibly longer
        // (manifests as the post-deal opening curtain "sometimes taking
        // forever" to clear when the round-start work hitches the FPS).
        // pow(c, dt*60) is the closed-form continuous decay equivalent and
        // reproduces the old behavior exactly when dt = 1/60.
        let dt_scale = dt_clamped * 60.0;
        let density_dis_step = (density_dis as f32).powf(dt_scale);
        let vel_dis_step = (vel_dis as f32).powf(dt_scale);
        queue.write_buffer(
            &self.fluid_uniforms_buf,
            0,
            bytemuck::bytes_of(&FluidUniformsGpu {
                grid_size: [GRID_X as f32, GRID_Y as f32, GRID_Z as f32, 0.0],
                grid_min: [self.grid_min.x, self.grid_min.y, self.grid_min.z, 0.0],
                grid_max: [self.grid_max.x, self.grid_max.y, self.grid_max.z, 0.0],
                inv_extent: [inv_extent.x, inv_extent.y, inv_extent.z, 0.0],
                params: [dt_clamped, density_dis_step, vel_dis_step, buoyancy],
            }),
        );

        // ── Pack impulses ──────────────────────────────────────────────
        let mut injection = InjectionParamsGpu {
            points: [InjectionPointGpu {
                pos_radius: [0.0; 4],
                vel_density: [0.0; 4],
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
                vel_density: [
                    imp.world_vel.x,
                    imp.world_vel.y,
                    imp.world_vel.z,
                    imp.density,
                ],
            };
        }
        self.impulses.clear();
        queue.write_buffer(&self.injection_buf, 0, bytemuck::bytes_of(&injection));

        let wg_x = (GRID_X + WG - 1) / WG;
        let wg_y = (GRID_Y + WG - 1) / WG;
        let wg_z = (GRID_Z + WG - 1) / WG;

        // 1. Inject: vd[0] → vd[1]
        {
            let mut p = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fluid3-inject"),
                timestamp_writes: None,
            });
            p.set_pipeline(&self.inject_pipeline);
            p.set_bind_group(0, &self.inject_bg, &[]);
            p.dispatch_workgroups(wg_x, wg_y, wg_z);
        }

        // 2. Advect: vd[1] → vd[0]
        {
            let mut p = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fluid3-advect"),
                timestamp_writes: None,
            });
            p.set_pipeline(&self.advect_pipeline);
            p.set_bind_group(0, &self.advect_bg, &[]);
            p.dispatch_workgroups(wg_x, wg_y, wg_z);
        }

        // 3. Divergence (reads vd[0], writes div + clears pressure[0]).
        {
            let mut p = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fluid3-divergence"),
                timestamp_writes: None,
            });
            p.set_pipeline(&self.divergence_pipeline);
            p.set_bind_group(0, &self.divergence_bg, &[]);
            p.dispatch_workgroups(wg_x, wg_y, wg_z);
        }

        // 4. Jacobi N iterations (even count → result lands in pressure[0]).
        for j in 0..jacobi_iters {
            let bg_idx = (j % 2) as usize; // 0: read p[0]→p[1]; 1: read p[1]→p[0]
            let mut p = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fluid3-jacobi"),
                timestamp_writes: None,
            });
            p.set_pipeline(&self.jacobi_pipeline);
            p.set_bind_group(0, &self.jacobi_bgs[bg_idx], &[]);
            p.dispatch_workgroups(wg_x, wg_y, wg_z);
        }

        // 5. Projection: vd[0] - ∇p → vd[1]
        {
            let mut p = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fluid3-project"),
                timestamp_writes: None,
            });
            p.set_pipeline(&self.project_pipeline);
            p.set_bind_group(0, &self.project_bg, &[]);
            p.dispatch_workgroups(wg_x, wg_y, wg_z);
        }

        // 6. Copy vd[1] → vd[0] so the next frame's inject reads the latest state.
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.vd[1],
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &self.vd[0],
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: GRID_X,
                height: GRID_Y,
                depth_or_array_layers: GRID_Z,
            },
        );

        // 7. Lightbake: walk every voxel of the freshly-projected vd[0],
        // evaluate every candle point light at the voxel center, and
        // write the pre-lit smoke colour into `lit_density`. Skipped on
        // the very first frame (or after a depth recreate) until
        // `rebuild_render_bind_group` has been called with the renderer
        // -owned point lights buffer.
        if let Some(ref bg) = self.lightbake_bg {
            let mut p = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fluid3-lightbake"),
                timestamp_writes: None,
            });
            p.set_pipeline(&self.lightbake_pipeline);
            p.set_bind_group(0, bg, &[]);
            p.dispatch_workgroups(wg_x, wg_y, wg_z);
        }
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
        // Volume raymarch reads pre-lit smoke from `lit_density_view` —
        // the per-step lighting loop has been hoisted into a per-voxel
        // compute bake (see `shaders/fluid3_lightbake.wgsl`), so the
        // lights buffer no longer appears in this layout.
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
            ],
        }));
        // Lightbake reads vd[0], writes lit_density, and references the
        // camera + point lights uniforms. Built here because this is the
        // only entry point that has the renderer-owned lights buffer.
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

    /// Upload the per-frame volume camera uniform. Call after `set_camera` and
    /// before issuing the render pass.
    pub fn upload_camera_uniform(
        &self,
        queue: &wgpu::Queue,
        view_proj: Mat4,
        cam_pos: Vec3,
        intensity: SmokeIntensity,
    ) {
        let inv_vp = view_proj.inverse();
        let (max_alpha, step_count, light_strength, ambient) = match intensity {
            SmokeIntensity::Off => (0.0, 8.0, 0.0, 0.0),
            SmokeIntensity::Subtle => (0.45, 28.0, 1.0, 0.08),
            SmokeIntensity::Strong => (0.65, 40.0, 1.3, 0.11),
            SmokeIntensity::OverTheTop => (0.85, 56.0, 1.6, 0.14),
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
                params: [max_alpha, step_count, light_strength, ambient],
            }),
        );
    }

    /// Render the volumetric raymarch into the offscreen smoke target. Begins
    /// its own render pass on `encoder`, scissoring to the smoke AABB's
    /// projected screen rect when one is supplied so the fragment shader
    /// only runs on pixels the plume can possibly cover.
    ///
    /// `globals_bind_group` is the renderer-wide group 0 (screen + time +
    /// gamma uniform) — the volume shader needs it to convert NDC into
    /// depth-texture pixel coordinates. Caller is responsible for ensuring
    /// `set_detail` has been called at least once before this method.
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
                    // Cleared to fully transparent so pixels outside the
                    // scissor stay invisible during composite.
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
            // Guard against zero-size or out-of-bounds rects which wgpu
            // rejects as a validation error.
            if w > 0 && h > 0 && x + w <= self.offscreen_w && y + h <= self.offscreen_h {
                pass.set_scissor_rect(x, y, w, h);
            }
        }
        pass.set_pipeline(&self.render_pipeline);
        pass.set_bind_group(0, globals_bind_group, &[]);
        pass.set_bind_group(1, render_bg, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Composite the offscreen smoke target onto the swap chain. Called from
    /// inside the existing post-smoke render pass at the location of the
    /// `RenderOp::FluidSmoke` marker — the existing pass already has the
    /// right load ops on color/depth, so the composite is just a fullscreen
    /// triangle with premultiplied-alpha blending.
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
