//! Depth-limited expectimax for in-blind turns (play, discard, structure, consumables).
//!
//! Uses oracle wall draws (same peek as the legacy discard rollout) and
//! [`BlindPlanCheckpoint`](super::blind_sim::BlindPlanCheckpoint) restore so
//! candidate actions run through real `RunState` transitions.
//!
//! - **Depth 1:** one-ply unified action choice (play vs discard vs structure vs consumable).
//! - **Depth 2:** default recommended search.
//! - **Depth 3+:** prunes child branches to the top [`INNER_BRANCH_LIMIT`] by one-step utility.

use crate::core::consumable::Consumable;
use crate::core::scoring::format_meld_groups;
use crate::core::talisman::TalismanKind;
use crate::game::event_bus::EventBus;
use crate::game::run::RunState;

use super::blind_sim::{BlindAction, BlindPlanCheckpoint, branch_from_checkpoint};
use super::relic_analytics;
use super::stats::{BotScoringAction, RunStats};
use super::{
    PlayBlindOutcome, blind_slot_key, buff_talisman_lift_on_hand, discard_candidates,
    drain_post_action_bus, fmt_indices, score_breakdown_for_play_indices,
    top_k_plays_in_hand, transform_talisman_lift_on_hand,
};

const MAX_PLAY_CANDIDATES: usize = 4;
const MAX_DISCARD_CANDIDATES: usize = 5;
/// Inner plies (depth ≥ 2 remaining): keep only this many child actions.
const INNER_BRANCH_LIMIT: usize = 5;

#[derive(Clone, Copy)]
struct ActionEnumConfig {
    max_play: usize,
    max_discard: usize,
    consumables: bool,
}

const ENUM_FULL: ActionEnumConfig = ActionEnumConfig {
    max_play: MAX_PLAY_CANDIDATES,
    max_discard: MAX_DISCARD_CANDIDATES,
    consumables: true,
};

const ENUM_MID: ActionEnumConfig = ActionEnumConfig {
    max_play: 3,
    max_discard: 3,
    consumables: false,
};

fn enum_cfg_for_search_depth(depth: u32) -> ActionEnumConfig {
    match depth {
        1 => ENUM_MID,
        _ => ENUM_FULL,
    }
}

struct TurnEvalCache {
    top_plays: Vec<(u64, Vec<usize>)>,
    best_play: u64,
    structure_preview: u64,
}

fn build_turn_cache(run: &RunState, play_k: usize) -> TurnEvalCache {
    let top_plays = top_k_plays_in_hand(run, run.hand(), play_k);
    let best_play = top_plays.first().map(|(s, _)| *s).unwrap_or(0);
    let structure_preview = if run.can_trigger_structure_now() {
        run.preview_manual_trigger_total()
    } else {
        0
    };
    TurnEvalCache {
        top_plays,
        best_play,
        structure_preview,
    }
}

fn consumable_slot(run: &RunState, target: Consumable) -> Option<usize> {
    run.consumables.items.iter().position(|c| *c == target)
}

fn talisman_lift(run: &RunState, kind: TalismanKind) -> i32 {
    if kind.enhancement().is_none() {
        transform_talisman_lift_on_hand(run, run.hand(), kind)
            .map(|d| d.max(0) as i32)
            .unwrap_or(0)
    } else {
        buff_talisman_lift_on_hand(run, run.hand(), kind)
    }
}

fn enumerate_actions(run: &RunState, cfg: ActionEnumConfig) -> Vec<BlindAction> {
    let cache = build_turn_cache(run, cfg.max_play);
    let mut out = Vec::new();

    if cfg.consumables {
        for item in &run.consumables.items {
            match item {
                Consumable::Zodiac(z) => out.push(BlindAction::UseZodiac(*z)),
                Consumable::Talisman(t) if talisman_lift(run, *t) > 0 => {
                    out.push(BlindAction::UseTalisman(*t));
                }
                Consumable::Talisman(_) | Consumable::Memorial(_) => {}
            }
        }
    }

    if cache.structure_preview > 0 {
        out.push(BlindAction::TriggerStructure);
    }

    for (_, indices) in &cache.top_plays {
        out.push(BlindAction::Play(indices.clone()));
    }

    if run.discards_remaining > 0 && run.plays_remaining > 0 {
        for cand in discard_candidates(run.hand(), cfg.max_discard) {
            out.push(BlindAction::Discard(cand));
        }
    }

    out
}

