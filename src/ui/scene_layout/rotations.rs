//! Per-scene maps of placement rotation degrees for the renderer.

use rustc_hash::FxHashMap;

use crate::ui::placement::Placement;

use super::{
    CollectionPositions, GameplayPositions, MainMenuExteriorPositions, ShopPositions,
    TileSelectPositions, TutorialPositions,
};

fn insert_rotation(out: &mut FxHashMap<String, [f32; 3]>, name: &str, p: &Placement) {
    if p.rx_deg != 0.0 || p.ry_deg != 0.0 || p.rz_deg != 0.0 {
        out.insert(name.to_string(), [p.rx_deg, p.ry_deg, p.rz_deg]);
    }
}

impl GameplayPositions {
    pub fn committed_rotations(&self) -> FxHashMap<String, [f32; 3]> {
        use super::gameplay::{gameplay_field_path, GameplayField};
        let mut out = FxHashMap::default();
        for &field in GameplayField::ALL {
            insert_rotation(
                &mut out,
                gameplay_field_path(field),
                self.field_ref(field),
            );
        }
        out
    }
}

impl CollectionPositions {
    pub fn committed_rotations(&self) -> FxHashMap<String, [f32; 3]> {
        use super::collection::{collection_field_path, CollectionField};
        let mut out = FxHashMap::default();
        for &field in CollectionField::ALL {
            insert_rotation(
                &mut out,
                collection_field_path(field),
                self.field_ref(field),
            );
        }
        out
    }
}

impl ShopPositions {
    pub fn committed_rotations(&self) -> FxHashMap<String, [f32; 3]> {
        use super::shop::{shop_field_path, ShopField};
        let mut out = FxHashMap::default();
        for &field in ShopField::ALL {
            insert_rotation(&mut out, shop_field_path(field), self.field_ref(field));
        }
        out
    }
}

impl MainMenuExteriorPositions {
    pub fn committed_rotations(&self) -> FxHashMap<String, [f32; 3]> {
        use super::main_menu_exterior::{main_menu_exterior_field_path, MainMenuExteriorField};
        let mut out = FxHashMap::default();
        for &field in MainMenuExteriorField::ALL {
            insert_rotation(
                &mut out,
                main_menu_exterior_field_path(field),
                self.field_ref(field),
            );
        }
        out
    }
}

impl TileSelectPositions {
    pub fn committed_rotations(&self) -> FxHashMap<String, [f32; 3]> {
        use super::tile_select::{tile_select_field_path, TileSelectField};
        let mut out = FxHashMap::default();
        for &field in TileSelectField::ALL {
            insert_rotation(
                &mut out,
                tile_select_field_path(field),
                self.field_ref(field),
            );
        }
        out
    }
}

impl TutorialPositions {
    pub fn committed_rotations(&self) -> FxHashMap<String, [f32; 3]> {
        use super::tutorial::{tutorial_field_path, TutorialField};
        let mut out = FxHashMap::default();
        for &field in TutorialField::ALL {
            insert_rotation(
                &mut out,
                tutorial_field_path(field),
                self.field_ref(field),
            );
        }
        out
    }
}
