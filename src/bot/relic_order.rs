//! Inventory ordering for position-sensitive relics (Mirror Tile, Shadow Hand, Hungry Ghost).
//!
//! Mirror Tile copies the relic to its right; Shadow Hand copies the leftmost relic
//! (when that slot is not Shadow Hand). Hungry Ghost destroys the relic to its right
//! at round start and banks permanent mult from the victim's sell value.

use std::collections::BTreeMap;

use crate::core::relic::{RelicId, RelicState, relic_sell_price_live};
use crate::game::run::RunState;

use super::ShopMarginalBase;

#[inline]
fn roster_is_order_sensitive(relics: &RelicState) -> bool {
    relics.has(RelicId::MirrorTile)
        || relics.has(RelicId::ShadowHand)
        || relics.has(RelicId::HungryGhost)
}

/// Headless analogue of shop relic reordering: hill-climb swaps until inventory order
/// maximizes sampled best-play value (including Hungry Ghost's next-round feed).
pub(crate) fn bot_optimize_relic_order(run: &mut RunState) {
    if !roster_is_order_sensitive(&run.relics) {
        return;
    }
    let base = ShopMarginalBase::new(run);
    bot_optimize_relic_order_with_base(run, &base);
}

pub(crate) fn bot_optimize_relic_order_with_base(run: &mut RunState, base: &ShopMarginalBase) {
    if !roster_is_order_sensitive(&run.relics) {
        return;
    }
    let counters = run.relic_counters.clone();
    hill_climb_relic_order(run, &run.relics.clone(), &counters, base);
}

/// Optimize a hypothetical roster (and counters after Hungry Ghost feed) for shop valuation.
pub(crate) fn bot_prepare_hypothetical_roster(
    run: &RunState,
    mut relics: RelicState,
    base: &ShopMarginalBase,
) -> (RelicState, BTreeMap<RelicId, i32>) {
    if !roster_is_order_sensitive(&relics) {
        return (relics, run.relic_counters.clone());
    }
    let mut counters = run.relic_counters.clone();
    hill_climb_relic_order_on(&mut relics, &mut counters, run, base);
    (relics, counters)
}

fn hill_climb_relic_order(
    run: &mut RunState,
    start: &RelicState,
    start_counters: &BTreeMap<RelicId, i32>,
    base: &ShopMarginalBase,
) {
    let mut relics = start.clone();
    let mut counters = start_counters.clone();
    hill_climb_relic_order_on(&mut relics, &mut counters, run, base);
    run.relics = relics;
}

fn hill_climb_relic_order_on(
    relics: &mut RelicState,
    counters: &mut BTreeMap<RelicId, i32>,
    run: &RunState,
    base: &ShopMarginalBase,
) {
    let n = relics.active.len();
    if n <= 1 {
        return;
    }

    seed_relic_order_heuristics(relics);

    let mut best = roster_eval_score(run, relics, counters, base);
    let mut improved = true;
    while improved {
        improved = false;
        for i in 0..n {
            for j in i + 1..n {
                relics.active.swap(i, j);
                let score = roster_eval_score(run, relics, counters, base);
                if score > best {
                    best = score;
                    improved = true;
                } else {
                    relics.active.swap(i, j);
                }
            }
        }
    }
}

/// Cheap fixes when relics were appended to inventory tail (shop default).
fn seed_relic_order_heuristics(relics: &mut RelicState) {
    let n = relics.active.len();
    if n <= 1 {
        return;
    }

    if relics.active.first() == Some(&RelicId::ShadowHand) {
        relics.swap_relics(0, n - 1);
    }

    if let Some(mirror_idx) = relics
        .active
        .iter()
        .position(|&id| id == RelicId::MirrorTile)
        && mirror_idx + 1 >= n && mirror_idx > 0 {
            relics.swap_relics(mirror_idx, mirror_idx - 1);
        }

    if let Some(hg_idx) = relics
        .active
        .iter()
        .position(|&id| id == RelicId::HungryGhost)
        && hg_idx + 1 >= n && hg_idx > 0 {
            relics.swap_relics(hg_idx, hg_idx - 1);
        }
}

