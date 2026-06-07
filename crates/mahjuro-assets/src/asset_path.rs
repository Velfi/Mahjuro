//! Asset loading: ZIP packs (`pack_manifest.json`, baked by `build.rs`) or loose `MAHJURO_ASSETS`.

use std::sync::Arc;

use memmap2::Mmap;

pub use crate::asset_sources::{
    get, get_cached, get_mmap_loose, init, list_tilesets, log_all_assets, mount_pack_once,
    prefetch_gameplay_bulk_pack_once, prefetch_lazy_packs, prefetch_lazy_packs_after_menu_once,
    prefetch_rooms_pack_once, AssetFile,
};

/// Load asset bytes, preferring `<path>.zst` when present (room offline bakes).
pub fn load_asset_bytes(path: &str) -> Option<Vec<u8>> {
    let zst_path = format!("{path}.zst");
    if let Some(file) = get_cached(&zst_path) {
        return zstd::decode_all(file.as_ref()).ok();
    }
    get_cached(path).map(|b| b.to_vec())
}

/// Memory-map a loose-tree asset (no copy). ZIP / pack entries still use [`load_asset_bytes`].
pub fn get_mmap(path: &str) -> Option<Mmap> {
    get_mmap_loose(path)
}

/// Like [`get`] but returns shared bytes on cache hit.
pub fn get_shared(path: &str) -> Option<Arc<[u8]>> {
    get_cached(path)
}

/// Tilesets shipped for internal scenes only (title screen, etc.); omitted from
/// Options so players cannot select or "unlock" them as a cosmetic choice.
pub const INTERNAL_ONLY_TILESETS: &[&str] = &[];

#[inline]
pub fn is_internal_only_tileset(name: &str) -> bool {
    INTERNAL_ONLY_TILESETS.contains(&name)
}

/// Built-in tilesets from mounted assets (packs or `MAHJURO_ASSETS`), excluding
/// [`INTERNAL_ONLY_TILESETS`].
pub fn list_builtin_player_tilesets() -> Vec<String> {
    list_tilesets()
        .into_iter()
        .filter(|n| !is_internal_only_tileset(n))
        .collect()
}

/// Built-in + validated player mod tilesets (`mod:<folder_name>`).
pub fn list_player_tilesets() -> Vec<String> {
    let mut names = list_builtin_player_tilesets();
    for entry in crate::tileset_mod::list_mod_tilesets() {
        names.push(entry.id);
    }
    names
}

pub use crate::tileset_mod::tileset_display_name;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn list_mod_tileset_ids_use_namespace() {
        let base = std::env::temp_dir().join(format!(
            "mahjuro_list_player_tilesets_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&base);
        crate::tileset_mod::set_mod_tilesets_root_for_tests(base.clone());

        let good = base.join("mods/tilesets/cobalt");
        fs::create_dir_all(&good).unwrap();
        fs::write(
            good.join("atlas.toml"),
            "tile_width = 8\ntile_height = 8\ncolumns = 9\nlayout = [\"B1\"]\n",
        )
        .unwrap();
        let img = image::RgbaImage::new(72, 8);
        img.save(good.join("atlas.png")).unwrap();

        let mods: Vec<String> = crate::tileset_mod::list_mod_tilesets()
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert!(mods.iter().any(|n| n == "mod:cobalt"));
        assert_eq!(tileset_display_name("mod:cobalt"), "cobalt (mod)");
    }
}
