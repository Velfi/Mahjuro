//! Steamworks client — achievements, stats sync, and callbacks.

use std::sync::Arc;

use steamworks::{AppId, Client};

use super::Achievement;

/// Mahjuro's Steam App ID (configured in Steamworks partner backend).
const MAHJURO_APP_ID: u32 = 4636490;

pub enum SteamClient {
    /// Steam initialized successfully. The inner client is `Send + Sync`
    /// in `steamworks` 0.13, so callbacks can be ticked from the main
    /// thread without an extra wrapper type.
    Connected { client: Arc<Client> },
    /// Steam is unavailable. Every method is a logged no-op. Used for
    /// `--no-steam`, headless CLI subcommands, and any init failure.
    Disabled,
}

impl SteamClient {
    /// Initialize the Steamworks API. Logs and returns `Disabled` on any
    /// failure path so the caller never has to special-case errors.
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
                Self::Connected {
                    client: Arc::new(client),
                }
            }
            Err(err) => {
                log::warn!(
                    "Steam init failed ({err:?}); achievements/stats/overlay will be disabled this session",
                );
                Self::Disabled
            }
        }
    }

    /// Construct a disabled client without attempting init. Used by
    /// `--no-steam` and headless CLI paths.
    pub fn disabled() -> Self {
        Self::Disabled
    }

    /// Drain pending Steam callbacks. Should be called once per frame
    /// from the event loop. Cheap; safe to call when disabled.
    pub fn run_callbacks(&self) {
        if let Self::Connected { client, .. } = self {
            client.run_callbacks();
        }
    }

    /// Unlock an achievement. Idempotent — Steam itself silently ignores
    /// repeat unlocks of an already-unlocked achievement, but we also
    /// short-circuit to avoid the round-trip when we can. `store_stats`
    /// is called immediately so the toast pops without waiting for the
    /// next stats flush.
    pub fn unlock_achievement(&self, ach: Achievement) {
        let Self::Connected { client, .. } = self else {
            return;
        };
        let api_name = ach.api_name();
        let stats = client.user_stats();
        let helper = stats.achievement(api_name);
        match helper.get() {
            Ok(true) => return, // already unlocked; nothing to do
            Ok(false) => {}
            Err(_) => {
                // `get` returns `Err(())` until the stats blob has been
                // received from the server. Setting before that point
                // is unsafe per the Steamworks docs, so bail and let the
                // next caller retry once stats are loaded.
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

    /// Push meta profile counters to Steam Stats (partner-configured INT
    /// stats). Idempotent from the game's perspective — values are always
    /// rewritten from local [`crate::core::progression::PlayerProgress`].
    ///
    /// No-op when disabled. If Steam has not finished loading the user's
    /// stats blob yet, sets are skipped (same as achievements); the next
    /// sync after `run_callbacks` has drained usually succeeds.
    ///
    /// Call after profile saves and when switching profiles so the library
    /// matches disk.
    pub fn sync_profile_stats(&self, progress: &crate::core::progression::PlayerProgress) {
        let Self::Connected { client, .. } = self else {
            return;
        };
        let stats = client.user_stats();
        let snapshot = super::stat::profile_stat_snapshot(progress);
        let mut any_ok = false;
        for (st, value) in snapshot {
            let name = st.api_name();
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
}

/// Whether `steam_api64.dll` can be loaded from the executable directory.
///
/// Probes [`LoadLibraryW`] for `steam_api64.dll` next to the executable before any
/// Steamworks calls so we can use [`SteamClient::Disabled`] when the redistributable
/// is absent (no delay-load / loader dialog).
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
