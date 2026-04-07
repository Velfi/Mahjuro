//! WGPU: depth-tested 3D tile meshes for the hand + 2D UI quads on top.

use std::collections::HashMap;

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use glam::Mat4;
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::core::relic::RelicId;
use crate::scenes::BackgroundId;
use crate::core::tile::{Suit, Tile};
use crate::render::animation::apply_transform_rect;
use crate::render::decal::{
    load_noto_emoji_font, load_ui_font, rasterize_label, rasterize_tile_decal, tile_short_label,
    tile_suit_emoji,
};
use crate::render::tile_glb::{Vertex3dTex, load_glb_tile_from_bytes, normalize_mesh};

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    screen: [f32; 2],
    time: f32,
    _pad: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_proj: [f32; 16],
    model: [f32; 16],
    base_color_factor: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuInstance {
    pub rect: [f32; 4],
    pub color: [f32; 4],
}

#[allow(dead_code)]
struct TileMeshGpu {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
}

/// A relic icon to draw as a textured quad at a screen-space rect.
pub struct RelicIcon {
    /// Position in screen pixels: [x, y, w, h].
    pub rect: [f32; 4],
    /// Which relic image to display.
    pub relic_id: crate::core::relic::RelicId,
}

/// One tooltip's rendering data — quads and text drawn as a unit so that
/// child tooltips fully occlude their parents (like DOM z-ordering).
pub struct TooltipLayer {
    pub instances: Vec<GpuInstance>,
    pub labels: Vec<TextLabel>,
}

/// A rasterized text label to draw over a screen-space rect.
pub struct TextLabel {
    /// Position in screen pixels: [x, y, w, h].
    pub rect: [f32; 4],
    /// Text to render.
    pub text: String,
    /// Colour for the text glyphs (default: white).
    pub color: [f32; 4],
}

/// GPU resources for a single hand tile.
///
/// Each tile has its own uniform buffer (updated every frame with the per-tile
/// model matrix) and bind group (holds the tile's rasterised decal texture).
/// Storing them per-tile means all 14 `write_buffer` calls target distinct
/// GPU allocations, so every tile's matrix is visible when the command buffer
/// executes — no dynamic-offset trickery required.
#[allow(dead_code)]
struct HandTileGpu {
    /// Written every frame with view_proj + model + base_color_factor.
    uniform_buffer: wgpu::Buffer,
    /// Binds: uniform buffer · shared albedo · shared sampler · per-tile decal.
    bind_group: wgpu::BindGroup,
    /// Cached to skip re-rasterisation when the tile hasn't changed.
    tile_id: (Suit, u8),
    /// Main label (number or name) for the tile face.
    symbol: String,
    /// Emoji suit indicator rendered below the main label.
    suit_emoji: String,
    /// Suit colour for rendering the symbol (RGBA, linear).
    suit_color: [f32; 4],
    /// Kept alive so the GPU texture is not freed while bind_group references it.
    #[allow(dead_code)]
    decal_texture: wgpu::Texture,
}

/// A tile animating away from the hand (discard / score removal).
struct DepartingTile {
    /// Visual identity for rendering.
    symbol: String,
    suit_emoji: String,
    suit_color: [f32; 4],
    /// Screen-space rect at the moment of departure.
    start_rect: [f32; 4],
    /// Velocity: (vx, vy) in pixels/sec. Positive Y = downward.
    velocity: (f32, f32),
    /// Angular velocity for the "skip" rotation (radians/sec).
    spin_speed: f32,
    /// Seconds elapsed since departure started.
    elapsed: f32,
    /// Total lifetime before the tile disappears.
    lifetime: f32,
}

pub struct WgpuRenderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    quad_pipeline: wgpu::RenderPipeline,
    tile_quad_pipeline: wgpu::RenderPipeline,
    light_beam_pipeline: wgpu::RenderPipeline,
    #[allow(dead_code)]
    tile_pipeline: wgpu::RenderPipeline,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    tile_material_layout: wgpu::BindGroupLayout,
    tile_sampler: wgpu::Sampler,
    /// Shared albedo texture (from the GLB, or 1×1 white fallback).
    #[allow(dead_code)]
    tile_albedo_texture: wgpu::Texture,
    tile_albedo_view: wgpu::TextureView,
    tile_base_color_factor: [f32; 4],
    /// Per-hand-tile GPU resources; kept in sync with the hand via `update_hand_tiles`.
    hand_tiles: Vec<HandTileGpu>,
    #[allow(dead_code)]
    vertex_buffer: wgpu::Buffer,
    #[allow(dead_code)]
    index_buffer: wgpu::Buffer,
    #[allow(dead_code)]
    tile_mesh: Option<TileMeshGpu>,
    // --- Text overlay pipeline ---
    text_pipeline: wgpu::RenderPipeline,
    text_bind_group_layout: wgpu::BindGroupLayout,
    // --- Image quad pipeline (full-colour textures for relic icons) ---
    image_pipeline: wgpu::RenderPipeline,
    ui_font: Option<fontdue::Font>,
    emoji_font: Option<fontdue::Font>,
    pub size: winit::dpi::PhysicalSize<u32>,
    /// Last focused tile index — used to detect focus changes.
    last_focus: usize,
    /// When the focused tile changed: (slot_index, start_time). Drives the 360° spin.
    focus_spin: Option<(usize, Instant)>,
    /// Per-tile focus blend factor (0.0 = unfocused, 1.0 = focused). Lerped each frame.
    focus_t: Vec<f32>,
    /// Per-tile Y animation offset (positive = below rest position). Lerped toward 0 each frame.
    tile_anim_y: Vec<f32>,
    /// Per-tile X animation offset (in slot-width units). Used for sort shuffle animations.
    tile_anim_x: Vec<f32>,
    /// Per-tile unique id — used to detect when a tile slot changes identity.
    tile_uids: Vec<u32>,
    /// Tiles currently animating away (discard/score). Each entry: (HandTileGpu data, slot rect, velocity, elapsed time).
    departing_tiles: Vec<DepartingTile>,
    /// Hand slots from the previous frame, used for departure animation positioning.
    prev_hand_slots: Vec<(f32, f32, f32, f32)>,
    /// Timestamp of the previous frame — used to compute delta time for lerping.
    last_frame: Instant,
    /// Creation time — used as a stable reference for cyclic animations.
    creation_time: Instant,
    /// Cached relic icon textures, populated asynchronously from the loader thread.
    relic_textures: HashMap<RelicId, RelicTextureGpu>,
    /// Receives decoded relic RGBA data from the background loader thread.
    relic_rx: Option<mpsc::Receiver<DecodedRelicImage>>,
    /// Cached background textures, populated asynchronously.
    background_textures: HashMap<BackgroundId, BackgroundTextureGpu>,
    /// Receives decoded background image data from the background loader thread.
    background_rx: Option<mpsc::Receiver<DecodedBackgroundImage>>,
    /// GPU fluid simulation for atmospheric smoke effects (None if compute unsupported).
    pub fluid: Option<super::fluid::FluidSim>,
}

