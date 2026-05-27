//! Locate `mahjuro-bake` binaries for `build.rs` GPU bakes.
//!
//! Do not spawn nested `cargo build` from build scripts — the parent `cargo` already
//! holds the target directory lock, so a child build deadlocks at `mahjuro(build)`.
//! Build the bake tools once (any profile), then rebuild `mahjuro`:
//! `cargo build -p mahjuro-headless --bin mahjuro-bake --features bake`
//! `cargo build -p mahjuro-render --bin mahjuro-bake-decal-atlases`

use std::path::{Path, PathBuf};

pub fn find_decal_bake_exe(profile_dir: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    let names = ["mahjuro-bake-decal-atlases.exe"];
    #[cfg(not(windows))]
    let names = ["mahjuro-bake-decal-atlases"];

    for name in names {
        let p = profile_dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

pub fn find_relic_bake_exe(profile_dir: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    let names = ["mahjuro-bake-relics.exe"];
    #[cfg(not(windows))]
    let names = ["mahjuro-bake-relics"];

    for name in names {
        let p = profile_dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

pub fn require_relic_bake_exe(profile_dir: &Path) -> PathBuf {
    find_relic_bake_exe(profile_dir).unwrap_or_else(|| {
        panic!(
            "mahjuro-bake-relics not found in {}; build it before the relic bake \
             (nested `cargo build` from build.rs deadlocks): \
             `cargo run -p mahjuro-render --bin mahjuro-bake-relics`",
            profile_dir.display()
        );
    })
}

pub fn require_decal_bake_exe(profile_dir: &Path) -> PathBuf {
    find_decal_bake_exe(profile_dir).unwrap_or_else(|| {
        panic!(
            "mahjuro-bake-decal-atlases not found in {}; build it before the showcase decal bake \
             (nested `cargo build` from build.rs deadlocks): \
             `cargo build -p mahjuro-render --bin mahjuro-bake-decal-atlases`",
            profile_dir.display()
        );
    })
}

pub fn find_bake_exe(profile_dir: &Path) -> Option<PathBuf> {
    #[cfg(windows)]
    let names = ["mahjuro-bake.exe"];
    #[cfg(not(windows))]
    let names = ["mahjuro-bake"];

    for name in names {
        let p = profile_dir.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

pub fn require_bake_exe(profile_dir: &Path) -> PathBuf {
    find_bake_exe(profile_dir).unwrap_or_else(|| {
        panic!(
            "mahjuro-bake not found in {}; build it before room GPU bakes \
             (nested `cargo build` from build.rs deadlocks): \
             `cargo build -p mahjuro-headless --bin mahjuro-bake --features bake`",
            profile_dir.display()
        );
    })
}
