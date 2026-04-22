//! Tile types for a riichi-style set (numbered suits + honors).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Suit {
    // Ordered: numbered suits first, then honors, then bonus.
    Characters,
    Bamboos,
    Circles,
    Wind,
    Dragon,
    /// Bonus flower tiles (ranks 1–4). Rare wildcards that can substitute for
    /// one missing tile in a triplet or sequence (max one flower per meld).
    Flower,
    /// Bonus season tiles (ranks 1–4: Spring, Summer, Autumn, Winter).
    /// Used only in solitaire mode; not part of the main game deck.
    Season,
}

/// A talisman-applied enhancement attached to an individual tile. The
/// enhancement is recorded against the tile's id in
/// [`crate::game::run::RunState::tile_enhancements`] and re-stamped onto the
/// hand whenever tiles are drawn, so it persists for the rest of the run
/// (across plays, discards, refills, and new-round redeals). See
/// [`crate::core::talisman`] for the consumables that stamp these onto every
/// hand tile at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TileEnhancement {
    /// +20 chips when this tile is part of a scored meld.
    Jade,
    /// +25 flat chips when this tile is scored, meld or pair.
    Pearl,
    /// +$1 when this tile is part of a scored meld.
    Gilded,
    /// ×1.15 mult applied once per meld that contains this tile.
    Polychrome,
}

impl TileEnhancement {
    /// Numeric ID passed to the tile_3d shader via `base_color_factor.z`.
    /// 0 = no enhancement (used by `Option::map_or`), 1–4 = Jade/Pearl/Gilded/Polychrome.
    pub fn shader_id(self) -> f32 {
        match self {
            TileEnhancement::Jade => 1.0,
            TileEnhancement::Pearl => 2.0,
            TileEnhancement::Gilded => 3.0,
            TileEnhancement::Polychrome => 4.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Tile {
    pub suit: Suit,
    /// Number tiles: 1–9. Winds: 1–4. Dragons: 1–3.
    pub rank: u8,
    /// Unique id within a run (duplicates allowed).
    pub id: u32,
    /// Optional talisman enhancement. Newly-built tiles default to `None`;
    /// `RunState` re-applies persistent enhancements (keyed by tile id) every
    /// time a tile is drawn into the player's hand.
    #[serde(default)]
    pub enhancement: Option<TileEnhancement>,
    /// Transient render-only flag used by the tile decal rasterizer to stamp
    /// a debuff marker on the face. Gameplay/scoring debuffs are still sourced
    /// from run state.
    #[serde(default)]
    pub debuffed_visual: bool,
}

impl Tile {
    pub fn new(suit: Suit, rank: u8, id: u32) -> Self {
        Self {
            suit,
            rank,
            id,
            enhancement: None,
            debuffed_visual: false,
        }
    }

    pub fn is_number_tile(&self) -> bool {
        matches!(self.suit, Suit::Characters | Suit::Bamboos | Suit::Circles)
    }

    /// Returns `true` for bonus flower tiles.
    pub fn is_flower(&self) -> bool {
        self.suit == Suit::Flower
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
            Suit::Flower => format!("F{}", self.rank),
            Suit::Season => format!("S{}", self.rank),
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
            Suit::Flower => match self.rank {
                1 => "Plum Blossom".into(),
                2 => "Orchid".into(),
                3 => "Chrysanthemum".into(),
                4 => "Bamboo".into(),
                _ => format!("Flower {}", self.rank),
            },
            Suit::Season => match self.rank {
                1 => "Spring".into(),
                2 => "Summer".into(),
                3 => "Autumn".into(),
                4 => "Winter".into(),
                _ => format!("Season {}", self.rank),
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
            Suit::Flower | Suit::Season => "bonus",
            Suit::Characters | Suit::Bamboos | Suit::Circles => {
                if self.rank == 1 || self.rank == 9 {
                    "terminal"
                } else {
                    "simple"
                }
            }
        }
    }

    /// One-line description of this flower's triggered scoring effect, or
    /// `None` for non-flower tiles. Suitable for tooltips and tile info.
    pub fn flower_effect_label(&self) -> Option<&'static str> {
        if self.suit != Suit::Flower {
            return None;
        }
        Some(match self.rank {
            1 => "+40 chips",
            2 => "+1.5 mult",
            3 => "+15 chips per meld",
            4 => "+$4 gold",
            _ => return None,
        })
    }

    /// Base point value of a single tile face: numbered tiles are worth their
    /// rank (1–9), honors (winds & dragons) are flat 12. The honor pump (was 10)
    /// gives Yakuhai/Honitsu/Honroutou builds enough early-game traction to be
    /// playable from ante 1, when leveled-yaku mults haven't kicked in yet.
    pub fn point_value(&self) -> u32 {
        match self.suit {
            Suit::Characters | Suit::Bamboos | Suit::Circles => self.rank as u32,
            Suit::Wind | Suit::Dragon => 12,
            // Flower wildcards contribute no chip value — their power is structural.
            Suit::Flower | Suit::Season => 0,
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
            // Flowers — warm pink, reads as "special bonus" at a glance.
            Suit::Flower => [0.90, 0.45, 0.55, 1.0],
            // Seasons — cool teal, distinct from flowers.
            Suit::Season => [0.30, 0.70, 0.65, 1.0],
        }
    }
}

/// Stable ordering for rack sorts: **suit → rank → id** only.
///
/// Kept explicit so the hand strip always follows face order; [`Tile`]'s derived
/// [`Ord`] also includes enhancement / visual flags after `id`.
#[inline]
pub fn cmp_sort_order(a: &Tile, b: &Tile) -> std::cmp::Ordering {
    a.suit
        .cmp(&b.suit)
        .then(a.rank.cmp(&b.rank))
        .then(a.id.cmp(&b.id))
}

#[cfg(test)]
mod sort_order_tests {
    use super::{Suit, Tile, cmp_sort_order};

    #[test]
    fn cmp_sort_order_ranks_ascending_within_suit() {
        let mut v = vec![
            Tile::new(Suit::Circles, 7, 1),
            Tile::new(Suit::Circles, 2, 2),
            Tile::new(Suit::Circles, 5, 3),
        ];
        v.sort_by(cmp_sort_order);
        assert_eq!(v[0].rank, 2);
        assert_eq!(v[1].rank, 5);
        assert_eq!(v[2].rank, 7);
    }

    #[test]
    fn cmp_sort_order_places_circles_before_dragons() {
        let mut v = vec![
            Tile::new(Suit::Dragon, 1, 1),
            Tile::new(Suit::Circles, 5, 2),
        ];
        v.sort_by(cmp_sort_order);
        assert_eq!(v[0].suit, Suit::Circles);
        assert_eq!(v[1].suit, Suit::Dragon);
    }
}
