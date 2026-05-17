use super::*;

use clap::ValueEnum;
use indicatif::{ParallelProgressIterator, ProgressBar, ProgressIterator, ProgressStyle};

use crate::bot::export_schema::{BotExportMeta, EXPORT_SCHEMA_VERSION};
use crate::bot::stats::RunTimeoutSnapshot;
use std::time::Duration;

/// File format for `mahjuro bot --output-file`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "lower")]
pub enum BotOutputFormat {
    /// Plain text (same layout as the printed summary).
    #[default]
    Txt,
    Json,
    Html,
}

impl BotOutputFormat {
    pub fn infer_from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref()
        {
            Some("json") => Self::Json,
            Some("htm" | "html") => Self::Html,
            _ => Self::Txt,
        }
    }

    fn export_label(self) -> &'static str {
        match self {
            Self::Txt => "text",
            Self::Json => "JSON",
            Self::Html => "HTML",
        }
    }
}

#[derive(Clone, Debug)]
pub struct BotOutputTarget {
    pub path: PathBuf,
    pub format: BotOutputFormat,
}

#[derive(Clone, Debug, Default)]
pub struct BotConfig {
    pub base_target: Option<u32>,
    pub target_scaling: Option<f32>,
    pub starting_plays: Option<u32>,
    pub starting_discards: Option<u32>,
    pub starting_gold: Option<u32>,
    pub relic_weight: Option<f32>,
    pub zodiac_weight: Option<f32>,
    pub talisman_weight: Option<f32>,
    pub pack_weight: Option<f32>,
    pub skip_threshold_multiplier: Option<f32>,
    pub forced_relic: Option<RelicId>,
    /// Difficulty stake used when building `GameMode`. `None` means Spring;
    /// `Some(Stake)` applies that tier's target / shop / boss deltas. Exposed
    /// on the CLI via `--stake` so balance snapshots can be stratified by
    /// difficulty.
    pub stake: Option<crate::core::stake::Stake>,
}

#[derive(Clone, Copy, Debug)]
pub struct BotStrategy {
    pub relic_weight: f32,
    pub zodiac_weight: f32,
    pub talisman_weight: f32,
    pub pack_weight: f32,
    pub skip_threshold_multiplier: f32,
}

impl BotStrategy {
    pub fn from_config(cfg: &BotConfig) -> Self {
        let d = Self::default();
        Self {
            relic_weight: cfg.relic_weight.unwrap_or(d.relic_weight),
            zodiac_weight: cfg.zodiac_weight.unwrap_or(d.zodiac_weight),
            talisman_weight: cfg.talisman_weight.unwrap_or(d.talisman_weight),
            pack_weight: cfg.pack_weight.unwrap_or(d.pack_weight),
            skip_threshold_multiplier: cfg
                .skip_threshold_multiplier
                .unwrap_or(d.skip_threshold_multiplier),
        }
    }
}

impl Default for BotStrategy {
    fn default() -> Self {
        Self {
            relic_weight: 2.0,
            zodiac_weight: 0.5,
            talisman_weight: 0.5,
            pack_weight: 0.5,
            skip_threshold_multiplier: 2.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct BotRunOptions {
    pub log: bool,
    pub output: Option<BotOutputTarget>,
    /// One JSON object per line (`RunStats`) for tooling / quant analysis.
    pub output_runs: Option<PathBuf>,
    /// Wall-clock cap for a single bot run attempt. `None` disables timeouts.
    pub run_timeout: Option<Duration>,
    /// Extra attempts after a timed-out run (`total attempts = 1 + timeout_retries`).
    pub timeout_retries: u32,
}

impl Default for BotRunOptions {
    fn default() -> Self {
        Self {
            log: false,
            output: None,
            output_runs: None,
            run_timeout: None,
            timeout_retries: 1,
        }
    }
}

/// One timed-out attempt (before a retry or as the final failed attempt).
#[derive(Debug, Clone, serde::Serialize)]
pub struct BotTimeoutDiag {
    pub run_index: u32,
    /// 0 = first attempt, 1 = first retry, …
    pub attempt: u32,
    #[serde(flatten)]
    pub snapshot: RunTimeoutSnapshot,
}

#[derive(Debug)]
pub struct HeadlessBotBatch {
    pub aggregate: AggregateStats,
    pub runs: Vec<RunStats>,
    pub timeout_events: Vec<BotTimeoutDiag>,
}

#[derive(Serialize)]
struct BotExportPayload<'a> {
    schema_version: u32,
    meta: BotExportMeta,
    runs: u32,
    mode: &'a GameMode,
    aggregate: crate::bot::export_schema::BotAggregateV2,
    derived: crate::bot::export_schema::BotReportDerived,
}

fn bot_export_value(
    runs: u32,
    mode: &GameMode,
    agg: &AggregateStats,
) -> anyhow::Result<serde_json::Value> {
    let ykc = crate::core::yaku::YakuKind::all().len();
    let payload = BotExportPayload {
        schema_version: EXPORT_SCHEMA_VERSION,
        meta: BotExportMeta {
            yaku_kind_count: ykc,
            crate_version: env!("CARGO_PKG_VERSION"),
        },
        runs,
        mode,
        aggregate: agg.to_aggregate_v2(),
        derived: agg.to_derived(ykc),
    };
    Ok(serde_json::to_value(&payload)?)
}

fn write_runs_jsonl(path: &Path, runs: &[RunStats]) -> anyhow::Result<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut f = std::fs::File::create(path)?;
    for r in runs {
        serde_json::to_writer(&mut f, r)?;
        writeln!(f)?;
    }
    Ok(())
}

#[derive(Serialize)]
struct SweepCellExport {
    base_target: u32,
    target_scaling: f32,
    starting_plays: u32,
    aggregate: AggregateStats,
}

#[derive(Serialize)]
struct SweepExport {
    runs_per_cell: u32,
    cells: Vec<SweepCellExport>,
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(value)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, json)?;
    Ok(())
}

