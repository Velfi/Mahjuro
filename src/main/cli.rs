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
    /// Call `ISteamInput::Init` and sync with `RunFrame` each tick. **Not for
    /// normal play:** gamepads are read via SDL; this can break controllers when
    /// Steam is running. Use to test In-Game Actions (`game_actions_*.vdf`) /
    /// overlay binding. Same as env `MAHJURO_STEAM_INPUT=1`.
    #[arg(long, global = true, action = ArgAction::SetTrue)]
    pub steam_input: bool,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Bot(BotCli),
    BotGraph(BotGraphCli),
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
    /// Boss slug for runs where gameplay bosses matter (`gameplay`, `pick_blind`, …).
    /// Parsed but otherwise low impact for menu-only scenes.
    #[arg(long)]
    pub boss: Option<String>,
    #[arg(long)]
    pub fresh_progress: bool,
    /// Drive shop focus to a specific prop or item before capture, so
    /// hover/focus-only chrome (plaques, spotlights, focus rings) renders
    /// in the screenshot. Only honored when `--scene shop`.
    /// Values: `journal`, `bell`, `abacus`, `relic:N`,
    /// `ribbon:N`, `talisman:N`, `pack:N` (N is 0-based index).
    #[arg(long)]
    pub shop_focus: Option<String>,
    /// Override the Yaku Journal cover-open animation amount (0.0 = closed,
    /// 1.0 = fully open). Only honored when `--scene shop`. Useful for
    /// capturing mid-tween states without trying to time the screenshot
    /// precisely against the live animation.
    #[arg(long)]
    pub journal_open: Option<f32>,
    /// Override the click-to-open journal transition progress (0.0 = just
    /// clicked, 1.0 = about to push YakuJournalScene). Drives both the
    /// cover-open phase and the zoom phase via a single `[0, 1]` time
    /// fraction relative to `JournalTransition::TOTAL_DUR`. Only honored
    /// when `--scene shop`.
    #[arg(long)]
    pub journal_transition: Option<f32>,
    /// Override `ShopEnvLightingTune::gltf_emissive_scale` for this capture
    /// (glTF room mesh emissive gain). Compare e.g. `1` vs `12` on `--scene pick_blind`.
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
    /// Push the item-inspect overlay (orbit camera) before capture. Shop
    /// requires `--shop-focus` on an inspectable target (`relic:N`,
    /// `ribbon:N`, `talisman:N`, `pack:N`). Only valid with `--scene shop`
    /// or `collection` (not full-screen showcase pack scenes).
    #[arg(long, action = ArgAction::SetTrue)]
    pub item_inspect: bool,
}

#[derive(Debug, Args)]
pub struct BotCli {
    #[arg(default_value_t = 100)]
    pub runs: u32,
    #[arg(long)]
    pub base_target: Option<u32>,
    #[arg(long)]
    pub target_scale: Option<f32>,
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
    #[arg(long)]
    pub export_json: Option<PathBuf>,
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
}

#[derive(Debug, Args)]
pub struct ForcedRelicSweepCli {
    #[arg(long, default_value_t = 500)]
    pub runs: u32,
    #[arg(long)]
    pub export_json: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct BotGraphCli {
    #[arg(default_value_t = 10_000)]
    pub runs: u32,
    #[arg(long)]
    pub slug: Option<String>,
    #[arg(long)]
    pub label: Option<String>,
    #[arg(long)]
    pub base_target: Option<u32>,
    #[arg(long)]
    pub target_scale: Option<f32>,
    #[arg(long)]
    pub plays: Option<u32>,
    #[arg(long)]
    pub discards: Option<u32>,
    #[arg(long)]
    pub gold: Option<u32>,
    /// Difficulty stake for the balance snapshot.
    #[arg(long)]
    pub stake: Option<crate::core::stake::Stake>,
    #[arg(long, action = ArgAction::SetTrue)]
    pub bot_log: bool,
}

impl BotCli {
    pub fn bot_config(&self) -> bot::BotConfig {
        bot::BotConfig {
            base_target: self.base_target,
            target_scaling: self.target_scale,
            starting_plays: self.plays,
            starting_discards: self.discards,
            starting_gold: self.gold,
            stake: self.stake,
            ..Default::default()
        }
    }
}

impl BotGraphCli {
    pub fn bot_config(&self) -> bot::BotConfig {
        bot::BotConfig {
            base_target: self.base_target,
            target_scaling: self.target_scale,
            starting_plays: self.plays,
            starting_discards: self.discards,
            starting_gold: self.gold,
            stake: self.stake,
            ..Default::default()
        }
    }
}
