use crate::{
    core::{
        hand::{DetectedMeld, validate_selection_with_rules},
        ordeal,
        relic::RelicId,
        rules::RuleModifier,
        scoring::EffectiveRelics,
        tile::{Suit, Tile},
    },
    game::{
        event_bus::{EventBus, GameEvent},
        run::{RunState, try_disgust_substitution, try_wind_substitution},
    },
};

use super::relic_removal::TransformationPrimaryRelic;

impl RunState {
    /// Try validating tiles, applying WildWinds substitutions if needed.
    /// Returns the decomposition and the (possibly modified) tiles used for scoring.
    pub fn try_validate_with_wildcards(
        &self,
        tiles: &[Tile],
    ) -> Option<(Vec<DetectedMeld>, Vec<Tile>)> {
        self.try_validate_with_wildcards_and_rules(
            tiles,
            &self.validation_rules_for_structure_commits(),
        )
    }

    /// Like [`Self::try_validate_with_wildcards`], but against an explicit rule set.
    pub fn try_validate_with_wildcards_and_rules(
        &self,
        tiles: &[Tile],
        validation_rules: &[RuleModifier],
    ) -> Option<(Vec<DetectedMeld>, Vec<Tile>)> {
        // Try standard validation first.
        if let Some(sets) = validate_selection_with_rules(tiles, validation_rules) {
            return Some((sets, tiles.to_vec()));
        }

        // Disgust: relabel West tiles as East so EW / EWW / EWWW validate as
        // pair / triplet / kong. Runs before WildWinds because it is strictly
        // narrower (only fires with E+W present) and can chain into WildWinds
        // for any *other* wind tiles in the selection.
        if self.relics.has(RelicId::Disgust) {
            let chain_winds = self.relics.has(RelicId::WildWinds);
            if let Some(result) = try_disgust_substitution(tiles, validation_rules, chain_winds) {
                return Some(result);
            }
        }

        // WildWinds: try substituting wind tiles.
        if self.relics.has(RelicId::WildWinds)
            && let Some(result) = try_wind_substitution(tiles, validation_rules)
        {
            return Some(result);
        }

        None
    }

    /// True when the selection is invalid under the active round rules but would
    /// validate if this boss blind's [`RuleModifier`] pushes were removed.
    pub fn selection_blocked_by_ordeal_rules(&self, tiles: &[Tile]) -> bool {
        let ordeal_rules: &[RuleModifier] = self
            .ordeal
            .effect
            .as_ref()
            .map(|e| e.rule_pushes.as_slice())
            .unwrap_or(&[]);
        if ordeal_rules.is_empty() || tiles.is_empty() {
            return false;
        }
        if self.try_validate_with_wildcards(tiles).is_some() {
            return false;
        }
        let without_ordeal: Vec<RuleModifier> = self
            .validation_rules_for_structure_commits()
            .into_iter()
            .filter(|r| !ordeal_rules.contains(r))
            .collect();
        self.try_validate_with_wildcards_and_rules(tiles, &without_ordeal)
            .is_some()
    }

    /// Meld decomposition for the staging-zone preview. Uses the same validation
    /// and decomposition pick as [`Self::commit_selection_to_structure`].
    pub fn preview_selection_melds(&self, tiles: &[Tile]) -> (Vec<DetectedMeld>, Vec<u32>, bool) {
        if tiles.is_empty() {
            return (Vec::new(), Vec::new(), false);
        }
        if let Some((sets, scoring_tiles)) = self.try_validate_with_wildcards(tiles) {
            let sets = self.pick_best_decomposition(sets, &scoring_tiles, tiles);
            return (sets, Vec::new(), true);
        }
        let rules = self.validation_rules_for_structure_commits();
        let (melds, bad) = crate::core::hand::staging_preview_melds(tiles, &rules);
        (melds, bad, false)
    }

    /// Toggle whether a hand tile is marked for discard.
    pub fn toggle_select(&mut self, index: usize) {
        if index < self.selected.len() {
            self.selected[index] = !self.selected[index];
        }
    }

    /// Clear all selections.
    pub fn clear_selection(&mut self) {
        self.selected.iter_mut().for_each(|s| *s = false);
    }

