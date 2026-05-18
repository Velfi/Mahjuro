use super::*;

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

        let item = self.consumables.take(index)?;
        match item {
            Consumable::Zodiac(z) => {
                let yaku = z.yaku();
                let new_level = self.yaku_levels.level_up(yaku);
                Some(ConsumableUseResult::Zodiac { yaku, new_level })
            }
            Consumable::Talisman(t) => {
                if let Some(enh) = t.enhancement() {
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
                } else {
                    self.apply_talisman_transform(t);
                }
                bus.push(GameEvent::TalismanUsed(t));
                Some(ConsumableUseResult::Talisman { kind: t })
            }
        }
    }

    /// Rewrite every tile in the current hand per a transform talisman.
    fn apply_talisman_transform(&mut self, kind: crate::core::talisman::TalismanKind) {
        use crate::core::talisman::TalismanKind;
        use crate::core::tile::Suit;
        use rand::RngExt;

        if self.hand.is_empty() {
            return;
        }

        match kind {
            TalismanKind::Bamboo | TalismanKind::Dots | TalismanKind::Characters => {
                let target = match kind {
                    TalismanKind::Bamboo => Suit::Bamboos,
                    TalismanKind::Dots => Suit::Dots,
                    TalismanKind::Characters => Suit::Characters,
                    _ => unreachable!(),
                };
                for tile in &mut self.hand {
                    if tile.is_number_tile() {
                        tile.suit = target;
                    }
                }
            }
            TalismanKind::Honors => {
                let mut rng = rand::rng();
                let honor_suits = [Suit::Wind, Suit::Dragon];
                for tile in &mut self.hand {
                    if !tile.is_number_tile() {
                        continue;
                    }
                    let suit = honor_suits[rng.random_range(0..honor_suits.len())];
                    tile.suit = suit;
                    tile.rank = if suit == Suit::Wind {
                        rng.random_range(1..=4)
                    } else {
                        rng.random_range(1..=3)
                    };
                }
            }
            TalismanKind::Wildflower => {
                let mut rng = rand::rng();
                for tile in &mut self.hand {
                    tile.suit = Suit::Flower;
                    tile.rank = rng.random_range(1..=4);
                }
            }
            TalismanKind::Conformity => {
                let mut rng = rand::rng();
                let template = self.hand[rng.random_range(0..self.hand.len())];
                for tile in &mut self.hand {
                    tile.suit = template.suit;
                    tile.rank = template.rank;
                }
            }
            TalismanKind::Pearl | TalismanKind::Gilded | TalismanKind::Polychrome => {
                unreachable!("buff talisman path");
            }
        }

        self.clear_hand_selection_flags();
        self.hand.sort();
        self.restamp_hand_enhancements();
    }

    fn clear_hand_selection_flags(&mut self) {
        for s in &mut self.selected {
            *s = false;
        }
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
