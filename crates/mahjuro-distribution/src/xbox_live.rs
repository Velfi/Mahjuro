//! Xbox Live backend (Microsoft Store).

use std::ffi::CString;
use std::os::raw::{c_char, c_int};

use mahjuro_core::core::progression::PlayerProgress;

use crate::achievement::Achievement;
use crate::backend::DistributionBackend;
use crate::stat::profile_stat_snapshot;

unsafe extern "C" {
    fn xbox_init() -> bool;
    fn xbox_unlock_achievement(id: *const c_char) -> bool;
    fn xbox_set_stat_i32(name: *const c_char, value: c_int) -> bool;
    fn xbox_flush_stats();
}

fn c_str(value: &str) -> Option<CString> {
    CString::new(value).ok()
}

pub struct XboxBackend {
    ready: bool,
}

impl XboxBackend {
    pub fn init() -> Self {
        let ready = unsafe { xbox_init() };
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
        let Some(c_id) = c_str(id) else {
            log::warn!("Xbox unlock skipped: invalid achievement id '{id}'");
            return;
        };
        if !unsafe { xbox_unlock_achievement(c_id.as_ptr()) } {
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
            let name = stat.xbox_stat_name();
            let Some(c_name) = c_str(name) else {
                continue;
            };
            if unsafe { xbox_set_stat_i32(c_name.as_ptr(), value) } {
                any = true;
            }
        }
        if any {
            unsafe { xbox_flush_stats() };
        }
    }

    fn is_overlay_available(&self) -> bool {
        false
    }
}
