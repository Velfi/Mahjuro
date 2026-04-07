//! GPU-accelerated 2D Eulerian fluid simulation for atmospheric smoke effects.
//!
//! Runs a Navier-Stokes simulation on a 240x150 grid via compute shaders.
//! Mouse/tile interactions inject colored velocity+density. The density field
//! renders as an alpha-blended overlay between tiles and UI elements.

use wgpu::util::DeviceExt;

use crate::persistence::SmokeIntensity;

const GRID_W: u32 = 240;
const GRID_H: u32 = 150;
const GRID_CELLS: u32 = GRID_W * GRID_H;

/// A single injection impulse queued for the current frame.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct InjectionPointGpu {
    pos_radius: [f32; 4],  // (grid_x, grid_y, radius, strength)
    vel_density: [f32; 4], // (vel_x, vel_y, _, _)
    color_pad: [f32; 4],   // (density_r, density_g, density_b, _)
}

const MAX_INJECTIONS: usize = 8;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct InjectionParamsGpu {
    points: [InjectionPointGpu; MAX_INJECTIONS],
    active_count: u32,
    _pad: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FluidUniformsGpu {
    grid_w: f32,
    grid_h: f32,
    inv_grid_w: f32,
    inv_grid_h: f32,
    dt: f32,
    density_dissipation: f32,
    velocity_dissipation: f32,
    _pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PressureParamsGpu {
    mode: u32,
    _pad: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FluidRenderParamsGpu {
    max_alpha: f32,
    _pad: [f32; 3],
}

/// Queued injection impulse (screen-space coordinates).
pub struct Impulse {
    pub screen_x: f32,
    pub screen_y: f32,
    pub radius: f32,
    pub vel_x: f32,
    pub vel_y: f32,
    pub color: [f32; 3],
    pub strength: f32,
}

pub struct FluidSim {
    // Simulation storage buffers (ping-pong pairs).
    velocity_x: [wgpu::Buffer; 2],
    velocity_y: [wgpu::Buffer; 2],
    density_r: [wgpu::Buffer; 2],
    density_g: [wgpu::Buffer; 2],
    density_b: [wgpu::Buffer; 2],

    // Uniform buffers.
    fluid_uniforms_buf: wgpu::Buffer,
    injection_buf: wgpu::Buffer,
    render_params_buf: wgpu::Buffer,

    // Density texture for rendering.
    #[allow(dead_code)]
    density_texture: wgpu::Texture,
    #[allow(dead_code)]
    density_view: wgpu::TextureView,

    // Compute pipelines.
    inject_pipeline: wgpu::ComputePipeline,
    advect_pipeline: wgpu::ComputePipeline,
    pressure_pipeline: wgpu::ComputePipeline,
    density_to_tex_pipeline: wgpu::ComputePipeline,

    // Bind groups for compute passes.
    inject_bind_group: wgpu::BindGroup,
    // Advection bind groups: [0] reads A writes B, [1] reads B writes A.
    advect_bind_groups: [wgpu::BindGroup; 2],
    // Pressure bind groups:
    // [0] = divergence (mode 0), vel set 1, writes pressure[0] + divergence
    // [1] = jacobi (mode 1), reads pressure[0] writes pressure[1]
    // [2] = jacobi (mode 1), reads pressure[1] writes pressure[0]
    // [3] = projection (mode 2), reads final pressure, vel set 1
    pressure_bind_groups: [wgpu::BindGroup; 4],
    // Density-to-texture bind group — reads density set 0, writes texture.
    density_to_tex_bind_group: wgpu::BindGroup,

    // Render pipeline.
    render_pipeline: wgpu::RenderPipeline,
    render_bind_group: wgpu::BindGroup,

    // Pending impulses for this frame.
    impulses: Vec<Impulse>,

    // Screen dimensions for coordinate conversion.
    screen_w: f32,
    screen_h: f32,
}

impl FluidSim {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        globals_layout: &wgpu::BindGroupLayout,
        surface_format: wgpu::TextureFormat,
        screen_w: f32,
        screen_h: f32,
    ) -> Self {
        // --- Storage buffers ---
        let zeros = vec![0.0f32; GRID_CELLS as usize];
        let zero_bytes = bytemuck::cast_slice(&zeros);

        let make_buf = |label: &str| -> wgpu::Buffer {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: zero_bytes,
                usage: wgpu::BufferUsages::STORAGE
                    | wgpu::BufferUsages::COPY_SRC
                    | wgpu::BufferUsages::COPY_DST,
            })
        };

        let velocity_x = [make_buf("fluid-vx-a"), make_buf("fluid-vx-b")];
        let velocity_y = [make_buf("fluid-vy-a"), make_buf("fluid-vy-b")];
        let density_r = [make_buf("fluid-dr-a"), make_buf("fluid-dr-b")];
        let density_g = [make_buf("fluid-dg-a"), make_buf("fluid-dg-b")];
        let density_b = [make_buf("fluid-db-a"), make_buf("fluid-db-b")];
        let pressure = [make_buf("fluid-p-a"), make_buf("fluid-p-b")];
        let divergence = make_buf("fluid-div");

        // --- Uniform buffers ---
        let fluid_uniforms = FluidUniformsGpu {
            grid_w: GRID_W as f32,
            grid_h: GRID_H as f32,
            inv_grid_w: 1.0 / GRID_W as f32,
            inv_grid_h: 1.0 / GRID_H as f32,
            dt: 1.0 / 60.0,
            density_dissipation: 0.996,
            velocity_dissipation: 0.99,
            _pad: 0.0,
        };
        let fluid_uniforms_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fluid-uniforms"),
            contents: bytemuck::bytes_of(&fluid_uniforms),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let injection_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fluid-injection"),
            contents: bytemuck::bytes_of(&InjectionParamsGpu {
                points: [InjectionPointGpu {
                    pos_radius: [0.0; 4],
                    vel_density: [0.0; 4],
                    color_pad: [0.0; 4],
                }; MAX_INJECTIONS],
                active_count: 0,
                _pad: [0; 3],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let pressure_params_bufs = [0u32, 1, 2].map(|mode| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(match mode {
                    0 => "fluid-pp-divergence",
                    1 => "fluid-pp-jacobi",
                    _ => "fluid-pp-projection",
                }),
                contents: bytemuck::bytes_of(&PressureParamsGpu { mode, _pad: [0; 3] }),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        });

        let render_params_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fluid-render-params"),
            contents: bytemuck::bytes_of(&FluidRenderParamsGpu {
                max_alpha: 0.5,
                _pad: [0.0; 3],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // --- Density texture for rendering ---
        let density_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("fluid-density-tex"),
            size: wgpu::Extent3d {
                width: GRID_W,
                height: GRID_H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });
        let density_view = density_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // No staging buffer needed — density-to-texture is done via compute shader.

        // --- Compute shader modules ---
        let inject_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fluid-inject"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/shaders/fluid_inject.wgsl"
                ))
                .into(),
            ),
        });
        let advect_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fluid-advect"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/shaders/fluid_advect.wgsl"
                ))
                .into(),
            ),
        });
        let pressure_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fluid-pressure"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/shaders/fluid_pressure.wgsl"
                ))
                .into(),
            ),
        });

        // --- Inject bind group layout ---
        let inject_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid-inject-layout"),
            entries: &[
                // 0: fluid uniforms
                bgl_uniform(0, wgpu::ShaderStages::COMPUTE),
                // 1: injection params
                bgl_uniform(1, wgpu::ShaderStages::COMPUTE),
                // 2-6: velocity_x, velocity_y, density_r, density_g, density_b (read_write)
                bgl_storage_rw(2, wgpu::ShaderStages::COMPUTE),
                bgl_storage_rw(3, wgpu::ShaderStages::COMPUTE),
                bgl_storage_rw(4, wgpu::ShaderStages::COMPUTE),
                bgl_storage_rw(5, wgpu::ShaderStages::COMPUTE),
                bgl_storage_rw(6, wgpu::ShaderStages::COMPUTE),
            ],
        });

        let inject_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fluid-inject-pl"),
            bind_group_layouts: &[Some(&inject_layout)],
            immediate_size: 0,
        });
        let inject_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fluid-inject-pipeline"),
            layout: Some(&inject_pl),
            module: &inject_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // Inject bind group always operates on buffer set A (current=0 start).
        // We re-create bind groups when we need to swap, OR we always inject into
        // the "current" set. For simplicity, we create two inject bind groups.
        let inject_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fluid-inject-bg"),
            layout: &inject_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: fluid_uniforms_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: injection_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: velocity_x[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: velocity_y[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: density_r[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: density_g[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: density_b[0].as_entire_binding(),
                },
            ],
        });

        // --- Advection bind group layout ---
        let advect_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid-advect-layout"),
            entries: &[
                // 0: fluid uniforms
                bgl_uniform(0, wgpu::ShaderStages::COMPUTE),
                // 1-5: source (read-only)
                bgl_storage_ro(1, wgpu::ShaderStages::COMPUTE),
                bgl_storage_ro(2, wgpu::ShaderStages::COMPUTE),
                bgl_storage_ro(3, wgpu::ShaderStages::COMPUTE),
                bgl_storage_ro(4, wgpu::ShaderStages::COMPUTE),
                bgl_storage_ro(5, wgpu::ShaderStages::COMPUTE),
                // 6-10: destination (read_write)
                bgl_storage_rw(6, wgpu::ShaderStages::COMPUTE),
                bgl_storage_rw(7, wgpu::ShaderStages::COMPUTE),
                bgl_storage_rw(8, wgpu::ShaderStages::COMPUTE),
                bgl_storage_rw(9, wgpu::ShaderStages::COMPUTE),
                bgl_storage_rw(10, wgpu::ShaderStages::COMPUTE),
            ],
        });

        let advect_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fluid-advect-pl"),
            bind_group_layouts: &[Some(&advect_layout)],
            immediate_size: 0,
        });
        let advect_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fluid-advect-pipeline"),
            layout: Some(&advect_pl),
            module: &advect_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // Advection bind groups: [0] reads set 0, writes set 1. [1] reads set 1, writes set 0.
        let make_advect_bg = |label: &str, src: usize, dst: usize| -> wgpu::BindGroup {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &advect_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: fluid_uniforms_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: velocity_x[src].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: velocity_y[src].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: density_r[src].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: density_g[src].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: density_b[src].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: velocity_x[dst].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 7,
                        resource: velocity_y[dst].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 8,
                        resource: density_r[dst].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 9,
                        resource: density_g[dst].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: density_b[dst].as_entire_binding(),
                    },
                ],
            })
        };
        let advect_bind_groups = [
            make_advect_bg("fluid-advect-bg-0", 0, 1),
            make_advect_bg("fluid-advect-bg-1", 1, 0),
        ];

        // --- Pressure bind group layout ---
        let pressure_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid-pressure-layout"),
            entries: &[
                // 0: fluid uniforms
                bgl_uniform(0, wgpu::ShaderStages::COMPUTE),
                // 1: pressure params
                bgl_uniform(1, wgpu::ShaderStages::COMPUTE),
                // 2-3: velocity_x, velocity_y (read_write for projection)
                bgl_storage_rw(2, wgpu::ShaderStages::COMPUTE),
                bgl_storage_rw(3, wgpu::ShaderStages::COMPUTE),
                // 4: pressure source (read)
                bgl_storage_ro(4, wgpu::ShaderStages::COMPUTE),
                // 5: pressure dest (write)
                bgl_storage_rw(5, wgpu::ShaderStages::COMPUTE),
                // 6: divergence (read_write)
                bgl_storage_rw(6, wgpu::ShaderStages::COMPUTE),
            ],
        });

        let pressure_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fluid-pressure-pl"),
            bind_group_layouts: &[Some(&pressure_layout)],
            immediate_size: 0,
        });
        let pressure_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fluid-pressure-pipeline"),
            layout: Some(&pressure_pl),
            module: &pressure_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // Pressure bind groups: [0] reads pressure[0] writes pressure[1], [1] vice versa.
        // After advection, the result is in the buffer set indicated by `1 - current`.
        // We use velocity from that destination set for divergence/projection.
        let make_pressure_bg = |label: &str,
                                mode: usize,
                                vel_set: usize,
                                p_src: usize,
                                p_dst: usize|
         -> wgpu::BindGroup {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &pressure_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: fluid_uniforms_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: pressure_params_bufs[mode].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: velocity_x[vel_set].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: velocity_y[vel_set].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: pressure[p_src].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: pressure[p_dst].as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 6,
                        resource: divergence.as_entire_binding(),
                    },
                ],
            })
        };
        // Pressure bind groups for each mode:
        // [0] divergence: mode 0, vel set 1, writes pressure[0] + divergence
        // [1] jacobi A→B: mode 1, reads pressure[0] writes pressure[1]
        // [2] jacobi B→A: mode 1, reads pressure[1] writes pressure[0]
        // [3] projection: mode 2, reads final pressure (determined at dispatch time)
        let pressure_bind_groups = [
            make_pressure_bg("fluid-p-div", 0, 1, 1, 0), // divergence (p_src=1 to avoid aliasing p_dst=0)
            make_pressure_bg("fluid-p-jac-0", 1, 1, 0, 1), // jacobi: read p[0], write p[1]
            make_pressure_bg("fluid-p-jac-1", 1, 1, 1, 0), // jacobi: read p[1], write p[0]
            make_pressure_bg("fluid-p-proj", 2, 1, 0, 1), // projection (p_src doesn't matter much)
        ];

        // --- Density-to-texture compute pipeline ---
        let d2t_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fluid-d2t"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/shaders/fluid_density_to_tex.wgsl"
                ))
                .into(),
            ),
        });

        let d2t_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid-d2t-layout"),
            entries: &[
                bgl_uniform(0, wgpu::ShaderStages::COMPUTE),
                bgl_storage_ro(1, wgpu::ShaderStages::COMPUTE),
                bgl_storage_ro(2, wgpu::ShaderStages::COMPUTE),
                bgl_storage_ro(3, wgpu::ShaderStages::COMPUTE),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });

        let d2t_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fluid-d2t-pl"),
            bind_group_layouts: &[Some(&d2t_layout)],
            immediate_size: 0,
        });

        let density_to_tex_pipeline =
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("fluid-d2t-pipeline"),
                layout: Some(&d2t_pl),
                module: &d2t_shader,
                entry_point: Some("main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                cache: None,
            });

        let density_to_tex_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fluid-d2t-bg"),
            layout: &d2t_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: fluid_uniforms_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: density_r[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: density_g[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: density_b[0].as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&density_view),
                },
            ],
        });

        // --- Render pipeline ---
        let render_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fluid-render-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/shaders/fluid_render.wgsl"
                ))
                .into(),
            ),
        });

        let density_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("fluid-density-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let render_bg_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fluid-render-bg-layout"),
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
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let render_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fluid-render-pl"),
            bind_group_layouts: &[Some(globals_layout), Some(&render_bg_layout)],
            immediate_size: 0,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            }],
        };

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<super::wgpu_renderer::GpuInstance>()
                as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        };

        let depth_ui = wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fluid-render-pipeline"),
            layout: Some(&render_layout),
            vertex: wgpu::VertexState {
                module: &render_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout, instance_layout],
            },
            fragment: Some(wgpu::FragmentState {
                module: &render_shader,
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
            depth_stencil: Some(depth_ui),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("fluid-render-bg"),
            layout: &render_bg_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&density_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&density_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: render_params_buf.as_entire_binding(),
                },
            ],
        });

        let _ = queue; // used implicitly for initial uploads via create_buffer_init

        Self {
            velocity_x,
            velocity_y,
            density_r,
            density_g,
            density_b,
            fluid_uniforms_buf,
            injection_buf,
            render_params_buf,
            density_texture,
            density_view,
            inject_pipeline,
            advect_pipeline,
            pressure_pipeline,
            density_to_tex_pipeline,
            inject_bind_group,
            advect_bind_groups,
            pressure_bind_groups,
            density_to_tex_bind_group,
            render_pipeline,
            render_bind_group,
            impulses: Vec::new(),
            screen_w,
            screen_h,
        }
    }

    /// Queue an impulse for the current frame (screen-space coordinates).
    pub fn inject_impulse(
        &mut self,
        screen_x: f32,
        screen_y: f32,
        radius: f32,
        vel_x: f32,
        vel_y: f32,
        color: [f32; 3],
        strength: f32,
    ) {
        if self.impulses.len() < MAX_INJECTIONS {
            self.impulses.push(Impulse {
                screen_x,
                screen_y,
                radius,
                vel_x,
                vel_y,
                color,
                strength,
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
        let (density_dissipation, velocity_dissipation, inject_mult, max_alpha, jacobi_iters) =
            match intensity {
                SmokeIntensity::Off => return,
                SmokeIntensity::Subtle => (0.992, 0.99, 0.5, 0.25, 14u32),
                SmokeIntensity::Strong => (0.996, 0.99, 1.0, 0.5, 20),
                SmokeIntensity::OverTheTop => (0.999, 0.995, 2.0, 0.8, 26),
            };

        // Update fluid uniforms.
        let uniforms = FluidUniformsGpu {
            grid_w: GRID_W as f32,
            grid_h: GRID_H as f32,
            inv_grid_w: 1.0 / GRID_W as f32,
            inv_grid_h: 1.0 / GRID_H as f32,
            dt: dt.min(0.05),
            density_dissipation,
            velocity_dissipation,
            _pad: 0.0,
        };
        queue.write_buffer(&self.fluid_uniforms_buf, 0, bytemuck::bytes_of(&uniforms));

        // Update render params.
        queue.write_buffer(
            &self.render_params_buf,
            0,
            bytemuck::bytes_of(&FluidRenderParamsGpu {
                max_alpha,
                _pad: [0.0; 3],
            }),
        );

        // Build injection data.
        let mut injection = InjectionParamsGpu {
            points: [InjectionPointGpu {
                pos_radius: [0.0; 4],
                vel_density: [0.0; 4],
                color_pad: [0.0; 4],
            }; MAX_INJECTIONS],
            active_count: self.impulses.len().min(MAX_INJECTIONS) as u32,
            _pad: [0; 3],
        };

        for (i, imp) in self.impulses.iter().take(MAX_INJECTIONS).enumerate() {
            let gx = imp.screen_x / self.screen_w * GRID_W as f32;
            let gy = imp.screen_y / self.screen_h * GRID_H as f32;
            // Scale velocity from screen-space to grid-space.
            let vx = imp.vel_x / self.screen_w * GRID_W as f32 * inject_mult;
            let vy = imp.vel_y / self.screen_h * GRID_H as f32 * inject_mult;
            injection.points[i] = InjectionPointGpu {
                pos_radius: [gx, gy, imp.radius, imp.strength * inject_mult],
                vel_density: [vx, vy, 0.0, 0.0],
                color_pad: [
                    imp.color[0] * imp.strength * inject_mult,
                    imp.color[1] * imp.strength * inject_mult,
                    imp.color[2] * imp.strength * inject_mult,
                    0.0,
                ],
            };
        }
        self.impulses.clear();

        queue.write_buffer(&self.injection_buf, 0, bytemuck::bytes_of(&injection));

        let wg_x = (GRID_W + 7) / 8;
        let wg_y = (GRID_H + 7) / 8;

        // 1. Injection pass — inject into current buffer set (always set 0 since we
        //    rebuild bind groups referencing set 0, and we copy results as needed).
        //    For simplicity, injection always targets set 0, and advection reads set 0
        //    writes set 1. After advection, the "live" data is in set 1.
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fluid-inject"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.inject_pipeline);
            pass.set_bind_group(0, &self.inject_bind_group, &[]);
            pass.dispatch_workgroups(wg_x, wg_y, 1);
        }

        // 2. Advection pass — reads set 0, writes set 1.
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fluid-advect"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.advect_pipeline);
            pass.set_bind_group(0, &self.advect_bind_groups[0], &[]);
            pass.dispatch_workgroups(wg_x, wg_y, 1);
        }

        // 3. Divergence pass — mode 0, operates on velocity set 1.
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fluid-divergence"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pressure_pipeline);
            pass.set_bind_group(0, &self.pressure_bind_groups[0], &[]); // divergence bg
            pass.dispatch_workgroups(wg_x, wg_y, 1);
        }

        // 4. Jacobi iterations — ping-pong pressure buffers.
        // bind_groups[1] reads p[0] writes p[1], bind_groups[2] reads p[1] writes p[0].
        for j in 0..jacobi_iters {
            let bg_idx = if j % 2 == 0 { 1 } else { 2 }; // jacobi bind groups
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fluid-jacobi"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pressure_pipeline);
            pass.set_bind_group(0, &self.pressure_bind_groups[bg_idx], &[]);
            pass.dispatch_workgroups(wg_x, wg_y, 1);
        }

        // 5. Projection pass — mode 2, subtract pressure gradient from velocity (set 1).
        // After jacobi_iters iterations starting with bg[1] (reads p[0]),
        // the final result is in p[jacobi_iters % 2] when even, p[1-jacobi_iters%2] when odd.
        // But our projection bg[3] has a fixed pressure_src. We need to pick the right one.
        // For now, bg[3] uses pressure_src = pressure[0]. After even iterations, result is in p[1].
        // After odd iterations, result is in p[0]. We need to handle this.
        // Simplest: always do an even number of Jacobi iterations.
        // Or: create two projection bind groups. Let's just ensure even iterations.
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fluid-projection"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pressure_pipeline);
            pass.set_bind_group(0, &self.pressure_bind_groups[3], &[]); // projection bg
            pass.dispatch_workgroups(wg_x, wg_y, 1);
        }

        // 6. Copy density from set 1 back to set 0 so injection works next frame.
        let buf_size = (GRID_CELLS as u64) * 4; // f32 = 4 bytes
        encoder.copy_buffer_to_buffer(&self.velocity_x[1], 0, &self.velocity_x[0], 0, buf_size);
        encoder.copy_buffer_to_buffer(&self.velocity_y[1], 0, &self.velocity_y[0], 0, buf_size);
        encoder.copy_buffer_to_buffer(&self.density_r[1], 0, &self.density_r[0], 0, buf_size);
        encoder.copy_buffer_to_buffer(&self.density_g[1], 0, &self.density_g[0], 0, buf_size);
        encoder.copy_buffer_to_buffer(&self.density_b[1], 0, &self.density_b[0], 0, buf_size);

        // 7. Convert density buffers → RGBA texture for rendering.
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fluid-d2t"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.density_to_tex_pipeline);
            pass.set_bind_group(0, &self.density_to_tex_bind_group, &[]);
            pass.dispatch_workgroups(wg_x, wg_y, 1);
        }
    }

    /// Draw the smoke overlay. Call during the render pass, between tiles and UI.
    pub fn draw<'a>(
        &'a self,
        pass: &mut wgpu::RenderPass<'a>,
        globals_bind_group: &'a wgpu::BindGroup,
        vertex_buffer: &'a wgpu::Buffer,
        index_buffer: &'a wgpu::Buffer,
        instance_buffer: &'a wgpu::Buffer,
    ) {
        pass.set_pipeline(&self.render_pipeline);
        pass.set_bind_group(0, globals_bind_group, &[]);
        pass.set_bind_group(1, &self.render_bind_group, &[]);
        pass.set_vertex_buffer(0, vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, instance_buffer.slice(..));
        pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        pass.draw_indexed(0..6, 0, 0..1);
    }

    pub fn update_screen_size(&mut self, w: f32, h: f32) {
        self.screen_w = w;
        self.screen_h = h;
    }
}

// ---------------------------------------------------------------------------
// Bind group layout helpers
// ---------------------------------------------------------------------------

fn bgl_uniform(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn bgl_storage_rw(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn bgl_storage_ro(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}
