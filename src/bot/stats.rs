use std::io::Write;

use serde::{Deserialize, Serialize};

use super::*;
use crate::bot::reporting::human_readable_score;
use crate::core::relic::RelicId;
use crate::core::yaku::YakuKind;
use crate::core::zodiac::YakuLevels;
use crate::game::event_bus::GameOverReason;
use crate::game::run::RunState;

/// Shop visits with `run.wing <=` this value (after `advance_round`, before the
/// next blind) count toward **early** relic timing stats; larger ante counts as **late**.
/// Used only for bot reporting (selection-bias visibility), not gameplay.
pub const RELIC_SHOP_TIMING_EARLY_WING_MAX: u32 = 3;

/// Minimum shop purchases in **each** of early/late buckets to emit a timing-split row.
pub const MIN_SHOP_TIMING_SPLIT_PER_BUCKET: u32 = 15;

/// Categorize why a non-victory run ended, for balance-plot bucketing.
/// Returns one of: "no-legal-hand", "only-unplayable", "only-no-score",
/// "stuck-other", or "target-miss" (scoring failure without systemic block).
fn classify_run_death_cause(s: &RunStats) -> &'static str {
    if s.bot_issue_no_valid_hand > 0 {
        "no-legal-hand"
    } else if s.bot_issue_only_valid_unplayable > 0 {
        "only-unplayable"
    } else if s.bot_issue_only_valid_no_score > 0 {
        "only-no-score"
    } else if s.bot_issue_other_stuck > 0 {
        "stuck-other"
    } else {
        "target-miss"
    }
}

/// Snapshot when a headless bot run hits the wall-clock deadline (see `BotRunOptions::run_timeout`).
#[derive(Debug, Clone, Serialize)]
pub struct RunTimeoutSnapshot {
    pub phase: String,
    pub wing: u32,
    pub chamber: String,
    /// Decision turns completed this blind when phase is `playing_chamber`, else `None`.
    pub chamber_turn: Option<u32>,
    /// Raw `RunState` fields at timeout. Meaning depends on [`Self::phase`]:
    /// - `playing_chamber`: cumulative score toward the **current** blind vs its `target_score`.
    /// - `shop`: usually totals from the **just-cleared** blind (often ≫ target); `target_score`
    ///   is not the upcoming fight’s goal — `apply_chamber` has not run yet.
    /// - `outer`: between high-level loop iterations; same fields, rarely hit.
    pub round_score: u64,
    pub target_score: u32,
    pub plays_remaining: u32,
    pub discards_remaining: u32,
    pub elapsed_ms: u64,
}

