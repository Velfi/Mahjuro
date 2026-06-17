//! Offline baked room GI lightmaps for static room GLB scenes (shop / hallway /
//! staircase / archive / main menu / gameplay / shadow test room).
//!
//! Lightmaps are filled by `mahjuro-bake --kinds lightmap` with a dedicated
//! offline GPU compute tracer against authored room GLB geometry/materials and
//! existing embedded punctual lights. Runtime room shaders sample the resulting
//! RLM texture directly.

use std::sync::Arc;

use anyhow::Context;
use glam::{Vec2, Vec3, Vec4};
use gltf::texture::WrappingMode;
use wgpu::util::DeviceExt;

use crate::room_glb;
use crate::tile_glb::{GltfAlphaMode, LoadedPrimitive, RgbaTextureCpu};
use crate::wgpu_renderer::PointLight;
use mahjuro_assets::asset_path;

const LIGHTMAP_MAGIC: &[u8; 4] = b"RLM1";
pub const LIGHTMAP_VERSION: u32 = 2;
pub const LIGHTMAP_FORMAT_RGBA32F_LINEAR: u32 = 1;
const LIGHTMAP_HEADER_BYTES: usize = 36;
const ROOM_GI_BAKE_SHADER: &str = concat!(
    include_str!("../../../shaders/scene_pbr_core.wgsl"),
    "\n",
    include_str!("../../../shaders/room_gi_bake.wgsl"),
);
/// GPU bake primary directions per lightmap texel.
pub const ROOM_GI_GPU_DIR_SAMPLES: u32 = 128;
/// Minimum primary directions before adaptive lightmap sampling may stop.
pub const ROOM_GI_GPU_ADAPTIVE_MIN_DIR_SAMPLES: u32 = 64;
/// Relative standard-error target for adaptive primary radiance sampling.
pub const ROOM_GI_GPU_ADAPTIVE_REL_STDERR: f32 = 0.035;
/// Deterministic one-bounce cosine-hemisphere samples per primary hit.
pub const ROOM_GI_GPU_SECONDARY_SAMPLES: u32 = 8;
const ROOM_GI_TRACE_MAX_WORLD: f32 = 250_000.0;
const ROOM_GI_GPU_BVH_LEAF_MAX: usize = 8;
const ROOM_GI_GPU_BVH_BOUNDS_PAD: f32 = 1.0e-3;
const ROOM_GI_LIGHTMAP_DENOISE_PASSES: usize = 1;
const ROOM_GI_LIGHTMAP_DILATE_PASSES: usize = 8;
const ROOM_GI_LIGHTMAP_MAX_CHANNEL: f32 = 1.0e5;
const ROOM_GI_LIGHTMAP_MIN_LIT_FRACTION: f32 = 0.01;
/// Shadow & AO lab: mostly shadow receivers; a few directly lit texels are enough.
const ROOM_GI_LIGHTMAP_MIN_LIT_FRACTION_SHADOW_TEST: f32 = 5.0e-5;
const ROOM_GI_LIGHTMAP_MIN_AVG_LUMA_SHADOW_TEST: f64 = 1.0e-5;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoomGiRoom {
    Shop = 0,
    Hallway = 1,
    Archive = 2,
    MainMenu = 3,
    Stairway = 4,
    Gameplay = 5,
    ShadowTestRoom = 6,
}

pub const ROOM_GI_ROOM_COUNT: usize = 7;

#[inline]
pub fn room_gi_room_index(room: RoomGiRoom) -> usize {
    match room {
        RoomGiRoom::Shop => 0,
        RoomGiRoom::Hallway => 1,
        RoomGiRoom::Archive => 2,
        RoomGiRoom::MainMenu => 3,
        RoomGiRoom::Stairway => 4,
        RoomGiRoom::Gameplay => 5,
        RoomGiRoom::ShadowTestRoom => 6,
    }
}

impl RoomGiRoom {
    pub const ALL: [Self; ROOM_GI_ROOM_COUNT] = [
        Self::Shop,
        Self::Hallway,
        Self::Archive,
        Self::MainMenu,
        Self::Stairway,
        Self::Gameplay,
        Self::ShadowTestRoom,
    ];

    pub fn slug(self) -> &'static str {
        mahjuro_bake_stamp::room_slugs::ALL[room_gi_room_index(self)]
    }

    /// Offline bake filename under a room bake output directory.
    pub fn offline_bake_filename(self, ext: &str) -> String {
        format!("{}.{}", self.slug(), ext)
    }

    pub fn lightmap_asset_path(self) -> &'static str {
        match self {
            Self::Shop => "data/room_lightmap/shop.lightmap.rlm",
            Self::Hallway => "data/room_lightmap/hallway.lightmap.rlm",
            Self::Archive => "data/room_lightmap/archive.lightmap.rlm",
            Self::MainMenu => "data/room_lightmap/main_menu.lightmap.rlm",
            Self::Stairway => "data/room_lightmap/stairway.lightmap.rlm",
            Self::Gameplay => "data/room_lightmap/gameplay.lightmap.rlm",
            Self::ShadowTestRoom => "data/room_lightmap/shadow_test_room.lightmap.rlm",
        }
    }

    pub fn scene_key(self) -> &'static str {
        match self {
            Self::Shop => crate::scene_keys::SHOP,
            Self::Hallway => crate::scene_keys::HALLWAY,
            Self::Archive => crate::scene_keys::ARCHIVE,
            Self::MainMenu => crate::scene_keys::MAIN_MENU,
            Self::Stairway => crate::scene_keys::STAIRWAY,
            Self::Gameplay => crate::scene_keys::GAMEPLAY,
            Self::ShadowTestRoom => crate::scene_keys::SHADOW_AO_LAB,
        }
    }

    pub fn from_ops(
        shop: bool,
        hallway: bool,
        staircase: bool,
        archive: bool,
        main_menu: bool,
        gameplay: bool,
    ) -> Option<Self> {
        if shop {
            Some(Self::Shop)
        } else if hallway {
            Some(Self::Hallway)
        } else if staircase {
            Some(Self::Stairway)
        } else if archive {
            Some(Self::Archive)
        } else if main_menu {
            Some(Self::MainMenu)
        } else if gameplay {
            Some(Self::Gameplay)
        } else {
            None
        }
    }
}

fn room_gi_room_from_u32(room: u32) -> anyhow::Result<RoomGiRoom> {
    match room {
        0 => Ok(RoomGiRoom::Shop),
        1 => Ok(RoomGiRoom::Hallway),
        2 => Ok(RoomGiRoom::Archive),
        3 => Ok(RoomGiRoom::MainMenu),
        4 => Ok(RoomGiRoom::Stairway),
        5 => Ok(RoomGiRoom::Gameplay),
        6 => Ok(RoomGiRoom::ShadowTestRoom),
        n => anyhow::bail!("{n}"),
    }
}

/// Optional runtime room lightmap texture (`.lightmap.rlm` / `.zst`).
pub fn load_room_gi_lightmap(room: RoomGiRoom) -> anyhow::Result<Option<RoomGiLightmapBake>> {
    let path = room.lightmap_asset_path();
    let compressed_path = format!("{path}.zst");
    if let Some(file) = asset_path::get_cached(&compressed_path) {
        let raw = zstd::decode_all(file.as_ref())
            .with_context(|| format!("{compressed_path}: zstd decode"))?;
        return RoomGiLightmapBake::decode_rgba32f_texture_for_room(&raw, room)
            .map(Some)
            .with_context(|| format!("{compressed_path}: decode room lightmap"));
    }
    if let Some(file) = asset_path::get_cached(path) {
        return RoomGiLightmapBake::decode_rgba32f_texture_for_room(file.as_ref(), room)
            .map(Some)
            .with_context(|| format!("{path}: decode room lightmap"));
    }
    Ok(None)
}

#[derive(Clone, Copy, Debug)]
pub struct RoomGiGpuBakeParams {
    pub room: RoomGiRoom,
    pub bake_width: u32,
    pub bake_height: u32,
    pub lighting: room_glb::RoomEnvLightingTune,
    /// Scene Look room height scale before room-specific scale transforms such as main-menu size.
    pub height_scale: f32,
}

pub struct RoomGiLightmapBake {
    pub room: RoomGiRoom,
    pub width: u32,
    pub height: u32,
    /// Per room primitive UV transform into the lightmap atlas: xy offset, zw scale.
    pub primitive_uv_rects: Vec<[f32; 4]>,
    /// Linear HDR RGBA, row-major.
    pub pixels_rgba_f32: Vec<f32>,
}

impl RoomGiLightmapBake {
    pub fn encode_rgba32f_texture(&self) -> anyhow::Result<Vec<u8>> {
        let texel_count = self.lightmap_texel_count()?;
        let expected_values = texel_count
            .checked_mul(4)
            .ok_or_else(|| anyhow::anyhow!("room GI lightmap value count overflow"))?;
        anyhow::ensure!(
            self.pixels_rgba_f32.len() == expected_values,
            "room GI lightmap payload size mismatch (expected {} floats, got {})",
            expected_values,
            self.pixels_rgba_f32.len()
        );
        anyhow::ensure!(
            self.primitive_uv_rects.len() <= u32::MAX as usize,
            "room GI lightmap primitive UV rect count overflow"
        );

        let payload_bytes = expected_values
            .checked_mul(std::mem::size_of::<f32>())
            .ok_or_else(|| anyhow::anyhow!("room GI lightmap byte count overflow"))?;
        let rect_payload_bytes = self
            .primitive_uv_rects
            .len()
            .checked_mul(4)
            .and_then(|n| n.checked_mul(std::mem::size_of::<f32>()))
            .ok_or_else(|| anyhow::anyhow!("room GI lightmap UV rect byte count overflow"))?;
        let mut out =
            Vec::with_capacity(LIGHTMAP_HEADER_BYTES + payload_bytes + rect_payload_bytes);
        out.extend_from_slice(LIGHTMAP_MAGIC);
        push_le_u32(&mut out, LIGHTMAP_VERSION);
        push_le_u32(&mut out, self.room as u32);
        push_le_u32(&mut out, self.width);
        push_le_u32(&mut out, self.height);
        push_le_u32(&mut out, LIGHTMAP_FORMAT_RGBA32F_LINEAR);
        push_le_u32(&mut out, texel_count as u32);
        push_le_u32(&mut out, self.primitive_uv_rects.len() as u32);
        push_le_u32(&mut out, 0);
        debug_assert_eq!(out.len(), LIGHTMAP_HEADER_BYTES);

        for value in &self.pixels_rgba_f32 {
            let finite = if value.is_finite() { *value } else { 0.0 };
            out.extend_from_slice(&finite.to_le_bytes());
        }
        for rect in &self.primitive_uv_rects {
            for value in rect {
                let finite = if value.is_finite() { *value } else { 0.0 };
                out.extend_from_slice(&finite.to_le_bytes());
            }
        }
        Ok(out)
    }

