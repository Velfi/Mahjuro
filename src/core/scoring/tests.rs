use super::*;
use crate::core::hand::find_pairs_and_triplets;
use crate::core::relic::{RelicId, RelicState, ScoreContext};
use crate::core::rules::RuleModifier;
use crate::core::tile::{Suit, Tile};

fn ctx_with(relics: &RelicState, scored_last_turn: bool) -> ScoreContext<'_> {
    ScoreContext {
        relics,
        tile_debuffs: &[],
        scored_last_turn,
        dora_faces: vec![],
        available_yaku: vec![],
        round_wind: None,
        first_full_hand_of_round: false,
        plays_used: 0,
        riichi_active: false,
        yaku_levels: None,
        played_yaku_this_round: vec![],
        gold: 0,
        total_score: 0,
        is_final_play: false,
        tile_polisher_bonus: 0,
        relic_counters: std::collections::BTreeMap::new(),
        unscored_hand_tiles: 0,
        river_runner_bonus: 0,
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
    assert_eq!(breakdown.final_chips, 416);
    assert_eq!(breakdown.final_mult, 16.0);
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
    ctx.yaku_levels = Some(levels);
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
    ctx.tile_debuffs = &[crate::core::debuff::TileDebuff::Class(
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
fn white_silence_mults_white_dragon_pair() {
    let hand = vec![Tile::new(Suit::Dragon, 3, 0), Tile::new(Suit::Dragon, 3, 1)];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::WhiteSilence]);
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
fn red_dragon_rage_mults_red_triplet() {
    let hand = vec![
        Tile::new(Suit::Dragon, 1, 0),
        Tile::new(Suit::Dragon, 1, 1),
        Tile::new(Suit::Dragon, 1, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::RedDragonRage]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_chips, 126);
    assert_eq!(breakdown.final_mult, 9.0);
    assert_eq!(breakdown.total, 1134);
}

#[test]
fn red_dragon_rage_fires_on_any_dragon_triplet() {
    let hand = vec![
        Tile::new(Suit::Dragon, 2, 0),
        Tile::new(Suit::Dragon, 2, 1),
        Tile::new(Suit::Dragon, 2, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::RedDragonRage]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert!(
        breakdown
            .steps
            .iter()
            .any(|s| s.source == "Red Dragon Rage")
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
    ctx.dora_faces = vec![(Suit::Characters, 5)];
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
    assert_eq!(breakdown.final_mult, 16.0);
    assert_eq!(breakdown.total, 6656);
}
