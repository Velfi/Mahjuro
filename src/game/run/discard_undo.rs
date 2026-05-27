//! Snapshot of run state immediately before a discard, for optional undo.

use crate::core::deck::Wall;
use crate::core::relic::{RelicId, RelicState};
use crate::game::engine_state::GameplayCoreState;

/// Captures enough of [`super::RunState`] to restore hand, wall, counters,
/// and relic tutorial flags after a discard (+ refill) while no other action ran.
#[derive(Clone, Debug)]
pub struct DiscardUndoSnapshot {
    hand: Vec<crate::core::tile::Tile>,
    selected: Vec<bool>,
    discards_remaining: u32,
    wall: Wall,
    yen: i32,
    tiles_discarded: u32,
    times_restocked: u32,
    relic_counters: std::collections::BTreeMap<RelicId, i32>,
    relics: RelicState,
    relic_activations_len: usize,
}

impl DiscardUndoSnapshot {
    pub fn capture(run: &super::RunState) -> Self {
        Self {
            hand: run.hand.clone(),
            selected: run.selected.clone(),
            discards_remaining: run.discards_remaining,
            wall: run.wall.clone(),
            yen: run.yen,
            tiles_discarded: run.tiles_discarded,
            times_restocked: run.times_restocked,
            relic_counters: run.relic_counters.clone(),
            relics: run.relics.clone(),
            relic_activations_len: run.relic_activations.len(),
        }
    }
}

impl super::RunState {
    pub fn apply_discard_undo(
        &mut self,
        snap: DiscardUndoSnapshot,
        bus: Option<&mut crate::game::event_bus::EventBus>,
    ) {
        let yen_delta = snap.yen - self.yen;
        self.discards_remaining = snap.discards_remaining;
        self.wall = snap.wall;
        self.yen = snap.yen;
        self.tiles_discarded = snap.tiles_discarded;
        self.times_restocked = snap.times_restocked;
        self.relic_counters = snap.relic_counters;
        self.relics = snap.relics;
        self.relic_activations.truncate(snap.relic_activations_len);
        let hand = snap.hand;
        let selected = snap.selected;
        GameplayCoreState::with_run_mut(self, |core| {
            core.hand = hand;
            core.selected = selected;
        });
        self.restamp_hand_enhancements();
        if yen_delta != 0 {
            self.notify_run_yen_changed(yen_delta, bus);
        }
    }       
}
