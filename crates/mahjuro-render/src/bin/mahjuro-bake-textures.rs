//! CPU-only bake: generic BTX1 textures under `data/texture_baked/`.

use mahjuro_bake_stamp::texture::Texture;
use mahjuro_bake_stamp::BakeKind;

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

    let manifest = mahjuro_render::static_texture_bakes::static_texture_bake_manifest();
    let mut baked = 0usize;
    let mut missing = 0usize;

    for entry in manifest {
        let Some(file) = mahjuro_assets::asset_path::get(&entry.path) else {
            log::warn!("skip missing static texture: {}", entry.path);
            missing += 1;
            continue;
        };
        let img = image::load_from_memory(&file.data)
            .map_err(|e| anyhow::anyhow!("failed to decode {}: {e}", entry.path))?
            .into_rgba8();
        let (w, h) = img.dimensions();
        let rgba = img.into_raw();
        let payload =
            mahjuro_render::baked_texture::encode_rgba_bc7_mip_chain(&rgba, w, h, entry.color)?;
        let bytes = mahjuro_render::baked_texture::encode_btx(&payload)?;
        let rel = mahjuro_render::baked_texture::baked_texture_asset_path(&entry.path);
        let out = assets.join(&rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out, &bytes)?;
        log::info!("baked texture: {} ({} bytes)", out.display(), bytes.len());
        baked += 1;
    }

    bake_gltf_material_textures(&assets, &mut baked, &mut missing)?;
    bake_talisman_meshes(&assets, &mut baked, &mut missing)?;

    log::info!("static texture bake finished ({baked} baked, {missing} missing)");
    if missing == 0 {
        let stamped = Texture::write_stamp(&repo)?;
        log::info!(
            "refreshed {} ({})",
            stamped.stamp_path.display(),
            stamped.hash
        );
    } else {
        log::warn!(
            "{missing} missing texture source(s); leaving {} alone so build.rs still flags the gap",
            Texture::STAMP_PATH
        );
    }
    Ok(())
}

fn bake_talisman_meshes(
    assets: &std::path::Path,
    baked: &mut usize,
    missing: &mut usize,
) -> anyhow::Result<()> {
    for mask_path in talisman_mask_paths() {
        let Some(file) = mahjuro_assets::asset_path::get(mask_path) else {
            log::warn!("skip missing talisman mesh mask: {mask_path}");
            *missing += 1;
            continue;
        };
        let img = image::load_from_memory(&file.data)
            .map_err(|e| anyhow::anyhow!("failed to decode {mask_path}: {e}"))?
            .into_rgba8();
        let (w, h) = img.dimensions();
        let rgba = img.into_raw();
        let Some(mesh) =
            mahjuro_render::relic_dish::build_talisman_mesh_from_rgba(&rgba, w, h, mask_path)
        else {
            anyhow::bail!("failed to build talisman mesh from {mask_path}");
        };
        let bytes = mahjuro_render::talisman_mesh::encode_baked_talisman_mesh(&mesh)?;
        let rel = mahjuro_render::talisman_mesh::baked_talisman_mesh_asset_path(mask_path);
        let out = assets.join(&rel);
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out, &bytes)?;
        log::info!(
            "baked talisman mesh: {} ({} bytes)",
            out.display(),
            bytes.len()
        );
        *baked += 1;
    }
    Ok(())
}

fn talisman_mask_paths() -> Vec<&'static str> {
    let mut paths = Vec::new();
    paths.extend(
        mahjuro_core::core::talisman::TalismanKind::all()
            .iter()
            .map(|kind| kind.mask_asset_path()),
    );
    paths.extend(
        mahjuro_core::core::memorial_talisman::MemorialTalismanKind::all()
            .iter()
            .map(|kind| kind.mask_asset_path()),
    );
    paths.sort_unstable();
    paths.dedup();
    paths
}

