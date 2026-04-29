//! Steamworks integration. Wraps the `steamworks` crate so the rest of
//! the game can call into it through one type that gracefully no-ops
//! when Steam isn't available (CLI screenshots, headless bot, dev runs
//! with `--no-steam`, players who launched outside Steam, etc.).
//!
//! The init contract: `SteamClient::init` is called once at startup. On
//! success it returns a `Connected` client wrapping a `steamworks::Client`.
//! On failure (no Steam running, no license, init disabled by flag) it
//! returns `Disabled`. Either way, the rest of the codebase calls the
//! same methods (`unlock_achievement`, `run_callbacks`, …) — the
//! `Disabled` variant just logs and returns.

use std::sync::Arc;

use steamworks::{AppId, Client};

pub mod achievement;

pub use achievement::Achievement;

/// Mahjuro's Steam App ID (configured in Steamworks partner backend).
const MAHJURO_APP_ID: u32 = 4636490;

/// Whether this process was launched by the Steam client. Steam injects
/// these env vars into every game it spawns, so this is true even when
/// `SteamClient::init` later fails (offline mode, license check race,
/// etc.) — useful for "we're running off a Steam-installed binary, leave
/// auto-update to Steam" decisions.
pub fn launched_via_steam() -> bool {
    std::env::var_os("SteamAppId").is_some()
        || std::env::var_os("SteamGameId").is_some()
        || std::env::var_os("SteamClientLaunch").is_some()
}

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

    /// Whether Steamworks initialized successfully this session.
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected { .. })
    }

    /// Drain pending Steam callbacks. Should be called once per frame
    /// from the event loop. Cheap; safe to call when disabled.
    pub fn run_callbacks(&self) {
        if let Self::Connected { client } = self {
            client.run_callbacks();
        }
    }

    /// Unlock an achievement. Idempotent — Steam itself silently ignores
    /// repeat unlocks of an already-unlocked achievement, but we also
    /// short-circuit to avoid the round-trip when we can. `store_stats`
    /// is called immediately so the toast pops without waiting for the
    /// next stats flush.
    pub fn unlock_achievement(&self, ach: Achievement) {
        let Self::Connected { client } = self else {
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
}

/// Whether `steam_api64.dll` can be loaded from the executable directory.
///
/// On Windows we link the Steam API with `/DELAYLOAD` so the process starts even
/// when the redistributable DLL is absent; this probes [`LoadLibrary`] before any
/// Steamworks calls so we can fall back to [`SteamClient::Disabled`] instead of
/// a loader error dialog.
#[cfg(windows)]
pub(crate) fn steamworks_dll_ready() -> bool {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use std::path::PathBuf;

    type HMODULE = *mut std::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn GetModuleFileNameW(h_module: HMODULE, lp_filename: *mut u16, n_size: u32) -> u32;
        fn LoadLibraryW(lp_lib_file_name: *const u16) -> HMODULE;
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
pub(crate) fn steamworks_dll_ready() -> bool {
    true
}
