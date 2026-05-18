//! Offline baked emissive probe SH for static room GLB scenes (shop / hallway / archive / main menu).
//!
//! Probes are filled once by `mahjuro bake-room-gi` and uploaded at runtime so the
//! per-frame `emissive-probe-update` compute pass can be skipped on static room views.
//! Dynamic updates resume when [`crate::render::draw_cmd::UiFrame::room_gi_dynamic`] is set
//! (shop item inspect dolly / orbit).
//!
//! **Rebake** after changing room GLB layout, probe grid, or
//! [`crate::render::room_glb::ROOM_EMISSIVE_PROBE_DIR_SAMPLES`] /
//! [`crate::render::room_glb::ROOM_EMISSIVE_PROBE_MARCH_STEPS`] — see `AGENTS.md`.

use std::sync::{Arc, OnceLock};

use glam::Vec3;

use crate::asset_path;
use crate::render::room_glb;

const MAGIC: &[u8; 4] = b"MGI1";
const VERSION: u32 = 2;
const PROBE_SH_STRIDE: usize = 9 * std::mem::size_of::<[f32; 4]>();

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RoomGiRoom {
    Shop = 0,
    Hallway = 1,
    Archive = 2,
    MainMenu = 3,
}

impl RoomGiRoom {
    pub fn asset_path(self) -> &'static str {
        match self {
            Self::Shop => "data/room_gi/shop.mgi",
            Self::Hallway => "data/room_gi/hallway.mgi",
            Self::Archive => "data/room_gi/archive.mgi",
            Self::MainMenu => "data/room_gi/main_menu.mgi",
        }
    }

    pub fn from_ops(shop: bool, hallway: bool, archive: bool, main_menu: bool) -> Option<Self> {
        if shop {
            Some(Self::Shop)
        } else if hallway {
            Some(Self::Hallway)
        } else if archive {
            Some(Self::Archive)
        } else if main_menu {
            Some(Self::MainMenu)
        } else {
            None
        }
    }
}

/// Probe march settings recorded in the bake file (`bake-room-gi` always uses High-quality values).
pub fn bake_probe_sample_params() -> (u32, u32) {
    (
        room_glb::ROOM_EMISSIVE_PROBE_DIR_SAMPLES,
        room_glb::ROOM_EMISSIVE_PROBE_MARCH_STEPS,
    )
}

/// Parsed offline probe coefficients + metadata from a `.mgi` file.
pub struct RoomGiBake {
    pub room: RoomGiRoom,
    pub grid: [u32; 3],
    pub probe_count: u32,
    pub world_min: Vec3,
    pub world_max: Vec3,
    pub bake_width: u32,
    pub bake_height: u32,
    pub dir_samples: u32,
    pub march_steps: u32,
    /// Bake-time view matrix (metadata; runtime does not require a match).
    pub ref_view_proj: [f32; 16],
    /// Raw GPU layout: `probe_count` × 9 × `vec4<f32>` (xyz + pad per SH band).
    pub probe_sh_bytes: Arc<[u8]>,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct RoomGiBakeHeader {
    magic: [u8; 4],
    version: u32,
    room: u32,
    grid_nx: u32,
    grid_ny: u32,
    grid_nz: u32,
    probe_count: u32,
    world_min: [f32; 4],
    world_max: [f32; 4],
    bake_width: u32,
    bake_height: u32,
    dir_samples: u32,
    march_steps: u32,
    ref_view_proj: [f32; 16],
}

impl RoomGiBake {
    pub fn encode(&self) -> Vec<u8> {
        let header = RoomGiBakeHeader {
            magic: *MAGIC,
            version: VERSION,
            room: self.room as u32,
            grid_nx: self.grid[0],
            grid_ny: self.grid[1],
            grid_nz: self.grid[2],
            probe_count: self.probe_count,
            world_min: [self.world_min.x, self.world_min.y, self.world_min.z, 0.0],
            world_max: [self.world_max.x, self.world_max.y, self.world_max.z, 0.0],
            bake_width: self.bake_width,
            bake_height: self.bake_height,
            dir_samples: self.dir_samples,
            march_steps: self.march_steps,
            ref_view_proj: self.ref_view_proj,
        };
        let mut out =
            Vec::with_capacity(std::mem::size_of::<RoomGiBakeHeader>() + self.probe_sh_bytes.len());
        out.extend_from_slice(bytemuck::bytes_of(&header));
        out.extend_from_slice(&self.probe_sh_bytes);
        out
    }

