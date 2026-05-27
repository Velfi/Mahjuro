//! Resolve or build the `mahjuro-bake` workspace binary for `build.rs` GPU bakes.

use std::path::{Path, PathBuf};
use std::process::Command;

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

/// `mahjuro-bake` is a separate workspace crate; build it when stale room data needs a GPU pass.
pub fn ensure_bake_exe(repo: &Path, profile_dir: &Path) -> Option<PathBuf> {
    if let Some(exe) = find_bake_exe(profile_dir) {
        return Some(exe);
    }
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| std::ffi::OsString::from("cargo"));
    let status = Command::new(cargo)
        .args(["build", "-p", "mahjuro-headless", "--bin", "mahjuro-bake", "--quiet"])
        .current_dir(repo)
        .status();
    match status {
        Ok(s) if s.success() => find_bake_exe(profile_dir),
        Ok(s) => {
            println!(
                "cargo:warning=failed to build mahjuro-bake (exit {s}); offline room bake skipped"
            );
            None
        }
        Err(e) => {
            println!("cargo:warning=failed to spawn `cargo build -p mahjuro-bake`: {e}");
            None
        }
    }
}
