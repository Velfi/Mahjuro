use super::*;
use crate::core::hand::find_pairs_and_triplets;
use crate::core::relic::{
    RelicId, RelicState, ScoreContext, ScoreEconomyBundle, ScorePatternBundle, ScoreRelicBundle,
    ScoreRoundBundle, ScoreTileBundle,
};
use crate::core::rules::RuleModifier;
use crate::core::tile::{Suit, Tile};

fn ctx_with(relics: &RelicState, scored_last_turn: bool) -> ScoreContext<'_> {
    ScoreContext {
        relic: ScoreRelicBundle {
            roster: relics,
            counters: std::collections::BTreeMap::new(),
        },
        tiles: ScoreTileBundle {
            debuffs: &[],
            hand_for_ghost: &[],
        },
        round: ScoreRoundBundle {
            scored_last_turn,
            plays_used: 0,
            round_wind: None,
            played_yaku_this_round: vec![],
            is_final_play: false,
        },
        pattern: ScorePatternBundle {
            dora_faces: vec![],
            available_yaku: vec![],
            yaku_levels: None,
        },
        economy: ScoreEconomyBundle {
            gold: 0,
            total_score: 0,
        },
        structure: None,
    }
}

fn relics(ids: Vec<RelicId>) -> RelicState {
    RelicState {
        active: ids,
        ..Default::default()
    }
}

#[test]
fn bare_triplet_of_threes() {
    let hand = vec![
        Tile::new(Suit::Bamboos, 3, 0),
        Tile::new(Suit::Bamboos, 3, 1),
        Tile::new(Suit::Bamboos, 3, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    assert_eq!(breakdown.base_chips, 59);
    assert_eq!(breakdown.final_mult, 1.0);
    assert_eq!(breakdown.total, 59);
}

#[test]
fn honor_triplet_uses_flat_value() {
    let hand = vec![
        Tile::new(Suit::Wind, 1, 0),
        Tile::new(Suit::Wind, 1, 1),
        Tile::new(Suit::Wind, 1, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    assert_eq!(breakdown.base_chips, 86);
}

#[test]
fn triplet_boost_adds_chips_to_triplet() {
    let hand = vec![
        Tile::new(Suit::Bamboos, 3, 0),
        Tile::new(Suit::Bamboos, 3, 1),
        Tile::new(Suit::Bamboos, 3, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::TripletBoost]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_chips, 99);
    assert_eq!(breakdown.final_mult, 1.2);
    assert_eq!(breakdown.total, 118);
    assert!(breakdown.steps.iter().any(|s| s.source == "Triplet Boost"));
}

#[test]
fn sequence_surge_adds_chips_to_sequence() {
    let hand = vec![
        Tile::new(Suit::Characters, 1, 0),
        Tile::new(Suit::Characters, 2, 1),
        Tile::new(Suit::Characters, 3, 2),
    ];
    let sets = vec![crate::core::hand::DetectedSet {
        kind: crate::core::hand::SetKind::Sequence,
        tile_ids: vec![0, 1, 2],
    }];
    let r = relics(vec![RelicId::SequenceSurge]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_chips, 59);
}

#[test]
fn stacked_yaku_score_full_value_without_loadout_gating() {
    let hand = vec![
        Tile::new(Suit::Bamboos, 1, 0),
        Tile::new(Suit::Bamboos, 2, 1),
        Tile::new(Suit::Bamboos, 3, 2),
        Tile::new(Suit::Bamboos, 4, 3),
        Tile::new(Suit::Bamboos, 5, 4),
        Tile::new(Suit::Bamboos, 6, 5),
        Tile::new(Suit::Bamboos, 7, 6),
        Tile::new(Suit::Bamboos, 8, 7),
        Tile::new(Suit::Bamboos, 9, 8),
        Tile::new(Suit::Bamboos, 5, 9),
        Tile::new(Suit::Bamboos, 5, 10),
        Tile::new(Suit::Bamboos, 5, 11),
        Tile::new(Suit::Bamboos, 7, 12),
        Tile::new(Suit::Bamboos, 7, 13),
    ];
    let sets = vec![
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Sequence,
            tile_ids: vec![0, 1, 2],
        },
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Sequence,
            tile_ids: vec![3, 4, 5],
        },
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Sequence,
            tile_ids: vec![6, 7, 8],
        },
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Triplet,
            tile_ids: vec![9, 10, 11],
        },
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Pair,
            tile_ids: vec![12, 13],
        },
    ];
    let r = RelicState::default();
    let ctx = ctx_with(&r, false);
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    assert_eq!(breakdown.final_chips, 443);
    assert_eq!(breakdown.final_mult, 18.0);
}

