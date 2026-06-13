//! Copy the Steamworks redistributable next to the binary so `cargo run`
//! and the produced binary can dlopen it without needing `DYLD_LIBRARY_PATH`
//! or post-build steps.
//!
//! `steamworks-sys` already copies the dylib into its own `OUT_DIR` and
//! adds a link-search path there, which makes linking succeed. But the
//! macOS dylib's install_name is `@loader_path/libsteam_api.dylib`, so at
//! runtime the loader looks for it next to the executable — which means
//! every consumer needs to copy it themselves. We do that here.
//!
//! For packaged builds (Mahjuro.app, .deb, .msi) the platform packaging
//! script is responsible for placing the dylib alongside the binary in
//! the bundle layout — see `scripts/package-macos.sh`.
//!
//! Linux does not search the executable directory for shared libraries by
//! default. We pass `-Wl,-rpath,$ORIGIN` so `libsteam_api.so` can live next to
//! `mahjuro` (same layout as Steam depots and Windows release zips). SDL is linked statically
//! (`sdl3` feature `build-from-source-static`).
//!
//! On macOS we pass `-Wl,-rpath,@loader_path` so `libsteam_api.dylib` next to
//! `mahjuro` resolves (see `scripts/package-macos.sh`).
//!
//! **Asset packs:** when pack-eligible inputs change, runs `tools/bake_assets/bake_assets.py`
//! into `target/<profile>/` (see `build/asset_pack_bake.rs`; `.DS_Store` and other excluded
//! paths do not invalidate the bake). Set `MAHJURO_SKIP_ASSET_BAKE=1` to skip entirely.
//!
//! **Room GI / shadow / decal / relic bakes:** committed outputs under `assets/` must
//! match their input stamps. On local host builds, `build.rs` panics when stale so the
//! user can run the appropriate rebake command. CI (`CI=true`) and cross-compiles
//! also panic. Set `MAHJURO_SKIP_OFFLINE_BAKES=1` to disable freshness checks.
//! - `mahjuro` is built with `headless-screenshot` or `offline-bake-support`,
//! `MAHJURO_SKIP_COMMITTED_BAKE_CHECKS=1`, `MAHJURO_SKIP_OFFLINE_BAKES=1`, or a per-bake
//! `MAHJURO_SKIP_*_BAKE=1`.
//!
//! **Windows DXC:** on MSVC x64, copies `dxcompiler.dll` + `dxil.dll` from
//! `third_party/dxc-redist/x64/` (see `scripts/fetch-dxc-redist.ps1`), `$MAHJURO_DXC_REDIST`,
//! or the Windows SDK redist next to the binary so DX12 uses DXC instead of FXC.

#[path = "build/asset_pack_bake.rs"]
mod asset_pack_bake;
#[path = "build/dxc_redist.rs"]
mod dxc_redist;
#[path = "build/offline_bake.rs"]
mod offline_bake;
#[path = "build/relic_bake.rs"]
mod relic_bake;
#[path = "build/room_gi_bake.rs"]
mod room_gi_bake;
#[path = "build/room_gpu_bake.rs"]
mod room_gpu_bake;
#[path = "build/room_shadow_bake.rs"]
mod room_shadow_bake;
#[path = "build/showcase_decal_bake.rs"]
mod showcase_decal_bake;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    emit_debug_menu_cfg();

    // Windows: link steam_api64 normally; `steam::steamworks_dll_ready` loads the DLL
    // with LoadLibraryW before Client::init so we can skip Steam when it is missing.
    let target = env::var("TARGET").unwrap_or_default();

    if target.contains("linux") && !target.contains("android") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
    }

    if target.contains("apple-darwin") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path");
    }

    println!("cargo:rerun-if-env-changed=STEAM_SDK_LOCATION");
    println!("cargo:rerun-if-env-changed=MAHJURO_SKIP_ASSET_BAKE");
    println!(
        "cargo:rerun-if-env-changed={}",
        mahjuro_bake_stamp::SKIP_COMMITTED_BAKE_CHECKS_ENV
    );
    println!(
        "cargo:rerun-if-env-changed={}",
        mahjuro_bake_stamp::SKIP_OFFLINE_BAKES_ENV
    );
    println!("cargo:rerun-if-changed=build.rs");
    offline_bake::emit_rerun_if_changed();
    room_gpu_bake::emit_rerun_if_changed();
    showcase_decal_bake::emit_rerun_if_changed();
    relic_bake::emit_rerun_if_changed();

    if let Some(out_dir) = env::var_os("OUT_DIR").map(PathBuf::from)
        && let Some(profile_dir) = profile_dir(&out_dir)
        && let Some(repo) = env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from)
    {
        asset_pack_bake::emit_rerun_if_changed(&repo, &profile_dir);
        offline_bake::ensure_committed_offline_bakes_current(&repo, &profile_dir);
        asset_pack_bake::maybe_bake_asset_packs(&repo, &profile_dir);
    }

    if env::var("CARGO_FEATURE_DIST_STEAM").is_ok() {
        copy_steam_redistributable_next_to_binary();
    }

    dxc_redist::copy_dxc_redist_next_to_binary();
}

