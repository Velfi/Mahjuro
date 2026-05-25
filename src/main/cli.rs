use std::path::PathBuf;

use super::*;

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Cli {
    /// Skip Steamworks SDK initialization. Use for local dev runs when
    /// you don't want the overlay attaching or Steam taking over the
    /// foreground app role. Headless subcommands (bot, screenshot,
    /// sweeps) always behave as if `--no-steam` were set regardless of
    /// this flag.
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub no_steam: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Bot(BotCli),
    Sweep(SweepCli),
    StrategySweep(StrategySweepCli),
    ForcedRelicSweep(ForcedRelicSweepCli),
    /// Render a single frame offscreen (no window) and save a PNG.
    ///
    /// **Prefer a release binary** (`cargo build --release`, then `target/release/mahjuro`):
    /// debug builds are much slower on each run because GPU shader/pipeline setup dominates.
    ///
    /// Use `--scene showcase --pack …` or `--scene tile_pack_celebration` for the
    /// pack-opening overlay alone (black stage + settled reveal row); it is not `--scene shop`.
    /// Use `--scene showcase` without `--pack` for the zodiac ribbon (same as `zodiac_celebration`).
    /// Use `--scene game_over_level_up` for meta profile level-up (`Showcase` / `MetaLevelUpPresenter`).
    Screenshot(ScreenshotCli),
    /// Bake offline emissive probe SH for a static room GLB (shop / hallway / staircase / archive / main menu / gameplay).
    ///
    /// Writes `assets/data/room_gi/<room>.mgi` for packaging into the gameplay asset pack.
    /// Prefer **`cargo build --release`** — same cold-start cost as `screenshot`.
    BakeRoomGi(BakeRoomGiCli),
    /// Bake offline directional shadow + contact AO for a static room GLB.
    BakeRoomShadows(BakeRoomShadowsCli),
    /// Internal: Win32 Vulkan WSI smoke test (parent uses this to fall back to DX12 on fault).
    #[command(hide = true, name = "vulkan-wsi-probe")]
    VulkanWsiProbe,
}

/// Offline probe GI bake at the resting room camera (1920×1080 by default).
#[derive(Debug, Args)]
pub struct BakeRoomGiCli {
    /// Room to bake: `shop`, `hallway`, `staircase`, `archive`, `main_menu`, or `gameplay`.
    pub room: String,
    #[arg(long, default_value = "assets/data/room_gi")]
    pub output_dir: PathBuf,
    #[arg(long, default_value_t = 1920)]
    pub width: u32,
    #[arg(long, default_value_t = 1080)]
    pub height: u32,
    #[arg(long, default_value_t = 24)]
    pub warmup_frames: u32,
}

/// Offline room shadow bake (2048² depth + contact AO) at the resting room camera.
#[derive(Debug, Args)]
pub struct BakeRoomShadowsCli {
    /// Room to bake: `shop`, `hallway`, `archive`, or `main_menu`.
    pub room: String,
    #[arg(long, default_value = "assets/data/room_shadow")]
    pub output_dir: PathBuf,
    #[arg(long, default_value_t = 1920)]
    pub width: u32,
    #[arg(long, default_value_t = 1080)]
    pub height: u32,
    #[arg(long, default_value_t = 24)]
    pub warmup_frames: u32,
}

