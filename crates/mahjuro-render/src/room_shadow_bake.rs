//! Offline baked directional shadow maps for static room GLB scenes.
//!
//! Filled by `mahjuro bake-room` and sampled at runtime so room meshes
//! use a stable contact shadow field while catalog props use the live shadow map.
//!
//! **Rebake** after changing room GLB layout, key-light direction, or shadow
//! frustum tuning — see `docs/agents/room-shadows-and-baking.md`.

use std::sync::{Arc, OnceLock};

use crate::room_gi_bake::RoomGiRoom;
use glam::Vec3;
use mahjuro_assets::asset_path;

const MAGIC: &[u8; 4] = b"MSH1";
pub const VERSION: u32 = mahjuro_bake_stamp::room_shadow::MSH_FORMAT_VERSION;
/// Window height used by `mahjuro-bake` room shadow captures (see `bake_cli.rs`).
pub const ROOM_SHADOW_BAKE_REFERENCE_H: f32 = 1080.0;

/// Scale runtime world positions into bake-space before sampling `.msh` contact AO.
#[inline]
pub fn contact_ao_world_scale_ratio(window_h: f32) -> f32 {
    (ROOM_SHADOW_BAKE_REFERENCE_H / window_h.max(1.0)).clamp(0.25, 4.0)
}

/// Reject contact AO when receiver light-space depth differs from baked depth at the
/// same UV — stops ceiling/floor bake edges from painting horizontal bands on walls.
pub const CONTACT_AO_DEPTH_COHERENCE_EPS: f32 = 0.02;

/// CPU-side contact-AO sample at a runtime world position (matches shader math).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ContactAoWorldProbe {
    pub ndc: [f32; 3],
    pub uv: [f32; 2],
    pub ao: u8,
    pub baked_depth: Option<f32>,
    pub depth_delta: Option<f32>,
    /// Whether contact AO would darken this texel after depth-coherence gating.
    pub applies: bool,
}

/// Sample committed `.msh` contact AO at `world` using bake LVP + window-height scale.
pub fn probe_contact_ao_world(
    bake: &RoomShadowBake,
    world: glam::Vec3,
    window_h: f32,
) -> Option<ContactAoWorldProbe> {
    probe_contact_ao_at_world(bake, world, contact_ao_world_scale_ratio(window_h))
}

/// Sample contact AO with an explicit world-position scale (`1.0` for [`crate::shadow_ao_lab`]).
pub fn probe_contact_ao_at_world(
    bake: &RoomShadowBake,
    world: glam::Vec3,
    world_scale: f32,
) -> Option<ContactAoWorldProbe> {
    let ao = bake.ao_bytes.as_ref()?;
    let scale = world_scale;
    let lvp = glam::Mat4::from_cols_array(&bake.light_view_proj);
    let scaled = world * scale;
    let clip = lvp * scaled.extend(1.0);
    if clip.w.abs() < 1e-8 {
        return None;
    }
    let ndc_v = clip.truncate() / clip.w;
    if ndc_v.z < 0.0 || ndc_v.z > 1.0 {
        return None;
    }
    let uv = [ndc_v.x * 0.5 + 0.5, ndc_v.y * -0.5 + 0.5];
    if uv[0] < 0.0 || uv[0] > 1.0 || uv[1] < 0.0 || uv[1] > 1.0 {
        return None;
    }
    let w = bake.width as usize;
    let h = bake.height as usize;
    let x = ((uv[0] * (w as f32 - 1.0)).round() as usize).min(w - 1);
    let y = ((uv[1] * (h as f32 - 1.0)).round() as usize).min(h - 1);
    let ao_val = *ao.get(y * w + x)?;
    let baked_depth = bake
        .depth_bytes
        .chunks_exact(4)
        .nth(y * w + x)
        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()));
    let depth_delta = baked_depth.map(|d| (d - ndc_v.z).abs());
    let applies =
        depth_delta.is_none_or(|d| d <= CONTACT_AO_DEPTH_COHERENCE_EPS) && (ao_val as f32) < 250.0;
    Some(ContactAoWorldProbe {
        ndc: ndc_v.to_array(),
        uv,
        ao: ao_val,
        baked_depth,
        depth_delta,
        applies,
    })
}

#[derive(Clone)]
pub struct RoomShadowBake {
    pub room: RoomGiRoom,
    pub width: u32,
    pub height: u32,
    pub light_view_proj: [f32; 16],
    pub depth_bias: f32,
    /// Raw `Depth32Float` texels in row-major order (`width * height` × 4 bytes).
    pub depth_bytes: Arc<[u8]>,
    /// Optional contact-AO multiplier (`width * height` bytes, 1.0 = no extra darken).
    pub ao_bytes: Option<Arc<[u8]>>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RoomShadowBakeHeader {
    magic: [u8; 4],
    version: u32,
    room: u32,
    width: u32,
    height: u32,
    light_view_proj: [f32; 16],
    depth_bias: f32,
    depth_byte_len: u32,
    ao_byte_len: u32,
}

impl RoomShadowBake {
    pub fn encode(&self) -> Vec<u8> {
        let ao_len = self.ao_bytes.as_ref().map(|b| b.len() as u32).unwrap_or(0);
        let header = RoomShadowBakeHeader {
            magic: *MAGIC,
            version: VERSION,
            room: self.room as u32,
            width: self.width,
            height: self.height,
            light_view_proj: self.light_view_proj,
            depth_bias: self.depth_bias,
            depth_byte_len: self.depth_bytes.len() as u32,
            ao_byte_len: ao_len,
        };
        let mut out = Vec::with_capacity(
            std::mem::size_of::<RoomShadowBakeHeader>() + self.depth_bytes.len(),
        );
        out.extend_from_slice(bytemuck::bytes_of(&header));
        out.extend_from_slice(&self.depth_bytes);
        if let Some(ao) = &self.ao_bytes {
            out.extend_from_slice(ao);
        }
        out
    }

