#![allow(dead_code)]
//! Chinese Zodiac consumable cards — Mahjuro's planet-card analogue.
//!
//! Each Zodiac card is mapped 1:1 to one yaku in [`crate::core::yaku`]
//! (including Kokushi Musō via the Qilin ribbon). Using a card increments the *level* of its yaku for
//! the rest of the run, scaling both the chip and mult contributions per the
//! formula in `YakuKind::mult_bonus_at` / `chip_bonus_at`:
//!
//!   mult_bonus(level) = base_mult + 0.5 × (level - 1)
//!   chip_bonus(level) = base_chips + 20 × (level - 1)
//!
//! Drop economy:
//!   * Small Blind clear → 1 random Zodiac
//!   * Big Blind clear   → 1 random Zodiac
//!   * Boss Blind clear  → 2 random Zodiacs (or one Festival pack of 3 → pick 1)
//!   * Shop              → 4 gold per single, 6 gold per Festival pack
//!     (Qilin is omitted from shop zodiac rolls until Kokushi Musō has been
//!     scored at least once on the save profile.)
//!
//! The consumable inventory holds 2 cards by default and is expandable via
//! the Brocade Pouch (+1) relic.

use serde::{Deserialize, Serialize};

use crate::core::yaku::YakuKind;

/// Zodiac ribbon kinds: the thirteen calendar animals (Mouse precedes Rat) plus
/// **Qilin** for Kokushi Musō. Variant order matters for serialization stability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZodiacKind {
    Mouse,
    Rat,
    Ox,
    Tiger,
    Rabbit,
    Dragon,
    Snake,
    Horse,
    Goat,
    Monkey,
    Rooster,
    Dog,
    Pig,
    /// Mythical auspicious beast; levels Kokushi Musō (thirteen orphans).
    Qilin,
}

impl ZodiacKind {
    /// All ribbon kinds: calendar order (Mouse precedes Rat), then Qilin.
    pub fn all() -> &'static [ZodiacKind] {
        &[
            ZodiacKind::Mouse,
            ZodiacKind::Rat,
            ZodiacKind::Ox,
            ZodiacKind::Tiger,
            ZodiacKind::Rabbit,
            ZodiacKind::Dragon,
            ZodiacKind::Snake,
            ZodiacKind::Horse,
            ZodiacKind::Goat,
            ZodiacKind::Monkey,
            ZodiacKind::Rooster,
            ZodiacKind::Dog,
            ZodiacKind::Pig,
            ZodiacKind::Qilin,
        ]
    }

    /// English display name.
    pub fn name(self) -> &'static str {
        match self {
            ZodiacKind::Mouse => "Mouse",
            ZodiacKind::Rat => "Rat",
            ZodiacKind::Ox => "Ox",
            ZodiacKind::Tiger => "Tiger",
            ZodiacKind::Rabbit => "Rabbit",
            ZodiacKind::Dragon => "Dragon",
            ZodiacKind::Snake => "Snake",
            ZodiacKind::Horse => "Horse",
            ZodiacKind::Goat => "Goat",
            ZodiacKind::Monkey => "Monkey",
            ZodiacKind::Rooster => "Rooster",
            ZodiacKind::Dog => "Dog",
            ZodiacKind::Pig => "Pig",
            ZodiacKind::Qilin => "Qilin",
        }
    }

    /// The yaku this zodiac levels up when used. The 1:1 mapping is fixed in
    /// the design plan; see Part C of the design doc for the rationale of
    /// each pairing.
    pub fn yaku(self) -> YakuKind {
        match self {
            ZodiacKind::Mouse => YakuKind::Honitsu,
            ZodiacKind::Rat => YakuKind::Chinitsu,
            ZodiacKind::Ox => YakuKind::Toitoi,
            ZodiacKind::Tiger => YakuKind::Honroutou,
            ZodiacKind::Rabbit => YakuKind::Iipeikou,
            ZodiacKind::Dragon => YakuKind::FullHand,
            ZodiacKind::Snake => YakuKind::Ittsu,
            ZodiacKind::Horse => YakuKind::SanshokuDoujun,
            ZodiacKind::Goat => YakuKind::Junchan,
            ZodiacKind::Monkey => YakuKind::Tanyao,
            ZodiacKind::Rooster => YakuKind::ChickenHand,
            ZodiacKind::Dog => YakuKind::Yakuhai,
            ZodiacKind::Pig => YakuKind::Chiitoitsu,
            ZodiacKind::Qilin => YakuKind::KokushiMusou,
        }
    }

    /// Lowercase slug used for asset filenames (e.g. `dragon` →
    /// `assets/textures/zodiac_dragon_{top,mid,bot}.png`). See
    /// `scripts/generate_zodiac_ribbons.py`.
    pub fn slug(self) -> &'static str {
        match self {
            ZodiacKind::Mouse => "mouse",
            ZodiacKind::Rat => "rat",
            ZodiacKind::Ox => "ox",
            ZodiacKind::Tiger => "tiger",
            ZodiacKind::Rabbit => "rabbit",
            ZodiacKind::Dragon => "dragon",
            ZodiacKind::Snake => "snake",
            ZodiacKind::Horse => "horse",
            ZodiacKind::Goat => "goat",
            ZodiacKind::Monkey => "monkey",
            ZodiacKind::Rooster => "rooster",
            ZodiacKind::Dog => "dog",
            ZodiacKind::Pig => "pig",
            ZodiacKind::Qilin => "qilin",
        }
    }

    /// Look up the zodiac that levels a given yaku, if any.
    pub fn for_yaku(yaku: YakuKind) -> Option<ZodiacKind> {
        Self::all().iter().copied().find(|z| z.yaku() == yaku)
    }

    /// Shop price in gold for buying one copy of any zodiac (same for all kinds).
    pub fn shop_price() -> u32 {
        6
    }
}