#[test]
fn yaku_levels_scale_chip_and_mult() {
    let hand = vec![
        Tile::new(Suit::Circles, 5, 0),
        Tile::new(Suit::Circles, 5, 1),
        Tile::new(Suit::Circles, 5, 2),
        Tile::new(Suit::Bamboos, 7, 3),
        Tile::new(Suit::Bamboos, 7, 4),
        Tile::new(Suit::Bamboos, 7, 5),
        Tile::new(Suit::Wind, 1, 6),
        Tile::new(Suit::Wind, 1, 7),
    ];
    let sets = vec![
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Triplet,
            tile_ids: vec![0, 1, 2],
        },
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Triplet,
            tile_ids: vec![3, 4, 5],
        },
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Pair,
            tile_ids: vec![6, 7],
        },
    ];
    let r = RelicState::default();
    let mut levels = crate::core::zodiac::YakuLevels::default();
    levels.levels.insert(crate::core::yaku::YakuKind::Toitoi, 3);
    let mut ctx = ctx_with(&r, false);
    ctx.pattern.yaku_levels = Some(levels);
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    let toitoi_chip = breakdown
        .steps
        .iter()
        .find(|s| s.source == "Toitoi" && s.kind == StepKind::Chips);
    assert!(toitoi_chip.is_some());
}

#[test]
fn pair_power_grants_chips_and_mult() {
    let hand = vec![
        Tile::new(Suit::Circles, 7, 0),
        Tile::new(Suit::Circles, 7, 1),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::PairPower]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_chips, 62);
    assert_eq!(breakdown.final_mult, 2.0);
    assert_eq!(breakdown.total, 124);
}

#[test]
fn debuffed_terminal_tiles_keep_pair_bonus_but_lose_tile_points() {
    let hand = vec![
        Tile::new(Suit::Circles, 1, 0),
        Tile::new(Suit::Circles, 1, 1),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = RelicState::default();
    let mut ctx = ctx_with(&r, false);
    ctx.tiles.debuffs = &[crate::core::debuff::TileDebuff::Class(
        crate::core::debuff::TileDebuffClass::Terminals,
    )];
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    assert_eq!(breakdown.base_chips, 18);
    assert_eq!(breakdown.total, 18);
}

#[test]
fn debuffed_relic_is_disabled_for_scoring() {
    let hand = vec![
        Tile::new(Suit::Circles, 7, 0),
        Tile::new(Suit::Circles, 7, 1),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let mut r = relics(vec![RelicId::PairPower]);
    r.debuffed.insert(RelicId::PairPower);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_chips, 32);
    assert_eq!(breakdown.final_mult, 1.0);
    assert_eq!(breakdown.total, 32);
}

#[test]
fn white_dragons_hush_mults_white_dragon_pair() {
    let hand = vec![Tile::new(Suit::Dragon, 3, 0), Tile::new(Suit::Dragon, 3, 1)];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::WhiteDragonsHush]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_chips, 42);
    assert_eq!(breakdown.final_mult, 5.0);
    assert_eq!(breakdown.total, 210);
}