fn copy_steam_redistributable_next_to_binary() {
    let out_dir = match env::var_os("OUT_DIR") {
        Some(p) => PathBuf::from(p),
        None => return,
    };
    let Some(profile_dir) = profile_dir(&out_dir) else {
        return;
    };

    let target = env::var("TARGET").unwrap_or_default();
    let Some((subdir, file)) = steam_redist_names(&target) else {
        return;
    };

    let sdk = env::var_os("STEAM_SDK_LOCATION")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let had_sdk_env = sdk.is_some();
    let from_sdk = sdk.map(|s| s.join("redistributable_bin").join(subdir).join(file));

    let src = from_sdk
        .filter(|p| p.exists())
        .or_else(|| steamworks_sys_out_redist(&out_dir, file));

    let Some(src) = src else {
        if had_sdk_env {
            println!(
                "cargo:warning=Steam redistributable not under STEAM_SDK_LOCATION — \
                 expected next to the binary for packaged builds; skipping copy of {}",
                file,
            );
        }
        return;
    };

    let dst = profile_dir.join(file);
    if let Err(e) = fs::copy(&src, &dst) {
        println!(
            "cargo:warning=failed to copy {} → {}: {}",
            src.display(),
            dst.display(),
            e,
        );
    }
}

fn steam_redist_names(target: &str) -> Option<(&'static str, &'static str)> {
    if target.contains("darwin") {
        Some(("osx", "libsteam_api.dylib"))
    } else if target.contains("linux") && !target.contains("android") {
        let subdir = if target.contains("aarch64") {
            "linuxarm64"
        } else if target.contains("i686") {
            "linux32"
        } else {
            "linux64"
        };
        Some((subdir, "libsteam_api.so"))
    } else if target.contains("windows") {
        let subdir = if target.contains("i686") { "" } else { "win64" };
        let file = if target.contains("i686") {
            "steam_api.dll"
        } else {
            "steam_api64.dll"
        };
        Some((subdir, file))
    } else {
        None
    }
}

/// After `steamworks-sys` builds, its copy of the Steam API library lives in
/// `target/.../build/steamworks-sys-<hash>/out/`. Reuse that when
/// `STEAM_SDK_LOCATION` is unset (crates.io `steamworks-sys` vendors the
/// redistributable under `lib/steam/`).
fn steamworks_sys_out_redist(mahjuro_out_dir: &Path, file: &str) -> Option<PathBuf> {
    let build_dir = mahjuro_out_dir.parent()?.parent()?;
    let entries = fs::read_dir(build_dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(prefix) = name.to_str() else {
            continue;
        };
        if prefix.starts_with("steamworks-sys-") {
            let candidate = entry.path().join("out").join(file);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Enable the `debug_menu_enabled` cfg whenever the debug menubar should be
/// compiled in. Always on in debug builds; in release builds, opt in by
/// setting `MAHJURO_DEBUG_MENU=1` — useful for collecting accurate perf
/// metrics with the debug overlays available.
fn emit_debug_menu_cfg() {
    println!("cargo:rustc-check-cfg=cfg(debug_menu_enabled)");
    println!("cargo:rerun-if-env-changed=MAHJURO_DEBUG_MENU");

    let debug_profile = env::var("DEBUG")
        .map(|v| v != "false" && v != "0")
        .unwrap_or(false);
    let env_opt_in = env::var("MAHJURO_DEBUG_MENU")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false);

    if debug_profile || env_opt_in {
        println!("cargo:rustc-cfg=debug_menu_enabled");
    }
}

/// Walk up from `OUT_DIR` past `build/<crate>-<hash>/out/` to the profile
/// directory (`target/<profile>/`). Returns `None` if the layout is
/// unexpected (e.g. unusual `target-dir` overrides).
fn profile_dir(out_dir: &Path) -> Option<PathBuf> {
    // OUT_DIR = target/<profile>/build/<crate>-<hash>/out
    // Pops:                ^^^^         ^^^^^^^^^^^^^^^ ^^^
    //                       3            2              1
    let mut p = out_dir.to_path_buf();
    for _ in 0..3 {
        p.pop();
    }
    Some(p)
}