/// Escape `<` so embedded JSON cannot prematurely close a `<script>` tag.
fn escape_json_for_html_script(json: &str) -> String {
    json.replace('<', "\\u003c")
}

fn write_bot_html(path: &Path, json_embedded: &str) -> anyhow::Result<()> {
    const TEMPLATE: &str = include_str!("bot_report_template.html");
    let html = TEMPLATE.replace("__BOT_EXPORT_JSON__", json_embedded);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, html)?;
    Ok(())
}

fn write_bot_export(
    target: &BotOutputTarget,
    runs: u32,
    mode: &GameMode,
    agg: &AggregateStats,
) -> anyhow::Result<()> {
    if let Some(parent) = target.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    match target.format {
        BotOutputFormat::Json => {
            let v = bot_export_value(runs, mode, agg)?;
            let json = serde_json::to_string_pretty(&v)?;
            std::fs::write(&target.path, json)?;
            Ok(())
        }
        BotOutputFormat::Txt => {
            let mut buf = Vec::new();
            agg.write_summary(&mut buf)?;
            std::fs::write(&target.path, buf)?;
            Ok(())
        }
        BotOutputFormat::Html => {
            let v = bot_export_value(runs, mode, agg)?;
            let json_compact = serde_json::to_string(&v)?;
            let embedded = escape_json_for_html_script(&json_compact);
            write_bot_html(&target.path, &embedded)
        }
    }
}

impl BotConfig {
    pub fn into_mode(self) -> GameMode {
        // Start from the stake-aware preset so base_target / price_multiplier /
        // starting_rules reflect the chosen difficulty; the CLI overrides
        // below then apply *on top* of that (handy for A/B'ing individual
        // knobs at a given stake).
        let stake = self.stake.unwrap_or_default();
        let mut mode =
            GameMode::with_material_and_stake(crate::persistence::TileMaterial::default(), stake);
        if let Some(v) = self.base_target {
            mode.base_target = v;
        }
        if let Some(v) = self.target_scaling {
            mode.target_scaling = v;
        }
        if let Some(v) = self.starting_plays {
            mode.starting_plays = v;
        }
        if let Some(v) = self.starting_discards {
            mode.starting_discards = v;
        }
        if let Some(v) = self.starting_gold {
            mode.starting_gold = v;
        }
        mode
    }
}

pub(crate) fn run_with_sequential(
    n: u32,
    config: BotConfig,
    options: BotRunOptions,
) -> AggregateStats {
    let mut agg = AggregateStats::default();
    for i in 0..n {
        let (s, _) = run_scheduled_bot_slot(i + 1, config.clone(), options.clone());
        agg.record(&s);
    }
    agg
}

