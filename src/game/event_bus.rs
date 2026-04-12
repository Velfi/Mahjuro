//! Simple event queue for UI and core.

use crate::core::tile::Tile;

/// Itemized breakdown of the gold awarded for clearing a blind. Mirrors
/// the calculation in `RunState::play_selected` so the celebration UI can
/// show the player exactly where each coin came from.
#[derive(Clone, Copy, Debug, Default)]
pub struct RoundPayout {
    pub base_reward: u32,
    pub unused_play_bonus: u32,
    pub interest: u32,
    pub green_luck_bonus: u32,
    pub total: u32,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum GameEvent {
    TileDrawn(Tile),
    TileDiscarded {
        slot_index: usize,
    },
    ScoreUpdated(u64),
    /// A scoring cascade just revealed step `index` of the breakdown.
    /// Fires once per step, on the frame the reveal edge is crossed.
    ScoreStepRevealed {
        index: usize,
    },
    /// Gold amount changed (coins flying into/out of dish).
    GoldChanged {
        delta: i32,
    },
    /// A scoring cascade just transitioned into its final-total beat.
    /// Fires once per cascade, on the frame the transition happens.
    ScoreCascadeFinal,
    RoundComplete {
        reached_target: bool,
        payout: RoundPayout,
    },
    RunComplete,
    GameOver {
        final_score: u64,
    },
    /// Tiles permanently destroyed via the Kiln talisman.
    TilesDestroyed {
        count: usize,
    },
    /// A tile pack was purchased in the shop.
    PackBought,
    /// The pack opening celebration started (foil tearing).
    PackOpened,
    /// An individual tile was revealed during pack opening.
    PackTileRevealed,
    /// A zodiac ribbon close-up celebration started (shop).
    ZodiacReveal,
    /// A zodiac was consumed and levelled up a yaku.
    ZodiacLevelUp,
    /// A relic's effect just fired (scoring, round-end, discard, consumable
    /// interaction, etc.). Drives the glow + wiggle animation on every scene
    /// that renders relics.
    RelicActivated(crate::core::relic::RelicId),
    /// A single hand scored at least the entire blind target — candles
    /// flare up and sparks fly.
    CandleFlare,
    /// Melds were committed into the structure (mirror).
    StructureCommitted,
    /// Tutorial milestone celebration (first pair, first triplet, etc.).
    TutorialMilestone(crate::game::tutorial::TutorialMilestone),
    /// An action was rejected as invalid (e.g. bad meld, structure full).
    InvalidAction,
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
