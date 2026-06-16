//! Lazy GPU upload for deferred room GLB environments.
//!
//! Contract (all hub/run room GLBs):
//! - CPU decode may run on a worker; GPU upload is always main-thread.
//! - [`room_cpu_env_ready`] + per-`*_cpu_ready_for_gpu_upload` gate uploads.
//! - [`try_commit_room_environment_gpu_upload`] never panics on eviction races — it
//!   logs, re-queues CPU decode, and retries on a later frame.
//! - Low-memory GPU eviction uses [`crate::room_gpu_resident::RoomGpuResidentDesc`].
//! - The active scene room is pinned in the GPU LRU during [`WgpuRenderer::poll_room_prefetch_gpu_uploads`].

use super::*;

use crate::room_gpu_resident::victory_uses_3d_moon;
use crate::scene_keys;

use std::time::{Duration, Instant};

use crate::gltf_helpers::{
    GLTF_PBR_FLAG_GAMEPLAY_CASH_IN_POLYCHROME, GLTF_PBR_FLAG_MAIN_MENU_MOON_PHASE,
    GLTF_PBR_FLAG_MAIN_MENU_STAR_RAINBOW, GLTF_PBR_FLAG_ROOM_ARCHIVE_DECAL,
    GLTF_PBR_FLAG_ROOM_CANDLE_WAX, GLTF_PBR_FLAG_ROOM_DYNAMIC_SHADOW_RECEIVER,
    GLTF_PBR_FLAG_ROOM_HALLWAY_WALL_TINT, GLTF_PBR_FLAG_ROOM_READABLE_SURFACE,
    GLTF_PBR_FLAG_SKIP_BAKED_CONTACT_AO, GltfPbrUniform, build_sampler_descriptor,
};
use crate::room_env_gltf::{RoomEnvPrimitiveCpu, RoomTextureUsageClass};
use crate::room_gi_bake::RoomGiRoom;
use crate::wgpu_renderer::resources::{RoomEnvTextureCache, RoomEnvTextureDedupeHint};
use wgpu::util::DeviceExt;

pub(super) use crate::room_gpu_resident::{
    ROOM_ARCHIVE, ROOM_GAMEPLAY, ROOM_HALLWAY, ROOM_MAIN_MENU, ROOM_SHOP, ROOM_STAIRCASE,
    RoomGpuResidentDesc, RoomGpuResidentId,
};

use crate::score_roller_layout::{self, GAMEPLAY_SCORE_ROLLER_SLOT_COUNT};

#[inline]
fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

#[derive(Clone, Copy, Debug, Default)]
struct RoomUploadAuditMetrics {
    payload: crate::room_gpu_profile::RoomCpuUploadPayload,
    packed_asset_bytes_read: u64,
    decoded_cpu_payload_bytes: u64,
    gpu_resident_estimate_bytes: u64,
}

/// Max main-thread wall time for one gameplay env upload slice during prefetch / fade.
const GAMEPLAY_ROOM_GPU_UPLOAD_BUDGET_MS: f32 = 6.0;
/// Larger slice while idle on Performance/Visuals so gameplay finishes before scene entry.
const GAMEPLAY_EAGER_UPLOAD_BUDGET_MS: f32 = 32.0;
/// Slice budget for standard room env uploads (shop/hallway) during active scene / fades.
const ROOM_ENV_GPU_UPLOAD_BUDGET_MS: f32 = 4.0;
/// Eager warm-up budget for standard room env uploads while idle on non-low-memory presets.
const ROOM_ENV_EAGER_UPLOAD_BUDGET_MS: f32 = 24.0;
/// Per-frame GPU upload budget while the splash loading plate is up (Performance/Visuals).
const SPLASH_EAGER_ROOM_GPU_UPLOAD_BUDGET_MS: f32 = 96.0;

fn splash_eager_room_gpu_mask(mode: mahjuro_gfx_types::GraphicsMode) -> u8 {
    match mode {
        mahjuro_gfx_types::GraphicsMode::LowMemory => ROOM_MAIN_MENU,
        _ => {
            ROOM_MAIN_MENU
                | ROOM_SHOP
                | ROOM_ARCHIVE
                | ROOM_HALLWAY
                | ROOM_GAMEPLAY
                | ROOM_STAIRCASE
        }
    }
}

pub(super) fn splash_eager_rooms_gpu_loaded(
    mode: mahjuro_gfx_types::GraphicsMode,
    rooms_gpu_loaded: u8,
) -> bool {
    let mask = splash_eager_room_gpu_mask(mode);
    rooms_gpu_loaded & mask == mask
}
/// Per-frame upload budget while a transition is held at full black.
///
/// Keeping this bounded prevents single-frame stalls when destination rooms still need
/// heavy uploads (notably gameplay) at scene-swap time.
const TRANSITION_BLACK_ROOM_GPU_UPLOAD_BUDGET_MS: f32 = 12.0;

fn gameplay_eager_upload_budget_ms(mode: mahjuro_gfx_types::GraphicsMode) -> f32 {
    match mode {
        mahjuro_gfx_types::GraphicsMode::LowMemory => GAMEPLAY_ROOM_GPU_UPLOAD_BUDGET_MS,
        _ => GAMEPLAY_EAGER_UPLOAD_BUDGET_MS,
    }
}

fn room_env_eager_upload_budget_ms(mode: mahjuro_gfx_types::GraphicsMode) -> f32 {
    match mode {
        mahjuro_gfx_types::GraphicsMode::LowMemory => ROOM_ENV_GPU_UPLOAD_BUDGET_MS,
        _ => ROOM_ENV_EAGER_UPLOAD_BUDGET_MS,
    }
}

/// Drain prefetch workers and return whether CPU env meshes are ready — never block the main thread.
fn room_cpu_env_ready(ready: impl Fn() -> bool) -> bool {
    crate::room_preload::try_drain_room_cpu_prefetch_threads();
    ready()
}

fn room_upload_runtime_phase(scene_key: Option<&str>) -> crate::room_gpu_profile::RuntimePhase {
    use crate::room_gpu_profile::RuntimePhase;
    match scene_key {
        None => RuntimePhase::StartupBlocking,
        Some(scene_keys::GAMEPLAY | scene_keys::DEFEAT) => RuntimePhase::GameplayInteractive,
        Some(scene_keys::VICTORY) => RuntimePhase::MenuInteractive,
        Some(_) => RuntimePhase::MenuInteractive,
    }
}

fn collect_room_upload_audit_metrics(
    room: &'static str,
    cpu: &crate::room_glb::RoomGlbCpu,
) -> RoomUploadAuditMetrics {
    let payload = crate::room_gpu_profile::count_cpu_payload(&cpu.environment_primitives);
    let texture_summary =
        crate::room_gpu_profile::log_room_texture_audit(room, &cpu.environment_primitives);
    let geometry_gpu_bytes = payload.vertex_bytes.saturating_add(payload.index_bytes);
    RoomUploadAuditMetrics {
        payload,
        packed_asset_bytes_read: cpu.packed_asset_bytes_read,
        decoded_cpu_payload_bytes: cpu.decoded_cpu_payload_bytes,
        gpu_resident_estimate_bytes: texture_summary
            .total_gpu_bytes_estimate_bytes
            .saturating_add(geometry_gpu_bytes),
    }
}

fn retained_room_cpu_payload_bytes(cpu: &crate::room_glb::RoomGlbCpu) -> u64 {
    crate::room_gpu_profile::count_cpu_payload(&cpu.environment_primitives).total_bytes()
}

#[inline]
fn hallway_env_shader_flags(node_name: Option<&str>) -> u32 {
    if node_name == Some(crate::hallway_glb::HALLWAY_WALLS_NODE) {
        GLTF_PBR_FLAG_ROOM_HALLWAY_WALL_TINT
    } else {
        0
    }
}

#[inline]
fn archive_env_shader_flags(node_name: Option<&str>) -> u32 {
    if matches!(
        node_name,
        Some(
            crate::archive_glb::SIGN_DESCRIPTION_LEFT
                | crate::archive_glb::SIGN_DESCRIPTION_RIGHT
                | crate::archive_glb::INSPECT_PLAQUE
        )
    ) {
        GLTF_PBR_FLAG_ROOM_ARCHIVE_DECAL
    } else {
        0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RoomEnvSurfaceRole {
    StaticRoom,
    ReadableSurface,
}

#[inline]
fn room_env_surface_role(
    scene_key: &str,
    node_name: Option<&str>,
    _material_name: Option<&str>,
) -> RoomEnvSurfaceRole {
    if scene_key != scene_keys::ARCHIVE {
        return RoomEnvSurfaceRole::StaticRoom;
    }
    let Some(name) = node_name else {
        return RoomEnvSurfaceRole::StaticRoom;
    };
    let node = name.to_ascii_lowercase();
    let readable = node.starts_with("text_")
        || node.starts_with("btn_")
        || node.starts_with("sign_")
        || node.starts_with("plaque_")
        || matches!(node.as_str(), "inspect_plaque" | "brass_plaque");
    if readable {
        RoomEnvSurfaceRole::ReadableSurface
    } else {
        RoomEnvSurfaceRole::StaticRoom
    }
}

#[inline]
fn room_env_surface_role_flags(
    scene_key: &str,
    node_name: Option<&str>,
    material_name: Option<&str>,
) -> u32 {
    match room_env_surface_role(scene_key, node_name, material_name) {
        RoomEnvSurfaceRole::StaticRoom => 0,
        RoomEnvSurfaceRole::ReadableSurface => {
            GLTF_PBR_FLAG_ROOM_READABLE_SURFACE | GLTF_PBR_FLAG_SKIP_BAKED_CONTACT_AO
        }
    }
}

#[inline]
fn main_menu_env_shader_flags(node_name: Option<&str>) -> u32 {
    let Some(name) = node_name else {
        return 0;
    };
    if crate::main_menu_glb::is_main_menu_moon_env_node(name) {
        GLTF_PBR_FLAG_MAIN_MENU_MOON_PHASE
    } else if crate::main_menu_glb::is_main_menu_star_env_node(name) {
        GLTF_PBR_FLAG_MAIN_MENU_STAR_RAINBOW
    } else {
        0
    }
}

#[inline]
fn room_env_candle_wax_flags(
    scene_key: &str,
    node_name: Option<&str>,
    material_name: Option<&str>,
) -> u32 {
    let is_candle_wax = match scene_key {
        scene_keys::SHOP => {
            material_name.is_some_and(|name| name.to_ascii_lowercase().starts_with("candle wax"))
        }
        scene_keys::GAMEPLAY => {
            node_name.is_some_and(|name| name.starts_with("candles"))
                && material_name == Some("Cream Scratched Porcelain")
        }
        _ => false,
    };
    if is_candle_wax {
        GLTF_PBR_FLAG_ROOM_CANDLE_WAX
    } else {
        0
    }
}

#[inline]
fn shop_dynamic_shadow_receiver_flags(
    scene_key: &str,
    node_name: Option<&str>,
    material_name: Option<&str>,
) -> u32 {
    if scene_key != scene_keys::SHOP {
        return 0;
    }
    let node = node_name.unwrap_or("").to_ascii_lowercase();
    let material = material_name.unwrap_or("").to_ascii_lowercase();
    let receives = contains_any(&node, &["cubby", "recess", "hole", "pillow", "tabletop"])
        || contains_any(
            &node,
            &[
                "player_gold_dish",
                "player_relic_dish",
                "player_talisman_dish",
            ],
        )
        || contains_any(
            &material,
            &[
                "red velvet",
                "ratten wicker",
                "wicker",
                "stone.107",
                "concrete surface",
            ],
        );
    if receives {
        GLTF_PBR_FLAG_ROOM_DYNAMIC_SHADOW_RECEIVER
    } else {
        0
    }
}

#[inline]
fn room_env_shader_flags(
    scene_key: &str,
    node_name: Option<&str>,
    material_name: Option<&str>,
) -> u32 {
    let scene_flags = match scene_key {
        scene_keys::HALLWAY => hallway_env_shader_flags(node_name),
        scene_keys::ARCHIVE => archive_env_shader_flags(node_name),
        scene_keys::MAIN_MENU => main_menu_env_shader_flags(node_name),
        _ => 0,
    };
    scene_flags
        | room_env_surface_role_flags(scene_key, node_name, material_name)
        | room_env_candle_wax_flags(scene_key, node_name, material_name)
        | shop_dynamic_shadow_receiver_flags(scene_key, node_name, material_name)
}

#[inline]
fn scene_key_room_shadow_room(scene_key: &str) -> Option<RoomGiRoom> {
    match scene_key {
        scene_keys::SHOP => Some(RoomGiRoom::Shop),
        scene_keys::HALLWAY => Some(RoomGiRoom::Hallway),
        scene_keys::ARCHIVE => Some(RoomGiRoom::Archive),
        scene_keys::MAIN_MENU => Some(RoomGiRoom::MainMenu),
        scene_keys::STAIRWAY => Some(RoomGiRoom::Stairway),
        scene_keys::GAMEPLAY => Some(RoomGiRoom::Gameplay),
        _ => None,
    }
}

#[inline]
fn room_env_baked_contact_ao_flags(
    scene_key: &str,
    node_name: Option<&str>,
    material_name: Option<&str>,
) -> u32 {
    let Some(room) = scene_key_room_shadow_room(scene_key) else {
        return 0;
    };
    let class = crate::room_shadow_bake::primitive_contact_ao_class(room, node_name, material_name);
    if class.receiver <= 0.0 {
        GLTF_PBR_FLAG_SKIP_BAKED_CONTACT_AO
    } else {
        0
    }
}

#[inline]
fn room_env_pbr_uniform(
    prim: &crate::tile_glb::LoadedPrimitive,
    scene_key: &str,
    node_name: Option<&str>,
    material_name: Option<&str>,
) -> GltfPbrUniform {
    let mut pbr_uniform = GltfPbrUniform::from_loaded(
        prim.metallic_factor,
        prim.roughness_factor,
        prim.emissive_factor,
        prim.alpha_mode,
        prim.alpha_cutoff,
    );
    pbr_uniform.add_flags(room_env_shader_flags(scene_key, node_name, material_name));
    pbr_uniform.add_flags(room_env_baked_contact_ao_flags(
        scene_key,
        node_name,
        material_name,
    ));
    pbr_uniform._pad1 = [
        room_lightmap_wrap_mode_u32(prim.sampler.wrap_s),
        room_lightmap_wrap_mode_u32(prim.sampler.wrap_t),
    ];
    pbr_uniform
}

fn room_lightmap_wrap_mode_u32(mode: gltf::texture::WrappingMode) -> u32 {
    match mode {
        gltf::texture::WrappingMode::ClampToEdge => 0,
        gltf::texture::WrappingMode::Repeat => 1,
        gltf::texture::WrappingMode::MirroredRepeat => 2,
    }
}

fn create_room_env_mesh_buffers(
    device: &wgpu::Device,
    label_prefix: &str,
    i: usize,
    prim: &crate::tile_glb::LoadedPrimitive,
) -> (wgpu::Buffer, wgpu::Buffer, u32) {
    let mesh = crate::room_lightmap_uv::build_room_env_lightmap_gpu_mesh(prim);
    let index_count = mesh.indices.len() as u32;
    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(&format!("{label_prefix}-env-verts-{i}")),
        contents: bytemuck::cast_slice(&mesh.vertices),
        usage: wgpu::BufferUsages::VERTEX,
    });
    let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some(&format!("{label_prefix}-env-idx-{i}")),
        contents: bytemuck::cast_slice(&mesh.indices),
        usage: wgpu::BufferUsages::INDEX,
    });
    (vb, ib, index_count)
}

fn room_shadow_mask_classes(
    room: crate::room_gi_bake::RoomGiRoom,
    env_prims: &[RoomEnvPrimitiveCpu],
) -> Vec<crate::room_shadow_bake::PrimitiveContactAoClass> {
    env_prims
        .iter()
        .map(|prim| {
            crate::room_shadow_bake::primitive_contact_ao_class(
                room,
                prim.gltf_node_name.as_deref(),
                prim.material_name.as_deref(),
            )
        })
        .collect()
}

fn sampler_class_hash(sampler: crate::gltf_helpers::GltfSamplerCpu) -> u64 {
    use std::hash::{Hash, Hasher};
    fn wrap_tag(wrap: gltf::texture::WrappingMode) -> u8 {
        match wrap {
            gltf::texture::WrappingMode::ClampToEdge => 0,
            gltf::texture::WrappingMode::MirroredRepeat => 1,
            gltf::texture::WrappingMode::Repeat => 2,
        }
    }
    fn mag_tag(mag: Option<gltf::texture::MagFilter>) -> u8 {
        match mag {
            Some(gltf::texture::MagFilter::Nearest) => 1,
            Some(gltf::texture::MagFilter::Linear) => 2,
            None => 0,
        }
    }
    fn min_tag(min: Option<gltf::texture::MinFilter>) -> u8 {
        match min {
            Some(gltf::texture::MinFilter::Nearest) => 1,
            Some(gltf::texture::MinFilter::Linear) => 2,
            Some(gltf::texture::MinFilter::NearestMipmapNearest) => 3,
            Some(gltf::texture::MinFilter::LinearMipmapNearest) => 4,
            Some(gltf::texture::MinFilter::NearestMipmapLinear) => 5,
            Some(gltf::texture::MinFilter::LinearMipmapLinear) => 6,
            None => 0,
        }
    }
    let mut hasher = rustc_hash::FxHasher::default();
    wrap_tag(sampler.wrap_s).hash(&mut hasher);
    wrap_tag(sampler.wrap_t).hash(&mut hasher);
    mag_tag(sampler.mag_filter).hash(&mut hasher);
    min_tag(sampler.min_filter).hash(&mut hasher);
    hasher.finish()
}

fn room_texture_dedupe_hint(
    env_prim: &RoomEnvPrimitiveCpu,
    usage: RoomTextureUsageClass,
    mips: bool,
) -> RoomEnvTextureDedupeHint {
    let source_identity = env_prim
        .texture_sources
        .for_usage(usage)
        .map(|src| src.source_identity.clone());
    RoomEnvTextureDedupeHint {
        source_identity,
        usage_class: usage as u8,
        sampler_class_hash: sampler_class_hash(env_prim.mesh.sampler),
        mip_policy_tag: if mips { 1 } else { 0 },
    }
}

fn upload_room_texture_slot(
    room_tex_cache: &mut RoomEnvTextureCache,
    ctx: &RoomGpuUploadCtx<'_>,
    label: String,
    env_prim: &RoomEnvPrimitiveCpu,
    usage: RoomTextureUsageClass,
    rgba: Option<&(Vec<u8>, u32, u32)>,
    mip_chain: Option<&[(Vec<u8>, u32, u32)]>,
    format: wgpu::TextureFormat,
    mips: bool,
    fallback: &wgpu::TextureView,
) -> wgpu::TextureView {
    let hint = room_texture_dedupe_hint(env_prim, usage, mips);
    room_tex_cache.upload_slot_with_hint(
        ctx.device,
        ctx.queue,
        label,
        rgba,
        mip_chain,
        format,
        mips,
        fallback,
        Some(&hint),
    )
}

struct RoomLightmapGpu {
    view: wgpu::TextureView,
    uv_rects: Vec<[f32; 4]>,
}

