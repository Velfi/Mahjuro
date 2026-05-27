use super::*;
use crate::core::hand::find_pairs_and_triplets;
use crate::core::relic::{
    RelicId, RelicState, ScoreContext, ScoreEconomyBundle, ScorePatternBundle, ScoreRelicBundle,
    ScoreRoundBundle, ScoreTileBundle,
};
use crate::core::rules::RuleModifier;
use crate::core::tile::{Suit, Tile, TileEnhancement};

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
            bonus_round_wind: None,
            played_yaku_this_round: vec![],
            is_final_play: false,
        },
        pattern: ScorePatternBundle {
            dora_faces: vec![],
            available_yaku: vec![],
            yaku_levels: None,
        },
        economy: ScoreEconomyBundle {
            yen: 0,
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
        Tile::new(Suit::Souzu, 3, 0),
        Tile::new(Suit::Souzu, 3, 1),
        Tile::new(Suit::Souzu, 3, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    assert_eq!(breakdown.base_chips, 9);
    assert_eq!(breakdown.final_mult, 1.0);
    assert_eq!(breakdown.total, 9);
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
    assert_eq!(breakdown.base_chips, 45);
}

#[test]
fn triplet_boost_adds_chips_to_triplet() {
    let hand = vec![
        Tile::new(Suit::Souzu, 3, 0),
        Tile::new(Suit::Souzu, 3, 1),
        Tile::new(Suit::Souzu, 3, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::TripletBoost]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_chips, 69);
    assert_eq!(breakdown.final_mult, 1.35);
    assert_eq!(breakdown.total, 93);
    assert!(breakdown.steps.iter().any(|s| s.source == "Triplet Boost"));
}

#[test]
fn sequence_surge_adds_chips_to_sequence() {
    let hand = vec![
        Tile::new(Suit::Manzu, 1, 0),
        Tile::new(Suit::Manzu, 2, 1),
        Tile::new(Suit::Manzu, 3, 2),
    ];
    let sets = vec![crate::core::hand::DetectedMeld {
        kind: crate::core::hand::MeldKind::Sequence,
        tile_ids: vec![0, 1, 2],
    }];
    let r = relics(vec![RelicId::SequenceSurge]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_chips, 46);
}

#[test]
fn stacked_yaku_score_full_value_without_loadout_gating() {
    let hand = vec![
        Tile::new(Suit::Souzu, 1, 0),
        Tile::new(Suit::Souzu, 2, 1),
        Tile::new(Suit::Souzu, 3, 2),
        Tile::new(Suit::Souzu, 4, 3),
        Tile::new(Suit::Souzu, 5, 4),
        Tile::new(Suit::Souzu, 6, 5),
        Tile::new(Suit::Souzu, 7, 6),
        Tile::new(Suit::Souzu, 8, 7),
        Tile::new(Suit::Souzu, 9, 8),
        Tile::new(Suit::Souzu, 5, 9),
        Tile::new(Suit::Souzu, 5, 10),
        Tile::new(Suit::Souzu, 5, 11),
        Tile::new(Suit::Souzu, 7, 12),
        Tile::new(Suit::Souzu, 7, 13),
    ];
    let sets = vec![
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Sequence,
            tile_ids: vec![0, 1, 2],
        },
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Sequence,
            tile_ids: vec![3, 4, 5],
        },
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Sequence,
            tile_ids: vec![6, 7, 8],
        },
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Triplet,
            tile_ids: vec![9, 10, 11],
        },
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Pair,
            tile_ids: vec![12, 13],
        },
    ];
    let r = RelicState::default();
    let ctx = ctx_with(&r, false);
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    assert_eq!(breakdown.final_chips, 422);
    assert_eq!(breakdown.final_mult, 19.0);
}

#[test]
fn yaku_levels_scale_chip_and_mult() {
    let hand = vec![
        Tile::new(Suit::Pinzu, 5, 0),
        Tile::new(Suit::Pinzu, 5, 1),
        Tile::new(Suit::Pinzu, 5, 2),
        Tile::new(Suit::Souzu, 7, 3),
        Tile::new(Suit::Souzu, 7, 4),
        Tile::new(Suit::Souzu, 7, 5),
        Tile::new(Suit::Wind, 1, 6),
        Tile::new(Suit::Wind, 1, 7),
    ];
    let sets = vec![
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Triplet,
            tile_ids: vec![0, 1, 2],
        },
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Triplet,
            tile_ids: vec![3, 4, 5],
        },
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Pair,
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
    let hand = vec![Tile::new(Suit::Pinzu, 7, 0), Tile::new(Suit::Pinzu, 7, 1)];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::PairPower]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_chips, 59);
    assert_eq!(breakdown.final_mult, 2.5);
    assert_eq!(breakdown.total, 147);
}

