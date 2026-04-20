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
                matches!(tile.suit, Suit::Characters | Suit::Bamboos | Suit::Circles)
                    && matches!(tile.rank, 1 | 9)
            }
            TileDebuffClass::Simples => {
                matches!(tile.suit, Suit::Characters | Suit::Bamboos | Suit::Circles)
                    && matches!(tile.rank, 2..=8)
            }
            TileDebuffClass::MiddleTiles => {
                matches!(tile.suit, Suit::Characters | Suit::Bamboos | Suit::Circles)
                    && tile.rank == 5
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum TileDebuff {
    Suit(Suit),
    Class(TileDebuffClass),
}

impl TileDebuff {
    pub fn label(self) -> &'static str {
        match self {
            TileDebuff::Suit(Suit::Characters) => "Characters",
            TileDebuff::Suit(Suit::Bamboos) => "Bamboos",
            TileDebuff::Suit(Suit::Circles) => "Circles",
            TileDebuff::Suit(Suit::Wind) => "Winds",
            TileDebuff::Suit(Suit::Dragon) => "Dragons",
            TileDebuff::Suit(Suit::Flower) => "Flowers",
            TileDebuff::Suit(Suit::Season) => "Seasons",
            TileDebuff::Class(class) => class.label(),
        }
    }

    pub fn matches(self, tile: &Tile) -> bool {
        match self {
            TileDebuff::Suit(suit) => tile.suit == suit,
            TileDebuff::Class(class) => class.matches(tile),
        }
    }
}
