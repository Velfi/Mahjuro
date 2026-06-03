use crate::game::run::RunState;
use crate::{
    core::{
        deck::Wall,
        ordeal,
        relic::RelicId,
        rules::{ChamberKind, RuleModifier},
        tile::Suit,
    },
    game::{engine_state::GameplayCoreState, event_bus::EventBus},
};

impl RunState {
    /// Clear win/loss terminal flags. Call when leaving an active round or
    /// between rounds so hallway / shop actions cannot re-fire resolution.
    pub(super) fn clear_round_resolution_state(&mut self) {
        self.round_score = 0;
        self.round_end_queued = false;
    }

    /// Zero round score and set the upcoming chamber target before the gameplay
    /// scene loads. `apply_chamber` still runs later for the deal and boss hooks;
    /// this only primes HUD values so score rollers do not spin off the last round.
    pub fn preset_round_hud_for_chamber_entry(&mut self, blind: ChamberKind) {
        self.clear_round_resolution_state();
        self.target_score = self.chamber_score_target(blind);
    }

    /// Apply a blind choice: sets target score, dispatches boss effect on
    /// boss blinds, and applies any per-round resource resets.
    pub fn apply_chamber(&mut self, blind: ChamberKind, bus: Option<&mut EventBus>) {
        if blind == ChamberKind::Ordeal {
            self.ensure_ordeal_revealed();
        }
        self.clear_round_resolution_state();
        self.chamber = blind;
        self.memorial_round.clear();
        self.reset_round_resources();
        self.tile_debuffs.clear();
        self.relics.clear_debuffs();
        // Per-ante exponential base × blind multiplier (Balatro-style); see
        // `core::chamber_target`. Skips do not inflate later targets.
        self.target_score = self.chamber_score_target(blind);
        // Boss dispatch — push rule modifiers and run the on_apply hook so
        // category-C taxers (zero discards, hand-size shrink, gold cost) take
        // effect before the player draws their first hand.
        if blind == ChamberKind::Ordeal {
            // Read from the resolved effect (built at reveal time) so reactive
            // bosses' chosen variants land correctly. Take/restore to dodge
            // the &mut self conflict when calling on_apply.
            if let Some(eff) = self.ordeal.effect.take() {
                for &m in &eff.rule_pushes {
                    if !self.round_rules.contains(&m) {
                        self.round_rules.push(m);
                    }
                }
                self.tile_debuffs = eff.tile_debuffs.clone();
                self.relics.set_debuffed(eff.relic_debuffs.iter().copied());
                if let Some(hook) = eff.on_apply {
                    hook(self);
                }
                self.ordeal.effect = Some(eff);
            }
        }
        self.feed_hungry_ghosts_at_round_start();
        self.refresh_windreader_bonus_wind();
        // Ant Trail: when held, sequences may wrap 9→1 (9-1-2, 8-9-1). The
        // validator already supports this via RuleModifier::SequenceWrap, so
        // we just inject it here.
        if self.relics.has(crate::core::relic::RelicId::AntTrail)
            && !self.round_rules.contains(&RuleModifier::SequenceWrap)
        {
            self.round_rules.push(RuleModifier::SequenceWrap);
        }
        // Fold next-round Wide Hand into the round's effective hand size so
        // every refill and hand-size check sees the same total for the round.
        if self.tag_bonus_hand_size != 0 {
            self.ordeal.bonus_hand_size += self.tag_bonus_hand_size;
            self.tag_bonus_hand_size = 0;
        }
        // ReducedPlays modifier reduces plays from 4 to 3.
        if self.round_rules.contains(&RuleModifier::ReducedPlays) {
            self.plays_remaining = self.plays_remaining.min(3);
        }
        self.apply_pending_round_resource_bonuses();
        self.sync_round_resource_caps();
        // Deal the wall and hand for this round.
        let overflow = self
            .relics
            .has(crate::core::relic::RelicId::StrengthInNumbers);
        self.wall = Wall::from_filtered_with_packs(
            &self.removed_tile_ids,
            &self.tile_packs,
            &self.tile_enhancements,
            overflow,
            &self.joker_extra_faces,
        );
        if self.relics.has(crate::core::relic::RelicId::DoraCrown) {
            self.wall.reveal_extra_dora_indicator();
        }
        self.hand.clear();
        let draw_count = ordeal::effective_hand_size(self);
        let lotus = self.relics.has(crate::core::relic::RelicId::LotusBloom);
        for _ in 0..draw_count {
            if let Some(t) = self.wall.draw() {
                if lotus && t.suit == Suit::Flower {
                    *self
                        .relic_counters
                        .entry(crate::core::relic::RelicId::LotusBloom)
                        .or_insert(0) += 1;
                }
                self.hand.push(t);
            }
        }
        GameplayCoreState::with_run_mut(self, |core| {
            core.finalize_opening_deal();
        });
        self.restamp_hand_enhancements();
        // SetMagnet: pull 4th copies of any triplets in the opening hand.
        // No bus available at deal time; create a throwaway sink.
        let mut _sink = EventBus::default();
        self.set_magnet_draw_fourths(&mut _sink);
        GameplayCoreState::with_run_mut(self, |core| {
            core.finalize_hand_after_draw();
        });
        self.joker_tile_add_starting_hand_copy();

        // Round-start yen (Charity, Sweepstakes): one `apply_yen_reward` so `bus` is not moved twice.
        let mut round_start_yen = 0i32;
        if self.relics.has(RelicId::Charity) && self.yen < 10 {
            round_start_yen += 5;
            self.push_relic_activation(RelicId::Charity);
        }
        // Sweepstakes: 25% +¥2, 25% +¥4, 50% nothing. Rolled each round start.
        // Fortune's Favor doubles the weight of each payout vs. nothing → ⅓ / ⅓ / ⅓.
        if self.relics.has(crate::core::relic::RelicId::Sweepstakes) {
            use rand::RngExt;

            let mut rng = rand::rng();
            let fortunes = self.relics.has(RelicId::FortunesFavor);
            let payout: i32 = if fortunes {
                match rng.random_range(0..6) {
                    0 | 1 => 2,
                    2 | 3 => 4,
                    _ => 0,
                }
            } else {
                match rng.random_range(0..4) {
                    0 => 2,
                    1 => 4,
                    _ => 0,
                }
            };
            if payout > 0 {
                round_start_yen += payout;
                self.relic_activations
                    .push(crate::core::relic::RelicId::Sweepstakes);
            }
        }
        if round_start_yen > 0 {
            self.apply_yen_reward(round_start_yen, bus);
        }
    }

