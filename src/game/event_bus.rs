//! Simple event queue for UI and core.

use crate::core::tile::Tile;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum GameEvent {
    TileDrawn(Tile),
    TileDiscarded {
        slot_index: usize,
    },
    ScoreUpdated(u32),
    /// A scoring cascade just revealed step `index` of the breakdown.
    /// Fires once per step, on the frame the reveal edge is crossed.
    ScoreStepRevealed {
        index: usize,
    },
    /// A scoring cascade just transitioned into its final-total beat.
    /// Fires once per cascade, on the frame the transition happens.
    ScoreCascadeFinal,
    RoundComplete {
        reached_target: bool,
    },
    RunComplete,
    GameOver {
        final_score: u32,
    },
}

#[derive(Default)]
pub struct EventBus {
    pub queue: Vec<GameEvent>,
}

impl EventBus {
    pub fn push(&mut self, e: GameEvent) {
        self.queue.push(e);
    }

    pub fn drain(&mut self) -> impl Iterator<Item = GameEvent> + '_ {
        self.queue.drain(..)
    }
}
