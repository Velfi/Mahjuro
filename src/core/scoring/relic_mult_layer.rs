//! Relic-driven **mult** and late **chip** lines applied after yaku
//! ([`super::dora_yaku_layer`]). Earlier relic chip/mult effects live in [`super::pre_yaku_layer`].
//!
//! Keeping this in one module makes the main pipeline easier to read and gives a
//! single place to audit "post-yaku" balance.

use crate::core::hand::MeldKind;
use crate::core::relic::{RelicId, TURTLE_SHELL_CHIPS};
use crate::core::tile::Suit;

use super::layer_input::{PostYakuRelicLayerOpts, ScoringLayerInput, ScoringLayerOut};
use super::push_steps::{push_chips, push_mult};
use super::tea_bonus::{
    tea_harmony_chips, tea_purity_mult, tea_respect_chips, tea_tranquility_chips,
};
use super::{tile_by_id, tile_is_debuffed};

pub(crate) fn apply_post_yaku_relic_modifiers(
    input: &ScoringLayerInput<'_>,
    out: ScoringLayerOut<'_>,
    opts: PostYakuRelicLayerOpts<'_>,
) {
    let ScoringLayerInput {
        ctx,
        tiles,
        sets,
        eff,
    } = *input;
    let ScoringLayerOut { chips, mult, steps } = out;
    let PostYakuRelicLayerOpts {
        honor_triple,
        no_seq_bonus,
        has_triplet_boost,
        detected_yaku,
    } = opts;
    let has = |id: RelicId| eff.has(ctx.relic.roster, id);
    let count = |id: RelicId| eff.count(ctx.relic.roster, id);

    if has(RelicId::DragonRage) {
        for s in sets {
            if !matches!(s.kind, MeldKind::Triplet | MeldKind::Kong) {
                continue;
            }
            let is_dragon = s
                .tile_ids
                .first()
                .and_then(|id| tile_by_id(tiles, *id))
                .is_some_and(|t| t.suit == Suit::Dragon);
            if is_dragon {
                push_mult(steps, *chips, mult, "Dragon Rage", 8.0);
            }
        }
    }

    if has(RelicId::WhiteDragonsHush) {
        for s in sets {
            if s.kind != MeldKind::Pair {
                continue;
            }
            let is_white_dragon = s
                .tile_ids
                .first()
                .and_then(|id| tile_by_id(tiles, *id))
                .is_some_and(|t| t.suit == Suit::Dragon && t.rank == 3);
            if is_white_dragon {
                push_mult(steps, *chips, mult, "White Dragon's Hush", 6.0);
            }
        }
    }

    if has(RelicId::SequenceSurge) {
        let seq_count = sets.iter().filter(|s| s.kind == MeldKind::Sequence).count() as i32;
        if seq_count > 0 {
            push_mult(
                steps,
                *chips,
                mult,
                "Sequence Surge",
                0.8 * seq_count as f64,
            );
        }
    }

    if has(RelicId::PairPower) {
        let pair_count = sets.iter().filter(|s| s.kind == MeldKind::Pair).count() as i32;
        if pair_count > 0 {
            push_mult(steps, *chips, mult, "Pair Power", 1.5 * pair_count as f64);
        }
    }

    if has(RelicId::KanDrum) {
        let kong_count = sets.iter().filter(|s| s.kind == MeldKind::Kong).count() as i32;
        if kong_count > 0 {
            push_mult(steps, *chips, mult, "Kan Drum", 6.0 * kong_count as f64);
        }
    }

    if has(RelicId::KongsBlessing) {
        let kong_count = sets.iter().filter(|s| s.kind == MeldKind::Kong).count() as i32;
        if kong_count > 0 {
            push_mult(
                steps,
                *chips,
                mult,
                "Kong's Blessing",
                3.0 * kong_count as f64,
            );
        }
    }

    if has_triplet_boost {
        let trip_count = sets
            .iter()
            .filter(|s| matches!(s.kind, MeldKind::Triplet | MeldKind::Kong))
            .count() as i32;
        if trip_count > 0 {
            push_mult(
                steps,
                *chips,
                mult,
                "Triplet Boost",
                0.35 * trip_count as f64,
            );
        }
    }

    if has(RelicId::WindReader) {
        let mut round_winds = Vec::new();
        if let Some(w) = ctx.round.round_wind {
            round_winds.push(w);
        }
        if let Some(w) = ctx.round.bonus_round_wind
            && !round_winds.contains(&w)
        {
            round_winds.push(w);
        }
        for s in sets {
            if !matches!(s.kind, MeldKind::Triplet | MeldKind::Kong) {
                continue;
            }
            let is_round_wind = s
                .tile_ids
                .first()
                .and_then(|id| tile_by_id(tiles, *id))
                .is_some_and(|t| t.suit == Suit::Wind && round_winds.contains(&t.rank));
            if is_round_wind {
                push_mult(steps, *chips, mult, "Windreader", 9.0);
            }
        }
    }

    if honor_triple {
        for s in sets {
            if !matches!(s.kind, MeldKind::Triplet | MeldKind::Kong) {
                continue;
            }
            let is_honor = s
                .tile_ids
                .first()
                .and_then(|id| tile_by_id(tiles, *id))
                .is_some_and(|t| matches!(t.suit, Suit::Wind | Suit::Dragon));
            if is_honor {
                push_mult(steps, *chips, mult, "Honor Triple (rule)", 3.0);
            }
        }
    }

    if no_seq_bonus && !sets.iter().any(|s| s.kind == MeldKind::Sequence) {
        push_mult(steps, *chips, mult, "No-Seq Bonus (rule)", 3.0);
    }

    if has(RelicId::Ikebana) {
        let flower_count = sets
            .iter()
            .flat_map(|s| &s.tile_ids)
            .filter(|id| {
                tile_by_id(tiles, **id).is_some_and(|t| {
                    t.suit == Suit::Flower && !tile_is_debuffed(t, ctx.tiles.debuffs)
                })
            })
            .count();
        if flower_count >= 2 {
            push_mult(steps, *chips, mult, "Ikebana", 9.0);
        }
    }

    if has(RelicId::LuckySeven) {
        let count7 = sets
            .iter()
            .flat_map(|s| &s.tile_ids)
            .filter_map(|id| tile_by_id(tiles, *id))
            .filter(|t| !tile_is_debuffed(t, ctx.tiles.debuffs))
            .filter(|t| matches!(t.suit, Suit::Souzu | Suit::Manzu | Suit::Pinzu) && t.rank == 7)
            .count();
        if count7 > 0 {
            push_mult(steps, *chips, mult, "Lucky Seven", 3.0 * count7 as f64);
        }
    }

    for _ in 0..count(RelicId::PaperLantern) {
        push_mult(steps, *chips, mult, "Paper Lantern", 6.0);
    }

    if has(RelicId::MultiplierMaster) {
        let n = ctx.relic.roster.len() as f64;
        if n > 0.0 {
            push_mult(steps, *chips, mult, "Multiplier Master", n);
        }
    }

    if has(RelicId::ChainReaction) && ctx.round.scored_last_turn {
        push_mult(steps, *chips, mult, "Chain Reaction", 6.0);
    }

    if has(RelicId::ClosedGate) {
        let all_terminal_or_honor = sets
            .iter()
            .flat_map(|s| &s.tile_ids)
            .filter_map(|id| tile_by_id(tiles, *id))
            .all(|t| {
                matches!(t.suit, Suit::Wind | Suit::Dragon)
                    || (matches!(t.suit, Suit::Souzu | Suit::Manzu | Suit::Pinzu)
                        && (t.rank == 1 || t.rank == 9))
            });
        if all_terminal_or_honor {
            push_mult(steps, *chips, mult, "Closed Gate", 6.0);
        }
    }

    if has(RelicId::GoldenEngine) {
        let bonus = crate::core::relic::golden_engine_mult_bonus(ctx.economy.gold) as f64;
        if bonus > 0.0 {
            push_mult(steps, *chips, mult, "Golden Engine", bonus);
        }
    }

    if has(RelicId::MonarchButterfly) {
        let excess = ctx
            .relic
            .counters
            .get(&RelicId::MonarchButterfly)
            .copied()
            .unwrap_or(0);
        let bonus = crate::core::relic::monarch_butterfly_bonus_chips(excess);
        if bonus > 0 {
            push_chips(steps, chips, *mult, "Monarch Butterfly", bonus);
        }
    }

    if has(RelicId::Momentum) && ctx.round.plays_used > 0 {
        push_mult(
            steps,
            *chips,
            mult,
            "Momentum",
            1.0 * ctx.round.plays_used as f64,
        );
    }

    if has(RelicId::Minimalist) && sets.len() == 1 && sets[0].kind == MeldKind::Pair {
        push_mult(steps, *chips, mult, "Minimalist", 6.0);
    }

    if has(RelicId::TurtleShell) && ctx.economy.gold > 0 {
        push_chips(steps, chips, *mult, "Turtle Shell", TURTLE_SHELL_CHIPS);
    }

    if has(RelicId::SilkThread) {
        let thread_mult = ctx
            .relic
            .counters
            .get(&RelicId::SilkThread)
            .copied()
            .unwrap_or(0);
        if thread_mult > 0 {
            push_mult(steps, *chips, mult, "Silk Thread", thread_mult as f64 / 8.0);
        }
    }

    if has(RelicId::SilkMoth) {
        push_mult(steps, *chips, mult, "Silk Moth", 3.0);
    }

    for _ in 0..count(RelicId::EulersNumber) {
        push_mult(steps, *chips, mult, "Euler's Number", std::f64::consts::E);
    }

    for _ in 0..count(RelicId::PiConstant) {
        push_mult(steps, *chips, mult, "Pi", std::f64::consts::PI);
    }

    if has(RelicId::Humility) {
        let streak = ctx
            .relic
            .counters
            .get(&RelicId::Humility)
            .copied()
            .unwrap_or(0);
        if streak > 0 {
            push_mult(steps, *chips, mult, "Humility", 0.75 * streak as f64);
        }
    }

    if has(RelicId::Obsession) {
        let rounds = ctx
            .relic
            .counters
            .get(&RelicId::Obsession)
            .copied()
            .unwrap_or(0);
        if rounds > 0 {
            push_mult(steps, *chips, mult, "Obsession", 0.5 * rounds as f64);
        }
    }

    if has(RelicId::Bonfire) {
        let sold = ctx
            .relic
            .counters
            .get(&RelicId::Bonfire)
            .copied()
            .unwrap_or(0);
        if sold > 0 {
            push_mult(steps, *chips, mult, "Bonfire", 0.6 * sold as f64);
        }
    }

    if has(RelicId::Temperance) {
        let stacks = ctx
            .relic
            .counters
            .get(&RelicId::Temperance)
            .copied()
            .unwrap_or(0);
        if stacks > 0 {
            push_mult(steps, *chips, mult, "Temperance", stacks as f64 / 8.0);
        }
    }

    if has(RelicId::Chastity) && !tiles.is_empty() {
        let all_unenhanced = tiles.iter().all(|t| t.enhancement.is_none());
        if all_unenhanced {
            push_mult(steps, *chips, mult, "Chastity", 3.0);
        }
    }

    if has(RelicId::Kintsugi) {
        let broken = ctx
            .relic
            .counters
            .get(&RelicId::Kintsugi)
            .copied()
            .unwrap_or(0);
        if broken > 0 {
            push_mult(steps, *chips, mult, "Kintsugi", broken as f64);
        }
    }

    if has(RelicId::Rakuware) {
        if let Some(c) = tea_harmony_chips(tiles) {
            push_chips(steps, chips, *mult, "Rakuware · Harmony", c);
        }
        if let Some(c) = tea_respect_chips(tiles) {
            push_chips(steps, chips, *mult, "Rakuware · Respect", c);
        }
        if let Some(m) = tea_purity_mult(tiles) {
            push_mult(steps, *chips, mult, "Rakuware · Purity", m);
        }
        if let Some(c) = tea_tranquility_chips(sets) {
            push_chips(steps, chips, *mult, "Rakuware · Tranquility", c);
        }
    }

    if has(RelicId::SolitarySage) {
        let empty = ctx
            .relic
            .roster
            .max_slots
            .saturating_sub(ctx.relic.roster.active.len());
        if empty > 0 {
            push_mult(steps, *chips, mult, "Solitary Sage", 2.5 * empty as f64);
        }
    }

    if has(RelicId::CurioCabinet) {
        let bonus: u32 = ctx
            .relic
            .roster
            .active
            .iter()
            .copied()
            .filter(|&id| id != RelicId::CurioCabinet)
            .map(|id| crate::core::relic::relic_sell_price_live(id, &ctx.relic.counters))
            .sum();
        if bonus > 0 {
            push_mult(steps, *chips, mult, "Curio Cabinet", bonus as f64);
        }
    }

    if has(RelicId::LotusBloom) {
        let blooms = ctx
            .relic
            .counters
            .get(&RelicId::LotusBloom)
            .copied()
            .unwrap_or(0);
        if blooms > 0 {
            push_mult(steps, *chips, mult, "Lotus Bloom", 0.75 * blooms as f64);
        }
    }

    if has(RelicId::WallWeaver) {
        let overflow_extras = if eff.has(ctx.relic.roster, RelicId::StrengthInNumbers) {
            68
        } else {
            0
        };
        let extra_added = ctx
            .relic
            .counters
            .get(&RelicId::WallWeaver)
            .copied()
            .unwrap_or(0)
            .max(0);
        let excess = overflow_extras + extra_added;
        if excess > 0 {
            push_mult(steps, *chips, mult, "Wall Weaver", 0.35 * excess as f64);
        }
    }

    if has(RelicId::Heirloom) {
        let bosses = ctx
            .relic
            .counters
            .get(&RelicId::Heirloom)
            .copied()
            .unwrap_or(0)
            .max(0);
        if bosses > 0 {
            push_mult(steps, *chips, mult, "Heirloom", bosses as f64);
        }
    }

    if has(RelicId::Tourist) {
        let mut seen = [false; 6];
        for s in sets {
            for &tid in &s.tile_ids {
                let Some(t) = tile_by_id(tiles, tid) else {
                    continue;
                };
                if tile_is_debuffed(t, ctx.tiles.debuffs) {
                    continue;
                }
                let idx = match t.suit {
                    Suit::Manzu => 0,
                    Suit::Souzu => 1,
                    Suit::Pinzu => 2,
                    Suit::Wind => 3,
                    Suit::Dragon => 4,
                    Suit::Flower => 5,
                    Suit::Season => continue,
                };
                seen[idx] = true;
            }
        }
        let distinct = seen.iter().filter(|b| **b).count();
        if distinct > 0 {
            push_mult(steps, *chips, mult, "Tourist", 4.0 * distinct as f64);
        }
    }

    if has(RelicId::CrackedTile) {
        use rand::RngExt;
        let mut rng = rand::rng();
        let bonus: f64 = rng.random_range(0.0..=12.0);
        if bonus > 0.0 {
            push_mult(
                steps,
                *chips,
                mult,
                "Cracked Tile",
                (bonus * 10.0).floor() / 10.0,
            );
        }
    }

    if has(RelicId::HungryGhost) {
        let perm_mult = ctx
            .relic
            .counters
            .get(&RelicId::HungryGhost)
            .copied()
            .unwrap_or(0);
        if perm_mult > 0 {
            push_mult(steps, *chips, mult, "Hungry Ghost", perm_mult as f64 / 10.0);
        }
    }

    if has(RelicId::CrownOfPatterns) && !detected_yaku.is_empty() {
        let distinct = detected_yaku.len() as f64;
        push_mult(steps, *chips, mult, "Crown of Patterns", 4.0 * distinct);
    }

    if has(RelicId::WayOfPurity) {
        let numbered_suits: Vec<Suit> = sets
            .iter()
            .flat_map(|s| &s.tile_ids)
            .filter_map(|id| tile_by_id(tiles, *id))
            .map(|t| t.suit)
            .filter(|s| matches!(s, Suit::Souzu | Suit::Manzu | Suit::Pinzu))
            .collect();
        if !numbered_suits.is_empty() {
            let first = numbered_suits[0];
            let all_same = numbered_suits.iter().all(|&s| s == first)
                && sets
                    .iter()
                    .flat_map(|s| &s.tile_ids)
                    .filter_map(|id| tile_by_id(tiles, *id))
                    .all(|t| matches!(t.suit, Suit::Souzu | Suit::Manzu | Suit::Pinzu));
            if all_same {
                let delta = *mult * 2.5;
                push_mult(steps, *chips, mult, "Way of Purity", delta);
            }
        }
    }

    if has(RelicId::WayOfPairs) && !sets.is_empty() && sets.iter().all(|s| s.kind == MeldKind::Pair)
    {
        let delta = *mult * 1.5;
        push_mult(steps, *chips, mult, "Way of Pairs", delta);
    }

    if has(RelicId::WayOfTriplets)
        && !sets.is_empty()
        && sets
            .iter()
            .all(|s| matches!(s.kind, MeldKind::Triplet | MeldKind::Kong))
    {
        let delta = *mult * 2.5;
        push_mult(steps, *chips, mult, "Way of Triplets", delta);
    }

    if has(RelicId::WayOfSequences)
        && !sets.is_empty()
        && sets.iter().all(|s| s.kind == MeldKind::Sequence)
    {
        let delta = *mult * 1.5;
        push_mult(steps, *chips, mult, "Way of Sequences", delta);
    }

    for _ in 0..count(RelicId::StoneLantern) {
        let delta = *mult * 2.5;
        push_mult(steps, *chips, mult, "Stone Lantern", delta);
    }

    for _ in 0..count(RelicId::GlassCannon) {
        let delta = *mult * 6.0;
        push_mult(steps, *chips, mult, "Glass Cannon", delta);
    }
}