/// Room GLB mesh counts can drift from committed lightmaps between rebakes; keep GPU upload alive
/// by truncating or padding per-primitive atlas rects to the live primitive count.
fn align_room_lightmap_uv_rects(
    room: RoomGiRoom,
    uv_rects: Vec<[f32; 4]>,
    prim_count: usize,
) -> Vec<[f32; 4]> {
    if uv_rects.len() == prim_count {
        return uv_rects;
    }
    log::warn!(
        "{room:?} room lightmap primitive UV rect count ({}) does not match room GPU upload ({prim_count}); \
         aligning to GPU primitive count — rebake room lightmap after GLB changes",
        uv_rects.len(),
    );
    let mut aligned = uv_rects;
    aligned.truncate(prim_count);
    aligned.resize(prim_count, [0.0; 4]);
    aligned
}

fn upload_room_lightmap_gpu(
    ctx: &RoomGpuUploadCtx<'_>,
    room: RoomGiRoom,
    prim_count: usize,
    label: &str,
) -> RoomLightmapGpu {
    let bake = crate::room_gi_bake::load_room_gi_lightmap(room)
        .unwrap_or_else(|e| panic!("{room:?} room lightmap bake is invalid: {e:#}"))
        .unwrap_or_else(|| {
            panic!("{room:?} room lightmap bake is missing; run `cargo build` to rebake room GI")
        });
    let uv_rects = align_room_lightmap_uv_rects(room, bake.primitive_uv_rects, prim_count);
    let expected_values = bake.width as usize * bake.height as usize * 4;
    assert_eq!(
        bake.pixels_rgba_f32.len(),
        expected_values,
        "{:?} room lightmap pixel count does not match dimensions",
        bake.room
    );
    let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: bake.width.max(1),
            height: bake.height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    write_room_lightmap_texture(
        ctx.queue,
        &texture,
        bake.width.max(1),
        bake.height.max(1),
        bytemuck::cast_slice(&bake.pixels_rgba_f32),
    );
    RoomLightmapGpu {
        view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        uv_rects,
    }
}

fn write_room_lightmap_texture(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    tight_rgba32f: &[u8],
) {
    let unpadded = width * 16;
    let bytes_per_row = unpadded.div_ceil(256) * 256;
    let extent = wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    };
    let mut padded = vec![0u8; (bytes_per_row * height) as usize];
    for y in 0..height {
        let src = (y * unpadded) as usize;
        let dst = (y * bytes_per_row) as usize;
        padded[dst..dst + unpadded as usize]
            .copy_from_slice(&tight_rgba32f[src..src + unpadded as usize]);
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &padded,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_row),
            rows_per_image: Some(height),
        },
        extent,
    );
}

fn create_room_env_material_bind_group(
    ctx: &RoomGpuUploadCtx<'_>,
    label: &'static str,
    uniform_buffer: &wgpu::Buffer,
    prim: &TilePrimitiveGpu,
    decal_view: &wgpu::TextureView,
    distortion_buffer: &wgpu::Buffer,
    lightmap_view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    ctx.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout: ctx.room_env_material_layout,
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
                resource: wgpu::BindingResource::Sampler(&prim.sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(decal_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&prim.normal_view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: prim.pbr_uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(&prim.metallic_roughness_view),
            },
            wgpu::BindGroupEntry {
                binding: 7,
                resource: wgpu::BindingResource::TextureView(&prim.emissive_view),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: distortion_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: wgpu::BindingResource::TextureView(lightmap_view),
            },
        ],
    })
}

pub(super) struct GameplayRoomGpuUpload {
    prim_count: usize,
    next_prim: usize,
    prims: Vec<TilePrimitiveGpu>,
    shadow_mask_classes: Vec<crate::room_shadow_bake::PrimitiveContactAoClass>,
    room_tex_cache: RoomEnvTextureCache,
    _white_albedo_tex: wgpu::Texture,
    white_albedo_view: wgpu::TextureView,
    _decal_tex: wgpu::Texture,
    gameplay_decal_view: wgpu::TextureView,
    cash_in_prim_indices: Vec<usize>,
    score_roller_prim_groups: Vec<Vec<usize>>,
    score_roller_pivots_doc: Vec<[f32; 3]>,
    score_roller_axes_doc: Vec<[f32; 3]>,
    score_roller_found: [bool; GAMEPLAY_SCORE_ROLLER_SLOT_COUNT],
    started_at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum IncrementalRoomEnvKind {
    Shop,
    Hallway,
}

impl IncrementalRoomEnvKind {
    fn room(self) -> crate::room_gi_bake::RoomGiRoom {
        match self {
            Self::Shop => crate::room_gi_bake::RoomGiRoom::Shop,
            Self::Hallway => crate::room_gi_bake::RoomGiRoom::Hallway,
        }
    }

    fn scene_key(self) -> &'static str {
        match self {
            Self::Shop => scene_keys::SHOP,
            Self::Hallway => scene_keys::HALLWAY,
        }
    }

    fn label_prefix(self) -> &'static str {
        match self {
            Self::Shop => "shop",
            Self::Hallway => "hallway",
        }
    }

    fn bind_group_label(self) -> &'static str {
        match self {
            Self::Shop => "shop-env-bg",
            Self::Hallway => "hallway-env-bg",
        }
    }

    fn shadow_label(self) -> &'static str {
        match self {
            Self::Shop => "shop-env-shadow",
            Self::Hallway => "hallway-env-shadow",
        }
    }

    fn shadow_warp_label(self) -> &'static str {
        match self {
            Self::Shop => "shop-env-shadow-warp",
            Self::Hallway => "hallway-env-shadow-warp",
        }
    }

    fn shadow_mask_label(self) -> &'static str {
        match self {
            Self::Shop => "shop-env-shadow-mask",
            Self::Hallway => "hallway-env-shadow-mask",
        }
    }

    fn uniform_label(self) -> &'static str {
        match self {
            Self::Shop => "shop-env-uniform",
            Self::Hallway => "hallway-env-uniform",
        }
    }
}

pub(super) struct IncrementalRoomEnvGpuUpload {
    kind: IncrementalRoomEnvKind,
    prim_count: usize,
    next_prim: usize,
    prims: Vec<TilePrimitiveGpu>,
    shadow_mask_classes: Vec<crate::room_shadow_bake::PrimitiveContactAoClass>,
    room_tex_cache: RoomEnvTextureCache,
    _white_albedo_tex: wgpu::Texture,
    white_albedo_view: wgpu::TextureView,
    _decal_tex: wgpu::Texture,
    decal_view: wgpu::TextureView,
    shop_eyeball_prim_indices: Vec<usize>,
    started_at: Instant,
}

/// Commit a finished room GPU upload, or schedule CPU reload when mesh RAM was evicted
/// between the ready check and upload (low-memory LRU / warm-prefetch races).
fn try_commit_room_environment_gpu_upload(
    bit: u8,
    glb: &'static str,
    prims: Vec<TilePrimitiveGpu>,
    gpu_wrap: Option<ShopEnvironmentGpu>,
) -> Option<(Vec<TilePrimitiveGpu>, ShopEnvironmentGpu)> {
    if gpu_wrap.is_none() || prims.is_empty() {
        log::error!(
            "{glb} GPU upload skipped: CPU environment not resident (prefetch or eviction race); will retry"
        );
        RoomGpuResidentDesc::restart_cpu_after_gpu_miss(bit);
        return None;
    }
    Some((prims, gpu_wrap.unwrap()))
}

/// Rooms whose blocking-upload stall was already logged; cleared when the room recovers.
static BLOCKING_UPLOAD_STALL_LOGGED: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(0);

/// Whether a blocking (unlimited-budget) room upload loop can still make progress.
///
/// The upload tick returns `false` while the room's CPU mesh cache is absent; the
/// blocking drivers spin on the tick, so they must bail when no decode is in flight
/// and none can be queued (low-memory eviction drops the cache, and the re-queued
/// prefetch is refused while another room holds the only decode slot). Spinning in
/// that state hangs the main thread on a black screen.
fn blocking_room_upload_can_progress(id: RoomGpuResidentId) -> bool {
    use std::sync::atomic::Ordering;
    let desc = id.desc();
    if (desc.cpu_ready_for_gpu_upload)() {
        BLOCKING_UPLOAD_STALL_LOGGED.fetch_and(!desc.bit(), Ordering::Relaxed);
        return true;
    }
    (desc.start_cpu_prefetch)();
    if desc
        .prefetch_slot
        .is_some_and(crate::room_preload::room_prefetch_in_flight)
    {
        return true;
    }
    if BLOCKING_UPLOAD_STALL_LOGGED.fetch_or(desc.bit(), Ordering::Relaxed) & desc.bit() == 0 {
        log::warn!(
            "{} blocking GPU upload stalled: CPU cache not resident and no decode in flight; \
             deferring to per-frame retry",
            desc.glb,
        );
    }
    false
}

/// Low-memory GPU eviction: drop CPU mesh RAM (see [`RoomGpuResidentDesc::clear_cpu_cache_for_gpu_evict`]).
pub(crate) fn clear_room_cpu_cache_for_gpu_evict(bit: u8) {
    RoomGpuResidentDesc::clear_cpu_cache_for_gpu_evict(bit);
}

/// Re-queue CPU decode after low-memory eviction (see [`RoomGpuResidentDesc::restart_cpu_after_gpu_miss`]).
pub(crate) fn on_low_memory_room_gpu_evict_restart_prefetch(bit: u8) {
    RoomGpuResidentDesc::restart_cpu_after_gpu_miss(bit);
}

pub(super) struct RoomGpuUploadCtx<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub room_env_material_layout: &'a wgpu::BindGroupLayout,
    pub shadow_caster_layout: &'a wgpu::BindGroupLayout,
    pub room_shadow_mask_layout: &'a wgpu::BindGroupLayout,
    pub shadow_warp_layout: &'a wgpu::BindGroupLayout,
    pub tile_default_normal_view: &'a wgpu::TextureView,
    pub tile_glb_default_mr_view: &'a wgpu::TextureView,
    pub tile_glb_default_emissive_view: &'a wgpu::TextureView,
}

fn load_shop_room_gpu(
    ctx: RoomGpuUploadCtx<'_>,
) -> (
    Vec<TilePrimitiveGpu>,
    Option<ShopEnvironmentGpu>,
    crate::room_gltf_anim::RoomGltfAnimGpu,
    Vec<usize>,
) {
    crate::room_glb::with_shop_glb_cpu(|cpu_opt| {
        let mut prims = Vec::new();
        let mut gpu_wrap = None;
        let mut shop_gltf_anim = crate::room_gltf_anim::RoomGltfAnimGpu::default();
        let mut shop_eyeball_prim_indices = Vec::new();
        let Some(cpu) = cpu_opt else {
            return (prims, gpu_wrap, shop_gltf_anim, shop_eyeball_prim_indices);
        };
        shop_gltf_anim = crate::room_gltf_anim::RoomGltfAnimGpu::from_room_cpu(
            &cpu.gltf_anim_library,
            &cpu.environment_primitives,
            "shop.glb",
        );
        if !cpu.environment_primitives.is_empty() {
            let mut room_tex_cache = RoomEnvTextureCache::new();
            let (_white_tex, white_albedo_view) = white_albedo(ctx.device, ctx.queue);
            for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                if env_prim.gltf_node_name.as_deref() == Some("Eyeball") {
                    shop_eyeball_prim_indices.push(i);
                }
                let prim = &env_prim.mesh;
                let (vb, ib, index_count) =
                    create_room_env_mesh_buffers(ctx.device, "shop", i, prim);
                let mips = crate::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                let albedo_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("shop-env-albedo-{i}"),
                    env_prim,
                    RoomTextureUsageClass::BaseColorSrgb,
                    prim.albedo_rgba.as_deref(),
                    prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    &white_albedo_view,
                );
                let normal_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("shop-env-normal-{i}"),
                    env_prim,
                    RoomTextureUsageClass::NormalLinear,
                    prim.normal_rgba.as_deref(),
                    prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_default_normal_view,
                );
                let metallic_roughness_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("shop-env-mr-{i}"),
                    env_prim,
                    RoomTextureUsageClass::MetallicRoughnessLinear,
                    prim.metallic_roughness_rgba.as_deref(),
                    prim.metallic_roughness_mip_chain
                        .as_deref()
                        .map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_glb_default_mr_view,
                );
                let emissive_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("shop-env-emissive-{i}"),
                    env_prim,
                    RoomTextureUsageClass::EmissiveSrgb,
                    prim.emissive_rgba.as_deref(),
                    prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    ctx.tile_glb_default_emissive_view,
                );
                let pbr_uniform = room_env_pbr_uniform(
                    prim,
                    scene_keys::SHOP,
                    env_prim.gltf_node_name.as_deref(),
                    env_prim.material_name.as_deref(),
                );
                let pbr_uniform_buffer =
                    ctx.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("shop-pbr-{i}")),
                            contents: bytemuck::bytes_of(&pbr_uniform),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                let sampler = ctx
                    .device
                    .create_sampler(&build_sampler_descriptor(prim.sampler, None));
                prims.push(TilePrimitiveGpu {
                    vertex_buffer: vb,
                    index_buffer: ib,
                    index_count,
                    albedo_view,
                    normal_view,
                    metallic_roughness_view,
                    emissive_view,
                    pbr_uniform_buffer,
                    sampler,
                    pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                    material_bind_group: None,
                });
            }
            let (_white_tex, shop_decal_view) = white_albedo(ctx.device, ctx.queue);
            let uniform_buffers =
                create_room_env_camera_uniform_buffers(ctx.device, prims.len(), "shop-env-uniform");
            let distortion_buffer =
                ctx.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("shop-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
            let lightmap = upload_room_lightmap_gpu(
                &ctx,
                crate::room_gi_bake::RoomGiRoom::Shop,
                prims.len(),
                "shop-env-lightmap",
            );
            let bind_groups: Vec<wgpu::BindGroup> = prims
                .iter()
                .enumerate()
                .map(|(pi, p)| {
                    create_room_env_material_bind_group(
                        &ctx,
                        "shop-env-bg",
                        &uniform_buffers[pi],
                        p,
                        &shop_decal_view,
                        &distortion_buffer,
                        &lightmap.view,
                    )
                })
                .collect();
            let (shadow_uniform_buffers, shadow_bind_groups) = create_room_env_shadow_gpu_batch(
                ctx.device,
                ctx.shadow_caster_layout,
                prims.len(),
                "shop-env-shadow",
            );
            let (shadow_mask_uniform_buffers, shadow_mask_bind_groups) =
                create_room_shadow_mask_gpu_batch(
                    ctx.device,
                    ctx.room_shadow_mask_layout,
                    &room_shadow_mask_classes(
                        crate::room_gi_bake::RoomGiRoom::Shop,
                        &cpu.environment_primitives,
                    ),
                    "shop-env-shadow-mask",
                );
            let shadow_warp_bind_group = create_shadow_warp_bind_group(
                ctx.device,
                ctx.shadow_warp_layout,
                &distortion_buffer,
                "shop-env-shadow-warp",
            );
            gpu_wrap = Some(ShopEnvironmentGpu {
                uniform_buffers,
                distortion_buffer,
                shadow_uniform_buffers,
                shadow_bind_groups,
                _shadow_mask_uniform_buffers: shadow_mask_uniform_buffers,
                shadow_mask_bind_groups,
                shadow_warp_bind_group,
                bind_groups,
                lightmap_uv_rects: lightmap.uv_rects,
                archive_sign_decal_texture: None,
                archive_sign_decal_size: None,
                archive_inspect_plaque_decal_texture: None,
                archive_inspect_plaque_decal_size: None,
            });
            if shop_eyeball_prim_indices.is_empty() {
                if let Some(bindings) = shop_gltf_anim.clip_prim_bindings.get("eyeball_travel") {
                    shop_eyeball_prim_indices = bindings.iter().map(|(pi, _)| *pi).collect();
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
}

fn load_hallway_room_gpu(
    ctx: RoomGpuUploadCtx<'_>,
) -> (Vec<TilePrimitiveGpu>, Option<ShopEnvironmentGpu>) {
    crate::hallway_glb::with_hallway_glb_cpu(|cpu_opt| {
        let mut prims = Vec::new();
        let mut gpu_wrap = None;
        let Some(cpu) = cpu_opt else {
            return (prims, gpu_wrap);
        };
        if !cpu.environment_primitives.is_empty() {
            let mut room_tex_cache = RoomEnvTextureCache::new();
            let (_white_tex, white_albedo_view) = white_albedo(ctx.device, ctx.queue);
            for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                let prim = &env_prim.mesh;
                let (vb, ib, index_count) =
                    create_room_env_mesh_buffers(ctx.device, "hallway", i, prim);
                let mips = crate::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                let albedo_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("hallway-env-albedo-{i}"),
                    env_prim,
                    RoomTextureUsageClass::BaseColorSrgb,
                    prim.albedo_rgba.as_deref(),
                    prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    &white_albedo_view,
                );
                let normal_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("hallway-env-normal-{i}"),
                    env_prim,
                    RoomTextureUsageClass::NormalLinear,
                    prim.normal_rgba.as_deref(),
                    prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_default_normal_view,
                );
                let metallic_roughness_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("hallway-env-mr-{i}"),
                    env_prim,
                    RoomTextureUsageClass::MetallicRoughnessLinear,
                    prim.metallic_roughness_rgba.as_deref(),
                    prim.metallic_roughness_mip_chain
                        .as_deref()
                        .map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_glb_default_mr_view,
                );
                let emissive_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("hallway-env-emissive-{i}"),
                    env_prim,
                    RoomTextureUsageClass::EmissiveSrgb,
                    prim.emissive_rgba.as_deref(),
                    prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    ctx.tile_glb_default_emissive_view,
                );
                let pbr_uniform = room_env_pbr_uniform(
                    prim,
                    scene_keys::HALLWAY,
                    env_prim.gltf_node_name.as_deref(),
                    env_prim.material_name.as_deref(),
                );
                let pbr_uniform_buffer =
                    ctx.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("hallway-pbr-{i}")),
                            contents: bytemuck::bytes_of(&pbr_uniform),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                let sampler = ctx
                    .device
                    .create_sampler(&build_sampler_descriptor(prim.sampler, None));
                prims.push(TilePrimitiveGpu {
                    vertex_buffer: vb,
                    index_buffer: ib,
                    index_count,
                    albedo_view,
                    normal_view,
                    metallic_roughness_view,
                    emissive_view,
                    pbr_uniform_buffer,
                    sampler,
                    pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                    material_bind_group: None,
                });
            }
            let (_white_tex, hallway_decal_view) = white_albedo(ctx.device, ctx.queue);
            let uniform_buffers = create_room_env_camera_uniform_buffers(
                ctx.device,
                prims.len(),
                "hallway-env-uniform",
            );
            let distortion_buffer =
                ctx.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("hallway-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
            let lightmap = upload_room_lightmap_gpu(
                &ctx,
                crate::room_gi_bake::RoomGiRoom::Hallway,
                prims.len(),
                "hallway-env-lightmap",
            );
            let bind_groups: Vec<wgpu::BindGroup> = prims
                .iter()
                .enumerate()
                .map(|(pi, p)| {
                    create_room_env_material_bind_group(
                        &ctx,
                        "hallway-env-bg",
                        &uniform_buffers[pi],
                        p,
                        &hallway_decal_view,
                        &distortion_buffer,
                        &lightmap.view,
                    )
                })
                .collect();
            let (shadow_uniform_buffers, shadow_bind_groups) = create_room_env_shadow_gpu_batch(
                ctx.device,
                ctx.shadow_caster_layout,
                prims.len(),
                "hallway-env-shadow",
            );
            let (shadow_mask_uniform_buffers, shadow_mask_bind_groups) =
                create_room_shadow_mask_gpu_batch(
                    ctx.device,
                    ctx.room_shadow_mask_layout,
                    &room_shadow_mask_classes(
                        crate::room_gi_bake::RoomGiRoom::Hallway,
                        &cpu.environment_primitives,
                    ),
                    "hallway-env-shadow-mask",
                );
            let shadow_warp_bind_group = create_shadow_warp_bind_group(
                ctx.device,
                ctx.shadow_warp_layout,
                &distortion_buffer,
                "hallway-env-shadow-warp",
            );
            gpu_wrap = Some(ShopEnvironmentGpu {
                uniform_buffers,
                distortion_buffer,
                shadow_uniform_buffers,
                shadow_bind_groups,
                _shadow_mask_uniform_buffers: shadow_mask_uniform_buffers,
                shadow_mask_bind_groups,
                shadow_warp_bind_group,
                bind_groups,
                lightmap_uv_rects: lightmap.uv_rects,
                archive_sign_decal_texture: None,
                archive_sign_decal_size: None,
                archive_inspect_plaque_decal_texture: None,
                archive_inspect_plaque_decal_size: None,
            });
            log::info!("hallway.glb GPU: {} primitive draw(s)", prims.len());
        }
        (prims, gpu_wrap)
    })
}