#[test]
fn debuffed_terminal_tiles_score_zero_chips() {
    let hand = vec![Tile::new(Suit::Pinzu, 1, 0), Tile::new(Suit::Pinzu, 1, 1)];
    let sets = find_pairs_and_triplets(&hand);
    let r = RelicState::default();
    let mut ctx = ctx_with(&r, false);
    ctx.tiles.debuffs = &[crate::core::debuff::TileDebuff::Class(
        crate::core::debuff::TileDebuffClass::Terminals,
    )];
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    assert_eq!(breakdown.base_chips, 0);
    assert_eq!(breakdown.total, 0);
}

#[test]
fn debuffed_relic_is_disabled_for_scoring() {
    let hand = vec![Tile::new(Suit::Pinzu, 7, 0), Tile::new(Suit::Pinzu, 7, 1)];
    let sets = find_pairs_and_triplets(&hand);
    let mut r = relics(vec![RelicId::PairPower]);
    r.debuffed.insert(RelicId::PairPower);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_chips, 14);
    assert_eq!(breakdown.final_mult, 1.0);
    assert_eq!(breakdown.total, 14);
}

#[test]
fn minimalist_mults_single_meld() {
    let hand = vec![
        Tile::new(Suit::Souzu, 3, 0),
        Tile::new(Suit::Souzu, 3, 1),
        Tile::new(Suit::Souzu, 3, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::Minimalist]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_chips, 9);
    assert_eq!(breakdown.final_mult, 7.0);
    assert!(breakdown.steps.iter().any(|s| s.source == "Minimalist"));
}

#[test]
fn minimalist_skipped_with_multiple_melds() {
    let hand = vec![
        Tile::new(Suit::Souzu, 3, 0),
        Tile::new(Suit::Souzu, 3, 1),
        Tile::new(Suit::Souzu, 3, 2),
        Tile::new(Suit::Manzu, 5, 3),
        Tile::new(Suit::Manzu, 6, 4),
        Tile::new(Suit::Manzu, 7, 5),
    ];
    let sets = vec![
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Triplet,
            tile_ids: vec![0, 1, 2],
        },
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Sequence,
            tile_ids: vec![3, 4, 5],
        },
    ];
    let r = relics(vec![RelicId::Minimalist]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert!(!breakdown.steps.iter().any(|s| s.source == "Minimalist"));
}

#[test]
fn white_dragons_hush_mults_white_dragon_pair() {
    let hand = vec![Tile::new(Suit::Dragon, 3, 0), Tile::new(Suit::Dragon, 3, 1)];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::WhiteDragonsHush]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_chips, 30);
    assert_eq!(breakdown.final_mult, 7.0);
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
    assert_eq!(breakdown.final_chips, 171);
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
    assert_eq!(breakdown.final_chips, 105);
    assert_eq!(breakdown.final_mult, 12.0);
    assert_eq!(breakdown.total, 1260);
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
    assert!(breakdown.steps.iter().any(|s| s.source == "Dragon Rage"));
}

#[test]
fn multiplier_master_adds_two_mult_per_relic() {
    let hand = vec![
        Tile::new(Suit::Manzu, 9, 0),
        Tile::new(Suit::Manzu, 9, 1),
        Tile::new(Suit::Manzu, 9, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::MultiplierMaster]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_mult, 3.0);

    let r2 = relics(vec![RelicId::MultiplierMaster, RelicId::ChainReaction]);
    let breakdown2 = score_sets(&hand, &sets, &ctx_with(&r2, false), &[]);
    assert_eq!(breakdown2.final_mult, 5.0);

    let mut r3 = relics(vec![RelicId::MultiplierMaster, RelicId::ChainReaction]);
    r3.set_debuffed([RelicId::ChainReaction]);
    let breakdown3 = score_sets(&hand, &sets, &ctx_with(&r3, false), &[]);
    assert_eq!(breakdown3.final_mult, 5.0);
}

