//! Steam API stub for room-bake builds (no `steamworks` dependency).

pub mod achievement;
pub use achievement::Achievement;

/// No-op Steam client; bake paths never call into Steam.
#[derive(Clone, Copy, Debug)]
pub enum SteamClient {
    Disabled,
}

impl SteamClient {
    pub fn init() -> Self {
        Self::Disabled
    }

    pub fn disabled() -> Self {
        Self::Disabled
    }

    pub fn run_callbacks(&self) {}

    pub fn unlock_achievement(&self, _ach: Achievement) {}

    pub fn sync_profile_stats(&self, _progress: &crate::core::progression::PlayerProgress) {}
}

pub(crate) fn steamworks_dll_ready() -> bool {
    false
}
