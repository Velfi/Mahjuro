//! Shader module compilation + render pipeline creation during renderer init.

use std::time::Instant;

use super::super::*;

pub(super) struct ShadersAndPipelinesParams<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub size: crate::physical_size::PhysicalSize,
    pub render_size: crate::physical_size::PhysicalSize,
    pub format: wgpu::TextureFormat,
    pub ssr_prev_depth_view: &'a wgpu::TextureView,
    pub depth_r32_snapshot_view: &'a wgpu::TextureView,
}

pub(super) struct ShadersAndPipelinesInit {
    pub arc_ring_quad_pipeline: wgpu::RenderPipeline,
    pub arc_ring_quad_pipeline_display: wgpu::RenderPipeline,
    pub bloom_bind_group_layout: wgpu::BindGroupLayout,
    pub bloom_blur_h_params_buffer: wgpu::Buffer,
    pub bloom_blur_pipeline: wgpu::RenderPipeline,
    pub bloom_blur_v_params_buffer: wgpu::Buffer,
    pub bloom_composite_bind_group: wgpu::BindGroup,
    pub bloom_composite_bind_group_layout: wgpu::BindGroupLayout,
    pub bloom_composite_params_buffer: wgpu::Buffer,
    pub bloom_composite_pipeline: wgpu::RenderPipeline,
    pub bloom_extract_bind_group_layout: wgpu::BindGroupLayout,
    pub bloom_extract_params_buffer: wgpu::Buffer,
    pub bloom_extract_pipeline: wgpu::RenderPipeline,
    pub bloom_ping_bind_group: wgpu::BindGroup,
    pub bloom_ping_texture: wgpu::Texture,
    pub bloom_ping_view: wgpu::TextureView,
    pub bloom_pong_bind_group: wgpu::BindGroup,
    pub bloom_pong_texture: wgpu::Texture,
    pub bloom_pong_view: wgpu::TextureView,
    pub bloom_sampler: wgpu::Sampler,
    pub bloom_scene_bind_group: wgpu::BindGroup,
    pub cascade_composite_bind_group: wgpu::BindGroup,
    pub cascade_composite_layout: wgpu::BindGroupLayout,
    pub cascade_composite_pipeline: wgpu::RenderPipeline,
    pub cascade_composite_sampler: wgpu::Sampler,
    pub cascade_offscreen_texture: wgpu::Texture,
    pub cascade_offscreen_view: wgpu::TextureView,
    pub default_shadow_quality: mahjuro_gfx_types::ShadowQuality,
    pub depth_quad_debug_pipeline: wgpu::RenderPipeline,
    pub depth_quad_debug_pipeline_display: wgpu::RenderPipeline,
    pub depth_quad_pipeline: wgpu::RenderPipeline,
    pub depth_quad_pipeline_display: wgpu::RenderPipeline,
    pub emissive_gi_composite_bind_group: wgpu::BindGroup,
    pub emissive_gi_composite_bind_group_layout: wgpu::BindGroupLayout,
    pub emissive_gi_composite_pipeline: wgpu::RenderPipeline,
    pub emissive_gi_texture: wgpu::Texture,
    pub emissive_gi_view: wgpu::TextureView,
    pub emissive_probe_apply_bind_group: wgpu::BindGroup,
    pub emissive_probe_apply_bind_group_layout: wgpu::BindGroupLayout,
    pub emissive_probe_apply_pipeline: wgpu::RenderPipeline,
    pub emissive_probe_update_bind_group: wgpu::BindGroup,
    pub emissive_probe_update_bind_group_layout: wgpu::BindGroupLayout,
    pub emissive_probe_update_pipeline: wgpu::ComputePipeline,
    pub flame_pipeline: wgpu::RenderPipeline,
    pub flame_view_bind_group: wgpu::BindGroup,
    pub flame_view_buffer: wgpu::Buffer,
    pub flame_volume_mesh: crate::lit_mesh::LitMeshGpu,
    pub globals_bind_group: wgpu::BindGroup,
    pub globals_buffer: wgpu::Buffer,
    pub golden_dust_pipeline: wgpu::RenderPipeline,
    pub gradient_quad_pipeline: wgpu::RenderPipeline,
    pub image_pipeline: wgpu::RenderPipeline,
    pub image_pipeline_scene_hdr: wgpu::RenderPipeline,
    pub image_shader: wgpu::ShaderModule,
    pub journal_scene_texture: wgpu::Texture,
    pub journal_scene_view: wgpu::TextureView,
    pub lit_mesh_blended_pipeline: wgpu::RenderPipeline,
    pub lit_mesh_material_layout: wgpu::BindGroupLayout,
    pub lit_mesh_pipeline: wgpu::RenderPipeline,
    pub lit_mesh_spot_ssr_bind_group: wgpu::BindGroup,
    pub lit_mesh_spot_ssr_layout: wgpu::BindGroupLayout,
    pub lit_mesh_ssr_buffer: wgpu::Buffer,
    pub lit_mesh_ssr_sampler: wgpu::Sampler,
    pub moonlit_water_bind_group: wgpu::BindGroup,
    pub moonlit_water_pipeline: wgpu::RenderPipeline,
    pub point_lights_bind_group: wgpu::BindGroup,
    pub point_lights_buffer: wgpu::Buffer,
    pub point_shadow_array: ShadowDepthArrayGpu,
    pub post_bloom_texture: wgpu::Texture,
    pub post_bloom_view: wgpu::TextureView,
    pub probe_gi_frame_uniform_buffer: wgpu::Buffer,
    pub probe_sh_buffer: wgpu::Buffer,
    pub quad_pipeline: wgpu::RenderPipeline,
    pub quad_pipeline_display: wgpu::RenderPipeline,
    pub room_baked_shadow_gpu:
        [Option<impl_room_shadow::RoomBakedShadowGpu>; crate::room_gi_bake::ROOM_GI_ROOM_COUNT],
    pub room_emissive_texture: wgpu::Texture,
    pub room_emissive_view: wgpu::TextureView,
    pub scene_color_downsample_bind_group: wgpu::BindGroup,
    pub scene_color_downsample_pipeline: wgpu::RenderPipeline,
    pub scene_color_texture: wgpu::Texture,
    pub scene_color_view: wgpu::TextureView,
    pub scene_prev_texture: wgpu::Texture,
    pub scene_prev_view: wgpu::TextureView,
    pub shadow_ao_sampler: wgpu::Sampler,
    pub shadow_ao_white_texture: wgpu::Texture,
    pub shadow_ao_white_view: wgpu::TextureView,
    pub shadow_baked_depth_dummy_texture: wgpu::Texture,
    pub shadow_baked_depth_dummy_view: wgpu::TextureView,
    pub shadow_caster_layout: wgpu::BindGroupLayout,
    pub shadow_compare_sampler: wgpu::Sampler,
    pub shadow_globals_buffer: wgpu::Buffer,
    pub shadow_pipeline: wgpu::RenderPipeline,
    pub shadow_pipeline_room_env: wgpu::RenderPipeline,
    pub shadow_sample_bind_group: wgpu::BindGroup,
    pub shadow_sample_layout: wgpu::BindGroupLayout,
    pub shadow_warp_disabled_bind_group: wgpu::BindGroup,
    pub shadow_warp_layout: wgpu::BindGroupLayout,
    pub shooting_star_cascade_pipeline: wgpu::RenderPipeline,
    pub shop_pipeline_blend_cull: wgpu::RenderPipeline,
    pub shop_pipeline_blend_double: wgpu::RenderPipeline,
    pub shop_pipeline_mrt_blend_cull: wgpu::RenderPipeline,
    pub shop_pipeline_mrt_blend_double: wgpu::RenderPipeline,
    pub shop_pipeline_mrt_opaque_cull: wgpu::RenderPipeline,
    pub shop_pipeline_opaque_cull: wgpu::RenderPipeline,
    pub spot_lights_bind_group: wgpu::BindGroup,
    pub spot_lights_buffer: wgpu::Buffer,
    pub spot_shadow_array: ShadowDepthArrayGpu,
    pub squircle_quad_pipeline: wgpu::RenderPipeline,
    pub squircle_quad_pipeline_display: wgpu::RenderPipeline,
    pub starfield_pipeline: wgpu::RenderPipeline,
    pub sunlit_water_pipeline: wgpu::RenderPipeline,
    pub text_bind_group_layout: wgpu::BindGroupLayout,
    pub text_overlay_pipeline_layout: wgpu::PipelineLayout,
    pub text_pipeline: wgpu::RenderPipeline,
    pub text_pipeline_scene_hdr: wgpu::RenderPipeline,
    pub text_shader: wgpu::ShaderModule,
    pub tile_base_color_factor: [f32; 4],
    pub tile_default_normal_texture: wgpu::Texture,
    pub tile_default_normal_view: wgpu::TextureView,
    pub tile_env_distortion_placeholder: wgpu::Buffer,
    pub tile_glow_pipeline: wgpu::RenderPipeline,
    pub tile_material_layout: wgpu::BindGroupLayout,
    pub tile_occluders_buffer: wgpu::Buffer,
    pub tile_outline_frame_bind_group: wgpu::BindGroup,
    pub tile_outline_frame_uniform_buffer: wgpu::Buffer,
    pub tile_outline_instance_buffer: wgpu::Buffer,
    pub tile_outline_pipeline: wgpu::RenderPipeline,
    pub tile_pipeline_blend_cull: wgpu::RenderPipeline,
    pub tile_pipeline_blend_double: wgpu::RenderPipeline,
    pub tile_pipeline_opaque_cull: wgpu::RenderPipeline,
    pub tile_sampler: wgpu::Sampler,
    pub tonemap_bind_group: wgpu::BindGroup,
    pub tonemap_bind_group_layout: wgpu::BindGroupLayout,
    pub tonemap_bind_group_scene: wgpu::BindGroup,
    pub tonemap_params_buffer: wgpu::Buffer,
    pub tonemap_pipeline: wgpu::RenderPipeline,
    pub tonemap_pipeline_layout: wgpu::PipelineLayout,
    pub tonemap_rgba16f_pipeline: wgpu::RenderPipeline,
    pub tonemap_shader_module: wgpu::ShaderModule,
}

