use mahjuro::game::run::RunState;

/// Replace `run`'s hand with a curated hero rack for marketing captures.
#[cfg(feature = "screenshot")]
pub(crate) fn setup_hero_state(run: &mut RunState) {
    use mahjuro::core::relic::RelicId;
    use mahjuro::core::tile::{Suit, Tile};
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

#[cfg(feature = "screenshot")]
pub(crate) fn setup_shop_state(run: &mut RunState) {
    mahjuro::room_bake::setup_shop_state(run);
}

#[cfg(feature = "screenshot")]
pub(crate) fn setup_defeat_game_over_screenshot_state(run: &mut RunState) {
    use mahjuro::core::ordeal::OrdealKind;
    use mahjuro::core::rules::ChamberKind;
    use mahjuro::core::yaku::YakuKind;

    run.wing = 6;
    run.run_number = 18;
    run.chamber = ChamberKind::Ordeal;
    run.ordeal.upcoming = Some(OrdealKind::Rot);
    run.round_score = 5674;
    run.target_score = 10486;
    run.yen = 12;
    run.tiles_played = 170;
    run.tiles_discarded = 97;
    run.times_restocked = 44;
    run.best_structure_score = 6577;
    run.best_structure_name = ".tsu".into();
    run.yaku_times_played.insert(YakuKind::FullHand, 6);
}

#[cfg(feature = "screenshot")]
pub(crate) fn setup_victory_game_over_screenshot_state(run: &mut RunState) {
    use mahjuro::core::ordeal::OrdealKind;
    use mahjuro::core::rules::ChamberKind;
    use mahjuro::core::yaku::YakuKind;

    run.wing = 8;
    run.run_number = 22;
    run.chamber = ChamberKind::Ordeal;
    run.ordeal.upcoming = Some(OrdealKind::House);
    run.round_score = 12490;
    run.target_score = 11900;
    run.yen = 28;
    run.tiles_played = 209;
    run.tiles_discarded = 76;
    run.times_restocked = 39;
    run.best_structure_score = 8312;
    run.best_structure_name = "Toitoi".into();
    run.yaku_times_played.insert(YakuKind::Toitoi, 8);
}

#[cfg(feature = "screenshot")]
pub(crate) fn setup_gameplay_screenshot_state(run: &mut RunState) {
    mahjuro::room_bake::setup_gameplay_bake_state(run);
}

#[cfg(feature = "screenshot")]
pub(crate) fn force_ordeal_chamber(run: &mut RunState, kind: mahjuro::core::ordeal::OrdealKind) {
    use mahjuro::OrdealKindExt;
    run.chamber = mahjuro::core::rules::ChamberKind::Ordeal;
    run.upcoming_chamber = mahjuro::core::rules::ChamberKind::Ordeal;
    run.wing = kind.def().min_wing.max(run.wing);
    run.ordeal.upcoming = Some(kind);
    run.resolve_upcoming_ordeal();
    run.apply_chamber(mahjuro::core::rules::ChamberKind::Ordeal, None);
}

/// Prime shop stock generation (Qilin ribbon gate uses profile progress).
#[cfg(feature = "screenshot")]
pub(crate) fn prime_shop_stock(
    run: &mut RunState,
    progress: &mahjuro::core::progression::PlayerProgress,
) {
    use mahjuro::scenes::shop::ShopScene;
    let _ = ShopScene::new(run, progress);
}
