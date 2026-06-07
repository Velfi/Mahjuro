//! Game Center backend (Mac App Store).

use mahjuro_core::core::progression::PlayerProgress;

use crate::achievement::Achievement;
use crate::backend::DistributionBackend;
use crate::stat::profile_stat_snapshot;

pub struct GameCenterBackend {
    authenticated: bool,
}

impl GameCenterBackend {
    pub fn init() -> Self {
        #[cfg(target_os = "macos")]
        {
            let mut backend = Self {
                authenticated: false,
            };
            backend.authenticate();
            backend
        }
        #[cfg(not(target_os = "macos"))]
        {
            log::warn!("Game Center backend requested on non-macOS target; disabled");
            Self {
                authenticated: false,
            }
        }
    }
}

#[cfg(target_os = "macos")]
impl GameCenterBackend {
    fn authenticate(&mut self) {
        use block2::RcBlock;
        use objc2_game_kit::GKLocalPlayer;

        let player = unsafe { GKLocalPlayer::localPlayer() };
        if unsafe { player.isAuthenticated() } {
            self.authenticated = true;
            log::info!("Game Center already authenticated");
            return;
        }

        let block = RcBlock::new(move |error: *mut objc2_foundation::NSError| {
            if error.is_null() {
                log::info!("Game Center authenticated");
            } else {
                log::warn!("Game Center authenticate failed");
            }
        });
        unsafe {
            player.authenticateWithCompletionHandler(Some(&block));
        }
        if unsafe { player.isAuthenticated() } {
            self.authenticated = true;
        }
    }

    fn report_achievement(&self, ach: Achievement) {
        use block2::RcBlock;
        use objc2::AnyThread;
        use objc2_foundation::{NSArray, NSString};
        use objc2_game_kit::GKAchievement;

        let id = NSString::from_str(ach.game_center_id());
        let allocated = GKAchievement::alloc();
        let achievement = unsafe { GKAchievement::initWithIdentifier(allocated, &id) };
        unsafe {
            achievement.setPercentComplete(100.0);
            achievement.setShowsCompletionBanner(true);
        }

        let block = RcBlock::new(move |error: *mut objc2_foundation::NSError| {
            if error.is_null() {
                log::info!(
                    "Game Center achievement reported: {}",
                    ach.game_center_id()
                );
            } else {
                log::warn!(
                    "Game Center reportAchievement failed for '{}'",
                    ach.game_center_id()
                );
            }
        });

        let array = NSArray::from_retained_slice(std::slice::from_ref(&achievement));
        unsafe {
            GKAchievement::reportAchievements_withCompletionHandler(&array, Some(&block));
        }
    }
}

impl DistributionBackend for GameCenterBackend {
    fn tick(&self) {}

    fn unlock_achievement(&self, ach: Achievement) {
        #[cfg(target_os = "macos")]
        {
            if !self.authenticated {
                log::debug!(
                    "Game Center not authenticated; skipping '{}'",
                    ach.game_center_id()
                );
                return;
            }
            self.report_achievement(ach);
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = ach;
        }
    }

    fn sync_profile_stats(&self, progress: &PlayerProgress) {
        let snapshot = profile_stat_snapshot(progress);
        for (stat, value) in snapshot {
            log::debug!(
                "Game Center stat sync (leaderboard '{}'): {value}",
                stat.game_center_leaderboard_id()
            );
        }
    }

    fn is_overlay_available(&self) -> bool {
        false
    }
}
