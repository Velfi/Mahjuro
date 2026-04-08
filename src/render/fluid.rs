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

use crate::persistence::SmokeIntensity;

// ──────────────────────────────────────────────────────────────────────
// Grid configuration
// ──────────────────────────────────────────────────────────────────────

const GRID_X: u32 = 96;
const GRID_Y: u32 = 48;
const GRID_Z: u32 = 96;
const WG: u32 = 4;

const MAX_INJECTIONS: usize = 32;

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

    // Volume render pipeline (fullscreen triangle, alpha-blended).
    render_pipeline: wgpu::RenderPipeline,
    render_layout: wgpu::BindGroupLayout,
    render_bg: Option<wgpu::BindGroup>,

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

        let view_desc = wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D3),
            ..Default::default()
        };
        let vd_view = [
            vd[0].create_view(&view_desc),
            vd[1].create_view(&view_desc),
        ];
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
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/fluid3_inject.wgsl")),
        );
        let advect_shader = make_shader(
            "fluid3-advect",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/fluid3_advect.wgsl")),
        );
        let divergence_shader = make_shader(
            "fluid3-divergence",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/fluid3_divergence.wgsl")),
        );
        let jacobi_shader = make_shader(
            "fluid3-jacobi",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/fluid3_jacobi.wgsl")),
        );
        let project_shader = make_shader(
            "fluid3-project",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/fluid3_project.wgsl")),
        );
        let volume_shader = make_shader(
            "fluid3-volume",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/fluid3_volume.wgsl")),
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
                // 4: point lights uniform
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
            pressure,
            pressure_view,
            divergence,
            divergence_view,
            linear_sampler,
            fluid_uniforms_buf,
            injection_buf,
            cam_buf,
            inject_pipeline,
            advect_pipeline,
            divergence_pipeline,
            jacobi_pipeline,
            project_pipeline,
            inject_bg,
            advect_bg,
            divergence_bg,
            jacobi_bgs,
            project_bg,
            render_pipeline,
            render_layout,
            render_bg: None,
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
    }

    /// Set the smoke volume bounds for the current frame.
    pub fn set_grid_bounds(&mut self, grid_min: Vec3, grid_max: Vec3) {
        self.grid_min = grid_min;
        self.grid_max = grid_max;
    }

    /// Queue a world-space impulse for the current frame.
    pub fn inject_impulse(
        &mut self,
        world_pos: Vec3,
        world_vel: Vec3,
        radius: f32,
        density: f32,
    ) {
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
        queue.write_buffer(
            &self.fluid_uniforms_buf,
            0,
            bytemuck::bytes_of(&FluidUniformsGpu {
                grid_size: [GRID_X as f32, GRID_Y as f32, GRID_Z as f32, 0.0],
                grid_min: [self.grid_min.x, self.grid_min.y, self.grid_min.z, 0.0],
                grid_max: [self.grid_max.x, self.grid_max.y, self.grid_max.z, 0.0],
                inv_extent: [inv_extent.x, inv_extent.y, inv_extent.z, 0.0],
                params: [dt_clamped, density_dis, vel_dis, buoyancy],
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
                pos_radius: [imp.world_pos.x, imp.world_pos.y, imp.world_pos.z, imp.radius],
                vel_density: [imp.world_vel.x, imp.world_vel.y, imp.world_vel.z, imp.density],
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
    }

    /// Build (or rebuild) the render bind group for the current depth view.
    /// The depth texture is recreated on resize, so call this whenever the
    /// depth view changes.
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
                bge(1, wgpu::BindingResource::TextureView(&self.vd_view[0])),
                bge(2, wgpu::BindingResource::Sampler(&self.linear_sampler)),
                bge(3, wgpu::BindingResource::TextureView(depth_view)),
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
            SmokeIntensity::Subtle => (0.45, 28.0, 1.4, 0.10),
            SmokeIntensity::Strong => (0.65, 40.0, 1.8, 0.14),
            SmokeIntensity::OverTheTop => (0.85, 56.0, 2.2, 0.18),
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

    /// Draw the volumetric smoke. The render bind group must have been built
    /// for the current frame's depth view.
    pub fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        globals_bind_group: &wgpu::BindGroup,
    ) {
        let Some(ref bg) = self.render_bg else {
            return;
        };
        pass.set_pipeline(&self.render_pipeline);
        pass.set_bind_group(0, globals_bind_group, &[]);
        pass.set_bind_group(1, bg, &[]);
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
