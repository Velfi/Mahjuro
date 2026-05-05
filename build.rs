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

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

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

    println!("cargo:rerun-if-env-changed=STEAM_SDK_LOCATION");
    println!("cargo:rerun-if-changed=build.rs");

    let Some(sdk) = env::var_os("STEAM_SDK_LOCATION").map(PathBuf::from) else {
        // No SDK set — `steamworks-sys` will fall back to its bundled
        // path or fail with its own clear error. Nothing to copy.
        return;
    };

    let target = env::var("TARGET").unwrap_or_default();
    let (subdir, file) = if target.contains("darwin") {
        ("osx", "libsteam_api.dylib")
    } else if target.contains("linux") {
        let arch = if target.contains("aarch64") {
            "linuxarm64"
        } else if target.contains("i686") {
            "linux32"
        } else {
            "linux64"
        };
        (arch, "libsteam_api.so")
    } else if target.contains("windows") {
        let arch = if target.contains("i686") { "" } else { "win64" };
        (arch, "steam_api64.dll")
    } else {
        return;
    };

    let src = sdk.join("redistributable_bin").join(subdir).join(file);
    if !src.exists() {
        // Don't hard-fail: a contributor without the SDK should still be
        // able to `cargo check` the crate. `steamworks-sys`'s own build
        // script will surface a real error if linking fails.
        println!(
            "cargo:warning=Steam dylib not found at {} — skipping copy",
            src.display(),
        );
        return;
    }

    // OUT_DIR is `target/<profile>/build/<crate>-<hash>/out`. Walk up to
    // the profile dir so the dylib lands next to the final binary.
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let Some(profile_dir) = profile_dir(&out_dir) else {
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
