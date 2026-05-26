//! Tile types for a standard mahjong set (numbered suits + honors).

use serde::{Deserialize, Deserializer, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Suit {
    // Ordered: numbered suits first, then honors, then bonus.
    #[serde(alias = "Characters")]
    Manzu,
    #[serde(alias = "Bamboos")]
    Souzu,
    #[serde(alias = "Dots")]
    Pinzu,
    Wind,
    Dragon,
    /// Bonus flower tiles (ranks 1–4). Rare wildcards that can substitute for
    /// one missing tile in a triplet or sequence (max one flower per meld).
    Flower,
    /// Bonus season tiles (ranks 1–4: Spring, Summer, Autumn, Winter).
    /// Used only in solitaire mode; not part of the main game deck.
    Season,
}

impl Suit {
    /// Solid tint when this suit’s name is highlighted as UI vocabulary
    /// (tutorial copy, etc.). Dragon uses the Chun (red) face hue so the
    /// keyword stays legible without a rank context.
    pub const fn keyword_color(self) -> [f32; 4] {
        match self {
            Suit::Manzu => [0.581, 0.320, 0.298, 1.0],
            Suit::Souzu => [0.386, 0.582, 0.429, 1.0],
            Suit::Pinzu => [0.306, 0.393, 0.567, 1.0],
            Suit::Wind => [0.639, 0.596, 0.422, 1.0],
            Suit::Dragon => [0.560, 0.277, 0.268, 1.0],
            Suit::Flower => [0.704, 0.508, 0.552, 1.0],
            Suit::Season => [0.476, 0.650, 0.628, 1.0],
        }
    }
}

/// A talisman-applied enhancement attached to an individual tile. The
/// enhancement is recorded against the tile's id in
/// [`crate::game::run::RunState::tile_enhancements`] and re-stamped onto the
/// hand whenever tiles are drawn, so it persists for the rest of the run
/// (across plays, discards, refills, and new-round redeals). See
/// [`crate::core::talisman`] for the consumables that stamp these onto every
/// hand tile at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TileEnhancement {
    /// Flat chips per scored meld that includes this stamp (see scoring pipeline; Polychrome’s chip twin).
    Pearl,
    /// +$1 gold once per scored meld that includes this stamp (pairs included).
    Gilded,
    /// ×1.25 mult applied once per meld that contains this tile.
    Polychrome,
}

impl<'de> Deserialize<'de> for TileEnhancement {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "pearl" | "jade" => Ok(Self::Pearl),
            "gilded" => Ok(Self::Gilded),
            "polychrome" => Ok(Self::Polychrome),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["pearl", "gilded", "polychrome"],
            )),
        }
    }
}

