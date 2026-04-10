//! Background self-updater using GitHub releases.

use std::sync::mpsc;
use std::thread;

use self_update::backends::github;

const REPO_OWNER: &str = "Velfi";
const REPO_NAME: &str = "Mahjuro";

/// Outcome of a background update attempt.
pub enum UpdateResult {
    /// Successfully replaced the binary. The user should restart.
    Updated { new_version: String },
    /// A newer version exists but the update failed. Includes the
    /// release page URL so the user can download manually.
    UpdateFailed {
        new_version: String,
        release_url: String,
        error: String,
    },
}

/// Handle to a one-shot background update. Poll each frame with
/// [`poll`](UpdateChecker::poll).
pub struct UpdateChecker {
    rx: mpsc::Receiver<Option<UpdateResult>>,
    done: bool,
}

impl UpdateChecker {
    /// Spawn a background thread that checks for (and applies) updates.
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();

        thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let result = run_update();
                let _ = tx.send(result);
            })
            .expect("spawn update-check thread");

        Self { rx, done: false }
    }

    /// Non-blocking poll. Returns `Some` exactly once when the check
    /// completes and an update was attempted.
    pub fn poll(&mut self) -> Option<UpdateResult> {
        if self.done {
            return None;
        }
        match self.rx.try_recv() {
            Ok(Some(result)) => {
                self.done = true;
                Some(result)
            }
            Ok(None) => {
                self.done = true;
                None
            }
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.done = true;
                None
            }
        }
    }
}

/// The target string embedded in release archive names. Must match the
/// archive names produced by the release workflow.
fn update_target() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos-universal"
    } else if cfg!(target_os = "windows") {
        "windows-x86_64"
    } else {
        "linux-x86_64"
    }
}

fn run_update() -> Option<UpdateResult> {
    let current = env!("CARGO_PKG_VERSION");
    log::info!("checking for updates (current: v{current})...");

    let bin_name = if cfg!(target_os = "windows") {
        "mahjuro.exe"
    } else {
        "mahjuro"
    };

    let updater = match github::Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(bin_name)
        .target(update_target())
        .current_version(current)
        .no_confirm(true)
        .show_output(false)
        .show_download_progress(false)
        .build()
    {
        Ok(u) => u,
        Err(e) => {
            log::warn!("update check configure failed: {e}");
            return None;
        }
    };

    let latest = match updater.get_latest_release() {
        Ok(r) => r,
        Err(e) => {
            log::warn!("update check failed: {e}");
            return None;
        }
    };

    let latest_version = latest.version.trim_start_matches('v');
    if !version_is_newer(current, latest_version) {
        log::info!("already up to date (v{current})");
        return None;
    }

    log::info!("update available: v{current} -> v{latest_version}");
    log::info!("downloading and applying update...");

    let new_version = latest_version.to_string();
    match updater.update() {
        Ok(status) => {
            log::info!("update applied: {status:?}");
            Some(UpdateResult::Updated { new_version })
        }
        Err(e) => {
            let release_url = format!(
                "https://github.com/{REPO_OWNER}/{REPO_NAME}/releases/tag/v{new_version}"
            );
            log::warn!("update download/apply failed: {e}");
            Some(UpdateResult::UpdateFailed {
                new_version,
                release_url,
                error: e.to_string(),
            })
        }
    }
}

/// Simple semver comparison: returns true if `latest` is strictly newer than
/// `current`. Only compares major.minor.patch numeric triples.
fn version_is_newer(current: &str, latest: &str) -> bool {
    let parse = |s: &str| -> (u32, u32, u32) {
        let mut parts = s.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
        let major = parts.next().unwrap_or(0);
        let minor = parts.next().unwrap_or(0);
        let patch = parts.next().unwrap_or(0);
        (major, minor, patch)
    };
    parse(latest) > parse(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_is_newer() {
        assert!(version_is_newer("0.1.5", "0.1.6"));
        assert!(version_is_newer("0.1.5", "0.2.0"));
        assert!(version_is_newer("0.1.5", "1.0.0"));
        assert!(!version_is_newer("0.1.5", "0.1.5"));
        assert!(!version_is_newer("0.1.5", "0.1.4"));
        assert!(!version_is_newer("1.0.0", "0.9.9"));
    }
}
