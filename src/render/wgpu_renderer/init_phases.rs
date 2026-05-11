//! Large chunks of `WgpuRenderer::new` split into `#[inline(never)]` helpers so
//! LLVM does not optimize one multi-thousand-line function as a single unit.

use std::time::Instant;

use super::embedded_wgsl;
use super::{
    RenderTarget, TargetInit, clamp_render_physical_size, create_depth, create_depth_copy,
};

/// All WGSL `ShaderModule`s created during renderer init (no pipelines).
pub(super) struct RendererShaderPack {
    pub quad: wgpu::ShaderModule,
    pub tile: wgpu::ShaderModule,
    pub shop: wgpu::ShaderModule,
    pub text: wgpu::ShaderModule,
    pub gradient: wgpu::ShaderModule,
    pub squircle: wgpu::ShaderModule,
    pub flame: wgpu::ShaderModule,
    pub starfield: wgpu::ShaderModule,
    pub ember_drift: wgpu::ShaderModule,
    pub golden_dust: wgpu::ShaderModule,
    pub moonlit_water: wgpu::ShaderModule,
    pub sunlit_water: wgpu::ShaderModule,
    pub mountain_haze: wgpu::ShaderModule,
    pub shooting_star_cascade: wgpu::ShaderModule,
    pub cascade_composite: wgpu::ShaderModule,
    pub tile_outline: wgpu::ShaderModule,
    pub tile_glow: wgpu::ShaderModule,
    pub lit_mesh: wgpu::ShaderModule,
    pub shadow: wgpu::ShaderModule,
    pub image: wgpu::ShaderModule,
    pub bloom_extract: wgpu::ShaderModule,
    pub bloom_blur: wgpu::ShaderModule,
    pub bloom_composite: wgpu::ShaderModule,
    pub tonemap: wgpu::ShaderModule,
    pub emissive_probe_update: wgpu::ShaderModule,
    pub emissive_probe_apply: wgpu::ShaderModule,
    pub emissive_gi_composite: wgpu::ShaderModule,
}

#[inline(never)]
pub(super) fn create_renderer_shader_modules(device: &wgpu::Device) -> RendererShaderPack {
    RendererShaderPack {
        quad: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quad-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::QUAD.into()),
        }),
        tile: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tile-3d-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::TILE_3D.into()),
        }),
        shop: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shop-glb-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::SHOP_GLB.into()),
        }),
        text: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("text-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::TEXT_QUAD.into()),
        }),
        gradient: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gradient_quad.wgsl"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::GRADIENT_QUAD.into()),
        }),
        squircle: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("squircle_quad.wgsl"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::SQUIRCLE_QUAD.into()),
        }),
        flame: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flame.wgsl"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::FLAME.into()),
        }),
        starfield: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("starfield-pipeline"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::STARFIELD.into()),
        }),
        ember_drift: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ember-drift-pipeline"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::EMBER_DRIFT.into()),
        }),
        golden_dust: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("golden-dust-pipeline"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::GOLDEN_DUST.into()),
        }),
        moonlit_water: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("moonlit-water-pipeline"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::MOONLIT_WATER.into()),
        }),
        sunlit_water: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("sunlit-water-pipeline"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::SUNLIT_WATER.into()),
        }),
        mountain_haze: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("mountain-haze-pipeline"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::MOUNTAIN_HAZE.into()),
        }),
        shooting_star_cascade: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shooting-star-cascade-pipeline"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::SHOOTING_STAR_CASCADE.into()),
        }),
        cascade_composite: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cascade-composite-pipeline"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::SHOOTING_STAR_CASCADE_COMPOSITE.into()),
        }),
        tile_outline: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tile-outline-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::TILE_OUTLINE.into()),
        }),
        tile_glow: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tile-glow-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::TILE_GLOW.into()),
        }),
        lit_mesh: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lit-mesh-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::LIT_MESH.into()),
        }),
        shadow: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::SHADOW.into()),
        }),
        image: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("image-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::IMAGE_QUAD.into()),
        }),
        bloom_extract: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom-extract-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::BLOOM_EXTRACT.into()),
        }),
        bloom_blur: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom-blur-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::BLOOM_BLUR.into()),
        }),
        bloom_composite: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom-composite-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::BLOOM_COMPOSITE.into()),
        }),
        tonemap: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tonemap-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::TONEMAP_COMPOSITE.into()),
        }),
        emissive_probe_update: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("emissive-probe-update-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::EMISSIVE_PROBE_UPDATE.into()),
        }),
        emissive_probe_apply: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("emissive-probe-apply-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::EMISSIVE_PROBE_APPLY.into()),
        }),
        emissive_gi_composite: device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("emissive-gi-composite-shader"),
            source: wgpu::ShaderSource::Wgsl(embedded_wgsl::EMISSIVE_GI_COMPOSITE.into()),
        }),
    }
}

