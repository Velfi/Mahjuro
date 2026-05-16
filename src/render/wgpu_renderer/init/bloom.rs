use std::mem::size_of;

use super::super::*;

pub struct BloomBundle {
    pub extract_pipeline: wgpu::RenderPipeline,
    pub blur_pipeline: wgpu::RenderPipeline,
    pub composite_pipeline: wgpu::RenderPipeline,
    pub bind_group_layout: wgpu::BindGroupLayout,
    pub extract_bind_group_layout: wgpu::BindGroupLayout,
    pub composite_bind_group_layout: wgpu::BindGroupLayout,
    pub extract_params_buffer: wgpu::Buffer,
    pub blur_h_params_buffer: wgpu::Buffer,
    pub blur_v_params_buffer: wgpu::Buffer,
    pub composite_params_buffer: wgpu::Buffer,
    pub sampler: wgpu::Sampler,
    pub scene_bind_group: wgpu::BindGroup,
    pub ping_bind_group: wgpu::BindGroup,
    pub pong_bind_group: wgpu::BindGroup,
    pub composite_bind_group: wgpu::BindGroup,
    pub ping_texture: wgpu::Texture,
    pub ping_view: wgpu::TextureView,
    pub pong_texture: wgpu::Texture,
    pub pong_view: wgpu::TextureView,
}

pub fn build_bloom(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bloom_w: u32,
    bloom_h: u32,
    scene_hdr_format: wgpu::TextureFormat,
    bloom_extract_shader: &wgpu::ShaderModule,
    bloom_blur_shader: &wgpu::ShaderModule,
    bloom_composite_shader: &wgpu::ShaderModule,
    shop_linear_bloom_view: &wgpu::TextureView,
    scene_color_view: &wgpu::TextureView,
) -> BloomBundle {
    let (bloom_ping_texture, bloom_ping_view) =
        crate::render::wgpu_renderer::resources::create_post_texture(
            device,
            scene_hdr_format,
            bloom_w,
            bloom_h,
            "bloom-ping",
        );
    let (bloom_pong_texture, bloom_pong_view) =
        crate::render::wgpu_renderer::resources::create_post_texture(
            device,
            scene_hdr_format,
            bloom_w,
            bloom_h,
            "bloom-pong",
        );

    let bloom_params_size = size_of::<BloomParams>() as u64;
    let bloom_ub = |label: &'static str| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: bloom_params_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    };

    let bloom_extract_params_buffer = bloom_ub("bloom-extract-params");
    let bloom_blur_h_params_buffer = bloom_ub("bloom-blur-h-params");
    let bloom_blur_v_params_buffer = bloom_ub("bloom-blur-v-params");
    let bloom_composite_params_buffer = bloom_ub("bloom-composite-params");

    let bloom_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("bloom-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });

    let bloom_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom-bg-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

    let bloom_extract_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom-extract-bg-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

    let bloom_composite_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom-composite-bg-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

    let bloom_blur_pipeline_layout =
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bloom-blur-pl"),
            bind_group_layouts: &[Some(&bloom_bind_group_layout)],
            immediate_size: 0,
        });

    let bloom_extract_pipeline_layout =
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bloom-extract-pl"),
            bind_group_layouts: &[Some(&bloom_extract_bind_group_layout)],
            immediate_size: 0,
        });

    let bloom_extract_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("bloom-extract-pipeline"),
        layout: Some(&bloom_extract_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &bloom_extract_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &bloom_extract_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: scene_hdr_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let bloom_blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("bloom-blur-pipeline"),
        layout: Some(&bloom_blur_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &bloom_blur_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &bloom_blur_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: scene_hdr_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let bloom_composite_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("bloom-composite-pl"),
        bind_group_layouts: &[Some(&bloom_composite_bind_group_layout)],
        immediate_size: 0,
    });

    let bloom_composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("bloom-composite-pipeline"),
        layout: Some(&bloom_composite_layout),
        vertex: wgpu::VertexState {
            module: &bloom_composite_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &bloom_composite_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: scene_hdr_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let bloom_scene_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bloom-scene-bg"),
        layout: &bloom_extract_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: bloom_extract_params_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&scene_color_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&shop_linear_bloom_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&bloom_sampler),
            },
        ],
    });

    let bloom_ping_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bloom-ping-bg"),
        layout: &bloom_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: bloom_blur_h_params_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&bloom_ping_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&bloom_sampler),
            },
        ],
    });

    let bloom_pong_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bloom-pong-bg"),
        layout: &bloom_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: bloom_blur_v_params_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&bloom_pong_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&bloom_sampler),
            },
        ],
    });

    let bloom_composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("bloom-composite-bg"),
        layout: &bloom_composite_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: bloom_composite_params_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&scene_color_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&bloom_ping_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&bloom_sampler),
            },
        ],
    });

    let inv_bw = 1.0 / bloom_w as f32;
    let inv_bh = 1.0 / bloom_h as f32;
    let bloom_data0 = [1.1_f32, 0.0, inv_bw, inv_bh];

    queue.write_buffer(
        &bloom_extract_params_buffer,
        0,
        bytemuck::bytes_of(&BloomParams {
            data0: bloom_data0,
            data1: [0.0; 4],
        }),
    );
    queue.write_buffer(
        &bloom_blur_h_params_buffer,
        0,
        bytemuck::bytes_of(&BloomParams {
            data0: bloom_data0,
            data1: [1.0, 0.0, 0.0, 0.0],
        }),
    );
    queue.write_buffer(
        &bloom_blur_v_params_buffer,
        0,
        bytemuck::bytes_of(&BloomParams {
            data0: bloom_data0,
            data1: [0.0, 1.0, 0.0, 0.0],
        }),
    );
    queue.write_buffer(
        &bloom_composite_params_buffer,
        0,
        bytemuck::bytes_of(&BloomParams {
            data0: bloom_data0,
            data1: [0.0; 4],
        }),
    );

    BloomBundle {
        extract_pipeline: bloom_extract_pipeline,
        blur_pipeline: bloom_blur_pipeline,
        composite_pipeline: bloom_composite_pipeline,
        bind_group_layout: bloom_bind_group_layout,
        extract_bind_group_layout: bloom_extract_bind_group_layout,
        composite_bind_group_layout: bloom_composite_bind_group_layout,
        extract_params_buffer: bloom_extract_params_buffer,
        blur_h_params_buffer: bloom_blur_h_params_buffer,
        blur_v_params_buffer: bloom_blur_v_params_buffer,
        composite_params_buffer: bloom_composite_params_buffer,
        sampler: bloom_sampler,
        scene_bind_group: bloom_scene_bind_group,
        ping_bind_group: bloom_ping_bind_group,
        pong_bind_group: bloom_pong_bind_group,
        composite_bind_group: bloom_composite_bind_group,
        ping_texture: bloom_ping_texture,
        ping_view: bloom_ping_view,
        pong_texture: bloom_pong_texture,
        pong_view: bloom_pong_view,
    }
}
