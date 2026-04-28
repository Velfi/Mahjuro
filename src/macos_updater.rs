//! Sparkle-backed updater for macOS.
//!
//! Loads `Sparkle.framework` (embedded in `Mahjuro.app/Contents/Frameworks/`)
//! at runtime and drives the `SPUStandardUpdaterController` API. Sparkle owns
//! the entire update UX from here: appcast polling, the "update available"
//! sheet, download progress, signature verification, atomic bundle swap, and
//! relaunch.
//!
//! macOS bundle protection (Gatekeeper) blocks any process from modifying its
//! own `.app` bundle in place — see Apple's DTS guidance — so a self-replace
//! strategy can't work for signed installs in `/Applications`. Sparkle solves
//! this by spawning its `Autoupdate` helper process which performs the swap
//! after the parent app exits.

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject};

/// Holds a strong reference to the `SPUStandardUpdaterController` for the
/// lifetime of the app. Dropping this disables auto-update checks.
pub struct SparkleUpdater {
    _controller: Retained<NSObject>,
}

impl SparkleUpdater {
    /// Initialize Sparkle and start its background scheduler. Returns `None`
    /// if `Sparkle.framework` isn't loaded into the process — typically when
    /// running from `cargo run` (the framework is only embedded in the
    /// release `.app` bundle).
    pub fn start() -> Option<Self> {
        let cls = AnyClass::get(c"SPUStandardUpdaterController")?;

        // [[SPUStandardUpdaterController alloc]
        //     initWithStartingUpdater:YES
        //     updaterDelegate:nil
        //     userDriverDelegate:nil]
        //
        // `startingUpdater:YES` triggers Sparkle's automatic background
        // scheduler immediately, which is what we want — no in-game UI to
        // duplicate.
        let alloc: *mut AnyObject = unsafe { msg_send![cls, alloc] };
        let controller: *mut AnyObject = unsafe {
            msg_send![
                alloc,
                initWithStartingUpdater: true,
                updaterDelegate: std::ptr::null::<AnyObject>(),
                userDriverDelegate: std::ptr::null::<AnyObject>(),
            ]
        };

        if controller.is_null() {
            log::warn!("Sparkle: SPUStandardUpdaterController init returned nil");
            return None;
        }

        // Cast through NSObject so objc2's Retained takes ownership of the
        // +1 retain count we got from `alloc/init`.
        let retained: Retained<NSObject> =
            unsafe { Retained::from_raw(controller as *mut NSObject)? };

        log::info!("Sparkle: updater controller initialized");
        Some(Self {
            _controller: retained,
        })
    }
}

