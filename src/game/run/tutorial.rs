use super::*;

impl RunState {
    /// Advance the tutorial to the next lesson. Adjusts hand size, target,
    /// discards, and available yaku per the lesson definition. Returns the
    /// new lesson number, or `None` if the tutorial finished.
    pub fn advance_tutorial_lesson(&mut self) -> Option<u32> {
        let tutorial = self.tutorial.as_mut()?;
        let next = tutorial.advance()?;
        let lesson = tutorial.current_lesson_def();

        // Apply lesson overrides to the mode.
        self.mode.apply_lesson(lesson);

        // Update run state from mode.
        self.available_yaku = self.mode.starting_yaku.clone();
        self.target_score = self.mode.base_target;
        self.base_target = self.mode.base_target;
        self.plays_remaining = self.round_play_cap();
        self.discards_remaining = self.round_discard_cap();
        self.sync_round_resource_caps();

        // Adjust hand size: grow by drawing more tiles if needed.
        let target_hand_size = self.mode.hand_size;
        while self.hand.len() < target_hand_size {
            if let Some(t) = self.wall.draw() {
                self.hand.push(t);
            } else {
                break;
            }
        }
        self.hand.sort();
        self.selected.resize(self.hand.len(), false);

        // Seed guaranteed melds for early lessons.
        self.seed_tutorial_hand();

        Some(next)
    }

    /// Retry the current tutorial blind after a failure. Records the failure
    /// for adaptive difficulty, re-deals the hand, and resets plays/discards
    /// without advancing the lesson or ante. The lowered target is applied
    /// via `retry_target_factor` in `apply_blind`.
    pub fn retry_tutorial_blind(&mut self) {
        if let Some(ref mut tut) = self.tutorial {
            tut.record_failure();
        }

        // Reset round state (same blind, same lesson).
        self.round_score = 0;
        self.round_rules.clear();
        self.reset_round_resources();
        self.last_breakdown = None;
        self.scored_last_turn = false;
        self.quickdraw_uses_remaining = crate::game::run::QUICKDRAW_USES_PER_ROUND;
        self.joker_used = false;
        self.full_hand_played_this_round = false;
        self.played_yaku_this_round.clear();
        self.honors_scored_this_round = false;

        // Re-deal the wall and hand.
        let overflow = self
            .relics
            .has(crate::core::relic::RelicId::StrengthInNumbers);
        self.wall = Wall::from_filtered_with_packs(
            &self.removed_tile_ids,
            &self.tile_packs,
            &self.tile_enhancements,
            overflow,
        );
        if self.relics.has(crate::core::relic::RelicId::DoraCrown) {
            self.wall.reveal_extra_dora_indicator();
        }
        self.hand.clear();
        let draw_count = self.mode.hand_size;
        for _ in 0..draw_count {
            if let Some(t) = self.wall.draw() {
                self.hand.push(t);
            }
        }
        self.hand.sort();
        self.selected = vec![false; self.hand.len()];
        self.restamp_hand_enhancements();
        self.seed_tutorial_hand();

        // Round re-apply (lowered target via retry_target_factor, fresh
        // hand, on-round-start triggers) is deferred to the gameplay scene
        // via `GameplayScene::with_pending_blind(self.blind)`.
    }

