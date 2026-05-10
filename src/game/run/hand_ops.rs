use super::*;

impl RunState {
    /// Try validating tiles, applying JokerTile / WildWinds substitutions if needed.
    /// Returns the decomposition and the (possibly modified) tiles used for scoring.
    pub fn try_validate_with_wildcards(
        &self,
        tiles: &[Tile],
    ) -> Option<(Vec<DetectedSet>, Vec<Tile>)> {
        let validation_rules = self.validation_rules_for_structure_commits();
        // Try standard validation first.
        if let Some(sets) = validate_selection_with_rules(tiles, &validation_rules) {
            return Some((sets, tiles.to_vec()));
        }

        // JokerTile: try substituting one tile with each possible face.
        if self.relics.has(RelicId::JokerTile)
            && !self.joker_used
            && let Some(result) = try_joker_substitution(tiles, &validation_rules)
        {
            return Some(result);
        }

        // Disgust: relabel West tiles as East so EW / EWW / EWWW validate as
        // pair / triplet / kong. Runs before WildWinds because it is strictly
        // narrower (only fires with E+W present) and can chain into WildWinds
        // for any *other* wind tiles in the selection.
        if self.relics.has(RelicId::Disgust) {
            let chain_winds = self.relics.has(RelicId::WildWinds);
            if let Some(result) = try_disgust_substitution(tiles, &validation_rules, chain_winds) {
                return Some(result);
            }
        }

        // WildWinds: try substituting wind tiles.
        if self.relics.has(RelicId::WildWinds)
            && let Some(result) = try_wind_substitution(tiles, &validation_rules)
        {
            return Some(result);
        }

        None
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

    /// Check if the current selection forms a valid playable hand.
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
        self.try_validate_with_wildcards(&selected_tiles).is_some()
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
    /// `refill_hand` once the discard departure animation has had time to play.
    /// Returns the number of tiles removed, or 0 if nothing was selected or no
    /// discards remain.
    pub fn discard_selected_no_refill(&mut self, bus: &mut EventBus) -> usize {
        use crate::game::engine_state::GameplayCoreState;

        if !self.tutorial_discard_allowed() {
            return 0;
        }

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
        let honor_gold = if self.relics.has(RelicId::NoHonorButWealth) {
            selected_indices
                .iter()
                .filter_map(|&i| self.hand.get(i))
                .filter(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
                .count() as i32
        } else {
            0
        };

        let removed = GameplayCoreState::with_run_mut(self, |core| core.discard_selected());
        let count = removed.len();
        if count == 0 {
            return 0;
        }

        if honor_gold > 0 {
            self.gold = self.gold.saturating_add(honor_gold);
            self.relic_activations.push(RelicId::NoHonorButWealth);
        }
        for _ in &selected_indices {
            bus.push(GameEvent::TileDiscarded);
        }
        self.tiles_discarded = self.tiles_discarded.saturating_add(count as u32);

        if let Some(ref mut tut) = self.tutorial
            && tut.celebrate(crate::game::tutorial::TutorialMilestone::FirstDiscard)
        {
            bus.push(GameEvent::TutorialMilestone(
                crate::game::tutorial::TutorialMilestone::FirstDiscard,
            ));
        }

        if self.relics.has(RelicId::SilkThread) {
            self.relic_activations.push(RelicId::SilkThread);
            let v = self.relic_counters.entry(RelicId::SilkThread).or_insert(40);
            *v = (*v - 3).max(0);
            if *v == 0 {
                self.relic_counters.remove(&RelicId::SilkThread);
                self.relics.active.retain(|&r| r != RelicId::SilkThread);
                self.silk_thread_extinct = true;
                self.note_relic_destroyed();
                bus.push(GameEvent::TransformationSuccessorDiscovered(RelicId::SilkMoth));
                bus.push(GameEvent::AchievementUnlocked(
                    crate::steam::Achievement::SilkMothEmerged,
                ));
            }
        }

        // Silk Moth: produce $1 per discard action and accumulate the lifetime
        // total in `relic_counters[SilkMoth]` so the live tooltip can show it.
        if self.relics.has(RelicId::SilkMoth) {
            self.gold = self.gold.saturating_add(1);
            *self.relic_counters.entry(RelicId::SilkMoth).or_insert(0) += 1;
            self.relic_activations.push(RelicId::SilkMoth);
        }

        count
    }

    /// Draw tiles from the wall until the hand is full, then sort and reset
    /// the selection vector to match the new hand size. Honors boss-induced
    /// hand-size shrinks (e.g. The Whisper).
    pub fn refill_hand(&mut self, bus: &mut EventBus) {
        use crate::game::engine_state::GameplayCoreState;

        let target = boss::effective_hand_size(self);
        let lotus = self.relics.has(RelicId::LotusBloom);
        let mut drawn: Vec<Tile> = Vec::new();
        while self.hand.len() + drawn.len() < target {
            let Some(t) = self.wall.draw() else { break };
            if lotus && t.suit == Suit::Flower {
                *self.relic_counters.entry(RelicId::LotusBloom).or_insert(0) += 1;
                self.relic_activations.push(RelicId::LotusBloom);
            }
            bus.push(GameEvent::TileDrawn);
            drawn.push(t);
        }

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

        self.seed_tutorial_hand();
        self.restamp_hand_enhancements();
        self.try_autotrigger_structure_full(bus);
        self.emit_round_resolution_events(bus);
    }
}