    /// Add the chosen relic, scale up the base target, and reset for the next round.
    /// The actual target_score is set later by `apply_chamber`.
    ///
    /// Balatro-style ante progression: chip base grows each ante; `apply_chamber`
    /// applies Small/Big/Boss multipliers for that ante.
    pub fn advance_round(&mut self, bus: &mut EventBus) {
        self.roll_lantern_maybe_shatter(bus);
        // Nest Egg: increment rounds held (affects sell value).
        if self.relics.has(RelicId::NestEgg) {
            *self.relic_counters.entry(RelicId::NestEgg).or_insert(0) += 1;
            self.push_relic_activation(RelicId::NestEgg);
        }
        // Obsession: check if the player's most-used yaku was NOT scored
        // this round. If so, increment the counter.
        if self.relics.has(RelicId::Obsession) {
            let top_yaku = self
                .yaku_times_played
                .iter()
                .max_by_key(|(_, count)| **count)
                .map(|(&y, _)| y);
            if let Some(top) = top_yaku {
                if !self.played_yaku_this_round.contains(&top) {
                    *self.relic_counters.entry(RelicId::Obsession).or_insert(0) += 1;
                    self.push_relic_activation(RelicId::Obsession);
                } else {
                    // Reset on use — rewards variety, not just avoidance.
                    self.relic_counters.insert(RelicId::Obsession, 0);
                }
            }
        }

        // Defeating the Boss completes an ante (`ante` increments below).
        let was_boss = self.chamber == ChamberKind::Ordeal;
        if was_boss {
            self.score_after_wing
                .push((self.wing, self.total_score_earned));
            self.wing += 1;
        }
        // Heirloom: +1 mult per blind *played* (skips don't count — this
        // path only runs when a blind was cleared).
        if self.relics.has(RelicId::Heirloom) {
            *self.relic_counters.entry(RelicId::Heirloom).or_insert(0) += 1;
        }
        // Snowball: stacks once per cleared blind (skips don't count), capped.
        if self.relics.has(RelicId::Snowball) {
            let e = self.relic_counters.entry(RelicId::Snowball).or_insert(0);
            *e = (*e + 1).min(crate::core::relic::SNOWBALL_STACK_CAP);
        }
        // Kong Collector: per-round kong tally is consumed at round end; clear it now.
        self.relic_counters.remove(&RelicId::KongCollector);
        self.run_number += 1;
        // `target_score` is recomputed by `apply_chamber` when the next blind is picked.
        self.round_rules.clear();
        self.reset_round_resources();
        self.last_breakdown = None;
        self.scored_last_turn = false;
        self.full_hand_played_this_round = false;
        self.ordeal.bonus_hand_size = 0;
        self.ordeal.yen_cost_per_play = 0;
        self.played_yaku_this_round.clear();
        self.honors_scored_this_round = false;
        self.upcoming_chamber = self.upcoming_chamber.next();
        // Leave `chamber` on the blind we just cleared until `apply_chamber`
        // starts the next one — otherwise gameplay round-end still shows the
        // next ordeal's icon and target before the player enters that blind.
        GameplayCoreState::with_run_mut(self, |core| {
            core.clear_hand_structure_bank();
        });
        self.tag_bonus_hand_size = 0;

        if was_boss {
            // Roll fresh skip-reward tags for the new wing. Shop-oriented
            // rewards must survive into the post-boss shop, but any
            // one-blind combat bonuses should expire at the wing boundary.
            self.roll_ante_tags();
            self.clear_next_chamber_tag_modifiers();
            // Boss identity and reactive `on_reveal` hooks run in
            // `ensure_ordeal_revealed` when the player is about to enter the
            // Ordeal blind (pick_chamber or `apply_chamber`), not here.
            self.ordeal.upcoming = None;
            self.ordeal.effect = None;
            self.ordeal.tax_collector_cost = 0;
        }
    }

