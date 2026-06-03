//! CPU-only bake: `textures/tile_sets/<name>/showcase_decal_atlas.png` per player tileset.
//!
//! On success, refreshes `assets/textures/tile_sets/.decal_bake_stamp` with the same
//! FNV-1a hash that `mahjuro`'s `build.rs` recomputes.

use mahjuro_bake_stamp::BakeKind;
use mahjuro_bake_stamp::showcase_decal::ShowcaseDecal;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let assets = std::env::var_os("MAHJURO_ASSETS")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("CARGO_MANIFEST_DIR")
                .map(|p| std::path::PathBuf::from(p).join("../..").join("assets"))
        })
        .ok_or_else(|| anyhow::anyhow!("set MAHJURO_ASSETS or run from the repo"))?;

    // SAFETY: single-threaded bake binary; no concurrent env access.
    unsafe {
        std::env::set_var("MAHJURO_ASSETS", &assets);
    }
    mahjuro_assets::asset_path::init();

    let ui_font = mahjuro_render::decal::load_ui_font().cloned();
    let emoji_font = mahjuro_render::decal::load_noto_emoji_font();

    let (atlas_w, atlas_h) = mahjuro_render::showcase_decal_atlas::atlas_dimensions();
    let mut baked = 0usize;
    for tileset in mahjuro_assets::asset_path::list_tilesets() {
        let rel = mahjuro_render::showcase_decal_atlas::baked_atlas_asset_path(&tileset);
        let out = assets.join(&rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let rgba = mahjuro_render::showcase_decal_atlas::rasterize_showcase_decal_atlas_rgba(
            ui_font.as_ref(),
            emoji_font.as_ref(),
            Some(&tileset),
        );
        let img = image::RgbaImage::from_raw(atlas_w, atlas_h, rgba)
            .ok_or_else(|| anyhow::anyhow!("atlas buffer size mismatch for {tileset}"))?;
        img.save(&out)?;
        log::info!("baked showcase decal atlas: {}", out.display());
        baked += 1;
    }
    log::info!("showcase decal atlas bake finished ({baked} tilesets)");

    let repo = repo_root()?;
    let stamped = ShowcaseDecal::write_stamp(&repo)?;
    log::info!(
        "refreshed {} ({})",
        stamped.stamp_path.display(),
        stamped.hash
    );
    Ok(())
}

/// Repo root with no `..` components — must match `build.rs`'s digest paths.
fn repo_root() -> anyhow::Result<std::path::PathBuf> {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("CARGO_MANIFEST_DIR has no grandparent"))
}