fn load_main_menu_room_gpu(
    ctx: RoomGpuUploadCtx<'_>,
) -> (Vec<TilePrimitiveGpu>, Option<ShopEnvironmentGpu>) {
    crate::main_menu_glb::with_main_menu_glb_cpu(|cpu_opt| {
        let mut prims = Vec::new();
        let mut gpu_wrap = None;
        let Some(cpu) = cpu_opt else {
            return (prims, gpu_wrap);
        };
        if !cpu.environment_primitives.is_empty() {
            let mut room_tex_cache = RoomEnvTextureCache::new();
            let (_white_tex, white_albedo_view) = white_albedo(ctx.device, ctx.queue);
            for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                let prim = &env_prim.mesh;
                let (vb, ib, index_count) =
                    create_room_env_mesh_buffers(ctx.device, "main_menu", i, prim);
                let mips = crate::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                let albedo_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("main_menu-env-albedo-{i}"),
                    env_prim,
                    RoomTextureUsageClass::BaseColorSrgb,
                    prim.albedo_rgba.as_deref(),
                    prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    &white_albedo_view,
                );
                let normal_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("main_menu-env-normal-{i}"),
                    env_prim,
                    RoomTextureUsageClass::NormalLinear,
                    prim.normal_rgba.as_deref(),
                    prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_default_normal_view,
                );
                let metallic_roughness_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("main_menu-env-mr-{i}"),
                    env_prim,
                    RoomTextureUsageClass::MetallicRoughnessLinear,
                    prim.metallic_roughness_rgba.as_deref(),
                    prim.metallic_roughness_mip_chain
                        .as_deref()
                        .map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_glb_default_mr_view,
                );
                let emissive_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("main_menu-env-emissive-{i}"),
                    env_prim,
                    RoomTextureUsageClass::EmissiveSrgb,
                    prim.emissive_rgba.as_deref(),
                    prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    ctx.tile_glb_default_emissive_view,
                );
                let pbr_uniform = room_env_pbr_uniform(
                    prim,
                    scene_keys::MAIN_MENU,
                    env_prim.gltf_node_name.as_deref(),
                    env_prim.material_name.as_deref(),
                );
                let pbr_uniform_buffer =
                    ctx.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("main_menu-pbr-{i}")),
                            contents: bytemuck::bytes_of(&pbr_uniform),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                let sampler = ctx
                    .device
                    .create_sampler(&build_sampler_descriptor(prim.sampler, None));
                prims.push(TilePrimitiveGpu {
                    vertex_buffer: vb,
                    index_buffer: ib,
                    index_count,
                    albedo_view,
                    normal_view,
                    metallic_roughness_view,
                    emissive_view,
                    pbr_uniform_buffer,
                    sampler,
                    pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                    material_bind_group: None,
                });
            }
            let (_white_tex, main_menu_decal_view) = white_albedo(ctx.device, ctx.queue);
            let uniform_buffers = create_room_env_camera_uniform_buffers(
                ctx.device,
                prims.len(),
                "main_menu-env-uniform",
            );
            let distortion_buffer =
                ctx.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("main_menu-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
            let lightmap = upload_room_lightmap_gpu(
                &ctx,
                crate::room_gi_bake::RoomGiRoom::MainMenu,
                prims.len(),
                "main-menu-env-lightmap",
            );
            let bind_groups: Vec<wgpu::BindGroup> = prims
                .iter()
                .enumerate()
                .map(|(pi, p)| {
                    create_room_env_material_bind_group(
                        &ctx,
                        "main_menu-env-bg",
                        &uniform_buffers[pi],
                        p,
                        &main_menu_decal_view,
                        &distortion_buffer,
                        &lightmap.view,
                    )
                })
                .collect();
            let (shadow_uniform_buffers, shadow_bind_groups) = create_room_env_shadow_gpu_batch(
                ctx.device,
                ctx.shadow_caster_layout,
                prims.len(),
                "main_menu-env-shadow",
            );
            let (shadow_mask_uniform_buffers, shadow_mask_bind_groups) =
                create_room_shadow_mask_gpu_batch(
                    ctx.device,
                    ctx.room_shadow_mask_layout,
                    &room_shadow_mask_classes(
                        crate::room_gi_bake::RoomGiRoom::MainMenu,
                        &cpu.environment_primitives,
                    ),
                    "main_menu-env-shadow-mask",
                );
            let shadow_warp_bind_group = create_shadow_warp_bind_group(
                ctx.device,
                ctx.shadow_warp_layout,
                &distortion_buffer,
                "main_menu-env-shadow-warp",
            );
            gpu_wrap = Some(ShopEnvironmentGpu {
                uniform_buffers,
                distortion_buffer,
                shadow_uniform_buffers,
                shadow_bind_groups,
                _shadow_mask_uniform_buffers: shadow_mask_uniform_buffers,
                shadow_mask_bind_groups,
                shadow_warp_bind_group,
                bind_groups,
                lightmap_uv_rects: lightmap.uv_rects,
                archive_sign_decal_texture: None,
                archive_sign_decal_size: None,
                archive_inspect_plaque_decal_texture: None,
                archive_inspect_plaque_decal_size: None,
            });
            log::info!("main_menu.glb GPU: {} primitive draw(s)", prims.len());
        }
        (prims, gpu_wrap)
    })
}

fn load_staircase_room_gpu(
    ctx: RoomGpuUploadCtx<'_>,
) -> (Vec<TilePrimitiveGpu>, Option<ShopEnvironmentGpu>) {
    crate::staircase_glb::with_staircase_glb_cpu(|cpu_opt| {
        let mut prims = Vec::new();
        let mut gpu_wrap = None;
        let Some(cpu) = cpu_opt else {
            return (prims, gpu_wrap);
        };
        if !cpu.environment_primitives.is_empty() {
            let mut room_tex_cache = RoomEnvTextureCache::new();
            let (_white_tex, white_albedo_view) = white_albedo(ctx.device, ctx.queue);
            for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                let prim = &env_prim.mesh;
                let (vb, ib, index_count) =
                    create_room_env_mesh_buffers(ctx.device, "staircase", i, prim);
                let mips = crate::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                let albedo_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("staircase-env-albedo-{i}"),
                    env_prim,
                    RoomTextureUsageClass::BaseColorSrgb,
                    prim.albedo_rgba.as_deref(),
                    prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    &white_albedo_view,
                );
                let normal_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("staircase-env-normal-{i}"),
                    env_prim,
                    RoomTextureUsageClass::NormalLinear,
                    prim.normal_rgba.as_deref(),
                    prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_default_normal_view,
                );
                let metallic_roughness_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("staircase-env-mr-{i}"),
                    env_prim,
                    RoomTextureUsageClass::MetallicRoughnessLinear,
                    prim.metallic_roughness_rgba.as_deref(),
                    prim.metallic_roughness_mip_chain
                        .as_deref()
                        .map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_glb_default_mr_view,
                );
                let emissive_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("staircase-env-emissive-{i}"),
                    env_prim,
                    RoomTextureUsageClass::EmissiveSrgb,
                    prim.emissive_rgba.as_deref(),
                    prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    ctx.tile_glb_default_emissive_view,
                );
                let pbr_uniform = room_env_pbr_uniform(
                    prim,
                    scene_keys::STAIRWAY,
                    env_prim.gltf_node_name.as_deref(),
                    env_prim.material_name.as_deref(),
                );
                let pbr_uniform_buffer =
                    ctx.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("staircase-pbr-{i}")),
                            contents: bytemuck::bytes_of(&pbr_uniform),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                let sampler = ctx
                    .device
                    .create_sampler(&build_sampler_descriptor(prim.sampler, None));
                prims.push(TilePrimitiveGpu {
                    vertex_buffer: vb,
                    index_buffer: ib,
                    index_count,
                    albedo_view,
                    normal_view,
                    metallic_roughness_view,
                    emissive_view,
                    pbr_uniform_buffer,
                    sampler,
                    pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                    material_bind_group: None,
                });
            }
            let (_white_tex, staircase_decal_view) = white_albedo(ctx.device, ctx.queue);
            let uniform_buffers = create_room_env_camera_uniform_buffers(
                ctx.device,
                prims.len(),
                "staircase-env-uniform",
            );
            let distortion_buffer =
                ctx.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("staircase-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
            let lightmap = upload_room_lightmap_gpu(
                &ctx,
                crate::room_gi_bake::RoomGiRoom::Stairway,
                prims.len(),
                "staircase-env-lightmap",
            );
            let bind_groups: Vec<wgpu::BindGroup> = prims
                .iter()
                .enumerate()
                .map(|(pi, p)| {
                    create_room_env_material_bind_group(
                        &ctx,
                        "staircase-env-bg",
                        &uniform_buffers[pi],
                        p,
                        &staircase_decal_view,
                        &distortion_buffer,
                        &lightmap.view,
                    )
                })
                .collect();
            let (shadow_uniform_buffers, shadow_bind_groups) = create_room_env_shadow_gpu_batch(
                ctx.device,
                ctx.shadow_caster_layout,
                prims.len(),
                "staircase-env-shadow",
            );
            let (shadow_mask_uniform_buffers, shadow_mask_bind_groups) =
                create_room_shadow_mask_gpu_batch(
                    ctx.device,
                    ctx.room_shadow_mask_layout,
                    &room_shadow_mask_classes(
                        crate::room_gi_bake::RoomGiRoom::Stairway,
                        &cpu.environment_primitives,
                    ),
                    "staircase-env-shadow-mask",
                );
            let shadow_warp_bind_group = create_shadow_warp_bind_group(
                ctx.device,
                ctx.shadow_warp_layout,
                &distortion_buffer,
                "staircase-env-shadow-warp",
            );
            gpu_wrap = Some(ShopEnvironmentGpu {
                uniform_buffers,
                distortion_buffer,
                shadow_uniform_buffers,
                shadow_bind_groups,
                _shadow_mask_uniform_buffers: shadow_mask_uniform_buffers,
                shadow_mask_bind_groups,
                shadow_warp_bind_group,
                bind_groups,
                lightmap_uv_rects: lightmap.uv_rects,
                archive_sign_decal_texture: None,
                archive_sign_decal_size: None,
                archive_inspect_plaque_decal_texture: None,
                archive_inspect_plaque_decal_size: None,
            });
            log::info!("staircase.glb GPU: {} primitive draw(s)", prims.len());
        }
        (prims, gpu_wrap)
    })
}

fn load_shadow_test_room_gpu(
    ctx: RoomGpuUploadCtx<'_>,
) -> (Vec<TilePrimitiveGpu>, Option<ShopEnvironmentGpu>) {
    crate::shadow_test_room_glb::with_shadow_test_room_glb_cpu(|cpu_opt| {
        let mut prims = Vec::new();
        let mut gpu_wrap = None;
        let Some(cpu) = cpu_opt else {
            return (prims, gpu_wrap);
        };
        if !cpu.environment_primitives.is_empty() {
            let mut room_tex_cache = RoomEnvTextureCache::new();
            let (_white_tex, white_albedo_view) = white_albedo(ctx.device, ctx.queue);
            for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                let prim = &env_prim.mesh;
                let (vb, ib, index_count) =
                    create_room_env_mesh_buffers(ctx.device, "shadow-test-room", i, prim);
                let mips = crate::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                let albedo_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("shadow-test-room-env-albedo-{i}"),
                    env_prim,
                    RoomTextureUsageClass::BaseColorSrgb,
                    prim.albedo_rgba.as_deref(),
                    prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    &white_albedo_view,
                );
                let normal_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("shadow-test-room-env-normal-{i}"),
                    env_prim,
                    RoomTextureUsageClass::NormalLinear,
                    prim.normal_rgba.as_deref(),
                    prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_default_normal_view,
                );
                let metallic_roughness_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("shadow-test-room-env-mr-{i}"),
                    env_prim,
                    RoomTextureUsageClass::MetallicRoughnessLinear,
                    prim.metallic_roughness_rgba.as_deref(),
                    prim.metallic_roughness_mip_chain
                        .as_deref()
                        .map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_glb_default_mr_view,
                );
                let emissive_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("shadow-test-room-env-emissive-{i}"),
                    env_prim,
                    RoomTextureUsageClass::EmissiveSrgb,
                    prim.emissive_rgba.as_deref(),
                    prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    ctx.tile_glb_default_emissive_view,
                );
                let pbr_uniform = room_env_pbr_uniform(
                    prim,
                    scene_keys::SHADOW_AO_LAB,
                    env_prim.gltf_node_name.as_deref(),
                    env_prim.material_name.as_deref(),
                );
                let pbr_uniform_buffer =
                    ctx.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("shadow-test-room-pbr-{i}")),
                            contents: bytemuck::bytes_of(&pbr_uniform),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                let sampler = ctx
                    .device
                    .create_sampler(&build_sampler_descriptor(prim.sampler, None));
                prims.push(TilePrimitiveGpu {
                    vertex_buffer: vb,
                    index_buffer: ib,
                    index_count,
                    albedo_view,
                    normal_view,
                    metallic_roughness_view,
                    emissive_view,
                    pbr_uniform_buffer,
                    sampler,
                    pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                    material_bind_group: None,
                });
            }
            let (_white_tex, shadow_test_room_decal_view) = white_albedo(ctx.device, ctx.queue);
            let uniform_buffers = create_room_env_camera_uniform_buffers(
                ctx.device,
                prims.len(),
                "shadow-test-room-env-uniform",
            );
            let distortion_buffer =
                ctx.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("shadow-test-room-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
            let lightmap = upload_room_lightmap_gpu(
                &ctx,
                crate::room_gi_bake::RoomGiRoom::ShadowTestRoom,
                prims.len(),
                "shadow-test-room-env-lightmap",
            );
            let bind_groups: Vec<wgpu::BindGroup> = prims
                .iter()
                .enumerate()
                .map(|(pi, p)| {
                    create_room_env_material_bind_group(
                        &ctx,
                        "shadow-test-room-env-bg",
                        &uniform_buffers[pi],
                        p,
                        &shadow_test_room_decal_view,
                        &distortion_buffer,
                        &lightmap.view,
                    )
                })
                .collect();
            let (shadow_uniform_buffers, shadow_bind_groups) = create_room_env_shadow_gpu_batch(
                ctx.device,
                ctx.shadow_caster_layout,
                prims.len(),
                "shadow-test-room-env-shadow",
            );
            let (shadow_mask_uniform_buffers, shadow_mask_bind_groups) =
                create_room_shadow_mask_gpu_batch(
                    ctx.device,
                    ctx.room_shadow_mask_layout,
                    &room_shadow_mask_classes(
                        crate::room_gi_bake::RoomGiRoom::ShadowTestRoom,
                        &cpu.environment_primitives,
                    ),
                    "shadow-test-room-env-shadow-mask",
                );
            let shadow_warp_bind_group = create_shadow_warp_bind_group(
                ctx.device,
                ctx.shadow_warp_layout,
                &distortion_buffer,
                "shadow-test-room-env-shadow-warp",
            );
            gpu_wrap = Some(ShopEnvironmentGpu {
                uniform_buffers,
                distortion_buffer,
                shadow_uniform_buffers,
                shadow_bind_groups,
                _shadow_mask_uniform_buffers: shadow_mask_uniform_buffers,
                shadow_mask_bind_groups,
                shadow_warp_bind_group,
                bind_groups,
                lightmap_uv_rects: lightmap.uv_rects,
                archive_sign_decal_texture: None,
                archive_sign_decal_size: None,
                archive_inspect_plaque_decal_texture: None,
                archive_inspect_plaque_decal_size: None,
            });
            log::info!(
                "shadow_test_room.glb GPU: {} primitive draw(s)",
                prims.len()
            );
        }
        (prims, gpu_wrap)
    })
}

fn create_cleared_archive_decal_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    width: u32,
    height: u32,
) -> wgpu::Texture {
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
    let bytes_per_row = crate::wgpu_renderer::resources::rgba8_copy_bytes_per_row(width);
    let clear = vec![0u8; (bytes_per_row * height) as usize];
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &clear,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_row),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    tex
}

