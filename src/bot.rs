//! Headless bot runner used for tuning balance.
//!
//! The bot picks the highest-scoring valid play available in its current hand each turn.
//! Between turns it strategically discards isolated tiles via 1-step rollout when the
//! best play falls below the pace needed to clear. Between blinds it values relics and
//! consumables, visits the shop, buys the most useful affordable upgrade, and skips
//! Small/Big blinds when its expected score comfortably exceeds the target.
//!
//! Run with: `cargo run --release -- --bot 200`

use rand::RngExt;
use rand::seq::SliceRandom;
use rayon::prelude::*;
use serde::Serialize;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::core::consumable::Consumable;
use crate::core::deck::Wall;
use crate::core::hand::{SetKind, detect_all_sets, validate_selection_with_rules};
use crate::core::relic::{RelicId, RelicState, ScoreContext, all_relic_defs, relic_shop_price};
use crate::core::rules::{BlindKind, RuleModifier};
use crate::core::scoring::score_sets_with_original;
use crate::core::structure::StructureTriggerMeta;
use crate::core::talisman::TalismanKind;
use crate::core::tile::{Suit, Tile};
use crate::core::tile_pack::TilePackKind;
use crate::core::zodiac::{YakuLevels, ZodiacKind};
use crate::game::event_bus::GameOverReason;
use crate::game::event_bus::{EventBus, GameEvent};
use crate::game::game_mode::GameMode;
use crate::game::run::{FINAL_ANTE, RunState};

mod reporting;
mod stats;

pub use reporting::{
    BotConfig, BotRunOptions, BotStrategy, StrategyFile, run_forced_relic_sweep, run_headless,
    run_headless_aggregate, run_strategy_sweep, run_sweep,
};
use stats::clear_payout_breakdown;
pub use stats::{AggregateStats, RunStats};

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
            run.relic_counters.insert(RelicId::MeltingIce, 80);
        }
        RelicId::SilkThread => {
            run.relic_counters.insert(RelicId::SilkThread, 40);
        }
        RelicId::TeaCeremony => {
            run.relic_counters.insert(RelicId::TeaCeremony, 3);
        }
        _ => {}
    }
    run.recompute_capacities();
}

macro_rules! bot_log {
    ($enabled:expr, $($arg:tt)*) => {
        if $enabled {
            println!($($arg)*);
        }
    };
}

