use super::*;

use crate::game::engine_state::GameplayCoreState;

impl RunState {
    /// Use a consumable from the shared inventory at `index`. Zodiacs level
    /// their yaku for the run; Talismans stamp their enhancement onto every
    /// tile currently in the player's hand. Returns a [`ConsumableUseResult`]
    /// describing what happened so the UI can log/animate appropriately.
    pub fn use_consumable(
        &mut self,
        index: usize,
        bus: &mut crate::game::event_bus::EventBus,
    ) -> Option<ConsumableUseResult> {
        use crate::core::consumable::Consumable;

        let pending = self.consumables.items.get(index).copied()?;
        if let Consumable::Talisman(t) = pending
            && t.acts_on_selection()
            && !self.selected.iter().any(|&s| s)
        {
            return None;
        }
        let item = self.consumables.take(index)?;
        match item {
            Consumable::Zodiac(z) => {
                let yaku = z.yaku();
                let new_level = self.yaku_levels.level_up(yaku);
                Some(ConsumableUseResult::Zodiac { yaku, new_level })
            }
            Consumable::Talisman(t) => {
                if t.acts_on_selection() {
                    self.apply_talisman_to_selection(t, bus);
                } else {
                    let enh = t.enhancement().expect("buff talisman has enhancement");
                    // Record the enhancement against each current hand tile's id
                    // so it persists when those tiles get redrawn next round.
                    for tile in &self.hand {
                        self.tile_enhancements.insert(tile.id, enh);
                    }
                    // Brocade Pouch: also mark the run's global buff so every
                    // tile drawn from now on (not just the 14 in hand) carries
                    // this enhancement. Latest buff talisman wins, matching
                    // the "most-recent wins" rule for per-tile stamps.
                    if self.relics.has(RelicId::BrocadePouch) {
                        self.global_buff_enhancement = Some(enh);
                    }
                    crate::core::talisman::apply_to_hand(&mut self.hand, t);
                }
                bus.push(GameEvent::TalismanUsed(t));
                Some(ConsumableUseResult::Talisman { kind: t })
            }
        }
    }

    /// Apply a selection-targeting talisman to currently selected hand tiles.
    /// Clears selection and re-sorts the hand. Caller must ensure at least one
    /// tile is selected.
    fn apply_talisman_to_selection(
        &mut self,
        kind: crate::core::talisman::TalismanKind,
        bus: &mut crate::game::event_bus::EventBus,
    ) {
        use crate::core::talisman::TalismanKind;
        use crate::core::tile::Suit;
        use rand::RngExt;

        match kind {
            TalismanKind::Kiln => {
                let _ = self.destroy_selected_tiles(bus);
            }
            TalismanKind::Bamboo | TalismanKind::Dots | TalismanKind::Characters => {
                let target = match kind {
                    TalismanKind::Bamboo => Suit::Bamboos,
                    TalismanKind::Dots => Suit::Circles,
                    TalismanKind::Characters => Suit::Characters,
                    _ => unreachable!(),
                };
                for i in 0..self.hand.len() {
                    if !self.selected.get(i).copied().unwrap_or(false) {
                        continue;
                    }
                    let tile = &mut self.hand[i];
                    if tile.is_number_tile() {
                        tile.suit = target;
                    }
                }
                self.clear_hand_selection_flags();
                self.hand.sort();
                self.restamp_hand_enhancements();
            }
            TalismanKind::Honors => {
                let mut rng = rand::rng();
                for i in 0..self.hand.len() {
                    if !self.selected.get(i).copied().unwrap_or(false) {
                        continue;
                    }
                    let tile = &mut self.hand[i];
                    if !tile.is_number_tile() {
                        continue;
                    }
                    let honor_suits = [Suit::Wind, Suit::Dragon];
                    let suit = honor_suits[rng.random_range(0..honor_suits.len())];
                    let rank: u8 = if suit == Suit::Wind {
                        rng.random_range(1..=4)
                    } else {
                        rng.random_range(1..=3)
                    };
                    tile.suit = suit;
                    tile.rank = rank;
                }
                self.clear_hand_selection_flags();
                self.hand.sort();
                self.restamp_hand_enhancements();
            }
            TalismanKind::Wildflower => {
                let mut rng = rand::rng();
                for i in 0..self.hand.len() {
                    if !self.selected.get(i).copied().unwrap_or(false) {
                        continue;
                    }
                    let tile = &mut self.hand[i];
                    tile.suit = Suit::Flower;
                    tile.rank = rng.random_range(1..=4);
                }
                self.clear_hand_selection_flags();
                self.hand.sort();
                self.restamp_hand_enhancements();
            }
            TalismanKind::Conformity => {
                let selected_indices: Vec<usize> = (0..self.hand.len())
                    .filter(|&i| self.selected.get(i).copied().unwrap_or(false))
                    .collect();
                if selected_indices.is_empty() {
                    return;
                }
                if self.hand.is_empty() {
                    return;
                }
                let mut rng = rand::rng();
                let template_idx = rng.random_range(0..self.hand.len());
                let template = self.hand[template_idx];
                for i in selected_indices {
                    self.hand[i].suit = template.suit;
                    self.hand[i].rank = template.rank;
                }
                self.clear_hand_selection_flags();
                self.hand.sort();
                self.restamp_hand_enhancements();
            }
            TalismanKind::Pearl
            | TalismanKind::Gilded
            | TalismanKind::Polychrome => {
                unreachable!("selection-only path");
            }
        }
    }

