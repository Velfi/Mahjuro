//! Required offline bakes (room GI / shadows, showcase decal atlases).

use crate::room_gi_bake::RoomGiRoom;
use crate::{relic_bake, room_gi_bake, room_shadow_bake, showcase_decal_atlas};

/// Fail fast at renderer init when any shipped offline bake is missing or corrupt.
pub fn require_all_at_startup() -> anyhow::Result<()> {
    for room in RoomGiRoom::ALL {
        room_shadow_bake::require_room_shadow_bake(room)?;
        room_gi_bake::require_room_gi_bake(room)?;
    }
    for tileset in mahjuro_assets::asset_path::list_player_tilesets() {
        let path = showcase_decal_atlas::baked_atlas_asset_path(&tileset);
        anyhow::ensure!(
            showcase_decal_atlas::baked_showcase_decal_atlas_available(&tileset),
            "missing baked showcase decal atlas at {path}; run `cargo build` \
             (needs mahjuro-bake-decal-atlases in target/<profile>/)"
        );
    }
    relic_bake::require_all_relic_bakes()?;
    Ok(())
}
