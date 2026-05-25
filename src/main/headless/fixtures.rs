use crate::game::run::RunState;
use crate::scenes::shop::ShopScene;

/// Replace `run`'s hand with a curated hero rack for marketing captures.
pub(crate) fn setup_hero_state(run: &mut RunState) {
    use crate::core::relic::RelicId;
    use crate::core::tile::{Suit, Tile};
    *run.hand_mut() = vec![
        Tile::new(Suit::Dragon, 1, 100),
        Tile::new(Suit::Dragon, 1, 101),
        Tile::new(Suit::Dragon, 1, 102),
        Tile::new(Suit::Dragon, 3, 103),
        Tile::new(Suit::Dragon, 3, 104),
        Tile::new(Suit::Dragon, 3, 105),
        Tile::new(Suit::Souzu, 5, 106),
        Tile::new(Suit::Souzu, 6, 107),
        Tile::new(Suit::Souzu, 7, 108),
        Tile::new(Suit::Pinzu, 1, 109),
        Tile::new(Suit::Pinzu, 2, 110),
        Tile::new(Suit::Pinzu, 3, 111),
        Tile::new(Suit::Wind, 1, 112),
        Tile::new(Suit::Wind, 1, 113),
    ];
    run.hand_mut().sort();
    *run.selected_mut() = vec![true; run.hand().len()];

    run.relics.active.clear();
    for r in [
        RelicId::DragonRage,
        RelicId::WhiteDragonsHush,
        RelicId::GreenLuck,
        RelicId::GoldenEngine,
    ] {
        if !run.relics.is_full() {
            run.relics.active.push(r);
        }
    }
}

pub(crate) fn setup_shop_state(run: &mut RunState) {
    run.gold = 42;
    run.run_number = 3;
    run.wing = 3;
    run.tag_rich_stock = true;
}

pub(crate) fn setup_defeat_game_over_screenshot_state(run: &mut RunState) {
    use crate::core::ordeal::OrdealKind;
    use crate::core::rules::ChamberKind;
    use crate::core::yaku::YakuKind;

    run.wing = 6;
    run.run_number = 18;
    run.chamber = ChamberKind::Ordeal;
    run.ordeal.upcoming = Some(OrdealKind::Rot);
    run.round_score = 5674;
    run.target_score = 10486;
    run.gold = 12;
    run.tiles_played = 170;
    run.tiles_discarded = 97;
    run.times_restocked = 44;
    run.best_structure_score = 6577;
    run.best_structure_name = ".tsu".into();
    run.yaku_times_played.insert(YakuKind::FullHand, 6);
}

pub(crate) fn setup_victory_game_over_screenshot_state(run: &mut RunState) {
    use crate::core::ordeal::OrdealKind;
    use crate::core::rules::ChamberKind;
    use crate::core::yaku::YakuKind;

    run.wing = 8;
    run.run_number = 22;
    run.chamber = ChamberKind::Ordeal;
    run.ordeal.upcoming = Some(OrdealKind::House);
    run.round_score = 12490;
    run.target_score = 11900;
    run.gold = 28;
    run.tiles_played = 209;
    run.tiles_discarded = 76;
    run.times_restocked = 39;
    run.best_structure_score = 8312;
    run.best_structure_name = "Toitoi".into();
    run.yaku_times_played.insert(YakuKind::Toitoi, 8);
}

pub(crate) fn setup_gameplay_screenshot_state(run: &mut RunState) {
    use crate::core::consumable::Consumable;
    use crate::core::hand::{DetectedMeld, MeldKind};
    use crate::core::relic::RelicId;
    use crate::core::talisman::TalismanKind;
    use crate::core::tile::{Suit, Tile};
    use crate::core::zodiac::ZodiacKind;

    setup_hero_state(run);
    *run.selected_mut() = vec![false; run.hand().len()];

    run.set_auto_cash_in_on_full_structure(false);
    *run.structure_tiles_mut() = vec![
        Tile::new(Suit::Manzu, 1, 1),
        Tile::new(Suit::Manzu, 1, 2),
        Tile::new(Suit::Manzu, 2, 3),
        Tile::new(Suit::Manzu, 3, 4),
        Tile::new(Suit::Manzu, 4, 5),
        Tile::new(Suit::Pinzu, 2, 6),
        Tile::new(Suit::Pinzu, 3, 7),
        Tile::new(Suit::Pinzu, 4, 8),
        Tile::new(Suit::Souzu, 5, 9),
        Tile::new(Suit::Souzu, 6, 10),
        Tile::new(Suit::Souzu, 7, 11),
        Tile::new(Suit::Wind, 1, 12),
        Tile::new(Suit::Wind, 1, 13),
        Tile::new(Suit::Wind, 1, 14),
    ];
    *run.structure_sets_mut() = vec![
        DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![1, 2],
        },
        DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![3, 4, 5],
        },
        DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![6, 7, 8],
        },
        DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![9, 10, 11],
        },
        DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![12, 13, 14],
        },
    ];

    if !run.relics.is_full() {
        run.relics.active.push(RelicId::Geese);
    }

    run.consumables.items.clear();
    let _ = run
        .consumables
        .try_push(Consumable::Talisman(TalismanKind::Pearl));
    let _ = run
        .consumables
        .try_push(Consumable::Zodiac(ZodiacKind::Dragon));
}

pub(crate) fn force_ordeal_chamber(run: &mut RunState, kind: crate::core::ordeal::OrdealKind) {
    run.chamber = crate::core::rules::ChamberKind::Ordeal;
    run.upcoming_chamber = crate::core::rules::ChamberKind::Ordeal;
    run.wing = kind.def().min_wing.max(run.wing);
    run.ordeal.upcoming = Some(kind);
    run.resolve_upcoming_ordeal();
    run.apply_chamber(crate::core::rules::ChamberKind::Ordeal, None);
}

/// Prime shop stock generation (Qilin ribbon gate uses profile progress).
pub(crate) fn prime_shop_stock(
    run: &mut RunState,
    progress: &crate::core::progression::PlayerProgress,
) {
    let _ = ShopScene::new(run, progress);
}
