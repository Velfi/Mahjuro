//! Required offline bakes at renderer init (showcase decal atlases only).
//!
//! Room shadow/GI (`.msh`/`.mgi`) and relic RLC1 bakes are lazy-loaded at runtime;
//! `build.rs` stamp checks enforce committed outputs at compile time.

use crate::showcase_decal_atlas;

/// Fail fast at renderer init when showcase decal atlases are missing.
pub fn require_all_at_startup() -> anyhow::Result<()> {
    for tileset in mahjuro_assets::asset_path::list_player_tilesets() {
        let path = showcase_decal_atlas::baked_atlas_asset_path(&tileset);
        anyhow::ensure!(
            showcase_decal_atlas::baked_showcase_decal_atlas_available(&tileset),
            "missing baked showcase decal atlas at {path}; run `cargo build` \
             (needs mahjuro-bake-decal-atlases in target/<profile>/)"
        );
    }
    Ok(())
}