/// Headless one-shot capture: runs `warmup_frames` settle ticks offscreen, then writes `--output`.
///
/// Each CLI invocation builds a fresh renderer; use **`target/release/mahjuro`** for captures
/// (especially when scripting many scenes) — `target/debug/mahjuro` is far slower to start.
#[derive(Debug, Args)]
pub struct ScreenshotCli {
    /// Root scene id (e.g. `shop`, `gameplay`, `collection`, `showcase`,
    /// `zodiac_celebration`, `tile_pack_celebration`, `main_menu_exterior`, …).
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
    /// Ordeal slug for runs where gameplay ordeals matter (`gameplay`, `pick_chamber`, …).
    /// Parsed but otherwise low impact for menu-only scenes.
    #[arg(long, alias = "boss")]
    pub ordeal: Option<String>,
    #[arg(long)]
    pub fresh_progress: bool,
    /// Drive shop focus to a specific prop or item before capture, so
    /// hover/focus-only chrome (plaques, spotlights, focus rings) renders
    /// in the screenshot. Only honored when `--scene shop`.
    /// Values: `journal`, `bell`, `abacus`, `relic:N`,
    /// `ribbon:N`, `talisman:N`, `pack:N` (N is 0-based index).
    #[arg(long)]
    pub shop_focus: Option<String>,
    /// Override `RoomEnvLightingTune::gltf_emissive_scale` for this capture
    /// (glTF room mesh emissive gain). Compare e.g. `1` vs `12` on `--scene pick_chamber`.
    #[arg(long)]
    pub gltf_emissive_scale: Option<f32>,
    /// Zodiac animal slug for `--scene zodiac_celebration` or `--scene showcase`
    /// without `--pack` (e.g. snake, dragon). Default: snake.
    #[arg(long)]
    pub zodiac: Option<String>,
    /// **Zodiac ribbon only:** displayed yaku tier on the full-screen zodiac
    /// showcase (shop ribbon / in-run zodiac level). Not meta profile level;
    /// for that see `--scene game_over_level_up`.
    ///
    /// Used by `--scene zodiac_celebration` or zodiac-mode `--scene showcase` (no `--pack`).
    /// Default: 2.
    #[arg(long, visible_alias = "zodiac-yaku-level")]
    pub celebration_level: Option<u32>,
    /// Pack variant for `--scene tile_pack_celebration` or `--scene showcase --pack …`.
    /// Accepts compact
    /// names from pack titles (`honors`, `terminals`, `bamboogrove`, …) or
    /// `TilePackKind` debug strings (`Honors`, `ScrollLibrary`). Default: honors.
    #[arg(long)]
    pub pack: Option<String>,
    /// Push the item-inspect overlay (turntable + zoom) before capture. Shop
    /// requires `--shop-focus` on an inspectable target (`relic:N`,
    /// `ribbon:N`, `talisman:N`, `pack:N`). Only valid with `--scene shop`
    /// or `collection` (not full-screen showcase pack scenes).
    #[arg(long, action = ArgAction::SetTrue)]
    pub item_inspect: bool,
    /// `game_over_defeat` only: play one headless bot run and use its terminal
    /// `RunState` (stats, boss, memorial selection from the live journal).
    #[arg(long, action = ArgAction::SetTrue)]
    pub bot_play: bool,
    /// `game_over_defeat` only: hydrate the defeat screen from
    /// `PlayerProgress::run_history` on the loaded profile (see `--profile`).
    #[arg(long, visible_alias = "run-history-index")]
    pub from_run_history: Option<usize>,
    /// Profile slot for `--from-run-history` (default: active profile from settings).
    #[arg(long)]
    pub profile: Option<usize>,
    /// 1-based page for paginated scenes (`guide`, `tutorial`). Guide e.g. 4 =
    /// Scoring Basics; tutorial 1 = tiles, 2 = scoring.
    #[arg(long, visible_alias = "guide-page")]
    pub page: Option<u32>,
    /// `game_over_defeat` only: append N bot runs to the in-memory profile before
    /// `--from-run-history` (does not write disk; use with `--from-run-history`).
    #[arg(long)]
    pub seed_bot_runs: Option<usize>,
}

