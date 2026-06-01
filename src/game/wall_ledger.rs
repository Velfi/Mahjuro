//! Read model for the Wall Ledger overlay — grouped tile supply with drawn/undrawn state.

use crate::core::deck::Wall;
use crate::core::relic::RelicId;
use crate::core::tile::{Suit, Tile, cmp_sort_order};
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

/// Group every wall tile by `(suit, rank)` so pack / overflow / joker extras stack
/// in the same grid cell as the standard four copies.
fn group_tiles(tiles: &[(Tile, bool)]) -> (Vec<WallLedgerFaceGroup>, Vec<WallLedgerFaceGroup>) {
    use std::collections::BTreeMap;

    let mut by_face: BTreeMap<(Suit, u8), Vec<WallTileEntry>> = BTreeMap::new();

    for &(tile, drawn) in tiles {
        by_face
            .entry((tile.suit, tile.rank))
            .or_default()
            .push(WallTileEntry { tile, drawn });
    }

    let standard_groups = by_face
        .into_iter()
        .map(|((suit, rank), mut copies)| {
            copies.sort_by(|a, b| cmp_sort_order(&a.tile, &b.tile));
            WallLedgerFaceGroup {
                suit,
                rank,
                copies,
            }
        })
        .collect();

    (standard_groups, Vec::new())
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

    #[test]
    fn pack_tiles_merge_into_standard_face_groups() {
        use crate::core::tile_pack::TilePackKind;

        let mut run = RunState::new_with_material(crate::persistence::TileMaterial::Bamboo);
        run.tile_packs.push(TilePackKind::Manzu);
        let ledger = read_wall_ledger(&run, WallLedgerMode::ShopPreview);
        assert!(ledger.pack_groups.is_empty());
        let manzu_copies: usize = ledger
            .standard_groups
            .iter()
            .filter(|g| g.suit == Suit::Manzu)
            .map(|g| g.copies.len())
            .sum();
        assert_eq!(manzu_copies, 44, "36 base manzu + 8 from Manzu pack");
    }
}
