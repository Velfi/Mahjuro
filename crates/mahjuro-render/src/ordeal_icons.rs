//! Boss HUD icon decode (atlas + processed PNG).

use crate::draw_cmd::ImageQuadSource;
use mahjuro_assets::asset_path;
use mahjuro_core::core::ordeal_kind::OrdealKind;

pub const ORDEAL_ICON_ATLAS_PNG: &str = "textures/ordeal_icons/atlas.png";

pub fn ordeal_icon_processed_asset(slug: &str) -> String {
    format!("textures/ordeal_icons/processed/ordeal_{slug}.png")
}

pub fn ordeal_icon_rgba(kind: OrdealKind) -> Option<(Vec<u8>, u32, u32)> {
    let slug = kind.atlas_slug();
    let path = ordeal_icon_processed_asset(slug);
    if let Some(asset) = asset_path::get(&path)
        && let Ok(img) = image::load_from_memory(&asset.data)
    {
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        if w > 0 && h > 0 {
            return Some((rgba.into_raw(), w, h));
        }
    }
    crate::temptation_atlas::extract_sprite_rgba(ORDEAL_ICON_ATLAS_PNG, slug)
}

/// Icon source for 2D HUD quads (pick-chamber, etc.).
pub fn ordeal_icon_source(kind: OrdealKind) -> ImageQuadSource {
    ImageQuadSource::PackedAtlas {
        sheet: ORDEAL_ICON_ATLAS_PNG,
        name: kind.atlas_slug(),
    }
}
