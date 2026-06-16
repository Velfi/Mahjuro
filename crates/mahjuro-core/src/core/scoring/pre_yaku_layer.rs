//! Early relic chips/mults, talismans, flowers, and Dragon Echo —
//! everything applied before [`super::dora_yaku_layer`] (Dora and yaku).

use crate::core::hand::{DetectedMeld, MeldKind};
use crate::core::relic::{RelicId, ScoreContext};
use crate::core::scoring::push_steps::push_yen;
use crate::core::tile::{Suit, Tile};
use crate::core::yaku::structure_would_score_chicken_hand;

use super::layer_input::{PreYakuLayerOpts, ScoringLayerInput, ScoringLayerOut};
use super::push_steps::{push_fu, push_han};
use super::tea_bonus::{
    tea_harmony_fu, tea_purity_han, tea_respect_fu, tea_tranquility_fu,
};
use super::{tile_by_id, tile_is_debuffed};

pub const TRIPLET_BOOST_FU: i32 = 50;
pub const PLAIN_DEALING_FU_PER_SIMPLE_TILE: i32 = 18;
pub const EVEN_KEEL_FU_PER_TILE: i32 = 12;

#[inline]
fn effective_point_value(t: &Tile, ctx: &ScoreContext<'_>) -> i32 {
    if tile_is_debuffed(t, ctx.tiles.debuffs) {
        0
    } else {
        t.point_value() as i32
    }
}

fn meld_base_fu(s: &DetectedMeld, tiles: &[Tile], ctx: &ScoreContext<'_>) -> i32 {
    s.tile_ids
        .iter()
        .filter_map(|&tid| tile_by_id(tiles, tid))
        .map(|t| effective_point_value(t, ctx))
        .sum()
}

#[inline]
fn tile_is_simple_number(t: &Tile) -> bool {
    matches!(t.suit, Suit::Manzu | Suit::Souzu | Suit::Pinzu) && (2..=8).contains(&t.rank)
}

#[inline]
fn tile_is_even_keel_rank(t: &Tile) -> bool {
    matches!(t.suit, Suit::Manzu | Suit::Souzu | Suit::Pinzu) && (4..=6).contains(&t.rank)
}

