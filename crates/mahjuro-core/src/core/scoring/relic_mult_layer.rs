//! Relic-driven **han** and late **chip** lines applied after yaku
//! ([`super::dora_yaku_layer`]). Earlier relic chip/mult effects live in [`super::pre_yaku_layer`].
//!
//! Keeping this in one module makes the main pipeline easier to read and gives a
//! single place to audit "post-yaku" balance.

use crate::core::hand::{DetectedMeld, MeldKind};
use crate::core::relic::{RelicId, TURTLE_SHELL_FU};
use crate::core::tile::{Suit, Tile};

use super::layer_input::{PostYakuRelicLayerOpts, ScoringLayerInput, ScoringLayerOut};
use super::push_steps::{push_fu, push_han};
use super::tea_bonus::{tea_harmony_fu, tea_purity_han, tea_respect_fu, tea_tranquility_fu};
use super::{tile_by_id, tile_is_debuffed};

pub const TRIPLET_BOOST_HAN_PER_TRIPLET: f64 = 0.5;
pub const MINIMALIST_FU: i32 = 120;
pub const MINIMALIST_HAN: f64 = 2.0;
pub const OPEN_GATE_HAN: f64 = 6.0;
pub const CHAIN_REACTION_HAN: f64 = 6.0;

fn meld_all_tiles_match(s: &DetectedMeld, tiles: &[Tile], pred: impl Fn(&Tile) -> bool) -> bool {
    !s.tile_ids.is_empty()
        && s.tile_ids
            .iter()
            .all(|&tid| tile_by_id(tiles, tid).is_some_and(|t| pred(t)))
}

fn structure_has_suit_meld(sets: &[DetectedMeld], tiles: &[Tile], suit: Suit) -> bool {
    sets.iter()
        .any(|s| meld_all_tiles_match(s, tiles, |t| t.suit == suit))
}

