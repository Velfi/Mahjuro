//! Typed achievement IDs. The `api_name` mapping is the contract with
//! the Steamworks partner backend — these strings must match the API
//! Names configured at <https://partner.steamgames.com/apps/achievements/4636490>.
//!
//! Adding a new achievement: add a variant here, give it an `api_name`,
//! configure the matching achievement in the partner backend, and call
//! `SteamClient::unlock_achievement` from wherever the trigger fires.

/// All Mahjuro achievements. Designed as a funnel — each one marks a
/// checkpoint where players commonly drop off, so completion rates on
/// the Steam achievement page act as a free retention dashboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Achievement {
    /// Finished or skipped the tutorial. Confirms onboarding is done.
    TutorialComplete,
    /// Scored their first structure. The earliest signal that the core
    /// scoring loop "clicked" for the player.
    FirstStructure,
    /// Cleared their first chamber after the tutorial.
    FirstBlindCleared,
    /// Defeated their first ordeal. First real difficulty checkpoint.
    FirstOrdealDefeated,
    /// Won a full run for the first time.
    FirstRunCompleted,
    /// Finished 10 runs (victory or defeat). Retention signal.
    TenRunsPlayed,
    /// Unlocked Summer — the first season above Spring on a material.
    Season2Unlocked,
    /// Unlocked Autumn on a material.
    Season3Unlocked,
    /// Unlocked Winter on a material.
    Season4Unlocked,
    /// Played into every non-final ordeal at least once (hallway Play, not Skip).
    AllBossesSeen,
    /// Silk Thread metamorphosed into Silk Moth — the player babied a
    /// fragile relic all the way to its terminal state instead of selling
    /// it. Marks engagement with the fragile / scaling cluster.
    SilkMothEmerged,
    /// Melting Ice thawed into Taotie — same fragile-cluster milestone
    /// as `SilkMothEmerged`, but for the chips half of the pair.
    TaotieAwakened,
    /// XXXL Egg burned out into Geese — fragile retrigger cluster.
    GeeseTakeFlight,
    /// Scored Kokushi Musō (thirteen orphans) — the alternate full-hand pattern.
    ThirteenOrphans,
    /// Beat **The House** on the final wing.
    HouseDefeated,
}

impl Achievement {
    /// Steamworks API Name. Must match the partner backend exactly.
    pub fn api_name(self) -> &'static str {
        match self {
            Self::TutorialComplete => "TUTORIAL_COMPLETE",
            Self::FirstStructure => "FIRST_STRUCTURE",
            Self::FirstBlindCleared => "FIRST_BLIND_CLEARED",
            Self::FirstOrdealDefeated => "FIRST_BOSS_DEFEATED",
            Self::FirstRunCompleted => "FIRST_RUN_COMPLETED",
            Self::TenRunsPlayed => "TEN_RUNS_PLAYED",
            Self::Season2Unlocked => "STAKE_2_UNLOCKED",
            Self::Season3Unlocked => "STAKE_3_UNLOCKED",
            Self::Season4Unlocked => "STAKE_4_UNLOCKED",
            Self::AllBossesSeen => "ALL_BOSSES_SEEN",
            Self::SilkMothEmerged => "SILK_MOTH_EMERGED",
            Self::TaotieAwakened => "TAOTIE_AWAKENED",
            Self::GeeseTakeFlight => "GEESE_TAKE_FLIGHT",
            Self::ThirteenOrphans => "THIRTEEN_ORPHANS",
            Self::HouseDefeated => "HOUSE_DEFEATED",
        }
    }

    /// Steam achievement fired when a run victory unlocks the next season tier.
    pub fn for_newly_unlocked_season(season: crate::core::season::Season) -> Option<Self> {
        match season {
            crate::core::season::Season::Spring => None,
            crate::core::season::Season::Summer => Some(Self::Season2Unlocked),
            crate::core::season::Season::Autumn => Some(Self::Season3Unlocked),
            crate::core::season::Season::Winter => Some(Self::Season4Unlocked),
        }
    }
}
