use anyhow::Context;

use super::bot;
use super::main_cli::Command;

/// Run non-interactive CLI subcommands. Returns `Ok(true)` when the command
/// was handled and the app should exit before creating the window.
pub fn run_cli_command(command: Option<Command>) -> anyhow::Result<bool> {
    // Headless tuning subcommands. Examples:
    //   mahjuro bot
    //   mahjuro bot 200 --base-target 250 --plays 5
    //   mahjuro bot -o /tmp/bot.json
    //   mahjuro bot -o report.html --output-format html
    //   mahjuro sweep
    //   mahjuro sweep --runs 50 --export-json /tmp/sweep.json
    match command {
        Some(Command::Sweep(sweep)) => {
            let bases: &[u32] = &[400, 450, 500, 550, 600];
            let plays: &[u32] = &[4, 5];
            bot::run_sweep(sweep.runs, bases, plays, sweep.export_json.as_deref());
            Ok(true)
        }
        Some(Command::StrategySweep(args)) => {
            let bytes = std::fs::read(&args.strategies_file).with_context(|| {
                format!(
                    "failed to read strategies file {}",
                    args.strategies_file.display()
                )
            })?;
            let file: bot::StrategyFile = serde_json::from_slice(&bytes).with_context(|| {
                format!(
                    "failed to parse strategies file {}",
                    args.strategies_file.display()
                )
            })?;
            let strategies: Vec<(String, bot::BotConfig)> = file
                .strategies
                .into_iter()
                .map(|s| (s.name.clone(), s.to_bot_config()))
                .collect();
            bot::run_strategy_sweep(
                strategies,
                args.runs,
                args.export_json.as_deref(),
                args.bot_run_options(),
            );
            Ok(true)
        }
        Some(Command::ForcedRelicSweep(args)) => {
            bot::run_forced_relic_sweep(args.runs, args.export_json.as_deref());
            Ok(true)
        }
        Some(Command::Bot(bot_cli)) => {
            bot::run_headless(
                bot_cli.runs,
                bot_cli.bot_config(),
                bot_cli.bot_run_options(),
            );
            Ok(true)
        }
        Some(Command::VulkanWsiProbe) => {
            let shell = crate::sdl_shell::SdlShell::new("Vulkan WSI probe", 256, 256, false)?;
            crate::render::wgpu_renderer::run_vulkan_wsi_probe_with_window(shell.window)?;
            Ok(true)
        }
        None => Ok(false),
    }
}
