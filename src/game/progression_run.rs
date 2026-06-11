//! Run-record capture and hydration (needs live [`RunState`](crate::game::run::RunState)).

use crate::core::chamber_target::FINAL_WING;
use crate::core::progression::{RunOutcome, RunRecord};
use crate::core::rules::ChamberKind;
use crate::core::run_chronicle;
use crate::game::ordeal::OrdealKindExt;
use crate::game::run::RunState;
use mahjuro_types::GameOverReason;

pub fn run_record_defeat_reason(record: &RunRecord) -> Option<GameOverReason> {
    match record.outcome {
        RunOutcome::Victory => None,
        RunOutcome::Defeat { reason } => Some(reason),
    }
}

pub fn hydrate_game_over_run(record: &RunRecord, run: &mut RunState) {
    run.run_number = record.run_number;
    run.wing = record.final_wing;
    run.chamber = record.final_chamber;
    run.round_score = record.round_score;
    run.target_score = record.target_score;
    run.total_score_earned = record.total_score_earned;
    run.yen = record.final_yen;
    run.plays_remaining = record.plays_remaining;
    run.discards_remaining = record.discards_remaining;
    run.plays_max = record.plays_max;
    run.discards_max = record.discards_max;
    run.tiles_played = record.tiles_played;
    run.tiles_discarded = record.tiles_discarded;
    run.times_restocked = record.times_restocked;
    run.best_structure_score = record.best_structure_score;
    run.best_structure_name = record.best_structure_name.clone();
    run.best_hand_tiles = record.best_hand_tiles.clone();
    run.score_after_wing = record.score_after_wing.clone();
    run.yaku_times_played = record.yaku_times_played.clone();
    run.defeat_memorial_kind = record.memorial_kind;
    run.ordeal.upcoming = record.final_ordeal;
    if record.final_chamber == ChamberKind::Ordeal && record.final_ordeal.is_some() {
        run.resolve_upcoming_ordeal();
    }
}

pub fn run_record_from_run(run: &RunState, outcome: RunOutcome) -> RunRecord {
    let timestamp_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let final_wing = final_wing_for_record(run, outcome);
    let final_ordeal = if run.chamber == ChamberKind::Ordeal {
        run.ordeal.upcoming
    } else {
        None
    };
    RunRecord {
        timestamp_unix,
        run_number: run.run_number,
        outcome,
        final_wing,
        final_chamber: run.chamber,
        final_ordeal,
        round_score: run.round_score,
        target_score: run.target_score,
        total_score_earned: run.total_score_earned,
        final_yen: run.yen,
        plays_remaining: run.plays_remaining,
        discards_remaining: run.discards_remaining,
        plays_max: run.plays_max,
        discards_max: run.discards_max,
        tiles_played: run.tiles_played,
        tiles_discarded: run.tiles_discarded,
        times_restocked: run.times_restocked,
        best_structure_score: run.best_structure_score,
        best_structure_name: run.best_structure_name.clone(),
        yaku_times_played: run.yaku_times_played.clone(),
        relics_owned: run.relics.active.clone(),
        consumables_owned: run.consumables.items.clone(),
        tile_material: run.mode.tile_material,
        season: run.mode.season,
        tutorial_run: false,
        memorial_kind: run.defeat_memorial_kind,
        best_hand_tiles: run.best_hand_tiles.clone(),
        score_after_wing: finalize_score_after_wing(run, final_wing),
        chronicle: finalize_run_chronicle(run, outcome, final_wing),
        duration_secs: run_duration_secs(run),
    }
}

fn final_wing_for_record(run: &RunState, outcome: RunOutcome) -> u32 {
    if matches!(outcome, RunOutcome::Victory) {
        run.wing.min(FINAL_WING)
    } else {
        run.wing
    }
}

fn run_duration_secs(run: &RunState) -> u32 {
    let end = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let start = run.chronicle.started_unix;
    if end > start {
        (end - start).min(u32::MAX as u64) as u32
    } else {
        0
    }
}

fn finalize_run_chronicle(
    run: &RunState,
    outcome: RunOutcome,
    final_wing: u32,
) -> run_chronicle::RunChronicle {
    let mut chronicle = run.chronicle.clone();
    let victory = matches!(outcome, RunOutcome::Victory);
    chronicle.finalize_for_outcome(
        victory,
        run.total_score_earned,
        final_wing,
        run.plays_remaining,
    );

    if let Some(bd) = run.last_breakdown.as_ref() {
        chronicle.set_terminal_breakdown(bd, run.mode.season);
        if victory {
            chronicle.signature_hand = Some(run_chronicle::signature_from_breakdown(
                bd,
                &run.best_hand_tiles,
            ));
        }
    }

    if !victory {
        let ordeal_name = if run.chamber == ChamberKind::Ordeal {
            run.ordeal.upcoming.map(|b| b.name().to_string())
        } else {
            None
        };
        chronicle.record_run_end_defeat(
            final_wing,
            run.chamber,
            ordeal_name.as_deref(),
            run.round_score,
        );
    }

    if chronicle.signature_hand.is_none() && !run.best_hand_tiles.is_empty() {
        chronicle.signature_hand = Some(run_chronicle::SignatureHandRecord {
            tiles: run.best_hand_tiles.clone(),
            yaku: run.yaku_times_played.keys().copied().take(6).collect(),
            yaku_han_total: 0,
            dora_count: 0,
            aka_dora_count: 0,
            ura_dora_count: 0,
        });
    }

    chronicle
}

fn finalize_score_after_wing(run: &RunState, final_wing: u32) -> Vec<(u32, u64)> {
    let mut v = run.score_after_wing.clone();
    let terminal = (final_wing, run.total_score_earned);
    if v.last().copied() != Some(terminal) {
        v.push(terminal);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::event_bus::EventBus;

    #[test]
    fn victory_record_after_final_boss_uses_final_playable_wing() {
        let mut run = RunState::new_demo();
        let mut bus = EventBus::default();
        let total_score = 62_000;
        run.wing = FINAL_WING;
        run.chamber = ChamberKind::Ordeal;
        run.upcoming_chamber = ChamberKind::Ordeal;
        run.total_score_earned = total_score;
        run.round_score = 40_000;
        run.target_score = run.chamber_score_target(ChamberKind::Ordeal);
        run.plays_remaining = 2;

        run.advance_round(&mut bus);
        assert!(run.is_run_complete());

        let record = run_record_from_run(&run, RunOutcome::Victory);

        assert_eq!(record.final_wing, FINAL_WING);
        assert_eq!(
            record.score_after_wing.last(),
            Some(&(FINAL_WING, total_score))
        );
        assert!(
            !record
                .score_after_wing
                .iter()
                .any(|&(wing, _)| wing > FINAL_WING)
        );
        assert_eq!(
            record.chronicle.victory_tier,
            Some(run_chronicle::VictoryTier::Exceptional)
        );
        assert!(
            record
                .chronicle
                .milestones
                .iter()
                .any(|milestone| milestone == "Speed Clear")
        );
    }
}
