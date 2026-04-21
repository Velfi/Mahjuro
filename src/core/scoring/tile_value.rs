use crate::core::relic::{RelicId, RelicState};
use crate::core::tile::{Suit, Tile};

use super::tile_is_debuffed;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TileEffectiveValue {
    pub base_chips: i32,
    pub bonus_chips: i32,
    pub mult_bonus: f64,
    pub sources: Vec<(&'static str, String)>,
}

impl TileEffectiveValue {
    pub fn total_chips(&self) -> i32 {
        self.base_chips + self.bonus_chips
    }
}

pub fn tile_effective_value(
    tile: &Tile,
    relics: &RelicState,
    dora_faces: &[(Suit, u8)],
    tile_debuffs: &[crate::core::debuff::TileDebuff],
) -> TileEffectiveValue {
    use crate::core::tile::TileEnhancement;

    let mut out = TileEffectiveValue {
        base_chips: if tile_is_debuffed(tile, tile_debuffs) {
            0
        } else {
            tile.point_value() as i32
        },
        bonus_chips: 0,
        mult_bonus: 0.0,
        sources: Vec::new(),
    };

    if tile_is_debuffed(tile, tile_debuffs) {
        out.sources.push((
            "Debuffed",
            "This tile still forms hands, but scores 0 tile points".into(),
        ));
        return out;
    }

    if let Some(enh) = tile.enhancement {
        match enh {
            TileEnhancement::Pearl => {
                out.bonus_chips += 25;
                out.sources.push(("Pearl Talisman", "+25 chips".into()));
            }
            TileEnhancement::Jade => {
                out.bonus_chips += 20;
                out.sources.push(("Jade Talisman", "+20 chips".into()));
            }
            TileEnhancement::Gilded => {
                out.mult_bonus += 0.4;
                out.sources.push(("Gilded Talisman", "+0.4 mult".into()));
            }
            TileEnhancement::Polychrome => {
                out.mult_bonus += 0.15;
                out.sources
                    .push(("Polychrome Talisman", "+0.15 mult / meld".into()));
            }
        }
    }

    if dora_faces.contains(&(tile.suit, tile.rank)) {
        let per_dora = if relics.has(RelicId::DoraCrown) {
            35
        } else {
            25
        };
        out.bonus_chips += per_dora;
        out.sources.push(("Dora", format!("+{per_dora} chips")));
    }

    if relics.has(RelicId::HonorFury) && matches!(tile.suit, Suit::Wind | Suit::Dragon) {
        out.bonus_chips += 28;
        out.sources.push(("Honor Fury", "+28 chips".into()));
    }

    out
}