    pub fn decode_for_room(bytes: &[u8], expected_room: RoomGiRoom) -> anyhow::Result<Self> {
        let header_size = std::mem::size_of::<RoomShadowBakeHeader>();
        anyhow::ensure!(
            bytes.len() >= header_size,
            "room shadow bake: file too small"
        );
        let header: &RoomShadowBakeHeader = bytemuck::try_from_bytes(&bytes[..header_size])
            .map_err(|e| anyhow::anyhow!("room shadow bake header: {e}"))?;
        anyhow::ensure!(header.magic == *MAGIC, "room shadow bake: bad magic");
        anyhow::ensure!(
            header.version == VERSION,
            "room shadow bake: unsupported version {}",
            header.version
        );
        let room = match header.room {
            0 => RoomGiRoom::Shop,
            1 => RoomGiRoom::Hallway,
            2 => RoomGiRoom::Archive,
            3 => RoomGiRoom::MainMenu,
            4 => RoomGiRoom::Stairway,
            5 => RoomGiRoom::Gameplay,
            6 => RoomGiRoom::ShadowTestRoom,
            _ => anyhow::bail!("room shadow bake: unknown room id {}", header.room),
        };
        anyhow::ensure!(
            room == expected_room,
            "room shadow bake: file is {:?}, expected {:?}",
            room,
            expected_room
        );
        let depth_end = header_size + header.depth_byte_len as usize;
        anyhow::ensure!(
            bytes.len() >= depth_end,
            "room shadow bake: depth payload truncated"
        );
        let depth_bytes: Arc<[u8]> = Arc::from(&bytes[header_size..depth_end]);
        let expected_depth = (header.width as usize)
            .checked_mul(header.height as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or_else(|| anyhow::anyhow!("room shadow bake: invalid dimensions"))?;
        anyhow::ensure!(
            depth_bytes.len() == expected_depth,
            "room shadow bake: depth byte len mismatch"
        );
        let ao_bytes = if header.ao_byte_len > 0 {
            let ao_end = depth_end + header.ao_byte_len as usize;
            anyhow::ensure!(
                bytes.len() >= ao_end,
                "room shadow bake: AO payload truncated"
            );
            let ao: Arc<[u8]> = Arc::from(&bytes[depth_end..ao_end]);
            anyhow::ensure!(
                ao.len() == header.width as usize * header.height as usize,
                "room shadow bake: AO len mismatch"
            );
            Some(ao)
        } else {
            None
        };
        Ok(Self {
            room,
            width: header.width,
            height: header.height,
            light_view_proj: header.light_view_proj,
            depth_bias: header.depth_bias,
            depth_bytes,
            ao_bytes,
        })
    }
}

/// Neighbor depth must exceed the center by at least this much (light clip Z) to count as
/// contact occlusion — suppresses shelf-wide darkening on nearly coplanar baked texels.
const CONTACT_AO_DEPTH_EPS: f32 = 0.003;
/// Strength of depth-discontinuity darkening (lower than the first bake pass — large room
/// shells were washing out under the old `×18` / `0.82` cap).
const CONTACT_AO_STRENGTH: f32 = 10.0;
const CONTACT_AO_MAX_DARKEN: f32 = 0.58;

#[derive(Clone, Copy, Debug)]
struct ContactAoTuning {
    radius: i32,
    depth_eps: f32,
    strength: f32,
    max_darken: f32,
    max_neighbor_depth_delta: f32,
    surface_depth_coherence_eps: f32,
    same_prim_normal_min_dot: f32,
    same_prim_scale: f32,
    cross_prim_normal_min_dot: f32,
}

impl ContactAoTuning {
    fn for_room(room: RoomGiRoom) -> Self {
        let default = Self {
            radius: 2,
            depth_eps: CONTACT_AO_DEPTH_EPS,
            strength: CONTACT_AO_STRENGTH,
            max_darken: CONTACT_AO_MAX_DARKEN,
            max_neighbor_depth_delta: 0.075,
            surface_depth_coherence_eps: 0.018,
            same_prim_normal_min_dot: 0.78,
            same_prim_scale: 0.18,
            cross_prim_normal_min_dot: -0.20,
        };
        match room {
            RoomGiRoom::Shop => Self {
                strength: 6.5,
                max_darken: 0.38,
                max_neighbor_depth_delta: 0.050,
                surface_depth_coherence_eps: 0.014,
                same_prim_normal_min_dot: 0.84,
                same_prim_scale: 0.05,
                cross_prim_normal_min_dot: -0.10,
                ..default
            },
            RoomGiRoom::Hallway => Self {
                strength: 6.0,
                max_darken: 0.34,
                max_neighbor_depth_delta: 0.045,
                surface_depth_coherence_eps: 0.024,
                same_prim_normal_min_dot: 0.88,
                same_prim_scale: 0.04,
                cross_prim_normal_min_dot: -0.05,
                ..default
            },
            RoomGiRoom::MainMenu => Self {
                strength: 7.5,
                max_darken: 0.44,
                max_neighbor_depth_delta: 0.060,
                ..default
            },
            RoomGiRoom::Gameplay => Self {
                strength: 8.0,
                max_darken: 0.46,
                max_neighbor_depth_delta: 0.065,
                ..default
            },
            RoomGiRoom::Archive | RoomGiRoom::Stairway | RoomGiRoom::ShadowTestRoom => default,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PrimitiveContactAoClass {
    pub receiver: f32,
    pub occluder: f32,
}

impl PrimitiveContactAoClass {
    const DEFAULT: Self = Self {
        receiver: 1.0,
        occluder: 1.0,
    };
}

#[derive(Clone, Debug)]
struct RoomShadowSurfaceMap {
    width: usize,
    height: usize,
    depth: Vec<f32>,
    normal: Vec<Vec3>,
    normal_valid: Vec<bool>,
    prim_id: Vec<u32>,
    receiver: Vec<f32>,
    occluder: Vec<f32>,
}

impl RoomShadowSurfaceMap {
    fn new(width: u32, height: u32) -> Self {
        let len = width as usize * height as usize;
        Self {
            width: width as usize,
            height: height as usize,
            depth: vec![1.0; len],
            normal: vec![Vec3::Z; len],
            normal_valid: vec![false; len],
            prim_id: vec![0; len],
            receiver: vec![0.0; len],
            occluder: vec![0.0; len],
        }
    }

    fn covered_texels(&self) -> usize {
        self.prim_id.iter().filter(|&&id| id != 0).count()
    }

    fn has_surface(&self, idx: usize) -> bool {
        self.prim_id[idx] != 0 && self.depth[idx].is_finite() && self.depth[idx] < 1.0
    }
}

/// Build contact AO from a GPU-rendered room-shadow depth map and a matching
/// GPU-rendered receiver/occluder mask. This is the production bake path: both
/// inputs come from the same room-shadow capture pass, so AO classification
/// cannot drift from the depth layer that runtime samples.
pub fn bake_contact_ao_for_room_from_mask(
    room: RoomGiRoom,
    width: u32,
    height: u32,
    depth_bytes: &[u8],
    mask_bytes: &[u8],
    normal_bytes: &[u8],
) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        depth_bytes.len() == width as usize * height as usize * 4,
        "room shadow AO {room:?}: depth byte len mismatch"
    );
    anyhow::ensure!(
        mask_bytes.len() == width as usize * height as usize * 4,
        "room shadow AO {room:?}: mask byte len mismatch"
    );
    anyhow::ensure!(
        normal_bytes.len() == width as usize * height as usize * 4,
        "room shadow AO {room:?}: normal byte len mismatch"
    );
    let surface = build_room_shadow_surface_map_from_mask(
        width,
        height,
        depth_bytes,
        mask_bytes,
        normal_bytes,
    );
    anyhow::ensure!(
        surface.covered_texels() > 0,
        "room shadow AO {room:?}: GPU surface mask captured no usable room texels"
    );
    let tuning = ContactAoTuning::for_room(room);
    let ao = bake_contact_ao_with_surface_map(width, height, depth_bytes, &surface, tuning);
    let stats = contact_ao_stats(width, height, depth_bytes, &surface, &ao, tuning);
    log_contact_ao_stats(room, &stats);
    debug_dump_contact_ao_inputs(room, width, height, depth_bytes, &surface);
    Ok(ao)
}

#[derive(Clone, Copy, Debug)]
struct ContactAoStats {
    pixels: usize,
    depth_covered: usize,
    surface_covered: usize,
    surface_on_depth: usize,
    coherent: usize,
    dark: usize,
    gpu_depth_min: Option<f32>,
    gpu_depth_max: Option<f32>,
    surface_depth_min: Option<f32>,
    surface_depth_max: Option<f32>,
    depth_delta_min: Option<f32>,
    depth_delta_p50: Option<f32>,
    depth_delta_p90: Option<f32>,
    signed_delta_p50: Option<f32>,
    signed_delta_p90: Option<f32>,
}

fn contact_ao_stats(
    width: u32,
    height: u32,
    depth_bytes: &[u8],
    surface: &RoomShadowSurfaceMap,
    ao: &[u8],
    tuning: ContactAoTuning,
) -> ContactAoStats {
    let mut depth = vec![0.0f32; width as usize * height as usize];
    let mut depth_covered = 0usize;
    let mut surface_on_depth = 0usize;
    let mut coherent = 0usize;
    let mut deltas = Vec::new();
    let mut signed_deltas = Vec::new();
    let mut gpu_depth_min = f32::INFINITY;
    let mut gpu_depth_max = f32::NEG_INFINITY;
    let mut surface_depth_min = f32::INFINITY;
    let mut surface_depth_max = f32::NEG_INFINITY;
    for (i, chunk) in depth_bytes.chunks_exact(4).enumerate() {
        let d = f32::from_le_bytes(chunk.try_into().unwrap());
        depth[i] = d;
        if d > 0.001 && d < 0.999 {
            depth_covered += 1;
            gpu_depth_min = gpu_depth_min.min(d);
            gpu_depth_max = gpu_depth_max.max(d);
            if surface.has_surface(i) {
                surface_on_depth += 1;
                let signed = surface.depth[i] - d;
                deltas.push(signed.abs());
                signed_deltas.push(signed);
                surface_depth_min = surface_depth_min.min(surface.depth[i]);
                surface_depth_max = surface_depth_max.max(surface.depth[i]);
            }
            if surface_depth_coherent(surface, &depth, i, tuning) {
                coherent += 1;
            }
        }
    }
    deltas.sort_by(f32::total_cmp);
    signed_deltas.sort_by(f32::total_cmp);
    let depth_delta_min = deltas.first().copied();
    let depth_delta_p50 = percentile_sorted(&deltas, 0.50);
    let depth_delta_p90 = percentile_sorted(&deltas, 0.90);
    let signed_delta_p50 = percentile_sorted(&signed_deltas, 0.50);
    let signed_delta_p90 = percentile_sorted(&signed_deltas, 0.90);
    let dark = ao.iter().filter(|&&b| b < 250).count();
    let pixels = (width as usize * height as usize).max(1);
    ContactAoStats {
        pixels,
        depth_covered,
        surface_covered: surface.covered_texels(),
        surface_on_depth,
        coherent,
        dark,
        gpu_depth_min: gpu_depth_min.is_finite().then_some(gpu_depth_min),
        gpu_depth_max: gpu_depth_max.is_finite().then_some(gpu_depth_max),
        surface_depth_min: surface_depth_min.is_finite().then_some(surface_depth_min),
        surface_depth_max: surface_depth_max.is_finite().then_some(surface_depth_max),
        depth_delta_min,
        depth_delta_p50,
        depth_delta_p90,
        signed_delta_p50,
        signed_delta_p90,
    }
}

fn log_contact_ao_stats(room: RoomGiRoom, stats: &ContactAoStats) {
    log::info!(
        "room shadow AO {room:?}: depth_covered={:.3}% surface_covered={:.3}% coherent={:.3}% dark_ao={:.3}%",
        stats.depth_covered as f32 * 100.0 / stats.pixels as f32,
        stats.surface_covered as f32 * 100.0 / stats.pixels as f32,
        stats.coherent as f32 * 100.0 / stats.pixels as f32,
        stats.dark as f32 * 100.0 / stats.pixels as f32,
    );
    if stats.depth_covered > 0 {
        log::info!(
            "room shadow AO {room:?}: surface_on_depth={:.3}% depth_delta_min={:?} p50={:?} p90={:?} signed_p50={:?} signed_p90={:?} gpu_depth={:?}..{:?} surface_depth={:?}..{:?}",
            stats.surface_on_depth as f32 * 100.0 / stats.depth_covered as f32,
            stats.depth_delta_min,
            stats.depth_delta_p50,
            stats.depth_delta_p90,
            stats.signed_delta_p50,
            stats.signed_delta_p90,
            stats.gpu_depth_min,
            stats.gpu_depth_max,
            stats.surface_depth_min,
            stats.surface_depth_max,
        );
    }
}

fn percentile_sorted(values: &[f32], percentile: f32) -> Option<f32> {
    if values.is_empty() {
        return None;
    }
    let idx = ((values.len() - 1) as f32 * percentile.clamp(0.0, 1.0)).round() as usize;
    values.get(idx).copied()
}

fn debug_dump_contact_ao_inputs(
    room: RoomGiRoom,
    width: u32,
    height: u32,
    depth_bytes: &[u8],
    surface: &RoomShadowSurfaceMap,
) {
    let dir = if let Some(dir) = std::env::var_os("MAHJURO_ROOM_SHADOW_DEBUG_DUMP") {
        std::path::PathBuf::from(dir)
    } else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("room shadow AO {room:?}: debug dump mkdir failed: {e:#}");
        return;
    }
    let slug = room.shadow_asset_path().replace(['/', '.'], "_");
    let mut depth = vec![1.0f32; width as usize * height as usize];
    for (i, chunk) in depth_bytes.chunks_exact(4).enumerate() {
        depth[i] = f32::from_le_bytes(chunk.try_into().unwrap());
    }
    let gpu = make_pgm(width, height, |i| depth_to_u8(depth[i]));
    let surf = make_pgm(width, height, |i| {
        if surface.has_surface(i) {
            depth_to_u8(surface.depth[i])
        } else {
            0
        }
    });
    let cover = make_ppm(width, height, |i| {
        let g = depth[i] > 0.001 && depth[i] < 0.999;
        let s = surface.has_surface(i);
        match (g, s) {
            (true, true) => [255, 220, 48],
            (true, false) => [220, 48, 48],
            (false, true) => [48, 200, 80],
            (false, false) => [0, 0, 0],
        }
    });
    let delta = make_ppm(width, height, |i| {
        if !(depth[i] > 0.001 && depth[i] < 0.999 && surface.has_surface(i)) {
            return [0, 0, 0];
        }
        let d = surface.depth[i] - depth[i];
        if d.abs() <= ContactAoTuning::for_room(room).surface_depth_coherence_eps {
            [32, 220, 64]
        } else if d > 0.0 {
            let v = ((d / 0.35).clamp(0.0, 1.0) * 255.0) as u8;
            [v, 0, 0]
        } else {
            let v = (((-d) / 0.35).clamp(0.0, 1.0) * 255.0) as u8;
            [0, 64, v]
        }
    });
    for (name, bytes) in [
        ("gpu_depth.pgm", gpu),
        ("surface_depth.pgm", surf),
        ("coverage.ppm", cover),
        ("delta.ppm", delta),
    ] {
        let path = dir.join(format!("{slug}_{name}"));
        if let Err(e) = std::fs::write(&path, bytes) {
            log::warn!(
                "room shadow AO {room:?}: debug dump {} failed: {e:#}",
                path.display()
            );
        }
    }
}

fn make_pgm(width: u32, height: u32, mut pixel: impl FnMut(usize) -> u8) -> Vec<u8> {
    let mut out = format!("P5\n{} {}\n255\n", width, height).into_bytes();
    out.reserve(width as usize * height as usize);
    for i in 0..width as usize * height as usize {
        out.push(pixel(i));
    }
    out
}

fn make_ppm(width: u32, height: u32, mut pixel: impl FnMut(usize) -> [u8; 3]) -> Vec<u8> {
    let mut out = format!("P6\n{} {}\n255\n", width, height).into_bytes();
    out.reserve(width as usize * height as usize * 3);
    for i in 0..width as usize * height as usize {
        out.extend_from_slice(&pixel(i));
    }
    out
}

fn depth_to_u8(d: f32) -> u8 {
    if d > 0.001 && d < 0.999 && d.is_finite() {
        ((1.0 - d.clamp(0.0, 1.0)) * 255.0).round() as u8
    } else {
        0
    }
}

fn build_room_shadow_surface_map_from_mask(
    width: u32,
    height: u32,
    depth_bytes: &[u8],
    mask_bytes: &[u8],
    normal_bytes: &[u8],
) -> RoomShadowSurfaceMap {
    let mut map = RoomShadowSurfaceMap::new(width, height);
    for i in 0..width as usize * height as usize {
        let d = f32::from_le_bytes(depth_bytes[i * 4..i * 4 + 4].try_into().unwrap());
        let r = mask_bytes[i * 4] as f32 / 255.0;
        let g = mask_bytes[i * 4 + 1] as f32 / 255.0;
        let prim_lo = mask_bytes[i * 4 + 2] as u32;
        let prim_hi = mask_bytes[i * 4 + 3] as u32;
        let prim_id = prim_lo | (prim_hi << 8);
        if prim_id == 0 || !(0.001..0.999).contains(&d) || !d.is_finite() {
            continue;
        }
        map.depth[i] = d;
        map.prim_id[i] = prim_id;
        map.receiver[i] = r;
        map.occluder[i] = g;
        let n = Vec3::new(
            normal_bytes[i * 4] as f32 / 255.0 * 2.0 - 1.0,
            normal_bytes[i * 4 + 1] as f32 / 255.0 * 2.0 - 1.0,
            normal_bytes[i * 4 + 2] as f32 / 255.0 * 2.0 - 1.0,
        );
        let len_sq = n.length_squared();
        if len_sq > 0.25 && len_sq.is_finite() {
            map.normal[i] = n / len_sq.sqrt();
            map.normal_valid[i] = true;
        }
    }
    map
}

pub fn primitive_contact_ao_class(
    room: RoomGiRoom,
    node_name: Option<&str>,
    material_name: Option<&str>,
) -> PrimitiveContactAoClass {
    let mut class = PrimitiveContactAoClass::DEFAULT;
    let node = node_name.unwrap_or("").to_ascii_lowercase();
    let material = material_name.unwrap_or("").to_ascii_lowercase();

    if room == RoomGiRoom::Hallway {
        if contains_any(&node, &["walls"]) || contains_any(&material, &["wall"]) {
            class.receiver = class.receiver.min(0.0);
            class.occluder = class.occluder.min(0.20);
        }

        if contains_any(&node, &["ceiling"]) || contains_any(&material, &["ceiling"]) {
            class.receiver = class.receiver.min(0.0);
            class.occluder = class.occluder.min(0.10);
        }

        if contains_any(&node, &["btn_", "painting", "lamp", "cord", "trim"])
            || contains_any(&material, &["old gold", "lamp", "paint", "white wood"])
        {
            class.receiver = class.receiver.min(0.08);
            class.occluder = class.occluder.min(0.45);
        }

        if contains_any(&node, &["floor"]) || contains_any(&material, &["sauna room planks"]) {
            class.receiver = class.receiver.max(1.0);
            class.occluder = class.occluder.max(0.85);
        }

        if contains_any(&node, &["table"]) || contains_any(&material, &["dark wood"]) {
            class.receiver = class.receiver.min(0.65);
            class.occluder = class.occluder.min(0.80);
        }

        return class;
    }

    if room != RoomGiRoom::Shop {
        return class;
    }

    if contains_any(&node, &["manekineko", "maneki neko"]) {
        class.receiver = class.receiver.min(0.0);
        class.occluder = class.occluder.min(0.35);
        return class;
    }

    if contains_any(&node, &["mysterious_sheet", "cloth", "sheet"])
        || contains_any(&material, &["sheet fabric"])
    {
        class.receiver = class.receiver.min(0.10);
        class.occluder = class.occluder.min(0.35);
    }

    if contains_any(&node, &["cubby", "recess", "hole"]) || contains_any(&material, &["red velvet"])
    {
        class.receiver = class.receiver.min(0.22);
        class.occluder = class.occluder.min(0.70);
    }

    if contains_any(&node, &["pillow"]) || contains_any(&material, &["ratten wicker", "wicker"]) {
        class.receiver = class.receiver.min(0.50);
        class.occluder = class.occluder.min(0.75);
    }

    class
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

#[inline]
#[cfg(test)]
fn normalized_or(v: Vec3, fallback: Vec3) -> Vec3 {
    let len_sq = v.length_squared();
    if len_sq > 1e-12 && len_sq.is_finite() {
        v / len_sq.sqrt()
    } else {
        fallback
    }
}

fn bake_contact_ao_with_surface_map(
    width: u32,
    height: u32,
    depth_bytes: &[u8],
    surface: &RoomShadowSurfaceMap,
    tuning: ContactAoTuning,
) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    debug_assert_eq!(surface.width, w);
    debug_assert_eq!(surface.height, h);
    let mut depth = vec![0.0f32; w * h];
    for (i, chunk) in depth_bytes.chunks_exact(4).enumerate() {
        depth[i] = f32::from_le_bytes(chunk.try_into().unwrap());
    }
    let mut ao = vec![255u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let d = depth[i];
            let receiver = surface.receiver[i];
            if d >= 1.0 || receiver <= 0.0 || !surface_depth_coherent(surface, &depth, i, tuning) {
                ao[i] = 255;
                continue;
            }

            let mut occ = 0.0f32;
            let mut n = 0.0f32;
            for dy in -tuning.radius..=tuning.radius {
                for dx in -tuning.radius..=tuning.radius {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let nx = ((x as i32 + dx).clamp(0, w as i32 - 1)) as usize;
                    let ny = ((y as i32 + dy).clamp(0, h as i32 - 1)) as usize;
                    let ni = ny * w + nx;
                    if surface.occluder[ni] <= 0.0
                        || !surface_depth_coherent(surface, &depth, ni, tuning)
                    {
                        continue;
                    }
                    let nd = depth[ni];
                    let delta = (nd - d).max(0.0);
                    if delta < tuning.depth_eps || delta > tuning.max_neighbor_depth_delta {
                        continue;
                    }
                    let normal_dot = surface.normal[i].dot(surface.normal[ni]);
                    let same_prim =
                        surface.prim_id[i] != 0 && surface.prim_id[i] == surface.prim_id[ni];
                    let normals_valid = surface.normal_valid[i] && surface.normal_valid[ni];
                    let mut weight = surface.occluder[ni];
                    if same_prim {
                        if normals_valid && normal_dot < tuning.same_prim_normal_min_dot {
                            continue;
                        }
                        if !normals_valid {
                            weight *= tuning.same_prim_scale;
                        }
                    } else if normals_valid && normal_dot < tuning.cross_prim_normal_min_dot {
                        continue;
                    }
                    if weight <= 0.0 {
                        continue;
                    }
                    occ += delta * tuning.strength * weight;
                    n += 1.0;
                }
            }
            let darken = if n > 0.0 {
                (occ / n).clamp(0.0, tuning.max_darken) * receiver
            } else {
                0.0
            };
            ao[i] = ((1.0 - darken) * 255.0).round() as u8;
        }
    }
    ao
}

fn surface_depth_coherent(
    surface: &RoomShadowSurfaceMap,
    gpu_depth: &[f32],
    idx: usize,
    tuning: ContactAoTuning,
) -> bool {
    surface.has_surface(idx)
        && gpu_depth[idx].is_finite()
        && (surface.depth[idx] - gpu_depth[idx]).abs() <= tuning.surface_depth_coherence_eps
}

/// Build a simple contact-AO field from a baked depth map (light-space depth texels).
pub fn bake_contact_ao_from_depth(width: u32, height: u32, depth_bytes: &[u8]) -> Vec<u8> {
    let w = width as usize;
    let h = height as usize;
    let mut depth = vec![0.0f32; w * h];
    for (i, chunk) in depth_bytes.chunks_exact(4).enumerate() {
        depth[i] = f32::from_le_bytes(chunk.try_into().unwrap());
    }
    let mut ao = vec![255u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let d = depth[i];
            if d >= 1.0 {
                ao[i] = 255;
                continue;
            }
            let mut occ = 0.0f32;
            let mut n = 0.0f32;
            for dy in -2..=2 {
                for dx in -2..=2 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let nx = ((x as i32 + dx).clamp(0, w as i32 - 1)) as usize;
                    let ny = ((y as i32 + dy).clamp(0, h as i32 - 1)) as usize;
                    let nd = depth[ny * w + nx];
                    let delta = (nd - d).max(0.0);
                    if delta < CONTACT_AO_DEPTH_EPS {
                        continue;
                    }
                    occ += delta * CONTACT_AO_STRENGTH;
                    n += 1.0;
                }
            }
            let darken = if n > 0.0 {
                (occ / n).clamp(0.0, CONTACT_AO_MAX_DARKEN)
            } else {
                0.0
            };
            ao[i] = ((1.0 - darken) * 255.0).round() as u8;
        }
    }
    ao
}

