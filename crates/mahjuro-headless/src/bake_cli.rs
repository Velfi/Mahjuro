use std::path::PathBuf;

use clap::{Parser, ValueEnum};

/// Which offline room bakes `mahjuro-bake` should produce.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum RoomBakeKind {
    /// Emissive probe SH → `assets/data/room_gi/<room>.mgi`
    Gi,
    /// Directional depth + contact AO → `assets/data/room_shadow/<room>.msh`
    Shadow,
}

/// Offline room lighting bakes at each room's resting camera (1920×1080 by default).
#[derive(Debug, Parser)]
#[command(
    name = "mahjuro-bake",
    about = "Bake offline room GI probes (.mgi) and shadow maps (.msh)"
)]
pub struct BakeRoomCli {
    /// Room slug(s): `shop`, `hallway`, `staircase`, `archive`, `main_menu`, `gameplay`
    /// (aliases: `pick_chamber`, `collection`, `main_menu_exterior`, `stairway`).
    /// Omit to bake every static room.
    #[arg(value_name = "ROOM")]
    pub rooms: Vec<String>,
    /// Bakes to run (default: `gi,shadow`).
    #[arg(
        long,
        value_enum,
        value_delimiter = ',',
        default_values_t = [RoomBakeKind::Gi, RoomBakeKind::Shadow]
    )]
    pub kinds: Vec<RoomBakeKind>,
    #[arg(long, default_value = "assets/data/room_gi")]
    pub gi_dir: PathBuf,
    #[arg(long, default_value = "assets/data/room_shadow")]
    pub shadow_dir: PathBuf,
    #[arg(long, default_value_t = 1920)]
    pub width: u32,
    #[arg(long, default_value_t = 1080)]
    pub height: u32,
    /// Idle ticks before each GPU readback (layout / probe settle).
    #[arg(long, default_value_t = 24)]
    pub warmup_frames: u32,
}