fn run_scheduled_bot_slot(
    run_index: u32,
    config: BotConfig,
    options: BotRunOptions,
) -> (RunStats, Vec<BotTimeoutDiag>) {
    let max_attempts = options.timeout_retries.saturating_add(1);
    let mut diags = Vec::new();
    let mut last = RunStats::default();
    for attempt in 0..max_attempts {
        let (_run, s) =
            super::play_run_with_options(config.clone(), options.clone(), Some(run_index));
        if !s.run_timed_out {
            return (s, diags);
        }
        if let Some(ref snap) = s.timeout_detail {
            diags.push(BotTimeoutDiag {
                run_index,
                attempt,
                snapshot: snap.clone(),
            });
        }
        last = s;
    }
    (last, diags)
}

fn bot_runs_progress_bar(n: u32) -> ProgressBar {
    let pb = ProgressBar::new(u64::from(n));
    pb.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] \
             {human_pos:>8}/{human_len:>8} ({percent:>3}%) | ETA {eta_precise}",
        )
        .expect("bot progress template")
        .progress_chars("=>-"),
    );
    pb.set_message("bot runs");
    pb
}

fn timeout_progress_line(snapshot: &RunTimeoutSnapshot) -> String {
    match snapshot.phase.as_str() {
        "playing_blind" => format!(
            "blind score/target {}/{}",
            snapshot.round_score, snapshot.target_score
        ),
        "shop" => format!(
            "after last clear: scored {} vs that blind's target {} (not the next blind's goal)",
            snapshot.round_score, snapshot.target_score
        ),
        "outer" => format!(
            "between loop steps: round_score={} target_score={}",
            snapshot.round_score, snapshot.target_score
        ),
        _ => format!(
            "round_score/target_score {}/{}",
            snapshot.round_score, snapshot.target_score
        ),
    }
}

fn print_bot_timeout_report(events: &[BotTimeoutDiag]) {
    if events.is_empty() {
        return;
    }
    use std::collections::BTreeMap;
    let mut by_phase: BTreeMap<&str, u32> = BTreeMap::new();
    for ev in events {
        *by_phase.entry(ev.snapshot.phase.as_str()).or_insert(0) += 1;
    }
    let breakdown = by_phase
        .iter()
        .map(|(phase, n)| format!("{phase}={n}"))
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "\n=== Bot wall-clock timeouts: {} timed-out attempt(s) (retries may have followed) ===\n  by phase: {breakdown}",
        events.len(),
    );
    for ev in events {
        let s = &ev.snapshot;
        let progress = timeout_progress_line(s);
        println!(
            "  run #{}, attempt {}: phase={} | ante {} | {} | blind_turn={:?} | {} | plays {} discards {} | {}ms",
            ev.run_index,
            ev.attempt.saturating_add(1),
            s.phase,
            s.ante,
            s.blind,
            s.blind_turn,
            progress,
            s.plays_remaining,
            s.discards_remaining,
            s.elapsed_ms,
        );
    }
}

pub fn run_headless_aggregate(
    n: u32,
    config: BotConfig,
    options: BotRunOptions,
) -> HeadlessBotBatch {
    let mode = config.clone().into_mode();
    let timeout_label = options
        .run_timeout
        .map(|d| format!("{d:?}"))
        .unwrap_or_else(|| "off".to_string());
    println!(
        "Running bot for {} runs (base_target={}, target_scaling={}, plays={}, discards={}, gold={}, log={}, run_timeout={}, timeout_retries={})...",
        n,
        mode.base_target,
        mode.target_scaling,
        mode.starting_plays,
        mode.starting_discards,
        mode.starting_gold,
        options.log,
        timeout_label,
        options.timeout_retries,
    );
    let paired: Vec<(RunStats, Vec<BotTimeoutDiag>)> = if n == 0 {
        Vec::new()
    } else if options.log {
        let pb = bot_runs_progress_bar(n);
        let out = (0..n)
            .progress_with(pb.clone())
            .map(|i| run_scheduled_bot_slot(i + 1, config.clone(), options.clone()))
            .collect();
        pb.finish_and_clear();
        out
    } else {
        let pb = bot_runs_progress_bar(n);
        let out = (0..n)
            .into_par_iter()
            .progress_with(pb.clone())
            .map(|i| run_scheduled_bot_slot(i + 1, config.clone(), options.clone()))
            .collect();
        pb.finish_and_clear();
        out
    };

    let mut timeout_events = Vec::new();
    let mut runs = Vec::with_capacity(paired.len());
    for (s, mut diags) in paired {
        timeout_events.append(&mut diags);
        runs.push(s);
    }

    let mut agg = AggregateStats::default();
    for (i, s) in runs.iter().enumerate() {
        agg.record(s);
        let run_number = i as u32 + 1;
        if run_number.is_multiple_of(25) || run_number == n {
            let outcome = if s.run_timed_out {
                format!(
                    "TIMED OUT ({})",
                    s.timeout_detail
                        .as_ref()
                        .map(|d| d.phase.as_str())
                        .unwrap_or("?")
                )
            } else if s.victory {
                format!("VICTORY (ante {})", s.died_on_ante)
            } else {
                format!("died ante {} on {}", s.died_on_ante, s.died_on_blind.name())
            };
            println!(
                "  [{:>4}/{}] last: {} (cleared {} blinds, score {})",
                run_number,
                n,
                outcome,
                s.blinds_cleared,
                human_readable_score(s.total_score as f64),
            );
        }
    }
    print_bot_timeout_report(&timeout_events);
    HeadlessBotBatch {
        aggregate: agg,
        runs,
        timeout_events,
    }
}