    pub fn decode_rgba32f_texture_for_room(
        bytes: &[u8],
        expected_room: RoomGiRoom,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            bytes.len() >= LIGHTMAP_HEADER_BYTES,
            "room GI lightmap file too small ({} bytes)",
            bytes.len()
        );
        anyhow::ensure!(
            &bytes[0..4] == LIGHTMAP_MAGIC,
            "room GI lightmap: bad magic"
        );
        let version = read_le_u32(bytes, 4)?;
        anyhow::ensure!(
            version == LIGHTMAP_VERSION,
            "room GI lightmap: unsupported version {} (expected {LIGHTMAP_VERSION})",
            version
        );
        let room_id = read_le_u32(bytes, 8)?;
        let room = room_gi_room_from_u32(room_id)
            .with_context(|| format!("room GI lightmap: unknown room id {room_id}"))?;
        anyhow::ensure!(
            room == expected_room,
            "room GI lightmap: file is for {room:?} but expected {expected_room:?}"
        );
        let width = read_le_u32(bytes, 12)?;
        let height = read_le_u32(bytes, 16)?;
        let format = read_le_u32(bytes, 20)?;
        anyhow::ensure!(
            format == LIGHTMAP_FORMAT_RGBA32F_LINEAR,
            "room GI lightmap: unsupported format {}",
            format
        );
        let texel_count = read_le_u32(bytes, 24)? as usize;
        let primitive_count = read_le_u32(bytes, 28)? as usize;
        let expected_texels = (width as usize)
            .checked_mul(height as usize)
            .ok_or_else(|| anyhow::anyhow!("room GI lightmap dimensions overflow"))?;
        anyhow::ensure!(
            texel_count == expected_texels,
            "room GI lightmap texel count mismatch"
        );
        let expected_payload = expected_texels
            .checked_mul(4)
            .and_then(|n| n.checked_mul(std::mem::size_of::<f32>()))
            .ok_or_else(|| anyhow::anyhow!("room GI lightmap payload overflow"))?;
        let expected_rect_payload = primitive_count
            .checked_mul(4)
            .and_then(|n| n.checked_mul(std::mem::size_of::<f32>()))
            .ok_or_else(|| anyhow::anyhow!("room GI lightmap UV rect payload overflow"))?;
        anyhow::ensure!(
            bytes.len() == LIGHTMAP_HEADER_BYTES + expected_payload + expected_rect_payload,
            "room GI lightmap payload size mismatch (expected {}, got {})",
            LIGHTMAP_HEADER_BYTES + expected_payload + expected_rect_payload,
            bytes.len()
        );
        let mut pixels_rgba_f32 = Vec::with_capacity(expected_texels * 4);
        let pixel_end = LIGHTMAP_HEADER_BYTES + expected_payload;
        for chunk in bytes[LIGHTMAP_HEADER_BYTES..pixel_end].chunks_exact(4) {
            pixels_rgba_f32.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
        }
        let mut primitive_uv_rects = Vec::with_capacity(primitive_count);
        for rect_bytes in bytes[pixel_end..].chunks_exact(16) {
            let mut rect = [0.0; 4];
            for (i, value_bytes) in rect_bytes.chunks_exact(4).enumerate() {
                rect[i] = f32::from_le_bytes([
                    value_bytes[0],
                    value_bytes[1],
                    value_bytes[2],
                    value_bytes[3],
                ]);
            }
            primitive_uv_rects.push(rect);
        }
        Ok(Self {
            room,
            width,
            height,
            primitive_uv_rects,
            pixels_rgba_f32,
        })
    }

    fn lightmap_texel_count(&self) -> anyhow::Result<usize> {
        anyhow::ensure!(
            self.width > 0 && self.height > 0,
            "room GI lightmap dimensions must be non-zero"
        );
        (self.width as usize)
            .checked_mul(self.height as usize)
            .ok_or_else(|| anyhow::anyhow!("room GI lightmap dimensions overflow"))
    }
}

