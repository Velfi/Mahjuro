//! Headless bot runner used for tuning balance.
//!
//! The bot picks the highest-scoring valid play available in its current hand each turn
//! (same validation path as real commits: structure-commit rules + wildcard relic resolution).
//! Candidate plays are meld-built subsets plus every 14-tile Kokushi Musō orphan combination
//! (the meld enumerator never emits twelve singletons + one pair).
//! Between turns it strategically discards isolated tiles via 1-step rollout when the
//! best play falls below the pace needed to clear. Between blinds it values relics and
//! consumables, visits the shop, buys the most useful affordable upgrade (selling a
//! weaker owned relic first when inventory is full), and skips
//! Small/Big blinds when its expected score comfortably exceeds the target.
//!
//! Run with: `cargo run --release -- bot 200`

use crate::core::OrdealKindExt;
use rand::RngExt;
use rand::seq::SliceRandom;
use rayon::prelude::*;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::core::consumable::Consumable;
use crate::core::deck::Wall;
use crate::core::hand::detect_all_sets;
use crate::core::relic::{
    RelicId, RelicState, ScoreContext, ScoreEconomyBundle, ScorePatternBundle, ScoreRelicBundle,
    ScoreRoundBundle, ScoreTileBundle, all_relic_defs, apply_merchants_eye_discount,
    golden_engine_han_bonus, relic_sell_price_live, relic_shop_price,
};
use crate::core::rules::{ChamberKind, RuleModifier};
use crate::core::scoring::{
    ScoreBreakdown, StepKind, format_meld_groups, score_sets_with_original,
};
use crate::core::structure::StructureTriggerMeta;
use crate::core::talisman::TalismanKind;
use crate::core::tile::{Suit, Tile};
use crate::core::tile_pack::{TilePackInstance, TilePackKind};
use crate::core::zodiac::{YakuLevels, ZodiacKind};
use crate::game::event_bus::GameOverReason;
use crate::game::event_bus::{EventBus, GameEvent};
use crate::game::game_mode::{GameMode, HAND_SIZE};
use crate::game::run::{
    FINAL_WING, KIOSK_RELIC_SLOTS, RunState, enumerate_candidate_play_masks,
    relic_eligible_for_shop_stock, roll_shop_offer_counts,
};

mod chamber_planner;
mod chamber_sim;
mod export_schema;
mod relic_analytics;
mod relic_order;
mod reporting;
mod stats;
mod stats_derived;
mod stats_wilson;

pub use reporting::{
    BotConfig, BotOutputFormat, BotOutputTarget, BotRunOptions, BotStrategy, BotTimeoutDiag,
    DEFAULT_BOT_RUN_TIMEOUT_SECS, HeadlessBotBatch, StrategyFile, append_bot_run_to_progress,
    default_strategies_file, export_play_history_html, read_strategies_file,
    run_forced_relic_sweep, run_headless, run_headless_aggregate, run_strategy_sweep, run_sweep,
    seed_progress_from_bot_runs, strategy_config_by_name,
};
use stats::clear_payout_breakdown;
pub use stats::{AggregateStats, RunStats, RunTimeoutSnapshot};
use stats::{BotScoreSourceEntry, BotScoringAction, PeakChamberRelicState, PeakChamberSnapshot};

fn relic_display_name(id: RelicId) -> &'static str {
    all_relic_defs()
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.name)
        .unwrap_or("?")
}

fn acquire_relic(run: &mut RunState, id: RelicId, strategy: &BotStrategy) {
    run.grant_relic(id);
    if strategy.relic_order_optimization_enabled() {
        relic_order::bot_optimize_relic_order(run);
    }
}

/// Headless analogue of `ShopCommand::SellRelic`.
fn bot_sell_relic(run: &mut RunState, index: usize, bus: Option<&mut EventBus>) -> Option<u32> {
    if index >= run.relics.active.len() {
        return None;
    }
    let rid = run.relics.active[index];
    let refund = relic_sell_price_live(rid, &run.relic_counters);
    run.relics.active.remove(index);
    run.clear_relic_run_metadata(rid);
    if !run.relics.has(RelicId::IGotAGuy) {
        run.relic_counters.remove(&RelicId::IGotAGuy);
    }
    run.apply_yen_reward(refund as i32, bus);
    *run.relic_counters.entry(RelicId::Bonfire).or_insert(0) += 1;
    if run.relics.has(RelicId::Bonfire) {
        run.relic_activations.push(RelicId::Bonfire);
    }
    run.recompute_capacities();
    Some(refund)
}

#[macro_export]
macro_rules! bot_log {
    ($enabled:expr, $($arg:tt)*) => {
        if $enabled {
            println!($($arg)*);
        }
    };
}

fn chamber_slot_key(run: &RunState) -> String {
    format!("{:02}-{}", run.wing, run.chamber.name())
}

fn chamber_log_label(run: &RunState, blind: ChamberKind) -> String {
    match blind {
        ChamberKind::Ordeal => {
            let ordeal_name = run
                .ordeal
                .upcoming
                .map(|ordeal| ordeal.name())
                .unwrap_or("Unknown Ordeal");
            format!("{} ({})", blind.name(), ordeal_name)
        }
        _ => blind.name().to_string(),
    }
}

fn run_deadline_expired(deadline: Option<Instant>) -> bool {
    deadline.is_some_and(|d| Instant::now() >= d)
}

fn record_timeout_snapshot(
    stats: &mut RunStats,
    run: &RunState,
    phase: &str,
    active_chamber: ChamberKind,
    chamber_turn: Option<u32>,
    started: Instant,
) {
    stats.run_timed_out = true;
    stats.victory = false;
    stats.died_on_wing = run.wing;
    stats.died_on_chamber = active_chamber;
    stats.death_reason = None;
    stats.timeout_detail = Some(stats::RunTimeoutSnapshot {
        phase: phase.to_string(),
        wing: run.wing,
        chamber: chamber_log_label(run, active_chamber),
        chamber_turn,
        round_score: run.round_score,
        target_score: run.target_score,
        plays_remaining: run.plays_remaining,
        discards_remaining: run.discards_remaining,
        elapsed_ms: started.elapsed().as_millis() as u64,
    });
}

fn fmt_indices(indices: &[usize]) -> impl fmt::Display + '_ {
    struct DisplayIndices<'a>(&'a [usize]);

    impl fmt::Display for DisplayIndices<'_> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "[")?;
            for (i, idx) in self.0.iter().enumerate() {
                if i > 0 {
                    write!(f, ", ")?;
                }
                write!(f, "{idx}")?;
            }
            write!(f, "]")
        }
    }

    DisplayIndices(indices)
}

fn current_ordeal_name(run: &RunState) -> Option<&'static str> {
    run.ordeal.upcoming.map(|boss| boss.name())
}

#[derive(Debug, Default)]
struct HandOptionAnalysis {
    valid_count: u32,
    committable_count: u32,
    positive_score_count: u32,
}

#[derive(Clone, Copy, Debug)]
enum TerminalIssueCause {
    OutOfPlays,
    NoActionsRemaining,
    RejectedChosenPlay,
    NoDiscardsRemaining,
    EmptyHand,
}

impl TerminalIssueCause {
    fn label(self) -> &'static str {
        match self {
            Self::OutOfPlays => "out-of-plays",
            Self::NoActionsRemaining => "no-actions-remaining",
            Self::RejectedChosenPlay => "rejected-chosen-play",
            Self::NoDiscardsRemaining => "no-discards-remaining",
            Self::EmptyHand => "empty-hand",
        }
    }
}

fn analyze_hand_options(run: &RunState, hand: &[Tile]) -> HandOptionAnalysis {
    let n = hand.len();
    if !(2..=20).contains(&n) {
        return HandOptionAnalysis::default();
    }

    let mut analysis = HandOptionAnalysis::default();
    let mut ctx = bot_score_context_base(run, &run.relics, None, None);
    let base_set_len = run.structure_sets().len();
    let base_tile_len = run.structure_tiles().len();
    let mut merged_sets = run.structure_sets().to_vec();
    let mut merged_tiles = run.structure_tiles().to_vec();
    let commit_rules = run.validation_rules_for_structure_commits();
    for mask in enumerate_candidate_play_masks(hand, &commit_rules) {
        let tiles = tiles_from_play_mask(hand, n, mask);
        let Some((sets, scoring_tiles)) = run.try_validate_with_wildcards(&tiles) else {
            continue;
        };

        analysis.valid_count += 1;

        if !structure_commit_fits(run, scoring_tiles.len(), &sets) {
            continue;
        }
        analysis.committable_count += 1;

        merged_sets.truncate(base_set_len);
        merged_sets.extend(sets.iter().cloned());
        merged_tiles.truncate(base_tile_len);
        merged_tiles.extend(scoring_tiles.iter().copied());
        ctx.structure = Some(StructureTriggerMeta {
            meld_count: merged_sets.len() as u32,
            inject_chicken_if_no_yaku: true,
        });
        let breakdown =
            score_sets_with_original(&merged_tiles, &merged_sets, &ctx, &run.round_rules, &tiles);
        if breakdown.total > 0 {
            analysis.positive_score_count += 1;
        }
    }

    analysis
}

fn record_terminal_hand_issue(stats: &mut RunStats, run: &RunState, cause: TerminalIssueCause) {
    *stats
        .bot_issues_by_chamber
        .entry(chamber_log_label(run, run.chamber))
        .or_insert(0) += 1;
    if let Some(ordeal_name) = current_ordeal_name(run) {
        *stats
            .bot_issues_by_ordeal
            .entry(ordeal_name.to_string())
            .or_insert(0) += 1;
    }

    let analysis = analyze_hand_options(run, run.hand());
    let issue_key = if analysis.valid_count == 0 {
        "no-valid".to_string()
    } else if analysis.valid_count == 1 && analysis.committable_count == 0 {
        "only-valid-unplayable".to_string()
    } else if analysis.valid_count == 1 && analysis.positive_score_count == 0 {
        "only-valid-no-score".to_string()
    } else if analysis.committable_count == 0 {
        format!("other:{}:all-valid-blocked", cause.label())
    } else if analysis.positive_score_count == 0 {
        format!("other:{}:all-committable-zero-score", cause.label())
    } else {
        format!("other:{}:playable-lines-remained", cause.label())
    };
    *stats.bot_issues_by_reason.entry(issue_key).or_insert(0) += 1;

    if analysis.valid_count == 0 {
        stats.bot_issue_no_valid_hand += 1;
        return;
    }

    if analysis.valid_count == 1 && analysis.committable_count == 0 {
        stats.bot_issue_only_valid_unplayable += 1;
        return;
    }

    if analysis.valid_count == 1 && analysis.positive_score_count == 0 {
        stats.bot_issue_only_valid_no_score += 1;
        return;
    }

    if analysis.committable_count > 0 && analysis.positive_score_count > 0 {
        stats.bot_issue_lost_with_available_lines += 1;
    } else {
        stats.bot_issue_other_stuck += 1;
    }
}

/// Score context shared across all candidate masks for one `evaluate_play_masks` / analysis pass.
/// Only [`ScoreContext::structure`] changes per mask (meld count for structure-depth relics).
fn bot_score_context_base<'a>(
    run: &'a RunState,
    relics: &'a RelicState,
    yaku_levels_override: Option<&YakuLevels>,
    counters_override: Option<&BTreeMap<RelicId, i32>>,
) -> ScoreContext<'a> {
    let plays_rem_after = run.plays_remaining.saturating_sub(1);
    let plays_used_after = run.plays_max.saturating_sub(plays_rem_after);
    ScoreContext {
        relic: ScoreRelicBundle {
            roster: relics,
            counters: counters_override
                .cloned()
                .unwrap_or_else(|| run.relic_counters.clone()),
        },
        tiles: ScoreTileBundle {
            debuffs: &run.tile_debuffs,
            hand_for_ghost: run.hand(),
        },
        round: ScoreRoundBundle {
            scored_last_turn: run.scored_last_turn,
            plays_used: plays_used_after,
            round_wind: Some(ChamberKind::round_wind_for_wing(run.wing)),
            bonus_round_wind: run.bonus_round_wind_for_yaku(),
            played_yaku_this_round: run.played_yaku_this_round.clone(),
            is_final_play: plays_rem_after == 0,
        },
        pattern: ScorePatternBundle {
            dora_faces: run.wall.dora_faces(),
            available_yaku: run.available_yaku.clone(),
            yaku_levels: Some(
                yaku_levels_override
                    .cloned()
                    .unwrap_or_else(|| run.yaku_levels.clone()),
            ),
        },
        economy: ScoreEconomyBundle {
            yen: run.yen,
            total_score: run.total_score_earned,
        },
        structure: None,
    }
}

fn tiles_from_play_mask(hand: &[Tile], hand_len: usize, mask: u32) -> Vec<Tile> {
    let count = mask.count_ones() as usize;
    let mut tiles = Vec::with_capacity(count);
    for (i, &tile) in hand.iter().enumerate().take(hand_len) {
        if mask & (1 << i) != 0 {
            tiles.push(tile);
        }
    }
    tiles
}

fn indices_from_play_mask(hand_len: usize, mask: u32) -> Vec<usize> {
    (0..hand_len).filter(|i| mask & (1 << i) != 0).collect()
}