/// Pre-loaded relic icon texture + bind group for the image pipeline.
struct RelicTextureGpu {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

/// Decoded relic image data sent from the background loader thread.
struct DecodedRelicImage {
    id: RelicId,
    name: &'static str,
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

/// Pre-loaded background texture + bind group for the image pipeline.
struct BackgroundTextureGpu {
    #[allow(dead_code)]
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

/// Decoded background image data sent from the background loader thread.
struct DecodedBackgroundImage {
    id: BackgroundId,
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

// ---------------------------------------------------------------------------
// Texture helpers
// ---------------------------------------------------------------------------

fn upload_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
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
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn white_albedo(device: &wgpu::Device, queue: &wgpu::Queue) -> (wgpu::Texture, wgpu::TextureView) {
    upload_rgba_texture(
        device,
        queue,
        "tile-albedo-white",
        &[255, 255, 255, 255],
        1,
        1,
    )
}

/// Spawn a background thread that decodes all relic PNGs and sends the RGBA
/// data back over a channel.  The main thread uploads to the GPU as results
/// arrive (see `poll_relic_textures`).
fn spawn_relic_loader() -> mpsc::Receiver<DecodedRelicImage> {
    use crate::core::relic::all_relic_defs;

    let (tx, rx) = mpsc::channel();

    // Collect the static data we need before moving into the thread.
    let defs: Vec<(RelicId, &'static str, String)> = all_relic_defs()
        .iter()
        .map(|d| {
            let asset_path = format!("relics/{}", d.id.asset_filename());
            (d.id, d.name, asset_path)
        })
        .collect();

    std::thread::Builder::new()
        .name("relic-loader".into())
        .spawn(move || {
            for (id, name, asset_path) in defs {
                let bytes = match crate::asset_path::get(&asset_path) {
                    Some(file) => file.data.to_vec(),
                    None => {
                        log::warn!("relic icon not found in embedded assets: {asset_path}");
                        continue;
                    }
                };
                let img = match image::load_from_memory(&bytes) {
                    Ok(img) => img.into_rgba8(),
                    Err(e) => {
                        log::warn!("failed to decode relic icon {asset_path}: {e}");
                        continue;
                    }
                };
                let (w, h) = img.dimensions();
                let msg = DecodedRelicImage {
                    id,
                    name,
                    rgba: img.into_raw(),
                    width: w,
                    height: h,
                };
                if tx.send(msg).is_err() {
                    break; // receiver dropped, renderer shut down
                }
            }
            log::info!("relic-loader thread finished");
        })
        .expect("failed to spawn relic-loader thread");

    rx
}

/// Spawn a background thread that decodes all background PNGs and sends the RGBA
/// data back over a channel.
fn spawn_background_loader() -> mpsc::Receiver<DecodedBackgroundImage> {
    let (tx, rx) = mpsc::channel();

    let backgrounds: Vec<(BackgroundId, &'static str)> = [
        BackgroundId::Menu,
        BackgroundId::Gameplay,
        BackgroundId::Score,
    ]
    .iter()
    .filter_map(|id| id.asset_path().map(|p| (*id, p)))
    .collect();

    std::thread::Builder::new()
        .name("bg-loader".into())
        .spawn(move || {
            for (id, asset_path) in backgrounds {
                let bytes = match crate::asset_path::get(asset_path) {
                    Some(file) => file.data.to_vec(),
                    None => {
                        log::warn!("background image not found: {asset_path}");
                        continue;
                    }
                };
                let img = match image::load_from_memory(&bytes) {
                    Ok(img) => img.into_rgba8(),
                    Err(e) => {
                        log::warn!("failed to decode background {asset_path}: {e}");
                        continue;
                    }
                };
                let (w, h) = img.dimensions();
                let msg = DecodedBackgroundImage {
                    id,
                    rgba: img.into_raw(),
                    width: w,
                    height: h,
                };
                if tx.send(msg).is_err() {
                    break;
                }
            }
            log::info!("bg-loader thread finished");
        })
        .expect("failed to spawn bg-loader thread");

    rx
}

fn create_depth(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

// ---------------------------------------------------------------------------
// Per-tile GPU resource builder (free function avoids double-borrow of `self`)
// ---------------------------------------------------------------------------

fn make_hand_tile_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    albedo_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    base_color_factor: [f32; 4],
    tile: &Tile,
) -> HandTileGpu {
    let identity = Mat4::IDENTITY;
    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("hand-tile-cam"),
        contents: bytemuck::bytes_of(&CameraUniform {
            view_proj: identity.to_cols_array(),
            model: identity.to_cols_array(),
            base_color_factor,
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    const DECAL_SIZE: u32 = 256;
    let rgba = rasterize_tile_decal(tile, DECAL_SIZE);
    let (decal_texture, decal_view) = upload_rgba_texture(
        device,
        queue,
        "hand-tile-decal",
        &rgba,
        DECAL_SIZE,
        DECAL_SIZE,
    );

    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("hand-tile-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(albedo_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&decal_view),
            },
        ],
    });

    let symbol = tile_short_label(tile);
    let suit_emoji = tile_suit_emoji(tile).to_string();
    let suit_color = tile.suit_color();
    HandTileGpu {
        uniform_buffer,
        bind_group,
        tile_id: (tile.suit, tile.rank),
        symbol,
        suit_emoji,
        suit_color,
        decal_texture,
    }
}

// ---------------------------------------------------------------------------
// WgpuRenderer impl
// ---------------------------------------------------------------------------

impl WgpuRenderer {
    pub fn new(window: Arc<Window>) -> anyhow::Result<Self> {
        let t_total = Instant::now();
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window)?;

        let t0 = Instant::now();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .map_err(|e| anyhow::anyhow!("adapter: {e:?}"))?;
        log::info!("wgpu adapter acquired in {:?}", t0.elapsed());

        let caps = surface.get_capabilities(&adapter);
        let format = caps
            .formats
            .iter()
            .find(|f| f.is_srgb())
            .copied()
            .unwrap_or(caps.formats[0]);

        let mut limits =
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());

