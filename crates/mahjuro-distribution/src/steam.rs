//! Steamworks backend.

pub mod workshop;
pub mod workshop_publish;

use std::sync::Arc;

use mahjuro_core::core::progression::PlayerProgress;
use steamworks::{AppId, Client};

use crate::achievement::Achievement;
use crate::backend::DistributionBackend;
use crate::stat::profile_stat_snapshot;

/// Mahjuro's Steam App ID (configured in Steamworks partner backend).
pub const MAHJURO_APP_ID: u32 = 4636490;

pub enum SteamBackend {
    Connected {
        client: Arc<Client>,
        workshop: workshop::TilesetWorkshop,
    },
    Disabled,
}

impl SteamBackend {
    pub fn init() -> Self {
        match Client::init_app(AppId(MAHJURO_APP_ID)) {
            Ok(client) => {
                let name = client.friends().name();
                let steam_id = client.user().steam_id();
                log::info!(
                    "Steam connected: user='{}' steam_id={} app_id={}",
                    name,
                    steam_id.raw(),
                    MAHJURO_APP_ID,
                );
                let client = Arc::new(client);
                let workshop = workshop::TilesetWorkshop::new(client.clone());
                Self::Connected { client, workshop }
            }
            Err(err) => {
                log::warn!(
                    "Steam init failed ({err:?}); achievements/stats/overlay will be disabled this session",
                );
                Self::Disabled
            }
        }
    }

    pub fn disabled() -> Self {
        Self::Disabled
    }

    pub fn open_tileset_workshop(&self) {
        workshop::open_tileset_workshop_overlay();
    }
}

impl DistributionBackend for SteamBackend {
    fn tick(&self) {
        if let Self::Connected { client, workshop } = self {
            client.run_callbacks();
            workshop.tick();
        }
    }

    fn unlock_achievement(&self, ach: Achievement) {
        let Self::Connected { client, .. } = self else {
            return;
        };
        let api_name = ach.steam_api_name();
        let stats = client.user_stats();
        let helper = stats.achievement(api_name);
        match helper.get() {
            Ok(true) => return,
            Ok(false) => {}
            Err(()) => {
                log::debug!(
                    "stats not yet loaded; skipping unlock for '{api_name}' (will retry on next trigger)",
                );
                return;
            }
        }
        if let Err(()) = helper.set() {
            log::warn!("failed to set achievement '{api_name}'");
            return;
        }
        if let Err(()) = stats.store_stats() {
            log::warn!("set_achievement('{api_name}') ok, but store_stats failed");
            return;
        }
        log::info!("unlocked Steam achievement: {api_name}");
    }

    fn sync_profile_stats(&self, progress: &PlayerProgress) {
        let Self::Connected { client, .. } = self else {
            return;
        };
        let stats = client.user_stats();
        let snapshot = profile_stat_snapshot(progress);
        let mut any_ok = false;
        for (st, value) in snapshot {
            let name = st.steam_api_name();
            match stats.set_stat_i32(name, value) {
                Ok(()) => any_ok = true,
                Err(()) => {
                    log::debug!(
                        "Steam stat '{name}' not set to {value} (stats not loaded yet or unknown name)",
                    );
                }
            }
        }
        if !any_ok {
            return;
        }
        if let Err(()) = stats.store_stats() {
            log::warn!("Steam profile stat sync: store_stats failed");
        } else {
            log::debug!("Steam profile stats synced");
        }
    }

    fn is_overlay_available(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }
}

/// Whether `steam_api64.dll` can be loaded from the executable directory.
#[cfg(windows)]
pub fn steamworks_dll_ready() -> bool {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::PathBuf;

    type HModule = *mut std::ffi::c_void;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleFileNameW(h_module: HModule, lp_filename: *mut u16, n_size: u32) -> u32;
        fn LoadLibraryW(lp_lib_file_name: *const u16) -> HModule;
    }

    const CAP: usize = 512;
    let mut buf = vec![0u16; CAP];
    let n = unsafe { GetModuleFileNameW(std::ptr::null_mut(), buf.as_mut_ptr(), CAP as u32) };
    if n == 0 || (n as usize) >= CAP {
        log::warn!("GetModuleFileNameW failed; cannot probe for steam_api64.dll");
        return false;
    }
    let exe = OsString::from_wide(&buf[..n as usize]);
    let exe_path = PathBuf::from(exe);
    let Some(dir) = exe_path.parent() else {
        return false;
    };
    let dll_path = dir.join("steam_api64.dll");
    let wide: Vec<u16> = dll_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let h = unsafe { LoadLibraryW(wide.as_ptr()) };
    !h.is_null()
}

#[cfg(not(windows))]
pub fn steamworks_dll_ready() -> bool {
    true
}
