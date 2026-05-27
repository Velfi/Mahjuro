use super::super::*;

use crate::render::gltf_helpers::{GltfPbrUniform, build_sampler_descriptor};
use crate::render::moths_to_a_light::{
    build_bug_body_mesh, build_bug_wing_blur_mesh, build_bug_wing_mesh,
};

pub(super) fn build_renderer_new(target_init: TargetInit) -> anyhow::Result<WgpuRenderer> {
    // Instance, adapter, device, surface/offscreen target, depth — see
    // `init_phases::early_gpu_and_depth`.
    let super::super::init_phases::EarlyGpuState {
        device,
        queue,
        size,
        target,
        config,
        format,
        swapchain_sdr_format,
        swapchain_hdr_available,
        timestamp_supported,
        gpu_profiler_backend,
        depth_texture,
        depth_view,
        ssr_prev_depth_texture,
        ssr_prev_depth_view,
    } = {
        let _early = crate::startup_profile::scope("wgpu.early_gpu");
        super::super::init_phases::early_gpu_and_depth(target_init)?
    };

    // Linear HDR intermediate — main scene + bloom; tonemap maps to the swapchain format.
    let scene_hdr_format = SCENE_HDR_FORMAT;

    let t_shaders = Instant::now();
    let super::super::init_phases::RendererShaderPack {
        quad: shader,
        tile: tile_shader,
        shop: shop_shader,
        text: text_shader,
        gradient: gradient_shader,
        squircle: squircle_shader,
        flame: flame_shader,
        starfield: starfield_shader,
        ember_drift: ember_drift_shader,
        golden_dust: golden_dust_shader,
        moonlit_water: moonlit_water_shader,
        sunlit_water: sunlit_water_shader,
        shooting_star_cascade: shooting_star_cascade_shader,
        cascade_composite: cascade_composite_shader,
        scene_color_downsample: scene_color_downsample_shader,
        tile_outline: tile_outline_shader,
        tile_glow: tile_glow_shader,
        lit_mesh: lit_mesh_shader,
        shadow: shadow_shader,
        image: image_shader,
        bloom_extract: bloom_extract_shader,
        bloom_blur: bloom_blur_shader,
        bloom_composite: bloom_composite_shader,
        tonemap: tonemap_shader_module,
        emissive_probe_update: emissive_probe_update_shader,
        emissive_probe_apply: emissive_probe_apply_shader,
        emissive_gi_composite: emissive_gi_composite_shader,
    } = super::super::init_phases::create_renderer_shader_modules(&device);

    let globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("globals"),
        contents: bytemuck::bytes_of(&Globals {
            screen: [size.width as f32, size.height as f32],
            time: 0.0,
            gamma: 1.0,
            cursor_pos: [size.width as f32 * 0.5, size.height as f32 * 0.5],
            transition_progress: 0.0,
            quality_level: 2.0,
            moon_phase: current_moon_phase(),
            _globals_pad: [0.0; 3],
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("globals-layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });

    let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("globals-bg"),
        layout: &globals_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: globals_buffer.as_entire_binding(),
        }],
    });

    // Point-light uniform buffer + bind group (group 1 of the tile pipeline).
    // Initialised empty; populated each frame from `frame.scene_lighting`.
    let point_lights_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("point-lights"),
        contents: bytemuck::bytes_of(&PointLightsBuf::from_scene_punctual(
            &PunctualLightBakeParams {
                src: &[],
                candle_count: 0,
                flame_height_world: 0.0,
                lit_mesh_punctual_intensity_scale: 1.0,
                screen_w: 1.0,
                screen_h: 1.0,
                gamma: 1.0,
                time: 0.0,
            },
        )),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    // Companion uniform: per-frame analytic tile occluders for the
    // candle-pool ray-AABB shadow test in lit_mesh.wgsl. Lives on the
    // same bind group so the lit-mesh pipeline only needs one extra
    // binding to read it. Other shaders sharing this layout simply
    // ignore the binding.
    let tile_occluders_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("tile-occluders"),
        contents: bytemuck::bytes_of(&TileOccludersBuf::empty()),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let point_lights_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("point-lights-layout"),
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
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let point_lights_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("point-lights-bg"),
        layout: &point_lights_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: point_lights_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: tile_occluders_buffer.as_entire_binding(),
            },
        ],
    });

    // Spotlight buffer + bind group (group 3 of the tile pipeline).
    // Initialised empty; populated each frame from `frame.scene_lighting.spot_lights`.
    let spot_lights_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("spot-lights"),
        contents: bytemuck::bytes_of(&SpotLightsBuf::empty()),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let spot_lights_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("spot-lights-layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let spot_lights_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("spot-lights-bg"),
        layout: &spot_lights_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: spot_lights_buffer.as_entire_binding(),
        }],
    });

    let tile_material_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("tile-material-layout"),
        entries: &[
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
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 4,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 5,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 6,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
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
            wgpu::BindGroupLayoutEntry {
                binding: 8,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });

    let tile_env_distortion_placeholder =
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tile-env-distortion-disabled"),
            contents: bytemuck::bytes_of(&crate::render::hallway_glb::HallwayDistortion::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

    // Outline shell: frame uniform only — per-tile model/colour use a second
    // vertex buffer (Instance step) so we never need storage buffers in VS
    // (some adapters report `max_storage_buffers_per_shader_stage` = 0).
    let tile_outline_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tile-outline-bind-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

    let tile_glb_file = crate::asset_path::get("3d/tile_plastic.glb")
        .or_else(|| crate::asset_path::get("3d/Tile.glb"));
    let loaded_glb = match tile_glb_file {
        Some(file) => load_glb_tile_from_bytes(&file.data),
        None => Err(anyhow::anyhow!(
            "3d/tile_plastic.glb (or 3d/Tile.glb) not found (packs or assets/)"
        )),
    };

    let tile_base_color_factor = [1.0_f32, 1.0, 1.0, 1.0];

    let tile_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("tile-sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    let (tile_default_normal_texture, tile_default_normal_view) =
        flat_normal_map_ts(&device, &queue);

    let quad_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("quad-pl"),
        bind_group_layouts: &[Some(&globals_layout)],
        immediate_size: 0,
    });

    // ---- Moon albedo texture (LRO WAC real heightmap) ----
    let moonlit_water_bind_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("moonlit-water-bind-layout"),
            entries: &[
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
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        // textureLoad in moonlit_water.wgsl — no sampler (FXC/DX12).
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    },
                    count: None,
                },
            ],
        });
    let moonlit_water_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("moonlit-water-pl"),
        bind_group_layouts: &[Some(&moonlit_water_bind_layout)],
        immediate_size: 0,
    });
    let (_moon_albedo_texture, moon_albedo_view) =
        load_metal_heightmap(&device, &queue, "textures/moon_albedo.png", "moon-albedo");
    let moonlit_water_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("moonlit-water-bg"),
        layout: &moonlit_water_bind_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&moon_albedo_view),
            },
        ],
    });

    // ---- Shadow map resources (depth texture + sampler + layouts) ----
    // Built up here so the shared sampling layout can be plumbed into
    // both `tile_layout` and `lit_mesh_pl` below as group 2.
    const SHADOW_MAP_SIZE: u32 =
        crate::render::punctual_shadow_atlas::PUNCTUAL_SHADOW_ATLAS_SIZE;
    let shadow_map_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow-map"),
        size: wgpu::Extent3d {
            width: SHADOW_MAP_SIZE,
            height: SHADOW_MAP_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let shadow_map_view = shadow_map_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("shadow-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        compare: Some(wgpu::CompareFunction::LessEqual),
        ..Default::default()
    });
    let shadow_caster_layout = create_shadow_caster_layout(&device);
    let shadow_warp_layout = create_shadow_warp_layout(&device);
    let shadow_warp_dummy_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("shadow-warp-disabled-uniform"),
        contents: bytemuck::bytes_of(&crate::render::hallway_glb::HallwayDistortion::default()),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let shadow_warp_disabled_bind_group = create_shadow_warp_bind_group(
        &device,
        &shadow_warp_layout,
        &shadow_warp_dummy_buffer,
        "shadow-warp-disabled-bg",
    );
    let shadow_sample_layout = create_shadow_sample_layout(&device);
    let shadow_globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("shadow-globals"),
        contents: bytemuck::bytes_of(&ShadowGlobals::empty_punctual()),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let shadow_ao_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("shadow-ao-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    let shadow_ao_white_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow-ao-white"),
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let shadow_ao_white_view = shadow_ao_white_texture.create_view(&Default::default());
    let shadow_baked_depth_white_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow-baked-depth-white"),
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let shadow_baked_depth_white_view =
        shadow_baked_depth_white_texture.create_view(&Default::default());
    const BAKED_DEPTH_FAR: [u8; 4] = 0x3F80_0000u32.to_le_bytes();
    let baked_depth_white_pixels: [u8; 64] = std::array::from_fn(|i| BAKED_DEPTH_FAR[i % 4]);
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &shadow_baked_depth_white_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &baked_depth_white_pixels,
        wgpu::TexelCopyBufferLayout {
            bytes_per_row: Some(16),
            rows_per_image: Some(4),
            ..Default::default()
        },
        wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
    );
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &shadow_ao_white_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &[255u8; 16],
        wgpu::TexelCopyBufferLayout {
            bytes_per_row: Some(4),
            rows_per_image: Some(4),
            ..Default::default()
        },
        wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
    );
    let shadow_sample_bind_group = crate::render::lit_mesh::create_shadow_sample_bind_group(
        &device,
        &shadow_sample_layout,
        "shadow-sample-bg",
        &shadow_globals_buffer,
        &shadow_map_view,
        &shadow_sampler,
        &shadow_baked_depth_white_view,
        &shadow_ao_white_view,
        &shadow_ao_sampler,
    );
    let room_baked_shadow_gpu = WgpuRenderer::load_room_baked_shadows(
        &device,
        &queue,
        &shadow_sample_layout,
        &shadow_map_view,
        &shadow_sampler,
        &shadow_ao_sampler,
    );

    let tile_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("tile-pl"),
        bind_group_layouts: &[
            Some(&tile_material_layout),
            Some(&point_lights_layout),
            Some(&shadow_sample_layout),
            Some(&spot_lights_layout),
        ],
        immediate_size: 0,
    });

    let tile_outline_pipeline_layout =
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tile-outline-pl"),
            bind_group_layouts: &[
                Some(&tile_outline_bind_group_layout),
                Some(&point_lights_layout),
                Some(&shadow_sample_layout),
                Some(&spot_lights_layout),
            ],
            immediate_size: 0,
        });

    let tile_outline_frame_uniform_buffer =
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tile-outline-frame"),
            contents: bytemuck::bytes_of(&TileOutlineFrameUniform {
                view_proj: glam::Mat4::IDENTITY.to_cols_array(),
                hdr_tonemap: [0.0_f32; 4],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
    let tile_outline_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tile-outline-instances"),
        size: (std::mem::size_of::<TileOutlineInstance>() as u64)
            * (super::super::MAX_SHOWCASE_TILE_SLOTS as u64),
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let tile_outline_frame_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tile-outline-frame-bg"),
        layout: &tile_outline_bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: tile_outline_frame_uniform_buffer.as_entire_binding(),
        }],
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
        array_stride: std::mem::size_of::<GpuInstance>() as wgpu::BufferAddress,
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
            wgpu::VertexAttribute {
                offset: 32,
                shader_location: 3,
                format: wgpu::VertexFormat::Uint32,
            },
        ],
    };

    let depth_3d = wgpu::DepthStencilState {
        format: wgpu::TextureFormat::Depth32Float,
        depth_write_enabled: Some(true),
        depth_compare: Some(wgpu::CompareFunction::Less),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    };

    let depth_3d_blend = wgpu::DepthStencilState {
        depth_write_enabled: Some(false),
        ..depth_3d.clone()
    };

    let depth_ui = wgpu::DepthStencilState {
        format: wgpu::TextureFormat::Depth32Float,
        depth_write_enabled: Some(false),
        depth_compare: Some(wgpu::CompareFunction::Always),
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    };

    let quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("quad-pipeline"),
        layout: Some(&quad_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[vertex_layout.clone(), instance_layout.clone()],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: scene_hdr_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(depth_ui.clone()),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let quad_pipeline_display = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("quad-pipeline-display"),
        layout: Some(&quad_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[vertex_layout.clone(), instance_layout.clone()],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(depth_ui.clone()),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    // Gradient-quad pipeline — alpha-feathered panels behind HUD
    // content. Same `rect`/`color` payload as the base quad_pipeline
    // plus a per-instance `feather` vec4 that drives the shader's
    // falloff (edge softness + axial↔radial blend). Standard alpha
    // blend so multiple gradient quads compose correctly.
    let gradient_instance_layout = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<GradientQuadInstance>() as wgpu::BufferAddress,
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
            wgpu::VertexAttribute {
                offset: 32,
                shader_location: 3,
                format: wgpu::VertexFormat::Float32x4,
            },
        ],
    };

    let gradient_quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("gradient-quad-pipeline"),
        layout: Some(&quad_layout),
        vertex: wgpu::VertexState {
            module: &gradient_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[vertex_layout.clone(), gradient_instance_layout],
        },
        fragment: Some(wgpu::FragmentState {
            module: &gradient_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: scene_hdr_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(depth_ui.clone()),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let squircle_quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("squircle-quad-pipeline"),
        layout: Some(&quad_layout),
        vertex: wgpu::VertexState {
            module: &squircle_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[vertex_layout.clone(), instance_layout.clone()],
        },
        fragment: Some(wgpu::FragmentState {
            module: &squircle_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: scene_hdr_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(depth_ui.clone()),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    // Flame pipeline — 3D billboarded particle fire. The shader
    // reads per-particle world position + age from the instance
    // buffer, constructs a camera-facing quad in the vertex stage,
    // and dissolves the billboard against an age threshold in the
    // fragment stage. Depth-test Less + write off so the particles
    // are correctly occluded by meshes (wax body, coin pile, etc)
    // without self-occluding in z-fight when they overlap.
    //
    // Bind groups: group(0) = 2D Globals (time + screen + gamma),
    //              group(1) = SSR globals (view_proj, view_pos).
    //
    // Lit_mesh spot+SSR layout (group 3): created before `lit_mesh_pl`
    // because the pipeline references it; unrelated to the flame pipeline.
    let lit_mesh_spot_ssr_layout = create_lit_mesh_spot_ssr_layout(&device);
    // Flame-only view layout: just the view_proj/view_pos buffer at
    // binding(0), visible to BOTH stages (the vertex stage needs
    // view_proj to project billboards; the fragment stage may use
    // view_pos for view-relative tricks later). The lit-mesh SSR
    // layout restricts binding(0) to FRAGMENT only, so we can't
    // reuse it here.
    let flame_view_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("flame-view-layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let flame_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("flame-pl"),
        bind_group_layouts: &[Some(&globals_layout), Some(&flame_view_layout)],
        immediate_size: 0,
    });
    let flame_mesh_vertex_layout = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<crate::render::tile_glb::Vertex3dTex>()
            as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: 12,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: 24,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x2,
            },
        ],
    };
    let flame_instance_layout = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<crate::render::flame_volume::GpuFlameInstance>()
            as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 3,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: 16,
                shader_location: 4,
                format: wgpu::VertexFormat::Float32x4,
            },
        ],
    };
    let flame_volume_mesh = crate::render::lit_mesh::LitMeshGpu::new(
        &device,
        &crate::render::candle_flame_mesh::build_candle_flame_volume_mesh(),
        "candle-flame-volume",
    );
    let flame_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("flame-pipeline"),
        layout: Some(&flame_pl),
        vertex: wgpu::VertexState {
            module: &flame_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[flame_mesh_vertex_layout, flame_instance_layout],
        },
        fragment: Some(wgpu::FragmentState {
            module: &flame_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: scene_hdr_format,
                // Additive blend so flames brighten whatever's behind them.
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::SrcAlpha,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            // Billboards are symmetric so cull is a footgun (we'd
            // have to care about wind-space normal orientation).
            cull_mode: None,
            ..Default::default()
        },
        // Depth-test Less + write off. Matches the `lit_mesh_blended`
        // pattern: particles are occluded by opaque geometry in front
        // of them (coin pile, wax body) but stack freely with each
        // other in additive blend.
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        }),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let flame_view_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("flame-view-uniform"),
        contents: bytemuck::bytes_of(&FlameViewUniform {
            view_proj: Mat4::IDENTITY.to_cols_array(),
            view_pos: [0.0; 4],
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let flame_view_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("flame-view-bg"),
        layout: &flame_view_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: flame_view_buffer.as_entire_binding(),
        }],
    });

    // ── Fullscreen additive vignette pipelines ─────────────────────
    // Starfield, ember-drift, and golden-dust all share the same
    // layout: no vertex buffers, globals-only bind group, additive
    // blend onto the UI colour target.
    let vignette_pipeline = |label: &str, module: &wgpu::ShaderModule| -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: scene_hdr_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(depth_ui.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    };

    let starfield_pipeline = vignette_pipeline("starfield-pipeline", &starfield_shader);
    let ember_drift_pipeline = vignette_pipeline("ember-drift-pipeline", &ember_drift_shader);
    let golden_dust_pipeline = vignette_pipeline("golden-dust-pipeline", &golden_dust_shader);
    // moonlit_water gets its own pipeline so it can bind the moon albedo
    // texture alongside globals in a dedicated bind group layout.
    let moonlit_water_pipeline = {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("moonlit-water-pipeline"),
            layout: Some(&moonlit_water_layout),
            vertex: wgpu::VertexState {
                module: &moonlit_water_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &moonlit_water_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: scene_hdr_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(depth_ui.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    };
    let sunlit_water_pipeline = vignette_pipeline("sunlit-water-pipeline", &sunlit_water_shader);
    // The cascade shader is heavy per-pixel; it renders into a half-res
    // offscreen target and is additively composited back into the main
    // pass. The offscreen pipeline writes with REPLACE blend since the
    // target is cleared per-frame before the pre-pass.
    let shooting_star_cascade_pipeline = {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shooting-star-cascade-pipeline"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &shooting_star_cascade_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shooting_star_cascade_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: scene_hdr_format,
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
        })
    };
    let cascade_composite_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("cascade-composite-bgl"),
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
            ],
        });
    let cascade_composite_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("cascade-composite-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    let cascade_composite_pipeline = {
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("cascade-composite-pl"),
            bind_group_layouts: &[Some(&cascade_composite_layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cascade-composite-pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &cascade_composite_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &cascade_composite_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: scene_hdr_format,
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(depth_ui.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    };

    // Half-res scene_color → scene_prev blit. Reuses the cascade-composite
    // bind group layout (texture + filtering sampler) for parity with the
    // existing "tiny fullscreen blit" shader infra.
    let scene_color_downsample_pipeline = {
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("scene-color-downsample-pl"),
            bind_group_layouts: &[Some(&cascade_composite_layout)],
            immediate_size: 0,
        });
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("scene-color-downsample-pipeline"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &scene_color_downsample_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &scene_color_downsample_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: scene_hdr_format,
                    blend: None,
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
        })
    };

    let tile_vertex_layout = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<Vertex3dTex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: 12,
                shader_location: 1,
                format: wgpu::VertexFormat::Float32x3,
            },
            wgpu::VertexAttribute {
                offset: 24,
                shader_location: 2,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: 32,
                shader_location: 3,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: 48,
                shader_location: 4,
                format: wgpu::VertexFormat::Float32x2,
            },
            wgpu::VertexAttribute {
                offset: 56,
                shader_location: 5,
                format: wgpu::VertexFormat::Float32x4,
            },
        ],
    };

    let mk_tile_pipeline = |label: &'static str,
                            blend: Option<wgpu::BlendState>,
                            depth: &wgpu::DepthStencilState,
                            cull_back: bool|
     -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&tile_layout),
            vertex: wgpu::VertexState {
                module: &tile_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: std::slice::from_ref(&tile_vertex_layout),
            },
            fragment: Some(wgpu::FragmentState {
                module: &tile_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: scene_hdr_format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: if cull_back {
                    Some(wgpu::Face::Back)
                } else {
                    None
                },
                ..Default::default()
            },
            depth_stencil: Some(depth.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    };

    let tile_pipeline_opaque_double = mk_tile_pipeline("tile-opaque-ds", None, &depth_3d, false);
    let tile_pipeline_opaque_cull = mk_tile_pipeline("tile-opaque-cull", None, &depth_3d, true);
    let tile_pipeline_blend_double = mk_tile_pipeline(
        "tile-blend-ds",
        Some(wgpu::BlendState::ALPHA_BLENDING),
        &depth_3d_blend,
        false,
    );
    let tile_pipeline_blend_cull = mk_tile_pipeline(
        "tile-blend-cull",
        Some(wgpu::BlendState::ALPHA_BLENDING),
        &depth_3d_blend,
        true,
    );

    let mk_shop_pipeline = |label: &'static str,
                            blend: Option<wgpu::BlendState>,
                            depth: &wgpu::DepthStencilState,
                            cull_back: bool|
     -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&tile_layout),
            vertex: wgpu::VertexState {
                module: &shop_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: std::slice::from_ref(&tile_vertex_layout),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shop_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: scene_hdr_format,
                    blend,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: if cull_back {
                    Some(wgpu::Face::Back)
                } else {
                    None
                },
                ..Default::default()
            },
            depth_stencil: Some(depth.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    };

    let shop_pipeline_opaque_double = mk_shop_pipeline("shop-opaque-ds", None, &depth_3d, false);
    let shop_pipeline_opaque_cull = mk_shop_pipeline("shop-opaque-cull", None, &depth_3d, true);
    let shop_pipeline_blend_double = mk_shop_pipeline(
        "shop-blend-ds",
        Some(wgpu::BlendState::ALPHA_BLENDING),
        &depth_3d_blend,
        false,
    );
    let shop_pipeline_blend_cull = mk_shop_pipeline(
        "shop-blend-cull",
        Some(wgpu::BlendState::ALPHA_BLENDING),
        &depth_3d_blend,
        true,
    );

    // Emissive-only pre-pass into `room_emissive_view` for screen-space GI.
    // Single attachment now that the linear-HDR pre-pass was retired (scene
    // shaders write linear HDR to `scene_color` directly — see
    // `tonemap_composite.wgsl`).
    let mk_shop_mrt_pipeline = |label: &'static str,
                                blend0: Option<wgpu::BlendState>,
                                depth: &wgpu::DepthStencilState,
                                cull_back: bool|
     -> wgpu::RenderPipeline {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&tile_layout),
            vertex: wgpu::VertexState {
                module: &shop_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: std::slice::from_ref(&tile_vertex_layout),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shop_shader,
                entry_point: Some("fs_main_emissive"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: scene_hdr_format,
                    blend: blend0,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: if cull_back {
                    Some(wgpu::Face::Back)
                } else {
                    None
                },
                ..Default::default()
            },
            depth_stencil: Some(depth.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    };

    let shop_pipeline_mrt_opaque_double =
        mk_shop_mrt_pipeline("shop-emissive-opaque-ds", None, &depth_3d, false);
    let shop_pipeline_mrt_opaque_cull =
        mk_shop_mrt_pipeline("shop-emissive-opaque-cull", None, &depth_3d, true);
    let blend_mrt = Some(wgpu::BlendState::ALPHA_BLENDING);
    let shop_pipeline_mrt_blend_double =
        mk_shop_mrt_pipeline("shop-emissive-blend-ds", blend_mrt, &depth_3d_blend, false);
    let shop_pipeline_mrt_blend_cull =
        mk_shop_mrt_pipeline("shop-emissive-blend-cull", blend_mrt, &depth_3d_blend, true);

    // ---- Gold outline shell pipeline (selected tiles) ----
    let tile_outline_instance_vertex_layout = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<TileOutlineInstance>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &[
            wgpu::VertexAttribute {
                offset: 0,
                shader_location: 6,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: 16,
                shader_location: 7,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: 32,
                shader_location: 8,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: 48,
                shader_location: 9,
                format: wgpu::VertexFormat::Float32x4,
            },
            wgpu::VertexAttribute {
                offset: 64,
                shader_location: 10,
                format: wgpu::VertexFormat::Float32x4,
            },
        ],
    };
    let tile_outline_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("tile-outline-pipeline"),
        layout: Some(&tile_outline_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &tile_outline_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[tile_vertex_layout, tile_outline_instance_vertex_layout],
        },
        fragment: Some(wgpu::FragmentState {
            module: &tile_outline_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: scene_hdr_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            // We want to draw only the *back* side of the inflated
            // shell (the side facing away from the camera) so the
            // regular tile mesh can overwrite the interior and
            // leave a thin rim. The tile model matrix has
            // determinant +1 (tile_basis is an even permutation),
            // so winding is preserved — culling Front leaves only
            // the back-facing shell fragments (the ones that peek
            // out past the tile silhouette), which is what we want.
            cull_mode: Some(wgpu::Face::Front),
            ..Default::default()
        },
        depth_stencil: Some(depth_3d.clone()),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    // ---- Tile glow pipeline (selected tile additive halo) ----
    let tile_glow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("tile-glow-pipeline"),
        layout: Some(&quad_layout),
        vertex: wgpu::VertexState {
            module: &tile_glow_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[vertex_layout.clone(), instance_layout.clone()],
        },
        fragment: Some(wgpu::FragmentState {
            module: &tile_glow_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: scene_hdr_format,
                // Additive blend so the glow brightens the table /
                // tile sides without darkening anything.
                blend: Some(wgpu::BlendState {
                    color: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::SrcAlpha,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                    alpha: wgpu::BlendComponent {
                        src_factor: wgpu::BlendFactor::One,
                        dst_factor: wgpu::BlendFactor::One,
                        operation: wgpu::BlendOperation::Add,
                    },
                }),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(depth_ui.clone()),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    // ---- Lit-mesh pipeline (procedural candles + wood table) ----
    let lit_mesh_material_layout = create_lit_mesh_material_layout(&device);
    let lit_mesh_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("lit-mesh-pl"),
        bind_group_layouts: &[
            Some(&lit_mesh_material_layout),
            Some(&point_lights_layout),
            Some(&shadow_sample_layout),
            Some(&lit_mesh_spot_ssr_layout),
        ],
        immediate_size: 0,
    });
    let lit_mesh_ssr_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("lit-mesh-ssr-uniform"),
        contents: bytemuck::bytes_of(&SsrGlobals {
            inv_view_proj: Mat4::IDENTITY.to_cols_array(),
            view_proj: Mat4::IDENTITY.to_cols_array(),
            view_pos: [0.0; 4],
            params: [0.0; 4],
            felt: [2.0, 0.0, 0.0, 0.0],
            shop_punctual: [0.0; 4],
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let lit_mesh_ssr_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("lit-mesh-ssr-sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest,
        ..Default::default()
    });
    let (scene_prev_w, scene_prev_h) = scene_prev_size(size.width.max(1), size.height.max(1));
    let (scene_prev_texture, scene_prev_view) =
        create_scene_prev(&device, scene_hdr_format, scene_prev_w, scene_prev_h);
    let (scene_color_texture, scene_color_view) = create_scene_color(
        &device,
        scene_hdr_format,
        size.width.max(1),
        size.height.max(1),
    );
    let (post_bloom_texture, post_bloom_view) = create_scene_color(
        &device,
        scene_hdr_format,
        size.width.max(1),
        size.height.max(1),
    );
    let (room_emissive_texture, room_emissive_view) = create_scene_color(
        &device,
        scene_hdr_format,
        size.width.max(1),
        size.height.max(1),
    );
    // Fullscreen offscreen for the live yaku-journal GPU render (book
    // page surface samples this in screen space; see `lit_mesh.wgsl`).
    let (journal_scene_texture, journal_scene_view) = create_journal_scene(
        &device,
        scene_hdr_format,
        size.width.max(1),
        size.height.max(1),
    );
    let (cascade_offscreen_texture, cascade_offscreen_view) = create_cascade_offscreen(
        &device,
        scene_hdr_format,
        size.width.max(1),
        size.height.max(1),
    );
    let cascade_composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("cascade-composite-bg"),
        layout: &cascade_composite_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&cascade_offscreen_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&cascade_composite_sampler),
            },
        ],
    });
    // Sampled by the half-res `scene_color → scene_prev` blit each frame.
    // Rebuilt in `resize()` whenever `scene_color_view` is recreated.
    let scene_color_downsample_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("scene-color-downsample-bg"),
        layout: &cascade_composite_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&scene_color_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&cascade_composite_sampler),
            },
        ],
    });
    let bloom_w = (size.width.max(1) / 2).max(1);
    let bloom_h = (size.height.max(1) / 2).max(1);

    let bloom_bundle = super::bloom::build_bloom(&super::bloom::BloomBuildParams {
        device: &device,
        queue: &queue,
        width: bloom_w,
        height: bloom_h,
        scene_hdr_format,
        extract_shader: &bloom_extract_shader,
        blur_shader: &bloom_blur_shader,
        composite_shader: &bloom_composite_shader,
        scene_color_view: &scene_color_view,
    });
    let bloom_extract_pipeline = bloom_bundle.extract_pipeline;
    let bloom_blur_pipeline = bloom_bundle.blur_pipeline;
    let bloom_composite_pipeline = bloom_bundle.composite_pipeline;
    let bloom_bind_group_layout = bloom_bundle.bind_group_layout;
    let bloom_extract_bind_group_layout = bloom_bundle.extract_bind_group_layout;
    let bloom_composite_bind_group_layout = bloom_bundle.composite_bind_group_layout;
    let bloom_extract_params_buffer = bloom_bundle.extract_params_buffer;
    let bloom_blur_h_params_buffer = bloom_bundle.blur_h_params_buffer;
    let bloom_blur_v_params_buffer = bloom_bundle.blur_v_params_buffer;
    let bloom_composite_params_buffer = bloom_bundle.composite_params_buffer;
    let bloom_sampler = bloom_bundle.sampler;
    let bloom_scene_bind_group = bloom_bundle.scene_bind_group;
    let bloom_ping_bind_group = bloom_bundle.ping_bind_group;
    let bloom_pong_bind_group = bloom_bundle.pong_bind_group;
    let bloom_composite_bind_group = bloom_bundle.composite_bind_group;
    let bloom_ping_texture = bloom_bundle.ping_texture;
    let bloom_ping_view = bloom_bundle.ping_view;
    let bloom_pong_texture = bloom_bundle.pong_texture;
    let bloom_pong_view = bloom_bundle.pong_view;
    let (emissive_gi_texture, emissive_gi_view) =
        create_post_texture(&device, scene_hdr_format, bloom_w, bloom_h, "emissive-gi");
    let lit_mesh_spot_ssr_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("lit-mesh-spot-ssr-bg"),
        layout: &lit_mesh_spot_ssr_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: spot_lights_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: lit_mesh_ssr_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&scene_prev_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&ssr_prev_depth_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(&lit_mesh_ssr_sampler),
            },
        ],
    });
    let lit_mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("lit-mesh-pipeline"),
        layout: Some(&lit_mesh_pl),
        vertex: wgpu::VertexState {
            module: &lit_mesh_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex3dTex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        offset: 0,
                        shader_location: 0,
                        format: wgpu::VertexFormat::Float32x3,
                    },
                    wgpu::VertexAttribute {
                        offset: 12,
                        shader_location: 1,
                        format: wgpu::VertexFormat::Float32x3,
                    },
                    wgpu::VertexAttribute {
                        offset: 24,
                        shader_location: 2,
                        format: wgpu::VertexFormat::Float32x2,
                    },
                    wgpu::VertexAttribute {
                        offset: 32,
                        shader_location: 3,
                        format: wgpu::VertexFormat::Float32x4,
                    },
                ],
            }],
        },
        fragment: Some(wgpu::FragmentState {
            module: &lit_mesh_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: scene_hdr_format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(depth_3d.clone()),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let lit_mesh_blended_pipeline =
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lit-mesh-blended-pipeline"),
            layout: Some(&lit_mesh_pl),
            vertex: wgpu::VertexState {
                module: &lit_mesh_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex3dTex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: 12,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: 24,
                            shader_location: 2,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                        wgpu::VertexAttribute {
                            offset: 32,
                            shader_location: 3,
                            format: wgpu::VertexFormat::Float32x4,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &lit_mesh_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: scene_hdr_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

    // ---- Shadow pipeline (depth-only pre-pass) ----
    // Shared by lit-mesh casters (table-excluded) and hand tiles —
    // both vertex layouts begin with `position : vec3<f32>` at
    // offset 0, and the shader only reads location 0.
    let shadow_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("shadow-pl"),
        bind_group_layouts: &[Some(&shadow_caster_layout), Some(&shadow_warp_layout)],
        immediate_size: 0,
    });
    let shadow_depth_state = wgpu::DepthStencilState {
        format: wgpu::TextureFormat::Depth32Float,
        depth_write_enabled: Some(true),
        depth_compare: Some(wgpu::CompareFunction::Less),
        stencil: wgpu::StencilState::default(),
        // Slope-scaled bias to fight acne. The constant component is
        // small because the lit shaders also subtract a depth bias.
        bias: wgpu::DepthBiasState {
            constant: 2,
            slope_scale: 2.5,
            clamp: 0.0,
        },
    };
    let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("shadow-pipeline"),
        layout: Some(&shadow_pl),
        vertex: wgpu::VertexState {
            module: &shadow_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            // Match lit_mesh / tile_glb / room-GLB vertex stride so one
            // pipeline can render any caster. Only attribute 0 (position)
            // is read by the shader.
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex3dTex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                }],
            }],
        },
        fragment: None,
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: Some(wgpu::Face::Front),
            ..Default::default()
        },
        depth_stencil: Some(shadow_depth_state.clone()),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let shadow_pipeline_room_env = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("shadow-pipeline-room-env"),
        layout: Some(&shadow_pl),
        vertex: wgpu::VertexState {
            module: &shadow_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex3dTex>() as wgpu::BufferAddress,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                }],
            }],
        },
        fragment: None,
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: Some(shadow_depth_state),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    // ---- Text pipeline ----
    let text_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("text-bg-layout"),
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
            ],
        });

    let text_overlay_pipeline_layout =
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("text-pl"),
            bind_group_layouts: &[Some(&globals_layout), Some(&text_bind_group_layout)],
            immediate_size: 0,
        });
    let text_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("text-pipeline"),
        layout: Some(&text_overlay_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &text_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[vertex_layout.clone(), instance_layout.clone()],
        },
        fragment: Some(wgpu::FragmentState {
            module: &text_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(depth_ui.clone()),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let text_pipeline_scene_hdr = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("text-pipeline-scene-hdr"),
        layout: Some(&text_overlay_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &text_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[vertex_layout.clone(), instance_layout.clone()],
        },
        fragment: Some(wgpu::FragmentState {
            module: &text_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: scene_hdr_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(depth_ui.clone()),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    // ---- Image pipeline (full-colour textured quads for relic icons, etc.) ----
    let image_pipeline_scene_hdr = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("image-pipeline-scene-hdr"),
        layout: Some(&text_overlay_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &image_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[vertex_layout.clone(), instance_layout.clone()],
        },
        fragment: Some(wgpu::FragmentState {
            module: &image_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: scene_hdr_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(depth_ui.clone()),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let image_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("image-pipeline"),
        layout: Some(&text_overlay_pipeline_layout),
        vertex: wgpu::VertexState {
            module: &image_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[vertex_layout, instance_layout],
        },
        fragment: Some(wgpu::FragmentState {
            module: &image_shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: Some(depth_ui.clone()),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });

    let tonemap_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tonemap-bg-layout"),
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
    let tonemap_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("tonemap-pl"),
        bind_group_layouts: &[Some(&tonemap_bind_group_layout)],
        immediate_size: 0,
    });
    let make_tonemap_pipe = |label: &'static str, out_fmt: wgpu::TextureFormat| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&tonemap_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &tonemap_shader_module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &tonemap_shader_module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: out_fmt,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        })
    };
    let tonemap_pipeline = make_tonemap_pipe("tonemap-pipeline", config.format);
    let tonemap_rgba16f_pipeline =
        make_tonemap_pipe("tonemap-rgba16f-pipeline", wgpu::TextureFormat::Rgba16Float);

    let tonemap_params_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tonemap-params"),
        size: std::mem::size_of::<TonemapParams>() as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let tonemap_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tonemap-pass-bg"),
        layout: &tonemap_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: tonemap_params_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&post_bloom_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&bloom_sampler),
            },
        ],
    });
    // Alternative tonemap bind group that samples the linear HDR scene
    // directly when bloom + fisheye + GI are all inactive. Lets the
    // scene-composite-pass be skipped on the Steam Deck baseline (where
    // bloom is disabled in `EffectLayers::BASELINE`), saving one
    // fullscreen R16G16B16A16 read+write per frame.
    let tonemap_bind_group_scene = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tonemap-pass-scene-bg"),
        layout: &tonemap_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: tonemap_params_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&scene_color_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&bloom_sampler),
            },
        ],
    });

    let probe_gi_frame_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("probe-gi-frame-uniform"),
        size: std::mem::size_of::<super::super::ProbeGiFrameUniform>() as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let probe_sh_stride = 9 * std::mem::size_of::<glam::Vec4>();
    let probe_sh_bytes = (crate::render::room_glb::ROOM_EMISSIVE_PROBE_MAX as usize
        * probe_sh_stride) as wgpu::BufferAddress;
    let probe_sh_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("emissive-probe-sh"),
        size: probe_sh_bytes,
        usage: wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_DST
            | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let emissive_probe_update_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("emissive-probe-update-bg-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        sample_type: wgpu::TextureSampleType::Depth,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
    let emissive_probe_apply_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("emissive-probe-apply-bg-layout"),
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
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
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
    let emissive_probe_update_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("emissive-probe-update-pl"),
        bind_group_layouts: &[Some(&emissive_probe_update_bind_group_layout)],
        immediate_size: 0,
    });
    let emissive_probe_apply_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("emissive-probe-apply-pl"),
        bind_group_layouts: &[Some(&emissive_probe_apply_bind_group_layout)],
        immediate_size: 0,
    });
    let emissive_probe_update_pipeline =
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("emissive-probe-update-pipeline"),
            layout: Some(&emissive_probe_update_pl),
            module: &emissive_probe_update_shader,
            entry_point: Some("update_probes"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
    let emissive_probe_apply_pipeline =
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("emissive-probe-apply-pipeline"),
            layout: Some(&emissive_probe_apply_pl),
            vertex: wgpu::VertexState {
                module: &emissive_probe_apply_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &emissive_probe_apply_shader,
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
    let emissive_probe_update_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("emissive-probe-update-bg"),
        layout: &emissive_probe_update_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: probe_gi_frame_uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&room_emissive_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&depth_view),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::Sampler(&bloom_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: probe_sh_buffer.as_entire_binding(),
            },
        ],
    });
    let emissive_probe_apply_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("emissive-probe-apply-bg"),
        layout: &emissive_probe_apply_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: probe_gi_frame_uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: probe_sh_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&depth_view),
            },
        ],
    });

    let emissive_gi_composite_bind_group_layout =
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("emissive-gi-composite-bg-layout"),
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
            ],
        });
    let emissive_gi_composite_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("emissive-gi-composite-pl"),
        bind_group_layouts: &[Some(&emissive_gi_composite_bind_group_layout)],
        immediate_size: 0,
    });
    let additive_one_one = wgpu::BlendComponent {
        src_factor: wgpu::BlendFactor::One,
        dst_factor: wgpu::BlendFactor::One,
        operation: wgpu::BlendOperation::Add,
    };
    let emissive_gi_composite_pipeline =
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("emissive-gi-composite-pipeline"),
            layout: Some(&emissive_gi_composite_pl),
            vertex: wgpu::VertexState {
                module: &emissive_gi_composite_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &emissive_gi_composite_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: scene_hdr_format,
                    blend: Some(wgpu::BlendState {
                        color: additive_one_one,
                        alpha: wgpu::BlendComponent::REPLACE,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
    let emissive_gi_composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("emissive-gi-composite-bg"),
        layout: &emissive_gi_composite_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&emissive_gi_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&bloom_sampler),
            },
        ],
    });
    crate::startup_profile::record("wgpu.shaders_and_pipelines", t_shaders.elapsed());

    let t_fonts = Instant::now();
    let ui_font = load_ui_font().cloned();
    if ui_font.is_some() {
        log::debug!("UI font loaded.");
    } else {
        log::warn!("No UI font found; panel text will be blank.");
    }
    let ui_font_italic = crate::render::decal::load_ui_font_italic().cloned();
    let mono_font = load_mono_font().cloned();
    if mono_font.is_some() {
        log::debug!("Mono UI font loaded.");
    } else {
        log::warn!("No mono UI font found; tabular Chronicle text falls back to serif.");
    }
    let emoji_font = load_noto_emoji_font();
    if emoji_font.is_some() {
        log::debug!("Noto Emoji font loaded.");
    } else {
        log::warn!("No Noto Emoji font found; tile symbols may be blank.");
    }
    crate::startup_profile::record("wgpu.fonts", t_fonts.elapsed());

    let quad_v: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("quad-verts"),
        contents: bytemuck::cast_slice(&quad_v),
        usage: wgpu::BufferUsages::VERTEX,
    });

    let idx: [u16; 6] = [0, 1, 2, 2, 1, 3];
    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("quad-idx"),
        contents: bytemuck::cast_slice(&idx),
        usage: wgpu::BufferUsages::INDEX,
    });

    let (_tile_glb_default_mr_tex, tile_glb_default_mr_view) =
        default_metallic_roughness_map(&device, &queue);
    let (_tile_glb_default_emissive_tex, tile_glb_default_emissive_view) =
        default_emissive_map(&device, &queue);

    let (
        tile_primitives,
        tile_outline_vertex_buffer,
        tile_outline_index_buffer,
        tile_outline_index_count,
    ) = {
        let _tile = crate::startup_profile::scope("wgpu.tile_mesh");
        let mut tile_outline_merge_vertices: Vec<Vertex3dTex> = Vec::new();
        let mut tile_outline_merge_indices: Vec<u32> = Vec::new();
        let tile_primitives: Vec<TilePrimitiveGpu> = match loaded_glb {
            Ok(mut mesh) => {
                normalize_mesh(&mut mesh);
                log::info!("Loaded 3D tile: {} primitive(s)", mesh.primitives.len());
                for prim in mesh.primitives.iter() {
                    let base = tile_outline_merge_vertices.len() as u32;
                    tile_outline_merge_vertices.extend_from_slice(&prim.vertices);
                    tile_outline_merge_indices.extend(prim.indices.iter().map(|&ix| ix + base));
                }
                let mut out = Vec::with_capacity(mesh.primitives.len());
                for (i, prim) in mesh.primitives.iter().enumerate() {
                    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("tile-verts"),
                        contents: bytemuck::cast_slice(&prim.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                    let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("tile-idx"),
                        contents: bytemuck::cast_slice(&prim.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                    let mips = crate::render::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                    let (_albedo_texture, albedo_view) = match &prim.albedo_rgba {
                        Some((rgba, w, h)) => upload_rgba_texture_with_mips(&TextureUploadParams {
                            device: &device,
                            queue: &queue,
                            label: "tile-prim-albedo".to_string(),
                            rgba,
                            width: *w,
                            height: *h,
                            format: wgpu::TextureFormat::Rgba8UnormSrgb,
                            mips,
                        }),
                        None => white_albedo(&device, &queue),
                    };
                    let normal_view = match &prim.normal_rgba {
                        Some((rgba, w, h)) => {
                            upload_rgba_texture_with_mips(&TextureUploadParams {
                                device: &device,
                                queue: &queue,
                                label: format!("tile-prim-normal-{i}"),
                                rgba,
                                width: *w,
                                height: *h,
                                format: wgpu::TextureFormat::Rgba8Unorm,
                                mips,
                            })
                            .1
                        }
                        None => tile_default_normal_view.clone(),
                    };
                    let metallic_roughness_view = match &prim.metallic_roughness_rgba {
                        Some((rgba, w, h)) => {
                            upload_rgba_texture_with_mips(&TextureUploadParams {
                                device: &device,
                                queue: &queue,
                                label: format!("tile-prim-mr-{i}"),
                                rgba,
                                width: *w,
                                height: *h,
                                format: wgpu::TextureFormat::Rgba8Unorm,
                                mips,
                            })
                            .1
                        }
                        None => tile_glb_default_mr_view.clone(),
                    };
                    let emissive_view = match &prim.emissive_rgba {
                        Some((rgba, w, h)) => {
                            upload_rgba_texture_with_mips(&TextureUploadParams {
                                device: &device,
                                queue: &queue,
                                label: format!("tile-prim-emissive-{i}"),
                                rgba,
                                width: *w,
                                height: *h,
                                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                                mips,
                            })
                            .1
                        }
                        None => tile_glb_default_emissive_view.clone(),
                    };
                    let pbr_uniform_buffer =
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("tile-pbr-{i}")),
                            contents: bytemuck::bytes_of(&GltfPbrUniform::from_loaded(
                                prim.metallic_factor,
                                prim.roughness_factor,
                                prim.emissive_factor,
                                prim.alpha_mode,
                                prim.alpha_cutoff,
                            )),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                    let sampler =
                        device.create_sampler(&build_sampler_descriptor(prim.sampler, None));
                    log::info!(
                        "  prim {}: {} verts, {} idx, has_tex={}",
                        i,
                        prim.vertices.len(),
                        prim.indices.len(),
                        prim.albedo_rgba.is_some(),
                    );
                    out.push(TilePrimitiveGpu {
                        vertex_buffer: vb,
                        index_buffer: ib,
                        index_count: prim.indices.len() as u32,
                        albedo_view,
                        normal_view,
                        metallic_roughness_view,
                        emissive_view,
                        pbr_uniform_buffer,
                        sampler,
                        pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                    });
                }
                out
            }
            Err(e) => {
                log::warn!("Could not load tile mesh GLB (3D hand tiles disabled): {e:#}");
                Vec::new()
            }
        };

        let dummy_outline_vertex = Vertex3dTex {
            position: [0.0, 0.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            uv: [0.0, 0.0],
            tangent: Vertex3dTex::DEFAULT_TANGENT,
            uv_emr: [0.0, 0.0],
            color: [1.0, 1.0, 1.0, 1.0],
        };
        let (tile_outline_vertex_buffer, tile_outline_index_buffer, tile_outline_index_count) =
            if tile_outline_merge_indices.is_empty() {
                let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("tile-outline-verts-dummy"),
                    contents: bytemuck::cast_slice(&[dummy_outline_vertex; 3]),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("tile-outline-idx-dummy"),
                    contents: bytemuck::cast_slice(&[0u32, 1, 2]),
                    usage: wgpu::BufferUsages::INDEX,
                });
                (vb, ib, 0u32)
            } else {
                let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("tile-outline-verts-merged"),
                    contents: bytemuck::cast_slice(&tile_outline_merge_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("tile-outline-idx-merged"),
                    contents: bytemuck::cast_slice(&tile_outline_merge_indices),
                    usage: wgpu::BufferUsages::INDEX,
                });
                (vb, ib, tile_outline_merge_indices.len() as u32)
            };
        (
            tile_primitives,
            tile_outline_vertex_buffer,
            tile_outline_index_buffer,
            tile_outline_index_count,
        )
    };

    let (shop_env_primitives, shop_environment, shop_gltf_anim, shop_eyeball_prim_indices) = {
        let _shop = crate::startup_profile::scope("wgpu.room.shop");
        crate::render::room_glb::with_shop_glb_cpu(|cpu_opt| {
            let mut prims = Vec::new();
            let mut gpu_wrap = None;
            let mut shop_gltf_anim = crate::render::room_gltf_anim::RoomGltfAnimGpu::default();
            let mut shop_eyeball_prim_indices = Vec::new();
            let Some(cpu) = cpu_opt else {
                return (prims, gpu_wrap, shop_gltf_anim, shop_eyeball_prim_indices);
            };
            shop_gltf_anim = crate::render::room_gltf_anim::RoomGltfAnimGpu::from_room_cpu(
                &cpu.gltf_anim_library,
                &cpu.environment_primitives,
                "shop.glb",
            );
            if !cpu.environment_primitives.is_empty() {
                let mut room_tex_cache = RoomEnvTextureCache::new();
                let (_white_tex, white_albedo_view) = white_albedo(&device, &queue);
                for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                    if env_prim.gltf_node_name.as_deref() == Some("Eyeball") {
                        shop_eyeball_prim_indices.push(i);
                    }
                    let prim = &env_prim.mesh;
                    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("shop-env-verts-{i}")),
                        contents: bytemuck::cast_slice(&prim.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                    let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("shop-env-idx-{i}")),
                        contents: bytemuck::cast_slice(&prim.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                    let mips = crate::render::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                    let albedo_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("shop-env-albedo-{i}"),
                        prim.albedo_rgba.as_ref(),
                        prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8UnormSrgb,
                        mips,
                        &white_albedo_view,
                    );
                    let normal_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("shop-env-normal-{i}"),
                        prim.normal_rgba.as_ref(),
                        prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8Unorm,
                        mips,
                        &tile_default_normal_view,
                    );
                    let metallic_roughness_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("shop-env-mr-{i}"),
                        prim.metallic_roughness_rgba.as_ref(),
                        prim.metallic_roughness_mip_chain
                            .as_deref()
                            .map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8Unorm,
                        mips,
                        &tile_glb_default_mr_view,
                    );
                    let emissive_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("shop-env-emissive-{i}"),
                        prim.emissive_rgba.as_ref(),
                        prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8UnormSrgb,
                        mips,
                        &tile_glb_default_emissive_view,
                    );
                    let pbr_uniform_buffer =
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("shop-pbr-{i}")),
                            contents: bytemuck::bytes_of(&GltfPbrUniform::from_loaded(
                                prim.metallic_factor,
                                prim.roughness_factor,
                                prim.emissive_factor,
                                prim.alpha_mode,
                                prim.alpha_cutoff,
                            )),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                    let sampler =
                        device.create_sampler(&build_sampler_descriptor(prim.sampler, None));
                    prims.push(TilePrimitiveGpu {
                        vertex_buffer: vb,
                        index_buffer: ib,
                        index_count: prim.indices.len() as u32,
                        albedo_view,
                        normal_view,
                        metallic_roughness_view,
                        emissive_view,
                        pbr_uniform_buffer,
                        sampler,
                        pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                    });
                }
                let (_white_tex, shop_decal_view) = white_albedo(&device, &queue);
                let shop_candle_sss_tex = load_room_env_png_texture(
                    &device,
                    &queue,
                    crate::render::room_env_gltf::SHOP_CANDLE_SSS_BAKE_TEXTURE,
                    "shop-candle-sss-bake",
                    wgpu::TextureFormat::Rgba8Unorm,
                );
                let shop_candle_sss_view = shop_candle_sss_tex
                    .as_ref()
                    .map(|(_, v)| v)
                    .unwrap_or(&shop_decal_view);
                let uniform_buffers = create_room_env_camera_uniform_buffers(
                    &device,
                    prims.len(),
                    "shop-env-uniform",
                );
                let distortion_buffer =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("shop-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::render::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
                let bind_groups: Vec<wgpu::BindGroup> = prims
                    .iter()
                    .enumerate()
                    .map(|(pi, p)| {
                        let is_candle = cpu
                            .environment_primitives
                            .get(pi)
                            .and_then(|ep| ep.gltf_node_name.as_deref())
                            .is_some_and(
                                crate::render::room_env_gltf::is_shop_candle_wax_node_name,
                            );
                        let decal_view = if is_candle {
                            shop_candle_sss_view
                        } else {
                            &shop_decal_view
                        };
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("shop-env-bg"),
                            layout: &tile_material_layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: uniform_buffers[pi].as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::TextureView(&p.albedo_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::Sampler(&p.sampler),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 3,
                                    resource: wgpu::BindingResource::TextureView(decal_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 4,
                                    resource: wgpu::BindingResource::TextureView(&p.normal_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 5,
                                    resource: p.pbr_uniform_buffer.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 6,
                                    resource: wgpu::BindingResource::TextureView(
                                        &p.metallic_roughness_view,
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 7,
                                    resource: wgpu::BindingResource::TextureView(&p.emissive_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 8,
                                    resource: distortion_buffer.as_entire_binding(),
                                },
                            ],
                        })
                    })
                    .collect();
                let (shadow_uniform_buffers, shadow_bind_groups) =
                    create_room_env_shadow_gpu_batch(
                        &device,
                        &shadow_caster_layout,
                        prims.len(),
                        "shop-env-shadow",
                    );
                let shadow_warp_bind_group = create_shadow_warp_bind_group(
                    &device,
                    &shadow_warp_layout,
                    &distortion_buffer,
                    "shop-env-shadow-warp",
                );
                gpu_wrap = Some(ShopEnvironmentGpu {
                    uniform_buffers,
                    distortion_buffer,
                    shadow_uniform_buffers,
                    shadow_bind_groups,
                    shadow_warp_bind_group,
                    bind_groups,
                    archive_sign_decal_texture: None,
                    shop_candle_sss_texture: shop_candle_sss_tex.map(|(t, _)| t),
                });
                if shop_eyeball_prim_indices.is_empty() {
                    if let Some(bindings) = shop_gltf_anim.clip_prim_bindings.get("eyeball_travel") {
                        shop_eyeball_prim_indices =
                            bindings.iter().map(|(pi, _)| *pi).collect();
                        log::info!(
                            "shop.glb GPU: Eyeball prims from eyeball_travel bindings {:?}",
                            shop_eyeball_prim_indices
                        );
                    }
                } else {
                    log::info!(
                        "shop.glb GPU: Eyeball primitive indices {:?}",
                        shop_eyeball_prim_indices
                    );
                }
                log::info!("shop.glb GPU: {} primitive draw(s)", prims.len());
            }
            (prims, gpu_wrap, shop_gltf_anim, shop_eyeball_prim_indices)
        })
    };

    crate::render::room_glb::release_shop_environment_cpu_sources_after_gpu_upload();

    let (hallway_env_primitives, hallway_environment) = {
        let _hallway = crate::startup_profile::scope("wgpu.room.hallway");
        crate::render::hallway_glb::with_hallway_glb_cpu(|cpu_opt| {
            let mut prims = Vec::new();
            let mut gpu_wrap = None;
            let Some(cpu) = cpu_opt else {
                return (prims, gpu_wrap);
            };
            if !cpu.environment_primitives.is_empty() {
                let mut room_tex_cache = RoomEnvTextureCache::new();
                let (_white_tex, white_albedo_view) = white_albedo(&device, &queue);
                for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                    let prim = &env_prim.mesh;
                    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("hallway-env-verts-{i}")),
                        contents: bytemuck::cast_slice(&prim.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                    let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("hallway-env-idx-{i}")),
                        contents: bytemuck::cast_slice(&prim.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                    let mips = crate::render::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                    let albedo_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("hallway-env-albedo-{i}"),
                        prim.albedo_rgba.as_ref(),
                        prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8UnormSrgb,
                        mips,
                        &white_albedo_view,
                    );
                    let normal_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("hallway-env-normal-{i}"),
                        prim.normal_rgba.as_ref(),
                        prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8Unorm,
                        mips,
                        &tile_default_normal_view,
                    );
                    let metallic_roughness_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("hallway-env-mr-{i}"),
                        prim.metallic_roughness_rgba.as_ref(),
                        prim.metallic_roughness_mip_chain
                            .as_deref()
                            .map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8Unorm,
                        mips,
                        &tile_glb_default_mr_view,
                    );
                    let emissive_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("hallway-env-emissive-{i}"),
                        prim.emissive_rgba.as_ref(),
                        prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8UnormSrgb,
                        mips,
                        &tile_glb_default_emissive_view,
                    );
                    let pbr_uniform_buffer =
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("hallway-pbr-{i}")),
                            contents: bytemuck::bytes_of(&GltfPbrUniform::from_loaded(
                                prim.metallic_factor,
                                prim.roughness_factor,
                                prim.emissive_factor,
                                prim.alpha_mode,
                                prim.alpha_cutoff,
                            )),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                    let sampler =
                        device.create_sampler(&build_sampler_descriptor(prim.sampler, None));
                    prims.push(TilePrimitiveGpu {
                        vertex_buffer: vb,
                        index_buffer: ib,
                        index_count: prim.indices.len() as u32,
                        albedo_view,
                        normal_view,
                        metallic_roughness_view,
                        emissive_view,
                        pbr_uniform_buffer,
                        sampler,
                        pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                    });
                }
                let (_white_tex, hallway_decal_view) = white_albedo(&device, &queue);
                let uniform_buffers = create_room_env_camera_uniform_buffers(
                    &device,
                    prims.len(),
                    "hallway-env-uniform",
                );
                let distortion_buffer =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("hallway-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::render::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
                let bind_groups: Vec<wgpu::BindGroup> = prims
                    .iter()
                    .enumerate()
                    .map(|(pi, p)| {
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("hallway-env-bg"),
                            layout: &tile_material_layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: uniform_buffers[pi].as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::TextureView(&p.albedo_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::Sampler(&p.sampler),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 3,
                                    resource: wgpu::BindingResource::TextureView(
                                        &hallway_decal_view,
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 4,
                                    resource: wgpu::BindingResource::TextureView(&p.normal_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 5,
                                    resource: p.pbr_uniform_buffer.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 6,
                                    resource: wgpu::BindingResource::TextureView(
                                        &p.metallic_roughness_view,
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 7,
                                    resource: wgpu::BindingResource::TextureView(&p.emissive_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 8,
                                    resource: distortion_buffer.as_entire_binding(),
                                },
                            ],
                        })
                    })
                    .collect();
                let (shadow_uniform_buffers, shadow_bind_groups) =
                    create_room_env_shadow_gpu_batch(
                        &device,
                        &shadow_caster_layout,
                        prims.len(),
                        "hallway-env-shadow",
                    );
                let shadow_warp_bind_group = create_shadow_warp_bind_group(
                    &device,
                    &shadow_warp_layout,
                    &distortion_buffer,
                    "hallway-env-shadow-warp",
                );
                gpu_wrap = Some(ShopEnvironmentGpu {
                    uniform_buffers,
                    distortion_buffer,
                    shadow_uniform_buffers,
                    shadow_bind_groups,
                    shadow_warp_bind_group,
                    bind_groups,
                    archive_sign_decal_texture: None,
                    shop_candle_sss_texture: None,
                });
                log::info!("hallway.glb GPU: {} primitive draw(s)", prims.len());
            }
            (prims, gpu_wrap)
        })
    };

    crate::render::hallway_glb::release_hallway_environment_cpu_sources_after_gpu_upload();

    let (staircase_env_primitives, staircase_environment) = {
        let _staircase = crate::startup_profile::scope("wgpu.room.staircase");
        crate::render::staircase_glb::with_staircase_glb_cpu(|cpu_opt| {
            let mut prims = Vec::new();
            let mut gpu_wrap = None;
            let Some(cpu) = cpu_opt else {
                return (prims, gpu_wrap);
            };
            if !cpu.environment_primitives.is_empty() {
                let mut room_tex_cache = RoomEnvTextureCache::new();
                let (_white_tex, white_albedo_view) = white_albedo(&device, &queue);
                for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                    let prim = &env_prim.mesh;
                    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("staircase-env-verts-{i}")),
                        contents: bytemuck::cast_slice(&prim.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                    let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("staircase-env-idx-{i}")),
                        contents: bytemuck::cast_slice(&prim.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                    let mips = crate::render::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                    let albedo_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("staircase-env-albedo-{i}"),
                        prim.albedo_rgba.as_ref(),
                        prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8UnormSrgb,
                        mips,
                        &white_albedo_view,
                    );
                    let normal_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("staircase-env-normal-{i}"),
                        prim.normal_rgba.as_ref(),
                        prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8Unorm,
                        mips,
                        &tile_default_normal_view,
                    );
                    let metallic_roughness_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("staircase-env-mr-{i}"),
                        prim.metallic_roughness_rgba.as_ref(),
                        prim.metallic_roughness_mip_chain
                            .as_deref()
                            .map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8Unorm,
                        mips,
                        &tile_glb_default_mr_view,
                    );
                    let emissive_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("staircase-env-emissive-{i}"),
                        prim.emissive_rgba.as_ref(),
                        prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8UnormSrgb,
                        mips,
                        &tile_glb_default_emissive_view,
                    );
                    let pbr_uniform_buffer =
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("staircase-pbr-{i}")),
                            contents: bytemuck::bytes_of(&GltfPbrUniform::from_loaded(
                                prim.metallic_factor,
                                prim.roughness_factor,
                                prim.emissive_factor,
                                prim.alpha_mode,
                                prim.alpha_cutoff,
                            )),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                    let sampler =
                        device.create_sampler(&build_sampler_descriptor(prim.sampler, None));
                    prims.push(TilePrimitiveGpu {
                        vertex_buffer: vb,
                        index_buffer: ib,
                        index_count: prim.indices.len() as u32,
                        albedo_view,
                        normal_view,
                        metallic_roughness_view,
                        emissive_view,
                        pbr_uniform_buffer,
                        sampler,
                        pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                    });
                }
                let (_white_tex, staircase_decal_view) = white_albedo(&device, &queue);
                let uniform_buffers = create_room_env_camera_uniform_buffers(
                    &device,
                    prims.len(),
                    "staircase-env-uniform",
                );
                let distortion_buffer =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("staircase-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::render::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
                let bind_groups: Vec<wgpu::BindGroup> = prims
                    .iter()
                    .enumerate()
                    .map(|(pi, p)| {
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("staircase-env-bg"),
                            layout: &tile_material_layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: uniform_buffers[pi].as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::TextureView(&p.albedo_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::Sampler(&p.sampler),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 3,
                                    resource: wgpu::BindingResource::TextureView(
                                        &staircase_decal_view,
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 4,
                                    resource: wgpu::BindingResource::TextureView(&p.normal_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 5,
                                    resource: p.pbr_uniform_buffer.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 6,
                                    resource: wgpu::BindingResource::TextureView(
                                        &p.metallic_roughness_view,
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 7,
                                    resource: wgpu::BindingResource::TextureView(&p.emissive_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 8,
                                    resource: distortion_buffer.as_entire_binding(),
                                },
                            ],
                        })
                    })
                    .collect();
                let (shadow_uniform_buffers, shadow_bind_groups) =
                    create_room_env_shadow_gpu_batch(
                        &device,
                        &shadow_caster_layout,
                        prims.len(),
                        "staircase-env-shadow",
                    );
                let shadow_warp_bind_group = create_shadow_warp_bind_group(
                    &device,
                    &shadow_warp_layout,
                    &distortion_buffer,
                    "staircase-env-shadow-warp",
                );
                gpu_wrap = Some(ShopEnvironmentGpu {
                    uniform_buffers,
                    distortion_buffer,
                    shadow_uniform_buffers,
                    shadow_bind_groups,
                    shadow_warp_bind_group,
                    bind_groups,
                    archive_sign_decal_texture: None,
                    shop_candle_sss_texture: None,
                });
                log::info!("staircase.glb GPU: {} primitive draw(s)", prims.len());
            }
            (prims, gpu_wrap)
        })
    };

    crate::render::staircase_glb::release_staircase_environment_cpu_sources_after_gpu_upload();

    let (
        archive_env_primitives,
        archive_environment,
        archive_sign_left_prim_idx,
        archive_sign_right_prim_idx,
        archive_page_left_prim_indices,
        archive_page_right_prim_indices,
        archive_env_shadow_caster_mask,
    ) = {
        let _archive = crate::startup_profile::scope("wgpu.room.archive");
        crate::render::archive_glb::with_archive_glb_cpu(|cpu_opt| {
            let mut prims = Vec::new();
            let mut gpu_wrap = None;
            let mut sign_l = None;
            let mut sign_r = None;
            let mut page_left = Vec::new();
            let mut page_right = Vec::new();
            let mut shadow_caster_mask = Vec::new();
            let Some(cpu) = cpu_opt else {
                return (
                    prims,
                    gpu_wrap,
                    sign_l,
                    sign_r,
                    page_left,
                    page_right,
                    shadow_caster_mask,
                );
            };
            if !cpu.environment_primitives.is_empty() {
                let mut room_tex_cache = RoomEnvTextureCache::new();
                let (_white_tex, white_albedo_view) = white_albedo(&device, &queue);
                for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                    if let Some(ref name) = env_prim.gltf_node_name {
                        if name == crate::render::archive_glb::SIGN_DESCRIPTION_LEFT {
                            sign_l = Some(i);
                        } else if name == crate::render::archive_glb::SIGN_DESCRIPTION_RIGHT {
                            sign_r = Some(i);
                        } else if name == crate::render::archive_glb::BTN_PAGE_LEFT {
                            page_left.push(i);
                        } else if name == crate::render::archive_glb::BTN_PAGE_RIGHT {
                            page_right.push(i);
                        }
                    }
                    let prim = &env_prim.mesh;
                    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("archive-env-verts-{i}")),
                        contents: bytemuck::cast_slice(&prim.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                    let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("archive-env-idx-{i}")),
                        contents: bytemuck::cast_slice(&prim.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                    let mips = crate::render::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                    let albedo_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("archive-env-albedo-{i}"),
                        prim.albedo_rgba.as_ref(),
                        prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8UnormSrgb,
                        mips,
                        &white_albedo_view,
                    );
                    let normal_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("archive-env-normal-{i}"),
                        prim.normal_rgba.as_ref(),
                        prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8Unorm,
                        mips,
                        &tile_default_normal_view,
                    );
                    let metallic_roughness_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("archive-env-mr-{i}"),
                        prim.metallic_roughness_rgba.as_ref(),
                        prim.metallic_roughness_mip_chain
                            .as_deref()
                            .map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8Unorm,
                        mips,
                        &tile_glb_default_mr_view,
                    );
                    let emissive_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("archive-env-emissive-{i}"),
                        prim.emissive_rgba.as_ref(),
                        prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8UnormSrgb,
                        mips,
                        &tile_glb_default_emissive_view,
                    );
                    let pbr_uniform_buffer =
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("archive-pbr-{i}")),
                            contents: bytemuck::bytes_of(&GltfPbrUniform::from_loaded(
                                prim.metallic_factor,
                                prim.roughness_factor,
                                prim.emissive_factor,
                                prim.alpha_mode,
                                prim.alpha_cutoff,
                            )),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                    let sampler =
                        device.create_sampler(&build_sampler_descriptor(prim.sampler, None));
                    shadow_caster_mask.push(
                        crate::render::archive_glb::archive_prim_casts_room_shadow(
                            env_prim.gltf_node_name.as_deref(),
                        ),
                    );
                    prims.push(TilePrimitiveGpu {
                        vertex_buffer: vb,
                        index_buffer: ib,
                        index_count: prim.indices.len() as u32,
                        albedo_view,
                        normal_view,
                        metallic_roughness_view,
                        emissive_view,
                        pbr_uniform_buffer,
                        sampler,
                        pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                    });
                }
                let (sign_decal_w, sign_decal_h) = crate::render::decal::decal_dimensions(
                    &crate::render::primitive::DecalLayout::Fit {
                        target_short_edge: crate::render::decal::PLAQUE_DECAL_HEIGHT,
                    },
                    crate::render::archive_glb::archive_sign_description_decal_extents_for(cpu),
                );
                let sign_decal_clear = vec![0u8; (sign_decal_w * sign_decal_h * 4) as usize];
                let archive_sign_decal_tex = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("archive-sign-decal"),
                    size: wgpu::Extent3d {
                        width: sign_decal_w,
                        height: sign_decal_h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8UnormSrgb,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &archive_sign_decal_tex,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    &sign_decal_clear,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(4 * sign_decal_w),
                        rows_per_image: Some(sign_decal_h),
                    },
                    wgpu::Extent3d {
                        width: sign_decal_w,
                        height: sign_decal_h,
                        depth_or_array_layers: 1,
                    },
                );
                let archive_decal_view =
                    archive_sign_decal_tex.create_view(&wgpu::TextureViewDescriptor::default());
                let uniform_buffers = create_room_env_camera_uniform_buffers(
                    &device,
                    prims.len(),
                    "archive-env-uniform",
                );
                let distortion_buffer =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("archive-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::render::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
                let bind_groups: Vec<wgpu::BindGroup> = prims
                    .iter()
                    .enumerate()
                    .map(|(pi, p)| {
                        // Only description boards sample the CPU decal atlas; other room
                        // meshes bind a 1×1 white stub so a shared atlas cannot leak.
                        let decal_view = if sign_l == Some(pi) || sign_r == Some(pi) {
                            &archive_decal_view
                        } else {
                            &white_albedo_view
                        };
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("archive-env-bg"),
                            layout: &tile_material_layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: uniform_buffers[pi].as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::TextureView(&p.albedo_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::Sampler(&p.sampler),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 3,
                                    resource: wgpu::BindingResource::TextureView(decal_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 4,
                                    resource: wgpu::BindingResource::TextureView(&p.normal_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 5,
                                    resource: p.pbr_uniform_buffer.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 6,
                                    resource: wgpu::BindingResource::TextureView(
                                        &p.metallic_roughness_view,
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 7,
                                    resource: wgpu::BindingResource::TextureView(&p.emissive_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 8,
                                    resource: distortion_buffer.as_entire_binding(),
                                },
                            ],
                        })
                    })
                    .collect();
                let (shadow_uniform_buffers, shadow_bind_groups) =
                    create_room_env_shadow_gpu_batch(
                        &device,
                        &shadow_caster_layout,
                        prims.len(),
                        "archive-env-shadow",
                    );
                let shadow_warp_bind_group = create_shadow_warp_bind_group(
                    &device,
                    &shadow_warp_layout,
                    &distortion_buffer,
                    "archive-env-shadow-warp",
                );
                gpu_wrap = Some(ShopEnvironmentGpu {
                    uniform_buffers,
                    distortion_buffer,
                    shadow_uniform_buffers,
                    shadow_bind_groups,
                    shadow_warp_bind_group,
                    bind_groups,
                    archive_sign_decal_texture: Some(archive_sign_decal_tex),
                    shop_candle_sss_texture: None,
                });
                log::info!("archive.glb GPU: {} primitive draw(s)", prims.len());
            }
            (
                prims,
                gpu_wrap,
                sign_l,
                sign_r,
                page_left,
                page_right,
                shadow_caster_mask,
            )
        })
    };

    crate::render::archive_glb::release_archive_environment_cpu_sources_after_gpu_upload();

    let (main_menu_env_primitives, main_menu_environment) = {
        let _menu = crate::startup_profile::scope("wgpu.room.main_menu");
        crate::render::main_menu_glb::with_main_menu_glb_cpu(|cpu_opt| {
            let mut prims = Vec::new();
            let mut gpu_wrap = None;
            let Some(cpu) = cpu_opt else {
                return (prims, gpu_wrap);
            };
            if !cpu.environment_primitives.is_empty() {
                let mut room_tex_cache = RoomEnvTextureCache::new();
                let (_white_tex, white_albedo_view) = white_albedo(&device, &queue);
                for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                    let prim = &env_prim.mesh;
                    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("main_menu-env-verts-{i}")),
                        contents: bytemuck::cast_slice(&prim.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                    let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("main_menu-env-idx-{i}")),
                        contents: bytemuck::cast_slice(&prim.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                    let mips = crate::render::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                    let albedo_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("main_menu-env-albedo-{i}"),
                        prim.albedo_rgba.as_ref(),
                        prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8UnormSrgb,
                        mips,
                        &white_albedo_view,
                    );
                    let normal_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("main_menu-env-normal-{i}"),
                        prim.normal_rgba.as_ref(),
                        prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8Unorm,
                        mips,
                        &tile_default_normal_view,
                    );
                    let metallic_roughness_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("main_menu-env-mr-{i}"),
                        prim.metallic_roughness_rgba.as_ref(),
                        prim.metallic_roughness_mip_chain
                            .as_deref()
                            .map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8Unorm,
                        mips,
                        &tile_glb_default_mr_view,
                    );
                    let emissive_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("main_menu-env-emissive-{i}"),
                        prim.emissive_rgba.as_ref(),
                        prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8UnormSrgb,
                        mips,
                        &tile_glb_default_emissive_view,
                    );
                    let pbr_uniform_buffer =
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("main_menu-pbr-{i}")),
                            contents: bytemuck::bytes_of(&GltfPbrUniform::from_loaded(
                                prim.metallic_factor,
                                prim.roughness_factor,
                                prim.emissive_factor,
                                prim.alpha_mode,
                                prim.alpha_cutoff,
                            )),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                    let sampler =
                        device.create_sampler(&build_sampler_descriptor(prim.sampler, None));
                    prims.push(TilePrimitiveGpu {
                        vertex_buffer: vb,
                        index_buffer: ib,
                        index_count: prim.indices.len() as u32,
                        albedo_view,
                        normal_view,
                        metallic_roughness_view,
                        emissive_view,
                        pbr_uniform_buffer,
                        sampler,
                        pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                    });
                }
                let (_white_tex, main_menu_decal_view) = white_albedo(&device, &queue);
                let uniform_buffers = create_room_env_camera_uniform_buffers(
                    &device,
                    prims.len(),
                    "main_menu-env-uniform",
                );
                let distortion_buffer =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("main_menu-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::render::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
                let bind_groups: Vec<wgpu::BindGroup> = prims
                    .iter()
                    .enumerate()
                    .map(|(pi, p)| {
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("main_menu-env-bg"),
                            layout: &tile_material_layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: uniform_buffers[pi].as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::TextureView(&p.albedo_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::Sampler(&p.sampler),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 3,
                                    resource: wgpu::BindingResource::TextureView(
                                        &main_menu_decal_view,
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 4,
                                    resource: wgpu::BindingResource::TextureView(&p.normal_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 5,
                                    resource: p.pbr_uniform_buffer.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 6,
                                    resource: wgpu::BindingResource::TextureView(
                                        &p.metallic_roughness_view,
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 7,
                                    resource: wgpu::BindingResource::TextureView(&p.emissive_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 8,
                                    resource: distortion_buffer.as_entire_binding(),
                                },
                            ],
                        })
                    })
                    .collect();
                let (shadow_uniform_buffers, shadow_bind_groups) =
                    create_room_env_shadow_gpu_batch(
                        &device,
                        &shadow_caster_layout,
                        prims.len(),
                        "main_menu-env-shadow",
                    );
                let shadow_warp_bind_group = create_shadow_warp_bind_group(
                    &device,
                    &shadow_warp_layout,
                    &distortion_buffer,
                    "main_menu-env-shadow-warp",
                );
                gpu_wrap = Some(ShopEnvironmentGpu {
                    uniform_buffers,
                    distortion_buffer,
                    shadow_uniform_buffers,
                    shadow_bind_groups,
                    shadow_warp_bind_group,
                    bind_groups,
                    archive_sign_decal_texture: None,
                    shop_candle_sss_texture: None,
                });
                log::info!("main_menu.glb GPU: {} primitive draw(s)", prims.len());
            }
            (prims, gpu_wrap)
        })
    };

    crate::render::main_menu_glb::release_main_menu_environment_cpu_sources_after_gpu_upload();

    let main_menu_env_collision_meshes =
        crate::render::main_menu_glb::with_main_menu_glb_cpu(|opt| {
            opt.map(|c| c.collision_meshes.clone()).unwrap_or_default()
        });

    let (gameplay_env_primitives, gameplay_environment, gameplay_cash_in_prim_indices, gameplay_env_shadow_caster_mask) = {
        let _gameplay = crate::startup_profile::scope("wgpu.room.gameplay");
        crate::render::gameplay_glb::with_gameplay_glb_cpu(|cpu_opt| {
            let mut prims = Vec::new();
            let mut gpu_wrap = None;
            let mut gameplay_cash_in_prim_indices = Vec::new();
            let mut gameplay_env_shadow_caster_mask = Vec::new();
            let Some(cpu) = cpu_opt else {
                return (
                    prims,
                    gpu_wrap,
                    gameplay_cash_in_prim_indices,
                    gameplay_env_shadow_caster_mask,
                );
            };
            if !cpu.environment_primitives.is_empty() {
                let mut room_tex_cache = RoomEnvTextureCache::new();
                let (_white_tex, white_albedo_view) = white_albedo(&device, &queue);
                for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                    if let Some(ref name) = env_prim.gltf_node_name {
                        if matches!(
                            name.as_str(),
                            crate::render::gameplay_glb::BTN_CASH_IN
                                | crate::render::gameplay_glb::LABEL_CASH_IN
                        ) {
                            gameplay_cash_in_prim_indices.push(i);
                        }
                    }
                    gameplay_env_shadow_caster_mask.push(
                        crate::render::gameplay_glb::gameplay_prim_casts_room_shadow(
                            env_prim.gltf_node_name.as_deref(),
                        ),
                    );
                    let prim = &env_prim.mesh;
                    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("gameplay-env-verts-{i}")),
                        contents: bytemuck::cast_slice(&prim.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                    let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(&format!("gameplay-env-idx-{i}")),
                        contents: bytemuck::cast_slice(&prim.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                    let mips = crate::render::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                    let albedo_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("gameplay-env-albedo-{i}"),
                        prim.albedo_rgba.as_ref(),
                        prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8UnormSrgb,
                        mips,
                        &white_albedo_view,
                    );
                    let normal_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("gameplay-env-normal-{i}"),
                        prim.normal_rgba.as_ref(),
                        prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8Unorm,
                        mips,
                        &tile_default_normal_view,
                    );
                    let metallic_roughness_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("gameplay-env-mr-{i}"),
                        prim.metallic_roughness_rgba.as_ref(),
                        prim.metallic_roughness_mip_chain
                            .as_deref()
                            .map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8Unorm,
                        mips,
                        &tile_glb_default_mr_view,
                    );
                    let emissive_view = room_tex_cache.upload_slot(
                        &device,
                        &queue,
                        format!("gameplay-env-emissive-{i}"),
                        prim.emissive_rgba.as_ref(),
                        prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                        wgpu::TextureFormat::Rgba8UnormSrgb,
                        mips,
                        &tile_glb_default_emissive_view,
                    );
                    let pbr_uniform_buffer =
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("gameplay-pbr-{i}")),
                            contents: bytemuck::bytes_of(&GltfPbrUniform::from_loaded(
                                prim.metallic_factor,
                                prim.roughness_factor,
                                prim.emissive_factor,
                                prim.alpha_mode,
                                prim.alpha_cutoff,
                            )),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                    let sampler =
                        device.create_sampler(&build_sampler_descriptor(prim.sampler, None));
                    prims.push(TilePrimitiveGpu {
                        vertex_buffer: vb,
                        index_buffer: ib,
                        index_count: prim.indices.len() as u32,
                        albedo_view,
                        normal_view,
                        metallic_roughness_view,
                        emissive_view,
                        pbr_uniform_buffer,
                        sampler,
                        pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                    });
                }
                let (_white_tex, gameplay_decal_view) = white_albedo(&device, &queue);
                let gameplay_candle_sss_tex = load_room_env_png_texture(
                    &device,
                    &queue,
                    crate::render::room_env_gltf::SHOP_CANDLE_SSS_BAKE_TEXTURE,
                    "gameplay-candle-sss-bake",
                    wgpu::TextureFormat::Rgba8Unorm,
                );
                let gameplay_candle_sss_view = gameplay_candle_sss_tex
                    .as_ref()
                    .map(|(_, v)| v)
                    .unwrap_or(&gameplay_decal_view);
                let uniform_buffers = create_room_env_camera_uniform_buffers(
                    &device,
                    prims.len(),
                    "gameplay-env-uniform",
                );
                let distortion_buffer =
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("gameplay-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::render::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
                let bind_groups: Vec<wgpu::BindGroup> = prims
                    .iter()
                    .enumerate()
                    .map(|(pi, p)| {
                        let is_candle = cpu
                            .environment_primitives
                            .get(pi)
                            .and_then(|ep| ep.gltf_node_name.as_deref())
                            .is_some_and(
                                crate::render::room_env_gltf::is_shop_candle_wax_node_name,
                            );
                        let decal_view = if is_candle {
                            gameplay_candle_sss_view
                        } else {
                            &gameplay_decal_view
                        };
                        device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("gameplay-env-bg"),
                            layout: &tile_material_layout,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: uniform_buffers[pi].as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::TextureView(&p.albedo_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::Sampler(&p.sampler),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 3,
                                    resource: wgpu::BindingResource::TextureView(&decal_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 4,
                                    resource: wgpu::BindingResource::TextureView(&p.normal_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 5,
                                    resource: p.pbr_uniform_buffer.as_entire_binding(),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 6,
                                    resource: wgpu::BindingResource::TextureView(
                                        &p.metallic_roughness_view,
                                    ),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 7,
                                    resource: wgpu::BindingResource::TextureView(&p.emissive_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 8,
                                    resource: distortion_buffer.as_entire_binding(),
                                },
                            ],
                        })
                    })
                    .collect();
                let (shadow_uniform_buffers, shadow_bind_groups) =
                    create_room_env_shadow_gpu_batch(
                        &device,
                        &shadow_caster_layout,
                        prims.len(),
                        "gameplay-env-shadow",
                    );
                let shadow_warp_bind_group = create_shadow_warp_bind_group(
                    &device,
                    &shadow_warp_layout,
                    &distortion_buffer,
                    "gameplay-env-shadow-warp",
                );
                gpu_wrap = Some(ShopEnvironmentGpu {
                    uniform_buffers,
                    distortion_buffer,
                    shadow_uniform_buffers,
                    shadow_bind_groups,
                    shadow_warp_bind_group,
                    bind_groups,
                    archive_sign_decal_texture: None,
                    shop_candle_sss_texture: gameplay_candle_sss_tex.map(|(t, _)| t),
                });
                log::info!("gameplay.glb GPU: {} primitive draw(s)", prims.len());
                return (
                    prims,
                    gpu_wrap,
                    gameplay_cash_in_prim_indices,
                    gameplay_env_shadow_caster_mask,
                );
            }
            (
                prims,
                gpu_wrap,
                gameplay_cash_in_prim_indices,
                gameplay_env_shadow_caster_mask,
            )
        })
    };

    crate::render::gameplay_glb::release_gameplay_environment_cpu_sources_after_gpu_upload();

    let gameplay_env_collision_meshes = crate::render::gameplay_glb::with_gameplay_glb_cpu(|opt| {
        opt.map(|c| c.collision_meshes.clone()).unwrap_or_default()
    });

    let shop_env_collision_meshes = crate::render::room_glb::with_shop_glb_cpu(|opt| {
        opt.map(|c| c.collision_meshes.clone()).unwrap_or_default()
    });

    // Kick off background relic image loading (non-blocking).
    let relic_load_start = Some(Instant::now());
    let relic_rx = Some(spawn_relic_loader());
    let (_lit_mesh_relief_default_tex, lit_mesh_relief_default_view) =
        flat_relief_height(&device, &queue);
    let pack_textures_map = {
        let _pack = crate::startup_profile::scope("wgpu.pack_textures");
        load_pack_textures(&device, &queue, &lit_mesh_relief_default_view)
    };
    let (background_load_start, background_rx) =
        if super::resources::ASYNC_LOADED_BACKGROUNDS.is_empty() {
            (None, None)
        } else {
            (Some(Instant::now()), Some(spawn_background_loader()))
        };

    // ---- Lit-mesh procedural geometry (candles) ----
    let t_lit_meshes = Instant::now();
    let relic_box_cpu = build_relic_mesh();
    let relic_box_tris: Vec<[glam::Vec3; 3]> = relic_box_cpu
        .indices
        .chunks_exact(3)
        .map(|c| {
            let a = relic_box_cpu.vertices[c[0] as usize].position;
            let b = relic_box_cpu.vertices[c[1] as usize].position;
            let d = relic_box_cpu.vertices[c[2] as usize].position;
            [
                glam::Vec3::from(a),
                glam::Vec3::from(b),
                glam::Vec3::from(d),
            ]
        })
        .collect();
    let relic_box_mesh = LitMeshGpu::new(&device, &relic_box_cpu, "relic-mesh");
    let pack_mesh = LitMeshGpu::new(&device, &build_pack_mesh(), "pack-mesh");
    let ribbon_mesh = LitMeshGpu::new(&device, &build_ribbon_mesh(), "ribbon");
    let talisman_mesh = LitMeshGpu::new(&device, &build_talisman_mesh(), "talisman");
    let bug_body_mesh = LitMeshGpu::new(&device, &build_bug_body_mesh(), "bug-body");
    let bug_wing_mesh = LitMeshGpu::new(&device, &build_bug_wing_mesh(), "bug-wing");
    let bug_wing_blur_mesh = LitMeshGpu::new(&device, &build_bug_wing_blur_mesh(), "bug-wing-blur");
    // Phase-1 primitive registry: parallel GPU copies of meshes
    // the generic `Object3dKind::Primitive` dispatch can reach by
    // `MeshId`. Legacy named fields above still own their own
    // allocations during the migration window.
    let mut primitive_meshes: rustc_hash::FxHashMap<MeshId, std::sync::Arc<LitMeshGpu>> =
        rustc_hash::FxHashMap::default();
    {
        let unit_cube_cpu = {
            let mut verts: Vec<crate::render::tile_glb::Vertex3dTex> = Vec::new();
            let mut idx: Vec<u32> = Vec::new();
            push_box(
                &mut verts,
                &mut idx,
                Aabb::new(-0.5, 0.5, -0.5, 0.5, -0.5, 0.5),
            );
            MeshCpu {
                vertices: verts,
                indices: idx,
                default_material: MaterialParams {
                    kind: MaterialKind::Plain,
                    base_color: [1.0, 1.0, 1.0, 1.0],
                    specular_strength: 0.25,
                    specular_power: 32.0,
                },
            }
        };
        primitive_meshes.insert(
            MeshId::Cube,
            std::sync::Arc::new(LitMeshGpu::new(&device, &unit_cube_cpu, "primitive-cube")),
        );
        primitive_meshes.insert(
            MeshId::BeveledSlab,
            std::sync::Arc::new(LitMeshGpu::new(
                &device,
                &build_plaque_mesh(),
                "primitive-slab",
            )),
        );
        primitive_meshes.insert(
            MeshId::CabinetColumn,
            std::sync::Arc::new(LitMeshGpu::new(
                &device,
                &build_cabinet_mesh(),
                "primitive-cabinet-column",
            )),
        );
        primitive_meshes.insert(
            MeshId::CabinetRails,
            std::sync::Arc::new(LitMeshGpu::new(
                &device,
                &build_cabinet_rails_mesh(),
                "primitive-cabinet-rails",
            )),
        );
        primitive_meshes.insert(
            MeshId::ShopActionProp,
            std::sync::Arc::new(LitMeshGpu::new(
                &device,
                &build_shop_action_prop_mesh(),
                "primitive-shop-action-prop",
            )),
        );
        primitive_meshes.insert(
            MeshId::DiscSquare,
            std::sync::Arc::new(LitMeshGpu::new(
                &device,
                &build_dish_mesh(),
                "primitive-dish-square",
            )),
        );
        primitive_meshes.insert(
            MeshId::DiscRound,
            std::sync::Arc::new(LitMeshGpu::new(
                &device,
                &build_round_dish_mesh(),
                "primitive-dish-round",
            )),
        );
        primitive_meshes.insert(
            MeshId::PorcelainDish,
            std::sync::Arc::new(LitMeshGpu::new(
                &device,
                &build_porcelain_dish_mesh(),
                "primitive-porcelain-dish",
            )),
        );
        // Cylinder is sized by `Object3d::extents` — reuse the coin
        // mesh (Y-up unit cylinder) so callers pay nothing extra.
        primitive_meshes.insert(
            MeshId::Cylinder,
            std::sync::Arc::new(LitMeshGpu::new(
                &device,
                &build_coin_mesh(),
                "primitive-cylinder",
            )),
        );
        primitive_meshes.insert(
            MeshId::Ofuda,
            std::sync::Arc::new(LitMeshGpu::new(
                &device,
                &build_ofuda_mesh(),
                "primitive-ofuda",
            )),
        );
        primitive_meshes.insert(
            MeshId::Abacus,
            std::sync::Arc::new(LitMeshGpu::new(
                &device,
                &build_abacus_mesh(),
                "primitive-abacus",
            )),
        );
        primitive_meshes.insert(
            MeshId::AbacusHeavenBeads,
            std::sync::Arc::new(LitMeshGpu::new(
                &device,
                &build_abacus_heaven_beads_mesh(),
                "primitive-abacus-heaven-beads",
            )),
        );
        primitive_meshes.insert(
            MeshId::AbacusEarthBeads,
            std::sync::Arc::new(LitMeshGpu::new(
                &device,
                &build_abacus_earth_beads_mesh(),
                "primitive-abacus-earth-beads",
            )),
        );
        primitive_meshes.insert(
            MeshId::ShopBell,
            std::sync::Arc::new(LitMeshGpu::new(
                &device,
                &build_shop_bell_mesh(),
                "primitive-shop-bell",
            )),
        );
        primitive_meshes.insert(
            MeshId::BellTassel,
            std::sync::Arc::new(LitMeshGpu::new(
                &device,
                &build_bell_tassel_mesh(),
                "primitive-bell-tassel",
            )),
        );
        primitive_meshes.insert(
            MeshId::ProgressMeterRail,
            std::sync::Arc::new(LitMeshGpu::new(
                &device,
                &build_progress_meter_rail_mesh(),
                "primitive-progress-meter-rail",
            )),
        );
        primitive_meshes.insert(
            MeshId::ProgressMeterPip,
            std::sync::Arc::new(LitMeshGpu::new(
                &device,
                &build_progress_meter_pip_mesh(),
                "primitive-progress-meter-pip",
            )),
        );
    }
    // Per-shape texture override: the coin cylinder needs its
    // engraved heightmap bound at both albedo and relief slots so
    // the Metal branch in lit_mesh.wgsl can sample the cash-coin
    // relief. Populated now so `primitive_textures` is ready by
    // the time `dispatch_primitive` first creates an instance.
    let mut primitive_textures: rustc_hash::FxHashMap<
        crate::render::primitive::MeshId,
        (wgpu::TextureView, wgpu::TextureView),
    > = rustc_hash::FxHashMap::default();
    let bone_tablet_mesh = LitMeshGpu::new(&device, &build_bone_tablet_mesh(), "bone-tablet");
    let wood_tablet_mesh = LitMeshGpu::new(&device, &build_wood_tablet_mesh(), "wood-tablet");
    let book_mesh = LitMeshGpu::new(&device, &build_book_body_mesh(), "book");
    let book_cover_mesh = LitMeshGpu::new(&device, &build_book_cover_mesh(), "book-cover");
    // The legacy "bowl" slot now hosts the discard river mesh — a stone
    // trough with an animated water surface. Field/variant names stayed
    // (`bowl_mesh`, `BowlPlacement`, `GameplayPick::DiscardBowl`) to keep
    // this swap to a single mesh substitution; renaming is a follow-up.
    let bowl_mesh = LitMeshGpu::new(&device, &build_river_mesh(), "river");
    let mirror_mesh = LitMeshGpu::new(&device, &build_mirror_mesh(), "mirror");
    let tally_stick_base_mesh =
        LitMeshGpu::new(&device, &build_tally_stick_base_mesh(), "tally-stick-base");
    let tally_stick_tip_mesh =
        LitMeshGpu::new(&device, &build_tally_stick_tip_mesh(), "tally-stick-tip");
    // Shared 1×1 white texture for procedural meshes that don't sample.
    let (_lit_mesh_white_tex, lit_mesh_white_view) = white_albedo(&device, &queue);

    let mut relic_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_RELIC_SLOTS);
    for _ in 0..MAX_RELIC_SLOTS {
        relic_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
    }
    let mut ordeal_icon_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_ORDEAL_ICON_SLOTS);
    for _ in 0..MAX_ORDEAL_ICON_SLOTS {
        ordeal_icon_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
    }
    let mut pack_instances: Vec<LitMeshInstance> = Vec::with_capacity(4);
    for _ in 0..4 {
        pack_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
    }
    let ribbon_zodiac_tex = load_zodiac_ribbon_textures(&device, &queue);
    let mut ribbon_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_RIBBON_SLOTS);
    for _ in 0..MAX_RIBBON_SLOTS {
        ribbon_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
    }
    let ribbon_slot_zodiac: Vec<Option<u8>> = vec![None; MAX_RIBBON_SLOTS];
    // Coin face heightmap (Chinese cash-coin engraving). Bound at slot 1
    // of every shop-pile coin instance; the metal branch in lit_mesh.wgsl
    // samples it as a heightfield to perturb the coin's surface normal so
    // the engraved characters and rim catch the candle highlights. Pegs
    // reuse coin geometry but keep the white texture and `Plain` material
    // so they aren't affected.
    let (_lit_mesh_coin_height_tex, lit_mesh_coin_height_view) =
        load_coin_heightmap(&device, &queue);
    // Register the coin heightmap as the per-shape texture override
    // for Cylinder primitives so engraved-coin callers sample it.
    primitive_textures.insert(
        crate::render::primitive::MeshId::Cylinder,
        (
            lit_mesh_coin_height_view.clone(),
            lit_mesh_coin_height_view.clone(),
        ),
    );
    let mut bug_body_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
    let mut bug_wing_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
    let mut bug_wing_r_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
    let mut bug_wing_blur_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
    let mut bug_wing_blur_r_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
    for _ in 0..MAX_BUG_SLOTS {
        bug_body_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
        bug_wing_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
        bug_wing_r_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
        bug_wing_blur_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
        bug_wing_blur_r_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
    }
    let orb_mesh = LitMeshGpu::new(&device, &build_orb_mesh(), "material-orb");
    let mut orb_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_ORB_SLOTS);
    for _ in 0..MAX_ORB_SLOTS {
        orb_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
    }
    // Per-kind heightmap textures for talisman tablets. Each is a PNG
    // asset loaded from assets/textures/ and uploaded as a linear RGBA8
    // texture. Falls back to a flat mid-gray 1×1 if the asset is missing.
    // Order matches `TalismanKind::all()` — reuse art where dedicated assets
    // are not yet present.
    let talisman_height_paths = crate::core::talisman::TalismanKind::heightmap_paths();
    let mut talisman_height_views: Vec<wgpu::TextureView> = Vec::new();
    for &(path, label) in talisman_height_paths {
        let (_tex, view) = load_metal_heightmap(&device, &queue, path, label);
        talisman_height_views.push(view);
    }
    let talisman_mask_paths = crate::core::talisman::TalismanKind::mask_paths();
    let mut talisman_mask_views: Vec<wgpu::TextureView> = Vec::new();
    for &(path, label) in talisman_mask_paths {
        let (_tex, view) = load_metal_heightmap(&device, &queue, path, label);
        talisman_mask_views.push(view);
    }
    let memorial_talisman_height_paths =
        crate::core::memorial_talisman::MemorialTalismanKind::heightmap_paths();
    let mut memorial_talisman_height_views: Vec<wgpu::TextureView> = Vec::new();
    for &(path, label) in memorial_talisman_height_paths {
        let (_tex, view) = load_metal_heightmap(&device, &queue, path, label);
        memorial_talisman_height_views.push(view);
    }
    let memorial_talisman_mask_paths =
        crate::core::memorial_talisman::MemorialTalismanKind::mask_paths();
    let mut memorial_talisman_mask_views: Vec<wgpu::TextureView> = Vec::new();
    for &(path, label) in memorial_talisman_mask_paths {
        let (_tex, view) = load_metal_heightmap(&device, &queue, path, label);
        memorial_talisman_mask_views.push(view);
    }
    let talisman_slot_kind: Vec<Option<u8>> = vec![None; MAX_TALISMAN_SLOTS];
    let mut talisman_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_TALISMAN_SLOTS);
    for _ in 0..MAX_TALISMAN_SLOTS {
        talisman_instances.push(LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        ));
    }
    // ── Skeuomorphic gameplay HUD slot pools (phase 1) ─────────────
    let make_pool = |n: usize| -> Vec<LitMeshInstance> {
        (0..n)
            .map(|_| {
                LitMeshInstance::new(
                    &device,
                    &lit_mesh_material_layout,
                    &shadow_caster_layout,
                    &lit_mesh_white_view,
                    &lit_mesh_relief_default_view,
                    &tile_sampler,
                )
            })
            .collect()
    };
    // Plaque instances grow on demand via `ensure_plaque_slots` rather
    // than reserving a fixed cap — see that helper for context.
    // Cabinet instances grow on demand via `ensure_lit_mesh_pool` —
    // collection is the only consumer for now and only ever needs a
    // single instance, so reserving a fixed cap would be silly.
    let yaku_tablet_instances = make_pool(MAX_YAKU_TABLET_SLOTS);
    let wood_tablet_instances = make_pool(MAX_WOOD_TABLET_SLOTS);
    let book_instances = make_pool(MAX_BOOK_SLOTS);
    let book_cover_instances = make_pool(MAX_BOOK_SLOTS);
    let bowl_instances = make_pool(MAX_BOWL_SLOTS);
    // Bronze mirror face heightmap (Han/Tang four-spirit relief). Bound
    // at slot 1 of every mirror instance; the metal branch in
    // lit_mesh.wgsl samples it as a heightfield to perturb the polished
    // face's surface normal so the cast guardians and TLV marks catch
    // the candle highlights. Same setup as the coin pile above.
    let (_lit_mesh_mirror_height_tex, lit_mesh_mirror_height_view) =
        load_mirror_heightmap(&device, &queue);
    let mirror_instances: Vec<LitMeshInstance> = (0..MAX_MIRROR_SLOTS)
        .map(|_| {
            LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_mirror_height_view,
                &lit_mesh_mirror_height_view,
                &tile_sampler,
            )
        })
        .collect();
    // Each visible stick consumes two slots (bone + tip) so the pool is
    // sized at `2 × MAX_TALLY_STICK_SLOTS` to cover the worst case of
    // every slot populated.
    let tally_stick_instances = make_pool(MAX_TALLY_STICK_SLOTS * 2);
    let wall_tile_instances = make_pool(MAX_WALL_TILE_SLOTS);
    let extruded_glyph_instances = make_pool(MAX_EXTRUDED_GLYPH_SLOTS);
    let debug_axes_instances = make_pool(3);
    crate::startup_profile::record("wgpu.lit_meshes_and_pools", t_lit_meshes.elapsed());

    // Build the GPU profiler up-front while we still have a borrow of
    // device/queue (the struct literal below moves them).
    let gpu_profiler = crate::render::gpu_profiler::GpuProfiler::new(
        &device,
        &queue,
        timestamp_supported,
        gpu_profiler_backend,
    );

    queue.write_buffer(
        &tonemap_params_buffer,
        0,
        bytemuck::bytes_of(&TonemapParams {
            exposure: 1.0,
            mode: 0.0,
            vhs_enabled: 0.0,
            time: 0.0,
            vhs_chromatic: 0.0,
            vhs_scanline: 0.0,
            vhs_grain: 0.0,
            vhs_vignette: 0.0,
            grain_frame: 0.0,
        }),
    );

    // Per-frame bump-allocated buffer pool used by the highest-frequency
    // per-frame instance vertex uploads in `runtime/render.rs`. Created
    // before the struct literal so the `&device` borrow doesn't conflict
    // with `device` being moved into the `WgpuRenderer.device` field.
    let frame_buffer_pool =
        super::super::frame_pool::FrameBufferPool::new(&device, "frame-buffer-pool", 1 << 20);

    Ok(WgpuRenderer {
        target,
        device,
        queue,
        config,
        depth_texture,
        depth_view,
        ssr_prev_depth_texture,
        ssr_prev_depth_view,
        quad_pipeline,
        quad_pipeline_display,
        gradient_quad_pipeline,
        squircle_quad_pipeline,
        flame_pipeline,
        flame_volume_mesh,
        flame_view_buffer,
        flame_view_bind_group,
        flame_instance_staging: Vec::with_capacity(32),
        starfield_pipeline,
        ember_drift_pipeline,
        golden_dust_pipeline,
        moonlit_water_pipeline,
        moonlit_water_bind_group,
        sunlit_water_pipeline,
        shooting_star_cascade_pipeline,
        cascade_composite_pipeline,
        cascade_composite_layout,
        cascade_composite_sampler,
        cascade_offscreen_texture,
        cascade_offscreen_view,
        cascade_composite_bind_group,
        scene_color_downsample_pipeline,
        scene_color_downsample_bind_group,
        tile_pipeline_opaque_double,
        tile_pipeline_opaque_cull,
        tile_pipeline_blend_double,
        tile_pipeline_blend_cull,
        shop_pipeline_opaque_double,
        shop_pipeline_opaque_cull,
        shop_pipeline_blend_double,
        shop_pipeline_blend_cull,
        shop_pipeline_mrt_opaque_double,
        shop_pipeline_mrt_opaque_cull,
        shop_pipeline_mrt_blend_double,
        shop_pipeline_mrt_blend_cull,
        tile_outline_pipeline,
        tile_glow_pipeline,
        globals_buffer,
        globals_bind_group,
        tile_material_layout,
        tile_env_distortion_placeholder,
        tile_outline_frame_uniform_buffer,
        tile_outline_instance_buffer,
        tile_outline_frame_bind_group,
        tile_outline_instances_staging: Vec::new(),
        tile_outline_batch_ranges: Vec::new(),
        point_lights_buffer,
        tile_occluders_buffer,
        point_lights_bind_group,
        spot_lights_buffer,
        spot_lights_bind_group,
        tile_sampler,
        _tile_default_normal_texture: tile_default_normal_texture,
        _tile_glb_default_mr_texture: _tile_glb_default_mr_tex,
        _tile_glb_default_emissive_texture: _tile_glb_default_emissive_tex,
        tile_primitives,
        tile_outline_vertex_buffer,
        tile_outline_index_buffer,
        tile_outline_index_count,
        shop_env_primitives,
        shop_environment,
        shop_gltf_anim,
        shop_gltf_anim_missing_clip_warned: std::cell::Cell::new(false),
        shop_eyeball_prim_indices,
        hallway_env_primitives,
        hallway_environment,
        staircase_env_primitives,
        staircase_environment,
        archive_env_primitives,
        archive_environment,
        main_menu_env_primitives,
        main_menu_environment,
        gameplay_env_primitives,
        gameplay_cash_in_prim_indices,
        gameplay_env_shadow_caster_mask,
        gameplay_environment,
        archive_sign_left_prim_idx,
        archive_sign_right_prim_idx,
        archive_page_left_prim_indices,
        archive_page_right_prim_indices,
        archive_env_shadow_caster_mask,
        archive_sign_decal_upload_key: 0,
        frame_env_tunes: rustc_hash::FxHashMap::default(),
        active_frame_env: crate::game::scene_look_tuning::RoomEnvFrameTune::default(),
        shop_env_collision_meshes,
        main_menu_env_collision_meshes,
        gameplay_env_collision_meshes,
        tile_base_color_factor,
        // Populated on first render() from RenderSettings.tileset_name.
        tile_set: None,
        hand_tiles: Vec::new(),
        showcase_tiles: Vec::new(),
        tile_face_overlays: rustc_hash::FxHashMap::default(),
        image_quad_overlays: rustc_hash::FxHashMap::default(),
        image_quad_missing: rustc_hash::FxHashSet::default(),
        debuff_marker_overlay: None,
        text_label_cache: rustc_hash::FxHashMap::default(),
        text_cache_frame: 0,
        vertex_buffer,
        index_buffer,
        text_pipeline,
        text_pipeline_scene_hdr,
        text_bind_group_layout,
        text_overlay_pipeline_layout,
        text_shader_module: text_shader,
        image_shader_module: image_shader,
        image_pipeline,
        image_pipeline_scene_hdr,
        ui_font,
        emoji_font,
        ui_font_italic,
        mono_font,
        size,
        last_focus: usize::MAX,
        focus_spin: None,
        focus_t: Vec::new(),
        tile_anim_y: Vec::new(),
        tile_anim_x: Vec::new(),
        tile_uids: Vec::new(),
        proj: ProjectionCache::default(),
        last_pick_models: Vec::new(),
        last_pick_camera: None,
        last_gameplay_cash_in_button_visible: false,
        last_relic_models: Vec::new(),
        relic_slot_texture: vec![None; MAX_RELIC_SLOTS],
        ordeal_icon_instances,
        ordeal_icon_meshes: rustc_hash::FxHashMap::default(),
        ordeal_icon_textures: rustc_hash::FxHashMap::default(),
        ordeal_icon_slot_texture: vec![None; MAX_ORDEAL_ICON_SLOTS],
        pack_instances,
        pack_slot_texture: vec![None; 4],
        ribbon_mesh,
        talisman_mesh,
        ribbon_instances,
        ribbon_slot_zodiac,
        ribbon_zodiac_tex,
        talisman_instances,
        bug_body_mesh,
        bug_wing_mesh,
        bug_body_instances,
        bug_wing_instances,
        bug_wing_r_instances,
        bug_wing_blur_mesh,
        bug_wing_blur_instances,
        bug_wing_blur_r_instances,
        orb_mesh,
        orb_instances,
        last_ribbon_models: Vec::new(),
        last_ribbon_batch_slot_counts: Vec::new(),
        last_talisman_models: Vec::new(),
        last_aux_dish_aabbs: Vec::new(),
        bone_tablet_mesh,
        wood_tablet_mesh,
        book_mesh,
        book_cover_mesh,
        bowl_mesh,
        mirror_mesh,
        tally_stick_base_mesh,
        tally_stick_tip_mesh,
        yaku_tablet_instances,
        wood_tablet_instances,
        book_instances,
        book_cover_instances,
        bowl_instances,
        mirror_instances,
        tally_stick_instances,
        wall_tile_instances,
        extruded_glyph_instances,
        glyph_cpu_cache: crate::render::glyph_mesh::GlyphMeshCache::new(),
        extruded_glyph_meshes: rustc_hash::FxHashMap::default(),
        primitive_meshes,
        primitive_instances: rustc_hash::FxHashMap::default(),
        primitive_textures,
        last_primitive_pick_models: rustc_hash::FxHashMap::default(),
        debug_axes_instances,
        last_yaku_tablet_models: Vec::new(),
        last_wood_tablet_models: Vec::new(),
        last_bowl_model: None,
        last_mirror_model: None,
        active_scene_key: None,
        last_frame: Instant::now(),
        frame_dt: 0.0,
        obj3d_hover_state: rustc_hash::FxHashMap::default(),
        creation_time: Instant::now(),
        vhs_grain_frame: 0,
        relic_textures: rustc_hash::FxHashMap::default(),
        relic_rx,
        relic_load_start,
        relic_profile_mesh_cpu: std::time::Duration::ZERO,
        relic_profile_upload_cpu: std::time::Duration::ZERO,
        pack_textures: pack_textures_map,
        background_textures: rustc_hash::FxHashMap::default(),
        background_rx,
        background_load_start,
        prev_tile_world: rustc_hash::FxHashMap::default(),
        tile_uid_scratch: rustc_hash::FxHashSet::default(),
        prev_frame_shadows_enabled: false,
        cached_shadow_light_view_proj: glam::Mat4::IDENTITY.to_cols_array(),
        punctual_shadow_lights: Vec::new(),
        cached_punctual_shadow_hash: 0,
        shop_inspect_subject_shadow_slot: None,
        shadow_placement_anim_id: 0,
        placement_shadow_room: None,
        placement_shadow_casts: true,
        showcase_decal_atlas: None,
        showcase_decal_atlas_tileset: None,
        showcase_decal_atlas_cache: std::collections::VecDeque::new(),
        lit_mesh_material_layout,
        lit_mesh_spot_ssr_layout,
        lit_mesh_ssr_buffer,
        lit_mesh_spot_ssr_bind_group,
        lit_mesh_ssr_sampler,
        scene_prev_texture,
        scene_prev_view,
        scene_color_texture,
        scene_color_view,
        room_emissive_texture,
        room_emissive_view,
        emissive_gi_texture,
        emissive_gi_view,
        emissive_probe_update_pipeline,
        emissive_probe_update_bind_group_layout,
        emissive_probe_update_bind_group,
        emissive_probe_apply_pipeline,
        emissive_probe_apply_bind_group_layout,
        emissive_probe_apply_bind_group,
        probe_gi_frame_uniform_buffer,
        probe_sh_buffer,
        probe_gi_tick: 0,
        probe_gi_last_view_proj: glam::Mat4::IDENTITY.to_cols_array(),
        probe_gi_last_size: (0, 0),
        probe_gi_had_room: false,
        probe_gi_gpu_room: None,
        room_gi_capture_pending: None,
        room_gi_capture_meta: None,
        room_gi_captured: None,
        room_baked_shadow_gpu,
        active_room_baked_shadow: None,
        room_shadow_capture_pending: None,
        room_shadow_captured: None,
        emissive_gi_composite_pipeline,
        emissive_gi_composite_bind_group_layout,
        emissive_gi_composite_bind_group,
        journal_scene_texture,
        journal_scene_view,
        journal_scene_view_generation: 0,
        bloom_extract_pipeline,
        bloom_blur_pipeline,
        bloom_composite_pipeline,
        bloom_bind_group_layout,
        bloom_extract_bind_group_layout,
        bloom_composite_bind_group_layout,
        bloom_extract_params_buffer,
        bloom_blur_h_params_buffer,
        bloom_blur_v_params_buffer,
        bloom_composite_params_buffer,
        bloom_sampler,
        bloom_scene_bind_group,
        bloom_ping_bind_group,
        bloom_pong_bind_group,
        bloom_composite_bind_group,
        bloom_ping_texture,
        bloom_ping_view,
        bloom_pong_texture,
        bloom_pong_view,
        post_bloom_texture,
        post_bloom_view,
        tonemap_pipeline,
        tonemap_rgba16f_pipeline,
        tonemap_bind_group_layout,
        tonemap_params_buffer,
        tonemap_bind_group,
        tonemap_bind_group_scene,
        frame_buffer_pool,
        tonemap_shader_module,
        tonemap_pipeline_layout,
        swapchain_sdr_format,
        swapchain_hdr_available,
        tonemap_exposure: 1.0,
        tonemap_vhs_enabled: false,
        tonemap_vhs_chromatic: 0.001,
        tonemap_vhs_scanline: 0.040,
        tonemap_vhs_grain: 0.020,
        tonemap_vhs_vignette: 0.100,
        rain_tuning: crate::render::rain_tuning::RainTuning::load(),
        lit_mesh_pipeline,
        lit_mesh_blended_pipeline,
        lit_mesh_white_view,
        lit_mesh_relief_default_view,
        talisman_height_views,
        talisman_mask_views,
        memorial_talisman_height_views,
        memorial_talisman_mask_views,
        talisman_slot_kind,
        relic_box_mesh,
        relic_box_tris,
        relic_tri_lists: rustc_hash::FxHashMap::default(),
        pack_mesh,
        relic_meshes: rustc_hash::FxHashMap::default(),
        felt_shader_lod: 2.0,
        relic_instances,
        shadow_map_texture,
        shadow_map_view,
        shadow_caster_layout,
        shadow_warp_disabled_bind_group,
        shadow_globals_buffer,
        shadow_sample_bind_group,
        shadow_pipeline,
        shadow_pipeline_room_env,
        gpu_profiler,
        pending_screenshot: std::cell::Cell::new(None),
        acquire_telemetry: super::super::runtime::AcquireTelemetry::new(),
    })
}
