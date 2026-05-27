//! CPU-only bake: `data/relic_baked/<slug>.rlc` per relic (mask-cut albedo + relief + mesh).

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let assets = std::env::var_os("MAHJURO_ASSETS")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("CARGO_MANIFEST_DIR").map(|p| {
                std::path::PathBuf::from(p)
                    .join("../..")
                    .join("assets")
            })
        })
        .ok_or_else(|| anyhow::anyhow!("set MAHJURO_ASSETS or run from the repo"))?;

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
    Ok(())
}
