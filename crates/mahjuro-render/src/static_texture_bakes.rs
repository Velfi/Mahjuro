//! Manifest for shipped static sampled-art textures that get BTX1 bakes.

use std::collections::BTreeMap;

use crate::baked_texture::BakedTextureColor;

#[derive(Clone, Debug)]
pub struct StaticTextureBake {
    pub path: String,
    pub color: BakedTextureColor,
}

impl StaticTextureBake {
    fn new(path: impl Into<String>, color: BakedTextureColor) -> Self {
        Self {
            path: path.into(),
            color,
        }
    }
}

pub fn static_texture_bake_manifest() -> Vec<StaticTextureBake> {
    let mut out = BTreeMap::<String, BakedTextureColor>::new();

    add_texture_tree(&mut out);

    add(
        &mut out,
        "textures/boot_loading_msdf.png",
        BakedTextureColor::Linear,
    );
    add(
        &mut out,
        "textures/loading/zelda_built_this.png",
        BakedTextureColor::Srgb,
    );
    add(
        &mut out,
        "textures/moon_albedo.png",
        BakedTextureColor::Linear,
    );
    add(
        &mut out,
        "textures/mirror_heightmap.png",
        BakedTextureColor::Linear,
    );
    add(
        &mut out,
        "textures/coin_heightmap.png",
        BakedTextureColor::Linear,
    );
    add(
        &mut out,
        "textures/arrow_right.png",
        BakedTextureColor::Srgb,
    );
    add(
        &mut out,
        "textures/ordeal_icons/atlas.png",
        BakedTextureColor::Srgb,
    );

    for i in 0..=5 {
        add(
            &mut out,
            format!("textures/depth_well/depth_well_{i}.png"),
            BakedTextureColor::Srgb,
        );
    }

    for tileset in mahjuro_assets::asset_path::list_builtin_player_tilesets() {
        add(
            &mut out,
            crate::showcase_decal_atlas::baked_atlas_asset_path(&tileset),
            BakedTextureColor::Srgb,
        );
    }

    for &kind in mahjuro_core::core::tile_pack::TilePackKind::all() {
        add(
            &mut out,
            format!("textures/tile_packs/{}", kind.asset_filename()),
            BakedTextureColor::Srgb,
        );
    }

    for &z in mahjuro_core::core::zodiac::ZodiacKind::all() {
        let slug = z.slug();
        add(
            &mut out,
            format!("textures/zodiacs/zodiac_{slug}.png"),
            BakedTextureColor::Srgb,
        );
        add(
            &mut out,
            format!("textures/zodiacs/zodiac_{slug}_material.png"),
            BakedTextureColor::Linear,
        );
    }

    for &(path, _) in mahjuro_core::core::talisman::TalismanKind::heightmap_paths() {
        add(&mut out, path, BakedTextureColor::Linear);
    }
    for &(path, _) in mahjuro_core::core::talisman::TalismanKind::mask_paths() {
        add(&mut out, path, BakedTextureColor::Linear);
    }
    for &(path, _) in mahjuro_core::core::memorial_talisman::MemorialTalismanKind::heightmap_paths()
    {
        add(&mut out, path, BakedTextureColor::Linear);
    }
    for &(path, _) in mahjuro_core::core::memorial_talisman::MemorialTalismanKind::mask_paths() {
        add(&mut out, path, BakedTextureColor::Linear);
    }

    out.into_iter()
        .map(|(path, color)| StaticTextureBake::new(path, color))
        .collect()
}

fn add(
    out: &mut BTreeMap<String, BakedTextureColor>,
    path: impl Into<String>,
    color: BakedTextureColor,
) {
    out.insert(path.into(), color);
}

fn add_texture_tree(out: &mut BTreeMap<String, BakedTextureColor>) {
    let root = std::env::var_os("MAHJURO_ASSETS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("assets"));
    let textures = root.join("textures");
    let Ok(paths) = collect_image_paths(&textures) else {
        return;
    };
    for path in paths {
        let Ok(rel) = path.strip_prefix(&root) else {
            continue;
        };
        let rel = rel.to_string_lossy().replace('\\', "/");
        if !is_runtime_texture_asset(&rel) {
            continue;
        }
        let color = classify_texture_color(&rel);
        add(out, rel, color);
    }
}

fn collect_image_paths(root: &std::path::Path) -> std::io::Result<Vec<std::path::PathBuf>> {
    let mut out = Vec::new();
    collect_image_paths_inner(root, &mut out)?;
    Ok(out)
}

fn collect_image_paths_inner(
    dir: &std::path::Path,
    out: &mut Vec<std::path::PathBuf>,
) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_image_paths_inner(&path, out)?;
        } else if is_image_file(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn is_image_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            ext.eq_ignore_ascii_case("png")
                || ext.eq_ignore_ascii_case("jpg")
                || ext.eq_ignore_ascii_case("jpeg")
        })
        .unwrap_or(false)
}

fn is_runtime_texture_asset(path: &str) -> bool {
    path.starts_with("textures/")
        && !path.starts_with("textures/relics/")
        && !path.starts_with("textures/kenney_input-prompts/")
        && !path.starts_with("textures/temptations/")
        && path != "textures/main_menu_logo.png"
        && !path.contains("/source/")
        && !path.ends_with("_raw.png")
}

fn classify_texture_color(path: &str) -> BakedTextureColor {
    let lower = path.to_ascii_lowercase();
    if lower.contains("normal") {
        BakedTextureColor::NormalLinear
    } else if lower.contains("height")
        || lower.contains("heightmap")
        || lower.contains("mask")
        || lower.contains("material")
        || lower.contains("roughness")
        || lower.contains("metallic")
        || lower.contains("specular")
        || lower.contains("msdf")
    {
        BakedTextureColor::Linear
    } else {
        BakedTextureColor::Srgb
    }
}