    pub fn decode_for_room(bytes: &[u8], expected_room: RoomGiRoom) -> anyhow::Result<Self> {
        let header_size = std::mem::size_of::<RoomGiBakeHeader>();
        anyhow::ensure!(
            bytes.len() >= header_size,
            "room GI bake file too small ({} bytes)",
            bytes.len()
        );
        let header: &RoomGiBakeHeader = bytemuck::try_from_bytes(&bytes[..header_size])
            .map_err(|e| anyhow::anyhow!("room GI header align: {e}"))?;
        anyhow::ensure!(&header.magic == MAGIC, "room GI bake: bad magic");
        anyhow::ensure!(
            header.version == VERSION,
            "room GI bake: unsupported version {} (expected {VERSION})",
            header.version
        );
        let room = match header.room {
            0 => RoomGiRoom::Shop,
            1 => RoomGiRoom::Hallway,
            2 => RoomGiRoom::Archive,
            3 => RoomGiRoom::MainMenu,
            n => anyhow::bail!("room GI bake: unknown room id {n}"),
        };
        anyhow::ensure!(
            room == expected_room,
            "room GI bake: file is for {room:?} but expected {expected_room:?}"
        );
        let expected_grid = room_glb::ROOM_EMISSIVE_PROBE_GRID;
        anyhow::ensure!(
            header.grid_nx == expected_grid[0]
                && header.grid_ny == expected_grid[1]
                && header.grid_nz == expected_grid[2],
            "room GI bake grid {:?}×{}×{} does not match runtime {:?}",
            header.grid_nx,
            header.grid_ny,
            header.grid_nz,
            expected_grid
        );
        let probe_count = header.probe_count;
        anyhow::ensure!(
            probe_count == expected_grid[0] * expected_grid[1] * expected_grid[2],
            "room GI probe_count mismatch"
        );
        anyhow::ensure!(
            probe_count <= room_glb::ROOM_EMISSIVE_PROBE_MAX,
            "room GI probe_count {probe_count} > max"
        );
        let (exp_dir, exp_march) = bake_probe_sample_params();
        anyhow::ensure!(
            header.dir_samples == exp_dir && header.march_steps == exp_march,
            "room GI bake probe settings {}×{} do not match runtime {}×{} — rebake with \
             `mahjuro bake-room-gi`",
            header.dir_samples,
            header.march_steps,
            exp_dir,
            exp_march
        );
        let sh_bytes = probe_count as usize * PROBE_SH_STRIDE;
        anyhow::ensure!(
            bytes.len() == header_size + sh_bytes,
            "room GI bake payload size mismatch (expected {}, got {})",
            header_size + sh_bytes,
            bytes.len()
        );
        Ok(Self {
            room,
            grid: [header.grid_nx, header.grid_ny, header.grid_nz],
            probe_count,
            world_min: Vec3::new(header.world_min[0], header.world_min[1], header.world_min[2]),
            world_max: Vec3::new(header.world_max[0], header.world_max[1], header.world_max[2]),
            bake_width: header.bake_width,
            bake_height: header.bake_height,
            dir_samples: header.dir_samples,
            march_steps: header.march_steps,
            ref_view_proj: header.ref_view_proj,
            probe_sh_bytes: Arc::from(bytes[header_size..].to_vec()),
        })
    }