#[derive(Debug, Clone, Copy, Default)]
struct ClearPayoutBreakdown {
    base_reward: u32,
    unused_play_bonus: u32,
    interest: u32,
    relic_bonus: u32,
    total: u32,
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

fn format_top_counts(
    counts: &std::collections::BTreeMap<String, u32>,
    limit: usize,
) -> Option<String> {
    if counts.is_empty() {
        return None;
    }

    let mut items: Vec<(&String, &u32)> = counts.iter().collect();
    items.sort_by(|(ka, va), (kb, vb)| vb.cmp(va).then_with(|| ka.cmp(kb)));
    Some(
        items
            .into_iter()
            .take(limit)
            .map(|(name, count)| format!("{name} x{count}"))
            .collect::<Vec<_>>()
            .join(", "),
    )
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

fn analyze_hand_options(
    run: &RunState,
    hand: &[Tile],
    rules: &[RuleModifier],
) -> HandOptionAnalysis {
    let n = hand.len();
    if !(2..=20).contains(&n) {
        return HandOptionAnalysis::default();
    }

    let mut analysis = HandOptionAnalysis::default();
    for mask in enumerate_candidate_play_masks(hand, rules) {
        let indices: Vec<usize> = (0..n).filter(|i| mask & (1 << i) != 0).collect();
        let tiles: Vec<Tile> = indices.iter().map(|&i| hand[i]).collect();
        let Some((sets, scoring_tiles)) = run.try_validate_with_wildcards(&tiles) else {
            continue;
        };

        analysis.valid_count += 1;

        if run.uses_structure_bank() {
            let kongs_after = run
                .structure_sets
                .iter()
                .chain(sets.iter())
                .filter(|s| s.kind == SetKind::Kong)
                .count();
            if run.structure_tiles.len() + scoring_tiles.len()
                > crate::game::run::HAND_SIZE + kongs_after
            {
                continue;
            }
        }
        analysis.committable_count += 1;

        let mut merged_sets = run.structure_sets.clone();
        merged_sets.extend(sets.iter().cloned());
        let mut merged_tiles = run.structure_tiles.clone();
        merged_tiles.extend(scoring_tiles.iter().copied());
        let ctx = ctx_for_merged_commit(run, &run.relics, &merged_tiles, &merged_sets, None);
        let breakdown = score_sets_with_original(&merged_tiles, &merged_sets, &ctx, rules, &tiles);
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

    let analysis = analyze_hand_options(run, &run.hand, &run.round_rules);
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

/// Score context for evaluating a **commit** (merged structure + new melds) as if triggered.
fn ctx_for_merged_commit<'a>(
    run: &'a RunState,
    relics: &'a RelicState,
    merged_tiles: &[Tile],
    merged_sets: &[crate::core::hand::DetectedSet],
    yaku_levels: Option<&YakuLevels>,
) -> ScoreContext<'a> {
    let plays_rem_after = run.plays_remaining.saturating_sub(1);
    let plays_used_after = run.plays_max.saturating_sub(plays_rem_after);
    let meta = StructureTriggerMeta {
        meld_count: merged_sets.len() as u32,
        inject_chicken_if_no_yaku: true,
    };
    ScoreContext {
        relics,
        tile_debuffs: &run.tile_debuffs,
        scored_last_turn: run.scored_last_turn,
        dora_faces: run.wall.dora_faces(),
        available_yaku: run.available_yaku.clone(),
        round_wind: Some(BlindKind::round_wind_for_ante(run.ante)),
        first_full_hand_of_round: !run.full_hand_played_this_round,
        plays_used: plays_used_after,
        riichi_active: false,
        yaku_levels: Some(
            yaku_levels
                .cloned()
                .unwrap_or_else(|| run.yaku_levels.clone()),
        ),
        played_yaku_this_round: run.played_yaku_this_round.clone(),
        gold: run.gold,
        total_score: run.total_score_earned,
        is_final_play: plays_rem_after == 0,
        relic_counters: run.relic_counters.clone(),
        unscored_hand_tiles: run.hand.len().saturating_sub(merged_tiles.len()),
        structure: Some(meta),
    }
}

/// Find the best (score, indices) commit from `hand`, merging into the current structure.
/// `relics_override` / `yaku_levels_override` are for bot-side what-if evaluation.
fn best_play_in_hand(
    run: &RunState,
    hand: &[Tile],
    rules: &[RuleModifier],
    relics_override: Option<&RelicState>,
    yaku_levels_override: Option<&YakuLevels>,
) -> Option<(u64, Vec<usize>)> {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
    struct PlayRank {
        score: u64,
        meld_count: usize,
        tile_count: usize,
    }

    let n = hand.len();
    if !(2..=20).contains(&n) {
        return None;
    }
    let relics = relics_override.unwrap_or(&run.relics);
    let mut best: Option<(PlayRank, Vec<usize>)> = None;
    for mask in enumerate_candidate_play_masks(hand, rules) {
        let count = mask.count_ones() as usize;
        let mut tiles: Vec<Tile> = Vec::with_capacity(count);
        for (i, &tile) in hand.iter().enumerate().take(n) {
            if mask & (1 << i) != 0 {
                tiles.push(tile);
            }
        }
        let Some(sets) = validate_selection_with_rules(&tiles, rules) else {
            continue;
        };
        if run.uses_structure_bank() {
            let kongs_after = run
                .structure_sets
                .iter()
                .chain(sets.iter())
                .filter(|s| s.kind == SetKind::Kong)
                .count();
            if run.structure_tiles.len() + tiles.len() > crate::game::run::HAND_SIZE + kongs_after {
                continue;
            }
        }
        let mut merged_sets = run.structure_sets.clone();
        merged_sets.extend(sets.iter().cloned());
        let mut merged_tiles = run.structure_tiles.clone();
        merged_tiles.extend(tiles.iter().copied());
        let ctx = ctx_for_merged_commit(
            run,
            relics,
            &merged_tiles,
            &merged_sets,
            yaku_levels_override,
        );
        let breakdown = score_sets_with_original(&merged_tiles, &merged_sets, &ctx, rules, &tiles);
        if breakdown.total == 0 {
            continue;
        }
        let rank = PlayRank {
            score: breakdown.total,
            meld_count: sets.len(),
            tile_count: tiles.len(),
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
    };

    let mut masks = std::collections::HashSet::new();
    enumerate_regular_subsets(&regular, &flowers, 0, subset_rules, 0, &mut masks);
    let mut out: Vec<u32> = masks.into_iter().collect();
    out.sort_unstable();
    out
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
}

fn enumerate_regular_subsets(
    remaining: &[IndexedTile],
    flowers: &[IndexedTile],
    current_mask: u32,
    rules: SubsetRules,
    current_tile_count: usize,
    out: &mut std::collections::HashSet<u32>,
) {
    let SubsetRules {
        allow_wrap,
        no_sequences,
        require_honor,
        must_play_five,
    } = rules;
    if current_tile_count > 14 || (must_play_five && current_tile_count > 5) {
        return;
    }

    if remaining.is_empty() {
        emit_leaf_masks(
            flowers,
            current_mask,
            current_tile_count,
            must_play_five,
            out,
        );
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

    if !flowers.is_empty()
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
        for seq in sequence_candidates(remaining, allow_wrap, !flowers.is_empty(), first) {
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
    must_play_five: bool,
    out: &mut std::collections::HashSet<u32>,
) {
    for extra_mask in flower_only_masks(flowers) {
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

fn flower_only_masks(flowers: &[IndexedTile]) -> Vec<u32> {
    let mut masks = vec![0];

    for select_count in 2..=flowers.len().min(4) {
        collect_flower_masks(flowers, select_count, 0, 0, &mut masks);
    }

    masks
}

fn collect_flower_masks(
    flowers: &[IndexedTile],
    select_count: usize,
    start: usize,
    current_mask: u32,
    out: &mut Vec<u32>,
) {
    if select_count == 0 {
        out.push(current_mask);
        return;
    }

    for idx in start..=flowers.len() - select_count {
        collect_flower_masks(
            flowers,
            select_count - 1,
            idx + 1,
            current_mask | (1 << flowers[idx].hand_index),
            out,
        );
    }
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
    best_play_in_hand(run, &run.hand, &run.round_rules, None, None)
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
    use std::collections::HashSet;
    let drop_set: HashSet<usize> = discard_indices.iter().copied().collect();
    let k = discard_indices.len();
    let peeked = run.wall.peek_next(k);
    let mut new_hand: Vec<Tile> = run
        .hand
        .iter()
        .enumerate()
        .filter(|(i, _)| !drop_set.contains(i))
        .map(|(_, t)| *t)
        .collect();
    new_hand.extend_from_slice(peeked);
    new_hand.sort();
    best_play_in_hand(run, &new_hand, &run.round_rules, None, None)
        .map(|(s, _)| s)
        .unwrap_or(0)
}

/// Play the current blind to completion. Returns `true` if the bot reached the target.
fn play_blind(run: &mut RunState, stats: &mut RunStats, log: bool) -> bool {
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
            return true;
        }
        if let Some(reason) = run.round_failure_reason() {
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
            return false;
        }
        turn += 1;

        bot_log!(
            log,
            "    turn {:>2}: score {}/{} | plays {} | discards {} | hand {} | gold {}",
            turn,
            run.round_score,
            run.target_score,
            run.plays_remaining,
            run.discards_remaining,
            run.hand.len(),
            run.gold
        );

        if use_bot_consumables(run, log) {
            continue;
        }

        let best = pick_best_play(run);
        let best_score = best.as_ref().map(|(s, _)| *s).unwrap_or(0);

        // Structure: cash in when the current structure scores at least as much as the best
        // commit preview (saves a play), or when there is no positive commit but the structure can score.
        let trigger_preview = if run.uses_structure_bank() && run.can_trigger_structure_now() {
            run.preview_manual_trigger_total()
        } else {
            0
        };
        if run.uses_structure_bank()
            && trigger_preview > 0
            && (best_score == 0 || trigger_preview >= best_score)
        {
            let earned = run.trigger_structure_manual(&mut bus);
            if earned > 0 {
                bot_log!(
                    log,
                    "      action: trigger structure for {} (best play {})",
                    earned,
                    best_score
                );
                for ev in bus.drain() {
                    if let GameEvent::RoundComplete { payout, .. } = ev {
                        run.gold = run.gold.saturating_add(payout.total as i32);
                    }
                }
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
            let candidates = discard_candidates(&run.hand, 5);
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
                for _ in bus.drain() {}
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
            run.clear_selection();
            for i in &indices {
                run.toggle_select(*i);
            }
            let plays_before = run.plays_remaining;
            let score_before = run.round_score;
            let committed = run.score_selected_tiles(&mut bus);
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
                return false;
            }
            stats.plays_used += 1;
            for ev in bus.drain() {
                if let GameEvent::RoundComplete { payout, .. } = ev {
                    run.gold = run.gold.saturating_add(payout.total as i32);
                }
            }
            continue;
        }

        // No positive-scoring play and no strategic discard helped — random discard
        // as a last-resort shake-up before busting.
        if run.discards_remaining == 0 {
            record_terminal_hand_issue(stats, run, TerminalIssueCause::NoDiscardsRemaining);
            bot_log!(log, "      action: no discards remaining");
            return false;
        }
        run.clear_selection();
        let hand_n = run.hand.len();
        if hand_n == 0 {
            record_terminal_hand_issue(stats, run, TerminalIssueCause::EmptyHand);
            bot_log!(log, "      action: hand empty, cannot continue");
            return false;
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
        for _ in bus.drain() {}
    }
}

#[cfg(test)]
mod tests {
    use super::{
        best_play_in_hand, enumerate_candidate_play_masks, pick_best_play,
        remaining_antes_including_current, scale_long_term_value_for_ante, talisman_marginal_value,
        use_bot_consumables, zodiac_marginal_value,
    };
    use crate::core::consumable::Consumable;
    use crate::core::hand::{DetectedSet, SetKind};
    use crate::core::talisman::TalismanKind;
    use crate::core::tile::{Suit, Tile};
    use crate::core::zodiac::ZodiacKind;
    use crate::game::run::{FINAL_ANTE, HAND_SIZE, RunState};

    fn brute_force_best_play_in_hand(run: &RunState) -> Option<(u64, Vec<usize>)> {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
        struct PlayRank {
            score: u64,
            meld_count: usize,
            tile_count: usize,
        }

        let n = run.hand.len();
        let mut best: Option<(PlayRank, Vec<usize>)> = None;
        let limit: u32 = 1u32 << n;
        for mask in 1u32..limit {
            let count = mask.count_ones() as usize;
            // `validate_selection_with_rules` accepts several sizes (e.g. 3 = one meld,
            // 4 = kong or two flower pairs, 6 = two melds). Do not skip by count —
            // an old `4`-tile exclusion caused brute force to disagree with
            // `enumerate_candidate_play_masks` / `best_play_in_hand`.
            if matches!(count, 0 | 1) {
                continue;
            }
            let mut tiles = Vec::with_capacity(count);
            for i in 0..n {
                if mask & (1 << i) != 0 {
                    tiles.push(run.hand[i]);
                }
            }
            let Some(sets) =
                crate::core::hand::validate_selection_with_rules(&tiles, &run.round_rules)
            else {
                continue;
            };
            if run.uses_structure_bank() {
                let kongs_after = run
                    .structure_sets
                    .iter()
                    .chain(sets.iter())
                    .filter(|s| s.kind == SetKind::Kong)
                    .count();
                if run.structure_tiles.len() + tiles.len()
                    > crate::game::run::HAND_SIZE + kongs_after
                {
                    continue;
                }
            }
            let mut merged_sets = run.structure_sets.clone();
            merged_sets.extend(sets.iter().cloned());
            let mut merged_tiles = run.structure_tiles.clone();
            merged_tiles.extend(tiles.iter().copied());
            let ctx =
                super::ctx_for_merged_commit(run, &run.relics, &merged_tiles, &merged_sets, None);
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
                tile_count: tiles.len(),
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
        run.hand = vec![
            t(Suit::Characters, 3, 1),
            t(Suit::Characters, 3, 2),
            t(Suit::Characters, 3, 3),
            t(Suit::Bamboos, 6, 4),
            t(Suit::Bamboos, 6, 5),
            t(Suit::Bamboos, 6, 6),
            t(Suit::Circles, 2, 7),
            t(Suit::Circles, 3, 8),
            t(Suit::Circles, 4, 9),
        ];
        run.hand.sort();
        run
    }

    #[test]
    fn bot_skips_structure_commits_that_overflow_the_bank() {
        let mut run = RunState::new_demo();
        run.structure_tiles = run.hand.iter().take(12).copied().collect();
        run.structure_sets = vec![
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: run.structure_tiles[0..3].iter().map(|t| t.id).collect(),
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: run.structure_tiles[3..6].iter().map(|t| t.id).collect(),
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: run.structure_tiles[6..9].iter().map(|t| t.id).collect(),
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: run.structure_tiles[9..12].iter().map(|t| t.id).collect(),
            },
        ];
        assert_eq!(run.structure_tiles.len(), HAND_SIZE - 2);

        let best = pick_best_play(&run);
        assert!(
            best.as_ref()
                .map(|(_, indices)| run.structure_tiles.len() + indices.len() <= HAND_SIZE)
                .unwrap_or(true),
            "bot should not choose a play that the structure bank would reject"
        );
    }

    #[test]
    fn enumerated_masks_match_bruteforce_best_play_on_demo_hand() {
        let run = RunState::new_demo();
        assert_eq!(pick_best_play(&run), brute_force_best_play_in_hand(&run));
    }

    #[test]
    fn enumerated_masks_match_bruteforce_best_play_with_flowers() {
        let mut run = RunState::new_demo();
        run.hand = vec![
            t(Suit::Characters, 2, 1),
            t(Suit::Characters, 3, 2),
            t(Suit::Characters, 5, 3),
            t(Suit::Characters, 5, 4),
            t(Suit::Characters, 5, 5),
            t(Suit::Bamboos, 7, 6),
            t(Suit::Bamboos, 8, 7),
            t(Suit::Dragon, 1, 8),
            t(Suit::Dragon, 1, 9),
            t(Suit::Flower, 1, 10),
            t(Suit::Flower, 2, 11),
        ];
        run.hand.sort();
        let new_best = best_play_in_hand(&run, &run.hand, &run.round_rules, None, None);
        let old_best = brute_force_best_play_in_hand(&run);
        assert_eq!(new_best, old_best);
    }

    #[test]
    fn candidate_masks_only_produce_valid_selections() {
        let mut run = RunState::new_demo();
        run.hand = vec![
            t(Suit::Characters, 1, 1),
            t(Suit::Characters, 2, 2),
            t(Suit::Characters, 3, 3),
            t(Suit::Characters, 7, 4),
            t(Suit::Characters, 7, 5),
            t(Suit::Characters, 7, 6),
            t(Suit::Wind, 1, 7),
            t(Suit::Wind, 1, 8),
            t(Suit::Flower, 1, 9),
            t(Suit::Flower, 2, 10),
            t(Suit::Flower, 3, 11),
            t(Suit::Flower, 4, 12),
        ];
        run.hand.sort();

        for mask in enumerate_candidate_play_masks(&run.hand, &run.round_rules) {
            let tiles: Vec<_> = run
                .hand
                .iter()
                .enumerate()
                .filter_map(|(i, t)| (mask & (1 << i) != 0).then_some(*t))
                .collect();
            assert!(
                crate::core::hand::validate_selection_with_rules(&tiles, &run.round_rules)
                    .is_some(),
                "invalid candidate mask {mask:b} for hand {:?}",
                run.hand
            );
        }
    }

    #[test]
    fn candidate_masks_include_each_flower_identity_for_wildcard_melds() {
        let mut run = RunState::new_demo();
        run.hand = vec![
            t(Suit::Characters, 5, 1),
            t(Suit::Characters, 5, 2),
            t(Suit::Flower, 1, 3),
            t(Suit::Flower, 2, 4),
        ];
        run.hand.sort();

        let masks = enumerate_candidate_play_masks(&run.hand, &run.round_rules);
        let selected_ids: Vec<Vec<u32>> = masks
            .iter()
            .map(|mask| {
                run.hand
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
        assert!(zodiac_marginal_value(&run, ZodiacKind::Ox) > 0);
    }

    #[test]
    fn talisman_value_is_positive_when_it_buffs_a_scoring_hand() {
        let run = scoring_test_run();
        assert!(talisman_marginal_value(&run, TalismanKind::Jade) > 0);
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

        assert!(use_bot_consumables(&mut run, false));
        assert_eq!(run.yaku_levels.level_of(ZodiacKind::Ox.yaku()), 2);
        assert!(
            run.hand.iter().all(|tile| tile.enhancement.is_some()),
            "best talisman should stamp the hand"
        );
        assert!(run.consumables.items.is_empty());
    }
}

/// Number of synthetic random hands sampled when evaluating a relic's value.
/// Higher = more accurate signal but slower (each sample runs `best_play_in_hand`,
/// which is the bot's hot loop).
const RELIC_EVAL_SAMPLES: usize = 4;

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
        run.relics.has(RelicId::Overflow),
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
    best_play_in_hand(
        run,
        hand,
        &run.round_rules,
        relics_override,
        yaku_levels_override,
    )
    .map(|(s, _)| s)
    .unwrap_or(0)
}

/// Estimate the value of owning `candidate` by averaging the best-play score
/// improvement across the current hand *and* several synthetic random hands.
///
/// We need the random sampling because most relics' effects are
/// hand-conditional — `TripletBoost` is worthless on a hand with no triplets,
/// `BambooCharm` does nothing without bamboo tiles. Evaluating only the current
/// hand systematically under-values relics whose payoff is "applies whenever you
/// happen to draw the right tiles." A handful of synthetic hands surface that
/// expected value.
///
/// Wall-mutating relics (`Overflow`, `SetMagnet`, `QuickDraw`, `WildWinds`,
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

fn relic_marginal_value(run: &RunState, candidate: RelicId) -> i32 {
    if run.relics.owns(candidate) {
        return -1;
    }
    if run.relics.is_full() {
        return 0;
    }

    let mut hypothetical = run.relics.clone();
    hypothetical.active.push(candidate);

    // Sample 1: the bot's actual current hand (weighted heavily).
    let mut delta_sum: i64 = best_play_score_for_hand(run, &run.hand, Some(&hypothetical), None)
        as i64
        - best_play_score_for_hand(run, &run.hand, None, None) as i64;
    let mut sample_count: i64 = 1;

    // Samples 2..N: synthetic random hands from fresh walls.
    for _ in 0..RELIC_EVAL_SAMPLES {
        let hand = sample_random_hand(run.mode.hand_size);
        delta_sum += best_play_score_for_hand(run, &hand, Some(&hypothetical), None) as i64
            - best_play_score_for_hand(run, &hand, None, None) as i64;
        sample_count += 1;
    }

    scale_long_term_value_for_ante((delta_sum / sample_count) as i32, run.ante)
}

fn zodiac_marginal_value(run: &RunState, zodiac: ZodiacKind) -> i32 {
    let mut hypothetical = run.yaku_levels.clone();
    hypothetical.level_up(zodiac.yaku());

    let mut delta_sum: i64 = best_play_score_for_hand(run, &run.hand, None, Some(&hypothetical))
        as i64
        - best_play_score_for_hand(run, &run.hand, None, None) as i64;
    let mut sample_count: i64 = 1;

    for _ in 0..RELIC_EVAL_SAMPLES {
        let hand = sample_random_hand(run.mode.hand_size);
        delta_sum += best_play_score_for_hand(run, &hand, None, Some(&hypothetical)) as i64
            - best_play_score_for_hand(run, &hand, None, None) as i64;
        sample_count += 1;
    }

    scale_long_term_value_for_ante((delta_sum / sample_count) as i32, run.ante)
}

/// Given a hand slice, pick a tile selection (indices into `hand`) for a
/// selection-acting talisman and return the expected best-play score delta.
/// Returns `None` for stochastic talismans (Honors/Wildflower/Conformity) or
/// when no meaningful selection exists — the bot leaves those on the shelf.
fn best_selection_for_talisman_on_hand(
    run: &RunState,
    hand: &[Tile],
    kind: TalismanKind,
) -> Option<(Vec<usize>, i64)> {
    use crate::core::tile::Suit;
    if hand.is_empty() {
        return None;
    }
    let base = best_play_score_for_hand(run, hand, None, None) as i64;
    match kind {
        // Kiln destroys selected tiles. Simulate dropping k lowest-
        // participation tiles (no draws simulated since shop-time hand is
        // empty anyway — we approximate Kiln's value as "remove dead weight
        // and replace with a typical random tile").
        TalismanKind::Kiln => {
            let counts = tile_meld_participation(hand);
            let mut indexed: Vec<(usize, u32)> = counts.into_iter().enumerate().collect();
            indexed.sort_by_key(|(_, c)| *c);
            let order: Vec<usize> = indexed.into_iter().map(|(i, _)| i).collect();
            let max_k = order.len().min(3);
            let mut best: Option<(Vec<usize>, i64)> = None;
            for k in 1..=max_k {
                let sel: Vec<usize> = order.iter().take(k).copied().collect();
                let drop_set: std::collections::HashSet<usize> = sel.iter().copied().collect();
                let mut replaced: Vec<Tile> = hand
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| !drop_set.contains(i))
                    .map(|(_, t)| *t)
                    .collect();
                let drawn = sample_random_hand(k);
                replaced.extend(drawn);
                replaced.sort();
                let after = best_play_score_for_hand(run, &replaced, None, None) as i64;
                let delta = after - base;
                if best.as_ref().map(|(_, d)| delta > *d).unwrap_or(true) {
                    best = Some((sel, delta));
                }
            }
            best
        }
        // Suit-transforms rewrite selected numbered tiles to a target suit.
        // Convert all numbered tiles NOT already in the target suit and
        // measure the delta against the baseline.
        TalismanKind::Bamboo | TalismanKind::Dots | TalismanKind::Characters => {
            let target = match kind {
                TalismanKind::Bamboo => Suit::Bamboos,
                TalismanKind::Dots => Suit::Circles,
                TalismanKind::Characters => Suit::Characters,
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
            Some((sel, after - base))
        }
        // Wildflower: selected tiles become flowers. Flowers are
        // wildcards that score any meld, so the value comes from flooding
        // the hand with them. Select every non-flower tile and simulate
        // the transform — rank is randomized per tile but doesn't affect
        // scoring (flowers are wildcards). Rank 1 is a fine placeholder.
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
            for &i in &sel {
                simulated[i].suit = Suit::Flower;
                simulated[i].rank = 1;
            }
            simulated.sort();
            let after = best_play_score_for_hand(run, &simulated, None, None) as i64;
            Some((sel, after - base))
        }
        // Conformity: selected tiles become copies of a random hand tile
        // (game-picked, including the selected set). Since the template is
        // random, average the score delta over every possible template
        // pick to get expected value. For the returned selection we just
        // select every tile — any template then produces 14 copies of
        // that template, which trivially scores Toitoi or similar.
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
            let avg_delta = total_delta / hand.len() as i64;
            let sel: Vec<usize> = (0..hand.len()).collect();
            Some((sel, avg_delta))
        }
        // Honors: selected numbered tiles become random honors. The
        // honors need to pair up to score, and the RNG outcome is wide,
        // but the rule-of-thumb is that converting ~5 tiles produces
        // roughly 2 scoring melds (a triplet + a pair). Simulate a
        // deterministic "one dragon triplet + one wind triplet" outcome
        // on 6 low-participation numbered tiles and measure the delta.
        TalismanKind::Honors => {
            use crate::core::tile::Suit;
            let counts = tile_meld_participation(hand);
            let mut indexed: Vec<(usize, u32)> = hand
                .iter()
                .enumerate()
                .filter(|(_, t)| t.is_number_tile())
                .map(|(i, _)| (i, counts[i]))
                .collect();
            if indexed.len() < 6 {
                return None;
            }
            indexed.sort_by_key(|(_, c)| *c);
            let sel: Vec<usize> = indexed.iter().take(6).map(|(i, _)| *i).collect();
            let mut simulated = hand.to_vec();
            // First 3 → Red Dragon, next 3 → East Wind. Deterministic
            // stand-in for the "2 honor triplets" typical outcome.
            for (n, &i) in sel.iter().enumerate() {
                if n < 3 {
                    simulated[i].suit = Suit::Dragon;
                    simulated[i].rank = 1;
                } else {
                    simulated[i].suit = Suit::Wind;
                    simulated[i].rank = 1;
                }
            }
            simulated.sort();
            let after = best_play_score_for_hand(run, &simulated, None, None) as i64;
            Some((sel, after - base))
        }
        // Buff talismans should never reach this function.
        TalismanKind::Jade
        | TalismanKind::Pearl
        | TalismanKind::Gilded
        | TalismanKind::Polychrome => None,
    }
}

/// Convenience wrapper that picks a selection on the bot's current hand.
/// Used at talisman-use time (the hand is populated then).
fn best_selection_for_talisman(run: &RunState, kind: TalismanKind) -> Option<(Vec<usize>, i64)> {
    best_selection_for_talisman_on_hand(run, &run.hand, kind)
}

/// Estimate the score improvement from using a selection talisman on a
/// typical future hand. Used at shop-time where the current hand is empty.
///
/// Selection talismans are **one-shot**: they fire on one hand and vanish.
/// Relics, by contrast, apply to every future blind. To make the two
/// valuations comparable, we discount the raw delta by roughly the number
/// of blinds still ahead — a talisman worth +X on one hand is only worth
/// +X/blinds_remaining versus a relic worth +X/blind on every blind.
fn sampled_selection_talisman_value(run: &RunState, kind: TalismanKind) -> i32 {
    let mut delta_sum: i64 = 0;
    let mut count: i64 = 0;
    for _ in 0..RELIC_EVAL_SAMPLES + 1 {
        let hand = sample_random_hand(run.mode.hand_size);
        if let Some((_, delta)) = best_selection_for_talisman_on_hand(run, &hand, kind) {
            delta_sum += delta;
            count += 1;
        }
    }
    if count == 0 {
        return 0;
    }
    let avg = (delta_sum / count).max(0);
    // Blinds remaining in the run (3 per ante, including current).
    let remaining_blinds = (remaining_antes_including_current(run.ante) as i64 * 3).max(1);
    (avg / remaining_blinds) as i32
}

fn talisman_marginal_value(run: &RunState, talisman: TalismanKind) -> i32 {
    if talisman.acts_on_selection() {
        return sampled_selection_talisman_value(run, talisman);
    }
    if talisman.enhancement().is_none() {
        return 0;
    }
    // Buff talismans stamp every tile in hand. Sample a few hands (like
    // zodiacs) to avoid zero-variance on an unlucky current hand, then
    // use the average single-play delta as a conservative one-shot value.
    let mut delta_sum: i64 = {
        let base = best_play_score_for_hand(run, &run.hand, None, None) as i64;
        let mut enhanced_hand = run.hand.clone();
        crate::core::talisman::apply_to_hand(&mut enhanced_hand, talisman);
        let buffed = best_play_score_for_hand(run, &enhanced_hand, None, None) as i64;
        buffed - base
    };
    let mut sample_count: i64 = 1;
    for _ in 0..RELIC_EVAL_SAMPLES {
        let hand = sample_random_hand(run.mode.hand_size);
        let base = best_play_score_for_hand(run, &hand, None, None) as i64;
        let mut enhanced = hand.clone();
        crate::core::talisman::apply_to_hand(&mut enhanced, talisman);
        let buffed = best_play_score_for_hand(run, &enhanced, None, None) as i64;
        delta_sum += buffed - base;
        sample_count += 1;
    }
    // One-shot: talisman only pays out this round (buffed tiles leave the
    // hand as plays happen and are replaced by unbuffed draws). Use a
    // single best-play delta as a conservative estimate — it's the score
    // lift the bot would see on its next commit, which competes directly
    // against relics/zodiacs that permanently affect all future rounds.
    let raw = (delta_sum / sample_count) as i32;
    // Brocade Pouch promotes buff talismans from one-shot to run-long: the
    // enhancement stamps every drawn tile for the rest of the run, so value
    // compounds across antes the same way a relic does.
    if run.relics.has(RelicId::BrocadePouch) {
        return scale_long_term_value_for_ante(raw, run.ante);
    }
    // Polychrome is uniquely multiplicative (×1.2 mult per meld, scales with
    // the rest of the mult stack) rather than additive-per-tile like Jade/
    // Pearl/Gilded. A single best-play delta dramatically understates it:
    // the same stamped hand typically supports 2–3 plays in a round before
    // draw-attrition thins out the stamped tiles, and the ×1.2 compounds
    // against mult that grows through the round (Snowball, Momentum,
    // sequence bonuses, etc.). Boost the raw delta to reflect that
    // per-round payoff.
    if matches!(talisman, TalismanKind::Polychrome) {
        return raw.saturating_mul(2);
    }
    raw
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
    for _ in 0..RELIC_EVAL_SAMPLES + 1 {
        let base_wall = Wall::from_filtered_with_packs(
            &run.removed_tile_ids,
            &run.tile_packs,
            &run.tile_enhancements,
            run.relics.has(RelicId::Overflow),
        );
        let mut base_hand = Vec::with_capacity(run.mode.hand_size);
        let mut base_wall = base_wall;
        for _ in 0..run.mode.hand_size {
            if let Some(t) = base_wall.draw() {
                base_hand.push(t);
            }
        }
        base_hand.sort();
        let with = sample_random_hand_with_extra_pack(run, kind, run.mode.hand_size);
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

fn use_bot_consumables(run: &mut RunState, log: bool) -> bool {
    let mut used_any = false;

    while let Some(idx) = run
        .consumables
        .items
        .iter()
        .position(|c| matches!(c, Consumable::Zodiac(_)))
    {
        let zodiac = match run.consumables.items[idx] {
            Consumable::Zodiac(z) => z,
            Consumable::Talisman(_) => unreachable!(),
        };
        let _ = run.use_consumable(idx, &mut crate::game::event_bus::EventBus::default());
        bot_log!(log, "      action: use zodiac {:?}", zodiac);
        used_any = true;
    }

    let base = pick_best_play(run).map(|(s, _)| s).unwrap_or(0);
    let mut best_talisman: Option<(usize, TalismanKind, i32)> = None;
    for (idx, consumable) in run.consumables.items.iter().copied().enumerate() {
        let Consumable::Talisman(kind) = consumable else {
            continue;
        };
        // At use-time, evaluate against the actual current hand rather
        // than the sampled shop-time estimate. Selection talismans score
        // their real score lift on this specific hand; buff talismans use
        // the existing marginal-value function.
        let delta = if kind.acts_on_selection() {
            best_selection_for_talisman(run, kind)
                .map(|(_, d)| d.max(0) as i32)
                .unwrap_or(0)
        } else {
            talisman_marginal_value(run, kind)
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
        if kind.acts_on_selection() {
            let Some((selection, _)) = best_selection_for_talisman(run, kind) else {
                // Valuation returned >0 but no usable selection — bail
                // rather than consuming with an empty selection, which the
                // run-state guards against anyway.
                return used_any;
            };
            // Clear any stale selection, then select the chosen tiles.
            let existing: Vec<usize> = (0..run.hand.len())
                .filter(|&i| run.selected.get(i).copied().unwrap_or(false))
                .collect();
            for i in existing {
                run.toggle_select(i);
            }
            for i in &selection {
                run.toggle_select(*i);
            }
        }
        let _ = run.use_consumable(idx, &mut crate::game::event_bus::EventBus::default());
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
fn visit_shop(run: &mut RunState, stats: &mut RunStats, log: bool, strategy: &BotStrategy) {
    // Consume tag-granted shop modifiers (headless analogue of ShopScene::new).
    let extra_relics: usize = if run.tag_rich_stock { 2 } else { 0 };
    let patron_gift = run.tag_patron_gift;
    // Free reroll is a no-op for the bot (it doesn't reroll).
    run.tag_free_reroll = false;
    run.tag_patron_gift = false;
    run.tag_rich_stock = false;

    let defs = all_relic_defs();
    let extinct = run.paper_lantern_extinct;
    let mut pool: Vec<RelicId> = defs
        .iter()
        .filter(|d| {
            if !run.available_relics.contains(&d.id) || run.relics.owns(d.id) {
                return false;
            }
            if d.id == RelicId::PhantomRelic {
                return false;
            }
            if d.id == RelicId::PaperLantern && extinct {
                return false;
            }
            if d.id == RelicId::IronLantern && !extinct {
                return false;
            }
            true
        })
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
        if shop.is_empty() {
            bot_log!(log, "    shop: leaving ({})", "no offerings left");
            break;
        }
        // Find the best affordable offer with positive marginal value.
        let mut best: Option<(usize, i32)> = None;
        for (i, offer) in shop.iter().copied().enumerate() {
            let price = match offer {
                ShopOffer::Relic(id) => {
                    if free_relic {
                        0
                    } else {
                        run.mode.scale_shop_price(relic_shop_price(id, &run.relics))
                    }
                }
                ShopOffer::Zodiac(_) => run.mode.scale_shop_price(ZodiacKind::shop_price()),
                ShopOffer::Talisman(kind) => run.mode.scale_shop_price(kind.shop_price()),
                ShopOffer::Pack(kind) => run.mode.scale_shop_price(kind.shop_price()),
            };
            if price as i32 > run.gold {
                continue;
            }
            let raw_mv = match offer {
                ShopOffer::Relic(id) => relic_marginal_value(run, id),
                ShopOffer::Zodiac(zodiac) => zodiac_marginal_value(run, zodiac),
                ShopOffer::Talisman(kind) => {
                    if run.consumables.is_full()
                        || run.consumables.items.iter().any(
                            |c| matches!(c, Consumable::Talisman(existing) if *existing == kind),
                        )
                    {
                        0
                    } else {
                        talisman_marginal_value(run, kind)
                    }
                }
                ShopOffer::Pack(kind) => pack_marginal_value(run, kind),
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
            if best.as_ref().map(|(_, b)| mv > *b).unwrap_or(true) {
                best = Some((i, mv));
            }
        }
        let Some((idx, marginal_value)) = best else {
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
                run.gold -= price as i32;
                acquire_relic(run, id);
                stats.relics_bought += 1;
                *stats
                    .relics_picked
                    .entry(relic_display_name(id))
                    .or_insert(0) += 1;
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
                let price = run.mode.scale_shop_price(ZodiacKind::shop_price());
                run.gold -= price as i32;
                let new_level = run.yaku_levels.level_up(zodiac.yaku());
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
                let price = run.mode.scale_shop_price(kind.shop_price());
                run.gold -= price as i32;
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
                let price = run.mode.scale_shop_price(kind.shop_price());
                run.gold -= price as i32;
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
    let target = run.base_target.saturating_mul(run.run_number);
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

fn play_run_with_options(
    config: BotConfig,
    options: BotRunOptions,
    run_number: Option<u32>,
) -> RunStats {
    let strategy = BotStrategy::from_config(&config);
    let forced_relic = config.forced_relic;
    let mode = config.into_mode();
    let mut run = RunState::new(mode);
    let mut stats = RunStats::default();
    let mut bus = EventBus::default();
    let log = options.log;

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
                run.apply_tag(tag);
                let gold_after = run.gold;
                let realized_gold = gold_after.saturating_sub(gold_before).max(0) as u32;
                stats.gold_from_skip_tags += realized_gold;
                stats.skip_tag_gold_value += tag.gold_value();
                *stats.skipped_tags.entry(tag.name()).or_insert(0) += 1;
                if let Some((zodiac, _, _)) = run.pending_zodiac_celebration {
                    *stats.zodiacs_picked.entry(zodiac.name()).or_insert(0) += 1;
                }
            }
            run.skip_to_next_blind();
            stats.blinds_skipped += 1;
            continue;
        }

        stats.total_target_score += run.target_score as u64;
        run.apply_blind(blind);
        let boss_for_this_blind = if matches!(blind, BlindKind::Boss) {
            current_boss_name(&run).map(|name| name.to_string())
        } else {
            None
        };
        if let Some(boss_name) = &boss_for_this_blind {
            stats.boss_faced.insert(boss_name.clone(), 1);
        }
        let cleared = play_blind(&mut run, &mut stats, log);
        stats.total_score += run.round_score;
        stats.peak_blind_score = stats.peak_blind_score.max(run.round_score);
        stats.died_on_ante = run.ante;
        stats.died_on_blind = blind;
        if let Some(boss_name) = &boss_for_this_blind
            && cleared
        {
            stats.boss_beaten.insert(boss_name.clone(), 1);
        }
        if !cleared {
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
        stats.blinds_cleared += 1;
        let blind_overscore = run.round_score.saturating_sub(run.target_score as u64);
        stats.total_overscore += blind_overscore;
        let slot_key = format!("{:02}-{}", run.ante, blind.name());
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
        visit_shop(&mut run, &mut stats, log, &strategy);
    }

    stats.final_gold = run.gold;
    bot_log!(log, "== bot run stats: {:?} ==", stats);
    stats
}
