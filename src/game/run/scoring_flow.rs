use super::relic_removal::TransformationPrimaryRelic;
use crate::{
    OrdealKindExt,
    core::{
        debuff::TileDebuff,
        hand::{DetectedMeld, MeldKind, decomposition_canonical_key, enumerate_decompositions, kong_structure_bonus},
        hand_intent::{decomposition_affinity, infer_decomposition_bias},
        ordeal::{self, OrdealKind},
        relic::{
            RelicId, ScoreContext, ScoreEconomyBundle, ScorePatternBundle, ScoreRelicBundle,
            ScoreRoundBundle, ScoreTileBundle,
        },
        rules::{ChamberKind, RuleModifier},
        scoring::{EffectiveRelics, ScoreBreakdown, score_sets_with_original, tile_is_debuffed},
        structure::{
            StructureTriggerKind, StructureTriggerMeta, can_trigger_structure,
            is_winning_structure_shape, played_meld_chips, star_tile_yaku_pool,
            structure_cannot_grow_further,
        },
        tile::{Suit, Tile},
        yaku::{YakuKind, yaku_after_pool_filter},
    },
    game::{
        event_bus::{EventBus, GameEvent, GameOverReason},
        game_mode::HAND_SIZE,
        run::{RunState, enumerate_candidate_play_masks, structure_label_from_yaku},
    },
    sfx_id::SfxId,
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
            self.resolve_round_end(bus);
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

        self.tiles_played = self.tiles_played.saturating_add(scoring_tiles.len() as u32);

        if !self.selection_commit_capacity_ok(&sets, scoring_tiles.len()) {
            bus.push(GameEvent::InvalidAction);
            return 0;
        }
        crate::game::engine_state::GameplayCoreState::with_run_mut(self, |core| {
            core.commit_sets_to_structure(&sets, &scoring_tiles);
        });
        bus.push(GameEvent::StructureCommitted);
        self.onboarding_notify_structure_committed();

        if self.relics.has(RelicId::MeltingIce) {
            let v = self
                .relic_counters
                .entry(RelicId::MeltingIce)
                .or_insert(crate::core::relic::MELTING_ICE_START_CHIPS);
            *v = (*v - crate::core::relic::MELTING_ICE_DECAY_PER_PLAY).max(0);
            if *v == 0 {
                self.on_transformation_primary_burned(TransformationPrimaryRelic::MeltingIce, bus);
            }
        }
        if self.relics.has(RelicId::XxxlEgg) {
            let v = self.relic_counters.entry(RelicId::XxxlEgg).or_insert(3);
            *v -= 1;
            if *v <= 0 {
                self.on_transformation_primary_burned(TransformationPrimaryRelic::XxxlEgg, bus);
            }
        }
        let has_honors = scoring_tiles
            .iter()
            .any(|t| matches!(t.suit, Suit::Wind | Suit::Dragon));
        if self.relics.has(RelicId::Humility) {
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

        let effective = ordeal::effective_hand_size(self);
        let draw_target = if copy_eff.has(&self.relics, RelicId::QuickDraw) {
            self.push_relic_activation(RelicId::QuickDraw);
            effective + 2
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

        if self.chamber == ChamberKind::Ordeal
            && let Some(eff) = self.ordeal.effect.take()
        {
            if let Some(hook) = eff.on_play {
                hook(self);
            }
            self.ordeal.effect = Some(eff);
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
        let rw = Some(ChamberKind::round_wind_for_wing(self.wing));
        let bonus_rw = self.bonus_round_wind_for_yaku();
        let scoring_tile_debuffs = self.scoring_tile_debuffs(&scoring_tiles);
        if scoring_tiles.iter().any(|t| {
            matches!(t.suit, Suit::Wind | Suit::Dragon)
                && !tile_is_debuffed(t, &scoring_tile_debuffs)
        }) {
            self.honors_scored_this_round = true;
        }
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
                yen: self.yen,
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
                breakdown_total =
                    breakdown_total.saturating_add(self.memorial_round.next_cashin_bonus_chips);
            }
            self.memorial_round.next_cashin_bonus_chips = 0;
            self.memorial_round.next_cashin_yaku = None;
        }
        let pre_round = self.round_score;
        let target = self.target_score as u64;
        let absorbs_excess = self.relics.has(crate::core::relic::RelicId::Chrysalis)
            || self
                .relics
                .has(crate::core::relic::RelicId::MonarchButterfly);
        let (applied, excess_to_absorb) = if absorbs_excess {
            if pre_round >= target {
                (0u64, breakdown_total)
            } else if pre_round.saturating_add(breakdown_total) <= target {
                (breakdown_total, 0u64)
            } else {
                let to_target = target.saturating_sub(pre_round);
                (to_target, breakdown_total.saturating_sub(to_target))
            }
        } else {
            (breakdown_total, 0u64)
        };

        self.round_score = self.round_score.saturating_add(applied);
        self.total_score_earned = self.total_score_earned.saturating_add(applied);
        if breakdown_total > self.best_structure_score {
            self.best_structure_score = breakdown_total;
            self.best_structure_name = structure_label_from_yaku(&breakdown.detected_yaku);
            self.best_hand_tiles = scoring_tiles.iter().map(|t| t.display_copy()).collect();
        }

        if excess_to_absorb > 0 {
            let cur = self
                .relic_counters
                .entry(crate::core::relic::RelicId::MonarchButterfly)
                .or_insert(0);
            let room = i64::from(i32::MAX) - i64::from(*cur);
            let add = (excess_to_absorb.min(room.max(0) as u64)) as i32;
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
                .or_insert(0) += crate::core::relic::TILE_POLISHER_CHIPS_PER_TILE * tile_count;
            self.push_relic_activation(RelicId::TilePolisher);
        }
        if self.relics.has(RelicId::RiverRunner) {
            let seq_count = sets.iter().filter(|s| s.kind == MeldKind::Sequence).count() as i32;
            if seq_count > 0 {
                *self.relic_counters.entry(RelicId::RiverRunner).or_insert(0) +=
                    crate::core::relic::RIVER_RUNNER_CHIPS_PER_SEQUENCE * seq_count;
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
            use crate::core::relic::TAOTIE_CHIPS_PER_DEVOURED;
            let mut devoured = 0i32;
            for tile in &scoring_tiles {
                if matches!(tile.suit, Suit::Wind | Suit::Dragon) {
                    self.removed_tile_ids.insert(tile.id);
                    self.tile_enhancements.remove(&tile.id);
                    self.transformed_tiles.remove(&tile.id);
                    devoured += 1;
                }
            }
            if devoured > 0 {
                *self.relic_counters.entry(RelicId::Taotie).or_insert(0) +=
                    TAOTIE_CHIPS_PER_DEVOURED * devoured;
                self.push_relic_activation(RelicId::Taotie);
                bus.push(GameEvent::TilesDestroyed);
            }
        }
        if self.chamber == ChamberKind::Ordeal {
            let destroy_suit = match self.ordeal.upcoming {
                Some(OrdealKind::DeadAir) => Some(Suit::Wind),
                Some(OrdealKind::StGeorge) => Some(Suit::Dragon),
                _ => None,
            };
            if let Some(suit) = destroy_suit {
                let mut destroyed = 0u32;
                for tile in &scoring_tiles {
                    if tile.suit == suit {
                        self.removed_tile_ids.insert(tile.id);
                        self.tile_enhancements.remove(&tile.id);
                        self.transformed_tiles.remove(&tile.id);
                        destroyed += 1;
                    }
                }
                if destroyed > 0 {
                    bus.push(GameEvent::TilesDestroyed);
                }
            }
        }
        if self.relics.has(RelicId::StarTile) {
            use rand::RngExt;
            use rand::seq::IndexedRandom;

            let star_pool = star_tile_yaku_pool(
                &breakdown.detected_yaku,
                structure_meta,
                &scoring_tiles,
                &sets,
            );
            if !star_pool.is_empty() {
                let mut rng = rand::rng();
                let prob = if self.relics.has(RelicId::FortunesFavor) {
                    2
                } else {
                    1
                };
                if rng.random_ratio(prob, 4)
                    && let Some(&y) = star_pool.choose(&mut rng)
                {
                    let _new_level = self.yaku_levels.level_up(y);
                    self.push_relic_activation(RelicId::StarTile);
                }
            }
        }
        if breakdown.flower_yen > 0 {
            self.apply_yen_reward(breakdown.flower_yen, Some(bus));
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

        if scored_full_hand {
            let treasures = EffectiveRelics::from_roster(&self.relics)
                .count(&self.relics, RelicId::EightTreasures);
            for _ in 0..treasures {
                use rand::seq::IndexedRandom;

                let mut rng = rand::rng();
                if let Some(&z) = self.zodiac_spawn_pool().choose(&mut rng) {
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

        if breakdown_total > 0 && self.relics.has(RelicId::Kindling) {
            let e = self.relic_counters.entry(RelicId::Kindling).or_insert(0);
            *e = (*e + 1).min(crate::core::relic::KINDLING_STACK_CAP);
        }

        if destroy_glass_cannon {
            let _ = self.destroy_relic_with_activation_fx(RelicId::GlassCannon, Some(bus));
        }

        applied
    }

    fn scoring_tile_debuffs(&self, scoring_tiles: &[Tile]) -> Vec<TileDebuff> {
        let mut debuffs = self.tile_debuffs.clone();
        let dragon_without_honors = self.chamber == ChamberKind::Ordeal
            && self.ordeal.upcoming == Some(OrdealKind::Dragon)
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
        let rw = Some(ChamberKind::round_wind_for_wing(self.wing));
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
        let rw = Some(ChamberKind::round_wind_for_wing(self.wing));
        let bonus_rw = self.bonus_round_wind_for_yaku();
        if !is_winning_structure_shape(&self.structure_tiles, &self.structure_sets)
            && !structure_cannot_grow_further(
                &self.structure_tiles,
                &self.structure_sets,
                HAND_SIZE,
            )
        {
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

    /// HUD refresh after a scoring mutation; does not evaluate chamber end.
    pub(super) fn push_score_updated_event(&mut self, bus: &mut EventBus) {
        if !self.suppress_chamber_resolution {
            bus.push(GameEvent::ScoreUpdated);
        }
    }

    /// Idempotent: enqueue blind clear / run loss when terminal predicates hold.
    pub fn resolve_round_end(&mut self, bus: &mut EventBus) {
        if self.suppress_chamber_resolution || self.round_end_queued || self.discard_refill_pending
        {
            return;
        }
        if self.round_score >= self.target_score as u64 {
            let base_reward = self.chamber.clear_reward();
            let unused_play_bonus = self.plays_remaining;
            let interest = (self.yen.max(0) as u32 / 5).min(3);
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
                let bonus = (self.yen.max(0) as u32 / 4).min(4);
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
                let bonus = self.wing.max(1);
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
            let memorial_clear = self.memorial_round.clear_yen_bonus;
            self.memorial_round.clear_yen_bonus = 0;
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
            let ordeal_name = if self.chamber == ChamberKind::Ordeal {
                self.ordeal.upcoming.map(|b| b.name())
            } else {
                None
            };
            self.chronicle.record_chamber_cleared(
                self.wing,
                self.chamber,
                ordeal_name,
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
            self.round_end_queued = true;
            if self.chamber == ChamberKind::Ordeal
                && let Some(bk) = self.ordeal.upcoming
                && !self.onboarding_active()
            {
                bus.push(GameEvent::OrdealDefeated(bk));
            }
        } else if let Some(reason) = self.round_failure_reason() {
            if self.try_second_wind_salvage(reason, bus) {
                self.round_end_queued = true;
            } else if !self.try_talisman_salvage(reason, bus) {
                bus.push(GameEvent::GameOver { reason });
                self.round_end_queued = true;
            }
        }
    }

    pub(super) fn emit_round_resolution_events(&mut self, bus: &mut EventBus) {
        self.push_score_updated_event(bus);
        self.resolve_round_end(bus);
    }

    /// When a round would end in defeat, consume a memorial talisman that grants
    /// plays or discards so the blind can continue.
    fn try_talisman_salvage(&mut self, reason: GameOverReason, bus: &mut EventBus) -> bool {
        let Some(index) = self.find_salvage_talisman_index(reason) else {
            return false;
        };
        if self.use_consumable(index, bus).is_none() {
            return false;
        }
        if self.round_failure_reason().is_none() {
            bus.push(GameEvent::ScoreUpdated);
        }
        true
    }

    /// When a round would end in defeat, Second Wind is destroyed and the blind
    /// is forfeited (no gold payout); [`RunState::forfeit_current_chamber_second_wind`]
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

    /// Played meld chips in structure (for HUD tiers).
    pub fn structure_played_meld_chips(&self) -> i32 {
        played_meld_chips(&self.structure_tiles, &self.structure_sets)
    }

    /// Whether [`Self::trigger_structure_manual`] can score (structure non-empty and rules allow).
    pub fn can_trigger_structure_now(&self) -> bool {
        if self.structure_sets.is_empty() {
            return false;
        }
        if self.cash_in_blocked_until_discards_spent() {
            return false;
        }
        let rw = Some(ChamberKind::round_wind_for_wing(self.wing));
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
        let rw = Some(ChamberKind::round_wind_for_wing(self.wing));
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
                yen: self.yen,
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

    /// Enumerate every valid meld decomposition of the selection and pick the
    /// one that would score best at cash-in. Full winning submissions score the
    /// selection alone; partial commits score the merged structure (existing
    /// melds plus the candidate split). Ties fall back to hand-shape affinity,
    /// then preview yaku weight, then a canonical decomposition key.
    pub(crate) fn pick_best_decomposition(
        &self,
        default_sets: Vec<DetectedMeld>,
        scoring_tiles: &[Tile],
        original_tiles: &[Tile],
    ) -> Vec<DetectedMeld> {
        // A full hand has 14 tiles plus each kong's excess over a triplet (4→+1, 5→+2).
        let kong_bonus = kong_structure_bonus(default_sets.iter());
        let is_full_hand =
            scoring_tiles.len() >= HAND_SIZE && scoring_tiles.len() == HAND_SIZE + kong_bonus;
        let bias = infer_decomposition_bias(&self.hand);
        let rules = self.validation_rules_for_structure_commits();
        let mut alternatives = enumerate_decompositions(scoring_tiles, &rules);
        alternatives.retain(|sets| self.selection_commit_capacity_ok(sets, scoring_tiles.len()));
        if alternatives.is_empty() {
            return default_sets;
        }
        if alternatives.len() == 1 {
            return alternatives[0].clone();
        }

        let rw = Some(ChamberKind::round_wind_for_wing(self.wing));
        let bonus_rw = self.bonus_round_wind_for_yaku();
        let base_set_len = self.structure_sets.len();
        let mut merged_sets = self.structure_sets.clone();
        let mut merged_tiles = self.structure_tiles.clone();
        if !is_full_hand {
            merged_tiles.extend(scoring_tiles.iter().copied());
        }
        let score_tile_debuffs = if is_full_hand {
            self.scoring_tile_debuffs(scoring_tiles)
        } else {
            self.scoring_tile_debuffs(&merged_tiles)
        };

        let preview_yaku_weight = |sets: &[DetectedMeld]| -> i64 {
            let yaku = if is_full_hand {
                yaku_after_pool_filter(
                    scoring_tiles,
                    sets,
                    rw,
                    bonus_rw,
                    Some(original_tiles),
                    &self.available_yaku,
                )
            } else {
                let mut merged = self.structure_sets.clone();
                merged.extend(sets.iter().cloned());
                yaku_after_pool_filter(
                    &merged_tiles,
                    &merged,
                    rw,
                    bonus_rw,
                    Some(original_tiles),
                    &self.available_yaku,
                )
            };
            preview_yaku_bundle_weight(&yaku)
        };

        let mut score_decomp = |sets: &[DetectedMeld]| -> u64 {
            let (tiles, structure_meta) = if is_full_hand {
                (scoring_tiles, None)
            } else {
                merged_sets.truncate(base_set_len);
                merged_sets.extend(sets.iter().cloned());
                let meta = StructureTriggerMeta {
                    meld_count: merged_sets.len() as u32,
                    inject_chicken_if_no_yaku: true,
                };
                (merged_tiles.as_slice(), Some(meta))
            };
            let ctx = ScoreContext {
                relic: ScoreRelicBundle {
                    roster: &self.relics,
                    counters: self.relic_counters.clone(),
                },
                tiles: ScoreTileBundle {
                    debuffs: &score_tile_debuffs,
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
                    yen: self.yen,
                    total_score: self.total_score_earned,
                },
                structure: structure_meta,
            };
            let sets_for_score = if is_full_hand {
                sets
            } else {
                merged_sets.as_slice()
            };
            score_sets_with_original(
                tiles,
                sets_for_score,
                &ctx,
                &self.round_rules,
                original_tiles,
            )
            .total
        };

        let mut best = default_sets;
        let mut best_total = score_decomp(&best);
        let mut best_affinity = decomposition_affinity(&best, bias);
        let mut best_yaku_weight = preview_yaku_weight(&best);
        let mut best_key = decomposition_canonical_key(scoring_tiles, &best);
        for candidate in alternatives {
            let total = score_decomp(&candidate);
            let affinity = decomposition_affinity(&candidate, bias);
            let yaku_weight = preview_yaku_weight(&candidate);
            let key = decomposition_canonical_key(scoring_tiles, &candidate);
            let take = total > best_total
                || (total == best_total && affinity > best_affinity)
                || (total == best_total
                    && affinity == best_affinity
                    && yaku_weight > best_yaku_weight)
                || (total == best_total
                    && affinity == best_affinity
                    && yaku_weight == best_yaku_weight
                    && key < best_key);
            if take {
                best_total = total;
                best_affinity = affinity;
                best_yaku_weight = yaku_weight;
                best_key = key;
                best = candidate;
            }
        }
        best
    }

    /// Meld split and tile lists for in-play yaku tablets. Uses committed
    /// structure plus [`Self::pick_best_decomposition`] on the current
    /// selection — the same path as play commit and manual cash-in.
    pub(crate) fn melds_for_yaku_preview(
        &self,
        selected_tiles: &[Tile],
    ) -> (Vec<DetectedMeld>, Vec<Tile>, Vec<Tile>) {
        let mut sets = self.structure_sets.clone();
        let structure_tiles = self.structure_tiles.clone();

        if selected_tiles.is_empty() {
            return (sets, structure_tiles.clone(), structure_tiles);
        }

        let Some((selected_sets, scoring_tiles)) = self.try_validate_with_wildcards(selected_tiles)
        else {
            return (sets, structure_tiles.clone(), structure_tiles);
        };

        let best_sets = self.pick_best_decomposition(selected_sets, &scoring_tiles, selected_tiles);

        let mut original = structure_tiles.clone();
        original.extend(selected_tiles.iter().copied());

        let mut effective = structure_tiles;
        effective.extend(scoring_tiles.iter().copied());

        sets.extend(best_sets);
        (sets, effective, original)
    }

    /// Rules used when validating a selection before committing it to the structure
    /// (differs from [`RunState::round_rules`] e.g. honor-gated tutorial modifiers).
    pub fn validation_rules_for_structure_commits(&self) -> Vec<RuleModifier> {
        let mut rules: Vec<RuleModifier> = self
            .round_rules
            .iter()
            .copied()
            .filter(|rule| *rule != RuleModifier::RequireHonor)
            .collect();
        if self.relics.has(RelicId::KingKong) {
            rules.push(RuleModifier::FiveTileKong);
        }
        rules
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
            let best_sets =
                self.pick_best_decomposition(new_sets, &scoring_tiles, &tiles);
            if !self.selection_commit_capacity_ok(&best_sets, scoring_tiles.len()) {
                continue;
            }
            return true;
        }
        false
    }

    fn no_actions_remaining(&self) -> bool {
        if self.discard_refill_pending {
            return false;
        }
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

fn preview_yaku_bundle_weight(yaku: &[YakuKind]) -> i64 {
    yaku
        .iter()
        .map(|y| y.chip_bonus() as i64 + (y.mult_bonus() * 100.0).round() as i64)
        .sum()
}
