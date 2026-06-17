//! CPU-only bake: generic BTX1 textures under `data/texture_baked/`.

use mahjuro_bake_stamp::BakeKind;
use mahjuro_bake_stamp::texture::{
    Texture, compute_entry_hash, read_texture_sidecar, texture_sidecar_path, write_texture_sidecar,
};

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
    let mut unchanged = 0usize;
    let mut bootstrapped = 0usize;
    let mut missing = 0usize;
    let global_status = Texture::bake_status(&repo);
    let global_stamp_ok = global_status.stamp_ok && global_status.outputs_ok;

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
        let rel = mahjuro_render::baked_texture::baked_texture_asset_path(&entry.path);
        bake_texture_slot(
            &assets,
            &rel,
            &rgba,
            w,
            h,
            entry.color,
            "static texture",
            global_stamp_ok,
            &mut baked,
            &mut unchanged,
            &mut bootstrapped,
        )?;
    }

    bake_gltf_material_textures(
        &assets,
        global_stamp_ok,
        &mut baked,
        &mut unchanged,
        &mut bootstrapped,
        &mut missing,
    )?;
    bake_talisman_meshes(&assets, &mut baked, &mut missing)?;

    log::info!(
        "static texture bake finished ({baked} baked, {unchanged} unchanged, \
         {bootstrapped} bootstrapped, {missing} missing)"
    );
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
    global_stamp_ok: bool,
    baked: &mut usize,
    unchanged: &mut usize,
    bootstrapped: &mut usize,
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
                global_stamp_ok,
                baked,
                unchanged,
                bootstrapped,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.mesh.normal_btx_source_path.as_deref(),
                prim.mesh.normal_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::NormalLinear,
                global_stamp_ok,
                baked,
                unchanged,
                bootstrapped,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.mesh.metallic_roughness_btx_source_path.as_deref(),
                prim.mesh.metallic_roughness_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::Linear,
                global_stamp_ok,
                baked,
                unchanged,
                bootstrapped,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.mesh.emissive_btx_source_path.as_deref(),
                prim.mesh.emissive_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::Srgb,
                global_stamp_ok,
                baked,
                unchanged,
                bootstrapped,
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
                global_stamp_ok,
                baked,
                unchanged,
                bootstrapped,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.normal_btx_source_path.as_deref(),
                prim.normal_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::NormalLinear,
                global_stamp_ok,
                baked,
                unchanged,
                bootstrapped,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.metallic_roughness_btx_source_path.as_deref(),
                prim.metallic_roughness_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::Linear,
                global_stamp_ok,
                baked,
                unchanged,
                bootstrapped,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.emissive_btx_source_path.as_deref(),
                prim.emissive_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::Srgb,
                global_stamp_ok,
                baked,
                unchanged,
                bootstrapped,
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
    global_stamp_ok: bool,
    baked: &mut usize,
    unchanged: &mut usize,
    bootstrapped: &mut usize,
) -> anyhow::Result<()> {
    let (Some(source_path), Some((rgba, w, h))) = (source_path, rgba) else {
        return Ok(());
    };
    let rel = mahjuro_render::baked_texture::baked_texture_asset_path(source_path);
    bake_texture_slot(
        assets,
        &rel,
        rgba,
        *w,
        *h,
        color,
        label,
        global_stamp_ok,
        baked,
        unchanged,
        bootstrapped,
    )
}

fn bake_texture_slot(
    assets: &std::path::Path,
    rel: &str,
    rgba: &[u8],
    w: u32,
    h: u32,
    color: mahjuro_render::baked_texture::BakedTextureColor,
    label: &str,
    global_stamp_ok: bool,
    baked: &mut usize,
    unchanged: &mut usize,
    bootstrapped: &mut usize,
) -> anyhow::Result<()> {
    let out = assets.join(rel);
    let sidecar_path = texture_sidecar_path(&out);
    let entry_hash = compute_entry_hash(texture_color_tag(color), w, h, rgba);
    let sidecar = read_texture_sidecar(&sidecar_path);
    let out_ok = out.is_file();

    if sidecar.as_deref() == Some(entry_hash.as_str()) && out_ok {
        log::info!("unchanged texture: {}", out.display());
        *unchanged += 1;
        return Ok(());
    }

    if sidecar.is_none() && out_ok && global_stamp_ok {
        write_texture_sidecar(&sidecar_path, &entry_hash)?;
        log::info!("bootstrapped texture sidecar: {}", sidecar_path.display());
        *bootstrapped += 1;
        return Ok(());
    }

    let payload = mahjuro_render::baked_texture::encode_rgba_bc7_mip_chain(rgba, w, h, color)?;
    let bytes = mahjuro_render::baked_texture::encode_btx(&payload)?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, &bytes)?;
    write_texture_sidecar(&sidecar_path, &entry_hash)?;
    log::info!(
        "baked texture: {} ({label}, {} bytes)",
        out.display(),
        bytes.len()
    );
    *baked += 1;
    Ok(())
}

fn texture_color_tag(color: mahjuro_render::baked_texture::BakedTextureColor) -> &'static str {
    match color {
        mahjuro_render::baked_texture::BakedTextureColor::Srgb => "srgb",
        mahjuro_render::baked_texture::BakedTextureColor::Linear => "linear",
        mahjuro_render::baked_texture::BakedTextureColor::NormalLinear => "normal-linear",
    }
}

fn repo_root() -> anyhow::Result<std::path::PathBuf> {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("CARGO_MANIFEST_DIR has no grandparent"))
}
