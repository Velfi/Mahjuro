use super::*;
use std::process::Command as ProcessCommand;

#[derive(Debug, Serialize, Deserialize)]
pub struct BotGraphSnapshot {
    pub slug: String,
    pub label: String,
    pub runs: u32,
    pub win_rate: f64,
    pub avg_blinds: f64,
    pub avg_antes: f64,
    pub avg_total_score_m: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_surplus_per_blind: Option<f64>,
    pub avg_plays: f64,
    pub avg_discards: f64,
    pub avg_skips: f64,
    pub avg_relics: f64,
    pub avg_gold_spent: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_gold_earned: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clear_base: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clear_plays: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clear_interest: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clear_relics: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_final_gold: Option<f64>,
    pub deaths_by_ante: std::collections::BTreeMap<u32, u32>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub overscore_by_slot: std::collections::BTreeMap<String, u64>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub cleared_by_slot: std::collections::BTreeMap<String, u64>,
}

fn avg_u64(total: u64, runs: u32) -> f64 {
    if runs == 0 {
        0.0
    } else {
        total as f64 / runs as f64
    }
}

fn avg_i64(total: i64, runs: u32) -> f64 {
    if runs == 0 {
        0.0
    } else {
        total as f64 / runs as f64
    }
}

pub fn default_snapshot_slug(mode: &game::game_mode::GameMode, runs: u32) -> String {
    format!(
        "bt{}_ts{}_p{}_d{}_g{}_{}",
        mode.base_target,
        format!("{:.2}", mode.target_scaling).replace('.', "_"),
        mode.starting_plays,
        mode.starting_discards,
        mode.starting_gold,
        runs
    )
}

pub fn default_snapshot_label(mode: &game::game_mode::GameMode, runs: u32) -> String {
    format!(
        "Base {}\nScale {:.2}\nP{} D{} G{}\n({} runs)",
        mode.base_target,
        mode.target_scaling,
        mode.starting_plays,
        mode.starting_discards,
        mode.starting_gold,
        runs
    )
}

pub fn build_bot_graph_snapshot(
    agg: &bot::AggregateStats,
    slug: String,
    label: String,
) -> BotGraphSnapshot {
    let runs = agg.runs;
    BotGraphSnapshot {
        slug,
        label,
        runs,
        win_rate: if runs == 0 {
            0.0
        } else {
            agg.victories as f64 * 100.0 / runs as f64
        },
        avg_blinds: avg_u64(agg.blinds_cleared_total, runs),
        avg_antes: avg_u64(agg.antes_cleared_total, runs),
        avg_total_score_m: avg_u64(agg.total_score, runs) / 1_000_000.0,
        avg_surplus_per_blind: if agg.blinds_cleared_total == 0 {
            Some(0.0)
        } else {
            Some(agg.total_overscore as f64 / agg.blinds_cleared_total as f64)
        },
        avg_plays: avg_u64(agg.total_plays, runs),
        avg_discards: avg_u64(agg.total_discards, runs),
        avg_skips: avg_u64(agg.total_blinds_skipped, runs),
        avg_relics: avg_u64(agg.total_relics_bought, runs),
        avg_gold_spent: avg_u64(agg.total_gold_spent, runs),
        avg_gold_earned: Some(avg_u64(
            agg.total_gold_from_clears + agg.total_gold_from_skip_tags,
            runs,
        )),
        clear_base: Some(avg_u64(agg.total_gold_from_clear_base, runs)),
        clear_plays: Some(avg_u64(agg.total_gold_from_unused_plays, runs)),
        clear_interest: Some(avg_u64(agg.total_gold_from_interest, runs)),
        clear_relics: Some(avg_u64(agg.total_gold_from_clear_relics, runs)),
        avg_final_gold: Some(avg_i64(agg.total_final_gold, runs)),
        deaths_by_ante: agg.deaths_by_ante.clone(),
        overscore_by_slot: agg.overscore_by_slot.clone(),
        cleared_by_slot: agg.cleared_by_slot.clone(),
    }
}

fn load_snapshots(path: &Path) -> anyhow::Result<Vec<BotGraphSnapshot>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&raw)?)
}

fn write_snapshots(path: &Path, snapshots: &[BotGraphSnapshot]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(snapshots)?;
    std::fs::write(path, json)?;
    Ok(())
}

pub fn upsert_snapshot(path: &Path, snapshot: BotGraphSnapshot) -> anyhow::Result<()> {
    let mut snapshots = load_snapshots(path)?;
    if let Some(existing) = snapshots.iter_mut().find(|item| item.slug == snapshot.slug) {
        *existing = snapshot;
    } else {
        snapshots.push(snapshot);
    }
    write_snapshots(path, &snapshots)
}

pub fn render_bot_graphs(repo_root: &Path) -> anyhow::Result<()> {
    let status = ProcessCommand::new("python3")
        .arg("tools/plot_bot_balance.py")
        .current_dir(repo_root)
        .status()?;
    anyhow::ensure!(status.success(), "graph render failed with status {status}");
    Ok(())
}