/// True when the committed `.msh` has real depth coverage and non-trivial contact AO.
pub fn room_shadow_bake_is_effective(bake: &RoomShadowBake) -> bool {
    let pixel_count = (bake.width as u64)
        .saturating_mul(bake.height as u64)
        .max(1) as f32;
    let mut depth_covered = 0u32;
    for chunk in bake.depth_bytes.chunks_exact(4) {
        let d = f32::from_le_bytes(chunk.try_into().unwrap());
        if d > 0.001 && d < 0.999 {
            depth_covered += 1;
        }
    }
    if depth_covered as f32 / pixel_count < 0.005 {
        return false;
    }
    let Some(ao) = bake.ao_bytes.as_ref() else {
        return true;
    };
    let dark = ao.iter().filter(|&&b| b < 250).count();
    contact_ao_dark_effective(dark, ao.len(), depth_covered as usize)
}

fn contact_ao_dark_effective(dark: usize, ao_len: usize, depth_covered: usize) -> bool {
    let dark_frac_pixels = dark as f32 / ao_len.max(1) as f32;
    let dark_frac_covered = dark as f32 / depth_covered.max(1) as f32;
    dark_frac_pixels >= 0.001 || dark_frac_covered >= 0.02
}

/// Game runtime must load valid committed `.msh` files. Offline bakers skip this so stale
/// placeholders do not block capturing fresh bakes (live punctual shadows during bake).
pub fn committed_room_shadows_required() -> bool {
    crate::offline_bakes::committed_offline_bakes_required()
}

