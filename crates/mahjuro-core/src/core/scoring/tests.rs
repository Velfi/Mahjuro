use super::*;
use crate::core::hand::{DetectedMeld, MeldKind, find_pairs_and_triplets};
use crate::core::relic::{
    RelicId, RelicState, ScoreContext, ScoreEconomyBundle, ScorePatternBundle, ScoreRelicBundle,
    ScoreRoundBundle, ScoreTileBundle, kindling_han_bonus,
};
use crate::core::tile::{Suit, Tile, TileEnhancement};
use crate::core::yaku::YakuKind;

fn with_tanyao_fu(base_meld_fu: i32) -> i32 {
    base_meld_fu + YakuKind::Tanyao.fu_bonus()
}

fn with_tanyao_han(extra_han: f64) -> f64 {
    SCORING_BASE_HAN + YakuKind::Tanyao.han_bonus() + extra_han
}

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
    let base_fu = 9;
    let fu = with_tanyao_fu(base_fu);
    let han = with_tanyao_han(0.0);
    assert_eq!(breakdown.base_fu, base_fu);
    assert_eq!(breakdown.final_fu, fu);
    assert_eq!(breakdown.final_han, han);
    assert_eq!(breakdown.total, combine(fu, han));
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
    assert_eq!(breakdown.base_fu, 45);
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
    let base_fu = 9;
    let fu = with_tanyao_fu(base_fu) + TRIPLET_BOOST_FU;
    let han = with_tanyao_han(TRIPLET_BOOST_HAN_PER_TRIPLET);
    assert_eq!(breakdown.final_fu, fu);
    assert_eq!(breakdown.final_han, han);
    assert_eq!(breakdown.total, combine(fu, han));
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
    assert_eq!(breakdown.final_fu, 56);
    assert_eq!(breakdown.final_han, 2.25);
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
    assert_eq!(breakdown.final_fu, 332);
    assert_eq!(breakdown.final_han, 14.0);
    assert_eq!(breakdown.total, 4648);
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
        .find(|s| s.source == "Toitoi" && s.kind == StepKind::Fu);
    assert!(toitoi_chip.is_some());
}

#[test]
fn pair_power_grants_chips_and_mult() {
    let hand = vec![Tile::new(Suit::Pinzu, 7, 0), Tile::new(Suit::Pinzu, 7, 1)];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::PairPower]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_fu, 59);
    assert_eq!(breakdown.final_han, 2.25);
    assert_eq!(breakdown.total, 132);
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
    assert_eq!(breakdown.base_fu, 0);
    assert_eq!(breakdown.total, 0);
}

#[test]
fn debuffed_relic_is_disabled_for_scoring() {
    let hand = vec![Tile::new(Suit::Pinzu, 7, 0), Tile::new(Suit::Pinzu, 7, 1)];
    let sets = find_pairs_and_triplets(&hand);
    let mut r = relics(vec![RelicId::PairPower]);
    r.debuffed.insert(RelicId::PairPower);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_fu, 14);
    assert_eq!(breakdown.final_han, 1.0);
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
    let base_fu = 9;
    let fu = with_tanyao_fu(base_fu) + MINIMALIST_FU;
    let han = with_tanyao_han(MINIMALIST_HAN);
    assert_eq!(breakdown.final_fu, fu);
    assert_eq!(breakdown.final_han, han);
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
    assert_eq!(breakdown.final_fu, 30);
    assert_eq!(breakdown.final_han, 7.0);
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
    assert_eq!(breakdown.final_fu, 171);
}

