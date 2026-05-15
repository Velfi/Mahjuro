//! Skip-tag HUD icons — packed atlas under `textures/skip_tags/`.

use crate::core::tag::TagKind;
use crate::render::draw_cmd::PromptIconSource;

pub const SKIP_TAG_ATLAS_PNG: &str = "textures/skip_tags/atlas.png";

/// Icon source for the given skip-reward tag.
pub fn skip_tag_icon_source(tag: TagKind) -> PromptIconSource {
    PromptIconSource::PackedAtlas {
        sheet: SKIP_TAG_ATLAS_PNG,
        name: tag.atlas_slug(),
    }
}
