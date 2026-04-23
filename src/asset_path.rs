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

/// Enumerate tileset directory names under `assets/sets/` that ship a usable
/// atlas (both `atlas.toml` and `atlas.png` present). Returns a sorted list
/// with `"original"` pinned first so it remains the default pick.
pub fn list_tilesets() -> Vec<String> {
    let mut names: Vec<String> = <Assets as Embed>::iter()
        .filter_map(|path| {
            let rest = path.strip_prefix("sets/")?;
            let (name, file) = rest.split_once('/')?;
            (file == "atlas.toml").then(|| name.to_string())
        })
        .filter(|name| get(&format!("sets/{name}/atlas.png")).is_some())
        .collect();
    names.sort();
    names.dedup();
    if let Some(pos) = names.iter().position(|n| n == "original") {
        names.swap(0, pos);
    }
    names
}
