use super::*;
use crate::core::consumable::Consumable;
use crate::core::memorial_talisman::MemorialTalismanKind;
use crate::game::engine_state::GameplayCoreState;
use crate::game::event_bus::GameOverReason;

impl RunState {
    /// Index of the best memorial talisman in inventory that can avert `reason`.
    pub fn find_salvage_talisman_index(&self, reason: GameOverReason) -> Option<usize> {
        for &kind in MemorialTalismanKind::salvage_candidates(reason) {
            if let Some(index) = self.consumables.items.iter().position(|c| {
                matches!(c, Consumable::Memorial(k) if *k == kind)
            }) {
                return Some(index);
            }
        }
        None
    }

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
        self.chronicle.note_consumable_used(item);
        match item {
            Consumable::Zodiac(z) => {
                self.defeat_journal.zodiac_uses = self.defeat_journal.zodiac_uses.saturating_add(1);
                let yaku = z.yaku();
                let new_level = self.yaku_levels.level_up_for_zodiac(z);
                Some(ConsumableUseResult::Zodiac { yaku, new_level })
            }
            Consumable::Talisman(t) => {
                self.defeat_journal.record_talisman_use(t);
                if let Some(enh) = t.enhancement() {
                    for tile in &self.hand {
                        self.tile_enhancements.insert(tile.id, enh);
                    }
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
            Consumable::Memorial(kind) => {
                self.apply_memorial_talisman(kind, bus);
                bus.push(GameEvent::MemorialTalismanUsed(kind));
                Some(ConsumableUseResult::Memorial { kind })
            }
        }
    }

    fn apply_memorial_talisman(
        &mut self,
        kind: crate::core::memorial_talisman::MemorialTalismanKind,
        bus: &mut EventBus,
    ) {
        use crate::core::memorial_talisman::{
            MemorialTalismanKind, buff_saint_enhancement, transformer_target_suit,
        };

        let snapshot = self.memorial_snapshot.as_ref();
        let journal_discards = snapshot.map(|s| s.tiles_discarded).unwrap_or(0);

        match kind {
            MemorialTalismanKind::Exhausted => {
                self.plays_remaining = self.plays_remaining.saturating_add(2);
                self.sync_round_resource_caps();
            }
            MemorialTalismanKind::FrozenHand => {
                self.discards_remaining = self.discards_remaining.saturating_add(1);
                self.sync_round_resource_caps();
                self.hand.clear();
                self.selected.clear();
                self.refill_hand(bus);
            }
            MemorialTalismanKind::Skipper => {
                let bonus =
                    4u32.saturating_add(snapshot.map(|s| s.journal.chambers_skipped).unwrap_or(0));
                self.memorial_round.clear_gold_bonus = bonus.min(12);
            }
            MemorialTalismanKind::Hoarder => {
                self.apply_gold_reward(
                    MemorialTalismanKind::HOARDER_GOLD as i32,
                    Some(bus),
                );
            }
            MemorialTalismanKind::FullDish => {
                self.discards_remaining = self.discards_remaining.saturating_add(1);
                self.sync_round_resource_caps();
            }
            MemorialTalismanKind::Discarded => {
                let extra = (journal_discards / 10).clamp(1, 3);
                self.discards_remaining = self.discards_remaining.saturating_add(extra);
                self.sync_round_resource_caps();
            }
            MemorialTalismanKind::BossMark => {
                self.plays_remaining = self.plays_remaining.saturating_add(1);
                self.sync_round_resource_caps();
            }
            MemorialTalismanKind::BuffSaint => {
                let enh = snapshot
                    .map(buff_saint_enhancement)
                    .unwrap_or(crate::core::tile::TileEnhancement::Pearl);
                for tile in &mut self.hand {
                    self.tile_enhancements.insert(tile.id, enh);
                    tile.enhancement = Some(enh);
                }
            }
            MemorialTalismanKind::Transformer => {
                let suit = snapshot
                    .map(transformer_target_suit)
                    .unwrap_or(crate::core::tile::Suit::Souzu);
                for tile in &mut self.hand {
                    if tile.is_number_tile() {
                        tile.suit = suit;
                    }
                }
                self.clear_hand_selection_flags();
                GameplayCoreState::with_run_mut(self, |core| core.finalize_hand_after_draw());
            }
            MemorialTalismanKind::TagBearer => {
                self.plays_remaining = self.plays_remaining.saturating_add(1);
                self.discards_remaining = self.discards_remaining.saturating_add(1);
                self.sync_round_resource_caps();
            }
            MemorialTalismanKind::MeldMason => {
                self.memorial_round.next_cashin_bonus_chips = 80;
                self.memorial_round.next_cashin_yaku = snapshot.and_then(|s| s.dominant_yaku);
            }
            MemorialTalismanKind::DeepWalker => {
                self.memorial_round.next_cashin_bonus_chips = 60;
                self.memorial_round.next_cashin_yaku = None;
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
            TalismanKind::Souzu | TalismanKind::Pinzu | TalismanKind::Manzu => {
                let target = match kind {
                    TalismanKind::Souzu => Suit::Souzu,
                    TalismanKind::Pinzu => Suit::Pinzu,
                    TalismanKind::Manzu => Suit::Manzu,
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
    pub(crate) fn restamp_hand_enhancements(&mut self) {
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

    /// Recompute inventory capacities from currently-owned relics.
    /// Idempotent — call after any relic add/remove.
    pub fn recompute_capacities(&mut self) {
        let mut relic_slots = 6usize;
        if self.relics.owns(RelicId::BrocadePouch) {
            relic_slots += 1;
        }
        self.relics.max_slots = relic_slots.max(self.relics.active.len());

        let mut consumable_cap = self.mode.consumable_capacity;
        if self.relics.owns(RelicId::BrocadePouch) {
            consumable_cap += 1;
        }
        self.consumables.capacity = consumable_cap;
    }
}