    /// Check if a set of detected meld kinds is valid for the current
    /// tutorial lesson. Returns `Ok(())` or an error message.
    pub fn tutorial_validate_sets(&self, set_kinds: &[SetKind]) -> Result<(), &'static str> {
        if let Some(ref tutorial) = self.tutorial
            && tutorial.is_active()
        {
            let lesson = tutorial.current_lesson_def();
            return crate::game::tutorial::validate_sets_for_lesson(set_kinds, lesson);
        }
        Ok(())
    }

    /// Check if discarding is allowed in the current tutorial lesson.
    pub fn tutorial_discard_allowed(&self) -> bool {
        match &self.tutorial {
            Some(tutorial) if tutorial.is_active() => tutorial.current_lesson_def().discard_enabled,
            _ => true,
        }
    }

    /// Check if the shop should be shown in the current tutorial lesson.
    pub fn tutorial_shop_enabled(&self) -> bool {
        match &self.tutorial {
            Some(tutorial) if tutorial.is_active() => tutorial.current_lesson_def().shop_enabled,
            _ => true,
        }
    }

    /// Whether tile affinity glow should be active.
    pub fn tutorial_affinity_glow(&self) -> bool {
        match &self.tutorial {
            Some(tutorial) if tutorial.is_active() => tutorial.current_lesson_def().affinity_glow,
            _ => false,
        }
    }

    /// Whether the scoring cascade should run in annotated slow-mo.
    pub fn tutorial_annotated_cascade(&self) -> bool {
        match &self.tutorial {
            Some(tutorial) if tutorial.is_active() && !tutorial.cascade_annotated => {
                tutorial.current_lesson_def().annotated_cascade
            }
            _ => false,
        }
    }

    /// Seed guaranteed melds into the current hand for tutorial lessons.
    /// Shuffles the hand first so retries produce different tile layouts,
    /// then overwrites a few positions to ensure the player can form the
    /// melds that the current lesson teaches. Only affects lessons 1-5.
    pub(super) fn seed_tutorial_hand(&mut self) {
        use rand::RngExt;
        use rand::seq::SliceRandom;

        let lesson_id = match &self.tutorial {
            Some(t) if t.is_active() => t.current_lesson,
            _ => return,
        };

        // Only certain lessons need seeded guaranteed melds; later lessons
        // play with whatever the wall gives.
        if lesson_id > 7 || lesson_id == 6 {
            return;
        }

        // Shuffle before seeding so the base tiles vary between attempts.
        let mut rng = rand::rng();
        self.hand.shuffle(&mut rng);

        match lesson_id {
            1 => {
                // Ensure at least 2 pairs. Pick two tiles and duplicate them.
                if self.hand.len() >= 4 {
                    let face0 = (self.hand[0].suit, self.hand[0].rank);
                    // Make hand[1] match hand[0].
                    self.hand[1].suit = face0.0;
                    self.hand[1].rank = face0.1;
                    // Make another pair from hand[2].
                    let face1 = (self.hand[2].suit, self.hand[2].rank);
                    self.hand[3].suit = face1.0;
                    self.hand[3].rank = face1.1;
                    self.hand.sort();
                }
            }
            2 => {
                // Ensure at least 1 triplet.
                if self.hand.len() >= 3 {
                    let face = (self.hand[0].suit, self.hand[0].rank);
                    self.hand[1].suit = face.0;
                    self.hand[1].rank = face.1;
                    self.hand[2].suit = face.0;
                    self.hand[2].rank = face.1;
                    self.hand.sort();
                }
            }
            3 => {
                // Ensure at least 1 sequence. Pick a numbered suit tile and
                // make the next two consecutive.
                let len = self.hand.len();
                if len >= 3 {
                    let base_face = self
                        .hand
                        .iter()
                        .find(|t| t.is_number_tile() && t.rank <= 7)
                        .map(|t| (t.suit, t.rank));
                    if let Some((suit, rank)) = base_face {
                        let i = len - 2;
                        let j = len - 1;
                        self.hand[i].suit = suit;
                        self.hand[i].rank = rank + 1;
                        self.hand[j].suit = suit;
                        self.hand[j].rank = rank + 2;
                    }
                    self.hand.sort();
                }
            }
            4 | 5 => {
                // Ensure the 14-tile hand has a sequence + triplet + pair
                // (close to FullHand territory for lesson 5).
                let len = self.hand.len();
                if len >= 9 {
                    // Triplet from first tile.
                    let face = (self.hand[0].suit, self.hand[0].rank);
                    self.hand[1].suit = face.0;
                    self.hand[1].rank = face.1;
                    self.hand[2].suit = face.0;
                    self.hand[2].rank = face.1;
                    // Sequence from a mid-range numbered suit.
                    let seq_face = self.hand[4..]
                        .iter()
                        .find(|t| t.is_number_tile() && t.rank <= 7)
                        .map(|t| (t.suit, t.rank));
                    if let Some((suit, rank)) = seq_face {
                        self.hand[5].suit = suit;
                        self.hand[5].rank = rank + 1;
                        self.hand[6].suit = suit;
                        self.hand[6].rank = rank + 2;
                    }
                    // Pair from hand[7].
                    let pf = (self.hand[7].suit, self.hand[7].rank);
                    self.hand[8].suit = pf.0;
                    self.hand[8].rank = pf.1;
                    self.hand.sort();
                }
            }
            7 => {
                // Guarantee an honor triplet so the player can trigger Yakuhai.
                use crate::core::tile::Suit;

                let len = self.hand.len();
                if len >= 3 {
                    let honor_suits = [Suit::Wind, Suit::Dragon];
                    let suit = honor_suits[rng.random_range(0..honor_suits.len())];
                    let rank: u8 = if suit == Suit::Wind {
                        rng.random_range(1..=4) // East, South, West, North
                    } else {
                        rng.random_range(1..=3) // Red, Green, White
                    };
                    self.hand[0].suit = suit;
                    self.hand[0].rank = rank;
                    self.hand[1].suit = suit;
                    self.hand[1].rank = rank;
                    self.hand[2].suit = suit;
                    self.hand[2].rank = rank;
                    self.hand.sort();
                }
            }
            _ => {}
        }
    }
}
