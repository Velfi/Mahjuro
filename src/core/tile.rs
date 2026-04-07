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

    /// Long, human-readable name suitable for tooltips, e.g.
    /// "5 of Bamboo", "East Wind", "Red Dragon (Chun)".
    pub fn full_name(&self) -> String {
        match self.suit {
            Suit::Characters => format!("{} of Characters", self.rank),
            Suit::Bamboos => format!("{} of Bamboo", self.rank),
            Suit::Circles => format!("{} of Circles", self.rank),
            Suit::Wind => match self.rank {
                1 => "East Wind".into(),
                2 => "South Wind".into(),
                3 => "West Wind".into(),
                4 => "North Wind".into(),
                _ => format!("Wind {}", self.rank),
            },
            Suit::Dragon => match self.rank {
                1 => "Red Dragon (Chun)".into(),
                2 => "Green Dragon (Hatsu)".into(),
                3 => "White Dragon (Haku)".into(),
                _ => format!("Dragon {}", self.rank),
            },
        }
    }

    /// Classification used in scoring and tooltips:
    /// - "terminal" for 1 or 9 of a numbered suit
    /// - "simple"   for 2–8 of a numbered suit
    /// - "honor"    for any wind or dragon
    pub fn category(&self) -> &'static str {
        match self.suit {
            Suit::Wind | Suit::Dragon => "honor",
            Suit::Characters | Suit::Bamboos | Suit::Circles => {
                if self.rank == 1 || self.rank == 9 {
                    "terminal"
                } else {
                    "simple"
                }
            }
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
            // Dragons are coloured per rank in the traditional set:
            //   1 = Chun  (中) → red
            //   2 = Hatsu (發) → green
            //   3 = Haku  (白) → ivory/white
            Suit::Dragon => match self.rank {
                1 => [0.85, 0.20, 0.18, 1.0], // red
                2 => [0.20, 0.65, 0.30, 1.0], // green
                3 => [0.90, 0.88, 0.82, 1.0], // ivory white
                _ => [0.60, 0.20, 0.70, 1.0], // fallback (shouldn't happen)
            },
        }
    }
}
