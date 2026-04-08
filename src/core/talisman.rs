//! Talismans — tarot-style consumables that buff every tile in the player's
//! current hand at once.
//!
//! In Balatro, tarots target a few selected cards. Mahjuro's hands are 14
//! tiles wide, so selecting 1–3 would never feel meaningful. Instead, every
//! talisman applies its enhancement to the **whole hand**: one click, one big
//! per-hand swing. The enhancement lives on each [`crate::core::tile::Tile`]
//! while it sits in the hand and is dropped when the tile leaves (played or
//! discarded).
//!
//! Talismans live in the same inventory as Zodiacs (see
//! [`crate::core::consumable::ConsumableInventory`]) — they share slot space
//! so the player chooses how to spend their consumable budget each round.

use serde::{Deserialize, Serialize};

use crate::core::tile::{Tile, TileEnhancement};

/// One talisman variant. Each maps to a single [`TileEnhancement`] kind that
/// gets stamped onto every tile in the hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TalismanKind {
    Jade,
    Pearl,
    Gilded,
    Polychrome,
}

impl TalismanKind {
    pub fn all() -> &'static [TalismanKind] {
        &[
            TalismanKind::Jade,
            TalismanKind::Pearl,
            TalismanKind::Gilded,
            TalismanKind::Polychrome,
        ]
    }

    pub fn name(self) -> &'static str {
        match self {
            TalismanKind::Jade => "Jade Talisman",
            TalismanKind::Pearl => "Pearl Talisman",
            TalismanKind::Gilded => "Gilded Talisman",
            TalismanKind::Polychrome => "Polychrome Talisman",
        }
    }

    /// One-line shop tooltip.
    pub fn description(self) -> &'static str {
        match self {
            TalismanKind::Jade => "Every tile in hand: +20 chips when scored in a meld.",
            TalismanKind::Pearl => "Every tile in hand: +30 flat chips when scored.",
            TalismanKind::Gilded => "Every tile in hand: +0.5 mult when scored in a meld.",
            TalismanKind::Polychrome => {
                "Every meld played from this hand gets ×1.2 mult."
            }
        }
    }

    /// The enhancement this talisman stamps onto each hand tile.
    pub fn enhancement(self) -> TileEnhancement {
        match self {
            TalismanKind::Jade => TileEnhancement::Jade,
            TalismanKind::Pearl => TileEnhancement::Pearl,
            TalismanKind::Gilded => TileEnhancement::Gilded,
            TalismanKind::Polychrome => TileEnhancement::Polychrome,
        }
    }

    /// Flat shop price in gold. Tuned higher than a Zodiac (4g) because the
    /// effect lands all at once and is more swingy.
    pub fn shop_price() -> u32 {
        6
    }
}

/// Apply a talisman to every tile in the given hand. A tile can only carry
/// one talisman mark at a time, so any existing enhancement is **replaced**
/// by the new one (you can re-stamp the hand at will — most-recent wins).
/// Returns the number of tiles that were stamped.
pub fn apply_to_hand(hand: &mut [Tile], kind: TalismanKind) -> usize {
    let enh = kind.enhancement();
    for tile in hand.iter_mut() {
        tile.enhancement = Some(enh);
    }
    hand.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tile::Suit;

    #[test]
    fn apply_stamps_every_tile() {
        let mut hand = vec![
            Tile::new(Suit::Bamboos, 3, 0),
            Tile::new(Suit::Circles, 7, 1),
            Tile::new(Suit::Dragon, 1, 2),
        ];
        apply_to_hand(&mut hand, TalismanKind::Jade);
        for t in &hand {
            assert_eq!(t.enhancement, Some(TileEnhancement::Jade));
        }
    }

    #[test]
    fn apply_replaces_existing_enhancement() {
        // Each tile can carry only one talisman mark at a time — applying a
        // new talisman overwrites whatever was on the tile before.
        let mut hand = vec![Tile::new(Suit::Bamboos, 5, 0)];
        apply_to_hand(&mut hand, TalismanKind::Jade);
        apply_to_hand(&mut hand, TalismanKind::Pearl);
        assert_eq!(hand[0].enhancement, Some(TileEnhancement::Pearl));
    }

    #[test]
    fn each_kind_has_unique_enhancement() {
        let mut seen = std::collections::HashSet::new();
        for &k in TalismanKind::all() {
            assert!(seen.insert(k.enhancement()), "duplicate for {:?}", k);
        }
    }
}