pub fn run_headless(n: u32, config: BotConfig, options: BotRunOptions) {
    let mode = config.clone().into_mode();
    let batch = run_headless_aggregate(n, config, options.clone());
    batch.aggregate.print_summary();
    if let Some(ref path) = options.output_runs {
        match write_runs_jsonl(path, &batch.runs) {
            Ok(()) => println!(
                "exported {} per-run JSON lines to {}",
                batch.runs.len(),
                path.display()
            ),
            Err(err) => eprintln!("failed to export runs to {}: {err}", path.display()),
        }
    }
    if let Some(ref target) = options.output {
        match write_bot_export(target, n, &mode, &batch.aggregate) {
            Ok(()) => println!(
                "exported bot {} to {}",
                target.format.export_label(),
                target.path.display()
            ),
            Err(err) => eprintln!(
                "failed to export bot {} to {}: {err}",
                target.format.export_label(),
                target.path.display()
            ),
        }
    }
}

pub fn run_sweep(
    runs_per_cell: u32,
    base_targets: &[u32],
    scalings: &[f32],
    plays_values: &[u32],
    export_json: Option<&Path>,
) {
    let mut export_cells = Vec::new();
    println!(
        "Sweep: {} bases × {} scalings × {} plays-values × {} runs/cell = {} runs total",
        base_targets.len(),
        scalings.len(),
        plays_values.len(),
        runs_per_cell,
        base_targets.len() * scalings.len() * plays_values.len() * runs_per_cell as usize,
    );
    println!();
    println!(
        "Each cell shows: antes_cleared_avg / win_rate_pct (avg blinds_cleared, avg total_score)"
    );

    for &plays in plays_values {
        println!("\n── starting_plays = {} ──", plays);
        print!("{:>10} |", "base \\ sc");
        for s in scalings {
            print!(" {:^22} |", format!("{:.2}", s));
        }
        println!();
        print!("{:->10}-+", "");
        for _ in scalings {
            print!("{:->24}+", "");
        }
        println!();

        for &base in base_targets {
            let cells: Vec<(String, AggregateStats, f32)> = scalings
                .par_iter()
                .map(|&scaling| {
                    let cfg = BotConfig {
                        base_target: Some(base),
                        target_scaling: Some(scaling),
                        starting_plays: Some(plays),
                        ..Default::default()
                    };
                    let agg = run_with_sequential(runs_per_cell, cfg, BotRunOptions::default());
                    let avg_antes = agg.antes_cleared_total as f64 / agg.runs as f64;
                    let win_pct = agg.victories as f64 * 100.0 / agg.runs as f64;
                    let avg_blinds = agg.blinds_cleared_total as f64 / agg.runs as f64;
                    let avg_score = agg.total_score as f64 / agg.runs as f64;
                    (
                        format!(
                            " {:>4.1}/{:>4.1}% ({:>3.1} blinds, {}) |",
                            avg_antes,
                            win_pct,
                            avg_blinds,
                            human_readable_score(avg_score)
                        ),
                        agg,
                        scaling,
                    )
                })
                .collect();
            if export_json.is_some() {
                for (_, agg, scaling) in &cells {
                    export_cells.push(SweepCellExport {
                        base_target: base,
                        target_scaling: *scaling,
                        starting_plays: plays,
                        aggregate: agg.clone(),
                    });
                }
            }
            print!("{:>10} |", base);
            for (cell, _, _) in cells {
                print!("{cell}");
            }
            println!();
        }
    }
    println!();
    if let Some(path) = export_json {
        let payload = SweepExport {
            runs_per_cell,
            cells: export_cells,
        };
        match write_json(path, &payload) {
            Ok(()) => println!("exported sweep JSON to {}", path.display()),
            Err(err) => eprintln!("failed to export sweep JSON to {}: {err}", path.display()),
        }
    }
}

