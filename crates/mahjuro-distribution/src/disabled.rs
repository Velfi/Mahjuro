//! No-op backend when platform services are unavailable or disabled.

use mahjuro_core::core::progression::PlayerProgress;

use crate::achievement::Achievement;
use crate::backend::DistributionBackend;

#[derive(Clone, Copy, Debug, Default)]
pub struct DisabledBackend;

impl DistributionBackend for DisabledBackend {
    fn tick(&self) {}

    fn unlock_achievement(&self, ach: Achievement) {
        log::debug!(
            "platform services disabled; skipping achievement '{}'",
            ach.steam_api_name()
        );
    }

    fn sync_profile_stats(&self, _progress: &PlayerProgress) {}

    fn is_overlay_available(&self) -> bool {
        false
    }
}
