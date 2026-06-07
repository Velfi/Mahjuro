//! Multi-store distribution: achievements, stats, and sandbox I/O.

#![deny(unused_imports)]

mod achievement;
mod backend;
mod disabled;
#[cfg(feature = "macos-store")]
mod game_center;
mod platform_paths;
mod platform_shell;
mod stat;
#[cfg(feature = "steam")]
pub mod steam;
#[cfg(feature = "windows-store")]
mod xbox_live;

pub use achievement::Achievement;
pub use backend::{DistributionBackend, DistributionClient, DistributionConfig};
pub use stat::{ProfileStat, profile_stat_snapshot};

/// Sandbox-aware filesystem paths.
pub struct PlatformPaths;
impl PlatformPaths {
    pub fn allows_loose_asset_override() -> bool {
        platform_paths::allows_loose_asset_override()
    }
    pub fn allows_external_log_file() -> bool {
        platform_paths::allows_external_log_file()
    }
    pub fn data_root() -> std::path::PathBuf {
        platform_paths::data_root()
    }
    pub fn crash_log_path() -> std::path::PathBuf {
        platform_paths::crash_log_path()
    }
    pub fn play_stats_export_path(profile_index: usize) -> std::path::PathBuf {
        platform_paths::play_stats_export_path(profile_index)
    }
    pub fn tileset_mods_root() -> std::path::PathBuf {
        platform_paths::tileset_mods_root()
    }
}

/// Store-safe shell integration.
pub struct PlatformShell;
impl PlatformShell {
    pub fn reveal_in_file_manager(path: &std::path::Path) -> std::io::Result<()> {
        platform_shell::reveal_in_file_manager(path)
    }
    pub fn resolve_play_stats_export_path(profile_index: usize) -> Option<std::path::PathBuf> {
        platform_shell::resolve_play_stats_export_path(profile_index)
    }
}

#[cfg(all(feature = "steam", feature = "macos-store"))]
compile_error!("enable only one of: dist-steam, dist-mas, dist-msstore");

#[cfg(all(feature = "steam", feature = "windows-store"))]
compile_error!("enable only one of: dist-steam, dist-mas, dist-msstore");

#[cfg(all(feature = "macos-store", feature = "windows-store"))]
compile_error!("enable only one of: dist-steam, dist-mas, dist-msstore");

/// Initialize the distribution backend for this SKU.
pub fn init(config: DistributionConfig) -> DistributionClient {
    if config.platform_services_disabled {
        return DistributionClient::Disabled(disabled::DisabledBackend);
    }

    #[cfg(feature = "steam")]
    {
        if !steam::steamworks_dll_ready() {
            log::warn!(
                "steam_api redistributable was not found next to this executable (or failed to load); \
                 achievements/stats will be disabled this session"
            );
            return DistributionClient::Disabled(disabled::DisabledBackend);
        }
        return DistributionClient::Steam(steam::SteamBackend::init());
    }

    #[cfg(feature = "macos-store")]
    {
        return DistributionClient::GameCenter(game_center::GameCenterBackend::init());
    }

    #[cfg(feature = "windows-store")]
    {
        return DistributionClient::Xbox(xbox_live::XboxBackend::init());
    }

    #[cfg(not(any(feature = "steam", feature = "macos-store", feature = "windows-store")))]
    {
        log::warn!("no distribution feature enabled; platform services disabled");
        DistributionClient::Disabled(disabled::DisabledBackend)
    }
}
