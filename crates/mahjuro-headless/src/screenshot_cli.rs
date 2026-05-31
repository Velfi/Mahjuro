use std::path::PathBuf;

use clap::{ArgAction, Parser};

/// Offscreen PNG capture for marketing, docs, and regression baselines.
#[derive(Debug, Parser)]
#[command(name = "mahjuro-screenshot", about = "Render one offscreen frame to a PNG")]
pub struct ScreenshotCli {
    /// Root scene id (e.g. `main_menu`, `shop`, `hallway`, `archive`, `defeat`, `showcase`, …).
    #[arg(long)]
    pub scene: String,
    #[arg(long, default_value = "/tmp/mahjuro-screenshot.png")]
    pub output: PathBuf,
    #[arg(long, default_value_t = 1920)]
    pub width: u32,
    #[arg(long, default_value_t = 1080)]
    pub height: u32,
    /// Extra idle ticks before the capture frame (layout/asset settling).
    #[arg(long, default_value_t = 12)]
    pub warmup_frames: u32,
    /// Ordeal slug for runs where gameplay ordeals matter (`gameplay`, `hallway`, …).
    #[arg(long, alias = "boss")]
    pub ordeal: Option<String>,
    #[arg(long)]
    pub fresh_progress: bool,
    /// Drive shop focus to a specific prop or item before capture (shop scene only).
    #[arg(long)]
    pub shop_focus: Option<String>,
    /// Override `RoomEnvLightingTune::gltf_emissive_scale` for this capture.
    #[arg(long)]
    pub gltf_emissive_scale: Option<f32>,
    #[arg(long)]
    pub zodiac: Option<String>,
    #[arg(long, visible_alias = "zodiac-yaku-level")]
    pub celebration_level: Option<u32>,
    #[arg(long)]
    pub pack: Option<String>,
    #[arg(long, action = ArgAction::SetTrue)]
    pub item_inspect: bool,
    #[arg(long, action = ArgAction::SetTrue)]
    pub bot_play: bool,
    #[arg(long, visible_alias = "run-history-index")]
    pub from_run_history: Option<usize>,
    #[arg(long)]
    pub profile: Option<usize>,
    #[arg(long, visible_alias = "guide-page")]
    pub page: Option<u32>,
    #[arg(long)]
    pub seed_bot_runs: Option<usize>,
}
