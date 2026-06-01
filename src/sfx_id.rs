//! Sound identifiers (no rodio). Playback lives in [`crate::audio`] when enabled.

/// Background music + win/loss jingles (under `assets/audio/music/`).
///
/// Looping tracks (`MainMenu`, `Gameplay`, `GameplayIntense`, `Shop`) play via
/// [`AudioManager::set_music_track`] / [`AudioManager::set_gameplay_music`].
/// One-shot intros (`GameplayIntro`, `GameplayIntenseIntro`) and win/loss
/// stingers (`ChamberWin`, `ChamberLoss`, `OrdealWin`, `OrdealLoss`) play once
/// on the music sink but use the **SFX** volume slider (same as [`AmbientId`]
/// beds). Intros hand off to their paired loop via
/// [`AudioManager::play_music_intro_then_loop`]; stingers via
/// [`AudioManager::play_music_jingle`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MusicId {
    MainMenu,
    /// Regular blind loop (after [`MusicId::GameplayIntro`]).
    Gameplay,
    /// Boss blind loop (after [`MusicId::GameplayIntenseIntro`]).
    GameplayIntense,
    /// One-shot lead-in before [`MusicId::Gameplay`].
    GameplayIntro,
    /// One-shot lead-in before [`MusicId::GameplayIntense`].
    GameplayIntenseIntro,
    Shop,
    Credits,
    /// Stinger when the player clears a Small/Big blind.
    ChamberWin,
    /// Stinger when the player fails a Small/Big blind.
    ChamberLoss,
    /// Stinger when the player defeats a Boss blind.
    OrdealWin,
    /// Stinger when the player fails a Boss blind.
    OrdealLoss,
}

/// Looping scene ambience (under `assets/audio/ambient/`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AmbientId {
    MainMenuRain,
    /// Incandescent bulb hum in the pick-blind hallway.
    HallwayBulbBuzz,
}

/// Sound effect identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SfxId {
    UiConfirm,
    UiCancel,
    TileClick,
    TileSelect,
    TileDeselect,
    TilePlace,
    TileDiscard,
    ScoreReveal,
    ScoreStep,
    /// Base sample for the cascade tick. Cycled through eight semitone-spaced
    /// pitches per step via [`AudioManager::play_score_tick`] so the reveal
    /// sequence audibly climbs a scale.
    ScoreTick,
    /// Brassy hit jingle layered on top of `ScoreFinal` to add weight to
    /// the closing crescendo.
    ScoreCrescendo,
    ScoreFinal,
    RelicPickup,
    InvalidAction,
    CoinDrop,
    RoundWin,
    GameOver,
    /// Tile pack purchased in the shop.
    PackBuy,
    /// Tile pack foil wrapper tearing open.
    PackOpen,
    /// Individual tile revealed during pack opening celebration.
    PackTileReveal,
    /// Zodiac ribbon close-up reveal (mystical shimmer).
    ZodiacReveal,
    /// Zodiac consumed — yaku levelled up (positive stinger).
    ZodiacLevelUp,
    /// Rising whoosh when candles flare up on a blind-breaking hand.
    CandleFlareWhoosh,
    /// Heavy impact sting layered on top of the whoosh.
    CandleFlareImpact,
    /// Melds locked into the structure bank (distinct from [`Self::TilePlace`]).
    StructureCommit,
    /// Focus moved to a hand tile.
    FocusHandTile,
    /// Focus moved to an action-bar button (Play, Discard, Sort, etc.).
    FocusButton,
    /// Focus moved to a consumable slot.
    FocusConsumable,
    /// Focus moved to a relic on the shelf.
    FocusRelic,
    /// Focus moved to a UI peg (hand-size, discards, etc.).
    FocusPeg,
    /// Focus moved to the gold readout.
    FocusGold,
    /// Focus moved to a yaku-progress tablet.
    FocusYakuTablet,
    /// Focus moved to the dora indicator stand.
    FocusDora,
    /// A gameplay round begins.
    RoundStart,
    /// Player skipped a blind on pick_chamber (lights brownout).
    ChamberSkipped,
    /// Brief bulb flicker + dip when room GLB lighting brownouts.
    BrownoutFlicker,
    /// Occasional floor/board creak in shop / hallway / archive rooms.
    RoomCreak,
    /// The shooting-star cascade transition fires (dramatic scene change).
    StarShimmer,
    /// Relic / zodiac / talisman purchased in the shop.
    Purchase,
    /// Relic / consumable sold back for gold.
    Sell,
    /// Pause menu opened.
    Pause,
    /// Pause menu dismissed (game resumed).
    Unpause,
    /// One chime per dora tile in a scored hand. Queued sequentially so
    /// multiple dora play as a rolling ding-ding-ding rather than a single
    /// stacked hit.
    DoraScored,
    /// Structure cashed in (manual Trigger or auto-full). Bright bell-stack
    /// chime that precedes the scoring cascade.
    CashIn,
    /// Chips/×/mult trio snaps together at the start of the hand-off
    /// tween — signals "the accounting is done, here's the total".
    CascadeMerge,
    /// Merged total leaves the pad and begins its flight toward the score
    /// reel — a rising whoosh that cues the upward motion.
    CascadeLaunch,
    /// Merged total lands in the score reel at the end of the flight —
    /// crisp impact paired with the reel finishing its tick-up.
    CascadeLand,
    /// One stinger per yaku detected in a scored hand. Emitted from the
    /// scoring loop so stacked yaku play as a rolling sequence of distinct
    /// cues rather than a single hit.
    YakuTanyao,
    YakuToitoi,
    YakuFullHand,
    YakuYakuhai,
    YakuIipeikou,
    YakuSanshokuDoujun,
    YakuIttsu,
    YakuHonitsu,
    YakuChinitsu,
    YakuJunchan,
    YakuHonroutou,
    YakuChiitoitsu,
    YakuKokushiMusou,
    YakuChickenHand,
    YakuChanta,
    YakuRyanpeikou,
    YakuSanshokuDoukou,
    YakuPinfu,
    /// Played ~1 second after the victory screen appears.
    Victory,
    /// Alternate victory stinger; `Victory` / `Victory2` are picked at random.
    Victory2,
    /// Played ~1 second after the defeat screen appears.
    Defeat,
    /// Played whenever the main menu (start screen) is entered.
    MainMenuEnter,
    /// Generic level-up stinger.
    LevelUp,
    /// Tiles permanently destroyed (e.g. Taotie).
    TilesDestroyed,
    /// Boss blind encountered — ominous sting as the round begins.
    OrdealEncountered,
    /// Boss blind defeated — triumphant sting.
    OrdealDefeated,
    /// Talisman purchased from the shop.
    TalismanPurchased,
    /// Talisman consumed from the dish.
    TalismanUsed,
    /// Looping mechanical whir while gameplay score odometer rollers spin.
    RollersSpin,
}