#[test]
fn plain_dealing_adds_chips_per_simple_tile() {
    let hand = vec![
        Tile::new(Suit::Pinzu, 5, 0),
        Tile::new(Suit::Pinzu, 5, 1),
        Tile::new(Suit::Pinzu, 5, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::PlainDealing]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    let base_fu = 15;
    let simple_tiles = 3;
    let fu = with_tanyao_fu(base_fu) + PLAIN_DEALING_FU_PER_SIMPLE_TILE * simple_tiles;
    assert_eq!(breakdown.final_fu, fu);
    assert!(breakdown.steps.iter().any(|s| s.source == "Plain Dealing"));
}

#[test]
fn even_keel_adds_chips_for_middle_ranks() {
    let hand = vec![
        Tile::new(Suit::Souzu, 5, 0),
        Tile::new(Suit::Souzu, 5, 1),
        Tile::new(Suit::Souzu, 5, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::EvenKeel]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    let base_fu = 15;
    let even_keel_tiles = 3;
    let fu = with_tanyao_fu(base_fu) + EVEN_KEEL_FU_PER_TILE * even_keel_tiles;
    assert_eq!(breakdown.final_fu, fu);
    assert!(breakdown.steps.iter().any(|s| s.source == "Even Keel"));
}

#[test]
fn blue_tiles_white_dragon_mults_structure_with_pinzu_and_white_dragon() {
    use crate::core::structure::StructureTriggerMeta;
    let hand = vec![
        Tile::new(Suit::Pinzu, 5, 0),
        Tile::new(Suit::Pinzu, 5, 1),
        Tile::new(Suit::Pinzu, 5, 2),
        Tile::new(Suit::Dragon, 3, 3),
        Tile::new(Suit::Dragon, 3, 4),
    ];
    let sets = vec![
        DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![0, 1, 2],
        },
        DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![3, 4],
        },
    ];
    let r = relics(vec![RelicId::BlueTilesWhiteDragon]);
    let mut ctx = ctx_with(&r, false);
    ctx.structure = Some(StructureTriggerMeta {
        meld_count: sets.len() as u32,
        inject_chicken_if_no_yaku: false,
    });
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    assert_eq!(breakdown.final_han, 11.0);
    assert!(
        breakdown
            .steps
            .iter()
            .any(|s| s.source == "Blue Tiles White Dragon")
    );
}

#[test]
fn blue_tiles_white_dragon_ignores_direct_plays() {
    let hand = vec![
        Tile::new(Suit::Pinzu, 5, 0),
        Tile::new(Suit::Pinzu, 5, 1),
        Tile::new(Suit::Pinzu, 5, 2),
        Tile::new(Suit::Dragon, 3, 3),
        Tile::new(Suit::Dragon, 3, 4),
    ];
    let sets = vec![
        DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![0, 1, 2],
        },
        DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![3, 4],
        },
    ];
    let r = relics(vec![RelicId::BlueTilesWhiteDragon]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_han, 5.0);
    assert!(
        !breakdown
            .steps
            .iter()
            .any(|s| s.source == "Blue Tiles White Dragon")
    );
}

#[test]
fn green_tiles_green_dragon_mults_structure_with_souzu_and_green_dragon() {
    use crate::core::structure::StructureTriggerMeta;
    let hand = vec![
        Tile::new(Suit::Souzu, 5, 0),
        Tile::new(Suit::Souzu, 5, 1),
        Tile::new(Suit::Souzu, 5, 2),
        Tile::new(Suit::Dragon, 2, 3),
        Tile::new(Suit::Dragon, 2, 4),
    ];
    let sets = vec![
        DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![0, 1, 2],
        },
        DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![3, 4],
        },
    ];
    let r = relics(vec![RelicId::GreenTilesGreenDragon]);
    let mut ctx = ctx_with(&r, false);
    ctx.structure = Some(StructureTriggerMeta {
        meld_count: sets.len() as u32,
        inject_chicken_if_no_yaku: false,
    });
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    assert_eq!(breakdown.final_han, 11.0);
    assert!(
        breakdown
            .steps
            .iter()
            .any(|s| s.source == "Green Tiles Green Dragon")
    );
}

#[test]
fn red_tiles_red_dragon_mults_structure_with_manzu_and_red_dragon() {
    use crate::core::structure::StructureTriggerMeta;
    let hand = vec![
        Tile::new(Suit::Manzu, 5, 0),
        Tile::new(Suit::Manzu, 5, 1),
        Tile::new(Suit::Manzu, 5, 2),
        Tile::new(Suit::Dragon, 1, 3),
        Tile::new(Suit::Dragon, 1, 4),
    ];
    let sets = vec![
        DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![0, 1, 2],
        },
        DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![3, 4],
        },
    ];
    let r = relics(vec![RelicId::RedTilesRedDragon]);
    let mut ctx = ctx_with(&r, false);
    ctx.structure = Some(StructureTriggerMeta {
        meld_count: sets.len() as u32,
        inject_chicken_if_no_yaku: false,
    });
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    assert_eq!(breakdown.final_han, 11.0);
    assert!(
        breakdown
            .steps
            .iter()
            .any(|s| s.source == "Red Tiles Red Dragon")
    );
}

