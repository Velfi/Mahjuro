use super::*;

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

#[derive(Clone, Debug, Default)]
pub struct BotRunOptions {
    pub log: bool,
    pub export_json: Option<PathBuf>,
}

#[derive(Serialize)]
struct BotExport<'a> {
    runs: u32,
    mode: &'a GameMode,
    aggregate: &'a AggregateStats,
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

impl BotConfig {
    pub fn into_mode(self) -> GameMode {
        let mut mode = GameMode::standard();
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

pub fn play_run_with(config: BotConfig) -> RunStats {
    super::play_run_with_options(config, BotRunOptions::default(), None)
}

pub(crate) fn run_with_sequential(n: u32, config: BotConfig) -> AggregateStats {
    let mut agg = AggregateStats::default();
    for _ in 0..n {
        let s = play_run_with(config.clone());
        agg.record(&s);
    }
    agg
}

pub fn run_headless_aggregate(n: u32, config: BotConfig, options: BotRunOptions) -> AggregateStats {
    let mode = config.clone().into_mode();
    println!(
        "Running bot for {} runs (base_target={}, target_scaling={}, plays={}, discards={}, gold={}, log={})...",
        n,
        mode.base_target,
        mode.target_scaling,
        mode.starting_plays,
        mode.starting_discards,
        mode.starting_gold,
        options.log,
    );
    let runs: Vec<RunStats> = if options.log {
        (0..n)
            .map(|i| super::play_run_with_options(config.clone(), options.clone(), Some(i + 1)))
            .collect()
    } else {
        (0..n)
            .into_par_iter()
            .map(|i| super::play_run_with_options(config.clone(), options.clone(), Some(i + 1)))
            .collect()
    };

    let mut agg = AggregateStats::default();
    for (i, s) in runs.iter().enumerate() {
        agg.record(s);
        let run_number = i as u32 + 1;
        if run_number.is_multiple_of(25) || run_number == n {
            let outcome = if s.victory {
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
    agg
}

pub fn run_headless(n: u32, config: BotConfig, options: BotRunOptions) {
    let mode = config.clone().into_mode();
    let agg = run_headless_aggregate(n, config, options.clone());
    agg.print_summary();
    if let Some(path) = options.export_json.as_deref() {
        let payload = BotExport {
            runs: n,
            mode: &mode,
            aggregate: &agg,
        };
        match write_json(path, &payload) {
            Ok(()) => println!("exported bot JSON to {}", path.display()),
            Err(err) => eprintln!("failed to export bot JSON to {}: {err}", path.display()),
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
                    let agg = run_with_sequential(runs_per_cell, cfg);
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
) {
    if strategies.is_empty() {
        println!("no strategies to run");
        return;
    }
    println!(
        "Strategy sweep: {} strategies × {} runs = {} total",
        strategies.len(),
        runs_per_strategy,
        strategies.len() * runs_per_strategy as usize,
    );

    let total = strategies.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    let start = std::time::Instant::now();
    let results: Vec<(String, AggregateStats)> = strategies
        .into_par_iter()
        .map(|(name, cfg)| {
            let agg = run_with_sequential(runs_per_strategy, cfg);
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
            let agg = run_with_sequential(runs_per_relic, cfg);
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
