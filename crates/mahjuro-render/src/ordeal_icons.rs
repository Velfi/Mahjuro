//! Boss HUD icon decode (atlas + processed PNG).

use crate::draw_cmd::ImageQuadSource;
use mahjuro_core::core::ordeal_kind::OrdealKind;

pub const ORDEAL_ICON_ATLAS_PNG: &str = "textures/ordeal_icons/atlas.png";

pub fn ordeal_icon_processed_asset(slug: &str) -> String {
    format!("textures/ordeal_icons/processed/ordeal_{slug}.png")
}

pub fn ordeal_icon_processed_asset_static(kind: OrdealKind) -> &'static str {
    match kind {
        OrdealKind::Drought => "textures/ordeal_icons/processed/ordeal_drought.png",
        OrdealKind::Whisper => "textures/ordeal_icons/processed/ordeal_whisper.png",
        OrdealKind::Gate => "textures/ordeal_icons/processed/ordeal_gate.png",
        OrdealKind::Grove => "textures/ordeal_icons/processed/ordeal_grove.png",
        OrdealKind::Coin => "textures/ordeal_icons/processed/ordeal_coin.png",
        OrdealKind::Rot => "textures/ordeal_icons/processed/ordeal_rot.png",
        OrdealKind::Hermit => "textures/ordeal_icons/processed/ordeal_hermit.png",
        OrdealKind::Forest => "textures/ordeal_icons/processed/ordeal_forest.png",
        OrdealKind::Bureaucrat => "textures/ordeal_icons/processed/ordeal_bureaucrat.png",
        OrdealKind::Drunkard => "textures/ordeal_icons/processed/ordeal_drunkard.png",
        OrdealKind::Ash => "textures/ordeal_icons/processed/ordeal_ash.png",
        OrdealKind::Furnace => "textures/ordeal_icons/processed/ordeal_furnace.png",
        OrdealKind::Relic => "textures/ordeal_icons/processed/ordeal_relic.png",
        OrdealKind::Blight => "textures/ordeal_icons/processed/ordeal_blight.png",
        OrdealKind::Hex => "textures/ordeal_icons/processed/ordeal_hex.png",
        OrdealKind::Famine => "textures/ordeal_icons/processed/ordeal_famine.png",
        OrdealKind::Tempest => "textures/ordeal_icons/processed/ordeal_tempest.png",
        OrdealKind::Censor => "textures/ordeal_icons/processed/ordeal_censor.png",
        OrdealKind::Mirror => "textures/ordeal_icons/processed/ordeal_mirror.png",
        OrdealKind::Counterweight => "textures/ordeal_icons/processed/ordeal_counterweight.png",
        OrdealKind::TaxCollector => "textures/ordeal_icons/processed/ordeal_tax_collector.png",
        OrdealKind::Dragon => "textures/ordeal_icons/processed/ordeal_dragon.png",
        OrdealKind::House => "textures/ordeal_icons/processed/ordeal_house.png",
        OrdealKind::DeadAir => "textures/ordeal_icons/processed/ordeal_dead_air.png",
        OrdealKind::StGeorge => "textures/ordeal_icons/processed/ordeal_st_george.png",
    }
}

pub fn ordeal_icon_rgba(kind: OrdealKind) -> Option<(Vec<u8>, u32, u32)> {
    let slug = kind.atlas_slug();
    let path = ordeal_icon_processed_asset(slug);
    if let Ok((rgba, w, h)) = crate::baked_texture::load_rgba_for_cpu(&path) {
        if w > 0 && h > 0 {
            return Some((rgba, w, h));
        }
    }
    crate::temptation_atlas::extract_sprite_rgba(ORDEAL_ICON_ATLAS_PNG, slug)
}

/// Icon source for 2D HUD quads (pick-chamber, etc.).
pub fn ordeal_icon_source(kind: OrdealKind) -> ImageQuadSource {
    ImageQuadSource::Asset {
        path: ordeal_icon_processed_asset_static(kind),
    }
}
