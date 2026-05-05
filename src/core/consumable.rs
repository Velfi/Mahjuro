//! Shared consumable inventory: zodiac cards and talismans in one capped slot
//! list. The player trades slots between the two types each run. Capacity is
//! set by the game mode (default 2, expandable via relics).

use serde::{Deserialize, Serialize};

use crate::core::talisman::TalismanKind;
use crate::core::zodiac::ZodiacKind;

/// One slot in the shared inventory — either a Zodiac or a Talisman.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum Consumable {
    Zodiac(ZodiacKind),
    Talisman(TalismanKind),
}

impl Consumable {
    pub fn name(self) -> String {
        match self {
            Consumable::Zodiac(z) => z.name().to_string(),
            Consumable::Talisman(t) => t.name().to_string(),
        }
    }
}

/// Capped inventory of consumables. Pushing past capacity is rejected — the
/// caller chooses to use, sell, or skip.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConsumableInventory {
    pub items: Vec<Consumable>,
    pub capacity: usize,
}

impl Default for ConsumableInventory {
    fn default() -> Self {
        // 2 base slots — matches `GameMode::standard().consumable_capacity`.
        // Brocade Pouch pushes this up via `recompute_capacities`.
        Self {
            items: Vec::new(),
            capacity: 2,
        }
    }
}

impl ConsumableInventory {
    pub fn is_full(&self) -> bool {
        self.items.len() >= self.capacity
    }

    /// Try to add a consumable. Returns `true` on success, `false` if full.
    pub fn try_push(&mut self, c: Consumable) -> bool {
        if self.is_full() {
            false
        } else {
            self.items.push(c);
            true
        }
    }

    /// Remove and return the consumable at `index`, if any.
    pub fn take(&mut self, index: usize) -> Option<Consumable> {
        if index < self.items.len() {
            Some(self.items.remove(index))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_respects_capacity() {
        let mut inv = ConsumableInventory {
            items: Vec::new(),
            capacity: 2,
        };
        assert!(inv.try_push(Consumable::Zodiac(ZodiacKind::Rat)));
        assert!(inv.try_push(Consumable::Talisman(TalismanKind::Jade)));
        assert!(inv.is_full());
        assert!(!inv.try_push(Consumable::Zodiac(ZodiacKind::Ox)));
    }

    #[test]
    fn take_removes_at_index() {
        let mut inv = ConsumableInventory::default();
        inv.try_push(Consumable::Zodiac(ZodiacKind::Rat));
        inv.try_push(Consumable::Talisman(TalismanKind::Pearl));
        let taken = inv.take(0);
        assert!(matches!(taken, Some(Consumable::Zodiac(ZodiacKind::Rat))));
        assert_eq!(inv.items.len(), 1);
    }
}
