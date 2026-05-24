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
//! Run with: `cargo run --release -- --bot 200`

use rand::RngExt;
use rand::seq::SliceRandom;
use rayon::prelude::*;
use serde::Serialize;
use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::core::consumable::Consumable;
use crate::core::deck::Wall;
use crate::core::hand::{MeldKind, detect_all_sets, validate_selection_with_rules};
use crate::core::relic::{
    RelicId, RelicState, ScoreContext, ScoreEconomyBundle, ScorePatternBundle, ScoreRelicBundle,
    ScoreRoundBundle, ScoreTileBundle, all_relic_defs, apply_merchants_eye_discount,
    relic_sell_price_live, relic_shop_price,
};
use crate::core::rules::{BlindKind, RuleModifier};
use crate::core::scoring::{ScoreBreakdown, format_meld_groups, score_sets_with_original};
use crate::core::structure::StructureTriggerMeta;
use crate::core::talisman::TalismanKind;
use crate::core::tile::{Suit, Tile};
use crate::core::tile_pack::TilePackKind;
use crate::core::zodiac::{YakuLevels, ZodiacKind};
use crate::game::event_bus::GameOverReason;
use crate::game::event_bus::{EventBus, GameEvent};
use crate::game::game_mode::{GameMode, HAND_SIZE};
use crate::game::run::{FINAL_ANTE, RunState, relic_eligible_for_shop_stock};

mod blind_planner;
mod blind_sim;
mod export_schema;
mod relic_analytics;
mod reporting;
mod stats;
mod stats_derived;
mod stats_wilson;

pub use reporting::{
    BotConfig, BotOutputFormat, BotOutputTarget, BotRunOptions, BotStrategy, BotTimeoutDiag,
    DEFAULT_BOT_RUN_TIMEOUT_SECS, HeadlessBotBatch, StrategyFile, append_bot_run_to_progress,
    export_play_history_html, run_forced_relic_sweep, run_headless, run_headless_aggregate,
    run_strategy_sweep, run_sweep, seed_progress_from_bot_runs,
};
use stats::clear_payout_breakdown;
pub use stats::{AggregateStats, RunStats, RunTimeoutSnapshot};
use stats::{BotScoringAction, PeakBlindSnapshot};

fn relic_display_name(id: RelicId) -> &'static str {
    all_relic_defs()
        .iter()
        .find(|d| d.id == id)
        .map(|d| d.name)
        .unwrap_or("?")
}

