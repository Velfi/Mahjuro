//! Meta progression and unlocks.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::core::relic::RelicId;
use crate::core::rules::RuleModifier;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PlayerProgress {
    pub unlocked_relics: HashSet<RelicId>,
    pub unlocked_rules: HashSet<RuleModifier>,
    pub high_scores: Vec<u32>,
}

impl PlayerProgress {
    pub fn new() -> Self {
        Self {
            unlocked_relics: HashSet::new(),
            unlocked_rules: HashSet::new(),
            high_scores: Vec::new(),
        }
    }

    pub fn record_score(&mut self, score: u32) {
        self.high_scores.push(score);
        self.high_scores.sort_by(|a, b| b.cmp(a));
        self.high_scores.truncate(10);
    }
}
