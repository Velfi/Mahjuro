//! Run-state fixtures for offline room lighting bakes.

use mahjuro::core::consumable::Consumable;
use mahjuro::core::hand::{DetectedMeld, MeldKind};
use mahjuro::core::relic::RelicId;
use mahjuro::core::talisman::TalismanKind;
use mahjuro::core::tile::{Suit, Tile};
use mahjuro::core::zodiac::ZodiacKind;
use mahjuro::game::run::RunState;
use mahjuro::main_render_settings::RenderSettings;
use mahjuro::persistence::AppSettings;

pub fn setup_shop_state(run: &mut RunState) {
    run.yen = 42;
    run.run_number = 3;
    run.wing = 3;
    run.tag_rich_stock = 1;
}

fn setup_hero_hand(run: &mut RunState) {
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

pub fn setup_gameplay_bake_state(run: &mut RunState) {
    setup_hero_hand(run);
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

/// Deterministic render settings for offline bakes (fresh-install defaults, not local `app_settings.json`).
pub fn bake_render_settings() -> RenderSettings {
    let settings = AppSettings::default();
    RenderSettings {
        effects_quality: settings.effects_quality,
        tile_preset: settings.tile_preset,
        tile_material: settings.tile_material,
        tileset_name: settings.tileset_name.clone(),
        gamma: settings.gamma,
        graphics_mode: settings.graphics_mode,
        hdr_enabled: settings.hdr_enabled,
        vhs_enabled: false,
    }
}

/// Deterministic profile for offline bakes (empty profile, not local save data).
pub fn bake_player_progress() -> mahjuro::core::progression::PlayerProgress {
    mahjuro::core::progression::PlayerProgress::default()
}
