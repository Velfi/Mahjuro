//! Read model for the Wall Ledger overlay — grouped tile supply with drawn/undrawn state.

use crate::core::deck::Wall;
use crate::core::relic::RelicId;
use crate::core::tile::{Suit, Tile, cmp_sort_order};
use crate::core::tile_pack::PACK_TILE_ID_BASE;
use crate::game::run::RunState;

/// Whether the ledger reflects the live draw pile or a shop preview of the next round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallLedgerMode {
    Live,
    ShopPreview,
}

#[derive(Clone, Debug)]
pub struct WallTileEntry {
    pub tile: Tile,
    pub drawn: bool,
}

#[derive(Clone, Debug)]
pub struct WallLedgerFaceGroup {
    pub suit: Suit,
    pub rank: u8,
    pub copies: Vec<WallTileEntry>,
}

#[derive(Clone, Debug)]
pub struct WallLedgerReadModel {
    pub mode: WallLedgerMode,
    pub standard_groups: Vec<WallLedgerFaceGroup>,
    pub pack_groups: Vec<WallLedgerFaceGroup>,
    pub remaining: usize,
    pub total: usize,
    pub subtitle: String,
}

fn is_pack_tile_id(id: u32) -> bool {
    id >= PACK_TILE_ID_BASE && id < crate::core::deck::OVERFLOW_TILE_ID_BASE
}

fn group_tiles(tiles: &[(Tile, bool)]) -> (Vec<WallLedgerFaceGroup>, Vec<WallLedgerFaceGroup>) {
    use std::collections::BTreeMap;

    let mut standard: BTreeMap<(Suit, u8), Vec<WallTileEntry>> = BTreeMap::new();
    let mut pack: BTreeMap<(Suit, u8), Vec<WallTileEntry>> = BTreeMap::new();

    for &(tile, drawn) in tiles {
        let entry = WallTileEntry { tile, drawn };
        let key = (tile.suit, tile.rank);
        if is_pack_tile_id(tile.id) {
            pack.entry(key).or_default().push(entry);
        } else {
            standard.entry(key).or_default().push(entry);
        }
    }

    let to_groups = |map: BTreeMap<(Suit, u8), Vec<WallTileEntry>>| {
        map.into_iter()
            .map(|((suit, rank), mut copies)| {
                copies.sort_by(|a, b| cmp_sort_order(&a.tile, &b.tile));
                WallLedgerFaceGroup {
                    suit,
                    rank,
                    copies,
                }
            })
            .collect()
    };

    (to_groups(standard), to_groups(pack))
}

fn live_entries(wall: &Wall) -> Vec<(Tile, bool)> {
    let cursor = wall.draw_cursor();
    wall.all_tiles()
        .iter()
        .enumerate()
        .map(|(i, &tile)| (tile, i < cursor))
        .collect()
}

pub fn read_wall_ledger(run: &RunState, mode: WallLedgerMode) -> WallLedgerReadModel {
    match mode {
        WallLedgerMode::Live => {
            let entries = live_entries(&run.wall);
            let remaining = run.wall.remaining();
            let total = entries.len();
            let (standard_groups, pack_groups) = group_tiles(&entries);
            let subtitle = format!("{remaining} yet to draw · {total} in the round");
            WallLedgerReadModel {
                mode,
                standard_groups,
                pack_groups,
                remaining,
                total,
                subtitle,
            }
        }
        WallLedgerMode::ShopPreview => {
            let overflow = run.relics.has(RelicId::StrengthInNumbers);
            let preview = Wall::preview_composition(
                &run.removed_tile_ids,
                &run.tile_packs,
                &run.tile_enhancements,
                overflow,
                &run.joker_extra_faces,
            );
            let entries: Vec<(Tile, bool)> = preview
                .all_tiles()
                .iter()
                .copied()
                .map(|tile| (tile, false))
                .collect();
            let total = entries.len();
            let (standard_groups, pack_groups) = group_tiles(&entries);
            WallLedgerReadModel {
                mode,
                standard_groups,
                pack_groups,
                remaining: total,
                total,
                subtitle: format!("{total} tiles · next round supply"),
            }
        }
    }
}

/// Total tile count for the next round — shown on the shop wall HUD.
pub fn shop_wall_hud_count(run: &RunState) -> usize {
    read_wall_ledger(run, WallLedgerMode::ShopPreview).total
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::deck::{Wall, build_wall};

    #[test]
    fn live_ledger_marks_drawn_by_cursor() {
        let mut wall = Wall::from_unshuffled(build_wall());
        wall.draw();
        wall.draw();
        let mut run = RunState::new_with_material(crate::persistence::TileMaterial::Bamboo);
        run.wall = wall;
        let ledger = read_wall_ledger(&run, WallLedgerMode::Live);
        assert_eq!(ledger.remaining, 138);
        assert_eq!(ledger.total, 140);
        let drawn: usize = ledger
            .standard_groups
            .iter()
            .chain(&ledger.pack_groups)
            .flat_map(|g| g.copies.iter())
            .filter(|c| c.drawn)
            .count();
        assert_eq!(drawn, 2);
    }

    #[test]
    fn shop_preview_all_vivid() {
        let run = RunState::new_with_material(crate::persistence::TileMaterial::Bamboo);
        let ledger = read_wall_ledger(&run, WallLedgerMode::ShopPreview);
        assert_eq!(ledger.remaining, ledger.total);
        assert!(
            ledger
                .standard_groups
                .iter()
                .chain(&ledger.pack_groups)
                .flat_map(|g| &g.copies)
                .all(|c| !c.drawn)
        );
    }
}