pub(crate) fn human_readable_score(score: f64) -> String {
    let formatter = human_format::Formatter::new();
    formatter.format(score)
}

#[derive(serde::Deserialize)]
pub struct StrategyFile {
    pub strategies: Vec<StrategyDef>,
}

#[derive(serde::Deserialize, Clone)]
pub struct StrategyDef {
    pub name: String,
    #[serde(default)]
    pub relic_weight: Option<f32>,
    #[serde(default)]
    pub zodiac_weight: Option<f32>,
    #[serde(default)]
    pub talisman_weight: Option<f32>,
    #[serde(default)]
    pub pack_weight: Option<f32>,
    #[serde(default)]
    pub skip_threshold_multiplier: Option<f32>,
    #[serde(default)]
    pub base_target: Option<u32>,
    #[serde(default)]
    pub target_scaling: Option<f32>,
    #[serde(default)]
    pub starting_plays: Option<u32>,
    #[serde(default)]
    pub starting_discards: Option<u32>,
    #[serde(default)]
    pub starting_gold: Option<u32>,
}

impl StrategyDef {
    pub fn to_bot_config(&self) -> BotConfig {
        BotConfig {
            base_target: self.base_target,
            target_scaling: self.target_scaling,
            starting_plays: self.starting_plays,
            starting_discards: self.starting_discards,
            starting_gold: self.starting_gold,
            forced_relic: None,
            relic_weight: self.relic_weight,
            zodiac_weight: self.zodiac_weight,
            talisman_weight: self.talisman_weight,
            pack_weight: self.pack_weight,
            skip_threshold_multiplier: self.skip_threshold_multiplier,
            stake: None,
        }
    }
}

#[derive(Serialize)]
struct StrategySweepCellExport {
    name: String,
    aggregate: AggregateStats,
}

#[derive(Serialize)]
struct StrategySweepExport {
    runs_per_strategy: u32,
    strategies: Vec<StrategySweepCellExport>,
}

