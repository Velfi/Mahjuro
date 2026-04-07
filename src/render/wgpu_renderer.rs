//! WGPU: depth-tested 3D tile meshes for the hand + 2D UI quads on top.

use std::collections::HashMap;

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use glam::Mat4;
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::core::relic::RelicId;
use crate::core::tile::{Suit, Tile};
use crate::render::animation::apply_transform_rect;
use crate::render::candle_mesh::{
    CandlePlacement, WICK_TIP_Y, build_candle_wax_mesh, build_candle_wick_mesh,
};
use crate::render::decal::{
    load_noto_emoji_font, load_ui_font, rasterize_label, rasterize_tile_face_decal,
    tile_short_label, tile_suit_emoji,
};
use crate::render::draw_cmd::{DrawCmd, UiFrame};
use crate::render::lit_mesh::{
    LitMeshGpu, LitMeshInstance, MaterialParams, create_lit_mesh_material_layout,
};
use crate::render::table_mesh::build_table_mesh;
use crate::render::tile_glb::{Vertex3dTex, load_glb_tile_from_bytes, normalize_mesh};
use crate::scenes::BackgroundId;

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

/// Maximum number of point lights uploaded each frame. Must match the array
/// length in tile_3d.wgsl.
pub const MAX_POINT_LIGHTS: usize = 8;

