use super::*;

impl RunState {
    /// Apply a blind choice: sets target score, dispatches boss effect on
    /// boss blinds, and applies any per-round resource resets.
    pub fn apply_blind(&mut self, blind: BlindKind) {
        self.blind = blind;
        self.round_score = 0;
        self.reset_round_resources();
        self.tile_debuffs.clear();
        self.relics.clear_debuffs();
        // Target scales linearly with the blind's precedence: the Nth blind
        // (skipped or played) targets `base_target * N`. Base is 200 by default,
        // so blind 5 = 1000, blind 16 = 3200.
        let mut target = self.base_target.saturating_mul(self.run_number);
        // Tutorial adaptive difficulty: lower the target after repeated failures.
        if let Some(ref tut) = self.tutorial
            && tut.is_active()
        {
            target = (target as f32 * tut.retry_target_factor()) as u32;
        }
        self.target_score = target;
        // Boss dispatch — push rule modifiers and run the on_apply hook so
        // category-C taxers (zero discards, hand-size shrink, gold cost) take
        // effect before the player draws their first hand.
        let simplified = self
            .tutorial
            .as_ref()
            .is_some_and(|t| t.is_active() && t.current_lesson_def().simplified_boss);
        if blind == BlindKind::Boss && !simplified {
            // Read from the resolved effect (built at reveal time) so reactive
            // bosses' chosen variants land correctly. Take/restore to dodge
            // the &mut self conflict when calling on_apply.
            if let Some(eff) = self.boss.effect.take() {
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
                self.boss.effect = Some(eff);
            }
        }
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
            self.boss.bonus_hand_size += self.tag_bonus_hand_size;
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
        );
        if self.relics.has(crate::core::relic::RelicId::DoraCrown) {
            self.wall.reveal_extra_dora_indicator();
        }
        self.hand.clear();
        let draw_count = boss::effective_hand_size(self);
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
        self.hand.sort();
        self.selected = vec![false; self.hand.len()];
        self.structure_sets.clear();
        self.structure_tiles.clear();
        self.restamp_hand_enhancements();
        // SetMagnet: pull 4th copies of any triplets in the opening hand.
        // No bus available at deal time; create a throwaway sink.
        let mut _sink = EventBus::default();
        self.set_magnet_draw_fourths(&mut _sink);
        self.hand.sort();

