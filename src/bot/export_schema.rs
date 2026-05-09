//! Serializable shapes for `mahjuro bot -o report.json` (schema version 2).
//! `aggregate` splits sums vs maps; `derived` holds precomputed dashboard / summary numbers.

use serde::Serialize;
use std::collections::BTreeMap;

pub const EXPORT_SCHEMA_VERSION: u32 = 2;

#[derive(Serialize)]
pub struct BotExportMeta {
    pub yaku_kind_count: usize,
    pub crate_version: &'static str,
}

#[derive(Serialize)]
pub struct AggregateSums {
    pub runs: u32,
    pub blinds_cleared_total: u64,
    pub antes_cleared_total: u64,
    pub victories: u32,
    pub max_ante_reached: u32,
    pub total_score: u64,
    pub total_plays: u64,
    pub total_discards: u64,
    pub total_strategic_discards: u64,
    pub total_blinds_skipped: u64,
    pub total_relics_bought: u64,
    pub total_gold_spent: u64,
    pub total_final_gold: i64,
    pub total_gold_from_clears: u64,
    pub total_gold_from_clear_base: u64,
    pub total_gold_from_unused_plays: u64,
    pub total_gold_from_interest: u64,
    pub total_gold_from_clear_relics: u64,
    pub total_gold_from_skip_tags: u64,
    pub total_skip_tag_gold_value: u64,
    pub total_target_score: u64,
    pub total_overscore: u64,
    pub peak_blind_score: u64,
    pub total_bot_issue_no_valid_hand: u64,
    pub total_bot_issue_only_valid_unplayable: u64,
    pub total_bot_issue_only_valid_no_score: u64,
    pub total_bot_issue_other_stuck: u64,
    pub total_bot_issue_lost_with_available_lines: u64,
    pub total_structure_triggers: u64,
    pub total_structure_trigger_points: u64,
    pub total_second_wind_forfeits: u64,
    pub deaths_out_of_plays: u32,
    pub deaths_no_actions_remaining: u32,
    pub total_gold_clear_green_luck: u64,
    pub total_gold_clear_gold_idol: u64,
    pub total_gold_clear_jade_abacus: u64,
    pub total_gold_clear_patience: u64,
    pub total_turns: u64,
    pub sum_peak_hand_size: u64,
    pub total_tiles_destroyed: u64,
}

#[derive(Serialize)]
pub struct AggregateMaps {
    pub bot_issues_by_reason: BTreeMap<String, u32>,
    pub deaths_by_ante: BTreeMap<String, u32>,
    pub deaths_by_blind: BTreeMap<String, u32>,
    pub deaths_by_ante_cause: BTreeMap<String, u32>,
    pub skipped_tags: BTreeMap<String, u32>,
    pub relics_picked: BTreeMap<String, u32>,
    pub relics_picked_victories: BTreeMap<String, u32>,
    pub talismans_picked: BTreeMap<String, u32>,
    pub zodiacs_picked: BTreeMap<String, u32>,
    pub packs_picked: BTreeMap<String, u32>,
    pub bot_issues_by_blind: BTreeMap<String, u32>,
    pub bot_issues_by_boss: BTreeMap<String, u32>,
    pub overscore_by_slot: BTreeMap<String, u64>,
    pub cleared_by_slot: BTreeMap<String, u64>,
    pub turns_by_blind_slot: BTreeMap<String, u64>,
    pub turns_cleared_by_slot: BTreeMap<String, u64>,
    pub discards_by_blind_slot: BTreeMap<String, u64>,
    pub boss_faced: BTreeMap<String, u32>,
    pub boss_beaten: BTreeMap<String, u32>,
    pub yaku_scored: BTreeMap<String, u64>,
    pub zodiacs_used: BTreeMap<String, u64>,
    pub talismans_used: BTreeMap<String, u64>,
    pub relic_activations: BTreeMap<String, u64>,
    pub transformations_successor: BTreeMap<String, u64>,
}

#[derive(Serialize)]
pub struct BotAggregateV2 {
    pub sums: AggregateSums,
    pub maps: AggregateMaps,
}

#[derive(Default, Serialize)]
pub struct PerRunAverages {
    pub win_rate_pct: f64,
    pub blinds_cleared: f64,
    pub antes_cleared: f64,
    pub total_score: f64,
    pub plays: f64,
    pub discards: f64,
    pub strategic_discards: f64,
    pub random_discards: f64,
    pub blinds_skipped: f64,
    pub relics_bought: f64,
    pub gold_spent: f64,
    pub gold_from_clears: f64,
    pub gold_from_skip_tags: f64,
    pub skip_tag_gold_value: f64,
    pub gold_clear_base: f64,
    pub gold_clear_unused_plays: f64,
    pub gold_clear_interest: f64,
    pub gold_clear_relics: f64,
    pub gold_clear_green_luck: f64,
    pub gold_clear_gold_idol: f64,
    pub gold_clear_jade_abacus: f64,
    pub gold_clear_patience: f64,
    pub second_wind_forfeits: f64,
    pub structure_triggers: f64,
    pub structure_trigger_points_per_run: f64,
    pub turns: f64,
    pub peak_hand_size: f64,
    pub tiles_destroyed: f64,
    pub relic_activations: f64,
    pub final_gold: f64,
    pub target_score_faced: f64,
    pub overscore: f64,
    pub score_to_target_ratio: f64,
}