fn push_le_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn read_le_u32(bytes: &[u8], offset: usize) -> anyhow::Result<u32> {
    let end = offset
        .checked_add(4)
        .ok_or_else(|| anyhow::anyhow!("room GI lightmap header offset overflow"))?;
    let chunk = bytes
        .get(offset..end)
        .ok_or_else(|| anyhow::anyhow!("room GI lightmap header truncated"))?;
    Ok(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
}

/// Bake a per-texel room lightmap preview atlas with the same GPU scene tracer.
pub fn bake_room_gi_lightmap_gpu(
    params: RoomGiGpuBakeParams,
    max_dim: u32,
) -> anyhow::Result<RoomGiLightmapBake> {
    let cpu = decode_room_glb_for_gi(params.room)?;
    let env_height_scale = effective_room_height_scale(params.room, params.height_scale);
    let gpu = create_room_gi_bake_device()?;
    let scene = GpuBakeScene::from_room(
        &params,
        &cpu,
        env_height_scale,
        gpu.max_texture_dimension_2d,
    )?;
    let lightmap = build_gpu_lightmap_texels(&scene, max_dim.max(16))?;
    validate_lightmap_mapping(params.room, &lightmap)?;
    let pixels_rgba_f32 = dispatch_room_gi_gpu_lightmap_bake(&gpu, &scene, &lightmap)?;
    validate_lightmap_radiance(
        params.room,
        &lightmap,
        &pixels_rgba_f32,
        "gpu-postprocessed",
    )?;
    log::info!(
        "room GI lightmap GPU bake {:?}: {}x{}, {} mapped texels, {} primitive charts, {} tris",
        params.room,
        lightmap.width,
        lightmap.height,
        lightmap.mapped_texels,
        lightmap.primitive_uv_rects.len(),
        scene.triangles.len(),
    );
    Ok(RoomGiLightmapBake {
        room: params.room,
        width: lightmap.width,
        height: lightmap.height,
        primitive_uv_rects: lightmap.primitive_uv_rects,
        pixels_rgba_f32,
    })
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuBakeParams {
    counts: [u32; 4],
    grid: [u32; 4],
    world_min: [f32; 4],
    world_extent: [f32; 4],
    trace_params: [f32; 4],
    lighting_params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuBakeTriangle {
    p0: [f32; 4],
    p1: [f32; 4],
    p2: [f32; 4],
    n0: [f32; 4],
    n1: [f32; 4],
    n2: [f32; 4],
    uv0_uv1: [f32; 4],
    uv2_uvemr0: [f32; 4],
    uvemr1_uvemr2: [f32; 4],
    color0: [f32; 4],
    color1: [f32; 4],
    color2: [f32; 4],
    meta: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuBakeMaterial {
    albedo_rect: [u32; 4],
    mr_rect: [u32; 4],
    emissive_rect: [u32; 4],
    factors: [f32; 4],
    emissive_factor: [f32; 4],
    flags: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuBakeLight {
    pos_range: [f32; 4],
    color_intensity: [f32; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuBakeBvhNode {
    bounds_min: [f32; 4],
    bounds_max: [f32; 4],
    meta: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuBakeLightmapTexel {
    tri: [u32; 4],
    bary: [f32; 4],
}

struct GpuBakeLightmap {
    width: u32,
    height: u32,
    primitive_uv_rects: Vec<[f32; 4]>,
    primitive_mapped_texels: Vec<usize>,
    mapped_texels: usize,
    texels: Vec<GpuBakeLightmapTexel>,
}

#[derive(Clone, Copy)]
struct GpuBakeLightmapChart {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

#[derive(Clone, Copy, Debug)]
struct PrimitiveLightmapHint {
    target_min_side: u32,
    area_scale: f64,
}

impl Default for PrimitiveLightmapHint {
    fn default() -> Self {
        Self {
            target_min_side: 0,
            area_scale: 1.0,
        }
    }
}

struct RoomGiLightmapBakeGpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
    max_texture_dimension_2d: u32,
}

struct GpuBakeScene {
    triangles: Vec<GpuBakeTriangle>,
    lightmap_uvs: Vec<[[f32; 2]; 3]>,
    primitive_lightmap_hints: Vec<PrimitiveLightmapHint>,
    materials: Vec<GpuBakeMaterial>,
    lights: Vec<GpuBakeLight>,
    bvh_nodes: Vec<GpuBakeBvhNode>,
    bvh_indices: Vec<u32>,
    albedo_atlas: BakeTextureAtlas,
    mr_atlas: BakeTextureAtlas,
    emissive_atlas: BakeTextureAtlas,
    inv_doc_scale: f32,
    ray_bias: f32,
    ambient_scale: f32,
    linear_exposure: f32,
}

impl GpuBakeScene {
    fn from_room(
        params: &RoomGiGpuBakeParams,
        cpu: &room_glb::RoomGlbCpu,
        env_height_scale: f32,
        max_texture_dimension_2d: u32,
    ) -> anyhow::Result<Self> {
        let world_scale =
            room_glb::room_env_world_scale(params.bake_height as f32, env_height_scale);
        let center_doc = cpu
            .environment_bounds_doc
            .map(|b| b.center())
            .unwrap_or(Vec3::ZERO);
        let atlas_max = max_texture_dimension_2d.min(8192).max(1);
        let mut atlases = BakeAtlasBuilders::new(atlas_max);
        let mut materials = Vec::with_capacity(cpu.environment_primitives.len());
        let mut triangles = Vec::new();
        let mut lightmap_uvs = Vec::new();
        let mut primitive_lightmap_hints = Vec::with_capacity(cpu.environment_primitives.len());
        for prim in &cpu.environment_primitives {
            primitive_lightmap_hints.push(primitive_lightmap_hint(
                params.room,
                prim.gltf_node_name.as_deref(),
            ));
            let material = GpuBakeMaterial::from_loaded(
                &prim.mesh,
                &mut atlases,
                params.lighting.gltf_emissive_scale.max(0.0),
            )
            .with_context(|| {
                format!(
                    "{:?}: pack GI bake material {:?}",
                    params.room, prim.gltf_node_name
                )
            })?;
            let material_idx = materials.len();
            materials.push(material);
            append_primitive_triangles(
                &mut triangles,
                &mut lightmap_uvs,
                &prim.mesh,
                material_idx,
                center_doc,
                world_scale,
            );
        }
        anyhow::ensure!(
            !triangles.is_empty(),
            "{:?}: no triangles available for GPU GI bake",
            params.room
        );
        anyhow::ensure!(
            lightmap_uvs.len() == triangles.len(),
            "{:?}: GI bake lightmap UV count does not match triangle count",
            params.room
        );
        let bvh = GpuBakeBvh::build(&triangles)
            .with_context(|| format!("{:?}: build GPU GI bake BVH", params.room))?;
        let lights = room_lights_for_gpu_bake(params, cpu, env_height_scale);
        if lights.is_empty() {
            log::warn!(
                "{:?}: GPU GI lightmap found no embedded punctual lights; bake will contain emissive-only radiance",
                params.room,
            );
        }
        let atlases = atlases.finish()?;
        Ok(Self {
            triangles,
            lightmap_uvs,
            primitive_lightmap_hints,
            materials,
            lights,
            bvh_nodes: bvh.nodes,
            bvh_indices: bvh.indices,
            albedo_atlas: atlases.albedo,
            mr_atlas: atlases.mr,
            emissive_atlas: atlases.emissive,
            inv_doc_scale: if world_scale.abs() > 1e-8 {
                1.0 / world_scale
            } else {
                0.0
            },
            ray_bias: (world_scale.abs() * 2.0e-4).clamp(0.025, 2.0),
            ambient_scale: params.lighting.ambient_scale.max(0.0),
            linear_exposure: params.lighting.room_glb_linear_hdr_gain().max(0.0),
        })
    }
}

fn primitive_lightmap_hint(room: RoomGiRoom, node_name: Option<&str>) -> PrimitiveLightmapHint {
    if room != RoomGiRoom::Archive {
        return PrimitiveLightmapHint::default();
    }

    let node = node_name.unwrap_or("").to_ascii_lowercase();
    match node.as_str() {
        "sign_description_left" | "sign_description_right" => PrimitiveLightmapHint {
            target_min_side: 160,
            area_scale: 7.0,
        },
        "plaque_scene_title" => PrimitiveLightmapHint {
            target_min_side: 160,
            area_scale: 10.0,
        },
        "text_scene_title" => PrimitiveLightmapHint {
            target_min_side: 192,
            area_scale: 8.0,
        },
        "btn_main_menu" | "btn_switch_save" => PrimitiveLightmapHint {
            target_min_side: 112,
            area_scale: 6.0,
        },
        "text_main_menu" | "text_switch_save" => PrimitiveLightmapHint {
            target_min_side: 128,
            area_scale: 4.0,
        },
        "text_flavor_quad" => PrimitiveLightmapHint {
            target_min_side: 144,
            area_scale: 5.0,
        },
        "btn_relics_tab" | "btn_zodiacs_tab" | "btn_bosses_tab" | "btn_talismans_tab" => {
            PrimitiveLightmapHint {
                target_min_side: 64,
                area_scale: 5.0,
            }
        }
        "text_relics_tab" | "text_zodiacs_tab" | "text_bosses_tab" | "text_talismans_tab" => {
            PrimitiveLightmapHint {
                target_min_side: 112,
                area_scale: 4.0,
            }
        }
        "btn_chronicle_tab" | "text_chronicle_cover" => PrimitiveLightmapHint {
            target_min_side: 64,
            area_scale: 3.0,
        },
        "btn_page_left" | "btn_page_right" => PrimitiveLightmapHint {
            target_min_side: 24,
            area_scale: 2.0,
        },
        _ if node.starts_with("cubby") => PrimitiveLightmapHint {
            target_min_side: 128,
            area_scale: 5.0,
        },
        _ => PrimitiveLightmapHint::default(),
    }
}

struct GpuBakeBvh {
    nodes: Vec<GpuBakeBvhNode>,
    indices: Vec<u32>,
}

#[derive(Clone, Copy)]
struct GpuBakeTriangleBounds {
    min: Vec3,
    max: Vec3,
    centroid: Vec3,
}

impl GpuBakeBvh {
    fn build(triangles: &[GpuBakeTriangle]) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !triangles.is_empty(),
            "cannot build an empty GPU GI bake BVH"
        );
        anyhow::ensure!(
            triangles.len() <= u32::MAX as usize,
            "GPU GI bake triangle count exceeds u32 indexing"
        );
        let mut tri_bounds = Vec::with_capacity(triangles.len());
        for tri in triangles {
            tri_bounds.push(triangle_bounds(tri));
        }
        let mut indices = (0..triangles.len() as u32).collect::<Vec<_>>();
        let mut nodes = Vec::with_capacity(triangles.len().saturating_mul(2));
        build_gpu_bvh_range(&mut nodes, &mut indices, &tri_bounds, 0, triangles.len())?;
        Ok(Self { nodes, indices })
    }
}

fn build_gpu_bvh_range(
    nodes: &mut Vec<GpuBakeBvhNode>,
    indices: &mut [u32],
    tri_bounds: &[GpuBakeTriangleBounds],
    start: usize,
    end: usize,
) -> anyhow::Result<u32> {
    anyhow::ensure!(start < end, "invalid empty GPU GI bake BVH range");
    let node_idx = nodes.len();
    anyhow::ensure!(
        node_idx <= u32::MAX as usize,
        "GPU GI bake BVH node count exceeds u32 indexing"
    );
    nodes.push(GpuBakeBvhNode {
        bounds_min: [0.0; 4],
        bounds_max: [0.0; 4],
        meta: [0; 4],
    });

    let mut bounds_min = Vec3::splat(f32::INFINITY);
    let mut bounds_max = Vec3::splat(f32::NEG_INFINITY);
    for &idx in &indices[start..end] {
        let b = tri_bounds[idx as usize];
        bounds_min = bounds_min.min(b.min);
        bounds_max = bounds_max.max(b.max);
    }
    let count = end - start;
    if count <= ROOM_GI_GPU_BVH_LEAF_MAX {
        nodes[node_idx] = GpuBakeBvhNode::leaf(bounds_min, bounds_max, start, count)?;
        return Ok(node_idx as u32);
    }

    let mut centroid_min = Vec3::splat(f32::INFINITY);
    let mut centroid_max = Vec3::splat(f32::NEG_INFINITY);
    for &idx in &indices[start..end] {
        let c = tri_bounds[idx as usize].centroid;
        centroid_min = centroid_min.min(c);
        centroid_max = centroid_max.max(c);
    }
    let extent = centroid_max - centroid_min;
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };
    if extent[axis] <= f32::EPSILON || !extent[axis].is_finite() {
        nodes[node_idx] = GpuBakeBvhNode::leaf(bounds_min, bounds_max, start, count)?;
        return Ok(node_idx as u32);
    }

    indices[start..end].sort_by(|&a, &b| {
        tri_bounds[a as usize].centroid[axis]
            .partial_cmp(&tri_bounds[b as usize].centroid[axis])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mid = start + count / 2;
    if mid == start || mid == end {
        nodes[node_idx] = GpuBakeBvhNode::leaf(bounds_min, bounds_max, start, count)?;
        return Ok(node_idx as u32);
    }
    let left = build_gpu_bvh_range(nodes, indices, tri_bounds, start, mid)?;
    let right = build_gpu_bvh_range(nodes, indices, tri_bounds, mid, end)?;
    nodes[node_idx] = GpuBakeBvhNode::internal(bounds_min, bounds_max, left, right);
    Ok(node_idx as u32)
}

fn triangle_bounds(tri: &GpuBakeTriangle) -> GpuBakeTriangleBounds {
    let p0 = Vec3::new(tri.p0[0], tri.p0[1], tri.p0[2]);
    let p1 = Vec3::new(tri.p1[0], tri.p1[1], tri.p1[2]);
    let p2 = Vec3::new(tri.p2[0], tri.p2[1], tri.p2[2]);
    let min = p0.min(p1).min(p2);
    let max = p0.max(p1).max(p2);
    GpuBakeTriangleBounds {
        min,
        max,
        centroid: (min + max) * 0.5,
    }
}

impl GpuBakeBvhNode {
    fn internal(bounds_min: Vec3, bounds_max: Vec3, left: u32, right: u32) -> Self {
        Self {
            bounds_min: vec3_pad(bounds_min - Vec3::splat(ROOM_GI_GPU_BVH_BOUNDS_PAD), 0.0),
            bounds_max: vec3_pad(bounds_max + Vec3::splat(ROOM_GI_GPU_BVH_BOUNDS_PAD), 0.0),
            meta: [left, right, 0, 0],
        }
    }

    fn leaf(
        bounds_min: Vec3,
        bounds_max: Vec3,
        start: usize,
        count: usize,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            start <= u32::MAX as usize && count <= u32::MAX as usize,
            "GPU GI bake BVH leaf exceeds u32 indexing"
        );
        Ok(Self {
            bounds_min: vec3_pad(bounds_min - Vec3::splat(ROOM_GI_GPU_BVH_BOUNDS_PAD), 0.0),
            bounds_max: vec3_pad(bounds_max + Vec3::splat(ROOM_GI_GPU_BVH_BOUNDS_PAD), 0.0),
            meta: [start as u32, count as u32, 1, 0],
        })
    }
}

fn build_gpu_lightmap_texels(
    scene: &GpuBakeScene,
    max_dim: u32,
) -> anyhow::Result<GpuBakeLightmap> {
    let primitive_count = scene.materials.len();
    anyhow::ensure!(
        primitive_count > 0,
        "GI lightmap charting requires at least one primitive"
    );
    anyhow::ensure!(
        primitive_count <= u32::MAX as usize,
        "GI lightmap primitive count exceeds u32 indexing"
    );
    let max_dim = max_dim.max(16);
    let (width, height, charts, primitive_uv_rects) =
        build_area_weighted_lightmap_charts(scene, primitive_count, max_dim)?;
    let mut texels = vec![GpuBakeLightmapTexel::empty(); (width as usize) * (height as usize)];

    for (tri_idx, tri) in scene.triangles.iter().enumerate() {
        let chart = charts
            .get(tri.meta[0] as usize)
            .ok_or_else(|| anyhow::anyhow!("GI lightmap triangle chart index out of range"))?;
        let uv = scene
            .lightmap_uvs
            .get(tri_idx)
            .ok_or_else(|| anyhow::anyhow!("GI lightmap triangle UV index out of range"))?;
        let p = [
            lightmap_uv_to_pixel(Vec2::from_array(uv[0]), *chart),
            lightmap_uv_to_pixel(Vec2::from_array(uv[1]), *chart),
            lightmap_uv_to_pixel(Vec2::from_array(uv[2]), *chart),
        ];
        if (p[1] - p[0]).perp_dot(p[2] - p[0]).abs() < 1.0e-5 {
            continue;
        }
        let min_x = p
            .iter()
            .map(|v| v.x.floor() as i32)
            .min()
            .unwrap_or(0)
            .clamp(0, width as i32 - 1);
        let max_x = p
            .iter()
            .map(|v| v.x.ceil() as i32)
            .max()
            .unwrap_or(0)
            .clamp(0, width as i32 - 1);
        let min_y = p
            .iter()
            .map(|v| v.y.floor() as i32)
            .min()
            .unwrap_or(0)
            .clamp(0, height as i32 - 1);
        let max_y = p
            .iter()
            .map(|v| v.y.ceil() as i32)
            .max()
            .unwrap_or(0)
            .clamp(0, height as i32 - 1);
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let center = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let Some(bary) = barycentric_2d(center, p[0], p[1], p[2]) else {
                    continue;
                };
                if bary.min_element() < -0.001 {
                    continue;
                }
                let idx = y as usize * width as usize + x as usize;
                texels[idx] = GpuBakeLightmapTexel {
                    tri: [tri_idx as u32, 0, 0, 0],
                    bary: [bary.x, bary.y, bary.z, 0.0],
                };
            }
        }
    }
    let mut primitive_mapped_texels = vec![0usize; primitive_count];
    let mut mapped_texels = 0usize;
    for texel in &texels {
        if !texel.is_mapped() {
            continue;
        }
        mapped_texels += 1;
        if let Some(tri) = scene.triangles.get(texel.tri[0] as usize)
            && let Some(count) = primitive_mapped_texels.get_mut(tri.meta[0] as usize)
        {
            *count += 1;
        }
    }
    anyhow::ensure!(
        mapped_texels > 0,
        "GI lightmap UV rasterization produced no mapped texels"
    );
    Ok(GpuBakeLightmap {
        width,
        height,
        primitive_uv_rects,
        primitive_mapped_texels,
        mapped_texels,
        texels,
    })
}

#[derive(Clone, Copy, Default)]
struct PrimitiveLightmapStats {
    area: f64,
    triangles: u32,
}

#[derive(Clone, Copy)]
struct LightmapChartRequest {
    primitive_idx: usize,
    side: u32,
    min_side: u32,
}

fn build_area_weighted_lightmap_charts(
    scene: &GpuBakeScene,
    primitive_count: usize,
    max_dim: u32,
) -> anyhow::Result<(u32, u32, Vec<GpuBakeLightmapChart>, Vec<[f32; 4]>)> {
    let mut stats = vec![PrimitiveLightmapStats::default(); primitive_count];
    for tri in &scene.triangles {
        let primitive_idx = tri.meta[0] as usize;
        if let Some(stat) = stats.get_mut(primitive_idx) {
            stat.area += gpu_triangle_area(tri) as f64;
            stat.triangles = stat.triangles.saturating_add(1);
        }
    }
    let total_area = stats
        .iter()
        .enumerate()
        .map(|(primitive_idx, stat)| {
            let hint = scene
                .primitive_lightmap_hints
                .get(primitive_idx)
                .copied()
                .unwrap_or_default();
            stat.area.max(0.0) * hint.area_scale.max(0.0)
        })
        .sum::<f64>()
        .max(0.0);
    let max_chart_side = max_dim.saturating_sub(2).max(1);
    let hard_min_side = max_dim.min(6).max(1);
    let atlas_budget = (max_dim as f64 * max_dim as f64 * 0.68)
        .max(primitive_count as f64 * hard_min_side as f64 * hard_min_side as f64);
    let mut desired = Vec::with_capacity(primitive_count);
    for (primitive_idx, stat) in stats.iter().enumerate() {
        let hint = scene
            .primitive_lightmap_hints
            .get(primitive_idx)
            .copied()
            .unwrap_or_default();
        let weighted_area = stat.area.max(0.0) * hint.area_scale.max(0.0);
        let area_share = if total_area > 0.0 {
            weighted_area / total_area
        } else {
            1.0 / primitive_count.max(1) as f64
        };
        let area_side = (atlas_budget * area_share).sqrt().ceil() as u32;
        let triangle_grid = ceil_sqrt_u32(stat.triangles.max(1));
        let triangle_side = triangle_grid.saturating_mul(2).max(hard_min_side);
        let min_side = next_chart_side_with_primitive_lightmap_coverage(
            scene,
            primitive_idx,
            hard_min_side,
            max_chart_side,
        );
        let base_side = area_side
            .max(triangle_side)
            .max(min_side)
            .max(hint.target_min_side.min(max_chart_side))
            .clamp(hard_min_side, max_chart_side);
        let side = next_chart_side_with_primitive_lightmap_coverage(
            scene,
            primitive_idx,
            base_side,
            max_chart_side,
        );
        desired.push(LightmapChartRequest {
            primitive_idx,
            side,
            min_side,
        });
    }

    let padding = u32::from(max_dim >= 8);
    let mut scale = 1.0f32;
    for _ in 0..24 {
        let requests = desired
            .iter()
            .map(|request| {
                let scaled_side = ((request.side as f32 * scale).ceil() as u32)
                    .clamp(request.min_side, max_chart_side);
                LightmapChartRequest {
                    primitive_idx: request.primitive_idx,
                    side: next_chart_side_with_primitive_lightmap_coverage(
                        scene,
                        request.primitive_idx,
                        scaled_side,
                        max_chart_side,
                    ),
                    min_side: request.min_side,
                }
            })
            .collect::<Vec<_>>();
        if let Some((width, height, charts)) =
            pack_lightmap_chart_requests(&requests, max_dim, padding)
        {
            let primitive_uv_rects = charts
                .iter()
                .map(|chart| {
                    lightmap_chart_uv_rect(chart.x, chart.y, chart.w, chart.h, width, height)
                })
                .collect();
            return Ok((width, height, charts, primitive_uv_rects));
        }
        scale *= 0.86;
    }

    anyhow::bail!(
        "GI lightmap area-weighted primitive charts could not fit {} charts into {}x{}",
        primitive_count,
        max_dim,
        max_dim
    )
}

fn pack_lightmap_chart_requests(
    requests: &[LightmapChartRequest],
    max_dim: u32,
    padding: u32,
) -> Option<(u32, u32, Vec<GpuBakeLightmapChart>)> {
    let mut sorted = requests.to_vec();
    sorted.sort_by(|a, b| {
        b.side
            .cmp(&a.side)
            .then_with(|| a.primitive_idx.cmp(&b.primitive_idx))
    });
    let mut charts = vec![
        GpuBakeLightmapChart {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
        };
        requests.len()
    ];
    let mut x = 0u32;
    let mut y = 0u32;
    let mut row_h = 0u32;
    let mut used_w = 0u32;
    let mut used_h = 0u32;
    for request in sorted {
        let alloc_w = request.side.checked_add(padding.saturating_mul(2))?;
        let alloc_h = alloc_w;
        if alloc_w > max_dim || alloc_h > max_dim {
            return None;
        }
        if x > 0 && x.checked_add(alloc_w)? > max_dim {
            x = 0;
            y = y.checked_add(row_h)?;
            row_h = 0;
        }
        if y.checked_add(alloc_h)? > max_dim {
            return None;
        }
        charts[request.primitive_idx] = GpuBakeLightmapChart {
            x: x + padding,
            y: y + padding,
            w: request.side,
            h: request.side,
        };
        x = x.checked_add(alloc_w)?;
        row_h = row_h.max(alloc_h);
        used_w = used_w.max(x);
        used_h = used_h.max(y + alloc_h);
    }
    Some((used_w.max(1), used_h.max(1), charts))
}

fn next_chart_side_with_primitive_lightmap_coverage(
    scene: &GpuBakeScene,
    primitive_idx: usize,
    hard_min_side: u32,
    max_chart_side: u32,
) -> u32 {
    let min_side = hard_min_side.max(1).min(max_chart_side.max(1));
    for side in min_side..=max_chart_side.max(min_side) {
        if primitive_lightmap_uvs_cover_chart_texel(scene, primitive_idx, side) {
            return side;
        }
    }
    max_chart_side.max(min_side)
}

fn primitive_lightmap_uvs_cover_chart_texel(
    scene: &GpuBakeScene,
    primitive_idx: usize,
    side: u32,
) -> bool {
    let chart = GpuBakeLightmapChart {
        x: 0,
        y: 0,
        w: side.max(1),
        h: side.max(1),
    };
    for (tri_idx, tri) in scene.triangles.iter().enumerate() {
        if tri.meta[0] as usize != primitive_idx {
            continue;
        }
        let Some(uv) = scene.lightmap_uvs.get(tri_idx) else {
            continue;
        };
        if lightmap_uv_triangle_covers_chart_texel(*uv, chart) {
            return true;
        }
    }
    false
}

fn lightmap_uv_triangle_covers_chart_texel(uv: [[f32; 2]; 3], chart: GpuBakeLightmapChart) -> bool {
    let p = [
        lightmap_uv_to_pixel(Vec2::from_array(uv[0]), chart),
        lightmap_uv_to_pixel(Vec2::from_array(uv[1]), chart),
        lightmap_uv_to_pixel(Vec2::from_array(uv[2]), chart),
    ];
    if (p[1] - p[0]).perp_dot(p[2] - p[0]).abs() < 1.0e-5 {
        return false;
    }
    let max_x = chart.x.saturating_add(chart.w.saturating_sub(1)) as i32;
    let max_y = chart.y.saturating_add(chart.h.saturating_sub(1)) as i32;
    let min_x = p
        .iter()
        .map(|v| v.x.floor() as i32)
        .min()
        .unwrap_or(chart.x as i32)
        .clamp(chart.x as i32, max_x);
    let max_px = p
        .iter()
        .map(|v| v.x.ceil() as i32)
        .max()
        .unwrap_or(chart.x as i32)
        .clamp(chart.x as i32, max_x);
    let min_y = p
        .iter()
        .map(|v| v.y.floor() as i32)
        .min()
        .unwrap_or(chart.y as i32)
        .clamp(chart.y as i32, max_y);
    let max_py = p
        .iter()
        .map(|v| v.y.ceil() as i32)
        .max()
        .unwrap_or(chart.y as i32)
        .clamp(chart.y as i32, max_y);
    for y in min_y..=max_py {
        for x in min_x..=max_px {
            let center = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let Some(bary) = barycentric_2d(center, p[0], p[1], p[2]) else {
                continue;
            };
            if bary.min_element() >= -0.001 {
                return true;
            }
        }
    }
    false
}

fn gpu_triangle_area(tri: &GpuBakeTriangle) -> f32 {
    let p0 = Vec3::new(tri.p0[0], tri.p0[1], tri.p0[2]);
    let p1 = Vec3::new(tri.p1[0], tri.p1[1], tri.p1[2]);
    let p2 = Vec3::new(tri.p2[0], tri.p2[1], tri.p2[2]);
    let area = (p1 - p0).cross(p2 - p0).length() * 0.5;
    if area.is_finite() { area.max(0.0) } else { 0.0 }
}

fn lightmap_uv_to_pixel(uv: Vec2, chart: GpuBakeLightmapChart) -> Vec2 {
    let u = uv.x.clamp(0.0, 1.0);
    let v = uv.y.clamp(0.0, 1.0);
    Vec2::new(
        lightmap_chart_axis_to_pixel(chart.x, chart.w, u),
        lightmap_chart_axis_to_pixel(chart.y, chart.h, v),
    )
}

fn lightmap_chart_uv_rect(x: u32, y: u32, w: u32, h: u32, atlas_w: u32, atlas_h: u32) -> [f32; 4] {
    let sx = w.saturating_sub(1).max(1) as f32;
    let sy = h.saturating_sub(1).max(1) as f32;
    [
        (x as f32 + 0.5) / atlas_w.max(1) as f32,
        (y as f32 + 0.5) / atlas_h.max(1) as f32,
        sx / atlas_w.max(1) as f32,
        sy / atlas_h.max(1) as f32,
    ]
}

fn lightmap_chart_axis_to_pixel(origin: u32, size: u32, uv: f32) -> f32 {
    if size <= 1 {
        return origin as f32 + 0.5;
    }
    origin as f32 + 0.5 + uv * size.saturating_sub(1) as f32
}

fn validate_lightmap_mapping(room: RoomGiRoom, lightmap: &GpuBakeLightmap) -> anyhow::Result<()> {
    anyhow::ensure!(
        lightmap.primitive_uv_rects.len() == lightmap.primitive_mapped_texels.len(),
        "{room:?}: lightmap primitive rect/count mismatch"
    );
    let missing = lightmap
        .primitive_mapped_texels
        .iter()
        .enumerate()
        .filter_map(|(i, &count)| (count == 0).then_some(i))
        .take(16)
        .collect::<Vec<_>>();
    anyhow::ensure!(
        missing.is_empty(),
        "{room:?}: GI lightmap has primitives with no mapped texels: {:?}",
        missing
    );
    Ok(())
}

fn shadow_test_room_lightmap_has_radiance(
    lit_fraction: f32,
    avg_luma: f64,
    max_channel: f32,
) -> bool {
    max_channel > 0.0
        && (lit_fraction >= ROOM_GI_LIGHTMAP_MIN_LIT_FRACTION_SHADOW_TEST
            || avg_luma > ROOM_GI_LIGHTMAP_MIN_AVG_LUMA_SHADOW_TEST)
}

fn validate_lightmap_radiance(
    room: RoomGiRoom,
    lightmap: &GpuBakeLightmap,
    pixels: &[f32],
    stage: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        pixels.len() == lightmap.texels.len() * 4,
        "{room:?}: {stage} GI lightmap payload mismatch"
    );
    let mut invalid = 0usize;
    let mut first_invalid = None;
    let mut lit = 0usize;
    let mut visible = 0usize;
    let mut max_channel = 0.0f32;
    let mut luma_sum = 0.0f64;
    for (i, texel) in lightmap.texels.iter().enumerate() {
        if !texel.is_mapped() {
            continue;
        }
        let p = &pixels[i * 4..i * 4 + 4];
        if p[3].is_finite() && p[3] <= 0.5 {
            continue;
        }
        visible += 1;
        let rgb = [p[0], p[1], p[2]];
        if rgb
            .iter()
            .any(|v| !v.is_finite() || *v < -1.0e-4 || *v > ROOM_GI_LIGHTMAP_MAX_CHANNEL)
            || !p[3].is_finite()
        {
            invalid += 1;
            if first_invalid.is_none() {
                first_invalid = Some((i, [p[0], p[1], p[2], p[3]]));
            }
            continue;
        }
        max_channel = max_channel.max(rgb[0]).max(rgb[1]).max(rgb[2]);
        let luma = lightmap_luminance(rgb);
        luma_sum += luma as f64;
        if luma > 1.0e-5 {
            lit += 1;
        }
    }
    anyhow::ensure!(
        visible > 0,
        "{room:?}: {stage} GI lightmap has no visible mapped texels"
    );
    anyhow::ensure!(
        invalid == 0,
        "{room:?}: {stage} GI lightmap contains {invalid} invalid mapped texel(s); first={first_invalid:?}"
    );
    let lit_fraction = lit as f32 / visible as f32;
    let avg_luma = luma_sum / visible as f64;
    let effectively_black = match room {
        RoomGiRoom::ShadowTestRoom => {
            !shadow_test_room_lightmap_has_radiance(lit_fraction, avg_luma, max_channel)
        }
        _ => lit_fraction < ROOM_GI_LIGHTMAP_MIN_LIT_FRACTION || avg_luma <= 1.0e-7,
    };
    anyhow::ensure!(
        !effectively_black,
        "{room:?}: {stage} GI lightmap is effectively black (lit_fraction={lit_fraction:.4}, avg_luma={avg_luma:.6})"
    );
    anyhow::ensure!(
        max_channel > 0.0,
        "{room:?}: {stage} GI lightmap has no positive radiance"
    );
    Ok(())
}

fn lightmap_luminance(rgb: [f32; 3]) -> f32 {
    rgb[0].max(0.0) * 0.2126 + rgb[1].max(0.0) * 0.7152 + rgb[2].max(0.0) * 0.0722
}

fn ceil_sqrt_u32(n: u32) -> u32 {
    if n <= 1 {
        return n;
    }
    let mut x = (n as f32).sqrt().ceil() as u32;
    while x.saturating_mul(x) < n {
        x += 1;
    }
    x
}

fn barycentric_2d(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> Option<Vec3> {
    let v0 = b - a;
    let v1 = c - a;
    let v2 = p - a;
    let d00 = v0.dot(v0);
    let d01 = v0.dot(v1);
    let d11 = v1.dot(v1);
    let d20 = v2.dot(v0);
    let d21 = v2.dot(v1);
    let denom = d00 * d11 - d01 * d01;
    if denom.abs() < 1.0e-8 || !denom.is_finite() {
        return None;
    }
    let v = (d11 * d20 - d01 * d21) / denom;
    let w = (d00 * d21 - d01 * d20) / denom;
    let u = 1.0 - v - w;
    Some(Vec3::new(u, v, w))
}

impl GpuBakeLightmapTexel {
    fn empty() -> Self {
        Self {
            tri: [u32::MAX, 0, 0, 0],
            bary: [0.0; 4],
        }
    }

    fn is_mapped(&self) -> bool {
        self.tri[0] != u32::MAX
    }
}

struct BakeTextureAtlas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

#[derive(Clone, Copy)]
struct AtlasRegion {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

impl AtlasRegion {
    fn to_array(self) -> [u32; 4] {
        [self.x, self.y, self.w, self.h]
    }
}

struct PendingAtlasUpload {
    tex: RgbaTextureCpu,
    region: AtlasRegion,
}

struct TextureAtlasBuilder {
    max_dim: u32,
    default_region: AtlasRegion,
    next_x: u32,
    next_y: u32,
    row_h: u32,
    used_w: u32,
    used_h: u32,
    uploads: Vec<PendingAtlasUpload>,
    by_ptr: std::collections::HashMap<usize, AtlasRegion>,
}

impl TextureAtlasBuilder {
    fn new(max_dim: u32, default_rgba: [u8; 4]) -> Self {
        let default_tex = Arc::new((default_rgba.to_vec(), 1, 1));
        let default_region = AtlasRegion {
            x: 0,
            y: 0,
            w: 1,
            h: 1,
        };
        Self {
            max_dim: max_dim.max(1),
            default_region,
            next_x: 1,
            next_y: 0,
            row_h: 1,
            used_w: 1,
            used_h: 1,
            uploads: vec![PendingAtlasUpload {
                tex: default_tex,
                region: default_region,
            }],
            by_ptr: std::collections::HashMap::new(),
        }
    }

    fn add_texture(&mut self, tex: Option<&RgbaTextureCpu>) -> anyhow::Result<AtlasRegion> {
        let Some(tex) = tex else {
            return Ok(self.default_region);
        };
        let (pixels, width, height) = tex.as_ref();
        if pixels.is_empty() || *width == 0 || *height == 0 {
            return Ok(self.default_region);
        }
        let key = Arc::as_ptr(tex) as usize;
        if let Some(region) = self.by_ptr.get(&key).copied() {
            return Ok(region);
        }
        let w = *width;
        let h = *height;
        anyhow::ensure!(
            w <= self.max_dim && h <= self.max_dim,
            "GI bake texture {w}x{h} exceeds atlas dimension {}",
            self.max_dim
        );
        if self.next_x + w > self.max_dim {
            self.next_x = 0;
            self.next_y += self.row_h;
            self.row_h = 0;
        }
        anyhow::ensure!(
            self.next_y + h <= self.max_dim,
            "GI bake texture atlas overflow at {w}x{h}; atlas limit {}",
            self.max_dim
        );
        let region = AtlasRegion {
            x: self.next_x,
            y: self.next_y,
            w,
            h,
        };
        self.next_x += w;
        self.row_h = self.row_h.max(h);
        self.used_w = self.used_w.max(region.x + w);
        self.used_h = self.used_h.max(region.y + h);
        self.uploads.push(PendingAtlasUpload {
            tex: Arc::clone(tex),
            region,
        });
        self.by_ptr.insert(key, region);
        Ok(region)
    }

    fn finish(self) -> anyhow::Result<BakeTextureAtlas> {
        let width = self.used_w.max(1);
        let height = self.used_h.max(1);
        let mut pixels = vec![0u8; (width as usize) * (height as usize) * 4];
        for upload in self.uploads {
            let (src, src_w, src_h) = upload.tex.as_ref();
            anyhow::ensure!(
                src.len() >= (*src_w as usize) * (*src_h as usize) * 4,
                "GI bake atlas source texture payload is truncated"
            );
            for row in 0..upload.region.h {
                let src_off = (row as usize) * (*src_w as usize) * 4;
                let dst_off = (((upload.region.y + row) as usize) * (width as usize)
                    + upload.region.x as usize)
                    * 4;
                let byte_len = upload.region.w as usize * 4;
                pixels[dst_off..dst_off + byte_len]
                    .copy_from_slice(&src[src_off..src_off + byte_len]);
            }
        }
        Ok(BakeTextureAtlas {
            width,
            height,
            pixels,
        })
    }
}

struct BakeAtlasBuilders {
    albedo: TextureAtlasBuilder,
    mr: TextureAtlasBuilder,
    emissive: TextureAtlasBuilder,
}

struct BakeAtlases {
    albedo: BakeTextureAtlas,
    mr: BakeTextureAtlas,
    emissive: BakeTextureAtlas,
}

impl BakeAtlasBuilders {
    fn new(max_dim: u32) -> Self {
        Self {
            albedo: TextureAtlasBuilder::new(max_dim, [255, 255, 255, 255]),
            mr: TextureAtlasBuilder::new(max_dim, [255, 255, 255, 255]),
            emissive: TextureAtlasBuilder::new(max_dim, [255, 255, 255, 255]),
        }
    }

    fn finish(self) -> anyhow::Result<BakeAtlases> {
        Ok(BakeAtlases {
            albedo: self.albedo.finish()?,
            mr: self.mr.finish()?,
            emissive: self.emissive.finish()?,
        })
    }
}

impl GpuBakeMaterial {
    fn from_loaded(
        mesh: &LoadedPrimitive,
        atlases: &mut BakeAtlasBuilders,
        emissive_scale: f32,
    ) -> anyhow::Result<Self> {
        let alpha_mode = match mesh.alpha_mode {
            GltfAlphaMode::Opaque => 0,
            GltfAlphaMode::Mask => 1,
            GltfAlphaMode::Blend => 2,
        };
        Ok(Self {
            albedo_rect: atlases
                .albedo
                .add_texture(mesh.albedo_rgba.as_ref())?
                .to_array(),
            mr_rect: atlases
                .mr
                .add_texture(mesh.metallic_roughness_rgba.as_ref())?
                .to_array(),
            emissive_rect: atlases
                .emissive
                .add_texture(mesh.emissive_rgba.as_ref())?
                .to_array(),
            factors: [
                mesh.metallic_factor.max(0.0),
                mesh.roughness_factor.clamp(0.02, 1.0),
                mesh.alpha_cutoff.clamp(0.0, 1.0),
                emissive_scale,
            ],
            emissive_factor: [
                mesh.emissive_factor[0].max(0.0),
                mesh.emissive_factor[1].max(0.0),
                mesh.emissive_factor[2].max(0.0),
                0.0,
            ],
            flags: [
                alpha_mode,
                wrap_mode_u32(mesh.sampler.wrap_s),
                wrap_mode_u32(mesh.sampler.wrap_t),
                0,
            ],
        })
    }
}

fn append_primitive_triangles(
    out: &mut Vec<GpuBakeTriangle>,
    lightmap_uvs: &mut Vec<[[f32; 2]; 3]>,
    mesh: &LoadedPrimitive,
    material_idx: usize,
    center_doc: Vec3,
    world_scale: f32,
) {
    for tri in crate::room_lightmap_uv::room_lightmap_triangles(mesh) {
        append_triangle_from_vertices(
            out,
            lightmap_uvs,
            mesh,
            tri.source_indices,
            material_idx,
            center_doc,
            world_scale,
            tri.lightmap_uv,
        );
    }
}

fn append_triangle_from_vertices(
    out: &mut Vec<GpuBakeTriangle>,
    lightmap_uvs: &mut Vec<[[f32; 2]; 3]>,
    mesh: &LoadedPrimitive,
    ids: [usize; 3],
    material_idx: usize,
    center_doc: Vec3,
    world_scale: f32,
    lightmap_uv: [[f32; 2]; 3],
) {
    let v = [
        mesh.vertices[ids[0]],
        mesh.vertices[ids[1]],
        mesh.vertices[ids[2]],
    ];
    let p = [
        (Vec3::from_array(v[0].position) - center_doc) * world_scale,
        (Vec3::from_array(v[1].position) - center_doc) * world_scale,
        (Vec3::from_array(v[2].position) - center_doc) * world_scale,
    ];
    let face_n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
    if face_n.length_squared() < 0.5 || !vec3_finite(face_n) {
        return;
    }
    let n = [
        normal_or_face(v[0].normal, face_n),
        normal_or_face(v[1].normal, face_n),
        normal_or_face(v[2].normal, face_n),
    ];
    let uv = [
        Vec2::from_array(v[0].uv),
        Vec2::from_array(v[1].uv),
        Vec2::from_array(v[2].uv),
    ];
    let uv_emr = [
        Vec2::from_array(v[0].uv_emr),
        Vec2::from_array(v[1].uv_emr),
        Vec2::from_array(v[2].uv_emr),
    ];
    let color = [
        Vec4::from_array(v[0].color).max(Vec4::ZERO),
        Vec4::from_array(v[1].color).max(Vec4::ZERO),
        Vec4::from_array(v[2].color).max(Vec4::ZERO),
    ];
    out.push(GpuBakeTriangle {
        p0: vec3_pad(p[0], 0.0),
        p1: vec3_pad(p[1], 0.0),
        p2: vec3_pad(p[2], 0.0),
        n0: vec3_pad(n[0], 0.0),
        n1: vec3_pad(n[1], 0.0),
        n2: vec3_pad(n[2], 0.0),
        uv0_uv1: [uv[0].x, uv[0].y, uv[1].x, uv[1].y],
        uv2_uvemr0: [uv[2].x, uv[2].y, uv_emr[0].x, uv_emr[0].y],
        uvemr1_uvemr2: [uv_emr[1].x, uv_emr[1].y, uv_emr[2].x, uv_emr[2].y],
        color0: color[0].to_array(),
        color1: color[1].to_array(),
        color2: color[2].to_array(),
        meta: [material_idx as u32, 0, 0, 0],
    });
    lightmap_uvs.push(lightmap_uv);
}

fn room_lights_for_gpu_bake(
    params: &RoomGiGpuBakeParams,
    cpu: &room_glb::RoomGlbCpu,
    env_height_scale: f32,
) -> Vec<GpuBakeLight> {
    let profile = match params.room {
        RoomGiRoom::Shop | RoomGiRoom::Gameplay => {
            crate::room_gltf_punctual::RoomPunctualProfile::Candles {
                flame_time_s: 0.0,
                lamp_flicker: 1.0,
                flicker_amp: 0.0,
            }
        }
        RoomGiRoom::MainMenu => crate::room_gltf_punctual::RoomPunctualProfile::MainMenu,
        RoomGiRoom::Hallway
        | RoomGiRoom::Archive
        | RoomGiRoom::Stairway
        | RoomGiRoom::ShadowTestRoom => crate::room_gltf_punctual::RoomPunctualProfile::Standard,
    };
    crate::room_gltf_punctual::embedded_point_lights_runtime_tagged(
        cpu,
        params.bake_width as f32,
        params.bake_height as f32,
        env_height_scale,
        &params.lighting,
        profile,
        room_glb_asset_label(params.room),
    )
    .into_iter()
    .map(|tagged| bake_light_from_point(tagged.light, params.bake_width, params.bake_height))
    .collect()
}

fn bake_light_from_point(light: PointLight, width: u32, height: u32) -> GpuBakeLight {
    let pos = crate::world_space::pixel_to_world(
        width as f32,
        height as f32,
        light.pos[0],
        light.pos[1],
        light.pos[2],
    );
    let color = Vec3::from_array(light.color).max(Vec3::ZERO);
    GpuBakeLight {
        pos_range: [pos.x, pos.y, pos.z, light.radius.max(0.0)],
        color_intensity: [color.x, color.y, color.z, light.intensity.max(0.0)],
        params: [1.0, 0.0, 0.0, 0.0],
    }
}

fn create_room_gi_bake_device() -> anyhow::Result<RoomGiLightmapBakeGpu> {
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::from_env().unwrap_or_default(),
        compatible_surface: None,
        force_fallback_adapter: false,
    }))
    .map_err(|e| anyhow::anyhow!("room GI GPU bake adapter: {e:?}"))?;
    let limits = wgpu::Limits::default().using_resolution(adapter.limits());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("room-gi-bake-device"),
        required_features: wgpu::Features::empty(),
        required_limits: limits,
        experimental_features: wgpu::ExperimentalFeatures::default(),
        memory_hints: wgpu::MemoryHints::default(),
        trace: wgpu::Trace::default(),
    }))
    .map_err(|e| anyhow::anyhow!("room GI GPU bake device: {e:?}"))?;
    let info = adapter.get_info();
    log::info!(
        "room GI GPU bake adapter '{}' ({:?})",
        info.name,
        info.backend
    );
    Ok(RoomGiLightmapBakeGpu {
        device,
        queue,
        max_texture_dimension_2d: adapter.limits().max_texture_dimension_2d,
    })
}