pub(super) fn init_shaders_and_pipelines(
    params: ShadersAndPipelinesParams<'_>,
) -> ShadersAndPipelinesInit {
    let ShadersAndPipelinesParams {
        device,
        queue,
        size,
        render_size,
        format,
        ssr_prev_depth_view,
        depth_r32_snapshot_view,
    } = params;

    let scene_hdr_format = SCENE_HDR_FORMAT;

    let _phase = crate::startup_profile::scope("wgpu.phase.shaders_and_pipelines");

    let t_shaders = Instant::now();
    let t_shader_modules = Instant::now();
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
    crate::startup_profile::record("wgpu.shader_modules", t_shader_modules.elapsed());

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

    let default_shadow_quality = ShadowQuality::Off;
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
    let (scene_prev_w, scene_prev_h) =
        scene_prev_size(render_size.width.max(1), render_size.height.max(1));
    let (scene_prev_texture, scene_prev_view) =
        create_scene_prev(&device, scene_hdr_format, scene_prev_w, scene_prev_h);
    let (scene_color_texture, scene_color_view) = create_scene_color(
        &device,
        scene_hdr_format,
        render_size.width.max(1),
        render_size.height.max(1),
    );
    let (post_bloom_texture, post_bloom_view) = create_scene_color(
        &device,
        scene_hdr_format,
        render_size.width.max(1),
        render_size.height.max(1),
    );
    let (room_emissive_texture, room_emissive_view) = create_scene_color(
        &device,
        scene_hdr_format,
        render_size.width.max(1),
        render_size.height.max(1),
    );
    // Fullscreen offscreen for the live yaku-journal GPU render (book
    // page surface samples this in screen space; see `lit_mesh.wgsl`).
    let (journal_scene_texture, journal_scene_view) = create_journal_scene(
        &device,
        scene_hdr_format,
        render_size.width.max(1),
        render_size.height.max(1),
    );
    let (cascade_offscreen_texture, cascade_offscreen_view) = create_cascade_offscreen(
        &device,
        scene_hdr_format,
        render_size.width.max(1),
        render_size.height.max(1),
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
    let bloom_w = (render_size.width.max(1) / 2).max(1);
    let bloom_h = (render_size.height.max(1) / 2).max(1);

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
    let tonemap_pipeline = make_tonemap_pipe("tonemap-pipeline", format);
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
    ShadersAndPipelinesInit {
        arc_ring_quad_pipeline,
        arc_ring_quad_pipeline_display,
        bloom_bind_group_layout,
        bloom_blur_h_params_buffer,
        bloom_blur_pipeline,
        bloom_blur_v_params_buffer,
        bloom_composite_bind_group,
        bloom_composite_bind_group_layout,
        bloom_composite_params_buffer,
        bloom_composite_pipeline,
        bloom_extract_bind_group_layout,
        bloom_extract_params_buffer,
        bloom_extract_pipeline,
        bloom_ping_bind_group,
        bloom_ping_texture,
        bloom_ping_view,
        bloom_pong_bind_group,
        bloom_pong_texture,
        bloom_pong_view,
        bloom_sampler,
        bloom_scene_bind_group,
        cascade_composite_bind_group,
        cascade_composite_layout,
        cascade_composite_pipeline,
        cascade_composite_sampler,
        cascade_offscreen_texture,
        cascade_offscreen_view,
        default_shadow_quality,
        depth_quad_debug_pipeline,
        depth_quad_debug_pipeline_display,
        depth_quad_pipeline,
        depth_quad_pipeline_display,
        emissive_gi_composite_bind_group,
        emissive_gi_composite_bind_group_layout,
        emissive_gi_composite_pipeline,
        emissive_gi_texture,
        emissive_gi_view,
        emissive_probe_apply_bind_group,
        emissive_probe_apply_bind_group_layout,
        emissive_probe_apply_pipeline,
        emissive_probe_update_bind_group,
        emissive_probe_update_bind_group_layout,
        emissive_probe_update_pipeline,
        flame_pipeline,
        flame_view_bind_group,
        flame_view_buffer,
        flame_volume_mesh,
        globals_bind_group,
        globals_buffer,
        golden_dust_pipeline,
        gradient_quad_pipeline,
        image_pipeline,
        image_pipeline_scene_hdr,
        image_shader,
        journal_scene_texture,
        journal_scene_view,
        lit_mesh_blended_pipeline,
        lit_mesh_material_layout,
        lit_mesh_pipeline,
        lit_mesh_spot_ssr_bind_group,
        lit_mesh_spot_ssr_layout,
        lit_mesh_ssr_buffer,
        lit_mesh_ssr_sampler,
        moonlit_water_bind_group,
        moonlit_water_pipeline,
        point_lights_bind_group,
        point_lights_buffer,
        point_shadow_array,
        post_bloom_texture,
        post_bloom_view,
        probe_gi_frame_uniform_buffer,
        probe_sh_buffer,
        quad_pipeline,
        quad_pipeline_display,
        room_baked_shadow_gpu,
        room_emissive_texture,
        room_emissive_view,
        scene_color_downsample_bind_group,
        scene_color_downsample_pipeline,
        scene_color_texture,
        scene_color_view,
        scene_prev_texture,
        scene_prev_view,
        shadow_ao_sampler,
        shadow_ao_white_texture,
        shadow_ao_white_view,
        shadow_baked_depth_dummy_texture,
        shadow_baked_depth_dummy_view,
        shadow_caster_layout,
        shadow_compare_sampler,
        shadow_globals_buffer,
        shadow_pipeline,
        shadow_pipeline_room_env,
        shadow_sample_bind_group,
        shadow_sample_layout,
        shadow_warp_disabled_bind_group,
        shadow_warp_layout,
        shooting_star_cascade_pipeline,
        shop_pipeline_blend_cull,
        shop_pipeline_blend_double,
        shop_pipeline_mrt_blend_cull,
        shop_pipeline_mrt_blend_double,
        shop_pipeline_mrt_opaque_cull,
        shop_pipeline_opaque_cull,
        spot_lights_bind_group,
        spot_lights_buffer,
        spot_shadow_array,
        squircle_quad_pipeline,
        squircle_quad_pipeline_display,
        starfield_pipeline,
        sunlit_water_pipeline,
        text_bind_group_layout,
        text_overlay_pipeline_layout,
        text_pipeline,
        text_pipeline_scene_hdr,
        text_shader,
        tile_base_color_factor,
        tile_default_normal_texture,
        tile_default_normal_view,
        tile_env_distortion_placeholder,
        tile_glow_pipeline,
        tile_material_layout,
        tile_occluders_buffer,
        tile_outline_frame_bind_group,
        tile_outline_frame_uniform_buffer,
        tile_outline_instance_buffer,
        tile_outline_pipeline,
        tile_pipeline_blend_cull,
        tile_pipeline_blend_double,
        tile_pipeline_opaque_cull,
        tile_sampler,
        tonemap_bind_group,
        tonemap_bind_group_layout,
        tonemap_bind_group_scene,
        tonemap_params_buffer,
        tonemap_pipeline,
        tonemap_pipeline_layout,
        tonemap_rgba16f_pipeline,
        tonemap_shader_module,
    }
}