        // Sweepstakes: 25% +$2, 25% +$4, 50% nothing. Rolled each round start.
        if self.relics.has(crate::core::relic::RelicId::Sweepstakes) {
            use rand::RngExt;

            let mut rng = rand::rng();
            let roll: u32 = rng.random_range(0..4);
            let payout: i32 = match roll {
                0 => 2,
                1 => 4,
                _ => 0,
            };
            if payout > 0 {
                self.gold = self.gold.saturating_add(payout);
                self.relic_activations
                    .push(crate::core::relic::RelicId::Sweepstakes);
            }
        }
    }

    /// Add the chosen relic, scale up the base target, and reset for the next round.
    /// The actual target_score is set later by `apply_blind`.
    ///
    /// Balatro-style ante progression: `base_target` is the *ante's* base, and the
    /// Small/Big/Boss multipliers in `apply_blind` derive each blind's actual target.
    /// We only grow `base_target` when the player defeats the Boss and rolls into the
    /// next ante; within an ante, the base stays put.
    pub fn advance_round(&mut self, bus: &mut EventBus) {
        // Fortune's Favor halves destruction chances (doubles survival).
        let fortunes = self.relics.has(RelicId::FortunesFavor);
        // Paper Lantern: 1-in-5 chance to burn up at round end. When it
        // burns, the slot empties and Paper goes extinct for the rest of
        // the run — Silver Filigree Lantern then enters the shop pool.
        // Fortune's Favor: 1-in-10 instead.
        if self.relics.has(RelicId::PaperLantern) {
            use rand::RngExt;

            let mut rng = rand::rng();
            let denom = if fortunes { 10 } else { 5 };
            if rng.random_ratio(1, denom) {
                self.relics.active.retain(|&r| r != RelicId::PaperLantern);
                self.paper_lantern_extinct = true;
                self.note_relic_destroyed();
            }
        }
        // Silver Filigree Lantern: 1-in-1000 chance to shatter at round end.
        // Fortune's Favor: 1-in-2000.
        if self.relics.has(RelicId::SilverFiligreeLantern) {
            use rand::RngExt;

            let mut rng = rand::rng();
            let denom = if fortunes { 2000 } else { 1000 };
            if rng.random_ratio(1, denom) {
                self.relics
                    .active
                    .retain(|&r| r != RelicId::SilverFiligreeLantern);
                self.note_relic_destroyed();
            }
        }
        // Nest Egg: increment rounds held (affects sell value).
        if self.relics.has(RelicId::NestEgg) {
            *self.relic_counters.entry(RelicId::NestEgg).or_insert(0) += 1;
            self.relic_activations.push(RelicId::NestEgg);
        }
        // Phantom Relic: increment rounds held.
        if self.relics.has(RelicId::PhantomRelic) {
            *self
                .relic_counters
                .entry(RelicId::PhantomRelic)
                .or_insert(0) += 1;
            self.relic_activations.push(RelicId::PhantomRelic);
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
                    self.relic_activations.push(RelicId::Obsession);
                } else {
                    // Reset on use — rewards variety, not just avoidance.
                    self.relic_counters.insert(RelicId::Obsession, 0);
                }
            }
        }

        // Defeating the Boss completes an ante. Target scaling is now driven
        // by `run_number` in `apply_blind`, so `base_target` stays constant.
        let was_boss = self.blind == BlindKind::Boss;
        if was_boss {
            self.ante += 1;
            if self.relics.has(RelicId::BeggarsCup) {
                *self.relic_counters.entry(RelicId::BeggarsCup).or_insert(0) += 1;
            }
            if let Some(ref mut tut) = self.tutorial
                && tut.celebrate(crate::game::tutorial::TutorialMilestone::FirstBossCleared)
            {
                bus.push(GameEvent::TutorialMilestone(
                    crate::game::tutorial::TutorialMilestone::FirstBossCleared,
                ));
            }
        }
        // Heirloom: +1 mult per blind *played* (skips don't count — this
        // path only runs when a blind was cleared).
        if self.relics.has(RelicId::Heirloom) {
            *self.relic_counters.entry(RelicId::Heirloom).or_insert(0) += 1;
        }
        // Kong Collector: per-round kong tally is consumed at round end; clear it now.
        self.relic_counters.remove(&RelicId::KongCollector);
        self.run_number += 1;
        // `target_score` is recomputed by `apply_blind` when the next blind is picked.
        self.round_rules.clear();
        self.reset_round_resources();
        self.last_breakdown = None;
        self.scored_last_turn = false;
        self.quickdraw_uses_remaining = crate::game::run::QUICKDRAW_USES_PER_ROUND;
        self.joker_used = false;
        self.full_hand_played_this_round = false;
        self.boss.bonus_hand_size = 0;
        self.boss.gold_cost_per_play = 0;
        self.played_yaku_this_round.clear();
        self.honors_scored_this_round = false;
        self.upcoming_blind = self.upcoming_blind.next();
        self.blind = self.upcoming_blind;
        self.hand.clear();
        self.selected.clear();
        self.structure_sets.clear();
        self.structure_tiles.clear();
        self.tag_bonus_hand_size = 0;

        // Tutorial: advance to the next lesson and apply its overrides.
        // This may resize the hand and adjust the target.
        if self.tutorial.as_ref().is_some_and(|t| t.is_active()) {
            self.advance_tutorial_lesson();
        }

        // Roll the next ante's boss when we cross an ante boundary. Final
        // ante draws from the dedicated final pool; everyone else draws
        // without replacement from the regular pool.
        if was_boss {
            let mut rng = rand::rng();
            let boss_floor = self.mode.stake.boss_min_ante_floor();
            self.boss.upcoming = if self.ante == FINAL_ANTE {
                Some(boss::pick_final(&mut rng))
            } else if self.ante > FINAL_ANTE {
                None
            } else {
                boss::pick_for_ante_with_floor(
                    &mut self.boss.pool_remaining,
                    self.ante,
                    boss_floor,
                    &mut rng,
                )
            };
            // Bake the resolved effect now so reactive bosses see the
            // post-shop run state of the *outgoing* ante (their reveal
            // moment) and pick_blind shows the chosen variant immediately.
            self.resolve_upcoming_boss();
            // Roll fresh skip-reward tags for the new ante. Shop-oriented
            // rewards must survive into the post-boss shop, but any
            // one-blind combat bonuses should expire at the ante boundary.
            self.roll_ante_tags();
            self.clear_next_blind_tag_modifiers();
        }
    }

    /// Skip the upcoming blind: advance to the next in the cycle without
    /// playing or visiting the shop. Resets per-round state. Skipping is
    /// not allowed for the Boss blind — callers should check first.
    pub fn skip_to_next_blind(&mut self) {
        self.upcoming_blind = self.upcoming_blind.next();
        self.run_number += 1;
        // `target_score` is recomputed by `apply_blind` when the next blind is picked.
        self.round_rules.clear();
        self.reset_round_resources();
        self.last_breakdown = None;
        self.scored_last_turn = false;
        self.quickdraw_uses_remaining = crate::game::run::QUICKDRAW_USES_PER_ROUND;
        self.joker_used = false;
        // Reset per-round boss-effect state. The ante's `upcoming_boss` is
        // unchanged — skipping a Small/Big still leaves the same boss waiting.
        self.boss.bonus_hand_size = 0;
        self.boss.gold_cost_per_play = 0;
        self.played_yaku_this_round.clear();
        self.honors_scored_this_round = false;
        self.blind = self.upcoming_blind;
        self.hand.clear();
        self.selected.clear();
        self.structure_sets.clear();
        self.structure_tiles.clear();
    }
}
