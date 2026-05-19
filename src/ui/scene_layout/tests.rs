use super::collection::collection_field_path;
use super::gameplay::gameplay_field_path;
use super::main_menu_exterior::main_menu_exterior_field_path;
use super::shop::shop_field_path;
use super::tile_select::tile_select_field_path;
use super::tutorial::tutorial_field_path;
use super::*;
use crate::ui::{
    placement::{ArrangeDelta, ArrangeTarget, apply_arrange},
    scene_layout::{
        collection::{COLLECTION_HIERARCHY, CollectionField},
        gameplay::{GAMEPLAY_HIERARCHY, GameplayField, sanitize_gameplay_positions},
        main_menu_exterior::{MAIN_MENU_EXTERIOR_HIERARCHY, MainMenuExteriorField},
        shop::{SHOP_HIERARCHY, ShopField, sanitize_shop_positions},
        tile_select::{TILE_SELECT_HIERARCHY, TileSelectField},
        tutorial::{TUTORIAL_HIERARCHY, TutorialField},
    },
};

const EPS: f32 = 1e-5;

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() < EPS
}

#[test]
fn shop_positions_serde_roundtrip() {
    let orig = ShopPositions::default();
    let json = serde_json::to_string(&orig).unwrap();
    let restored: ShopPositions = serde_json::from_str(&json).unwrap();
    assert!(approx(restored.counter.nx, orig.counter.nx));
    assert!(approx(restored.relics.nx, orig.relics.nx));
    assert!(approx(restored.lamp.lift_mm, orig.lamp.lift_mm));
}

#[test]
fn gameplay_positions_serde_roundtrip() {
    let orig = GameplayPositions::default();
    let json = serde_json::to_string(&orig).unwrap();
    let restored: GameplayPositions = serde_json::from_str(&json).unwrap();
    assert!(approx(restored.relic_col.nx, orig.relic_col.nx));
    assert!(approx(restored.dora.nx, orig.dora.nx));
    assert!(approx(restored.hand_strip.ny, orig.hand_strip.ny));
    assert!(approx(restored.plaque.lift_mm, orig.plaque.lift_mm));
}

#[test]
fn shop_positions_sparse_json_uses_defaults() {
    let json = r#"{ "counter": { "nx": 0.42 } }"#;
    let p: ShopPositions = serde_json::from_str(json).unwrap();
    assert!(approx(p.counter.nx, 0.42));
    let default = ShopPositions::default();
    assert!(approx(p.relics.nx, default.relics.nx));
    assert!(approx(p.lamp.lift_mm, default.lamp.lift_mm));
}

#[test]
fn arrange_counter_via_generic_handler() {
    let mut p = ShopPositions::default();
    let before = p.counter.nx;
    let ok = apply_arrange(
        &mut p,
        "shop.counter",
        ArrangeDelta {
            dnx: 0.01,
            ..Default::default()
        },
    );
    assert!(ok);
    assert!(approx(p.counter.nx, before + 0.01));
}

#[test]
fn arrange_hand_strip_accumulates_rotation() {
    let mut p = GameplayPositions::default();
    let before_rx = p.hand_strip.rx_deg;
    let ok = apply_arrange(
        &mut p,
        "gameplay.hand.strip",
        ArrangeDelta {
            d_rx_deg: 2.5,
            ..Default::default()
        },
    );
    assert!(ok);
    assert!(approx(p.hand_strip.rx_deg, before_rx + 2.5));
}

#[test]
fn arrange_bowl_is_a_regular_placement() {
    let mut p = GameplayPositions::default();
    let before_nx = p.bowl.nx;
    let ok = apply_arrange(
        &mut p,
        "gameplay.action_bar.bowl",
        ArrangeDelta {
            dnx: 0.01,
            ..Default::default()
        },
    );
    assert!(ok);
    assert!(approx(p.bowl.nx, before_nx + 0.01));
}

#[test]
fn arrange_shop_group_moves_every_child_column() {
    let mut p = ShopPositions::default();
    let before_relics = p.relics.nx;
    let before_packs = p.packs.nx;
    let before_talismans = p.talismans.nx;
    let before_ribbons = p.ribbons.nx;
    let ok = apply_arrange(
        &mut p,
        "shop.for_sale",
        ArrangeDelta {
            dnx: 0.01,
            ..Default::default()
        },
    );
    assert!(ok);
    assert!(approx(p.relics.nx, before_relics + 0.01));
    assert!(approx(p.packs.nx, before_packs + 0.01));
    assert!(approx(p.talismans.nx, before_talismans + 0.01));
    assert!(approx(p.ribbons.nx, before_ribbons + 0.01));
}

