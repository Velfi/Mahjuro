use super::*;

impl RunState {
    /// Start the curated onboarding run used by the first-time tutorial
    /// campaign. This is a single guided shop + boss flow, not the legacy
    /// lesson ladder.
    pub fn new_onboarding() -> Self {
        let mut mode = GameMode::with_material(crate::persistence::TileMaterial::Bamboo);
        mode.starting_gold = 16;
        mode.starting_plays = 5;
        mode.starting_discards = 4;
        mode.base_target = 220;
        mode.target_scaling = 1.0;
        mode.starting_yaku = tutorial_yaku();
        mode.consumable_capacity = 2;

        let mut state = Self::new(mode.clone());
        state.mode = mode;
        state.available_yaku = tutorial_yaku();
        state.available_rules = state.mode.starting_rules.clone();
        state.base_target = state.mode.base_target;
        state.target_score = state.mode.base_target;
        state.gold = state.mode.starting_gold as i32;
        state.ante = 1;
        state.run_number = 1;
        state.blind = BlindKind::Small;
        state.upcoming_blind = BlindKind::Small;
        state.tutorial = None;
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

    pub fn begin_onboarding_finale(&mut self) {
        if let Some(ref mut onboarding) = self.onboarding {
            onboarding.phase = OnboardingPhase::Finale;
        }
        self.available_yaku = tutorial_yaku();
        self.boss.upcoming = Some(TUTORIAL_BOSS);
        self.resolve_upcoming_boss();
        self.upcoming_blind = BlindKind::Boss;
        // Round setup (wall, hand, Sweepstakes, DoraCrown, boss on_apply) is
        // deferred to `GameplayScene::with_pending_blind(BlindKind::Boss)` so
        // on-round-start triggers play after the opening smoke curtain.
    }

    /// After losing the onboarding boss blind, reset the round and re-deal
    /// from a fresh wall (same target and boss rules). The fresh deal and
    /// on-round-start triggers are fired by the gameplay scene after the
    /// opening transition — the caller must route through
    /// `GameplayScene::with_pending_blind(self.blind)`.
    pub fn retry_onboarding_finale(&mut self) {
        self.round_score = 0;
        self.last_breakdown = None;
        self.scored_last_turn = false;
        self.quickdraw_uses_remaining = crate::game::run::QUICKDRAW_USES_PER_ROUND;
        self.joker_used = false;
        self.full_hand_played_this_round = false;
        self.played_yaku_this_round.clear();
        self.honors_scored_this_round = false;
    }
}