pub fn run_strategy_sweep(
    strategies: Vec<(String, BotConfig)>,
    runs_per_strategy: u32,
    export_json: Option<&Path>,
    run_options: BotRunOptions,
) {
    if strategies.is_empty() {
        println!("no strategies to run");
        return;
    }
    let timeout_label = run_options
        .run_timeout
        .map(|d| format!("{d:?}"))
        .unwrap_or_else(|| "off".to_string());
    println!(
        "Strategy sweep: {} strategies × {} runs = {} total (run_timeout={}, timeout_retries={})",
        strategies.len(),
        runs_per_strategy,
        strategies.len() * runs_per_strategy as usize,
        timeout_label,
        run_options.timeout_retries,
    );

    let total = strategies.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let start = std::time::Instant::now();
    let results: Vec<(String, AggregateStats)> = strategies
        .into_par_iter()
        .map(|(name, cfg)| {
            let agg = run_with_sequential(runs_per_strategy, cfg, run_options.clone());
            let finished = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            let elapsed = start.elapsed().as_secs_f64();
            let win_pct = agg.victories as f64 * 100.0 / agg.runs.max(1) as f64;
            println!(
                "  [{:>2}/{}] {:<24} win {:>5.1}%  ({:>5.1}s elapsed)",
                finished, total, name, win_pct, elapsed,
            );
            (name, agg)
        })
        .collect();

    let mut ranked = results;
    ranked.sort_by(|(_, a), (_, b)| {
        let wa = a.victories as f64 / a.runs.max(1) as f64;
        let wb = b.victories as f64 / b.runs.max(1) as f64;
        wb.partial_cmp(&wa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                let aa = a.antes_cleared_total as f64 / a.runs.max(1) as f64;
                let ab = b.antes_cleared_total as f64 / b.runs.max(1) as f64;
                ab.partial_cmp(&aa).unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    println!();
    println!(
        "{:>4}  {:<22}  {:>6}  {:>6}  {:>10}  {:>7}  {:>7}",
        "rank", "strategy", "win%", "antes", "score", "relics", "packs"
    );
    println!(
        "{:->4}  {:-<22}  {:->6}  {:->6}  {:->10}  {:->7}  {:->7}",
        "", "", "", "", "", "", ""
    );
    for (i, (name, agg)) in ranked.iter().enumerate() {
        let runs = agg.runs.max(1) as f64;
        let win_pct = agg.victories as f64 * 100.0 / runs;
        let antes = agg.antes_cleared_total as f64 / runs;
        let score = agg.total_score as f64 / runs;
        let relics = agg.total_relics_bought as f64 / runs;
        let packs: u64 = agg.packs_picked.values().map(|v| *v as u64).sum();
        let packs_per_run = packs as f64 / runs;
        println!(
            "{:>4}  {:<22}  {:>5.1}%  {:>6.2}  {:>10}  {:>7.2}  {:>7.2}",
            i + 1,
            name,
            win_pct,
            antes,
            human_readable_score(score),
            relics,
            packs_per_run,
        );
    }

    if let Some(path) = export_json {
        let payload = StrategySweepExport {
            runs_per_strategy,
            strategies: ranked
                .into_iter()
                .map(|(name, aggregate)| StrategySweepCellExport { name, aggregate })
                .collect(),
        };
        match write_json(path, &payload) {
            Ok(()) => println!("\nexported strategy sweep JSON to {}", path.display()),
            Err(err) => eprintln!(
                "\nfailed to export strategy sweep JSON to {}: {err}",
                path.display()
            ),
        }
    }
}

#[derive(Serialize)]
struct ForcedRelicCellExport {
    relic: String,
    aggregate: AggregateStats,
}

#[derive(Serialize)]
struct ForcedRelicExport {
    runs_per_relic: u32,
    baseline_aggregate: AggregateStats,
    relics: Vec<ForcedRelicCellExport>,
}

pub fn run_forced_relic_sweep(runs_per_relic: u32, export_json: Option<&Path>) {
    let relic_ids: Vec<RelicId> = all_relic_defs().iter().map(|d| d.id).collect();
    let total = relic_ids.len() + 1;
    println!(
        "Forced-relic sweep: {} relics + 1 control × {} runs = {} total",
        relic_ids.len(),
        runs_per_relic,
        total * runs_per_relic as usize,
    );

    let done = std::sync::atomic::AtomicUsize::new(0);
    let start = std::time::Instant::now();

    let mut cells: Vec<Option<RelicId>> = Vec::with_capacity(total);
    cells.push(None);
    for id in &relic_ids {
        cells.push(Some(*id));
    }

    let results: Vec<(Option<RelicId>, AggregateStats)> = cells
        .into_par_iter()
        .map(|forced| {
            let cfg = BotConfig {
                forced_relic: forced,
                ..Default::default()
            };
            let agg = run_with_sequential(runs_per_relic, cfg, BotRunOptions::default());
            let finished = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            let elapsed = start.elapsed().as_secs_f64();
            let win_pct = agg.victories as f64 * 100.0 / agg.runs.max(1) as f64;
            let label = match forced {
                Some(id) => relic_display_name(id),
                None => "(control)",
            };
            println!(
                "  [{:>2}/{}] {:<24} win {:>5.1}%  ({:>5.1}s elapsed)",
                finished, total, label, win_pct, elapsed,
            );
            (forced, agg)
        })
        .collect();

    let mut baseline = None;
    let mut relic_rows = Vec::new();
    for (forced, agg) in results {
        match forced {
            None => baseline = Some(agg),
            Some(id) => relic_rows.push((id, agg)),
        }
    }
    let baseline = baseline.expect("control cell missing");
    let baseline_win = baseline.victories as f64 * 100.0 / baseline.runs.max(1) as f64;

    relic_rows.sort_by(|(_, a), (_, b)| {
        let wa = a.victories as f64 / a.runs.max(1) as f64;
        let wb = b.victories as f64 / b.runs.max(1) as f64;
        wb.partial_cmp(&wa).unwrap_or(std::cmp::Ordering::Equal)
    });

    println!();
    println!("Control win rate: {:.1}%\n", baseline_win);
    println!(
        "{:>4}  {:<22}  {:>6}  {:>7}  {:>7}  {:>10}",
        "rank", "relic", "win%", "Δwin", "antes", "score"
    );
    println!(
        "{:->4}  {:-<22}  {:->6}  {:->7}  {:->7}  {:->10}",
        "", "", "", "", "", ""
    );
    for (i, (id, agg)) in relic_rows.iter().enumerate() {
        let runs = agg.runs.max(1) as f64;
        let win_pct = agg.victories as f64 * 100.0 / runs;
        let delta = win_pct - baseline_win;
        let antes = agg.antes_cleared_total as f64 / runs;
        let score = agg.total_score as f64 / runs;
        println!(
            "{:>4}  {:<22}  {:>5.1}%  {:>+6.1}  {:>7.2}  {:>10}",
            i + 1,
            relic_display_name(*id),
            win_pct,
            delta,
            antes,
            human_readable_score(score),
        );
    }

    if let Some(path) = export_json {
        let payload = ForcedRelicExport {
            runs_per_relic,
            baseline_aggregate: baseline,
            relics: relic_rows
                .into_iter()
                .map(|(id, aggregate)| ForcedRelicCellExport {
                    relic: relic_display_name(id).to_string(),
                    aggregate,
                })
                .collect(),
        };
        match write_json(path, &payload) {
            Ok(()) => println!("\nexported forced-relic sweep JSON to {}", path.display()),
            Err(err) => eprintln!(
                "\nfailed to export forced-relic sweep JSON to {}: {err}",
                path.display()
            ),
        }
    }
}

fn blinds_cleared_before_death(blind: crate::core::rules::BlindKind) -> u32 {
    match blind {
        crate::core::rules::BlindKind::Small => 0,
        crate::core::rules::BlindKind::Big => 1,
        crate::core::rules::BlindKind::Boss => 2,
    }
}

/// Approximate [`RunStats`] from a saved [`crate::core::progression::RunRecord`] for the
/// play-history HTML export. Fields that are only tracked in the headless bot stay default.
pub fn run_stats_from_progress_record(rec: &crate::core::progression::RunRecord) -> RunStats {
    use crate::core::progression::RunOutcome;
    use crate::core::relic::all_relic_defs;
    use crate::core::rules::BlindKind;
    use crate::game::run::FINAL_ANTE;
    use std::collections::BTreeMap;

    let victory = matches!(rec.outcome, RunOutcome::Victory);
    let (antes_cleared, blinds_cleared) = if victory {
        (FINAL_ANTE, FINAL_ANTE.saturating_mul(3))
    } else {
        let completed_antes = rec.final_ante.saturating_sub(1);
        (
            completed_antes,
            completed_antes
                .saturating_mul(3)
                .saturating_add(blinds_cleared_before_death(rec.final_blind)),
        )
    };
    let death_reason = match rec.outcome {
        RunOutcome::Defeat { reason } => Some(reason),
        _ => None,
    };
    let mut yaku_scored: BTreeMap<&'static str, u32> = BTreeMap::new();
    for (&yk, &n) in &rec.yaku_times_played {
        *yaku_scored.entry(yk.name()).or_insert(0) += n;
    }
    let final_relics: Vec<String> = rec
        .relics_owned
        .iter()
        .map(|&id| {
            all_relic_defs()
                .iter()
                .find(|d| d.id == id)
                .map(|d| d.name.to_string())
                .unwrap_or_else(|| format!("{id:?}"))
        })
        .collect();
    let final_consumables: Vec<String> = rec.consumables_owned.iter().map(|c| c.name()).collect();
    let mut boss_faced: BTreeMap<String, u8> = BTreeMap::new();
    let mut boss_beaten: BTreeMap<String, u8> = BTreeMap::new();
    if rec.final_blind == BlindKind::Boss
        && let Some(bk) = rec.final_boss {
            let key = bk.name().to_string();
            boss_faced.insert(key.clone(), 1);
            if victory {
                boss_beaten.insert(key, 1);
            }
        }
    RunStats {
        blinds_cleared,
        antes_cleared,
        victory,
        died_on_ante: rec.final_ante,
        died_on_blind: rec.final_blind,
        total_score: rec.total_score_earned,
        discards_used: rec.tiles_discarded,
        final_gold: rec.final_gold,
        peak_blind_score: rec.best_structure_score,
        yaku_scored,
        final_relics,
        final_consumables,
        death_reason,
        boss_faced,
        boss_beaten,
        ..RunStats::default()
    }
}

fn boss_kind_by_display_name(name: &str) -> Option<crate::core::boss::BossKind> {
    use crate::core::boss::{all_bosses, final_bosses};
    all_bosses()
        .iter()
        .chain(final_bosses().iter())
        .find(|d| d.name == name)
        .map(|d| d.kind)
}

/// Merge one finished bot run into profile progression (chronicle + career counters).
pub fn append_bot_run_to_progress(
    progress: &mut crate::core::progression::PlayerProgress,
    run: &crate::game::run::RunState,
    stats: &RunStats,
    history_index: u64,
) {
    use crate::core::progression::{RunOutcome, RunRecord};
    use crate::core::relic::all_relic_defs;
    use crate::core::talisman::TalismanKind;
    use crate::core::yaku::YakuKind;
    use crate::game::event_bus::GameOverReason;

    let outcome = if stats.victory {
        RunOutcome::Victory
    } else {
        RunOutcome::Defeat {
            reason: stats
                .death_reason
                .unwrap_or(GameOverReason::OutOfPlays),
        }
    };
    let mut record = RunRecord::from_run(run, outcome);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    record.timestamp_unix = now.saturating_sub(history_index.saturating_mul(86_400));
    record.tutorial_run = false;

    progress.run_history.push(record);
    progress.runs_completed = progress.runs_completed.saturating_add(1);
    progress.record_score(stats.total_score);
    if stats.victory {
        progress.has_won = true;
        let _ = progress.record_stake_victory(run.mode.tile_material, run.mode.stake);
    }

    for (&name, &count) in &stats.yaku_scored {
        if let Some(yk) = YakuKind::all().iter().find(|yk| yk.name() == name) {
            *progress.yaku_times_scored.entry(*yk).or_insert(0) += count;
        }
    }
    for (name, &faced) in &stats.boss_faced {
        if let Some(kind) = boss_kind_by_display_name(name) {
            *progress
                .boss_times_encountered
                .entry(kind)
                .or_insert(0) += u32::from(faced);
        }
    }
    for (name, &beaten) in &stats.boss_beaten {
        if let Some(kind) = boss_kind_by_display_name(name) {
            *progress.boss_times_defeated.entry(kind).or_insert(0) += u32::from(beaten);
        }
    }
    for (&name, &n) in &stats.relic_activations {
        if let Some(def) = all_relic_defs().iter().find(|d| d.name == name) {
            *progress
                .relic_times_activated
                .entry(def.id)
                .or_insert(0) += n;
        }
    }
    for (&name, &n) in &stats.talismans_picked {
        if let Some(kind) = TalismanKind::all().iter().find(|tk| tk.name() == name) {
            *progress
                .talisman_times_purchased
                .entry(*kind)
                .or_insert(0) += n;
        }
    }
    for (&name, &n) in &stats.talismans_used {
        if let Some(kind) = TalismanKind::all().iter().find(|tk| tk.name() == name) {
            *progress.talisman_times_used.entry(*kind).or_insert(0) += n;
        }
    }
}

/// Play `count` headless bot runs and append each to `progress.run_history` (and related career stats).
/// Returns how many runs were recorded (timeouts still count if they produced terminal state).
pub fn seed_progress_from_bot_runs(
    progress: &mut crate::core::progression::PlayerProgress,
    count: usize,
) -> usize {
    if count == 0 {
        return 0;
    }
    let config = BotConfig::default();
    let options = BotRunOptions {
        log: false,
        ..BotRunOptions::default()
    };
    let base_index = progress.run_history.len() as u64;
    for i in 0..count {
        let run_idx = progress.runs_completed.saturating_add(1).max(1);
        let (run, stats) = super::play_bot_run(config.clone(), options.clone(), Some(run_idx));
        append_bot_run_to_progress(progress, &run, &stats, base_index + i as u64);
    }
    count
}

/// Write the same interactive HTML report as `mahjuro bot -o …html`, using
/// completed runs from [`crate::core::progression::PlayerProgress::run_history`].
/// Tutorial runs are skipped. Returns an error when there is nothing to export.
pub fn export_play_history_html(
    path: &Path,
    progress: &crate::core::progression::PlayerProgress,
) -> anyhow::Result<()> {
    use crate::game::game_mode::GameMode;

    let records: Vec<&crate::core::progression::RunRecord> = progress
        .run_history
        .iter()
        .filter(|r| !r.tutorial_run)
        .collect();
    if records.is_empty() {
        anyhow::bail!(
            "No completed runs to export yet (tutorial runs are excluded). Finish a run first."
        );
    }
    let last = records.last().expect("non-empty");
    let mode = GameMode::with_material_and_stake(last.tile_material, last.stake);
    let mut agg = AggregateStats::default();
    for rec in &records {
        agg.record(&run_stats_from_progress_record(rec));
    }
    let runs = records.len() as u32;
    let v = bot_export_value(runs, &mode, &agg)?;
    let json_compact = serde_json::to_string(&v)?;
    let embedded = escape_json_for_html_script(&json_compact);
    write_bot_html(path, &embedded)?;
    Ok(())
}