fn dispatch_room_gi_gpu_lightmap_bake(
    gpu: &RoomGiLightmapBakeGpu,
    scene: &GpuBakeScene,
    lightmap: &GpuBakeLightmap,
) -> anyhow::Result<Vec<f32>> {
    let texel_count = lightmap.width as u64 * lightmap.height as u64;
    let output_bytes = texel_count * std::mem::size_of::<[f32; 4]>() as u64;
    let params = GpuBakeParams {
        counts: [
            scene.triangles.len() as u32,
            scene.materials.len() as u32,
            scene.lights.len() as u32,
            ROOM_GI_GPU_DIR_SAMPLES,
        ],
        grid: [lightmap.width, lightmap.height, 1, texel_count as u32],
        world_min: [0.0; 4],
        world_extent: [1.0, 1.0, 1.0, 0.0],
        trace_params: [
            scene.inv_doc_scale,
            scene.ray_bias,
            ROOM_GI_TRACE_MAX_WORLD,
            ROOM_GI_GPU_ADAPTIVE_MIN_DIR_SAMPLES as f32,
        ],
        lighting_params: [
            scene.ambient_scale,
            scene.linear_exposure,
            ROOM_GI_GPU_SECONDARY_SAMPLES as f32,
            ROOM_GI_GPU_ADAPTIVE_REL_STDERR,
        ],
    };
    let params_buffer = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("room-gi-lightmap-params"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM,
        });
    let triangle_buffer = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("room-gi-lightmap-triangles"),
            contents: bytemuck::cast_slice(&scene.triangles),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let material_buffer = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("room-gi-lightmap-materials"),
            contents: bytemuck::cast_slice(&scene.materials),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let light_payload = if scene.lights.is_empty() {
        vec![GpuBakeLight::zero()]
    } else {
        scene.lights.clone()
    };
    let light_buffer = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("room-gi-lightmap-lights"),
            contents: bytemuck::cast_slice(&light_payload),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let bvh_node_buffer = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("room-gi-lightmap-bvh-nodes"),
            contents: bytemuck::cast_slice(&scene.bvh_nodes),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let bvh_index_buffer = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("room-gi-lightmap-bvh-indices"),
            contents: bytemuck::cast_slice(&scene.bvh_indices),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let texel_buffer = gpu
        .device
        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("room-gi-lightmap-texels"),
            contents: bytemuck::cast_slice(&lightmap.texels),
            usage: wgpu::BufferUsages::STORAGE,
        });
    let albedo_texture = upload_bake_atlas(
        &gpu.device,
        &gpu.queue,
        "room-gi-lightmap-albedo-atlas",
        &scene.albedo_atlas,
    );
    let mr_texture = upload_bake_atlas(
        &gpu.device,
        &gpu.queue,
        "room-gi-lightmap-mr-atlas",
        &scene.mr_atlas,
    );
    let emissive_texture = upload_bake_atlas(
        &gpu.device,
        &gpu.queue,
        "room-gi-lightmap-emissive-atlas",
        &scene.emissive_atlas,
    );
    let albedo_view = albedo_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let mr_view = mr_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let emissive_view = emissive_texture.create_view(&wgpu::TextureViewDescriptor::default());
    let output_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("room-gi-lightmap-output"),
        size: output_bytes,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let temp_buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("room-gi-lightmap-temp"),
        size: output_bytes,
        usage: wgpu::BufferUsages::STORAGE,
        mapped_at_creation: false,
    });
    let staging = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("room-gi-lightmap-output-staging"),
        size: output_bytes,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let layout = gpu
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("room-gi-lightmap-bg-layout"),
            entries: &[
                storage_layout_entry(0, true),
                storage_layout_entry(1, true),
                storage_layout_entry(2, true),
                storage_layout_entry(3, true),
                texture_layout_entry(4),
                texture_layout_entry(5),
                texture_layout_entry(6),
                storage_layout_entry(8, true),
                storage_layout_entry(9, true),
                storage_layout_entry(10, true),
                storage_layout_entry(11, false),
                storage_layout_entry(12, false),
            ],
        });
    let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("room-gi-lightmap-bg"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: params_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: triangle_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: material_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: light_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&albedo_view),
            },
            wgpu::BindGroupEntry {
                binding: 5,
                resource: wgpu::BindingResource::TextureView(&mr_view),
            },
            wgpu::BindGroupEntry {
                binding: 6,
                resource: wgpu::BindingResource::TextureView(&emissive_view),
            },
            wgpu::BindGroupEntry {
                binding: 8,
                resource: bvh_node_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 9,
                resource: bvh_index_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 10,
                resource: texel_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 11,
                resource: output_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 12,
                resource: temp_buffer.as_entire_binding(),
            },
        ],
    });
    let module = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("room-gi-lightmap-shader"),
            source: wgpu::ShaderSource::Wgsl(ROOM_GI_BAKE_SHADER.into()),
        });
    let pipeline_layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("room-gi-lightmap-pl"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
    let lightmap_pipeline = create_room_lightmap_compute_pipeline(
        &gpu.device,
        &pipeline_layout,
        &module,
        "lightmap_main",
    );
    let alpha_pipeline = create_room_lightmap_compute_pipeline(
        &gpu.device,
        &pipeline_layout,
        &module,
        "lightmap_alpha_main",
    );
    let denoise_pipeline = create_room_lightmap_compute_pipeline(
        &gpu.device,
        &pipeline_layout,
        &module,
        "lightmap_denoise_main",
    );
    let dilate_pipeline = create_room_lightmap_compute_pipeline(
        &gpu.device,
        &pipeline_layout,
        &module,
        "lightmap_dilate_main",
    );
    let copy_pipeline = create_room_lightmap_compute_pipeline(
        &gpu.device,
        &pipeline_layout,
        &module,
        "lightmap_copy_tmp_main",
    );
    let finalize_pipeline = create_room_lightmap_compute_pipeline(
        &gpu.device,
        &pipeline_layout,
        &module,
        "lightmap_finalize_main",
    );
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("room-gi-lightmap-encoder"),
        });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("room-gi-lightmap-pass"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, &bind_group, &[]);
        let workgroups = (lightmap.width.div_ceil(8), lightmap.height.div_ceil(8), 1);
        pass.set_pipeline(&lightmap_pipeline);
        pass.dispatch_workgroups(workgroups.0, workgroups.1, workgroups.2);
        pass.set_pipeline(&alpha_pipeline);
        pass.dispatch_workgroups(workgroups.0, workgroups.1, workgroups.2);
        for _ in 0..ROOM_GI_LIGHTMAP_DENOISE_PASSES {
            pass.set_pipeline(&denoise_pipeline);
            pass.dispatch_workgroups(workgroups.0, workgroups.1, workgroups.2);
            pass.set_pipeline(&copy_pipeline);
            pass.dispatch_workgroups(workgroups.0, workgroups.1, workgroups.2);
        }
        for _ in 0..ROOM_GI_LIGHTMAP_DILATE_PASSES {
            pass.set_pipeline(&dilate_pipeline);
            pass.dispatch_workgroups(workgroups.0, workgroups.1, workgroups.2);
            pass.set_pipeline(&copy_pipeline);
            pass.dispatch_workgroups(workgroups.0, workgroups.1, workgroups.2);
        }
        pass.set_pipeline(&finalize_pipeline);
        pass.dispatch_workgroups(workgroups.0, workgroups.1, workgroups.2);
    }
    encoder.copy_buffer_to_buffer(&output_buffer, 0, &staging, 0, output_bytes);
    gpu.queue.submit(std::iter::once(encoder.finish()));

    let slice = staging.slice(..output_bytes);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        let _ = tx.send(result);
    });
    gpu.device.poll(wgpu::PollType::wait_indefinitely())?;
    rx.recv()
        .map_err(|_| anyhow::anyhow!("room GI lightmap GPU bake map channel closed"))?
        .map_err(|e| anyhow::anyhow!("room GI lightmap GPU bake map failed: {e:?}"))?;
    let mapped = slice.get_mapped_range();
    let values = bytemuck::cast_slice::<u8, f32>(&mapped).to_vec();
    drop(mapped);
    staging.unmap();
    Ok(values)
}

