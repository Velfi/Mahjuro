//! Meta progression and unlocks.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::core::relic::RelicId;
use crate::core::rules::RuleModifier;
use crate::core::yaku::YakuKind;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlayerProgress {
    pub unlocked_relics: HashSet<RelicId>,
    pub unlocked_rules: HashSet<RuleModifier>,
    pub high_scores: Vec<u32>,
    /// Total runs completed (drives level progression).
    #[serde(default)]
    pub runs_completed: u32,
    /// Extra plays per round from permanent upgrades.
    #[serde(default)]
    pub bonus_plays: u32,
    /// Extra starting relics at run start.
    #[serde(default)]
    pub starting_relic_slots: u32,
}

impl PlayerProgress {
    pub fn new() -> Self {
        Self {
            unlocked_relics: HashSet::new(),
            unlocked_rules: HashSet::new(),
            high_scores: Vec::new(),
            runs_completed: 0,
            bonus_plays: 0,
            starting_relic_slots: 0,
        }
    }

    pub fn record_score(&mut self, score: u32) {
        self.high_scores.push(score);
        self.high_scores.sort_by(|a, b| b.cmp(a));
        self.high_scores.truncate(10);
    }

    /// Current progression level (1–7) based on runs completed.
    pub fn current_level(&self) -> u32 {
        match self.runs_completed {
            0 => 1,
            1..=2 => 2,
            3..=5 => 3,
            6..=9 => 4,
            10..=14 => 5,
            15..=19 => 6,
            _ => 7,
        }
    }

    /// Check if a new level was reached and apply unlocks. Returns the new level if changed.
    pub fn check_level_up(&mut self) -> Option<u32> {
        let level = self.current_level();
        let unlocks = unlocks_for_level(level);

        let mut changed = false;
        for relic in unlocks.relics {
            if self.unlocked_relics.insert(relic) {
                changed = true;
            }
        }
        for rule in unlocks.rules {
            if self.unlocked_rules.insert(rule) {
                changed = true;
            }
        }

        if changed { Some(level) } else { None }
    }

    /// Relics available for this player's progression level.
    pub fn available_relics(&self) -> Vec<RelicId> {
        let level = self.current_level();
        let mut available = Vec::new();
        for l in 1..=level {
            available.extend(unlocks_for_level(l).relics);
        }
        available
    }

    /// Rules available for this player's progression level.
    pub fn available_rules(&self) -> Vec<RuleModifier> {
        let level = self.current_level();
        let mut available = Vec::new();
        for l in 1..=level {
            available.extend(unlocks_for_level(l).rules);
        }
        available
    }

    /// Yaku patterns available for this player's progression level.
    /// FullHand and Yakuhai are always available; remaining yaku gate on
    /// progression level. (The Patch B Codex loadout system in run state
    /// further restricts which of these contribute at full value during a
    /// given run — see `RunState::yaku_loadout`.)
    pub fn available_yaku(&self) -> Vec<YakuKind> {
        let level = self.current_level();
        let mut available = vec![YakuKind::FullHand, YakuKind::Yakuhai];
        if level >= 2 {
            available.push(YakuKind::Toitoi);
            available.push(YakuKind::Tanyao);
        }
        if level >= 3 {
            available.push(YakuKind::Iipeikou);
            available.push(YakuKind::Honitsu);
        }
        if level >= 4 {
            available.push(YakuKind::Chinitsu);
            available.push(YakuKind::Chiitoitsu);
        }
        if level >= 5 {
            available.push(YakuKind::SanshokuDoujun);
            available.push(YakuKind::Honroutou);
        }
        if level >= 6 {
            available.push(YakuKind::Junchan);
            available.push(YakuKind::Ittsu);
        }
        available
    }

    /// Whether dora tiles are enabled at this level.
    pub fn dora_enabled(&self) -> bool {
        self.current_level() >= 4
    }
}

struct LevelUnlocks {
    relics: Vec<RelicId>,
    rules: Vec<RuleModifier>,
}

fn unlocks_for_level(level: u32) -> LevelUnlocks {
    match level {
        1 => LevelUnlocks {
            relics: vec![
                RelicId::TripletBoost,
                RelicId::SequenceSurge,
                RelicId::PairPower,
                RelicId::WallPeek,
                RelicId::ZodiacPouch,
            ],
            rules: vec![RuleModifier::PairDoubleScore],
        },
        2 => LevelUnlocks {
            relics: vec![
                RelicId::MultiplierMaster,
                RelicId::GreenLuck,
                RelicId::QuickDraw,
                RelicId::ShantenLens,
            ],
            rules: vec![],
        },
        3 => LevelUnlocks {
            relics: vec![
                RelicId::ChainReaction,
                RelicId::SetMagnet,
                RelicId::RoundCompass,
                RelicId::YakuScholar,
            ],
            rules: vec![RuleModifier::SequenceWrap],
        },
        4 => LevelUnlocks {
            relics: vec![
                RelicId::HonorFury,
                RelicId::WhiteSilence,
                RelicId::Overflow,
                RelicId::CodexCompass,
            ],
            rules: vec![RuleModifier::NoSequenceBonus],
        },
        5 => LevelUnlocks {
            relics: vec![
                RelicId::JokerTile,
                RelicId::WildWinds,
                RelicId::KanDrum,
                RelicId::DoraCrown,
                RelicId::TenpaiTalisman,
            ],
            rules: vec![RuleModifier::HonorTripleScore],
        },
        6 => LevelUnlocks {
            relics: vec![
                RelicId::RiichiStick,
                RelicId::RiverEraser,
                RelicId::FuritenWard,
                RelicId::LunarAlmanac,
            ],
            rules: vec![RuleModifier::NoSequences, RuleModifier::ReducedPlays],
        },
        7 => LevelUnlocks {
            relics: vec![
                RelicId::RedDragonRage,
                RelicId::DragonEcho,
                RelicId::EightTreasures,
                RelicId::KongsBlessing,
            ],
            rules: vec![],
        },
        _ => LevelUnlocks {
            relics: vec![],
            rules: vec![],
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_progression() {
        let mut p = PlayerProgress::new();
        assert_eq!(p.current_level(), 1);
        p.runs_completed = 1;
        assert_eq!(p.current_level(), 2);
        p.runs_completed = 3;
        assert_eq!(p.current_level(), 3);
        p.runs_completed = 6;
        assert_eq!(p.current_level(), 4);
        p.runs_completed = 20;
        assert_eq!(p.current_level(), 7);
    }

    #[test]
    fn level_up_unlocks_relics() {
        let mut p = PlayerProgress::new();
        p.runs_completed = 1;
        let level = p.check_level_up();
        assert!(level.is_some());
        assert!(p.unlocked_relics.contains(&RelicId::MultiplierMaster));
    }

    #[test]
    fn available_relics_grow_with_level() {
        let mut p = PlayerProgress::new();
        let l1 = p.available_relics().len();
        p.runs_completed = 6;
        let l4 = p.available_relics().len();
        assert!(l4 > l1);
    }

    #[test]
    fn dora_unlocks_at_level_4() {
        let mut p = PlayerProgress::new();
        assert!(!p.dora_enabled());
        p.runs_completed = 6;
        assert!(p.dora_enabled());
    }
}
