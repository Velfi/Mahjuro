//! Achievement IDs — single source of truth for Steam, Game Center, and Xbox Live.

/// All Mahjuro achievements. Designed as a funnel — each one marks a
/// checkpoint where players commonly drop off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Achievement {
    TutorialComplete,
    FirstStructure,
    FirstBlindCleared,
    FirstOrdealDefeated,
    FirstRunCompleted,
    TenRunsPlayed,
    Season2Unlocked,
    Season3Unlocked,
    Season4Unlocked,
    AllBossesSeen,
    SilkMothEmerged,
    TaotieAwakened,
    GeeseTakeFlight,
    ThirteenOrphans,
    HouseDefeated,
}

impl Achievement {
    pub const ALL: [Self; 15] = [
        Self::TutorialComplete,
        Self::FirstStructure,
        Self::FirstBlindCleared,
        Self::FirstOrdealDefeated,
        Self::FirstRunCompleted,
        Self::TenRunsPlayed,
        Self::Season2Unlocked,
        Self::Season3Unlocked,
        Self::Season4Unlocked,
        Self::AllBossesSeen,
        Self::SilkMothEmerged,
        Self::TaotieAwakened,
        Self::GeeseTakeFlight,
        Self::ThirteenOrphans,
        Self::HouseDefeated,
    ];

    /// Steamworks API Name (app 4636490).
    pub fn steam_api_name(self) -> &'static str {
        self.partner_id()
    }

    /// Game Center achievement identifier (App Store Connect).
    pub fn game_center_id(self) -> &'static str {
        self.partner_id()
    }

    /// Xbox Live achievement ID (Partner Center).
    pub fn xbox_achievement_id(self) -> &'static str {
        self.partner_id()
    }

    /// Shared partner-portal identifier across backends (configure matching
    /// records in each store dashboard).
    fn partner_id(self) -> &'static str {
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

    /// Achievement fired when a run victory unlocks the next season tier.
    pub fn for_newly_unlocked_season(season: mahjuro_core::core::season::Season) -> Option<Self> {
        match season {
            mahjuro_core::core::season::Season::Spring => None,
            mahjuro_core::core::season::Season::Summer => Some(Self::Season2Unlocked),
            mahjuro_core::core::season::Season::Autumn => Some(Self::Season3Unlocked),
            mahjuro_core::core::season::Season::Winter => Some(Self::Season4Unlocked),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_achievements_have_three_backend_ids() {
        for ach in Achievement::ALL {
            assert!(!ach.steam_api_name().is_empty());
            assert!(!ach.game_center_id().is_empty());
            assert!(!ach.xbox_achievement_id().is_empty());
            assert_eq!(ach.steam_api_name(), ach.game_center_id());
            assert_eq!(ach.steam_api_name(), ach.xbox_achievement_id());
        }
    }
}