#[test]
fn dragon_echo_retriggers_dragon_melds() {
    let hand = vec![
        Tile::new(Suit::Manzu, 1, 0),
        Tile::new(Suit::Manzu, 2, 1),
        Tile::new(Suit::Manzu, 3, 2),
        Tile::new(Suit::Dragon, 1, 3),
        Tile::new(Suit::Dragon, 1, 4),
        Tile::new(Suit::Dragon, 1, 5),
        Tile::new(Suit::Souzu, 7, 6),
        Tile::new(Suit::Souzu, 8, 7),
        Tile::new(Suit::Souzu, 9, 8),
    ];
    let sets = vec![
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Sequence,
            tile_ids: vec![0, 1, 2],
        },
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Triplet,
            tile_ids: vec![3, 4, 5],
        },
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Sequence,
            tile_ids: vec![6, 7, 8],
        },
    ];
    let base = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    let with_echo = score_sets(
        &hand,
        &sets,
        &ctx_with(&relics(vec![RelicId::DragonEcho]), false),
        &[],
    );
    assert_eq!(with_echo.final_chips, base.final_chips + 45);
    assert!(with_echo.steps.iter().any(|s| s.source == "Dragon Echo"));
}

#[test]
fn dragon_echo_retriggers_dragon_pairs() {
    let hand = vec![Tile::new(Suit::Dragon, 3, 0), Tile::new(Suit::Dragon, 3, 1)];
    let sets = find_pairs_and_triplets(&hand);
    let base = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    let with_echo = score_sets(
        &hand,
        &sets,
        &ctx_with(&relics(vec![RelicId::DragonEcho]), false),
        &[],
    );
    assert_eq!(with_echo.final_chips, base.final_chips + 30);
    assert!(with_echo.steps.iter().any(|s| s.source == "Dragon Echo"));
}

#[test]
fn dragon_echo_ignores_non_dragon_triplet() {
    let hand = vec![
        Tile::new(Suit::Manzu, 1, 0),
        Tile::new(Suit::Manzu, 2, 1),
        Tile::new(Suit::Manzu, 3, 2),
        Tile::new(Suit::Wind, 1, 3),
        Tile::new(Suit::Wind, 1, 4),
        Tile::new(Suit::Wind, 1, 5),
    ];
    let sets = vec![
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Sequence,
            tile_ids: vec![0, 1, 2],
        },
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Triplet,
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
        Tile::new(Suit::Manzu, 5, 0),
        Tile::new(Suit::Manzu, 5, 1),
        Tile::new(Suit::Manzu, 5, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::ChainReaction]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, true), &[]);
    assert_eq!(breakdown.final_mult, 7.0);
    assert_eq!(breakdown.total, 105);
}

#[test]
fn chain_reaction_inactive_when_not_scored_last_turn() {
    let hand = vec![
        Tile::new(Suit::Manzu, 5, 0),
        Tile::new(Suit::Manzu, 5, 1),
        Tile::new(Suit::Manzu, 5, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::ChainReaction]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_mult, 1.0);
}

#[test]
fn pair_double_rule_adds_chips() {
    let hand = vec![Tile::new(Suit::Pinzu, 5, 0), Tile::new(Suit::Pinzu, 5, 1)];
    let sets = find_pairs_and_triplets(&hand);
    let breakdown = score_sets(
        &hand,
        &sets,
        &ctx_with(&RelicState::default(), false),
        &[RuleModifier::PairDoubleScore],
    );
    assert_eq!(breakdown.final_chips, 55);
    assert_eq!(breakdown.total, 55);
}