#[test]
fn honor_fury_adds_chips_per_honor_tile() {
    let hand = vec![
        Tile::new(Suit::Wind, 1, 0),
        Tile::new(Suit::Wind, 1, 1),
        Tile::new(Suit::Wind, 1, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::HonorFury]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_chips, 170);
}

#[test]
fn dragon_rage_mults_red_triplet() {
    let hand = vec![
        Tile::new(Suit::Dragon, 1, 0),
        Tile::new(Suit::Dragon, 1, 1),
        Tile::new(Suit::Dragon, 1, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::DragonRage]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_chips, 126);
    assert_eq!(breakdown.final_mult, 9.0);
    assert_eq!(breakdown.total, 1134);
}

#[test]
fn dragon_rage_fires_on_any_dragon_triplet() {
    let hand = vec![
        Tile::new(Suit::Dragon, 2, 0),
        Tile::new(Suit::Dragon, 2, 1),
        Tile::new(Suit::Dragon, 2, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::DragonRage]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert!(
        breakdown
            .steps
            .iter()
            .any(|s| s.source == "Dragon Rage")
    );
}

#[test]
fn multiplier_master_scales_with_relic_count() {
    let hand = vec![
        Tile::new(Suit::Characters, 9, 0),
        Tile::new(Suit::Characters, 9, 1),
        Tile::new(Suit::Characters, 9, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![
        RelicId::MultiplierMaster,
        RelicId::SetMagnet,
        RelicId::QuickDraw,
    ]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_mult, 2.5);
}

#[test]
fn dragon_echo_copies_adjacent_set_chips() {
    let hand = vec![
        Tile::new(Suit::Characters, 1, 0),
        Tile::new(Suit::Characters, 2, 1),
        Tile::new(Suit::Characters, 3, 2),
        Tile::new(Suit::Dragon, 1, 3),
        Tile::new(Suit::Dragon, 1, 4),
        Tile::new(Suit::Dragon, 1, 5),
        Tile::new(Suit::Bamboos, 7, 6),
        Tile::new(Suit::Bamboos, 8, 7),
        Tile::new(Suit::Bamboos, 9, 8),
    ];
    let sets = vec![
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Sequence,
            tile_ids: vec![0, 1, 2],
        },
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Triplet,
            tile_ids: vec![3, 4, 5],
        },
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Sequence,
            tile_ids: vec![6, 7, 8],
        },
    ];
    let r = relics(vec![RelicId::DragonEcho]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    let (idx, _) = breakdown
        .steps
        .iter()
        .enumerate()
        .find(|(_, s)| s.source == "Dragon Echo")
        .unwrap();
    let prev_chips = if idx == 0 {
        breakdown.base_chips
    } else {
        breakdown.steps[idx - 1].running_chips
    };
    let echo_delta = breakdown.steps[idx].running_chips - prev_chips;
    assert_eq!(echo_delta, 86);
}

#[test]
fn dragon_echo_ignores_non_dragon_triplet() {
    let hand = vec![
        Tile::new(Suit::Characters, 1, 0),
        Tile::new(Suit::Characters, 2, 1),
        Tile::new(Suit::Characters, 3, 2),
        Tile::new(Suit::Wind, 1, 3),
        Tile::new(Suit::Wind, 1, 4),
        Tile::new(Suit::Wind, 1, 5),
    ];
    let sets = vec![
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Sequence,
            tile_ids: vec![0, 1, 2],
        },
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Triplet,
            tile_ids: vec![3, 4, 5],
        },
    ];
    let r = relics(vec![RelicId::DragonEcho]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert!(!breakdown.steps.iter().any(|s| s.source == "Dragon Echo"));
}

#[test]
fn chain_reaction_adds_mult_when_scored_last_turn() {
    let hand = vec![
        Tile::new(Suit::Characters, 5, 0),
        Tile::new(Suit::Characters, 5, 1),
        Tile::new(Suit::Characters, 5, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::ChainReaction]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, true), &[]);
    assert_eq!(breakdown.final_mult, 5.0);
    assert_eq!(breakdown.total, 325);
}

