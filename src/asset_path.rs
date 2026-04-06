//! Compile-time asset embedding via `rust-embed`.
//!
//! All files under the project's `assets/` directory (except Blender sources)
//! are baked into the binary.  Call [`get`] with a path relative to `assets/`
//! to retrieve the bytes at runtime — no filesystem access required.

use rust_embed::Embed;

#[derive(Embed)]
#[folder = "assets/"]
#[exclude = "*.blend"]
#[exclude = "*.blend1"]
pub struct Assets;

/// Convenience: return the embedded bytes for `path` (relative to `assets/`).
pub fn get(path: &str) -> Option<rust_embed::EmbeddedFile> {
    <Assets as Embed>::get(path)
}

/// Log all embedded asset paths (for debugging).
pub fn log_all_assets() {
    log::info!("Embedded assets:");
    for name in <Assets as Embed>::iter() {
        log::info!("  {name}");
    }
}
