//! Stairway decimation — permanently remove 10 wall tiles (5 player, 5 House).

use rand::seq::IndexedRandom;

use crate::core::deck::{OVERFLOW_TILE_ID_BASE, Wall};
use crate::core::relic::RelicId;
use crate::core::tile::Tile;
use crate::core::tile_pack::PACK_TILE_ID_BASE;
use crate::game::event_bus::{EventBus, GameEvent};
use crate::game::run::RunState;

pub const PLAYER_PICKS: usize = 5;
pub const HOUSE_PICKS: usize = 5;

/// Next-round wall tiles (shop preview composition).
pub fn decimation_preview_tiles(run: &RunState) -> Vec<Tile> {
    let overflow = run.relics.has(RelicId::StrengthInNumbers);
    Wall::preview_composition(
        &run.removed_tile_ids,
        &run.tile_packs,
        &run.tile_enhancements,
        overflow,
        &run.joker_extra_faces,
    )
    .all_tiles()
    .to_vec()
}

/// Standard wall copies eligible for decimation (includes flowers; excludes packs / overflow / joker).
pub fn is_decimation_eligible(tile: &Tile) -> bool {
    let id = tile.id;
    if id >= PACK_TILE_ID_BASE && id < OVERFLOW_TILE_ID_BASE {
        return false;
    }
    if id >= OVERFLOW_TILE_ID_BASE {
        return false;
    }
    true
}

pub fn decimation_eligible_tiles(run: &RunState) -> Vec<Tile> {
    decimation_preview_tiles(run)
        .into_iter()
        .filter(is_decimation_eligible)
        .collect()
}

/// IDs the House may claim after the player locks in `player_ids`.
pub fn decimation_house_pool(run: &RunState, player_ids: &[u32]) -> Vec<u32> {
    let player_set: rustc_hash::FxHashSet<u32> = player_ids.iter().copied().collect();
    decimation_eligible_tiles(run)
        .into_iter()
        .map(|t| t.id)
        .filter(|id| !player_set.contains(id) && !run.removed_tile_ids.contains(id))
        .collect()
}

pub fn pick_house_tiles(pool: &[u32], rng: &mut impl rand::Rng) -> Vec<u32> {
    let take = HOUSE_PICKS.min(pool.len());
    if take == 0 {
        return Vec::new();
    }
    pool.sample(rng, take).copied().collect()
}

pub fn can_seal_decimation(run: &RunState, player_ids: &[u32]) -> bool {
    player_ids.len() == PLAYER_PICKS && decimation_house_pool(run, player_ids).len() >= HOUSE_PICKS
}

pub fn apply_decimation(
    run: &mut RunState,
    player_ids: [u32; PLAYER_PICKS],
    house_ids: [u32; HOUSE_PICKS],
    bus: &mut EventBus,
    emit_destroyed_event: bool,
) {
    for id in player_ids.iter().chain(house_ids.iter()) {
        run.removed_tile_ids.insert(*id);
        run.tile_enhancements.remove(id);
    }
    run.decimations_used = run.decimations_used.saturating_add(1);
    if emit_destroyed_event {
        bus.push(GameEvent::TilesDestroyed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tile::{Suit, Tile};
    use crate::core::tile_pack::PACK_TILE_ID_BASE;
    use crate::game::event_bus::EventBus;
    use crate::game::run::RunState;
    use crate::persistence::TileMaterial;

    fn fresh_run() -> RunState {
        RunState::new_with_material(TileMaterial::Bamboo)
    }

    #[test]
    fn eligibility_includes_flowers_excludes_pack_and_overflow() {
        let flower = Tile::new(Suit::Flower, 1, 136);
        let man = Tile::new(Suit::Manzu, 1, 0);
        let pack = Tile::new(Suit::Manzu, 5, PACK_TILE_ID_BASE);
        let overflow = Tile::new(Suit::Manzu, 5, OVERFLOW_TILE_ID_BASE);
        assert!(is_decimation_eligible(&flower));
        assert!(is_decimation_eligible(&man));
        assert!(!is_decimation_eligible(&pack));
        assert!(!is_decimation_eligible(&overflow));
    }

    #[test]
    fn house_pool_excludes_player_picks_and_removed() {
        let mut run = fresh_run();
        let eligible = decimation_eligible_tiles(&run);
        assert!(eligible.len() >= 10);
        let player: Vec<u32> = eligible.iter().take(5).map(|t| t.id).collect();
        run.removed_tile_ids.insert(eligible[9].id);
        let pool = decimation_house_pool(&run, &player);
        for id in &player {
            assert!(!pool.contains(id));
        }
        assert!(!pool.contains(&eligible[9].id));
        assert!(pool.len() >= 5);
    }

    #[test]
    fn apply_decimation_removes_ten_from_preview() {
        let mut run = fresh_run();
        let mut bus = EventBus::default();
        let before = decimation_preview_tiles(&run).len();
        let eligible = decimation_eligible_tiles(&run);
        let player: [u32; 5] = eligible.iter().take(5).map(|t| t.id).collect::<Vec<_>>().try_into().unwrap();
        let pool = decimation_house_pool(&run, &player);
        let house: [u32; 5] = pool.iter().take(5).copied().collect::<Vec<_>>().try_into().unwrap();
        apply_decimation(&mut run, player, house, &mut bus, true);
        let after = decimation_preview_tiles(&run).len();
        assert_eq!(before - after, 10);
        assert_eq!(run.decimations_used, 1);
        assert!(
            bus.queue
                .iter()
                .any(|e| matches!(e, GameEvent::TilesDestroyed))
        );
    }

    #[test]
    fn player_can_mark_four_of_same_face() {
        let run = fresh_run();
        let face = (Suit::Manzu, 5u8);
        let same_face: Vec<u32> = decimation_eligible_tiles(&run)
            .into_iter()
            .filter(|t| t.suit == face.0 && t.rank == face.1)
            .map(|t| t.id)
            .collect();
        assert_eq!(same_face.len(), 4);
        let player: Vec<u32> = same_face.clone();
        let pool = decimation_house_pool(&run, &player);
        for id in &same_face {
            assert!(!pool.contains(id));
        }
        assert!(pool.len() >= 5);
    }

    #[test]
    fn pick_house_tiles_takes_five_when_available() {
        let pool: Vec<u32> = (100..200).collect();
        let picked = pick_house_tiles(&pool, &mut rand::rng());
        assert_eq!(picked.len(), 5);
    }
}
