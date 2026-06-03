use super::super::*;

use crate::moths_to_a_light::{build_bug_body_mesh, build_bug_wing_blur_mesh, build_bug_wing_mesh};

pub(super) fn build_renderer_new(
    target_init: TargetInit,
    #[cfg(feature = "windowed")] present_boot_frame: bool,
    #[cfg(feature = "windowed")] boot_input_poll: Option<&mut dyn FnMut()>,
) -> anyhow::Result<WgpuRenderer> {
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
        depth_r32_snapshot_texture,
        depth_r32_snapshot_view,
    } = {
        let _early = crate::startup_profile::scope("wgpu.early_gpu");
        super::super::init_phases::early_gpu_and_depth(target_init)?
    };
    #[cfg(feature = "windowed")]
    let mut boot_splash: Option<super::super::boot_splash::BootSplash<'_>> = if present_boot_frame {
        Some(super::super::boot_splash::BootSplash::new(
            &device,
            &queue,
            format,
            size.width,
            size.height,
        )?)
    } else {
        None
    };
    #[cfg(feature = "windowed")]
    let mut boot_poll_slot = boot_input_poll;
    #[cfg(feature = "windowed")]
    super::super::boot_splash::boot_present(
        &mut boot_splash,
        &target,
        &config,
        0.06,
        &mut boot_poll_slot,
    );
    let depth_copy_staging_buffer = super::super::resources::create_depth_copy_staging(
        &device,
        config.width.max(1),
        config.height.max(1),
    );

    {
        let _bakes = crate::startup_profile::scope("wgpu.offline_bakes");
        crate::offline_bakes::require_all_at_startup()?;
    }
    #[cfg(feature = "windowed")]
    super::super::boot_splash::boot_present(
        &mut boot_splash,
        &target,
        &config,
        0.12,
        &mut boot_poll_slot,
    );

    // Linear HDR intermediate — main scene + bloom; tonemap maps to the swapchain format.
    let scene_hdr_format = SCENE_HDR_FORMAT;

    let t_shaders = Instant::now();
    let super::super::init_phases::RendererShaderPack {
        quad: shader,
        depth_quad: depth_quad_shader,
        depth_quad_debug: depth_quad_debug_shader,
        tile: tile_shader,
        shop: shop_shader,
        text: text_shader,
        gradient: gradient_shader,
        arc_ring: arc_ring_shader,
        squircle: squircle_shader,
        flame: flame_shader,
        starfield: starfield_shader,
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
            contents: bytemuck::bytes_of(&crate::hallway_glb::HallwayDistortion::default()),
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

    // ---- Shadow map resources (depth arrays + sampler + layouts) ----
    use crate::lit_mesh::create_shadow_depth_array;
    use crate::wgpu_renderer::constants::{MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS};
    use mahjuro_gfx_types::ShadowQuality;

    let default_shadow_quality = ShadowQuality::High;
    let point_shadow_array = create_shadow_depth_array(
        &device,
        "point-shadow-array",
        default_shadow_quality.point_map_size(),
        MAX_POINT_LIGHTS as u32,
    );
    let spot_shadow_array = create_shadow_depth_array(
        &device,
        "spot-shadow-array",
        default_shadow_quality.spot_map_size(),
        MAX_SPOT_LIGHTS as u32,
    );
    let shadow_compare_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
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
        contents: bytemuck::bytes_of(&crate::hallway_glb::HallwayDistortion::default()),
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
        contents: bytemuck::bytes_of(&ShadowGlobals::empty()),
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
    let shadow_baked_depth_dummy_texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("shadow-baked-depth-dummy"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let shadow_baked_depth_dummy_view =
        shadow_baked_depth_dummy_texture.create_view(&Default::default());
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &shadow_baked_depth_dummy_texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::bytes_of(&0.5f32),
        wgpu::TexelCopyBufferLayout {
            bytes_per_row: Some(4),
            rows_per_image: Some(1),
            ..Default::default()
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let shadow_sample_bind_group = crate::lit_mesh::create_shadow_sample_bind_group(
        &device,
        &shadow_sample_layout,
        "shadow-sample-bg",
        &shadow_globals_buffer,
        &point_shadow_array.array_view,
        &spot_shadow_array.array_view,
        &shadow_compare_sampler,
        &shadow_ao_white_view,
        &shadow_ao_sampler,
        &shadow_baked_depth_dummy_view,
    );
    let room_baked_shadow_gpu: [Option<impl_room_shadow::RoomBakedShadowGpu>;
        crate::room_gi_bake::ROOM_GI_ROOM_COUNT] = std::array::from_fn(|_| None);

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
    let depth_ui_test = wgpu::DepthStencilState {
        depth_compare: Some(wgpu::CompareFunction::LessEqual),
        ..depth_ui.clone()
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
    let depth_quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("depth-quad-pipeline"),
        layout: Some(&quad_layout),
        vertex: wgpu::VertexState {
            module: &depth_quad_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[vertex_layout.clone(), instance_layout.clone()],
        },
        fragment: Some(wgpu::FragmentState {
            module: &depth_quad_shader,
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
        depth_stencil: Some(depth_ui_test.clone()),
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    let depth_quad_pipeline_display =
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("depth-quad-pipeline-display"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &depth_quad_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout.clone(), instance_layout.clone()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &depth_quad_shader,
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
            depth_stencil: Some(depth_ui_test.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
    let depth_quad_debug_pipeline =
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("depth-quad-debug-pipeline"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &depth_quad_debug_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout.clone(), instance_layout.clone()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &depth_quad_debug_shader,
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
            depth_stencil: Some(depth_ui_test.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
    let depth_quad_debug_pipeline_display =
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("depth-quad-debug-pipeline-display"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &depth_quad_debug_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout.clone(), instance_layout.clone()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &depth_quad_debug_shader,
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
            depth_stencil: Some(depth_ui_test.clone()),
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

    let arc_ring_instance_layout = wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ArcRingQuadInstance>() as wgpu::BufferAddress,
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
            wgpu::VertexAttribute {
                offset: 48,
                shader_location: 4,
                format: wgpu::VertexFormat::Float32x4,
            },
        ],
    };

    let arc_ring_quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("arc-ring-quad-pipeline"),
        layout: Some(&quad_layout),
        vertex: wgpu::VertexState {
            module: &arc_ring_shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[vertex_layout.clone(), arc_ring_instance_layout.clone()],
        },
        fragment: Some(wgpu::FragmentState {
            module: &arc_ring_shader,
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
    let arc_ring_quad_pipeline_display =
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("arc-ring-quad-pipeline-display"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &arc_ring_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout.clone(), arc_ring_instance_layout],
            },
            fragment: Some(wgpu::FragmentState {
                module: &arc_ring_shader,
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
    let squircle_quad_pipeline_display =
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("squircle-quad-pipeline-display"),
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
        array_stride: std::mem::size_of::<crate::tile_glb::Vertex3dTex>() as wgpu::BufferAddress,
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
        array_stride: std::mem::size_of::<crate::flame_volume::GpuFlameInstance>()
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
    let flame_volume_mesh = crate::lit_mesh::LitMeshGpu::new(
        &device,
        &crate::candle_flame_mesh::build_candle_flame_volume_mesh(),
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
            tuning: crate::flame_tuning::FlameTuning::load().shader_fields(),
            _pad: [0.0; 3],
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
    // Starfield and golden-dust share the same
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
            hdr_tonemap: [0.0; 4],
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
        // Bias lives in the lit shaders (`shadow_globals.params.y`); keep the
        // depth pass un-biased so compare refs match stored texels.
        bias: wgpu::DepthBiasState {
            constant: 0,
            slope_scale: 0.0,
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
            // Front-facing geometry toward the light (tile tops, table felt) must
            // rasterize into the projected depth maps — culling front faces left
            // maps cleared to 1.0 (always lit on sample).
            cull_mode: Some(wgpu::Face::Back),
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
    let probe_sh_bytes = (crate::room_glb::ROOM_EMISSIVE_PROBE_MAX as usize * probe_sh_stride)
        as wgpu::BufferAddress;
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
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
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
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
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
                resource: wgpu::BindingResource::TextureView(&depth_r32_snapshot_view),
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
                resource: wgpu::BindingResource::TextureView(&depth_r32_snapshot_view),
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
    #[cfg(feature = "windowed")]
    super::super::boot_splash::boot_present(
        &mut boot_splash,
        &target,
        &config,
        0.48,
        &mut boot_poll_slot,
    );

    let t_fonts = Instant::now();
    let ui_font = load_ui_font().cloned();
    if ui_font.is_some() {
        log::debug!("UI font loaded.");
    } else {
        log::warn!("No UI font found; panel text will be blank.");
    }
    let ui_font_italic = crate::decal::load_ui_font_italic().cloned();
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
    #[cfg(feature = "windowed")]
    super::super::boot_splash::boot_present(
        &mut boot_splash,
        &target,
        &config,
        0.55,
        &mut boot_poll_slot,
    );

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
    let tile_env_normal_view = tile_default_normal_view.clone();
    let tile_env_mr_view = tile_glb_default_mr_view.clone();
    let tile_env_emissive_view = tile_glb_default_emissive_view.clone();

    let tile_meshes = {
        let _tile = crate::startup_profile::scope("wgpu.tile_mesh");
        use crate::tile_glb::{
            load_glb_tile_from_bytes, normalize_mesh, tile_glb_asset_path, tile_material_index,
        };
        use mahjuro_gfx_types::TileMaterial;
        let tile_glb_defaults = crate::gltf_prop::GltfTileGpuDefaults {
            device: &device,
            queue: &queue,
            default_normal_view: &tile_default_normal_view,
            default_mr_view: &tile_glb_default_mr_view,
            default_emissive_view: &tile_glb_default_emissive_view,
        };
        let mut sets: [Option<TileMeshGpuSet>; crate::tile_glb::TILE_MATERIAL_MESH_COUNT] =
            std::array::from_fn(|_| None);
        for material in [
            TileMaterial::Bamboo,
            TileMaterial::Plastic,
            TileMaterial::TortoiseShell,
        ] {
            let path = tile_glb_asset_path(material);
            let label = format!("tile-{material:?}");
            let empty = crate::tile_glb::LoadedTile {
                primitives: Vec::new(),
            };
            let mesh_set = match mahjuro_assets::asset_path::get(path) {
                Some(file) => match load_glb_tile_from_bytes(&file.data) {
                    Ok(mut mesh) => {
                        normalize_mesh(&mut mesh);
                        log::info!(
                            "Loaded 3D tile {:?}: {} primitive(s) from {path}",
                            material,
                            mesh.primitives.len()
                        );
                        for (i, prim) in mesh.primitives.iter().enumerate() {
                            log::info!(
                                "  {:?} prim {i}: {} verts, {} idx, face={}",
                                material,
                                prim.vertices.len(),
                                prim.indices.len(),
                                prim.vertices.first().is_some_and(|v| v.color[3] > 0.5),
                            );
                        }
                        crate::gltf_prop::upload_tile_mesh_gpu_set(
                            &tile_glb_defaults,
                            &label,
                            &mesh,
                        )
                    }
                    Err(e) => {
                        log::warn!("Could not load tile mesh GLB {path}: {e:#}");
                        crate::gltf_prop::upload_tile_mesh_gpu_set(
                            &tile_glb_defaults,
                            &label,
                            &empty,
                        )
                    }
                },
                None => {
                    log::warn!("Tile mesh GLB missing at {path} (packs or assets/)");
                    crate::gltf_prop::upload_tile_mesh_gpu_set(&tile_glb_defaults, &label, &empty)
                }
            };
            sets[tile_material_index(material)] = Some(mesh_set);
        }
        sets.map(|slot| slot.expect("tile mesh slot filled above"))
    };
    #[cfg(feature = "windowed")]
    super::super::boot_splash::boot_present(
        &mut boot_splash,
        &target,
        &config,
        0.66,
        &mut boot_poll_slot,
    );

    // Deferred room GPU uploads — see `room_gpu_load.rs` (`ensure_*_room_gpu`).
    let shop_gltf_anim = crate::room_gltf_anim::RoomGltfAnimGpu::default();
    let (shop_env_primitives, shop_environment, shop_eyeball_prim_indices) =
        (Vec::new(), None, Vec::new());
    let (hallway_env_primitives, hallway_environment) = (Vec::new(), None);
    let (staircase_env_primitives, staircase_environment) = (Vec::new(), None);
    let (
        archive_env_primitives,
        archive_environment,
        archive_sign_left_prim_idx,
        archive_sign_right_prim_idx,
        archive_inspect_plaque_prim_idx,
        archive_plaque_backing_prim_idx,
        archive_page_left_prim_indices,
        archive_page_right_prim_indices,
    ) = (
        Vec::new(),
        None,
        None,
        None,
        None,
        None,
        Vec::new(),
        Vec::new(),
    );
    let (main_menu_env_primitives, main_menu_environment) = (Vec::new(), None);
    let main_menu_env_collision_meshes = Vec::new();
    let (
        gameplay_env_primitives,
        gameplay_environment,
        gameplay_cash_in_prim_indices,
        gameplay_score_roller_prim_groups,
        gameplay_score_roller_pivots_doc,
        gameplay_score_roller_axes_doc,
    ) = (
        Vec::new(),
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    );
    let gameplay_env_collision_meshes = Vec::new();
    let shop_env_collision_meshes = Vec::new();

    // Relic decode starts on first frame (`ensure_relic_loader_started`) so sync boot
    // is not competing with 100+ PNG decodes on a background thread.
    let relic_load_start = None;
    let relic_rx = None;
    let (_lit_mesh_relief_default_tex, lit_mesh_relief_default_view) =
        flat_relief_height(&device, &queue);
    let pack_textures_map = {
        let _pack = crate::startup_profile::scope("wgpu.pack_textures");
        load_pack_textures(&device, &queue, &lit_mesh_relief_default_view)
    };
    #[cfg(feature = "windowed")]
    super::super::boot_splash::boot_present(
        &mut boot_splash,
        &target,
        &config,
        0.72,
        &mut boot_poll_slot,
    );
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
    let coin_glb_file = mahjuro_assets::asset_path::get("3d/coin.glb")
        .expect("3d/coin.glb not embedded (packs or assets/)");
    let mut coin_tile = crate::tile_glb::load_glb_tile_from_node_name(
        &coin_glb_file.data,
        Some(crate::coin_glb::COIN_GLB_NODE),
    )
    .expect("coin.glb node decode");
    crate::tile_glb::normalize_mesh(&mut coin_tile);
    log::info!(
        "Loaded coin.glb: {} material slot(s)",
        coin_tile.primitives.len()
    );
    // Phase-1 primitive registry: parallel GPU copies of meshes
    // the generic `Object3dKind::Primitive` dispatch can reach by
    // `MeshId`. Legacy named fields above still own their own
    // allocations during the migration window.
    let mut primitive_meshes: rustc_hash::FxHashMap<MeshId, std::sync::Arc<LitMeshGpu>> =
        rustc_hash::FxHashMap::default();
    {
        let unit_cube_cpu = {
            let mut verts: Vec<crate::tile_glb::Vertex3dTex> = Vec::new();
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
        // Cylinder — generic Y-up unit disc (legacy); yen coins use [`MeshId::Coin`].
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
    let primitive_textures: rustc_hash::FxHashMap<
        crate::primitive::MeshId,
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
    let coin_glb_primitives = crate::gltf_prop::upload_gltf_tile_primitives(
        &crate::gltf_prop::GltfTileGpuDefaults {
            device: &device,
            queue: &queue,
            default_normal_view: &tile_default_normal_view,
            default_mr_view: &tile_glb_default_mr_view,
            default_emissive_view: &tile_glb_default_emissive_view,
        },
        "coin-glb",
        &coin_tile.primitives,
    );
    let (main_menu_rain_hit_debug_mesh, main_menu_rain_hit_debug_instance) =
        super::super::resources::init_main_menu_rain_hit_debug(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        );

    let relic_instances: Vec<LitMeshInstance> = Vec::new();
    let ordeal_icon_instances: Vec<LitMeshInstance> = Vec::new();
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
    let orb_instances: Vec<LitMeshInstance> = Vec::new();
    let talisman_height_views: Vec<wgpu::TextureView> = Vec::new();
    let talisman_mask_views: Vec<wgpu::TextureView> = Vec::new();
    let memorial_talisman_height_views: Vec<wgpu::TextureView> = Vec::new();
    let memorial_talisman_mask_views: Vec<wgpu::TextureView> = Vec::new();
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
    // Shop journal books are cheap; gameplay HUD instance pools are deferred
    // until first gameplay draw (`ensure_gameplay_hud_pools`).
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
    let book_instances = make_pool(MAX_BOOK_SLOTS);
    let book_cover_instances = make_pool(MAX_BOOK_SLOTS);
    let yaku_tablet_instances: Vec<LitMeshInstance> = Vec::new();
    let wood_tablet_instances: Vec<LitMeshInstance> = Vec::new();
    let bowl_instances: Vec<LitMeshInstance> = Vec::new();
    let mirror_instances: Vec<LitMeshInstance> = Vec::new();
    let tally_stick_instances: Vec<LitMeshInstance> = Vec::new();
    let wall_tile_instances: Vec<LitMeshInstance> = Vec::new();
    let extruded_glyph_instances: Vec<LitMeshInstance> = Vec::new();
    let debug_axes_instances = make_pool(3);
    crate::startup_profile::record("wgpu.lit_meshes_and_pools", t_lit_meshes.elapsed());
    #[cfg(feature = "windowed")]
    super::super::boot_splash::boot_present(
        &mut boot_splash,
        &target,
        &config,
        0.92,
        &mut boot_poll_slot,
    );

    // Build the GPU profiler up-front while we still have a borrow of
    // device/queue (the struct literal below moves them).
    let gpu_profiler = crate::gpu_profiler::GpuProfiler::new(
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

    #[cfg(feature = "windowed")]
    super::super::boot_splash::boot_present(
        &mut boot_splash,
        &target,
        &config,
        1.0,
        &mut boot_poll_slot,
    );
    #[cfg(feature = "windowed")]
    drop(boot_splash);

    Ok(WgpuRenderer {
        target,
        device,
        queue,
        config,
        depth_texture,
        depth_view,
        ssr_prev_depth_texture,
        ssr_prev_depth_view,
        depth_r32_snapshot_texture,
        depth_r32_snapshot_view,
        depth_copy_staging_buffer,
        quad_pipeline,
        quad_pipeline_display,
        depth_quad_pipeline,
        depth_quad_pipeline_display,
        depth_quad_debug_pipeline,
        depth_quad_debug_pipeline_display,
        gradient_quad_pipeline,
        arc_ring_quad_pipeline,
        arc_ring_quad_pipeline_display,
        squircle_quad_pipeline,
        squircle_quad_pipeline_display,
        flame_pipeline,
        flame_volume_mesh,
        flame_view_buffer,
        flame_view_bind_group,
        flame_instance_staging: Vec::with_capacity(32),
        starfield_pipeline,
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
        tile_pipeline_opaque_cull,
        tile_pipeline_blend_double,
        tile_pipeline_blend_cull,
        shop_pipeline_opaque_cull,
        shop_pipeline_blend_double,
        shop_pipeline_blend_cull,
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
        tile_meshes,
        active_tile_material: mahjuro_gfx_types::TileMaterial::Bamboo,
        shop_env_primitives,
        shop_environment,
        shop_gltf_anim,
        shop_gltf_anim_missing_clip_warned: std::cell::Cell::new(false),
        shop_eyeball_prim_indices,
        rooms_gpu_loaded: 0,
        room_profile_frame_dt_ms: 1000.0 / 60.0,
        shadow_warp_layout,
        tile_env_normal_view,
        tile_env_mr_view,
        tile_env_emissive_view,
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
        gameplay_score_roller_prim_groups,
        gameplay_score_roller_pivots_doc,
        gameplay_score_roller_axes_doc,
        gameplay_score_roller_drive_values: std::cell::RefCell::new([0.0; 2]),
        gameplay_score_roller_drive_initialized: std::cell::RefCell::new([false; 2]),
        gameplay_score_roller_roll_elapsed: std::cell::RefCell::new(0.0),
        gameplay_environment,
        archive_sign_left_prim_idx,
        archive_sign_right_prim_idx,
        archive_inspect_plaque_prim_idx,
        archive_plaque_backing_prim_idx,
        archive_page_left_prim_indices,
        archive_page_right_prim_indices,
        archive_sign_decal_upload_key: 0,
        archive_inspect_plaque_decal_upload_key: 0,
        frame_env_tunes: rustc_hash::FxHashMap::default(),
        active_frame_env: crate::room_glb::RoomEnvFrameTune::default(),
        shop_env_collision_meshes,
        main_menu_env_collision_meshes,
        gameplay_env_collision_meshes,
        tile_base_color_factor,
        // Populated on first render() from RenderSettings.tileset_name.
        tile_set: None,
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
        glyph_cpu_cache: crate::glyph_mesh::GlyphMeshCache::new(),
        extruded_glyph_meshes: rustc_hash::FxHashMap::default(),
        primitive_meshes,
        primitive_instances: rustc_hash::FxHashMap::default(),
        primitive_textures,
        coin_glb_primitives,
        coin_glb_instances: Vec::new(),
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
        gameplay_hud_pools_ready: false,
        talisman_textures_ready: false,
        relic_rx,
        relic_load_finished: false,
        relic_load_start,
        relic_profile_upload_cpu: std::time::Duration::ZERO,
        pack_textures: pack_textures_map,
        background_textures: rustc_hash::FxHashMap::default(),
        background_rx,
        background_load_start,
        prev_tile_world: rustc_hash::FxHashMap::default(),
        tile_uid_scratch: rustc_hash::FxHashSet::default(),
        prev_shadow_quality: ShadowQuality::Off,
        cached_shadow_light_view_proj: glam::Mat4::IDENTITY.to_cols_array(),
        projected_shadow_lights: Vec::new(),
        cached_projected_shadow_hash: 0,
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
        lab_baked_shadow: None,
        active_lab_baked_shadow: false,
        room_shadow_capture_pending: None,
        room_shadow_captured: None,
        shadow_probe_last_log: Instant::now(),
        shadow_probe_last_caster_count: usize::MAX,
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
        main_menu_effects: crate::main_menu_effects_tuning::MainMenuEffectsTuning::load(),
        lit_mesh_pipeline,
        lit_mesh_blended_pipeline,
        lit_mesh_white_view,
        lit_mesh_relief_default_view,
        talisman_height_views,
        talisman_mask_views,
        memorial_talisman_height_views,
        memorial_talisman_mask_views,
        talisman_meshes: rustc_hash::FxHashMap::default(),
        memorial_talisman_meshes: rustc_hash::FxHashMap::default(),
        talisman_meshes_ready: false,
        talisman_slot_kind,
        relic_box_mesh,
        relic_box_tris,
        relic_tri_lists: rustc_hash::FxHashMap::default(),
        pack_mesh,
        relic_meshes: rustc_hash::FxHashMap::default(),
        relic_instances,
        point_shadow_array,
        spot_shadow_array,
        shadow_sample_layout,
        shadow_compare_sampler,
        shadow_ao_sampler,
        _shadow_ao_white_texture: shadow_ao_white_texture,
        shadow_ao_white_view,
        _shadow_baked_depth_dummy_texture: shadow_baked_depth_dummy_texture,
        shadow_baked_depth_dummy_view,
        shadow_caster_layout,
        shadow_warp_disabled_bind_group,
        shadow_globals_buffer,
        shadow_sample_bind_group,
        shadow_pipeline,
        shadow_pipeline_room_env,
        gpu_profiler,
        pending_screenshot: std::cell::Cell::new(None),
        acquire_telemetry: super::super::runtime::AcquireTelemetry::new(),
        shadow_quality: default_shadow_quality,
        flame_tuning: crate::flame_tuning::FlameTuning::load(),
        main_menu_pride_rainbow_debug:
            crate::main_menu_glb::main_menu_pride_rainbow_default_enabled(),
        main_menu_moon_phase_debug: crate::main_menu_moon_tuning::MainMenuMoonPhaseDebug::default(),
        main_menu_rain_hit_debug_mesh,
        main_menu_rain_hit_debug_instance,
        probe_gi_stale_aabb_warned_room: None,
    })
}