#[test]
fn arrange_shop_dotted_path_matches_leaf() {
    let mut p = ShopPositions::default();
    let before = p.counter.nx;
    let ok = apply_arrange(
        &mut p,
        "shop.counter",
        ArrangeDelta {
            dnx: 0.01,
            ..Default::default()
        },
    );
    assert!(ok);
    assert!(approx(p.counter.nx, before + 0.01));
}

#[test]
fn arrange_gameplay_group_moves_hand_area() {
    let mut p = GameplayPositions::default();
    let before_strip_rx = p.hand_strip.rx_deg;
    let before_yaku_rx = p.yaku_tablet.rx_deg;
    let ok = apply_arrange(
        &mut p,
        "gameplay.hand",
        ArrangeDelta {
            d_rx_deg: 1.0,
            ..Default::default()
        },
    );
    assert!(ok);
    assert!(approx(p.hand_strip.rx_deg, before_strip_rx + 1.0));
    assert!(approx(p.yaku_tablet.rx_deg, before_yaku_rx + 1.0));
}

#[test]
fn arrange_plaque_is_a_regular_placement() {
    let mut p = GameplayPositions::default();
    let before_ny = p.plaque.ny;
    let ok = apply_arrange(
        &mut p,
        "gameplay.score_panel.plaque",
        ArrangeDelta {
            dny: 0.01,
            ..Default::default()
        },
    );
    assert!(ok);
    assert!(approx(p.plaque.ny, before_ny + 0.01));
}

#[test]
fn shop_hierarchy_leaves_all_resolve() {
    use crate::ui::placement::all_leaf_names;
    let mut p = ShopPositions::default();
    for leaf in all_leaf_names(SHOP_HIERARCHY) {
        assert!(p.placement_mut(leaf).is_some());
    }
}

#[test]
fn shop_field_path_roundtrip() {
    for &field in ShopField::ALL {
        let path = shop_field_path(field);
        assert_eq!(super::shop::lookup_shop_field(path), Some(field));
    }
}

#[test]
fn shop_field_paths_all_in_hierarchy() {
    use crate::ui::placement::all_leaf_names;
    let leaves: Vec<&'static str> = all_leaf_names(SHOP_HIERARCHY);
    for &field in ShopField::ALL {
        let path = shop_field_path(field);
        assert!(leaves.contains(&path));
    }
}

#[test]
fn gameplay_hierarchy_leaves_all_resolve() {
    use crate::ui::placement::all_leaf_names;
    let mut p = GameplayPositions::default();
    for leaf in all_leaf_names(GAMEPLAY_HIERARCHY) {
        assert!(p.placement_mut(leaf).is_some());
    }
}

#[test]
fn gameplay_field_path_roundtrip() {
    for &field in GameplayField::ALL {
        let path = gameplay_field_path(field);
        assert_eq!(super::gameplay::lookup_gameplay_field(path), Some(field));
    }
}

#[test]
fn gameplay_field_paths_all_in_hierarchy() {
    use crate::ui::placement::all_leaf_names;
    let leaves: Vec<&'static str> = all_leaf_names(GAMEPLAY_HIERARCHY);
    for &field in GameplayField::ALL {
        let path = gameplay_field_path(field);
        assert!(leaves.contains(&path));
    }
}

#[test]
fn sanitize_shop_restores_non_finite_fields() {
    let mut p = ShopPositions::default();
    let default_counter_nx = p.counter.nx;
    p.counter.nx = f32::NAN;
    p.lamp.lift_mm = f32::INFINITY;
    sanitize_shop_positions(&mut p);
    assert!(approx(p.counter.nx, default_counter_nx));
    assert!(p.lamp.lift_mm.is_finite());
}

#[test]
fn sanitize_gameplay_restores_non_finite_fields() {
    let mut p = GameplayPositions::default();
    let default_dora_ny = p.dora.ny;
    p.dora.ny = f32::NAN;
    sanitize_gameplay_positions(&mut p);
    assert!(approx(p.dora.ny, default_dora_ny));
}

#[test]
fn sanitize_leaves_valid_placements_alone() {
    let mut p = ShopPositions::default();
    p.counter.nx = 0.42;
    sanitize_shop_positions(&mut p);
    assert!(approx(p.counter.nx, 0.42));
}