/// Push a relic onto the run's active list and run any first-purchase
/// bookkeeping (counter initialization, capacity recomputation). Shared
/// by the shop buy path and by forced-relic injection at run start.
fn acquire_relic(run: &mut RunState, id: RelicId) {
    run.relics.active.push(id);
    match id {
        RelicId::MeltingIce => {
            run.relic_counters.insert(
                RelicId::MeltingIce,
                crate::core::relic::MELTING_ICE_START_CHIPS,
            );
        }
        RelicId::SilkThread => {
            run.relic_counters.insert(RelicId::SilkThread, 40);
        }
        RelicId::RustlingGooseEgg => {
            run.relic_counters.insert(RelicId::RustlingGooseEgg, 3);
        }
        RelicId::TeaCeremony => {
            run.relic_counters.insert(RelicId::TeaCeremony, 0);
        }
        RelicId::Chrysalis => {
            run.relic_counters.insert(RelicId::MonarchButterfly, 0);
        }
        RelicId::MonarchButterfly => {
            run.relic_counters.insert(RelicId::MonarchButterfly, 0);
        }
        RelicId::Rakuware => {}
        _ => {}
    }
    run.recompute_capacities();
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
    run.apply_gold_reward(refund as i32, bus);
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

fn blind_slot_key(run: &RunState) -> String {
    format!("{:02}-{}", run.ante, run.blind.name())
}

fn blind_log_label(run: &RunState, blind: BlindKind) -> String {
    match blind {
        BlindKind::Boss => {
            let boss_name = run
                .boss
                .upcoming
                .map(|boss| boss.name())
                .unwrap_or("Unknown Boss");
            format!("{} ({})", blind.name(), boss_name)
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
    active_blind: BlindKind,
    blind_turn: Option<u32>,
    started: Instant,
) {
    stats.run_timed_out = true;
    stats.victory = false;
    stats.died_on_ante = run.ante;
    stats.died_on_blind = active_blind;
    stats.death_reason = None;
    stats.timeout_detail = Some(stats::RunTimeoutSnapshot {
        phase: phase.to_string(),
        ante: run.ante,
        blind: blind_log_label(run, active_blind),
        blind_turn,
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

fn current_boss_name(run: &RunState) -> Option<&'static str> {
    run.boss.upcoming.map(|boss| boss.name())
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
    let mut ctx = bot_score_context_base(run, &run.relics, None);
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
        .bot_issues_by_blind
        .entry(blind_log_label(run, run.blind))
        .or_insert(0) += 1;
    if let Some(boss_name) = current_boss_name(run) {
        *stats
            .bot_issues_by_boss
            .entry(boss_name.to_string())
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
) -> ScoreContext<'a> {
    let plays_rem_after = run.plays_remaining.saturating_sub(1);
    let plays_used_after = run.plays_max.saturating_sub(plays_rem_after);
    ScoreContext {
        relic: ScoreRelicBundle {
            roster: relics,
            counters: run.relic_counters.clone(),
        },
        tiles: ScoreTileBundle {
            debuffs: &run.tile_debuffs,
            hand_for_ghost: run.hand(),
        },
        round: ScoreRoundBundle {
            scored_last_turn: run.scored_last_turn,
            plays_used: plays_used_after,
            round_wind: Some(BlindKind::round_wind_for_ante(run.ante)),
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
            gold: run.gold,
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

/// Whether a commit of `new_sets` with `scoring_tile_count` tiles fits the structure bank.
fn structure_commit_fits(
    run: &RunState,
    scoring_tile_count: usize,
    new_sets: &[crate::core::hand::DetectedMeld],
) -> bool {
    let kongs_after = run
        .structure_sets()
        .iter()
        .chain(new_sets.iter())
        .filter(|s| s.kind == MeldKind::Kong)
        .count();
    run.structure_tiles().len() + scoring_tile_count <= HAND_SIZE + kongs_after
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct PlayRank {
    score: u64,
    meld_count: usize,
    tile_count: usize,
}

/// Convert immediate gold (Gilded, flowers, etc.) into shop-comparable score units.
/// Uses the upcoming blind's chip target vs its flat clear payout as the exchange rate.
fn shop_payoff_units(run: &RunState, score: u64, gold: i32) -> i64 {
    let score_part = score as i64;
    if gold <= 0 {
        return score_part;
    }
    let target = run.target_score.max(1) as i64;
    let clear_gold = run.blind.clear_reward().max(1) as i64;
    score_part + gold as i64 * target / clear_gold
}

/// Best play from an explicit candidate mask list (used by [`best_play_in_hand`] and benches).
/// Masks must be enumerated for **structure commits** (see [`RunState::validation_rules_for_structure_commits`]).
fn evaluate_play_masks_payoff(
    run: &RunState,
    hand: &[Tile],
    relics_override: Option<&RelicState>,
    yaku_levels_override: Option<&YakuLevels>,
    masks: &[u32],
) -> Option<(u64, i32, Vec<usize>)> {
    let n = hand.len();
    if !(2..=20).contains(&n) {
        return None;
    }
    let relics = relics_override.unwrap_or(&run.relics);
    let mut ctx = bot_score_context_base(run, relics, yaku_levels_override);
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
        if breakdown.total == 0 && breakdown.flower_gold <= 0 {
            continue;
        }
        let rank = PlayRank {
            score: breakdown.total,
            meld_count: sets.len(),
            tile_count: scoring_tiles.len(),
        };
        let indices = indices_from_play_mask(n, mask);
        let gold = breakdown.flower_gold;
        if best
            .as_ref()
            .map(|(best_rank, _, _)| rank > *best_rank)
            .unwrap_or(true)
        {
            best = Some((rank, gold, indices));
        }
    }
    best.map(|(rank, gold, indices)| (rank.score, gold, indices))
}

pub(crate) fn evaluate_play_masks(
    run: &RunState,
    hand: &[Tile],
    relics_override: Option<&RelicState>,
    yaku_levels_override: Option<&YakuLevels>,
    masks: &[u32],
) -> Option<(u64, Vec<usize>)> {
    evaluate_play_masks_payoff(run, hand, relics_override, yaku_levels_override, masks)
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
    let mut ctx = bot_score_context_base(run, relics, None);
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
        if breakdown.total == 0 && breakdown.flower_gold <= 0 {
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
    top.into_iter().map(|(rank, indices)| (rank.score, indices)).collect()
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
    let mut ctx = bot_score_context_base(run, &run.relics, None);
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

/// Masks that pass meld validation and structure-bank checks (still may score to zero).
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
    let mut ctx = bot_score_context_base(run, relics, yaku_levels_override);
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
) -> Option<(u64, Vec<usize>)> {
    let commit_rules = run.validation_rules_for_structure_commits();
    let masks = enumerate_candidate_play_masks(hand, &commit_rules);
    evaluate_play_masks(run, hand, relics_override, yaku_levels_override, &masks)
}

#[derive(Clone, Copy)]
struct IndexedTile {
    hand_index: usize,
    tile: Tile,
}

fn enumerate_candidate_play_masks(hand: &[Tile], rules: &[RuleModifier]) -> Vec<u32> {
    let mut regular = Vec::with_capacity(hand.len());
    let mut flowers = Vec::new();
    for (hand_index, &tile) in hand.iter().enumerate() {
        let indexed = IndexedTile { hand_index, tile };
        if tile.is_flower() {
            flowers.push(indexed);
        } else {
            regular.push(indexed);
        }
    }
    regular.sort_by_key(|it| it.tile);
    flowers.sort_by_key(|it| it.tile);

    let subset_rules = SubsetRules {
        allow_wrap: rules.contains(&RuleModifier::SequenceWrap),
        no_sequences: rules.contains(&RuleModifier::NoSequences),
        require_honor: rules.contains(&RuleModifier::RequireHonor),
        must_play_five: rules.contains(&RuleModifier::MustPlayFive),
        no_flower_wildcards: rules.contains(&RuleModifier::NoFlowerWildcards),
    };

    let mut masks: rustc_hash::FxHashSet<u32> = rustc_hash::FxHashSet::default();
    enumerate_regular_subsets(&regular, &flowers, 0, subset_rules, 0, &mut masks);
    push_kokushi_play_masks(hand, rules, &mut masks);
    let mut out: Vec<u32> = masks.into_iter().collect();
    out.sort_unstable();
    out
}

/// Advance `pos` (length k, strictly increasing) to the next k-combination of indices `0..n`.
/// Returns `false` when `pos` already was the last combination.
fn next_combination_in_range(pos: &mut [usize], n: usize) -> bool {
    let k = pos.len();
    if k == 0 || k > n {
        return false;
    }
    for i in (0..k).rev() {
        let upper = n - k + i;
        if pos[i] < upper {
            pos[i] += 1;
            for j in i + 1..k {
                pos[j] = pos[j - 1] + 1;
            }
            return true;
        }
    }
    false
}

/// Kokushi Musō uses twelve [`MeldKind::Single`]s and one pair — the meld-based enumerator
/// never selects singletons, so it would miss every Kokushi win. Add every 14-tile orphan
/// subset that [`validate_selection_with_rules`] accepts as Kokushi (or another valid hand).
fn push_kokushi_play_masks(
    hand: &[Tile],
    rules: &[RuleModifier],
    out: &mut rustc_hash::FxHashSet<u32>,
) {
    if rules.contains(&RuleModifier::MustPlayFive) {
        return;
    }
    let pool: Vec<usize> = hand
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.is_flower() && t.is_kokushi_orphan())
        .map(|(i, _)| i)
        .collect();
    let olen = pool.len();
    if olen < 14 {
        return;
    }
    let mut pos: Vec<usize> = (0..14).collect();
    loop {
        let mask: u32 = pos.iter().fold(0u32, |acc, &pi| acc | (1u32 << pool[pi]));
        let tiles: Vec<Tile> = pos.iter().map(|&pi| hand[pool[pi]]).collect();
        if validate_selection_with_rules(&tiles, rules).is_some() {
            out.insert(mask);
        }
        if !next_combination_in_range(&mut pos, olen) {
            break;
        }
    }
}

/// Active rule modifiers for `enumerate_regular_subsets`: captures the
/// boolean flags that shape which plays are legal so the recursion doesn't
/// thread them as separate params.
#[derive(Clone, Copy)]
struct SubsetRules {
    allow_wrap: bool,
    no_sequences: bool,
    require_honor: bool,
    must_play_five: bool,
    no_flower_wildcards: bool,
}

fn enumerate_regular_subsets(
    remaining: &[IndexedTile],
    flowers: &[IndexedTile],
    current_mask: u32,
    rules: SubsetRules,
    current_tile_count: usize,
    out: &mut rustc_hash::FxHashSet<u32>,
) {
    let SubsetRules {
        allow_wrap,
        no_sequences,
        require_honor,
        must_play_five,
        no_flower_wildcards,
    } = rules;
    if current_tile_count > 14 || (must_play_five && current_tile_count > 5) {
        return;
    }

    if remaining.is_empty() {
        emit_leaf_masks(flowers, current_mask, current_tile_count, rules, out);
        return;
    }

    let first = remaining[0];

    // Skip this tile entirely; it won't be part of the scored selection.
    enumerate_regular_subsets(
        &remaining[1..],
        flowers,
        current_mask,
        rules,
        current_tile_count,
        out,
    );

    if remaining.len() >= 2
        && same_face(first.tile, remaining[1].tile)
        && (!require_honor || tiles_have_honor(&[first.tile, remaining[1].tile]))
    {
        enumerate_regular_subsets(
            &remaining[2..],
            flowers,
            current_mask | (1 << first.hand_index) | (1 << remaining[1].hand_index),
            rules,
            current_tile_count + 2,
            out,
        );
    }

    if remaining.len() >= 3
        && same_face(first.tile, remaining[1].tile)
        && same_face(first.tile, remaining[2].tile)
        && (!require_honor || tiles_have_honor(&[first.tile, remaining[1].tile, remaining[2].tile]))
    {
        enumerate_regular_subsets(
            &remaining[3..],
            flowers,
            current_mask
                | (1 << first.hand_index)
                | (1 << remaining[1].hand_index)
                | (1 << remaining[2].hand_index),
            rules,
            current_tile_count + 3,
            out,
        );
    }

    if remaining.len() >= 4
        && same_face(first.tile, remaining[1].tile)
        && same_face(first.tile, remaining[2].tile)
        && same_face(first.tile, remaining[3].tile)
        && (!require_honor
            || tiles_have_honor(&[
                first.tile,
                remaining[1].tile,
                remaining[2].tile,
                remaining[3].tile,
            ]))
    {
        enumerate_regular_subsets(
            &remaining[4..],
            flowers,
            current_mask
                | (1 << first.hand_index)
                | (1 << remaining[1].hand_index)
                | (1 << remaining[2].hand_index)
                | (1 << remaining[3].hand_index),
            rules,
            current_tile_count + 4,
            out,
        );
    }

    let can_use_flower_wildcard = !flowers.is_empty() && !no_flower_wildcards;
    if can_use_flower_wildcard
        && remaining.len() >= 2
        && same_face(first.tile, remaining[1].tile)
        && (!require_honor || tiles_have_honor(&[first.tile, remaining[1].tile]))
    {
        for (flower_idx, flower) in flowers.iter().copied().enumerate() {
            enumerate_regular_subsets(
                &remaining[2..],
                &remove_flower(flowers, flower_idx),
                current_mask
                    | (1 << first.hand_index)
                    | (1 << remaining[1].hand_index)
                    | (1 << flower.hand_index),
                rules,
                current_tile_count + 3,
                out,
            );
        }
    }

    if !no_sequences && first.tile.is_number_tile() && !require_honor {
        for seq in sequence_candidates(remaining, allow_wrap, can_use_flower_wildcard, first) {
            let mut next_mask = current_mask | (1 << first.hand_index);
            let mut remove = vec![0usize];
            for idx in seq.regular_indices {
                next_mask |= 1 << remaining[idx].hand_index;
                remove.push(idx);
            }
            let rest = remove_indices(remaining, &remove);
            if seq.uses_flower {
                for (flower_idx, flower) in flowers.iter().copied().enumerate() {
                    enumerate_regular_subsets(
                        &rest,
                        &remove_flower(flowers, flower_idx),
                        next_mask | (1 << flower.hand_index),
                        rules,
                        current_tile_count + 3,
                        out,
                    );
                }
            } else {
                enumerate_regular_subsets(
                    &rest,
                    flowers,
                    next_mask,
                    rules,
                    current_tile_count + 3,
                    out,
                );
            }
        }
    }
}

fn emit_leaf_masks(
    flowers: &[IndexedTile],
    current_mask: u32,
    current_tile_count: usize,
    rules: SubsetRules,
    out: &mut rustc_hash::FxHashSet<u32>,
) {
    let must_play_five = rules.must_play_five;
    let round_rules = if rules.no_flower_wildcards {
        &[RuleModifier::NoFlowerWildcards][..]
    } else {
        &[][..]
    };
    for extra_mask in flower_meld_partition_masks(flowers, round_rules) {
        let total_mask = current_mask | extra_mask;
        let total_count = total_mask.count_ones() as usize;
        if total_count == 0 {
            continue;
        }
        if must_play_five {
            if total_count == 5 {
                out.insert(total_mask);
            }
        } else if total_count >= current_tile_count {
            out.insert(total_mask);
        }
    }
}

fn flower_meld_partition_masks(flowers: &[IndexedTile], rules: &[RuleModifier]) -> Vec<u32> {
    let indexed: Vec<(usize, u32)> = flowers.iter().map(|f| (f.hand_index, f.tile.id)).collect();
    crate::core::hand::decomposition::flower_meld_partition_masks(&indexed, rules)
}

fn remove_flower(flowers: &[IndexedTile], remove_idx: usize) -> Vec<IndexedTile> {
    flowers
        .iter()
        .enumerate()
        .filter_map(|(idx, flower)| (idx != remove_idx).then_some(*flower))
        .collect()
}

fn same_face(a: Tile, b: Tile) -> bool {
    a.suit == b.suit && a.rank == b.rank
}

fn tiles_have_honor(tiles: &[Tile]) -> bool {
    tiles
        .iter()
        .any(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
}

fn remove_indices(remaining: &[IndexedTile], remove: &[usize]) -> Vec<IndexedTile> {
    let mut remove_flags = vec![false; remaining.len()];
    for &idx in remove {
        remove_flags[idx] = true;
    }
    remaining
        .iter()
        .enumerate()
        .filter_map(|(idx, tile)| (!remove_flags[idx]).then_some(*tile))
        .collect()
}

#[derive(Clone, Copy)]
struct SequenceCandidate {
    regular_indices: [usize; 2],
    uses_flower: bool,
}

fn sequence_candidates(
    remaining: &[IndexedTile],
    allow_wrap: bool,
    can_use_flower: bool,
    first: IndexedTile,
) -> Vec<SequenceCandidate> {
    let mut out = Vec::new();
    push_sequence_candidate(
        remaining,
        first.tile.suit,
        [first.tile.rank + 1, first.tile.rank + 2],
        false,
        &mut out,
    );
    if can_use_flower {
        push_sequence_candidate(
            remaining,
            first.tile.suit,
            [first.tile.rank + 1, first.tile.rank + 2],
            true,
            &mut out,
        );
    }

    if allow_wrap {
        for ranks in wrap_sequence_ranks(first.tile.rank) {
            push_sequence_candidate(remaining, first.tile.suit, *ranks, false, &mut out);
            if can_use_flower {
                push_sequence_candidate(remaining, first.tile.suit, *ranks, true, &mut out);
            }
        }
    }

    out
}

fn push_sequence_candidate(
    remaining: &[IndexedTile],
    suit: Suit,
    ranks: [u8; 2],
    allow_one_missing: bool,
    out: &mut Vec<SequenceCandidate>,
) {
    let mut found = Vec::new();
    for &rank in &ranks {
        let next = remaining
            .iter()
            .enumerate()
            .skip(1)
            .find_map(|(idx, tile)| {
                (tile.tile.suit == suit && tile.tile.rank == rank).then_some(idx)
            });
        if let Some(idx) = next {
            found.push(idx);
        } else if !allow_one_missing {
            return;
        }
    }

    match found.len() {
        2 => out.push(SequenceCandidate {
            regular_indices: [found[0], found[1]],
            uses_flower: false,
        }),
        1 if allow_one_missing => out.push(SequenceCandidate {
            regular_indices: [found[0], found[0]],
            uses_flower: true,
        }),
        _ => {}
    }
}

fn wrap_sequence_ranks(rank: u8) -> &'static [[u8; 2]] {
    match rank {
        1 => &[[9, 2], [8, 9]],
        2 => &[[9, 1]],
        8 => &[[9, 1]],
        9 => &[[8, 1], [1, 2]],
        _ => &[],
    }
}

/// Search for the highest-scoring playable selection in the current hand.
/// Returns `(score, indices)`, or `None` if no positive-scoring play exists.
pub fn pick_best_play(run: &RunState) -> Option<(u64, Vec<usize>)> {
    best_play_in_hand(run, run.hand(), None, None)
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

/// Count masks that pass validation + structure-bank checks (before scoring).
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
    best_play_in_hand(run, &new_hand, None, None)
        .map(|(s, _)| s)
        .unwrap_or(0)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlayBlindOutcome {
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
) -> Option<PlayBlindOutcome> {
    let events: Vec<GameEvent> = bus.drain().collect();
    for ev in events {
        match ev {
            GameEvent::RoundComplete {
                payout,
                reached_target: true,
            } => {
                run.apply_gold_reward(payout.total as i32, Some(bus));
            }
            GameEvent::RoundComplete {
                reached_target: false,
                ..
            } => {
                run.forfeit_current_blind_second_wind(bus);
                return Some(PlayBlindOutcome::SecondWindForfeit);
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
                *stats
                    .transformations_successor
                    .entry(rname)
                    .or_insert(0) += 1;
                *stats.relics_picked.entry(rname).or_insert(0) += 1;
            }
            _ => {}
        }
    }
    None
}

/// Play the current blind to completion. Returns outcome and the number of **decision
/// turns** taken this blind (incremented once per loop iteration after failure checks).
fn play_blind(
    run: &mut RunState,
    stats: &mut RunStats,
    log: bool,
    deadline: Option<Instant>,
    scoring_log: &mut Vec<BotScoringAction>,
    strategy: &BotStrategy,
) -> (PlayBlindOutcome, u32) {
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
            return (PlayBlindOutcome::Cleared, turn);
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
            return (PlayBlindOutcome::LostRun, turn);
        }
        if run_deadline_expired(deadline) {
            return (PlayBlindOutcome::TimedOut, turn);
        }
        turn += 1;
        let slot = blind_slot_key(run);
        *stats.turns_by_blind_slot.entry(slot).or_insert(0) += 1;
        stats.turns_total += 1;
        stats.peak_hand_size = stats.peak_hand_size.max(run.hand().len() as u32);

        bot_log!(
            log,
            "    turn {:>2}: score {}/{} | plays {} | discards {} | hand {} | gold {}",
            turn,
            run.round_score,
            run.target_score,
            run.plays_remaining,
            run.discards_remaining,
            run.hand().len(),
            run.gold
        );

        if strategy.blind_planner_depth == 0 && use_bot_consumables(run, stats, log) {
            continue;
        }

        if strategy.blind_planner_depth >= 1 {
            match blind_planner::execute_planned_turn(
                run,
                stats,
                log,
                strategy.blind_planner_depth,
                scoring_log,
                &mut bus,
            ) {
                Some(blind_planner::PlannedTurnOutcome::Continued) => continue,
                Some(blind_planner::PlannedTurnOutcome::SecondWindForfeit) => {
                    return (PlayBlindOutcome::SecondWindForfeit, turn);
                }
                Some(blind_planner::PlannedTurnOutcome::Failed) | None => {}
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
                == Some(PlayBlindOutcome::SecondWindForfeit)
            {
                return (PlayBlindOutcome::SecondWindForfeit, turn);
            }
            let structure_delta = run.round_score.saturating_sub(score_before_structure);
            if structure_delta > 0 {
                scoring_log.push(BotScoringAction {
                    kind: "structure".into(),
                    points: structure_delta,
                    tiles: format_meld_groups(&cash_in_tiles, &cash_in_sets),
                });
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
        if can_discard {
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
                    .discards_by_blind_slot
                    .entry(blind_slot_key(run))
                    .or_insert(0) += 1;
                if drain_post_action_bus(run, &mut bus, stats)
                    == Some(PlayBlindOutcome::SecondWindForfeit)
                {
                    return (PlayBlindOutcome::SecondWindForfeit, turn);
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
            let bank_before_commit = run.structure_tiles().to_vec();
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
                return (PlayBlindOutcome::LostRun, turn);
            }
            stats.plays_used += 1;
            if drain_post_action_bus(run, &mut bus, stats)
                == Some(PlayBlindOutcome::SecondWindForfeit)
            {
                return (PlayBlindOutcome::SecondWindForfeit, turn);
            }
            let play_delta = run.round_score.saturating_sub(score_before);
            if play_delta > 0 {
                let tiles = if run.structure_tiles().is_empty() {
                    let mut full = bank_before_commit;
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
                scoring_log.push(BotScoringAction {
                    kind: "play".into(),
                    points: play_delta,
                    tiles,
                });
            }
            continue;
        }

        // No positive-scoring play and no strategic discard helped — random discard
        // as a last-resort shake-up before busting.
        if run.discards_remaining == 0 {
            record_terminal_hand_issue(stats, run, TerminalIssueCause::NoDiscardsRemaining);
            bot_log!(log, "      action: no discards remaining");
            return (PlayBlindOutcome::LostRun, turn);
        }
        run.clear_selection();
        let hand_n = run.hand().len();
        if hand_n == 0 {
            record_terminal_hand_issue(stats, run, TerminalIssueCause::EmptyHand);
            bot_log!(log, "      action: hand empty, cannot continue");
            return (PlayBlindOutcome::LostRun, turn);
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
            .discards_by_blind_slot
            .entry(blind_slot_key(run))
            .or_insert(0) += 1;
        if drain_post_action_bus(run, &mut bus, stats) == Some(PlayBlindOutcome::SecondWindForfeit)
        {
            return (PlayBlindOutcome::SecondWindForfeit, turn);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BotStrategy, EventBus, RunStats, ShopMarginalBase, best_play_in_hand,
        enumerate_candidate_play_masks, pick_best_play, relic_hold_value_with_base,
        relic_marginal_value_with_base, relic_shop_offer_value_with_base,
        remaining_antes_including_current, scale_long_term_value_for_ante,
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
    use crate::game::run::{FINAL_ANTE, RunState};

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
            let kongs_after = run
                .structure_sets()
                .iter()
                .chain(sets.iter())
                .filter(|s| s.kind == MeldKind::Kong)
                .count();
            if run.structure_tiles().len() + scoring_tiles.len() > HAND_SIZE + kongs_after {
                continue;
            }
            let mut merged_sets = run.structure_sets().to_vec();
            merged_sets.extend(sets.iter().cloned());
            let mut merged_tiles = run.structure_tiles().to_vec();
            merged_tiles.extend(scoring_tiles.iter().copied());
            let mut ctx = super::bot_score_context_base(run, &run.relics, None);
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
    fn bot_skips_structure_commits_that_overflow_the_bank() {
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
            "bot should not choose a play that the structure bank would reject"
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
        let new_best = best_play_in_hand(&run, run.hand(), None, None);
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
            best_play_in_hand(&run, run.hand(), None, None),
            brute_force_best_play_in_hand(&run)
        );
    }

    #[test]
    fn wildflower_marginal_value_is_positive() {
        let run = scoring_test_run();
        assert!(
            talisman_marginal_value(&run, TalismanKind::Wildflower) > 0,
            "wildflower should score well on sampled hands"
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
    fn gilded_talisman_shop_value_counts_gold_from_scored_melds() {
        let run = scoring_test_run();
        let pearl = talisman_marginal_value(&run, TalismanKind::Pearl);
        let gilded = talisman_marginal_value(&run, TalismanKind::Gilded);
        assert!(
            gilded > 0,
            "gilded should contribute shop value via gold (pearl={pearl}, gilded={gilded})"
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
    fn remaining_antes_helper_tracks_final_ante() {
        assert_eq!(remaining_antes_including_current(1), FINAL_ANTE);
        assert_eq!(remaining_antes_including_current(FINAL_ANTE), 1);
        assert_eq!(remaining_antes_including_current(FINAL_ANTE + 1), 0);
    }

    #[test]
    fn long_term_value_scales_down_late_in_the_run() {
        let baseline = FINAL_ANTE as i32 * 10;
        assert_eq!(scale_long_term_value_for_ante(baseline, 1), baseline);
        assert_eq!(scale_long_term_value_for_ante(baseline, FINAL_ANTE), 10);
        assert!(
            scale_long_term_value_for_ante(baseline, FINAL_ANTE)
                < scale_long_term_value_for_ante(baseline, 2)
        );
    }

    #[test]
    fn aggregate_stats_do_not_count_victories_as_deaths() {
        let mut agg = super::AggregateStats::default();
        let win = super::RunStats {
            victory: true,
            died_on_ante: FINAL_ANTE,
            antes_cleared: FINAL_ANTE,
            ..Default::default()
        };
        agg.record(&win);

        assert_eq!(agg.victories, 1);
        assert_eq!(agg.max_ante_reached, FINAL_ANTE);
        assert!(agg.deaths_by_ante.is_empty());
        assert!(agg.deaths_by_blind.is_empty());
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
            RelicId::GreenLuck,
        ];
        assert!(run.relics.is_full());
        let base = ShopMarginalBase::new(&run);
        let mut sell_index = None;
        let _swap_mv = relic_shop_offer_value_with_base(
            &run,
            RelicId::JadeSerpent,
            &base,
            &mut sell_index,
        );
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
        run.gold = 0;
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
        let sold = sell_underperforming_relics(&mut run, &mut stats, false, &strategy, &base, &mut bus);
        assert_eq!(sold, 1);
        assert_eq!(stats.relics_sold, 1);
        assert_eq!(run.relics.active, vec![RelicId::PairPower]);
        assert!(run.gold > 0);
    }
}

#[derive(Clone, Copy, Debug)]
enum ShopOffer {
    Relic(RelicId),
    Zodiac(ZodiacKind),
    Talisman(TalismanKind),
    Pack(TilePackKind),
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
    packs.push(extra_pack);
    let mut wall = Wall::from_filtered_with_packs(
        &run.removed_tile_ids,
        &packs,
        &run.tile_enhancements,
        run.relics.has(RelicId::StrengthInNumbers),
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

fn best_play_score_for_hand(
    run: &RunState,
    hand: &[Tile],
    relics_override: Option<&RelicState>,
    yaku_levels_override: Option<&YakuLevels>,
) -> u64 {
    best_play_in_hand(run, hand, relics_override, yaku_levels_override)
        .map(|(s, _)| s)
        .unwrap_or(0)
}

/// Best-play value for shop/talisman estimates: blind chips plus gold converted at
/// target÷clear_reward so Gilded and other gold sources compete with score upgrades.
fn best_play_shop_value_for_hand(
    run: &RunState,
    hand: &[Tile],
    relics_override: Option<&RelicState>,
    yaku_levels_override: Option<&YakuLevels>,
) -> i64 {
    let commit_rules = run.validation_rules_for_structure_commits();
    let masks = enumerate_candidate_play_masks(hand, &commit_rules);
    evaluate_play_masks_payoff(run, hand, relics_override, yaku_levels_override, &masks)
        .map(|(score, gold, _)| shop_payoff_units(run, score, gold))
        .unwrap_or(0)
}

/// Synthetic random hands per shop valuation (plus the real current hand).
/// Late antes use fewer samples — each one runs `best_play_in_hand`.
fn relic_eval_sample_count(ante: u32) -> usize {
    if ante >= FINAL_ANTE {
        2
    } else if ante >= 5 {
        3
    } else {
        4
    }
}

/// Cached hands and baseline best-play scores for one shop "next purchase" iteration.
struct ShopMarginalBase {
    hands: Vec<Vec<Tile>>,
    baseline: Vec<u64>,
}

impl ShopMarginalBase {
    fn new(run: &RunState) -> Self {
        let n = relic_eval_sample_count(run.ante);
        let size = crate::core::boss::effective_hand_size(run);
        let mut hands = Vec::with_capacity(n + 1);
        hands.push(run.hand().to_vec());
        for _ in 0..n {
            hands.push(sample_random_hand(size));
        }
        let baseline: Vec<u64> = hands
            .iter()
            .map(|h| best_play_score_for_hand(run, h, None, None))
            .collect();
        Self { hands, baseline }
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
fn remaining_antes_including_current(ante: u32) -> u32 {
    if ante > FINAL_ANTE {
        0
    } else {
        FINAL_ANTE - ante + 1
    }
}

fn scale_long_term_value_for_ante(raw_value: i32, ante: u32) -> i32 {
    if raw_value <= 0 {
        return raw_value;
    }
    let remaining = remaining_antes_including_current(ante) as i64;
    let scaled = (raw_value as i64 * remaining + FINAL_ANTE as i64 - 1) / FINAL_ANTE as i64;
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

    let mut delta_sum: i64 = 0;
    for (h, hand) in base.hands.iter().enumerate() {
        delta_sum += best_play_score_for_hand(run, hand, Some(&hypothetical), None) as i64
            - base.baseline[h] as i64;
    }
    let sample_count = base.hands.len() as i64;
    scale_long_term_value_for_ante((delta_sum / sample_count) as i32, run.ante)
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

    let mut delta_sum: i64 = 0;
    for (h, hand) in base.hands.iter().enumerate() {
        delta_sum += best_play_score_for_hand(run, hand, Some(&hypothetical), None) as i64
            - base.baseline[h] as i64;
    }
    let sample_count = base.hands.len() as i64;
    scale_long_term_value_for_ante((delta_sum / sample_count) as i32, run.ante)
}

fn relic_hold_value_with_base(run: &RunState, index: usize, base: &ShopMarginalBase) -> i32 {
    if index >= run.relics.active.len() {
        return 0;
    }
    let mut without = run.relics.clone();
    without.active.remove(index);

    let mut delta_sum: i64 = 0;
    for hand in &base.hands {
        delta_sum += best_play_score_for_hand(run, hand, Some(&run.relics), None) as i64
            - best_play_score_for_hand(run, hand, Some(&without), None) as i64;
    }
    let sample_count = base.hands.len() as i64;
    if sample_count == 0 {
        return 0;
    }
    scale_long_term_value_for_ante((delta_sum / sample_count) as i32, run.ante)
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
        run.gold + refund >= price
    } else if run.relics.is_full() && !relic_can_expand_inventory(candidate) {
        false
    } else {
        run.gold >= price
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
        delta_sum += best_play_score_for_hand(run, hand, None, Some(&hypothetical)) as i64
            - base.baseline[h] as i64;
    }
    let sample_count = base.hands.len() as i64;
    scale_long_term_value_for_ante((delta_sum / sample_count) as i32, run.ante)
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
    let base = best_play_score_for_hand(run, hand, None, None) as i64;
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
            let after = best_play_score_for_hand(run, &simulated, None, None) as i64;
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
            let after = best_play_score_for_hand(run, &simulated, None, None) as i64;
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
                let after = best_play_score_for_hand(run, &simulated, None, None) as i64;
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
                let after = best_play_score_for_hand(run, &simulated, None, None) as i64;
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
    let remaining_blinds = (remaining_antes_including_current(ante) as i64 * 3).max(1);
    (raw_avg as i64 / remaining_blinds) as i32
}

pub(crate) fn buff_talisman_lift_on_hand(run: &RunState, hand: &[Tile], talisman: TalismanKind) -> i32 {
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
        return scale_long_term_value_for_ante(raw_avg, run.ante);
    }
    let mut mv = talisman_one_shot_shop_value(raw_avg, run.ante);
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
fn pack_marginal_value(run: &RunState, kind: TilePackKind) -> i32 {
    let mut delta_sum: i64 = 0;
    let mut sample_count: i64 = 0;
    // Baseline samples the run's *current* wall (with any already-owned
    // packs mixed in); comparison samples the wall with the prospective
    // pack added. This captures diminishing returns — a second Flowers
    // Pack is worth much less than the first.
    let pack_iters = relic_eval_sample_count(run.ante).saturating_add(1);
    for _ in 0..pack_iters {
        let base_wall = Wall::from_filtered_with_packs(
            &run.removed_tile_ids,
            &run.tile_packs,
            &run.tile_enhancements,
            run.relics.has(RelicId::StrengthInNumbers),
        );
        let target = crate::core::boss::effective_hand_size(run);
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
            crate::core::boss::effective_hand_size(run),
        );
        let base_score = best_play_score_for_hand(run, &base_hand, None, None) as i64;
        let enriched = best_play_score_for_hand(run, &with, None, None) as i64;
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
    scale_long_term_value_for_ante(avg, run.ante)
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
    qilin_ribbon_unlocked: bool,
    bus: &mut crate::game::event_bus::EventBus,
) -> ShopVisitOutcome {
    // Consume tag-granted shop modifiers (headless analogue of ShopScene::new).
    let extra_relics: usize = if run.tag_rich_stock { 2 } else { 0 };
    let patron_gift = run.tag_patron_gift;
    // Free reroll is a no-op for the bot (it doesn't reroll).
    run.tag_free_reroll = false;
    run.tag_patron_gift = false;
    run.tag_rich_stock = false;

    let defs = all_relic_defs();
    let pool_x = run.relic_shop_pool_extinction();
    let mut pool: Vec<RelicId> = defs
        .iter()
        .filter(|d| relic_eligible_for_shop_stock(d.id, &run.relics, &run.available_relics, pool_x))
        .map(|d| d.id)
        .collect();
    pool.shuffle(&mut rand::rng());
    let mut rng = rand::rng();
    const MAX_RIBBONS: usize = 4;
    const MAX_RELICS: usize = 6;
    let mut n_relics = rng.random_range(0..=MAX_RELICS) + extra_relics;
    let mut n_zodiacs = rng.random_range(1..=MAX_RIBBONS);
    let mut n_talismans = rng.random_range(1..=MAX_RIBBONS);
    while n_zodiacs + n_talismans > MAX_RIBBONS {
        if n_talismans >= n_zodiacs {
            n_talismans -= 1;
        } else {
            n_zodiacs -= 1;
        }
    }
    while n_relics + n_zodiacs + n_talismans < 2 {
        let relics_room = n_relics < MAX_RELICS;
        let ribbons_room = n_zodiacs + n_talismans < MAX_RIBBONS;
        let zodiacs_room = ribbons_room && n_zodiacs < MAX_RIBBONS;
        let talismans_room = ribbons_room && n_talismans < MAX_RIBBONS;
        let mut choices = Vec::with_capacity(3);
        if relics_room {
            choices.push(0u8);
        }
        if zodiacs_room {
            choices.push(1u8);
        }
        if talismans_room {
            choices.push(2u8);
        }
        if choices.is_empty() {
            break;
        }
        match choices[rng.random_range(0..choices.len())] {
            0 => n_relics += 1,
            1 => n_zodiacs += 1,
            _ => n_talismans += 1,
        }
    }

    let mut zodiac_pool: Vec<ZodiacKind> = ZodiacKind::all().to_vec();
    if !qilin_ribbon_unlocked {
        zodiac_pool.retain(|&z| z != ZodiacKind::Qilin);
    }
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

    let mut free_relic = patron_gift;
    bot_log!(
        log,
        "    shop: gold {} | relic slots {}/{} | consumables {}/{} | offerings {:?}{}",
        run.gold,
        run.relics.active.len(),
        run.relics.max_slots,
        run.consumables.items.len(),
        run.consumables.capacity,
        shop,
        if free_relic {
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
            let shop_base = ShopMarginalBase::new(run);
            if sell_underperforming_relics(run, stats, log, strategy, &shop_base, bus) > 0 {
                continue;
            }
            bot_log!(log, "    shop: leaving ({})", "no offerings left");
            break;
        }
        let shop_base = ShopMarginalBase::new(run);
        if sell_underperforming_relics(run, stats, log, strategy, &shop_base, bus) > 0 {
            continue;
        }
        // Find the best affordable offer with positive marginal value.
        let mut best: Option<(usize, i32, Option<usize>)> = None;
        for (i, offer) in shop.iter().copied().enumerate() {
            let price = match offer {
                ShopOffer::Relic(id) => {
                    if free_relic {
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
                    if price_i32 > run.gold {
                        continue;
                    }
                    zodiac_marginal_value_with_base(run, zodiac, &shop_base)
                }
                ShopOffer::Talisman(kind) => {
                    if price_i32 > run.gold {
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
                    if price_i32 > run.gold {
                        continue;
                    }
                    pack_marginal_value(run, kind)
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
            if best
                .as_ref()
                .map(|(_, b, _)| mv > *b)
                .unwrap_or(true)
            {
                best = Some((i, mv, sell_index));
            }
        }
        let Some((idx, marginal_value, sell_index)) = best else {
            let shop_base = ShopMarginalBase::new(run);
            if sell_underperforming_relics(run, stats, log, strategy, &shop_base, bus) > 0 {
                continue;
            }
            bot_log!(log, "    shop: no positive-value affordable purchase");
            break;
        };
        let offer = shop.remove(idx);
        match offer {
            ShopOffer::Relic(id) => {
                let price = if free_relic {
                    0
                } else {
                    run.mode.scale_shop_price(relic_shop_price(id, &run.relics))
                };
                free_relic = false;
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
                run.apply_gold_delta(-(price as i32), Some(bus));
                acquire_relic(run, id);
                stats.relics_bought += 1;
                let rname = relic_display_name(id);
                relic_analytics::record_marginal_buy(stats, rname, marginal_value);
                *stats.relics_picked.entry(rname).or_insert(0) += 1;
                if run.ante <= crate::bot::stats::RELIC_SHOP_TIMING_EARLY_ANTE_MAX {
                    *stats.relics_picked_shop_early.entry(rname).or_insert(0) += 1;
                } else {
                    *stats.relics_picked_shop_late.entry(rname).or_insert(0) += 1;
                }
                stats.gold_spent += price;
                bot_log!(
                    log,
                    "    shop: bought {:?} for {} (marginal value {}, gold now {})",
                    id,
                    price,
                    marginal_value,
                    run.gold
                );
            }
            ShopOffer::Zodiac(zodiac) => {
                let price = run.mode.scale_shop_price(apply_merchants_eye_discount(
                    ZodiacKind::shop_price(),
                    &run.relics,
                ));
                run.apply_gold_delta(-(price as i32), Some(bus));
                let new_level = run.yaku_levels.level_up_for_zodiac(zodiac);
                stats.gold_spent += price;
                *stats.zodiacs_picked.entry(zodiac.name()).or_insert(0) += 1;
                bot_log!(
                    log,
                    "    shop: bought zodiac {:?} for {} (marginal value {}, level {}, gold now {})",
                    zodiac,
                    price,
                    marginal_value,
                    new_level,
                    run.gold
                );
            }
            ShopOffer::Talisman(kind) => {
                let price = run
                    .mode
                    .scale_shop_price(apply_merchants_eye_discount(kind.shop_price(), &run.relics));
                run.apply_gold_delta(-(price as i32), Some(bus));
                run.consumables.items.push(Consumable::Talisman(kind));
                stats.gold_spent += price;
                *stats.talismans_picked.entry(kind.name()).or_insert(0) += 1;
                bot_log!(
                    log,
                    "    shop: bought talisman {:?} for {} (marginal value {}, gold now {})",
                    kind,
                    price,
                    marginal_value,
                    run.gold
                );
            }
            ShopOffer::Pack(kind) => {
                let price = run
                    .mode
                    .scale_shop_price(apply_merchants_eye_discount(kind.shop_price(), &run.relics));
                run.apply_gold_delta(-(price as i32), Some(bus));
                // Mirror the real shop: pre-stamp any enhancement from the
                // pack kind onto the tiles' IDs, then append the pack. The
                // wall gets rebuilt with these packs at the start of every
                // round (`advance_round` calls `from_filtered_with_packs`).
                let pack_idx = run.tile_packs.len();
                let start_id = crate::core::tile_pack::PACK_TILE_ID_BASE
                    + (pack_idx as u32) * crate::core::tile_pack::PACK_ID_STRIDE;
                if let Some(enh) = kind.pre_enhancement() {
                    for t in kind.generate_tiles(start_id) {
                        run.tile_enhancements.insert(t.id, enh);
                    }
                }
                run.tile_packs.push(kind);
                stats.gold_spent += price;
                *stats.packs_picked.entry(kind.name()).or_insert(0) += 1;
                bot_log!(
                    log,
                    "    shop: bought pack {:?} for {} (marginal value {}, gold now {})",
                    kind,
                    price,
                    marginal_value,
                    run.gold
                );
            }
        }
    }
    ShopVisitOutcome::Completed
}

/// Decide whether to skip the upcoming non-Boss blind. We skip when the bot can
/// reasonably expect to clear the blind anyway *and* it can't, so the gold reward
/// from skipping is more valuable than the gold reward from clearing. Specifically:
/// the bot's projected total score (best play × plays_remaining) must comfortably
/// exceed the blind's target so we'd be wasting plays clearing a trivially-easy
/// blind. Boss blinds can never be skipped.
fn should_skip_blind(run: &RunState, blind: BlindKind, strategy: &BotStrategy) -> bool {
    if matches!(blind, BlindKind::Boss) {
        return false;
    }
    let target = run.blind_score_target(blind);
    let best = pick_best_play(run).map(|(s, _)| s).unwrap_or(0);
    if best == 0 {
        return false;
    }
    // Optimistic projection: assume the bot can repeat its current best play for
    // every remaining play (ignores discard refresh).
    let projected = best.saturating_mul(run.plays_remaining as u64);
    // Only skip if we'd over-shoot by `skip_threshold_multiplier` × target
    // (default 2.0). Higher = never skip; lower = skip aggressively.
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
    let mode = config.into_mode();
    let mut run = RunState::new(mode);
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
        acquire_relic(&mut run, id);
        bot_log!(log, "  forced relic injection: {:?}", id);
    }

    bot_log!(
        log,
        "== bot run{} start: ante {} | blind {} | target {} | gold {} ==",
        run_number.map(|n| format!(" #{n}")).unwrap_or_default(),
        run.ante,
        blind_log_label(&run, run.upcoming_blind),
        run.target_score,
        run.gold
    );

    loop {
        if run.is_run_complete() {
            stats.victory = true;
            stats.died_on_ante = FINAL_ANTE;
            bot_log!(
                log,
                "== bot run complete: victory at ante {} ==",
                FINAL_ANTE
            );
            break;
        }
        if run_deadline_expired(deadline) {
            record_timeout_snapshot(&mut stats, &run, "outer", run.upcoming_blind, None, started);
            break;
        }
        let blind = run.upcoming_blind;
        bot_log!(
            log,
            "  ante {} | blind {} | target {} | gold {} | relics {}",
            run.ante,
            blind_log_label(&run, blind),
            run.target_score,
            run.gold,
            run.relics.active.len()
        );

        // Skip strategy: bank gold on Small/Big when projected score comfortably
        // overshoots the target. Tag rewards replace flat gold — apply them
        // the same way the pick-blind scene does.
        if should_skip_blind(&run, blind, &strategy) {
            bot_log!(log, "    action: skip {}", blind.name());
            if let Some(tag) = run.tag_for_blind(blind) {
                let gold_before = run.gold;
                run.apply_tag(tag, Some(&mut bus));
                let gold_after = run.gold;
                let realized_gold = gold_after.saturating_sub(gold_before).max(0) as u32;
                stats.gold_from_skip_tags += realized_gold;
                stats.skip_tag_gold_value += tag.gold_value();
                *stats.skipped_tags.entry(tag.name()).or_insert(0) += 1;
                for (zodiac, _, _) in &run.pending_zodiac_celebrations {
                    *stats.zodiacs_picked.entry(zodiac.name()).or_insert(0) += 1;
                }
            }
            run.skip_to_next_blind();
            stats.blinds_skipped += 1;
            continue;
        }

        if run_deadline_expired(deadline) {
            record_timeout_snapshot(&mut stats, &run, "outer", blind, None, started);
            break;
        }

        stats.total_target_score += run.target_score as u64;
        run.apply_blind(blind, Some(&mut bus));
        let boss_for_this_blind = if matches!(blind, BlindKind::Boss) {
            current_boss_name(&run).map(|name| name.to_string())
        } else {
            None
        };
        if let Some(boss_name) = &boss_for_this_blind {
            stats.boss_faced.insert(boss_name.clone(), 1);
        }
        let mut blind_scoring: Vec<BotScoringAction> = Vec::new();
        let (outcome, blind_turns) =
            play_blind(
                &mut run,
                &mut stats,
                log,
                deadline,
                &mut blind_scoring,
                &strategy,
            );
        let blind_score = run.round_score;
        if matches!(blind, BlindKind::Boss) {
            *stats.boss_score_by_ante.entry(run.ante).or_insert(0) += blind_score;
            *stats.boss_attempts_by_ante.entry(run.ante).or_insert(0) += 1;
        }
        stats.total_score += blind_score;
        if blind_score > stats.peak_blind_score {
            stats.peak_blind_detail = Some(PeakBlindSnapshot {
                blind_slot: format!("{:02}-{}", run.ante, blind.name()),
                blind_label: blind_log_label(&run, blind),
                target_score: run.target_score,
                total_score: blind_score,
                relics: run
                    .relics
                    .active
                    .iter()
                    .map(|&id| relic_display_name(id).to_string())
                    .collect(),
                scoring_actions: blind_scoring,
            });
        }
        stats.peak_blind_score = stats.peak_blind_score.max(blind_score);
        stats.died_on_ante = run.ante;
        stats.died_on_blind = blind;
        if let Some(boss_name) = &boss_for_this_blind
            && outcome == PlayBlindOutcome::Cleared
        {
            stats.boss_beaten.insert(boss_name.clone(), 1);
        }
        if outcome == PlayBlindOutcome::TimedOut {
            record_timeout_snapshot(
                &mut stats,
                &run,
                "playing_blind",
                blind,
                Some(blind_turns),
                started,
            );
            stats.final_gold = run.gold;
            bot_log!(
                log,
                "== bot run end: wall-clock timeout during {} ==",
                blind_log_label(&run, blind)
            );
            break;
        }
        if outcome == PlayBlindOutcome::LostRun {
            stats.final_gold = run.gold;
            bot_log!(
                log,
                "== bot run end: died on ante {} {} with gold {} ==",
                run.ante,
                blind_log_label(&run, blind),
                run.gold
            );
            break;
        }
        if outcome == PlayBlindOutcome::SecondWindForfeit {
            stats.second_wind_forfeits += 1;
            bot_log!(
                log,
                "  second wind — forfeited blind; gold {} | next {:?}",
                run.gold,
                run.upcoming_blind
            );
            let qilin_unlocked = stats.kokushi_musou_scored();
            match visit_shop(
                &mut run,
                &mut stats,
                log,
                &strategy,
                deadline,
                qilin_unlocked,
                &mut bus,
            ) {
                ShopVisitOutcome::Completed => {}
                ShopVisitOutcome::TimedOut => {
                    record_timeout_snapshot(
                        &mut stats,
                        &run,
                        "shop",
                        run.upcoming_blind,
                        None,
                        started,
                    );
                    stats.final_gold = run.gold;
                    break;
                }
            }
            continue;
        }
        stats.blinds_cleared += 1;
        let blind_overscore = run.round_score.saturating_sub(run.target_score as u64);
        stats.total_overscore += blind_overscore;
        let slot_key = format!("{:02}-{}", run.ante, blind.name());
        *stats
            .turns_cleared_by_slot
            .entry(slot_key.clone())
            .or_insert(0) += blind_turns;
        *stats.overscore_by_slot.entry(slot_key.clone()).or_insert(0) += blind_overscore;
        *stats.cleared_by_slot.entry(slot_key).or_insert(0) += 1;
        if matches!(blind, BlindKind::Boss) {
            stats.antes_cleared += 1;
        }
        let payout = clear_payout_breakdown(&run);
        stats.gold_from_clears += payout.total;
        stats.gold_from_clear_base += payout.base_reward;
        stats.gold_from_unused_plays += payout.unused_play_bonus;
        stats.gold_from_interest += payout.interest;
        stats.gold_from_clear_relics += payout.relic_bonus;
        stats.gold_clear_green_luck += payout.green_luck_bonus;
        stats.gold_clear_gold_idol += payout.gold_idol_bonus;
        stats.gold_clear_jade_abacus += payout.jade_abacus_bonus;
        stats.gold_clear_patience += payout.patience_bonus;

        bot_log!(
            log,
            "  blind {} cleared; advancing with gold {} and total score {}",
            blind_log_label(&run, blind),
            run.gold,
            stats.total_score
        );

        run.advance_round(&mut bus);

        // Shop visit happens after advance_round (matching Shop → PickBlind scene
        // flow), so we evaluate purchases against the freshly-drawn next hand.
        let qilin_unlocked = stats.kokushi_musou_scored();
        match visit_shop(
            &mut run,
            &mut stats,
            log,
            &strategy,
            deadline,
            qilin_unlocked,
            &mut bus,
        ) {
            ShopVisitOutcome::Completed => {}
            ShopVisitOutcome::TimedOut => {
                record_timeout_snapshot(
                    &mut stats,
                    &run,
                    "shop",
                    run.upcoming_blind,
                    None,
                    started,
                );
                stats.final_gold = run.gold;
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
    stats.final_gold = run.gold;
    bot_log!(log, "== bot run stats: {:?} ==", stats);
    (run, stats)
}