    /// How many tiles are currently selected for discard.
    pub fn selected_count(&self) -> usize {
        self.selected.iter().filter(|&&s| s).count()
    }

    /// True when the current hand selection is invalid solely because of this
    /// boss blind's rule modifiers (see [`Self::selection_blocked_by_ordeal_rules`]).
    pub fn hand_selection_blocked_by_boss(&self) -> bool {
        use crate::core::rules::ChamberKind;
        if self.chamber != ChamberKind::Ordeal || self.selected_count() == 0 {
            return false;
        }
        let selected_tiles: Vec<Tile> = self
            .hand
            .iter()
            .zip(self.selected.iter())
            .filter(|&(_, &sel)| sel)
            .map(|(t, _)| *t)
            .collect();
        !self.is_selection_valid() && self.selection_blocked_by_ordeal_rules(&selected_tiles)
    }

    /// Check if the current selection can be played into structure right now.
    pub fn is_selection_valid(&self) -> bool {
        if self.selected_count() == 0 {
            return false;
        }
        let selected_tiles: Vec<Tile> = self
            .hand
            .iter()
            .zip(self.selected.iter())
            .filter(|&(_, &sel)| sel)
            .map(|(t, _)| *t)
            .collect();
        let Some((sets, scoring_tiles)) = self.try_validate_with_wildcards(&selected_tiles) else {
            return false;
        };
        let sets = self.pick_best_decomposition(sets, &scoring_tiles, &selected_tiles);
        use crate::core::hand::kong_structure_bonus;
        use crate::game::game_mode::HAND_SIZE;
        let kongs_after =
            kong_structure_bonus(self.structure_sets.iter().chain(sets.iter()));
        self.structure_tiles.len() + scoring_tiles.len() <= HAND_SIZE + kongs_after
    }

    /// Player-facing copy for the current selection when Play is rejected.
    pub fn rejected_play_hint(&self) -> Option<String> {
        let selected_tiles: Vec<Tile> = self
            .hand
            .iter()
            .zip(self.selected.iter())
            .filter(|&(_, &sel)| sel)
            .map(|(t, _)| *t)
            .collect();
        if selected_tiles.is_empty() {
            return None;
        }
        if self.is_selection_valid() {
            return None;
        }
        let rules = self.validation_rules_for_structure_commits();
        if self.try_validate_with_wildcards(&selected_tiles).is_none() {
            return Some(crate::core::hand::selection_rejection_hint(
                &selected_tiles,
                &rules,
            ));
        }
        self.play_rejection_callout().map(str::to_string)
    }