#[test]
fn collection_field_path_roundtrip() {
    for &field in CollectionField::ALL {
        let path = collection_field_path(field);
        assert_eq!(
            super::collection::lookup_collection_field(path),
            Some(field)
        );
    }
}

#[test]
fn collection_hierarchy_leaves_all_resolve() {
    use crate::ui::placement::all_leaf_names;
    let mut p = CollectionPositions::default();
    for leaf in all_leaf_names(COLLECTION_HIERARCHY) {
        assert!(p.placement_mut(leaf).is_some());
    }
}

#[test]
fn main_menu_exterior_field_path_roundtrip() {
    for &field in MainMenuExteriorField::ALL {
        let path = main_menu_exterior_field_path(field);
        assert_eq!(
            super::main_menu_exterior::lookup_main_menu_exterior_field(path),
            Some(field)
        );
    }
}

#[test]
fn main_menu_exterior_hierarchy_leaves_all_resolve() {
    use crate::ui::placement::all_leaf_names;
    let mut p = MainMenuExteriorPositions::default();
    for leaf in all_leaf_names(MAIN_MENU_EXTERIOR_HIERARCHY) {
        assert!(p.placement_mut(leaf).is_some());
    }
}

#[test]
fn tutorial_field_path_roundtrip() {
    for &field in TutorialField::ALL {
        let path = tutorial_field_path(field);
        assert_eq!(super::tutorial::lookup_tutorial_field(path), Some(field));
    }
}

#[test]
fn tutorial_hierarchy_leaves_all_resolve() {
    use crate::ui::placement::all_leaf_names;
    let mut p = TutorialPositions::default();
    for leaf in all_leaf_names(TUTORIAL_HIERARCHY) {
        assert!(p.placement_mut(leaf).is_some());
    }
}

#[test]
fn tile_select_field_path_roundtrip() {
    for &field in TileSelectField::ALL {
        let path = tile_select_field_path(field);
        assert_eq!(
            super::tile_select::lookup_tile_select_field(path),
            Some(field)
        );
    }
}

#[test]
fn gameplay_dora_round_wind_accept_full_arrange_delta() {
    let mut p = GameplayPositions::default();
    let d0 = p.dora;
    let rw0 = p.round_wind;
    assert!(apply_arrange(
        &mut p,
        "gameplay.dora",
        ArrangeDelta {
            dnx: 0.01,
            dny: -0.02,
            d_lift_mm: 1.5,
            d_rx_deg: 2.0,
            d_ry_deg: -3.0,
            d_rz_deg: 5.0,
        },
    ));
    assert!(approx(p.dora.nx, d0.nx + 0.01));
    assert!(approx(p.dora.ny, d0.ny - 0.02));
    assert!(approx(p.dora.lift_mm, d0.lift_mm + 1.5));
    assert!(approx(p.dora.rx_deg, d0.rx_deg + 2.0));
    assert!(approx(p.dora.ry_deg, d0.ry_deg - 3.0));
    assert!(approx(p.dora.rz_deg, d0.rz_deg + 5.0));

    assert!(apply_arrange(
        &mut p,
        "gameplay.round_wind",
        ArrangeDelta {
            d_rx_deg: 1.0,
            ..Default::default()
        },
    ));
    assert!(approx(p.round_wind.rx_deg, rw0.rx_deg + 1.0));
}

/// `object3d_placement` applies arrange overrides under these names; they are
/// not (yet) backed by `GameplayPositions` / `GAMEPLAY_HIERARCHY`, so debug
/// arrange cannot persist nudges for them — only live preview if we added keys.
#[test]
fn gameplay_arrange_ephemeral_renderer_paths_not_in_hierarchy() {
    use crate::ui::scene_layout::gameplay::lookup_gameplay_field;
    for path in [
        "gameplay.score_popup",
        "gameplay.cascade_token.chips",
        "gameplay.cascade_token.mult",
        "gameplay.wall_tile",
        "gameplay.action_bar.tablet",
    ] {
        assert!(
            lookup_gameplay_field(path).is_none(),
            "expected no persisted placement for {path}"
        );
    }
}

#[test]
fn tile_select_hierarchy_leaves_all_resolve() {
    use crate::ui::placement::all_leaf_names;
    let mut p = TileSelectPositions::default();
    for leaf in all_leaf_names(TILE_SELECT_HIERARCHY) {
        assert!(p.placement_mut(leaf).is_some());
    }
}