#[test]
fn chow_line_mults_three_sequences() {
    let hand = vec![
        Tile::new(Suit::Manzu, 1, 0),
        Tile::new(Suit::Manzu, 2, 1),
        Tile::new(Suit::Manzu, 3, 2),
        Tile::new(Suit::Souzu, 3, 3),
        Tile::new(Suit::Souzu, 4, 4),
        Tile::new(Suit::Souzu, 5, 5),
        Tile::new(Suit::Pinzu, 6, 6),
        Tile::new(Suit::Pinzu, 7, 7),
        Tile::new(Suit::Pinzu, 8, 8),
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
    ];
    let r = relics(vec![RelicId::ChowLine]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_han, 5.0);
    assert!(breakdown.steps.iter().any(|s| s.source == "Chow Line"));
}

#[test]
fn open_gate_mults_all_simple_structure() {
    let hand = vec![
        Tile::new(Suit::Manzu, 2, 0),
        Tile::new(Suit::Manzu, 3, 1),
        Tile::new(Suit::Manzu, 4, 2),
        Tile::new(Suit::Souzu, 5, 3),
        Tile::new(Suit::Souzu, 5, 4),
    ];
    let sets = vec![
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Sequence,
            tile_ids: vec![0, 1, 2],
        },
        crate::core::hand::DetectedMeld {
            kind: crate::core::hand::MeldKind::Pair,
            tile_ids: vec![3, 4],
        },
    ];
    let r = relics(vec![RelicId::OpenGate]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    let han = SCORING_BASE_HAN + YakuKind::Tanyao.han_bonus() + OPEN_GATE_HAN;
    assert_eq!(breakdown.final_han, han);
    assert!(breakdown.steps.iter().any(|s| s.source == "Open Gate"));
    assert!(breakdown.steps.iter().any(|s| s.source == "Tanyao"));
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
    assert_eq!(breakdown.final_fu, 120);
    assert_eq!(breakdown.final_han, 11.0);
    assert_eq!(breakdown.total, 1320);
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
fn multiplier_master_adds_one_and_half_mult_per_relic() {
    let hand = vec![
        Tile::new(Suit::Manzu, 9, 0),
        Tile::new(Suit::Manzu, 9, 1),
        Tile::new(Suit::Manzu, 9, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::MultiplierMaster]);
    let breakdown = score_sets(&hand, &sets, &ctx_with(&r, false), &[]);
    assert_eq!(breakdown.final_han, 2.5);

    let r2 = relics(vec![RelicId::MultiplierMaster, RelicId::ChainReaction]);
    let breakdown2 = score_sets(&hand, &sets, &ctx_with(&r2, false), &[]);
    assert_eq!(breakdown2.final_han, 4.0);

    let mut r3 = relics(vec![RelicId::MultiplierMaster, RelicId::ChainReaction]);
    r3.set_debuffed([RelicId::ChainReaction]);
    let breakdown3 = score_sets(&hand, &sets, &ctx_with(&r3, false), &[]);
    assert_eq!(breakdown3.final_han, 4.0);
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
    assert_eq!(with_echo.final_fu, base.final_fu + 45);
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
    assert_eq!(with_echo.final_fu, base.final_fu + 30);
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
    let base_fu = 15;
    let fu = with_tanyao_fu(base_fu);
    let han = with_tanyao_han(CHAIN_REACTION_HAN);
    assert_eq!(breakdown.final_han, han);
    assert_eq!(breakdown.total, combine(fu, han));
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
    assert_eq!(breakdown.final_han, with_tanyao_han(0.0));
}

#[test]
fn kindling_adds_mult_from_prior_cashins_this_chamber() {
    let hand = vec![
        Tile::new(Suit::Manzu, 5, 0),
        Tile::new(Suit::Manzu, 5, 1),
        Tile::new(Suit::Manzu, 5, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::Kindling]);
    let mut counters = std::collections::BTreeMap::new();
    counters.insert(RelicId::Kindling, 2);
    let ctx = ScoreContext {
        relic: ScoreRelicBundle {
            roster: &r,
            counters,
        },
        tiles: ScoreTileBundle {
            debuffs: &[],
            hand_for_ghost: &[],
        },
        round: ScoreRoundBundle {
            scored_last_turn: false,
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
    };
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    let prior_cashins = 2;
    let han = SCORING_BASE_HAN + YakuKind::Tanyao.han_bonus() + kindling_han_bonus(prior_cashins);
    assert_eq!(breakdown.final_han, han);
    assert!(breakdown.steps.iter().any(|s| s.source == "Kindling"));
}

#[test]
fn wall_weaver_adds_han_from_tiles_beyond_standard_wall() {
    let hand = vec![
        Tile::new(Suit::Manzu, 5, 0),
        Tile::new(Suit::Manzu, 5, 1),
        Tile::new(Suit::Manzu, 5, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::WallWeaver]);
    let mut counters = std::collections::BTreeMap::new();
    counters.insert(RelicId::WallWeaver, 8);
    let ctx = ScoreContext {
        relic: ScoreRelicBundle {
            roster: &r,
            counters,
        },
        tiles: ScoreTileBundle {
            debuffs: &[],
            hand_for_ghost: &[],
        },
        round: ScoreRoundBundle {
            scored_last_turn: false,
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
    };
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    let bonus = (0.35_f64 * 8.0).min(8.0);
    let han = SCORING_BASE_HAN + YakuKind::Tanyao.han_bonus() + bonus;
    assert_eq!(breakdown.final_han, han);
    assert!(breakdown.steps.iter().any(|s| s.source == "Wall Weaver"));
}

#[test]
fn wall_weaver_stacks_overflow_with_pack_extras() {
    let hand = vec![
        Tile::new(Suit::Manzu, 5, 0),
        Tile::new(Suit::Manzu, 5, 1),
        Tile::new(Suit::Manzu, 5, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::WallWeaver, RelicId::StrengthInNumbers]);
    let mut counters = std::collections::BTreeMap::new();
    counters.insert(RelicId::WallWeaver, 8);
    let ctx = ScoreContext {
        relic: ScoreRelicBundle {
            roster: &r,
            counters,
        },
        tiles: ScoreTileBundle {
            debuffs: &[],
            hand_for_ghost: &[],
        },
        round: ScoreRoundBundle {
            scored_last_turn: false,
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
    };
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    let bonus = (0.35 * (68 + 8) as f64).min(8.0);
    let han = SCORING_BASE_HAN + YakuKind::Tanyao.han_bonus() + bonus;
    assert_eq!(breakdown.final_han, han);
}

#[test]
fn dora_fu_per_matching_tile() {
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
    let prev_fu = if idx == 0 {
        breakdown.base_fu
    } else {
        breakdown.steps[idx - 1].running_fu
    };
    let dora_delta = breakdown.steps[idx].running_fu - prev_fu;
    assert_eq!(dora_delta, 300);
}

#[test]
fn dora_fu_per_matching_flower_tile() {
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
    let prev_fu = if idx == 0 {
        breakdown.base_fu
    } else {
        breakdown.steps[idx - 1].running_fu
    };
    let dora_delta = breakdown.steps[idx].running_fu - prev_fu;
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
    assert_eq!(breakdown.base_fu, 74);
    assert_eq!(breakdown.final_han, 14.0);
    assert_eq!(breakdown.total, 4648);
}

fn dora_fu_delta(breakdown: &ScoreBreakdown) -> i32 {
    let mut total = 0;
    let mut prev = breakdown.base_fu;
    for s in &breakdown.steps {
        let delta = s.running_fu - prev;
        if s.source.starts_with("Dora") {
            total += delta;
        }
        prev = s.running_fu;
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
    assert_eq!(dora_fu_delta(&breakdown), 450);
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
    assert_eq!(dora_fu_delta(&breakdown), 600);
}

fn garden_keeper_chips_delta(breakdown: &ScoreBreakdown) -> i32 {
    let mut total = 0;
    let mut prev = breakdown.base_fu;
    for s in &breakdown.steps {
        let delta = s.running_fu - prev;
        if s.source == "Garden Keeper" {
            total += delta;
        }
        prev = s.running_fu;
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
    assert_eq!(breakdown.final_fu, base.final_fu + 40);
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
fn chicken_hand_scores_on_partial_two_pair_structure() {
    use crate::core::structure::StructureTriggerMeta;
    use crate::core::yaku::YakuKind;
    let tiles = vec![
        Tile::new(Suit::Pinzu, 5, 0),
        Tile::new(Suit::Pinzu, 5, 1),
        Tile::new(Suit::Dragon, 1, 2),
        Tile::new(Suit::Dragon, 1, 3),
    ];
    let sets = vec![
        DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![0, 1],
        },
        DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![2, 3],
        },
    ];
    let r = RelicState::default();
    let mut ctx = ctx_with(&r, false);
    ctx.pattern.available_yaku = YakuKind::all()
        .iter()
        .copied()
        .filter(|&y| y != YakuKind::KokushiMusou)
        .collect();
    ctx.structure = Some(StructureTriggerMeta {
        meld_count: sets.len() as u32,
        inject_chicken_if_no_yaku: true,
    });
    let breakdown = score_sets(&tiles, &sets, &ctx, &[]);
    assert!(
        breakdown.detected_yaku.contains(&YakuKind::ChickenHand),
        "expected Chicken Hand on partial two-pair cash-in, got {:?}",
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
            .map(|s| s.running_fu)
            .last(),
        Some(breakdown.base_fu + 100)
    );
}

#[test]
fn polychrome_stamp_adds_0_25_mult_per_tile() {
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
        (breakdown.final_han - 1.5).abs() < f64::EPSILON,
        "expected +0.25 Han per stamped tile (pair -> +0.5), got {}",
        breakdown.final_han
    );
}

#[test]
fn dragon_echo_does_not_retrigger_pearl_meld_bonus() {
    let hand = vec![
        {
            let mut t = Tile::new(Suit::Dragon, 1, 0);
            t.enhancement = Some(TileEnhancement::Pearl);
            t
        },
        {
            let mut t = Tile::new(Suit::Dragon, 1, 1);
            t.enhancement = Some(TileEnhancement::Pearl);
            t
        },
        {
            let mut t = Tile::new(Suit::Dragon, 1, 2);
            t.enhancement = Some(TileEnhancement::Pearl);
            t
        },
    ];
    let sets = find_pairs_and_triplets(&hand);
    let base = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    let with_echo = score_sets(
        &hand,
        &sets,
        &ctx_with(&relics(vec![RelicId::DragonEcho]), false),
        &[],
    );
    assert_eq!(
        base.steps
            .iter()
            .filter(|s| s.source == "Pearl Talisman")
            .count(),
        1
    );
    assert_eq!(
        with_echo
            .steps
            .iter()
            .filter(|s| s.source == "Pearl Talisman")
            .count(),
        1
    );
    assert_eq!(with_echo.final_fu, base.final_fu + 45);
}

#[test]
fn dragon_echo_does_not_retrigger_polychrome_han_bonus() {
    let hand = vec![
        {
            let mut t = Tile::new(Suit::Dragon, 1, 0);
            t.enhancement = Some(TileEnhancement::Polychrome);
            t
        },
        {
            let mut t = Tile::new(Suit::Dragon, 1, 1);
            t.enhancement = Some(TileEnhancement::Polychrome);
            t
        },
        {
            let mut t = Tile::new(Suit::Dragon, 1, 2);
            t.enhancement = Some(TileEnhancement::Polychrome);
            t
        },
    ];
    let sets = find_pairs_and_triplets(&hand);
    let base = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    let with_echo = score_sets(
        &hand,
        &sets,
        &ctx_with(&relics(vec![RelicId::DragonEcho]), false),
        &[],
    );
    assert_eq!(
        base.steps
            .iter()
            .filter(|s| s.source == "Polychrome Talisman")
            .count(),
        3
    );
    assert_eq!(
        with_echo
            .steps
            .iter()
            .filter(|s| s.source == "Polychrome Talisman")
            .count(),
        3
    );
    assert_eq!(with_echo.final_han, base.final_han);
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
    assert_eq!(with_echo.final_fu, base.final_fu + 45);
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
        with_crown.final_han,
        base.final_han + 4.0 * yaku_count as f64
    );
}

#[test]
fn geese_retriggers_first_five_melds() {
    let hand: Vec<Tile> = (0..12)
        .map(|i| Tile::new(Suit::Pinzu, (i / 2 + 1) as u8, i as u32))
        .collect();
    let sets = find_pairs_and_triplets(&hand);
    assert_eq!(sets.len(), 6);
    let base = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    let with_geese = score_sets(
        &hand,
        &sets,
        &ctx_with(&relics(vec![RelicId::Geese]), false),
        &[],
    );
    assert_eq!(with_geese.final_fu, base.final_fu + 30);
    assert!(with_geese.steps.iter().any(|s| s.source == "Geese"));
}

#[test]
fn geese_full_meld_beats_partial_tiles() {
    let hand = vec![
        Tile::new(Suit::Pinzu, 9, 0),
        Tile::new(Suit::Pinzu, 9, 1),
        Tile::new(Suit::Pinzu, 9, 2),
        Tile::new(Suit::Pinzu, 9, 3),
        Tile::new(Suit::Manzu, 1, 4),
        Tile::new(Suit::Manzu, 2, 5),
        Tile::new(Suit::Manzu, 3, 6),
    ];
    let sets = vec![
        DetectedMeld {
            kind: MeldKind::Kong,
            tile_ids: vec![0, 1, 2, 3],
        },
        DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![4, 5, 6],
        },
    ];
    let base = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    let with_geese = score_sets(
        &hand,
        &sets,
        &ctx_with(&relics(vec![RelicId::Geese]), false),
        &[],
    );
    assert_eq!(with_geese.final_fu, base.final_fu + 42);
}

#[test]
fn xxxl_egg_retriggers_all_melds() {
    let hand = vec![
        Tile::new(Suit::Souzu, 3, 0),
        Tile::new(Suit::Souzu, 3, 1),
        Tile::new(Suit::Souzu, 3, 2),
        Tile::new(Suit::Dragon, 1, 3),
        Tile::new(Suit::Dragon, 1, 4),
        Tile::new(Suit::Dragon, 1, 5),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::XxxlEgg]);
    let mut counters = std::collections::BTreeMap::new();
    counters.insert(RelicId::XxxlEgg, 2);
    let ctx = ScoreContext {
        relic: ScoreRelicBundle {
            roster: &r,
            counters,
        },
        tiles: ScoreTileBundle {
            debuffs: &[],
            hand_for_ghost: &[],
        },
        round: ScoreRoundBundle {
            scored_last_turn: false,
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
    };
    let base = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    let with_egg = score_sets(&hand, &sets, &ctx, &[]);
    assert_eq!(with_egg.final_fu, base.final_fu + base.base_fu);
    assert!(with_egg.steps.iter().any(|s| s.source == "XXXL Egg"));
}

#[test]
fn easter_egg_retriggers_on_chicken_hand() {
    use crate::core::structure::StructureTriggerMeta;
    let tiles = vec![
        Tile::new(Suit::Pinzu, 5, 0),
        Tile::new(Suit::Pinzu, 5, 1),
        Tile::new(Suit::Dragon, 1, 2),
        Tile::new(Suit::Dragon, 1, 3),
    ];
    let sets = vec![
        DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![0, 1],
        },
        DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![2, 3],
        },
    ];
    let r = relics(vec![RelicId::EasterEgg]);
    let mut ctx = ctx_with(&r, false);
    ctx.pattern.available_yaku = YakuKind::all()
        .iter()
        .copied()
        .filter(|&y| y != YakuKind::KokushiMusou)
        .collect();
    ctx.structure = Some(StructureTriggerMeta {
        meld_count: sets.len() as u32,
        inject_chicken_if_no_yaku: true,
    });
    let base = score_sets(&tiles, &sets, &ctx_with(&RelicState::default(), false), &[]);
    let with_egg = score_sets(&tiles, &sets, &ctx, &[]);
    assert_eq!(with_egg.final_fu, base.final_fu + base.base_fu);
    assert!(with_egg.steps.iter().any(|s| s.source == "Easter Egg"));
}

