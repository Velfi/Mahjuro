//! Boss HUD icons — packed atlas under `textures/boss_icons/`.
//!
//! Layout ids match [`crate::core::boss::BossKind::atlas_slug`] and
//! [`crate::core::boss::BossKind::ALL`]. Full-resolution
//! `processed/boss_{slug}.png` (512×512) is preferred for 3D meshes; the atlas
//! is used for 2D [`ImageQuadSource::PackedAtlas`] and as a fallback.

use crate::core::boss::BossKind;
use crate::render::draw_cmd::ImageQuadSource;

pub const BOSS_ICON_ATLAS_PNG: &str = "textures/boss_icons/atlas.png";

/// Per-boss processed icon (post-processed, square, typically 512×512).
pub fn boss_icon_processed_asset(slug: &str) -> String {
    format!("textures/boss_icons/processed/boss_{slug}.png")
}

/// Decode the best available RGBA for a boss icon (processed PNG first, else atlas cell).
pub fn boss_icon_rgba(kind: BossKind) -> Option<(Vec<u8>, u32, u32)> {
    let slug = kind.atlas_slug();
    let path = boss_icon_processed_asset(slug);
    if let Some(asset) = crate::asset_path::get(&path) {
        if let Ok(img) = image::load_from_memory(&asset.data) {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            if w > 0 && h > 0 {
                return Some((rgba.into_raw(), w, h));
            }
        }
    }
    crate::render::skip_tag_atlas::extract_sprite_rgba(BOSS_ICON_ATLAS_PNG, slug)
}

/// Icon source for the given boss (pick-blind, etc.).
pub fn boss_icon_source(kind: BossKind) -> ImageQuadSource {
    ImageQuadSource::PackedAtlas {
        sheet: BOSS_ICON_ATLAS_PNG,
        name: kind.atlas_slug(),
    }
}
