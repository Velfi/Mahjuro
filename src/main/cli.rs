use std::path::PathBuf;

use super::*;

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Cli {
    /// Skip Steamworks SDK initialization. Use for local dev runs when
    /// you don't want the overlay attaching or Steam taking over the
    /// foreground app role. Offscreen tools (`mahjuro-bake`, `mahjuro-screenshot`
    /// in `crates/mahjuro-headless`) and bot/sweep CLIs always skip Steam regardless of
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
    /// Internal: Win32 Vulkan WSI smoke test (parent uses this to fall back to DX12 on fault).
    #[command(hide = true, name = "vulkan-wsi-probe")]
    VulkanWsiProbe,
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
    pub yen: Option<u32>,
    /// Difficulty season: spring (baseline), summer, autumn, or winter. Omit
    /// for Spring. Stratifies balance snapshots so each tier's target /
    /// shop / boss deltas can be evaluated independently.
    #[arg(long, visible_alias = "stake")]
    pub season: Option<crate::core::season::Season>,
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
    /// In-blind expectimax depth (`0` = legacy greedy, `1` = one-ply unified default, `2` = two-ply, `3` = pruned).
    #[arg(long, default_value_t = 1)]
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
            starting_yen: self.yen,
            season: self.season,
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