#[test]
fn chain_reaction_inactive_when_not_scored_last_turn() {
    let hand = vec![
        Tile::new(Suit::Characters, 5, 0),
        Tile::new(Suit::Characters, 5, 1),
        Tile::new(Suit::Characters, 5, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::ChainReaction]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_mult, 1.0);
}

#[test]
fn pair_double_rule_adds_chips() {
    let hand = vec![
        Tile::new(Suit::Circles, 5, 0),
        Tile::new(Suit::Circles, 5, 1),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let breakdown = score_sets(
        &hand,
        &sets,
        &ctx_with(&RelicState::default(), false),
        &[RuleModifier::PairDoubleScore],
    );
    assert_eq!(breakdown.final_chips, 58);
    assert_eq!(breakdown.total, 58);
}

#[test]
fn dora_chips_per_matching_tile() {
    let hand = vec![
        Tile::new(Suit::Characters, 5, 0),
        Tile::new(Suit::Characters, 5, 1),
        Tile::new(Suit::Characters, 5, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = RelicState::default();
    let mut ctx = ctx_with(&r, false);
    ctx.pattern.dora_faces = vec![(Suit::Characters, 5)];
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    let (idx, _) = breakdown
        .steps
        .iter()
        .enumerate()
        .find(|(_, s)| s.source.starts_with("Dora"))
        .unwrap();
    let prev_chips = if idx == 0 {
        breakdown.base_chips
    } else {
        breakdown.steps[idx - 1].running_chips
    };
    let dora_delta = breakdown.steps[idx].running_chips - prev_chips;
    assert_eq!(dora_delta, 75);
}

#[test]
fn explosive_flush_full_hand_demonstration() {
    let hand = vec![
        Tile::new(Suit::Bamboos, 1, 0),
        Tile::new(Suit::Bamboos, 2, 1),
        Tile::new(Suit::Bamboos, 3, 2),
        Tile::new(Suit::Bamboos, 4, 3),
        Tile::new(Suit::Bamboos, 5, 4),
        Tile::new(Suit::Bamboos, 6, 5),
        Tile::new(Suit::Bamboos, 7, 6),
        Tile::new(Suit::Bamboos, 8, 7),
        Tile::new(Suit::Bamboos, 9, 8),
        Tile::new(Suit::Bamboos, 5, 9),
        Tile::new(Suit::Bamboos, 5, 10),
        Tile::new(Suit::Bamboos, 5, 11),
        Tile::new(Suit::Bamboos, 7, 12),
        Tile::new(Suit::Bamboos, 7, 13),
    ];
    let sets = vec![
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Sequence,
            tile_ids: vec![0, 1, 2],
        },
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Sequence,
            tile_ids: vec![3, 4, 5],
        },
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Sequence,
            tile_ids: vec![6, 7, 8],
        },
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Triplet,
            tile_ids: vec![9, 10, 11],
        },
        crate::core::hand::DetectedSet {
            kind: crate::core::hand::SetKind::Pair,
            tile_ids: vec![12, 13],
        },
    ];
    let breakdown = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    assert_eq!(breakdown.base_chips, 226);
    assert_eq!(breakdown.final_mult, 18.0);
    assert_eq!(breakdown.total, 7974);
}

fn dora_chips_delta(breakdown: &ScoreBreakdown) -> i32 {
    let mut total = 0;
    let mut prev = breakdown.base_chips;
    for s in &breakdown.steps {
        let delta = s.running_chips - prev;
        if s.source.starts_with("Dora") {
            total += delta;
        }
        prev = s.running_chips;
    }
    total
}

#[test]
fn dora_crown_alone_adds_ten_per_dora() {
    let hand = vec![
        Tile::new(Suit::Characters, 5, 0),
        Tile::new(Suit::Characters, 5, 1),
        Tile::new(Suit::Characters, 5, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::DoraCrown]);
    let mut ctx = ctx_with(&r, false);
    ctx.pattern.dora_faces = vec![(Suit::Characters, 5)];
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    // 3 dora * (25 base + 10 crown bonus) = 105
    assert_eq!(dora_chips_delta(&breakdown), 105);
}

