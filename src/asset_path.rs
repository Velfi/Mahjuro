//! Asset loading: shipped ZIP packs (see `pack_manifest.json`) or loose `assets/` in dev.

pub use crate::asset_sources::{
    get, init, list_tilesets, log_all_assets, prefetch_lazy_packs,
    prefetch_lazy_packs_after_menu_once, AssetFile,
};

/// Tilesets shipped for internal scenes only (title screen, etc.); omitted from
/// Options so players cannot select or "unlock" them as a cosmetic choice.
pub const INTERNAL_ONLY_TILESETS: &[&str] = &[];

#[inline]
pub fn is_internal_only_tileset(name: &str) -> bool {
    INTERNAL_ONLY_TILESETS.iter().any(|&s| s == name)
}

/// Like [`list_tilesets`] but excludes [`INTERNAL_ONLY_TILESETS`].
pub fn list_player_tilesets() -> Vec<String> {
    list_tilesets()
        .into_iter()
        .filter(|n| !is_internal_only_tileset(n))
        .collect()
}
