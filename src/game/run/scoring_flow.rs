use super::relic_removal::TransformationPrimaryRelic;
use crate::{
    audio::SfxId,
    core::{
        boss::{self, BossKind},
        debuff::TileDebuff,
        hand::{DetectedMeld, MeldKind, enumerate_decompositions},
        hand_intent::{DecompositionBias, decomposition_affinity, infer_decomposition_bias},
        relic::{
            RelicId, ScoreContext, ScoreEconomyBundle, ScorePatternBundle, ScoreRelicBundle,
            ScoreRoundBundle, ScoreTileBundle,
        },
        rules::{BlindKind, RuleModifier},
        scoring::{EffectiveRelics, ScoreBreakdown, score_sets_with_original},
        structure::{
            StructureTriggerKind, StructureTriggerMeta, banked_meld_chips, can_trigger_structure,
            is_winning_structure_shape,
        },
        tile::{Suit, Tile},
    },
    game::{
        event_bus::{EventBus, GameEvent, GameOverReason},
        game_mode::HAND_SIZE,
        run::{RunState, enumerate_candidate_play_masks, structure_label_from_yaku},
    },
};

impl RunState {
    /// The House boss: structure cash-in is locked until every discard for the round is spent.
    pub(crate) fn cash_in_blocked_until_discards_spent(&self) -> bool {
        self.round_rules
            .contains(&RuleModifier::CashInRequiresNoDiscards)
            && self.discards_remaining > 0
    }

    /// Move validated melds from hand into **structure**; consumes one play.
    /// Returns points added this step (`0` or `1` success token for UI; real score is on trigger).
    pub fn commit_selection_to_structure(&mut self, bus: &mut EventBus) -> u64 {
        if self.plays_remaining == 0 || self.selected_count() == 0 {
            return 0;
        }

        let selected_tiles: Vec<Tile> = self
            .hand
            .iter()
            .zip(self.selected.iter())
            .filter(|&(_, &sel)| sel)
            .map(|(t, _)| *t)
            .collect();

        let (sets, scoring_tiles) = match self.try_validate_with_wildcards(&selected_tiles) {
            Some(result) => result,
            None => {
                bus.push(GameEvent::InvalidAction);
                return 0;
            }
        };
        let sets = self.pick_best_decomposition(sets, &scoring_tiles, &selected_tiles);

        if scoring_tiles != selected_tiles && self.relics.has(RelicId::JokerTile) {
            self.joker_used = true;
            self.push_relic_activation(RelicId::JokerTile);
        }
        self.tiles_played = self.tiles_played.saturating_add(scoring_tiles.len() as u32);

        let current_tile_count = self.structure_tiles.len();
        let kongs_after = self
            .structure_sets
            .iter()
            .chain(sets.iter())
            .filter(|s| s.kind == MeldKind::Kong)
            .count();
        if current_tile_count + scoring_tiles.len() > HAND_SIZE + kongs_after {
            bus.push(GameEvent::InvalidAction);
            return 0;
        }
        crate::game::engine_state::GameplayCoreState::with_run_mut(self, |core| {
            core.commit_sets_to_structure(&sets, &scoring_tiles);
        });
        bus.push(GameEvent::StructureCommitted);
        self.onboarding_notify_structure_committed();

        if self.relics.has(RelicId::MeltingIce) {
            let v = self.relic_counters.entry(RelicId::MeltingIce).or_insert(80);
            *v = (*v - 8).max(0);
            if *v == 0 {
                self.on_transformation_primary_burned(TransformationPrimaryRelic::MeltingIce, bus);
            }
        }
        if self.relics.has(RelicId::RustlingGooseEgg) {
            let v = self
                .relic_counters
                .entry(RelicId::RustlingGooseEgg)
                .or_insert(3);
            *v -= 1;
            if *v <= 0 {
                self.on_transformation_primary_burned(
                    TransformationPrimaryRelic::RustlingGooseEgg,
                    bus,
                );
            }
        }
        if self.relics.has(RelicId::Humility) {
            let has_honors = scoring_tiles
                .iter()
                .any(|t| matches!(t.suit, Suit::Wind | Suit::Dragon));
            let v = self.relic_counters.entry(RelicId::Humility).or_insert(0);
            if has_honors {
                *v = 0;
            } else {
                *v += 1;
            }
        }
        if self.relics.has(RelicId::LotusBloom) {
            let flower_count = scoring_tiles
                .iter()
                .filter(|t| t.suit == Suit::Flower)
                .count() as i32;
            if flower_count > 0 {
                *self.relic_counters.entry(RelicId::LotusBloom).or_insert(0) += flower_count;
                self.push_relic_activation(RelicId::LotusBloom);
            }
        }
        let copy_eff = EffectiveRelics::from_roster(&self.relics);
        if copy_eff.has(&self.relics, RelicId::KongCollector) {
            let kong_count = sets.iter().filter(|s| s.kind == MeldKind::Kong).count() as i32;
            if kong_count > 0 {
                let times = copy_eff.count(&self.relics, RelicId::KongCollector);
                *self
                    .relic_counters
                    .entry(RelicId::KongCollector)
                    .or_insert(0) += kong_count.saturating_mul(times as i32);
                self.push_relic_activation(RelicId::KongCollector);
            }
        }

        self.scored_last_turn = false;

        crate::game::engine_state::GameplayCoreState::with_run_mut(self, |core| {
            let _ = core.take_selected_tiles();
        });

        let effective = boss::effective_hand_size(self);
        let draw_target = if copy_eff.has(&self.relics, RelicId::QuickDraw) {
            self.push_relic_activation(RelicId::QuickDraw);
            effective + 1
        } else {
            effective
        };
        let mut drawn: Vec<Tile> = Vec::new();
        while self.hand.len() + drawn.len() < draw_target {
            let Some(t) = self.wall.draw() else { break };
            bus.push(GameEvent::TileDrawn);
            drawn.push(t);
        }
        self.chronicle
            .note_tiles_drawn(drawn.len().min(u32::MAX as usize) as u32);
        let restocked = !drawn.is_empty();
        crate::game::engine_state::GameplayCoreState::with_run_mut(self, |core| {
            core.push_drawn_tiles(&drawn);
        });
        if restocked {
            self.times_restocked = self.times_restocked.saturating_add(1);
        }

        self.set_magnet_draw_fourths(bus);
        crate::game::engine_state::GameplayCoreState::with_run_mut(self, |core| {
            core.finalize_hand_after_draw();
        });
        self.restamp_hand_enhancements();

        if self.blind == BlindKind::Boss
            && let Some(eff) = self.boss.effect.take()
        {
            if let Some(hook) = eff.on_play {
                hook(self);
            }
            self.boss.effect = Some(eff);
        }

        self.try_autotrigger_structure_full(bus);
        if self.plays_remaining == 0
            && self.round_score < self.target_score as u64
            && !self.structure_sets.is_empty()
        {
            let _ = self.trigger_structure(StructureTriggerKind::AutoNoPlays, bus);
        }
        self.emit_round_resolution_events(bus);
        1
    }

