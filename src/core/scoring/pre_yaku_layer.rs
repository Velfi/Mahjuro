//! Meld bonuses, early relic chips/mults, talismans, flowers, and Dragon Echo —
//! everything applied before [`super::dora_yaku_layer`] (Dora, yaku, structure depth).

use crate::core::hand::MeldKind;
use crate::core::relic::{RelicId, ScoreContext};
use crate::core::tile::{Suit, Tile};

use super::layer_input::{PreYakuLayerOpts, ScoringLayerInput, ScoringLayerOut};
use super::push_steps::{push_chips, push_gold, push_mult};
use super::tea_bonus::{
    tea_harmony_chips, tea_purity_mult, tea_respect_chips, tea_tranquility_chips,
};
use super::{meld_chip_bonus, tile_by_id, tile_is_debuffed};

#[inline]
fn effective_point_value(t: &Tile, ctx: &ScoreContext<'_>) -> i32 {
    if tile_is_debuffed(t, ctx.tiles.debuffs) {
        0
    } else {
        t.point_value() as i32
    }
}

pub(crate) fn apply_pre_yaku_scoring(
    input: &ScoringLayerInput<'_>,
    out: ScoringLayerOut<'_>,
    opts: PreYakuLayerOpts<'_>,
) {
    let ScoringLayerInput {
        ctx,
        tiles,
        sets,
        eff,
    } = *input;
    let ScoringLayerOut { chips, mult, steps } = out;
    let PreYakuLayerOpts {
        pair_double,
        has_triplet_boost,
        flower_gold,
    } = opts;
    let has_sequence_surge = eff.has(ctx.relic.roster, RelicId::SequenceSurge);
    let has_pair_power = eff.has(ctx.relic.roster, RelicId::PairPower);
    let has_honor_fury = eff.has(ctx.relic.roster, RelicId::HonorFury);

    if eff.has(ctx.relic.roster, RelicId::Snowball) {
        let stacks = ctx
            .relic
            .counters
            .get(&RelicId::Snowball)
            .copied()
            .unwrap_or(0);
        let bonus = crate::core::relic::snowball_score_chips(stacks);
        if bonus > 0 {
            push_chips(steps, chips, *mult, "Snowball", bonus);
        }
    }

    for s in sets {
        match s.kind {
            MeldKind::Triplet | MeldKind::Kong if has_triplet_boost => {
                push_chips(steps, chips, *mult, "Triplet Boost", 40);
            }
            MeldKind::Sequence if has_sequence_surge => {
                push_chips(steps, chips, *mult, "Sequence Surge", 25);
            }
            MeldKind::Pair if has_pair_power => {
                push_chips(steps, chips, *mult, "Pair Power", 30);
            }
            _ => {}
        }
        if matches!(s.kind, MeldKind::Kong) && eff.has(ctx.relic.roster, RelicId::KongsBlessing) {
            push_chips(steps, chips, *mult, "Kong's Blessing", 120);
        }

        if has_honor_fury {
            let honor_count = s
                .tile_ids
                .iter()
                .filter_map(|id| tile_by_id(tiles, *id))
                .filter(|t| !tile_is_debuffed(t, ctx.tiles.debuffs))
                .filter(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
                .count() as i32;
            if honor_count > 0 {
                push_chips(steps, chips, *mult, "Honor Fury", 28 * honor_count);
            }
        }
    }

    let has_jade_serpent = eff.has(ctx.relic.roster, RelicId::JadeSerpent);
    let has_ruby_serpent = eff.has(ctx.relic.roster, RelicId::RubySerpent);
    let has_lapis_serpent = eff.has(ctx.relic.roster, RelicId::LapisSerpent);
    let has_edge_runner = eff.has(ctx.relic.roster, RelicId::EdgeRunner);
    let has_low_tide = eff.has(ctx.relic.roster, RelicId::LowTide);
    let has_high_tide = eff.has(ctx.relic.roster, RelicId::HighTide);
    if has_jade_serpent
        || has_ruby_serpent
        || has_lapis_serpent
        || has_edge_runner
        || has_low_tide
        || has_high_tide
    {
        for s in sets {
            for &tid in &s.tile_ids {
                let Some(t) = tile_by_id(tiles, tid) else {
                    continue;
                };
                if tile_is_debuffed(t, ctx.tiles.debuffs) {
                    continue;
                }
                if has_jade_serpent && t.suit == Suit::Bamboos {
                    push_chips(steps, chips, *mult, "Jade Serpent", 8);
                }
                if has_ruby_serpent && t.suit == Suit::Characters {
                    push_chips(steps, chips, *mult, "Ruby Serpent", 8);
                }
                if has_lapis_serpent && t.suit == Suit::Dots {
                    push_chips(steps, chips, *mult, "Lapis Serpent", 8);
                }
                if has_edge_runner
                    && matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Dots)
                    && (t.rank == 1 || t.rank == 9)
                {
                    push_chips(steps, chips, *mult, "Edge Runner", 12);
                }
                if has_low_tide
                    && matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Dots)
                    && t.rank <= 3
                {
                    push_chips(steps, chips, *mult, "Low Tide", 6);
                }
                if has_high_tide
                    && matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Dots)
                    && t.rank >= 7
                {
                    push_chips(steps, chips, *mult, "High Tide", 6);
                }
            }
        }
    }

    if pair_double {
        let pair_count = sets.iter().filter(|s| s.kind == MeldKind::Pair).count() as i32;
        if pair_count > 0 {
            push_chips(steps, chips, *mult, "Pair Double (rule)", 30 * pair_count);
        }
    }

    if eff.has(ctx.relic.roster, RelicId::TilePolisher) {
        let bonus = ctx
            .relic
            .counters
            .get(&RelicId::TilePolisher)
            .copied()
            .unwrap_or(0);
        if bonus > 0 {
            push_chips(steps, chips, *mult, "Tile Polisher", bonus);
        }
    }

    if eff.has(ctx.relic.roster, RelicId::LastBreath)
        && ctx.round.is_final_play
        && ctx.structure.is_some()
    {
        let mut retrigger_chips = 0i32;
        for s in sets {
            for &tid in &s.tile_ids {
                if let Some(t) = tile_by_id(tiles, tid) {
                    retrigger_chips += effective_point_value(t, ctx);
                }
            }
        }
        if retrigger_chips > 0 {
            push_chips(steps, chips, *mult, "Last Breath", retrigger_chips);
        }
    }

    if eff.has(ctx.relic.roster, RelicId::Geese) {
        let mut retrigger = 0i32;
        let mut remaining = 5;
        'outer: for s in sets {
            for &tid in &s.tile_ids {
                if remaining == 0 {
                    break 'outer;
                }
                if let Some(t) = tile_by_id(tiles, tid) {
                    retrigger += effective_point_value(t, ctx);
                    remaining -= 1;
                }
            }
        }
        if retrigger > 0 {
            push_chips(steps, chips, *mult, "Geese", retrigger);
        }
    }

    if eff.has(ctx.relic.roster, RelicId::VoiceOfThePeople) {
        let mut retrigger = 0i32;
        for s in sets {
            for &tid in &s.tile_ids {
                if let Some(t) = tile_by_id(tiles, tid)
                    && matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Dots)
                    && t.rank <= 4
                {
                    retrigger += effective_point_value(t, ctx);
                }
            }
        }
        if retrigger > 0 {
            push_chips(steps, chips, *mult, "Voice of the People", retrigger);
        }
    }

    if eff.has(ctx.relic.roster, RelicId::VoiceOfTheElite) {
        let mut retrigger = 0i32;
        for s in sets {
            for &tid in &s.tile_ids {
                if let Some(t) = tile_by_id(tiles, tid)
                    && matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Dots)
                    && t.rank >= 6
                {
                    retrigger += effective_point_value(t, ctx);
                }
            }
        }
        if retrigger > 0 {
            push_chips(steps, chips, *mult, "Voice of the Elite", retrigger);
        }
    }

    if eff.has(ctx.relic.roster, RelicId::RustlingGooseEgg) {
        let charges = ctx
            .relic
            .counters
            .get(&RelicId::RustlingGooseEgg)
            .copied()
            .unwrap_or(0);
        if charges > 0 {
            let mut retrigger = 0i32;
            for s in sets {
                for &tid in &s.tile_ids {
                    if let Some(t) = tile_by_id(tiles, tid) {
                        retrigger += effective_point_value(t, ctx);
                    }
                }
            }
            if retrigger > 0 {
                push_chips(steps, chips, *mult, "XXXL Egg", retrigger);
            }
        }
    }

    if eff.has(ctx.relic.roster, RelicId::TeaCeremony) {
        let phase = ctx
            .relic
            .counters
            .get(&RelicId::TeaCeremony)
            .copied()
            .unwrap_or(0)
            .clamp(0, 3);
        match phase {
            0 => {
                if let Some(c) = tea_harmony_chips(tiles) {
                    push_chips(steps, chips, *mult, "Tea Ceremony · Harmony", c);
                }
            }
            1 => {
                if let Some(c) = tea_respect_chips(tiles) {
                    push_chips(steps, chips, *mult, "Tea Ceremony · Respect", c);
                }
            }
            2 => {
                if let Some(m) = tea_purity_mult(tiles) {
                    push_mult(steps, *chips, mult, "Tea Ceremony · Purity", m);
                }
            }
            3 => {
                if let Some(c) = tea_tranquility_chips(sets) {
                    push_chips(steps, chips, *mult, "Tea Ceremony · Tranquility", c);
                }
            }
            _ => {}
        }
    }

    if eff.has(ctx.relic.roster, RelicId::GhostHand) && !ctx.tiles.hand_for_ghost.is_empty() {
        use rustc_hash::FxHashSet;
        let scored_ids: FxHashSet<u32> = sets
            .iter()
            .flat_map(|s| s.tile_ids.iter().copied())
            .collect();
        let mut ghost_chips = 0i32;
        for t in ctx.tiles.hand_for_ghost {
            if scored_ids.contains(&t.id) {
                continue;
            }
            ghost_chips += effective_point_value(t, ctx);
        }
        if ghost_chips > 0 {
            push_chips(steps, chips, *mult, "Ghost Hand", ghost_chips);
        }
    }

    if eff.has(ctx.relic.roster, RelicId::RiverRunner) {
        let bonus = ctx
            .relic
            .counters
            .get(&RelicId::RiverRunner)
            .copied()
            .unwrap_or(0);
        if bonus > 0 {
            push_chips(steps, chips, *mult, "River Runner", bonus);
        }
    }

    if eff.has(ctx.relic.roster, RelicId::MeltingIce) {
        let ice_chips = ctx
            .relic
            .counters
            .get(&RelicId::MeltingIce)
            .copied()
            .unwrap_or(0);
        if ice_chips > 0 {
            push_chips(steps, chips, *mult, "Melting Ice", ice_chips);
        }
    }

    if eff.has(ctx.relic.roster, RelicId::Taotie) {
        // Permanent +80 base, plus the accumulated chip bonus from every
        // honor the mask has devoured this run. The accumulator grows in
        // `apply_scored_melds` at cash-in time (each devoured honor adds
        // 20 chips and the tile is permanently removed from the wall).
        push_chips(steps, chips, *mult, "Taotie", 80);
        let devoured_chips = ctx
            .relic
            .counters
            .get(&RelicId::Taotie)
            .copied()
            .unwrap_or(0);
        if devoured_chips > 0 {
            push_chips(steps, chips, *mult, "Taotie (devoured)", devoured_chips);
        }
    }

    {
        use crate::core::tile::TileEnhancement;
        /// Flat chips per scored meld that includes ≥1 Pearl-stamped tile (Polychrome’s chip twin).
        const PEARL_CHIPS_PER_MELD: i32 = 100;
        let mut pearl_melds = 0i32;
        let mut gilded_gold = 0i32;
        let mut polychrome_melds = 0i32;
        for s in sets {
            let mut meld_has_pearl = false;
            let mut meld_has_polychrome = false;
            for &tid in &s.tile_ids {
                let Some(t) = tile_by_id(tiles, tid) else {
                    continue;
                };
                if tile_is_debuffed(t, ctx.tiles.debuffs) {
                    continue;
                }
                let Some(enh) = t.enhancement else { continue };
                match enh {
                    TileEnhancement::Pearl => meld_has_pearl = true,
                    TileEnhancement::Gilded => {
                        if !matches!(s.kind, MeldKind::Pair) {
                            gilded_gold += 1;
                        }
                    }
                    TileEnhancement::Polychrome => meld_has_polychrome = true,
                }
            }
            if meld_has_pearl {
                pearl_melds += 1;
            }
            if meld_has_polychrome {
                polychrome_melds += 1;
            }
        }
        for _ in 0..pearl_melds {
            push_chips(steps, chips, *mult, "Pearl Talisman", PEARL_CHIPS_PER_MELD);
        }
        if gilded_gold > 0 {
            push_gold(
                steps,
                flower_gold,
                *chips,
                *mult,
                "Gilded Talisman",
                gilded_gold,
            );
        }
        for _ in 0..polychrome_melds {
            let delta = *mult * 0.2;
            push_mult(steps, *chips, mult, "Polychrome Talisman", delta);
        }
    }

    {
        let garden_keeper = eff.count(ctx.relic.roster, RelicId::GardenKeeper);
        if garden_keeper > 0 {
            const CHIPS_PER_FLOWER: i32 = 25;
            let delta = CHIPS_PER_FLOWER * garden_keeper as i32;
            for s in sets {
                for &tid in &s.tile_ids {
                    let Some(t) = tile_by_id(tiles, tid) else {
                        continue;
                    };
                    if t.suit != Suit::Flower || tile_is_debuffed(t, ctx.tiles.debuffs) {
                        continue;
                    }
                    push_chips(steps, chips, *mult, "Garden Keeper", delta);
                }
            }
        }
    }

    if eff.has(ctx.relic.roster, RelicId::Hanami) {
        for s in sets {
            for &tid in &s.tile_ids {
                let Some(t) = tile_by_id(tiles, tid) else {
                    continue;
                };
                if t.suit != Suit::Flower || tile_is_debuffed(t, ctx.tiles.debuffs) {
                    continue;
                }
                push_gold(steps, flower_gold, *chips, *mult, "Hanami", 3);
            }
        }
    }

    if eff.has(ctx.relic.roster, RelicId::DragonEcho) {
        let set_bases: Vec<i32> = sets
            .iter()
            .map(|s| {
                let mut c = meld_chip_bonus(s.kind);
                for &tid in &s.tile_ids {
                    if let Some(t) = tile_by_id(tiles, tid) {
                        c += effective_point_value(t, ctx);
                    }
                }
                c
            })
            .collect();
        let is_dragon_trip: Vec<bool> = sets
            .iter()
            .map(|s| {
                if !matches!(s.kind, MeldKind::Triplet | MeldKind::Kong) {
                    return false;
                }
                s.tile_ids
                    .first()
                    .and_then(|id| tile_by_id(tiles, *id))
                    .is_some_and(|t| t.suit == Suit::Dragon)
            })
            .collect();

        for (i, &is_echoer) in is_dragon_trip.iter().enumerate() {
            if !is_echoer {
                continue;
            }
            let echo: i32 = set_bases
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i && !is_dragon_trip[j])
                .map(|(_, b)| *b)
                .sum();
            if echo > 0 {
                push_chips(steps, chips, *mult, "Dragon Echo", echo);
            }
        }
    }
}