/// Whether a commit of `new_sets` with `scoring_tile_count` tiles fits in structure.
fn structure_commit_fits(
    run: &RunState,
    scoring_tile_count: usize,
    new_sets: &[crate::core::hand::DetectedMeld],
) -> bool {
    use crate::core::hand::kong_structure_bonus;
    let kongs_after = kong_structure_bonus(run.structure_sets().iter().chain(new_sets.iter()));
    run.structure_tiles().len() + scoring_tile_count <= HAND_SIZE + kongs_after
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PlayRank {
    score: u64,
    meld_count: usize,
    tile_count: usize,
}

/// Convert immediate yen (Gilded, flowers, etc.) into shop-comparable score units.
/// Uses the upcoming blind's chip target vs its flat clear payout as the exchange rate.
fn shop_payoff_units(run: &RunState, score: u64, yen: i32) -> i64 {
    let score_part = score as i64;
    if yen <= 0 {
        return score_part;
    }
    let target = run.target_score.max(1) as i64;
    let clear_yen = run.chamber.clear_reward().max(1) as i64;
    score_part + yen as i64 * target / clear_yen
}

/// Best play from an explicit candidate mask list (used by [`best_play_in_hand`] and benches).
/// Masks must be enumerated for **structure commits** (see [`RunState::validation_rules_for_structure_commits`]).
fn evaluate_play_masks_payoff(
    run: &RunState,
    hand: &[Tile],
    relics_override: Option<&RelicState>,
    yaku_levels_override: Option<&YakuLevels>,
    counters_override: Option<&BTreeMap<RelicId, i32>>,
    masks: &[u32],
) -> Option<(u64, i32, Vec<usize>)> {
    let n = hand.len();
    if !(2..=20).contains(&n) {
        return None;
    }
    let relics = relics_override.unwrap_or(&run.relics);
    let mut ctx = bot_score_context_base(run, relics, yaku_levels_override, counters_override);
    let base_set_len = run.structure_sets().len();
    let base_tile_len = run.structure_tiles().len();
    let mut merged_sets = run.structure_sets().to_vec();
    let mut merged_tiles = run.structure_tiles().to_vec();
    let mut best: Option<(PlayRank, i32, Vec<usize>)> = None;
    for &mask in masks {
        let tiles = tiles_from_play_mask(hand, n, mask);
        let Some((sets, scoring_tiles)) = run.try_validate_with_wildcards(&tiles) else {
            continue;
        };
        if !structure_commit_fits(run, scoring_tiles.len(), &sets) {
            continue;
        }
        merged_sets.truncate(base_set_len);
        merged_sets.extend(sets.iter().cloned());
        merged_tiles.truncate(base_tile_len);
        merged_tiles.extend(scoring_tiles.iter().copied());
        ctx.structure = Some(StructureTriggerMeta {
            meld_count: merged_sets.len() as u32,
            inject_chicken_if_no_yaku: true,
        });
        let breakdown =
            score_sets_with_original(&merged_tiles, &merged_sets, &ctx, &run.round_rules, &tiles);
        if breakdown.total == 0 && breakdown.flower_yen <= 0 {
            continue;
        }
        let rank = PlayRank {
            score: breakdown.total,
            meld_count: sets.len(),
            tile_count: scoring_tiles.len(),
        };
        let indices = indices_from_play_mask(n, mask);
        let yen = breakdown.flower_yen;
        if best
            .as_ref()
            .map(|(best_rank, _, _)| rank > *best_rank)
            .unwrap_or(true)
        {
            best = Some((rank, yen, indices));
        }
    }
    best.map(|(rank, yen, indices)| (rank.score, yen, indices))
}

pub(crate) fn evaluate_play_masks(
    run: &RunState,
    hand: &[Tile],
    relics_override: Option<&RelicState>,
    yaku_levels_override: Option<&YakuLevels>,
    masks: &[u32],
) -> Option<(u64, Vec<usize>)> {
    evaluate_play_masks_payoff(
        run,
        hand,
        relics_override,
        yaku_levels_override,
        None,
        masks,
    )
    .map(|(score, _, indices)| (score, indices))
}

/// Top-scoring play commits in one mask-evaluation pass (shared score context).
pub(crate) fn top_k_plays_in_hand(
    run: &RunState,
    hand: &[Tile],
    k: usize,
) -> Vec<(u64, Vec<usize>)> {
    let n = hand.len();
    if !(2..=20).contains(&n) || k == 0 {
        return Vec::new();
    }
    let commit_rules = run.validation_rules_for_structure_commits();
    let masks = enumerate_candidate_play_masks(hand, &commit_rules);
    let relics = &run.relics;
    let mut ctx = bot_score_context_base(run, relics, None, None);
    let base_set_len = run.structure_sets().len();
    let base_tile_len = run.structure_tiles().len();
    let mut merged_sets = run.structure_sets().to_vec();
    let mut merged_tiles = run.structure_tiles().to_vec();
    let mut top: Vec<(PlayRank, Vec<usize>)> = Vec::with_capacity(k.min(masks.len()));

    for &mask in &masks {
        let tiles = tiles_from_play_mask(hand, n, mask);
        let Some((sets, scoring_tiles)) = run.try_validate_with_wildcards(&tiles) else {
            continue;
        };
        if !structure_commit_fits(run, scoring_tiles.len(), &sets) {
            continue;
        }
        merged_sets.truncate(base_set_len);
        merged_sets.extend(sets.iter().cloned());
        merged_tiles.truncate(base_tile_len);
        merged_tiles.extend(scoring_tiles.iter().copied());
        ctx.structure = Some(StructureTriggerMeta {
            meld_count: merged_sets.len() as u32,
            inject_chicken_if_no_yaku: true,
        });
        let breakdown =
            score_sets_with_original(&merged_tiles, &merged_sets, &ctx, &run.round_rules, &tiles);
        if breakdown.total == 0 && breakdown.flower_yen <= 0 {
            continue;
        }
        let rank = PlayRank {
            score: breakdown.total,
            meld_count: sets.len(),
            tile_count: scoring_tiles.len(),
        };
        let indices = indices_from_play_mask(n, mask);
        if top.len() < k {
            top.push((rank, indices));
            top.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        } else if rank > top.last().expect("top non-empty").0 {
            top.pop();
            top.push((rank, indices));
            top.sort_unstable_by(|a, b| b.0.cmp(&a.0));
        }
    }

    top.sort_unstable_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    top.dedup_by(|a, b| a.1 == b.1);
    top.truncate(k);
    top.into_iter()
        .map(|(rank, indices)| (rank.score, indices))
        .collect()
}

/// Full score breakdown for a specific hand subset (committed-play attribution).
fn score_breakdown_for_play_indices(
    run: &RunState,
    hand: &[Tile],
    indices: &[usize],
) -> Option<ScoreBreakdown> {
    let n = hand.len();
    if indices.is_empty() || n == 0 {
        return None;
    }
    let mut mask = 0u32;
    for &i in indices {
        if i >= n {
            return None;
        }
        mask |= 1 << i;
    }
    let tiles = tiles_from_play_mask(hand, n, mask);
    let (sets, scoring_tiles) = run.try_validate_with_wildcards(&tiles)?;
    if !structure_commit_fits(run, scoring_tiles.len(), &sets) {
        return None;
    }
    let mut ctx = bot_score_context_base(run, &run.relics, None, None);
    let mut merged_sets = run.structure_sets().to_vec();
    let mut merged_tiles = run.structure_tiles().to_vec();
    merged_sets.extend(sets.iter().cloned());
    merged_tiles.extend(scoring_tiles.iter().copied());
    ctx.structure = Some(StructureTriggerMeta {
        meld_count: merged_sets.len() as u32,
        inject_chicken_if_no_yaku: true,
    });
    Some(score_sets_with_original(
        &merged_tiles,
        &merged_sets,
        &ctx,
        &run.round_rules,
        &tiles,
    ))
}

fn source_rows_from_points(
    points_by_source: std::collections::BTreeMap<String, u64>,
) -> Vec<BotScoreSourceEntry> {
    let mut rows: Vec<BotScoreSourceEntry> = points_by_source
        .into_iter()
        .map(|(name, points)| BotScoreSourceEntry { name, points })
        .collect();
    rows.sort_by(|a, b| b.points.cmp(&a.points).then_with(|| a.name.cmp(&b.name)));
    rows
}

fn is_yaku_or_dora_source(
    source: &str,
    yaku_names: &std::collections::BTreeSet<&'static str>,
) -> bool {
    source.starts_with("Dora ") || yaku_names.contains(source)
}

pub(crate) fn build_bot_scoring_action(
    kind: &str,
    points: u64,
    tiles: Option<String>,
    breakdown: Option<&ScoreBreakdown>,
    yaku_levels_before: &YakuLevels,
    yen_before: i32,
    golden_engine_active: bool,
) -> BotScoringAction {
    let mut action = BotScoringAction {
        kind: kind.to_string(),
        points,
        tiles,
        tiles_points: None,
        yaku_points: None,
        relic_points: None,
        other_points: None,
        yen_held: Some(yen_before),
        golden_engine_han_bonus: golden_engine_active
            .then_some(golden_engine_han_bonus(yen_before)),
        yaku: Vec::new(),
        yaku_steps: Vec::new(),
        relic_steps: Vec::new(),
    };
    let Some(breakdown) = breakdown else {
        return action;
    };

    let tile_points = breakdown
        .base_steps
        .last()
        .map(|s| s.running_total)
        .unwrap_or(0);
    let yaku_names: std::collections::BTreeSet<&'static str> =
        breakdown.detected_yaku.iter().map(|y| y.name()).collect();

    let mut yaku_points = 0u64;
    let mut relic_points = 0u64;
    let mut other_points = 0u64;
    let mut yaku_step_points: std::collections::BTreeMap<String, u64> =
        std::collections::BTreeMap::new();
    let mut relic_step_points: std::collections::BTreeMap<String, u64> =
        std::collections::BTreeMap::new();

    let mut prev_total = tile_points;
    for step in &breakdown.steps {
        let delta = step.running_total.saturating_sub(prev_total);
        prev_total = step.running_total;
        if matches!(step.kind, StepKind::Final | StepKind::Yen) {
            continue;
        }
        if let Some(relic_name) = relic_analytics::match_step_source_to_relic(&step.source) {
            relic_points += delta;
            *relic_step_points.entry(relic_name.to_string()).or_insert(0) += delta;
        } else if is_yaku_or_dora_source(&step.source, &yaku_names) {
            yaku_points += delta;
            *yaku_step_points.entry(step.source.clone()).or_insert(0) += delta;
        } else {
            other_points += delta;
        }
    }

    action.tiles_points = Some(tile_points);
    action.yaku_points = Some(yaku_points);
    action.relic_points = Some(relic_points);
    action.other_points = Some(other_points);
    action.yaku = breakdown
        .detected_yaku
        .iter()
        .map(|&yk| format!("{} Lv{}", yk.name(), yaku_levels_before.level_of(yk)))
        .collect();
    action.yaku_steps = source_rows_from_points(yaku_step_points);
    action.relic_steps = source_rows_from_points(relic_step_points);
    action
}

fn peak_chamber_relic_state(run: &RunState) -> Vec<PeakChamberRelicState> {
    run.relics
        .active
        .iter()
        .enumerate()
        .map(|(idx, &id)| PeakChamberRelicState {
            slot: idx.saturating_add(1) as u32,
            name: relic_display_name(id).to_string(),
            counter: run.relic_counters.get(&id).copied(),
            debuffed: run.relics.is_debuffed(id),
        })
        .collect()
}

/// Masks that pass meld validation and structure checks (still may score to zero).
fn masks_passing_validate_and_structure(run: &RunState, hand: &[Tile], masks: &[u32]) -> usize {
    let n = hand.len();
    if !(2..=20).contains(&n) {
        return 0;
    }
    let mut hits = 0usize;
    for &mask in masks {
        let tiles = tiles_from_play_mask(hand, n, mask);
        let Some((sets, scoring_tiles)) = run.try_validate_with_wildcards(&tiles) else {
            continue;
        };
        if structure_commit_fits(run, scoring_tiles.len(), &sets) {
            hits += 1;
        }
    }
    hits
}

/// Masks that reach a positive score after full relic/yaku evaluation.
fn masks_with_positive_score(
    run: &RunState,
    hand: &[Tile],
    masks: &[u32],
    relics_override: Option<&RelicState>,
    yaku_levels_override: Option<&YakuLevels>,
) -> usize {
    let n = hand.len();
    if !(2..=20).contains(&n) {
        return 0;
    }
    let relics = relics_override.unwrap_or(&run.relics);
    let mut ctx = bot_score_context_base(run, relics, yaku_levels_override, None);
    let base_set_len = run.structure_sets().len();
    let base_tile_len = run.structure_tiles().len();
    let mut merged_sets = run.structure_sets().to_vec();
    let mut merged_tiles = run.structure_tiles().to_vec();
    let mut hits = 0usize;
    for &mask in masks {
        let tiles = tiles_from_play_mask(hand, n, mask);
        let Some((sets, scoring_tiles)) = run.try_validate_with_wildcards(&tiles) else {
            continue;
        };
        if !structure_commit_fits(run, scoring_tiles.len(), &sets) {
            continue;
        }
        merged_sets.truncate(base_set_len);
        merged_sets.extend(sets.iter().cloned());
        merged_tiles.truncate(base_tile_len);
        merged_tiles.extend(scoring_tiles.iter().copied());
        ctx.structure = Some(StructureTriggerMeta {
            meld_count: merged_sets.len() as u32,
            inject_chicken_if_no_yaku: true,
        });
        let breakdown =
            score_sets_with_original(&merged_tiles, &merged_sets, &ctx, &run.round_rules, &tiles);
        if breakdown.total > 0 {
            hits += 1;
        }
    }
    hits
}

/// Find the best (score, indices) commit from `hand`, merging into the current structure.
/// `relics_override` / `yaku_levels_override` are for bot-side what-if evaluation.
fn best_play_in_hand(
    run: &RunState,
    hand: &[Tile],
    relics_override: Option<&RelicState>,
    yaku_levels_override: Option<&YakuLevels>,
    counters_override: Option<&BTreeMap<RelicId, i32>>,
) -> Option<(u64, Vec<usize>)> {
    let commit_rules = run.validation_rules_for_structure_commits();
    let masks = enumerate_candidate_play_masks(hand, &commit_rules);
    evaluate_play_masks_payoff(
        run,
        hand,
        relics_override,
        yaku_levels_override,
        counters_override,
        &masks,
    )
    .map(|(score, _, indices)| (score, indices))
}

/// Search for the highest-scoring playable selection in the current hand.
/// Returns `(score, indices)`, or `None` if no positive-scoring play exists.
pub fn pick_best_play(run: &RunState) -> Option<(u64, Vec<usize>)> {
    best_play_in_hand(run, run.hand(), None, None, None)
}

/// Demo [`RunState`] with `tiles` as the current hand (sorted). For benches/tests only.
#[doc(hidden)]
pub fn bench_fixture_run(mut tiles: Vec<Tile>) -> RunState {
    let mut run = RunState::new_demo();
    tiles.sort();
    *run.hand_mut() = tiles;
    run
}

/// Candidate play bitmasks for `hand` under `rules` (Criterion: pair with [`bench_evaluate_play_masks`]).
/// For gameplay-accurate enumeration, pass [`RunState::validation_rules_for_structure_commits`].
#[doc(hidden)]
pub fn bench_enumerate_play_masks(hand: &[Tile], rules: &[RuleModifier]) -> Vec<u32> {
    enumerate_candidate_play_masks(hand, rules)
}

/// Full scoring pass over precomputed masks (enumeration excluded).
#[doc(hidden)]
pub fn bench_evaluate_play_masks(
    run: &RunState,
    hand: &[Tile],
    masks: &[u32],
) -> Option<(u64, Vec<usize>)> {
    evaluate_play_masks(run, hand, None, None, masks)
}

/// Count masks that pass validation + structure checks (before scoring).
#[doc(hidden)]
pub fn bench_count_masks_validate_structure(run: &RunState, hand: &[Tile], masks: &[u32]) -> usize {
    masks_passing_validate_and_structure(run, hand, masks)
}

/// Count masks that yield a positive score (includes full scoring work).
#[doc(hidden)]
pub fn bench_count_masks_positive_score(run: &RunState, hand: &[Tile], masks: &[u32]) -> usize {
    masks_with_positive_score(run, hand, masks, None, None)
}

/// Rate each tile by how many *potential* melds in the current hand it participates in.
/// A tile that appears in zero detected sets is "isolated" and a prime discard target.
/// Returns a vector parallel to `hand` containing usage counts.
fn tile_meld_participation(hand: &[Tile]) -> Vec<u32> {
    let sets = detect_all_sets(hand);
    let mut counts = vec![0u32; hand.len()];
    for s in &sets {
        for id in &s.tile_ids {
            if let Some(idx) = hand.iter().position(|t| t.id == *id) {
                counts[idx] += 1;
            }
        }
    }
    // Orphan terminals/honors often sit outside standard meld detection until the hand is
    // nearly Kokushi — bias discards toward non-orphans when many distinct orphans are present.
    let mut orphan_faces: rustc_hash::FxHashSet<(Suit, u8)> = rustc_hash::FxHashSet::default();
    for t in hand {
        if !t.is_flower() && t.is_kokushi_orphan() {
            orphan_faces.insert((t.suit, t.rank));
        }
    }
    let d = orphan_faces.len() as u32;
    if d >= 8 {
        let bonus = d.saturating_sub(4).max(4);
        for (i, t) in hand.iter().enumerate() {
            if !t.is_flower() && t.is_kokushi_orphan() {
                counts[i] = counts[i].saturating_add(bonus);
            }
        }
    }
    counts
}

/// Generate up to `max_k` discard candidates ordered by tile participation: candidate K
/// drops the K lowest-participation tiles. Tiles already pulling weight in detected
/// melds are dumped last so we never voluntarily throw away a built partial.
fn discard_candidates(hand: &[Tile], max_k: usize) -> Vec<Vec<usize>> {
    if hand.len() < 3 {
        return Vec::new();
    }
    let counts = tile_meld_participation(hand);
    let mut indexed: Vec<(usize, u32)> = counts.into_iter().enumerate().collect();
    indexed.sort_by_key(|(_, c)| *c);
    let order: Vec<usize> = indexed.into_iter().map(|(i, _)| i).collect();
    let cap = max_k.min(hand.len() - 2);
    (1..=cap)
        .map(|k| order.iter().take(k).copied().collect())
        .collect()
}

/// Simulate discarding `discard_indices` from the hand and drawing replacements off the
/// top of the wall (peeked, not consumed). Returns the best playable score that the
/// resulting hand could produce. Uses 1-step lookahead with the actual upcoming tiles —
/// "perfect-information" oracle, which gives us a tuning ceiling rather than a
/// realistic player bot.
fn rollout_post_discard_score(run: &RunState, discard_indices: &[usize]) -> u64 {
    let drop_set: rustc_hash::FxHashSet<usize> = discard_indices.iter().copied().collect();
    let k = discard_indices.len();
    let peeked = run.wall.peek_next(k);
    let mut new_hand: Vec<Tile> = run
        .hand()
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop_set.contains(i))
        .map(|(_, t)| *t)
        .collect();
    new_hand.extend_from_slice(peeked);
    new_hand.sort();
    best_play_in_hand(run, &new_hand, None, None, None)
        .map(|(s, _)| s)
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlayChamberOutcome {
    Cleared,
    LostRun,
    SecondWindForfeit,
    TimedOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShopVisitOutcome {
    Completed,
    TimedOut,
}

/// Drains scoring / refill events. A `RoundComplete` with `reached_target: false` is Second
/// Wind: apply the forfeit transition immediately (matches deferred UI handling).
fn drain_post_action_bus(
    run: &mut RunState,
    bus: &mut EventBus,
    stats: &mut RunStats,
) -> Option<PlayChamberOutcome> {
    let events: Vec<GameEvent> = bus.drain().collect();
    for ev in events {
        match ev {
            GameEvent::RoundComplete {
                payout,
                reached_target: true,
            } => {
                run.apply_yen_reward(payout.total as i32, Some(bus));
            }
            GameEvent::RoundComplete {
                reached_target: false,
                ..
            } => {
                run.forfeit_current_chamber_second_wind(bus);
                return Some(PlayChamberOutcome::SecondWindForfeit);
            }
            GameEvent::YakuScored(yk) => {
                *stats.yaku_scored.entry(yk.name()).or_insert(0) += 1;
            }
            GameEvent::RelicActivated(id) => {
                *stats
                    .relic_activations
                    .entry(relic_display_name(id))
                    .or_insert(0) += 1;
            }
            GameEvent::TalismanUsed(kind) => {
                *stats.talismans_used.entry(kind.name()).or_insert(0) += 1;
            }
            GameEvent::TilesDestroyed => {
                stats.tiles_destroyed += 1;
            }
            GameEvent::TransformationSuccessorDiscovered(id) => {
                let rname = relic_display_name(id);
                *stats.transformations_successor.entry(rname).or_insert(0) += 1;
                *stats.relics_picked.entry(rname).or_insert(0) += 1;
            }
            _ => {}
        }
    }
    None
}

/// Play the current blind to completion. Returns outcome and the number of **decision
/// turns** taken this blind (incremented once per loop iteration after failure checks).
fn play_chamber(
    run: &mut RunState,
    stats: &mut RunStats,
    log: bool,
    deadline: Option<Instant>,
    scoring_log: &mut Vec<BotScoringAction>,
    strategy: &BotStrategy,
) -> (PlayChamberOutcome, u32) {
    let mut bus = EventBus::default();
    let mut rng = rand::rng();
    let mut turn = 0u32;

    loop {
        if run.round_score >= run.target_score as u64 {
            bot_log!(
                log,
                "    blind cleared after {} turns with score {}/{}",
                turn,
                run.round_score,
                run.target_score
            );
            return (PlayChamberOutcome::Cleared, turn);
        }
        if let Some(reason) = run.round_failure_reason() {
            stats.death_reason = Some(reason);
            let cause = match reason {
                GameOverReason::OutOfPlays => TerminalIssueCause::OutOfPlays,
                GameOverReason::NoActionsRemaining => TerminalIssueCause::NoActionsRemaining,
            };
            record_terminal_hand_issue(stats, run, cause);
            bot_log!(
                log,
                "    blind failed after {} turns with score {}/{}",
                turn,
                run.round_score,
                run.target_score
            );
            return (PlayChamberOutcome::LostRun, turn);
        }
        if run_deadline_expired(deadline) {
            return (PlayChamberOutcome::TimedOut, turn);
        }
        turn += 1;
        let slot = chamber_slot_key(run);
        *stats.turns_by_chamber_slot.entry(slot).or_insert(0) += 1;
        stats.turns_total += 1;
        stats.peak_hand_size = stats.peak_hand_size.max(run.hand().len() as u32);

        bot_log!(
            log,
            "    turn {:>2}: score {}/{} | plays {} | discards {} | hand {} | yen {}",
            turn,
            run.round_score,
            run.target_score,
            run.plays_remaining,
            run.discards_remaining,
            run.hand().len(),
            run.yen
        );

        if strategy.chamber_planner_depth == 0 && use_bot_consumables(run, stats, log) {
            continue;
        }

        if strategy.chamber_planner_depth >= 1 {
            match chamber_planner::execute_planned_turn(
                run,
                stats,
                log,
                strategy.chamber_planner_depth,
                scoring_log,
                &mut bus,
            ) {
                Some(chamber_planner::PlannedTurnOutcome::Continued) => continue,
                Some(chamber_planner::PlannedTurnOutcome::SecondWindForfeit) => {
                    return (PlayChamberOutcome::SecondWindForfeit, turn);
                }
                Some(chamber_planner::PlannedTurnOutcome::Failed) | None => {}
            }
        }

        let best = pick_best_play(run);
        let best_score = best.as_ref().map(|(s, _)| *s).unwrap_or(0);

        // Structure: cash in when the current structure scores at least as much as the best
        // commit preview (saves a play), or when there is no positive commit but the structure can score.
        let trigger_preview = if run.can_trigger_structure_now() {
            run.preview_manual_trigger_total()
        } else {
            0
        };
        if trigger_preview > 0 && (best_score == 0 || trigger_preview >= best_score) {
            let score_before_structure = run.round_score;
            let yen_before = run.yen;
            let yaku_levels_before = run.yaku_levels.clone();
            let golden_engine_active = run.relics.has(RelicId::GoldenEngine);
            let cash_in_tiles = run.structure_tiles().to_vec();
            let cash_in_sets = run.structure_sets().to_vec();
            let earned = run.trigger_structure_manual(&mut bus);
            stats.structure_triggers += 1;
            stats.structure_trigger_points += earned;
            if earned > 0 {
                bot_log!(
                    log,
                    "      action: trigger structure for {} (best play {})",
                    earned,
                    best_score
                );
            }
            if drain_post_action_bus(run, &mut bus, stats)
                == Some(PlayChamberOutcome::SecondWindForfeit)
            {
                return (PlayChamberOutcome::SecondWindForfeit, turn);
            }
            let structure_delta = run.round_score.saturating_sub(score_before_structure);
            if structure_delta > 0 {
                scoring_log.push(build_bot_scoring_action(
                    "structure",
                    structure_delta,
                    format_meld_groups(&cash_in_tiles, &cash_in_sets),
                    run.last_breakdown.as_ref(),
                    &yaku_levels_before,
                    yen_before,
                    golden_engine_active,
                ));
            }
            if earned > 0 {
                continue;
            }
        }

        // Strategic discard via 1-step rollout: try several candidate discard subsets,
        // peek the actual upcoming wall tiles, evaluate the best play in each
        // hypothetical hand, and take the discard whose post-rollout best play beats
        // the current best by a meaningful margin. The margin requirement prevents the
        // bot from burning a discard for a marginal +1 swing.
        let can_discard = run.discards_remaining > 0 && run.plays_remaining > 1;
        let mut did_discard = false;
        if can_discard && !strategy.oracle_discards_enabled() {
            let need = (run.target_score as u64).saturating_sub(run.round_score);
            let cant_clear_pace =
                best_score == 0 || best_score.saturating_mul(run.plays_remaining as u64) < need;
            if cant_clear_pace {
                if let Some(indices) = discard_candidates(run.hand(), 1).into_iter().next() {
                    bot_log!(
                        log,
                        "      action: heuristic discard {} (no wall peek)",
                        fmt_indices(&indices),
                    );
                    run.clear_selection();
                    for i in &indices {
                        run.toggle_select(*i);
                    }
                    run.discard_selected(&mut bus);
                    stats.discards_used += 1;
                    stats.strategic_discards += 1;
                    *stats
                        .discards_by_chamber_slot
                        .entry(chamber_slot_key(run))
                        .or_insert(0) += 1;
                    if drain_post_action_bus(run, &mut bus, stats)
                        == Some(PlayChamberOutcome::SecondWindForfeit)
                    {
                        return (PlayChamberOutcome::SecondWindForfeit, turn);
                    }
                    did_discard = true;
                }
            }
        } else if can_discard {
            let candidates = discard_candidates(run.hand(), 5);
            // Margin scales with how far we are from target — late in the round we
            // need bigger swings to be worth losing a play.
            let need = (run.target_score as u64).saturating_sub(run.round_score);
            let margin = (need / (run.plays_remaining as u64 + 1)).max(5);
            let mut best_after: Option<(u64, Vec<usize>)> = None;
            for cand in candidates {
                let hyp = rollout_post_discard_score(run, &cand);
                if best_after.as_ref().map(|(s, _)| hyp > *s).unwrap_or(true) {
                    best_after = Some((hyp, cand));
                }
            }
            if let Some((after_score, indices)) = best_after
                && after_score >= best_score + margin
            {
                bot_log!(
                    log,
                    "      action: strategic discard {} -> projected {} (best {} + margin {})",
                    fmt_indices(&indices),
                    after_score,
                    best_score,
                    margin
                );
                run.clear_selection();
                for i in &indices {
                    run.toggle_select(*i);
                }
                run.discard_selected(&mut bus);
                stats.discards_used += 1;
                stats.strategic_discards += 1;
                *stats
                    .discards_by_chamber_slot
                    .entry(chamber_slot_key(run))
                    .or_insert(0) += 1;
                if drain_post_action_bus(run, &mut bus, stats)
                    == Some(PlayChamberOutcome::SecondWindForfeit)
                {
                    return (PlayChamberOutcome::SecondWindForfeit, turn);
                }
                did_discard = true;
            }
        }
        if did_discard {
            continue;
        }

        if let Some((_, indices)) = best {
            bot_log!(
                log,
                "      action: play {} for {} points",
                fmt_indices(&indices),
                best_score
            );
            let hand_before = run.hand().to_vec();
            if let Some(breakdown) = score_breakdown_for_play_indices(run, &hand_before, &indices) {
                relic_analytics::record_score_breakdown(stats, &breakdown);
            }
            let yen_before = run.yen;
            let yaku_levels_before = run.yaku_levels.clone();
            let golden_engine_active = run.relics.has(RelicId::GoldenEngine);
            let structure_before_commit = run.structure_tiles().to_vec();
            let mut idx_sorted: Vec<usize> = indices.to_vec();
            idx_sorted.sort_unstable();
            let selected_tiles: Vec<Tile> = idx_sorted
                .iter()
                .filter_map(|&i| hand_before.get(i).copied())
                .collect();
            let commit_melds = run.try_validate_with_wildcards(&selected_tiles);
            run.clear_selection();
            for i in &indices {
                run.toggle_select(*i);
            }
            let plays_before = run.plays_remaining;
            let score_before = run.round_score;
            let committed = run.commit_selection_to_structure(&mut bus);
            if committed == 0
                && run.plays_remaining == plays_before
                && run.round_score == score_before
            {
                bot_log!(
                    log,
                    "      action: rejected play {} (state unchanged)",
                    fmt_indices(&indices)
                );
                run.clear_selection();
                for _ in bus.drain() {}
                record_terminal_hand_issue(stats, run, TerminalIssueCause::RejectedChosenPlay);
                return (PlayChamberOutcome::LostRun, turn);
            }
            stats.plays_used += 1;
            if drain_post_action_bus(run, &mut bus, stats)
                == Some(PlayChamberOutcome::SecondWindForfeit)
            {
                return (PlayChamberOutcome::SecondWindForfeit, turn);
            }
            let play_delta = run.round_score.saturating_sub(score_before);
            if play_delta > 0 {
                let tiles = if run.structure_tiles().is_empty() {
                    let mut full = structure_before_commit;
                    if let Some((_, scoring_tiles)) = &commit_melds {
                        full.extend(scoring_tiles.iter().copied());
                    } else {
                        full.extend(selected_tiles.iter().copied());
                    }
                    run.try_validate_with_wildcards(&full)
                        .and_then(|(sets, _)| format_meld_groups(&full, &sets))
                } else {
                    commit_melds
                        .and_then(|(sets, scoring_tiles)| format_meld_groups(&scoring_tiles, &sets))
                };
                scoring_log.push(build_bot_scoring_action(
                    "play",
                    play_delta,
                    tiles,
                    run.last_breakdown.as_ref(),
                    &yaku_levels_before,
                    yen_before,
                    golden_engine_active,
                ));
            }
            continue;
        }

        // No positive-scoring play and no strategic discard helped — random discard
        // as a last-resort shake-up before busting.
        if run.discards_remaining == 0 {
            record_terminal_hand_issue(stats, run, TerminalIssueCause::NoDiscardsRemaining);
            bot_log!(log, "      action: no discards remaining");
            return (PlayChamberOutcome::LostRun, turn);
        }
        run.clear_selection();
        let hand_n = run.hand().len();
        if hand_n == 0 {
            record_terminal_hand_issue(stats, run, TerminalIssueCause::EmptyHand);
            bot_log!(log, "      action: hand empty, cannot continue");
            return (PlayChamberOutcome::LostRun, turn);
        }
        let drop_n = rng.random_range(1..=hand_n.min(5));
        let mut indices: Vec<usize> = (0..hand_n).collect();
        indices.shuffle(&mut rng);
        let chosen: Vec<usize> = indices.iter().copied().take(drop_n).collect();
        bot_log!(
            log,
            "      action: fallback discard {}",
            fmt_indices(&chosen)
        );
        for i in indices.into_iter().take(drop_n) {
            run.toggle_select(i);
        }
        run.discard_selected(&mut bus);
        stats.discards_used += 1;
        *stats
            .discards_by_chamber_slot
            .entry(chamber_slot_key(run))
            .or_insert(0) += 1;
        if drain_post_action_bus(run, &mut bus, stats)
            == Some(PlayChamberOutcome::SecondWindForfeit)
        {
            return (PlayChamberOutcome::SecondWindForfeit, turn);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BotStrategy, EventBus, RunStats, ShopMarginalBase, best_play_in_hand,
        enumerate_candidate_play_masks, pick_best_play, relic_hold_value_with_base,
        relic_marginal_value_with_base, relic_shop_offer_value_with_base,
        remaining_wings_including_current, scale_long_term_value_for_wing,
        sell_underperforming_relics, talisman_marginal_value, use_bot_consumables,
        zodiac_marginal_value_with_base,
    };
    use crate::core::consumable::Consumable;
    use crate::core::hand::{DetectedMeld, MeldKind};
    use crate::core::relic::RelicId;
    use crate::core::talisman::TalismanKind;
    use crate::core::tile::{Suit, Tile};
    use crate::core::zodiac::ZodiacKind;
    use crate::game::game_mode::HAND_SIZE;
    use crate::game::run::{FINAL_WING, RunState};

    fn brute_force_best_play_in_hand(run: &RunState) -> Option<(u64, Vec<usize>)> {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        struct PlayRank {
            score: u64,
            meld_count: usize,
            tile_count: usize,
        }

        let n = run.hand().len();
        let mut best: Option<(PlayRank, Vec<usize>)> = None;
        let limit: u32 = 1u32 << n;
        for mask in 1u32..limit {
            let count = mask.count_ones() as usize;
            // `try_validate_with_wildcards` accepts several sizes (e.g. 3 = one meld,
            // 4 = kong or two flower pairs, 6 = two melds). Do not skip by count —
            // an old `4`-tile exclusion caused brute force to disagree with
            // `enumerate_candidate_play_masks` / `best_play_in_hand`.
            if matches!(count, 0 | 1) {
                continue;
            }
            let mut tiles = Vec::with_capacity(count);
            for i in 0..n {
                if mask & (1 << i) != 0 {
                    tiles.push(run.hand()[i]);
                }
            }
            let Some((sets, scoring_tiles)) = run.try_validate_with_wildcards(&tiles) else {
                continue;
            };
            let kongs_after = crate::core::hand::kong_structure_bonus(
                run.structure_sets().iter().chain(sets.iter()),
            );
            if run.structure_tiles().len() + scoring_tiles.len() > HAND_SIZE + kongs_after {
                continue;
            }
            let mut merged_sets = run.structure_sets().to_vec();
            merged_sets.extend(sets.iter().cloned());
            let mut merged_tiles = run.structure_tiles().to_vec();
            merged_tiles.extend(scoring_tiles.iter().copied());
            let mut ctx = super::bot_score_context_base(run, &run.relics, None, None);
            ctx.structure = Some(super::StructureTriggerMeta {
                meld_count: merged_sets.len() as u32,
                inject_chicken_if_no_yaku: true,
            });
            let total = crate::core::scoring::score_sets_with_original(
                &merged_tiles,
                &merged_sets,
                &ctx,
                &run.round_rules,
                &tiles,
            )
            .total;
            if total == 0 {
                continue;
            }
            let rank = PlayRank {
                score: total,
                meld_count: sets.len(),
                tile_count: scoring_tiles.len(),
            };
            let indices: Vec<usize> = (0..n).filter(|i| mask & (1 << i) != 0).collect();
            if best
                .as_ref()
                .map(|(best_rank, _)| rank > *best_rank)
                .unwrap_or(true)
            {
                best = Some((rank, indices));
            }
        }
        best.map(|(rank, indices)| (rank.score, indices))
    }

    fn t(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    fn scoring_test_run() -> RunState {
        let mut run = RunState::new_demo();
        *run.hand_mut() = vec![
            t(Suit::Manzu, 3, 1),
            t(Suit::Manzu, 3, 2),
            t(Suit::Manzu, 3, 3),
            t(Suit::Souzu, 6, 4),
            t(Suit::Souzu, 6, 5),
            t(Suit::Souzu, 6, 6),
            t(Suit::Pinzu, 2, 7),
            t(Suit::Pinzu, 3, 8),
            t(Suit::Pinzu, 4, 9),
        ];
        run.hand_mut().sort();
        run
    }

    #[test]
    fn bot_skips_structure_commits_that_overflow_structure() {
        let mut run = RunState::new_demo();
        *run.structure_tiles_mut() = run.hand().iter().take(12).copied().collect();
        *run.structure_sets_mut() = vec![
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: run.structure_tiles()[0..3].iter().map(|t| t.id).collect(),
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: run.structure_tiles()[3..6].iter().map(|t| t.id).collect(),
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: run.structure_tiles()[6..9].iter().map(|t| t.id).collect(),
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: run.structure_tiles()[9..12].iter().map(|t| t.id).collect(),
            },
        ];
        assert_eq!(run.structure_tiles().len(), HAND_SIZE - 2);

        let best = pick_best_play(&run);
        assert!(
            best.as_ref()
                .map(|(_, indices)| run.structure_tiles().len() + indices.len() <= HAND_SIZE)
                .unwrap_or(true),
            "bot should not choose a play that structure would reject"
        );
    }

    #[test]
    fn enumerated_masks_match_bruteforce_best_play_on_demo_hand() {
        // `RunState::new` / `new_demo` deal is non-deterministic — see `RunState::new` docs.
        let run = scoring_test_run();
        assert_eq!(pick_best_play(&run), brute_force_best_play_in_hand(&run));
    }

    #[test]
    fn bot_finds_kokushi_musou_on_complete_hand() {
        let mut run = RunState::new_demo();
        *run.hand_mut() = vec![
            t(Suit::Manzu, 1, 0),
            t(Suit::Manzu, 9, 1),
            t(Suit::Souzu, 1, 2),
            t(Suit::Souzu, 9, 3),
            t(Suit::Pinzu, 1, 4),
            t(Suit::Pinzu, 9, 5),
            t(Suit::Wind, 1, 6),
            t(Suit::Wind, 2, 7),
            t(Suit::Wind, 3, 8),
            t(Suit::Wind, 4, 9),
            t(Suit::Dragon, 1, 10),
            t(Suit::Dragon, 2, 11),
            t(Suit::Dragon, 3, 12),
            t(Suit::Manzu, 1, 13),
        ];
        run.hand_mut().sort();
        let best = pick_best_play(&run).expect("kokushi should score");
        assert_eq!(best.1.len(), 14);
        let mask: u32 = best.1.iter().fold(0u32, |acc, &i| acc | (1u32 << i));
        let commit_rules = run.validation_rules_for_structure_commits();
        assert!(enumerate_candidate_play_masks(run.hand(), &commit_rules).contains(&mask));
        assert_eq!(Some(best), brute_force_best_play_in_hand(&run));
    }

    #[test]
    fn enumerated_masks_match_bruteforce_best_play_with_flowers() {
        let mut run = RunState::new_demo();
        *run.hand_mut() = vec![
            t(Suit::Manzu, 2, 1),
            t(Suit::Manzu, 3, 2),
            t(Suit::Manzu, 5, 3),
            t(Suit::Manzu, 5, 4),
            t(Suit::Manzu, 5, 5),
            t(Suit::Souzu, 7, 6),
            t(Suit::Souzu, 8, 7),
            t(Suit::Dragon, 1, 8),
            t(Suit::Dragon, 1, 9),
            t(Suit::Flower, 1, 10),
            t(Suit::Flower, 2, 11),
        ];
        run.hand_mut().sort();
        let new_best = best_play_in_hand(&run, run.hand(), None, None, None);
        let old_best = brute_force_best_play_in_hand(&run);
        assert_eq!(new_best, old_best);
    }

    #[test]
    fn candidate_masks_only_produce_valid_selections() {
        let mut run = RunState::new_demo();
        *run.hand_mut() = vec![
            t(Suit::Manzu, 1, 1),
            t(Suit::Manzu, 2, 2),
            t(Suit::Manzu, 3, 3),
            t(Suit::Manzu, 7, 4),
            t(Suit::Manzu, 7, 5),
            t(Suit::Manzu, 7, 6),
            t(Suit::Wind, 1, 7),
            t(Suit::Wind, 1, 8),
            t(Suit::Flower, 1, 9),
            t(Suit::Flower, 2, 10),
            t(Suit::Flower, 3, 11),
            t(Suit::Flower, 4, 12),
        ];
        run.hand_mut().sort();

        let commit_rules = run.validation_rules_for_structure_commits();
        for mask in enumerate_candidate_play_masks(run.hand(), &commit_rules) {
            let tiles: Vec<_> = run
                .hand()
                .iter()
                .enumerate()
                .filter_map(|(i, t)| (mask & (1 << i) != 0).then_some(*t))
                .collect();
            assert!(
                run.try_validate_with_wildcards(&tiles).is_some(),
                "invalid candidate mask {mask:b} for hand {:?}",
                run.hand()
            );
        }
    }

    #[test]
    fn all_flower_hand_enumeration_matches_brute_force() {
        let mut run = RunState::new_demo();
        *run.hand_mut() = (1..=14)
            .map(|i| t(Suit::Flower, ((i - 1) % 4) + 1, i as u32))
            .collect();
        run.hand_mut().sort();
        assert_eq!(
            best_play_in_hand(&run, run.hand(), None, None, None),
            brute_force_best_play_in_hand(&run)
        );
    }

    #[test]
    fn wildflower_marginal_value_is_positive() {
        let mut run = scoring_test_run();
        // Scattered 14-tile hand: wildflower turns every tile into a flower wildcard
        // and unlocks a much stronger play than the baseline melds.
        *run.hand_mut() = vec![
            t(Suit::Manzu, 3, 1),
            t(Suit::Manzu, 5, 2),
            t(Suit::Souzu, 1, 3),
            t(Suit::Souzu, 3, 4),
            t(Suit::Souzu, 6, 5),
            t(Suit::Souzu, 8, 6),
            t(Suit::Souzu, 9, 7),
            t(Suit::Pinzu, 1, 8),
            t(Suit::Pinzu, 2, 9),
            t(Suit::Pinzu, 3, 10),
            t(Suit::Pinzu, 7, 11),
            t(Suit::Pinzu, 9, 12),
            t(Suit::Wind, 1, 13),
            t(Suit::Wind, 2, 14),
        ];
        run.hand_mut().sort();
        let lift =
            super::transform_talisman_lift_on_hand(&run, run.hand(), TalismanKind::Wildflower)
                .expect("wildflower should apply to a non-flower hand");
        assert!(
            lift > 0,
            "wildflower should improve a scattered hand (lift={lift})"
        );
        let base = ShopMarginalBase::with_synthetic_samples(&run, 0, false);
        assert!(
            super::talisman_marginal_value_with_base(&run, TalismanKind::Wildflower, &base) > 0,
            "wildflower shop value should be positive on a helpful hand"
        );
    }

    #[test]
    fn candidate_masks_include_each_flower_identity_for_wildcard_melds() {
        let mut run = RunState::new_demo();
        *run.hand_mut() = vec![
            t(Suit::Manzu, 5, 1),
            t(Suit::Manzu, 5, 2),
            t(Suit::Flower, 1, 3),
            t(Suit::Flower, 2, 4),
        ];
        run.hand_mut().sort();

        let commit_rules = run.validation_rules_for_structure_commits();
        let masks = enumerate_candidate_play_masks(run.hand(), &commit_rules);
        let selected_ids: Vec<Vec<u32>> = masks
            .iter()
            .map(|mask| {
                run.hand()
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, tile)| (mask & (1 << idx) != 0).then_some(tile.id))
                    .collect()
            })
            .collect();

        assert!(
            selected_ids.contains(&vec![1, 2, 3]),
            "expected masks to include 5m-5m with flower 1, got {selected_ids:?}"
        );
        assert!(
            selected_ids.contains(&vec![1, 2, 4]),
            "expected masks to include 5m-5m with flower 2, got {selected_ids:?}"
        );
    }

    #[test]
    fn bot_prefers_more_melds_when_score_ties() {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        struct PlayRank {
            score: u64,
            meld_count: usize,
            tile_count: usize,
        }

        let single_meld = PlayRank {
            score: 120,
            meld_count: 1,
            tile_count: 3,
        };
        let double_meld = PlayRank {
            score: 120,
            meld_count: 2,
            tile_count: 6,
        };

        assert!(double_meld > single_meld);
    }

    #[test]
    fn zodiac_value_is_positive_when_it_levels_a_relevant_yaku() {
        let run = scoring_test_run();
        let shop_base = ShopMarginalBase::new(&run);
        assert!(zodiac_marginal_value_with_base(&run, ZodiacKind::Ox, &shop_base) > 0);
    }

    #[test]
    fn talisman_value_is_positive_when_it_buffs_a_scoring_hand() {
        let run = scoring_test_run();
        assert!(talisman_marginal_value(&run, TalismanKind::Pearl) > 0);
    }

    #[test]
    fn gilded_talisman_shop_value_counts_yen_from_scored_melds() {
        let run = scoring_test_run();
        let pearl = talisman_marginal_value(&run, TalismanKind::Pearl);
        let gilded = talisman_marginal_value(&run, TalismanKind::Gilded);
        assert!(
            gilded > 0,
            "gilded should contribute shop value via yen (pearl={pearl}, gilded={gilded})"
        );
    }

    #[test]
    fn honors_talisman_values_hands_with_few_numbered_tiles() {
        let mut run = scoring_test_run();
        *run.hand_mut() = vec![
            t(Suit::Manzu, 3, 1),
            t(Suit::Manzu, 4, 2),
            t(Suit::Manzu, 5, 3),
            t(Suit::Wind, 1, 4),
            t(Suit::Wind, 1, 5),
            t(Suit::Dragon, 1, 6),
            t(Suit::Dragon, 1, 7),
            t(Suit::Dragon, 1, 8),
        ];
        run.hand_mut().sort();
        assert!(
            super::transform_talisman_lift_on_hand(&run, run.hand(), TalismanKind::Honors)
                .is_some(),
            "honors should value hands with any numbered tiles"
        );
    }

    #[test]
    fn remaining_wings_helper_tracks_final_wing() {
        assert_eq!(remaining_wings_including_current(1), FINAL_WING);
        assert_eq!(remaining_wings_including_current(FINAL_WING), 1);
        assert_eq!(remaining_wings_including_current(FINAL_WING + 1), 0);
    }

    #[test]
    fn long_term_value_scales_down_late_in_the_run() {
        let baseline = FINAL_WING as i32 * 10;
        assert_eq!(scale_long_term_value_for_wing(baseline, 1), baseline);
        assert_eq!(scale_long_term_value_for_wing(baseline, FINAL_WING), 10);
        assert!(
            scale_long_term_value_for_wing(baseline, FINAL_WING)
                < scale_long_term_value_for_wing(baseline, 2)
        );
    }

    #[test]
    fn aggregate_stats_do_not_count_victories_as_deaths() {
        let mut agg = super::AggregateStats::default();
        let win = super::RunStats {
            victory: true,
            died_on_wing: FINAL_WING,
            wings_cleared: FINAL_WING,
            ..Default::default()
        };
        agg.record(&win);

        assert_eq!(agg.victories, 1);
        assert_eq!(agg.max_wing_reached, FINAL_WING);
        assert!(agg.deaths_by_wing.is_empty());
        assert!(agg.deaths_by_chamber.is_empty());
    }

    #[test]
    fn bot_uses_zodiacs_and_best_talisman_from_inventory() {
        let mut run = scoring_test_run();
        run.consumables.items = vec![
            Consumable::Zodiac(ZodiacKind::Ox),
            Consumable::Talisman(TalismanKind::Pearl),
        ];

        assert!(use_bot_consumables(
            &mut run,
            &mut RunStats::default(),
            false
        ));
        assert_eq!(run.yaku_levels.level_of(ZodiacKind::Ox.yaku()), 2);
        assert!(
            run.hand().iter().all(|tile| tile.enhancement.is_some()),
            "best talisman should stamp the hand"
        );
        assert!(run.consumables.items.is_empty());
    }

    #[test]
    fn relic_shop_value_evaluates_swap_when_inventory_full() {
        let mut run = scoring_test_run();
        run.relics.active = vec![
            RelicId::PairPower,
            RelicId::TripletBoost,
            RelicId::SequenceSurge,
            RelicId::HonorFury,
            RelicId::DragonRage,
        ];
        run.recompute_capacities();
        assert!(run.relics.is_full());
        let base = ShopMarginalBase::new(&run);
        let mut sell_index = None;
        let _swap_mv =
            relic_shop_offer_value_with_base(&run, RelicId::JadeSerpent, &base, &mut sell_index);
        assert_eq!(
            relic_marginal_value_with_base(&run, RelicId::JadeSerpent, &base),
            0,
            "straight add path should stay blocked when full"
        );
    }

    #[test]
    fn relic_hold_value_is_zero_without_scoring_relics() {
        let run = scoring_test_run();
        let base = ShopMarginalBase::new(&run);
        assert_eq!(relic_hold_value_with_base(&run, 0, &base), 0);
    }

    #[test]
    fn relic_hold_value_rises_with_scoring_relic() {
        let mut run = scoring_test_run();
        run.relics.active = vec![RelicId::PairPower];
        let base = ShopMarginalBase::new(&run);
        assert!(
            relic_hold_value_with_base(&run, 0, &base) > 0,
            "Pair Power should contribute positive hold value"
        );
    }

    #[test]
    fn proactive_sell_drops_low_hold_relic() {
        let mut run = scoring_test_run();
        run.relics.active = vec![RelicId::PairPower, RelicId::GreenLuck];
        run.yen = 0;
        let base = ShopMarginalBase::new(&run);
        let hold_pair = relic_hold_value_with_base(&run, 0, &base);
        let hold_luck = relic_hold_value_with_base(&run, 1, &base);
        assert!(hold_pair > hold_luck);

        let strategy = BotStrategy {
            sell_enabled: true,
            sell_hold_threshold: hold_pair - 1,
            sell_max_per_visit: 1,
            ..BotStrategy::default()
        };
        let mut stats = RunStats::default();
        let mut bus = EventBus::default();
        let sold =
            sell_underperforming_relics(&mut run, &mut stats, false, &strategy, &base, &mut bus);
        assert_eq!(sold, 1);
        assert_eq!(stats.relics_sold, 1);
        assert_eq!(run.relics.active, vec![RelicId::PairPower]);
        assert!(run.yen > 0);
    }

    #[test]
    fn no_relic_acquisition_strategy_blocks_relic_shop_offers() {
        let strategy = BotStrategy {
            no_relic_acquisition: true,
            ..BotStrategy::default()
        };
        assert!(super::strategy_blocks_shop_offer(
            &strategy,
            super::ShopOffer::Relic(RelicId::PairPower)
        ));
        assert!(!super::strategy_blocks_shop_offer(
            &strategy,
            super::ShopOffer::Zodiac(ZodiacKind::Ox)
        ));
    }
}

#[derive(Clone, Copy, Debug)]
enum ShopOffer {
    Relic(RelicId),
    Zodiac(ZodiacKind),
    Talisman(TalismanKind),
    Pack(TilePackKind),
}

fn strategy_blocks_shop_offer(strategy: &BotStrategy, offer: ShopOffer) -> bool {
    strategy.no_relic_acquisition && matches!(offer, ShopOffer::Relic(_))
}

/// Draw a random 14-tile hand from a fresh shuffled wall. Used for relic value
/// sampling — gives a "typical hand" the relic would face in future plays, not
/// just the bot's specific current hand.
fn sample_random_hand(size: usize) -> Vec<Tile> {
    let mut wall = Wall::from_standard_shuffled();
    let mut hand = Vec::with_capacity(size);
    for _ in 0..size {
        if let Some(t) = wall.draw() {
            hand.push(t);
        }
    }
    hand.sort();
    hand
}

/// Draw a random hand from a wall that includes the run's existing packs
/// plus `extra_pack` (at the next available slot). Used to value a
/// prospective pack purchase by sampling what future hands look like once
/// its tiles are mixed into the wall.
fn sample_random_hand_with_extra_pack(
    run: &RunState,
    extra_pack: TilePackKind,
    size: usize,
) -> Vec<Tile> {
    let mut packs = run.tile_packs.clone();
    packs.push(TilePackInstance::new(extra_pack));
    let mut wall = Wall::from_filtered_with_packs(
        &run.removed_tile_ids,
        &packs,
        &run.tile_enhancements,
        &run.transformed_tiles,
        run.relics.has(RelicId::StrengthInNumbers),
        &run.joker_extra_faces,
    );
    let mut hand = Vec::with_capacity(size);
    for _ in 0..size {
        if let Some(t) = wall.draw() {
            hand.push(t);
        }
    }
    hand.sort();
    hand
}

pub(crate) fn best_play_score_for_hand(
    run: &RunState,
    hand: &[Tile],
    relics_override: Option<&RelicState>,
    yaku_levels_override: Option<&YakuLevels>,
    counters_override: Option<&BTreeMap<RelicId, i32>>,
) -> u64 {
    best_play_in_hand(
        run,
        hand,
        relics_override,
        yaku_levels_override,
        counters_override,
    )
    .map(|(s, _)| s)
    .unwrap_or(0)
}

/// Best-play value for shop/talisman estimates: blind Fu plus yen converted at
/// target÷clear_reward so Gilded and other yen sources compete with score upgrades.
fn best_play_shop_value_for_hand(
    run: &RunState,
    hand: &[Tile],
    relics_override: Option<&RelicState>,
    yaku_levels_override: Option<&YakuLevels>,
) -> i64 {
    let commit_rules = run.validation_rules_for_structure_commits();
    let masks = enumerate_candidate_play_masks(hand, &commit_rules);
    evaluate_play_masks_payoff(
        run,
        hand,
        relics_override,
        yaku_levels_override,
        None,
        &masks,
    )
    .map(|(score, yen, _)| shop_payoff_units(run, score, yen))
    .unwrap_or(0)
}

/// Synthetic random hands per shop valuation (plus the real current hand).
/// Late antes use fewer samples — each one runs `best_play_in_hand`.
pub(crate) fn relic_eval_sample_count(ante: u32) -> usize {
    if ante >= FINAL_WING {
        2
    } else if ante >= 5 {
        3
    } else {
        4
    }
}

/// Cached hands and baseline best-play scores for one shop "next purchase" iteration.
pub(crate) struct ShopMarginalBase {
    pub(crate) hands: Vec<Vec<Tile>>,
    baseline: Vec<u64>,
    pub(crate) optimize_relic_order: bool,
}

impl ShopMarginalBase {
    fn new(run: &RunState) -> Self {
        Self::for_strategy(run, &BotStrategy::default())
    }

    fn for_strategy(run: &RunState, strategy: &BotStrategy) -> Self {
        Self::with_synthetic_samples(
            run,
            strategy.shop_synthetic_sample_count(run.wing),
            strategy.relic_order_optimization_enabled(),
        )
    }

    fn with_synthetic_samples(run: &RunState, n: usize, optimize_relic_order: bool) -> Self {
        let size = crate::core::ordeal::effective_hand_size(run);
        let mut hands = Vec::with_capacity(n + 1);
        hands.push(run.hand().to_vec());
        for _ in 0..n {
            hands.push(sample_random_hand(size));
        }
        let baseline: Vec<u64> = hands
            .iter()
            .map(|h| best_play_score_for_hand(run, h, None, None, None))
            .collect();
        Self {
            hands,
            baseline,
            optimize_relic_order,
        }
    }
}

/// Estimate the value of owning `candidate` by averaging the best-play score
/// improvement across the current hand *and* several synthetic random hands.
///
/// We need the random sampling because most relics' effects are
/// hand-conditional — `TripletBoost` is worthless on a hand with no triplets,
/// Souzu serpent relics do nothing without souzu tiles. Evaluating only the current
/// hand systematically under-values relics whose payoff is "applies whenever you
/// happen to draw the right tiles." A handful of synthetic hands surface that
/// expected value.
///
/// Wall-mutating relics (`StrengthInNumbers`, `SetMagnet`, `QuickDraw`, `WildWinds`,
/// `JokerTile`) are still under-valued because we don't simulate draws between
/// plays. Rarity tie-break compensates.
fn remaining_wings_including_current(ante: u32) -> u32 {
    if ante > FINAL_WING {
        0
    } else {
        FINAL_WING - ante + 1
    }
}

fn scale_long_term_value_for_wing(raw_value: i32, ante: u32) -> i32 {
    if raw_value <= 0 {
        return raw_value;
    }
    let remaining = remaining_wings_including_current(ante) as i64;
    let scaled = (raw_value as i64 * remaining + FINAL_WING as i64 - 1) / FINAL_WING as i64;
    scaled as i32
}

#[inline]
fn relic_can_expand_inventory(id: RelicId) -> bool {
    matches!(id, RelicId::BrocadePouch)
}

fn relic_marginal_value_with_base(
    run: &RunState,
    candidate: RelicId,
    base: &ShopMarginalBase,
) -> i32 {
    if run.relics.owns(candidate) {
        return -1;
    }
    if run.relics.is_full() && !relic_can_expand_inventory(candidate) {
        return 0;
    }

    let mut hypothetical = run.relics.clone();
    hypothetical.active.push(candidate);
    let (hypothetical, counters) =
        relic_order::bot_prepare_hypothetical_roster(run, hypothetical, base);

    let mut delta_sum: i64 = 0;
    for (h, hand) in base.hands.iter().enumerate() {
        delta_sum += best_play_score_for_hand(run, hand, Some(&hypothetical), None, Some(&counters))
            as i64
            - base.baseline[h] as i64;
    }
    let sample_count = base.hands.len() as i64;
    scale_long_term_value_for_wing((delta_sum / sample_count) as i32, run.wing)
}

fn relic_swap_marginal_value_with_base(
    run: &RunState,
    candidate: RelicId,
    replace_index: usize,
    base: &ShopMarginalBase,
) -> i32 {
    if run.relics.owns(candidate) || replace_index >= run.relics.active.len() {
        return 0;
    }

    let mut hypothetical = run.relics.clone();
    hypothetical.active[replace_index] = candidate;
    let (hypothetical, counters) =
        relic_order::bot_prepare_hypothetical_roster(run, hypothetical, base);

    let mut delta_sum: i64 = 0;
    for (h, hand) in base.hands.iter().enumerate() {
        delta_sum += best_play_score_for_hand(run, hand, Some(&hypothetical), None, Some(&counters))
            as i64
            - base.baseline[h] as i64;
    }
    let sample_count = base.hands.len() as i64;
    scale_long_term_value_for_wing((delta_sum / sample_count) as i32, run.wing)
}

fn relic_hold_value_with_base(run: &RunState, index: usize, base: &ShopMarginalBase) -> i32 {
    if index >= run.relics.active.len() {
        return 0;
    }
    let (with, with_counters) =
        relic_order::bot_prepare_hypothetical_roster(run, run.relics.clone(), base);
    let mut without = run.relics.clone();
    without.active.remove(index);
    let (without, without_counters) =
        relic_order::bot_prepare_hypothetical_roster(run, without, base);

    let mut delta_sum: i64 = 0;
    for hand in &base.hands {
        delta_sum += best_play_score_for_hand(run, hand, Some(&with), None, Some(&with_counters))
            as i64
            - best_play_score_for_hand(run, hand, Some(&without), None, Some(&without_counters))
                as i64;
    }
    let sample_count = base.hands.len() as i64;
    if sample_count == 0 {
        return 0;
    }
    scale_long_term_value_for_wing((delta_sum / sample_count) as i32, run.wing)
}

/// Sell owned relics whose hold value is at or below the strategy threshold.
/// Returns how many slots were cleared this pass.
fn sell_underperforming_relics(
    run: &mut RunState,
    stats: &mut RunStats,
    log: bool,
    strategy: &BotStrategy,
    base: &ShopMarginalBase,
    bus: &mut EventBus,
) -> u32 {
    if !strategy.sell_enabled || run.relics.active.is_empty() {
        return 0;
    }

    let mut sold = 0u32;
    while sold < strategy.sell_max_per_visit {
        let mut worst: Option<(usize, i32)> = None;
        for idx in 0..run.relics.active.len() {
            let hold = relic_hold_value_with_base(run, idx, base);
            if hold <= strategy.sell_hold_threshold
                && worst.map(|(_, best_hold)| hold < best_hold).unwrap_or(true)
            {
                worst = Some((idx, hold));
            }
        }
        let Some((idx, hold)) = worst else {
            break;
        };
        let rid = run.relics.active[idx];
        let Some(refund) = bot_sell_relic(run, idx, Some(bus)) else {
            break;
        };
        stats.relics_sold += 1;
        let rname = relic_display_name(rid);
        relic_analytics::record_hold_sell(stats, rname, hold);
        *stats.relics_sold_picked.entry(rname).or_insert(0) += 1;
        bot_log!(
            log,
            "    shop: sold {:?} for {} (hold value {}, threshold {})",
            rid,
            refund,
            hold,
            strategy.sell_hold_threshold
        );
        sold += 1;
    }
    sold
}

/// Marginal shop value for a relic offer. When inventory is full, returns the best
/// swap lift and the owned slot to sell; otherwise behaves like a straight purchase.
fn relic_shop_offer_value_with_base(
    run: &RunState,
    candidate: RelicId,
    base: &ShopMarginalBase,
    sell_index: &mut Option<usize>,
) -> i32 {
    *sell_index = None;
    if run.relics.owns(candidate) {
        return -1;
    }
    if !run.relics.is_full() || relic_can_expand_inventory(candidate) {
        return relic_marginal_value_with_base(run, candidate, base);
    }

    let mut best = 0;
    for idx in 0..run.relics.active.len() {
        let mv = relic_swap_marginal_value_with_base(run, candidate, idx, base);
        if mv > best {
            best = mv;
            *sell_index = Some(idx);
        }
    }
    if best <= 0 {
        *sell_index = None;
    }
    best
}

fn relic_offer_affordable(
    run: &RunState,
    price: i32,
    candidate: RelicId,
    sell_index: Option<usize>,
) -> bool {
    if let Some(idx) = sell_index {
        let owned = run.relics.active[idx];
        let refund = relic_sell_price_live(owned, &run.relic_counters) as i32;
        run.yen + refund >= price
    } else if run.relics.is_full() && !relic_can_expand_inventory(candidate) {
        false
    } else {
        run.yen >= price
    }
}

fn zodiac_marginal_value_with_base(
    run: &RunState,
    zodiac: ZodiacKind,
    base: &ShopMarginalBase,
) -> i32 {
    let mut hypothetical = run.yaku_levels.clone();
    hypothetical.level_up_for_zodiac(zodiac);

    let mut delta_sum: i64 = 0;
    for (h, hand) in base.hands.iter().enumerate() {
        delta_sum += best_play_score_for_hand(run, hand, None, Some(&hypothetical), None) as i64
            - base.baseline[h] as i64;
    }
    let sample_count = base.hands.len() as i64;
    scale_long_term_value_for_wing((delta_sum / sample_count) as i32, run.wing)
}

/// Simulate applying a transform talisman to every tile in `hand` and return
/// the expected best-play score delta. Returns `None` when the transform would
/// not change the hand.
pub(crate) fn transform_talisman_lift_on_hand(
    run: &RunState,
    hand: &[Tile],
    kind: TalismanKind,
) -> Option<i64> {
    use crate::core::tile::Suit;
    if hand.is_empty() {
        return None;
    }
    let base = best_play_score_for_hand(run, hand, None, None, None) as i64;
    match kind {
        // Suit-transforms rewrite selected numbered tiles to a target suit.
        // Convert all numbered tiles NOT already in the target suit and
        // measure the delta against the baseline.
        TalismanKind::Souzu | TalismanKind::Pinzu | TalismanKind::Manzu => {
            let target = match kind {
                TalismanKind::Souzu => Suit::Souzu,
                TalismanKind::Pinzu => Suit::Pinzu,
                TalismanKind::Manzu => Suit::Manzu,
                _ => unreachable!(),
            };
            let sel: Vec<usize> = hand
                .iter()
                .enumerate()
                .filter(|(_, t)| t.is_number_tile() && t.suit != target)
                .map(|(i, _)| i)
                .collect();
            if sel.is_empty() {
                return None;
            }
            let mut simulated = hand.to_vec();
            for &i in &sel {
                if simulated[i].is_number_tile() {
                    simulated[i].suit = target;
                }
            }
            simulated.sort();
            let after = best_play_score_for_hand(run, &simulated, None, None, None) as i64;
            Some(after - base)
        }
        // Wildflower: selected tiles become flowers. Flood the hand and
        // simulate — ranks are randomized in-game but distinct ranks help
        // the play enumerator distinguish flower identities.
        TalismanKind::Wildflower => {
            let sel: Vec<usize> = hand
                .iter()
                .enumerate()
                .filter(|(_, t)| t.suit != Suit::Flower)
                .map(|(i, _)| i)
                .collect();
            if sel.is_empty() {
                return None;
            }
            let mut simulated = hand.to_vec();
            for (n, &i) in sel.iter().enumerate() {
                simulated[i].suit = Suit::Flower;
                simulated[i].rank = ((n % 4) + 1) as u8;
            }
            simulated.sort();
            let after = best_play_score_for_hand(run, &simulated, None, None, None) as i64;
            Some(after - base)
        }
        // Conformity: every tile becomes a copy of a random hand tile. Since
        // the template is random, average the score delta over every possible
        // template pick to get expected value.
        TalismanKind::Conformity => {
            if hand.len() < 2 {
                return None;
            }
            let mut total_delta: i64 = 0;
            for template_idx in 0..hand.len() {
                let template = hand[template_idx];
                let mut simulated = hand.to_vec();
                for t in simulated.iter_mut() {
                    t.suit = template.suit;
                    t.rank = template.rank;
                }
                simulated.sort();
                let after = best_play_score_for_hand(run, &simulated, None, None, None) as i64;
                total_delta += after - base;
            }
            Some(total_delta / hand.len() as i64)
        }
        // Honors: every numbered tile becomes a random honor. Average several
        // RNG draws to match in-game variance.
        TalismanKind::Honors => {
            use crate::core::tile::Suit;
            let sel: Vec<usize> = hand
                .iter()
                .enumerate()
                .filter(|(_, t)| t.is_number_tile())
                .map(|(i, _)| i)
                .collect();
            if sel.is_empty() {
                return None;
            }
            const HONORS_SAMPLES: i64 = 8;
            let honor_suits = [Suit::Wind, Suit::Dragon];
            let mut total_delta: i64 = 0;
            for _ in 0..HONORS_SAMPLES {
                let mut rng = rand::rng();
                let mut simulated = hand.to_vec();
                for &i in &sel {
                    let suit = honor_suits[rng.random_range(0..honor_suits.len())];
                    simulated[i].suit = suit;
                    simulated[i].rank = if suit == Suit::Wind {
                        rng.random_range(1..=4)
                    } else {
                        rng.random_range(1..=3)
                    };
                }
                simulated.sort();
                let after = best_play_score_for_hand(run, &simulated, None, None, None) as i64;
                total_delta += after - base;
            }
            Some(total_delta / HONORS_SAMPLES)
        }
        // Buff talismans should never reach this function.
        TalismanKind::Pearl | TalismanKind::Gilded | TalismanKind::Polychrome => None,
    }
}

/// Average per-hand lift from a transform talisman across the shop sample batch.
fn sampled_transform_talisman_raw_avg(
    run: &RunState,
    kind: TalismanKind,
    base: &ShopMarginalBase,
) -> i32 {
    let mut delta_sum: i64 = 0;
    let mut count: i64 = 0;
    for hand in &base.hands {
        if let Some(delta) = transform_talisman_lift_on_hand(run, hand, kind) {
            delta_sum += delta;
            count += 1;
        }
    }
    if count == 0 {
        return 0;
    }
    (delta_sum / count).max(0) as i32
}

/// One-shot consumables only pay out once; scale down vs relics that persist every blind.
fn talisman_one_shot_shop_value(raw_avg: i32, ante: u32) -> i32 {
    if raw_avg <= 0 {
        return 0;
    }
    let remaining_chambers = (remaining_wings_including_current(ante) as i64 * 3).max(1);
    (raw_avg as i64 / remaining_chambers) as i32
}

pub(crate) fn buff_talisman_lift_on_hand(
    run: &RunState,
    hand: &[Tile],
    talisman: TalismanKind,
) -> i32 {
    let baseline = best_play_shop_value_for_hand(run, hand, None, None);
    let mut enhanced = hand.to_vec();
    crate::core::talisman::apply_to_hand(&mut enhanced, talisman);
    let buffed = best_play_shop_value_for_hand(run, &enhanced, None, None);
    (buffed - baseline).max(0) as i32
}

fn buff_talisman_raw_avg(run: &RunState, talisman: TalismanKind, base: &ShopMarginalBase) -> i32 {
    let mut delta_sum: i64 = 0;
    for hand in &base.hands {
        delta_sum += buff_talisman_lift_on_hand(run, hand, talisman) as i64;
    }
    let sample_count = base.hands.len() as i64;
    (delta_sum / sample_count) as i32
}

fn talisman_marginal_value_with_base(
    run: &RunState,
    talisman: TalismanKind,
    base: &ShopMarginalBase,
) -> i32 {
    let raw_avg = if talisman.enhancement().is_some() {
        buff_talisman_raw_avg(run, talisman, base)
    } else {
        sampled_transform_talisman_raw_avg(run, talisman, base)
    };
    if run.relics.has(RelicId::BrocadePouch) {
        return scale_long_term_value_for_wing(raw_avg, run.wing);
    }
    let mut mv = talisman_one_shot_shop_value(raw_avg, run.wing);
    if matches!(talisman, TalismanKind::Polychrome | TalismanKind::Pearl) {
        mv = mv.saturating_mul(2);
    }
    mv
}

#[cfg(test)]
fn talisman_marginal_value(run: &RunState, talisman: TalismanKind) -> i32 {
    talisman_marginal_value_with_base(run, talisman, &ShopMarginalBase::new(run))
}

/// Estimate the value of buying a booster pack. Pack tiles permanently
/// enrich the wall, so the benefit pays out across every future round —
/// mirror the relic value model (sample several hands, scale by remaining
/// antes) rather than the one-shot talisman model.
fn pack_marginal_value(run: &RunState, kind: TilePackKind, strategy: &BotStrategy) -> i32 {
    let mut delta_sum: i64 = 0;
    let mut sample_count: i64 = 0;
    // Baseline samples the run's *current* wall (with any already-owned
    // packs mixed in); comparison samples the wall with the prospective
    // pack added. This captures diminishing returns — a second Flowers
    // Pack is worth much less than the first.
    let pack_iters = strategy
        .shop_synthetic_sample_count(run.wing)
        .saturating_add(1);
    for _ in 0..pack_iters {
        let base_wall = Wall::from_filtered_with_packs(
            &run.removed_tile_ids,
            &run.tile_packs,
            &run.tile_enhancements,
            &run.transformed_tiles,
            run.relics.has(RelicId::StrengthInNumbers),
            &run.joker_extra_faces,
        );
        let target = crate::core::ordeal::effective_hand_size(run);
        let mut base_hand = Vec::with_capacity(target);
        let mut base_wall = base_wall;
        for _ in 0..target {
            if let Some(t) = base_wall.draw() {
                base_hand.push(t);
            }
        }
        base_hand.sort();
        let with = sample_random_hand_with_extra_pack(
            run,
            kind,
            crate::core::ordeal::effective_hand_size(run),
        );
        let base_score = best_play_score_for_hand(run, &base_hand, None, None, None) as i64;
        let enriched = best_play_score_for_hand(run, &with, None, None, None) as i64;
        delta_sum += enriched - base_score;
        sample_count += 1;
    }
    if sample_count == 0 {
        return 0;
    }
    // Halve the raw delta before long-term scaling. Rationale: relics
    // give per-play multipliers that compound across the round's 5 plays;
    // a pack tile only scores on the plays that actually include it
    // (typically 1–2 of the 5). Raw best-play delta overstates the
    // round-level benefit. Empirically this keeps the bot from crowding
    // out higher-value relic purchases.
    let avg = (delta_sum / sample_count) as i32 / 2;
    scale_long_term_value_for_wing(avg, run.wing)
}

fn use_bot_consumables(run: &mut RunState, stats: &mut RunStats, log: bool) -> bool {
    let mut used_any = false;

    while let Some(idx) = run
        .consumables
        .items
        .iter()
        .position(|c| matches!(c, Consumable::Zodiac(_)))
    {
        let zodiac = match run.consumables.items[idx] {
            Consumable::Zodiac(z) => z,
            Consumable::Talisman(_) | Consumable::Memorial(_) => unreachable!(),
        };
        let _ = run.use_consumable(idx, &mut crate::game::event_bus::EventBus::default());
        *stats.zodiacs_used.entry(zodiac.name()).or_insert(0) += 1;
        bot_log!(log, "      action: use zodiac {:?}", zodiac);
        used_any = true;
    }

    let base = pick_best_play(run).map(|(s, _)| s).unwrap_or(0);
    let mut best_talisman: Option<(usize, TalismanKind, i32)> = None;
    for (idx, consumable) in run.consumables.items.iter().copied().enumerate() {
        let Consumable::Talisman(kind) = consumable else {
            continue;
        };
        // At use-time, evaluate against the actual current hand (not the
        // shop's sampled / discounted estimate).
        let delta = if kind.enhancement().is_none() {
            transform_talisman_lift_on_hand(run, run.hand(), kind)
                .map(|d| d.max(0) as i32)
                .unwrap_or(0)
        } else {
            buff_talisman_lift_on_hand(run, run.hand(), kind)
        };
        if delta <= 0 {
            continue;
        }
        if best_talisman
            .as_ref()
            .map(|(_, _, best_delta)| delta > *best_delta)
            .unwrap_or(true)
        {
            best_talisman = Some((idx, kind, delta));
        }
    }

    if let Some((idx, kind, delta)) = best_talisman {
        let _ = run.use_consumable(idx, &mut crate::game::event_bus::EventBus::default());
        *stats.talismans_used.entry(kind.name()).or_insert(0) += 1;
        bot_log!(
            log,
            "      action: use talisman {:?} (+{} projected best-play value from {})",
            kind,
            delta,
            base
        );
        used_any = true;
    }

    used_any
}

/// Headless analogue of `ShopScene::new` + buy loop. Rolls relics plus
/// consumables and buys the affordable offer with the largest positive
/// marginal value.
fn visit_shop(
    run: &mut RunState,
    stats: &mut RunStats,
    log: bool,
    strategy: &BotStrategy,
    deadline: Option<Instant>,
    bus: &mut crate::game::event_bus::EventBus,
) -> ShopVisitOutcome {
    if strategy.relic_order_optimization_enabled() {
        relic_order::bot_optimize_relic_order(run);
    }
    // Consume tag-granted shop modifiers (headless analogue of ShopScene::new).
    let extra_relics: usize = (run.tag_rich_stock as usize * 2).max(run.tag_patron_gift as usize);
    let patron_gifts = run.tag_patron_gift;
    // Free restock is a no-op for the bot (it doesn't restock).
    run.tag_free_restock = 0;
    run.tag_patron_gift = 0;
    run.tag_rich_stock = 0;

    let defs = all_relic_defs();
    let pool_x = run.relic_shop_pool_extinction();
    let mut pool: Vec<RelicId> = defs
        .iter()
        .filter(|d| relic_eligible_for_shop_stock(d.id, &run.relics, &run.available_relics, pool_x))
        .map(|d| d.id)
        .collect();
    pool.shuffle(&mut rand::rng());
    let mut rng = rand::rng();
    let min_relic_slots = usize::from(!pool.is_empty());
    let crate::game::run::ShopOfferCounts {
        n_relics,
        n_zodiacs,
        n_talismans,
    } = roll_shop_offer_counts(extra_relics, KIOSK_RELIC_SLOTS, min_relic_slots, &mut rng);

    let mut zodiac_pool = run.zodiac_spawn_pool();
    zodiac_pool.shuffle(&mut rng);
    let mut talisman_pool: Vec<TalismanKind> = TalismanKind::all().to_vec();
    talisman_pool.shuffle(&mut rng);
    let mut pack_pool: Vec<TilePackKind> = TilePackKind::all().to_vec();
    pack_pool.shuffle(&mut rng);

    let mut shop: Vec<ShopOffer> = pool
        .into_iter()
        .take(n_relics)
        .map(ShopOffer::Relic)
        .collect();
    shop.extend(
        zodiac_pool
            .into_iter()
            .take(n_zodiacs)
            .map(ShopOffer::Zodiac),
    );
    shop.extend(
        talisman_pool
            .into_iter()
            .take(n_talismans)
            .map(ShopOffer::Talisman),
    );
    // 2 pack slots per shop, matching `N_TILE_PACKS` in `src/scenes/shop.rs`.
    shop.extend(pack_pool.into_iter().take(2).map(ShopOffer::Pack));

    for offer in &shop {
        if let ShopOffer::Relic(id) = offer {
            relic_analytics::record_shop_offer(stats, relic_display_name(*id));
        }
    }

    let mut free_relics = patron_gifts;
    bot_log!(
        log,
        "    shop: yen {} | relic slots {}/{} | consumables {}/{} | offerings {:?}{}",
        run.yen,
        run.relics.active.len(),
        run.relics.max_slots,
        run.consumables.items.len(),
        run.consumables.capacity,
        shop,
        if free_relics > 0 {
            " | patron gift active"
        } else {
            ""
        }
    );
    loop {
        if run_deadline_expired(deadline) {
            bot_log!(log, "    shop: deadline hit mid-visit");
            return ShopVisitOutcome::TimedOut;
        }
        if shop.is_empty() {
            let shop_base = ShopMarginalBase::for_strategy(run, strategy);
            if sell_underperforming_relics(run, stats, log, strategy, &shop_base, bus) > 0 {
                continue;
            }
            bot_log!(log, "    shop: leaving ({})", "no offerings left");
            break;
        }
        let shop_base = ShopMarginalBase::for_strategy(run, strategy);
        if sell_underperforming_relics(run, stats, log, strategy, &shop_base, bus) > 0 {
            continue;
        }
        // Find the best affordable offer with positive marginal value.
        let mut best: Option<(usize, i32, Option<usize>)> = None;
        for (i, offer) in shop.iter().copied().enumerate() {
            if strategy_blocks_shop_offer(strategy, offer) {
                continue;
            }
            let price = match offer {
                ShopOffer::Relic(id) => {
                    if free_relics > 0 {
                        0
                    } else {
                        run.mode.scale_shop_price(relic_shop_price(id, &run.relics))
                    }
                }
                ShopOffer::Zodiac(_) => run.mode.scale_shop_price(apply_merchants_eye_discount(
                    ZodiacKind::shop_price(),
                    &run.relics,
                )),
                ShopOffer::Talisman(kind) => run
                    .mode
                    .scale_shop_price(apply_merchants_eye_discount(kind.shop_price(), &run.relics)),
                ShopOffer::Pack(kind) => run
                    .mode
                    .scale_shop_price(apply_merchants_eye_discount(kind.shop_price(), &run.relics)),
            };
            let price_i32 = price as i32;
            let mut sell_index = None;
            let raw_mv = match offer {
                ShopOffer::Relic(id) => {
                    let mv = relic_shop_offer_value_with_base(run, id, &shop_base, &mut sell_index);
                    if !relic_offer_affordable(run, price_i32, id, sell_index) {
                        continue;
                    }
                    mv
                }
                ShopOffer::Zodiac(zodiac) => {
                    if price_i32 > run.yen {
                        continue;
                    }
                    zodiac_marginal_value_with_base(run, zodiac, &shop_base)
                }
                ShopOffer::Talisman(kind) => {
                    if price_i32 > run.yen {
                        continue;
                    }
                    if run.consumables.is_full()
                        || run.consumables.items.iter().any(
                            |c| matches!(c, Consumable::Talisman(existing) if *existing == kind),
                        )
                    {
                        0
                    } else {
                        talisman_marginal_value_with_base(run, kind, &shop_base)
                    }
                }
                ShopOffer::Pack(kind) => {
                    if price_i32 > run.yen {
                        continue;
                    }
                    pack_marginal_value(run, kind, strategy)
                }
            };
            // Apply the strategy's category weight. weight = 0 zeros the
            // value out; weight > 1 biases the bot toward that category.
            let weight = match offer {
                ShopOffer::Relic(_) => strategy.relic_weight,
                ShopOffer::Zodiac(_) => strategy.zodiac_weight,
                ShopOffer::Talisman(_) => strategy.talisman_weight,
                ShopOffer::Pack(_) => strategy.pack_weight,
            };
            let mv = (raw_mv as f32 * weight) as i32;
            if mv <= 0 {
                continue;
            }
            if best.as_ref().map(|(_, b, _)| mv > *b).unwrap_or(true) {
                best = Some((i, mv, sell_index));
            }
        }
        let Some((idx, marginal_value, sell_index)) = best else {
            let shop_base = ShopMarginalBase::for_strategy(run, strategy);
            if sell_underperforming_relics(run, stats, log, strategy, &shop_base, bus) > 0 {
                continue;
            }
            bot_log!(log, "    shop: no positive-value affordable purchase");
            break;
        };
        let offer = shop.remove(idx);
        match offer {
            ShopOffer::Relic(id) => {
                let price = if free_relics > 0 {
                    0
                } else {
                    run.mode.scale_shop_price(relic_shop_price(id, &run.relics))
                };
                free_relics = free_relics.saturating_sub(1);
                if let Some(sell_idx) = sell_index {
                    let sold = run.relics.active[sell_idx];
                    let hold = relic_hold_value_with_base(run, sell_idx, &shop_base);
                    if let Some(refund) = bot_sell_relic(run, sell_idx, Some(bus)) {
                        relic_analytics::record_hold_sell(stats, relic_display_name(sold), hold);
                        bot_log!(
                            log,
                            "    shop: sold {:?} for {} (making room, hold {})",
                            sold,
                            refund,
                            hold
                        );
                    }
                }
                run.apply_yen_delta(-(price as i32), Some(bus));
                acquire_relic(run, id, strategy);
                stats.relics_bought += 1;
                let rname = relic_display_name(id);
                relic_analytics::record_marginal_buy(stats, rname, marginal_value);
                *stats.relics_picked.entry(rname).or_insert(0) += 1;
                if run.wing <= crate::bot::stats::RELIC_SHOP_TIMING_EARLY_WING_MAX {
                    *stats.relics_picked_shop_early.entry(rname).or_insert(0) += 1;
                } else {
                    *stats.relics_picked_shop_late.entry(rname).or_insert(0) += 1;
                }
                stats.yen_spent += price;
                bot_log!(
                    log,
                    "    shop: bought {:?} for {} (marginal value {}, yen now {})",
                    id,
                    price,
                    marginal_value,
                    run.yen
                );
            }
            ShopOffer::Zodiac(zodiac) => {
                let price = run.mode.scale_shop_price(apply_merchants_eye_discount(
                    ZodiacKind::shop_price(),
                    &run.relics,
                ));
                run.apply_yen_delta(-(price as i32), Some(bus));
                let new_level = run.yaku_levels.level_up_for_zodiac(zodiac);
                stats.yen_spent += price;
                *stats.zodiacs_picked.entry(zodiac.name()).or_insert(0) += 1;
                // Bot shop flow applies zodiac effects immediately on purchase
                // (same as game shop command path), so treat that as a "use"
                // for telemetry parity with inventory consumes.
                *stats.zodiacs_used.entry(zodiac.name()).or_insert(0) += 1;
                bot_log!(
                    log,
                    "    shop: bought zodiac {:?} for {} (marginal value {}, level {}, yen now {})",
                    zodiac,
                    price,
                    marginal_value,
                    new_level,
                    run.yen
                );
            }
            ShopOffer::Talisman(kind) => {
                let price = run
                    .mode
                    .scale_shop_price(apply_merchants_eye_discount(kind.shop_price(), &run.relics));
                run.apply_yen_delta(-(price as i32), Some(bus));
                run.consumables.items.push(Consumable::Talisman(kind));
                stats.yen_spent += price;
                *stats.talismans_picked.entry(kind.name()).or_insert(0) += 1;
                bot_log!(
                    log,
                    "    shop: bought talisman {:?} for {} (marginal value {}, yen now {})",
                    kind,
                    price,
                    marginal_value,
                    run.yen
                );
            }
            ShopOffer::Pack(kind) => {
                let price = run
                    .mode
                    .scale_shop_price(apply_merchants_eye_discount(kind.shop_price(), &run.relics));
                run.apply_yen_delta(-(price as i32), Some(bus));
                // Mirror the real shop: pre-stamp any enhancement from the
                // pack kind onto the tiles' IDs, then append the pack. The
                // wall gets rebuilt with these packs at the start of every
                // round (`advance_round` calls `from_filtered_with_packs`).
                let pack_idx = run.tile_packs.len();
                let start_id = crate::core::tile_pack::PACK_TILE_ID_BASE
                    + (pack_idx as u32) * crate::core::tile_pack::PACK_ID_STRIDE;
                let instance = TilePackInstance::new(kind);
                if let Some(enh) = kind.pre_enhancement() {
                    for t in instance.tiles_at(start_id) {
                        run.tile_enhancements.insert(t.id, enh);
                    }
                }
                run.tile_packs.push(instance);
                stats.yen_spent += price;
                *stats.packs_picked.entry(kind.name()).or_insert(0) += 1;
                bot_log!(
                    log,
                    "    shop: bought pack {:?} for {} (marginal value {}, yen now {})",
                    kind,
                    price,
                    marginal_value,
                    run.yen
                );
            }
        }
    }
    ShopVisitOutcome::Completed
}

/// Decide whether to skip the upcoming non-Boss blind.
///
/// Strategy-sweep data (200 runs/strategy, full pool) shows `never-skip` (37.5%)
/// beats `baseline` (35%) beats `always-skip` (33.5%). Clearing a "trivially easy"
/// blind yields ¥4–5 base + unused-play bonus + interest ≈ ¥7–10, while the
/// expected temptation value is ~¥6.75 (weighted average across the tag pool) —
/// so clearing is usually better on raw yen AND accumulates per-play relic stacks.
///
/// We only skip when BOTH conditions hold:
/// 1. The temptation's yen_value beats the estimated clear value (tag is worth it).
/// 2. The projected score exceeds `skip_threshold_multiplier` × target (we're
///    comfortably ahead and don't need the plays).
///
/// Boss blinds can never be skipped.
fn should_skip_chamber(run: &RunState, blind: ChamberKind, strategy: &BotStrategy) -> bool {
    if matches!(blind, ChamberKind::Ordeal) {
        return false;
    }
    let target = run.chamber_score_target(blind);
    let best = pick_best_play(run).map(|(s, _)| s).unwrap_or(0);
    if best == 0 {
        return false;
    }

    // Estimate the clear reward: base payout + expected unused plays (half of
    // starting plays, conservatively) + capped interest.
    let clear_base = blind.clear_reward();
    let expected_unused_plays = run.plays_remaining / 2;
    let interest = (run.yen.max(0) as u32 / 5).min(3);
    let expected_clear_yen = clear_base + expected_unused_plays + interest;

    // Only skip if the tag is worth more than clearing.
    let tag_yen = run
        .tag_for_chamber(blind)
        .map(|t| t.yen_value())
        .unwrap_or(0);
    if tag_yen <= expected_clear_yen {
        return false;
    }

    // Also require a comfortable score overshoot so we're not gambling.
    let projected = best.saturating_mul(run.plays_remaining as u64);
    let threshold = (target as f32 * strategy.skip_threshold_multiplier).max(0.0) as u64;
    projected >= threshold
}

/// Play one headless bot run; returns terminal [`RunState`] and aggregate stats.
pub fn play_bot_run(
    config: BotConfig,
    options: BotRunOptions,
    run_number: Option<u32>,
) -> (RunState, RunStats) {
    play_run_with_options(config, options, run_number)
}

fn play_run_with_options(
    config: BotConfig,
    options: BotRunOptions,
    run_number: Option<u32>,
) -> (RunState, RunStats) {
    let strategy = BotStrategy::from_config(&config);
    let forced_relic = config.forced_relic;
    let meta_depth = config.meta_depth;
    let mode = config.into_mode();
    let mut run = RunState::new(mode);
    if let Some(depth) = meta_depth {
        let depth = depth.clamp(1, crate::core::progression::MAX_PROGRESS_LEVEL);
        let mut progress = crate::core::progression::PlayerProgress::new();
        progress.level_progress_points =
            crate::core::progression::PlayerProgress::min_points_for_level(depth);
        let _ = progress.check_level_up();
        run.apply_progression(&progress);
    }
    let mut stats = RunStats::default();
    let mut bus = EventBus::default();
    let log = options.log;
    let started = Instant::now();
    let deadline = options
        .run_timeout
        .filter(|d| !d.is_zero())
        .map(|d| started + d);

    // Forced-relic injection (for causal sweeps): grant the relic for
    // free at run start so every run in the cell has the same build
    // starter. The bot then plays normally from there.
    if let Some(id) = forced_relic {
        acquire_relic(&mut run, id, &strategy);
        bot_log!(log, "  forced relic injection: {:?}", id);
    }

    bot_log!(
        log,
        "== bot run{} start: ante {} | blind {} | target {} | yen {} ==",
        run_number.map(|n| format!(" #{n}")).unwrap_or_default(),
        run.wing,
        chamber_log_label(&run, run.upcoming_chamber),
        run.target_score,
        run.yen
    );

    loop {
        if run.is_run_complete() {
            stats.victory = true;
            stats.died_on_wing = FINAL_WING;
            bot_log!(
                log,
                "== bot run complete: victory at ante {} ==",
                FINAL_WING
            );
            break;
        }
        if run_deadline_expired(deadline) {
            record_timeout_snapshot(
                &mut stats,
                &run,
                "outer",
                run.upcoming_chamber,
                None,
                started,
            );
            break;
        }
        let blind = run.upcoming_chamber;
        bot_log!(
            log,
            "  ante {} | blind {} | target {} | yen {} | relics {}",
            run.wing,
            chamber_log_label(&run, blind),
            run.target_score,
            run.yen,
            run.relics.active.len()
        );

        // Skip strategy: bank yen on Small/Big when projected score comfortably
        // overshoots the target. Tag rewards replace flat yen — apply them
        // the same way the pick-blind scene does.
        if should_skip_chamber(&run, blind, &strategy) {
            bot_log!(log, "    action: skip {}", blind.name());
            if let Some(tag) = run.tag_for_chamber(blind) {
                let yen_before = run.yen;
                run.apply_tag(tag, Some(&mut bus));
                let yen_after = run.yen;
                let realized_yen = yen_after.saturating_sub(yen_before).max(0) as u32;
                stats.yen_from_temptations += realized_yen;
                stats.temptation_yen_value += tag.yen_value();
                *stats.temptations_taken.entry(tag.name()).or_insert(0) += 1;
                for (zodiac, _, _) in &run.pending_zodiac_celebrations {
                    *stats.zodiacs_picked.entry(zodiac.name()).or_insert(0) += 1;
                }
            }
            run.skip_to_next_chamber();
            stats.chambers_skipped += 1;
            continue;
        }

        if run_deadline_expired(deadline) {
            record_timeout_snapshot(&mut stats, &run, "outer", blind, None, started);
            break;
        }

        stats.total_target_score += run.target_score as u64;
        run.apply_chamber(blind, Some(&mut bus));
        let ordeal_for_this_chamber = if matches!(blind, ChamberKind::Ordeal) {
            current_ordeal_name(&run).map(|name| name.to_string())
        } else {
            None
        };
        if let Some(ordeal_name) = &ordeal_for_this_chamber {
            stats.ordeal_faced.insert(ordeal_name.clone(), 1);
        }
        let mut chamber_scoring: Vec<BotScoringAction> = Vec::new();
        let (outcome, chamber_turns) = play_chamber(
            &mut run,
            &mut stats,
            log,
            deadline,
            &mut chamber_scoring,
            &strategy,
        );
        let chamber_score = run.round_score;
        if matches!(blind, ChamberKind::Ordeal) {
            *stats.ordeal_score_by_wing.entry(run.wing).or_insert(0) += chamber_score;
            *stats.ordeal_attempts_by_wing.entry(run.wing).or_insert(0) += 1;
        }
        stats.total_score += chamber_score;
        if chamber_score > stats.peak_chamber_score {
            let golden_engine_active = run.relics.has(RelicId::GoldenEngine);
            stats.peak_chamber_detail = Some(PeakChamberSnapshot {
                chamber_slot: format!("{:02}-{}", run.wing, blind.name()),
                chamber_label: chamber_log_label(&run, blind),
                target_score: run.target_score,
                total_score: chamber_score,
                relics: run
                    .relics
                    .active
                    .iter()
                    .map(|&id| relic_display_name(id).to_string())
                    .collect(),
                yen_held: Some(run.yen),
                golden_engine_han_bonus: golden_engine_active
                    .then_some(golden_engine_han_bonus(run.yen)),
                relic_state: peak_chamber_relic_state(&run),
                scoring_actions: chamber_scoring,
            });
        }
        stats.peak_chamber_score = stats.peak_chamber_score.max(chamber_score);
        stats.died_on_wing = run.wing;
        stats.died_on_chamber = blind;
        if let Some(ordeal_name) = &ordeal_for_this_chamber
            && outcome == PlayChamberOutcome::Cleared
        {
            stats.ordeal_beaten.insert(ordeal_name.clone(), 1);
        }
        if outcome == PlayChamberOutcome::TimedOut {
            record_timeout_snapshot(
                &mut stats,
                &run,
                "playing_chamber",
                blind,
                Some(chamber_turns),
                started,
            );
            stats.final_yen = run.yen;
            bot_log!(
                log,
                "== bot run end: wall-clock timeout during {} ==",
                chamber_log_label(&run, blind)
            );
            break;
        }
        if outcome == PlayChamberOutcome::LostRun {
            stats.final_yen = run.yen;
            bot_log!(
                log,
                "== bot run end: died on ante {} {} with yen {} ==",
                run.wing,
                chamber_log_label(&run, blind),
                run.yen
            );
            break;
        }
        if outcome == PlayChamberOutcome::SecondWindForfeit {
            stats.second_wind_forfeits += 1;
            bot_log!(
                log,
                "  second wind — forfeited blind; yen {} | next {:?}",
                run.yen,
                run.upcoming_chamber
            );
            match visit_shop(&mut run, &mut stats, log, &strategy, deadline, &mut bus) {
                ShopVisitOutcome::Completed => {}
                ShopVisitOutcome::TimedOut => {
                    record_timeout_snapshot(
                        &mut stats,
                        &run,
                        "shop",
                        run.upcoming_chamber,
                        None,
                        started,
                    );
                    stats.final_yen = run.yen;
                    break;
                }
            }
            continue;
        }
        stats.chambers_cleared += 1;
        let blind_overscore = run.round_score.saturating_sub(run.target_score as u64);
        stats.total_overscore += blind_overscore;
        let slot_key = format!("{:02}-{}", run.wing, blind.name());
        *stats
            .turns_cleared_by_slot
            .entry(slot_key.clone())
            .or_insert(0) += chamber_turns;
        *stats.overscore_by_slot.entry(slot_key.clone()).or_insert(0) += blind_overscore;
        *stats.cleared_by_slot.entry(slot_key).or_insert(0) += 1;
        if matches!(blind, ChamberKind::Ordeal) {
            stats.wings_cleared += 1;
        }
        let payout = clear_payout_breakdown(&run);
        stats.yen_from_clears += payout.total;
        stats.yen_from_clear_base += payout.base_reward;
        stats.yen_from_unused_plays += payout.unused_play_bonus;
        stats.yen_from_interest += payout.interest;
        stats.yen_from_clear_relics += payout.relic_bonus;
        stats.yen_clear_green_luck += payout.green_luck_bonus;
        stats.yen_clear_gold_idol += payout.gold_idol_bonus;
        stats.yen_clear_jade_abacus += payout.jade_abacus_bonus;
        stats.yen_clear_patience += payout.patience_bonus;

        bot_log!(
            log,
            "  blind {} cleared; advancing with yen {} and total score {}",
            chamber_log_label(&run, blind),
            run.yen,
            stats.total_score
        );

        run.advance_round(&mut bus);

        // Shop visit happens after advance_round (matching Shop → Hallway scene
        // flow), so we evaluate purchases against the freshly-drawn next hand.
        match visit_shop(&mut run, &mut stats, log, &strategy, deadline, &mut bus) {
            ShopVisitOutcome::Completed => {}
            ShopVisitOutcome::TimedOut => {
                record_timeout_snapshot(
                    &mut stats,
                    &run,
                    "shop",
                    run.upcoming_chamber,
                    None,
                    started,
                );
                stats.final_yen = run.yen;
                break;
            }
        }
    }

    stats.final_relics = run
        .relics
        .active
        .iter()
        .map(|&id| relic_display_name(id).to_string())
        .collect();
    stats.final_consumables = run.consumables.items.iter().map(|c| c.name()).collect();
    stats.final_yaku_levels = run.yaku_levels.clone();
    stats.final_yen = run.yen;
    bot_log!(log, "== bot run stats: {:?} ==", stats);
    (run, stats)
}