    fn clear_hand_selection_flags(&mut self) {
        for s in &mut self.selected {
            *s = false;
        }
    }

    /// Maximum number of tiles that can be removed from the wall via the Kiln.
    /// The wall needs enough tiles to deal a full hand each round.
    const MAX_REMOVED_TILES: usize = 56;

    /// Canonical *tile destroyed* path (Kiln talisman).
    ///
    /// The "destroyed" keyword is the
    /// player-facing name for permanent removal of a tile from a run; for
    /// tiles, that means writing the tile id into `removed_tile_ids` so the
    /// per-round `Wall::from_filtered_with_packs` rebuild excludes it for
    /// the rest of the run. New tile-destruction effects (Taotie's devour
    /// is the other current site, in `scoring_flow.rs`) should follow the
    /// same primitive: insert into `removed_tile_ids`, drop the tile's
    /// enhancement, and emit `GameEvent::TilesDestroyed`. Round-only effects
    /// (e.g. Tempest's wall burn) are *not* destruction — they just draw off
    /// this round's wall and leave `removed_tile_ids` untouched.
    pub fn destroy_selected_tiles(&mut self, bus: &mut crate::game::event_bus::EventBus) -> usize {
        let budget = Self::MAX_REMOVED_TILES.saturating_sub(self.removed_tile_ids.len());
        let mut destroyed = 0usize;
        let mut kept_hand = Vec::new();
        let mut kept_sel = Vec::new();
        for (i, tile) in self.hand.iter().enumerate() {
            if self.selected[i] && destroyed < budget {
                self.removed_tile_ids.insert(tile.id);
                self.tile_enhancements.remove(&tile.id);
                destroyed += 1;
            } else {
                kept_hand.push(*tile);
                kept_sel.push(false);
            }
        }
        self.hand = kept_hand;
        self.selected = kept_sel;
        // Refill hand from the wall.
        while self.hand.len() < crate::core::boss::effective_hand_size(self) {
            if let Some(t) = self.wall.draw() {
                self.hand.push(t);
                self.selected.push(false);
            } else {
                break;
            }
        }
        GameplayCoreState::with_run_mut(self, |core| {
            core.finalize_hand_after_draw();
        });
        self.restamp_hand_enhancements();
        if destroyed > 0 {
            bus.push(crate::game::event_bus::GameEvent::TilesDestroyed);
        }
        destroyed
    }

    /// Re-stamp every tile in the current hand with whatever enhancement is
    /// stored against its id in `tile_enhancements`. Called after any path
    /// that adds tiles to the hand (initial deal, post-play refill, mid-round
    /// draws, new-round redeal) so talisman effects survive for the whole run.
    pub(super) fn restamp_hand_enhancements(&mut self) {
        if self.tile_enhancements.is_empty() && self.global_buff_enhancement.is_none() {
            return;
        }
        for tile in &mut self.hand {
            if let Some(&enh) = self.tile_enhancements.get(&tile.id) {
                tile.enhancement = Some(enh);
            } else if let Some(enh) = self.global_buff_enhancement {
                tile.enhancement = Some(enh);
            }
        }
    }

    /// Recompute consumable inventory capacity from
    /// currently-owned relics. Idempotent — call after any relic add/remove.
    /// The inventory is shared between Zodiacs and Talismans; the base
    /// capacity comes from `GameMode::consumable_capacity` (default 2).
    pub fn recompute_capacities(&mut self) {
        let mut consumable_cap = self.mode.consumable_capacity;
        if self.relics.has(RelicId::BrocadePouch) {
            consumable_cap += 1;
        }
        self.consumables.capacity = consumable_cap;
    }
}
