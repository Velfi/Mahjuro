//! Steam achievements (always) and Steamworks client (interactive builds).
//!
//! Controller input is handled entirely by SDL3. The client covers
//! achievements, stats sync, and Steam callbacks only.

pub mod achievement;
pub use achievement::Achievement;

#[cfg(any(feature = "game", feature = "headless-screenshot"))]
mod client;
#[cfg(any(feature = "game", feature = "headless-screenshot"))]
pub(crate) mod stat;

#[cfg(any(feature = "game", feature = "headless-screenshot"))]
pub use client::{SteamClient, steamworks_dll_ready};
