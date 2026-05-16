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
//! `mahjuro` (same layout as Steam depots and GitHub release tarballs — see
//! release workflow AppImage / linux tarball steps). SDL is linked statically
//! (`sdl3` feature `build-from-source-static`).
//!
//! On macOS we pass `-Wl,-rpath,@loader_path` so `libsteam_api.dylib` next to
//! `mahjuro` resolves (see `scripts/package-macos.sh`).
//!
//! **Asset packs:** this script runs `tools/bake_assets/bake_assets.py` into
//! `target/<profile>/` (or `target/<triple>/<profile>/` when cross-compiling) so
//! `pack_manifest.json` and the zip packs sit next to the game binary. Set
//! `MAHJURO_SKIP_ASSET_BAKE=1` to skip (you must supply packs or `MAHJURO_ASSETS`).
//!
//! **Room GI bakes:** when inputs change, `build/room_gi_bake.rs` may run
//! `mahjuro bake-room-gi` (release builds only unless `MAHJURO_ROOM_GI_BAKE=1`).
//! Skip with `MAHJURO_SKIP_ROOM_GI_BAKE=1`. See `AGENTS.md`.

#[path = "build/room_gi_bake.rs"]
mod room_gi_bake;

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    emit_debug_menu_cfg();

    // Defer resolving steam_api64.dll until first use so we can run `main` and
    // skip Steamworks when the DLL is missing (see `steam::steamworks_dll_ready`).
    let target = env::var("TARGET").unwrap_or_default();
    if target.contains("windows-msvc") && target.contains("x86_64") {
        println!("cargo:rustc-link-arg=/DELAYLOAD:steam_api64.dll");
        // `__delayLoadHelper2` lives in delayimp.lib (required for /DELAYLOAD).
        println!("cargo:rustc-link-lib=delayimp");
    }

    if target.contains("linux") && !target.contains("android") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
    }

    if target.contains("apple-darwin") {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@loader_path");
    }

    println!("cargo:rerun-if-env-changed=STEAM_SDK_LOCATION");
    println!("cargo:rerun-if-env-changed=MAHJURO_SKIP_ASSET_BAKE");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=tools/bake_assets/bake_assets.py");
    println!("cargo:rerun-if-changed=tools/bake_assets/pack_rules.json");
    println!("cargo:rerun-if-changed=assets");

    room_gi_bake::emit_rerun_if_changed();

    if let Some(out_dir) = env::var_os("OUT_DIR").map(PathBuf::from) {
        if let Some(profile_dir) = profile_dir(&out_dir) {
            if let Some(repo) = env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from) {
                room_gi_bake::maybe_bake_room_gi(&repo, &profile_dir);
            }
            bake_asset_packs(&profile_dir);
        }
    }

    copy_steam_redistributable_next_to_binary();
}

fn bake_asset_packs(profile_dir: &Path) {
    if env::var("MAHJURO_SKIP_ASSET_BAKE")
        .map(|v| {
            let v = v.trim();
            !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false")
        })
        .unwrap_or(false)
    {
        println!(
            "cargo:warning=MAHJURO_SKIP_ASSET_BAKE: skipping pack bake; ensure {} exists or set MAHJURO_ASSETS",
            profile_dir.join("pack_manifest.json").display()
        );
        return;
    }

    let repo = match env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from) {
        Some(p) if p.is_dir() => p,
        _ => {
            println!("cargo:warning=CARGO_MANIFEST_DIR unset; skipping pack bake");
            return;
        }
    };

    let script = repo.join("tools/bake_assets/bake_assets.py");
    if !script.is_file() {
        println!(
            "cargo:warning={} missing; skipping pack bake",
            script.display()
        );
        return;
    }

    let profile = env::var("PROFILE").unwrap_or_default();
    let release = profile == "release";

    if let Err(e) = run_pack_bake(&repo, &script, profile_dir, release) {
        panic!("asset pack bake failed: {e}");
    }
}

/// Run `bake_assets.py` with a Python interpreter available on PATH.
fn run_pack_bake(repo: &Path, script: &Path, out_dir: &Path, release: bool) -> Result<(), String> {
    let lossy: &[&str] = if release { &[] } else { &["--no-lossy"] };

    #[cfg(windows)]
    let attempts: &[(&str, &[&str])] = &[("python3", &[]), ("python", &[]), ("py", &["-3"])];
    #[cfg(not(windows))]
    let attempts: &[(&str, &[&str])] = &[("python3", &[]), ("python", &[])];

    for (cmd, prefix) in attempts {
        let mut c = Command::new(cmd);
        c.current_dir(repo);
        for p in *prefix {
            c.arg(p);
        }
        c.arg(script);
        c.arg("--out").arg(out_dir);
        for a in lossy {
            c.arg(a);
        }

        match c.status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => {
                return Err(format!(
                    "{cmd} exited with {status} (cwd {}, args for bake_assets.py)",
                    repo.display()
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("failed to spawn {cmd}: {e}")),
        }
    }

    Err(
        "no Python interpreter found (tried python3, python, and on Windows py -3); \
         install Python or set MAHJURO_SKIP_ASSET_BAKE=1 and provide MAHJURO_ASSETS"
            .into(),
    )
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
