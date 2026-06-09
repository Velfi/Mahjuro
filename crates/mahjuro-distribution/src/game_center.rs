//! Game Center backend (Mac App Store).

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use mahjuro_core::core::progression::PlayerProgress;

use crate::achievement::Achievement;
use crate::backend::DistributionBackend;
use crate::stat::profile_stat_snapshot;

pub struct GameCenterBackend {
    authenticated: Arc<AtomicBool>,
    auth_started: AtomicBool,
    pending: Mutex<Vec<Achievement>>,
}

impl GameCenterBackend {
    pub fn init() -> Self {
        #[cfg(target_os = "macos")]
        {
            let backend = Self {
                authenticated: Arc::new(AtomicBool::new(false)),
                auth_started: AtomicBool::new(false),
                pending: Mutex::new(Vec::new()),
            };
            backend.start_authentication();
            backend
        }
        #[cfg(not(target_os = "macos"))]
        {
            log::warn!("Game Center backend requested on non-macOS target; disabled");
            Self {
                authenticated: Arc::new(AtomicBool::new(false)),
                auth_started: AtomicBool::new(false),
                pending: Mutex::new(Vec::new()),
            }
        }
    }
}

#[cfg(target_os = "macos")]
impl GameCenterBackend {
    fn start_authentication(&self) {
        if self.auth_started.swap(true, Ordering::AcqRel) {
            return;
        }
        use block2::RcBlock;
        use objc2_game_kit::GKLocalPlayer;

        let player = unsafe { GKLocalPlayer::localPlayer() };
        if unsafe { player.isAuthenticated() } {
            self.authenticated.store(true, Ordering::Release);
            log::info!("Game Center already authenticated");
            self.flush_pending();
            return;
        }

        let authed = Arc::clone(&self.authenticated);
        let block = RcBlock::new(
            move |view_controller: *mut objc2_app_kit::NSViewController,
                  error: *mut objc2_foundation::NSError| {
                if !view_controller.is_null() {
                    present_game_center_auth_sheet(view_controller);
                    return;
                }
                if error.is_null() {
                    authed.store(true, Ordering::Release);
                    log::info!("Game Center authenticated");
                } else {
                    log::warn!("Game Center authenticate failed");
                }
            },
        );
        unsafe {
            player.setAuthenticateHandler(Some(&block));
        }
    }

    fn refresh_authentication(&self) {
        use objc2_game_kit::GKLocalPlayer;
        let player = unsafe { GKLocalPlayer::localPlayer() };
        let now = unsafe { player.isAuthenticated() };
        if now && !self.authenticated.load(Ordering::Acquire) {
            self.authenticated.store(true, Ordering::Release);
            log::info!("Game Center authentication became ready");
            self.flush_pending();
        }
    }

    fn flush_pending(&self) {
        let mut pending = match self.pending.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        if pending.is_empty() {
            return;
        }
        let batch: Vec<Achievement> = pending.drain(..).collect();
        drop(pending);
        for ach in batch {
            self.report_achievement(ach);
        }
    }

    fn report_achievement(&self, ach: Achievement) {
        use block2::RcBlock;
        use objc2::AnyThread;
        use objc2_foundation::{NSArray, NSString};
        use objc2_game_kit::GKAchievement;

        let id = ach.game_center_id();
        let id_ns = NSString::from_str(id);
        let allocated = GKAchievement::alloc();
        let achievement = unsafe { GKAchievement::initWithIdentifier(allocated, &id_ns) };
        unsafe {
            achievement.setPercentComplete(100.0);
            achievement.setShowsCompletionBanner(true);
        }

        let block = RcBlock::new(move |error: *mut objc2_foundation::NSError| {
            if error.is_null() {
                log::info!("Game Center achievement reported: {id}");
            } else {
                log::warn!("Game Center reportAchievement failed for '{id}'");
            }
        });

        let array = NSArray::from_retained_slice(std::slice::from_ref(&achievement));
        unsafe {
            GKAchievement::reportAchievements_withCompletionHandler(&array, Some(&block));
        }
    }
}

#[cfg(target_os = "macos")]
fn present_game_center_auth_sheet(view_controller: *mut objc2_app_kit::NSViewController) {
    use objc2::MainThreadMarker;
    use objc2::rc::Retained;
    use objc2_app_kit::NSApplication;

    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("Game Center auth UI requires the main thread");
        return;
    };
    let Some(vc) = (unsafe { Retained::retain(view_controller) }) else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    let presenter = app
        .mainWindow()
        .and_then(|window| window.contentViewController())
        .or_else(|| {
            app.keyWindow()
                .and_then(|window| window.contentViewController())
        });
    let Some(presenter) = presenter else {
        log::warn!("Game Center auth UI has no window contentViewController to present from");
        return;
    };
    presenter.presentViewControllerAsSheet(&vc);
}

impl DistributionBackend for GameCenterBackend {
    fn tick(&self) {
        #[cfg(target_os = "macos")]
        {
            self.refresh_authentication();
            if self.authenticated.load(Ordering::Acquire) {
                self.flush_pending();
            }
        }
    }

    fn unlock_achievement(&self, ach: Achievement) {
        #[cfg(target_os = "macos")]
        {
            if !self.authenticated.load(Ordering::Acquire) {
                if let Ok(mut pending) = self.pending.lock() {
                    if !pending.contains(&ach) {
                        pending.push(ach);
                    }
                }
                self.start_authentication();
                log::debug!(
                    "Game Center not authenticated yet; queued '{}'",
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
