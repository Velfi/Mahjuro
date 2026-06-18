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
pub(crate) fn setup_wall_ledger_screenshot_state(run: &mut RunState) {
    use mahjuro::core::debuff::TileDebuff;
    use mahjuro::core::deck::{Wall, build_wall};
    use mahjuro::core::tile::Suit;
    use mahjuro::core::tile_pack::{TilePackInstance, TilePackKind};

    setup_shop_state(run);
    run.tile_packs
        .push(TilePackInstance::new(TilePackKind::Souzu));
    run.wall = Wall::from_unshuffled(build_wall());
    for _ in 0..88 {
        run.wall.draw();
    }
    run.tile_debuffs.push(TileDebuff::Suit(Suit::Manzu));
}

#[cfg(feature = "screenshot")]
pub(crate) fn setup_shop_state(run: &mut RunState) {
    crate::room_bake::setup_shop_state(run);
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
    run.best_structure_name = "Ittsu".into();
    run.yaku_times_played.insert(YakuKind::Shousangen, 6);
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
    crate::room_bake::setup_gameplay_bake_state(run);
}

/// Gameplay with a valid meld selected (green dragon triplet) for mirror-glow captures.
#[cfg(feature = "screenshot")]
pub(crate) fn setup_gameplay_valid_play_screenshot_state(run: &mut RunState) {
    use mahjuro::core::tile::{Suit, Tile};

    setup_gameplay_screenshot_state(run);
    run.structure_tiles_mut().clear();
    run.structure_sets_mut().clear();
    let hand = run.hand_mut();
    if hand.len() >= 3 {
        hand[0] = Tile::new(Suit::Souzu, 5, 400);
        hand[1] = Tile::new(Suit::Souzu, 6, 401);
        hand[2] = Tile::new(Suit::Souzu, 7, 402);
        hand.sort();
    }
    let hand_len = run.hand().len();
    let mut pick = vec![false; hand_len];
    let mut souzu_seq = [5u8, 6, 7];
    for (i, tile) in run.hand().iter().enumerate() {
        if tile.suit == Suit::Souzu {
            if let Some(slot) = souzu_seq.iter().position(|&rank| rank == tile.rank) {
                pick[i] = true;
                souzu_seq[slot] = 0;
            }
        }
    }
    *run.selected_mut() = pick;
    run.plays_remaining = run.plays_remaining.max(3);
    debug_assert!(run.is_selection_valid());
}

/// Gameplay backdrop for the round-win celebration modal capture.
#[cfg(feature = "screenshot")]
pub(crate) fn setup_round_win_screenshot_state(run: &mut RunState) {
    use mahjuro::core::rules::ChamberKind;

    setup_gameplay_screenshot_state(run);
    run.wing = 2;
    run.chamber = ChamberKind::Small;
    run.round_score = 4688;
    run.target_score = 1000;
    run.yen = 18;
    run.plays_remaining = 0;
    run.discards_remaining = 4;
}

/// Gameplay backdrop for the game-over loss modal capture.
#[cfg(feature = "screenshot")]
pub(crate) fn setup_game_over_screenshot_state(run: &mut RunState) {
    use mahjuro::core::rules::ChamberKind;

    setup_gameplay_screenshot_state(run);
    run.wing = 2;
    run.chamber = ChamberKind::Small;
    run.round_score = 840;
    run.target_score = 1000;
    run.yen = 18;
    run.plays_remaining = 0;
    run.discards_remaining = 0;
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
