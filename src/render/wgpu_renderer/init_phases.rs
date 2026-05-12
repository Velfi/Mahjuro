//! Large chunks of `WgpuRenderer::new` split into `#[inline(never)]` helpers so
//! LLVM does not optimize one multi-thousand-line function as a single unit.

use std::time::Instant;

use super::embedded_wgsl;
use super::{
    RenderTarget, TargetInit, clamp_render_physical_size, create_depth, create_depth_copy,
};

/// Env set on the `vulkan-wsi-probe` subprocess so it does not recurse into another probe.
#[cfg(target_os = "windows")]
pub(crate) const VULKAN_PROBE_CHILD_ENV: &str = "MAHJURO_VULKAN_PROBE_CHILD";

#[cfg(target_os = "windows")]
fn win32_maybe_clear_vulkan_env_after_probe() {
    use std::process::Command;
    if std::env::var_os(VULKAN_PROBE_CHILD_ENV).is_some() {
        return;
    }
    if std::env::var_os("MAHJURO_SKIP_VULKAN_WSI_PROBE").is_some() {
        return;
    }
    let wb = std::env::var("WGPU_BACKEND").unwrap_or_default().to_lowercase();
    if !wb.contains("vulkan") && wb != "vk" {
        return;
    }
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let status = Command::new(exe)
        .arg("vulkan-wsi-probe")
        .env(VULKAN_PROBE_CHILD_ENV, "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let ok = matches!(&status, Ok(s) if s.success());
    if !ok {
        log::warn!(
            "Vulkan WSI smoke probe failed ({status:?}); using DirectX 12 instead. \
             Set MAHJURO_SKIP_VULKAN_WSI_PROBE=1 to force Vulkan."
        );
        // SAFETY: renderer init runs on the main thread before background work; no concurrent
        // environment access elsewhere during this early probe path.
        unsafe {
            std::env::remove_var("WGPU_BACKEND");
        }
    }
}

/// Internal CLI (`mahjuro vulkan-wsi-probe`): window + adapter + device + first swapchain configure.
pub(crate) fn run_vulkan_wsi_probe_smoke() -> anyhow::Result<()> {
    let shell = crate::sdl_shell::SdlShell::new("Vulkan WSI probe", 256, 256, false)?;
    early_gpu_and_depth(TargetInit::Windowed {
        window: shell.window.clone(),
        hdr_enabled: false,
    })?;
    Ok(())
}

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
    #[cfg(target_os = "windows")]
    win32_maybe_clear_vulkan_env_after_probe();

    let mut instance_desc =
        wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    if cfg!(target_os = "windows") && std::env::var_os("WGPU_BACKEND").is_none() {
        // Vulkan + Win32 swapchain still faults on some AMD stacks; DX12 is the safe default.
        // Set `WGPU_BACKEND=vulkan` (or `vk`) to test Vulkan.
        instance_desc.backends = wgpu::Backends::DX12;
    }
    let instance = wgpu::Instance::new(instance_desc);
    log::debug!("wgpu: instance created");

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
            log::debug!("wgpu: window surface created");
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

    let power_preference =
        wgpu::PowerPreference::from_env().unwrap_or(wgpu::PowerPreference::default());
    let t_adapter = Instant::now();
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference,
        compatible_surface: surface_opt.as_ref(),
        force_fallback_adapter: false,
    }))
    .map_err(|e| anyhow::anyhow!("adapter: {e:?}"))?;
    let ai = adapter.get_info();
    log::debug!(
        "wgpu: adapter OK — '{}' ({:?}, power_pref={power_preference:?}) in {:?}",
        ai.name,
        ai.backend,
        t_adapter.elapsed()
    );

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

    let win32_vulkan = cfg!(target_os = "windows")
        && adapter.get_info().backend == wgpu::Backend::Vulkan;

    let (format, swapchain_sdr_format, swapchain_hdr_available) = match surface_opt.as_ref() {
        Some(surface) => {
            let caps = surface.get_capabilities(&adapter);
            // Win32 Vulkan WSI often advertises several sRGB formats; picking RGBA8 first can
            // still route through a BGRA-oriented present path on AMD — some drivers fault in
            // vkCreateSwapchainKHR when the format doesn't match the compositor's preference.
            let swapchain_sdr_format = if win32_vulkan {
                caps.formats
                    .iter()
                    .copied()
                    .find(|&f| f == wgpu::TextureFormat::Bgra8UnormSrgb)
                    .or_else(|| caps.formats.iter().find(|f| f.is_srgb()).copied())
                    .unwrap_or(caps.formats[0])
            } else {
                caps.formats
                    .iter()
                    .find(|f| f.is_srgb())
                    .copied()
                    .unwrap_or(caps.formats[0])
            };
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
    log::debug!("wgpu: surface format / caps resolved");

    // `downlevel_webgl2_defaults` zeros storage-buffer counts for WebGL2-tier parity; the
    // emissive probe GI compute pass needs at least one `storage` binding. Take the adapter
    // ceiling so Metal/Vulkan/DX12 keep full capability (see crash: max_storage_buffers… = 0).
    //
    // Win32 + Vulkan: starting from the WebGL2 downlevel preset has been linked to AMD
    // proprietary (LLPC) faults during the first `Surface::configure` / swapchain creation.
    // Use WebGPU default limits (still clamped to the adapter via `using_resolution`) so the
    // logical device matches what the desktop drivers expect.
    let mut limits = if win32_vulkan {
        wgpu::Limits::default().using_resolution(adapter.limits())
    } else {
        wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits())
    };
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
    // TIMESTAMP_QUERY + Win32 Vulkan + some AMD stacks: sporadic faults around swapchain setup.
    let timestamp_supported = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY)
        && !win32_vulkan;
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

    log::debug!("wgpu: requesting logical device…");
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
    log::debug!("wgpu: device + queue OK in {:?}", t_device.elapsed());

    let (target, config) = match surface_opt {
        Some(surface) => {
            let caps = surface.get_capabilities(&adapter);
            let mut config = surface
                .get_default_config(&adapter, size.width.max(1), size.height.max(1))
                .ok_or_else(|| anyhow::anyhow!("no default surface config"))?;
            config.format = format;
            config.present_mode = wgpu::PresentMode::Fifo;
            config.desired_maximum_frame_latency = 2;

            if adapter.get_info().backend == wgpu::Backend::Vulkan {
                // Win32 Vulkan: `Auto` alpha is resolved inside wgpu, but some AMD + WSI stacks
                // have faulted in vkCreateSwapchainKHR; match the pattern from working samples
                // (explicit first advertised mode — typically Opaque).
                if let Some(&mode) = caps.alpha_modes.first() {
                    config.alpha_mode = mode;
                }
                // Dual-buffer swapchain (latency 1) avoids a class of AMD driver issues around
                // triple-buffered present on Windows Vulkan.
                config.desired_maximum_frame_latency = config.desired_maximum_frame_latency.min(1);
                #[cfg(target_os = "windows")]
                {
                    // AMD + Windows Vulkan: requesting TRANSFER_SRC on the swapchain has been
                    // observed to segfault inside the driver during/just after vkCreateSwapchainKHR.
                    // Opt back in with MAHJURO_VULKAN_WIN_SURFACE_COPY=1 (may crash on some stacks).
                    if std::env::var_os("MAHJURO_VULKAN_WIN_SURFACE_COPY").is_some() {
                        config.usage |= wgpu::TextureUsages::COPY_SRC;
                    }
                }
                #[cfg(not(target_os = "windows"))]
                {
                    config.usage |= wgpu::TextureUsages::COPY_SRC;
                }
            } else {
                config.usage |= wgpu::TextureUsages::COPY_SRC;
            }
            log::debug!(
                "wgpu: configuring swapchain ({}×{}, {:?})…",
                config.width,
                config.height,
                config.format
            );
            surface.configure(&device, &config);
            log::debug!("wgpu: swapchain configured");
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
