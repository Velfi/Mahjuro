//! Tile types for a riichi-style set (numbered suits + honors).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Suit {
    // Ordered: numbered suits first, then honors.
    Characters,
    Bamboos,
    Circles,
    Wind,
    Dragon,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Tile {
    pub suit: Suit,
    /// Number tiles: 1–9. Winds: 1–4. Dragons: 1–3.
    pub rank: u8,
    /// Unique id within a run (duplicates allowed).
    pub id: u32,
}

impl Tile {
    pub fn new(suit: Suit, rank: u8, id: u32) -> Self {
        Self { suit, rank, id }
    }

    /// Same tile face (ignores id).
    #[allow(dead_code)]
    pub fn matches_face(&self, other: &Tile) -> bool {
        self.suit == other.suit && self.rank == other.rank
    }

    #[allow(dead_code)]
    pub fn is_number_tile(&self) -> bool {
        matches!(self.suit, Suit::Characters | Suit::Bamboos | Suit::Circles)
    }

    /// Short display label, e.g. "3m", "7s", "East", "Red".
    pub fn label(&self) -> String {
        match self.suit {
            Suit::Wind => match self.rank {
                1 => "East".into(),
                2 => "South".into(),
                3 => "West".into(),
                4 => "North".into(),
                _ => format!("W{}", self.rank),
            },
            Suit::Dragon => match self.rank {
                1 => "Red".into(),
                2 => "Green".into(),
                3 => "White".into(),
                _ => format!("D{}", self.rank),
            },
            Suit::Characters => format!("{}m", self.rank),
            Suit::Bamboos => format!("{}s", self.rank),
            Suit::Circles => format!("{}p", self.rank),
        }
    }

    /// Base point value of a single tile face: numbered tiles are worth their
    /// rank (1–9), honors (winds & dragons) are flat 10.
    pub fn point_value(&self) -> u32 {
        match self.suit {
            Suit::Characters | Suit::Bamboos | Suit::Circles => self.rank as u32,
            Suit::Wind | Suit::Dragon => 10,
        }
    }

    /// RGBA color hint for the tile's suit, for UI rendering.
    pub fn suit_color(&self) -> [f32; 4] {
        match self.suit {
            Suit::Characters => [0.85, 0.25, 0.20, 1.0], // red
            Suit::Bamboos => [0.20, 0.65, 0.30, 1.0],    // green
            Suit::Circles => [0.20, 0.40, 0.80, 1.0],    // blue
            Suit::Wind => [0.70, 0.60, 0.20, 1.0],       // gold
            Suit::Dragon => [0.60, 0.20, 0.70, 1.0],     // purple
        }
    }
}