/// CPU-side description of a point light. Scenes push these into
/// [`crate::render::draw_cmd::UiFrame::point_lights`]; the renderer translates
/// them into [`PointLightGpu`] each frame.
#[derive(Clone, Copy, Debug)]
pub struct PointLight {
    /// World-space position of the light. The first two components match the
    /// pixel-space coordinate system used for tile model matrices (with the
    /// usual `y → -y` flip the renderer applies); `z` lets candle wicks sit in
    /// front of the table plane so 3D meshes catch the light correctly.
    pub pos: [f32; 3],
    /// Falloff radius in pixels. Outside this distance the light contributes
    /// nothing.
    pub radius: f32,
    /// Linear-space RGB tint.
    pub color: [f32; 3],
    /// Brightness multiplier. >1.0 is fine — the tile shader is unclamped.
    pub intensity: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PointLightGpu {
    /// xyz = world-space position, w = radius.
    pos: [f32; 4],
    /// rgb = colour, a = intensity.
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PointLightsBuf {
    /// `count.x` = number of active lights; rest is std140 padding.
    count: [u32; 4],
    lights: [PointLightGpu; MAX_POINT_LIGHTS],
}

impl PointLightsBuf {
    /// Build the std140 light buffer, mapping each light's pixel-space
    /// `(x, y)` onto the table-plane world (`world_x = x - w/2`,
    /// `world_z = y - h/2`). The third position component is treated as the
    /// height above the table plane (`world_y`).
    fn from_lights(src: &[PointLight], screen_w: f32, screen_h: f32) -> Self {
        let mut lights = [PointLightGpu {
            pos: [0.0; 4],
            color: [0.0; 4],
        }; MAX_POINT_LIGHTS];
        let n = src.len().min(MAX_POINT_LIGHTS);
        for (i, l) in src.iter().take(n).enumerate() {
            let wx = l.pos[0] - screen_w * 0.5;
            let wz = l.pos[1] - screen_h * 0.5;
            let wy = l.pos[2];
            lights[i] = PointLightGpu {
                pos: [wx, wy, wz, l.radius],
                color: [l.color[0], l.color[1], l.color[2], l.intensity],
            };
        }
        Self {
            count: [n as u32, 0, 0, 0],
            lights,
        }
    }
}

/// One material slot of the tile mesh — vertex/index buffers + the primitive's
/// own albedo texture.  A tile may consist of several of these (e.g. an ivory
/// face primitive and a bamboo back primitive).
#[allow(dead_code)]
struct TilePrimitiveGpu {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    albedo_texture: wgpu::Texture,
    albedo_view: wgpu::TextureView,
    base_color_factor: [f32; 4],
}

/// A relic icon to draw as a textured quad at a screen-space rect.
pub struct RelicIcon {
    /// Position in screen pixels: [x, y, w, h].
    pub rect: [f32; 4],
    /// Which relic image to display.
    pub relic_id: crate::core::relic::RelicId,
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
    /// One bind group per tile-mesh primitive.  Each binds the per-tile uniform
    /// + per-tile decal + that primitive's own albedo texture.
    bind_groups: Vec<wgpu::BindGroup>,
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
    flame_pipeline: wgpu::RenderPipeline,
    #[allow(dead_code)]
    tile_pipeline: wgpu::RenderPipeline,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    tile_material_layout: wgpu::BindGroupLayout,
    /// Per-frame point-light array uploaded to the tile pipeline (group 1).
    point_lights_buffer: wgpu::Buffer,
    point_lights_bind_group: wgpu::BindGroup,
    tile_sampler: wgpu::Sampler,
    /// Per-primitive GPU resources for the tile mesh (one entry per glTF
    /// primitive, e.g. ivory face + bamboo body).
    tile_primitives: Vec<TilePrimitiveGpu>,
    /// Identity factor used by every primitive (kept for the cam uniform).
    tile_base_color_factor: [f32; 4],
    /// Per-hand-tile GPU resources; kept in sync with the hand via `update_hand_tiles`.
    hand_tiles: Vec<HandTileGpu>,
    #[allow(dead_code)]
    vertex_buffer: wgpu::Buffer,
    #[allow(dead_code)]
    index_buffer: wgpu::Buffer,
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
    /// Per-tile screen-space rects after the perspective projection, captured
    /// at the end of the previous frame. Indexed by hand position. Used by
    /// the scene layer (one frame stale) so hover tooltips and other 2D HUD
    /// overlays can anchor to the actual visible tile rather than its flat
    /// layout slot.
    last_projected_hand_rects: Vec<(usize, [f32; 4])>,
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

    // ── Procedural lit meshes (candles + wood table) ────────────────────
    /// Bind-group layout shared by every lit-mesh instance.
    lit_mesh_material_layout: wgpu::BindGroupLayout,
    /// Pipeline for procedural scene props (candles, table). Shares the
    /// `point_lights_layout` (group 1) with the tile pipeline.
    lit_mesh_pipeline: wgpu::RenderPipeline,
    /// 1×1 white texture used as a placeholder albedo for procedural meshes
    /// that don't sample from a texture.
    #[allow(dead_code)]
    lit_mesh_white_tex: wgpu::Texture,
    lit_mesh_white_view: wgpu::TextureView,
    /// Shared procedural meshes.
    candle_wax_mesh: LitMeshGpu,
    candle_wick_mesh: LitMeshGpu,
    table_mesh: LitMeshGpu,
    /// Pre-allocated per-candle uniform buffers + bind groups (one per
    /// primitive). Indexed by candle slot, then 0=wax/1=wick.
    candle_instances: Vec<[LitMeshInstance; 2]>,
    /// Single uniform buffer + bind group for the gameplay-scene table.
    table_instance: LitMeshInstance,
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
    primitives: &[TilePrimitiveGpu],
    sampler: &wgpu::Sampler,
    base_color_factor: [f32; 4],
    ui_font: Option<&fontdue::Font>,
    emoji_font: Option<&fontdue::Font>,
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

    // The tile face is 0.734 wide × 1.0 tall in local coords (see tile_3d.wgsl
    // — local Z is the short on-screen-horizontal axis, local X is the long
    // on-screen-vertical axis). Match that aspect in the texture so the GPU
    // stretching doesn't distort the rasterised glyphs.
    const DECAL_W: u32 = 192;
    const DECAL_H: u32 = 256;
    let rgba = rasterize_tile_face_decal(tile, ui_font, emoji_font, DECAL_W, DECAL_H);
    let (decal_texture, decal_view) =
        upload_rgba_texture(device, queue, "hand-tile-decal", &rgba, DECAL_W, DECAL_H);

    let bind_groups: Vec<wgpu::BindGroup> = primitives
        .iter()
        .map(|prim| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("hand-tile-bg"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&prim.albedo_view),
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
            })
        })
        .collect();

    let symbol = tile_short_label(tile);
    let suit_emoji = tile_suit_emoji(tile).to_string();
    let suit_color = tile.suit_color();
    HandTileGpu {
        uniform_buffer,
        bind_groups,
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

        // Point-light uniform buffer + bind group (group 1 of the tile pipeline).
        // Initialised empty; populated each frame from `frame.point_lights`.
        let point_lights_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("point-lights"),
            contents: bytemuck::bytes_of(&PointLightsBuf::from_lights(&[], 1.0, 1.0)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let point_lights_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("point-lights-layout"),
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
        let point_lights_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point-lights-bg"),
            layout: &point_lights_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: point_lights_buffer.as_entire_binding(),
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

        let tile_base_color_factor = [1.0_f32, 1.0, 1.0, 1.0];

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
            bind_group_layouts: &[Some(&tile_material_layout), Some(&point_lights_layout)],
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

        // Flame pipeline — additive procedural fire on a quad. Reuses
        // quad_layout (only needs globals.time) and shares the unit-quad
        // vertex/instance buffers with quad_pipeline.
        let flame_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flame.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/flame.wgsl")).into(),
            ),
        });

        let flame_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("flame-pipeline"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &flame_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout.clone(), instance_layout.clone()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &flame_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
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
            depth_stencil: Some(depth_3d.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // ---- Lit-mesh pipeline (procedural candles + wood table) ----
        let lit_mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lit-mesh-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/lit_mesh.wgsl").into()),
        });
        let lit_mesh_material_layout = create_lit_mesh_material_layout(&device);
        let lit_mesh_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lit-mesh-pl"),
            bind_group_layouts: &[Some(&lit_mesh_material_layout), Some(&point_lights_layout)],
            immediate_size: 0,
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
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &lit_mesh_shader,
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
            depth_stencil: Some(depth_3d.clone()),
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
        let tile_primitives: Vec<TilePrimitiveGpu> = match loaded_glb {
            Ok(mut mesh) => {
                normalize_mesh(&mut mesh);
                log::info!("Loaded 3D tile: {} primitive(s)", mesh.primitives.len());
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
                    let (albedo_texture, albedo_view) = match &prim.albedo_rgba {
                        Some((rgba, w, h)) => {
                            upload_rgba_texture(&device, &queue, "tile-prim-albedo", rgba, *w, *h)
                        }
                        None => white_albedo(&device, &queue),
                    };
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
                        albedo_texture,
                        albedo_view,
                        base_color_factor: prim.base_color_factor,
                    });
                }
                out
            }
            Err(e) => {
                log::warn!("Could not load Tile.glb (3D hand tiles disabled): {e:#}");
                Vec::new()
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

        // ---- Lit-mesh procedural geometry (candles + table) ----
        let candle_wax_mesh = LitMeshGpu::new(&device, &build_candle_wax_mesh(), "candle-wax");
        let candle_wick_mesh = LitMeshGpu::new(&device, &build_candle_wick_mesh(), "candle-wick");
        let table_mesh = LitMeshGpu::new(&device, &build_table_mesh(), "table");

        // Shared 1×1 white texture for procedural meshes that don't sample.
        let (lit_mesh_white_tex, lit_mesh_white_view) = white_albedo(&device, &queue);

        // Pre-allocate four candle slots (matches the gameplay layout's
        // four ambient candles). Each slot owns two instances: wax + wick.
        const NUM_CANDLE_SLOTS: usize = 4;
        let mut candle_instances: Vec<[LitMeshInstance; 2]> = Vec::with_capacity(NUM_CANDLE_SLOTS);
        for _ in 0..NUM_CANDLE_SLOTS {
            candle_instances.push([
                LitMeshInstance::new(
                    &device,
                    &lit_mesh_material_layout,
                    &lit_mesh_white_view,
                    &tile_sampler,
                ),
                LitMeshInstance::new(
                    &device,
                    &lit_mesh_material_layout,
                    &lit_mesh_white_view,
                    &tile_sampler,
                ),
            ]);
        }
        let table_instance = LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &lit_mesh_white_view,
            &tile_sampler,
        );

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
            flame_pipeline,
            tile_pipeline,
            globals_buffer,
            globals_bind_group,
            tile_material_layout,
            point_lights_buffer,
            point_lights_bind_group,
            tile_sampler,
            tile_primitives,
            tile_base_color_factor,
            hand_tiles: Vec::new(),
            vertex_buffer,
            index_buffer,
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
            last_projected_hand_rects: Vec::new(),
            last_frame: Instant::now(),
            creation_time: Instant::now(),
            relic_textures: HashMap::new(),
            relic_rx,
            background_textures: HashMap::new(),
            background_rx,
            fluid,
            lit_mesh_material_layout,
            lit_mesh_pipeline,
            lit_mesh_white_tex,
            lit_mesh_white_view,
            candle_wax_mesh,
            candle_wick_mesh,
            table_mesh,
            candle_instances,
            table_instance,
        })
    }

    #[allow(dead_code)]
    pub fn has_tile_mesh(&self) -> bool {
        !self.tile_primitives.is_empty()
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
                    self.relic_textures.insert(
                        img.id,
                        RelicTextureGpu {
                            texture: tex,
                            bind_group,
                        },
                    );
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        if finished {
            log::info!(
                "all {} relic textures uploaded to GPU",
                self.relic_textures.len()
            );
            self.relic_rx = None; // drop the channel
        }
    }

    /// Drain any decoded background images from the loader and upload to GPU.
    fn poll_background_textures(&mut self) {
        let Some(ref rx) = self.background_rx else {
            return;
        };
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
                    self.background_textures.insert(
                        img.id,
                        BackgroundTextureGpu {
                            texture: tex,
                            bind_group,
                        },
                    );
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        if finished {
            log::info!(
                "all {} background textures uploaded to GPU",
                self.background_textures.len()
            );
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
        spin_active
            || lerp_active
            || slide_active
            || departing_active
            || !self.hand_tiles.is_empty()
    }

    /// Per-hand-tile screen-space rects after the perspective projection,
    /// captured at the end of the previous frame. Indexed by hand position.
    /// Empty before the first frame is drawn — callers should fall back to
    /// the flat layout slot rects in that case.
    pub fn projected_hand_rects(&self) -> &[(usize, [f32; 4])] {
        &self.last_projected_hand_rects
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
                &self.tile_primitives,
                &self.tile_sampler,
                self.tile_base_color_factor,
                self.ui_font.as_ref(),
                self.emoji_font.as_ref(),
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
            let Some(htg) = self.hand_tiles.get(idx) else {
                continue;
            };
            let Some(&(sx, sy, sw, sh)) = self.prev_hand_slots.get(idx) else {
                continue;
            };

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
    /// `frame.cmds` is walked in order — earlier cmds render under later ones.
    /// Contiguous runs of `DrawCmd::Quad` are batched into a single instanced
    /// draw, which is invisible to scenes and preserves ordering.
    pub fn render(
        &mut self,
        frame: &UiFrame,
        smoke_intensity: crate::persistence::SmokeIntensity,
        draw_settle_speed: f32,
        sort_settle_speed: f32,
    ) -> anyhow::Result<()> {
        let hand_slots: &[(f32, f32, f32, f32)] = &frame.hand_slots;
        let focus = frame.focus;
        let selected: &[bool] = &frame.selected_tiles;
        let hint_indices: &[usize] = &frame.hint_indices;
        // Upload any relic/background textures that finished decoding.
        self.poll_relic_textures();
        self.poll_background_textures();

        let surface_frame = match self.surface.get_current_texture() {
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
        let view = surface_frame
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
        let dt = now
            .saturating_duration_since(self.last_frame)
            .as_secs_f32()
            .min(0.05);
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

        // Upload point lights for the tile shader (group 1). Scenes push
        // candle/spot lights into `frame.point_lights` in pixel-layout
        // coordinates; we map them onto the table-plane world for upload.
        let pl_w = self.size.width.max(1) as f32;
        let pl_h = self.size.height.max(1) as f32;
        self.queue.write_buffer(
            &self.point_lights_buffer,
            0,
            bytemuck::bytes_of(&PointLightsBuf::from_lights(
                &frame.point_lights,
                pl_w,
                pl_h,
            )),
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

        // Build 2D backdrop quads (selection borders, hint pulses) and text
        // labels (just the focused arrow — the symbol+emoji live in the 3D
        // tile decal now).  Per-tile model matrices for the 3D mesh draw are
        // also written here.
        let mut tile_quads: Vec<GpuInstance> = Vec::new();
        let mut tile_labels: Vec<TextLabel> = Vec::new();
        let mut emoji_labels: Vec<TextLabel> = Vec::new();
        let mut tile_3d_rects: Vec<(usize, [f32; 4])> = Vec::new();

        // ── Person-at-the-table camera ──────────────────────────────────
        // The 3D world is a horizontal table in the XZ plane (y=0). The
        // player sits in front of the table (large +Z), eyes above the
        // table, looking down and slightly forward. We map the layout's
        // pixel coordinates onto the table's surface so the existing
        // pixel-space layout still drives where things go:
        //
        //   world_x =  pixel_x - w * 0.5
        //   world_z =  pixel_y - h * 0.5     (pixel y grows downward, so
        //                                     bottom of screen → +z, near
        //                                     player; top of screen → -z,
        //                                     far edge of the table)
        //   world_y =  height above the table (0 = sitting on the wood)
        //
        // The 2D UI overlays (score panel, buttons, text) keep using the
        // pixel-orthographic quad pipeline and float over the 3D scene as
        // a HUD.
        let w = self.size.width.max(1) as f32;
        let h = self.size.height.max(1) as f32;
        let eye_height = h * 0.55;
        let eye_back = h * 0.95;
        let look_target = glam::Vec3::new(0.0, 0.0, -h * 0.10);
        let cam_pos = glam::Vec3::new(0.0, eye_height, eye_back);
        let view_mat = Mat4::look_at_rh(cam_pos, look_target, glam::Vec3::new(0.0, 1.0, 0.0));
        let fov_y = 55.0_f32.to_radians();
        let aspect = w / h;
        let proj = Mat4::perspective_rh(fov_y, aspect, 1.0, h * 12.0);
        let view_proj = proj * view_mat;
        let view_proj_arr = view_proj.to_cols_array();

        // Helper: map a layout pixel position onto the table-plane world.
        let pixel_to_world = |px: f32, py: f32, world_y: f32| -> glam::Vec3 {
            glam::Vec3::new(px - w * 0.5, world_y, py - h * 0.5)
        };
        // Helper: project a world position to integer screen pixels for use
        // in 2D overlay quads (selection halos, hint pulses, hover arrows).
        let project_to_screen = |world: glam::Vec3| -> (f32, f32) {
            let clip = view_proj * glam::Vec4::new(world.x, world.y, world.z, 1.0);
            let inv_w = 1.0 / clip.w.max(1e-6);
            let nx = clip.x * inv_w;
            let ny = clip.y * inv_w;
            let sx = (nx * 0.5 + 0.5) * w;
            let sy = (1.0 - (ny * 0.5 + 0.5)) * h;
            (sx, sy)
        };

        // ── Flame screen anchors ────────────────────────────────────────
        // The flame is a 2D additive quad in screen-pixel space, but it
        // needs to sit on top of a 3D candle wick whose screen position
        // depends on the gameplay-camera projection. Walk the cmd list,
        // find the CandleBatch, project each candle's wick tip with the
        // same view_proj we just built, and produce per-candle flame
        // rects (x, y, w, h) sized to match the candle's projected
        // pixel height. The Flame batching loop below consumes these in
        // order, overriding whatever the scene chose.
        //
        // We size the flame as a fraction of the *projected* candle
        // height so far candles get a smaller flame than near ones — the
        // perspective foreshortening is non-trivial because the four
        // votives sit at noticeably different depths on the table.
        let flame_anchors: Vec<[f32; 4]> = {
            let mut out: Vec<[f32; 4]> = Vec::new();
            for cmd in frame.cmds.iter() {
                if let DrawCmd::CandleBatch(placements) = cmd {
                    for p in placements.iter() {
                        let base_world = pixel_to_world(p.world_pos[0], p.world_pos[1], 0.0);
                        let tip_world = pixel_to_world(
                            p.world_pos[0],
                            p.world_pos[1],
                            crate::render::candle_mesh::WICK_TIP_Y * p.scale,
                        );
                        let (bsx, bsy) = project_to_screen(base_world);
                        let (tsx, tsy) = project_to_screen(tip_world);
                        // Projected pixel height of the candle from base
                        // to wick tip — used to scale the flame so it
                        // matches the candle's perspective foreshortening.
                        let candle_pix_h = (bsy - tsy).abs().max(1.0);
                        // Flame proportions relative to the candle's
                        // total projected height. These constants reproduce
                        // the original ~46×28 flame on a ~150-tall candle
                        // and scale gracefully with depth.
                        let flame_h = candle_pix_h * 0.42;
                        let flame_w = candle_pix_h * 0.26;
                        // Anchor: flame *base* sits at the wick tip. The
                        // shader maps `corner.y=1` to the bottom of the
                        // rect, so base_y = rect.y + rect.w; solve for
                        // rect.y = tip_sy - flame_h. Center horizontally
                        // around the projected wick.
                        let rect_x = tsx - flame_w * 0.5;
                        let rect_y = tsy - flame_h;
                        out.push([rect_x, rect_y, flame_w, flame_h]);
                    }
                    break; // assume only one candle batch per frame
                }
            }
            out
        };
        let mut next_flame_anchor: usize = 0;

        // Tile-mesh local extents (after `normalize_mesh` in tile_glb.rs):
        //   local X — long face axis  (extent ~1.000) → table-Z (front-back)
        //   local Z — short face axis (extent ~0.734) → table-X (left-right)
        //   local Y — thickness        (extent ~0.424) → world Y (up off table)
        //
        // The new basis maps the mesh into a tile lying flat with its
        // front face (+Y normal) pointing straight up at the camera.
        const LOCAL_X_EXTENT: f32 = 1.000;
        const LOCAL_Y_EXTENT: f32 = 0.424;
        const LOCAL_Z_EXTENT: f32 = 0.734;
        let tile_basis = Mat4::from_cols(
            glam::Vec4::new(0.0, 0.0, 1.0, 0.0), // local X → world +Z (front-back)
            glam::Vec4::new(0.0, 1.0, 0.0, 0.0), // local Y → world +Y (face up)
            glam::Vec4::new(1.0, 0.0, 0.0, 0.0), // local Z → world +X (left-right)
            glam::Vec4::new(0.0, 0.0, 0.0, 1.0),
        );

        {
            for (i, _htg) in self.hand_tiles.iter().enumerate() {
                let Some(&(sx, sy, sw, sh)) = hand_slots.get(i) else {
                    continue;
                };
                let is_focused = i == focus;
                let is_selected = selected.get(i).copied().unwrap_or(false);
                let slide_y = self.tile_anim_y.get(i).copied().unwrap_or(0.0);
                let slide_x_slots = self.tile_anim_x.get(i).copied().unwrap_or(0.0);

                // Tile face dimensions in pixel units (pre-projection). The
                // long axis runs front-back on the table.
                let tile_short_px = sw * 0.85; // left-right footprint on the table
                let tile_long_px = tile_short_px * 1.33; // front-back footprint
                let tile_thickness_px = tile_short_px * 0.34;

                // Tile center in pixel-layout coords.
                let cx_px = sx + sw * 0.5 + slide_x_slots * sw;
                // The slide_y residual still pushes the tile back briefly
                // for new-tile entry; in table-space that becomes a +z push
                // (further from the player) which reads as the tile sliding
                // in across the wood toward its final spot.
                let cy_px = sy + sh * 0.5 + slide_y;

                // World position: laid flat just above the table.
                let world_y_lift = tile_thickness_px * 0.5 + 4.0;
                let world = pixel_to_world(cx_px, cy_px, world_y_lift);

                // Tilt rotation, computed once and reused for both the
                // model matrix below and the overlay-anchor projection.
                // Pivot is at the tile's bottom-front corner in world
                // space (after basis * scale): bottom in world Y,
                // front (toward camera) in world +Z.
                let tilt_angle = 22.0_f32.to_radians();
                let tilt_pivot = glam::Vec3::new(
                    0.0,
                    -tile_thickness_px * 0.5,
                    tile_long_px * 0.5,
                );
                let tilt = Mat4::from_translation(tilt_pivot)
                    * Mat4::from_rotation_x(tilt_angle)
                    * Mat4::from_translation(-tilt_pivot);

                // Helper: take a point expressed *relative to the tile
                // center* (in world axes — z is front-back, y is up),
                // tilt it, translate to the world tile position, and
                // project to screen.
                let tilted_to_screen = |local: glam::Vec3| -> (f32, f32) {
                    let tilted = tilt.transform_point3(local);
                    project_to_screen(world + tilted)
                };

                // Project the tile center to screen space so 2D overlay
                // anchors (selection halo, hint pulse, hover arrow) follow
                // the tile's actual on-screen position under the tilted
                // camera.
                let (proj_cx, proj_cy) = tilted_to_screen(glam::Vec3::ZERO);
                // Project the back-top corner of the tile face so the
                // overlay can be sized to match the foreshortened (and
                // now tilted) tile footprint. The face is on the +Y
                // side of the slab; the back edge is at -Z.
                let corner_local = glam::Vec3::new(
                    tile_short_px * 0.5,
                    tile_thickness_px * 0.5,
                    -tile_long_px * 0.5,
                );
                let (proj_corner_x, proj_corner_y) = tilted_to_screen(corner_local);
                let proj_half_w = (proj_corner_x - proj_cx).abs().max(8.0);
                let proj_half_h = (proj_corner_y - proj_cy).abs().max(8.0);
                let overlay_w = proj_half_w * 2.0;
                let overlay_h = proj_half_h * 2.0;
                let overlay_x = proj_cx - proj_half_w;
                let overlay_y = proj_cy - proj_half_h;

                // Selection halo behind the 3D tile.
                if is_selected {
                    let pad = overlay_w * 0.08;
                    tile_quads.push(GpuInstance {
                        rect: [
                            overlay_x - pad,
                            overlay_y - pad,
                            overlay_w + pad * 2.0,
                            overlay_h + pad * 2.0,
                        ],
                        color: [0.9, 0.75, 0.2, 1.0],
                    });
                }

                // Hint tiles get a vertical light beam (built below) but no
                // border-style halo — the rectangular halo reads as a
                // selection indicator and confused which tiles are actually
                // selected.

                tile_3d_rects.push((i, [overlay_x, overlay_y, overlay_w, overlay_h]));

                // Hover arrow above the focused tile (in screen space).
                if is_focused {
                    let bob_period = 1.5_f32;
                    let bob_amp = overlay_h * 0.08;
                    let bob_y = (self.creation_time.elapsed().as_secs_f32() / bob_period
                        * std::f32::consts::TAU)
                        .sin()
                        * bob_amp;
                    let arrow_h = overlay_h * 0.32;
                    let arrow_w = overlay_w * 0.65;
                    let arrow_x = overlay_x + (overlay_w - arrow_w) * 0.5;
                    let arrow_y = overlay_y - arrow_h - overlay_h * 0.05 + bob_y;
                    tile_labels.push(TextLabel {
                        rect: [arrow_x, arrow_y, arrow_w, arrow_h],
                        text: "▼".to_string(),
                        color: [0.85, 0.1, 0.1, 1.0],
                    });
                }

                // Build the per-tile model matrix and write its uniform.
                if let Some(htg) = self.hand_tiles.get(i) {
                    let scale = glam::Vec3::new(
                        tile_long_px / LOCAL_X_EXTENT, // local X (long) → world Z (front-back)
                        tile_thickness_px / LOCAL_Y_EXTENT, // local Y → world Y (thickness)
                        tile_short_px / LOCAL_Z_EXTENT, // local Z (short) → world X (left-right)
                    );
                    // `tilt` was computed above the projection block so
                    // both the model matrix and the overlay anchors share
                    // the same rotation.
                    let model = Mat4::from_translation(world)
                        * tilt
                        * tile_basis
                        * Mat4::from_scale(scale);
                    self.queue.write_buffer(
                        &htg.uniform_buffer,
                        0,
                        bytemuck::bytes_of(&CameraUniform {
                            view_proj: view_proj_arr,
                            model: model.to_cols_array(),
                            base_color_factor: self.tile_base_color_factor,
                        }),
                    );
                }
            }
        }

        // Snapshot the projected tile rects for the next frame's scene draw
        // (used by hover tooltips and any other 2D HUD that needs to anchor
        // to the actual visible tile).
        self.last_projected_hand_rects = tile_3d_rects.clone();

        // Build light beam instances for hinted tiles. Look up the (already
        // arc-lifted) tile rect from `tile_3d_rects` so the beam stays
        // anchored to the visible tile position rather than the flat slot.
        let mut light_beams: Vec<GpuInstance> = Vec::new();
        for &idx in hint_indices {
            let Some(&(_, rect)) = tile_3d_rects.iter().find(|(i, _)| *i == idx) else {
                continue;
            };
            let tx = rect[0];
            let ty = rect[1];
            let tile_w = rect[2];
            let tile_h = rect[3];
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
                color: [
                    dep.suit_color[0],
                    dep.suit_color[1],
                    dep.suit_color[2],
                    alpha,
                ],
            });

            // Suit emoji.
            let bot_h = sh * 0.40;
            emoji_labels.push(TextLabel {
                rect: [sx + inset_x, sy + sh * 0.55, sw - inset_x * 2.0, bot_h],
                text: dep.suit_emoji.clone(),
                color: [
                    dep.suit_color[0],
                    dep.suit_color[1],
                    dep.suit_color[2],
                    alpha,
                ],
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

        // ── Pre-rasterize text labels → GPU textures + bind groups ──────
        struct TextDraw {
            inst_buf: wgpu::Buffer,
            bind_group: wgpu::BindGroup,
            #[allow(dead_code)]
            _tex: wgpu::Texture,
        }
        let make_text_draw = |device: &wgpu::Device,
                              queue: &wgpu::Queue,
                              text_bgl: &wgpu::BindGroupLayout,
                              sampler: &wgpu::Sampler,
                              lbl: &TextLabel,
                              font: &fontdue::Font|
         -> TextDraw {
            let tw = (lbl.rect[2] as u32).max(1);
            let th = (lbl.rect[3] as u32).max(1);
            let rgba = rasterize_label(font, &lbl.text, tw, th);
            let (tex, view) = upload_rgba_texture(device, queue, "text-lbl", &rgba, tw, th);
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("text-lbl-bg"),
                layout: text_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            });
            let inst = GpuInstance {
                rect: lbl.rect,
                color: lbl.color,
            };
            let inst_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
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

        // ── Hand tile face/emoji label GPU draws (consumed by HandTileFaces) ──
        let mut hand_face_draws: Vec<TextDraw> = Vec::new();
        if let Some(ref font) = self.ui_font {
            for lbl in &tile_labels {
                hand_face_draws.push(make_text_draw(
                    &self.device,
                    &self.queue,
                    &self.text_bind_group_layout,
                    &self.tile_sampler,
                    lbl,
                    font,
                ));
            }
        }
        if let Some(ref font) = self.emoji_font {
            for lbl in &emoji_labels {
                hand_face_draws.push(make_text_draw(
                    &self.device,
                    &self.queue,
                    &self.text_bind_group_layout,
                    &self.tile_sampler,
                    lbl,
                    font,
                ));
            }
        }

        // ── Walk frame.cmds; build per-cmd GPU resources + a parallel ─────
        // ── ordered op list, batching contiguous Quad runs into a single ──
        // ── instanced draw. ────────────────────────────────────────────────
        struct RelicDraw {
            inst_buf: wgpu::Buffer,
            relic_id: RelicId,
        }

        enum RenderOp {
            Background(BackgroundId),
            Table,
            CandleBatch(usize), // index into `candle_batches`
            QuadBatch { buf_idx: usize, count: u32 },
            FlameBatch { buf_idx: usize, count: u32 },
            TextDraw(usize),
            RelicIconDraw(usize),
            HandTileBackdrop,
            HandTileFaces,
            FluidSmoke,
        }

        let mut quad_buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut flame_buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut text_draws: Vec<TextDraw> = Vec::new();
        let mut relic_draws: Vec<RelicDraw> = Vec::new();
        let mut candle_batches: Vec<&[CandlePlacement]> = Vec::new();
        let mut ops: Vec<RenderOp> = Vec::new();

        let mut i = 0;
        while i < frame.cmds.len() {
            match &frame.cmds[i] {
                DrawCmd::Background(id) => {
                    ops.push(RenderOp::Background(*id));
                    i += 1;
                }
                DrawCmd::Table => {
                    ops.push(RenderOp::Table);
                    i += 1;
                }
                DrawCmd::CandleBatch(placements) => {
                    let idx = candle_batches.len();
                    candle_batches.push(placements.as_slice());
                    ops.push(RenderOp::CandleBatch(idx));
                    i += 1;
                }
                DrawCmd::FluidSmoke => {
                    ops.push(RenderOp::FluidSmoke);
                    i += 1;
                }
                DrawCmd::HandTileBackdrop => {
                    ops.push(RenderOp::HandTileBackdrop);
                    i += 1;
                }
                DrawCmd::HandTileFaces => {
                    ops.push(RenderOp::HandTileFaces);
                    i += 1;
                }
                DrawCmd::Quad(_) => {
                    // Collect contiguous run of Quad cmds into a single batch.
                    let mut batch: Vec<GpuInstance> = Vec::new();
                    while let Some(DrawCmd::Quad(inst)) = frame.cmds.get(i) {
                        batch.push(*inst);
                        i += 1;
                    }
                    let buf = self
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("quad-batch"),
                            contents: bytemuck::cast_slice(&batch),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                    let buf_idx = quad_buffers.len();
                    quad_buffers.push(buf);
                    ops.push(RenderOp::QuadBatch {
                        buf_idx,
                        count: batch.len() as u32,
                    });
                }
                DrawCmd::Flame(_) => {
                    // Collect contiguous run of Flame cmds into a single batch.
                    // Each instance's screen-space rect is overridden with the
                    // pre-projected wick anchor for the matching candle, so the
                    // flame stays glued to the wick under camera perspective.
                    let mut batch: Vec<GpuInstance> = Vec::new();
                    while let Some(DrawCmd::Flame(inst)) = frame.cmds.get(i) {
                        let mut fixed = *inst;
                        if let Some(anchor) = flame_anchors.get(next_flame_anchor) {
                            fixed.rect = *anchor;
                            next_flame_anchor += 1;
                        }
                        batch.push(fixed);
                        i += 1;
                    }
                    let buf = self
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("flame-batch"),
                            contents: bytemuck::cast_slice(&batch),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                    let buf_idx = flame_buffers.len();
                    flame_buffers.push(buf);
                    ops.push(RenderOp::FlameBatch {
                        buf_idx,
                        count: batch.len() as u32,
                    });
                }
                DrawCmd::Text(lbl) => {
                    if let Some(ref font) = self.ui_font {
                        let td = make_text_draw(
                            &self.device,
                            &self.queue,
                            &self.text_bind_group_layout,
                            &self.tile_sampler,
                            lbl,
                            font,
                        );
                        let idx = text_draws.len();
                        text_draws.push(td);
                        ops.push(RenderOp::TextDraw(idx));
                    }
                    i += 1;
                }
                DrawCmd::RelicIcon(icon) => {
                    if self.relic_textures.contains_key(&icon.relic_id) {
                        let inst = GpuInstance {
                            rect: icon.rect,
                            color: [1.0, 1.0, 1.0, 1.0],
                        };
                        let inst_buf =
                            self.device
                                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                    label: Some("relic-icon-inst"),
                                    contents: bytemuck::cast_slice(&[inst]),
                                    usage: wgpu::BufferUsages::VERTEX,
                                });
                        let idx = relic_draws.len();
                        relic_draws.push(RelicDraw {
                            inst_buf,
                            relic_id: icon.relic_id,
                        });
                        ops.push(RenderOp::RelicIconDraw(idx));
                    }
                    i += 1;
                }
            }
        }

        // ── Update procedural lit-mesh uniforms (table + candles) ───────
        // Written before the render pass begins, since the pass borrows
        // `self` immutably.
        let needs_table = ops.iter().any(|o| matches!(o, RenderOp::Table));
        if needs_table {
            // Horizontal table: the mesh is built in XY (normal +Z), so we
            // rotate -90° around X to lay it flat with normal +Y. Then
            // scale it big enough to comfortably exceed the visible
            // foreshortened footprint of the table-plane region.
            let table_w = w * 3.5;
            let table_d = h * 3.5;
            let model = Mat4::from_translation(glam::Vec3::new(0.0, 0.0, 0.0))
                * Mat4::from_rotation_x(-std::f32::consts::FRAC_PI_2)
                * Mat4::from_scale(glam::Vec3::new(table_w, table_d, 1.0));
            self.table_instance.write_uniform(
                &self.queue,
                view_proj_arr,
                model,
                self.table_mesh.default_material,
            );
        }
        // Candles: scenes pass `world_pos = (pixel_x, pixel_y, world_y_lift)`
        // — we map pixel x/y onto the table plane and use world_y as the
        // base height above the wood (usually 0 so the candle sits on it).
        for batch in &candle_batches {
            for (slot_i, placement) in batch.iter().enumerate() {
                let Some(instances) = self.candle_instances.get(slot_i) else {
                    break;
                };
                let base = pixel_to_world(
                    placement.world_pos[0],
                    placement.world_pos[1],
                    placement.world_pos[2],
                );
                let model = Mat4::from_translation(base)
                    * Mat4::from_scale(glam::Vec3::splat(placement.scale));
                instances[0].write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    self.candle_wax_mesh.default_material,
                );
                instances[1].write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    self.candle_wick_mesh.default_material,
                );
            }
        }

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
            rect: [
                0.0,
                0.0,
                self.size.width.max(1) as f32,
                self.size.height.max(1) as f32,
            ],
            color: [1.0, 1.0, 1.0, 1.0],
        };
        let fluid_inst_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("fluid-inst"),
                contents: bytemuck::cast_slice(&[fluid_inst]),
                usage: wgpu::BufferUsages::VERTEX,
            });

        // Pre-create background image instance buffer (must outlive render pass).
        let bg_inst = GpuInstance {
            rect: [
                0.0,
                0.0,
                self.size.width.max(1) as f32,
                self.size.height.max(1) as f32,
            ],
            color: [1.0, 1.0, 1.0, 1.0],
        };
        let bg_inst_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("bg-inst"),
                contents: bytemuck::cast_slice(&[bg_inst]),
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

            // Walk the ordered op list. Earlier ops render under later ones.
            for op in &ops {
                match op {
                    RenderOp::Background(id) => {
                        if let Some(bg_tex) = self.background_textures.get(id) {
                            pass.set_pipeline(&self.image_pipeline);
                            pass.set_bind_group(0, &self.globals_bind_group, &[]);
                            pass.set_bind_group(1, &bg_tex.bind_group, &[]);
                            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                            pass.set_vertex_buffer(1, bg_inst_buf.slice(..));
                            pass.set_index_buffer(
                                self.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint16,
                            );
                            pass.draw_indexed(0..6, 0, 0..1);
                        }
                    }
                    RenderOp::Table => {
                        pass.set_pipeline(&self.lit_mesh_pipeline);
                        pass.set_bind_group(0, &self.table_instance.bind_group, &[]);
                        pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.table_mesh.vertex_buffer.slice(..));
                        pass.set_index_buffer(
                            self.table_mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(0..self.table_mesh.index_count, 0, 0..1);
                    }
                    RenderOp::CandleBatch(batch_idx) => {
                        let batch = candle_batches[*batch_idx];
                        pass.set_pipeline(&self.lit_mesh_pipeline);
                        pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                        for (slot_i, _) in batch.iter().enumerate() {
                            let Some(instances) = self.candle_instances.get(slot_i) else {
                                break;
                            };
                            // Wax body.
                            pass.set_bind_group(0, &instances[0].bind_group, &[]);
                            pass.set_vertex_buffer(0, self.candle_wax_mesh.vertex_buffer.slice(..));
                            pass.set_index_buffer(
                                self.candle_wax_mesh.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            pass.draw_indexed(0..self.candle_wax_mesh.index_count, 0, 0..1);
                            // Wick.
                            pass.set_bind_group(0, &instances[1].bind_group, &[]);
                            pass.set_vertex_buffer(
                                0,
                                self.candle_wick_mesh.vertex_buffer.slice(..),
                            );
                            pass.set_index_buffer(
                                self.candle_wick_mesh.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint32,
                            );
                            pass.draw_indexed(0..self.candle_wick_mesh.index_count, 0, 0..1);
                        }
                    }
                    RenderOp::HandTileBackdrop => {
                        if let Some(ref lbb) = light_beam_buffer {
                            pass.set_pipeline(&self.light_beam_pipeline);
                            pass.set_bind_group(0, &self.globals_bind_group, &[]);
                            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                            pass.set_vertex_buffer(1, lbb.slice(..));
                            pass.set_index_buffer(
                                self.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint16,
                            );
                            pass.draw_indexed(0..6, 0, 0..light_beams.len() as u32);
                        }
                        if let Some(ref tib) = tile_instance_buffer {
                            // Halo/selection backdrop quads (drawn before the
                            // 3D tile mesh so the tile sits on top of them).
                            pass.set_pipeline(&self.tile_quad_pipeline);
                            pass.set_bind_group(0, &self.globals_bind_group, &[]);
                            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                            pass.set_vertex_buffer(1, tib.slice(..));
                            pass.set_index_buffer(
                                self.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint16,
                            );
                            pass.draw_indexed(0..6, 0, 0..tile_quads.len() as u32);
                        }
                        // 3D hand tiles: one draw per (tile, primitive).
                        // Tiles in the GLB have multiple material slots
                        // (e.g. ivory face + bamboo body); draw each.
                        if !self.tile_primitives.is_empty() {
                            pass.set_pipeline(&self.tile_pipeline);
                            // Point lights (group 1) are the same for every
                            // tile this frame — bind once outside the loop.
                            pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                            for (i, _) in &tile_3d_rects {
                                let Some(htg) = self.hand_tiles.get(*i) else {
                                    continue;
                                };
                                for (pi, prim) in self.tile_primitives.iter().enumerate() {
                                    let Some(bg) = htg.bind_groups.get(pi) else {
                                        continue;
                                    };
                                    pass.set_bind_group(0, bg, &[]);
                                    pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                                    pass.set_index_buffer(
                                        prim.index_buffer.slice(..),
                                        wgpu::IndexFormat::Uint32,
                                    );
                                    pass.draw_indexed(0..prim.index_count, 0, 0..1);
                                }
                            }
                        }
                    }
                    RenderOp::FluidSmoke => {
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
                    }
                    RenderOp::HandTileFaces => {
                        for td in &hand_face_draws {
                            pass.set_pipeline(&self.text_pipeline);
                            pass.set_bind_group(0, &self.globals_bind_group, &[]);
                            pass.set_bind_group(1, &td.bind_group, &[]);
                            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                            pass.set_vertex_buffer(1, td.inst_buf.slice(..));
                            pass.set_index_buffer(
                                self.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint16,
                            );
                            pass.draw_indexed(0..6, 0, 0..1);
                        }
                    }
                    RenderOp::QuadBatch { buf_idx, count } => {
                        pass.set_pipeline(&self.quad_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                        pass.set_vertex_buffer(1, quad_buffers[*buf_idx].slice(..));
                        pass.set_index_buffer(
                            self.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        pass.draw_indexed(0..6, 0, 0..*count);
                    }
                    RenderOp::FlameBatch { buf_idx, count } => {
                        pass.set_pipeline(&self.flame_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                        pass.set_vertex_buffer(1, flame_buffers[*buf_idx].slice(..));
                        pass.set_index_buffer(
                            self.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        pass.draw_indexed(0..6, 0, 0..*count);
                    }
                    RenderOp::TextDraw(idx) => {
                        let td = &text_draws[*idx];
                        pass.set_pipeline(&self.text_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
                        pass.set_bind_group(1, &td.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                        pass.set_vertex_buffer(1, td.inst_buf.slice(..));
                        pass.set_index_buffer(
                            self.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        pass.draw_indexed(0..6, 0, 0..1);
                    }
                    RenderOp::RelicIconDraw(idx) => {
                        let rd = &relic_draws[*idx];
                        if let Some(rtex) = self.relic_textures.get(&rd.relic_id) {
                            pass.set_pipeline(&self.image_pipeline);
                            pass.set_bind_group(0, &self.globals_bind_group, &[]);
                            pass.set_bind_group(1, &rtex.bind_group, &[]);
                            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                            pass.set_vertex_buffer(1, rd.inst_buf.slice(..));
                            pass.set_index_buffer(
                                self.index_buffer.slice(..),
                                wgpu::IndexFormat::Uint16,
                            );
                            pass.draw_indexed(0..6, 0, 0..1);
                        }
                    }
                }
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
        surface_frame.present();
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
    _modifier: (f32, f32, f32, f32),
    anim_scale_score: f32,
    plays: u32,
    plays_max: u32,
    discards: u32,
    discards_max: u32,
) -> Vec<GpuInstance> {
    use crate::render::theme::color as themec;

    // The 3D table + candles are the visual centerpiece. The score panel and
    // modifier strip do NOT draw full-width opaque cartouches that would
    // obscure the 3D scene — instead the score cartouche is a narrow
    // centered pill, and the modifier strip skips its background entirely
    // so the candles + table show through.
    let (sx, sy, sw, sh) = (score.0, score.1, score.2, score.3);
    let mut v: Vec<GpuInstance> = Vec::new();

    // Centered score cartouche — ~38% of the strip width with the panel's
    // animation scale-pop applied. Translucent indigo so the background
    // bleeds through softly.
    let cart_w = sw * 0.38;
    let cart_h = sh * 0.78;
    let cart_x_base = sx + (sw - cart_w) * 0.5;
    let cart_y_base = sy + (sh - cart_h) * 0.5;
    let (cx, cy, cw, ch) = apply_transform_rect(
        cart_x_base,
        cart_y_base,
        cart_w,
        cart_h,
        crate::render::animation::Transform2D {
            offset_x: 0.0,
            offset_y: 0.0,
            scale: anim_scale_score,
            opacity: 1.0,
        },
    );
    v.push(GpuInstance {
        rect: [cx, cy, cw, ch],
        color: themec::alpha(themec::MIDNIGHT, 0.78),
    });
    let bt = (ch * 0.025).clamp(1.0, 3.0);
    v.push(GpuInstance {
        rect: [cx, cy, cw, bt],
        color: themec::GOLD,
    });
    v.push(GpuInstance {
        rect: [cx, cy + ch - bt, cw, bt],
        color: themec::GOLD,
    });
    v.push(GpuInstance {
        rect: [cx, cy + bt, bt, ch - 2.0 * bt],
        color: themec::GOLD,
    });
    v.push(GpuInstance {
        rect: [cx + cw - bt, cy + bt, bt, ch - 2.0 * bt],
        color: themec::GOLD,
    });

    // Pip indicators — two stacked rows of jade/amber pills floating at the
    // right edge of the logical score-panel region (NOT over the cartouche).
    let pip = (sh * 0.22).clamp(8.0, 28.0);
    let gap = pip * 0.25;
    let margin = pip * 0.9;
    let row_gap = pip * 0.3;

    let total_h = pip + row_gap + pip;
    let row1_y = sy + (sh - total_h) * 0.5;
    let row2_y = row1_y + pip + row_gap;

    let plays_row_w = plays_max as f32 * pip + (plays_max.saturating_sub(1)) as f32 * gap;
    let plays_x0 = sx + sw - margin - plays_row_w;
    for i in 0..plays_max {
        let x = plays_x0 + i as f32 * (pip + gap);
        let filled = i < plays;
        v.push(GpuInstance {
            rect: [x, row1_y, pip, pip],
            color: if filled {
                themec::JADE
            } else {
                themec::alpha(themec::JADE, 0.25)
            },
        });
    }

    let disc_row_w = discards_max as f32 * pip + (discards_max.saturating_sub(1)) as f32 * gap;
    let disc_x0 = sx + sw - margin - disc_row_w;
    for i in 0..discards_max {
        let x = disc_x0 + i as f32 * (pip + gap);
        let filled = i < discards;
        v.push(GpuInstance {
            rect: [x, row2_y, pip, pip],
            color: if filled {
                themec::AMBER
            } else {
                themec::alpha(themec::AMBER, 0.25)
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
    use crate::render::theme::color as themec;
    let mut v = Vec::new();

    // Score panel hero cartouche.
    v.push(GpuInstance {
        rect: [score.0, score.1, score.2, score.3],
        color: themec::TWILIGHT,
    });
    v.push(GpuInstance {
        rect: [modifier.0, modifier.1, modifier.2, modifier.3],
        color: themec::INDIGO,
    });

    // Place the 3 choice quads centred in the hand strip, each ~4 tile-widths wide.
    let pick_indices = [1usize, 6, 11];
    for (choice_idx, &slot_idx) in pick_indices.iter().enumerate() {
        let slot = hand_slots
            .get(slot_idx)
            .or_else(|| hand_slots.get(choice_idx));
        if let Some(&(x, y, w, h)) = slot {
            // Faint gold halo around the focused choice.
            if choice_idx == cursor {
                let halo = h * 0.04;
                v.push(GpuInstance {
                    rect: [x - halo, y - halo, w + halo * 2.0, h + halo * 2.0],
                    color: themec::alpha(themec::GOLD, 0.5),
                });
            }
            let bg = if choice_idx == cursor {
                themec::TWILIGHT
            } else {
                themec::DUSK
            };
            v.push(GpuInstance {
                rect: [x, y, w, h],
                color: bg,
            });
            // Gold inset border.
            let bt = (h * 0.025).clamp(1.0, 3.0);
            let border = if choice_idx == cursor {
                themec::GOLD
            } else {
                themec::BRASS
            };
            v.push(GpuInstance {
                rect: [x, y, w, bt],
                color: border,
            });
            v.push(GpuInstance {
                rect: [x, y + h - bt, w, bt],
                color: border,
            });
            v.push(GpuInstance {
                rect: [x, y + bt, bt, h - 2.0 * bt],
                color: border,
            });
            v.push(GpuInstance {
                rect: [x + w - bt, y + bt, bt, h - 2.0 * bt],
                color: border,
            });
        }
    }

    v
}
