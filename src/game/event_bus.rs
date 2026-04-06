//! Simple event queue for UI and core.

use crate::core::tile::Tile;

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum GameEvent {
    TileDrawn(Tile),
    TileDiscarded { slot_index: usize },
    ScoreUpdated(u32),
    RoundComplete { reached_target: bool },
    RunComplete,
    GameOver { final_score: u32 },
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