impl GpuBakeLight {
    fn zero() -> Self {
        Self {
            pos_range: [0.0; 4],
            color_intensity: [0.0; 4],
            params: [0.0; 4],
        }
    }
}

fn create_room_lightmap_compute_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    module: &wgpu::ShaderModule,
    entry_point: &'static str,
) -> wgpu::ComputePipeline {
    device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some(entry_point),
        layout: Some(layout),
        module,
        entry_point: Some(entry_point),
        compilation_options: wgpu::PipelineCompilationOptions::default(),
        cache: None,
    })
}

fn storage_layout_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: if binding == 0 {
                wgpu::BufferBindingType::Uniform
            } else {
                wgpu::BufferBindingType::Storage { read_only }
            },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Texture {
            multisampled: false,
            view_dimension: wgpu::TextureViewDimension::D2,
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
        },
        count: None,
    }
}

fn upload_bake_atlas(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &'static str,
    atlas: &BakeTextureAtlas,
) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: atlas.width,
            height: atlas.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let unpadded_bpr = atlas.width * 4;
    let padded_bpr = unpadded_bpr.div_ceil(256) * 256;
    let data = if padded_bpr == unpadded_bpr {
        std::borrow::Cow::Borrowed(atlas.pixels.as_slice())
    } else {
        let mut padded = vec![0u8; (padded_bpr * atlas.height) as usize];
        for y in 0..atlas.height {
            let src = (y * unpadded_bpr) as usize;
            let dst = (y * padded_bpr) as usize;
            padded[dst..dst + unpadded_bpr as usize]
                .copy_from_slice(&atlas.pixels[src..src + unpadded_bpr as usize]);
        }
        std::borrow::Cow::Owned(padded)
    };
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(padded_bpr),
            rows_per_image: Some(atlas.height),
        },
        wgpu::Extent3d {
            width: atlas.width,
            height: atlas.height,
            depth_or_array_layers: 1,
        },
    );
    texture
}