#[test]
fn mirror_tile_doubles_dora_crown_bonus() {
    let hand = vec![
        Tile::new(Suit::Characters, 5, 0),
        Tile::new(Suit::Characters, 5, 1),
        Tile::new(Suit::Characters, 5, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::MirrorTile, RelicId::DoraCrown]);
    let mut ctx = ctx_with(&r, false);
    ctx.pattern.dora_faces = vec![(Suit::Characters, 5)];
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    // 3 dora * 25 base + (3 dora * 10 crown bonus) * 2 (mirror) = 75 + 60 = 135
    assert_eq!(dora_chips_delta(&breakdown), 135);
}

fn flower_chips_delta(breakdown: &ScoreBreakdown) -> i32 {
    let mut total = 0;
    let mut prev = breakdown.base_chips;
    for s in &breakdown.steps {
        let delta = s.running_chips - prev;
        if s.source.contains("Plum") || s.source.contains("Chrysanthemum") {
            total += delta;
        }
        prev = s.running_chips;
    }
    total
}

#[test]
fn mirror_tile_doubles_garden_keeper_extra_pass() {
    use crate::core::hand::{DetectedSet, SetKind};
    // One pair to satisfy minimum scoring + a flower tile.
    let hand = vec![
        Tile::new(Suit::Bamboos, 5, 0),
        Tile::new(Suit::Bamboos, 5, 1),
        Tile::new(Suit::Flower, 1, 2), // Plum Blossom: 40 chips
    ];
    let sets = vec![
        DetectedSet {
            kind: SetKind::Pair,
            tile_ids: vec![0, 1],
        },
        DetectedSet {
            kind: SetKind::Pair,
            tile_ids: vec![2],
        }, // single flower as pseudo-set
    ];
    // Base GardenKeeper: triggers = 2 -> 80 chips of plum.
    // After refactor with Mirror: base pass 40 + GardenKeeper second pass 40 + Mirror duplicates GardenKeeper's second pass 40 = 120.
    let r = relics(vec![RelicId::MirrorTile, RelicId::GardenKeeper]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(flower_chips_delta(&breakdown), 120);
}

#[test]
fn kokushi_musou_scores_when_omitted_from_available_yaku() {
    use crate::core::hand::validate_selection;
    use crate::core::yaku::YakuKind;
    let tiles = vec![
        Tile::new(Suit::Characters, 1, 0),
        Tile::new(Suit::Characters, 9, 1),
        Tile::new(Suit::Bamboos, 1, 2),
        Tile::new(Suit::Bamboos, 9, 3),
        Tile::new(Suit::Circles, 1, 4),
        Tile::new(Suit::Circles, 9, 5),
        Tile::new(Suit::Wind, 1, 6),
        Tile::new(Suit::Wind, 2, 7),
        Tile::new(Suit::Wind, 3, 8),
        Tile::new(Suit::Wind, 4, 9),
        Tile::new(Suit::Dragon, 1, 10),
        Tile::new(Suit::Dragon, 2, 11),
        Tile::new(Suit::Dragon, 3, 12),
        Tile::new(Suit::Characters, 1, 13),
    ];
    let sets = validate_selection(&tiles).expect("kokushi decomposition");
    let r = RelicState::default();
    let mut ctx = ctx_with(&r, false);
    ctx.pattern.available_yaku = YakuKind::all()
        .iter()
        .copied()
        .filter(|&y| y != YakuKind::KokushiMusou)
        .collect();
    let breakdown = score_sets(&tiles, &sets, &ctx, &[]);
    assert!(
        breakdown.detected_yaku.contains(&YakuKind::KokushiMusou),
        "expected Kokushi despite secret-yaku filter, got {:?}",
        breakdown.detected_yaku
    );
}
