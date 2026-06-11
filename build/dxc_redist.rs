//! Copy `dxcompiler.dll` and `dxil.dll` next to the binary on Windows so wgpu's
//! DX12 backend uses DXC instead of falling back to FXC (`d3dcompiler_47.dll`).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const DXC_FILES: &[&str] = &["dxcompiler.dll", "dxil.dll"];

pub fn copy_dxc_redist_next_to_binary() {
    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") || !target.contains("msvc") || target.contains("i686") {
        return;
    }

    println!("cargo:rerun-if-env-changed=MAHJURO_DXC_REDIST");
    for file in DXC_FILES {
        println!("cargo:rerun-if-changed=third_party/dxc-redist/x64/{file}");
    }

    let Some(out_dir) = env::var_os("OUT_DIR").map(PathBuf::from) else {
        return;
    };
    let Some(profile_dir) = super::profile_dir(&out_dir) else {
        return;
    };

    let Some(src_dir) = resolve_dxc_redist_dir() else {
        maybe_warn_missing_dxc();
        return;
    };

    for file in DXC_FILES {
        let src = src_dir.join(file);
        if !src.is_file() {
            maybe_warn_missing_dxc();
            return;
        }
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
}

fn resolve_dxc_redist_dir() -> Option<PathBuf> {
    if let Some(raw) = env::var_os("MAHJURO_DXC_REDIST").filter(|s| !s.is_empty()) {
        let dir = PathBuf::from(raw);
        if dir.is_dir() {
            return Some(dir);
        }
    }

    let repo = env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from)?;
    let bundled = repo.join("third_party/dxc-redist/x64");
    if bundled.is_dir() && dxc_pair_present(&bundled) {
        return Some(bundled);
    }

    windows_sdk_d3d_redist()
}

fn dxc_pair_present(dir: &Path) -> bool {
    DXC_FILES.iter().all(|file| dir.join(file).is_file())
}

fn windows_sdk_d3d_redist() -> Option<PathBuf> {
    let kits_root = PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\Redist\D3D\x64");
    if dxc_pair_present(&kits_root) {
        return Some(kits_root);
    }
    None
}

fn maybe_warn_missing_dxc() {
    let release = env::var("PROFILE").map(|p| p == "release").unwrap_or(false);
    if release {
        println!(
            "cargo:warning=DXC redist not found (dxcompiler.dll + dxil.dll). \
             Run scripts/fetch-dxc-redist.ps1, set MAHJURO_DXC_REDIST, or install the Windows SDK \
             redist — otherwise DX12 may fall back to FXC at runtime."
        );
    }
}