fn structure_has_dragon_meld(sets: &[DetectedMeld], tiles: &[Tile], rank: u8) -> bool {
    sets.iter()
        .any(|s| meld_all_tiles_match(s, tiles, |t| t.suit == Suit::Dragon && t.rank == rank))
}

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
    let ScoringLayerOut { fu, han, steps } = out;
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
                push_han(steps, *fu, han, "Dragon Rage", 8.0);
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
                push_han(steps, *fu, han, "White Dragon's Hush", 6.0);
            }
        }
    }

    if has(RelicId::SequenceSurge) {
        let seq_count = sets.iter().filter(|s| s.kind == MeldKind::Sequence).count() as i32;
        if seq_count > 0 {
            push_han(steps, *fu, han, "Sequence Surge", 1.25 * seq_count as f64);
        }
    }

    if has(RelicId::PairPower) {
        let pair_count = sets.iter().filter(|s| s.kind == MeldKind::Pair).count() as i32;
        if pair_count > 0 {
            push_han(steps, *fu, han, "Pair Power", 1.25 * pair_count as f64);
        }
    }

    if has(RelicId::KanDrum) {
        let kong_count = sets.iter().filter(|s| s.kind == MeldKind::Kong).count() as i32;
        if kong_count > 0 {
            push_han(steps, *fu, han, "Kan Drum", 8.0 * kong_count as f64);
        }
    }

    if has(RelicId::KongsBlessing) {
        let kong_count = sets.iter().filter(|s| s.kind == MeldKind::Kong).count() as i32;
        if kong_count > 0 {
            push_han(steps, *fu, han, "Kong's Blessing", 3.0 * kong_count as f64);
        }
    }

    if has_triplet_boost {
        let trip_count = sets
            .iter()
            .filter(|s| matches!(s.kind, MeldKind::Triplet | MeldKind::Kong))
            .count() as i32;
        if trip_count > 0 {
            push_han(
                steps,
                *fu,
                han,
                "Triplet Boost",
                TRIPLET_BOOST_HAN_PER_TRIPLET * trip_count as f64,
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
                push_han(steps, *fu, han, "Windreader", 9.0);
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
                push_han(steps, *fu, han, "Honor Triple (rule)", 3.0);
            }
        }
    }

    if no_seq_bonus && !sets.iter().any(|s| s.kind == MeldKind::Sequence) {
        push_han(steps, *fu, han, "No-Seq Bonus (rule)", 3.0);
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
            push_han(steps, *fu, han, "Ikebana", 9.0);
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
            let bonus = (1.5 * count7 as f64).min(9.0);
            push_han(steps, *fu, han, "Lucky Seven", bonus);
        }
    }

    for _ in 0..count(RelicId::PaperLantern) {
        push_han(steps, *fu, han, "Paper Lantern", 4.0);
    }

    if has(RelicId::MultiplierMaster) {
        let n = ctx.relic.roster.len() as f64;
        if n > 0.0 {
            push_han(steps, *fu, han, "Multiplier Master", n * 1.5);
        }
    }

    if has(RelicId::ChainReaction) && ctx.round.scored_last_turn {
        push_han(steps, *fu, han, "Chain Reaction", CHAIN_REACTION_HAN);
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
            push_han(steps, *fu, han, "Closed Gate", 6.0);
        }
    }

    if has(RelicId::OpenGate) {
        let all_simple = sets
            .iter()
            .flat_map(|s| &s.tile_ids)
            .filter_map(|id| tile_by_id(tiles, *id))
            .filter(|t| !t.is_flower())
            .all(|t| {
                matches!(t.suit, Suit::Souzu | Suit::Manzu | Suit::Pinzu)
                    && (2..=8).contains(&t.rank)
            });
        if all_simple {
            push_han(steps, *fu, han, "Open Gate", OPEN_GATE_HAN);
        }
    }

    if has(RelicId::ChowLine) {
        let seq_count = sets.iter().filter(|s| s.kind == MeldKind::Sequence).count();
        if seq_count >= 3 {
            push_han(steps, *fu, han, "Chow Line", 4.0);
        }
    }

    if ctx.structure.is_some() {
        if has(RelicId::BlueTilesWhiteDragon)
            && structure_has_suit_meld(sets, tiles, Suit::Pinzu)
            && structure_has_dragon_meld(sets, tiles, 3)
        {
            push_han(steps, *fu, han, "Blue Tiles White Dragon", 6.0);
        }
        if has(RelicId::GreenTilesGreenDragon)
            && structure_has_suit_meld(sets, tiles, Suit::Souzu)
            && structure_has_dragon_meld(sets, tiles, 2)
        {
            push_han(steps, *fu, han, "Green Tiles Green Dragon", 6.0);
        }
        if has(RelicId::RedTilesRedDragon)
            && structure_has_suit_meld(sets, tiles, Suit::Manzu)
            && structure_has_dragon_meld(sets, tiles, 1)
        {
            push_han(steps, *fu, han, "Red Tiles Red Dragon", 6.0);
        }
    }

    if has(RelicId::GoldenEngine) {
        let bonus = crate::core::relic::golden_engine_han_bonus(ctx.economy.yen) as f64;
        if bonus > 0.0 {
            push_han(steps, *fu, han, "Golden Engine", bonus);
        }
    }

    if has(RelicId::MonarchButterfly) {
        let excess = ctx
            .relic
            .counters
            .get(&RelicId::MonarchButterfly)
            .copied()
            .unwrap_or(0);
        let bonus = crate::core::relic::monarch_butterfly_bonus_fu(excess);
        if bonus > 0 {
            push_fu(steps, fu, *han, "Monarch Butterfly", bonus);
        }
    }

    if has(RelicId::Momentum) && ctx.round.plays_used > 0 {
        push_han(
            steps,
            *fu,
            han,
            "Momentum",
            2.0 * ctx.round.plays_used as f64,
        );
    }

    if has(RelicId::Minimalist) && sets.len() == 1 && sets[0].kind != MeldKind::Single {
        push_fu(steps, fu, *han, "Minimalist", MINIMALIST_FU);
        push_han(steps, *fu, han, "Minimalist", MINIMALIST_HAN);
    }

    if has(RelicId::TurtleShell) && ctx.economy.yen > 0 {
        push_fu(steps, fu, *han, "Turtle Shell", TURTLE_SHELL_FU);
    }

    if has(RelicId::SilkThread) {
        let thread_mult = ctx
            .relic
            .counters
            .get(&RelicId::SilkThread)
            .copied()
            .unwrap_or(0);
        if thread_mult > 0 {
            push_han(steps, *fu, han, "Silk Thread", thread_mult as f64 / 8.0);
        }
    }

    if has(RelicId::SilkMoth) {
        push_han(steps, *fu, han, "Silk Moth", 3.0);
    }

    for _ in 0..count(RelicId::EulersNumber) {
        let delta = *han * (std::f64::consts::E - 1.0);
        push_han(steps, *fu, han, "Euler's Number", delta);
    }

    for _ in 0..count(RelicId::PiConstant) {
        let delta = *han * (std::f64::consts::PI - 1.0);
        push_han(steps, *fu, han, "Pi", delta);
    }

    if has(RelicId::Humility) {
        let streak = ctx
            .relic
            .counters
            .get(&RelicId::Humility)
            .copied()
            .unwrap_or(0);
        if streak > 0 {
            push_han(steps, *fu, han, "Humility", 0.75 * streak as f64);
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
            push_han(steps, *fu, han, "Obsession", 0.5 * rounds as f64);
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
            push_han(steps, *fu, han, "Bonfire", 0.6 * sold as f64);
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
            let bonus = (stacks as f64 / 8.0).min(10.0);
            push_han(steps, *fu, han, "Temperance", bonus);
        }
    }

    if has(RelicId::Chastity) && !tiles.is_empty() {
        let all_unenhanced = tiles.iter().all(|t| t.enhancement.is_none());
        if all_unenhanced {
            push_fu(steps, fu, *han, "Chastity", 80);
            push_han(steps, *fu, han, "Chastity", 1.0);
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
            push_han(steps, *fu, han, "Kintsugi", broken as f64);
        }
    }

    if has(RelicId::Rakuware) {
        if let Some(c) = tea_harmony_fu(tiles) {
            push_fu(steps, fu, *han, "Rakuware · Harmony", c);
        }
        if let Some(c) = tea_respect_fu(tiles) {
            push_fu(steps, fu, *han, "Rakuware · Respect", c);
        }
        if let Some(m) = tea_purity_han(tiles) {
            push_han(steps, *fu, han, "Rakuware · Purity", m);
        }
        if let Some(c) = tea_tranquility_fu(sets) {
            push_fu(steps, fu, *han, "Rakuware · Tranquility", c);
        }
    }

    if has(RelicId::SolitarySage) {
        let empty = ctx
            .relic
            .roster
            .max_slots
            .saturating_sub(ctx.relic.roster.active.len());
        if empty > 0 {
            push_han(steps, *fu, han, "Solitary Sage", 2.5 * empty as f64);
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
            push_han(steps, *fu, han, "Curio Cabinet", (bonus as f64).min(15.0));
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
            push_han(
                steps,
                *fu,
                han,
                "Lotus Bloom",
                (0.75 * blooms as f64).min(12.0),
            );
        }
    }

    if has(RelicId::WallWeaver) {
        let overflow_extras = if eff.has(ctx.relic.roster, RelicId::StrengthInNumbers) {
            crate::core::deck::OVERFLOW_RELIC_EXTRA_TILES as i32
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
            push_han(
                steps,
                *fu,
                han,
                "Wall Weaver",
                (0.35 * excess as f64).min(8.0),
            );
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
            push_han(steps, *fu, han, "Heirloom", (bosses as f64).min(12.0));
        }
    }

    if has(RelicId::Kindling) {
        let total = ctx
            .relic
            .counters
            .get(&RelicId::Kindling)
            .copied()
            .unwrap_or(0)
            .max(0);
        let bonus = crate::core::relic::kindling_han_bonus(total);
        if bonus > 0.0 {
            push_han(steps, *fu, han, "Kindling", bonus);
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
            push_han(steps, *fu, han, "Tourist", 2.0 * distinct as f64);
        }
    }

    if has(RelicId::CrackedTile) {
        use rand::RngExt;
        let mut rng = rand::rng();
        let bonus: f64 = rng.random_range(2.0..=8.0);
        if bonus > 0.0 {
            push_han(
                steps,
                *fu,
                han,
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
            push_han(
                steps,
                *fu,
                han,
                "Hungry Ghost",
                (perm_mult as f64 / 10.0).min(20.0),
            );
        }
    }

    if has(RelicId::CrownOfPatterns) && !detected_yaku.is_empty() {
        let distinct = detected_yaku.len() as f64;
        push_han(steps, *fu, han, "Crown of Patterns", 4.0 * distinct);
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
                push_han(steps, *fu, han, "Way of Purity", 6.0);
            }
        }
    }

    if has(RelicId::WayOfPairs) {
        let pair_count = sets.iter().filter(|s| s.kind == MeldKind::Pair).count();
        if pair_count >= 3 {
            push_han(steps, *fu, han, "Way of Pairs", 6.0);
        }
    }

    if has(RelicId::WayOfTriplets) {
        let triplet_like = sets
            .iter()
            .filter(|s| matches!(s.kind, MeldKind::Triplet | MeldKind::Kong))
            .count();
        if triplet_like >= 2 {
            push_han(steps, *fu, han, "Way of Triplets", 7.0);
        }
    }

    if has(RelicId::WayOfSequences) {
        let seq_count = sets.iter().filter(|s| s.kind == MeldKind::Sequence).count();
        if seq_count >= 2 {
            push_han(steps, *fu, han, "Way of Sequences", 5.0);
        }
    }

    for _ in 0..count(RelicId::StoneLantern) {
        let delta = *han * 2.0;
        push_han(steps, *fu, han, "Stone Lantern", delta);
    }

    for _ in 0..count(RelicId::GlassCannon) {
        push_han(steps, *fu, han, "Glass Cannon", 12.0);
    }
}
