//! Temptation HUD icons — packed atlas under `textures/temptations/`.

use crate::core::tag::TagKind;
use crate::render::draw_cmd::ImageQuadSource;

pub const TEMPTATION_ATLAS_PNG: &str = "textures/temptations/atlas.png";

/// Icon source for the given temptation.
pub fn temptation_icon_source(tag: TagKind) -> ImageQuadSource {
    ImageQuadSource::PackedAtlas {
        sheet: TEMPTATION_ATLAS_PNG,
        name: tag.atlas_slug(),
    }
}
