//! CPU-only bake: generic BTX1 textures under `data/texture_baked/`.

use std::collections::BTreeMap;

use indicatif::{ProgressBar, ProgressStyle};
use mahjuro_bake_stamp::BakeKind;
use mahjuro_bake_stamp::texture::{
    Texture, compute_entry_hash, read_texture_sidecar, texture_sidecar_path, write_texture_sidecar,
};

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

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
    let mut summary = BakeSummary::default();
    let global_status = Texture::bake_status(&repo);
    let global_stamp_ok = global_status.stamp_ok && global_status.outputs_ok;
    let talisman_masks = talisman_mask_paths();
    let plan = BakePlan::build(&manifest, &talisman_masks)?;
    let progress = texture_bake_progress(plan.total_work());

    for entry in manifest {
        progress.set_message(format!("static {}", entry.path));
        let Some(file) = mahjuro_assets::asset_path::get(&entry.path) else {
            summary.record_missing("static textures");
            progress.inc(1);
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
            &mut summary,
        )?;
        progress.inc(1);
    }

    bake_gltf_material_textures(&assets, global_stamp_ok, &progress, &mut summary)?;
    bake_talisman_meshes(&assets, &talisman_masks, &progress, &mut summary)?;
    progress.finish_and_clear();

    if summary.missing == 0 {
        let stamped = Texture::write_stamp(&repo)?;
        summary.stamp = Some(format!(
            "{} ({})",
            stamped.stamp_path.display(),
            stamped.hash
        ));
    } else {
        log::warn!(
            "{} missing texture source(s); leaving {} alone so build.rs still flags the gap",
            summary.missing,
            Texture::STAMP_PATH
        );
    }
    summary.print_tree();
    Ok(())
}

