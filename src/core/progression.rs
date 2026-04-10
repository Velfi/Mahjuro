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
    /// Whether the player has completed (or skipped) the tutorial.
    #[serde(default)]
    pub tutorial_completed: bool,
    /// Whether the player has ever won a run (defeated the final boss).
    /// Unlocks the Plastic tile material.
    #[serde(default)]
    pub has_won: bool,
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
            tutorial_completed: false,
            has_won: false,
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

    /// Check if a new level was reached and apply unlocks.
    /// Returns details of what was unlocked, or `None` if nothing new.
    pub fn check_level_up(&mut self) -> Option<LevelUpResult> {
        let level = self.current_level();
        let unlocks = unlocks_for_level(level);

        let mut changed = false;
        let mut new_relics = Vec::new();
        for relic in unlocks.relics {
            if self.unlocked_relics.insert(relic) {
                new_relics.push(relic);
                changed = true;
            }
        }
        let mut new_rules = Vec::new();
        for rule in unlocks.rules {
            if self.unlocked_rules.insert(rule) {
                new_rules.push(rule);
                changed = true;
            }
        }

        // Yaku and dora are level-gated (not tracked in a HashSet), so they
        // are always "new" for their unlock level.
        if !unlocks.yaku.is_empty() || unlocks.dora {
            changed = true;
        }

        if changed {
            Some(LevelUpResult {
                new_level: level,
                relics: new_relics,
                rules: new_rules,
                yaku: unlocks.yaku,
                dora: unlocks.dora,
            })
        } else {
            None
        }
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
        for l in 1..=level {
            available.extend(unlocks_for_level(l).yaku);
        }
        available
    }

    /// Whether the Plastic tile material is unlocked (requires first victory).
    pub fn plastic_unlocked(&self) -> bool {
        self.has_won
    }

    /// Whether dora tiles are enabled at this level.
    pub fn dora_enabled(&self) -> bool {
        let level = self.current_level();
        (1..=level).any(|l| unlocks_for_level(l).dora)
    }
}

struct LevelUnlocks {
    relics: Vec<RelicId>,
    rules: Vec<RuleModifier>,
    yaku: Vec<YakuKind>,
    dora: bool,
}

/// What was unlocked when the player leveled up.
pub struct LevelUpResult {
    pub new_level: u32,
    pub relics: Vec<RelicId>,
    pub rules: Vec<RuleModifier>,
    pub yaku: Vec<YakuKind>,
    pub dora: bool,
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
                RelicId::JadeSerpent,
                RelicId::InkBrush,
            ],
            rules: vec![RuleModifier::PairDoubleScore],
            yaku: vec![],
            dora: false,
        },
        2 => LevelUnlocks {
            relics: vec![
                RelicId::MultiplierMaster,
                RelicId::GreenLuck,
                RelicId::QuickDraw,
                RelicId::ShantenShove,
                RelicId::PearlDiver,
                RelicId::LowTide,
            ],
            rules: vec![],
            yaku: vec![YakuKind::Toitoi, YakuKind::Tanyao],
            dora: false,
        },
        3 => LevelUnlocks {
            relics: vec![
                RelicId::ChainReaction,
                RelicId::SetMagnet,
                RelicId::RoundCompass,
                RelicId::YakuScholar,
                RelicId::MerchantsEye,
                RelicId::Momentum,
            ],
            rules: vec![RuleModifier::SequenceWrap],
            yaku: vec![YakuKind::Iipeikou, YakuKind::Honitsu],
            dora: false,
        },
        4 => LevelUnlocks {
            relics: vec![
                RelicId::HonorFury,
                RelicId::WhiteSilence,
                RelicId::Overflow,
                // CodexCompass disabled — see core::relic::all_relic_defs.
                RelicId::EdgeRunner,
                RelicId::TurtleShell,
            ],
            rules: vec![RuleModifier::NoSequenceBonus],
            yaku: vec![YakuKind::Chinitsu, YakuKind::Chiitoitsu],
            dora: true,
        },
        5 => LevelUnlocks {
            relics: vec![
                RelicId::JokerTile,
                RelicId::WildWinds,
                RelicId::KanDrum,
                RelicId::DoraCrown,
                RelicId::TenpaiTalisman,
                RelicId::LuckySeven,
                RelicId::Minimalist,
            ],
            rules: vec![RuleModifier::HonorTripleScore],
            yaku: vec![YakuKind::SanshokuDoujun, YakuKind::Honroutou],
            dora: false,
        },
        6 => LevelUnlocks {
            relics: vec![
                // RiichiStick / RiverEraser / FuritenWard disabled — see
                // core::relic::all_relic_defs and PATCH_D / PATCH_E docs.
                RelicId::LunarAlmanac,
                RelicId::SecondWind,
                RelicId::GoldFurnace,
                RelicId::ClosedGate,
            ],
            rules: vec![RuleModifier::NoSequences, RuleModifier::ReducedPlays],
            yaku: vec![YakuKind::Junchan, YakuKind::Ittsu],
            dora: false,
        },
        7 => LevelUnlocks {
            relics: vec![
                RelicId::RedDragonRage,
                RelicId::DragonEcho,
                RelicId::EightTreasures,
                RelicId::KongsBlessing,
                RelicId::GlassCannon,
                RelicId::Snowball,
            ],
            rules: vec![],
            yaku: vec![],
            dora: false,
        },
        _ => LevelUnlocks {
            relics: vec![],
            rules: vec![],
            yaku: vec![],
            dora: false,
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
        let result = p.check_level_up();
        assert!(result.is_some());
        let result = result.unwrap();
        assert_eq!(result.new_level, 2);
        assert!(result.relics.contains(&RelicId::MultiplierMaster));
        assert!(result.yaku.contains(&YakuKind::Toitoi));
        assert!(result.yaku.contains(&YakuKind::Tanyao));
        assert!(!result.dora);
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
