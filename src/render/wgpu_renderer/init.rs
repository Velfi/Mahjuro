use super::*;

use crate::render::gltf_helpers::{GltfPbrUniform, build_sampler_descriptor};
use crate::render::lamp_mesh::{build_bug_body_mesh, build_bug_wing_blur_mesh, build_bug_wing_mesh};

impl WgpuRenderer {
    pub fn new(target_init: TargetInit) -> anyhow::Result<Self> {
        let t_total = Instant::now();
        let instance = wgpu::Instance::default();

        // Branch on target: the windowed path creates a Surface *before*
        // adapter selection (compatible_surface), then picks a format from
        // the surface caps. The headless path requests an adapter without
        // any surface and picks the format itself.
        let (surface_opt, size, hdr_enabled): (
            Option<wgpu::Surface<'static>>,
            winit::dpi::PhysicalSize<u32>,
            bool,
        ) = match &target_init {
            TargetInit::Windowed {
                window,
                hdr_enabled,
            } => {
                let size = window.inner_size();
                let surface = instance.create_surface(window.clone())?;
                (Some(surface), size, *hdr_enabled)
            }
            TargetInit::Headless {
                width,
                height,
                hdr_enabled,
            } => {
                let size = winit::dpi::PhysicalSize::new((*width).max(1), (*height).max(1));
                (None, size, *hdr_enabled)
            }
        };

        let size = super::clamp_render_physical_size(size);

        let t0 = Instant::now();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: surface_opt.as_ref(),
            force_fallback_adapter: false,
        }))
        .map_err(|e| anyhow::anyhow!("adapter: {e:?}"))?;
        log::info!("wgpu adapter acquired in {:?}", t0.elapsed());

        // Flag CPU-fallback adapters in headless mode so local runs don't
        // silently get wrong anti-aliasing. Still valid on CI without a GPU.
        if surface_opt.is_none() {
            let info = adapter.get_info();
            if info.device_type == wgpu::DeviceType::Cpu {
                log::warn!(
                    "headless renderer using CPU fallback adapter '{}' ({:?}); anti-aliasing may differ from GPU runs",
                    info.name,
                    info.backend
                );
            }
        }

        // Pick the output format. Windowed mode queries the surface caps;
        // headless mode hard-picks Rgba8UnormSrgb — every backend supports
        // it as RENDER_ATTACHMENT | COPY_SRC and the existing PNG readback
        // already handles sRGB8 correctly (no BGRA swap needed).
        let (format, swapchain_sdr_format, swapchain_hdr_available) = match surface_opt.as_ref() {
            Some(surface) => {
                let caps = surface.get_capabilities(&adapter);
                let swapchain_sdr_format = caps
                    .formats
                    .iter()
                    .find(|f| f.is_srgb())
                    .copied()
                    .unwrap_or(caps.formats[0]);
                let swapchain_hdr_available =
                    caps.formats.contains(&wgpu::TextureFormat::Rgba16Float);
                let format = if hdr_enabled {
                    if swapchain_hdr_available {
                        log::info!("HDR enabled — using Rgba16Float surface format");
                        wgpu::TextureFormat::Rgba16Float
                    } else {
                        log::warn!(
                            "HDR requested but Rgba16Float not supported; falling back to sRGB"
                        );
                        swapchain_sdr_format
                    }
                } else {
                    swapchain_sdr_format
                };
                (format, swapchain_sdr_format, swapchain_hdr_available)
            }
            None => {
                if hdr_enabled {
                    log::info!("headless renderer ignoring hdr_enabled; screenshots are sRGB8");
                }
                (
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    false,
                )
            }
        };

        // Linear HDR intermediate — main scene + bloom; tonemap maps to the swapchain format.
        let scene_hdr_format = SCENE_HDR_FORMAT;

        let limits =
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());

        // Opt into TIMESTAMP_QUERY when the adapter supports it so the GPU
        // pass profiler (Debug menu → "Profile GPU…") can record start/end
        // ticks per render pass. The feature is optional — on backends that
        // lack it the profiler stays a no-op.
        let mut required_features = wgpu::Features::CLEAR_TEXTURE;
        let timestamp_supported = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        if timestamp_supported {
            required_features |= wgpu::Features::TIMESTAMP_QUERY;
            // INSIDE_ENCODERS allows `encoder.write_timestamp()` outside of
            // render passes — only needed for debug profiling tools.
            #[cfg(debug_assertions)]
            if adapter
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
            {
                required_features |= wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
            }
        }

        let t0 = Instant::now();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("mahjuro-device"),
            required_features,
            required_limits: limits,
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::default(),
        }))
        .map_err(|e| anyhow::anyhow!("device: {e:?}"))?;
        log::info!("wgpu device created in {:?}", t0.elapsed());

        // Build the shared `SurfaceConfiguration` that downstream textures
        // track against. Windowed mode seeds it from `get_default_config`
        // and calls `surface.configure`; headless mode fills in the same
        // fields by hand (alpha_mode / view_formats don't matter for the
        // texture path) and creates the offscreen render-attachment.
        let (target, config) = match surface_opt {
            Some(surface) => {
                let mut config = surface
                    .get_default_config(&adapter, size.width.max(1), size.height.max(1))
                    .ok_or_else(|| anyhow::anyhow!("no default surface config"))?;
                config.format = format;
                config.present_mode = wgpu::PresentMode::Fifo;
                config.desired_maximum_frame_latency = 2;
                // Need COPY_SRC so we can snapshot the swapchain into
                // `scene_prev_texture` at end-of-frame for the lacquer SSR pass.
                config.usage |= wgpu::TextureUsages::COPY_SRC;
                surface.configure(&device, &config);
                (RenderTarget::Surface(surface), config)
            }
            None => {
                let config = wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    format,
                    width: size.width.max(1),
                    height: size.height.max(1),
                    present_mode: wgpu::PresentMode::Fifo,
                    desired_maximum_frame_latency: 2,
                    alpha_mode: wgpu::CompositeAlphaMode::Auto,
                    view_formats: vec![],
                };
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("headless-frame-target"),
                    size: wgpu::Extent3d {
                        width: config.width,
                        height: config.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: config.usage,
                    view_formats: &[],
                });
                (RenderTarget::Offscreen { texture }, config)
            }
        };

        let (depth_texture, depth_view) =
            create_depth(&device, size.width.max(1), size.height.max(1));
        // Separate depth snapshot for the lacquered-table SSR sample —
        // copied after Pass A together with `scene_prev_texture`.
        let (ssr_prev_depth_texture, ssr_prev_depth_view) =
            create_depth_copy(&device, size.width.max(1), size.height.max(1));

        let t0 = Instant::now();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quad-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../../shaders/quad.wgsl").into()),
        });

        let tile_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tile-3d-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../../shaders/tile_3d.wgsl").into()),
        });

        let shop_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shop-glb-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../../shaders/shop_glb.wgsl").into()),
        });

        let text_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("text-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../../shaders/text_quad.wgsl").into(),
            ),
        });

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
        // Initialised empty; populated each frame from `frame.point_lights`.
        let point_lights_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("point-lights"),
            contents: bytemuck::bytes_of(&PointLightsBuf::from_lights(
                &[],
                0,
                0.0,
                1.0,
                1.0,
                1.0,
                1.0,
                0.0,
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
        // Shop `KHR_lights_punctual` — separate buffer; bound at group 1 binding 2 so `lit_mesh`
        // stays within WebGPU's max_bind_groups = 4 (Metal).
        let shop_gltf_point_lights_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shop-gltf-point-lights"),
            contents: bytemuck::bytes_of(&PointLightsBuf::from_lights(
                &[],
                0,
                0.0,
                1.0,
                1.0,
                1.0,
                1.0,
                0.0,
            )),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let point_lights_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: shop_gltf_point_lights_buffer.as_entire_binding(),
                },
            ],
        });

        // Spotlight buffer + bind group (group 3 of the tile pipeline).
        // Initialised empty; populated each frame from `frame.spot_lights`.
        let spot_lights_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("spot-lights"),
            contents: bytemuck::bytes_of(&SpotLightsBuf::empty()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let spot_lights_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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

        let shop_gltf_point_lights_scene_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shop-gltf-point-lights-scene-bg"),
            layout: &point_lights_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: shop_gltf_point_lights_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: tile_occluders_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: shop_gltf_point_lights_buffer.as_entire_binding(),
                },
            ],
        });

        let tile_material_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
                ],
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

        let loaded_glb = match crate::asset_path::get("Tile.glb") {
            Some(file) => load_glb_tile_from_bytes(&file.data),
            None => Err(anyhow::anyhow!("Tile.glb not found in embedded assets")),
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
        let moon_albedo_tex_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("moon-albedo-layout"),
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
        let moonlit_water_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("moonlit-water-pl"),
            bind_group_layouts: &[Some(&globals_layout), Some(&moon_albedo_tex_layout)],
            immediate_size: 0,
        });
        let (_moon_albedo_texture, moon_albedo_view) =
            crate::render::texture_upload::load_metal_heightmap(
                &device,
                &queue,
                "textures/moon_albedo.png",
                "moon-albedo",
            );
        let moon_albedo_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("moon-albedo-bg"),
            layout: &moon_albedo_tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&moon_albedo_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&tile_sampler),
                },
            ],
        });

        // ── Mountain-haze uniform (density/colour/horizon/drift) ───────────
        // Live-driven from the Volumetric debug overlay; see
        // `set_haze_tuning` below for the per-frame write path.
        let haze_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("haze-uniform"),
            contents: bytemuck::bytes_of(&HazeUniform {
                color_density: [0.080, 0.105, 0.145, 1.0],
                params: [0.55, 1.0, 0.0, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let haze_uniform_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("haze-uniform-layout"),
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
        let haze_uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("haze-uniform-bg"),
            layout: &haze_uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: haze_uniform_buffer.as_entire_binding(),
            }],
        });
        let mountain_haze_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mountain-haze-pl"),
            bind_group_layouts: &[Some(&globals_layout), Some(&haze_uniform_layout)],
            immediate_size: 0,
        });

        // ---- Shadow map resources (depth texture + sampler + layouts) ----
        // Built up here so the shared sampling layout can be plumbed into
        // both `tile_layout` and `lit_mesh_pl` below as group 2.
        const SHADOW_MAP_SIZE: u32 = 2048;
        let _shadow_map_texture = device.create_texture(&wgpu::TextureDescriptor {
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_map_view =
            _shadow_map_texture.create_view(&wgpu::TextureViewDescriptor::default());
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
        let shadow_sample_layout = create_shadow_sample_layout(&device);
        let shadow_globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shadow-globals"),
            contents: bytemuck::bytes_of(&ShadowGlobals {
                light_view_proj: glam::Mat4::IDENTITY.to_cols_array(),
                params: [0.0, 0.0015, 1.0 / SHADOW_MAP_SIZE as f32, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let shadow_sample_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow-sample-bg"),
            layout: &shadow_sample_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: shadow_globals_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&shadow_map_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
        });

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
                * (super::MAX_SHOWCASE_TILE_SLOTS as u64),
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

        let gradient_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gradient_quad.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/shaders/gradient_quad.wgsl"
                ))
                .into(),
            ),
        });

        let gradient_quad_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
        let flame_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flame.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/flame.wgsl")).into(),
            ),
        });
        let flame_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flame-pl"),
            bind_group_layouts: &[Some(&globals_layout), Some(&flame_view_layout)],
            immediate_size: 0,
        });
        let flame_particle_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::render::flame_particles::GpuFlameParticle>()
                as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // pos (xyz) + age (w) packed into a vec4 for tidy layout.
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
                // scale + phase + brightness + pad → vec4.
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        };
        let flame_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("flame-pipeline"),
            layout: Some(&flame_pl),
            vertex: wgpu::VertexState {
                module: &flame_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout.clone(), flame_particle_layout],
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
        let vignette_pipeline = |label: &str, wgsl: &str| -> wgpu::RenderPipeline {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(wgsl.into()),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&quad_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
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

        let starfield_pipeline = vignette_pipeline(
            "starfield-pipeline",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/starfield.wgsl"
            )),
        );
        let ember_drift_pipeline = vignette_pipeline(
            "ember-drift-pipeline",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/ember_drift.wgsl"
            )),
        );
        let golden_dust_pipeline = vignette_pipeline(
            "golden-dust-pipeline",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/golden_dust.wgsl"
            )),
        );
        // moonlit_water gets its own pipeline so it can bind the moon albedo
        // texture at group 1 in addition to the globals at group 0.
        let moonlit_water_pipeline = {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("moonlit-water-pipeline"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/shaders/moonlit_water.wgsl"
                    ))
                    .into(),
                ),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("moonlit-water-pipeline"),
                layout: Some(&moonlit_water_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
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
        let sunlit_water_pipeline = vignette_pipeline(
            "sunlit-water-pipeline",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/sunlit_water.wgsl"
            )),
        );
        // Mountain-haze uses a custom pipeline layout so the fragment shader
        // can bind the haze uniform (group 1) alongside globals (group 0).
        let mountain_haze_pipeline = {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("mountain-haze-pipeline"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/shaders/mountain_haze.wgsl"
                    ))
                    .into(),
                ),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mountain-haze-pipeline"),
                layout: Some(&mountain_haze_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
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
        // The cascade shader is heavy per-pixel; it renders into a half-res
        // offscreen target and is additively composited back into the main
        // pass. The offscreen pipeline writes with REPLACE blend since the
        // target is cleared per-frame before the pre-pass.
        let shooting_star_cascade_pipeline = {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("shooting-star-cascade-pipeline"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/shaders/shooting_star_cascade.wgsl"
                    ))
                    .into(),
                ),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("shooting-star-cascade-pipeline"),
                layout: Some(&quad_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
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
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("cascade-composite-pipeline"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/shaders/shooting_star_cascade_composite.wgsl"
                    ))
                    .into(),
                ),
            });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cascade-composite-pl"),
                bind_group_layouts: &[Some(&cascade_composite_layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("cascade-composite-pipeline"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
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

        let tile_pipeline_opaque_double =
            mk_tile_pipeline("tile-opaque-ds", None, &depth_3d, false);
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

        let shop_pipeline_opaque_double =
            mk_shop_pipeline("shop-opaque-ds", None, &depth_3d, false);
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

        // ---- Gold outline shell pipeline (selected tiles) ----
        let tile_outline_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tile-outline-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../../shaders/tile_outline.wgsl").into(),
            ),
        });
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
        let tile_outline_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
        let tile_glow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tile-glow-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../../shaders/tile_glow.wgsl").into(),
            ),
        });
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
        let lit_mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lit-mesh-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../../shaders/lit_mesh.wgsl").into()),
        });
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
        let (scene_prev_texture, scene_prev_view) = create_scene_prev(
            &device,
            scene_hdr_format,
            size.width.max(1),
            size.height.max(1),
        );
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
        // Journal page target — the shop's open-book mesh samples this
        // as the page-spread albedo so the embedded yaku-journal scene
        // appears painted on the open pages.
        let journal_page_texture = create_journal_page(&device, format);
        // Fullscreen offscreen for the live yaku-journal GPU render.
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
        let bloom_w = (size.width.max(1) / 2).max(1);
        let bloom_h = (size.height.max(1) / 2).max(1);
        let (bloom_ping_texture, bloom_ping_view) =
            create_post_texture(&device, scene_hdr_format, bloom_w, bloom_h, "bloom-ping");
        let (bloom_pong_texture, bloom_pong_view) =
            create_post_texture(&device, scene_hdr_format, bloom_w, bloom_h, "bloom-pong");
        let bloom_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bloom-params"),
            contents: bytemuck::bytes_of(&BloomParams {
                data0: [1.1, 0.0, 1.0 / bloom_w as f32, 1.0 / bloom_h as f32],
                data1: [1.0, 0.0, 0.0, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
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

        // Felt fluff shells: same blend/depth as `lit_mesh_blended_pipeline` but
        // `vs_felt_shell_instanced` uses `instance_index` as the shell layer so
        // one draw replaces N per-shell uniform buffers + bind-group swaps.
        let lit_mesh_felt_shell_instanced_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("lit-mesh-felt-shell-instanced-pipeline"),
                layout: Some(&lit_mesh_pl),
                vertex: wgpu::VertexState {
                    module: &lit_mesh_shader,
                    entry_point: Some("vs_felt_shell_instanced"),
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
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../../shaders/shadow.wgsl").into()),
        });
        let shadow_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow-pl"),
            bind_group_layouts: &[Some(&shadow_caster_layout)],
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
                // Match the lit_mesh / tile_glb vertex stride so a single
                // pipeline can render either caster type. Only attribute 0
                // (position) is read by the shader.
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
        let text_pipeline_scene_hdr =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
        let image_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("image-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../../shaders/image_quad.wgsl").into(),
            ),
        });
        let image_pipeline_scene_hdr =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
        let bloom_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bloom-pl"),
            bind_group_layouts: &[Some(&bloom_bind_group_layout)],
            immediate_size: 0,
        });
        let bloom_extract_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom-extract-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../../shaders/bloom_extract.wgsl").into(),
            ),
        });
        let bloom_blur_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom-blur-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../../shaders/bloom_blur.wgsl").into(),
            ),
        });
        let bloom_extract_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("bloom-extract-pipeline"),
                layout: Some(&bloom_layout),
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
            layout: Some(&bloom_layout),
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
        let bloom_composite_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("bloom-composite-pl"),
                bind_group_layouts: &[Some(&bloom_composite_bind_group_layout)],
                immediate_size: 0,
            });
        let bloom_composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom-composite-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../../shaders/bloom_composite.wgsl").into(),
            ),
        });
        let bloom_composite_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
        let tonemap_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("tonemap-pl"),
                bind_group_layouts: &[Some(&tonemap_bind_group_layout)],
                immediate_size: 0,
            });
        let tonemap_shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tonemap-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../../shaders/tonemap_composite.wgsl").into(),
            ),
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

        let bloom_scene_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-scene-bg"),
            layout: &bloom_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: bloom_params_buffer.as_entire_binding(),
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
        let bloom_ping_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-ping-bg"),
            layout: &bloom_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: bloom_params_buffer.as_entire_binding(),
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
                    resource: bloom_params_buffer.as_entire_binding(),
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
                    resource: bloom_params_buffer.as_entire_binding(),
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

        log::info!("shaders + pipelines compiled in {:?}", t0.elapsed());

        let t0 = Instant::now();
        let ui_font = load_ui_font();
        if ui_font.is_some() {
            log::info!("UI font loaded.");
        } else {
            log::warn!("No UI font found; panel text will be blank.");
        }
        let emoji_font = load_noto_emoji_font();
        if emoji_font.is_some() {
            log::info!("Noto Emoji font loaded.");
        } else {
            log::warn!("No Noto Emoji font found; tile symbols may be blank.");
        }

        log::info!("fonts loaded in {:?}", t0.elapsed());

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

        let t0 = Instant::now();
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
                        Some((rgba, w, h)) => upload_rgba_texture_with_mips(
                            &device,
                            &queue,
                            "tile-prim-albedo",
                            rgba,
                            *w,
                            *h,
                            wgpu::TextureFormat::Rgba8UnormSrgb,
                            mips,
                        ),
                        None => white_albedo(&device, &queue),
                    };
                    let normal_view = match &prim.normal_rgba {
                        Some((rgba, w, h)) => {
                            upload_rgba_texture_with_mips(
                                &device,
                                &queue,
                                &format!("tile-prim-normal-{i}"),
                                rgba,
                                *w,
                                *h,
                                wgpu::TextureFormat::Rgba8Unorm,
                                mips,
                            )
                            .1
                        }
                        None => tile_default_normal_view.clone(),
                    };
                    let metallic_roughness_view = match &prim.metallic_roughness_rgba {
                        Some((rgba, w, h)) => {
                            upload_rgba_texture_with_mips(
                                &device,
                                &queue,
                                &format!("tile-prim-mr-{i}"),
                                rgba,
                                *w,
                                *h,
                                wgpu::TextureFormat::Rgba8Unorm,
                                mips,
                            )
                            .1
                        }
                        None => tile_glb_default_mr_view.clone(),
                    };
                    let emissive_view = match &prim.emissive_rgba {
                        Some((rgba, w, h)) => {
                            upload_rgba_texture_with_mips(
                                &device,
                                &queue,
                                &format!("tile-prim-emissive-{i}"),
                                rgba,
                                *w,
                                *h,
                                wgpu::TextureFormat::Rgba8UnormSrgb,
                                mips,
                            )
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
                log::warn!("Could not load Tile.glb (3D hand tiles disabled): {e:#}");
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

        log::info!("tile mesh loaded in {:?}", t0.elapsed());

        let (shop_env_primitives, shop_environment) = crate::render::shop_glb::with_shop_glb_cpu(
            |cpu_opt| {
            let mut prims = Vec::new();
            let mut gpu_wrap = None;
            let Some(cpu) = cpu_opt else {
                return (prims, gpu_wrap);
            };
                if !cpu.environment_primitives.is_empty() {
                    for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
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
                        let mips =
                            crate::render::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                        let (_albedo_texture, albedo_view) = match &prim.albedo_rgba {
                            Some((rgba, aw, ah)) => upload_rgba_texture_with_mips(
                                &device,
                                &queue,
                                &format!("shop-env-albedo-{i}"),
                                rgba,
                                *aw,
                                *ah,
                                wgpu::TextureFormat::Rgba8UnormSrgb,
                                mips,
                            ),
                            None => white_albedo(&device, &queue),
                        };
                        let normal_view = match &prim.normal_rgba {
                            Some((rgba, nw, nh)) => {
                                upload_rgba_texture_with_mips(
                                    &device,
                                    &queue,
                                    &format!("shop-env-normal-{i}"),
                                    rgba,
                                    *nw,
                                    *nh,
                                    wgpu::TextureFormat::Rgba8Unorm,
                                    mips,
                                )
                                .1
                            }
                            None => tile_default_normal_view.clone(),
                        };
                        let metallic_roughness_view = match &prim.metallic_roughness_rgba {
                            Some((rgba, w, h)) => {
                                upload_rgba_texture_with_mips(
                                    &device,
                                    &queue,
                                    &format!("shop-env-mr-{i}"),
                                    rgba,
                                    *w,
                                    *h,
                                    wgpu::TextureFormat::Rgba8Unorm,
                                    mips,
                                )
                                .1
                            }
                            None => tile_glb_default_mr_view.clone(),
                        };
                        let emissive_view = match &prim.emissive_rgba {
                            Some((rgba, w, h)) => {
                                upload_rgba_texture_with_mips(
                                    &device,
                                    &queue,
                                    &format!("shop-env-emissive-{i}"),
                                    rgba,
                                    *w,
                                    *h,
                                    wgpu::TextureFormat::Rgba8UnormSrgb,
                                    mips,
                                )
                                .1
                            }
                            None => tile_glb_default_emissive_view.clone(),
                        };
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
                    let identity = Mat4::IDENTITY;
                    let uniform_buffer =
                        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("shop-env-uniform"),
                            contents: bytemuck::bytes_of(&CameraUniform {
                                view_proj: identity.to_cols_array(),
                                model: identity.to_cols_array(),
                                base_color_factor: [
                                    1.0,
                                    0.0,
                                    0.0,
                                    crate::render::tile_body::TEXTURED_BASE_MAP_BODY_KIND,
                                ],
                                cam_pos: [0.0; 3],
                                tile_seed: 0.0,
                                decal_atlas_uv: [0.0, 0.0, 1.0, 1.0],
                                hdr_tonemap: [0.0; 4],
                            }),
                            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                        });
                    let bind_groups: Vec<wgpu::BindGroup> = prims
                        .iter()
                        .enumerate()
                        .map(|(_bi, p)| {
                            device.create_bind_group(&wgpu::BindGroupDescriptor {
                                label: Some("shop-env-bg"),
                                layout: &tile_material_layout,
                                entries: &[
                                    wgpu::BindGroupEntry {
                                        binding: 0,
                                        resource: uniform_buffer.as_entire_binding(),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 1,
                                        resource: wgpu::BindingResource::TextureView(
                                            &p.albedo_view,
                                        ),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 2,
                                        resource: wgpu::BindingResource::Sampler(&p.sampler),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 3,
                                        resource: wgpu::BindingResource::TextureView(
                                            &shop_decal_view,
                                        ),
                                    },
                                    wgpu::BindGroupEntry {
                                        binding: 4,
                                        resource: wgpu::BindingResource::TextureView(
                                            &p.normal_view,
                                        ),
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
                                        resource: wgpu::BindingResource::TextureView(
                                            &p.emissive_view,
                                        ),
                                    },
                                ],
                            })
                        })
                        .collect();
                    gpu_wrap = Some(ShopEnvironmentGpu {
                        uniform_buffer,
                        bind_groups,
                    });
                    log::info!("Shop.glb GPU: {} primitive draw(s)", prims.len());
                }
            (prims, gpu_wrap)
            },
        );

        crate::render::shop_glb::release_shop_environment_cpu_sources_after_gpu_upload();

        let shop_env_collision_meshes = crate::render::shop_glb::with_shop_glb_cpu(|opt| {
            opt.map(|c| c.collision_meshes.clone()).unwrap_or_default()
        });

        // Kick off background relic image loading (non-blocking).
        let relic_load_start = Some(Instant::now());
        let relic_rx = Some(spawn_relic_loader());
        let (_lit_mesh_relief_default_tex, lit_mesh_relief_default_view) =
            flat_relief_height(&device, &queue);
        let pack_textures_map = load_pack_textures(
            &device,
            &queue,
            &text_bind_group_layout,
            &tile_sampler,
            &lit_mesh_relief_default_view,
        );
        // Kick off background image loading (non-blocking).
        let background_load_start = Some(Instant::now());
        let background_rx = Some(spawn_background_loader());

        // ---- Lit-mesh procedural geometry (candles + table) ----
        let candle_wax_mesh = LitMeshGpu::new(&device, &build_candle_wax_mesh(), "candle-wax");
        let candle_wick_mesh = LitMeshGpu::new(&device, &build_candle_wick_mesh(), "candle-wick");
        let table_mesh = LitMeshGpu::new(&device, &build_table_mesh(), "table");
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
        let shrine_mesh = LitMeshGpu::new(&device, &build_shrine_mesh(), "shrine");
        let dora_plinth_mesh = LitMeshGpu::new(&device, &build_dora_plinth_mesh(), "dora-plinth");
        let bug_body_mesh = LitMeshGpu::new(&device, &build_bug_body_mesh(), "bug-body");
        let bug_wing_mesh = LitMeshGpu::new(&device, &build_bug_wing_mesh(), "bug-wing");
        let bug_wing_blur_mesh =
            LitMeshGpu::new(&device, &build_bug_wing_blur_mesh(), "bug-wing-blur");
        // Phase-1 primitive registry: parallel GPU copies of meshes
        // the generic `Object3dKind::Primitive` dispatch can reach by
        // `MeshId`. Legacy named fields above still own their own
        // allocations during the migration window.
        let mut primitive_meshes: HashMap<MeshId, std::sync::Arc<LitMeshGpu>> = HashMap::new();
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
        }
        // Per-shape texture override: the coin cylinder needs its
        // engraved heightmap bound at both albedo and relief slots so
        // the Metal branch in lit_mesh.wgsl can sample the cash-coin
        // relief. Populated now so `primitive_textures` is ready by
        // the time `dispatch_primitive` first creates an instance.
        let mut primitive_textures: HashMap<
            crate::render::primitive::MeshId,
            (wgpu::TextureView, wgpu::TextureView),
        > = HashMap::new();
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

        // Pre-allocate candle slots (gameplay: score pair + two hand-strip
        // pairs + footlight). Each slot owns two instances: wax + wick.
        const NUM_CANDLE_SLOTS: usize = 7;
        let mut candle_instances: Vec<[LitMeshInstance; 2]> = Vec::with_capacity(NUM_CANDLE_SLOTS);
        for _ in 0..NUM_CANDLE_SLOTS {
            candle_instances.push([
                LitMeshInstance::new(
                    &device,
                    &lit_mesh_material_layout,
                    &shadow_caster_layout,
                    &lit_mesh_white_view,
                    &lit_mesh_relief_default_view,
                    &tile_sampler,
                ),
                LitMeshInstance::new(
                    &device,
                    &lit_mesh_material_layout,
                    &shadow_caster_layout,
                    &lit_mesh_white_view,
                    &lit_mesh_relief_default_view,
                    &tile_sampler,
                ),
            ]);
        }
        let table_instance = LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        );
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
        let ribbon_slot_zodiac: Vec<Option<(u8, u8)>> = vec![None; MAX_RIBBON_SLOTS];
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
        let talisman_height_paths = [
            ("textures/talismans/talisman_jade.png", "talisman-jade-hm"),
            ("textures/talismans/talisman_pearl.png", "talisman-pearl-hm"),
            (
                "textures/talismans/talisman_gilded.png",
                "talisman-gilded-hm",
            ),
            (
                "textures/talismans/talisman_polychrome.png",
                "talisman-polychrome-hm",
            ),
            ("textures/talismans/talisman_kiln.png", "talisman-kiln-hm"),
            (
                "textures/talismans/talisman_bamboo.png",
                "talisman-bamboo-hm",
            ),
            ("textures/talismans/talisman_dots.png", "talisman-dots-hm"),
            (
                "textures/talismans/talisman_characters.png",
                "talisman-characters-hm",
            ),
            (
                "textures/talismans/talisman_honors.png",
                "talisman-honors-hm",
            ),
            (
                "textures/talismans/talisman_wildflower.png",
                "talisman-wildflower-hm",
            ),
            (
                "textures/talismans/talisman_conformity.png",
                "talisman-conformity-hm",
            ),
        ];
        let mut talisman_height_views: Vec<wgpu::TextureView> = Vec::new();
        for &(path, label) in &talisman_height_paths {
            let (_tex, view) = load_metal_heightmap(&device, &queue, path, label);
            talisman_height_views.push(view);
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
        let mut shrine_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_SHRINE_SLOTS);
        for _ in 0..MAX_SHRINE_SLOTS {
            shrine_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_white_view,
                &lit_mesh_relief_default_view,
                &tile_sampler,
            ));
        }
        let mut dora_plinth_instances: Vec<LitMeshInstance> =
            Vec::with_capacity(MAX_DORA_PLINTH_SLOTS);
        for _ in 0..MAX_DORA_PLINTH_SLOTS {
            dora_plinth_instances.push(LitMeshInstance::new(
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
        let cascade_token_instances = make_pool(MAX_CASCADE_TOKEN_SLOTS);
        let extruded_glyph_instances = make_pool(MAX_EXTRUDED_GLYPH_SLOTS);
        let debug_axes_instances = make_pool(3);

        // Build the GPU profiler up-front while we still have a borrow of
        // device/queue (the struct literal below moves them).
        let gpu_profiler =
            crate::render::gpu_profiler::GpuProfiler::new(&device, &queue, timestamp_supported);

        log::info!("WgpuRenderer::new() total: {:?}", t_total.elapsed());

        Ok(Self {
            target,
            device,
            queue,
            config,
            depth_texture,
            depth_view,
            ssr_prev_depth_texture,
            ssr_prev_depth_view,
            quad_pipeline,
            gradient_quad_pipeline,
            flame_pipeline,
            flame_view_buffer,
            flame_view_bind_group,
            flame_particles: crate::render::flame_particles::FlameParticleSystem::new(),
            flame_particle_staging: Vec::with_capacity(512),
            starfield_pipeline,
            ember_drift_pipeline,
            golden_dust_pipeline,
            moonlit_water_pipeline,
            moon_albedo_bind_group,
            sunlit_water_pipeline,
            mountain_haze_pipeline,
            haze_uniform_buffer,
            haze_uniform_bind_group,
            shooting_star_cascade_pipeline,
            cascade_composite_pipeline,
            cascade_composite_layout,
            cascade_composite_sampler,
            cascade_offscreen_texture,
            cascade_offscreen_view,
            cascade_composite_bind_group,
            tile_pipeline_opaque_double,
            tile_pipeline_opaque_cull,
            tile_pipeline_blend_double,
            tile_pipeline_blend_cull,
            shop_pipeline_opaque_double,
            shop_pipeline_opaque_cull,
            shop_pipeline_blend_double,
            shop_pipeline_blend_cull,
            tile_outline_pipeline,
            tile_glow_pipeline,
            globals_buffer,
            globals_bind_group,
            tile_material_layout,
            tile_outline_frame_uniform_buffer,
            tile_outline_instance_buffer,
            tile_outline_frame_bind_group,
            tile_outline_instances_staging: Vec::new(),
            tile_outline_batch_ranges: Vec::new(),
            point_lights_buffer,
            tile_occluders_buffer,
            point_lights_bind_group,
            shop_gltf_point_lights_buffer,
            shop_gltf_point_lights_scene_bind_group,
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
            shop_env_height_scale: crate::render::shop_glb::SHOP_ENV_HEIGHT_SCALE,
            shop_env_linear_exposure: crate::render::shop_glb::SHOP_ENV_LINEAR_EXPOSURE,
            shop_env_ambient_scale: crate::render::shop_glb::SHOP_ENV_AMBIENT_SCALE,
            shop_lit_mesh_gltf_punctual_scale: crate::render::shop_glb::SHOP_LIT_MESH_GLTF_PUNCTUAL_SCALE,
            shop_env_collision_meshes,
            tile_base_color_factor,
            // Populated on first render() from RenderSettings.tileset_name.
            tile_set: None,
            hand_tiles: Vec::new(),
            showcase_tiles: Vec::new(),
            tile_face_overlays: HashMap::new(),
            debuff_marker_overlay: None,
            text_label_cache: HashMap::new(),
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
            size,
            last_focus: usize::MAX,
            focus_spin: None,
            focus_t: Vec::new(),
            tile_anim_y: Vec::new(),
            tile_anim_x: Vec::new(),
            tile_uids: Vec::new(),
            departing_tiles: Vec::new(),
            proj: ProjectionCache::default(),
            last_pick_models: Vec::new(),
            last_pick_camera: None,
            last_relic_models: Vec::new(),
            last_pickable_relic_models: Vec::new(),
            relic_slot_texture: vec![None; MAX_RELIC_SLOTS],
            pack_instances,
            pack_slot_texture: vec![None; 4],
            ribbon_mesh,
            talisman_mesh,
            shrine_mesh,
            dora_plinth_mesh,
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
            shrine_instances,
            dora_plinth_instances,
            last_ribbon_models: Vec::new(),
            last_ribbon_slot_count: 0,
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
            cascade_token_instances,
            extruded_glyph_instances,
            glyph_cpu_cache: crate::render::glyph_mesh::GlyphMeshCache::new(),
            extruded_glyph_meshes: HashMap::new(),
            primitive_meshes,
            primitive_instances: HashMap::new(),
            primitive_textures,
            last_primitive_pick_models: HashMap::new(),
            debug_axes_instances,
            last_yaku_tablet_models: Vec::new(),
            last_wood_tablet_models: Vec::new(),
            last_bowl_model: None,
            last_mirror_model: None,
            last_debug_pickables: Vec::new(),
            last_gameplay_fog_wall_horizon_y: None,
            last_gameplay_fog_wall_center_x: None,
            active_scene_key: None,
            debug_arrange_override: None,
            committed_arrange_rotations: std::collections::HashMap::new(),
            last_frame: Instant::now(),
            frame_dt: 0.0,
            obj3d_hover_state: HashMap::new(),
            creation_time: Instant::now(),
            relic_textures: HashMap::new(),
            relic_rx,
            relic_load_start,
            pack_textures: pack_textures_map,
            background_textures: HashMap::new(),
            background_rx,
            background_load_start,
            prev_tile_world: HashMap::new(),
            prev_frame_shadows_enabled: false,
            showcase_decal_atlas: None,
            showcase_decal_atlas_tileset: None,
            lit_mesh_material_layout,
            lit_mesh_spot_ssr_layout,
            lit_mesh_ssr_buffer,
            lit_mesh_spot_ssr_bind_group,
            lit_mesh_ssr_sampler,
            scene_prev_texture,
            scene_prev_view,
            scene_color_texture,
            scene_color_view,
            journal_page_texture,
            journal_scene_texture,
            journal_scene_view,
            journal_scene_view_generation: 0,
            bloom_extract_pipeline,
            bloom_blur_pipeline,
            bloom_composite_pipeline,
            bloom_bind_group_layout,
            bloom_composite_bind_group_layout,
            bloom_params_buffer,
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
            tonemap_shader_module,
            tonemap_pipeline_layout,
            swapchain_sdr_format,
            swapchain_hdr_available,
            tonemap_exposure: 1.0,
            lit_mesh_pipeline,
            lit_mesh_blended_pipeline,
            lit_mesh_felt_shell_instanced_pipeline,
            lit_mesh_white_view,
            lit_mesh_relief_default_view,
            talisman_height_views,
            talisman_slot_kind,
            candle_wax_mesh,
            candle_wick_mesh,
            table_mesh,
            relic_box_mesh,
            relic_box_tris,
            relic_tri_lists: HashMap::new(),
            pack_mesh,
            relic_meshes: HashMap::new(),
            candle_instances,
            table_instance,
            active_felt_shell_count: 0,
            felt_shader_lod: 2.0,
            // Default to felt; `apply_render_settings` overwrites this each
            // frame once the user's persisted choice has been threaded in.
            table_material: MaterialParams::lacquered_wood(),
            relic_instances,
            shadow_map_view,
            shadow_caster_layout,
            shadow_globals_buffer,
            shadow_sample_bind_group,
            shadow_pipeline,
            gpu_profiler,
            pending_screenshot: std::cell::Cell::new(None),
        })
    }
}