fn bake_gltf_material_textures(
    assets: &std::path::Path,
    baked: &mut usize,
    missing: &mut usize,
) -> anyhow::Result<()> {
    for (asset_path, label, loader) in room_glb_bake_loaders() {
        let Some(file) = mahjuro_assets::asset_path::get(asset_path) else {
            log::warn!("skip missing GLB texture source: {asset_path}");
            *missing += 1;
            continue;
        };
        let cpu = loader(&file.data)?;
        for prim in &cpu.environment_primitives {
            bake_primitive_slot(
                assets,
                label,
                prim.mesh.albedo_btx_source_path.as_deref(),
                prim.mesh.albedo_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::Srgb,
                baked,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.mesh.normal_btx_source_path.as_deref(),
                prim.mesh.normal_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::NormalLinear,
                baked,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.mesh.metallic_roughness_btx_source_path.as_deref(),
                prim.mesh.metallic_roughness_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::Linear,
                baked,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.mesh.emissive_btx_source_path.as_deref(),
                prim.mesh.emissive_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::Srgb,
                baked,
            )?;
        }
    }

    for (asset_path, label) in [
        (
            "3d/tile_bamboo_and_ivory.glb",
            "3d/tile_bamboo_and_ivory.glb",
        ),
        ("3d/tile_plastic.glb", "3d/tile_plastic.glb"),
        ("3d/tile_tortoise_shell.glb", "3d/tile_tortoise_shell.glb"),
        ("3d/coin.glb", "3d/coin.glb"),
    ] {
        let Some(file) = mahjuro_assets::asset_path::get(asset_path) else {
            log::warn!("skip missing GLB texture source: {asset_path}");
            *missing += 1;
            continue;
        };
        let mesh = if asset_path == "3d/coin.glb" {
            mahjuro_render::tile_glb::load_glb_tile_from_node_name_with_label(
                &file.data,
                Some(mahjuro_render::coin_glb::COIN_GLB_NODE),
                Some(label),
            )?
        } else {
            mahjuro_render::tile_glb::load_glb_tile_from_bytes_with_label(&file.data, label)?
        };
        for prim in &mesh.primitives {
            bake_primitive_slot(
                assets,
                label,
                prim.albedo_btx_source_path.as_deref(),
                prim.albedo_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::Srgb,
                baked,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.normal_btx_source_path.as_deref(),
                prim.normal_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::NormalLinear,
                baked,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.metallic_roughness_btx_source_path.as_deref(),
                prim.metallic_roughness_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::Linear,
                baked,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.emissive_btx_source_path.as_deref(),
                prim.emissive_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::Srgb,
                baked,
            )?;
        }
    }
    Ok(())
}

type RoomGlbLoader = fn(&[u8]) -> anyhow::Result<mahjuro_render::room_glb::RoomGlbCpu>;

fn room_glb_bake_loaders() -> &'static [(&'static str, &'static str, RoomGlbLoader)] {
    &[
        (
            "3d/shop.glb",
            "shop.glb",
            mahjuro_render::room_glb::load_shop_glb_from_bytes,
        ),
        (
            "3d/gameplay.glb",
            "gameplay.glb",
            mahjuro_render::gameplay_glb::load_gameplay_glb_from_bytes,
        ),
        (
            "3d/hallway.glb",
            "hallway.glb",
            mahjuro_render::hallway_glb::load_hallway_glb_from_bytes,
        ),
        (
            "3d/staircase.glb",
            "staircase.glb",
            mahjuro_render::staircase_glb::load_staircase_glb_from_bytes,
        ),
        (
            "3d/archive.glb",
            "archive.glb",
            mahjuro_render::archive_glb::load_archive_glb_from_bytes,
        ),
        (
            "3d/main_menu.glb",
            "main_menu.glb",
            mahjuro_render::main_menu_glb::load_main_menu_glb_from_bytes,
        ),
        (
            "3d/shadow_test_room.glb",
            "shadow_test_room.glb",
            mahjuro_render::shadow_test_room_glb::load_shadow_test_room_glb_from_bytes,
        ),
    ]
}

fn bake_primitive_slot(
    assets: &std::path::Path,
    label: &str,
    source_path: Option<&str>,
    rgba: Option<&(Vec<u8>, u32, u32)>,
    color: mahjuro_render::baked_texture::BakedTextureColor,
    baked: &mut usize,
) -> anyhow::Result<()> {
    let (Some(source_path), Some((rgba, w, h))) = (source_path, rgba) else {
        return Ok(());
    };
    let payload = mahjuro_render::baked_texture::encode_rgba_bc7_mip_chain(rgba, *w, *h, color)?;
    let bytes = mahjuro_render::baked_texture::encode_btx(&payload)?;
    let rel = mahjuro_render::baked_texture::baked_texture_asset_path(source_path);
    let out = assets.join(&rel);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, &bytes)?;
    log::info!(
        "baked GLB texture: {} ({label}, {} bytes)",
        out.display(),
        bytes.len()
    );
    *baked += 1;
    Ok(())
}

fn repo_root() -> anyhow::Result<std::path::PathBuf> {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("CARGO_MANIFEST_DIR has no grandparent"))
}