fn decode_room_glb_for_gi(room: RoomGiRoom) -> anyhow::Result<room_glb::RoomGlbCpu> {
    let path = room_glb_asset_path(room);
    let file =
        asset_path::get(path).ok_or_else(|| anyhow::anyhow!("missing {path} for room GI bake"))?;
    match room {
        RoomGiRoom::Shop => room_glb::load_shop_glb_from_bytes(&file.data),
        RoomGiRoom::Hallway => crate::hallway_glb::load_hallway_glb_from_bytes(&file.data),
        RoomGiRoom::Archive => crate::archive_glb::load_archive_glb_from_bytes(&file.data),
        RoomGiRoom::MainMenu => crate::main_menu_glb::load_main_menu_glb_from_bytes(&file.data),
        RoomGiRoom::Stairway => crate::staircase_glb::load_staircase_glb_from_bytes(&file.data),
        RoomGiRoom::Gameplay => crate::gameplay_glb::load_gameplay_glb_from_bytes(&file.data),
        RoomGiRoom::ShadowTestRoom => {
            crate::shadow_test_room_glb::load_shadow_test_room_glb_from_bytes(&file.data)
        }
    }
    .with_context(|| format!("decode {path} for room GI bake"))
}

fn room_glb_asset_path(room: RoomGiRoom) -> &'static str {
    match room {
        RoomGiRoom::Shop => "3d/shop.glb",
        RoomGiRoom::Hallway => "3d/hallway.glb",
        RoomGiRoom::Archive => "3d/archive.glb",
        RoomGiRoom::MainMenu => "3d/main_menu.glb",
        RoomGiRoom::Stairway => "3d/staircase.glb",
        RoomGiRoom::Gameplay => "3d/gameplay.glb",
        RoomGiRoom::ShadowTestRoom => "3d/shadow_test_room.glb",
    }
}

fn room_glb_asset_label(room: RoomGiRoom) -> &'static str {
    match room {
        RoomGiRoom::Shop => "shop.glb",
        RoomGiRoom::Hallway => "hallway.glb",
        RoomGiRoom::Archive => "archive.glb",
        RoomGiRoom::MainMenu => "main_menu.glb",
        RoomGiRoom::Stairway => "staircase.glb",
        RoomGiRoom::Gameplay => "gameplay.glb",
        RoomGiRoom::ShadowTestRoom => "shadow_test_room.glb",
    }
}

fn effective_room_height_scale(room: RoomGiRoom, height_scale: f32) -> f32 {
    match room {
        RoomGiRoom::MainMenu => crate::main_menu_glb::main_menu_env_height_scale(height_scale),
        _ => height_scale,
    }
}

fn wrap_mode_u32(mode: WrappingMode) -> u32 {
    match mode {
        WrappingMode::ClampToEdge => 0,
        WrappingMode::Repeat => 1,
        WrappingMode::MirroredRepeat => 2,
    }
}

fn normal_or_face(normal: [f32; 3], face: Vec3) -> Vec3 {
    let n = Vec3::from_array(normal).normalize_or_zero();
    if n.length_squared() >= 0.5 && vec3_finite(n) {
        n
    } else {
        face
    }
}