fn blind_utility_with(run: &RunState, best_play: u64, structure_preview: u64) -> f64 {
    let target = run.target_score.max(1) as f64;
    if run.round_score >= run.target_score as u64 {
        return 1_000_000.0 + run.plays_remaining as f64 * 100.0;
    }
    if run.round_failure_reason().is_some() {
        return -1_000_000.0;
    }

    let need = (run.target_score as u64).saturating_sub(run.round_score) as f64;
    let best = best_play as f64;
    let structure_preview = structure_preview as f64;
    let resources = run.plays_remaining as f64 + 0.3 * run.discards_remaining as f64;

    let repeat = best * run.plays_remaining as f64 * 0.8;
    let optimistic = run.round_score as f64 + repeat + structure_preview * 0.6;
    let progress = 1.0 - need / target;
    let reach = (optimistic / target).clamp(0.0, 1.5);

    let bank = run.structure_tiles().len() as f64;
    let bank_value = bank * best * 0.08;

    progress * 12_000.0 + reach * 6_000.0 + resources * 80.0 + bank_value
}

/// Heuristic position value: progress toward clearing the blind plus resource slack.
pub(crate) fn blind_utility(run: &RunState) -> f64 {
    let cache = build_turn_cache(run, 1);
    blind_utility_with(run, cache.best_play, cache.structure_preview)
}

/// Cheap one-step score for branch pruning (no play-mask enumeration).
fn quick_blind_utility(run: &RunState) -> f64 {
    let target = run.target_score.max(1) as f64;
    if run.round_score >= run.target_score as u64 {
        return 1_000_000.0 + run.plays_remaining as f64 * 100.0;
    }
    if run.round_failure_reason().is_some() {
        return -1_000_000.0;
    }
    let need = (run.target_score as u64).saturating_sub(run.round_score) as f64;
    let progress = 1.0 - need / target;
    let resources = run.plays_remaining as f64 + 0.3 * run.discards_remaining as f64;
    let bank = run.structure_tiles().len() as f64;
    progress * 12_000.0 + resources * 80.0 + bank * 200.0
}

fn utility_after_action(run: &RunState) -> f64 {
    let cache = build_turn_cache(run, 1);
    blind_utility_with(run, cache.best_play, cache.structure_preview)
}

fn prune_actions(
    checkpoint: &BlindPlanCheckpoint,
    run: &mut RunState,
    actions: Vec<BlindAction>,
    limit: usize,
) -> Vec<BlindAction> {
    if actions.len() <= limit {
        return actions;
    }
    let mut quick: Vec<(f64, BlindAction)> = actions
        .into_iter()
        .filter_map(|action| {
            branch_from_checkpoint(checkpoint, run, &action, |run| quick_blind_utility(run))
                .map(|score| (score, action))
        })
        .collect();
    checkpoint.restore(run);
    quick.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    quick.truncate(limit);
    quick.into_iter().map(|(_, action)| action).collect()
}

fn search_value(run: &mut RunState, depth: u32) -> f64 {
    if depth == 0 || run.round_score >= run.target_score as u64 || run.round_failure_reason().is_some()
    {
        return utility_after_action(run);
    }

    let checkpoint = BlindPlanCheckpoint::capture(run);
    let cfg = enum_cfg_for_search_depth(depth);
    let branch_limit = if depth >= 2 {
        INNER_BRANCH_LIMIT
    } else {
        usize::MAX
    };
    let actions = prune_actions(
        &checkpoint,
        run,
        enumerate_actions(run, cfg),
        branch_limit,
    );
    if actions.is_empty() {
        checkpoint.restore(run);
        return utility_after_action(run);
    }

    let mut best = f64::NEG_INFINITY;
    for action in actions {
        let value = branch_from_checkpoint(&checkpoint, run, &action, |run| {
            if depth <= 1 {
                utility_after_action(run)
            } else {
                search_value(run, depth - 1)
            }
        });
        let Some(value) = value else { continue };
        if value > best {
            best = value;
        }
    }
    checkpoint.restore(run);
    if best.is_finite() {
        best
    } else {
        utility_after_action(run)
    }
}

