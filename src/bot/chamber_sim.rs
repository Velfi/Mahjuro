//! Immutable checkpoints and branch helpers for blind-turn expectimax search.
//!
//! Checkpoints capture the full sim-relevant [`RunState`] slice once; sibling
//! branches restore from the same checkpoint without deep-cloning the wall
//! (see [`Wall`](crate::core::deck::Wall) copy-on-write storage).

use crate::core::consumable::Consumable;
use crate::core::hand::DetectedMeld;
use crate::core::relic::{RelicId, RelicState};
use crate::core::talisman::TalismanKind;
use crate::core::tile::{Tile, TileEnhancement};
use crate::core::zodiac::{YakuLevels, ZodiacKind};
use crate::game::event_bus::{EventBus, GameEvent};
use crate::game::run::RunState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChamberAction {
    Play(Vec<usize>),
    Discard(Vec<usize>),
    TriggerStructure,
    UseZodiac(ZodiacKind),
    UseTalisman(TalismanKind),
}

/// Full run slice needed to rewind a blind-turn simulation branch.
#[derive(Clone, Debug)]
pub(crate) struct ChamberPlanCheckpoint {
    hand: Vec<Tile>,
    selected: Vec<bool>,
    structure_sets: Vec<DetectedMeld>,
    structure_tiles: Vec<Tile>,
    wall: crate::core::deck::Wall,
    round_score: u64,
    plays_remaining: u32,
    discards_remaining: u32,
    relic_counters: std::collections::BTreeMap<RelicId, i32>,
    relics: RelicState,
    scored_last_turn: bool,
    joker_used: bool,
    full_hand_played_this_round: bool,
    played_yaku_this_round: Vec<crate::core::yaku::YakuKind>,
    tiles_discarded: u32,
    times_restocked: u32,
    tile_enhancements: std::collections::BTreeMap<u32, TileEnhancement>,
    global_buff_enhancement: Option<TileEnhancement>,
    yaku_levels: YakuLevels,
    consumables: crate::core::consumable::ConsumableInventory,
    ordeal: crate::game::run::OrdealState,
}

impl ChamberPlanCheckpoint {
    pub(crate) fn capture(run: &RunState) -> Self {
        Self {
            hand: run.hand().to_vec(),
            selected: run.selected_slice().to_vec(),
            structure_sets: run.structure_sets().to_vec(),
            structure_tiles: run.structure_tiles().to_vec(),
            wall: run.wall.clone(),
            round_score: run.round_score,
            plays_remaining: run.plays_remaining,
            discards_remaining: run.discards_remaining,
            relic_counters: run.relic_counters.clone(),
            relics: run.relics.clone(),
            scored_last_turn: run.scored_last_turn,
            joker_used: run.joker_used,
            full_hand_played_this_round: run.full_hand_played_this_round,
            played_yaku_this_round: run.played_yaku_this_round.clone(),
            tiles_discarded: run.tiles_discarded,
            times_restocked: run.times_restocked,
            tile_enhancements: run.tile_enhancements.clone(),
            global_buff_enhancement: run.global_buff_enhancement,
            yaku_levels: run.yaku_levels.clone(),
            consumables: run.consumables.clone(),
            ordeal: run.ordeal.clone(),
        }
    }

    pub(crate) fn restore(&self, run: &mut RunState) {
        run.wall = self.wall.clone();
        run.round_score = self.round_score;
        run.plays_remaining = self.plays_remaining;
        run.discards_remaining = self.discards_remaining;
        run.relic_counters = self.relic_counters.clone();
        run.relics = self.relics.clone();
        run.scored_last_turn = self.scored_last_turn;
        run.joker_used = self.joker_used;
        run.full_hand_played_this_round = self.full_hand_played_this_round;
        run.played_yaku_this_round = self.played_yaku_this_round.clone();
        run.tiles_discarded = self.tiles_discarded;
        run.times_restocked = self.times_restocked;
        run.tile_enhancements = self.tile_enhancements.clone();
        run.global_buff_enhancement = self.global_buff_enhancement;
        run.yaku_levels = self.yaku_levels.clone();
        run.consumables = self.consumables.clone();
        run.ordeal = self.ordeal.clone();
        run.set_gameplay_core_slice(
            self.hand.clone(),
            self.selected.clone(),
            self.structure_sets.clone(),
            self.structure_tiles.clone(),
        );
        run.relic_activations.clear();
        run.restamp_hand_enhancements();
    }
}

pub(crate) fn sim_drain_bus(run: &mut RunState, bus: &mut EventBus) {
    for ev in bus.drain() {
        if let GameEvent::RoundComplete {
            payout,
            reached_target: true,
        } = ev
        {
            run.apply_yen_reward(payout.total as i32, None);
        }
    }
}

fn consumable_slot(run: &RunState, target: Consumable) -> Option<usize> {
    run.consumables.items.iter().position(|c| *c == target)
}

pub(crate) fn apply_chamber_action(
    run: &mut RunState,
    action: &ChamberAction,
    bus: &mut EventBus,
) -> bool {
    match action {
        ChamberAction::Play(indices) => {
            run.clear_selection();
            for i in indices {
                run.toggle_select(*i);
            }
            let ok = run.commit_selection_to_structure(bus) > 0;
            if ok {
                sim_drain_bus(run, bus);
            }
            ok
        }
        ChamberAction::Discard(indices) => {
            run.clear_selection();
            for i in indices {
                run.toggle_select(*i);
            }
            let ok = run.discard_selected(bus) > 0;
            if ok {
                sim_drain_bus(run, bus);
            }
            ok
        }
        ChamberAction::TriggerStructure => {
            let earned = run.trigger_structure_manual(bus);
            if earned > 0 {
                sim_drain_bus(run, bus);
            }
            earned > 0
        }
        ChamberAction::UseZodiac(z) => {
            let Some(idx) = consumable_slot(run, Consumable::Zodiac(*z)) else {
                return false;
            };
            run.use_consumable(idx, bus).is_some()
        }
        ChamberAction::UseTalisman(t) => {
            let Some(idx) = consumable_slot(run, Consumable::Talisman(*t)) else {
                return false;
            };
            run.use_consumable(idx, bus).is_some()
        }
    }
}

/// Restore `checkpoint`, apply `action`, then run `eval` on the resulting state.
pub(crate) fn branch_from_checkpoint<T>(
    checkpoint: &ChamberPlanCheckpoint,
    run: &mut RunState,
    action: &ChamberAction,
    eval: impl FnOnce(&mut RunState) -> T,
) -> Option<T> {
    checkpoint.restore(run);
    let mut bus = EventBus::default();
    if !apply_chamber_action(run, action, &mut bus) {
        return None;
    }
    Some(eval(run))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::game_mode::GameMode;

    #[test]
    fn checkpoint_restores_wall_after_branch_mutation() {
        let mut run = RunState::new(GameMode::standard());
        let remaining_before = run.wall.remaining();
        let checkpoint = ChamberPlanCheckpoint::capture(&run);

        checkpoint.restore(&mut run);
        for _ in 0..5 {
            run.wall.draw();
        }
        assert_eq!(run.wall.remaining(), remaining_before.saturating_sub(5));

        checkpoint.restore(&mut run);
        assert_eq!(run.wall.remaining(), remaining_before);
    }
}
