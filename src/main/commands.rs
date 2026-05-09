use std::path::Path;

use anyhow::Context;

use super::bot;
use super::main_bot_graph::{
    build_bot_graph_snapshot, default_snapshot_label, default_snapshot_slug, render_bot_graphs,
    upsert_snapshot,
};
use super::main_cli::Command;
use super::main_headless;

/// Run non-interactive CLI subcommands. Returns `Ok(true)` when the command
/// was handled and the app should exit before creating the window.
pub fn run_cli_command(command: Option<Command>) -> anyhow::Result<bool> {
    // Headless tuning subcommands. Examples:
    //   mahjuro bot
    //   mahjuro bot 200 --base-target 250 --target-scale 1.3 --plays 5
    //   mahjuro bot -o /tmp/bot.json
    //   mahjuro bot -o report.html --output-format html
    //   mahjuro bot-graph 10000 --slug baseline_10k --label "Baseline\n(10k runs)"
    //   mahjuro sweep
    //   mahjuro sweep --runs 50 --export-json /tmp/sweep.json
    match command {
        Some(Command::Sweep(sweep)) => {
            let bases: &[u32] = &[200, 250, 300, 350];
            let scales: &[f32] = &[1.20, 1.30, 1.40, 1.50];
            let plays: &[u32] = &[4, 5];
            bot::run_sweep(
                sweep.runs,
                bases,
                scales,
                plays,
                sweep.export_json.as_deref(),
            );
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
        Some(Command::BotGraph(bot_graph)) => {
            let config = bot_graph.bot_config();
            let mode = config.clone().into_mode();
            let slug = bot_graph
                .slug
                .clone()
                .unwrap_or_else(|| default_snapshot_slug(&mode, bot_graph.runs));
            let label = bot_graph
                .label
                .clone()
                .unwrap_or_else(|| default_snapshot_label(&mode, bot_graph.runs))
                .replace("\\n", "\n");
            let batch = bot::run_headless_aggregate(
                bot_graph.runs,
                config,
                bot_graph.bot_run_options(),
            );
            batch.aggregate.print_summary();

            let snapshot = build_bot_graph_snapshot(&batch.aggregate, slug.clone(), label);
            let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
            let snapshot_path = repo_root.join("docs").join("bot_balance_runs.json");
            upsert_snapshot(&snapshot_path, snapshot)?;
            println!(
                "updated bot graph snapshot '{}' in {}",
                slug,
                snapshot_path.display()
            );
            render_bot_graphs(repo_root)?;
            println!("regenerated docs bot balance graphs");
            Ok(true)
        }
        Some(Command::Screenshot(s)) => {
            main_headless::run_screenshot_command(s)?;
            Ok(true)
        }
        None => Ok(false),
    }
}
