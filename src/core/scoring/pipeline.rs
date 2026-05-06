use crate::core::hand::{DetectedSet, SetKind};
use crate::core::relic::{RelicId, ScoreContext};
use crate::core::rules::RuleModifier;
use crate::core::structure::structure_depth_mult_bonus;
use crate::core::tile::{Suit, Tile};
use crate::core::yaku::{YakuKind, detect_yaku_with_wind};

use super::{
    ScoreBreakdown, ScoreStep, StepKind, combine, describe_set, fmt_mult, meld_chip_bonus,
    tile_by_id, tile_is_debuffed,
};

pub fn score_sets_with_original(
    tiles: &[Tile],
    sets: &[DetectedSet],
    ctx: &ScoreContext<'_>,
    rules: &[RuleModifier],
    original_tiles: &[Tile],
) -> ScoreBreakdown {
    score_sets_inner(tiles, sets, ctx, rules, Some(original_tiles))
}

#[cfg(test)]
pub fn score_sets(
    tiles: &[Tile],
    sets: &[DetectedSet],
    ctx: &ScoreContext<'_>,
    rules: &[RuleModifier],
) -> ScoreBreakdown {
    score_sets_inner(tiles, sets, ctx, rules, None)
}

fn score_sets_inner(
    tiles: &[Tile],
    sets: &[DetectedSet],
    ctx: &ScoreContext<'_>,
    rules: &[RuleModifier],
    original_tiles: Option<&[Tile]>,
) -> ScoreBreakdown {
    let mut steps: Vec<ScoreStep> = Vec::new();
    let mut base_steps: Vec<ScoreStep> = Vec::new();
    let mut chips: i32;
    let mut mult: f64 = 1.0;

    let pair_double = rules.contains(&RuleModifier::PairDoubleScore);
    let honor_triple = rules.contains(&RuleModifier::HonorTripleScore);
    let no_seq_bonus = rules.contains(&RuleModifier::NoSequenceBonus);
    let pairs_zero = rules.contains(&RuleModifier::PairsScoreZero);
    let sequences_halved = rules.contains(&RuleModifier::SequencesHalved);
    let censor_repeats = rules.contains(&RuleModifier::CensorRepeats);
    let effective_point_value = |t: &Tile| -> i32 {
        if tile_is_debuffed(t, ctx.tile_debuffs) {
            0
        } else {
            t.point_value() as i32
        }
    };

    let mirrored: Option<RelicId> = if ctx.relics.has(RelicId::MirrorTile) {
        ctx.relics.relic_after(RelicId::MirrorTile)
    } else {
        None
    };
    let shadowed: Option<RelicId> = if ctx.relics.has(RelicId::ShadowHand) {
        ctx.relics
            .active
            .first()
            .filter(|&&id| id != RelicId::ShadowHand)
            .copied()
    } else {
        None
    };
    let has = |id: RelicId| -> bool {
        ctx.relics.has(id) || mirrored == Some(id) || shadowed == Some(id)
    };
    let count = |id: RelicId| -> u32 {
        let owned = ctx.relics.has(id) as u32;
        let mirror = (mirrored == Some(id)) as u32;
        let shadow = (shadowed == Some(id)) as u32;
        owned + mirror + shadow
    };

    macro_rules! push_chips {
        ($source:expr, $delta:expr) => {{
            let delta: i32 = $delta;
            chips += delta;
            steps.push(ScoreStep {
                source: $source.into(),
                kind: StepKind::Chips,
                tile_ids: Vec::new(),
                running_chips: chips,
                running_mult: mult,
                running_total: combine(chips, mult),
            });
        }};
    }
    macro_rules! push_mult {
        ($source:expr, $delta:expr) => {{
            let delta: f64 = $delta;
            mult += delta;
            steps.push(ScoreStep {
                source: $source.into(),
                kind: StepKind::Mult,
                tile_ids: Vec::new(),
                running_chips: chips,
                running_mult: mult,
                running_total: combine(chips, mult),
            });
        }};
    }
    let mut flower_gold: i32 = 0;
    macro_rules! push_gold {
        ($source:expr, $delta:expr) => {{
            let delta: i32 = $delta;
            flower_gold += delta;
            steps.push(ScoreStep {
                source: $source.into(),
                kind: StepKind::Gold,
                tile_ids: Vec::new(),
                running_chips: chips,
                running_mult: mult,
                running_total: combine(chips, mult),
            });
        }};
    }

    let mut base_chips: i32 = 0;
    for s in sets {
        let mut meld_contrib = meld_chip_bonus(s.kind);
        for &tid in &s.tile_ids {
            if let Some(t) = tile_by_id(tiles, tid) {
                meld_contrib += effective_point_value(t);
            }
        }
        if pairs_zero && s.kind == SetKind::Pair {
            meld_contrib = 0;
        }
        if sequences_halved && s.kind == SetKind::Sequence {
            meld_contrib /= 2;
        }
        base_chips += meld_contrib;
        base_steps.push(ScoreStep {
            source: describe_set(tiles, s),
            kind: StepKind::Chips,
            tile_ids: s.tile_ids.clone(),
            running_chips: base_chips,
            running_mult: 1.0,
            running_total: combine(base_chips, 1.0),
        });
    }
    chips = base_chips;

    let has_triplet_boost = has(RelicId::TripletBoost);
    let has_sequence_surge = has(RelicId::SequenceSurge);
    let has_pair_power = has(RelicId::PairPower);
    let has_honor_fury = has(RelicId::HonorFury);

    for s in sets {
        match s.kind {
            SetKind::Triplet | SetKind::Kong if has_triplet_boost => {
                push_chips!("Triplet Boost", 40)
            }
            SetKind::Sequence if has_sequence_surge => push_chips!("Sequence Surge", 25),
            SetKind::Pair if has_pair_power => push_chips!("Pair Power", 30),
            _ => {}
        }
        if matches!(s.kind, SetKind::Kong) && has(RelicId::KongsBlessing) {
            push_chips!("Kong's Blessing", 120);
        }

        if has_honor_fury {
            let honor_count = s
                .tile_ids
                .iter()
                .filter_map(|id| tile_by_id(tiles, *id))
                .filter(|t| !tile_is_debuffed(t, ctx.tile_debuffs))
                .filter(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
                .count() as i32;
            if honor_count > 0 {
                push_chips!("Honor Fury", 28 * honor_count);
            }
        }
    }

    let has_jade_serpent = has(RelicId::JadeSerpent);
    let has_red_serpent = has(RelicId::RedSerpent);
    let has_blue_serpent = has(RelicId::BlueSerpent);
    let has_edge_runner = has(RelicId::EdgeRunner);
    let has_low_tide = has(RelicId::LowTide);
    let has_high_tide = has(RelicId::HighTide);
    if has_jade_serpent
        || has_red_serpent
        || has_blue_serpent
        || has_edge_runner
        || has_low_tide
        || has_high_tide
    {
        for s in sets {
            for &tid in &s.tile_ids {
                let Some(t) = tile_by_id(tiles, tid) else {
                    continue;
                };
                if tile_is_debuffed(t, ctx.tile_debuffs) {
                    continue;
                }
                if has_jade_serpent && t.suit == Suit::Bamboos {
                    push_chips!("Jade Serpent", 8);
                }
                if has_red_serpent && t.suit == Suit::Characters {
                    push_chips!("Red Serpent", 8);
                }
                if has_blue_serpent && t.suit == Suit::Circles {
                    push_chips!("Blue Serpent", 8);
                }
                if has_edge_runner
                    && matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Circles)
                    && (t.rank == 1 || t.rank == 9)
                {
                    push_chips!("Edge Runner", 12);
                }
                if has_low_tide
                    && matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Circles)
                    && t.rank <= 3
                {
                    push_chips!("Low Tide", 6);
                }
                if has_high_tide
                    && matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Circles)
                    && t.rank >= 7
                {
                    push_chips!("High Tide", 6);
                }
            }
        }
    }

    if pair_double {
        let pair_count = sets.iter().filter(|s| s.kind == SetKind::Pair).count() as i32;
        if pair_count > 0 {
            push_chips!("Pair Double (rule)", 30 * pair_count);
        }
    }

    if has(RelicId::TilePolisher) {
        let bonus = ctx
            .relic_counters
            .get(&RelicId::TilePolisher)
            .copied()
            .unwrap_or(0);
        if bonus > 0 {
            push_chips!("Tile Polisher", bonus);
        }
    }

    if has(RelicId::LastBreath) && ctx.is_final_play && ctx.structure.is_some() {
        let mut retrigger_chips = 0i32;
        for s in sets {
            for &tid in &s.tile_ids {
                if let Some(t) = tile_by_id(tiles, tid) {
                    retrigger_chips += effective_point_value(t);
                }
            }
        }
        if retrigger_chips > 0 {
            push_chips!("Last Breath", retrigger_chips);
        }
    }

    if has(RelicId::Geese) {
        let mut retrigger = 0i32;
        let mut remaining = 5;
        'outer: for s in sets {
            for &tid in &s.tile_ids {
                if remaining == 0 {
                    break 'outer;
                }
                if let Some(t) = tile_by_id(tiles, tid) {
                    retrigger += effective_point_value(t);
                    remaining -= 1;
                }
            }
        }
        if retrigger > 0 {
            push_chips!("Geese", retrigger);
        }
    }

    if has(RelicId::VoiceOfThePeople) {
        let mut retrigger = 0i32;
        for s in sets {
            for &tid in &s.tile_ids {
                if let Some(t) = tile_by_id(tiles, tid)
                    && matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Circles)
                    && t.rank <= 4
                {
                    retrigger += effective_point_value(t);
                }
            }
        }
        if retrigger > 0 {
            push_chips!("Voice of the People", retrigger);
        }
    }

    if has(RelicId::VoiceOfTheElite) {
        let mut retrigger = 0i32;
        for s in sets {
            for &tid in &s.tile_ids {
                if let Some(t) = tile_by_id(tiles, tid)
                    && matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Circles)
                    && t.rank >= 6
                {
                    retrigger += effective_point_value(t);
                }
            }
        }
        if retrigger > 0 {
            push_chips!("Voice of the Elite", retrigger);
        }
    }

    if has(RelicId::TeaCeremony) {
        let charges = ctx
            .relic_counters
            .get(&RelicId::TeaCeremony)
            .copied()
            .unwrap_or(0);
        if charges > 0 {
            let mut retrigger = 0i32;
            for s in sets {
                for &tid in &s.tile_ids {
                    if let Some(t) = tile_by_id(tiles, tid) {
                        retrigger += effective_point_value(t);
                    }
                }
            }
            if retrigger > 0 {
                push_chips!("Tea Ceremony", retrigger);
            }
        }
    }

    if has(RelicId::GhostHand) && ctx.unscored_hand_tiles > 0 {
        push_chips!("Ghost Hand", 2 * ctx.unscored_hand_tiles as i32);
    }

    if has(RelicId::RiverRunner) {
        let bonus = ctx
            .relic_counters
            .get(&RelicId::RiverRunner)
            .copied()
            .unwrap_or(0);
        if bonus > 0 {
            push_chips!("River Runner", bonus);
        }
    }

    if has(RelicId::MeltingIce) {
        let ice_chips = ctx
            .relic_counters
            .get(&RelicId::MeltingIce)
            .copied()
            .unwrap_or(0);
        if ice_chips > 0 {
            push_chips!("Melting Ice", ice_chips);
        }
    }

    if has(RelicId::Taotie) {
        // Permanent +80 base, plus the accumulated chip bonus from every
        // honor the mask has devoured this run. The accumulator grows in
        // `apply_scored_melds` at cash-in time (each devoured honor adds
        // 20 chips and the tile is permanently removed from the wall).
        push_chips!("Taotie", 80);
        let devoured_chips = ctx
            .relic_counters
            .get(&RelicId::Taotie)
            .copied()
            .unwrap_or(0);
        if devoured_chips > 0 {
            push_chips!("Taotie (devoured)", devoured_chips);
        }
    }

    {
        use crate::core::tile::TileEnhancement;
        let mut jade_chips = 0i32;
        let mut pearl_chips = 0i32;
        let mut gilded_gold = 0i32;
        let mut polychrome_melds = 0i32;
        for s in sets {
            let mut meld_has_polychrome = false;
            for &tid in &s.tile_ids {
                let Some(t) = tile_by_id(tiles, tid) else {
                    continue;
                };
                if tile_is_debuffed(t, ctx.tile_debuffs) {
                    continue;
                }
                let Some(enh) = t.enhancement else { continue };
                match enh {
                    TileEnhancement::Jade => {
                        if !matches!(s.kind, SetKind::Pair) {
                            jade_chips += 20;
                        }
                    }
                    TileEnhancement::Pearl => pearl_chips += 25,
                    TileEnhancement::Gilded => {
                        if !matches!(s.kind, SetKind::Pair) {
                            gilded_gold += 1;
                        }
                    }
                    TileEnhancement::Polychrome => meld_has_polychrome = true,
                }
            }
            if meld_has_polychrome {
                polychrome_melds += 1;
            }
        }
        if jade_chips > 0 {
            push_chips!("Jade Talisman", jade_chips);
        }
        if pearl_chips > 0 {
            push_chips!("Pearl Talisman", pearl_chips);
        }
        if gilded_gold > 0 {
            push_gold!("Gilded Talisman", gilded_gold);
        }
        for _ in 0..polychrome_melds {
            let delta = mult * 0.2;
            push_mult!("Polychrome Talisman", delta);
        }
    }

    {
        let meld_count = sets.len() as i32;
        let garden_keeper_passes = count(RelicId::GardenKeeper);
        let hanami = has(RelicId::Hanami);
        for s in sets {
            for &tid in &s.tile_ids {
                let Some(t) = tile_by_id(tiles, tid) else {
                    continue;
                };
                if t.suit != Suit::Flower || tile_is_debuffed(t, ctx.tile_debuffs) {
                    continue;
                }
                macro_rules! score_flower {
                    ($suffix:expr) => {
                        match t.rank {
                            1 => push_chips!(format!("Plum Blossom{}", $suffix), 40),
                            2 => push_mult!(format!("Orchid{}", $suffix), 1.5),
                            3 => push_chips!(format!("Chrysanthemum{}", $suffix), 15 * meld_count),
                            4 => push_gold!(format!("Bamboo{}", $suffix), 4),
                            _ => {}
                        }
                    };
                }
                score_flower!("");
                for _ in 0..garden_keeper_passes {
                    score_flower!(" (Garden Keeper)");
                }
                if hanami {
                    push_gold!("Hanami", 3);
                }
            }
        }
    }

    if has(RelicId::DragonEcho) {
        let set_bases: Vec<i32> = sets
            .iter()
            .map(|s| {
                let mut c = meld_chip_bonus(s.kind);
                for &tid in &s.tile_ids {
                    if let Some(t) = tile_by_id(tiles, tid) {
                        c += effective_point_value(t);
                    }
                }
                c
            })
            .collect();
        let is_dragon_trip: Vec<bool> = sets
            .iter()
            .map(|s| {
                if !matches!(s.kind, SetKind::Triplet | SetKind::Kong) {
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
                push_chips!("Dragon Echo", echo);
            }
        }
    }

    if !ctx.dora_faces.is_empty() {
        let dora_count = tiles
            .iter()
            .filter(|t| !tile_is_debuffed(t, ctx.tile_debuffs))
            .filter(|t| ctx.dora_faces.contains(&(t.suit, t.rank)))
            .count() as i32;
        if dora_count > 0 {
            let delta = 25 * dora_count;
            chips += delta;
            steps.push(ScoreStep {
                source: format!("Dora ×{dora_count}"),
                kind: StepKind::Chips,
                tile_ids: tiles
                    .iter()
                    .filter(|t| !tile_is_debuffed(t, ctx.tile_debuffs))
                    .filter(|t| ctx.dora_faces.contains(&(t.suit, t.rank)))
                    .map(|t| t.id)
                    .collect(),
                running_chips: chips,
                running_mult: mult,
                running_total: combine(chips, mult),
            });
            for _ in 0..count(RelicId::DoraCrown) {
                push_chips!(format!("Dora Crown ×{dora_count}"), 10 * dora_count);
            }
        }
    }

    let all_yaku = detect_yaku_with_wind(tiles, sets, ctx.round_wind, original_tiles);
    let mut detected_yaku: Vec<YakuKind> = if ctx.available_yaku.is_empty() {
        all_yaku
    } else {
        all_yaku
            .into_iter()
            .filter(|y| ctx.available_yaku.contains(y))
            .collect()
    };
    if let Some(st) = &ctx.structure
        && st.inject_chicken_if_no_yaku
        && detected_yaku.is_empty()
    {
        detected_yaku.push(YakuKind::ChickenHand);
    }
    let level_of =
        |y: YakuKind| -> u32 { ctx.yaku_levels.as_ref().map(|m| m.level_of(y)).unwrap_or(1) };
    for yaku in &detected_yaku {
        let level = level_of(*yaku);
        let mut mult_bonus = yaku.mult_bonus_at(level);
        let mut chip_bonus = yaku.chip_bonus_at(level);
        if censor_repeats && ctx.played_yaku_this_round.contains(yaku) {
            chip_bonus = (chip_bonus as f64 * 0.5).floor() as i32;
            mult_bonus *= 0.5;
        }
        if chip_bonus != 0 {
            push_chips!(yaku.name(), chip_bonus);
        }
        push_mult!(yaku.name(), mult_bonus);
    }

    if let Some(st) = &ctx.structure {
        let depth = structure_depth_mult_bonus(st.meld_count);
        if depth > 0.0 {
            push_mult!("Structure depth", depth);
        }
    }

    if has(RelicId::RedDragonRage) {
        for s in sets {
            if !matches!(s.kind, SetKind::Triplet | SetKind::Kong) {
                continue;
            }
            let is_dragon = s
                .tile_ids
                .first()
                .and_then(|id| tile_by_id(tiles, *id))
                .is_some_and(|t| t.suit == Suit::Dragon);
            if is_dragon {
                push_mult!("Red Dragon Rage", 5.0);
            }
        }
    }

    if has(RelicId::WhiteDragonsHush) {
        for s in sets {
            if s.kind != SetKind::Pair {
                continue;
            }
            let is_white_dragon = s
                .tile_ids
                .first()
                .and_then(|id| tile_by_id(tiles, *id))
                .is_some_and(|t| t.suit == Suit::Dragon && t.rank == 3);
            if is_white_dragon {
                push_mult!("White Dragon's Hush", 4.0);
            }
        }
    }

    if has(RelicId::SequenceSurge) {
        let seq_count = sets.iter().filter(|s| s.kind == SetKind::Sequence).count() as i32;
        if seq_count > 0 {
            push_mult!("Sequence Surge", 0.5 * seq_count as f64);
        }
    }

    if has(RelicId::PairPower) {
        let pair_count = sets.iter().filter(|s| s.kind == SetKind::Pair).count() as i32;
        if pair_count > 0 {
            push_mult!("Pair Power", pair_count as f64);
        }
    }

    if has(RelicId::KanDrum) {
        let kong_count = sets.iter().filter(|s| s.kind == SetKind::Kong).count() as i32;
        if kong_count > 0 {
            push_mult!("Kan Drum", 4.0 * kong_count as f64);
        }
    }

    if has(RelicId::KongsBlessing) {
        let kong_count = sets.iter().filter(|s| s.kind == SetKind::Kong).count() as i32;
        if kong_count > 0 {
            push_mult!("Kong's Blessing", 2.0 * kong_count as f64);
        }
    }

    if has_triplet_boost {
        let trip_count = sets
            .iter()
            .filter(|s| matches!(s.kind, SetKind::Triplet | SetKind::Kong))
            .count() as i32;
        if trip_count > 0 {
            push_mult!("Triplet Boost", 0.2 * trip_count as f64);
        }
    }

    if has(RelicId::RoundCompass)
        && let Some(wind) = ctx.round_wind
    {
        for s in sets {
            if !matches!(s.kind, SetKind::Triplet | SetKind::Kong) {
                continue;
            }
            let is_round_wind = s
                .tile_ids
                .first()
                .and_then(|id| tile_by_id(tiles, *id))
                .is_some_and(|t| t.suit == Suit::Wind && t.rank == wind);
            if is_round_wind {
                push_mult!("Round Compass", 6.0);
            }
        }
    }

    if honor_triple {
        for s in sets {
            if !matches!(s.kind, SetKind::Triplet | SetKind::Kong) {
                continue;
            }
            let is_honor = s
                .tile_ids
                .first()
                .and_then(|id| tile_by_id(tiles, *id))
                .is_some_and(|t| matches!(t.suit, Suit::Wind | Suit::Dragon));
            if is_honor {
                push_mult!("Honor Triple (rule)", 3.0);
            }
        }
    }

    if no_seq_bonus && !sets.iter().any(|s| s.kind == SetKind::Sequence) {
        push_mult!("No-Seq Bonus (rule)", 3.0);
    }

    if has(RelicId::Ikebana) {
        let flower_count = sets
            .iter()
            .flat_map(|s| &s.tile_ids)
            .filter(|id| {
                tile_by_id(tiles, **id).is_some_and(|t| {
                    t.suit == Suit::Flower && !tile_is_debuffed(t, ctx.tile_debuffs)
                })
            })
            .count();
        if flower_count >= 2 {
            push_mult!("Ikebana", 6.0);
        }
    }

    if has(RelicId::LuckySeven) {
        let count = sets
            .iter()
            .flat_map(|s| &s.tile_ids)
            .filter_map(|id| tile_by_id(tiles, *id))
            .filter(|t| !tile_is_debuffed(t, ctx.tile_debuffs))
            .filter(|t| {
                matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Circles) && t.rank == 7
            })
            .count();
        if count > 0 {
            push_mult!("Lucky Seven", 1.5 * count as f64);
        }
    }

    for _ in 0..count(RelicId::PaperLantern) {
        push_mult!("Paper Lantern", 4.0);
    }

    if has(RelicId::MultiplierMaster) {
        let bonus = 0.5 * ctx.relics.enabled_len() as f64;
        if bonus > 0.0 {
            push_mult!("Multiplier Master", bonus);
        }
    }

    if has(RelicId::ChainReaction) && ctx.scored_last_turn {
        push_mult!("Chain Reaction", 4.0);
    }

    if has(RelicId::ClosedGate) {
        let all_terminal_or_honor = sets
            .iter()
            .flat_map(|s| &s.tile_ids)
            .filter_map(|id| tile_by_id(tiles, *id))
            .all(|t| {
                matches!(t.suit, Suit::Wind | Suit::Dragon)
                    || (matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Circles)
                        && (t.rank == 1 || t.rank == 9))
            });
        if all_terminal_or_honor {
            push_mult!("Closed Gate", 4.0);
        }
    }

    if has(RelicId::GoldenEngine) {
        let bonus = (ctx.gold.max(0) as f64 / 5.0).floor();
        if bonus > 0.0 {
            push_mult!("Golden Engine", bonus);
        }
    }

    if has(RelicId::Snowball) {
        let bonus = ctx.total_score as f64 / 5000.0;
        if bonus > 0.0 {
            push_mult!("Snowball", bonus);
        }
    }

    if has(RelicId::Momentum) && ctx.plays_used > 0 {
        push_mult!("Momentum", 0.5 * ctx.plays_used as f64);
    }

    if has(RelicId::Minimalist) && sets.len() == 1 && sets[0].kind == SetKind::Pair {
        push_mult!("Minimalist", 4.0);
    }

    if has(RelicId::TurtleShell) && mult < 3.0 {
        push_chips!("Turtle Shell", 50);
    }

    if has(RelicId::SilkThread) {
        let thread_mult = ctx
            .relic_counters
            .get(&RelicId::SilkThread)
            .copied()
            .unwrap_or(0);
        if thread_mult > 0 {
            push_mult!("Silk Thread", thread_mult as f64 / 10.0);
        }
    }

    if has(RelicId::SilkMoth) {
        push_mult!("Silk Moth", 2.0);
    }

    if has(RelicId::Humility) {
        let streak = ctx
            .relic_counters
            .get(&RelicId::Humility)
            .copied()
            .unwrap_or(0);
        if streak > 0 {
            push_mult!("Humility", 0.5 * streak as f64);
        }
    }

    if has(RelicId::Obsession) {
        let rounds = ctx
            .relic_counters
            .get(&RelicId::Obsession)
            .copied()
            .unwrap_or(0);
        if rounds > 0 {
            push_mult!("Obsession", 0.3 * rounds as f64);
        }
    }

    if has(RelicId::Bonfire) {
        let sold = ctx
            .relic_counters
            .get(&RelicId::Bonfire)
            .copied()
            .unwrap_or(0);
        if sold > 0 {
            push_mult!("Bonfire", 0.4 * sold as f64);
        }
    }

    if has(RelicId::Kintsugi) {
        let broken = ctx
            .relic_counters
            .get(&RelicId::Kintsugi)
            .copied()
            .unwrap_or(0);
        if broken > 0 {
            push_mult!("Kintsugi", broken as f64);
        }
    }

    if has(RelicId::SolitarySage) {
        let empty = ctx.relics.max_slots.saturating_sub(ctx.relics.active.len());
        if empty > 0 {
            push_mult!("Solitary Sage", 1.5 * empty as f64);
        }
    }

    if has(RelicId::CurioCabinet) {
        let bonus: u32 = ctx
            .relics
            .active
            .iter()
            .copied()
            .filter(|&id| id != RelicId::CurioCabinet)
            .map(|id| crate::core::relic::relic_sell_price_live(id, &ctx.relic_counters))
            .sum();
        if bonus > 0 {
            push_mult!("Curio Cabinet", bonus as f64);
        }
    }

    if has(RelicId::LotusBloom) {
        let blooms = ctx
            .relic_counters
            .get(&RelicId::LotusBloom)
            .copied()
            .unwrap_or(0);
        if blooms > 0 {
            push_mult!("Lotus Bloom", 0.5 * blooms as f64);
        }
    }

    if has(RelicId::WallWeaver) {
        let overflow_extras = if ctx.relics.has(RelicId::StrengthInNumbers) {
            68
        } else {
            0
        };
        let extra_added = ctx
            .relic_counters
            .get(&RelicId::WallWeaver)
            .copied()
            .unwrap_or(0)
            .max(0);
        let excess = overflow_extras + extra_added;
        if excess > 0 {
            push_mult!("Wall Weaver", 0.2 * excess as f64);
        }
    }

    if has(RelicId::Heirloom) {
        let bosses = ctx
            .relic_counters
            .get(&RelicId::Heirloom)
            .copied()
            .unwrap_or(0)
            .max(0);
        if bosses > 0 {
            push_mult!("Heirloom", bosses as f64);
        }
    }

    if has(RelicId::Tourist) {
        let mut seen = [false; 6];
        for s in sets {
            for &tid in &s.tile_ids {
                let Some(t) = tile_by_id(tiles, tid) else {
                    continue;
                };
                if tile_is_debuffed(t, ctx.tile_debuffs) {
                    continue;
                }
                let idx = match t.suit {
                    Suit::Characters => 0,
                    Suit::Bamboos => 1,
                    Suit::Circles => 2,
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
            push_mult!("Tourist", 3.0 * distinct as f64);
        }
    }

    if has(RelicId::CrackedTile) {
        use rand::RngExt;
        let mut rng = rand::rng();
        let bonus: f64 = rng.random_range(0.0..=8.0);
        if bonus > 0.0 {
            push_mult!("Cracked Tile", (bonus * 10.0).floor() / 10.0);
        }
    }

    if has(RelicId::HungryGhost) {
        let perm_mult = ctx
            .relic_counters
            .get(&RelicId::HungryGhost)
            .copied()
            .unwrap_or(0);
        if perm_mult > 0 {
            push_mult!("Hungry Ghost", perm_mult as f64 / 10.0);
        }
    }

    if has(RelicId::WayOfPurity) {
        let numbered_suits: Vec<Suit> = sets
            .iter()
            .flat_map(|s| &s.tile_ids)
            .filter_map(|id| tile_by_id(tiles, *id))
            .map(|t| t.suit)
            .filter(|s| matches!(s, Suit::Bamboos | Suit::Characters | Suit::Circles))
            .collect();
        if !numbered_suits.is_empty() {
            let first = numbered_suits[0];
            let all_same = numbered_suits.iter().all(|&s| s == first)
                && sets
                    .iter()
                    .flat_map(|s| &s.tile_ids)
                    .filter_map(|id| tile_by_id(tiles, *id))
                    .all(|t| matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Circles));
            if all_same {
                let delta = mult * 1.5;
                push_mult!("Way of Purity", delta);
            }
        }
    }

    if has(RelicId::WayOfPairs) && !sets.is_empty() && sets.iter().all(|s| s.kind == SetKind::Pair)
    {
        let delta = mult;
        push_mult!("Way of Pairs", delta);
    }

    if has(RelicId::WayOfTriplets)
        && !sets.is_empty()
        && sets
            .iter()
            .all(|s| matches!(s.kind, SetKind::Triplet | SetKind::Kong))
    {
        let delta = mult * 1.5;
        push_mult!("Way of Triplets", delta);
    }

    if has(RelicId::WayOfSequences)
        && !sets.is_empty()
        && sets.iter().all(|s| s.kind == SetKind::Sequence)
    {
        let delta = mult;
        push_mult!("Way of Sequences", delta);
    }

    for _ in 0..count(RelicId::SilverFiligreeLantern) {
        let delta = mult;
        push_mult!("Silver Filigree Lantern", delta);
    }

    for _ in 0..count(RelicId::GlassCannon) {
        let delta = mult;
        push_mult!("Glass Cannon", delta);
    }

    {
        struct Delta {
            source: String,
            kind: StepKind,
            chip_delta: i32,
            mult_delta: f64,
            tile_ids: Vec<u32>,
        }
        let mut deltas: Vec<Delta> = Vec::with_capacity(steps.len());
        let mut prev_c = base_chips;
        let mut prev_m = 1.0_f64;
        for s in &steps {
            deltas.push(Delta {
                source: s.source.clone(),
                kind: s.kind,
                chip_delta: s.running_chips - prev_c,
                mult_delta: s.running_mult - prev_m,
                tile_ids: s.tile_ids.clone(),
            });
            prev_c = s.running_chips;
            prev_m = s.running_mult;
        }

        deltas.sort_by_key(|d| match d.kind {
            StepKind::Chips => 0,
            StepKind::Mult => 1,
            StepKind::Gold => 2,
            StepKind::Final => 3,
        });

        let mut rc = base_chips;
        let mut rm = 1.0_f64;
        steps.clear();
        for d in deltas {
            rc += d.chip_delta;
            rm += d.mult_delta;
            steps.push(ScoreStep {
                source: d.source,
                kind: d.kind,
                tile_ids: d.tile_ids,
                running_chips: rc,
                running_mult: rm,
                running_total: combine(rc, rm),
            });
        }
    }

    let final_chips = chips;
    let final_mult = mult;
    let total = combine(final_chips, final_mult);
    steps.push(ScoreStep {
        source: format!("{} × {}", final_chips, fmt_mult(final_mult)),
        kind: StepKind::Final,
        tile_ids: Vec::new(),
        running_chips: final_chips,
        running_mult: final_mult,
        running_total: total,
    });

    ScoreBreakdown {
        base_chips,
        base_points: base_chips,
        base_steps,
        steps,
        detected_yaku,
        final_chips,
        final_mult,
        total,
        flower_gold,
        scored_set_kinds: sets.iter().map(|s| s.kind).collect(),
    }
}