    /// Floating callout when Play is rejected for structure capacity (not bad melds).
    pub fn play_rejection_callout(&self) -> Option<&'static str> {
        let selected_tiles: Vec<Tile> = self
            .hand
            .iter()
            .zip(self.selected.iter())
            .filter(|&(_, &sel)| sel)
            .map(|(t, _)| *t)
            .collect();
        if selected_tiles.is_empty() || self.is_selection_valid() {
            return None;
        }
        if self.try_validate_with_wildcards(&selected_tiles).is_none() {
            return None;
        }
        use crate::core::hand::kong_structure_bonus;
        use crate::game::game_mode::HAND_SIZE;
        let kongs_now = kong_structure_bonus(self.structure_sets.iter());
        let capacity = HAND_SIZE + kongs_now;
        if self.structure_tiles.len() >= capacity {
            Some("It's already full")
        } else {
            Some("Too many melds")
        }
    }

    /// Discard all selected tiles (costs 1 discard), then auto-draw back to HAND_SIZE.
    /// Returns the number of tiles discarded, or 0 if nothing was selected or no discards left.
    pub fn discard_selected(&mut self, bus: &mut EventBus) -> usize {
        let count = self.discard_selected_no_refill(bus);
        if count > 0 {
            self.refill_hand(bus);
        }
        count
    }

    /// Remove all selected tiles and decrement the discard counter, but do NOT
    /// auto-draw replacements. The caller is responsible for invoking
    /// [`Self::refill_hand`] once the discard river animation has finished.
    ///
    /// This is the **only** hand mutation that splits removal from redraw; plays
    /// commit and refill atomically in [`super::scoring_flow::RunState::commit_selection_to_structure`].
    /// Sets [`super::RunState::discard_refill_pending`] until refill completes.
    /// Returns the number of tiles removed, or 0 if nothing was selected or no
    /// discards remain.
    pub fn discard_selected_no_refill(&mut self, bus: &mut EventBus) -> usize {
        use crate::game::engine_state::GameplayCoreState;

        // Snapshot selected indices and honor count *before* removal so the
        // engine-owned mutation can return tiles without losing slot info.
        let selected_indices: Vec<usize> = self
            .selected
            .iter()
            .enumerate()
            .filter(|(_, s)| **s)
            .map(|(i, _)| i)
            .collect();
        if selected_indices.is_empty() || self.discards_remaining == 0 {
            return 0;
        }
        let eff = EffectiveRelics::from_roster(&self.relics);
        let honor_yen = if eff.has(&self.relics, RelicId::NoHonorButWealth) {
            let base = selected_indices
                .iter()
                .filter_map(|&i| self.hand.get(i))
                .filter(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
                .count() as i32;
            base.saturating_mul(eff.count(&self.relics, RelicId::NoHonorButWealth) as i32)
        } else {
            0
        };

        let removed = GameplayCoreState::with_run_mut(self, |core| core.discard_selected());
        let count = removed.len();
        if count == 0 {
            return 0;
        }

        if honor_yen > 0 {
            self.apply_yen_reward(honor_yen, Some(bus));
            self.push_relic_activation(RelicId::NoHonorButWealth);
        }
        for tile in &removed {
            self.chronicle.note_discarded_tile(tile);
            bus.push(GameEvent::TileDiscarded);
        }
        self.chronicle.note_turn();
        self.tiles_discarded = self.tiles_discarded.saturating_add(count as u32);

        if self.relics.has(RelicId::SilkThread) {
            self.push_relic_activation(RelicId::SilkThread);
            let v = self.relic_counters.entry(RelicId::SilkThread).or_insert(40);
            *v = (*v - 3).max(0);
            if *v == 0 {
                self.on_transformation_primary_burned(TransformationPrimaryRelic::SilkThread, bus);
            }
        }

        // Silk Moth: produce ¥1 yen per discard action and accumulate the lifetime
        // total in `relic_counters[SilkMoth]` so the live tooltip can show it.
        if self.relics.has(RelicId::SilkMoth) {
            self.apply_yen_reward(1, Some(bus));
            *self.relic_counters.entry(RelicId::SilkMoth).or_insert(0) += 1;
            self.push_relic_activation(RelicId::SilkMoth);
        }

        self.onboarding_notify_discard();
        self.discard_refill_pending = true;
        count
    }

    /// Draw tiles from the wall until the hand is full, then sort and reset
    /// the selection vector to match the new hand size. Honors boss-induced
    /// hand-size shrinks (e.g. The Whisper).
    pub fn refill_hand(&mut self, bus: &mut EventBus) {
        use crate::game::engine_state::GameplayCoreState;

        self.discard_refill_pending = false;
        let target = ordeal::effective_hand_size(self);
        let lotus = self.relics.has(RelicId::LotusBloom);
        let mut drawn: Vec<Tile> = Vec::new();
        while self.hand.len() + drawn.len() < target {
            let Some(t) = self.wall.draw() else { break };
            if lotus && t.suit == Suit::Flower {
                *self.relic_counters.entry(RelicId::LotusBloom).or_insert(0) += 1;
                self.push_relic_activation(RelicId::LotusBloom);
            }
            bus.push(GameEvent::TileDrawn);
            drawn.push(t);
        }

        self.chronicle
            .note_tiles_drawn(drawn.len().min(u32::MAX as usize) as u32);
        GameplayCoreState::with_run_mut(self, |core| {
            core.push_drawn_tiles(&drawn);
        });
        let restocked = !drawn.is_empty();

        if restocked {
            self.times_restocked = self.times_restocked.saturating_add(1);
        }
        self.set_magnet_draw_fourths(bus);

        GameplayCoreState::with_run_mut(self, |core| {
            core.finalize_hand_after_draw();
        });

        self.restamp_hand_enhancements();
        self.try_autotrigger_structure_full(bus);
        self.emit_round_resolution_events(bus);
    }
}
