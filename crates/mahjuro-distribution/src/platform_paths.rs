//! Sandbox-aware filesystem paths for store and Steam SKUs.

use std::path::PathBuf;

const APP_DIR: &str = "Mahjuro";

/// Whether this binary allows `MAHJURO_ASSETS` loose-tree overrides.
pub fn allows_loose_asset_override() -> bool {
    !cfg!(any(feature = "macos-store", feature = "windows-store"))
}

/// Whether arbitrary `MAHJURO_LOG_FILE` paths are allowed outside the container.
pub fn allows_external_log_file() -> bool {
    allows_loose_asset_override()
}

/// Root for saves, settings, mods, and crash logs.
pub fn data_root() -> PathBuf {
    #[cfg(feature = "windows-store")]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local).join(APP_DIR);
        }
    }
    dirs::config_dir()
        .or_else(dirs::data_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_DIR)
}

/// Crash log path inside the container.
pub fn crash_log_path() -> PathBuf {
    data_root().join("logs").join("crash.log")
}

/// Default export basename for play-stats HTML (store builds use a save panel).
pub fn play_stats_export_basename(profile_index: usize) -> String {
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("play_stats_profile{}_{ts}.html", profile_index + 1)
}

/// Destination for play-stats export on Steam / dev builds (Downloads when available).
pub fn play_stats_export_path(profile_index: usize) -> PathBuf {
    let base = dirs::download_dir()
        .or_else(dirs::document_dir)
        .map(|p| p.join(APP_DIR))
        .unwrap_or_else(data_root);
    let _ = std::fs::create_dir_all(&base);
    base.join(play_stats_export_basename(profile_index))
}

/// Tileset mod install folder (always under [`data_root`]).
pub fn tileset_mods_root() -> PathBuf {
    data_root().join("mods").join("tilesets")
}