    /// Core scoring path for resolved melds (structure cash-in).
    pub(super) fn apply_scored_melds(
        &mut self,
        sets: Vec<DetectedMeld>,
        scoring_tiles: Vec<Tile>,
        original_for_wildcard: Vec<Tile>,
        structure_meta: Option<StructureTriggerMeta>,
        bus: &mut EventBus,
    ) -> u64 {
        let destroy_glass_cannon = self.relics.has(RelicId::GlassCannon);
        let rw = Some(BlindKind::round_wind_for_ante(self.ante));
        let bonus_rw = self.bonus_round_wind_for_yaku();
        let scoring_tile_debuffs = self.scoring_tile_debuffs(&scoring_tiles);
        let ctx = ScoreContext {
            relic: ScoreRelicBundle {
                roster: &self.relics,
                counters: self.relic_counters.clone(),
            },
            tiles: ScoreTileBundle {
                debuffs: &scoring_tile_debuffs,
                hand_for_ghost: &self.hand,
            },
            round: ScoreRoundBundle {
                scored_last_turn: self.scored_last_turn,
                plays_used: self.round_play_cap().saturating_sub(self.plays_remaining),
                round_wind: rw,
                bonus_round_wind: bonus_rw,
                played_yaku_this_round: self.played_yaku_this_round.clone(),
                is_final_play: self.plays_remaining == 0,
            },
            pattern: ScorePatternBundle {
                dora_faces: self.wall.dora_faces(),
                available_yaku: self.available_yaku.clone(),
                yaku_levels: Some(self.yaku_levels.clone()),
            },
            economy: ScoreEconomyBundle {
                gold: self.gold,
                total_score: self.total_score_earned,
            },
            structure: structure_meta,
        };
        let breakdown = score_sets_with_original(
            &scoring_tiles,
            &sets,
            &ctx,
            &self.round_rules,
            &original_for_wildcard,
        );
        let mut breakdown_total = breakdown.total;
        if self.memorial_round.next_cashin_bonus_chips > 0 {
            let yaku_ok = self
                .memorial_round
                .next_cashin_yaku
                .map(|required| breakdown.detected_yaku.contains(&required))
                .unwrap_or(true);
            if yaku_ok {
                breakdown_total = breakdown_total.saturating_add(self.memorial_round.next_cashin_bonus_chips);
            }
            self.memorial_round.next_cashin_bonus_chips = 0;
            self.memorial_round.next_cashin_yaku = None;
        }
        let pre_round = self.round_score;
        let absorb_excess = (self.relics.has(crate::core::relic::RelicId::Chrysalis)
            || self
                .relics
                .has(crate::core::relic::RelicId::MonarchButterfly))
            && pre_round >= self.target_score as u64;
        let applied = if absorb_excess { 0u64 } else { breakdown_total };

        self.round_score = self.round_score.saturating_add(applied);
        self.total_score_earned = self.total_score_earned.saturating_add(applied);
        if applied > self.best_structure_score {
            self.best_structure_score = applied;
            self.best_structure_name = structure_label_from_yaku(&breakdown.detected_yaku);
            self.best_hand_tiles = scoring_tiles.iter().map(|t| t.display_copy()).collect();
        }

        if absorb_excess && breakdown_total > 0 {
            let cur = self
                .relic_counters
                .entry(crate::core::relic::RelicId::MonarchButterfly)
                .or_insert(0);
            let room = i64::from(i32::MAX) - i64::from(*cur);
            let add = (breakdown_total.min(room.max(0) as u64)) as i32;
            *cur = cur.saturating_add(add);
            if self.relics.has(crate::core::relic::RelicId::Chrysalis) {
                self.push_relic_activation(crate::core::relic::RelicId::Chrysalis);
            }
            if self
                .relics
                .has(crate::core::relic::RelicId::MonarchButterfly)
            {
                self.push_relic_activation(crate::core::relic::RelicId::MonarchButterfly);
            }

            let excess = self
                .relic_counters
                .get(&crate::core::relic::RelicId::MonarchButterfly)
                .copied()
                .unwrap_or(0);
            if self.relics.has(crate::core::relic::RelicId::Chrysalis)
                && excess >= crate::core::relic::CHRYSALIS_HATCH_EXCESS_THRESHOLD
                && let Some(pos) = self
                    .relics
                    .active
                    .iter()
                    .position(|&r| r == crate::core::relic::RelicId::Chrysalis)
            {
                self.complete_chrysalis_hatch_in_slot(pos, bus);
            }
        }

        if self.relics.has(RelicId::TilePolisher) {
            let tile_count: i32 = sets.iter().map(|s| s.tile_ids.len() as i32).sum();
            *self
                .relic_counters
                .entry(RelicId::TilePolisher)
                .or_insert(0) += 3 * tile_count;
            self.push_relic_activation(RelicId::TilePolisher);
        }
        if self.relics.has(RelicId::RiverRunner) {
            let seq_count = sets.iter().filter(|s| s.kind == MeldKind::Sequence).count() as i32;
            if seq_count > 0 {
                *self.relic_counters.entry(RelicId::RiverRunner).or_insert(0) += 20 * seq_count;
                self.push_relic_activation(RelicId::RiverRunner);
            }
        }
        if self.relics.has(RelicId::Taotie) {
            // The hungry mask devours honors at the moment of consumption.
            // Each devoured honor permanently grows Taotie's chip bonus by
            // CHIPS_PER_DEVOURED and is removed from the run's tile supply
            // (won't reappear in next round's wall — same `removed_tile_ids`
            // primitive as other permanent destruction). The wall has 28 honors
            // total, so we skip any removal budget check.
            //
            // Anti-synergy with Honor Fury / Windreader / Yakuhai is
            // deliberate — feeding the mask drains the supply those relics
            // depend on, which gives the build a real shape.
            const CHIPS_PER_DEVOURED: i32 = 20;
            let mut devoured = 0i32;
            for tile in &scoring_tiles {
                if matches!(tile.suit, Suit::Wind | Suit::Dragon) {
                    self.removed_tile_ids.insert(tile.id);
                    self.tile_enhancements.remove(&tile.id);
                    devoured += 1;
                }
            }
            if devoured > 0 {
                *self.relic_counters.entry(RelicId::Taotie).or_insert(0) +=
                    CHIPS_PER_DEVOURED * devoured;
                self.push_relic_activation(RelicId::Taotie);
                bus.push(GameEvent::TilesDestroyed);
            }
        }
        if self.relics.has(RelicId::StarTile) && !breakdown.detected_yaku.is_empty() {
            use rand::RngExt;
            use rand::seq::IndexedRandom;

            let mut rng = rand::rng();
            let prob = if self.relics.has(RelicId::FortunesFavor) {
                2
            } else {
                1
            };
            if rng.random_ratio(prob, 4)
                && let Some(&y) = breakdown.detected_yaku.choose(&mut rng)
            {
                let _new_level = self.yaku_levels.level_up(y);
                self.push_relic_activation(RelicId::StarTile);
            }
        }
        if breakdown.flower_gold > 0 {
            self.apply_gold_reward(breakdown.flower_gold, Some(bus));
        }
        let scored_full_hand = breakdown
            .detected_yaku
            .contains(&crate::core::yaku::YakuKind::FullHand);
        if scored_full_hand {
            self.full_hand_played_this_round = true;
        }
        if self.relics.has(RelicId::KanDrum) {
            let kong_count = sets.iter().filter(|s| s.kind == MeldKind::Kong).count() as u32;
            if kong_count > 0 {
                self.plays_remaining = self.plays_remaining.saturating_add(kong_count);
                self.push_relic_activation(RelicId::KanDrum);
            }
        }
        for &y in &breakdown.detected_yaku {
            *self.yaku_times_played.entry(y).or_insert(0) += 1;
            if !self.played_yaku_this_round.contains(&y) {
                self.played_yaku_this_round.push(y);
            }
            bus.push(GameEvent::YakuScored(y));
        }
        if breakdown
            .detected_yaku
            .contains(&crate::core::yaku::YakuKind::KokushiMusou)
        {
            bus.push(GameEvent::AchievementUnlocked(
                crate::steam::Achievement::ThirteenOrphans,
            ));
        }
        self.chronicle
            .absorb_scoring(&breakdown, &scoring_tiles, &self.yaku_levels);
        self.chronicle.note_turn();
        self.last_breakdown = Some(breakdown);
        self.scored_last_turn = breakdown_total > 0;

        if !self.honors_scored_this_round
            && scoring_tiles
                .iter()
                .any(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
        {
            self.honors_scored_this_round = true;
        }

        if scored_full_hand {
            let treasures = EffectiveRelics::from_roster(&self.relics)
                .count(&self.relics, RelicId::EightTreasures);
            for _ in 0..treasures {
                use rand::seq::IndexedRandom;

                let mut rng = rand::rng();
                if let Some(&z) = crate::core::zodiac::ZodiacKind::all().choose(&mut rng) {
                    self.consumables
                        .items
                        .push(crate::core::consumable::Consumable::Zodiac(z));
                    self.push_relic_activation(RelicId::EightTreasures);
                }
            }
        }

        if breakdown_total > 0 && self.relics.has(RelicId::TeaCeremony) {
            self.push_relic_activation(RelicId::TeaCeremony);
            let phase = self
                .relic_counters
                .get(&RelicId::TeaCeremony)
                .copied()
                .unwrap_or(0)
                .clamp(0, 3);
            if phase >= 3 {
                if let Some(pos) = self
                    .relics
                    .active
                    .iter()
                    .position(|&r| r == RelicId::TeaCeremony)
                {
                    self.complete_tea_ceremony_graduation_in_slot(pos, bus);
                }
            } else {
                self.relic_counters.insert(RelicId::TeaCeremony, phase + 1);
            }
        }

        if destroy_glass_cannon {
            let _ = self.destroy_relic_with_activation_fx(RelicId::GlassCannon, Some(bus));
        }

        applied
    }

    fn scoring_tile_debuffs(&self, scoring_tiles: &[Tile]) -> Vec<TileDebuff> {
        let mut debuffs = self.tile_debuffs.clone();
        let dragon_without_honors = self.blind == BlindKind::Boss
            && self.boss.upcoming == Some(BossKind::Dragon)
            && !scoring_tiles
                .iter()
                .any(|t| matches!(t.suit, Suit::Wind | Suit::Dragon));
        if dragon_without_honors {
            for suit in [Suit::Manzu, Suit::Souzu, Suit::Pinzu, Suit::Flower] {
                if scoring_tiles.iter().any(|t| t.suit == suit) {
                    debuffs.push(TileDebuff::Suit(suit));
                }
            }
        }
        debuffs
    }

    pub fn trigger_structure(&mut self, kind: StructureTriggerKind, bus: &mut EventBus) -> u64 {
        if self.structure_sets.is_empty() {
            return 0;
        }
        if self.cash_in_blocked_until_discards_spent() {
            return 0;
        }
        let rw = Some(BlindKind::round_wind_for_ante(self.ante));
        let bonus_rw = self.bonus_round_wind_for_yaku();
        if kind == StructureTriggerKind::Manual
            && !can_trigger_structure(
                &self.structure_tiles,
                &self.structure_sets,
                rw,
                bonus_rw,
                &self.available_yaku,
                &self.round_rules,
            )
        {
            return 0;
        }

        let structure_sets = std::mem::take(&mut self.structure_sets);
        let structure_tiles = std::mem::take(&mut self.structure_tiles);
        let meta = StructureTriggerMeta {
            meld_count: structure_sets.len() as u32,
            inject_chicken_if_no_yaku: true,
        };
        let earned = self.apply_scored_melds(
            structure_sets,
            structure_tiles.clone(),
            structure_tiles,
            Some(meta),
            bus,
        );
        let _ = kind;
        bus.push(GameEvent::UiSound(SfxId::CashIn));
        self.hand.sort();
        earned
    }

    pub(crate) fn try_autotrigger_structure_full(&mut self, bus: &mut EventBus) {
        if !self.auto_cash_in_on_full_structure {
            return;
        }
        if self.structure_sets.is_empty() {
            return;
        }
        if self.cash_in_blocked_until_discards_spent() {
            return;
        }
        let rw = Some(BlindKind::round_wind_for_ante(self.ante));
        let bonus_rw = self.bonus_round_wind_for_yaku();
        if !is_winning_structure_shape(&self.structure_tiles, &self.structure_sets) {
            return;
        }
        if !can_trigger_structure(
            &self.structure_tiles,
            &self.structure_sets,
            rw,
            bonus_rw,
            &self.available_yaku,
            &self.round_rules,
        ) {
            return;
        }
        let _ = self.trigger_structure(StructureTriggerKind::AutoFull, bus);
    }

    pub(super) fn emit_round_resolution_events(&mut self, bus: &mut EventBus) {
        bus.push(GameEvent::ScoreUpdated);
        if self.round_score >= self.target_score as u64 {
            let base_reward = self.blind.clear_reward();
            let unused_play_bonus = self.plays_remaining;
            let interest = (self.gold.max(0) as u32 / 5).min(3);
            let green_luck_bonus =
                if self.relics.has(RelicId::GreenLuck) && !self.honors_scored_this_round {
                    self.push_relic_activation(RelicId::GreenLuck);
                    4
                } else {
                    0
                };
            let gold_idol_bonus = if self.relics.has(RelicId::GoldIdol) {
                self.push_relic_activation(RelicId::GoldIdol);
                3u32
            } else {
                0
            };
            let jade_abacus_bonus = if self.relics.has(RelicId::JadeAbacus) {
                let bonus = (self.gold.max(0) as u32 / 4).min(4);
                if bonus > 0 {
                    self.push_relic_activation(RelicId::JadeAbacus);
                }
                bonus
            } else {
                0
            };
            let patience_bonus = if self.relics.has(RelicId::Patience) {
                let bonus = 2 * self.discards_remaining;
                if bonus > 0 {
                    self.push_relic_activation(RelicId::Patience);
                }
                bonus
            } else {
                0
            };
            let kong_collector_bonus = if self.relics.has(RelicId::KongCollector) {
                let kongs = self
                    .relic_counters
                    .get(&RelicId::KongCollector)
                    .copied()
                    .unwrap_or(0)
                    .max(0) as u32;
                let bonus = 5u32.saturating_mul(kongs);
                if bonus > 0 {
                    self.push_relic_activation(RelicId::KongCollector);
                }
                bonus
            } else {
                0
            };
            let beggars_cup_bonus = if self.relics.has(RelicId::BeggarsCup) {
                let bosses = self
                    .relic_counters
                    .get(&RelicId::BeggarsCup)
                    .copied()
                    .unwrap_or(0)
                    .max(0) as u32;
                let bonus = 1u32.saturating_add(bosses);
                self.push_relic_activation(RelicId::BeggarsCup);
                bonus
            } else {
                0
            };
            let cosmopolitan_bonus = if self.relics.has(RelicId::Cosmopolitan) {
                let unique_yaku = self.played_yaku_this_round.len() as u32;
                if unique_yaku > 0 {
                    self.push_relic_activation(RelicId::Cosmopolitan);
                }
                unique_yaku
            } else {
                0
            };
            if self.relics.has(RelicId::Temperance) && self.plays_remaining > 0 {
                let stacks = self.plays_remaining as i32 * 5;
                *self.relic_counters.entry(RelicId::Temperance).or_insert(0) += stacks;
                self.push_relic_activation(RelicId::Temperance);
            }
            let memorial_clear = self.memorial_round.clear_gold_bonus;
            self.memorial_round.clear_gold_bonus = 0;
            let gold_earned = base_reward
                .saturating_add(unused_play_bonus)
                .saturating_add(interest)
                .saturating_add(green_luck_bonus)
                .saturating_add(gold_idol_bonus)
                .saturating_add(jade_abacus_bonus)
                .saturating_add(patience_bonus)
                .saturating_add(kong_collector_bonus)
                .saturating_add(beggars_cup_bonus)
                .saturating_add(cosmopolitan_bonus)
                .saturating_add(memorial_clear);
            let reward_note = if gold_earned > 0 {
                format!("+{gold_earned} Gold")
            } else {
                String::new()
            };
            let boss_name = if self.blind == BlindKind::Boss {
                self.boss.upcoming.map(|b| b.name())
            } else {
                None
            };
            self.chronicle.record_blind_cleared(
                self.ante,
                self.blind,
                boss_name,
                reward_note,
                self.round_score,
            );
            bus.push(GameEvent::RoundComplete {
                reached_target: true,
                payout: crate::game::event_bus::RoundPayout {
                    base_reward,
                    unused_play_bonus,
                    interest,
                    green_luck_bonus,
                    total: gold_earned,
                },
            });
            if self.blind == BlindKind::Boss
                && let Some(bk) = self.boss.upcoming
            {
                bus.push(GameEvent::BossDefeated(bk));
            }
        } else if let Some(reason) = self.round_failure_reason()
            && !self.try_second_wind_salvage(reason, bus)
        {
            bus.push(GameEvent::GameOver { reason });
        }
    }

    /// When a round would end in defeat, Second Wind is destroyed and the blind
    /// is forfeited (no gold payout); [`RunState::forfeit_current_blind_second_wind`]
    /// runs when the UI drains the deferred `RoundComplete`.
    fn try_second_wind_salvage(&mut self, _reason: GameOverReason, bus: &mut EventBus) -> bool {
        if !self.relics.has(RelicId::SecondWind) {
            return false;
        }
        let _ = self.destroy_relic_with_activation_fx(RelicId::SecondWind, Some(bus));
        bus.push(GameEvent::RoundComplete {
            reached_target: false,
            payout: crate::game::event_bus::RoundPayout::default(),
        });
        true
    }

    /// Banked meld chips in structure (for HUD tiers).
    pub fn structure_banked_meld_chips(&self) -> i32 {
        banked_meld_chips(&self.structure_tiles, &self.structure_sets)
    }

    /// Whether [`Self::trigger_structure_manual`] can score (structure non-empty and rules allow).
    pub fn can_trigger_structure_now(&self) -> bool {
        if self.structure_sets.is_empty() {
            return false;
        }
        if self.cash_in_blocked_until_discards_spent() {
            return false;
        }
        let rw = Some(BlindKind::round_wind_for_ante(self.ante));
        let bonus_rw = self.bonus_round_wind_for_yaku();
        can_trigger_structure(
            &self.structure_tiles,
            &self.structure_sets,
            rw,
            bonus_rw,
            &self.available_yaku,
            &self.round_rules,
        )
    }

    /// Read-only scoring breakdown for a manual structure cash-in (no state change).
    /// RNG-driven relic hooks in a real [`Self::trigger_structure`] may differ slightly.
    pub fn preview_manual_trigger_breakdown(&self) -> Option<ScoreBreakdown> {
        if self.structure_sets.is_empty() {
            return None;
        }
        if self.cash_in_blocked_until_discards_spent() {
            return None;
        }
        let rw = Some(BlindKind::round_wind_for_ante(self.ante));
        let bonus_rw = self.bonus_round_wind_for_yaku();
        if !can_trigger_structure(
            &self.structure_tiles,
            &self.structure_sets,
            rw,
            bonus_rw,
            &self.available_yaku,
            &self.round_rules,
        ) {
            return None;
        }
        let sets = self.structure_sets.clone();
        let scoring_tiles = self.structure_tiles.clone();
        let original_for_wildcard = scoring_tiles.clone();
        let scoring_tile_debuffs = self.scoring_tile_debuffs(&scoring_tiles);
        let meta = StructureTriggerMeta {
            meld_count: sets.len() as u32,
            inject_chicken_if_no_yaku: true,
        };
        let ctx = ScoreContext {
            relic: ScoreRelicBundle {
                roster: &self.relics,
                counters: self.relic_counters.clone(),
            },
            tiles: ScoreTileBundle {
                debuffs: &scoring_tile_debuffs,
                hand_for_ghost: &self.hand,
            },
            round: ScoreRoundBundle {
                scored_last_turn: self.scored_last_turn,
                plays_used: self.round_play_cap().saturating_sub(self.plays_remaining),
                round_wind: rw,
                bonus_round_wind: bonus_rw,
                played_yaku_this_round: self.played_yaku_this_round.clone(),
                is_final_play: self.plays_remaining == 0,
            },
            pattern: ScorePatternBundle {
                dora_faces: self.wall.dora_faces(),
                available_yaku: self.available_yaku.clone(),
                yaku_levels: Some(self.yaku_levels.clone()),
            },
            economy: ScoreEconomyBundle {
                gold: self.gold,
                total_score: self.total_score_earned,
            },
            structure: Some(meta),
        };
        Some(score_sets_with_original(
            &scoring_tiles,
            &sets,
            &ctx,
            &self.round_rules,
            &original_for_wildcard,
        ))
    }

    /// Read-only preview of points from a manual structure cash-in (no state change).
    /// RNG-driven relic hooks in a real [`Self::trigger_structure`] may differ slightly.
    pub fn preview_manual_trigger_total(&self) -> u64 {
        self.preview_manual_trigger_breakdown()
            .map(|breakdown| breakdown.total)
            .unwrap_or(0)
    }

    /// Manual structure cash-in (no play cost) + round resolution events.
    pub fn trigger_structure_manual(&mut self, bus: &mut EventBus) -> u64 {
        let earned = self.trigger_structure(StructureTriggerKind::Manual, bus);
        if earned > 0 {
            self.onboarding_notify_cash_in();
        }
        self.emit_round_resolution_events(bus);
        earned
    }

    /// When the submission could be a full winning hand (14+ tiles), enumerate
    /// every valid meld decomposition and pick the highest-scoring one. For
    /// shorter plays (partial structure commits), only re-rank when the
    /// player's full hand reveals a clear pairs-vs-triplets lean — otherwise
    /// the backtracker's first-found decomposition is fine and enumeration
    /// overhead isn't worth it.
    ///
    /// Ties on full hands fall back to affinity, then to the first
    /// decomposition found.
    fn pick_best_decomposition(
        &self,
        default_sets: Vec<DetectedMeld>,
        scoring_tiles: &[Tile],
        original_tiles: &[Tile],
    ) -> Vec<DetectedMeld> {
        // A full hand has 14 + kong_count tiles (kongs use 4 tiles each).
        let kongs = default_sets
            .iter()
            .filter(|s| s.kind == MeldKind::Kong)
            .count();
        let is_full_hand =
            scoring_tiles.len() >= HAND_SIZE && scoring_tiles.len() == HAND_SIZE + kongs;
        let bias = infer_decomposition_bias(&self.hand);
        // Partial submissions only need re-ranking when the player's hand
        // reveals an intent the greedy backtracker would override (e.g.
        // committing 1-1-1-1 as two pairs while building Chiitoitsu).
        if !is_full_hand && matches!(bias, DecompositionBias::Neutral) {
            return default_sets;
        }
        let rules = self.validation_rules_for_structure_commits();
        let alternatives = enumerate_decompositions(scoring_tiles, &rules);
        if alternatives.len() <= 1 {
            return default_sets;
        }
        if !is_full_hand {
            // Affinity-only pick: no scoring engine for partial commits.
            let mut best = default_sets;
            let mut best_affinity = decomposition_affinity(&best, bias);
            for candidate in alternatives {
                let affinity = decomposition_affinity(&candidate, bias);
                if affinity > best_affinity {
                    best_affinity = affinity;
                    best = candidate;
                }
            }
            return best;
        }
        let scoring_tile_debuffs = self.scoring_tile_debuffs(scoring_tiles);
        let rw = Some(BlindKind::round_wind_for_ante(self.ante));
        let bonus_rw = self.bonus_round_wind_for_yaku();
        let ctx = ScoreContext {
            relic: ScoreRelicBundle {
                roster: &self.relics,
                counters: self.relic_counters.clone(),
            },
            tiles: ScoreTileBundle {
                debuffs: &scoring_tile_debuffs,
                hand_for_ghost: &self.hand,
            },
            round: ScoreRoundBundle {
                scored_last_turn: self.scored_last_turn,
                plays_used: self.round_play_cap().saturating_sub(self.plays_remaining),
                round_wind: rw,
                bonus_round_wind: bonus_rw,
                played_yaku_this_round: self.played_yaku_this_round.clone(),
                is_final_play: self.plays_remaining == 0,
            },
            pattern: ScorePatternBundle {
                dora_faces: self.wall.dora_faces(),
                available_yaku: self.available_yaku.clone(),
                yaku_levels: Some(self.yaku_levels.clone()),
            },
            economy: ScoreEconomyBundle {
                gold: self.gold,
                total_score: self.total_score_earned,
            },
            structure: None,
        };
        let mut best = default_sets;
        let mut best_total = score_sets_with_original(
            scoring_tiles,
            &best,
            &ctx,
            &self.round_rules,
            original_tiles,
        )
        .total;
        let mut best_affinity = decomposition_affinity(&best, bias);
        for candidate in alternatives {
            let total = score_sets_with_original(
                scoring_tiles,
                &candidate,
                &ctx,
                &self.round_rules,
                original_tiles,
            )
            .total;
            let affinity = decomposition_affinity(&candidate, bias);
            let take = total > best_total || (total == best_total && affinity > best_affinity);
            if take {
                best_total = total;
                best_affinity = affinity;
                best = candidate;
            }
        }
        best
    }

    /// Rules used when validating a selection before committing it to the structure
    /// (differs from [`RunState::round_rules`] e.g. honor-gated tutorial modifiers).
    pub fn validation_rules_for_structure_commits(&self) -> Vec<RuleModifier> {
        self.round_rules
            .iter()
            .copied()
            .filter(|rule| *rule != RuleModifier::RequireHonor)
            .collect()
    }

    fn has_any_committable_play(&self) -> bool {
        if self.plays_remaining == 0 {
            return false;
        }
        let hand_len = self.hand.len();
        if !(2..=20).contains(&hand_len) {
            return false;
        }

        let rules = self.validation_rules_for_structure_commits();
        for mask in enumerate_candidate_play_masks(&self.hand, &rules) {
            let indices: Vec<usize> = (0..hand_len).filter(|i| mask & (1 << i) != 0).collect();
            let tiles: Vec<Tile> = indices.iter().map(|&i| self.hand[i]).collect();
            let Some((new_sets, scoring_tiles)) = self.try_validate_with_wildcards(&tiles) else {
                continue;
            };

            let kongs_after = self
                .structure_sets
                .iter()
                .chain(new_sets.iter())
                .filter(|s| s.kind == MeldKind::Kong)
                .count();
            if self.structure_tiles.len() + scoring_tiles.len() > HAND_SIZE + kongs_after {
                continue;
            }
            return true;
        }
        false
    }

    fn no_actions_remaining(&self) -> bool {
        if self.round_score >= self.target_score as u64 || self.plays_remaining == 0 {
            return false;
        }

        let can_discard = self.discards_remaining > 0 && !self.hand.is_empty();
        !self.has_any_committable_play() && !self.can_trigger_structure_now() && !can_discard
    }

    pub fn round_failure_reason(&self) -> Option<GameOverReason> {
        if self.round_score >= self.target_score as u64 {
            None
        } else if self.plays_remaining == 0 {
            Some(GameOverReason::OutOfPlays)
        } else if self.no_actions_remaining() {
            Some(GameOverReason::NoActionsRemaining)
        } else {
            None
        }
    }
}