#[derive(Serialize)]
pub struct KpiTile {
    pub id: String,
    pub label: String,
    pub value: String,
    pub hint: String,
    pub highlight: bool,
}

#[derive(Serialize)]
pub struct LossBreakdownDerived {
    pub losses: u32,
    pub out_of_plays: u32,
    pub out_of_plays_pct_of_losses: f64,
    pub no_actions_remaining: u32,
    pub no_actions_pct_of_losses: f64,
}

#[derive(Serialize)]
pub struct DeathAnteRow {
    pub ante: u32,
    pub count: u32,
    pub pct_of_runs: f64,
    /// ASCII bar length (CLI), up to 50.
    pub text_bar_hashes: u32,
    /// 0–100 width vs largest ante bucket (dashboard).
    pub dashboard_bar_pct: f64,
}

#[derive(Serialize)]
pub struct NamedCountPct {
    pub name: String,
    pub count: u32,
    pub pct_of_runs: f64,
}

#[derive(Serialize)]
pub struct SurplusSlotRow {
    pub slot: String,
    pub clears: u64,
    pub avg_surplus: f64,
    /// 0–100 log-scaled vs max bucket (matches CLI).
    pub bar_pct: f64,
}

#[derive(Serialize)]
pub struct AvgTurnsClearRow {
    pub slot: String,
    pub avg_turns: f64,
}

#[derive(Serialize)]
pub struct RelicBuyRow {
    pub name: String,
    pub bought: u32,
    pub won: u32,
    pub win_pct: f64,
}

#[derive(Serialize)]
pub struct RelicWinRateRow {
    pub name: String,
    pub bought: u32,
    pub won: u32,
    pub win_pct: f64,
    pub delta_vs_baseline_pct: f64,
}

#[derive(Serialize)]
pub struct YakuRow {
    pub name: String,
    pub awards: u64,
    pub per_run: f64,
    pub share_pct: f64,
    pub delta_uniform_pct: f64,
}

#[derive(Serialize)]
pub struct YakuDerived {
    pub total_awards: u64,
    pub awards_per_run: f64,
    pub uniform_baseline_pct: f64,
    pub kind_count: usize,
    pub rows: Vec<YakuRow>,
    pub never_awarded: Vec<String>,
}

#[derive(Serialize)]
pub struct NamedCount {
    pub name: String,
    pub count: u64,
}

#[derive(Serialize)]
pub struct NamedPerRun {
    pub name: String,
    pub count: u32,
    pub per_run: f64,
}

#[derive(Serialize)]
pub struct BotIssuesDerived {
    pub dead_end_total: u64,
    pub no_valid: u64,
    pub only_unplayable: u64,
    pub only_no_score: u64,
    pub other: u64,
    pub lost_with_lines: u64,
    pub hottest_blinds: Vec<(String, u32)>,
    pub hottest_bosses: Vec<(String, u32)>,
    pub reasons_top: Vec<(String, u32)>,
}

#[derive(Serialize)]
pub struct MapTable {
    pub title: String,
    pub rows: Vec<NamedCount>,
}

#[derive(Serialize)]
pub struct BotReportDerived {
    pub per_run: PerRunAverages,
    pub kpis: Vec<KpiTile>,
    pub loss_breakdown: Option<LossBreakdownDerived>,
    pub deaths_by_ante: Vec<DeathAnteRow>,
    pub deaths_by_blind: Vec<NamedCountPct>,
    pub surplus_by_slot: Vec<SurplusSlotRow>,
    pub avg_turns_to_clear: Vec<AvgTurnsClearRow>,
    pub skip_tags: Vec<NamedCountPct>,
    pub consumable_zodiacs: Vec<NamedCount>,
    pub consumable_talismans: Vec<NamedCount>,
    pub transformations_top: Vec<NamedCount>,
    pub relics_bought: Vec<RelicBuyRow>,
    pub relics_by_win_rate: Vec<RelicWinRateRow>,
    pub talismans_shop: Vec<NamedPerRun>,
    pub zodiacs_shop: Vec<NamedPerRun>,
    pub packs_shop: Vec<NamedPerRun>,
    pub yaku: Option<YakuDerived>,
    pub bot_issues: Option<BotIssuesDerived>,
    pub supplementary_tables: Vec<MapTable>,
}

impl Default for BotReportDerived {
    fn default() -> Self {
        Self {
            per_run: PerRunAverages::default(),
            kpis: Vec::new(),
            loss_breakdown: None,
            deaths_by_ante: Vec::new(),
            deaths_by_blind: Vec::new(),
            surplus_by_slot: Vec::new(),
            avg_turns_to_clear: Vec::new(),
            skip_tags: Vec::new(),
            consumable_zodiacs: Vec::new(),
            consumable_talismans: Vec::new(),
            transformations_top: Vec::new(),
            relics_bought: Vec::new(),
            relics_by_win_rate: Vec::new(),
            talismans_shop: Vec::new(),
            zodiacs_shop: Vec::new(),
            packs_shop: Vec::new(),
            yaku: None,
            bot_issues: None,
            supplementary_tables: Vec::new(),
        }
    }
}
