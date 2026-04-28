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
