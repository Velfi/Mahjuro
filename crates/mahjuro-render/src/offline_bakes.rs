//! Required offline bakes at renderer init.
//!
//! Showcase decal atlases, room `.mgi`/`.msh`, and relic RLC1 payloads are validated
//! eagerly when committed bakes are required. `build.rs` stamp checks still enforce
//! outputs at compile time; runtime checks catch missing assets in dev skips or bad packs.

use anyhow::Context;
use std::time::Instant;

use crate::room_gi_bake::RoomGiRoom;
use crate::room_shadow_bake;

/// Game runtime must load valid committed offline bakes. Offline bakers skip this so stale
/// placeholders do not block capturing fresh bakes.
pub fn committed_offline_bakes_required() -> bool {
    if cfg!(feature = "bake") {
        return false;
    }
    !mahjuro_bake_stamp::skip_committed_bake_checks()
}

/// Fail fast at renderer init when required offline bakes are missing or invalid.
pub fn require_all_at_startup() -> anyhow::Result<()> {
    if !committed_offline_bakes_required() {
        return Ok(());
    }

    let t_showcase = Instant::now();
    require_showcase_decal_atlases()?;
    crate::startup_profile::record("wgpu.offline_bakes.showcase", t_showcase.elapsed());

    let t_room_gi = Instant::now();
    require_room_gi_bakes()?;
    crate::startup_profile::record("wgpu.offline_bakes.room_gi", t_room_gi.elapsed());

    let t_room_shadow = Instant::now();
    require_room_shadow_bakes()?;
    crate::startup_profile::record("wgpu.offline_bakes.room_shadow", t_room_shadow.elapsed());

    let t_relic = Instant::now();
    require_relic_bakes()?;
    crate::startup_profile::record("wgpu.offline_bakes.relic", t_relic.elapsed());

    Ok(())
}

fn require_showcase_decal_atlases() -> anyhow::Result<()> {
    for tileset in mahjuro_assets::asset_path::list_player_tilesets() {
        let path = crate::showcase_decal_atlas::baked_atlas_asset_path(&tileset);
        anyhow::ensure!(
            crate::showcase_decal_atlas::baked_showcase_decal_atlas_available(&tileset),
            "missing baked showcase decal atlas at {path}; run `cargo build` \
             (needs mahjuro-bake-decal-atlases in target/<profile>/)"
        );
    }
    Ok(())
}

fn require_room_gi_bakes() -> anyhow::Result<()> {
    for room in RoomGiRoom::ALL {
        crate::room_gi_bake::require_room_gi_bake(room)
            .with_context(|| format!("room GI bake {room:?}"))?;
    }
    Ok(())
}

fn require_room_shadow_bakes() -> anyhow::Result<()> {
    for room in room_shadow_bake::runtime_required_room_shadow_bakes() {
        room_shadow_bake::require_effective_room_shadow_bake(room)
            .with_context(|| format!("room shadow bake {room:?}"))?;
    }
    Ok(())
}

fn require_relic_bakes() -> anyhow::Result<()> {
    for def in mahjuro_core::core::relic::all_relic_defs() {
        let path = crate::relic_bake::baked_relic_asset_path(def.id);
        crate::relic_bake::validate_baked_relic(def.id)
            .with_context(|| format!("baked relic at {path}"))?;
    }
    Ok(())
}
