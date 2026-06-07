//! Distribution backend trait — achievements, stats, callbacks.

use enum_dispatch::enum_dispatch;
use mahjuro_core::core::progression::PlayerProgress;

use crate::achievement::Achievement;
use crate::disabled::DisabledBackend;
#[cfg(feature = "macos-store")]
use crate::game_center::GameCenterBackend;
#[cfg(feature = "steam")]
use crate::steam::SteamBackend;
#[cfg(feature = "windows-store")]
use crate::xbox_live::XboxBackend;

/// Platform services configuration passed at init.
#[derive(Clone, Copy, Debug, Default)]
pub struct DistributionConfig {
    /// Skip platform sign-in and achievement sync (dev / headless).
    pub platform_services_disabled: bool,
}

/// Store-agnostic achievements and profile stat sync.
#[enum_dispatch]
pub trait DistributionBackend {
    /// Drain pending platform callbacks (Steam overlay pump, etc.).
    fn tick(&self);

    /// Unlock an achievement. Idempotent on all backends.
    fn unlock_achievement(&self, ach: Achievement);

    /// Push meta profile counters to the active store backend.
    fn sync_profile_stats(&self, progress: &PlayerProgress);

    /// Whether an in-game overlay is available (Steam only today).
    fn is_overlay_available(&self) -> bool;
}

/// Active distribution client for this binary SKU.
#[enum_dispatch(DistributionBackend)]
pub enum DistributionClient {
    #[cfg(feature = "steam")]
    Steam(SteamBackend),
    #[cfg(feature = "macos-store")]
    GameCenter(GameCenterBackend),
    #[cfg(feature = "windows-store")]
    Xbox(XboxBackend),
    Disabled(DisabledBackend),
}
