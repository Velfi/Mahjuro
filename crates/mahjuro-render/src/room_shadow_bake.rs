//! Offline baked directional shadow maps for static room GLB scenes.
//!
//! Filled by `mahjuro bake-room` and sampled at runtime so room meshes
//! use a stable contact shadow field while catalog props use the live shadow map.
//!
//! **Rebake** after changing room GLB layout, key-light direction, or shadow
//! frustum tuning — see `docs/agents/room-shadows-and-baking.md`.

use std::sync::{Arc, OnceLock};

use mahjuro_assets::asset_path;
use crate::room_gi_bake::RoomGiRoom;

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
    let baked_depth = bake.depth_bytes.chunks_exact(4).nth(y * w + x).map(|chunk| {
        f32::from_le_bytes(chunk.try_into().unwrap())
    });
    let depth_delta = baked_depth.map(|d| (d - ndc_v.z).abs());
    let applies = depth_delta.is_none_or(|d| d <= CONTACT_AO_DEPTH_COHERENCE_EPS)
        && (ao_val as f32) < 250.0;
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
    // Archive cubby-only bakes store all-white contact AO; runtime does not sample `.msh` yet.
    if bake.room == RoomGiRoom::Archive {
        return true;
    }
    let Some(ao) = bake.ao_bytes.as_ref() else {
        return true;
    };
    let dark = ao.iter().filter(|&&b| b < 250).count();
    dark as f32 / ao.len().max(1) as f32 >= 0.001
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
        return Some(
            require_effective_room_shadow_bake(room).unwrap_or_else(|e| panic!("{e:#}")),
        );
    }
    let file = asset_path::get(room.shadow_asset_path())?;
    RoomShadowBake::decode_for_room(&file.data, room)
        .map(Arc::new)
        .map_err(|e| {
            log::warn!("{}: {e:#}", room.shadow_asset_path());
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
];

pub fn cached_room_shadow_bake(room: RoomGiRoom) -> Option<Arc<RoomShadowBake>> {
    crate::room_bake_cache::cached_room_bake(room, &BAKE_CACHE, load_room_shadow_bake)
}

/// Required offline room shadow bake (`.msh`).
pub fn require_room_shadow_bake(room: RoomGiRoom) -> anyhow::Result<Arc<RoomShadowBake>> {
    let path = room.shadow_asset_path();
    let file = asset_path::get(path).ok_or_else(|| {
        anyhow::anyhow!(
            "missing room shadow bake at {path}; run `cargo build` \
             (needs mahjuro-bake --features bake in target/<profile>/)"
        )
    })?;
    RoomShadowBake::decode_for_room(&file.data, room)
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
        let bounds = with_shop_glb_cpu(|o| o.and_then(|c| c.environment_bounds_doc))
            .expect("shop bounds");
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
                eprintln!("corner {:?} uv {:?} ndc_z {:.3} ao {}", corner, uv, ndc_z, a);
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
    fn archive_effective_with_depth_and_all_white_ao() {
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
        assert!(room_shadow_bake_is_effective(&bake));
        assert!(validate_room_shadow_bake_effective(&bake, RoomGiRoom::Archive).is_ok());
    }
}
