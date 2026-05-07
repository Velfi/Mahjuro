//! Steamworks integration. Wraps the `steamworks` crate so the rest of
//! the game can call into it through one type that gracefully no-ops
//! when Steam isn't available (CLI screenshots, headless bot, dev runs
//! with `--no-steam`, players who launched outside Steam, etc.).
//!
//! ## Controllers vs Steam Input API
//!
//! Mahjuro reads gamepads **only through SDL** (`SdlShell`, `InputState`).
//! The Steam Input API (`ISteamInput::Init`, action sets, etc.) is for games
//! that poll Steam for digital/analog actions. Initializing it while still
//! using SDL for pads can leave SDL with no usable gamepad when the Steam
//! client is running.
//!
//! **Default (shipping):** we do **not** call `ISteamInput::Init`. Steam
//! overlay, achievements, and friends still work; SDL reads pads from the OS
//! like any other SDL game (use Steam’s per-game controller settings if needed).
//!
//! **Opt-in (development / IGA testing):** pass `enable_steam_input_api:
//! true` to [`SteamClient::init`], or set `--steam-input` / `MAHJURO_STEAM_INPUT=1`.
//! That registers `game_actions_4636490.vdf` (if present), calls `Init`, and
//! enables [`SteamClient::run_steam_input_frame`] each frame. Full In-Game
//! Actions support would still require reading actions via the Steam Input
//! API instead of SDL — this path is only for experiments and overlay binding.
//!
//! The init contract: [`SteamClient::init`] is called once at startup. On
//! success it returns `Connected`; on failure or `--no-steam` it returns
//! `Disabled`. Either way, call sites use the same methods (`unlock_achievement`,
//! `run_callbacks`, …) — `Disabled` is a logged no-op.

use std::path::PathBuf;
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

/// `MAHJURO_STEAM_INPUT=1` or `true` — same effect as [`SteamClient::init`]
/// with `enable_steam_input_api: true` (see module docs).
pub fn steam_input_api_requested_via_env() -> bool {
    std::env::var("MAHJURO_STEAM_INPUT")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub enum SteamClient {
    /// Steam initialized successfully. The inner client is `Send + Sync`
    /// in `steamworks` 0.13, so callbacks can be ticked from the main
    /// thread without an extra wrapper type.
    Connected {
        client: Arc<Client>,
        /// `ISteamInput::Init` succeeded (opt-in only) — call
        /// [`Self::run_steam_input_frame`] once per frame before SDL input.
        steam_input_ready: bool,
    },
    /// Steam is unavailable. Every method is a logged no-op. Used for
    /// `--no-steam`, headless CLI subcommands, and any init failure.
    Disabled,
}

impl SteamClient {
    /// Initialize the Steamworks API. Logs and returns `Disabled` on any
    /// failure path so the caller never has to special-case errors.
    ///
    /// `enable_steam_input_api`: when `true`, registers the IGA VDF (if
    /// present), calls `ISteamInput::Init`, and enables [`Self::run_steam_input_frame`].
    /// Prefer `false` for normal play (SDL gamepads). See the module-level **Controllers vs Steam Input API** section.
    pub fn init(enable_steam_input_api: bool) -> Self {
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
                let steam_input_ready = init_steam_input(client.as_ref(), enable_steam_input_api);
                Self::Connected {
                    client,
                    steam_input_ready,
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
        if let Self::Connected { client, .. } = self {
            client.run_callbacks();
        }
    }

    /// Synchronize Steam Input before SDL gamepad events / polling. No-op when
    /// Steam is disabled or Steam Input failed to initialize.
    pub fn run_steam_input_frame(&self) {
        let Self::Connected {
            client,
            steam_input_ready,
        } = self
        else {
            return;
        };
        if !steam_input_ready {
            return;
        }
        client.input().run_frame();
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
}

fn steam_input_iga_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let p = dir.join("game_actions_4636490.vdf");
    p.is_file().then_some(p)
}

/// [`ISteamInput::SetInputActionManifestFilePath`] + [`ISteamInput::Init`] when `enable` is true.
/// When `enable` is false, does not touch the Steam Input interface (SDL-first; see module docs).
fn init_steam_input(client: &Client, enable: bool) -> bool {
    if !enable {
        log::debug!(
            "Steam Input API: off (SDL gamepads). Enable with --steam-input or MAHJURO_STEAM_INPUT=1 for IGA / Init experiments only."
        );
        return false;
    }

    let input = client.input();
    if let Some(path) = steam_input_iga_path() {
        let p = path.to_string_lossy();
        if input.set_input_action_manifest_file_path(&p) {
            log::debug!("Steam Input: loaded In-Game Actions from {p}");
        } else {
            log::warn!("Steam Input: SetInputActionManifestFilePath failed for {p}");
        }
    } else {
        log::debug!(
            "Steam Input: game_actions_4636490.vdf not next to executable — Steam defaults only"
        );
    }

    let ok = input.init(true);
    if ok {
        log::debug!("Steam Input: ISteamInput::Init ok (explicit RunFrame before SDL each frame)");
    } else {
        log::warn!("Steam Input: ISteamInput::Init returned false");
    }
    ok
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
    unsafe extern "system" {
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