    /// Second Wind: leave the current blind with no blind clear gold and no boss
    /// / ante credit. Relic hooks that run at a normal round end (Paper Lantern,
    /// Nest Egg, Obsession, …) still apply; Heirloom does not (blind was not cleared).
    #[cfg(any(feature = "game", feature = "headless-screenshot", test))]
    pub(crate) fn forfeit_current_chamber_second_wind(&mut self, bus: &mut EventBus) {
        self.roll_lantern_maybe_shatter(bus);
        if self.relics.has(RelicId::NestEgg) {
            *self.relic_counters.entry(RelicId::NestEgg).or_insert(0) += 1;
            self.push_relic_activation(RelicId::NestEgg);
        }
        if self.relics.has(RelicId::Obsession) {
            let top_yaku = self
                .yaku_times_played
                .iter()
                .max_by_key(|(_, count)| **count)
                .map(|(&y, _)| y);
            if let Some(top) = top_yaku {
                if !self.played_yaku_this_round.contains(&top) {
                    *self.relic_counters.entry(RelicId::Obsession).or_insert(0) += 1;
                    self.push_relic_activation(RelicId::Obsession);
                } else {
                    self.relic_counters.insert(RelicId::Obsession, 0);
                }
            }
        }

        self.relic_counters.remove(&RelicId::KongCollector);
        self.run_number += 1;
        self.round_rules.clear();
        self.reset_round_resources();
        self.last_breakdown = None;
        self.scored_last_turn = false;
        self.full_hand_played_this_round = false;
        self.ordeal.bonus_hand_size = 0;
        self.ordeal.yen_cost_per_play = 0;
        self.played_yaku_this_round.clear();
        self.honors_scored_this_round = false;
        self.upcoming_chamber = self.upcoming_chamber.next();
        self.chamber = self.upcoming_chamber;
        GameplayCoreState::with_run_mut(self, |core| {
            core.clear_hand_structure_bank();
        });
        self.tag_bonus_hand_size = 0;
    }

    /// Skip the upcoming blind: advance to the next in the cycle without
    /// playing or visiting the shop. Resets per-round state. Skipping is
    /// not allowed for the Boss blind — callers should check first.
    pub fn skip_to_next_chamber(&mut self) {
        self.defeat_journal.chambers_skipped =
            self.defeat_journal.chambers_skipped.saturating_add(1);
        self.chronicle.record_chamber_skipped(
            self.wing,
            self.upcoming_chamber,
            "Skip reward".into(),
        );
        self.upcoming_chamber = self.upcoming_chamber.next();
        self.run_number += 1;
        self.clear_round_resolution_state();
        // `target_score` is recomputed by `apply_chamber` when the next blind is picked.
        self.round_rules.clear();
        self.reset_round_resources();
        self.last_breakdown = None;
        self.scored_last_turn = false;
        // Reset per-round boss-effect state. The ante's `upcoming_ordeal` is
        // unchanged — skipping a Small/Big still leaves the same boss waiting.
        self.ordeal.bonus_hand_size = 0;
        self.ordeal.yen_cost_per_play = 0;
        self.played_yaku_this_round.clear();
        self.honors_scored_this_round = false;
        self.memorial_round.clear();
        self.chamber = self.upcoming_chamber;
        GameplayCoreState::with_run_mut(self, |core| {
            core.clear_hand_structure_bank();
        });
    }
}