#[test]
fn dora_chips_per_matching_tile() {
    let hand = vec![
        Tile::new(Suit::Manzu, 5, 0),
        Tile::new(Suit::Manzu, 5, 1),
        Tile::new(Suit::Manzu, 5, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = RelicState::default();
    let mut ctx = ctx_with(&r, false);
    ctx.pattern.dora_faces = vec![(Suit::Manzu, 5)];
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
    assert_eq!(dora_delta, 300);
}

#[test]
fn dora_chips_per_matching_flower_tile() {
    let hand = vec![
        Tile::new(Suit::Flower, 2, 0),
        Tile::new(Suit::Manzu, 3, 1),
        Tile::new(Suit::Manzu, 3, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = RelicState::default();
    let mut ctx = ctx_with(&r, false);
    ctx.pattern.dora_faces = vec![(Suit::Flower, 2)];
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
    assert_eq!(dora_delta, 100);
}

#[test]
fn explosive_flush_full_hand_demonstration() {
    let hand = vec![
        Tile::new(Suit::Souzu, 1, 0),
        Tile::new(Suit::Souzu, 2, 1),
        Tile::new(Suit::Souzu, 3, 2),
        Tile::new(Suit::Souzu, 4, 3),
        Tile::new(Suit::Souzu, 5, 4),
        Tile::new(Suit::Souzu, 6, 5),
        Tile::new(Suit::Souzu, 7, 6),
        Tile::new(Suit::Souzu, 8, 7),
        Tile::new(Suit::Souzu, 9, 8),
        Tile::new(Suit::Souzu, 5, 9),
        Tile::new(Suit::Souzu, 5, 10),
        Tile::new(Suit::Souzu, 5, 11),
        Tile::new(Suit::Souzu, 7, 12),
        Tile::new(Suit::Souzu, 7, 13),
    ];
    let sets = vec![
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Sequence,
            tile_ids: vec![0, 1, 2],
        },
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Sequence,
            tile_ids: vec![3, 4, 5],
        },
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Sequence,
            tile_ids: vec![6, 7, 8],
        },
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Triplet,
            tile_ids: vec![9, 10, 11],
        },
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Pair,
            tile_ids: vec![12, 13],
        },
    ];
    let breakdown = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    assert_eq!(breakdown.base_chips, 74);
    assert_eq!(breakdown.final_mult, 19.0);
    assert_eq!(breakdown.total, 8018);
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
        Tile::new(Suit::Manzu, 5, 0),
        Tile::new(Suit::Manzu, 5, 1),
        Tile::new(Suit::Manzu, 5, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::DoraCrown]);
    let mut ctx = ctx_with(&r, false);
    ctx.pattern.dora_faces = vec![(Suit::Manzu, 5)];
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    // 3 dora * (100 base + 50 crown bonus) = 450
    assert_eq!(dora_chips_delta(&breakdown), 450);
}

#[test]
fn mirror_tile_doubles_dora_crown_bonus() {
    let hand = vec![
        Tile::new(Suit::Manzu, 5, 0),
        Tile::new(Suit::Manzu, 5, 1),
        Tile::new(Suit::Manzu, 5, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::MirrorTile, RelicId::DoraCrown]);
    let mut ctx = ctx_with(&r, false);
    ctx.pattern.dora_faces = vec![(Suit::Manzu, 5)];
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    // 3 dora * 100 base + (3 dora * 50 crown bonus) * 2 (mirror) = 300 + 300 = 600
    assert_eq!(dora_chips_delta(&breakdown), 600);
}

fn garden_keeper_chips_delta(breakdown: &ScoreBreakdown) -> i32 {
    let mut total = 0;
    let mut prev = breakdown.base_chips;
    for s in &breakdown.steps {
        let delta = s.running_chips - prev;
        if s.source == "Garden Keeper" {
            total += delta;
        }
        prev = s.running_chips;
    }
    total
}

#[test]
fn garden_keeper_adds_chips_per_scored_flower() {
    use crate::core::hand::{DetectedMeld, MeldKind};
    let hand = vec![
        Tile::new(Suit::Souzu, 5, 0),
        Tile::new(Suit::Souzu, 5, 1),
        Tile::new(Suit::Flower, 1, 2),
    ];
    let sets = vec![
        DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![0, 1],
        },
        DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![2],
        },
    ];
    let no_relics = RelicState::default();
    let ctx = ctx_with(&no_relics, false);
    let base = score_sets(&hand, &sets, &ctx, &[]);
    let r = relics(vec![RelicId::GardenKeeper]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(garden_keeper_chips_delta(&breakdown), 40);
    assert_eq!(breakdown.final_chips, base.final_chips + 40);
}

#[test]
fn mirror_tile_doubles_garden_keeper_flower_chips() {
    use crate::core::hand::{DetectedMeld, MeldKind};
    let hand = vec![
        Tile::new(Suit::Souzu, 5, 0),
        Tile::new(Suit::Souzu, 5, 1),
        Tile::new(Suit::Flower, 1, 2),
    ];
    let sets = vec![
        DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![0, 1],
        },
        DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![2],
        },
    ];
    let r = relics(vec![RelicId::MirrorTile, RelicId::GardenKeeper]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(garden_keeper_chips_delta(&breakdown), 80);
}