fn load_archive_room_gpu(
    ctx: RoomGpuUploadCtx<'_>,
) -> (
    Vec<TilePrimitiveGpu>,
    Option<ShopEnvironmentGpu>,
    Option<usize>,
    Option<usize>,
    Option<usize>,
    Option<usize>,
    Vec<usize>,
    Vec<usize>,
) {
    crate::archive_glb::with_archive_glb_cpu(|cpu_opt| {
        let mut prims = Vec::new();
        let mut gpu_wrap = None;
        let mut sign_l = None;
        let mut sign_r = None;
        let mut inspect_plaque = None;
        let mut plaque_backing = None;
        let mut page_left = Vec::new();
        let mut page_right = Vec::new();
        let Some(cpu) = cpu_opt else {
            return (
                prims,
                gpu_wrap,
                sign_l,
                sign_r,
                inspect_plaque,
                plaque_backing,
                page_left,
                page_right,
            );
        };
        if !cpu.environment_primitives.is_empty() {
            let mut room_tex_cache = RoomEnvTextureCache::new();
            let (_white_tex, white_albedo_view) = white_albedo(ctx.device, ctx.queue);
            for (i, env_prim) in cpu.environment_primitives.iter().enumerate() {
                if let Some(ref name) = env_prim.gltf_node_name {
                    if name == crate::archive_glb::SIGN_DESCRIPTION_LEFT {
                        sign_l = Some(i);
                    } else if name == crate::archive_glb::SIGN_DESCRIPTION_RIGHT {
                        sign_r = Some(i);
                    } else if name == crate::archive_glb::INSPECT_PLAQUE {
                        inspect_plaque = Some(i);
                    } else if name == crate::archive_glb::PLAQUE_BACKING {
                        plaque_backing = Some(i);
                    } else if name == crate::archive_glb::BTN_PAGE_LEFT {
                        page_left.push(i);
                    } else if name == crate::archive_glb::BTN_PAGE_RIGHT {
                        page_right.push(i);
                    }
                }
                let prim = &env_prim.mesh;
                let (vb, ib, index_count) =
                    create_room_env_mesh_buffers(ctx.device, "archive", i, prim);
                let mips = crate::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
                let albedo_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("archive-env-albedo-{i}"),
                    env_prim,
                    RoomTextureUsageClass::BaseColorSrgb,
                    prim.albedo_rgba.as_deref(),
                    prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    &white_albedo_view,
                );
                let normal_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("archive-env-normal-{i}"),
                    env_prim,
                    RoomTextureUsageClass::NormalLinear,
                    prim.normal_rgba.as_deref(),
                    prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_default_normal_view,
                );
                let metallic_roughness_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("archive-env-mr-{i}"),
                    env_prim,
                    RoomTextureUsageClass::MetallicRoughnessLinear,
                    prim.metallic_roughness_rgba.as_deref(),
                    prim.metallic_roughness_mip_chain
                        .as_deref()
                        .map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8Unorm,
                    mips,
                    ctx.tile_glb_default_mr_view,
                );
                let emissive_view = upload_room_texture_slot(
                    &mut room_tex_cache,
                    &ctx,
                    format!("archive-env-emissive-{i}"),
                    env_prim,
                    RoomTextureUsageClass::EmissiveSrgb,
                    prim.emissive_rgba.as_deref(),
                    prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    mips,
                    ctx.tile_glb_default_emissive_view,
                );
                let pbr_uniform = room_env_pbr_uniform(
                    prim,
                    scene_keys::ARCHIVE,
                    env_prim.gltf_node_name.as_deref(),
                    env_prim.material_name.as_deref(),
                );
                let pbr_uniform_buffer =
                    ctx.device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some(&format!("archive-pbr-{i}")),
                            contents: bytemuck::bytes_of(&pbr_uniform),
                            usage: wgpu::BufferUsages::UNIFORM,
                        });
                let sampler = ctx
                    .device
                    .create_sampler(&build_sampler_descriptor(prim.sampler, None));
                prims.push(TilePrimitiveGpu {
                    vertex_buffer: vb,
                    index_buffer: ib,
                    index_count,
                    albedo_view,
                    normal_view,
                    metallic_roughness_view,
                    emissive_view,
                    pbr_uniform_buffer,
                    sampler,
                    pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
                    material_bind_group: None,
                });
            }
            let decal_layout = crate::primitive::DecalLayout::Fit {
                target_short_edge: crate::decal::PLAQUE_DECAL_HEIGHT,
            };
            let (sign_decal_w, sign_decal_h) = crate::decal::decal_dimensions(
                &decal_layout,
                crate::archive_glb::archive_sign_description_decal_extents_for(cpu),
            );
            let (inspect_decal_w, inspect_decal_h) = crate::decal::decal_dimensions(
                &decal_layout,
                crate::archive_glb::archive_inspect_plaque_decal_extents_for(cpu),
            );
            let archive_sign_decal_tex = create_cleared_archive_decal_texture(
                ctx.device,
                ctx.queue,
                "archive-sign-decal",
                sign_decal_w,
                sign_decal_h,
            );
            let archive_inspect_decal_tex = create_cleared_archive_decal_texture(
                ctx.device,
                ctx.queue,
                "archive-inspect-plaque-decal",
                inspect_decal_w,
                inspect_decal_h,
            );
            let archive_sign_decal_view =
                archive_sign_decal_tex.create_view(&wgpu::TextureViewDescriptor::default());
            let archive_inspect_decal_view =
                archive_inspect_decal_tex.create_view(&wgpu::TextureViewDescriptor::default());
            let uniform_buffers = create_room_env_camera_uniform_buffers(
                ctx.device,
                prims.len(),
                "archive-env-uniform",
            );
            let distortion_buffer =
                ctx.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("archive-env-distortion"),
                        contents: bytemuck::bytes_of(
                            &crate::hallway_glb::HallwayDistortion::default(),
                        ),
                        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    });
            let lightmap = upload_room_lightmap_gpu(
                &ctx,
                crate::room_gi_bake::RoomGiRoom::Archive,
                prims.len(),
                "archive-env-lightmap",
            );
            let bind_groups: Vec<wgpu::BindGroup> = prims
                .iter()
                .enumerate()
                .map(|(pi, p)| {
                    let decal_view = if sign_l == Some(pi) || sign_r == Some(pi) {
                        &archive_sign_decal_view
                    } else if inspect_plaque == Some(pi) {
                        &archive_inspect_decal_view
                    } else {
                        &white_albedo_view
                    };
                    create_room_env_material_bind_group(
                        &ctx,
                        "archive-env-bg",
                        &uniform_buffers[pi],
                        p,
                        decal_view,
                        &distortion_buffer,
                        &lightmap.view,
                    )
                })
                .collect();
            let (shadow_uniform_buffers, shadow_bind_groups) = create_room_env_shadow_gpu_batch(
                ctx.device,
                ctx.shadow_caster_layout,
                prims.len(),
                "archive-env-shadow",
            );
            let (shadow_mask_uniform_buffers, shadow_mask_bind_groups) =
                create_room_shadow_mask_gpu_batch(
                    ctx.device,
                    ctx.room_shadow_mask_layout,
                    &room_shadow_mask_classes(
                        crate::room_gi_bake::RoomGiRoom::Archive,
                        &cpu.environment_primitives,
                    ),
                    "archive-env-shadow-mask",
                );
            let shadow_warp_bind_group = create_shadow_warp_bind_group(
                ctx.device,
                ctx.shadow_warp_layout,
                &distortion_buffer,
                "archive-env-shadow-warp",
            );
            gpu_wrap = Some(ShopEnvironmentGpu {
                uniform_buffers,
                distortion_buffer,
                shadow_uniform_buffers,
                shadow_bind_groups,
                _shadow_mask_uniform_buffers: shadow_mask_uniform_buffers,
                shadow_mask_bind_groups,
                shadow_warp_bind_group,
                bind_groups,
                lightmap_uv_rects: lightmap.uv_rects,
                archive_sign_decal_texture: Some(archive_sign_decal_tex),
                archive_sign_decal_size: Some((sign_decal_w, sign_decal_h)),
                archive_inspect_plaque_decal_texture: Some(archive_inspect_decal_tex),
                archive_inspect_plaque_decal_size: Some((inspect_decal_w, inspect_decal_h)),
            });
            log::info!("archive.glb GPU: {} primitive draw(s)", prims.len());
        }
        (
            prims,
            gpu_wrap,
            sign_l,
            sign_r,
            inspect_plaque,
            plaque_backing,
            page_left,
            page_right,
        )
    })
}

fn begin_incremental_room_env_gpu_upload(
    kind: IncrementalRoomEnvKind,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Option<IncrementalRoomEnvGpuUpload> {
    let (prim_count, shadow_mask_classes) = match kind {
        IncrementalRoomEnvKind::Shop => crate::room_glb::with_shop_glb_cpu(|cpu_opt| {
            cpu_opt.and_then(|cpu| {
                let n = cpu.environment_primitives.len();
                (n > 0).then(|| {
                    (
                        n,
                        room_shadow_mask_classes(kind.room(), &cpu.environment_primitives),
                    )
                })
            })
        })?,
        IncrementalRoomEnvKind::Hallway => crate::hallway_glb::with_hallway_glb_cpu(|cpu_opt| {
            cpu_opt.and_then(|cpu| {
                let n = cpu.environment_primitives.len();
                (n > 0).then(|| {
                    (
                        n,
                        room_shadow_mask_classes(kind.room(), &cpu.environment_primitives),
                    )
                })
            })
        })?,
    };
    let (white_tex, white_view) = white_albedo(device, queue);
    let (decal_tex, decal_view) = white_albedo(device, queue);
    Some(IncrementalRoomEnvGpuUpload {
        kind,
        prim_count,
        next_prim: 0,
        prims: Vec::with_capacity(prim_count),
        shadow_mask_classes,
        room_tex_cache: RoomEnvTextureCache::new(),
        _white_albedo_tex: white_tex,
        white_albedo_view: white_view,
        _decal_tex: decal_tex,
        decal_view,
        shop_eyeball_prim_indices: Vec::new(),
        started_at: Instant::now(),
    })
}

fn upload_incremental_room_env_prim_gpu(
    kind: IncrementalRoomEnvKind,
    i: usize,
    env_prim: &RoomEnvPrimitiveCpu,
    ctx: &RoomGpuUploadCtx<'_>,
    white_albedo_view: &wgpu::TextureView,
    room_tex_cache: &mut RoomEnvTextureCache,
) -> TilePrimitiveGpu {
    let prim = &env_prim.mesh;
    let label_prefix = kind.label_prefix();
    let (vb, ib, index_count) = create_room_env_mesh_buffers(ctx.device, label_prefix, i, prim);
    let mips = crate::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
    let albedo_view = upload_room_texture_slot(
        room_tex_cache,
        ctx,
        format!("{label_prefix}-env-albedo-{i}"),
        env_prim,
        RoomTextureUsageClass::BaseColorSrgb,
        prim.albedo_rgba.as_deref(),
        prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
        wgpu::TextureFormat::Rgba8UnormSrgb,
        mips,
        white_albedo_view,
    );
    let normal_view = upload_room_texture_slot(
        room_tex_cache,
        ctx,
        format!("{label_prefix}-env-normal-{i}"),
        env_prim,
        RoomTextureUsageClass::NormalLinear,
        prim.normal_rgba.as_deref(),
        prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
        wgpu::TextureFormat::Rgba8Unorm,
        mips,
        ctx.tile_default_normal_view,
    );
    let metallic_roughness_view = upload_room_texture_slot(
        room_tex_cache,
        ctx,
        format!("{label_prefix}-env-mr-{i}"),
        env_prim,
        RoomTextureUsageClass::MetallicRoughnessLinear,
        prim.metallic_roughness_rgba.as_deref(),
        prim.metallic_roughness_mip_chain
            .as_deref()
            .map(|c| c.as_slice()),
        wgpu::TextureFormat::Rgba8Unorm,
        mips,
        ctx.tile_glb_default_mr_view,
    );
    let emissive_view = upload_room_texture_slot(
        room_tex_cache,
        ctx,
        format!("{label_prefix}-env-emissive-{i}"),
        env_prim,
        RoomTextureUsageClass::EmissiveSrgb,
        prim.emissive_rgba.as_deref(),
        prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
        wgpu::TextureFormat::Rgba8UnormSrgb,
        mips,
        ctx.tile_glb_default_emissive_view,
    );
    let pbr_uniform = room_env_pbr_uniform(
        prim,
        kind.scene_key(),
        env_prim.gltf_node_name.as_deref(),
        env_prim.material_name.as_deref(),
    );
    let pbr_uniform_buffer = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label_prefix}-pbr-{i}")),
            contents: bytemuck::bytes_of(&pbr_uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let sampler = ctx
        .device
        .create_sampler(&build_sampler_descriptor(prim.sampler, None));
    TilePrimitiveGpu {
        vertex_buffer: vb,
        index_buffer: ib,
        index_count,
        albedo_view,
        normal_view,
        metallic_roughness_view,
        emissive_view,
        pbr_uniform_buffer,
        sampler,
        pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
        material_bind_group: None,
    }
}

fn finalize_incremental_room_env_gpu_upload(
    upload: IncrementalRoomEnvGpuUpload,
    ctx: RoomGpuUploadCtx<'_>,
) -> (Vec<TilePrimitiveGpu>, Option<ShopEnvironmentGpu>) {
    let IncrementalRoomEnvGpuUpload {
        kind,
        prim_count,
        prims,
        shadow_mask_classes,
        decal_view,
        ..
    } = upload;
    debug_assert_eq!(prims.len(), prim_count);
    let label_prefix = kind.label_prefix();
    let distortion_buffer = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label_prefix}-env-distortion")),
            contents: bytemuck::bytes_of(&crate::hallway_glb::HallwayDistortion::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
    let uniform_buffers =
        create_room_env_camera_uniform_buffers(ctx.device, prim_count, kind.uniform_label());
    let lightmap = upload_room_lightmap_gpu(
        &ctx,
        kind.room(),
        prim_count,
        &format!("{}-env-lightmap", label_prefix),
    );
    let bind_groups: Vec<wgpu::BindGroup> = prims
        .iter()
        .enumerate()
        .map(|(pi, p)| {
            create_room_env_material_bind_group(
                &ctx,
                kind.bind_group_label(),
                &uniform_buffers[pi],
                p,
                &decal_view,
                &distortion_buffer,
                &lightmap.view,
            )
        })
        .collect();
    let (shadow_uniform_buffers, shadow_bind_groups) = create_room_env_shadow_gpu_batch(
        ctx.device,
        ctx.shadow_caster_layout,
        prim_count,
        kind.shadow_label(),
    );
    let (shadow_mask_uniform_buffers, shadow_mask_bind_groups) = create_room_shadow_mask_gpu_batch(
        ctx.device,
        ctx.room_shadow_mask_layout,
        &shadow_mask_classes,
        kind.shadow_mask_label(),
    );
    let shadow_warp_bind_group = create_shadow_warp_bind_group(
        ctx.device,
        ctx.shadow_warp_layout,
        &distortion_buffer,
        kind.shadow_warp_label(),
    );
    (
        prims,
        Some(ShopEnvironmentGpu {
            uniform_buffers,
            distortion_buffer,
            shadow_uniform_buffers,
            shadow_bind_groups,
            _shadow_mask_uniform_buffers: shadow_mask_uniform_buffers,
            shadow_mask_bind_groups,
            shadow_warp_bind_group,
            bind_groups,
            lightmap_uv_rects: lightmap.uv_rects,
            archive_sign_decal_texture: None,
            archive_sign_decal_size: None,
            archive_inspect_plaque_decal_texture: None,
            archive_inspect_plaque_decal_size: None,
        }),
    )
}

fn record_gameplay_env_prim_metadata(
    upload: &mut GameplayRoomGpuUpload,
    i: usize,
    env_prim: &RoomEnvPrimitiveCpu,
    cpu: &crate::room_glb::RoomGlbCpu,
) {
    if let Some(ref name) = env_prim.gltf_node_name {
        if matches!(
            name.as_str(),
            crate::gameplay_glb::BTN_CASH_IN | crate::gameplay_glb::LABEL_CASH_IN
        ) {
            upload.cash_in_prim_indices.push(i);
        }
        if let Some(raw_idx) = score_roller_layout::gameplay_score_roller_raw_index(name)
            && let Some(slot) = score_roller_layout::gameplay_score_roller_slot_remap(raw_idx)
        {
            upload.score_roller_prim_groups[slot].push(i);
            upload.score_roller_found[slot] = true;
            if let Some(bind) = cpu.node_bind_poses.get(name) {
                let pivot = bind.bind_world_doc.w_axis.truncate();
                upload.score_roller_pivots_doc[slot] = pivot.to_array();
                let axis = bind
                    .bind_world_doc
                    .transform_vector3(glam::Vec3::X)
                    .normalize_or_zero();
                upload.score_roller_axes_doc[slot] = axis.to_array();
            }
        }
    }
}

fn upload_gameplay_env_prim_gpu(
    i: usize,
    env_prim: &RoomEnvPrimitiveCpu,
    ctx: &RoomGpuUploadCtx<'_>,
    white_albedo_view: &wgpu::TextureView,
    room_tex_cache: &mut RoomEnvTextureCache,
) -> TilePrimitiveGpu {
    let prim = &env_prim.mesh;
    let (vb, ib, index_count) = create_room_env_mesh_buffers(ctx.device, "gameplay", i, prim);
    let mips = crate::gltf_helpers::wants_mipmaps(prim.sampler.min_filter);
    let albedo_view = upload_room_texture_slot(
        room_tex_cache,
        ctx,
        format!("gameplay-env-albedo-{i}"),
        env_prim,
        RoomTextureUsageClass::BaseColorSrgb,
        prim.albedo_rgba.as_deref(),
        prim.albedo_mip_chain.as_deref().map(|c| c.as_slice()),
        wgpu::TextureFormat::Rgba8UnormSrgb,
        mips,
        white_albedo_view,
    );
    let normal_view = upload_room_texture_slot(
        room_tex_cache,
        ctx,
        format!("gameplay-env-normal-{i}"),
        env_prim,
        RoomTextureUsageClass::NormalLinear,
        prim.normal_rgba.as_deref(),
        prim.normal_mip_chain.as_deref().map(|c| c.as_slice()),
        wgpu::TextureFormat::Rgba8Unorm,
        mips,
        ctx.tile_default_normal_view,
    );
    let metallic_roughness_view = upload_room_texture_slot(
        room_tex_cache,
        ctx,
        format!("gameplay-env-mr-{i}"),
        env_prim,
        RoomTextureUsageClass::MetallicRoughnessLinear,
        prim.metallic_roughness_rgba.as_deref(),
        prim.metallic_roughness_mip_chain
            .as_deref()
            .map(|c| c.as_slice()),
        wgpu::TextureFormat::Rgba8Unorm,
        mips,
        ctx.tile_glb_default_mr_view,
    );
    let emissive_view = upload_room_texture_slot(
        room_tex_cache,
        ctx,
        format!("gameplay-env-emissive-{i}"),
        env_prim,
        RoomTextureUsageClass::EmissiveSrgb,
        prim.emissive_rgba.as_deref(),
        prim.emissive_mip_chain.as_deref().map(|c| c.as_slice()),
        wgpu::TextureFormat::Rgba8UnormSrgb,
        mips,
        ctx.tile_glb_default_emissive_view,
    );
    let mut pbr_uniform = room_env_pbr_uniform(
        prim,
        scene_keys::GAMEPLAY,
        env_prim.gltf_node_name.as_deref(),
        env_prim.material_name.as_deref(),
    );
    if env_prim.gltf_node_name.as_deref() == Some(crate::gameplay_glb::BTN_CASH_IN) {
        pbr_uniform.add_flags(GLTF_PBR_FLAG_GAMEPLAY_CASH_IN_POLYCHROME);
    }
    let pbr_uniform_buffer = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("gameplay-pbr-{i}")),
            contents: bytemuck::bytes_of(&pbr_uniform),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let sampler = ctx
        .device
        .create_sampler(&build_sampler_descriptor(prim.sampler, None));
    TilePrimitiveGpu {
        vertex_buffer: vb,
        index_buffer: ib,
        index_count,
        albedo_view,
        normal_view,
        metallic_roughness_view,
        emissive_view,
        pbr_uniform_buffer,
        sampler,
        pipeline_key: TileGlbPipelineKey::from_loaded_primitive(prim),
        material_bind_group: None,
    }
}

fn begin_gameplay_room_gpu_upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Option<GameplayRoomGpuUpload> {
    crate::gameplay_glb::with_gameplay_glb_cpu(|cpu_opt| {
        let cpu = cpu_opt?;
        if cpu.environment_primitives.is_empty() {
            return None;
        }
        let prim_count = cpu.environment_primitives.len();
        let (white_tex, white_view) = white_albedo(device, queue);
        let (decal_tex, decal_view) = white_albedo(device, queue);
        Some(GameplayRoomGpuUpload {
            prim_count,
            next_prim: 0,
            prims: Vec::with_capacity(prim_count),
            shadow_mask_classes: room_shadow_mask_classes(
                crate::room_gi_bake::RoomGiRoom::Gameplay,
                &cpu.environment_primitives,
            ),
            room_tex_cache: RoomEnvTextureCache::new(),
            _white_albedo_tex: white_tex,
            white_albedo_view: white_view,
            _decal_tex: decal_tex,
            gameplay_decal_view: decal_view,
            cash_in_prim_indices: Vec::new(),
            score_roller_prim_groups: vec![Vec::new(); GAMEPLAY_SCORE_ROLLER_SLOT_COUNT],
            score_roller_pivots_doc: vec![[0.0, 0.0, 0.0]; GAMEPLAY_SCORE_ROLLER_SLOT_COUNT],
            score_roller_axes_doc: vec![[1.0, 0.0, 0.0]; GAMEPLAY_SCORE_ROLLER_SLOT_COUNT],
            score_roller_found: [false; GAMEPLAY_SCORE_ROLLER_SLOT_COUNT],
            started_at: Instant::now(),
        })
    })
}

