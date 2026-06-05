//! Simple event queue for UI and core.

pub use crate::sfx_id::SfxId;
pub use mahjuro_types::GameOverReason;

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

pub enum GameEvent {
    TileDrawn,
    TileDiscarded,
    ScoreUpdated,
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
    ScoreCascadeFinal {
        earned: u64,
    },
    RoundComplete {
        reached_target: bool,
        payout: RoundPayout,
    },
    GameOver {
        reason: GameOverReason,
    },
    /// Tiles permanently destroyed (e.g. Taotie devouring honors).
    TilesDestroyed,
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
    /// An action was rejected as invalid (e.g. bad meld, structure full).
    InvalidAction,
    /// Play a one-shot UI / tutorial sound (no gameplay side effects).
    UiSound(SfxId),
    /// Start the hold-to-act windup bed (`reel_up`) on a dedicated sink.
    HoldWindupStart,
    /// Stop an in-progress hold windup (early release / focus change).
    HoldWindupStop,
    /// Play a relic's stinger without counting it as an activation
    /// (e.g. when a relic is purchased from the shop).
    PlayRelicStinger(crate::core::relic::RelicId),
    /// Player committed to an ordeal chamber — App layer bumps
    /// `PlayerProgress::ordeal_times_encountered` and saves.
    OrdealEncountered(crate::core::ordeal::OrdealKind),
    /// Ordeal chamber cleared with target reached — App layer bumps
    /// `PlayerProgress::ordeal_times_defeated` and saves. Not emitted
    /// during the tutorial finale.
    OrdealDefeated(crate::core::ordeal::OrdealKind),
    /// Player just bought a talisman from the shop — App layer bumps
    /// `PlayerProgress::talisman_times_purchased` and saves.
    TalismanPurchased(crate::core::talisman::TalismanKind),
    /// Player consumed a talisman from the dish — App layer bumps
    /// `PlayerProgress::talisman_times_used` and saves.
    TalismanUsed(crate::core::talisman::TalismanKind),
    /// Memorial remnant used from the dish (in-round effect).
    MemorialTalismanUsed(crate::core::memorial_talisman::MemorialTalismanKind),
    /// Structure cash-in awarded this yaku. App layer bumps
    /// `PlayerProgress::yaku_times_scored` and saves.
    YakuScored(crate::core::yaku::YakuKind),
    /// Gameplay code asks the App layer to fire a Steam achievement.
    /// Used for milestones the gameplay layer detects but cannot unlock
    /// directly (the `steam` module is owned by `main`, not `game`).
    AchievementUnlocked(crate::steam::Achievement),
    /// A fragile relic burned — reveal its successor in Collection
    /// (`PlayerProgress::discovered_transformation_successors`). Shops use
    /// run burn flags only; successors are not added to meta `available_relics`.
    TransformationSuccessorDiscovered(crate::core::relic::RelicId),
    /// Player focused a catalog cubby in Archive — App layer marks it seen in
    /// `PlayerProgress::archive_seen_*` and saves.
    ArchiveItemSeen(crate::core::archive_seen::ArchiveSeenMark),
    /// First Archive open on a legacy profile — seed seen sets from the current catalog.
    ArchiveSeedSeenIfNeeded,
    /// Informational dialog only. The main loop turns this into a modal on
    /// the next bus drain.
    InfoModal {
        title: String,
        body: String,
    },
    /// Show in-game help for remapping controllers (SDL reads the OS / Steam
    /// driver layout; the Options menu can swap face-button semantics).
    OpenControllerMappingHelp,
    /// Brief hallway bulb flicker + brownout (`RoomGltfBrownout` on the App).
    RoomGltfBrownout,
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
