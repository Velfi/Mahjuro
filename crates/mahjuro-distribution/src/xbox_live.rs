//! Xbox Live backend (Microsoft Store).

use mahjuro_core::core::progression::PlayerProgress;

use crate::achievement::Achievement;
use crate::backend::DistributionBackend;
use crate::stat::profile_stat_snapshot;

#[cxx::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("mahjuro-distribution/cpp/xbox_shim/xbox_shim.h");
        fn xbox_init() -> bool;
        fn xbox_unlock_achievement(id: &str) -> bool;
        fn xbox_set_stat_i32(name: &str, value: i32) -> bool;
        fn xbox_flush_stats();
    }
}

pub struct XboxBackend {
    ready: bool,
}

impl XboxBackend {
    pub fn init() -> Self {
        let ready = ffi::xbox_init();
        if ready {
            log::info!("Xbox Live shim initialized");
        } else {
            log::warn!("Xbox Live shim init failed; achievements disabled");
        }
        Self { ready }
    }
}

impl DistributionBackend for XboxBackend {
    fn tick(&self) {}

    fn unlock_achievement(&self, ach: Achievement) {
        if !self.ready {
            return;
        }
        let id = ach.xbox_achievement_id();
        if !ffi::xbox_unlock_achievement(id) {
            log::warn!("Xbox unlock failed for '{id}'");
        } else {
            log::info!("unlocked Xbox achievement: {id}");
        }
    }

    fn sync_profile_stats(&self, progress: &PlayerProgress) {
        if !self.ready {
            return;
        }
        let snapshot = profile_stat_snapshot(progress);
        let mut any = false;
        for (stat, value) in snapshot {
            if ffi::xbox_set_stat_i32(stat.xbox_stat_name(), value) {
                any = true;
            }
        }
        if any {
            ffi::xbox_flush_stats();
        }
    }

    fn is_overlay_available(&self) -> bool {
        false
    }
}
