//! Background self-updater using GitHub releases.
//!
//! Two-phase: a background check first discovers whether an update exists
//! (cheap, network-only). The user is then prompted before we actually
//! download and replace the binary.

use std::sync::mpsc;
use std::thread;

use self_update::backends::github;

const REPO_OWNER: &str = "Velfi";
const REPO_NAME: &str = "Mahjuro";

/// Outcome of the background update pipeline.
pub enum UpdateResult {
    /// A newer version exists. Present this to the user and, if they
    /// confirm, call [`UpdateChecker::start_install`].
    UpdateAvailable { new_version: String },
    /// Successfully replaced the binary. The user should restart.
    Updated { new_version: String },
    /// The user opted in but the download/apply step failed. Includes the
    /// release page URL so the user can download manually.
    UpdateFailed {
        new_version: String,
        release_url: String,
        error: String,
    },
}

/// Handle to the background update pipeline. Poll each frame with
/// [`poll`](UpdateChecker::poll).
pub struct UpdateChecker {
    rx: mpsc::Receiver<Option<UpdateResult>>,
    tx: mpsc::Sender<Option<UpdateResult>>,
    done: bool,
}

impl UpdateChecker {
    /// Spawn a background thread that checks whether an update is
    /// available. Does **not** download or apply — that happens only after
    /// the user confirms via [`start_install`](Self::start_install).
    pub fn spawn() -> Self {
        let (tx, rx) = mpsc::channel();

        let tx_clone = tx.clone();
        thread::Builder::new()
            .name("update-check".into())
            .spawn(move || {
                let result = run_check();
                let _ = tx_clone.send(result);
            })
            .expect("spawn update-check thread");

        Self {
            rx,
            tx,
            done: false,
        }
    }

    /// Non-blocking poll. Returns `Some` each time the background pipeline
    /// produces a new result (availability, success, or failure).
    pub fn poll(&mut self) -> Option<UpdateResult> {
        if self.done {
            return None;
        }
        match self.rx.try_recv() {
            Ok(Some(result)) => Some(result),
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

    /// Spawn a second background thread that downloads and applies the
    /// update for `new_version`. The result is delivered via
    /// [`poll`](Self::poll).
    pub fn start_install(&mut self, new_version: String) {
        let tx = self.tx.clone();
        thread::Builder::new()
            .name("update-install".into())
            .spawn(move || {
                let result = Some(run_install(new_version));
                let _ = tx.send(result);
            })
            .expect("spawn update-install thread");
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

fn bin_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "mahjuro.exe"
    } else {
        "mahjuro"
    }
}

fn release_url_for(version: &str) -> String {
    format!("https://github.com/{REPO_OWNER}/{REPO_NAME}/releases/tag/v{version}")
}

fn run_check() -> Option<UpdateResult> {
    let current = env!("CARGO_PKG_VERSION");
    log::info!("checking for updates (current: v{current})...");

    let updater = match github::Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(bin_name())
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

    let latest_version = latest.version.trim_start_matches('v').to_string();
    if !version_is_newer(current, &latest_version) {
        log::info!("already up to date (v{current})");
        return None;
    }

    log::info!("update available: v{current} -> v{latest_version}");
    Some(UpdateResult::UpdateAvailable {
        new_version: latest_version,
    })
}

fn run_install(new_version: String) -> UpdateResult {
    let current = env!("CARGO_PKG_VERSION");
    log::info!("downloading v{new_version}...");

    let updater = match github::Update::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(bin_name())
        .target(update_target())
        .current_version(current)
        .no_confirm(true)
        .show_output(false)
        .show_download_progress(false)
        .build()
    {
        Ok(u) => u,
        Err(e) => {
            log::warn!("update install configure failed: {e}");
            return UpdateResult::UpdateFailed {
                new_version: new_version.clone(),
                release_url: release_url_for(&new_version),
                error: e.to_string(),
            };
        }
    };

    match updater.update() {
        Ok(status) => {
            log::info!("update applied: {status:?}");
            UpdateResult::Updated { new_version }
        }
        Err(e) => {
            log::warn!("update download/apply failed: {e}");
            UpdateResult::UpdateFailed {
                new_version: new_version.clone(),
                release_url: release_url_for(&new_version),
                error: e.to_string(),
            }
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
