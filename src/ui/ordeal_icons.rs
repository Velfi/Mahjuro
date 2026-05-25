//! Boss HUD icons — packed atlas under `textures/ordeal_icons/`.
//!
//! Layout ids match [`crate::core::ordeal::OrdealKind::atlas_slug`] and
//! [`crate::core::ordeal::OrdealKind::ALL`]. Full-resolution
//! `processed/ordeal_{slug}.png` (512×512) is preferred for 3D meshes; the atlas
//! is used for 2D [`ImageQuadSource::PackedAtlas`] and as a fallback.

use crate::core::ordeal::OrdealKind;
use crate::render::draw_cmd::ImageQuadSource;

pub const ORDEAL_ICON_ATLAS_PNG: &str = "textures/ordeal_icons/atlas.png";

/// Per-ordeal processed icon (post-processed, square, typically 512×512).
pub fn ordeal_icon_processed_asset(slug: &str) -> String {
    format!("textures/ordeal_icons/processed/ordeal_{slug}.png")
}

/// Decode the best available RGBA for a ordeal icon (processed PNG first, else atlas cell).
pub fn ordeal_icon_rgba(kind: OrdealKind) -> Option<(Vec<u8>, u32, u32)> {
    let slug = kind.atlas_slug();
    let path = ordeal_icon_processed_asset(slug);
    if let Some(asset) = crate::asset_path::get(&path)
        && let Ok(img) = image::load_from_memory(&asset.data)
    {
        let rgba = img.to_rgba8();
        let (w, h) = rgba.dimensions();
        if w > 0 && h > 0 {
            return Some((rgba.into_raw(), w, h));
        }
    }
    crate::render::skip_tag_atlas::extract_sprite_rgba(ORDEAL_ICON_ATLAS_PNG, slug)
}

/// Icon source for the given ordeal (pick-chamber, etc.).
pub fn ordeal_icon_source(kind: OrdealKind) -> ImageQuadSource {
    ImageQuadSource::PackedAtlas {
        sheet: ORDEAL_ICON_ATLAS_PNG,
        name: kind.atlas_slug(),
    }
}