#[derive(Debug, Args)]
pub struct BotCli {
    #[arg(default_value_t = 100)]
    pub runs: u32,
    #[arg(long)]
    pub base_target: Option<u32>,
    #[arg(long)]
    pub plays: Option<u32>,
    #[arg(long)]
    pub discards: Option<u32>,
    #[arg(long)]
    pub gold: Option<u32>,
    /// Difficulty stake: spring (baseline), summer, autumn, or winter. Omit
    /// for Spring. Stratifies balance snapshots so each tier's target /
    /// shop / boss deltas can be evaluated independently.
    #[arg(long)]
    pub stake: Option<crate::core::stake::Stake>,
    #[arg(long, action = ArgAction::SetTrue)]
    pub bot_log: bool,
    /// Write bot results to this path. Format is taken from `--output-format`, or
    /// from the extension (`.json`, `.html`/`.htm`, else plain text).
    #[arg(long, short = 'o', visible_alias = "export-json")]
    pub output_file: Option<PathBuf>,
    /// Output format when using `--output-file`. If omitted, inferred from the path.
    #[arg(long, value_enum)]
    pub output_format: Option<bot::BotOutputFormat>,
    /// Write one `RunStats` JSON object per line (JSON Lines) for per-run analysis.
    #[arg(long)]
    pub output_runs: Option<PathBuf>,
    /// Wall-clock limit per run attempt, in seconds. 0 disables timeouts.
    #[arg(long, default_value_t = bot::DEFAULT_BOT_RUN_TIMEOUT_SECS)]
    pub bot_run_timeout_secs: u32,
    /// After a timed-out attempt, retry this many extra times (`1` = up to 2 attempts total).
    #[arg(long, default_value_t = 1)]
    pub timeout_retries: u32,
    /// Proactively sell owned relics whose hold value is at or below the threshold.
    #[arg(long, action = ArgAction::SetTrue)]
    pub sell_enabled: bool,
    /// Sell relics at or below this hold-value threshold (default 0).
    #[arg(long)]
    pub sell_hold_threshold: Option<i32>,
    /// Max proactive sells per shop visit (default 2).
    #[arg(long)]
    pub sell_max_per_visit: Option<u32>,
    /// In-blind expectimax depth (`0` = legacy greedy, `1` = one-ply unified, `2` = recommended default, `3` = pruned).
    #[arg(long, default_value_t = 2)]
    pub chamber_planner_depth: u32,
}

#[derive(Debug, Args)]
pub struct SweepCli {
    #[arg(long, default_value_t = 40)]
    pub runs: u32,
    #[arg(long)]
    pub export_json: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct StrategySweepCli {
    pub strategies_file: PathBuf,
    #[arg(long, default_value_t = 1000)]
    pub runs: u32,
    #[arg(long)]
    pub export_json: Option<PathBuf>,
    /// Wall-clock limit per run attempt, in seconds. 0 disables timeouts.
    #[arg(long, default_value_t = bot::DEFAULT_BOT_RUN_TIMEOUT_SECS)]
    pub bot_run_timeout_secs: u32,
    /// After a timed-out attempt, retry this many extra times (`1` = up to 2 attempts total).
    #[arg(long, default_value_t = 1)]
    pub timeout_retries: u32,
}

#[derive(Debug, Args)]
pub struct ForcedRelicSweepCli {
    #[arg(long, default_value_t = 500)]
    pub runs: u32,
    #[arg(long)]
    pub export_json: Option<PathBuf>,
}

impl BotCli {
    pub fn bot_config(&self) -> bot::BotConfig {
        bot::BotConfig {
            base_target: self.base_target,
            starting_plays: self.plays,
            starting_discards: self.discards,
            starting_gold: self.gold,
            stake: self.stake,
            sell_enabled: if self.sell_enabled { Some(true) } else { None },
            sell_hold_threshold: self.sell_hold_threshold,
            sell_max_per_visit: self.sell_max_per_visit,
            chamber_planner_depth: Some(self.chamber_planner_depth),
            ..Default::default()
        }
    }

    pub fn bot_run_options(&self) -> bot::BotRunOptions {
        bot::BotRunOptions {
            log: self.bot_log,
            output: self.bot_output_target(),
            output_runs: self.output_runs.clone(),
            run_timeout: if self.bot_run_timeout_secs == 0 {
                None
            } else {
                Some(std::time::Duration::from_secs(
                    self.bot_run_timeout_secs as u64,
                ))
            },
            timeout_retries: self.timeout_retries,
        }
    }

    pub fn bot_output_target(&self) -> Option<bot::BotOutputTarget> {
        let path = self.output_file.as_ref()?;
        let format = self
            .output_format
            .unwrap_or_else(|| bot::BotOutputFormat::infer_from_path(path));
        Some(bot::BotOutputTarget {
            path: path.clone(),
            format,
        })
    }
}

impl StrategySweepCli {
    pub fn bot_run_options(&self) -> bot::BotRunOptions {
        bot::BotRunOptions {
            log: false,
            output: None,
            output_runs: None,
            run_timeout: if self.bot_run_timeout_secs == 0 {
                None
            } else {
                Some(std::time::Duration::from_secs(
                    self.bot_run_timeout_secs as u64,
                ))
            },
            timeout_retries: self.timeout_retries,
        }
    }
}