        // Upgrade compute/storage limits from the adapter so the fluid simulation
        // can use compute shaders on native targets.  The base webgl2 defaults set
        // these to 0 and using_resolution() doesn't touch them.
        let al = adapter.limits();
        limits.max_compute_workgroups_per_dimension = al.max_compute_workgroups_per_dimension;
        limits.max_compute_workgroup_size_x = al.max_compute_workgroup_size_x;
        limits.max_compute_workgroup_size_y = al.max_compute_workgroup_size_y;
        limits.max_compute_workgroup_size_z = al.max_compute_workgroup_size_z;
        limits.max_compute_invocations_per_workgroup = al.max_compute_invocations_per_workgroup;
        limits.max_storage_buffers_per_shader_stage = al.max_storage_buffers_per_shader_stage;
        limits.max_storage_textures_per_shader_stage = al.max_storage_textures_per_shader_stage;
        limits.max_storage_buffer_binding_size = al.max_storage_buffer_binding_size;

        let t0 = Instant::now();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("mahjuro-device"),
            required_features: wgpu::Features::empty(),
            required_limits: limits,
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::default(),
        }))
        .map_err(|e| anyhow::anyhow!("device: {e:?}"))?;
        log::info!("wgpu device created in {:?}", t0.elapsed());

        let mut config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .ok_or_else(|| anyhow::anyhow!("no default surface config"))?;
        config.format = format;
        config.present_mode = wgpu::PresentMode::Fifo;
        config.desired_maximum_frame_latency = 2;
        surface.configure(&device, &config);

        let (depth_texture, depth_view) =
            create_depth(&device, size.width.max(1), size.height.max(1));

