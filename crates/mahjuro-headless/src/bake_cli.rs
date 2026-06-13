use std::path::PathBuf;

use clap::{Parser, ValueEnum};

/// Which offline room bakes `mahjuro-bake` should produce.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum RoomBakeKind {
    /// Per-texel room lightmap atlas → `.lightmap.rlm.zst` plus PNG preview.
    Lightmap,
    /// Directional depth + contact AO → `assets/data/room_shadow/<room>.msh`
    Shadow,
}

/// Offline room lighting bakes at each room's resting camera (1920×1080 by default).
#[derive(Debug, Parser)]
#[command(
    name = "mahjuro-bake",
    about = "Bake offline room GI lightmaps (.rlm) and shadow maps (.msh)"
)]
pub struct BakeRoomCli {
    /// Room slug(s): `shop`, `hallway`, `stairway`, `archive`, `main_menu`, `gameplay`,
    /// `shadow_test_room`
    /// (legacy aliases: `pick_chamber`, `collection`, `main_menu_exterior`, `staircase`).
    /// Omit to bake every lightmapped room plus the runtime shadow rooms.
    #[arg(value_name = "ROOM")]
    pub rooms: Vec<String>,
    /// Bakes to run (default: `lightmap,shadow`).
    #[arg(
        long,
        value_enum,
        value_delimiter = ',',
        default_values_t = [RoomBakeKind::Lightmap, RoomBakeKind::Shadow]
    )]
    pub kinds: Vec<RoomBakeKind>,
    #[arg(long, default_value = "assets/data/room_lightmap")]
    pub lightmap_dir: PathBuf,
    #[arg(long, default_value_t = mahjuro_bake_stamp::room_gi::ROOM_LIGHTMAP_SIZE)]
    pub lightmap_size: u32,
    #[arg(long, default_value = "assets/data/room_shadow")]
    pub shadow_dir: PathBuf,
    #[arg(long, default_value_t = 1920)]
    pub width: u32,
    #[arg(long, default_value_t = 1080)]
    pub height: u32,
    /// Idle ticks before each GPU readback (layout settle).
    #[arg(long, default_value_t = 24)]
    pub warmup_frames: u32,
}
