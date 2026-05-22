//! Offline baked directional shadow maps for static room GLB scenes.
//!
//! Filled by `mahjuro bake-room-shadows` and sampled at runtime so room meshes
//! use a stable contact shadow field while catalog props use the live shadow map.
//!
//! **Rebake** after changing room GLB layout, key-light direction, or shadow
//! frustum tuning — see `AGENTS.md`.

use std::sync::{Arc, OnceLock};

use crate::asset_path;
use crate::render::room_gi_bake::RoomGiRoom;

const MAGIC: &[u8; 4] = b"MSH1";
const VERSION: u32 = 2;

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
        let mut out =
            Vec::with_capacity(std::mem::size_of::<RoomShadowBakeHeader>() + self.depth_bytes.len());
        out.extend_from_slice(bytemuck::bytes_of(&header));
        out.extend_from_slice(&self.depth_bytes);
        if let Some(ao) = &self.ao_bytes {
            out.extend_from_slice(ao);
        }
        out
    }

    pub fn decode_for_room(bytes: &[u8], expected_room: RoomGiRoom) -> anyhow::Result<Self> {
        let header_size = std::mem::size_of::<RoomShadowBakeHeader>();
        anyhow::ensure!(bytes.len() >= header_size, "room shadow bake: file too small");
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
            anyhow::ensure!(bytes.len() >= ao_end, "room shadow bake: AO payload truncated");
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

fn load_room_shadow_bake(room: RoomGiRoom) -> Option<Arc<RoomShadowBake>> {
    let file = asset_path::get(room.shadow_asset_path())?;
    RoomShadowBake::decode_for_room(&file.data, room)
        .map(Arc::new)
        .map_err(|e| {
            log::warn!("{}: {e:#}", room.shadow_asset_path());
            e
        })
        .ok()
}

static BAKE_CACHE: [OnceLock<Option<Arc<RoomShadowBake>>>; 4] = [
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
];

fn cache_slot(room: RoomGiRoom) -> &'static OnceLock<Option<Arc<RoomShadowBake>>> {
    match room {
        RoomGiRoom::Shop => &BAKE_CACHE[0],
        RoomGiRoom::Hallway => &BAKE_CACHE[1],
        RoomGiRoom::Archive => &BAKE_CACHE[2],
        RoomGiRoom::MainMenu => &BAKE_CACHE[3],
    }
}

pub fn cached_room_shadow_bake(room: RoomGiRoom) -> Option<Arc<RoomShadowBake>> {
    let slot = cache_slot(room);
    slot.get_or_init(|| load_room_shadow_bake(room)).clone()
}

impl RoomGiRoom {
    pub fn shadow_asset_path(self) -> &'static str {
        match self {
            Self::Shop => "data/room_shadow/shop.msh",
            Self::Hallway => "data/room_shadow/hallway.msh",
            Self::Archive => "data/room_shadow/archive.msh",
            Self::MainMenu => "data/room_shadow/main_menu.msh",
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
}