fn meld_all_number_ranks(s: &DetectedMeld, tiles: &[Tile], pred: impl Fn(u8) -> bool) -> bool {
    !s.tile_ids.is_empty()
        && s.tile_ids.iter().all(|&tid| {
            tile_by_id(tiles, tid).is_some_and(|t| {
                matches!(t.suit, Suit::Souzu | Suit::Manzu | Suit::Pinzu) && pred(t.rank)
            })
        })
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
    let ScoringLayerOut { fu, han, steps } = out;
    let PreYakuLayerOpts {
        has_triplet_boost,
        flower_yen,
        original_tiles,
    } = opts;
    let has_sequence_surge = eff.has(ctx.relic.roster, RelicId::SequenceSurge);
    let has_pair_power = eff.has(ctx.relic.roster, RelicId::PairPower);
    let has_honor_fury = eff.has(ctx.relic.roster, RelicId::HonorFury);
    let has_plain_dealing = eff.has(ctx.relic.roster, RelicId::PlainDealing);
    let has_even_keel = eff.has(ctx.relic.roster, RelicId::EvenKeel);

    if eff.has(ctx.relic.roster, RelicId::Snowball) {
        let stacks = ctx
            .relic
            .counters
            .get(&RelicId::Snowball)
            .copied()
            .unwrap_or(0);
        let bonus = crate::core::relic::snowball_score_fu(stacks);
        if bonus > 0 {
            push_fu(steps, fu, *han, "Snowball", bonus);
        }
    }

    for s in sets {
        match s.kind {
            MeldKind::Triplet | MeldKind::Kong if has_triplet_boost => {
                push_fu(steps, fu, *han, "Triplet Boost", TRIPLET_BOOST_FU);
            }
            MeldKind::Sequence if has_sequence_surge => {
                push_fu(steps, fu, *han, "Sequence Surge", 50);
            }
            MeldKind::Pair if has_pair_power => {
                push_fu(steps, fu, *han, "Pair Power", 45);
            }
            _ => {}
        }
        if matches!(s.kind, MeldKind::Kong) && eff.has(ctx.relic.roster, RelicId::KongsBlessing) {
            push_fu(steps, fu, *han, "Kong's Blessing", 180);
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
                push_fu(steps, fu, *han, "Honor Fury", 42 * honor_count);
            }
        }
        if has_plain_dealing {
            let simple_count = s
                .tile_ids
                .iter()
                .filter_map(|id| tile_by_id(tiles, *id))
                .filter(|t| !tile_is_debuffed(t, ctx.tiles.debuffs))
                .filter(|t| tile_is_simple_number(t))
                .count() as i32;
            if simple_count > 0 {
                push_fu(
                    steps,
                    fu,
                    *han,
                    "Plain Dealing",
                    PLAIN_DEALING_FU_PER_SIMPLE_TILE * simple_count,
                );
            }
        }
        if has_even_keel {
            let mid_count = s
                .tile_ids
                .iter()
                .filter_map(|id| tile_by_id(tiles, *id))
                .filter(|t| !tile_is_debuffed(t, ctx.tiles.debuffs))
                .filter(|t| tile_is_even_keel_rank(t))
                .count() as i32;
            if mid_count > 0 {
                push_fu(
                    steps,
                    fu,
                    *han,
                    "Even Keel",
                    EVEN_KEEL_FU_PER_TILE * mid_count,
                );
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
                if has_jade_serpent && t.suit == Suit::Souzu {
                    push_fu(steps, fu, *han, "Jade Serpent", 12);
                }
                if has_ruby_serpent && t.suit == Suit::Manzu {
                    push_fu(steps, fu, *han, "Ruby Serpent", 12);
                }
                if has_lapis_serpent && t.suit == Suit::Pinzu {
                    push_fu(steps, fu, *han, "Lapis Serpent", 12);
                }
                if has_edge_runner
                    && matches!(t.suit, Suit::Souzu | Suit::Manzu | Suit::Pinzu)
                    && (t.rank == 1 || t.rank == 9)
                {
                    push_fu(steps, fu, *han, "Edge Runner", 18);
                }
                if has_low_tide
                    && matches!(t.suit, Suit::Souzu | Suit::Manzu | Suit::Pinzu)
                    && t.rank <= 3
                {
                    push_fu(steps, fu, *han, "Low Tide", 10);
                }
                if has_high_tide
                    && matches!(t.suit, Suit::Souzu | Suit::Manzu | Suit::Pinzu)
                    && t.rank >= 7
                {
                    push_fu(steps, fu, *han, "High Tide", 10);
                }
            }
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
            push_fu(steps, fu, *han, "Tile Polisher", bonus);
        }
    }

    if eff.has(ctx.relic.roster, RelicId::LastBreath)
        && ctx.round.is_final_play
        && ctx.structure.is_some()
    {
        let retrigger_chips: i32 = sets.iter().map(|s| meld_base_fu(s, tiles, ctx)).sum();
        if retrigger_chips > 0 {
            push_fu(steps, fu, *han, "Last Breath", retrigger_chips);
        }
    }

    if eff.has(ctx.relic.roster, RelicId::Geese) {
        let retrigger: i32 = sets
            .iter()
            .take(5)
            .map(|s| meld_base_fu(s, tiles, ctx))
            .sum();
        if retrigger > 0 {
            push_fu(steps, fu, *han, "Geese", retrigger);
        }
    }

    if eff.has(ctx.relic.roster, RelicId::VoiceOfThePeople) {
        let retrigger: i32 = sets
            .iter()
            .filter(|s| meld_all_number_ranks(s, tiles, |rank| rank <= 4))
            .map(|s| meld_base_fu(s, tiles, ctx))
            .sum();
        if retrigger > 0 {
            push_fu(steps, fu, *han, "Voice of the People", retrigger);
        }
    }

    if eff.has(ctx.relic.roster, RelicId::VoiceOfTheElite) {
        let retrigger: i32 = sets
            .iter()
            .filter(|s| meld_all_number_ranks(s, tiles, |rank| rank >= 6))
            .map(|s| meld_base_fu(s, tiles, ctx))
            .sum();
        if retrigger > 0 {
            push_fu(steps, fu, *han, "Voice of the Elite", retrigger);
        }
    }

    if eff.has(ctx.relic.roster, RelicId::XxxlEgg) {
        let charges = ctx
            .relic
            .counters
            .get(&RelicId::XxxlEgg)
            .copied()
            .unwrap_or(0);
        if charges > 0 {
            let retrigger: i32 = sets.iter().map(|s| meld_base_fu(s, tiles, ctx)).sum();
            if retrigger > 0 {
                push_fu(steps, fu, *han, "XXXL Egg", retrigger);
            }
        }
    }

    if eff.has(ctx.relic.roster, RelicId::EasterEgg)
        && structure_would_score_chicken_hand(
            ctx.structure,
            tiles,
            sets,
            ctx.round.round_wind,
            ctx.round.bonus_round_wind,
            original_tiles,
            &ctx.pattern.available_yaku,
        )
    {
        let retrigger: i32 = sets.iter().map(|s| meld_base_fu(s, tiles, ctx)).sum();
        if retrigger > 0 {
            push_fu(steps, fu, *han, "Easter Egg", retrigger);
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
                if let Some(c) = tea_harmony_fu(tiles) {
                    push_fu(steps, fu, *han, "Tea Ceremony · Harmony", c);
                }
            }
            1 => {
                if let Some(c) = tea_respect_fu(tiles) {
                    push_fu(steps, fu, *han, "Tea Ceremony · Respect", c);
                }
            }
            2 => {
                if let Some(m) = tea_purity_han(tiles) {
                    push_han(steps, *fu, han, "Tea Ceremony · Purity", m);
                }
            }
            3 => {
                if let Some(c) = tea_tranquility_fu(sets) {
                    push_fu(steps, fu, *han, "Tea Ceremony · Tranquility", c);
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
            push_fu(steps, fu, *han, "Ghost Hand", ghost_chips);
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
            push_fu(steps, fu, *han, "River Runner", bonus);
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
            push_fu(steps, fu, *han, "Melting Ice", ice_chips);
        }
    }

    if eff.has(ctx.relic.roster, RelicId::Taotie) {
        // Permanent +80 base, plus the accumulated chip bonus from every
        // honor the mask has devoured this run. The accumulator grows in
        // `apply_scored_melds` at cash-in time (each devoured honor adds
        // 20 Fu and the tile is permanently removed from the wall).
        push_fu(
            steps,
            fu,
            *han,
            "Taotie",
            crate::core::relic::TAOTIE_BASE_CHIPS,
        );
        let devoured_chips = ctx
            .relic
            .counters
            .get(&RelicId::Taotie)
            .copied()
            .unwrap_or(0);
        if devoured_chips > 0 {
            push_fu(steps, fu, *han, "Taotie (devoured)", devoured_chips);
        }
    }

    {
        use crate::core::tile::TileEnhancement;
        /// Flat Fu per scored meld that includes >=1 Pearl-stamped tile.
        const PEARL_CHIPS_PER_MELD: i32 = 100;
        /// Additive Han bonus per scored Polychrome-stamped tile.
        const POLYCHROME_MULT_PER_TILE: f64 = 0.25;
        let mut pearl_melds = 0i32;
        let mut gilded_yen = 0i32;
        let mut polychrome_tiles = 0i32;
        for s in sets {
            let mut meld_has_pearl = false;
            let mut meld_has_gilded = false;
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
                    TileEnhancement::Gilded => meld_has_gilded = true,
                    TileEnhancement::Polychrome => polychrome_tiles += 1,
                }
            }
            if meld_has_pearl {
                pearl_melds += 1;
            }
            if meld_has_gilded {
                gilded_yen += 1;
            }
        }
        for _ in 0..pearl_melds {
            push_fu(steps, fu, *han, "Pearl Talisman", PEARL_CHIPS_PER_MELD);
        }
        if gilded_yen > 0 {
            push_yen(
                steps,
                flower_yen,
                *fu,
                *han,
                "Gilded Talisman",
                gilded_yen,
            );
        }
        for _ in 0..polychrome_tiles {
            push_han(
                steps,
                *fu,
                han,
                "Polychrome Talisman",
                POLYCHROME_MULT_PER_TILE,
            );
        }
    }

    {
        let garden_keeper = eff.count(ctx.relic.roster, RelicId::GardenKeeper);
        if garden_keeper > 0 {
            const CHIPS_PER_FLOWER: i32 = 40;
            let delta = CHIPS_PER_FLOWER * garden_keeper as i32;
            for s in sets {
                for &tid in &s.tile_ids {
                    let Some(t) = tile_by_id(tiles, tid) else {
                        continue;
                    };
                    if t.suit != Suit::Flower || tile_is_debuffed(t, ctx.tiles.debuffs) {
                        continue;
                    }
                    push_fu(steps, fu, *han, "Garden Keeper", delta);
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
                push_yen(steps, flower_yen, *fu, *han, "Hanami", 3);
            }
        }
    }

    if eff.has(ctx.relic.roster, RelicId::AncestorEcho) && !sets.is_empty() {
        let best = sets
            .iter()
            .map(|s| meld_base_fu(s, tiles, ctx))
            .max()
            .unwrap_or(0);
        if best > 0 {
            push_fu(steps, fu, *han, "Ancestor Echo", best);
        }
    }

    if eff.has(ctx.relic.roster, RelicId::DragonEcho) {
        for s in sets {
            let is_dragon_meld = !s.tile_ids.is_empty()
                && s.tile_ids
                    .iter()
                    .all(|&tid| tile_by_id(tiles, tid).is_some_and(|t| t.suit == Suit::Dragon));
            if is_dragon_meld {
                let base = meld_base_fu(s, tiles, ctx);
                if base > 0 {
                    push_fu(steps, fu, *han, "Dragon Echo", base);
                }
            }
        }
    }
}
