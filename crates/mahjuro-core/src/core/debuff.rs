use serde::{Deserialize, Serialize};

use crate::core::tile::{Suit, Tile};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TileDebuffClass {
    Honors,
    Terminals,
    Simples,
    MiddleTiles,
}

impl TileDebuffClass {
    pub fn label(self) -> &'static str {
        match self {
            TileDebuffClass::Honors => "Honors",
            TileDebuffClass::Terminals => "Terminals",
            TileDebuffClass::Simples => "Simples",
            TileDebuffClass::MiddleTiles => "Rank-5 tiles",
        }
    }

    pub fn matches(self, tile: &Tile) -> bool {
        match self {
            TileDebuffClass::Honors => matches!(tile.suit, Suit::Wind | Suit::Dragon),
            TileDebuffClass::Terminals => {
                matches!(tile.suit, Suit::Manzu | Suit::Souzu | Suit::Pinzu)
                    && matches!(tile.rank, 1 | 9)
            }
            TileDebuffClass::Simples => {
                matches!(tile.suit, Suit::Manzu | Suit::Souzu | Suit::Pinzu)
                    && matches!(tile.rank, 2..=8)
            }
            TileDebuffClass::MiddleTiles => {
                matches!(tile.suit, Suit::Manzu | Suit::Souzu | Suit::Pinzu) && tile.rank == 5
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum TileDebuff {
    Suit(Suit),
    Class(TileDebuffClass),
    /// Debuffs a rank across Manzu, Souzu, and Pinzu (The Blight's dominant-rank axis).
    Rank(u8),
}

impl TileDebuff {
    pub fn label(self) -> &'static str {
        match self {
            TileDebuff::Suit(Suit::Manzu) => "Manzu",
            TileDebuff::Suit(Suit::Souzu) => "Souzu",
            TileDebuff::Suit(Suit::Pinzu) => "Pinzu",
            TileDebuff::Suit(Suit::Wind) => "Winds",
            TileDebuff::Suit(Suit::Dragon) => "Dragons",
            TileDebuff::Suit(Suit::Flower) => "Flowers",
            TileDebuff::Suit(Suit::Season) => "Seasons",
            TileDebuff::Class(class) => class.label(),
            TileDebuff::Rank(_) => "Rank",
        }
    }

    /// Player-facing debuff name (includes resolved ranks for [`Self::Rank`]).
    pub fn display_label(self) -> String {
        match self {
            TileDebuff::Rank(rank) => format!("Rank-{rank}"),
            other => other.label().to_string(),
        }
    }

    pub fn matches(self, tile: &Tile) -> bool {
        match self {
            TileDebuff::Suit(suit) => tile.suit == suit,
            TileDebuff::Class(class) => class.matches(tile),
            TileDebuff::Rank(rank) => {
                matches!(tile.suit, Suit::Manzu | Suit::Souzu | Suit::Pinzu) && tile.rank == rank
            }
        }
    }

    /// Rank with the highest copy count across Manzu, Souzu, and Pinzu. Ties → lowest rank.
    pub fn dominant_rank(tiles: &[Tile]) -> Self {
        let mut counts = [0usize; 10];
        for tile in tiles {
            if matches!(tile.suit, Suit::Manzu | Suit::Souzu | Suit::Pinzu) {
                counts[tile.rank as usize] += 1;
            }
        }
        let rank = (1..=9u8)
            .max_by(|&a, &b| {
                counts[a as usize]
                    .cmp(&counts[b as usize])
                    .then_with(|| b.cmp(&a))
            })
            .unwrap_or(1);
        TileDebuff::Rank(rank)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn rank_debuff_matches_all_number_suits() {
        let debuff = TileDebuff::Rank(6);
        assert!(debuff.matches(&tile(Suit::Pinzu, 6, 1)));
        assert!(debuff.matches(&tile(Suit::Manzu, 6, 2)));
        assert!(debuff.matches(&tile(Suit::Souzu, 6, 3)));
        assert!(!debuff.matches(&tile(Suit::Pinzu, 5, 4)));
        assert!(!debuff.matches(&tile(Suit::Wind, 1, 5)));
    }

    #[test]
    fn dominant_rank_picks_highest_count() {
        let debuff = TileDebuff::dominant_rank(&[
            tile(Suit::Pinzu, 6, 1),
            tile(Suit::Pinzu, 6, 2),
            tile(Suit::Pinzu, 6, 3),
            tile(Suit::Manzu, 1, 4),
        ]);
        assert_eq!(debuff, TileDebuff::Rank(6));
    }

    #[test]
    fn dominant_rank_sums_across_suits() {
        let debuff = TileDebuff::dominant_rank(&[
            tile(Suit::Pinzu, 6, 1),
            tile(Suit::Manzu, 6, 2),
            tile(Suit::Souzu, 6, 3),
            tile(Suit::Pinzu, 9, 4),
            tile(Suit::Pinzu, 9, 5),
        ]);
        assert_eq!(debuff, TileDebuff::Rank(6));
    }

    #[test]
    fn dominant_rank_tie_breaks_to_lowest_rank() {
        let debuff = TileDebuff::dominant_rank(&[
            tile(Suit::Souzu, 5, 1),
            tile(Suit::Souzu, 5, 2),
            tile(Suit::Manzu, 7, 3),
            tile(Suit::Manzu, 7, 4),
        ]);
        assert_eq!(debuff, TileDebuff::Rank(5));
    }
}