fn bake_talisman_meshes(
    assets: &std::path::Path,
    mask_paths: &[&str],
    progress: &ProgressBar,
    summary: &mut BakeSummary,
) -> anyhow::Result<()> {
    for &mask_path in mask_paths {
        progress.set_message(format!("mesh {mask_path}"));
        let Some(file) = mahjuro_assets::asset_path::get(mask_path) else {
            summary.record_missing("talisman meshes");
            progress.inc(1);
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
        summary.record_baked("talisman meshes", bytes.len());
        progress.inc(1);
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
    progress: &ProgressBar,
    summary: &mut BakeSummary,
) -> anyhow::Result<()> {
    for &(asset_path, label, loader) in room_glb_bake_loaders() {
        progress.set_message(format!("load {asset_path}"));
        let Some(file) = mahjuro_assets::asset_path::get(asset_path) else {
            summary.record_missing(label);
            progress.inc(1);
            continue;
        };
        let cpu = loader(&file.data)?;
        progress.inc(1);
        for prim in &cpu.environment_primitives {
            bake_primitive_slot(
                assets,
                label,
                prim.mesh.albedo_btx_source_path.as_deref(),
                prim.mesh.albedo_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::Srgb,
                global_stamp_ok,
                progress,
                summary,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.mesh.normal_btx_source_path.as_deref(),
                prim.mesh.normal_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::NormalLinear,
                global_stamp_ok,
                progress,
                summary,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.mesh.metallic_roughness_btx_source_path.as_deref(),
                prim.mesh.metallic_roughness_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::Linear,
                global_stamp_ok,
                progress,
                summary,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.mesh.emissive_btx_source_path.as_deref(),
                prim.mesh.emissive_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::Srgb,
                global_stamp_ok,
                progress,
                summary,
            )?;
        }
    }

    for &(asset_path, label) in tile_glb_bake_sources() {
        progress.set_message(format!("load {asset_path}"));
        let Some(file) = mahjuro_assets::asset_path::get(asset_path) else {
            summary.record_missing(label);
            progress.inc(1);
            continue;
        };
        let mesh = if asset_path == "3d/coin.glb" {
            mahjuro_render::tile_glb::load_glb_tile_from_node_name_with_label(
                &file.data,
                Some(mahjuro_render::coin_glb::COIN_GLB_NODE),
                Some(label),
            )?
        } else if matches!(
            asset_path,
            mahjuro_render::tally_stick_mesh::PLAY_TALLY_STICK_GLB_PATH
                | mahjuro_render::tally_stick_mesh::DISCARD_TALLY_STICK_GLB_PATH
        ) {
            mahjuro_render::tally_stick_mesh::load_tally_stick_glb_tile(asset_path)
        } else {
            mahjuro_render::tile_glb::load_glb_tile_from_bytes_with_label(&file.data, label)?
        };
        progress.inc(1);
        for prim in &mesh.primitives {
            bake_primitive_slot(
                assets,
                label,
                prim.albedo_btx_source_path.as_deref(),
                prim.albedo_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::Srgb,
                global_stamp_ok,
                progress,
                summary,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.normal_btx_source_path.as_deref(),
                prim.normal_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::NormalLinear,
                global_stamp_ok,
                progress,
                summary,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.metallic_roughness_btx_source_path.as_deref(),
                prim.metallic_roughness_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::Linear,
                global_stamp_ok,
                progress,
                summary,
            )?;
            bake_primitive_slot(
                assets,
                label,
                prim.emissive_btx_source_path.as_deref(),
                prim.emissive_rgba.as_deref(),
                mahjuro_render::baked_texture::BakedTextureColor::Srgb,
                global_stamp_ok,
                progress,
                summary,
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

fn tile_glb_bake_sources() -> &'static [(&'static str, &'static str)] {
    &[
        (
            "3d/tile_bamboo_and_ivory.glb",
            "3d/tile_bamboo_and_ivory.glb",
        ),
        ("3d/tile_plastic.glb", "3d/tile_plastic.glb"),
        ("3d/tile_tortoise_shell.glb", "3d/tile_tortoise_shell.glb"),
        ("3d/coin.glb", "3d/coin.glb"),
        (
            mahjuro_render::tally_stick_mesh::PLAY_TALLY_STICK_GLB_PATH,
            mahjuro_render::tally_stick_mesh::PLAY_TALLY_STICK_GLB_PATH,
        ),
        (
            mahjuro_render::tally_stick_mesh::DISCARD_TALLY_STICK_GLB_PATH,
            mahjuro_render::tally_stick_mesh::DISCARD_TALLY_STICK_GLB_PATH,
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
    progress: &ProgressBar,
    summary: &mut BakeSummary,
) -> anyhow::Result<()> {
    let (Some(source_path), Some((rgba, w, h))) = (source_path, rgba) else {
        return Ok(());
    };
    let rel = mahjuro_render::baked_texture::baked_texture_asset_path(source_path);
    progress.set_message(format!("{label} {source_path}"));
    bake_texture_slot(
        assets,
        &rel,
        rgba,
        *w,
        *h,
        color,
        label,
        global_stamp_ok,
        summary,
    )?;
    progress.inc(1);
    Ok(())
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
    summary: &mut BakeSummary,
) -> anyhow::Result<()> {
    let out = assets.join(rel);
    let sidecar_path = texture_sidecar_path(&out);
    let entry_hash = compute_entry_hash(&texture_color_tag(color), w, h, rgba);
    let sidecar = read_texture_sidecar(&sidecar_path);
    let out_ok = out.is_file();

    if sidecar.as_deref() == Some(entry_hash.as_str()) && out_ok {
        summary.record_unchanged(label);
        return Ok(());
    }

    if sidecar.is_none() && out_ok && global_stamp_ok {
        write_texture_sidecar(&sidecar_path, &entry_hash)?;
        summary.record_bootstrapped(label);
        return Ok(());
    }

    let payload = mahjuro_render::baked_texture::encode_rgba_bc7_mip_chain(rgba, w, h, color)?;
    let bytes = mahjuro_render::baked_texture::encode_btx(&payload)?;
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&out, &bytes)?;
    write_texture_sidecar(&sidecar_path, &entry_hash)?;
    summary.record_baked(label, bytes.len());
    Ok(())
}

struct BakePlan {
    static_textures: usize,
    talisman_meshes: usize,
    gltf_loads: usize,
    gltf_textures: usize,
}

impl BakePlan {
    fn build(
        manifest: &[mahjuro_render::static_texture_bakes::StaticTextureBake],
        talisman_masks: &[&str],
    ) -> anyhow::Result<Self> {
        Ok(Self {
            static_textures: manifest.len(),
            talisman_meshes: talisman_masks.len(),
            gltf_loads: room_glb_bake_loaders().len() + tile_glb_bake_sources().len(),
            gltf_textures: count_gltf_texture_slots()?,
        })
    }

    fn total_work(&self) -> usize {
        self.static_textures + self.talisman_meshes + self.gltf_loads + self.gltf_textures
    }
}

fn count_gltf_texture_slots() -> anyhow::Result<usize> {
    let mut slots = 0usize;

    for &(asset_path, _, loader) in room_glb_bake_loaders() {
        let Some(file) = mahjuro_assets::asset_path::get(asset_path) else {
            continue;
        };
        let cpu = loader(&file.data)?;
        for prim in &cpu.environment_primitives {
            slots += count_primitive_slot(
                prim.mesh.albedo_btx_source_path.as_deref(),
                prim.mesh.albedo_rgba.as_deref(),
            );
            slots += count_primitive_slot(
                prim.mesh.normal_btx_source_path.as_deref(),
                prim.mesh.normal_rgba.as_deref(),
            );
            slots += count_primitive_slot(
                prim.mesh.metallic_roughness_btx_source_path.as_deref(),
                prim.mesh.metallic_roughness_rgba.as_deref(),
            );
            slots += count_primitive_slot(
                prim.mesh.emissive_btx_source_path.as_deref(),
                prim.mesh.emissive_rgba.as_deref(),
            );
        }
    }

    for &(asset_path, label) in tile_glb_bake_sources() {
        let Some(file) = mahjuro_assets::asset_path::get(asset_path) else {
            continue;
        };
        let mesh = if asset_path == "3d/coin.glb" {
            mahjuro_render::tile_glb::load_glb_tile_from_node_name_with_label(
                &file.data,
                Some(mahjuro_render::coin_glb::COIN_GLB_NODE),
                Some(label),
            )
        } else if matches!(
            asset_path,
            mahjuro_render::tally_stick_mesh::PLAY_TALLY_STICK_GLB_PATH
                | mahjuro_render::tally_stick_mesh::DISCARD_TALLY_STICK_GLB_PATH
        ) {
            Ok(mahjuro_render::tally_stick_mesh::load_tally_stick_glb_tile(
                asset_path,
            ))
        } else {
            mahjuro_render::tile_glb::load_glb_tile_from_bytes_with_label(&file.data, label)
        };
        let mesh = mesh?;
        for prim in &mesh.primitives {
            slots += count_primitive_slot(
                prim.albedo_btx_source_path.as_deref(),
                prim.albedo_rgba.as_deref(),
            );
            slots += count_primitive_slot(
                prim.normal_btx_source_path.as_deref(),
                prim.normal_rgba.as_deref(),
            );
            slots += count_primitive_slot(
                prim.metallic_roughness_btx_source_path.as_deref(),
                prim.metallic_roughness_rgba.as_deref(),
            );
            slots += count_primitive_slot(
                prim.emissive_btx_source_path.as_deref(),
                prim.emissive_rgba.as_deref(),
            );
        }
    }

    Ok(slots)
}

fn count_primitive_slot(source_path: Option<&str>, rgba: Option<&(Vec<u8>, u32, u32)>) -> usize {
    usize::from(source_path.is_some() && rgba.is_some())
}

#[derive(Default)]
struct BakeSummary {
    groups: BTreeMap<String, GroupSummary>,
    baked: usize,
    unchanged: usize,
    bootstrapped: usize,
    missing: usize,
    bytes_written: usize,
    stamp: Option<String>,
}

#[derive(Default)]
struct GroupSummary {
    baked: usize,
    unchanged: usize,
    bootstrapped: usize,
    missing: usize,
    bytes_written: usize,
}

impl BakeSummary {
    fn group_mut(&mut self, label: &str) -> &mut GroupSummary {
        self.groups.entry(label.to_string()).or_default()
    }

    fn record_baked(&mut self, label: &str, bytes: usize) {
        self.baked += 1;
        self.bytes_written += bytes;
        let group = self.group_mut(label);
        group.baked += 1;
        group.bytes_written += bytes;
    }

    fn record_unchanged(&mut self, label: &str) {
        self.unchanged += 1;
        self.group_mut(label).unchanged += 1;
    }

    fn record_bootstrapped(&mut self, label: &str) {
        self.bootstrapped += 1;
        self.group_mut(label).bootstrapped += 1;
    }

    fn record_missing(&mut self, label: &str) {
        self.missing += 1;
        self.group_mut(label).missing += 1;
    }

    fn print_tree(&self) {
        println!("texture bake summary");
        println!(
            "├─ total: {} baked, {} unchanged, {} bootstrapped, {} missing",
            self.baked, self.unchanged, self.bootstrapped, self.missing
        );
        println!("├─ written: {}", format_bytes(self.bytes_written));
        match &self.stamp {
            Some(stamp) => println!("├─ stamp: {stamp}"),
            None => println!("├─ stamp: unchanged due to missing inputs"),
        }
        println!("└─ outputs");
        let group_count = self.groups.len();
        for (index, (label, group)) in self.groups.iter().enumerate() {
            let branch = if index + 1 == group_count {
                "   └─"
            } else {
                "   ├─"
            };
            println!(
                "{branch} {label}: {} baked, {} unchanged, {} bootstrapped, {} missing, {} written",
                group.baked,
                group.unchanged,
                group.bootstrapped,
                group.missing,
                format_bytes(group.bytes_written)
            );
        }
    }
}

fn texture_bake_progress(total: usize) -> ProgressBar {
    let progress = ProgressBar::new(total as u64);
    progress.set_style(
        ProgressStyle::with_template(
            "{spinner:.green} {msg:.48} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len}",
        )
        .expect("valid progress template")
        .progress_chars("=>-"),
    );
    progress
}

fn format_bytes(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= MIB {
        format!("{:.1} MiB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes / KIB)
    } else {
        format!("{bytes:.0} B")
    }
}

fn texture_color_tag(color: mahjuro_render::baked_texture::BakedTextureColor) -> String {
    let color = match color {
        mahjuro_render::baked_texture::BakedTextureColor::Srgb => "srgb",
        mahjuro_render::baked_texture::BakedTextureColor::Linear => "linear",
        mahjuro_render::baked_texture::BakedTextureColor::NormalLinear => "normal-linear",
    };
    format!("btx:{color}")
}

fn repo_root() -> anyhow::Result<std::path::PathBuf> {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .ok_or_else(|| anyhow::anyhow!("CARGO_MANIFEST_DIR has no grandparent"))
}