    /// World AABB from the bake matches the live room bounds within a small tolerance.
    pub fn aabb_matches(&self, mn: Vec3, mx: Vec3) -> bool {
        const EPS: f32 = 0.08;
        (self.world_min - mn).abs().max_element() < EPS && (self.world_max - mx).abs().max_element() < EPS
    }
}

fn load_room_gi_bake(room: RoomGiRoom) -> Option<Arc<RoomGiBake>> {
    let file = asset_path::get(room.asset_path())?;
    RoomGiBake::decode_for_room(&file.data, room)
        .map(Arc::new)
        .ok()
        .or_else(|| {
            log::warn!(
                "room GI bake {:?} failed to parse ({})",
                room,
                room.asset_path()
            );
            None
        })
}

static BAKE_CACHE: [OnceLock<Option<Arc<RoomGiBake>>>; 4] = [
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
    OnceLock::new(),
];

fn cache_slot(room: RoomGiRoom) -> &'static OnceLock<Option<Arc<RoomGiBake>>> {
    match room {
        RoomGiRoom::Shop => &BAKE_CACHE[0],
        RoomGiRoom::Hallway => &BAKE_CACHE[1],
        RoomGiRoom::Archive => &BAKE_CACHE[2],
        RoomGiRoom::MainMenu => &BAKE_CACHE[3],
    }
}

/// Load (and cache) the offline bake for `room`, if present in the asset pack.
pub fn cached_room_gi_bake(room: RoomGiRoom) -> Option<Arc<RoomGiBake>> {
    cache_slot(room)
        .get_or_init(|| load_room_gi_bake(room))
        .clone()
}

/// Placeholder SH payload for GPU readback (`mahjuro bake-room-gi`).
pub fn probe_sh_meta(
    room: RoomGiRoom,
    world_min: Vec3,
    world_max: Vec3,
    ref_view_proj: [f32; 16],
    bake_width: u32,
    bake_height: u32,
) -> RoomGiBake {
    let grid = room_glb::ROOM_EMISSIVE_PROBE_GRID;
    let probe_count = grid[0] * grid[1] * grid[2];
    let sh_len = probe_count as usize * PROBE_SH_STRIDE;
    let (dir_samples, march_steps) = bake_probe_sample_params();
    RoomGiBake {
        room,
        grid,
        probe_count,
        world_min,
        world_max,
        bake_width,
        bake_height,
        dir_samples,
        march_steps,
        ref_view_proj,
        probe_sh_bytes: Arc::from(vec![0u8; sh_len]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_header_and_payload_size() {
        let (dir_samples, march_steps) = bake_probe_sample_params();
        let bake = RoomGiBake {
            room: RoomGiRoom::Shop,
            grid: room_glb::ROOM_EMISSIVE_PROBE_GRID,
            probe_count: 168,
            world_min: Vec3::new(-1.0, -2.0, -3.0),
            world_max: Vec3::new(4.0, 5.0, 6.0),
            bake_width: 1920,
            bake_height: 1080,
            dir_samples,
            march_steps,
            ref_view_proj: [0.25; 16],
            probe_sh_bytes: Arc::from(vec![0u8; 168 * PROBE_SH_STRIDE]),
        };
        let bytes = bake.encode();
        let back = RoomGiBake::decode_for_room(&bytes, RoomGiRoom::Shop).expect("decode");
        assert_eq!(back.probe_count, 168);
        assert_eq!(back.dir_samples, dir_samples);
        assert_eq!(back.march_steps, march_steps);
    }

    #[test]
    fn decode_rejects_wrong_room() {
        let (dir_samples, march_steps) = bake_probe_sample_params();
        let bake = RoomGiBake {
            room: RoomGiRoom::Shop,
            grid: room_glb::ROOM_EMISSIVE_PROBE_GRID,
            probe_count: 168,
            world_min: Vec3::ZERO,
            world_max: Vec3::ONE,
            bake_width: 1920,
            bake_height: 1080,
            dir_samples,
            march_steps,
            ref_view_proj: [0.0; 16],
            probe_sh_bytes: Arc::from(vec![0u8; 168 * PROBE_SH_STRIDE]),
        };
        let bytes = bake.encode();
        assert!(RoomGiBake::decode_for_room(&bytes, RoomGiRoom::Hallway).is_err());
    }
}