fn finalize_gameplay_room_gpu_upload(
    upload: GameplayRoomGpuUpload,
    ctx: RoomGpuUploadCtx<'_>,
) -> (
    Vec<TilePrimitiveGpu>,
    Option<ShopEnvironmentGpu>,
    Vec<usize>,
    Vec<Vec<usize>>,
    Vec<[f32; 3]>,
    Vec<[f32; 3]>,
) {
    let GameplayRoomGpuUpload {
        prim_count,
        prims,
        shadow_mask_classes,
        gameplay_decal_view,
        cash_in_prim_indices,
        score_roller_prim_groups,
        score_roller_pivots_doc,
        score_roller_axes_doc,
        score_roller_found,
        ..
    } = upload;
    debug_assert_eq!(prims.len(), prim_count);

    let distortion_buffer = ctx
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gameplay-env-distortion"),
            contents: bytemuck::bytes_of(&crate::hallway_glb::HallwayDistortion::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
    let uniform_buffers =
        create_room_env_camera_uniform_buffers(ctx.device, prim_count, "gameplay-env-uniform");
    let lightmap = upload_room_lightmap_gpu(
        &ctx,
        crate::room_gi_bake::RoomGiRoom::Gameplay,
        prim_count,
        "gameplay-env-lightmap",
    );
    let bind_groups: Vec<wgpu::BindGroup> = prims
        .iter()
        .enumerate()
        .map(|(pi, p)| {
            create_room_env_material_bind_group(
                &ctx,
                "gameplay-env-bg",
                &uniform_buffers[pi],
                p,
                &gameplay_decal_view,
                &distortion_buffer,
                &lightmap.view,
            )
        })
        .collect();
    let (shadow_uniform_buffers, shadow_bind_groups) = create_room_env_shadow_gpu_batch(
        ctx.device,
        ctx.shadow_caster_layout,
        prim_count,
        "gameplay-env-shadow",
    );
    let (shadow_mask_uniform_buffers, shadow_mask_bind_groups) = create_room_shadow_mask_gpu_batch(
        ctx.device,
        ctx.room_shadow_mask_layout,
        &shadow_mask_classes,
        "gameplay-env-shadow-mask",
    );
    let shadow_warp_bind_group = create_shadow_warp_bind_group(
        ctx.device,
        ctx.shadow_warp_layout,
        &distortion_buffer,
        "gameplay-env-shadow-warp",
    );
    let gpu_wrap = Some(ShopEnvironmentGpu {
        uniform_buffers,
        distortion_buffer,
        shadow_uniform_buffers,
        shadow_bind_groups,
        _shadow_mask_uniform_buffers: shadow_mask_uniform_buffers,
        shadow_mask_bind_groups,
        shadow_warp_bind_group,
        bind_groups,
        lightmap_uv_rects: lightmap.uv_rects,
        archive_sign_decal_texture: None,
        archive_sign_decal_size: None,
        archive_inspect_plaque_decal_texture: None,
        archive_inspect_plaque_decal_size: None,
    });
    log::info!("gameplay.glb GPU: {prim_count} primitive draw(s)");
    let mapped_rolls = score_roller_found.iter().filter(|&&b| b).count();
    log::debug!(
        "gameplay.glb score rollers: all_found={} ({mapped_rolls}/{GAMEPLAY_SCORE_ROLLER_SLOT_COUNT})",
        mapped_rolls == GAMEPLAY_SCORE_ROLLER_SLOT_COUNT
    );
    if mapped_rolls != GAMEPLAY_SCORE_ROLLER_SLOT_COUNT {
        log::warn!(
            "gameplay.glb score rollers missing: found {mapped_rolls}/{GAMEPLAY_SCORE_ROLLER_SLOT_COUNT}"
        );
    }
    (
        prims,
        gpu_wrap,
        cash_in_prim_indices,
        score_roller_prim_groups,
        score_roller_pivots_doc,
        score_roller_axes_doc,
    )
}

impl WgpuRenderer {
    fn cancel_shop_room_gpu_upload(&mut self) {
        self.shop_room_gpu_upload = None;
    }

    fn cancel_hallway_room_gpu_upload(&mut self) {
        self.hallway_room_gpu_upload = None;
    }

    fn tick_shop_room_gpu_upload(&mut self, budget_ms: f32) -> bool {
        if self.rooms_gpu_loaded & ROOM_SHOP != 0 {
            self.cancel_shop_room_gpu_upload();
            return true;
        }
        if !room_cpu_env_ready(crate::room_glb::shop_cpu_ready_for_gpu_upload) {
            return false;
        }
        if self.shop_room_gpu_upload.is_none() {
            if !self.preflight_room_gpu_headroom_for_upload(true) {
                return false;
            }
            let ctx = self.room_gpu_upload_ctx();
            self.shop_room_gpu_upload = begin_incremental_room_env_gpu_upload(
                IncrementalRoomEnvKind::Shop,
                ctx.device,
                ctx.queue,
            );
        }
        let Some(mut upload) = self.shop_room_gpu_upload.take() else {
            log::error!("shop.glb GPU upload skipped: CPU environment not resident; will retry");
            RoomGpuResidentDesc::restart_cpu_after_gpu_miss(ROOM_SHOP);
            return false;
        };
        let unlimited = budget_ms >= 1.0e6;
        let deadline = if unlimited {
            None
        } else {
            Some(Instant::now() + Duration::from_secs_f32((budget_ms / 1000.0).max(0.0)))
        };
        while upload.next_prim < upload.prim_count {
            if let Some(dl) = deadline
                && Instant::now() >= dl
            {
                break;
            }
            let i = upload.next_prim;
            let ctx = self.room_gpu_upload_ctx();
            let prim_gpu = crate::room_glb::with_shop_glb_cpu(|cpu_opt| {
                let cpu = cpu_opt?;
                let env_prim = cpu.environment_primitives.get(i)?;
                if env_prim.gltf_node_name.as_deref() == Some("Eyeball") {
                    upload.shop_eyeball_prim_indices.push(i);
                }
                Some(upload_incremental_room_env_prim_gpu(
                    IncrementalRoomEnvKind::Shop,
                    i,
                    env_prim,
                    &ctx,
                    &upload.white_albedo_view,
                    &mut upload.room_tex_cache,
                ))
            });
            let Some(prim_gpu) = prim_gpu else {
                log::error!(
                    "shop.glb GPU upload aborted: CPU cache evicted mid-upload; will retry"
                );
                RoomGpuResidentDesc::restart_cpu_after_gpu_miss(ROOM_SHOP);
                self.cancel_shop_room_gpu_upload();
                return false;
            };
            upload.prims.push(prim_gpu);
            upload.next_prim += 1;
        }
        if upload.next_prim < upload.prim_count {
            self.shop_room_gpu_upload = Some(upload);
            return false;
        }

        let started_at = upload.started_at;
        let mut shop_eyeball_prim_indices = upload.shop_eyeball_prim_indices.clone();
        let metrics = crate::room_glb::with_shop_glb_cpu(|o| {
            o.map(|c| collect_room_upload_audit_metrics("shop.glb", c))
                .unwrap_or_default()
        });
        let frame_dt_ms = self.room_profile_frame_dt_ms;
        let phase = room_upload_runtime_phase(self.active_scene_key.map(|k| k as &str));
        let _cpu = crate::cpu_profiler::scope("wgpu.room.shop");
        let _startup = crate::startup_profile::scope("wgpu.room.shop");
        let finalize_t0 = Instant::now();
        let (prims, gpu_wrap) = {
            let ctx = self.room_gpu_upload_ctx();
            finalize_incremental_room_env_gpu_upload(upload, ctx)
        };
        let Some((prims, gpu_wrap)) =
            try_commit_room_environment_gpu_upload(ROOM_SHOP, "shop.glb", prims, gpu_wrap)
        else {
            self.cancel_shop_room_gpu_upload();
            return false;
        };
        self.shop_env_primitives = prims;
        self.shop_environment = Some(gpu_wrap);
        let mut shop_gltf_anim = crate::room_gltf_anim::RoomGltfAnimGpu::default();
        crate::room_glb::with_shop_glb_cpu(|cpu_opt| {
            let Some(cpu) = cpu_opt else {
                return;
            };
            shop_gltf_anim = crate::room_gltf_anim::RoomGltfAnimGpu::from_room_cpu(
                &cpu.gltf_anim_library,
                &cpu.environment_primitives,
                "shop.glb",
            );
            if shop_eyeball_prim_indices.is_empty()
                && let Some(bindings) = shop_gltf_anim.clip_prim_bindings.get("eyeball_travel")
            {
                shop_eyeball_prim_indices = bindings.iter().map(|(pi, _)| *pi).collect();
                if !shop_eyeball_prim_indices.is_empty() {
                    log::info!(
                        "shop.glb GPU: Eyeball prims from eyeball_travel bindings {:?}",
                        shop_eyeball_prim_indices
                    );
                }
            }
        });
        self.shop_gltf_anim = shop_gltf_anim;
        self.shop_eyeball_prim_indices = shop_eyeball_prim_indices;
        if !self.shop_eyeball_prim_indices.is_empty() {
            log::info!(
                "shop.glb GPU: Eyeball primitive indices {:?}",
                self.shop_eyeball_prim_indices
            );
        }
        self.shop_env_collision_meshes = crate::room_glb::with_shop_glb_cpu(|o| {
            o.map(|c| c.collision_meshes.clone()).unwrap_or_default()
        });
        crate::room_glb::release_shop_environment_cpu_sources_after_gpu_upload();
        self.rooms_gpu_loaded |= ROOM_SHOP;
        self.note_room_gpu_resident(ROOM_SHOP);
        let retained_cpu = crate::room_glb::with_shop_glb_cpu(|o| {
            o.map(retained_room_cpu_payload_bytes).unwrap_or_default()
        });
        crate::room_gpu_profile::log_room_residency_after_upload(
            "shop.glb",
            phase,
            metrics.packed_asset_bytes_read,
            metrics.decoded_cpu_payload_bytes,
            metrics.payload.total_bytes(),
            metrics.gpu_resident_estimate_bytes,
            retained_cpu,
            0,
            0,
            &self.device,
        );
        crate::room_preload::start_hallway_cpu_prefetch();
        log::info!(
            "shop.glb GPU: {} primitive draw(s)",
            self.shop_env_primitives.len()
        );
        if crate::room_gpu_profile::enabled() {
            let finalize_ms = finalize_t0.elapsed().as_secs_f64() * 1000.0;
            let total_ms = started_at.elapsed().as_secs_f64() * 1000.0;
            let mb = metrics.payload.total_bytes() as f64 / (1024.0 * 1024.0);
            let hitch = crate::room_gpu_profile::frame_timing_tag(frame_dt_ms, phase);
            log::info!(
                "room gpu profile: shop.glb GPU upload — finalize {finalize_ms:.1} ms | \
                 {total_ms:.1} ms total wall | {prims} prims | {total_mb:.2} MiB CPU payload | \
                 prev frame dt {frame_dt_ms:.1} ms ({hitch}, phase={phase})",
                prims = metrics.payload.primitives,
                total_mb = mb,
                phase = phase.label(),
            );
        }
        crate::startup_profile::log_sample("wgpu.room.shop", "first shop GPU upload");
        true
    }

    fn drive_shop_room_gpu_upload(&mut self, budget_ms: f32) {
        if !self.integrated_low_memory_allows_room_gpu_upload(ROOM_SHOP) {
            self.cancel_shop_room_gpu_upload();
            return;
        }
        if self.rooms_gpu_loaded & ROOM_SHOP != 0 {
            self.cancel_shop_room_gpu_upload();
            return;
        }
        if budget_ms >= 1.0e6 {
            while !self.tick_shop_room_gpu_upload(budget_ms) {
                if !blocking_room_upload_can_progress(RoomGpuResidentId::Shop) {
                    break;
                }
            }
        } else {
            let _ = self.tick_shop_room_gpu_upload(budget_ms);
        }
    }

    fn tick_hallway_room_gpu_upload(&mut self, budget_ms: f32) -> bool {
        if self.rooms_gpu_loaded & ROOM_HALLWAY != 0 {
            self.cancel_hallway_room_gpu_upload();
            return true;
        }
        if !room_cpu_env_ready(crate::hallway_glb::hallway_cpu_ready_for_gpu_upload) {
            return false;
        }
        if self.hallway_room_gpu_upload.is_none() {
            if !self.preflight_room_gpu_headroom_for_upload(true) {
                return false;
            }
            let ctx = self.room_gpu_upload_ctx();
            self.hallway_room_gpu_upload = begin_incremental_room_env_gpu_upload(
                IncrementalRoomEnvKind::Hallway,
                ctx.device,
                ctx.queue,
            );
        }
        let Some(mut upload) = self.hallway_room_gpu_upload.take() else {
            log::error!("hallway.glb GPU upload skipped: CPU environment not resident; will retry");
            RoomGpuResidentDesc::restart_cpu_after_gpu_miss(ROOM_HALLWAY);
            return false;
        };
        let unlimited = budget_ms >= 1.0e6;
        let deadline = if unlimited {
            None
        } else {
            Some(Instant::now() + Duration::from_secs_f32((budget_ms / 1000.0).max(0.0)))
        };
        while upload.next_prim < upload.prim_count {
            if let Some(dl) = deadline
                && Instant::now() >= dl
            {
                break;
            }
            let i = upload.next_prim;
            let ctx = self.room_gpu_upload_ctx();
            let prim_gpu = crate::hallway_glb::with_hallway_glb_cpu(|cpu_opt| {
                let cpu = cpu_opt?;
                let env_prim = cpu.environment_primitives.get(i)?;
                Some(upload_incremental_room_env_prim_gpu(
                    IncrementalRoomEnvKind::Hallway,
                    i,
                    env_prim,
                    &ctx,
                    &upload.white_albedo_view,
                    &mut upload.room_tex_cache,
                ))
            });
            let Some(prim_gpu) = prim_gpu else {
                log::error!(
                    "hallway.glb GPU upload aborted: CPU cache evicted mid-upload; will retry"
                );
                RoomGpuResidentDesc::restart_cpu_after_gpu_miss(ROOM_HALLWAY);
                self.cancel_hallway_room_gpu_upload();
                return false;
            };
            upload.prims.push(prim_gpu);
            upload.next_prim += 1;
        }
        if upload.next_prim < upload.prim_count {
            self.hallway_room_gpu_upload = Some(upload);
            return false;
        }

        let started_at = upload.started_at;
        let metrics = crate::hallway_glb::with_hallway_glb_cpu(|o| {
            o.map(|c| collect_room_upload_audit_metrics("hallway.glb", c))
                .unwrap_or_default()
        });
        let frame_dt_ms = self.room_profile_frame_dt_ms;
        let phase = room_upload_runtime_phase(self.active_scene_key.map(|k| k as &str));
        let _cpu = crate::cpu_profiler::scope("wgpu.room.hallway");
        let _startup = crate::startup_profile::scope("wgpu.room.hallway");
        let finalize_t0 = Instant::now();
        let (prims, gpu_wrap) = {
            let ctx = self.room_gpu_upload_ctx();
            finalize_incremental_room_env_gpu_upload(upload, ctx)
        };
        let Some((prims, gpu_wrap)) =
            try_commit_room_environment_gpu_upload(ROOM_HALLWAY, "hallway.glb", prims, gpu_wrap)
        else {
            self.cancel_hallway_room_gpu_upload();
            return false;
        };
        self.hallway_env_primitives = prims;
        self.hallway_environment = Some(gpu_wrap);
        crate::hallway_glb::release_hallway_environment_cpu_sources_after_gpu_upload();
        self.rooms_gpu_loaded |= ROOM_HALLWAY;
        self.note_room_gpu_resident(ROOM_HALLWAY);
        let retained_cpu = crate::hallway_glb::with_hallway_glb_cpu(|o| {
            o.map(retained_room_cpu_payload_bytes).unwrap_or_default()
        });
        crate::room_gpu_profile::log_room_residency_after_upload(
            "hallway.glb",
            phase,
            metrics.packed_asset_bytes_read,
            metrics.decoded_cpu_payload_bytes,
            metrics.payload.total_bytes(),
            metrics.gpu_resident_estimate_bytes,
            retained_cpu,
            0,
            0,
            &self.device,
        );
        crate::room_preload::start_gameplay_cpu_prefetch();
        log::info!(
            "hallway.glb GPU: {} primitive draw(s)",
            self.hallway_env_primitives.len()
        );
        if crate::room_gpu_profile::enabled() {
            let finalize_ms = finalize_t0.elapsed().as_secs_f64() * 1000.0;
            let total_ms = started_at.elapsed().as_secs_f64() * 1000.0;
            let mb = metrics.payload.total_bytes() as f64 / (1024.0 * 1024.0);
            let hitch = crate::room_gpu_profile::frame_timing_tag(frame_dt_ms, phase);
            log::info!(
                "room gpu profile: hallway.glb GPU upload — finalize {finalize_ms:.1} ms | \
                 {total_ms:.1} ms total wall | {prims} prims | {total_mb:.2} MiB CPU payload | \
                 prev frame dt {frame_dt_ms:.1} ms ({hitch}, phase={phase})",
                prims = metrics.payload.primitives,
                total_mb = mb,
                phase = phase.label(),
            );
        }
        crate::startup_profile::log_sample("wgpu.room.hallway", "first hallway GPU upload");
        true
    }

    fn drive_hallway_room_gpu_upload(&mut self, budget_ms: f32) {
        if !self.integrated_low_memory_allows_room_gpu_upload(ROOM_HALLWAY) {
            self.cancel_hallway_room_gpu_upload();
            return;
        }
        if self.rooms_gpu_loaded & ROOM_HALLWAY != 0 {
            self.cancel_hallway_room_gpu_upload();
            return;
        }
        if budget_ms >= 1.0e6 {
            while !self.tick_hallway_room_gpu_upload(budget_ms) {
                if !blocking_room_upload_can_progress(RoomGpuResidentId::Hallway) {
                    break;
                }
            }
        } else {
            let _ = self.tick_hallway_room_gpu_upload(budget_ms);
        }
    }

    fn cancel_gameplay_room_gpu_upload(&mut self) {
        self.gameplay_room_gpu_upload = None;
    }

    /// Upload gameplay room env prims across frames when `budget_ms` is finite.
    /// Returns `true` once the room is resident on the GPU.
    fn tick_gameplay_room_gpu_upload(&mut self, budget_ms: f32) -> bool {
        if self.rooms_gpu_loaded & ROOM_GAMEPLAY != 0 {
            self.cancel_gameplay_room_gpu_upload();
            return true;
        }
        if !room_cpu_env_ready(crate::gameplay_glb::gameplay_cpu_ready_for_gpu_upload) {
            return false;
        }

        if self.gameplay_room_gpu_upload.is_none() {
            let ctx = self.room_gpu_upload_ctx();
            self.gameplay_room_gpu_upload = begin_gameplay_room_gpu_upload(ctx.device, ctx.queue);
        }
        let Some(mut upload) = self.gameplay_room_gpu_upload.take() else {
            log::error!(
                "gameplay.glb GPU upload skipped: CPU environment not resident; will retry"
            );
            RoomGpuResidentDesc::restart_cpu_after_gpu_miss(ROOM_GAMEPLAY);
            return false;
        };

        let unlimited = budget_ms >= 1.0e6;
        let deadline = if unlimited {
            None
        } else {
            Some(Instant::now() + Duration::from_secs_f32((budget_ms / 1000.0).max(0.0)))
        };

        while upload.next_prim < upload.prim_count {
            if let Some(dl) = deadline
                && Instant::now() >= dl
            {
                break;
            }
            let i = upload.next_prim;
            let ctx = self.room_gpu_upload_ctx();
            let mut cpu_evicted = false;
            let prim_gpu = crate::gameplay_glb::with_gameplay_glb_cpu(|cpu_opt| {
                let Some(cpu) = cpu_opt else {
                    cpu_evicted = true;
                    return None;
                };
                let env_prim = &cpu.environment_primitives[i];
                record_gameplay_env_prim_metadata(&mut upload, i, env_prim, cpu);
                Some(upload_gameplay_env_prim_gpu(
                    i,
                    env_prim,
                    &ctx,
                    &upload.white_albedo_view,
                    &mut upload.room_tex_cache,
                ))
            });
            if cpu_evicted || prim_gpu.is_none() {
                log::error!(
                    "gameplay.glb GPU upload aborted: CPU cache evicted mid-upload; will retry"
                );
                RoomGpuResidentDesc::restart_cpu_after_gpu_miss(ROOM_GAMEPLAY);
                return false;
            }
            upload.prims.push(prim_gpu.unwrap());
            upload.next_prim += 1;
        }

        if upload.next_prim < upload.prim_count {
            self.gameplay_room_gpu_upload = Some(upload);
            return false;
        }

        let started_at = upload.started_at;
        self.ensure_gameplay_hud_pools();
        let metrics = crate::gameplay_glb::with_gameplay_glb_cpu(|o| {
            o.map(|c| collect_room_upload_audit_metrics("gameplay.glb", c))
                .unwrap_or_default()
        });
        let frame_dt_ms = self.room_profile_frame_dt_ms;
        let phase = room_upload_runtime_phase(self.active_scene_key.map(|k| k as &str));
        let _cpu = crate::cpu_profiler::scope("wgpu.room.gameplay");
        let _startup = crate::startup_profile::scope("wgpu.room.gameplay");
        let finalize_t0 = Instant::now();
        let (prims, gpu_wrap, cash_in, roller_groups, roller_pivots, roller_axes) = {
            let ctx = self.room_gpu_upload_ctx();
            finalize_gameplay_room_gpu_upload(upload, ctx)
        };
        let Some((prims, gpu_wrap)) =
            try_commit_room_environment_gpu_upload(ROOM_GAMEPLAY, "gameplay.glb", prims, gpu_wrap)
        else {
            self.cancel_gameplay_room_gpu_upload();
            return false;
        };
        self.gameplay_env_primitives = prims;
        self.gameplay_environment = Some(gpu_wrap);
        self.gameplay_cash_in_prim_indices = cash_in;
        self.gameplay_score_roller_prim_groups = roller_groups;
        self.gameplay_score_roller_pivots_doc = roller_pivots;
        self.gameplay_score_roller_axes_doc = roller_axes;
        self.gameplay_score_roller_drive_initialized
            .replace([false; 2]);
        self.gameplay_env_collision_meshes = crate::gameplay_glb::with_gameplay_glb_cpu(|o| {
            o.map(|c| c.collision_meshes.clone()).unwrap_or_default()
        });
        crate::gameplay_glb::release_gameplay_environment_cpu_sources_after_gpu_upload();
        self.rooms_gpu_loaded |= ROOM_GAMEPLAY;
        self.note_room_gpu_resident(ROOM_GAMEPLAY);
        let retained_cpu = crate::gameplay_glb::with_gameplay_glb_cpu(|o| {
            o.map(retained_room_cpu_payload_bytes).unwrap_or_default()
        });
        crate::room_gpu_profile::log_room_residency_after_upload(
            "gameplay.glb",
            phase,
            metrics.packed_asset_bytes_read,
            metrics.decoded_cpu_payload_bytes,
            metrics.payload.total_bytes(),
            metrics.gpu_resident_estimate_bytes,
            retained_cpu,
            0,
            0,
            &self.device,
        );
        if crate::room_gpu_profile::enabled() {
            let finalize_ms = finalize_t0.elapsed().as_secs_f64() * 1000.0;
            let total_ms = started_at.elapsed().as_secs_f64() * 1000.0;
            let mb = metrics.payload.total_bytes() as f64 / (1024.0 * 1024.0);
            let hitch = crate::room_gpu_profile::frame_timing_tag(frame_dt_ms, phase);
            log::info!(
                "room gpu profile: gameplay.glb GPU upload — finalize {finalize_ms:.1} ms | \
                 {total_ms:.1} ms total wall | {prims} prims | {total_mb:.2} MiB CPU payload | \
                 prev frame dt {frame_dt_ms:.1} ms ({hitch}, phase={phase})",
                prims = metrics.payload.primitives,
                total_mb = mb,
                phase = phase.label(),
            );
        }
        crate::startup_profile::log_sample("wgpu.room.gameplay", "first gameplay GPU upload");
        true
    }

    fn drive_gameplay_room_gpu_upload(&mut self, budget_ms: f32) {
        if !self.integrated_low_memory_allows_room_gpu_upload(ROOM_GAMEPLAY) {
            self.cancel_gameplay_room_gpu_upload();
            return;
        }
        if self.rooms_gpu_loaded & ROOM_GAMEPLAY != 0 {
            return;
        }
        if budget_ms >= 1.0e6 {
            while !self.tick_gameplay_room_gpu_upload(budget_ms) {
                if !blocking_room_upload_can_progress(RoomGpuResidentId::Gameplay) {
                    break;
                }
            }
        } else {
            let _ = self.tick_gameplay_room_gpu_upload(budget_ms);
        }
    }
}

impl WgpuRenderer {
    pub(super) fn ensure_room_gpu_for_draw_cmds(&mut self, cmds: &[DrawCmd]) {
        let mut need = 0u8;
        let mut need_shadow_test_room = false;
        for cmd in cmds {
            match cmd {
                DrawCmd::ShopEnvironment => need |= ROOM_SHOP,
                DrawCmd::MainMenuEnvironment => need |= ROOM_MAIN_MENU,
                DrawCmd::HallwayEnvironment => need |= ROOM_HALLWAY,
                DrawCmd::StaircaseEnvironment => need |= ROOM_STAIRCASE,
                DrawCmd::ArchiveEnvironment => need |= ROOM_ARCHIVE,
                DrawCmd::GameplayEnvironment => need |= ROOM_GAMEPLAY,
                DrawCmd::ShadowTestEnvironment => need_shadow_test_room = true,
                _ => {}
            }
        }
        if need & ROOM_SHOP != 0 {
            self.ensure_shop_room_gpu();
        }
        if need & ROOM_MAIN_MENU != 0 {
            self.ensure_main_menu_room_gpu();
        }
        if need & ROOM_HALLWAY != 0 {
            self.ensure_hallway_room_gpu();
        }
        if need & ROOM_STAIRCASE != 0 {
            self.ensure_staircase_room_gpu();
        }
        if need & ROOM_ARCHIVE != 0 {
            self.ensure_archive_room_gpu();
        }
        if need & ROOM_GAMEPLAY != 0 {
            self.ensure_gameplay_room_gpu();
        }
        if need_shadow_test_room {
            self.ensure_shadow_test_room_gpu();
        }
    }

    pub fn ensure_gameplay_room_gpu_for_resume(&mut self) {
        self.ensure_gameplay_room_gpu();
    }

    /// Preload room GPU data for picking before [`Self::render`] builds ops.
    pub fn ensure_rooms_for_scene_key(&mut self, key: Option<&str>) {
        let Some(key) = key else {
            return;
        };
        let norm = scene_keys::normalize_scene_key(key);
        if norm == scene_keys::HALLWAY {
            self.ensure_shop_room_gpu();
        }
        if let Some(bit) = self.room_gpu_bit_for_scene_key(key) {
            self.ensure_room_gpu_for_bit(bit);
        }
    }

    fn ensure_room_gpu_for_bit(&mut self, bit: u8) {
        match bit {
            ROOM_MAIN_MENU => self.ensure_main_menu_room_gpu(),
            ROOM_SHOP => self.ensure_shop_room_gpu(),
            ROOM_HALLWAY => self.ensure_hallway_room_gpu(),
            ROOM_STAIRCASE => self.ensure_staircase_room_gpu(),
            ROOM_ARCHIVE => self.ensure_archive_room_gpu(),
            ROOM_GAMEPLAY => self.ensure_gameplay_room_gpu(),
            _ => {}
        }
    }

    /// Drop progressive room uploads that are not allowed for the current pin.
    fn cancel_unpinned_room_gpu_uploads(&mut self) {
        if !self.integrated_low_memory_allows_room_gpu_upload(ROOM_SHOP) {
            self.cancel_shop_room_gpu_upload();
        }
        if !self.integrated_low_memory_allows_room_gpu_upload(ROOM_HALLWAY) {
            self.cancel_hallway_room_gpu_upload();
        }
        if !self.integrated_low_memory_allows_room_gpu_upload(ROOM_GAMEPLAY) {
            self.cancel_gameplay_room_gpu_upload();
        }
    }

    /// Start background CPU decode for the next room in the run chain.
    pub fn prefetch_room_chain_next(&mut self, scene: crate::room_preload::RoomSceneChain) {
        crate::room_preload::start_room_cpu_prefetch(scene);
    }

    /// Join finished prefetch workers and upload room GPU data when CPU caches are ready.
    ///
    /// Only uploads rooms that match the current hub/run chain — avoids parking shop,
    /// hallway, and gameplay environments on the GPU while the main menu is idle.
    ///
    /// `frame_dt_ms` is the previous frame wall time; logged as a hitch proxy when
    /// [`crate::room_gpu_profile`] is enabled.
    /// `continue_warmup` — when set (Continue into an in-progress run), also upload
    /// the saved destination room chain on the main menu once CPU prefetch finishes.
    ///
    /// `pending_scene_key` — scene we are fading into (`App::pending_scene`), if any;
    /// uploads the destination room during fade-out so the black-frame swap does not stall.
    ///
    /// `pending_transition_at_black` — transition alpha has reached zero; finish destination
    /// room uploads as fast as possible while the screen stays black.
    pub fn poll_room_prefetch_gpu_uploads(
        &mut self,
        scene_key: Option<&str>,
        frame_dt_ms: f32,
        continue_warmup: crate::room_preload::ContinueRoomWarmup,
        pending_scene_key: Option<&str>,
        pending_transition_at_black: bool,
    ) {
        self.room_profile_frame_dt_ms = frame_dt_ms;
        // While held at full black, pin the destination room so integrated low-memory GPUs
        // can evict the source scene (e.g. main menu) and upload the pending shop room.
        self.poll_pinned_room_gpu_bit = if pending_transition_at_black {
            pending_scene_key.and_then(|k| self.room_gpu_bit_for_scene_key(k))
        } else {
            scene_key.and_then(|k| self.room_gpu_bit_for_scene_key(k))
        };
        if self.integrated_low_memory_gpu() {
            self.cancel_unpinned_room_gpu_uploads();
        }
        self.refresh_gpu_memory_pressure();
        crate::room_preload::try_drain_room_cpu_prefetch_threads();
        crate::room_preload::kick_eager_all_room_cpu_prefetches();
        if continue_warmup != crate::room_preload::ContinueRoomWarmup::None {
            crate::room_preload::kick_continue_run_cpu_prefetches(continue_warmup);
        }

        let mut room_env_upload_done = false;

        let upload_gameplay = |this: &mut Self, budget_ms: f32| {
            if this.rooms_gpu_loaded & ROOM_GAMEPLAY != 0 {
                return;
            }
            if !crate::gameplay_glb::gameplay_cpu_ready_for_gpu_upload() {
                return;
            }
            this.drive_gameplay_room_gpu_upload(budget_ms);
        };

        if !pending_transition_at_black {
            match scene_key.map(scene_keys::normalize_scene_key) {
                Some(scene_keys::MAIN_MENU) => {
                    if !room_env_upload_done {
                        let before = self.rooms_gpu_loaded;
                        self.ensure_main_menu_room_gpu();
                        if self.rooms_gpu_loaded != before {
                            room_env_upload_done = true;
                        }
                    }
                    crate::room_preload::advance_hub_cpu_prefetch_chain(true);
                    if !room_env_upload_done {
                        self.ensure_talisman_textures();
                    }
                    match continue_warmup {
                        crate::room_preload::ContinueRoomWarmup::Shop => {
                            self.maybe_upload_one_room_env(
                                &mut room_env_upload_done,
                                ROOM_SHOP,
                                crate::room_glb::shop_cpu_ready_for_gpu_upload(),
                                |r| r.ensure_shop_room_gpu(),
                            );
                        }
                        crate::room_preload::ContinueRoomWarmup::Hallway => {
                            self.maybe_upload_one_room_env(
                                &mut room_env_upload_done,
                                ROOM_SHOP,
                                crate::room_glb::shop_cpu_ready_for_gpu_upload(),
                                |r| r.ensure_shop_room_gpu(),
                            );
                            self.maybe_upload_one_room_env(
                                &mut room_env_upload_done,
                                ROOM_HALLWAY,
                                crate::hallway_glb::hallway_cpu_ready_for_gpu_upload(),
                                |r| r.ensure_hallway_room_gpu(),
                            );
                        }
                        crate::room_preload::ContinueRoomWarmup::Gameplay => {
                            upload_gameplay(
                                self,
                                gameplay_eager_upload_budget_ms(self.graphics_mode),
                            );
                        }
                        crate::room_preload::ContinueRoomWarmup::None => {}
                    }
                }
                Some(scene_keys::SHOP) | Some(scene_keys::HALLWAY) => {
                    self.maybe_upload_one_room_env(
                        &mut room_env_upload_done,
                        ROOM_SHOP,
                        crate::room_glb::shop_cpu_ready_for_gpu_upload(),
                        |r| r.ensure_shop_room_gpu(),
                    );
                    self.maybe_upload_one_room_env(
                        &mut room_env_upload_done,
                        ROOM_HALLWAY,
                        crate::hallway_glb::hallway_cpu_ready_for_gpu_upload(),
                        |r| r.ensure_hallway_room_gpu(),
                    );
                    if self.rooms_gpu_loaded & ROOM_HALLWAY != 0
                        && self.rooms_gpu_loaded & ROOM_GAMEPLAY == 0
                    {
                        crate::room_preload::start_gameplay_cpu_prefetch();
                    }
                    upload_gameplay(self, GAMEPLAY_ROOM_GPU_UPLOAD_BUDGET_MS);
                }
                Some(scene_keys::ARCHIVE) => {
                    self.maybe_upload_one_room_env(
                        &mut room_env_upload_done,
                        ROOM_ARCHIVE,
                        crate::archive_glb::archive_cpu_ready_for_gpu_upload(),
                        |r| r.ensure_archive_room_gpu(),
                    );
                }
                Some(scene_keys::VICTORY) => {
                    if victory_uses_3d_moon(self.graphics_mode) && !room_env_upload_done {
                        let before = self.rooms_gpu_loaded;
                        self.ensure_main_menu_room_gpu();
                        if self.rooms_gpu_loaded != before {
                            room_env_upload_done = true;
                        }
                    }
                }
                Some(scene_keys::GAMEPLAY) | Some(scene_keys::DEFEAT) => {
                    upload_gameplay(self, GAMEPLAY_ROOM_GPU_UPLOAD_BUDGET_MS);
                }
                _ => {}
            }
        }

        if let Some(pending) = pending_scene_key {
            if pending_transition_at_black {
                self.drive_pending_scene_room_gpu_at_black(pending);
            } else {
                match scene_keys::normalize_scene_key(pending) {
                    scene_keys::MAIN_MENU => {
                        crate::room_preload::start_main_menu_cpu_prefetch();
                        self.maybe_upload_one_room_env(
                            &mut room_env_upload_done,
                            ROOM_MAIN_MENU,
                            crate::main_menu_glb::main_menu_cpu_ready_for_gpu_upload(),
                            |r| r.ensure_main_menu_room_gpu(),
                        );
                    }
                    scene_keys::VICTORY => {
                        if victory_uses_3d_moon(self.graphics_mode) {
                            crate::room_preload::start_main_menu_cpu_prefetch();
                            self.maybe_upload_one_room_env(
                                &mut room_env_upload_done,
                                ROOM_MAIN_MENU,
                                crate::main_menu_glb::main_menu_cpu_ready_for_gpu_upload(),
                                |r| r.ensure_main_menu_room_gpu(),
                            );
                        }
                    }
                    scene_keys::GAMEPLAY | scene_keys::DEFEAT => {
                        crate::room_preload::start_gameplay_cpu_prefetch();
                        upload_gameplay(self, GAMEPLAY_ROOM_GPU_UPLOAD_BUDGET_MS);
                    }
                    scene_keys::HALLWAY => {
                        crate::room_preload::start_hallway_cpu_prefetch();
                        self.maybe_upload_one_room_env(
                            &mut room_env_upload_done,
                            ROOM_HALLWAY,
                            crate::hallway_glb::hallway_cpu_ready_for_gpu_upload(),
                            |r| r.ensure_hallway_room_gpu(),
                        );
                    }
                    scene_keys::SHOP => {
                        crate::room_preload::start_shop_cpu_prefetch();
                        self.maybe_upload_one_room_env(
                            &mut room_env_upload_done,
                            ROOM_SHOP,
                            crate::room_glb::shop_cpu_ready_for_gpu_upload(),
                            |r| r.ensure_shop_room_gpu(),
                        );
                    }
                    scene_keys::ARCHIVE => {
                        crate::room_preload::start_archive_cpu_prefetch();
                        self.maybe_upload_one_room_env(
                            &mut room_env_upload_done,
                            ROOM_ARCHIVE,
                            crate::archive_glb::archive_cpu_ready_for_gpu_upload(),
                            |r| r.ensure_archive_room_gpu(),
                        );
                    }
                    scene_keys::STAIRWAY => {
                        if !room_env_upload_done && self.rooms_gpu_loaded & ROOM_STAIRCASE == 0 {
                            let before = self.rooms_gpu_loaded;
                            self.ensure_staircase_room_gpu();
                            if self.rooms_gpu_loaded != before {
                                room_env_upload_done = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        self.advance_eager_room_gpu_warmup(&mut room_env_upload_done);
    }

    fn room_gpu_upload_ctx(&self) -> RoomGpuUploadCtx<'_> {
        RoomGpuUploadCtx {
            device: &self.device,
            queue: &self.queue,
            room_env_material_layout: &self.room_env_material_layout,
            shadow_caster_layout: &self.shadow_caster_layout,
            room_shadow_mask_layout: &self.room_shadow_mask_layout,
            shadow_warp_layout: &self.shadow_warp_layout,
            tile_default_normal_view: &self.tile_env_normal_view,
            tile_glb_default_mr_view: &self.tile_env_mr_view,
            tile_glb_default_emissive_view: &self.tile_env_emissive_view,
        }
    }

    /// Shared upload path for rooms without extra GPU metadata (main menu, hallway, staircase).
    fn ensure_standard_room_env_gpu(
        &mut self,
        id: RoomGpuResidentId,
        collect_metrics: impl FnOnce() -> RoomUploadAuditMetrics,
        load: impl FnOnce(RoomGpuUploadCtx<'_>) -> (Vec<TilePrimitiveGpu>, Option<ShopEnvironmentGpu>),
        install: impl FnOnce(&mut Self, Vec<TilePrimitiveGpu>, ShopEnvironmentGpu),
        collect_retained_cpu_bytes: impl FnOnce() -> u64,
        after_installed: impl FnOnce(&mut Self),
    ) {
        let desc = id.desc();
        let bit = desc.bit();
        if self.rooms_gpu_loaded & bit != 0 {
            return;
        }
        if !self.integrated_low_memory_allows_room_gpu_upload(bit) {
            return;
        }
        if !room_cpu_env_ready(desc.cpu_ready_for_gpu_upload) {
            return;
        }
        let metrics = collect_metrics();
        let frame_dt_ms = self.room_profile_frame_dt_ms;
        let phase = room_upload_runtime_phase(self.active_scene_key.map(|k| k as &str));
        crate::room_gpu_profile::measure_gpu_upload(
            desc.glb,
            desc.startup_scope,
            metrics.payload,
            frame_dt_ms,
            phase,
            || {
                let ctx = self.room_gpu_upload_ctx();
                let (prims, gpu_wrap) = load(ctx);
                let Some((prims, gpu_wrap)) =
                    try_commit_room_environment_gpu_upload(bit, desc.glb, prims, gpu_wrap)
                else {
                    return;
                };
                install(self, prims, gpu_wrap);
                self.rooms_gpu_loaded |= bit;
                self.note_room_gpu_resident(bit);
                let retained_cpu = collect_retained_cpu_bytes();
                crate::room_gpu_profile::log_room_residency_after_upload(
                    desc.glb,
                    phase,
                    metrics.packed_asset_bytes_read,
                    metrics.decoded_cpu_payload_bytes,
                    metrics.payload.total_bytes(),
                    metrics.gpu_resident_estimate_bytes,
                    retained_cpu,
                    0,
                    0,
                    &self.device,
                );
                after_installed(self);
            },
        );
    }

    pub(super) fn clear_room_gpu_resident_fields(&mut self, id: RoomGpuResidentId) {
        match id {
            RoomGpuResidentId::MainMenu => {
                self.main_menu_env_primitives.clear();
                self.main_menu_environment = None;
                self.main_menu_moon_prim_indices.clear();
                self.main_menu_env_collision_meshes.clear();
            }
            RoomGpuResidentId::Shop => {
                self.shop_room_gpu_upload = None;
                self.shop_env_primitives.clear();
                self.shop_environment = None;
                self.shop_gltf_anim = crate::room_gltf_anim::RoomGltfAnimGpu::default();
                self.shop_eyeball_prim_indices.clear();
                self.shop_env_collision_meshes.clear();
            }
            RoomGpuResidentId::Hallway => {
                self.hallway_room_gpu_upload = None;
                self.hallway_env_primitives.clear();
                self.hallway_environment = None;
            }
            RoomGpuResidentId::Staircase => {
                self.staircase_env_primitives.clear();
                self.staircase_environment = None;
            }
            RoomGpuResidentId::Archive => {
                self.archive_env_primitives.clear();
                self.archive_environment = None;
                self.archive_sign_left_prim_idx = None;
                self.archive_sign_right_prim_idx = None;
                self.archive_inspect_plaque_prim_idx = None;
                self.archive_plaque_backing_prim_idx = None;
                self.archive_page_left_prim_indices.clear();
                self.archive_page_right_prim_indices.clear();
            }
            RoomGpuResidentId::Gameplay => {
                self.gameplay_room_gpu_upload = None;
                self.gameplay_env_primitives.clear();
                self.gameplay_environment = None;
                self.gameplay_cash_in_prim_indices.clear();
                self.gameplay_score_roller_prim_groups.clear();
                self.gameplay_score_roller_pivots_doc.clear();
                self.gameplay_score_roller_axes_doc.clear();
                self.gameplay_env_collision_meshes.clear();
            }
        }
    }

    pub(super) fn ensure_main_menu_room_gpu(&mut self) {
        // After GPU eviction, CPU vertex buffers were released — re-decode before upload.
        crate::main_menu_glb::with_main_menu_glb_cpu(|_| ());
        self.ensure_standard_room_env_gpu(
            RoomGpuResidentId::MainMenu,
            || {
                crate::main_menu_glb::with_main_menu_glb_cpu(|o| {
                    o.map(|c| collect_room_upload_audit_metrics("main_menu.glb", c))
                        .unwrap_or_default()
                })
            },
            load_main_menu_room_gpu,
            |this, prims, gpu_wrap| {
                this.main_menu_env_primitives = prims;
                this.main_menu_environment = Some(gpu_wrap);
                this.main_menu_moon_prim_indices =
                    crate::main_menu_glb::with_main_menu_glb_cpu(|o| {
                        o.map(|cpu| {
                            cpu.environment_primitives
                                .iter()
                                .enumerate()
                                .filter_map(|(i, ep)| {
                                    (ep.gltf_node_name.as_deref()
                                        == Some(crate::main_menu_glb::MAIN_MENU_MOON_MESH_NODE))
                                    .then_some(i)
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                    });
                this.main_menu_env_collision_meshes =
                    crate::main_menu_glb::with_main_menu_glb_cpu(|o| {
                        o.map(|c| c.collision_meshes.clone()).unwrap_or_default()
                    });
                crate::main_menu_glb::release_main_menu_environment_cpu_sources_after_gpu_upload();
            },
            || {
                crate::main_menu_glb::with_main_menu_glb_cpu(|o| {
                    o.map(retained_room_cpu_payload_bytes).unwrap_or_default()
                })
            },
            |_| {},
        );
    }

    pub(super) fn ensure_shop_room_gpu(&mut self) {
        let desc = RoomGpuResidentId::Shop.desc();
        if !self.integrated_low_memory_allows_room_gpu_upload(desc.bit()) {
            self.cancel_shop_room_gpu_upload();
            return;
        }
        if self.rooms_gpu_loaded & desc.bit() != 0 {
            return;
        }
        if self.shop_room_gpu_upload.is_some() {
            self.drive_shop_room_gpu_upload(f32::MAX);
            return;
        }
        if !room_cpu_env_ready(desc.cpu_ready_for_gpu_upload) {
            return;
        }
        let metrics = crate::room_glb::with_shop_glb_cpu(|o| {
            o.map(|c| collect_room_upload_audit_metrics("shop.glb", c))
                .unwrap_or_default()
        });
        let frame_dt_ms = self.room_profile_frame_dt_ms;
        let phase = room_upload_runtime_phase(self.active_scene_key.map(|k| k as &str));
        crate::room_gpu_profile::measure_gpu_upload(
            desc.glb,
            desc.startup_scope,
            metrics.payload,
            frame_dt_ms,
            phase,
            || {
                let ctx = self.room_gpu_upload_ctx();
                let (prims, gpu_wrap, anim, eyeball) = load_shop_room_gpu(ctx);
                let Some((prims, gpu_wrap)) =
                    try_commit_room_environment_gpu_upload(desc.bit(), desc.glb, prims, gpu_wrap)
                else {
                    return;
                };
                self.shop_env_primitives = prims;
                self.shop_environment = Some(gpu_wrap);
                self.shop_gltf_anim = anim;
                self.shop_eyeball_prim_indices = eyeball;
                self.shop_env_collision_meshes = crate::room_glb::with_shop_glb_cpu(|o| {
                    o.map(|c| c.collision_meshes.clone()).unwrap_or_default()
                });
                crate::room_glb::release_shop_environment_cpu_sources_after_gpu_upload();
                self.rooms_gpu_loaded |= desc.bit();
                self.note_room_gpu_resident(desc.bit());
                let retained_cpu = crate::room_glb::with_shop_glb_cpu(|o| {
                    o.map(retained_room_cpu_payload_bytes).unwrap_or_default()
                });
                crate::room_gpu_profile::log_room_residency_after_upload(
                    desc.glb,
                    phase,
                    metrics.packed_asset_bytes_read,
                    metrics.decoded_cpu_payload_bytes,
                    metrics.payload.total_bytes(),
                    metrics.gpu_resident_estimate_bytes,
                    retained_cpu,
                    0,
                    0,
                    &self.device,
                );
                crate::room_preload::start_hallway_cpu_prefetch();
            },
        );
    }

    pub(super) fn ensure_hallway_room_gpu(&mut self) {
        if self.hallway_room_gpu_upload.is_some() {
            self.drive_hallway_room_gpu_upload(f32::MAX);
            return;
        }
        self.ensure_standard_room_env_gpu(
            RoomGpuResidentId::Hallway,
            || {
                crate::hallway_glb::with_hallway_glb_cpu(|o| {
                    o.map(|c| collect_room_upload_audit_metrics("hallway.glb", c))
                        .unwrap_or_default()
                })
            },
            load_hallway_room_gpu,
            |this, prims, gpu_wrap| {
                this.hallway_env_primitives = prims;
                this.hallway_environment = Some(gpu_wrap);
                crate::hallway_glb::release_hallway_environment_cpu_sources_after_gpu_upload();
            },
            || {
                crate::hallway_glb::with_hallway_glb_cpu(|o| {
                    o.map(retained_room_cpu_payload_bytes).unwrap_or_default()
                })
            },
            |_| {
                crate::room_preload::start_gameplay_cpu_prefetch();
            },
        );
    }

    pub(super) fn ensure_staircase_room_gpu(&mut self) {
        self.ensure_standard_room_env_gpu(
            RoomGpuResidentId::Staircase,
            || {
                crate::staircase_glb::with_staircase_glb_cpu(|o| {
                    o.map(|c| collect_room_upload_audit_metrics("staircase.glb", c))
                        .unwrap_or_default()
                })
            },
            load_staircase_room_gpu,
            |this, prims, gpu_wrap| {
                this.staircase_env_primitives = prims;
                this.staircase_environment = Some(gpu_wrap);
                crate::staircase_glb::release_staircase_environment_cpu_sources_after_gpu_upload();
            },
            || {
                crate::staircase_glb::with_staircase_glb_cpu(|o| {
                    o.map(retained_room_cpu_payload_bytes).unwrap_or_default()
                })
            },
            |_| {},
        );
    }

    pub(super) fn ensure_shadow_test_room_gpu(&mut self) {
        if self.shadow_test_room_environment.is_some()
            && !self.shadow_test_room_env_primitives.is_empty()
        {
            return;
        }
        crate::shadow_test_room_glb::decode_shadow_test_room_glb_into_cache();
        if !crate::shadow_test_room_glb::shadow_test_room_cpu_ready_for_gpu_upload() {
            return;
        }
        let metrics = crate::shadow_test_room_glb::with_shadow_test_room_glb_cpu(|o| {
            o.map(|c| collect_room_upload_audit_metrics("shadow_test_room.glb", c))
                .unwrap_or_default()
        });
        let frame_dt_ms = self.room_profile_frame_dt_ms;
        let phase = room_upload_runtime_phase(self.active_scene_key.map(|k| k as &str));
        crate::room_gpu_profile::measure_gpu_upload(
            "shadow_test_room.glb",
            "wgpu.room.shadow_test_room",
            metrics.payload,
            frame_dt_ms,
            phase,
            || {
                let ctx = self.room_gpu_upload_ctx();
                let (prims, gpu_wrap) = load_shadow_test_room_gpu(ctx);
                let Some(gpu_wrap) = gpu_wrap else {
                    log::error!("shadow_test_room.glb GPU upload skipped: CPU environment missing");
                    return;
                };
                if prims.is_empty() {
                    log::error!("shadow_test_room.glb GPU upload skipped: no primitives");
                    return;
                }
                self.shadow_test_room_env_primitives = prims;
                self.shadow_test_room_environment = Some(gpu_wrap);
                crate::shadow_test_room_glb::release_shadow_test_room_environment_cpu_sources_after_gpu_upload();
                let retained_cpu =
                    crate::shadow_test_room_glb::with_shadow_test_room_glb_cpu(|o| {
                        o.map(retained_room_cpu_payload_bytes).unwrap_or_default()
                    });
                crate::room_gpu_profile::log_room_residency_after_upload(
                    "shadow_test_room.glb",
                    phase,
                    metrics.packed_asset_bytes_read,
                    metrics.decoded_cpu_payload_bytes,
                    metrics.payload.total_bytes(),
                    metrics.gpu_resident_estimate_bytes,
                    retained_cpu,
                    0,
                    0,
                    &self.device,
                );
            },
        );
    }

    pub(super) fn ensure_archive_room_gpu(&mut self) {
        let desc = RoomGpuResidentId::Archive.desc();
        if !self.integrated_low_memory_allows_room_gpu_upload(desc.bit()) {
            return;
        }
        if self.rooms_gpu_loaded & desc.bit() != 0 {
            return;
        }
        if !room_cpu_env_ready(desc.cpu_ready_for_gpu_upload) {
            return;
        }
        let metrics = crate::archive_glb::with_archive_glb_cpu(|o| {
            o.map(|c| collect_room_upload_audit_metrics("archive.glb", c))
                .unwrap_or_default()
        });
        let frame_dt_ms = self.room_profile_frame_dt_ms;
        let phase = room_upload_runtime_phase(self.active_scene_key.map(|k| k as &str));
        crate::room_gpu_profile::measure_gpu_upload(
            desc.glb,
            desc.startup_scope,
            metrics.payload,
            frame_dt_ms,
            phase,
            || {
                let ctx = self.room_gpu_upload_ctx();
                let (
                    prims,
                    gpu_wrap,
                    sign_l,
                    sign_r,
                    inspect_plaque,
                    plaque_backing,
                    page_left,
                    page_right,
                ) = load_archive_room_gpu(ctx);
                let Some((prims, gpu_wrap)) =
                    try_commit_room_environment_gpu_upload(desc.bit(), desc.glb, prims, gpu_wrap)
                else {
                    return;
                };
                self.archive_env_primitives = prims;
                self.archive_environment = Some(gpu_wrap);
                self.archive_sign_left_prim_idx = sign_l;
                self.archive_sign_right_prim_idx = sign_r;
                self.archive_inspect_plaque_prim_idx = inspect_plaque;
                self.archive_plaque_backing_prim_idx = plaque_backing;
                self.archive_page_left_prim_indices = page_left;
                self.archive_page_right_prim_indices = page_right;
                crate::archive_glb::release_archive_environment_cpu_sources_after_gpu_upload();
                self.rooms_gpu_loaded |= desc.bit();
                self.note_room_gpu_resident(desc.bit());
                let retained_cpu = crate::archive_glb::with_archive_glb_cpu(|o| {
                    o.map(retained_room_cpu_payload_bytes).unwrap_or_default()
                });
                crate::room_gpu_profile::log_room_residency_after_upload(
                    desc.glb,
                    phase,
                    metrics.packed_asset_bytes_read,
                    metrics.decoded_cpu_payload_bytes,
                    metrics.payload.total_bytes(),
                    metrics.gpu_resident_estimate_bytes,
                    retained_cpu,
                    0,
                    0,
                    &self.device,
                );
            },
        );
    }

    pub(super) fn ensure_gameplay_room_gpu(&mut self) {
        self.drive_gameplay_room_gpu_upload(f32::MAX);
    }

    /// Upload every hub/run room GLB while the splash plate is up (Performance/Visuals only).
    pub(super) fn drive_splash_eager_room_gpu_boot(&mut self) {
        if self.graphics_mode == mahjuro_gfx_types::GraphicsMode::LowMemory {
            return;
        }
        if splash_eager_rooms_gpu_loaded(self.graphics_mode, self.rooms_gpu_loaded) {
            return;
        }
        let deadline = Instant::now()
            + Duration::from_secs_f32(SPLASH_EAGER_ROOM_GPU_UPLOAD_BUDGET_MS / 1000.0);
        while Instant::now() < deadline {
            if splash_eager_rooms_gpu_loaded(self.graphics_mode, self.rooms_gpu_loaded) {
                break;
            }
            let before = self.rooms_gpu_loaded;
            let remaining_ms = deadline
                .saturating_duration_since(Instant::now())
                .as_secs_f32()
                * 1000.0;
            if remaining_ms <= 0.05 {
                break;
            }
            self.drive_splash_next_eager_room_upload(remaining_ms);
            let in_flight = self.shop_room_gpu_upload.is_some()
                || self.hallway_room_gpu_upload.is_some()
                || self.gameplay_room_gpu_upload.is_some();
            if self.rooms_gpu_loaded == before && !in_flight {
                self.join_splash_blocking_cpu_decode_for_next_room();
                break;
            }
        }
    }

    /// Splash may block briefly on CPU decode so GPU warm-up keeps moving during the logo plate.
    fn join_splash_blocking_cpu_decode_for_next_room(&self) {
        if self.rooms_gpu_loaded & ROOM_SHOP == 0
            && !crate::room_glb::shop_cpu_ready_for_gpu_upload()
        {
            crate::room_preload::start_shop_cpu_prefetch();
            crate::room_preload::join_shop_cpu_prefetch_blocking();
            return;
        }
        if self.rooms_gpu_loaded & ROOM_ARCHIVE == 0
            && !crate::archive_glb::archive_cpu_ready_for_gpu_upload()
        {
            crate::room_preload::start_archive_cpu_prefetch();
            crate::room_preload::join_archive_cpu_prefetch_blocking();
            return;
        }
        if self.rooms_gpu_loaded & ROOM_HALLWAY == 0
            && !crate::hallway_glb::hallway_cpu_ready_for_gpu_upload()
        {
            crate::room_preload::start_hallway_cpu_prefetch();
            crate::room_preload::join_hallway_cpu_prefetch_blocking();
            return;
        }
        if self.rooms_gpu_loaded & ROOM_GAMEPLAY == 0
            && !crate::gameplay_glb::gameplay_cpu_ready_for_gpu_upload()
        {
            crate::room_preload::start_gameplay_cpu_prefetch();
            crate::room_preload::join_gameplay_cpu_prefetch_blocking();
            return;
        }
        if self.rooms_gpu_loaded & ROOM_STAIRCASE == 0
            && !crate::staircase_glb::staircase_cpu_ready_for_gpu_upload()
        {
            crate::room_preload::kick_eager_all_room_cpu_prefetches();
            let _ = crate::staircase_glb::with_staircase_glb_cpu(|_| ());
        }
    }

    fn drive_splash_next_eager_room_upload(&mut self, budget_ms: f32) {
        if self.gameplay_room_gpu_upload.is_some() {
            self.drive_gameplay_room_gpu_upload(budget_ms.min(GAMEPLAY_EAGER_UPLOAD_BUDGET_MS));
            return;
        }
        if self.shop_room_gpu_upload.is_some() {
            self.drive_shop_room_gpu_upload(budget_ms);
            return;
        }
        if self.hallway_room_gpu_upload.is_some() {
            self.drive_hallway_room_gpu_upload(budget_ms);
            return;
        }
        if self.rooms_gpu_loaded & ROOM_SHOP == 0
            && crate::room_glb::shop_cpu_ready_for_gpu_upload()
        {
            self.drive_shop_room_gpu_upload(budget_ms);
            return;
        }
        if self.rooms_gpu_loaded & ROOM_ARCHIVE == 0
            && crate::archive_glb::archive_cpu_ready_for_gpu_upload()
        {
            self.ensure_archive_room_gpu();
            return;
        }
        if self.rooms_gpu_loaded & ROOM_HALLWAY == 0
            && crate::hallway_glb::hallway_cpu_ready_for_gpu_upload()
        {
            self.drive_hallway_room_gpu_upload(budget_ms);
            return;
        }
        if self.rooms_gpu_loaded & ROOM_GAMEPLAY == 0
            && crate::gameplay_glb::gameplay_cpu_ready_for_gpu_upload()
        {
            self.drive_gameplay_room_gpu_upload(budget_ms.min(GAMEPLAY_EAGER_UPLOAD_BUDGET_MS));
            return;
        }
        if self.rooms_gpu_loaded & ROOM_STAIRCASE == 0
            && crate::staircase_glb::staircase_cpu_ready_for_gpu_upload()
        {
            self.ensure_staircase_room_gpu();
        }
    }

    /// Frame-paced eager GPU warm-up for rooms not required by the active scene.
    fn advance_eager_room_gpu_warmup(&mut self, done: &mut bool) {
        if *done {
            return;
        }
        // One resident slot on integrated GPUs — warm only during black-frame transitions.
        if self.integrated_low_memory_gpu() {
            return;
        }
        let snapshot = self.gpu_memory_pressure_snapshot();
        let pressure =
            crate::gpu_memory_pressure::eager_warm_pressure(&snapshot, self.graphics_mode);
        if pressure == crate::gpu_memory_pressure::GpuMemoryPressure::Critical {
            self.preflight_room_gpu_headroom_for_upload(false);
            crate::gpu_memory_pressure::log_eager_preload("paused", "all", pressure);
            return;
        }

        if self.gameplay_room_gpu_upload.is_some() {
            self.drive_gameplay_room_gpu_upload(gameplay_eager_upload_budget_ms(
                self.graphics_mode,
            ));
            *done = true;
            return;
        }

        let hub_only = pressure == crate::gpu_memory_pressure::GpuMemoryPressure::Constrained
            && self.graphics_mode == mahjuro_gfx_types::GraphicsMode::LowMemory;

        if self.poll_pinned_room_gpu_bit != Some(ROOM_SHOP) {
            self.maybe_upload_one_room_env_eager(
                done,
                ROOM_SHOP,
                crate::room_glb::shop_cpu_ready_for_gpu_upload(),
                |r| r.ensure_shop_room_gpu(),
                pressure,
            );
            if *done {
                return;
            }
        }
        if self.poll_pinned_room_gpu_bit != Some(ROOM_ARCHIVE) {
            self.maybe_upload_one_room_env_eager(
                done,
                ROOM_ARCHIVE,
                crate::archive_glb::archive_cpu_ready_for_gpu_upload(),
                |r| r.ensure_archive_room_gpu(),
                pressure,
            );
            if *done {
                return;
            }
        }
        if hub_only {
            return;
        }
        if self.poll_pinned_room_gpu_bit != Some(ROOM_HALLWAY) {
            self.maybe_upload_one_room_env_eager(
                done,
                ROOM_HALLWAY,
                crate::hallway_glb::hallway_cpu_ready_for_gpu_upload(),
                |r| r.ensure_hallway_room_gpu(),
                pressure,
            );
            if *done {
                return;
            }
        }
        if self.poll_pinned_room_gpu_bit != Some(ROOM_GAMEPLAY)
            && crate::gameplay_glb::gameplay_cpu_ready_for_gpu_upload()
            && self.rooms_gpu_loaded & ROOM_GAMEPLAY == 0
        {
            let budget = gameplay_eager_upload_budget_ms(self.graphics_mode);
            if self.preflight_room_gpu_headroom_for_upload(false) {
                let before = self.rooms_gpu_loaded;
                self.drive_gameplay_room_gpu_upload(budget);
                if self.rooms_gpu_loaded != before {
                    *done = true;
                    crate::gpu_memory_pressure::log_eager_preload(
                        "uploaded",
                        crate::room_gpu_resident::RoomGpuResidentId::log_label(ROOM_GAMEPLAY),
                        pressure,
                    );
                    return;
                }
                if self.gameplay_room_gpu_upload.is_some() {
                    *done = true;
                    return;
                }
            } else {
                crate::gpu_memory_pressure::log_eager_preload(
                    "paused",
                    crate::room_gpu_resident::RoomGpuResidentId::log_label(ROOM_GAMEPLAY),
                    pressure,
                );
                return;
            }
        }
        if self.poll_pinned_room_gpu_bit != Some(ROOM_STAIRCASE) {
            self.maybe_upload_one_room_env_eager(
                done,
                ROOM_STAIRCASE,
                crate::staircase_glb::staircase_cpu_ready_for_gpu_upload(),
                |r| r.ensure_staircase_room_gpu(),
                pressure,
            );
        }
    }

    /// Finish destination room GPU uploads while a scene transition is held at full black.
    fn drive_pending_scene_room_gpu_at_black(&mut self, pending: &str) {
        let budget = TRANSITION_BLACK_ROOM_GPU_UPLOAD_BUDGET_MS;
        let key = scene_keys::normalize_scene_key(pending);
        if let Some(bit) = self.room_gpu_bit_for_scene_key(key) {
            if self.integrated_low_memory_gpu() {
                self.evict_room_gpu_residents_except(bit);
            }
        }
        match key {
            scene_keys::MAIN_MENU => {
                self.ensure_room_cpu_resident_for_transition(RoomGpuResidentId::MainMenu);
                self.ensure_main_menu_room_gpu();
            }
            scene_keys::SHOP => {
                self.ensure_room_cpu_resident_for_transition(RoomGpuResidentId::Shop);
                self.drive_shop_room_gpu_upload(budget);
            }
            scene_keys::HALLWAY => {
                if self.rooms_gpu_loaded & ROOM_SHOP == 0 {
                    self.ensure_room_cpu_resident_for_transition(RoomGpuResidentId::Shop);
                    self.drive_shop_room_gpu_upload(budget);
                }
                self.ensure_room_cpu_resident_for_transition(RoomGpuResidentId::Hallway);
                self.drive_hallway_room_gpu_upload(budget);
            }
            scene_keys::STAIRWAY => {
                self.ensure_room_cpu_resident_for_transition(RoomGpuResidentId::Staircase);
                self.ensure_staircase_room_gpu();
            }
            scene_keys::ARCHIVE => {
                self.ensure_room_cpu_resident_for_transition(RoomGpuResidentId::Archive);
                self.ensure_archive_room_gpu();
            }
            scene_keys::VICTORY => {
                if victory_uses_3d_moon(self.graphics_mode) {
                    self.ensure_room_cpu_resident_for_transition(RoomGpuResidentId::MainMenu);
                    self.ensure_main_menu_room_gpu();
                }
            }
            scene_keys::GAMEPLAY | scene_keys::DEFEAT => {
                self.ensure_room_cpu_resident_for_transition(RoomGpuResidentId::Gameplay);
                self.drive_gameplay_room_gpu_upload(budget);
            }
            _ => {}
        }
    }

    /// Ensure destination-room CPU data is available while transition is held at full black.
    ///
    /// This helper is only called from `drive_pending_scene_room_gpu_at_black`, so blocking is
    /// always safe and guarantees forward progress when CPU decode residency caps are saturated.
    fn ensure_room_cpu_resident_for_transition(&self, id: RoomGpuResidentId) {
        let desc = id.desc();
        if (desc.cpu_decoded)() {
            return;
        }
        (desc.start_cpu_prefetch)();
        // Staircase prefetch is kicked from the eager chain, not per-room start hook.
        if id == RoomGpuResidentId::Staircase {
            crate::room_preload::kick_eager_all_room_cpu_prefetches();
        }
        match id {
            RoomGpuResidentId::MainMenu => {
                crate::room_preload::join_main_menu_cpu_prefetch_blocking();
                let _ = crate::main_menu_glb::with_main_menu_glb_cpu(|_| ());
            }
            RoomGpuResidentId::Shop => {
                crate::room_preload::join_shop_cpu_prefetch_blocking();
                let _ = crate::room_glb::with_shop_glb_cpu(|_| ());
            }
            RoomGpuResidentId::Hallway => {
                crate::room_preload::join_hallway_cpu_prefetch_blocking();
                let _ = crate::hallway_glb::with_hallway_glb_cpu(|_| ());
            }
            RoomGpuResidentId::Archive => {
                crate::room_preload::join_archive_cpu_prefetch_blocking();
                let _ = crate::archive_glb::with_archive_glb_cpu(|_| ());
            }
            RoomGpuResidentId::Gameplay => {
                crate::room_preload::join_gameplay_cpu_prefetch_blocking();
                let _ = crate::gameplay_glb::with_gameplay_glb_cpu(|_| ());
            }
            RoomGpuResidentId::Staircase => {
                let _ = crate::staircase_glb::with_staircase_glb_cpu(|_| ());
            }
        }
    }

    /// Run at most one full room GLB GPU upload per [`poll_room_prefetch_gpu_uploads`] call.
    fn maybe_upload_one_room_env(
        &mut self,
        done: &mut bool,
        bit: u8,
        ready: bool,
        upload: impl FnOnce(&mut Self),
    ) {
        if *done || !ready || self.rooms_gpu_loaded & bit != 0 {
            return;
        }
        if !self.preflight_room_gpu_headroom_for_upload(true) {
            return;
        }
        if bit == ROOM_SHOP {
            let before = self.rooms_gpu_loaded;
            self.drive_shop_room_gpu_upload(ROOM_ENV_GPU_UPLOAD_BUDGET_MS);
            if self.rooms_gpu_loaded != before || self.shop_room_gpu_upload.is_some() {
                *done = true;
            }
            return;
        }
        if bit == ROOM_HALLWAY {
            let before = self.rooms_gpu_loaded;
            self.drive_hallway_room_gpu_upload(ROOM_ENV_GPU_UPLOAD_BUDGET_MS);
            if self.rooms_gpu_loaded != before || self.hallway_room_gpu_upload.is_some() {
                *done = true;
            }
            return;
        }
        let before = self.rooms_gpu_loaded;
        upload(self);
        if self.rooms_gpu_loaded != before {
            *done = true;
        }
    }

    /// Eager warm-up upload gated on memory pressure (skips when at cap and nothing evictable).
    fn maybe_upload_one_room_env_eager(
        &mut self,
        done: &mut bool,
        bit: u8,
        ready: bool,
        upload: impl FnOnce(&mut Self),
        pressure: crate::gpu_memory_pressure::GpuMemoryPressure,
    ) {
        if *done || !ready || self.rooms_gpu_loaded & bit != 0 {
            return;
        }
        if !self.preflight_room_gpu_headroom_for_upload(false) {
            crate::gpu_memory_pressure::log_eager_preload(
                "paused",
                crate::room_gpu_resident::RoomGpuResidentId::log_label(bit),
                pressure,
            );
            return;
        }
        if bit == ROOM_SHOP {
            let before = self.rooms_gpu_loaded;
            self.drive_shop_room_gpu_upload(room_env_eager_upload_budget_ms(self.graphics_mode));
            if self.rooms_gpu_loaded != before {
                *done = true;
                crate::gpu_memory_pressure::log_eager_preload(
                    "uploaded",
                    crate::room_gpu_resident::RoomGpuResidentId::log_label(bit),
                    pressure,
                );
            } else if self.shop_room_gpu_upload.is_some() {
                *done = true;
            }
            return;
        }
        if bit == ROOM_HALLWAY {
            let before = self.rooms_gpu_loaded;
            self.drive_hallway_room_gpu_upload(room_env_eager_upload_budget_ms(self.graphics_mode));
            if self.rooms_gpu_loaded != before {
                *done = true;
                crate::gpu_memory_pressure::log_eager_preload(
                    "uploaded",
                    crate::room_gpu_resident::RoomGpuResidentId::log_label(bit),
                    pressure,
                );
            } else if self.hallway_room_gpu_upload.is_some() {
                *done = true;
            }
            return;
        }
        let before = self.rooms_gpu_loaded;
        upload(self);
        if self.rooms_gpu_loaded != before {
            *done = true;
            crate::gpu_memory_pressure::log_eager_preload(
                "uploaded",
                crate::room_gpu_resident::RoomGpuResidentId::log_label(bit),
                pressure,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_room_lightmap_uv_rects_truncates_and_pads() {
        let rects = vec![[0.1; 4], [0.2; 4], [0.3; 4]];
        assert_eq!(
            align_room_lightmap_uv_rects(RoomGiRoom::MainMenu, rects.clone(), 2),
            vec![[0.1; 4], [0.2; 4]]
        );
        assert_eq!(
            align_room_lightmap_uv_rects(RoomGiRoom::MainMenu, rects, 4),
            vec![[0.1; 4], [0.2; 4], [0.3; 4], [0.0; 4]]
        );
    }

    #[test]
    fn splash_eager_room_gpu_mask_low_memory_is_main_menu_only() {
        assert_eq!(
            splash_eager_room_gpu_mask(mahjuro_gfx_types::GraphicsMode::LowMemory),
            ROOM_MAIN_MENU
        );
    }

    #[test]
    fn splash_eager_room_gpu_mask_performance_includes_all_hub_run_rooms() {
        let mask = splash_eager_room_gpu_mask(mahjuro_gfx_types::GraphicsMode::Performance);
        assert_eq!(
            mask,
            ROOM_MAIN_MENU
                | ROOM_SHOP
                | ROOM_ARCHIVE
                | ROOM_HALLWAY
                | ROOM_GAMEPLAY
                | ROOM_STAIRCASE
        );
    }

    #[test]
    fn splash_eager_rooms_gpu_loaded_respects_mode_mask() {
        assert!(!splash_eager_rooms_gpu_loaded(
            mahjuro_gfx_types::GraphicsMode::Performance,
            ROOM_MAIN_MENU
        ));
        assert!(splash_eager_rooms_gpu_loaded(
            mahjuro_gfx_types::GraphicsMode::LowMemory,
            ROOM_MAIN_MENU
        ));
    }

    #[test]
    fn room_env_shader_flags_hallway_walls_only() {
        assert_eq!(
            room_env_shader_flags(
                scene_keys::HALLWAY,
                Some(crate::hallway_glb::HALLWAY_WALLS_NODE),
                Some("wall"),
            ),
            GLTF_PBR_FLAG_ROOM_HALLWAY_WALL_TINT
        );
        assert_eq!(
            room_env_shader_flags(scene_keys::HALLWAY, Some("ceiling"), Some("ceiling")),
            0
        );
    }

    #[test]
    fn room_env_baked_contact_ao_flags_hallway_broad_surfaces_skip() {
        assert_eq!(
            room_env_baked_contact_ao_flags(
                scene_keys::HALLWAY,
                Some(crate::hallway_glb::HALLWAY_WALLS_NODE),
                Some("wall"),
            ),
            GLTF_PBR_FLAG_SKIP_BAKED_CONTACT_AO
        );
        assert_eq!(
            room_env_baked_contact_ao_flags(scene_keys::HALLWAY, Some("ceiling"), Some("ceiling")),
            GLTF_PBR_FLAG_SKIP_BAKED_CONTACT_AO
        );
        assert_eq!(
            room_env_baked_contact_ao_flags(
                scene_keys::HALLWAY,
                Some("floor"),
                Some("Sauna Room planks"),
            ),
            0
        );
    }

    #[test]
    fn shop_dynamic_shadow_receiver_flags_only_receiver_surfaces() {
        assert_eq!(
            shop_dynamic_shadow_receiver_flags(
                scene_keys::SHOP,
                Some("cubby_recess_01"),
                Some("Red velvet"),
            ),
            GLTF_PBR_FLAG_ROOM_DYNAMIC_SHADOW_RECEIVER
        );
        assert_eq!(
            shop_dynamic_shadow_receiver_flags(
                scene_keys::SHOP,
                Some("ManekinekoB"),
                Some("Material.001"),
            ),
            0
        );
        assert_eq!(
            room_env_baked_contact_ao_flags(
                scene_keys::SHOP,
                Some("ManekinekoB"),
                Some("Material.001"),
            ),
            GLTF_PBR_FLAG_SKIP_BAKED_CONTACT_AO
        );
    }

    #[test]
    fn room_env_shader_flags_archive_decal_hosts() {
        let readable_decal_flags = GLTF_PBR_FLAG_ROOM_ARCHIVE_DECAL
            | GLTF_PBR_FLAG_ROOM_READABLE_SURFACE
            | GLTF_PBR_FLAG_SKIP_BAKED_CONTACT_AO;
        assert_eq!(
            room_env_shader_flags(
                scene_keys::ARCHIVE,
                Some(crate::archive_glb::SIGN_DESCRIPTION_LEFT),
                Some("sign"),
            ),
            readable_decal_flags
        );
        assert_eq!(
            room_env_shader_flags(
                scene_keys::ARCHIVE,
                Some(crate::archive_glb::SIGN_DESCRIPTION_RIGHT),
                Some("sign"),
            ),
            readable_decal_flags
        );
        assert_eq!(
            room_env_shader_flags(
                scene_keys::ARCHIVE,
                Some(crate::archive_glb::INSPECT_PLAQUE),
                Some("plaque"),
            ),
            readable_decal_flags
        );
        assert_eq!(
            room_env_shader_flags(
                scene_keys::ARCHIVE,
                Some(crate::archive_glb::PLAQUE_BACKING),
                Some("plaque"),
            ),
            GLTF_PBR_FLAG_ROOM_READABLE_SURFACE | GLTF_PBR_FLAG_SKIP_BAKED_CONTACT_AO
        );
    }

    #[test]
    fn room_env_surface_role_marks_readable_ui_surfaces() {
        assert_eq!(
            room_env_surface_role(scene_keys::ARCHIVE, Some("text_scene_title"), Some("Brass")),
            RoomEnvSurfaceRole::ReadableSurface
        );
        assert_eq!(
            room_env_surface_role(scene_keys::ARCHIVE, Some("btn_page_left"), Some("Wood.001")),
            RoomEnvSurfaceRole::ReadableSurface
        );
        assert_eq!(
            room_env_surface_role(
                scene_keys::GAMEPLAY,
                Some("btn_cash_in"),
                Some("Casted Iron")
            ),
            RoomEnvSurfaceRole::StaticRoom
        );
        assert_eq!(
            room_env_surface_role(scene_keys::ARCHIVE, Some("Cubby.001"), Some("Wood.001")),
            RoomEnvSurfaceRole::StaticRoom
        );
        assert_eq!(
            room_env_surface_role(scene_keys::SHOP, Some("floor"), Some("Dark wood")),
            RoomEnvSurfaceRole::StaticRoom
        );
    }

    #[test]
    fn room_env_shader_flags_candle_wax_primitives_only() {
        let shop_wax =
            room_env_shader_flags(scene_keys::SHOP, Some("Candle.006"), Some("Candle wax.002"));
        assert_eq!(shop_wax, GLTF_PBR_FLAG_ROOM_CANDLE_WAX);
        assert_eq!(
            room_env_shader_flags(
                scene_keys::SHOP,
                Some("Candle.006"),
                Some("Almost black torch wick"),
            ),
            0
        );
        assert_eq!(
            room_env_shader_flags(
                scene_keys::GAMEPLAY,
                Some("candles.003"),
                Some("Cream Scratched Porcelain"),
            ),
            GLTF_PBR_FLAG_ROOM_CANDLE_WAX
        );
        assert_eq!(
            room_env_shader_flags(
                scene_keys::GAMEPLAY,
                Some("candle_wicks.003"),
                Some("Charcoal Wood"),
            ),
            0
        );
    }

    #[test]
    fn room_env_shader_flags_main_menu_moon_and_star() {
        assert_eq!(
            room_env_shader_flags(
                scene_keys::MAIN_MENU,
                Some(crate::main_menu_glb::MAIN_MENU_MOON_MESH_NODE),
                Some("moon"),
            ),
            GLTF_PBR_FLAG_MAIN_MENU_MOON_PHASE
        );
        assert_eq!(
            room_env_shader_flags(scene_keys::MAIN_MENU, Some("star.001"), Some("star")),
            GLTF_PBR_FLAG_MAIN_MENU_STAR_RAINBOW
        );
        assert_eq!(
            room_env_shader_flags(scene_keys::MAIN_MENU, Some("dock"), Some("dock")),
            0
        );
    }

    #[test]
    fn room_env_shader_flags_scene_scoped() {
        assert_eq!(
            room_env_shader_flags(
                scene_keys::SHOP,
                Some(crate::hallway_glb::HALLWAY_WALLS_NODE),
                Some("wall"),
            ),
            0
        );
        assert_eq!(
            room_env_shader_flags("unknown_scene", Some("star.001"), Some("star")),
            0
        );
    }
}
