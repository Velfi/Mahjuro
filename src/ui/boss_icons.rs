//! Boss HUD icons — packed atlas under `textures/boss_icons/`.
//!
//! Layout ids match [`crate::core::boss::BossKind::atlas_slug`] and
//! [`crate::core::boss::BossKind::ALL`].

use crate::core::boss::BossKind;
use crate::render::draw_cmd::ImageQuadSource;

pub const BOSS_ICON_ATLAS_PNG: &str = "textures/boss_icons/atlas.png";

/// Icon source for the given boss (pick-blind, etc.).
pub fn boss_icon_source(kind: BossKind) -> ImageQuadSource {
    ImageQuadSource::PackedAtlas {
        sheet: BOSS_ICON_ATLAS_PNG,
        name: kind.atlas_slug(),
    }
}