pub(crate) fn choose_blind_action(run: &mut RunState, depth: u32) -> Option<BlindAction> {
    if depth == 0 {
        return None;
    }
    let checkpoint = BlindPlanCheckpoint::capture(run);
    let root_limit = if depth >= 3 {
        INNER_BRANCH_LIMIT
    } else {
        usize::MAX
    };
    let actions = prune_actions(
        &checkpoint,
        run,
        enumerate_actions(run, ENUM_FULL),
        root_limit,
    );
    if actions.is_empty() {
        return None;
    }

    let mut best_action: Option<BlindAction> = None;
    let mut best_value = f64::NEG_INFINITY;
    for action in actions {
        let Some(value) = branch_from_checkpoint(&checkpoint, run, &action, |run| {
            if depth <= 1 {
                utility_after_action(run)
            } else {
                search_value(run, depth - 1)
            }
        }) else {
            continue;
        };
        if value > best_value {
            best_value = value;
            best_action = Some(action);
        }
    }
    checkpoint.restore(run);
    best_action
}

pub(crate) enum PlannedTurnOutcome {
    Continued,
    SecondWindForfeit,
    Failed,
}

/// Run one planner-chosen turn action. Returns `None` when the planner defers to legacy logic.
pub(crate) fn execute_planned_turn(
    run: &mut RunState,
    stats: &mut RunStats,
    log: bool,
    depth: u32,
    scoring_log: &mut Vec<BotScoringAction>,
    bus: &mut EventBus,
) -> Option<PlannedTurnOutcome> {
    let action = choose_blind_action(run, depth)?;
    match &action {
        BlindAction::Play(indices) => {
            let best_score = top_k_plays_in_hand(run, run.hand(), 1)
                .first()
                .map(|(s, _)| *s)
                .unwrap_or(0);
            crate::bot_log!(
                log,
                "      planner: play {} for ~{} pts (depth {})",
                fmt_indices(indices),
                best_score,
                depth
            );
            let hand_before = run.hand().to_vec();
            if let Some(breakdown) = score_breakdown_for_play_indices(run, &hand_before, indices) {
                relic_analytics::record_score_breakdown(stats, &breakdown);
            }
            run.clear_selection();
            for i in indices {
                run.toggle_select(*i);
            }
            let score_before = run.round_score;
            let committed = run.commit_selection_to_structure(bus);
            if committed == 0 && run.round_score == score_before {
                return Some(PlannedTurnOutcome::Failed);
            }
            stats.plays_used += 1;
            if drain_post_action_bus(run, bus, stats) == Some(PlayBlindOutcome::SecondWindForfeit) {
                return Some(PlannedTurnOutcome::SecondWindForfeit);
            }
            let play_delta = run.round_score.saturating_sub(score_before);
            if play_delta > 0 {
                scoring_log.push(BotScoringAction {
                    kind: "play".into(),
                    points: play_delta,
                    tiles: None,
                });
            }
            Some(PlannedTurnOutcome::Continued)
        }
        BlindAction::Discard(indices) => {
            crate::bot_log!(
                log,
                "      planner: discard {} (depth {})",
                fmt_indices(indices),
                depth
            );
            run.clear_selection();
            for i in indices {
                run.toggle_select(*i);
            }
            run.discard_selected(bus);
            stats.discards_used += 1;
            stats.strategic_discards += 1;
            *stats
                .discards_by_blind_slot
                .entry(blind_slot_key(run))
                .or_insert(0) += 1;
            if drain_post_action_bus(run, bus, stats) == Some(PlayBlindOutcome::SecondWindForfeit) {
                return Some(PlannedTurnOutcome::SecondWindForfeit);
            }
            Some(PlannedTurnOutcome::Continued)
        }
        BlindAction::TriggerStructure => {
            let best_score = top_k_plays_in_hand(run, run.hand(), 1)
                .first()
                .map(|(s, _)| *s)
                .unwrap_or(0);
            let score_before = run.round_score;
            let cash_in_tiles = run.structure_tiles().to_vec();
            let cash_in_sets = run.structure_sets().to_vec();
            let earned = run.trigger_structure_manual(bus);
            stats.structure_triggers += 1;
            stats.structure_trigger_points += earned;
            crate::bot_log!(
                log,
                "      planner: trigger structure for {} (best play {}, depth {})",
                earned,
                best_score,
                depth
            );
            if drain_post_action_bus(run, bus, stats) == Some(PlayBlindOutcome::SecondWindForfeit) {
                return Some(PlannedTurnOutcome::SecondWindForfeit);
            }
            let structure_delta = run.round_score.saturating_sub(score_before);
            if structure_delta > 0 {
                scoring_log.push(BotScoringAction {
                    kind: "structure".into(),
                    points: structure_delta,
                    tiles: format_meld_groups(&cash_in_tiles, &cash_in_sets),
                });
            }
            Some(PlannedTurnOutcome::Continued)
        }
        BlindAction::UseZodiac(z) => {
            let Some(idx) = consumable_slot(run, Consumable::Zodiac(*z)) else {
                return Some(PlannedTurnOutcome::Failed);
            };
            let _ = run.use_consumable(idx, bus);
            *stats.zodiacs_used.entry(z.name()).or_insert(0) += 1;
            crate::bot_log!(log, "      planner: use zodiac {:?} (depth {})", z, depth);
            if drain_post_action_bus(run, bus, stats) == Some(PlayBlindOutcome::SecondWindForfeit) {
                return Some(PlannedTurnOutcome::SecondWindForfeit);
            }
            Some(PlannedTurnOutcome::Continued)
        }
        BlindAction::UseTalisman(t) => {
            let lift = talisman_lift(run, *t);
            let Some(idx) = consumable_slot(run, Consumable::Talisman(*t)) else {
                return Some(PlannedTurnOutcome::Failed);
            };
            let _ = run.use_consumable(idx, bus);
            *stats.talismans_used.entry(t.name()).or_insert(0) += 1;
            crate::bot_log!(
                log,
                "      planner: use talisman {:?} (+{} lift, depth {})",
                t,
                lift,
                depth
            );
            if drain_post_action_bus(run, bus, stats) == Some(PlayBlindOutcome::SecondWindForfeit) {
                return Some(PlannedTurnOutcome::SecondWindForfeit);
            }
            Some(PlannedTurnOutcome::Continued)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::consumable::Consumable;
    use crate::core::zodiac::ZodiacKind;
    use crate::game::game_mode::GameMode;

    fn planner_test_run() -> RunState {
        RunState::new(GameMode::standard())
    }

    #[test]
    fn blind_utility_rises_with_round_score() {
        let mut run = planner_test_run();
        run.target_score = 1000;
        run.round_score = 100;
        let low = blind_utility(&run);
        run.round_score = 600;
        let high = blind_utility(&run);
        assert!(high > low);
    }

    #[test]
    fn choose_action_depth_one_unifies_play_and_discard() {
        let mut run = planner_test_run();
        assert!(choose_blind_action(&mut run, 1).is_some());
    }

    #[test]
    fn choose_action_depth_two() {
        let mut run = planner_test_run();
        assert!(choose_blind_action(&mut run, 2).is_some());
    }

    #[test]
    fn zodiac_in_action_set_when_held() {
        let mut run = planner_test_run();
        run.consumables.items.push(Consumable::Zodiac(ZodiacKind::Ox));
        let actions = enumerate_actions(&run, ENUM_FULL);
        assert!(actions.iter().any(|a| matches!(a, BlindAction::UseZodiac(ZodiacKind::Ox))));
    }
}