impl TileEnhancement {
    /// Numeric ID passed to the tile_3d shader via `base_color_factor.z`.
    /// 0 = none; 1 = pearl, 2 = gilded, 3 = polychrome.
    pub fn shader_id(self) -> f32 {
        match self {
            TileEnhancement::Pearl => 1.0,
            TileEnhancement::Gilded => 2.0,
            TileEnhancement::Polychrome => 3.0,
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

    /// Face-only copy for chronicle / showcase (stable across runs).
    pub fn display_copy(self) -> Self {
        Self {
            id: 0,
            suit: self.suit,
            rank: self.rank,
            enhancement: self.enhancement,
            debuffed_visual: false,
        }
    }

    /// True for the 13 terminal/honor faces used in Kokushi Musō (1/9 in each
    /// number suit, four winds, three dragons). Flowers and seasons are never orphans.
    pub fn is_kokushi_orphan_face(suit: Suit, rank: u8) -> bool {
        match suit {
            Suit::Manzu | Suit::Souzu | Suit::Pinzu => rank == 1 || rank == 9,
            Suit::Wind => (1..=4).contains(&rank),
            Suit::Dragon => (1..=3).contains(&rank),
            Suit::Flower | Suit::Season => false,
        }
    }

    pub fn is_kokushi_orphan(self) -> bool {
        Self::is_kokushi_orphan_face(self.suit, self.rank)
    }

    pub fn is_number_tile(&self) -> bool {
        matches!(self.suit, Suit::Manzu | Suit::Souzu | Suit::Pinzu)
    }

    /// Returns `true` for bonus flower tiles.
    pub fn is_flower(&self) -> bool {
        self.suit == Suit::Flower
    }

    /// Compact notation for logs, cascade lines, and tooltips (`3m`/`7s`, winds `E`/`S`/…).
    /// Prefer [`Self::full_name`] for the primary player-facing name.
    pub fn label(&self) -> String {
        match self.suit {
            Suit::Wind => match self.rank {
                1 => "E".into(),
                2 => "S".into(),
                3 => "W".into(),
                4 => "N".into(),
                _ => format!("W{}", self.rank),
            },
            Suit::Dragon => match self.rank {
                1 => "R".into(),
                2 => "G".into(),
                3 => "Wh".into(),
                _ => format!("D{}", self.rank),
            },
            Suit::Manzu => format!("{}m", self.rank),
            Suit::Souzu => format!("{}s", self.rank),
            Suit::Pinzu => format!("{}p", self.rank),
            Suit::Flower => format!("F{}", self.rank),
            Suit::Season => format!("S{}", self.rank),
        }
    }

    /// Long, human-readable name, e.g. "5 of Souzu", "East Wind", "Red Dragon (Chun)".
    pub fn full_name(&self) -> String {
        match self.suit {
            Suit::Manzu => format!("{} of Manzu", self.rank),
            Suit::Souzu => format!("{} of Souzu", self.rank),
            Suit::Pinzu => format!("{} of Pinzu", self.rank),
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

    /// Base point value of a single tile face: numbered tiles are worth their
    /// rank (1–9), honors (winds & dragons) are flat 12. The honor pump (was 10)
    /// gives Yakuhai/Honitsu/Honroutou builds enough early-game traction to be
    /// playable from ante 1, when leveled-yaku mults haven't kicked in yet.
    pub fn point_value(&self) -> u32 {
        match self.suit {
            Suit::Manzu | Suit::Souzu | Suit::Pinzu => self.rank as u32,
            Suit::Wind | Suit::Dragon => 15,
            // Flower wildcards contribute no chip value — their power is structural.
            Suit::Flower | Suit::Season => 0,
        }
    }

    /// RGBA color hint for the tile's suit, for UI rendering.
    pub fn suit_color(&self) -> [f32; 4] {
        match self.suit {
            Suit::Manzu => Suit::Manzu.keyword_color(),
            Suit::Souzu => Suit::Souzu.keyword_color(),
            Suit::Pinzu => Suit::Pinzu.keyword_color(),
            Suit::Wind => Suit::Wind.keyword_color(),
            // Dragons are coloured per rank in the traditional set:
            //   1 = Chun  (中) → red
            //   2 = Hatsu (發) → green
            //   3 = Haku  (白) → ivory/white
            Suit::Dragon => match self.rank {
                1 => Suit::Dragon.keyword_color(),
                2 => Suit::Souzu.keyword_color(),
                3 => [0.90, 0.88, 0.82, 1.0], // ivory white (low sat already)
                _ => [0.55, 0.42, 0.58, 1.0], // fallback (shouldn't happen)
            },
            Suit::Flower => Suit::Flower.keyword_color(),
            Suit::Season => Suit::Season.keyword_color(),
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
        let mut v = [
            Tile::new(Suit::Pinzu, 7, 1),
            Tile::new(Suit::Pinzu, 2, 2),
            Tile::new(Suit::Pinzu, 5, 3),
        ];
        v.sort_by(cmp_sort_order);
        assert_eq!(v[0].rank, 2);
        assert_eq!(v[1].rank, 5);
        assert_eq!(v[2].rank, 7);
    }

    #[test]
    fn cmp_sort_order_places_pinzu_before_dragons() {
        let mut v = [Tile::new(Suit::Dragon, 1, 1), Tile::new(Suit::Pinzu, 5, 2)];
        v.sort_by(cmp_sort_order);
        assert_eq!(v[0].suit, Suit::Pinzu);
        assert_eq!(v[1].suit, Suit::Dragon);
    }

    #[test]
    fn label_uses_compact_honor_shorthand() {
        assert_eq!(Tile::new(Suit::Pinzu, 9, 0).label(), "9p");
        assert_eq!(Tile::new(Suit::Wind, 1, 0).label(), "E");
        assert_eq!(Tile::new(Suit::Dragon, 3, 0).label(), "Wh");
    }
}
