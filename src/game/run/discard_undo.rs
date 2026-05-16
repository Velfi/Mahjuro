//! Snapshot of run state immediately before a discard, for optional undo.

use crate::core::deck::Wall;
use crate::core::relic::{RelicId, RelicState};
use crate::game::engine_state::GameplayCoreState;
use crate::game::tutorial::TutorialState;

/// Captures enough of [`super::RunState`] to restore hand, wall, counters,
/// and relic tutorial flags after a discard (+ refill) while no other action ran.
#[derive(Clone, Debug)]
pub struct DiscardUndoSnapshot {
    hand: Vec<crate::core::tile::Tile>,
    selected: Vec<bool>,
    discards_remaining: u32,
    wall: Wall,
    gold: i32,
    tiles_discarded: u32,
    times_restocked: u32,
    relic_counters: std::collections::BTreeMap<RelicId, i32>,
    relics: RelicState,
    relic_activations_len: usize,
    tutorial: Option<TutorialState>,
}

impl DiscardUndoSnapshot {
    pub fn capture(run: &super::RunState) -> Self {
        Self {
            hand: run.hand.clone(),
            selected: run.selected.clone(),
            discards_remaining: run.discards_remaining,
            wall: run.wall.clone(),
            gold: run.gold,
            tiles_discarded: run.tiles_discarded,
            times_restocked: run.times_restocked,
            relic_counters: run.relic_counters.clone(),
            relics: run.relics.clone(),
            relic_activations_len: run.relic_activations.len(),
            tutorial: run.tutorial.clone(),
        }
    }
}

impl super::RunState {
    pub fn apply_discard_undo(
        &mut self,
        snap: DiscardUndoSnapshot,
        bus: Option<&mut crate::game::event_bus::EventBus>,
    ) {
        let gold_delta = snap.gold - self.gold;
        self.discards_remaining = snap.discards_remaining;
        self.wall = snap.wall;
        self.gold = snap.gold;
        self.tiles_discarded = snap.tiles_discarded;
        self.times_restocked = snap.times_restocked;
        self.relic_counters = snap.relic_counters;
        self.relics = snap.relics;
        self.tutorial = snap.tutorial;
        self.relic_activations.truncate(snap.relic_activations_len);
        let hand = snap.hand;
        let selected = snap.selected;
        GameplayCoreState::with_run_mut(self, |core| {
            core.hand = hand;
            core.selected = selected;
        });
        self.restamp_hand_enhancements();
        if gold_delta != 0 {
            self.notify_run_gold_changed(gold_delta, bus);
        }
    }
}