/// Fail when a decoded bake is a placeholder (all-zero depth / all-white AO).
pub fn validate_room_shadow_bake_effective(
    bake: &RoomShadowBake,
    room: RoomGiRoom,
) -> anyhow::Result<()> {
    if room_shadow_bake_is_effective(bake) {
        return Ok(());
    }
    anyhow::bail!(
        "room shadow bake {room:?} at {} has no usable depth/AO — \
         run `scripts/rebake-offline.sh room`",
        room.shadow_asset_path(),
    )
}

/// Load and validate a room `.msh`; missing or placeholder bakes are hard errors.
pub fn require_effective_room_shadow_bake(room: RoomGiRoom) -> anyhow::Result<Arc<RoomShadowBake>> {
    let bake = require_room_shadow_bake(room)?;
    validate_room_shadow_bake_effective(&bake, room)?;
    Ok(bake)
}

/// Rooms that must have a committed `.msh` at game runtime (archive uses punctual-only today).
pub fn runtime_required_room_shadow_bakes() -> [RoomGiRoom; 5] {
    [
        RoomGiRoom::Shop,
        RoomGiRoom::Hallway,
        RoomGiRoom::MainMenu,
        RoomGiRoom::Stairway,
        RoomGiRoom::Gameplay,
    ]
}