/// Small inventory of Zodiac cards held mid-run. Capacity is set by the run
/// (default 2, +1 from Brocade Pouch). Pushing past capacity is rejected —
/// the caller chooses to use, sell, or skip.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZodiacInventory {
    pub items: Vec<ZodiacKind>,
    pub capacity: usize,
}

impl Default for ZodiacInventory {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            capacity: 2,
        }
    }
}

impl ZodiacInventory {
    pub fn is_full(&self) -> bool {
        self.items.len() >= self.capacity
    }

    /// Try to add a card. Returns `true` on success, `false` if full.
    pub fn try_push(&mut self, z: ZodiacKind) -> bool {
        if self.is_full() {
            false
        } else {
            self.items.push(z);
            true
        }
    }

    /// Remove and return the card at `index`, if any.
    pub fn take(&mut self, index: usize) -> Option<ZodiacKind> {
        if index < self.items.len() {
            Some(self.items.remove(index))
        } else {
            None
        }
    }
}

/// Map of YakuKind → current level for the run. Defaults to level 1 for any
/// yaku not yet leveled. Centralized so scoring + UI both read the same source
/// of truth.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct YakuLevels {
    /// Sparse map: only yaku that have been leveled appear here.
    pub levels: std::collections::HashMap<YakuKind, u32>,
}

impl YakuLevels {
    /// Current level of `yaku` (defaults to 1 if never leveled).
    pub fn level_of(&self, yaku: YakuKind) -> u32 {
        self.levels.get(&yaku).copied().unwrap_or(1)
    }

    /// Increment a yaku's level by 1 (used when a Zodiac card is consumed).
    pub fn level_up(&mut self, yaku: YakuKind) -> u32 {
        let current = self.level_of(yaku);
        let next = current + 1;
        self.levels.insert(yaku, next);
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_zodiac_has_unique_yaku() {
        let mut seen: std::collections::HashSet<YakuKind> = Default::default();
        for &z in ZodiacKind::all() {
            assert!(seen.insert(z.yaku()), "duplicate yaku for {:?}", z);
        }
        assert_eq!(seen.len(), 14);
    }

    #[test]
    fn for_yaku_round_trips() {
        for &z in ZodiacKind::all() {
            let yk = z.yaku();
            assert_eq!(ZodiacKind::for_yaku(yk), Some(z));
        }
    }

    #[test]
    fn inventory_respects_capacity() {
        let mut inv = ZodiacInventory::default();
        assert!(inv.try_push(ZodiacKind::Rat));
        assert!(inv.try_push(ZodiacKind::Ox));
        assert!(inv.is_full());
        assert!(!inv.try_push(ZodiacKind::Tiger));
    }

    #[test]
    fn yaku_levels_default_to_one() {
        let yl = YakuLevels::default();
        assert_eq!(yl.level_of(YakuKind::Toitoi), 1);
    }

    #[test]
    fn level_up_increments() {
        let mut yl = YakuLevels::default();
        assert_eq!(yl.level_up(YakuKind::Toitoi), 2);
        assert_eq!(yl.level_up(YakuKind::Toitoi), 3);
        assert_eq!(yl.level_of(YakuKind::Toitoi), 3);
    }
}