#[test]
fn kokushi_musou_scores_when_omitted_from_available_yaku() {
    use crate::core::hand::validate_selection;
    use crate::core::yaku::YakuKind;
    let tiles = vec![
        Tile::new(Suit::Manzu, 1, 0),
        Tile::new(Suit::Manzu, 9, 1),
        Tile::new(Suit::Souzu, 1, 2),
        Tile::new(Suit::Souzu, 9, 3),
        Tile::new(Suit::Pinzu, 1, 4),
        Tile::new(Suit::Pinzu, 9, 5),
        Tile::new(Suit::Wind, 1, 6),
        Tile::new(Suit::Wind, 2, 7),
        Tile::new(Suit::Wind, 3, 8),
        Tile::new(Suit::Wind, 4, 9),
        Tile::new(Suit::Dragon, 1, 10),
        Tile::new(Suit::Dragon, 2, 11),
        Tile::new(Suit::Dragon, 3, 12),
        Tile::new(Suit::Manzu, 1, 13),
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

#[test]
fn chicken_hand_scores_when_omitted_from_available_yaku() {
    use crate::core::hand::validate_selection;
    use crate::core::structure::StructureTriggerMeta;
    use crate::core::yaku::YakuKind;
    let tiles = vec![
        Tile::new(Suit::Manzu, 1, 0),
        Tile::new(Suit::Manzu, 2, 1),
        Tile::new(Suit::Manzu, 3, 2),
        Tile::new(Suit::Souzu, 4, 3),
        Tile::new(Suit::Souzu, 5, 4),
        Tile::new(Suit::Souzu, 6, 5),
        Tile::new(Suit::Pinzu, 7, 6),
        Tile::new(Suit::Pinzu, 8, 7),
        Tile::new(Suit::Pinzu, 9, 8),
        Tile::new(Suit::Pinzu, 3, 9),
        Tile::new(Suit::Pinzu, 3, 10),
        Tile::new(Suit::Pinzu, 3, 11),
        Tile::new(Suit::Wind, 2, 12),
        Tile::new(Suit::Wind, 2, 13),
    ];
    let sets = validate_selection(&tiles).expect("chicken decomposition");
    let r = RelicState::default();
    let mut ctx = ctx_with(&r, false);
    ctx.pattern.available_yaku = vec![YakuKind::Tanyao, YakuKind::Toitoi];
    ctx.structure = Some(StructureTriggerMeta {
        meld_count: sets.len() as u32,
        inject_chicken_if_no_yaku: true,
    });
    let breakdown = score_sets(&tiles, &sets, &ctx, &[]);
    assert!(
        breakdown.detected_yaku.contains(&YakuKind::ChickenHand),
        "expected Chicken Hand despite tutorial pool, got {:?}",
        breakdown.detected_yaku
    );
}

#[test]
fn pearl_stamp_adds_100_chips_per_meld() {
    let hand = vec![
        {
            let mut t = Tile::new(Suit::Souzu, 3, 0);
            t.enhancement = Some(TileEnhancement::Pearl);
            t
        },
        {
            let mut t = Tile::new(Suit::Souzu, 3, 1);
            t.enhancement = Some(TileEnhancement::Pearl);
            t
        },
        {
            let mut t = Tile::new(Suit::Souzu, 3, 2);
            t.enhancement = Some(TileEnhancement::Pearl);
            t
        },
    ];
    let sets = find_pairs_and_triplets(&hand);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    assert_eq!(
        breakdown
            .steps
            .iter()
            .filter(|s| s.source == "Pearl Talisman")
            .map(|s| s.running_chips)
            .last(),
        Some(breakdown.base_chips + 100)
    );
}

#[test]
fn polychrome_stamp_adds_0_25_mult_per_meld() {
    let hand = vec![
        {
            let mut t = Tile::new(Suit::Manzu, 5, 0);
            t.enhancement = Some(TileEnhancement::Polychrome);
            t
        },
        {
            let mut t = Tile::new(Suit::Manzu, 5, 1);
            t.enhancement = Some(TileEnhancement::Polychrome);
            t
        },
    ];
    let sets = vec![crate::core::hand::DetectedMeld {
        kind: crate::core::hand::MeldKind::Pair,
        tile_ids: vec![0, 1],
    }];
    let breakdown = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    assert!(
        (breakdown.final_mult - 1.25).abs() < f64::EPSILON,
        "expected ×1.25 mult from one polychrome meld, got {}",
        breakdown.final_mult
    );
}

#[test]
fn gilded_stamp_pays_once_per_meld_including_pairs() {
    let mut t0 = Tile::new(Suit::Manzu, 5, 0);
    let mut t1 = Tile::new(Suit::Manzu, 5, 1);
    t0.enhancement = Some(TileEnhancement::Gilded);
    t1.enhancement = Some(TileEnhancement::Gilded);
    let hand = vec![t0, t1];
    let sets = vec![crate::core::hand::DetectedMeld {
        kind: crate::core::hand::MeldKind::Pair,
        tile_ids: vec![0, 1],
    }];
    let breakdown = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    assert_eq!(breakdown.flower_yen, 1);
    assert!(
        breakdown
            .steps
            .iter()
            .any(|s| s.source == "Gilded Talisman" && s.kind == StepKind::Yen)
    );
}

#[test]
fn gilded_stamp_pays_once_per_meld_not_per_tile() {
    let hand = vec![
        {
            let mut t = Tile::new(Suit::Souzu, 3, 0);
            t.enhancement = Some(TileEnhancement::Gilded);
            t
        },
        {
            let mut t = Tile::new(Suit::Souzu, 3, 1);
            t.enhancement = Some(TileEnhancement::Gilded);
            t
        },
        {
            let mut t = Tile::new(Suit::Souzu, 3, 2);
            t.enhancement = Some(TileEnhancement::Gilded);
            t
        },
    ];
    let sets = find_pairs_and_triplets(&hand);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    assert_eq!(breakdown.flower_yen, 1);
}

#[test]
fn format_meld_groups_matches_cascade_set_labels() {
    use crate::core::hand::{DetectedMeld, MeldKind};
    let tiles: Vec<Tile> = (0..16).map(|i| Tile::new(Suit::Pinzu, 9, i)).collect();
    let sets: Vec<DetectedMeld> = (0..4)
        .map(|k| DetectedMeld {
            kind: MeldKind::Kong,
            tile_ids: (k * 4..k * 4 + 4).map(|i| i as u32).collect(),
        })
        .collect();
    let out = format_meld_groups(&tiles, &sets).expect("labels");
    assert_eq!(
        out,
        "Kong  9p 9p 9p 9p · Kong  9p 9p 9p 9p · Kong  9p 9p 9p 9p · Kong  9p 9p 9p 9p"
    );
}

#[test]
fn ancestor_echo_retriggers_highest_value_meld() {
    let hand = vec![
        Tile::new(Suit::Souzu, 3, 0),
        Tile::new(Suit::Souzu, 3, 1),
        Tile::new(Suit::Souzu, 3, 2),
        Tile::new(Suit::Dragon, 1, 3),
        Tile::new(Suit::Dragon, 1, 4),
        Tile::new(Suit::Dragon, 1, 5),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let base = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    let with_echo = score_sets(
        &hand,
        &sets,
        &ctx_with(&relics(vec![RelicId::AncestorEcho]), false),
        &[],
    );
    assert_eq!(with_echo.final_chips, base.final_chips + 45);
    assert!(with_echo.steps.iter().any(|s| s.source == "Ancestor Echo"));
}

#[test]
fn crown_of_patterns_adds_mult_per_distinct_yaku() {
    use crate::core::hand::validate_selection;
    let tiles = vec![
        Tile::new(Suit::Manzu, 2, 0),
        Tile::new(Suit::Manzu, 3, 1),
        Tile::new(Suit::Manzu, 4, 2),
        Tile::new(Suit::Souzu, 2, 3),
        Tile::new(Suit::Souzu, 3, 4),
        Tile::new(Suit::Souzu, 4, 5),
        Tile::new(Suit::Pinzu, 2, 6),
        Tile::new(Suit::Pinzu, 3, 7),
        Tile::new(Suit::Pinzu, 4, 8),
        Tile::new(Suit::Manzu, 5, 9),
        Tile::new(Suit::Manzu, 5, 10),
        Tile::new(Suit::Manzu, 6, 11),
        Tile::new(Suit::Manzu, 6, 12),
        Tile::new(Suit::Manzu, 6, 13),
    ];
    let sets = validate_selection(&tiles).expect("full hand");
    let base = score_sets(&tiles, &sets, &ctx_with(&RelicState::default(), false), &[]);
    let with_crown = score_sets(
        &tiles,
        &sets,
        &ctx_with(&relics(vec![RelicId::CrownOfPatterns]), false),
        &[],
    );
    let yaku_count = with_crown.detected_yaku.len();
    assert!(yaku_count >= 2);
    assert_eq!(
        with_crown.final_mult,
        base.final_mult + 4.0 * yaku_count as f64
    );
}