fn vec3_finite(v: Vec3) -> bool {
    v.x.is_finite() && v.y.is_finite() && v.z.is_finite()
}

fn vec3_pad(v: Vec3, w: f32) -> [f32; 4] {
    [v.x, v.y, v.z, w]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_bake_slugs_match_bake_stamp() {
        for (room, slug) in RoomGiRoom::ALL
            .into_iter()
            .zip(mahjuro_bake_stamp::room_slugs::ALL.iter().copied())
        {
            assert_eq!(room.slug(), slug);
            assert_eq!(
                room.offline_bake_filename("lightmap.rlm"),
                format!("{slug}.lightmap.rlm")
            );
        }
    }

    #[test]
    fn round_trip_lightmap_rgba32f_texture() {
        let bake = RoomGiLightmapBake {
            room: RoomGiRoom::Shop,
            width: 2,
            height: 1,
            primitive_uv_rects: vec![[0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 0.5, 1.0]],
            pixels_rgba_f32: vec![1.5, 0.25, 0.0, 1.0, 8.0, 4.0, 2.0, 0.0],
        };
        let bytes = bake.encode_rgba32f_texture().expect("encode");
        let back = RoomGiLightmapBake::decode_rgba32f_texture_for_room(&bytes, RoomGiRoom::Shop)
            .expect("decode");
        assert_eq!(back.room, RoomGiRoom::Shop);
        assert_eq!(back.width, 2);
        assert_eq!(back.height, 1);
        assert_eq!(back.primitive_uv_rects, bake.primitive_uv_rects);
        assert_eq!(back.pixels_rgba_f32, bake.pixels_rgba_f32);
    }

    fn tiny_lightmap(primitive_mapped_texels: Vec<usize>) -> GpuBakeLightmap {
        GpuBakeLightmap {
            width: 3,
            height: 1,
            primitive_uv_rects: vec![[0.0, 0.0, 1.0, 1.0]; primitive_mapped_texels.len()],
            primitive_mapped_texels,
            mapped_texels: 1,
            texels: vec![
                GpuBakeLightmapTexel {
                    tri: [0, 0, 0, 0],
                    bary: [1.0, 0.0, 0.0, 0.0],
                },
                GpuBakeLightmapTexel::empty(),
                GpuBakeLightmapTexel::empty(),
            ],
        }
    }

    fn lightmap_with_alpha_cutout_hole() -> GpuBakeLightmap {
        GpuBakeLightmap {
            width: 2,
            height: 1,
            primitive_uv_rects: vec![[0.0, 0.0, 1.0, 1.0]],
            primitive_mapped_texels: vec![2],
            mapped_texels: 2,
            texels: vec![
                GpuBakeLightmapTexel {
                    tri: [0, 0, 0, 0],
                    bary: [1.0, 0.0, 0.0, 0.0],
                },
                GpuBakeLightmapTexel {
                    tri: [0, 0, 0, 0],
                    bary: [0.0, 1.0, 0.0, 0.0],
                },
            ],
        }
    }

    #[test]
    fn lightmap_validation_ignores_alpha_cutout_holes() {
        let lightmap = lightmap_with_alpha_cutout_hole();
        let pixels = vec![1.0, 0.5, 0.25, 1.0, 0.0, 0.0, 0.0, 0.0];

        validate_lightmap_radiance(RoomGiRoom::Shop, &lightmap, &pixels, "raw")
            .expect("alpha-cutout hole should not count as black visible radiance");
    }

    #[test]
    fn shadow_test_room_lightmap_validation_accepts_sparse_direct_light() {
        assert!(shadow_test_room_lightmap_has_radiance(
            0.0053, 0.045718, 1.0
        ));
        assert!(shadow_test_room_lightmap_has_radiance(
            0.0003, 0.002626, 1.0
        ));
        assert!(shadow_test_room_lightmap_has_radiance(
            0.0001, 0.000544, 1.0
        ));
        assert!(!shadow_test_room_lightmap_has_radiance(0.0, 0.0, 0.0));
        assert!(ROOM_GI_LIGHTMAP_MIN_LIT_FRACTION_SHADOW_TEST < ROOM_GI_LIGHTMAP_MIN_LIT_FRACTION);
    }

    #[test]
    fn lightmap_mapping_rejects_uncovered_primitives() {
        let lightmap = tiny_lightmap(vec![1, 0]);
        let err = validate_lightmap_mapping(RoomGiRoom::Shop, &lightmap).unwrap_err();
        assert!(
            err.to_string().contains("no mapped texels"),
            "unexpected error: {err:#}"
        );
    }

    #[test]
    fn archive_lightmap_hints_prioritize_large_visible_surfaces() {
        let title = primitive_lightmap_hint(RoomGiRoom::Archive, Some("text_scene_title"));
        assert!(title.target_min_side >= 192);
        assert!(title.area_scale > 1.0);

        let cubby = primitive_lightmap_hint(RoomGiRoom::Archive, Some("Cubby.001"));
        assert!(cubby.target_min_side >= 128);
        assert!(cubby.area_scale > 1.0);

        let shop = primitive_lightmap_hint(RoomGiRoom::Shop, Some("text_scene_title"));
        assert_eq!(shop.target_min_side, 0);
        assert_eq!(shop.area_scale, 1.0);
    }

    #[test]
    fn hallway_lamp_cords_keep_minimum_lightmap_coverage() {
        if std::env::var_os("MAHJURO_ASSETS").is_none() {
            eprintln!("skip: set MAHJURO_ASSETS to a loose assets directory");
            return;
        }
        let Some(file) = mahjuro_assets::asset_path::get("3d/hallway.glb") else {
            eprintln!("skip: no 3d/hallway.glb");
            return;
        };
        let cpu = crate::hallway_glb::load_hallway_glb_from_bytes(&file.data).expect("decode");
        let params = RoomGiGpuBakeParams {
            room: RoomGiRoom::Hallway,
            bake_width: 1920,
            bake_height: 1080,
            lighting: room_glb::RoomEnvLightingTune::default(),
            height_scale: 1.0,
        };
        let scene = GpuBakeScene::from_room(&params, &cpu, 1.0, 4096).expect("scene");
        let lightmap =
            build_gpu_lightmap_texels(&scene, mahjuro_bake_stamp::room_gi::ROOM_LIGHTMAP_SIZE)
                .expect("lightmap mapping");
        validate_lightmap_mapping(RoomGiRoom::Hallway, &lightmap).expect("valid mapping");
        for primitive_idx in 18..=21 {
            assert!(
                lightmap.primitive_mapped_texels[primitive_idx] > 0,
                "hallway lamp-cord primitive {primitive_idx} must keep mapped texels"
            );
        }
    }

    fn test_room_glb_with_lights(
        lights: Vec<crate::room_env_gltf::RoomGltfEmbeddedPointLight>,
    ) -> room_glb::RoomGlbCpu {
        room_glb::RoomGlbCpu {
            packed_asset_bytes_read: 0,
            decoded_cpu_payload_bytes: 0,
            markers: rustc_hash::FxHashMap::default(),
            environment_primitives: Vec::new(),
            environment_primitives_released: false,
            environment_bounds_doc: None,
            marker_mesh_bounds_doc: rustc_hash::FxHashMap::default(),
            collision_meshes: Vec::new(),
            embedded_perspective_camera: None,
            embedded_cameras_by_name: rustc_hash::FxHashMap::default(),
            embedded_point_lights: lights,
            rain_surface_meshes: Vec::new(),
            rain_surface_merged: None,
            node_bind_poses: rustc_hash::FxHashMap::default(),
            gltf_anim_library: crate::room_gltf_anim::RoomGltfAnimLibrary::default(),
        }
    }

    fn embedded_light(
        name: &str,
        pos_doc: Vec3,
        color_linear: [f32; 3],
        intensity: f32,
        range_doc: Option<f32>,
        is_candle: bool,
    ) -> crate::room_env_gltf::RoomGltfEmbeddedPointLight {
        crate::room_env_gltf::RoomGltfEmbeddedPointLight {
            node_name: name.to_string(),
            pos_doc,
            color_linear,
            is_candle,
            is_lantern: false,
            intensity,
            range_doc,
        }
    }

    fn assert_light_close(a: &GpuBakeLight, b: &GpuBakeLight) {
        for (av, bv) in a
            .pos_range
            .iter()
            .chain(a.color_intensity.iter())
            .chain(a.params.iter())
            .zip(
                b.pos_range
                    .iter()
                    .chain(b.color_intensity.iter())
                    .chain(b.params.iter()),
            )
        {
            assert!(
                (*av - *bv).abs() <= 1.0e-5,
                "light mismatch: {a:?} vs {b:?}"
            );
        }
    }

    #[test]
    fn gpu_bake_punctual_lights_match_runtime_conversion() {
        let tune = room_glb::RoomEnvLightingTune::SOURCE_DEFAULTS;
        let width = 1920;
        let height = 1080;
        let env_height_scale = 1.25;
        let cpu = test_room_glb_with_lights(vec![
            embedded_light(
                "light_candle.001",
                Vec3::new(0.25, -0.5, 1.2),
                [1.0, 0.86, 0.7],
                20.0,
                None,
                true,
            ),
            embedded_light(
                "light_fill",
                Vec3::new(-0.5, 0.25, 2.0),
                [0.2, 0.4, 1.0],
                3.0,
                Some(4.0),
                false,
            ),
        ]);
        let params = RoomGiGpuBakeParams {
            room: RoomGiRoom::Shop,
            bake_width: width,
            bake_height: height,
            lighting: tune,
            height_scale: env_height_scale,
        };
        let profile = crate::room_gltf_punctual::RoomPunctualProfile::Candles {
            flame_time_s: 0.0,
            lamp_flicker: 1.0,
            flicker_amp: 0.0,
        };
        let expected = crate::room_gltf_punctual::embedded_point_lights_runtime_tagged(
            &cpu,
            width as f32,
            height as f32,
            env_height_scale,
            &tune,
            profile,
            "shop.glb",
        )
        .into_iter()
        .map(|tagged| bake_light_from_point(tagged.light, width, height))
        .collect::<Vec<_>>();

        let actual = room_lights_for_gpu_bake(&params, &cpu, env_height_scale);

        assert_eq!(actual.len(), expected.len());
        for (a, e) in actual.iter().zip(expected.iter()) {
            assert_light_close(a, e);
        }
    }

    #[test]
    fn gpu_bake_shader_reuses_scene_pbr_core() {
        let core = include_str!("../../../shaders/scene_pbr_core.wgsl");
        let bake_body = include_str!("../../../shaders/room_gi_bake.wgsl");
        let bake = ROOM_GI_BAKE_SHADER;
        assert!(
            bake.starts_with(core),
            "room GI bake shader must be composed from scene_pbr_core.wgsl"
        );
        for token in [
            "scene_fresnel_schlick",
            "scene_distribution_ggx",
            "scene_geometry_smith",
            "scene_dielectric_kd",
            "scene_pbr_direct_surface",
            "scene_pbr_sample_point_light",
            "scene_pbr_sample_spot_light",
            "scene_pbr_direct_punctual_radiance",
            "scene_pbr_direct_sampled_light",
        ] {
            assert!(
                bake.contains(token),
                "room GI bake shader missing shared core token {token}"
            );
        }
        for token in [
            "fn fresnel_schlick",
            "fn distribution_ggx",
            "fn geometry_smith",
        ] {
            assert!(
                !bake_body.contains(token),
                "room_gi_bake.wgsl should use scene_pbr_core.wgsl instead of local {token}"
            );
        }
    }

    #[test]
    fn room_runtime_and_bake_use_shared_direct_lighting_function() {
        let core = include_str!("../../../shaders/scene_pbr_core.wgsl");
        assert!(
            core.contains("fn scene_pbr_sample_point_light(")
                && core.contains("fn scene_pbr_sample_spot_light(")
                && core.contains("fn scene_pbr_direct_punctual_radiance("),
            "scene_pbr_core.wgsl must own point-light sampling and direct punctual BRDF"
        );
        assert!(
            core.contains("fn scene_pbr_direct_sampled_light("),
            "scene_pbr_core.wgsl must expose one sampled-light direct evaluator"
        );
        assert!(
            core.contains("fn scene_pbr_direct_surface("),
            "scene_pbr_core.wgsl must own direct-light surface construction"
        );

        for (name, shader) in [
            ("room_glb", include_str!("../../../shaders/room_glb.wgsl")),
            (
                "room_gi_bake",
                include_str!("../../../shaders/room_gi_bake.wgsl"),
            ),
        ] {
            for token in [
                "scene_pbr_sample_point_light",
                "scene_pbr_direct_sampled_light",
                "scene_pbr_direct_surface",
            ] {
                assert!(
                    shader.contains(token),
                    "{name} must call shared direct lighting token {token}"
                );
            }
            for forbidden in [
                "fn bake_f0",
                "let f0 = bake_f0",
                "let F = scene_fresnel_schlick",
                "let D = scene_distribution_ggx",
                "let G = scene_geometry_smith",
                "spec_brdf =",
                "scene_pbr_direct_punctual_radiance(",
                "ScenePbrDirectLight(",
            ] {
                assert!(
                    !shader.contains(forbidden),
                    "{name} has reintroduced local direct-light BRDF code: {forbidden}"
                );
            }
        }

        let room_glb = include_str!("../../../shaders/room_glb.wgsl");
        let bake_body = include_str!("../../../shaders/room_gi_bake.wgsl");
        assert_eq!(
            room_glb.matches("scene_pbr_direct_sampled_light(").count(),
            2,
            "room_glb direct punctual Lo must be exactly the shared point + spot calls"
        );
        assert_eq!(
            bake_body.matches("scene_pbr_direct_sampled_light(").count(),
            1,
            "room_gi_bake bounce-source direct punctual radiance must be exactly the shared point-light call"
        );
        assert!(
            room_glb.contains(
                "let direct = scene_pbr_direct_sampled_light(pbr_surface, point_sample, punc_vis);\n        Lo = Lo + direct.total;"
            ),
            "room_glb point-light Lo must feed the shared sampled-light direct evaluator"
        );
        assert!(
            room_glb.contains(
                "let direct = scene_pbr_direct_sampled_light(pbr_surface, spot_sample, punc_vis);\n        Lo = Lo + direct.total;"
            ),
            "room_glb spot-light Lo must feed the shared sampled-light direct evaluator"
        );
        assert!(
            bake_body.contains(
                "lgt.params.x,\n            params.trace_params.x,\n        );"
            ) && bake_body.contains(
                "let direct = scene_pbr_direct_sampled_light(pbr_surface, point_sample, 1.0);\n        out = out + direct.total;"
            ),
            "room_gi_bake bounce-source direct light accumulation must use buffered attenuation kind and the same sampled-light evaluator"
        );
        assert!(
            bake_body.contains("let base = surface_receiver_indirect_base(hit, sample);"),
            "room_gi_bake lightmap receivers must store indirect/base radiance, not duplicate runtime direct light"
        );
        assert!(
            room_glb.contains(
                "let pbr_surface = scene_pbr_direct_surface(albedo, n_world, V, metallic, roughness);"
            ),
            "room_glb must build direct-light surfaces through the shared helper"
        );
        assert!(
            bake_body.contains(
                "let pbr_surface = scene_pbr_direct_surface(\n        sample.albedo,\n        hit.normal,\n        V,\n        sample.metallic,\n        sample.roughness,\n    );"
            ),
            "room_gi_bake must build bounce-source direct-light surfaces through the shared helper"
        );
        assert!(
            bake_body.contains(
                "scene_pbr_direct_surface(sample.albedo, n, v, sample.metallic, sample.roughness)"
            ),
            "room_gi_bake secondary transport must reuse the shared direct-light surface helper"
        );

        let room_glb = include_str!("../../../shaders/room_glb.wgsl");
        assert!(
            room_glb.contains("scene_pbr_sample_spot_light"),
            "room_glb must use the shared spot-light sampler"
        );
        for forbidden in [
            "let atten_spot = scene_smooth_point_atten",
            "let spot_factor = khr_spot_angle_attenuation_scene",
        ] {
            assert!(
                !room_glb.contains(forbidden),
                "room_glb has reintroduced local spot-light sampling: {forbidden}"
            );
        }
    }

    #[test]
    fn room_runtime_and_bake_share_point_light_sampling_contract() {
        let runtime = include_str!("../../../shaders/room_glb.wgsl");
        let bake = include_str!("../../../shaders/room_gi_bake.wgsl");
        let rust = include_str!("room_gi_bake.rs");

        assert!(
            runtime.contains(
                "let point_sample = scene_pbr_sample_point_light(\n            in.world_pos,\n            light_pos,\n            range_w,\n            vec4<f32>(pl.color.rgb * boss_light_rgb_mul, pl.color.a),\n            kind,\n            cam.room_env_params.y,\n        );"
            ),
            "runtime room point lights must use the shared sampler with runtime position/range/color/kind/inv-doc-scale inputs"
        );
        assert!(
            bake.contains(
                "let point_sample = scene_pbr_sample_point_light(\n            hit.point,\n            lgt.pos_range.xyz,\n            lgt.pos_range.w,\n            lgt.color_intensity,\n            lgt.params.x,\n            params.trace_params.x,\n        );"
            ),
            "bake bounce-source point lights must use the shared sampler with equivalent buffered inputs"
        );
        assert!(
            bake.contains(
                "struct BakeLight {\n    pos_range: vec4<f32>,\n    color_intensity: vec4<f32>,\n    params: vec4<f32>,\n}"
            ) && rust.contains("params: [1.0, 0.0, 0.0, 0.0],"),
            "bake lights must carry runtime attenuation kind instead of hardcoding it in WGSL"
        );
        assert!(
            bake.contains(
                "fn surface_receiver_indirect_base(hit: Hit, sample: SurfaceSample) -> vec3<f32>"
            ) && bake.contains("let base = surface_receiver_indirect_base(hit, sample);")
                && bake.contains("fn surface_bounce_source_radiance_sampled("),
            "bake must keep primary lightmap receiver radiance separate from bounce-source direct lighting"
        );
        assert!(
            !bake.contains(concat!("visible", "_to_light"))
                && !bake.contains("trace_scene(point + normal")
                && !bake.contains("let base = surface_bounce_source_radiance_sampled"),
            "bake must not reintroduce a separate full-GLB primary punctual visibility or direct receiver term"
        );
    }

    #[test]
    fn gpu_bake_shader_has_nontrivial_transport_and_outputs() {
        assert!(
            ROOM_GI_GPU_DIR_SAMPLES >= 64,
            "room GI bake needs enough primary samples for production lightmaps"
        );
        assert!(
            ROOM_GI_GPU_ADAPTIVE_MIN_DIR_SAMPLES > 0
                && ROOM_GI_GPU_ADAPTIVE_MIN_DIR_SAMPLES <= ROOM_GI_GPU_DIR_SAMPLES,
            "adaptive GI sampling must have a nonzero minimum within the max sample budget"
        );
        assert!(
            ROOM_GI_GPU_ADAPTIVE_REL_STDERR > 0.0,
            "adaptive GI sampling must keep a positive error target"
        );
        assert!(
            ROOM_GI_GPU_SECONDARY_SAMPLES > 0,
            "secondary GI bounce sampling must remain enabled"
        );
        assert!(
            ROOM_GI_LIGHTMAP_DENOISE_PASSES > 0,
            "room GI lightmaps must run denoising before shipping"
        );

        let bake = ROOM_GI_BAKE_SHADER;
        for token in [
            "atlas_sample_coords",
            "bilerp_rgba",
            "textureLoad(albedo_atlas, s.p00, 0)",
            "textureLoad(albedo_atlas, s.p10, 0)",
            "textureLoad(albedo_atlas, s.p01, 0)",
            "textureLoad(albedo_atlas, s.p11, 0)",
            "cosine_hemi_dir",
            "sample_ggx_reflection_dir",
            "ggx_reflection_pdf",
            "mis_mixture_pdf",
            "scene_pbr_surface_brdf",
            "sample.roughness",
            "world_environment_radiance",
            "world_hemisphere_radiance",
            "surface_receiver_indirect_base",
            "surface_bounce_source_radiance_sampled",
            "fn surface_path_radiance_sample(hit: Hit, view_dir: vec3<f32>, seed: u32, sample_idx: u32)",
            "for (var bi: u32 = 0u; bi < secondary_count; bi = bi + 1u)",
            "let bounce_idx = sample_idx * secondary_count + bi",
            "bounce_sum = bounce_sum +",
            "let bounce = bounce_sum / f32(max(secondary_count, 1u))",
            "fn adaptive_surface_radiance(hit: Hit, view_dir: vec3<f32>, texel_seed: u32)",
            "let max_samples = max(params.counts.w, 1u)",
            "let min_samples = min(max(u32(params.trace_params.w), 1u), max_samples)",
            "let rel_stderr_target = max(params.lighting_params.w, 0.0)",
            "if (rel_stderr <= rel_stderr_target)",
            "lightmap_main",
            "LightmapTexel",
        ] {
            assert!(
                bake.contains(token),
                "room_gi_bake.wgsl missing {token}; this guards filtered textures, secondary bounce, and lightmap output"
            );
        }
    }

    #[test]
    fn room_gi_outputs_are_lightmap_textures_not_legacy_probes() {
        let legacy_probe_ext = concat!(".", "m", "gi");
        for room in RoomGiRoom::ALL {
            let path = room.lightmap_asset_path();
            assert!(
                path.starts_with("data/room_lightmap/")
                    && path.ends_with(".lightmap.rlm")
                    && !path.contains(legacy_probe_ext),
                "room GI asset path must point at an RLM lightmap texture, got {path}"
            );
            assert!(
                room.offline_bake_filename("lightmap.rlm")
                    .ends_with(".lightmap.rlm"),
                "offline room GI output must be a lightmap RLM file"
            );
        }
        let bake_stamp = include_str!("../../../crates/mahjuro-bake-stamp/src/room_gi.rs");
        assert!(
            bake_stamp.contains("assets/data/room_lightmap")
                && bake_stamp.contains("lightmap.rlm")
                && !bake_stamp.contains(legacy_probe_ext),
            "room GI stamp must track committed lightmap outputs, not legacy probe files"
        );
    }

    #[test]
    fn runtime_room_rendering_does_not_require_legacy_bakes() {
        let runtime = include_str!("wgpu_renderer/runtime/render.rs");
        assert!(
            !runtime.contains("require_cached_room_gi_bake(")
                && !runtime.contains("require_room_gi_bake("),
            "room render path must not load legacy GI bakes"
        );
    }

    #[test]
    fn runtime_lit_receivers_use_shared_light_sampling() {
        for (name, shader, min_point_calls, min_spot_calls) in [
            (
                "room_glb",
                include_str!("../../../shaders/room_glb.wgsl"),
                1usize,
                1usize,
            ),
            (
                "tile_3d",
                include_str!("../../../shaders/tile_3d.wgsl"),
                1usize,
                1usize,
            ),
            (
                "lit_mesh",
                include_str!("../../../shaders/lit_mesh.wgsl"),
                2usize,
                2usize,
            ),
        ] {
            assert!(
                shader.matches("scene_pbr_sample_point_light(").count() >= min_point_calls,
                "{name} must use shared point-light sampling"
            );
            assert!(
                shader.matches("scene_pbr_sample_spot_light(").count() >= min_spot_calls,
                "{name} must use shared spot-light sampling"
            );
            for forbidden in [
                "scene_smooth_point_atten(",
                "punctual_attenuation_with_inv_doc_scale(",
                "khr_spot_angle_attenuation_scene(",
                "let atten_sp",
                "let spot_factor",
            ] {
                assert!(
                    !shader.contains(forbidden),
                    "{name} has reintroduced local light sampling: {forbidden}"
                );
            }
        }
    }

    #[test]
    fn runtime_shaders_do_not_use_legacy_gi_lighting() {
        let scene_pbr = include_str!("../../../shaders/scene_pbr_lights.wgsl");
        let legacy_probe_prefix = concat!("scene_", "probe_");
        assert!(
            !scene_pbr.contains(legacy_probe_prefix),
            "runtime legacy GI helper shader should remain a no-op include"
        );

        for (name, shader) in [
            ("room_glb", include_str!("../../../shaders/room_glb.wgsl")),
            ("lit_mesh", include_str!("../../../shaders/lit_mesh.wgsl")),
            ("tile_3d", include_str!("../../../shaders/tile_3d.wgsl")),
        ] {
            assert!(
                !shader.contains(legacy_probe_prefix),
                "{name} should not use runtime legacy GI lighting"
            );
        }
    }
}