/// Score a pre-round roster: simulate Hungry Ghost feed, then average best-play lift.
fn roster_eval_score(
    run: &RunState,
    relics: &RelicState,
    counters: &BTreeMap<RelicId, i32>,
    base: &ShopMarginalBase,
) -> i64 {
    let mut post_relics = relics.clone();
    let mut post_counters = counters.clone();
    simulate_hungry_ghost_feed(&mut post_relics, &mut post_counters);

    let mut sum = 0i64;
    for hand in &base.hands {
        sum += super::best_play_score_for_hand(
            run,
            hand,
            Some(&post_relics),
            None,
            Some(&post_counters),
        ) as i64;
    }
    sum
}

/// Match [`RunState::feed_hungry_ghosts_at_round_start`] for bot valuation (no debuff check).
fn simulate_hungry_ghost_feed(relics: &mut RelicState, counters: &mut BTreeMap<RelicId, i32>) {
    loop {
        let Some(hg_idx) = relics
            .active
            .iter()
            .position(|&id| id == RelicId::HungryGhost)
        else {
            break;
        };
        if relics.is_debuffed(RelicId::HungryGhost) {
            break;
        }
        if hg_idx + 1 >= relics.active.len() {
            break;
        }
        let victim_id = relics.active[hg_idx + 1];
        let victim_value = relic_sell_price_live(victim_id, counters) as i32;
        relics.active.remove(hg_idx + 1);
        *counters.entry(RelicId::HungryGhost).or_insert(0) += victim_value * 10;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::relic::RelicId;
    use crate::core::tile::{Suit, Tile};
    use crate::game::run::RunState;

    fn scoring_fixture(relics: Vec<RelicId>) -> RunState {
        let mut run = RunState::new_demo();
        *run.hand_mut() = vec![
            Tile::new(Suit::Manzu, 3, 1),
            Tile::new(Suit::Manzu, 3, 2),
            Tile::new(Suit::Manzu, 3, 3),
            Tile::new(Suit::Souzu, 6, 4),
            Tile::new(Suit::Souzu, 6, 5),
            Tile::new(Suit::Souzu, 6, 6),
            Tile::new(Suit::Pinzu, 2, 7),
            Tile::new(Suit::Pinzu, 3, 8),
            Tile::new(Suit::Pinzu, 4, 9),
        ];
        run.hand_mut().sort();
        run.relics.active = relics;
        run
    }

    #[test]
    fn shadow_hand_moves_off_first_slot() {
        let mut run = scoring_fixture(vec![
            RelicId::ShadowHand,
            RelicId::PairPower,
            RelicId::TripletBoost,
        ]);
        bot_optimize_relic_order(&mut run);
        assert_ne!(run.relics.active.first(), Some(&RelicId::ShadowHand));
    }

    #[test]
    fn mirror_tile_placed_before_copy_target() {
        let mut run = scoring_fixture(vec![
            RelicId::TripletBoost,
            RelicId::PairPower,
            RelicId::MirrorTile,
        ]);
        bot_optimize_relic_order(&mut run);
        let mirror_idx = run
            .relics
            .active
            .iter()
            .position(|&id| id == RelicId::MirrorTile)
            .expect("mirror");
        assert!(mirror_idx + 1 < run.relics.active.len());
    }

    #[test]
    fn hungry_ghost_sacrifice_improves_over_default_append() {
        let mut run = scoring_fixture(vec![
            RelicId::PairPower,
            RelicId::TripletBoost,
            RelicId::HungryGhost,
        ]);
        let base = ShopMarginalBase::new(&run);
        let append_score = roster_eval_score(
            &run,
            &run.relics,
            &run.relic_counters,
            &base,
        );
        bot_optimize_relic_order_with_base(&mut run, &base);
        let optimized_score = roster_eval_score(
            &run,
            &run.relics,
            &run.relic_counters,
            &base,
        );
        assert!(optimized_score >= append_score);
        let hg_idx = run
            .relics
            .active
            .iter()
            .position(|&id| id == RelicId::HungryGhost)
            .expect("hungry ghost");
        assert!(hg_idx + 1 < run.relics.active.len());
    }
}