fn load_room_shadow_bake(room: RoomGiRoom) -> Option<Arc<RoomShadowBake>> {
    if committed_room_shadows_required() {
        return Some(require_effective_room_shadow_bake(room).unwrap_or_else(|e| panic!("{e:#}")));
    }
    let path = room.shadow_asset_path();
    decode_room_shadow_bytes(room, path)
}

fn decode_room_shadow_bytes(room: RoomGiRoom, path: &str) -> Option<Arc<RoomShadowBake>> {
    let file = asset_path::load_asset_bytes(path)?;
    RoomShadowBake::decode_for_room(&file, room)
        .map(Arc::new)
        .map_err(|e| {
            log::warn!("{path}: {e:#}");
            e
        })
        .ok()
}

static BAKE_CACHE: [OnceLock<Option<Arc<RoomShadowBake>>>;
    crate::room_gi_bake::ROOM_GI_ROOM_COUNT] = [
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
];

pub fn cached_room_shadow_bake(room: RoomGiRoom) -> Option<Arc<RoomShadowBake>> {
    crate::room_bake_cache::cached_room_bake(room, &BAKE_CACHE, load_room_shadow_bake)
}

/// Required offline room shadow bake (`.msh`).
pub fn require_room_shadow_bake(room: RoomGiRoom) -> anyhow::Result<Arc<RoomShadowBake>> {
    let path = room.shadow_asset_path();
    let file = asset_path::load_asset_bytes(path).ok_or_else(|| {
        anyhow::anyhow!(
            "missing room shadow bake at {path}; run `cargo build` \
             (needs mahjuro-bake --features bake in target/<profile>/)"
        )
    })?;
    RoomShadowBake::decode_for_room(&file, room)
        .map(Arc::new)
        .map_err(|e| anyhow::anyhow!("{path}: {e:#}"))
}