/// One scoring step toward [`PeakChamberSnapshot::total_score`] (a committed play or structure cash-in).
///
/// `tiles`: meld-grouped labels (same as cascade HUD, e.g. `Kong  9p 9p 9p 9p · Triplet  3m 3m 3m`) —
/// full structure on cash-in; partial commits show only the melds just played.
fn bool_is_false(v: &bool) -> bool {
    !*v
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotScoreSourceEntry {
    pub name: String,
    pub points: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeakChamberRelicState {
    /// 1-based tray slot index.
    pub slot: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub counter: Option<i32>,
    #[serde(default, skip_serializing_if = "bool_is_false")]
    pub debuffed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotScoringAction {
    pub kind: String,
    pub points: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiles: Option<String>,
    /// Base meld chips from tiles before yaku/relic multipliers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tiles_points: Option<u64>,
    /// Round-score delta attributed to yaku / dora score steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yaku_points: Option<u64>,
    /// Round-score delta attributed to relic-named score steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relic_points: Option<u64>,
    /// Any remaining score delta not classified as tiles/yaku/relic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other_points: Option<u64>,
    /// Gold in wallet at score time (used by economy relics like Golden Engine).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yen_held: Option<i32>,
    /// Golden Engine mult bonus (`floor(gold_held / 3)`) when active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub golden_engine_mult_bonus: Option<i32>,
    /// Yaku detected for this action (with levels used at score time).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub yaku: Vec<String>,
    /// Yaku/Dora scoring steps and their score deltas.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub yaku_steps: Vec<BotScoreSourceEntry>,
    /// Relic-named scoring steps and their score deltas.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relic_steps: Vec<BotScoreSourceEntry>,
}

/// Context for the highest single-blind `round_score` seen in a batch (or one run).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeakChamberSnapshot {
    #[serde(alias = "blind_slot")]
    pub chamber_slot: String,
    #[serde(alias = "blind_label")]
    pub chamber_label: String,
    pub target_score: u32,
    pub total_score: u64,
    pub relics: Vec<String>,
    /// Wallet gold at blind end (also used by Golden Engine scaling).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub yen_held: Option<i32>,
    /// `floor(yen_held / 3)` at blind end when Golden Engine is active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub golden_engine_mult_bonus: Option<i32>,
    /// Active relic slot state at blind end (counter/debuff visibility).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub relic_state: Vec<PeakChamberRelicState>,
    pub scoring_actions: Vec<BotScoringAction>,
}

pub(crate) fn aggregate_stats_slot_sort_key(slot: &str) -> (u32, u32) {
    let (ante_str, rest) = slot.split_once('-').unwrap_or((slot, ""));
    let ante = ante_str.parse::<u32>().unwrap_or(99);
    let blind_order = match rest {
        "Small Chamber" => 0,
        "Big Chamber" => 1,
        "Ordeal Chamber" => 2,
        _ => 99,
    };
    (ante, blind_order)
}

#[derive(Debug, Clone, Serialize)]
pub struct RunStats {
    pub chambers_cleared: u32,
    pub wings_cleared: u32,
    pub victory: bool,
    pub died_on_wing: u32,
    pub died_on_chamber: ChamberKind,
    pub total_score: u64,
    pub plays_used: u32,
    pub discards_used: u32,
    pub strategic_discards: u32,
    pub final_yen: i32,
    pub chambers_skipped: u32,
    pub relics_bought: u32,
    /// Proactive or swap-driven relic sells during shop visits.
    pub relics_sold: u32,
    pub relics_sold_picked: std::collections::BTreeMap<&'static str, u32>,
    /// Relic offerings rolled in shop stock this run (including unpurchased).
    #[serde(default)]
    pub relic_shop_offers: std::collections::BTreeMap<&'static str, u32>,
    /// Bot marginal-value estimate at purchase time.
    #[serde(default)]
    pub relic_marginal_buy_sum: std::collections::BTreeMap<&'static str, i64>,
    #[serde(default)]
    pub relic_marginal_buy_count: std::collections::BTreeMap<&'static str, u32>,
    #[serde(default)]
    pub relic_marginal_buy_min: std::collections::BTreeMap<&'static str, i32>,
    #[serde(default)]
    pub relic_marginal_buy_max: std::collections::BTreeMap<&'static str, i32>,
    #[serde(default)]
    pub relic_marginal_buy_buckets:
        std::collections::BTreeMap<&'static str, std::collections::BTreeMap<&'static str, u32>>,
    /// Hold-value estimate when sold (proactive or swap).
    #[serde(default)]
    pub relic_hold_sell_sum: std::collections::BTreeMap<&'static str, i64>,
    #[serde(default)]
    pub relic_hold_sell_count: std::collections::BTreeMap<&'static str, u32>,
    #[serde(default)]
    pub relic_hold_sell_min: std::collections::BTreeMap<&'static str, i32>,
    #[serde(default)]
    pub relic_hold_sell_max: std::collections::BTreeMap<&'static str, i32>,
    #[serde(default)]
    pub relic_hold_sell_buckets:
        std::collections::BTreeMap<&'static str, std::collections::BTreeMap<&'static str, u32>>,
    /// Score points attributed to relic sources on committed plays.
    #[serde(default)]
    pub relic_score_points: std::collections::BTreeMap<&'static str, u64>,
    #[serde(default)]
    pub relic_score_chips: std::collections::BTreeMap<&'static str, u64>,
    #[serde(default)]
    pub relic_score_mult_pts: std::collections::BTreeMap<&'static str, u64>,
    #[serde(default)]
    pub relic_score_yen: std::collections::BTreeMap<&'static str, u64>,
    #[serde(default)]
    pub relic_score_triggers: std::collections::BTreeMap<&'static str, u32>,
    pub yen_spent: u32,
    pub yen_from_clears: u32,
    pub yen_from_clear_base: u32,
    pub yen_from_unused_plays: u32,
    pub yen_from_interest: u32,
    pub yen_from_clear_relics: u32,
    pub yen_from_temptations: u32,
    pub temptation_yen_value: u32,
    pub total_target_score: u64,
    pub total_overscore: u64,
    pub peak_chamber_score: u64,
    /// Filled when this run sets a new [`Self::peak_chamber_score`]: relics tray + per-play/structure points.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_chamber_detail: Option<PeakChamberSnapshot>,
    pub temptations_taken: std::collections::BTreeMap<&'static str, u32>,
    pub relics_picked: std::collections::BTreeMap<&'static str, u32>,
    /// Shop buys with `run.wing <= RELIC_SHOP_TIMING_EARLY_WING_MAX` at purchase.
    pub relics_picked_shop_early: std::collections::BTreeMap<&'static str, u32>,
    /// Shop buys with `run.wing > RELIC_SHOP_TIMING_EARLY_WING_MAX` at purchase.
    pub relics_picked_shop_late: std::collections::BTreeMap<&'static str, u32>,
    pub talismans_picked: std::collections::BTreeMap<&'static str, u32>,
    pub zodiacs_picked: std::collections::BTreeMap<&'static str, u32>,
    pub packs_picked: std::collections::BTreeMap<&'static str, u32>,
    pub bot_issue_no_valid_hand: u32,
    pub bot_issue_only_valid_unplayable: u32,
    pub bot_issue_only_valid_no_score: u32,
    pub bot_issue_other_stuck: u32,
    pub bot_issue_lost_with_available_lines: u32,
    pub bot_issues_by_reason: std::collections::BTreeMap<String, u32>,
    pub bot_issues_by_chamber: std::collections::BTreeMap<String, u32>,
    pub bot_issues_by_ordeal: std::collections::BTreeMap<String, u32>,
    pub overscore_by_slot: std::collections::BTreeMap<String, u64>,
    pub cleared_by_slot: std::collections::BTreeMap<String, u32>,
    pub ordeal_faced: std::collections::BTreeMap<String, u8>,
    pub ordeal_beaten: std::collections::BTreeMap<String, u8>,
    /// Sum of round scores on boss blinds, keyed by ante (clears and losses).
    #[serde(default)]
    pub ordeal_score_by_wing: std::collections::BTreeMap<u32, u64>,
    /// Boss blind attempts per ante.
    #[serde(default)]
    pub ordeal_attempts_by_wing: std::collections::BTreeMap<u32, u32>,
    /// Count of [`GameEvent::YakuScored`] per committed scoring action (one play may award several).
    pub yaku_scored: std::collections::BTreeMap<&'static str, u32>,
    /// Zodiac applications: dish consumes during blinds plus shop purchases
    /// (shop applies immediately).
    pub zodiacs_used: std::collections::BTreeMap<&'static str, u32>,
    /// Talisman uses from dish + any [`GameEvent::TalismanUsed`] seen on the scoring bus.
    pub talismans_used: std::collections::BTreeMap<&'static str, u32>,
    pub structure_triggers: u32,
    pub structure_trigger_points: u64,
    pub second_wind_forfeits: u32,
    /// Set when the run ends on [`RunState::round_failure_reason`]; absent for victory / odd losses.
    pub death_reason: Option<GameOverReason>,
    pub yen_clear_green_luck: u32,
    pub yen_clear_gold_idol: u32,
    pub yen_clear_jade_abacus: u32,
    pub yen_clear_patience: u32,
    pub turns_total: u32,
    pub turns_by_chamber_slot: std::collections::BTreeMap<String, u32>,
    /// Turn count for blinds that **cleared** only (denominator-friendly with `cleared_by_slot`).
    pub turns_cleared_by_slot: std::collections::BTreeMap<String, u32>,
    pub discards_by_chamber_slot: std::collections::BTreeMap<String, u32>,
    pub peak_hand_size: u32,
    pub relic_activations: std::collections::BTreeMap<&'static str, u32>,
    pub tiles_destroyed: u32,
    pub transformations_successor: std::collections::BTreeMap<&'static str, u32>,
    /// Active relic display names in tray order at run end.
    pub final_relics: Vec<String>,
    /// Remaining consumable labels at run end.
    pub final_consumables: Vec<String>,
    pub final_yaku_levels: YakuLevels,
    /// Set when the run stopped because `run_timeout` elapsed (after all retries exhausted, or the single attempt).
    #[serde(default)]
    pub run_timed_out: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_detail: Option<RunTimeoutSnapshot>,
}

impl RunStats {
    /// Kokushi Musō scored at least once this run (bot has no meta profile;
    /// mirrors the shop Qilin gate for simulation).
    pub fn kokushi_musou_scored(&self) -> bool {
        *self
            .yaku_scored
            .get(YakuKind::KokushiMusou.name())
            .unwrap_or(&0)
            > 0
    }
}

impl Default for RunStats {
    fn default() -> Self {
        Self {
            chambers_cleared: 0,
            wings_cleared: 0,
            victory: false,
            died_on_wing: 1,
            died_on_chamber: ChamberKind::Small,
            total_score: 0,
            plays_used: 0,
            discards_used: 0,
            strategic_discards: 0,
            final_yen: 0,
            chambers_skipped: 0,
            relics_bought: 0,
            relics_sold: 0,
            relics_sold_picked: std::collections::BTreeMap::new(),
            relic_shop_offers: std::collections::BTreeMap::new(),
            relic_marginal_buy_sum: std::collections::BTreeMap::new(),
            relic_marginal_buy_count: std::collections::BTreeMap::new(),
            relic_marginal_buy_min: std::collections::BTreeMap::new(),
            relic_marginal_buy_max: std::collections::BTreeMap::new(),
            relic_marginal_buy_buckets: std::collections::BTreeMap::new(),
            relic_hold_sell_sum: std::collections::BTreeMap::new(),
            relic_hold_sell_count: std::collections::BTreeMap::new(),
            relic_hold_sell_min: std::collections::BTreeMap::new(),
            relic_hold_sell_max: std::collections::BTreeMap::new(),
            relic_hold_sell_buckets: std::collections::BTreeMap::new(),
            relic_score_points: std::collections::BTreeMap::new(),
            relic_score_chips: std::collections::BTreeMap::new(),
            relic_score_mult_pts: std::collections::BTreeMap::new(),
            relic_score_yen: std::collections::BTreeMap::new(),
            relic_score_triggers: std::collections::BTreeMap::new(),
            yen_spent: 0,
            yen_from_clears: 0,
            yen_from_clear_base: 0,
            yen_from_unused_plays: 0,
            yen_from_interest: 0,
            yen_from_clear_relics: 0,
            yen_from_temptations: 0,
            temptation_yen_value: 0,
            total_target_score: 0,
            total_overscore: 0,
            peak_chamber_score: 0,
            peak_chamber_detail: None,
            temptations_taken: std::collections::BTreeMap::new(),
            relics_picked: std::collections::BTreeMap::new(),
            relics_picked_shop_early: std::collections::BTreeMap::new(),
            relics_picked_shop_late: std::collections::BTreeMap::new(),
            talismans_picked: std::collections::BTreeMap::new(),
            zodiacs_picked: std::collections::BTreeMap::new(),
            packs_picked: std::collections::BTreeMap::new(),
            bot_issue_no_valid_hand: 0,
            bot_issue_only_valid_unplayable: 0,
            bot_issue_only_valid_no_score: 0,
            bot_issue_other_stuck: 0,
            bot_issue_lost_with_available_lines: 0,
            bot_issues_by_reason: std::collections::BTreeMap::new(),
            bot_issues_by_chamber: std::collections::BTreeMap::new(),
            bot_issues_by_ordeal: std::collections::BTreeMap::new(),
            overscore_by_slot: std::collections::BTreeMap::new(),
            cleared_by_slot: std::collections::BTreeMap::new(),
            ordeal_faced: std::collections::BTreeMap::new(),
            ordeal_beaten: std::collections::BTreeMap::new(),
            ordeal_score_by_wing: std::collections::BTreeMap::new(),
            ordeal_attempts_by_wing: std::collections::BTreeMap::new(),
            yaku_scored: std::collections::BTreeMap::new(),
            zodiacs_used: std::collections::BTreeMap::new(),
            talismans_used: std::collections::BTreeMap::new(),
            structure_triggers: 0,
            structure_trigger_points: 0,
            second_wind_forfeits: 0,
            death_reason: None,
            yen_clear_green_luck: 0,
            yen_clear_gold_idol: 0,
            yen_clear_jade_abacus: 0,
            yen_clear_patience: 0,
            turns_total: 0,
            turns_by_chamber_slot: std::collections::BTreeMap::new(),
            turns_cleared_by_slot: std::collections::BTreeMap::new(),
            discards_by_chamber_slot: std::collections::BTreeMap::new(),
            peak_hand_size: 0,
            relic_activations: std::collections::BTreeMap::new(),
            tiles_destroyed: 0,
            transformations_successor: std::collections::BTreeMap::new(),
            final_relics: Vec::new(),
            final_consumables: Vec::new(),
            final_yaku_levels: YakuLevels::default(),
            run_timed_out: false,
            timeout_detail: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct AggregateStats {
    pub runs: u32,
    pub chambers_cleared_total: u64,
    pub wings_cleared_total: u64,
    pub victories: u32,
    pub max_wing_reached: u32,
    pub total_score: u64,
    pub total_plays: u64,
    pub total_discards: u64,
    pub total_strategic_discards: u64,
    pub total_chambers_skipped: u64,
    pub total_relics_bought: u64,
    pub total_relics_sold: u64,
    pub total_yen_spent: u64,
    pub total_final_yen: i64,
    pub total_yen_from_clears: u64,
    pub total_yen_from_clear_base: u64,
    pub total_yen_from_unused_plays: u64,
    pub total_yen_from_interest: u64,
    pub total_yen_from_clear_relics: u64,
    pub total_yen_from_temptations: u64,
    pub total_temptation_yen_value: u64,
    pub total_target_score: u64,
    pub total_overscore: u64,
    pub peak_chamber_score: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_chamber_detail: Option<PeakChamberSnapshot>,
    pub total_bot_issue_no_valid_hand: u64,
    pub total_bot_issue_only_valid_unplayable: u64,
    pub total_bot_issue_only_valid_no_score: u64,
    pub total_bot_issue_other_stuck: u64,
    pub total_bot_issue_lost_with_available_lines: u64,
    pub bot_issues_by_reason: std::collections::BTreeMap<String, u32>,
    pub deaths_by_wing: std::collections::BTreeMap<u32, u32>,
    pub deaths_by_chamber: std::collections::BTreeMap<&'static str, u32>,
    pub temptations_taken: std::collections::BTreeMap<&'static str, u32>,
    pub relics_picked: std::collections::BTreeMap<&'static str, u32>,
    pub relics_picked_victories: std::collections::BTreeMap<&'static str, u32>,
    pub relics_picked_shop_early: std::collections::BTreeMap<&'static str, u32>,
    pub relics_picked_shop_early_victories: std::collections::BTreeMap<&'static str, u32>,
    pub relics_picked_shop_late: std::collections::BTreeMap<&'static str, u32>,
    pub relics_picked_shop_late_victories: std::collections::BTreeMap<&'static str, u32>,
    pub talismans_picked: std::collections::BTreeMap<&'static str, u32>,
    pub zodiacs_picked: std::collections::BTreeMap<&'static str, u32>,
    pub packs_picked: std::collections::BTreeMap<&'static str, u32>,
    pub bot_issues_by_chamber: std::collections::BTreeMap<String, u32>,
    pub bot_issues_by_ordeal: std::collections::BTreeMap<String, u32>,
    pub overscore_by_slot: std::collections::BTreeMap<String, u64>,
    pub cleared_by_slot: std::collections::BTreeMap<String, u64>,
    pub ordeal_faced: std::collections::BTreeMap<String, u32>,
    pub ordeal_beaten: std::collections::BTreeMap<String, u32>,
    #[serde(default)]
    pub ordeal_score_by_wing: std::collections::BTreeMap<u32, u64>,
    #[serde(default)]
    pub ordeal_attempts_by_wing: std::collections::BTreeMap<u32, u32>,
    pub deaths_by_wing_cause: std::collections::BTreeMap<String, u32>,
    pub yaku_scored: std::collections::BTreeMap<&'static str, u64>,
    pub total_zodiacs_used: std::collections::BTreeMap<&'static str, u64>,
    pub total_talismans_used: std::collections::BTreeMap<&'static str, u64>,
    pub total_structure_triggers: u64,
    pub total_structure_trigger_points: u64,
    pub total_second_wind_forfeits: u64,
    pub deaths_out_of_plays: u32,
    pub deaths_no_actions_remaining: u32,
    pub total_yen_clear_green_luck: u64,
    pub total_yen_clear_gold_idol: u64,
    pub total_yen_clear_jade_abacus: u64,
    pub total_yen_clear_patience: u64,
    pub total_turns: u64,
    pub sum_peak_hand_size: u64,
    pub turns_by_chamber_slot: std::collections::BTreeMap<String, u64>,
    pub turns_cleared_by_slot: std::collections::BTreeMap<String, u64>,
    pub discards_by_chamber_slot: std::collections::BTreeMap<String, u64>,
    pub relic_activations: std::collections::BTreeMap<&'static str, u64>,
    pub total_tiles_destroyed: u64,
    pub transformations_successor: std::collections::BTreeMap<&'static str, u64>,
    /// Shop stock appearances (all offerings, not just buys).
    #[serde(default)]
    pub relic_shop_offers: std::collections::BTreeMap<&'static str, u64>,
    #[serde(default)]
    pub relic_marginal_buy_sum: std::collections::BTreeMap<&'static str, i64>,
    #[serde(default)]
    pub relic_marginal_buy_count: std::collections::BTreeMap<&'static str, u64>,
    #[serde(default)]
    pub relic_marginal_buy_min: std::collections::BTreeMap<&'static str, i32>,
    #[serde(default)]
    pub relic_marginal_buy_max: std::collections::BTreeMap<&'static str, i32>,
    #[serde(default)]
    pub relic_marginal_buy_bucket_totals: std::collections::BTreeMap<String, u64>,
    #[serde(default)]
    pub relic_hold_sell_sum: std::collections::BTreeMap<&'static str, i64>,
    #[serde(default)]
    pub relic_hold_sell_count: std::collections::BTreeMap<&'static str, u64>,
    #[serde(default)]
    pub relic_hold_sell_min: std::collections::BTreeMap<&'static str, i32>,
    #[serde(default)]
    pub relic_hold_sell_max: std::collections::BTreeMap<&'static str, i32>,
    #[serde(default)]
    pub relic_hold_sell_bucket_totals: std::collections::BTreeMap<String, u64>,
    /// Sum of `wings_cleared` for runs that bought this relic ≥1.
    #[serde(default)]
    pub relic_depth_with_antes_sum: std::collections::BTreeMap<&'static str, u64>,
    #[serde(default)]
    pub relic_depth_with_runs: std::collections::BTreeMap<&'static str, u64>,
    /// Sum of `wings_cleared` for runs that never bought this relic.
    #[serde(default)]
    pub relic_depth_without_antes_sum: std::collections::BTreeMap<&'static str, u64>,
    #[serde(default)]
    pub relic_depth_without_runs: std::collections::BTreeMap<&'static str, u64>,
    #[serde(default)]
    pub relic_score_points: std::collections::BTreeMap<&'static str, u64>,
    #[serde(default)]
    pub relic_score_chips: std::collections::BTreeMap<&'static str, u64>,
    #[serde(default)]
    pub relic_score_mult_pts: std::collections::BTreeMap<&'static str, u64>,
    #[serde(default)]
    pub relic_score_yen: std::collections::BTreeMap<&'static str, u64>,
    #[serde(default)]
    pub relic_score_triggers: std::collections::BTreeMap<&'static str, u64>,
    /// Runs that ended with `RunStats::run_timed_out` after all timeout retries.
    pub timed_out_runs: u32,
}

impl AggregateStats {
    pub(crate) fn record(&mut self, s: &RunStats) {
        self.runs += 1;
        if s.run_timed_out {
            self.timed_out_runs += 1;
        }
        self.chambers_cleared_total += s.chambers_cleared as u64;
        self.wings_cleared_total += s.wings_cleared as u64;
        if s.victory {
            self.victories += 1;
        }
        self.max_wing_reached = self.max_wing_reached.max(s.died_on_wing);
        self.total_score += s.total_score;
        self.total_plays += s.plays_used as u64;
        self.total_discards += s.discards_used as u64;
        self.total_strategic_discards += s.strategic_discards as u64;
        self.total_chambers_skipped += s.chambers_skipped as u64;
        self.total_relics_bought += s.relics_bought as u64;
        self.total_relics_sold += s.relics_sold as u64;
        self.total_yen_spent += s.yen_spent as u64;
        self.total_final_yen += s.final_yen as i64;
        self.total_yen_from_clears += s.yen_from_clears as u64;
        self.total_yen_from_clear_base += s.yen_from_clear_base as u64;
        self.total_yen_from_unused_plays += s.yen_from_unused_plays as u64;
        self.total_yen_from_interest += s.yen_from_interest as u64;
        self.total_yen_from_clear_relics += s.yen_from_clear_relics as u64;
        self.total_yen_from_temptations += s.yen_from_temptations as u64;
        self.total_temptation_yen_value += s.temptation_yen_value as u64;
        self.total_yen_clear_green_luck += s.yen_clear_green_luck as u64;
        self.total_yen_clear_gold_idol += s.yen_clear_gold_idol as u64;
        self.total_yen_clear_jade_abacus += s.yen_clear_jade_abacus as u64;
        self.total_yen_clear_patience += s.yen_clear_patience as u64;
        self.total_target_score += s.total_target_score;
        self.total_overscore += s.total_overscore;
        let prev_peak = self.peak_chamber_score;
        self.peak_chamber_score = prev_peak.max(s.peak_chamber_score);
        if s.peak_chamber_score == self.peak_chamber_score && self.peak_chamber_score > prev_peak {
            self.peak_chamber_detail = s.peak_chamber_detail.clone();
        }
        self.total_bot_issue_no_valid_hand += s.bot_issue_no_valid_hand as u64;
        self.total_bot_issue_only_valid_unplayable += s.bot_issue_only_valid_unplayable as u64;
        self.total_bot_issue_only_valid_no_score += s.bot_issue_only_valid_no_score as u64;
        self.total_bot_issue_other_stuck += s.bot_issue_other_stuck as u64;
        self.total_bot_issue_lost_with_available_lines +=
            s.bot_issue_lost_with_available_lines as u64;
        for (reason, count) in &s.bot_issues_by_reason {
            *self.bot_issues_by_reason.entry(reason.clone()).or_insert(0) += *count;
        }
        if !s.victory && !s.run_timed_out {
            *self.deaths_by_wing.entry(s.died_on_wing).or_insert(0) += 1;
            *self
                .deaths_by_chamber
                .entry(s.died_on_chamber.name())
                .or_insert(0) += 1;
            match s.death_reason {
                Some(GameOverReason::OutOfPlays) => self.deaths_out_of_plays += 1,
                Some(GameOverReason::NoActionsRemaining) => self.deaths_no_actions_remaining += 1,
                None => {}
            }
        }
        for (tag, count) in &s.temptations_taken {
            *self.temptations_taken.entry(tag).or_insert(0) += *count;
        }
        for (name, count) in &s.relics_picked {
            *self.relics_picked.entry(name).or_insert(0) += *count;
            if s.victory {
                *self.relics_picked_victories.entry(name).or_insert(0) += *count;
            }
        }
        for (name, count) in &s.relics_picked_shop_early {
            *self.relics_picked_shop_early.entry(name).or_insert(0) += *count;
            if s.victory {
                *self
                    .relics_picked_shop_early_victories
                    .entry(name)
                    .or_insert(0) += *count;
            }
        }
        for (name, count) in &s.relics_picked_shop_late {
            *self.relics_picked_shop_late.entry(name).or_insert(0) += *count;
            if s.victory {
                *self
                    .relics_picked_shop_late_victories
                    .entry(name)
                    .or_insert(0) += *count;
            }
        }
        for (name, count) in &s.talismans_picked {
            *self.talismans_picked.entry(name).or_insert(0) += *count;
        }
        for (name, count) in &s.zodiacs_picked {
            *self.zodiacs_picked.entry(name).or_insert(0) += *count;
        }
        for (name, count) in &s.packs_picked {
            *self.packs_picked.entry(name).or_insert(0) += *count;
        }
        for (blind, count) in &s.bot_issues_by_chamber {
            *self.bot_issues_by_chamber.entry(blind.clone()).or_insert(0) += *count;
        }
        for (boss, count) in &s.bot_issues_by_ordeal {
            *self.bot_issues_by_ordeal.entry(boss.clone()).or_insert(0) += *count;
        }
        for (slot, overscore) in &s.overscore_by_slot {
            *self.overscore_by_slot.entry(slot.clone()).or_insert(0) += *overscore;
        }
        for (slot, count) in &s.cleared_by_slot {
            *self.cleared_by_slot.entry(slot.clone()).or_insert(0) += *count as u64;
        }
        for boss in s.ordeal_faced.keys() {
            *self.ordeal_faced.entry(boss.clone()).or_insert(0) += 1;
        }
        for boss in s.ordeal_beaten.keys() {
            *self.ordeal_beaten.entry(boss.clone()).or_insert(0) += 1;
        }
        for (ante, score) in &s.ordeal_score_by_wing {
            *self.ordeal_score_by_wing.entry(*ante).or_insert(0) += score;
        }
        for (ante, attempts) in &s.ordeal_attempts_by_wing {
            *self.ordeal_attempts_by_wing.entry(*ante).or_insert(0) += attempts;
        }
        if !s.victory && !s.run_timed_out {
            let cause = classify_run_death_cause(s);
            let key = format!("{}|{}", s.died_on_wing, cause);
            *self.deaths_by_wing_cause.entry(key).or_insert(0) += 1;
        }
        self.total_structure_triggers += s.structure_triggers as u64;
        self.total_structure_trigger_points += s.structure_trigger_points;
        self.total_second_wind_forfeits += s.second_wind_forfeits as u64;
        self.total_turns += s.turns_total as u64;
        self.sum_peak_hand_size += s.peak_hand_size as u64;
        self.total_tiles_destroyed += s.tiles_destroyed as u64;
        for (name, count) in &s.zodiacs_used {
            *self.total_zodiacs_used.entry(name).or_insert(0) += *count as u64;
        }
        for (name, count) in &s.talismans_used {
            *self.total_talismans_used.entry(name).or_insert(0) += *count as u64;
        }
        for (slot, count) in &s.turns_by_chamber_slot {
            *self.turns_by_chamber_slot.entry(slot.clone()).or_insert(0) += *count as u64;
        }
        for (slot, count) in &s.turns_cleared_by_slot {
            *self.turns_cleared_by_slot.entry(slot.clone()).or_insert(0) += *count as u64;
        }
        for (slot, count) in &s.discards_by_chamber_slot {
            *self
                .discards_by_chamber_slot
                .entry(slot.clone())
                .or_insert(0) += *count as u64;
        }
        for (name, count) in &s.relic_activations {
            *self.relic_activations.entry(name).or_insert(0) += *count as u64;
        }
        for (name, count) in &s.transformations_successor {
            *self.transformations_successor.entry(name).or_insert(0) += *count as u64;
        }
        for (name, count) in &s.yaku_scored {
            *self.yaku_scored.entry(name).or_insert(0) += *count as u64;
        }
        super::relic_analytics::merge_run_relic_analytics(self, s);
    }

    pub fn to_bot_aggregate(&self) -> crate::bot::export_schema::BotAggregate {
        crate::bot::stats_derived::bot_aggregate_from(self)
    }

    pub fn to_derived(
        &self,
        yaku_kind_count: usize,
        base_target: u32,
        runs: Option<&[RunStats]>,
    ) -> crate::bot::export_schema::BotReportDerived {
        crate::bot::stats_derived::derived_from_batch(
            self,
            runs.unwrap_or(&[]),
            yaku_kind_count,
            base_target,
        )
    }

    pub fn write_summary_with_runs(
        &self,
        w: &mut dyn Write,
        runs: Option<&[RunStats]>,
    ) -> std::io::Result<()> {
        macro_rules! out {
            ($($t:tt)*) => {
                writeln!(w, $($t)*)?;
            };
        }

        out!("\n=== Bot Stats ({} runs) ===", self.runs);
        if self.runs == 0 {
            return Ok(());
        }
        let d = self.to_derived(
            YakuKind::all().len(),
            crate::core::season::Season::Spring.base_target(),
            runs,
        );
        let pr = &d.per_run;
        out!(
            "victories:           {} / {} ({:.1}%)",
            self.victories,
            self.runs,
            pr.win_rate_pct
        );
        if let Some(ref w) = d.overall_win_rate_wilson_95 {
            out!("  win rate 95% CI:   {:.1}% – {:.1}% (Wilson)", w.lo, w.hi);
        }
        if self.timed_out_runs > 0 {
            out!(
                "runs timed out:      {} (deadline hit after retries; see console dump if enabled)",
                self.timed_out_runs
            );
        }
        out!("avg blinds cleared:  {:.2}", pr.chambers_cleared);
        out!("avg antes cleared:   {:.2}", pr.wings_cleared);
        out!("max ante reached:    {}", self.max_wing_reached);
        out!(
            "avg total score:     {}",
            human_readable_score(pr.total_score)
        );
        out!("avg plays used:      {:.2}", pr.plays);
        out!(
            "avg discards used:   {:.2} ({:.2} strategic, {:.2} random)",
            pr.discards,
            pr.strategic_discards,
            pr.random_discards
        );
        out!("avg blinds skipped:  {:.2}", pr.chambers_skipped);
        out!(
            "avg relics bought:   {:.2} (avg gold spent: {:.1})",
            pr.relics_bought,
            pr.yen_spent
        );
        out!(
            "avg gold earned:     {:.1} clears + {:.1} temptations = {:.1}",
            pr.yen_from_clears,
            pr.yen_from_temptations,
            pr.yen_from_clears + pr.yen_from_temptations
        );
        out!(
            "avg clear payout:    {:.1} base + {:.1} plays + {:.1} interest + {:.1} relics",
            pr.yen_clear_base,
            pr.yen_clear_unused_plays,
            pr.yen_clear_interest,
            pr.yen_clear_relics
        );
        out!(
            "  (relic slice:      {:.1} green luck + {:.1} gold idol + {:.1} jade abacus + {:.1} patience)",
            pr.yen_clear_green_luck,
            pr.yen_clear_gold_idol,
            pr.yen_clear_jade_abacus,
            pr.yen_clear_patience
        );
        out!("avg second-wind forfeits: {:.2}", pr.second_wind_forfeits);
        out!(
            "avg structure cash-ins: {:.2} ({:.0} pts/run from structure)",
            pr.structure_triggers,
            pr.structure_trigger_points_per_run
        );
        out!(
            "avg turns / run:     {:.2} (avg peak hand size {:.2})",
            pr.turns,
            pr.peak_hand_size
        );
        out!(
            "tiles destroyed:     {:.2}/run | relic activations: {:.0}/run",
            pr.tiles_destroyed,
            pr.relic_activations
        );
        out!(
            "avg final yen:      {:.1} (avg temptation value taken: {:.1})",
            pr.final_yen,
            pr.temptation_yen_value
        );
        out!(
            "avg targets faced:   {} (score/target {:.2}x, avg overscore {}, peak blind score {})",
            human_readable_score(pr.target_score_faced),
            pr.score_to_target_ratio,
            human_readable_score(pr.overscore),
            human_readable_score(self.peak_chamber_score as f64)
        );

        if let Some(ref lb) = d.loss_breakdown {
            out!("\nloss — engine stop reason (round exhausted; excludes odd failures):");
            out!(
                "  out of plays: {:>4} ({:.1}% of losses) | no actions: {:>4} ({:.1}%)",
                lb.out_of_plays,
                lb.out_of_plays_pct_of_losses,
                lb.no_actions_remaining,
                lb.no_actions_pct_of_losses
            );
        }

        out!("\ndeaths by ante (Wilson 95% CI on % of all runs):");
        for row in &d.deaths_by_wing {
            let bar = "#".repeat(row.text_bar_hashes as usize);
            out!(
                "  ante {:>2}: {:>4} ({:>5.1}% [{:.1}–{:.1}]) {}",
                row.wing,
                row.count,
                row.pct_of_runs,
                row.pct_ci_lo,
                row.pct_ci_hi,
                bar
            );
        }

        if !d.deaths_by_wing_hazard.is_empty() {
            out!("\ndeath hazard P(die on ante | reached ante), Wilson 95% CI:");
            out!(
                "  {:>4} {:>8} {:>8} {:>8} {:>14}",
                "ante",
                "reached",
                "deaths",
                "hazard",
                "95% CI"
            );
            for row in &d.deaths_by_wing_hazard {
                out!(
                    "  {:>4} {:>8} {:>8} {:>7.1}% [{:>5.1}–{:>5.1}]",
                    row.wing,
                    row.reached,
                    row.deaths,
                    row.hazard_pct,
                    row.hazard_ci_lo,
                    row.hazard_ci_hi
                );
            }
        }

        out!("\ndeaths by blind:");
        for row in &d.deaths_by_chamber {
            out!(
                "  {:<12} {:>4} ({:>5.1}%)",
                row.name,
                row.count,
                row.pct_of_runs
            );
        }

        if !d.surplus_by_slot.is_empty() {
            out!("\navg score surplus per blind (log-scaled bars):");
            for row in &d.surplus_by_slot {
                let bar_len = ((row.bar_pct / 100.0) * 40.0).round() as usize;
                let bar = "#".repeat(bar_len.min(40));
                out!(
                    "  {:<18} (n={:>5}) {:>10}  {}",
                    row.slot,
                    row.clears,
                    human_readable_score(row.avg_surplus),
                    bar
                );
            }
        }

        if !d.avg_turns_to_clear.is_empty() {
            out!("\navg turns to clear (cleared blinds only):");
            for row in &d.avg_turns_to_clear {
                out!("  {:<18} {:>6.2}", row.slot, row.avg_turns);
            }
        }

        if !d.temptations.is_empty() {
            out!("\ntemptations taken:");
            for row in &d.temptations {
                out!(
                    "  {:<16} {:>4} ({:>5.1}%)",
                    row.name,
                    row.count,
                    row.pct_of_runs
                );
            }
        }

        if !d.consumable_zodiacs.is_empty() || !d.consumable_talismans.is_empty() {
            out!("\nconsumables used (in-round + scoring events, not shop buys):");
            if !d.consumable_zodiacs.is_empty() {
                let line: String = d
                    .consumable_zodiacs
                    .iter()
                    .take(12)
                    .map(|x| format!("{} x{}", x.name, x.count))
                    .collect::<Vec<_>>()
                    .join(", ");
                out!("  zodiacs: {line}");
            }
            if !d.consumable_talismans.is_empty() {
                let line: String = d
                    .consumable_talismans
                    .iter()
                    .take(12)
                    .map(|x| format!("{} x{}", x.name, x.count))
                    .collect::<Vec<_>>()
                    .join(", ");
                out!("  talismans: {line}");
            }
        }

        if !d.transformations_top.is_empty() {
            out!("\ntransformation successors discovered:");
            for row in d.transformations_top.iter().take(16) {
                out!("  {:<24} {:>5}", row.name, row.count);
            }
        }

        if !d.relics_bought.is_empty() {
            const MIN_SAMPLES_FOR_WIN_CORR: u32 = 20;
            let overall_win_rate = pr.win_rate_pct;
            let ante_mx = d.relic_shop_timing_early_wing_max;
            out!(
                "\nrelics bought (sorted by total purchases; Wilson 95% CI on win% when bought ≥ 1):"
            );
            out!(
                "  %late = share of buys when run.wing > {} at the shop visit.",
                ante_mx
            );
            out!(
                "  {:<22} {:>7} {:>7} {:>8} {:>16} {:>7} {:>8}",
                "relic",
                "bought",
                "won",
                "win%",
                "95% CI",
                "Δbase",
                "%late"
            );
            for row in &d.relics_bought {
                let late_s = row
                    .pct_shop_late
                    .map(|p| format!("{:.0}%", p))
                    .unwrap_or_else(|| "—".into());
                out!(
                    "  {:<22} {:>7} {:>7} {:>7.1}% [{:>5.1}–{:>5.1}] {:>+7.1} {:>8}",
                    row.name,
                    row.bought,
                    row.won,
                    row.win_pct,
                    row.win_pct_ci_lo,
                    row.win_pct_ci_hi,
                    row.delta_vs_baseline_pct,
                    late_s
                );
            }
            if !d.relics_by_win_rate.is_empty() {
                out!(
                    "\nrelics by win-rate (≥{} samples, baseline {:.1}%):",
                    MIN_SAMPLES_FOR_WIN_CORR,
                    overall_win_rate
                );
                out!(
                    "  {:<22} {:>7} {:>7} {:>8} {:>16} {:>8} {:>8}",
                    "relic",
                    "bought",
                    "won",
                    "win%",
                    "95% CI",
                    "Δ",
                    "%late"
                );
                for row in &d.relics_by_win_rate {
                    let late_s = row
                        .pct_shop_late
                        .map(|p| format!("{:.0}%", p))
                        .unwrap_or_else(|| "—".into());
                    out!(
                        "  {:<22} {:>7} {:>7} {:>7.1}% [{:>5.1}–{:>5.1}] {:>+7.1} {:>8}",
                        row.name,
                        row.bought,
                        row.won,
                        row.win_pct,
                        row.win_pct_ci_lo,
                        row.win_pct_ci_hi,
                        row.delta_vs_baseline_pct,
                        late_s
                    );
                }
            }
            if !d.relics_shop_timing_split.is_empty() {
                out!(
                    "\nrelic shop timing (≥{} buys in each bucket; early = ante≤{}, late = ante>{}):",
                    MIN_SHOP_TIMING_SPLIT_PER_BUCKET,
                    ante_mx,
                    ante_mx
                );
                out!(
                    "  {:<22} {:>5} {:>5} {:>8} {:>5} {:>5} {:>8} {:>8}",
                    "relic",
                    "eN",
                    "eW",
                    "eWin%",
                    "lN",
                    "lW",
                    "lWin%",
                    "late−early"
                );
                for row in &d.relics_shop_timing_split {
                    out!(
                        "  {:<22} {:>5} {:>5} {:>7.1}% {:>5} {:>5} {:>7.1}% {:>+7.1}",
                        row.name,
                        row.early_bought,
                        row.early_won,
                        row.early_win_pct,
                        row.late_bought,
                        row.late_won,
                        row.late_win_pct,
                        row.timing_gap_pct
                    );
                }
            }
            if !d.relics_shop_funnel.is_empty() {
                out!(
                    "\nrelic shop funnel (offers vs buys; low take% + high offers → undervalued):"
                );
                out!(
                    "  {:<22} {:>7} {:>7} {:>7} {:>10}",
                    "relic",
                    "offers",
                    "bought",
                    "take%",
                    "avg_marg"
                );
                for row in d.relics_shop_funnel.iter().take(20) {
                    out!(
                        "  {:<22} {:>7} {:>7} {:>6.1}% {:>10.0}",
                        row.name,
                        row.offers,
                        row.bought,
                        row.take_rate_pct,
                        row.avg_marginal_at_buy
                    );
                }
            }
            if !d.relics_depth_split.is_empty() {
                out!("\nrelic depth split (avg antes: runs that bought vs never bought):");
                out!(
                    "  {:<22} {:>7} {:>7} {:>7} {:>7} {:>7}",
                    "relic",
                    "runs+",
                    "antes+",
                    "runs−",
                    "antes−",
                    "Δ"
                );
                let mut depth: Vec<_> = d.relics_depth_split.iter().collect();
                depth.sort_by(|a, b| {
                    b.delta_antes
                        .partial_cmp(&a.delta_antes)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                for row in depth.iter().filter(|r| r.runs_with > 0).take(15) {
                    out!(
                        "  {:<22} {:>7} {:>7.2} {:>7} {:>7.2} {:>+7.2}",
                        row.name,
                        row.runs_with,
                        row.avg_antes_with,
                        row.runs_without,
                        row.avg_antes_without,
                        row.delta_antes
                    );
                }
            }
            if !d.relics_score_attribution.is_empty() {
                out!("\nrelic score attribution (committed plays, top by score pts):");
                out!(
                    "  {:<22} {:>8} {:>10} {:>10} {:>10}",
                    "relic",
                    "triggers",
                    "score",
                    "chips",
                    "mult"
                );
                for row in d.relics_score_attribution.iter().take(15) {
                    out!(
                        "  {:<22} {:>8} {:>10} {:>10} {:>10}",
                        row.name,
                        row.triggers,
                        row.score_pts,
                        row.chips_pts,
                        row.mult_pts
                    );
                }
            }
        }

        if !d.talismans_shop.is_empty() {
            out!("\ntalismans bought:");
            for row in &d.talismans_shop {
                out!(
                    "  {:<22} {:>5} ({:.2}/run)",
                    row.name,
                    row.count,
                    row.per_run
                );
            }
        }

        if !d.zodiacs_shop.is_empty() {
            out!("\nzodiacs acquired:");
            for row in &d.zodiacs_shop {
                out!(
                    "  {:<22} {:>5} ({:.2}/run)",
                    row.name,
                    row.count,
                    row.per_run
                );
            }
        }
        if !d.zodiac_balance.is_empty() {
            out!(
                "\nzodiac balance (acquire/use/win telemetry; win% is conditioned on runs that acquired):"
            );
            out!(
                "  {:<14} {:<18} {:>6} {:>6} {:>7} {:>8} {:>18} {:>8}",
                "zodiac",
                "yaku",
                "acq",
                "used",
                "use/acq",
                "Δwin",
                "95% CI",
                "avgLvl"
            );
            for row in d.zodiac_balance.iter().filter(|r| r.acquired > 0) {
                out!(
                    "  {:<14} {:<18} {:>6} {:>6} {:>7.2} {:>+7.1} [{:>4.1},{:>4.1}] {:>8.2}",
                    row.zodiac,
                    row.yaku,
                    row.acquired,
                    row.used,
                    row.use_per_acquire,
                    row.delta_vs_baseline_pct,
                    row.win_pct_ci_lo,
                    row.win_pct_ci_hi,
                    row.avg_primary_yaku_level_when_acquired
                );
            }
        }
        if !d.zodiac_balance_alarms.is_empty() {
            out!("\nzodiac balance alarms:");
            for alarm in &d.zodiac_balance_alarms {
                out!(
                    "  [{}] {:<14} {:<20} observed={:>6.2} threshold={:>6.2}  {}",
                    alarm.severity,
                    alarm.zodiac,
                    alarm.signal,
                    alarm.observed,
                    alarm.threshold,
                    alarm.detail
                );
            }
        }

        if let Some(ref y) = d.yaku {
            out!("\nyaku scored (each committed play may count multiple patterns):");
            out!(
                "  total awards: {} ({:.2}/run); uniform baseline {:.1}% per kind ({} kinds)",
                y.total_awards,
                y.awards_per_run,
                y.uniform_baseline_pct,
                y.kind_count
            );
            out!(
                "  {:<22} {:>8} {:>8} {:>8} {:>8}",
                "yaku",
                "awards",
                "/run",
                "share%",
                "Δuni%"
            );
            for row in &y.rows {
                out!(
                    "  {:<22} {:>8} {:>8.2} {:>7.1}% {:>+7.1}%",
                    row.name,
                    row.awards,
                    row.per_run,
                    row.share_pct,
                    row.delta_uniform_pct
                );
            }
            if !y.never_awarded.is_empty() {
                out!(
                    "\n  no awards (underused: bot never triggered these, {} kinds):",
                    y.never_awarded.len()
                );
                for name in &y.never_awarded {
                    out!("    {}", name);
                }
            }
        }

        if !d.packs_shop.is_empty() {
            out!("\npacks bought:");
            for row in &d.packs_shop {
                out!(
                    "  {:<22} {:>5} ({:.2}/run)",
                    row.name,
                    row.count,
                    row.per_run
                );
            }
        }

        if let Some(ref bi) = d.bot_issues {
            out!("\nbot terminal issues:");
            if bi.dead_end_total > 0 {
                out!("  dead-ends:");
                out!(
                    "  total {:>6} | no-valid {} | only-valid-unplayable {} | only-valid-no-score {} | other {}",
                    bi.dead_end_total,
                    bi.no_valid,
                    bi.only_unplayable,
                    bi.only_no_score,
                    bi.other
                );
            }
            if bi.lost_with_lines > 0 {
                out!("  lost with lines available: {}", bi.lost_with_lines);
            }
            if !bi.hottest_chambers.is_empty() {
                let s = bi
                    .hottest_chambers
                    .iter()
                    .map(|(a, b)| format!("{a} x{b}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                out!("  hottest blinds: {s}");
            }
            if !bi.hottest_ordeals.is_empty() {
                let s = bi
                    .hottest_ordeals
                    .iter()
                    .map(|(a, b)| format!("{a} x{b}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                out!("  hottest bosses: {s}");
            }
            if !bi.reasons_top.is_empty() {
                let s = bi
                    .reasons_top
                    .iter()
                    .map(|(a, b)| format!("{a} x{b}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                out!("  reasons: {s}");
            }
        }

        Ok(())
    }

    pub fn write_summary(&self, w: &mut dyn Write) -> std::io::Result<()> {
        self.write_summary_with_runs(w, None)
    }

    pub fn print_summary_with_runs(&self, runs: Option<&[RunStats]>) {
        let _ = self.write_summary_with_runs(&mut std::io::stdout().lock(), runs);
    }

    pub fn print_summary(&self) {
        self.print_summary_with_runs(None);
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ClearPayoutBreakdown {
    pub base_reward: u32,
    pub unused_play_bonus: u32,
    pub interest: u32,
    pub green_luck_bonus: u32,
    pub gold_idol_bonus: u32,
    pub jade_abacus_bonus: u32,
    pub patience_bonus: u32,
    pub relic_bonus: u32,
    pub total: u32,
}

pub(crate) fn clear_payout_breakdown(run: &RunState) -> ClearPayoutBreakdown {
    let base_reward = run.chamber.clear_reward();
    let unused_play_bonus = run.plays_remaining;
    let interest = (run.yen.max(0) as u32 / 5).min(3);
    let green_luck_bonus = if run.relics.has(RelicId::GreenLuck) && !run.honors_scored_this_round {
        4
    } else {
        0
    };
    let gold_idol_bonus = if run.relics.has(RelicId::GoldIdol) {
        3
    } else {
        0
    };
    let jade_abacus_bonus = if run.relics.has(RelicId::JadeAbacus) {
        (run.yen.max(0) as u32 / 4).min(4)
    } else {
        0
    };
    let patience_bonus = if run.relics.has(RelicId::Patience) {
        2 * run.discards_remaining
    } else {
        0
    };
    let relic_bonus = green_luck_bonus + gold_idol_bonus + jade_abacus_bonus + patience_bonus;
    let total = base_reward + unused_play_bonus + interest + relic_bonus;
    ClearPayoutBreakdown {
        base_reward,
        unused_play_bonus,
        interest,
        green_luck_bonus,
        gold_idol_bonus,
        jade_abacus_bonus,
        patience_bonus,
        relic_bonus,
        total,
    }
}
