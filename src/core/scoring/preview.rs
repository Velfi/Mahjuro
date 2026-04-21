use crate::core::hand::DetectedSet;
use crate::core::tile::Tile;
use crate::core::yaku::{YakuKind, detect_yaku_with_wind};

use super::{combine, meld_chip_bonus, tile_by_id, tile_is_debuffed};

#[allow(dead_code)]
#[derive(Clone, Debug)]
pub struct ScorePreview {
    pub chips: i32,
    pub mult: f64,
    pub detected_yaku: Vec<YakuKind>,
    pub estimated_total: u64,
}

#[allow(dead_code)]
pub fn preview_score(
    tiles: &[Tile],
    sets: &[DetectedSet],
    available_yaku: &[YakuKind],
    tile_debuffs: &[crate::core::debuff::TileDebuff],
    original_tiles: Option<&[Tile]>,
) -> ScorePreview {
    let mut chips: i32 = 0;
    for s in sets {
        chips += meld_chip_bonus(s.kind);
        for &tid in &s.tile_ids {
            if let Some(t) = tile_by_id(tiles, tid) {
                if !tile_is_debuffed(t, tile_debuffs) {
                    chips += t.point_value() as i32;
                }
            }
        }
    }
    let all_yaku = detect_yaku_with_wind(tiles, sets, None, original_tiles);
    let visible_yaku: Vec<YakuKind> = if available_yaku.is_empty() {
        all_yaku
    } else {
        all_yaku
            .into_iter()
            .filter(|y| available_yaku.contains(y))
            .collect()
    };
    let mut mult: f64 = 1.0;
    for y in &visible_yaku {
        mult += y.mult_bonus();
    }
    let estimated_total = combine(chips, mult);
    ScorePreview {
        chips,
        mult,
        detected_yaku: visible_yaku,
        estimated_total,
    }
}
