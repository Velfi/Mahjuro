//! Open files and folders in the platform shell (Finder, Explorer, …).

use std::path::Path;

/// Reveal `path` in the system file manager.
pub fn reveal_in_file_manager(path: &Path) -> std::io::Result<()> {
    mahjuro_distribution::PlatformShell::reveal_in_file_manager(path)
}

/// Ensure the tileset mod install folder exists and open it in the file manager.
pub fn open_tileset_mods_folder() -> Result<std::path::PathBuf, String> {
    mahjuro_assets::tileset_mod::ensure_mod_tilesets_scaffold();
    let path = mahjuro_assets::tileset_mod::mod_tilesets_root();
    reveal_in_file_manager(&path).map_err(|e| format!("{e}"))?;
    Ok(path)
}