        let t0 = Instant::now();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quad-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/quad.wgsl").into()),
        });

        let tile_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tile-3d-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/tile_3d.wgsl").into()),
        });

        let text_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("text-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/text_quad.wgsl").into()),
        });

        let globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globals"),
            contents: bytemuck::bytes_of(&Globals {
                screen: [size.width as f32, size.height as f32],
                time: 0.0,
                _pad: 0.0,
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
                ],
            });

        let loaded_glb = match crate::asset_path::get("Tile.glb") {
            Some(file) => load_glb_tile_from_bytes(&file.data),
            None => Err(anyhow::anyhow!("Tile.glb not found in embedded assets")),
        };

        let tile_base_color_factor = loaded_glb
            .as_ref()
            .ok()
            .and_then(|t| t.primitives.first())
            .map(|p| p.base_color_factor)
            .unwrap_or([1.0, 1.0, 1.0, 1.0]);

        let (tile_albedo_texture, tile_albedo_view) = match &loaded_glb {
            Ok(t)
                if t.primitives
                    .first()
                    .map(|p| p.albedo_rgba.is_some())
                    .unwrap_or(false) =>
            {
                let (rgba, w, h) = t.primitives[0].albedo_rgba.as_ref().unwrap();
                upload_rgba_texture(&device, &queue, "tile-albedo", rgba, *w, *h)
            }
            _ => white_albedo(&device, &queue),
        };

        let tile_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tile-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let quad_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quad-pl"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });

        let tile_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tile-pl"),
            bind_group_layouts: &[Some(&tile_material_layout)],
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

        // Tile quad pipeline — SDF rounded rect with ivory/bamboo look.
        let tile_quad_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tile_quad.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/shaders/tile_quad.wgsl"
                ))
                .into(),
            ),
        });

        let tile_quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tile-quad-pipeline"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &tile_quad_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout.clone(), instance_layout.clone()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &tile_quad_shader,
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

        // Light beam pipeline — volumetric directional light with procedural dust.
        let light_beam_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("light_beam.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/shaders/light_beam.wgsl"
                ))
                .into(),
            ),
        });

        let light_beam_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("light-beam-pipeline"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &light_beam_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout.clone(), instance_layout.clone()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &light_beam_shader,
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
            ],
        };

        let tile_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tile-pipeline"),
            layout: Some(&tile_layout),
            vertex: wgpu::VertexState {
                module: &tile_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[tile_vertex_layout],
            },
            fragment: Some(wgpu::FragmentState {
                module: &tile_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
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
            depth_stencil: Some(depth_3d),
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

        let text_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("text-pl"),
            bind_group_layouts: &[Some(&globals_layout), Some(&text_bind_group_layout)],
            immediate_size: 0,
        });
        let text_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("text-pipeline"),
            layout: Some(&text_layout),
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

        // ---- Image pipeline (full-colour textured quads for relic icons, etc.) ----
        let image_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("image-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/image_quad.wgsl").into()),
        });
        let image_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("image-pipeline"),
            layout: Some(&text_layout),
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

        let t0 = Instant::now();
        let tile_mesh = match loaded_glb {
            Ok(mut mesh) => {
                normalize_mesh(&mut mesh);
                let prim = &mesh.primitives[0];
                log::info!(
                    "Loaded 3D tile: {} vertices, {} indices, base color tex: {}",
                    prim.vertices.len(),
                    prim.indices.len(),
                    prim.albedo_rgba.is_some()
                );
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
                Some(TileMeshGpu {
                    vertex_buffer: vb,
                    index_buffer: ib,
                    index_count: prim.indices.len() as u32,
                })
            }
            Err(e) => {
                log::warn!("Could not load Tile.glb (3D hand tiles disabled): {e:#}");
                None
            }
        };

        log::info!("tile mesh loaded in {:?}", t0.elapsed());

        // Kick off background relic image loading (non-blocking).
        let relic_rx = Some(spawn_relic_loader());
        // Kick off background image loading (non-blocking).
        let background_rx = Some(spawn_background_loader());

        // Create fluid simulation (requires compute shader support).
        let fluid = {
            let limits = device.limits();
            if limits.max_compute_workgroups_per_dimension > 0 {
                log::info!("Compute shaders supported — creating fluid simulation.");
                Some(super::fluid::FluidSim::new(
                    &device,
                    &queue,
                    &globals_layout,
                    format,
                    size.width as f32,
                    size.height as f32,
                ))
            } else {
                log::warn!("Compute shaders not supported — smoke effects disabled.");
                None
            }
        };

        log::info!("WgpuRenderer::new() total: {:?}", t_total.elapsed());

        Ok(Self {
            surface,
            device,
            queue,
            config,
            depth_texture,
            depth_view,
            quad_pipeline,
            tile_quad_pipeline,
            light_beam_pipeline,
            tile_pipeline,
            globals_buffer,
            globals_bind_group,
            tile_material_layout,
            tile_sampler,
            tile_albedo_texture,
            tile_albedo_view,
            tile_base_color_factor,
            hand_tiles: Vec::new(),
            vertex_buffer,
            index_buffer,
            tile_mesh,
            text_pipeline,
            text_bind_group_layout,
            image_pipeline,
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
            prev_hand_slots: Vec::new(),
            last_frame: Instant::now(),
            creation_time: Instant::now(),
            relic_textures: HashMap::new(),
            relic_rx,
            background_textures: HashMap::new(),
            background_rx,
            fluid,
        })
    }

    #[allow(dead_code)]
    pub fn has_tile_mesh(&self) -> bool {
        self.tile_mesh.is_some()
    }

    /// Returns `true` while background asset loading (relic/background textures)
    /// is still in progress.
    pub fn is_loading(&self) -> bool {
        self.relic_rx.is_some() || self.background_rx.is_some()
    }

    /// Drain any decoded relic images from the background loader and upload them
    /// to the GPU.  Called once per frame; a no-op once all images are loaded.
    fn poll_relic_textures(&mut self) {
        let Some(ref rx) = self.relic_rx else { return };
        let mut finished = false;
        // Non-blocking drain: upload every image that's ready this frame.
        loop {
            match rx.try_recv() {
                Ok(img) => {
                    let (tex, view) = upload_rgba_texture(
                        &self.device,
                        &self.queue,
                        img.name,
                        &img.rgba,
                        img.width,
                        img.height,
                    );
                    let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some(img.name),
                        layout: &self.text_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.tile_sampler),
                            },
                        ],
                    });
                    self.relic_textures.insert(img.id, RelicTextureGpu { texture: tex, bind_group });
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        if finished {
            log::info!("all {} relic textures uploaded to GPU", self.relic_textures.len());
            self.relic_rx = None; // drop the channel
        }
    }

    /// Drain any decoded background images from the loader and upload to GPU.
    fn poll_background_textures(&mut self) {
        let Some(ref rx) = self.background_rx else { return };
        let mut finished = false;
        loop {
            match rx.try_recv() {
                Ok(img) => {
                    let label = format!("bg-{:?}", img.id);
                    let (tex, view) = upload_rgba_texture(
                        &self.device,
                        &self.queue,
                        &label,
                        &img.rgba,
                        img.width,
                        img.height,
                    );
                    let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some(&label),
                        layout: &self.text_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.tile_sampler),
                            },
                        ],
                    });
                    self.background_textures.insert(img.id, BackgroundTextureGpu { texture: tex, bind_group });
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        if finished {
            log::info!("all {} background textures uploaded to GPU", self.background_textures.len());
            self.background_rx = None;
        }
    }

    /// Returns true while any tile animation (spin or lift lerp) is still running.
    pub fn is_spinning(&self) -> bool {
        const SPIN_SECS: f32 = 0.4;
        let spin_active = if let Some((_slot, start)) = self.focus_spin {
            start.elapsed().as_secs_f32() < SPIN_SECS
        } else {
            false
        };
        // Also keep animating while any tile's focus_t hasn't settled.
        let lerp_active = self.focus_t.iter().enumerate().any(|(i, &ft)| {
            let target = if i == self.last_focus { 1.0 } else { 0.0 };
            (ft - target).abs() > 0.001
        });
        // Keep animating while any tile is sliding into position.
        let slide_active = self.tile_anim_y.iter().any(|&y| y.abs() > 0.5)
            || self.tile_anim_x.iter().any(|&x| x.abs() > 0.01);
        let departing_active = !self.departing_tiles.is_empty();
        spin_active || lerp_active || slide_active || departing_active || !self.hand_tiles.is_empty()
    }

    /// Ensure `hand_tiles` matches `tiles`.
    ///
    /// Only re-rasterises decals for slots whose tile identity (suit + rank)
    /// has changed, so unchanged tiles keep their GPU textures.
    pub fn update_hand_tiles(&mut self, tiles: &[Tile]) {
        // Build old uid → slot index map before we modify anything.
        let old_uid_to_slot: std::collections::HashMap<u32, usize> = self
            .tile_uids
            .iter()
            .enumerate()
            .filter(|&(_, &uid)| uid != u32::MAX)
            .map(|(i, &uid)| (uid, i))
            .collect();

        self.hand_tiles.truncate(tiles.len());
        self.focus_t.resize(tiles.len(), 0.0);
        self.tile_anim_y.resize(tiles.len(), 0.0);
        self.tile_anim_x.resize(tiles.len(), 0.0);
        self.tile_uids.resize(tiles.len(), u32::MAX);

        // Count truly new tiles (not previously in hand) for staggered draw animation.
        let mut new_tile_order: usize = 0;

        for (i, tile) in tiles.iter().enumerate() {
            let id = (tile.suit, tile.rank);
            let uid = tile.id;
            let is_new = self.tile_uids[i] != uid;
            self.tile_uids[i] = uid;

            if !is_new {
                // Tile at this slot hasn't changed at all.
                if self
                    .hand_tiles
                    .get(i)
                    .map(|d| d.tile_id == id)
                    .unwrap_or(false)
                {
                    continue;
                }
            }

            if is_new {
                if let Some(&old_slot) = old_uid_to_slot.get(&uid) {
                    // Tile existed before but moved slots (sort). Animate horizontally.
                    let slot_offset = (old_slot as f32) - (i as f32);
                    self.tile_anim_x[i] = slot_offset;
                    // Don't set Y animation — it's not a new tile, just repositioned.
                } else {
                    // Truly new tile (drawn from wall). Stagger the Y offset.
                    let stagger = new_tile_order as f32 * 30.0;
                    self.tile_anim_y[i] = 120.0 + stagger;
                    new_tile_order += 1;
                }
            }
            let htg = make_hand_tile_gpu(
                &self.device,
                &self.queue,
                &self.tile_material_layout,
                &self.tile_albedo_view,
                &self.tile_sampler,
                self.tile_base_color_factor,
                tile,
            );
            if i < self.hand_tiles.len() {
                self.hand_tiles[i] = htg;
            } else {
                self.hand_tiles.push(htg);
            }
        }
    }

    /// Spawn skip-off-table departure animations for the given tile slot indices.
    /// Call this *before* `update_hand_tiles` removes the tiles so we can capture
    /// their visual data. Uses `prev_hand_slots` for screen positions.
    /// `depart_lifetime` — base lifetime in seconds for the departure animation.
    pub fn depart_tiles(&mut self, indices: &[usize], depart_lifetime: f32) {
        use std::f32::consts::PI;
        let mut rng_seed = self.creation_time.elapsed().as_nanos() as u32;
        let cheap_rand = |seed: &mut u32| -> f32 {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 17;
            *seed ^= *seed << 5;
            (*seed as f32) / u32::MAX as f32
        };

        for (order, &idx) in indices.iter().enumerate() {
            let Some(htg) = self.hand_tiles.get(idx) else { continue };
            let Some(&(sx, sy, sw, sh)) = self.prev_hand_slots.get(idx) else { continue };

            // Compute the tile rect (matching render logic).
            let tile_w = sw * 0.85;
            let tile_h = tile_w * 1.33;
            let tx = sx + (sw - tile_w) * 0.5;
            let ty = sy + (sh - tile_h) * 0.5;

            // Random direction biased upward and outward — "skip off the table".
            let angle = -PI * 0.25 - cheap_rand(&mut rng_seed) * PI * 0.5; // -45° to -135° (upward arc)
            let speed = 250.0 + cheap_rand(&mut rng_seed) * 200.0;
            let vx = angle.cos() * speed;
            let vy = angle.sin() * speed;

            self.departing_tiles.push(DepartingTile {
                symbol: htg.symbol.clone(),
                suit_emoji: htg.suit_emoji.clone(),
                suit_color: htg.suit_color,
                start_rect: [tx, ty, tile_w, tile_h],
                velocity: (vx, vy),
                spin_speed: (2.0 + cheap_rand(&mut rng_seed) * 4.0) * if cheap_rand(&mut rng_seed) > 0.5 { 1.0 } else { -1.0 },
                elapsed: -(order as f32) * 0.06, // stagger departures slightly
                lifetime: depart_lifetime + cheap_rand(&mut rng_seed) * 0.3,
            });
        }
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);

        self.depth_texture.destroy();
        let (dt, dv) = create_depth(&self.device, new_size.width, new_size.height);
        self.depth_texture = dt;
        self.depth_view = dv;

        self.queue.write_buffer(
            &self.globals_buffer,
            0,
            bytemuck::bytes_of(&Globals {
                screen: [new_size.width as f32, new_size.height as f32],
                time: self.creation_time.elapsed().as_secs_f32(),
                _pad: 0.0,
            }),
        );

        if let Some(ref mut fluid) = self.fluid {
            fluid.update_screen_size(new_size.width as f32, new_size.height as f32);
        }
    }

    /// Render one frame.
    ///
    /// * `instances`    — 2D UI quads (score panel, modifier strip, relic choices …).
    /// * `hand_slots`   — screen-space `(x, y, w, h)` rects for each hand tile;
    ///                    must match the length passed to `update_hand_tiles`.
    /// * `focus`        — index of the focused hand tile (gets a scale + lift boost).
    /// * `text_labels`  — text overlaid on top of UI quads (score, relics, etc.).
    /// * `relic_icons`  — relic image quads drawn via pre-loaded textures.
    /// * `tooltip_layers`  — per-tooltip rendering data; each layer's quads and
    ///                       text are drawn before the next so children occlude parents.
    /// * `overlay_split` — if `Some((inst_start, label_start))`, instances and
    ///   text_labels are split into scene content (before the index) and overlay
    ///   content (from the index onward). Overlay quads and text are drawn *after*
    ///   tile symbols so they correctly occlude hand tiles.
    pub fn render(
        &mut self,
        instances: &[GpuInstance],
        hand_slots: &[(f32, f32, f32, f32)],
        focus: usize,
        selected: &[bool],
        text_labels: &[TextLabel],
        relic_icons: &[RelicIcon],
        tooltip_layers: &[TooltipLayer],
        background: BackgroundId,
        smoke_intensity: crate::persistence::SmokeIntensity,
        hint_indices: &[usize],
        overlay_split: Option<(usize, usize)>,
        draw_settle_speed: f32,
        sort_settle_speed: f32,
    ) -> anyhow::Result<()> {
        // Upload any relic/background textures that finished decoding.
        self.poll_relic_textures();
        self.poll_background_textures();

        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.surface.configure(&self.device, &self.config);
                return Ok(());
            }
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Validation => {
                return Ok(());
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let _w = self.size.width.max(1) as f32;
        let _h = self.size.height.max(1) as f32;

        // Detect focus changes and start a 360° CW spin for the newly focused tile.
        if focus != self.last_focus {
            self.focus_spin = Some((focus, Instant::now()));
            self.last_focus = focus;
        }

        // Lerp per-tile slide animations toward 0 (ease-out).
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32().min(0.05);
        self.last_frame = now;
        let slide_speed = draw_settle_speed; // higher = faster settle
        for y in self.tile_anim_y.iter_mut() {
            *y *= (-slide_speed * dt).exp(); // exponential ease-out
            if y.abs() < 0.5 {
                *y = 0.0;
            }
        }
        let slide_speed_x = sort_settle_speed; // horizontal settle for sort/drag
        for x in self.tile_anim_x.iter_mut() {
            *x *= (-slide_speed_x * dt).exp();
            if x.abs() < 0.01 {
                *x = 0.0;
            }
        }

        // Update globals with current time for animated shaders.
        self.queue.write_buffer(
            &self.globals_buffer,
            0,
            bytemuck::bytes_of(&Globals {
                screen: [self.size.width as f32, self.size.height as f32],
                time: self.creation_time.elapsed().as_secs_f32(),
                _pad: 0.0,
            }),
        );

        // Update departing tiles (skip-off-table animation).
        for tile in self.departing_tiles.iter_mut() {
            tile.elapsed += dt;
            // Gravity pulls tiles downward as they skip off.
            tile.velocity.1 += 600.0 * dt;
        }
        self.departing_tiles.retain(|t| t.elapsed < t.lifetime);

        // Save hand slots for next frame's departure animations.
        self.prev_hand_slots = hand_slots.to_vec();

        // Build 2D tile quads and text labels for hand tiles.
        let mut tile_quads: Vec<GpuInstance> = Vec::new();
        let mut tile_labels: Vec<TextLabel> = Vec::new(); // main labels (UI font)
        let mut emoji_labels: Vec<TextLabel> = Vec::new(); // suit emoji (Noto Emoji)
        {
            for (i, htg) in self.hand_tiles.iter().enumerate() {
                let Some(&(sx, sy, sw, sh)) = hand_slots.get(i) else {
                    continue;
                };
                let is_focused = i == focus;
                let is_selected = selected.get(i).copied().unwrap_or(false);
                // Add slide-in animation offsets.
                let slide_y = self.tile_anim_y.get(i).copied().unwrap_or(0.0);
                let slide_x_slots = self.tile_anim_x.get(i).copied().unwrap_or(0.0);
                // Tile aspect ratio roughly 3:4 (width:height).
                let tile_w = sw * 0.85;
                let tile_h = tile_w * 1.33;
                let tx = sx + (sw - tile_w) * 0.5 + slide_x_slots * sw;
                let ty = sy + (sh - tile_h) * 0.5 + slide_y;

                // Active/selected: golden border behind the tile.
                if is_selected {
                    let pad = tile_w * 0.08;
                    tile_quads.push(GpuInstance {
                        rect: [tx - pad, ty - pad, tile_w + pad * 2.0, tile_h + pad * 2.0],
                        color: [0.9, 0.75, 0.2, 1.0],
                    });
                }

                // Tile quad — encode directional light hint via color when active.
                let tile_color = if hint_indices.contains(&i) {
                    let t = self.creation_time.elapsed().as_secs_f32();
                    let pulse = (t * 1.5).sin() * 0.15 + 0.55;
                    [0.3, 0.9, 0.4, pulse]
                } else {
                    [0.0, 0.0, 0.0, 1.0]
                };
                tile_quads.push(GpuInstance {
                    rect: [tx, ty, tile_w, tile_h],
                    color: tile_color,
                });

                // Hover: bobbing red arrow above focused tile.
                if is_focused {
                    let bob_period = 1.5_f32; // seconds per full cycle
                    let bob_amp = tile_h * 0.08;
                    let bob_y = (self.creation_time.elapsed().as_secs_f32() / bob_period * std::f32::consts::TAU).sin() * bob_amp;
                    let arrow_h = tile_h * 0.32;
                    let arrow_w = tile_w * 0.65;
                    let arrow_x = tx + (tile_w - arrow_w) * 0.5;
                    let arrow_y = ty - arrow_h - tile_h * 0.05 + bob_y;
                    tile_labels.push(TextLabel {
                        rect: [arrow_x, arrow_y, arrow_w, arrow_h],
                        text: "▼".to_string(),
                        color: [0.85, 0.1, 0.1, 1.0],
                    });
                }

                // Main label (number/name) in the upper portion, suit-colored.
                let inset_x = tile_w * 0.10;
                let top_h = tile_h * 0.50;
                tile_labels.push(TextLabel {
                    rect: [
                        tx + inset_x,
                        ty + tile_h * 0.05,
                        tile_w - inset_x * 2.0,
                        top_h,
                    ],
                    text: htg.symbol.clone(),
                    color: htg.suit_color,
                });

                // Emoji suit indicator in the lower portion.
                let bot_h = tile_h * 0.40;
                emoji_labels.push(TextLabel {
                    rect: [
                        tx + inset_x,
                        ty + tile_h * 0.55,
                        tile_w - inset_x * 2.0,
                        bot_h,
                    ],
                    text: htg.suit_emoji.clone(),
                    color: htg.suit_color,
                });
            }
        }

        // Build light beam instances for hinted tiles.
        let mut light_beams: Vec<GpuInstance> = Vec::new();
        for &idx in hint_indices {
            let Some(&(sx, sy, sw, sh)) = hand_slots.get(idx) else { continue };
            let slide_y = self.tile_anim_y.get(idx).copied().unwrap_or(0.0);
            let slide_x_slots = self.tile_anim_x.get(idx).copied().unwrap_or(0.0);
            let tile_w = sw * 0.85;
            let tile_h = tile_w * 1.33;
            let tx = sx + (sw - tile_w) * 0.5 + slide_x_slots * sw;
            let ty = sy + (sh - tile_h) * 0.5 + slide_y;
            // Light beam quad: narrow cone extending upward from the tile.
            let beam_w = tile_w * 1.4;
            let beam_h = tile_h * 2.8;
            let bx = tx + tile_w * 0.5 - beam_w * 0.5;
            let by = ty + tile_h * 0.5 - beam_h * 0.55; // offset upward
            light_beams.push(GpuInstance {
                rect: [bx, by, beam_w, beam_h],
                color: [0.3, 0.85, 0.4, 0.7],
            });
        }

        // Render departing tiles (skip-off-table animation).
        for dep in &self.departing_tiles {
            let t = dep.elapsed;
            let dx = dep.velocity.0 * t;
            let dy = dep.velocity.1 * t + 300.0 * t * t; // parabolic arc (gravity)
            let x = dep.start_rect[0] + dx;
            let y = dep.start_rect[1] + dy;
            let w = dep.start_rect[2];
            let h = dep.start_rect[3];

            // Fade out over lifetime.
            let alpha = (1.0 - (t / dep.lifetime)).max(0.0);
            // Shrink slightly as it departs.
            let shrink = 1.0 - 0.3 * (t / dep.lifetime);
            let sw = w * shrink;
            let sh = h * shrink;
            let sx = x + (w - sw) * 0.5;
            let sy = y + (h - sh) * 0.5;

            // Tile background.
            tile_quads.push(GpuInstance {
                rect: [sx, sy, sw, sh],
                color: [0.0, 0.0, 0.0, alpha],
            });

            // Main label.
            let inset_x = sw * 0.10;
            let top_h = sh * 0.50;
            tile_labels.push(TextLabel {
                rect: [sx + inset_x, sy + sh * 0.05, sw - inset_x * 2.0, top_h],
                text: dep.symbol.clone(),
                color: [dep.suit_color[0], dep.suit_color[1], dep.suit_color[2], alpha],
            });

            // Suit emoji.
            let bot_h = sh * 0.40;
            emoji_labels.push(TextLabel {
                rect: [sx + inset_x, sy + sh * 0.55, sw - inset_x * 2.0, bot_h],
                text: dep.suit_emoji.clone(),
                color: [dep.suit_color[0], dep.suit_color[1], dep.suit_color[2], alpha],
            });
        }

        // Tile quad instance buffer (separate from scene instances — uses tile_quad_pipeline).
        let tile_instance_buffer = if tile_quads.is_empty() {
            None
        } else {
            Some(
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("tile-instances"),
                        contents: bytemuck::cast_slice(&tile_quads),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
            )
        };

        // Light beam instance buffer (rendered behind tiles).
        let light_beam_buffer = if light_beams.is_empty() {
            None
        } else {
            Some(
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("light-beams"),
                        contents: bytemuck::cast_slice(&light_beams),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
            )
        };

        // Split instances into scene quads and overlay quads.
        let (scene_instances, overlay_instances) = match overlay_split {
            Some((inst_split, _)) => {
                let split = inst_split.min(instances.len());
                (&instances[..split], &instances[split..])
            }
            None => (instances, &[][..]),
        };

        // Instance buffer for 2D UI quads (scene panels, etc.).
        let instance_buffer = if scene_instances.is_empty() {
            None
        } else {
            Some(
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("instances"),
                        contents: bytemuck::cast_slice(scene_instances),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
            )
        };

        // Overlay instance buffer (drawn after tile text to occlude hand tiles).
        let overlay_instance_buffer = if overlay_instances.is_empty() {
            None
        } else {
            Some(
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("overlay-instances"),
                        contents: bytemuck::cast_slice(overlay_instances),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
            )
        };

        // Pre-rasterize text labels → GPU textures + bind groups.
        struct TextDraw {
            inst_buf: wgpu::Buffer,
            bind_group: wgpu::BindGroup,
            #[allow(dead_code)]
            _tex: wgpu::Texture,
        }
        // Rasterize tile labels with emoji font, scene labels with UI font.
        let make_text_draw = |lbl: &TextLabel, font: &fontdue::Font| -> TextDraw {
            let tw = (lbl.rect[2] as u32).max(1);
            let th = (lbl.rect[3] as u32).max(1);
            let rgba = rasterize_label(font, &lbl.text, tw, th);
            let (tex, view) =
                upload_rgba_texture(&self.device, &self.queue, "text-lbl", &rgba, tw, th);
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("text-lbl-bg"),
                layout: &self.text_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.tile_sampler),
                    },
                ],
            });
            let inst = GpuInstance {
                rect: lbl.rect,
                color: lbl.color,
            };
            let inst_buf = self
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("text-inst"),
                    contents: bytemuck::cast_slice(&[inst]),
                    usage: wgpu::BufferUsages::VERTEX,
                });
            TextDraw {
                inst_buf,
                bind_group,
                _tex: tex,
            }
        };

        // Split text labels into scene and overlay groups.
        let (scene_text_labels, overlay_text_labels) = match overlay_split {
            Some((_, label_split)) => {
                let split = label_split.min(text_labels.len());
                (&text_labels[..split], &text_labels[split..])
            }
            None => (text_labels, &[][..]),
        };

        let mut text_draws: Vec<TextDraw> = Vec::new();
        let mut overlay_text_draws: Vec<TextDraw> = Vec::new();

        // Main tile labels + scene labels: UI font.
        if let Some(ref font) = self.ui_font {
            text_draws.extend(tile_labels.iter().map(|lbl| make_text_draw(lbl, font)));
            text_draws.extend(scene_text_labels.iter().map(|lbl| make_text_draw(lbl, font)));
            overlay_text_draws.extend(overlay_text_labels.iter().map(|lbl| make_text_draw(lbl, font)));
        }

        // Emoji suit indicators: Noto Emoji font.
        if let Some(ref font) = self.emoji_font {
            text_draws.extend(emoji_labels.iter().map(|lbl| make_text_draw(lbl, font)));
        }

        // Per-tooltip-layer GPU resources (quads + text for each tooltip,
        // drawn in order so children fully occlude parents).
        struct TooltipLayerGpu {
            inst_buf: Option<wgpu::Buffer>,
            inst_count: u32,
            text_draws: Vec<TextDraw>,
        }
        let tooltip_layer_gpus: Vec<TooltipLayerGpu> = tooltip_layers
            .iter()
            .map(|layer| {
                let inst_buf = if layer.instances.is_empty() {
                    None
                } else {
                    Some(self.device.create_buffer_init(
                        &wgpu::util::BufferInitDescriptor {
                            label: Some("tooltip-layer-inst"),
                            contents: bytemuck::cast_slice(&layer.instances),
                            usage: wgpu::BufferUsages::VERTEX,
                        },
                    ))
                };
                let text_draws: Vec<TextDraw> = if let Some(ref font) = self.ui_font {
                    layer.labels.iter().map(|lbl| make_text_draw(lbl, font)).collect()
                } else {
                    vec![]
                };
                TooltipLayerGpu {
                    inst_buf,
                    inst_count: layer.instances.len() as u32,
                    text_draws,
                }
            })
            .collect();

        // Relic icon instance buffers (use pre-loaded textures, not rasterized text).
        struct RelicDraw {
            inst_buf: wgpu::Buffer,
            relic_id: RelicId,
        }
        let relic_draws: Vec<RelicDraw> = relic_icons
            .iter()
            .filter(|icon| self.relic_textures.contains_key(&icon.relic_id))
            .map(|icon| {
                let inst = GpuInstance {
                    rect: icon.rect,
                    color: [1.0, 1.0, 1.0, 1.0], // tint: white = no tint
                };
                let inst_buf = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("relic-icon-inst"),
                        contents: bytemuck::cast_slice(&[inst]),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                RelicDraw {
                    inst_buf,
                    relic_id: icon.relic_id,
                }
            })
            .collect();

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        // Run fluid simulation compute passes (before render pass).
        if let Some(ref mut fluid) = self.fluid {
            let dt = self.last_frame.elapsed().as_secs_f32().max(1.0 / 120.0);
            fluid.step(&mut encoder, &self.queue, dt, smoke_intensity);
        }

        // Pre-create fluid fullscreen quad instance buffer (must outlive render pass).
        let fluid_inst = GpuInstance {
            rect: [0.0, 0.0, self.size.width.max(1) as f32, self.size.height.max(1) as f32],
            color: [1.0, 1.0, 1.0, 1.0],
        };
        let fluid_inst_buf = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("fluid-inst"),
            contents: bytemuck::cast_slice(&[fluid_inst]),
            usage: wgpu::BufferUsages::VERTEX,
        });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.07,
                            g: 0.05,
                            b: 0.10,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });

            // 0. Draw background image (full-screen quad, if available).
            if let Some(bg_tex) = self.background_textures.get(&background) {
                let w = self.size.width.max(1) as f32;
                let h = self.size.height.max(1) as f32;
                let bg_inst = GpuInstance {
                    rect: [0.0, 0.0, w, h],
                    color: [1.0, 1.0, 1.0, 1.0],
                };
                let bg_inst_buf =
                    self.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("bg-inst"),
                            contents: bytemuck::cast_slice(&[bg_inst]),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                pass.set_pipeline(&self.image_pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.set_bind_group(1, &bg_tex.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, bg_inst_buf.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..1);
            }

            // 0b. Draw light beams behind hinted tiles.
            if let Some(ref lbb) = light_beam_buffer {
                pass.set_pipeline(&self.light_beam_pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, lbb.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..light_beams.len() as u32);
            }

            // 1a. Draw tile quads (SDF rounded rectangles).
            if let Some(ref tib) = tile_instance_buffer {
                pass.set_pipeline(&self.tile_quad_pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, tib.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..tile_quads.len() as u32);
            }

            // 1b. Draw fluid smoke overlay (between tiles and UI).
            if smoke_intensity != crate::persistence::SmokeIntensity::Off {
                if let Some(ref fluid) = self.fluid {
                    fluid.draw(
                        &mut pass,
                        &self.globals_bind_group,
                        &self.vertex_buffer,
                        &self.index_buffer,
                        &fluid_inst_buf,
                    );
                }
            }

            // 1c. Draw 2D UI quads (scene panels, etc.).
            if let Some(ref ib) = instance_buffer {
                pass.set_pipeline(&self.quad_pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, ib.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..scene_instances.len() as u32);
            }

            // 2. Draw text labels (tile symbols + scene text, alpha-blended).
            for td in &text_draws {
                pass.set_pipeline(&self.text_pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.set_bind_group(1, &td.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, td.inst_buf.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..1);
            }

            // 3. Draw relic icon images (pre-loaded textures, alpha-blended).
            for rd in &relic_draws {
                if let Some(rtex) = self.relic_textures.get(&rd.relic_id) {
                    pass.set_pipeline(&self.image_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.set_bind_group(1, &rtex.bind_group, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, rd.inst_buf.slice(..));
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..6, 0, 0..1);
                }
            }

            // 3b. Draw overlay quads (dimmer + panel — on top of tile text).
            if let Some(ref oib) = overlay_instance_buffer {
                pass.set_pipeline(&self.quad_pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, oib.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..overlay_instances.len() as u32);
            }

            // 3c. Draw overlay text labels (on top of overlay quads).
            for td in &overlay_text_draws {
                pass.set_pipeline(&self.text_pipeline);
                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                pass.set_bind_group(1, &td.bind_group, &[]);
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_vertex_buffer(1, td.inst_buf.slice(..));
                pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                pass.draw_indexed(0..6, 0, 0..1);
            }

            // 4. Draw tooltip layers — each layer's quads then text, so child
            //    tooltips fully occlude their parents (DOM-like z-ordering).
            for layer in &tooltip_layer_gpus {
                if let Some(ref tib) = layer.inst_buf {
                    pass.set_pipeline(&self.quad_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, tib.slice(..));
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..6, 0, 0..layer.inst_count);
                }
                for td in &layer.text_draws {
                    pass.set_pipeline(&self.text_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.set_bind_group(1, &td.bind_group, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, td.inst_buf.slice(..));
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..6, 0, 0..1);
                }
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Instance builders
// ---------------------------------------------------------------------------

/// Build GPU instances for the score panel and modifier strip.
///
/// Hand tiles are now 3D meshes, so this function no longer generates hand
/// slot quads.
pub fn build_instances_from_layout(
    score: (f32, f32, f32, f32),
    modifier: (f32, f32, f32, f32),
    anim_scale_score: f32,
    plays: u32,
    plays_max: u32,
    discards: u32,
    discards_max: u32,
) -> Vec<GpuInstance> {
    let (sx, sy, sw, sh) = apply_transform_rect(
        score.0,
        score.1,
        score.2,
        score.3,
        crate::render::animation::Transform2D {
            offset_x: 0.0,
            offset_y: 0.0,
            scale: anim_scale_score,
            opacity: 1.0,
        },
    );
    let mut v = vec![
        GpuInstance {
            rect: [sx, sy, sw, sh],
            color: [0.15, 0.35, 0.55, 0.95],
        },
        GpuInstance {
            rect: [modifier.0, modifier.1, modifier.2, modifier.3],
            color: [0.25, 0.22, 0.35, 0.95],
        },
    ];

    // Pip indicators — two rows on the right side of the score panel.
    // Row 1: plays (teal squares). Row 2: discards (orange squares).
    // Sizes scale with the score panel height so proportions stay constant.
    let pip = (score.3 * 0.22).clamp(8.0, 28.0);
    let gap = pip * 0.25;
    let margin = pip * 0.9;
    let row_gap = pip * 0.3;

    let total_h = pip + row_gap + pip;
    let row1_y = sy + (sh - total_h) * 0.5;
    let row2_y = row1_y + pip + row_gap;

    // Plays row (right-aligned).
    let plays_row_w = plays_max as f32 * pip + (plays_max.saturating_sub(1)) as f32 * gap;
    let plays_x0 = sx + sw - margin - plays_row_w;
    for i in 0..plays_max {
        let x = plays_x0 + i as f32 * (pip + gap);
        let filled = i < plays;
        v.push(GpuInstance {
            rect: [x, row1_y, pip, pip],
            color: if filled {
                [0.20, 0.72, 0.58, 1.0]
            } else {
                [0.10, 0.22, 0.20, 0.55]
            },
        });
    }

    // Discards row (right-aligned to same right edge).
    let disc_row_w = discards_max as f32 * pip + (discards_max.saturating_sub(1)) as f32 * gap;
    let disc_x0 = sx + sw - margin - disc_row_w;
    for i in 0..discards_max {
        let x = disc_x0 + i as f32 * (pip + gap);
        let filled = i < discards;
        v.push(GpuInstance {
            rect: [x, row2_y, pip, pip],
            color: if filled {
                [0.92, 0.55, 0.18, 1.0]
            } else {
                [0.30, 0.18, 0.08, 0.55]
            },
        });
    }

    v
}

/// Build instances for the relic-pick screen: score panel, modifier strip, and 3 choice quads.
pub fn build_instances_relic_pick(
    score: (f32, f32, f32, f32),
    modifier: (f32, f32, f32, f32),
    hand_slots: &[(f32, f32, f32, f32)],
    cursor: usize,
) -> Vec<GpuInstance> {
    let mut v = Vec::new();

    v.push(GpuInstance {
        rect: [score.0, score.1, score.2, score.3],
        color: [0.15, 0.35, 0.55, 0.95],
    });
    v.push(GpuInstance {
        rect: [modifier.0, modifier.1, modifier.2, modifier.3],
        color: [0.25, 0.22, 0.35, 0.95],
    });

    // Place the 3 choice quads centred in the hand strip, each ~4 tile-widths wide.
    let pick_indices = [1usize, 6, 11];
    for (choice_idx, &slot_idx) in pick_indices.iter().enumerate() {
        let slot = hand_slots
            .get(slot_idx)
            .or_else(|| hand_slots.get(choice_idx));
        if let Some(&(x, y, w, h)) = slot {
            let color = if choice_idx == cursor {
                [0.9, 0.75, 0.2, 1.0]
            } else {
                [0.3, 0.3, 0.5, 0.95]
            };
            v.push(GpuInstance {
                rect: [x, y, w, h],
                color,
            });
        }
    }

    v
}