#[test]
fn easter_egg_does_not_retrigger_with_yaku() {
    use crate::core::structure::StructureTriggerMeta;
    let hand = vec![
        Tile::new(Suit::Souzu, 3, 0),
        Tile::new(Suit::Souzu, 3, 1),
        Tile::new(Suit::Souzu, 3, 2),
        Tile::new(Suit::Dragon, 1, 3),
        Tile::new(Suit::Dragon, 1, 4),
        Tile::new(Suit::Dragon, 1, 5),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let r = relics(vec![RelicId::EasterEgg]);
    let mut ctx = ctx_with(&r, false);
    ctx.structure = Some(StructureTriggerMeta {
        meld_count: sets.len() as u32,
        inject_chicken_if_no_yaku: true,
    });
    let breakdown = score_sets(&hand, &sets, &ctx, &[]);
    assert!(
        !breakdown.detected_yaku.contains(&YakuKind::ChickenHand),
        "expected a real yaku, not Chicken Hand, got {:?}",
        breakdown.detected_yaku
    );
    assert!(
        !breakdown.steps.iter().any(|s| s.source == "Easter Egg"),
        "Easter Egg should not retrigger when Chicken Hand is not scored"
    );
}

#[test]
fn voice_of_the_people_retriggers_whole_low_meld() {
    let hand = vec![
        Tile::new(Suit::Manzu, 2, 0),
        Tile::new(Suit::Manzu, 2, 1),
        Tile::new(Suit::Manzu, 2, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let base = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    let with_voice = score_sets(
        &hand,
        &sets,
        &ctx_with(&relics(vec![RelicId::VoiceOfThePeople]), false),
        &[],
    );
    assert_eq!(with_voice.final_fu, base.final_fu + base.base_fu);
    assert!(
        with_voice
            .steps
            .iter()
            .any(|s| s.source == "Voice of the People")
    );
}

#[test]
fn voice_of_the_people_skips_mixed_rank_meld() {
    let hand = vec![
        Tile::new(Suit::Manzu, 3, 0),
        Tile::new(Suit::Manzu, 4, 1),
        Tile::new(Suit::Manzu, 5, 2),
    ];
    let sets = vec![DetectedMeld {
        kind: MeldKind::Sequence,
        tile_ids: vec![0, 1, 2],
    }];
    let base = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    let with_voice = score_sets(
        &hand,
        &sets,
        &ctx_with(&relics(vec![RelicId::VoiceOfThePeople]), false),
        &[],
    );
    assert_eq!(with_voice.final_fu, base.final_fu);
    assert!(
        !with_voice
            .steps
            .iter()
            .any(|s| s.source == "Voice of the People")
    );
}

#[test]
fn voice_of_the_elite_retriggers_whole_high_meld() {
    let hand = vec![
        Tile::new(Suit::Souzu, 8, 0),
        Tile::new(Suit::Souzu, 8, 1),
        Tile::new(Suit::Souzu, 8, 2),
    ];
    let sets = find_pairs_and_triplets(&hand);
    let base = score_sets(&hand, &sets, &ctx_with(&RelicState::default(), false), &[]);
    let with_voice = score_sets(
        &hand,
        &sets,
        &ctx_with(&relics(vec![RelicId::VoiceOfTheElite]), false),
        &[],
    );
    assert_eq!(with_voice.final_fu, base.final_fu + base.base_fu);
    assert!(
        with_voice
            .steps
            .iter()
            .any(|s| s.source == "Voice of the Elite")
    );
}
