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
    /// Finished the tutorial. Confirms onboarding actually completes.
    TutorialComplete,
    /// Scored their first structure. The earliest signal that the core
    /// scoring loop "clicked" for the player.
    FirstStructure,
    /// Beat round 1. Confirms one full round of the loop.
    FirstBlindCleared,
    /// Beat their first boss blind. First real difficulty checkpoint.
    FirstBossDefeated,
    /// Won a full run for the first time.
    FirstRunCompleted,
    /// Started 10 distinct runs. Retention signal.
    TenRunsPlayed,
    /// Unlocked the Summer stake (first step up from baseline).
    Stake2Unlocked,
    /// Encountered every boss blind at least once.
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
    /// Beat **The House** on the final boss blind.
    HouseDefeated,
}

impl Achievement {
    /// Steamworks API Name. Must match the partner backend exactly.
    pub fn api_name(self) -> &'static str {
        match self {
            Self::TutorialComplete => "TUTORIAL_COMPLETE",
            Self::FirstStructure => "FIRST_STRUCTURE",
            Self::FirstBlindCleared => "FIRST_BLIND_CLEARED",
            Self::FirstBossDefeated => "FIRST_BOSS_DEFEATED",
            Self::FirstRunCompleted => "FIRST_RUN_COMPLETED",
            Self::TenRunsPlayed => "TEN_RUNS_PLAYED",
            Self::Stake2Unlocked => "STAKE_2_UNLOCKED",
            Self::AllBossesSeen => "ALL_BOSSES_SEEN",
            Self::SilkMothEmerged => "SILK_MOTH_EMERGED",
            Self::TaotieAwakened => "TAOTIE_AWAKENED",
            Self::GeeseTakeFlight => "GEESE_TAKE_FLIGHT",
            Self::ThirteenOrphans => "THIRTEEN_ORPHANS",
            Self::HouseDefeated => "HOUSE_DEFEATED",
        }
    }
}
