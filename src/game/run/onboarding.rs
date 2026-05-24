use crate::core::deck::Wall;
use crate::core::rules::BlindKind;
use crate::game::engine_state::GameplayCoreState;
use crate::game::game_mode::GameMode;
use crate::game::onboarding::{
    LESSONS_DISCARDS, LESSONS_HAND_SIZE, LESSONS_PLAYS, LESSONS_TARGET, OnboardingPhase,
    OnboardingState, TUTORIAL_BOSS, tutorial_yaku,
};
use crate::game::run::RunState;

impl RunState {
    /// Start the curated onboarding run used by the first-time tutorial
    /// campaign. Slides → guided Lessons blind → shop → boss finale.
    pub fn new_onboarding() -> Self {
        let mut mode = GameMode::with_material(crate::persistence::TileMaterial::Bamboo);
        mode.starting_gold = 16;
        mode.starting_plays = LESSONS_PLAYS;
        mode.starting_discards = LESSONS_DISCARDS;
        mode.hand_size = LESSONS_HAND_SIZE;
        mode.base_target = LESSONS_TARGET;
        mode.starting_yaku = Vec::new();
        mode.consumable_capacity = 2;

        let mut state = Self::new(mode.clone());
        state.mode = mode;
        state.available_yaku = Vec::new();
        state.available_rules = state.mode.starting_rules.clone();
        state.base_target = LESSONS_TARGET;
        state.target_score = LESSONS_TARGET;
        state.gold = state.mode.starting_gold as i32;
        state.ante = 1;
        state.run_number = 1;
        state.blind = BlindKind::Small;
        state.upcoming_blind = BlindKind::Small;
        state.onboarding = Some(OnboardingState::new());
        state.boss.upcoming = Some(TUTORIAL_BOSS);
        state.resolve_upcoming_boss();
        state.small_blind_tag = None;
        state.big_blind_tag = None;
        state.tag_free_reroll = false;
        state.tag_patron_gift = false;
        state.tag_rich_stock = false;
        state.tag_bonus_plays = 0;
        state.tag_bonus_discards = 0;
        state.tag_bonus_hand_size = 0;
        state
    }

    pub fn onboarding_active(&self) -> bool {
        self.onboarding.is_some()
    }

    pub fn onboarding_phase(&self) -> Option<OnboardingPhase> {
        self.onboarding.as_ref().map(|o| o.phase)
    }

    pub fn onboarding_lessons_active(&self) -> bool {
        self.onboarding.as_ref().is_some_and(|o| o.lessons_active())
    }

    pub fn onboarding_discard_allowed(&self) -> bool {
        match &self.onboarding {
            Some(o) if o.lessons_active() => o.discard_allowed_in_lessons(),
            _ => true,
        }
    }

    /// Configure the run for the guided Lessons blind after the campaign slides.
    pub fn begin_onboarding_lessons(&mut self) {
        if let Some(ref mut onboarding) = self.onboarding {
            onboarding.phase = OnboardingPhase::Lessons;
            onboarding.step = 0;
            onboarding.discard_river_tooltip_shown = false;
            onboarding.scored_once = false;
        }
        self.mode.hand_size = LESSONS_HAND_SIZE;
        self.mode.starting_plays = LESSONS_PLAYS;
        self.mode.starting_discards = LESSONS_DISCARDS;
        self.mode.base_target = LESSONS_TARGET;
        self.available_yaku = Vec::new();
        self.base_target = LESSONS_TARGET;
        self.target_score = LESSONS_TARGET;
        self.blind = BlindKind::Small;
        self.upcoming_blind = BlindKind::Small;
        self.boss.upcoming = None;
    }

    pub fn begin_onboarding_finale(&mut self) {
        if let Some(ref mut onboarding) = self.onboarding {
            onboarding.phase = OnboardingPhase::Finale;
            onboarding.finale_intro_shown = false;
        }
        self.available_yaku = tutorial_yaku();
        self.boss.upcoming = Some(TUTORIAL_BOSS);
        self.resolve_upcoming_boss();
        self.upcoming_blind = BlindKind::Boss;
    }

    /// After losing the onboarding boss blind, reset the round without advancing.
    pub fn retry_onboarding_finale(&mut self) {
        self.round_score = 0;
        self.last_breakdown = None;
        self.scored_last_turn = false;
        self.joker_used = false;
        self.full_hand_played_this_round = false;
        self.played_yaku_this_round.clear();
        self.honors_scored_this_round = false;
    }

    /// Retry the Lessons blind after missing the target.
    pub fn retry_onboarding_lessons(&mut self) {
        if let Some(ref mut onboarding) = self.onboarding {
            onboarding.step = 0;
            onboarding.discard_river_tooltip_shown = false;
            onboarding.scored_once = false;
        }
        self.round_score = 0;
        self.round_rules.clear();
        self.reset_round_resources();
        self.last_breakdown = None;
        self.scored_last_turn = false;
        self.joker_used = false;
        self.full_hand_played_this_round = false;
        self.played_yaku_this_round.clear();
        self.honors_scored_this_round = false;

        let overflow = self
            .relics
            .has(crate::core::relic::RelicId::StrengthInNumbers);
        self.wall = Wall::from_filtered_with_packs(
            &self.removed_tile_ids,
            &self.tile_packs,
            &self.tile_enhancements,
            overflow,
        );
        self.hand.clear();
        let draw_count = crate::core::boss::effective_hand_size(self);
        for _ in 0..draw_count {
            if let Some(t) = self.wall.draw() {
                self.hand.push(t);
            }
        }
        GameplayCoreState::with_run_mut(self, |core| {
            core.finalize_hand_after_draw();
        });
        self.restamp_hand_enhancements();
        self.seed_onboarding_hand();
    }

    pub fn onboarding_notify_structure_committed(&mut self) {
        if let Some(ref mut onboarding) = self.onboarding
            && onboarding.lessons_active()
            && onboarding.step <= 1
        {
            onboarding.step = 2;
        }
    }

    pub fn onboarding_notify_cash_in(&mut self) {
        if let Some(ref mut onboarding) = self.onboarding
            && onboarding.lessons_active()
        {
            onboarding.scored_once = true;
            if onboarding.step <= 2 {
                onboarding.step = 3;
            }
        }
    }

    pub fn onboarding_notify_discard(&mut self) {
        if let Some(ref mut onboarding) = self.onboarding
            && onboarding.lessons_active()
        {
            onboarding.discard_river_tooltip_shown = true;
            if onboarding.step == 3 {
                onboarding.step = 4;
            }
        }
    }

    /// Seed a starter hand for the Lessons blind.
    pub fn seed_onboarding_hand(&mut self) {
        if !self.onboarding_lessons_active() {
            return;
        }

        use rand::seq::SliceRandom;

        let mut rng = rand::rng();
        self.hand.shuffle(&mut rng);

        if self.hand.len() >= 4 {
            let face0 = (self.hand[0].suit, self.hand[0].rank);
            self.hand[1].suit = face0.0;
            self.hand[1].rank = face0.1;
            let face1 = (self.hand[2].suit, self.hand[2].rank);
            self.hand[3].suit = face1.0;
            self.hand[3].rank = face1.1;
            self.hand.sort();
        }
    }
}