/// All SFX variants in display order. Single source of truth shared by the
/// startup loader and the debug "Sound Effects Test" overlay so they can't drift.
pub fn all_sfx_ids() -> &'static [SfxId] {
    &[
        SfxId::UiConfirm,
        SfxId::UiCancel,
        SfxId::TileClick,
        SfxId::TileSelect,
        SfxId::TileDeselect,
        SfxId::TilePlace,
        SfxId::TileDiscard,
        SfxId::ScoreReveal,
        SfxId::ScoreStep,
        SfxId::ScoreTick,
        SfxId::ScoreCrescendo,
        SfxId::ScoreFinal,
        SfxId::RelicPickup,
        SfxId::InvalidAction,
        SfxId::CoinDrop,
        SfxId::RoundWin,
        SfxId::GameOver,
        SfxId::PackBuy,
        SfxId::PackOpen,
        SfxId::PackTileReveal,
        SfxId::ZodiacReveal,
        SfxId::ZodiacLevelUp,
        SfxId::CandleFlareWhoosh,
        SfxId::CandleFlareImpact,
        SfxId::StructureCommit,
        SfxId::FocusHandTile,
        SfxId::FocusButton,
        SfxId::FocusConsumable,
        SfxId::FocusRelic,
        SfxId::FocusPeg,
        SfxId::FocusGold,
        SfxId::FocusYakuTablet,
        SfxId::FocusDora,
        SfxId::RoundStart,
        SfxId::ChamberSkipped,
        SfxId::BrownoutFlicker,
        SfxId::RoomCreak,
        SfxId::StarShimmer,
        SfxId::Purchase,
        SfxId::Sell,
        SfxId::Pause,
        SfxId::Unpause,
        SfxId::DoraScored,
        SfxId::CashIn,
        SfxId::CascadeMerge,
        SfxId::CascadeLaunch,
        SfxId::CascadeLand,
        SfxId::YakuTanyao,
        SfxId::YakuToitoi,
        SfxId::YakuFullHand,
        SfxId::YakuYakuhai,
        SfxId::YakuIipeikou,
        SfxId::YakuSanshokuDoujun,
        SfxId::YakuIttsu,
        SfxId::YakuHonitsu,
        SfxId::YakuChinitsu,
        SfxId::YakuJunchan,
        SfxId::YakuHonroutou,
        SfxId::YakuChiitoitsu,
        SfxId::YakuKokushiMusou,
        SfxId::YakuChickenHand,
        SfxId::YakuChanta,
        SfxId::YakuRyanpeikou,
        SfxId::YakuSanshokuDoukou,
        SfxId::YakuPinfu,
        SfxId::Victory,
        SfxId::Victory2,
        SfxId::Defeat,
        SfxId::MainMenuEnter,
        SfxId::LevelUp,
        SfxId::TilesDestroyed,
        SfxId::OrdealEncountered,
        SfxId::OrdealDefeated,
        SfxId::TalismanPurchased,
        SfxId::TalismanUsed,
        SfxId::RollersSpin,
    ]
}

impl SfxId {
    pub fn for_yaku(kind: crate::core::yaku::YakuKind) -> SfxId {
        use crate::core::yaku::YakuKind;
        match kind {
            YakuKind::Tanyao => SfxId::YakuTanyao,
            YakuKind::Toitoi => SfxId::YakuToitoi,
            YakuKind::FullHand => SfxId::YakuFullHand,
            YakuKind::Yakuhai => SfxId::YakuYakuhai,
            YakuKind::Iipeikou => SfxId::YakuIipeikou,
            YakuKind::SanshokuDoujun => SfxId::YakuSanshokuDoujun,
            YakuKind::Ittsu => SfxId::YakuIttsu,
            YakuKind::Honitsu => SfxId::YakuHonitsu,
            YakuKind::Chinitsu => SfxId::YakuChinitsu,
            YakuKind::Junchan => SfxId::YakuJunchan,
            YakuKind::Honroutou => SfxId::YakuHonroutou,
            YakuKind::Chiitoitsu => SfxId::YakuChiitoitsu,
            YakuKind::KokushiMusou => SfxId::YakuKokushiMusou,
            YakuKind::ChickenHand => SfxId::YakuChickenHand,
            YakuKind::Chanta => SfxId::YakuChanta,
            YakuKind::Ryanpeikou => SfxId::YakuRyanpeikou,
            YakuKind::SanshokuDoukou => SfxId::YakuSanshokuDoukou,
            YakuKind::Pinfu => SfxId::YakuPinfu,
        }
    }
}