/// Through depth + SSR depth-copy textures (before WGSL modules).
pub(super) struct EarlyGpuState {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub size: crate::physical_size::PhysicalSize,
    pub target: RenderTarget,
    pub config: wgpu::SurfaceConfiguration,
    pub format: wgpu::TextureFormat,
    pub swapchain_sdr_format: wgpu::TextureFormat,
    pub swapchain_hdr_available: bool,
    pub timestamp_supported: bool,
    pub depth_texture: wgpu::Texture,
    pub depth_view: wgpu::TextureView,
    pub ssr_prev_depth_texture: wgpu::Texture,
    pub ssr_prev_depth_view: wgpu::TextureView,
}

#[inline(never)]
pub(super) fn early_gpu_and_depth(target_init: TargetInit) -> anyhow::Result<EarlyGpuState> {
    let instance = wgpu::Instance::default();

    let (surface_opt, size, hdr_enabled): (
        Option<wgpu::Surface<'static>>,
        crate::physical_size::PhysicalSize,
        bool,
    ) = match &target_init {
        TargetInit::Windowed {
            window,
            hdr_enabled,
        } => {
            let (pw, ph) = window.size_in_pixels();
            let size = crate::physical_size::PhysicalSize::new(pw.max(1), ph.max(1));
            use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
            let raw_window_handle = window
                .window_handle()
                .map_err(|e| anyhow::anyhow!("window_handle: {e}"))?
                .as_raw();
            let raw_display_handle = window
                .display_handle()
                .map_err(|e| anyhow::anyhow!("display_handle: {e}"))?
                .as_raw();
            let surface = unsafe {
                instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                    raw_display_handle: Some(raw_display_handle),
                    raw_window_handle,
                })?
            };
            (Some(surface), size, *hdr_enabled)
        }
        TargetInit::Headless {
            width,
            height,
            hdr_enabled,
        } => {
            let size = crate::physical_size::PhysicalSize::new((*width).max(1), (*height).max(1));
            (None, size, *hdr_enabled)
        }
    };

    let size = clamp_render_physical_size(size);

    let t_adapter = Instant::now();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: surface_opt.as_ref(),
        force_fallback_adapter: false,
    }))
    .map_err(|e| anyhow::anyhow!("adapter: {e:?}"))?;
    log::debug!("wgpu adapter acquired in {:?}", t_adapter.elapsed());

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

    let (format, swapchain_sdr_format, swapchain_hdr_available) = match surface_opt.as_ref() {
        Some(surface) => {
            let caps = surface.get_capabilities(&adapter);
            let swapchain_sdr_format = caps
                .formats
                .iter()
                .find(|f| f.is_srgb())
                .copied()
                .unwrap_or(caps.formats[0]);
            let swapchain_hdr_available = caps.formats.contains(&wgpu::TextureFormat::Rgba16Float);
            let format = if hdr_enabled {
                if swapchain_hdr_available {
                    log::info!("HDR enabled — using Rgba16Float surface format");
                    wgpu::TextureFormat::Rgba16Float
                } else {
                    log::warn!("HDR requested but Rgba16Float not supported; falling back to sRGB");
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

    // `downlevel_webgl2_defaults` zeros storage-buffer counts for WebGL2-tier parity; the
    // emissive probe GI compute pass needs at least one `storage` binding. Take the adapter
    // ceiling so Metal/Vulkan/DX12 keep full capability (see crash: max_storage_buffers… = 0).
    let mut limits = wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());
    let al = adapter.limits();
    limits.max_storage_buffers_per_shader_stage = limits
        .max_storage_buffers_per_shader_stage
        .max(al.max_storage_buffers_per_shader_stage)
        .max(1);
    limits.max_storage_buffer_binding_size = limits
        .max_storage_buffer_binding_size
        .max(al.max_storage_buffer_binding_size);
    // Same downlevel preset disables compute (`max_compute_*` = 0); emissive probe update uses
    // `@workgroup_size(64, 1, 1)` and `dispatch_workgroups` — restore adapter compute limits.
    limits.max_compute_invocations_per_workgroup = limits
        .max_compute_invocations_per_workgroup
        .max(al.max_compute_invocations_per_workgroup)
        .max(64);
    limits.max_compute_workgroup_size_x = limits
        .max_compute_workgroup_size_x
        .max(al.max_compute_workgroup_size_x)
        .max(64);
    limits.max_compute_workgroup_size_y = limits
        .max_compute_workgroup_size_y
        .max(al.max_compute_workgroup_size_y)
        .max(1);
    limits.max_compute_workgroup_size_z = limits
        .max_compute_workgroup_size_z
        .max(al.max_compute_workgroup_size_z)
        .max(1);
    limits.max_compute_workgroup_storage_size = limits
        .max_compute_workgroup_storage_size
        .max(al.max_compute_workgroup_storage_size);
    limits.max_compute_workgroups_per_dimension = limits
        .max_compute_workgroups_per_dimension
        .max(al.max_compute_workgroups_per_dimension)
        .max(1);

    // Opt into TIMESTAMP_QUERY when the adapter supports it so the GPU pass profiler can record
    // start/end ticks per render pass. INSIDE_ENCODERS (debug only) allows
    // `encoder.write_timestamp()` outside of render passes.
    let mut required_features = wgpu::Features::CLEAR_TEXTURE;
    let timestamp_supported = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
    if timestamp_supported {
        required_features |= wgpu::Features::TIMESTAMP_QUERY;
        #[cfg(debug_assertions)]
        if adapter
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
        {
            required_features |= wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
        }
    }

    let t_device = Instant::now();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("mahjuro-device"),
        required_features,
        required_limits: limits,
        experimental_features: wgpu::ExperimentalFeatures::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::default(),
    }))
    .map_err(|e| anyhow::anyhow!("device: {e:?}"))?;
    log::debug!("wgpu device created in {:?}", t_device.elapsed());

    let (target, config) = match surface_opt {
        Some(surface) => {
            let mut config = surface
                .get_default_config(&adapter, size.width.max(1), size.height.max(1))
                .ok_or_else(|| anyhow::anyhow!("no default surface config"))?;
            config.format = format;
            config.present_mode = wgpu::PresentMode::Fifo;
            config.desired_maximum_frame_latency = 2;
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

    let (depth_texture, depth_view) = create_depth(&device, size.width.max(1), size.height.max(1));
    let (ssr_prev_depth_texture, ssr_prev_depth_view) =
        create_depth_copy(&device, size.width.max(1), size.height.max(1));

    Ok(EarlyGpuState {
        device,
        queue,
        size,
        target,
        config,
        format,
        swapchain_sdr_format,
        swapchain_hdr_available,
        timestamp_supported,
        depth_texture,
        depth_view,
        ssr_prev_depth_texture,
        ssr_prev_depth_view,
    })
}