impl RoomGiRoom {
    pub fn shadow_asset_path(self) -> &'static str {
        match self {
            Self::Shop => "data/room_shadow/shop.msh",
            Self::Hallway => "data/room_shadow/hallway.msh",
            Self::Archive => "data/room_shadow/archive.msh",
            Self::MainMenu => "data/room_shadow/main_menu.msh",
            Self::Stairway => "data/room_shadow/stairway.msh",
            Self::Gameplay => "data/room_shadow/gameplay.msh",
            Self::ShadowTestRoom => "data/room_shadow/shadow_test_room.msh",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BAKE_SIZE: u32 = 2048;

    #[test]
    fn round_trip_header_and_payload_size() {
        let depth_len = (BAKE_SIZE * BAKE_SIZE * 4) as usize;
        let ao_len = (BAKE_SIZE * BAKE_SIZE) as usize;
        let bake = RoomShadowBake {
            room: RoomGiRoom::Archive,
            width: BAKE_SIZE,
            height: BAKE_SIZE,
            light_view_proj: glam::Mat4::IDENTITY.to_cols_array(),
            depth_bias: 0.005,
            depth_bytes: Arc::from(vec![0u8; depth_len]),
            ao_bytes: Some(Arc::from(vec![255u8; ao_len])),
        };
        let bytes = bake.encode();
        let back = RoomShadowBake::decode_for_room(&bytes, RoomGiRoom::Archive).expect("decode");
        assert_eq!(back.width, BAKE_SIZE);
        assert_eq!(back.depth_bytes.len(), depth_len);
        assert_eq!(back.ao_bytes.as_ref().map(|a| a.len()), Some(ao_len));
    }

    #[test]
    fn ineffective_when_depth_and_ao_are_empty_placeholders() {
        let depth_len = (512 * 512 * 4) as usize;
        let bake = RoomShadowBake {
            room: RoomGiRoom::Shop,
            width: 512,
            height: 512,
            light_view_proj: glam::Mat4::IDENTITY.to_cols_array(),
            depth_bias: 0.005,
            depth_bytes: Arc::from(vec![0u8; depth_len]),
            ao_bytes: Some(Arc::from(vec![255u8; 512 * 512])),
        };
        assert!(!room_shadow_bake_is_effective(&bake));
        assert!(validate_room_shadow_bake_effective(&bake, RoomGiRoom::Shop).is_err());
    }

    fn project_world_to_ao_uv(lvp: glam::Mat4, world: glam::Vec3) -> Option<(glam::Vec2, f32)> {
        let clip = lvp * world.extend(1.0);
        if clip.w.abs() < 1e-8 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        if ndc.z < 0.0 || ndc.z > 1.0 {
            return None;
        }
        let uv = glam::Vec2::new(ndc.x * 0.5 + 0.5, ndc.y * -0.5 + 0.5);
        if uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 {
            return None;
        }
        Some((uv, ndc.z))
    }

    fn sample_bake_ao(bake: &RoomShadowBake, uv: glam::Vec2) -> Option<u8> {
        let ao = bake.ao_bytes.as_ref()?;
        let w = bake.width as usize;
        let h = bake.height as usize;
        let x = ((uv.x * (w as f32 - 1.0)).round() as usize).min(w - 1);
        let y = ((uv.y * (h as f32 - 1.0)).round() as usize).min(h - 1);
        ao.get(y * w + x).copied()
    }

    /// Committed `.msh` contact AO is often white at room geometry UVs (see runtime logs).
    #[test]
    fn committed_shop_msh_corner_uv_ao_diagnostic() {
        use crate::room_env_gltf::room_world_bounds_corners_centered;
        use crate::room_gi_bake::RoomGiRoom;
        use crate::room_glb::with_shop_glb_cpu;

        let bake = require_room_shadow_bake(RoomGiRoom::Shop).expect("shop.msh");
        let lvp = glam::Mat4::from_cols_array(&bake.light_view_proj);
        let bounds =
            with_shop_glb_cpu(|o| o.and_then(|c| c.environment_bounds_doc)).expect("shop bounds");
        const BAKE_H: f32 = 1080.0;
        const ENV_H: f32 = 1.0;
        let corners = room_world_bounds_corners_centered(bounds, BAKE_H, ENV_H);
        assert!(!corners.is_empty(), "shop corners");
        let mut uv_min = glam::Vec2::splat(f32::INFINITY);
        let mut uv_max = glam::Vec2::splat(f32::NEG_INFINITY);
        let mut ao_samples = Vec::new();
        for corner in &corners {
            let Some((uv, ndc_z)) = project_world_to_ao_uv(lvp, *corner) else {
                eprintln!("corner {:?} outside frustum", corner);
                continue;
            };
            uv_min = uv_min.min(uv);
            uv_max = uv_max.max(uv);
            if let Some(a) = sample_bake_ao(&bake, uv) {
                eprintln!(
                    "corner {:?} uv {:?} ndc_z {:.3} ao {}",
                    corner, uv, ndc_z, a
                );
                ao_samples.push(a);
            }
        }
        let uv_span = uv_max - uv_min;
        eprintln!(
            "shop corner uv span=({:.4}, {:.4}) samples={:?} min={} max={}",
            uv_span.x,
            uv_span.y,
            ao_samples,
            ao_samples.iter().copied().min().unwrap_or(255),
            ao_samples.iter().copied().max().unwrap_or(255),
        );
        // Documents current bake: corners map inside frustum but AO texels are white.
        assert!(uv_span.x > 0.05 && uv_span.y > 0.05);
    }

    #[test]
    fn cached_shop_shadow_matches_required_asset_load() {
        use crate::room_gi_bake::RoomGiRoom;

        let cached = cached_room_shadow_bake(RoomGiRoom::Shop).expect("cached shop shadow");
        let required = require_room_shadow_bake(RoomGiRoom::Shop).expect("required shop shadow");
        assert_eq!(cached.width, required.width);
        assert_eq!(cached.height, required.height);
        assert_eq!(cached.light_view_proj, required.light_view_proj);
        assert_eq!(cached.depth_bytes.as_ref(), required.depth_bytes.as_ref());
        assert_eq!(cached.ao_bytes.as_deref(), required.ao_bytes.as_deref());
        assert!(room_shadow_bake_is_effective(&cached));
    }

    #[test]
    fn committed_room_shadow_bakes_are_effective() {
        for room in [
            RoomGiRoom::Shop,
            RoomGiRoom::Hallway,
            RoomGiRoom::Archive,
            RoomGiRoom::MainMenu,
            RoomGiRoom::Stairway,
            RoomGiRoom::Gameplay,
        ] {
            let bake = require_room_shadow_bake(room).expect("room shadow bake should load");
            validate_room_shadow_bake_effective(&bake, room)
                .expect("room shadow bake should have usable depth and AO");
        }
    }

    #[test]
    fn effective_when_depth_has_coverage_and_ao_varies() {
        let w = 8u32;
        let h = 8u32;
        let mut depth = vec![0u8; (w * h * 4) as usize];
        depth[0..4].copy_from_slice(&0.4f32.to_le_bytes());
        let mut ao = vec![255u8; (w * h) as usize];
        ao[0] = 180;
        let bake = RoomShadowBake {
            room: RoomGiRoom::Shop,
            width: w,
            height: h,
            light_view_proj: glam::Mat4::IDENTITY.to_cols_array(),
            depth_bias: 0.005,
            depth_bytes: Arc::from(depth),
            ao_bytes: Some(Arc::from(ao)),
        };
        assert!(room_shadow_bake_is_effective(&bake));
    }

    #[test]
    fn archive_all_white_ao_is_not_effective() {
        let w = 512u32;
        let h = 512u32;
        let mut depth = vec![0u8; (w * h * 4) as usize];
        for i in 0..2000 {
            depth[i * 4..(i + 1) * 4].copy_from_slice(&0.4f32.to_le_bytes());
        }
        let bake = RoomShadowBake {
            room: RoomGiRoom::Archive,
            width: w,
            height: h,
            light_view_proj: glam::Mat4::IDENTITY.to_cols_array(),
            depth_bias: 0.005,
            depth_bytes: Arc::from(depth),
            ao_bytes: Some(Arc::from(vec![255u8; (w * h) as usize])),
        };
        assert!(!room_shadow_bake_is_effective(&bake));
        assert!(validate_room_shadow_bake_effective(&bake, RoomGiRoom::Archive).is_err());
    }

    #[test]
    fn effective_when_sparse_depth_has_relative_ao_coverage() {
        let w = 512u32;
        let h = 512u32;
        let mut depth = vec![0u8; (w * h * 4) as usize];
        for i in 0..2000 {
            depth[i * 4..(i + 1) * 4].copy_from_slice(&0.4f32.to_le_bytes());
        }
        let mut ao = vec![255u8; (w * h) as usize];
        for b in ao.iter_mut().take(50) {
            *b = 220;
        }
        let bake = RoomShadowBake {
            room: RoomGiRoom::Hallway,
            width: w,
            height: h,
            light_view_proj: glam::Mat4::IDENTITY.to_cols_array(),
            depth_bias: 0.005,
            depth_bytes: Arc::from(depth),
            ao_bytes: Some(Arc::from(ao)),
        };
        assert!(room_shadow_bake_is_effective(&bake));
    }

    #[test]
    fn hallway_walls_do_not_receive_baked_contact_ao() {
        let walls = primitive_contact_ao_class(RoomGiRoom::Hallway, Some("walls"), Some("wall"));
        assert_eq!(walls.receiver, 0.0);
        assert!(walls.occluder > 0.0);

        let floor = primitive_contact_ao_class(
            RoomGiRoom::Hallway,
            Some("floor"),
            Some("Sauna Room planks"),
        );
        assert_eq!(floor.receiver, 1.0);
        assert_eq!(floor.occluder, 1.0);

        let sign = primitive_contact_ao_class(
            RoomGiRoom::Hallway,
            Some("btn_play_round"),
            Some("Material.002"),
        );
        assert!(sign.receiver < 0.10);
        assert!(sign.occluder < 0.50);
    }

    #[test]
    fn shop_maneki_neko_does_not_receive_baked_contact_ao() {
        let cat =
            primitive_contact_ao_class(RoomGiRoom::Shop, Some("ManekinekoB"), Some("Material.001"));
        assert_eq!(cat.receiver, 0.0);
        assert!(cat.occluder <= 0.35);
    }

    fn test_ao_tuning() -> ContactAoTuning {
        ContactAoTuning {
            radius: 1,
            depth_eps: 0.001,
            strength: 10.0,
            max_darken: 0.80,
            max_neighbor_depth_delta: 0.10,
            surface_depth_coherence_eps: 0.005,
            same_prim_normal_min_dot: 0.90,
            same_prim_scale: 0.0,
            cross_prim_normal_min_dot: -0.20,
        }
    }

    fn depth_bytes(values: &[f32]) -> Vec<u8> {
        let mut out = Vec::with_capacity(values.len() * 4);
        for value in values {
            out.extend_from_slice(&value.to_le_bytes());
        }
        out
    }

    fn class_mask_bytes(classes: &[(f32, f32, u16)]) -> Vec<u8> {
        let mut out = Vec::with_capacity(classes.len() * 4);
        for &(receiver, occluder, prim_id) in classes {
            out.push((receiver.clamp(0.0, 1.0) * 255.0).round() as u8);
            out.push((occluder.clamp(0.0, 1.0) * 255.0).round() as u8);
            out.push((prim_id & 0x00ff) as u8);
            out.push((prim_id >> 8) as u8);
        }
        out
    }

    fn normal_mask_bytes(normals: &[Vec3]) -> Vec<u8> {
        let mut out = Vec::with_capacity(normals.len() * 4);
        for &normal in normals {
            let n = normalized_or(normal, Vec3::Z);
            for channel in [n.x, n.y, n.z] {
                out.push(((channel * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8);
            }
            out.push(255);
        }
        out
    }

    fn small_surface_map(depth: &[f32]) -> RoomShadowSurfaceMap {
        let mut surface = RoomShadowSurfaceMap::new(3, 1);
        for (i, d) in depth.iter().copied().enumerate() {
            surface.depth[i] = d;
            surface.normal[i] = Vec3::Z;
            surface.normal_valid[i] = true;
            surface.prim_id[i] = i as u32 + 1;
            surface.receiver[i] = 1.0;
            surface.occluder[i] = 1.0;
        }
        surface
    }

    #[test]
    fn gpu_mask_surface_map_decodes_normals_for_ao_coherence() {
        let depth = [0.40, 0.40, 0.44];
        let bytes = depth_bytes(&depth);
        let mask = class_mask_bytes(&[(1.0, 1.0, 1), (1.0, 1.0, 1), (1.0, 1.0, 1)]);
        let normals = normal_mask_bytes(&[Vec3::Z, Vec3::Z, Vec3::X]);
        let surface = build_room_shadow_surface_map_from_mask(3, 1, &bytes, &mask, &normals);

        assert!(surface.normal_valid[1]);
        assert!(surface.normal_valid[2]);
        assert!(surface.normal[1].dot(surface.normal[2]) < 0.05);

        let ao = bake_contact_ao_with_surface_map(3, 1, &bytes, &surface, test_ao_tuning());
        assert_eq!(ao[1], 255);
    }

    #[test]
    fn gpu_mask_ao_requires_matching_normal_buffer() {
        let depth = depth_bytes(&[0.40, 0.40, 0.44]);
        let mask = class_mask_bytes(&[(1.0, 1.0, 1), (1.0, 1.0, 2), (1.0, 1.0, 3)]);

        let err = bake_contact_ao_for_room_from_mask(RoomGiRoom::Shop, 3, 1, &depth, &mask, &[])
            .expect_err("missing normal buffer must fail");
        assert!(err.to_string().contains("normal byte len mismatch"));
    }

    #[test]
    fn classified_ao_darkens_only_valid_receiver_texels() {
        let depth = [0.40, 0.40, 0.44];
        let bytes = depth_bytes(&depth);
        let mut surface = small_surface_map(&depth);
        surface.receiver[1] = 0.0;

        let ao = bake_contact_ao_with_surface_map(3, 1, &bytes, &surface, test_ao_tuning());
        assert_eq!(ao[1], 255);

        surface.receiver[1] = 1.0;
        let ao = bake_contact_ao_with_surface_map(3, 1, &bytes, &surface, test_ao_tuning());
        assert!(ao[1] < 255, "valid receiver should darken from neighbor");
    }

    #[test]
    fn classified_ao_requires_neighbor_occluders() {
        let depth = [0.40, 0.40, 0.44];
        let bytes = depth_bytes(&depth);
        let mut surface = small_surface_map(&depth);
        surface.occluder[2] = 0.0;

        let ao = bake_contact_ao_with_surface_map(3, 1, &bytes, &surface, test_ao_tuning());
        assert_eq!(ao[1], 255);
    }

    #[test]
    fn classified_ao_suppresses_same_primitive_hard_edges() {
        let depth = [0.40, 0.40, 0.44];
        let bytes = depth_bytes(&depth);
        let mut surface = small_surface_map(&depth);
        surface.prim_id[2] = surface.prim_id[1];
        surface.normal[2] = Vec3::X;

        let ao = bake_contact_ao_with_surface_map(3, 1, &bytes, &surface, test_ao_tuning());
        assert_eq!(ao[1], 255);
    }

    #[test]
    fn classified_ao_rejects_surface_depth_mismatch() {
        let depth = [0.40, 0.40, 0.44];
        let bytes = depth_bytes(&depth);
        let mut surface = small_surface_map(&depth);
        surface.depth[2] = 0.30;

        let ao = bake_contact_ao_with_surface_map(3, 1, &bytes, &surface, test_ao_tuning());
        assert_eq!(ao[1], 255);
    }
}
