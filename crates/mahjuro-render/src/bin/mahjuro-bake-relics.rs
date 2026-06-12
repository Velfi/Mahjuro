//! CPU-only bake: `data/relic_baked/<slug>.rlc` per relic (mask-cut albedo + relief + mesh).
//!
//! On success, refreshes `assets/data/relic_baked/.inputs_stamp` with the same FNV-1a
//! hash that `mahjuro`'s `build.rs` recomputes, so the next `cargo build` won't
//! panic with "relic RLC2 bake is out of date".

use mahjuro_bake_stamp::BakeKind;
use mahjuro_bake_stamp::relic::Relic;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let repo = repo_root()?;
    let assets = std::env::var_os("MAHJURO_ASSETS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| repo.join("assets"));

    // SAFETY: single-threaded bake binary; no concurrent env access.
    unsafe {
        std::env::set_var("MAHJURO_ASSETS", &assets);
    }
    mahjuro_assets::asset_path::init();

    let out_dir = assets.join("data/relic_baked");
    std::fs::create_dir_all(&out_dir)?;

    let defs = mahjuro_core::core::relic::all_relic_defs();
    let mut baked = 0usize;
    let mut skipped = 0usize;
    for d in defs {
        let rel = mahjuro_render::relic_bake::baked_relic_asset_path(d.id);
        let out = assets.join(&rel);
        let Some((msg, _mesh_build)) =
            mahjuro_render::relic_pipeline::decode_relic_assets(d.id, d.name)
        else {
            log::warn!(
                "skip {:?}: no source PNG at {} or {}",
                d.id,
                d.id.render_texture_path(),
                d.id.source_object_path()
            );
            skipped += 1;
            continue;
        };
        let bytes = mahjuro_render::relic_bake::encode_baked_relic(&msg)?;
        std::fs::write(&out, &bytes)?;
        log::info!("baked relic: {} ({} bytes)", out.display(), bytes.len());
        baked += 1;
    }
    log::info!(
        "relic bake finished ({baked} ok, {skipped} skipped, {} total)",
        defs.len()
    );

    if skipped == 0 {
        let stamped = Relic::write_stamp(&repo)?;
        log::info!(
            "refreshed {} ({})",
            stamped.stamp_path.display(),
            stamped.hash
        );
    } else {
        log::warn!(
            "{} skipped relic(s); leaving {} alone so build.rs still flags the gap",
            skipped,
            Relic::STAMP_PATH
        );
    }
    Ok(())
}

/// Repo root with no `..` components. The build script uses `CARGO_MANIFEST_DIR`
/// of `mahjuro` (already canonical); we mirror that by walking the parent chain
/// rather than `join("../..")`, since `Fnv64::write_path_key` hashes the literal
/// path string and any `..` would silently desync from `build.rs`'s digest.
fn repo_root() -> anyhow::Result<std::path::PathBuf> {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("CARGO_MANIFEST_DIR has no grandparent"))
}
