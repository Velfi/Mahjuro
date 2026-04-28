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
    BotGraph(BotGraphCli),
    Sweep(SweepCli),
    StrategySweep(StrategySweepCli),
    ForcedRelicSweep(ForcedRelicSweepCli),
    Screenshot(ScreenshotCli),
}

#[derive(Debug, Args)]
pub struct ScreenshotCli {
    #[arg(long)]
    pub scene: String,
    #[arg(long, default_value = "/tmp/mahjuro-screenshot.png")]
    pub output: PathBuf,
    #[arg(long, default_value_t = 1920)]
    pub width: u32,
    #[arg(long, default_value_t = 1080)]
    pub height: u32,
    #[arg(long, default_value_t = 12)]
    pub warmup_frames: u32,
    #[arg(long)]
    pub boss: Option<String>,
    #[arg(long)]
    pub fresh_progress: bool,
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
